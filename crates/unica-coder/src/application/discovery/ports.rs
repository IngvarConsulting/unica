use super::contract::{DiscoverRequest, Proposal};
use super::determinism::evidence_record_digest;
use super::model::{
    CheckState, Coverage, EvidencePort, EvidenceProvider, EvidenceRecord, ProviderFact,
    ProviderOutcomeSnapshot, ProviderReadiness, ReceiptEligibility,
};
use crate::domain::source_snapshot::{
    ResolvedSourceSelection, ResolvedSourceSet, SourceReadError, SourceSetSnapshot, SourceSnapshot,
};
use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum DiscoveryError {
    Operation(String),
    SourceReadiness(SourceReadinessError),
    SnapshotCapture(SnapshotCaptureError),
    ProviderContractViolation { provider: String, reason: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SnapshotCaptureReason {
    SourceChangedDuringCapture,
    UnsafeSourceTopology,
    SnapshotDeadlineExceeded,
    TransientSourceIo,
    MalformedSourceMaterial,
    UnsupportedSourceLayout,
    InvalidSourcePath,
    SnapshotResourceLimit,
    SnapshotInvariantViolation,
}

impl SnapshotCaptureReason {
    pub(crate) fn reason_code(self) -> &'static str {
        match self {
            Self::SourceChangedDuringCapture => "source_changed_during_capture",
            Self::UnsafeSourceTopology => "unsafe_source_topology",
            Self::SnapshotDeadlineExceeded => "source_snapshot_deadline",
            Self::TransientSourceIo => "source_io_unavailable",
            Self::MalformedSourceMaterial => "malformed_source_material",
            Self::UnsupportedSourceLayout => "unsupported_source_layout",
            Self::InvalidSourcePath => "invalid_source_path",
            Self::SnapshotResourceLimit => "source_snapshot_resource_limit",
            Self::SnapshotInvariantViolation => "source_snapshot_invariant_violation",
        }
    }

    fn retryable(self) -> bool {
        matches!(
            self,
            Self::SourceChangedDuringCapture
                | Self::SnapshotDeadlineExceeded
                | Self::TransientSourceIo
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SnapshotCaptureError {
    pub(crate) reason: SnapshotCaptureReason,
    pub(crate) retryable: bool,
    pub(crate) detail: String,
}

impl SnapshotCaptureError {
    pub(crate) fn new(reason: SnapshotCaptureReason, detail: impl Into<String>) -> Self {
        Self {
            reason,
            retryable: reason.retryable(),
            detail: detail.into(),
        }
    }

    pub(crate) fn classify(detail: impl Into<String>) -> Self {
        let detail = detail.into();
        let reason = if has_any_prefix(
            &detail,
            &["source_mapping_changed:", "source_snapshot_unavailable:"],
        ) {
            SnapshotCaptureReason::SourceChangedDuringCapture
        } else if detail.starts_with("source_snapshot_deadline:") {
            SnapshotCaptureReason::SnapshotDeadlineExceeded
        } else if has_any_prefix(
            &detail,
            &[
                "source_snapshot_file_limit:",
                "source_snapshot_byte_limit:",
                "source_snapshot_traversal_limit:",
                "source_snapshot_traversal_depth:",
                "source_map_config_too_large:",
            ],
        ) {
            SnapshotCaptureReason::SnapshotResourceLimit
        } else if has_any_prefix(
            &detail,
            &[
                "source_root_symlink:",
                "source_root_escape:",
                "symlink_or_reparse_escape:",
                "material_file_not_regular:",
                "material_subtree_not_directory:",
                "file_identity_unavailable:",
                "source_map_config_not_regular:",
                "symlink_or_reparse_marker:",
            ],
        ) {
            SnapshotCaptureReason::UnsafeSourceTopology
        } else if has_any_prefix(
            &detail,
            &[
                "malformed_registration:",
                "malformed_registered_object:",
                "malformed_descriptor:",
                "duplicate_registration:",
                "duplicate_nested_registration:",
                "invalid_registration_value:",
                "registered_object_identity_mismatch:",
                "registered_material_missing:",
                "unknown_registration_kind:",
            ],
        ) {
            SnapshotCaptureReason::MalformedSourceMaterial
        } else if has_any_prefix(
            &detail,
            &[
                "empty_configured_path:",
                "absolute_source_root:",
                "invalid_configured_path:",
                "empty_path_component:",
                "path_traversal:",
                "embedded_current_dir:",
                "path_escape:",
                "invalid_path_component:",
                "invalid_material_path:",
                "non_utf8_material_path:",
            ],
        ) {
            SnapshotCaptureReason::InvalidSourcePath
        } else if has_any_prefix(
            &detail,
            &[
                "unsupported_source_format:",
                "source_root_not_directory:",
                "workspace_root_not_directory:",
            ],
        ) {
            SnapshotCaptureReason::UnsupportedSourceLayout
        } else if has_any_prefix(
            &detail,
            &[
                "workspace_root_unavailable:",
                "source_root_unavailable:",
                "source_root_unreadable:",
                "source_map_config_unavailable:",
                "marker_unavailable:",
                "material_file_unavailable:",
                "material_file_unreadable:",
                "material_subtree_unavailable:",
                "material_subtree_unreadable:",
                "path_unavailable:",
            ],
        ) {
            SnapshotCaptureReason::TransientSourceIo
        } else {
            SnapshotCaptureReason::SnapshotInvariantViolation
        };
        Self::new(reason, detail)
    }

    pub(crate) fn source_changed(detail: impl Into<String>) -> Self {
        Self::new(SnapshotCaptureReason::SourceChangedDuringCapture, detail)
    }

    pub(crate) fn reason_code(&self) -> &'static str {
        self.reason.reason_code()
    }

    pub(crate) fn retryable(&self) -> bool {
        self.retryable
    }
}

fn has_any_prefix(value: &str, prefixes: &[&str]) -> bool {
    prefixes.iter().any(|prefix| value.starts_with(prefix))
}

impl From<SnapshotCaptureError> for DiscoveryError {
    fn from(error: SnapshotCaptureError) -> Self {
        Self::SnapshotCapture(error)
    }
}

impl From<String> for SnapshotCaptureError {
    fn from(detail: String) -> Self {
        Self::classify(detail)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SourceReadinessReason {
    UnknownSourceFormat,
    InvalidSourceFormat,
    UnsupportedSourceKind,
    UnsupportedSourceFormat,
    UnsupportedDestinationKind,
    UnsupportedDestinationFormat,
}

impl SourceReadinessReason {
    pub(crate) fn reason_code(self) -> &'static str {
        match self {
            Self::UnknownSourceFormat => "unknown_source_format",
            Self::InvalidSourceFormat => "invalid_source_format",
            Self::UnsupportedSourceKind => "unsupported_source_kind",
            Self::UnsupportedSourceFormat => "unsupported_source_format",
            Self::UnsupportedDestinationKind => "unsupported_destination_kind",
            Self::UnsupportedDestinationFormat => "unsupported_destination_format",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SourceRole {
    Analysis,
    Destination,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SourceReadinessError {
    pub(crate) reason: SourceReadinessReason,
    pub(crate) role: SourceRole,
    pub(crate) source_set: String,
    pub(crate) retryable: bool,
}

impl SourceReadinessError {
    pub(crate) fn new(reason: SourceReadinessReason, role: SourceRole, source_set: &str) -> Self {
        Self {
            reason,
            role,
            source_set: source_set.to_string(),
            retryable: false,
        }
    }

    pub(crate) fn reason_code(&self) -> &'static str {
        self.reason.reason_code()
    }

    pub(crate) fn retryable(&self) -> bool {
        self.retryable
    }
}

impl fmt::Display for DiscoveryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Operation(message) => formatter.write_str(message),
            Self::SourceReadiness(error) => {
                write!(formatter, "{}: {}", error.reason_code(), error.source_set)
            }
            Self::SnapshotCapture(error) => {
                write!(formatter, "{}: {}", error.reason_code(), error.detail)
            }
            Self::ProviderContractViolation { provider, reason } => {
                write!(formatter, "{provider} contract violation: {reason}")
            }
        }
    }
}

impl std::error::Error for DiscoveryError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DiscoveryExecutionContext {
    pub(crate) workspace_root: String,
    pub(crate) workspace_epoch: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DiscoveryQueryPlan {
    pub(crate) request: DiscoverRequest,
}

impl DiscoveryQueryPlan {
    pub(crate) fn normalized(request: &DiscoverRequest) -> Self {
        let mut request = request.clone();
        request.concepts.sort();
        request.search_terms.sort();
        request.known_artifacts.sort();
        request
            .proposals
            .sort_by(|left, right| left.id.cmp(&right.id));
        Self { request }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ProviderBatch<T> {
    pub(crate) provider: EvidenceProvider,
    pub(crate) records: Vec<T>,
    pub(crate) coverage: Coverage,
    pub(crate) reason_code: Option<String>,
    pub(crate) retryable: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ProviderIssue {
    pub(crate) provider: EvidenceProvider,
    pub(crate) coverage: Coverage,
    pub(crate) reason_code: String,
    pub(crate) retryable: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ProviderOutcome<T> {
    Complete(ProviderBatch<T>),
    Bounded(ProviderBatch<T>),
    Unavailable(ProviderIssue),
    Failed(ProviderIssue),
    ContractViolation(String),
}

impl ProviderOutcome<EvidenceRecord> {
    pub(crate) fn complete(
        provider: EvidenceProvider,
        records: Vec<EvidenceRecord>,
    ) -> Result<Self, String> {
        let batch = ProviderBatch {
            provider,
            records,
            coverage: Coverage::Complete,
            reason_code: None,
            retryable: false,
        };
        validate_batch(&batch)?;
        Ok(Self::Complete(batch))
    }

    pub(crate) fn bounded(
        provider: EvidenceProvider,
        reason_code: &str,
        retryable: bool,
        records: Vec<EvidenceRecord>,
    ) -> Result<Self, String> {
        let batch = ProviderBatch {
            provider,
            records,
            coverage: Coverage::Bounded,
            reason_code: Some(stable_reason(reason_code)?),
            retryable,
        };
        validate_batch(&batch)?;
        Ok(Self::Bounded(batch))
    }

    pub(crate) fn unavailable(
        provider: EvidenceProvider,
        reason_code: &str,
        retryable: bool,
    ) -> Result<Self, String> {
        Ok(Self::Unavailable(ProviderIssue {
            provider,
            coverage: Coverage::Unknown,
            reason_code: stable_reason(reason_code)?,
            retryable,
        }))
    }

    pub(crate) fn failed(
        provider: EvidenceProvider,
        reason_code: &str,
        retryable: bool,
    ) -> Result<Self, String> {
        Ok(Self::Failed(ProviderIssue {
            provider,
            coverage: Coverage::Unknown,
            reason_code: stable_reason(reason_code)?,
            retryable,
        }))
    }

    pub(crate) fn contract_violation(reason: &str) -> Self {
        Self::ContractViolation(reason.to_string())
    }

    pub(crate) fn collect(
        self,
        expected_port: EvidencePort,
    ) -> Result<CollectedProviderOutcome, DiscoveryError> {
        match self {
            Self::Complete(batch) => collect_ready(expected_port, batch, CheckState::Passed),
            Self::Bounded(batch) => collect_ready(expected_port, batch, CheckState::Passed),
            Self::Unavailable(issue) => collect_issue(
                expected_port,
                issue,
                CheckState::Unavailable,
                ProviderReadiness::Unavailable,
            ),
            Self::Failed(issue) => collect_issue(
                expected_port,
                issue,
                CheckState::Failed,
                ProviderReadiness::Failed,
            ),
            Self::ContractViolation(reason) => Err(contract_error(expected_port, reason)),
        }
    }

    pub(crate) fn collect_for_snapshot(
        self,
        expected_port: EvidencePort,
        snapshot: &SourceSnapshot,
    ) -> Result<CollectedProviderOutcome, DiscoveryError> {
        let mut collected = self.collect(expected_port)?;
        validate_freshness_against_snapshot(expected_port, &collected.records, snapshot)?;
        for record in &mut collected.records {
            record.freshness.workspace_epoch = snapshot.workspace_epoch;
        }
        Ok(collected)
    }
}

#[derive(Debug, Clone)]
pub(crate) struct CollectedProviderOutcome {
    pub(crate) port: EvidencePort,
    pub(crate) provider: EvidenceProvider,
    pub(crate) records: Vec<EvidenceRecord>,
    pub(crate) state: CheckState,
    pub(crate) coverage: Coverage,
    pub(crate) reason_code: Option<String>,
    pub(crate) retryable: bool,
    pub(crate) snapshot: ProviderOutcomeSnapshot,
}

impl CollectedProviderOutcome {
    pub(crate) fn is_degraded(&self) -> bool {
        self.coverage != Coverage::Complete || self.state != CheckState::Passed
    }
}

fn collect_ready(
    expected_port: EvidencePort,
    batch: ProviderBatch<EvidenceRecord>,
    state: CheckState,
) -> Result<CollectedProviderOutcome, DiscoveryError> {
    validate_expected_batch(expected_port, &batch)?;
    let snapshot = ProviderOutcomeSnapshot::from_records(
        expected_port,
        &batch.provider.name,
        &batch.provider.version,
        batch.coverage,
        batch.reason_code.clone(),
        &batch.records,
    )
    .map_err(|reason| contract_error(expected_port, reason))?;
    Ok(CollectedProviderOutcome {
        port: expected_port,
        provider: batch.provider,
        records: batch.records,
        state,
        coverage: batch.coverage,
        reason_code: batch.reason_code,
        retryable: batch.retryable,
        snapshot,
    })
}

fn collect_issue(
    expected_port: EvidencePort,
    issue: ProviderIssue,
    state: CheckState,
    readiness: ProviderReadiness,
) -> Result<CollectedProviderOutcome, DiscoveryError> {
    if issue.provider.port != expected_port {
        return Err(contract_error(
            expected_port,
            "provider identity names the wrong evidence port".to_string(),
        ));
    }
    let snapshot = ProviderOutcomeSnapshot::new(
        expected_port,
        &issue.provider.name,
        &issue.provider.version,
        readiness,
        issue.coverage,
        Some(issue.reason_code.clone()),
        Vec::new(),
    )
    .map_err(|reason| contract_error(expected_port, reason))?;
    Ok(CollectedProviderOutcome {
        port: expected_port,
        provider: issue.provider,
        records: Vec::new(),
        state,
        coverage: issue.coverage,
        reason_code: Some(issue.reason_code),
        retryable: issue.retryable,
        snapshot,
    })
}

fn validate_batch(batch: &ProviderBatch<EvidenceRecord>) -> Result<(), String> {
    batch.provider.validate()?;
    for record in &batch.records {
        if record.provider != batch.provider {
            return Err("every fact must carry the batch provider identity".to_string());
        }
        if record.coverage != batch.coverage {
            return Err("every fact must carry the batch coverage".to_string());
        }
        record.freshness.validate()?;
        if let Some(location) = &record.location {
            location.validate()?;
        }
    }
    Ok(())
}

fn validate_expected_batch(
    expected_port: EvidencePort,
    batch: &ProviderBatch<EvidenceRecord>,
) -> Result<(), DiscoveryError> {
    validate_batch(batch).map_err(|reason| contract_error(expected_port, reason))?;
    if batch.provider.port != expected_port {
        return Err(contract_error(
            expected_port,
            "provider identity names the wrong evidence port".to_string(),
        ));
    }
    for record in &batch.records {
        validate_fact_for_port(expected_port, &record.fact)
            .map_err(|reason| contract_error(expected_port, reason))?;
        evidence_record_digest(record)
            .map_err(|error| contract_error(expected_port, error.to_string()))?;
    }
    Ok(())
}

fn validate_freshness_against_snapshot(
    expected_port: EvidencePort,
    records: &[EvidenceRecord],
    snapshot: &SourceSnapshot,
) -> Result<(), DiscoveryError> {
    for record in records {
        let Some(linked) = snapshot.snapshot_named(&record.freshness.source_set) else {
            return Err(contract_error(
                expected_port,
                "evidence freshness names a source set outside the captured snapshot".into(),
            ));
        };
        if linked.source_fingerprint != record.freshness.source_fingerprint {
            return Err(contract_error(
                expected_port,
                "evidence freshness does not match the captured source identity".into(),
            ));
        }
    }
    Ok(())
}

fn validate_fact_for_port(port: EvidencePort, fact: &ProviderFact) -> Result<(), String> {
    let allowed = match port {
        EvidencePort::MetadataCatalog => matches!(
            fact,
            ProviderFact::MetadataPresent { .. }
                | ProviderFact::MetadataAbsent { .. }
                | ProviderFact::PlatformCallback { .. }
                | ProviderFact::Binding { .. }
        ),
        EvidencePort::CodeSearch => matches!(fact, ProviderFact::CodeOccurrence { .. }),
        EvidencePort::Definition => matches!(
            fact,
            ProviderFact::DefinitionPresent { .. } | ProviderFact::DefinitionAbsent { .. }
        ),
        EvidencePort::CallGraph => matches!(fact, ProviderFact::Call { .. }),
        EvidencePort::FormInspection => matches!(fact, ProviderFact::Binding { .. }),
        EvidencePort::SupportState => matches!(fact, ProviderFact::Support { .. }),
    };
    if !allowed {
        return Err("provider returned a fact variant owned by another port".to_string());
    }
    if let ProviderFact::Binding {
        relation, details, ..
    } = fact
    {
        validate_binding_contract(port, *relation, details)?;
    }
    Ok(())
}

fn validate_binding_contract(
    port: EvidencePort,
    relation: super::model::FlowKind,
    details: &super::model::BindingDetails,
) -> Result<(), String> {
    use super::model::{BindingDetails, FlowKind};

    let valid = match details {
        BindingDetails::Structural => {
            port == EvidencePort::MetadataCatalog
                && matches!(relation, FlowKind::Contains | FlowKind::Defines)
        }
        BindingDetails::EventSubscription { .. } => {
            port == EvidencePort::MetadataCatalog && relation == FlowKind::Subscribes
        }
        BindingDetails::FormCommand { .. } => {
            port == EvidencePort::FormInspection && relation == FlowKind::Handles
        }
        BindingDetails::CommonCommand { .. }
        | BindingDetails::ScheduledJob { .. }
        | BindingDetails::HttpRoute { .. }
        | BindingDetails::ExchangePlan { .. } => {
            port == EvidencePort::MetadataCatalog && relation == FlowKind::Handles
        }
    };
    if valid {
        Ok(())
    } else {
        Err("binding details, relation, and supplying port are incompatible".to_string())
    }
}

fn stable_reason(reason: &str) -> Result<String, String> {
    if reason.is_empty()
        || reason.len() > 128
        || !reason
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
    {
        return Err("reason code must be lowercase snake_case".to_string());
    }
    Ok(reason.to_string())
}

fn contract_error(port: EvidencePort, reason: String) -> DiscoveryError {
    DiscoveryError::ProviderContractViolation {
        provider: port.wire_name().to_string(),
        reason,
    }
}

macro_rules! evidence_port {
    ($name:ident, $method:ident) => {
        pub(crate) trait $name {
            fn $method(
                &self,
                plan: &DiscoveryQueryPlan,
                context: &EvidenceExecutionContext<'_>,
            ) -> ProviderOutcome<EvidenceRecord>;
        }
    };
}

evidence_port!(MetadataCatalogPort, metadata);
evidence_port!(CodeSearchPort, search);
evidence_port!(DefinitionPort, definitions);
evidence_port!(CallGraphPort, calls);
evidence_port!(FormInspectionPort, forms);
evidence_port!(SupportStatePort, support);

pub(crate) struct EvidenceExecutionContext<'a> {
    pub(crate) workspace: &'a DiscoveryExecutionContext,
    pub(crate) snapshot: &'a SourceSnapshot,
    pub(crate) source_reader: &'a dyn SourceSnapshotPort,
}

pub(crate) trait ProjectSourceResolverPort {
    fn resolve_all(
        &self,
        context: &DiscoveryExecutionContext,
        requested_analysis: Option<&str>,
        requested_mutations: &[String],
    ) -> Result<ResolvedSourceSelection, DiscoveryError>;
}

pub(crate) trait SourceSnapshotPort {
    fn capture(
        &self,
        analysis: &ResolvedSourceSet,
        mutation_sources: &[ResolvedSourceSet],
        workspace_epoch: u64,
    ) -> Result<SourceSnapshot, SnapshotCaptureError>;

    fn read_verified(
        &self,
        snapshot: &SourceSetSnapshot,
        workspace_relative_path: &str,
    ) -> Result<Vec<u8>, SourceReadError> {
        let _ = snapshot;
        Err(SourceReadError::SnapshotUnavailable {
            path: workspace_relative_path.to_string(),
            detail: "snapshot reader is not implemented".into(),
        })
    }

    fn read_optional_verified(
        &self,
        snapshot: &SourceSetSnapshot,
        workspace_relative_path: &str,
    ) -> Result<Option<Vec<u8>>, SourceReadError> {
        self.read_verified(snapshot, workspace_relative_path)
            .map(Some)
    }
}

pub(crate) struct ReceiptIssuanceRequest<'a> {
    pub(crate) proposals: &'a [Proposal],
    pub(crate) snapshot: &'a SourceSnapshot,
}

pub(crate) trait ReceiptIssuerPort {
    fn assess(
        &self,
        request: &ReceiptIssuanceRequest<'_>,
    ) -> Result<ReceiptEligibility, DiscoveryError>;
}

pub(crate) struct NoopReceiptIssuer;

impl ReceiptIssuerPort for NoopReceiptIssuer {
    fn assess(
        &self,
        _request: &ReceiptIssuanceRequest<'_>,
    ) -> Result<ReceiptEligibility, DiscoveryError> {
        Ok(ReceiptEligibility {
            eligible: false,
            blockers: vec!["receipt_store_not_implemented".to_string()],
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::discovery::contract::{ArtifactKind, ArtifactRef, ExecutionContext};
    use crate::application::discovery::determinism::canonicalize_evidence;
    use crate::application::discovery::model::{
        BindingDetails, FlowKind, Freshness, HttpVerb, ProviderFact,
    };
    use crate::domain::project_sources::{SourceFormat, SourceSetKind};
    use crate::domain::source_snapshot::{
        ManifestEntry, MaterialFile, ResolvedSourceSet, SourceManifest, SourceSetSnapshot,
        SourceSnapshot,
    };
    use std::collections::BTreeMap;

    const FINGERPRINT: &str =
        "sha256:1111111111111111111111111111111111111111111111111111111111111111";

    #[test]
    fn snapshot_capture_retry_matrix_is_stable_and_typed() {
        let reasons = [
            SnapshotCaptureReason::SourceChangedDuringCapture,
            SnapshotCaptureReason::UnsafeSourceTopology,
            SnapshotCaptureReason::SnapshotDeadlineExceeded,
            SnapshotCaptureReason::TransientSourceIo,
            SnapshotCaptureReason::MalformedSourceMaterial,
            SnapshotCaptureReason::UnsupportedSourceLayout,
            SnapshotCaptureReason::InvalidSourcePath,
            SnapshotCaptureReason::SnapshotResourceLimit,
            SnapshotCaptureReason::SnapshotInvariantViolation,
        ];
        assert_eq!(
            reasons.map(SnapshotCaptureReason::reason_code),
            [
                "source_changed_during_capture",
                "unsafe_source_topology",
                "source_snapshot_deadline",
                "source_io_unavailable",
                "malformed_source_material",
                "unsupported_source_layout",
                "invalid_source_path",
                "source_snapshot_resource_limit",
                "source_snapshot_invariant_violation",
            ]
        );
        assert_eq!(
            reasons.map(SnapshotCaptureReason::retryable),
            [true, false, true, true, false, false, false, false, false]
        );
        for (detail, reason, retryable) in [
            (
                "source_mapping_changed: source map changed during resolution",
                SnapshotCaptureReason::SourceChangedDuringCapture,
                true,
            ),
            (
                "source_snapshot_unavailable: concurrent mutation",
                SnapshotCaptureReason::SourceChangedDuringCapture,
                true,
            ),
            (
                "source_snapshot_deadline: authoritative snapshot discarded",
                SnapshotCaptureReason::SnapshotDeadlineExceeded,
                true,
            ),
            (
                "material_file_unreadable: transient read failure",
                SnapshotCaptureReason::TransientSourceIo,
                true,
            ),
            (
                "source_snapshot_file_limit: authoritative snapshot discarded",
                SnapshotCaptureReason::SnapshotResourceLimit,
                false,
            ),
            (
                "source_snapshot_byte_limit: authoritative snapshot discarded",
                SnapshotCaptureReason::SnapshotResourceLimit,
                false,
            ),
            (
                "source_snapshot_traversal_limit: authoritative snapshot discarded",
                SnapshotCaptureReason::SnapshotResourceLimit,
                false,
            ),
            (
                "source_snapshot_traversal_depth: authoritative snapshot discarded",
                SnapshotCaptureReason::SnapshotResourceLimit,
                false,
            ),
            (
                "symlink_or_reparse_escape: stable component",
                SnapshotCaptureReason::UnsafeSourceTopology,
                false,
            ),
            (
                "malformed_registered_object: invalid XML",
                SnapshotCaptureReason::MalformedSourceMaterial,
                false,
            ),
            (
                "unknown_registration_kind: FutureObject",
                SnapshotCaptureReason::MalformedSourceMaterial,
                false,
            ),
        ] {
            let error = SnapshotCaptureError::classify(detail);
            assert_eq!(error.reason, reason, "{detail}");
            assert_eq!(error.retryable(), retryable, "{detail}");
        }
    }

    fn binding_outcome(
        port: EvidencePort,
        relation: FlowKind,
        details: BindingDetails,
    ) -> ProviderOutcome<EvidenceRecord> {
        let provider =
            EvidenceProvider::new(port, &format!("test-{}", port.wire_name()), "1").unwrap();
        let subject = ArtifactRef::parse(ArtifactKind::Module, "CommonModule.Entry").unwrap();
        let object = ArtifactRef::parse(ArtifactKind::Method, "CommonModule.Flow.Run").unwrap();
        ProviderOutcome::complete(
            provider.clone(),
            vec![EvidenceRecord::from_fact(
                ProviderFact::Binding {
                    subject,
                    object,
                    relation,
                    details,
                },
                None,
                provider,
                Coverage::Complete,
                Freshness::new("main", FINGERPRINT, 1).unwrap(),
            )],
        )
        .unwrap()
    }

    fn captured_snapshot() -> SourceSnapshot {
        let source = ResolvedSourceSet::new(
            "main".into(),
            SourceSetKind::Configuration,
            "src".into(),
            SourceFormat::PlatformXml,
            format!("sha256:{}", "a".repeat(64)),
        )
        .unwrap();
        let manifest = SourceManifest::new(BTreeMap::from([(
            "src/Configuration.xml".into(),
            ManifestEntry::Present(
                MaterialFile::new(1, format!("sha256:{}", "b".repeat(64))).unwrap(),
            ),
        )]))
        .unwrap();
        SourceSnapshot::new(
            SourceSetSnapshot::from_manifest(source, manifest).unwrap(),
            vec![],
            9,
        )
        .unwrap()
    }

    #[test]
    fn freshness_binds_source_identity_but_epoch_is_diagnostic_only() {
        let snapshot = captured_snapshot();
        let fingerprint = snapshot.analysis.source_fingerprint.clone();
        let mut older_epoch = binding_outcome(
            EvidencePort::MetadataCatalog,
            FlowKind::Contains,
            BindingDetails::Structural,
        );
        let ProviderOutcome::Complete(batch) = &mut older_epoch else {
            unreachable!()
        };
        batch.records[0].freshness = Freshness::new("main", &fingerprint, 1).unwrap();
        assert!(older_epoch
            .collect_for_snapshot(EvidencePort::MetadataCatalog, &snapshot)
            .is_ok());

        let ProviderOutcome::Complete(mut batch) = binding_outcome(
            EvidencePort::MetadataCatalog,
            FlowKind::Contains,
            BindingDetails::Structural,
        ) else {
            unreachable!()
        };
        batch.records[0].freshness = Freshness::new("main", &fingerprint, 1).unwrap();
        let mut current = batch.records[0].clone();
        current.freshness.workspace_epoch = snapshot.workspace_epoch;
        let provider = batch.provider.clone();
        let forward = ProviderOutcome::complete(
            provider.clone(),
            vec![batch.records[0].clone(), current.clone()],
        )
        .unwrap()
        .collect_for_snapshot(EvidencePort::MetadataCatalog, &snapshot)
        .unwrap();
        let reverse = ProviderOutcome::complete(provider, vec![current, batch.records.remove(0)])
            .unwrap()
            .collect_for_snapshot(EvidencePort::MetadataCatalog, &snapshot)
            .unwrap();
        let forward = canonicalize_evidence(forward.records).unwrap();
        let reverse = canonicalize_evidence(reverse.records).unwrap();
        assert_eq!(forward, reverse);
        assert_eq!(forward.len(), 1);
        assert_eq!(
            forward[0].freshness.workspace_epoch,
            snapshot.workspace_epoch
        );

        for (source_set, source_fingerprint) in
            [("other", fingerprint.as_str()), ("main", FINGERPRINT)]
        {
            let mut invalid = binding_outcome(
                EvidencePort::MetadataCatalog,
                FlowKind::Contains,
                BindingDetails::Structural,
            );
            let ProviderOutcome::Complete(batch) = &mut invalid else {
                unreachable!()
            };
            batch.records[0].freshness = Freshness::new(source_set, source_fingerprint, 9).unwrap();
            assert!(matches!(
                invalid.collect_for_snapshot(EvidencePort::MetadataCatalog, &snapshot),
                Err(DiscoveryError::ProviderContractViolation { .. })
            ));
        }
    }

    fn binding_cases() -> Vec<(BindingDetails, Vec<(EvidencePort, FlowKind)>)> {
        vec![
            (
                BindingDetails::Structural,
                vec![
                    (EvidencePort::MetadataCatalog, FlowKind::Contains),
                    (EvidencePort::MetadataCatalog, FlowKind::Defines),
                ],
            ),
            (
                BindingDetails::EventSubscription {
                    event: "BeforeWrite".into(),
                    context: ExecutionContext::Server,
                },
                vec![(EvidencePort::MetadataCatalog, FlowKind::Subscribes)],
            ),
            (
                BindingDetails::FormCommand {
                    action: "Run".into(),
                    context: ExecutionContext::Client,
                },
                vec![(EvidencePort::FormInspection, FlowKind::Handles)],
            ),
            (
                BindingDetails::CommonCommand {
                    action: "Run".into(),
                    context: ExecutionContext::Client,
                },
                vec![(EvidencePort::MetadataCatalog, FlowKind::Handles)],
            ),
            (
                BindingDetails::ScheduledJob {
                    enabled: true,
                    context: ExecutionContext::Server,
                },
                vec![(EvidencePort::MetadataCatalog, FlowKind::Handles)],
            ),
            (
                BindingDetails::HttpRoute {
                    verb: HttpVerb::Post,
                    url_template: "/flow".into(),
                    context: ExecutionContext::Server,
                },
                vec![(EvidencePort::MetadataCatalog, FlowKind::Handles)],
            ),
            (
                BindingDetails::ExchangePlan {
                    event: "OnReceive".into(),
                    context: ExecutionContext::Server,
                },
                vec![(EvidencePort::MetadataCatalog, FlowKind::Handles)],
            ),
        ]
    }

    #[test]
    fn binding_contract_accepts_every_canonical_detail_relation_and_port() {
        for (details, allowed) in binding_cases() {
            for (port, relation) in allowed {
                binding_outcome(port, relation, details.clone())
                    .collect(port)
                    .unwrap();
            }
        }
    }

    #[test]
    fn binding_contract_rejects_every_incompatible_relation_or_supplying_port() {
        let ports = [
            EvidencePort::MetadataCatalog,
            EvidencePort::CodeSearch,
            EvidencePort::Definition,
            EvidencePort::CallGraph,
            EvidencePort::FormInspection,
            EvidencePort::SupportState,
        ];
        let relations = [
            FlowKind::Contains,
            FlowKind::Defines,
            FlowKind::Calls,
            FlowKind::Handles,
            FlowKind::Subscribes,
            FlowKind::Uses,
        ];
        for (details, allowed) in binding_cases() {
            for port in ports {
                for relation in relations {
                    if allowed.contains(&(port, relation)) {
                        continue;
                    }
                    assert!(matches!(
                        binding_outcome(port, relation, details.clone()).collect(port),
                        Err(DiscoveryError::ProviderContractViolation { .. })
                    ));
                }
            }
        }
    }

    #[test]
    fn form_inspection_calls_structural_is_rejected_before_graph_promotion() {
        assert!(matches!(
            binding_outcome(
                EvidencePort::FormInspection,
                FlowKind::Calls,
                BindingDetails::Structural,
            )
            .collect(EvidencePort::FormInspection),
            Err(DiscoveryError::ProviderContractViolation { .. })
        ));
    }
}
