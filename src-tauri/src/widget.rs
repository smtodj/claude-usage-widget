//! Hands the latest reading to the macOS desktop widget.
//!
//! The widget (`macos/widget`, bundled as `Contents/PlugIns/UsageWidget.appex`)
//! is sandboxed and ad-hoc signed, so it can't share an App Group with this
//! app. Instead this app writes `usage.json` straight into the widget's own
//! sandbox container, which the widget sees as its home directory, then runs
//! `widget-reload` to have WidgetKit redraw it.

#![cfg_attr(not(target_os = "macos"), allow(dead_code))]

use serde::Serialize;
use usage_core::Usage;

/// Must match `PRODUCT_BUNDLE_IDENTIFIER` in macos/widget/project.yml.
const WIDGET_BUNDLE_ID: &str = "io.github.smtodj.claude-usage-widget.widget";

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

#[cfg(target_os = "macos")]
pub fn publish(usage: Option<&Usage>, error: Option<&str>) {
    use std::sync::Mutex;

    // What the widget shows, without the timestamp: only redraw on change.
    static LAST_SHOWN: Mutex<Option<String>> = Mutex::new(None);

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
    let shown = serde_json::to_string(&(&data.windows, &data.error)).unwrap_or_default();
    let changed = LAST_SHOWN.lock().unwrap().as_deref() != Some(shown.as_str());

    let Ok(json) = serde_json::to_vec(&data) else {
        return;
    };
    let tmp = dir.join("usage.json.tmp");
    if std::fs::write(&tmp, json).is_err() || std::fs::rename(&tmp, dir.join("usage.json")).is_err()
    {
        return;
    }

    if changed {
        *LAST_SHOWN.lock().unwrap() = Some(shown);
        let helper = std::env::current_exe()
            .ok()
            .and_then(|exe| exe.parent().map(|d| d.join("widget-reload")));
        if let Some(helper) = helper.filter(|h| h.is_file()) {
            let _ = std::process::Command::new(helper).status();
        }
    }
}

#[cfg(not(target_os = "macos"))]
pub fn publish(_usage: Option<&Usage>, _error: Option<&str>) {}

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
}
