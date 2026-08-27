use super::validate_platform_xml_binding;
use crate::infrastructure::native_operations::apply::{
    empty_apply_family_batch, hidden_apply_family_unimplemented, ApplyPlanError,
    ApplyPlanErrorKind, ApplyStagedState, PlannedApplyEffects,
};
use crate::infrastructure::workspace_actor::{MetadataApplyAuthority, ProviderRootBinding};
use serde_json::Value;

#[derive(Debug)]
pub(crate) struct MetadataPlanOperation {
    operation: String,
    op_index: usize,
}

impl MetadataPlanOperation {
    pub(crate) fn operation(&self) -> &str {
        &self.operation
    }

    pub(crate) const fn op_index(&self) -> usize {
        self.op_index
    }
}

pub(crate) fn parse_metadata_plan_operation(
    operation: &str,
    _args: &Value,
    op_index: usize,
    binding: &ProviderRootBinding,
) -> Result<MetadataPlanOperation, ApplyPlanError> {
    validate_platform_xml_binding(binding, op_index)?;
    Ok(MetadataPlanOperation {
        operation: operation.to_string(),
        op_index,
    })
}

pub(crate) fn plan_metadata_batch(
    staged: ApplyStagedState,
    authority: MetadataApplyAuthority<'_>,
    operations: &[MetadataPlanOperation],
) -> Result<(ApplyStagedState, PlannedApplyEffects), ApplyPlanError> {
    let Some(first) = operations.first() else {
        return Err(empty_apply_family_batch());
    };
    if !authority.owns_staged_state(&staged) {
        return Err(ApplyPlanError::new(
            ApplyPlanErrorKind::InvalidState,
            "metadata planner authority does not own the staged state",
        )
        .at_path("ops"));
    }
    let _ = first.operation();
    Err(hidden_apply_family_unimplemented(first.op_index()))
}

#[cfg(test)]
mod tests {
    use super::{parse_metadata_plan_operation, plan_metadata_batch};
    use crate::infrastructure::native_operations::apply::ApplyPlanErrorKind;
    use crate::infrastructure::native_operations::apply_families::tests::ApplySeamFixture;
    use serde_json::json;

    #[test]
    fn metadata_apply_seam_routes_actor_authorized_batch_to_stable_unsupported() {
        let fixture = ApplySeamFixture::new();
        let admission = fixture.admission();
        let staged = admission.staged_state().unwrap();
        let authority = admission
            .metadata_planning_authority(&fixture.binding)
            .unwrap();
        let operation = parse_metadata_plan_operation(
            "object.create",
            &json!({"at": "main:Configuration"}),
            0,
            &fixture.binding,
        )
        .unwrap();

        let error = plan_metadata_batch(staged, authority, &[operation]).unwrap_err();

        assert_eq!(error.kind(), ApplyPlanErrorKind::ProviderUnavailable);
        assert_eq!(error.path(), Some("ops[0].op"));
        assert_eq!(
            error.to_string(),
            "hidden v0.13 apply family is not implemented"
        );
    }
}
