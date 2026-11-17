//! Wall-clock time for the logs, the records and the panels: milliseconds since 1970, shown in the computer's own time
//! zone, and the RFC 3339 text the records keep. The records compare that text as text (the store drops events
//! `time < ?`), so it keeps the one form they have always had: UTC with `+00:00`, a fraction only when there is one.
use jiff::{Timestamp, tz::TimeZone};

/// Now, in milliseconds since 1970.
pub fn now_ms() -> i64 { Timestamp::now().as_millisecond() }

/// The moment in the computer's time zone, in `strftime` form (`%m-%d %H:%M`); empty for a moment out of range.
pub fn local(ms: i64, format: &str) -> String {
    Timestamp::from_millisecond(ms).map(|t| t.to_zoned(TimeZone::system()).strftime(format).to_string()).unwrap_or_default()
}

/// Now in the computer's time zone, in `strftime` form.
pub fn local_now(format: &str) -> String { local(now_ms(), format) }

/// The day of the week in the computer's time zone, 0 for Monday to 6 for Sunday.
pub fn weekday(ms: i64) -> Option<usize> {
    Timestamp::from_millisecond(ms).ok().map(|t| t.to_zoned(TimeZone::system()).weekday().to_monday_zero_offset() as usize)
}

/// The moment as the records write it: `2026-11-17T11:03:28.526+00:00`, without a fraction on a whole second.
pub fn rfc3339(ms: i64) -> String {
    let Ok(t) = Timestamp::from_millisecond(ms) else { return String::new() };
    let fraction = if ms.rem_euclid(1000) == 0 { String::new() } else { format!(".{:03}", ms.rem_euclid(1000)) };
    format!("{}{fraction}+00:00", t.strftime("%Y-%m-%dT%H:%M:%S"))
}

/// An RFC 3339 moment (any offset) in milliseconds since 1970.
pub fn parse_ms(text: &str) -> Option<i64> { text.parse::<Timestamp>().ok().map(|t| t.as_millisecond()) }

/// A local date and time in milliseconds since 1970, for tests and a person's input.
pub fn local_ms(year: i16, month: i8, day: i8, hour: i8, minute: i8) -> Option<i64> {
    jiff::civil::datetime(year, month, day, hour, minute, 0, 0).to_zoned(TimeZone::system()).ok().map(|z| z.timestamp().as_millisecond())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_records_keep_their_form() {
        assert_eq!(rfc3339(1_794_913_408_526), "2026-11-17T11:03:28.526+00:00");
        assert_eq!(rfc3339(1_794_913_408_000), "2026-11-17T11:03:28+00:00", "no fraction on a whole second");
        assert_eq!(rfc3339(1_794_913_408_050), "2026-11-17T11:03:28.050+00:00");
        assert_eq!(rfc3339(0), "1970-01-01T00:00:00+00:00");
        assert_eq!(parse_ms("2026-11-17T11:03:28.526+00:00"), Some(1_794_913_408_526));
        assert_eq!(parse_ms("2026-11-17T20:03:28.526+09:00"), Some(1_794_913_408_526), "another offset");
        assert_eq!(parse_ms("2026-11-17T11:03:28.526Z"), Some(1_794_913_408_526));
        assert_eq!(parse_ms("nothing"), None);
        assert_eq!(local(i64::MAX, "%H:%M"), "", "out of range");
    }

}

