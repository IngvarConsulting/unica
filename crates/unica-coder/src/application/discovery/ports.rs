use super::contract::{DiscoverRequest, Proposal};
use super::determinism::evidence_record_digest;
use super::model::{
    CheckState, Coverage, EvidencePort, EvidenceProvider, EvidenceRecord, ProviderFact,
    ProviderOutcomeSnapshot, ProviderReadiness, ReceiptEligibility,
};
use crate::domain::source_snapshot::{ResolvedSourceSet, SourceSnapshot};
use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum DiscoveryError {
    Operation(String),
    ProviderContractViolation { provider: String, reason: String },
}

impl fmt::Display for DiscoveryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Operation(message) => formatter.write_str(message),
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
                context: &DiscoveryExecutionContext,
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

pub(crate) trait ProjectSourceResolverPort {
    fn resolve(
        &self,
        context: &DiscoveryExecutionContext,
        requested_source_set: Option<&str>,
    ) -> Result<ResolvedSourceSet, DiscoveryError>;
}

pub(crate) trait SourceSnapshotPort {
    fn capture(
        &self,
        analysis: &ResolvedSourceSet,
        mutation_sources: &[ResolvedSourceSet],
        workspace_epoch: u64,
    ) -> Result<SourceSnapshot, DiscoveryError>;
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
    use crate::application::discovery::model::{
        BindingDetails, FlowKind, Freshness, HttpVerb, ProviderFact,
    };

    const FINGERPRINT: &str =
        "sha256:1111111111111111111111111111111111111111111111111111111111111111";

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
