pub use crate::domain::code_intelligence::ProviderDeadline;
use crate::domain::{
    cancellation::CancellationToken,
    operational_config::DIAGNOSTICS_ANALYZE_DEFAULT_SECONDS,
    project_sources::ProjectSourceSet,
    source_location::SourceLocation,
    source_roots::ResolvedSourceRoot,
    source_target::{MetadataAddress, ResolvedTarget, TargetKind},
    workspace::WorkspaceContext,
};
use serde::Serialize;
use std::{fmt, sync::Arc, time::Duration};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum DiagnosticAction {
    Analyze,
    Findings,
    Status,
    Catalog,
}

impl DiagnosticAction {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Analyze => "analyze",
            Self::Findings => "findings",
            Self::Status => "status",
            Self::Catalog => "catalog",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "analyze" => Some(Self::Analyze),
            "findings" => Some(Self::Findings),
            "status" => Some(Self::Status),
            "catalog" => Some(Self::Catalog),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Ord, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct DiagnosticProviderId(&'static str);

impl DiagnosticProviderId {
    pub const fn new_const(value: &'static str) -> Self {
        Self(value)
    }

    pub const fn as_str(self) -> &'static str {
        self.0
    }
}

pub const BSL_ANALYZER_PROVIDER: DiagnosticProviderId =
    DiagnosticProviderId::new_const("bsl-analyzer");

pub const LIVE_DIAGNOSTIC_PROVIDERS: &[DiagnosticProviderId] = &[BSL_ANALYZER_PROVIDER];

pub const DIAGNOSTIC_LIMIT_DEFAULT: usize = 200;

/// Budget for the actions that never resolve an `OperationalConfig`
/// (INV-APP-CONFIG-SNAPSHOT): `findings`, `status` and `catalog`. It is the
/// compiled analyze fallback, so both budgets stay one number.
pub const DIAGNOSTIC_BUDGET_WITHOUT_CONFIG: Duration =
    Duration::from_secs(DIAGNOSTICS_ANALYZE_DEFAULT_SECONDS);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiagnosticRequest {
    pub action: DiagnosticAction,
    pub source_set: String,
    pub metadata_path: Option<MetadataAddress>,
    pub filter: DiagnosticFilter,
    pub range: Option<DiagnosticRange>,
    pub limit: usize,
    pub timeout: Option<Duration>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiagnosticRequestError {
    pub code: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub field: Option<&'static str>,
    pub message: String,
    pub retryable: bool,
}

impl fmt::Display for DiagnosticRequestError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for DiagnosticRequestError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiagnosticMapError {
    pub code: &'static str,
    pub message: String,
}

impl fmt::Display for DiagnosticMapError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for DiagnosticMapError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiagnosticProviderRequest {
    pub action: DiagnosticAction,
    pub source_set: String,
    pub metadata_path: Option<MetadataAddress>,
    pub target_kind: TargetKind,
    pub filter: DiagnosticFilter,
    pub range: Option<DiagnosticRange>,
}

#[derive(Debug, Clone)]
pub struct DiagnosticContext {
    pub workspace: WorkspaceContext,
    pub source_set: ProjectSourceSet,
    pub source_root: ResolvedSourceRoot,
    pub target: ResolvedTarget,
}

impl DiagnosticContext {
    pub fn new(
        workspace: WorkspaceContext,
        source_set: ProjectSourceSet,
        source_root: ResolvedSourceRoot,
        target: ResolvedTarget,
    ) -> Self {
        Self {
            workspace,
            source_set,
            source_root,
            target,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiagnosticSelection {
    pub source_set: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata_path: Option<MetadataAddress>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_kind: Option<TargetKind>,
    pub providers: Vec<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub filter: Option<DiagnosticFilter>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiagnosticFilter {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min_severity: Option<DiagnosticSeverity>,
    pub codes: Vec<DiagnosticCodeFilter>,
}

impl Default for DiagnosticFilter {
    fn default() -> Self {
        Self {
            min_severity: Some(DiagnosticSeverity::Warning),
            codes: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiagnosticCodeFilter {
    pub provider: String,
    pub code: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiagnosticRange {
    pub start_line: usize,
    pub start_column: usize,
    pub end_line: usize,
    pub end_column: usize,
}

impl DiagnosticRange {
    pub fn is_non_empty(self) -> bool {
        (self.start_line, self.start_column) < (self.end_line, self.end_column)
    }

    /// A zero-width observation range is a caret position, not "no position":
    /// providers emit one wherever a fix inserts text. It belongs to a
    /// requested range when the caret falls inside that half-open range.
    pub fn intersects(self, other: Self) -> bool {
        if !self.is_non_empty() {
            return (other.start_line, other.start_column) <= (self.start_line, self.start_column)
                && (self.start_line, self.start_column) < (other.end_line, other.end_column);
        }
        (self.start_line, self.start_column) < (other.end_line, other.end_column)
            && (other.start_line, other.start_column) < (self.end_line, self.end_column)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum DiagnosticSeverity {
    Error,
    Warning,
    Info,
    Hint,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum DiagnosticTag {
    Unnecessary,
    Deprecated,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum DiagnosticFocusKind {
    Target,
    SourceRange,
    Metadata,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MetadataElement {
    pub collection: String,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MetadataFocus {
    pub element_path: Vec<MetadataElement>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub property: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum DiagnosticFocus {
    Target,
    SourceRange {
        range: DiagnosticRange,
    },
    Metadata {
        element_path: Vec<MetadataElement>,
        #[serde(skip_serializing_if = "Option::is_none")]
        property: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        language: Option<String>,
    },
}

impl From<MetadataFocus> for DiagnosticFocus {
    fn from(focus: MetadataFocus) -> Self {
        Self::Metadata {
            element_path: focus.element_path,
            property: focus.property,
            language: focus.language,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum UnaddressableReason {
    ResourceNotAddressable,
    OwnerUnproven,
    SourceFormatUnsupported,
}

/// Provider-private location. A resource handle may be a relative path, an
/// absolute path, or a URI and therefore must never cross the public boundary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiagnosticObservationLocation {
    Logical {
        metadata_path: Option<MetadataAddress>,
    },
    Resource {
        handle: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiagnosticObservationFocus {
    Target,
    SourceRange(DiagnosticRange),
    Metadata(MetadataFocus),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiagnosticObservation {
    Diagnostic {
        provider: DiagnosticProviderId,
        location: DiagnosticObservationLocation,
        focus: DiagnosticObservationFocus,
        code: String,
        severity: DiagnosticSeverity,
        message: String,
        tags: Vec<DiagnosticTag>,
    },
    ResourceFailure {
        provider: DiagnosticProviderId,
        location: DiagnosticObservationLocation,
        error: DiagnosticError,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiagnosticRuleObservation {
    pub provider: DiagnosticProviderId,
    pub code: String,
    pub default_severity: DiagnosticSeverity,
    pub title: String,
    pub description: Option<String>,
    pub tags: Vec<DiagnosticTag>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum DiagnosticProviderStatus {
    Completed,
    Empty,
    Unsupported,
    Unavailable,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum DiagnosticReadinessState {
    NotStarted,
    Building,
    Ready,
    Stale,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiagnosticReadiness {
    pub state: DiagnosticReadinessState,
    pub retryable: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiagnosticError {
    pub code: String,
    pub message: String,
    pub retryable: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiagnosticProviderOutcome {
    pub status: DiagnosticProviderStatus,
    pub complete: bool,
    pub version: Option<String>,
    pub observations: Vec<DiagnosticObservation>,
    pub rules: Vec<DiagnosticRuleObservation>,
    pub readiness: Option<DiagnosticReadiness>,
    pub error: Option<DiagnosticError>,
}

impl DiagnosticProviderOutcome {
    pub fn empty(status: DiagnosticProviderStatus) -> Self {
        Self {
            status,
            complete: matches!(
                status,
                DiagnosticProviderStatus::Completed | DiagnosticProviderStatus::Empty
            ),
            version: None,
            observations: Vec::new(),
            rules: Vec::new(),
            readiness: None,
            error: None,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct DiagnosticProviderDescriptor {
    pub id: DiagnosticProviderId,
    pub actions: &'static [DiagnosticAction],
    pub findings_target_kinds: &'static [TargetKind],
    pub emits_focus_kinds: &'static [DiagnosticFocusKind],
}

impl DiagnosticProviderDescriptor {
    pub fn supports_action(&self, action: DiagnosticAction) -> bool {
        self.actions.contains(&action)
    }

    pub fn supports_findings_target(&self, target_kind: TargetKind) -> bool {
        self.findings_target_kinds.contains(&target_kind)
    }
}

pub trait DiagnosticProvider: Send + Sync {
    fn descriptor(&self) -> &'static DiagnosticProviderDescriptor;

    fn execute(
        &self,
        request: &DiagnosticProviderRequest,
        context: &DiagnosticContext,
        deadline: ProviderDeadline,
        cancellation: &CancellationToken,
    ) -> DiagnosticProviderOutcome;
}

pub struct DiagnosticProviderRegistry {
    providers: Vec<Arc<dyn DiagnosticProvider>>,
}

impl fmt::Debug for DiagnosticProviderRegistry {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DiagnosticProviderRegistry")
            .field("ids", &self.ids().collect::<Vec<_>>())
            .finish()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DuplicateDiagnosticProviderId {
    pub provider_id: DiagnosticProviderId,
}

impl fmt::Display for DuplicateDiagnosticProviderId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "duplicate diagnostic provider id `{}`",
            self.provider_id.as_str()
        )
    }
}

impl std::error::Error for DuplicateDiagnosticProviderId {}

impl DiagnosticProviderRegistry {
    pub fn new(
        providers: Vec<Arc<dyn DiagnosticProvider>>,
    ) -> Result<Self, DuplicateDiagnosticProviderId> {
        for (index, provider) in providers.iter().enumerate() {
            let provider_id = provider.descriptor().id;
            if providers[..index]
                .iter()
                .any(|registered| registered.descriptor().id == provider_id)
            {
                return Err(DuplicateDiagnosticProviderId { provider_id });
            }
        }
        Ok(Self { providers })
    }

    pub fn providers(&self) -> &[Arc<dyn DiagnosticProvider>] {
        &self.providers
    }

    pub fn ids(&self) -> impl Iterator<Item = DiagnosticProviderId> + '_ {
        self.providers
            .iter()
            .map(|provider| provider.descriptor().id)
    }

    pub fn descriptors(&self) -> impl Iterator<Item = &'static DiagnosticProviderDescriptor> + '_ {
        self.providers.iter().map(|provider| provider.descriptor())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum DiagnosticResultState {
    Completed,
    Partial,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum DiagnosticItemKind {
    Diagnostic,
    ResourceFailure,
    DiagnosticRule,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiagnosticCapabilities {
    pub actions: Vec<DiagnosticAction>,
    pub findings_target_kinds: Vec<TargetKind>,
    pub emits_focus_kinds: Vec<DiagnosticFocusKind>,
}

impl From<&DiagnosticProviderDescriptor> for DiagnosticCapabilities {
    fn from(descriptor: &DiagnosticProviderDescriptor) -> Self {
        Self {
            actions: descriptor.actions.to_vec(),
            findings_target_kinds: descriptor.findings_target_kinds.to_vec(),
            emits_focus_kinds: descriptor.emits_focus_kinds.to_vec(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiagnosticProviderSection {
    pub id: &'static str,
    pub status: DiagnosticProviderStatus,
    pub complete: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub capabilities: Option<DiagnosticCapabilities>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub readiness: Option<DiagnosticReadiness>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub items_total: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub items_returned: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resource_failures: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub truncated: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<DiagnosticError>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum DiagnosticItem {
    Diagnostic {
        provider: &'static str,
        location: SourceLocation,
        #[serde(skip_serializing_if = "Option::is_none")]
        location_reason: Option<UnaddressableReason>,
        focus: DiagnosticFocus,
        code: String,
        severity: DiagnosticSeverity,
        message: String,
        tags: Vec<DiagnosticTag>,
    },
    ResourceFailure {
        provider: &'static str,
        location: SourceLocation,
        #[serde(skip_serializing_if = "Option::is_none")]
        location_reason: Option<UnaddressableReason>,
        error: DiagnosticError,
    },
    DiagnosticRule {
        provider: &'static str,
        code: String,
        default_severity: DiagnosticSeverity,
        title: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        description: Option<String>,
        tags: Vec<DiagnosticTag>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiagnosticResult {
    pub ok: bool,
    pub action: DiagnosticAction,
    pub selection: DiagnosticSelection,
    pub state: DiagnosticResultState,
    pub complete: bool,
    pub providers: Vec<DiagnosticProviderSection>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub items_total: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub items_returned: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub truncated: Option<bool>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub items: Vec<DiagnosticItem>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::source_target::{
        MetadataAddress, TargetKind, PLATFORM_XML_8_3_27_FORMAT_2_20,
    };
    use serde_json::{json, Value};
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    const BSL_LANGUAGE_SERVER_PROVIDER: DiagnosticProviderId =
        DiagnosticProviderId::new_const("bsl-language-server");
    const METADATA_VALIDATOR_PROVIDER: DiagnosticProviderId =
        DiagnosticProviderId::new_const("metadata-validator");

    static BSL_LS_DESCRIPTOR: DiagnosticProviderDescriptor = DiagnosticProviderDescriptor {
        id: BSL_LANGUAGE_SERVER_PROVIDER,
        actions: &[DiagnosticAction::Findings, DiagnosticAction::Status],
        findings_target_kinds: &[TargetKind::Module],
        emits_focus_kinds: &[DiagnosticFocusKind::SourceRange],
    };
    static METADATA_DESCRIPTOR: DiagnosticProviderDescriptor = DiagnosticProviderDescriptor {
        id: METADATA_VALIDATOR_PROVIDER,
        actions: &[DiagnosticAction::Analyze, DiagnosticAction::Findings],
        findings_target_kinds: &[TargetKind::MetadataObject],
        emits_focus_kinds: &[DiagnosticFocusKind::Metadata],
    };

    struct StubProvider(&'static DiagnosticProviderDescriptor);

    impl DiagnosticProvider for StubProvider {
        fn descriptor(&self) -> &'static DiagnosticProviderDescriptor {
            self.0
        }

        fn execute(
            &self,
            _request: &DiagnosticProviderRequest,
            _context: &DiagnosticContext,
            _deadline: ProviderDeadline,
            _cancellation: &CancellationToken,
        ) -> DiagnosticProviderOutcome {
            DiagnosticProviderOutcome::empty(DiagnosticProviderStatus::Empty)
        }
    }

    fn provider(descriptor: &'static DiagnosticProviderDescriptor) -> Arc<dyn DiagnosticProvider> {
        Arc::new(StubProvider(descriptor))
    }

    #[test]
    fn zero_width_observation_range_matches_the_requested_range_that_contains_its_caret() {
        let requested = DiagnosticRange {
            start_line: 10,
            start_column: 0,
            end_line: 20,
            end_column: 0,
        };
        let caret = |line: usize, column: usize| DiagnosticRange {
            start_line: line,
            start_column: column,
            end_line: line,
            end_column: column,
        };

        assert!(!caret(10, 0).is_non_empty());
        assert!(
            caret(10, 0).intersects(requested),
            "caret on the first line"
        );
        assert!(
            caret(19, 40).intersects(requested),
            "caret on the last line"
        );
        assert!(
            !caret(9, 40).intersects(requested),
            "caret before the range"
        );
        assert!(
            !caret(20, 0).intersects(requested),
            "the requested end is exclusive"
        );
        assert!(!caret(40, 4).intersects(requested), "caret after the range");
    }

    fn metadata_address(value: &str) -> MetadataAddress {
        MetadataAddress::parse(PLATFORM_XML_8_3_27_FORMAT_2_20, value).unwrap()
    }

    #[test]
    fn provider_registry_preserves_registration_order_and_supports_future_ids() {
        let registry = DiagnosticProviderRegistry::new(vec![
            provider(&METADATA_DESCRIPTOR),
            provider(&BSL_LS_DESCRIPTOR),
        ])
        .unwrap();

        assert_eq!(
            registry.ids().collect::<Vec<_>>(),
            vec![METADATA_VALIDATOR_PROVIDER, BSL_LANGUAGE_SERVER_PROVIDER]
        );
        assert_eq!(
            registry
                .descriptors()
                .map(|descriptor| descriptor.id)
                .collect::<Vec<_>>(),
            vec![METADATA_VALIDATOR_PROVIDER, BSL_LANGUAGE_SERVER_PROVIDER]
        );
    }

    #[test]
    fn provider_registry_rejects_duplicate_ids() {
        let error = DiagnosticProviderRegistry::new(vec![
            provider(&BSL_LS_DESCRIPTOR),
            provider(&BSL_LS_DESCRIPTOR),
        ])
        .unwrap_err();

        assert_eq!(error.provider_id, BSL_LANGUAGE_SERVER_PROVIDER);
    }

    #[test]
    fn addressed_diagnostic_location_matches_source_location_wire_shape() {
        let metadata_path = metadata_address("CommonModule.Diagnostics.Module");
        let diagnostic = SourceLocation::Addressed {
            source_set: "main".to_string(),
            metadata_path: Some(metadata_path),
            target_kind: TargetKind::Module,
        };

        assert_eq!(
            serde_json::to_value(diagnostic).unwrap(),
            json!({
                "kind": "addressed",
                "sourceSet": "main",
                "metadataPath": "CommonModule.Diagnostics.Module",
                "targetKind": "module"
            })
        );
    }

    #[test]
    fn diagnostic_ranges_are_zero_based_half_open_coordinate_objects() {
        let range = DiagnosticRange {
            start_line: 0,
            start_column: 4,
            end_line: 0,
            end_column: 9,
        };

        assert_eq!(
            serde_json::to_value(range).unwrap(),
            json!({
                "startLine": 0,
                "startColumn": 4,
                "endLine": 0,
                "endColumn": 9
            })
        );
    }

    #[test]
    fn metadata_focus_serializes_only_canonical_logical_coordinates() {
        let focus = DiagnosticFocus::Metadata {
            element_path: vec![MetadataElement {
                collection: "attributes".to_string(),
                name: "Price".to_string(),
            }],
            property: Some("Type".to_string()),
            language: Some("ru".to_string()),
        };

        assert_eq!(
            serde_json::to_value(focus).unwrap(),
            json!({
                "kind": "metadata",
                "elementPath": [{"collection": "attributes", "name": "Price"}],
                "property": "Type",
                "language": "ru"
            })
        );
    }

    #[test]
    fn public_diagnostic_result_does_not_expose_provider_transport_details() {
        let location = SourceLocation::Unaddressable {
            source_set: "main".to_string(),
            owner_metadata_path: Some(metadata_address("Catalog.Products")),
            path: "Catalogs/Products/Ext/Unknown.xml".to_string(),
        };
        let result = DiagnosticResult {
            ok: true,
            action: DiagnosticAction::Findings,
            selection: DiagnosticSelection {
                source_set: "main".to_string(),
                metadata_path: None,
                target_kind: Some(TargetKind::SourceRoot),
                providers: vec!["bsl-language-server"],
                filter: Some(DiagnosticFilter::default()),
                limit: Some(200),
            },
            state: DiagnosticResultState::Completed,
            complete: true,
            providers: vec![DiagnosticProviderSection {
                id: "bsl-language-server",
                status: DiagnosticProviderStatus::Completed,
                complete: true,
                version: Some("1.0".to_string()),
                capabilities: None,
                readiness: None,
                items_total: Some(1),
                items_returned: Some(1),
                resource_failures: Some(0),
                truncated: Some(false),
                error: None,
            }],
            items_total: Some(1),
            items_returned: Some(1),
            truncated: Some(false),
            items: vec![DiagnosticItem::Diagnostic {
                provider: "bsl-language-server",
                location,
                location_reason: Some(UnaddressableReason::ResourceNotAddressable),
                focus: DiagnosticFocus::Target,
                code: "LS001".to_string(),
                severity: DiagnosticSeverity::Warning,
                message: "Logical finding".to_string(),
                tags: vec![DiagnosticTag::Unnecessary],
            }],
        };

        let value = serde_json::to_value(result).unwrap();
        assert_public_keys_are_transport_neutral(&value);
        assert_eq!(
            value["items"][0]["location"]["path"],
            json!("Catalogs/Products/Ext/Unknown.xml")
        );
        assert_eq!(
            value["items"][0]["locationReason"],
            json!("resourceNotAddressable")
        );
        assert!(value["items"][0]["location"].get("observedPath").is_none());
        assert!(value["items"][0]["location"].get("reason").is_none());
    }

    fn assert_public_keys_are_transport_neutral(value: &Value) {
        match value {
            Value::Object(object) => {
                for (key, child) in object {
                    assert!(
                        !matches!(
                            key.as_str(),
                            "observedPath" | "uri" | "transport" | "command" | "stdout" | "stderr"
                        ),
                        "public diagnostics leaked transport field `{key}`: {value}"
                    );
                    assert_public_keys_are_transport_neutral(child);
                }
            }
            Value::Array(items) => {
                for item in items {
                    assert_public_keys_are_transport_neutral(item);
                }
            }
            _ => {}
        }
    }

    #[test]
    fn provider_deadline_type_remains_shared_with_other_provider_domains() {
        let deadline = ProviderDeadline::new(Instant::now() + Duration::from_secs(1));
        assert!(deadline.remaining() <= Duration::from_secs(1));
    }
}
