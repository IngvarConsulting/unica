use crate::owner::{
    prove_already_read_metadata_owner, prove_already_read_source_set_owner,
    EventOwnerDescriptorProof, EventOwnerEvidencePlan, PlatformXmlSourceSetOwnerEvidence,
};
use unica_source_contract::portable_path::PortablePathExt;
use unica_source_contract::proposal::{ApplyPlanError, ApplyPlanErrorKind, ApplyProposal};
use unica_source_contract::SourceSetKind;
pub fn prove_owner_plan(
    staged: &mut ApplyProposal<'_>,
    plan: &EventOwnerEvidencePlan,
) -> Result<(), ApplyPlanError> {
    let mut evidence = Vec::<PlatformXmlSourceSetOwnerEvidence>::new();
    for (index, step) in plan.descriptors.iter().enumerate() {
        if let Some(parent) = step.registered_by {
            let parent = evidence.get(parent).ok_or_else(|| {
                ApplyPlanError::new(
                    ApplyPlanErrorKind::ProviderUnavailable,
                    "owner evidence plan references an unavailable parent",
                )
            })?;
            let name = step.expected_name.as_deref().ok_or_else(|| {
                ApplyPlanError::new(
                    ApplyPlanErrorKind::ProviderUnavailable,
                    "metadata owner expectation has no exact name",
                )
            })?;
            if !parent.registers(&step.expected_kind, name) {
                return Err(ApplyPlanError::new(
                    ApplyPlanErrorKind::NotFound,
                    format!(
                        "owner `{}` is not registered by descriptor {}",
                        step.expected_kind,
                        index.saturating_sub(1)
                    ),
                ));
            }
        }
        let bytes = staged
            .read(&step.relative)
            .map_err(|error| {
                ApplyPlanError::new(ApplyPlanErrorKind::Staging(error.kind()), error.to_string())
            })?
            .ok_or_else(|| {
                ApplyPlanError::new(
                    ApplyPlanErrorKind::ProviderUnavailable,
                    format!(
                        "proved owner descriptor `{}` is absent",
                        step.relative.portable()
                    ),
                )
            })?;
        let actual = match step.proof {
            EventOwnerDescriptorProof::SourceSet => {
                prove_already_read_source_set_owner(&step.relative, &bytes, plan.source_set_kind)
            }
            EventOwnerDescriptorProof::Metadata => {
                prove_already_read_metadata_owner(&step.relative, &bytes)
            }
        }
        .map_err(|error| {
            ApplyPlanError::new(ApplyPlanErrorKind::ProviderUnavailable, error.message)
        })?;
        if actual.artifact_kind() != step.expected_kind
            || step
                .expected_name
                .as_deref()
                .is_some_and(|name| actual.artifact_name() != Some(name))
            || actual.version() != Some("2.20")
            || (step.proof == EventOwnerDescriptorProof::SourceSet
                && matches!(
                    plan.source_set_kind,
                    SourceSetKind::Configuration | SourceSetKind::Extension
                )
                && actual.is_configuration_extension()
                    != (plan.source_set_kind == SourceSetKind::Extension))
        {
            return Err(ApplyPlanError::new(
                ApplyPlanErrorKind::ProviderUnavailable,
                format!(
                    "owner descriptor `{}` has inconsistent identity or format",
                    step.relative.portable()
                ),
            ));
        }
        evidence.push(actual);
    }
    Ok(())
}
