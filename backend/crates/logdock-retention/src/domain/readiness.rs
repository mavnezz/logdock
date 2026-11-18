/// Why Logdock cannot take data right now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Problem {
    /// The main database cannot be read or written.
    DatabaseUnavailable,
    /// Free space in the data directory fell below the minimum.
    LowDisk { free_bytes: u64, min_free_bytes: u64 },
    /// A writer has had work waiting without committing any of it.
    WriterStalled { writer: &'static str },
}

impl Problem {
    pub fn code(&self) -> &'static str {
        match self {
            Self::DatabaseUnavailable => "database_unavailable",
            Self::LowDisk { .. } => "low_disk",
            Self::WriterStalled { .. } => "writer_stalled",
        }
    }
}

/// The answer of the readiness check: ready when nothing is wrong.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Readiness {
    pub problems: Vec<Problem>,
}

impl Readiness {
    pub fn is_ready(&self) -> bool {
        self.problems.is_empty()
    }

    /// Free space alone decides whether ingest is refused: the endpoints
    /// answer 503 instead of writing a full disk.
    pub fn disk_full(&self) -> bool {
        self.problems.iter().any(|problem| matches!(problem, Problem::LowDisk { .. }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_ready_without_problems() {
        assert!(Readiness::default().is_ready());
        let low = Readiness { problems: vec![Problem::LowDisk { free_bytes: 1, min_free_bytes: 2 }] };
        assert!(!low.is_ready());
        assert!(low.disk_full());
        let stalled = Readiness { problems: vec![Problem::WriterStalled { writer: "logs" }] };
        assert!(!stalled.disk_full());
        assert_eq!(stalled.problems[0].code(), "writer_stalled");
        assert_eq!(Problem::DatabaseUnavailable.code(), "database_unavailable");
        assert_eq!(low.problems[0].code(), "low_disk");
    }
}
