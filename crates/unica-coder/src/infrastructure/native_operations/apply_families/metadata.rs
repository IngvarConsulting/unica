//! Existing cross-component canonical metadata tests.
use super::validate_platform_xml_binding;
use serde_json::Value;
use unica_address::address::QualifiedAddress;
use unica_metadata_designer::decode::*;
use unica_metadata_designer::planner::set_ml_property;
use unica_operations::OperationDescriptor;
use unica_source_contract::proposal::{
    ApplyPlanError, IndexedPlanOperation, ProvisionalApplyEffect,
};
use unica_workspace::actor::ProviderRootBinding;
#[cfg(any(test, feature = "test-support"))]
fn parse_metadata_plan_operation_in(
    descriptor: Option<OperationDescriptor>,
    operation: &str,
    args: &Value,
    op_index: usize,
    binding: &ProviderRootBinding,
) -> Result<MetadataPlanOperation, ApplyPlanError> {
    validate_platform_xml_binding(binding, operation, op_index)?;
    unica_metadata_designer::decode::parse_metadata_plan_operation_in(
        descriptor,
        operation,
        args,
        op_index,
        &MetadataDecodeContext {
            source_set_name: binding.source_set_name(),
            source_kind: binding.source_kind(),
            registry: crate::domain::apply::operation_registry(),
        },
    )
}

#[cfg(any(test, feature = "test-support"))]
pub(crate) fn parse_metadata_plan_operation(
    operation: &str,
    args: &Value,
    op_index: usize,
    binding: &ProviderRootBinding,
) -> Result<MetadataPlanOperation, ApplyPlanError> {
    let at = args
        .get("at")
        .and_then(Value::as_str)
        .and_then(|raw| QualifiedAddress::resolve_input(raw, &[binding.source_set_name()]).ok());
    let descriptor = super::resolved_descriptor(operation, at.as_ref(), binding);
    parse_metadata_plan_operation_in(descriptor, operation, args, op_index, binding)
}

#[cfg(any(test, feature = "test-support"))]
pub(crate) fn plan_metadata_batch_owned_for_test(
    mut staged: unica_workspace::apply::ApplyStagedState,
    authority: unica_workspace::actor::MetadataApplyAuthority<'_>,
    operations: &[IndexedPlanOperation<MetadataPlanOperation>],
) -> Result<
    (
        unica_workspace::apply::ApplyStagedState,
        Vec<ProvisionalApplyEffect>,
    ),
    ApplyPlanError,
> {
    let code_type_plan =
        unica_metadata_designer::planner::PredefinedCodeTypePlan::from_operations(operations);
    let effects = unica_metadata_designer::planner::plan_metadata_batch(&crate::infrastructure::native_operations::meta::template_source::MetadataPlanningReadAdapter::new(&authority),unica_metadata_designer::planner::TemplateSeedEmitter(unica_templates_designer::mxl_root::empty_spreadsheet_document_xml), &mut staged.proposal(), &authority, operations, &code_type_plan, &mut Default::default())?;
    code_type_plan.validate_final(&mut staged.proposal(), authority.source_kind())?;
    Ok((staged, effects))
}

#[cfg(test)]
mod tests;
