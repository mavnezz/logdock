//! When a task should run: a crontab as Laravel's scheduler writes it, or an
//! interval.

use logdock_kernel::{Day, Timestamp};

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0:?} is no schedule Logdock understands")]
pub struct InvalidSchedule(pub String);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IntervalUnit {
    Minute,
    Hour,
    Day,
    Week,
    Month,
    Year,
}

impl IntervalUnit {
    fn seconds(self) -> i64 {
        match self {
            IntervalUnit::Minute => 60,
            IntervalUnit::Hour => 3600,
            IntervalUnit::Day => 86_400,
            IntervalUnit::Week => 7 * 86_400,
            // Near enough for a missed run.
            IntervalUnit::Month => 30 * 86_400,
            IntervalUnit::Year => 365 * 86_400,
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        Some(match text.trim_end_matches('s') {
            "minute" => IntervalUnit::Minute,
            "hour" => IntervalUnit::Hour,
            "day" => IntervalUnit::Day,
            "week" => IntervalUnit::Week,
            "month" => IntervalUnit::Month,
            "year" => IntervalUnit::Year,
            _ => return None,
        })
    }

    pub fn as_str(self) -> &'static str {
        match self {
            IntervalUnit::Minute => "minute",
            IntervalUnit::Hour => "hour",
            IntervalUnit::Day => "day",
            IntervalUnit::Week => "week",
            IntervalUnit::Month => "month",
            IntervalUnit::Year => "year",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Schedule {
    Crontab(Crontab),
    Interval { every: u32, unit: IntervalUnit },
}

impl Schedule {
    /// As written: the crontab, or `every 5 minute`.
    pub fn text(&self) -> String {
        match self {
            Schedule::Crontab(crontab) => crontab.text.clone(),
            Schedule::Interval { every, unit } => format!("every {every} {}", unit.as_str()),
        }
    }

    /// The first run after `after`. `offset` gives the schedule's time zone's
    /// offset from UTC in seconds at a point in time (a crontab is read in
    /// local time; daylight saving time moves the offset).
    pub fn next_run(&self, after: Timestamp, offset: &dyn Fn(Timestamp) -> i32) -> Option<Timestamp> {
        match self {
            Schedule::Interval { every, unit } => Some(Timestamp::from_unix_seconds(
                after.unix_seconds() + i64::from((*every).max(1)) * unit.seconds(),
            )),
            Schedule::Crontab(crontab) => {
                let local = after.unix_seconds() + i64::from(offset(after));
                let next_local = crontab.next_after(local)?;
                let guess = next_local - i64::from(offset(after));
                let shift = offset(Timestamp::from_unix_seconds(guess));
                Some(Timestamp::from_unix_seconds(next_local - i64::from(shift)))
            }
        }
    }
}

/// Five fields: minute, hour, day of month, month, day of week.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Crontab {
    pub text: String,
    minutes: u64,
    hours: u64,
    days: u64,
    months: u64,
    weekdays: u64,
    any_day: bool,
    any_weekday: bool,
}

const MONTHS: [&str; 12] =
    ["jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec"];
const WEEKDAYS: [&str; 7] = ["sun", "mon", "tue", "wed", "thu", "fri", "sat"];

fn field(text: &str, min: u32, max: u32, names: &[&str], offset: u32) -> Option<u64> {
    let value = |part: &str| -> Option<u32> {
        if let Some(at) = names.iter().position(|n| part.eq_ignore_ascii_case(n)) {
            return Some(at as u32 + offset);
        }
        part.parse().ok()
    };
    let mut mask = 0u64;
    for part in text.split(',') {
        let (range, step) = match part.split_once('/') {
            Some((range, step)) => (range, step.parse::<u32>().ok().filter(|s| *s > 0)?),
            None => (part, 1),
        };
        let (start, end) = if range == "*" {
            (min, max)
        } else if let Some((a, b)) = range.split_once('-') {
            (value(a)?, value(b)?)
        } else {
            let at = value(range)?;
            (at, if part.contains('/') { max } else { at })
        };
        if start < min || end > max || start > end {
            return None;
        }
        let mut at = start;
        while at <= end {
            mask |= 1 << at;
            at += step;
        }
    }
    Some(mask)
}

impl Crontab {
    pub fn parse(text: &str) -> Result<Crontab, InvalidSchedule> {
        let invalid = || InvalidSchedule(text.to_owned());
        let expanded = match text.trim() {
            "@yearly" | "@annually" => "0 0 1 1 *",
            "@monthly" => "0 0 1 * *",
            "@weekly" => "0 0 * * 0",
            "@daily" | "@midnight" => "0 0 * * *",
            "@hourly" => "0 * * * *",
            other => other,
        };
        let parts: Vec<&str> = expanded.split_whitespace().collect();
        let [minute, hour, day, month, weekday] = parts.as_slice() else { return Err(invalid()) };
        let mut weekdays = field(weekday, 0, 7, &WEEKDAYS, 0).ok_or_else(invalid)?;
        // 7 is Sunday as well.
        if weekdays & (1 << 7) != 0 {
            weekdays = (weekdays | 1) & !(1 << 7);
        }
        Ok(Crontab {
            text: text.trim().to_owned(),
            minutes: field(minute, 0, 59, &[], 0).ok_or_else(invalid)?,
            hours: field(hour, 0, 23, &[], 0).ok_or_else(invalid)?,
            days: field(day, 1, 31, &[], 0).ok_or_else(invalid)?,
            months: field(month, 1, 12, &MONTHS, 1).ok_or_else(invalid)?,
            weekdays,
            any_day: day.starts_with('*'),
            any_weekday: weekday.starts_with('*'),
        })
    }

    fn day_matches(&self, day_of_month: u8, weekday: i64) -> bool {
        let day = self.days & (1 << day_of_month) != 0;
        let weekday = self.weekdays & (1 << weekday) != 0;
        // As cron has it: with both fields restricted, either may match.
        match (self.any_day, self.any_weekday) {
            (true, true) => true,
            (true, false) => weekday,
            (false, true) => day,
            (false, false) => day || weekday,
        }
    }

    /// The first matching minute after `local` (seconds of local wall-clock
    /// time, counted like Unix time), in the same terms.
    pub fn next_after(&self, local: i64) -> Option<i64> {
        let start = local.div_euclid(60) * 60 + 60;
        let first_day = start.div_euclid(86_400);
        // Eight years find every 29 February on any weekday.
        for day in first_day..first_day + 8 * 366 {
            let date = Day::from_days(day).start().utc();
            if self.months & (1 << date.month) == 0 || !self.day_matches(date.day, (day + 4).rem_euclid(7)) {
                continue;
            }
            let from = if day == first_day { start - day * 86_400 } else { 0 };
            for hour in from / 3600..24 {
                if self.hours & (1 << hour) == 0 {
                    continue;
                }
                let first_minute = if hour == from / 3600 { from % 3600 / 60 } else { 0 };
                for minute in first_minute..60 {
                    if self.minutes & (1 << minute) != 0 {
                        return Some(day * 86_400 + hour * 3600 + minute * 60);
                    }
                }
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(text: &str) -> i64 {
        Timestamp::parse_rfc3339(text).unwrap().unix_seconds()
    }

    fn next(crontab: &str, after: &str) -> String {
        let local = Crontab::parse(crontab).unwrap().next_after(at(after)).unwrap();
        Timestamp::from_unix_seconds(local).utc().to_string()
    }

    #[test]
    fn finds_the_next_run_of_a_crontab() {
        assert_eq!(next("* * * * *", "2026-11-19T01:08:19Z"), "2026-11-19T01:09:00.000000Z");
        assert_eq!(next("*/15 * * * *", "2026-11-19T01:08:00Z"), "2026-11-19T01:15:00.000000Z");
        assert_eq!(next("0 3 * * *", "2026-11-19T03:00:00Z"), "2026-11-20T03:00:00.000000Z");
        assert_eq!(
            next("30 2 * * 1-5", "2026-11-21T00:00:00Z"),
            "2026-11-23T02:30:00.000000Z",
            "Saturday → Monday"
        );
        assert_eq!(next("0 0 1 jan *", "2026-11-19T00:00:00Z"), "2027-01-01T00:00:00.000000Z");
        assert_eq!(next("0 0 29 2 *", "2026-11-19T00:00:00Z"), "2028-02-29T00:00:00.000000Z");
        assert_eq!(
            next("0 12 1 * 0", "2026-11-19T00:00:00Z"),
            "2026-11-22T12:00:00.000000Z",
            "day or weekday"
        );
        assert_eq!(next("0 9 * * sun", "2026-11-19T00:00:00Z"), next("0 9 * * 7", "2026-11-19T00:00:00Z"));
        assert_eq!(next("@hourly", "2026-11-19T01:08:00Z"), "2026-11-19T02:00:00.000000Z");
        assert_eq!(next("5,10 8-9 * * *", "2026-11-19T08:07:00Z"), "2026-11-19T08:10:00.000000Z");
    }

    #[test]
    fn refuses_what_is_no_crontab() {
        for text in [
            "",
            "* * * *",
            "60 * * * *",
            "* 24 * * *",
            "* * 0 * *",
            "*/0 * * * *",
            "5-1 * * * *",
            "x * * * *",
        ] {
            assert!(Crontab::parse(text).is_err(), "{text:?}");
        }
        assert!(Crontab::parse("0 0 31 2 *").unwrap().next_after(0).is_none(), "never");
    }

    #[test]
    fn reads_crontabs_in_their_time_zone_and_steps_intervals() {
        let berlin = |at: Timestamp| if at.unix_seconds() < at_summer_end() { 7200 } else { 3600 };
        let schedule = Schedule::Crontab(Crontab::parse("0 3 * * *").unwrap());
        let after = Timestamp::parse_rfc3339("2026-11-19T12:00:00Z").unwrap();
        let run = schedule.next_run(after, &berlin).unwrap();
        assert_eq!(
            run.utc().to_string(),
            "2026-11-20T02:00:00.000000Z",
            "03:00 in Berlin is 02:00 UTC in winter"
        );
        let interval = Schedule::Interval { every: 2, unit: IntervalUnit::Hour };
        assert_eq!(
            interval.next_run(after, &|_| 0).unwrap().utc().to_string(),
            "2026-11-19T14:00:00.000000Z"
        );
        assert_eq!(interval.text(), "every 2 hour");
        assert_eq!(schedule.text(), "0 3 * * *");
        for unit in ["minute", "hours", "day", "week", "month", "year"] {
            assert!(IntervalUnit::parse(unit).is_some());
        }
        assert!(IntervalUnit::parse("fortnight").is_none());
    }

    fn at_summer_end() -> i64 {
        at("2026-10-25T01:00:00Z")
    }
}
