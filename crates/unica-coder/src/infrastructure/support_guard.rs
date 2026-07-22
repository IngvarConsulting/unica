use crate::application::operation_descriptors::{
    native_operation_descriptor, SupportGuardPolicy, SupportGuardRequirement,
};
use crate::application::ports::SupportGuardCheck;
use crate::application::{AdapterOutcome, ToolHandler, ToolSpec};
use crate::domain::workspace::WorkspaceContext;
use crate::infrastructure::native_operations::common::{
    absolutize, path_arg, required_string, support_guard_violation, SupportGuardViolation,
};
use crate::infrastructure::native_operations::{meta, template};
use serde_json::{Map, Value};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SupportGuardMode {
    Deny,
    Warn,
    Off,
}

pub(crate) fn evaluate_support_guard(
    spec: ToolSpec,
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> Result<SupportGuardCheck, String> {
    let Some((target_path, requirement)) = support_guard_target(spec, args, context) else {
        return Ok(SupportGuardCheck::Allow);
    };
    let Some(violation) = support_guard_violation(&target_path, requirement) else {
        return Ok(SupportGuardCheck::Allow);
    };
    if violation.code == "support-state-invalid" {
        return Ok(SupportGuardCheck::Block(support_guard_blocked_outcome(
            spec,
            &violation,
            requirement,
        )));
    }

    Ok(match support_guard_mode(&violation.config_dir, context) {
        SupportGuardMode::Off => SupportGuardCheck::Allow,
        SupportGuardMode::Warn => SupportGuardCheck::Warn(format!(
            "[support guard] ПРЕДУПРЕЖДЕНИЕ: {}. Цель: {}",
            violation.reason,
            violation.target_path.display()
        )),
        SupportGuardMode::Deny => {
            SupportGuardCheck::Block(support_guard_blocked_outcome(spec, &violation, requirement))
        }
    })
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
        SupportGuardPolicy::PathArgs { names, requirement } => {
            support_guard_path_arg(args, context, names, requirement)
        }
        SupportGuardPolicy::MetaRemove { requirement } => {
            support_guard_meta_remove_target(args, context).map(|path| (path, requirement))
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

fn support_guard_meta_remove_target(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> Option<PathBuf> {
    let config_dir = path_arg(args, &["configDir", "ConfigDir"])?;
    let object = required_string(args, &["object", "Object"], "Object").ok()?;
    let (object_type, object_name) = object.split_once('.')?;
    let type_dir = meta::meta_remove_type_plural(object_type)?;
    Some(
        absolutize(config_dir, &context.cwd)
            .join(type_dir)
            .join(format!("{object_name}.xml")),
    )
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

fn support_guard_mode(config_dir: &Path, context: &WorkspaceContext) -> SupportGuardMode {
    let Some(project_file) = find_v8_project_file(&context.cwd)
        .or_else(|| find_v8_project_file(config_dir))
        .or_else(|| find_v8_project_file(&context.workspace_root))
    else {
        return SupportGuardMode::Deny;
    };
    let Ok(text) = std::fs::read_to_string(&project_file) else {
        return SupportGuardMode::Deny;
    };
    let Ok(project) = serde_json::from_str::<Value>(text.trim_start_matches('\u{feff}')) else {
        return SupportGuardMode::Deny;
    };
    let project_dir = project_file.parent().unwrap_or_else(|| Path::new(""));
    let config_dir = normalize_guard_path(config_dir);

    if let Some(databases) = project.get("databases").and_then(Value::as_array) {
        for database in databases {
            let Some(config_src) = database.get("configSrc").and_then(Value::as_str) else {
                continue;
            };
            let config_src = PathBuf::from(config_src);
            let config_src = if config_src.is_absolute() {
                config_src
            } else {
                project_dir.join(config_src)
            };
            let config_src = normalize_guard_path(&config_src);
            if (config_dir == config_src || config_dir.starts_with(&config_src))
                && database
                    .get("editingAllowedCheck")
                    .and_then(Value::as_str)
                    .is_some()
            {
                return support_guard_mode_value(
                    database
                        .get("editingAllowedCheck")
                        .and_then(Value::as_str)
                        .expect("checked above"),
                );
            }
        }
    }

    project
        .get("editingAllowedCheck")
        .and_then(Value::as_str)
        .map(support_guard_mode_value)
        .unwrap_or(SupportGuardMode::Deny)
}

fn find_v8_project_file(start: &Path) -> Option<PathBuf> {
    let mut current = if start.is_dir() {
        start.to_path_buf()
    } else {
        start.parent()?.to_path_buf()
    };
    for _ in 0..20 {
        let candidate = current.join(".v8-project.json");
        if candidate.is_file() {
            return Some(candidate);
        }
        let Some(parent) = current.parent() else {
            break;
        };
        if parent == current {
            break;
        }
        current = parent.to_path_buf();
    }
    None
}

fn support_guard_mode_value(value: &str) -> SupportGuardMode {
    match value {
        "warn" => SupportGuardMode::Warn,
        "off" => SupportGuardMode::Off,
        _ => SupportGuardMode::Deny,
    }
}

fn normalize_guard_path(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

fn support_guard_blocked_outcome(
    spec: ToolSpec,
    violation: &SupportGuardViolation,
    requirement: SupportGuardRequirement,
) -> AdapterOutcome {
    let target = violation.target_path.display();
    let head = "[support-guard] Редактирование отклонено: это объект типовой конфигурации на поддержке поставщика, прямое редактирование молча сломает будущие обновления.";
    let cfe = "Рекомендуемый путь: внести доработку в расширение (навыки cfe-borrow / cfe-patch-method) — состояние поддержки менять не нужно, обновления вендора сохраняются.";
    let override_note =
        "Снять проверку для этой базы: editingAllowedCheck = warn|off в .v8-project.json.";
    let invalid_state_note = "Параметр editingAllowedCheck = warn|off не отключает эту fail-closed проверку существующего повреждённого или нечитаемого файла.";
    let (state, fix, policy_note) = match violation.code {
        "support-state-invalid" => (
            format!("Состояние поддержки не подтверждено: {}.", violation.reason),
            "Исправьте или восстановите Ext/ParentConfigurations.bin перед любой мутацией конфигурации. Отсутствующий файл означает собственную конфигурацию, но существующий повреждённый или нечитаемый файл небезопасно считать отсутствующим."
                .to_string(),
            invalid_state_note,
        ),
        "capability-off" => (
            format!(
                "Состояние: у всей конфигурации выключена возможность изменения (режим read-only «из коробки») — поэтому объект «{target}» редактировать нельзя."
            ),
            format!(
                "Либо снять защиту явно (навык support-edit, два шага):\n  support-edit -Path \"{}\" -Capability on — включить возможность изменения (объекты пока остаются на замке);\n  support-edit -Path \"{target}\" -Set editable — открыть этот объект для редактирования.\n  Изменение применяется в базу полной загрузкой выгрузки и обходит механизм обновлений вендора.",
                violation.config_dir.display()
            ),
            override_note,
        ),
        "not-removed" if requirement == SupportGuardRequirement::Removed => (
            format!(
                "Состояние: объект «{target}» на поддержке (не снят с поддержки) — его удаление разорвёт обновления вендора."
            ),
            format!(
                "Либо сначала снять объект с поддержки, затем удалять:\n  support-edit -Path \"{target}\" -Set off-support — объект уходит из-под обновлений, после этого удаление безопасно."
            ),
            override_note,
        ),
        _ => (
            format!(
                "Состояние: объект «{target}» на замке (возможность изменения конфигурации включена, но сам объект не редактируется)."
            ),
            format!(
                "Либо разрешить редактирование этого объекта (навык support-edit, выбрать одно):\n  support-edit -Path \"{target}\" -Set editable — редактировать и дальше получать обновления вендора (возможны конфликты слияния);\n  support-edit -Path \"{target}\" -Set off-support — снять с поддержки: обновления по объекту больше не приходят."
            ),
            override_note,
        ),
    };
    let message = format!("{head}\n{state}\n{cfe}\n{fix}\n{policy_note}");
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
