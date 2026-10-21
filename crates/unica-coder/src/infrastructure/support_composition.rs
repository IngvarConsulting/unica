//! Actual source binding, exact preimage guards, publication and terminal projection.
use crate::infrastructure::native_operations::common::*;

use serde_json::{Map, Value};
use std::fs;
use std::path::{Path, PathBuf};
use unica_execution_contract::adapter_outcome::AdapterOutcome;
use unica_metadata::support_edit::*;
use unica_metadata_designer::support::{parse_support_header, support_root_uuid_from_bytes};
use unica_metadata_designer::support_codec::*;
use unica_source_contract::proposal::DirectoryMembershipSelector;
use unica_source_contract::workspace::WorkspaceContext;
use unica_workspace::compile_transaction::*;
use unica_workspace::discovery::ProjectDiscoveryReader;
pub(crate) fn invoke_mutation(
    operation: &str,
    _tool_name: &str,
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> Option<AdapterOutcome> {
    match operation {
        "support-edit" => Some(edit_support(args, context)),
        _ => None,
    }
}

fn edit_support(args: &Map<String, Value>, context: &WorkspaceContext) -> AdapterOutcome {
    edit_support_with_data(args, context).outcome
}

pub(crate) fn edit_support_with_data(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> SupportEditExecution {
    match edit_support_execution(args, context) {
        Ok((outcome, data)) => SupportEditExecution {
            outcome,
            data: Some(data),
        },
        Err(error) => SupportEditExecution {
            outcome: AdapterOutcome {
                ok: false,
                summary: "support-edit failed".to_string(),
                changes: Vec::new(),
                warnings: Vec::new(),
                errors: vec![error],
                artifacts: Vec::new(),
                stdout: None,
                stderr: None,
                command: None,
            },
            data: None,
        },
    }
}

#[cfg(test)]
fn edit_support_result(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> Result<AdapterOutcome, String> {
    edit_support_execution(args, context).map(|(outcome, _)| outcome)
}

fn edit_support_execution(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> Result<(AdapterOutcome, SupportEditData), String> {
    let action = support_edit_action(args)?;
    let action_label = match &action {
        SupportEditAction::Capability(_) => "capability",
        SupportEditAction::Set(_) => "objectRule",
    };
    let target_path = support_target_path(args, context)?;
    if !target_path.exists() {
        return Err(format!("Путь не найден: {}", target_path.display()));
    }
    let resolved_path = target_path
        .canonicalize()
        .unwrap_or_else(|_| target_path.clone());
    let Some(config_dir) = find_support_config_dir(&resolved_path) else {
        return Err(format!(
            "Не найден корень конфигурации (Configuration.xml) над путём: {}",
            resolved_path.display()
        ));
    };
    let config_path = config_dir.join("Configuration.xml");
    let config_preimage = fs::read(&config_path).map_err(|err| {
        format!(
            "failed to read configuration owner {}: {err}",
            config_path.display()
        )
    })?;
    let uuid_dependency_reads = if matches!(&action, SupportEditAction::Set(_)) {
        support_uuid_dependency_paths(&resolved_path)
            .into_iter()
            .map(|path| {
                let preimage = if path == config_path {
                    config_preimage.clone()
                } else {
                    fs::read(&path).map_err(|err| {
                        format!(
                            "failed to read support UUID dependency {}: {err}",
                            path.display()
                        )
                    })?
                };
                let uuid = support_root_uuid_from_bytes(&preimage);
                Ok::<_, String>((path, preimage, uuid))
            })
            .collect::<Result<Vec<_>, _>>()?
    } else {
        Vec::new()
    };
    let bin_path = config_dir.join("Ext").join("ParentConfigurations.bin");
    if !bin_path.exists() {
        return Ok(noop_outcome(
            action_label,
            "Конфигурация не на поддержке (Ext/ParentConfigurations.bin отсутствует) — переключать нечего.",
        ));
    }
    let raw = fs::read(&bin_path)
        .map_err(|err| format!("failed to read {}: {err}", bin_path.display()))?;
    if raw.len() <= 32 {
        return Ok(noop_outcome(
            action_label,
            "Поддержка снята полностью (пустой ParentConfigurations.bin) — переключать нечего.",
        ));
    }
    let text = decode_parent_configurations(&raw)?;
    let Some((global_flag, vendor_count)) = parse_support_header(&text) else {
        return Err("Неизвестный формат ParentConfigurations.bin".to_string());
    };
    if vendor_count == 0 {
        return Ok(noop_outcome(
            action_label,
            "Поддержка снята полностью (пустой ParentConfigurations.bin) — переключать нечего.",
        ));
    }

    let mut capability_vendor_payload_preimages = None;
    let (mut outcome, updated, data) = match action {
        SupportEditAction::Capability(capability) => {
            if (global_flag == 0) == capability.enabled() {
                let word = if capability.enabled() {
                    "включена"
                } else {
                    "выключена"
                };
                return Ok(noop_outcome(
                    action_label,
                    format!("Возможность изменения конфигурации уже {word} — изменений нет."),
                ));
            }
            let vendor_payload_preimages = support_vendor_payload_preimages(
                &ProjectDiscoveryReader::new(&context.cwd),
                &config_dir,
            )?;
            preflight_required_vendor_payloads(
                &config_dir,
                &text,
                vendor_count,
                capability,
                vendor_payload_preimages
                    .1
                    .iter()
                    .map(|(path, _)| path.as_path()),
            )?;
            capability_vendor_payload_preimages = Some(vendor_payload_preimages);
            plan_capability(&bin_path, &text, capability, &resolved_path)
        }
        SupportEditAction::Set(rule) => {
            if global_flag != 0 {
                return Err(format!(
                    "Возможность изменения конфигурации выключена — пообъектное переключение недоступно.\n  Сначала: support-edit -Path {} -Capability on или unica.support.edit Path={} Capability=on",
                    resolved_path.display(),
                    resolved_path.display()
                ));
            }
            let object_uuid = if let Some(uuid) = uuid_dependency_reads
                .iter()
                .find_map(|(_, _, uuid)| uuid.clone())
            {
                uuid
            } else if uuid_dependency_reads
                .iter()
                .any(|(path, _, _)| path.extension().is_some_and(|ext| ext == "xml"))
            {
                return Err(format!(
                    "support UUID dependency does not contain a metadata UUID for path: {}",
                    resolved_path.display()
                ));
            } else {
                support_root_uuid_from_bytes(&config_preimage).ok_or_else(|| {
                    format!(
                        "Не удалось определить объект по пути: {}",
                        resolved_path.display()
                    )
                })?
            };
            plan_object_rule(&bin_path, &text, &object_uuid, rule, &resolved_path)
        }
    }?;
    if outcome.changes.is_empty() {
        return Ok((outcome, data));
    }

    let (vendor_payload_snapshot, vendor_payload_reads) = if let Some(preimages) =
        capability_vendor_payload_preimages
    {
        preimages
    } else {
        support_vendor_payload_preimages(&ProjectDiscoveryReader::new(&context.cwd), &config_dir)?
    };
    let updated_bytes = parent_configurations_bytes(&updated);
    let mut transaction = CompileTransaction::new();
    transaction.replace_bytes(&bin_path, &raw, updated_bytes.clone())?;
    guard_exact_preimage_if_unprotected(&mut transaction, &config_path, &config_preimage)?;
    for (path, preimage, _) in &uuid_dependency_reads {
        guard_exact_preimage_if_unprotected(&mut transaction, path, preimage)?;
    }
    for (path, preimage) in &vendor_payload_reads {
        guard_exact_preimage_if_unprotected(&mut transaction, path, preimage)?;
    }
    let vendor_payload_directory = config_dir.join("Ext").join("ParentConfigurations");
    transaction.guard_or_verify_directory_membership(
        &vendor_payload_directory,
        DirectoryMembershipSelector::CfFilesAsciiCaseInsensitive,
        vendor_payload_snapshot,
    )?;
    let mut format_dependencies = vec![config_path.as_path()];
    for (path, _, _) in &uuid_dependency_reads {
        format_dependencies.push(path.as_path());
    }
    guard_active_format_dependencies(&mut transaction, &format_dependencies, context)?;
    let report = transaction.commit_with_post_validation(|| {
        let actual = fs::read(&bin_path)
            .map_err(|err| format!("failed to verify {}: {err}", bin_path.display()))?;
        if actual != updated_bytes {
            return Err(format!(
                "support-edit post-write validation failed for {}",
                bin_path.display()
            ));
        }
        let actual_text = decode_parent_configurations(&actual)?;
        parse_support_header(&actual_text).ok_or_else(|| {
            format!(
                "support-edit post-write validation could not parse {}",
                bin_path.display()
            )
        })?;
        Ok(())
    })?;
    outcome.warnings.extend(report.cleanup_warnings);
    Ok((outcome, data))
}

fn support_target_path(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> Result<PathBuf, String> {
    unica_operation_contract::input::path_arg(args, &["Path", "path", "TargetPath", "targetPath"])
        .map(|path| unica_operation_contract::input::absolutize(path, &context.cwd))
        .ok_or_else(|| "missing required argument: Path".to_string())
}

/// Read-only capability preflight used by the public application guard.
///
/// The mutation handler repeats this validation while binding exact preimages.
/// This keeps preview and apply aligned without weakening commit-time race
/// protection.
pub(crate) fn preflight_support_edit_capability(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> Result<(), String> {
    let SupportEditAction::Capability(capability) = support_edit_action(args)? else {
        return Ok(());
    };
    let target_path = support_target_path(args, context)?;
    if !target_path.exists() {
        return Err(format!("Путь не найден: {}", target_path.display()));
    }
    let resolved_path = target_path
        .canonicalize()
        .unwrap_or_else(|_| target_path.clone());
    let Some(config_dir) = find_support_config_dir(&resolved_path) else {
        return Err(format!(
            "Не найден корень конфигурации (Configuration.xml) над путём: {}",
            resolved_path.display()
        ));
    };
    let bin_path = config_dir.join("Ext").join("ParentConfigurations.bin");
    if !bin_path.exists() {
        return Ok(());
    }
    let raw = fs::read(&bin_path)
        .map_err(|error| format!("failed to read {}: {error}", bin_path.display()))?;
    if raw.len() <= 32 {
        return Ok(());
    }
    let text = decode_parent_configurations(&raw)?;
    let Some((global_flag, vendor_count)) = parse_support_header(&text) else {
        return Err("Неизвестный формат ParentConfigurations.bin".to_string());
    };
    if vendor_count == 0 || (global_flag == 0) == capability.enabled() {
        return Ok(());
    }
    let (_, vendor_payloads) =
        support_vendor_payload_paths(&ProjectDiscoveryReader::new(&context.cwd), &config_dir)?;
    preflight_required_vendor_payloads(
        &config_dir,
        &text,
        vendor_count,
        capability,
        vendor_payloads.iter().map(PathBuf::as_path),
    )
}

fn project_support_noop(data: &SupportEditData) -> AdapterOutcome {
    AdapterOutcome {
        ok: true,
        summary: data.reason.clone().unwrap_or_default(),
        changes: Vec::new(),
        warnings: Vec::new(),
        errors: Vec::new(),
        artifacts: Vec::new(),
        stdout: None,
        stderr: None,
        command: None,
    }
}
fn noop_outcome(action: &str, message: impl Into<String>) -> (AdapterOutcome, SupportEditData) {
    let data = noop_support_data(action, message);
    (project_support_noop(&data), data)
}

pub(crate) struct SupportEditExecution {
    pub(crate) outcome: AdapterOutcome,
    pub(crate) data: Option<SupportEditData>,
}
pub(crate) fn plan_capability(
    bin_path: &Path,
    text: &str,
    capability: SupportCapability,
    target_path: &Path,
) -> Result<(AdapterOutcome, String, SupportEditData), String> {
    let (updated, data) = plan_capability_data(bin_path, text, capability)?;
    let global_target = capability.target_flag();
    let object_count = data.object_rules_reset;
    let summary = if capability.enabled() {
        "Возможность изменения конфигурации ВКЛЮЧЕНА"
    } else {
        "Возможность изменения конфигурации ВЫКЛЮЧЕНА"
    };
    Ok((
        AdapterOutcome {
            ok: true,
            summary: summary.to_string(),
            changes: vec![
                format!("updated {}", bin_path.display()),
                format!("set global editing flag to {global_target}"),
                format!("reset object support rules: {object_count}"),
            ],
            warnings: if capability.enabled() {
                vec![
                    "Все объекты поставщика оставлены на замке; включайте editable/off-support точечно."
                        .to_string(),
                ]
            } else {
                Vec::new()
            },
            errors: Vec::new(),
            artifacts: vec![
                bin_path.display().to_string(),
                target_path.display().to_string(),
            ],
            stdout: None,
            stderr: None,
            command: None,
        },
        updated,
        data,
    ))
}
pub(crate) fn plan_object_rule(
    bin_path: &Path,
    text: &str,
    object_uuid: &str,
    rule: SupportObjectRule,
    target_path: &Path,
) -> Result<(AdapterOutcome, String, SupportEditData), String> {
    let (updated, data) = plan_object_rule_data(bin_path, text, object_uuid, rule)?;
    let changed = data.records_changed;
    let summary = format!("Объект uuid {object_uuid} → {}.", rule.state_text());
    if !data.applied {
        return Ok((project_support_noop(&data), updated, data));
    }
    Ok((
        AdapterOutcome {
            ok: true,
            summary: summary.clone(),
            changes: vec![
                format!("updated {}", bin_path.display()),
                format!("set object {object_uuid} support rule to {}", rule.flag()),
                format!("updated support records: {changed}"),
            ],
            warnings: if matches!(rule, SupportObjectRule::Editable) {
                vec![
                    "Объект продолжит получать обновления вендора; при обновлении возможны конфликты."
                        .to_string(),
                ]
            } else {
                Vec::new()
            },
            errors: Vec::new(),
            artifacts: vec![
                bin_path.display().to_string(),
                target_path.display().to_string(),
            ],
            stdout: None,
            stderr: None,
            command: None,
        },
        updated,
        data,
    ))
}

#[cfg(test)]
#[path = "../../tests/platform/support_tests.rs"]
pub(crate) mod tests;
