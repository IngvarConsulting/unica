use super::canonical_json::{operation_input_digest, CanonicalJsonError};
use super::{BranchedLifecycleToolName, DurableExecutionPolicy, OperationId, Sha256Digest};
use serde_json::Value;
use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
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
pub enum OperationInvariantError {
    MissingOwnerState { state: OperationState },
    OwnerStateNotAllowed { state: OperationState },
    MissingRecoveryDigest,
    RecoveryDigestNotAllowed { state: OperationState },
    MissingTerminalEnvelopeDigest,
    TerminalEnvelopeDigestNotAllowed { state: OperationState },
}

impl fmt::Display for OperationInvariantError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingOwnerState { state } => {
                write!(formatter, "{state:?} operations require an owner state")
            }
            Self::OwnerStateNotAllowed { state } => {
                write!(formatter, "{state:?} operations cannot have an owner state")
            }
            Self::MissingRecoveryDigest => {
                formatter.write_str("effect-unknown operations require a recovery digest")
            }
            Self::RecoveryDigestNotAllowed { state } => {
                write!(
                    formatter,
                    "{state:?} operations cannot have a recovery digest"
                )
            }
            Self::MissingTerminalEnvelopeDigest => {
                formatter.write_str("terminal operations require a terminal envelope digest")
            }
            Self::TerminalEnvelopeDigestNotAllowed { state } => {
                write!(
                    formatter,
                    "{state:?} operations cannot have a terminal envelope digest"
                )
            }
        }
    }
}

impl std::error::Error for OperationInvariantError {}

#[derive(Debug, Clone, PartialEq, Eq)]
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
    fn new(
        state: OperationState,
        owner_state: Option<OperationOwnerState>,
        terminal_envelope_digest: Option<Sha256Digest>,
        recovery_digest: Option<Sha256Digest>,
    ) -> Result<Self, OperationInvariantError> {
        match (
            state,
            owner_state,
            terminal_envelope_digest,
            recovery_digest,
        ) {
            (OperationState::Registered, None, _, _) => {
                Err(OperationInvariantError::MissingOwnerState { state })
            }
            (OperationState::Registered, Some(_), _, Some(_)) => {
                Err(OperationInvariantError::RecoveryDigestNotAllowed { state })
            }
            (OperationState::Registered, Some(_), Some(_), None) => {
                Err(OperationInvariantError::TerminalEnvelopeDigestNotAllowed { state })
            }
            (OperationState::Registered, Some(owner_state), None, None) => {
                Ok(Self::Registered { owner_state })
            }
            (OperationState::IntentWritten, None, _, _) => {
                Err(OperationInvariantError::MissingOwnerState { state })
            }
            (OperationState::IntentWritten, Some(_), _, Some(_)) => {
                Err(OperationInvariantError::RecoveryDigestNotAllowed { state })
            }
            (OperationState::IntentWritten, Some(_), Some(_), None) => {
                Err(OperationInvariantError::TerminalEnvelopeDigestNotAllowed { state })
            }
            (OperationState::IntentWritten, Some(owner_state), None, None) => {
                Ok(Self::IntentWritten { owner_state })
            }
            (OperationState::EffectUnknown, Some(_), _, _) => {
                Err(OperationInvariantError::OwnerStateNotAllowed { state })
            }
            (OperationState::EffectUnknown, None, _, None) => {
                Err(OperationInvariantError::MissingRecoveryDigest)
            }
            (OperationState::EffectUnknown, None, Some(_), Some(_)) => {
                Err(OperationInvariantError::TerminalEnvelopeDigestNotAllowed { state })
            }
            (OperationState::EffectUnknown, None, None, Some(recovery_digest)) => {
                Ok(Self::EffectUnknown { recovery_digest })
            }
            (OperationState::Terminal, Some(_), _, _) => {
                Err(OperationInvariantError::OwnerStateNotAllowed { state })
            }
            (OperationState::Terminal, None, _, Some(_)) => {
                Err(OperationInvariantError::RecoveryDigestNotAllowed { state })
            }
            (OperationState::Terminal, None, None, None) => {
                Err(OperationInvariantError::MissingTerminalEnvelopeDigest)
            }
            (OperationState::Terminal, None, Some(terminal_envelope_digest), None) => {
                Ok(Self::Terminal {
                    terminal_envelope_digest,
                })
            }
        }
    }

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
pub struct OperationReplayView {
    operation_id: OperationId,
    tool_name: BranchedLifecycleToolName,
    policy: DurableExecutionPolicy,
    canonical_input_digest: Sha256Digest,
    state: ValidatedOperationState,
}

impl OperationReplayView {
    #[allow(clippy::too_many_arguments)]
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "Task 5 validated loader is the production record-parts consumer"
        )
    )]
    pub(super) fn from_validated_record_parts(
        operation_id: OperationId,
        tool_name: BranchedLifecycleToolName,
        policy: DurableExecutionPolicy,
        canonical_input_digest: Sha256Digest,
        state: OperationState,
        owner_state: Option<OperationOwnerState>,
        terminal_envelope_digest: Option<Sha256Digest>,
        recovery_digest: Option<Sha256Digest>,
    ) -> Result<Self, OperationInvariantError> {
        let state = ValidatedOperationState::new(
            state,
            owner_state,
            terminal_envelope_digest,
            recovery_digest,
        )?;

        Ok(Self {
            operation_id,
            tool_name,
            policy,
            canonical_input_digest,
            state,
        })
    }

    pub fn operation_id(&self) -> &OperationId {
        &self.operation_id
    }

    pub fn tool_name(&self) -> BranchedLifecycleToolName {
        self.tool_name
    }

    pub fn policy(&self) -> DurableExecutionPolicy {
        self.policy
    }

    pub fn state(&self) -> OperationState {
        self.state.operation_state()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReplayDisposition {
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReplayClassificationError {
    RequestMustBeObject,
    NonInteroperableInteger,
    NonInteroperableString,
    Canonicalization,
}

impl fmt::Display for ReplayClassificationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::RequestMustBeObject => {
                formatter.write_str("operation request must be a JSON object")
            }
            Self::NonInteroperableInteger => {
                formatter.write_str("integer is outside the I-JSON interoperability range")
            }
            Self::NonInteroperableString => {
                formatter.write_str("string contains an I-JSON forbidden Unicode scalar")
            }
            Self::Canonicalization => formatter.write_str("JSON canonicalization failed"),
        }
    }
}

impl std::error::Error for ReplayClassificationError {}

impl From<CanonicalJsonError> for ReplayClassificationError {
    fn from(value: CanonicalJsonError) -> Self {
        match value {
            CanonicalJsonError::RequestMustBeObject => Self::RequestMustBeObject,
            CanonicalJsonError::NonInteroperableInteger => Self::NonInteroperableInteger,
            CanonicalJsonError::NonInteroperableString => Self::NonInteroperableString,
            CanonicalJsonError::Canonicalization(_) => Self::Canonicalization,
        }
    }
}

pub fn classify_replay(
    record: Option<&OperationReplayView>,
    incoming_tool_name: BranchedLifecycleToolName,
    incoming_policy: DurableExecutionPolicy,
    request: &Value,
) -> Result<ReplayDisposition, ReplayClassificationError> {
    let observed_input_digest =
        operation_input_digest(incoming_tool_name, incoming_policy, request)?;
    let Some(record) = record else {
        return Ok(ReplayDisposition::DispatchNew {
            canonical_input_digest: observed_input_digest,
        });
    };

    if record.tool_name != incoming_tool_name
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
    use super::{
        classify_replay, OperationOwnerState, OperationReplayView, OperationState,
        ReplayClassificationError, ReplayDisposition,
    };
    use crate::domain::branched_development::{
        canonical_json::operation_input_digest, BranchedLifecycleToolName, DurableExecutionPolicy,
        OperationId, Sha256Digest,
    };
    use serde_json::json;
    use std::str::FromStr;

    fn operation_id() -> OperationId {
        OperationId::from_str("123e4567-e89b-12d3-a456-426614174000").unwrap()
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

    #[test]
    fn no_record_returns_the_classifier_computed_digest_for_registration() {
        let request = request();
        let expected = operation_input_digest(
            BranchedLifecycleToolName::MergeApply,
            DurableExecutionPolicy::JournaledEffect,
            &request,
        )
        .unwrap();

        assert_eq!(
            classify_replay(
                None,
                BranchedLifecycleToolName::MergeApply,
                DurableExecutionPolicy::JournaledEffect,
                &request,
            ),
            Ok(ReplayDisposition::DispatchNew {
                canonical_input_digest: expected,
            })
        );
    }

    #[test]
    fn invalid_request_is_rejected_before_dispatch_without_a_record() {
        assert_eq!(
            classify_replay(
                None,
                BranchedLifecycleToolName::MergeApply,
                DurableExecutionPolicy::JournaledEffect,
                &json!([]),
            ),
            Err(ReplayClassificationError::RequestMustBeObject)
        );
    }

    #[test]
    fn non_i_json_requests_are_rejected_before_dispatch_without_a_record() {
        for (request, expected) in [
            (
                json!({"taskId": "TASK-137", "forbidden": "\u{fdd0}"}),
                ReplayClassificationError::NonInteroperableString,
            ),
            (
                json!({"taskId": "TASK-137", "tooLarge": 9_007_199_254_740_992_u64}),
                ReplayClassificationError::NonInteroperableInteger,
            ),
        ] {
            assert_eq!(
                classify_replay(
                    None,
                    BranchedLifecycleToolName::MergeApply,
                    DurableExecutionPolicy::JournaledEffect,
                    &request,
                ),
                Err(expected)
            );
        }
    }

    fn durable_record(
        state: OperationState,
        owner_state: Option<OperationOwnerState>,
        terminal_envelope_digest: Option<Sha256Digest>,
        recovery_digest: Option<Sha256Digest>,
        stored_request: &serde_json::Value,
    ) -> OperationReplayView {
        OperationReplayView::from_validated_record_parts(
            operation_id(),
            BranchedLifecycleToolName::MergeApply,
            DurableExecutionPolicy::JournaledEffect,
            operation_input_digest(
                BranchedLifecycleToolName::MergeApply,
                DurableExecutionPolicy::JournaledEffect,
                stored_request,
            )
            .unwrap(),
            state,
            owner_state,
            terminal_envelope_digest,
            recovery_digest,
        )
        .unwrap()
    }

    fn classify(
        record: Option<&OperationReplayView>,
    ) -> Result<ReplayDisposition, ReplayClassificationError> {
        let request = request();
        classify_replay(
            record,
            BranchedLifecycleToolName::MergeApply,
            DurableExecutionPolicy::JournaledEffect,
            &request,
        )
    }

    #[test]
    fn stored_replay_view_needs_only_validated_record_parts_not_the_original_request() {
        let stored_request = request();
        let record = durable_record(
            OperationState::Registered,
            Some(OperationOwnerState::Live),
            None,
            None,
            &stored_request,
        );

        assert_eq!(record.state(), OperationState::Registered);
    }

    #[test]
    fn different_incoming_tool_and_policy_mismatch_before_every_state_disposition() {
        let request = request();
        for record in [
            durable_record(
                OperationState::Registered,
                Some(OperationOwnerState::Live),
                None,
                None,
                &request,
            ),
            durable_record(
                OperationState::IntentWritten,
                Some(OperationOwnerState::Orphaned),
                None,
                None,
                &request,
            ),
            durable_record(
                OperationState::EffectUnknown,
                None,
                None,
                Some(digest('c')),
                &request,
            ),
            durable_record(
                OperationState::Terminal,
                None,
                Some(digest('d')),
                None,
                &request,
            ),
        ] {
            for (different_tool, different_policy) in [
                (
                    BranchedLifecycleToolName::MergeVerify,
                    DurableExecutionPolicy::JournaledEffect,
                ),
                (
                    BranchedLifecycleToolName::MergeApply,
                    DurableExecutionPolicy::Contained,
                ),
            ] {
                let observed =
                    operation_input_digest(different_tool, different_policy, &request).unwrap();

                assert_eq!(
                    classify_replay(Some(&record), different_tool, different_policy, &request),
                    Ok(ReplayDisposition::ReplayMismatch {
                        expected: operation_input_digest(
                            BranchedLifecycleToolName::MergeApply,
                            DurableExecutionPolicy::JournaledEffect,
                            &request,
                        )
                        .unwrap(),
                        observed,
                    })
                );
            }
        }
    }

    #[test]
    fn invalid_i_json_request_precedes_every_state_disposition() {
        let stored_request = request();
        for record in [
            durable_record(
                OperationState::Registered,
                Some(OperationOwnerState::Live),
                None,
                None,
                &stored_request,
            ),
            durable_record(
                OperationState::IntentWritten,
                Some(OperationOwnerState::Orphaned),
                None,
                None,
                &stored_request,
            ),
            durable_record(
                OperationState::EffectUnknown,
                None,
                None,
                Some(digest('c')),
                &stored_request,
            ),
            durable_record(
                OperationState::Terminal,
                None,
                Some(digest('d')),
                None,
                &stored_request,
            ),
        ] {
            assert_eq!(
                classify_replay(
                    Some(&record),
                    BranchedLifecycleToolName::MergeApply,
                    DurableExecutionPolicy::JournaledEffect,
                    &json!({"taskId": "TASK-137", "forbidden": "\u{fdd0}"}),
                ),
                Err(ReplayClassificationError::NonInteroperableString)
            );
        }
    }

    #[test]
    fn replay_binds_reordered_equivalent_input_and_every_non_top_level_field() {
        let stored_request = request();
        let record = durable_record(
            OperationState::Terminal,
            None,
            Some(digest('d')),
            None,
            &stored_request,
        );
        let reordered = json!({
            "guard": {"digest": "guarded"},
            "taskId": "TASK-137",
            "approval": {"decision": "apply", "digest": "approved"},
            "operationId": "123e4567-e89b-12d3-a456-426614174001",
        });

        assert_eq!(
            classify_replay(
                Some(&record),
                BranchedLifecycleToolName::MergeApply,
                DurableExecutionPolicy::JournaledEffect,
                &reordered,
            ),
            Ok(ReplayDisposition::ReplayTerminal {
                terminal_envelope_digest: digest('d'),
            })
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
                    BranchedLifecycleToolName::MergeApply,
                    DurableExecutionPolicy::JournaledEffect,
                    &changed,
                ),
                Ok(ReplayDisposition::ReplayMismatch { .. })
            ));
        }
    }

    fn record(
        state: OperationState,
        owner_state: Option<OperationOwnerState>,
        terminal_envelope_digest: Option<Sha256Digest>,
        recovery_digest: Option<Sha256Digest>,
    ) -> OperationReplayView {
        let request = request();
        durable_record(
            state,
            owner_state,
            terminal_envelope_digest,
            recovery_digest,
            &request,
        )
    }

    #[test]
    fn no_record_dispatches_a_new_operation() {
        let request = request();
        assert_eq!(
            classify(None),
            Ok(ReplayDisposition::DispatchNew {
                canonical_input_digest: operation_input_digest(
                    BranchedLifecycleToolName::MergeApply,
                    DurableExecutionPolicy::JournaledEffect,
                    &request,
                )
                .unwrap(),
            })
        );
    }

    #[test]
    fn input_mismatch_precedes_every_state_specific_disposition() {
        let incoming = json!({"taskId": "TASK-137", "approval": {"digest": "different"}});
        for record in [
            record(
                OperationState::Registered,
                Some(OperationOwnerState::Live),
                None,
                None,
            ),
            record(
                OperationState::IntentWritten,
                Some(OperationOwnerState::Orphaned),
                None,
                None,
            ),
            record(OperationState::EffectUnknown, None, None, Some(digest('c'))),
            record(OperationState::Terminal, None, Some(digest('d')), None),
        ] {
            assert_eq!(
                classify_replay(
                    Some(&record),
                    BranchedLifecycleToolName::MergeApply,
                    DurableExecutionPolicy::JournaledEffect,
                    &incoming,
                ),
                Ok(ReplayDisposition::ReplayMismatch {
                    expected: operation_input_digest(
                        BranchedLifecycleToolName::MergeApply,
                        DurableExecutionPolicy::JournaledEffect,
                        &request(),
                    )
                    .unwrap(),
                    observed: operation_input_digest(
                        BranchedLifecycleToolName::MergeApply,
                        DurableExecutionPolicy::JournaledEffect,
                        &incoming,
                    )
                    .unwrap(),
                })
            );
        }
    }

    #[test]
    fn matching_terminal_replays_its_terminal_envelope() {
        let record = record(OperationState::Terminal, None, Some(digest('d')), None);

        assert_eq!(
            classify(Some(&record)),
            Ok(ReplayDisposition::ReplayTerminal {
                terminal_envelope_digest: digest('d'),
            })
        );
    }

    #[test]
    fn matching_live_registered_and_intent_written_are_in_progress() {
        for record in [
            record(
                OperationState::Registered,
                Some(OperationOwnerState::Live),
                None,
                None,
            ),
            record(
                OperationState::IntentWritten,
                Some(OperationOwnerState::Live),
                None,
                None,
            ),
        ] {
            assert_eq!(classify(Some(&record)), Ok(ReplayDisposition::InProgress));
        }
    }

    #[test]
    fn matching_orphaned_states_follow_their_separate_recovery_paths() {
        let registered = record(
            OperationState::Registered,
            Some(OperationOwnerState::Orphaned),
            None,
            None,
        );
        let intent_written = record(
            OperationState::IntentWritten,
            Some(OperationOwnerState::Orphaned),
            None,
            None,
        );

        assert_eq!(
            classify(Some(&registered)),
            Ok(ReplayDisposition::ResumeRegistered)
        );
        assert_eq!(
            classify(Some(&intent_written)),
            Ok(ReplayDisposition::ObserveIntentWritten)
        );
    }

    #[test]
    fn matching_unknown_effect_requires_its_recovery_plan() {
        let record = record(OperationState::EffectUnknown, None, None, Some(digest('c')));

        assert_eq!(
            classify(Some(&record)),
            Ok(ReplayDisposition::RecoveryRequired {
                recovery_digest: digest('c'),
            })
        );
    }

    #[test]
    fn replay_view_preserves_the_public_state_projection() {
        for (record, expected_state) in [
            (
                record(
                    OperationState::Registered,
                    Some(OperationOwnerState::Live),
                    None,
                    None,
                ),
                OperationState::Registered,
            ),
            (
                record(
                    OperationState::IntentWritten,
                    Some(OperationOwnerState::Orphaned),
                    None,
                    None,
                ),
                OperationState::IntentWritten,
            ),
            (
                record(OperationState::EffectUnknown, None, None, Some(digest('c'))),
                OperationState::EffectUnknown,
            ),
            (
                record(OperationState::Terminal, None, Some(digest('d')), None),
                OperationState::Terminal,
            ),
        ] {
            assert_eq!(record.state(), expected_state);
        }
    }

    #[test]
    fn constructor_rejects_each_illegal_presence_rule() {
        for state in [OperationState::Registered, OperationState::IntentWritten] {
            assert!(record_result(state, None, None, None).is_err());
        }
        for state in [OperationState::EffectUnknown, OperationState::Terminal] {
            assert!(record_result(state, Some(OperationOwnerState::Live), None, None).is_err());
        }

        assert!(record_result(OperationState::EffectUnknown, None, None, None).is_err());
        for state in [
            OperationState::Registered,
            OperationState::IntentWritten,
            OperationState::Terminal,
        ] {
            assert!(record_result(state, owner_for(state), None, Some(digest('c'))).is_err());
        }

        assert!(record_result(OperationState::Terminal, None, None, None).is_err());
        for state in [
            OperationState::Registered,
            OperationState::IntentWritten,
            OperationState::EffectUnknown,
        ] {
            assert!(record_result(
                state,
                owner_for(state),
                Some(digest('d')),
                recovery_for(state),
            )
            .is_err());
        }
    }

    fn owner_for(state: OperationState) -> Option<OperationOwnerState> {
        match state {
            OperationState::Registered | OperationState::IntentWritten => {
                Some(OperationOwnerState::Live)
            }
            OperationState::EffectUnknown | OperationState::Terminal => None,
        }
    }

    fn recovery_for(state: OperationState) -> Option<Sha256Digest> {
        match state {
            OperationState::EffectUnknown => Some(digest('c')),
            OperationState::Registered
            | OperationState::IntentWritten
            | OperationState::Terminal => None,
        }
    }

    fn record_result(
        state: OperationState,
        owner_state: Option<OperationOwnerState>,
        terminal_envelope_digest: Option<Sha256Digest>,
        recovery_digest: Option<Sha256Digest>,
    ) -> Result<OperationReplayView, super::OperationInvariantError> {
        let request = request();
        OperationReplayView::from_validated_record_parts(
            operation_id(),
            BranchedLifecycleToolName::MergeApply,
            DurableExecutionPolicy::JournaledEffect,
            operation_input_digest(
                BranchedLifecycleToolName::MergeApply,
                DurableExecutionPolicy::JournaledEffect,
                &request,
            )
            .unwrap(),
            state,
            owner_state,
            terminal_envelope_digest,
            recovery_digest,
        )
    }
}
