use super::contract::{ArtifactKind, ArtifactRef, Proposal};
use super::determinism::evidence_id;
use super::evidence_graph::EvidenceGraph;
use super::model::{
    Coverage, EvidencePort, FactAnswer, FlowKind, ProposalFacts, ProposalVerdict, ProviderFact,
    SupportState, Verdict,
};
use super::ports::CollectedProviderOutcome;
use std::collections::{BTreeMap, BTreeSet};

pub(crate) struct ProposalValidation {
    pub(crate) verdicts: Vec<ProposalVerdict>,
    pub(crate) material_ports: BTreeMap<String, BTreeSet<EvidencePort>>,
}

pub(crate) struct ProposalValidator;

impl ProposalValidator {
    pub(crate) fn validate(
        proposals: &[Proposal],
        graph: &EvidenceGraph,
        providers: &[CollectedProviderOutcome],
    ) -> Result<ProposalValidation, String> {
        let mut verdicts = Vec::with_capacity(proposals.len());
        let mut material_ports = BTreeMap::new();
        for proposal in proposals {
            let owner = owner_for(&proposal.target);
            let conflicts = graph.conflicts.iter().filter(|conflict| {
                conflict.artifact == proposal.target
                    || owner
                        .as_ref()
                        .is_some_and(|owner| conflict.artifact == *owner)
            });
            let conflict_codes: BTreeSet<_> = conflicts.map(|item| item.code.clone()).collect();

            let definition = provider(providers, EvidencePort::Definition)?;
            let metadata = provider(providers, EvidencePort::MetadataCatalog)?;
            let support_provider = provider(providers, EvidencePort::SupportState)?;
            let call_graph = provider(providers, EvidencePort::CallGraph)?;
            let forms = provider(providers, EvidencePort::FormInspection)?;

            let definition_answer = existence_answer(
                definition,
                &proposal.target,
                |fact| matches!(fact, ProviderFact::DefinitionPresent { .. }),
                |fact| matches!(fact, ProviderFact::DefinitionAbsent { .. }),
            );
            let owner_answer = owner.as_ref().map_or(FactAnswer::Yes, |owner| {
                existence_answer(
                    metadata,
                    owner,
                    |fact| matches!(fact, ProviderFact::MetadataPresent { .. }),
                    |fact| matches!(fact, ProviderFact::MetadataAbsent { .. }),
                )
            });
            let exists = if !conflict_codes.is_empty() {
                FactAnswer::Unknown
            } else {
                combine_existence(definition_answer, owner_answer)
            };

            let connection_ports = graph
                .connection_ports
                .get(&proposal.target)
                .cloned()
                .unwrap_or_default();
            let runtime_reachable = if connection_ports.contains(&EvidencePort::CallGraph)
                || connection_ports.contains(&EvidencePort::FormInspection)
                || connection_ports.contains(&EvidencePort::MetadataCatalog)
            {
                FactAnswer::Yes
            } else if call_graph.coverage == Coverage::Complete
                && forms.coverage == Coverage::Complete
                && metadata.coverage == Coverage::Complete
            {
                FactAnswer::No
            } else {
                FactAnswer::Unknown
            };

            let support_states: BTreeSet<_> = support_provider
                .records
                .iter()
                .filter_map(|record| match &record.fact {
                    ProviderFact::Support { subject, state } if subject == &proposal.target => {
                        Some(*state)
                    }
                    _ => None,
                })
                .collect();
            let support = if support_states.len() == 1 {
                *support_states.iter().next().expect("one support state")
            } else {
                SupportState::Unknown
            };

            let mut coverage_gaps = BTreeSet::new();
            let mut blockers = conflict_codes;
            if exists == FactAnswer::Unknown {
                coverage_gaps.insert("existence_inconclusive".to_string());
            }
            if runtime_reachable == FactAnswer::Unknown {
                coverage_gaps.insert("runtime_reachability_inconclusive".to_string());
            }
            if support == SupportState::Unknown {
                coverage_gaps.insert("support_state_inconclusive".to_string());
            }
            if support_states.len() > 1 {
                blockers.insert("conflicting_support_facts".to_string());
            }

            let verdict = if !blockers.is_empty() {
                Verdict::Unknown
            } else if exists == FactAnswer::No || runtime_reachable == FactAnswer::No {
                Verdict::Contradicted
            } else if exists == FactAnswer::Yes
                && runtime_reachable == FactAnswer::Yes
                && support != SupportState::Unknown
            {
                Verdict::Supported
            } else {
                Verdict::Unknown
            };
            let evidence_ids = relevant_evidence_ids(providers, &proposal.target, owner.as_ref())?;
            verdicts.push(ProposalVerdict {
                proposal_id: proposal.id.clone(),
                verdict,
                facts: ProposalFacts {
                    exists,
                    runtime_reachable,
                    support,
                },
                evidence_ids,
                coverage_gaps: coverage_gaps.into_iter().collect(),
                blockers: blockers.into_iter().collect(),
            });

            let mut material =
                BTreeSet::from([EvidencePort::MetadataCatalog, EvidencePort::Definition]);
            if exists == FactAnswer::Yes {
                if runtime_reachable == FactAnswer::Yes {
                    if connection_ports.contains(&EvidencePort::CallGraph) {
                        material.insert(EvidencePort::CallGraph);
                    } else {
                        material.insert(EvidencePort::FormInspection);
                    }
                    material.insert(EvidencePort::SupportState);
                } else {
                    material.insert(EvidencePort::CallGraph);
                    material.insert(EvidencePort::FormInspection);
                }
            }
            material_ports.insert(proposal.id.clone(), material);
        }
        Ok(ProposalValidation {
            verdicts,
            material_ports,
        })
    }
}

fn provider(
    providers: &[CollectedProviderOutcome],
    port: EvidencePort,
) -> Result<&CollectedProviderOutcome, String> {
    providers
        .iter()
        .find(|provider| provider.port == port)
        .ok_or_else(|| format!("missing {} outcome", port.wire_name()))
}

fn existence_answer(
    provider: &CollectedProviderOutcome,
    target: &ArtifactRef,
    positive: impl Fn(&ProviderFact) -> bool,
    negative: impl Fn(&ProviderFact) -> bool,
) -> FactAnswer {
    let relevant: Vec<_> = provider
        .records
        .iter()
        .filter(|record| record.fact.subject() == target)
        .collect();
    let has_positive = relevant.iter().any(|record| positive(&record.fact));
    let has_negative = relevant.iter().any(|record| negative(&record.fact));
    match (has_positive, has_negative, provider.coverage) {
        (true, false, _) => FactAnswer::Yes,
        (false, true, Coverage::Complete) => FactAnswer::No,
        (false, false, Coverage::Complete) => FactAnswer::No,
        _ => FactAnswer::Unknown,
    }
}

fn combine_existence(definition: FactAnswer, owner: FactAnswer) -> FactAnswer {
    if definition == FactAnswer::No || owner == FactAnswer::No {
        FactAnswer::No
    } else if definition == FactAnswer::Yes && owner == FactAnswer::Yes {
        FactAnswer::Yes
    } else {
        FactAnswer::Unknown
    }
}

fn owner_for(target: &ArtifactRef) -> Option<ArtifactRef> {
    if target.kind != ArtifactKind::Method {
        return Some(target.clone());
    }
    let segments: Vec<_> = target.canonical_ref.split('.').collect();
    if segments.first() == Some(&"CommonModule") && segments.len() >= 3 {
        ArtifactRef::parse(
            ArtifactKind::Module,
            &format!("{}.{}", segments[0], segments[1]),
        )
        .ok()
    } else if segments.len() >= 2 {
        ArtifactRef::parse(
            ArtifactKind::MetadataObject,
            &format!("{}.{}", segments[0], segments[1]),
        )
        .ok()
    } else {
        None
    }
}

fn relevant_evidence_ids(
    providers: &[CollectedProviderOutcome],
    target: &ArtifactRef,
    owner: Option<&ArtifactRef>,
) -> Result<Vec<String>, String> {
    let mut ids = BTreeSet::new();
    for record in providers.iter().flat_map(|provider| &provider.records) {
        let relevant = record.fact.subject() == target
            || record.fact.object() == Some(target)
            || owner.is_some_and(|owner| record.fact.subject() == owner);
        if relevant {
            ids.insert(evidence_id(record).map_err(|error| error.to_string())?);
        }
    }
    Ok(ids.into_iter().collect())
}

#[allow(dead_code)]
fn _runtime_edge_kind_is_typed(kind: FlowKind) -> bool {
    matches!(
        kind,
        FlowKind::Calls | FlowKind::Handles | FlowKind::Subscribes | FlowKind::Uses
    )
}
