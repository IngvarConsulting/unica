use super::scalars::NormalizedUtcInstant;
use super::schema::one_of_schema;
use super::selectors::TaskOperationSelector;
use super::status::OperationLease;
#[cfg(test)]
use crate::domain::branched_development::OperationOwnerState;
use crate::domain::branched_development::{
    DurableExecutionPolicy, OperationId, ProjectId, Sha256Digest, TaskId, UnicaId,
};
use schemars::{JsonSchema, Schema, SchemaGenerator};
use serde::{Deserialize, Serialize};
use std::borrow::Cow;

macro_rules! operation_state_literal {
    ($name:ident, $wire:literal) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
        enum $name {
            #[serde(rename = $wire)]
            Value,
        }
    };
}

operation_state_literal!(RegisteredOperationRecordState, "registered");
operation_state_literal!(IntentWrittenOperationRecordState, "intentWritten");
operation_state_literal!(EffectUnknownOperationRecordState, "effectUnknown");
operation_state_literal!(TerminalOperationRecordState, "terminal");

/// The complete authoritative container identity for one durable operation.
/// It deliberately stores neither a path nor the caller's spelling of `cwd`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "scopeKind", rename_all = "camelCase", deny_unknown_fields)]
pub(crate) enum OperationScope {
    #[serde(rename = "startAttempt")]
    StartAttempt {
        #[serde(rename = "workspaceIdentityDigest")]
        workspace_identity_digest: Sha256Digest,
        #[serde(rename = "taskId")]
        task_id: TaskId,
    },
    #[serde(rename = "task")]
    Task {
        #[serde(rename = "projectId")]
        project_id: ProjectId,
        #[serde(rename = "taskId")]
        task_id: TaskId,
        #[serde(rename = "instanceId")]
        instance_id: UnicaId,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RegisteredOperationRecord {
    operation_id: OperationId,
    scope: OperationScope,
    operation: TaskOperationSelector,
    policy: DurableExecutionPolicy,
    canonical_input_digest: Sha256Digest,
    registered_at: NormalizedUtcInstant,
    operation_lease: OperationLease,
    state: RegisteredOperationRecordState,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct IntentWrittenOperationRecord {
    operation_id: OperationId,
    scope: OperationScope,
    operation: TaskOperationSelector,
    policy: DurableExecutionPolicy,
    canonical_input_digest: Sha256Digest,
    registered_at: NormalizedUtcInstant,
    operation_lease: OperationLease,
    state: IntentWrittenOperationRecordState,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct EffectUnknownOperationRecord {
    operation_id: OperationId,
    scope: OperationScope,
    operation: TaskOperationSelector,
    policy: DurableExecutionPolicy,
    canonical_input_digest: Sha256Digest,
    registered_at: NormalizedUtcInstant,
    // Audit reference to the removed final lease. The generic typed loader
    // compares it with the authoritative locator because this row no longer
    // contains the lease preimage itself.
    last_operation_lease_digest: Sha256Digest,
    state: EffectUnknownOperationRecordState,
    recovery_digest: Sha256Digest,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct TerminalOperationRecord<TerminalEnvelope> {
    operation_id: OperationId,
    scope: OperationScope,
    operation: TaskOperationSelector,
    policy: DurableExecutionPolicy,
    canonical_input_digest: Sha256Digest,
    registered_at: NormalizedUtcInstant,
    // See `EffectUnknownOperationRecord`: the authoritative locator supplies
    // the final digest after the current lease has left the record.
    last_operation_lease_digest: Sha256Digest,
    state: TerminalOperationRecordState,
    terminal_envelope_digest: Sha256Digest,
    terminal_envelope: TerminalEnvelope,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
enum OperationRecordKind<TerminalEnvelope> {
    Registered(RegisteredOperationRecord),
    IntentWritten(IntentWrittenOperationRecord),
    EffectUnknown(EffectUnknownOperationRecord),
    Terminal(TerminalOperationRecord<TerminalEnvelope>),
}

/// Closed generic storage framing. It has no production terminal alias or
/// constructor until the real terminal union is bound in Task 16.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub(crate) struct OperationRecord<TerminalEnvelope>(OperationRecordKind<TerminalEnvelope>);

impl<TerminalEnvelope> JsonSchema for OperationRecord<TerminalEnvelope>
where
    TerminalEnvelope: JsonSchema,
{
    fn schema_name() -> Cow<'static, str> {
        format!(
            "OperationRecordOf{}",
            TerminalEnvelope::schema_name()
                .replace(|character: char| !character.is_ascii_alphanumeric(), "_")
        )
        .into()
    }

    fn schema_id() -> Cow<'static, str> {
        format!("OperationRecord<{}>", TerminalEnvelope::schema_id()).into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        one_of_schema(vec![
            generator.subschema_for::<RegisteredOperationRecord>(),
            generator.subschema_for::<IntentWrittenOperationRecord>(),
            generator.subschema_for::<EffectUnknownOperationRecord>(),
            generator.subschema_for::<TerminalOperationRecord<TerminalEnvelope>>(),
        ])
    }
}

fn scope_accepts_selector(scope: &OperationScope, operation: &TaskOperationSelector) -> bool {
    matches!(
        (scope, operation),
        (
            OperationScope::StartAttempt { .. },
            TaskOperationSelector::BranchedStart(_)
        )
    ) || matches!(scope, OperationScope::Task { .. })
        && !matches!(operation, TaskOperationSelector::BranchedStart(_))
}

#[cfg(test)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StorageContractError {
    CandidateRejected,
    InvalidShape,
    CrossScope,
    ScopeSelectorMismatch,
    OperationBindingMismatch,
    InvalidOperationLease,
    OperationLeaseDigestMismatch,
    LastOperationLeaseDigestMismatch,
    InvalidTerminalEnvelopeDigest,
}

#[cfg(test)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TestExpectedOperationState(TestExpectedOperationStateKind);

#[cfg(test)]
#[derive(Debug, Clone, PartialEq, Eq)]
enum TestExpectedOperationStateKind {
    Registered {
        operation_lease_digest: Sha256Digest,
        owner_state: OperationOwnerState,
    },
    IntentWritten {
        operation_lease_digest: Sha256Digest,
        owner_state: OperationOwnerState,
    },
    EffectUnknown {
        last_operation_lease_digest: Sha256Digest,
    },
    Terminal {
        last_operation_lease_digest: Sha256Digest,
    },
}

#[cfg(test)]
impl TestExpectedOperationState {
    pub(crate) fn registered(
        operation_lease_digest: Sha256Digest,
        owner_state: OperationOwnerState,
    ) -> Self {
        Self(TestExpectedOperationStateKind::Registered {
            operation_lease_digest,
            owner_state,
        })
    }

    pub(crate) fn intent_written(
        operation_lease_digest: Sha256Digest,
        owner_state: OperationOwnerState,
    ) -> Self {
        Self(TestExpectedOperationStateKind::IntentWritten {
            operation_lease_digest,
            owner_state,
        })
    }

    pub(crate) fn effect_unknown(last_operation_lease_digest: Sha256Digest) -> Self {
        Self(TestExpectedOperationStateKind::EffectUnknown {
            last_operation_lease_digest,
        })
    }

    pub(crate) fn terminal(last_operation_lease_digest: Sha256Digest) -> Self {
        Self(TestExpectedOperationStateKind::Terminal {
            last_operation_lease_digest,
        })
    }
}

#[cfg(test)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TestAuthoritativeOperationLocator(TestAuthoritativeOperationLocatorKind);

#[cfg(test)]
#[derive(Debug, Clone, PartialEq, Eq)]
enum TestAuthoritativeOperationLocatorKind {
    StartAttemptStorageKey {
        workspace_identity_digest: Sha256Digest,
        task_id: TaskId,
        operation_id: OperationId,
        operation: TaskOperationSelector,
        policy: DurableExecutionPolicy,
        state: TestExpectedOperationState,
    },
    AuthoritativeParentTask {
        project_id: ProjectId,
        task_id: TaskId,
        instance_id: UnicaId,
        operation_id: OperationId,
        operation: TaskOperationSelector,
        policy: DurableExecutionPolicy,
        state: TestExpectedOperationState,
    },
}

#[cfg(test)]
impl TestAuthoritativeOperationLocator {
    pub(crate) fn start_attempt_storage_key(
        workspace_identity_digest: Sha256Digest,
        task_id: TaskId,
        operation_id: OperationId,
        operation: TaskOperationSelector,
        policy: DurableExecutionPolicy,
        state: TestExpectedOperationState,
    ) -> Self {
        Self(
            TestAuthoritativeOperationLocatorKind::StartAttemptStorageKey {
                workspace_identity_digest,
                task_id,
                operation_id,
                operation,
                policy,
                state,
            },
        )
    }

    pub(crate) fn authoritative_parent_task(
        project_id: ProjectId,
        task_id: TaskId,
        instance_id: UnicaId,
        operation_id: OperationId,
        operation: TaskOperationSelector,
        policy: DurableExecutionPolicy,
        state: TestExpectedOperationState,
    ) -> Self {
        Self(
            TestAuthoritativeOperationLocatorKind::AuthoritativeParentTask {
                project_id,
                task_id,
                instance_id,
                operation_id,
                operation,
                policy,
                state,
            },
        )
    }

    fn matches_container(&self, operation_id: &OperationId, scope: &OperationScope) -> bool {
        match (&self.0, scope) {
            (
                TestAuthoritativeOperationLocatorKind::StartAttemptStorageKey {
                    workspace_identity_digest: expected_workspace,
                    task_id: expected_task,
                    operation_id: expected_operation,
                    ..
                },
                OperationScope::StartAttempt {
                    workspace_identity_digest,
                    task_id,
                },
            ) => {
                operation_id == expected_operation
                    && workspace_identity_digest == expected_workspace
                    && task_id == expected_task
            }
            (
                TestAuthoritativeOperationLocatorKind::AuthoritativeParentTask {
                    project_id: expected_project,
                    task_id: expected_task,
                    instance_id: expected_instance,
                    operation_id: expected_operation,
                    ..
                },
                OperationScope::Task {
                    project_id,
                    task_id,
                    instance_id,
                },
            ) => {
                operation_id == expected_operation
                    && project_id == expected_project
                    && task_id == expected_task
                    && instance_id == expected_instance
            }
            _ => false,
        }
    }

    fn operation(&self) -> &TaskOperationSelector {
        match &self.0 {
            TestAuthoritativeOperationLocatorKind::StartAttemptStorageKey { operation, .. }
            | TestAuthoritativeOperationLocatorKind::AuthoritativeParentTask {
                operation, ..
            } => operation,
        }
    }

    fn policy(&self) -> DurableExecutionPolicy {
        match &self.0 {
            TestAuthoritativeOperationLocatorKind::StartAttemptStorageKey { policy, .. }
            | TestAuthoritativeOperationLocatorKind::AuthoritativeParentTask { policy, .. } => {
                *policy
            }
        }
    }

    fn state(&self) -> &TestExpectedOperationState {
        match &self.0 {
            TestAuthoritativeOperationLocatorKind::StartAttemptStorageKey { state, .. }
            | TestAuthoritativeOperationLocatorKind::AuthoritativeParentTask { state, .. } => state,
        }
    }
}

#[cfg(test)]
#[derive(Debug, Clone, PartialEq, Eq)]
enum TestOperationReplayState<TerminalEnvelope> {
    Registered {
        operation_lease: OperationLease,
        owner_state: OperationOwnerState,
    },
    IntentWritten {
        operation_lease: OperationLease,
        owner_state: OperationOwnerState,
    },
    EffectUnknown {
        last_operation_lease_digest: Sha256Digest,
        recovery_digest: Sha256Digest,
    },
    Terminal {
        last_operation_lease_digest: Sha256Digest,
        terminal_envelope_digest: Sha256Digest,
        terminal_envelope: TerminalEnvelope,
    },
}

#[cfg(test)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TestOperationReplayStateRef<'a, TerminalEnvelope> {
    Registered {
        operation_lease: &'a OperationLease,
        owner_state: OperationOwnerState,
    },
    IntentWritten {
        operation_lease: &'a OperationLease,
        owner_state: OperationOwnerState,
    },
    EffectUnknown {
        last_operation_lease_digest: &'a Sha256Digest,
        recovery_digest: &'a Sha256Digest,
    },
    Terminal {
        last_operation_lease_digest: &'a Sha256Digest,
        terminal_envelope_digest: &'a Sha256Digest,
        terminal_envelope: &'a TerminalEnvelope,
    },
}

/// Test-only typed replay projection. Production replay stays unavailable until
/// Task 16 binds the real mutating terminal envelope.
#[cfg(test)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TestOperationReplayView<TerminalEnvelope> {
    operation_id: OperationId,
    scope: OperationScope,
    operation: TaskOperationSelector,
    policy: DurableExecutionPolicy,
    canonical_input_digest: Sha256Digest,
    registered_at: NormalizedUtcInstant,
    state: TestOperationReplayState<TerminalEnvelope>,
}

#[cfg(test)]
impl<TerminalEnvelope> TestOperationReplayView<TerminalEnvelope> {
    pub(crate) fn operation_id(&self) -> &OperationId {
        &self.operation_id
    }

    pub(crate) fn scope(&self) -> &OperationScope {
        &self.scope
    }

    pub(crate) fn operation(&self) -> &TaskOperationSelector {
        &self.operation
    }

    pub(crate) fn policy(&self) -> DurableExecutionPolicy {
        self.policy
    }

    pub(crate) fn canonical_input_digest(&self) -> &Sha256Digest {
        &self.canonical_input_digest
    }

    pub(crate) fn registered_at(&self) -> &NormalizedUtcInstant {
        &self.registered_at
    }

    pub(crate) fn state(&self) -> TestOperationReplayStateRef<'_, TerminalEnvelope> {
        match &self.state {
            TestOperationReplayState::Registered {
                operation_lease,
                owner_state,
            } => TestOperationReplayStateRef::Registered {
                operation_lease,
                owner_state: *owner_state,
            },
            TestOperationReplayState::IntentWritten {
                operation_lease,
                owner_state,
            } => TestOperationReplayStateRef::IntentWritten {
                operation_lease,
                owner_state: *owner_state,
            },
            TestOperationReplayState::EffectUnknown {
                last_operation_lease_digest,
                recovery_digest,
            } => TestOperationReplayStateRef::EffectUnknown {
                last_operation_lease_digest,
                recovery_digest,
            },
            TestOperationReplayState::Terminal {
                last_operation_lease_digest,
                terminal_envelope_digest,
                terminal_envelope,
            } => TestOperationReplayStateRef::Terminal {
                last_operation_lease_digest,
                terminal_envelope_digest,
                terminal_envelope,
            },
        }
    }
}

#[cfg(test)]
fn decode_opaque_candidate(
    candidate: &crate::domain::branched_development::operation_preflight::OpaqueOperationRecordCandidate,
) -> Result<serde_json::Value, StorageContractError> {
    let value = crate::domain::i_json::from_slice(candidate.source_bytes())
        .map_err(|_| StorageContractError::CandidateRejected)?;
    let object = value
        .as_object()
        .ok_or(StorageContractError::CandidateRejected)?;
    if matches!(object.get("policy"), Some(serde_json::Value::String(policy)) if policy == "readOnly")
    {
        return Err(StorageContractError::CandidateRejected);
    }
    Ok(value)
}

#[cfg(test)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
enum UncheckedOperationState {
    Registered,
    IntentWritten,
    EffectUnknown,
    Terminal,
}

#[cfg(test)]
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct UncheckedOperationRecord<TerminalEnvelope> {
    operation_id: OperationId,
    scope: OperationScope,
    operation: TaskOperationSelector,
    policy: DurableExecutionPolicy,
    canonical_input_digest: Sha256Digest,
    registered_at: NormalizedUtcInstant,
    operation_lease: Option<serde_json::Value>,
    last_operation_lease_digest: Option<Sha256Digest>,
    state: UncheckedOperationState,
    terminal_envelope_digest: Option<Sha256Digest>,
    terminal_envelope: Option<TerminalEnvelope>,
    recovery_digest: Option<Sha256Digest>,
}

#[cfg(test)]
fn operation_record_schema_accepts<TerminalEnvelope>(value: &serde_json::Value) -> bool
where
    TerminalEnvelope: JsonSchema,
{
    use super::schema::{
        is_i_json_lf_text, is_i_json_single_line_text, is_normalized_utc_instant,
        I_JSON_LF_TEXT_FORMAT, I_JSON_SINGLE_LINE_TEXT_FORMAT, NORMALIZED_UTC_INSTANT_FORMAT,
    };

    let schema = serde_json::to_value(schemars::schema_for!(OperationRecord<TerminalEnvelope>))
        .expect("operation-record schema must serialize");
    jsonschema::options()
        .with_draft(jsonschema::Draft::Draft202012)
        .with_format(I_JSON_SINGLE_LINE_TEXT_FORMAT, is_i_json_single_line_text)
        .with_format(I_JSON_LF_TEXT_FORMAT, is_i_json_lf_text)
        .with_format(NORMALIZED_UTC_INSTANT_FORMAT, is_normalized_utc_instant)
        .should_validate_formats(true)
        .should_ignore_unknown_formats(false)
        .build(&schema)
        .expect("operation-record schema must compile")
        .is_valid(value)
}

/// Validates the preflight-approved opaque candidate's shape, authoritative
/// container/producer/state binding, lease hashes, and generic terminal digest
/// before exposing any replay fields.
#[cfg(test)]
pub(in crate::domain::branched_development) fn load_test_operation_record<TerminalEnvelope>(
    candidate: &crate::domain::branched_development::operation_preflight::OpaqueOperationRecordCandidate,
    expected_locator: &TestAuthoritativeOperationLocator,
) -> Result<TestOperationReplayView<TerminalEnvelope>, StorageContractError>
where
    TerminalEnvelope: for<'de> Deserialize<'de>
        + Serialize
        + JsonSchema
        + crate::domain::branched_development::canonical_json::ContractDigestRecord,
{
    use crate::domain::branched_development::canonical_json::canonical_contract_digest;

    let value = decode_opaque_candidate(candidate)?;
    if !operation_record_schema_accepts::<TerminalEnvelope>(&value) {
        return Err(StorageContractError::InvalidShape);
    }
    let unchecked: UncheckedOperationRecord<TerminalEnvelope> =
        serde_json::from_value(value.clone()).map_err(|_| StorageContractError::InvalidShape)?;

    if !expected_locator.matches_container(&unchecked.operation_id, &unchecked.scope) {
        return Err(StorageContractError::CrossScope);
    }
    if !scope_accepts_selector(&unchecked.scope, &unchecked.operation) {
        return Err(StorageContractError::ScopeSelectorMismatch);
    }
    if &unchecked.operation != expected_locator.operation()
        || unchecked.policy != expected_locator.policy()
    {
        return Err(StorageContractError::OperationBindingMismatch);
    }

    let state = match (unchecked.state, &expected_locator.state().0) {
        (
            UncheckedOperationState::Registered,
            TestExpectedOperationStateKind::Registered {
                operation_lease_digest: expected_lease_digest,
                owner_state,
            },
        ) => {
            let raw_lease = unchecked
                .operation_lease
                .ok_or(StorageContractError::InvalidShape)?;
            if unchecked.last_operation_lease_digest.is_some()
                || unchecked.terminal_envelope_digest.is_some()
                || unchecked.terminal_envelope.is_some()
                || unchecked.recovery_digest.is_some()
            {
                return Err(StorageContractError::InvalidShape);
            }
            let operation_lease = OperationLease::load_test_json(&raw_lease)
                .map_err(|_| StorageContractError::InvalidOperationLease)?;
            if operation_lease.lease_digest() != expected_lease_digest {
                return Err(StorageContractError::OperationLeaseDigestMismatch);
            }
            TestOperationReplayState::Registered {
                operation_lease,
                owner_state: *owner_state,
            }
        }
        (
            UncheckedOperationState::IntentWritten,
            TestExpectedOperationStateKind::IntentWritten {
                operation_lease_digest: expected_lease_digest,
                owner_state,
            },
        ) => {
            let raw_lease = unchecked
                .operation_lease
                .ok_or(StorageContractError::InvalidShape)?;
            if unchecked.last_operation_lease_digest.is_some()
                || unchecked.terminal_envelope_digest.is_some()
                || unchecked.terminal_envelope.is_some()
                || unchecked.recovery_digest.is_some()
            {
                return Err(StorageContractError::InvalidShape);
            }
            let operation_lease = OperationLease::load_test_json(&raw_lease)
                .map_err(|_| StorageContractError::InvalidOperationLease)?;
            if operation_lease.lease_digest() != expected_lease_digest {
                return Err(StorageContractError::OperationLeaseDigestMismatch);
            }
            TestOperationReplayState::IntentWritten {
                operation_lease,
                owner_state: *owner_state,
            }
        }
        (
            UncheckedOperationState::EffectUnknown,
            TestExpectedOperationStateKind::EffectUnknown {
                last_operation_lease_digest: expected_last_lease_digest,
            },
        ) => {
            if unchecked.operation_lease.is_some()
                || unchecked.terminal_envelope_digest.is_some()
                || unchecked.terminal_envelope.is_some()
            {
                return Err(StorageContractError::InvalidShape);
            }
            let last_operation_lease_digest = unchecked
                .last_operation_lease_digest
                .ok_or(StorageContractError::InvalidShape)?;
            if &last_operation_lease_digest != expected_last_lease_digest {
                return Err(StorageContractError::LastOperationLeaseDigestMismatch);
            }
            TestOperationReplayState::EffectUnknown {
                last_operation_lease_digest,
                recovery_digest: unchecked
                    .recovery_digest
                    .ok_or(StorageContractError::InvalidShape)?,
            }
        }
        (
            UncheckedOperationState::Terminal,
            TestExpectedOperationStateKind::Terminal {
                last_operation_lease_digest: expected_last_lease_digest,
            },
        ) => {
            if unchecked.operation_lease.is_some() || unchecked.recovery_digest.is_some() {
                return Err(StorageContractError::InvalidShape);
            }
            let last_operation_lease_digest = unchecked
                .last_operation_lease_digest
                .ok_or(StorageContractError::InvalidShape)?;
            if &last_operation_lease_digest != expected_last_lease_digest {
                return Err(StorageContractError::LastOperationLeaseDigestMismatch);
            }
            let terminal_envelope = unchecked
                .terminal_envelope
                .ok_or(StorageContractError::InvalidShape)?;
            let terminal_envelope_digest = unchecked
                .terminal_envelope_digest
                .ok_or(StorageContractError::InvalidShape)?;
            let actual_digest = canonical_contract_digest(&terminal_envelope, None)
                .map_err(|_| StorageContractError::InvalidTerminalEnvelopeDigest)?;
            if terminal_envelope_digest != actual_digest {
                return Err(StorageContractError::InvalidTerminalEnvelopeDigest);
            }
            TestOperationReplayState::Terminal {
                last_operation_lease_digest,
                terminal_envelope_digest,
                terminal_envelope,
            }
        }
        _ => return Err(StorageContractError::OperationBindingMismatch),
    };

    Ok(TestOperationReplayView {
        operation_id: unchecked.operation_id,
        scope: unchecked.scope,
        operation: unchecked.operation,
        policy: unchecked.policy,
        canonical_input_digest: unchecked.canonical_input_digest,
        registered_at: unchecked.registered_at,
        state,
    })
}

#[cfg(test)]
pub(in crate::domain::branched_development) mod tests {
    use super::super::schema::audit_json_schema;
    use super::super::selectors::{
        BranchedCleanupSelector, BranchedCleanupSelectorVariant, BranchedStartSelector,
        BranchedStatusSelector,
    };
    use super::super::status::OperationLeaseAuthority;
    use super::*;
    use crate::domain::branched_development::canonical_json::{
        canonical_contract_digest, contract_digest_record_sealed, ContractDigestRecord,
    };
    use crate::domain::branched_development::contracts::scalars::PositiveGeneration;
    use crate::domain::branched_development::operation::{OperationReplayView, OperationState};
    use crate::domain::branched_development::operation_preflight::{
        preflight, OpaqueOperationRecordCandidate,
    };
    use schemars::schema_for;
    use serde_json::{json, Map, Value};
    use std::str::FromStr;

    const OPERATION_ID: &str = "10000000-0000-0000-0000-000000000001";
    const OTHER_OPERATION_ID: &str = "10000000-0000-0000-0000-000000000002";
    const OWNER_ID: &str = "20000000-0000-0000-0000-000000000001";
    const OTHER_OWNER_ID: &str = "20000000-0000-0000-0000-000000000002";
    const PROJECT_ID: &str = "30000000-0000-0000-0000-000000000001";
    const OTHER_PROJECT_ID: &str = "30000000-0000-0000-0000-000000000002";
    const INSTANCE_ID: &str = "40000000-0000-0000-0000-000000000001";
    const OTHER_INSTANCE_ID: &str = "40000000-0000-0000-0000-000000000002";
    const TASK_ID: &str = "task-storage-1";
    const OTHER_TASK_ID: &str = "task-storage-2";

    #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
    enum TestTerminalResultKind {
        #[serde(rename = "completed")]
        Completed,
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
    enum TestTerminalOutcome {
        #[serde(rename = "verified")]
        Verified,
    }

    #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    struct TestTerminalEvidence {
        outcome: TestTerminalOutcome,
        sequence: PositiveGeneration,
    }

    #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    struct TestTerminalData {
        evidence: TestTerminalEvidence,
    }

    /// The sole Task 11 terminal fixture is recursively typed and closed.
    #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    struct TestTerminalEnvelope {
        result_kind: TestTerminalResultKind,
        operation_id: OperationId,
        task_id: TaskId,
        data: TestTerminalData,
    }

    impl contract_digest_record_sealed::Sealed for TestTerminalEnvelope {}
    impl ContractDigestRecord for TestTerminalEnvelope {}

    const _: fn() = || {
        trait AmbiguousIfDeserialize<Marker> {
            fn assert_not_deserialize() {}
        }
        struct ImplementsDeserialize;
        impl<T: ?Sized> AmbiguousIfDeserialize<()> for T {}
        impl<T: serde::de::DeserializeOwned> AmbiguousIfDeserialize<ImplementsDeserialize> for T {}
        let _ = <OperationRecord<TestTerminalEnvelope> as AmbiguousIfDeserialize<_>>::
            assert_not_deserialize;
    };

    fn digest(character: char) -> Sha256Digest {
        Sha256Digest::from_str(&character.to_string().repeat(64)).unwrap()
    }

    fn operation_id(value: &str) -> OperationId {
        OperationId::from_str(value).unwrap()
    }

    fn task_id(value: &str) -> TaskId {
        TaskId::from_str(value).unwrap()
    }

    fn unica_id(value: &str) -> UnicaId {
        UnicaId::from_str(value).unwrap()
    }

    fn project_id(value: &str) -> ProjectId {
        ProjectId::from_str(value).unwrap()
    }

    fn instant(value: &str) -> NormalizedUtcInstant {
        NormalizedUtcInstant::from_str(value).unwrap()
    }

    fn start_scope() -> OperationScope {
        OperationScope::StartAttempt {
            workspace_identity_digest: digest('a'),
            task_id: task_id(TASK_ID),
        }
    }

    fn task_scope() -> OperationScope {
        OperationScope::Task {
            project_id: project_id(PROJECT_ID),
            task_id: task_id(TASK_ID),
            instance_id: unica_id(INSTANCE_ID),
        }
    }

    fn start_selector() -> TaskOperationSelector {
        TaskOperationSelector::BranchedStart(BranchedStartSelector::new())
    }

    fn task_selector() -> TaskOperationSelector {
        TaskOperationSelector::BranchedCleanup(BranchedCleanupSelector::new(
            BranchedCleanupSelectorVariant::Apply,
        ))
    }

    fn operation_lease() -> OperationLease {
        operation_lease_for(
            OWNER_ID,
            1,
            "2026-07-22T00:00:00Z",
            "2026-07-22T00:00:10Z",
            "2026-07-22T00:01:00Z",
        )
    }

    fn alternate_operation_lease() -> OperationLease {
        operation_lease_for(
            OTHER_OWNER_ID,
            2,
            "2026-07-22T00:02:00Z",
            "2026-07-22T00:02:10Z",
            "2026-07-22T00:03:00Z",
        )
    }

    fn operation_lease_for(
        owner_id: &str,
        generation: u64,
        acquired_at: &str,
        heartbeat_at: &str,
        expires_at: &str,
    ) -> OperationLease {
        OperationLease::new(
            OperationLeaseAuthority::new(
                unica_id(owner_id),
                PositiveGeneration::new(generation).unwrap(),
                instant(acquired_at),
                instant(heartbeat_at),
                instant(expires_at),
            )
            .unwrap(),
        )
        .unwrap()
    }

    fn operation_lease_digest() -> Sha256Digest {
        operation_lease().lease_digest().clone()
    }

    fn terminal_envelope() -> TestTerminalEnvelope {
        TestTerminalEnvelope {
            result_kind: TestTerminalResultKind::Completed,
            operation_id: operation_id(OPERATION_ID),
            task_id: task_id(TASK_ID),
            data: TestTerminalData {
                evidence: TestTerminalEvidence {
                    outcome: TestTerminalOutcome::Verified,
                    sequence: PositiveGeneration::new(1).unwrap(),
                },
            },
        }
    }

    fn terminal_envelope_digest() -> Sha256Digest {
        canonical_contract_digest(&terminal_envelope(), None).unwrap()
    }

    fn common_record(
        scope: OperationScope,
        operation: TaskOperationSelector,
    ) -> Map<String, Value> {
        serde_json::from_value::<Map<String, Value>>(json!({
            "operationId": OPERATION_ID,
            "scope": scope,
            "operation": operation,
            "policy": "localJournaled",
            "canonicalInputDigest": digest('b'),
            "registeredAt": "2026-07-22T00:00:00Z"
        }))
        .unwrap()
    }

    fn registered_record(scope: OperationScope, operation: TaskOperationSelector) -> Value {
        let mut record = common_record(scope, operation);
        record.insert(
            "operationLease".into(),
            serde_json::to_value(operation_lease()).unwrap(),
        );
        record.insert("state".into(), json!("registered"));
        Value::Object(record)
    }

    fn intent_written_record(scope: OperationScope, operation: TaskOperationSelector) -> Value {
        let mut record = common_record(scope, operation);
        record.insert(
            "operationLease".into(),
            serde_json::to_value(operation_lease()).unwrap(),
        );
        record.insert("state".into(), json!("intentWritten"));
        Value::Object(record)
    }

    fn effect_unknown_record(scope: OperationScope, operation: TaskOperationSelector) -> Value {
        let mut record = common_record(scope, operation);
        record.insert(
            "lastOperationLeaseDigest".into(),
            json!(operation_lease_digest()),
        );
        record.insert("state".into(), json!("effectUnknown"));
        record.insert("recoveryDigest".into(), json!(digest('d')));
        Value::Object(record)
    }

    fn terminal_record(scope: OperationScope, operation: TaskOperationSelector) -> Value {
        let mut record = common_record(scope, operation);
        record.insert(
            "lastOperationLeaseDigest".into(),
            json!(operation_lease_digest()),
        );
        record.insert("state".into(), json!("terminal"));
        record.insert(
            "terminalEnvelopeDigest".into(),
            json!(terminal_envelope_digest()),
        );
        record.insert(
            "terminalEnvelope".into(),
            serde_json::to_value(terminal_envelope()).unwrap(),
        );
        Value::Object(record)
    }

    fn registered_state() -> TestExpectedOperationState {
        TestExpectedOperationState::registered(operation_lease_digest(), OperationOwnerState::Live)
    }

    fn intent_written_state() -> TestExpectedOperationState {
        TestExpectedOperationState::intent_written(
            operation_lease_digest(),
            OperationOwnerState::Orphaned,
        )
    }

    fn effect_unknown_state() -> TestExpectedOperationState {
        TestExpectedOperationState::effect_unknown(operation_lease_digest())
    }

    fn terminal_state() -> TestExpectedOperationState {
        TestExpectedOperationState::terminal(operation_lease_digest())
    }

    fn locator(
        scope: OperationScope,
        operation: TaskOperationSelector,
        state: TestExpectedOperationState,
    ) -> TestAuthoritativeOperationLocator {
        match scope {
            OperationScope::StartAttempt {
                workspace_identity_digest,
                task_id,
            } => TestAuthoritativeOperationLocator::start_attempt_storage_key(
                workspace_identity_digest,
                task_id,
                operation_id(OPERATION_ID),
                operation,
                DurableExecutionPolicy::LocalJournaled,
                state,
            ),
            OperationScope::Task {
                project_id,
                task_id,
                instance_id,
            } => TestAuthoritativeOperationLocator::authoritative_parent_task(
                project_id,
                task_id,
                instance_id,
                operation_id(OPERATION_ID),
                operation,
                DurableExecutionPolicy::LocalJournaled,
                state,
            ),
        }
    }

    fn opaque_candidate(
        value: &Value,
    ) -> Result<OpaqueOperationRecordCandidate, StorageContractError> {
        opaque_candidate_from_bytes(serde_json::to_vec(value).unwrap())
    }

    fn opaque_candidate_from_bytes(
        source_bytes: Vec<u8>,
    ) -> Result<OpaqueOperationRecordCandidate, StorageContractError> {
        preflight(std::sync::Arc::from(source_bytes))
            .into_opaque_candidate()
            .ok_or(StorageContractError::CandidateRejected)
    }

    fn load(
        value: &Value,
        expected_locator: &TestAuthoritativeOperationLocator,
    ) -> Result<TestOperationReplayView<TestTerminalEnvelope>, StorageContractError> {
        let candidate = opaque_candidate(value)?;
        load_test_operation_record(&candidate, expected_locator)
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    pub(in crate::domain::branched_development) enum ValidatedReplayFixtureState {
        Registered(OperationOwnerState),
        IntentWritten(OperationOwnerState),
        EffectUnknown(Sha256Digest),
        Terminal,
    }

    /// Test consumers may exercise replay classification only through the
    /// opaque preflight candidate and the typed storage loader. This fixture
    /// deliberately returns the root replay view, so the private terminal
    /// envelope type never becomes a second construction authority.
    pub(in crate::domain::branched_development) fn load_validated_replay_fixture(
        operation_id: OperationId,
        scope: OperationScope,
        operation: TaskOperationSelector,
        policy: DurableExecutionPolicy,
        canonical_input_digest: Sha256Digest,
        state: ValidatedReplayFixtureState,
    ) -> OperationReplayView {
        let mut record = serde_json::from_value::<Map<String, Value>>(json!({
            "operationId": operation_id,
            "scope": scope,
            "operation": operation,
            "policy": policy,
            "canonicalInputDigest": canonical_input_digest,
            "registeredAt": "2026-07-22T00:00:00Z"
        }))
        .unwrap();

        let expected_state = match state {
            ValidatedReplayFixtureState::Registered(owner_state) => {
                record.insert(
                    "operationLease".into(),
                    serde_json::to_value(operation_lease()).unwrap(),
                );
                record.insert("state".into(), json!("registered"));
                TestExpectedOperationState::registered(operation_lease_digest(), owner_state)
            }
            ValidatedReplayFixtureState::IntentWritten(owner_state) => {
                record.insert(
                    "operationLease".into(),
                    serde_json::to_value(operation_lease()).unwrap(),
                );
                record.insert("state".into(), json!("intentWritten"));
                TestExpectedOperationState::intent_written(operation_lease_digest(), owner_state)
            }
            ValidatedReplayFixtureState::EffectUnknown(recovery_digest) => {
                record.insert(
                    "lastOperationLeaseDigest".into(),
                    json!(operation_lease_digest()),
                );
                record.insert("state".into(), json!("effectUnknown"));
                record.insert("recoveryDigest".into(), json!(recovery_digest));
                TestExpectedOperationState::effect_unknown(operation_lease_digest())
            }
            ValidatedReplayFixtureState::Terminal => {
                record.insert(
                    "lastOperationLeaseDigest".into(),
                    json!(operation_lease_digest()),
                );
                record.insert("state".into(), json!("terminal"));
                record.insert(
                    "terminalEnvelopeDigest".into(),
                    json!(terminal_envelope_digest()),
                );
                record.insert(
                    "terminalEnvelope".into(),
                    serde_json::to_value(terminal_envelope()).unwrap(),
                );
                TestExpectedOperationState::terminal(operation_lease_digest())
            }
        };
        let record = Value::Object(record);
        let operation_id = serde_json::from_value(record["operationId"].clone()).unwrap();
        let scope: OperationScope = serde_json::from_value(record["scope"].clone()).unwrap();
        let operation: TaskOperationSelector =
            serde_json::from_value(record["operation"].clone()).unwrap();
        let expected =
            locator_for_exact_record(operation_id, scope, operation, policy, expected_state);
        let candidate = opaque_candidate(&record).unwrap();
        let storage_view =
            load_test_operation_record::<TestTerminalEnvelope>(&candidate, &expected).unwrap();
        OperationReplayView::from_validated_storage(&storage_view)
    }

    pub(in crate::domain::branched_development) fn validated_terminal_envelope_digest(
    ) -> Sha256Digest {
        terminal_envelope_digest()
    }

    fn locator_for_exact_record(
        operation_id: OperationId,
        scope: OperationScope,
        operation: TaskOperationSelector,
        policy: DurableExecutionPolicy,
        state: TestExpectedOperationState,
    ) -> TestAuthoritativeOperationLocator {
        match scope {
            OperationScope::StartAttempt {
                workspace_identity_digest,
                task_id,
            } => TestAuthoritativeOperationLocator::start_attempt_storage_key(
                workspace_identity_digest,
                task_id,
                operation_id,
                operation,
                policy,
                state,
            ),
            OperationScope::Task {
                project_id,
                task_id,
                instance_id,
            } => TestAuthoritativeOperationLocator::authoritative_parent_task(
                project_id,
                task_id,
                instance_id,
                operation_id,
                operation,
                policy,
                state,
            ),
        }
    }

    fn assert_shape_valid(value: &Value) {
        assert!(
            operation_record_schema_accepts::<TestTerminalEnvelope>(value),
            "expected schema-valid record: {value}"
        );
    }

    fn assert_shape_invalid(value: &Value) {
        assert!(
            !operation_record_schema_accepts::<TestTerminalEnvelope>(value),
            "expected schema-invalid record: {value}"
        );
    }

    #[test]
    fn generic_operation_record_schema_has_exactly_four_physical_states() {
        let schema =
            serde_json::to_value(schema_for!(OperationRecord<TestTerminalEnvelope>)).unwrap();
        audit_json_schema(&schema).unwrap();

        for record in [
            registered_record(start_scope(), start_selector()),
            intent_written_record(start_scope(), start_selector()),
            effect_unknown_record(start_scope(), start_selector()),
            terminal_record(start_scope(), start_selector()),
        ] {
            assert_shape_valid(&record);
        }

        let mut observed = effect_unknown_record(start_scope(), start_selector());
        observed["state"] = json!("observed");
        assert_shape_invalid(&observed);
        let mut unknown = effect_unknown_record(start_scope(), start_selector());
        unknown["state"] = json!("unknown");
        assert_shape_invalid(&unknown);
    }

    #[test]
    fn presence_matrix_is_exact_for_every_state() {
        let lease = serde_json::to_value(operation_lease()).unwrap();
        let last_lease = json!(digest('c'));
        let terminal_digest = json!(terminal_envelope_digest());
        let terminal = serde_json::to_value(terminal_envelope()).unwrap();
        let recovery = json!(digest('d'));

        for mut record in [
            registered_record(start_scope(), start_selector()),
            intent_written_record(start_scope(), start_selector()),
        ] {
            record.as_object_mut().unwrap().remove("operationLease");
            assert_shape_invalid(&record);
        }
        for field in [
            ("lastOperationLeaseDigest", last_lease.clone()),
            ("terminalEnvelopeDigest", terminal_digest.clone()),
            ("terminalEnvelope", terminal.clone()),
            ("recoveryDigest", recovery.clone()),
        ] {
            for mut record in [
                registered_record(start_scope(), start_selector()),
                intent_written_record(start_scope(), start_selector()),
            ] {
                record
                    .as_object_mut()
                    .unwrap()
                    .insert(field.0.into(), field.1.clone());
                assert_shape_invalid(&record);
            }
        }

        for required in ["lastOperationLeaseDigest", "recoveryDigest"] {
            let mut record = effect_unknown_record(start_scope(), start_selector());
            record.as_object_mut().unwrap().remove(required);
            assert_shape_invalid(&record);
        }
        for (field, value) in [
            ("operationLease", lease.clone()),
            ("terminalEnvelopeDigest", terminal_digest.clone()),
            ("terminalEnvelope", terminal.clone()),
        ] {
            let mut record = effect_unknown_record(start_scope(), start_selector());
            record.as_object_mut().unwrap().insert(field.into(), value);
            assert_shape_invalid(&record);
        }

        for required in [
            "lastOperationLeaseDigest",
            "terminalEnvelopeDigest",
            "terminalEnvelope",
        ] {
            let mut record = terminal_record(start_scope(), start_selector());
            record.as_object_mut().unwrap().remove(required);
            assert_shape_invalid(&record);
        }
        for (field, value) in [("operationLease", lease), ("recoveryDigest", recovery)] {
            let mut record = terminal_record(start_scope(), start_selector());
            record.as_object_mut().unwrap().insert(field.into(), value);
            assert_shape_invalid(&record);
        }
    }

    #[test]
    fn record_uses_one_typed_operation_and_has_no_version_or_digest_metadata() {
        let typed = OperationRecord::<TestTerminalEnvelope>(OperationRecordKind::Registered(
            RegisteredOperationRecord {
                operation_id: operation_id(OPERATION_ID),
                scope: start_scope(),
                operation: start_selector(),
                policy: DurableExecutionPolicy::LocalJournaled,
                canonical_input_digest: digest('b'),
                registered_at: instant("2026-07-22T00:00:00Z"),
                operation_lease: operation_lease(),
                state: RegisteredOperationRecordState::Value,
            },
        ));
        let encoded = serde_json::to_value(typed).unwrap();
        assert_shape_valid(&encoded);
        assert_eq!(encoded["operation"]["toolName"], "unica.branched.start");
        for forbidden in [
            "toolName",
            "requestVariant",
            "schemaVersion",
            "schemaDigest",
        ] {
            assert!(encoded.get(forbidden).is_none());
            let mut poisoned = encoded.clone();
            poisoned
                .as_object_mut()
                .unwrap()
                .insert(forbidden.into(), json!("poison"));
            assert_shape_invalid(&poisoned);
        }

        let mut missing_selector = encoded.clone();
        missing_selector
            .as_object_mut()
            .unwrap()
            .remove("operation");
        missing_selector
            .as_object_mut()
            .unwrap()
            .insert("toolName".into(), json!("unica.branched.start"));
        assert_shape_invalid(&missing_selector);
    }

    #[test]
    fn read_only_policy_is_impossible_before_replay_construction() {
        let mut record = registered_record(start_scope(), start_selector());
        record["policy"] = json!("readOnly");
        assert_shape_invalid(&record);
        assert_eq!(
            load(
                &record,
                &locator(start_scope(), start_selector(), registered_state()),
            ),
            Err(StorageContractError::CandidateRejected)
        );
    }

    #[test]
    fn only_the_preflight_opaque_object_branch_reaches_typed_loading() {
        let valid_bytes =
            serde_json::to_vec(&registered_record(start_scope(), start_selector())).unwrap();
        let candidate = opaque_candidate_from_bytes(valid_bytes.clone()).unwrap();
        assert_eq!(candidate.source_bytes().as_ref(), valid_bytes.as_slice());

        let mut invalid_utf8 = br#"{"policy":"localJournaled","value":""#.to_vec();
        invalid_utf8.push(0xff);
        invalid_utf8.extend_from_slice(br#""}"#);
        for rejected in [
            invalid_utf8,
            br#"["not-an-object"]"#.to_vec(),
            br#"{"policy":"readOnly"}"#.to_vec(),
            br#"{"policy":"localJournaled","policy":"readOnly"}"#.to_vec(),
        ] {
            assert!(matches!(
                opaque_candidate_from_bytes(rejected),
                Err(StorageContractError::CandidateRejected)
            ));
        }
    }

    #[test]
    fn generic_layer_defers_physical_selector_policy_binding_to_task_16() {
        // `branched.status` is physically read-only, but this generic framing
        // has no variant-policy registry authority. Task 16 must reject this
        // pairing when it binds `CurrentOperationRecord`; Task 11 rejects only
        // the impossible persisted `policy: readOnly` literal itself.
        let operation = TaskOperationSelector::BranchedStatus(BranchedStatusSelector::new());
        let record = registered_record(task_scope(), operation.clone());
        assert_shape_valid(&record);
        assert!(load(
            &record,
            &locator(task_scope(), operation, registered_state()),
        )
        .is_ok());
    }

    #[test]
    fn complete_start_attempt_scope_and_storage_key_are_compared() {
        let expected = locator(start_scope(), start_selector(), registered_state());
        let valid = registered_record(start_scope(), start_selector());
        assert!(load(&valid, &expected).is_ok());

        let mut substitutions = Vec::new();
        let mut operation = valid.clone();
        operation["operationId"] = json!(OTHER_OPERATION_ID);
        substitutions.push(operation);
        let mut workspace = valid.clone();
        workspace["scope"]["workspaceIdentityDigest"] = json!(digest('e'));
        substitutions.push(workspace);
        let mut task = valid;
        task["scope"]["taskId"] = json!(OTHER_TASK_ID);
        substitutions.push(task);

        for substituted in substitutions {
            assert_shape_valid(&substituted);
            assert_eq!(
                load(&substituted, &expected),
                Err(StorageContractError::CrossScope)
            );
        }
    }

    #[test]
    fn complete_task_scope_is_compared_with_the_authoritative_locator() {
        let expected = locator(task_scope(), task_selector(), registered_state());
        let valid = registered_record(task_scope(), task_selector());
        assert!(load(&valid, &expected).is_ok());

        let mut substitutions = Vec::new();
        let mut project = valid.clone();
        project["scope"]["projectId"] = json!(OTHER_PROJECT_ID);
        substitutions.push(project);
        let mut task = valid.clone();
        task["scope"]["taskId"] = json!(OTHER_TASK_ID);
        substitutions.push(task);
        let mut instance = valid;
        instance["scope"]["instanceId"] = json!(OTHER_INSTANCE_ID);
        substitutions.push(instance);

        for substituted in substitutions {
            assert_shape_valid(&substituted);
            assert_eq!(
                load(&substituted, &expected),
                Err(StorageContractError::CrossScope)
            );
        }
    }

    #[test]
    fn scope_and_selector_must_describe_the_same_container_kind() {
        let start_with_task_selector = registered_record(start_scope(), task_selector());
        assert_shape_valid(&start_with_task_selector);
        assert_eq!(
            load(
                &start_with_task_selector,
                &locator(start_scope(), task_selector(), registered_state()),
            ),
            Err(StorageContractError::ScopeSelectorMismatch)
        );

        let task_with_start_selector = registered_record(task_scope(), start_selector());
        assert_shape_valid(&task_with_start_selector);
        assert_eq!(
            load(
                &task_with_start_selector,
                &locator(task_scope(), start_selector(), registered_state()),
            ),
            Err(StorageContractError::ScopeSelectorMismatch)
        );
    }

    #[test]
    fn operation_scope_never_persists_a_path_or_caller_cwd_spelling() {
        for (scope, operation) in [
            (start_scope(), start_selector()),
            (task_scope(), task_selector()),
        ] {
            let valid = registered_record(scope, operation);
            for (field, value) in [
                ("cwd", json!("/caller/alias")),
                ("workspacePath", json!("/canonical/workspace")),
            ] {
                let mut poisoned = valid.clone();
                poisoned["scope"]
                    .as_object_mut()
                    .unwrap()
                    .insert(field.into(), value);
                assert_shape_invalid(&poisoned);
            }
        }
    }

    #[test]
    fn every_lease_preimage_member_and_both_digests_are_revalidated() {
        let valid = registered_record(start_scope(), start_selector());
        let expected = locator(start_scope(), start_selector(), registered_state());
        assert!(load(&valid, &expected).is_ok());

        let substitutions = [
            ("ownerInstanceId", json!(OTHER_OWNER_ID)),
            ("generation", json!(2)),
            ("acquiredAt", json!("2026-07-22T00:00:01Z")),
            ("heartbeatAt", json!("2026-07-22T00:00:11Z")),
            ("expiresAt", json!("2026-07-22T00:01:01Z")),
            ("heartbeatDigest", json!(digest('e'))),
            ("leaseDigest", json!(digest('f'))),
        ];
        for (field, replacement) in substitutions {
            let mut substituted = valid.clone();
            substituted["operationLease"][field] = replacement;
            assert_shape_valid(&substituted);
            assert_eq!(
                load(&substituted, &expected),
                Err(StorageContractError::InvalidOperationLease),
                "accepted substituted lease field {field}"
            );
        }

        let mut unordered = valid;
        unordered["operationLease"]["heartbeatAt"] = json!("2026-07-22T00:01:00Z");
        assert_shape_valid(&unordered);
        assert_eq!(
            load(&unordered, &expected),
            Err(StorageContractError::InvalidOperationLease)
        );
    }

    #[test]
    fn fully_rehashed_different_lease_is_rejected_by_authoritative_digest() {
        let mut substituted = registered_record(start_scope(), start_selector());
        substituted["operationLease"] = serde_json::to_value(alternate_operation_lease()).unwrap();
        assert_shape_valid(&substituted);

        let result = load(
            &substituted,
            &locator(start_scope(), start_selector(), registered_state()),
        );
        assert_eq!(
            result,
            Err(StorageContractError::OperationLeaseDigestMismatch)
        );
    }

    #[test]
    fn locator_binds_exact_selector_policy_and_physical_state() {
        let expected = locator(task_scope(), task_selector(), registered_state());

        let different_selector = registered_record(
            task_scope(),
            TaskOperationSelector::BranchedStatus(BranchedStatusSelector::new()),
        );
        assert_shape_valid(&different_selector);
        assert_eq!(
            load(&different_selector, &expected),
            Err(StorageContractError::OperationBindingMismatch)
        );

        let mut different_policy = registered_record(task_scope(), task_selector());
        different_policy["policy"] = json!("contained");
        assert_shape_valid(&different_policy);
        assert_eq!(
            load(&different_policy, &expected),
            Err(StorageContractError::OperationBindingMismatch)
        );

        let different_state = intent_written_record(task_scope(), task_selector());
        assert_shape_valid(&different_state);
        assert_eq!(
            load(&different_state, &expected),
            Err(StorageContractError::OperationBindingMismatch)
        );
    }

    #[test]
    fn locator_binds_last_lease_digest_after_current_lease_is_removed() {
        for (mut record, expected) in [
            (
                effect_unknown_record(start_scope(), start_selector()),
                locator(start_scope(), start_selector(), effect_unknown_state()),
            ),
            (
                terminal_record(start_scope(), start_selector()),
                locator(start_scope(), start_selector(), terminal_state()),
            ),
        ] {
            record["lastOperationLeaseDigest"] = json!(digest('f'));
            assert_shape_valid(&record);
            assert_eq!(
                load(&record, &expected),
                Err(StorageContractError::LastOperationLeaseDigestMismatch)
            );
        }
    }

    #[test]
    fn terminal_envelope_and_its_digest_are_revalidated_generically() {
        let valid = terminal_record(start_scope(), start_selector());
        let expected = locator(start_scope(), start_selector(), terminal_state());
        let loaded = load(&valid, &expected).unwrap();
        assert!(matches!(
            loaded.state(),
            TestOperationReplayStateRef::Terminal { .. }
        ));

        let mut changed_envelope = valid.clone();
        changed_envelope["terminalEnvelope"]["data"]["evidence"]["sequence"] = json!(2);
        assert_shape_valid(&changed_envelope);
        assert_eq!(
            load(&changed_envelope, &expected),
            Err(StorageContractError::InvalidTerminalEnvelopeDigest)
        );

        let mut changed_digest = valid;
        changed_digest["terminalEnvelopeDigest"] = json!(digest('f'));
        assert_shape_valid(&changed_digest);
        assert_eq!(
            load(&changed_digest, &expected),
            Err(StorageContractError::InvalidTerminalEnvelopeDigest)
        );

        let mut consistently_rehashed = terminal_record(start_scope(), start_selector());
        consistently_rehashed["terminalEnvelope"]["data"]["evidence"]["sequence"] = json!(2);
        let rehashed: TestTerminalEnvelope =
            serde_json::from_value(consistently_rehashed["terminalEnvelope"].clone()).unwrap();
        consistently_rehashed["terminalEnvelopeDigest"] =
            json!(canonical_contract_digest(&rehashed, None).unwrap());
        assert_shape_valid(&consistently_rehashed);
        assert!(load(&consistently_rehashed, &expected).is_ok());
    }

    #[test]
    fn opaque_terminal_candidate_reaches_the_exact_root_replay_view() {
        let scope = task_scope();
        let operation = task_selector();
        let candidate =
            opaque_candidate(&terminal_record(scope.clone(), operation.clone())).unwrap();
        let expected = locator(scope.clone(), operation.clone(), terminal_state());

        let storage_view =
            load_test_operation_record::<TestTerminalEnvelope>(&candidate, &expected).unwrap();
        let TestOperationReplayStateRef::Terminal {
            terminal_envelope_digest: observed_terminal_envelope_digest,
            terminal_envelope: observed_terminal_envelope,
            ..
        } = storage_view.state()
        else {
            panic!("authoritative terminal locator returned a non-terminal replay state");
        };
        assert_eq!(
            observed_terminal_envelope_digest,
            &terminal_envelope_digest()
        );
        assert_eq!(observed_terminal_envelope, &terminal_envelope());

        let replay = OperationReplayView::from_validated_storage(&storage_view);
        assert_eq!(replay.operation_id(), &operation_id(OPERATION_ID));
        assert_eq!(replay.scope(), &scope);
        assert_eq!(replay.operation(), &operation);
        assert_eq!(replay.policy(), DurableExecutionPolicy::LocalJournaled);
        assert_eq!(replay.canonical_input_digest(), &digest('b'));
        assert_eq!(replay.state(), OperationState::Terminal);
    }

    #[test]
    fn all_four_valid_records_construct_typed_test_replay_views() {
        let views = [
            load(
                &registered_record(start_scope(), start_selector()),
                &locator(start_scope(), start_selector(), registered_state()),
            )
            .unwrap(),
            load(
                &intent_written_record(start_scope(), start_selector()),
                &locator(start_scope(), start_selector(), intent_written_state()),
            )
            .unwrap(),
            load(
                &effect_unknown_record(start_scope(), start_selector()),
                &locator(start_scope(), start_selector(), effect_unknown_state()),
            )
            .unwrap(),
            load(
                &terminal_record(start_scope(), start_selector()),
                &locator(start_scope(), start_selector(), terminal_state()),
            )
            .unwrap(),
        ];
        assert!(matches!(
            views[0].state(),
            TestOperationReplayStateRef::Registered {
                owner_state: OperationOwnerState::Live,
                ..
            }
        ));
        assert!(matches!(
            views[1].state(),
            TestOperationReplayStateRef::IntentWritten {
                owner_state: OperationOwnerState::Orphaned,
                ..
            }
        ));
        assert!(matches!(
            views[2].state(),
            TestOperationReplayStateRef::EffectUnknown { .. }
        ));
        assert!(matches!(
            views[3].state(),
            TestOperationReplayStateRef::Terminal { .. }
        ));
        assert!(views
            .iter()
            .all(|view| view.operation_id() == &operation_id(OPERATION_ID)));
        assert_eq!(views[0].scope(), &start_scope());
        assert_eq!(views[0].operation(), &start_selector());
        assert_eq!(views[0].policy(), DurableExecutionPolicy::LocalJournaled);
        assert_eq!(views[0].canonical_input_digest(), &digest('b'));
        assert_eq!(views[0].registered_at(), &instant("2026-07-22T00:00:00Z"));
    }
}
