//! Reads the Gemini app's (gemini.google.com) usage limits.
//!
//! Gemini's Settings → Usage limits page (`/usage`) ships its numbers inside
//! the page itself, in `<script type="application/json" data-bg3-relay-preload>`
//! blocks. One of the strings in there holds a `getUsageInfo` object:
//!
//! ```json
//! {"subscriptionTier":"TIER1","pools":[
//!   {"poolType":"GXU_WEEKLY","percentageUsed":0.038,"gxu":{"windowRefreshesAt":"2026-10-20T04:38:38Z"}},
//!   {"poolType":"GXU_FIVE_HOURLY","percentageUsed":0.23,"gxu":{"windowRefreshesAt":"2026-10-15T05:38:38Z"}}]}
//! ```
//!
//! `percentageUsed` is a fraction and, protobuf style, left out when zero.
//! The page has to be loaded with the user's Google session, so the app
//! fetches it from a webview signed in to Gemini (see [`GEMINI_FETCH_JS`]);
//! this module only turns those blocks into a [`Usage`]. Like the Claude
//! endpoints this is undocumented and may change, so anything that doesn't
//! fit that shape is an error rather than a guessed number.

use chrono::{DateTime, Utc};
use serde_json::Value;

use crate::{Usage, UsageError, UsageWindow};

/// Page script that loads the usage page from a gemini.google.com page.
///
/// `__ID__` is replaced with a request id. The outcome lands in
/// `window.__claudeUsageGemini` as `{id, text}`, `text` being `OK:<JSON array
/// of the page's preload blocks>`, `E:login`, `E:http:<status>` or
/// `E:net:<message>`.
pub const GEMINI_FETCH_JS: &str = r#"
(async () => {
  const id = __ID__;
  const done = (text) => { window.__claudeUsageGemini = { id, text }; };
  try {
    if (location.host !== "gemini.google.com") return done("E:login");
    const res = await fetch("https://gemini.google.com/usage?hl=ko", { credentials: "include" });
    // Signed-out visitors get sent to Google's sign-in.
    if (new URL(res.url).host !== "gemini.google.com") return done("E:login");
    if (res.status === 401 || res.status === 403) return done("E:login");
    if (!res.ok) return done("E:http:" + res.status);
    const html = await res.text();
    const blocks = [];
    const re = /<script\b[^>]*\bdata-bg3-relay-preload\b[^>]*>([\s\S]*?)<\/script>/g;
    for (let m; (m = re.exec(html)); ) blocks.push(m[1]);
    // No XSRF token on the page means nobody is signed in.
    const g = window.WIZ_global_data;
    if (!blocks.length && !(g && g.SNlM0e)) return done("E:login");
    done("OK:" + JSON.stringify(blocks));
  } catch (e) {
    done("E:net:" + e);
  }
})();
"#;

/// Turns the page script's outcome (see [`GEMINI_FETCH_JS`]) into usage.
pub fn parse_gemini_result(text: &str, now: DateTime<Utc>) -> Result<Usage, UsageError> {
    if let Some(blocks) = text.strip_prefix("OK:") {
        let blocks: Vec<String> =
            serde_json::from_str(blocks).map_err(|e| format_changed(&e.to_string()))?;
        return parse_gemini_usage(&blocks, now);
    }
    let err = text.strip_prefix("E:").unwrap_or(text);
    if err == "login" {
        return Err(UsageError::GeminiLoggedOut);
    }
    if let Some(code) = err.strip_prefix("http:") {
        return Err(match code.parse() {
            Ok(429) => UsageError::RateLimited,
            Ok(code) => UsageError::Http(code),
            Err(_) => UsageError::Parse(err.into()),
        });
    }
    Err(UsageError::Network(
        err.strip_prefix("net:").unwrap_or(err).into(),
    ))
}

/// Parses the usage page's preload blocks (each one the text of a
/// `data-bg3-relay-preload` script).
pub fn parse_gemini_usage(blocks: &[String], now: DateTime<Utc>) -> Result<Usage, UsageError> {
    let info = blocks
        .iter()
        .filter_map(|b| serde_json::from_str::<Value>(b).ok())
        .find_map(|v| find_usage_info(&v))
        .ok_or_else(|| format_changed("페이지에 사용량 정보가 없어요"))?;
    let pools = info
        .get("pools")
        .and_then(Value::as_array)
        .ok_or_else(|| format_changed("사용량 목록이 없어요"))?;

    let mut windows: Vec<UsageWindow> = pools.iter().filter_map(read_window).collect();
    if windows.is_empty() {
        return Err(format_changed("알아볼 수 있는 한도가 없어요"));
    }
    windows.sort_by_key(|w| w.key != "five_hour");
    Ok(Usage {
        windows,
        fetched_at: now,
    })
}

const USAGE_KEY: &str = "\"getUsageInfo\":";

/// The `getUsageInfo` object. The preload blocks nest it in JSON encoded as
/// strings (`{"events":[{"chunk":"[{\"data\":…"}]}`), so this looks through
/// every string and reads the value that follows the key.
fn find_usage_info(v: &Value) -> Option<Value> {
    match v {
        Value::Object(map) => map
            .get("getUsageInfo")
            .filter(|info| info.is_object())
            .cloned()
            .or_else(|| map.values().find_map(find_usage_info)),
        Value::Array(items) => items.iter().find_map(find_usage_info),
        Value::String(s) => {
            let start = s.find(USAGE_KEY)? + USAGE_KEY.len();
            serde_json::Deserializer::from_str(&s[start..])
                .into_iter::<Value>()
                .next()?
                .ok()
                .filter(Value::is_object)
        }
        _ => None,
    }
}

/// `{"poolType", "percentageUsed", "gxu": {"windowRefreshesAt"}}`
fn read_window(pool: &Value) -> Option<UsageWindow> {
    let (key, label) = match pool.get("poolType")?.as_str()? {
        "GXU_FIVE_HOURLY" => ("five_hour", "5시간 세션"),
        "GXU_WEEKLY" => ("seven_day", "주간"),
        _ => return None,
    };
    let used = match pool.get("percentageUsed") {
        None | Some(Value::Null) => 0.0,
        Some(v) => v.as_f64()?,
    };
    let used_percent = (used * 100.0).clamp(0.0, 100.0);
    let resets_at = pool
        .get("gxu")
        .and_then(|g| g.get("windowRefreshesAt"))
        .and_then(Value::as_str)
        .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
        .map(|t| t.with_timezone(&Utc));
    Some(UsageWindow {
        key: key.into(),
        label: label.into(),
        used_percent,
        remaining_percent: 100.0 - used_percent,
        resets_at,
    })
}

fn format_changed(detail: &str) -> UsageError {
    UsageError::Parse(format!("Gemini 응답 형식이 바뀐 것 같아요 ({detail})"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The preload block as the usage page ships it (trimmed after the
    /// usage object, which the parser doesn't need).
    const PAGE_CHUNK: &str = concat!(
        r#"[{"data":{"viewer":{"rpcs":{"getUserStatus":{"featureGates":["CAN_ENROLL_IN_AUREUS"]},"#,
        r#""getUsageInfo":{"subscriptionTier":"TIER1","pools":["#,
        r#"{"poolType":"GXU_WEEKLY","percentageUsed":0.03802563,"gxu":{"windowRefreshesAt":"2026-10-20T04:38:38.164214Z"}},"#,
        r#"{"poolType":"GXU_FIVE_HOURLY","percentageUsed":0.23,"gxu":{"windowRefreshesAt":"2026-10-15T05:38:38.164073Z"}}],"#,
        r#""advancedSettings":null,"useOverageAiCredits":false},"id":"ViewerRpcs__viewer_rpcs_v1","#
    );

    fn block(chunk: &str) -> String {
        serde_json::json!({"events": [{"id": "usage:0", "kind": "chunk", "chunk": chunk}]})
            .to_string()
    }

    #[test]
    fn parses_session_and_weekly_limits() {
        let u = parse_gemini_usage(&[block(PAGE_CHUNK)], Utc::now()).unwrap();
        let keys: Vec<_> = u.windows.iter().map(|w| w.key.as_str()).collect();
        assert_eq!(keys, ["five_hour", "seven_day"]);
        let session = u.window("five_hour").unwrap();
        assert!((session.used_percent - 23.0).abs() < 1e-9);
        assert!((session.remaining_percent - 77.0).abs() < 1e-9);
        assert_eq!(session.resets_at.unwrap().timestamp(), 1_792_042_718);
        let week = u.window("seven_day").unwrap();
        assert!((week.used_percent - 3.802563).abs() < 1e-6);
        assert_eq!(week.resets_at.unwrap().timestamp(), 1_792_471_118);
    }

    #[test]
    fn missing_percentage_means_unused() {
        let chunk = r#"[{"getUsageInfo":{"pools":[{"poolType":"GXU_FIVE_HOURLY","gxu":{}}]}}]"#;
        let u = parse_gemini_usage(&[block(chunk)], Utc::now()).unwrap();
        let session = u.window("five_hour").unwrap();
        assert_eq!(session.remaining_percent, 100.0);
        assert!(session.resets_at.is_none());
    }

    #[test]
    fn skips_unrelated_blocks() {
        let blocks = ["not json".to_string(), "{}".to_string(), block(PAGE_CHUNK)];
        assert!(parse_gemini_usage(&blocks, Utc::now()).is_ok());
    }

    #[test]
    fn rejects_unknown_shapes() {
        for chunk in [
            r#"[{"data":{}}]"#,
            r#"{"getUsageInfo":{"pools":"x"}}"#,
            r#"{"getUsageInfo":{"pools":[{"poolType":"OTHER","percentageUsed":0.5}]}}"#,
            r#"{"getUsageInfo":{"pools":[{"poolType":"GXU_WEEKLY","percentageUsed":"x"}]}}"#,
        ] {
            assert!(
                matches!(
                    parse_gemini_usage(&[block(chunk)], Utc::now()),
                    Err(UsageError::Parse(_))
                ),
                "{chunk}"
            );
        }
        assert!(matches!(
            parse_gemini_usage(&[], Utc::now()),
            Err(UsageError::Parse(_))
        ));
    }

    #[test]
    fn maps_page_script_outcomes() {
        let now = Utc::now();
        let ok = format!("OK:{}", serde_json::json!([block(PAGE_CHUNK)]));
        assert!(parse_gemini_result(&ok, now).is_ok());
        assert!(matches!(
            parse_gemini_result("OK:<html>", now),
            Err(UsageError::Parse(_))
        ));
        assert!(matches!(
            parse_gemini_result("E:login", now),
            Err(UsageError::GeminiLoggedOut)
        ));
        assert!(matches!(
            parse_gemini_result("E:http:500", now),
            Err(UsageError::Http(500))
        ));
        assert!(matches!(
            parse_gemini_result("E:http:429", now),
            Err(UsageError::RateLimited)
        ));
        assert!(matches!(
            parse_gemini_result("E:net:TypeError: Load failed", now),
            Err(UsageError::Network(m)) if m == "TypeError: Load failed"
        ));
    }
}
