use crate::application::operation_descriptors::{
    native_operation_descriptor, SupportGuardPolicy, SupportGuardRequirement,
};
use crate::application::ports::SupportGuardCheck;
use crate::application::{AdapterOutcome, ToolHandler, ToolSpec};
use crate::domain::workspace::WorkspaceContext;
use crate::infrastructure::native_operations::common::{
    absolutize, find_support_config_dir, path_arg, required_string, support_object_uuid_for_path,
    support_root_uuid,
};
use crate::infrastructure::native_operations::{meta, template};
use crate::infrastructure::source_adapters::platform_xml::support::{
    read_support_facts, EffectiveSupportRule,
};
use crate::domain::navigation::Authorability;
use serde_json::{Map, Value};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SupportGuardMode {
    Deny,
    Warn,
    Off,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SupportGuardViolation {
    pub code: &'static str,
    pub reason: String,
    pub target_path: PathBuf,
    pub config_dir: PathBuf,
}

pub(crate) fn support_guard_violation(
    target_path: &Path,
    requirement: SupportGuardRequirement,
) -> Option<SupportGuardViolation> {
    let target_path = target_path
        .canonicalize()
        .unwrap_or_else(|_| target_path.to_path_buf());
    let config_dir = find_support_config_dir(&target_path)?;
    let facts = read_support_facts(&config_dir.join("Ext").join("ParentConfigurations.bin"));
    let object_uuid = support_object_uuid_for_path(&target_path)
        .or_else(|| support_root_uuid(&config_dir.join("Configuration.xml")));
    let effective = facts.effective_rule_for(object_uuid.as_deref().unwrap_or(""));
    if effective == EffectiveSupportRule::Unreadable {
        let error = facts.parse_error().expect("unreadable facts carry a parse error");
        let location = error
            .offset
            .map(|offset| format!(" at byte {offset}"))
            .unwrap_or_default();
            return Some(SupportGuardViolation {
                code: "support-state-unreadable",
                reason: format!(
                    "не удалось прочитать состояние поддержки (ParentConfigurations.bin): {}{}; безопасность правки не подтверждена",
                    error.context,
                    location,
                ),
                target_path,
                config_dir,
            });
    }
    if requirement == SupportGuardRequirement::Removed {
        return match effective {
            EffectiveSupportRule::Removed => None,
            EffectiveSupportRule::ConfigurationReadOnly => Some(SupportGuardViolation {
                code: "capability-off",
                reason: "возможность изменения конфигурации выключена (вся конфигурация read-only)".to_string(),
                target_path,
                config_dir,
            }),
            EffectiveSupportRule::Locked | EffectiveSupportRule::Editable | EffectiveSupportRule::Absent => Some(SupportGuardViolation {
                code: "not-removed",
                reason: "объект не снят с поддержки; удаление сломает обновления".to_string(),
                target_path,
                config_dir,
            }),
            EffectiveSupportRule::Unreadable => unreachable!("unreadable returns before requirement evaluation"),
        };
    }
    match effective.authorability() {
        Authorability::Authorable => None,
        Authorability::ConfigurationReadOnly => Some(SupportGuardViolation {
            code: "capability-off",
            reason: "возможность изменения конфигурации выключена (вся конфигурация read-only)".to_string(),
            target_path,
            config_dir,
        }),
        Authorability::SupportLocked => Some(SupportGuardViolation {
            code: if requirement == SupportGuardRequirement::Removed { "not-removed" } else { "locked" },
            reason: if requirement == SupportGuardRequirement::Removed {
                "объект или конфигурация не сняты с поддержки — удаление сломает обновления".to_string()
            } else {
                "объект или конфигурация на замке — редактирование сломает обновления".to_string()
            },
            target_path,
            config_dir,
        }),
        Authorability::UnknownSupportState | Authorability::UnknownReadOnly | Authorability::DerivedReadOnly => Some(SupportGuardViolation {
            code: "support-state-unreadable",
            reason: "состояние поддержки объекта или конфигурации неизвестно; безопасность правки не подтверждена".to_string(),
            target_path,
            config_dir,
        }),
    }
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
    if violation.code == "support-state-unreadable" {
        let message = format!(
            "[support-guard] Редактирование отклонено: состояние поддержки для объекта «{target}» нельзя достоверно прочитать.\nСостояние: {}.\nПроверьте или восстановите {}. Пока состояние поддержки не прочитано, правки заблокированы.",
            violation.reason,
            violation
                .config_dir
                .join("Ext")
                .join("ParentConfigurations.bin")
                .display(),
        );
        return AdapterOutcome {
            ok: false,
            summary: format!("{} blocked by support guard", spec.name),
            changes: Vec::new(),
            warnings: Vec::new(),
            errors: vec![message.clone()],
            artifacts: vec![violation.target_path.display().to_string()],
            stdout: None,
            stderr: Some(format!("{message}\n")),
            command: None,
        };
    }
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
    use super::{support_guard_blocked_outcome, support_guard_violation, SupportGuardViolation};
    use crate::application::{SupportGuardRequirement, ToolHandler, ToolSpec};
    use crate::domain::cache::CacheAccess;
    use std::fs;
    use std::path::PathBuf;

    #[test]
    fn unreadable_support_state_block_does_not_claim_vendor_lock() {
        let outcome = support_guard_blocked_outcome(
            ToolSpec {
                name: "unica.meta.edit",
                description: "test",
                mutating: true,
                cache_access: CacheAccess::default(),
                handler: ToolHandler::NativeOperation {
                    operation: "meta-edit",
                    event: None,
                },
            },
            &SupportGuardViolation {
                code: "support-state-unreadable",
                reason: "не удалось прочитать состояние поддержки".to_string(),
                target_path: PathBuf::from("/workspace/src/Documents/Shipment.xml"),
                config_dir: PathBuf::from("/workspace/src"),
            },
            crate::application::SupportGuardRequirement::Editable,
        );
        let message = outcome.errors.join("\n");

        assert!(message.contains("состояние поддержки"), "{message}");
        assert!(message.contains("правки заблокированы"), "{message}");
        assert!(
            !message.contains("объект типовой конфигурации"),
            "{message}"
        );
        assert!(!message.contains("на замке"), "{message}");
    }

    #[test]
    fn configuration_lock_blocks_an_editable_child_rule() {
        let root = std::env::temp_dir().join(format!(
            "unica-support-guard-monotonic-{}",
            std::process::id()
        ));
        let target = root.join("Documents/Shipment.xml");
        fs::create_dir_all(target.parent().unwrap()).unwrap();
        fs::write(
            root.join("Configuration.xml"),
            "<MetaDataObject uuid=\"aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa\"/>",
        )
        .unwrap();
        fs::write(
            &target,
            "<MetaDataObject uuid=\"cccccccc-cccc-cccc-cccc-cccccccccccc\"/>",
        )
        .unwrap();
        fs::create_dir_all(root.join("Ext")).unwrap();
        fs::write(
            root.join("Ext/ParentConfigurations.bin"),
            "{6,0,1,dddddddd-dddd-dddd-dddd-dddddddddddd,0,eeeeeeee-eeee-eeee-eeee-eeeeeeeeeeee,\"1.0\",\"Vendor\",\"VendorConf\",3,1,0,aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa,aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa,0,1,cccccccc-cccc-cccc-cccc-cccccccccccc,cccccccc-cccc-cccc-cccc-cccccccccccc,2,1,bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb,bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb}",
        )
        .unwrap();

        let violation = support_guard_violation(&target, SupportGuardRequirement::Editable)
            .expect("configuration lock must block editable child");
        assert_eq!(violation.code, "locked");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn removed_requirement_requires_an_exact_removed_object_rule() {
        let root = std::env::temp_dir().join(format!(
            "unica-support-guard-removed-{}",
            std::process::id()
        ));
        let target = root.join("Documents/Shipment.xml");
        fs::create_dir_all(target.parent().unwrap()).unwrap();
        fs::write(
            root.join("Configuration.xml"),
            "<MetaDataObject uuid=\"aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa\"/>",
        )
        .unwrap();
        fs::write(
            &target,
            "<MetaDataObject uuid=\"cccccccc-cccc-cccc-cccc-cccccccccccc\"/>",
        )
        .unwrap();
        fs::create_dir_all(root.join("Ext")).unwrap();
        let bin = root.join("Ext/ParentConfigurations.bin");
        fs::write(&bin, support_payload("1")).unwrap();

        assert_eq!(
            support_guard_violation(&target, SupportGuardRequirement::Removed)
                .expect("editable is not removed")
                .code,
            "not-removed"
        );

        fs::write(&bin, support_payload("2")).unwrap();
        assert_eq!(
            support_guard_violation(&target, SupportGuardRequirement::Removed),
            None
        );
        fs::remove_dir_all(root).unwrap();
    }

    fn support_payload(object_state: &str) -> String {
        format!(
            "{{6,0,1,dddddddd-dddd-dddd-dddd-dddddddddddd,0,eeeeeeee-eeee-eeee-eeee-eeeeeeeeeeee,\"1.0\",\"Vendor\",\"VendorConf\",3,1,1,aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa,aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa,0,{object_state},cccccccc-cccc-cccc-cccc-cccccccccccc,cccccccc-cccc-cccc-cccc-cccccccccccc,2,1,bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb,bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb}}"
        )
    }
}
