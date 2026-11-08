//! Text for numbers and times on the screens. The core sends ISO 8601 UTC strings
//! and plain numbers, and the screens show them as the web app does.

use std::time::{SystemTime, UNIX_EPOCH};

/// Seconds since the epoch of an ISO 8601 UTC string such as `2026-11-02T10:00:00.000Z`.
/// A string that is only a date counts as its midnight.
pub fn unix_secs(iso: &str) -> Option<i64> {
    let num = |s: Option<&str>| s?.parse::<i64>().ok();
    let mut date = iso.get(..10)?.split('-');
    let (y, m, d) = (num(date.next())?, num(date.next())?, num(date.next())?);
    let mut clock = iso.get(11..19).unwrap_or("00:00:00").split(':');
    let (hh, mm, ss) = (num(clock.next())?, num(clock.next())?, num(clock.next())?);
    if !(1..=12).contains(&m) || !(1..=31).contains(&d) {
        return None;
    }
    // Days from the civil date: the algorithm of Howard Hinnant.
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    Some(days * 86_400 + hh * 3600 + mm * 60 + ss)
}

/// The `file://` URL of `file`. Each byte outside the unreserved set and `/` becomes `%XX`,
/// so a space or a `#` in the path stays part of the path.
pub fn file_url(file: &std::path::Path) -> String {
    use std::os::unix::ffi::OsStrExt;
    let mut url = String::from("file://");
    for &b in file.as_os_str().as_bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'/' | b'-' | b'.' | b'_' | b'~') {
            url.push(b as char);
        } else {
            url.push_str(&format!("%{b:02X}"));
        }
    }
    url
}

pub fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

/// `YYYY-MM-DD` of a time in seconds since the epoch, in UTC.
pub fn date_from_secs(secs: i64) -> String {
    // The algorithm of Howard Hinnant, the inverse of `unix_secs`.
    let z = secs.div_euclid(86_400) + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}")
}

/// `YYYY-MM-DD` of an ISO string, or an empty string.
pub fn date_of(iso: &str) -> String {
    iso.get(..10).unwrap_or_default().to_string()
}

/// `3 min ago`, `5 h ago` or `12 d ago`, as `agentsi status` writes it.
pub fn ago(iso: &str, now: i64) -> String {
    let Some(then) = unix_secs(iso) else {
        return "never".into();
    };
    let min = ((now - then).max(0) + 30) / 60;
    if min < 60 {
        format!("{min} min ago")
    } else if min < 48 * 60 {
        format!("{} h ago", (min + 30) / 60)
    } else {
        format!("{} d ago", (min + 720) / 1440)
    }
}

/// `950`, `12.3k`, `4.5M`.
pub fn compact(n: u64) -> String {
    // Round to a tenth first, so that 999,990 reads `1M` and not `1000k`.
    let tenths = |unit: u64| (n + unit / 20) / (unit / 10);
    if n < 1000 {
        n.to_string()
    } else if tenths(1000) < 10_000 {
        trim(format!("{:.1}", tenths(1000) as f64 / 10.)) + "k"
    } else {
        trim(format!("{:.1}", tenths(1_000_000) as f64 / 10.)) + "M"
    }
}

fn trim(s: String) -> String {
    s.strip_suffix(".0").map(String::from).unwrap_or(s)
}

pub fn usd(n: f64) -> String {
    format!("${n:.2}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_url_encodes_a_space_a_hash_and_a_percent_sign() {
        assert_eq!(
            file_url(std::path::Path::new("/home/a b/c#d/100%/docs")),
            "file:///home/a%20b/c%23d/100%25/docs"
        );
    }

    #[test]
    fn seconds_give_their_utc_date() {
        assert_eq!(date_from_secs(0), "1970-01-01");
        assert_eq!(date_from_secs(951_782_400), "2000-02-29");
        assert_eq!(date_from_secs(1_793_700_000), "2026-11-03");
    }

    #[test]
    fn an_iso_string_gives_its_unix_seconds() {
        assert_eq!(unix_secs("1970-01-01T00:00:00.000Z"), Some(0));
        assert_eq!(unix_secs("2000-02-29T00:00:00.005Z"), Some(951_782_400));
        assert_eq!(unix_secs("2026-11-03T10:00:00.123Z"), Some(1_793_700_000));
        assert_eq!(unix_secs("2026-11-03"), Some(1_793_664_000));
        assert_eq!(unix_secs("nope"), None);
        assert_eq!(unix_secs("2026-13-01T00:00:00Z"), None);
    }

    #[test]
    fn a_time_reads_as_minutes_hours_or_days_ago() {
        let now = unix_secs("2026-11-03T12:00:00.000Z").unwrap();
        assert_eq!(ago("2026-11-03T11:57:00.000Z", now), "3 min ago");
        assert_eq!(ago("2026-11-03T07:00:00.000Z", now), "5 h ago");
        assert_eq!(ago("2026-10-22T12:00:00.000Z", now), "12 d ago");
        // A time in the future is "0 min ago", and no time is "never".
        assert_eq!(ago("2026-11-03T13:00:00.000Z", now), "0 min ago");
        assert_eq!(ago("", now), "never");
    }

    #[test]
    fn numbers_are_short() {
        assert_eq!(compact(950), "950");
        assert_eq!(compact(12_340), "12.3k");
        assert_eq!(compact(2_000), "2k");
        assert_eq!(compact(4_500_000), "4.5M");
        assert_eq!(compact(999_949), "999.9k");
        assert_eq!(compact(999_950), "1M");
        assert_eq!(compact(999_999), "1M");
        assert_eq!(compact(1000), "1k");
        assert_eq!(usd(0.125), "$0.12");
        assert_eq!(date_of("2026-11-02T10:00:00.000Z"), "2026-11-02");
        assert_eq!(date_of(""), "");
    }
}
