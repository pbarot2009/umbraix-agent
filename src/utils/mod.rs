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

/// Substring scan: finds `AIza` + 35 key chars ANYWHERE in the text.
///
/// The old version only split on whitespace, so `key=AIza...`, `"AIza..."`,
/// `` `AIza...` `` or `AIza...,` slipped through. This scans every `AIza`
/// occurrence instead.
pub fn contains_google_key(text: &str) -> bool {
    find_google_key(text).is_some()
}

fn find_google_key(text: &str) -> Option<(usize, usize)> {
    let bytes = text.as_bytes();
    let mut i = 0;
    while i + 39 <= bytes.len() {
        if &bytes[i..i + 4] == b"AIza" {
            let end = i + 39;
            // Byte-level boundary + charset check: avoids O(n^2) char walks
            // and stays panic-free on multibyte text by verifying the slice
            // is a valid char boundary before indexing as &str.
            if text.is_char_boundary(i)
                && text.is_char_boundary(end)
                && text.get(i..end).is_some_and(|s| {
                    s.bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
                })
            {
                // Require a non-key-char (or string edge) on both sides so we
                // don't match a 50-char token that merely contains AIza.
                let before_ok = if i == 0 {
                    true
                } else {
                    // Previous byte: ASCII key chars are single-byte, so a
                    // multibyte char boundary implies non-key-char safely.
                    !bytes[i - 1].is_ascii_alphanumeric()
                        && bytes[i - 1] != b'-'
                        && bytes[i - 1] != b'_'
                };
                let after_ok = if end == bytes.len() {
                    true
                } else {
                    !bytes[end].is_ascii_alphanumeric() && bytes[end] != b'-' && bytes[end] != b'_'
                };
                if before_ok && after_ok {
                    return Some((i, end));
                }
            }
        }
        // Advance by char, not byte, to stay on UTF-8 boundaries.
        i += text[i..].chars().next().map(|c| c.len_utf8()).unwrap_or(1);
    }
    None
}

/// Redaction marker used everywhere a key might surface.
pub const REDACTED_KEY: &str = "[REDACTED_API_KEY]";

/// Replace every embedded Google API key with `[REDACTED_API_KEY]`.
/// Returns `(cleaned, found_any)`.
pub fn scrub_google_keys(text: &str) -> (String, bool) {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    let mut found = false;
    while let Some((s, e)) = find_google_key(rest) {
        found = true;
        out.push_str(&rest[..s]);
        out.push_str(REDACTED_KEY);
        rest = &rest[e..];
    }
    out.push_str(rest);
    (out, found)
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
    fn detects_embedded_keys() {
        let key = format!("AIza{}", "B".repeat(35));
        assert!(contains_google_key(&format!("key={key}")));
        assert!(contains_google_key(&format!("`{key}`")));
        assert!(contains_google_key(&format!("\"{key}\",")));
        assert!(contains_google_key(&format!("my key is:{key}.")));
        assert!(!contains_google_key("AIza short"));
        assert!(!contains_google_key("no keys here at all"));
    }

    #[test]
    fn scrubs_keys() {
        let key = format!("AIza{}", "C".repeat(35));
        let (cleaned, found) = scrub_google_keys(&format!("a {key} b {key} c"));
        assert!(found);
        assert!(!contains_google_key(&cleaned));
        assert_eq!(cleaned.matches(REDACTED_KEY).count(), 2);
        let (same, none) = scrub_google_keys("nothing to hide");
        assert!(!none);
        assert_eq!(same, "nothing to hide");
    }

    #[test]
    fn formats_durations() {
        assert_eq!(format_duration(5), "5s");
        assert_eq!(format_duration(65), "1m 5s");
        assert_eq!(format_duration(3700), "1h 1m");
        assert_eq!(format_duration(90_000), "1d 1h 0m");
    }
}
