//! Reads how much of the Claude plan allowance is left.
//!
//! The numbers come from the same endpoint Claude Code's `/usage` command
//! uses (`GET https://api.anthropic.com/api/oauth/usage`). It is
//! authenticated with the OAuth token Claude Code already stores on the
//! machine, so no API key is needed, and reading it does not consume usage.
//! The endpoint is not publicly documented and may change.

mod credentials;
mod format;
mod usage;

pub use credentials::{read_credentials, Credentials, CredentialsSource};
pub use format::{format_reset, short_label};
pub use usage::{fetch_usage, parse_usage, Usage, UsageWindow};

use std::fmt;

#[derive(Debug, thiserror::Error)]
pub enum UsageError {
    #[error(
        "Claude Code 로그인 정보를 찾지 못했어요. 터미널에서 `claude`를 실행해 로그인해 주세요."
    )]
    NotLoggedIn,
    #[error("Claude Code 토큰이 만료됐어요. 터미널에서 `claude`를 한 번 실행하면 갱신돼요.")]
    TokenExpired,
    #[error("인증에 실패했어요(401). 터미널에서 `claude`를 다시 실행해 주세요.")]
    Unauthorized,
    #[error("요청이 너무 많아요(429). 잠시 후 다시 시도할게요.")]
    RateLimited,
    #[error("서버 응답 오류: HTTP {0}")]
    Http(u16),
    #[error("네트워크 오류: {0}")]
    Network(String),
    #[error("응답을 해석하지 못했어요: {0}")]
    Parse(String),
    #[error("로그인 정보를 읽지 못했어요: {0}")]
    Credentials(String),
}

/// Reads credentials and fetches usage in one call.
pub fn current_usage() -> Result<Usage, UsageError> {
    let creds = read_credentials()?;
    fetch_usage(&creds.access_token)
}

impl fmt::Display for CredentialsSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CredentialsSource::Env => write!(f, "CLAUDE_USAGE_TOKEN"),
            CredentialsSource::Keychain => write!(f, "macOS Keychain"),
            CredentialsSource::File(p) => write!(f, "{}", p.display()),
        }
    }
}
