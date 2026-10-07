//! claude.ai login: opens claude.ai in a window, waits for the `sessionKey`
//! cookie it sets after login, and keeps that cookie in the OS credential
//! store (macOS Keychain / Windows Credential Manager). The password is
//! typed into claude.ai itself; this app never sees it.

use std::sync::atomic::{AtomicUsize, Ordering};
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

/// Page script for sign-in popups: `window.close()` does nothing in an
/// embedded webview, so turn it into a navigation to [`CLOSE_HOST`], which
/// [`open_popup`] catches and answers by closing the window.
const POPUP_CLOSE_JS: &str = r#"
(() => {
  const close = window.close.bind(window);
  window.close = () => {
    try { location.href = "https://close.claude-usage.invalid/"; } catch (_) { close(); }
  };
})();
"#;
const CLOSE_HOST: &str = "close.claude-usage.invalid";

/// Opens `window.open()` popups (Google / Apple sign-in) as real windows.
///
/// The window starts on `about:blank` and WebKit loads the requested page
/// into it itself. Loading the URL from here instead would make it a fresh
/// navigation that drops `window.opener`, and Google's sign-in hands its
/// result back to claude.ai through `window.opener.postMessage`.
fn open_popup(app: &AppHandle, features: NewWindowFeatures) -> NewWindowResponse<Wry> {
    let label = format!(
        "login-popup-{}",
        POPUP_COUNTER.fetch_add(1, Ordering::Relaxed)
    );
    let nav_app = app.clone();
    let nav_label = label.clone();
    let blank: Url = "about:blank".parse().expect("valid URL");
    let built = WebviewWindowBuilder::new(app, label, WebviewUrl::External(blank))
        .window_features(features)
        .initialization_script(POPUP_CLOSE_JS)
        .on_navigation(move |url| {
            if !is_close_request(url) {
                return true;
            }
            let app = nav_app.clone();
            let label = nav_label.clone();
            let _ = nav_app.run_on_main_thread(move || {
                if let Some(popup) = app.get_webview_window(&label) {
                    let _ = popup.destroy();
                }
                if let Some(login) = app.get_webview_window(LOGIN_LABEL) {
                    let _ = login.set_focus();
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

fn is_close_request(url: &Url) -> bool {
    url.host_str() == Some(CLOSE_HOST)
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
        .on_new_window(move |_url, features| open_popup(&popup_app, features))
        .on_navigation(|url| {
            // Page scripts can be shared with popups; never leave for the
            // popups' close signal.
            !is_close_request(url)
        })
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
    fn recognizes_close_requests() {
        let u = |s: &str| s.parse::<Url>().unwrap();
        assert!(is_close_request(&u("https://close.claude-usage.invalid/")));
        assert!(!is_close_request(&u(
            "https://accounts.google.com/gsi/transform"
        )));
        assert!(!is_close_request(&u("https://claude.ai/login")));
    }
}
