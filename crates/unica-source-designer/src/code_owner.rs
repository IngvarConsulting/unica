use crate::format_profile::writable_profile_refusal;
use crate::module_layout::PlatformXmlModuleIdentity;
use crate::owner::{
    prove_already_read_metadata_owner, prove_already_read_source_set_owner,
    PlatformXmlSourceSetOwnerEvidence,
};
use std::path::Path;
use unica_source_contract::portable_path::PortablePathExt;
use unica_source_contract::proposal::{ApplyPlanError, ApplyPlanErrorKind, ApplyProposal};
use unica_source_contract::SourceSetKind;
pub fn prove_staged_code_owner(
    staged: &mut ApplyProposal<'_>,
    source_kind: SourceSetKind,
    expected_format: &str,
    identity: &PlatformXmlModuleIdentity,
    at_path: &str,
) -> Result<Vec<Vec<u8>>, ApplyPlanError> {
    let root_relative = Path::new("Configuration.xml");
    let root = staged
        .read(root_relative)
        .map_err(|error| ApplyPlanError::staging(error, at_path))?
        .ok_or_else(|| {
            ApplyPlanError::new(
                ApplyPlanErrorKind::NotFound,
                "source-set owner descriptor is absent",
            )
            .at_path(at_path)
        })?;
    let mut evidence = prove_already_read_source_set_owner(root_relative, &root, source_kind)
        .map_err(|_| invalid_code_owner(at_path))?;
    require_code_owner_format(&evidence, expected_format, root_relative, at_path)?;
    let mut owner_bytes = vec![root];
    let owner_parts = identity.address.as_str().split('.').collect::<Vec<_>>();
    let (owner_pairs, remainder) =
        owner_parts[..owner_parts.len().saturating_sub(1)].as_chunks::<2>();
    if !remainder.is_empty() {
        return Err(ApplyPlanError::new(
            ApplyPlanErrorKind::Postcondition,
            "module logical owner has an incomplete kind/name pair",
        )
        .at_path(at_path));
    }
    let mut owner_index = 0usize;
    for relative in &identity.descriptors {
        if relative == root_relative {
            continue;
        }
        let expected = owner_pairs.get(owner_index).ok_or_else(|| {
            ApplyPlanError::new(
                ApplyPlanErrorKind::Postcondition,
                "module descriptor depth exceeds its logical owner",
            )
            .at_path(at_path)
        })?;
        if !evidence.registers(expected[0], expected[1]) {
            return Err(ApplyPlanError::new(
                ApplyPlanErrorKind::NotFound,
                "logical module owner is not registered by its parent descriptor",
            )
            .at_path(at_path));
        }
        let bytes = staged
            .read(relative)
            .map_err(|error| ApplyPlanError::staging(error, at_path))?
            .ok_or_else(|| {
                ApplyPlanError::new(
                    ApplyPlanErrorKind::NotFound,
                    "logical module owner descriptor is absent",
                )
                .at_path(at_path)
            })?;
        let current = prove_already_read_metadata_owner(relative, &bytes)
            .map_err(|_| invalid_code_owner(at_path))?;
        require_code_owner_format(&current, expected_format, relative, at_path)?;
        if current.artifact_kind() != expected[0] || current.artifact_name() != Some(expected[1]) {
            return Err(invalid_code_owner(at_path));
        }
        owner_bytes.push(bytes);
        evidence = current;
        owner_index += 1;
    }
    if owner_index != owner_pairs.len() {
        return Err(ApplyPlanError::new(
            ApplyPlanErrorKind::Postcondition,
            "module descriptor chain is incomplete for its logical owner",
        )
        .at_path(at_path));
    }
    Ok(owner_bytes)
}
/// The refusal names the owner file, the version it records and the
/// re-export the platform needs, as the shared writable-profile guard does.
fn require_code_owner_format(
    evidence: &PlatformXmlSourceSetOwnerEvidence,
    expected_format: &str,
    relative: &Path,
    at_path: &str,
) -> Result<(), ApplyPlanError> {
    if evidence.version() == Some(expected_format) {
        return Ok(());
    }
    let message = writable_profile_refusal(relative.portable(), evidence.version()).map_or_else(
        || {
            format!(
                "{}: module owner does not match the actor-issued serialization profile {expected_format}",
                relative.portable()
            )
        },
        |refusal| refusal.message,
    );
    Err(ApplyPlanError::new(ApplyPlanErrorKind::InvalidSource, message).at_path(at_path))
}
fn invalid_code_owner(at_path: &str) -> ApplyPlanError {
    ApplyPlanError::new(
        ApplyPlanErrorKind::InvalidSource,
        "logical module owner evidence is invalid",
    )
    .at_path(at_path)
}
