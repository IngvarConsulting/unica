use super::common::{code_patch_source_target, guard_code_patch_resolved_target};
use crate::application::SupportGuardRequirement;
use crate::infrastructure::platform_xml_source_targets::{
    resolve_platform_xml_target, revalidate_platform_xml_target, ClosedPlatformXmlTarget,
    TargetKindPolicy,
};
use crate::infrastructure::support_guard::{
    evaluate_resolved_support_guard, ResolvedSupportGuardCheck,
};
use serde_json::{Map, Value};
use std::fs;
use std::path::PathBuf;
use unica_address::source_target::ResolvedTarget;
use unica_bsl::input::*;
#[cfg(test)]
use unica_bsl::text::test_support::arguments;
use unica_bsl::text::*;
use unica_execution_contract::adapter_outcome::AdapterOutcome;
use unica_metadata_designer::borrowed_image::stage_borrowed_module_state;
use unica_metadata_designer::code_support::prove_staged_code_support;
use unica_source_contract::events::{DomainEvent, DomainEventKind};
use unica_source_contract::proposal::{
    ApplyPlanError, ApplyPlanErrorKind, ApplyProposal, ApplySourceAuthority, ApplyStagingError,
    PlannedApplyEffects,
};
use unica_source_contract::workspace::WorkspaceContext;
use unica_source_designer::code_owner::prove_staged_code_owner;
use unica_source_designer::module_layout::{
    module_relative, module_source_address, platform_xml_module_identity,
};
use unica_workspace::compile_transaction::CompileTransaction;

pub(crate) fn apply_with_data(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> CodePatchExecution {
    patch_inner(args, context, PatchMode::Apply)
}

pub(crate) fn preview_with_data(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> CodePatchExecution {
    patch_inner(args, context, PatchMode::Preview)
}

pub(crate) struct CodePatchExecution {
    pub(crate) outcome: AdapterOutcome,
    pub(crate) data: Option<CodePatchData>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum PatchMode {
    Apply,
    Preview,
}

/// Plans a run of code operations; `first_index` is the place of the first
/// of them in the call's `ops`, so a refusal names the operation the caller
/// sent.
pub(crate) fn plan_code_batch(
    staged: &mut ApplyProposal<'_>,
    authority: &dyn ApplySourceAuthority,
    operations: &[CodePlanOperation],
    first_index: usize,
) -> Result<PlannedApplyEffects, ApplyPlanError> {
    if !authority.owns_staged_state(staged) {
        return Err(ApplyPlanError::new(
            ApplyPlanErrorKind::InvalidState,
            "staged source belongs to another apply admission",
        ));
    }
    let mut provisional_effects = Vec::new();
    for (op_index, operation) in operations.iter().enumerate() {
        plan_one_code_operation(
            staged,
            &mut provisional_effects,
            authority,
            operation,
            first_index + op_index,
        )?;
    }
    let final_changes = staged.planned_changes();
    let mut effects = PlannedApplyEffects::default();
    for (relative, event) in provisional_effects {
        if final_changes
            .iter()
            .any(|change| change.relative_path == relative)
        {
            effects.append_at(event, vec![relative]);
        }
    }
    Ok(effects)
}

fn plan_one_code_operation(
    staged: &mut ApplyProposal<'_>,
    effects: &mut Vec<(PathBuf, DomainEvent)>,
    authority: &dyn ApplySourceAuthority,
    operation: &CodePlanOperation,
    op_index: usize,
) -> Result<(), ApplyPlanError> {
    let at = code_operation_at(operation);
    let module_at = code_projection(at).map_or_else(|| at.clone(), |(module, _)| module);
    let at_path = format!("ops[{op_index}].args.at");
    if at.source_set() != authority.source_set_name() {
        return Err(bad_code_arg(
            &at_path,
            "at belongs to another actor-admitted source set",
        ));
    }
    let capability =
        crate::domain::source_capabilities::bsl_profile_for_source(authority.source_profile())
            .and_then(|profile| profile.module_capability(&module_at))
            .ok_or_else(|| {
                ApplyPlanError::new(
                    ApplyPlanErrorKind::InvalidSource,
                    "at does not identify one exact writable module terminal",
                )
                .at_path(&at_path)
            })?;
    let target = module_source_address(
        &module_at,
        capability.owner_kind(),
        capability.role(),
        capability.route_kind(),
    )
    .map_err(|_| {
        ApplyPlanError::new(
            ApplyPlanErrorKind::ProviderUnavailable,
            "module layout is unavailable for the actor-issued profile",
        )
        .at_path(&at_path)
    })?;
    let relative = module_relative(&target, authority.source_kind()).map_err(|_| {
        ApplyPlanError::new(
            ApplyPlanErrorKind::ProviderUnavailable,
            "module source layout is unavailable for the actor-issued source kind",
        )
        .at_path(&at_path)
    })?;
    let identity = platform_xml_module_identity(&relative).map_err(|_| {
        ApplyPlanError::new(
            ApplyPlanErrorKind::InvalidSource,
            "module layout does not round-trip through the Platform XML mapper",
        )
        .at_path(&at_path)
    })?;
    if identity.address != target {
        return Err(ApplyPlanError::new(
            ApplyPlanErrorKind::Postcondition,
            "module layout identity does not match its logical target",
        )
        .at_path(&at_path));
    }

    let owner_bytes = prove_staged_code_owner(
        staged,
        authority.source_kind(),
        authority.expected_format(),
        &identity,
        &at_path,
    )?;
    prove_staged_code_support(
        staged,
        authority.support_policy_mode(),
        &owner_bytes,
        &at_path,
    )?;
    let before = staged
        .read(&relative)
        .map_err(|error| staged_code_error(error, &at_path))?;
    let postimage =
        plan_code_postimage(before.as_deref().unwrap_or_default(), operation, op_index)?;
    stage_borrowed_module_state(staged, effects, authority, &identity, &at_path)?;
    if postimage.no_op {
        return Ok(());
    }
    match before.as_deref() {
        Some(before) => staged
            .replace(&relative, before, postimage.after.clone())
            .map_err(|error| staged_code_error(error, &at_path))?,
        None => staged
            .create(&relative, postimage.after.clone())
            .map_err(|error| staged_code_error(error, &at_path))?,
    }
    if staged
        .read(&relative)
        .map_err(|error| staged_code_error(error, &at_path))?
        .as_deref()
        != Some(postimage.after.as_slice())
    {
        return Err(ApplyPlanError::new(
            ApplyPlanErrorKind::Postcondition,
            "staged module postimage did not remain exact",
        )
        .at_path(at_path));
    }
    effects.push((
        relative,
        DomainEvent::new(DomainEventKind::ModuleChanged, module_at.to_string()),
    ));
    Ok(())
}

fn staged_code_error(error: ApplyStagingError, path: &str) -> ApplyPlanError {
    ApplyPlanError::staging(error, path)
}

fn patch_inner(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
    mode: PatchMode,
) -> CodePatchExecution {
    match build_patch(args, context) {
        Ok(plan) => finish_patch(plan, mode, context),
        Err(error) => CodePatchExecution::failure(error, None),
    }
}

struct CodePatchPlan {
    target: CodePatchTarget,
    text: CodeTextPatchPlan,
}

fn build_patch(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> Result<CodePatchPlan, String> {
    let target = resolve_target(args, context)?;
    // A module the platform never exported reads as no bytes at all, which is
    // exactly the preimage a first body is appended to.
    let before = if target.existed {
        fs::read(&target.path)
            .map_err(|error| format!("failed to read {}: {error}", target.path.display()))?
    } else {
        Vec::new()
    };
    let text = plan_legacy_patch(args, before)?;
    Ok(CodePatchPlan { target, text })
}

fn finish_patch(
    plan: CodePatchPlan,
    mode: PatchMode,
    context: &WorkspaceContext,
) -> CodePatchExecution {
    let CodePatchPlan { target, text: plan } = plan;
    let data = match validate_legacy_patch(
        &plan,
        CodePatchIdentity {
            source_set: &target.resolved.source_set,
            metadata_path: target.metadata_path(),
            target_kind: target.resolved.target_kind,
            owner: &target.owner,
            module_role: &target.module_role,
        },
    ) {
        Ok(data) => data,
        Err(failure) => {
            return CodePatchExecution::failure(failure.message, failure.data.map(|data| *data))
        }
    };
    if mode == PatchMode::Apply && !plan.no_op {
        let publish_result = (|| -> Result<(), String> {
            let mut transaction = CompileTransaction::new();
            if target.existed {
                transaction.replace_bytes(&target.path, &plan.before, plan.after.clone())?;
            } else {
                // `create_bytes` refuses if the file appeared meanwhile, so a
                // concurrent creator is not overwritten either.
                transaction.create_bytes(&target.path, plan.after.clone())?;
            }
            let revalidated =
                guard_code_patch_resolved_target(&mut transaction, &target.handle, context)?;
            if revalidated != target.path {
                return Err("resolved code.patch target changed before publication".to_string());
            }
            guard_resolved_support(&revalidated, context)?;
            transaction.commit()?;
            Ok(())
        })();
        if let Err(error) = publish_result {
            return CodePatchExecution::failure(format!("publish BSL module: {error}"), Some(data));
        }
    }
    let (edit, verb) = match plan.site {
        PatchSite::Insertion(_) => ("insertion", "inserted"),
        PatchSite::Replacement(_) => ("replacement", "replaced"),
    };
    let outcome = AdapterOutcome {
        ok: true,
        summary: if plan.no_op {
            "unica.code.patch is already applied".to_string()
        } else if mode == PatchMode::Preview {
            format!("dry run: unica.code.patch planned one {edit}")
        } else {
            format!("unica.code.patch applied one {edit}")
        },
        changes: (mode == PatchMode::Apply && !plan.no_op)
            .then(|| {
                format!(
                    "{} + {}: {verb} BSL content",
                    target.resolved.source_set,
                    target.metadata_path()
                )
            })
            .into_iter()
            .collect(),
        warnings: Vec::new(),
        errors: Vec::new(),
        artifacts: vec![format!(
            "{} + {}",
            target.resolved.source_set,
            target.metadata_path()
        )],
        stdout: None,
        stderr: None,
        command: None,
    };
    CodePatchExecution {
        outcome,
        data: Some(data),
    }
}

impl CodePatchExecution {
    fn failure(error: String, data: Option<CodePatchData>) -> Self {
        Self {
            outcome: AdapterOutcome {
                ok: false,
                summary: "unica.code.patch failed".to_string(),
                changes: Vec::new(),
                warnings: Vec::new(),
                errors: vec![error.clone()],
                artifacts: Vec::new(),
                stdout: None,
                stderr: Some(format!("{error}\n")),
                command: None,
            },
            data,
        }
    }
}

struct CodePatchTarget {
    path: PathBuf,
    resolved: ResolvedTarget,
    handle: ClosedPlatformXmlTarget,
    owner: String,
    module_role: String,
    /// Whether the module file existed when the patch was planned. The platform
    /// omits an empty object or manager module on export, so a legitimate
    /// address can point at a file that is not there yet.
    existed: bool,
}

impl CodePatchTarget {
    fn metadata_path(&self) -> &str {
        self.resolved
            .metadata_path
            .as_ref()
            .expect("code.patch resolves one module metadataPath")
            .as_str()
    }
}

fn resolve_target(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> Result<CodePatchTarget, String> {
    let source_target = code_patch_source_target(args).map_err(|error| error.to_string())?;
    // `ModuleOnly` is the write surface's declaration: a descriptor or any other
    // metadata object address is refused here, not deeper.
    let resolution = resolve_platform_xml_target(
        context,
        &source_target,
        TargetKindPolicy::ModuleOnlyAllowingAbsent,
    )
    .map_err(|error| error.to_string())?;
    let path = revalidate_platform_xml_target(context, &resolution.handle)
        .map_err(|error| error.to_string())?
        .path;
    let mut authorization = CompileTransaction::new();
    let guarded_path =
        guard_code_patch_resolved_target(&mut authorization, &resolution.handle, context)?;
    if guarded_path != path {
        return Err("resolved code.patch target changed during planning".to_string());
    }
    guard_resolved_support(&path, context)?;
    let metadata_path = resolution
        .resolved
        .metadata_path
        .as_ref()
        .expect("module resolver returns metadataPath");
    let segments = metadata_path.segments().collect::<Vec<_>>();
    let (owner, module_role) = match segments.as_slice() {
        [module_role] => ("Configuration".to_string(), (*module_role).to_string()),
        [kind, name, .., module_role] => (format!("{kind}.{name}"), (*module_role).to_string()),
        _ => {
            return Err("resolved module metadataPath has an invalid shape".to_string());
        }
    };
    let existed = path.is_file();
    Ok(CodePatchTarget {
        path,
        resolved: resolution.resolved,
        handle: resolution.handle,
        owner,
        module_role,
        existed,
    })
}

fn guard_resolved_support(
    target: &std::path::Path,
    context: &WorkspaceContext,
) -> Result<(), String> {
    match evaluate_resolved_support_guard(target, SupportGuardRequirement::Editable, context) {
        ResolvedSupportGuardCheck::Allow | ResolvedSupportGuardCheck::Warn(_) => Ok(()),
        ResolvedSupportGuardCheck::Block(violation) => {
            Err(format!("support guard: {}", violation.reason))
        }
    }
}

#[cfg(test)]
#[path = "../../../tests/platform/code_module_state.rs"]
mod module_state_tests;

#[cfg(test)]
pub(crate) fn plan_code_batch_owned_for_test(
    mut staged: unica_workspace::apply::ApplyStagedState,
    authority: unica_workspace::actor::CodeApplyAuthority<'_>,
    operations: &[CodePlanOperation],
) -> Result<
    (
        unica_workspace::apply::ApplyStagedState,
        PlannedApplyEffects,
    ),
    ApplyPlanError,
> {
    let effects = plan_code_batch(&mut staged.proposal(), &authority, operations, 0)?;
    Ok((staged, effects))
}

#[cfg(test)]
#[path = "../../../tests/platform/code_patch_composition.rs"]
pub(super) mod tests;
