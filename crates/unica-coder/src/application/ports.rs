use super::{AdapterOutcome, InvocationMode, ToolSpec};
use crate::application::metadata::{MetaFailure, MetaInfoRequest, MetadataRequest};
use crate::application::source_navigation::{
    SourceChildrenRequest, SourceChildrenResult, SourceLocateRequest, SourceLocateResult,
    SourceResolveRequest, SourceResolveResult,
};
use crate::application::source_resources::{SourceReadRequest, SourceResourcesRequest};
use crate::domain::cache::{CacheAccess, CacheReport};
use crate::domain::cancellation::CancellationToken;
use crate::domain::code_intelligence::{
    CodeIntelligenceContext, CodeIntelligenceReadRequest, CodeIntelligenceRegistry,
    CodeSearchScope, ProviderDeadline,
};
use crate::domain::diagnostics::{
    DiagnosticContext, DiagnosticItem, DiagnosticMapError, DiagnosticObservation,
    DiagnosticProviderRegistry, DiagnosticRequest, DiagnosticRequestError,
};
use crate::domain::events::DomainEvent;
use crate::domain::metadata::{
    MetaCollectionsData, MetaDiagnostic, MetaDiagnosticCode, MetaInfoData, MetaInfoDeclarations,
    MetaInfoDetails, MetaInfoPropertyData, MetaMutationData, MetaPredefinedItemsData,
    MetaRelationsData, MetaSupportStatus, MetaUsageData, MetaValidationData, MetaValidationStatus,
    MetadataKind,
};
use crate::domain::operational_config::{OperationalConfig, OperationalConfigDiagnostic};
use crate::domain::project_health::{ProjectHealthInspectionError, ProjectHealthSnapshot};
use crate::domain::source_resources::{
    ResourceManifestPage, SourceReadResult, SourceResourceError,
};
use crate::domain::source_target::MetadataAddress;
use crate::domain::subsystem::SubsystemAddress;
use crate::domain::workspace::WorkspaceContext;
use serde_json::{Map, Value};
use std::fmt;
use std::path::PathBuf;

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

    pub(crate) fn with_data_events_and_projection(
        adapter: AdapterOutcome,
        data: Value,
        events: Vec<DomainEvent>,
        projected_events: Vec<DomainEvent>,
    ) -> Self {
        Self {
            adapter,
            data: Some(data),
            job: None,
            events,
            projected_events,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MetadataAuxiliaryXmlKind {
    ExchangePlanContent,
    BusinessProcessFlowchart,
    PredefinedData(MetadataKind),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MetadataChildResourceKind {
    FormContent,
    TemplateContent {
        template_type: MetadataTemplateType,
        part: MetadataTemplateResourcePart,
    },
    Module,
    /// A payload member the platform writes beside a child but Unica never
    /// interprets — form help, help assets, form item pictures, HTML template
    /// assets. It is carried verbatim; the closed guard still names the exact
    /// path shapes that may appear, so a mutation can never clobber bytes the
    /// contract does not recognise. Its relative path lives in the child's
    /// [`MetadataChildFootprintEvidence::retained`] at the resource's ordinal.
    Retained,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MetadataTemplateType {
    HtmlDocument,
    TextDocument,
    SpreadsheetDocument,
    BinaryData,
    DataCompositionSchema,
}

impl MetadataTemplateType {
    pub(crate) fn from_descriptor_value(value: &str) -> Option<Self> {
        match value {
            "HTMLDocument" => Some(Self::HtmlDocument),
            "TextDocument" => Some(Self::TextDocument),
            "SpreadsheetDocument" => Some(Self::SpreadsheetDocument),
            "BinaryData" => Some(Self::BinaryData),
            "DataCompositionSchema" => Some(Self::DataCompositionSchema),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MetadataTemplateResourcePart {
    Primary,
    HtmlPage,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MetadataChildProfile {
    Form,
    Command,
    Template(MetadataTemplateType),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum MetadataChildDirectoryKind {
    Root,
    Extension,
    HtmlPages,
    /// A directory the contract does not name on its own — it exists only
    /// because a payload file sits under it, such as `Ext/Form` around a form
    /// module or `Ext/Help` around help pages. It repeats once per directory
    /// instead of naming one.
    Nested,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MetadataChildFootprintEvidence {
    pub(crate) child: MetadataAddress,
    pub(crate) profile: MetadataChildProfile,
    pub(crate) directories: Vec<MetadataChildDirectoryKind>,
    /// Relative paths of the retained payload members, sorted. A resource with
    /// [`MetadataChildResourceKind::Retained`] and ordinal `n` is this list's
    /// `n`-th entry, which is how the validator recovers the paths it needs to
    /// re-derive the directory topology.
    pub(crate) retained: Vec<PathBuf>,
}

#[allow(dead_code)] // Concrete providers land after this orchestration seam.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum MetadataResourceRole {
    Descriptor,
    Registration,
    Module {
        owner: MetadataAddress,
    },
    Form {
        owner: MetadataAddress,
        name: String,
    },
    Template {
        owner: MetadataAddress,
        name: String,
    },
    Command {
        owner: MetadataAddress,
        name: String,
    },
    ChildResource {
        child: MetadataAddress,
        kind: MetadataChildResourceKind,
        ordinal: usize,
    },
    Dependency {
        target: MetadataAddress,
    },
    AuxiliaryXml {
        owner: MetadataAddress,
        kind: MetadataAuxiliaryXmlKind,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MetadataResourceImage {
    /// Provider-neutral logical role and identity; physical paths stay opaque.
    pub(crate) role: MetadataResourceRole,
    pub(crate) bytes: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MetadataValidationSubject {
    pub(crate) target: MetadataAddress,
    pub(crate) resources: Vec<MetadataResourceImage>,
    pub(crate) child_footprints: Vec<MetadataChildFootprintEvidence>,
    pub(crate) registrar_evidence: MetadataEvidenceAvailability,
    /// `None` — подсистемы не просматривались: субъект ничего не утверждает о
    /// членствах объекта. `meta.info` собирает обе ролевые группы, а мутации
    /// оставляют поле пустым; полнота по умолчанию сделала бы несобранное
    /// доказанным.
    pub(crate) subsystem_evidence: Option<MetadataSubsystemEvidence>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum MetadataSubsystemEvidence {
    Complete {
        functional_subsystems: Vec<SubsystemAddress>,
        interface_subsystems: Vec<SubsystemAddress>,
    },
    Unavailable(Vec<MetaDiagnostic>),
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) enum MetadataEvidenceAvailability {
    #[default]
    Complete,
    Unavailable(Vec<MetaDiagnostic>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MetaLocalInfo {
    pub(crate) metadata_path: MetadataAddress,
    pub(crate) kind: MetadataKind,
    pub(crate) details: MetaInfoDetails,
    pub(crate) name: String,
    pub(crate) synonym: Option<String>,
    pub(crate) support: MetaSupportStatus,
    pub(crate) properties: Vec<MetaInfoPropertyData>,
    pub(crate) declarations: MetaInfoDeclarations,
    /// Internal owner context needed to validate typed Predefined.xml values.
    /// It is deliberately not copied into the public `meta.info` result.
    pub(crate) predefined_code_type: Option<String>,
    pub(crate) relations: MetaRelationsData,
    pub(crate) collections: MetaCollectionsData,
    pub(crate) diagnostics: Vec<MetaDiagnostic>,
}

impl MetaLocalInfo {
    pub(crate) fn into_info(
        self,
        validation: MetaValidationData,
        enrichment: MetaEnrichment,
        functional_subsystems: Option<Vec<SubsystemAddress>>,
        interface_subsystems: Option<Vec<SubsystemAddress>>,
    ) -> MetaInfoData {
        MetaInfoData {
            metadata_path: self.metadata_path,
            details: self.details,
            name: self.name,
            synonym: self.synonym,
            support: self.support,
            properties: self.properties,
            declarations: self.declarations,
            relations: self.relations,
            collections: self.collections,
            functional_subsystems,
            interface_subsystems,
            predefined_items: enrichment.predefined_items,
            usage: enrichment.usage,
            validation,
        }
    }
}

pub(crate) struct MetadataRead {
    pub(crate) local: MetaLocalInfo,
    pub(crate) validation_subject: MetadataValidationSubject,
}

/// Everything a metadata read adds on top of the object's own descriptor.
///
/// The three parts have different natures and are kept apart on purpose:
/// `predefined_items` is the object's own content, `usage` is what the source
/// tree says references it, and `related` is what the code index says. Only the
/// last can be stale or missing, so only it carries index metadata.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct MetaEnrichment {
    pub(crate) predefined_items: Option<MetaPredefinedItemsData>,
    pub(crate) usage: MetaUsageData,
    pub(crate) diagnostics: Vec<MetaDiagnostic>,
}

pub(crate) type MetaRelatedData = MetaEnrichment;
pub(crate) type MetadataValidationResult = MetaValidationData;

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
    fn validation_subject(&self) -> &MetadataValidationSubject;
    fn publish(
        self: Box<Self>,
        cancellation: &CancellationToken,
    ) -> Result<MetaPublishReport, MetaFailure>;
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

    fn inspect_project_health(
        &self,
        _context: &WorkspaceContext,
        _cancellation: &CancellationToken,
        _deadline: ProviderDeadline,
    ) -> Result<ProjectHealthSnapshot, ProjectHealthInspectionError> {
        Err(ProjectHealthInspectionError::Fatal(
            "project health inspector is not configured".into(),
        ))
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

    fn read_metadata_local(
        &self,
        _request: &MetaInfoRequest,
        _context: &WorkspaceContext,
        _cancellation: &CancellationToken,
    ) -> Result<MetadataRead, MetaFailure> {
        Err(unavailable_metadata_diagnostic("metadata provider is not configured").into())
    }

    fn read_metadata_related(
        &self,
        _request: &MetaInfoRequest,
        _local: &MetaLocalInfo,
        _context: &WorkspaceContext,
        _cancellation: &CancellationToken,
    ) -> MetaRelatedData {
        MetaEnrichment::default()
    }

    fn validate_metadata(
        &self,
        _subject: &MetadataValidationSubject,
        _context: &WorkspaceContext,
        _cancellation: &CancellationToken,
    ) -> MetadataValidationResult {
        MetaValidationData {
            status: MetaValidationStatus::Failed,
            diagnostics: vec![unavailable_metadata_diagnostic(
                "metadata validator is not configured",
            )],
        }
    }

    fn validate_metadata_read(
        &self,
        subject: &MetadataValidationSubject,
        context: &WorkspaceContext,
        cancellation: &CancellationToken,
    ) -> MetadataValidationResult {
        self.validate_metadata(subject, context, cancellation)
    }

    fn prepare_metadata_mutation(
        &self,
        _request: &MetadataRequest,
        _context: &WorkspaceContext,
        _cancellation: &CancellationToken,
    ) -> Result<Box<dyn PreparedMetadataMutation>, MetaFailure> {
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

    fn resolve_source_navigation(
        &self,
        _request: SourceResolveRequest,
        _context: &WorkspaceContext,
        _cancellation: &CancellationToken,
    ) -> Result<SourceResolveResult, String> {
        Err("source navigation resolver is not configured".to_string())
    }

    fn children_source_navigation(
        &self,
        _request: SourceChildrenRequest,
        _context: &WorkspaceContext,
        _cancellation: &CancellationToken,
    ) -> Result<SourceChildrenResult, String> {
        Err("source navigation traversal is not configured".to_string())
    }

    fn locate_source_navigation(
        &self,
        _request: SourceLocateRequest,
        _context: &WorkspaceContext,
        _cancellation: &CancellationToken,
    ) -> Result<SourceLocateResult, String> {
        Err("source navigation locator is not configured".to_string())
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
    ) -> Result<Vec<DiagnosticItem>, DiagnosticMapError> {
        observations
            .into_iter()
            .map(|observation| self.map_diagnostic_observation(observation, context, cancellation))
            .collect()
    }

    fn source_resources(
        &self,
        _request: SourceResourcesRequest,
        _context: &WorkspaceContext,
        _cancellation: &CancellationToken,
    ) -> Result<ResourceManifestPage, SourceResourceError> {
        Err(SourceResourceError::new(
            crate::domain::source_resources::SourceResourceErrorCode::SourceUnavailable,
            "source resource provider is not configured",
        ))
    }

    fn read_source_resource(
        &self,
        _request: SourceReadRequest,
        _context: &WorkspaceContext,
        _cancellation: &CancellationToken,
    ) -> Result<SourceReadResult, SourceResourceError> {
        Err(SourceResourceError::new(
            crate::domain::source_resources::SourceResourceErrorCode::SourceUnavailable,
            "source resource provider is not configured",
        ))
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
}

#[cfg(test)]
mod tests {
    use super::HandlerOutcome;
    use crate::application::AdapterOutcome;

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
