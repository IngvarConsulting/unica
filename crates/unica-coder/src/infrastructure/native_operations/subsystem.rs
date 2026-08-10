#![allow(dead_code, unused_imports)]

use crate::application::operation_descriptors::SUBSYSTEM_PATH;
use crate::application::AdapterOutcome;
use crate::domain::cancellation::{cancelled_error, CancellationToken};
use crate::domain::code_intelligence::ProviderDeadline;
use crate::domain::format_profile::{classify_root_version, FormatCompatibility};
use crate::domain::project_sources::SourceSetKind;
use crate::domain::subsystem::SubsystemAddress;
use crate::domain::workspace::WorkspaceContext;
use crate::infrastructure::platform_xml_owner::root_version_literal;
use crate::infrastructure::project_sources::discover_project_source_map;
use crate::infrastructure::source_roots::normalize_path_identity;
use crate::infrastructure::subsystem_topology::{
    capture_registered_subsystem_topology, SubsystemTopology, SubsystemTopologyNode,
};
use roxmltree::Document;
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::ffi::OsStr;
use std::fs;
use std::io::{self, Write};
use std::path::{Component, Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use super::common::*;
use super::compile_transaction::{CompileTransaction, RegistrationStatus};
use super::{cf::*, cfe::*, dcs::*, form::*, interface::*, meta::*, mxl::*, role::*, template::*};

#[cfg(test)]
type SubsystemCompileAfterRootOwnerProbeHook = Box<dyn FnOnce(&Path)>;

#[cfg(test)]
thread_local! {
    static SUBSYSTEM_COMPILE_AFTER_ROOT_OWNER_PROBE_HOOK:
        std::cell::RefCell<Option<SubsystemCompileAfterRootOwnerProbeHook>> =
        const { std::cell::RefCell::new(None) };
}

#[cfg(test)]
fn with_subsystem_compile_after_root_owner_probe_hook<T>(
    hook: impl FnOnce(&Path) + 'static,
    action: impl FnOnce() -> T,
) -> T {
    struct Reset(Option<SubsystemCompileAfterRootOwnerProbeHook>);
    impl Drop for Reset {
        fn drop(&mut self) {
            SUBSYSTEM_COMPILE_AFTER_ROOT_OWNER_PROBE_HOOK.with(|slot| {
                slot.replace(self.0.take());
            });
        }
    }

    let previous = SUBSYSTEM_COMPILE_AFTER_ROOT_OWNER_PROBE_HOOK
        .with(|slot| slot.replace(Some(Box::new(hook))));
    let _reset = Reset(previous);
    action()
}

#[cfg(test)]
fn run_subsystem_compile_after_root_owner_probe_hook(path: &Path) {
    if let Some(hook) =
        SUBSYSTEM_COMPILE_AFTER_ROOT_OWNER_PROBE_HOOK.with(|slot| slot.borrow_mut().take())
    {
        hook(path);
    }
}

pub(crate) struct SubsystemInfoData {
    pub(crate) name: String,
    pub(crate) synonym: String,
    pub(crate) comment: String,
    pub(crate) include_ci: String,
    pub(crate) use_one_command: String,
    pub(crate) explanation: String,
    pub(crate) picture: String,
    pub(crate) content_items: Vec<String>,
    pub(crate) groups: Vec<(String, Vec<String>)>,
    pub(crate) child_names: Vec<String>,
    pub(crate) has_ci: bool,
}

pub(crate) struct SubsystemValidationReport {
    pub(crate) errors: usize,
    pub(crate) warnings: usize,
    pub(crate) ok_count: usize,
    pub(crate) detailed: bool,
    pub(crate) lines: Vec<String>,
}

impl SubsystemValidationReport {
    pub(crate) fn new(detailed: bool) -> Self {
        Self {
            errors: 0,
            warnings: 0,
            ok_count: 0,
            detailed,
            lines: Vec::new(),
        }
    }

    pub(crate) fn out(&mut self, msg: impl Into<String>) {
        self.lines.push(msg.into());
    }

    pub(crate) fn ok(&mut self, msg: impl AsRef<str>) {
        self.ok_count += 1;
        if self.detailed {
            self.lines.push(format!("[OK]    {}", msg.as_ref()));
        }
    }

    pub(crate) fn error(&mut self, msg: impl AsRef<str>) {
        self.errors += 1;
        self.lines.push(format!("[ERROR] {}", msg.as_ref()));
    }

    pub(crate) fn warn(&mut self, msg: impl AsRef<str>) {
        self.warnings += 1;
        self.lines.push(format!("[WARN]  {}", msg.as_ref()));
    }

    pub(crate) fn finish(mut self, sub_name: &str) -> String {
        let checks = self.ok_count + self.errors + self.warnings;
        if self.errors == 0 && self.warnings == 0 && !self.detailed {
            format!("=== Validation OK: Subsystem.{sub_name} ({checks} checks) ===")
        } else {
            self.out("");
            self.out(format!(
                "=== Result: {} errors, {} warnings ({checks} checks) ===",
                self.errors, self.warnings
            ));
            format!("{}\r\n", self.lines.join("\r\n"))
        }
    }
}

pub(crate) struct SubsystemEditModel {
    pub(crate) version: String,
    pub(crate) uuid: String,
    pub(crate) name: String,
    pub(crate) synonym: String,
    pub(crate) comment: String,
    pub(crate) include_help: String,
    pub(crate) include_ci: String,
    pub(crate) use_one_command: String,
    pub(crate) explanation: String,
    pub(crate) picture: String,
    pub(crate) content: Vec<String>,
    pub(crate) children: Vec<String>,
}

#[derive(Default)]
pub(crate) struct SubsystemEditCounters {
    pub(crate) added: usize,
    pub(crate) removed: usize,
    pub(crate) modified: usize,
}

struct SubsystemEditResult {
    data: SubsystemEditData,
    artifacts: Vec<PathBuf>,
    changes: Vec<String>,
    warnings: Vec<String>,
}

/// Typed answer of `unica.subsystem.edit` (ADR-0023). Every requested operation
/// is reported, including the ones that changed nothing, and a content
/// reference that had to be normalized says so instead of printing `[NORM]`.
#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SubsystemEditData {
    pub(crate) subsystem: String,
    pub(crate) operations: Vec<SubsystemEditOperationData>,
    pub(crate) added: usize,
    pub(crate) removed: usize,
    pub(crate) modified: usize,
    /// Child subsystem descriptors created because the edit referenced them.
    pub(crate) created_stubs: Vec<String>,
    /// The edit commits only through post-validation; this records that the
    /// validator actually ran over the written subsystem.
    pub(crate) validated: bool,
    pub(crate) mutation: MutationData,
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SubsystemEditOperationData {
    pub(crate) operation: String,
    pub(crate) target: Option<String>,
    pub(crate) applied: bool,
    /// Why nothing changed; `null` when the operation applied.
    pub(crate) reason: Option<String>,
    /// The reference as it was requested, when it had to be normalized to the
    /// canonical form; `null` when it was already canonical.
    pub(crate) normalized_from: Option<String>,
}

impl SubsystemEditOperationData {
    fn applied(operation: &str, target: impl Into<Option<String>>) -> Self {
        Self {
            operation: operation.to_string(),
            target: target.into(),
            applied: true,
            reason: None,
            normalized_from: None,
        }
    }

    fn skipped(operation: &str, target: impl Into<Option<String>>, reason: &str) -> Self {
        Self {
            operation: operation.to_string(),
            target: target.into(),
            applied: false,
            reason: Some(reason.to_string()),
            normalized_from: None,
        }
    }

    fn normalized_from(mut self, original: String) -> Self {
        self.normalized_from = Some(original);
        self
    }
}

/// Collects what the subsystem edit did, replacing the `&mut String` the
/// helpers used to append prose to.
pub(crate) type SubsystemEditLog = Vec<SubsystemEditOperationData>;

pub(crate) struct SubsystemEditExecution {
    pub(crate) outcome: AdapterOutcome,
    pub(crate) data: Option<SubsystemEditData>,
}

fn validate_subsystem_metadata_name(argument: &str, value: &str) -> Result<(), String> {
    let mut components = Path::new(value).components();
    let is_single_path_component = matches!(
        components.next(),
        Some(Component::Normal(component)) if component == OsStr::new(value)
    ) && components.next().is_none();

    if form_is_xml_ncname(value) && is_single_path_component {
        Ok(())
    } else {
        Err(format!(
            "{argument} must be a valid Unicode XML NCName and a single path component: {value:?}"
        ))
    }
}

fn canonical_subsystem_boolean(value: &Value, property: &str) -> Result<String, String> {
    match value {
        Value::Bool(value) => Ok(value.to_string()),
        Value::String(value) if value == "true" || value == "false" => Ok(value.clone()),
        _ => Err(format!(
            "{property} must be a JSON boolean or the canonical string true or false"
        )),
    }
}

fn subsystem_boolean_field(
    definition: &Value,
    property: &str,
    default: bool,
) -> Result<String, String> {
    definition
        .get(property)
        .map(|value| canonical_subsystem_boolean(value, property))
        .unwrap_or_else(|| Ok(default.to_string()))
}

fn subsystem_validation_args(path: &Path) -> Map<String, Value> {
    let mut args = Map::new();
    args.insert(
        "SubsystemPath".to_string(),
        Value::String(path.display().to_string()),
    );
    args
}

pub(crate) fn subsystem_command_interface_path(subsystem_xml: &Path) -> PathBuf {
    subsystem_dir_for_xml(subsystem_xml)
        .join("Ext")
        .join("CommandInterface.xml")
}

/// Return the exact version-owning XML candidates read by subsystem semantic
/// validation for the supplied descriptor paths. A subsystem validator reads
/// its descriptor and, when present, the direct Ext/CommandInterface.xml
/// sidecar. Configuration.xml uses the configuration validator instead.
pub(crate) fn subsystem_validation_format_dependency_paths(
    descriptor_paths: &[&Path],
) -> Vec<PathBuf> {
    let mut paths = Vec::with_capacity(descriptor_paths.len() * 2);
    for descriptor in descriptor_paths {
        paths.push((*descriptor).to_path_buf());
        if descriptor.file_name() != Some(OsStr::new("Configuration.xml")) {
            paths.push(subsystem_command_interface_path(descriptor));
        }
    }
    paths.sort();
    paths.dedup();
    paths
}

/// Return the exact platform XML documents whose contents the public
/// subsystem read handlers inspect for the requested path semantics.
pub(crate) fn subsystem_read_format_dependency_paths(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
    operation: &str,
) -> Result<Vec<PathBuf>, String> {
    let raw_path = required_path(args, SUBSYSTEM_PATH, "SubsystemPath")?;
    let path = absolutize(raw_path, &context.cwd);

    if operation == "subsystem-validate" {
        let descriptor = resolve_subsystem_validate_xml(path)?;
        return Ok(subsystem_validation_format_dependency_paths(&[
            descriptor.as_path()
        ]));
    }

    if operation != "subsystem-info" {
        return Ok(Vec::new());
    }
    Err("subsystem-info format dependencies require prepared registered evidence".to_string())
}

fn require_subsystem_validation(outcome: &AdapterOutcome) -> Result<(), String> {
    if outcome.ok {
        return Ok(());
    }
    let errors = outcome
        .errors
        .iter()
        .map(|error| error.trim())
        .filter(|error| !error.is_empty())
        .collect::<Vec<_>>()
        .join("; ");
    let stderr = outcome.stderr.as_deref().unwrap_or("").trim();
    let stdout = outcome.stdout.as_deref().unwrap_or("").trim();
    let details = if !errors.is_empty() {
        errors
    } else if !stderr.is_empty() {
        stderr.to_string()
    } else if !stdout.is_empty() {
        stdout.to_string()
    } else {
        outcome.summary.clone()
    };
    Err(format!("subsystem semantic validation failed: {details}"))
}

fn require_subsystem_registration_owner_validation(
    path: &Path,
    context: &WorkspaceContext,
) -> Result<(), String> {
    if path.file_name() != Some(OsStr::new("Configuration.xml")) {
        return validate_subsystem_owner_path(path, context);
    }

    validate_cf_owner_path(path, context).map_err(|detail| {
        format!(
            "subsystem.compile Configuration owner validation failed for {}: {}",
            path.display(),
            detail.trim()
        )
    })
}

pub(crate) fn edit_subsystem(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> AdapterOutcome {
    edit_subsystem_with_data(args, context).outcome
}

pub(crate) fn edit_subsystem_with_data(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> SubsystemEditExecution {
    let edit_result = (|| -> Result<SubsystemEditResult, String> {
        let definition_file = path_arg(args, &["definitionFile", "DefinitionFile"]);
        let operation = string_arg(args, &["operation", "Operation"]);
        if definition_file.is_some() && operation.is_some() {
            return Err("Cannot use both -DefinitionFile and -Operation".to_string());
        }
        if definition_file.is_none() && operation.is_none() {
            return Err("Either -DefinitionFile or -Operation is required".to_string());
        }

        let raw_path = required_path(args, SUBSYSTEM_PATH, "SubsystemPath")?;
        let resolved_path = resolve_subsystem_edit_xml(absolutize(raw_path, &context.cwd))?;
        let original = fs::read(&resolved_path)
            .map_err(|err| format!("failed to read {}: {err}", resolved_path.display()))?;
        let mut model = load_subsystem_edit_model(&resolved_path)?;
        let obj_name = model.name.clone();
        let mut transaction = CompileTransaction::new();
        let operations = subsystem_edit_operations_guarded(
            args,
            &context.cwd,
            operation,
            definition_file,
            &mut transaction,
        )?;
        let mut counters = SubsystemEditCounters::default();
        let mut log = SubsystemEditLog::new();
        let mut child_stubs = BTreeMap::<PathBuf, String>::new();
        let mut reused_child_subsystems = BTreeMap::<PathBuf, Vec<u8>>::new();

        for (op_name, value) in operations {
            match op_name.as_str() {
                "add-content" => {
                    subsystem_edit_add_content(&mut model, &value, &mut counters, &mut log)?;
                }
                "remove-content" => {
                    subsystem_edit_remove_content(&mut model, &value, &mut counters, &mut log)?;
                }
                "add-child" => subsystem_edit_add_child(
                    &mut model,
                    &resolved_path,
                    &value,
                    &mut counters,
                    &mut log,
                    &mut child_stubs,
                    &mut reused_child_subsystems,
                )?,
                "remove-child" => {
                    subsystem_edit_remove_child(&mut model, &value, &mut counters, &mut log)?
                }
                "set-property" => {
                    subsystem_edit_set_property(&mut model, &value, &mut counters, &mut log)?
                }
                _ => return Err(format!("Unknown operation: {op_name}")),
            }
        }

        validate_subsystem_metadata_name("Name", &model.name)?;
        if !is_valid_uuid(&model.uuid) {
            return Err(format!(
                "Subsystem UUID must be a valid UUID: {:?}",
                model.uuid
            ));
        }
        for (property, value) in [
            ("IncludeHelpInContents", &model.include_help),
            ("IncludeInCommandInterface", &model.include_ci),
            ("UseOneCommand", &model.use_one_command),
        ] {
            canonical_subsystem_boolean(&Value::String(value.clone()), property)?;
        }
        for child in &model.children {
            validate_subsystem_metadata_name("Child subsystem name", child)?;
        }
        {
            let final_child_names = model
                .children
                .iter()
                .map(String::as_str)
                .collect::<HashSet<_>>();
            let remains_registered = |path: &Path| {
                path.file_stem()
                    .and_then(|value| value.to_str())
                    .is_some_and(|name| final_child_names.contains(name))
            };
            child_stubs.retain(|path, _| remains_registered(path));
            reused_child_subsystems.retain(|path, _| remains_registered(path));
        }

        transaction.replace_bytes(
            &resolved_path,
            &original,
            utf8_bom_bytes(&emit_subsystem_edit_model(&model)),
        )?;
        for (path, xml) in &child_stubs {
            transaction.create_utf8_bom_text(path, xml)?;
        }
        for child_path in reused_child_subsystems.keys() {
            let outcome = validate_subsystem(&subsystem_validation_args(child_path), context);
            require_subsystem_validation(&outcome)?;
        }
        for (path, preimage) in &reused_child_subsystems {
            guard_exact_preimage_if_unprotected(&mut transaction, path, preimage)?;
        }
        let mut validation_targets = vec![resolved_path.as_path()];
        validation_targets.extend(reused_child_subsystems.keys().map(PathBuf::as_path));
        let format_dependencies = subsystem_validation_format_dependency_paths(&validation_targets);
        let format_dependency_refs = format_dependencies
            .iter()
            .map(PathBuf::as_path)
            .collect::<Vec<_>>();
        guard_active_format_dependencies(&mut transaction, &format_dependency_refs, context)?;

        let validation_args = subsystem_validation_args(&resolved_path);
        let mut validation_stdout = None;
        let mut validated = false;
        let report = transaction.commit_with_post_validation(|| {
            for child_path in reused_child_subsystems.keys() {
                let outcome = validate_subsystem(&subsystem_validation_args(child_path), context);
                require_subsystem_validation(&outcome)?;
            }
            let outcome = validate_subsystem(&validation_args, context);
            validated = outcome.ok;
            validation_stdout = outcome.stdout.clone();
            require_subsystem_validation(&outcome)
        })?;

        let _ = validation_stdout;
        let mut mutation = MutationData::new(true);
        for path in &report.created {
            mutation = mutation.created(path);
        }
        for path in &report.updated {
            mutation = mutation.updated(path);
        }
        let data = SubsystemEditData {
            subsystem: obj_name.to_string(),
            operations: log,
            added: counters.added,
            removed: counters.removed,
            modified: counters.modified,
            created_stubs: child_stubs
                .keys()
                .map(|path| path.display().to_string())
                .collect(),
            validated,
            mutation,
        };

        let mut changes = report
            .created
            .iter()
            .map(|path| format!("created {}", path.display()))
            .collect::<Vec<_>>();
        changes.extend(
            report
                .updated
                .iter()
                .map(|path| format!("updated {}", path.display())),
        );
        let mut artifacts = report.created;
        artifacts.extend(report.updated);
        Ok(SubsystemEditResult {
            data,
            artifacts,
            changes,
            warnings: report.cleanup_warnings,
        })
    })();

    match edit_result {
        Ok(result) => SubsystemEditExecution {
            outcome: AdapterOutcome {
                ok: true,
                summary: format!(
                    "unica.subsystem.edit applied {} of {} operation(s) to {}",
                    result
                        .data
                        .operations
                        .iter()
                        .filter(|op| op.applied)
                        .count(),
                    result.data.operations.len(),
                    result.data.subsystem
                ),
                changes: result.changes,
                warnings: result.warnings,
                errors: Vec::new(),
                artifacts: result
                    .artifacts
                    .iter()
                    .map(|path| path.display().to_string())
                    .collect(),
                stdout: None,
                stderr: None,
                command: None,
            },
            data: Some(result.data),
        },
        Err(error) => SubsystemEditExecution {
            outcome: AdapterOutcome {
                ok: false,
                summary: "unica.subsystem.edit failed in native subsystem editor".to_string(),
                changes: Vec::new(),
                warnings: Vec::new(),
                errors: vec![error.clone()],
                artifacts: Vec::new(),
                stdout: None,
                stderr: Some(format!("{error}\n")),
                command: None,
            },
            data: None,
        },
    }
}

pub(crate) fn subsystem_edit_operations(
    args: &Map<String, Value>,
    cwd: &Path,
    operation: Option<&str>,
    definition_file: Option<PathBuf>,
) -> Result<Vec<(String, Value)>, String> {
    if let Some(definition_file) = definition_file {
        let definition_file = absolutize(definition_file, cwd);
        let text = fs::read_to_string(&definition_file)
            .map_err(|err| format!("failed to read {}: {err}", definition_file.display()))?;
        let parsed: Value = serde_json::from_str(text.trim_start_matches('\u{feff}'))
            .map_err(|err| format!("failed to parse {}: {err}", definition_file.display()))?;
        Ok(subsystem_edit_operations_from_value(parsed, operation))
    } else {
        Ok(vec![(
            operation.unwrap_or("").to_string(),
            Value::String(
                string_arg(args, &["value", "Value"])
                    .unwrap_or_default()
                    .to_string(),
            ),
        )])
    }
}

fn subsystem_edit_operations_guarded(
    args: &Map<String, Value>,
    cwd: &Path,
    operation: Option<&str>,
    definition_file: Option<PathBuf>,
    transaction: &mut CompileTransaction,
) -> Result<Vec<(String, Value)>, String> {
    if let Some(definition_file) = definition_file {
        let definition_file = absolutize(definition_file, cwd);
        let parsed = FileBackedJson::read(&definition_file, |err| {
            format!("failed to parse {}: {err}", definition_file.display())
        })?
        .bind_to(transaction)?;
        Ok(subsystem_edit_operations_from_value(parsed, operation))
    } else {
        subsystem_edit_operations(args, cwd, operation, None)
    }
}

fn subsystem_edit_operations_from_value(
    parsed: Value,
    operation: Option<&str>,
) -> Vec<(String, Value)> {
    let items = match parsed {
        Value::Array(items) => items,
        other => vec![other],
    };
    items
        .into_iter()
        .map(|item| {
            let op_name = item
                .get("operation")
                .and_then(Value::as_str)
                .unwrap_or(operation.unwrap_or(""))
                .to_string();
            let value = item
                .get("value")
                .cloned()
                .unwrap_or_else(|| Value::String(String::new()));
            (op_name, value)
        })
        .collect()
}

pub(crate) fn subsystem_edit_ml_text(props: roxmltree::Node<'_, '_>, tag: &str) -> String {
    meta_info_child(props, tag)
        .and_then(|node| {
            node.descendants()
                .find(|child| role_info_element(*child, "content", None))
                .and_then(|child| child.text())
                .or_else(|| node.text())
        })
        .map(str::trim)
        .unwrap_or("")
        .to_string()
}

pub(crate) fn subsystem_edit_picture_text(props: roxmltree::Node<'_, '_>) -> String {
    meta_info_child(props, "Picture")
        .and_then(|node| {
            node.children()
                .find(|child| role_info_element(*child, "Ref", None))
                .and_then(|child| child.text())
                .or_else(|| node.text())
        })
        .map(str::trim)
        .unwrap_or("")
        .to_string()
}

pub(crate) fn subsystem_edit_value_list(value: &Value) -> Result<Vec<String>, String> {
    match value {
        Value::String(text) => {
            let trimmed = text.trim();
            if trimmed.starts_with('[') {
                let parsed: Value = serde_json::from_str(trimmed)
                    .map_err(|err| format!("failed to parse value list: {err}"))?;
                subsystem_edit_array_strings(&parsed)
            } else {
                Ok(vec![text.to_string()])
            }
        }
        Value::Array(_) => subsystem_edit_array_strings(value),
        _ => Ok(vec![json_value_to_python_string(value)]),
    }
}

pub(crate) fn subsystem_edit_array_strings(value: &Value) -> Result<Vec<String>, String> {
    let Some(items) = value.as_array() else {
        return Err("value must be an array".to_string());
    };
    Ok(items.iter().map(json_value_to_python_string).collect())
}

pub(crate) fn subsystem_edit_object(value: &Value) -> Result<Value, String> {
    if value.is_object() {
        Ok(value.clone())
    } else if let Some(text) = value.as_str() {
        serde_json::from_str(text).map_err(|err| format!("failed to parse JSON value: {err}"))
    } else {
        Err("value must be a JSON object".to_string())
    }
}

pub(crate) fn subsystem_edit_add_content(
    model: &mut SubsystemEditModel,
    value: &Value,
    counters: &mut SubsystemEditCounters,
    log: &mut SubsystemEditLog,
) -> Result<(), String> {
    let mut existing = model.content.iter().cloned().collect::<HashSet<_>>();
    for raw in subsystem_edit_value_list(value)? {
        let item = normalize_subsystem_content_ref(&raw);
        let mut entry = if existing.contains(&item) {
            SubsystemEditOperationData::skipped(
                "add-content",
                item.clone(),
                "уже в составе подсистемы",
            )
        } else {
            model.content.push(item.clone());
            existing.insert(item.clone());
            counters.added += 1;
            SubsystemEditOperationData::applied("add-content", item.clone())
        };
        if item != raw {
            entry = entry.normalized_from(raw);
        }
        log.push(entry);
    }
    Ok(())
}

pub(crate) fn subsystem_edit_remove_content(
    model: &mut SubsystemEditModel,
    value: &Value,
    counters: &mut SubsystemEditCounters,
    log: &mut SubsystemEditLog,
) -> Result<(), String> {
    for item in subsystem_edit_value_list(value)? {
        if let Some(index) = model.content.iter().position(|value| value == &item) {
            model.content.remove(index);
            counters.removed += 1;
            log.push(SubsystemEditOperationData::applied("remove-content", item));
        } else {
            log.push(SubsystemEditOperationData::skipped(
                "remove-content",
                item,
                "не найден в составе подсистемы",
            ));
        }
    }
    Ok(())
}

pub(crate) fn subsystem_edit_add_child(
    model: &mut SubsystemEditModel,
    resolved_path: &Path,
    value: &Value,
    counters: &mut SubsystemEditCounters,
    log: &mut SubsystemEditLog,
    child_stubs: &mut BTreeMap<PathBuf, String>,
    reused_child_subsystems: &mut BTreeMap<PathBuf, Vec<u8>>,
) -> Result<(), String> {
    let child_name = value
        .as_str()
        .ok_or_else(|| "Child subsystem name must be a string".to_string())?
        .to_string();
    validate_subsystem_metadata_name("Child subsystem name", &child_name)?;
    if model.children.iter().any(|value| value == &child_name) {
        log.push(SubsystemEditOperationData::skipped(
            "add-child",
            child_name,
            "уже в составе подсистемы",
        ));
        return Ok(());
    }

    model.children.push(child_name.clone());
    counters.added += 1;
    log.push(SubsystemEditOperationData::applied(
        "add-child",
        child_name.clone(),
    ));

    let parent_dir = resolved_path.parent().unwrap_or_else(|| Path::new(""));
    let parent_base = resolved_path
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("");
    let child_subs_dir = parent_dir.join(parent_base).join("Subsystems");
    let child_xml = child_subs_dir.join(format!("{child_name}.xml"));
    match fs::symlink_metadata(&child_xml) {
        Ok(metadata) if metadata.is_file() && !metadata.file_type().is_symlink() => {
            let preimage = fs::read(&child_xml).map_err(|error| {
                format!(
                    "failed to read existing child subsystem {}: {error}",
                    child_xml.display()
                )
            })?;
            reused_child_subsystems.insert(child_xml.clone(), preimage);
            log.push(SubsystemEditOperationData::skipped(
                "create-child-stub",
                child_xml.display().to_string(),
                "дочерняя подсистема уже существует и переиспользована",
            ));
        }
        Ok(_) => {
            return Err(format!(
                "existing child subsystem target is not a regular file: {}",
                child_xml.display()
            ));
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            child_stubs.insert(
                child_xml,
                child_subsystem_stub_xml(&child_name, &model.version),
            );
        }
        Err(error) => {
            return Err(format!(
                "failed to inspect child subsystem target {}: {error}",
                child_xml.display()
            ));
        }
    }
    Ok(())
}

pub(crate) fn subsystem_edit_remove_child(
    model: &mut SubsystemEditModel,
    value: &Value,
    counters: &mut SubsystemEditCounters,
    log: &mut SubsystemEditLog,
) -> Result<(), String> {
    let child_name = value
        .as_str()
        .ok_or_else(|| "Child subsystem name must be a string".to_string())?
        .to_string();
    validate_subsystem_metadata_name("Child subsystem name", &child_name)?;
    if let Some(index) = model.children.iter().position(|value| value == &child_name) {
        model.children.remove(index);
        counters.removed += 1;
        log.push(SubsystemEditOperationData::applied(
            "remove-child",
            child_name,
        ));
    } else {
        log.push(SubsystemEditOperationData::skipped(
            "remove-child",
            child_name,
            "не найдена в составе подсистемы",
        ));
    }
    Ok(())
}

pub(crate) fn subsystem_edit_set_property(
    model: &mut SubsystemEditModel,
    value: &Value,
    counters: &mut SubsystemEditCounters,
    log: &mut SubsystemEditLog,
) -> Result<(), String> {
    let value = subsystem_edit_object(value)?;
    let prop_name = value
        .get("name")
        .map(json_value_to_python_string)
        .ok_or_else(|| "set-property requires {name, value}".to_string())?;
    let raw_prop_value = value.get("value");
    let prop_value = raw_prop_value
        .map(json_value_to_python_string)
        .unwrap_or_default();
    match prop_name.as_str() {
        "IncludeInCommandInterface" => {
            let canonical = canonical_subsystem_boolean(
                raw_prop_value.ok_or_else(|| "set-property requires {name, value}".to_string())?,
                &prop_name,
            )?;
            model.include_ci = canonical.clone();
            counters.modified += 1;
            log.push(SubsystemEditOperationData::applied(
                "set-property",
                format!("{prop_name}={canonical}"),
            ));
        }
        "UseOneCommand" => {
            let canonical = canonical_subsystem_boolean(
                raw_prop_value.ok_or_else(|| "set-property requires {name, value}".to_string())?,
                &prop_name,
            )?;
            model.use_one_command = canonical.clone();
            counters.modified += 1;
            log.push(SubsystemEditOperationData::applied(
                "set-property",
                format!("{prop_name}={canonical}"),
            ));
        }
        "IncludeHelpInContents" => {
            let canonical = canonical_subsystem_boolean(
                raw_prop_value.ok_or_else(|| "set-property requires {name, value}".to_string())?,
                &prop_name,
            )?;
            model.include_help = canonical.clone();
            counters.modified += 1;
            log.push(SubsystemEditOperationData::applied(
                "set-property",
                format!("{prop_name}={canonical}"),
            ));
        }
        "Synonym" => {
            model.synonym = prop_value.clone();
            counters.modified += 1;
            log.push(SubsystemEditOperationData::applied(
                "set-property",
                format!("Synonym={prop_value}"),
            ));
        }
        "Explanation" => {
            model.explanation = prop_value.clone();
            counters.modified += 1;
            log.push(SubsystemEditOperationData::applied(
                "set-property",
                format!("Explanation={prop_value}"),
            ));
        }
        "Comment" => {
            model.comment = prop_value.clone();
            counters.modified += 1;
            log.push(SubsystemEditOperationData::applied(
                "set-property",
                format!("Comment={prop_value}"),
            ));
        }
        "Picture" => {
            model.picture = prop_value.clone();
            counters.modified += 1;
            log.push(SubsystemEditOperationData::applied(
                "set-property",
                format!("Picture={prop_value}"),
            ));
        }
        "Name" => {
            validate_subsystem_metadata_name("Name", &prop_value)?;
            model.name = prop_value.clone();
            counters.modified += 1;
            log.push(SubsystemEditOperationData::applied(
                "set-property",
                format!("Name={prop_value}"),
            ));
        }
        _ => {
            return Err(format!("Property '{prop_name}' not found in Properties"));
        }
    }
    Ok(())
}

pub(crate) fn validate_subsystem(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> AdapterOutcome {
    let result = (|| -> Result<(bool, String, PathBuf), String> {
        let raw_path = required_path(args, SUBSYSTEM_PATH, "SubsystemPath")?;
        let path = absolutize(raw_path, &context.cwd);
        let detailed = bool_arg(args, &["detailed", "Detailed"]);
        let xml_path = match resolve_subsystem_validate_xml(path) {
            Ok(path) => path,
            Err(stdout) => {
                return Ok((false, format!("{stdout}\n"), PathBuf::new()));
            }
        };

        let mut report = SubsystemValidationReport::new(detailed);
        let text = fs::read_to_string(&xml_path)
            .map_err(|err| format!("failed to read {}: {err}", xml_path.display()))?;
        let source = text.trim_start_matches('\u{feff}');
        let doc = match Document::parse(source) {
            Ok(doc) => doc,
            Err(err) => {
                report.error(format!("1. XML parse error: {err}"));
                let result = report.finish("");
                return Ok((false, result, xml_path));
            }
        };

        let root = doc.root_element();
        let version_literal = root_version_literal(source, root);
        let version = version_literal.as_deref().unwrap_or("");
        match classify_root_version(version_literal.as_deref()) {
            Ok(FormatCompatibility::Supported { .. }) => report.ok("Export format: 2.20"),
            Ok(compatibility) => report.warn(format_compatibility_warning(&compatibility)),
            Err(error) => report.error(error.to_string()),
        }
        let Some(sub) = root.children().find(|node| {
            role_info_element(*node, "Subsystem", Some("http://v8.1c.ru/8.3/MDClasses"))
        }) else {
            report.error("1. Root structure: expected MetaDataObject/Subsystem, not found");
            let result = report.finish("");
            return Ok((false, result, xml_path));
        };
        let uuid_val = sub.attribute("uuid").unwrap_or("");
        if !uuid_val.is_empty() && is_valid_uuid(uuid_val) {
            report.ok(format!(
                "1. Root structure: MetaDataObject/Subsystem, uuid={uuid_val}, version {version}"
            ));
        } else {
            report.error("1. Root structure: invalid or missing uuid");
        }

        let Some(props) = sub.children().find(|node| {
            role_info_element(*node, "Properties", Some("http://v8.1c.ru/8.3/MDClasses"))
        }) else {
            report.error("2. Properties: <Properties> element not found");
            let result = report.finish("");
            return Ok((false, result, xml_path));
        };

        let required_props = [
            "Name",
            "Synonym",
            "Comment",
            "IncludeHelpInContents",
            "IncludeInCommandInterface",
            "UseOneCommand",
            "Explanation",
            "Picture",
            "Content",
        ];
        let missing = required_props
            .iter()
            .filter(|prop| {
                props.children().all(|node| {
                    !role_info_element(node, prop, Some("http://v8.1c.ru/8.3/MDClasses"))
                })
            })
            .copied()
            .collect::<Vec<_>>();
        if missing.is_empty() {
            report.ok("2. Properties: all 9 required properties present");
        } else {
            report.error(format!("2. Properties: missing: {}", missing.join(", ")));
        }

        let sub_name = child_text(props, "Name", Some("http://v8.1c.ru/8.3/MDClasses"))
            .trim()
            .to_string();
        let header = format!("=== Validation: Subsystem.{sub_name} ===");
        report.out("");
        report.out(header.clone());
        report.lines.insert(0, String::new());
        report.lines.insert(0, header);

        match validate_subsystem_metadata_name("Name", &sub_name) {
            Ok(()) => report.ok(format!(
                "3. Name: \"{sub_name}\" - valid metadata identifier"
            )),
            Err(error) => report.error(format!("3. Name: {error}")),
        }

        if let Some(syn) = props
            .children()
            .find(|node| role_info_element(*node, "Synonym", Some("http://v8.1c.ru/8.3/MDClasses")))
        {
            let items = syn
                .children()
                .filter(|node| role_info_element(*node, "item", None));
            let mut item_count = 0usize;
            let mut first_content = String::new();
            for item in items {
                item_count += 1;
                if first_content.is_empty() {
                    if let Some(content) = item
                        .children()
                        .find(|node| role_info_element(*node, "content", None))
                        .and_then(|node| node.text())
                    {
                        if !content.is_empty() {
                            first_content = content.to_string();
                        }
                    }
                }
            }
            if item_count > 0 {
                report.ok(format!(
                    "4. Synonym: \"{first_content}\" ({item_count} lang(s))"
                ));
            } else {
                report.warn("4. Synonym: element exists but no v8:item children");
            }
        } else {
            report.warn("4. Synonym: empty or missing");
        }

        let mut bool_ok = true;
        let mut use_one = String::new();
        for prop in [
            "IncludeHelpInContents",
            "IncludeInCommandInterface",
            "UseOneCommand",
        ] {
            if let Some(node) = props
                .children()
                .find(|node| role_info_element(*node, prop, Some("http://v8.1c.ru/8.3/MDClasses")))
            {
                let value = node.text().unwrap_or("").trim().to_string();
                if prop == "UseOneCommand" {
                    use_one = value.clone();
                }
                if value != "true" && value != "false" {
                    report.error(format!(
                        "5. Boolean property {prop} = \"{value}\" (expected true/false)"
                    ));
                    bool_ok = false;
                }
            }
        }
        if bool_ok {
            report.ok("5. Boolean properties: valid");
        }

        let mut content_items = Vec::<String>::new();
        if let Some(content) = props
            .children()
            .find(|node| role_info_element(*node, "Content", Some("http://v8.1c.ru/8.3/MDClasses")))
        {
            let xr_items = content
                .children()
                .filter(|node| role_info_element(*node, "Item", None))
                .collect::<Vec<_>>();
            if xr_items.is_empty() {
                report.ok("6. Content: empty (no items)");
            } else {
                let mut content_ok = true;
                for item in &xr_items {
                    let type_attr = attribute_by_local_name(*item, "type").unwrap_or("");
                    let text = item.text().unwrap_or("").trim().to_string();
                    content_items.push(text.clone());
                    if type_attr != "xr:MDObjectRef" {
                        report.error(format!(
                            "6. Content item \"{text}\": xsi:type=\"{type_attr}\" (expected xr:MDObjectRef)"
                        ));
                        content_ok = false;
                    }
                    if !is_subsystem_content_ref(&text) && !is_valid_uuid(&text) {
                        report.error(format!(
                            "6. Content item \"{text}\": invalid format (expected Type.Name or UUID)"
                        ));
                        content_ok = false;
                    }
                    if let Some((prefix, _)) = text.split_once('.') {
                        if subsystem_known_plural_types().contains(&prefix) {
                            report.error(format!(
                                "6. Content item \"{text}\": uses plural form \"{prefix}\" (platform requires singular, e.g. Catalog not Catalogs)"
                            ));
                            content_ok = false;
                        }
                    }
                }
                if content_ok {
                    report.ok(format!(
                        "6. Content: {} items, all valid MDObjectRef format",
                        xr_items.len()
                    ));
                }
            }
        } else {
            report.ok("6. Content: empty (no items)");
        }

        if !content_items.is_empty() {
            let duplicates = duplicates_preserve_order(&content_items);
            if duplicates.is_empty() {
                report.ok("7. Content: no duplicates");
            } else {
                report.warn(format!(
                    "7. Content: duplicates found: {}",
                    duplicates.join(", ")
                ));
            }
        }

        let mut child_names = Vec::<String>::new();
        if let Some(child_objs) = sub.children().find(|node| {
            role_info_element(*node, "ChildObjects", Some("http://v8.1c.ru/8.3/MDClasses"))
        }) {
            let children = child_objs
                .children()
                .filter(|node| node.is_element())
                .collect::<Vec<_>>();
            if children.is_empty() {
                report.ok("8. ChildObjects: empty (leaf subsystem)");
            } else {
                let mut child_ok = true;
                for child in children {
                    let local = child.tag_name().name();
                    if local != "Subsystem" {
                        report.error(format!("8. ChildObjects: unexpected element <{local}>"));
                        child_ok = false;
                    } else {
                        let text = child.text().unwrap_or("").trim();
                        match validate_subsystem_metadata_name("Child subsystem name", text) {
                            Ok(()) => child_names.push(text.to_string()),
                            Err(error) => {
                                report.error(format!("8. ChildObjects: {error}"));
                                child_ok = false;
                            }
                        }
                    }
                }
                if child_ok {
                    report.ok(format!(
                        "8. ChildObjects: {} entries, all non-empty",
                        child_names.len()
                    ));
                }
            }
        } else {
            report.ok("8. ChildObjects: empty (leaf subsystem)");
        }

        if !child_names.is_empty() {
            let duplicates = duplicates_preserve_order(&child_names);
            if duplicates.is_empty() {
                report.ok("9. ChildObjects: no duplicates");
            } else {
                report.error(format!(
                    "9. ChildObjects: duplicates: {}",
                    duplicates.join(", ")
                ));
            }

            let subs_dir = subsystem_dir_for_xml(&xml_path).join("Subsystems");
            let missing_files = child_names
                .iter()
                .filter(|name| !subs_dir.join(format!("{name}.xml")).exists())
                .cloned()
                .collect::<Vec<_>>();
            if missing_files.is_empty() {
                report.ok(format!(
                    "10. ChildObjects files: all {} files exist",
                    child_names.len()
                ));
            } else {
                report.warn(format!(
                    "10. ChildObjects files: missing: {}",
                    missing_files.join(", ")
                ));
            }
        }

        let ci_path = subsystem_command_interface_path(&xml_path);
        if ci_path.exists() {
            match fs::read_to_string(&ci_path)
                .map_err(|err| format!("failed to read {}: {err}", ci_path.display()))
                .and_then(|text| {
                    Document::parse(text.trim_start_matches('\u{feff}'))
                        .map(|_| ())
                        .map_err(|err| format!("{err}"))
                }) {
                Ok(()) => report.ok("11. CommandInterface: exists, well-formed"),
                Err(err) => report.warn(format!(
                    "11. CommandInterface: exists but NOT well-formed: {err}"
                )),
            }
        } else {
            report.ok("11. CommandInterface: not present");
        }

        if let Some(picture) = props
            .children()
            .find(|node| role_info_element(*node, "Picture", Some("http://v8.1c.ru/8.3/MDClasses")))
        {
            let children = picture
                .children()
                .filter(|node| node.is_element())
                .collect::<Vec<_>>();
            if children.is_empty() {
                report.ok("12. Picture: empty (not set)");
            } else if let Some(pic_ref) = children
                .iter()
                .find(|node| role_info_element(**node, "Ref", None))
            {
                let ref_text = pic_ref.text().unwrap_or("");
                if ref_text.starts_with("CommonPicture.") {
                    report.ok(format!("12. Picture: {ref_text}"));
                } else {
                    report.warn(format!(
                        "12. Picture: \"{ref_text}\" (expected CommonPicture.XXX)"
                    ));
                }
            } else {
                report.warn("12. Picture: has children but no xr:Ref content");
            }
        } else {
            report.ok("12. Picture: empty (not set)");
        }

        if use_one == "true" {
            if content_items.len() == 1 {
                report.ok("13. UseOneCommand: true, Content has exactly 1 item");
            } else {
                report.warn(format!(
                    "13. UseOneCommand: true but Content has {} items (expected 1)",
                    content_items.len()
                ));
            }
        } else {
            report.ok("13. UseOneCommand: false (no constraint)");
        }

        let ok = report.errors == 0;
        let result = report.finish(&sub_name);
        Ok((ok, result, xml_path))
    })();

    match result {
        Ok((ok, text, artifact)) => {
            let artifacts = if artifact.as_os_str().is_empty() {
                Vec::new()
            } else {
                vec![artifact.display().to_string()]
            };
            AdapterOutcome {
                ok,
                summary: if ok {
                    "unica.subsystem.validate completed with native subsystem validator".to_string()
                } else {
                    "unica.subsystem.validate failed in native subsystem validator".to_string()
                },
                changes: Vec::new(),
                warnings: Vec::new(),
                errors: if ok {
                    Vec::new()
                } else {
                    vec![text.trim().to_string()]
                },
                artifacts,
                stdout: Some(text),
                stderr: Some(String::new()),
                command: None,
            }
        }
        Err(error) => AdapterOutcome {
            ok: false,
            summary: "unica.subsystem.validate failed in native subsystem validator".to_string(),
            changes: Vec::new(),
            warnings: Vec::new(),
            errors: vec![error.clone()],
            artifacts: Vec::new(),
            stdout: None,
            stderr: Some(format!("{error}\n")),
            command: None,
        },
    }
}

pub(crate) fn validate_subsystem_owner_path(
    path: &Path,
    context: &WorkspaceContext,
) -> Result<(), String> {
    let outcome = validate_subsystem(&subsystem_validation_args(path), context);
    require_subsystem_validation(&outcome).map_err(|error| {
        format!(
            "subsystem owner validation failed for {}: {error}",
            path.display()
        )
    })
}

#[cfg(test)]
mod subsystem_info_typed_result_tests {
    use super::*;
    use crate::infrastructure::platform::testing::{
        create_dir_symlink_for_test, create_file_symlink_for_test,
    };
    use std::time::{SystemTime, UNIX_EPOCH};

    fn workspace(name: &str, with_command_interface: bool) -> (WorkspaceContext, PathBuf) {
        let root = std::env::temp_dir().join(format!(
            "unica-subsystem-info-typed-{name}-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(root.join("src/Subsystems/Продажи/Ext")).unwrap();
        fs::write(
            root.join("src/Subsystems/Продажи.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Subsystem><Properties><Name>Продажи</Name><IncludeInCommandInterface>true</IncludeInCommandInterface><Content xmlns:xr="http://v8.1c.ru/8.3/xcf/readable"><xr:Item>Catalog.Goods</xr:Item></Content></Properties></Subsystem></MetaDataObject>"#,
        )
        .unwrap();
        if with_command_interface {
            fs::write(
                root.join("src/Subsystems/Продажи/Ext/CommandInterface.xml"),
                r#"<CommandInterface xmlns="http://v8.1c.ru/8.3/xcf/extrnprops" version="2.20"><CommandsVisibility><Command name="Catalog.Goods.Create"><Visibility xmlns:xr="http://v8.1c.ru/8.3/xcf/readable"><xr:Common>false</xr:Common></Visibility></Command></CommandsVisibility></CommandInterface>"#,
            )
            .unwrap();
        }
        let physical_root = root.canonicalize().unwrap();
        let context = WorkspaceContext {
            cwd: physical_root.clone(),
            workspace_root: physical_root.clone(),
            cache_root: physical_root.join(".build/unica"),
            workspace_epoch: 1,
        };
        (context, root)
    }

    fn info_args() -> Map<String, Value> {
        Map::from_iter([(
            "SubsystemPath".to_string(),
            json!("src/Subsystems/Продажи.xml"),
        )])
    }

    fn write_configuration(root: &Path, registrations: &[&str]) {
        let registrations = registrations
            .iter()
            .map(|name| format!("<Subsystem>{name}</Subsystem>"))
            .collect::<String>();
        fs::write(
            root.join("Configuration.xml"),
            format!(
                r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Configuration><Properties><Name>Test</Name></Properties><ChildObjects>{registrations}</ChildObjects></Configuration></MetaDataObject>"#
            ),
        )
        .unwrap();
    }

    fn create_directory_symlink_or_skip(target: &Path, link: &Path) -> bool {
        match create_dir_symlink_for_test(target, link) {
            Some(Ok(())) => true,
            Some(Err(error)) if error.raw_os_error() == Some(1314) => false,
            Some(Err(error)) => panic!("failed to create directory symlink fixture: {error}"),
            None => false,
        }
    }

    fn create_file_symlink_or_skip(target: &Path, link: &Path) -> bool {
        match create_file_symlink_for_test(target, link) {
            Some(Ok(())) => true,
            Some(Err(error)) if error.raw_os_error() == Some(1314) => false,
            Some(Err(error)) => panic!("failed to create file symlink fixture: {error}"),
            None => false,
        }
    }

    /// `Mode=tree` answered the hierarchy question; typing must not drop the
    /// capability, only change how it is selected.
    #[test]
    fn pointing_at_the_subsystems_folder_answers_only_with_tree() {
        let (context, root) = workspace("tree", false);
        fs::write(
            root.join("src/Configuration.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Configuration><Properties><Name>Test</Name></Properties><ChildObjects><Subsystem>СтандартныеПодсистемы</Subsystem><Subsystem>Служебные</Subsystem></ChildObjects></Configuration></MetaDataObject>"#,
        )
        .unwrap();
        fs::write(
            root.join("src/Subsystems/СтандартныеПодсистемы.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Subsystem><Properties><Name>СтандартныеПодсистемы</Name><IncludeInCommandInterface>true</IncludeInCommandInterface><Content/></Properties><ChildObjects><Subsystem>Обсуждения</Subsystem></ChildObjects></Subsystem></MetaDataObject>"#,
        )
        .unwrap();
        fs::create_dir_all(root.join("src/Subsystems/СтандартныеПодсистемы/Subsystems")).unwrap();
        fs::write(
            root.join("src/Subsystems/СтандартныеПодсистемы/Subsystems/Обсуждения.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" xmlns:xr="http://v8.1c.ru/8.3/xcf/readable" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" version="2.20"><Subsystem><Properties><Name>Обсуждения</Name><IncludeInCommandInterface>true</IncludeInCommandInterface><Content><xr:Item xsi:type="xr:MDObjectRef">CommonForm.Обсуждение</xr:Item></Content></Properties><ChildObjects/></Subsystem></MetaDataObject>"#,
        )
        .unwrap();
        fs::write(
            root.join("src/Subsystems/Служебные.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Subsystem><Properties><Name>Служебные</Name><IncludeInCommandInterface>false</IncludeInCommandInterface><Content/></Properties><ChildObjects><Subsystem>ФоновыеЗадания</Subsystem></ChildObjects></Subsystem></MetaDataObject>"#,
        )
        .unwrap();
        fs::create_dir_all(root.join("src/Subsystems/Служебные/Subsystems")).unwrap();
        fs::write(
            root.join("src/Subsystems/Служебные/Subsystems/ФоновыеЗадания.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Subsystem><Properties><Name>ФоновыеЗадания</Name><IncludeInCommandInterface>true</IncludeInCommandInterface><Content/></Properties><ChildObjects/></Subsystem></MetaDataObject>"#,
        )
        .unwrap();
        let args = Map::from_iter([("SubsystemPath".to_string(), json!("src/Subsystems"))]);

        let execution = analyze_subsystem_info(&args, &context);

        assert!(execution.outcome.ok, "{:?}", execution.outcome);
        assert_eq!(
            serde_json::to_value(execution.data.expect("the folder answers with a tree")).unwrap(),
            serde_json::json!({
                "tree": [
                    {
                        "name": "СтандартныеПодсистемы",
                        "content": 0,
                        "children": [
                            {"name": "Обсуждения", "content": 1, "children": []}
                        ]
                    },
                    {
                        "name": "Служебные",
                        "content": 0,
                        "children": [
                            {"name": "ФоновыеЗадания", "content": 0, "children": []}
                        ]
                    }
                ]
            })
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn nested_subsystems_folder_is_not_a_tree_target() {
        let (context, root) = workspace("nested-tree", false);
        fs::write(
            root.join("src/Configuration.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Configuration><Properties><Name>Test</Name></Properties><ChildObjects><Subsystem>СтандартныеПодсистемы</Subsystem></ChildObjects></Configuration></MetaDataObject>"#,
        )
        .unwrap();
        fs::write(
            root.join("src/Subsystems/СтандартныеПодсистемы.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Subsystem><Properties><Name>СтандартныеПодсистемы</Name><IncludeInCommandInterface>true</IncludeInCommandInterface><Content/></Properties><ChildObjects><Subsystem>Обсуждения</Subsystem></ChildObjects></Subsystem></MetaDataObject>"#,
        )
        .unwrap();
        fs::create_dir_all(root.join("src/Subsystems/СтандартныеПодсистемы/Subsystems")).unwrap();
        fs::write(
            root.join("src/Subsystems/СтандартныеПодсистемы/Subsystems/Обсуждения.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Subsystem><Properties><Name>Обсуждения</Name><IncludeInCommandInterface>true</IncludeInCommandInterface><Content/></Properties><ChildObjects/></Subsystem></MetaDataObject>"#,
        )
        .unwrap();
        let args = Map::from_iter([(
            "SubsystemPath".to_string(),
            json!("src/Subsystems/СтандартныеПодсистемы/Subsystems"),
        )]);

        let execution = analyze_subsystem_info(&args, &context);

        assert!(!execution.outcome.ok, "{:?}", execution.outcome);
        assert!(execution.data.is_none());
        assert!(
            execution
                .outcome
                .errors
                .iter()
                .any(|error| error.contains("root Subsystems directory")),
            "{:?}",
            execution.outcome
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn nested_configuration_cannot_lend_registered_topology_to_a_local_xml() {
        let (context, root) = workspace("nested-configuration-decoy", false);
        let source_root = root.join("src");
        write_configuration(&source_root, &["Parent"]);
        fs::write(
            source_root.join("Subsystems/Parent.xml"),
            child_subsystem_stub_xml("Parent", "2.20"),
        )
        .unwrap();
        let nested_root = source_root.join("Subsystems/Parent");
        fs::create_dir_all(nested_root.join("Subsystems")).unwrap();
        write_configuration(&nested_root, &["Child"]);
        fs::write(
            nested_root.join("Subsystems/Child.xml"),
            child_subsystem_stub_xml("Child", "2.20"),
        )
        .unwrap();
        let args = Map::from_iter([(
            "SubsystemPath".to_string(),
            json!("src/Subsystems/Parent/Subsystems/Child.xml"),
        )]);

        let execution = analyze_subsystem_info(&args, &context);

        assert!(execution.outcome.ok, "{:?}", execution.outcome);
        let SubsystemInfoAnswer::Subsystem(data) = execution
            .data
            .expect("the unregistered XML remains local data")
        else {
            panic!("a concrete XML must answer with subsystem data");
        };
        assert!(
            data.tree.is_none(),
            "a nested Configuration.xml must not redefine the registered source root"
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn unconfigured_configuration_source_cannot_lend_registered_topology() {
        let (context, root) = workspace("unconfigured-configuration-decoy", false);
        fs::write(
            root.join("v8project.yaml"),
            "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: configured\n",
        )
        .unwrap();
        let configured = root.join("configured");
        fs::create_dir_all(configured.join("Subsystems")).unwrap();
        write_configuration(&configured, &[]);

        let decoy = root.join("src");
        write_configuration(&decoy, &["Продажи"]);
        let args = Map::from_iter([(
            "SubsystemPath".to_string(),
            json!("src/Subsystems/Продажи.xml"),
        )]);

        let execution = analyze_subsystem_info(&args, &context);

        assert!(execution.outcome.ok, "{:?}", execution.outcome);
        let SubsystemInfoAnswer::Subsystem(data) =
            execution.data.expect("the exact XML remains local data")
        else {
            panic!("a concrete XML must answer with subsystem data");
        };
        assert!(
            data.tree.is_none(),
            "an undeclared source root must not lend registered topology"
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn single_subsystem_directory_alias_is_not_an_info_target() {
        let (context, root) = workspace("single-directory-alias", false);
        let source_root = root.join("src");
        write_configuration(&source_root, &["Parent"]);
        fs::write(
            source_root.join("Subsystems/Parent.xml"),
            child_subsystem_stub_xml("Parent", "2.20"),
        )
        .unwrap();
        fs::create_dir_all(source_root.join("Subsystems/Parent")).unwrap();
        let args = Map::from_iter([("SubsystemPath".to_string(), json!("src/Subsystems/Parent"))]);

        let execution = analyze_subsystem_info(&args, &context);

        assert!(!execution.outcome.ok, "{:?}", execution.outcome);
        assert!(execution.data.is_none());
        assert!(
            execution
                .outcome
                .errors
                .iter()
                .any(|error| error.contains("XML file or the root Subsystems directory")),
            "{:?}",
            execution.outcome
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn concrete_subsystem_contains_its_root_chain_and_complete_descendant_tree() {
        let (context, root) = workspace("focused-tree", false);
        fs::write(
            root.join("src/Configuration.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Configuration><Properties><Name>Test</Name></Properties><ChildObjects><Subsystem>Корень</Subsystem><Subsystem>ДругойКорень</Subsystem></ChildObjects></Configuration></MetaDataObject>"#,
        )
        .unwrap();
        fs::write(
            root.join("src/Subsystems/Корень.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Subsystem><Properties><Name>Корень</Name><IncludeInCommandInterface>true</IncludeInCommandInterface><Content/></Properties><ChildObjects><Subsystem>Родитель</Subsystem><Subsystem>Соседняя</Subsystem></ChildObjects></Subsystem></MetaDataObject>"#,
        )
        .unwrap();
        fs::write(
            root.join("src/Subsystems/ДругойКорень.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Subsystem><Properties><Name>ДругойКорень</Name><IncludeInCommandInterface>true</IncludeInCommandInterface><Content/></Properties><ChildObjects/></Subsystem></MetaDataObject>"#,
        )
        .unwrap();
        fs::create_dir_all(root.join("src/Subsystems/Корень/Subsystems/Родитель/Subsystems"))
            .unwrap();
        fs::write(
            root.join("src/Subsystems/Корень/Subsystems/Родитель.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Subsystem><Properties><Name>Родитель</Name><IncludeInCommandInterface>true</IncludeInCommandInterface><Content/></Properties><ChildObjects><Subsystem>Выбранная</Subsystem></ChildObjects></Subsystem></MetaDataObject>"#,
        )
        .unwrap();
        fs::write(
            root.join("src/Subsystems/Корень/Subsystems/Соседняя.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Subsystem><Properties><Name>Соседняя</Name><IncludeInCommandInterface>true</IncludeInCommandInterface><Content/></Properties><ChildObjects/></Subsystem></MetaDataObject>"#,
        )
        .unwrap();
        fs::write(
            root.join("src/Subsystems/Корень/Subsystems/Родитель/Subsystems/Выбранная.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Subsystem><Properties><Name>Выбранная</Name><IncludeInCommandInterface>true</IncludeInCommandInterface><Content/></Properties><ChildObjects><Subsystem>Потомок</Subsystem></ChildObjects></Subsystem></MetaDataObject>"#,
        )
        .unwrap();
        fs::create_dir_all(
            root.join("src/Subsystems/Корень/Subsystems/Родитель/Subsystems/Выбранная/Subsystems"),
        )
        .unwrap();
        fs::write(
            root.join("src/Subsystems/Корень/Subsystems/Родитель/Subsystems/Выбранная/Subsystems/Потомок.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Subsystem><Properties><Name>Потомок</Name><IncludeInCommandInterface>true</IncludeInCommandInterface><Content/></Properties><ChildObjects/></Subsystem></MetaDataObject>"#,
        )
        .unwrap();
        let args = Map::from_iter([(
            "SubsystemPath".to_string(),
            json!("src/Subsystems/Корень/Subsystems/Родитель/Subsystems/Выбранная.xml"),
        )]);

        let execution = analyze_subsystem_info(&args, &context);

        assert!(execution.outcome.ok, "{:?}", execution.outcome);
        let SubsystemInfoAnswer::Subsystem(data) =
            execution.data.expect("a file answers with subsystem data")
        else {
            panic!("a file must not answer as a directory tree");
        };
        assert_eq!(
            serde_json::to_value(&data.tree).unwrap(),
            serde_json::json!([{
                "name": "Корень",
                "content": 0,
                "children": [{
                    "name": "Родитель",
                    "content": 0,
                    "children": [{
                        "name": "Выбранная",
                        "content": 0,
                        "children": [{"name": "Потомок", "content": 0, "children": []}]
                    }]
                }]
            }])
        );
        let serialized = serde_json::to_value(data).unwrap();
        assert!(serialized.get("functionalSubsystems").is_none());
        assert!(serialized.get("interfaceSubsystems").is_none());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn unregistered_alias_keeps_local_data_without_borrowing_a_registered_tree() {
        let (context, root) = workspace("unregistered-alias", false);
        let source_root = root.join("src");
        write_configuration(&source_root, &["Sales"]);
        fs::write(
            source_root.join("Subsystems/Sales.xml"),
            child_subsystem_stub_xml("Sales", "2.20"),
        )
        .unwrap();
        fs::write(
            source_root.join("Subsystems/Copy.xml"),
            child_subsystem_stub_xml("Sales", "2.20"),
        )
        .unwrap();
        let args = Map::from_iter([(
            "SubsystemPath".to_string(),
            json!("src/Subsystems/Copy.xml"),
        )]);

        let execution = analyze_subsystem_info(&args, &context);

        assert!(execution.outcome.ok, "{:?}", execution.outcome);
        let SubsystemInfoAnswer::Subsystem(data) = execution
            .data
            .expect("the alias remains readable local data")
        else {
            panic!("a concrete XML must answer with local subsystem data");
        };
        assert_eq!(data.name, "Sales");
        assert!(
            data.tree.is_none(),
            "an unregistered file stem cannot borrow the registered Sales identity"
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn parent_component_before_subsystems_is_rejected_before_any_registered_read() {
        let (context, root) = workspace("parent-component", false);
        let source_root = root.join("src");
        write_configuration(&source_root, &["Sales"]);
        fs::write(
            source_root.join("Subsystems/Sales.xml"),
            child_subsystem_stub_xml("Sales", "2.20"),
        )
        .unwrap();

        let outside = root.join("outside");
        fs::create_dir_all(outside.join("nested")).unwrap();
        fs::create_dir_all(outside.join("Subsystems")).unwrap();
        write_configuration(&outside, &["Sales"]);
        fs::write(
            outside.join("Subsystems/Sales.xml"),
            child_subsystem_stub_xml("Sales", "2.20"),
        )
        .unwrap();
        if !create_directory_symlink_or_skip(&outside.join("nested"), &source_root.join("tmp")) {
            let _ = fs::remove_dir_all(root);
            return;
        }
        let args = Map::from_iter([(
            "SubsystemPath".to_string(),
            json!("src/tmp/../Subsystems/Sales.xml"),
        )]);

        let execution = analyze_subsystem_info(&args, &context);

        assert!(!execution.outcome.ok, "{:?}", execution.outcome);
        assert!(execution.data.is_none());
        assert!(
            execution
                .outcome
                .errors
                .iter()
                .any(|error| error.contains("parent path component")),
            "{:?}",
            execution.outcome
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn bak_symlink_alias_is_not_a_subsystem_info_xml_target() {
        let (context, root) = workspace("bak-alias", false);
        let source_root = root.join("src");
        write_configuration(&source_root, &["Sales"]);
        let registered = source_root.join("Subsystems/Sales.xml");
        fs::write(&registered, child_subsystem_stub_xml("Sales", "2.20")).unwrap();
        let alias = source_root.join("Subsystems/Sales.bak");
        if !create_file_symlink_or_skip(&registered, &alias) {
            let _ = fs::remove_dir_all(root);
            return;
        }
        let args = Map::from_iter([(
            "SubsystemPath".to_string(),
            json!("src/Subsystems/Sales.bak"),
        )]);

        let execution = analyze_subsystem_info(&args, &context);

        assert!(!execution.outcome.ok, "{:?}", execution.outcome);
        assert!(execution.data.is_none());
        assert!(
            execution
                .outcome
                .errors
                .iter()
                .any(|error| error.contains("exact lowercase .xml")),
            "{:?}",
            execution.outcome
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn xml_symlink_alias_is_not_a_subsystem_info_target() {
        let (context, root) = workspace("xml-symlink-alias", false);
        let registered = root.join("src/Subsystems/Продажи.xml");
        let alias = root.join("src/Subsystems/Alias.xml");
        if !create_file_symlink_or_skip(&registered, &alias) {
            let _ = fs::remove_dir_all(root);
            return;
        }
        let args = Map::from_iter([(
            "SubsystemPath".to_string(),
            json!("src/Subsystems/Alias.xml"),
        )]);

        let execution = analyze_subsystem_info(&args, &context);

        assert!(!execution.outcome.ok, "{:?}", execution.outcome);
        assert!(execution.data.is_none());
        assert!(
            execution
                .outcome
                .errors
                .iter()
                .any(|error| error.contains("regular lowercase .xml")),
            "{:?}",
            execution.outcome
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn extensionless_alias_is_not_a_subsystem_info_xml_target() {
        let (context, root) = workspace("extensionless-alias", false);
        let source_root = root.join("src");
        write_configuration(&source_root, &["Sales"]);
        fs::write(
            source_root.join("Subsystems/Sales.xml"),
            child_subsystem_stub_xml("Sales", "2.20"),
        )
        .unwrap();
        fs::write(
            source_root.join("Subsystems/Sales"),
            child_subsystem_stub_xml("Sales", "2.20"),
        )
        .unwrap();
        let args = Map::from_iter([("SubsystemPath".to_string(), json!("src/Subsystems/Sales"))]);

        let execution = analyze_subsystem_info(&args, &context);

        assert!(!execution.outcome.ok, "{:?}", execution.outcome);
        assert!(execution.data.is_none());
        assert!(
            execution
                .outcome
                .errors
                .iter()
                .any(|error| error.contains("exact lowercase .xml")),
            "{:?}",
            execution.outcome
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn uppercase_xml_alias_is_not_a_subsystem_info_xml_target() {
        let (context, root) = workspace("uppercase-xml-alias", false);
        let source_root = root.join("src");
        fs::write(
            source_root.join("Subsystems/Sales.XML"),
            child_subsystem_stub_xml("Sales", "2.20"),
        )
        .unwrap();
        let args = Map::from_iter([(
            "SubsystemPath".to_string(),
            json!("src/Subsystems/Sales.XML"),
        )]);

        let execution = analyze_subsystem_info(&args, &context);

        assert!(!execution.outcome.ok, "{:?}", execution.outcome);
        assert!(execution.data.is_none());
        assert!(
            execution
                .outcome
                .errors
                .iter()
                .any(|error| error.contains("exact lowercase .xml")),
            "{:?}",
            execution.outcome
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn root_subsystems_symlink_is_not_followed_for_a_tree_answer() {
        let (context, root) = workspace("root-symlink", false);
        let source_root = root.join("src");
        fs::remove_dir_all(source_root.join("Subsystems")).unwrap();
        write_configuration(&source_root, &["Sales"]);

        let external = root.join("external-root");
        fs::create_dir_all(external.join("Subsystems")).unwrap();
        write_configuration(&external, &["Sales"]);
        fs::write(
            external.join("Subsystems/Sales.xml"),
            child_subsystem_stub_xml("Sales", "2.20"),
        )
        .unwrap();
        if !create_directory_symlink_or_skip(
            &external.join("Subsystems"),
            &source_root.join("Subsystems"),
        ) {
            let _ = fs::remove_dir_all(root);
            return;
        }
        let args = Map::from_iter([("SubsystemPath".to_string(), json!("src/Subsystems"))]);

        let execution = analyze_subsystem_info(&args, &context);

        assert!(!execution.outcome.ok, "{:?}", execution.outcome);
        assert!(execution.data.is_none());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn nested_subsystems_symlink_is_not_followed_for_a_tree_answer() {
        let (context, root) = workspace("nested-symlink", false);
        let source_root = root.join("src");
        write_configuration(&source_root, &["Root"]);
        fs::write(
            source_root.join("Subsystems/Root.xml"),
            child_subsystem_stub_xml("Root", "2.20").replacen(
                "<ChildObjects/>",
                "<ChildObjects><Subsystem>Child</Subsystem></ChildObjects>",
                1,
            ),
        )
        .unwrap();

        let external = root.join("external-nested");
        fs::create_dir_all(external.join("Subsystems")).unwrap();
        write_configuration(&external, &["Child"]);
        fs::write(
            external.join("Subsystems/Child.xml"),
            child_subsystem_stub_xml("Child", "2.20"),
        )
        .unwrap();
        let nested = source_root.join("Subsystems/Root/Subsystems");
        fs::create_dir_all(nested.parent().unwrap()).unwrap();
        if !create_directory_symlink_or_skip(&external.join("Subsystems"), &nested) {
            let _ = fs::remove_dir_all(root);
            return;
        }
        let args = Map::from_iter([(
            "SubsystemPath".to_string(),
            json!("src/Subsystems/Root/Subsystems"),
        )]);

        let execution = analyze_subsystem_info(&args, &context);

        assert!(!execution.outcome.ok, "{:?}", execution.outcome);
        assert!(execution.data.is_none());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn subsystem_info_answers_content_and_command_interface_at_once() {
        let (context, root) = workspace("full", true);

        let execution = analyze_subsystem_info(&info_args(), &context);

        assert!(execution.outcome.ok, "{:?}", execution.outcome);
        assert!(execution.outcome.stdout.is_none());
        let SubsystemInfoAnswer::Subsystem(data) =
            execution.data.expect("subsystem info answers with data")
        else {
            panic!("a file must answer as one subsystem");
        };
        assert_eq!(data.name, "Продажи");
        assert_eq!(data.content, vec!["Catalog.Goods".to_string()]);
        assert!(data.tree.is_none());
        let interface = data
            .command_interface
            .expect("the subsystem ships a command interface");
        assert_eq!(interface.visibility.len(), 1);
        assert!(!interface.visibility[0].visible);
        let _ = fs::remove_dir_all(root);
    }

    /// An absent `CommandInterface.xml` and an interface that hides nothing are
    /// different facts, so one is `null` and the other is an empty list.
    #[test]
    fn a_missing_command_interface_is_null_not_an_empty_interface() {
        let (context, root) = workspace("bare", false);

        let execution = analyze_subsystem_info(&info_args(), &context);

        assert!(execution.outcome.ok, "{:?}", execution.outcome);
        let SubsystemInfoAnswer::Subsystem(data) =
            execution.data.expect("subsystem info answers with data")
        else {
            panic!("a file must answer as one subsystem");
        };
        assert!(data.tree.is_none());
        assert!(data.command_interface.is_none());
        let _ = fs::remove_dir_all(root);
    }
}

#[cfg(test)]
pub(crate) fn analyze_subsystem_info(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> SubsystemInfoExecution {
    prepare_subsystem_info_with_checkpoint(args, context, || Ok(()))
        .map(|prepared| prepared.execution)
        .unwrap_or_else(subsystem_info_failure)
}

pub(crate) fn analyze_subsystem_info_cancellable(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
    cancellation: &CancellationToken,
    deadline: ProviderDeadline,
) -> SubsystemInfoExecution {
    prepare_subsystem_info(args, context, cancellation, deadline)
        .map(|prepared| prepared.execution)
        .unwrap_or_else(|error| subsystem_info_failure(error.to_string()))
}

#[derive(Debug)]
pub(crate) enum SubsystemInfoPreparationError {
    Control(String),
    Provider(String),
}

impl SubsystemInfoPreparationError {
    pub(crate) fn is_control(&self) -> bool {
        matches!(self, Self::Control(_))
    }
}

impl std::fmt::Display for SubsystemInfoPreparationError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Control(message) | Self::Provider(message) => formatter.write_str(message),
        }
    }
}

impl std::error::Error for SubsystemInfoPreparationError {}

pub(crate) fn prepare_subsystem_info(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
    cancellation: &CancellationToken,
    deadline: ProviderDeadline,
) -> Result<PreparedSubsystemInfo, SubsystemInfoPreparationError> {
    let control_error = std::cell::RefCell::new(None);
    let prepared = prepare_subsystem_info_with_checkpoint(args, context, || {
        let checkpoint = subsystem_info_checkpoint(cancellation, deadline);
        if let Err(error) = &checkpoint {
            if let Some(error) = subsystem_info_control_error(error) {
                control_error.replace(Some(error));
            }
        }
        checkpoint
    });
    prepared.map_err(|error| {
        control_error
            .into_inner()
            .unwrap_or(SubsystemInfoPreparationError::Provider(error))
    })
}

fn subsystem_info_checkpoint(
    cancellation: &CancellationToken,
    deadline: ProviderDeadline,
) -> io::Result<()> {
    if cancellation.is_cancelled() {
        return Err(io::Error::new(
            io::ErrorKind::Interrupted,
            cancelled_error("unica.subsystem.info stopped during registered topology read"),
        ));
    }
    if deadline.remaining().is_zero() {
        return Err(io::Error::new(
            io::ErrorKind::TimedOut,
            "unica.subsystem.info provider deadline exceeded",
        ));
    }
    Ok(())
}

pub(crate) fn ensure_subsystem_info_control(
    cancellation: &CancellationToken,
    deadline: ProviderDeadline,
) -> Result<(), SubsystemInfoPreparationError> {
    subsystem_info_checkpoint(cancellation, deadline).map_err(|error| {
        subsystem_info_control_error(&error).unwrap_or_else(|| {
            SubsystemInfoPreparationError::Provider(subsystem_info_checkpoint_message(error))
        })
    })
}

fn subsystem_info_control_error(error: &io::Error) -> Option<SubsystemInfoPreparationError> {
    let message = error.to_string();
    if error.kind() == io::ErrorKind::Interrupted
        && message.starts_with(crate::domain::cancellation::CANCELLED_PREFIX)
    {
        return Some(SubsystemInfoPreparationError::Control(message));
    }
    if error.kind() == io::ErrorKind::TimedOut {
        return Some(SubsystemInfoPreparationError::Control(format!(
            "subsystem info snapshot failed: {message}"
        )));
    }
    None
}

fn prepare_subsystem_info_with_checkpoint(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
    mut checkpoint: impl FnMut() -> io::Result<()>,
) -> Result<PreparedSubsystemInfo, String> {
    checkpoint().map_err(subsystem_info_checkpoint_message)?;
    let raw_path = required_path(args, SUBSYSTEM_PATH, "SubsystemPath")?;
    reject_subsystem_info_parent_components(&raw_path)?;
    let path = absolutize(raw_path, &context.cwd);

    // Only the source root's `Subsystems/` folder asks the hierarchy question.
    // A nested folder would omit the required ancestor chain, while a
    // single-subsystem directory is not the public XML address at all.
    if path.is_dir() {
        if path.file_name().and_then(|name| name.to_str()) != Some("Subsystems") {
            return Err(
                "SubsystemPath must name an XML file or the root Subsystems directory".to_string(),
            );
        }
        let (source_root, parent_address) =
            subsystem_tree_scope(&path, context)?.ok_or_else(|| {
                "the requested Subsystems directory is outside every configured source root"
                    .to_string()
            })?;
        if !parent_address.is_empty() {
            return Err(
                "SubsystemPath must name the root Subsystems directory, not a nested Subsystems directory"
                    .to_string(),
            );
        }
        let topology = capture_registered_subsystem_topology(&source_root, &mut checkpoint)
            .map_err(|error| error.to_string())?;
        let tree = topology
            .roots
            .iter()
            .map(subsystem_tree_node)
            .collect::<Vec<_>>();
        let summary = format!(
            "unica.subsystem.info described {} subsystem(s) in the requested scope",
            tree.len()
        );
        let format_documents = topology_format_documents(&source_root, &topology);
        checkpoint().map_err(subsystem_info_checkpoint_message)?;
        return Ok(PreparedSubsystemInfo {
            execution: subsystem_info_success(SubsystemInfoAnswer::Tree { tree }, path, summary),
            format_documents,
        });
    }

    if path.extension().and_then(|extension| extension.to_str()) != Some("xml") {
        return Err(
            "SubsystemPath must name an exact lowercase .xml file or the root Subsystems directory"
                .to_string(),
        );
    }

    let metadata = fs::symlink_metadata(&path)
        .map_err(|error| format!("[ERROR] File not found: {}: {error}", path.display()))?;
    if crate::infrastructure::platform::filesystem::metadata_is_link_or_reparse_point(&metadata)
        || !metadata.is_file()
    {
        return Err(
            "SubsystemPath must name a regular lowercase .xml file, not a filesystem alias"
                .to_string(),
        );
    }
    let xml_path = path;
    let scope = subsystem_descriptor_scope(&xml_path, context)?;
    let mut format_documents = Vec::new();
    let (descriptor_bytes, tree) = if let Some((source_root, address_names, address)) = scope {
        let topology = capture_registered_subsystem_topology(&source_root, &mut checkpoint)
            .map_err(|error| error.to_string())?;
        format_documents = topology_format_documents(&source_root, &topology);
        if let Some(registered) = subsystem_node_by_address(&topology.roots, &address) {
            let relative = xml_path.strip_prefix(&source_root).map_err(|_| {
                "registered subsystem descriptor escaped its lexical source root".to_string()
            })?;
            let descriptor = topology
                .dependency_documents()
                .iter()
                .find(|document| document.path == relative)
                .ok_or_else(|| {
                    format!("registered subsystem `{address}` has no captured descriptor")
                })?;
            let tree = focused_subsystem_tree_from_topology(
                &topology,
                &address_names,
                &address,
                registered,
            )?;
            (descriptor.bytes.clone(), Some(vec![tree]))
        } else {
            let bytes = read_subsystem_info_file(&xml_path, &mut checkpoint)?;
            format_documents.push(SubsystemInfoFormatDocument {
                path: xml_path.clone(),
                bytes: bytes.clone(),
            });
            (bytes, None)
        }
    } else {
        let bytes = read_subsystem_info_file(&xml_path, &mut checkpoint)?;
        format_documents.push(SubsystemInfoFormatDocument {
            path: xml_path.clone(),
            bytes: bytes.clone(),
        });
        (bytes, None)
    };
    let descriptor_text = std::str::from_utf8(&descriptor_bytes)
        .map_err(|error| format!("failed to read {}: {error}", xml_path.display()))?;
    let (data, _) = parse_subsystem_info_data(&xml_path, descriptor_text)?;
    if tree.is_some() {
        let selected_name = xml_path
            .file_name()
            .and_then(|name| name.to_str())
            .and_then(|name| name.strip_suffix(".xml"))
            .expect("registered subsystem shape was checked before capture");
        if data.name != selected_name {
            return Err(format!(
                "registered subsystem `{selected_name}` declares Name `{}` instead of `{selected_name}`",
                data.name
            ));
        }
    }
    checkpoint().map_err(subsystem_info_checkpoint_message)?;

    let command_interface_path = subsystem_command_interface_path(&xml_path);
    checkpoint().map_err(subsystem_info_checkpoint_message)?;
    let command_interface = if command_interface_path.is_file() {
        let bytes = read_subsystem_info_file(&command_interface_path, &mut checkpoint)?;
        let text = std::str::from_utf8(&bytes).map_err(|error| {
            format!(
                "failed to read {}: {error}",
                command_interface_path.display()
            )
        })?;
        let data = parse_subsystem_command_interface_data(&command_interface_path, text)?;
        format_documents.push(SubsystemInfoFormatDocument {
            path: command_interface_path,
            bytes,
        });
        Some(data)
    } else {
        None
    };
    checkpoint().map_err(subsystem_info_checkpoint_message)?;

    // Overview, content, ci and full were slices of one subsystem chosen to
    // keep a printed report short. Data carries all of them at once.
    let support = object_support_state(&xml_path);
    checkpoint().map_err(subsystem_info_checkpoint_message)?;
    let result = SubsystemInfoResult {
        name: data.name,
        synonym: subsystem_optional(data.synonym),
        comment: subsystem_optional(data.comment),
        explanation: subsystem_optional(data.explanation),
        picture: subsystem_optional(data.picture),
        include_in_command_interface: subsystem_optional(data.include_ci),
        use_one_command: subsystem_optional(data.use_one_command),
        support,
        content: data.content_items,
        groups: data
            .groups
            .into_iter()
            .map(|(name, items)| SubsystemGroupData { name, items })
            .collect(),
        children: data.child_names,
        tree,
        command_interface,
    };
    let summary = format!(
        "unica.subsystem.info described {} with {} content item(s)",
        result.name,
        result.content.len()
    );
    checkpoint().map_err(subsystem_info_checkpoint_message)?;
    Ok(PreparedSubsystemInfo {
        execution: subsystem_info_success(
            SubsystemInfoAnswer::Subsystem(Box::new(result)),
            xml_path,
            summary,
        ),
        format_documents,
    })
}

fn subsystem_info_success(
    data: SubsystemInfoAnswer,
    artifact: PathBuf,
    summary: String,
) -> SubsystemInfoExecution {
    SubsystemInfoExecution {
        outcome: AdapterOutcome {
            ok: true,
            summary,
            changes: Vec::new(),
            warnings: Vec::new(),
            errors: Vec::new(),
            artifacts: vec![artifact.display().to_string()],
            stdout: None,
            stderr: Some(String::new()),
            command: None,
        },
        data: Some(data),
    }
}

pub(crate) fn subsystem_info_failure(error: String) -> SubsystemInfoExecution {
    SubsystemInfoExecution {
        outcome: AdapterOutcome {
            ok: false,
            summary: "unica.subsystem.info failed in native subsystem analyzer".to_string(),
            changes: Vec::new(),
            warnings: Vec::new(),
            errors: vec![error.clone()],
            artifacts: Vec::new(),
            stdout: None,
            stderr: Some(format!("{error}\n")),
            command: None,
        },
        data: None,
    }
}

fn read_subsystem_info_file(
    path: &Path,
    mut checkpoint: impl FnMut() -> io::Result<()>,
) -> Result<Vec<u8>, String> {
    checkpoint().map_err(subsystem_info_checkpoint_message)?;
    let bytes =
        fs::read(path).map_err(|error| format!("failed to read {}: {error}", path.display()))?;
    checkpoint().map_err(subsystem_info_checkpoint_message)?;
    Ok(bytes)
}

fn topology_format_documents(
    source_root: &Path,
    topology: &SubsystemTopology,
) -> Vec<SubsystemInfoFormatDocument> {
    topology
        .dependency_documents()
        .iter()
        .map(|document| SubsystemInfoFormatDocument {
            path: source_root.join(&document.path),
            bytes: document.bytes.clone(),
        })
        .collect()
}

fn subsystem_info_checkpoint_message(error: io::Error) -> String {
    if error.kind() == io::ErrorKind::Interrupted
        && error
            .to_string()
            .starts_with(crate::domain::cancellation::CANCELLED_PREFIX)
    {
        return error.to_string();
    }
    format!("subsystem info snapshot failed: {error}")
}

fn reject_subsystem_info_parent_components(path: &Path) -> Result<(), String> {
    if path
        .components()
        .any(|component| matches!(component, Component::ParentDir))
    {
        return Err("SubsystemPath contains a parent path component (`..`)".to_string());
    }
    Ok(())
}

/// Typed answer of `unica.subsystem.info` (ADR-0023).
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SubsystemInfoResult {
    pub(crate) name: String,
    pub(crate) synonym: Option<String>,
    pub(crate) comment: Option<String>,
    pub(crate) explanation: Option<String>,
    pub(crate) picture: Option<String>,
    pub(crate) include_in_command_interface: Option<String>,
    pub(crate) use_one_command: Option<String>,
    pub(crate) support: ObjectSupportData,
    pub(crate) content: Vec<String>,
    pub(crate) groups: Vec<SubsystemGroupData>,
    pub(crate) children: Vec<String>,
    /// A focused registered tree: one ancestor chain down to this subsystem,
    /// then its complete descendant tree. Standalone XML has no proved tree.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) tree: Option<Vec<SubsystemTreeNode>>,
    /// `null` when the subsystem ships no `CommandInterface.xml`, which is a
    /// different fact from an interface that hides nothing.
    pub(crate) command_interface: Option<SubsystemCommandInterfaceData>,
}

/// The tool answers about whatever it was pointed at: one subsystem for a file,
/// the hierarchy for a `Subsystems/` folder.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase", untagged)]
pub(crate) enum SubsystemInfoAnswer {
    Subsystem(Box<SubsystemInfoResult>),
    Tree { tree: Vec<SubsystemTreeNode> },
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SubsystemGroupData {
    pub(crate) name: String,
    pub(crate) items: Vec<String>,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SubsystemCommandInterfaceData {
    pub(crate) visibility: Vec<SubsystemCommandVisibilityData>,
    pub(crate) placement: Vec<SubsystemCommandPlacementData>,
    pub(crate) order: Vec<SubsystemGroupData>,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SubsystemCommandVisibilityData {
    pub(crate) command: String,
    pub(crate) visible: bool,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SubsystemCommandPlacementData {
    pub(crate) command: String,
    pub(crate) group: Option<String>,
    pub(crate) placement: Option<String>,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SubsystemTreeNode {
    pub(crate) name: String,
    pub(crate) content: usize,
    pub(crate) children: Vec<SubsystemTreeNode>,
}

pub(crate) struct SubsystemInfoExecution {
    pub(crate) outcome: AdapterOutcome,
    pub(crate) data: Option<SubsystemInfoAnswer>,
}

pub(crate) struct PreparedSubsystemInfo {
    pub(crate) execution: SubsystemInfoExecution,
    pub(crate) format_documents: Vec<SubsystemInfoFormatDocument>,
}

pub(crate) struct SubsystemInfoFormatDocument {
    pub(crate) path: PathBuf,
    pub(crate) bytes: Vec<u8>,
}

fn subsystem_optional(value: String) -> Option<String> {
    (!value.is_empty()).then_some(value)
}

fn subsystem_tree_node(node: &SubsystemTopologyNode) -> SubsystemTreeNode {
    SubsystemTreeNode {
        name: node.name.clone(),
        content: node.content.len(),
        children: node.children.iter().map(subsystem_tree_node).collect(),
    }
}

fn focused_subsystem_tree_from_topology(
    topology: &SubsystemTopology,
    address_names: &[String],
    address: &SubsystemAddress,
    registered: &SubsystemTopologyNode,
) -> Result<SubsystemTreeNode, String> {
    let file_name = address_names
        .last()
        .expect("a subsystem address always has a leaf");
    if registered.name != *file_name {
        return Err(format!(
            "registered subsystem `{address}` has topology leaf `{}` instead of `{file_name}`",
            registered.name
        ));
    }
    focused_subsystem_tree_node(&topology.roots, address_names).ok_or_else(|| {
        format!("registered subsystem `{address}` is missing from the captured topology")
    })
}

fn subsystem_node_by_address<'a>(
    nodes: &'a [SubsystemTopologyNode],
    address: &SubsystemAddress,
) -> Option<&'a SubsystemTopologyNode> {
    for node in nodes {
        if node.address == *address {
            return Some(node);
        }
        if let Some(found) = subsystem_node_by_address(&node.children, address) {
            return Some(found);
        }
    }
    None
}

fn subsystem_descriptor_scope(
    descriptor: &Path,
    context: &WorkspaceContext,
) -> Result<Option<(PathBuf, Vec<String>, SubsystemAddress)>, String> {
    let Some(file_name) = descriptor.file_name().and_then(|name| name.to_str()) else {
        return Ok(None);
    };
    let Some(file_name) = file_name.strip_suffix(".xml") else {
        return Ok(None);
    };
    let Some(parent) = descriptor.parent() else {
        return Ok(None);
    };
    if parent.file_name().and_then(|name| name.to_str()) != Some("Subsystems") {
        return Ok(None);
    }
    let Some((source_root, mut address_names)) = subsystem_tree_scope(parent, context)? else {
        return Ok(None);
    };
    match fs::symlink_metadata(source_root.join("Configuration.xml")) {
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(format!(
                "failed to inspect Configuration.xml for subsystem scope: {error}"
            ))
        }
    }
    address_names.push(file_name.to_string());
    let address = SubsystemAddress::from_names(address_names.iter().map(String::as_str))
        .map_err(|error| error.to_string())?;
    Ok(Some((source_root, address_names, address)))
}

fn focused_subsystem_tree_node(
    nodes: &[SubsystemTopologyNode],
    address_names: &[String],
) -> Option<SubsystemTreeNode> {
    let (name, descendants) = address_names.split_first()?;
    let node = nodes.iter().find(|node| node.name == *name)?;
    if descendants.is_empty() {
        return Some(subsystem_tree_node(node));
    }
    let child = focused_subsystem_tree_node(&node.children, descendants)?;
    Some(SubsystemTreeNode {
        name: node.name.clone(),
        content: node.content.len(),
        children: vec![child],
    })
}

fn subsystem_tree_scope(
    path: &Path,
    context: &WorkspaceContext,
) -> Result<Option<(PathBuf, Vec<String>)>, String> {
    let subsystem_ancestors = path
        .ancestors()
        .filter(|ancestor| {
            ancestor.file_name().and_then(|name| name.to_str()) == Some("Subsystems")
        })
        .collect::<Vec<_>>();
    let configured_source_roots = discover_project_source_map(&context.workspace_root)?
        .source_sets
        .into_iter()
        .filter(|source_set| {
            matches!(
                source_set.kind,
                SourceSetKind::Configuration | SourceSetKind::Extension
            )
        })
        .map(|source_set| {
            let configured = PathBuf::from(source_set.path);
            let configured = if configured.is_absolute() {
                configured
            } else {
                context.workspace_root.join(configured)
            };
            normalize_path_identity(&configured)
        })
        .collect::<Result<Vec<_>, _>>()?;
    let configured_outermost = subsystem_ancestors.iter().copied().find(|ancestor| {
        ancestor.parent().is_some_and(|parent| {
            normalize_path_identity(parent)
                .is_ok_and(|parent| configured_source_roots.contains(&parent))
        })
    });
    let outermost = if configured_source_roots.is_empty() {
        subsystem_ancestors.last().copied()
    } else {
        configured_outermost
    };
    let Some(outermost) = outermost else {
        return Ok(None);
    };
    let source_root = outermost
        .parent()
        .ok_or_else(|| "Subsystems directory has no source root".to_string())?
        .to_path_buf();
    let relative = path
        .strip_prefix(outermost)
        .map_err(|_| "failed to identify the requested subsystem scope".to_string())?;
    let parts = relative
        .components()
        .map(|component| match component {
            Component::Normal(value) => value
                .to_str()
                .map(str::to_string)
                .ok_or_else(|| "subsystem scope contains a non-Unicode name".to_string()),
            _ => Err("subsystem scope contains an invalid path component".to_string()),
        })
        .collect::<Result<Vec<_>, _>>()?;
    if parts.len() % 2 != 0
        || parts
            .chunks_exact(2)
            .any(|pair| pair.get(1).map(String::as_str) != Some("Subsystems"))
    {
        return Err("subsystem scope does not follow the nested Subsystems layout".to_string());
    }
    let parent_address = parts.chunks_exact(2).map(|pair| pair[0].clone()).collect();
    Ok(Some((source_root, parent_address)))
}

/// Reads the command interface as data. `None` means the file is absent; an
/// interface that declares nothing yields empty lists instead.
pub(crate) fn subsystem_command_interface_data(
    subsystem_path: &Path,
) -> Result<Option<SubsystemCommandInterfaceData>, String> {
    let ci_path = subsystem_command_interface_path(subsystem_path);
    if !ci_path.is_file() {
        return Ok(None);
    }
    let text = fs::read_to_string(&ci_path)
        .map_err(|err| format!("failed to read {}: {err}", ci_path.display()))?;
    parse_subsystem_command_interface_data(&ci_path, &text).map(Some)
}

fn parse_subsystem_command_interface_data(
    ci_path: &Path,
    text: &str,
) -> Result<SubsystemCommandInterfaceData, String> {
    let doc = Document::parse(text.trim_start_matches('\u{feff}'))
        .map_err(|err| format!("XML parse error in {}: {err}", ci_path.display()))?;
    let root = doc.root_element();
    const CI_NS: &str = "http://v8.1c.ru/8.3/xcf/extrnprops";

    let mut visibility = Vec::new();
    if let Some(section) = root
        .children()
        .find(|node| role_info_element(*node, "CommandsVisibility", Some(CI_NS)))
    {
        for cmd in section
            .children()
            .filter(|node| role_info_element(*node, "Command", Some(CI_NS)))
        {
            let common = cmd
                .descendants()
                .find(|node| role_info_element(*node, "Common", None))
                .and_then(|node| node.text());
            visibility.push(SubsystemCommandVisibilityData {
                command: cmd.attribute("name").unwrap_or("").to_string(),
                visible: common != Some("false"),
            });
        }
    }

    let mut placement = Vec::new();
    if let Some(section) = root
        .children()
        .find(|node| role_info_element(*node, "CommandsPlacement", Some(CI_NS)))
    {
        for cmd in section
            .children()
            .filter(|node| role_info_element(*node, "Command", Some(CI_NS)))
        {
            placement.push(SubsystemCommandPlacementData {
                command: cmd.attribute("name").unwrap_or("").to_string(),
                group: subsystem_optional(child_text(cmd, "CommandGroup", Some(CI_NS))),
                placement: subsystem_optional(child_text(cmd, "Placement", Some(CI_NS))),
            });
        }
    }

    let mut order = Vec::<(String, Vec<String>)>::new();
    if let Some(section) = root
        .children()
        .find(|node| role_info_element(*node, "CommandsOrder", Some(CI_NS)))
    {
        for cmd in section
            .children()
            .filter(|node| role_info_element(*node, "Command", Some(CI_NS)))
        {
            let group = child_text(cmd, "CommandGroup", Some(CI_NS));
            push_group_item(
                &mut order,
                if group.is_empty() { "?" } else { &group },
                cmd.attribute("name").unwrap_or("").to_string(),
            );
        }
    }

    Ok(SubsystemCommandInterfaceData {
        visibility,
        placement,
        order: order
            .into_iter()
            .map(|(name, items)| SubsystemGroupData { name, items })
            .collect(),
    })
}

pub(crate) fn subsystem_info_ci_lines(
    sub_name: &str,
    subsystem_path: &Path,
) -> Result<Vec<String>, String> {
    let ci_path = subsystem_command_interface_path(subsystem_path);
    let mut lines = vec![format!("Командный интерфейс: {sub_name}"), String::new()];
    if !ci_path.is_file() {
        lines.push("Файл CommandInterface.xml не найден.".to_string());
        lines.push(format!("Путь: {}", ci_path.display()));
        return Ok(lines);
    }

    let text = fs::read_to_string(&ci_path)
        .map_err(|err| format!("failed to read {}: {err}", ci_path.display()))?;
    let doc = Document::parse(text.trim_start_matches('\u{feff}'))
        .map_err(|err| format!("XML parse error in {}: {err}", ci_path.display()))?;
    let root = doc.root_element();
    const CI_NS: &str = "http://v8.1c.ru/8.3/xcf/extrnprops";

    if let Some(section) = root
        .children()
        .find(|node| role_info_element(*node, "CommandsVisibility", Some(CI_NS)))
    {
        let mut hidden = Vec::new();
        let mut shown = Vec::new();
        for cmd in section
            .children()
            .filter(|node| role_info_element(*node, "Command", Some(CI_NS)))
        {
            let name = cmd.attribute("name").unwrap_or("").to_string();
            let common = cmd
                .descendants()
                .find(|node| role_info_element(*node, "Common", None))
                .and_then(|node| node.text());
            if common == Some("false") {
                hidden.push(name);
            } else {
                shown.push(name);
            }
        }
        let total = hidden.len() + shown.len();
        lines.push(format!("Видимость ({total}):"));
        if !hidden.is_empty() {
            lines.push(format!("  СКРЫТО ({}):", hidden.len()));
            for item in hidden {
                lines.push(format!("    {item}"));
            }
        }
        if !shown.is_empty() {
            lines.push(format!("  ПОКАЗАНО ({}):", shown.len()));
            for item in shown {
                lines.push(format!("    {item}"));
            }
        }
        lines.push(String::new());
    }

    if let Some(section) = root
        .children()
        .find(|node| role_info_element(*node, "CommandsPlacement", Some(CI_NS)))
    {
        let placements = section
            .children()
            .filter(|node| role_info_element(*node, "Command", Some(CI_NS)))
            .map(|cmd| {
                let name = cmd.attribute("name").unwrap_or("");
                let group = child_text(cmd, "CommandGroup", Some(CI_NS));
                let placement = child_text(cmd, "Placement", Some(CI_NS));
                format!(
                    "  {name} → {} ({})",
                    if group.is_empty() { "?" } else { &group },
                    if placement.is_empty() {
                        "?"
                    } else {
                        &placement
                    }
                )
            })
            .collect::<Vec<_>>();
        if !placements.is_empty() {
            lines.push(format!("Размещение ({}):", placements.len()));
            lines.extend(placements);
            lines.push(String::new());
        }
    }

    if let Some(section) = root
        .children()
        .find(|node| role_info_element(*node, "CommandsOrder", Some(CI_NS)))
    {
        let mut groups = Vec::<(String, Vec<String>)>::new();
        for cmd in section
            .children()
            .filter(|node| role_info_element(*node, "Command", Some(CI_NS)))
        {
            let name = cmd.attribute("name").unwrap_or("").to_string();
            let group = child_text(cmd, "CommandGroup", Some(CI_NS));
            push_group_item(
                &mut groups,
                if group.is_empty() { "?" } else { &group },
                name,
            );
        }
        let total = groups.iter().map(|(_, items)| items.len()).sum::<usize>();
        if total > 0 {
            lines.push(format!("Порядок команд ({total}):"));
            for (group, items) in groups {
                lines.push(format!("  [{group}]:"));
                for item in items {
                    lines.push(format!("    {item}"));
                }
            }
            lines.push(String::new());
        }
    }

    Ok(lines)
}

pub(crate) fn subsystem_info_tree(
    path: &Path,
    name_filter: &str,
) -> Result<(Vec<String>, PathBuf), String> {
    let mut lines = Vec::<String>::new();
    if path.is_dir() {
        let label = path
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or("");
        lines.push(format!("Дерево подсистем от: {label}/"));
        lines.push(String::new());
        let mut files = fs::read_dir(path)
            .map_err(|err| format!("failed to read {}: {err}", path.display()))?
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|entry| {
                entry.is_file()
                    && entry
                        .extension()
                        .and_then(|value| value.to_str())
                        .map(|ext| ext.eq_ignore_ascii_case("xml"))
                        .unwrap_or(false)
            })
            .collect::<Vec<_>>();
        files.sort_by_key(|path| {
            path.file_name()
                .and_then(|value| value.to_str())
                .unwrap_or("")
                .to_lowercase()
        });
        if !name_filter.is_empty() {
            files.retain(|path| {
                path.file_stem().and_then(|value| value.to_str()) == Some(name_filter)
            });
            if files.is_empty() {
                return Err(format!(
                    "[ERROR] Subsystem '{name_filter}' not found in {}",
                    path.display()
                ));
            }
        }
        for (index, file) in files.iter().enumerate() {
            build_subsystem_tree_entry(file, "", index == files.len() - 1, true, &mut lines)?;
        }
        Ok((lines, path.to_path_buf()))
    } else {
        if !path.is_file() {
            return Err(format!("[ERROR] File not found: {}", path.display()));
        }
        build_subsystem_tree_entry(path, "", true, true, &mut lines)?;
        Ok((lines, path.to_path_buf()))
    }
}

pub(crate) fn subsystem_dir_for_xml(xml_path: &Path) -> PathBuf {
    let dir = xml_path.parent().unwrap_or_else(|| Path::new(""));
    let base = xml_path
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("");
    dir.join(base)
}

pub(crate) fn subsystem_content_items(props: roxmltree::Node<'_, '_>) -> Vec<String> {
    props
        .children()
        .find(|node| role_info_element(*node, "Content", Some("http://v8.1c.ru/8.3/MDClasses")))
        .map(|content| {
            content
                .children()
                .filter(|node| role_info_element(*node, "Item", None))
                .filter_map(|node| node.text().map(ToOwned::to_owned))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default()
}

pub(crate) fn subsystem_child_names(sub: roxmltree::Node<'_, '_>) -> Vec<String> {
    sub.children()
        .find(|node| {
            role_info_element(*node, "ChildObjects", Some("http://v8.1c.ru/8.3/MDClasses"))
        })
        .map(|children| {
            children
                .children()
                .filter(|node| role_info_element(*node, "Subsystem", None))
                .map(|node| node.text().unwrap_or("").to_string())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default()
}

pub(crate) fn subsystem_group_content(items: &[String]) -> Vec<(String, Vec<String>)> {
    let mut groups = Vec::<(String, Vec<String>)>::new();
    for item in items {
        let (type_name, name) = if let Some((type_name, name)) = item.split_once('.') {
            (type_name.to_string(), name.to_string())
        } else if looks_like_uuid_prefix(item) {
            ("[UUID]".to_string(), item.clone())
        } else {
            ("[Other]".to_string(), item.clone())
        };
        push_group_item(&mut groups, &type_name, name);
    }
    groups
}

pub(crate) fn subsystem_known_plural_types() -> &'static [&'static str] {
    &[
        "Catalogs",
        "Documents",
        "Enums",
        "Constants",
        "Reports",
        "DataProcessors",
        "InformationRegisters",
        "AccumulationRegisters",
        "AccountingRegisters",
        "CalculationRegisters",
        "ChartsOfAccounts",
        "ChartsOfCharacteristicTypes",
        "ChartsOfCalculationTypes",
        "BusinessProcesses",
        "Tasks",
        "ExchangePlans",
        "DocumentJournals",
        "CommonModules",
        "CommonCommands",
        "CommonForms",
        "CommonPictures",
        "CommonTemplates",
        "CommonAttributes",
        "CommandGroups",
        "Roles",
        "SessionParameters",
        "FilterCriteria",
        "XDTOPackages",
        "WebServices",
        "HTTPServices",
        "WSReferences",
        "EventSubscriptions",
        "ScheduledJobs",
        "SettingsStorages",
        "FunctionalOptions",
        "FunctionalOptionsParameters",
        "DefinedTypes",
        "DocumentNumerators",
        "Sequences",
        "Subsystems",
        "StyleItems",
        "IntegrationServices",
    ]
}

struct SubsystemCompileResult {
    stdout: String,
    artifacts: Vec<PathBuf>,
    changes: Vec<String>,
    warnings: Vec<String>,
}

pub(crate) fn compile_subsystem(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> AdapterOutcome {
    compile_subsystem_internal(args, context, false)
}

pub(crate) fn preview_subsystem_compile(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> Result<AdapterOutcome, String> {
    let outcome = compile_subsystem_internal(args, context, true);
    if outcome.ok {
        Ok(outcome)
    } else {
        Err(outcome.errors.join("; "))
    }
}

fn compile_subsystem_internal(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
    dry_run: bool,
) -> AdapterOutcome {
    let write_result = (|| -> Result<SubsystemCompileResult, String> {
        let mut transaction = CompileTransaction::new();
        let definition_file = path_arg(args, &["definitionFile", "DefinitionFile"]);
        let value_arg = string_arg(args, &["value", "Value"]);
        if definition_file.is_some() && value_arg.is_some() {
            return Err("Cannot use both -DefinitionFile and -Value".to_string());
        }
        if definition_file.is_none() && value_arg.is_none() {
            return Err("Either -DefinitionFile or -Value is required".to_string());
        }

        let defn = if let Some(definition_file) = definition_file {
            let definition_file = absolutize(definition_file, &context.cwd);
            if !definition_file.exists() {
                return Err(format!(
                    "Definition file not found: {}",
                    definition_file.display()
                ));
            }
            FileBackedJson::read(&definition_file, |err| {
                format!("failed to parse subsystem JSON: {err}")
            })?
            .bind_to(&mut transaction)?
        } else {
            serde_json::from_str(value_arg.unwrap_or_default().trim_start_matches('\u{feff}'))
                .map_err(|err| format!("failed to parse subsystem JSON: {err}"))?
        };

        let obj_name = defn
            .get("name")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| "JSON must have non-empty string 'name' field".to_string())?
            .to_string();
        validate_subsystem_metadata_name("Name", &obj_name)?;

        let output_dir = required_path(args, &["outputDir", "OutputDir"], "OutputDir")
            .map(|path| absolutize(path, &context.cwd))?;
        let format_version = crate::domain::format_profile::ACTIVE_FORMAT_PROFILE
            .export_format
            .to_string();

        let synonym = json_string_field(&defn, "synonym")
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| split_camel_case(&obj_name));
        let comment = json_string_field(&defn, "comment").unwrap_or_default();
        let include_help_in_contents =
            subsystem_boolean_field(&defn, "includeHelpInContents", true)?;
        let include_in_ci = subsystem_boolean_field(&defn, "includeInCommandInterface", true)?;
        let use_one_command = subsystem_boolean_field(&defn, "useOneCommand", false)?;
        let explanation = json_string_field(&defn, "explanation").unwrap_or_default();
        let picture = json_string_field(&defn, "picture").unwrap_or_default();
        let subsystem_uuid = match defn.get("uuid") {
            None => fresh_uuid(),
            Some(Value::String(value)) if is_valid_uuid(value) => value.clone(),
            Some(value) => {
                return Err(format!(
                    "UUID must be an exact valid UUID string when provided: {value}"
                ));
            }
        };

        let mut stdout = String::new();
        let mut normalized_count = 0usize;
        let mut content_items = Vec::new();
        if let Some(content) = defn.get("content").or_else(|| defn.get("objects")) {
            if let Some(items) = content.as_array() {
                for item in items {
                    let raw = json_value_to_python_string(item);
                    let normalized = normalize_subsystem_content_ref(&raw);
                    if normalized != raw {
                        stdout.push_str(&format!("[NORM] Content: {raw} -> {normalized}\n"));
                        normalized_count += 1;
                    }
                    content_items.push(normalized);
                }
            }
        }
        if normalized_count > 0 {
            stdout.push_str(&format!(
                "[INFO] Normalized {normalized_count} content reference(s) to singular English form\n"
            ));
        }

        let children = match defn.get("children") {
            None => Vec::new(),
            Some(Value::Array(items)) => {
                let mut children = Vec::with_capacity(items.len());
                for item in items {
                    let child = item
                        .as_str()
                        .ok_or_else(|| "each child subsystem name must be a string".to_string())?
                        .to_string();
                    validate_subsystem_metadata_name("Child subsystem name", &child)?;
                    children.push(child);
                }
                children
            }
            Some(_) => return Err("children must be an array of strings".to_string()),
        };

        let mut lines = Vec::new();
        lines.push("<?xml version=\"1.0\" encoding=\"UTF-8\"?>".to_string());
        lines.push(format!(
            "<MetaDataObject xmlns=\"http://v8.1c.ru/8.3/MDClasses\" xmlns:app=\"http://v8.1c.ru/8.2/managed-application/core\" xmlns:cfg=\"http://v8.1c.ru/8.1/data/enterprise/current-config\" xmlns:cmi=\"http://v8.1c.ru/8.2/managed-application/cmi\" xmlns:ent=\"http://v8.1c.ru/8.1/data/enterprise\" xmlns:lf=\"http://v8.1c.ru/8.2/managed-application/logform\" xmlns:style=\"http://v8.1c.ru/8.1/data/ui/style\" xmlns:sys=\"http://v8.1c.ru/8.1/data/ui/fonts/system\" xmlns:v8=\"http://v8.1c.ru/8.1/data/core\" xmlns:v8ui=\"http://v8.1c.ru/8.1/data/ui\" xmlns:web=\"http://v8.1c.ru/8.1/data/ui/colors/web\" xmlns:win=\"http://v8.1c.ru/8.1/data/ui/colors/windows\" xmlns:xen=\"http://v8.1c.ru/8.3/xcf/enums\" xmlns:xpr=\"http://v8.1c.ru/8.3/xcf/predef\" xmlns:xr=\"http://v8.1c.ru/8.3/xcf/readable\" xmlns:xs=\"http://www.w3.org/2001/XMLSchema\" xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\" version=\"{format_version}\">"
        ));
        lines.push(format!(
            "\t<Subsystem uuid=\"{}\">",
            escape_xml(&subsystem_uuid)
        ));
        lines.push("\t\t<Properties>".to_string());
        lines.push(format!("\t\t\t<Name>{}</Name>", escape_xml(&obj_name)));
        emit_mltext(&mut lines, "\t\t\t", "Synonym", &synonym);
        if comment.is_empty() {
            lines.push("\t\t\t<Comment/>".to_string());
        } else {
            lines.push(format!("\t\t\t<Comment>{}</Comment>", escape_xml(&comment)));
        }
        lines.push(format!(
            "\t\t\t<IncludeHelpInContents>{include_help_in_contents}</IncludeHelpInContents>"
        ));
        lines.push(format!(
            "\t\t\t<IncludeInCommandInterface>{include_in_ci}</IncludeInCommandInterface>"
        ));
        lines.push(format!(
            "\t\t\t<UseOneCommand>{use_one_command}</UseOneCommand>"
        ));
        emit_mltext(&mut lines, "\t\t\t", "Explanation", &explanation);
        if picture.is_empty() {
            lines.push("\t\t\t<Picture/>".to_string());
        } else {
            lines.push("\t\t\t<Picture>".to_string());
            lines.push(format!("\t\t\t\t<xr:Ref>{}</xr:Ref>", escape_xml(&picture)));
            lines.push("\t\t\t\t<xr:LoadTransparent>false</xr:LoadTransparent>".to_string());
            lines.push("\t\t\t</Picture>".to_string());
        }
        if content_items.is_empty() {
            lines.push("\t\t\t<Content/>".to_string());
        } else {
            lines.push("\t\t\t<Content>".to_string());
            for item in &content_items {
                lines.push(format!(
                    "\t\t\t\t<xr:Item xsi:type=\"xr:MDObjectRef\">{}</xr:Item>",
                    escape_xml(item)
                ));
            }
            lines.push("\t\t\t</Content>".to_string());
        }
        lines.push("\t\t</Properties>".to_string());
        if children.is_empty() {
            lines.push("\t\t<ChildObjects/>".to_string());
        } else {
            lines.push("\t\t<ChildObjects>".to_string());
            for child in &children {
                lines.push(format!(
                    "\t\t\t<Subsystem>{}</Subsystem>",
                    escape_xml(child)
                ));
            }
            lines.push("\t\t</ChildObjects>".to_string());
        }
        lines.push("\t</Subsystem>".to_string());
        lines.push("</MetaDataObject>".to_string());

        let parent = path_arg(args, &["parent", "Parent"]);
        let subs_dir = if let Some(parent_path) = &parent {
            let parent_path = absolutize(parent_path.clone(), &context.cwd);
            if !parent_path.exists() {
                return Err(format!(
                    "Parent subsystem not found: {}",
                    parent_path.display()
                ));
            }
            let parent_dir = parent_path
                .parent()
                .map(Path::to_path_buf)
                .unwrap_or_else(|| output_dir.clone());
            let parent_base_name = parent_path
                .file_stem()
                .and_then(|value| value.to_str())
                .unwrap_or_default();
            parent_dir.join(parent_base_name).join("Subsystems")
        } else {
            output_dir.join("Subsystems")
        };

        let target_xml = subs_dir.join(format!("{obj_name}.xml"));
        match fs::symlink_metadata(&target_xml) {
            Ok(metadata) if metadata.is_file() && !metadata.file_type().is_symlink() => {
                let outcome = validate_subsystem(&subsystem_validation_args(&target_xml), context);
                require_subsystem_validation(&outcome)?;
                let message = format!(
                    "[SKIP] Subsystem '{obj_name}' already exists at {}; no files changed\n",
                    target_xml.display()
                );
                return Ok(SubsystemCompileResult {
                    stdout: message,
                    artifacts: Vec::new(),
                    changes: Vec::new(),
                    warnings: Vec::new(),
                });
            }
            Ok(_) => {
                return Err(format!(
                    "existing subsystem target is not a regular file: {}",
                    target_xml.display()
                ));
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(format!(
                    "failed to inspect subsystem target {}: {error}",
                    target_xml.display()
                ));
            }
        }
        let root_configuration_candidate = output_dir.join("Configuration.xml");
        let parent_xml_path = if let Some(parent_path) = parent {
            Some(absolutize(parent_path, &context.cwd))
        } else {
            Some(root_configuration_candidate.clone())
        };
        #[cfg(test)]
        run_subsystem_compile_after_root_owner_probe_hook(&root_configuration_candidate);
        transaction.create_utf8_bom_text(&target_xml, format!("{}\n", lines.join("\n")))?;
        let mut artifacts = vec![target_xml.clone()];
        let mut reused_child_subsystems = BTreeMap::<PathBuf, Vec<u8>>::new();
        stdout.push_str(&format!("[OK] Created: {}\n", target_xml.display()));

        if !children.is_empty() {
            let child_subs_dir = subs_dir.join(&obj_name).join("Subsystems");
            if !child_subs_dir.exists() {
                stdout.push_str(&format!(
                    "[OK] Created directory: {}\n",
                    child_subs_dir.display()
                ));
            }
            let mut seen = Vec::<String>::new();
            for child in &children {
                if seen.iter().any(|value| value == child) {
                    continue;
                }
                seen.push(child.clone());
                let child_xml = child_subs_dir.join(format!("{child}.xml"));
                match fs::symlink_metadata(&child_xml) {
                    Ok(metadata) if metadata.is_file() && !metadata.file_type().is_symlink() => {
                        let preimage = fs::read(&child_xml).map_err(|error| {
                            format!(
                                "failed to read existing child subsystem {}: {error}",
                                child_xml.display()
                            )
                        })?;
                        reused_child_subsystems.insert(child_xml.clone(), preimage);
                        stdout.push_str(&format!(
                            "[SKIP] Child subsystem already exists: {}\n",
                            child_xml.display()
                        ));
                    }
                    Ok(_) => {
                        return Err(format!(
                            "existing child subsystem target is not a regular file: {}",
                            child_xml.display()
                        ));
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                        transaction.create_utf8_bom_text(
                            &child_xml,
                            child_subsystem_stub_xml(child, &format_version),
                        )?;
                        stdout.push_str(&format!("[OK] Created stub: {}\n", child_xml.display()));
                        artifacts.push(child_xml);
                    }
                    Err(error) => {
                        return Err(format!(
                            "failed to inspect child subsystem target {}: {error}",
                            child_xml.display()
                        ));
                    }
                }
            }
        }

        let parent_registration_status = if let Some(parent_xml_path) = &parent_xml_path {
            let status =
                transaction.register_canonical_child(parent_xml_path, "Subsystem", &obj_name)?;
            match status {
                RegistrationStatus::Added => stdout.push_str(&format!(
                    "[OK] Registered in: {}\n",
                    parent_xml_path.display()
                )),
                RegistrationStatus::AlreadyPresent => stdout.push_str(&format!(
                    "[SKIP] Already registered in: {}\n",
                    parent_xml_path.display()
                )),
                RegistrationStatus::MissingTarget => {
                    stdout.push_str("[INFO] No parent XML to register in\n")
                }
            }
            Some(status)
        } else {
            stdout.push_str("[INFO] No parent XML to register in\n");
            None
        };
        let parent_owner_registered = matches!(
            parent_registration_status,
            Some(RegistrationStatus::Added | RegistrationStatus::AlreadyPresent)
        );

        stdout.push('\n');
        stdout.push_str("=== subsystem-compile summary ===\n");
        stdout.push_str(&format!("  Name:     {obj_name}\n"));
        stdout.push_str(&format!("  UUID:     {subsystem_uuid}\n"));
        stdout.push_str(&format!("  Content:  {} objects\n", content_items.len()));
        stdout.push_str(&format!("  Children: {}\n", children.len()));
        stdout.push_str(&format!("  File:     {}\n", target_xml.display()));

        let (artifacts, changes, warnings, output) = if dry_run {
            if parent_owner_registered {
                let parent_xml_path = parent_xml_path
                    .as_deref()
                    .expect("registered parent path must be present");
                require_subsystem_registration_owner_validation(parent_xml_path, context)?;
            }
            for child_path in reused_child_subsystems.keys() {
                let outcome = validate_subsystem(&subsystem_validation_args(child_path), context);
                require_subsystem_validation(&outcome)?;
            }
            (
                Vec::new(),
                transaction.dry_run_changes(),
                Vec::new(),
                transaction.dry_run_stdout(),
            )
        } else {
            for child_path in reused_child_subsystems.keys() {
                let outcome = validate_subsystem(&subsystem_validation_args(child_path), context);
                require_subsystem_validation(&outcome)?;
            }
            for (path, preimage) in &reused_child_subsystems {
                guard_exact_preimage_if_unprotected(&mut transaction, path, preimage)?;
            }
            let mut validation_targets = vec![target_xml.as_path()];
            if let Some(parent_xml_path) = &parent_xml_path {
                validation_targets.push(parent_xml_path);
            }
            validation_targets.extend(reused_child_subsystems.keys().map(PathBuf::as_path));
            let format_dependencies =
                subsystem_validation_format_dependency_paths(&validation_targets);
            let format_dependency_refs = format_dependencies
                .iter()
                .map(PathBuf::as_path)
                .collect::<Vec<_>>();
            guard_active_format_dependencies(&mut transaction, &format_dependency_refs, context)?;
            let validation_args = subsystem_validation_args(&target_xml);
            let report = transaction.commit_with_post_validation(|| {
                if parent_owner_registered {
                    let parent_xml_path = parent_xml_path
                        .as_deref()
                        .expect("registered parent path must be present");
                    require_subsystem_registration_owner_validation(parent_xml_path, context)?;
                }
                for child_path in reused_child_subsystems.keys() {
                    let outcome =
                        validate_subsystem(&subsystem_validation_args(child_path), context);
                    require_subsystem_validation(&outcome)?;
                }
                let outcome = validate_subsystem(&validation_args, context);
                require_subsystem_validation(&outcome)
            })?;
            let mut changes = report
                .created
                .iter()
                .map(|path| format!("created {}", path.display()))
                .collect::<Vec<_>>();
            changes.extend(
                report
                    .updated
                    .iter()
                    .map(|path| format!("updated {}", path.display())),
            );
            let mut committed_artifacts = report.created;
            committed_artifacts.extend(report.updated);
            (
                committed_artifacts,
                changes,
                report.cleanup_warnings,
                stdout,
            )
        };

        Ok(SubsystemCompileResult {
            stdout: output,
            artifacts,
            changes,
            warnings,
        })
    })();

    match write_result {
        Ok(result) => AdapterOutcome {
            ok: true,
            summary: if dry_run {
                "dry run: unica.subsystem.compile planned native subsystem compilation".to_string()
            } else {
                "unica.subsystem.compile completed with native XML writer".to_string()
            },
            changes: result.changes,
            warnings: result.warnings,
            errors: Vec::new(),
            artifacts: result
                .artifacts
                .iter()
                .map(|path| path.display().to_string())
                .collect(),
            stdout: Some(result.stdout),
            stderr: None,
            command: None,
        },
        Err(error) => AdapterOutcome {
            ok: false,
            summary: "unica.subsystem.compile failed in native XML writer".to_string(),
            changes: Vec::new(),
            warnings: Vec::new(),
            errors: vec![error.clone()],
            artifacts: Vec::new(),
            stdout: None,
            stderr: Some(format!("{error}\n")),
            command: None,
        },
    }
}

pub(crate) fn write_child_subsystem_stub(
    child_path: &Path,
    child_name: &str,
    format_version: &str,
) -> Result<(), String> {
    write_utf8_bom(
        child_path,
        &child_subsystem_stub_xml(child_name, format_version),
    )
}

pub(crate) fn child_subsystem_stub_xml(child_name: &str, format_version: &str) -> String {
    let subsystem_uuid = fresh_uuid();
    let mut lines = Vec::new();
    lines.push("<?xml version=\"1.0\" encoding=\"UTF-8\"?>".to_string());
    lines.push(format!(
        "<MetaDataObject xmlns=\"http://v8.1c.ru/8.3/MDClasses\" xmlns:app=\"http://v8.1c.ru/8.2/managed-application/core\" xmlns:cfg=\"http://v8.1c.ru/8.1/data/enterprise/current-config\" xmlns:cmi=\"http://v8.1c.ru/8.2/managed-application/cmi\" xmlns:ent=\"http://v8.1c.ru/8.1/data/enterprise\" xmlns:lf=\"http://v8.1c.ru/8.2/managed-application/logform\" xmlns:style=\"http://v8.1c.ru/8.1/data/ui/style\" xmlns:sys=\"http://v8.1c.ru/8.1/data/ui/fonts/system\" xmlns:v8=\"http://v8.1c.ru/8.1/data/core\" xmlns:v8ui=\"http://v8.1c.ru/8.1/data/ui\" xmlns:web=\"http://v8.1c.ru/8.1/data/ui/colors/web\" xmlns:win=\"http://v8.1c.ru/8.1/data/ui/colors/windows\" xmlns:xen=\"http://v8.1c.ru/8.3/xcf/enums\" xmlns:xpr=\"http://v8.1c.ru/8.3/xcf/predef\" xmlns:xr=\"http://v8.1c.ru/8.3/xcf/readable\" xmlns:xs=\"http://www.w3.org/2001/XMLSchema\" xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\" version=\"{format_version}\">"
    ));
    lines.push(format!("\t<Subsystem uuid=\"{}\">", subsystem_uuid));
    lines.push("\t\t<Properties>".to_string());
    lines.push(format!("\t\t\t<Name>{}</Name>", escape_xml(child_name)));
    lines.push("\t\t\t<Synonym/>".to_string());
    lines.push("\t\t\t<Comment/>".to_string());
    lines.push("\t\t\t<IncludeHelpInContents>true</IncludeHelpInContents>".to_string());
    lines.push("\t\t\t<IncludeInCommandInterface>true</IncludeInCommandInterface>".to_string());
    lines.push("\t\t\t<UseOneCommand>false</UseOneCommand>".to_string());
    lines.push("\t\t\t<Explanation/>".to_string());
    lines.push("\t\t\t<Picture/>".to_string());
    lines.push("\t\t\t<Content/>".to_string());
    lines.push("\t\t</Properties>".to_string());
    lines.push("\t\t<ChildObjects/>".to_string());
    lines.push("\t</Subsystem>".to_string());
    lines.push("</MetaDataObject>".to_string());
    format!("{}\n", lines.join("\n"))
}

pub(crate) fn normalize_subsystem_content_ref(raw: &str) -> String {
    let Some(dot_idx) = raw.find('.') else {
        return raw.to_string();
    };
    let type_part = &raw[..dot_idx];
    let name_part = &raw[dot_idx + 1..];
    let normalized = subsystem_content_type(type_part).unwrap_or(type_part);
    format!("{normalized}.{name_part}")
}

pub(crate) fn subsystem_content_type(type_part: &str) -> Option<&'static str> {
    match type_part {
        "Catalogs" | "Справочник" | "Каталог" | "Справочники" => {
            Some("Catalog")
        }
        "Documents" | "Документ" | "Документы" => Some("Document"),
        "Enums" | "Перечисление" | "Перечисления" => Some("Enum"),
        "Constants" | "Константа" | "Константы" => Some("Constant"),
        "Reports" | "Отчёт" | "Отчет" | "Отчёты" | "Отчеты" => Some("Report"),
        "DataProcessors" | "Обработка" | "Обработки" => Some("DataProcessor"),
        "InformationRegisters" | "РегистрСведений" | "РегистрыСведений" => {
            Some("InformationRegister")
        }
        "AccumulationRegisters" | "РегистрНакопления" | "РегистрыНакопления" => {
            Some("AccumulationRegister")
        }
        "AccountingRegisters" | "РегистрБухгалтерии" | "РегистрыБухгалтерии" => {
            Some("AccountingRegister")
        }
        "CalculationRegisters"
        | "РегистрРасчёта"
        | "РегистрРасчета"
        | "РегистрыРасчёта"
        | "РегистрыРасчета" => Some("CalculationRegister"),
        "ChartsOfAccounts" | "ПланСчетов" | "ПланыСчетов" => {
            Some("ChartOfAccounts")
        }
        "ChartsOfCharacteristicTypes" | "ПланВидовХарактеристик" | "ПланыВидовХарактеристик" => {
            Some("ChartOfCharacteristicTypes")
        }
        "ChartsOfCalculationTypes"
        | "ПланВидовРасчёта"
        | "ПланВидовРасчета"
        | "ПланыВидовРасчёта"
        | "ПланыВидовРасчета" => Some("ChartOfCalculationTypes"),
        "BusinessProcesses" | "БизнесПроцесс" | "БизнесПроцессы" => {
            Some("BusinessProcess")
        }
        "Tasks" | "Задача" | "Задачи" => Some("Task"),
        "ExchangePlans" | "ПланОбмена" | "ПланыОбмена" => Some("ExchangePlan"),
        "DocumentJournals" | "ЖурналДокументов" | "ЖурналыДокументов" => {
            Some("DocumentJournal")
        }
        "CommonModules" | "ОбщийМодуль" | "ОбщиеМодули" => {
            Some("CommonModule")
        }
        "CommonCommands" | "ОбщаяКоманда" | "ОбщиеКоманды" => {
            Some("CommonCommand")
        }
        "CommonForms" | "ОбщаяФорма" | "ОбщиеФормы" => Some("CommonForm"),
        "CommonPictures" | "ОбщаяКартинка" | "ОбщиеКартинки" => {
            Some("CommonPicture")
        }
        "CommonTemplates" | "ОбщийМакет" | "ОбщиеМакеты" => {
            Some("CommonTemplate")
        }
        "CommonAttributes" | "ОбщийРеквизит" | "ОбщиеРеквизиты" => {
            Some("CommonAttribute")
        }
        "CommandGroups" | "ГруппаКоманд" | "ГруппыКоманд" => {
            Some("CommandGroup")
        }
        "Roles" | "Роль" | "Роли" => Some("Role"),
        "SessionParameters" | "ПараметрСеанса" | "ПараметрыСеанса" => {
            Some("SessionParameter")
        }
        "FilterCriteria" | "КритерийОтбора" | "КритерииОтбора" => {
            Some("FilterCriterion")
        }
        "XDTOPackages" | "ПакетXDTO" | "ПакетыXDTO" => Some("XDTOPackage"),
        "WebServices" | "ВебСервис" | "ВебСервисы" => Some("WebService"),
        "HTTPServices" | "HTTPСервис" | "HTTPСервисы" => Some("HTTPService"),
        "WSReferences" | "WSСсылка" | "WSСсылки" => Some("WSReference"),
        "EventSubscriptions" | "ПодпискаНаСобытие" | "ПодпискиНаСобытия" => {
            Some("EventSubscription")
        }
        "ScheduledJobs" | "РегламентноеЗадание" | "РегламентныеЗадания" => {
            Some("ScheduledJob")
        }
        "SettingsStorages" | "ХранилищеНастроек" | "ХранилищаНастроек" => {
            Some("SettingsStorage")
        }
        "FunctionalOptions" | "ФункциональнаяОпция" | "ФункциональныеОпции" => {
            Some("FunctionalOption")
        }
        "FunctionalOptionsParameters" | "ПараметрФункциональныхОпций" => {
            Some("FunctionalOptionsParameter")
        }
        "DefinedTypes" | "ОпределяемыйТип" | "ОпределяемыеТипы" => {
            Some("DefinedType")
        }
        "DocumentNumerators" | "НумераторДокументов" => {
            Some("DocumentNumerator")
        }
        "Sequences" | "Последовательность" => Some("Sequence"),
        "Subsystems" | "Подсистема" | "Подсистемы" => Some("Subsystem"),
        "StyleItems" | "ЭлементСтиля" | "ЭлементыСтиля" => {
            Some("StyleItem")
        }
        "IntegrationServices" | "СервисИнтеграции" | "СервисыИнтеграции" => {
            Some("IntegrationService")
        }
        _ => None,
    }
}

pub(crate) fn invoke_read(
    operation: &str,
    _tool_name: &str,
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> Option<Result<AdapterOutcome, String>> {
    match operation {
        "subsystem-validate" => Some(Ok(validate_subsystem(args, context))),
        _ => None,
    }
}

pub(crate) fn invoke_mutation(
    operation: &str,
    _tool_name: &str,
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> Option<AdapterOutcome> {
    match operation {
        "subsystem-compile" => Some(compile_subsystem(args, context)),
        "subsystem-edit" => Some(edit_subsystem(args, context)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::UnicaApplication;
    use crate::domain::workspace::WorkspaceContext;
    use crate::infrastructure::native_operations::single_file_publisher::with_before_commit_hook;
    use serde_json::{json, Map, Value};
    use std::fs;
    use std::path::Path;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_context(name: &str) -> WorkspaceContext {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("unica-subsystem-compile-{name}-{nanos}"));
        fs::create_dir_all(&root).unwrap();
        WorkspaceContext {
            cwd: root.clone(),
            workspace_root: root.clone(),
            cache_root: root.join(".build").join("unica"),
            workspace_epoch: 1,
        }
    }

    fn compile_args(output_dir: &Path, definition: Value) -> Map<String, Value> {
        let mut args = Map::new();
        args.insert(
            "OutputDir".to_string(),
            Value::String(output_dir.display().to_string()),
        );
        args.insert("Value".to_string(), Value::String(definition.to_string()));
        args
    }

    fn edit_definition_args(
        context: &WorkspaceContext,
        subsystem_path: &Path,
        definition: Value,
    ) -> Map<String, Value> {
        let definition_path = context.cwd.join("subsystem-edit-definition.json");
        fs::write(&definition_path, definition.to_string()).unwrap();
        let mut args = Map::new();
        args.insert(
            "SubsystemPath".to_string(),
            Value::String(subsystem_path.display().to_string()),
        );
        args.insert(
            "DefinitionFile".to_string(),
            Value::String(definition_path.display().to_string()),
        );
        args
    }

    fn create_edit_fixture(context: &WorkspaceContext, name: &str) -> PathBuf {
        let outcome = compile_subsystem(
            &compile_args(
                &context.cwd,
                json!({
                    "name": name,
                    "uuid": "11111111-2222-4333-8444-555555555555"
                }),
            ),
            context,
        );
        assert!(outcome.ok, "{:?}", outcome.errors);
        context.cwd.join("Subsystems").join(format!("{name}.xml"))
    }

    fn subsystem_uuid(output_dir: &Path, name: &str) -> String {
        let xml_path = output_dir.join("Subsystems").join(format!("{name}.xml"));
        let xml = fs::read_to_string(&xml_path).unwrap();
        let marker = "<Subsystem uuid=\"";
        let start = xml.find(marker).unwrap() + marker.len();
        let end = xml[start..].find('"').unwrap() + start;
        xml[start..end].to_string()
    }

    fn child_subsystem_uuid(output_dir: &Path, parent: &str, child: &str) -> String {
        let xml_path = output_dir
            .join("Subsystems")
            .join(parent)
            .join("Subsystems")
            .join(format!("{child}.xml"));
        let xml = fs::read_to_string(&xml_path).unwrap();
        let marker = "<Subsystem uuid=\"";
        let start = xml.find(marker).unwrap() + marker.len();
        let end = xml[start..].find('"').unwrap() + start;
        xml[start..end].to_string()
    }

    fn configuration_bytes() -> Vec<u8> {
        let text = concat!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\r\n",
            "<MetaDataObject xmlns=\"http://v8.1c.ru/8.3/MDClasses\" xmlns:xr=\"http://v8.1c.ru/8.3/xcf/readable\" version=\"2.20\">\r\n",
            "\t<Configuration uuid=\"aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa\">\r\n",
            "\t\t<InternalInfo>\r\n",
            "\t\t\t<xr:ContainedObject><xr:ClassId>9cd510cd-abfc-11d4-9434-004095e12fc7</xr:ClassId><xr:ObjectId>00000000-0000-0000-0000-000000000002</xr:ObjectId></xr:ContainedObject>\r\n",
            "\t\t\t<xr:ContainedObject><xr:ClassId>9fcd25a0-4822-11d4-9414-008048da11f9</xr:ClassId><xr:ObjectId>00000000-0000-0000-0000-000000000003</xr:ObjectId></xr:ContainedObject>\r\n",
            "\t\t\t<xr:ContainedObject><xr:ClassId>e3687481-0a87-462c-a166-9f34594f9bba</xr:ClassId><xr:ObjectId>00000000-0000-0000-0000-000000000004</xr:ObjectId></xr:ContainedObject>\r\n",
            "\t\t\t<xr:ContainedObject><xr:ClassId>9de14907-ec23-4a07-96f0-85521cb6b53b</xr:ClassId><xr:ObjectId>00000000-0000-0000-0000-000000000005</xr:ObjectId></xr:ContainedObject>\r\n",
            "\t\t\t<xr:ContainedObject><xr:ClassId>51f2d5d8-ea4d-4064-8892-82951750031e</xr:ClassId><xr:ObjectId>00000000-0000-0000-0000-000000000006</xr:ObjectId></xr:ContainedObject>\r\n",
            "\t\t\t<xr:ContainedObject><xr:ClassId>e68182ea-4237-4383-967f-90c1e3370bc7</xr:ClassId><xr:ObjectId>00000000-0000-0000-0000-000000000007</xr:ObjectId></xr:ContainedObject>\r\n",
            "\t\t\t<xr:ContainedObject><xr:ClassId>fb282519-d103-4dd3-bc12-cb271d631dfc</xr:ClassId><xr:ObjectId>00000000-0000-0000-0000-000000000008</xr:ObjectId></xr:ContainedObject>\r\n",
            "\t\t</InternalInfo>\r\n",
            "\t\t<Properties><Name>Demo</Name><ConfigurationExtensionCompatibilityMode>Version8_3_27</ConfigurationExtensionCompatibilityMode><DefaultLanguage>Language.Russian</DefaultLanguage></Properties>\r\n",
            "\t\t<ChildObjects>\r\n",
            "\t\t\t<Language>Russian</Language>\r\n",
            "\t\t\t<StyleItem>Accent</StyleItem>\r\n",
            "\t\t</ChildObjects>\r\n",
            "\t</Configuration>\r\n",
            "</MetaDataObject><!-- registrar-tail -->\r\n\r\n"
        );
        let mut bytes = b"\xef\xbb\xbf".to_vec();
        bytes.extend_from_slice(text.as_bytes());
        bytes
    }

    fn write_configuration(root: &Path) -> Vec<u8> {
        let bytes = configuration_bytes();
        fs::create_dir_all(root.join("Languages")).unwrap();
        fs::write(root.join("Languages/Russian.xml"), b"language marker").unwrap();
        fs::write(root.join("Configuration.xml"), &bytes).unwrap();
        bytes
    }

    fn write_command_interface(subsystem_xml: &Path, version: &str) -> PathBuf {
        let path = subsystem_dir_for_xml(subsystem_xml)
            .join("Ext")
            .join("CommandInterface.xml");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(
            &path,
            format!(
                r#"<CommandInterface xmlns="http://v8.1c.ru/8.3/xcf/extrnprops" version="{version}"/>"#
            ),
        )
        .unwrap();
        path
    }

    #[test]
    fn subsystem_validate_reports_validation_failures_in_errors() {
        let context = temp_context("validate-errors");
        let subsystem_path = context.cwd.join("missing-subsystem.xml");
        let outcome = validate_subsystem(
            &Map::from_iter([(
                "SubsystemPath".to_string(),
                Value::String(subsystem_path.display().to_string()),
            )]),
            &context,
        );

        assert!(!outcome.ok, "{outcome:?}");
        assert!(
            outcome.errors.join("\n").contains("not found"),
            "{outcome:?}"
        );
        fs::remove_dir_all(&context.cwd).unwrap();
    }

    #[test]
    fn public_subsystem_compile_rejects_platform_invalid_configuration_owner_without_changes() {
        let context = temp_context("public-invalid-owner-enum");
        let source = context.cwd.join("src");
        fs::create_dir_all(&source).unwrap();
        fs::write(
            context.cwd.join("v8project.yaml"),
            "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
        )
        .unwrap();
        let valid = write_configuration(&source);
        let invalid = String::from_utf8(valid[3..].to_vec())
            .unwrap()
            .replace(
                "<ConfigurationExtensionCompatibilityMode>Version8_3_27</ConfigurationExtensionCompatibilityMode>",
                "<ConfigurationExtensionCompatibilityMode>Bogus</ConfigurationExtensionCompatibilityMode>",
            );
        let mut invalid_bytes = b"\xef\xbb\xbf".to_vec();
        invalid_bytes.extend_from_slice(invalid.as_bytes());
        let config_path = source.join("Configuration.xml");
        fs::write(&config_path, &invalid_bytes).unwrap();
        let mut args = compile_args(
            Path::new("src"),
            json!({
                "name": "AuditSubsystem",
                "uuid": "11111111-2222-4333-8444-555555555555"
            }),
        );
        args.insert(
            "cwd".to_string(),
            Value::String(context.cwd.display().to_string()),
        );
        args.insert("dryRun".to_string(), Value::Bool(false));

        let outcome = UnicaApplication::new()
            .call_tool("unica.subsystem.compile", &args)
            .unwrap();

        assert!(!outcome.ok, "{outcome:?}");
        let errors = outcome.errors.join("\n");
        assert!(
            errors.contains("ConfigurationExtensionCompatibilityMode"),
            "{outcome:?}"
        );
        assert!(errors.contains("Bogus"), "{outcome:?}");
        assert_eq!(fs::read(config_path).unwrap(), invalid_bytes);
        assert!(!source.join("Subsystems/AuditSubsystem.xml").exists());
        assert!(!source.join("Subsystems").exists());
        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn nested_subsystem_compile_rolls_back_if_format_owner_changes_during_publication() {
        let context = temp_context("nested-format-owner-race");
        let source = context.cwd.join("src");
        fs::create_dir_all(&source).unwrap();
        fs::write(
            context.cwd.join("v8project.yaml"),
            "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
        )
        .unwrap();
        write_configuration(&source);
        let parent_args = compile_args(
            &source,
            json!({
                "name": "Parent",
                "uuid": "11111111-2222-4333-8444-555555555555"
            }),
        );
        let parent_outcome = compile_subsystem(&parent_args, &context);
        assert!(parent_outcome.ok, "{parent_outcome:?}");
        let parent = source.join("Subsystems/Parent.xml");
        let parent_before = fs::read(&parent).unwrap();
        let owner = source.join("Configuration.xml");
        let mut concurrent_owner = fs::read(&owner).unwrap();
        concurrent_owner.extend_from_slice(b" ");
        let owner_for_hook = owner.clone();
        let concurrent_for_hook = concurrent_owner.clone();
        let mut child_args = compile_args(
            &source,
            json!({
                "name": "Child",
                "uuid": "66666666-7777-4888-8999-aaaaaaaaaaaa"
            }),
        );
        child_args.insert(
            "Parent".to_string(),
            Value::String(parent.display().to_string()),
        );

        let outcome = with_before_commit_hook(
            move |_| fs::write(&owner_for_hook, &concurrent_for_hook).unwrap(),
            || compile_subsystem(&child_args, &context),
        );

        assert!(!outcome.ok, "{outcome:?}");
        assert!(
            outcome.errors.join("\n").contains("read guard"),
            "{outcome:?}"
        );
        assert_eq!(fs::read(&owner).unwrap(), concurrent_owner);
        assert_eq!(fs::read(&parent).unwrap(), parent_before);
        assert!(!source
            .join("Subsystems/Parent/Subsystems/Child.xml")
            .exists());
        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn root_subsystem_compile_rejects_newer_configuration_that_appears_after_probe() {
        let context = temp_context("root-owner-appears-after-probe");
        let detached = temp_context("detached-root-owner-appears-after-probe");
        let source = detached.cwd.clone();
        fs::create_dir_all(&source).unwrap();
        let newer = String::from_utf8(configuration_bytes())
            .unwrap()
            .replace(r#"version="2.20""#, r#"version="2.21""#)
            .into_bytes();
        let config_path = source.join("Configuration.xml");
        let config_for_hook = config_path.clone();
        let newer_for_hook = newer.clone();
        let args = compile_args(
            &source,
            json!({
                "name": "LateOwner",
                "uuid": "11111111-2222-4333-8444-555555555555"
            }),
        );

        let outcome = with_subsystem_compile_after_root_owner_probe_hook(
            move |_| fs::write(&config_for_hook, &newer_for_hook).unwrap(),
            || compile_subsystem(&args, &context),
        );

        assert!(!outcome.ok, "{outcome:?}");
        assert!(outcome.errors.join("\n").contains("2.21"), "{outcome:?}");
        assert_eq!(fs::read(&config_path).unwrap(), newer);
        assert!(!source.join("Subsystems/LateOwner.xml").exists());
        let _ = fs::remove_dir_all(&source);
        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn root_subsystem_compile_rolls_back_if_supported_configuration_appears_during_publication() {
        let context = temp_context("root-owner-appears-during-publication");
        let detached = temp_context("detached-root-owner-appears-during-publication");
        let source = detached.cwd.clone();
        fs::create_dir_all(&source).unwrap();
        let config_path = source.join("Configuration.xml");
        let config_for_hook = config_path.clone();
        let supported = configuration_bytes();
        let args = compile_args(
            &source,
            json!({
                "name": "LateOwner",
                "uuid": "11111111-2222-4333-8444-555555555555"
            }),
        );

        let outcome = with_before_commit_hook(
            move |_| fs::write(&config_for_hook, &supported).unwrap(),
            || compile_subsystem(&args, &context),
        );

        assert!(!outcome.ok, "{outcome:?}");
        assert!(
            outcome.errors.join("\n").contains("absence guard"),
            "{outcome:?}"
        );
        assert!(config_path.is_file());
        assert!(!source.join("Subsystems/LateOwner.xml").exists());
        let _ = fs::remove_dir_all(&source);
        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn root_subsystem_compile_validates_supported_configuration_that_appears_after_probe() {
        let context = temp_context("root-invalid-owner-appears-after-probe");
        let detached = temp_context("detached-root-invalid-owner-appears-after-probe");
        let source = detached.cwd.clone();
        fs::create_dir_all(source.join("Languages")).unwrap();
        fs::write(source.join("Languages/Russian.xml"), b"language marker").unwrap();
        let invalid = String::from_utf8(configuration_bytes())
            .unwrap()
            .replace(
                "<ConfigurationExtensionCompatibilityMode>Version8_3_27</ConfigurationExtensionCompatibilityMode>",
                "<ConfigurationExtensionCompatibilityMode>Bogus</ConfigurationExtensionCompatibilityMode>",
            )
            .into_bytes();
        let config_path = source.join("Configuration.xml");
        let config_for_hook = config_path.clone();
        let invalid_for_hook = invalid.clone();
        let args = compile_args(
            &source,
            json!({
                "name": "LateOwner",
                "uuid": "11111111-2222-4333-8444-555555555555"
            }),
        );

        let outcome = with_subsystem_compile_after_root_owner_probe_hook(
            move |_| fs::write(&config_for_hook, &invalid_for_hook).unwrap(),
            || compile_subsystem(&args, &context),
        );

        assert!(!outcome.ok, "{outcome:?}");
        assert!(
            outcome
                .errors
                .join("\n")
                .contains("ConfigurationExtensionCompatibilityMode"),
            "{outcome:?}"
        );
        assert_eq!(fs::read(&config_path).unwrap(), invalid);
        assert!(!source.join("Subsystems/LateOwner.xml").exists());
        let _ = fs::remove_dir_all(&source);
        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn subsystem_edit_rolls_back_if_format_owner_changes_during_publication() {
        let context = temp_context("edit-format-owner-race");
        let source = context.cwd.join("src");
        fs::create_dir_all(&source).unwrap();
        fs::write(
            context.cwd.join("v8project.yaml"),
            "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
        )
        .unwrap();
        write_configuration(&source);
        let create_args = compile_args(
            &source,
            json!({
                "name": "Editable",
                "uuid": "11111111-2222-4333-8444-555555555555"
            }),
        );
        let create_outcome = compile_subsystem(&create_args, &context);
        assert!(create_outcome.ok, "{create_outcome:?}");
        let subsystem = source.join("Subsystems/Editable.xml");
        let subsystem_before = fs::read(&subsystem).unwrap();
        let owner = source.join("Configuration.xml");
        let mut concurrent_owner = fs::read(&owner).unwrap();
        concurrent_owner.extend_from_slice(b" ");
        let owner_for_hook = owner.clone();
        let concurrent_for_hook = concurrent_owner.clone();
        let args = edit_definition_args(
            &context,
            &subsystem,
            json!({
                "operation": "set-property",
                "value": {
                    "name": "IncludeHelpInContents",
                    "value": false
                }
            }),
        );

        let outcome = with_before_commit_hook(
            move |_| fs::write(&owner_for_hook, &concurrent_for_hook).unwrap(),
            || edit_subsystem(&args, &context),
        );

        assert!(!outcome.ok, "{outcome:?}");
        assert!(
            outcome.errors.join("\n").contains("read guard"),
            "{outcome:?}"
        );
        assert_eq!(fs::read(&owner).unwrap(), concurrent_owner);
        assert_eq!(fs::read(&subsystem).unwrap(), subsystem_before);
        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn subsystem_compile_exact_binds_a_reused_existing_child() {
        let context = temp_context("compile-reused-child-race");
        let config_before = write_configuration(&context.cwd);
        let child_path = context
            .cwd
            .join("Subsystems/Parent/Subsystems/ExistingChild.xml");
        fs::create_dir_all(child_path.parent().unwrap()).unwrap();
        let child_before = utf8_bom_bytes(&child_subsystem_stub_xml("ExistingChild", "2.20"));
        fs::write(&child_path, &child_before).unwrap();
        let mut concurrent_child = child_before.clone();
        concurrent_child.extend_from_slice(b"<!-- concurrent -->");
        let child_for_hook = child_path.clone();
        let concurrent_for_hook = concurrent_child.clone();
        let args = compile_args(
            &context.cwd,
            json!({
                "name": "Parent",
                "uuid": "11111111-2222-4333-8444-555555555555",
                "children": ["ExistingChild"]
            }),
        );

        let outcome = with_before_commit_hook(
            move |_| fs::write(&child_for_hook, &concurrent_for_hook).unwrap(),
            || compile_subsystem(&args, &context),
        );

        assert!(!outcome.ok, "{outcome:?}");
        assert!(
            outcome.errors.join("\n").contains("read guard"),
            "{outcome:?}"
        );
        assert_eq!(fs::read(&child_path).unwrap(), concurrent_child);
        assert_eq!(
            fs::read(context.cwd.join("Configuration.xml")).unwrap(),
            config_before
        );
        assert!(!context.cwd.join("Subsystems/Parent.xml").exists());
        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn subsystem_edit_exact_binds_a_reused_existing_child() {
        let context = temp_context("edit-reused-child-race");
        let subsystem = create_edit_fixture(&context, "Parent");
        let subsystem_before = fs::read(&subsystem).unwrap();
        let child_path = context
            .cwd
            .join("Subsystems/Parent/Subsystems/ExistingChild.xml");
        fs::create_dir_all(child_path.parent().unwrap()).unwrap();
        let child_before = utf8_bom_bytes(&child_subsystem_stub_xml("ExistingChild", "2.20"));
        fs::write(&child_path, &child_before).unwrap();
        let mut concurrent_child = child_before.clone();
        concurrent_child.extend_from_slice(b"<!-- concurrent -->");
        let child_for_hook = child_path.clone();
        let concurrent_for_hook = concurrent_child.clone();
        let args = edit_definition_args(
            &context,
            &subsystem,
            json!({
                "operation": "add-child",
                "value": "ExistingChild"
            }),
        );

        let outcome = with_before_commit_hook(
            move |_| fs::write(&child_for_hook, &concurrent_for_hook).unwrap(),
            || edit_subsystem(&args, &context),
        );

        assert!(!outcome.ok, "{outcome:?}");
        assert!(
            outcome.errors.join("\n").contains("read guard"),
            "{outcome:?}"
        );
        assert_eq!(fs::read(&child_path).unwrap(), concurrent_child);
        assert_eq!(fs::read(&subsystem).unwrap(), subsystem_before);
        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn subsystem_edit_rejects_newer_direct_command_interface_without_mutation() {
        let context = temp_context("edit-newer-direct-ci");
        let subsystem = create_edit_fixture(&context, "Editable");
        let subsystem_before = fs::read(&subsystem).unwrap();
        let command_interface = write_command_interface(&subsystem, "2.21");
        let command_interface_before = fs::read(&command_interface).unwrap();
        let args = edit_definition_args(
            &context,
            &subsystem,
            json!({
                "operation": "set-property",
                "value": {
                    "name": "IncludeHelpInContents",
                    "value": false
                }
            }),
        );

        let outcome = edit_subsystem(&args, &context);

        assert!(!outcome.ok, "{outcome:?}");
        assert!(
            outcome
                .errors
                .join("\n")
                .contains("newer than supported 2.20"),
            "{outcome:?}"
        );
        assert_eq!(fs::read(&subsystem).unwrap(), subsystem_before);
        assert_eq!(
            fs::read(&command_interface).unwrap(),
            command_interface_before
        );
        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn subsystem_compile_rejects_newer_reused_child_command_interface_without_mutation() {
        let context = temp_context("compile-newer-reused-child-ci");
        let config_before = write_configuration(&context.cwd);
        let child = context
            .cwd
            .join("Subsystems/Parent/Subsystems/ExistingChild.xml");
        fs::create_dir_all(child.parent().unwrap()).unwrap();
        let child_before = utf8_bom_bytes(&child_subsystem_stub_xml("ExistingChild", "2.20"));
        fs::write(&child, &child_before).unwrap();
        let command_interface = write_command_interface(&child, "2.21");
        let command_interface_before = fs::read(&command_interface).unwrap();
        let args = compile_args(
            &context.cwd,
            json!({
                "name": "Parent",
                "uuid": "11111111-2222-4333-8444-555555555555",
                "children": ["ExistingChild"]
            }),
        );

        let outcome = compile_subsystem(&args, &context);

        assert!(!outcome.ok, "{outcome:?}");
        assert!(
            outcome
                .errors
                .join("\n")
                .contains("newer than supported 2.20"),
            "{outcome:?}"
        );
        assert_eq!(fs::read(&child).unwrap(), child_before);
        assert_eq!(
            fs::read(&command_interface).unwrap(),
            command_interface_before
        );
        assert_eq!(
            fs::read(context.cwd.join("Configuration.xml")).unwrap(),
            config_before
        );
        assert!(!context.cwd.join("Subsystems/Parent.xml").exists());
        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn subsystem_compile_rejects_newer_direct_command_interface_without_mutation() {
        let context = temp_context("compile-newer-direct-ci");
        let config_before = write_configuration(&context.cwd);
        let target = context.cwd.join("Subsystems/NewSubsystem.xml");
        let command_interface = write_command_interface(&target, "2.21");
        let command_interface_before = fs::read(&command_interface).unwrap();
        let args = compile_args(
            &context.cwd,
            json!({
                "name": "NewSubsystem",
                "uuid": "11111111-2222-4333-8444-555555555555"
            }),
        );

        let outcome = compile_subsystem(&args, &context);

        assert!(!outcome.ok, "{outcome:?}");
        assert!(
            outcome
                .errors
                .join("\n")
                .contains("newer than supported 2.20"),
            "{outcome:?}"
        );
        assert_eq!(
            fs::read(&command_interface).unwrap(),
            command_interface_before
        );
        assert_eq!(
            fs::read(context.cwd.join("Configuration.xml")).unwrap(),
            config_before
        );
        assert!(!target.exists());
        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn subsystem_edit_ignores_newer_command_interface_of_unrelated_neighbor() {
        let context = temp_context("edit-unrelated-newer-ci");
        let subsystem = create_edit_fixture(&context, "Editable");
        let neighbor = context.cwd.join("Subsystems/Neighbor.xml");
        fs::write(
            &neighbor,
            utf8_bom_bytes(&child_subsystem_stub_xml("Neighbor", "2.20")),
        )
        .unwrap();
        let neighbor_command_interface = write_command_interface(&neighbor, "2.21");
        let neighbor_before = fs::read(&neighbor).unwrap();
        let neighbor_command_interface_before = fs::read(&neighbor_command_interface).unwrap();
        let args = edit_definition_args(
            &context,
            &subsystem,
            json!({
                "operation": "set-property",
                "value": {
                    "name": "IncludeHelpInContents",
                    "value": false
                }
            }),
        );

        let outcome = edit_subsystem(&args, &context);

        assert!(outcome.ok, "{outcome:?}");
        assert_eq!(fs::read(&neighbor).unwrap(), neighbor_before);
        assert_eq!(
            fs::read(&neighbor_command_interface).unwrap(),
            neighbor_command_interface_before
        );
        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn subsystem_compile_prioritizes_a_newer_reused_child_over_an_older_parent() {
        let context = temp_context("compile-mixed-dependency-versions");
        let config_path = context.cwd.join("Configuration.xml");
        let older_config = String::from_utf8(write_configuration(&context.cwd))
            .unwrap()
            .replacen(r#"version="2.20""#, r#"version="2.19""#, 1)
            .into_bytes();
        fs::write(&config_path, &older_config).unwrap();
        let child_path = context
            .cwd
            .join("Subsystems/Parent/Subsystems/NewerChild.xml");
        fs::create_dir_all(child_path.parent().unwrap()).unwrap();
        let newer_child = child_subsystem_stub_xml("NewerChild", "2.21").into_bytes();
        fs::write(&child_path, &newer_child).unwrap();
        let args = compile_args(
            &context.cwd,
            json!({
                "name": "Parent",
                "uuid": "11111111-2222-4333-8444-555555555555",
                "children": ["NewerChild"]
            }),
        );

        let outcome = compile_subsystem(&args, &context);

        assert!(!outcome.ok, "{outcome:?}");
        let errors = outcome.errors.join("\n");
        assert!(errors.contains("newer than supported 2.20"), "{outcome:?}");
        assert!(errors.contains("1C 8.5 support is planned"), "{outcome:?}");
        assert!(!errors.contains("re-export the source"), "{outcome:?}");
        assert_eq!(fs::read(&config_path).unwrap(), older_config);
        assert_eq!(fs::read(&child_path).unwrap(), newer_child);
        assert!(!context.cwd.join("Subsystems/Parent.xml").exists());
        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn compile_subsystem_preserves_explicit_uuid() {
        let context = temp_context("explicit-uuid");
        let explicit_uuid = "11111111-2222-3333-4444-555555555555";
        let args = compile_args(
            &context.cwd,
            json!({
                "name": "ExplicitUuidSubsystem",
                "uuid": explicit_uuid
            }),
        );

        let outcome = compile_subsystem(&args, &context);

        assert!(outcome.ok, "{:?}", outcome.errors);
        assert_eq!(
            subsystem_uuid(&context.cwd, "ExplicitUuidSubsystem"),
            explicit_uuid
        );
        let generated =
            fs::read_to_string(context.cwd.join("Subsystems/ExplicitUuidSubsystem.xml")).unwrap();
        assert!(generated.contains(r#"version="2.20""#), "{generated}");
        assert!(!generated.contains(r#"version="2.17""#), "{generated}");
        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn compile_subsystem_rejects_traversal_in_name_before_writing() {
        let context = temp_context("reject-name-traversal");
        let args = compile_args(
            &context.cwd,
            json!({
                "name": "../EscapedSubsystem"
            }),
        );

        let outcome = compile_subsystem(&args, &context);

        assert!(!outcome.ok, "{outcome:?}");
        assert!(outcome.errors.join("\n").contains("XML NCName"));
        assert!(!context.cwd.join("EscapedSubsystem.xml").exists());
        assert!(!context.cwd.join("Subsystems").exists());
        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn compile_subsystem_rejects_traversal_in_child_name_before_writing() {
        let context = temp_context("reject-child-traversal");
        let args = compile_args(
            &context.cwd,
            json!({
                "name": "SafeSubsystem",
                "children": ["../EscapedChild"]
            }),
        );

        let outcome = compile_subsystem(&args, &context);

        assert!(!outcome.ok, "{outcome:?}");
        assert!(outcome.errors.join("\n").contains("XML NCName"));
        assert!(!context.cwd.join("Subsystems").exists());
        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn compile_subsystem_rejects_invalid_explicit_uuid_before_writing() {
        let context = temp_context("reject-invalid-uuid");
        let args = compile_args(
            &context.cwd,
            json!({
                "name": "InvalidUuidSubsystem",
                "uuid": "not-a-uuid"
            }),
        );

        let outcome = compile_subsystem(&args, &context);

        assert!(!outcome.ok, "{outcome:?}");
        assert!(outcome.errors.join("\n").contains("UUID"));
        assert!(!context.cwd.join("Subsystems").exists());
        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn compile_subsystem_rejects_non_canonical_boolean_properties_before_writing() {
        for property in [
            "includeHelpInContents",
            "includeInCommandInterface",
            "useOneCommand",
        ] {
            let context = temp_context(&format!("reject-boolean-{property}"));
            let mut definition = json!({
                "name": "InvalidBooleanSubsystem"
            });
            definition[property] = Value::String("banana".to_string());
            let args = compile_args(&context.cwd, definition);

            let outcome = compile_subsystem(&args, &context);

            assert!(!outcome.ok, "{property}: {outcome:?}");
            assert!(
                outcome.errors.join("\n").contains("true or false"),
                "{property}: {:?}",
                outcome.errors
            );
            assert!(!context.cwd.join("Subsystems").exists(), "{property}");
            let _ = fs::remove_dir_all(&context.cwd);
        }
    }

    #[test]
    fn edit_subsystem_rejects_traversal_in_child_name_before_writing() {
        let context = temp_context("edit-reject-child-traversal");
        let subsystem_path = create_edit_fixture(&context, "EditableSubsystem");
        let parent_before = fs::read(&subsystem_path).unwrap();
        let args = edit_definition_args(
            &context,
            &subsystem_path,
            json!({
                "operation": "add-child",
                "value": "../EscapedChild"
            }),
        );

        let outcome = edit_subsystem(&args, &context);

        assert!(!outcome.ok, "{outcome:?}");
        assert!(outcome.errors.join("\n").contains("XML NCName"));
        assert_eq!(fs::read(&subsystem_path).unwrap(), parent_before);
        assert!(!context.cwd.join("Subsystems/EditableSubsystem").exists());
        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn edit_subsystem_rejects_non_canonical_boolean_properties_before_writing() {
        for property in [
            "IncludeHelpInContents",
            "IncludeInCommandInterface",
            "UseOneCommand",
        ] {
            let context = temp_context(&format!("edit-reject-boolean-{property}"));
            let subsystem_path = create_edit_fixture(&context, "EditableSubsystem");
            let parent_before = fs::read(&subsystem_path).unwrap();
            let args = edit_definition_args(
                &context,
                &subsystem_path,
                json!({
                    "operation": "set-property",
                    "value": {
                        "name": property,
                        "value": "banana"
                    }
                }),
            );

            let outcome = edit_subsystem(&args, &context);

            assert!(!outcome.ok, "{property}: {outcome:?}");
            assert!(
                outcome.errors.join("\n").contains("true or false"),
                "{property}: {:?}",
                outcome.errors
            );
            assert_eq!(fs::read(&subsystem_path).unwrap(), parent_before);
            let _ = fs::remove_dir_all(&context.cwd);
        }
    }

    #[test]
    fn edit_subsystem_late_unknown_operation_leaves_child_stub_absent() {
        let context = temp_context("edit-late-unknown-operation");
        let subsystem_path = create_edit_fixture(&context, "EditableSubsystem");
        let parent_before = fs::read(&subsystem_path).unwrap();
        let child_path = context
            .cwd
            .join("Subsystems/EditableSubsystem/Subsystems/PlannedChild.xml");
        let args = edit_definition_args(
            &context,
            &subsystem_path,
            json!([
                {
                    "operation": "add-child",
                    "value": "PlannedChild"
                },
                {
                    "operation": "bogus",
                    "value": ""
                }
            ]),
        );

        let outcome = edit_subsystem(&args, &context);

        assert!(!outcome.ok, "{outcome:?}");
        assert!(outcome.errors.join("\n").contains("Unknown operation"));
        assert_eq!(fs::read(&subsystem_path).unwrap(), parent_before);
        assert!(!child_path.exists());
        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn edit_subsystem_plans_child_stubs_from_final_batch_state() {
        let context = temp_context("edit-final-batch-state");
        let subsystem_path = create_edit_fixture(&context, "EditableSubsystem");
        let child_path = context
            .cwd
            .join("Subsystems/EditableSubsystem/Subsystems/TransientChild.xml");
        let args = edit_definition_args(
            &context,
            &subsystem_path,
            json!([
                {
                    "operation": "add-child",
                    "value": "TransientChild"
                },
                {
                    "operation": "remove-child",
                    "value": "TransientChild"
                }
            ]),
        );

        let outcome = edit_subsystem(&args, &context);

        assert!(outcome.ok, "{:?}", outcome.errors);
        assert!(!child_path.exists());
        let parent = fs::read_to_string(&subsystem_path).unwrap();
        assert!(!parent.contains("<Subsystem>TransientChild</Subsystem>"));
        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn edit_subsystem_does_not_register_or_clobber_invalid_existing_child_target() {
        let context = temp_context("edit-invalid-existing-child");
        let subsystem_path = create_edit_fixture(&context, "EditableSubsystem");
        let parent_before = fs::read(&subsystem_path).unwrap();
        let child_path = context
            .cwd
            .join("Subsystems/EditableSubsystem/Subsystems/ExistingChild.xml");
        fs::create_dir_all(child_path.parent().unwrap()).unwrap();
        let child_before = b"sentinel-partial-child".to_vec();
        fs::write(&child_path, &child_before).unwrap();
        let args = edit_definition_args(
            &context,
            &subsystem_path,
            json!({
                "operation": "add-child",
                "value": "ExistingChild"
            }),
        );

        let outcome = edit_subsystem(&args, &context);

        assert!(!outcome.ok, "{outcome:?}");
        assert!(
            outcome.errors.join("\n").contains("semantic validation"),
            "{:?}",
            outcome.errors
        );
        assert!(
            outcome.errors.join("\n").contains("XML parse error"),
            "{:?}",
            outcome.errors
        );
        assert_eq!(fs::read(&subsystem_path).unwrap(), parent_before);
        assert_eq!(fs::read(&child_path).unwrap(), child_before);
        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn edit_subsystem_rolls_back_parent_and_child_stub_after_post_write_failure() {
        use crate::infrastructure::native_operations::compile_transaction::{
            with_commit_failpoint, CommitFailpoint,
        };

        let context = temp_context("edit-post-write-rollback");
        let subsystem_path = create_edit_fixture(&context, "EditableSubsystem");
        let parent_before = fs::read(&subsystem_path).unwrap();
        let child_path = context
            .cwd
            .join("Subsystems/EditableSubsystem/Subsystems/PlannedChild.xml");
        let args = edit_definition_args(
            &context,
            &subsystem_path,
            json!({
                "operation": "add-child",
                "value": "PlannedChild"
            }),
        );

        let outcome = with_commit_failpoint(CommitFailpoint::PostWriteValidation, || {
            edit_subsystem(&args, &context)
        });

        assert!(!outcome.ok, "{outcome:?}");
        assert!(outcome.errors.join("\n").contains("post-write validation"));
        assert_eq!(fs::read(&subsystem_path).unwrap(), parent_before);
        assert!(!child_path.exists());
        assert!(!context.cwd.join("Subsystems/EditableSubsystem").exists());
        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn compile_subsystem_generates_unique_uuid_when_missing() {
        let context = temp_context("generated-uuid");
        for name in ["GeneratedUuidA", "GeneratedUuidB"] {
            let args = compile_args(
                &context.cwd,
                json!({
                    "name": name
                }),
            );

            let outcome = compile_subsystem(&args, &context);
            assert!(outcome.ok, "{:?}", outcome.errors);
        }

        let first_uuid = subsystem_uuid(&context.cwd, "GeneratedUuidA");
        let second_uuid = subsystem_uuid(&context.cwd, "GeneratedUuidB");
        assert_ne!(first_uuid, second_uuid);
        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn compile_subsystem_generates_unique_child_stub_uuid_when_missing() {
        let context = temp_context("generated-child-uuid");
        for (parent, child) in [
            ("GeneratedParentA", "GeneratedChildA"),
            ("GeneratedParentB", "GeneratedChildB"),
        ] {
            let args = compile_args(
                &context.cwd,
                json!({
                    "name": parent,
                    "children": [child]
                }),
            );

            let outcome = compile_subsystem(&args, &context);
            assert!(outcome.ok, "{:?}", outcome.errors);
        }

        let first_uuid = child_subsystem_uuid(&context.cwd, "GeneratedParentA", "GeneratedChildA");
        let second_uuid = child_subsystem_uuid(&context.cwd, "GeneratedParentB", "GeneratedChildB");
        assert_ne!(first_uuid, second_uuid);
        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn compile_subsystem_registers_in_canonical_position_and_preserves_crlf() {
        let context = temp_context("canonical-registration");
        let config_path = context.cwd.join("Configuration.xml");
        let config_before = write_configuration(&context.cwd);
        let args = compile_args(
            &context.cwd,
            json!({
                "name": "SampleArea",
                "uuid": "11111111-2222-4333-8444-555555555555"
            }),
        );

        let preview = preview_subsystem_compile(&args, &context).unwrap();
        assert!(preview.ok, "{:?}", preview.errors);
        assert!(preview.summary.contains("dry run"));
        assert!(preview
            .changes
            .iter()
            .any(|change| change.contains("would create") && change.contains("SampleArea.xml")));
        assert!(preview
            .changes
            .iter()
            .any(|change| change.contains("would update") && change.contains("Configuration.xml")));
        assert!(preview.stdout.unwrap_or_default().contains("@@ bytes"));
        assert!(preview.artifacts.is_empty());
        assert_eq!(fs::read(&config_path).unwrap(), config_before);
        assert!(!context.cwd.join("Subsystems/SampleArea.xml").exists());

        let outcome = compile_subsystem(&args, &context);

        assert!(outcome.ok, "{:?}", outcome.errors);
        let bytes = fs::read(&config_path).unwrap();
        let text = String::from_utf8(bytes).unwrap();
        assert!(text.contains(concat!(
            "\t\t\t<Language>Russian</Language>\r\n",
            "\t\t\t<Subsystem>SampleArea</Subsystem>\r\n",
            "\t\t\t<StyleItem>Accent</StyleItem>\r\n"
        )));
        assert!(text.ends_with("<!-- registrar-tail -->\r\n\r\n"));
        assert!(!text.replace("\r\n", "").contains('\n'));
        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn nested_subsystem_compile_sorts_names_case_insensitively_in_parent() {
        let context = temp_context("nested-canonical-order");
        let parent_path = context.cwd.join("Subsystems/Parent.xml");
        fs::create_dir_all(parent_path.parent().unwrap()).unwrap();
        let parent = child_subsystem_stub_xml("Parent", "2.20")
            .replace(
                "\t\t<ChildObjects/>",
                "\t\t<ChildObjects>\n\t\t\t<Subsystem>alpha</Subsystem>\n\t\t\t<Subsystem>Zulu</Subsystem>\n\t\t</ChildObjects>",
            )
            .replace(
                "</MetaDataObject>\n",
                "</MetaDataObject><!-- parent-tail -->\n",
            )
            .replace('\n', "\r\n");
        fs::write(&parent_path, utf8_bom_bytes(&parent)).unwrap();
        let mut args = compile_args(
            &context.cwd,
            json!({
                "name": "Beta",
                "uuid": "11111111-2222-4333-8444-555555555555"
            }),
        );
        args.insert(
            "Parent".to_string(),
            Value::String(parent_path.display().to_string()),
        );

        let outcome = compile_subsystem(&args, &context);

        assert!(outcome.ok, "{:?}", outcome.errors);
        let parent = String::from_utf8(fs::read(&parent_path).unwrap()).unwrap();
        assert!(parent.contains(concat!(
            "\t\t\t<Subsystem>alpha</Subsystem>\r\n",
            "\t\t\t<Subsystem>Beta</Subsystem>\r\n",
            "\t\t\t<Subsystem>Zulu</Subsystem>\r\n"
        )));
        assert!(parent.ends_with("<!-- parent-tail -->\r\n"));
        assert!(!parent.replace("\r\n", "").contains('\n'));
        assert!(context
            .cwd
            .join("Subsystems/Parent/Subsystems/Beta.xml")
            .is_file());

        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn repeated_subsystem_compile_does_not_overwrite_or_report_changes() {
        let context = temp_context("repeat-noop");
        let args = compile_args(
            &context.cwd,
            json!({
                "name": "StableArea",
                "uuid": "11111111-2222-4333-8444-555555555555",
                "children": ["StableChild"]
            }),
        );
        let first = compile_subsystem(&args, &context);
        assert!(first.ok, "{:?}", first.errors);
        let object_path = context.cwd.join("Subsystems/StableArea.xml");
        let child_path = context
            .cwd
            .join("Subsystems/StableArea/Subsystems/StableChild.xml");
        let object_before = fs::read(&object_path).unwrap();
        let child_before = fs::read(&child_path).unwrap();

        let repeated = compile_subsystem(&args, &context);

        assert!(repeated.ok, "{:?}", repeated.errors);
        assert!(repeated.changes.is_empty(), "{:?}", repeated.changes);
        assert!(repeated.artifacts.is_empty(), "{:?}", repeated.artifacts);
        assert_eq!(fs::read(&object_path).unwrap(), object_before);
        assert_eq!(fs::read(&child_path).unwrap(), child_before);
        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn subsystem_compile_rolls_back_after_object_files_failure() {
        use crate::infrastructure::native_operations::compile_transaction::{
            with_commit_failpoint, CommitFailpoint,
        };

        let context = temp_context("rollback-after-files");
        let config_path = context.cwd.join("Configuration.xml");
        let config_before = write_configuration(&context.cwd);
        let args = compile_args(
            &context.cwd,
            json!({
                "name": "RollbackArea",
                "uuid": "11111111-2222-4333-8444-555555555555",
                "children": ["RollbackChild"]
            }),
        );

        let outcome = with_commit_failpoint(CommitFailpoint::AfterObjectFiles, || {
            compile_subsystem(&args, &context)
        });

        assert!(!outcome.ok, "{outcome:?}");
        assert!(outcome.errors.join("\n").contains("after object files"));
        assert_eq!(fs::read(&config_path).unwrap(), config_before);
        assert!(!context.cwd.join("Subsystems").exists());
        let _ = fs::remove_dir_all(&context.cwd);
    }
}
