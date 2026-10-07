//! Reads usage through the claude.ai website session, for people who use
//! Claude in the browser or apps rather than Claude Code.
//!
//! claude.ai's own Settings → Usage page calls
//! `GET /api/organizations/{org}/usage`, which returns the same
//! `five_hour` / `seven_day` windows as the OAuth endpoint. It is
//! authenticated with the `sessionKey` cookie set when you log in.

use std::time::Duration;

use chrono::Utc;
use serde::Deserialize;

use crate::{parse_usage, Usage, UsageError};

const BASE: &str = "https://claude.ai/api";
// claude.ai sits behind Cloudflare, which is stricter with unknown clients.
const USER_AGENT: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.0 Safari/605.1.15";

#[derive(Deserialize)]
struct Organization {
    uuid: String,
    #[serde(default)]
    capabilities: Vec<String>,
}

/// Fetches usage with a claude.ai `sessionKey` cookie value.
pub fn fetch_usage_web(session_key: &str) -> Result<Usage, UsageError> {
    let agent = ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(15))
        .try_proxy_from_env(true)
        .build();
    let get = |path: &str| -> Result<String, UsageError> {
        let resp = agent
            .get(&format!("{BASE}{path}"))
            .set("Cookie", &format!("sessionKey={session_key}"))
            .set("Accept", "application/json")
            .set("User-Agent", USER_AGENT)
            .call();
        match resp {
            Ok(r) => r
                .into_string()
                .map_err(|e| UsageError::Network(e.to_string())),
            Err(ureq::Error::Status(401 | 403, _)) => Err(UsageError::SessionExpired),
            Err(ureq::Error::Status(429, _)) => Err(UsageError::RateLimited),
            Err(ureq::Error::Status(code, _)) => Err(UsageError::Http(code)),
            Err(e) => Err(UsageError::Network(e.to_string())),
        }
    };

    let orgs = get("/organizations")?;
    let org = pick_organization(&orgs)?;
    let body = get(&format!("/organizations/{org}/usage"))?;
    parse_usage(&body, Utc::now())
}

/// Picks the organization that holds the chat subscription; personal
/// accounts usually have exactly one.
fn pick_organization(json: &str) -> Result<String, UsageError> {
    let orgs: Vec<Organization> =
        serde_json::from_str(json).map_err(|e| UsageError::Parse(e.to_string()))?;
    orgs.iter()
        .find(|o| o.capabilities.iter().any(|c| c == "chat"))
        .or_else(|| orgs.first())
        .map(|o| o.uuid.clone())
        .ok_or(UsageError::SessionExpired)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefers_chat_organization() {
        let json = r#"[
            {"uuid": "api-org", "capabilities": ["api"]},
            {"uuid": "chat-org", "capabilities": ["chat", "claude_max"]}
        ]"#;
        assert_eq!(pick_organization(json).unwrap(), "chat-org");
    }

    #[test]
    fn falls_back_to_first_organization() {
        let json = r#"[{"uuid": "only"}]"#;
        assert_eq!(pick_organization(json).unwrap(), "only");
    }

    #[test]
    fn no_organization_means_logged_out() {
        assert!(matches!(
            pick_organization("[]"),
            Err(UsageError::SessionExpired)
        ));
    }
}
