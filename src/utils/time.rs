//! Time utilities

use chrono::{DateTime, Duration, Utc};

/// Parse duration string like "1h", "30m", "1d"
pub fn parse_duration(s: &str) -> Option<Duration> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }

    let value = s[..s.len()-1].parse::<i64>().ok()?;

    match s.chars().last()? {
        's' => Some(Duration::seconds(value)),
        'm' => Some(Duration::minutes(value)),
        'h' => Some(Duration::hours(value)),
        'd' => Some(Duration::days(value)),
        'w' => Some(Duration::weeks(value)),
        _ => None,
    }
}

/// Get time range from duration string
pub fn get_time_range_from_duration(duration_str: &str) -> Option<(DateTime<Utc>, DateTime<Utc>)> {
    let duration = parse_duration(duration_str)?;
    let end = Utc::now();
    let start = end - duration;
    Some((start, end))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_duration() {
        assert_eq!(parse_duration("1h"), Some(Duration::hours(1)));
        assert_eq!(parse_duration("30m"), Some(Duration::minutes(30)));
        assert_eq!(parse_duration("1d"), Some(Duration::days(1)));
        assert_eq!(parse_duration("1w"), Some(Duration::weeks(1)));
    }
}
