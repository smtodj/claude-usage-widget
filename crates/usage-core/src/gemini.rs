//! Reads the Gemini app's (gemini.google.com) usage limits.
//!
//! Gemini's Settings → Usage limits page calls the private `jSf9Qc` RPC on
//! `/_/BardChatUi/data/batchexecute`. The request has to carry the user's
//! Google session, so the app sends it from a webview signed in to Gemini
//! (see [`GEMINI_FETCH_JS`]); this module only turns the response into a
//! [`Usage`].
//!
//! The RPC's payload is `[2, [[limit, used, type, [[reset_secs, nanos]]], …], …]`
//! where type 1 is the rolling 5-hour window and type 2 the weekly one.
//! Like the Claude endpoints it is undocumented and may change, so anything
//! that doesn't fit that shape is an error rather than a guessed number.

use chrono::{DateTime, Utc};
use serde_json::Value;

use crate::{Usage, UsageError, UsageWindow};

pub const GEMINI_USAGE_RPC: &str = "jSf9Qc";

/// Page script that fetches the usage RPC from a gemini.google.com page.
///
/// `__ID__` is replaced with a request id and `__FRESH__` with `true` to
/// skip the tokens embedded in the page (they can go stale on a page left
/// open for long). The outcome lands in `window.__claudeUsageGemini` as
/// `{id, text}`, `text` being `OK:<response body>`, `E:login`,
/// `E:http:<status>` or `E:net:<message>`.
pub const GEMINI_FETCH_JS: &str = r#"
(async () => {
  const id = __ID__;
  const done = (text) => { window.__claudeUsageGemini = { id, text }; };
  const origin = "https://gemini.google.com";
  try {
    if (location.host !== "gemini.google.com") return done("E:login");
    let at, bl;
    const g = window.WIZ_global_data;
    if (!__FRESH__ && g) { at = g.SNlM0e; bl = g.cfb2h; }
    if (!at || !bl) {
      const html = await (await fetch(origin + "/app", { credentials: "include" })).text();
      at = (/"SNlM0e":"([^"]+)"/.exec(html) || [])[1];
      bl = (/"cfb2h":"([^"]+)"/.exec(html) || [])[1];
    }
    // No XSRF token means nobody is signed in.
    if (!at || !bl) return done("E:login");
    const body = new URLSearchParams({
      "f.req": JSON.stringify([[["jSf9Qc", "[]", null, "generic"]]]),
      at,
    });
    const res = await fetch(
      origin + "/_/BardChatUi/data/batchexecute?rpcids=jSf9Qc&source-path=%2Fusage&bl=" +
        encodeURIComponent(bl) + "&rt=c",
      {
        method: "POST",
        credentials: "include",
        headers: { "Content-Type": "application/x-www-form-urlencoded;charset=UTF-8" },
        body,
      },
    );
    if (res.status === 401 || res.status === 403) return done("E:login");
    if (!res.ok) return done("E:http:" + res.status);
    done("OK:" + (await res.text()));
  } catch (e) {
    done("E:net:" + e);
  }
})();
"#;

/// Turns the page script's outcome (see [`GEMINI_FETCH_JS`]) into usage.
pub fn parse_gemini_result(text: &str, now: DateTime<Utc>) -> Result<Usage, UsageError> {
    if let Some(body) = text.strip_prefix("OK:") {
        return parse_gemini_usage(body, now);
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

/// Parses a `batchexecute` response body for the usage RPC.
pub fn parse_gemini_usage(body: &str, now: DateTime<Utc>) -> Result<Usage, UsageError> {
    let row = rpc_row(body).ok_or_else(|| format_changed("응답에 사용량 항목이 없어요"))?;
    // A row without a payload is how the RPC reports a missing or expired
    // session (error code 7) while answering HTTP 200.
    let Some(payload) = row.get(2).and_then(Value::as_str) else {
        return Err(UsageError::GeminiLoggedOut);
    };
    let payload: Value =
        serde_json::from_str(payload).map_err(|e| format_changed(&e.to_string()))?;
    let entries = payload
        .get(1)
        .and_then(Value::as_array)
        .ok_or_else(|| format_changed("사용량 목록이 없어요"))?;

    let mut windows: Vec<UsageWindow> = entries.iter().filter_map(read_window).collect();
    if windows.is_empty() {
        return Err(format_changed("알아볼 수 있는 한도가 없어요"));
    }
    windows.sort_by_key(|w| w.key != "five_hour");
    Ok(Usage {
        windows,
        fetched_at: now,
    })
}

/// The `["wrb.fr", "jSf9Qc", payload, …]` row. The body starts with the
/// `)]}'` guard and, with `rt=c`, alternates length lines and JSON lines.
fn rpc_row(body: &str) -> Option<Vec<Value>> {
    body.lines()
        .filter_map(|line| serde_json::from_str::<Vec<Value>>(line.trim()).ok())
        .flatten()
        .filter_map(|row| match row {
            Value::Array(row) => Some(row),
            _ => None,
        })
        .find(|row| {
            row.first().and_then(Value::as_str) == Some("wrb.fr")
                && row.get(1).and_then(Value::as_str) == Some(GEMINI_USAGE_RPC)
        })
}

/// `[limit, used, type, [[reset_secs, nanos]]]`
fn read_window(entry: &Value) -> Option<UsageWindow> {
    let limit = entry.get(0)?.as_f64()?;
    let used = entry.get(1)?.as_f64()?;
    if limit <= 0.0 || used < 0.0 {
        return None;
    }
    let (key, label) = match entry.get(2)?.as_i64()? {
        1 => ("five_hour", "5시간 세션"),
        2 => ("seven_day", "주간"),
        _ => return None,
    };
    let used_percent = (used / limit * 100.0).clamp(0.0, 100.0);
    let resets_at = entry
        .get(3)
        .and_then(|r| r.get(0))
        .and_then(|r| r.get(0))
        .and_then(Value::as_i64)
        .and_then(|secs| DateTime::from_timestamp(secs, 0));
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

    // A real response shape, captured by AIQuotaTool (MIT).
    const RAW: &str = concat!(
        ")]}'\n\n188\n",
        r#"[["wrb.fr","jSf9Qc","[2,[[48384,12096,2,[[1790777593,651754000]]],[2400,600,1,[[1790230393,651662000]]]],false]",null,null,null,"generic"],["di",222]]"#,
        "\n25\n",
        r#"[["e",4,null,null,224]]"#,
        "\n"
    );

    #[test]
    fn parses_session_and_weekly_limits() {
        let u = parse_gemini_usage(RAW, Utc::now()).unwrap();
        let keys: Vec<_> = u.windows.iter().map(|w| w.key.as_str()).collect();
        assert_eq!(keys, ["five_hour", "seven_day"]);
        let session = u.window("five_hour").unwrap();
        assert_eq!(session.used_percent, 25.0);
        assert_eq!(session.remaining_percent, 75.0);
        assert_eq!(session.resets_at.unwrap().timestamp(), 1_790_230_393);
        let week = u.window("seven_day").unwrap();
        assert_eq!(week.remaining_percent, 75.0);
        assert_eq!(week.resets_at.unwrap().timestamp(), 1_790_777_593);
    }

    #[test]
    fn payload_less_row_means_signed_out() {
        let body = ")]}'\n\n60\n[[\"wrb.fr\",\"jSf9Qc\",null,null,null,[7],\"generic\"]]\n";
        assert!(matches!(
            parse_gemini_usage(body, Utc::now()),
            Err(UsageError::GeminiLoggedOut)
        ));
    }

    #[test]
    fn rejects_unknown_shapes() {
        for payload in ["[2,\"x\"]", "[2,[[0,0,1,[]]]]", "[2,[[10,0,9,[]]]]", "[]"] {
            let body = format!(
                ")]}}'\n{}\n",
                serde_json::json!([["wrb.fr", "jSf9Qc", payload, null, null, null, "generic"]])
            );
            assert!(
                matches!(
                    parse_gemini_usage(&body, Utc::now()),
                    Err(UsageError::Parse(_))
                ),
                "{payload}"
            );
        }
        assert!(matches!(
            parse_gemini_usage("<html>", Utc::now()),
            Err(UsageError::Parse(_))
        ));
    }

    #[test]
    fn maps_page_script_outcomes() {
        let now = Utc::now();
        assert!(parse_gemini_result(&format!("OK:{RAW}"), now).is_ok());
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
