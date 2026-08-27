pub(crate) mod dcs_mxl;
pub(crate) mod form_resource;
pub(crate) mod metadata;

use crate::domain::apply::{ApplyRequest, OperationFamily};
use crate::domain::project_sources::{SourceFormat, SourceSetKind};
use crate::infrastructure::native_operations::apply::{
    hidden_apply_family_unimplemented, ApplyPlanError, ApplyPlanErrorKind, ApplyStagedState,
    PlannedApplyEffects,
};
use crate::infrastructure::workspace_actor::{ApplyAdmission, ProviderRootBinding};

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
    let mut staged = admission
        .staged_state()
        .map_err(|error| ApplyPlanError::staging(error, "ops"))?;
    let mut accumulated = PlannedApplyEffects::default();

    for (op_index, operation) in request.ops().iter().enumerate() {
        let Some(family) = crate::application::v13::apply::dispatch_family(operation.name()) else {
            return Err(hidden_apply_family_unimplemented(op_index));
        };
        let args = serde_json::Value::Object(operation.args().clone());
        let planned = match family {
            OperationFamily::Metadata | OperationFamily::Properties => {
                let parsed = metadata::parse_metadata_plan_operation(
                    operation.name(),
                    &args,
                    op_index,
                    binding,
                )?;
                let authority = admission.metadata_planning_authority(binding)?;
                metadata::plan_metadata_batch(staged, authority, &[parsed])
            }
            OperationFamily::Form
            | OperationFamily::Role
            | OperationFamily::Xdto
            | OperationFamily::Subsystem
            | OperationFamily::Support => {
                let parsed = form_resource::parse_form_resource_plan_operation(
                    operation.name(),
                    &args,
                    op_index,
                    binding,
                )?;
                let authority = admission.form_resource_planning_authority(binding)?;
                form_resource::plan_form_resource_batch(staged, authority, &[parsed])
            }
            OperationFamily::Dcs | OperationFamily::Mxl => {
                let parsed = dcs_mxl::parse_dcs_mxl_plan_operation(
                    operation.name(),
                    &args,
                    op_index,
                    binding,
                )?;
                let authority = admission.dcs_mxl_planning_authority(binding)?;
                dcs_mxl::plan_dcs_mxl_batch(staged, authority, &[parsed])
            }
            OperationFamily::Code | OperationFamily::Event => {
                return Err(hidden_apply_family_unimplemented(op_index));
            }
        }?;
        staged = planned.0;
        for event in planned.1.into_events() {
            accumulated.append(event);
        }
    }

    Ok((staged, accumulated))
}

#[cfg(test)]
mod tests {
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
            std::fs::create_dir_all(&source).unwrap();
            std::fs::write(
                root.path().join("v8project.yaml"),
                "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
            )
            .unwrap();
            std::fs::write(
                source.join("Configuration.xml"),
                r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Configuration><Properties><Name>Main</Name></Properties><ChildObjects/></Configuration></MetaDataObject>"#,
            )
            .unwrap();
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
}
