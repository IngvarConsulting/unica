use crate::support::{parse_support_state_compat_bytes, support_root_uuid_from_bytes};
use std::path::Path;
use unica_source_contract::proposal::{ApplyPlanError, ApplyPlanErrorKind, ApplyProposal};
pub fn prove_staged_code_support(
    staged: &mut ApplyProposal<'_>,
    support_policy: unica_source_contract::proposal::SupportPolicyMode,
    source_kind: unica_source_contract::SourceSetKind,
    owner_bytes: &[Vec<u8>],
    at_path: &str,
) -> Result<(), ApplyPlanError> {
    if matches!(
        source_kind,
        unica_source_contract::SourceSetKind::ExternalProcessor
            | unica_source_contract::SourceSetKind::ExternalReport
    ) {
        return Ok(());
    }
    let marker = staged
        .read(Path::new("Ext/ParentConfigurations.bin"))
        .map_err(|error| ApplyPlanError::staging(error, at_path))?;
    if support_policy != unica_source_contract::proposal::SupportPolicyMode::Deny {
        return Ok(());
    }
    let state = parse_support_state_compat_bytes(marker.as_deref());
    let Some(state) = state else {
        return Ok(());
    };
    if state.removed() {
        return Ok(());
    }
    if !state.global_editing_enabled() {
        return Err(ApplyPlanError::new(
            ApplyPlanErrorKind::InvalidState,
            "actor support policy denies editing this protected source",
        )
        .at_path(at_path));
    }
    let owner_uuid = owner_bytes
        .iter()
        .rev()
        .find_map(|bytes| support_root_uuid_from_bytes(bytes));
    if owner_uuid
        .as_deref()
        .and_then(|uuid| state.object_rule(uuid))
        == Some(0)
    {
        return Err(ApplyPlanError::new(
            ApplyPlanErrorKind::InvalidState,
            "actor support policy denies editing this protected module owner",
        )
        .at_path(at_path));
    }
    Ok(())
}
