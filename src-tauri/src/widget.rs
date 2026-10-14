//! Hands the latest readings to the macOS desktop widgets.
//!
//! The widgets (`macos/widget`, bundled as `Contents/PlugIns/UsageWidget.appex`)
//! are sandboxed and ad-hoc signed, so they can't share an App Group with
//! this app. Instead this app writes each [`Feed`]'s JSON straight into the
//! extension's own sandbox container, which it sees as its home directory,
//! then runs `widget-reload <kind>` to have WidgetKit redraw that widget.
//!
//!
//! WidgetKit gives each widget a daily reload budget (roughly 40–70), and
//! once it is spent it ignores reload requests for a while, leaving the
//! widget stale. So a reload is requested only when what the widget shows
//! changes, and at most every [`MIN_RELOAD_INTERVAL`]. The file itself is
//! rewritten on every poll, so the widget's own refresh button and its
//! scheduled refreshes always pick up the latest reading.

#![cfg_attr(not(target_os = "macos"), allow(dead_code))]

use serde::Serialize;
use usage_core::Usage;

/// Must match `PRODUCT_BUNDLE_IDENTIFIER` in macos/widget/project.yml.
const WIDGET_BUNDLE_ID: &str = "io.github.smtodj.claude-usage-widget.widget";

const MIN_RELOAD_INTERVAL: std::time::Duration = std::time::Duration::from_secs(15 * 60);

/// Windows the widget displays (see `shownWindows` in UsageWidget.swift).
const SHOWN_KEYS: [&str; 2] = ["five_hour", "seven_day"];

/// One widget's data. File names and kinds must match UsageWidget.swift.
#[derive(Clone, Copy)]
pub enum Feed {
    Claude = 0,
    Gemini = 1,
}

impl Feed {
    fn file(self) -> &'static str {
        match self {
            Feed::Claude => "usage.json",
            Feed::Gemini => "gemini.json",
        }
    }

    fn kind(self) -> &'static str {
        match self {
            Feed::Claude => "ClaudeUsageWidget",
            Feed::Gemini => "GeminiUsageWidget",
        }
    }
}

#[derive(Serialize, PartialEq)]
struct WidgetData {
    /// Unix seconds.
    updated_at: i64,
    windows: Vec<WidgetWindow>,
    error: Option<String>,
}

#[derive(Serialize, PartialEq)]
struct WidgetWindow {
    key: String,
    label: String,
    remaining_percent: f64,
    /// Unix seconds.
    resets_at: Option<i64>,
}

fn widget_data(usage: Option<&Usage>, error: Option<&str>, now: i64) -> WidgetData {
    WidgetData {
        updated_at: now,
        windows: usage
            .map(|u| {
                u.windows
                    .iter()
                    .map(|w| WidgetWindow {
                        key: w.key.clone(),
                        label: w.label.clone(),
                        remaining_percent: w.remaining_percent,
                        resets_at: w.resets_at.map(|t| t.timestamp()),
                    })
                    .collect()
            })
            .unwrap_or_default(),
        error: error.map(str::to_owned),
    }
}

/// What the widget visibly shows: whole percents and reset times to the
/// minute of the session and weekly windows, and whether there is an error.
fn shown_summary(data: &WidgetData) -> String {
    let mut parts: Vec<String> = data
        .windows
        .iter()
        .filter(|w| SHOWN_KEYS.contains(&w.key.as_str()))
        .map(|w| {
            format!(
                "{}:{}:{}",
                w.key,
                w.remaining_percent.round(),
                w.resets_at.map(|t| t / 60).unwrap_or(-1)
            )
        })
        .collect();
    parts.push(format!("error:{}", data.error.is_some()));
    parts.join(" ")
}

fn should_reload(changed: bool, since_last: Option<std::time::Duration>) -> bool {
    changed && since_last.is_none_or(|d| d >= MIN_RELOAD_INTERVAL)
}

#[cfg(target_os = "macos")]
pub fn publish(feed: Feed, usage: Option<&Usage>, error: Option<&str>) {
    use std::sync::Mutex;

    use std::time::Instant;

    /// Per feed: summary its widget was last asked to redraw with, and when.
    static LAST_RELOAD: Mutex<[Option<(String, Instant)>; 2]> = Mutex::new([None, None]);

    let Some(home) = std::env::var_os("HOME") else {
        return;
    };
    // macOS creates the container the first time the widget runs (for
    // example when it is shown in the widget gallery). Creating it
    // ourselves could confuse the container manager, so wait for it.
    let dir = std::path::PathBuf::from(home)
        .join("Library/Containers")
        .join(WIDGET_BUNDLE_ID)
        .join("Data");
    if !dir.is_dir() {
        return;
    }

    let data = widget_data(usage, error, chrono::Utc::now().timestamp());
    let shown = shown_summary(&data);

    let Ok(json) = serde_json::to_vec(&data) else {
        return;
    };
    let tmp = dir.join(format!("{}.tmp", feed.file()));
    if std::fs::write(&tmp, json).is_err() || std::fs::rename(&tmp, dir.join(feed.file())).is_err()
    {
        return;
    }

    let mut all = LAST_RELOAD.lock().unwrap();
    let last = &mut all[feed as usize];
    let changed = last.as_ref().is_none_or(|(s, _)| *s != shown);
    if should_reload(changed, last.as_ref().map(|(_, at)| at.elapsed())) {
        *last = Some((shown, Instant::now()));
        drop(all);
        let helper = std::env::current_exe()
            .ok()
            .and_then(|exe| exe.parent().map(|d| d.join("widget-reload")));
        if let Some(helper) = helper.filter(|h| h.is_file()) {
            let _ = std::process::Command::new(helper).arg(feed.kind()).status();
        }
    }
}

#[cfg(not(target_os = "macos"))]
pub fn publish(_feed: Feed, _usage: Option<&Usage>, _error: Option<&str>) {}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Utc};
    use usage_core::UsageWindow;

    #[test]
    fn converts_usage_for_the_widget() {
        let reset = Utc.with_ymd_and_hms(2026, 10, 7, 9, 0, 0).unwrap();
        let usage = Usage {
            windows: vec![UsageWindow {
                key: "five_hour".into(),
                label: "5시간 세션".into(),
                used_percent: 18.0,
                remaining_percent: 82.0,
                resets_at: Some(reset),
            }],
            fetched_at: reset,
        };
        let json =
            serde_json::to_value(widget_data(Some(&usage), Some("오류"), 1_700_000_000)).unwrap();
        assert_eq!(
            json,
            serde_json::json!({
                "updated_at": 1_700_000_000,
                "windows": [{
                    "key": "five_hour",
                    "label": "5시간 세션",
                    "remaining_percent": 82.0,
                    "resets_at": reset.timestamp(),
                }],
                "error": "오류",
            })
        );
        let empty = serde_json::to_value(widget_data(None, None, 0)).unwrap();
        assert_eq!(empty["windows"], serde_json::json!([]));
    }

    fn window(key: &str, remaining: f64, resets_at: Option<i64>) -> WidgetWindow {
        WidgetWindow {
            key: key.into(),
            label: key.into(),
            remaining_percent: remaining,
            resets_at,
        }
    }

    #[test]
    fn summary_changes_only_with_what_the_widget_shows() {
        let data = |five: f64, reset: i64, other: f64, error: Option<&str>| WidgetData {
            updated_at: 0,
            windows: vec![
                window("five_hour", five, Some(reset)),
                window("seven_day", 40.0, None),
                window("iguana_necktie", other, None),
            ],
            error: error.map(str::to_owned),
        };
        let base = shown_summary(&data(72.2, 6000, 100.0, None));
        // Sub-percent drift, seconds in the reset time, hidden windows: same.
        assert_eq!(base, shown_summary(&data(71.9, 6030, 50.0, None)));
        // A different whole percent, reset minute or error state: changed.
        assert_ne!(base, shown_summary(&data(70.0, 6000, 100.0, None)));
        assert_ne!(base, shown_summary(&data(72.2, 6060, 100.0, None)));
        assert_ne!(base, shown_summary(&data(72.2, 6000, 100.0, Some("x"))));
    }

    #[test]
    fn reloads_at_most_every_interval() {
        use std::time::Duration;
        assert!(should_reload(true, None));
        assert!(!should_reload(false, None));
        assert!(!should_reload(true, Some(Duration::from_secs(60))));
        assert!(should_reload(true, Some(MIN_RELOAD_INTERVAL)));
    }
}
