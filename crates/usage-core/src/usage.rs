use std::time::Duration;

use chrono::{DateTime, Utc};
use serde::Serialize;
use serde_json::Value;

use crate::UsageError;

const USAGE_URL: &str = "https://api.anthropic.com/api/oauth/usage";
const OAUTH_BETA: &str = "oauth-2025-04-20";

/// One rate-limit window, e.g. the 5-hour session or the 7-day week.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct UsageWindow {
    /// Key as returned by the API: `five_hour`, `seven_day`, `seven_day_opus`, ...
    pub key: String,
    /// Human label, e.g. "5시간 세션".
    pub label: String,
    /// Percent used, 0–100.
    pub used_percent: f64,
    /// Percent left, 0–100.
    pub remaining_percent: f64,
    pub resets_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Usage {
    pub windows: Vec<UsageWindow>,
    pub fetched_at: DateTime<Utc>,
}

impl Usage {
    pub fn window(&self, key: &str) -> Option<&UsageWindow> {
        self.windows.iter().find(|w| w.key == key)
    }
}

pub fn fetch_usage(access_token: &str) -> Result<Usage, UsageError> {
    let agent = ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(15))
        .try_proxy_from_env(true)
        .build();
    let resp = agent
        .get(USAGE_URL)
        .set("Authorization", &format!("Bearer {access_token}"))
        .set("anthropic-beta", OAUTH_BETA)
        .set("Accept", "application/json")
        .set(
            "User-Agent",
            concat!("claude-usage-widget/", env!("CARGO_PKG_VERSION")),
        )
        .call();
    let body = match resp {
        Ok(r) => r
            .into_string()
            .map_err(|e| UsageError::Network(e.to_string()))?,
        Err(ureq::Error::Status(401 | 403, _)) => return Err(UsageError::Unauthorized),
        Err(ureq::Error::Status(429, _)) => return Err(UsageError::RateLimited),
        Err(ureq::Error::Status(code, _)) => return Err(UsageError::Http(code)),
        Err(e) => return Err(UsageError::Network(e.to_string())),
    };
    parse_usage(&body, Utc::now())
}

/// Parses the endpoint's JSON. Any top-level object with a numeric
/// `utilization` is treated as a window, so new windows show up without a
/// code change; known ones are sorted first.
pub fn parse_usage(body: &str, now: DateTime<Utc>) -> Result<Usage, UsageError> {
    let root: Value = serde_json::from_str(body).map_err(|e| UsageError::Parse(e.to_string()))?;
    let obj = root
        .as_object()
        .ok_or_else(|| UsageError::Parse("JSON object가 아니에요".into()))?;

    let mut windows: Vec<UsageWindow> = obj
        .iter()
        .filter_map(|(key, v)| {
            let used = v.get("utilization")?.as_f64()?.clamp(0.0, 100.0);
            let resets_at = v
                .get("resets_at")
                .and_then(Value::as_str)
                .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
                .map(|d| d.with_timezone(&Utc));
            Some(UsageWindow {
                key: key.clone(),
                label: label_for(key),
                used_percent: used,
                remaining_percent: 100.0 - used,
                resets_at,
            })
        })
        .collect();

    if windows.is_empty() {
        return Err(UsageError::Parse("사용량 정보가 없어요".into()));
    }
    windows.sort_by_key(|w| (rank(&w.key), w.key.clone()));
    Ok(Usage {
        windows,
        fetched_at: now,
    })
}

fn rank(key: &str) -> usize {
    match key {
        "five_hour" => 0,
        "seven_day" => 1,
        "seven_day_opus" => 2,
        "seven_day_sonnet" => 3,
        _ => 10,
    }
}

fn label_for(key: &str) -> String {
    match key {
        "five_hour" => "5시간 세션".into(),
        "seven_day" => "주간 (전체 모델)".into(),
        "seven_day_opus" => "주간 (Opus)".into(),
        "seven_day_sonnet" => "주간 (Sonnet)".into(),
        "seven_day_oauth_apps" => "주간 (연동 앱)".into(),
        other => other.replace('_', " "),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"{
        "five_hour": {"utilization": 28.0, "resets_at": "2026-10-07T05:00:00.123456+00:00"},
        "seven_day": {"utilization": 61.5, "resets_at": "2026-10-10T03:00:00+00:00"},
        "seven_day_oauth_apps": null,
        "seven_day_opus": {"utilization": 0.0, "resets_at": null},
        "seven_day_sonnet": {"utilization": 12.0, "resets_at": "2026-10-10T03:00:00+00:00"},
        "extra_usage": {"is_enabled": false, "monthly_limit": null}
    }"#;

    #[test]
    fn parses_known_windows_in_order() {
        let u = parse_usage(SAMPLE, Utc::now()).unwrap();
        let keys: Vec<_> = u.windows.iter().map(|w| w.key.as_str()).collect();
        assert_eq!(
            keys,
            [
                "five_hour",
                "seven_day",
                "seven_day_opus",
                "seven_day_sonnet"
            ]
        );
        let s = u.window("five_hour").unwrap();
        assert_eq!(s.used_percent, 28.0);
        assert_eq!(s.remaining_percent, 72.0);
        assert!(s.resets_at.is_some());
        assert!(u.window("seven_day_opus").unwrap().resets_at.is_none());
    }

    #[test]
    fn clamps_out_of_range_values() {
        let u = parse_usage(r#"{"five_hour":{"utilization":130}}"#, Utc::now()).unwrap();
        assert_eq!(u.windows[0].remaining_percent, 0.0);
    }

    #[test]
    fn rejects_body_without_windows() {
        assert!(parse_usage(r#"{"error":"x"}"#, Utc::now()).is_err());
        assert!(parse_usage("not json", Utc::now()).is_err());
    }
}
