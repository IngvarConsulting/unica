use crate::model::ProviderRole;
use std::time::Duration;

const SEARCH_TOTAL_DEFAULT_SECONDS: u64 = 300;
const SEARCH_RLM_DEFAULT_SECONDS: u64 = 300;
const SEARCH_GIT_GREP_DEFAULT_SECONDS: u64 = 30;
const PROVIDER_READ_DEFAULT_SECONDS: u64 = 45;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CodeIntelligenceDeadlines {
    search_total_timeout: Duration,
    search_rlm_timeout: Duration,
    search_git_grep_timeout: Duration,
    provider_read_timeout: Duration,
}

impl CodeIntelligenceDeadlines {
    /// Values already resolved and validated by the configuration owner.
    pub const fn from_timeouts(
        search_total_timeout: Duration,
        search_rlm_timeout: Duration,
        search_git_grep_timeout: Duration,
        provider_read_timeout: Duration,
    ) -> Self {
        Self {
            search_total_timeout,
            search_rlm_timeout,
            search_git_grep_timeout,
            provider_read_timeout,
        }
    }

    pub const fn compiled_defaults() -> Self {
        Self {
            search_total_timeout: Duration::from_secs(SEARCH_TOTAL_DEFAULT_SECONDS),
            search_rlm_timeout: Duration::from_secs(SEARCH_RLM_DEFAULT_SECONDS),
            search_git_grep_timeout: Duration::from_secs(SEARCH_GIT_GREP_DEFAULT_SECONDS),
            provider_read_timeout: Duration::from_secs(PROVIDER_READ_DEFAULT_SECONDS),
        }
    }

    pub const fn search_total_timeout(self) -> Duration {
        self.search_total_timeout
    }

    pub const fn search_rlm_timeout(self) -> Duration {
        self.search_rlm_timeout
    }

    pub const fn search_git_grep_timeout(self) -> Duration {
        self.search_git_grep_timeout
    }

    pub const fn provider_read_timeout(self) -> Duration {
        self.provider_read_timeout
    }

    pub const fn search_timeout_for(self, role: ProviderRole) -> Duration {
        match role {
            ProviderRole::Semantic => self.search_rlm_timeout,
            ProviderRole::Symbol => self.search_total_timeout,
            ProviderRole::Lexical => self.search_git_grep_timeout,
        }
    }

    pub fn within_search_budget(mut self, remaining: Duration) -> Self {
        self.search_total_timeout = self.search_total_timeout.min(remaining);
        self.search_git_grep_timeout = self.search_git_grep_timeout.min(remaining);
        self
    }

    #[cfg(any(test, feature = "test-support"))]
    pub const fn for_test(timeout: Duration) -> Self {
        Self {
            search_total_timeout: timeout,
            search_rlm_timeout: timeout,
            search_git_grep_timeout: timeout,
            provider_read_timeout: timeout,
        }
    }

    #[cfg(any(test, feature = "test-support"))]
    pub const fn for_test_values(
        search_total_timeout: Duration,
        search_rlm_timeout: Duration,
        search_git_grep_timeout: Duration,
        provider_read_timeout: Duration,
    ) -> Self {
        Self {
            search_total_timeout,
            search_rlm_timeout,
            search_git_grep_timeout,
            provider_read_timeout,
        }
    }
}
