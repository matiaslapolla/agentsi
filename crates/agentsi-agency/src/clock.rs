//! Time as `Date.toISOString()` prints it, and the few date rules that the
//! TypeScript code uses.

use std::time::{SystemTime, UNIX_EPOCH};

pub fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64)
}

/// `1970-01-01T00:00:00.000Z` for 0.
pub fn iso(ms: i64) -> String {
    let secs = ms.div_euclid(1000);
    let (y, m, d) = civil(secs.div_euclid(86_400));
    let rest = secs.rem_euclid(86_400);
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}.{:03}Z",
        rest / 3600,
        rest % 3600 / 60,
        rest % 60,
        ms.rem_euclid(1000)
    )
}

/// The civil date of a day count since 1970-01-01 (Howard Hinnant's algorithm).
fn civil(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    (yoe + era * 400 + i64::from(month <= 2), month, day)
}

fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = y - i64::from(m <= 2);
    let era = y.div_euclid(400);
    let yoe = y.rem_euclid(400);
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// The milliseconds since the epoch of an ISO 8601 text such as
/// `2026-10-09T10:00:00.123Z` (the digits of the fraction and the offset are
/// optional). `None` when the text does not parse.
pub fn parse_iso_ms(text: &str) -> Option<i64> {
    let b = text.as_bytes();
    let num = |from: usize, len: usize| -> Option<i64> {
        std::str::from_utf8(b.get(from..from + len)?)
            .ok()?
            .parse()
            .ok()
    };
    let (y, m, d) = (num(0, 4)?, num(5, 2)?, num(8, 2)?);
    if b.get(4) != Some(&b'-') || b.get(7) != Some(&b'-') {
        return None;
    }
    let (mut h, mut mi, mut s, mut ms) = (0, 0, 0, 0);
    let mut rest = 10;
    if b.len() > 10 {
        if !matches!(b[10], b'T' | b' ') {
            return None;
        }
        (h, mi) = (num(11, 2)?, num(14, 2)?);
        rest = 16;
        if b.get(16) == Some(&b':') {
            s = num(17, 2)?;
            rest = 19;
        }
        if b.get(rest) == Some(&b'.') {
            let digits: String = text[rest + 1..]
                .chars()
                .take_while(char::is_ascii_digit)
                .take(3)
                .collect();
            ms = format!("{digits:0<3}").parse().ok()?;
        }
    }
    // A numeric offset (`+01:00`, `+0100` or `+01`) moves the instant to UTC, as `new Date(text)` does.
    let offset = match text.get(rest..).and_then(|t| t.find(['+', '-'])) {
        Some(at) => {
            let at = rest + at;
            let sign = if b[at] == b'+' { 1 } else { -1 };
            let oh = num(at + 1, 2).unwrap_or(0);
            let from = if b.get(at + 3) == Some(&b':') {
                at + 4
            } else {
                at + 3
            };
            sign * (oh * 60 + num(from, 2).unwrap_or(0)) * 60_000
        }
        None => 0,
    };
    Some(((days_from_civil(y, m, d) * 24 + h) * 60 + mi) * 60_000 + s * 1000 + ms - offset)
}

/// True for `YYYY-MM-DD` that names a real day.
pub fn is_date(text: &str) -> bool {
    let b = text.as_bytes();
    if b.len() != 10 || b[4] != b'-' || b[7] != b'-' {
        return false;
    }
    let digits = |r: std::ops::Range<usize>| b[r].iter().all(u8::is_ascii_digit);
    if !(digits(0..4) && digits(5..7) && digits(8..10)) {
        return false;
    }
    let (y, m, d): (i64, i64, i64) = (
        text[0..4].parse().unwrap_or(0),
        text[5..7].parse().unwrap_or(0),
        text[8..10].parse().unwrap_or(0),
    );
    (1..=12).contains(&m) && d >= 1 && civil(days_from_civil(y, m, d)) == (y, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_formats_and_parses_the_same_instants() {
        assert_eq!(iso(0), "1970-01-01T00:00:00.000Z");
        let ms = 1_793_700_000_123;
        assert_eq!(iso(ms), "2026-11-03T10:00:00.123Z");
        assert_eq!(parse_iso_ms("2026-11-03T10:00:00.123Z"), Some(ms));
        assert_eq!(
            parse_iso_ms("2026-11-03T10:00:00.123456Z"),
            Some(ms),
            "microseconds keep the milliseconds"
        );
        assert_eq!(parse_iso_ms("2026-11-03"), Some(ms - 36_000_123));
        assert_eq!(
            parse_iso_ms("2026-11-03T11:00:00.123+01:00"),
            Some(ms),
            "an offset moves the instant to UTC"
        );
        assert_eq!(parse_iso_ms("2026-11-03T05:30:00-04:30"), Some(ms - 123));
        assert_eq!(parse_iso_ms("2026-11-03T15:30:00+0530"), Some(ms - 123));
        assert_eq!(parse_iso_ms("2026-11-03T06:00-04:00"), Some(ms - 123));
        assert_eq!(parse_iso_ms("not a date"), None);
    }

    #[test]
    fn a_date_must_name_a_real_day() {
        assert!(is_date("2024-02-29"));
        assert!(!is_date("2023-02-29"));
        assert!(!is_date("2026-13-40"));
        assert!(!is_date("2026-1-01"));
        assert!(!is_date("2026-01-01T00:00"));
    }
}
