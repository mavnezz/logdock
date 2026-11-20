//! How long a stage keeps its data, when not as long as the server's
//! settings say. Kept with the project's settings, applied by the Retention
//! context and the segments.

/// A stage's own retention, in days per kind; `None` keeps a kind as long
/// as the server's setting says.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct StageRetention {
    pub logs_days: Option<u32>,
    pub events_days: Option<u32>,
    pub traces_days: Option<u32>,
}

impl StageRetention {
    /// Days a stage may keep a kind: a day to ten years.
    pub const DAYS: std::ops::RangeInclusive<u32> = 1..=3650;

    pub fn is_valid(&self) -> bool {
        self.days().into_iter().flatten().all(|d| Self::DAYS.contains(&d))
    }

    /// Whether it keeps everything as the server says.
    pub fn is_default(&self) -> bool {
        self.days().iter().all(Option::is_none)
    }

    fn days(&self) -> [Option<u32>; 3] {
        [self.logs_days, self.events_days, self.traces_days]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn takes_a_day_to_ten_years() {
        let stage = StageRetention { logs_days: Some(3), ..Default::default() };
        assert!(stage.is_valid() && !stage.is_default());
        assert!(StageRetention::default().is_default());
        assert!(!StageRetention { traces_days: Some(0), ..stage }.is_valid());
        assert!(!StageRetention { events_days: Some(3651), ..stage }.is_valid());
    }
}
