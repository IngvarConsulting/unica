use crate::support::{parse_support_state_compat_bytes, support_root_uuid_from_bytes};
use std::path::Path;
use unica_source_contract::proposal::{ApplyPlanError, ApplyPlanErrorKind, ApplyProposal};
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CodeSupportLock {
    Configuration,
    Owner { evidence_index: usize },
}

pub fn prove_staged_code_support(
    staged: &mut ApplyProposal<'_>,
    support_policy: unica_source_contract::proposal::SupportPolicyMode,
    owner_bytes: &[Vec<u8>],
    at_path: &str,
) -> Result<Option<CodeSupportLock>, ApplyPlanError> {
    let marker = staged
        .read(Path::new("Ext/ParentConfigurations.bin"))
        .map_err(|error| ApplyPlanError::staging(error, at_path))?;
    if support_policy == unica_source_contract::proposal::SupportPolicyMode::Off {
        return Ok(None);
    }
    let state = parse_support_state_compat_bytes(marker.as_deref());
    let Some(state) = state else {
        return Ok(None);
    };
    if state.removed() {
        return Ok(None);
    }
    if !state.global_editing_enabled() {
        if support_policy == unica_source_contract::proposal::SupportPolicyMode::Warn {
            return Ok(Some(CodeSupportLock::Configuration));
        }
        return Err(ApplyPlanError::new(
            ApplyPlanErrorKind::InvalidState,
            "actor support policy denies editing this protected source",
        )
        .at_path(at_path));
    }
    let owner_uuid = owner_bytes
        .iter()
        .enumerate()
        .rev()
        .find_map(|(index, bytes)| support_root_uuid_from_bytes(bytes).map(|uuid| (index, uuid)));
    if owner_uuid
        .as_ref()
        .and_then(|(_, uuid)| state.object_rule(uuid))
        == Some(0)
    {
        if support_policy == unica_source_contract::proposal::SupportPolicyMode::Warn {
            return Ok(
                owner_uuid.map(|(evidence_index, _)| CodeSupportLock::Owner { evidence_index })
            );
        }
        return Err(ApplyPlanError::new(
            ApplyPlanErrorKind::InvalidState,
            "actor support policy denies editing this protected module owner",
        )
        .at_path(at_path));
    }
    Ok(None)
}
