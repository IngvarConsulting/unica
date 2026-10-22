use super::{InvocationMode, ToolSpec};
use unica_execution_contract::adapter_outcome::AdapterOutcome;

use serde_json::{Map, Value};
use std::fmt;
use std::path::PathBuf;
use unica_artifact_delivery::engine::MissingEngine;
use unica_code_intelligence::model::CodeIntelligenceContext;
use unica_code_intelligence::model::CodeIntelligenceReadRequest;
use unica_code_intelligence::model::CodeIntelligenceRegistry;
use unica_code_intelligence::model::CodeSearchScope;
use unica_configuration::operational::OperationalConfig;
use unica_configuration::operational::OperationalConfigDiagnostic;
use unica_diagnostics::model::DiagnosticContext;
use unica_diagnostics::model::DiagnosticItem;
use unica_diagnostics::model::DiagnosticMapError;
use unica_diagnostics::model::DiagnosticObservation;
use unica_diagnostics::model::DiagnosticProviderRegistry;
use unica_diagnostics::model::DiagnosticRequest;
use unica_diagnostics::model::DiagnosticRequestError;
use unica_metadata::{
    MetaDiagnostic, MetaDiagnosticCode, MetaMutationData, MetaValidationData, MetaValidationStatus,
};
use unica_source_contract::events::DomainEvent;
use unica_source_contract::workspace::WorkspaceContext;
use unica_work_control::cancellation::CancellationToken;
use unica_work_control::progress::ProgressSink;
use unica_work_control::provider_deadline::ProviderDeadline;
use unica_workspace::cache::{CacheAccess, CacheReport};

pub(crate) struct HandlerOutcome {
    pub(crate) adapter: AdapterOutcome,
    pub(crate) data: Option<Value>,
    pub(crate) job: Option<Value>,
    pub(crate) events: Vec<DomainEvent>,
    pub(crate) projected_events: Vec<DomainEvent>,
    pub(crate) recorded_cache: Option<CacheReport>,
    pub(crate) diagnostics: Option<Value>,
}

impl HandlerOutcome {
    /// Типизированный результат с событиями: конструктор нужен тестовым
    /// портам, которые доказывают путь публикации событий через шов.
    #[cfg(test)]
    pub(crate) fn with_data_and_events(
        adapter: AdapterOutcome,
        data: Value,
        events: Vec<DomainEvent>,
    ) -> Self {
        Self {
            adapter,
            data: Some(data),
            job: None,
            events,
            projected_events: Vec::new(),
            recorded_cache: None,
            diagnostics: None,
        }
    }

    pub(crate) fn plain(adapter: AdapterOutcome) -> Self {
        Self {
            adapter,
            data: None,
            job: None,
            events: Vec::new(),
            projected_events: Vec::new(),
            recorded_cache: None,
            diagnostics: None,
        }
    }

    pub(crate) fn with_data(adapter: AdapterOutcome, data: Value) -> Self {
        Self {
            adapter,
            data: Some(data),
            job: None,
            events: Vec::new(),
            projected_events: Vec::new(),
            recorded_cache: None,
            diagnostics: None,
        }
    }
}

pub(crate) struct PreparedToolInvocation {
    pub(crate) format_guard: Option<FormatGuardCheck>,
    pub(crate) handler: Option<HandlerOutcome>,
}

impl PreparedToolInvocation {
    pub(crate) fn empty() -> Self {
        Self {
            format_guard: None,
            handler: None,
        }
    }
}

#[allow(dead_code)] // Concrete providers land after this orchestration seam.
pub(crate) struct MetaPublishReport {
    pub(crate) data: MetaMutationData,
    pub(crate) events: Vec<DomainEvent>,
    pub(crate) recorded_cache: Option<CacheReport>,
    pub(crate) warnings: Vec<String>,
}

impl MetaPublishReport {
    #[cfg(test)]
    pub(crate) fn source_only(data: MetaMutationData) -> Self {
        Self {
            data,
            events: Vec::new(),
            recorded_cache: None,
            warnings: Vec::new(),
        }
    }
}

pub(crate) trait PreparedMetadataMutation: Send {
    /// The preview and validation image are safe application-layer projections.
    /// Closed handles, preimages, locks, and transactions remain in `Self`.
    fn preview(&self) -> &MetaMutationData;
    fn validation_subject(&self) -> &unica_metadata::validation_subject::MetadataValidationSubject;
    fn publish(
        self: Box<Self>,
        cancellation: &CancellationToken,
    ) -> Result<MetaPublishReport, unica_metadata::request::MetaFailure>;
}

fn unavailable_metadata_diagnostic(message: impl Into<String>) -> MetaDiagnostic {
    MetaDiagnostic::error(MetaDiagnosticCode::CapabilityUnavailable, message)
}

pub(crate) enum SupportGuardCheck {
    Allow,
    Warn(String),
    Block(AdapterOutcome),
}

pub(crate) enum FormatGuardCheck {
    Allow,
    Warn {
        warning: String,
        diagnostic: Value,
    },
    Block {
        outcome: AdapterOutcome,
        diagnostic: Value,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum XdtoPublicErrorCode {
    SourceSetUnknown,
    TargetNotFound,
    NotAnXdtoPackage,
    PackageResourceMissing,
    ContainmentDenied,
}

impl XdtoPublicErrorCode {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::SourceSetUnknown => "source_set_unknown",
            Self::TargetNotFound => "target_not_found",
            Self::NotAnXdtoPackage => "not_an_xdto_package",
            Self::PackageResourceMissing => "package_resource_missing",
            Self::ContainmentDenied => "containment_denied",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct PublicFormatGuardError {
    code: XdtoPublicErrorCode,
    message: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct FormatGuardError {
    public: Option<PublicFormatGuardError>,
    internal_cause: String,
}

impl FormatGuardError {
    pub(crate) fn internal(cause: impl Into<String>) -> Self {
        Self {
            public: None,
            internal_cause: cause.into(),
        }
    }

    pub(crate) fn xdto(
        code: XdtoPublicErrorCode,
        public_message: impl Into<String>,
        internal_cause: impl Into<String>,
    ) -> Self {
        Self {
            public: Some(PublicFormatGuardError {
                code,
                message: public_message.into(),
            }),
            internal_cause: internal_cause.into(),
        }
    }

    pub(crate) fn public_projection(&self) -> Option<(XdtoPublicErrorCode, &str)> {
        self.public
            .as_ref()
            .map(|public| (public.code, public.message.as_str()))
    }

    pub(crate) fn into_internal_cause(self) -> String {
        self.internal_cause
    }
}

impl fmt::Display for FormatGuardError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.public.as_ref() {
            Some(public) => write!(formatter, "{}: {}", public.code.as_str(), public.message),
            None => formatter.write_str(&self.internal_cause),
        }
    }
}

impl std::error::Error for FormatGuardError {}

pub(crate) trait ApplicationPorts: Send + Sync {
    fn discover_workspace(
        &self,
        requested_cwd: Option<PathBuf>,
    ) -> Result<WorkspaceContext, String>;

    fn validate_tool_context(
        &self,
        spec: ToolSpec,
        args: &Map<String, Value>,
        mode: InvocationMode,
        context: &WorkspaceContext,
    ) -> Result<(), String>;

    fn load_operational_config(
        &self,
        _context: &WorkspaceContext,
    ) -> Result<OperationalConfig, OperationalConfigDiagnostic> {
        Ok(OperationalConfig::compiled_defaults())
    }

    fn prepare_tool_invocation(
        &self,
        _spec: ToolSpec,
        _args: &Map<String, Value>,
        _context: &WorkspaceContext,
        _mode: InvocationMode,
        _cancellation: &CancellationToken,
        _deadline: ProviderDeadline,
    ) -> Result<PreparedToolInvocation, String> {
        Ok(PreparedToolInvocation::empty())
    }

    /// Чего не хватает инструменту, чтобы запуститься. `None` — всё на месте.
    ///
    /// Спрашивается там, где вызов и так отказывает: отказ, назвавший одну
    /// причину из двух, уводит диагностику в заведомо неверную сторону.
    fn missing_engine(
        &self,
        _spec: ToolSpec,
        _cwd: Option<&std::path::Path>,
    ) -> Option<MissingEngine> {
        None
    }

    /// Дождаться движка, которого инструменту не хватает.
    ///
    /// Возвращает состояние exact SharedWork. `NotRequired`/`Ready` продолжают
    /// вызов; `Working`/`Failed` V12-адаптер проецирует в прежний `WorkState`.
    fn deliver_engine_if_missing(
        &self,
        _spec: ToolSpec,
        _context: &WorkspaceContext,
        _cancellation: &CancellationToken,
        _progress: &dyn ProgressSink,
    ) -> unica_artifact_delivery::shared::EngineDeliveryState {
        unica_artifact_delivery::shared::EngineDeliveryState::NotRequired
    }

    fn read_metadata_local(
        &self,
        _request: &unica_metadata::request::MetaInfoRequest,
        _context: &WorkspaceContext,
        _cancellation: &CancellationToken,
    ) -> Result<
        unica_metadata::validation_subject::MetadataRead,
        unica_metadata::request::MetaFailure,
    > {
        Err(unavailable_metadata_diagnostic("metadata provider is not configured").into())
    }

    fn read_metadata_related(
        &self,
        _request: &unica_metadata::request::MetaInfoRequest,
        _local: &unica_metadata::validation_subject::MetaLocalInfo,
        _context: &WorkspaceContext,
        _cancellation: &CancellationToken,
    ) -> unica_metadata::validation_subject::MetaRelatedData {
        unica_metadata::validation_subject::MetaEnrichment::default()
    }

    fn validate_metadata(
        &self,
        _subject: &unica_metadata::validation_subject::MetadataValidationSubject,
        _context: &WorkspaceContext,
        _cancellation: &CancellationToken,
    ) -> unica_metadata::validation_subject::MetadataValidationResult {
        MetaValidationData {
            status: MetaValidationStatus::Failed,
            diagnostics: vec![unavailable_metadata_diagnostic(
                "metadata validator is not configured",
            )],
        }
    }

    fn validate_metadata_read(
        &self,
        subject: &unica_metadata::validation_subject::MetadataValidationSubject,
        context: &WorkspaceContext,
        cancellation: &CancellationToken,
    ) -> unica_metadata::validation_subject::MetadataValidationResult {
        self.validate_metadata(subject, context, cancellation)
    }

    fn prepare_metadata_mutation(
        &self,
        _request: &unica_metadata::request::MetadataRequest,
        _context: &WorkspaceContext,
        _cancellation: &CancellationToken,
    ) -> Result<Box<dyn PreparedMetadataMutation>, unica_metadata::request::MetaFailure> {
        Err(unavailable_metadata_diagnostic("metadata mutation provider is not configured").into())
    }

    fn resolve_code_intelligence_context(
        &self,
        _context: &WorkspaceContext,
        _args: &Map<String, Value>,
    ) -> Result<CodeIntelligenceContext, String> {
        Err("code intelligence context resolver is not configured".to_string())
    }

    fn resolve_code_search_context(
        &self,
        _context: &WorkspaceContext,
        _args: &Map<String, Value>,
    ) -> Result<(CodeIntelligenceContext, CodeSearchScope), String> {
        Err("code search context resolver is not configured".to_string())
    }

    fn normalize_code_intelligence_read_request(
        &self,
        _request: CodeIntelligenceReadRequest,
        _context: &CodeIntelligenceContext,
    ) -> Result<CodeIntelligenceReadRequest, String> {
        Err("code intelligence path resolver is not configured".to_string())
    }

    fn code_intelligence_registry(&self) -> Result<CodeIntelligenceRegistry, String> {
        Err("code intelligence provider registry is not configured".to_string())
    }

    fn diagnostic_provider_registry(&self) -> Result<DiagnosticProviderRegistry, String> {
        Err("diagnostic provider registry is not configured".to_string())
    }

    fn resolve_diagnostic_context(
        &self,
        _request: &DiagnosticRequest,
        _context: &WorkspaceContext,
        _cancellation: &CancellationToken,
    ) -> Result<DiagnosticContext, DiagnosticRequestError> {
        Err(DiagnosticRequestError {
            code: "diagnostics_unavailable",
            field: None,
            message: "diagnostic context resolver is not configured".to_string(),
            retryable: false,
        })
    }

    fn map_diagnostic_observation(
        &self,
        _observation: DiagnosticObservation,
        _context: &DiagnosticContext,
        _cancellation: &CancellationToken,
    ) -> Result<DiagnosticItem, DiagnosticMapError> {
        Err(DiagnosticMapError {
            code: "diagnostics_unavailable",
            message: "diagnostic location mapper is not configured".to_string(),
        })
    }

    fn map_diagnostic_observations(
        &self,
        observations: Vec<DiagnosticObservation>,
        context: &DiagnosticContext,
        cancellation: &CancellationToken,
    ) -> Vec<Result<DiagnosticItem, DiagnosticMapError>> {
        observations
            .into_iter()
            .map(|observation| self.map_diagnostic_observation(observation, context, cancellation))
            .collect()
    }

    fn evaluate_format_guard(
        &self,
        _spec: ToolSpec,
        _args: &Map<String, Value>,
        _context: &WorkspaceContext,
    ) -> Result<FormatGuardCheck, FormatGuardError> {
        Ok(FormatGuardCheck::Allow)
    }

    fn evaluate_support_guard(
        &self,
        spec: ToolSpec,
        args: &Map<String, Value>,
        context: &WorkspaceContext,
    ) -> Result<SupportGuardCheck, String>;

    fn invoke_handler(
        &self,
        spec: ToolSpec,
        args: &Map<String, Value>,
        context: &WorkspaceContext,
        mode: InvocationMode,
        cancellation: &CancellationToken,
    ) -> Result<HandlerOutcome, String>;

    /// Config-aware adapters override this entry point; `invoke_handler`
    /// remains the compatibility path for handlers that consume no snapshot.
    fn invoke_handler_with_operational_config(
        &self,
        spec: ToolSpec,
        args: &Map<String, Value>,
        context: &WorkspaceContext,
        mode: InvocationMode,
        operational_config: Option<&OperationalConfig>,
        cancellation: &CancellationToken,
    ) -> Result<HandlerOutcome, String> {
        let _ = operational_config;
        self.invoke_handler(spec, args, context, mode, cancellation)
    }

    fn cache_report(
        &self,
        context: &WorkspaceContext,
        events: &[DomainEvent],
        mode: InvocationMode,
        cache_access: CacheAccess,
    ) -> Result<CacheReport, String>;

    fn notify_invalidation(&self, context: &WorkspaceContext, events: &[DomainEvent]);

    fn notify_committed_changes(
        &self,
        context: &WorkspaceContext,
        events: &[DomainEvent],
        cancellation: &CancellationToken,
        deadline: unica_work_control::operation_deadline::OperationDeadline,
    );
}

#[cfg(test)]
mod tests {
    use super::HandlerOutcome;
    use unica_execution_contract::adapter_outcome::AdapterOutcome;

    use serde_json::json;

    #[test]
    fn plain_handler_outcome_has_no_typed_data() {
        let outcome = HandlerOutcome::plain(AdapterOutcome::ok("plain"));

        assert_eq!(outcome.data, None);
        assert_eq!(outcome.job, None);
        assert!(outcome.events.is_empty());
        assert!(outcome.projected_events.is_empty());
    }

    #[test]
    fn handler_outcome_preserves_typed_data_separately_from_stdout() {
        let data = json!({"path": "src/Module.bsl", "noOp": false});
        let outcome = HandlerOutcome::with_data(AdapterOutcome::ok("structured"), data.clone());

        assert_eq!(outcome.data, Some(data));
        assert_eq!(outcome.job, None);
        assert!(outcome.events.is_empty());
        assert!(outcome.projected_events.is_empty());
        assert_eq!(outcome.adapter.stdout, None);
    }
}
