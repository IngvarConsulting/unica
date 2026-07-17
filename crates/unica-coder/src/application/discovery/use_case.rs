use super::contract::{DiscoverMode, DiscoverRequest, MutationIntent};
use super::determinism::{analysis_id, canonicalize_evidence, evidence_id};
use super::evidence_graph::EvidenceGraph;
use super::model::{
    Check, CheckOutcome, CheckSeverity, DiscoveryReport, DiscoverySource, DiscoveryStatus,
    EvidencePort, LinkedSourceSnapshot, ReceiptEligibility, SourceSnapshotRole, Verdict,
};
use super::ports::{
    CallGraphPort, CodeSearchPort, CollectedProviderOutcome, DefinitionPort, DiscoveryError,
    DiscoveryExecutionContext, DiscoveryQueryPlan, FormInspectionPort, MetadataCatalogPort,
    ProjectSourceResolverPort, ReceiptIssuanceRequest, ReceiptIssuerPort, SourceSnapshotPort,
    SupportStatePort,
};
use super::proposal_validator::{ProposalValidation, ProposalValidator};
use crate::domain::source_snapshot::{ResolvedSourceSet, SourceSnapshot};
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
        let analysis_source = self
            .source_resolver
            .resolve(&context, request.source_set.as_deref())?;
        analysis_source
            .validate()
            .map_err(DiscoveryError::Operation)?;
        let mutation_sources = self.resolve_mutation_sources(&context, &request)?;
        let snapshot = self.snapshot_port.capture(
            &analysis_source,
            &mutation_sources,
            context.workspace_epoch,
        )?;
        snapshot.validate().map_err(DiscoveryError::Operation)?;

        let plan = DiscoveryQueryPlan::normalized(&request);
        let providers = vec![
            self.metadata_catalog
                .metadata(&plan, &context)
                .collect(EvidencePort::MetadataCatalog)?,
            self.code_search
                .search(&plan, &context)
                .collect(EvidencePort::CodeSearch)?,
            self.definitions
                .definitions(&plan, &context)
                .collect(EvidencePort::Definition)?,
            self.call_graph
                .calls(&plan, &context)
                .collect(EvidencePort::CallGraph)?,
            self.form_inspection
                .forms(&plan, &context)
                .collect(EvidencePort::FormInspection)?,
            self.support_state
                .support(&plan, &context)
                .collect(EvidencePort::SupportState)?,
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

    fn resolve_mutation_sources(
        &self,
        context: &DiscoveryExecutionContext,
        request: &DiscoverRequest,
    ) -> Result<Vec<ResolvedSourceSet>, DiscoveryError> {
        let names = request
            .proposals
            .iter()
            .filter_map(|proposal| proposal.mutation_intent.as_ref())
            .map(|intent| match intent {
                MutationIntent::CfePatchMethod {
                    destination_source_set,
                    ..
                } => destination_source_set.clone(),
            })
            .collect::<BTreeSet<_>>();
        names
            .iter()
            .map(|name| self.source_resolver.resolve(context, Some(name)))
            .collect()
    }

    fn receipt_eligibility(
        &self,
        request: &DiscoverRequest,
        snapshot: &SourceSnapshot,
        validation: &ProposalValidation,
        checks: &[Check],
    ) -> Result<ReceiptEligibility, DiscoveryError> {
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
    if !conclusive {
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
        BindingDetails, Coverage, DiscoveryStatus, EvidencePort, EvidenceProvider, EvidenceRecord,
        FlowKind, Freshness, ProviderFact, ReceiptEligibility, SourceLocation, SupportState,
        Verdict,
    };
    use crate::application::discovery::ports::*;
    use crate::domain::project_sources::{SourceFormat, SourceSetKind};
    use crate::domain::source_snapshot::{ResolvedSourceSet, SourceSetSnapshot, SourceSnapshot};
    use serde_json::json;

    const FINGERPRINT: &str =
        "sha256:1111111111111111111111111111111111111111111111111111111111111111";
    const COMPOSITE: &str =
        "sha256:2222222222222222222222222222222222222222222222222222222222222222";

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
        EvidenceRecord::from_fact(
            fact,
            Some(SourceLocation::new("src/Flow.bsl", Some(1), Some(1)).unwrap()),
            EvidenceProvider::new(port, &format!("fake-{}", port.wire_name()), "1").unwrap(),
            Coverage::Complete,
            Freshness::new("main", FINGERPRINT, 7).unwrap(),
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
                    _context: &DiscoveryExecutionContext,
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
        fn resolve(
            &self,
            _context: &DiscoveryExecutionContext,
            _requested_source_set: Option<&str>,
        ) -> Result<ResolvedSourceSet, DiscoveryError> {
            Ok(ResolvedSourceSet {
                name: "main".into(),
                kind: SourceSetKind::Configuration,
                relative_root: "src".into(),
                source_format: SourceFormat::PlatformXml,
            })
        }
    }

    struct FakeSnapshotPort;

    impl SourceSnapshotPort for FakeSnapshotPort {
        fn capture(
            &self,
            analysis: &ResolvedSourceSet,
            _mutation_sources: &[ResolvedSourceSet],
            workspace_epoch: u64,
        ) -> Result<SourceSnapshot, DiscoveryError> {
            SourceSnapshot::new(
                SourceSetSnapshot {
                    source_set: analysis.clone(),
                    source_fingerprint: FINGERPRINT.into(),
                },
                Vec::new(),
                COMPOSITE.into(),
                workspace_epoch,
            )
            .map_err(DiscoveryError::Operation)
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
            let use_case = DiscoverExtensionPointsUseCase::new(
                &FakeSourceResolver,
                &FakeSnapshotPort,
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
}
