use chrono::{DateTime, Utc};

/// "2시간 13분 후 초기화", "3일 4시간 후 초기화", or "초기화 시각 미정".
pub fn format_reset(resets_at: Option<DateTime<Utc>>, now: DateTime<Utc>) -> String {
    let Some(at) = resets_at else {
        return "초기화 시각 미정".into();
    };
    let mins = (at - now).num_minutes().max(0);
    let (d, h, m) = (mins / 1440, (mins % 1440) / 60, mins % 60);
    let text = if d > 0 {
        format!("{d}일 {h}시간")
    } else if h > 0 {
        format!("{h}시간 {m}분")
    } else {
        format!("{m}분")
    };
    format!("{text} 후 초기화")
}

/// Compact label for the menu bar title: "5h", "7d", "Opus", ...
pub fn short_label(key: &str) -> &str {
    match key {
        "five_hour" => "5h",
        "seven_day" => "7d",
        "seven_day_opus" => "Opus",
        "seven_day_sonnet" => "Sonnet",
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;

    #[test]
    fn formats_reset_times() {
        let now = Utc::now();
        assert_eq!(format_reset(None, now), "초기화 시각 미정");
        assert_eq!(
            format_reset(Some(now + Duration::minutes(133)), now),
            "2시간 13분 후 초기화"
        );
        assert_eq!(
            format_reset(Some(now + Duration::minutes(3 * 1440 + 4 * 60 + 5)), now),
            "3일 4시간 후 초기화"
        );
        assert_eq!(
            format_reset(Some(now - Duration::minutes(5)), now),
            "0분 후 초기화"
        );
    }
}
