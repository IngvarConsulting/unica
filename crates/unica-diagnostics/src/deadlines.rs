use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CodeDiagnosticsDeadlines {
    analyze_timeout: Option<Duration>,
}

impl CodeDiagnosticsDeadlines {
    /// A value already resolved and validated by the configuration owner.
    pub const fn from_timeout(analyze_timeout: Option<Duration>) -> Self {
        Self { analyze_timeout }
    }

    pub const fn compiled_defaults() -> Self {
        Self {
            analyze_timeout: None,
        }
    }

    /// `None` — пользователь срок анализа не задавал, и анализ идёт до конца.
    pub const fn analyze_timeout(self) -> Option<Duration> {
        self.analyze_timeout
    }
}

pub const DIAGNOSTICS_ANALYZE_TIMEOUT_MIN_SECONDS: u64 = 30;

pub const DIAGNOSTICS_ANALYZE_TIMEOUT_MAX_SECONDS: u64 = 3600;
