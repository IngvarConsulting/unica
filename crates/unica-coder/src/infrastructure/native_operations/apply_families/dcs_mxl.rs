use super::validate_platform_xml_binding;
use crate::infrastructure::native_operations::apply::{
    empty_apply_family_batch, hidden_apply_family_unimplemented, ApplyPlanError,
    ApplyPlanErrorKind, ApplyStagedState,
};
use crate::infrastructure::native_operations::apply_families::request::{
    IndexedPlanOperation, ProvisionalApplyEffect,
};
use crate::infrastructure::workspace_actor::{DcsMxlApplyAuthority, ProviderRootBinding};
use serde_json::Value;

#[derive(Debug, Clone)]
pub(crate) struct DcsMxlPlanOperation {
    operation: String,
}

impl DcsMxlPlanOperation {
    pub(crate) fn operation(&self) -> &str {
        &self.operation
    }
}

pub(crate) fn parse_dcs_mxl_plan_operation(
    operation: &str,
    _args: &Value,
    op_index: usize,
    binding: &ProviderRootBinding,
) -> Result<DcsMxlPlanOperation, ApplyPlanError> {
    validate_platform_xml_binding(binding, op_index)?;
    Ok(DcsMxlPlanOperation {
        operation: operation.to_string(),
    })
}

pub(crate) fn plan_dcs_mxl_batch(
    staged: ApplyStagedState,
    authority: DcsMxlApplyAuthority<'_>,
    operations: &[IndexedPlanOperation<DcsMxlPlanOperation>],
) -> Result<(ApplyStagedState, Vec<ProvisionalApplyEffect>), ApplyPlanError> {
    let Some(first) = operations.first() else {
        return Err(empty_apply_family_batch());
    };
    if !authority.owns_staged_state(&staged) {
        return Err(ApplyPlanError::new(
            ApplyPlanErrorKind::InvalidState,
            "DCS/MXL planner authority does not own the staged state",
        )
        .at_path("ops"));
    }
    let _ = first.operation();
    Err(hidden_apply_family_unimplemented(first.index()))
}

#[cfg(test)]
mod tests {
    use super::{parse_dcs_mxl_plan_operation, plan_dcs_mxl_batch};
    use crate::infrastructure::native_operations::apply::ApplyPlanErrorKind;
    use crate::infrastructure::native_operations::apply_families::request::IndexedPlanOperation;
    use crate::infrastructure::native_operations::apply_families::tests::ApplySeamFixture;
    use serde_json::json;

    #[test]
    fn dcs_mxl_apply_seam_routes_actor_authorized_batch_to_stable_unsupported() {
        let fixture = ApplySeamFixture::new();
        let admission = fixture.admission();
        let staged = admission.staged_state().unwrap();
        let authority = admission
            .dcs_mxl_planning_authority(&fixture.binding)
            .unwrap();
        let operation = parse_dcs_mxl_plan_operation(
            "dcs.set",
            &json!({"at": "main:Configuration"}),
            0,
            &fixture.binding,
        )
        .unwrap();

        let operation = IndexedPlanOperation::new(0, operation);
        let error = plan_dcs_mxl_batch(staged, authority, &[operation]).unwrap_err();

        assert_eq!(error.kind(), ApplyPlanErrorKind::ProviderUnavailable);
        assert_eq!(error.path(), Some("ops[0].op"));
        assert_eq!(
            error.to_string(),
            "hidden v0.13 apply family is not implemented"
        );
    }
}
