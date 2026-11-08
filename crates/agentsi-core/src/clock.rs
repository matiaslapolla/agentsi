//! The time as an ISO 8601 string in UTC, like `Date.toISOString()`.

use std::time::{SystemTime, UNIX_EPOCH};

pub fn iso_now() -> String {
    let d = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    iso(d.as_secs() as i64, d.subsec_millis())
}

fn iso(secs: i64, millis: u32) -> String {
    let days = secs.div_euclid(86_400);
    let rest = secs.rem_euclid(86_400);
    // Civil date from days since 1970-01-01 (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}.{millis:03}Z",
        rest / 3600,
        rest % 3600 / 60,
        rest % 60
    )
}

#[cfg(test)]
mod tests {
    use super::iso;

    #[test]
    fn it_formats_known_instants() {
        assert_eq!(iso(0, 0), "1970-01-01T00:00:00.000Z");
        assert_eq!(iso(951_782_400, 5), "2000-02-29T00:00:00.005Z");
        assert_eq!(iso(1_793_700_000, 123), "2026-11-03T10:00:00.123Z");
    }
}
