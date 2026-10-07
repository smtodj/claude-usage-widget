mod session;

use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::sync::Mutex;
use std::time::Duration;

use chrono::{DateTime, Local, Utc};
use serde::Serialize;
use tauri::image::Image;
use tauri::menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Emitter, Manager, RunEvent, WindowEvent, Wry};
use tauri_plugin_autostart::{MacosLauncher, ManagerExt};
use usage_core::{format_reset, short_label, Usage, UsageError};

const TRAY_ID: &str = "main";
const POLL_INTERVAL: Duration = Duration::from_secs(120);

/// What the tray and the details window show.
#[derive(Clone, Serialize, Default)]
struct Snapshot {
    usage: Option<Usage>,
    error: Option<String>,
    /// When `error` is set, `usage` is the last good reading (stale).
    checked_at: Option<DateTime<Utc>>,
    /// Where the numbers came from: "claude.ai 로그인" or "Claude Code".
    source: Option<&'static str>,
    /// Whether a claude.ai session is stored (shows "로그아웃" instead of "로그인").
    web_logged_in: bool,
}

struct AppState {
    snapshot: Mutex<Snapshot>,
    refresh_tx: Mutex<Option<Sender<()>>>,
}

#[tauri::command]
fn get_usage(state: tauri::State<'_, AppState>) -> Snapshot {
    state.snapshot.lock().unwrap().clone()
}

#[tauri::command]
fn refresh_usage(state: tauri::State<'_, AppState>) {
    request_refresh(&state);
}

// Async so they run off the main thread: creating a window from a sync
// command deadlocks on Windows.
#[tauri::command]
async fn login(app: AppHandle) {
    open_login(&app);
}

#[tauri::command]
async fn logout(app: AppHandle) {
    session::logout(&app);
    request_refresh(&app.state::<AppState>());
}

/// A menu bar (Accessory) app is never the active app, so its new windows
/// open behind whatever the user is looking at unless it is activated.
fn bring_app_forward(app: &AppHandle) {
    #[cfg(target_os = "macos")]
    let _ = app.show();
    #[cfg(not(target_os = "macos"))]
    let _ = app;
}

fn open_login(app: &AppHandle) {
    bring_app_forward(app);
    session::open_login_window(app, |app| request_refresh(&app.state::<AppState>()));
}

fn request_refresh(state: &AppState) {
    if let Some(tx) = state.refresh_tx.lock().unwrap().as_ref() {
        let _ = tx.send(());
    }
}

pub fn run() {
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_autostart::init(
            MacosLauncher::LaunchAgent,
            None,
        ))
        .manage(AppState {
            snapshot: Mutex::new(Snapshot::default()),
            refresh_tx: Mutex::new(None),
        })
        .invoke_handler(tauri::generate_handler![
            get_usage,
            refresh_usage,
            login,
            logout
        ])
        .setup(|app| {
            // Menu bar only: no Dock icon.
            #[cfg(target_os = "macos")]
            app.set_activation_policy(tauri::ActivationPolicy::Accessory);

            let handle = app.handle().clone();
            TrayIconBuilder::with_id(TRAY_ID)
                .icon(Image::from_bytes(include_bytes!("../icons/tray.png"))?)
                .icon_as_template(true)
                .title("Claude …")
                .tooltip("Claude 사용량")
                .menu(&build_menu(&handle, &Snapshot::default())?)
                .show_menu_on_left_click(true)
                .on_menu_event(on_menu_event)
                .build(app)?;

            start_polling(handle);
            Ok(())
        })
        .on_window_event(|window, event| {
            // Closing the details window only hides it; the tray keeps running.
            if let WindowEvent::CloseRequested { api, .. } = event {
                if window.label() != "main" {
                    return;
                }
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .build(tauri::generate_context!())
        .expect("failed to build app");

    app.run(|_, event| {
        // Keep running with no windows open; only "종료" exits (code is Some).
        if let RunEvent::ExitRequested {
            api, code: None, ..
        } = event
        {
            api.prevent_exit();
        }
    });
}

fn start_polling(app: AppHandle) {
    let (tx, rx) = mpsc::channel::<()>();
    *app.state::<AppState>().refresh_tx.lock().unwrap() = Some(tx);

    std::thread::spawn(move || {
        let mut first = true;
        loop {
            poll_once(&app, first);
            first = false;
            match rx.recv_timeout(POLL_INTERVAL) {
                Ok(()) | Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => break,
            }
        }
    });
}

/// A stored claude.ai login wins; otherwise Claude Code's token is used.
fn read_usage() -> (Result<Usage, UsageError>, &'static str, bool) {
    match session::load() {
        Some(key) => (usage_core::fetch_usage_web(&key), "claude.ai 로그인", true),
        None => (usage_core::current_usage(), "Claude Code", false),
    }
}

fn poll_once(app: &AppHandle, first: bool) {
    let (result, source, web_logged_in) = read_usage();
    // First launch without a working login: open the login window right
    // away. Claude Code being absent, expired or unreadable all count.
    let needs_login = matches!(
        result,
        Err(UsageError::NotLoggedIn
            | UsageError::TokenExpired
            | UsageError::Unauthorized
            | UsageError::Credentials(_))
    );
    if first && !web_logged_in && needs_login {
        let handle = app.clone();
        let _ = app.run_on_main_thread(move || open_login(&handle));
    }
    let snapshot = {
        let state = app.state::<AppState>();
        let mut snap = state.snapshot.lock().unwrap();
        // Don't show the previous account's numbers after switching sources.
        if snap.source.is_some() && snap.source != Some(source) {
            snap.usage = None;
        }
        snap.source = Some(source);
        snap.web_logged_in = web_logged_in;
        match result {
            Ok(usage) => {
                snap.usage = Some(usage);
                snap.error = None;
            }
            Err(e) => snap.error = Some(e.to_string()),
        }
        snap.checked_at = Some(Utc::now());
        snap.clone()
    };
    update_tray(app, snapshot.clone());
    let _ = app.emit("usage-updated", snapshot);
}

fn update_tray(app: &AppHandle, snapshot: Snapshot) {
    let handle = app.clone();
    let _ = app.run_on_main_thread(move || {
        let Some(tray) = handle.tray_by_id(TRAY_ID) else {
            return;
        };
        let title = tray_title(&snapshot);
        let _ = tray.set_title(Some(&title));
        let _ = tray.set_tooltip(Some(format!("Claude 남은 사용량: {title}")));
        if let Ok(menu) = build_menu(&handle, &snapshot) {
            let _ = tray.set_menu(Some(menu));
        }
    });
}

/// "5h 72% · 7d 39%" (remaining), with a warning mark when the last read failed.
fn tray_title(s: &Snapshot) -> String {
    let Some(usage) = &s.usage else {
        return if s.error.is_some() {
            "Claude ⚠︎".into()
        } else {
            "Claude …".into()
        };
    };
    let parts: Vec<String> = ["five_hour", "seven_day"]
        .iter()
        .filter_map(|k| usage.window(k))
        .map(|w| format!("{} {:.0}%", short_label(&w.key), w.remaining_percent))
        .collect();
    let mut title = parts.join(" · ");
    if s.error.is_some() {
        title.push_str(" ⚠︎");
    }
    title
}

fn build_menu(app: &AppHandle, s: &Snapshot) -> tauri::Result<Menu<Wry>> {
    let menu = Menu::new(app)?;
    let info = |text: String| MenuItem::new(app, text, false, None::<&str>);

    if let Some(usage) = &s.usage {
        let now = Utc::now();
        for w in &usage.windows {
            menu.append(&info(format!(
                "{}: {:.0}% 남음 ({:.0}% 사용)",
                w.label, w.remaining_percent, w.used_percent
            ))?)?;
            menu.append(&info(format!("    {}", format_reset(w.resets_at, now)))?)?;
        }
    }
    if let Some(err) = &s.error {
        if s.usage.is_some() {
            menu.append(&info("⚠︎ 최신 값이 아니에요".into())?)?;
        }
        menu.append(&info(format!("⚠︎ {err}"))?)?;
    }
    if s.usage.is_none() && s.error.is_none() {
        menu.append(&info("불러오는 중…".into())?)?;
    }
    if let Some(at) = s.checked_at {
        let local: DateTime<Local> = at.into();
        let source = s.source.map(|x| format!(" · {x}")).unwrap_or_default();
        menu.append(&info(format!(
            "마지막 확인: {}{source}",
            local.format("%H:%M")
        ))?)?;
    }

    menu.append(&PredefinedMenuItem::separator(app)?)?;
    menu.append(&MenuItem::with_id(
        app,
        "refresh",
        "지금 새로고침",
        true,
        None::<&str>,
    )?)?;
    menu.append(&MenuItem::with_id(
        app,
        "details",
        "자세히 보기…",
        true,
        None::<&str>,
    )?)?;
    menu.append(&if s.web_logged_in {
        MenuItem::with_id(app, "logout", "claude.ai 로그아웃", true, None::<&str>)?
    } else {
        MenuItem::with_id(app, "login", "claude.ai로 로그인…", true, None::<&str>)?
    })?;
    let autostart = app.autolaunch().is_enabled().unwrap_or(false);
    menu.append(&CheckMenuItem::with_id(
        app,
        "autostart",
        "로그인 시 자동 실행",
        true,
        autostart,
        None::<&str>,
    )?)?;
    menu.append(&PredefinedMenuItem::separator(app)?)?;
    menu.append(&MenuItem::with_id(app, "quit", "종료", true, None::<&str>)?)?;
    Ok(menu)
}

fn on_menu_event(app: &AppHandle, event: tauri::menu::MenuEvent) {
    match event.id().as_ref() {
        "refresh" => request_refresh(&app.state::<AppState>()),
        "details" => {
            bring_app_forward(app);
            if let Some(w) = app.get_webview_window("main") {
                let _ = w.show();
                let _ = w.set_focus();
            }
        }
        "autostart" => {
            let launch = app.autolaunch();
            let _ = if launch.is_enabled().unwrap_or(false) {
                launch.disable()
            } else {
                launch.enable()
            };
            let snapshot = app.state::<AppState>().snapshot.lock().unwrap().clone();
            update_tray(app, snapshot);
        }
        "login" => open_login(app),
        "logout" => {
            session::logout(app);
            request_refresh(&app.state::<AppState>());
        }
        "quit" => app.exit(0),
        _ => {}
    }
}
