use serde::Serialize;
use std::fmt;
use std::time::Duration;

const SEARCH_TOTAL_DEFAULT_SECONDS: u64 = 120;
const SEARCH_RLM_DEFAULT_SECONDS: u64 = 45;
const SEARCH_GIT_GREP_DEFAULT_SECONDS: u64 = 15;
const PROVIDER_READ_DEFAULT_SECONDS: u64 = 45;
const DIAGNOSTICS_ANALYZE_DEFAULT_SECONDS: u64 = 120;
const EXPLICIT_DIAGNOSTICS_ANALYZE_MIN_SECONDS: u64 = 30;
const EXPLICIT_DIAGNOSTICS_ANALYZE_MAX_SECONDS: u64 = 3_600;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OperationalConfig {
    code_intelligence: CodeIntelligenceDeadlines,
    code_diagnostics: CodeDiagnosticsDeadlines,
}

impl OperationalConfig {
    pub const fn compiled_defaults() -> Self {
        Self {
            code_intelligence: CodeIntelligenceDeadlines::compiled_defaults(),
            code_diagnostics: CodeDiagnosticsDeadlines::compiled_defaults(),
        }
    }

    pub const fn code_intelligence(self) -> CodeIntelligenceDeadlines {
        self.code_intelligence
    }

    pub const fn code_diagnostics(self) -> CodeDiagnosticsDeadlines {
        self.code_diagnostics
    }

    pub fn with_diagnostics_analyze_timeout(
        mut self,
        timeout: Duration,
    ) -> Result<Self, OperationalConfigDiagnostic> {
        let minimum = Duration::from_secs(EXPLICIT_DIAGNOSTICS_ANALYZE_MIN_SECONDS);
        let maximum = Duration::from_secs(EXPLICIT_DIAGNOSTICS_ANALYZE_MAX_SECONDS);
        if timeout < minimum || timeout > maximum {
            return Err(OperationalConfigDiagnostic::new(
                OperationalConfigDiagnosticCode::OutOfRange,
                OperationalConfigDiagnosticSource::ExplicitArgument,
                "timeoutSeconds",
            ));
        }
        self.code_diagnostics = CodeDiagnosticsDeadlines {
            analyze_timeout: timeout,
        };
        Ok(self)
    }

    pub(crate) fn from_layers(
        shared: Option<&OperationalConfigLayer>,
        local: Option<&OperationalConfigLayer>,
    ) -> Result<Self, OperationalConfigDiagnostic> {
        let defaults = Self::compiled_defaults();
        let total = resolve_deadline(
            shared.and_then(|layer| layer.search_total_timeout),
            local.and_then(|layer| layer.search_total_timeout),
            defaults.code_intelligence.search_total_timeout,
        );
        let rlm = resolve_deadline(
            shared.and_then(|layer| layer.search_rlm_timeout),
            local.and_then(|layer| layer.search_rlm_timeout),
            defaults.code_intelligence.search_rlm_timeout,
        );
        let git_grep = resolve_deadline(
            shared.and_then(|layer| layer.search_git_grep_timeout),
            local.and_then(|layer| layer.search_git_grep_timeout),
            defaults.code_intelligence.search_git_grep_timeout,
        );
        let provider_read = resolve_deadline(
            shared.and_then(|layer| layer.provider_read_timeout),
            local.and_then(|layer| layer.provider_read_timeout),
            defaults.code_intelligence.provider_read_timeout,
        );
        let analyze = resolve_deadline(
            shared.and_then(|layer| layer.diagnostics_analyze_timeout),
            local.and_then(|layer| layer.diagnostics_analyze_timeout),
            defaults.code_diagnostics.analyze_timeout,
        );

        validate_relationship(
            total,
            rlm,
            OperationalConfigField::SearchTotal,
            OperationalConfigField::SearchRlm,
        )?;
        validate_relationship(
            total,
            git_grep,
            OperationalConfigField::SearchTotal,
            OperationalConfigField::SearchGitGrep,
        )?;

        Ok(Self {
            code_intelligence: CodeIntelligenceDeadlines {
                search_total_timeout: total.value,
                search_rlm_timeout: rlm.value,
                search_git_grep_timeout: git_grep.value,
                provider_read_timeout: provider_read.value,
            },
            code_diagnostics: CodeDiagnosticsDeadlines {
                analyze_timeout: analyze.value,
            },
        })
    }
}

impl Default for OperationalConfig {
    fn default() -> Self {
        Self::compiled_defaults()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CodeIntelligenceDeadlines {
    search_total_timeout: Duration,
    search_rlm_timeout: Duration,
    search_git_grep_timeout: Duration,
    provider_read_timeout: Duration,
}

impl CodeIntelligenceDeadlines {
    const fn compiled_defaults() -> Self {
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

    #[cfg(test)]
    pub(crate) const fn for_test(timeout: Duration) -> Self {
        Self {
            search_total_timeout: timeout,
            search_rlm_timeout: timeout,
            search_git_grep_timeout: timeout,
            provider_read_timeout: timeout,
        }
    }

    #[cfg(test)]
    pub(crate) const fn for_test_values(
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CodeDiagnosticsDeadlines {
    analyze_timeout: Duration,
}

impl CodeDiagnosticsDeadlines {
    const fn compiled_defaults() -> Self {
        Self {
            analyze_timeout: Duration::from_secs(DIAGNOSTICS_ANALYZE_DEFAULT_SECONDS),
        }
    }

    pub const fn analyze_timeout(self) -> Duration {
        self.analyze_timeout
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OperationalConfigDiagnosticCode {
    ReadFailed,
    InvalidToml,
    MissingField,
    UnsupportedVersion,
    UnknownField,
    InvalidType,
    OutOfRange,
    InconsistentValues,
}

impl OperationalConfigDiagnosticCode {
    const fn message(self) -> &'static str {
        match self {
            Self::ReadFailed => "operational configuration source could not be read",
            Self::InvalidToml => "operational configuration source is not valid TOML",
            Self::MissingField => "required operational configuration field is missing",
            Self::UnsupportedVersion => "operational configuration version is not supported",
            Self::UnknownField => "operational configuration field is not supported",
            Self::InvalidType => "operational configuration field has an invalid type",
            Self::OutOfRange => "operational configuration deadline is outside its supported range",
            Self::InconsistentValues => {
                "operational configuration deadlines violate a cross-field constraint"
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum OperationalConfigDiagnosticSource {
    #[serde(rename = "unica.toml")]
    Shared,
    #[serde(rename = "unica.local.toml")]
    Local,
    #[serde(rename = "arguments")]
    ExplicitArgument,
    #[serde(rename = "compiled_default")]
    CompiledDefault,
}

impl OperationalConfigDiagnosticSource {
    pub const fn basename(self) -> &'static str {
        match self {
            Self::Shared => "unica.toml",
            Self::Local => "unica.local.toml",
            Self::ExplicitArgument => "arguments",
            Self::CompiledDefault => "compiled_default",
        }
    }

    const fn precedence(self) -> u8 {
        match self {
            Self::CompiledDefault => 0,
            Self::Shared => 1,
            Self::Local => 2,
            Self::ExplicitArgument => 3,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OperationalConfigDiagnostic {
    code: OperationalConfigDiagnosticCode,
    source: OperationalConfigDiagnosticSource,
    field_path: String,
    message: &'static str,
}

impl OperationalConfigDiagnostic {
    pub(crate) fn new(
        code: OperationalConfigDiagnosticCode,
        source: OperationalConfigDiagnosticSource,
        field_path: impl Into<String>,
    ) -> Self {
        Self {
            code,
            source,
            field_path: field_path.into(),
            message: code.message(),
        }
    }

    pub const fn code(&self) -> OperationalConfigDiagnosticCode {
        self.code
    }

    pub const fn source(&self) -> OperationalConfigDiagnosticSource {
        self.source
    }

    pub fn field_path(&self) -> &str {
        &self.field_path
    }

    pub const fn message(&self) -> &'static str {
        self.message
    }
}

impl fmt::Display for OperationalConfigDiagnostic {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{} ({}:{})",
            self.message,
            self.source.basename(),
            self.field_path
        )
    }
}

impl std::error::Error for OperationalConfigDiagnostic {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum OperationalConfigField {
    SearchTotal,
    SearchRlm,
    SearchGitGrep,
    ProviderRead,
    DiagnosticsAnalyze,
}

impl OperationalConfigField {
    pub(crate) const fn path(self) -> &'static str {
        match self {
            Self::SearchTotal => "operational.code_intelligence.search_total_timeout_seconds",
            Self::SearchRlm => "operational.code_intelligence.search_rlm_timeout_seconds",
            Self::SearchGitGrep => "operational.code_intelligence.search_git_grep_timeout_seconds",
            Self::ProviderRead => "operational.code_intelligence.provider_read_timeout_seconds",
            Self::DiagnosticsAnalyze => "operational.code_diagnostics.analyze_timeout_seconds",
        }
    }

    const fn minimum(self) -> i64 {
        1
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct OperationalConfigLayer {
    search_total_timeout: Option<Duration>,
    search_rlm_timeout: Option<Duration>,
    search_git_grep_timeout: Option<Duration>,
    provider_read_timeout: Option<Duration>,
    diagnostics_analyze_timeout: Option<Duration>,
}

impl OperationalConfigLayer {
    pub(crate) fn set_timeout_seconds(
        &mut self,
        field: OperationalConfigField,
        seconds: i64,
        source: OperationalConfigDiagnosticSource,
    ) -> Result<(), OperationalConfigDiagnostic> {
        if seconds < field.minimum() {
            return Err(OperationalConfigDiagnostic::new(
                OperationalConfigDiagnosticCode::OutOfRange,
                source,
                field.path(),
            ));
        }
        let seconds = u64::try_from(seconds).map_err(|conversion_error_ignored| {
            let _ = conversion_error_ignored;
            OperationalConfigDiagnostic::new(
                OperationalConfigDiagnosticCode::OutOfRange,
                source,
                field.path(),
            )
        })?;
        let timeout = Duration::from_secs(seconds);
        match field {
            OperationalConfigField::SearchTotal => self.search_total_timeout = Some(timeout),
            OperationalConfigField::SearchRlm => self.search_rlm_timeout = Some(timeout),
            OperationalConfigField::SearchGitGrep => self.search_git_grep_timeout = Some(timeout),
            OperationalConfigField::ProviderRead => self.provider_read_timeout = Some(timeout),
            OperationalConfigField::DiagnosticsAnalyze => {
                self.diagnostics_analyze_timeout = Some(timeout)
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy)]
struct ResolvedDeadline {
    value: Duration,
    source: OperationalConfigDiagnosticSource,
}

fn resolve_deadline(
    shared: Option<Duration>,
    local: Option<Duration>,
    default: Duration,
) -> ResolvedDeadline {
    match (shared, local) {
        (shared_ignored, Some(value)) => {
            let _ = shared_ignored;
            ResolvedDeadline {
                value,
                source: OperationalConfigDiagnosticSource::Local,
            }
        }
        (Some(value), None) => ResolvedDeadline {
            value,
            source: OperationalConfigDiagnosticSource::Shared,
        },
        (None, None) => ResolvedDeadline {
            value: default,
            source: OperationalConfigDiagnosticSource::CompiledDefault,
        },
    }
}

fn validate_relationship(
    total: ResolvedDeadline,
    provider: ResolvedDeadline,
    total_field: OperationalConfigField,
    provider_field: OperationalConfigField,
) -> Result<(), OperationalConfigDiagnostic> {
    if provider.value <= total.value {
        return Ok(());
    }
    let (source, field) = if total.source.precedence() > provider.source.precedence() {
        (total.source, total_field)
    } else {
        (provider.source, provider_field)
    };
    Err(OperationalConfigDiagnostic::new(
        OperationalConfigDiagnosticCode::InconsistentValues,
        source,
        field.path(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compiled_defaults_preserve_existing_deadlines() {
        let config = OperationalConfig::compiled_defaults();
        let code = config.code_intelligence();

        assert_eq!(code.search_total_timeout(), Duration::from_secs(120));
        assert_eq!(code.search_rlm_timeout(), Duration::from_secs(45));
        assert_eq!(code.search_git_grep_timeout(), Duration::from_secs(15));
        assert_eq!(code.provider_read_timeout(), Duration::from_secs(45));
        assert_eq!(
            config.code_diagnostics().analyze_timeout(),
            Duration::from_secs(120)
        );
    }

    #[test]
    fn operational_layers_accept_deadlines_above_compiled_defaults() {
        let mut shared = OperationalConfigLayer::default();
        for (field, seconds) in [
            (OperationalConfigField::SearchTotal, 7_200),
            (OperationalConfigField::SearchRlm, 3_600),
            (OperationalConfigField::SearchGitGrep, 1_800),
            (OperationalConfigField::ProviderRead, 7_200),
            (OperationalConfigField::DiagnosticsAnalyze, 7_200),
        ] {
            shared
                .set_timeout_seconds(field, seconds, OperationalConfigDiagnosticSource::Shared)
                .expect("positive operational deadline");
        }
        let config = OperationalConfig::from_layers(Some(&shared), None).unwrap();
        assert_eq!(
            config.code_intelligence().search_total_timeout(),
            Duration::from_secs(7_200)
        );
        assert_eq!(
            config.code_diagnostics().analyze_timeout(),
            Duration::from_secs(7_200)
        );
    }

    #[test]
    fn same_layer_relationship_failure_is_attributed_to_the_provider_field() {
        let mut shared = OperationalConfigLayer::default();
        shared
            .set_timeout_seconds(
                OperationalConfigField::SearchTotal,
                10,
                OperationalConfigDiagnosticSource::Shared,
            )
            .unwrap();
        shared
            .set_timeout_seconds(
                OperationalConfigField::SearchRlm,
                20,
                OperationalConfigDiagnosticSource::Shared,
            )
            .unwrap();

        let diagnostic = OperationalConfig::from_layers(Some(&shared), None)
            .expect_err("provider deadline above total must fail");

        assert_eq!(
            diagnostic.source(),
            OperationalConfigDiagnosticSource::Shared
        );
        assert_eq!(
            diagnostic.field_path(),
            "operational.code_intelligence.search_rlm_timeout_seconds"
        );
    }

    #[test]
    fn explicit_diagnostics_timeout_is_validated_and_overlaid_immutably() {
        let defaults = OperationalConfig::compiled_defaults();
        let configured = defaults
            .with_diagnostics_analyze_timeout(Duration::from_secs(900))
            .expect("900 seconds is in the public diagnostics range");

        assert_eq!(
            defaults.code_diagnostics().analyze_timeout(),
            Duration::from_secs(120)
        );
        assert_eq!(
            configured.code_diagnostics().analyze_timeout(),
            Duration::from_secs(900)
        );
        let diagnostic = defaults
            .with_diagnostics_analyze_timeout(Duration::from_secs(29))
            .expect_err("explicit timeout below the schema minimum must fail");
        assert_eq!(
            diagnostic.code(),
            OperationalConfigDiagnosticCode::OutOfRange
        );
        assert_eq!(
            diagnostic.source(),
            OperationalConfigDiagnosticSource::ExplicitArgument
        );
        assert_eq!(diagnostic.field_path(), "timeoutSeconds");
    }

    #[test]
    fn cross_field_error_is_attributed_to_the_higher_precedence_value() {
        let mut shared = OperationalConfigLayer::default();
        shared
            .set_timeout_seconds(
                OperationalConfigField::SearchRlm,
                30,
                OperationalConfigDiagnosticSource::Shared,
            )
            .expect("shared RLM timeout is individually valid");
        let mut local = OperationalConfigLayer::default();
        local
            .set_timeout_seconds(
                OperationalConfigField::SearchTotal,
                20,
                OperationalConfigDiagnosticSource::Local,
            )
            .expect("local total timeout is individually valid");

        let diagnostic = OperationalConfig::from_layers(Some(&shared), Some(&local))
            .expect_err("effective RLM deadline exceeds effective total deadline");
        assert_eq!(
            diagnostic.source(),
            OperationalConfigDiagnosticSource::Local
        );
        assert_eq!(
            diagnostic.field_path(),
            "operational.code_intelligence.search_total_timeout_seconds"
        );
    }

    #[test]
    fn diagnostic_serialization_is_stable_and_contains_only_safe_fields() {
        let diagnostic = OperationalConfigDiagnostic::new(
            OperationalConfigDiagnosticCode::InvalidType,
            OperationalConfigDiagnosticSource::Shared,
            "operational.code_intelligence.provider_read_timeout_seconds",
        );

        assert_eq!(
            serde_json::to_value(diagnostic).expect("serialize operational config diagnostic"),
            serde_json::json!({
                "code": "invalid_type",
                "source": "unica.toml",
                "fieldPath": "operational.code_intelligence.provider_read_timeout_seconds",
                "message": "operational configuration field has an invalid type"
            })
        );
    }

    #[test]
    fn test_deadlines_support_subsecond_fake_clock_scenarios() {
        let timeout = Duration::from_millis(25);
        let deadlines = CodeIntelligenceDeadlines::for_test(timeout);

        assert_eq!(deadlines.search_total_timeout(), timeout);
        assert_eq!(deadlines.search_rlm_timeout(), timeout);
        assert_eq!(deadlines.search_git_grep_timeout(), timeout);
        assert_eq!(deadlines.provider_read_timeout(), timeout);
    }
}
