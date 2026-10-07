//! claude.ai login: opens claude.ai in a window, waits for the `sessionKey`
//! cookie it sets after login, and keeps that cookie in the OS credential
//! store (macOS Keychain / Windows Credential Manager). The password is
//! typed into claude.ai itself; this app never sees it.

use std::time::{Duration, Instant};

use tauri::{AppHandle, Manager, Url, WebviewUrl, WebviewWindowBuilder};

const SERVICE: &str = "io.github.smtodj.claude-usage-widget";
const ACCOUNT: &str = "claude.ai-sessionKey";
const LOGIN_LABEL: &str = "login";
const LOGIN_URL: &str = "https://claude.ai/login";
const COOKIE_URL: &str = "https://claude.ai";
const LOGIN_TIMEOUT: Duration = Duration::from_secs(15 * 60);

fn entry() -> keyring::Result<keyring::Entry> {
    keyring::Entry::new(SERVICE, ACCOUNT)
}

/// The stored claude.ai session key, if the user logged in through the app.
pub fn load() -> Option<String> {
    entry()
        .and_then(|e| e.get_password())
        .ok()
        .filter(|k| !k.is_empty())
}

fn save(key: &str) -> keyring::Result<()> {
    entry()?.set_password(key)
}

/// Forgets the session key and the login window's cookies, so the next
/// login can pick a different account.
pub fn logout(app: &AppHandle) {
    if let Ok(e) = entry() {
        let _ = e.delete_credential();
    }
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.clear_all_browsing_data();
    }
}

/// Opens the claude.ai login window (or focuses it) and calls `on_login`
/// once the session cookie has been captured and saved.
pub fn open_login_window(app: &AppHandle, on_login: impl Fn(&AppHandle) + Send + 'static) {
    if let Some(w) = app.get_webview_window(LOGIN_LABEL) {
        let _ = w.show();
        let _ = w.set_focus();
        return;
    }
    let url: Url = LOGIN_URL.parse().expect("valid login URL");
    let built = WebviewWindowBuilder::new(app, LOGIN_LABEL, WebviewUrl::External(url))
        .title("claude.ai 로그인 · Claude Usage")
        .inner_size(480.0, 720.0)
        .center()
        .build();
    let Ok(window) = built else {
        return;
    };
    let _ = window.set_focus();

    // Poll the cookie store off the main thread (reading cookies from the
    // main thread deadlocks on Windows).
    let app = app.clone();
    std::thread::spawn(move || {
        let started = Instant::now();
        let cookie_url: Url = COOKIE_URL.parse().expect("valid cookie URL");
        while started.elapsed() < LOGIN_TIMEOUT {
            std::thread::sleep(Duration::from_secs(1));
            let Some(window) = app.get_webview_window(LOGIN_LABEL) else {
                return; // closed by the user
            };
            let Ok(cookies) = window.cookies_for_url(cookie_url.clone()) else {
                continue;
            };
            let key = cookies
                .iter()
                .find(|c| c.name() == "sessionKey" && !c.value().is_empty())
                .map(|c| c.value().to_string());
            if let Some(key) = key {
                if save(&key).is_ok() {
                    let _ = window.destroy();
                    on_login(&app);
                }
                return;
            }
        }
    });
}
