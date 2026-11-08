//! Identity and digest spellings shared by the validation records.

use super::jcs;

/// `[A-Za-z0-9][A-Za-z0-9._:-]{0,127}` (contract §3.1 `Id`).
pub fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 128
        && id.starts_with(|c: char| c.is_ascii_alphanumeric())
        && id.bytes().all(|b| b.is_ascii_alphanumeric() || b"._:-".contains(&b))
}

/// `sha256:` followed by 64 lowercase hex digits.
pub fn valid_digest(digest: &str) -> bool {
    digest.strip_prefix("sha256:").is_some_and(|hex| hex.len() == 64 && hex.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)))
}

/// SHA-256 of `bytes` in wire spelling.
pub fn digest(bytes: &[u8]) -> String {
    solidcraft_io::candidate::digest(bytes)
}

/// SHA-256 of the JCS encoding of `value`.
pub fn value_digest(value: &serde_json::Value) -> Result<String, super::ValidationError> {
    Ok(digest(&jcs::canonical_json(value)?))
}

/// Unix milliseconds as the contract's `Timestamp` (UTC RFC 3339 with milliseconds).
pub fn timestamp(ms: u64) -> String {
    let millis = ms % 1000;
    let seconds = ms / 1000;
    let days = i64::try_from(seconds / 86_400).unwrap_or(i64::MAX / 2);
    let day_seconds = seconds % 86_400;
    let (year, month, day) = civil_from_days(days);
    let (hour, minute, second) = (day_seconds / 3600, (day_seconds / 60) % 60, day_seconds % 60);
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}.{millis:03}Z")
}

/// Days since 1970-01-01 to a civil date (Howard Hinnant's algorithm).
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if month <= 2 { year + 1 } else { year }, month, day)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timestamps_are_utc_with_milliseconds() {
        assert_eq!(timestamp(0), "1970-01-01T00:00:00.000Z");
        assert_eq!(timestamp(1_760_000_123_456), "2025-10-09T08:55:23.456Z");
    }
}
