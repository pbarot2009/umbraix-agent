pub mod logging;
pub mod rate_limit;

pub use rate_limit::{Cooldown, RateLimiter};

use std::time::{SystemTime, UNIX_EPOCH};

pub fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or_default()
}

pub fn today() -> i64 {
    now_secs() / 86_400
}

/// Short random uppercase hex identifier (request IDs, error references).
pub fn short_id(len: usize) -> String {
    use rand::Rng;
    const HEX: &[u8] = b"0123456789ABCDEF";
    let mut rng = rand::rng();
    (0..len)
        .map(|_| HEX[rng.random_range(0..HEX.len())] as char)
        .collect()
}

pub fn truncate_chars(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        return s.to_string();
    }
    let kept: String = s.chars().take(max_chars).collect();
    format!("{kept}…")
}

pub fn format_duration(secs: u64) -> String {
    let (d, h, m, s) = (secs / 86_400, secs / 3600 % 24, secs / 60 % 60, secs % 60);
    if d > 0 {
        format!("{d}d {h}h {m}m")
    } else if h > 0 {
        format!("{h}h {m}m")
    } else if m > 0 {
        format!("{m}m {s}s")
    } else {
        format!("{s}s")
    }
}

/// Heuristic: does this token look like a Google API key (`AIza` + 35 chars)?
pub fn looks_like_google_key(token: &str) -> bool {
    let t = token.trim_matches(|c: char| !c.is_ascii_alphanumeric() && c != '-' && c != '_');
    t.len() == 39
        && t.starts_with("AIza")
        && t.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

pub fn contains_google_key(text: &str) -> bool {
    text.split_whitespace().any(looks_like_google_key)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_id_has_len() {
        let id = short_id(6);
        assert_eq!(id.len(), 6);
        assert!(id.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn detects_google_keys() {
        let key = format!("AIza{}", "A".repeat(35));
        assert!(looks_like_google_key(&key));
        assert!(contains_google_key(&format!("here is my key {key} thanks")));
        assert!(!contains_google_key("hello AIza world"));
    }

    #[test]
    fn formats_durations() {
        assert_eq!(format_duration(5), "5s");
        assert_eq!(format_duration(65), "1m 5s");
        assert_eq!(format_duration(3700), "1h 1m");
        assert_eq!(format_duration(90_000), "1d 1h 0m");
    }
}
