use crate::application::operation_descriptors::{
    native_operation_descriptor, SupportGuardPolicy, SupportGuardRequirement,
};
use crate::application::ports::SupportGuardCheck;
use crate::application::{AdapterOutcome, ToolHandler, ToolSpec};
use crate::domain::workspace::WorkspaceContext;
use crate::infrastructure::native_operations::common::{
    absolutize, path_arg, required_string, resolve_code_patch_guard_path, support_guard_violation,
    SupportGuardViolation,
};
use crate::infrastructure::native_operations::compile_transaction::CompileTransaction;
use crate::infrastructure::native_operations::role::resolve_role_edit_guard_path;
use crate::infrastructure::native_operations::support;
use crate::infrastructure::native_operations::template;
use crate::infrastructure::native_operations::xdto::resolve_xdto_guard_path;
use crate::infrastructure::support_policy_evidence::{
    support_policy_mode_from_bytes, v8_project_candidates_for_directory, SupportPolicyMode,
};
use serde_json::{Map, Value};
use std::path::{Path, PathBuf};

pub(crate) enum ResolvedSupportGuardCheck {
    Allow,
    Warn(String),
    Block(SupportGuardViolation),
}

pub(crate) fn evaluate_support_guard(
    spec: ToolSpec,
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> Result<SupportGuardCheck, String> {
    if matches!(
        spec.handler,
        ToolHandler::NativeOperation {
            operation: "support-edit",
            ..
        }
    ) {
        if let Err(error) = support::preflight_support_edit_capability(args, context) {
            return Ok(SupportGuardCheck::Block(AdapterOutcome {
                ok: false,
                summary: "support-edit failed".to_string(),
                changes: Vec::new(),
                warnings: Vec::new(),
                errors: vec![error],
                artifacts: Vec::new(),
                stdout: None,
                stderr: None,
                command: None,
            }));
        }
    }
    let Some((target_path, requirement)) = support_guard_target(spec, args, context) else {
        return Ok(SupportGuardCheck::Allow);
    };
    Ok(
        match evaluate_resolved_support_guard(&target_path, requirement, context) {
            ResolvedSupportGuardCheck::Allow => SupportGuardCheck::Allow,
            ResolvedSupportGuardCheck::Warn(warning) => SupportGuardCheck::Warn(warning),
            ResolvedSupportGuardCheck::Block(violation) => SupportGuardCheck::Block(
                support_guard_blocked_outcome(spec, &violation, requirement),
            ),
        },
    )
}

pub(crate) fn evaluate_resolved_support_guard(
    target_path: &Path,
    requirement: SupportGuardRequirement,
    context: &WorkspaceContext,
) -> ResolvedSupportGuardCheck {
    let Some(violation) = support_guard_violation(target_path, requirement) else {
        return ResolvedSupportGuardCheck::Allow;
    };
    match support_guard_mode(&violation.config_dir, context) {
        SupportPolicyMode::Off => ResolvedSupportGuardCheck::Allow,
        SupportPolicyMode::Warn => ResolvedSupportGuardCheck::Warn(format!(
            "[support guard] ПРЕДУПРЕЖДЕНИЕ: {}. Цель: {}",
            violation.reason,
            violation.target_path.display()
        )),
        SupportPolicyMode::Deny => ResolvedSupportGuardCheck::Block(violation),
    }
}

/// Binds the exact support-state and project-policy inputs used by
/// `evaluate_resolved_support_guard`. Missing files are evidence too: a
/// concurrently appearing vendor-support file or nearer project policy must
/// invalidate a prepared mutation instead of silently changing authorization.
pub(crate) fn bind_resolved_support_guard_evidence(
    transaction: &mut CompileTransaction,
    target_path: &Path,
    context: &WorkspaceContext,
) -> Result<(), String> {
    if let Some(config_dir) =
        crate::infrastructure::native_operations::common::find_support_config_dir(target_path)
    {
        let support = config_dir.join("Ext/ParentConfigurations.bin");
        bind_optional_file(transaction, &support)?;
    }

    let config_dir =
        crate::infrastructure::native_operations::common::find_support_config_dir(target_path);
    let starts = [
        Some(context.cwd.as_path()),
        config_dir.as_deref(),
        Some(context.workspace_root.as_path()),
    ];
    for start in starts.into_iter().flatten() {
        for candidate in v8_project_candidates(start) {
            if candidate.is_file() {
                let bytes = std::fs::read(&candidate)
                    .map_err(|error| format!("failed to read support policy evidence: {error}"))?;
                transaction.guard_or_verify_exact_preimage(candidate, bytes)?;
                return Ok(());
            }
            match std::fs::symlink_metadata(&candidate) {
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    transaction.guard_path_absent(candidate)?;
                }
                Ok(_) => return Err("support policy evidence is not a regular file".to_string()),
                Err(error) => {
                    return Err(format!(
                        "failed to inspect support policy evidence: {error}"
                    ))
                }
            }
        }
    }
    Ok(())
}

fn bind_optional_file(transaction: &mut CompileTransaction, path: &Path) -> Result<(), String> {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_file() && !metadata.file_type().is_symlink() => {
            let bytes = std::fs::read(path)
                .map_err(|error| format!("failed to read support evidence: {error}"))?;
            transaction.guard_or_verify_exact_preimage(path, bytes)
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            transaction.guard_path_absent(path)
        }
        Ok(_) => Err("support evidence is not a regular file".to_string()),
        Err(error) => Err(format!("failed to inspect support evidence: {error}")),
    }
}

fn v8_project_candidates(start: &Path) -> Vec<PathBuf> {
    let current = if start.is_dir() {
        start.to_path_buf()
    } else {
        start.parent().unwrap_or(start).to_path_buf()
    };
    v8_project_candidates_for_directory(&current)
}

fn support_guard_target(
    spec: ToolSpec,
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> Option<(PathBuf, SupportGuardRequirement)> {
    let ToolHandler::NativeOperation { operation, .. } = spec.handler else {
        return None;
    };
    let policy = native_operation_descriptor(operation)?.support_guard?;
    match policy {
        SupportGuardPolicy::HandlerResolved { requirement } => {
            let resolved = match operation {
                "xdto-edit" => resolve_xdto_guard_path(args, context).ok(),
                "role-edit" => resolve_role_edit_guard_path(args, context).ok(),
                _ => resolve_code_patch_guard_path(args, context).ok(),
            };
            resolved.map(|path| (path, requirement))
        }
        SupportGuardPolicy::PathArgs { names, requirement } => {
            support_guard_path_arg(args, context, names, requirement)
        }
        SupportGuardPolicy::ObjectName { requirement } => {
            support_guard_object_name_target(args, context).map(|path| (path, requirement))
        }
    }
}

fn support_guard_path_arg(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
    names: &[&str],
    requirement: SupportGuardRequirement,
) -> Option<(PathBuf, SupportGuardRequirement)> {
    path_arg(args, names).map(|path| (absolutize(path, &context.cwd), requirement))
}

fn support_guard_object_name_target(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> Option<PathBuf> {
    let object_name = required_string(
        args,
        &["objectName", "ObjectName", "processorName", "ProcessorName"],
        "ObjectName",
    )
    .ok()?;
    let src_dir = path_arg(args, &["srcDir", "SrcDir"]).unwrap_or_else(|| PathBuf::from("src"));
    let src_dir = absolutize(src_dir, &context.cwd);
    let direct = src_dir.join(format!("{object_name}.xml"));
    if direct.exists() {
        return Some(direct);
    }
    for folder in template::template_add_object_type_folders() {
        let candidate = src_dir.join(folder).join(format!("{object_name}.xml"));
        if candidate.exists() {
            return Some(candidate);
        }
    }
    Some(direct)
}

fn support_guard_mode(config_dir: &Path, context: &WorkspaceContext) -> SupportPolicyMode {
    let Some(project_file) = find_v8_project_file(&context.cwd)
        .or_else(|| find_v8_project_file(config_dir))
        .or_else(|| find_v8_project_file(&context.workspace_root))
    else {
        return SupportPolicyMode::Deny;
    };
    let Ok(bytes) = std::fs::read(&project_file) else {
        return SupportPolicyMode::Deny;
    };
    let project_dir = project_file.parent().unwrap_or_else(|| Path::new(""));
    support_policy_mode_from_bytes(&bytes, project_dir, config_dir)
}

fn find_v8_project_file(start: &Path) -> Option<PathBuf> {
    v8_project_candidates(start)
        .into_iter()
        .find(|candidate| candidate.is_file())
}

fn support_guard_blocked_outcome(
    spec: ToolSpec,
    violation: &SupportGuardViolation,
    requirement: SupportGuardRequirement,
) -> AdapterOutcome {
    let target = violation.target_path.display();
    let head = "[support-guard] Редактирование отклонено: это объект типовой конфигурации на поддержке поставщика, прямое редактирование молча сломает будущие обновления.";
    let cfe = "Рекомендуемый путь: внести доработку в расширение (навыки cfe-borrow / cfe-patch-method) — состояние поддержки менять не нужно, обновления вендора сохраняются.";
    let off_note =
        "Снять проверку для этой базы: editingAllowedCheck = warn|off в .v8-project.json.";
    let (state, fix) = match violation.code {
        "capability-off" => (
            format!(
                "Состояние: у всей конфигурации выключена возможность изменения (режим read-only «из коробки») — поэтому объект «{target}» редактировать нельзя."
            ),
            format!(
                "Либо снять защиту явно (навык support-edit, два шага):\n  support-edit -Path \"{}\" -Capability on — включить возможность изменения (объекты пока остаются на замке);\n  support-edit -Path \"{target}\" -Set editable — открыть этот объект для редактирования.\n  Изменение применяется в базу полной загрузкой выгрузки и обходит механизм обновлений вендора.",
                violation.config_dir.display()
            ),
        ),
        "not-removed" if requirement == SupportGuardRequirement::Removed => (
            format!(
                "Состояние: объект «{target}» на поддержке (не снят с поддержки) — его удаление разорвёт обновления вендора."
            ),
            format!(
                "Либо сначала снять объект с поддержки, затем удалять:\n  support-edit -Path \"{target}\" -Set off-support — объект уходит из-под обновлений, после этого удаление безопасно."
            ),
        ),
        _ => (
            format!(
                "Состояние: объект «{target}» на замке (возможность изменения конфигурации включена, но сам объект не редактируется)."
            ),
            format!(
                "Либо разрешить редактирование этого объекта (навык support-edit, выбрать одно):\n  support-edit -Path \"{target}\" -Set editable — редактировать и дальше получать обновления вендора (возможны конфликты слияния);\n  support-edit -Path \"{target}\" -Set off-support — снять с поддержки: обновления по объекту больше не приходят."
            ),
        ),
    };
    let message = format!("{head}\n{state}\n{cfe}\n{fix}\n{off_note}");
    AdapterOutcome {
        ok: false,
        summary: format!("{} blocked by support guard", spec.name),
        changes: Vec::new(),
        warnings: Vec::new(),
        errors: vec![message.clone()],
        artifacts: vec![violation.target_path.display().to_string()],
        stdout: None,
        stderr: Some(format!("{message}\n")),
        command: None,
    }
}

#[cfg(test)]
mod tests {
    use super::support_guard_mode;
    use crate::domain::workspace::WorkspaceContext;
    use crate::infrastructure::support_policy_evidence::{
        support_policy_mode_value, SupportPolicyMode,
    };

    #[test]
    fn project_editing_policy_is_the_closed_support_guard_downgrade_source() {
        assert_eq!(support_policy_mode_value("warn"), SupportPolicyMode::Warn);
        assert_eq!(support_policy_mode_value("off"), SupportPolicyMode::Off);
        for value in ["deny", "", "WARN", "unsupported"] {
            assert_eq!(
                support_policy_mode_value(value),
                SupportPolicyMode::Deny,
                "{value}"
            );
        }

        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path();
        let cwd = root.join("caller/deep");
        let config = root.join("config/src");
        let workspace = root.join("workspace");
        std::fs::create_dir_all(&cwd).unwrap();
        std::fs::create_dir_all(&config).unwrap();
        std::fs::create_dir_all(&workspace).unwrap();
        let context = WorkspaceContext {
            cwd: cwd.clone(),
            workspace_root: workspace.clone(),
            cache_root: workspace.join(".build/unica"),
            workspace_epoch: 1,
        };
        assert_eq!(
            support_guard_mode(&config, &context),
            SupportPolicyMode::Deny
        );

        let policy_paths = [
            cwd.parent().unwrap().join(".v8-project.json"),
            config.parent().unwrap().join(".v8-project.json"),
            workspace.join(".v8-project.json"),
        ];
        let values = [
            ("deny", SupportPolicyMode::Deny),
            ("warn", SupportPolicyMode::Warn),
            ("off", SupportPolicyMode::Off),
        ];
        for (cwd_value, cwd_expected) in values {
            for (config_value, _) in values {
                for (workspace_value, _) in values {
                    for (path, value) in
                        policy_paths
                            .iter()
                            .zip([cwd_value, config_value, workspace_value])
                    {
                        std::fs::write(path, format!(r#"{{"editingAllowedCheck":"{value}"}}"#))
                            .unwrap();
                    }
                    assert_eq!(
                        support_guard_mode(&config, &context),
                        cwd_expected,
                        "cwd={cwd_value}, config={config_value}, workspace={workspace_value}"
                    );
                }
            }
        }

        std::fs::remove_file(&policy_paths[0]).unwrap();
        for (config_value, config_expected) in values {
            for (workspace_value, _) in values {
                std::fs::write(
                    &policy_paths[1],
                    format!(r#"{{"editingAllowedCheck":"{config_value}"}}"#),
                )
                .unwrap();
                std::fs::write(
                    &policy_paths[2],
                    format!(r#"{{"editingAllowedCheck":"{workspace_value}"}}"#),
                )
                .unwrap();
                assert_eq!(
                    support_guard_mode(&config, &context),
                    config_expected,
                    "config={config_value}, workspace={workspace_value}"
                );
            }
        }

        std::fs::remove_file(&policy_paths[1]).unwrap();
        for (workspace_value, expected) in values {
            std::fs::write(
                &policy_paths[2],
                format!(r#"{{"editingAllowedCheck":"{workspace_value}"}}"#),
            )
            .unwrap();
            assert_eq!(
                support_guard_mode(&config, &context),
                expected,
                "workspace={workspace_value}"
            );
        }

        std::fs::write(
            &policy_paths[2],
            serde_json::to_vec(&serde_json::json!({
                "editingAllowedCheck": "off",
                "databases": [{
                    "configSrc": config.to_string_lossy(),
                    "editingAllowedCheck": "warn"
                }]
            }))
            .unwrap(),
        )
        .unwrap();
        assert_eq!(
            support_guard_mode(&config, &context),
            SupportPolicyMode::Warn
        );

        for document in [
            "not json",
            r#"{}"#,
            r#"{"editingAllowedCheck":"WARN"}"#,
            r#"{"editingAllowedCheck":"unsupported"}"#,
        ] {
            std::fs::write(&policy_paths[2], document).unwrap();
            assert_eq!(
                support_guard_mode(&config, &context),
                SupportPolicyMode::Deny,
                "{document}"
            );
        }
        std::fs::remove_file(&policy_paths[2]).unwrap();
        assert_eq!(
            support_guard_mode(&config, &context),
            SupportPolicyMode::Deny
        );
    }

    #[test]
    fn public_support_guard_resolver_matrix_runs_real_handlers() {
        crate::application::tests::mutating_native_support_guard_coverage_is_explicit();
        crate::application::tests::code_patch_locked_support_blocks_preview_and_apply_before_handler();
        crate::application::tests::subsystem_compile_guards_locked_parent_before_both_planners();
        crate::application::tests::cf_init_support_exemption_reaches_preview_and_apply_handlers();
        project_editing_policy_is_the_closed_support_guard_downgrade_source();
    }
}
