//! claude.ai login: opens claude.ai in a window, waits for the `sessionKey`
//! cookie it sets after login, and keeps that cookie in the OS credential
//! store (macOS Keychain / Windows Credential Manager). The password is
//! typed into claude.ai itself; this app never sees it.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use tauri::webview::{NewWindowFeatures, NewWindowResponse};
use tauri::{AppHandle, Manager, Url, WebviewUrl, WebviewWindowBuilder, Wry};

const SERVICE: &str = "io.github.smtodj.claude-usage-widget";
const ACCOUNT: &str = "claude.ai-sessionKey";
const LOGIN_LABEL: &str = "login";
const LOGIN_URL: &str = "https://claude.ai/login";
const COOKIE_URL: &str = "https://claude.ai";
const LOGIN_TIMEOUT: Duration = Duration::from_secs(15 * 60);
// Google refuses to sign in inside embedded webviews it recognizes, and the
// default WKWebView user agent lacks the "Version/… Safari/…" part it looks
// for. Present as Safari so "Google로 계속하기" works.
const SAFARI_UA: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.0 Safari/605.1.15";

static POPUP_COUNTER: AtomicUsize = AtomicUsize::new(0);

/// Opens `window.open()` popups (Google / Apple sign-in) as real windows
/// that share the login window's cookies.
///
/// When the provider sends the popup back to claude.ai, claude.ai's page
/// expects to hand the result to its opener and close itself, which the
/// embedded webview doesn't carry through (the popup just goes blank). So
/// that return trip is loaded in the login window instead, where claude.ai
/// finishes the sign-in and sets its session cookie, and the popup closes.
fn open_popup(app: &AppHandle, url: Url, features: NewWindowFeatures) -> NewWindowResponse<Wry> {
    let label = format!(
        "login-popup-{}",
        POPUP_COUNTER.fetch_add(1, Ordering::Relaxed)
    );
    let left_claude = Arc::new(AtomicBool::new(false));
    let nav_app = app.clone();
    let nav_label = label.clone();
    let built = WebviewWindowBuilder::new(app, label, WebviewUrl::External(url))
        .window_features(features)
        .on_navigation(move |url| {
            if !is_claude(url) {
                left_claude.store(true, Ordering::Relaxed);
                return true;
            }
            // Only the GET return with the result in the query string can be
            // replayed in another window (a form POST, as Apple may use,
            // can't), and the claude.ai page that starts sign-in stays put.
            if !left_claude.load(Ordering::Relaxed) || !carries_auth_result(url) {
                return true;
            }
            let app = nav_app.clone();
            let label = nav_label.clone();
            let url = url.clone();
            let _ = nav_app.run_on_main_thread(move || {
                if let Some(login) = app.get_webview_window(LOGIN_LABEL) {
                    let _ = login.navigate(url);
                    let _ = login.set_focus();
                }
                if let Some(popup) = app.get_webview_window(&label) {
                    let _ = popup.destroy();
                }
            });
            false
        })
        .title("로그인")
        .user_agent(SAFARI_UA)
        .always_on_top(true)
        .focused(true)
        .build();
    match built {
        Ok(window) => NewWindowResponse::Create { window },
        Err(_) => NewWindowResponse::Allow,
    }
}

fn is_claude(url: &Url) -> bool {
    matches!(url.host_str(), Some(h) if h == "claude.ai" || h.ends_with(".claude.ai"))
}

fn carries_auth_result(url: &Url) -> bool {
    url.query_pairs().any(|(k, _)| k == "code" || k == "state")
}

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
    let popup_app = app.clone();
    let built = WebviewWindowBuilder::new(app, LOGIN_LABEL, WebviewUrl::External(url))
        .title("claude.ai 로그인 · Claude Usage")
        .user_agent(SAFARI_UA)
        .on_new_window(move |url, features| open_popup(&popup_app, url, features))
        .inner_size(480.0, 720.0)
        .center()
        .focused(true)
        // Stay in front: a menu bar app's windows otherwise open behind
        // the active app.
        .always_on_top(true)
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
                    for (label, w) in app.webview_windows() {
                        if label.starts_with("login-popup-") {
                            let _ = w.destroy();
                        }
                    }
                    let _ = window.destroy();
                    on_login(&app);
                }
                return;
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognizes_claude_hosts() {
        let u = |s: &str| s.parse::<Url>().unwrap();
        assert!(is_claude(&u("https://claude.ai/login/google-auth?code=x")));
        assert!(is_claude(&u("https://www.claude.ai/")));
        assert!(!is_claude(&u("https://accounts.google.com/o/oauth2/auth")));
        assert!(!is_claude(&u("https://notclaude.ai/")));
        assert!(carries_auth_result(&u(
            "https://claude.ai/cb?code=a&state=b"
        )));
        assert!(!carries_auth_result(&u("https://claude.ai/login")));
    }
}
