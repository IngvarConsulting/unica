use super::{ExecutionPolicy, OperationId, Sha256Digest};
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
pub struct OperationReplayView<TTool> {
    operation_id: OperationId,
    tool_name: TTool,
    policy: ExecutionPolicy,
    canonical_input_digest: Sha256Digest,
    state: ValidatedOperationState,
}

impl<TTool> OperationReplayView<TTool> {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        operation_id: OperationId,
        tool_name: TTool,
        policy: ExecutionPolicy,
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

    pub fn tool_name(&self) -> &TTool {
        &self.tool_name
    }

    pub fn policy(&self) -> ExecutionPolicy {
        self.policy
    }

    pub fn state(&self) -> OperationState {
        self.state.operation_state()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReplayDisposition {
    DispatchNew,
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

pub fn classify_replay<TTool>(
    record: Option<&OperationReplayView<TTool>>,
    observed_input_digest: &Sha256Digest,
) -> ReplayDisposition {
    let Some(record) = record else {
        return ReplayDisposition::DispatchNew;
    };

    if record.canonical_input_digest != *observed_input_digest {
        return ReplayDisposition::ReplayMismatch {
            expected: record.canonical_input_digest.clone(),
            observed: observed_input_digest.clone(),
        };
    }

    match &record.state {
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
    }
}

#[cfg(test)]
mod tests {
    use super::{
        classify_replay, OperationOwnerState, OperationReplayView, OperationState,
        ReplayDisposition,
    };
    use crate::domain::branched_development::{
        BranchedLifecycleToolName, ExecutionPolicy, OperationId, Sha256Digest,
    };
    use std::str::FromStr;

    fn operation_id() -> OperationId {
        OperationId::from_str("123e4567-e89b-12d3-a456-426614174000").unwrap()
    }

    fn digest(value: char) -> Sha256Digest {
        Sha256Digest::from_str(&value.to_string().repeat(64)).unwrap()
    }

    fn record(
        state: OperationState,
        owner_state: Option<OperationOwnerState>,
        terminal_envelope_digest: Option<Sha256Digest>,
        recovery_digest: Option<Sha256Digest>,
    ) -> OperationReplayView<BranchedLifecycleToolName> {
        OperationReplayView::new(
            operation_id(),
            BranchedLifecycleToolName::BranchedStart,
            ExecutionPolicy::LocalJournaled,
            digest('a'),
            state,
            owner_state,
            terminal_envelope_digest,
            recovery_digest,
        )
        .unwrap()
    }

    #[test]
    fn no_record_dispatches_a_new_operation() {
        assert_eq!(
            classify_replay::<BranchedLifecycleToolName>(None, &digest('a')),
            ReplayDisposition::DispatchNew
        );
    }

    #[test]
    fn input_mismatch_precedes_every_state_specific_disposition() {
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
                classify_replay(Some(&record), &digest('b')),
                ReplayDisposition::ReplayMismatch {
                    expected: digest('a'),
                    observed: digest('b'),
                }
            );
        }
    }

    #[test]
    fn matching_terminal_replays_its_terminal_envelope() {
        let record = record(OperationState::Terminal, None, Some(digest('d')), None);

        assert_eq!(
            classify_replay(Some(&record), &digest('a')),
            ReplayDisposition::ReplayTerminal {
                terminal_envelope_digest: digest('d'),
            }
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
            assert_eq!(
                classify_replay(Some(&record), &digest('a')),
                ReplayDisposition::InProgress
            );
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
            classify_replay(Some(&registered), &digest('a')),
            ReplayDisposition::ResumeRegistered
        );
        assert_eq!(
            classify_replay(Some(&intent_written), &digest('a')),
            ReplayDisposition::ObserveIntentWritten
        );
    }

    #[test]
    fn matching_unknown_effect_requires_its_recovery_plan() {
        let record = record(OperationState::EffectUnknown, None, None, Some(digest('c')));

        assert_eq!(
            classify_replay(Some(&record), &digest('a')),
            ReplayDisposition::RecoveryRequired {
                recovery_digest: digest('c'),
            }
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
    ) -> Result<OperationReplayView<BranchedLifecycleToolName>, super::OperationInvariantError>
    {
        OperationReplayView::new(
            operation_id(),
            BranchedLifecycleToolName::BranchedStart,
            ExecutionPolicy::LocalJournaled,
            digest('a'),
            state,
            owner_state,
            terminal_envelope_digest,
            recovery_digest,
        )
    }
}
