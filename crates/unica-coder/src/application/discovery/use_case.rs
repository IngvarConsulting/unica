use crate::application::discovery::contract::DiscoverRequest;
use crate::application::discovery::ports::DiscoveryPorts;
use crate::domain::cancellation::CancellationToken;
use crate::domain::discovery::{
    AnalysisSnapshot, AnalyzedFile, ArtifactId, ArtifactKind, BslFact, ConceptProvenance,
    DefinitionFact, DiscoveryConcept, DiscoveryEnvironment, DiscoveryError, DiscoveryQuery,
    DiscoveryQueryLimits, DiscoveryReport, DiscoverySource, DiscoveryStatus, DiscoveryWarning,
    Evidence, EvidenceId, EvidenceKind, EvidenceRelation, ExtensionPointCandidate, FactBatch,
    FormBinding, FormFact, LocatedFact, MetadataFact, MissingCheck, MissingCheckMateriality,
    PortableRelativePath, ProviderCoverage, ProviderDiagnostic, ProviderKind, ProviderOutcome,
    ProviderOutcomeKind, ProviderReport, RelatedArtifact, RuntimeFlowEdge, RuntimeFlowFact,
    SnapshotFingerprint, SourceInventory, StructuralEdge, StructuralRelationKind, SupportFact,
    SupportStateKind,
};
use std::collections::{BTreeMap, BTreeSet};

pub(crate) struct DiscoverExtensionPointsUseCase<'a> {
    ports: DiscoveryPorts<'a>,
}

impl<'a> DiscoverExtensionPointsUseCase<'a> {
    pub(crate) fn new(ports: DiscoveryPorts<'a>) -> Self {
        Self { ports }
    }

    pub(crate) fn execute(
        &self,
        request: &DiscoverRequest,
        environment: &DiscoveryEnvironment,
    ) -> Result<DiscoveryReport, DiscoveryError> {
        self.execute_inner(request, environment, None)
    }

    pub(crate) fn execute_cancellable(
        &self,
        request: &DiscoverRequest,
        environment: &DiscoveryEnvironment,
        cancellation: &CancellationToken,
    ) -> Result<DiscoveryReport, DiscoveryError> {
        self.execute_inner(request, environment, Some(cancellation))
    }

    fn execute_inner(
        &self,
        request: &DiscoverRequest,
        environment: &DiscoveryEnvironment,
        cancellation: Option<&CancellationToken>,
    ) -> Result<DiscoveryReport, DiscoveryError> {
        if environment.source_root().as_os_str().is_empty() {
            return Err(DiscoveryError::EmptySourceRoot);
        }

        let concepts = derive_concepts(request);
        let limits = request.limits();
        let mut query = DiscoveryQuery::new(
            request.task(),
            &concepts,
            request.search_terms(),
            request.objects(),
            DiscoveryQueryLimits {
                max_files: limits.max_files().get(),
                max_bytes: limits.max_bytes().get(),
                max_evidence: limits.max_evidence().get(),
                max_candidates: limits.max_candidates().get(),
                max_graph_depth: limits.max_graph_depth().get(),
            },
        );
        if let Some(cancellation) = cancellation {
            query = query.with_cancellation(cancellation);
        }
        let matcher = FactMatcher::new(&query);
        let mut accumulator = ReportAccumulator::new(query.limits().max_evidence as usize);

        ensure_discovery_active(&query)?;
        let inventory_outcome = self.ports.source_inventory.inventory(&query);
        ensure_discovery_active(&query)?;
        let mut inventory =
            EvaluatedOutcome::from_outcome(ProviderKind::SourceInventory, inventory_outcome);
        if let Some(files) = inventory.data.as_mut() {
            if let Err(diagnostic) = normalize_inventory(files, inventory.kind, query.limits()) {
                inventory.invalidate(diagnostic);
            }
        }
        if let Some(files) = inventory.data.as_ref() {
            accumulator.seed_inventory_identities(files);
        }
        accumulator.record_outcome(&inventory);

        match inventory.data.as_ref() {
            Some(files) => {
                ensure_discovery_active(&query)?;
                let metadata = self.ports.metadata_catalog.metadata(&query, files);
                ensure_discovery_active(&query)?;
                handle_batch(
                    ProviderKind::MetadataCatalog,
                    metadata,
                    &mut accumulator,
                    query.limits(),
                    Some(files),
                    |batch| metadata_contribution(batch, &matcher),
                );
                ensure_discovery_active(&query)?;
                let forms = self.ports.managed_forms.forms(&query, files);
                ensure_discovery_active(&query)?;
                handle_batch(
                    ProviderKind::ManagedForms,
                    forms,
                    &mut accumulator,
                    query.limits(),
                    Some(files),
                    |batch| form_contribution(batch, &matcher),
                );
                ensure_discovery_active(&query)?;
                let lexical = self.ports.bsl_search.search(&query, files);
                ensure_discovery_active(&query)?;
                handle_batch(
                    ProviderKind::BslSearch,
                    lexical,
                    &mut accumulator,
                    query.limits(),
                    Some(files),
                    lexical_contribution,
                );
                ensure_discovery_active(&query)?;
                let support = self.ports.support_state.support(&query, files);
                ensure_discovery_active(&query)?;
                handle_batch(
                    ProviderKind::SupportState,
                    support,
                    &mut accumulator,
                    query.limits(),
                    Some(files),
                    support_contribution,
                );
            }
            None => {
                for provider in [
                    ProviderKind::MetadataCatalog,
                    ProviderKind::ManagedForms,
                    ProviderKind::BslSearch,
                    ProviderKind::SupportState,
                ] {
                    accumulator.record_outcome(
                        &EvaluatedOutcome::<FactBatch<MetadataFact>>::absent(
                            provider,
                            ProviderOutcomeKind::Unavailable,
                            ProviderDiagnostic::material(
                                "source_inventory_unavailable",
                                "provider could not run because source inventory is unavailable",
                            ),
                        ),
                    );
                }
            }
        }

        ensure_discovery_active(&query)?;
        let definitions = self.ports.definitions.definitions(&query);
        ensure_discovery_active(&query)?;
        handle_batch(
            ProviderKind::Definitions,
            definitions,
            &mut accumulator,
            query.limits(),
            None,
            definition_contribution,
        );
        ensure_discovery_active(&query)?;
        let runtime_flow = self.ports.runtime_flow.runtime_flow(&query);
        ensure_discovery_active(&query)?;
        handle_batch(
            ProviderKind::RuntimeFlow,
            runtime_flow,
            &mut accumulator,
            query.limits(),
            None,
            |batch| runtime_flow_contribution(batch, &matcher),
        );

        ensure_discovery_active(&query)?;
        let query_limits = query.limits();
        Ok(accumulator.finish(concepts, environment, query_limits))
    }
}

fn ensure_discovery_active(query: &DiscoveryQuery<'_>) -> Result<(), DiscoveryError> {
    if query.is_cancelled() {
        Err(DiscoveryError::Cancelled)
    } else {
        Ok(())
    }
}

trait ProviderData {
    fn coverage(&self) -> ProviderCoverage;
}

trait ProviderFactContract {
    fn validate_contract(records: &[Self]) -> Result<(), ProviderDiagnostic>
    where
        Self: Sized;
}

macro_rules! provider_facts_without_additional_contract {
    ($($fact:ty),+ $(,)?) => {
        $(
            impl ProviderFactContract for $fact {
                fn validate_contract(_records: &[Self]) -> Result<(), ProviderDiagnostic> {
                    Ok(())
                }
            }
        )+
    };
}

provider_facts_without_additional_contract!(FormFact, BslFact, DefinitionFact, RuntimeFlowFact,);

impl ProviderFactContract for MetadataFact {
    fn validate_contract(records: &[Self]) -> Result<(), ProviderDiagnostic> {
        let mut kinds = BTreeMap::new();
        for fact in records {
            validate_artifact_kind(&mut kinds, &fact.artifact, fact.artifact_kind)?;
            match (&fact.container, fact.container_kind) {
                (Some(container), Some(container_kind)) => {
                    validate_artifact_kind(&mut kinds, container, container_kind)?;
                }
                (None, None) => {}
                _ => {
                    return Err(provider_contract_diagnostic(
                        "metadata_container_kind_missing",
                        "metadata container identity and kind must be supplied together",
                    ));
                }
            }
        }
        Ok(())
    }
}

fn validate_artifact_kind(
    kinds: &mut BTreeMap<ArtifactId, ArtifactKind>,
    artifact: &ArtifactId,
    kind: ArtifactKind,
) -> Result<(), ProviderDiagnostic> {
    if kinds
        .insert(artifact.clone(), kind)
        .is_some_and(|previous| previous != kind)
    {
        return Err(provider_contract_diagnostic(
            "artifact_kind_conflict",
            "provider returned conflicting kinds for one canonical artifact",
        ));
    }
    Ok(())
}

impl ProviderFactContract for SupportFact {
    fn validate_contract(records: &[Self]) -> Result<(), ProviderDiagnostic> {
        let mut states = BTreeMap::new();
        let mut kinds = BTreeMap::new();
        for fact in records {
            validate_artifact_kind(&mut kinds, &fact.artifact, fact.artifact_kind)?;
            if let Some(previous) = states.insert(&fact.artifact, fact.state) {
                if previous != fact.state {
                    return Err(provider_contract_diagnostic(
                        "support_state_conflict",
                        "support provider returned conflicting states for one artifact",
                    ));
                }
            }
        }
        Ok(())
    }
}

impl ProviderData for SourceInventory {
    fn coverage(&self) -> ProviderCoverage {
        self.coverage
    }
}

impl<T> ProviderData for FactBatch<T> {
    fn coverage(&self) -> ProviderCoverage {
        self.coverage
    }
}

struct EvaluatedOutcome<T> {
    provider: ProviderKind,
    kind: ProviderOutcomeKind,
    data: Option<T>,
    coverage: ProviderCoverage,
    diagnostic: Option<ProviderDiagnostic>,
}

impl<T: ProviderData> EvaluatedOutcome<T> {
    fn from_outcome(provider: ProviderKind, outcome: ProviderOutcome<T>) -> Self {
        match outcome {
            ProviderOutcome::Complete(data) => Self {
                provider,
                kind: ProviderOutcomeKind::Complete,
                coverage: data.coverage(),
                data: Some(data),
                diagnostic: None,
            },
            ProviderOutcome::Bounded { data, diagnostic } => Self {
                provider,
                kind: ProviderOutcomeKind::Bounded,
                coverage: data.coverage(),
                data: Some(data),
                diagnostic: Some(diagnostic),
            },
            ProviderOutcome::Unavailable(diagnostic) => {
                Self::absent(provider, ProviderOutcomeKind::Unavailable, diagnostic)
            }
            ProviderOutcome::Failed(diagnostic) => {
                Self::absent(provider, ProviderOutcomeKind::Failed, diagnostic)
            }
            ProviderOutcome::ContractViolation(diagnostic) => {
                Self::absent(provider, ProviderOutcomeKind::ContractViolation, diagnostic)
            }
        }
    }

    fn absent(
        provider: ProviderKind,
        kind: ProviderOutcomeKind,
        diagnostic: ProviderDiagnostic,
    ) -> Self {
        Self {
            provider,
            kind,
            data: None,
            coverage: ProviderCoverage::empty(),
            diagnostic: Some(diagnostic),
        }
    }

    fn invalidate(&mut self, diagnostic: ProviderDiagnostic) {
        self.kind = ProviderOutcomeKind::ContractViolation;
        self.data = None;
        self.diagnostic = Some(diagnostic);
    }
}

fn handle_batch<T, F>(
    provider: ProviderKind,
    outcome: ProviderOutcome<FactBatch<T>>,
    accumulator: &mut ReportAccumulator,
    limits: DiscoveryQueryLimits,
    inventory: Option<&SourceInventory>,
    build: F,
) where
    T: Clone + Ord + LocatedFact + ProviderFactContract,
    F: FnOnce(&FactBatch<T>) -> Result<ProviderContribution, ProviderDiagnostic>,
{
    let mut evaluated = EvaluatedOutcome::from_outcome(provider, outcome);
    let mut supplemental_diagnostic = None;
    let contribution = match evaluated.data.as_mut() {
        Some(batch) => {
            match normalize_batch(batch, evaluated.kind, limits, inventory).and_then(|()| {
                accumulator.validate_file_identities(&batch.analyzed_files)?;
                accumulator.validate_file_identities(&batch.contributors)?;
                let analyzed_files = batch.analyzed_files.clone();
                let remaining = accumulator.remaining_evidence();
                if batch.records.len() > remaining {
                    batch.records.truncate(remaining);
                    let diagnostic = ProviderDiagnostic::material(
                        "max_evidence_reached",
                        "discovery evidence was truncated by maxEvidence",
                    );
                    if outcome_is_complete(evaluated.kind) {
                        evaluated.kind = ProviderOutcomeKind::Bounded;
                        evaluated.diagnostic = Some(diagnostic);
                    } else {
                        supplemental_diagnostic = Some(diagnostic);
                    }
                }
                let mut contribution = build(batch)?;
                contribution.analyzed_files = analyzed_files;
                Ok(contribution)
            }) {
                Ok(contribution) => Some(contribution),
                Err(diagnostic) => {
                    evaluated.invalidate(diagnostic);
                    None
                }
            }
        }
        None => None,
    };

    if let Some(contribution) = contribution {
        if let Err(diagnostic) = accumulator.merge(contribution) {
            evaluated.invalidate(diagnostic);
        }
    }
    accumulator.record_outcome(&evaluated);
    if let Some(diagnostic) = supplemental_diagnostic {
        accumulator.record_diagnostic(provider, evaluated.kind, &diagnostic);
    }
}

fn normalize_inventory(
    inventory: &mut SourceInventory,
    outcome: ProviderOutcomeKind,
    limits: DiscoveryQueryLimits,
) -> Result<(), ProviderDiagnostic> {
    let file_count = checked_count(inventory.files.len(), "inventory_file_count_overflow")?;
    let byte_count = checked_bytes(
        inventory.files.iter().map(|file| file.bytes.len() as u64),
        "inventory_byte_count_overflow",
    )?;
    if file_count > limits.max_files || byte_count > limits.max_bytes {
        return Err(provider_contract_diagnostic(
            "inventory_limit_violation",
            "source inventory exceeded the validated discovery query limits",
        ));
    }
    inventory.files.sort_by(|left, right| {
        left.relative_path
            .cmp(&right.relative_path)
            .then_with(|| left.raw_hash.cmp(&right.raw_hash))
            .then_with(|| left.bytes.cmp(&right.bytes))
    });

    let mut normalized: Vec<crate::domain::discovery::SourceFile> =
        Vec::with_capacity(inventory.files.len());
    for file in inventory.files.drain(..) {
        if file.raw_hash != crate::domain::discovery::ContentHash::sha256(&file.bytes) {
            return Err(provider_contract_diagnostic(
                "inventory_hash_mismatch",
                "source inventory raw hash does not match the supplied bytes",
            ));
        }
        match normalized.last() {
            Some(previous) if previous.relative_path == file.relative_path => {
                if previous != &file {
                    return Err(provider_contract_diagnostic(
                        "inventory_path_conflict",
                        "source inventory returned conflicting records for one path",
                    ));
                }
                return Err(provider_contract_diagnostic(
                    "duplicate_inventory_path",
                    "source inventory returned a duplicate canonical path",
                ));
            }
            Some(_) | None => normalized.push(file),
        }
    }
    inventory.files = normalized;

    if inventory.coverage.files_analyzed != file_count
        || inventory.coverage.records != file_count
        || inventory.coverage.bytes_analyzed != byte_count
        || inventory.coverage.files_seen < file_count
    {
        return Err(provider_contract_diagnostic(
            "inventory_coverage_mismatch",
            "source inventory coverage does not match returned files, bytes, and records",
        ));
    }
    match outcome {
        ProviderOutcomeKind::Complete => {
            if inventory.coverage.files_seen != file_count {
                return Err(provider_contract_diagnostic(
                    "complete_inventory_coverage_mismatch",
                    "complete source inventory must account exactly for every returned file",
                ));
            }
        }
        ProviderOutcomeKind::Bounded => {
            let max_files_seen = limits.max_files.checked_add(1).unwrap_or(u32::MAX);
            if inventory.coverage.files_seen > max_files_seen {
                return Err(provider_contract_diagnostic(
                    "inventory_files_seen_limit_violation",
                    "bounded source inventory observed more than the triggering N+1 file",
                ));
            }
            let demonstrates_bound = inventory.coverage.files_seen > file_count
                || file_count == limits.max_files
                || byte_count == limits.max_bytes;
            if !demonstrates_bound {
                return Err(provider_contract_diagnostic(
                    "unsubstantiated_bounded_inventory",
                    "bounded source inventory coverage does not demonstrate a reached limit",
                ));
            }
        }
        ProviderOutcomeKind::Unavailable
        | ProviderOutcomeKind::Failed
        | ProviderOutcomeKind::ContractViolation => {
            return Err(provider_contract_diagnostic(
                "invalid_inventory_outcome_data",
                "non-data source inventory outcome unexpectedly carried data",
            ));
        }
    }
    Ok(())
}

fn normalize_batch<T>(
    batch: &mut FactBatch<T>,
    outcome: ProviderOutcomeKind,
    limits: DiscoveryQueryLimits,
    inventory: Option<&SourceInventory>,
) -> Result<(), ProviderDiagnostic>
where
    T: Ord + LocatedFact + ProviderFactContract,
{
    let record_count = checked_count(batch.records.len(), "provider_record_count_overflow")?;
    let contributor_count = checked_count(
        batch.contributors.len(),
        "provider_contributor_count_overflow",
    )?;
    let analyzed_count = checked_count(
        batch.analyzed_files.len(),
        "provider_analyzed_file_count_overflow",
    )?;
    let analyzed_bytes = checked_bytes(
        batch.analyzed_files.iter().map(|item| item.bytes),
        "provider_analyzed_byte_count_overflow",
    )?;
    let contributor_bytes = checked_bytes(
        batch.contributors.iter().map(|item| item.bytes),
        "provider_contributor_byte_count_overflow",
    )?;
    if record_count > u32::from(limits.max_evidence)
        || batch.coverage.files_seen > limits.max_files
        || batch.coverage.files_analyzed > limits.max_files
        || batch.coverage.bytes_analyzed > limits.max_bytes
        || analyzed_count > limits.max_files
        || analyzed_bytes > limits.max_bytes
        || contributor_count > limits.max_files
        || contributor_bytes > limits.max_bytes
    {
        return Err(provider_contract_diagnostic(
            "provider_limit_violation",
            "provider data exceeded the validated discovery query limits",
        ));
    }
    T::validate_contract(&batch.records)?;
    batch.records.sort();
    if batch.records.windows(2).any(|items| items[0] == items[1]) {
        return Err(provider_contract_diagnostic(
            "duplicate_provider_fact",
            "provider returned a duplicate typed fact",
        ));
    }
    normalize_analyzed_files(&mut batch.analyzed_files)?;
    batch.contributors.sort();

    let mut normalized: Vec<AnalyzedFile> = Vec::with_capacity(batch.contributors.len());
    for contributor in batch.contributors.drain(..) {
        match normalized.last() {
            Some(previous) if previous.relative_path == contributor.relative_path => {
                if previous != &contributor {
                    return Err(provider_contract_diagnostic(
                        "contributor_path_conflict",
                        "provider returned conflicting contributor hashes for one path",
                    ));
                }
                return Err(provider_contract_diagnostic(
                    "duplicate_contributor_path",
                    "provider returned a duplicate canonical contributor path",
                ));
            }
            Some(_) | None => normalized.push(contributor),
        }
    }

    let contributor_paths = normalized
        .iter()
        .map(|item| &item.relative_path)
        .collect::<BTreeSet<_>>();
    let fact_paths = batch
        .records
        .iter()
        .map(|fact| &fact.location().relative_path)
        .collect::<BTreeSet<_>>();
    if fact_paths != contributor_paths {
        return Err(provider_contract_diagnostic(
            "evidence_contributor_set_mismatch",
            "provider fact-location paths must exactly equal contributor paths",
        ));
    }
    batch.contributors = normalized;

    validate_contributors_are_analyzed(&batch.contributors, &batch.analyzed_files)?;

    if batch.coverage.records != record_count
        || batch.coverage.files_analyzed != analyzed_count
        || batch.coverage.bytes_analyzed != analyzed_bytes
        || batch.coverage.files_seen < batch.coverage.files_analyzed
    {
        return Err(provider_contract_diagnostic(
            "provider_coverage_mismatch",
            "provider coverage does not account for returned records and contributors",
        ));
    }
    if let Some(inventory) = inventory {
        validate_inventory_membership(&batch.analyzed_files, inventory)?;
        let inventory_count =
            checked_count(inventory.files.len(), "provider_inventory_count_overflow")?;
        if batch.coverage.files_seen > inventory_count {
            return Err(provider_contract_diagnostic(
                "provider_coverage_outside_inventory",
                "inventory-backed provider saw more eligible files than the supplied inventory contains",
            ));
        }
    }
    match outcome {
        ProviderOutcomeKind::Complete => {
            if batch.coverage.files_seen != batch.coverage.files_analyzed {
                return Err(provider_contract_diagnostic(
                    "complete_provider_coverage_mismatch",
                    "complete provider coverage must analyze every provider-eligible file seen",
                ));
            }
        }
        ProviderOutcomeKind::Bounded => {}
        ProviderOutcomeKind::Unavailable
        | ProviderOutcomeKind::Failed
        | ProviderOutcomeKind::ContractViolation => {
            return Err(provider_contract_diagnostic(
                "invalid_provider_outcome_data",
                "non-data provider outcome unexpectedly carried data",
            ));
        }
    }
    Ok(())
}

fn normalize_analyzed_files(
    analyzed_files: &mut Vec<AnalyzedFile>,
) -> Result<(), ProviderDiagnostic> {
    analyzed_files.sort();
    for files in analyzed_files.windows(2) {
        if files[0].relative_path == files[1].relative_path {
            let (code, message) = if files[0] == files[1] {
                (
                    "duplicate_analyzed_file_path",
                    "provider returned a duplicate canonical analyzed-file path",
                )
            } else {
                (
                    "analyzed_file_path_conflict",
                    "provider returned conflicting analyzed-file identities for one path",
                )
            };
            return Err(provider_contract_diagnostic(code, message));
        }
    }
    Ok(())
}

fn validate_contributors_are_analyzed(
    contributors: &[AnalyzedFile],
    analyzed_files: &[AnalyzedFile],
) -> Result<(), ProviderDiagnostic> {
    for contributor in contributors {
        let found = analyzed_files
            .binary_search_by(|file| file.relative_path.cmp(&contributor.relative_path));
        let Ok(index) = found else {
            return Err(provider_contract_diagnostic(
                "contributor_not_analyzed",
                "provider evidence contributor is absent from its analyzed-file set",
            ));
        };
        let Some(analyzed) = analyzed_files.get(index) else {
            return Err(provider_contract_diagnostic(
                "analyzed_file_index_contract_violation",
                "provider analyzed-file binary-search result was not addressable",
            ));
        };
        if analyzed != contributor {
            return Err(provider_contract_diagnostic(
                "contributor_analyzed_identity_mismatch",
                "provider evidence contributor differs from its analyzed-file identity",
            ));
        }
    }
    Ok(())
}

fn validate_inventory_membership(
    analyzed_files: &[AnalyzedFile],
    inventory: &SourceInventory,
) -> Result<(), ProviderDiagnostic> {
    for analyzed in analyzed_files {
        let found = inventory
            .files
            .binary_search_by(|file| file.relative_path.cmp(&analyzed.relative_path));
        let Ok(index) = found else {
            return Err(provider_contract_diagnostic(
                "analyzed_file_outside_inventory",
                "provider analyzed file is absent from the supplied source inventory",
            ));
        };
        let Some(file) = inventory.files.get(index) else {
            return Err(provider_contract_diagnostic(
                "inventory_index_contract_violation",
                "source inventory binary-search result was not addressable",
            ));
        };
        if file.analyzed_file() != *analyzed {
            return Err(provider_contract_diagnostic(
                "analyzed_file_inventory_identity_mismatch",
                "provider analyzed-file hash or byte count differs from source inventory identity",
            ));
        }
    }
    Ok(())
}

fn checked_count(count: usize, code: &str) -> Result<u32, ProviderDiagnostic> {
    u32::try_from(count).map_err(|_error| {
        provider_contract_diagnostic(
            code,
            "provider count cannot be represented in report coverage",
        )
    })
}

fn checked_bytes<I>(bytes: I, code: &str) -> Result<u64, ProviderDiagnostic>
where
    I: IntoIterator<Item = u64>,
{
    bytes.into_iter().try_fold(0_u64, |total, value| {
        total.checked_add(value).ok_or_else(|| {
            provider_contract_diagnostic(code, "provider byte count overflowed report coverage")
        })
    })
}

fn provider_contract_diagnostic(code: &str, message: &str) -> ProviderDiagnostic {
    ProviderDiagnostic::material(code, message)
}

#[derive(Default)]
struct ProviderContribution {
    analyzed_files: Vec<AnalyzedFile>,
    evidence: Vec<Evidence>,
    contributors: Vec<AnalyzedFile>,
    related: Vec<(ArtifactId, ArtifactKind, EvidenceId)>,
    structural_edges: Vec<(ArtifactId, ArtifactId, StructuralRelationKind, EvidenceId)>,
    runtime_flow_edges: Vec<(
        ArtifactId,
        ArtifactId,
        crate::domain::discovery::RuntimeFlowRelationKind,
        EvidenceId,
    )>,
    candidates: Vec<(ArtifactId, ArtifactKind, EvidenceId)>,
    support: Vec<(ArtifactId, SupportStateKind)>,
    warnings: Vec<DiscoveryWarning>,
}

fn metadata_contribution(
    batch: &FactBatch<MetadataFact>,
    matcher: &FactMatcher,
) -> Result<ProviderContribution, ProviderDiagnostic> {
    let mut contribution = ProviderContribution::default();
    let mut evidence_by_artifact = BTreeMap::new();
    for fact in &batch.records {
        let evidence = evidence_for(
            ProviderKind::MetadataCatalog,
            EvidenceKind::Metadata,
            &fact.artifact,
            EvidenceRelation::Structural(fact.relation),
            &fact.location,
            batch,
        )?;
        contribution.contributors.push(evidence.1.clone());
        contribution.related.push((
            fact.artifact.clone(),
            fact.artifact_kind,
            evidence.0.id.clone(),
        ));
        if let Some((container, container_kind)) = fact.container.as_ref().zip(fact.container_kind)
        {
            contribution
                .related
                .push((container.clone(), container_kind, evidence.0.id.clone()));
            contribution.structural_edges.push((
                container.clone(),
                fact.artifact.clone(),
                fact.relation,
                evidence.0.id.clone(),
            ));
        }
        if matcher.relevant(&fact.artifact, std::iter::empty::<&str>()) {
            contribution.candidates.push((
                fact.artifact.clone(),
                fact.artifact_kind,
                evidence.0.id.clone(),
            ));
        }
        evidence_by_artifact
            .entry(fact.artifact.clone())
            .or_insert_with(|| evidence.0.id.clone());
        contribution.evidence.push(evidence.0);
    }
    contribution.warnings = separate_series_warnings(batch, matcher, &evidence_by_artifact);
    Ok(contribution)
}

fn separate_series_warnings(
    batch: &FactBatch<MetadataFact>,
    matcher: &FactMatcher,
    evidence_by_artifact: &BTreeMap<ArtifactId, EvidenceId>,
) -> Vec<DiscoveryWarning> {
    let mut warnings = Vec::new();
    for series_attribute in batch.records.iter().filter(|fact| {
        fact.artifact_kind == ArtifactKind::Attribute
            && fact.relation == StructuralRelationKind::Contains
            && artifact_has_typed_name(&fact.artifact, "attribute", "серия")
    }) {
        let Some(goods_section_id) = series_attribute.container.as_ref() else {
            continue;
        };
        let Some(goods_section) = batch.records.iter().find(|fact| {
            fact.artifact == *goods_section_id
                && fact.artifact_kind == ArtifactKind::TabularSection
                && fact.relation == StructuralRelationKind::Contains
                && artifact_has_typed_name(&fact.artifact, "tabularsection", "товары")
        }) else {
            continue;
        };
        let Some(document) = goods_section.container.as_ref() else {
            continue;
        };
        if document.as_str().split('.').next() != Some("document") {
            continue;
        }
        if !matcher.relevant(document, std::iter::empty::<&str>())
            && !matcher.relevant(&series_attribute.artifact, std::iter::empty::<&str>())
        {
            continue;
        }
        for separate_section in batch.records.iter().filter(|fact| {
            fact.artifact_kind == ArtifactKind::TabularSection
                && fact.relation == StructuralRelationKind::Contains
                && fact.container.as_ref() == Some(document)
                && fact.artifact != goods_section.artifact
                && artifact_leaves_match(&series_attribute.artifact, &fact.artifact)
        }) {
            let Some(series_evidence) = evidence_by_artifact.get(&series_attribute.artifact) else {
                continue;
            };
            let Some(section_evidence) = evidence_by_artifact.get(&separate_section.artifact)
            else {
                continue;
            };
            let mut evidence_ids = vec![series_evidence.clone(), section_evidence.clone()];
            evidence_ids.sort();
            warnings.push(DiscoveryWarning {
                code: "separate_series_section".to_string(),
                message: "A point limited to Товары.Серия lacks coverage: the same relevant document contains a distinct series-related tabular section."
                    .to_string(),
                blocking: true,
                evidence_ids,
            });
        }
    }
    warnings.sort();
    warnings.dedup();
    warnings
}

fn artifact_leaves_match(left: &ArtifactId, right: &ArtifactId) -> bool {
    match (
        left.as_str().rsplit('.').next(),
        right.as_str().rsplit('.').next(),
    ) {
        (Some(left), Some(right)) => normalized_prefix_matches(left, right),
        (Some(_), None) | (None, Some(_)) | (None, None) => false,
    }
}

fn artifact_has_typed_name(artifact: &ArtifactId, object_type: &str, name: &str) -> bool {
    let mut segments = artifact.as_str().rsplit('.');
    matches!(
        (segments.next(), segments.next()),
        (Some(actual_name), Some(actual_type))
            if actual_name == name && actual_type == object_type
    )
}

fn form_contribution(
    batch: &FactBatch<FormFact>,
    matcher: &FactMatcher,
) -> Result<ProviderContribution, ProviderDiagnostic> {
    let mut contribution = ProviderContribution::default();
    for fact in &batch.records {
        let (target, target_kind, relation, supplemental, runtime_relation) = match &fact.binding {
            FormBinding::Data {
                target,
                target_kind,
                data_path,
            } => (
                target,
                *target_kind,
                EvidenceRelation::Structural(StructuralRelationKind::DataBinding),
                vec![data_path.as_str()],
                None,
            ),
            FormBinding::Command {
                command,
                handler,
                target,
                target_kind,
            } => (
                target,
                *target_kind,
                EvidenceRelation::RuntimeFlow(
                    crate::domain::discovery::RuntimeFlowRelationKind::Action,
                ),
                vec![command.as_str(), handler.as_str()],
                Some(crate::domain::discovery::RuntimeFlowRelationKind::Action),
            ),
            FormBinding::Event {
                event,
                handler,
                target,
                target_kind,
            } => (
                target,
                *target_kind,
                EvidenceRelation::RuntimeFlow(
                    crate::domain::discovery::RuntimeFlowRelationKind::Callback,
                ),
                vec![event.as_str(), handler.as_str()],
                Some(crate::domain::discovery::RuntimeFlowRelationKind::Callback),
            ),
        };
        let evidence = evidence_for(
            ProviderKind::ManagedForms,
            EvidenceKind::FormBinding,
            &fact.form,
            relation,
            &fact.location,
            batch,
        )?;
        contribution.contributors.push(evidence.1.clone());
        contribution
            .related
            .push((fact.form.clone(), ArtifactKind::Form, evidence.0.id.clone()));
        contribution
            .related
            .push((target.clone(), target_kind, evidence.0.id.clone()));
        match runtime_relation {
            Some(relation) => contribution.runtime_flow_edges.push((
                fact.form.clone(),
                target.clone(),
                relation,
                evidence.0.id.clone(),
            )),
            None => contribution.structural_edges.push((
                fact.form.clone(),
                target.clone(),
                StructuralRelationKind::DataBinding,
                evidence.0.id.clone(),
            )),
        }
        if matcher.relevant(&fact.form, supplemental.iter().copied())
            || matcher.relevant(target, supplemental.iter().copied())
        {
            contribution.candidates.push((
                fact.form.clone(),
                ArtifactKind::Form,
                evidence.0.id.clone(),
            ));
        }
        contribution.evidence.push(evidence.0);
    }
    Ok(contribution)
}

fn lexical_contribution(
    batch: &FactBatch<BslFact>,
) -> Result<ProviderContribution, ProviderDiagnostic> {
    let mut contribution = ProviderContribution::default();
    for fact in &batch.records {
        let evidence = evidence_for(
            ProviderKind::BslSearch,
            EvidenceKind::Lexical,
            &fact.artifact,
            EvidenceRelation::None,
            &fact.location,
            batch,
        )?;
        contribution.contributors.push(evidence.1.clone());
        contribution.related.push((
            fact.artifact.clone(),
            fact.artifact_kind,
            evidence.0.id.clone(),
        ));
        contribution.evidence.push(evidence.0);
    }
    Ok(contribution)
}

fn definition_contribution(
    batch: &FactBatch<DefinitionFact>,
) -> Result<ProviderContribution, ProviderDiagnostic> {
    let mut contribution = ProviderContribution::default();
    for fact in &batch.records {
        let evidence = evidence_for(
            ProviderKind::Definitions,
            EvidenceKind::Definition,
            &fact.definition,
            EvidenceRelation::Structural(StructuralRelationKind::Defines),
            &fact.location,
            batch,
        )?;
        contribution.contributors.push(evidence.1.clone());
        contribution.related.push((
            fact.owner.clone(),
            ArtifactKind::Module,
            evidence.0.id.clone(),
        ));
        contribution.related.push((
            fact.definition.clone(),
            ArtifactKind::Method,
            evidence.0.id.clone(),
        ));
        contribution.structural_edges.push((
            fact.owner.clone(),
            fact.definition.clone(),
            StructuralRelationKind::Defines,
            evidence.0.id.clone(),
        ));
        contribution.evidence.push(evidence.0);
    }
    Ok(contribution)
}

fn runtime_flow_contribution(
    batch: &FactBatch<RuntimeFlowFact>,
    matcher: &FactMatcher,
) -> Result<ProviderContribution, ProviderDiagnostic> {
    let mut contribution = ProviderContribution::default();
    for fact in &batch.records {
        let evidence = evidence_for(
            ProviderKind::RuntimeFlow,
            EvidenceKind::RuntimeFlow,
            &fact.target,
            EvidenceRelation::RuntimeFlow(fact.relation),
            &fact.location,
            batch,
        )?;
        contribution.contributors.push(evidence.1.clone());
        contribution
            .related
            .push((fact.source.clone(), fact.source_kind, evidence.0.id.clone()));
        contribution
            .related
            .push((fact.target.clone(), fact.target_kind, evidence.0.id.clone()));
        contribution.runtime_flow_edges.push((
            fact.source.clone(),
            fact.target.clone(),
            fact.relation,
            evidence.0.id.clone(),
        ));
        if matcher.relevant(&fact.source, std::iter::empty::<&str>())
            || matcher.relevant(&fact.target, std::iter::empty::<&str>())
        {
            contribution.candidates.push((
                fact.target.clone(),
                fact.target_kind,
                evidence.0.id.clone(),
            ));
        }
        contribution.evidence.push(evidence.0);
    }
    Ok(contribution)
}

fn support_contribution(
    batch: &FactBatch<SupportFact>,
) -> Result<ProviderContribution, ProviderDiagnostic> {
    let mut contribution = ProviderContribution::default();
    for fact in &batch.records {
        let evidence = evidence_for(
            ProviderKind::SupportState,
            EvidenceKind::SupportState,
            &fact.artifact,
            EvidenceRelation::None,
            &fact.location,
            batch,
        )?;
        contribution.contributors.push(evidence.1.clone());
        contribution.related.push((
            fact.artifact.clone(),
            fact.artifact_kind,
            evidence.0.id.clone(),
        ));
        contribution
            .support
            .push((fact.artifact.clone(), fact.state));
        contribution.evidence.push(evidence.0);
    }
    Ok(contribution)
}

fn evidence_for<T>(
    provider: ProviderKind,
    kind: EvidenceKind,
    target: &ArtifactId,
    relation: EvidenceRelation,
    location: &crate::domain::discovery::EvidenceLocation,
    batch: &FactBatch<T>,
) -> Result<(Evidence, AnalyzedFile), ProviderDiagnostic> {
    let contributor_index = batch
        .contributors
        .binary_search_by(|item| item.relative_path.cmp(&location.relative_path))
        .map_err(|_index| {
            provider_contract_diagnostic(
                "missing_evidence_contributor",
                "provider fact location has no exact analyzed-file contributor",
            )
        })?;
    let contributor = batch.contributors.get(contributor_index).ok_or_else(|| {
        provider_contract_diagnostic(
            "evidence_contributor_index_invalid",
            "provider contributor index was not addressable",
        )
    })?;
    let id = EvidenceId::from_fact(
        provider,
        kind,
        target,
        &relation,
        location,
        &contributor.raw_hash,
    );
    Ok((
        Evidence {
            id,
            provider,
            kind,
            target: target.clone(),
            relation,
            location: location.clone(),
            raw_content_hash: contributor.raw_hash.clone(),
        },
        contributor.clone(),
    ))
}

#[derive(Default)]
struct ReportAccumulator {
    max_evidence: usize,
    provider_outcomes: Vec<ProviderReport>,
    warnings: Vec<DiscoveryWarning>,
    missing_checks: Vec<MissingCheck>,
    evidence: BTreeMap<EvidenceId, Evidence>,
    known_files: BTreeMap<PortableRelativePath, AnalyzedFile>,
    contributors: BTreeMap<PortableRelativePath, AnalyzedFile>,
    related: BTreeMap<ArtifactId, (ArtifactKind, BTreeSet<EvidenceId>)>,
    structural_edges:
        BTreeMap<(ArtifactId, ArtifactId, StructuralRelationKind), BTreeSet<EvidenceId>>,
    runtime_flow_edges: BTreeMap<
        (
            ArtifactId,
            ArtifactId,
            crate::domain::discovery::RuntimeFlowRelationKind,
        ),
        BTreeSet<EvidenceId>,
    >,
    candidates: BTreeMap<(ArtifactId, ArtifactKind), BTreeSet<EvidenceId>>,
    support: BTreeMap<ArtifactId, SupportStateKind>,
}

impl ReportAccumulator {
    fn new(max_evidence: usize) -> Self {
        Self {
            max_evidence,
            ..Self::default()
        }
    }

    fn remaining_evidence(&self) -> usize {
        self.max_evidence.saturating_sub(self.evidence.len())
    }

    /// Seeds coherence checks only; snapshot contributors remain evidence-only.
    fn seed_inventory_identities(&mut self, inventory: &SourceInventory) {
        for file in &inventory.files {
            let analyzed = file.analyzed_file();
            self.known_files
                .insert(analyzed.relative_path.clone(), analyzed);
        }
    }

    fn validate_file_identities(&self, files: &[AnalyzedFile]) -> Result<(), ProviderDiagnostic> {
        for file in files {
            if self
                .known_files
                .get(&file.relative_path)
                .is_some_and(|known| known != file)
            {
                return Err(provider_contract_diagnostic(
                    "cross_provider_file_identity_conflict",
                    "provider file identity conflicts with an already accepted provider",
                ));
            }
        }
        Ok(())
    }

    fn record_outcome<T>(&mut self, evaluated: &EvaluatedOutcome<T>) {
        self.provider_outcomes.push(ProviderReport {
            provider: evaluated.provider,
            outcome: evaluated.kind,
            coverage: evaluated.coverage,
            diagnostic: evaluated.diagnostic.clone(),
        });
        if let Some(diagnostic) = &evaluated.diagnostic {
            self.record_diagnostic(evaluated.provider, evaluated.kind, diagnostic);
        }
    }

    fn record_diagnostic(
        &mut self,
        provider: ProviderKind,
        outcome: ProviderOutcomeKind,
        diagnostic: &ProviderDiagnostic,
    ) {
        self.missing_checks.push(MissingCheck {
            provider,
            materiality: diagnostic.materiality,
            code: diagnostic.code.clone(),
            message: diagnostic.message.clone(),
        });
        self.warnings.push(DiscoveryWarning {
            code: diagnostic.code.clone(),
            message: diagnostic.message.clone(),
            blocking: outcome_is_contract_violation(outcome)
                || diagnostic.materiality == MissingCheckMateriality::Material,
            evidence_ids: Vec::new(),
        });
    }

    fn merge(&mut self, contribution: ProviderContribution) -> Result<(), ProviderDiagnostic> {
        self.validate_file_identities(&contribution.analyzed_files)?;
        self.validate_file_identities(&contribution.contributors)?;
        let mut contribution_kinds = BTreeMap::new();
        for (artifact, kind, _) in &contribution.related {
            validate_artifact_kind(&mut contribution_kinds, artifact, *kind)?;
            if self
                .related
                .get(artifact)
                .is_some_and(|(previous_kind, _)| previous_kind != kind)
            {
                return Err(provider_contract_diagnostic(
                    "artifact_kind_conflict",
                    "provider returned a kind that conflicts with an already accepted artifact",
                ));
            }
        }
        let mut contribution_support = BTreeMap::new();
        for (artifact, state) in &contribution.support {
            if let Some(previous) = contribution_support.insert(artifact, state) {
                if previous != state {
                    return Err(provider_contract_diagnostic(
                        "support_state_conflict",
                        "support provider returned conflicting states for one artifact",
                    ));
                }
            }
            if let Some(previous) = self.support.get(artifact) {
                if previous != state {
                    return Err(provider_contract_diagnostic(
                        "support_state_conflict",
                        "support provider returned conflicting states for one artifact",
                    ));
                }
            }
        }

        for analyzed in contribution.analyzed_files {
            self.known_files
                .entry(analyzed.relative_path.clone())
                .or_insert(analyzed);
        }
        for contributor in contribution.contributors {
            self.contributors
                .entry(contributor.relative_path.clone())
                .or_insert(contributor);
        }
        for evidence in contribution.evidence {
            self.evidence.entry(evidence.id.clone()).or_insert(evidence);
        }
        for (artifact, kind, evidence_id) in contribution.related {
            let (_, evidence_ids) = self
                .related
                .entry(artifact)
                .or_insert_with(|| (kind, BTreeSet::new()));
            evidence_ids.insert(evidence_id);
        }
        for (source, target, relation, evidence_id) in contribution.structural_edges {
            self.structural_edges
                .entry((source, target, relation))
                .or_default()
                .insert(evidence_id);
        }
        for (source, target, relation, evidence_id) in contribution.runtime_flow_edges {
            self.runtime_flow_edges
                .entry((source, target, relation))
                .or_default()
                .insert(evidence_id);
        }
        for (target, kind, evidence_id) in contribution.candidates {
            self.candidates
                .entry((target, kind))
                .or_default()
                .insert(evidence_id);
        }
        for (artifact, state) in contribution.support {
            self.support.entry(artifact).or_insert(state);
        }
        self.warnings.extend(contribution.warnings);
        Ok(())
    }

    fn finish(
        mut self,
        concepts: Vec<DiscoveryConcept>,
        environment: &DiscoveryEnvironment,
        limits: DiscoveryQueryLimits,
    ) -> DiscoveryReport {
        self.apply_candidate_limit(limits.max_candidates as usize);
        self.provider_outcomes.sort();
        self.warnings.sort();
        self.warnings.dedup();
        self.missing_checks.sort();
        self.missing_checks.dedup();

        let contributors = self.contributors.into_values().collect::<Vec<_>>();
        let fingerprint =
            SnapshotFingerprint::from_manifest(environment.mapping_fingerprint(), &contributors);
        let status = if self
            .provider_outcomes
            .iter()
            .all(|report| outcome_is_complete(report.outcome))
        {
            DiscoveryStatus::Complete
        } else {
            DiscoveryStatus::Partial
        };
        let related_artifacts = self
            .related
            .into_iter()
            .map(|(artifact, (kind, evidence_ids))| RelatedArtifact {
                artifact,
                kind,
                evidence_ids: evidence_ids.into_iter().collect(),
            })
            .collect();
        let structural_edges = self
            .structural_edges
            .into_iter()
            .map(
                |((source, target, relation), evidence_ids)| StructuralEdge {
                    source,
                    target,
                    relation,
                    evidence_ids: evidence_ids.into_iter().collect(),
                },
            )
            .collect();
        let runtime_flow_edges = self
            .runtime_flow_edges
            .into_iter()
            .map(
                |((source, target, relation), evidence_ids)| RuntimeFlowEdge {
                    source,
                    target,
                    relation,
                    evidence_ids: evidence_ids.into_iter().collect(),
                },
            )
            .collect();
        let candidates = self
            .candidates
            .into_iter()
            .map(|((target, kind), evidence_ids)| {
                let support_state = self.support.get(&target).copied();
                ExtensionPointCandidate {
                    target,
                    kind,
                    evidence_ids: evidence_ids.into_iter().collect(),
                    support_state,
                }
            })
            .collect();

        DiscoveryReport {
            schema_version: 1,
            status,
            source: DiscoverySource {
                root: environment.source_root().to_path_buf(),
                mapping_fingerprint: environment.mapping_fingerprint().clone(),
            },
            analysis_snapshot: AnalysisSnapshot {
                mapping_fingerprint: environment.mapping_fingerprint().clone(),
                fingerprint,
                contributors,
            },
            concepts,
            provider_outcomes: self.provider_outcomes,
            related_artifacts,
            structural_edges,
            runtime_flow_edges,
            candidates,
            warnings: self.warnings,
            missing_checks: self.missing_checks,
            evidence: self.evidence.into_values().collect(),
        }
    }

    fn apply_candidate_limit(&mut self, maximum: usize) {
        if self.candidates.len() <= maximum {
            return;
        }
        let retained = self
            .candidates
            .keys()
            .take(maximum)
            .cloned()
            .collect::<BTreeSet<_>>();
        let affected = self
            .candidates
            .iter()
            .filter(|(key, _evidence_ids)| !retained.contains(*key))
            .flat_map(|(_key, evidence_ids)| evidence_ids)
            .filter_map(|id| self.evidence.get(id))
            .map(|evidence| evidence.provider)
            .collect::<BTreeSet<_>>();
        self.candidates
            .retain(|key, _evidence_ids| retained.contains(key));
        for provider in affected {
            self.record_limit(
                provider,
                "max_candidates_reached",
                "discovery candidates were truncated by maxCandidates",
            );
        }
    }

    fn record_limit(&mut self, provider: ProviderKind, code: &str, message: &str) {
        if let Some(report) = self
            .provider_outcomes
            .iter_mut()
            .find(|report| report.provider == provider)
        {
            if outcome_is_complete(report.outcome) {
                let diagnostic = ProviderDiagnostic::material(code, message);
                report.outcome = ProviderOutcomeKind::Bounded;
                report.diagnostic = Some(diagnostic);
            }
        }
        self.missing_checks.push(MissingCheck {
            provider,
            materiality: MissingCheckMateriality::Material,
            code: code.to_string(),
            message: message.to_string(),
        });
        self.warnings.push(DiscoveryWarning {
            code: code.to_string(),
            message: message.to_string(),
            blocking: true,
            evidence_ids: Vec::new(),
        });
    }
}

fn outcome_is_complete(kind: ProviderOutcomeKind) -> bool {
    match kind {
        ProviderOutcomeKind::Complete => true,
        ProviderOutcomeKind::Bounded
        | ProviderOutcomeKind::Unavailable
        | ProviderOutcomeKind::Failed
        | ProviderOutcomeKind::ContractViolation => false,
    }
}

fn outcome_is_contract_violation(kind: ProviderOutcomeKind) -> bool {
    match kind {
        ProviderOutcomeKind::ContractViolation => true,
        ProviderOutcomeKind::Complete
        | ProviderOutcomeKind::Bounded
        | ProviderOutcomeKind::Unavailable
        | ProviderOutcomeKind::Failed => false,
    }
}

fn derive_concepts(request: &DiscoverRequest) -> Vec<DiscoveryConcept> {
    let mut concepts = BTreeSet::new();
    for token in identifier_segments(request.task()) {
        concepts.insert(DiscoveryConcept {
            value: crate::domain::discovery::normalize_discovery_identity(&token),
            provenance: ConceptProvenance::TaskDerived,
        });
    }
    for explicit in request.concepts() {
        concepts.insert(DiscoveryConcept {
            value: crate::domain::discovery::normalize_discovery_identity(explicit),
            provenance: ConceptProvenance::Explicit,
        });
    }
    concepts.into_iter().collect()
}

fn identifier_segments(value: &str) -> Vec<String> {
    let mut segments = Vec::new();
    let mut current = String::new();
    let characters = value.chars().collect::<Vec<_>>();

    for (index, character) in characters.iter().copied().enumerate() {
        if !character.is_alphanumeric() {
            push_segment(&mut segments, &mut current);
            continue;
        }
        let previous = index.checked_sub(1).and_then(|index| characters.get(index));
        let next = characters.get(index + 1);
        let lower_or_digit_boundary = character.is_uppercase()
            && previous.is_some_and(|previous| previous.is_lowercase() || previous.is_numeric());
        let acronym_boundary = character.is_uppercase()
            && previous.is_some_and(|previous| previous.is_uppercase())
            && next.is_some_and(|next| next.is_lowercase());
        if !current.is_empty() && (lower_or_digit_boundary || acronym_boundary) {
            push_segment(&mut segments, &mut current);
        }
        current.push(character);
    }
    push_segment(&mut segments, &mut current);
    segments
}

fn push_segment(segments: &mut Vec<String>, current: &mut String) {
    if current.chars().count() >= 3 {
        segments.push(std::mem::take(current));
    } else {
        current.clear();
    }
}

struct FactMatcher {
    terms: Vec<String>,
    objects: BTreeSet<ArtifactId>,
}

impl FactMatcher {
    fn new(query: &DiscoveryQuery<'_>) -> Self {
        let mut terms = BTreeSet::new();
        for concept in query.concepts() {
            for segment in identifier_segments(&concept.value) {
                terms.insert(crate::domain::discovery::normalize_discovery_identity(
                    &segment,
                ));
            }
        }
        for search_term in query.search_terms() {
            for segment in identifier_segments(search_term) {
                terms.insert(crate::domain::discovery::normalize_discovery_identity(
                    &segment,
                ));
            }
        }
        Self {
            terms: terms.into_iter().collect(),
            objects: query.objects().iter().cloned().collect(),
        }
    }

    fn relevant<'b, I>(&self, artifact: &ArtifactId, supplemental: I) -> bool
    where
        I: IntoIterator<Item = &'b str>,
    {
        if self.objects.contains(artifact) {
            return true;
        }
        let mut values = artifact
            .as_str()
            .split('.')
            .map(str::to_string)
            .collect::<Vec<_>>();
        for value in supplemental {
            values.extend(identifier_segments(value));
        }
        values.iter().any(|value| {
            let normalized = crate::domain::discovery::normalize_discovery_identity(value);
            self.terms
                .iter()
                .any(|term| normalized_embedded_prefix_matches(term, &normalized))
        })
    }
}

fn normalized_embedded_prefix_matches(left: &str, right: &str) -> bool {
    right
        .char_indices()
        .any(|(index, _character)| normalized_prefix_matches(left, &right[index..]))
}

fn normalized_prefix_matches(left: &str, right: &str) -> bool {
    left.chars()
        .zip(right.chars())
        .take_while(|(left, right)| left == right)
        .count()
        >= 3
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::discovery::contract::parse_discover_request;
    use crate::application::discovery::ports::{
        BslSearchPort, DefinitionPort, DiscoveryPorts, ManagedFormPort, MetadataCatalogPort,
        RuntimeFlowPort, SourceInventoryPort, SupportStatePort,
    };
    use crate::domain::discovery::{
        AnalyzedFile, ArtifactId, ArtifactKind, BslFact, ConceptProvenance, ContentHash,
        DefinitionFact, DiscoveryEnvironment, DiscoveryStatus, EvidenceLocation, FactBatch,
        FormFact, MappingFingerprint, MetadataFact, ProviderCoverage, ProviderDiagnostic,
        ProviderKind, ProviderOutcome, RuntimeFlowFact, RuntimeFlowRelationKind, SourceFile,
        SourceInventory, StructuralRelationKind, SupportFact, SupportStateKind,
    };
    use serde_json::{json, Value};
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Barrier};

    #[derive(Clone)]
    struct FakePorts {
        inventory: ProviderOutcome<SourceInventory>,
        metadata: ProviderOutcome<FactBatch<MetadataFact>>,
        forms: ProviderOutcome<FactBatch<FormFact>>,
        lexical: ProviderOutcome<FactBatch<BslFact>>,
        definitions: ProviderOutcome<FactBatch<DefinitionFact>>,
        runtime_flow: ProviderOutcome<FactBatch<RuntimeFlowFact>>,
        support: ProviderOutcome<FactBatch<SupportFact>>,
    }

    impl FakePorts {
        fn complete_empty() -> Self {
            Self {
                inventory: ProviderOutcome::Complete(SourceInventory::empty()),
                metadata: ProviderOutcome::Complete(FactBatch::empty()),
                forms: ProviderOutcome::Complete(FactBatch::empty()),
                lexical: ProviderOutcome::Complete(FactBatch::empty()),
                definitions: ProviderOutcome::Complete(FactBatch::empty()),
                runtime_flow: ProviderOutcome::Complete(FactBatch::empty()),
                support: ProviderOutcome::Complete(FactBatch::empty()),
            }
        }

        fn with_metadata(mut self, batch: FactBatch<MetadataFact>) -> Self {
            self.metadata = ProviderOutcome::Complete(batch);
            self
        }

        fn with_inventory(mut self, files: Vec<SourceFile>) -> Self {
            let bytes = files.iter().map(|file| file.bytes.len() as u64).sum();
            let count = files.len() as u32;
            self.inventory = ProviderOutcome::Complete(SourceInventory {
                files,
                coverage: ProviderCoverage::new(count, count, bytes, count),
            });
            self
        }

        fn with_runtime_flow(
            mut self,
            outcome: ProviderOutcome<FactBatch<RuntimeFlowFact>>,
        ) -> Self {
            self.runtime_flow = outcome;
            self
        }

        fn with_support(mut self, batch: FactBatch<SupportFact>) -> Self {
            self.support = ProviderOutcome::Complete(batch);
            self
        }

        fn only_lexical(batch: FactBatch<BslFact>) -> Self {
            let mut ports = Self::complete_empty();
            ports.lexical = ProviderOutcome::Complete(batch);
            ports
        }

        fn contract_violating_forms() -> Self {
            let mut ports = Self::complete_empty();
            ports.forms = ProviderOutcome::ContractViolation(diagnostic("invalid_form_fact"));
            ports
        }

        fn as_ports(&self) -> DiscoveryPorts<'_> {
            DiscoveryPorts {
                source_inventory: self,
                metadata_catalog: self,
                managed_forms: self,
                bsl_search: self,
                definitions: self,
                runtime_flow: self,
                support_state: self,
            }
        }
    }

    impl SourceInventoryPort for FakePorts {
        fn inventory(
            &self,
            _query: &crate::domain::discovery::DiscoveryQuery<'_>,
        ) -> ProviderOutcome<SourceInventory> {
            self.inventory.clone()
        }
    }

    impl MetadataCatalogPort for FakePorts {
        fn metadata(
            &self,
            _query: &crate::domain::discovery::DiscoveryQuery<'_>,
            _files: &SourceInventory,
        ) -> ProviderOutcome<FactBatch<MetadataFact>> {
            self.metadata.clone()
        }
    }

    impl ManagedFormPort for FakePorts {
        fn forms(
            &self,
            _query: &crate::domain::discovery::DiscoveryQuery<'_>,
            _files: &SourceInventory,
        ) -> ProviderOutcome<FactBatch<FormFact>> {
            self.forms.clone()
        }
    }

    impl BslSearchPort for FakePorts {
        fn search(
            &self,
            _query: &crate::domain::discovery::DiscoveryQuery<'_>,
            _files: &SourceInventory,
        ) -> ProviderOutcome<FactBatch<BslFact>> {
            self.lexical.clone()
        }
    }

    impl DefinitionPort for FakePorts {
        fn definitions(
            &self,
            _query: &crate::domain::discovery::DiscoveryQuery<'_>,
        ) -> ProviderOutcome<FactBatch<DefinitionFact>> {
            self.definitions.clone()
        }
    }

    impl RuntimeFlowPort for FakePorts {
        fn runtime_flow(
            &self,
            _query: &crate::domain::discovery::DiscoveryQuery<'_>,
        ) -> ProviderOutcome<FactBatch<RuntimeFlowFact>> {
            self.runtime_flow.clone()
        }
    }

    impl SupportStatePort for FakePorts {
        fn support(
            &self,
            _query: &crate::domain::discovery::DiscoveryQuery<'_>,
            _files: &SourceInventory,
        ) -> ProviderOutcome<FactBatch<SupportFact>> {
            self.support.clone()
        }
    }

    struct BlockingMetadataPort {
        entered: Arc<Barrier>,
    }

    impl MetadataCatalogPort for BlockingMetadataPort {
        fn metadata(
            &self,
            query: &crate::domain::discovery::DiscoveryQuery<'_>,
            _files: &SourceInventory,
        ) -> ProviderOutcome<FactBatch<MetadataFact>> {
            self.entered.wait();
            while !query.is_cancelled() {
                std::thread::yield_now();
            }
            ProviderOutcome::Complete(FactBatch::empty())
        }
    }

    #[derive(Default)]
    struct LaterProviders {
        calls: AtomicUsize,
    }

    impl LaterProviders {
        fn record<T>(&self, value: T) -> T {
            self.calls.fetch_add(1, Ordering::SeqCst);
            value
        }
    }

    impl ManagedFormPort for LaterProviders {
        fn forms(
            &self,
            _query: &crate::domain::discovery::DiscoveryQuery<'_>,
            _files: &SourceInventory,
        ) -> ProviderOutcome<FactBatch<FormFact>> {
            self.record(ProviderOutcome::Complete(FactBatch::empty()))
        }
    }

    impl BslSearchPort for LaterProviders {
        fn search(
            &self,
            _query: &crate::domain::discovery::DiscoveryQuery<'_>,
            _files: &SourceInventory,
        ) -> ProviderOutcome<FactBatch<BslFact>> {
            self.record(ProviderOutcome::Complete(FactBatch::empty()))
        }
    }

    impl DefinitionPort for LaterProviders {
        fn definitions(
            &self,
            _query: &crate::domain::discovery::DiscoveryQuery<'_>,
        ) -> ProviderOutcome<FactBatch<DefinitionFact>> {
            self.record(ProviderOutcome::Complete(FactBatch::empty()))
        }
    }

    impl RuntimeFlowPort for LaterProviders {
        fn runtime_flow(
            &self,
            _query: &crate::domain::discovery::DiscoveryQuery<'_>,
        ) -> ProviderOutcome<FactBatch<RuntimeFlowFact>> {
            self.record(ProviderOutcome::Complete(FactBatch::empty()))
        }
    }

    impl SupportStatePort for LaterProviders {
        fn support(
            &self,
            _query: &crate::domain::discovery::DiscoveryQuery<'_>,
            _files: &SourceInventory,
        ) -> ProviderOutcome<FactBatch<SupportFact>> {
            self.record(ProviderOutcome::Complete(FactBatch::empty()))
        }
    }

    fn execute(
        fake: &FakePorts,
        request: crate::application::discovery::contract::DiscoverRequest,
    ) -> Result<crate::domain::discovery::DiscoveryReport, crate::domain::discovery::DiscoveryError>
    {
        let environment = DiscoveryEnvironment::new(
            PathBuf::from("src/configuration"),
            MappingFingerprint::from_identity("configuration:src/configuration"),
        );
        DiscoverExtensionPointsUseCase::new(fake.as_ports()).execute(&request, &environment)
    }

    #[test]
    fn cancellation_during_a_provider_stops_before_later_providers() {
        let base = FakePorts::complete_empty();
        let entered = Arc::new(Barrier::new(2));
        let metadata = BlockingMetadataPort {
            entered: Arc::clone(&entered),
        };
        let later = LaterProviders::default();
        let ports = DiscoveryPorts {
            source_inventory: &base,
            metadata_catalog: &metadata,
            managed_forms: &later,
            bsl_search: &later,
            definitions: &later,
            runtime_flow: &later,
            support_state: &later,
        };
        let use_case = DiscoverExtensionPointsUseCase::new(ports);
        let request = task_only();
        let environment = DiscoveryEnvironment::new(
            PathBuf::from("src/configuration"),
            MappingFingerprint::from_identity("configuration:src/configuration"),
        );
        let cancellation = crate::domain::cancellation::CancellationToken::new();

        let error = std::thread::scope(|scope| {
            scope.spawn(|| {
                entered.wait();
                cancellation.cancel();
            });
            use_case
                .execute_cancellable(&request, &environment, &cancellation)
                .expect_err("mid-provider cancellation must stop discovery")
        });

        assert_eq!(error, crate::domain::discovery::DiscoveryError::Cancelled);
        assert_eq!(later.calls.load(Ordering::SeqCst), 0);
    }

    fn request(
        task: &str,
        concepts: &[&str],
    ) -> crate::application::discovery::contract::DiscoverRequest {
        let args = json!({
            "mode": "explore",
            "task": task,
            "concepts": concepts,
        });
        let Value::Object(args) = args else {
            unreachable!("test JSON object is static")
        };
        parse_discover_request(&args).expect("valid discovery request")
    }

    fn task_only() -> crate::application::discovery::contract::DiscoverRequest {
        request("Find series extension points", &[])
    }

    fn request_with_evidence_limit(
        maximum: u16,
    ) -> crate::application::discovery::contract::DiscoverRequest {
        let args = json!({
            "mode": "explore",
            "task": "Find series extension points",
            "limits": { "maxEvidence": maximum },
        });
        let Value::Object(args) = args else {
            unreachable!("test JSON object is static")
        };
        parse_discover_request(&args).expect("valid bounded discovery request")
    }

    fn request_with_file_limit(
        maximum: u32,
    ) -> crate::application::discovery::contract::DiscoverRequest {
        let args = json!({
            "mode": "explore",
            "task": "Find series extension points",
            "limits": { "maxFiles": maximum },
        });
        let Value::Object(args) = args else {
            unreachable!("test JSON object is static")
        };
        parse_discover_request(&args).expect("valid file-bounded discovery request")
    }

    fn artifact(value: &str) -> ArtifactId {
        ArtifactId::parse(value).expect("valid test artifact")
    }

    fn series_id() -> ArtifactId {
        artifact("Document.Purchase.TabularSection.Series")
    }

    fn contributor(path: &str, raw: &[u8]) -> AnalyzedFile {
        AnalyzedFile {
            relative_path: PortableRelativePath::parse_str(path).expect("portable test path"),
            raw_hash: ContentHash::sha256(raw),
            bytes: raw.len() as u64,
        }
    }

    fn source_file(path: &str, raw: &[u8]) -> SourceFile {
        SourceFile {
            relative_path: PortableRelativePath::parse_str(path).expect("portable test path"),
            bytes: raw.to_vec(),
            raw_hash: ContentHash::sha256(raw),
        }
    }

    fn location(path: &str, line: u32) -> EvidenceLocation {
        EvidenceLocation {
            relative_path: PortableRelativePath::parse_str(path).expect("portable test path"),
            line: Some(line),
            column: None,
            xml_path: None,
        }
    }

    fn series_metadata_at(path: &str, raw: &[u8]) -> FactBatch<MetadataFact> {
        FactBatch {
            records: vec![MetadataFact {
                artifact: series_id(),
                artifact_kind: ArtifactKind::TabularSection,
                container: Some(artifact("Document.Purchase")),
                container_kind: Some(ArtifactKind::MetadataObject),
                relation: StructuralRelationKind::Contains,
                location: location(path, 7),
            }],
            analyzed_files: vec![contributor(path, raw)],
            contributors: vec![contributor(path, raw)],
            coverage: ProviderCoverage::new(1, 1, raw.len() as u64, 1),
        }
    }

    fn separate_series_metadata(path: &str, raw: &[u8]) -> FactBatch<MetadataFact> {
        separate_section_metadata(path, raw, "Серии")
    }

    fn separate_section_metadata(
        path: &str,
        raw: &[u8],
        section_name: &str,
    ) -> FactBatch<MetadataFact> {
        let document = artifact("Document.ПриобретениеТоваровУслуг");
        let goods = artifact("Document.ПриобретениеТоваровУслуг.TabularSection.Товары");
        FactBatch {
            records: vec![
                MetadataFact {
                    artifact: goods.clone(),
                    artifact_kind: ArtifactKind::TabularSection,
                    container: Some(document.clone()),
                    container_kind: Some(ArtifactKind::MetadataObject),
                    relation: StructuralRelationKind::Contains,
                    location: location(path, 5),
                },
                MetadataFact {
                    artifact: artifact(
                        "Document.ПриобретениеТоваровУслуг.TabularSection.Товары.Attribute.Серия",
                    ),
                    artifact_kind: ArtifactKind::Attribute,
                    container: Some(goods),
                    container_kind: Some(ArtifactKind::TabularSection),
                    relation: StructuralRelationKind::Contains,
                    location: location(path, 8),
                },
                MetadataFact {
                    artifact: artifact(&format!(
                        "Document.ПриобретениеТоваровУслуг.TabularSection.{section_name}"
                    )),
                    artifact_kind: ArtifactKind::TabularSection,
                    container: Some(document),
                    container_kind: Some(ArtifactKind::MetadataObject),
                    relation: StructuralRelationKind::Contains,
                    location: location(path, 13),
                },
            ],
            analyzed_files: vec![contributor(path, raw)],
            contributors: vec![contributor(path, raw)],
            coverage: ProviderCoverage::new(1, 1, raw.len() as u64, 3),
        }
    }

    fn lexical_hit() -> FactBatch<BslFact> {
        let raw = b"procedure FindSeries()";
        FactBatch {
            records: vec![BslFact {
                artifact: artifact("Document.Purchase.Module.ObjectModule"),
                artifact_kind: ArtifactKind::Module,
                matched_text: "Series".to_string(),
                location: location("Documents/Purchase/Ext/ObjectModule.bsl", 1),
            }],
            analyzed_files: vec![contributor("Documents/Purchase/Ext/ObjectModule.bsl", raw)],
            contributors: vec![contributor("Documents/Purchase/Ext/ObjectModule.bsl", raw)],
            coverage: ProviderCoverage::new(1, 1, raw.len() as u64, 1),
        }
    }

    fn typed_runtime_flow() -> FactBatch<RuntimeFlowFact> {
        let raw = b"CheckSeries();";
        FactBatch {
            records: vec![RuntimeFlowFact {
                source: series_id(),
                source_kind: ArtifactKind::TabularSection,
                target: artifact("Document.Purchase.Module.ObjectModule.Method.CheckSeries"),
                target_kind: ArtifactKind::Method,
                relation: RuntimeFlowRelationKind::Calls,
                location: location("Documents/Purchase/Ext/ObjectModule.bsl", 11),
            }],
            analyzed_files: vec![contributor("Documents/Purchase/Ext/ObjectModule.bsl", raw)],
            contributors: vec![contributor("Documents/Purchase/Ext/ObjectModule.bsl", raw)],
            coverage: ProviderCoverage::new(1, 1, raw.len() as u64, 1),
        }
    }

    fn diagnostic(code: &str) -> ProviderDiagnostic {
        ProviderDiagnostic::material(code, format!("diagnostic: {code}"))
    }

    fn conflicting_support_states() -> FactBatch<SupportFact> {
        let raw = b"support";
        FactBatch {
            records: vec![
                SupportFact {
                    artifact: artifact("Document.Purchase"),
                    artifact_kind: ArtifactKind::MetadataObject,
                    state: SupportStateKind::Locked,
                    location: location("Ext/ParentConfigurations.bin", 1),
                },
                SupportFact {
                    artifact: artifact("Document.Purchase"),
                    artifact_kind: ArtifactKind::MetadataObject,
                    state: SupportStateKind::Editable,
                    location: location("Ext/ParentConfigurations.bin", 1),
                },
            ],
            analyzed_files: vec![contributor("Ext/ParentConfigurations.bin", raw)],
            contributors: vec![contributor("Ext/ParentConfigurations.bin", raw)],
            coverage: ProviderCoverage::new(1, 1, raw.len() as u64, 2),
        }
    }

    #[test]
    fn unavailable_flow_provider_keeps_metadata_and_makes_report_partial() {
        let fake = FakePorts::complete_empty()
            .with_inventory(vec![source_file("Documents/Purchase.xml", b"metadata")])
            .with_metadata(series_metadata_at("Documents/Purchase.xml", b"metadata"))
            .with_runtime_flow(ProviderOutcome::Unavailable(diagnostic("index_missing")));

        let report = execute(&fake, task_only()).expect("partial report");

        assert_eq!(report.status, DiscoveryStatus::Partial);
        assert!(report
            .candidates
            .iter()
            .any(|item| item.target == series_id()));
        assert!(report
            .missing_checks
            .iter()
            .any(|item| item.provider == ProviderKind::RuntimeFlow));
        assert!(report
            .evidence
            .iter()
            .any(|item| item.provider == ProviderKind::MetadataCatalog));
        assert_eq!(report.structural_edges.len(), 1);
    }

    #[test]
    fn actual_goods_series_and_relevant_separate_section_emit_structural_warning() {
        let raw = b"actual metadata structure";
        let path = "Documents/ПриобретениеТоваровУслуг.xml";
        let fake = FakePorts::complete_empty()
            .with_inventory(vec![source_file(path, raw)])
            .with_metadata(separate_series_metadata(path, raw));

        let report = execute(
            &fake,
            request(
                "Контролировать срок годности серий при поступлении товаров",
                &[],
            ),
        )
        .expect("structural discovery report");

        let warning = report
            .warnings
            .iter()
            .find(|warning| warning.code == "separate_series_section")
            .expect("separate-series structural warning");
        let expected_evidence = report
            .evidence
            .iter()
            .filter(|evidence| {
                matches!(
                    evidence.target.as_str(),
                    "document.приобретениетоваровуслуг.tabularsection.товары.attribute.серия"
                        | "document.приобретениетоваровуслуг.tabularsection.серии"
                )
            })
            .map(|evidence| evidence.id.clone())
            .collect::<BTreeSet<_>>();
        assert_eq!(
            warning
                .evidence_ids
                .iter()
                .cloned()
                .collect::<BTreeSet<_>>(),
            expected_evidence
        );
        assert_eq!(warning.evidence_ids.len(), 2);
        assert!(warning.message.contains("Товары.Серия"));
        assert!(!warning.message.contains("отклон"));
    }

    #[test]
    fn unrelated_distinct_section_leaf_does_not_emit_series_warning() {
        let raw = b"actual metadata structure";
        let path = "Documents/ПриобретениеТоваровУслуг.xml";
        let fake = FakePorts::complete_empty()
            .with_inventory(vec![source_file(path, raw)])
            .with_metadata(separate_section_metadata(path, raw, "Услуги"));

        let report = execute(&fake, request("Проверить услуги документа", &[]))
            .expect("structural discovery report");

        assert!(report
            .warnings
            .iter()
            .all(|warning| warning.code != "separate_series_section"));
    }

    #[test]
    fn conflicting_metadata_kinds_for_one_artifact_invalidate_the_provider() {
        let raw = b"conflicting metadata kinds";
        let path = "Documents/Purchase.xml";
        let conflicting = FactBatch {
            records: vec![
                MetadataFact {
                    artifact: artifact("Document.Purchase"),
                    artifact_kind: ArtifactKind::MetadataObject,
                    container: None,
                    container_kind: None,
                    relation: StructuralRelationKind::Contains,
                    location: location(path, 1),
                },
                MetadataFact {
                    artifact: artifact("Document.Purchase"),
                    artifact_kind: ArtifactKind::Form,
                    container: None,
                    container_kind: None,
                    relation: StructuralRelationKind::Contains,
                    location: location(path, 2),
                },
            ],
            analyzed_files: vec![contributor(path, raw)],
            contributors: vec![contributor(path, raw)],
            coverage: ProviderCoverage::new(1, 1, raw.len() as u64, 2),
        };
        let fake = FakePorts::complete_empty()
            .with_inventory(vec![source_file(path, raw)])
            .with_metadata(conflicting);

        let report = execute(&fake, task_only()).expect("partial discovery report");

        assert!(report.provider_outcomes.iter().any(|outcome| {
            outcome.provider == ProviderKind::MetadataCatalog
                && outcome.outcome == ProviderOutcomeKind::ContractViolation
        }));
        assert!(report.related_artifacts.is_empty());
    }

    #[test]
    fn cross_provider_artifact_kind_conflict_invalidates_later_provider_atomically() {
        let metadata_raw = b"metadata";
        let support_raw = b"support";
        let metadata_path = "Documents/Purchase.xml";
        let support_path = "Ext/ParentConfigurations.bin";
        let support = FactBatch {
            records: vec![SupportFact {
                artifact: artifact("Document.Purchase"),
                artifact_kind: ArtifactKind::Form,
                state: SupportStateKind::Locked,
                location: location(support_path, 1),
            }],
            analyzed_files: vec![contributor(support_path, support_raw)],
            contributors: vec![contributor(support_path, support_raw)],
            coverage: ProviderCoverage::new(1, 1, support_raw.len() as u64, 1),
        };
        let fake = FakePorts::complete_empty()
            .with_inventory(vec![
                source_file(metadata_path, metadata_raw),
                source_file(support_path, support_raw),
            ])
            .with_metadata(series_metadata_at(metadata_path, metadata_raw))
            .with_support(support);

        let report = execute(&fake, task_only()).expect("partial discovery report");

        assert!(report.provider_outcomes.iter().any(|outcome| {
            outcome.provider == ProviderKind::SupportState
                && outcome.outcome == ProviderOutcomeKind::ContractViolation
        }));
        assert_eq!(
            report
                .related_artifacts
                .iter()
                .filter(|related| related.artifact == artifact("Document.Purchase"))
                .map(|related| related.kind)
                .collect::<Vec<_>>(),
            vec![ArtifactKind::MetadataObject]
        );
        assert!(report
            .evidence
            .iter()
            .all(|evidence| evidence.provider != ProviderKind::SupportState));
    }

    #[test]
    fn lexical_fact_never_creates_a_runtime_flow_edge_or_candidate() {
        let report = execute(
            &FakePorts::only_lexical(lexical_hit()).with_inventory(vec![source_file(
                "Documents/Purchase/Ext/ObjectModule.bsl",
                b"procedure FindSeries()",
            )]),
            task_only(),
        )
        .expect("lexical report");

        assert!(report.runtime_flow_edges.is_empty());
        assert!(report.candidates.is_empty());
        assert_eq!(report.related_artifacts.len(), 1);
    }

    #[test]
    fn typed_runtime_flow_fact_creates_a_runtime_edge_and_candidate() {
        let mut fake = FakePorts::complete_empty();
        fake.runtime_flow = ProviderOutcome::Complete(typed_runtime_flow());

        let report = execute(&fake, task_only()).expect("runtime-flow report");

        assert_eq!(report.runtime_flow_edges.len(), 1);
        assert_eq!(
            report.runtime_flow_edges[0].relation,
            RuntimeFlowRelationKind::Calls
        );
        assert!(report.candidates.iter().any(|candidate| {
            candidate.target == artifact("Document.Purchase.Module.ObjectModule.Method.CheckSeries")
        }));
    }

    #[test]
    fn contract_violation_excludes_all_records_from_that_provider() {
        let report =
            execute(&FakePorts::contract_violating_forms(), task_only()).expect("partial report");

        assert!(report
            .evidence
            .iter()
            .all(|item| item.provider != ProviderKind::ManagedForms));
        assert!(report.warnings.iter().any(|item| item.blocking));
        assert!(report
            .missing_checks
            .iter()
            .any(|item| item.provider == ProviderKind::ManagedForms));
    }

    #[test]
    fn conflicting_support_records_become_a_contract_violation_and_are_excluded() {
        let mut fake = FakePorts::complete_empty();
        fake.support = ProviderOutcome::Complete(conflicting_support_states());
        fake = fake.with_inventory(vec![source_file(
            "Ext/ParentConfigurations.bin",
            b"support",
        )]);

        let report = execute(&fake, task_only()).expect("partial report");

        assert!(report.provider_outcomes.iter().any(|item| {
            item.provider == ProviderKind::SupportState
                && item.outcome == crate::domain::discovery::ProviderOutcomeKind::ContractViolation
        }));
        assert!(report
            .evidence
            .iter()
            .all(|item| item.provider != ProviderKind::SupportState));
    }

    #[test]
    fn provider_contracts_are_validated_before_global_budget_truncation() {
        let metadata = series_metadata_at("Documents/Purchase.xml", b"metadata");
        let support = conflicting_support_states();
        let mut fake = FakePorts::complete_empty()
            .with_inventory(vec![
                source_file("Documents/Purchase.xml", b"metadata"),
                source_file("Ext/ParentConfigurations.bin", b"support"),
            ])
            .with_metadata(metadata);
        fake.support = ProviderOutcome::Complete(support);

        let report = execute(&fake, request_with_evidence_limit(2)).expect("partial report");

        assert!(report.provider_outcomes.iter().any(|item| {
            item.provider == ProviderKind::SupportState
                && item.outcome == crate::domain::discovery::ProviderOutcomeKind::ContractViolation
        }));
        assert!(report
            .evidence
            .iter()
            .all(|item| item.provider != ProviderKind::SupportState));
    }

    #[test]
    fn complete_inventory_with_lying_coverage_is_a_contract_violation() {
        let mut fake = FakePorts::complete_empty();
        fake.inventory = ProviderOutcome::Complete(SourceInventory {
            files: vec![source_file("Documents/Purchase.xml", b"metadata")],
            coverage: ProviderCoverage::empty(),
        });

        let report = execute(&fake, task_only()).expect("partial report");

        assert!(report.provider_outcomes.iter().any(|item| {
            item.provider == ProviderKind::SourceInventory
                && item.outcome == crate::domain::discovery::ProviderOutcomeKind::ContractViolation
        }));
        assert!(report.evidence.is_empty());
    }

    #[test]
    fn duplicate_canonical_inventory_path_is_a_contract_violation_even_if_identical() {
        let file = source_file("Documents/Purchase.xml", b"metadata");
        let mut fake = FakePorts::complete_empty();
        fake.inventory = ProviderOutcome::Complete(SourceInventory {
            files: vec![file.clone(), file],
            coverage: ProviderCoverage::new(1, 1, 8, 1),
        });

        let report = execute(&fake, task_only()).expect("partial report");

        assert!(report.provider_outcomes.iter().any(|item| {
            item.provider == ProviderKind::SourceInventory
                && item.outcome == crate::domain::discovery::ProviderOutcomeKind::ContractViolation
        }));
    }

    #[test]
    fn inventory_cannot_exceed_validated_query_limits() {
        let fake = FakePorts::complete_empty().with_inventory(vec![
            source_file("Documents/Purchase.xml", b"metadata-a"),
            source_file("Documents/Receipt.xml", b"metadata-b"),
        ]);
        let report = execute(&fake, request_with_file_limit(1)).expect("partial report");

        assert!(report.provider_outcomes.iter().any(|item| {
            item.provider == ProviderKind::SourceInventory
                && item.outcome == crate::domain::discovery::ProviderOutcomeKind::ContractViolation
        }));
    }

    #[test]
    fn bounded_inventory_may_report_the_triggering_n_plus_one_file_seen() {
        let file = source_file("Documents/Purchase.xml", b"metadata");
        let byte_count = file.bytes.len() as u64;
        let mut fake = FakePorts::complete_empty();
        fake.inventory = ProviderOutcome::Bounded {
            data: SourceInventory {
                files: vec![file],
                coverage: ProviderCoverage::new(2, 1, byte_count, 1),
            },
            diagnostic: diagnostic("inventory_file_limit_reached"),
        };

        let report = execute(&fake, request_with_file_limit(1)).expect("bounded report");

        assert!(report.provider_outcomes.iter().any(|item| {
            item.provider == ProviderKind::SourceInventory
                && item.outcome == crate::domain::discovery::ProviderOutcomeKind::Bounded
                && item.coverage == ProviderCoverage::new(2, 1, byte_count, 1)
        }));
    }

    #[test]
    fn bounded_inventory_rejects_u32_max_files_seen_when_it_exceeds_n_plus_one() {
        let file = source_file("Documents/Purchase.xml", b"metadata");
        let byte_count = file.bytes.len() as u64;
        let mut inventory = SourceInventory {
            files: vec![file],
            coverage: ProviderCoverage::new(u32::MAX, 1, byte_count, 1),
        };

        let error = normalize_inventory(
            &mut inventory,
            ProviderOutcomeKind::Bounded,
            DiscoveryQueryLimits {
                max_files: 1,
                max_bytes: u64::MAX,
                max_evidence: u16::MAX,
                max_candidates: u16::MAX,
                max_graph_depth: u8::MAX,
            },
        )
        .expect_err("filesSeen beyond N+1 must violate the bounded contract");

        assert_eq!(error.code, "inventory_files_seen_limit_violation");
    }

    #[test]
    fn bounded_inventory_still_cannot_return_more_than_max_files() {
        let files = vec![
            source_file("Documents/Purchase.xml", b"metadata-a"),
            source_file("Documents/Receipt.xml", b"metadata-b"),
        ];
        let byte_count = files.iter().map(|file| file.bytes.len() as u64).sum();
        let mut fake = FakePorts::complete_empty();
        fake.inventory = ProviderOutcome::Bounded {
            data: SourceInventory {
                files,
                coverage: ProviderCoverage::new(3, 2, byte_count, 2),
            },
            diagnostic: diagnostic("inventory_file_limit_reached"),
        };

        let report = execute(&fake, request_with_file_limit(1)).expect("partial report");

        assert!(report.provider_outcomes.iter().any(|item| {
            item.provider == ProviderKind::SourceInventory
                && item.outcome == crate::domain::discovery::ProviderOutcomeKind::ContractViolation
        }));
    }

    #[test]
    fn complete_inventory_still_requires_exact_files_seen_coverage() {
        let file = source_file("Documents/Purchase.xml", b"metadata");
        let byte_count = file.bytes.len() as u64;
        let mut fake = FakePorts::complete_empty();
        fake.inventory = ProviderOutcome::Complete(SourceInventory {
            files: vec![file],
            coverage: ProviderCoverage::new(2, 1, byte_count, 1),
        });

        let report = execute(&fake, request_with_file_limit(1)).expect("partial report");

        assert!(report.provider_outcomes.iter().any(|item| {
            item.provider == ProviderKind::SourceInventory
                && item.outcome == crate::domain::discovery::ProviderOutcomeKind::ContractViolation
        }));
    }

    #[test]
    fn complete_empty_batch_can_report_no_eligible_files_in_nonempty_inventory() {
        let mut fake = FakePorts::complete_empty()
            .with_inventory(vec![source_file("Documents/Purchase.xml", b"metadata")]);
        fake.metadata = ProviderOutcome::Complete(FactBatch::empty());

        let report = execute(&fake, task_only()).expect("complete report");

        assert!(report.provider_outcomes.iter().any(|item| {
            item.provider == ProviderKind::MetadataCatalog
                && item.outcome == crate::domain::discovery::ProviderOutcomeKind::Complete
                && item.coverage == ProviderCoverage::empty()
        }));
        assert_eq!(report.status, DiscoveryStatus::Complete);
    }

    #[test]
    fn complete_empty_batch_is_negative_for_its_explicit_empty_eligible_scope() {
        let fake = FakePorts::complete_empty()
            .with_inventory(vec![source_file("Documents/Purchase.xml", b"metadata")]);

        let report = execute(&fake, task_only()).expect("complete negative report");

        assert_eq!(report.status, DiscoveryStatus::Complete);
        assert!(report.evidence.is_empty());
        assert!(report.missing_checks.is_empty());
        assert!(report.provider_outcomes.iter().all(|item| {
            item.outcome == crate::domain::discovery::ProviderOutcomeKind::Complete
        }));
    }

    #[test]
    fn complete_provider_scope_does_not_include_ineligible_mixed_inventory_files() {
        let metadata = b"metadata";
        let fake = FakePorts::complete_empty()
            .with_inventory(vec![
                source_file("Documents/Purchase.xml", metadata),
                source_file("Documents/Purchase/Ext/ObjectModule.bsl", b"bsl"),
                source_file("Ext/ParentConfigurations.bin", b"support"),
            ])
            .with_metadata(series_metadata_at("Documents/Purchase.xml", metadata));

        let report = execute(&fake, task_only()).expect("complete mixed-inventory report");

        assert_eq!(report.status, DiscoveryStatus::Complete);
        assert!(report.provider_outcomes.iter().any(|item| {
            item.provider == ProviderKind::MetadataCatalog
                && item.outcome == crate::domain::discovery::ProviderOutcomeKind::Complete
                && item.coverage == ProviderCoverage::new(1, 1, metadata.len() as u64, 1)
        }));
    }

    #[test]
    fn inventory_independent_complete_batch_requires_exact_local_coverage() {
        let mut fake = FakePorts::complete_empty();
        let mut runtime = typed_runtime_flow();
        runtime.coverage.files_seen += 1;
        fake.runtime_flow = ProviderOutcome::Complete(runtime);

        let report = execute(&fake, task_only()).expect("partial report");

        assert!(report.provider_outcomes.iter().any(|item| {
            item.provider == ProviderKind::RuntimeFlow
                && item.outcome == crate::domain::discovery::ProviderOutcomeKind::ContractViolation
        }));
        assert!(report.runtime_flow_edges.is_empty());
    }

    #[test]
    fn bounded_provider_scope_may_leave_seen_eligible_files_unanalyzed() {
        let mut fake = FakePorts::complete_empty().with_inventory(vec![
            source_file("Documents/Purchase.xml", b"metadata"),
            source_file("Documents/Purchase/Ext/ObjectModule.bsl", b"bsl"),
        ]);
        fake.metadata = ProviderOutcome::Bounded {
            data: FactBatch {
                records: Vec::new(),
                analyzed_files: Vec::new(),
                contributors: Vec::new(),
                coverage: ProviderCoverage::new(1, 0, 0, 0),
            },
            diagnostic: diagnostic("metadata_limit_reached"),
        };

        let report = execute(&fake, task_only()).expect("bounded report");

        assert!(report.provider_outcomes.iter().any(|item| {
            item.provider == ProviderKind::MetadataCatalog
                && item.outcome == crate::domain::discovery::ProviderOutcomeKind::Bounded
                && item.coverage == ProviderCoverage::new(1, 0, 0, 0)
        }));
    }

    #[test]
    fn extra_contributor_without_a_fact_invalidates_the_whole_provider() {
        let mut batch = series_metadata_at("Documents/Purchase.xml", b"metadata-a");
        batch
            .contributors
            .push(contributor("Documents/Receipt.xml", b"metadata-b"));
        batch.coverage = ProviderCoverage::new(2, 2, 20, 1);
        let fake = FakePorts::complete_empty()
            .with_inventory(vec![
                source_file("Documents/Purchase.xml", b"metadata-a"),
                source_file("Documents/Receipt.xml", b"metadata-b"),
            ])
            .with_metadata(batch);

        let report = execute(&fake, task_only()).expect("partial report");

        assert!(report.provider_outcomes.iter().any(|item| {
            item.provider == ProviderKind::MetadataCatalog
                && item.outcome == crate::domain::discovery::ProviderOutcomeKind::ContractViolation
        }));
        assert!(report
            .evidence
            .iter()
            .all(|item| item.provider != ProviderKind::MetadataCatalog));
    }

    #[test]
    fn duplicate_canonical_contributor_path_invalidates_the_whole_provider() {
        let mut batch = series_metadata_at("Documents/Purchase.xml", b"metadata");
        batch.contributors.push(batch.contributors[0].clone());
        let fake = FakePorts::complete_empty()
            .with_inventory(vec![source_file("Documents/Purchase.xml", b"metadata")])
            .with_metadata(batch);

        let report = execute(&fake, task_only()).expect("partial report");

        assert!(report.provider_outcomes.iter().any(|item| {
            item.provider == ProviderKind::MetadataCatalog
                && item.outcome == crate::domain::discovery::ProviderOutcomeKind::ContractViolation
        }));
    }

    #[test]
    fn analyzed_file_paths_are_an_exact_identity_set() {
        let duplicate = {
            let mut batch = series_metadata_at("Documents/Purchase.xml", b"metadata");
            batch.analyzed_files.push(batch.analyzed_files[0].clone());
            batch
        };
        let conflict = {
            let mut batch = series_metadata_at("Documents/Purchase.xml", b"metadata");
            let mut conflicting = batch.analyzed_files[0].clone();
            conflicting.raw_hash = ContentHash::sha256(b"different");
            batch.analyzed_files.push(conflicting);
            batch
        };

        for (batch, expected_code) in [
            (duplicate, "duplicate_analyzed_file_path"),
            (conflict, "analyzed_file_path_conflict"),
        ] {
            let report = execute(
                &FakePorts::complete_empty()
                    .with_inventory(vec![source_file("Documents/Purchase.xml", b"metadata")])
                    .with_metadata(batch),
                task_only(),
            )
            .expect("partial report");

            assert!(report.provider_outcomes.iter().any(|item| {
                item.provider == ProviderKind::MetadataCatalog
                    && item.outcome
                        == crate::domain::discovery::ProviderOutcomeKind::ContractViolation
                    && item
                        .diagnostic
                        .as_ref()
                        .is_some_and(|diagnostic| diagnostic.code == expected_code)
            }));
        }
    }

    #[test]
    fn contributor_must_match_its_analyzed_file_identity() {
        let inventory = vec![source_file("Documents/Purchase.xml", b"metadata")];
        let mut wrong_hash = series_metadata_at("Documents/Purchase.xml", b"metadata");
        wrong_hash.contributors[0].raw_hash = ContentHash::sha256(b"different");
        let mut wrong_bytes = series_metadata_at("Documents/Purchase.xml", b"metadata");
        wrong_bytes.contributors[0].bytes += 1;

        for batch in [wrong_hash, wrong_bytes] {
            let report = execute(
                &FakePorts::complete_empty()
                    .with_inventory(inventory.clone())
                    .with_metadata(batch),
                task_only(),
            )
            .expect("partial report");

            assert!(report.provider_outcomes.iter().any(|item| {
                item.provider == ProviderKind::MetadataCatalog
                    && item.outcome
                        == crate::domain::discovery::ProviderOutcomeKind::ContractViolation
            }));
            assert!(report
                .evidence
                .iter()
                .all(|item| item.provider != ProviderKind::MetadataCatalog));
        }
    }

    #[test]
    fn analyzed_file_must_match_inventory_path_hash_and_byte_identity() {
        let inventory = vec![source_file("Documents/Purchase.xml", b"metadata")];
        let mut wrong_hash = series_metadata_at("Documents/Purchase.xml", b"metadata");
        wrong_hash.analyzed_files[0].raw_hash = ContentHash::sha256(b"different");
        wrong_hash.contributors[0] = wrong_hash.analyzed_files[0].clone();
        let mut wrong_bytes = series_metadata_at("Documents/Purchase.xml", b"metadata");
        wrong_bytes.analyzed_files[0].bytes += 1;
        wrong_bytes.contributors[0] = wrong_bytes.analyzed_files[0].clone();
        wrong_bytes.coverage.bytes_analyzed += 1;
        let outside_inventory = series_metadata_at("Documents/Outside.xml", b"metadata");

        for batch in [wrong_hash, wrong_bytes, outside_inventory] {
            let report = execute(
                &FakePorts::complete_empty()
                    .with_inventory(inventory.clone())
                    .with_metadata(batch),
                task_only(),
            )
            .expect("partial report");

            assert!(report.provider_outcomes.iter().any(|item| {
                item.provider == ProviderKind::MetadataCatalog
                    && item.outcome
                        == crate::domain::discovery::ProviderOutcomeKind::ContractViolation
            }));
            assert!(report
                .evidence
                .iter()
                .all(|item| item.provider != ProviderKind::MetadataCatalog));
        }
    }

    #[test]
    fn lying_batch_coverage_invalidates_the_whole_provider() {
        let mut batch = series_metadata_at("Documents/Purchase.xml", b"metadata");
        batch.coverage = ProviderCoverage::empty();
        let fake = FakePorts::complete_empty()
            .with_inventory(vec![source_file("Documents/Purchase.xml", b"metadata")])
            .with_metadata(batch);

        let report = execute(&fake, task_only()).expect("partial report");

        assert!(report.provider_outcomes.iter().any(|item| {
            item.provider == ProviderKind::MetadataCatalog
                && item.outcome == crate::domain::discovery::ProviderOutcomeKind::ContractViolation
        }));
        assert!(report
            .evidence
            .iter()
            .all(|item| item.provider != ProviderKind::MetadataCatalog));
    }

    #[test]
    fn analyzed_files_without_facts_are_coverage_not_snapshot_contributors() {
        let contributing = b"metadata-a";
        let no_match = b"metadata-b";
        let mut batch = series_metadata_at("Documents/Purchase.xml", contributing);
        batch
            .analyzed_files
            .push(contributor("Documents/Receipt.xml", no_match));
        batch.coverage =
            ProviderCoverage::new(2, 2, (contributing.len() + no_match.len()) as u64, 1);
        let fake = FakePorts::complete_empty()
            .with_inventory(vec![
                source_file("Documents/Purchase.xml", contributing),
                source_file("Documents/Receipt.xml", no_match),
            ])
            .with_metadata(batch);

        let report = execute(&fake, task_only()).expect("complete report");

        assert_eq!(report.status, DiscoveryStatus::Complete);
        assert_eq!(report.evidence.len(), 1);
        assert_eq!(report.analysis_snapshot.contributors.len(), 1);
        assert_eq!(
            report.analysis_snapshot.contributors[0]
                .relative_path
                .as_str(),
            "Documents/Purchase.xml"
        );
    }

    #[test]
    fn bounded_outcome_without_truncated_coverage_is_a_contract_violation() {
        let mut fake = FakePorts::complete_empty();
        fake.inventory = ProviderOutcome::Bounded {
            data: SourceInventory::empty(),
            diagnostic: diagnostic("bounded_without_limit"),
        };

        let report = execute(&fake, task_only()).expect("partial report");

        assert!(report.provider_outcomes.iter().any(|item| {
            item.provider == ProviderKind::SourceInventory
                && item.outcome == crate::domain::discovery::ProviderOutcomeKind::ContractViolation
        }));
    }

    #[test]
    fn every_non_complete_outcome_is_partial_and_has_a_missing_check() {
        let mut fake = FakePorts::complete_empty();
        fake.metadata = ProviderOutcome::Bounded {
            data: FactBatch::empty(),
            diagnostic: diagnostic("metadata_bounded"),
        };
        fake.definitions = ProviderOutcome::Failed(diagnostic("definitions_failed"));
        fake.runtime_flow = ProviderOutcome::Unavailable(diagnostic("flow_unavailable"));
        fake.forms = ProviderOutcome::ContractViolation(diagnostic("forms_invalid"));

        let report = execute(&fake, task_only()).expect("partial report");

        assert_eq!(report.status, DiscoveryStatus::Partial);
        for provider in [
            ProviderKind::MetadataCatalog,
            ProviderKind::Definitions,
            ProviderKind::RuntimeFlow,
            ProviderKind::ManagedForms,
        ] {
            assert_eq!(
                report
                    .provider_outcomes
                    .iter()
                    .filter(|item| item.provider == provider)
                    .count(),
                1,
                "exactly one outcome for {provider:?}"
            );
            assert!(report
                .missing_checks
                .iter()
                .any(|item| item.provider == provider));
        }
    }

    #[test]
    fn empty_complete_outcomes_are_complete_negative_evidence() {
        let report = execute(&FakePorts::complete_empty(), task_only()).expect("complete report");

        assert_eq!(report.status, DiscoveryStatus::Complete);
        assert_eq!(report.provider_outcomes.len(), 7);
        assert!(report.missing_checks.is_empty());
        assert!(report.evidence.is_empty());

        let payload = serde_json::to_value(report).expect("serialize complete report");
        let serialized = payload.to_string();
        for forbidden in ["task", "receipt", "score", "confidence"] {
            assert!(
                !serialized.contains(&format!("\"{forbidden}\"")),
                "report must not contain `{forbidden}`"
            );
        }
    }

    #[test]
    fn concept_derivation_splits_identifiers_without_case_folding() {
        let report = execute(
            &FakePorts::complete_empty(),
            request(
                "FindSeries ПроверитьСрок Straße STRASSE XMLParser НДСРасчет",
                &["  СЕРИИ  "],
            ),
        )
        .expect("concept report");

        let concepts = report
            .concepts
            .iter()
            .map(|item| (item.value.as_str(), item.provenance))
            .collect::<Vec<_>>();
        assert!(concepts.contains(&("find", ConceptProvenance::TaskDerived)));
        assert!(concepts.contains(&("series", ConceptProvenance::TaskDerived)));
        assert!(concepts.contains(&("проверить", ConceptProvenance::TaskDerived)));
        assert!(concepts.contains(&("срок", ConceptProvenance::TaskDerived)));
        assert!(concepts.contains(&("straße", ConceptProvenance::TaskDerived)));
        assert!(concepts.contains(&("strasse", ConceptProvenance::TaskDerived)));
        assert!(concepts.contains(&("xml", ConceptProvenance::TaskDerived)));
        assert!(concepts.contains(&("parser", ConceptProvenance::TaskDerived)));
        assert!(concepts.contains(&("ндс", ConceptProvenance::TaskDerived)));
        assert!(concepts.contains(&("расчет", ConceptProvenance::TaskDerived)));
        assert!(!concepts.contains(&("xmlparser", ConceptProvenance::TaskDerived)));
        assert!(!concepts.contains(&("ндсрасчет", ConceptProvenance::TaskDerived)));
        assert!(concepts.contains(&("серии", ConceptProvenance::Explicit)));
        assert!(!concepts.iter().any(|(value, _)| *value == "ser"));
    }

    #[test]
    fn acronym_segments_can_select_candidates_by_their_full_normalized_parts() {
        let raw = b"metadata";
        let path = "DataProcessors.xml";
        let batch = FactBatch {
            records: vec![
                MetadataFact {
                    artifact: artifact("DataProcessor.ParserHook"),
                    artifact_kind: ArtifactKind::MetadataObject,
                    container: None,
                    container_kind: None,
                    relation: StructuralRelationKind::Contains,
                    location: location(path, 1),
                },
                MetadataFact {
                    artifact: artifact("DataProcessor.РасчетHandler"),
                    artifact_kind: ArtifactKind::MetadataObject,
                    container: None,
                    container_kind: None,
                    relation: StructuralRelationKind::Contains,
                    location: location(path, 2),
                },
            ],
            analyzed_files: vec![contributor(path, raw)],
            contributors: vec![contributor(path, raw)],
            coverage: ProviderCoverage::new(1, 1, raw.len() as u64, 2),
        };
        let fake = FakePorts::complete_empty()
            .with_inventory(vec![source_file(path, raw)])
            .with_metadata(batch);

        let report = execute(&fake, request("XMLParser НДСРасчет", &[])).expect("candidate report");

        assert!(report
            .candidates
            .iter()
            .any(|item| item.target == artifact("DataProcessor.ParserHook")));
        assert!(report
            .candidates
            .iter()
            .any(|item| item.target == artifact("DataProcessor.РасчетHandler")));
    }

    #[test]
    fn task_segment_selects_a_concatenated_platform_identifier_without_a_dictionary() {
        let raw = b"metadata";
        let path = "DataProcessors/Series.xml";
        let target = artifact("DataProcessor.ПодборСерийВДокументы");
        let batch = FactBatch {
            records: vec![MetadataFact {
                artifact: target.clone(),
                artifact_kind: ArtifactKind::MetadataObject,
                container: None,
                container_kind: None,
                relation: StructuralRelationKind::Contains,
                location: location(path, 1),
            }],
            analyzed_files: vec![contributor(path, raw)],
            contributors: vec![contributor(path, raw)],
            coverage: ProviderCoverage::new(1, 1, raw.len() as u64, 1),
        };
        let fake = FakePorts::complete_empty()
            .with_inventory(vec![source_file(path, raw)])
            .with_metadata(batch);

        let report = execute(
            &fake,
            request("При поступлении контролировать срок годности серий", &[]),
        )
        .expect("candidate report");

        assert!(report.candidates.iter().any(|item| item.target == target));
    }

    #[test]
    fn report_serialization_and_snapshot_are_independent_of_provider_record_order() {
        let first = series_metadata_at("Documents/Purchase.xml", b"metadata-a");
        let mut second = series_metadata_at("Documents/Receipt.xml", b"metadata-b");
        second.records[0].artifact = artifact("Document.Receipt.TabularSection.Series");
        second.records[0].container = Some(artifact("Document.Receipt"));

        let mut forward_batch = FactBatch {
            records: vec![first.records[0].clone(), second.records[0].clone()],
            analyzed_files: vec![
                first.contributors[0].clone(),
                second.contributors[0].clone(),
            ],
            contributors: vec![
                first.contributors[0].clone(),
                second.contributors[0].clone(),
            ],
            coverage: ProviderCoverage::new(2, 2, 20, 2),
        };
        let mut reverse_batch = forward_batch.clone();
        reverse_batch.records.reverse();
        reverse_batch.analyzed_files.reverse();
        reverse_batch.contributors.reverse();

        let forward = execute(
            &FakePorts::complete_empty()
                .with_inventory(vec![
                    source_file("Documents/Purchase.xml", b"metadata-a"),
                    source_file("Documents/Receipt.xml", b"metadata-b"),
                ])
                .with_metadata(forward_batch.clone()),
            task_only(),
        )
        .expect("forward report");
        forward_batch.records.reverse();
        forward_batch.contributors.reverse();
        let reverse = execute(
            &FakePorts::complete_empty()
                .with_inventory(vec![
                    source_file("Documents/Receipt.xml", b"metadata-b"),
                    source_file("Documents/Purchase.xml", b"metadata-a"),
                ])
                .with_metadata(reverse_batch),
            task_only(),
        )
        .expect("reverse report");

        assert_eq!(forward.analysis_snapshot, reverse.analysis_snapshot);
        assert_eq!(
            serde_json::to_vec(&forward).expect("serialize forward report"),
            serde_json::to_vec(&reverse).expect("serialize reverse report")
        );
    }

    #[test]
    fn raw_content_hash_changes_evidence_and_snapshot_fingerprints() {
        let first = execute(
            &FakePorts::complete_empty()
                .with_inventory(vec![source_file("Documents/Purchase.xml", b"a\n")])
                .with_metadata(series_metadata_at("Documents/Purchase.xml", b"a\n")),
            task_only(),
        )
        .expect("first report");
        let second = execute(
            &FakePorts::complete_empty()
                .with_inventory(vec![source_file("Documents/Purchase.xml", b"a\r\n")])
                .with_metadata(series_metadata_at("Documents/Purchase.xml", b"a\r\n")),
            task_only(),
        )
        .expect("second report");

        assert_ne!(first.evidence[0].id, second.evidence[0].id);
        assert_ne!(
            first.analysis_snapshot.fingerprint,
            second.analysis_snapshot.fingerprint
        );
    }

    #[test]
    fn provider_data_exceeding_max_evidence_is_a_contract_violation() {
        let first = series_metadata_at("Documents/Purchase.xml", b"metadata-a");
        let mut second = series_metadata_at("Documents/Receipt.xml", b"metadata-b");
        second.records[0].artifact = artifact("Document.Receipt.TabularSection.Series");
        second.records[0].container = Some(artifact("Document.Receipt"));
        let batch = FactBatch {
            records: vec![first.records[0].clone(), second.records[0].clone()],
            analyzed_files: vec![
                first.contributors[0].clone(),
                second.contributors[0].clone(),
            ],
            contributors: vec![
                first.contributors[0].clone(),
                second.contributors[0].clone(),
            ],
            coverage: ProviderCoverage::new(2, 2, 20, 2),
        };

        let report = execute(
            &FakePorts::complete_empty()
                .with_inventory(vec![
                    source_file("Documents/Purchase.xml", b"metadata-a"),
                    source_file("Documents/Receipt.xml", b"metadata-b"),
                ])
                .with_metadata(batch),
            request_with_evidence_limit(1),
        )
        .expect("bounded report");

        assert!(report.evidence.is_empty());
        assert!(report.analysis_snapshot.contributors.is_empty());
        assert_eq!(report.status, DiscoveryStatus::Partial);
        assert!(report.provider_outcomes.iter().any(|item| {
            item.provider == ProviderKind::MetadataCatalog
                && item.outcome == crate::domain::discovery::ProviderOutcomeKind::ContractViolation
        }));
        assert!(report.related_artifacts.is_empty());
        assert!(report.structural_edges.is_empty());
        assert!(report.candidates.is_empty());
    }

    #[test]
    fn global_evidence_budget_stops_later_provider_before_graph_materialization() {
        let mut fake = FakePorts::complete_empty()
            .with_inventory(vec![source_file("Documents/Purchase.xml", b"metadata")])
            .with_metadata(series_metadata_at("Documents/Purchase.xml", b"metadata"));
        fake.runtime_flow = ProviderOutcome::Complete(typed_runtime_flow());

        let report = execute(&fake, request_with_evidence_limit(1)).expect("bounded report");

        assert_eq!(report.evidence.len(), 1);
        assert_eq!(report.evidence[0].provider, ProviderKind::MetadataCatalog);
        assert_eq!(report.structural_edges.len(), 1);
        assert!(report.runtime_flow_edges.is_empty());
        assert!(report.provider_outcomes.iter().any(|item| {
            item.provider == ProviderKind::RuntimeFlow
                && item.outcome == crate::domain::discovery::ProviderOutcomeKind::Bounded
                && item
                    .diagnostic
                    .as_ref()
                    .is_some_and(|diagnostic| diagnostic.code == "max_evidence_reached")
        }));
    }

    #[test]
    fn exhausted_budget_still_rejects_later_conflicting_analyzed_identity() {
        let metadata = b"metadata";
        let no_match = b"receipt-metadata";
        let flow = b"CheckSeries();";
        let conflicting = b"changed-receipt-metadata";
        let flow_path = "Documents/Purchase/Ext/ObjectModule.bsl";
        let runtime = FactBatch {
            records: vec![RuntimeFlowFact {
                source: series_id(),
                source_kind: ArtifactKind::TabularSection,
                target: artifact("Document.Purchase.Module.ObjectModule.Method.CheckSeries"),
                target_kind: ArtifactKind::Method,
                relation: RuntimeFlowRelationKind::Calls,
                location: location(flow_path, 11),
            }],
            analyzed_files: vec![
                contributor("Documents/Receipt.xml", conflicting),
                contributor(flow_path, flow),
            ],
            contributors: vec![contributor(flow_path, flow)],
            coverage: ProviderCoverage::new(2, 2, (conflicting.len() + flow.len()) as u64, 1),
        };
        let mut metadata_batch = series_metadata_at("Documents/Purchase.xml", metadata);
        metadata_batch
            .analyzed_files
            .push(contributor("Documents/Receipt.xml", no_match));
        metadata_batch.coverage =
            ProviderCoverage::new(2, 2, (metadata.len() + no_match.len()) as u64, 1);
        let mut fake = FakePorts::complete_empty()
            .with_inventory(vec![
                source_file("Documents/Purchase.xml", metadata),
                source_file("Documents/Receipt.xml", no_match),
            ])
            .with_metadata(metadata_batch);
        fake.runtime_flow = ProviderOutcome::Complete(runtime);

        let report = execute(&fake, request_with_evidence_limit(1)).expect("partial report");

        assert_eq!(report.evidence.len(), 1);
        assert_eq!(report.evidence[0].provider, ProviderKind::MetadataCatalog);
        assert!(report.provider_outcomes.iter().any(|item| {
            item.provider == ProviderKind::RuntimeFlow
                && item.outcome == crate::domain::discovery::ProviderOutcomeKind::ContractViolation
                && item.diagnostic.as_ref().is_some_and(|diagnostic| {
                    diagnostic.code == "cross_provider_file_identity_conflict"
                })
        }));
        assert!(report.runtime_flow_edges.is_empty());
    }

    #[test]
    fn exhausted_runtime_conflicts_with_validated_inventory_only_identity() {
        let inventory_path = "Documents/InventoryOnly.xml";
        let inventory_raw = b"inventory";
        let definition_path = "External/Definitions.bsl";
        let definition_raw = b"procedure FindSeries()";
        let flow_path = "External/Flow.bsl";
        let flow_raw = b"CheckSeries();";

        let definitions = FactBatch {
            records: vec![DefinitionFact {
                owner: artifact("CommonModule.Series"),
                definition: artifact("CommonModule.Series.Method.FindSeries"),
                name: "FindSeries".to_string(),
                location: location(definition_path, 1),
            }],
            analyzed_files: vec![contributor(definition_path, definition_raw)],
            contributors: vec![contributor(definition_path, definition_raw)],
            coverage: ProviderCoverage::new(1, 1, definition_raw.len() as u64, 1),
        };
        let runtime = FactBatch {
            records: vec![RuntimeFlowFact {
                source: series_id(),
                source_kind: ArtifactKind::TabularSection,
                target: artifact("CommonModule.Series.Method.FindSeries"),
                target_kind: ArtifactKind::Method,
                relation: RuntimeFlowRelationKind::Calls,
                location: location(flow_path, 1),
            }],
            analyzed_files: vec![
                contributor(inventory_path, b"conflicting-inventory"),
                contributor(flow_path, flow_raw),
            ],
            contributors: vec![contributor(flow_path, flow_raw)],
            coverage: ProviderCoverage::new(
                2,
                2,
                (b"conflicting-inventory".len() + flow_raw.len()) as u64,
                1,
            ),
        };
        let mut fake = FakePorts::complete_empty()
            .with_inventory(vec![source_file(inventory_path, inventory_raw)]);
        fake.definitions = ProviderOutcome::Complete(definitions);
        fake.runtime_flow = ProviderOutcome::Complete(runtime);

        let report = execute(&fake, request_with_evidence_limit(1)).expect("partial report");

        assert_eq!(report.evidence.len(), 1);
        assert_eq!(report.evidence[0].provider, ProviderKind::Definitions);
        assert!(report.provider_outcomes.iter().any(|item| {
            item.provider == ProviderKind::RuntimeFlow
                && item.outcome == crate::domain::discovery::ProviderOutcomeKind::ContractViolation
                && item.diagnostic.as_ref().is_some_and(|diagnostic| {
                    diagnostic.code == "cross_provider_file_identity_conflict"
                })
        }));
        assert!(report.runtime_flow_edges.is_empty());
        assert_eq!(report.analysis_snapshot.contributors.len(), 1);
        assert!(report
            .analysis_snapshot
            .contributors
            .iter()
            .all(|file| file.relative_path.as_str() != inventory_path));
    }
}
