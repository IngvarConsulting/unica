use crate::application::discovery::contract::DiscoverRequest;
use crate::application::discovery::ports::DiscoveryPorts;
use crate::domain::cancellation::CancellationToken;
use crate::domain::discovery::{
    AnalysisSnapshot, AnalyzedFile, ArtifactId, ArtifactKind, BslFact, CandidateRecommendation,
    CandidateRecommendationBasis, ConceptProvenance, DefinitionFact, DiscoveryConcept,
    DiscoveryEnvironment, DiscoveryError, DiscoveryMatchStrength, DiscoveryMatcher, DiscoveryQuery,
    DiscoveryQueryLimits, DiscoveryReport, DiscoverySource, DiscoveryStatus, DiscoveryWarning,
    Evidence, EvidenceId, EvidenceKind, EvidenceRelation, ExtensionPointCandidate, FactBatch,
    FormBinding, FormFact, LocatedFact, MetadataFact, MissingCheck, MissingCheckMateriality,
    PortableRelativePath, ProviderCoverage, ProviderDiagnostic, ProviderKind, ProviderOutcome,
    ProviderOutcomeKind, ProviderReport, RelatedArtifact, RuntimeFlowEdge, RuntimeFlowFact,
    SnapshotFingerprint, SourceInventory, SourceInventoryBound, StructuralEdge,
    StructuralRelationKind, SupportFact, SupportStateKind, DISCOVERY_MATCH_WORK_BOUND_CODE,
    SOURCE_INVENTORY_TRAVERSAL_BOUND_CODE,
};
use std::collections::{BTreeMap, BTreeSet};

pub(crate) struct DiscoverExtensionPointsUseCase<'a> {
    ports: DiscoveryPorts<'a>,
}

impl<'a> DiscoverExtensionPointsUseCase<'a> {
    pub(crate) fn new(ports: DiscoveryPorts<'a>) -> Self {
        Self { ports }
    }

    pub(crate) fn execute_cancellable(
        &self,
        request: &DiscoverRequest,
        environment: &DiscoveryEnvironment,
        cancellation: &CancellationToken,
    ) -> Result<DiscoveryReport, DiscoveryError> {
        self.execute_inner(request, environment, cancellation)
    }

    fn execute_inner(
        &self,
        request: &DiscoverRequest,
        environment: &DiscoveryEnvironment,
        cancellation: &CancellationToken,
    ) -> Result<DiscoveryReport, DiscoveryError> {
        if environment.source_root().as_os_str().is_empty() {
            return Err(DiscoveryError::EmptySourceRoot);
        }

        let concepts = derive_concepts(request);
        let limits = request.limits();
        let query = DiscoveryQuery::new(
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
        )
        .with_cancellation(cancellation);
        let matcher = DiscoveryMatcher::new(&query);
        let mut accumulator = ReportAccumulator::new(query.limits().max_evidence as usize);

        ensure_discovery_active(&query)?;
        let inventory_outcome = self.ports.source_inventory.inventory(&query);
        ensure_discovery_active(&query)?;
        let mut inventory =
            EvaluatedOutcome::from_outcome(ProviderKind::SourceInventory, inventory_outcome);
        let inventory_diagnostic = inventory.diagnostic.as_ref();
        if let Some(files) = inventory.data.as_mut() {
            if let Err(diagnostic) =
                normalize_inventory(files, inventory.kind, inventory_diagnostic, query.limits())
            {
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
                    BatchPolicy::new(query.limits(), 6, Some(files)).with_matcher(&matcher),
                    |records| prioritize_metadata_records(records, &matcher),
                    |batch| metadata_contribution(batch, &matcher),
                );
                ensure_discovery_active(&query)?;
                let forms = self.ports.managed_forms.forms(&query, files);
                ensure_discovery_active(&query)?;
                handle_batch(
                    ProviderKind::ManagedForms,
                    forms,
                    &mut accumulator,
                    BatchPolicy::new(query.limits(), 5, Some(files)).with_matcher(&matcher),
                    |records| prioritize_form_records(records, &matcher),
                    |batch| form_contribution(batch, &matcher),
                );
                ensure_discovery_active(&query)?;
                let lexical = self.ports.bsl_search.search(&query, files);
                ensure_discovery_active(&query)?;
                handle_batch(
                    ProviderKind::BslSearch,
                    lexical,
                    &mut accumulator,
                    BatchPolicy::new(query.limits(), 4, Some(files)),
                    |_records| {},
                    lexical_contribution,
                );
                ensure_discovery_active(&query)?;
                let support = self.ports.support_state.support(&query, files);
                ensure_discovery_active(&query)?;
                handle_batch(
                    ProviderKind::SupportState,
                    support,
                    &mut accumulator,
                    BatchPolicy::new(query.limits(), 3, Some(files)).with_matcher(&matcher),
                    |records| prioritize_support_records(records, &matcher),
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
            BatchPolicy::new(query.limits(), 2, None),
            |_records| {},
            definition_contribution,
        );
        ensure_discovery_active(&query)?;
        let runtime_flow = self.ports.runtime_flow.runtime_flow(&query);
        ensure_discovery_active(&query)?;
        handle_batch(
            ProviderKind::RuntimeFlow,
            runtime_flow,
            &mut accumulator,
            BatchPolicy::new(query.limits(), 1, None).with_matcher(&matcher),
            |records| prioritize_runtime_records(records, &matcher),
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

provider_facts_without_additional_contract!(BslFact, DefinitionFact,);

impl ProviderFactContract for FormFact {
    fn validate_contract(records: &[Self]) -> Result<(), ProviderDiagnostic> {
        let mut kinds = BTreeMap::new();
        for fact in records {
            validate_artifact_kind(&mut kinds, &fact.form, ArtifactKind::Form)?;
            match &fact.binding {
                FormBinding::Data {
                    target,
                    target_kind,
                    data_path,
                } => {
                    if !matches!(
                        target_kind,
                        ArtifactKind::MetadataObject
                            | ArtifactKind::TabularSection
                            | ArtifactKind::Attribute
                    ) {
                        return Err(provider_contract_diagnostic(
                            "form_data_target_kind_invalid",
                            "managed-form data bindings must target a metadata object, tabular section, or attribute",
                        ));
                    }
                    validate_platform_path(data_path, "form_data_path_invalid")?;
                    validate_artifact_kind(&mut kinds, target, *target_kind)?;
                }
                FormBinding::Command {
                    command,
                    handler,
                    target,
                    target_kind,
                } => {
                    validate_platform_name(command, "form_command_name_invalid")?;
                    validate_form_handler(
                        &fact.form,
                        handler,
                        target,
                        *target_kind,
                        "form_command_handler_invalid",
                    )?;
                    validate_artifact_kind(&mut kinds, target, *target_kind)?;
                }
                FormBinding::Event {
                    event,
                    handler,
                    target,
                    target_kind,
                } => {
                    validate_platform_name(event, "form_event_name_invalid")?;
                    validate_form_handler(
                        &fact.form,
                        handler,
                        target,
                        *target_kind,
                        "form_event_handler_invalid",
                    )?;
                    validate_artifact_kind(&mut kinds, target, *target_kind)?;
                }
            }
        }
        Ok(())
    }
}

fn validate_form_handler(
    form: &ArtifactId,
    handler: &str,
    target: &ArtifactId,
    target_kind: ArtifactKind,
    code: &str,
) -> Result<(), ProviderDiagnostic> {
    validate_platform_name(handler, code)?;
    if target_kind != ArtifactKind::Method {
        return Err(provider_contract_diagnostic(
            code,
            "managed-form command and event bindings must target a method",
        ));
    }
    let expected = ArtifactId::parse(&format!(
        "{}.Module.FormModule.Method.{handler}",
        form.display_str()
    ))
    .map_err(|_error| {
        provider_contract_diagnostic(code, "managed-form handler identity is invalid")
    })?;
    if target != &expected {
        return Err(provider_contract_diagnostic(
            code,
            "managed-form handler target must be the canonical method beneath the form module",
        ));
    }
    Ok(())
}

fn validate_platform_path(value: &str, code: &str) -> Result<(), ProviderDiagnostic> {
    if value.trim() != value
        || value.is_empty()
        || value
            .split('.')
            .any(|segment| !platform_name_is_valid(segment))
    {
        return Err(provider_contract_diagnostic(
            code,
            "managed-form data path must contain nonblank canonical platform identifiers",
        ));
    }
    Ok(())
}

fn validate_platform_name(value: &str, code: &str) -> Result<(), ProviderDiagnostic> {
    if !platform_name_is_valid(value) {
        return Err(provider_contract_diagnostic(
            code,
            "managed-form binding name must be a nonblank canonical platform identifier",
        ));
    }
    Ok(())
}

fn platform_name_is_valid(value: &str) -> bool {
    value.trim() == value
        && !value.is_empty()
        && value
            .chars()
            .next()
            .is_some_and(|character| character == '_' || character.is_alphabetic())
        && value
            .chars()
            .all(|character| character == '_' || character.is_alphanumeric())
}

impl ProviderFactContract for RuntimeFlowFact {
    fn validate_contract(records: &[Self]) -> Result<(), ProviderDiagnostic> {
        let mut kinds = BTreeMap::new();
        for fact in records {
            validate_artifact_kind(&mut kinds, &fact.source, fact.source_kind)?;
            validate_artifact_kind(&mut kinds, &fact.target, fact.target_kind)?;
            let valid = match fact.relation {
                crate::domain::discovery::RuntimeFlowRelationKind::Calls => {
                    fact.source_kind == ArtifactKind::Method
                        && fact.target_kind == ArtifactKind::Method
                }
                crate::domain::discovery::RuntimeFlowRelationKind::Action => {
                    matches!(fact.source_kind, ArtifactKind::Form | ArtifactKind::Command)
                        && fact.target_kind == ArtifactKind::Method
                }
                crate::domain::discovery::RuntimeFlowRelationKind::Callback => {
                    fact.source_kind == ArtifactKind::Form
                        && fact.target_kind == ArtifactKind::Method
                }
                crate::domain::discovery::RuntimeFlowRelationKind::EventSubscription => false,
            };
            if !valid {
                return Err(provider_contract_diagnostic(
                    "runtime_flow_shape_invalid",
                    "runtime-flow fact does not satisfy the fail-closed typed relation matrix",
                ));
            }
        }
        Ok(())
    }
}

impl ProviderFactContract for MetadataFact {
    fn validate_contract(records: &[Self]) -> Result<(), ProviderDiagnostic> {
        let mut kinds = BTreeMap::new();
        let mut parents = BTreeMap::new();
        for fact in records {
            if fact.relation != StructuralRelationKind::Contains {
                return Err(provider_contract_diagnostic(
                    "metadata_relation_invalid",
                    "metadata catalog facts must use the contains relation",
                ));
            }
            let search_name =
                crate::domain::discovery::normalize_discovery_identity(&fact.search_name);
            let artifact_leaf = fact.artifact.normalized_str().rsplit('.').next();
            if artifact_leaf != Some(search_name.as_str()) {
                return Err(provider_contract_diagnostic(
                    "metadata_search_name_mismatch",
                    "metadata raw search name must normalize to the canonical artifact leaf",
                ));
            }
            if !metadata_artifact_kind_is_valid(fact.artifact_kind) {
                return Err(provider_contract_diagnostic(
                    "metadata_artifact_kind_invalid",
                    "metadata catalog facts must describe metadata objects, tabular sections, attributes, forms, or commands",
                ));
            }
            validate_artifact_kind(&mut kinds, &fact.artifact, fact.artifact_kind)?;
            let parent = match (&fact.container, fact.container_kind) {
                (Some(container), Some(container_kind)) => {
                    let claimed_parent = Some((container.clone(), container_kind));
                    if parents
                        .get(&fact.artifact)
                        .is_some_and(|previous| previous != &claimed_parent)
                    {
                        return Err(provider_contract_diagnostic(
                            "metadata_parent_conflict",
                            "metadata artifact must have one canonical parent relationship",
                        ));
                    }
                    if container == &fact.artifact {
                        return Err(provider_contract_diagnostic(
                            "metadata_self_parent",
                            "metadata artifact cannot contain itself",
                        ));
                    }
                    if !metadata_artifact_kind_is_valid(container_kind)
                        || !metadata_contains_kind_is_valid(container_kind, fact.artifact_kind)
                    {
                        return Err(provider_contract_diagnostic(
                            "metadata_hierarchy_kind_invalid",
                            "metadata contains relation has an invalid parent and child kind combination",
                        ));
                    }
                    validate_artifact_kind(&mut kinds, container, container_kind)?;
                    claimed_parent
                }
                (None, None) => {
                    if parents.get(&fact.artifact).is_some_and(Option::is_some) {
                        return Err(provider_contract_diagnostic(
                            "metadata_parent_conflict",
                            "metadata artifact must have one canonical parent relationship",
                        ));
                    }
                    None
                }
                _ => {
                    return Err(provider_contract_diagnostic(
                        "metadata_container_kind_missing",
                        "metadata container identity and kind must be supplied together",
                    ));
                }
            };
            parents.entry(fact.artifact.clone()).or_insert(parent);
        }
        validate_metadata_hierarchy_is_acyclic(&parents)?;
        for fact in records {
            if let Some(container) = &fact.container {
                if !metadata_contains_identity_is_valid(
                    container,
                    &fact.artifact,
                    fact.artifact_kind,
                ) {
                    return Err(provider_contract_diagnostic(
                        "metadata_hierarchy_identity_invalid",
                        "metadata contains relation must connect canonical direct parent and child identities",
                    ));
                }
            }
        }
        Ok(())
    }
}

fn metadata_artifact_kind_is_valid(kind: ArtifactKind) -> bool {
    matches!(
        kind,
        ArtifactKind::MetadataObject
            | ArtifactKind::TabularSection
            | ArtifactKind::Attribute
            | ArtifactKind::Form
            | ArtifactKind::Command
    )
}

fn metadata_contains_kind_is_valid(parent: ArtifactKind, child: ArtifactKind) -> bool {
    match parent {
        ArtifactKind::MetadataObject => metadata_artifact_kind_is_valid(child),
        ArtifactKind::TabularSection => child == ArtifactKind::Attribute,
        ArtifactKind::Attribute
        | ArtifactKind::Form
        | ArtifactKind::Command
        | ArtifactKind::Module
        | ArtifactKind::Method => false,
    }
}

fn metadata_contains_identity_is_valid(
    parent: &ArtifactId,
    child: &ArtifactId,
    child_kind: ArtifactKind,
) -> bool {
    let parent_identity = parent.normalized_str();
    let child_identity = child.normalized_str();
    let is_direct_descendant = child_identity
        .strip_prefix(parent_identity)
        .and_then(|suffix| suffix.strip_prefix('.'))
        .is_some_and(|suffix| suffix.split('.').count() == 2);
    if is_direct_descendant {
        return true;
    }
    parent_identity.starts_with("configuration.")
        && parent_identity.split('.').count() == 2
        && child_kind == ArtifactKind::MetadataObject
        && child_identity.split('.').count() == 2
}

fn validate_metadata_hierarchy_is_acyclic(
    parents: &BTreeMap<ArtifactId, Option<(ArtifactId, ArtifactKind)>>,
) -> Result<(), ProviderDiagnostic> {
    for artifact in parents.keys() {
        let mut visited = BTreeSet::new();
        let mut current = artifact;
        while let Some(Some((parent, _parent_kind))) = parents.get(current) {
            if !visited.insert(current.clone()) {
                return Err(provider_contract_diagnostic(
                    "metadata_hierarchy_cycle",
                    "metadata contains relationships must form an acyclic hierarchy",
                ));
            }
            current = parent;
        }
    }
    Ok(())
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
        self.coverage = ProviderCoverage::empty();
        self.diagnostic = Some(diagnostic);
    }
}

struct BatchPolicy<'a> {
    limits: DiscoveryQueryLimits,
    remaining_provider_slots: usize,
    inventory: Option<&'a SourceInventory>,
    matcher: Option<&'a DiscoveryMatcher>,
}

impl<'a> BatchPolicy<'a> {
    fn new(
        limits: DiscoveryQueryLimits,
        remaining_provider_slots: usize,
        inventory: Option<&'a SourceInventory>,
    ) -> Self {
        Self {
            limits,
            remaining_provider_slots,
            inventory,
            matcher: None,
        }
    }

    fn with_matcher(mut self, matcher: &'a DiscoveryMatcher) -> Self {
        self.matcher = Some(matcher);
        self
    }
}

fn handle_batch<T, P, F>(
    provider: ProviderKind,
    outcome: ProviderOutcome<FactBatch<T>>,
    accumulator: &mut ReportAccumulator,
    policy: BatchPolicy<'_>,
    prioritize: P,
    build: F,
) where
    T: Clone + Ord + LocatedFact + ProviderFactContract,
    P: FnOnce(&mut Vec<T>),
    F: FnOnce(&FactBatch<T>) -> Result<ProviderContribution, ProviderDiagnostic>,
{
    let mut evaluated = EvaluatedOutcome::from_outcome(provider, outcome);
    if evaluated.kind == ProviderOutcomeKind::Complete
        && policy.inventory.is_some_and(source_inventory_is_incomplete)
    {
        evaluated.kind = ProviderOutcomeKind::Bounded;
        evaluated.diagnostic = Some(ProviderDiagnostic::material(
            "source_inventory_incomplete",
            "provider evidence is limited to an incomplete source inventory",
        ));
    }
    let mut supplemental_diagnostic = None;
    let contribution =
        match evaluated.data.as_mut() {
            Some(batch) => {
                match normalize_batch(batch, evaluated.kind, policy.limits, policy.inventory)
                    .and_then(|()| {
                        accumulator.validate_file_identities(&batch.analyzed_files)?;
                        accumulator.validate_file_identities(&batch.contributors)?;
                        let analyzed_files = batch.analyzed_files.clone();
                        let remaining = accumulator.remaining_evidence();
                        let allowance =
                            fair_evidence_allowance(remaining, policy.remaining_provider_slots);
                        prioritize(&mut batch.records);
                        if batch.records.len() > allowance {
                            batch.records.truncate(allowance);
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
                        if policy.matcher.is_some_and(DiscoveryMatcher::work_exhausted) {
                            let diagnostic = ProviderDiagnostic::material(
                            DISCOVERY_MATCH_WORK_BOUND_CODE,
                            "typed discovery matching stopped at its request-derived work limit",
                        );
                            if outcome_is_complete(evaluated.kind) {
                                evaluated.kind = ProviderOutcomeKind::Bounded;
                                evaluated.diagnostic = Some(diagnostic);
                            } else if !evaluated.diagnostic.as_ref().is_some_and(|current| {
                                current.code == DISCOVERY_MATCH_WORK_BOUND_CODE
                            }) {
                                supplemental_diagnostic = Some(diagnostic);
                            }
                        }
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

fn source_inventory_is_incomplete(inventory: &SourceInventory) -> bool {
    inventory.coverage.files_seen > inventory.coverage.files_analyzed || inventory.bound.is_some()
}

fn fair_evidence_allowance(remaining: usize, provider_slots: usize) -> usize {
    if remaining == 0 || provider_slots == 0 {
        return 0;
    }
    remaining.div_ceil(provider_slots)
}

fn normalize_inventory(
    inventory: &mut SourceInventory,
    outcome: ProviderOutcomeKind,
    diagnostic: Option<&ProviderDiagnostic>,
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
            if inventory.bound.is_some() {
                return Err(provider_contract_diagnostic(
                    "complete_inventory_bound_marker",
                    "complete source inventory must not carry a resource-bound marker",
                ));
            }
            if inventory.coverage.files_seen != file_count {
                return Err(provider_contract_diagnostic(
                    "complete_inventory_coverage_mismatch",
                    "complete source inventory must account exactly for every returned file",
                ));
            }
        }
        ProviderOutcomeKind::Bounded => {
            let max_files_seen = limits.max_files.saturating_add(1);
            if inventory.coverage.files_seen > max_files_seen {
                return Err(provider_contract_diagnostic(
                    "inventory_files_seen_limit_violation",
                    "bounded source inventory observed more than the triggering N+1 file",
                ));
            }
            let traversal_bound = inventory.bound == Some(SourceInventoryBound::TraversalEntries);
            let traversal_diagnostic = diagnostic
                .is_some_and(|diagnostic| diagnostic.code == SOURCE_INVENTORY_TRAVERSAL_BOUND_CODE);
            if traversal_bound != traversal_diagnostic {
                return Err(provider_contract_diagnostic(
                    "inventory_bound_diagnostic_mismatch",
                    "source inventory bound marker conflicts with its typed diagnostic",
                ));
            }
            let has_n_plus_one_probe = file_count
                .checked_add(1)
                .is_some_and(|trigger| inventory.coverage.files_seen == trigger);
            let demonstrates_bound = has_n_plus_one_probe || traversal_bound;
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

fn normalize_analyzed_files(analyzed_files: &mut [AnalyzedFile]) -> Result<(), ProviderDiagnostic> {
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
    runtime_roots: BTreeSet<ArtifactId>,
    candidates: Vec<(
        ArtifactId,
        ArtifactKind,
        EvidenceId,
        DiscoveryMatchStrength,
        CandidateRecommendationBasis,
    )>,
    support: Vec<(ArtifactId, SupportStateKind)>,
    warnings: Vec<DiscoveryWarning>,
}

fn prioritize_metadata_records(records: &mut [MetadataFact], matcher: &DiscoveryMatcher) {
    let containers = records
        .iter()
        .filter_map(|fact| {
            fact.container
                .as_ref()
                .map(|container| (fact.artifact.clone(), container.clone()))
        })
        .collect::<BTreeMap<_, _>>();
    let strengths = records
        .iter()
        .map(|fact| {
            (
                fact.artifact.clone(),
                matcher.strength(&fact.artifact, std::iter::once(fact.search_name.as_str())),
            )
        })
        .fold(
            BTreeMap::<ArtifactId, DiscoveryMatchStrength>::new(),
            |mut strengths, (artifact, strength)| {
                strengths
                    .entry(artifact)
                    .and_modify(|known| *known = (*known).max(strength))
                    .or_insert(strength);
                strengths
            },
        );
    let mut structural_context = BTreeSet::new();
    for artifact in strengths
        .iter()
        .filter(|(_artifact, strength)| strength.is_match())
        .map(|(artifact, _strength)| artifact)
    {
        let mut current = artifact;
        while let Some(container) = containers.get(current) {
            if !structural_context.insert(container.clone()) {
                break;
            }
            current = container;
        }
    }
    records.sort_by(|left, right| {
        let left_strength = strengths
            .get(&left.artifact)
            .copied()
            .unwrap_or(DiscoveryMatchStrength::NONE);
        let right_strength = strengths
            .get(&right.artifact)
            .copied()
            .unwrap_or(DiscoveryMatchStrength::NONE);
        let retention_rank = |fact: &MetadataFact, strength: DiscoveryMatchStrength| {
            if strength.is_strong_match() {
                3
            } else if structural_context.contains(&fact.artifact) {
                2
            } else if strength.is_match() {
                1
            } else {
                0
            }
        };
        retention_rank(right, right_strength)
            .cmp(&retention_rank(left, left_strength))
            .then_with(|| right_strength.cmp(&left_strength))
            .then_with(|| left.cmp(right))
    });
}

fn prioritize_form_records(records: &mut [FormFact], matcher: &DiscoveryMatcher) {
    records.sort_by(|left, right| {
        form_fact_strength(right, matcher)
            .cmp(&form_fact_strength(left, matcher))
            .then_with(|| left.cmp(right))
    });
}

fn form_fact_strength(fact: &FormFact, matcher: &DiscoveryMatcher) -> DiscoveryMatchStrength {
    match &fact.binding {
        FormBinding::Data {
            target, data_path, ..
        } => matcher
            .strength(&fact.form, std::iter::once(data_path.as_str()))
            .max(matcher.strength(target, std::iter::once(data_path.as_str()))),
        FormBinding::Command {
            command,
            handler,
            target,
            ..
        } => {
            let supplemental = [command.as_str(), handler.as_str()];
            matcher
                .strength(&fact.form, supplemental)
                .max(matcher.strength(target, supplemental))
        }
        FormBinding::Event {
            event,
            handler,
            target,
            ..
        } => {
            let supplemental = [event.as_str(), handler.as_str()];
            matcher
                .strength(&fact.form, supplemental)
                .max(matcher.strength(target, supplemental))
        }
    }
}

fn prioritize_support_records(records: &mut [SupportFact], matcher: &DiscoveryMatcher) {
    records.sort_by(|left, right| {
        matcher
            .strength(&right.artifact, std::iter::empty::<&str>())
            .cmp(&matcher.strength(&left.artifact, std::iter::empty::<&str>()))
            .then_with(|| left.cmp(right))
    });
}

fn prioritize_runtime_records(records: &mut [RuntimeFlowFact], matcher: &DiscoveryMatcher) {
    use crate::domain::discovery::RuntimeFlowRelationKind;

    records.sort_by(|left, right| {
        let rank = |fact: &RuntimeFlowFact| {
            let base = matches!(
                fact.relation,
                RuntimeFlowRelationKind::Action | RuntimeFlowRelationKind::Callback
            );
            let strength = matcher
                .strength(&fact.source, std::iter::empty::<&str>())
                .max(matcher.strength(&fact.target, std::iter::empty::<&str>()));
            (base && strength.is_match(), strength, base)
        };
        rank(right).cmp(&rank(left)).then_with(|| left.cmp(right))
    });
}

fn metadata_contribution(
    batch: &FactBatch<MetadataFact>,
    matcher: &DiscoveryMatcher,
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
        let strength = matcher.strength(&fact.artifact, std::iter::once(fact.search_name.as_str()));
        if strength.is_match() {
            contribution.candidates.push((
                fact.artifact.clone(),
                fact.artifact_kind,
                evidence.0.id.clone(),
                strength,
                CandidateRecommendationBasis::MetadataStructure,
            ));
        }
        evidence_by_artifact
            .entry(fact.artifact.clone())
            .or_insert_with(|| evidence.0.id.clone());
        contribution.evidence.push(evidence.0);
    }
    contribution.warnings = alternative_section_warnings(batch, matcher, &evidence_by_artifact);
    Ok(contribution)
}

fn alternative_section_warnings(
    batch: &FactBatch<MetadataFact>,
    matcher: &DiscoveryMatcher,
    evidence_by_artifact: &BTreeMap<ArtifactId, EvidenceId>,
) -> Vec<DiscoveryWarning> {
    let mut warnings = Vec::new();
    for nested_attribute in batch.records.iter().filter(|fact| {
        fact.artifact_kind == ArtifactKind::Attribute
            && fact.relation == StructuralRelationKind::Contains
            && matcher
                .strength(&fact.artifact, std::iter::once(fact.search_name.as_str()))
                .is_match()
    }) {
        let Some(parent_section_id) = nested_attribute.container.as_ref() else {
            continue;
        };
        let Some(parent_section) = batch.records.iter().find(|fact| {
            fact.artifact == *parent_section_id
                && fact.artifact_kind == ArtifactKind::TabularSection
                && fact.relation == StructuralRelationKind::Contains
        }) else {
            continue;
        };
        let Some(metadata_object) = parent_section.container.as_ref() else {
            continue;
        };
        for alternative_section in batch.records.iter().filter(|fact| {
            fact.artifact_kind == ArtifactKind::TabularSection
                && fact.relation == StructuralRelationKind::Contains
                && fact.container.as_ref() == Some(metadata_object)
                && fact.artifact != parent_section.artifact
                && matcher
                    .strength(&fact.artifact, std::iter::once(fact.search_name.as_str()))
                    .is_match()
        }) {
            let Some(attribute_evidence) = evidence_by_artifact.get(&nested_attribute.artifact)
            else {
                continue;
            };
            let Some(section_evidence) = evidence_by_artifact.get(&alternative_section.artifact)
            else {
                continue;
            };
            let mut evidence_ids = vec![attribute_evidence.clone(), section_evidence.clone()];
            evidence_ids.sort();
            warnings.push(DiscoveryWarning {
                code: "alternative_relevant_tabular_section".to_string(),
                message: "A point limited to a relevant nested attribute lacks coverage: the same metadata object contains another task-relevant tabular section."
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

fn form_contribution(
    batch: &FactBatch<FormFact>,
    matcher: &DiscoveryMatcher,
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
        let strength = matcher
            .strength(&fact.form, supplemental.iter().copied())
            .max(matcher.strength(target, supplemental.iter().copied()));
        if strength.is_match() {
            if runtime_relation.is_some() {
                contribution.runtime_roots.insert(target.clone());
            }
            contribution.candidates.push((
                fact.form.clone(),
                ArtifactKind::Form,
                evidence.0.id.clone(),
                strength,
                CandidateRecommendationBasis::ManagedFormBinding,
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
    matcher: &DiscoveryMatcher,
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
        let strength = matcher
            .strength(&fact.source, std::iter::empty::<&str>())
            .max(matcher.strength(&fact.target, std::iter::empty::<&str>()));
        if matches!(
            fact.relation,
            crate::domain::discovery::RuntimeFlowRelationKind::Action
                | crate::domain::discovery::RuntimeFlowRelationKind::Callback
        ) && strength.is_match()
        {
            contribution.runtime_roots.insert(fact.target.clone());
            contribution.candidates.push((
                fact.target.clone(),
                fact.target_kind,
                evidence.0.id.clone(),
                strength,
                CandidateRecommendationBasis::ProvenRuntimeFlow,
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
    runtime_roots: BTreeSet<ArtifactId>,
    candidates: BTreeMap<(ArtifactId, ArtifactKind), CandidateEvidence>,
    support: BTreeMap<ArtifactId, SupportStateKind>,
}

struct CandidateEvidence {
    strength: DiscoveryMatchStrength,
    evidence_ids: BTreeSet<EvidenceId>,
    recommendation_basis: BTreeSet<CandidateRecommendationBasis>,
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
        self.runtime_roots.extend(contribution.runtime_roots);
        for (target, kind, evidence_id, strength, recommendation_basis) in contribution.candidates {
            let candidate =
                self.candidates
                    .entry((target, kind))
                    .or_insert_with(|| CandidateEvidence {
                        strength,
                        evidence_ids: BTreeSet::new(),
                        recommendation_basis: BTreeSet::new(),
                    });
            candidate.strength = candidate.strength.max(strength);
            candidate.evidence_ids.insert(evidence_id);
            candidate.recommendation_basis.insert(recommendation_basis);
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
        self.apply_runtime_graph_depth(limits.max_graph_depth);
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
            .map(|((target, kind), candidate)| {
                let support_state = self.support.get(&target).copied();
                ExtensionPointCandidate {
                    target,
                    kind,
                    evidence_ids: candidate.evidence_ids.into_iter().collect(),
                    recommendation: candidate_recommendation(candidate.recommendation_basis),
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

    fn apply_runtime_graph_depth(&mut self, maximum: u8) {
        use crate::domain::discovery::RuntimeFlowRelationKind;

        let mut retained_calls = BTreeSet::new();
        let mut depth_by_method = self
            .runtime_roots
            .iter()
            .cloned()
            .map(|root| (root, 0_u8))
            .collect::<BTreeMap<_, _>>();
        let mut pending = self.runtime_roots.iter().cloned().collect::<Vec<_>>();
        let mut cursor = 0;
        let mut truncated = false;
        while let Some(source) = pending.get(cursor).cloned() {
            cursor += 1;
            let depth = depth_by_method.get(&source).copied().unwrap_or(0);
            let outgoing = self
                .runtime_flow_edges
                .keys()
                .filter(|(edge_source, _target, relation)| {
                    edge_source == &source && *relation == RuntimeFlowRelationKind::Calls
                })
                .cloned()
                .collect::<Vec<_>>();
            if depth >= maximum {
                truncated |= !outgoing.is_empty();
                continue;
            }
            for edge in outgoing {
                let target = edge.1.clone();
                retained_calls.insert(edge);
                let target_depth = depth.saturating_add(1);
                let should_visit = depth_by_method
                    .get(&target)
                    .is_none_or(|known_depth| target_depth < *known_depth);
                if should_visit {
                    depth_by_method.insert(target.clone(), target_depth);
                    pending.push(target);
                }
            }
        }

        self.runtime_flow_edges.retain(|key, _evidence_ids| {
            key.2 != RuntimeFlowRelationKind::Calls || retained_calls.contains(key)
        });
        if truncated {
            self.record_limit(
                ProviderKind::RuntimeFlow,
                "graph_depth_limit",
                "runtime call-graph traversal was truncated by maxGraphDepth",
            );
        }
    }

    fn apply_candidate_limit(&mut self, maximum: usize) {
        if self.candidates.len() <= maximum {
            return;
        }
        let mut ranked = self
            .candidates
            .iter()
            .map(|(key, candidate)| (candidate.strength, key.clone()))
            .collect::<Vec<_>>();
        ranked.sort_by(|(left_strength, left_key), (right_strength, right_key)| {
            right_strength
                .cmp(left_strength)
                .then_with(|| left_key.cmp(right_key))
        });
        let retained = ranked
            .into_iter()
            .take(maximum)
            .map(|(_strength, key)| key)
            .collect::<BTreeSet<_>>();
        let affected = self
            .candidates
            .iter()
            .filter(|(key, _candidate)| !retained.contains(*key))
            .flat_map(|(_key, candidate)| &candidate.evidence_ids)
            .filter_map(|id| self.evidence.get(id))
            .map(|evidence| evidence.provider)
            .collect::<BTreeSet<_>>();
        self.candidates
            .retain(|key, _candidate| retained.contains(key));
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

fn candidate_recommendation(
    basis: BTreeSet<CandidateRecommendationBasis>,
) -> CandidateRecommendation {
    let ordered_basis = basis.iter().copied().collect::<Vec<_>>();
    let summary = match ordered_basis.as_slice() {
        [CandidateRecommendationBasis::MetadataStructure] =>
            "Review this structural metadata point as an extension-point candidate; typed metadata evidence connects it to the task.",
        [CandidateRecommendationBasis::ManagedFormBinding] =>
            "Review this managed form as an extension-point candidate; typed form-binding evidence connects it to the task-relevant flow.",
        [CandidateRecommendationBasis::ProvenRuntimeFlow] =>
            "Review this runtime target as an extension-point candidate; typed runtime-flow evidence connects it to the task.",
        _ =>
            "Review this point as an extension-point candidate; multiple accepted typed evidence paths connect it to the task.",
    };
    CandidateRecommendation {
        summary: summary.to_string(),
        basis: basis.into_iter().collect(),
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
    for token in crate::domain::discovery::discovery_identifier_segments(request.task()) {
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
                bound: None,
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

        fn with_forms(mut self, batch: FactBatch<FormFact>) -> Self {
            self.forms = ProviderOutcome::Complete(batch);
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
        DiscoverExtensionPointsUseCase::new(fake.as_ports()).execute_cancellable(
            &request,
            &environment,
            &CancellationToken::new(),
        )
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

    fn request_with_graph_depth(
        maximum: u8,
    ) -> crate::application::discovery::contract::DiscoverRequest {
        let args = json!({
            "mode": "explore",
            "task": "Find series extension points",
            "limits": { "maxGraphDepth": maximum },
        });
        let Value::Object(args) = args else {
            unreachable!("test JSON object is static")
        };
        parse_discover_request(&args).expect("valid depth-bounded discovery request")
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
            bytes: raw.to_vec().into(),
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
                search_name: "Series".to_string(),
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
                    search_name: "Товары".to_string(),
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
                    search_name: "Серия".to_string(),
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
                    search_name: section_name.to_string(),
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
                source: artifact("DataProcessor.Purchase.Form.SeriesSelection"),
                source_kind: ArtifactKind::Form,
                target: artifact("Document.Purchase.Module.ObjectModule.Method.CheckSeries"),
                target_kind: ArtifactKind::Method,
                relation: RuntimeFlowRelationKind::Callback,
                location: location("Documents/Purchase/Ext/ObjectModule.bsl", 11),
            }],
            analyzed_files: vec![contributor("Documents/Purchase/Ext/ObjectModule.bsl", raw)],
            contributors: vec![contributor("Documents/Purchase/Ext/ObjectModule.bsl", raw)],
            coverage: ProviderCoverage::new(1, 1, raw.len() as u64, 1),
        }
    }

    fn runtime_call_chain() -> FactBatch<RuntimeFlowFact> {
        let raw = b"OpenSeries(); NextSeries(); FinishSeries();";
        let form = artifact("DataProcessor.Purchase.Form.SeriesSelection");
        let first = artifact(
            "DataProcessor.Purchase.Form.SeriesSelection.Module.FormModule.Method.OpenSeries",
        );
        let second = artifact("CommonModule.Series.Method.NextSeries");
        let third = artifact("CommonModule.Series.Method.FinishSeries");
        FactBatch {
            records: vec![
                RuntimeFlowFact {
                    source: form,
                    source_kind: ArtifactKind::Form,
                    target: first.clone(),
                    target_kind: ArtifactKind::Method,
                    relation: RuntimeFlowRelationKind::Action,
                    location: location("DataProcessors/Purchase/Ext/Form/Module.bsl", 1),
                },
                RuntimeFlowFact {
                    source: first,
                    source_kind: ArtifactKind::Method,
                    target: second.clone(),
                    target_kind: ArtifactKind::Method,
                    relation: RuntimeFlowRelationKind::Calls,
                    location: location("DataProcessors/Purchase/Ext/Form/Module.bsl", 2),
                },
                RuntimeFlowFact {
                    source: second,
                    source_kind: ArtifactKind::Method,
                    target: third,
                    target_kind: ArtifactKind::Method,
                    relation: RuntimeFlowRelationKind::Calls,
                    location: location("DataProcessors/Purchase/Ext/Form/Module.bsl", 3),
                },
            ],
            analyzed_files: vec![contributor(
                "DataProcessors/Purchase/Ext/Form/Module.bsl",
                raw,
            )],
            contributors: vec![contributor(
                "DataProcessors/Purchase/Ext/Form/Module.bsl",
                raw,
            )],
            coverage: ProviderCoverage::new(1, 1, raw.len() as u64, 3),
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
    fn relevant_nested_attribute_and_alternative_section_emit_generic_structural_warning() {
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
            .find(|warning| warning.code == "alternative_relevant_tabular_section")
            .expect("alternative-section structural warning");
        let expected_evidence = report
            .evidence
            .iter()
            .filter(|evidence| {
                matches!(
                    evidence.target.normalized_str(),
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
        assert!(warning.message.contains("relevant nested attribute"));
        assert!(!warning.message.contains("Товары"));
        assert!(!warning.message.contains("Серия"));
    }

    #[test]
    fn structural_warning_does_not_require_the_parent_section_name_to_match() {
        let raw = b"actual metadata structure";
        let path = "Documents/ПриобретениеТоваровУслуг.xml";
        let fake = FakePorts::complete_empty()
            .with_inventory(vec![source_file(path, raw)])
            .with_metadata(separate_series_metadata(path, raw));

        let report = execute(&fake, request("серий", &[])).expect("structural discovery report");

        assert!(report
            .warnings
            .iter()
            .any(|warning| warning.code == "alternative_relevant_tabular_section"));
    }

    #[test]
    fn shared_three_character_prefixes_do_not_emit_structural_warning() {
        let raw = b"actual metadata structure";
        let path = "Documents/ПриобретениеТоваровУслуг.xml";
        for section_name in ["СервисныеУслуги", "Сертификаты"] {
            let discovery_request = request(
                "Контролировать срок годности серий при поступлении товаров",
                &[],
            );
            let concepts = derive_concepts(&discovery_request);
            let query = DiscoveryQuery::new(
                discovery_request.task(),
                &concepts,
                discovery_request.search_terms(),
                discovery_request.objects(),
                DiscoveryQueryLimits {
                    max_files: 1,
                    max_bytes: 1,
                    max_evidence: 1,
                    max_candidates: 1,
                    max_graph_depth: 1,
                },
            );
            let matcher = DiscoveryMatcher::new(&query);
            let alternative = artifact(&format!(
                "Document.ПриобретениеТоваровУслуг.TabularSection.{section_name}"
            ));
            let strength = matcher.strength(&alternative, std::iter::once(section_name));
            assert!(!strength.is_match(), "{section_name}: {strength:?}");
            let fake = FakePorts::complete_empty()
                .with_inventory(vec![source_file(path, raw)])
                .with_metadata(separate_section_metadata(path, raw, section_name));

            let report = execute(&fake, discovery_request).expect("structural discovery report");

            assert!(
                report
                    .warnings
                    .iter()
                    .all(|warning| warning.code != "alternative_relevant_tabular_section"),
                "false structural warning for {section_name}: {:?}",
                report.warnings
            );
        }
    }

    #[test]
    fn conflicting_metadata_kinds_for_one_artifact_invalidate_the_provider() {
        let raw = b"conflicting metadata kinds";
        let path = "Documents/Purchase.xml";
        let conflicting = FactBatch {
            records: vec![
                MetadataFact {
                    artifact: artifact("Document.Purchase"),
                    search_name: "Purchase".to_string(),
                    artifact_kind: ArtifactKind::MetadataObject,
                    container: None,
                    container_kind: None,
                    relation: StructuralRelationKind::Contains,
                    location: location(path, 1),
                },
                MetadataFact {
                    artifact: artifact("Document.Purchase"),
                    search_name: "Purchase".to_string(),
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
    fn metadata_search_name_must_normalize_to_the_artifact_leaf() {
        let raw = b"mismatched raw metadata name";
        let path = "DataProcessors/Readiness.xml";
        let batch = FactBatch {
            records: vec![MetadataFact {
                artifact: artifact("DataProcessor.Готовность"),
                search_name: "Товары".to_string(),
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

        let report = execute(&fake, request("товаров", &[])).expect("partial report");

        let metadata = report
            .provider_outcomes
            .iter()
            .find(|outcome| outcome.provider == ProviderKind::MetadataCatalog)
            .expect("metadata outcome");
        assert_eq!(metadata.outcome, ProviderOutcomeKind::ContractViolation);
        assert_eq!(
            metadata
                .diagnostic
                .as_ref()
                .map(|diagnostic| diagnostic.code.as_str()),
            Some("metadata_search_name_mismatch")
        );
        assert!(report.candidates.is_empty());
    }

    fn assert_metadata_hierarchy_contract_violation(
        records: Vec<MetadataFact>,
        expected_code: &str,
    ) {
        let raw = b"invalid metadata hierarchy";
        let path = "Catalogs/Hierarchy.xml";
        let batch = FactBatch {
            coverage: ProviderCoverage::new(1, 1, raw.len() as u64, records.len() as u32),
            records,
            analyzed_files: vec![contributor(path, raw)],
            contributors: vec![contributor(path, raw)],
        };
        let fake = FakePorts::complete_empty()
            .with_inventory(vec![source_file(path, raw)])
            .with_metadata(batch);

        let report = execute(&fake, task_only()).expect("partial report");
        let metadata = report
            .provider_outcomes
            .iter()
            .find(|outcome| outcome.provider == ProviderKind::MetadataCatalog)
            .expect("metadata outcome");
        assert_eq!(metadata.outcome, ProviderOutcomeKind::ContractViolation);
        assert_eq!(metadata.coverage, ProviderCoverage::empty());
        assert_eq!(
            metadata.diagnostic.as_ref().map(|item| item.code.as_str()),
            Some(expected_code)
        );
        assert!(report
            .evidence
            .iter()
            .all(|evidence| evidence.provider != ProviderKind::MetadataCatalog));
    }

    #[test]
    fn metadata_contract_rejects_non_contains_relations_and_non_metadata_kinds() {
        assert_metadata_hierarchy_contract_violation(
            vec![MetadataFact {
                artifact: artifact("Catalog.Products"),
                search_name: "Products".to_string(),
                artifact_kind: ArtifactKind::MetadataObject,
                container: None,
                container_kind: None,
                relation: StructuralRelationKind::Defines,
                location: location("Catalogs/Hierarchy.xml", 1),
            }],
            "metadata_relation_invalid",
        );
        assert_metadata_hierarchy_contract_violation(
            vec![MetadataFact {
                artifact: artifact("CommonModule.Tools.Method.Run"),
                search_name: "Run".to_string(),
                artifact_kind: ArtifactKind::Method,
                container: None,
                container_kind: None,
                relation: StructuralRelationKind::Contains,
                location: location("Catalogs/Hierarchy.xml", 1),
            }],
            "metadata_artifact_kind_invalid",
        );
    }

    #[test]
    fn metadata_contract_rejects_illegal_parent_kinds_and_multiple_parents() {
        assert_metadata_hierarchy_contract_violation(
            vec![MetadataFact {
                artifact: artifact("Catalog.Products.Attribute.Series"),
                search_name: "Series".to_string(),
                artifact_kind: ArtifactKind::Attribute,
                container: Some(artifact("Catalog.Products.Attribute.Parent")),
                container_kind: Some(ArtifactKind::Attribute),
                relation: StructuralRelationKind::Contains,
                location: location("Catalogs/Hierarchy.xml", 1),
            }],
            "metadata_hierarchy_kind_invalid",
        );
        assert_metadata_hierarchy_contract_violation(
            vec![MetadataFact {
                artifact: artifact("Document.Other.TabularSection.Series"),
                search_name: "Series".to_string(),
                artifact_kind: ArtifactKind::TabularSection,
                container: Some(artifact("Document.Purchase")),
                container_kind: Some(ArtifactKind::MetadataObject),
                relation: StructuralRelationKind::Contains,
                location: location("Catalogs/Hierarchy.xml", 1),
            }],
            "metadata_hierarchy_identity_invalid",
        );
        let child = artifact("Catalog.Products.Attribute.Series");
        assert_metadata_hierarchy_contract_violation(
            vec![
                MetadataFact {
                    artifact: child.clone(),
                    search_name: "Series".to_string(),
                    artifact_kind: ArtifactKind::Attribute,
                    container: Some(artifact("Catalog.Products")),
                    container_kind: Some(ArtifactKind::MetadataObject),
                    relation: StructuralRelationKind::Contains,
                    location: location("Catalogs/Hierarchy.xml", 1),
                },
                MetadataFact {
                    artifact: child,
                    search_name: "Series".to_string(),
                    artifact_kind: ArtifactKind::Attribute,
                    container: Some(artifact("Catalog.Alternatives")),
                    container_kind: Some(ArtifactKind::MetadataObject),
                    relation: StructuralRelationKind::Contains,
                    location: location("Catalogs/Hierarchy.xml", 2),
                },
            ],
            "metadata_parent_conflict",
        );
    }

    #[test]
    fn metadata_contract_rejects_direct_child_identity_with_wrong_kind_token() {
        assert_metadata_hierarchy_contract_violation(
            vec![MetadataFact {
                artifact: artifact("Document.Purchase.Form.Series"),
                search_name: "Series".to_string(),
                artifact_kind: ArtifactKind::Attribute,
                container: Some(artifact("Document.Purchase")),
                container_kind: Some(ArtifactKind::MetadataObject),
                relation: StructuralRelationKind::Contains,
                location: location("Catalogs/Hierarchy.xml", 1),
            }],
            "metadata_hierarchy_identity_invalid",
        );
    }

    #[test]
    fn metadata_contract_rejects_self_parenting_and_indirect_cycles() {
        let self_parent = artifact("Catalog.Products");
        assert_metadata_hierarchy_contract_violation(
            vec![MetadataFact {
                artifact: self_parent.clone(),
                search_name: "Products".to_string(),
                artifact_kind: ArtifactKind::MetadataObject,
                container: Some(self_parent),
                container_kind: Some(ArtifactKind::MetadataObject),
                relation: StructuralRelationKind::Contains,
                location: location("Catalogs/Hierarchy.xml", 1),
            }],
            "metadata_self_parent",
        );
        let first = artifact("Catalog.First");
        let second = artifact("Catalog.Second");
        assert_metadata_hierarchy_contract_violation(
            vec![
                MetadataFact {
                    artifact: first.clone(),
                    search_name: "First".to_string(),
                    artifact_kind: ArtifactKind::MetadataObject,
                    container: Some(second.clone()),
                    container_kind: Some(ArtifactKind::MetadataObject),
                    relation: StructuralRelationKind::Contains,
                    location: location("Catalogs/Hierarchy.xml", 1),
                },
                MetadataFact {
                    artifact: second,
                    search_name: "Second".to_string(),
                    artifact_kind: ArtifactKind::MetadataObject,
                    container: Some(first),
                    container_kind: Some(ArtifactKind::MetadataObject),
                    relation: StructuralRelationKind::Contains,
                    location: location("Catalogs/Hierarchy.xml", 2),
                },
            ],
            "metadata_hierarchy_cycle",
        );
    }

    #[test]
    fn matcher_work_exhaustion_is_reported_as_a_bounded_provider() {
        let raw = b"metadata";
        let path = "Documents/Purchase.xml";
        let search_terms = (0..128)
            .map(|index| format!("UnrelatedSearchTerm{index:03}"))
            .collect::<Vec<_>>();
        let args = json!({
            "mode": "explore",
            "task": "Unrelated discovery request",
            "searchTerms": search_terms,
            "limits": { "maxEvidence": 1 },
        });
        let Value::Object(args) = args else {
            unreachable!("test JSON object is static")
        };
        let request = parse_discover_request(&args).expect("work-bounded request");
        let fake = FakePorts::complete_empty()
            .with_inventory(vec![source_file(path, raw)])
            .with_metadata(series_metadata_at(path, raw));

        let report = execute(&fake, request).expect("work-bounded report");
        let metadata = report
            .provider_outcomes
            .iter()
            .find(|outcome| outcome.provider == ProviderKind::MetadataCatalog)
            .expect("metadata outcome");

        assert_eq!(metadata.outcome, ProviderOutcomeKind::Bounded);
        assert_eq!(
            metadata.diagnostic.as_ref().map(|item| item.code.as_str()),
            Some("discovery_match_work_bound")
        );
        assert!(report
            .evidence
            .iter()
            .any(|evidence| evidence.provider == ProviderKind::MetadataCatalog));
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
            RuntimeFlowRelationKind::Callback
        );
        let candidate = report
            .candidates
            .iter()
            .find(|candidate| {
                candidate.target
                    == artifact("Document.Purchase.Module.ObjectModule.Method.CheckSeries")
            })
            .expect("typed runtime candidate");
        assert_eq!(
            serde_json::to_value(candidate).expect("serialized candidate")["recommendation"],
            json!({
                "summary": "Review this runtime target as an extension-point candidate; typed runtime-flow evidence connects it to the task.",
                "basis": ["proven_runtime_flow"]
            })
        );
    }

    #[test]
    fn max_graph_depth_counts_only_directed_calls_after_a_typed_runtime_root() {
        let mut fake = FakePorts::complete_empty();
        fake.runtime_flow = ProviderOutcome::Complete(runtime_call_chain());

        let report = execute(&fake, request_with_graph_depth(1)).expect("depth-bounded report");

        assert!(report.runtime_flow_edges.iter().any(|edge| {
            edge.relation == RuntimeFlowRelationKind::Action
                && edge.target
                    == artifact("DataProcessor.Purchase.Form.SeriesSelection.Module.FormModule.Method.OpenSeries")
        }));
        assert!(report.runtime_flow_edges.iter().any(|edge| {
            edge.relation == RuntimeFlowRelationKind::Calls
                && edge.target == artifact("CommonModule.Series.Method.NextSeries")
        }));
        assert!(!report.runtime_flow_edges.iter().any(|edge| {
            edge.relation == RuntimeFlowRelationKind::Calls
                && edge.target == artifact("CommonModule.Series.Method.FinishSeries")
        }));
        assert!(report.provider_outcomes.iter().any(|outcome| {
            outcome.provider == ProviderKind::RuntimeFlow
                && outcome.outcome == ProviderOutcomeKind::Bounded
                && outcome
                    .diagnostic
                    .as_ref()
                    .is_some_and(|diagnostic| diagnostic.code == "graph_depth_limit")
        }));
    }

    #[test]
    fn standalone_calls_are_not_promoted_without_a_typed_runtime_root() {
        let raw = b"NextSeries();";
        let mut fake = FakePorts::complete_empty();
        fake.runtime_flow = ProviderOutcome::Complete(FactBatch {
            records: vec![RuntimeFlowFact {
                source: artifact("CommonModule.Series.Method.StartSeries"),
                source_kind: ArtifactKind::Method,
                target: artifact("CommonModule.Series.Method.NextSeries"),
                target_kind: ArtifactKind::Method,
                relation: RuntimeFlowRelationKind::Calls,
                location: location("CommonModules/Series/Ext/Module.bsl", 1),
            }],
            analyzed_files: vec![contributor("CommonModules/Series/Ext/Module.bsl", raw)],
            contributors: vec![contributor("CommonModules/Series/Ext/Module.bsl", raw)],
            coverage: ProviderCoverage::new(1, 1, raw.len() as u64, 1),
        });

        let report = execute(&fake, task_only()).expect("unrooted runtime report");

        assert!(report.runtime_flow_edges.is_empty());
        assert!(report.candidates.is_empty());
        assert!(report.provider_outcomes.iter().any(|outcome| {
            outcome.provider == ProviderKind::RuntimeFlow
                && outcome.outcome == ProviderOutcomeKind::Complete
        }));
    }

    #[test]
    fn malformed_runtime_shape_invalidates_the_whole_batch_and_resets_coverage() {
        let raw = b"CheckSeries();";
        let mut fake = FakePorts::complete_empty();
        fake.runtime_flow = ProviderOutcome::Complete(FactBatch {
            records: vec![RuntimeFlowFact {
                source: series_id(),
                source_kind: ArtifactKind::TabularSection,
                target: artifact("CommonModule.Series.Method.CheckSeries"),
                target_kind: ArtifactKind::Method,
                relation: RuntimeFlowRelationKind::Calls,
                location: location("CommonModules/Series/Ext/Module.bsl", 1),
            }],
            analyzed_files: vec![contributor("CommonModules/Series/Ext/Module.bsl", raw)],
            contributors: vec![contributor("CommonModules/Series/Ext/Module.bsl", raw)],
            coverage: ProviderCoverage::new(1, 1, raw.len() as u64, 1),
        });

        let report = execute(&fake, task_only()).expect("contract-violation report");

        let outcome = report
            .provider_outcomes
            .iter()
            .find(|outcome| outcome.provider == ProviderKind::RuntimeFlow)
            .expect("runtime outcome");
        assert_eq!(outcome.outcome, ProviderOutcomeKind::ContractViolation);
        assert_eq!(outcome.coverage, ProviderCoverage::empty());
        assert!(report.runtime_flow_edges.is_empty());
        assert!(report
            .evidence
            .iter()
            .all(|evidence| evidence.provider != ProviderKind::RuntimeFlow));
    }

    #[test]
    fn malformed_form_binding_invalidates_the_whole_batch_and_resets_coverage() {
        let raw = b"<Form/>";
        let path = "DataProcessors/Purchase/Forms/Main/Ext/Form.xml";
        let batch = FactBatch {
            records: vec![FormFact {
                form: artifact("DataProcessor.Purchase.Form.Main"),
                binding: FormBinding::Data {
                    target: artifact("CommonModule.Series.Method.CheckSeries"),
                    target_kind: ArtifactKind::Method,
                    data_path: " ".to_string(),
                },
                location: location(path, 1),
            }],
            analyzed_files: vec![contributor(path, raw)],
            contributors: vec![contributor(path, raw)],
            coverage: ProviderCoverage::new(1, 1, raw.len() as u64, 1),
        };
        let fake = FakePorts::complete_empty()
            .with_inventory(vec![source_file(path, raw)])
            .with_forms(batch);

        let report = execute(&fake, task_only()).expect("contract-violation report");

        let outcome = report
            .provider_outcomes
            .iter()
            .find(|outcome| outcome.provider == ProviderKind::ManagedForms)
            .expect("form outcome");
        assert_eq!(outcome.outcome, ProviderOutcomeKind::ContractViolation);
        assert_eq!(outcome.coverage, ProviderCoverage::empty());
        assert!(report
            .evidence
            .iter()
            .all(|evidence| evidence.provider != ProviderKind::ManagedForms));
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
            bound: None,
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
            bound: None,
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
                bound: None,
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
    fn bounded_inventory_cannot_be_claimed_only_because_the_exact_limit_was_returned() {
        let file = source_file("Documents/Purchase.xml", b"metadata");
        let byte_count = file.bytes.len() as u64;
        let mut inventory = SourceInventory {
            files: vec![file],
            coverage: ProviderCoverage::new(1, 1, byte_count, 1),
            bound: None,
        };

        let error = normalize_inventory(
            &mut inventory,
            ProviderOutcomeKind::Bounded,
            Some(&diagnostic("source_inventory_file_bound")),
            DiscoveryQueryLimits {
                max_files: 1,
                max_bytes: byte_count,
                max_evidence: 1,
                max_candidates: 1,
                max_graph_depth: 1,
            },
        )
        .expect_err("an exact ceiling without an N+1 probe is not a proven bound");

        assert_eq!(error.code, "unsubstantiated_bounded_inventory");
    }

    #[test]
    fn bounded_inventory_keeps_inventory_backed_complete_batches_incomplete() {
        let raw = b"metadata";
        let path = "Documents/Purchase.xml";
        let file = source_file(path, raw);
        let mut fake = FakePorts::complete_empty().with_metadata(series_metadata_at(path, raw));
        fake.inventory = ProviderOutcome::Bounded {
            data: SourceInventory {
                files: vec![file],
                coverage: ProviderCoverage::new(2, 1, raw.len() as u64, 1),
                bound: None,
            },
            diagnostic: diagnostic("source_inventory_file_bound"),
        };

        let report = execute(&fake, request_with_file_limit(1)).expect("bounded report");

        let metadata = report
            .provider_outcomes
            .iter()
            .find(|outcome| outcome.provider == ProviderKind::MetadataCatalog)
            .expect("metadata outcome");
        assert_eq!(metadata.outcome, ProviderOutcomeKind::Bounded);
        assert_eq!(
            metadata.diagnostic.as_ref().map(|item| item.code.as_str()),
            Some("source_inventory_incomplete")
        );
        assert!(report
            .evidence
            .iter()
            .any(|evidence| evidence.provider == ProviderKind::MetadataCatalog));
    }

    #[test]
    fn traversal_bounded_inventory_keeps_eligible_coverage_without_contract_reclassification() {
        let file = source_file("Documents/Purchase.xml", b"metadata");
        let byte_count = file.bytes.len() as u64;
        let mut fake = FakePorts::complete_empty();
        fake.inventory = ProviderOutcome::Bounded {
            data: SourceInventory {
                files: vec![file],
                coverage: ProviderCoverage::new(1, 1, byte_count, 1),
                bound: Some(SourceInventoryBound::TraversalEntries),
            },
            diagnostic: diagnostic("source_inventory_traversal_bound"),
        };

        let report = execute(&fake, task_only()).expect("traversal-bounded report");

        let outcome = report
            .provider_outcomes
            .iter()
            .find(|item| item.provider == ProviderKind::SourceInventory)
            .expect("source inventory outcome");
        assert_eq!(
            outcome.outcome,
            crate::domain::discovery::ProviderOutcomeKind::Bounded
        );
        assert_eq!(outcome.coverage, ProviderCoverage::new(1, 1, byte_count, 1));
        assert_eq!(
            outcome.diagnostic.as_ref().map(|item| item.code.as_str()),
            Some("source_inventory_traversal_bound")
        );
        assert_eq!(report.status, DiscoveryStatus::Partial);
        assert!(report.missing_checks.iter().any(|check| {
            check.provider == ProviderKind::SourceInventory
                && check.code == SOURCE_INVENTORY_TRAVERSAL_BOUND_CODE
        }));
    }

    #[test]
    fn traversal_bound_marker_and_diagnostic_must_agree() {
        let mut fake = FakePorts::complete_empty();
        let mut inventory = SourceInventory::empty();
        inventory.bound = Some(SourceInventoryBound::TraversalEntries);
        fake.inventory = ProviderOutcome::Bounded {
            data: inventory,
            diagnostic: diagnostic("different_inventory_bound"),
        };

        let report = execute(&fake, task_only()).expect("partial report");

        let outcome = report
            .provider_outcomes
            .iter()
            .find(|item| item.provider == ProviderKind::SourceInventory)
            .expect("source inventory outcome");
        assert_eq!(
            outcome.outcome,
            crate::domain::discovery::ProviderOutcomeKind::ContractViolation
        );
        assert_eq!(
            outcome.diagnostic.as_ref().map(|item| item.code.as_str()),
            Some("inventory_bound_diagnostic_mismatch")
        );
    }

    #[test]
    fn bounded_inventory_rejects_u32_max_files_seen_when_it_exceeds_n_plus_one() {
        let file = source_file("Documents/Purchase.xml", b"metadata");
        let byte_count = file.bytes.len() as u64;
        let mut inventory = SourceInventory {
            files: vec![file],
            coverage: ProviderCoverage::new(u32::MAX, 1, byte_count, 1),
            bound: None,
        };

        let error = normalize_inventory(
            &mut inventory,
            ProviderOutcomeKind::Bounded,
            Some(&diagnostic("inventory_file_limit_reached")),
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
                bound: None,
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
            bound: None,
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
                    search_name: "ParserHook".to_string(),
                    artifact_kind: ArtifactKind::MetadataObject,
                    container: None,
                    container_kind: None,
                    relation: StructuralRelationKind::Contains,
                    location: location(path, 1),
                },
                MetadataFact {
                    artifact: artifact("DataProcessor.РасчетHandler"),
                    search_name: "РасчетHandler".to_string(),
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
                search_name: "ПодборСерийВДокументы".to_string(),
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

        let candidate = report
            .candidates
            .iter()
            .find(|item| item.target == target)
            .expect("metadata candidate");
        assert_eq!(
            serde_json::to_value(candidate).expect("serialized candidate")["recommendation"],
            json!({
                "summary": "Review this structural metadata point as an extension-point candidate; typed metadata evidence connects it to the task.",
                "basis": ["metadata_structure"]
            })
        );
    }

    #[test]
    fn task_term_does_not_match_an_arbitrary_identifier_infix() {
        let raw = b"metadata";
        let path = "DataProcessors/Readiness.xml";
        let false_positive = artifact("DataProcessor.Готовность");
        let batch = FactBatch {
            records: vec![MetadataFact {
                artifact: false_positive.clone(),
                search_name: "Готовность".to_string(),
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

        let report = execute(&fake, request("товаров", &[])).expect("candidate report");

        assert!(!report
            .candidates
            .iter()
            .any(|item| item.target == false_positive));
    }

    #[test]
    fn false_infix_matches_cannot_evict_an_intended_bounded_candidate() {
        let raw = b"metadata";
        let path = "DataProcessors/Candidates.xml";
        let false_positive = artifact("DataProcessor.Готовность");
        let intended = artifact("DataProcessor.Товары");
        let batch = FactBatch {
            records: vec![
                MetadataFact {
                    artifact: false_positive,
                    search_name: "Готовность".to_string(),
                    artifact_kind: ArtifactKind::MetadataObject,
                    container: None,
                    container_kind: None,
                    relation: StructuralRelationKind::Contains,
                    location: location(path, 1),
                },
                MetadataFact {
                    artifact: intended.clone(),
                    search_name: "Товары".to_string(),
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
        let args = json!({
            "mode": "explore",
            "task": "товаров",
            "limits": { "maxCandidates": 1 },
        });
        let Value::Object(args) = args else {
            unreachable!("test JSON object is static")
        };
        let request = parse_discover_request(&args).expect("bounded candidate request");

        let report = execute(&fake, request).expect("bounded candidate report");

        assert_eq!(
            report
                .candidates
                .iter()
                .map(|candidate| &candidate.target)
                .collect::<Vec<_>>(),
            vec![&intended]
        );
    }

    #[test]
    fn exact_candidate_outranks_more_than_one_hundred_weak_prefix_decoys() {
        let raw = b"metadata";
        let path = "DataProcessors/Candidates.xml";
        let exact = artifact("DataProcessor.Series");
        let mut records = (0..150)
            .map(|index| {
                let name = format!("SeriesDecoy{index:03}");
                MetadataFact {
                    artifact: artifact(&format!("DataProcessor.{name}")),
                    search_name: name,
                    artifact_kind: ArtifactKind::MetadataObject,
                    container: None,
                    container_kind: None,
                    relation: StructuralRelationKind::Contains,
                    location: location(path, index + 2),
                }
            })
            .collect::<Vec<_>>();
        records.push(MetadataFact {
            artifact: exact.clone(),
            search_name: "Series".to_string(),
            artifact_kind: ArtifactKind::MetadataObject,
            container: None,
            container_kind: None,
            relation: StructuralRelationKind::Contains,
            location: location(path, 1),
        });
        let batch = FactBatch {
            coverage: ProviderCoverage::new(1, 1, raw.len() as u64, records.len() as u32),
            records,
            analyzed_files: vec![contributor(path, raw)],
            contributors: vec![contributor(path, raw)],
        };
        let fake = FakePorts::complete_empty()
            .with_inventory(vec![source_file(path, raw)])
            .with_metadata(batch);
        let args = json!({
            "mode": "explore",
            "task": "Series",
            "limits": { "maxCandidates": 1 },
        });
        let Value::Object(args) = args else {
            unreachable!("test JSON object is static")
        };
        let request = parse_discover_request(&args).expect("bounded candidate request");

        let report = execute(&fake, request).expect("bounded candidate report");

        assert_eq!(report.candidates.len(), 1);
        assert_eq!(report.candidates[0].target, exact);
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
    fn global_evidence_budget_reserves_capacity_for_later_form_and_support_providers() {
        let metadata_raw = b"metadata";
        let form_raw = b"form";
        let support_raw = b"support";
        let metadata_path = "Documents/Decoys.xml";
        let form_path = "DataProcessors/Purchase/Forms/Main/Ext/Form.xml";
        let support_path = "Ext/ParentConfigurations.bin";
        let metadata_records = (0..3)
            .map(|index| MetadataFact {
                artifact: artifact(&format!("Document.Decoy{index}")),
                search_name: format!("Decoy{index}"),
                artifact_kind: ArtifactKind::MetadataObject,
                container: None,
                container_kind: None,
                relation: StructuralRelationKind::Contains,
                location: location(metadata_path, index + 1),
            })
            .collect::<Vec<_>>();
        let metadata = FactBatch {
            records: metadata_records,
            analyzed_files: vec![contributor(metadata_path, metadata_raw)],
            contributors: vec![contributor(metadata_path, metadata_raw)],
            coverage: ProviderCoverage::new(1, 1, metadata_raw.len() as u64, 3),
        };
        let forms = FactBatch {
            records: vec![FormFact {
                form: artifact("DataProcessor.Purchase.Form.Main"),
                binding: FormBinding::Data {
                    target: artifact("Document.Purchase.Attribute.Series"),
                    target_kind: ArtifactKind::Attribute,
                    data_path: "Объект.Серия".to_string(),
                },
                location: location(form_path, 1),
            }],
            analyzed_files: vec![contributor(form_path, form_raw)],
            contributors: vec![contributor(form_path, form_raw)],
            coverage: ProviderCoverage::new(1, 1, form_raw.len() as u64, 1),
        };
        let support = FactBatch {
            records: vec![SupportFact {
                artifact: artifact("DataProcessor.SupportProbe"),
                artifact_kind: ArtifactKind::MetadataObject,
                state: SupportStateKind::Editable,
                location: location(support_path, 1),
            }],
            analyzed_files: vec![contributor(support_path, support_raw)],
            contributors: vec![contributor(support_path, support_raw)],
            coverage: ProviderCoverage::new(1, 1, support_raw.len() as u64, 1),
        };
        let mut fake = FakePorts::complete_empty()
            .with_inventory(vec![
                source_file(metadata_path, metadata_raw),
                source_file(form_path, form_raw),
                source_file(support_path, support_raw),
            ])
            .with_metadata(metadata)
            .with_forms(forms);
        fake.support = ProviderOutcome::Complete(support);

        let report = execute(&fake, request_with_evidence_limit(3)).expect("fair bounded report");

        assert_eq!(report.evidence.len(), 3);
        assert_eq!(
            report
                .evidence
                .iter()
                .map(|evidence| evidence.provider)
                .collect::<BTreeSet<_>>(),
            BTreeSet::from([
                ProviderKind::MetadataCatalog,
                ProviderKind::ManagedForms,
                ProviderKind::SupportState,
            ])
        );
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
                source: artifact("Document.Purchase.Module.ObjectModule.Method.BeforeCheckSeries"),
                source_kind: ArtifactKind::Method,
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
                source: artifact("CommonModule.Series.Method.BeforeFindSeries"),
                source_kind: ArtifactKind::Method,
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
