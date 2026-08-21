#![allow(dead_code, unused_imports)]

use crate::application::AdapterOutcome;
use crate::domain::cache::{CacheAccess, CacheReport};
use crate::domain::events::{DomainEvent, DomainEventKind};
use crate::domain::role::{
    known_role_rights, nested_right_shape_is_modelled, parse_role_edit_request,
    validate_nested_right, RoleEditData, RoleEditEffect, RoleEditEffectAction,
};
use crate::domain::source_target::{
    MetadataAddress, SourceTarget, SourceTargetErrorCode, TargetKind,
    PLATFORM_XML_8_3_27_FORMAT_2_20,
};
use crate::domain::support_state::{
    ObjectSupportData as DomainObjectSupportData, SupportStateReader,
};
use crate::domain::workspace::WorkspaceContext;
use crate::infrastructure::native_operations::logical_selector::prove_attached_resource;
use crate::infrastructure::native_operations::text_snapshot::{
    resolve_line_ending, EolPolicy, LineEndingProfile, SourceTextSnapshot,
};
use crate::infrastructure::path_policy::WorkspacePathPolicy;
use crate::infrastructure::platform::filesystem::metadata_is_link_or_reparse_point;
use crate::infrastructure::platform_xml_roots::{
    platform_xml_publication_policy, PlatformXmlPublicationPolicy,
};
use crate::infrastructure::platform_xml_source_targets::{
    platform_xml_resource_evidence, resolve_platform_xml_target, ClosedPlatformXmlTarget,
    PlatformXmlResourceEvidence, TargetKindPolicy,
};
use crate::infrastructure::source_roots::normalize_path_identity;
use crate::infrastructure::support_guard::{
    bind_resolved_support_guard_evidence, evaluate_resolved_support_guard,
    ResolvedSupportGuardCheck,
};
use crate::infrastructure::support_state::WorkspaceSupportStateReader;
use crate::infrastructure::workspace_state::WorkspaceStateRepository;
use roxmltree::Document;
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use super::common::*;
use super::compile_transaction::{
    CommitFailure, CommitFailureKind, CompileTransaction, RegistrationStatus,
};
use super::{
    cf::*, cfe::*, dcs::*, form::*, interface::*, meta::*, mxl::*, subsystem::*, template::*,
};

#[cfg(test)]
type RoleCompileAfterConfigurationProbeHook = Box<dyn FnOnce(&Path)>;

#[cfg(test)]
thread_local! {
    static ROLE_COMPILE_AFTER_CONFIGURATION_PROBE_HOOK:
        std::cell::RefCell<Option<RoleCompileAfterConfigurationProbeHook>> =
        const { std::cell::RefCell::new(None) };
}

#[cfg(test)]
thread_local! {
    static ROLE_EDIT_BEFORE_NATIVE_GUARD_HOOK: std::cell::RefCell<Option<Box<dyn FnOnce()>>> =
        const { std::cell::RefCell::new(None) };
    static ROLE_EDIT_BEFORE_PUBLISH_HOOK: std::cell::RefCell<Option<Box<dyn FnOnce()>>> =
        const { std::cell::RefCell::new(None) };
    static ROLE_EDIT_AFTER_RIGHTS_REREAD_HOOK: std::cell::RefCell<Option<Box<dyn FnOnce()>>> =
        const { std::cell::RefCell::new(None) };
    static ROLE_EDIT_POST_VALIDATION_FAILURE: std::cell::Cell<bool> = const {
        std::cell::Cell::new(false)
    };
}

#[cfg(test)]
fn with_role_edit_before_native_guard_hook<T>(
    hook: impl FnOnce() + 'static,
    action: impl FnOnce() -> T,
) -> T {
    struct Reset(Option<Box<dyn FnOnce()>>);
    impl Drop for Reset {
        fn drop(&mut self) {
            ROLE_EDIT_BEFORE_NATIVE_GUARD_HOOK.with(|slot| {
                slot.replace(self.0.take());
            });
        }
    }
    let previous =
        ROLE_EDIT_BEFORE_NATIVE_GUARD_HOOK.with(|slot| slot.replace(Some(Box::new(hook))));
    let _reset = Reset(previous);
    action()
}

#[cfg(test)]
fn run_role_edit_before_native_guard_hook() {
    ROLE_EDIT_BEFORE_NATIVE_GUARD_HOOK.with(|slot| {
        if let Some(hook) = slot.borrow_mut().take() {
            hook();
        }
    });
}

#[cfg(test)]
fn with_role_edit_before_publish_hook<T>(
    hook: impl FnOnce() + 'static,
    action: impl FnOnce() -> T,
) -> T {
    struct Reset(Option<Box<dyn FnOnce()>>);
    impl Drop for Reset {
        fn drop(&mut self) {
            ROLE_EDIT_BEFORE_PUBLISH_HOOK.with(|slot| {
                slot.replace(self.0.take());
            });
        }
    }
    let previous = ROLE_EDIT_BEFORE_PUBLISH_HOOK.with(|slot| slot.replace(Some(Box::new(hook))));
    let _reset = Reset(previous);
    action()
}

#[cfg(test)]
fn run_role_edit_before_publish_hook() {
    ROLE_EDIT_BEFORE_PUBLISH_HOOK.with(|slot| {
        if let Some(hook) = slot.borrow_mut().take() {
            hook();
        }
    });
}

#[cfg(test)]
fn with_role_edit_after_rights_reread_hook<T>(
    hook: impl FnOnce() + 'static,
    action: impl FnOnce() -> T,
) -> T {
    struct Reset(Option<Box<dyn FnOnce()>>);
    impl Drop for Reset {
        fn drop(&mut self) {
            ROLE_EDIT_AFTER_RIGHTS_REREAD_HOOK.with(|slot| {
                slot.replace(self.0.take());
            });
        }
    }
    let previous =
        ROLE_EDIT_AFTER_RIGHTS_REREAD_HOOK.with(|slot| slot.replace(Some(Box::new(hook))));
    let _reset = Reset(previous);
    action()
}

#[cfg(test)]
fn run_role_edit_after_rights_reread_hook() {
    ROLE_EDIT_AFTER_RIGHTS_REREAD_HOOK.with(|slot| {
        if let Some(hook) = slot.borrow_mut().take() {
            hook();
        }
    });
}

#[cfg(test)]
fn with_role_edit_post_validation_failure<T>(action: impl FnOnce() -> T) -> T {
    struct Reset(bool);
    impl Drop for Reset {
        fn drop(&mut self) {
            ROLE_EDIT_POST_VALIDATION_FAILURE.with(|slot| slot.set(self.0));
        }
    }
    let previous = ROLE_EDIT_POST_VALIDATION_FAILURE.with(|slot| slot.replace(true));
    let _reset = Reset(previous);
    action()
}

#[cfg(test)]
fn with_role_compile_after_configuration_probe_hook<T>(
    hook: impl FnOnce(&Path) + 'static,
    action: impl FnOnce() -> T,
) -> T {
    struct Reset(Option<RoleCompileAfterConfigurationProbeHook>);
    impl Drop for Reset {
        fn drop(&mut self) {
            ROLE_COMPILE_AFTER_CONFIGURATION_PROBE_HOOK.with(|slot| {
                slot.replace(self.0.take());
            });
        }
    }

    let previous =
        ROLE_COMPILE_AFTER_CONFIGURATION_PROBE_HOOK.with(|slot| slot.replace(Some(Box::new(hook))));
    let _reset = Reset(previous);
    action()
}

#[cfg(test)]
fn run_role_compile_after_configuration_probe_hook(path: &Path) {
    if let Some(hook) =
        ROLE_COMPILE_AFTER_CONFIGURATION_PROBE_HOOK.with(|slot| slot.borrow_mut().take())
    {
        hook(path);
    }
}

#[derive(Clone)]
pub(crate) struct RoleRight {
    pub(crate) name: String,
    pub(crate) value: String,
    pub(crate) condition: Option<String>,
}

#[derive(Clone)]
pub(crate) struct RoleObject {
    pub(crate) name: String,
    pub(crate) rights: Vec<RoleRight>,
}

pub(crate) struct RoleInfoRightSummary {
    pub(crate) name: String,
    pub(crate) rls: bool,
}

pub(crate) struct RoleInfoObjectSummary {
    pub(crate) short_name: String,
    pub(crate) rights: Vec<RoleInfoRightSummary>,
}

pub(crate) struct RoleInfoGroup {
    pub(crate) type_prefix: String,
    pub(crate) objects: Vec<RoleInfoObjectSummary>,
}

struct RoleReadLayout {
    role_dir_name: String,
    metadata_path: PathBuf,
    configuration_path: PathBuf,
}

fn role_read_layout(rights_path: &Path) -> RoleReadLayout {
    let ext_dir = rights_path.parent().unwrap_or_else(|| Path::new(""));
    let role_dir = ext_dir
        .parent()
        .unwrap_or_else(|| Path::new(""))
        .to_path_buf();
    let roles_dir = role_dir
        .parent()
        .unwrap_or_else(|| Path::new(""))
        .to_path_buf();
    let role_dir_name = role_dir
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("")
        .to_string();
    let metadata_path = roles_dir.join(format!("{role_dir_name}.xml"));
    let configuration_path = roles_dir
        .parent()
        .unwrap_or_else(|| Path::new(""))
        .join("Configuration.xml");
    RoleReadLayout {
        role_dir_name,
        metadata_path,
        configuration_path,
    }
}

pub(crate) fn role_read_format_dependency_paths(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
    operation: &str,
) -> Result<Vec<PathBuf>, String> {
    let rights_path = resolve_role_read_rights_path(args, context)?;
    let layout = role_read_layout(&rights_path);

    let mut paths = vec![rights_path];
    if layout.metadata_path.is_file() {
        paths.push(layout.metadata_path);
    }
    if operation == "role-validate" && layout.configuration_path.is_file() {
        paths.push(layout.configuration_path);
    }
    Ok(paths)
}

/// Typed answer of `unica.role.info` (ADR-0023). Denied rights are always
/// present: hiding them behind a flag made "no denied rights" and "you did not
/// ask" the same observation.
#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RoleInfoData {
    pub(crate) name: String,
    pub(crate) synonym: Option<String>,
    pub(crate) support: DomainObjectSupportData,
    pub(crate) defaults: RoleDefaultsData,
    pub(crate) allowed: Vec<RoleGroupData>,
    pub(crate) denied: Vec<RoleGroupData>,
    pub(crate) totals: RoleTotalsData,
    pub(crate) restricted_objects: Vec<String>,
    pub(crate) templates: Vec<String>,
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RoleDefaultsData {
    pub(crate) set_for_new_objects: Option<String>,
    pub(crate) set_for_attributes_by_default: Option<String>,
    pub(crate) independent_rights_of_child_objects: Option<String>,
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RoleGroupData {
    pub(crate) kind: String,
    pub(crate) objects: Vec<RoleObjectData>,
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RoleObjectData {
    pub(crate) name: String,
    pub(crate) rights: Vec<RoleRightData>,
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RoleRightData {
    pub(crate) name: String,
    /// Row-level security restricts this right on this object.
    pub(crate) restricted: bool,
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RoleTotalsData {
    pub(crate) allowed: usize,
    pub(crate) denied: usize,
}

pub(crate) struct RoleInfoExecution {
    pub(crate) outcome: AdapterOutcome,
    pub(crate) data: Option<RoleInfoData>,
}

fn role_group_data(groups: Vec<RoleInfoGroup>) -> Vec<RoleGroupData> {
    groups
        .into_iter()
        .map(|group| RoleGroupData {
            kind: group.type_prefix,
            objects: group
                .objects
                .into_iter()
                .map(|object| RoleObjectData {
                    name: object.short_name,
                    rights: object
                        .rights
                        .into_iter()
                        .map(|right| RoleRightData {
                            name: right.name,
                            restricted: right.rls,
                        })
                        .collect(),
                })
                .collect(),
        })
        .collect()
}

fn role_attribute(value: &str) -> Option<String> {
    (!value.is_empty()).then(|| value.to_string())
}

pub(crate) fn analyze_role_info(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
    support_reader: &dyn SupportStateReader,
) -> RoleInfoExecution {
    let result = (|| -> Result<(RoleInfoData, PathBuf), String> {
        let selection = resolve_role_info_target(args, context)?;
        let rights_path = selection.resource_path;
        if !rights_path.is_file() {
            return Err(format!("[ERROR] File not found: {}", rights_path.display()));
        }

        let (role_name, role_synonym) = role_info_metadata(&rights_path);
        let rights_text = fs::read_to_string(&rights_path)
            .map_err(|err| format!("failed to read {}: {err}", rights_path.display()))?;
        let doc = Document::parse(rights_text.trim_start_matches('\u{feff}'))
            .map_err(|err| format!("XML parse error in {}: {err}", rights_path.display()))?;
        let root = doc.root_element();

        let set_for_new = root.attribute("setForNewObjects").unwrap_or("");
        let set_for_attrs = root.attribute("setForAttributesByDefault").unwrap_or("");
        let independent_child = root
            .attribute("independentRightsOfChildObjects")
            .unwrap_or("");

        let mut allowed = Vec::<RoleInfoGroup>::new();
        let mut denied = Vec::<RoleInfoGroup>::new();
        let mut rls_objects = Vec::<String>::new();
        let mut total_allowed = 0usize;
        let mut total_denied = 0usize;

        for obj in root
            .children()
            .filter(|node| role_info_element(*node, "object", Some("http://v8.1c.ru/8.2/roles")))
        {
            let mut obj_name = String::new();
            let mut rights = Vec::<RoleRight>::new();

            for child in obj.children().filter(|node| node.is_element()) {
                if role_info_element(child, "name", Some("http://v8.1c.ru/8.2/roles")) {
                    obj_name = child.text().unwrap_or("").to_string();
                }
                if role_info_element(child, "right", Some("http://v8.1c.ru/8.2/roles")) {
                    let mut right_name = String::new();
                    let mut right_value = String::new();
                    let mut has_rls = false;
                    for rc in child.children().filter(|node| node.is_element()) {
                        match rc.tag_name().name() {
                            "name" => right_name = rc.text().unwrap_or("").to_string(),
                            "value" => right_value = rc.text().unwrap_or("").to_string(),
                            "restrictionByCondition" => has_rls = true,
                            _ => {}
                        }
                    }
                    if !right_name.is_empty() && !right_value.is_empty() {
                        rights.push(RoleRight {
                            name: right_name,
                            value: right_value,
                            condition: has_rls.then(String::new),
                        });
                    }
                }
            }

            if obj_name.is_empty() || rights.is_empty() {
                continue;
            }
            let Some(dot_idx) = obj_name.find('.') else {
                continue;
            };
            let type_prefix = &obj_name[..dot_idx];
            let short_name = &obj_name[dot_idx + 1..];

            for right in rights {
                if right.value == "true" {
                    total_allowed += 1;
                    if right.condition.is_some() {
                        rls_objects.push(format!("{type_prefix}.{short_name} ({})", right.name));
                    }
                    add_role_info_right(
                        &mut allowed,
                        type_prefix,
                        short_name,
                        RoleInfoRightSummary {
                            name: right.name,
                            rls: right.condition.is_some(),
                        },
                    );
                } else {
                    total_denied += 1;
                    add_role_info_right(
                        &mut denied,
                        type_prefix,
                        short_name,
                        RoleInfoRightSummary {
                            name: right.name,
                            rls: false,
                        },
                    );
                }
            }
        }

        let mut templates = Vec::<String>::new();
        for template in root.children().filter(|node| {
            role_info_element(
                *node,
                "restrictionTemplate",
                Some("http://v8.1c.ru/8.2/roles"),
            )
        }) {
            for child in template.children().filter(|node| node.is_element()) {
                if child.tag_name().name() == "name" {
                    let mut name = child.text().unwrap_or("").to_string();
                    if let Some(paren_idx) = name.find('(') {
                        if paren_idx > 0 {
                            name.truncate(paren_idx);
                        }
                    }
                    templates.push(name);
                }
            }
        }

        let data = RoleInfoData {
            name: role_name,
            synonym: (!role_synonym.is_empty()).then_some(role_synonym),
            support: support_reader
                .object_support(&selection.target)
                .map_err(|error| error.to_string())?,
            defaults: RoleDefaultsData {
                set_for_new_objects: role_attribute(set_for_new),
                set_for_attributes_by_default: role_attribute(set_for_attrs),
                independent_rights_of_child_objects: role_attribute(independent_child),
            },
            allowed: role_group_data(allowed),
            // `ShowDenied` used to gate this list, so an empty answer could
            // mean "none" or "not asked for". Data always carries both.
            denied: role_group_data(denied),
            totals: RoleTotalsData {
                allowed: total_allowed,
                denied: total_denied,
            },
            restricted_objects: rls_objects,
            templates,
        };
        Ok((data, rights_path))
    })();

    match result {
        Ok((data, rights_path)) => RoleInfoExecution {
            outcome: AdapterOutcome {
                ok: true,
                summary: format!(
                    "unica.role.info described {} with {} allowed and {} denied right(s)",
                    data.name, data.totals.allowed, data.totals.denied
                ),
                changes: Vec::new(),
                warnings: Vec::new(),
                errors: Vec::new(),
                artifacts: vec![rights_path.display().to_string()],
                stdout: None,
                stderr: Some(String::new()),
                command: None,
            },
            data: Some(data),
        },
        Err(error) => RoleInfoExecution {
            outcome: AdapterOutcome {
                ok: false,
                summary: "unica.role.info failed in native role analyzer".to_string(),
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

pub(crate) fn role_info_metadata(rights_path: &Path) -> (String, String) {
    let layout = role_read_layout(rights_path);
    let role_folder_name = layout.role_dir_name;
    let meta_path = layout.metadata_path;

    let mut role_name = String::new();
    let mut role_synonym = String::new();
    if meta_path.is_file() {
        if let Ok(meta_text) = fs::read_to_string(&meta_path) {
            if let Ok(meta_doc) = Document::parse(meta_text.trim_start_matches('\u{feff}')) {
                for role in meta_doc
                    .descendants()
                    .filter(|node| role_info_element(*node, "Role", None))
                {
                    for props in role
                        .children()
                        .filter(|node| role_info_element(*node, "Properties", None))
                    {
                        if role_name.is_empty() {
                            role_name = props
                                .children()
                                .find(|node| role_info_element(*node, "Name", None))
                                .and_then(|node| node.text())
                                .unwrap_or("")
                                .to_string();
                        }
                        if role_synonym.is_empty() {
                            for synonym in props
                                .children()
                                .filter(|node| role_info_element(*node, "Synonym", None))
                            {
                                for item in synonym
                                    .children()
                                    .filter(|node| role_info_element(*node, "item", None))
                                {
                                    let lang = item
                                        .children()
                                        .find(|node| role_info_element(*node, "lang", None))
                                        .and_then(|node| node.text())
                                        .unwrap_or("");
                                    if lang == "ru" {
                                        role_synonym = item
                                            .children()
                                            .find(|node| role_info_element(*node, "content", None))
                                            .and_then(|node| node.text())
                                            .unwrap_or("")
                                            .to_string();
                                        break;
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    if role_name.is_empty() {
        role_name = role_folder_name;
    }

    (role_name, role_synonym)
}

pub(crate) fn role_info_element(
    node: roxmltree::Node<'_, '_>,
    local_name: &str,
    namespace: Option<&str>,
) -> bool {
    node.is_element()
        && node.tag_name().name() == local_name
        && namespace
            .map(|expected| node.tag_name().namespace() == Some(expected))
            .unwrap_or(true)
}

pub(crate) struct RoleValidationReport {
    pub(crate) lines: Vec<String>,
    pub(crate) errors: usize,
    pub(crate) warnings: usize,
    pub(crate) ok_count: usize,
    pub(crate) detailed: bool,
}

impl RoleValidationReport {
    pub(crate) fn new(detailed: bool) -> Self {
        Self {
            lines: Vec::new(),
            errors: 0,
            warnings: 0,
            ok_count: 0,
            detailed,
        }
    }

    pub(crate) fn ok(&mut self, msg: impl AsRef<str>) {
        self.ok_count += 1;
        if self.detailed {
            self.lines.push(format!("[OK]    {}", msg.as_ref()));
        }
    }

    pub(crate) fn warn(&mut self, msg: impl AsRef<str>) {
        self.warnings += 1;
        self.lines.push(format!("[WARN]  {}", msg.as_ref()));
    }

    pub(crate) fn error(&mut self, msg: impl AsRef<str>) {
        self.errors += 1;
        self.lines.push(format!("[ERROR] {}", msg.as_ref()));
    }

    pub(crate) fn finish(mut self, role_name: &str) -> String {
        self.lines
            .insert(0, format!("=== Validation: Role.{role_name} ==="));
        let checks = self.ok_count + self.errors + self.warnings;
        if self.errors == 0 && self.warnings == 0 && !self.detailed {
            format!("=== Validation OK: Role.{role_name} ({checks} checks) ===")
        } else {
            self.lines.push(String::new());
            self.lines.push(format!(
                "=== Result: {} errors, {} warnings ({checks} checks) ===",
                self.errors, self.warnings
            ));
            self.lines.join("\n")
        }
    }
}

pub(crate) fn validate_role(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> AdapterOutcome {
    let result = (|| -> Result<(bool, String, PathBuf), String> {
        let rights_path = resolve_role_read_rights_path(args, context)?;
        let detailed = bool_arg(args, &["detailed", "Detailed"]);

        let layout = role_read_layout(&rights_path);
        let metadata_path = layout.metadata_path;

        let mut report = RoleValidationReport::new(detailed);
        if !rights_path.exists() {
            report.error(format!("File not found: {}", rights_path.display()));
            let text = report.lines.join("\n");
            return Ok((false, text, rights_path));
        }

        let rights_text = fs::read_to_string(&rights_path)
            .map_err(|err| format!("failed to read {}: {err}", rights_path.display()))?;
        let doc = match Document::parse(rights_text.trim_start_matches('\u{feff}')) {
            Ok(doc) => {
                report.ok("XML well-formed");
                doc
            }
            Err(err) => {
                report.error(format!("XML parse error: {err}"));
                let text = report.lines.join("\n");
                return Ok((false, text, rights_path));
            }
        };

        let root = doc.root_element();
        let root_local = root.tag_name().name();
        let root_ns = root.tag_name().namespace().unwrap_or("");
        const RIGHTS_NS: &str = "http://v8.1c.ru/8.2/roles";

        if root_local != "Rights" {
            report.error(format!("Root element is '{root_local}', expected 'Rights'"));
        } else if root_ns != RIGHTS_NS {
            report.warn(format!("Namespace is '{root_ns}', expected '{RIGHTS_NS}'"));
        } else {
            report.ok("Root element: <Rights> with correct namespace");
        }

        let mut flags_found = 0usize;
        for flag in [
            "setForNewObjects",
            "setForAttributesByDefault",
            "independentRightsOfChildObjects",
        ] {
            if let Some(node) = root
                .children()
                .find(|node| role_info_element(*node, flag, Some(RIGHTS_NS)))
            {
                let value = node.text().unwrap_or("");
                if value != "true" && value != "false" {
                    report.warn(format!("{flag} = '{value}' (expected 'true' or 'false')"));
                }
                flags_found += 1;
            } else {
                report.warn(format!("Missing global flag: {flag}"));
            }
        }
        if flags_found == 3 {
            report.ok("3 global flags present");
        }

        let objects = root
            .children()
            .filter(|node| role_info_element(*node, "object", Some(RIGHTS_NS)))
            .collect::<Vec<_>>();
        let mut right_count = 0usize;
        let mut rls_count = 0usize;

        for obj in &objects {
            let mut obj_name = "";
            for child in obj.children().filter(|node| node.is_element()) {
                if role_info_element(child, "name", Some(RIGHTS_NS)) {
                    obj_name = child.text().unwrap_or("");
                    break;
                }
            }

            if obj_name.is_empty() {
                report.error("Object without <name>");
                continue;
            }

            let object_type = role_validate_object_type(obj_name);
            let is_nested = obj_name.matches('.').count() >= 2;
            if !is_nested && role_validate_known_rights(object_type).is_empty() {
                report.warn(format!("{obj_name}: unknown object type '{object_type}'"));
            }

            for child in obj.children().filter(|node| node.is_element()) {
                if !role_info_element(child, "right", Some(RIGHTS_NS)) {
                    continue;
                }

                let mut right_name = "";
                let mut right_value = "";
                for rc in child.children().filter(|node| node.is_element()) {
                    if rc.tag_name().namespace() != Some(RIGHTS_NS) {
                        continue;
                    }
                    match rc.tag_name().name() {
                        "name" => right_name = rc.text().unwrap_or(""),
                        "value" => right_value = rc.text().unwrap_or(""),
                        "restrictionByCondition" => {
                            rls_count += 1;
                            let cond_node = rc.children().find(|node| {
                                role_info_element(*node, "condition", Some(RIGHTS_NS))
                            });
                            if cond_node
                                .and_then(|node| node.text())
                                .unwrap_or("")
                                .is_empty()
                            {
                                report.warn(format!(
                                    "{obj_name}: RLS condition for '{right_name}' is empty"
                                ));
                            }
                        }
                        _ => {}
                    }
                }

                if right_name.is_empty() {
                    report.error(format!("{obj_name}: <right> without <name>"));
                    continue;
                }
                if right_value != "true" && right_value != "false" {
                    report.error(format!(
                        "{obj_name}: right '{right_name}' has invalid value '{right_value}'"
                    ));
                    continue;
                }

                right_count += 1;
                if is_nested {
                    // О раскладке вне закрытой модели наблюдение не судит:
                    // иначе легальные адреса платформы получают ложное
                    // предупреждение (см. `nested_right_shape_is_modelled`).
                    if nested_right_shape_is_modelled(obj_name) {
                        if let Err(message) = validate_nested_right(obj_name, right_name) {
                            report.warn(message);
                        }
                    }
                } else {
                    let valid_rights = role_validate_known_rights(object_type);
                    if !valid_rights.is_empty() && !valid_rights.contains(&right_name) {
                        let similar = role_validate_find_similar(right_name, valid_rights);
                        let suggestion = if similar.is_empty() {
                            String::new()
                        } else {
                            format!(" Did you mean: {}?", similar.join(", "))
                        };
                        report.warn(format!(
                            "{obj_name}: unknown right '{right_name}'.{suggestion}"
                        ));
                    } else if !valid_rights.is_empty()
                        && right_value == "true"
                        && right_name.ends_with("PredefinedData")
                    {
                        report.warn(format!(
                            "{obj_name}: '{right_name}' = true grants interactive changes to predefined data (predefined data is part of the configuration and should not be available to end users)"
                        ));
                    }
                }
            }
        }

        report.ok(format!("{} objects, {right_count} rights", objects.len()));
        if rls_count > 0 {
            report.ok(format!("{rls_count} RLS restrictions"));
        }

        let templates = root
            .children()
            .filter(|node| role_info_element(*node, "restrictionTemplate", Some(RIGHTS_NS)))
            .collect::<Vec<_>>();
        if !templates.is_empty() {
            let mut template_names = Vec::<String>::new();
            for template in &templates {
                let mut template_name = "";
                let mut template_condition = "";
                for child in template.children().filter(|node| node.is_element()) {
                    if child.tag_name().namespace() != Some(RIGHTS_NS) {
                        continue;
                    }
                    match child.tag_name().name() {
                        "name" => template_name = child.text().unwrap_or(""),
                        "condition" => template_condition = child.text().unwrap_or(""),
                        _ => {}
                    }
                }
                if template_name.is_empty() {
                    report.warn("Restriction template without <name>");
                } else {
                    let short_name = template_name
                        .find('(')
                        .filter(|idx| *idx > 0)
                        .map(|idx| &template_name[..idx])
                        .unwrap_or(template_name);
                    template_names.push(short_name.to_string());
                }
                if template_condition.is_empty() {
                    report.warn(format!("Template '{template_name}': empty <condition>"));
                }
            }
            report.ok(format!(
                "{} templates: {}",
                templates.len(),
                template_names.join(", ")
            ));
        }

        let mut inferred_role_name = String::new();
        if metadata_path.is_file() {
            report.lines.push(String::new());
            match fs::read_to_string(&metadata_path) {
                Ok(meta_text) => match Document::parse(meta_text.trim_start_matches('\u{feff}')) {
                    Ok(meta_doc) => {
                        if let Some(role_node) = meta_doc
                            .descendants()
                            .find(|node| role_info_element(*node, "Role", None))
                        {
                            let uuid_val = role_node.attribute("uuid").unwrap_or("");
                            if is_valid_uuid(uuid_val) {
                                report.ok(format!("Metadata: UUID valid ({uuid_val})"));
                            } else {
                                report.error(format!("Metadata: invalid UUID format '{uuid_val}'"));
                            }

                            let name_node = role_node
                                .descendants()
                                .find(|node| role_info_element(*node, "Name", None));
                            if let Some(name_text) = name_node.and_then(|node| node.text()) {
                                if !name_text.is_empty() {
                                    report.ok(format!("Metadata: Name = {name_text}"));
                                    inferred_role_name = name_text.to_string();
                                } else {
                                    report.error("Metadata: <Name> is empty or missing");
                                }
                            } else {
                                report.error("Metadata: <Name> is empty or missing");
                            }

                            let syn_node = role_node
                                .descendants()
                                .find(|node| role_info_element(*node, "Synonym", None));
                            if syn_node
                                .map(|node| node.children().any(|child| child.is_element()))
                                .unwrap_or(false)
                            {
                                report.ok("Metadata: Synonym present");
                            } else {
                                report.warn("Metadata: <Synonym> is empty");
                            }
                        } else {
                            report.error("Metadata: <Role> element not found");
                        }
                    }
                    Err(err) => report.error(format!("Metadata XML parse error: {err}")),
                },
                Err(err) => report.error(format!("Metadata XML parse error: {err}")),
            }
        }

        let config_xml_path = layout.configuration_path;
        if inferred_role_name.is_empty() {
            inferred_role_name = layout.role_dir_name;
        }

        if config_xml_path.exists() {
            report.lines.push(String::new());
            match fs::read_to_string(&config_xml_path) {
                Ok(config_text) => {
                    match Document::parse(config_text.trim_start_matches('\u{feff}')) {
                        Ok(cfg_doc) => {
                            if let Some(child_obj) = cfg_doc.descendants().find(|node| {
                                role_info_element(
                                    *node,
                                    "ChildObjects",
                                    Some("http://v8.1c.ru/8.3/MDClasses"),
                                ) && node.ancestors().any(|ancestor| {
                                    role_info_element(
                                        ancestor,
                                        "Configuration",
                                        Some("http://v8.1c.ru/8.3/MDClasses"),
                                    )
                                })
                            }) {
                                let found = child_obj.children().any(|node| {
                                    role_info_element(
                                        node,
                                        "Role",
                                        Some("http://v8.1c.ru/8.3/MDClasses"),
                                    ) && node.text().unwrap_or("") == inferred_role_name
                                });
                                if found {
                                    report.ok(format!(
                                    "Configuration.xml: <Role>{inferred_role_name}</Role> registered"
                                ));
                                } else {
                                    report.warn(format!(
                                    "Configuration.xml: <Role>{inferred_role_name}</Role> NOT found in ChildObjects"
                                ));
                                }
                            }
                        }
                        Err(err) => report.warn(format!("Configuration.xml: parse error — {err}")),
                    }
                }
                Err(err) => report.warn(format!("Configuration.xml: parse error — {err}")),
            }
        }

        let ok = report.errors == 0;
        let text = report.finish(&inferred_role_name);
        Ok((ok, text, rights_path))
    })();

    match result {
        Ok((ok, text, rights_path)) => AdapterOutcome {
            ok,
            summary: if ok {
                "unica.role.validate completed with native role validator".to_string()
            } else {
                "unica.role.validate failed in native role validator".to_string()
            },
            changes: Vec::new(),
            warnings: Vec::new(),
            errors: if ok {
                Vec::new()
            } else {
                vec![text.trim().to_string()]
            },
            artifacts: vec![rights_path.display().to_string()],
            stdout: Some(format!("{text}\n")),
            stderr: Some(String::new()),
            command: None,
        },
        Err(error) => AdapterOutcome {
            ok: false,
            summary: "unica.role.validate failed in native role validator".to_string(),
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

pub(crate) fn role_validate_object_type(name: &str) -> &str {
    name.split_once('.')
        .map(|(prefix, _)| prefix)
        .unwrap_or(name)
}

pub(crate) fn role_validate_find_similar(needle: &str, haystack: &[&str]) -> Vec<String> {
    let needle_lower = needle.to_lowercase();
    let mut result = Vec::new();
    for item in haystack {
        let item_lower = item.to_lowercase();
        if needle_lower.contains(&item_lower) || item_lower.contains(&needle_lower) {
            result.push((*item).to_string());
        }
        if result.len() >= 3 {
            break;
        }
    }
    result
}

pub(crate) fn role_validate_known_rights(object_type: &str) -> &'static [&'static str] {
    known_role_rights(object_type)
}

struct RoleCompileResult {
    stdout: String,
    stderr: String,
    artifacts: Vec<PathBuf>,
    changes: Vec<String>,
    warnings: Vec<String>,
}

const ROLE_RIGHTS_NAMESPACE: &str = "http://v8.1c.ru/8.2/roles";
const ROLE_METADATA_NAMESPACE: &str = "http://v8.1c.ru/8.3/MDClasses";
const XSI_NAMESPACE: &str = "http://www.w3.org/2001/XMLSchema-instance";

fn validate_role_compile_name(value: &str) -> Result<(), String> {
    let mut components = Path::new(value).components();
    let is_single_path_component = matches!(
        components.next(),
        Some(std::path::Component::Normal(component))
            if component == std::ffi::OsStr::new(value)
    ) && components.next().is_none();

    if form_is_xml_ncname(value) && is_single_path_component {
        Ok(())
    } else {
        Err(format!(
            "Role name must be a valid Unicode XML NCName and a single path component: {value:?}"
        ))
    }
}

fn role_compile_json_bool(definition: &Value, field: &str, default: bool) -> Result<bool, String> {
    match definition.get(field) {
        None => Ok(default),
        Some(Value::Bool(value)) => Ok(*value),
        Some(value) => Err(format!(
            "role.compile field '{field}' must be a JSON boolean true or false; got {value}"
        )),
    }
}

fn validate_compiled_role_rights_xml(xml: &str, format_version: &str) -> Result<(), String> {
    let doc = Document::parse(xml.trim_start_matches('\u{feff}'))
        .map_err(|error| format!("Rights XML parse error: {error}"))?;
    let root = doc.root_element();
    if root.tag_name().name() != "Rights"
        || root.tag_name().namespace() != Some(ROLE_RIGHTS_NAMESPACE)
    {
        return Err(format!(
            "Rights root must be {{{ROLE_RIGHTS_NAMESPACE}}}Rights, got {{{}}}{}",
            root.tag_name().namespace().unwrap_or(""),
            root.tag_name().name()
        ));
    }
    if root.attribute("version") != Some(format_version) {
        return Err(format!(
            "Rights version must be {format_version:?}, got {:?}",
            root.attribute("version")
        ));
    }

    for flag in [
        "setForNewObjects",
        "setForAttributesByDefault",
        "independentRightsOfChildObjects",
    ] {
        let nodes = root
            .children()
            .filter(|node| {
                node.is_element()
                    && node.tag_name().name() == flag
                    && node.tag_name().namespace() == Some(ROLE_RIGHTS_NAMESPACE)
            })
            .collect::<Vec<_>>();
        if nodes.len() != 1 {
            return Err(format!(
                "Rights must contain exactly one <{flag}> element, found {}",
                nodes.len()
            ));
        }
        let value = nodes[0].text().unwrap_or("");
        if !matches!(value, "true" | "false") {
            return Err(format!(
                "Rights <{flag}> must contain an xs:boolean true or false, got {value:?}"
            ));
        }
    }

    for right in root.descendants().filter(|node| {
        node.is_element()
            && node.tag_name().name() == "right"
            && node.tag_name().namespace() == Some(ROLE_RIGHTS_NAMESPACE)
    }) {
        let values = right
            .children()
            .filter(|node| {
                node.is_element()
                    && node.tag_name().name() == "value"
                    && node.tag_name().namespace() == Some(ROLE_RIGHTS_NAMESPACE)
            })
            .collect::<Vec<_>>();
        if values.len() != 1 || !matches!(values[0].text().unwrap_or(""), "true" | "false") {
            return Err(
                "every Rights <right> must contain exactly one xs:boolean <value>".to_string(),
            );
        }
    }

    Ok(())
}

fn validate_compiled_role_metadata_xml(
    xml: &str,
    role_name: &str,
    format_version: &str,
) -> Result<(), String> {
    let doc = Document::parse(xml.trim_start_matches('\u{feff}'))
        .map_err(|error| format!("role metadata XML parse error: {error}"))?;
    let root = doc.root_element();
    if root.tag_name().name() != "MetaDataObject"
        || root.tag_name().namespace() != Some(ROLE_METADATA_NAMESPACE)
    {
        return Err(format!(
            "role metadata root must be {{{ROLE_METADATA_NAMESPACE}}}MetaDataObject, got {{{}}}{}",
            root.tag_name().namespace().unwrap_or(""),
            root.tag_name().name()
        ));
    }
    if root.attribute("version") != Some(format_version) {
        return Err(format!(
            "role metadata version must be {format_version:?}, got {:?}",
            root.attribute("version")
        ));
    }

    let roles = root
        .children()
        .filter(|node| {
            node.is_element()
                && node.tag_name().name() == "Role"
                && node.tag_name().namespace() == Some(ROLE_METADATA_NAMESPACE)
        })
        .collect::<Vec<_>>();
    if roles.len() != 1 {
        return Err(format!(
            "role metadata must contain exactly one <Role>, found {}",
            roles.len()
        ));
    }
    let role = roles[0];
    let uuid = role.attribute("uuid").unwrap_or("");
    if !is_valid_uuid(uuid) {
        return Err(format!("role metadata has invalid UUID {uuid:?}"));
    }
    let names = role
        .children()
        .filter(|node| {
            node.is_element()
                && node.tag_name().name() == "Properties"
                && node.tag_name().namespace() == Some(ROLE_METADATA_NAMESPACE)
        })
        .flat_map(|properties| properties.children())
        .filter(|node| {
            node.is_element()
                && node.tag_name().name() == "Name"
                && node.tag_name().namespace() == Some(ROLE_METADATA_NAMESPACE)
        })
        .collect::<Vec<_>>();
    if names.len() != 1 || names[0].text().unwrap_or("") != role_name {
        return Err(format!(
            "role metadata <Name> must be {role_name:?}, got {:?}",
            names.first().and_then(|node| node.text())
        ));
    }

    Ok(())
}

#[cfg(test)]
std::thread_local! {
    static TEST_ROLE_COMPILE_POST_VALIDATION_FAILURE: std::cell::Cell<bool> = const {
        std::cell::Cell::new(false)
    };
}

#[cfg(test)]
fn with_role_compile_post_validation_failure<T>(action: impl FnOnce() -> T) -> T {
    struct Reset(bool);
    impl Drop for Reset {
        fn drop(&mut self) {
            TEST_ROLE_COMPILE_POST_VALIDATION_FAILURE.with(|slot| slot.set(self.0));
        }
    }

    let previous = TEST_ROLE_COMPILE_POST_VALIDATION_FAILURE.with(|slot| slot.replace(true));
    let _reset = Reset(previous);
    action()
}

fn validate_role_compile_post_state(
    metadata_path: &Path,
    rights_path: &Path,
    role_name: &str,
    format_version: &str,
) -> Result<(), String> {
    #[cfg(test)]
    if TEST_ROLE_COMPILE_POST_VALIDATION_FAILURE.with(|slot| slot.get()) {
        return Err("injected role semantic post-validation failure".to_string());
    }

    let metadata = fs::read_to_string(metadata_path)
        .map_err(|error| format!("failed to read {}: {error}", metadata_path.display()))?;
    validate_compiled_role_metadata_xml(&metadata, role_name, format_version)?;
    let rights = fs::read_to_string(rights_path)
        .map_err(|error| format!("failed to read {}: {error}", rights_path.display()))?;
    validate_compiled_role_rights_xml(&rights, format_version)
}

fn require_role_configuration_owner_validation(
    config_path: &Path,
    context: &WorkspaceContext,
) -> Result<(), String> {
    validate_cf_owner_path(config_path, context).map_err(|detail| {
        format!(
            "role.compile Configuration owner validation failed for {}: {}",
            config_path.display(),
            detail.trim()
        )
    })
}

pub(crate) fn compile_role(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> AdapterOutcome {
    compile_role_internal(args, context, false)
}

pub(crate) fn preview_role_compile(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> Result<AdapterOutcome, String> {
    let outcome = compile_role_internal(args, context, true);
    if outcome.ok {
        Ok(outcome)
    } else {
        Err(outcome.errors.join("; "))
    }
}

fn compile_role_internal(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
    dry_run: bool,
) -> AdapterOutcome {
    let write_result = (|| -> Result<RoleCompileResult, String> {
        let json_path_raw = required_path(args, &["jsonPath", "JsonPath"], "JsonPath")?;
        let json_path = absolutize(json_path_raw, &context.cwd);
        if !json_path.exists() {
            return Err(format!("File not found: {}", json_path.display()));
        }
        let mut transaction = CompileTransaction::new();
        let mut defn = FileBackedJson::read(&json_path, |err| {
            format!("failed to parse role JSON: {err}")
        })?
        .bind_to(&mut transaction)?;

        let role_name = defn
            .get("name")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned)
            .ok_or_else(|| "JSON must have 'name' field (role programmatic name)".to_string())?;
        validate_role_compile_name(&role_name)?;
        let sfno = role_compile_json_bool(&defn, "setForNewObjects", false)?.to_string();
        let sfab = role_compile_json_bool(&defn, "setForAttributesByDefault", true)?.to_string();
        let irco =
            role_compile_json_bool(&defn, "independentRightsOfChildObjects", false)?.to_string();
        let synonym = json_string_field(&defn, "synonym").unwrap_or_else(|| role_name.clone());
        let comment = json_string_field(&defn, "comment").unwrap_or_default();

        if !truthy_json_field(&defn, "objects") && truthy_json_field(&defn, "rights") {
            let rights = defn.get("rights").cloned().unwrap_or(Value::Null);
            if let Some(object) = defn.as_object_mut() {
                object.insert("objects".to_string(), rights);
            }
        }

        let output_dir_raw = required_path(args, &["outputDir", "OutputDir"], "OutputDir")?;
        let output_dir = absolutize(output_dir_raw.clone(), &context.cwd);
        let format_version = detect_format_version(&output_dir, context)?.to_string();
        let mut stderr = String::new();
        let mut parsed_objects = Vec::<RoleObject>::new();
        if let Some(objects) = defn.get("objects").and_then(Value::as_array) {
            for entry in objects {
                if let Some(parsed) = parse_role_object_entry(entry, &mut stderr) {
                    parsed_objects.push(parsed);
                }
            }
        }

        let mut rights_lines = Vec::<String>::new();
        rights_lines.push("<?xml version=\"1.0\" encoding=\"UTF-8\"?>".to_string());
        rights_lines.push("<Rights xmlns=\"http://v8.1c.ru/8.2/roles\"".to_string());
        rights_lines.push("        xmlns:xs=\"http://www.w3.org/2001/XMLSchema\"".to_string());
        rights_lines
            .push("        xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\"".to_string());
        rights_lines.push(format!(
            "        xsi:type=\"Rights\" version=\"{format_version}\">"
        ));
        rights_lines.push(format!("    <setForNewObjects>{sfno}</setForNewObjects>"));
        rights_lines.push(format!(
            "    <setForAttributesByDefault>{sfab}</setForAttributesByDefault>"
        ));
        rights_lines.push(format!(
            "    <independentRightsOfChildObjects>{irco}</independentRightsOfChildObjects>"
        ));

        let mut total_rights = 0usize;
        for object in &parsed_objects {
            rights_lines.push("    <object>".to_string());
            rights_lines.push(format!("        <name>{}</name>", escape_xml(&object.name)));
            for right in &object.rights {
                rights_lines.push("        <right>".to_string());
                rights_lines.push(format!(
                    "            <name>{}</name>",
                    escape_xml(&right.name)
                ));
                rights_lines.push(format!("            <value>{}</value>", right.value));
                if let Some(condition) = &right.condition {
                    rights_lines.push("            <restrictionByCondition>".to_string());
                    rights_lines.push(format!(
                        "                <condition>{}</condition>",
                        escape_xml(condition)
                    ));
                    rights_lines.push("            </restrictionByCondition>".to_string());
                }
                rights_lines.push("        </right>".to_string());
                total_rights += 1;
            }
            rights_lines.push("    </object>".to_string());
        }

        let mut template_count = 0usize;
        if let Some(templates) = defn.get("templates").and_then(Value::as_array) {
            for template in templates {
                rights_lines.push("    <restrictionTemplate>".to_string());
                rights_lines.push(format!(
                    "        <name>{}</name>",
                    escape_xml(&json_string_field(template, "name").unwrap_or_default())
                ));
                rights_lines.push(format!(
                    "        <condition>{}</condition>",
                    escape_xml(&json_string_field(template, "condition").unwrap_or_default())
                ));
                rights_lines.push("    </restrictionTemplate>".to_string());
                template_count += 1;
            }
        }
        rights_lines.push("</Rights>".to_string());
        let rights_xml = format!("{}\n", rights_lines.join("\n"));

        let leaf = output_dir
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or_default();
        let (roles_dir, config_dir) = if leaf == "Roles" {
            (
                output_dir.clone(),
                output_dir
                    .parent()
                    .map(Path::to_path_buf)
                    .unwrap_or_else(|| context.cwd.clone()),
            )
        } else {
            (output_dir.join("Roles"), output_dir.clone())
        };

        let metadata_path = roles_dir.join(format!("{role_name}.xml"));
        let rights_path = roles_dir.join(&role_name).join("Ext").join("Rights.xml");
        match fs::symlink_metadata(&metadata_path) {
            Ok(metadata) if metadata.is_file() && !metadata.file_type().is_symlink() => {
                let message = format!(
                    "[SKIP] Role '{role_name}' already exists at {}; no files changed\n",
                    metadata_path.display()
                );
                return Ok(RoleCompileResult {
                    stdout: message,
                    stderr,
                    artifacts: Vec::new(),
                    changes: Vec::new(),
                    warnings: Vec::new(),
                });
            }
            Ok(_) => {
                return Err(format!(
                    "existing role target is not a regular file: {}",
                    metadata_path.display()
                ));
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(format!(
                    "failed to inspect role target {}: {error}",
                    metadata_path.display()
                ));
            }
        }
        let config_xml_path = config_dir.join("Configuration.xml");
        let config_owner_exists = config_xml_path.is_file();
        #[cfg(test)]
        run_role_compile_after_configuration_probe_hook(&config_xml_path);
        if config_owner_exists {
            require_role_configuration_owner_validation(&config_xml_path, context)?;
        }
        let uid = fresh_metadata_uuid();
        let metadata_xml = role_metadata_xml(&role_name, &synonym, &comment, &format_version, &uid);
        validate_compiled_role_metadata_xml(&metadata_xml, &role_name, &format_version)?;
        validate_compiled_role_rights_xml(&rights_xml, &format_version)?;
        transaction.create_utf8_bom_text(&metadata_path, &metadata_xml)?;
        transaction.create_utf8_bom_text(&rights_path, &rights_xml)?;

        let reg_result =
            transaction.register_canonical_child(&config_xml_path, "Role", &role_name)?;
        let config_owner_registered = !matches!(reg_result, RegistrationStatus::MissingTarget);
        guard_active_format_owner(&mut transaction, &metadata_path, context)?;
        guard_active_format_owner(&mut transaction, &config_xml_path, context)?;

        let mut stdout = format!(
            "[OK] Role '{role_name}' compiled\n     UUID: {uid}\n     Metadata: {}\n     Rights:   {}\n     Objects: {}, Rights: {total_rights}, Templates: {template_count}\n",
            metadata_path.display(),
            rights_path.display(),
            parsed_objects.len()
        );
        match reg_result {
            RegistrationStatus::Added => stdout.push_str(&format!(
                "     Configuration.xml: <Role>{role_name}</Role> added to ChildObjects\n"
            )),
            RegistrationStatus::AlreadyPresent => stdout.push_str(&format!(
                "     Configuration.xml: <Role>{role_name}</Role> already registered\n"
            )),
            RegistrationStatus::MissingTarget => stderr.push_str(&format!(
                "WARNING: Configuration.xml not found at {} -- register manually\n",
                config_xml_path.display()
            )),
        }

        let (artifacts, changes, warnings, output) = if dry_run {
            if config_owner_registered {
                require_role_configuration_owner_validation(&config_xml_path, context)?;
            }
            (
                Vec::new(),
                transaction.dry_run_changes(),
                Vec::new(),
                transaction.dry_run_stdout(),
            )
        } else {
            let report = transaction.commit_with_post_validation(|| {
                if config_owner_registered {
                    require_role_configuration_owner_validation(&config_xml_path, context)?;
                }
                validate_role_compile_post_state(
                    &metadata_path,
                    &rights_path,
                    &role_name,
                    &format_version,
                )
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
            (report.created, changes, report.cleanup_warnings, stdout)
        };

        Ok(RoleCompileResult {
            stdout: output,
            stderr,
            artifacts,
            changes,
            warnings,
        })
    })();

    match write_result {
        Ok(result) => AdapterOutcome {
            ok: true,
            summary: if dry_run {
                "dry run: unica.role.compile planned native role compilation".to_string()
            } else {
                "unica.role.compile completed with native role writer".to_string()
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
            stderr: (!result.stderr.is_empty()).then_some(result.stderr),
            command: None,
        },
        Err(error) => AdapterOutcome {
            ok: false,
            summary: "unica.role.compile failed in native role writer".to_string(),
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

pub(crate) fn role_metadata_xml(
    role_name: &str,
    synonym: &str,
    comment: &str,
    format_version: &str,
    uid: &str,
) -> String {
    let mut lines = Vec::<String>::new();
    lines.push("<?xml version=\"1.0\" encoding=\"UTF-8\"?>".to_string());
    lines.push("<MetaDataObject xmlns=\"http://v8.1c.ru/8.3/MDClasses\"".to_string());
    lines.push("        xmlns:app=\"http://v8.1c.ru/8.2/managed-application/core\"".to_string());
    lines.push(
        "        xmlns:cfg=\"http://v8.1c.ru/8.1/data/enterprise/current-config\"".to_string(),
    );
    lines.push("        xmlns:cmi=\"http://v8.1c.ru/8.2/managed-application/cmi\"".to_string());
    lines.push("        xmlns:ent=\"http://v8.1c.ru/8.1/data/enterprise\"".to_string());
    lines.push("        xmlns:lf=\"http://v8.1c.ru/8.2/managed-application/logform\"".to_string());
    lines.push("        xmlns:style=\"http://v8.1c.ru/8.1/data/ui/style\"".to_string());
    lines.push("        xmlns:sys=\"http://v8.1c.ru/8.1/data/ui/fonts/system\"".to_string());
    lines.push("        xmlns:v8=\"http://v8.1c.ru/8.1/data/core\"".to_string());
    lines.push("        xmlns:v8ui=\"http://v8.1c.ru/8.1/data/ui\"".to_string());
    lines.push("        xmlns:web=\"http://v8.1c.ru/8.1/data/ui/colors/web\"".to_string());
    lines.push("        xmlns:win=\"http://v8.1c.ru/8.1/data/ui/colors/windows\"".to_string());
    lines.push("        xmlns:xen=\"http://v8.1c.ru/8.3/xcf/enums\"".to_string());
    lines.push("        xmlns:xpr=\"http://v8.1c.ru/8.3/xcf/predef\"".to_string());
    lines.push("        xmlns:xr=\"http://v8.1c.ru/8.3/xcf/readable\"".to_string());
    lines.push("        xmlns:xs=\"http://www.w3.org/2001/XMLSchema\"".to_string());
    lines.push("        xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\"".to_string());
    lines.push(format!("        version=\"{format_version}\">"));
    lines.push(format!("    <Role uuid=\"{uid}\">"));
    lines.push("        <Properties>".to_string());
    lines.push(format!(
        "            <Name>{}</Name>",
        escape_xml(role_name)
    ));
    lines.push("            <Synonym>".to_string());
    lines.push("                <v8:item>".to_string());
    lines.push("                    <v8:lang>ru</v8:lang>".to_string());
    lines.push(format!(
        "                    <v8:content>{}</v8:content>",
        escape_xml(synonym)
    ));
    lines.push("                </v8:item>".to_string());
    lines.push("            </Synonym>".to_string());
    if comment.is_empty() {
        lines.push("            <Comment/>".to_string());
    } else {
        lines.push(format!(
            "            <Comment>{}</Comment>",
            escape_xml(comment)
        ));
    }
    lines.push("        </Properties>".to_string());
    lines.push("    </Role>".to_string());
    lines.push("</MetaDataObject>".to_string());
    format!("{}\n", lines.join("\n"))
}

pub(crate) fn parse_role_object_entry(entry: &Value, stderr: &mut String) -> Option<RoleObject> {
    if let Some(text) = entry.as_str() {
        let Some((object_name, rights_text)) = text.split_once(':') else {
            stderr.push_str(&format!(
                "WARNING: Invalid string '{text}' -- expected 'Object.Name: @preset' or 'Object.Name: Right1, Right2'\n"
            ));
            return None;
        };
        let object_name = translate_role_object_name(object_name.trim());
        let object_type = role_object_type(&object_name);
        let right_names = if rights_text.trim().starts_with('@') {
            role_preset_rights(&object_type, rights_text.trim(), stderr)
        } else {
            rights_text
                .split(',')
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(translate_role_right_name)
                .collect()
        };
        return Some(RoleObject {
            name: object_name,
            rights: right_names
                .into_iter()
                .map(|name| RoleRight {
                    name,
                    value: "true".to_string(),
                    condition: None,
                })
                .collect(),
        });
    }

    let Some(object) = entry.as_object() else {
        stderr.push_str("WARNING: Object entry missing 'name' field\n");
        return None;
    };
    let object_name = object
        .get("name")
        .map(json_value_to_python_string)
        .filter(|value| !value.is_empty());
    let Some(object_name) = object_name else {
        stderr.push_str("WARNING: Object entry missing 'name' field\n");
        return None;
    };
    let object_name = translate_role_object_name(&object_name);
    let object_type = role_object_type(&object_name);
    let mut rights_order = Vec::<String>::new();
    let mut rights_map = std::collections::BTreeMap::<String, RoleRight>::new();

    if let Some(preset) = object.get("preset").map(json_value_to_python_string) {
        for right_name in role_preset_rights(&object_type, &preset, stderr) {
            if !rights_map.contains_key(&right_name) {
                rights_order.push(right_name.clone());
            }
            rights_map.insert(
                right_name.clone(),
                RoleRight {
                    name: right_name,
                    value: "true".to_string(),
                    condition: None,
                },
            );
        }
    }

    if let Some(rights) = object.get("rights") {
        if let Some(items) = rights.as_array() {
            for right in items {
                let right_name = translate_role_right_name(right.to_string().trim_matches('"'));
                if !rights_map.contains_key(&right_name) {
                    rights_order.push(right_name.clone());
                }
                rights_map.insert(
                    right_name.clone(),
                    RoleRight {
                        name: right_name,
                        value: "true".to_string(),
                        condition: None,
                    },
                );
            }
        } else if let Some(items) = rights.as_object() {
            for (right_name, value) in items {
                let right_name = translate_role_right_name(right_name);
                if !rights_map.contains_key(&right_name) {
                    rights_order.push(right_name.clone());
                }
                let bool_value = if value.as_bool() == Some(true)
                    || value.as_str() == Some("True")
                    || value.as_str() == Some("true")
                {
                    "true"
                } else {
                    "false"
                };
                rights_map.insert(
                    right_name.clone(),
                    RoleRight {
                        name: right_name,
                        value: bool_value.to_string(),
                        condition: None,
                    },
                );
            }
        }
    }

    if let Some(rls) = object.get("rls").and_then(Value::as_object) {
        for (right_name, condition) in rls {
            let right_name = translate_role_right_name(right_name);
            if let Some(right) = rights_map.get_mut(&right_name) {
                right.condition = Some(json_value_to_python_string(condition));
            } else {
                stderr.push_str(&format!(
                    "WARNING: {object_name}: RLS for '{right_name}' but this right is not in the rights list\n"
                ));
            }
        }
    }

    Some(RoleObject {
        name: object_name,
        rights: rights_order
            .into_iter()
            .filter_map(|name| rights_map.remove(&name))
            .collect(),
    })
}

pub(crate) fn translate_role_object_name(name: &str) -> String {
    name.split('.')
        .map(|part| match part {
            "Справочник" => "Catalog",
            "Документ" => "Document",
            "РегистрСведений" => "InformationRegister",
            "РегистрНакопления" => "AccumulationRegister",
            "РегистрБухгалтерии" => "AccountingRegister",
            "РегистрРасчета" => "CalculationRegister",
            "Константа" => "Constant",
            "ПланСчетов" => "ChartOfAccounts",
            "ПланВидовХарактеристик" => "ChartOfCharacteristicTypes",
            "ПланВидовРасчета" => "ChartOfCalculationTypes",
            "ПланОбмена" => "ExchangePlan",
            "БизнесПроцесс" => "BusinessProcess",
            "Задача" => "Task",
            "Обработка" => "DataProcessor",
            "Отчет" => "Report",
            "ОбщаяФорма" => "CommonForm",
            "ОбщаяКоманда" => "CommonCommand",
            "Подсистема" => "Subsystem",
            "КритерийОтбора" => "FilterCriterion",
            "ЖурналДокументов" => "DocumentJournal",
            "Последовательность" => "Sequence",
            "ВебСервис" => "WebService",
            "HTTPСервис" => "HTTPService",
            "СервисИнтеграции" => "IntegrationService",
            "ПараметрСеанса" => "SessionParameter",
            "ОбщийРеквизит" => "CommonAttribute",
            "Конфигурация" => "Configuration",
            "Перечисление" => "Enum",
            "Реквизит" => "Attribute",
            "СтандартныйРеквизит" => "StandardAttribute",
            "ТабличнаяЧасть" => "TabularSection",
            "Измерение" => "Dimension",
            "Ресурс" => "Resource",
            "Команда" => "Command",
            "РеквизитАдресации" => "AddressingAttribute",
            other => other,
        })
        .collect::<Vec<_>>()
        .join(".")
}

pub(crate) fn translate_role_right_name(name: &str) -> String {
    match name {
        "Чтение" => "Read",
        "Добавление" => "Insert",
        "Изменение" => "Update",
        "Удаление" => "Delete",
        "Просмотр" => "View",
        "Редактирование" => "Edit",
        "ВводПоСтроке" => "InputByString",
        "Проведение" => "Posting",
        "ОтменаПроведения" => "UndoPosting",
        "Использование" => "Use",
        other => other,
    }
    .to_string()
}

pub(crate) fn role_object_type(object_name: &str) -> String {
    object_name
        .split_once('.')
        .map(|(object_type, _)| object_type.to_string())
        .unwrap_or_else(|| object_name.to_string())
}

pub(crate) fn role_preset_rights(
    object_type: &str,
    preset_name: &str,
    stderr: &mut String,
) -> Vec<String> {
    let preset = preset_name.trim_start_matches('@');
    match (preset, object_type) {
        ("view", "Catalog" | "ExchangePlan" | "Document" | "ChartOfAccounts")
        | ("view", "ChartOfCharacteristicTypes" | "ChartOfCalculationTypes")
        | ("view", "BusinessProcess" | "Task") => {
            vec!["Read", "View", "InputByString"]
        }
        ("view", "InformationRegister" | "AccumulationRegister" | "AccountingRegister")
        | ("view", "CalculationRegister" | "Constant" | "DocumentJournal") => vec!["Read", "View"],
        ("view", "CommonForm" | "CommonCommand" | "Subsystem" | "FilterCriterion") => {
            vec!["View"]
        }
        ("view", "DataProcessor" | "Report") => vec!["Use", "View"],
        ("view", "Configuration") => {
            vec!["ThinClient", "WebClient", "Output", "SaveUserData", "MainWindowModeNormal"]
        }
        ("edit", "Catalog" | "ExchangePlan" | "ChartOfAccounts")
        | ("edit", "ChartOfCharacteristicTypes" | "ChartOfCalculationTypes") => vec![
            "Read",
            "Insert",
            "Update",
            "Delete",
            "View",
            "Edit",
            "InputByString",
            "InteractiveInsert",
            "InteractiveSetDeletionMark",
            "InteractiveClearDeletionMark",
        ],
        ("edit", "Document") => vec![
            "Read",
            "Insert",
            "Update",
            "Delete",
            "View",
            "Edit",
            "InputByString",
            "Posting",
            "UndoPosting",
            "InteractiveInsert",
            "InteractiveSetDeletionMark",
            "InteractiveClearDeletionMark",
            "InteractivePosting",
            "InteractivePostingRegular",
            "InteractiveUndoPosting",
            "InteractiveChangeOfPosted",
        ],
        ("edit", "InformationRegister" | "AccumulationRegister" | "AccountingRegister")
        | ("edit", "Constant") => vec!["Read", "Update", "View", "Edit"],
        ("edit", "SessionParameter") => vec!["Get", "Set"],
        ("edit", "CommonAttribute") => vec!["View", "Edit"],
        ("view", "SessionParameter") => vec!["Get"],
        ("view", "CommonAttribute") => vec!["View"],
        ("view", "Sequence") => vec!["Read"],
        ("edit", "Sequence") => vec!["Read", "Update"],
        ("edit", "DocumentJournal") => vec!["Read", "View"],
        ("view" | "edit", _) => {
            stderr.push_str(&format!(
                "WARNING: Preset '@{preset}' not defined for type '{object_type}'. Available: none\n"
            ));
            Vec::new()
        }
        _ => {
            stderr.push_str(&format!(
                "WARNING: Unknown preset '@{preset}'. Known: @view, @edit\n"
            ));
            Vec::new()
        }
    }
    .into_iter()
    .map(ToOwned::to_owned)
    .collect()
}

pub(crate) fn invoke_read(
    operation: &str,
    _tool_name: &str,
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> Option<Result<AdapterOutcome, String>> {
    match operation {
        // Typed answer: the registry keeps the prose-shaped signature, and the
        // data reaches the envelope through typed_result.rs.
        "role-info" => Some(Ok(analyze_role_info(
            args,
            context,
            &WorkspaceSupportStateReader::new(context),
        )
        .outcome)),
        "role-validate" => Some(Ok(validate_role(args, context))),
        _ => None,
    }
}

pub(crate) struct RoleEditExecution {
    pub(crate) outcome: AdapterOutcome,
    pub(crate) data: Option<RoleEditData>,
    pub(crate) events: Vec<DomainEvent>,
    pub(crate) recorded_cache: Option<CacheReport>,
}

struct RoleEditTarget {
    source_set: String,
    metadata_path: String,
    descriptor_path: PathBuf,
    descriptor_preimage: Vec<u8>,
    registration_path: PathBuf,
    registration_preimage: Vec<u8>,
    rights_path: PathBuf,
    rights_preimage: Vec<u8>,
    support_warning: Option<String>,
    /// Пары «объект, право», которых касался вызов: только их значения
    /// пост-образ обязан иметь булевыми.
    written_rights: Vec<(String, String)>,
}

struct RoleEditPlan {
    target: RoleEditTarget,
    postimage: Vec<u8>,
    data: RoleEditData,
}

struct RoleEditPublication {
    warnings: Vec<String>,
    event: DomainEvent,
    cache: CacheReport,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RoleResolutionCode {
    SourceSetUnknown,
    TargetNotFound,
    NotARole,
    ProviderUnavailable,
    ContainmentDenied,
    ProfileUnsupported,
}

impl RoleResolutionCode {
    const fn code(self) -> &'static str {
        match self {
            Self::SourceSetUnknown => "source_set_unknown",
            Self::TargetNotFound => "target_not_found",
            Self::NotARole => "not_a_role",
            Self::ProviderUnavailable => "provider_unavailable",
            Self::ContainmentDenied => "containment_denied",
            Self::ProfileUnsupported => "profile_unsupported",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct RoleResolutionFailure {
    code: RoleResolutionCode,
    reason: &'static str,
}

impl RoleResolutionFailure {
    const fn new(code: RoleResolutionCode, reason: &'static str) -> Self {
        Self { code, reason }
    }

    const fn code(self) -> &'static str {
        self.code.code()
    }

    const fn reason(self) -> &'static str {
        self.reason
    }
}

impl std::fmt::Display for RoleResolutionFailure {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}: {}", self.code(), self.reason())
    }
}

fn role_support_warning() -> String {
    "support_guard_warning: the role is protected by support policy; warn mode allows the mutation"
        .to_string()
}

pub(crate) fn resolve_role_edit_guard_path(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> Result<PathBuf, String> {
    let request = parse_role_edit_request(args).map_err(|error| error.to_string())?;
    resolve_role_descriptor(&request.source_set, &request.metadata_path, context)
        .map(|(_, evidence)| evidence.target_path)
        .map_err(|failure| failure.to_string())
}

fn resolve_role_descriptor(
    source_set: &str,
    metadata_path: &crate::domain::source_target::MetadataAddress,
    context: &WorkspaceContext,
) -> Result<(ClosedPlatformXmlTarget, PlatformXmlResourceEvidence), RoleResolutionFailure> {
    if !metadata_path.as_str().starts_with("Role.") {
        return Err(RoleResolutionFailure::new(
            RoleResolutionCode::NotARole,
            "metadataPath must identify a Role",
        ));
    }
    let target = SourceTarget {
        source_set: source_set.to_string(),
        metadata_path: Some(metadata_path.clone()),
    };
    let resolution = resolve_platform_xml_target(context, &target, TargetKindPolicy::Any)
        .map_err(|error| role_resolution_failure(error.code))?;
    if resolution.resolved.target_kind != TargetKind::MetadataObject {
        return Err(RoleResolutionFailure::new(
            RoleResolutionCode::NotARole,
            "metadataPath must identify a role object",
        ));
    }
    let evidence = platform_xml_resource_evidence(context, &resolution.handle).map_err(|_| {
        RoleResolutionFailure::new(
            RoleResolutionCode::ProviderUnavailable,
            "the role descriptor evidence is unavailable",
        )
    })?;
    Ok((resolution.handle, evidence))
}

fn role_resolution_failure(code: SourceTargetErrorCode) -> RoleResolutionFailure {
    match code {
        SourceTargetErrorCode::SourceSetRequired | SourceTargetErrorCode::SourceSetNotFound => {
            RoleResolutionFailure::new(
                RoleResolutionCode::SourceSetUnknown,
                "the requested source set is unavailable",
            )
        }
        SourceTargetErrorCode::MetadataAddressNotFound => RoleResolutionFailure::new(
            RoleResolutionCode::TargetNotFound,
            "the logical role target was not found",
        ),
        SourceTargetErrorCode::TargetKindMismatch
        | SourceTargetErrorCode::MetadataAddressInvalid => RoleResolutionFailure::new(
            RoleResolutionCode::NotARole,
            "metadataPath does not identify a role",
        ),
        SourceTargetErrorCode::ContainmentDenied => RoleResolutionFailure::new(
            RoleResolutionCode::ContainmentDenied,
            "the logical role target failed containment checks",
        ),
        SourceTargetErrorCode::AddressProfileUnsupported => RoleResolutionFailure::new(
            RoleResolutionCode::ProfileUnsupported,
            "the logical address profile is unsupported",
        ),
        SourceTargetErrorCode::SourceRootNotAddressable => RoleResolutionFailure::new(
            RoleResolutionCode::ProviderUnavailable,
            "the logical source provider is unavailable",
        ),
    }
}

pub(crate) fn preview_edit_with_data(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> RoleEditExecution {
    edit_with_data(args, context, true)
}

pub(crate) fn apply_edit_with_data(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> RoleEditExecution {
    edit_with_data(args, context, false)
}

fn edit_with_data(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
    preview: bool,
) -> RoleEditExecution {
    let request = match parse_role_edit_request(args) {
        Ok(request) => request,
        Err(error) => {
            let failed_data = args.get("metadataPath").and_then(Value::as_str).map(|raw| {
                RoleEditData::failed(
                    raw,
                    error.code,
                    error.message.clone(),
                    error.operation_index,
                )
            });
            return role_edit_failure(
                if preview {
                    "dry run: unica.role.edit rejected invalid arguments"
                } else {
                    "unica.role.edit rejected invalid arguments"
                },
                error.code,
                &error.message,
                failed_data,
            );
        }
    };
    let metadata_path = request.metadata_path.as_str().to_string();
    let planned = plan_role_edit(&request, context);
    let RoleEditPlan {
        target,
        postimage,
        data,
    } = match planned {
        Ok(plan) => plan,
        Err((code, message, operation_index)) => {
            let failed_data =
                RoleEditData::failed(metadata_path, code, message.clone(), operation_index);
            return role_edit_failure(
                if preview {
                    "dry run: unica.role.edit rejected role mutation"
                } else {
                    "unica.role.edit rejected role mutation"
                },
                code,
                &message,
                Some(failed_data),
            );
        }
    };

    let mut warnings = target
        .support_warning
        .clone()
        .into_iter()
        .collect::<Vec<_>>();
    let mut events = Vec::new();
    let mut recorded_cache = None;
    if !preview && data.changed {
        #[cfg(test)]
        run_role_edit_before_native_guard_hook();
        match publish_role_edit(&target, &postimage, context) {
            Ok(publication) => {
                for warning in publication.warnings {
                    if !warnings.contains(&warning) {
                        warnings.push(warning);
                    }
                }
                events.push(publication.event);
                recorded_cache = Some(publication.cache);
            }
            Err((code, warning)) => {
                if let Some(warning) = warning {
                    warnings.push(warning);
                }
                let failed = RoleEditData::failed(
                    target.metadata_path.clone(),
                    code,
                    "the role mutation could not be published atomically",
                    None,
                );
                return RoleEditExecution {
                    outcome: AdapterOutcome {
                        ok: false,
                        summary: "unica.role.edit could not publish role mutation".to_string(),
                        changes: Vec::new(),
                        warnings,
                        errors: vec![format!(
                            "{code}: the role mutation could not be published atomically"
                        )],
                        artifacts: Vec::new(),
                        stdout: None,
                        stderr: None,
                        command: None,
                    },
                    data: Some(failed),
                    events: Vec::new(),
                    recorded_cache: None,
                };
            }
        }
    }

    let changed = data.changed;
    let logical_target = format!("{} + {}", target.source_set, target.metadata_path);
    RoleEditExecution {
        outcome: AdapterOutcome {
            ok: true,
            summary: if !changed {
                "unica.role.edit is already applied"
            } else if preview {
                "dry run: unica.role.edit planned role mutation"
            } else {
                "unica.role.edit applied role mutation"
            }
            .to_string(),
            changes: changed
                .then(|| {
                    format!(
                        "{}: role rights {}",
                        logical_target,
                        if preview { "would change" } else { "changed" }
                    )
                })
                .into_iter()
                .collect(),
            warnings,
            errors: Vec::new(),
            artifacts: vec![logical_target],
            stdout: None,
            stderr: None,
            command: None,
        },
        data: Some(data),
        events,
        recorded_cache,
    }
}

fn role_edit_failure(
    summary: &str,
    code: &str,
    message: &str,
    data: Option<RoleEditData>,
) -> RoleEditExecution {
    RoleEditExecution {
        outcome: AdapterOutcome {
            ok: false,
            summary: summary.to_string(),
            changes: Vec::new(),
            warnings: Vec::new(),
            errors: vec![format!("{code}: {message}")],
            artifacts: Vec::new(),
            stdout: None,
            stderr: None,
            command: None,
        },
        data,
        events: Vec::new(),
        recorded_cache: None,
    }
}

fn plan_role_edit(
    request: &crate::domain::role::RoleEditRequest,
    context: &WorkspaceContext,
) -> Result<RoleEditPlan, (&'static str, String, Option<usize>)> {
    let (_handle, evidence) =
        resolve_role_descriptor(&request.source_set, &request.metadata_path, context)
            .map_err(|failure| (failure.code(), failure.reason().to_string(), None))?;
    let role_name = request
        .metadata_path
        .as_str()
        .strip_prefix("Role.")
        .expect("the domain parser accepts only Role metadata paths");
    let descriptor_preimage = read_regular_file(&evidence.target_path)
        .map_err(|message| ("target_unavailable", message, None))?;
    validate_role_edit_descriptor(&descriptor_preimage, role_name)
        .map_err(|message| ("format_incompatible", message, None))?;
    let registration_preimage = read_regular_file(&evidence.registration_path)
        .map_err(|message| ("target_unavailable", message, None))?;
    let rights_path = prove_role_rights_path(&evidence, role_name, context)
        .map_err(|message| ("containment_denied", message, None))?;
    let rights_preimage =
        read_regular_file(&rights_path).map_err(|message| ("target_unavailable", message, None))?;
    let (bom, mut body) = decode_role_xml(&rights_preimage)
        .map_err(|message| ("format_incompatible", message, None))?;
    validate_role_rights_document(&body, RoleValueScope::SourceImage)
        .map_err(|message| ("role_validation_failed", message, None))?;

    let support_warning = match evaluate_resolved_support_guard(
        &evidence.target_path,
        crate::application::SupportGuardRequirement::Editable,
        context,
    ) {
        ResolvedSupportGuardCheck::Allow => None,
        ResolvedSupportGuardCheck::Warn(_) => Some(role_support_warning()),
        ResolvedSupportGuardCheck::Block(_) => {
            return Err((
                "support_locked",
                "the logical role target is protected by support policy".to_string(),
                None,
            ));
        }
    };

    let mut effects = Vec::with_capacity(request.operations.len());
    for (operation_index, operation) in request.operations.iter().enumerate() {
        let (updated, effect) = apply_role_edit_operation(&body, operation, operation_index)
            .map_err(|message| {
                // Ненастроенный объект — отдельный отказ: writer вставляет
                // недостающее право, но не создаёт объектный блок, и вызывающая
                // сторона должна отличать это ограничение от порчи документа.
                let code = if message.contains("` was not found") {
                    "object_not_listed"
                } else {
                    "role_validation_failed"
                };
                (code, message, Some(operation_index))
            })?;
        body = updated;
        effects.push(effect);
    }
    let written_rights = request
        .operations
        .iter()
        .map(|operation| (operation.object_name.clone(), operation.right.clone()))
        .collect::<Vec<_>>();
    validate_role_rights_document(&body, RoleValueScope::WrittenRights(&written_rights))
        .map_err(|message| ("role_validation_failed", message, None))?;
    let postimage = encode_role_xml(bom, &body);
    let changed = rights_preimage != postimage;
    let metadata_path = request.metadata_path.as_str().to_string();

    Ok(RoleEditPlan {
        target: RoleEditTarget {
            source_set: request.source_set.clone(),
            metadata_path: metadata_path.clone(),
            descriptor_path: evidence.target_path,
            descriptor_preimage,
            registration_path: evidence.registration_path,
            registration_preimage,
            rights_path,
            rights_preimage,
            support_warning,
            written_rights,
        },
        postimage,
        data: RoleEditData::passed(metadata_path, changed, effects),
    })
}

fn publish_role_edit(
    target: &RoleEditTarget,
    postimage: &[u8],
    context: &WorkspaceContext,
) -> Result<RoleEditPublication, (&'static str, Option<String>)> {
    let address = MetadataAddress::parse(PLATFORM_XML_8_3_27_FORMAT_2_20, &target.metadata_path)
        .map_err(|_| ("concurrent_modification", None))?;
    let (current_handle, current_evidence) =
        resolve_role_descriptor(&target.source_set, &address, context).map_err(|failure| {
            match failure.code {
                RoleResolutionCode::ContainmentDenied => ("containment_denied", None),
                RoleResolutionCode::SourceSetUnknown
                | RoleResolutionCode::TargetNotFound
                | RoleResolutionCode::NotARole
                | RoleResolutionCode::ProviderUnavailable
                | RoleResolutionCode::ProfileUnsupported => ("concurrent_modification", None),
            }
        })?;
    if current_evidence.target_path != target.descriptor_path
        || current_evidence.registration_path != target.registration_path
    {
        return Err(("concurrent_modification", None));
    }
    let role_name = target
        .metadata_path
        .strip_prefix("Role.")
        .expect("planned role address stays canonical");
    let current_rights = prove_role_rights_path(&current_evidence, role_name, context)
        .map_err(|_| ("containment_denied", None))?;
    let current_rights_preimage =
        read_regular_file(&current_rights).map_err(|_| ("concurrent_modification", None))?;
    if current_rights != target.rights_path || current_rights_preimage != target.rights_preimage {
        return Err(("concurrent_modification", None));
    }

    #[cfg(test)]
    run_role_edit_after_rights_reread_hook();

    let mut transaction = CompileTransaction::new();
    transaction
        .replace_bytes_classified(
            &target.rights_path,
            &target.rights_preimage,
            postimage.to_vec(),
        )
        .map_err(|failure| (role_commit_failure_code(failure.kind()), None))?;
    transaction
        .guard_or_verify_exact_preimage(&target.descriptor_path, &target.descriptor_preimage)
        .map_err(|_| ("concurrent_modification", None))?;
    transaction
        .guard_or_verify_exact_preimage(&target.registration_path, &target.registration_preimage)
        .map_err(|_| ("concurrent_modification", None))?;
    bind_resolved_support_guard_evidence(&mut transaction, &target.descriptor_path, context)
        .map_err(|_| ("concurrent_modification", None))?;
    let current_descriptor =
        guard_resolved_platform_xml_target_dependencies(&mut transaction, &current_handle, context)
            .map_err(|_| ("concurrent_modification", None))?;
    if current_descriptor != target.descriptor_path {
        return Err(("concurrent_modification", None));
    }
    let mut publication_warnings = Vec::new();
    match evaluate_resolved_support_guard(
        &target.descriptor_path,
        crate::application::SupportGuardRequirement::Editable,
        context,
    ) {
        ResolvedSupportGuardCheck::Allow => {}
        ResolvedSupportGuardCheck::Warn(_) => publication_warnings.push(role_support_warning()),
        ResolvedSupportGuardCheck::Block(_) => return Err(("support_locked", None)),
    }

    let event = DomainEvent::new(DomainEventKind::RoleChanged, "unica.role.edit");
    let cache = WorkspaceStateRepository::new(context)
        .stage_report_in_transaction(
            &mut transaction,
            context,
            std::slice::from_ref(&event),
            false,
            CacheAccess {
                reads: &[],
                writes: &["metadata_graph", "rights_graph"],
            },
        )
        .map_err(|_| ("cache_publication_failed", None))?;

    let expected = postimage.to_vec();
    let written_rights = target.written_rights.clone();
    let handle = current_handle;
    let descriptor = target.descriptor_path.clone();
    let rights = target.rights_path.clone();
    let role_name = role_name.to_string();
    #[cfg(test)]
    run_role_edit_before_publish_hook();
    let report = transaction.commit_with_classified_post_validation(|| {
        #[cfg(test)]
        if ROLE_EDIT_POST_VALIDATION_FAILURE.with(|slot| slot.get()) {
            return Err(CommitFailure::provider(
                "injected role.edit post-validation failure",
            ));
        }
        let current =
            crate::infrastructure::platform_xml_source_targets::revalidate_platform_xml_target(
                context, &handle,
            )
            .map_err(|_| CommitFailure::concurrent("logical role target changed"))?;
        if current.path != descriptor {
            return Err(CommitFailure::concurrent("logical role target changed"));
        }
        let descriptor_bytes = fs::read(&descriptor)
            .map_err(|_| CommitFailure::provider("role descriptor is unavailable"))?;
        validate_role_edit_descriptor(&descriptor_bytes, &role_name)
            .map_err(CommitFailure::provider)?;
        let published =
            fs::read(&rights).map_err(|_| CommitFailure::provider("Rights.xml is unavailable"))?;
        if published != expected {
            return Err(CommitFailure::concurrent(
                "published Rights.xml differs from the planned postimage",
            ));
        }
        let (_, body) = decode_role_xml(&published).map_err(CommitFailure::provider)?;
        validate_role_rights_document(&body, RoleValueScope::WrittenRights(&written_rights))
            .map_err(CommitFailure::provider)
    });
    match report {
        Ok(report) => {
            if !report.cleanup_warnings.is_empty() {
                publication_warnings.push(
                    "publication_cleanup_incomplete: role rights were committed; private recovery cleanup is incomplete"
                        .to_string(),
                );
            }
            Ok(RoleEditPublication {
                warnings: publication_warnings,
                event,
                cache,
            })
        }
        Err(error) => Err((role_commit_failure_code(error.kind()), None)),
    }
}

fn role_commit_failure_code(kind: CommitFailureKind) -> &'static str {
    match kind {
        CommitFailureKind::ConcurrentModification => "concurrent_modification",
        CommitFailureKind::ProviderUnavailable => "provider_unavailable",
        CommitFailureKind::RollbackFailed => "rollback_failed",
    }
}

fn read_regular_file(path: &Path) -> Result<Vec<u8>, String> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|_| "required role resource is unavailable".to_string())?;
    if metadata_is_link_or_reparse_point(&metadata) || !metadata.is_file() {
        return Err("required role resource is not a direct regular file".to_string());
    }
    fs::read(path).map_err(|_| "required role resource is unavailable".to_string())
}

/// The identity check is role-specific; the proof that `<Roles>/<R>/Ext/Rights.xml`
/// is a direct regular file inside the selected source set is the general one
/// every subject reader needs, and lives in `logical_selector` (ADR-0049).
fn prove_role_rights_path(
    evidence: &PlatformXmlResourceEvidence,
    role_name: &str,
    context: &WorkspaceContext,
) -> Result<PathBuf, String> {
    if evidence
        .target_path
        .file_stem()
        .and_then(|value| value.to_str())
        != Some(role_name)
    {
        return Err("role descriptor identity does not match metadataPath".to_string());
    }
    prove_attached_resource(evidence, "Rights.xml", context).map_err(|failure| failure.to_string())
}

fn validate_role_edit_descriptor(raw: &[u8], role_name: &str) -> Result<(), String> {
    let (_, text) = decode_role_xml(raw)?;
    let document =
        Document::parse(&text).map_err(|_| "role descriptor is not well-formed XML".to_string())?;
    let root = document.root_element();
    if root.tag_name().name() != "MetaDataObject"
        || root.tag_name().namespace() != Some(ROLE_METADATA_NAMESPACE)
        || crate::infrastructure::platform_xml_owner::root_version_literal(&text, root).as_deref()
            != Some("2.20")
    {
        return Err("role descriptor must use exact MDClasses format 2.20".to_string());
    }
    let roles = direct_role_children(root, "Role", ROLE_METADATA_NAMESPACE);
    if roles.len() != 1 {
        return Err("role descriptor must contain exactly one direct Role".to_string());
    }
    let properties = direct_role_children(roles[0], "Properties", ROLE_METADATA_NAMESPACE);
    if properties.len() != 1 {
        return Err("Role must contain exactly one direct Properties element".to_string());
    }
    let names = direct_role_children(properties[0], "Name", ROLE_METADATA_NAMESPACE);
    if names.len() != 1 || role_text_content(names[0]) != role_name {
        return Err("Role.Properties.Name does not match metadataPath".to_string());
    }
    Ok(())
}

fn decode_role_xml(raw: &[u8]) -> Result<(bool, String), String> {
    let (bom, body) = if raw.starts_with(&[0xEF, 0xBB, 0xBF]) {
        (true, &raw[3..])
    } else {
        (false, raw)
    };
    let text = std::str::from_utf8(body)
        .map_err(|_| "role XML must be UTF-8 with an optional BOM".to_string())?;
    Ok((bom, text.to_string()))
}

fn encode_role_xml(bom: bool, body: &str) -> Vec<u8> {
    let mut output = Vec::with_capacity(body.len() + usize::from(bom) * 3);
    if bom {
        output.extend_from_slice(&[0xEF, 0xBB, 0xBF]);
    }
    output.extend_from_slice(body.as_bytes());
    output
}

fn direct_role_children<'a, 'input>(
    node: roxmltree::Node<'a, 'input>,
    name: &str,
    namespace: &str,
) -> Vec<roxmltree::Node<'a, 'input>> {
    node.children()
        .filter(|child| {
            child.is_element()
                && child.tag_name().name() == name
                && child.tag_name().namespace() == Some(namespace)
        })
        .collect()
}

fn role_text_content(node: roxmltree::Node<'_, '_>) -> String {
    node.descendants()
        .filter(|descendant| descendant.is_text())
        .filter_map(|descendant| descendant.text())
        .collect()
}

fn role_direct_boolean(node: roxmltree::Node<'_, '_>) -> Option<bool> {
    let mut text_nodes = node.children().filter(|child| child.is_text());
    let value = text_nodes.next()?.text()?;
    if text_nodes.next().is_some() {
        return None;
    }
    match value {
        "true" => Some(true),
        "false" => Some(false),
        _ => None,
    }
}

/// Чьё значение права обязано быть булевым. Прообраз источника писателю не
/// принадлежит: небулевое значение соседнего права — свойство исходника, а не
/// дефект мутации, и запрещать им правку другого права нельзя. Пост-образ
/// строго проверяется ровно там, где писатель менял байты.
#[derive(Clone, Copy)]
enum RoleValueScope<'a> {
    SourceImage,
    WrittenRights(&'a [(String, String)]),
}

impl RoleValueScope<'_> {
    fn requires_boolean(self, object_name: &str, right_name: &str) -> bool {
        match self {
            Self::SourceImage => false,
            Self::WrittenRights(written) => written
                .iter()
                .any(|(object, right)| object == object_name && right == right_name),
        }
    }
}

fn validate_role_rights_document(text: &str, values: RoleValueScope<'_>) -> Result<(), String> {
    let document =
        Document::parse(text).map_err(|_| "Rights.xml is not well-formed XML".to_string())?;
    let root = document.root_element();
    let namespace = root.tag_name().namespace().unwrap_or("");
    if (namespace, root.tag_name().name()) != (ROLE_RIGHTS_NAMESPACE, "Rights")
        || platform_xml_publication_policy(namespace, root.tag_name().name())
            != Some(PlatformXmlPublicationPolicy::ExactRootVersion)
        || crate::infrastructure::platform_xml_owner::root_version_literal(text, root).as_deref()
            != Some("2.20")
        || root.attribute((XSI_NAMESPACE, "type")) != Some("Rights")
    {
        return Err("Rights.xml must use exact roles format 2.20".to_string());
    }
    for flag in [
        "setForNewObjects",
        "setForAttributesByDefault",
        "independentRightsOfChildObjects",
    ] {
        let nodes = direct_role_children(root, flag, ROLE_RIGHTS_NAMESPACE);
        if nodes.len() != 1 || role_direct_boolean(nodes[0]).is_none() {
            return Err(format!(
                "Rights.xml must contain one direct boolean `{flag}` element"
            ));
        }
    }

    let mut seen_objects = HashSet::new();
    for object in direct_role_children(root, "object", ROLE_RIGHTS_NAMESPACE) {
        let names = direct_role_children(object, "name", ROLE_RIGHTS_NAMESPACE);
        if names.len() > 1 {
            return Err("a direct role object must not have duplicate direct names".to_string());
        }
        let Some(name) = names.first() else {
            continue;
        };
        let object_name = role_text_content(*name);
        if object_name.is_empty() {
            continue;
        }
        if !seen_objects.insert(object_name.clone()) {
            return Err(format!("duplicate role object `{object_name}`"));
        }
        let mut seen_rights = HashSet::new();
        for right in direct_role_children(object, "right", ROLE_RIGHTS_NAMESPACE) {
            let names = direct_role_children(right, "name", ROLE_RIGHTS_NAMESPACE);
            if names.len() > 1 {
                return Err(format!(
                    "a right of `{object_name}` must not have duplicate direct names"
                ));
            }
            let Some(name) = names.first() else {
                continue;
            };
            let right_name = role_text_content(*name);
            if right_name.is_empty() {
                continue;
            }
            if !seen_rights.insert(right_name.clone()) {
                return Err(format!(
                    "duplicate right `{right_name}` for object `{object_name}`"
                ));
            }
            let value_nodes = direct_role_children(right, "value", ROLE_RIGHTS_NAMESPACE);
            if value_nodes.len() != 1 {
                return Err(format!(
                    "right `{right_name}` of `{object_name}` must have one direct value"
                ));
            }
            if values.requires_boolean(&object_name, &right_name)
                && role_direct_boolean(value_nodes[0]).is_none()
            {
                return Err(format!(
                    "right `{right_name}` of `{object_name}` must have a boolean value"
                ));
            }
        }
    }
    Ok(())
}

enum RoleTextEdit {
    Replace(std::ops::Range<usize>, String),
    Insert(usize, String),
    Remove(std::ops::Range<usize>),
    None,
}

fn apply_role_edit_operation(
    text: &str,
    operation: &crate::domain::role::RoleEditOperation,
    operation_index: usize,
) -> Result<(String, RoleEditEffect), String> {
    let document =
        Document::parse(text).map_err(|_| "Rights.xml is not well-formed XML".to_string())?;
    let root = document.root_element();
    let mut objects = direct_role_children(root, "object", ROLE_RIGHTS_NAMESPACE)
        .into_iter()
        .filter(|object| {
            let names = direct_role_children(*object, "name", ROLE_RIGHTS_NAMESPACE);
            names.len() == 1
                && role_text_content(names[0]).as_str() == operation.object_name.as_str()
        })
        .collect::<Vec<_>>();
    if objects.len() > 1 {
        return Err(format!(
            "role object `{}` is ambiguous",
            operation.object_name
        ));
    }
    let mut segments = operation.object_name.split('.');
    let top_level =
        segments.next().is_some() && segments.next().is_some() && segments.next().is_none();
    let data_processor_cascade = !operation.value
        && operation.right == "Use"
        && top_level
        && operation.object_name.starts_with("DataProcessor.");
    // ibcmd 8.3.27.2074 хранит объектное право только при значении, отличном
    // от умолчания роли (`setForNewObjects`), и выбрасывает опустевший блок
    // `object`; явное право с умолчанием — не платформенная форма, и запись
    // до умолчания выражается удалением элемента.
    let stores_to_default =
        top_level && !data_processor_cascade && operation.value == role_set_for_new_objects(root);
    let Some(object) = objects.pop() else {
        if data_processor_cascade {
            return Ok((
                text.to_string(),
                RoleEditEffect {
                    operation_index,
                    operation: "setRight",
                    object_name: operation.object_name.clone(),
                    right: operation.right.clone(),
                    before: None,
                    after: false,
                    action: RoleEditEffectAction::RemoveObject,
                    changed: false,
                },
            ));
        }
        if stores_to_default {
            return Ok((
                text.to_string(),
                RoleEditEffect {
                    operation_index,
                    operation: "setRight",
                    object_name: operation.object_name.clone(),
                    right: operation.right.clone(),
                    before: None,
                    after: operation.value,
                    action: RoleEditEffectAction::SetRight,
                    changed: false,
                },
            ));
        }
        return Err(format!(
            "role object `{}` was not found",
            operation.object_name
        ));
    };

    let matching_rights = direct_role_children(object, "right", ROLE_RIGHTS_NAMESPACE)
        .into_iter()
        .filter(|right| {
            let names = direct_role_children(*right, "name", ROLE_RIGHTS_NAMESPACE);
            names.len() == 1 && role_text_content(names[0]).as_str() == operation.right.as_str()
        })
        .collect::<Vec<_>>();
    if matching_rights.len() > 1 {
        return Err(format!(
            "right `{}` of `{}` is ambiguous",
            operation.right, operation.object_name
        ));
    }
    let before = matching_rights.first().and_then(|right| {
        let values = direct_role_children(*right, "value", ROLE_RIGHTS_NAMESPACE);
        (values.len() == 1)
            .then(|| role_direct_boolean(values[0]))
            .flatten()
    });

    let removes_whole_block = stores_to_default
        && matching_rights.len() == 1
        && before != Some(operation.value)
        && direct_role_children(object, "right", ROLE_RIGHTS_NAMESPACE).len() == 1
        && object.children().filter(|child| child.is_element()).count() == 2;
    // `removeObject` — только платформенное правило DataProcessor; каноническое
    // опустошение блока остаётся логическим `setRight`: физическая форма
    // хранения не меняет смысла операции.
    let action = if data_processor_cascade {
        RoleEditEffectAction::RemoveObject
    } else {
        RoleEditEffectAction::SetRight
    };
    let edit = if data_processor_cascade {
        RoleTextEdit::Remove(line_aware_removal_range(text, object.range()))
    } else if let Some(right) = matching_rights.first() {
        if before == Some(operation.value) {
            RoleTextEdit::None
        } else if stores_to_default {
            // Переход к умолчанию — удаление элемента: платформа хранит только
            // отличающиеся права, а опустевший блок `object` не переживает её
            // экспорт. Блок с неизвестными детьми сохраняется — уходит лишь
            // само право.
            if removes_whole_block {
                RoleTextEdit::Remove(line_aware_removal_range(text, object.range()))
            } else {
                RoleTextEdit::Remove(line_aware_removal_range(text, right.range()))
            }
        } else {
            let values = direct_role_children(*right, "value", ROLE_RIGHTS_NAMESPACE);
            if values.len() != 1 {
                return Err(format!(
                    "right `{}` of `{}` must have one direct value",
                    operation.right, operation.object_name
                ));
            }
            let value = values[0];
            let range = value.range();
            let raw = &text[range.clone()];
            if raw.trim_end().ends_with("/>") {
                let trimmed_end = raw.trim_end().len();
                let slash = raw[..trimmed_end]
                    .rfind("/>")
                    .ok_or_else(|| "role value self-closing tag is malformed".to_string())?;
                let name_start = raw
                    .find('<')
                    .map(|offset| offset + 1)
                    .ok_or_else(|| "role value opening tag is malformed".to_string())?;
                let name_end = raw[name_start..]
                    .find(|character: char| {
                        character.is_whitespace() || matches!(character, '/' | '>')
                    })
                    .map(|offset| name_start + offset)
                    .ok_or_else(|| "role value QName is malformed".to_string())?;
                let qualified_name = &raw[name_start..name_end];
                RoleTextEdit::Replace(
                    range,
                    format!(
                        "{}>{}</{}>{}",
                        raw[..slash].trim_end(),
                        operation.value,
                        qualified_name,
                        &raw[trimmed_end..]
                    ),
                )
            } else if value.children().next().is_none() {
                // `<value></value>` инфосет-эквивалентен self-closing форме:
                // текст значения вставляется внутрь существующих тегов.
                let open_end = raw
                    .find('>')
                    .map(|offset| range.start + offset + 1)
                    .ok_or_else(|| "role value opening tag is malformed".to_string())?;
                RoleTextEdit::Insert(open_end, operation.value.to_string())
            } else {
                let text_nodes = value
                    .children()
                    .filter(|child| child.is_text())
                    .collect::<Vec<_>>();
                if text_nodes.len() != 1 || !matches!(text_nodes[0].text(), Some("true" | "false"))
                {
                    return Err(format!(
                        "right `{}` of `{}` must have one direct boolean text value",
                        operation.right, operation.object_name
                    ));
                }
                RoleTextEdit::Replace(text_nodes[0].range(), operation.value.to_string())
            }
        }
    } else if stores_to_default {
        // Отсутствующее право уже несёт умолчание роли: запись того же
        // значения — семантический no-op без нормализации чужих байтов.
        RoleTextEdit::None
    } else {
        let (offset, insertion) = right_insertion(text, object, &operation.right, operation.value)?;
        RoleTextEdit::Insert(offset, insertion)
    };
    drop(document);

    let mut updated = text.to_string();
    let changed = !matches!(edit, RoleTextEdit::None);
    match edit {
        RoleTextEdit::Replace(range, replacement) => updated.replace_range(range, &replacement),
        RoleTextEdit::Insert(offset, insertion) => updated.insert_str(offset, &insertion),
        RoleTextEdit::Remove(range) => updated.replace_range(range, ""),
        RoleTextEdit::None => {}
    }
    Ok((
        updated,
        RoleEditEffect {
            operation_index,
            operation: "setRight",
            object_name: operation.object_name.clone(),
            right: operation.right.clone(),
            before,
            after: operation.value,
            action,
            changed,
        },
    ))
}

fn role_set_for_new_objects(root: roxmltree::Node<'_, '_>) -> bool {
    direct_role_children(root, "setForNewObjects", ROLE_RIGHTS_NAMESPACE)
        .first()
        .and_then(|node| role_direct_boolean(*node))
        .unwrap_or(false)
}

fn line_aware_removal_range(text: &str, range: std::ops::Range<usize>) -> std::ops::Range<usize> {
    // Начало и конец строки считаются по `\r|\n`, как в `right_insertion`:
    // CR-only файл иначе оставлял бы после удаления пустую строку с отступом.
    let bytes = text.as_bytes();
    let line_start = bytes[..range.start]
        .iter()
        .rposition(|byte| matches!(byte, b'\r' | b'\n'))
        .map(|offset| offset + 1)
        .unwrap_or(0);
    let starts_on_own_line = text[line_start..range.start]
        .chars()
        .all(|character| matches!(character, ' ' | '\t'));
    let next_break = bytes[range.end..]
        .iter()
        .position(|byte| matches!(byte, b'\r' | b'\n'))
        .map(|offset| range.end + offset);
    let ends_on_own_line = next_break.is_some_and(|line_end| {
        text[range.end..line_end]
            .chars()
            .all(|character| matches!(character, ' ' | '\t'))
    });

    if starts_on_own_line && ends_on_own_line {
        let line_end = next_break.expect("ends_on_own_line requires a line break");
        let after = if text[line_end..].starts_with("\r\n") {
            line_end + 2
        } else {
            line_end + 1
        };
        line_start..after
    } else {
        range
    }
}

fn right_insertion(
    text: &str,
    object: roxmltree::Node<'_, '_>,
    right_name: &str,
    value: bool,
) -> Result<(usize, String), String> {
    let range = object.range();
    let raw = &text[range.clone()];
    let direct_rights = direct_role_children(object, "right", ROLE_RIGHTS_NAMESPACE);
    let object_prefix = lexical_role_prefix(text, object)?;
    // The object's own QName binding is necessarily in scope for inserted
    // descendants. A prefix copied from an existing right can be declared on
    // that sibling only and would be unbound on the newly emitted element.
    let right_prefix = object_prefix.clone();
    let name_prefix = object_prefix.clone();
    let value_prefix = object_prefix;
    let close_relative = raw
        .rfind("</")
        .ok_or_else(|| "role object closing tag is malformed".to_string())?;
    let close = range.start + close_relative;
    let line_start = text.as_bytes()[..close]
        .iter()
        .rposition(|byte| matches!(byte, b'\r' | b'\n'))
        .map(|offset| offset + 1)
        .unwrap_or(close);
    let has_line_layout = line_start < close
        && text[line_start..close]
            .chars()
            .all(|character| character == ' ' || character == '\t' || character == '\r');
    if !has_line_layout {
        return Ok((
            close,
            format!(
                "<{right_prefix}right><{name_prefix}name>{right_name}</{name_prefix}name><{value_prefix}value>{value}</{value_prefix}value></{right_prefix}right>"
            ),
        ));
    }
    let closing_indent = text[line_start..close].trim_end_matches('\r');
    let eol = role_source_eol(text)?;
    let right_indent = direct_rights
        .first()
        .map(|right| line_indent(text, right.range().start))
        .or_else(|| {
            direct_role_children(object, "name", ROLE_RIGHTS_NAMESPACE)
                .first()
                .map(|name| line_indent(text, name.range().start))
        })
        .filter(|indent| !indent.is_empty())
        .unwrap_or_else(|| format!("{closing_indent}\t"));
    let child_indent = format!("{right_indent}\t");
    Ok((
        line_start,
        format!(
            "{right_indent}<{right_prefix}right>{eol}{child_indent}<{name_prefix}name>{right_name}</{name_prefix}name>{eol}{child_indent}<{value_prefix}value>{value}</{value_prefix}value>{eol}{right_indent}</{right_prefix}right>{eol}"
        ),
    ))
}

fn role_source_eol(text: &str) -> Result<&'static str, String> {
    let snapshot = SourceTextSnapshot::from_bytes(text.as_bytes())
        .map_err(|error| format!("unsupported_node: {error}"))?;
    let policy = if snapshot.line_endings() == LineEndingProfile::None {
        EolPolicy::Lf
    } else {
        EolPolicy::Preserve
    };
    resolve_line_ending(policy, &snapshot, None)
        .map(|ending| ending.as_str())
        .map_err(|error| format!("unsupported_node: {error}"))
}

fn lexical_role_prefix(text: &str, node: roxmltree::Node<'_, '_>) -> Result<String, String> {
    let raw = &text[node.range()];
    let name_start = raw
        .find('<')
        .map(|offset| offset + 1)
        .ok_or_else(|| "role element opening tag is malformed".to_string())?;
    let name_end = raw[name_start..]
        .find(|character: char| character.is_whitespace() || matches!(character, '/' | '>'))
        .map(|offset| name_start + offset)
        .ok_or_else(|| "role element QName is malformed".to_string())?;
    let qname = &raw[name_start..name_end];
    Ok(qname
        .rsplit_once(':')
        .map(|(prefix, _)| format!("{prefix}:"))
        .unwrap_or_default())
}

fn line_indent(text: &str, offset: usize) -> String {
    let start = text[..offset]
        .rfind('\n')
        .map(|index| index + 1)
        .unwrap_or(0);
    text[start..offset]
        .chars()
        .take_while(|character| matches!(character, ' ' | '\t' | '\r'))
        .filter(|character| *character != '\r')
        .collect()
}

pub(crate) fn invoke_mutation(
    operation: &str,
    _tool_name: &str,
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> Option<AdapterOutcome> {
    match operation {
        "role-compile" => Some(compile_role(args, context)),
        _ => None,
    }
}

#[cfg(test)]
mod role_edit_contract_tests {
    use super::super::single_file_publisher::{with_publish_failpoints, PublishCheckpoint};
    use super::*;

    /// В фикстурах тестов документ целиком принадлежит писателю, поэтому
    /// булевость требуется от каждого права: строгость прежнего флага
    /// сохраняется, а продуктовый контракт остаётся адресным.
    fn validate_every_role_value(text: &str) -> Result<(), String> {
        let pairs = match Document::parse(text) {
            Ok(document) => {
                direct_role_children(document.root_element(), "object", ROLE_RIGHTS_NAMESPACE)
                    .into_iter()
                    .flat_map(|object| {
                        let object_name =
                            direct_role_children(object, "name", ROLE_RIGHTS_NAMESPACE)
                                .first()
                                .map(|node| role_text_content(*node))
                                .unwrap_or_default();
                        direct_role_children(object, "right", ROLE_RIGHTS_NAMESPACE)
                            .into_iter()
                            .map(move |right| {
                                let right_name =
                                    direct_role_children(right, "name", ROLE_RIGHTS_NAMESPACE)
                                        .first()
                                        .map(|node| role_text_content(*node))
                                        .unwrap_or_default();
                                (object_name.clone(), right_name)
                            })
                    })
                    .collect::<Vec<_>>()
            }
            Err(_) => Vec::new(),
        };
        validate_role_rights_document(text, RoleValueScope::WrittenRights(&pairs))
    }

    use crate::infrastructure::platform::testing::{
        create_dir_symlink_for_test, create_file_symlink_for_test, remove_dir_symlink_for_test,
    };
    use serde_json::json;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NONCE: AtomicU64 = AtomicU64::new(0);

    fn operation(
        object_name: &str,
        right: &str,
        value: bool,
    ) -> crate::domain::role::RoleEditOperation {
        crate::domain::role::RoleEditOperation {
            object_name: object_name.to_string(),
            right: right.to_string(),
            value,
        }
    }

    fn rights_xml(eol: &str, self_closing: &str) -> String {
        [
            r#"<Rights xmlns="http://v8.1c.ru/8.2/roles" xmlns:r="http://v8.1c.ru/8.2/roles" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" xsi:type="Rights" version="2.20">"#,
            "\t<setForNewObjects>false</setForNewObjects>",
            "\t<setForAttributesByDefault>true</setForAttributesByDefault>",
            "\t<independentRightsOfChildObjects>false</independentRightsOfChildObjects>",
            "\t<object>",
            "\t\t<name>Catalog.Demo</name>",
            "\t\t<future><name>Delete</name><value>false</value></future>",
            "\t\t<right>",
            "\t\t\t<name>Delete</name>",
            &format!("\t\t\t{self_closing}"),
            "\t\t\t<restrictionByCondition><condition>WHERE false</condition></restrictionByCondition>",
            "\t\t</right>",
            "\t\t<right><name>FuturePlatformRight</name><value>true</value><future/></right>",
            "\t</object>",
            "\t<object>",
            "\t\t<name>DataProcessor.Worker</name>",
            "\t\t<right><name>Use</name><value>true</value></right>",
            "\t\t<right><name>View</name><value>true</value></right>",
            "\t</object>",
            "\t<restrictionTemplate><name>Keep</name><condition>TRUE</condition></restrictionTemplate>",
            "</Rights>",
        ]
        .join(eol)
    }

    #[test]
    fn rights_profile_requires_registered_root_and_unprefixed_rights_xsi_type() {
        let valid = rights_xml("\n", "<value>true</value>");
        validate_every_role_value(&valid).unwrap();

        let alias = valid
            .replace("xmlns:xsi=", "xmlns:schemaInstance=")
            .replace("xsi:type=", "schemaInstance:type=");
        validate_every_role_value(&alias).unwrap();

        for invalid in [
            valid.replace(" xsi:type=\"Rights\"", ""),
            valid.replace("xsi:type=\"Rights\"", "xsi:type=\"Role\""),
            valid.replace(
                "xsi:type=\"Rights\"",
                "xmlns:roles=\"http://v8.1c.ru/8.2/roles\" xsi:type=\"roles:Rights\"",
            ),
            valid
                .replace(
                    "<Rights xmlns=\"http://v8.1c.ru/8.2/roles\"",
                    "<PredefinedData xmlns=\"http://v8.1c.ru/8.3/xcf/predef\"",
                )
                .replace("</Rights>", "</PredefinedData>"),
        ] {
            assert!(
                validate_every_role_value(&invalid).is_err(),
                "accepted incompatible root contract: {invalid}"
            );
        }
    }

    #[test]
    fn role_identity_matching_uses_complete_xml_string_values() {
        let body = rights_xml("\n", "<value>true</value>");
        let misleading_object = body.replacen(
            "<name>Catalog.Demo</name>",
            "<name>Catalog.Demo<!--future-->Suffix</name>",
            1,
        );
        let error = apply_role_edit_operation(
            &misleading_object,
            &operation("Catalog.Demo", "Delete", true),
            0,
        )
        .expect_err("a prefix text node must not identify the role object");
        assert!(error.contains("was not found"), "{error}");
        validate_every_role_value(&misleading_object).unwrap();

        let misleading_right = body.replacen(
            "\t\t\t<name>Delete</name>",
            "\t\t\t<name>Delete<!--future-->Suffix</name>",
            1,
        );
        let (updated, effect) = apply_role_edit_operation(
            &misleading_right,
            &operation("Catalog.Demo", "Delete", true),
            0,
        )
        .unwrap();
        assert_eq!(effect.before, None);
        assert!(effect.changed);
        assert!(updated.contains("<name>Delete<!--future-->Suffix</name>"));
        let document = Document::parse(&updated).unwrap();
        let object =
            direct_role_children(document.root_element(), "object", ROLE_RIGHTS_NAMESPACE)[0];
        let right_names = direct_role_children(object, "right", ROLE_RIGHTS_NAMESPACE)
            .into_iter()
            .map(|right| {
                role_text_content(direct_role_children(right, "name", ROLE_RIGHTS_NAMESPACE)[0])
            })
            .collect::<Vec<_>>();
        assert!(right_names.iter().any(|name| name == "DeleteSuffix"));
        assert!(right_names.iter().any(|name| name == "Delete"));
        validate_every_role_value(&updated).unwrap();

        let descriptor = format!(
            r#"<MetaDataObject xmlns="{ROLE_METADATA_NAMESPACE}" version="2.20"><Role><Properties><Name>Demo<!--future-->Suffix</Name></Properties></Role></MetaDataObject>"#
        );
        assert!(validate_role_edit_descriptor(descriptor.as_bytes(), "Demo").is_err());
        validate_role_edit_descriptor(descriptor.as_bytes(), "DemoSuffix").unwrap();
        let encoded_version = descriptor.replace("version=\"2.20\"", "version=\"2.2&#48;\"");
        assert!(validate_role_edit_descriptor(encoded_version.as_bytes(), "DemoSuffix").is_err());

        let split_boolean =
            body.replacen("<value>true</value>", "<value>tr<!--future-->ue</value>", 1);
        validate_role_rights_document(&split_boolean, RoleValueScope::SourceImage).unwrap();
        assert!(validate_every_role_value(&split_boolean).is_err());
        let error = apply_role_edit_operation(
            &split_boolean,
            &operation("Catalog.Demo", "Delete", true),
            0,
        )
        .expect_err("a split boolean must not have a distinct no-op interpretation");
        assert!(error.contains("one direct boolean text value"), "{error}");
    }

    #[test]
    fn right_insertion_rejects_mixed_eol_and_preserves_uniform_lone_cr() {
        let mixed = rights_xml("\n", "<value>true</value>").replacen('\n', "\r\n", 1);
        let mixed_document = Document::parse(&mixed).unwrap();
        let mixed_object = direct_role_children(
            mixed_document.root_element(),
            "object",
            ROLE_RIGHTS_NAMESPACE,
        )[0];
        let error = right_insertion(&mixed, mixed_object, "Edit", true)
            .expect_err("mixed EOL must not choose a global fallback");
        assert!(error.contains("ambiguous"), "{error}");

        let lone_cr = rights_xml("\r", "<value>true</value>");
        let lone_cr_document = Document::parse(&lone_cr).unwrap();
        let lone_cr_object = direct_role_children(
            lone_cr_document.root_element(),
            "object",
            ROLE_RIGHTS_NAMESPACE,
        )[0];
        let (_, insertion) = right_insertion(&lone_cr, lone_cr_object, "Edit", true).unwrap();
        assert!(insertion.contains('\r'));
        assert!(!insertion.contains('\n'));
    }

    #[test]
    fn range_writer_uses_direct_children_and_preserves_unknown_xml_bom_and_eol() {
        for (self_closing, expected_value) in [
            ("<value/>", "<value>true</value>"),
            ("<value />", "<value>true</value>"),
            ("<value></value>", "<value>true</value>"),
            (
                "<r:value future=\"x\"/>",
                "<r:value future=\"x\">true</r:value>",
            ),
        ] {
            let body = rights_xml("\r\n", self_closing);
            let (updated, effect) =
                apply_role_edit_operation(&body, &operation("Catalog.Demo", "Delete", true), 0)
                    .unwrap();
            assert_eq!(effect.before, None);
            assert!(effect.changed);
            assert!(updated.contains(expected_value), "{updated}");
            assert!(updated.contains("<future><name>Delete</name><value>false</value></future>"));
            assert!(updated.contains("restrictionByCondition"));
            assert!(updated.contains("FuturePlatformRight"));
            assert!(updated.contains("restrictionTemplate"));
            assert!(!updated.replace("\r\n", "").contains('\n'));
            validate_every_role_value(&updated).unwrap();

            let encoded = encode_role_xml(true, &updated);
            assert!(encoded.starts_with(&[0xEF, 0xBB, 0xBF]));
        }
    }

    #[test]
    fn range_writer_replaces_only_boolean_text_and_preserves_value_markup() {
        let body = rights_xml(
            "\n",
            r#"<r:value future=">">false<!--future-value-marker--></r:value>"#,
        );

        let (updated, effect) =
            apply_role_edit_operation(&body, &operation("Catalog.Demo", "Delete", true), 0)
                .unwrap();

        assert!(effect.changed);
        assert!(updated.contains(r#"<r:value future=">">true<!--future-value-marker--></r:value>"#));
        validate_every_role_value(&updated).unwrap();
    }

    #[test]
    fn writer_ignores_and_preserves_unnamed_forward_compatible_blocks() {
        let unnamed_right = "<right future=\"unnamed-right\"><value>true</value><future/></right>";
        let unnamed_object =
            "<object future=\"unnamed-object\"><right><value>false</value></right></object>";
        let body = rights_xml("\n", "<value>true</value>")
            .replacen(
                "\t\t<right>",
                &format!("\t\t{unnamed_right}\n\t\t<right>"),
                1,
            )
            .replace("\n</Rights>", &format!("\n\t{unnamed_object}\n</Rights>"));

        let (updated, effect) =
            apply_role_edit_operation(&body, &operation("Catalog.Demo", "Delete", false), 0)
                .unwrap();

        assert!(effect.changed);
        assert!(updated.contains(unnamed_right));
        assert!(updated.contains(unnamed_object));
        validate_every_role_value(&updated).unwrap();
    }

    #[test]
    fn writer_inserts_with_existing_eol_and_data_processor_false_removes_whole_object() {
        let body = rights_xml("\r\n", "<value>true</value>");
        let (inserted, effect) =
            apply_role_edit_operation(&body, &operation("Catalog.Demo", "View", true), 0).unwrap();
        assert!(effect.changed);
        assert_eq!(effect.before, None);
        assert!(inserted.contains("<name>View</name>\r\n"));
        assert!(!inserted.replace("\r\n", "").contains('\n'));

        let (removed, effect) = apply_role_edit_operation(
            &inserted,
            &operation("DataProcessor.Worker", "Use", false),
            1,
        )
        .unwrap();
        assert_eq!(effect.action, RoleEditEffectAction::RemoveObject);
        assert_eq!(effect.before, Some(true));
        assert!(!removed.contains("DataProcessor.Worker"));
        assert_eq!(removed.matches("<name>View</name>").count(), 1);
        assert!(removed.contains("Catalog.Demo"));
        assert!(removed.contains("restrictionTemplate"));
    }

    #[test]
    fn data_processor_removal_preserves_eol_when_object_follows_an_inline_sibling() {
        let body = rights_xml("\n", "<value>true</value>").replace(
            "\t</object>\n\t<object>\n\t\t<name>DataProcessor.Worker</name>",
            "\t</object><object>\n\t\t<name>DataProcessor.Worker</name>",
        );
        assert!(body.contains("\t</object><object>\n"), "{body}");

        let (removed, effect) =
            apply_role_edit_operation(&body, &operation("DataProcessor.Worker", "Use", false), 0)
                .unwrap();

        assert_eq!(effect.action, RoleEditEffectAction::RemoveObject);
        assert_eq!(effect.before, Some(true));
        assert!(!removed.contains("DataProcessor.Worker"));
        assert!(
            removed.contains("\t</object>\n\t<restrictionTemplate>"),
            "{removed}"
        );
        validate_every_role_value(&removed).unwrap();
    }

    #[test]
    fn missing_right_inherits_prefix_only_roles_qnames() {
        let body = concat!(
            "<r:Rights xmlns:r=\"http://v8.1c.ru/8.2/roles\" ",
            "xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\" ",
            "xsi:type=\"Rights\" version=\"2.20\">\n",
            "\t<r:setForNewObjects>false</r:setForNewObjects>\n",
            "\t<r:setForAttributesByDefault>true</r:setForAttributesByDefault>\n",
            "\t<r:independentRightsOfChildObjects>false</r:independentRightsOfChildObjects>\n",
            "\t<r:object>\n",
            "\t\t<r:name>Catalog.Demo</r:name>\n",
            "\t\t<r:right><r:name>Delete</r:name><r:value>true</r:value></r:right>\n",
            "\t</r:object>\n",
            "</r:Rights>"
        );
        let (updated, effect) =
            apply_role_edit_operation(body, &operation("Catalog.Demo", "View", true), 0).unwrap();
        assert!(effect.changed);
        assert!(updated.contains("<r:right>"), "{updated}");
        assert!(updated.contains("<r:name>View</r:name>"), "{updated}");
        assert!(updated.contains("<r:value>true</r:value>"), "{updated}");
        assert!(!updated.contains("<right>"), "{updated}");
        validate_every_role_value(&updated).unwrap();
    }

    #[test]
    fn missing_right_does_not_copy_a_sibling_local_namespace_prefix() {
        let local_right = concat!(
            "<x:right xmlns:x=\"http://v8.1c.ru/8.2/roles\" future=\"keep\">",
            "<x:name>Delete</x:name><x:value>true</x:value><x:future/>",
            "</x:right>"
        );
        let body = concat!(
            "<Rights xmlns=\"http://v8.1c.ru/8.2/roles\" ",
            "xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\" ",
            "xsi:type=\"Rights\" version=\"2.20\">",
            "<setForNewObjects>false</setForNewObjects>",
            "<setForAttributesByDefault>true</setForAttributesByDefault>",
            "<independentRightsOfChildObjects>false</independentRightsOfChildObjects>",
            "<object><name>Catalog.Demo</name>",
            "<x:right xmlns:x=\"http://v8.1c.ru/8.2/roles\" future=\"keep\">",
            "<x:name>Delete</x:name><x:value>true</x:value><x:future/>",
            "</x:right></object></Rights>"
        );

        let (updated, effect) =
            apply_role_edit_operation(body, &operation("Catalog.Demo", "View", true), 0).unwrap();

        assert!(effect.changed);
        assert!(updated.contains(local_right));
        assert!(updated.contains("<right><name>View</name><value>true</value></right>"));
        assert_eq!(updated.matches("xmlns:x=").count(), 1);
        validate_every_role_value(&updated).unwrap();
    }

    #[test]
    fn duplicate_objects_and_rights_are_refused_before_any_mutation() {
        // Шесть веток отказа структурной проверки: неоднозначный документ
        // нельзя ни читать как истину, ни править «первым попавшимся».
        let base = rights_xml("\n", "<value>true</value>");
        for (label, body, expected) in [
            (
                "duplicate object",
                base.replacen(
                    "\t<object>\n\t\t<name>DataProcessor.Worker</name>",
                    "\t<object>\n\t\t<name>Catalog.Demo</name>\n\t\t<right><name>Read</name><value>true</value></right>\n\t</object>\n\t<object>\n\t\t<name>DataProcessor.Worker</name>",
                    1,
                ),
                "duplicate role object",
            ),
            (
                "duplicate object name",
                base.replacen(
                    "\t\t<name>Catalog.Demo</name>",
                    "\t\t<name>Catalog.Demo</name>\n\t\t<name>Catalog.Other</name>",
                    1,
                ),
                "duplicate direct names",
            ),
            (
                "duplicate right",
                base.replacen(
                    "\t\t<right><name>FuturePlatformRight</name><value>true</value><future/></right>",
                    "\t\t<right><name>Delete</name><value>false</value></right>",
                    1,
                ),
                "duplicate right `Delete`",
            ),
            (
                "duplicate right name",
                base.replacen(
                    "\t\t\t<name>Delete</name>",
                    "\t\t\t<name>Delete</name>\n\t\t\t<name>Insert</name>",
                    1,
                ),
                "must not have duplicate direct names",
            ),
            (
                "two direct values",
                base.replacen(
                    "\t\t\t<value>true</value>",
                    "\t\t\t<value>true</value>\n\t\t\t<value>false</value>",
                    1,
                ),
                "must have one direct value",
            ),
        ] {
            let error = validate_role_rights_document(&body, RoleValueScope::SourceImage)
                .expect_err(label);
            assert!(error.contains(expected), "{label}: {error}");
        }

        // Неоднозначность до мутации ловит и сам writer.
        let ambiguous_object = base.replacen(
            "\t<object>\n\t\t<name>DataProcessor.Worker</name>",
            "\t<object>\n\t\t<name>Catalog.Demo</name>\n\t\t<right><name>Read</name><value>true</value></right>\n\t</object>\n\t<object>\n\t\t<name>DataProcessor.Worker</name>",
            1,
        );
        let error = apply_role_edit_operation(
            &ambiguous_object,
            &operation("Catalog.Demo", "Read", false),
            0,
        )
        .expect_err("ambiguous object must not be resolved by picking the first");
        assert!(error.contains("is ambiguous"), "{error}");

        let ambiguous_right = base.replacen(
            "\t\t<right><name>FuturePlatformRight</name><value>true</value><future/></right>",
            "\t\t<right><name>Delete</name><value>false</value></right>",
            1,
        );
        let error = apply_role_edit_operation(
            &ambiguous_right,
            &operation("Catalog.Demo", "Delete", false),
            0,
        )
        .expect_err("ambiguous right must not be resolved by picking the first");
        assert!(error.contains("is ambiguous"), "{error}");
    }

    #[test]
    fn cdata_carries_identity_and_value_like_ordinary_text() {
        // CDATA — та же строка инфосета: адресация и чтение значения не должны
        // от неё зависеть.
        let body = rights_xml("\n", "<value><![CDATA[true]]></value>").replacen(
            "<name>Catalog.Demo</name>",
            "<name><![CDATA[Catalog.Demo]]></name>",
            1,
        );

        let (updated, effect) =
            apply_role_edit_operation(&body, &operation("Catalog.Demo", "Delete", false), 0)
                .unwrap();

        assert_eq!(effect.before, Some(true), "CDATA-значение прочитано");
        assert!(effect.changed);
        assert!(
            updated.contains("<![CDATA[Catalog.Demo]]>"),
            "имя объекта сохранено как есть: {updated}"
        );
        validate_every_role_value(&updated).unwrap();
    }

    #[test]
    fn an_untouched_non_boolean_neighbour_does_not_block_the_requested_edit() {
        // Небулевое значение соседнего права принадлежит исходнику, а не
        // писателю: раньше строгая пост-проверка отвергала весь вызов чужим
        // правом и без `operationIndex`.
        let body = rights_xml("\n", "<value>true</value>").replacen(
            "<right><name>FuturePlatformRight</name><value>true</value><future/></right>",
            "<right><name>FuturePlatformRight</name><value/><future/></right>",
            1,
        );
        assert!(body.contains("<value/>"), "{body}");

        let (updated, effect) =
            apply_role_edit_operation(&body, &operation("Catalog.Demo", "Delete", false), 0)
                .unwrap();

        assert!(effect.changed);
        assert!(updated.contains("<value/>"), "сосед сохранён как есть");
        let written = [("Catalog.Demo".to_string(), "Delete".to_string())];
        validate_role_rights_document(&updated, RoleValueScope::WrittenRights(&written)).unwrap();
        // А если бы писатель тронул именно это право, строгость осталась бы.
        let neighbour = [(
            "Catalog.Demo".to_string(),
            "FuturePlatformRight".to_string(),
        )];
        assert!(
            validate_role_rights_document(&updated, RoleValueScope::WrittenRights(&neighbour))
                .is_err()
        );
    }

    #[test]
    fn default_valued_right_is_stored_as_absence_like_the_platform_export() {
        // ibcmd 8.3.27.2074 хранит объектное право только при значении,
        // отличном от `setForNewObjects`; переход к умолчанию — удаление
        // элемента вместе с его RLS, а не явный `false`.
        let body = rights_xml("\n", "<value>true</value>");
        let (updated, effect) =
            apply_role_edit_operation(&body, &operation("Catalog.Demo", "Delete", false), 0)
                .unwrap();
        assert!(effect.changed);
        assert_eq!(effect.before, Some(true));
        assert!(!effect.after);
        assert_eq!(effect.action, RoleEditEffectAction::SetRight);
        let document = Document::parse(&updated).unwrap();
        let object =
            direct_role_children(document.root_element(), "object", ROLE_RIGHTS_NAMESPACE)[0];
        assert!(
            !direct_role_children(object, "right", ROLE_RIGHTS_NAMESPACE)
                .into_iter()
                .any(|right| {
                    role_text_content(direct_role_children(right, "name", ROLE_RIGHTS_NAMESPACE)[0])
                        == "Delete"
                }),
            "{updated}"
        );
        drop(document);
        assert!(!updated.contains("restrictionByCondition"), "{updated}");
        // Блок не опустел (остался FuturePlatformRight и неизвестный child) —
        // он сохраняется вместе с ними.
        assert!(updated.contains("<name>Catalog.Demo</name>"), "{updated}");
        assert!(updated.contains("FuturePlatformRight"), "{updated}");
        assert!(
            updated.contains("<future><name>Delete</name><value>false</value></future>"),
            "{updated}"
        );
        assert!(updated.contains("restrictionTemplate"), "{updated}");
        validate_every_role_value(&updated).unwrap();

        // Повтор — no-op: право уже несёт умолчание отсутствием.
        let (repeated, second) =
            apply_role_edit_operation(&updated, &operation("Catalog.Demo", "Delete", false), 1)
                .unwrap();
        assert_eq!(repeated, updated);
        assert!(!second.changed);
        assert_eq!(second.before, None);
    }

    #[test]
    fn removing_the_last_stored_right_drops_the_emptied_object_block() {
        let body = [
            "<Rights xmlns=\"http://v8.1c.ru/8.2/roles\" xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\" xsi:type=\"Rights\" version=\"2.20\">",
            "\t<setForNewObjects>false</setForNewObjects>",
            "\t<setForAttributesByDefault>true</setForAttributesByDefault>",
            "\t<independentRightsOfChildObjects>false</independentRightsOfChildObjects>",
            "\t<object>",
            "\t\t<name>Catalog.Solo</name>",
            "\t\t<right>",
            "\t\t\t<name>Read</name>",
            "\t\t\t<value>true</value>",
            "\t\t</right>",
            "\t</object>",
            "\t<object>",
            "\t\t<name>Catalog.Kept</name>",
            "\t\t<right><name>Read</name><value>true</value></right>",
            "\t</object>",
            "</Rights>",
        ]
        .join("\n");

        let (updated, effect) =
            apply_role_edit_operation(&body, &operation("Catalog.Solo", "Read", false), 0).unwrap();
        assert!(effect.changed);
        assert_eq!(effect.action, RoleEditEffectAction::SetRight);
        assert!(!updated.contains("Catalog.Solo"), "{updated}");
        assert!(updated.contains("Catalog.Kept"), "{updated}");
        validate_every_role_value(&updated).unwrap();
    }

    #[test]
    fn emptied_block_with_unknown_children_keeps_the_block_and_sheds_the_right() {
        let body = [
            "<Rights xmlns=\"http://v8.1c.ru/8.2/roles\" xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\" xsi:type=\"Rights\" version=\"2.20\">",
            "\t<setForNewObjects>false</setForNewObjects>",
            "\t<setForAttributesByDefault>true</setForAttributesByDefault>",
            "\t<independentRightsOfChildObjects>false</independentRightsOfChildObjects>",
            "\t<object>",
            "\t\t<name>Catalog.Solo</name>",
            "\t\t<future keep=\"true\"/>",
            "\t\t<right><name>Read</name><value>true</value></right>",
            "\t</object>",
            "</Rights>",
        ]
        .join("\n");

        let (updated, effect) =
            apply_role_edit_operation(&body, &operation("Catalog.Solo", "Read", false), 0).unwrap();
        assert!(effect.changed);
        assert_eq!(effect.action, RoleEditEffectAction::SetRight);
        assert!(updated.contains("<name>Catalog.Solo</name>"), "{updated}");
        assert!(updated.contains("<future keep=\"true\"/>"), "{updated}");
        assert!(!updated.contains("<name>Read</name>"), "{updated}");
        validate_every_role_value(&updated).unwrap();
    }

    #[test]
    fn set_for_new_objects_true_mirrors_the_stored_value_rule() {
        let body = rights_xml("\n", "<value>true</value>").replacen(
            "<setForNewObjects>false</setForNewObjects>",
            "<setForNewObjects>true</setForNewObjects>",
            1,
        );

        // Явный true при умолчании true уже совпадает с запросом — no-op:
        // нетронутая избыточная форма не нормализуется.
        let (same, noop) =
            apply_role_edit_operation(&body, &operation("Catalog.Demo", "Delete", true), 0)
                .unwrap();
        assert_eq!(same, body);
        assert!(!noop.changed);

        // false — значимое отклонение от умолчания: обычная замена значения.
        let (with_false, replaced) =
            apply_role_edit_operation(&body, &operation("Catalog.Demo", "Delete", false), 1)
                .unwrap();
        assert!(replaced.changed);
        assert!(with_false.contains("<value>false</value>"), "{with_false}");
        validate_every_role_value(&with_false).unwrap();

        // Возврат к true — переход к умолчанию: право уходит из хранения.
        let (canonical, removed) =
            apply_role_edit_operation(&with_false, &operation("Catalog.Demo", "Delete", true), 2)
                .unwrap();
        assert!(removed.changed);
        assert_eq!(removed.before, Some(false));
        let document = Document::parse(&canonical).unwrap();
        let object =
            direct_role_children(document.root_element(), "object", ROLE_RIGHTS_NAMESPACE)[0];
        assert!(
            !direct_role_children(object, "right", ROLE_RIGHTS_NAMESPACE)
                .into_iter()
                .any(|right| {
                    role_text_content(direct_role_children(right, "name", ROLE_RIGHTS_NAMESPACE)[0])
                        == "Delete"
                }),
            "{canonical}"
        );
        drop(document);
        validate_every_role_value(&canonical).unwrap();
    }

    #[test]
    fn missing_object_is_error_except_absent_data_processor_use_false_noop() {
        let body = rights_xml("\n", "<value>true</value>");

        let missing =
            apply_role_edit_operation(&body, &operation("Catalog.Missing", "Delete", true), 0)
                .unwrap_err();
        assert!(missing.contains("was not found"), "{missing}");

        // Умолчание роли на неперечисленном объекте уже действует: запись
        // того же значения — законный no-op, а не отказ.
        let (noop_text, noop) =
            apply_role_edit_operation(&body, &operation("Catalog.Missing", "Delete", false), 0)
                .unwrap();
        assert_eq!(noop_text, body);
        assert!(!noop.changed);
        assert_eq!(noop.before, None);
        assert!(!noop.after);
        assert_eq!(noop.action, RoleEditEffectAction::SetRight);

        let (without_processor, first) =
            apply_role_edit_operation(&body, &operation("DataProcessor.Worker", "Use", false), 1)
                .unwrap();
        assert!(first.changed);
        assert_eq!(first.action, RoleEditEffectAction::RemoveObject);
        assert!(!without_processor.contains("DataProcessor.Worker"));

        let (repeated, second) = apply_role_edit_operation(
            &without_processor,
            &operation("DataProcessor.Worker", "Use", false),
            2,
        )
        .unwrap();
        assert_eq!(repeated, without_processor);
        assert!(!second.changed);
        assert_eq!(second.before, None);
        assert_eq!(second.action, RoleEditEffectAction::RemoveObject);
    }

    #[test]
    fn external_data_source_table_right_round_trips_through_writer() {
        let body = r#"<Rights xmlns="http://v8.1c.ru/8.2/roles" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" xsi:type="Rights" version="2.20">
  <setForNewObjects>false</setForNewObjects>
  <setForAttributesByDefault>true</setForAttributesByDefault>
  <independentRightsOfChildObjects>false</independentRightsOfChildObjects>
  <object>
    <name>ExternalDataSource.Remote.Table.Items</name>
    <right><name>Read</name><value>true</value></right>
  </object>
</Rights>
"#;
        let (updated, effect) = apply_role_edit_operation(
            body,
            &operation(
                "ExternalDataSource.Remote.Table.Items",
                "InputByString",
                true,
            ),
            0,
        )
        .unwrap();

        assert!(effect.changed);
        assert!(updated.contains("<name>InputByString</name>"), "{updated}");
        validate_every_role_value(&updated).unwrap();
    }

    fn fixture(name: &str) -> (WorkspaceContext, Map<String, Value>, PathBuf) {
        let root = std::env::temp_dir().join(format!(
            "unica-role-edit-{name}-{}-{}",
            std::process::id(),
            NONCE.fetch_add(1, Ordering::Relaxed)
        ));
        let rights = root.join("src/Roles/Demo/Ext/Rights.xml");
        fs::create_dir_all(rights.parent().unwrap()).unwrap();
        fs::write(
            root.join("v8project.yaml"),
            "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
        )
        .unwrap();
        fs::write(
            root.join("src/Configuration.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Configuration uuid="aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa"><Properties><Name>Main</Name></Properties><ChildObjects><Role>Demo</Role></ChildObjects></Configuration></MetaDataObject>"#,
        )
        .unwrap();
        fs::write(
            root.join("src/Roles/Demo.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Role uuid="bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb"><Properties><Name>Demo</Name></Properties></Role></MetaDataObject>"#,
        )
        .unwrap();
        fs::write(
            &rights,
            encode_role_xml(true, &rights_xml("\r\n", "<value>true</value>")),
        )
        .unwrap();
        let context = WorkspaceContext {
            cwd: root.clone(),
            workspace_root: root.clone(),
            cache_root: root.join(".build/unica"),
            workspace_epoch: 1,
        };
        let args = json!({
            "sourceSet": "main",
            "metadataPath": "Role.Demo",
            "operations": [
                {"op":"setRight", "objectName":"Catalog.Demo", "right":"Delete", "value":false},
                {"op":"setRight", "objectName":"DataProcessor.Worker", "right":"Use", "value":false}
            ]
        })
        .as_object()
        .unwrap()
        .clone();
        (context, args, rights)
    }

    fn replace_format_version(raw: &[u8], replacement: Option<&str>) -> Vec<u8> {
        let marker = b" version=\"2.20\"";
        let offset = raw
            .windows(marker.len())
            .position(|window| window == marker)
            .expect("fixture carries exact format version");
        let mut updated = raw.to_vec();
        let replacement = replacement
            .map(|version| format!(" version=\"{version}\""))
            .unwrap_or_default();
        updated.splice(
            offset..offset + marker.len(),
            replacement.as_bytes().iter().copied(),
        );
        updated
    }

    #[test]
    fn a_mid_sequence_failure_publishes_nothing_and_names_its_operation() {
        // Последовательность операций — одна транзакция: отказ на второй не
        // должен оставить применённой первую.
        let (context, mut args, rights) = fixture("mid-sequence-failure");
        let before = fs::read(&rights).unwrap();
        args.insert(
            "operations".to_string(),
            json!([
                {"op":"setRight", "objectName":"Catalog.Demo", "right":"Delete", "value":false},
                {"op":"setRight", "objectName":"Catalog.Missing", "right":"Read", "value":true},
                {"op":"setRight", "objectName":"DataProcessor.Worker", "right":"Use", "value":false}
            ]),
        );

        let applied = apply_edit_with_data(&args, &context);

        assert!(!applied.outcome.ok, "{:?}", applied.outcome.errors);
        assert_eq!(
            fs::read(&rights).unwrap(),
            before,
            "провалившийся вызов не имеет права публиковать первую операцию"
        );
        let data = applied.data.expect("typed failure data");
        assert_eq!(data.diagnostics.len(), 1);
        assert_eq!(data.diagnostics[0].code, "object_not_listed");
        assert_eq!(
            data.diagnostics[0].operation_index,
            Some(1),
            "отказ назван своим индексом: {:?}",
            data.diagnostics
        );
        assert!(!data.changed);
        assert_eq!(
            data.validation.status,
            crate::domain::role::RoleEditValidationStatus::Failed
        );
        let encoded = serde_json::to_string(&data).unwrap();
        assert!(!encoded.contains("Rights.xml"), "{encoded}");
        fs::remove_dir_all(context.workspace_root).unwrap();
    }

    #[test]
    fn parse_failure_never_fabricates_a_role_metadata_path() {
        let (context, mut args, _) = fixture("invalid-metadata-path");

        args.remove("metadataPath");
        let missing = preview_edit_with_data(&args, &context);
        assert!(!missing.outcome.ok);
        assert!(missing.data.is_none());

        args.insert("metadataPath".to_string(), json!(42));
        let non_string = preview_edit_with_data(&args, &context);
        assert!(!non_string.outcome.ok);
        assert!(non_string.data.is_none());

        args.insert("metadataPath".to_string(), json!("Role.123"));
        let supplied = preview_edit_with_data(&args, &context);
        assert!(!supplied.outcome.ok);
        assert_eq!(supplied.data.unwrap().metadata_path, "Role.123");

        fs::remove_dir_all(context.workspace_root).unwrap();
    }

    #[test]
    fn role_edit_without_vendor_support_is_logically_addressed_and_idempotent() {
        let (context, args, rights) = fixture("roundtrip");
        assert!(
            !context
                .workspace_root
                .join("src/Ext/ParentConfigurations.bin")
                .exists(),
            "the no-support case must not accidentally inherit vendor support evidence"
        );
        let before = fs::read(&rights).unwrap();
        let preview = preview_edit_with_data(&args, &context);
        assert!(preview.outcome.ok, "{:?}", preview.outcome);
        assert!(preview.outcome.stdout.is_none());
        assert!(preview.outcome.stderr.is_none());
        assert_eq!(preview.outcome.artifacts, ["main + Role.Demo"]);
        let preview_data = preview.data.unwrap();
        assert!(preview_data.changed);
        assert_eq!(preview_data.effects.len(), 2);
        assert_eq!(preview_data.effects[0].operation_index, 0);
        assert_eq!(preview_data.effects[1].operation_index, 1);
        assert_eq!(fs::read(&rights).unwrap(), before);

        let applied = apply_edit_with_data(&args, &context);
        assert!(applied.outcome.ok, "{:?}", applied.outcome);
        assert_eq!(applied.events.len(), 1);
        assert_eq!(applied.events[0].kind, DomainEventKind::RoleChanged);
        let cache = applied
            .recorded_cache
            .as_ref()
            .expect("applied source and cache state commit together");
        assert_eq!(cache.events, ["RoleChanged"]);
        assert!(cache.invalidated.contains(&"rights_graph".to_string()));
        assert!(cache.refreshed.contains(&"rights_graph".to_string()));
        assert!(applied.data.unwrap().changed);
        let after = fs::read(&rights).unwrap();
        assert_ne!(after, before);
        assert!(after.starts_with(&[0xEF, 0xBB, 0xBF]));
        assert!(!String::from_utf8_lossy(&after).contains("DataProcessor.Worker"));

        let repeated = apply_edit_with_data(&args, &context);
        assert!(repeated.outcome.ok, "{:?}", repeated.outcome);
        assert!(repeated.events.is_empty());
        assert!(repeated.recorded_cache.is_none());
        assert!(!repeated.data.unwrap().changed);
        assert_eq!(fs::read(&rights).unwrap(), after);
        fs::remove_dir_all(context.workspace_root).unwrap();
    }

    #[test]
    fn exact_profile_rejects_rights_and_descriptor_versions_outside_2_20() {
        let (context, args, rights) = fixture("exact-profile");
        let original = fs::read(&rights).unwrap();

        for version in [Some("2.19"), Some("2.21"), Some("2.2&#48;"), None] {
            let incompatible = replace_format_version(&original, version);
            fs::write(&rights, &incompatible).unwrap();
            let rejected = apply_edit_with_data(&args, &context);
            assert!(!rejected.outcome.ok, "version={version:?}");
            assert_eq!(fs::read(&rights).unwrap(), incompatible);
        }

        fs::write(&rights, &original).unwrap();
        let descriptor = context.workspace_root.join("src/Roles/Demo.xml");
        let original_descriptor = fs::read(&descriptor).unwrap();
        for version in [Some("2.19"), Some("2.21"), Some("2.2&#48;"), None] {
            let incompatible = replace_format_version(&original_descriptor, version);
            fs::write(&descriptor, &incompatible).unwrap();
            let rejected = apply_edit_with_data(&args, &context);
            assert!(!rejected.outcome.ok, "descriptor version={version:?}");
            assert_eq!(fs::read(&rights).unwrap(), original);
            assert_eq!(fs::read(&descriptor).unwrap(), incompatible);
        }
        fs::write(&descriptor, original_descriptor).unwrap();
        fs::remove_dir_all(context.workspace_root).unwrap();
    }

    #[test]
    fn rights_drift_in_the_staging_window_is_classified_as_concurrent() {
        let (context, args, rights) = fixture("staging-byte-drift");
        let changed_rights = rights.clone();
        let changed = with_role_edit_after_rights_reread_hook(
            move || {
                let mut bytes = fs::read(&changed_rights).unwrap();
                bytes.extend_from_slice(b"<!-- concurrent -->");
                fs::write(&changed_rights, bytes).unwrap();
            },
            || apply_edit_with_data(&args, &context),
        );
        assert!(!changed.outcome.ok);
        assert!(changed.outcome.errors[0].starts_with("concurrent_modification:"));
        assert!(fs::read(&rights).unwrap().ends_with(b"<!-- concurrent -->"));
        assert!(changed.events.is_empty());
        assert!(changed.recorded_cache.is_none());
        fs::remove_dir_all(context.workspace_root).unwrap();

        let (context, args, rights) = fixture("staging-disappearance");
        let removed_rights = rights.clone();
        let removed = with_role_edit_after_rights_reread_hook(
            move || fs::remove_file(&removed_rights).unwrap(),
            || apply_edit_with_data(&args, &context),
        );
        assert!(!removed.outcome.ok);
        assert!(removed.outcome.errors[0].starts_with("concurrent_modification:"));
        assert!(!rights.exists());
        assert!(removed.events.is_empty());
        assert!(removed.recorded_cache.is_none());
        fs::remove_dir_all(context.workspace_root).unwrap();

        let (context, args, rights) = fixture("staging-symlink-swap");
        let original = fs::read(&rights).unwrap();
        let saved = rights.with_extension("saved");
        let outside = context.workspace_root.join("outside-rights.xml");
        fs::write(&outside, &original).unwrap();
        let Some(probe) = create_file_symlink_for_test(&outside, &saved) else {
            fs::remove_dir_all(context.workspace_root).unwrap();
            return;
        };
        if probe.is_err() {
            fs::remove_dir_all(context.workspace_root).unwrap();
            return;
        }
        fs::remove_file(&saved).unwrap();
        let swapped_rights = rights.clone();
        let swapped_saved = saved.clone();
        let swapped_outside = outside.clone();
        let swapped = with_role_edit_after_rights_reread_hook(
            move || {
                fs::rename(&swapped_rights, &swapped_saved).unwrap();
                create_file_symlink_for_test(&swapped_outside, &swapped_rights)
                    .expect("file links are available on this test host")
                    .expect("test host permits a file link");
            },
            || apply_edit_with_data(&args, &context),
        );
        assert!(!swapped.outcome.ok);
        assert!(swapped.outcome.errors[0].starts_with("concurrent_modification:"));
        assert_eq!(fs::read(&outside).unwrap(), original);
        assert_eq!(fs::read(&saved).unwrap(), original);
        assert!(swapped.events.is_empty());
        assert!(swapped.recorded_cache.is_none());
        fs::remove_file(&rights).unwrap();
        fs::remove_dir_all(context.workspace_root).unwrap();
    }

    #[test]
    fn descriptor_configuration_and_rights_preimage_drift_and_post_validation_are_failure_atomic() {
        let (context, args, rights) = fixture("transaction-guards");
        let original = fs::read(&rights).unwrap();

        let concurrent = {
            let rights = rights.clone();
            with_role_edit_before_publish_hook(
                move || {
                    let mut changed = fs::read(&rights).unwrap();
                    changed.extend_from_slice(b"<!-- concurrent -->");
                    fs::write(&rights, changed).unwrap();
                },
                || apply_edit_with_data(&args, &context),
            )
        };
        assert!(!concurrent.outcome.ok);
        assert!(
            concurrent.outcome.errors[0].starts_with("concurrent_modification:"),
            "{:?}",
            concurrent.outcome
        );
        let concurrent_bytes = fs::read(&rights).unwrap();
        assert!(concurrent_bytes.ends_with(b"<!-- concurrent -->"));

        fs::write(&rights, &original).unwrap();
        let descriptor = context.workspace_root.join("src/Roles/Demo.xml");
        let descriptor_before = fs::read(&descriptor).unwrap();
        let descriptor_drift = {
            let descriptor = descriptor.clone();
            with_role_edit_before_publish_hook(
                move || {
                    let mut changed = fs::read(&descriptor).unwrap();
                    changed.extend_from_slice(b"<!-- descriptor drift -->");
                    fs::write(&descriptor, changed).unwrap();
                },
                || apply_edit_with_data(&args, &context),
            )
        };
        assert!(!descriptor_drift.outcome.ok);
        assert!(
            descriptor_drift.outcome.errors[0].starts_with("concurrent_modification:"),
            "{:?}",
            descriptor_drift.outcome
        );
        assert_eq!(fs::read(&rights).unwrap(), original);
        assert_ne!(fs::read(&descriptor).unwrap(), descriptor_before);
        fs::write(&descriptor, descriptor_before).unwrap();

        let owner = context.workspace_root.join("src/Configuration.xml");
        let owner_before = fs::read(&owner).unwrap();
        let owner_drift = {
            let owner = owner.clone();
            with_role_edit_before_publish_hook(
                move || {
                    let mut changed = fs::read(&owner).unwrap();
                    changed.extend_from_slice(b"<!-- owner drift -->");
                    fs::write(&owner, changed).unwrap();
                },
                || apply_edit_with_data(&args, &context),
            )
        };
        assert!(!owner_drift.outcome.ok);
        assert!(
            owner_drift.outcome.errors[0].starts_with("concurrent_modification:"),
            "{:?}",
            owner_drift.outcome
        );
        assert_eq!(fs::read(&rights).unwrap(), original);
        assert_ne!(fs::read(&owner).unwrap(), owner_before);
        fs::write(&owner, owner_before).unwrap();

        fs::write(&rights, &original).unwrap();
        let rolled_back =
            with_role_edit_post_validation_failure(|| apply_edit_with_data(&args, &context));
        assert!(!rolled_back.outcome.ok);
        assert!(
            rolled_back.outcome.errors[0].starts_with("provider_unavailable:"),
            "{:?}",
            rolled_back.outcome
        );
        assert_eq!(fs::read(&rights).unwrap(), original);
        assert!(!context.cache_root.join("state.json").exists());
        assert!(!context
            .cache_root
            .join("caches/metadata_graph.json")
            .exists());
        assert!(!context.cache_root.join("caches/rights_graph.json").exists());
        fs::remove_dir_all(context.workspace_root).unwrap();
    }

    #[test]
    fn cache_planning_failure_leaves_role_source_unchanged() {
        let (context, args, rights) = fixture("cache-planning-failure");
        let before = fs::read(&rights).unwrap();
        fs::create_dir_all(context.cache_root.join("state.json")).unwrap();
        let rejected = apply_edit_with_data(&args, &context);
        assert!(!rejected.outcome.ok, "{:?}", rejected.outcome);
        assert!(rejected.outcome.errors[0].contains("cache_publication_failed"));
        assert_eq!(fs::read(&rights).unwrap(), before);
        fs::remove_dir_all(context.workspace_root).unwrap();
    }

    #[test]
    fn role_edit_rejects_invalid_right_for_nested_object_without_changing_rights() {
        let (context, args, rights) = fixture("invalid-nested-right");
        let before = fs::read(&rights).unwrap();

        let mut nested = args.clone();
        nested.insert(
            "operations".to_string(),
            json!([{
                "op":"setRight",
                "objectName":"DataProcessor.Worker.Command.Run",
                "right":"Use",
                "value":false
            }]),
        );
        let invalid = apply_edit_with_data(&nested, &context);
        assert!(!invalid.outcome.ok, "{:?}", invalid.outcome);
        let data = invalid.data.expect("typed failure data");
        assert_eq!(data.diagnostics.len(), 1);
        assert_eq!(data.diagnostics[0].code, "unsupported_right");
        assert_eq!(data.diagnostics[0].operation_index, Some(0));
        assert!(
            data.diagnostics[0]
                .message
                .contains("DataProcessor.Worker.Command.Run"),
            "{:?}",
            data.diagnostics
        );
        assert_eq!(fs::read(&rights).unwrap(), before);
        fs::remove_dir_all(context.workspace_root).unwrap();
    }

    #[test]
    fn role_edit_rejects_locked_vendor_supported_role_without_changing_rights() {
        let (context, args, rights) = fixture("locked-vendor-supported-role");
        let before = fs::read(&rights).unwrap();

        fs::create_dir_all(context.workspace_root.join("src/Ext")).unwrap();
        fs::write(
            context
                .workspace_root
                .join("src/Ext/ParentConfigurations.bin"),
            concat!(
                "\u{feff}{6,0,1,dddddddd-dddd-dddd-dddd-dddddddddddd,0,",
                "eeeeeeee-eeee-eeee-eeee-eeeeeeeeeeee,\"1.0\",\"Vendor\",",
                "\"VendorConf\",3,1,0,aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa,",
                "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa,0,0,",
                "bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb,",
                "bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb,2,0,",
                "cccccccc-cccc-cccc-cccc-cccccccccccc,",
                "cccccccc-cccc-cccc-cccc-cccccccccccc}"
            )
            .as_bytes(),
        )
        .unwrap();
        let denied = apply_edit_with_data(&args, &context);
        assert!(!denied.outcome.ok, "{:?}", denied.outcome);
        assert!(denied.outcome.errors[0].contains("support_locked"));
        assert_eq!(fs::read(&rights).unwrap(), before);
        fs::remove_dir_all(context.workspace_root).unwrap();
    }

    #[test]
    fn role_edit_changes_editable_vendor_supported_role_addressed_logically() {
        let (context, args, rights) = fixture("editable-vendor-supported-role");
        fs::create_dir_all(context.workspace_root.join("src/Ext")).unwrap();
        fs::write(
            context
                .workspace_root
                .join("src/Ext/ParentConfigurations.bin"),
            concat!(
                "\u{feff}{6,0,1,dddddddd-dddd-dddd-dddd-dddddddddddd,0,",
                "eeeeeeee-eeee-eeee-eeee-eeeeeeeeeeee,\"1.0\",\"Vendor\",",
                "\"VendorConf\",3,1,0,aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa,",
                "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa,1,0,",
                "bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb,",
                "bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb,2,0,",
                "cccccccc-cccc-cccc-cccc-cccccccccccc,",
                "cccccccc-cccc-cccc-cccc-cccccccccccc}"
            )
            .as_bytes(),
        )
        .unwrap();

        let support = WorkspaceSupportStateReader::new(&context)
            .object_support(&crate::domain::source_target::ResolvedTarget {
                source_set: "main".to_string(),
                metadata_path: Some(
                    MetadataAddress::parse(PLATFORM_XML_8_3_27_FORMAT_2_20, "Role.Demo").unwrap(),
                ),
                target_kind: TargetKind::MetadataObject,
            })
            .unwrap();
        assert_eq!(
            support.state,
            crate::domain::support_state::ObjectSupportState::EditableWithSupport
        );
        assert_eq!(support.direct_edit_safe, Some(true));

        let expected_body = [
            r#"<Rights xmlns="http://v8.1c.ru/8.2/roles" xmlns:r="http://v8.1c.ru/8.2/roles" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" xsi:type="Rights" version="2.20">"#,
            "\t<setForNewObjects>false</setForNewObjects>",
            "\t<setForAttributesByDefault>true</setForAttributesByDefault>",
            "\t<independentRightsOfChildObjects>false</independentRightsOfChildObjects>",
            "\t<object>",
            "\t\t<name>Catalog.Demo</name>",
            "\t\t<future><name>Delete</name><value>false</value></future>",
            "\t\t<right><name>FuturePlatformRight</name><value>true</value><future/></right>",
            "\t</object>",
            "\t<restrictionTemplate><name>Keep</name><condition>TRUE</condition></restrictionTemplate>",
            "</Rights>",
        ]
        .join("\r\n");
        let expected = encode_role_xml(true, &expected_body);

        let changed = apply_edit_with_data(&args, &context);

        assert!(changed.outcome.ok, "{:?}", changed.outcome);
        assert_eq!(fs::read(&rights).unwrap(), expected);
        assert!(changed.data.expect("typed success data").changed);
        fs::remove_dir_all(context.workspace_root).unwrap();
    }

    #[test]
    fn publish_time_allow_to_warn_transition_is_reported_once() {
        let (context, args, _rights) = fixture("allow-to-warn");
        let workspace = context.workspace_root.clone();
        let changed = with_role_edit_before_native_guard_hook(
            move || {
                fs::create_dir_all(workspace.join("src/Ext")).unwrap();
                fs::write(
                    workspace.join("src/Ext/ParentConfigurations.bin"),
                    concat!(
                        "\u{feff}{6,0,1,dddddddd-dddd-dddd-dddd-dddddddddddd,0,",
                        "eeeeeeee-eeee-eeee-eeee-eeeeeeeeeeee,\"1.0\",\"Vendor\",",
                        "\"VendorConf\",3,1,0,aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa,",
                        "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa,0,0,",
                        "bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb,",
                        "bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb,2,0,",
                        "cccccccc-cccc-cccc-cccc-cccccccccccc,",
                        "cccccccc-cccc-cccc-cccc-cccccccccccc}"
                    )
                    .as_bytes(),
                )
                .unwrap();
                fs::write(
                    workspace.join(".v8-project.json"),
                    r#"{"editingAllowedCheck":"warn"}"#,
                )
                .unwrap();
            },
            || apply_edit_with_data(&args, &context),
        );

        assert!(changed.outcome.ok, "{:?}", changed.outcome);
        assert_eq!(
            changed
                .outcome
                .warnings
                .iter()
                .filter(|warning| warning.contains("support_guard_warning"))
                .count(),
            1
        );
        fs::remove_dir_all(context.workspace_root).unwrap();
    }

    #[test]
    fn cleanup_warning_is_success_after_validated_committed_bytes() {
        let (context, args, rights) = fixture("cleanup-warning");
        let before = fs::read(&rights).unwrap();
        let result = with_publish_failpoints(&[PublishCheckpoint::Cleanup], || {
            apply_edit_with_data(&args, &context)
        });
        assert!(result.outcome.ok, "{:?}", result.outcome);
        assert!(result
            .outcome
            .warnings
            .iter()
            .any(|warning| warning.contains("publication_cleanup_incomplete")));
        assert_ne!(fs::read(&rights).unwrap(), before);
        fs::remove_dir_all(context.workspace_root).unwrap();
    }

    #[test]
    fn role_resource_symlink_swap_after_planning_is_rejected_before_publication() {
        let (context, args, rights) = fixture("symlink-swap");
        let role_dir = rights.parent().unwrap().parent().unwrap().to_path_buf();
        let saved_role = context.workspace_root.join("saved-role");
        let outside = context.workspace_root.join("outside-role");
        let outside_rights = outside.join("Ext/Rights.xml");
        fs::create_dir_all(outside_rights.parent().unwrap()).unwrap();
        let before = fs::read(&rights).unwrap();
        fs::write(&outside_rights, &before).unwrap();
        let probe_source = context.workspace_root.join("link-probe-source");
        let probe_link = context.workspace_root.join("link-probe");
        fs::create_dir(&probe_source).unwrap();
        let Some(probe_result) = create_dir_symlink_for_test(&probe_source, &probe_link) else {
            fs::remove_dir_all(context.workspace_root).unwrap();
            return;
        };
        if probe_result.is_err() {
            fs::remove_dir_all(context.workspace_root).unwrap();
            return;
        }
        remove_dir_symlink_for_test(&probe_link).unwrap();

        let swapped_role = role_dir.clone();
        let swapped_outside = outside.clone();
        let swapped_saved = saved_role.clone();
        let rejected = with_role_edit_before_publish_hook(
            move || {
                fs::rename(&swapped_role, &swapped_saved).unwrap();
                create_dir_symlink_for_test(&swapped_outside, &swapped_role)
                    .expect("directory links are available on supported hosts")
                    .expect("test host must permit a directory link");
            },
            || apply_edit_with_data(&args, &context),
        );
        assert!(!rejected.outcome.ok, "{:?}", rejected.outcome);
        assert_eq!(fs::read(&outside_rights).unwrap(), before);
        assert_eq!(fs::read(saved_role.join("Ext/Rights.xml")).unwrap(), before);
        remove_dir_symlink_for_test(&role_dir).unwrap();
        fs::remove_dir_all(context.workspace_root).unwrap();
    }

    #[test]
    fn symlinked_role_resource_is_rejected_without_following_it() {
        let (context, args, rights) = fixture("symlink");
        let role_dir = rights.parent().unwrap().parent().unwrap().to_path_buf();
        let outside = context.workspace_root.join("outside-role");
        fs::create_dir_all(outside.join("Ext")).unwrap();
        let outside_rights = outside.join("Ext/Rights.xml");
        let before = fs::read(&rights).unwrap();
        fs::write(&outside_rights, &before).unwrap();
        fs::remove_dir_all(&role_dir).unwrap();
        let Some(link_result) = create_dir_symlink_for_test(&outside, &role_dir) else {
            fs::remove_dir_all(context.workspace_root).unwrap();
            return;
        };
        if link_result.is_err() {
            fs::remove_dir_all(context.workspace_root).unwrap();
            return;
        }

        let rejected = apply_edit_with_data(&args, &context);
        assert!(!rejected.outcome.ok);
        assert_eq!(fs::read(&outside_rights).unwrap(), before);
        remove_dir_symlink_for_test(&role_dir).unwrap();
        fs::remove_dir_all(context.workspace_root).unwrap();
    }
}

#[cfg(test)]
mod role_info_typed_result_tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn workspace(name: &str) -> WorkspaceContext {
        let root = std::env::temp_dir().join(format!(
            "unica-role-info-typed-{name}-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(root.join("src/Roles/Reader/Ext")).unwrap();
        fs::write(
            root.join("src/Configuration.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Configuration><Properties><Name>Demo</Name></Properties><ChildObjects><Role>Reader</Role></ChildObjects></Configuration></MetaDataObject>"#,
        )
        .unwrap();
        fs::write(
            root.join("src/Roles/Reader.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Role><Properties><Name>Reader</Name></Properties></Role></MetaDataObject>"#,
        )
        .unwrap();
        fs::write(
            root.join("src/Roles/Reader/Ext/Rights.xml"),
            r#"<Rights xmlns="http://v8.1c.ru/8.2/roles" setForNewObjects="false" setForAttributesByDefault="true" independentRightsOfChildObjects="false">
  <object><name>Catalog.Goods</name>
    <right><name>Read</name><value>true</value><restrictionByCondition><condition>ГДЕ Ложь</condition></restrictionByCondition></right>
    <right><name>Insert</name><value>false</value></right>
  </object>
</Rights>"#,
        )
        .unwrap();
        WorkspaceContext {
            cwd: root.clone(),
            workspace_root: root.clone(),
            cache_root: root.join(".build/unica"),
            workspace_epoch: 1,
        }
    }

    /// The support state belongs to the object, and `Rights.xml` sits two
    /// directories below the configuration root. Reading it from the leaf path
    /// answered `notSupported` for a configuration that is on support.
    #[test]
    fn role_info_reads_the_support_state_from_the_configuration_root() {
        let context = workspace("support");
        fs::write(
            context.workspace_root.join("src/Configuration.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Configuration><Properties><Name>Demo</Name></Properties><ChildObjects><Role>Reader</Role></ChildObjects></Configuration></MetaDataObject>"#,
        )
        .unwrap();
        let args = Map::from_iter([(
            "RightsPath".to_string(),
            json!("src/Roles/Reader/Ext/Rights.xml"),
        )]);

        let execution =
            analyze_role_info(&args, &context, &WorkspaceSupportStateReader::new(&context));

        assert!(execution.outcome.ok, "{:?}", execution.outcome);
        let data = execution.data.expect("role info answers with data");
        // No ParentConfigurations.bin here, so the honest answer is
        // `notSupported` — but it must come from the resolved configuration
        // root, not from a directory the walk never reached.
        assert_eq!(
            data.support.state,
            crate::domain::support_state::ObjectSupportState::NotSupported,
            "{data:?}"
        );
        assert_eq!(data.support.direct_edit_safe, None, "{data:?}");
        let _ = fs::remove_dir_all(&context.workspace_root);
    }

    /// `ShowDenied` used to decide whether denied rights appeared at all, so an
    /// answer without them could mean "none" or "you did not ask".
    #[test]
    fn role_info_reports_allowed_and_denied_rights_without_a_flag() {
        let context = workspace("both");
        let args = Map::from_iter([(
            "RightsPath".to_string(),
            json!("src/Roles/Reader/Ext/Rights.xml"),
        )]);

        let execution =
            analyze_role_info(&args, &context, &WorkspaceSupportStateReader::new(&context));

        assert!(execution.outcome.ok, "{:?}", execution.outcome);
        assert!(execution.outcome.stdout.is_none());
        let data = execution.data.expect("role info answers with data");
        assert_eq!(data.name, "Reader");
        assert_eq!(data.totals.allowed, 1);
        assert_eq!(data.totals.denied, 1);
        assert_eq!(data.defaults.set_for_new_objects.as_deref(), Some("false"));
        let allowed = &data.allowed[0].objects[0];
        assert_eq!(allowed.name, "Goods");
        assert!(allowed.rights.iter().any(|right| right.restricted));
        assert!(!data.denied.is_empty(), "denied rights are always reported");
        assert_eq!(data.restricted_objects.len(), 1);
        let _ = fs::remove_dir_all(&context.workspace_root);
    }

    /// The same dump, made addressable: a project map naming the source set and
    /// a configuration that registers the role.
    fn addressable_workspace(name: &str) -> WorkspaceContext {
        let context = workspace(name);
        fs::write(
            context.workspace_root.join("v8project.yaml"),
            "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
        )
        .unwrap();
        fs::write(
            context.workspace_root.join("src/Configuration.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Configuration><Properties><Name>Demo</Name></Properties><ChildObjects><Role>Reader</Role></ChildObjects></Configuration></MetaDataObject>"#,
        )
        .unwrap();
        context
    }

    #[test]
    fn role_info_answers_identically_for_a_logical_and_a_physical_selector() {
        let context = addressable_workspace("bridge");

        let physical = analyze_role_info(
            &Map::from_iter([(
                "RightsPath".to_string(),
                json!("src/Roles/Reader/Ext/Rights.xml"),
            )]),
            &context,
            &WorkspaceSupportStateReader::new(&context),
        );
        let logical = analyze_role_info(
            &Map::from_iter([
                ("sourceSet".to_string(), json!("main")),
                ("metadataPath".to_string(), json!("Role.Reader")),
            ]),
            &context,
            &WorkspaceSupportStateReader::new(&context),
        );

        assert!(logical.outcome.ok, "{:?}", logical.outcome);
        assert!(physical.outcome.ok, "{:?}", physical.outcome);
        // Compare what the caller actually receives: the serialized
        // answer, not a struct the wire never carries.
        assert_eq!(
            serde_json::to_value(&logical.data).unwrap(),
            serde_json::to_value(&physical.data).unwrap()
        );
        let _ = fs::remove_dir_all(&context.workspace_root);
    }

    #[test]
    fn role_info_accepts_the_russian_kind_alias() {
        let context = addressable_workspace("bridge-alias");

        let logical = analyze_role_info(
            &Map::from_iter([
                ("sourceSet".to_string(), json!("main")),
                ("metadataPath".to_string(), json!("Роль.Reader")),
            ]),
            &context,
            &WorkspaceSupportStateReader::new(&context),
        );

        assert!(logical.outcome.ok, "{:?}", logical.outcome);
        assert_eq!(
            logical.data.expect("role info answers with data").name,
            "Reader"
        );
        let _ = fs::remove_dir_all(&context.workspace_root);
    }

    #[test]
    fn role_info_refuses_an_address_that_is_not_a_role() {
        let context = addressable_workspace("bridge-wrong-kind");

        let outcome = analyze_role_info(
            &Map::from_iter([
                ("sourceSet".to_string(), json!("main")),
                ("metadataPath".to_string(), json!("Catalog.Goods")),
            ]),
            &context,
            &WorkspaceSupportStateReader::new(&context),
        )
        .outcome;

        assert!(!outcome.ok);
        assert!(
            outcome
                .errors
                .iter()
                .chain(std::iter::once(&outcome.summary))
                .any(|message| message.contains("target_kind_unsupported")),
            "{outcome:?}"
        );
        let _ = fs::remove_dir_all(&context.workspace_root);
    }

    #[test]
    fn role_validate_answers_identically_for_a_logical_and_a_physical_selector() {
        let context = addressable_workspace("bridge-validate");

        let physical = validate_role(
            &Map::from_iter([(
                "RightsPath".to_string(),
                json!("src/Roles/Reader/Ext/Rights.xml"),
            )]),
            &context,
        );
        let logical = validate_role(
            &Map::from_iter([
                ("sourceSet".to_string(), json!("main")),
                ("metadataPath".to_string(), json!("Role.Reader")),
            ]),
            &context,
        );

        assert_eq!(logical.ok, physical.ok, "{logical:?} vs {physical:?}");
        assert_eq!(logical.errors, physical.errors);
        let _ = fs::remove_dir_all(&context.workspace_root);
    }
}

#[cfg(test)]
mod role_compile_contract_tests {
    use super::super::compile_transaction::{with_commit_failpoint, CommitFailpoint};
    use super::super::single_file_publisher::with_before_commit_hook;
    use super::*;
    use crate::application::UnicaApplication;

    fn temp_root(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "unica-role-{name}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&root).unwrap();
        root
    }

    fn context(root: &Path) -> WorkspaceContext {
        WorkspaceContext {
            cwd: root.to_path_buf(),
            workspace_root: root.to_path_buf(),
            cache_root: root.join(".build/unica"),
            workspace_epoch: 1,
        }
    }

    fn compile_args(definition: &Path, output_dir: &Path) -> Map<String, Value> {
        Map::from_iter([
            (
                "JsonPath".to_string(),
                Value::String(definition.display().to_string()),
            ),
            (
                "OutputDir".to_string(),
                Value::String(output_dir.display().to_string()),
            ),
        ])
    }

    fn write_definition(root: &Path, definition: &Value) -> PathBuf {
        let path = root.join("role.json");
        fs::write(&path, serde_json::to_vec_pretty(definition).unwrap()).unwrap();
        path
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
            "\t\t<Properties><Name>Demo</Name><ConfigurationExtensionCompatibilityMode>Version8_3_27</ConfigurationExtensionCompatibilityMode><DefaultLanguage>Language.English</DefaultLanguage></Properties>\r\n",
            "\t\t<ChildObjects><Language>English</Language><Catalog>Items</Catalog></ChildObjects>\r\n",
            "\t</Configuration>\r\n",
            "</MetaDataObject><!--exact-tail-->"
        );
        let mut bytes = b"\xef\xbb\xbf".to_vec();
        bytes.extend_from_slice(text.as_bytes());
        bytes
    }

    fn write_configuration(root: &Path) -> Vec<u8> {
        let bytes = configuration_bytes();
        fs::create_dir_all(root.join("Languages")).unwrap();
        fs::write(root.join("Languages/English.xml"), b"language marker").unwrap();
        fs::write(root.join("Configuration.xml"), &bytes).unwrap();
        bytes
    }

    #[test]
    fn role_validate_reports_validation_failures_in_errors() {
        let workspace = temp_root("validate-errors");
        let rights_path = workspace.join("missing-rights.xml");
        let outcome = validate_role(
            &Map::from_iter([(
                "RightsPath".to_string(),
                Value::String(rights_path.display().to_string()),
            )]),
            &context(&workspace),
        );

        assert!(!outcome.ok, "{outcome:?}");
        assert!(
            outcome.errors.join("\n").contains("File not found"),
            "{outcome:?}"
        );
        fs::remove_dir_all(workspace).unwrap();
    }

    #[test]
    fn public_role_compile_rejects_platform_invalid_configuration_owner_without_any_changes() {
        let workspace = temp_root("public-invalid-owner-enum");
        let source = workspace.join("src");
        fs::create_dir_all(&source).unwrap();
        fs::write(
            workspace.join("v8project.yaml"),
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
        let definition = write_definition(&workspace, &json!({ "name": "Reader" }));
        let args = Map::from_iter([
            (
                "cwd".to_string(),
                Value::String(workspace.display().to_string()),
            ),
            ("dryRun".to_string(), Value::Bool(false)),
            (
                "JsonPath".to_string(),
                Value::String(definition.display().to_string()),
            ),
            ("OutputDir".to_string(), Value::String("src".to_string())),
        ]);

        let outcome = UnicaApplication::new()
            .call_tool("unica.role.compile", &args)
            .unwrap();

        assert!(!outcome.ok, "{outcome:?}");
        let errors = outcome.errors.join("\n");
        assert!(
            errors.contains("ConfigurationExtensionCompatibilityMode"),
            "{outcome:?}"
        );
        assert!(errors.contains("Bogus"), "{outcome:?}");
        assert_eq!(fs::read(config_path).unwrap(), invalid_bytes);
        assert!(!source.join("Roles/Reader.xml").exists());
        assert!(!source.join("Roles/Reader/Ext/Rights.xml").exists());
        assert!(!source.join("Roles").exists());
        fs::remove_dir_all(workspace).unwrap();
    }

    #[test]
    fn public_role_compile_prioritizes_newer_existing_target_over_older_configuration() {
        let workspace = temp_root("public-existing-newer-target");
        let source = workspace.join("src");
        fs::create_dir_all(&source).unwrap();
        fs::write(
            workspace.join("v8project.yaml"),
            "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
        )
        .unwrap();
        let older_configuration = String::from_utf8(write_configuration(&source))
            .unwrap()
            .replacen(r#"version="2.20""#, r#"version="2.19""#, 1)
            .into_bytes();
        let config_path = source.join("Configuration.xml");
        fs::write(&config_path, &older_configuration).unwrap();
        let definition_path = write_definition(&workspace, &json!({ "name": "Reader" }));
        let definition = fs::read(&definition_path).unwrap();
        let metadata_path = source.join("Roles/Reader.xml");
        fs::create_dir_all(metadata_path.parent().unwrap()).unwrap();
        let newer_target = br#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.21"><Role/></MetaDataObject>"#.to_vec();
        fs::write(&metadata_path, &newer_target).unwrap();
        let args = Map::from_iter([
            (
                "cwd".to_string(),
                Value::String(workspace.display().to_string()),
            ),
            ("dryRun".to_string(), Value::Bool(false)),
            (
                "JsonPath".to_string(),
                Value::String(definition_path.display().to_string()),
            ),
            ("OutputDir".to_string(), Value::String("src".to_string())),
        ]);

        let outcome = UnicaApplication::new()
            .call_tool("unica.role.compile", &args)
            .unwrap();

        assert!(!outcome.ok, "{outcome:?}");
        let diagnostic = &outcome.diagnostics.as_ref().unwrap()["formatCompatibility"];
        assert_eq!(diagnostic["code"], "platformVersionUnsupported");
        assert_eq!(diagnostic["actualFormat"], "2.21");
        let warning = outcome.warnings.join("\n");
        assert!(warning.contains("1С 8.5"), "{warning}");
        assert!(!warning.contains("миграц"), "{warning}");
        assert!(!warning.contains("повторно выгруз"), "{warning}");
        assert!(!warning.contains("re-export"), "{warning}");
        assert_eq!(fs::read(&config_path).unwrap(), older_configuration);
        assert_eq!(fs::read(&metadata_path).unwrap(), newer_target);
        assert_eq!(fs::read(&definition_path).unwrap(), definition);
        assert!(!source.join("Roles/Reader/Ext/Rights.xml").exists());
        assert!(outcome.changes.is_empty(), "{outcome:?}");
        assert!(outcome.artifacts.is_empty(), "{outcome:?}");
        fs::remove_dir_all(workspace).unwrap();
    }

    #[test]
    fn role_compile_rejects_standalone_newer_configuration_without_creating_role() {
        let workspace = temp_root("standalone-newer-owner");
        let source = workspace.join("src");
        fs::create_dir_all(&source).unwrap();
        let supported = write_configuration(&source);
        let newer = String::from_utf8(supported)
            .unwrap()
            .replace(r#"version="2.20""#, r#"version="2.21""#)
            .into_bytes();
        let config_path = source.join("Configuration.xml");
        fs::write(&config_path, &newer).unwrap();
        let definition = write_definition(&workspace, &json!({ "name": "Reader" }));

        let outcome = compile_role(&compile_args(&definition, &source), &context(&workspace));

        assert!(!outcome.ok, "{outcome:?}");
        let diagnostics = outcome.errors.join("\n");
        assert!(diagnostics.contains("2.21"), "{diagnostics}");
        assert!(diagnostics.contains("1C 8.5"), "{diagnostics}");
        assert_eq!(fs::read(&config_path).unwrap(), newer);
        assert!(!source.join("Roles").exists());
        fs::remove_dir_all(workspace).unwrap();
    }

    #[test]
    fn role_compile_rejects_newer_configuration_that_appears_after_owner_probe() {
        let workspace = temp_root("newer-owner-appears-after-probe");
        let source = temp_root("detached-newer-owner-appears-after-probe");
        fs::create_dir_all(&source).unwrap();
        let definition = write_definition(&workspace, &json!({ "name": "Reader" }));
        let newer = String::from_utf8(configuration_bytes())
            .unwrap()
            .replace(r#"version="2.20""#, r#"version="2.21""#)
            .into_bytes();
        let config_path = source.join("Configuration.xml");
        let config_for_hook = config_path.clone();
        let newer_for_hook = newer.clone();

        let outcome = with_role_compile_after_configuration_probe_hook(
            move |_| fs::write(&config_for_hook, &newer_for_hook).unwrap(),
            || compile_role(&compile_args(&definition, &source), &context(&workspace)),
        );

        assert!(!outcome.ok, "{outcome:?}");
        assert!(outcome.errors.join("\n").contains("2.21"), "{outcome:?}");
        assert_eq!(fs::read(&config_path).unwrap(), newer);
        assert!(!source.join("Roles/Reader.xml").exists());
        assert!(!source.join("Roles/Reader/Ext/Rights.xml").exists());
        fs::remove_dir_all(source).unwrap();
        fs::remove_dir_all(workspace).unwrap();
    }

    #[test]
    fn role_compile_rolls_back_if_supported_format_owner_appears_during_publication() {
        let workspace = temp_root("supported-owner-appears-during-publication");
        let source = temp_root("detached-supported-owner-appears-during-publication");
        fs::create_dir_all(&source).unwrap();
        let definition = write_definition(&workspace, &json!({ "name": "Reader" }));
        let config_path = source.join("Configuration.xml");
        let config_for_hook = config_path.clone();
        let supported = configuration_bytes();

        let outcome = with_before_commit_hook(
            move |_| fs::write(&config_for_hook, &supported).unwrap(),
            || compile_role(&compile_args(&definition, &source), &context(&workspace)),
        );

        assert!(!outcome.ok, "{outcome:?}");
        assert!(
            outcome.errors.join("\n").contains("absence guard"),
            "{outcome:?}"
        );
        assert!(config_path.is_file());
        assert!(!source.join("Roles/Reader.xml").exists());
        assert!(!source.join("Roles/Reader/Ext/Rights.xml").exists());
        fs::remove_dir_all(source).unwrap();
        fs::remove_dir_all(workspace).unwrap();
    }

    #[test]
    fn role_compile_validates_invalid_format_owner_that_appears_after_owner_probe() {
        let workspace = temp_root("invalid-owner-appears-after-probe");
        let source = temp_root("detached-invalid-owner-appears-after-probe");
        fs::create_dir_all(source.join("Languages")).unwrap();
        fs::write(source.join("Languages/English.xml"), b"language marker").unwrap();
        let definition = write_definition(&workspace, &json!({ "name": "Reader" }));
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

        let outcome = with_role_compile_after_configuration_probe_hook(
            move |_| fs::write(&config_for_hook, &invalid_for_hook).unwrap(),
            || compile_role(&compile_args(&definition, &source), &context(&workspace)),
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
        assert!(!source.join("Roles/Reader.xml").exists());
        assert!(!source.join("Roles/Reader/Ext/Rights.xml").exists());
        fs::remove_dir_all(source).unwrap();
        fs::remove_dir_all(workspace).unwrap();
    }

    #[test]
    fn role_compile_rejects_unsafe_name_before_planning_paths() {
        for (case, role_name) in [("traversal", "../Outside"), ("xml", "Bad&Name")] {
            let root = temp_root(case);
            fs::write(
                root.join("Configuration.xml"),
                concat!(
                    "<MetaDataObject xmlns=\"http://v8.1c.ru/8.3/MDClasses\" version=\"2.21\">",
                    "<Configuration><ChildObjects/></Configuration></MetaDataObject>"
                ),
            )
            .unwrap();
            let definition = write_definition(&root, &json!({ "name": role_name }));

            let outcome = compile_role(&compile_args(&definition, &root), &context(&root));

            assert!(!outcome.ok, "{role_name}: {outcome:?}");
            let error = outcome.errors.join("\n");
            assert!(error.contains("Unicode XML NCName"), "{error}");
            assert!(error.contains("single path component"), "{error}");
            assert!(!error.contains("Export format"), "{error}");
            assert!(!root.join("Outside.xml").exists());
            assert!(!root.join("Outside/Ext/Rights.xml").exists());
            assert!(!root.join("Roles").exists());
            fs::remove_dir_all(root).unwrap();
        }
    }

    #[test]
    fn role_compile_rejects_non_boolean_global_flags_before_planning() {
        let cases = [
            ("setForNewObjects", json!("banana")),
            ("setForAttributesByDefault", json!(1)),
            ("independentRightsOfChildObjects", Value::Null),
            ("setForNewObjects", json!([true])),
            ("setForAttributesByDefault", json!("true")),
        ];

        for (index, (field, invalid)) in cases.into_iter().enumerate() {
            let root = temp_root(&format!("invalid-bool-{index}"));
            let mut definition = json!({ "name": format!("Reader{index}") });
            definition
                .as_object_mut()
                .unwrap()
                .insert(field.to_string(), invalid);
            let definition_path = write_definition(&root, &definition);

            let outcome = compile_role(&compile_args(&definition_path, &root), &context(&root));

            assert!(!outcome.ok, "{field}: {outcome:?}");
            let error = outcome.errors.join("\n");
            assert!(error.contains(field), "{error}");
            assert!(error.contains("JSON boolean"), "{error}");
            assert!(!root.join("Roles").exists());
            fs::remove_dir_all(root).unwrap();
        }
    }

    #[test]
    fn role_compile_emits_all_global_flags_as_exact_xs_booleans() {
        let root = temp_root("valid-bools");
        let definition = write_definition(
            &root,
            &json!({
                "name": "Роль_Чтение",
                "setForNewObjects": true,
                "setForAttributesByDefault": false,
                "independentRightsOfChildObjects": true
            }),
        );

        let outcome = compile_role(&compile_args(&definition, &root), &context(&root));

        assert!(outcome.ok, "{outcome:?}");
        let text = fs::read_to_string(root.join("Roles/Роль_Чтение/Ext/Rights.xml")).unwrap();
        let doc = Document::parse(text.trim_start_matches('\u{feff}')).unwrap();
        let root_node = doc.root_element();
        for (field, expected) in [
            ("setForNewObjects", "true"),
            ("setForAttributesByDefault", "false"),
            ("independentRightsOfChildObjects", "true"),
        ] {
            let value = root_node
                .children()
                .find(|node| node.is_element() && node.tag_name().name() == field)
                .and_then(|node| node.text());
            assert_eq!(value, Some(expected), "{field}: {text}");
        }
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn role_compile_escapes_object_and_right_names_as_xs_strings() {
        let root = temp_root("escaped-rights-names");
        let object_name = "Catalog.Items<&\"'";
        let right_name = "View<&\"'";
        let definition = write_definition(
            &root,
            &json!({
                "name": "Reader",
                "objects": [{
                    "name": object_name,
                    "rights": { "View<&\"'": true }
                }]
            }),
        );

        let outcome = compile_role(&compile_args(&definition, &root), &context(&root));

        assert!(outcome.ok, "{outcome:?}");
        let text = fs::read_to_string(root.join("Roles/Reader/Ext/Rights.xml")).unwrap();
        let doc = Document::parse(text.trim_start_matches('\u{feff}')).unwrap();
        let names = doc
            .descendants()
            .filter(|node| node.is_element() && node.tag_name().name() == "name")
            .filter_map(|node| node.text())
            .collect::<Vec<_>>();
        assert_eq!(names, vec![object_name, right_name]);
        assert!(text.contains("&lt;&amp;&quot;'"), "{text}");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn role_metadata_emitter_defensively_escapes_name() {
        let xml = role_metadata_xml(
            "Bad<&\"'Name",
            "Synonym",
            "",
            "2.20",
            "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa",
        );

        let doc = Document::parse(&xml).unwrap();
        let name = doc
            .descendants()
            .find(|node| node.is_element() && node.tag_name().name() == "Name")
            .and_then(|node| node.text());
        assert_eq!(name, Some("Bad<&\"'Name"));
    }

    #[test]
    fn role_compile_post_validation_failure_rolls_back_exactly() {
        let root = temp_root("post-validation-rollback");
        let config = root.join("Configuration.xml");
        let original = write_configuration(&root);
        let definition = write_definition(&root, &json!({ "name": "Reader" }));

        let outcome = with_commit_failpoint(CommitFailpoint::PostWriteValidation, || {
            compile_role(&compile_args(&definition, &root), &context(&root))
        });

        assert!(!outcome.ok, "{outcome:?}");
        assert!(
            outcome.errors.join("\n").contains("post-write validation"),
            "{outcome:?}"
        );
        assert_eq!(fs::read(&config).unwrap(), original);
        assert!(!root.join("Roles/Reader.xml").exists());
        assert!(!root.join("Roles/Reader/Ext/Rights.xml").exists());
        assert!(!root.join("Roles").exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn role_compile_semantic_post_validation_failure_rolls_back_exactly() {
        let root = temp_root("semantic-post-validation-rollback");
        let config = root.join("Configuration.xml");
        let original = write_configuration(&root);
        let definition = write_definition(&root, &json!({ "name": "Reader" }));

        let outcome = with_role_compile_post_validation_failure(|| {
            compile_role(&compile_args(&definition, &root), &context(&root))
        });

        assert!(!outcome.ok, "{outcome:?}");
        assert!(
            outcome
                .errors
                .join("\n")
                .contains("role semantic post-validation failure"),
            "{outcome:?}"
        );
        assert_eq!(fs::read(&config).unwrap(), original);
        assert!(!root.join("Roles/Reader.xml").exists());
        assert!(!root.join("Roles/Reader/Ext/Rights.xml").exists());
        assert!(!root.join("Roles").exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn role_compile_descriptors_use_active_format() {
        let root = temp_root("format");
        let definition = root.join("role.json");
        fs::write(&definition, r#"{"name":"Reader"}"#).unwrap();

        let outcome = compile_role(&compile_args(&definition, &root), &context(&root));

        assert!(outcome.ok, "{outcome:?}");
        for path in [
            root.join("Roles/Reader.xml"),
            root.join("Roles/Reader/Ext/Rights.xml"),
        ] {
            let generated = fs::read_to_string(path).unwrap();
            assert!(generated.contains(r#"version="2.20""#), "{generated}");
            assert!(!generated.contains(r#"version="2.17""#), "{generated}");
        }
        let _ = fs::remove_dir_all(root);
    }

    fn validate_role_stdout(rights_xml: &str) -> String {
        let workspace = temp_root("role-validate-predefined-data");
        let ext_dir = workspace.join("Roles/PredefinedDataEditor/Ext");
        fs::create_dir_all(&ext_dir).unwrap();
        let rights_path = ext_dir.join("Rights.xml");
        fs::write(&rights_path, rights_xml).unwrap();

        let args = Map::from_iter([
            (
                "RightsPath".to_string(),
                Value::String(rights_path.display().to_string()),
            ),
            ("Detailed".to_string(), Value::Bool(true)),
        ]);
        let outcome = validate_role(&args, &context(&workspace));
        let stdout = outcome.stdout.clone().unwrap_or_default();
        let _ = fs::remove_dir_all(&workspace);
        assert!(outcome.ok, "{outcome:?}");
        stdout
    }

    const PREDEFINED_DATA_WARNING: &str =
        "grants interactive changes to predefined data (predefined data is part of the configuration and should not be available to end users)";

    #[test]
    fn validate_role_warns_on_interactive_predefined_data_right_set_true() {
        let rights = concat!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n",
            "<Rights xmlns=\"http://v8.1c.ru/8.2/roles\" xsi:type=\"Rights\" version=\"2.20\"\n",
            "        xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\">\n",
            "    <setForNewObjects>false</setForNewObjects>\n",
            "    <setForAttributesByDefault>false</setForAttributesByDefault>\n",
            "    <independentRightsOfChildObjects>false</independentRightsOfChildObjects>\n",
            "    <object>\n",
            "        <name>Catalog.Products</name>\n",
            "        <right><name>Read</name><value>true</value></right>\n",
            "        <right><name>InteractiveDeletePredefinedData</name><value>true</value></right>\n",
            "    </object>\n",
            "</Rights>\n",
        );
        let stdout = validate_role_stdout(rights);
        assert!(
            stdout.contains(&format!(
                "Catalog.Products: 'InteractiveDeletePredefinedData' = true {PREDEFINED_DATA_WARNING}"
            )),
            "{stdout}"
        );
    }

    #[test]
    fn validate_role_allows_interactive_predefined_data_right_set_false() {
        let rights = concat!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n",
            "<Rights xmlns=\"http://v8.1c.ru/8.2/roles\" xsi:type=\"Rights\" version=\"2.20\"\n",
            "        xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\">\n",
            "    <setForNewObjects>false</setForNewObjects>\n",
            "    <setForAttributesByDefault>false</setForAttributesByDefault>\n",
            "    <independentRightsOfChildObjects>false</independentRightsOfChildObjects>\n",
            "    <object>\n",
            "        <name>Catalog.Products</name>\n",
            "        <right><name>InteractiveClearDeletionMarkPredefinedData</name><value>false</value></right>\n",
            "    </object>\n",
            "</Rights>\n",
        );
        let stdout = validate_role_stdout(rights);
        assert!(!stdout.contains(PREDEFINED_DATA_WARNING), "{stdout}");
    }

    #[test]
    fn validate_role_allows_ordinary_right_without_predefined_data_warning() {
        let rights = concat!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n",
            "<Rights xmlns=\"http://v8.1c.ru/8.2/roles\" xsi:type=\"Rights\" version=\"2.20\"\n",
            "        xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\">\n",
            "    <setForNewObjects>false</setForNewObjects>\n",
            "    <setForAttributesByDefault>false</setForAttributesByDefault>\n",
            "    <independentRightsOfChildObjects>false</independentRightsOfChildObjects>\n",
            "    <object>\n",
            "        <name>Catalog.Products</name>\n",
            "        <right><name>InteractiveDelete</name><value>true</value></right>\n",
            "    </object>\n",
            "</Rights>\n",
        );
        let stdout = validate_role_stdout(rights);
        assert!(!stdout.contains(PREDEFINED_DATA_WARNING), "{stdout}");
    }
}
