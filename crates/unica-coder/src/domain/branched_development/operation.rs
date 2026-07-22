use super::canonical_json::{operation_input_digest, CanonicalJsonError};
use super::contracts::selectors::TaskOperationSelector;
use super::contracts::storage::OperationScope;
#[cfg(test)]
use super::contracts::storage::{TestOperationReplayStateRef, TestOperationReplayView};
use super::{DurableExecutionPolicy, OperationId, Sha256Digest};
use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
/// The provisional replay kernel is intentionally not public until Phase 1
/// generates the closed task-operation tool union.
///
/// ```compile_fail,E0603
/// use unica_coder::domain::branched_development::operation::{
///     classify_replay, OperationReplayView, ReplayDisposition,
/// };
///
/// fn main() {
///     let _ = (classify_replay, OperationReplayView, ReplayDisposition::InProgress);
/// }
/// ```
pub enum OperationState {
    Registered,
    IntentWritten,
    EffectUnknown,
    Terminal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OperationOwnerState {
    Live,
    Orphaned,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "Task 16 binds production operation records to the replay view"
    )
)]
enum ValidatedOperationState {
    Registered {
        owner_state: OperationOwnerState,
    },
    IntentWritten {
        owner_state: OperationOwnerState,
    },
    EffectUnknown {
        recovery_digest: Sha256Digest,
    },
    Terminal {
        terminal_envelope_digest: Sha256Digest,
    },
}

impl ValidatedOperationState {
    fn operation_state(&self) -> OperationState {
        match self {
            Self::Registered { .. } => OperationState::Registered,
            Self::IntentWritten { .. } => OperationState::IntentWritten,
            Self::EffectUnknown { .. } => OperationState::EffectUnknown,
            Self::Terminal { .. } => OperationState::Terminal,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::domain::branched_development) struct OperationReplayView {
    operation_id: OperationId,
    scope: OperationScope,
    operation: TaskOperationSelector,
    policy: DurableExecutionPolicy,
    canonical_input_digest: Sha256Digest,
    state: ValidatedOperationState,
}

impl OperationReplayView {
    /// The only Task 11 bridge from durable bytes into replay classification.
    /// The storage loader has already validated schema, scope, selector,
    /// policy, lease hashes, final-lease lineage, and terminal-envelope hash.
    #[cfg(test)]
    pub(in crate::domain::branched_development) fn from_validated_storage<T>(
        view: &TestOperationReplayView<T>,
    ) -> Self {
        let state = match view.state() {
            TestOperationReplayStateRef::Registered { owner_state, .. } => {
                ValidatedOperationState::Registered { owner_state }
            }
            TestOperationReplayStateRef::IntentWritten { owner_state, .. } => {
                ValidatedOperationState::IntentWritten { owner_state }
            }
            TestOperationReplayStateRef::EffectUnknown {
                recovery_digest, ..
            } => ValidatedOperationState::EffectUnknown {
                recovery_digest: recovery_digest.clone(),
            },
            TestOperationReplayStateRef::Terminal {
                terminal_envelope_digest,
                terminal_envelope,
                ..
            } => {
                let _validated_terminal_envelope = terminal_envelope;
                ValidatedOperationState::Terminal {
                    terminal_envelope_digest: terminal_envelope_digest.clone(),
                }
            }
        };
        Self {
            operation_id: view.operation_id().clone(),
            scope: view.scope().clone(),
            operation: view.operation().clone(),
            policy: view.policy(),
            canonical_input_digest: view.canonical_input_digest().clone(),
            state,
        }
    }

    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "Task 6 status projection consumes the operation ID"
        )
    )]
    pub(in crate::domain::branched_development) fn operation_id(&self) -> &OperationId {
        &self.operation_id
    }

    #[cfg(test)]
    pub(in crate::domain::branched_development) fn scope(&self) -> &OperationScope {
        &self.scope
    }

    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "Task 11 status projection consumes the exact operation selector"
        )
    )]
    pub(in crate::domain::branched_development) fn operation(&self) -> &TaskOperationSelector {
        &self.operation
    }

    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "Task 6 status projection consumes the durable execution policy"
        )
    )]
    pub(in crate::domain::branched_development) fn policy(&self) -> DurableExecutionPolicy {
        self.policy
    }

    #[cfg(test)]
    pub(in crate::domain::branched_development) fn canonical_input_digest(&self) -> &Sha256Digest {
        &self.canonical_input_digest
    }

    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "Task 6 status projection consumes the operation state"
        )
    )]
    pub(in crate::domain::branched_development) fn state(&self) -> OperationState {
        self.state.operation_state()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ReplayDisposition {
    DispatchNew {
        canonical_input_digest: Sha256Digest,
    },
    ReplayMismatch {
        expected: Sha256Digest,
        observed: Sha256Digest,
    },
    ReplayTerminal {
        terminal_envelope_digest: Sha256Digest,
    },
    InProgress,
    ResumeRegistered,
    ObserveIntentWritten,
    RecoveryRequired {
        recovery_digest: Sha256Digest,
    },
}

#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "Task 5 durable replay dispatch calls the provisional classifier"
    )
)]
fn classify_replay(
    record: Option<&OperationReplayView>,
    incoming_operation: &TaskOperationSelector,
    incoming_policy: DurableExecutionPolicy,
    request: &Value,
) -> Result<ReplayDisposition, CanonicalJsonError> {
    let observed_input_digest =
        operation_input_digest(incoming_operation.tool_name(), incoming_policy, request)?;
    let Some(record) = record else {
        return Ok(ReplayDisposition::DispatchNew {
            canonical_input_digest: observed_input_digest,
        });
    };

    if record.operation != *incoming_operation
        || record.policy != incoming_policy
        || record.canonical_input_digest != observed_input_digest
    {
        return Ok(ReplayDisposition::ReplayMismatch {
            expected: record.canonical_input_digest.clone(),
            observed: observed_input_digest,
        });
    }

    Ok(match &record.state {
        ValidatedOperationState::Registered {
            owner_state: OperationOwnerState::Live,
        }
        | ValidatedOperationState::IntentWritten {
            owner_state: OperationOwnerState::Live,
        } => ReplayDisposition::InProgress,
        ValidatedOperationState::Registered {
            owner_state: OperationOwnerState::Orphaned,
        } => ReplayDisposition::ResumeRegistered,
        ValidatedOperationState::IntentWritten {
            owner_state: OperationOwnerState::Orphaned,
        } => ReplayDisposition::ObserveIntentWritten,
        ValidatedOperationState::EffectUnknown { recovery_digest } => {
            ReplayDisposition::RecoveryRequired {
                recovery_digest: recovery_digest.clone(),
            }
        }
        ValidatedOperationState::Terminal {
            terminal_envelope_digest,
        } => ReplayDisposition::ReplayTerminal {
            terminal_envelope_digest: terminal_envelope_digest.clone(),
        },
    })
}

#[cfg(test)]
mod tests {
    use super::super::canonical_json::CanonicalJsonError;
    use super::{
        classify_replay, OperationOwnerState, OperationReplayView, OperationState,
        ReplayDisposition,
    };
    use crate::domain::branched_development::contracts::selectors::{
        MergeApplySelector, MergeApplySelectorVariant, MergeVerifySelector,
        MergeVerifySelectorVariant, TaskOperationSelector,
    };
    use crate::domain::branched_development::contracts::storage::tests::{
        load_validated_replay_fixture, validated_terminal_envelope_digest,
        ValidatedReplayFixtureState,
    };
    use crate::domain::branched_development::{
        canonical_json::operation_input_digest, DurableExecutionPolicy, OperationId, ProjectId,
        Sha256Digest, TaskId, UnicaId,
    };
    use serde_json::json;
    use std::str::FromStr;

    fn operation_id() -> OperationId {
        OperationId::from_str("123e4567-e89b-12d3-a456-426614174000").unwrap()
    }

    fn task_scope() -> super::OperationScope {
        super::OperationScope::Task {
            project_id: ProjectId::from_str("30000000-0000-0000-0000-000000000137").unwrap(),
            task_id: TaskId::from_str("TASK-137").unwrap(),
            instance_id: UnicaId::from_str("123e4567-e89b-42d3-a456-426614174137").unwrap(),
        }
    }

    fn digest(value: char) -> Sha256Digest {
        Sha256Digest::from_str(&value.to_string().repeat(64)).unwrap()
    }

    fn request() -> serde_json::Value {
        json!({
            "operationId": "123e4567-e89b-12d3-a456-426614174000",
            "taskId": "TASK-137",
            "approval": {"digest": "approved", "decision": "apply"},
            "guard": {"digest": "guarded"},
        })
    }

    fn merge_apply(variant: MergeApplySelectorVariant) -> TaskOperationSelector {
        TaskOperationSelector::MergeApply(MergeApplySelector::new(variant))
    }

    fn task_merge_apply() -> TaskOperationSelector {
        merge_apply(MergeApplySelectorVariant::Task)
    }

    fn merge_verify() -> TaskOperationSelector {
        TaskOperationSelector::MergeVerify(MergeVerifySelector::new(
            MergeVerifySelectorVariant::MainIntegration,
        ))
    }

    #[test]
    fn no_record_returns_the_classifier_computed_digest_for_registration() {
        let request = request();
        let expected =
            Sha256Digest::parse("9dc343ee7376e65fba54e648873fe0884fabb5214408d774027907204a0a774c")
                .unwrap();

        assert_eq!(
            classify_replay(
                None,
                &task_merge_apply(),
                DurableExecutionPolicy::JournaledEffect,
                &request,
            )
            .unwrap(),
            ReplayDisposition::DispatchNew {
                canonical_input_digest: expected,
            }
        );
    }

    #[test]
    fn invalid_request_is_rejected_before_dispatch_without_a_record() {
        assert!(matches!(
            classify_replay(
                None,
                &task_merge_apply(),
                DurableExecutionPolicy::JournaledEffect,
                &json!([]),
            ),
            Err(CanonicalJsonError::RequestMustBeObject)
        ));
    }

    #[test]
    fn non_i_json_requests_are_rejected_before_dispatch_without_a_record() {
        assert!(matches!(
            classify_replay(
                None,
                &task_merge_apply(),
                DurableExecutionPolicy::JournaledEffect,
                &json!({"taskId": "TASK-137", "forbidden": "\u{fdd0}"}),
            ),
            Err(CanonicalJsonError::NonInteroperableString)
        ));
        assert!(matches!(
            classify_replay(
                None,
                &task_merge_apply(),
                DurableExecutionPolicy::JournaledEffect,
                &json!({"taskId": "TASK-137", "tooLarge": 9_007_199_254_740_992_u64}),
            ),
            Err(CanonicalJsonError::NonInteroperableInteger)
        ));
    }

    fn durable_record(
        state: ValidatedReplayFixtureState,
        stored_request: &serde_json::Value,
    ) -> OperationReplayView {
        load_validated_replay_fixture(
            operation_id(),
            task_scope(),
            task_merge_apply(),
            DurableExecutionPolicy::JournaledEffect,
            operation_input_digest(
                task_merge_apply().tool_name(),
                DurableExecutionPolicy::JournaledEffect,
                stored_request,
            )
            .unwrap(),
            state,
        )
    }

    fn classify(
        record: Option<&OperationReplayView>,
    ) -> Result<ReplayDisposition, CanonicalJsonError> {
        let request = request();
        classify_replay(
            record,
            &task_merge_apply(),
            DurableExecutionPolicy::JournaledEffect,
            &request,
        )
    }

    #[test]
    fn stored_replay_view_needs_only_validated_record_parts_not_the_original_request() {
        let stored_request = request();
        let record = durable_record(
            ValidatedReplayFixtureState::Registered(OperationOwnerState::Live),
            &stored_request,
        );

        assert_eq!(record.state(), OperationState::Registered);
        assert_eq!(record.operation_id(), &operation_id());
        assert_eq!(record.scope(), &task_scope());
        assert_eq!(record.operation(), &task_merge_apply());
        assert_eq!(record.policy(), DurableExecutionPolicy::JournaledEffect);
    }

    #[test]
    fn different_incoming_selector_or_policy_mismatches_before_every_state_disposition() {
        let request = request();
        for record in [
            durable_record(
                ValidatedReplayFixtureState::Registered(OperationOwnerState::Live),
                &request,
            ),
            durable_record(
                ValidatedReplayFixtureState::IntentWritten(OperationOwnerState::Orphaned),
                &request,
            ),
            durable_record(
                ValidatedReplayFixtureState::EffectUnknown(digest('c')),
                &request,
            ),
            durable_record(ValidatedReplayFixtureState::Terminal, &request),
        ] {
            for (different_operation, different_policy) in [
                (merge_verify(), DurableExecutionPolicy::JournaledEffect),
                (task_merge_apply(), DurableExecutionPolicy::Contained),
                (
                    merge_apply(MergeApplySelectorVariant::Original),
                    DurableExecutionPolicy::JournaledEffect,
                ),
            ] {
                let observed = operation_input_digest(
                    different_operation.tool_name(),
                    different_policy,
                    &request,
                )
                .unwrap();

                assert_eq!(
                    classify_replay(
                        Some(&record),
                        &different_operation,
                        different_policy,
                        &request,
                    )
                    .unwrap(),
                    ReplayDisposition::ReplayMismatch {
                        expected: operation_input_digest(
                            task_merge_apply().tool_name(),
                            DurableExecutionPolicy::JournaledEffect,
                            &request,
                        )
                        .unwrap(),
                        observed,
                    }
                );
            }
        }
    }

    #[test]
    fn invalid_i_json_request_precedes_every_state_disposition() {
        let stored_request = request();
        for record in [
            durable_record(
                ValidatedReplayFixtureState::Registered(OperationOwnerState::Live),
                &stored_request,
            ),
            durable_record(
                ValidatedReplayFixtureState::IntentWritten(OperationOwnerState::Orphaned),
                &stored_request,
            ),
            durable_record(
                ValidatedReplayFixtureState::EffectUnknown(digest('c')),
                &stored_request,
            ),
            durable_record(ValidatedReplayFixtureState::Terminal, &stored_request),
        ] {
            assert!(matches!(
                classify_replay(
                    Some(&record),
                    &task_merge_apply(),
                    DurableExecutionPolicy::JournaledEffect,
                    &json!({"taskId": "TASK-137", "forbidden": "\u{fdd0}"}),
                ),
                Err(CanonicalJsonError::NonInteroperableString)
            ));
        }
    }

    #[test]
    fn replay_binds_reordered_equivalent_input_and_every_non_top_level_field() {
        let stored_request = request();
        let record = durable_record(ValidatedReplayFixtureState::Terminal, &stored_request);
        let reordered = json!({
            "guard": {"digest": "guarded"},
            "taskId": "TASK-137",
            "approval": {"decision": "apply", "digest": "approved"},
            "operationId": "123e4567-e89b-12d3-a456-426614174001",
        });

        assert_eq!(
            classify_replay(
                Some(&record),
                &task_merge_apply(),
                DurableExecutionPolicy::JournaledEffect,
                &reordered,
            )
            .unwrap(),
            ReplayDisposition::ReplayTerminal {
                terminal_envelope_digest: validated_terminal_envelope_digest(),
            }
        );

        for changed in [
            json!({
                "operationId": "123e4567-e89b-12d3-a456-426614174000",
                "taskId": "TASK-137",
                "approval": {"digest": "changed", "decision": "apply"},
                "guard": {"digest": "guarded"},
            }),
            json!({
                "operationId": "123e4567-e89b-12d3-a456-426614174000",
                "taskId": "TASK-137",
                "approval": {"digest": "approved", "decision": "apply"},
                "guard": {"operationId": "nested-change", "digest": "guarded"},
            }),
            json!({
                "operationId": "123e4567-e89b-12d3-a456-426614174000",
                "taskId": "TASK-137",
                "approval": {"digest": "approved", "decision": "deny"},
                "guard": {"digest": "guarded"},
            }),
        ] {
            assert!(matches!(
                classify_replay(
                    Some(&record),
                    &task_merge_apply(),
                    DurableExecutionPolicy::JournaledEffect,
                    &changed,
                ),
                Ok(ReplayDisposition::ReplayMismatch { .. })
            ));
        }
    }

    fn record(state: ValidatedReplayFixtureState) -> OperationReplayView {
        let request = request();
        durable_record(state, &request)
    }

    #[test]
    fn input_mismatch_precedes_every_state_specific_disposition() {
        let incoming = json!({"taskId": "TASK-137", "approval": {"digest": "different"}});
        for record in [
            record(ValidatedReplayFixtureState::Registered(
                OperationOwnerState::Live,
            )),
            record(ValidatedReplayFixtureState::IntentWritten(
                OperationOwnerState::Orphaned,
            )),
            record(ValidatedReplayFixtureState::EffectUnknown(digest('c'))),
            record(ValidatedReplayFixtureState::Terminal),
        ] {
            assert_eq!(
                classify_replay(
                    Some(&record),
                    &task_merge_apply(),
                    DurableExecutionPolicy::JournaledEffect,
                    &incoming,
                )
                .unwrap(),
                ReplayDisposition::ReplayMismatch {
                    expected: operation_input_digest(
                        task_merge_apply().tool_name(),
                        DurableExecutionPolicy::JournaledEffect,
                        &request(),
                    )
                    .unwrap(),
                    observed: operation_input_digest(
                        task_merge_apply().tool_name(),
                        DurableExecutionPolicy::JournaledEffect,
                        &incoming,
                    )
                    .unwrap(),
                }
            );
        }
    }

    #[test]
    fn matching_terminal_replays_its_terminal_envelope() {
        let record = record(ValidatedReplayFixtureState::Terminal);

        assert_eq!(
            classify(Some(&record)).unwrap(),
            ReplayDisposition::ReplayTerminal {
                terminal_envelope_digest: validated_terminal_envelope_digest(),
            }
        );
    }

    #[test]
    fn matching_live_registered_and_intent_written_are_in_progress() {
        for record in [
            record(ValidatedReplayFixtureState::Registered(
                OperationOwnerState::Live,
            )),
            record(ValidatedReplayFixtureState::IntentWritten(
                OperationOwnerState::Live,
            )),
        ] {
            assert_eq!(
                classify(Some(&record)).unwrap(),
                ReplayDisposition::InProgress
            );
        }
    }

    #[test]
    fn matching_orphaned_states_follow_their_separate_recovery_paths() {
        let registered = record(ValidatedReplayFixtureState::Registered(
            OperationOwnerState::Orphaned,
        ));
        let intent_written = record(ValidatedReplayFixtureState::IntentWritten(
            OperationOwnerState::Orphaned,
        ));

        assert_eq!(
            classify(Some(&registered)).unwrap(),
            ReplayDisposition::ResumeRegistered
        );
        assert_eq!(
            classify(Some(&intent_written)).unwrap(),
            ReplayDisposition::ObserveIntentWritten
        );
    }

    #[test]
    fn matching_unknown_effect_requires_its_recovery_plan() {
        let record = record(ValidatedReplayFixtureState::EffectUnknown(digest('c')));

        assert_eq!(
            classify(Some(&record)).unwrap(),
            ReplayDisposition::RecoveryRequired {
                recovery_digest: digest('c'),
            }
        );
    }

    #[test]
    fn replay_view_preserves_the_state_projection() {
        for (record, expected_state) in [
            (
                record(ValidatedReplayFixtureState::Registered(
                    OperationOwnerState::Live,
                )),
                OperationState::Registered,
            ),
            (
                record(ValidatedReplayFixtureState::IntentWritten(
                    OperationOwnerState::Orphaned,
                )),
                OperationState::IntentWritten,
            ),
            (
                record(ValidatedReplayFixtureState::EffectUnknown(digest('c'))),
                OperationState::EffectUnknown,
            ),
            (
                record(ValidatedReplayFixtureState::Terminal),
                OperationState::Terminal,
            ),
        ] {
            assert_eq!(record.state(), expected_state);
        }
    }
}
