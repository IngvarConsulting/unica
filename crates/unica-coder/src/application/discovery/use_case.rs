use super::contract::{DiscoverMode, DiscoverRequest, MutationIntent};
use super::determinism::{analysis_id, canonicalize_evidence, evidence_id};
use super::evidence_graph::EvidenceGraph;
use super::model::{
    Check, CheckOutcome, CheckSeverity, CheckState, Coverage, DiscoveryReport, DiscoverySource,
    DiscoveryStatus, EvidencePort, FactAnswer, LinkedSourceSnapshot, ProposalFacts,
    ProposalVerdict, ReceiptEligibility, SourceSnapshotRole, SupportState, Verdict,
};
use super::ports::{
    CallGraphPort, CodeSearchPort, CollectedProviderOutcome, DefinitionPort, DiscoveryError,
    DiscoveryExecutionContext, DiscoveryQueryPlan, EvidenceExecutionContext, FormInspectionPort,
    MetadataCatalogPort, ProjectSourceResolverPort, ReceiptIssuanceRequest, ReceiptIssuerPort,
    SnapshotCaptureError, SnapshotCaptureReason, SourceReadinessError, SourceReadinessReason,
    SourceRole, SourceSnapshotPort, SupportStatePort,
};
use super::proposal_validator::{ProposalValidation, ProposalValidator};
use crate::domain::project_sources::{SourceFormat, SourceSetKind};
use crate::domain::source_snapshot::{ResolvedSourceSelection, ResolvedSourceSet, SourceSnapshot};
use std::collections::BTreeSet;

const ANALYSIS_CONTRACT_VERSION: &str = "project-discovery-v1";

pub(crate) struct DiscoverExtensionPointsUseCase<'a> {
    source_resolver: &'a dyn ProjectSourceResolverPort,
    snapshot_port: &'a dyn SourceSnapshotPort,
    metadata_catalog: &'a dyn MetadataCatalogPort,
    code_search: &'a dyn CodeSearchPort,
    definitions: &'a dyn DefinitionPort,
    call_graph: &'a dyn CallGraphPort,
    form_inspection: &'a dyn FormInspectionPort,
    support_state: &'a dyn SupportStatePort,
    receipt_issuer: &'a dyn ReceiptIssuerPort,
}

impl<'a> DiscoverExtensionPointsUseCase<'a> {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        source_resolver: &'a dyn ProjectSourceResolverPort,
        snapshot_port: &'a dyn SourceSnapshotPort,
        metadata_catalog: &'a dyn MetadataCatalogPort,
        code_search: &'a dyn CodeSearchPort,
        definitions: &'a dyn DefinitionPort,
        call_graph: &'a dyn CallGraphPort,
        form_inspection: &'a dyn FormInspectionPort,
        support_state: &'a dyn SupportStatePort,
        receipt_issuer: &'a dyn ReceiptIssuerPort,
    ) -> Self {
        Self {
            source_resolver,
            snapshot_port,
            metadata_catalog,
            code_search,
            definitions,
            call_graph,
            form_inspection,
            support_state,
            receipt_issuer,
        }
    }

    pub(crate) fn execute(
        &self,
        context: DiscoveryExecutionContext,
        request: DiscoverRequest,
    ) -> Result<DiscoveryReport, DiscoveryError> {
        if context.workspace_root.trim().is_empty() {
            return Err(DiscoveryError::Operation(
                "workspace root must not be blank".to_string(),
            ));
        }
        let mutation_names = mutation_source_names(&request);
        let resolved_sources = self.source_resolver.resolve_all(
            &context,
            request.source_set.as_deref(),
            &mutation_names,
        )?;
        validate_resolved_source_roles(&resolved_sources)?;
        let analysis_source = resolved_sources.analysis;
        analysis_source
            .validate()
            .map_err(DiscoveryError::Operation)?;
        let mutation_sources = resolved_sources.mutations;
        let captured_mutation_sources = if analysis_source.source_format
            == crate::domain::project_sources::SourceFormat::PlatformXml
        {
            mutation_sources.as_slice()
        } else {
            &[]
        };
        let mut snapshot = self.snapshot_port.capture(
            &analysis_source,
            captured_mutation_sources,
            context.workspace_epoch,
        )?;
        snapshot.validate().map_err(snapshot_invariant_error)?;
        validate_captured_snapshot(&snapshot, &analysis_source, captured_mutation_sources)?;
        snapshot.workspace_epoch = context.workspace_epoch;

        let plan = DiscoveryQueryPlan::normalized(&request);
        if snapshot.analysis.source_set.source_format
            != crate::domain::project_sources::SourceFormat::PlatformXml
        {
            return unsupported_source_format_report(&plan, &snapshot);
        }
        let evidence_context = EvidenceExecutionContext {
            workspace: &context,
            snapshot: &snapshot,
            source_reader: self.snapshot_port,
        };
        let providers = vec![
            self.metadata_catalog
                .metadata(&plan, &evidence_context)
                .collect_for_snapshot(EvidencePort::MetadataCatalog, &snapshot)?,
            self.code_search
                .search(&plan, &evidence_context)
                .collect_for_snapshot(EvidencePort::CodeSearch, &snapshot)?,
            self.definitions
                .definitions(&plan, &evidence_context)
                .collect_for_snapshot(EvidencePort::Definition, &snapshot)?,
            self.call_graph
                .calls(&plan, &evidence_context)
                .collect_for_snapshot(EvidencePort::CallGraph, &snapshot)?,
            self.form_inspection
                .forms(&plan, &evidence_context)
                .collect_for_snapshot(EvidencePort::FormInspection, &snapshot)?,
            self.support_state
                .support(&plan, &evidence_context)
                .collect_for_snapshot(EvidencePort::SupportState, &snapshot)?,
        ];
        let records = providers
            .iter()
            .flat_map(|provider| provider.records.iter().cloned())
            .collect::<Vec<_>>();
        let graph = EvidenceGraph::build(&records).map_err(DiscoveryError::Operation)?;
        let validation = ProposalValidator::validate(&plan.request.proposals, &graph, &providers)
            .map_err(DiscoveryError::Operation)?;
        let checks = build_checks(&providers, &graph, &validation)?;
        let status = report_status(plan.request.mode, &graph, &validation, &checks);
        let receipt_eligibility =
            self.receipt_eligibility(&plan.request, &snapshot, &validation, &checks)?;
        let source = discovery_source(&snapshot);
        let snapshots = providers
            .iter()
            .map(|provider| provider.snapshot.clone())
            .collect::<Vec<_>>();
        let analysis_id = analysis_id(
            &plan.request,
            ANALYSIS_CONTRACT_VERSION,
            &source,
            &snapshots,
        )
        .map_err(|error| DiscoveryError::Operation(error.to_string()))?;
        let evidence = canonicalize_evidence(records)
            .map_err(|error| DiscoveryError::Operation(error.to_string()))?;
        DiscoveryReport::new(
            status,
            analysis_id,
            source,
            graph.related_artifacts,
            graph.flow_edges,
            graph.candidates,
            validation.verdicts,
            evidence,
            checks,
            receipt_eligibility,
        )
        .map_err(DiscoveryError::Operation)
    }

    fn receipt_eligibility(
        &self,
        request: &DiscoverRequest,
        snapshot: &SourceSnapshot,
        validation: &ProposalValidation,
        checks: &[Check],
    ) -> Result<ReceiptEligibility, DiscoveryError> {
        if snapshot.analysis.source_set.source_format
            != crate::domain::project_sources::SourceFormat::PlatformXml
        {
            return Ok(ReceiptEligibility {
                eligible: false,
                blockers: vec!["unsupported_source_format".to_string()],
            });
        }
        let all_supported = request.mode == DiscoverMode::Validate
            && !validation.verdicts.is_empty()
            && validation.verdicts.iter().all(|verdict| {
                verdict.verdict == Verdict::Supported
                    && verdict.coverage_gaps.is_empty()
                    && verdict.blockers.is_empty()
            });
        let material_blocker = checks.iter().any(|check| {
            check.severity == CheckSeverity::Blocking
                && !matches!(
                    check.outcome,
                    CheckOutcome::Satisfied | CheckOutcome::NotApplicable
                )
        });
        if !all_supported || material_blocker {
            let mut blockers = BTreeSet::new();
            if request.mode != DiscoverMode::Validate {
                blockers.insert("validate_mode_required".to_string());
            }
            if !all_supported {
                blockers.insert("proposal_not_supported".to_string());
            }
            if material_blocker {
                blockers.insert("material_check_incomplete".to_string());
            }
            return Ok(ReceiptEligibility {
                eligible: false,
                blockers: blockers.into_iter().collect(),
            });
        }
        self.receipt_issuer.assess(&ReceiptIssuanceRequest {
            proposals: &request.proposals,
            snapshot,
        })
    }
}

fn validate_resolved_source_roles(
    selection: &ResolvedSourceSelection,
) -> Result<(), DiscoveryError> {
    selection.validate().map_err(DiscoveryError::Operation)?;
    let analysis = &selection.analysis;
    if !matches!(
        analysis.kind,
        SourceSetKind::Configuration | SourceSetKind::Extension
    ) {
        return Err(DiscoveryError::SourceReadiness(SourceReadinessError::new(
            SourceReadinessReason::UnsupportedSourceKind,
            SourceRole::Analysis,
            &analysis.name,
        )));
    }
    match analysis.source_format {
        SourceFormat::PlatformXml => {}
        SourceFormat::Edt if analysis.kind == SourceSetKind::Configuration => {}
        SourceFormat::Edt => {
            return Err(DiscoveryError::SourceReadiness(SourceReadinessError::new(
                SourceReadinessReason::UnsupportedSourceFormat,
                SourceRole::Analysis,
                &analysis.name,
            )));
        }
        SourceFormat::Unknown => {
            return Err(DiscoveryError::SourceReadiness(SourceReadinessError::new(
                SourceReadinessReason::UnknownSourceFormat,
                SourceRole::Analysis,
                &analysis.name,
            )));
        }
        SourceFormat::Invalid => {
            return Err(DiscoveryError::SourceReadiness(SourceReadinessError::new(
                SourceReadinessReason::InvalidSourceFormat,
                SourceRole::Analysis,
                &analysis.name,
            )));
        }
    }
    for mutation in &selection.mutations {
        mutation.validate().map_err(DiscoveryError::Operation)?;
        if mutation.kind != SourceSetKind::Extension {
            return Err(DiscoveryError::SourceReadiness(SourceReadinessError::new(
                SourceReadinessReason::UnsupportedDestinationKind,
                SourceRole::Destination,
                &mutation.name,
            )));
        }
        if mutation.source_format != SourceFormat::PlatformXml {
            return Err(DiscoveryError::SourceReadiness(SourceReadinessError::new(
                SourceReadinessReason::UnsupportedDestinationFormat,
                SourceRole::Destination,
                &mutation.name,
            )));
        }
    }
    Ok(())
}

fn mutation_source_names(request: &DiscoverRequest) -> Vec<String> {
    request
        .proposals
        .iter()
        .filter_map(|proposal| proposal.mutation_intent.as_ref())
        .map(|intent| match intent {
            MutationIntent::CfePatchMethod {
                destination_source_set,
                ..
            } => destination_source_set.clone(),
        })
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn unsupported_source_format_report(
    plan: &DiscoveryQueryPlan,
    snapshot: &SourceSnapshot,
) -> Result<DiscoveryReport, DiscoveryError> {
    let source = discovery_source(snapshot);
    let mut affects = plan
        .request
        .proposals
        .iter()
        .map(|proposal| format!("proposal:{}", proposal.id))
        .collect::<Vec<_>>();
    affects.sort();
    let checks = vec![Check::new(
        "source_readiness",
        "ProjectSourceResolverPort",
        CheckState::Skipped,
        CheckOutcome::Inconclusive,
        Coverage::Unknown,
        CheckSeverity::Blocking,
        affects,
        "unsupported_source_format",
        false,
        Vec::new(),
        Vec::new(),
    )
    .map_err(DiscoveryError::Operation)?];
    let proposal_verdicts = plan
        .request
        .proposals
        .iter()
        .map(|proposal| ProposalVerdict {
            proposal_id: proposal.id.clone(),
            verdict: Verdict::Unknown,
            facts: ProposalFacts {
                exists: FactAnswer::Unknown,
                runtime_reachable: FactAnswer::Unknown,
                support: SupportState::Unknown,
            },
            evidence_ids: Vec::new(),
            coverage_gaps: vec!["unsupported_source_format".into()],
            blockers: vec!["unsupported_source_format".into()],
        })
        .collect::<Vec<_>>();
    let analysis_id = analysis_id(&plan.request, ANALYSIS_CONTRACT_VERSION, &source, &[])
        .map_err(|error| DiscoveryError::Operation(error.to_string()))?;
    DiscoveryReport::new(
        DiscoveryStatus::Insufficient,
        analysis_id,
        source,
        Vec::new(),
        Vec::new(),
        Vec::new(),
        proposal_verdicts,
        Vec::new(),
        checks,
        ReceiptEligibility {
            eligible: false,
            blockers: vec!["unsupported_source_format".into()],
        },
    )
    .map_err(DiscoveryError::Operation)
}

fn validate_captured_snapshot(
    snapshot: &SourceSnapshot,
    analysis_source: &ResolvedSourceSet,
    mutation_sources: &[ResolvedSourceSet],
) -> Result<(), SnapshotCaptureError> {
    if snapshot.analysis.source_set != *analysis_source {
        return Err(snapshot_invariant_error(
            "captured analysis snapshot identity differs from the resolved source".to_string(),
        ));
    }
    if snapshot.mutations.len() != mutation_sources.len()
        || mutation_sources.iter().any(|expected| {
            !snapshot
                .mutations
                .iter()
                .any(|actual| actual.source_set == *expected)
        })
    {
        return Err(snapshot_invariant_error(
            "captured mutation snapshot identities differ from the resolved sources".to_string(),
        ));
    }
    Ok(())
}

fn snapshot_invariant_error(detail: String) -> SnapshotCaptureError {
    SnapshotCaptureError::new(SnapshotCaptureReason::SnapshotInvariantViolation, detail)
}

fn discovery_source(snapshot: &SourceSnapshot) -> DiscoverySource {
    let mut linked_source_snapshots = vec![LinkedSourceSnapshot {
        source_set: snapshot.analysis.source_set.name.clone(),
        role: SourceSnapshotRole::Analysis,
        source_fingerprint: snapshot.analysis.source_fingerprint.clone(),
    }];
    linked_source_snapshots.extend(snapshot.mutations.iter().map(|mutation| {
        LinkedSourceSnapshot {
            source_set: mutation.source_set.name.clone(),
            role: SourceSnapshotRole::Mutation,
            source_fingerprint: mutation.source_fingerprint.clone(),
        }
    }));
    DiscoverySource {
        analysis_source_set: snapshot.analysis.source_set.name.clone(),
        source_format: snapshot.analysis.source_set.source_format,
        workspace_epoch: snapshot.workspace_epoch,
        linked_source_snapshots,
        composite_source_fingerprint: snapshot.composite_fingerprint.clone(),
    }
}

fn build_checks(
    providers: &[CollectedProviderOutcome],
    graph: &EvidenceGraph,
    validation: &ProposalValidation,
) -> Result<Vec<Check>, DiscoveryError> {
    providers
        .iter()
        .map(|provider| {
            let conflicts = graph
                .conflicts
                .iter()
                .filter(|conflict| conflict.port == provider.port)
                .collect::<Vec<_>>();
            let provider_evidence_ids = provider
                .records
                .iter()
                .map(evidence_id)
                .collect::<Result<BTreeSet<_>, _>>()
                .map_err(|error| DiscoveryError::Operation(error.to_string()))?;
            let mut affects = validation
                .material_ports
                .iter()
                .filter(|(_, ports)| ports.contains(&provider.port))
                .map(|(proposal_id, _)| proposal_id)
                .map(|proposal_id| format!("proposal:{proposal_id}"))
                .collect::<Vec<_>>();
            affects.extend(
                graph
                    .candidates
                    .iter()
                    .filter(|candidate| {
                        candidate
                            .evidence_ids
                            .iter()
                            .any(|id| provider_evidence_ids.contains(id))
                    })
                    .map(|candidate| format!("candidate:{}", candidate.target.canonical_ref)),
            );
            affects.sort();
            affects.dedup();
            let is_material = affects.iter().any(|affect| affect.starts_with("proposal:"));
            let (outcome, reason_code, severity) = if !conflicts.is_empty() {
                (
                    CheckOutcome::Conflict,
                    "conflicting_evidence".to_string(),
                    if affects.is_empty() {
                        CheckSeverity::Warning
                    } else {
                        CheckSeverity::Blocking
                    },
                )
            } else if provider.is_degraded() {
                (
                    CheckOutcome::Inconclusive,
                    provider
                        .reason_code
                        .clone()
                        .unwrap_or_else(|| "provider_inconclusive".to_string()),
                    if is_material {
                        CheckSeverity::Blocking
                    } else {
                        CheckSeverity::Warning
                    },
                )
            } else if provider.records.is_empty() {
                (
                    CheckOutcome::NoMatch,
                    "no_match".to_string(),
                    CheckSeverity::Info,
                )
            } else {
                (
                    CheckOutcome::Satisfied,
                    "complete".to_string(),
                    CheckSeverity::Info,
                )
            };
            let mut evidence_ids = provider_evidence_ids.into_iter().collect::<Vec<_>>();
            evidence_ids.extend(
                conflicts
                    .iter()
                    .flat_map(|conflict| conflict.evidence_ids.iter().cloned()),
            );
            evidence_ids.sort();
            evidence_ids.dedup();
            Check::new(
                check_code(provider.port),
                provider.port.wire_name(),
                provider.state,
                outcome,
                provider.coverage,
                severity,
                affects,
                &reason_code,
                provider.retryable,
                Vec::new(),
                evidence_ids,
            )
            .map_err(DiscoveryError::Operation)
        })
        .collect()
}

fn check_code(port: EvidencePort) -> &'static str {
    match port {
        EvidencePort::MetadataCatalog => "metadata_catalog",
        EvidencePort::CodeSearch => "code_search",
        EvidencePort::Definition => "definition",
        EvidencePort::CallGraph => "call_graph",
        EvidencePort::FormInspection => "form_inspection",
        EvidencePort::SupportState => "support_state",
    }
}

fn report_status(
    mode: DiscoverMode,
    graph: &EvidenceGraph,
    validation: &ProposalValidation,
    checks: &[Check],
) -> DiscoveryStatus {
    let blocking_unresolved = checks.iter().any(|check| {
        check.severity == CheckSeverity::Blocking
            && !matches!(
                check.outcome,
                CheckOutcome::Satisfied | CheckOutcome::NotApplicable
            )
    });
    let conclusive = match mode {
        DiscoverMode::Explore => graph
            .candidates
            .iter()
            .any(|candidate| candidate.evidence_level == super::model::EvidenceLevel::Actionable),
        DiscoverMode::Validate => {
            !validation.verdicts.is_empty()
                && validation
                    .verdicts
                    .iter()
                    .all(|verdict| verdict.verdict != Verdict::Unknown)
        }
    };
    if blocking_unresolved || !conclusive {
        DiscoveryStatus::Insufficient
    } else if checks.iter().any(|check| {
        check.severity == CheckSeverity::Warning && check.outcome == CheckOutcome::Inconclusive
    }) {
        DiscoveryStatus::Partial
    } else {
        DiscoveryStatus::Complete
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::discovery::contract::{ArtifactKind, ArtifactRef, DiscoverRequest};
    use crate::application::discovery::model::{
        BindingDetails, Coverage, DiscoveryStatus, EvidenceLevel, EvidencePort, EvidenceProvider,
        EvidenceRecord, FactAnswer, FlowKind, Freshness, PlatformCallbackShape, ProviderFact,
        ReceiptEligibility, SourceLocation, SupportState, Verdict,
    };
    use crate::application::discovery::ports::*;
    use crate::domain::project_sources::{SourceFormat, SourceSetKind};
    use crate::domain::source_snapshot::{
        ManifestEntry, MaterialFile, ResolvedSourceSelection, ResolvedSourceSet, SourceManifest,
        SourceSetSnapshot, SourceSnapshot,
    };
    use serde_json::json;
    use std::collections::BTreeMap;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn fake_resolved(name: &str, mutation: bool) -> ResolvedSourceSet {
        ResolvedSourceSet::new(
            name.into(),
            if mutation {
                SourceSetKind::Extension
            } else {
                SourceSetKind::Configuration
            },
            if mutation {
                "src-cfe".into()
            } else {
                "src".into()
            },
            SourceFormat::PlatformXml,
            format!("sha256:{}", "a".repeat(64)),
        )
        .unwrap()
    }

    fn fake_source_snapshot(source_set: ResolvedSourceSet) -> SourceSetSnapshot {
        let path = if source_set.relative_root == "." {
            "Configuration.xml".to_string()
        } else {
            format!("{}/Configuration.xml", source_set.relative_root)
        };
        SourceSetSnapshot::from_manifest(
            source_set,
            SourceManifest::new(BTreeMap::from([(
                path,
                ManifestEntry::Present(
                    MaterialFile::new(1, format!("sha256:{}", "1".repeat(64))).unwrap(),
                ),
            )]))
            .unwrap(),
        )
        .unwrap()
    }

    fn artifact(kind: ArtifactKind, canonical_ref: &str) -> ArtifactRef {
        ArtifactRef::parse(kind, canonical_ref).unwrap()
    }

    fn target() -> ArtifactRef {
        artifact(ArtifactKind::Method, "CommonModule.Flow.Run")
    }

    fn owner() -> ArtifactRef {
        artifact(ArtifactKind::Module, "CommonModule.Flow")
    }

    fn caller() -> ArtifactRef {
        artifact(ArtifactKind::Method, "CommonModule.Entry.Start")
    }

    fn method_proposal() -> DiscoverRequest {
        serde_json::from_value(json!({
            "mode": "validate",
            "task": "validate hook",
            "concepts": ["write"],
            "proposals": [{
                "id": "method-hook",
                "target": {"kind": "method", "ref": "CommonModule.Flow.Run"},
                "intent": "run before write"
            }]
        }))
        .unwrap()
    }

    fn mutation_method_proposal() -> DiscoverRequest {
        serde_json::from_value(json!({
            "mode": "validate",
            "task": "validate mutation hook",
            "concepts": ["write"],
            "proposals": [{
                "id": "method-hook",
                "target": {"kind": "method", "ref": "CommonModule.Flow.Run"},
                "intent": "run before write",
                "mutationIntent": {
                    "tool": "unica.cfe.patch_method",
                    "destinationSourceSet": "extension",
                    "arguments": {
                        "ExtensionPath": "src-cfe",
                        "ModulePath": "CommonModules.Flow.Module",
                        "MethodName": "Run",
                        "InterceptorType": "Before"
                    }
                }
            }]
        }))
        .unwrap()
    }

    fn explore_request() -> DiscoverRequest {
        serde_json::from_value(json!({
            "mode": "explore",
            "task": "find hook",
            "concepts": ["write"],
            "searchTerms": ["Run"]
        }))
        .unwrap()
    }

    fn record(port: EvidencePort, fact: ProviderFact) -> EvidenceRecord {
        record_with_coverage(port, fact, Coverage::Complete)
    }

    fn record_with_coverage(
        port: EvidencePort,
        fact: ProviderFact,
        coverage: Coverage,
    ) -> EvidenceRecord {
        EvidenceRecord::from_fact(
            fact,
            Some(SourceLocation::new("src/Flow.bsl", Some(1), Some(1)).unwrap()),
            EvidenceProvider::new(port, &format!("fake-{}", port.wire_name()), "1").unwrap(),
            coverage,
            Freshness::new(
                "main",
                &fake_source_snapshot(fake_resolved("main", false)).source_fingerprint,
                7,
            )
            .unwrap(),
        )
    }

    fn complete(
        port: EvidencePort,
        records: Vec<EvidenceRecord>,
    ) -> ProviderOutcome<EvidenceRecord> {
        ProviderOutcome::complete(
            EvidenceProvider::new(port, &format!("fake-{}", port.wire_name()), "1").unwrap(),
            records,
        )
        .unwrap()
    }

    fn positive_metadata() -> ProviderOutcome<EvidenceRecord> {
        complete(
            EvidencePort::MetadataCatalog,
            vec![record(
                EvidencePort::MetadataCatalog,
                ProviderFact::MetadataPresent { subject: owner() },
            )],
        )
    }

    fn positive_definition() -> ProviderOutcome<EvidenceRecord> {
        complete(
            EvidencePort::Definition,
            vec![record(
                EvidencePort::Definition,
                ProviderFact::DefinitionPresent {
                    subject: target(),
                    definition: crate::application::discovery::model::DefinitionShape::new(
                        false,
                        true,
                        Vec::new(),
                    )
                    .unwrap(),
                },
            )],
        )
    }

    fn positive_call() -> ProviderOutcome<EvidenceRecord> {
        complete(
            EvidencePort::CallGraph,
            vec![record(
                EvidencePort::CallGraph,
                ProviderFact::Call {
                    subject: caller(),
                    object: target(),
                    resolution: crate::application::discovery::model::CallResolution::Resolved,
                    call_type: crate::application::discovery::model::CallType::Direct,
                    context: crate::application::discovery::contract::ExecutionContext::Server,
                },
            )],
        )
    }

    fn positive_support() -> ProviderOutcome<EvidenceRecord> {
        complete(
            EvidencePort::SupportState,
            vec![record(
                EvidencePort::SupportState,
                ProviderFact::Support {
                    subject: target(),
                    state: SupportState::Editable,
                },
            )],
        )
    }

    fn metadata_callback() -> ProviderOutcome<EvidenceRecord> {
        complete(
            EvidencePort::MetadataCatalog,
            vec![
                record(
                    EvidencePort::MetadataCatalog,
                    ProviderFact::MetadataPresent { subject: owner() },
                ),
                record(
                    EvidencePort::MetadataCatalog,
                    ProviderFact::PlatformCallback {
                        subject: owner(),
                        object: target(),
                        callback: PlatformCallbackShape::new(
                            "8.3.24",
                            "CommonModule",
                            "CommonModule",
                            "Run",
                            true,
                            Vec::new(),
                        )
                        .unwrap(),
                    },
                ),
            ],
        )
    }

    fn form_binding(coverage: Coverage) -> EvidenceRecord {
        record_with_coverage(
            EvidencePort::FormInspection,
            ProviderFact::Binding {
                subject: artifact(
                    ArtifactKind::FormCommand,
                    "Document.Sale.Form.Main.Command.Post",
                ),
                object: target(),
                relation: FlowKind::Handles,
                details: BindingDetails::FormCommand {
                    action: "Run".into(),
                    context: crate::application::discovery::contract::ExecutionContext::Client,
                },
            },
            coverage,
        )
    }

    fn structural_metadata(relation: FlowKind) -> ProviderOutcome<EvidenceRecord> {
        complete(
            EvidencePort::MetadataCatalog,
            vec![
                record(
                    EvidencePort::MetadataCatalog,
                    ProviderFact::MetadataPresent { subject: owner() },
                ),
                record(
                    EvidencePort::MetadataCatalog,
                    ProviderFact::Binding {
                        subject: owner(),
                        object: target(),
                        relation,
                        details: BindingDetails::Structural,
                    },
                ),
            ],
        )
    }

    #[derive(Clone)]
    struct FakeEvidencePorts {
        metadata: ProviderOutcome<EvidenceRecord>,
        code: ProviderOutcome<EvidenceRecord>,
        definitions: ProviderOutcome<EvidenceRecord>,
        calls: ProviderOutcome<EvidenceRecord>,
        forms: ProviderOutcome<EvidenceRecord>,
        support: ProviderOutcome<EvidenceRecord>,
    }

    impl FakeEvidencePorts {
        fn positive() -> Self {
            Self {
                metadata: positive_metadata(),
                code: complete(EvidencePort::CodeSearch, Vec::new()),
                definitions: positive_definition(),
                calls: positive_call(),
                forms: complete(EvidencePort::FormInspection, Vec::new()),
                support: positive_support(),
            }
        }
    }

    macro_rules! fake_port {
        ($trait_name:ident, $method:ident, $field:ident) => {
            impl $trait_name for FakeEvidencePorts {
                fn $method(
                    &self,
                    _plan: &DiscoveryQueryPlan,
                    _context: &EvidenceExecutionContext<'_>,
                ) -> ProviderOutcome<EvidenceRecord> {
                    self.$field.clone()
                }
            }
        };
    }

    fake_port!(MetadataCatalogPort, metadata, metadata);
    fake_port!(CodeSearchPort, search, code);
    fake_port!(DefinitionPort, definitions, definitions);
    fake_port!(CallGraphPort, calls, calls);
    fake_port!(FormInspectionPort, forms, forms);
    fake_port!(SupportStatePort, support, support);

    struct FakeSourceResolver;

    impl ProjectSourceResolverPort for FakeSourceResolver {
        fn resolve_all(
            &self,
            _context: &DiscoveryExecutionContext,
            requested_analysis: Option<&str>,
            requested_mutations: &[String],
        ) -> Result<ResolvedSourceSelection, DiscoveryError> {
            ResolvedSourceSelection::new(
                fake_resolved(requested_analysis.unwrap_or("main"), false),
                requested_mutations
                    .iter()
                    .map(|name| fake_resolved(name, true))
                    .collect(),
            )
            .map_err(DiscoveryError::Operation)
        }
    }

    struct FixedSourceResolver(ResolvedSourceSelection);

    impl ProjectSourceResolverPort for FixedSourceResolver {
        fn resolve_all(
            &self,
            _context: &DiscoveryExecutionContext,
            _requested_analysis: Option<&str>,
            _requested_mutations: &[String],
        ) -> Result<ResolvedSourceSelection, DiscoveryError> {
            Ok(self.0.clone())
        }
    }

    struct PanicSnapshotPort;

    impl SourceSnapshotPort for PanicSnapshotPort {
        fn capture(
            &self,
            _analysis: &ResolvedSourceSet,
            _mutation_sources: &[ResolvedSourceSet],
            _workspace_epoch: u64,
        ) -> Result<SourceSnapshot, SnapshotCaptureError> {
            panic!("invalid resolved source roles must be rejected before capture")
        }
    }

    fn source_with_role_shape(
        name: &str,
        kind: SourceSetKind,
        format: SourceFormat,
    ) -> ResolvedSourceSet {
        ResolvedSourceSet::new(
            name.into(),
            kind,
            format!("src-{name}"),
            format,
            format!("sha256:{}", "a".repeat(64)),
        )
        .unwrap()
    }

    struct FakeSnapshotPort;

    impl SourceSnapshotPort for FakeSnapshotPort {
        fn capture(
            &self,
            analysis: &ResolvedSourceSet,
            mutation_sources: &[ResolvedSourceSet],
            workspace_epoch: u64,
        ) -> Result<SourceSnapshot, SnapshotCaptureError> {
            SourceSnapshot::new(
                fake_source_snapshot(analysis.clone()),
                mutation_sources
                    .iter()
                    .cloned()
                    .map(fake_source_snapshot)
                    .collect(),
                workspace_epoch,
            )
            .map_err(SnapshotCaptureError::classify)
        }
    }

    struct AllowReceiptIssuer;

    impl ReceiptIssuerPort for AllowReceiptIssuer {
        fn assess(
            &self,
            _request: &ReceiptIssuanceRequest<'_>,
        ) -> Result<ReceiptEligibility, DiscoveryError> {
            Ok(ReceiptEligibility {
                eligible: true,
                blockers: Vec::new(),
            })
        }
    }

    struct Fixture {
        ports: FakeEvidencePorts,
    }

    impl Fixture {
        fn positive() -> Self {
            Self {
                ports: FakeEvidencePorts::positive(),
            }
        }

        fn execute(
            &self,
            request: DiscoverRequest,
        ) -> Result<crate::application::discovery::model::DiscoveryReport, DiscoveryError> {
            self.execute_with_snapshot(&FakeSnapshotPort, request)
        }

        fn execute_with_snapshot(
            &self,
            snapshot_port: &dyn SourceSnapshotPort,
            request: DiscoverRequest,
        ) -> Result<crate::application::discovery::model::DiscoveryReport, DiscoveryError> {
            let use_case = DiscoverExtensionPointsUseCase::new(
                &FakeSourceResolver,
                snapshot_port,
                &self.ports,
                &self.ports,
                &self.ports,
                &self.ports,
                &self.ports,
                &self.ports,
                &AllowReceiptIssuer,
            );
            use_case.execute(
                DiscoveryExecutionContext {
                    workspace_root: "/workspace".into(),
                    workspace_epoch: 7,
                },
                request,
            )
        }
    }

    #[test]
    fn complete_empty_definition_batch_can_contradict_exact_proposal() {
        let mut fixture = Fixture::positive();
        fixture.ports.definitions = complete(EvidencePort::Definition, Vec::new());

        let report = fixture.execute(method_proposal()).unwrap();

        assert_eq!(report.proposal_verdicts[0].verdict, Verdict::Contradicted);
        assert_eq!(report.status, DiscoveryStatus::Complete);
        assert!(!report.receipt_eligibility.eligible);
    }

    #[test]
    fn bounded_definition_is_unknown_not_contradicted() {
        let mut fixture = Fixture::positive();
        fixture.ports.definitions = ProviderOutcome::bounded(
            EvidenceProvider::new(EvidencePort::Definition, "fake-definition", "1").unwrap(),
            "result_limit",
            false,
            Vec::new(),
        )
        .unwrap();

        let report = fixture.execute(method_proposal()).unwrap();

        assert_eq!(report.proposal_verdicts[0].verdict, Verdict::Unknown);
        assert_eq!(report.status, DiscoveryStatus::Insufficient);
    }

    #[test]
    fn unavailable_definition_is_unknown_not_contradicted() {
        let mut fixture = Fixture::positive();
        fixture.ports.definitions = ProviderOutcome::unavailable(
            EvidenceProvider::new(EvidencePort::Definition, "fake-definition", "1").unwrap(),
            "index_building",
            true,
        )
        .unwrap();

        let report = fixture.execute(method_proposal()).unwrap();

        assert_eq!(report.proposal_verdicts[0].verdict, Verdict::Unknown);
        assert!(!report.receipt_eligibility.eligible);
    }

    #[test]
    fn unavailable_metadata_prevents_exact_negative_runtime_reachability() {
        let mut fixture = Fixture::positive();
        fixture.ports.metadata = ProviderOutcome::unavailable(
            EvidenceProvider::new(EvidencePort::MetadataCatalog, "fake-metadata", "1").unwrap(),
            "catalog_building",
            true,
        )
        .unwrap();
        fixture.ports.calls = complete(EvidencePort::CallGraph, Vec::new());
        fixture.ports.forms = complete(EvidencePort::FormInspection, Vec::new());

        let report = fixture.execute(method_proposal()).unwrap();

        assert_eq!(
            report.proposal_verdicts[0].facts.runtime_reachable,
            crate::application::discovery::model::FactAnswer::Unknown
        );
        assert_eq!(report.proposal_verdicts[0].verdict, Verdict::Unknown);
        assert_eq!(report.status, DiscoveryStatus::Insufficient);
    }

    #[test]
    fn bounded_metadata_prevents_exact_negative_runtime_reachability() {
        let mut fixture = Fixture::positive();
        fixture.ports.metadata = ProviderOutcome::bounded(
            EvidenceProvider::new(
                EvidencePort::MetadataCatalog,
                &format!("fake-{}", EvidencePort::MetadataCatalog.wire_name()),
                "1",
            )
            .unwrap(),
            "result_limit",
            false,
            vec![record_with_coverage(
                EvidencePort::MetadataCatalog,
                ProviderFact::MetadataPresent { subject: owner() },
                Coverage::Bounded,
            )],
        )
        .unwrap();
        fixture.ports.calls = complete(EvidencePort::CallGraph, Vec::new());
        fixture.ports.forms = complete(EvidencePort::FormInspection, Vec::new());

        let report = fixture.execute(method_proposal()).unwrap();

        assert_eq!(
            report.proposal_verdicts[0].facts.runtime_reachable,
            crate::application::discovery::model::FactAnswer::Unknown
        );
        assert_eq!(report.proposal_verdicts[0].verdict, Verdict::Unknown);
        assert_eq!(report.status, DiscoveryStatus::Insufficient);
    }

    #[test]
    fn failed_definition_is_nonfatal_but_unknown() {
        let mut fixture = Fixture::positive();
        fixture.ports.definitions = ProviderOutcome::failed(
            EvidenceProvider::new(EvidencePort::Definition, "fake-definition", "1").unwrap(),
            "provider_crashed",
            true,
        )
        .unwrap();

        let report = fixture.execute(method_proposal()).unwrap();

        assert_eq!(report.proposal_verdicts[0].verdict, Verdict::Unknown);
        assert!(report.checks.iter().any(|check| {
            check.provider == "DefinitionPort"
                && check.state == crate::application::discovery::model::CheckState::Failed
        }));
    }

    #[test]
    fn provider_contract_violation_is_the_only_fatal_evidence_outcome() {
        let mut fixture = Fixture::positive();
        fixture.ports.definitions = ProviderOutcome::contract_violation("wrong_fact_variant");

        assert!(matches!(
            fixture.execute(method_proposal()),
            Err(DiscoveryError::ProviderContractViolation { .. })
        ));
    }

    #[test]
    fn lexical_evidence_alone_never_yields_actionable_candidate() {
        let mut fixture = Fixture::positive();
        fixture.ports.metadata = complete(EvidencePort::MetadataCatalog, Vec::new());
        fixture.ports.definitions = complete(EvidencePort::Definition, Vec::new());
        fixture.ports.calls = complete(EvidencePort::CallGraph, Vec::new());
        fixture.ports.support = complete(EvidencePort::SupportState, Vec::new());
        fixture.ports.code = complete(
            EvidencePort::CodeSearch,
            vec![record(
                EvidencePort::CodeSearch,
                ProviderFact::CodeOccurrence {
                    subject: target(),
                    search_term: "Run".into(),
                },
            )],
        );

        let report = fixture.execute(explore_request()).unwrap();

        assert!(report.extension_point_candidates.is_empty());
        assert_eq!(
            report.related_artifacts[0].evidence_level,
            crate::application::discovery::model::EvidenceLevel::Lexical
        );
        assert_eq!(report.status, DiscoveryStatus::Insufficient);
    }

    #[test]
    fn connected_target_plus_known_support_is_actionable() {
        let report = Fixture::positive().execute(method_proposal()).unwrap();

        let candidate = report
            .extension_point_candidates
            .iter()
            .find(|candidate| candidate.target == target())
            .unwrap();
        assert_eq!(
            candidate.evidence_level,
            crate::application::discovery::model::EvidenceLevel::Actionable
        );
        assert_eq!(report.proposal_verdicts[0].verdict, Verdict::Supported);
    }

    #[test]
    fn disabled_scheduled_job_binding_is_observed_but_not_actionable() {
        let job = artifact(ArtifactKind::ScheduledJob, "ScheduledJob.Nightly");
        let mut fixture = Fixture::positive();
        fixture.ports.metadata = complete(
            EvidencePort::MetadataCatalog,
            vec![
                record(
                    EvidencePort::MetadataCatalog,
                    ProviderFact::MetadataPresent {
                        subject: job.clone(),
                    },
                ),
                record(
                    EvidencePort::MetadataCatalog,
                    ProviderFact::Binding {
                        subject: job.clone(),
                        object: target(),
                        relation: FlowKind::Handles,
                        details: BindingDetails::ScheduledJob {
                            enabled: false,
                            context:
                                crate::application::discovery::contract::ExecutionContext::Server,
                        },
                    },
                ),
            ],
        );
        fixture.ports.support = complete(
            EvidencePort::SupportState,
            vec![
                record(
                    EvidencePort::SupportState,
                    ProviderFact::Support {
                        subject: target(),
                        state: SupportState::Editable,
                    },
                ),
                record(
                    EvidencePort::SupportState,
                    ProviderFact::Support {
                        subject: job.clone(),
                        state: SupportState::Editable,
                    },
                ),
            ],
        );

        let report = fixture.execute(explore_request()).unwrap();

        assert!(report
            .related_artifacts
            .iter()
            .any(|artifact| artifact.artifact == job));
        assert!(!report
            .extension_point_candidates
            .iter()
            .any(|candidate| candidate.target == job));
    }

    #[test]
    fn material_blocker_survives_unrelated_provider_success() {
        let mut fixture = Fixture::positive();
        fixture.ports.definitions = ProviderOutcome::unavailable(
            EvidenceProvider::new(EvidencePort::Definition, "fake-definition", "1").unwrap(),
            "index_building",
            true,
        )
        .unwrap();
        fixture.ports.code = complete(
            EvidencePort::CodeSearch,
            vec![record(
                EvidencePort::CodeSearch,
                ProviderFact::CodeOccurrence {
                    subject: target(),
                    search_term: "Run".into(),
                },
            )],
        );

        let report = fixture.execute(method_proposal()).unwrap();

        assert!(report.checks.iter().any(|check| {
            check.provider == "DefinitionPort"
                && check.severity == crate::application::discovery::model::CheckSeverity::Blocking
                && check.affects == ["proposal:method-hook"]
        }));
        assert_eq!(report.proposal_verdicts[0].verdict, Verdict::Unknown);
    }

    #[test]
    fn non_material_optional_degradation_is_partial_and_may_keep_eligibility() {
        let mut fixture = Fixture::positive();
        fixture.ports.code = ProviderOutcome::unavailable(
            EvidenceProvider::new(EvidencePort::CodeSearch, "fake-search", "1").unwrap(),
            "index_building",
            true,
        )
        .unwrap();

        let report = fixture.execute(method_proposal()).unwrap();

        assert_eq!(report.status, DiscoveryStatus::Partial);
        assert_eq!(report.proposal_verdicts[0].verdict, Verdict::Supported);
        assert!(report.receipt_eligibility.eligible);
    }

    #[test]
    fn metadata_callback_makes_unavailable_form_inspection_optional() {
        let mut fixture = Fixture::positive();
        fixture.ports.metadata = metadata_callback();
        fixture.ports.calls = complete(EvidencePort::CallGraph, Vec::new());
        fixture.ports.forms = ProviderOutcome::unavailable(
            EvidenceProvider::new(EvidencePort::FormInspection, "fake-forms", "1").unwrap(),
            "form_index_building",
            true,
        )
        .unwrap();

        let report = fixture.execute(method_proposal()).unwrap();

        assert_eq!(report.proposal_verdicts[0].verdict, Verdict::Supported);
        assert_eq!(report.status, DiscoveryStatus::Partial);
        assert!(report.receipt_eligibility.eligible);
        assert!(report.checks.iter().any(|check| {
            check.provider == "FormInspectionPort"
                && check.severity == crate::application::discovery::model::CheckSeverity::Warning
                && !check
                    .affects
                    .iter()
                    .any(|item| item == "proposal:method-hook")
        }));
    }

    #[test]
    fn metadata_callback_makes_bounded_form_inspection_optional() {
        let mut fixture = Fixture::positive();
        fixture.ports.metadata = metadata_callback();
        fixture.ports.calls = complete(EvidencePort::CallGraph, Vec::new());
        fixture.ports.forms = ProviderOutcome::bounded(
            EvidenceProvider::new(
                EvidencePort::FormInspection,
                &format!("fake-{}", EvidencePort::FormInspection.wire_name()),
                "1",
            )
            .unwrap(),
            "result_limit",
            false,
            Vec::new(),
        )
        .unwrap();

        let report = fixture.execute(method_proposal()).unwrap();

        assert_eq!(report.proposal_verdicts[0].verdict, Verdict::Supported);
        assert_eq!(report.status, DiscoveryStatus::Partial);
        assert!(report.receipt_eligibility.eligible);
        assert!(report.checks.iter().any(|check| {
            check.provider == "FormInspectionPort"
                && check.severity == crate::application::discovery::model::CheckSeverity::Warning
                && !check
                    .affects
                    .iter()
                    .any(|item| item == "proposal:method-hook")
        }));
    }

    #[test]
    fn every_runtime_port_that_contributes_a_connection_is_material() {
        let mut fixture = Fixture::positive();
        fixture.ports.forms = ProviderOutcome::bounded(
            EvidenceProvider::new(
                EvidencePort::FormInspection,
                &format!("fake-{}", EvidencePort::FormInspection.wire_name()),
                "1",
            )
            .unwrap(),
            "result_limit",
            false,
            vec![form_binding(Coverage::Bounded)],
        )
        .unwrap();

        let report = fixture.execute(method_proposal()).unwrap();

        assert_eq!(report.proposal_verdicts[0].verdict, Verdict::Supported);
        assert_eq!(report.status, DiscoveryStatus::Insufficient);
        assert!(!report.receipt_eligibility.eligible);
        assert!(report.checks.iter().any(|check| {
            check.provider == "CallGraphPort"
                && check
                    .affects
                    .iter()
                    .any(|item| item == "proposal:method-hook")
        }));
        assert!(report.checks.iter().any(|check| {
            check.provider == "FormInspectionPort"
                && check.severity == crate::application::discovery::model::CheckSeverity::Blocking
                && check
                    .affects
                    .iter()
                    .any(|item| item == "proposal:method-hook")
        }));
    }

    fn assert_structural_edge_is_not_runtime(relation: FlowKind) {
        let mut fixture = Fixture::positive();
        fixture.ports.metadata = structural_metadata(relation);
        fixture.ports.calls = complete(EvidencePort::CallGraph, Vec::new());
        fixture.ports.forms = complete(EvidencePort::FormInspection, Vec::new());

        let report = fixture.execute(method_proposal()).unwrap();

        assert_eq!(report.proposal_verdicts[0].verdict, Verdict::Contradicted);
        assert_eq!(
            report.proposal_verdicts[0].facts.runtime_reachable,
            FactAnswer::No
        );
        assert!(!report.receipt_eligibility.eligible);
        assert!(report
            .flow_edges
            .iter()
            .any(|edge| edge.to == target() && edge.kind == relation));
        assert!(!report
            .extension_point_candidates
            .iter()
            .any(|candidate| candidate.target == target()));
        let related = report
            .related_artifacts
            .iter()
            .find(|artifact| artifact.artifact == target())
            .unwrap();
        assert_eq!(related.evidence_level, EvidenceLevel::Observed);
        assert!(!related
            .reason_codes
            .contains(&"runtime_connected".to_string()));
    }

    #[test]
    fn structural_defines_edge_is_observed_but_never_runtime_reachable() {
        assert_structural_edge_is_not_runtime(FlowKind::Defines);
    }

    #[test]
    fn structural_contains_edge_is_observed_but_never_runtime_reachable() {
        assert_structural_edge_is_not_runtime(FlowKind::Contains);
    }

    #[test]
    fn conflicting_facts_are_retained_and_block_receipt() {
        let mut fixture = Fixture::positive();
        fixture.ports.definitions = complete(
            EvidencePort::Definition,
            vec![
                record(
                    EvidencePort::Definition,
                    ProviderFact::DefinitionPresent {
                        subject: target(),
                        definition: crate::application::discovery::model::DefinitionShape::new(
                            false,
                            true,
                            Vec::new(),
                        )
                        .unwrap(),
                    },
                ),
                record(
                    EvidencePort::Definition,
                    ProviderFact::DefinitionAbsent { subject: target() },
                ),
            ],
        );

        let report = fixture.execute(method_proposal()).unwrap();

        let definition_codes = report
            .evidence
            .iter()
            .filter(|evidence| evidence.subject == target())
            .map(|evidence| evidence.fact_code.as_str())
            .collect::<BTreeSet<_>>();
        assert!(definition_codes.contains("definition_present"));
        assert!(definition_codes.contains("definition_absent"));
        assert_eq!(report.proposal_verdicts[0].verdict, Verdict::Unknown);
        assert!(report
            .checks
            .iter()
            .any(|check| check.outcome
                == crate::application::discovery::model::CheckOutcome::Conflict));
        assert!(!report.receipt_eligibility.eligible);
    }

    #[test]
    fn conflicting_definition_shapes_are_retained_and_block_receipt() {
        let mut fixture = Fixture::positive();
        fixture.ports.definitions = complete(
            EvidencePort::Definition,
            vec![
                record(
                    EvidencePort::Definition,
                    ProviderFact::DefinitionPresent {
                        subject: target(),
                        definition: crate::application::discovery::model::DefinitionShape::new(
                            false,
                            true,
                            Vec::new(),
                        )
                        .unwrap(),
                    },
                ),
                record(
                    EvidencePort::Definition,
                    ProviderFact::DefinitionPresent {
                        subject: target(),
                        definition: crate::application::discovery::model::DefinitionShape::new(
                            true,
                            false,
                            Vec::new(),
                        )
                        .unwrap(),
                    },
                ),
            ],
        );

        let report = fixture.execute(method_proposal()).unwrap();

        assert_eq!(
            report
                .evidence
                .iter()
                .filter(|evidence| evidence.subject == target()
                    && evidence.fact_code == "definition_present")
                .count(),
            2
        );
        assert_eq!(report.proposal_verdicts[0].verdict, Verdict::Unknown);
        assert!(report.proposal_verdicts[0]
            .blockers
            .contains(&"conflicting_definition_shapes".to_string()));
        assert!(report.checks.iter().any(|check| {
            check.provider == "DefinitionPort"
                && check.outcome == crate::application::discovery::model::CheckOutcome::Conflict
                && check.severity == crate::application::discovery::model::CheckSeverity::Blocking
                && check
                    .affects
                    .iter()
                    .any(|affect| affect == "proposal:method-hook")
                && check
                    .affects
                    .iter()
                    .any(|affect| affect.starts_with("candidate:"))
        }));
        assert!(!report.receipt_eligibility.eligible);
    }

    #[test]
    fn bounded_material_records_keep_supported_verdict_but_status_insufficient() {
        let mut fixture = Fixture::positive();
        fixture.ports.definitions = ProviderOutcome::bounded(
            EvidenceProvider::new(
                EvidencePort::Definition,
                &format!("fake-{}", EvidencePort::Definition.wire_name()),
                "1",
            )
            .unwrap(),
            "result_limit",
            false,
            vec![record_with_coverage(
                EvidencePort::Definition,
                ProviderFact::DefinitionPresent {
                    subject: target(),
                    definition: crate::application::discovery::model::DefinitionShape::new(
                        false,
                        true,
                        Vec::new(),
                    )
                    .unwrap(),
                },
                Coverage::Bounded,
            )],
        )
        .unwrap();

        let report = fixture.execute(method_proposal()).unwrap();

        assert_eq!(report.proposal_verdicts[0].verdict, Verdict::Supported);
        assert!(report.checks.iter().any(|check| {
            check.provider == "DefinitionPort"
                && check.severity == crate::application::discovery::model::CheckSeverity::Blocking
                && check.outcome == crate::application::discovery::model::CheckOutcome::Inconclusive
        }));
        assert_eq!(report.status, DiscoveryStatus::Insufficient);
        assert!(!report.receipt_eligibility.eligible);
    }

    struct AliasedAnalysisSnapshotPort;

    struct ConcurrentCaptureFailure;

    struct CorruptSnapshotPort;

    struct OlderEpochSnapshotPort;

    impl SourceSnapshotPort for OlderEpochSnapshotPort {
        fn capture(
            &self,
            analysis: &ResolvedSourceSet,
            mutation_sources: &[ResolvedSourceSet],
            workspace_epoch: u64,
        ) -> Result<SourceSnapshot, SnapshotCaptureError> {
            FakeSnapshotPort.capture(
                analysis,
                mutation_sources,
                workspace_epoch.saturating_sub(1),
            )
        }
    }

    impl SourceSnapshotPort for CorruptSnapshotPort {
        fn capture(
            &self,
            analysis: &ResolvedSourceSet,
            mutation_sources: &[ResolvedSourceSet],
            workspace_epoch: u64,
        ) -> Result<SourceSnapshot, SnapshotCaptureError> {
            let mut snapshot =
                FakeSnapshotPort.capture(analysis, mutation_sources, workspace_epoch)?;
            snapshot.composite_fingerprint = format!("sha256:{}", "f".repeat(64));
            Ok(snapshot)
        }
    }

    impl SourceSnapshotPort for ConcurrentCaptureFailure {
        fn capture(
            &self,
            _analysis: &ResolvedSourceSet,
            _mutation_sources: &[ResolvedSourceSet],
            _workspace_epoch: u64,
        ) -> Result<SourceSnapshot, SnapshotCaptureError> {
            Err(SnapshotCaptureError::source_changed(
                "test source substitution",
            ))
        }
    }

    #[test]
    fn snapshot_capture_error_type_survives_port_and_use_case_boundary() {
        let result =
            Fixture::positive().execute_with_snapshot(&ConcurrentCaptureFailure, method_proposal());

        let Err(DiscoveryError::SnapshotCapture(error)) = result else {
            panic!("typed snapshot capture failure was not preserved");
        };
        assert_eq!(
            error.reason,
            SnapshotCaptureReason::SourceChangedDuringCapture
        );
        assert_eq!(error.reason_code(), "source_changed_during_capture");
        assert!(error.retryable());
    }

    #[test]
    fn invalid_adapter_snapshot_is_a_typed_non_retryable_invariant_failure() {
        let result =
            Fixture::positive().execute_with_snapshot(&CorruptSnapshotPort, method_proposal());

        let Err(DiscoveryError::SnapshotCapture(error)) = result else {
            panic!("invalid adapter snapshot was not a typed snapshot failure");
        };
        assert_eq!(
            error.reason,
            SnapshotCaptureReason::SnapshotInvariantViolation
        );
        assert_eq!(error.reason_code(), "source_snapshot_invariant_violation");
        assert!(!error.retryable());
    }

    #[test]
    fn adapter_snapshot_epoch_is_normalized_as_diagnostic_metadata() {
        let report = Fixture::positive()
            .execute_with_snapshot(&OlderEpochSnapshotPort, method_proposal())
            .unwrap();

        assert_eq!(report.source.workspace_epoch, 7);
        assert!(report
            .evidence
            .iter()
            .all(|evidence| evidence.freshness.workspace_epoch == 7));
    }

    impl SourceSnapshotPort for AliasedAnalysisSnapshotPort {
        fn capture(
            &self,
            analysis: &ResolvedSourceSet,
            _mutation_sources: &[ResolvedSourceSet],
            workspace_epoch: u64,
        ) -> Result<SourceSnapshot, SnapshotCaptureError> {
            let mut aliased_analysis = analysis.clone();
            aliased_analysis.mapping_digest =
                "sha256:dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd"
                    .to_string();
            SourceSnapshot::new(
                fake_source_snapshot(aliased_analysis),
                Vec::new(),
                workspace_epoch,
            )
            .map_err(SnapshotCaptureError::classify)
        }
    }

    #[test]
    fn captured_analysis_snapshot_must_match_resolved_source_identity() {
        let result = Fixture::positive()
            .execute_with_snapshot(&AliasedAnalysisSnapshotPort, method_proposal());

        let Err(DiscoveryError::SnapshotCapture(error)) = result else {
            panic!("analysis identity mismatch was not a typed snapshot failure");
        };
        assert_eq!(
            error.reason,
            SnapshotCaptureReason::SnapshotInvariantViolation
        );
        assert!(!error.retryable());
    }

    #[derive(Clone, Copy)]
    enum MutationSnapshotMismatch {
        Omitted,
        Extra,
        Aliased,
    }

    impl SourceSnapshotPort for MutationSnapshotMismatch {
        fn capture(
            &self,
            analysis: &ResolvedSourceSet,
            mutation_sources: &[ResolvedSourceSet],
            workspace_epoch: u64,
        ) -> Result<SourceSnapshot, SnapshotCaptureError> {
            let mut mutations = mutation_sources
                .iter()
                .cloned()
                .map(fake_source_snapshot)
                .collect::<Vec<_>>();
            match self {
                Self::Omitted => mutations.clear(),
                Self::Extra => mutations.push(fake_source_snapshot(ResolvedSourceSet {
                    name: "extra".into(),
                    kind: SourceSetKind::Extension,
                    relative_root: "extra-src".into(),
                    source_format: SourceFormat::PlatformXml,
                    mapping_digest:
                        "sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc"
                            .into(),
                })),
                Self::Aliased => {
                    let mut source = mutations[0].source_set.clone();
                    source.relative_root = "aliased-src".into();
                    mutations[0] = fake_source_snapshot(source);
                }
            }
            SourceSnapshot::new(
                fake_source_snapshot(analysis.clone()),
                mutations,
                workspace_epoch,
            )
            .map_err(SnapshotCaptureError::classify)
        }
    }

    #[test]
    fn captured_mutation_snapshots_must_match_resolved_sources_exactly() {
        for mismatch in [
            MutationSnapshotMismatch::Omitted,
            MutationSnapshotMismatch::Extra,
            MutationSnapshotMismatch::Aliased,
        ] {
            let result =
                Fixture::positive().execute_with_snapshot(&mismatch, mutation_method_proposal());

            let Err(DiscoveryError::SnapshotCapture(error)) = result else {
                panic!("mutation identity mismatch was not a typed snapshot failure");
            };
            assert_eq!(
                error.reason,
                SnapshotCaptureReason::SnapshotInvariantViolation
            );
            assert!(!error.retryable());
        }
    }

    #[test]
    fn no_actionable_result_is_insufficient_not_operation_error() {
        let ports = FakeEvidencePorts {
            metadata: complete(EvidencePort::MetadataCatalog, Vec::new()),
            code: complete(EvidencePort::CodeSearch, Vec::new()),
            definitions: complete(EvidencePort::Definition, Vec::new()),
            calls: complete(EvidencePort::CallGraph, Vec::new()),
            forms: complete(EvidencePort::FormInspection, Vec::new()),
            support: complete(EvidencePort::SupportState, Vec::new()),
        };

        let report = Fixture { ports }.execute(explore_request()).unwrap();

        assert_eq!(report.status, DiscoveryStatus::Insufficient);
        assert!(report.extension_point_candidates.is_empty());
    }

    #[test]
    fn noop_receipt_issuer_adds_stable_explicit_blocker() {
        let fixture = Fixture::positive();
        let use_case = DiscoverExtensionPointsUseCase::new(
            &FakeSourceResolver,
            &FakeSnapshotPort,
            &fixture.ports,
            &fixture.ports,
            &fixture.ports,
            &fixture.ports,
            &fixture.ports,
            &fixture.ports,
            &NoopReceiptIssuer,
        );

        let report = use_case
            .execute(
                DiscoveryExecutionContext {
                    workspace_root: "/workspace".into(),
                    workspace_epoch: 7,
                },
                method_proposal(),
            )
            .unwrap();

        assert!(!report.receipt_eligibility.eligible);
        assert_eq!(
            report.receipt_eligibility.blockers,
            ["receipt_store_not_implemented"]
        );
    }

    #[test]
    fn edt_analysis_is_independently_ineligible_for_receipt() {
        let mut source = fake_resolved("main", false);
        source.source_format = SourceFormat::Edt;
        let snapshot = SourceSnapshot::new(fake_source_snapshot(source), Vec::new(), 7).unwrap();
        let fixture = Fixture::positive();
        let request = method_proposal();
        let validation = ProposalValidation {
            verdicts: Vec::new(),
            material_ports: BTreeMap::new(),
        };
        let use_case = DiscoverExtensionPointsUseCase::new(
            &FakeSourceResolver,
            &FakeSnapshotPort,
            &fixture.ports,
            &fixture.ports,
            &fixture.ports,
            &fixture.ports,
            &fixture.ports,
            &fixture.ports,
            &AllowReceiptIssuer,
        );

        let eligibility = use_case
            .receipt_eligibility(&request, &snapshot, &validation, &[])
            .unwrap();

        assert!(!eligibility.eligible);
        assert_eq!(eligibility.blockers, ["unsupported_source_format"]);
    }

    #[derive(Default)]
    struct EdtSourceResolver {
        resolved_mutation_count: AtomicUsize,
    }

    #[test]
    fn application_revalidates_resolver_role_kind_and_format_contract() {
        let valid_analysis = source_with_role_shape(
            "analysis",
            SourceSetKind::Configuration,
            SourceFormat::PlatformXml,
        );
        let cases = [
            (
                ResolvedSourceSelection::new(
                    source_with_role_shape(
                        "external",
                        SourceSetKind::ExternalProcessor,
                        SourceFormat::PlatformXml,
                    ),
                    vec![],
                )
                .unwrap(),
                "unsupported_source_kind",
                SourceRole::Analysis,
            ),
            (
                ResolvedSourceSelection::new(
                    source_with_role_shape(
                        "unknown",
                        SourceSetKind::Configuration,
                        SourceFormat::Unknown,
                    ),
                    vec![],
                )
                .unwrap(),
                "unknown_source_format",
                SourceRole::Analysis,
            ),
            (
                ResolvedSourceSelection::new(
                    source_with_role_shape(
                        "edt-extension",
                        SourceSetKind::Extension,
                        SourceFormat::Edt,
                    ),
                    vec![],
                )
                .unwrap(),
                "unsupported_source_format",
                SourceRole::Analysis,
            ),
            (
                ResolvedSourceSelection::new(
                    valid_analysis.clone(),
                    vec![source_with_role_shape(
                        "destination-config",
                        SourceSetKind::Configuration,
                        SourceFormat::PlatformXml,
                    )],
                )
                .unwrap(),
                "unsupported_destination_kind",
                SourceRole::Destination,
            ),
            (
                ResolvedSourceSelection::new(
                    valid_analysis,
                    vec![source_with_role_shape(
                        "destination-edt",
                        SourceSetKind::Extension,
                        SourceFormat::Edt,
                    )],
                )
                .unwrap(),
                "unsupported_destination_format",
                SourceRole::Destination,
            ),
        ];
        let providers = FakeEvidencePorts::positive();
        for (selection, reason, role) in cases {
            let resolver = FixedSourceResolver(selection);
            let use_case = DiscoverExtensionPointsUseCase::new(
                &resolver,
                &PanicSnapshotPort,
                &providers,
                &providers,
                &providers,
                &providers,
                &providers,
                &providers,
                &AllowReceiptIssuer,
            );
            let error = use_case
                .execute(
                    DiscoveryExecutionContext {
                        workspace_root: "/workspace".into(),
                        workspace_epoch: 7,
                    },
                    explore_request(),
                )
                .unwrap_err();
            let DiscoveryError::SourceReadiness(error) = error else {
                panic!("expected source-readiness error")
            };
            assert_eq!(error.reason_code(), reason);
            assert_eq!(error.role, role);
            assert!(!error.retryable());
        }
    }

    impl ProjectSourceResolverPort for EdtSourceResolver {
        fn resolve_all(
            &self,
            _context: &DiscoveryExecutionContext,
            requested_analysis: Option<&str>,
            requested_mutations: &[String],
        ) -> Result<ResolvedSourceSelection, DiscoveryError> {
            let mut source = fake_resolved(requested_analysis.unwrap_or("main"), false);
            source.source_format = SourceFormat::Edt;
            self.resolved_mutation_count
                .store(requested_mutations.len(), Ordering::SeqCst);
            ResolvedSourceSelection::new(
                source,
                requested_mutations
                    .iter()
                    .map(|name| fake_resolved(name, true))
                    .collect(),
            )
            .map_err(DiscoveryError::Operation)
        }
    }

    struct PanicEvidencePorts;

    #[derive(Default)]
    struct AnalysisOnlySnapshotPort {
        mutation_count: AtomicUsize,
    }

    impl SourceSnapshotPort for AnalysisOnlySnapshotPort {
        fn capture(
            &self,
            analysis: &ResolvedSourceSet,
            mutation_sources: &[ResolvedSourceSet],
            workspace_epoch: u64,
        ) -> Result<SourceSnapshot, SnapshotCaptureError> {
            self.mutation_count
                .store(mutation_sources.len(), Ordering::SeqCst);
            FakeSnapshotPort.capture(analysis, mutation_sources, workspace_epoch)
        }
    }

    macro_rules! panic_port {
        ($trait_name:ident, $method:ident) => {
            impl $trait_name for PanicEvidencePorts {
                fn $method(
                    &self,
                    _plan: &DiscoveryQueryPlan,
                    _context: &EvidenceExecutionContext<'_>,
                ) -> ProviderOutcome<EvidenceRecord> {
                    panic!("EDT readiness path must not invoke evidence providers")
                }
            }
        };
    }

    panic_port!(MetadataCatalogPort, metadata);
    panic_port!(CodeSearchPort, search);
    panic_port!(DefinitionPort, definitions);
    panic_port!(CallGraphPort, calls);
    panic_port!(FormInspectionPort, forms);
    panic_port!(SupportStatePort, support);

    #[test]
    fn edt_readiness_skips_providers_and_returns_typed_insufficient_report() {
        let providers = PanicEvidencePorts;
        let snapshots = AnalysisOnlySnapshotPort::default();
        let resolver = EdtSourceResolver::default();
        let use_case = DiscoverExtensionPointsUseCase::new(
            &resolver,
            &snapshots,
            &providers,
            &providers,
            &providers,
            &providers,
            &providers,
            &providers,
            &AllowReceiptIssuer,
        );

        let report = use_case
            .execute(
                DiscoveryExecutionContext {
                    workspace_root: "/workspace".into(),
                    workspace_epoch: 7,
                },
                mutation_method_proposal(),
            )
            .unwrap();

        assert_eq!(report.status, DiscoveryStatus::Insufficient);
        assert_eq!(resolver.resolved_mutation_count.load(Ordering::SeqCst), 1);
        assert_eq!(snapshots.mutation_count.load(Ordering::SeqCst), 0);
        assert!(report.related_artifacts.is_empty());
        assert!(report.flow_edges.is_empty());
        assert!(report.extension_point_candidates.is_empty());
        assert!(report.evidence.is_empty());
        assert_eq!(report.proposal_verdicts[0].verdict, Verdict::Unknown);
        assert_eq!(report.checks.len(), 1);
        let check = &report.checks[0];
        assert_eq!(check.code, "source_readiness");
        assert_eq!(check.provider, "ProjectSourceResolverPort");
        assert_eq!(check.state, CheckState::Skipped);
        assert_eq!(check.outcome, CheckOutcome::Inconclusive);
        assert_eq!(check.coverage, Coverage::Unknown);
        assert_eq!(check.severity, CheckSeverity::Blocking);
        assert_eq!(check.affects, ["proposal:method-hook"]);
        assert_eq!(check.reason_code, "unsupported_source_format");
        assert!(!check.retryable);
        assert!(check.details.is_empty());
        assert_eq!(
            report.receipt_eligibility.blockers,
            ["unsupported_source_format"]
        );
    }
}
