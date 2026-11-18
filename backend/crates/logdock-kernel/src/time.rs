use std::fmt;
use std::time::Duration;

const MICROS: i64 = 1_000_000;
const DAY_SECONDS: i64 = 86_400;

/// A point in time as microseconds since the Unix epoch (UTC).
///
/// Microseconds, because log lines written within one millisecond must keep
/// their order; maps directly onto an SQLite `INTEGER` and a JSON number.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Timestamp(i64);

impl Timestamp {
    pub const fn from_unix_micros(micros: i64) -> Self {
        Self(micros)
    }

    pub const fn from_unix_seconds(seconds: i64) -> Self {
        Self(seconds.saturating_mul(MICROS))
    }

    /// From fractional seconds, as Sentry's payloads carry them.
    pub fn from_unix_seconds_f64(seconds: f64) -> Option<Self> {
        let micros = (seconds * 1_000_000.0).round();
        (micros.is_finite() && micros.abs() < 9.0e18).then_some(Self(micros as i64))
    }

    pub const fn unix_micros(self) -> i64 {
        self.0
    }

    pub const fn unix_seconds(self) -> i64 {
        self.0.div_euclid(MICROS)
    }

    pub fn saturating_add(self, duration: Duration) -> Self {
        Self(self.0.saturating_add(micros(duration)))
    }

    pub fn saturating_sub(self, duration: Duration) -> Self {
        Self(self.0.saturating_sub(micros(duration)))
    }

    /// Time from `earlier` to `self`; zero if `earlier` is actually later.
    pub fn duration_since(self, earlier: Self) -> Duration {
        Duration::from_micros(u64::try_from(self.0.saturating_sub(earlier.0)).unwrap_or(0))
    }

    /// The UTC day this point falls on.
    pub fn day(self) -> Day {
        Day(self.unix_seconds().div_euclid(DAY_SECONDS))
    }

    /// The start of the minute this point falls in, in Unix seconds.
    pub fn minute(self) -> i64 {
        self.unix_seconds().div_euclid(60) * 60
    }

    /// The start of the hour this point falls in, in Unix seconds.
    pub fn hour(self) -> i64 {
        self.unix_seconds().div_euclid(3600) * 3600
    }

    /// The calendar date and time in UTC (proleptic Gregorian calendar).
    pub fn utc(self) -> UtcTime {
        let seconds = self.unix_seconds();
        let (year, month, day) = civil_from_days(seconds.div_euclid(DAY_SECONDS));
        let second_of_day = seconds.rem_euclid(DAY_SECONDS);
        UtcTime {
            year,
            month,
            day,
            hour: (second_of_day / 3600) as u8,
            minute: (second_of_day % 3600 / 60) as u8,
            second: (second_of_day % 60) as u8,
            micros: self.0.rem_euclid(MICROS) as u32,
        }
    }

    /// From a UTC calendar date and time; `None` for one that does not exist.
    pub fn from_utc(time: UtcTime) -> Option<Self> {
        let valid = (1..=12).contains(&time.month)
            && time.day >= 1
            && time.day <= days_in_month(time.year, time.month)
            && time.hour < 24
            && time.minute < 60
            && time.second < 60
            && time.micros < 1_000_000;
        if !valid {
            return None;
        }
        let days = days_from_civil(time.year, time.month, time.day);
        let seconds = days * DAY_SECONDS
            + i64::from(time.hour) * 3600
            + i64::from(time.minute) * 60
            + i64::from(time.second);
        Some(Self(seconds.checked_mul(MICROS)?.checked_add(i64::from(time.micros))?))
    }
}

fn micros(duration: Duration) -> i64 {
    i64::try_from(duration.as_micros()).unwrap_or(i64::MAX)
}

/// A [`Timestamp`] as a UTC calendar date and time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UtcTime {
    pub year: i64,
    pub month: u8,
    pub day: u8,
    pub hour: u8,
    pub minute: u8,
    pub second: u8,
    pub micros: u32,
}

/// RFC 3339 with microseconds, e.g. `2026-11-18T09:15:02.123456Z`.
impl fmt::Display for UtcTime {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}.{:06}Z",
            self.year, self.month, self.day, self.hour, self.minute, self.second, self.micros
        )
    }
}

/// A calendar day in UTC, counted in days since the Unix epoch. Data is kept
/// in one segment per day (docs/architecture.md, 5.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Day(i64);

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0:?} is not a day in the form YYYY-MM-DD")]
pub struct InvalidDay(pub String);

impl Day {
    pub const fn from_days(days: i64) -> Self {
        Self(days)
    }

    pub const fn days(self) -> i64 {
        self.0
    }

    pub fn start(self) -> Timestamp {
        Timestamp::from_unix_seconds(self.0 * DAY_SECONDS)
    }

    /// The first moment of the next day.
    pub fn end(self) -> Timestamp {
        Timestamp::from_unix_seconds((self.0 + 1) * DAY_SECONDS)
    }

    pub fn previous(self) -> Self {
        Self(self.0 - 1)
    }

    pub fn next(self) -> Self {
        Self(self.0 + 1)
    }

    /// Reads `YYYY-MM-DD`.
    pub fn parse(text: &str) -> Result<Self, InvalidDay> {
        let invalid = || InvalidDay(text.to_owned());
        let mut parts = text.split('-');
        let (Some(year), Some(month), Some(day), None) =
            (parts.next(), parts.next(), parts.next(), parts.next())
        else {
            return Err(invalid());
        };
        if year.len() != 4 || month.len() != 2 || day.len() != 2 {
            return Err(invalid());
        }
        let year: i64 = year.parse().map_err(|_| invalid())?;
        let month: u8 = month.parse().map_err(|_| invalid())?;
        let day: u8 = day.parse().map_err(|_| invalid())?;
        if !(1..=12).contains(&month) || day == 0 || day > days_in_month(year, month) {
            return Err(invalid());
        }
        Ok(Self(days_from_civil(year, month, day)))
    }
}

impl fmt::Display for Day {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (year, month, day) = civil_from_days(self.0);
        write!(f, "{year:04}-{month:02}-{day:02}")
    }
}

/// Howard Hinnant's `civil_from_days`.
fn civil_from_days(days: i64) -> (i64, u8, u8) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let day_of_era = z.rem_euclid(146_097);
    let year_of_era = (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let shifted_month = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * shifted_month + 2) / 5 + 1;
    let month = if shifted_month < 10 { shifted_month + 3 } else { shifted_month - 9 };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    (year, month as u8, day as u8)
}

/// Howard Hinnant's `days_from_civil`.
fn days_from_civil(year: i64, month: u8, day: u8) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let year_of_era = year.rem_euclid(400);
    let month = i64::from(month);
    let day_of_year = (153 * (if month > 2 { month - 3 } else { month + 9 }) + 2) / 5 + i64::from(day) - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

fn days_in_month(year: i64, month: u8) -> u8 {
    match month {
        2 if (year % 4 == 0 && year % 100 != 0) || year % 400 == 0 => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

/// The current time. A port, so tests can set it.
pub trait Clock: Send + Sync {
    fn now(&self) -> Timestamp;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_to_and_from_the_calendar() {
        let time = Timestamp::from_unix_micros(1_794_993_302_123_456);
        let utc = time.utc();
        assert_eq!(utc.to_string(), "2026-11-18T09:15:02.123456Z");
        assert_eq!(Timestamp::from_utc(utc), Some(time));
        assert_eq!(time.day().to_string(), "2026-11-18");
        assert_eq!(Timestamp::from_unix_seconds(-1).utc().to_string(), "1969-12-31T23:59:59.000000Z");
    }

    #[test]
    fn rejects_dates_that_do_not_exist() {
        let feb = |day| UtcTime { year: 2026, month: 2, day, hour: 0, minute: 0, second: 0, micros: 0 };
        assert!(Timestamp::from_utc(feb(28)).is_some());
        assert!(Timestamp::from_utc(feb(29)).is_none());
        let leap = UtcTime { year: 2028, ..feb(29) };
        assert!(Timestamp::from_utc(leap).is_some());
    }

    #[test]
    fn days_parse_print_and_bound_their_time() {
        let day = Day::parse("2026-11-18").unwrap();
        assert_eq!(day.to_string(), "2026-11-18");
        assert_eq!(day.start().day(), day);
        assert_eq!(day.end().day(), day.next());
        assert_eq!(day.next().previous(), day);
        assert_eq!(Day::parse("2028-02-29").unwrap().to_string(), "2028-02-29");
        for bad in ["2026-02-29", "2026-13-01", "26-11-18", "2026-11-18-1", "2026-1-18", "x"] {
            assert!(Day::parse(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn rounds_to_minutes_hours_and_measures_durations() {
        let time = Timestamp::from_unix_seconds(3_725);
        assert_eq!(time.minute(), 3_720);
        assert_eq!(time.hour(), 3_600);
        let later = time.saturating_add(Duration::from_millis(1500));
        assert_eq!(later.duration_since(time), Duration::from_millis(1500));
        assert_eq!(time.duration_since(later), Duration::ZERO);
        assert_eq!(later.saturating_sub(Duration::from_millis(1500)), time);
        assert_eq!(Timestamp::from_unix_seconds_f64(1.5), Some(Timestamp::from_unix_micros(1_500_000)));
        assert_eq!(Timestamp::from_unix_seconds_f64(f64::NAN), None);
    }
}
