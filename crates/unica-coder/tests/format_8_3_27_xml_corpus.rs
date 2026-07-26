use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use roxmltree::Document;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use unica_coder::application::{ToolHandler, UnicaApplication};

#[path = "platform/format_8_3_27_xml_corpus.rs"]
mod platform_support;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum XmlImpactClass {
    None,
    CreateOrModify,
    RemoveOrModify,
}

#[derive(Debug, Clone, Copy)]
struct MutatorRegistryEntry {
    tool: &'static str,
    operation: &'static str,
    impact: XmlImpactClass,
    case_ids: &'static [&'static str],
    required_branches: &'static [&'static str],
}

#[derive(Debug, Clone, Copy)]
struct ExecutableCase {
    id: &'static str,
    tool: &'static str,
    branch: &'static str,
}

static MUTATOR_REGISTRY: &[MutatorRegistryEntry] = &[
    MutatorRegistryEntry {
        tool: "unica.cf.edit",
        operation: "cf-edit",
        impact: XmlImpactClass::CreateOrModify,
        case_ids: &[
            "cf-edit-root-property",
            "cf-edit-set-panels",
            "cf-edit-set-home-page",
        ],
        required_branches: &["root-property", "set-panels", "set-home-page"],
    },
    MutatorRegistryEntry {
        tool: "unica.cf.init",
        operation: "cf-init",
        impact: XmlImpactClass::CreateOrModify,
        case_ids: &["cf-init-default"],
        required_branches: &["default"],
    },
    MutatorRegistryEntry {
        tool: "unica.cfe.borrow",
        operation: "cfe-borrow",
        impact: XmlImpactClass::CreateOrModify,
        case_ids: &["cfe-borrow-object", "cfe-borrow-managed-form"],
        required_branches: &["metadata-object", "managed-form"],
    },
    MutatorRegistryEntry {
        tool: "unica.cfe.init",
        operation: "cfe-init",
        impact: XmlImpactClass::CreateOrModify,
        case_ids: &["cfe-init-default"],
        required_branches: &["default"],
    },
    MutatorRegistryEntry {
        tool: "unica.cfe.patch_method",
        operation: "cfe-patch-method",
        impact: XmlImpactClass::CreateOrModify,
        case_ids: &[
            "cfe-patch-method-bsl-only",
            "cfe-patch-method-catalog-object-module",
            "cfe-patch-method-catalog-manager-module",
            "cfe-patch-method-information-register-record-set-module",
            "cfe-patch-method-catalog-form-module",
            "cfe-patch-method-constant-value-manager-module",
        ],
        required_branches: &[
            "CommonModule",
            "Catalog.ObjectModule",
            "Catalog.ManagerModule",
            "InformationRegister.RecordSetModule",
            "Catalog.Form",
            "Constant.ValueManagerModule",
        ],
    },
    MutatorRegistryEntry {
        tool: "unica.code.patch",
        operation: "code-patch",
        impact: XmlImpactClass::None,
        case_ids: &["code-patch-bsl-only"],
        required_branches: &["bsl-only"],
    },
    MutatorRegistryEntry {
        tool: "unica.dcs.compile",
        operation: "dcs-compile",
        impact: XmlImpactClass::CreateOrModify,
        case_ids: &["dcs-compile-owned-template"],
        required_branches: &["owned-template"],
    },
    MutatorRegistryEntry {
        tool: "unica.dcs.edit",
        operation: "dcs-edit",
        impact: XmlImpactClass::CreateOrModify,
        case_ids: &[
            "dcs-edit-owned-template",
            "dcs-edit-add-parameter-after-settings",
            "dcs-edit-set-structure-after-settings",
            "dcs-edit-modify-field-role-restriction",
        ],
        required_branches: &[
            "owned-template",
            "add-parameter-after-settings",
            "set-structure-after-settings",
            "modify-field-role-restriction",
        ],
    },
    MutatorRegistryEntry {
        tool: "unica.epf.init",
        operation: "epf-init",
        impact: XmlImpactClass::CreateOrModify,
        case_ids: &["epf-init-managed-form"],
        required_branches: &["managed-form"],
    },
    MutatorRegistryEntry {
        tool: "unica.erf.init",
        operation: "erf-init",
        impact: XmlImpactClass::CreateOrModify,
        case_ids: &["erf-init-managed-form"],
        required_branches: &["managed-form"],
    },
    MutatorRegistryEntry {
        tool: "unica.form.add",
        operation: "form-add",
        impact: XmlImpactClass::CreateOrModify,
        case_ids: &["form-add-managed"],
        required_branches: &["managed-form"],
    },
    MutatorRegistryEntry {
        tool: "unica.form.compile",
        operation: "form-compile",
        impact: XmlImpactClass::CreateOrModify,
        case_ids: &["form-compile-managed"],
        required_branches: &["managed-form"],
    },
    MutatorRegistryEntry {
        tool: "unica.form.edit",
        operation: "form-edit",
        impact: XmlImpactClass::CreateOrModify,
        case_ids: &["form-edit-managed"],
        required_branches: &["managed-form"],
    },
    MutatorRegistryEntry {
        tool: "unica.form.remove",
        operation: "form-remove",
        impact: XmlImpactClass::RemoveOrModify,
        case_ids: &["form-remove-managed"],
        required_branches: &["remove-managed-form"],
    },
    MutatorRegistryEntry {
        tool: "unica.help.add",
        operation: "help-add",
        impact: XmlImpactClass::CreateOrModify,
        case_ids: &["help-add-object"],
        required_branches: &["object-help"],
    },
    MutatorRegistryEntry {
        tool: "unica.interface.edit",
        operation: "interface-edit",
        impact: XmlImpactClass::CreateOrModify,
        case_ids: &["interface-edit-subsystem"],
        required_branches: &["subsystem-command-interface"],
    },
    MutatorRegistryEntry {
        tool: "unica.meta.compile",
        operation: "meta-compile",
        impact: XmlImpactClass::CreateOrModify,
        case_ids: &[
            "meta-compile-catalog",
            "meta-compile-document",
            "meta-compile-enum",
            "meta-compile-constant",
            "meta-compile-information-register",
            "meta-compile-accumulation-register",
            "meta-compile-accounting-register",
            "meta-compile-calculation-register",
            "meta-compile-chart-of-accounts",
            "meta-compile-chart-of-characteristic-types",
            "meta-compile-chart-of-calculation-types",
            "meta-compile-business-process",
            "meta-compile-task",
            "meta-compile-exchange-plan",
            "meta-compile-document-journal",
            "meta-compile-report",
            "meta-compile-data-processor",
            "meta-compile-common-module",
            "meta-compile-scheduled-job",
            "meta-compile-event-subscription",
            "meta-compile-http-service",
            "meta-compile-web-service",
            "meta-compile-defined-type",
        ],
        required_branches: &[
            "Catalog",
            "Document",
            "Enum",
            "Constant",
            "InformationRegister",
            "AccumulationRegister",
            "AccountingRegister",
            "CalculationRegister",
            "ChartOfAccounts",
            "ChartOfCharacteristicTypes",
            "ChartOfCalculationTypes",
            "BusinessProcess",
            "Task",
            "ExchangePlan",
            "DocumentJournal",
            "Report",
            "DataProcessor",
            "CommonModule",
            "ScheduledJob",
            "EventSubscription",
            "HTTPService",
            "WebService",
            "DefinedType",
        ],
    },
    MutatorRegistryEntry {
        tool: "unica.meta.edit",
        operation: "meta-edit",
        impact: XmlImpactClass::CreateOrModify,
        case_ids: &["meta-edit-property"],
        required_branches: &["modify-property"],
    },
    MutatorRegistryEntry {
        tool: "unica.meta.remove",
        operation: "meta-remove",
        impact: XmlImpactClass::RemoveOrModify,
        case_ids: &["meta-remove-object"],
        required_branches: &["remove-object"],
    },
    MutatorRegistryEntry {
        tool: "unica.mxl.compile",
        operation: "mxl-compile",
        impact: XmlImpactClass::CreateOrModify,
        case_ids: &["mxl-compile-owned-template"],
        required_branches: &["owned-template"],
    },
    MutatorRegistryEntry {
        tool: "unica.role.compile",
        operation: "role-compile",
        impact: XmlImpactClass::CreateOrModify,
        case_ids: &["role-compile-name-field"],
        required_branches: &["name-field"],
    },
    MutatorRegistryEntry {
        tool: "unica.subsystem.compile",
        operation: "subsystem-compile",
        impact: XmlImpactClass::CreateOrModify,
        case_ids: &["subsystem-compile-child"],
        required_branches: &["child-creation"],
    },
    MutatorRegistryEntry {
        tool: "unica.subsystem.edit",
        operation: "subsystem-edit",
        impact: XmlImpactClass::CreateOrModify,
        case_ids: &["subsystem-edit-add-child"],
        required_branches: &["add-child"],
    },
    MutatorRegistryEntry {
        tool: "unica.support.edit",
        operation: "support-edit",
        impact: XmlImpactClass::None,
        case_ids: &["support-edit-bin-only"],
        required_branches: &["parent-configurations-bin-only"],
    },
    MutatorRegistryEntry {
        tool: "unica.template.add",
        operation: "template-add",
        impact: XmlImpactClass::CreateOrModify,
        case_ids: &[
            "template-add-spreadsheet-document",
            "template-add-data-composition-schema",
            "template-add-text-document",
            "template-add-html-document",
            "template-add-binary-data",
        ],
        required_branches: &[
            "SpreadsheetDocument",
            "DataCompositionSchema",
            "TextDocument",
            "HTMLDocument",
            "BinaryData",
        ],
    },
    MutatorRegistryEntry {
        tool: "unica.template.remove",
        operation: "template-remove",
        impact: XmlImpactClass::RemoveOrModify,
        case_ids: &["template-remove-object-template"],
        required_branches: &["remove-object-template"],
    },
];

static EXECUTABLE_CASES: &[ExecutableCase] = &[
    ExecutableCase {
        id: "cf-edit-root-property",
        tool: "unica.cf.edit",
        branch: "root-property",
    },
    ExecutableCase {
        id: "cf-edit-set-panels",
        tool: "unica.cf.edit",
        branch: "set-panels",
    },
    ExecutableCase {
        id: "cf-edit-set-home-page",
        tool: "unica.cf.edit",
        branch: "set-home-page",
    },
    ExecutableCase {
        id: "cf-init-default",
        tool: "unica.cf.init",
        branch: "default",
    },
    ExecutableCase {
        id: "cfe-borrow-object",
        tool: "unica.cfe.borrow",
        branch: "metadata-object",
    },
    ExecutableCase {
        id: "cfe-borrow-managed-form",
        tool: "unica.cfe.borrow",
        branch: "managed-form",
    },
    ExecutableCase {
        id: "cfe-init-default",
        tool: "unica.cfe.init",
        branch: "default",
    },
    ExecutableCase {
        id: "cfe-patch-method-bsl-only",
        tool: "unica.cfe.patch_method",
        branch: "CommonModule",
    },
    ExecutableCase {
        id: "cfe-patch-method-catalog-object-module",
        tool: "unica.cfe.patch_method",
        branch: "Catalog.ObjectModule",
    },
    ExecutableCase {
        id: "cfe-patch-method-catalog-manager-module",
        tool: "unica.cfe.patch_method",
        branch: "Catalog.ManagerModule",
    },
    ExecutableCase {
        id: "cfe-patch-method-information-register-record-set-module",
        tool: "unica.cfe.patch_method",
        branch: "InformationRegister.RecordSetModule",
    },
    ExecutableCase {
        id: "cfe-patch-method-catalog-form-module",
        tool: "unica.cfe.patch_method",
        branch: "Catalog.Form",
    },
    ExecutableCase {
        id: "cfe-patch-method-constant-value-manager-module",
        tool: "unica.cfe.patch_method",
        branch: "Constant.ValueManagerModule",
    },
    ExecutableCase {
        id: "code-patch-bsl-only",
        tool: "unica.code.patch",
        branch: "bsl-only",
    },
    ExecutableCase {
        id: "dcs-compile-owned-template",
        tool: "unica.dcs.compile",
        branch: "owned-template",
    },
    ExecutableCase {
        id: "dcs-edit-owned-template",
        tool: "unica.dcs.edit",
        branch: "owned-template",
    },
    ExecutableCase {
        id: "dcs-edit-add-parameter-after-settings",
        tool: "unica.dcs.edit",
        branch: "add-parameter-after-settings",
    },
    ExecutableCase {
        id: "dcs-edit-set-structure-after-settings",
        tool: "unica.dcs.edit",
        branch: "set-structure-after-settings",
    },
    ExecutableCase {
        id: "dcs-edit-modify-field-role-restriction",
        tool: "unica.dcs.edit",
        branch: "modify-field-role-restriction",
    },
    ExecutableCase {
        id: "epf-init-managed-form",
        tool: "unica.epf.init",
        branch: "managed-form",
    },
    ExecutableCase {
        id: "erf-init-managed-form",
        tool: "unica.erf.init",
        branch: "managed-form",
    },
    ExecutableCase {
        id: "form-add-managed",
        tool: "unica.form.add",
        branch: "managed-form",
    },
    ExecutableCase {
        id: "form-compile-managed",
        tool: "unica.form.compile",
        branch: "managed-form",
    },
    ExecutableCase {
        id: "form-edit-managed",
        tool: "unica.form.edit",
        branch: "managed-form",
    },
    ExecutableCase {
        id: "form-remove-managed",
        tool: "unica.form.remove",
        branch: "remove-managed-form",
    },
    ExecutableCase {
        id: "help-add-object",
        tool: "unica.help.add",
        branch: "object-help",
    },
    ExecutableCase {
        id: "interface-edit-subsystem",
        tool: "unica.interface.edit",
        branch: "subsystem-command-interface",
    },
    ExecutableCase {
        id: "meta-compile-catalog",
        tool: "unica.meta.compile",
        branch: "Catalog",
    },
    ExecutableCase {
        id: "meta-compile-document",
        tool: "unica.meta.compile",
        branch: "Document",
    },
    ExecutableCase {
        id: "meta-compile-enum",
        tool: "unica.meta.compile",
        branch: "Enum",
    },
    ExecutableCase {
        id: "meta-compile-constant",
        tool: "unica.meta.compile",
        branch: "Constant",
    },
    ExecutableCase {
        id: "meta-compile-information-register",
        tool: "unica.meta.compile",
        branch: "InformationRegister",
    },
    ExecutableCase {
        id: "meta-compile-accumulation-register",
        tool: "unica.meta.compile",
        branch: "AccumulationRegister",
    },
    ExecutableCase {
        id: "meta-compile-accounting-register",
        tool: "unica.meta.compile",
        branch: "AccountingRegister",
    },
    ExecutableCase {
        id: "meta-compile-calculation-register",
        tool: "unica.meta.compile",
        branch: "CalculationRegister",
    },
    ExecutableCase {
        id: "meta-compile-chart-of-accounts",
        tool: "unica.meta.compile",
        branch: "ChartOfAccounts",
    },
    ExecutableCase {
        id: "meta-compile-chart-of-characteristic-types",
        tool: "unica.meta.compile",
        branch: "ChartOfCharacteristicTypes",
    },
    ExecutableCase {
        id: "meta-compile-chart-of-calculation-types",
        tool: "unica.meta.compile",
        branch: "ChartOfCalculationTypes",
    },
    ExecutableCase {
        id: "meta-compile-business-process",
        tool: "unica.meta.compile",
        branch: "BusinessProcess",
    },
    ExecutableCase {
        id: "meta-compile-task",
        tool: "unica.meta.compile",
        branch: "Task",
    },
    ExecutableCase {
        id: "meta-compile-exchange-plan",
        tool: "unica.meta.compile",
        branch: "ExchangePlan",
    },
    ExecutableCase {
        id: "meta-compile-document-journal",
        tool: "unica.meta.compile",
        branch: "DocumentJournal",
    },
    ExecutableCase {
        id: "meta-compile-report",
        tool: "unica.meta.compile",
        branch: "Report",
    },
    ExecutableCase {
        id: "meta-compile-data-processor",
        tool: "unica.meta.compile",
        branch: "DataProcessor",
    },
    ExecutableCase {
        id: "meta-compile-common-module",
        tool: "unica.meta.compile",
        branch: "CommonModule",
    },
    ExecutableCase {
        id: "meta-compile-scheduled-job",
        tool: "unica.meta.compile",
        branch: "ScheduledJob",
    },
    ExecutableCase {
        id: "meta-compile-event-subscription",
        tool: "unica.meta.compile",
        branch: "EventSubscription",
    },
    ExecutableCase {
        id: "meta-compile-http-service",
        tool: "unica.meta.compile",
        branch: "HTTPService",
    },
    ExecutableCase {
        id: "meta-compile-web-service",
        tool: "unica.meta.compile",
        branch: "WebService",
    },
    ExecutableCase {
        id: "meta-compile-defined-type",
        tool: "unica.meta.compile",
        branch: "DefinedType",
    },
    ExecutableCase {
        id: "meta-edit-property",
        tool: "unica.meta.edit",
        branch: "modify-property",
    },
    ExecutableCase {
        id: "meta-remove-object",
        tool: "unica.meta.remove",
        branch: "remove-object",
    },
    ExecutableCase {
        id: "mxl-compile-owned-template",
        tool: "unica.mxl.compile",
        branch: "owned-template",
    },
    ExecutableCase {
        id: "role-compile-name-field",
        tool: "unica.role.compile",
        branch: "name-field",
    },
    ExecutableCase {
        id: "subsystem-compile-child",
        tool: "unica.subsystem.compile",
        branch: "child-creation",
    },
    ExecutableCase {
        id: "subsystem-edit-add-child",
        tool: "unica.subsystem.edit",
        branch: "add-child",
    },
    ExecutableCase {
        id: "support-edit-bin-only",
        tool: "unica.support.edit",
        branch: "parent-configurations-bin-only",
    },
    ExecutableCase {
        id: "template-add-spreadsheet-document",
        tool: "unica.template.add",
        branch: "SpreadsheetDocument",
    },
    ExecutableCase {
        id: "template-add-data-composition-schema",
        tool: "unica.template.add",
        branch: "DataCompositionSchema",
    },
    ExecutableCase {
        id: "template-add-text-document",
        tool: "unica.template.add",
        branch: "TextDocument",
    },
    ExecutableCase {
        id: "template-add-html-document",
        tool: "unica.template.add",
        branch: "HTMLDocument",
    },
    ExecutableCase {
        id: "template-add-binary-data",
        tool: "unica.template.add",
        branch: "BinaryData",
    },
    ExecutableCase {
        id: "template-remove-object-template",
        tool: "unica.template.remove",
        branch: "remove-object-template",
    },
];

type XmlSnapshot = BTreeMap<String, String>;
type XmlPayloadSnapshot = BTreeMap<String, Vec<u8>>;
type NonXmlSnapshot = BTreeMap<String, String>;
type NonXmlPayloadSnapshot = BTreeMap<String, Vec<u8>>;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
struct XmlDelta {
    created: Vec<String>,
    modified: Vec<String>,
    removed: Vec<String>,
    unchanged: Vec<String>,
}

fn classify_xml_delta(before: &XmlSnapshot, after: &XmlSnapshot) -> XmlDelta {
    let mut delta = XmlDelta::default();
    for (path, before_hash) in before {
        match after.get(path) {
            None => delta.removed.push(path.clone()),
            Some(after_hash) if after_hash != before_hash => delta.modified.push(path.clone()),
            Some(_) => delta.unchanged.push(path.clone()),
        }
    }
    for path in after.keys() {
        if !before.contains_key(path) {
            delta.created.push(path.clone());
        }
    }
    delta
}

fn enforce_xml_impact(
    impact: XmlImpactClass,
    before: &XmlSnapshot,
    after: &XmlSnapshot,
) -> Result<XmlDelta, String> {
    let delta = classify_xml_delta(before, after);
    match impact {
        XmlImpactClass::None if before != after => {
            Err("XML impact None changed the complete XML map".to_string())
        }
        XmlImpactClass::CreateOrModify if delta.created.is_empty() && delta.modified.is_empty() => {
            Err("CreateOrModify case produced no XML create/modify delta".to_string())
        }
        XmlImpactClass::RemoveOrModify if delta.removed.is_empty() || delta.modified.is_empty() => {
            Err("RemoveOrModify case must remove XML and modify a surviving owner".to_string())
        }
        _ => Ok(delta),
    }
}

#[derive(Debug, Default)]
struct SequentialCallGate {
    target_call_active: bool,
    completed_target_calls: usize,
}

impl SequentialCallGate {
    fn begin(&mut self) -> Result<(), String> {
        if self.target_call_active {
            return Err("overlapping target calls are forbidden".to_string());
        }
        self.target_call_active = true;
        Ok(())
    }

    fn finish(&mut self) -> Result<(), String> {
        if !self.target_call_active {
            return Err("target call was not active".to_string());
        }
        self.target_call_active = false;
        self.completed_target_calls += 1;
        Ok(())
    }
}

fn common_args(workspace: &Path) -> Map<String, Value> {
    Map::from_iter([
        (
            "cwd".to_string(),
            Value::String(workspace.display().to_string()),
        ),
        ("dryRun".to_string(), Value::Bool(false)),
    ])
}

fn call_public_tool(tool: &str, args: &Map<String, Value>) -> Result<String, String> {
    assert_eq!(args.get("dryRun"), Some(&Value::Bool(false)));
    let app = UnicaApplication::new();
    let result = app.call_tool(tool, args)?;
    if !result.ok || !result.errors.is_empty() {
        return Err(format!(
            "{tool} failed: {}; errors={:?}",
            result.summary, result.errors
        ));
    }
    Ok(result.summary)
}

fn call_target_tool(
    gate: &mut SequentialCallGate,
    tool: &str,
    args: &Map<String, Value>,
) -> Result<String, String> {
    gate.begin()?;
    let result = call_public_tool(tool, args);
    gate.finish()?;
    result
}

fn sha256_file(path: &Path) -> Result<String, String> {
    let bytes =
        fs::read(path).map_err(|error| format!("cannot read {}: {error}", path.display()))?;
    Ok(format!("{:x}", Sha256::digest(bytes)))
}

fn visit_xml_files(
    root: &Path,
    directory: &Path,
    snapshot: &mut XmlSnapshot,
) -> Result<(), String> {
    let mut entries = fs::read_dir(directory)
        .map_err(|error| format!("cannot read {}: {error}", directory.display()))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("cannot enumerate {}: {error}", directory.display()))?;
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        let path = entry.path();
        let file_type = entry
            .file_type()
            .map_err(|error| format!("cannot inspect {}: {error}", path.display()))?;
        if file_type.is_symlink() {
            return Err(format!(
                "workspace symlink is forbidden: {}",
                path.display()
            ));
        }
        if file_type.is_dir() {
            visit_xml_files(root, &path, snapshot)?;
        } else if file_type.is_file()
            && path
                .extension()
                .is_some_and(|extension| extension.eq_ignore_ascii_case("xml"))
        {
            let relative = path
                .strip_prefix(root)
                .map_err(|error| format!("XML path escaped workspace: {error}"))?
                .to_string_lossy()
                .replace('\\', "/");
            snapshot.insert(relative, sha256_file(&path)?);
        }
    }
    Ok(())
}

fn snapshot_xml(workspace: &Path) -> Result<XmlSnapshot, String> {
    let mut snapshot = XmlSnapshot::new();
    visit_xml_files(workspace, workspace, &mut snapshot)?;
    Ok(snapshot)
}

fn capture_empty_directory_paths(root: &Path) -> Result<Vec<String>, String> {
    fn visit(root: &Path, directory: &Path, paths: &mut Vec<String>) -> Result<(), String> {
        let mut entries = fs::read_dir(directory)
            .map_err(|error| format!("cannot read {}: {error}", directory.display()))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| format!("cannot enumerate {}: {error}", directory.display()))?;
        entries.sort_by_key(|entry| entry.file_name());
        if entries.is_empty() && directory != root {
            paths.push(
                directory
                    .strip_prefix(root)
                    .map_err(|error| format!("empty directory escaped root: {error}"))?
                    .to_string_lossy()
                    .replace('\\', "/"),
            );
        }
        for entry in entries {
            let path = entry.path();
            let file_type = entry
                .file_type()
                .map_err(|error| format!("cannot inspect {}: {error}", path.display()))?;
            if file_type.is_symlink() {
                return Err(format!(
                    "workspace symlink is forbidden: {}",
                    path.display()
                ));
            }
            if file_type.is_dir() {
                visit(root, &path, paths)?;
            } else if !file_type.is_file() {
                return Err(format!(
                    "special workspace entry is forbidden: {}",
                    path.display()
                ));
            }
        }
        Ok(())
    }

    let mut paths = Vec::new();
    visit(root, root, &mut paths)?;
    paths.sort();
    Ok(paths)
}

fn capture_xml_payloads_recursive(
    workspace: &Path,
    directory: &Path,
    payloads: &mut XmlPayloadSnapshot,
) -> Result<(), String> {
    let mut entries = fs::read_dir(directory)
        .map_err(|error| format!("cannot read {}: {error}", directory.display()))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("cannot enumerate {}: {error}", directory.display()))?;
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        let source = entry.path();
        let file_type = entry
            .file_type()
            .map_err(|error| format!("cannot inspect {}: {error}", source.display()))?;
        if file_type.is_symlink() {
            return Err(format!(
                "workspace symlink is forbidden: {}",
                source.display()
            ));
        }
        if file_type.is_dir() {
            capture_xml_payloads_recursive(workspace, &source, payloads)?;
        } else if file_type.is_file()
            && source
                .extension()
                .is_some_and(|extension| extension.eq_ignore_ascii_case("xml"))
        {
            platform_support::require_single_link(&source)?;
            let relative = source
                .strip_prefix(workspace)
                .map_err(|error| format!("pre-snapshot XML escaped workspace: {error}"))?;
            let relative_text = relative.to_string_lossy().replace('\\', "/");
            let payload = fs::read(&source)
                .map_err(|error| format!("cannot read {}: {error}", source.display()))?;
            payloads.insert(relative_text, payload);
        } else if !file_type.is_file() {
            return Err(format!(
                "special workspace entry is forbidden: {}",
                source.display()
            ));
        }
    }
    Ok(())
}

fn capture_xml_payloads(workspace: &Path) -> Result<XmlPayloadSnapshot, String> {
    let mut payloads = XmlPayloadSnapshot::new();
    capture_xml_payloads_recursive(workspace, workspace, &mut payloads)?;
    Ok(payloads)
}

fn hashes_for_xml_payloads(payloads: &XmlPayloadSnapshot) -> XmlSnapshot {
    payloads
        .iter()
        .map(|(relative, payload)| (relative.clone(), format!("{:x}", Sha256::digest(payload))))
        .collect()
}

fn materialize_pre_xml(pre_root: &Path, payloads: &XmlPayloadSnapshot) -> Result<(), String> {
    fs::create_dir(pre_root)
        .map_err(|error| format!("cannot create {}: {error}", pre_root.display()))?;
    for (relative, payload) in payloads {
        let destination = pre_root.join(relative);
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent)
                .map_err(|error| format!("cannot create {}: {error}", parent.display()))?;
        }
        fs::write(&destination, payload)
            .map_err(|error| format!("cannot write {}: {error}", destination.display()))?;
    }
    let copied = snapshot_xml(pre_root)?;
    if copied != hashes_for_xml_payloads(payloads) {
        return Err("copied pre-snapshot XML bytes do not match the captured source".to_string());
    }
    Ok(())
}

fn materialize_pre_non_xml(
    case: &ExecutableCase,
    pre_root: &Path,
    payloads: &NonXmlPayloadSnapshot,
) -> Result<(), String> {
    fs::create_dir(pre_root)
        .map_err(|error| format!("cannot create {}: {error}", pre_root.display()))?;
    for (relative, payload) in payloads {
        let destination = pre_root.join(relative);
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent)
                .map_err(|error| format!("cannot create {}: {error}", parent.display()))?;
        }
        fs::write(&destination, payload)
            .map_err(|error| format!("cannot write {}: {error}", destination.display()))?;
    }
    let copied = capture_non_xml_payloads(case, pre_root)?;
    if copied != *payloads {
        return Err(
            "copied pre-snapshot non-XML bytes do not match the captured source".to_string(),
        );
    }
    Ok(())
}

fn require_xml_payloads_unchanged(
    root: &Path,
    expected: &XmlPayloadSnapshot,
    label: &str,
) -> Result<(), String> {
    let current = capture_xml_payloads(root)?;
    if &current != expected {
        return Err(format!(
            "{label} XML changed after its immutable byte snapshot"
        ));
    }
    Ok(())
}

fn safe_workspace_relative_path(workspace: &Path, path: &Path) -> Result<String, String> {
    let relative = path
        .strip_prefix(workspace)
        .map_err(|error| format!("platform payload escaped workspace: {error}"))?;
    let mut parts = Vec::new();
    for component in relative.components() {
        let Component::Normal(component) = component else {
            return Err(format!("unsafe platform payload path: {}", path.display()));
        };
        let component = component.to_str().ok_or_else(|| {
            format!(
                "non-UTF-8 platform payload path is forbidden: {}",
                path.display()
            )
        })?;
        if component.contains('\\') {
            return Err(format!(
                "non-portable platform payload path is forbidden: {}",
                path.display()
            ));
        }
        parts.push(component);
    }
    if parts.is_empty() {
        return Err(format!("empty platform payload path: {}", path.display()));
    }
    Ok(parts.join("/"))
}

fn capture_non_xml_payloads_recursive(
    workspace: &Path,
    directory: &Path,
    payloads: &mut NonXmlPayloadSnapshot,
) -> Result<(), String> {
    let mut entries = fs::read_dir(directory)
        .map_err(|error| format!("cannot read {}: {error}", directory.display()))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("cannot enumerate {}: {error}", directory.display()))?;
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        let source = entry.path();
        let file_type = entry
            .file_type()
            .map_err(|error| format!("cannot inspect {}: {error}", source.display()))?;
        if file_type.is_symlink() {
            return Err(format!(
                "platform source symlink is forbidden: {}",
                source.display()
            ));
        }
        if file_type.is_dir() {
            capture_non_xml_payloads_recursive(workspace, &source, payloads)?;
        } else if file_type.is_file()
            && !source
                .extension()
                .is_some_and(|extension| extension.eq_ignore_ascii_case("xml"))
        {
            platform_support::require_single_link(&source)?;
            let relative = safe_workspace_relative_path(workspace, &source)?;
            let payload = fs::read(&source)
                .map_err(|error| format!("cannot read {}: {error}", source.display()))?;
            if payloads.insert(relative.clone(), payload).is_some() {
                return Err(format!(
                    "duplicate platform non-XML payload path: {relative}"
                ));
            }
        } else if !file_type.is_file() {
            return Err(format!(
                "special platform source entry is forbidden: {}",
                source.display()
            ));
        }
    }
    Ok(())
}

fn capture_non_xml_payloads(
    case: &ExecutableCase,
    workspace: &Path,
) -> Result<NonXmlPayloadSnapshot, String> {
    let mut payloads = NonXmlPayloadSnapshot::new();
    for relative_root in platform_source_root_relatives(case)? {
        let root = workspace.join(relative_root);
        match fs::symlink_metadata(&root) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err(format!(
                    "platform source root symlink is forbidden: {}",
                    root.display()
                ));
            }
            Ok(metadata) if metadata.is_dir() => {
                capture_non_xml_payloads_recursive(workspace, &root, &mut payloads)?;
            }
            Ok(_) => {
                return Err(format!(
                    "platform source root is not a directory: {}",
                    root.display()
                ));
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(format!("cannot inspect {}: {error}", root.display()));
            }
        }
    }
    Ok(payloads)
}

fn capture_auxiliary_payloads(
    case: &ExecutableCase,
    workspace: &Path,
) -> Result<NonXmlPayloadSnapshot, String> {
    let mut payloads = NonXmlPayloadSnapshot::new();
    capture_non_xml_payloads_recursive(workspace, workspace, &mut payloads)?;
    let platform_roots = platform_source_root_relatives(case)?;
    payloads.retain(|relative, _| {
        !platform_roots
            .iter()
            .any(|root| relative == root || relative.starts_with(&format!("{root}/")))
    });
    Ok(payloads)
}

fn remove_internal_workspace_cache(workspace: &Path) -> Result<(), String> {
    let build_root = workspace.join(".build");
    let cache_root = build_root.join("unica");
    for path in [&build_root, &cache_root] {
        match fs::symlink_metadata(path) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err(format!(
                    "internal workspace cache symlink is forbidden: {}",
                    path.display()
                ));
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(error) => {
                return Err(format!("cannot inspect {}: {error}", path.display()));
            }
        }
    }
    if !cache_root.is_dir() {
        return Err(format!(
            "internal workspace cache is not a directory: {}",
            cache_root.display()
        ));
    }
    fs::remove_dir_all(&cache_root)
        .map_err(|error| format!("cannot remove {}: {error}", cache_root.display()))
}

fn hashes_for_non_xml_payloads(payloads: &NonXmlPayloadSnapshot) -> NonXmlSnapshot {
    payloads
        .iter()
        .map(|(relative, payload)| (relative.clone(), format!("{:x}", Sha256::digest(payload))))
        .collect()
}

fn require_non_xml_payloads_unchanged(
    case: &ExecutableCase,
    workspace: &Path,
    expected: &NonXmlPayloadSnapshot,
    label: &str,
) -> Result<(), String> {
    let current = capture_non_xml_payloads(case, workspace)?;
    if &current != expected {
        return Err(format!(
            "{label} non-XML changed after its immutable byte inventory"
        ));
    }
    Ok(())
}

fn registry_entry_for_case(case_id: &str) -> &'static MutatorRegistryEntry {
    MUTATOR_REGISTRY
        .iter()
        .find(|entry| entry.case_ids.contains(&case_id))
        .unwrap_or_else(|| panic!("case is absent from registry: {case_id}"))
}

fn effective_xml_impact(case_id: &str) -> XmlImpactClass {
    if case_id == "cfe-patch-method-catalog-form-module" {
        // cfe.borrow must already mark an adopted managed form as Extended.
        // Patching its module is therefore idempotent for XML while the other
        // supported module families atomically add their PropertyState.
        XmlImpactClass::None
    } else {
        registry_entry_for_case(case_id).impact
    }
}

fn cf_init_args(workspace: &Path, name: &str, output_dir: &str) -> Map<String, Value> {
    let mut args = common_args(workspace);
    args.insert("Name".to_string(), Value::String(name.to_string()));
    args.insert(
        "OutputDir".to_string(),
        Value::String(output_dir.to_string()),
    );
    args
}

static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

fn unique_temp_dir(label: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "unica-xml-corpus-{label}-{}-{}",
        std::process::id(),
        TEMP_COUNTER.fetch_add(1, Ordering::Relaxed)
    ))
}

fn write_designer_project(
    workspace: &Path,
    source_sets: &[(&str, &str, &str)],
) -> Result<(), String> {
    let mut text = "format: DESIGNER\nsource-set:\n".to_string();
    for (name, kind, path) in source_sets {
        text.push_str(&format!(
            "  - name: {name}\n    type: {kind}\n    path: {path}\n"
        ));
    }
    fs::write(workspace.join("v8project.yaml"), text)
        .map_err(|error| format!("cannot write v8project.yaml: {error}"))
}

fn copy_fixture_tree(source: &Path, destination: &Path) -> Result<(), String> {
    let metadata = fs::symlink_metadata(source)
        .map_err(|error| format!("cannot inspect fixture {}: {error}", source.display()))?;
    if metadata.file_type().is_symlink() {
        return Err(format!(
            "fixture symlink is forbidden: {}",
            source.display()
        ));
    }
    if metadata.is_file() {
        platform_support::require_single_link(source)?;
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent)
                .map_err(|error| format!("cannot create {}: {error}", parent.display()))?;
        }
        let payload = fs::read(source)
            .map_err(|error| format!("cannot read fixture {}: {error}", source.display()))?;
        return fs::write(destination, payload)
            .map_err(|error| format!("cannot copy fixture to {}: {error}", destination.display()));
    }
    if !metadata.is_dir() {
        return Err(format!(
            "special fixture entry is forbidden: {}",
            source.display()
        ));
    }
    fs::create_dir(destination)
        .map_err(|error| format!("cannot create {}: {error}", destination.display()))?;
    let mut entries = fs::read_dir(source)
        .map_err(|error| format!("cannot read fixture {}: {error}", source.display()))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("cannot enumerate fixture {}: {error}", source.display()))?;
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        copy_fixture_tree(&entry.path(), &destination.join(entry.file_name()))?;
    }
    Ok(())
}

fn seed_platform_support_fixture(workspace: &Path) -> Result<(), String> {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("tests/fixtures/platform_8_3_27/support-edit-bin-only/src");
    let destination = workspace.join("src");
    copy_fixture_tree(&fixture, &destination)?;
    write_designer_project(workspace, &[("main", "CONFIGURATION", "src")])?;
    for required in [
        destination.join("Configuration.xml"),
        destination.join("Ext/ParentConfigurations.bin"),
        destination.join("Ext/ParentConfigurations"),
    ] {
        if !required.exists() {
            return Err(format!(
                "platform support fixture is incomplete: {}",
                required.display()
            ));
        }
    }
    let vendor_payload_count = fs::read_dir(destination.join("Ext/ParentConfigurations"))
        .map_err(|error| format!("cannot enumerate support vendor payloads: {error}"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("cannot enumerate support vendor payloads: {error}"))?
        .into_iter()
        .filter(|entry| {
            entry
                .path()
                .extension()
                .is_some_and(|extension| extension.eq_ignore_ascii_case("cf"))
        })
        .count();
    if vendor_payload_count == 0 {
        return Err("platform support fixture has no Ext/ParentConfigurations/*.cf".to_string());
    }
    Ok(())
}

fn seed_configuration(workspace: &Path) -> Result<(), String> {
    call_public_tool(
        "unica.cf.init",
        &cf_init_args(workspace, "CorpusConfiguration", "src"),
    )?;
    write_designer_project(workspace, &[("main", "CONFIGURATION", "src")])
}

fn write_json_input(workspace: &Path, name: &str, value: &Value) -> Result<String, String> {
    let relative = format!("inputs/{name}.json");
    let path = workspace.join(&relative);
    fs::create_dir_all(path.parent().expect("input parent"))
        .map_err(|error| format!("cannot create inputs: {error}"))?;
    let bytes = serde_json::to_vec_pretty(value)
        .map_err(|error| format!("cannot serialize input {name}: {error}"))?;
    fs::write(&path, bytes).map_err(|error| format!("cannot write {}: {error}", path.display()))?;
    Ok(relative)
}

fn meta_compile_args(workspace: &Path, json_path: &str) -> Map<String, Value> {
    let mut args = common_args(workspace);
    args.insert("JsonPath".to_string(), Value::String(json_path.to_string()));
    args.insert("OutputDir".to_string(), Value::String("src".to_string()));
    args
}

fn seed_metadata(workspace: &Path, input_name: &str, definition: Value) -> Result<(), String> {
    let path = write_json_input(workspace, input_name, &definition)?;
    call_public_tool("unica.meta.compile", &meta_compile_args(workspace, &path))?;
    Ok(())
}

fn catalog_definition(name: &str) -> Value {
    json!({
        "type": "Catalog",
        "name": name,
        "synonym": "Corpus catalog",
        "codeLength": 9,
        "descriptionLength": 50,
        "attributes": [{"name": "Article", "type": "String(32)"}]
    })
}

fn seed_catalog(workspace: &Path) -> Result<(), String> {
    seed_metadata(
        workspace,
        "seed-catalog",
        catalog_definition("CorpusCatalog"),
    )
}

fn form_add_args(workspace: &Path) -> Map<String, Value> {
    let mut args = common_args(workspace);
    args.insert(
        "ObjectPath".to_string(),
        Value::String("src/Catalogs/CorpusCatalog.xml".to_string()),
    );
    args.insert(
        "FormName".to_string(),
        Value::String("CorpusForm".to_string()),
    );
    args.insert("Purpose".to_string(), Value::String("Object".to_string()));
    args.insert("SetDefault".to_string(), Value::Bool(true));
    args
}

fn seed_catalog_form(workspace: &Path) -> Result<(), String> {
    seed_catalog(workspace)?;
    call_public_tool("unica.form.add", &form_add_args(workspace))?;
    Ok(())
}

fn report_definition(name: &str) -> Value {
    json!({"type": "Report", "name": name, "synonym": "Corpus report"})
}

fn seed_report(workspace: &Path) -> Result<(), String> {
    seed_metadata(workspace, "seed-report", report_definition("CorpusReport"))
}

fn report_form_add_args(workspace: &Path) -> Map<String, Value> {
    let mut args = common_args(workspace);
    args.insert(
        "ObjectPath".to_string(),
        Value::String("src/Reports/CorpusReport.xml".to_string()),
    );
    args.insert(
        "FormName".to_string(),
        Value::String("CorpusForm".to_string()),
    );
    args.insert("Purpose".to_string(), Value::String("Object".to_string()));
    args.insert("SetDefault".to_string(), Value::Bool(true));
    args
}

fn seed_report_form(workspace: &Path) -> Result<(), String> {
    seed_report(workspace)?;
    call_public_tool("unica.form.add", &report_form_add_args(workspace))?;
    Ok(())
}

fn seed_document_recorder(workspace: &Path, register: &str) -> Result<(), String> {
    seed_metadata(
        workspace,
        "seed-recorder-document",
        json!({
            "type": "Document",
            "name": "CorpusRecorder",
            "registerRecords": [register]
        }),
    )
}

fn seed_event_handlers(workspace: &Path) -> Result<(), String> {
    seed_metadata(
        workspace,
        "seed-event-handlers",
        json!({
            "type": "CommonModule",
            "name": "EventHandlers",
            "context": "server"
        }),
    )?;
    fs::write(
        workspace.join("src/CommonModules/EventHandlers/Ext/Module.bsl"),
        concat!(
            "\u{feff}Procedure RunJob() Export\r\n",
            "EndProcedure\r\n\r\n",
            "Procedure OnBeforeWrite(Source, Cancel) Export\r\n",
            "EndProcedure\r\n"
        ),
    )
    .map_err(|error| format!("cannot seed EventHandlers module: {error}"))
}

fn template_args(workspace: &Path, template_name: &str, template_type: &str) -> Map<String, Value> {
    let mut args = common_args(workspace);
    args.insert(
        "ObjectName".to_string(),
        Value::String("CorpusReport".to_string()),
    );
    args.insert(
        "TemplateName".to_string(),
        Value::String(template_name.to_string()),
    );
    args.insert(
        "TemplateType".to_string(),
        Value::String(template_type.to_string()),
    );
    args.insert(
        "SrcDir".to_string(),
        Value::String("src/Reports".to_string()),
    );
    args
}

fn seed_dcs_template(workspace: &Path) -> Result<(), String> {
    call_public_tool(
        "unica.template.add",
        &template_args(workspace, "CorpusDcs", "DataCompositionSchema"),
    )?;
    let mut compile = common_args(workspace);
    compile.insert(
        "Value".to_string(),
        Value::String(dcs_definition().to_string()),
    );
    compile.insert(
        "OutputPath".to_string(),
        Value::String("src/Reports/CorpusReport/Templates/CorpusDcs/Ext/Template.xml".to_string()),
    );
    call_public_tool("unica.dcs.compile", &compile)?;
    Ok(())
}

fn subsystem_compile_args(
    workspace: &Path,
    name: &str,
    parent: Option<&str>,
) -> Map<String, Value> {
    let mut args = common_args(workspace);
    args.insert(
        "Value".to_string(),
        Value::String(json!({"name": name}).to_string()),
    );
    args.insert("OutputDir".to_string(), Value::String("src".to_string()));
    if let Some(parent) = parent {
        args.insert("Parent".to_string(), Value::String(parent.to_string()));
    }
    args
}

fn seed_parent_subsystem(workspace: &Path) -> Result<(), String> {
    call_public_tool(
        "unica.subsystem.compile",
        &subsystem_compile_args(workspace, "CorpusParent", None),
    )?;
    Ok(())
}

fn cfe_init_args(workspace: &Path) -> Map<String, Value> {
    let mut args = common_args(workspace);
    args.insert(
        "Name".to_string(),
        Value::String("CorpusExtension".to_string()),
    );
    args.insert("OutputDir".to_string(), Value::String("ext".to_string()));
    args.insert(
        "ConfigPath".to_string(),
        Value::String("src/Configuration.xml".to_string()),
    );
    args
}

fn seed_extension(workspace: &Path) -> Result<(), String> {
    write_designer_project(
        workspace,
        &[
            ("main", "CONFIGURATION", "src"),
            ("extension", "EXTENSION", "ext"),
        ],
    )?;
    call_public_tool("unica.cfe.init", &cfe_init_args(workspace))?;
    Ok(())
}

fn dcs_definition() -> Value {
    json!({
        "dataSets": [
            {
                "name": "Main",
                "query": "SELECT 1 AS Value, 10 AS Amount",
                "fields": ["Value:String", "Amount:Number(15,2)"]
            },
            {
                "name": "CatalogObject",
                "objectName": "Catalog.CorpusCatalog",
                "fields": [{
                    "dataPath": "Description",
                    "field": "Description",
                    "type": "String",
                    "presentationExpression": "Description"
                }]
            },
            {
                "name": "Combined",
                "fields": ["Value:String"],
                "items": [
                    {
                        "name": "UnionFirst",
                        "query": "SELECT 1 AS Value",
                        "fields": ["Value:String"]
                    },
                    {
                        "name": "UnionSecond",
                        "query": "SELECT 2 AS Value",
                        "fields": ["Value:String"]
                    }
                ]
            }
        ],
        "dataSetLinks": [{
            "source": "Main",
            "dest": "CatalogObject",
            "sourceExpr": "Value",
            "destExpr": "Description",
            "parameter": "Режим",
            "parameterListAllowed": true,
            "linkConditionExpression": "Value = Description",
            "startExpression": "Value",
            "required": true
        }],
        "calculatedFields": [{
            "dataPath": "CalculatedAmount",
            "expression": "Amount * 2",
            "title": "Calculated amount",
            "restrict": ["noFilter", "noGroup"],
            "useRestriction": ["noField", "noOrder"],
            "type": "decimal(15,2)"
        }],
        "parameters": [
            "Период: StandardPeriod = LastMonth @autoDates",
            {
                "name": "ТипКорпуса",
                "type": "string(16)",
                "availableAsField": false,
                "use": "Always"
            },
            {
                "name": "Режим",
                "title": "Mode",
                "type": "string(16)",
                "value": "A",
                "useRestriction": true,
                "expression": "&Source.Mode",
                "availableValues": [
                    {"value": "A", "presentation": "Alpha"},
                    {"value": "B", "presentation": "Beta"}
                ],
                "valueListAllowed": true,
                "availableAsField": false,
                "denyIncompleteValues": true,
                "use": "Always"
            }
        ],
        "settingsVariants": [{
            "name": "Main",
            "presentation": "Corpus settings",
            "settings": {
                "selection": ["Value", "Amount"],
                "filter": ["Amount > 0"],
                "dataParameters": [{"parameter": "Режим", "value": "A"}],
                "order": [{"field": "Amount", "direction": "Desc"}],
                "conditionalAppearance": [{
                    "fields": ["Amount"],
                    "filter": ["Amount > 0"],
                    "appearance": {"ЦветТекста": "web:Red"}
                }],
                "outputParameters": {"Заголовок": "Corpus report"},
                "structure": [{
                    "type": "group",
                    "name": "Corpus group",
                    "groupBy": ["Value"],
                    "filter": ["Amount > 0"],
                    "order": [{"field": "Amount", "direction": "Desc"}],
                    "selection": ["Value", "Amount"],
                    "conditionalAppearance": [{
                        "fields": ["Amount"],
                        "filter": ["Amount > 0"],
                        "appearance": {"ЦветТекста": "web:Red"}
                    }],
                    "outputParameters": {"Заголовок": "Corpus group"},
                    "children": [{"type": "group", "name": "Details"}]
                }]
            }
        }]
    })
}

fn meta_definition(kind: &str) -> Option<Value> {
    Some(match kind {
        "Catalog" => json!({
            "type": "Catalog",
            "name": "CorpusCatalog",
            "synonym": "Corpus catalog",
            "codeLength": 9,
            "descriptionLength": 50,
            "attributes": [
                {"name": "Article", "type": "String(32)"},
                {"name": "TypedValue", "type": "DefinedType.CorpusDefinedType"},
                {"name": "Storage", "type": "ValueStorage"}
            ]
        }),
        "Document" => json!({
            "type": "Document", "name": "CorpusDocument", "numberLength": 8,
            "attributes": ["Partner:String(100)|req,index"],
            "tabularSections": {"Lines": ["Quantity:Number(10,2)"]}
        }),
        "Enum" => json!({"type": "Enum", "name": "CorpusEnum", "values": ["New", "Closed"]}),
        "Constant" => json!({"type": "Constant", "name": "CorpusConstant", "valueType": "Boolean"}),
        "InformationRegister" => json!({
            "type": "InformationRegister", "name": "CorpusInformationRegister", "periodicity": "Month",
            "dimensions": ["Item:String(50)|master,index"], "resources": ["Price:Number(15,2)"]
        }),
        "AccumulationRegister" => json!({
            "type": "AccumulationRegister", "name": "CorpusAccumulationRegister", "registerType": "Balances",
            "dimensions": ["Warehouse:String(50)|index"], "resources": ["Quantity:Number(15,3)"]
        }),
        "AccountingRegister" => json!({
            "type": "AccountingRegister", "name": "CorpusAccountingRegister",
            "chartOfAccounts": "ChartOfAccounts.CorpusAccounts",
            "dimensions": ["Department:String(50)"], "resources": ["Amount:Number(15,2)"]
        }),
        "CalculationRegister" => json!({
            "type": "CalculationRegister", "name": "CorpusCalculationRegister",
            "chartOfCalculationTypes": "ChartOfCalculationTypes.CorpusCalculationTypes", "periodicity": "Month",
            "dimensions": ["Employee:String(50)"], "resources": ["Result:Number(15,2)"]
        }),
        "ChartOfAccounts" => json!({
            "type": "ChartOfAccounts", "name": "CorpusAccounts",
            "accountingFlags": ["Tax"]
        }),
        "ChartOfCharacteristicTypes" => json!({
            "type": "ChartOfCharacteristicTypes", "name": "CorpusCharacteristics",
            "valueTypes": ["String(50)", "Number(15,2)"]
        }),
        "ChartOfCalculationTypes" => json!({
            "type": "ChartOfCalculationTypes", "name": "CorpusCalculationTypes",
            "dependenceOnCalculationTypes": "OnActionPeriod",
            "baseCalculationTypes": ["ChartOfCalculationTypes.CorpusCalculationTypes"]
        }),
        "BusinessProcess" => json!({
            "type": "BusinessProcess", "name": "CorpusBusinessProcess", "task": "Task.CorpusTask",
            "attributes": ["Subject:String(100)"]
        }),
        "Task" => json!({
            "type": "Task", "name": "CorpusTask",
            "attributes": ["Priority:Number(3,0)"]
        }),
        "ExchangePlan" => json!({
            "type": "ExchangePlan", "name": "CorpusExchangePlan", "distributedInfoBase": true,
            "includeConfigurationExtensions": true, "attributes": ["NodeKind:String(20)"]
        }),
        "DocumentJournal" => json!({
            "type": "DocumentJournal", "name": "CorpusDocumentJournal",
            "registeredDocuments": ["Document.CorpusDocument"],
            "columns": [{"name": "Partner", "references": ["Document.CorpusDocument"]}]
        }),
        "Report" => report_definition("CorpusReport"),
        "DataProcessor" => json!({
            "type": "DataProcessor", "name": "CorpusDataProcessor", "attributes": ["FileName:String(260)"],
            "tabularSections": {"Rows": ["Value:String(100)"]}
        }),
        "CommonModule" => json!({
            "type": "CommonModule", "name": "CorpusCommonModule", "context": "server",
            "returnValuesReuse": "DuringRequest"
        }),
        "ScheduledJob" => json!({
            "type": "ScheduledJob", "name": "CorpusScheduledJob", "methodName": "EventHandlers.RunJob",
            "description": "Corpus job", "key": "corpus", "use": true, "predefined": true
        }),
        "EventSubscription" => json!({
            "type": "EventSubscription", "name": "CorpusEventSubscription",
            "source": ["String(10)", "DocumentObject.CorpusDocument", "CatalogObject.CorpusCatalog"], "event": "BeforeWrite",
            "handler": "EventHandlers.OnBeforeWrite"
        }),
        "HTTPService" => json!({
            "type": "HTTPService", "name": "CorpusHTTPService", "rootURL": "corpus", "reuseSessions": "AutoUse",
            "urlTemplates": {"Items": {"template": "/items/{id}", "methods": {"Get": "GET"}}}
        }),
        "WebService" => json!({
            "type": "WebService", "name": "CorpusWebService", "namespace": "urn:corpus", "reuseSessions": "AutoUse",
            "operations": {"Ping": {"returnType": "xs:string", "parameters": {"Text": "xs:string"}}}
        }),
        "DefinedType" => json!({
            "type": "DefinedType", "name": "CorpusDefinedType", "valueTypes": ["String(100)", "Number(15,2)"]
        }),
        _ => return None,
    })
}

fn prepare_target(case: &ExecutableCase, workspace: &Path) -> Result<Map<String, Value>, String> {
    fs::create_dir_all(workspace)
        .map_err(|error| format!("cannot create {}: {error}", workspace.display()))?;

    if case.id == "cf-init-default" {
        return Ok(cf_init_args(workspace, "CorpusConfiguration", "src"));
    }
    if matches!(case.id, "epf-init-managed-form" | "erf-init-managed-form") {
        let (kind, path, name, form) = if case.id.starts_with("epf") {
            (
                "EXTERNAL_DATA_PROCESSORS",
                "epf",
                "CorpusProcessor",
                "CorpusForm",
            )
        } else {
            ("EXTERNAL_REPORTS", "erf", "CorpusReport", "CorpusForm")
        };
        write_designer_project(workspace, &[("external", kind, path)])?;
        let mut args = common_args(workspace);
        args.insert("Name".to_string(), Value::String(name.to_string()));
        args.insert("OutputDir".to_string(), Value::String(path.to_string()));
        args.insert("FormName".to_string(), Value::String(form.to_string()));
        return Ok(args);
    }
    if case.id == "dcs-compile-owned-template" {
        seed_configuration(workspace)?;
        seed_report(workspace)?;
        seed_catalog(workspace)?;
        seed_metadata(
            workspace,
            "seed-unicode-defined-type",
            json!({
                "type": "DefinedType",
                "name": "ТипКорпуса",
                "valueTypes": ["String(16)"]
            }),
        )?;
        call_public_tool(
            "unica.template.add",
            &template_args(workspace, "CorpusTemplate", "DataCompositionSchema"),
        )?;
        let output = "src/Reports/CorpusReport/Templates/CorpusTemplate/Ext/Template.xml";
        fs::remove_file(workspace.join(output))
            .map_err(|error| format!("cannot remove preparatory DCS content: {error}"))?;
        let mut args = common_args(workspace);
        args.insert(
            "Value".to_string(),
            Value::String(dcs_definition().to_string()),
        );
        args.insert("OutputPath".to_string(), Value::String(output.to_string()));
        return Ok(args);
    }
    if case.id == "mxl-compile-owned-template" {
        seed_configuration(workspace)?;
        seed_report(workspace)?;
        call_public_tool(
            "unica.template.add",
            &template_args(workspace, "CorpusTemplate", "SpreadsheetDocument"),
        )?;
        let output = "src/Reports/CorpusReport/Templates/CorpusTemplate/Ext/Template.xml";
        fs::remove_file(workspace.join(output))
            .map_err(|error| format!("cannot remove preparatory MXL content: {error}"))?;
        let path = write_json_input(
            workspace,
            "mxl",
            &json!({
                "columns": 5,
                "defaultWidth": 10,
                "columnWidths": {"3": 20},
                "styles": {"right": {"align": "right"}},
                "areas": [{
                    "name": "A",
                    "rows": [{"cells": [
                        {"col": 1, "span": 2, "text": "spanned"},
                        {"col": 3, "text": "adjacent", "style": "right"},
                        {"col": 5, "text": "after gap"}
                    ]}]
                }]
            }),
        )?;
        let mut args = common_args(workspace);
        args.insert("JsonPath".to_string(), Value::String(path));
        args.insert("OutputPath".to_string(), Value::String(output.to_string()));
        return Ok(args);
    }
    if case.id == "support-edit-bin-only" {
        seed_platform_support_fixture(workspace)?;
        let mut args = common_args(workspace);
        args.insert("Path".to_string(), Value::String("src".to_string()));
        args.insert("Capability".to_string(), Value::String("off".to_string()));
        return Ok(args);
    }

    seed_configuration(workspace)?;

    if case.id.starts_with("cf-edit-") {
        if case.id == "cf-edit-set-home-page" {
            seed_catalog_form(workspace)?;
        }
        let (operation, value) = match case.id {
            "cf-edit-root-property" => ("modify-property", "Version=1.0".to_string()),
            "cf-edit-set-panels" => ("set-panels", json!({"top": ["open"]}).to_string()),
            "cf-edit-set-home-page" => (
                "set-home-page",
                json!({
                    "template": "OneColumn",
                    "left": [{
                        "form": "Catalog.CorpusCatalog.Form.CorpusForm",
                        "height": 15,
                        "visibility": true
                    }]
                })
                .to_string(),
            ),
            _ => unreachable!(),
        };
        let mut args = common_args(workspace);
        args.insert("ConfigPath".to_string(), Value::String("src".to_string()));
        args.insert(
            "Operation".to_string(),
            Value::String(operation.to_string()),
        );
        args.insert("Value".to_string(), Value::String(value));
        args.insert("NoValidate".to_string(), Value::Bool(true));
        return Ok(args);
    }

    if case.id == "cfe-init-default" {
        write_designer_project(
            workspace,
            &[
                ("main", "CONFIGURATION", "src"),
                ("extension", "EXTENSION", "ext"),
            ],
        )?;
        return Ok(cfe_init_args(workspace));
    }

    if case.id.starts_with("cfe-borrow-") {
        seed_catalog(workspace)?;
        if case.id.ends_with("managed-form") {
            call_public_tool("unica.form.add", &form_add_args(workspace))?;
        }
        seed_extension(workspace)?;
        let mut args = common_args(workspace);
        args.insert(
            "ExtensionPath".to_string(),
            Value::String("ext/Configuration.xml".to_string()),
        );
        args.insert(
            "ConfigPath".to_string(),
            Value::String("src/Configuration.xml".to_string()),
        );
        let object = if case.id.ends_with("managed-form") {
            "Catalog.CorpusCatalog.Form.CorpusForm"
        } else {
            "Catalog.CorpusCatalog"
        };
        args.insert("Object".to_string(), Value::String(object.to_string()));
        return Ok(args);
    }

    if case.id.starts_with("cfe-patch-method-") {
        let (object, module_path, base_module_path) = match case.branch {
            "CommonModule" => {
                seed_metadata(
                    workspace,
                    "seed-cfe-patch-common-module",
                    json!({
                        "type": "CommonModule",
                        "name": "CorpusModule",
                        "context": "server"
                    }),
                )?;
                (
                    "CommonModule.CorpusModule",
                    "CommonModule.CorpusModule",
                    "src/CommonModules/CorpusModule/Ext/Module.bsl",
                )
            }
            "Catalog.ObjectModule" => {
                seed_catalog(workspace)?;
                (
                    "Catalog.CorpusCatalog",
                    "Catalog.CorpusCatalog.ObjectModule",
                    "src/Catalogs/CorpusCatalog/Ext/ObjectModule.bsl",
                )
            }
            "Catalog.ManagerModule" => {
                seed_catalog(workspace)?;
                (
                    "Catalog.CorpusCatalog",
                    "Catalog.CorpusCatalog.ManagerModule",
                    "src/Catalogs/CorpusCatalog/Ext/ManagerModule.bsl",
                )
            }
            "InformationRegister.RecordSetModule" => {
                seed_metadata(
                    workspace,
                    "seed-cfe-patch-information-register",
                    meta_definition("InformationRegister")
                        .expect("InformationRegister seed definition"),
                )?;
                (
                    "InformationRegister.CorpusInformationRegister",
                    "InformationRegister.CorpusInformationRegister.RecordSetModule",
                    "src/InformationRegisters/CorpusInformationRegister/Ext/RecordSetModule.bsl",
                )
            }
            "Catalog.Form" => {
                seed_catalog_form(workspace)?;
                (
                    "Catalog.CorpusCatalog.Form.CorpusForm",
                    "Catalog.CorpusCatalog.Form.CorpusForm",
                    "src/Catalogs/CorpusCatalog/Forms/CorpusForm/Ext/Form/Module.bsl",
                )
            }
            "Constant.ValueManagerModule" => {
                seed_metadata(
                    workspace,
                    "seed-cfe-patch-constant",
                    meta_definition("Constant").expect("Constant seed definition"),
                )?;
                (
                    "Constant.CorpusConstant",
                    "Constant.CorpusConstant.ValueManagerModule",
                    "src/Constants/CorpusConstant/Ext/ValueManagerModule.bsl",
                )
            }
            other => {
                return Err(format!(
                    "unsupported cfe.patch_method corpus branch: {other}"
                ))
            }
        };
        fs::write(
            workspace.join(base_module_path),
            b"\xef\xbb\xbfProcedure Run()\r\nEndProcedure\r\n",
        )
        .map_err(|error| format!("cannot seed base cfe.patch_method BSL: {error}"))?;
        seed_extension(workspace)?;
        let mut borrow_args = common_args(workspace);
        borrow_args.insert(
            "ExtensionPath".to_string(),
            Value::String("ext/Configuration.xml".to_string()),
        );
        borrow_args.insert(
            "ConfigPath".to_string(),
            Value::String("src/Configuration.xml".to_string()),
        );
        borrow_args.insert("Object".to_string(), Value::String(object.to_string()));
        call_public_tool("unica.cfe.borrow", &borrow_args)?;
        let mut args = common_args(workspace);
        args.insert(
            "ExtensionPath".to_string(),
            Value::String("ext".to_string()),
        );
        args.insert(
            "ModulePath".to_string(),
            Value::String(module_path.to_string()),
        );
        args.insert("MethodName".to_string(), Value::String("Run".to_string()));
        args.insert(
            "InterceptorType".to_string(),
            Value::String("Before".to_string()),
        );
        return Ok(args);
    }

    if case.id == "code-patch-bsl-only" {
        seed_metadata(
            workspace,
            "code-module",
            json!({"type": "CommonModule", "name": "CorpusModule", "context": "server"}),
        )?;
        fs::write(
            workspace.join("src/CommonModules/CorpusModule/Ext/Module.bsl"),
            b"\xef\xbb\xbfProcedure Run()\r\n    Message(\"ok\");\r\nEndProcedure\r\n",
        )
        .map_err(|error| format!("cannot seed BSL module: {error}"))?;
        let mut args = common_args(workspace);
        args.insert("sourceDir".to_string(), Value::String("src".to_string()));
        args.insert(
            "path".to_string(),
            Value::String("src/CommonModules/CorpusModule/Ext/Module.bsl".to_string()),
        );
        args.insert("operation".to_string(), Value::String("insert".to_string()));
        args.insert("selector".to_string(), json!({"method": "Run"}));
        args.insert(
            "content".to_string(),
            Value::String("Procedure Added()\nEndProcedure".to_string()),
        );
        args.insert("position".to_string(), Value::String("after".to_string()));
        return Ok(args);
    }

    if case.id.starts_with("meta-compile-") {
        match case.branch {
            "Catalog" => seed_metadata(
                workspace,
                "seed-defined-type",
                json!({
                    "type": "DefinedType",
                    "name": "CorpusDefinedType",
                    "valueTypes": ["String(100)"]
                }),
            )?,
            "AccumulationRegister" => seed_document_recorder(
                workspace,
                "AccumulationRegister.CorpusAccumulationRegister",
            )?,
            "AccountingRegister" => {
                seed_metadata(
                    workspace,
                    "seed-chart-of-accounts",
                    json!({"type": "ChartOfAccounts", "name": "CorpusAccounts"}),
                )?;
                seed_document_recorder(workspace, "AccountingRegister.CorpusAccountingRegister")?;
            }
            "CalculationRegister" => {
                seed_metadata(
                    workspace,
                    "seed-chart-of-calculation-types",
                    json!({
                        "type": "ChartOfCalculationTypes",
                        "name": "CorpusCalculationTypes"
                    }),
                )?;
                seed_document_recorder(workspace, "CalculationRegister.CorpusCalculationRegister")?;
            }
            "BusinessProcess" => seed_metadata(
                workspace,
                "seed-task",
                json!({"type": "Task", "name": "CorpusTask"}),
            )?,
            "DocumentJournal" => seed_metadata(
                workspace,
                "seed-document",
                meta_definition("Document").expect("Document seed definition"),
            )?,
            "ScheduledJob" => seed_event_handlers(workspace)?,
            "EventSubscription" => {
                seed_metadata(
                    workspace,
                    "seed-document",
                    meta_definition("Document").expect("Document seed definition"),
                )?;
                seed_catalog(workspace)?;
                seed_event_handlers(workspace)?;
            }
            _ => {}
        }
        let definition = meta_definition(case.branch)
            .ok_or_else(|| format!("missing metadata definition for {}", case.branch))?;
        let path = write_json_input(workspace, case.id, &definition)?;
        return Ok(meta_compile_args(workspace, &path));
    }

    if matches!(case.id, "meta-edit-property" | "meta-remove-object") {
        seed_catalog(workspace)?;
        let mut args = common_args(workspace);
        if case.id == "meta-edit-property" {
            args.insert(
                "ObjectPath".to_string(),
                Value::String("src/Catalogs/CorpusCatalog.xml".to_string()),
            );
            args.insert(
                "Operation".to_string(),
                Value::String("modify-property".to_string()),
            );
            args.insert(
                "Value".to_string(),
                Value::String("Comment=Corpus edited".to_string()),
            );
        } else {
            args.insert("ConfigDir".to_string(), Value::String("src".to_string()));
            args.insert(
                "Object".to_string(),
                Value::String("Catalog.CorpusCatalog".to_string()),
            );
        }
        return Ok(args);
    }

    if case.id == "help-add-object" {
        seed_catalog(workspace)?;
        let mut args = common_args(workspace);
        args.insert(
            "ObjectName".to_string(),
            Value::String("Catalogs/CorpusCatalog".to_string()),
        );
        args.insert("SrcDir".to_string(), Value::String("src".to_string()));
        args.insert("Lang".to_string(), Value::String("ru".to_string()));
        return Ok(args);
    }

    if case.id == "form-add-managed" {
        seed_catalog(workspace)?;
        return Ok(form_add_args(workspace));
    }

    if case.id == "form-compile-managed" {
        seed_report_form(workspace)?;
        let mut args = common_args(workspace);
        let path = write_json_input(
            workspace,
            "form-compile",
            &json!({
                "title": "Corpus form",
                "properties": {
                    "autoTitle": false,
                    "width": 4_294_967_295_u64,
                    "height": 4_294_967_294_u64,
                    "scale": 98
                },
                "attributes": [{
                    "name": "Object",
                    "type": "ReportObject.CorpusReport",
                    "main": true
                }, {
                    "name": "Value",
                    "type": "String"
                }, {
                    "name": "Enabled",
                    "type": "Boolean"
                }, {
                    "name": "Picture",
                    "type": "Number"
                }, {
                    "name": "Rows",
                    "type": "ValueTable",
                    "columns": [{
                        "name": "Value",
                        "type": "String"
                    }, {
                        "name": "Presentation",
                        "type": "String"
                    }, {
                        "name": "Picture",
                        "type": "Number"
                    }]
                }],
                "commands": [{
                    "name": "CorpusAction",
                    "title": "Corpus action"
                }],
                "elements": [{
                    "name": "Header",
                    "group": "AlwaysHorizontal",
                    "behavior": "PopUp",
                    "currentRowUse": "DontUse",
                    "titleDataPath": "Value"
                }, {
                    "input": "Description",
                    "path": "Rows",
                    "titleLocation": "Top",
                    "footerDataPath": "Value",
                    "multipleValueDataPath": "Rows.Value",
                    "multipleValuePresentDataPath": "Rows.Presentation",
                    "multipleValuePictureDataPath": "Rows.Picture",
                    "width": 4_294_967_295_u64,
                    "height": 4_294_967_294_u64
                }, {
                    "check": "Enabled",
                    "path": "Enabled",
                    "checkBoxType": "Tumbler",
                    "titleLocation": "Bottom"
                }, {
                    "button": "CorpusActionButton",
                    "type": "CommandBarButton",
                    "command": "CorpusAction",
                    "representation": "Text",
                    "locationInCommandBar": "InCommandBar"
                }, {
                    "table": "Rows",
                    "path": "Rows",
                    "rowPictureDataPath": "Rows.Picture"
                }]
            }),
        )?;
        args.insert("JsonPath".to_string(), Value::String(path));
        args.insert(
            "OutputPath".to_string(),
            Value::String("src/Reports/CorpusReport/Forms/CorpusForm/Ext/Form.xml".to_string()),
        );
        return Ok(args);
    }

    if matches!(case.id, "form-edit-managed" | "form-remove-managed") {
        seed_catalog_form(workspace)?;
        let mut args = common_args(workspace);
        if case.id == "form-edit-managed" {
            args.insert(
                "FormPath".to_string(),
                Value::String(
                    "src/Catalogs/CorpusCatalog/Forms/CorpusForm/Ext/Form.xml".to_string(),
                ),
            );
            args.insert(
                "definition".to_string(),
                json!({"attributes": [{"name": "CorpusAdded", "type": "string"}]}),
            );
        } else {
            args.insert(
                "ObjectName".to_string(),
                Value::String("CorpusCatalog".to_string()),
            );
            args.insert(
                "FormName".to_string(),
                Value::String("CorpusForm".to_string()),
            );
            args.insert(
                "SrcDir".to_string(),
                Value::String("src/Catalogs".to_string()),
            );
        }
        return Ok(args);
    }

    if case.id == "interface-edit-subsystem" {
        seed_parent_subsystem(workspace)?;
        seed_catalog(workspace)?;
        let mut args = common_args(workspace);
        args.insert(
            "CIPath".to_string(),
            Value::String("src/Subsystems/CorpusParent/Ext/CommandInterface.xml".to_string()),
        );
        args.insert("Operation".to_string(), Value::String("hide".to_string()));
        args.insert(
            "Value".to_string(),
            Value::String("Catalog.CorpusCatalog.StandardCommand.OpenList".to_string()),
        );
        args.insert("CreateIfMissing".to_string(), Value::Bool(true));
        return Ok(args);
    }

    if case.id == "subsystem-compile-child" {
        seed_parent_subsystem(workspace)?;
        return Ok(subsystem_compile_args(
            workspace,
            "CorpusChild",
            Some("src/Subsystems/CorpusParent.xml"),
        ));
    }

    if case.id == "subsystem-edit-add-child" {
        seed_parent_subsystem(workspace)?;
        let mut args = common_args(workspace);
        args.insert(
            "SubsystemPath".to_string(),
            Value::String("src/Subsystems/CorpusParent.xml".to_string()),
        );
        args.insert(
            "Operation".to_string(),
            Value::String("add-child".to_string()),
        );
        args.insert(
            "Value".to_string(),
            Value::String("CorpusEditedChild".to_string()),
        );
        return Ok(args);
    }

    if case.id.starts_with("template-add-") {
        seed_report(workspace)?;
        let template_type = match case.branch {
            "SpreadsheetDocument" => "SpreadsheetDocument",
            "DataCompositionSchema" => "DataCompositionSchema",
            "TextDocument" => "Text",
            "HTMLDocument" => "HTML",
            "BinaryData" => "BinaryData",
            other => return Err(format!("unknown template branch: {other}")),
        };
        return Ok(template_args(workspace, "CorpusTemplate", template_type));
    }

    if case.id == "template-remove-object-template" {
        seed_report(workspace)?;
        call_public_tool(
            "unica.template.add",
            &template_args(workspace, "CorpusTemplate", "SpreadsheetDocument"),
        )?;
        let mut args = common_args(workspace);
        args.insert(
            "ObjectName".to_string(),
            Value::String("CorpusReport".to_string()),
        );
        args.insert(
            "TemplateName".to_string(),
            Value::String("CorpusTemplate".to_string()),
        );
        args.insert(
            "SrcDir".to_string(),
            Value::String("src/Reports".to_string()),
        );
        return Ok(args);
    }

    if matches!(
        case.id,
        "dcs-edit-owned-template"
            | "dcs-edit-add-parameter-after-settings"
            | "dcs-edit-set-structure-after-settings"
            | "dcs-edit-modify-field-role-restriction"
    ) {
        seed_report(workspace)?;
        seed_catalog(workspace)?;
        seed_metadata(
            workspace,
            "seed-unicode-defined-type",
            json!({
                "type": "DefinedType",
                "name": "ТипКорпуса",
                "valueTypes": ["String(16)"]
            }),
        )?;
        seed_dcs_template(workspace)?;
        let mut args = common_args(workspace);
        args.insert(
            "TemplatePath".to_string(),
            Value::String(
                "src/Reports/CorpusReport/Templates/CorpusDcs/Ext/Template.xml".to_string(),
            ),
        );
        let (operation, value, data_set) = match case.id {
            "dcs-edit-owned-template" => ("add-field", "Added:String", Some("Main")),
            "dcs-edit-add-parameter-after-settings" => (
                "add-parameter",
                "ПериодРедактирования [Edit period]: StandardPeriod = LastMonth @autoDates @hidden @always",
                None,
            ),
            "dcs-edit-set-structure-after-settings" => {
                ("set-structure", "Value @name=Edited > details", None)
            }
            "dcs-edit-modify-field-role-restriction" => (
                "modify-field",
                "Description [Updated]:String @required @dimension @required #noOrder #noField #noOrder",
                Some("CatalogObject"),
            ),
            _ => unreachable!(),
        };
        args.insert(
            "Operation".to_string(),
            Value::String(operation.to_string()),
        );
        args.insert("Value".to_string(), Value::String(value.to_string()));
        if let Some(data_set) = data_set {
            args.insert("DataSet".to_string(), Value::String(data_set.to_string()));
        }
        return Ok(args);
    }

    if case.id == "role-compile-name-field" {
        seed_catalog(workspace)?;
        let path = write_json_input(
            workspace,
            "role",
            &json!({
                "name": "CorpusReader",
                "synonym": "Corpus reader",
                "objects": ["Catalog.CorpusCatalog: @view"]
            }),
        )?;
        let mut args = common_args(workspace);
        args.insert("JsonPath".to_string(), Value::String(path));
        args.insert("OutputDir".to_string(), Value::String("src".to_string()));
        return Ok(args);
    }

    Err(format!("no executable preparation for case {}", case.id))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CorpusManifest {
    schema_version: u32,
    profile: String,
    empty_directory_paths: Vec<String>,
    cases: Vec<CorpusCase>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CorpusCase {
    id: String,
    workspace_path: String,
    pre_snapshot_path: String,
    platform_checkpoint: PlatformCheckpoint,
    checkpoint: String,
    tool_id: String,
    operation: String,
    branch: String,
    impact_class: String,
    xml_impact: String,
    pre_files: Vec<PreCorpusFile>,
    files: Vec<CorpusFile>,
    removed_paths: Vec<String>,
    pre_non_xml_files: Vec<PreNonXmlFile>,
    non_xml_files: Vec<NonXmlFile>,
    removed_non_xml_paths: Vec<String>,
    auxiliary_files: Vec<PreNonXmlFile>,
    pre_owner_versions: BTreeMap<String, String>,
    owner_versions: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PlatformCheckpoint {
    kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    source_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    base_source_path: Option<String>,
    covered_case_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CorpusFile {
    path: String,
    sha256: String,
    family: String,
    seed: bool,
    delta: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    owner_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    new_standalone: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PreCorpusFile {
    path: String,
    sha256: String,
    family: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    owner_path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PreNonXmlFile {
    path: String,
    sha256: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct NonXmlFile {
    path: String,
    sha256: String,
    seed: bool,
    delta: String,
}

struct PreCorpusContract {
    files: Vec<PreCorpusFile>,
    owner_versions: BTreeMap<String, String>,
}

struct NonXmlContract {
    pre_files: Vec<PreNonXmlFile>,
    files: Vec<NonXmlFile>,
    removed_paths: Vec<String>,
}

struct CaseFileContracts {
    pre_xml: PreCorpusContract,
    non_xml: NonXmlContract,
    auxiliary_files: Vec<PreNonXmlFile>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct CaseReport {
    schema_version: u32,
    profile: String,
    id: String,
    workspace_path: String,
    pre_snapshot_path: String,
    platform_checkpoint: PlatformCheckpoint,
    tool_id: String,
    operation: String,
    branch: String,
    impact_class: String,
    public_arguments: Value,
    target_call: TargetCallReport,
    pre_files: Vec<PreCorpusFile>,
    pre_non_xml_files: Vec<PreNonXmlFile>,
    non_xml_files: Vec<NonXmlFile>,
    removed_non_xml_paths: Vec<String>,
    auxiliary_files: Vec<PreNonXmlFile>,
    seed_outputs: Vec<String>,
    pre_xml_sha256: XmlSnapshot,
    post_xml_sha256: XmlSnapshot,
    delta: XmlDelta,
    remaining_xml: Vec<String>,
    removed_paths: Vec<RemovedPathReport>,
    owner_links: BTreeMap<String, String>,
    pre_owner_versions: BTreeMap<String, String>,
    owner_versions: BTreeMap<String, String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct TargetCallReport {
    sequence: usize,
    result_ok: bool,
    errors: Vec<String>,
    summary: String,
}

#[derive(Debug, Serialize)]
struct RemovedPathReport {
    path: String,
    sha256: String,
}

fn impact_class_name(impact: XmlImpactClass) -> &'static str {
    match impact {
        XmlImpactClass::None => "None",
        XmlImpactClass::CreateOrModify => "CreateOrModify",
        XmlImpactClass::RemoveOrModify => "RemoveOrModify",
    }
}

fn xml_impact_name(delta: &XmlDelta) -> &'static str {
    if !delta.removed.is_empty() {
        "removed"
    } else if !delta.created.is_empty() {
        "created"
    } else if !delta.modified.is_empty() {
        "modified"
    } else {
        "unchanged"
    }
}

fn xml_root_details_payload(
    payload: &[u8],
    label: &str,
) -> Result<(String, String, Option<String>, Option<String>), String> {
    let text = std::str::from_utf8(payload)
        .map_err(|error| format!("cannot decode XML {label}: {error}"))?;
    let source = text.trim_start_matches('\u{feff}');
    let document =
        Document::parse(source).map_err(|error| format!("cannot parse XML {label}: {error}"))?;
    let root = document.root_element();
    let namespace = root.tag_name().namespace().unwrap_or("").to_string();
    let local_name = root.tag_name().name().to_string();
    let child_type = root
        .children()
        .find(|child| child.is_element())
        .map(|child| child.tag_name().name().to_string());
    let version = root
        .attributes()
        .find(|attribute| attribute.namespace().is_none() && attribute.name() == "version")
        .and_then(|attribute| source.get(attribute.range_value()))
        .map(str::to_string);
    Ok((namespace, local_name, child_type, version))
}

fn family_for_xml_payload(payload: &[u8], label: &str) -> Result<String, String> {
    let (namespace, local_name, _, _) = xml_root_details_payload(payload, label)?;
    family_for_root(&namespace, &local_name, label)
}

fn family_for_root(namespace: &str, local_name: &str, label: &str) -> Result<String, String> {
    let family = match (namespace, local_name) {
        ("http://v8.1c.ru/8.3/MDClasses", "MetaDataObject") => "metadata",
        ("http://v8.1c.ru/8.3/xcf/scheme", "GraphicalSchema") => "flowchart",
        ("http://v8.1c.ru/8.3/xcf/logform", "Form") => "managed-form",
        ("http://v8.1c.ru/8.3/xcf/extrnprops", "CommandInterface") => "command-interface",
        ("http://v8.1c.ru/8.3/xcf/extrnprops", "Help") => "help",
        ("http://v8.1c.ru/8.3/xcf/extrnprops", "ExchangePlanContent") => "exchange-plan-content",
        ("http://v8.1c.ru/8.3/xcf/extrnprops", "HomePageWorkArea") => "home-page-work-area",
        ("http://v8.1c.ru/8.1/data-composition-system/schema", "DataCompositionSchema") => "dcs",
        ("http://v8.1c.ru/8.2/data/spreadsheet", "document") => "mxl",
        ("http://v8.1c.ru/8.2/managed-application/core", "ClientApplicationInterface") => {
            "client-application-interface"
        }
        ("http://v8.1c.ru/8.2/roles", "Rights") => "roles",
        _ => {
            return Err(format!(
                "unclassified XML root {{{namespace}}}{local_name}: {}",
                label
            ));
        }
    };
    Ok(family.to_string())
}

fn source_set_owner_roots_from_payloads(
    payloads: &XmlPayloadSnapshot,
) -> Result<BTreeMap<String, String>, String> {
    let allowed = [
        "Configuration",
        "ConfigurationExtension",
        "ExternalDataProcessor",
        "ExternalReport",
    ];
    let mut owners = BTreeMap::new();
    for (relative, payload) in payloads {
        let (namespace, local_name, child_type, _) = xml_root_details_payload(payload, relative)?;
        if namespace == "http://v8.1c.ru/8.3/MDClasses"
            && local_name == "MetaDataObject"
            && child_type
                .as_deref()
                .is_some_and(|kind| allowed.contains(&kind))
        {
            let root = Path::new(relative)
                .parent()
                .unwrap_or_else(|| Path::new(""))
                .to_string_lossy()
                .replace('\\', "/");
            owners.insert(relative.clone(), root);
        }
    }
    Ok(owners)
}

fn unique_deepest_owner_for_path<'a>(
    relative: &str,
    owners: &'a BTreeMap<String, String>,
) -> Result<Option<&'a str>, String> {
    let candidates = owners
        .iter()
        .filter(|(_, root)| {
            root.is_empty()
                || relative == root.as_str()
                || relative.starts_with(&format!("{root}/"))
        })
        .collect::<Vec<_>>();
    let Some(deepest) = candidates
        .iter()
        .map(|(_, root)| Path::new(root).components().count())
        .max()
    else {
        return Ok(None);
    };
    let deepest_owners = candidates
        .into_iter()
        .filter(|(_, root)| Path::new(root).components().count() == deepest)
        .map(|(owner, _)| owner.as_str())
        .collect::<Vec<_>>();
    if deepest_owners.len() != 1 {
        return Err(format!(
            "versionless XML has no unique deepest source-set owner: {relative}: {deepest_owners:?}"
        ));
    }
    Ok(deepest_owners.first().copied())
}

fn manifest_case_prefix(case_id: &str) -> String {
    format!("cases/{case_id}/workspace")
}

fn pre_snapshot_prefix(case_id: &str) -> String {
    format!("cases/{case_id}/pre-xml")
}

fn pre_non_xml_snapshot_prefix(case_id: &str) -> String {
    format!("cases/{case_id}/pre-non-xml")
}

fn manifest_path(case_id: &str, relative: &str) -> String {
    format!("{}/{relative}", manifest_case_prefix(case_id))
}

fn platform_checkpoint_for_case(case: &ExecutableCase) -> PlatformCheckpoint {
    let prefix = manifest_case_prefix(case.id);
    let (kind, source_path, base_source_path) = if case.id.starts_with("cfe-") {
        (
            "extension",
            Some(format!("{prefix}/ext")),
            Some(format!("{prefix}/src")),
        )
    } else if case.id.starts_with("epf-") {
        ("epf", Some(format!("{prefix}/epf")), None)
    } else if case.id.starts_with("erf-") {
        ("erf", Some(format!("{prefix}/erf")), None)
    } else {
        ("configuration", Some(format!("{prefix}/src")), None)
    };
    PlatformCheckpoint {
        kind: kind.to_string(),
        source_path,
        base_source_path,
        covered_case_ids: vec![case.id.to_string()],
    }
}

fn platform_source_root_relatives(
    case: &ExecutableCase,
) -> Result<&'static [&'static str], String> {
    let checkpoint = platform_checkpoint_for_case(case);
    match checkpoint.kind.as_str() {
        "configuration" => Ok(&["src"]),
        "extension" => Ok(&["src", "ext"]),
        "epf" => Ok(&["epf"]),
        "erf" => Ok(&["erf"]),
        other => Err(format!(
            "unknown platform checkpoint kind for {}: {other}",
            case.id
        )),
    }
}

fn prefixed_path(prefix: &str, relative: &str) -> String {
    format!("{prefix}/{relative}")
}

fn owner_versions_from_payloads(
    prefix: &str,
    payloads: &XmlPayloadSnapshot,
    owners: &BTreeMap<String, String>,
) -> Result<BTreeMap<String, String>, String> {
    let mut versions = BTreeMap::new();
    for owner in owners.keys() {
        let payload = payloads
            .get(owner)
            .ok_or_else(|| format!("pre-snapshot owner payload is absent: {owner}"))?;
        let (_, _, _, version) = xml_root_details_payload(payload, owner)?;
        let version = version
            .ok_or_else(|| format!("pre-snapshot source-set owner has no version: {owner}"))?;
        if version != "2.20" {
            return Err(format!(
                "pre-snapshot source-set owner is not format 2.20: {owner}: {version}"
            ));
        }
        versions.insert(prefixed_path(prefix, owner), version);
    }
    Ok(versions)
}

fn build_pre_contract(
    case: &ExecutableCase,
    pre_payloads: &XmlPayloadSnapshot,
) -> Result<PreCorpusContract, String> {
    let prefix = pre_snapshot_prefix(case.id);
    let owners = source_set_owner_roots_from_payloads(pre_payloads)?;
    let pre_owner_versions = owner_versions_from_payloads(&prefix, pre_payloads, &owners)?;
    let mut pre_files = Vec::new();
    for (relative, payload) in pre_payloads {
        let family = family_for_xml_payload(payload, relative)?;
        let (_, _, _, version) = xml_root_details_payload(payload, relative)?;
        let owner = if version.is_none() {
            unique_deepest_owner_for_path(relative, &owners)?
        } else {
            None
        };
        match version.as_deref() {
            Some("2.20") if owner.is_none() => {}
            Some("2.20") => {
                return Err(format!(
                    "version-bearing pre-snapshot XML must not declare an owner: {relative}"
                ));
            }
            Some(other) => {
                return Err(format!(
                    "pre-snapshot version-bearing XML is not format 2.20: {relative}: {other}"
                ));
            }
            None if owner.is_some() => {}
            None => {
                return Err(format!(
                    "versionless pre-snapshot XML has no same-case owner: {relative}"
                ));
            }
        }
        pre_files.push(PreCorpusFile {
            path: prefixed_path(&prefix, relative),
            sha256: format!("{:x}", Sha256::digest(payload)),
            family,
            owner_path: owner.map(|owner| prefixed_path(&prefix, owner)),
        });
    }
    pre_files.sort_by(|left, right| left.path.cmp(&right.path));
    Ok(PreCorpusContract {
        files: pre_files,
        owner_versions: pre_owner_versions,
    })
}

fn ensure_all_xml_listed(actual: &XmlSnapshot, listed: &BTreeSet<String>) -> Result<(), String> {
    let actual_paths = actual.keys().cloned().collect::<BTreeSet<_>>();
    if actual_paths != *listed {
        let unlisted = actual_paths
            .difference(listed)
            .next()
            .cloned()
            .unwrap_or_default();
        let stale = listed
            .difference(&actual_paths)
            .next()
            .cloned()
            .unwrap_or_default();
        return Err(format!(
            "manifest XML listing mismatch: unlisted={unlisted:?}, stale={stale:?}"
        ));
    }
    Ok(())
}

fn delta_for_path<'a>(delta: &'a XmlDelta, path: &str) -> &'a str {
    if delta.created.iter().any(|item| item == path) {
        "created"
    } else if delta.modified.iter().any(|item| item == path) {
        "modified"
    } else if delta.unchanged.iter().any(|item| item == path) {
        "unchanged"
    } else {
        panic!("post-snapshot path absent from delta: {path}");
    }
}

fn build_non_xml_contract(
    case: &ExecutableCase,
    pre_payloads: &NonXmlPayloadSnapshot,
    post_payloads: &NonXmlPayloadSnapshot,
) -> Result<NonXmlContract, String> {
    let before = hashes_for_non_xml_payloads(pre_payloads);
    let after = hashes_for_non_xml_payloads(post_payloads);
    let delta = classify_xml_delta(&before, &after);
    let mut pre_files = before
        .iter()
        .map(|(relative, sha256)| PreNonXmlFile {
            path: format!("{}/{relative}", pre_non_xml_snapshot_prefix(case.id)),
            sha256: sha256.clone(),
        })
        .collect::<Vec<_>>();
    let mut files = after
        .iter()
        .map(|(relative, sha256)| NonXmlFile {
            path: manifest_path(case.id, relative),
            sha256: sha256.clone(),
            seed: before.contains_key(relative),
            delta: delta_for_path(&delta, relative).to_string(),
        })
        .collect::<Vec<_>>();
    let mut removed_paths = delta
        .removed
        .iter()
        .map(|relative| manifest_path(case.id, relative))
        .collect::<Vec<_>>();
    pre_files.sort_by(|left, right| left.path.cmp(&right.path));
    files.sort_by(|left, right| left.path.cmp(&right.path));
    removed_paths.sort();

    let prefix = format!("{}/", manifest_case_prefix(case.id));
    let pre_prefix = format!("{}/", pre_non_xml_snapshot_prefix(case.id));
    let listed_post = files
        .iter()
        .map(|file| {
            file.path
                .strip_prefix(&prefix)
                .expect("non-XML case manifest prefix")
                .to_string()
        })
        .collect::<BTreeSet<_>>();
    let actual_post = after.keys().cloned().collect::<BTreeSet<_>>();
    if listed_post != actual_post {
        return Err(format!("manifest non-XML listing mismatch for {}", case.id));
    }
    let listed_pre = pre_files
        .iter()
        .map(|file| {
            file.path
                .strip_prefix(&pre_prefix)
                .expect("pre non-XML case manifest prefix")
                .to_string()
        })
        .collect::<BTreeSet<_>>();
    let actual_pre = before.keys().cloned().collect::<BTreeSet<_>>();
    if listed_pre != actual_pre {
        return Err(format!(
            "manifest pre non-XML listing mismatch for {}",
            case.id
        ));
    }

    Ok(NonXmlContract {
        pre_files,
        files,
        removed_paths,
    })
}

fn build_corpus_case(
    case: &ExecutableCase,
    post_payloads: &XmlPayloadSnapshot,
    entry: &MutatorRegistryEntry,
    contracts: CaseFileContracts,
    before: &XmlSnapshot,
    after: &XmlSnapshot,
    delta: &XmlDelta,
) -> Result<CorpusCase, String> {
    let CaseFileContracts {
        pre_xml,
        non_xml,
        auxiliary_files,
    } = contracts;
    let owners = source_set_owner_roots_from_payloads(post_payloads)?;
    let workspace_prefix = manifest_case_prefix(case.id);
    let owner_versions = owner_versions_from_payloads(&workspace_prefix, post_payloads, &owners)?;
    let mut files = Vec::new();
    for (relative, hash) in after {
        let payload = post_payloads
            .get(relative)
            .ok_or_else(|| format!("post-snapshot payload is absent: {relative}"))?;
        let family = family_for_xml_payload(payload, relative)?;
        let versionless = matches!(
            family.as_str(),
            "dcs" | "mxl" | "client-application-interface"
        );
        let owner = if versionless {
            unique_deepest_owner_for_path(relative, &owners)?
        } else {
            None
        };
        let new_standalone = (versionless && owner.is_none())
            .then_some(delta.created.iter().any(|item| item == relative));
        if versionless && owner.is_none() && new_standalone != Some(true) {
            return Err(format!(
                "versionless XML has neither same-case owner nor new standalone evidence: {relative}"
            ));
        }
        files.push(CorpusFile {
            path: manifest_path(case.id, relative),
            sha256: hash.clone(),
            family,
            seed: before.contains_key(relative),
            delta: delta_for_path(delta, relative).to_string(),
            owner_path: owner.map(|owner| manifest_path(case.id, owner)),
            new_standalone,
        });
    }
    files.sort_by(|left, right| left.path.cmp(&right.path));
    ensure_all_xml_listed(
        after,
        &files
            .iter()
            .map(|file| {
                file.path
                    .strip_prefix(&format!("{}/", manifest_case_prefix(case.id)))
                    .expect("case manifest prefix")
                    .to_string()
            })
            .collect(),
    )?;

    let mut removed_paths = delta
        .removed
        .iter()
        .map(|path| manifest_path(case.id, path))
        .collect::<Vec<_>>();
    removed_paths.sort();
    Ok(CorpusCase {
        id: case.id.to_string(),
        workspace_path: workspace_prefix,
        pre_snapshot_path: pre_snapshot_prefix(case.id),
        platform_checkpoint: platform_checkpoint_for_case(case),
        checkpoint: format!("cases/{}/case-report.json", case.id),
        tool_id: case.tool.to_string(),
        operation: entry.operation.to_string(),
        branch: case.branch.to_string(),
        impact_class: impact_class_name(effective_xml_impact(case.id)).to_string(),
        xml_impact: xml_impact_name(delta).to_string(),
        pre_files: pre_xml.files,
        files,
        removed_paths,
        pre_non_xml_files: non_xml.pre_files,
        non_xml_files: non_xml.files,
        removed_non_xml_paths: non_xml.removed_paths,
        auxiliary_files,
        pre_owner_versions: pre_xml.owner_versions,
        owner_versions,
    })
}

fn sanitize_value(value: &Value, workspace: &Path) -> Value {
    match value {
        Value::String(text) => {
            Value::String(text.replace(&workspace.display().to_string(), "$CASE_WORKSPACE"))
        }
        Value::Array(items) => Value::Array(
            items
                .iter()
                .map(|item| sanitize_value(item, workspace))
                .collect(),
        ),
        Value::Object(object) => Value::Object(
            object
                .iter()
                .map(|(key, value)| (key.clone(), sanitize_value(value, workspace)))
                .collect(),
        ),
        other => other.clone(),
    }
}

fn assert_case_postconditions(
    case: &ExecutableCase,
    workspace: &Path,
    before: &XmlSnapshot,
    after: &XmlSnapshot,
    delta: &XmlDelta,
) -> Result<(), String> {
    if matches!(effective_xml_impact(case.id), XmlImpactClass::None) {
        if before.is_empty() {
            return Err(format!("None-impact case has no seed XML: {}", case.id));
        }
        if before != after {
            return Err(format!("None-impact case changed XML: {}", case.id));
        }
    }
    if case.id == "meta-compile-business-process"
        && !after
            .keys()
            .any(|path| path.ends_with("/Ext/Flowchart.xml"))
    {
        return Err("BusinessProcess case did not generate Flowchart.xml".to_string());
    }
    if case.id == "meta-compile-exchange-plan"
        && !after.keys().any(|path| path.ends_with("/Ext/Content.xml"))
    {
        return Err("ExchangePlan case did not generate Ext/Content.xml".to_string());
    }
    if matches!(
        case.branch,
        "SpreadsheetDocument" | "DataCompositionSchema" | "HTMLDocument"
    ) && case.id.starts_with("template-add-")
        && !after
            .keys()
            .any(|path| path.ends_with("/Templates/CorpusTemplate/Ext/Template.xml"))
    {
        return Err(format!("{} template content is not XML", case.branch));
    }
    if case.id.starts_with("template-add-") && matches!(case.branch, "TextDocument" | "BinaryData")
    {
        let extension = match case.branch {
            "TextDocument" => "txt",
            "BinaryData" => "bin",
            _ => unreachable!(),
        };
        let relative =
            format!("src/Reports/CorpusReport/Templates/CorpusTemplate/Ext/Template.{extension}");
        if !workspace.join(&relative).is_file() || after.contains_key(&relative) {
            return Err(format!(
                "{} non-XML template content was missing or classified as XML",
                case.branch
            ));
        }
    }
    if case.id == "template-add-html-document" {
        let relative = "src/Reports/CorpusReport/Templates/CorpusTemplate/Ext/Template/ru.html";
        if !workspace.join(relative).is_file() || after.contains_key(relative) {
            return Err(
                "HTMLDocument page-set content was missing or classified as XML".to_string(),
            );
        }
    }
    if matches!(
        effective_xml_impact(case.id),
        XmlImpactClass::RemoveOrModify
    ) && (delta.removed.is_empty() || delta.modified.is_empty())
    {
        return Err(format!(
            "removal case lacks removed path or surviving owner modification: {}",
            case.id
        ));
    }
    Ok(())
}

fn run_corpus_case(
    output: &Path,
    case: &ExecutableCase,
    gate: &mut SequentialCallGate,
) -> Result<CorpusCase, String> {
    let case_root = output.join("cases").join(case.id);
    let workspace = case_root.join("workspace");
    fs::create_dir_all(&workspace)
        .map_err(|error| format!("cannot create {}: {error}", workspace.display()))?;
    let args = prepare_target(case, &workspace)?;
    remove_internal_workspace_cache(&workspace)?;
    let pre_root = case_root.join("pre-xml");
    let pre_non_xml_root = case_root.join("pre-non-xml");
    let pre_payloads = capture_xml_payloads(&workspace)?;
    let pre_non_xml_payloads = capture_non_xml_payloads(case, &workspace)?;
    let pre_auxiliary_payloads = capture_auxiliary_payloads(case, &workspace)?;
    let before = hashes_for_xml_payloads(&pre_payloads);
    let pre_contract = build_pre_contract(case, &pre_payloads)?;
    let sequence = gate.completed_target_calls + 1;
    let summary = call_target_tool(gate, case.tool, &args)?;
    remove_internal_workspace_cache(&workspace)?;
    materialize_pre_xml(&pre_root, &pre_payloads)?;
    materialize_pre_non_xml(case, &pre_non_xml_root, &pre_non_xml_payloads)?;
    let post_payloads = capture_xml_payloads(&workspace)?;
    let post_non_xml_payloads = capture_non_xml_payloads(case, &workspace)?;
    let post_auxiliary_payloads = capture_auxiliary_payloads(case, &workspace)?;
    if post_auxiliary_payloads != pre_auxiliary_payloads {
        let before = hashes_for_non_xml_payloads(&pre_auxiliary_payloads);
        let after = hashes_for_non_xml_payloads(&post_auxiliary_payloads);
        let delta = classify_xml_delta(&before, &after);
        return Err(format!(
            "target call created, changed, or removed a file outside the platform checkpoint boundary: {}: created={:?}, modified={:?}, removed={:?}",
            case.id, delta.created, delta.modified, delta.removed
        ));
    }
    let after = hashes_for_xml_payloads(&post_payloads);
    let entry = registry_entry_for_case(case.id);
    let effective_impact = effective_xml_impact(case.id);
    let delta = enforce_xml_impact(effective_impact, &before, &after)?;
    assert_case_postconditions(case, &workspace, &before, &after, &delta)?;
    let contracts = CaseFileContracts {
        pre_xml: pre_contract,
        non_xml: build_non_xml_contract(case, &pre_non_xml_payloads, &post_non_xml_payloads)?,
        auxiliary_files: pre_auxiliary_payloads
            .iter()
            .map(|(relative, payload)| PreNonXmlFile {
                path: manifest_path(case.id, relative),
                sha256: format!("{:x}", Sha256::digest(payload)),
            })
            .collect(),
    };
    let manifest_case = build_corpus_case(
        case,
        &post_payloads,
        entry,
        contracts,
        &before,
        &after,
        &delta,
    )?;
    let seed_outputs = before.keys().cloned().collect::<Vec<_>>();
    let remaining_xml = after.keys().cloned().collect::<Vec<_>>();
    let removed_paths = delta
        .removed
        .iter()
        .map(|path| RemovedPathReport {
            path: path.clone(),
            sha256: before
                .get(path)
                .expect("removed XML must have a pre-snapshot hash")
                .clone(),
        })
        .collect::<Vec<_>>();
    let owner_links = manifest_case
        .files
        .iter()
        .filter_map(|file| {
            file.owner_path
                .as_ref()
                .map(|owner| (file.path.clone(), owner.clone()))
        })
        .collect::<BTreeMap<_, _>>();
    let report = CaseReport {
        schema_version: 1,
        profile: "1c-8.3.27-export-2.20".to_string(),
        id: case.id.to_string(),
        workspace_path: manifest_case.workspace_path.clone(),
        pre_snapshot_path: manifest_case.pre_snapshot_path.clone(),
        platform_checkpoint: manifest_case.platform_checkpoint.clone(),
        tool_id: case.tool.to_string(),
        operation: entry.operation.to_string(),
        branch: case.branch.to_string(),
        impact_class: impact_class_name(effective_impact).to_string(),
        public_arguments: sanitize_value(&Value::Object(args), &workspace),
        target_call: TargetCallReport {
            sequence,
            result_ok: true,
            errors: Vec::new(),
            summary: summary.replace(&workspace.display().to_string(), "$CASE_WORKSPACE"),
        },
        pre_files: manifest_case.pre_files.clone(),
        pre_non_xml_files: manifest_case.pre_non_xml_files.clone(),
        non_xml_files: manifest_case.non_xml_files.clone(),
        removed_non_xml_paths: manifest_case.removed_non_xml_paths.clone(),
        auxiliary_files: manifest_case.auxiliary_files.clone(),
        seed_outputs,
        pre_xml_sha256: before,
        post_xml_sha256: after,
        delta,
        remaining_xml,
        removed_paths,
        owner_links,
        pre_owner_versions: manifest_case.pre_owner_versions.clone(),
        owner_versions: manifest_case.owner_versions.clone(),
    };
    fs::write(
        case_root.join("case-report.json"),
        serde_json::to_vec_pretty(&report)
            .map_err(|error| format!("cannot serialize case report: {error}"))?,
    )
    .map_err(|error| format!("cannot write case report: {error}"))?;
    require_xml_payloads_unchanged(&workspace, &post_payloads, "post-workspace")?;
    require_xml_payloads_unchanged(&pre_root, &pre_payloads, "materialized pre-snapshot")?;
    require_non_xml_payloads_unchanged(case, &workspace, &post_non_xml_payloads, "post-workspace")?;
    require_non_xml_payloads_unchanged(
        case,
        &pre_non_xml_root,
        &pre_non_xml_payloads,
        "materialized pre non-XML snapshot",
    )?;
    Ok(manifest_case)
}

fn sort_manifest(manifest: &mut CorpusManifest) {
    manifest.empty_directory_paths.sort();
    for case in &mut manifest.cases {
        case.pre_files
            .sort_by(|left, right| left.path.cmp(&right.path));
        case.files.sort_by(|left, right| left.path.cmp(&right.path));
        case.removed_paths.sort();
        case.pre_non_xml_files
            .sort_by(|left, right| left.path.cmp(&right.path));
        case.non_xml_files
            .sort_by(|left, right| left.path.cmp(&right.path));
        case.removed_non_xml_paths.sort();
        case.auxiliary_files
            .sort_by(|left, right| left.path.cmp(&right.path));
    }
    manifest.cases.sort_by(|left, right| left.id.cmp(&right.id));
}

fn generate_corpus(output: &Path) -> Result<CorpusManifest, String> {
    if !output.exists() {
        fs::create_dir(output).map_err(|error| {
            format!("cannot create corpus target {}: {error}", output.display())
        })?;
    }
    let mut cases = EXECUTABLE_CASES.iter().collect::<Vec<_>>();
    cases.sort_by_key(|case| case.id);
    let mut gate = SequentialCallGate::default();
    let mut generated = Vec::new();
    for case in cases {
        generated.push(run_corpus_case(output, case, &mut gate)?);
    }
    if gate.completed_target_calls != EXECUTABLE_CASES.len() {
        return Err(format!(
            "sequential target call count mismatch: {} != {}",
            gate.completed_target_calls,
            EXECUTABLE_CASES.len()
        ));
    }
    let mut manifest = CorpusManifest {
        schema_version: 2,
        profile: "1c-8.3.27-export-2.20".to_string(),
        empty_directory_paths: capture_empty_directory_paths(output)?,
        cases: generated,
    };
    sort_manifest(&mut manifest);
    fs::write(
        output.join("corpus-manifest.json"),
        serde_json::to_vec_pretty(&manifest)
            .map_err(|error| format!("cannot serialize corpus manifest: {error}"))?,
    )
    .map_err(|error| format!("cannot write corpus manifest: {error}"))?;
    Ok(manifest)
}

fn normalize_absolute_path(path: &Path) -> Result<PathBuf, String> {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .map_err(|error| format!("cannot resolve current directory: {error}"))?
            .join(path)
    };
    let mut normalized = PathBuf::new();
    for component in absolute.components() {
        match component {
            Component::Prefix(prefix) => normalized.push(prefix.as_os_str()),
            Component::RootDir => normalized.push(Path::new("/")),
            Component::CurDir => {}
            Component::ParentDir => {
                if !normalized.pop() {
                    return Err("output path escapes filesystem root".to_string());
                }
            }
            Component::Normal(part) => normalized.push(part),
        }
    }
    Ok(normalized)
}

fn canonical_or_parent(path: &Path) -> Result<PathBuf, String> {
    if path.exists() {
        return path
            .canonicalize()
            .map_err(|error| format!("cannot canonicalize {}: {error}", path.display()));
    }
    let parent = path
        .parent()
        .ok_or_else(|| "output target has no parent directory".to_string())?;
    if !parent.is_dir() {
        return Err(format!(
            "output target parent does not exist: {}",
            parent.display()
        ));
    }
    let canonical_parent = parent
        .canonicalize()
        .map_err(|error| format!("cannot canonicalize {}: {error}", parent.display()))?;
    let name = path
        .file_name()
        .ok_or_else(|| "output target has no final path component".to_string())?;
    Ok(canonical_parent.join(name))
}

fn validate_output_directory(
    raw: &str,
    repo_root: &Path,
    home_root: &Path,
) -> Result<PathBuf, String> {
    if raw.trim().is_empty() {
        return Err("UNICA_XML_CORPUS_DIR is empty".to_string());
    }
    let normalized = normalize_absolute_path(Path::new(raw.trim()))?;
    if fs::symlink_metadata(&normalized).is_ok_and(|metadata| metadata.file_type().is_symlink()) {
        return Err("corpus output target must not be a symlink".to_string());
    }
    let target = canonical_or_parent(&normalized)?;
    let filesystem_root = Path::new("/")
        .canonicalize()
        .map_err(|error| format!("cannot canonicalize filesystem root: {error}"))?;
    let home = home_root
        .canonicalize()
        .map_err(|error| format!("cannot canonicalize home root: {error}"))?;
    let repo = repo_root
        .canonicalize()
        .map_err(|error| format!("cannot canonicalize repository root: {error}"))?;
    if target == filesystem_root {
        return Err("refusing filesystem root as corpus output".to_string());
    }
    if target == home {
        return Err("refusing home root as corpus output".to_string());
    }
    if target == repo {
        return Err("refusing repository root as corpus output".to_string());
    }
    if target.exists() {
        if !target.is_dir() {
            return Err("corpus output target exists and is not a directory".to_string());
        }
        let mut entries = fs::read_dir(&target)
            .map_err(|error| format!("cannot inspect corpus output: {error}"))?;
        if entries.next().is_some() {
            return Err("corpus output directory must be empty".to_string());
        }
    }
    Ok(target)
}

fn configured_output_directory_from(
    raw: Option<&str>,
    repo_root: &Path,
    home_root: &Path,
) -> Result<PathBuf, String> {
    let raw = raw.ok_or_else(|| "UNICA_XML_CORPUS_DIR is not set".to_string())?;
    validate_output_directory(raw, repo_root, home_root)
}

fn configured_output_directory() -> Result<PathBuf, String> {
    let raw = std::env::var("UNICA_XML_CORPUS_DIR").ok();
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let repo_root = manifest_dir
        .parent()
        .and_then(Path::parent)
        .ok_or_else(|| "cannot resolve repository root".to_string())?;
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .ok_or_else(|| "HOME is not set".to_string())?;
    configured_output_directory_from(raw.as_deref(), repo_root, &home)
}

fn live_public_native_mutators() -> BTreeMap<&'static str, &'static str> {
    UnicaApplication::new()
        .tools()
        .into_iter()
        .filter_map(|tool| {
            if !tool.mutating {
                return None;
            }
            let ToolHandler::NativeOperation { operation, .. } = tool.handler else {
                return None;
            };
            Some((tool.name, operation))
        })
        .collect()
}

#[test]
fn every_public_native_mutator_has_xml_impact_and_case_coverage() {
    let live = live_public_native_mutators();
    let mut registry = BTreeMap::new();
    let mut seen_operations = BTreeSet::new();
    for entry in MUTATOR_REGISTRY {
        assert!(
            registry.insert(entry.tool, entry.operation).is_none(),
            "duplicate registry tool: {}",
            entry.tool
        );
        assert!(
            seen_operations.insert(entry.operation),
            "duplicate registry operation: {}",
            entry.operation
        );
    }

    assert_eq!(registry, live);

    let mut executable = BTreeMap::new();
    for case in EXECUTABLE_CASES {
        assert!(
            executable.insert(case.id, case).is_none(),
            "duplicate executable case: {}",
            case.id
        );
    }

    let mut referenced_cases = BTreeSet::new();
    for entry in MUTATOR_REGISTRY {
        assert!(
            !entry.case_ids.is_empty(),
            "registry row has no case: {}",
            entry.tool
        );
        let mut actual_branches = BTreeSet::new();
        for case_id in entry.case_ids {
            assert!(
                referenced_cases.insert(*case_id),
                "case referenced by multiple rows: {case_id}"
            );
            let case = executable
                .get(case_id)
                .unwrap_or_else(|| panic!("missing executable case: {case_id}"));
            assert_eq!(case.tool, entry.tool, "case tool mismatch: {case_id}");
            actual_branches.insert(case.branch);
        }
        let required_branches = entry
            .required_branches
            .iter()
            .copied()
            .collect::<BTreeSet<_>>();
        assert!(
            !required_branches.is_empty(),
            "registry row has no branch contract: {}",
            entry.tool
        );
        assert_eq!(
            actual_branches, required_branches,
            "branch coverage mismatch"
        );
        let _ = entry.impact;
    }
    assert_eq!(
        referenced_cases,
        executable.keys().copied().collect(),
        "stale executable case"
    );
}

#[test]
fn xml_delta_classifies_exact_pre_post_impact() {
    let before = BTreeMap::from([
        ("modified.xml".to_string(), "old".to_string()),
        ("removed.xml".to_string(), "gone".to_string()),
        ("unchanged.xml".to_string(), "same".to_string()),
    ]);
    let after = BTreeMap::from([
        ("created.xml".to_string(), "new".to_string()),
        ("modified.xml".to_string(), "changed".to_string()),
        ("unchanged.xml".to_string(), "same".to_string()),
    ]);

    assert_eq!(
        classify_xml_delta(&before, &after),
        XmlDelta {
            created: vec!["created.xml".to_string()],
            modified: vec!["modified.xml".to_string()],
            removed: vec!["removed.xml".to_string()],
            unchanged: vec!["unchanged.xml".to_string()],
        }
    );
}

#[test]
fn non_xml_contract_records_removed_payloads_outside_post_inventory() {
    let case = EXECUTABLE_CASES
        .iter()
        .find(|case| case.id == "form-remove-managed")
        .unwrap();
    let before = NonXmlPayloadSnapshot::from([
        ("src/kept.bsl".to_string(), b"before".to_vec()),
        ("src/removed.bin".to_string(), b"removed".to_vec()),
    ]);
    let after = NonXmlPayloadSnapshot::from([("src/kept.bsl".to_string(), b"after".to_vec())]);

    let contract = build_non_xml_contract(case, &before, &after).unwrap();

    assert_eq!(contract.pre_files.len(), 2);
    assert_eq!(contract.files.len(), 1);
    assert!(contract.files[0].path.ends_with("src/kept.bsl"));
    assert_eq!(contract.files[0].delta, "modified");
    assert_eq!(
        contract.removed_paths,
        ["cases/form-remove-managed/workspace/src/removed.bin"]
    );
}

#[test]
fn remove_impact_requires_removal_and_surviving_owner_modification() {
    let before = BTreeMap::from([
        ("owner.xml".to_string(), "old".to_string()),
        ("removed.xml".to_string(), "gone".to_string()),
    ]);
    let after = BTreeMap::from([("owner.xml".to_string(), "new".to_string())]);

    let delta = enforce_xml_impact(XmlImpactClass::RemoveOrModify, &before, &after).unwrap();

    assert_eq!(delta.removed, ["removed.xml"]);
    assert_eq!(delta.modified, ["owner.xml"]);
}

#[test]
fn none_impact_rejects_any_xml_map_change() {
    let before = BTreeMap::from([("seed.xml".to_string(), "before".to_string())]);
    let after = BTreeMap::from([("seed.xml".to_string(), "after".to_string())]);

    let error = enforce_xml_impact(XmlImpactClass::None, &before, &after).unwrap_err();

    assert!(error.contains("complete XML map"), "{error}");
}

#[test]
fn sequential_execution_invariant_rejects_overlap() {
    let mut gate = SequentialCallGate::default();
    gate.begin().unwrap();

    let error = gate.begin().unwrap_err();

    assert!(error.contains("overlapping"), "{error}");
    gate.finish().unwrap();
    assert_eq!(gate.completed_target_calls, 1);
}

#[test]
fn every_executable_case_has_an_exact_platform_checkpoint() {
    for case in EXECUTABLE_CASES {
        let checkpoint = platform_checkpoint_for_case(case);
        assert_ne!(
            checkpoint.kind, "none",
            "{} must not be excluded from the exact platform gate",
            case.id
        );
        assert!(
            checkpoint.source_path.is_some(),
            "{} has no platform source path",
            case.id
        );
        if case.id.starts_with("cfe-") {
            assert_eq!(checkpoint.kind, "extension", "{}", case.id);
            assert!(
                checkpoint.base_source_path.is_some(),
                "{} has no base configuration source path",
                case.id
            );
        }
    }
}

#[test]
fn cf_init_public_case_creates_real_xml() {
    let root = unique_temp_dir("cf-init-public-case");
    let workspace = root.join("workspace");
    fs::create_dir_all(&workspace).unwrap();
    let before = snapshot_xml(&workspace).unwrap();
    let args = cf_init_args(&workspace, "CorpusConfiguration", "src");
    let mut gate = SequentialCallGate::default();

    call_target_tool(&mut gate, "unica.cf.init", &args).unwrap();

    let after = snapshot_xml(&workspace).unwrap();
    let entry = registry_entry_for_case("cf-init-default");
    let delta = enforce_xml_impact(entry.impact, &before, &after).unwrap();
    assert!(delta.created.contains(&"src/Configuration.xml".to_string()));
    assert_eq!(gate.completed_target_calls, 1);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn pre_snapshot_materializes_only_immutable_pre_call_bytes() {
    let root = unique_temp_dir("immutable-pre-snapshot");
    let workspace = root.join("workspace");
    let source = workspace.join("src/Configuration.xml");
    fs::create_dir_all(source.parent().unwrap()).unwrap();
    let before = br#"<MetaDataObject version="2.20"><Configuration/></MetaDataObject>"#;
    let after = br#"<MetaDataObject version="2.20"><Configuration><Changed/></Configuration></MetaDataObject>"#;
    fs::write(&source, before).unwrap();

    let captured = capture_xml_payloads(&workspace).unwrap();
    fs::write(&source, after).unwrap();
    let pre_root = root.join("pre-xml");
    materialize_pre_xml(&pre_root, &captured).unwrap();

    let copied = pre_root.join("src/Configuration.xml");
    assert_eq!(fs::read(&copied).unwrap(), before);
    assert_ne!(fs::read(&source).unwrap(), fs::read(&copied).unwrap());
    platform_support::assert_independent_copy(&source, &copied);

    let collision = root.join("target-created-pre-xml");
    fs::create_dir(&collision).unwrap();
    fs::write(collision.join("tampered.xml"), b"<tampered/>").unwrap();
    let error = materialize_pre_xml(&collision, &captured).unwrap_err();
    assert!(error.contains("cannot create"), "{error}");
    assert_eq!(
        fs::read(collision.join("tampered.xml")).unwrap(),
        b"<tampered/>"
    );

    let captured_post = capture_xml_payloads(&workspace).unwrap();
    fs::write(
        &source,
        br#"<MetaDataObject version="2.20"><Configuration><LateMutation/></Configuration></MetaDataObject>"#,
    )
    .unwrap();
    let error =
        require_xml_payloads_unchanged(&workspace, &captured_post, "post-workspace").unwrap_err();
    assert!(error.contains("changed after"), "{error}");

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn non_xml_manifest_inventory_binds_bsl_tampering_for_none_impact_case() {
    let root = unique_temp_dir("non-xml-manifest-tamper");
    let case = EXECUTABLE_CASES
        .iter()
        .find(|case| case.id == "code-patch-bsl-only")
        .unwrap();
    let mut gate = SequentialCallGate::default();

    let generated = run_corpus_case(&root, case, &mut gate).unwrap();
    let manifest_case = serde_json::to_value(generated).unwrap();
    let module_suffix = "src/CommonModules/CorpusModule/Ext/Module.bsl";
    let pre_entry = manifest_case["preNonXmlFiles"]
        .as_array()
        .and_then(|files| {
            files.iter().find(|file| {
                file["path"]
                    .as_str()
                    .is_some_and(|path| path.ends_with(module_suffix))
            })
        })
        .expect("pre-call BSL must be bound by the manifest");
    let post_entry = manifest_case["nonXmlFiles"]
        .as_array()
        .and_then(|files| {
            files.iter().find(|file| {
                file["path"]
                    .as_str()
                    .is_some_and(|path| path.ends_with(module_suffix))
            })
        })
        .expect("post-call BSL must be bound by the manifest");

    assert_eq!(
        pre_entry["sha256"],
        format!(
            "{:x}",
            Sha256::digest(
                b"\xef\xbb\xbfProcedure Run()\r\n    Message(\"ok\");\r\nEndProcedure\r\n"
            )
        )
    );
    assert_eq!(post_entry["seed"], true);
    assert_eq!(post_entry["delta"], "modified");
    let workspace = root.join("cases/code-patch-bsl-only/workspace");
    let module = workspace.join(module_suffix);
    assert_eq!(post_entry["sha256"], sha256_file(&module).unwrap());
    assert!(manifest_case["preNonXmlFiles"]
        .as_array()
        .unwrap()
        .iter()
        .all(|file| {
            file["path"]
                .as_str()
                .is_some_and(|path| path.contains("/pre-non-xml/src/") && !path.ends_with(".json"))
        }));
    assert!(manifest_case["nonXmlFiles"]
        .as_array()
        .unwrap()
        .iter()
        .all(|file| {
            file["path"]
                .as_str()
                .is_some_and(|path| path.contains("/workspace/src/") && !path.ends_with(".json"))
        }));
    let materialized_pre_module = root
        .join("cases/code-patch-bsl-only/pre-non-xml")
        .join(module_suffix);
    assert_eq!(
        pre_entry["sha256"],
        sha256_file(&materialized_pre_module).unwrap()
    );

    let post_payloads = capture_non_xml_payloads(case, &workspace).unwrap();
    fs::write(&module, "tampered after manifest").unwrap();
    assert_ne!(post_entry["sha256"], sha256_file(&module).unwrap());
    let error =
        require_non_xml_payloads_unchanged(case, &workspace, &post_payloads, "tampered workspace")
            .unwrap_err();
    assert!(error.contains("changed after"), "{error}");
    fs::remove_dir_all(root).unwrap();
}

fn assert_platform_canonical_bsl(bytes: &[u8]) {
    assert!(
        bytes.starts_with(b"\xef\xbb\xbf"),
        "platform BSL must start with a UTF-8 BOM"
    );
    let payload = &bytes[3..];
    std::str::from_utf8(payload).expect("platform BSL payload must be UTF-8");
    assert!(
        payload.windows(2).any(|window| window == b"\r\n"),
        "platform BSL must contain CRLF line endings"
    );
    for (offset, byte) in payload.iter().enumerate() {
        match byte {
            b'\n' => assert_eq!(
                payload.get(offset.wrapping_sub(1)),
                Some(&b'\r'),
                "platform BSL contains a bare LF at byte {offset}"
            ),
            b'\r' => assert_eq!(
                payload.get(offset + 1),
                Some(&b'\n'),
                "platform BSL contains a bare CR at byte {offset}"
            ),
            _ => {}
        }
    }
}

#[test]
fn meta_reference_cases_seed_platform_canonical_event_handlers_bsl() {
    let expected = concat!(
        "\u{feff}Procedure RunJob() Export\r\n",
        "EndProcedure\r\n\r\n",
        "Procedure OnBeforeWrite(Source, Cancel) Export\r\n",
        "EndProcedure\r\n"
    )
    .as_bytes();

    for case_id in [
        "meta-compile-scheduled-job",
        "meta-compile-event-subscription",
    ] {
        let root = unique_temp_dir(case_id);
        let workspace = root.join("workspace");
        fs::create_dir_all(&workspace).unwrap();
        let case = EXECUTABLE_CASES
            .iter()
            .find(|case| case.id == case_id)
            .unwrap();

        prepare_target(case, &workspace).unwrap();

        let module =
            fs::read(workspace.join("src/CommonModules/EventHandlers/Ext/Module.bsl")).unwrap();
        assert_platform_canonical_bsl(&module);
        assert_eq!(module, expected, "{case_id}");
        fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn code_patch_case_seeds_and_preserves_platform_canonical_bsl_bytes() {
    let root = unique_temp_dir("code-patch-canonical-bsl");
    let case = EXECUTABLE_CASES
        .iter()
        .find(|case| case.id == "code-patch-bsl-only")
        .unwrap();
    let mut gate = SequentialCallGate::default();

    run_corpus_case(&root, case, &mut gate).unwrap();

    let relative = "src/CommonModules/CorpusModule/Ext/Module.bsl";
    let pre = fs::read(
        root.join("cases/code-patch-bsl-only/pre-non-xml")
            .join(relative),
    )
    .unwrap();
    let post = fs::read(
        root.join("cases/code-patch-bsl-only/workspace")
            .join(relative),
    )
    .unwrap();
    assert_platform_canonical_bsl(&pre);
    assert_platform_canonical_bsl(&post);
    assert!(
        std::str::from_utf8(&post[3..])
            .unwrap()
            .contains("Procedure Added()\r\nEndProcedure\r\n"),
        "inserted code must use the surrounding platform CRLF convention"
    );

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn cfe_patch_method_case_seeds_a_registered_adopted_common_module() {
    let root = unique_temp_dir("cfe-patch-registered-common-module");
    let workspace = root.join("workspace");
    fs::create_dir_all(&workspace).unwrap();
    let case = EXECUTABLE_CASES
        .iter()
        .find(|case| case.id == "cfe-patch-method-bsl-only")
        .unwrap();

    prepare_target(case, &workspace).unwrap();

    let extension = fs::read_to_string(workspace.join("ext/Configuration.xml")).unwrap();
    let extension_document = Document::parse(extension.trim_start_matches('\u{feff}')).unwrap();
    let registered = extension_document
        .descendants()
        .filter(|node| node.is_element() && node.tag_name().name() == "CommonModule")
        .filter_map(|node| node.text())
        .collect::<Vec<_>>();
    assert_eq!(registered, ["CorpusModule"]);

    let descriptor_path = workspace.join("ext/CommonModules/CorpusModule.xml");
    let descriptor = fs::read_to_string(&descriptor_path).unwrap();
    let descriptor_document = Document::parse(descriptor.trim_start_matches('\u{feff}')).unwrap();
    assert_eq!(
        descriptor_document.root_element().attribute("version"),
        Some("2.20")
    );
    let direct_objects = descriptor_document
        .root_element()
        .children()
        .filter(|node| node.is_element())
        .collect::<Vec<_>>();
    assert_eq!(direct_objects.len(), 1);
    assert_eq!(direct_objects[0].tag_name().name(), "CommonModule");
    let property = |name| {
        direct_objects[0]
            .descendants()
            .find(|node| node.is_element() && node.tag_name().name() == name)
            .and_then(|node| node.text())
    };
    assert_eq!(property("Name"), Some("CorpusModule"));
    assert_eq!(property("ObjectBelonging"), Some("Adopted"));
    let extended = property("ExtendedConfigurationObject").unwrap();
    assert!(
        uuid::Uuid::parse_str(extended).is_ok(),
        "borrowed descriptor must identify its base object with a GUID: {extended}"
    );
    assert!(
        !workspace
            .join("ext/CommonModules/CorpusModule/Ext/Module.bsl")
            .exists(),
        "borrow must leave Module.bsl creation to cfe.patch_method"
    );
    let base_module =
        fs::read(workspace.join("src/CommonModules/CorpusModule/Ext/Module.bsl")).unwrap();
    assert_platform_canonical_bsl(&base_module);
    assert!(
        std::str::from_utf8(&base_module[3..])
            .unwrap()
            .contains("Procedure Run()\r\n"),
        "the borrowed interceptor target must exist in the base CommonModule"
    );

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn non_xml_inventory_covers_every_xml_none_impact_case() {
    let expectations = [
        (
            "code-patch-bsl-only",
            "src/CommonModules/CorpusModule/Ext/Module.bsl",
            true,
            "modified",
        ),
        (
            "support-edit-bin-only",
            "src/Ext/ParentConfigurations.bin",
            true,
            "modified",
        ),
    ];
    for (case_id, payload_suffix, seed, delta) in expectations {
        let root = unique_temp_dir(case_id);
        let case = EXECUTABLE_CASES
            .iter()
            .find(|case| case.id == case_id)
            .unwrap();
        assert_eq!(effective_xml_impact(case_id), XmlImpactClass::None);
        let mut gate = SequentialCallGate::default();

        let generated = run_corpus_case(&root, case, &mut gate).unwrap();
        let manifest_case = serde_json::to_value(generated).unwrap();
        let post = manifest_case["nonXmlFiles"]
            .as_array()
            .and_then(|files| {
                files.iter().find(|file| {
                    file["path"]
                        .as_str()
                        .is_some_and(|path| path.ends_with(payload_suffix))
                })
            })
            .unwrap_or_else(|| panic!("{case_id} did not inventory {payload_suffix}"));

        assert_eq!(post["seed"], seed, "{case_id}");
        assert_eq!(post["delta"], delta, "{case_id}");
        assert_eq!(
            manifest_case["preNonXmlFiles"]
                .as_array()
                .unwrap()
                .iter()
                .any(|file| file["path"]
                    .as_str()
                    .is_some_and(|path| path.ends_with(payload_suffix))),
            seed,
            "{case_id}"
        );
        if payload_suffix.ends_with(".bsl") {
            assert_platform_canonical_bsl(
                &fs::read(
                    root.join("cases")
                        .join(case_id)
                        .join("workspace")
                        .join(payload_suffix),
                )
                .unwrap(),
            );
        }
        fs::remove_dir_all(root).unwrap();
    }
}

fn assert_exact_extended_property_state(path: &Path, expected_property: &str) {
    let xml = fs::read_to_string(path).unwrap();
    let document = Document::parse(xml.trim_start_matches('\u{feff}'))
        .unwrap_or_else(|error| panic!("{}: {error}: {xml}", path.display()));
    let states = document
        .descendants()
        .filter(|node| node.has_tag_name(("http://v8.1c.ru/8.3/xcf/readable", "PropertyState")))
        .filter_map(|state| {
            let property = state
                .children()
                .find(|node| node.has_tag_name(("http://v8.1c.ru/8.3/xcf/readable", "Property")))
                .and_then(|node| node.text())?;
            let value = state
                .children()
                .find(|node| node.has_tag_name(("http://v8.1c.ru/8.3/xcf/readable", "State")))
                .and_then(|node| node.text())?;
            Some((property, value))
        })
        .collect::<Vec<_>>();
    assert_eq!(
        states,
        [(expected_property, "Extended")],
        "{}: {xml}",
        path.display()
    );
}

#[test]
fn cfe_patch_method_inventory_covers_atomic_xml_and_bsl_change() {
    let expectations = [
        (
            "cfe-patch-method-bsl-only",
            "ext/CommonModules/CorpusModule/Ext/Module.bsl",
            "src/CommonModules/CorpusModule/Ext/Module.bsl",
            "ext/CommonModules/CorpusModule.xml",
            "Module",
            XmlImpactClass::CreateOrModify,
            "modified",
        ),
        (
            "cfe-patch-method-catalog-object-module",
            "ext/Catalogs/CorpusCatalog/Ext/ObjectModule.bsl",
            "src/Catalogs/CorpusCatalog/Ext/ObjectModule.bsl",
            "ext/Catalogs/CorpusCatalog.xml",
            "ObjectModule",
            XmlImpactClass::CreateOrModify,
            "modified",
        ),
        (
            "cfe-patch-method-catalog-manager-module",
            "ext/Catalogs/CorpusCatalog/Ext/ManagerModule.bsl",
            "src/Catalogs/CorpusCatalog/Ext/ManagerModule.bsl",
            "ext/Catalogs/CorpusCatalog.xml",
            "ManagerModule",
            XmlImpactClass::CreateOrModify,
            "modified",
        ),
        (
            "cfe-patch-method-information-register-record-set-module",
            "ext/InformationRegisters/CorpusInformationRegister/Ext/RecordSetModule.bsl",
            "src/InformationRegisters/CorpusInformationRegister/Ext/RecordSetModule.bsl",
            "ext/InformationRegisters/CorpusInformationRegister.xml",
            "RecordSetModule",
            XmlImpactClass::CreateOrModify,
            "modified",
        ),
        (
            "cfe-patch-method-catalog-form-module",
            "ext/Catalogs/CorpusCatalog/Forms/CorpusForm/Ext/Form/Module.bsl",
            "src/Catalogs/CorpusCatalog/Forms/CorpusForm/Ext/Form/Module.bsl",
            "ext/Catalogs/CorpusCatalog/Forms/CorpusForm.xml",
            "Form",
            XmlImpactClass::None,
            "unchanged",
        ),
        (
            "cfe-patch-method-constant-value-manager-module",
            "ext/Constants/CorpusConstant/Ext/ValueManagerModule.bsl",
            "src/Constants/CorpusConstant/Ext/ValueManagerModule.bsl",
            "ext/Constants/CorpusConstant.xml",
            "ValueManagerModule",
            XmlImpactClass::CreateOrModify,
            "modified",
        ),
    ];

    for (
        case_id,
        extension_module,
        base_module,
        descriptor,
        property,
        expected_impact,
        expected_descriptor_delta,
    ) in expectations
    {
        assert_eq!(
            registry_entry_for_case(case_id).impact,
            XmlImpactClass::CreateOrModify,
            "{case_id} must advertise its XML mutation capability"
        );
        assert_eq!(effective_xml_impact(case_id), expected_impact, "{case_id}");
        let root = unique_temp_dir(case_id);
        let case = EXECUTABLE_CASES
            .iter()
            .find(|case| case.id == case_id)
            .unwrap();
        let mut gate = SequentialCallGate::default();

        let generated = run_corpus_case(&root, case, &mut gate).unwrap();
        let manifest_case = serde_json::to_value(generated).unwrap();
        assert_eq!(
            manifest_case["impactClass"],
            impact_class_name(expected_impact),
            "{case_id}"
        );

        let extension_post = manifest_case["nonXmlFiles"]
            .as_array()
            .and_then(|files| {
                files.iter().find(|file| {
                    file["path"]
                        .as_str()
                        .is_some_and(|path| path.ends_with(extension_module))
                })
            })
            .unwrap_or_else(|| panic!("{case_id} did not inventory {extension_module}"));
        assert_eq!(extension_post["seed"], false, "{case_id}");
        assert_eq!(extension_post["delta"], "created", "{case_id}");

        let pre_base = manifest_case["preNonXmlFiles"]
            .as_array()
            .unwrap()
            .iter()
            .find(|file| {
                file["path"]
                    .as_str()
                    .is_some_and(|path| path.ends_with(base_module))
            })
            .expect("base target module must be captured before cfe.patch_method");
        let post_base = manifest_case["nonXmlFiles"]
            .as_array()
            .unwrap()
            .iter()
            .find(|file| {
                file["path"]
                    .as_str()
                    .is_some_and(|path| path.ends_with(base_module))
            })
            .expect("base target module must remain in the extension checkpoint");
        assert_eq!(post_base["seed"], true, "{case_id}");
        assert_eq!(post_base["delta"], "unchanged", "{case_id}");
        assert_eq!(post_base["sha256"], pre_base["sha256"], "{case_id}");

        let descriptor_post = manifest_case["files"]
            .as_array()
            .and_then(|files| {
                files.iter().find(|file| {
                    file["path"]
                        .as_str()
                        .is_some_and(|path| path.ends_with(descriptor))
                })
            })
            .unwrap_or_else(|| panic!("{case_id} did not inventory {descriptor}"));
        assert_eq!(
            descriptor_post["delta"], expected_descriptor_delta,
            "{case_id}"
        );

        let workspace = root.join("cases").join(case_id).join("workspace");
        assert_platform_canonical_bsl(&fs::read(workspace.join(extension_module)).unwrap());
        assert_platform_canonical_bsl(&fs::read(workspace.join(base_module)).unwrap());
        assert_exact_extended_property_state(&workspace.join(descriptor), property);
        fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn html_template_case_inventories_the_platform_page_set_layout() {
    let root = unique_temp_dir("html-template-page-set");
    let case = EXECUTABLE_CASES
        .iter()
        .find(|case| case.id == "template-add-html-document")
        .unwrap();
    let mut gate = SequentialCallGate::default();

    let generated = run_corpus_case(&root, case, &mut gate).unwrap();
    let manifest_case = serde_json::to_value(generated).unwrap();
    let descriptor_suffix = "src/Reports/CorpusReport/Templates/CorpusTemplate/Ext/Template.xml";
    let page_suffix = "src/Reports/CorpusReport/Templates/CorpusTemplate/Ext/Template/ru.html";

    assert!(manifest_case["files"]
        .as_array()
        .unwrap()
        .iter()
        .any(|file| file["path"]
            .as_str()
            .is_some_and(|path| path.ends_with(descriptor_suffix))));
    let page = manifest_case["nonXmlFiles"]
        .as_array()
        .unwrap()
        .iter()
        .find(|file| {
            file["path"]
                .as_str()
                .is_some_and(|path| path.ends_with(page_suffix))
        })
        .expect("HTML page must be captured as exact non-XML evidence");
    assert_eq!(page["seed"], false);
    assert_eq!(page["delta"], "created");

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn corpus_case_inventories_stable_files_outside_platform_boundaries() {
    let root = unique_temp_dir("auxiliary-files");
    let case = EXECUTABLE_CASES
        .iter()
        .find(|case| case.id == "meta-compile-catalog")
        .unwrap();
    let mut gate = SequentialCallGate::default();

    let generated = run_corpus_case(&root, case, &mut gate).unwrap();
    let paths = generated
        .auxiliary_files
        .iter()
        .map(|file| file.path.as_str())
        .collect::<BTreeSet<_>>();

    assert!(paths
        .iter()
        .any(|path| path.ends_with("/workspace/v8project.yaml")));
    assert!(paths.iter().any(|path| path.contains("/workspace/inputs/")));
    assert!(paths.iter().all(|path| !path.contains("/workspace/src/")));

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn owner_assignment_requires_same_case_source_set_path() {
    let owners = BTreeMap::from([
        ("src/Configuration.xml".to_string(), "src".to_string()),
        ("ext/Configuration.xml".to_string(), "ext".to_string()),
    ]);

    assert_eq!(
        unique_deepest_owner_for_path("src/Ext/ClientApplicationInterface.xml", &owners).unwrap(),
        Some("src/Configuration.xml")
    );
    assert_eq!(
        unique_deepest_owner_for_path("ext/Ext/ClientApplicationInterface.xml", &owners).unwrap(),
        Some("ext/Configuration.xml")
    );
    assert_eq!(
        unique_deepest_owner_for_path("standalone/Dcs.xml", &owners).unwrap(),
        None
    );

    let ambiguous = BTreeMap::from([
        ("src/Configuration.xml".to_string(), "src".to_string()),
        ("src/Extension.xml".to_string(), "src".to_string()),
    ]);
    assert!(
        unique_deepest_owner_for_path("src/Ext/ClientApplicationInterface.xml", &ambiguous)
            .unwrap_err()
            .contains("unique deepest")
    );
}

#[test]
fn unlisted_xml_prevention_rejects_missing_and_stale_paths() {
    let actual = BTreeMap::from([
        ("a.xml".to_string(), "a".to_string()),
        ("b.xml".to_string(), "b".to_string()),
    ]);
    let error = ensure_all_xml_listed(&actual, &BTreeSet::from(["a.xml".to_string()])).unwrap_err();
    assert!(error.contains("b.xml"), "{error}");

    let error = ensure_all_xml_listed(
        &actual,
        &BTreeSet::from([
            "a.xml".to_string(),
            "b.xml".to_string(),
            "stale.xml".to_string(),
        ]),
    )
    .unwrap_err();
    assert!(error.contains("stale.xml"), "{error}");
}

#[test]
fn output_directory_refusal_rules_are_fail_closed() {
    let root = unique_temp_dir("output-safety");
    let repo = root.join("repo");
    let home = root.join("home");
    let empty = root.join("empty");
    let non_empty = root.join("non-empty");
    fs::create_dir_all(&repo).unwrap();
    fs::create_dir_all(&home).unwrap();
    fs::create_dir_all(&empty).unwrap();
    fs::create_dir_all(&non_empty).unwrap();
    fs::write(non_empty.join("sentinel"), "keep").unwrap();

    assert!(configured_output_directory_from(None, &repo, &home).is_err());
    assert!(validate_output_directory("", &repo, &home).is_err());
    assert!(validate_output_directory("/", &repo, &home).is_err());
    assert!(validate_output_directory(home.to_str().unwrap(), &repo, &home).is_err());
    assert!(validate_output_directory(repo.to_str().unwrap(), &repo, &home).is_err());
    assert!(validate_output_directory(non_empty.to_str().unwrap(), &repo, &home).is_err());
    assert_eq!(
        validate_output_directory(empty.to_str().unwrap(), &repo, &home).unwrap(),
        empty.canonicalize().unwrap()
    );
    let absent = root.join("explicit-absent-target");
    assert_eq!(
        validate_output_directory(absent.to_str().unwrap(), &repo, &home).unwrap(),
        root.canonicalize().unwrap().join("explicit-absent-target")
    );
    assert!(!absent.exists());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn empty_directory_inventory_detects_added_and_removed_paths() {
    let root = unique_temp_dir("empty-directory-inventory");
    fs::create_dir_all(root.join("stable/with-file")).unwrap();
    fs::write(root.join("stable/with-file/payload.xml"), "<r/>").unwrap();
    fs::create_dir_all(root.join("removed-empty")).unwrap();

    let before = capture_empty_directory_paths(&root).unwrap();
    assert_eq!(before, vec!["removed-empty"]);

    fs::remove_dir(root.join("removed-empty")).unwrap();
    fs::create_dir(root.join("added-empty")).unwrap();
    let after = capture_empty_directory_paths(&root).unwrap();

    assert_eq!(after, vec!["added-empty"]);
    assert_ne!(before, after);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn manifest_sorting_and_task_7a_shape_round_trip() {
    let file = |path: &str| CorpusFile {
        path: path.to_string(),
        sha256: "a".repeat(64),
        family: "metadata".to_string(),
        seed: false,
        delta: "created".to_string(),
        owner_path: None,
        new_standalone: None,
    };
    let case = |id: &str, paths: &[&str]| CorpusCase {
        id: id.to_string(),
        workspace_path: format!("cases/{id}/workspace"),
        pre_snapshot_path: format!("cases/{id}/pre-xml"),
        platform_checkpoint: PlatformCheckpoint {
            kind: "configuration".to_string(),
            source_path: Some(format!("cases/{id}/workspace/src")),
            base_source_path: None,
            covered_case_ids: vec![id.to_string()],
        },
        checkpoint: format!("cases/{id}/case-report.json"),
        tool_id: "unica.cf.init".to_string(),
        operation: "cf-init".to_string(),
        branch: "default".to_string(),
        impact_class: "CreateOrModify".to_string(),
        xml_impact: "created".to_string(),
        pre_files: Vec::new(),
        files: paths.iter().map(|path| file(path)).collect(),
        removed_paths: Vec::new(),
        pre_non_xml_files: vec![PreNonXmlFile {
            path: format!("cases/{id}/pre-non-xml/src/Module.bsl"),
            sha256: "b".repeat(64),
        }],
        non_xml_files: vec![NonXmlFile {
            path: format!("cases/{id}/workspace/src/Module.bsl"),
            sha256: "c".repeat(64),
            seed: true,
            delta: "modified".to_string(),
        }],
        removed_non_xml_paths: Vec::new(),
        auxiliary_files: Vec::new(),
        pre_owner_versions: BTreeMap::new(),
        owner_versions: BTreeMap::new(),
    };
    let mut manifest = CorpusManifest {
        schema_version: 2,
        profile: "1c-8.3.27-export-2.20".to_string(),
        empty_directory_paths: vec![
            "cases/z/workspace/z-empty".to_string(),
            "cases/a/workspace/a-empty".to_string(),
        ],
        cases: vec![case("z", &["z/b.xml", "z/a.xml"]), case("a", &["a/a.xml"])],
    };

    sort_manifest(&mut manifest);
    let bytes = serde_json::to_vec(&manifest).unwrap();
    let reparsed: CorpusManifest = serde_json::from_slice(&bytes).unwrap();
    let task_7a_view: Value = serde_json::from_slice(&bytes).unwrap();

    assert_eq!(reparsed.cases[0].id, "a");
    assert_eq!(reparsed.cases[1].files[0].path, "z/a.xml");
    assert_eq!(task_7a_view["schemaVersion"], 2);
    assert_eq!(
        task_7a_view["emptyDirectoryPaths"],
        json!(["cases/a/workspace/a-empty", "cases/z/workspace/z-empty"])
    );
    assert_eq!(task_7a_view["profile"], "1c-8.3.27-export-2.20");
    for case in task_7a_view["cases"].as_array().unwrap() {
        assert!(case["id"].is_string());
        assert!(case["toolId"].is_string());
        assert!(case["xmlImpact"].is_string());
        assert!(case["preSnapshotPath"].is_string());
        assert!(case["preFiles"].is_array());
        assert!(case["preNonXmlFiles"].is_array());
        assert!(case["nonXmlFiles"].is_array());
        assert!(case["removedNonXmlPaths"].is_array());
        assert!(case["auxiliaryFiles"].is_array());
        assert!(case["preOwnerVersions"].is_object());
        for file in case["files"].as_array().unwrap() {
            assert!(file["path"].is_string());
            assert!(file["sha256"].is_string());
            assert!(file["family"].is_string());
            assert!(file["seed"].is_boolean());
        }
        for file in case["preNonXmlFiles"].as_array().unwrap() {
            assert!(file["path"].is_string());
            assert!(file["sha256"].is_string());
        }
        for file in case["nonXmlFiles"].as_array().unwrap() {
            assert!(file["path"].is_string());
            assert!(file["sha256"].is_string());
            assert!(file["seed"].is_boolean());
            assert!(file["delta"].is_string());
        }
    }
}

#[test]
fn dcs_corpus_exercises_xsd_order_sensitive_contracts() {
    let definition = dcs_definition();
    let data_sets = definition["dataSets"].as_array().unwrap();
    assert!(
        data_sets
            .iter()
            .any(|data_set| data_set["items"].is_array()),
        "DCS corpus must exercise DataSetUnion item emission"
    );

    let link = &definition["dataSetLinks"][0];
    assert!(link["linkConditionExpression"].is_string());
    assert!(link["startExpression"].is_string());

    let calculated = &definition["calculatedFields"][0];
    assert!(calculated["restrict"].is_array());
    assert!(calculated["useRestriction"].is_array());
    assert!(calculated["type"].is_string());

    let parameters = definition["parameters"].as_array().unwrap();
    assert!(parameters.iter().any(|parameter| {
        parameter["name"].as_str() == Some("ТипКорпуса")
            && parameter["type"].as_str() == Some("string(16)")
    }));
    assert!(parameters.iter().any(|parameter| {
        parameter["availableValues"]
            .as_array()
            .is_some_and(|values| !values.is_empty())
    }));
    assert!(parameters.iter().any(|parameter| {
        parameter
            .as_str()
            .is_some_and(|value| value.contains("@autoDates"))
    }));

    let settings = &definition["settingsVariants"][0]["settings"];
    for key in [
        "selection",
        "filter",
        "dataParameters",
        "order",
        "conditionalAppearance",
        "outputParameters",
        "structure",
    ] {
        assert!(!settings[key].is_null(), "missing settings.{key}");
    }
    let group = &settings["structure"][0];
    for key in [
        "groupBy",
        "filter",
        "order",
        "selection",
        "conditionalAppearance",
        "outputParameters",
    ] {
        assert!(!group[key].is_null(), "missing structure group {key}");
    }
}

#[test]
fn dcs_edit_corpus_covers_order_sensitive_mutations() {
    let entry = MUTATOR_REGISTRY
        .iter()
        .find(|entry| entry.tool == "unica.dcs.edit")
        .unwrap();
    assert_eq!(
        entry.case_ids,
        [
            "dcs-edit-owned-template",
            "dcs-edit-add-parameter-after-settings",
            "dcs-edit-set-structure-after-settings",
            "dcs-edit-modify-field-role-restriction",
        ]
    );
}

#[test]
fn cfe_patch_method_corpus_covers_every_supported_module_layout_family() {
    let entry = MUTATOR_REGISTRY
        .iter()
        .find(|entry| entry.tool == "unica.cfe.patch_method")
        .unwrap();
    assert_eq!(
        entry.case_ids,
        [
            "cfe-patch-method-bsl-only",
            "cfe-patch-method-catalog-object-module",
            "cfe-patch-method-catalog-manager-module",
            "cfe-patch-method-information-register-record-set-module",
            "cfe-patch-method-catalog-form-module",
            "cfe-patch-method-constant-value-manager-module",
        ]
    );
    assert_eq!(
        entry.required_branches,
        [
            "CommonModule",
            "Catalog.ObjectModule",
            "Catalog.ManagerModule",
            "InformationRegister.RecordSetModule",
            "Catalog.Form",
            "Constant.ValueManagerModule",
        ]
    );
}

#[test]
fn corpus_owner_version_uses_the_raw_lexical_attribute() {
    let payload = br#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.&#50;0"><Configuration/></MetaDataObject>"#;

    let (_, _, owner_type, version) =
        xml_root_details_payload(payload, "entity-version.xml").unwrap();

    assert_eq!(owner_type.as_deref(), Some("Configuration"));
    assert_eq!(version.as_deref(), Some("2.&#50;0"));
}

#[test]
#[ignore = "writes an explicit developer-selected public-tool XML corpus"]
fn generate_platform_xml_corpus() {
    let output = configured_output_directory().expect("safe UNICA_XML_CORPUS_DIR");

    let manifest = generate_corpus(&output).expect("generate complete public-tool XML corpus");

    assert_eq!(manifest.schema_version, 2);
    assert_eq!(manifest.profile, "1c-8.3.27-export-2.20");
    assert_eq!(manifest.cases.len(), EXECUTABLE_CASES.len());
}
