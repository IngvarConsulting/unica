pub(crate) mod dcs_mxl;
pub(crate) mod form_resource;
pub(crate) mod metadata;
mod request;

use crate::domain::apply::{ApplyRequest, OperationFamily};
use crate::domain::project_sources::{SourceFormat, SourceSetKind};
use crate::infrastructure::native_operations::apply::{
    hidden_apply_family_unimplemented, ApplyPlanError, ApplyPlanErrorKind, ApplyStagedState,
    PlannedApplyEffects,
};
use crate::infrastructure::workspace_actor::{ApplyAdmission, ProviderRootBinding};
use request::{reconcile_effects, IndexedPlanOperation, ProvisionalApplyEffect};

enum ParsedApplyOperation {
    Metadata(IndexedPlanOperation<metadata::MetadataPlanOperation>),
    FormResource(IndexedPlanOperation<form_resource::FormResourcePlanOperation>),
    DcsMxl(IndexedPlanOperation<dcs_mxl::DcsMxlPlanOperation>),
    Code(IndexedPlanOperation<crate::infrastructure::native_operations::code::CodePlanOperation>),
    Xdto(IndexedPlanOperation<crate::infrastructure::native_operations::xdto::XdtoPlanOperation>),
    Unsupported(usize),
}

impl ParsedApplyOperation {
    /// Consecutive operations of one family plan as a single batch.
    fn same_family(&self, other: &Self) -> bool {
        std::mem::discriminant(self) == std::mem::discriminant(other)
    }
}

/// A planner that already reconciled its events against its own staged
/// changes reports them tied to every path the batch changed: the request
/// finalizer then keeps an event only while all of those paths stay changed
/// in the final postimage.
fn batch_effects_as_provisional(
    before: &[std::path::PathBuf],
    staged: &ApplyStagedState,
    effects: PlannedApplyEffects,
    op_index: usize,
) -> Vec<ProvisionalApplyEffect> {
    let changed = staged
        .planned_changes()
        .into_iter()
        .map(|change| change.relative_path)
        .filter(|path| !before.contains(path))
        .collect::<Vec<_>>();
    effects
        .into_events()
        .into_iter()
        .map(|event| ProvisionalApplyEffect::spanning(changed.clone(), event, op_index))
        .collect()
}

fn validate_platform_xml_binding(
    binding: &ProviderRootBinding,
    op_index: usize,
) -> Result<(), ApplyPlanError> {
    if binding.source_format() != SourceFormat::PlatformXml
        || !matches!(
            binding.source_kind(),
            SourceSetKind::Configuration | SourceSetKind::Extension
        )
        || binding.source_profile().platform_profile().is_none()
        || binding.source_profile().serialization_format().is_none()
    {
        return Err(ApplyPlanError::new(
            ApplyPlanErrorKind::ProviderUnavailable,
            "admitted source has no writable exact Platform XML profile",
        )
        .at_path(format!("ops[{op_index}].op")));
    }
    Ok(())
}

pub(crate) fn plan_hidden_v13_apply(
    request: &ApplyRequest,
    binding: &ProviderRootBinding,
    admission: &ApplyAdmission,
) -> Result<(ApplyStagedState, PlannedApplyEffects), ApplyPlanError> {
    let parsed = request
        .ops()
        .iter()
        .enumerate()
        .map(|(op_index, operation)| {
            let Some(family) = crate::application::v13::apply::dispatch_family(operation.name())
            else {
                return Err(hidden_apply_family_unimplemented(op_index));
            };
            let args = serde_json::Value::Object(operation.args().clone());
            let parsed = match family {
                OperationFamily::Metadata | OperationFamily::Properties => {
                    let parsed = metadata::parse_metadata_plan_operation(
                        operation.name(),
                        &args,
                        op_index,
                        binding,
                    )?;
                    ParsedApplyOperation::Metadata(IndexedPlanOperation::new(op_index, parsed))
                }
                OperationFamily::Form
                | OperationFamily::Role
                | OperationFamily::Subsystem
                | OperationFamily::Support => {
                    let parsed = form_resource::parse_form_resource_plan_operation(
                        operation.name(),
                        &args,
                        op_index,
                        binding,
                    )?;
                    ParsedApplyOperation::FormResource(IndexedPlanOperation::new(op_index, parsed))
                }
                OperationFamily::Dcs | OperationFamily::Mxl => {
                    let parsed = dcs_mxl::parse_dcs_mxl_plan_operation(
                        operation.name(),
                        &args,
                        op_index,
                        binding,
                    )?;
                    ParsedApplyOperation::DcsMxl(IndexedPlanOperation::new(op_index, parsed))
                }
                OperationFamily::Code => {
                    let parsed =
                        crate::infrastructure::native_operations::code::parse_code_plan_operation(
                            operation.name(),
                            &args,
                            op_index,
                            binding,
                        )?;
                    ParsedApplyOperation::Code(IndexedPlanOperation::new(op_index, parsed))
                }
                OperationFamily::Xdto => {
                    let parsed =
                        crate::infrastructure::native_operations::xdto::parse_xdto_plan_operation(
                            operation.name(),
                            &args,
                            op_index,
                            binding,
                        )?;
                    ParsedApplyOperation::Xdto(IndexedPlanOperation::new(op_index, parsed))
                }
                OperationFamily::Event => ParsedApplyOperation::Unsupported(op_index),
            };
            Ok(parsed)
        })
        .collect::<Result<Vec<_>, ApplyPlanError>>()?;

    let mut staged = admission
        .staged_state()
        .map_err(|error| ApplyPlanError::staging(error, "ops"))?;
    let mut provisional = Vec::new();
    let mut cursor = 0;
    while cursor < parsed.len() {
        if let ParsedApplyOperation::Unsupported(index) = &parsed[cursor] {
            return Err(hidden_apply_family_unimplemented(*index));
        }
        let end = parsed[cursor..]
            .iter()
            .take_while(|operation| operation.same_family(&parsed[cursor]))
            .count()
            + cursor;
        match &parsed[cursor] {
            ParsedApplyOperation::Code(_) => {
                let operations = parsed[cursor..end]
                    .iter()
                    .map(|operation| match operation {
                        ParsedApplyOperation::Code(operation) => operation.operation().clone(),
                        _ => unreachable!("the selected run is code-only"),
                    })
                    .collect::<Vec<_>>();
                let before = staged
                    .planned_changes()
                    .into_iter()
                    .map(|change| change.relative_path)
                    .collect::<Vec<_>>();
                let authority = admission.code_planning_authority(binding)?;
                let planned = crate::infrastructure::native_operations::code::plan_code_batch(
                    staged,
                    authority,
                    &operations,
                )?;
                staged = planned.0;
                provisional.extend(batch_effects_as_provisional(
                    &before, &staged, planned.1, cursor,
                ));
            }
            ParsedApplyOperation::Xdto(_) => {
                let operations = parsed[cursor..end]
                    .iter()
                    .map(|operation| match operation {
                        ParsedApplyOperation::Xdto(operation) => operation.operation().clone(),
                        _ => unreachable!("the selected run is XDTO-only"),
                    })
                    .collect::<Vec<_>>();
                let before = staged
                    .planned_changes()
                    .into_iter()
                    .map(|change| change.relative_path)
                    .collect::<Vec<_>>();
                let authority = admission.xdto_planning_authority(binding)?;
                let planned = crate::infrastructure::native_operations::xdto::plan_xdto_batch(
                    staged,
                    authority,
                    &operations,
                )?;
                staged = planned.0;
                provisional.extend(batch_effects_as_provisional(
                    &before, &staged, planned.1, cursor,
                ));
            }
            ParsedApplyOperation::Metadata(_) => {
                let operations = parsed[cursor..end]
                    .iter()
                    .map(|operation| match operation {
                        ParsedApplyOperation::Metadata(operation) => operation.clone(),
                        _ => unreachable!("the selected run is metadata-only"),
                    })
                    .collect::<Vec<_>>();
                let authority = admission.metadata_planning_authority(binding)?;
                let planned = metadata::plan_metadata_batch(staged, authority, &operations)?;
                staged = planned.0;
                provisional.extend(planned.1);
            }
            ParsedApplyOperation::FormResource(_) => {
                let operations = parsed[cursor..end]
                    .iter()
                    .map(|operation| match operation {
                        ParsedApplyOperation::FormResource(operation) => operation.clone(),
                        _ => unreachable!("the selected run is form/resource-only"),
                    })
                    .collect::<Vec<_>>();
                let authority = admission.form_resource_planning_authority(binding)?;
                let planned =
                    form_resource::plan_form_resource_batch(staged, authority, &operations)?;
                staged = planned.0;
                provisional.extend(planned.1);
            }
            ParsedApplyOperation::DcsMxl(_) => {
                let operations = parsed[cursor..end]
                    .iter()
                    .map(|operation| match operation {
                        ParsedApplyOperation::DcsMxl(operation) => operation.clone(),
                        _ => unreachable!("the selected run is DCS/MXL-only"),
                    })
                    .collect::<Vec<_>>();
                let authority = admission.dcs_mxl_planning_authority(binding)?;
                let planned = dcs_mxl::plan_dcs_mxl_batch(staged, authority, &operations)?;
                staged = planned.0;
                provisional.extend(planned.1);
            }
            ParsedApplyOperation::Unsupported(_) => unreachable!("unsupported run handled above"),
        }
        cursor = end;
    }

    let effects = reconcile_effects(&staged, provisional);
    Ok((staged, effects))
}

#[cfg(test)]
mod tests {
    use super::request::{reconcile_effects, ProvisionalApplyEffect};
    use crate::domain::cancellation::CancellationToken;
    use crate::domain::code_intelligence::ProviderDeadline;
    use crate::domain::project_sources::{SourceFormat, SourceProfile, SourceSetKind};
    use crate::domain::workspace::WorkspaceContext;
    use crate::infrastructure::workspace_actor::{
        ApplyAdmission, ProviderRootBinding, WorkspaceActor, WorkspaceIdentity,
        WorkspaceSourceSetInput,
    };
    use std::sync::Arc;
    use std::time::Duration;

    pub(super) struct ApplySeamFixture {
        _root: tempfile::TempDir,
        actor: Arc<WorkspaceActor>,
        pub(super) binding: ProviderRootBinding,
    }

    impl ApplySeamFixture {
        pub(super) fn new() -> Self {
            let root = tempfile::tempdir().unwrap();
            let source = root.path().join("src");
            std::fs::create_dir_all(source.join("Documents")).unwrap();
            std::fs::write(
                root.path().join("v8project.yaml"),
                "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
            )
            .unwrap();
            std::fs::write(
                source.join("Configuration.xml"),
                r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Configuration><Properties><Name>Main</Name></Properties><ChildObjects><Document>First</Document><Document>Second</Document></ChildObjects></Configuration></MetaDataObject>"#,
            )
            .unwrap();
            for (name, comment) in [("First", ""), ("Second", "")] {
                std::fs::write(
                    source.join(format!("Documents/{name}.xml")),
                    format!(
                        r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" xmlns:v8="http://v8.1c.ru/8.1/data/core" version="2.20"><Document uuid="11111111-1111-4111-8111-111111111111"><Properties><Name>{name}</Name><Synonym/><Comment>{comment}</Comment></Properties><ChildObjects><Attribute uuid="22222222-2222-4222-8222-222222222222"><Properties><Name>Total</Name><Comment>original</Comment></Properties><ChildObjects/></Attribute></ChildObjects></Document></MetaDataObject>"#
                    ),
                )
                .unwrap();
            }
            let workspace_root = std::fs::canonicalize(root.path()).unwrap();
            let source = std::fs::canonicalize(source).unwrap();
            let context = WorkspaceContext {
                cwd: workspace_root.clone(),
                workspace_root: workspace_root.clone(),
                cache_root: workspace_root.join(".build/unica"),
                workspace_epoch: 1,
            };
            let identity = WorkspaceIdentity::new(
                &context,
                [WorkspaceSourceSetInput::new(
                    "main",
                    &source,
                    SourceSetKind::Configuration,
                    SourceFormat::PlatformXml,
                    SourceProfile::platform_xml_8_3_27_format_2_20(),
                )],
                "apply-family-seam-test",
            )
            .unwrap();
            let actor = Arc::new(WorkspaceActor::new(identity, context).unwrap());
            let binding = actor.bind_provider_root("main", &source).unwrap();
            Self {
                _root: root,
                actor,
                binding,
            }
        }

        pub(super) fn admission(&self) -> ApplyAdmission {
            self.actor
                .admit_apply(
                    &self.binding,
                    None,
                    true,
                    ProviderDeadline::from_budget(Duration::from_secs(5)),
                    &CancellationToken::new(),
                )
                .unwrap()
        }
    }

    #[test]
    fn request_level_reconciliation_drops_cancelled_effect_before_deduplication() {
        let fixture = ApplySeamFixture::new();
        let admission = fixture.admission();
        let mut staged = admission.staged_state().unwrap();
        let second_path = std::path::Path::new("Documents/Second.xml");
        let second_preimage = staged.read(second_path).unwrap().unwrap();
        let second_postimage = String::from_utf8(second_preimage.clone())
            .unwrap()
            .replace("<Comment></Comment>", "<Comment>survives</Comment>")
            .into_bytes();
        staged
            .replace(second_path, &second_preimage, second_postimage)
            .unwrap();

        assert_eq!(
            staged
                .planned_changes()
                .iter()
                .map(|change| change.relative_path.to_string_lossy().into_owned())
                .collect::<Vec<_>>(),
            ["Documents/Second.xml"]
        );
        let effects = reconcile_effects(
            &staged,
            vec![
                ProvisionalApplyEffect::single(
                    "Documents/First.xml",
                    crate::domain::events::DomainEvent::new(
                        crate::domain::events::DomainEventKind::MetadataChanged,
                        "shared",
                    ),
                    0,
                ),
                ProvisionalApplyEffect::single(
                    second_path,
                    crate::domain::events::DomainEvent::new(
                        crate::domain::events::DomainEventKind::SourceSetChanged,
                        "shared",
                    ),
                    1,
                ),
            ],
        );
        assert_eq!(
            effects
                .events()
                .iter()
                .map(|event| (event.kind, event.artifact.as_str()))
                .collect::<Vec<_>>(),
            [(
                crate::domain::events::DomainEventKind::SourceSetChanged,
                "shared"
            )]
        );
    }
}
