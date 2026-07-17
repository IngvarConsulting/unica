use super::contract::ArtifactRef;
use super::determinism::evidence_id;
use super::model::{
    BindingDetails, CallResolution, Candidate, DefinitionShape, EvidenceLevel, EvidencePort,
    EvidenceRecord, FlowEdge, FlowKind, ProviderFact, RelatedArtifact, SupportState,
};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone)]
pub(crate) struct EvidenceConflict {
    pub(crate) port: EvidencePort,
    pub(crate) artifact: ArtifactRef,
    pub(crate) code: String,
    pub(crate) evidence_ids: Vec<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct EvidenceGraph {
    pub(crate) related_artifacts: Vec<RelatedArtifact>,
    pub(crate) flow_edges: Vec<FlowEdge>,
    pub(crate) candidates: Vec<Candidate>,
    pub(crate) conflicts: Vec<EvidenceConflict>,
    pub(crate) connection_ports: BTreeMap<ArtifactRef, BTreeSet<EvidencePort>>,
}

#[derive(Default)]
struct ArtifactAccumulator {
    evidence_ids: BTreeSet<String>,
    reason_codes: BTreeSet<String>,
    lexical: bool,
    observed: bool,
    connected: bool,
    positive_existence: bool,
    support_states: BTreeSet<SupportState>,
}

#[derive(Default)]
struct PresenceAccumulator {
    positive: bool,
    negative: bool,
    evidence_ids: BTreeSet<String>,
}

impl EvidenceGraph {
    pub(crate) fn build(records: &[EvidenceRecord]) -> Result<Self, String> {
        let mut artifacts: BTreeMap<ArtifactRef, ArtifactAccumulator> = BTreeMap::new();
        let mut edges: BTreeMap<(ArtifactRef, ArtifactRef, FlowKind), BTreeSet<String>> =
            BTreeMap::new();
        let mut presence: BTreeMap<(EvidencePort, ArtifactRef), PresenceAccumulator> =
            BTreeMap::new();
        let mut definition_shapes: BTreeMap<ArtifactRef, Vec<(DefinitionShape, BTreeSet<String>)>> =
            BTreeMap::new();
        let mut support_evidence: BTreeMap<ArtifactRef, BTreeSet<String>> = BTreeMap::new();
        let mut connection_ports: BTreeMap<ArtifactRef, BTreeSet<EvidencePort>> = BTreeMap::new();

        for record in records {
            let id = evidence_id(record).map_err(|error| error.to_string())?;
            let subject = record.fact.subject().clone();
            touch(&mut artifacts, &subject, &id);
            if let Some(object) = record.fact.object() {
                touch(&mut artifacts, object, &id);
            }

            match &record.fact {
                ProviderFact::CodeOccurrence { .. } => {
                    let artifact = artifacts.get_mut(&subject).expect("subject was inserted");
                    artifact.lexical = true;
                    artifact.reason_codes.insert("lexical_match".to_string());
                }
                ProviderFact::MetadataPresent { .. } => {
                    let artifact = artifacts.get_mut(&subject).expect("subject was inserted");
                    artifact.observed = true;
                    artifact.positive_existence = true;
                    artifact
                        .reason_codes
                        .insert("artifact_observed".to_string());
                    let fact = presence
                        .entry((record.provider.port, subject.clone()))
                        .or_default();
                    fact.positive = true;
                    fact.evidence_ids.insert(id);
                }
                ProviderFact::DefinitionPresent { definition, .. } => {
                    let artifact = artifacts.get_mut(&subject).expect("subject was inserted");
                    artifact.observed = true;
                    artifact.positive_existence = true;
                    artifact
                        .reason_codes
                        .insert("artifact_observed".to_string());
                    let fact = presence
                        .entry((record.provider.port, subject.clone()))
                        .or_default();
                    fact.positive = true;
                    fact.evidence_ids.insert(id.clone());
                    let shapes = definition_shapes.entry(subject).or_default();
                    if let Some((_, evidence_ids)) =
                        shapes.iter_mut().find(|(shape, _)| shape == definition)
                    {
                        evidence_ids.insert(id);
                    } else {
                        shapes.push((definition.clone(), BTreeSet::from([id])));
                    }
                }
                ProviderFact::MetadataAbsent { .. } | ProviderFact::DefinitionAbsent { .. } => {
                    let fact = presence
                        .entry((record.provider.port, subject.clone()))
                        .or_default();
                    fact.negative = true;
                    fact.evidence_ids.insert(id);
                }
                ProviderFact::Binding {
                    object,
                    relation,
                    details,
                    ..
                } => {
                    if matches!(details, BindingDetails::ScheduledJob { enabled: false, .. }) {
                        observe_binding(&mut artifacts, &subject, object, &id);
                    } else {
                        add_edge(
                            &mut artifacts,
                            &mut edges,
                            &mut connection_ports,
                            &subject,
                            object,
                            *relation,
                            record.provider.port,
                            id,
                        );
                    }
                }
                ProviderFact::Call {
                    object,
                    resolution: CallResolution::Resolved,
                    ..
                } => {
                    add_edge(
                        &mut artifacts,
                        &mut edges,
                        &mut connection_ports,
                        &subject,
                        object,
                        FlowKind::Calls,
                        record.provider.port,
                        id,
                    );
                }
                ProviderFact::Call { .. } => {}
                ProviderFact::PlatformCallback { object, .. } => {
                    add_edge(
                        &mut artifacts,
                        &mut edges,
                        &mut connection_ports,
                        &subject,
                        object,
                        FlowKind::Handles,
                        record.provider.port,
                        id,
                    );
                }
                ProviderFact::Support { state, .. } => {
                    let artifact = artifacts.get_mut(&subject).expect("subject was inserted");
                    artifact.observed = true;
                    artifact.support_states.insert(*state);
                    artifact
                        .reason_codes
                        .insert("support_state_observed".to_string());
                    support_evidence.entry(subject).or_default().insert(id);
                }
            }
        }

        let mut conflicts = Vec::new();
        for ((port, artifact), facts) in presence {
            if facts.positive && facts.negative {
                conflicts.push(EvidenceConflict {
                    port,
                    artifact,
                    code: "conflicting_existence_facts".to_string(),
                    evidence_ids: facts.evidence_ids.into_iter().collect(),
                });
            }
        }
        for (artifact, shapes) in definition_shapes {
            if shapes.len() > 1 {
                conflicts.push(EvidenceConflict {
                    port: EvidencePort::Definition,
                    artifact,
                    code: "conflicting_definition_shapes".to_string(),
                    evidence_ids: shapes
                        .into_iter()
                        .flat_map(|(_, evidence_ids)| evidence_ids)
                        .collect(),
                });
            }
        }
        for (artifact, accumulator) in &artifacts {
            if accumulator.support_states.len() > 1 {
                conflicts.push(EvidenceConflict {
                    port: EvidencePort::SupportState,
                    artifact: artifact.clone(),
                    code: "conflicting_support_facts".to_string(),
                    evidence_ids: support_evidence
                        .get(artifact)
                        .cloned()
                        .unwrap_or_default()
                        .into_iter()
                        .collect(),
                });
            }
        }

        let conflict_by_artifact: BTreeMap<_, BTreeSet<_>> =
            conflicts
                .iter()
                .fold(BTreeMap::new(), |mut by_artifact, conflict| {
                    by_artifact
                        .entry(conflict.artifact.clone())
                        .or_insert_with(BTreeSet::new)
                        .insert(conflict.code.clone());
                    by_artifact
                });

        let mut candidates = Vec::new();
        for (target, artifact) in &mut artifacts {
            if !artifact.positive_existence || !has_runtime_connection(target, &edges) {
                continue;
            }
            let support_state = if artifact.support_states.len() == 1 {
                *artifact
                    .support_states
                    .iter()
                    .next()
                    .expect("one support state")
            } else {
                SupportState::Unknown
            };
            let mut blockers = conflict_by_artifact
                .get(target)
                .cloned()
                .unwrap_or_default();
            let evidence_level = if support_state == SupportState::Unknown {
                blockers.insert("support_state_unknown".to_string());
                EvidenceLevel::Connected
            } else if !blockers.is_empty() {
                EvidenceLevel::Connected
            } else {
                artifact
                    .reason_codes
                    .insert("actionable_extension_point".to_string());
                EvidenceLevel::Actionable
            };
            candidates.push(Candidate {
                target: target.clone(),
                evidence_level,
                support_state,
                reason_codes: artifact.reason_codes.iter().cloned().collect(),
                evidence_ids: artifact.evidence_ids.iter().cloned().collect(),
                blockers: blockers.into_iter().collect(),
            });
        }

        let related_artifacts = artifacts
            .into_iter()
            .map(|(artifact, facts)| {
                let evidence_level = if candidates.iter().any(|candidate| {
                    candidate.target == artifact
                        && candidate.evidence_level == EvidenceLevel::Actionable
                }) {
                    EvidenceLevel::Actionable
                } else if facts.connected {
                    EvidenceLevel::Connected
                } else if facts.observed {
                    EvidenceLevel::Observed
                } else {
                    EvidenceLevel::Lexical
                };
                RelatedArtifact {
                    artifact,
                    evidence_level,
                    reason_codes: facts.reason_codes.into_iter().collect(),
                    evidence_ids: facts.evidence_ids.into_iter().collect(),
                }
            })
            .collect();
        let flow_edges = edges
            .into_iter()
            .map(|((from, to, kind), evidence_ids)| FlowEdge {
                from,
                to,
                kind,
                evidence_ids: evidence_ids.into_iter().collect(),
            })
            .collect();

        Ok(Self {
            related_artifacts,
            flow_edges,
            candidates,
            conflicts,
            connection_ports,
        })
    }
}

fn touch(
    artifacts: &mut BTreeMap<ArtifactRef, ArtifactAccumulator>,
    artifact: &ArtifactRef,
    evidence_id: &str,
) {
    artifacts
        .entry(artifact.clone())
        .or_default()
        .evidence_ids
        .insert(evidence_id.to_string());
}

fn observe_binding(
    artifacts: &mut BTreeMap<ArtifactRef, ArtifactAccumulator>,
    from: &ArtifactRef,
    to: &ArtifactRef,
    evidence_id: &str,
) {
    for artifact in [from, to] {
        let facts = artifacts
            .get_mut(artifact)
            .expect("binding endpoint was inserted");
        facts.observed = true;
        facts.reason_codes.insert("binding_observed".to_string());
        facts.evidence_ids.insert(evidence_id.to_string());
    }
}

#[allow(clippy::too_many_arguments)]
fn add_edge(
    artifacts: &mut BTreeMap<ArtifactRef, ArtifactAccumulator>,
    edges: &mut BTreeMap<(ArtifactRef, ArtifactRef, FlowKind), BTreeSet<String>>,
    connection_ports: &mut BTreeMap<ArtifactRef, BTreeSet<EvidencePort>>,
    from: &ArtifactRef,
    to: &ArtifactRef,
    kind: FlowKind,
    port: EvidencePort,
    evidence_id: String,
) {
    edges
        .entry((from.clone(), to.clone(), kind))
        .or_default()
        .insert(evidence_id.clone());
    for artifact in [from, to] {
        let facts = artifacts
            .get_mut(artifact)
            .expect("edge endpoint was inserted");
        facts.connected = true;
        facts.reason_codes.insert("runtime_connected".to_string());
        facts.evidence_ids.insert(evidence_id.clone());
        connection_ports
            .entry(artifact.clone())
            .or_default()
            .insert(port);
    }
}

fn has_runtime_connection(
    target: &ArtifactRef,
    edges: &BTreeMap<(ArtifactRef, ArtifactRef, FlowKind), BTreeSet<String>>,
) -> bool {
    edges.keys().any(|(from, to, kind)| {
        (from == target || to == target)
            && matches!(
                kind,
                FlowKind::Calls | FlowKind::Handles | FlowKind::Subscribes | FlowKind::Uses
            )
    })
}
