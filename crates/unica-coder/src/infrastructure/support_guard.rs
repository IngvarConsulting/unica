use crate::application::operation_descriptors::{
    native_operation_descriptor, SupportGuardPolicy, SupportGuardRequirement,
};
use crate::application::ports::SupportGuardCheck;
use crate::application::{AdapterOutcome, ToolHandler, ToolSpec};
use crate::domain::workspace::WorkspaceContext;
use crate::infrastructure::native_operations::common::{absolutize, path_arg, required_string};
use crate::infrastructure::native_operations::{meta, template};
use serde_json::{Map, Value};
use std::path::{Path, PathBuf};
use unica_application::{GuardEnforcement, OperationalPolicyDecision, OperationalPolicyService};
use unica_format_core::ports::{
    AuthorabilityRequest, AuthorabilityRequirement, FormatDiagnostic, FormatDiagnosticCode,
};

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
    pub support_state_path: PathBuf,
}

fn support_guard_violation_with_context(
    target_path: &Path,
    requirement: SupportGuardRequirement,
    context: &WorkspaceContext,
) -> Option<SupportGuardViolation> {
    let target_path = target_path.to_path_buf();
    let session = unica_adapter_platform_xml::PlatformXmlAdapterFactory::new()
        .capture_unscoped_source(
            &target_path,
            &context.workspace_root,
            unica_format_core::ports::OwnerResolutionMode::Existing,
        );
    support_guard_violation_for_session(
        session,
        requirement,
        target_path,
        context.workspace_root.clone(),
    )
}

fn support_guard_violation_for_session(
    session: unica_format_core::ports::OperationalSourceSession,
    requirement: SupportGuardRequirement,
    target_path: PathBuf,
    fallback_root: PathBuf,
) -> Option<SupportGuardViolation> {
    let port = unica_adapter_platform_xml::PlatformXmlAdapterFactory::new().authorability_port();
    let request = AuthorabilityRequest::new(
        session,
        match requirement {
            SupportGuardRequirement::Editable => AuthorabilityRequirement::Editable,
            SupportGuardRequirement::Removed => AuthorabilityRequirement::Removed,
        },
    );
    let result = match port.inspect(&request) {
        Ok(result) => result,
        Err(_) => {
            return Some(SupportGuardViolation {
                code: "support-state-unreadable",
                reason:
                    "состояние поддержки не удалось прочитать — правки не подтверждены"
                        .to_string(),
                target_path,
                config_dir: fallback_root.clone(),
                support_state_path: fallback_root,
            })
        }
    };
    result.into_violation().map(|violation| {
        let diagnostic = violation.into_diagnostic();
        let code = match diagnostic.code() {
            FormatDiagnosticCode::SupportCapabilityDisabled => "capability-off",
            FormatDiagnosticCode::SupportRemovalRequired => "not-removed",
            FormatDiagnosticCode::SupportLocked => "locked",
            _ => "support-state-unreadable",
        };
        let reason = public_support_diagnostic_message(&diagnostic).to_string();
        SupportGuardViolation {
            code,
            reason,
            target_path,
            config_dir: fallback_root.clone(),
            support_state_path: fallback_root,
        }
    })
}

fn public_support_diagnostic_message(diagnostic: &FormatDiagnostic) -> &'static str {
    match diagnostic.code() {
        FormatDiagnosticCode::SupportCapabilityDisabled => {
            "Source editing is disabled by support policy."
        }
        FormatDiagnosticCode::SupportRemovalRequired => {
            "The object must be removed from support before this operation."
        }
        FormatDiagnosticCode::SupportLocked => "The object is locked by support policy.",
        _ => "состояние поддержки не удалось прочитать — правки не подтверждены",
    }
}

#[cfg(test)]
pub(crate) fn support_guard_violation(
    target_path: &Path,
    requirement: SupportGuardRequirement,
) -> Option<SupportGuardViolation> {
    let target_path = target_path.to_path_buf();
    let authorized_root = target_path
        .parent()
        .and_then(Path::parent)
        .unwrap_or_else(|| Path::new(""))
        .to_path_buf();
    let session = unica_adapter_platform_xml::PlatformXmlAdapterFactory::new()
        .capture_unscoped_source(
            &target_path,
            &authorized_root,
            unica_format_core::ports::OwnerResolutionMode::Existing,
        );
    support_guard_violation_for_session(
        session,
        requirement,
        target_path.clone(),
        target_path
            .parent()
            .unwrap_or_else(|| Path::new(""))
            .to_path_buf(),
    )
}

pub(crate) fn evaluate_support_guard(
    spec: ToolSpec,
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> Result<SupportGuardCheck, String> {
    let Some((target_path, requirement)) = support_guard_target(spec, args, context) else {
        return Ok(SupportGuardCheck::Allow);
    };
    let Some(violation) = support_guard_violation_with_context(&target_path, requirement, context)
    else {
        return Ok(SupportGuardCheck::Allow);
    };
    let enforcement = match support_guard_mode(&violation.config_dir, context) {
        SupportGuardMode::Off => GuardEnforcement::Off,
        SupportGuardMode::Warn => GuardEnforcement::Warn,
        SupportGuardMode::Deny => GuardEnforcement::Deny,
    };
    let diagnostic = FormatDiagnostic::new(
        match violation.code {
            "capability-off" => FormatDiagnosticCode::SupportCapabilityDisabled,
            "not-removed" => FormatDiagnosticCode::SupportRemovalRequired,
            "locked" => FormatDiagnosticCode::SupportLocked,
            _ => FormatDiagnosticCode::SupportStateUnreadable,
        },
        violation.reason.clone(),
    );
    Ok(
        match OperationalPolicyService::decide_authorability(Some(diagnostic), enforcement) {
            OperationalPolicyDecision::Allow => SupportGuardCheck::Allow,
            OperationalPolicyDecision::Warn(_) => SupportGuardCheck::Warn(format!(
                "[support guard] ПРЕДУПРЕЖДЕНИЕ: {}",
                violation.reason
            )),
            OperationalPolicyDecision::Block(_) => SupportGuardCheck::Block(
                support_guard_blocked_outcome(spec, &violation, requirement),
            ),
        },
    )
}

fn support_guard_target(
    spec: ToolSpec,
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> Option<(PathBuf, SupportGuardRequirement)> {
    let ToolHandler::NativeOperation { operation, .. } = spec.handler else {
        return None;
    };
    support_guard_target_for_operation(operation, args, context)
}

pub(crate) fn support_guard_target_for_operation(
    operation: &str,
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> Option<(PathBuf, SupportGuardRequirement)> {
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
    if violation.code == "support-state-unreadable" {
        let message = format!(
            "[support-guard] Редактирование отклонено: состояние поддержки нельзя достоверно прочитать.\nСостояние: {}. Пока состояние поддержки не прочитано, правки заблокированы.",
            violation.reason
        );
        return AdapterOutcome {
            ok: false,
            summary: format!("{} blocked by support guard", spec.name),
            changes: Vec::new(),
            warnings: Vec::new(),
            errors: vec![message.clone()],
            artifacts: Vec::new(),
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
            "Состояние: у всей конфигурации выключена возможность изменения.".to_string(),
            "Либо явно включить изменение конфигурации и затем разрешить изменение объекта."
                .to_string(),
        ),
        "not-removed" if requirement == SupportGuardRequirement::Removed => (
            "Состояние: объект не снят с поддержки; удаление разорвёт обновления поставщика."
                .to_string(),
            "Либо сначала явно снять объект с поддержки, затем повторить удаление.".to_string(),
        ),
        _ => (
            "Состояние: объект заблокирован политикой поддержки.".to_string(),
            "Либо явно разрешить редактирование объекта или снять его с поддержки.".to_string(),
        ),
    };
    let message = format!("{head}\n{state}\n{cfe}\n{fix}\n{off_note}");
    AdapterOutcome {
        ok: false,
        summary: format!("{} blocked by support guard", spec.name),
        changes: Vec::new(),
        warnings: Vec::new(),
        errors: vec![message.clone()],
        artifacts: Vec::new(),
        stdout: None,
        stderr: Some(format!("{message}\n")),
        command: None,
    }
}

#[cfg(test)]
mod tests {
    use super::{
        public_support_diagnostic_message, support_guard_blocked_outcome,
        support_guard_violation, SupportGuardViolation,
    };
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
                support_state_path: PathBuf::from("/workspace/src/Ext/ParentConfigurations.bin"),
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
        let public = serde_json::to_string(&outcome).unwrap();
        for forbidden in [
            "/workspace/src/Documents/Shipment.xml",
            "/workspace/src",
            "/workspace/src/Ext/ParentConfigurations.bin",
            "Configuration.xml",
            "ParentConfigurations.bin",
            "MetaDataObject",
            r"C:\private\source",
        ] {
            assert!(!public.contains(forbidden), "leaked {forbidden}: {public}");
        }
    }

    #[test]
    fn public_support_message_ignores_adapter_free_form_text() {
        let diagnostic = unica_format_core::ports::FormatDiagnostic::new(
            unica_format_core::ports::FormatDiagnosticCode::SupportLocked,
            r"/private/Configuration.xml C:\private MetaDataObject",
        );

        let public = public_support_diagnostic_message(&diagnostic);

        assert_eq!(public, "The object is locked by support policy.");
        assert!(!public.contains("private"));
        assert!(!public.contains("Configuration.xml"));
        assert!(!public.contains("MetaDataObject"));
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
            "{6,0,1,dddddddd-dddd-dddd-dddd-dddddddddddd,0,eeeeeeee-eeee-eeee-eeee-eeeeeeeeeeee,\"1.0\",\"Vendor\",\"VendorConf\",2,0,0,aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa,1,0,cccccccc-cccc-cccc-cccc-cccccccccccc,cccccccc-cccc-cccc-cccc-cccccccccccc}",
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
            "{{6,0,1,dddddddd-dddd-dddd-dddd-dddddddddddd,0,eeeeeeee-eeee-eeee-eeee-eeeeeeeeeeee,\"1.0\",\"Vendor\",\"VendorConf\",2,1,0,aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa,{object_state},0,cccccccc-cccc-cccc-cccc-cccccccccccc,cccccccc-cccc-cccc-cccc-cccccccccccc}}"
        )
    }
}
