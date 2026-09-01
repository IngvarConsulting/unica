use super::validate_platform_xml_binding;
use crate::infrastructure::native_operations::apply::{
    empty_apply_family_batch, hidden_apply_family_unimplemented, ApplyPlanError,
    ApplyPlanErrorKind, ApplyStagedState,
};
use crate::infrastructure::native_operations::apply_families::request::{
    IndexedPlanOperation, ProvisionalApplyEffect,
};
use crate::infrastructure::workspace_actor::{FormResourceApplyAuthority, ProviderRootBinding};
use serde_json::Value;

#[derive(Debug, Clone)]
pub(crate) struct FormResourcePlanOperation {
    operation: String,
}

impl FormResourcePlanOperation {
    pub(crate) fn operation(&self) -> &str {
        &self.operation
    }
}

pub(crate) fn parse_form_resource_plan_operation(
    operation: &str,
    _args: &Value,
    op_index: usize,
    binding: &ProviderRootBinding,
) -> Result<FormResourcePlanOperation, ApplyPlanError> {
    validate_platform_xml_binding(binding, op_index)?;
    Ok(FormResourcePlanOperation {
        operation: operation.to_string(),
    })
}

pub(crate) fn plan_form_resource_batch(
    staged: ApplyStagedState,
    authority: FormResourceApplyAuthority<'_>,
    operations: &[IndexedPlanOperation<FormResourcePlanOperation>],
) -> Result<(ApplyStagedState, Vec<ProvisionalApplyEffect>), ApplyPlanError> {
    let Some(first) = operations.first() else {
        return Err(empty_apply_family_batch());
    };
    if !authority.owns_staged_state(&staged) {
        return Err(ApplyPlanError::new(
            ApplyPlanErrorKind::InvalidState,
            "form/resource planner authority does not own the staged state",
        )
        .at_path("ops"));
    }
    let _ = first.operation();
    Err(hidden_apply_family_unimplemented(first.index()))
}

#[cfg(test)]
mod tests {
    use super::{parse_form_resource_plan_operation, plan_form_resource_batch};
    use crate::infrastructure::native_operations::apply::ApplyPlanErrorKind;
    use crate::infrastructure::native_operations::apply_families::request::IndexedPlanOperation;
    use crate::infrastructure::native_operations::apply_families::tests::ApplySeamFixture;
    use serde_json::json;

    #[test]
    fn form_resource_apply_seam_routes_actor_authorized_batch_to_stable_unsupported() {
        let fixture = ApplySeamFixture::new();
        let admission = fixture.admission();
        let staged = admission.staged_state().unwrap();
        let authority = admission
            .form_resource_planning_authority(&fixture.binding)
            .unwrap();
        let operation = parse_form_resource_plan_operation(
            "form.create",
            &json!({"at": "main:Configuration"}),
            0,
            &fixture.binding,
        )
        .unwrap();

        let operation = IndexedPlanOperation::new(0, operation);
        let error = plan_form_resource_batch(staged, authority, &[operation]).unwrap_err();

        assert_eq!(error.kind(), ApplyPlanErrorKind::ProviderUnavailable);
        assert_eq!(error.path(), Some("ops[0].op"));
        assert_eq!(
            error.to_string(),
            "hidden v0.13 apply family is not implemented"
        );
    }
}
