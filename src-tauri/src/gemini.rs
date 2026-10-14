//! Gemini app usage. Signing in opens Google's sign-in for Gemini in a
//! window; after that a hidden window stays on gemini.google.com and each
//! poll runs the usage request inside it ([`usage_core::GEMINI_FETCH_JS`]).
//!
//! Running the request in a Gemini page instead of copying cookies out
//! keeps the session alive: Google rotates its session cookies from its
//! own pages, and copied cookies stop working once that happens. The
//! password is typed into Google itself; this app never sees it.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use chrono::Utc;
use tauri::{AppHandle, Manager, Url, WebviewUrl, WebviewWindow, WebviewWindowBuilder};
use usage_core::{parse_gemini_result, Usage, UsageError, GEMINI_FETCH_JS};

use crate::session::SAFARI_UA;

const LOGIN_LABEL: &str = "gemini-login";
const FETCH_LABEL: &str = "gemini-fetch";
const APP_URL: &str = "https://gemini.google.com/app";
const LOGIN_URL: &str =
    "https://accounts.google.com/ServiceLogin?continue=https%3A%2F%2Fgemini.google.com%2Fapp";
/// Present in the app's data folder while Gemini is signed in.
const ENABLED_FILE: &str = "gemini-enabled";
const LOGIN_TIMEOUT: Duration = Duration::from_secs(15 * 60);
const LOAD_TIMEOUT: Duration = Duration::from_secs(30);
const FETCH_TIMEOUT: Duration = Duration::from_secs(30);

/// "yes" once the page is Gemini with a signed-in user (the XSRF token is
/// only there for one). Page probes return strings or null: wry turns the
/// result into JSON, and other kinds of value can't always be converted.
const SIGNED_IN_JS: &str = r#"(() => {
  try {
    return location.host === "gemini.google.com" && window.WIZ_global_data && window.WIZ_global_data.SNlM0e ? "yes" : "no";
  } catch (_) { return "no"; }
})()"#;
/// The page's host once it has finished loading.
const LOADED_JS: &str = r#"(() => document.readyState === "complete" ? location.host : null)()"#;

static NEXT_ID: AtomicU64 = AtomicU64::new(1);

fn marker(app: &AppHandle) -> Option<PathBuf> {
    app.path().app_data_dir().ok().map(|d| d.join(ENABLED_FILE))
}

/// Whether the user has signed in to Gemini through the app.
pub fn enabled(app: &AppHandle) -> bool {
    marker(app).is_some_and(|p| p.exists())
}

fn set_enabled(app: &AppHandle, on: bool) {
    let Some(path) = marker(app) else {
        return;
    };
    if on {
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = std::fs::write(path, b"");
    } else {
        let _ = std::fs::remove_file(path);
    }
}

/// Runs `js` in `window` and returns its string result. Blocks until the
/// page answers, so never call it on the main thread (the answer is
/// delivered there).
fn eval_string(window: &WebviewWindow, js: &str) -> Option<String> {
    let (tx, rx) = mpsc::channel();
    window
        .eval_with_callback(js, move |result| {
            let _ = tx.send(result);
        })
        .ok()?;
    let raw = rx.recv_timeout(Duration::from_secs(5)).ok()?;
    serde_json::from_str::<Option<String>>(&raw).ok().flatten()
}

/// Opens Google's sign-in for Gemini (or focuses it) and calls `on_login`
/// once Gemini shows a signed-in user.
pub fn open_login_window(app: &AppHandle, on_login: impl Fn(&AppHandle) + Send + 'static) {
    if let Some(w) = app.get_webview_window(LOGIN_LABEL) {
        let _ = w.show();
        let _ = w.set_focus();
        return;
    }
    let url: Url = LOGIN_URL.parse().expect("valid login URL");
    let built = WebviewWindowBuilder::new(app, LOGIN_LABEL, WebviewUrl::External(url))
        .title("Gemini 로그인 · Claude Usage")
        .user_agent(SAFARI_UA)
        .inner_size(480.0, 720.0)
        .center()
        .focused(true)
        // A menu bar app's windows otherwise open behind the active app.
        .always_on_top(true)
        .build();
    let Ok(window) = built else {
        return;
    };
    let _ = window.set_focus();

    let app = app.clone();
    std::thread::spawn(move || {
        let started = Instant::now();
        while started.elapsed() < LOGIN_TIMEOUT {
            std::thread::sleep(Duration::from_secs(1));
            let Some(window) = app.get_webview_window(LOGIN_LABEL) else {
                return; // closed by the user
            };
            if eval_string(&window, SIGNED_IN_JS).as_deref() == Some("yes") {
                set_enabled(&app, true);
                // Start the hidden page over with the new session.
                if let Some(w) = app.get_webview_window(FETCH_LABEL) {
                    let _ = w.destroy();
                }
                let _ = window.destroy();
                on_login(&app);
                return;
            }
        }
    });
}

/// Stops reading Gemini usage and signs Google out of the app's webviews,
/// so the next sign-in can pick a different account.
pub fn logout(app: &AppHandle) {
    set_enabled(app, false);
    for label in [FETCH_LABEL, LOGIN_LABEL] {
        if let Some(w) = app.get_webview_window(label) {
            let _ = w.destroy();
        }
    }
    crate::session::delete_cookies(app, |domain| {
        domain == "google.com" || domain.ends_with(".google.com")
    });
}

/// The hidden Gemini page, created on first use.
fn fetch_window(app: &AppHandle) -> Result<WebviewWindow, UsageError> {
    if let Some(w) = app.get_webview_window(FETCH_LABEL) {
        return Ok(w);
    }
    let url: Url = APP_URL.parse().expect("valid app URL");
    WebviewWindowBuilder::new(app, FETCH_LABEL, WebviewUrl::External(url))
        .title("Gemini")
        .user_agent(SAFARI_UA)
        .inner_size(800.0, 600.0)
        .visible(false)
        .skip_taskbar(true)
        .focused(false)
        .build()
        .map_err(|e| UsageError::Network(e.to_string()))
}

/// Reads Gemini usage through the hidden page. Blocks; call it off the
/// main thread.
pub fn fetch(app: &AppHandle) -> Result<Usage, UsageError> {
    let window = fetch_window(app)?;
    let started = Instant::now();
    let host = loop {
        if let Some(host) = eval_string(&window, LOADED_JS) {
            break host;
        }
        if started.elapsed() > LOAD_TIMEOUT {
            return Err(UsageError::Network("Gemini 페이지가 열리지 않아요".into()));
        }
        std::thread::sleep(Duration::from_millis(500));
    };
    if host != "gemini.google.com" {
        // Sent to Google's sign-in: the session is gone.
        return Err(UsageError::GeminiLoggedOut);
    }
    match run(&window, false) {
        // The tokens a long-open page holds can expire; try fresh ones,
        // and reload the page if even those don't work.
        Err(UsageError::GeminiLoggedOut) => {
            let result = run(&window, true);
            if matches!(result, Err(UsageError::GeminiLoggedOut)) {
                let _ = window.eval("location.reload()");
            }
            result
        }
        result => result,
    }
}

fn run(window: &WebviewWindow, fresh: bool) -> Result<Usage, UsageError> {
    let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    let js = GEMINI_FETCH_JS
        .replace("__ID__", &id.to_string())
        .replace("__FRESH__", if fresh { "true" } else { "false" });
    window
        .eval(js)
        .map_err(|e| UsageError::Network(e.to_string()))?;
    let probe = format!(
        "(() => {{ const r = window.__claudeUsageGemini; return r && r.id === {id} ? r.text : null; }})()"
    );
    let started = Instant::now();
    while started.elapsed() < FETCH_TIMEOUT {
        std::thread::sleep(Duration::from_millis(300));
        if let Some(text) = eval_string(window, &probe) {
            return parse_gemini_result(&text, Utc::now());
        }
    }
    Err(UsageError::Network("Gemini가 응답하지 않아요".into()))
}
