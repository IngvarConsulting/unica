#![allow(dead_code, unused_imports)]

use crate::domain::workspace::WorkspaceContext;
use crate::infrastructure::metadata_kinds::{
    metadata_kind, metadata_kind_by_directory, metadata_kind_index, METADATA_KIND_TAGS,
};
use crate::infrastructure::AdapterOutcome;
use roxmltree::Document;
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use super::common::*;
use super::{
    cfe::*, form::*, interface::*, meta::*, mxl::*, role::*, skd::*, subsystem::*, template::*,
};

const CF_MD_NS: &str = "http://v8.1c.ru/8.3/MDClasses";
const CF_XR_NS: &str = "http://v8.1c.ru/8.3/xcf/readable";
const CF_V8_NS: &str = "http://v8.1c.ru/8.1/data/core";
const CF_CAI_NS: &str = "http://v8.1c.ru/8.2/managed-application/core";
const CF_HP_NS: &str = "http://v8.1c.ru/8.3/xcf/extrnprops";
pub(crate) struct CfValidationReporter {
    pub(crate) errors: usize,
    pub(crate) warnings: usize,
    pub(crate) ok_count: usize,
    pub(crate) stopped: bool,
    pub(crate) max_errors: usize,
    pub(crate) detailed: bool,
    pub(crate) lines: Vec<String>,
    pub(crate) obj_name: String,
}

pub(crate) struct CfValidationRun {
    pub(crate) ok: bool,
    pub(crate) stdout: String,
    pub(crate) out_file: Option<PathBuf>,
    pub(crate) artifact: PathBuf,
    pub(crate) errors: Vec<String>,
}

impl CfValidationReporter {
    pub(crate) fn new(max_errors: usize, detailed: bool) -> Self {
        Self {
            errors: 0,
            warnings: 0,
            ok_count: 0,
            stopped: false,
            max_errors,
            detailed,
            lines: vec![String::new()],
            obj_name: "(unknown)".to_string(),
        }
    }

    pub(crate) fn ok(&mut self, message: impl Into<String>) {
        self.ok_count += 1;
        if self.detailed {
            self.lines.push(format!("[OK]    {}", message.into()));
        }
    }

    pub(crate) fn error(&mut self, message: impl Into<String>) {
        self.errors += 1;
        self.lines.push(format!("[ERROR] {}", message.into()));
        if self.errors >= self.max_errors {
            self.stopped = true;
        }
    }

    pub(crate) fn warn(&mut self, message: impl Into<String>) {
        self.warnings += 1;
        self.lines.push(format!("[WARN]  {}", message.into()));
    }

    pub(crate) fn finalize(mut self) -> (bool, String, Vec<String>) {
        let checks = self.ok_count + self.errors + self.warnings;
        let ok = self.errors == 0;
        if ok && self.warnings == 0 && !self.detailed {
            return (
                true,
                format!(
                    "=== Validation OK: Configuration.{} ({checks} checks) ===",
                    self.obj_name
                ),
                Vec::new(),
            );
        }
        self.lines.insert(
            0,
            format!("=== Validation: Configuration.{} ===", self.obj_name),
        );
        self.lines.push(String::new());
        self.lines.push(format!(
            "=== Result: {} errors, {} warnings ({checks} checks) ===",
            self.errors, self.warnings
        ));
        let errors = self
            .lines
            .iter()
            .filter(|line| line.starts_with("[ERROR]"))
            .cloned()
            .collect::<Vec<_>>();
        (ok, self.lines.join("\r\n") + "\r\n", errors)
    }
}

pub(crate) fn validate_cf(args: &Map<String, Value>, context: &WorkspaceContext) -> AdapterOutcome {
    const MD_NS: &str = "http://v8.1c.ru/8.3/MDClasses";
    const XR_NS: &str = "http://v8.1c.ru/8.3/xcf/readable";
    const HP_NS: &str = "http://v8.1c.ru/8.3/xcf/extrnprops";

    let result = (|| -> Result<CfValidationRun, String> {
        let raw_path = required_path(
            args,
            &["configPath", "ConfigPath", "path", "Path"],
            "ConfigPath",
        )?;
        let mut config_path = absolutize(raw_path, &context.cwd);
        if config_path.is_dir() {
            let candidate = config_path.join("Configuration.xml");
            if candidate.exists() {
                config_path = candidate;
            } else {
                return Err(format!(
                    "[ERROR] No Configuration.xml found in directory: {}",
                    config_path.display()
                ));
            }
        }
        if !config_path.exists() {
            return Err(format!("[ERROR] File not found: {}", config_path.display()));
        }
        let resolved_path = config_path
            .canonicalize()
            .unwrap_or_else(|_| config_path.clone());
        let config_dir = resolved_path.parent().unwrap_or(context.cwd.as_path());
        let out_file =
            path_arg(args, &["outFile", "OutFile"]).map(|path| absolutize(path, &context.cwd));
        let detailed = bool_arg(args, &["detailed", "Detailed"]);
        let max_errors = int_arg(args, &["maxErrors", "MaxErrors"])
            .and_then(|value| usize::try_from(value).ok())
            .filter(|value| *value > 0)
            .unwrap_or(30);

        let text = read_utf8_sig(&resolved_path)?;
        let doc = match Document::parse(text.trim_start_matches('\u{feff}')) {
            Ok(doc) => doc,
            Err(err) => {
                let mut report = CfValidationReporter::new(max_errors, detailed);
                report.obj_name = "(parse failed)".to_string();
                report.error(format!("1. XML parse failed: {err}"));
                let (ok, stdout, errors) = report.finalize();
                return Ok(CfValidationRun {
                    ok,
                    stdout,
                    out_file,
                    artifact: resolved_path,
                    errors,
                });
            }
        };

        let mut report = CfValidationReporter::new(max_errors, detailed);
        let root = doc.root_element();
        let mut check1_ok = true;
        let root_local = root.tag_name().name();
        let root_ns = root.tag_name().namespace().unwrap_or("");
        if root_local != "MetaDataObject" {
            report.error(format!(
                "1. Root element is '{root_local}', expected 'MetaDataObject'"
            ));
            let (ok, stdout, errors) = report.finalize();
            return Ok(CfValidationRun {
                ok,
                stdout,
                out_file,
                artifact: resolved_path,
                errors,
            });
        }
        if root_ns != MD_NS {
            report.error(format!(
                "1. Root namespace is '{root_ns}', expected '{MD_NS}'"
            ));
            check1_ok = false;
        }
        let version = root.attribute("version").unwrap_or("");
        if version.is_empty() {
            report.warn("1. Missing version attribute on MetaDataObject");
        } else if !matches!(version, "2.17" | "2.20" | "2.21") {
            report.warn(format!(
                "1. Unusual version '{version}' (expected 2.17, 2.20 or 2.21)"
            ));
        }

        let Some(cfg_node) = root
            .children()
            .find(|node| role_info_element(*node, "Configuration", Some(MD_NS)))
        else {
            report.error("1. No <Configuration> element found inside MetaDataObject");
            let (ok, stdout, errors) = report.finalize();
            return Ok(CfValidationRun {
                ok,
                stdout,
                out_file,
                artifact: resolved_path,
                errors,
            });
        };

        let cfg_uuid = cfg_node.attribute("uuid").unwrap_or("");
        if cfg_uuid.is_empty() {
            report.error("1. Missing uuid on <Configuration>");
            check1_ok = false;
        } else if !cf_validate_guid(cfg_uuid) {
            report.error(format!("1. Invalid uuid '{cfg_uuid}' on <Configuration>"));
            check1_ok = false;
        }

        let props_node = cfg_node
            .children()
            .find(|node| role_info_element(*node, "Properties", Some(MD_NS)));
        let name_node = props_node.and_then(|props| {
            props
                .children()
                .find(|node| role_info_element(*node, "Name", Some(MD_NS)))
        });
        let obj_name = name_node
            .and_then(|node| node.text())
            .filter(|value| !value.is_empty())
            .unwrap_or("(unknown)")
            .to_string();
        report.obj_name = obj_name.clone();

        if check1_ok {
            report.ok(format!(
                "1. Root structure: MetaDataObject/Configuration, version {version}"
            ));
        }
        if report.stopped {
            let (ok, stdout, errors) = report.finalize();
            return Ok(CfValidationRun {
                ok,
                stdout,
                out_file,
                artifact: resolved_path,
                errors,
            });
        }

        let internal_info = cfg_node
            .children()
            .find(|node| role_info_element(*node, "InternalInfo", Some(MD_NS)));
        if let Some(internal_info) = internal_info {
            let contained = internal_info
                .children()
                .filter(|node| role_info_element(*node, "ContainedObject", Some(XR_NS)))
                .collect::<Vec<_>>();
            let mut check2_ok = true;
            if contained.len() != 7 {
                report.warn(format!(
                    "2. InternalInfo: expected 7 ContainedObject, found {}",
                    contained.len()
                ));
            }
            let mut found_class_ids = HashSet::new();
            for co in &contained {
                let class_id = cf_validate_child_text(*co, "ClassId", Some(XR_NS));
                let object_id = cf_validate_child_text(*co, "ObjectId", Some(XR_NS));
                if class_id.is_empty() {
                    report.error("2. ContainedObject missing ClassId");
                    check2_ok = false;
                    continue;
                }
                if !cf_validate_class_ids().contains(&class_id.as_str()) {
                    report.error(format!("2. Unknown ClassId: {class_id}"));
                    check2_ok = false;
                }
                if !found_class_ids.insert(class_id.clone()) {
                    report.error(format!("2. Duplicate ClassId: {class_id}"));
                    check2_ok = false;
                }
                if object_id.is_empty() {
                    report.error(format!(
                        "2. ContainedObject missing ObjectId for ClassId {class_id}"
                    ));
                    check2_ok = false;
                } else if !cf_validate_guid(&object_id) {
                    report.error(format!(
                        "2. Invalid ObjectId '{object_id}' for ClassId {class_id}"
                    ));
                    check2_ok = false;
                }
            }
            let missing_ids = cf_validate_class_ids()
                .iter()
                .filter(|class_id| !found_class_ids.contains(**class_id))
                .count();
            if missing_ids > 0 {
                report.warn(format!("2. Missing ClassIds: {missing_ids} of 7"));
            }
            if check2_ok {
                report.ok(format!(
                    "2. InternalInfo: {} ContainedObject, all ClassIds valid",
                    contained.len()
                ));
            }
        } else {
            report.error("2. InternalInfo: missing");
        }
        if report.stopped {
            let (ok, stdout, errors) = report.finalize();
            return Ok(CfValidationRun {
                ok,
                stdout,
                out_file,
                artifact: resolved_path,
                errors,
            });
        }

        let mut def_lang = String::new();
        if let Some(props_node) = props_node {
            let mut check3_ok = true;
            if obj_name == "(unknown)" {
                report.error("3. Properties: Name is missing or empty");
                check3_ok = false;
            } else if !cf_validate_identifier(&obj_name) {
                report.error(format!(
                    "3. Properties: Name '{obj_name}' is not a valid 1C identifier"
                ));
                check3_ok = false;
            }

            let syn_present = props_node
                .children()
                .find(|node| role_info_element(*node, "Synonym", Some(MD_NS)))
                .map(|syn_node| !multilang_text(syn_node).is_empty())
                .unwrap_or(false);
            def_lang = cf_validate_child_text(props_node, "DefaultLanguage", Some(MD_NS));
            if def_lang.is_empty() {
                report.error("3. Properties: DefaultLanguage is missing or empty");
                check3_ok = false;
            }
            let default_run = cf_validate_child_text(props_node, "DefaultRunMode", Some(MD_NS));
            if default_run.is_empty() {
                report.warn("3. Properties: DefaultRunMode is missing or empty");
            }
            if check3_ok {
                let syn_info = if syn_present {
                    "Synonym present"
                } else {
                    "no Synonym"
                };
                report.ok(format!(
                    "3. Properties: Name=\"{obj_name}\", {syn_info}, DefaultLanguage={def_lang}"
                ));
            }

            let mut enum_checked = 0usize;
            let mut check4_ok = true;
            for property in cf_validate_enum_properties() {
                let value = cf_validate_child_text(props_node, property, Some(MD_NS));
                if !value.is_empty() {
                    let allowed = cf_validate_enum_allowed(property);
                    if !allowed.contains(&value.as_str()) {
                        report.error(format!(
                            "4. Property '{property}' has invalid value '{value}'"
                        ));
                        check4_ok = false;
                    }
                    enum_checked += 1;
                }
            }
            if check4_ok {
                report.ok(format!(
                    "4. Property values: {enum_checked} enum properties checked"
                ));
            }
        } else {
            report.error("3. Properties block missing");
            report.warn("4. No Properties block to check");
        }
        if report.stopped {
            let (ok, stdout, errors) = report.finalize();
            return Ok(CfValidationRun {
                ok,
                stdout,
                out_file,
                artifact: resolved_path,
                errors,
            });
        }

        let child_obj_node = cfg_node
            .children()
            .find(|node| role_info_element(*node, "ChildObjects", Some(MD_NS)));
        if let Some(child_obj_node) = child_obj_node {
            let mut check5_ok = true;
            let mut total_count = 0usize;
            let mut type_counts: Vec<(String, HashSet<String>)> = Vec::new();
            let mut type_first_index = HashSet::new();
            let mut last_type_order = -1isize;
            let mut order_ok = true;
            for child in child_obj_node.children().filter(|node| node.is_element()) {
                let type_name = child.tag_name().name().to_string();
                let object_name = child.text().unwrap_or("").to_string();
                if let Some(type_index) = cf_validate_child_object_type_index(&type_name) {
                    if type_first_index.insert(type_name.clone()) {
                        if (type_index as isize) < last_type_order {
                            report.warn(format!(
                                "5. Type '{type_name}' is out of canonical order (after type at position {last_type_order})"
                            ));
                            order_ok = false;
                        }
                        last_type_order = type_index as isize;
                    }
                } else {
                    report.error(format!("5. Unknown type '{type_name}' in ChildObjects"));
                    check5_ok = false;
                }

                let existing = type_counts
                    .iter_mut()
                    .find(|(name, _)| name == &type_name)
                    .map(|(_, names)| names);
                if let Some(names) = existing {
                    if !names.insert(object_name.clone()) {
                        report.error(format!("5. Duplicate: {type_name}.{object_name}"));
                        check5_ok = false;
                    }
                } else {
                    let mut names = HashSet::new();
                    names.insert(object_name);
                    type_counts.push((type_name, names));
                }
                total_count += 1;
            }
            if check5_ok {
                let order_info = if order_ok { ", order correct" } else { "" };
                report.ok(format!(
                    "5. ChildObjects: {} types, {total_count} objects{order_info}",
                    type_counts.len()
                ));
            }

            if !def_lang.is_empty() {
                let lang_name = def_lang.strip_prefix("Language.").unwrap_or(&def_lang);
                let found = child_obj_node.children().any(|child| {
                    role_info_element(child, "Language", Some(MD_NS))
                        && child.text().unwrap_or("") == lang_name
                });
                if found {
                    report.ok(format!(
                        "6. DefaultLanguage \"{def_lang}\" found in ChildObjects"
                    ));
                } else {
                    report.error(format!(
                        "6. DefaultLanguage \"{def_lang}\" not found in ChildObjects"
                    ));
                }
            } else {
                report.warn("6. Cannot check DefaultLanguage (empty)");
            }

            let lang_names = child_obj_node
                .children()
                .filter(|child| role_info_element(*child, "Language", Some(MD_NS)))
                .map(|child| child.text().unwrap_or("").to_string())
                .collect::<Vec<_>>();
            if lang_names.is_empty() {
                report.warn("7. No Language entries in ChildObjects");
            } else {
                let mut exist_count = 0usize;
                for lang_name in &lang_names {
                    let lang_file = config_dir
                        .join("Languages")
                        .join(format!("{lang_name}.xml"));
                    if lang_file.exists() {
                        exist_count += 1;
                    } else {
                        report.warn(format!(
                            "7. Language file missing: Languages/{lang_name}.xml"
                        ));
                    }
                }
                if exist_count == lang_names.len() {
                    report.ok(format!(
                        "7. Language files: {exist_count}/{} exist",
                        lang_names.len()
                    ));
                }
            }

            let mut dirs_to_check = Vec::<(String, usize)>::new();
            for child in child_obj_node.children().filter(|node| node.is_element()) {
                let type_name = child.tag_name().name();
                if type_name == "Language" {
                    continue;
                }
                if let Some(dir_name) = cf_validate_child_type_dir(type_name) {
                    if let Some((_, count)) = dirs_to_check
                        .iter_mut()
                        .find(|(existing, _)| existing == dir_name)
                    {
                        *count += 1;
                    } else {
                        dirs_to_check.push((dir_name.to_string(), 1));
                    }
                }
            }
            let missing_dirs = dirs_to_check
                .iter()
                .filter(|(dir_name, _)| !config_dir.join(dir_name).is_dir())
                .map(|(dir_name, count)| format!("{dir_name} ({count} objects)"))
                .collect::<Vec<_>>();
            if missing_dirs.is_empty() {
                report.ok(format!(
                    "8. Object directories: {} directories, all exist",
                    dirs_to_check.len()
                ));
            } else {
                for missing in missing_dirs {
                    report.warn(format!("8. Missing directory: {missing}"));
                }
            }
        } else {
            report.error("5. ChildObjects block missing");
            if def_lang.is_empty() {
                report.warn("6. Cannot check DefaultLanguage (empty)");
            } else {
                report.warn("6. Cannot check DefaultLanguage (no ChildObjects)");
            }
            report.warn("7. Cannot check language files (no ChildObjects)");
        }
        if report.stopped {
            let (ok, stdout, errors) = report.finalize();
            return Ok(CfValidationRun {
                ok,
                stdout,
                out_file,
                artifact: resolved_path,
                errors,
            });
        }

        let mut form_refs_checked = 0usize;
        let mut form_ref_errors = Vec::new();
        let home_page = config_dir.join("Ext").join("HomePageWorkArea.xml");
        if home_page.is_file() {
            match read_utf8_sig(&home_page) {
                Ok(home_page_text) => {
                    match Document::parse(home_page_text.trim_start_matches('\u{feff}')) {
                        Ok(hp_doc) => {
                            for form_node in hp_doc
                                .descendants()
                                .filter(|node| role_info_element(*node, "Form", Some(HP_NS)))
                            {
                                let form_ref = form_node.text().unwrap_or("").trim();
                                if form_ref.is_empty() {
                                    continue;
                                }
                                form_refs_checked += 1;
                                if !cf_validate_form_ref(config_dir, form_ref) {
                                    form_ref_errors.push(format!(
                                        "HomePageWorkArea.Form '{form_ref}' — file not found"
                                    ));
                                }
                            }
                        }
                        Err(err) => form_ref_errors
                            .push(format!("HomePageWorkArea.xml: parse error — {err}")),
                    }
                }
                Err(err) => {
                    form_ref_errors.push(format!("HomePageWorkArea.xml: parse error — {err}"));
                }
            }
        }
        if let Some(props_node) = props_node {
            for property in cf_validate_form_properties() {
                let form_ref = cf_validate_child_text(props_node, property, Some(MD_NS));
                if form_ref.trim().is_empty() {
                    continue;
                }
                form_refs_checked += 1;
                if !cf_validate_form_ref(config_dir, form_ref.trim()) {
                    form_ref_errors.push(format!(
                        "Properties.{property} '{}' — form not found",
                        form_ref.trim()
                    ));
                }
            }
        }
        if form_refs_checked == 0 {
            report.ok("9. Form references: none to check");
        } else if form_ref_errors.is_empty() {
            report.ok(format!("9. Form references: {form_refs_checked} verified"));
        } else {
            for error in form_ref_errors {
                report.error(format!("9. {error}"));
            }
        }

        let (ok, stdout, errors) = report.finalize();
        Ok(CfValidationRun {
            ok,
            stdout,
            out_file,
            artifact: resolved_path,
            errors,
        })
    })();

    match result {
        Ok(run) => {
            let mut stdout = run.stdout.clone();
            let mut artifacts = vec![run.artifact.display().to_string()];
            if let Some(out_file) = &run.out_file {
                match write_utf8_bom(out_file, &run.stdout) {
                    Ok(()) => {
                        stdout.push_str(&format!("Written to: {}\n", out_file.display()));
                        artifacts.push(out_file.display().to_string());
                    }
                    Err(error) => {
                        return AdapterOutcome {
                            ok: false,
                            summary: "unica.cf.validate failed in native configuration validator"
                                .to_string(),
                            changes: Vec::new(),
                            warnings: Vec::new(),
                            errors: vec![error.clone()],
                            artifacts,
                            stdout: None,
                            stderr: Some(format!("{error}\n")),
                            command: None,
                        };
                    }
                }
            }
            AdapterOutcome {
                ok: run.ok,
                summary: if run.ok {
                    "unica.cf.validate completed with native configuration validator".to_string()
                } else {
                    "unica.cf.validate failed in native configuration validator".to_string()
                },
                changes: Vec::new(),
                warnings: Vec::new(),
                errors: run.errors,
                artifacts,
                stdout: Some(stdout),
                stderr: Some(String::new()),
                command: None,
            }
        }
        Err(error) => AdapterOutcome {
            ok: false,
            summary: "unica.cf.validate failed in native configuration validator".to_string(),
            changes: Vec::new(),
            warnings: Vec::new(),
            errors: vec![error.clone()],
            artifacts: Vec::new(),
            stdout: Some(format!("{error}\n")),
            stderr: Some(String::new()),
            command: None,
        },
    }
}

pub(crate) fn cf_validate_child_text(
    node: roxmltree::Node<'_, '_>,
    local_name: &str,
    namespace: Option<&str>,
) -> String {
    node.children()
        .find(|child| role_info_element(*child, local_name, namespace))
        .and_then(|child| child.text())
        .unwrap_or("")
        .to_string()
}

pub(crate) fn cf_validate_guid(value: &str) -> bool {
    let parts = value.split('-').collect::<Vec<_>>();
    let lengths = [8, 4, 4, 4, 12];
    parts.len() == lengths.len()
        && parts
            .iter()
            .zip(lengths)
            .all(|(part, len)| part.len() == len && part.chars().all(|ch| ch.is_ascii_hexdigit()))
}

pub(crate) fn cf_validate_identifier(value: &str) -> bool {
    let mut chars = value.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !cf_validate_identifier_start(first) {
        return false;
    }
    chars.all(cf_validate_identifier_continue)
}

pub(crate) fn cf_validate_identifier_start(ch: char) -> bool {
    ch == '_' || ch.is_ascii_alphabetic() || ('А'..='я').contains(&ch) || matches!(ch, 'Ё' | 'ё')
}

pub(crate) fn cf_validate_identifier_continue(ch: char) -> bool {
    cf_validate_identifier_start(ch) || ch.is_ascii_digit()
}

pub(crate) fn cf_validate_class_ids() -> &'static [&'static str] {
    &[
        "9cd510cd-abfc-11d4-9434-004095e12fc7",
        "9fcd25a0-4822-11d4-9414-008048da11f9",
        "e3687481-0a87-462c-a166-9f34594f9bba",
        "9de14907-ec23-4a07-96f0-85521cb6b53b",
        "51f2d5d8-ea4d-4064-8892-82951750031e",
        "e68182ea-4237-4383-967f-90c1e3370bc7",
        "fb282519-d103-4dd3-bc12-cb271d631dfc",
    ]
}

pub(crate) fn cf_validate_enum_properties() -> &'static [&'static str] {
    &[
        "ConfigurationExtensionCompatibilityMode",
        "DefaultRunMode",
        "ScriptVariant",
        "DataLockControlMode",
        "ObjectAutonumerationMode",
        "ModalityUseMode",
        "SynchronousPlatformExtensionAndAddInCallUseMode",
        "InterfaceCompatibilityMode",
        "DatabaseTablespacesUseMode",
        "MainClientApplicationWindowMode",
        "CompatibilityMode",
    ]
}

pub(crate) fn cf_validate_enum_allowed(property: &str) -> &'static [&'static str] {
    const COMPAT: &[&str] = &[
        "DontUse",
        "Version8_1",
        "Version8_2_13",
        "Version8_2_16",
        "Version8_3_1",
        "Version8_3_2",
        "Version8_3_3",
        "Version8_3_4",
        "Version8_3_5",
        "Version8_3_6",
        "Version8_3_7",
        "Version8_3_8",
        "Version8_3_9",
        "Version8_3_10",
        "Version8_3_11",
        "Version8_3_12",
        "Version8_3_13",
        "Version8_3_14",
        "Version8_3_15",
        "Version8_3_16",
        "Version8_3_17",
        "Version8_3_18",
        "Version8_3_19",
        "Version8_3_20",
        "Version8_3_21",
        "Version8_3_22",
        "Version8_3_23",
        "Version8_3_24",
        "Version8_3_25",
        "Version8_3_26",
        "Version8_3_27",
        "Version8_3_28",
        "Version8_5_1",
    ];
    match property {
        "ConfigurationExtensionCompatibilityMode" | "CompatibilityMode" => COMPAT,
        "DefaultRunMode" => &["ManagedApplication", "OrdinaryApplication", "Auto"],
        "ScriptVariant" => &["Russian", "English"],
        "DataLockControlMode" => &["Automatic", "Managed", "AutomaticAndManaged"],
        "ObjectAutonumerationMode" => &["NotAutoFree", "AutoFree"],
        "ModalityUseMode" | "SynchronousPlatformExtensionAndAddInCallUseMode" => {
            &["DontUse", "Use", "UseWithWarnings"]
        }
        "InterfaceCompatibilityMode" => &[
            "Version8_2",
            "Version8_2EnableTaxi",
            "Taxi",
            "TaxiEnableVersion8_2",
            "TaxiEnableVersion8_5",
            "Version8_5EnableTaxi",
            "Version8_5",
            "Version8_3_24",
        ],
        "DatabaseTablespacesUseMode" => &["DontUse", "Use"],
        "MainClientApplicationWindowMode" => &["Normal", "Fullscreen", "Kiosk"],
        _ => &[],
    }
}

pub(crate) fn cf_validate_child_object_type_index(type_name: &str) -> Option<usize> {
    metadata_kind_index(type_name)
}

pub(crate) fn cf_validate_child_object_types() -> &'static [&'static str] {
    METADATA_KIND_TAGS
}

pub(crate) fn cf_validate_child_type_dir(type_name: &str) -> Option<&'static str> {
    metadata_kind(type_name).map(|kind| kind.directory)
}

pub(crate) fn cf_validate_form_properties() -> &'static [&'static str] {
    &[
        "DefaultReportForm",
        "DefaultReportVariantForm",
        "DefaultReportSettingsForm",
        "DefaultDynamicListSettingsForm",
        "DefaultSearchForm",
        "DefaultDataHistoryChangeHistoryForm",
        "DefaultDataHistoryVersionDataForm",
        "DefaultDataHistoryVersionDifferencesForm",
        "DefaultCollaborationSystemUsersChoiceForm",
        "DefaultConstantsForm",
    ]
}

pub(crate) fn cf_validate_form_ref(config_dir: &Path, form_ref: &str) -> bool {
    if form_ref.is_empty() || cf_validate_guid(form_ref) {
        return true;
    }
    let parts = form_ref.split('.').collect::<Vec<_>>();
    if parts.len() == 2 && parts[0] == "CommonForm" {
        let direct = config_dir
            .join("CommonForms")
            .join(parts[1])
            .join("Form.xml");
        let ext = config_dir
            .join("CommonForms")
            .join(parts[1])
            .join("Ext")
            .join("Form.xml");
        return direct.is_file() || ext.is_file();
    }
    if parts.len() == 4 && parts[2] == "Form" {
        if let Some(dir_name) = cf_validate_child_type_dir(parts[0]) {
            let direct = config_dir
                .join(dir_name)
                .join(parts[1])
                .join("Forms")
                .join(parts[3])
                .join("Form.xml");
            let ext = config_dir
                .join(dir_name)
                .join(parts[1])
                .join("Forms")
                .join(parts[3])
                .join("Ext")
                .join("Form.xml");
            return direct.is_file() || ext.is_file();
        }
    }
    false
}

pub(crate) fn analyze_cf_info(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> AdapterOutcome {
    const MD_NS: &str = "http://v8.1c.ru/8.3/MDClasses";

    let result = (|| -> Result<(String, Option<PathBuf>, PathBuf), String> {
        let raw_path = required_path(
            args,
            &["configPath", "ConfigPath", "path", "Path"],
            "ConfigPath",
        )?;
        let mut config_path = absolutize(raw_path, &context.cwd);
        if config_path.is_dir() {
            let candidate = config_path.join("Configuration.xml");
            if candidate.is_file() {
                config_path = candidate;
            } else {
                return Err(format!(
                    "[ERROR] No Configuration.xml found in directory: {}",
                    config_path.display()
                ));
            }
        }
        if !config_path.is_file() {
            return Err(format!("[ERROR] File not found: {}", config_path.display()));
        }

        let text = fs::read_to_string(&config_path)
            .map_err(|err| format!("failed to read {}: {err}", config_path.display()))?;
        let doc = Document::parse(text.trim_start_matches('\u{feff}'))
            .map_err(|err| format!("XML parse error in {}: {err}", config_path.display()))?;
        let root = doc.root_element();
        if root.tag_name().name() != "MetaDataObject" {
            return Err(
                "[ERROR] Not a valid 1C metadata XML file (no MetaDataObject root)".to_string(),
            );
        }
        let Some(cfg) = root
            .children()
            .find(|node| role_info_element(*node, "Configuration", Some(MD_NS)))
        else {
            return Err("[ERROR] No <Configuration> element found".to_string());
        };
        let Some(props) = cfg
            .children()
            .find(|node| role_info_element(*node, "Properties", Some(MD_NS)))
        else {
            return Err("[ERROR] No <Configuration>/<Properties> element found".to_string());
        };

        let mode = string_arg(args, &["mode", "Mode"]).unwrap_or("overview");
        let section = string_arg(args, &["section", "Section", "name", "Name"]).unwrap_or("");
        let out_file =
            path_arg(args, &["outFile", "OutFile"]).map(|path| absolutize(path, &context.cwd));

        let version = root.attribute("version").unwrap_or("");
        let cfg_name = cf_prop_text(props, "Name");
        let cfg_synonym = cf_prop_ml(props, "Synonym");
        let cfg_version = cf_prop_text(props, "Version");
        let cfg_vendor = cf_prop_text(props, "Vendor");
        let cfg_compat = cf_prop_text(props, "CompatibilityMode");
        let cfg_default_run = cf_prop_text(props, "DefaultRunMode");
        let cfg_script = cf_prop_text(props, "ScriptVariant");
        let cfg_default_lang = cf_prop_text(props, "DefaultLanguage");
        let cfg_data_lock = cf_prop_text(props, "DataLockControlMode");
        let cfg_modality = cf_prop_text(props, "ModalityUseMode");
        let cfg_intf_compat = cf_prop_text(props, "InterfaceCompatibilityMode");
        let cfg_ext_purpose = cf_prop_text(props, "ConfigurationExtensionPurpose");
        let support_lines =
            support_state_lines_for_configuration(&config_path, !cfg_ext_purpose.is_empty());

        let counts = cf_child_object_counts(cfg);
        let total_objects = counts.iter().map(|(_, count)| *count).sum::<usize>();
        let mut lines = Vec::<String>::new();

        let config_dir = config_path.parent().unwrap_or(context.cwd.as_path());

        if section == "home-page" {
            cf_append_home_page_section(&mut lines, config_dir, &cfg_name);
        } else if mode == "brief" {
            let syn_part = if cfg_synonym.is_empty() {
                String::new()
            } else {
                format!(" — \"{cfg_synonym}\"")
            };
            let ver_part = if cfg_version.is_empty() {
                String::new()
            } else {
                format!(" v{cfg_version}")
            };
            let compat_part = if cfg_compat.is_empty() {
                String::new()
            } else {
                format!(" | {cfg_compat}")
            };
            lines.push(format!(
                "Конфигурация: {cfg_name}{syn_part}{ver_part} | {total_objects} объектов{compat_part}"
            ));
        } else if mode == "overview" {
            let syn_part = if cfg_synonym.is_empty() {
                String::new()
            } else {
                format!(" — \"{cfg_synonym}\"")
            };
            let ver_part = if cfg_version.is_empty() {
                String::new()
            } else {
                format!(" v{cfg_version}")
            };
            lines.push(format!(
                "=== Конфигурация: {cfg_name}{syn_part}{ver_part} ==="
            ));
            lines.push(String::new());
            lines.push(format!("Формат:         {version}"));
            if !cfg_vendor.is_empty() {
                lines.push(format!("Поставщик:      {cfg_vendor}"));
            }
            if !cfg_version.is_empty() {
                lines.push(format!("Версия:         {cfg_version}"));
            }
            lines.extend(support_lines.clone());
            lines.push(format!("Совместимость:  {cfg_compat}"));
            lines.push(format!("Режим запуска:  {cfg_default_run}"));
            lines.push(format!("Язык скриптов:  {cfg_script}"));
            lines.push(format!("Язык:           {cfg_default_lang}"));
            lines.push(format!("Блокировки:     {cfg_data_lock}"));
            lines.push(format!("Модальность:    {cfg_modality}"));
            lines.push(format!("Интерфейс:      {cfg_intf_compat}"));
            lines.push(String::new());
            cf_append_counts(&mut lines, &counts, total_objects);
        } else if mode == "full" {
            let cfg_ext_compat = cf_prop_text(props, "ConfigurationExtensionCompatibilityMode");
            let cfg_auto_num = cf_prop_text(props, "ObjectAutonumerationMode");
            let cfg_sync_calls =
                cf_prop_text(props, "SynchronousPlatformExtensionAndAddInCallUseMode");
            let cfg_db_spaces = cf_prop_text(props, "DatabaseTablespacesUseMode");
            let cfg_window_mode = cf_prop_text(props, "MainClientApplicationWindowMode");
            let cfg_comment = cf_prop_text(props, "Comment");
            let cfg_prefix = cf_prop_text(props, "NamePrefix");
            let cfg_update_addr = cf_prop_text(props, "UpdateCatalogAddress");
            cf_append_full_info(
                &mut lines,
                cfg,
                props,
                version,
                &cfg_name,
                &cfg_synonym,
                &cfg_version,
                &cfg_vendor,
                &cfg_compat,
                &cfg_ext_compat,
                &cfg_default_run,
                &cfg_script,
                &cfg_default_lang,
                &cfg_data_lock,
                &cfg_modality,
                &cfg_intf_compat,
                &cfg_auto_num,
                &cfg_sync_calls,
                &cfg_db_spaces,
                &cfg_window_mode,
                &cfg_comment,
                &cfg_prefix,
                &cfg_update_addr,
                config_dir,
                &support_lines,
                &counts,
                total_objects,
            );
        } else {
            return Err(format!(
                "argument -Mode: invalid choice: '{mode}' (choose from 'overview', 'brief', 'full')"
            ));
        }

        let result_text = cf_paginate(lines, args);
        let mut stdout = format!("{result_text}\n");
        if let Some(out_file) = &out_file {
            write_utf8_bom(out_file, &result_text)?;
            stdout.push_str(&format!("\nWritten to: {}\n", out_file.display()));
        }

        Ok((stdout, out_file, config_path))
    })();

    match result {
        Ok((stdout, out_file, artifact)) => {
            let mut artifacts = vec![artifact.display().to_string()];
            if let Some(out_file) = out_file {
                artifacts.push(out_file.display().to_string());
            }
            AdapterOutcome {
                ok: true,
                summary: "unica.cf.info completed with native configuration analyzer".to_string(),
                changes: Vec::new(),
                warnings: Vec::new(),
                errors: Vec::new(),
                artifacts,
                stdout: Some(stdout),
                stderr: Some(String::new()),
                command: None,
            }
        }
        Err(error) => AdapterOutcome {
            ok: false,
            summary: "unica.cf.info failed in native configuration analyzer".to_string(),
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

pub(crate) fn cf_prop_text(props: roxmltree::Node<'_, '_>, local_name: &str) -> String {
    child_text(props, local_name, Some("http://v8.1c.ru/8.3/MDClasses"))
}

pub(crate) fn cf_prop_ml(props: roxmltree::Node<'_, '_>, local_name: &str) -> String {
    props
        .children()
        .find(|node| role_info_element(*node, local_name, Some("http://v8.1c.ru/8.3/MDClasses")))
        .map(multilang_text)
        .unwrap_or_default()
}

pub(crate) fn cf_child_object_counts(cfg: roxmltree::Node<'_, '_>) -> Vec<(String, usize)> {
    let mut counts = Vec::<(String, usize)>::new();
    if let Some(child_objects) = cfg.children().find(|node| {
        role_info_element(*node, "ChildObjects", Some("http://v8.1c.ru/8.3/MDClasses"))
    }) {
        for child in child_objects.children().filter(|node| node.is_element()) {
            let type_name = child.tag_name().name().to_string();
            if let Some((_, count)) = counts.iter_mut().find(|(name, _)| name == &type_name) {
                *count += 1;
            } else {
                counts.push((type_name, 1));
            }
        }
    }
    counts
}

pub(crate) fn cf_append_counts(
    lines: &mut Vec<String>,
    counts: &[(String, usize)],
    total_objects: usize,
) {
    lines.push(format!("--- Состав ({total_objects} объектов) ---"));
    lines.push(String::new());
    let max_type_len = cf_type_order()
        .iter()
        .filter(|type_name| counts.iter().any(|(name, _)| name == *type_name))
        .map(|type_name| cf_type_ru_name(type_name).chars().count())
        .max()
        .unwrap_or(0)
        .max(10);
    for type_name in cf_type_order() {
        if let Some((_, count)) = counts.iter().find(|(name, _)| name == type_name) {
            let ru_name = cf_type_ru_name(type_name);
            lines.push(format!("  {ru_name:<max_type_len$}  {count}"));
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn cf_append_full_info(
    lines: &mut Vec<String>,
    cfg: roxmltree::Node<'_, '_>,
    props: roxmltree::Node<'_, '_>,
    version: &str,
    cfg_name: &str,
    cfg_synonym: &str,
    cfg_version: &str,
    cfg_vendor: &str,
    cfg_compat: &str,
    cfg_ext_compat: &str,
    cfg_default_run: &str,
    cfg_script: &str,
    cfg_default_lang: &str,
    cfg_data_lock: &str,
    cfg_modality: &str,
    cfg_intf_compat: &str,
    cfg_auto_num: &str,
    cfg_sync_calls: &str,
    cfg_db_spaces: &str,
    cfg_window_mode: &str,
    cfg_comment: &str,
    cfg_prefix: &str,
    cfg_update_addr: &str,
    config_dir: &Path,
    support_lines: &[String],
    counts: &[(String, usize)],
    total_objects: usize,
) {
    let syn_part = if cfg_synonym.is_empty() {
        String::new()
    } else {
        format!(" — \"{cfg_synonym}\"")
    };
    let ver_part = if cfg_version.is_empty() {
        String::new()
    } else {
        format!(" v{cfg_version}")
    };
    lines.push(format!(
        "=== Конфигурация: {cfg_name}{syn_part}{ver_part} ==="
    ));
    lines.push(String::new());
    lines.push("--- Идентификация ---".to_string());
    lines.push(format!(
        "UUID:           {}",
        cfg.attribute("uuid").unwrap_or("")
    ));
    lines.push(format!("Имя:            {cfg_name}"));
    if !cfg_synonym.is_empty() {
        lines.push(format!("Синоним:        {cfg_synonym}"));
    }
    if !cfg_comment.is_empty() {
        lines.push(format!("Комментарий:    {cfg_comment}"));
    }
    if !cfg_prefix.is_empty() {
        lines.push(format!("Префикс:        {cfg_prefix}"));
    }
    if !cfg_vendor.is_empty() {
        lines.push(format!("Поставщик:      {cfg_vendor}"));
    }
    if !cfg_version.is_empty() {
        lines.push(format!("Версия:         {cfg_version}"));
    }
    lines.extend(support_lines.iter().cloned());
    if !cfg_update_addr.is_empty() {
        lines.push(format!("Каталог обн.:   {cfg_update_addr}"));
    }
    lines.push(String::new());
    lines.push("--- Режимы работы ---".to_string());
    lines.push(format!("Формат:              {version}"));
    lines.push(format!("Совместимость:       {cfg_compat}"));
    lines.push(format!("Совм. расширений:    {cfg_ext_compat}"));
    lines.push(format!("Режим запуска:       {cfg_default_run}"));
    lines.push(format!("Язык скриптов:       {cfg_script}"));
    lines.push(format!("Блокировки:          {cfg_data_lock}"));
    lines.push(format!("Автонумерация:       {cfg_auto_num}"));
    lines.push(format!("Модальность:         {cfg_modality}"));
    lines.push(format!("Синхр. вызовы:       {cfg_sync_calls}"));
    lines.push(format!("Интерфейс:           {cfg_intf_compat}"));
    lines.push(format!("Табл. пространства:  {cfg_db_spaces}"));
    lines.push(format!("Режим окна:          {cfg_window_mode}"));
    lines.push(String::new());
    lines.push("--- Назначение ---".to_string());
    lines.push(format!("Язык по умолч.:  {cfg_default_lang}"));
    cf_append_full_purpose_info(lines, props);
    cf_append_full_panel_layout(lines, config_dir);
    cf_append_full_home_page_summary(lines, config_dir);
    cf_append_full_storages_and_forms(lines, props);
    cf_append_full_multilang_info(lines, props);
    cf_append_full_mobile_functionalities(lines, props);
    cf_append_full_internal_info(lines, cfg);
    cf_append_full_child_objects(lines, cfg, counts, total_objects);
}

pub(crate) fn cf_append_full_purpose_info(lines: &mut Vec<String>, props: roxmltree::Node<'_, '_>) {
    if let Some(purpose_node) = props
        .children()
        .find(|node| role_info_element(*node, "UsePurposes", Some(CF_MD_NS)))
    {
        let purposes = purpose_node
            .children()
            .filter(|node| role_info_element(*node, "Value", Some(CF_V8_NS)))
            .filter_map(|node| node.text())
            .filter(|text| !text.is_empty())
            .map(ToOwned::to_owned)
            .collect::<Vec<_>>();
        if !purposes.is_empty() {
            lines.push(format!("Назначения:      {}", purposes.join(", ")));
        }
    }

    if let Some(roles_node) = props
        .children()
        .find(|node| role_info_element(*node, "DefaultRoles", Some(CF_MD_NS)))
    {
        let roles = roles_node
            .children()
            .filter(|node| role_info_element(*node, "Item", Some(CF_XR_NS)))
            .filter_map(|node| node.text())
            .filter(|text| !text.is_empty())
            .map(ToOwned::to_owned)
            .collect::<Vec<_>>();
        if !roles.is_empty() {
            lines.push(format!("Роли по умолч.:  {}", roles.len()));
            for role in roles {
                lines.push(format!("  - {role}"));
            }
        }
    }

    lines.push(format!(
        "Управл.формы в обычн.: {}",
        cf_prop_text(props, "UseManagedFormInOrdinaryApplication")
    ));
    lines.push(format!(
        "Обычн.формы в управл.: {}",
        cf_prop_text(props, "UseOrdinaryFormInManagedApplication")
    ));
    lines.push(String::new());
}

pub(crate) struct CfPanelLayout {
    pub(crate) top: Vec<Vec<String>>,
    pub(crate) left: Vec<Vec<String>>,
    pub(crate) right: Vec<Vec<String>>,
    pub(crate) bottom: Vec<Vec<String>>,
    pub(crate) declared: Vec<String>,
}

pub(crate) struct CfHomePageItem {
    pub(crate) form: String,
    pub(crate) height: i64,
    pub(crate) common: bool,
    pub(crate) roles: Vec<(String, bool)>,
}

pub(crate) struct CfHomePageLayout {
    pub(crate) template: String,
    pub(crate) left: Vec<CfHomePageItem>,
    pub(crate) right: Vec<CfHomePageItem>,
}

pub(crate) fn cf_panel_name(uuid: &str) -> String {
    match uuid {
        "cbab57f2-a0f3-4f0a-89ea-4cb19570ab75" => "Открытых".to_string(),
        "b553047f-c9aa-4157-978d-448ecad24248" => "Разделов".to_string(),
        "13322b22-3960-4d68-93a6-fe2dd7f28ca3" => "Избранного".to_string(),
        "c933ac92-92cd-459d-81cc-e0c8a83ced99" => "История".to_string(),
        "b2735bd3-d822-4430-ba59-c9e869693b24" => "Функций".to_string(),
        other => format!("?{other}"),
    }
}

pub(crate) fn cf_read_panel_layout(config_dir: &Path) -> Option<CfPanelLayout> {
    let path = config_dir
        .join("Ext")
        .join("ClientApplicationInterface.xml");
    let text = read_utf8_sig(&path).ok()?;
    let doc = Document::parse(text.trim_start_matches('\u{feff}')).ok()?;
    let root = doc.root_element();
    let side_slots = |side: &str| {
        root.children()
            .filter(|node| role_info_element(*node, side, Some(CF_CAI_NS)))
            .filter_map(|side_el| {
                let slot = side_el
                    .descendants()
                    .filter(|node| role_info_element(*node, "uuid", Some(CF_CAI_NS)))
                    .filter_map(|node| node.text())
                    .map(str::trim)
                    .filter(|text| !text.is_empty())
                    .map(cf_panel_name)
                    .collect::<Vec<_>>();
                if slot.is_empty() {
                    None
                } else {
                    Some(slot)
                }
            })
            .collect::<Vec<_>>()
    };
    let declared = root
        .children()
        .filter(|node| role_info_element(*node, "panelDef", Some(CF_CAI_NS)))
        .map(|node| cf_panel_name(node.attribute("id").unwrap_or("")))
        .collect::<Vec<_>>();
    Some(CfPanelLayout {
        top: side_slots("top"),
        left: side_slots("left"),
        right: side_slots("right"),
        bottom: side_slots("bottom"),
        declared,
    })
}

pub(crate) fn cf_format_layout_slots(slots: &[Vec<String>]) -> String {
    slots
        .iter()
        .map(|slot| {
            if slot.len() == 1 {
                slot[0].clone()
            } else {
                format!("Стек({})", slot.join(", "))
            }
        })
        .collect::<Vec<_>>()
        .join(" | ")
}

pub(crate) fn cf_append_full_panel_layout(lines: &mut Vec<String>, config_dir: &Path) {
    let Some(layout) = cf_read_panel_layout(config_dir) else {
        return;
    };
    lines.push("--- Раскладка панелей ---".to_string());
    for (side, slots) in [
        ("top", &layout.top),
        ("left", &layout.left),
        ("right", &layout.right),
        ("bottom", &layout.bottom),
    ] {
        if slots.is_empty() {
            lines.push(format!("  {:<7} —", side));
        } else {
            lines.push(format!("  {:<7} {}", side, cf_format_layout_slots(slots)));
        }
    }
    if !layout.declared.is_empty() {
        lines.push(format!("  объявлено: {}", layout.declared.join(", ")));
    }
    lines.push(String::new());
}

pub(crate) fn cf_read_home_page(config_dir: &Path) -> Option<CfHomePageLayout> {
    let path = config_dir.join("Ext").join("HomePageWorkArea.xml");
    let text = read_utf8_sig(&path).ok()?;
    let doc = Document::parse(text.trim_start_matches('\u{feff}')).ok()?;
    let root = doc.root_element();
    let template = child_text(root, "WorkingAreaTemplate", Some(CF_HP_NS))
        .trim()
        .to_string();
    Some(CfHomePageLayout {
        template,
        left: cf_home_page_column(root, "LeftColumn"),
        right: cf_home_page_column(root, "RightColumn"),
    })
}

pub(crate) fn cf_home_page_column(
    root: roxmltree::Node<'_, '_>,
    column_name: &str,
) -> Vec<CfHomePageItem> {
    let Some(column) = root
        .children()
        .find(|node| role_info_element(*node, column_name, Some(CF_HP_NS)))
    else {
        return Vec::new();
    };
    column
        .children()
        .filter(|node| role_info_element(*node, "Item", Some(CF_HP_NS)))
        .map(|item| {
            let form = child_text(item, "Form", Some(CF_HP_NS)).trim().to_string();
            let height = child_text(item, "Height", Some(CF_HP_NS))
                .trim()
                .parse::<i64>()
                .unwrap_or(10);
            let mut common = true;
            let mut roles = Vec::<(String, bool)>::new();
            if let Some(visibility) = item
                .children()
                .find(|node| role_info_element(*node, "Visibility", Some(CF_HP_NS)))
            {
                let common_text = child_text(visibility, "Common", Some(CF_XR_NS));
                if !common_text.trim().is_empty() {
                    common = common_text.trim() == "true";
                }
                roles = visibility
                    .children()
                    .filter(|node| role_info_element(*node, "Value", Some(CF_XR_NS)))
                    .map(|node| {
                        (
                            node.attribute("name").unwrap_or("").to_string(),
                            node.text().unwrap_or("").trim() == "true",
                        )
                    })
                    .collect::<Vec<_>>();
            }
            CfHomePageItem {
                form,
                height,
                common,
                roles,
            }
        })
        .collect::<Vec<_>>()
}

pub(crate) fn cf_append_full_home_page_summary(lines: &mut Vec<String>, config_dir: &Path) {
    let Some(home_page) = cf_read_home_page(config_dir) else {
        return;
    };
    lines.push("--- Начальная страница ---".to_string());
    lines.push(format!("  Шаблон: {}", home_page.template));
    lines.push(format!(
        "  LeftColumn: {}, RightColumn: {}  (детали: -Section home-page)",
        home_page.left.len(),
        home_page.right.len()
    ));
    lines.push(String::new());
}

pub(crate) fn cf_append_home_page_section(
    lines: &mut Vec<String>,
    config_dir: &Path,
    cfg_name: &str,
) {
    let Some(home_page) = cf_read_home_page(config_dir) else {
        lines.push("Файл Ext/HomePageWorkArea.xml не найден".to_string());
        return;
    };
    lines.push(format!("=== Начальная страница: {cfg_name} ==="));
    lines.push(String::new());
    lines.push(format!("Шаблон: {}", home_page.template));
    lines.push(String::new());
    for (label, items) in [
        ("LeftColumn", &home_page.left),
        ("RightColumn", &home_page.right),
    ] {
        if items.is_empty() {
            lines.push(format!("{label}: —"));
            lines.push(String::new());
            continue;
        }
        lines.push(format!("{label} ({}):", items.len()));
        for item in items {
            lines.push(cf_format_home_page_item(item, true));
            for (role, value) in &item.roles {
                lines.push(format!("      {role}: {value}"));
            }
        }
        lines.push(String::new());
    }
}

pub(crate) fn cf_format_home_page_item(item: &CfHomePageItem, detailed: bool) -> String {
    let mut badges = vec![format!("h={}", item.height)];
    if !item.common {
        badges.push("скрыта".to_string());
    }
    if !item.roles.is_empty() {
        if detailed {
            badges.push(format!("роли: {}", item.roles.len()));
        } else {
            badges.push(format!("+{} ролей", item.roles.len()));
        }
    }
    let tail = if badges.is_empty() {
        String::new()
    } else {
        format!(" ({})", badges.join(", "))
    };
    format!("    {}{tail}", item.form)
}

pub(crate) fn cf_append_full_storages_and_forms(
    lines: &mut Vec<String>,
    props: roxmltree::Node<'_, '_>,
) {
    lines.push("--- Хранилища и формы по умолчанию ---".to_string());
    for property in [
        "CommonSettingsStorage",
        "ReportsUserSettingsStorage",
        "ReportsVariantsStorage",
        "FormDataSettingsStorage",
        "DynamicListsUserSettingsStorage",
        "URLExternalDataStorage",
        "DefaultReportForm",
        "DefaultReportVariantForm",
        "DefaultReportSettingsForm",
        "DefaultReportAppearanceTemplate",
        "DefaultDynamicListSettingsForm",
        "DefaultSearchForm",
        "DefaultDataHistoryChangeHistoryForm",
        "DefaultDataHistoryVersionDataForm",
        "DefaultDataHistoryVersionDifferencesForm",
        "DefaultCollaborationSystemUsersChoiceForm",
        "DefaultConstantsForm",
        "DefaultInterface",
        "DefaultStyle",
    ] {
        let value = cf_prop_text(props, property);
        if !value.is_empty() {
            lines.push(format!("  {property}: {value}"));
        }
    }
    lines.push(String::new());
}

pub(crate) fn cf_append_full_multilang_info(
    lines: &mut Vec<String>,
    props: roxmltree::Node<'_, '_>,
) {
    let cfg_brief = cf_prop_ml(props, "BriefInformation");
    let cfg_detail = cf_prop_ml(props, "DetailedInformation");
    let cfg_copyright = cf_prop_ml(props, "Copyright");
    let cfg_vendor_addr = cf_prop_ml(props, "VendorInformationAddress");
    let cfg_info_addr = cf_prop_ml(props, "ConfigurationInformationAddress");
    if cfg_brief.is_empty()
        && cfg_detail.is_empty()
        && cfg_copyright.is_empty()
        && cfg_vendor_addr.is_empty()
        && cfg_info_addr.is_empty()
    {
        return;
    }

    lines.push("--- Информация ---".to_string());
    if !cfg_brief.is_empty() {
        lines.push(format!("Краткая:         {cfg_brief}"));
    }
    if !cfg_detail.is_empty() {
        lines.push(format!("Подробная:       {cfg_detail}"));
    }
    if !cfg_copyright.is_empty() {
        lines.push(format!("Copyright:       {cfg_copyright}"));
    }
    if !cfg_vendor_addr.is_empty() {
        lines.push(format!("Сайт поставщика: {cfg_vendor_addr}"));
    }
    if !cfg_info_addr.is_empty() {
        lines.push(format!("Адрес информ.:   {cfg_info_addr}"));
    }
    lines.push(String::new());
}

pub(crate) fn cf_append_full_mobile_functionalities(
    lines: &mut Vec<String>,
    props: roxmltree::Node<'_, '_>,
) {
    let Some(mobile_func) = props.children().find(|node| {
        role_info_element(
            *node,
            "UsedMobileApplicationFunctionalities",
            Some(CF_MD_NS),
        )
    }) else {
        return;
    };

    let mut enabled = Vec::<String>::new();
    let mut disabled = Vec::<String>::new();
    for func in mobile_func
        .children()
        .filter(|node| role_info_element(*node, "functionality", None))
    {
        let name = child_text(func, "functionality", None);
        let use_flag = child_text(func, "use", None);
        if use_flag == "true" {
            enabled.push(name);
        } else {
            disabled.push(name);
        }
    }

    let total = enabled.len() + disabled.len();
    lines.push(format!(
        "--- Мобильные функциональности ({total}, включено: {}) ---",
        enabled.len()
    ));
    for name in enabled {
        lines.push(format!("  [+] {name}"));
    }
    for name in disabled {
        lines.push(format!("  [-] {name}"));
    }
    lines.push(String::new());
}

pub(crate) fn cf_append_full_internal_info(lines: &mut Vec<String>, cfg: roxmltree::Node<'_, '_>) {
    let Some(internal_info) = cfg
        .children()
        .find(|node| role_info_element(*node, "InternalInfo", Some(CF_MD_NS)))
    else {
        return;
    };
    let contained = internal_info
        .children()
        .filter(|node| role_info_element(*node, "ContainedObject", Some(CF_XR_NS)))
        .collect::<Vec<_>>();
    lines.push(format!(
        "--- InternalInfo ({} ContainedObject) ---",
        contained.len()
    ));
    for co in contained {
        let class_id = child_text(co, "ClassId", Some(CF_XR_NS));
        let object_id = child_text(co, "ObjectId", Some(CF_XR_NS));
        lines.push(format!("  {class_id} -> {object_id}"));
    }
    lines.push(String::new());
}

pub(crate) fn cf_append_full_child_objects(
    lines: &mut Vec<String>,
    cfg: roxmltree::Node<'_, '_>,
    counts: &[(String, usize)],
    total_objects: usize,
) {
    lines.push(format!("--- Состав ({total_objects} объектов) ---"));
    lines.push(String::new());
    let child_objects = cfg
        .children()
        .find(|node| role_info_element(*node, "ChildObjects", Some(CF_MD_NS)));

    for type_name in cf_type_order() {
        let Some((_, count)) = counts.iter().find(|(name, _)| name == type_name) else {
            continue;
        };
        lines.push(format!(
            "  {} ({type_name}): {count}",
            cf_type_ru_name(type_name)
        ));
        if let Some(child_objects) = child_objects {
            for child in child_objects
                .children()
                .filter(|node| role_info_element(*node, type_name, Some(CF_MD_NS)))
            {
                lines.push(format!("    {}", child.text().unwrap_or("")));
            }
        }
    }
}

pub(crate) fn cf_paginate(lines: Vec<String>, args: &Map<String, Value>) -> String {
    let total = lines.len();
    let limit = int_arg(args, &["limit", "Limit"]).unwrap_or(150).max(0) as usize;
    let offset = int_arg(args, &["offset", "Offset"]).unwrap_or(0).max(0) as usize;
    if offset > 0 || limit < total {
        let start = offset.min(total);
        let end = (start + limit).min(total);
        let mut result = lines[start..end].join("\n");
        if end < total {
            result.push_str(&format!(
                "\n\n... ({end} of {total} lines, use -Offset {end} to continue)"
            ));
        }
        result
    } else {
        lines.join("\n")
    }
}

pub(crate) fn cf_type_order() -> &'static [&'static str] {
    METADATA_KIND_TAGS
}

pub(crate) fn cf_type_ru_name(type_name: &str) -> &'static str {
    metadata_kind(type_name).map_or("Unknown", |kind| kind.display_name_ru)
}

#[cfg(test)]
mod metadata_kind_consumer_tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn cf_consumers_share_the_canonical_metadata_kind_order() {
        let validate_kinds = cf_validate_child_object_types();

        assert_eq!(validate_kinds.len(), 45);
        assert_eq!(validate_kinds, cf_type_order());
        let bot_index = validate_kinds
            .iter()
            .position(|kind| *kind == "Bot")
            .expect("Bot must be a modeled metadata kind");
        assert_eq!(validate_kinds[bot_index - 1], "CommonModule");
        assert_eq!(validate_kinds[bot_index + 1], "CommonAttribute");
        assert_eq!(cf_validate_child_type_dir("Bot"), Some("Bots"));
        assert_eq!(cf_type_ru_name("Bot"), "Боты");
        assert_eq!(cf_validate_child_type_dir("SyntheticMetadata"), None);

        let directories = validate_kinds
            .iter()
            .map(|kind| cf_validate_child_type_dir(kind).expect("known kind has a directory"))
            .collect::<HashSet<_>>();
        assert_eq!(directories.len(), validate_kinds.len());
        for kind in validate_kinds {
            let directory = cf_validate_child_type_dir(kind).unwrap();
            assert_eq!(cf_edit_dir_to_type(directory), Some(*kind));
        }
    }

    #[test]
    fn child_object_insertion_orders_bot_and_rejects_unknown_kinds() {
        let mut xml = concat!(
            "<MetaDataObject><Configuration><ChildObjects>\n",
            "\t<CommonModule>Core</CommonModule>\n",
            "\t<CommonAttribute>Shared</CommonAttribute>\n",
            "</ChildObjects></Configuration></MetaDataObject>"
        )
        .to_string();

        assert!(cf_edit_add_child_object_text(&mut xml, "Bot", "Assistant").unwrap());
        assert!(
            xml.find("<CommonModule>Core</CommonModule>").unwrap()
                < xml.find("<Bot>Assistant</Bot>").unwrap()
        );
        assert!(
            xml.find("<Bot>Assistant</Bot>").unwrap()
                < xml
                    .find("<CommonAttribute>Shared</CommonAttribute>")
                    .unwrap()
        );
        assert!(!cf_edit_add_child_object_text(&mut xml, "Bot", "Assistant").unwrap());

        let before_unknown = xml.clone();
        let error = cf_edit_add_child_object_text(&mut xml, "SyntheticMetadata", "Unknown")
            .expect_err("unknown metadata kinds must be rejected");
        assert!(
            error.contains("Unknown type 'SyntheticMetadata'"),
            "{error}"
        );
        assert_eq!(xml, before_unknown);
    }

    #[test]
    fn child_object_insertion_preserves_crlf_for_existing_and_self_closing_lists() {
        let mut populated = concat!(
            "<MetaDataObject>\r\n",
            "\t<Configuration>\r\n",
            "\t\t<ChildObjects>\r\n",
            "\t\t\t<Catalog>Items</Catalog>\r\n",
            "\t\t</ChildObjects>\r\n",
            "\t</Configuration>\r\n",
            "</MetaDataObject>"
        )
        .to_string();

        assert!(cf_edit_add_child_object_text(&mut populated, "Report", "Sales").unwrap());
        assert!(populated.contains(concat!(
            "\t\t\t<Catalog>Items</Catalog>\r\n",
            "\t\t\t<Report>Sales</Report>\r\n",
            "\t\t</ChildObjects>"
        )));
        assert!(!populated.replace("\r\n", "").contains('\n'));

        let mut empty = concat!(
            "<MetaDataObject>\r\n",
            "\t<Configuration>\r\n",
            "\t\t<ChildObjects/>\r\n",
            "\t</Configuration>\r\n",
            "</MetaDataObject>"
        )
        .to_string();
        assert!(cf_edit_add_child_object_text(&mut empty, "Role", "User").unwrap());
        assert!(empty.contains(concat!(
            "\t\t<ChildObjects>\r\n",
            "\t\t\t<Role>User</Role>\r\n",
            "\t\t</ChildObjects>"
        )));
        assert!(!empty.replace("\r\n", "").contains('\n'));
    }

    #[test]
    fn child_object_insertion_preserves_cr_only_line_boundaries() {
        let mut xml = concat!(
            "<MetaDataObject>\r",
            "\t<Configuration>\r",
            "\t\t<ChildObjects>\r",
            "\t\t\t<Catalog>Items</Catalog>\r",
            "\t\t</ChildObjects>\r",
            "\t</Configuration>\r",
            "</MetaDataObject>"
        )
        .to_string();

        assert!(cf_edit_add_child_object_text(&mut xml, "Role", "Reader").unwrap());

        assert!(
            xml.contains(concat!(
                "\t\t\t<Role>Reader</Role>\r",
                "\t\t\t<Catalog>Items</Catalog>\r",
                "\t\t</ChildObjects>"
            )),
            "{xml:?}"
        );
        assert!(!xml.contains('\n'));
    }

    #[test]
    fn child_object_insertion_targets_only_the_root_metadata_child_objects() {
        let mut xml = concat!(
            "\u{feff}<MetaDataObject>\r\n",
            "\t<Configuration>\r\n",
            "\t\t<Properties>\r\n",
            "\t\t\t<!-- <ChildObjects><Role>CommentTrap</Role></ChildObjects> -->\r\n",
            "\t\t\t<Nested><ChildObjects><Role>NestedTrap</Role></ChildObjects></Nested>\r\n",
            "\t\t</Properties>\r\n",
            "\t\t<ChildObjects>\r\n",
            "\t\t\t<Catalog>Items</Catalog>\r\n",
            "\t\t</ChildObjects>\r\n",
            "\t</Configuration>\r\n",
            "</MetaDataObject>"
        )
        .to_string();

        assert!(cf_edit_add_child_object_text(&mut xml, "Role", "Reader").unwrap());

        assert!(
            xml.contains("<Nested><ChildObjects><Role>NestedTrap</Role></ChildObjects></Nested>")
        );
        assert!(xml.contains(concat!(
            "\t\t<ChildObjects>\r\n",
            "\t\t\t<Role>Reader</Role>\r\n",
            "\t\t\t<Catalog>Items</Catalog>\r\n",
            "\t\t</ChildObjects>"
        )));
        assert_eq!(xml.matches("<Role>Reader</Role>").count(), 1);
        assert!(!xml.replace("\r\n", "").contains('\n'));
    }
}

pub(crate) fn edit_cf(args: &Map<String, Value>, context: &WorkspaceContext) -> AdapterOutcome {
    let edit_result = (|| -> Result<(String, PathBuf, Vec<PathBuf>, bool), String> {
        let definition_file = path_arg(args, &["definitionFile", "DefinitionFile"]);
        let operation = string_arg(args, &["operation", "Operation"]);
        if definition_file.is_some() && operation.is_some() {
            return Err("Cannot use both -DefinitionFile and -Operation".to_string());
        }
        if definition_file.is_none() && operation.is_none() {
            return Err("Either -DefinitionFile or -Operation is required".to_string());
        }

        let config_path = resolve_cf_edit_config_path(args, context)?;
        let config_dir = config_path
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| context.cwd.clone());
        let source_text = read_utf8_sig(&config_path)?;
        let mut text = lxml_parser_normalized_text(&source_text);
        if !text.contains("<Configuration") {
            return Err("No <Configuration> element found".to_string());
        }
        let obj_name = cf_edit_config_name(&text)?;

        let operations = cf_edit_operations(args, &context.cwd, operation, definition_file)?;
        cf_edit_validate_child_object_kinds(&operations)?;
        let mut add_count = 0usize;
        let mut remove_count = 0usize;
        let mut modify_count = 0usize;
        let mut stdout = format!("[INFO] Configuration: {obj_name}\n");
        let mut artifacts = vec![config_path.clone()];
        let mut config_changed = false;

        for (op_name, op_value) in operations {
            match op_name.as_str() {
                "modify-property" => {
                    for item in cf_edit_batch_value(&op_value) {
                        let Some(eq_idx) = item.find('=') else {
                            return Err(format!(
                                "Invalid property format '{item}', expected 'Key=Value'"
                            ));
                        };
                        if eq_idx < 1 {
                            return Err(format!(
                                "Invalid property format '{item}', expected 'Key=Value'"
                            ));
                        }
                        let prop_name = item[..eq_idx].trim();
                        let prop_value = item[eq_idx + 1..].trim();
                        let replacement = if cf_edit_ml_properties().contains(&prop_name) {
                            cf_edit_ml_property_xml(prop_name, prop_value)
                        } else {
                            cf_edit_scalar_property_xml(prop_name, prop_value)
                        };
                        text = cf_edit_replace_property(&text, prop_name, &replacement)?;
                        config_changed = true;
                        modify_count += 1;
                        stdout.push_str(&format!("[INFO] Set {prop_name} = \"{prop_value}\"\n"));
                    }
                }
                "remove-childObject" => {
                    for item in cf_edit_batch_value(&op_value) {
                        let (type_name, obj_name_val) = cf_edit_parse_child_object(&item)?;
                        if cf_edit_remove_child_object_text(&mut text, type_name, obj_name_val)? {
                            remove_count += 1;
                            config_changed = true;
                            stdout
                                .push_str(&format!("[INFO] Removed: {type_name}.{obj_name_val}\n"));
                        } else {
                            stdout.push_str(&format!(
                                "[WARN] Not found: {type_name}.{obj_name_val}\n"
                            ));
                        }
                    }
                }
                "add-childObject" => {
                    for item in cf_edit_batch_value(&op_value) {
                        let (type_name, obj_name_val) = cf_edit_parse_child_object(&item)?;
                        if cf_validate_child_object_type_index(type_name).is_none() {
                            return Err(format!("Unknown type '{type_name}'"));
                        }
                        let type_dir = cf_validate_child_type_dir(type_name)
                            .ok_or_else(|| format!("Unknown type '{type_name}'"))?;
                        let object_file = config_dir
                            .join(type_dir)
                            .join(format!("{obj_name_val}.xml"));
                        if !object_file.exists() {
                            if type_name == "Bot" {
                                return Err(format!(
                                    "Object file not found: {type_dir}/{obj_name_val}.xml\n\
                                     cf-edit add-childObject only references objects that already exist on disk.\n\
                                     meta-compile does not support Bot; create the Bot metadata with platform tooling, then retry."
                                ));
                            }
                            let hint_skill = match type_name {
                                "Subsystem" => "subsystem-compile",
                                "Role" => "role-compile",
                                _ => "meta-compile",
                            };
                            return Err(format!(
                                "Object file not found: {type_dir}/{obj_name_val}.xml\n\
                                 cf-edit add-childObject only references objects that already exist on disk.\n\
                                 To create a new {type_name}, use {hint_skill} (auto-registers in Configuration.xml):\n\
                                   /{hint_skill} with {{\"type\":\"{type_name}\",\"name\":\"{obj_name_val}\"}}"
                            ));
                        }
                        if cf_edit_add_child_object_text(&mut text, type_name, obj_name_val)? {
                            add_count += 1;
                            config_changed = true;
                            stdout.push_str(&format!("[INFO] Added: {type_name}.{obj_name_val}\n"));
                        } else {
                            stdout.push_str(&format!(
                                "[WARN] Already exists: {type_name}.{obj_name_val}\n"
                            ));
                        }
                    }
                }
                "set-defaultRoles" => {
                    let roles = cf_edit_batch_value(&op_value)
                        .into_iter()
                        .map(|role| cf_edit_role_ref(&role))
                        .collect::<Vec<_>>();
                    text = cf_edit_replace_default_roles(&text, &roles)?;
                    config_changed = true;
                    modify_count += 1;
                    if roles.is_empty() {
                        stdout.push_str("[INFO] Cleared DefaultRoles\n");
                    } else {
                        stdout
                            .push_str(&format!("[INFO] Set DefaultRoles: {} roles\n", roles.len()));
                    }
                }
                "add-defaultRole" => {
                    let mut roles = cf_edit_default_roles(&text)?;
                    for role in cf_edit_batch_value(&op_value) {
                        let role_name = cf_edit_role_ref(&role);
                        if roles.contains(&role_name) {
                            stdout.push_str(&format!(
                                "[WARN] DefaultRole already exists: {role_name}\n"
                            ));
                        } else {
                            roles.push(role_name.clone());
                            add_count += 1;
                            stdout.push_str(&format!("[INFO] Added DefaultRole: {role_name}\n"));
                        }
                    }
                    text = cf_edit_replace_default_roles(&text, &roles)?;
                    config_changed = true;
                }
                "remove-defaultRole" => {
                    let mut roles = cf_edit_default_roles(&text)?;
                    for role in cf_edit_batch_value(&op_value) {
                        let role_name = cf_edit_role_ref(&role);
                        if let Some(index) =
                            roles.iter().position(|existing| existing == &role_name)
                        {
                            roles.remove(index);
                            remove_count += 1;
                            stdout.push_str(&format!("[INFO] Removed DefaultRole: {role_name}\n"));
                        } else {
                            stdout
                                .push_str(&format!("[WARN] DefaultRole not found: {role_name}\n"));
                        }
                    }
                    text = cf_edit_replace_default_roles(&text, &roles)?;
                    config_changed = true;
                }
                "set-panels" => {
                    let path = cf_edit_set_panels(&op_value, &config_dir)?;
                    modify_count += 1;
                    stdout.push_str(&format!("[INFO] Wrote panel layout: {}\n", path.display()));
                    artifacts.push(path);
                }
                "set-home-page" => {
                    let path = cf_edit_set_home_page(&op_value, &config_dir)?;
                    modify_count += 1;
                    stdout.push_str(&format!(
                        "[INFO] Wrote home page layout: {}\n",
                        path.display()
                    ));
                    artifacts.push(path);
                }
                _ => return Err(format!("Unknown operation: {op_name}")),
            }
        }

        if config_changed {
            write_utf8_bom(&config_path, &cf_edit_serialized_text(&text, &source_text))?;
            stdout.push_str(&format!("[INFO] Saved: {}\n", config_path.display()));
        } else {
            stdout.push_str("[INFO] No Configuration.xml changes\n");
        }

        if !bool_arg(args, &["noValidate", "NoValidate"]) {
            stdout.push('\n');
            stdout.push_str("--- Running cf-validate ---\n");
            let mut validate_args = Map::new();
            validate_args.insert(
                "ConfigPath".to_string(),
                Value::String(config_path.display().to_string()),
            );
            if let Some(validate_stdout) = validate_cf(&validate_args, context).stdout {
                stdout.push_str(&validate_stdout);
            }
        }

        stdout.push('\n');
        stdout.push_str("=== cf-edit summary ===\n");
        stdout.push_str(&format!("  Configuration: {obj_name}\n"));
        stdout.push_str(&format!("  Added:         {add_count}\n"));
        stdout.push_str(&format!("  Removed:       {remove_count}\n"));
        stdout.push_str(&format!("  Modified:      {modify_count}\n"));
        Ok((stdout, config_path, artifacts, config_changed))
    })();

    match edit_result {
        Ok((stdout, config_path, artifacts, config_changed)) => {
            let mut changes = Vec::new();
            if config_changed {
                changes.push(format!("updated {}", config_path.display()));
            }
            for artifact in &artifacts {
                if artifact != &config_path {
                    changes.push(format!("updated {}", artifact.display()));
                }
            }
            AdapterOutcome {
                ok: true,
                summary: "unica.cf.edit completed with native Configuration.xml editor".to_string(),
                changes,
                warnings: Vec::new(),
                errors: Vec::new(),
                artifacts: artifacts
                    .into_iter()
                    .map(|path| path.display().to_string())
                    .collect(),
                stdout: Some(stdout),
                stderr: None,
                command: None,
            }
        }
        Err(error) => AdapterOutcome {
            ok: false,
            summary: "unica.cf.edit failed in native Configuration.xml editor".to_string(),
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

pub(crate) fn cf_edit_operations(
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
        let items = match parsed {
            Value::Array(items) => items,
            other => vec![other],
        };
        let mut operations = Vec::new();
        for item in items {
            let op_name = item
                .get("operation")
                .and_then(Value::as_str)
                .unwrap_or(operation.unwrap_or(""))
                .to_string();
            let value = item
                .get("value")
                .cloned()
                .unwrap_or_else(|| Value::String(String::new()));
            operations.push((op_name, value));
        }
        Ok(operations)
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

pub(crate) fn cf_edit_validate_child_object_kinds(
    operations: &[(String, Value)],
) -> Result<(), String> {
    for (op_name, op_value) in operations {
        if !matches!(op_name.as_str(), "add-childObject" | "remove-childObject") {
            continue;
        }
        for item in cf_edit_batch_value(op_value) {
            let (type_name, _) = cf_edit_parse_child_object(&item)?;
            if cf_validate_child_object_type_index(type_name).is_none() {
                return Err(format!("Unknown type '{type_name}'"));
            }
        }
    }
    Ok(())
}

pub(crate) fn cf_edit_parse_child_object(item: &str) -> Result<(&str, &str), String> {
    let Some((type_name, object_name)) = item.split_once('.') else {
        return Err(format!("Invalid format '{item}', expected 'Type.Name'"));
    };
    if type_name.is_empty() || object_name.is_empty() {
        return Err(format!("Invalid format '{item}', expected 'Type.Name'"));
    }
    Ok((type_name, object_name))
}

pub(crate) fn cf_edit_batch_value(value: &Value) -> Vec<String> {
    let text = match value {
        Value::String(value) => value.clone(),
        other => other.to_string(),
    };
    text.split(";;")
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}

pub(crate) fn cf_edit_config_name(text: &str) -> Result<String, String> {
    let (props_start, props_end) = cf_edit_properties_body_range(text)?;
    let body = &text[props_start..props_end];
    if let Some((_, _, Some((name_start, name_end)))) =
        cf_edit_element_range(body, "Name").map(|(start, end, body)| {
            (
                props_start + start,
                props_start + end,
                body.map(|(start, end)| (props_start + start, props_start + end)),
            )
        })
    {
        return Ok(unescape_xml(text[name_start..name_end].trim()));
    }
    Ok(String::new())
}

pub(crate) fn cf_edit_ml_properties() -> &'static [&'static str] {
    &[
        "Synonym",
        "BriefInformation",
        "DetailedInformation",
        "Copyright",
        "VendorInformationAddress",
        "ConfigurationInformationAddress",
    ]
}

pub(crate) fn cf_edit_ml_property_xml(prop_name: &str, value: &str) -> String {
    if value.is_empty() {
        return format!("<{prop_name}/>");
    }
    format!(
        "<{prop_name}>\r\n\
         \t\t\t\t<v8:item>\r\n\
         \t\t\t\t\t<v8:lang>ru</v8:lang>\r\n\
         \t\t\t\t\t<v8:content>{}</v8:content>\r\n\
         \t\t\t\t</v8:item>\r\n\
         \t\t\t</{prop_name}>",
        escape_xml(value)
    )
}

pub(crate) fn cf_edit_scalar_property_xml(prop_name: &str, value: &str) -> String {
    if value.is_empty() {
        format!("<{prop_name}/>")
    } else {
        format!("<{prop_name}>{}</{prop_name}>", escape_xml(value))
    }
}

pub(crate) fn cf_edit_replace_property(
    text: &str,
    prop_name: &str,
    replacement: &str,
) -> Result<String, String> {
    let (props_start, props_end) = cf_edit_properties_body_range(text)?;
    let body = &text[props_start..props_end];
    let Some((start, end, _)) = cf_edit_element_range(body, prop_name) else {
        return Err(format!("Property '{prop_name}' not found in Properties"));
    };
    let abs_start = props_start + start;
    let abs_end = props_start + end;
    Ok(format!(
        "{}{}{}",
        &text[..abs_start],
        replacement,
        &text[abs_end..]
    ))
}

pub(crate) fn cf_edit_default_roles(text: &str) -> Result<Vec<String>, String> {
    let (props_start, props_end) = cf_edit_properties_body_range(text)?;
    let body = &text[props_start..props_end];
    let Some((_, _, body_range)) = cf_edit_element_range(body, "DefaultRoles") else {
        return Err("No <DefaultRoles> element found in Properties".to_string());
    };
    let Some((start, end)) = body_range else {
        return Ok(Vec::new());
    };
    let roles_body = &body[start..end];
    let mut roles = Vec::new();
    let mut offset = 0usize;
    while let Some(rel_start) = roles_body[offset..].find("<xr:Item") {
        let item_start = offset + rel_start;
        let Some(gt_rel) = roles_body[item_start..].find('>') else {
            break;
        };
        let value_start = item_start + gt_rel + 1;
        let Some(end_rel) = roles_body[value_start..].find("</xr:Item>") else {
            break;
        };
        let value_end = value_start + end_rel;
        roles.push(unescape_xml(roles_body[value_start..value_end].trim()));
        offset = value_end + "</xr:Item>".len();
    }
    Ok(roles)
}

pub(crate) fn cf_edit_replace_default_roles(
    text: &str,
    roles: &[String],
) -> Result<String, String> {
    cf_edit_replace_property(text, "DefaultRoles", &cf_edit_default_roles_xml(roles))
}

pub(crate) fn cf_edit_default_roles_xml(roles: &[String]) -> String {
    if roles.is_empty() {
        return "<DefaultRoles/>".to_string();
    }
    let body = roles
        .iter()
        .map(|role| {
            format!(
                "\r\n\t\t\t\t<xr:Item xsi:type=\"xr:MDObjectRef\">{}</xr:Item>",
                escape_xml(role)
            )
        })
        .collect::<String>();
    format!("<DefaultRoles>{body}\r\n\t\t\t</DefaultRoles>")
}

pub(crate) fn cf_edit_role_ref(role: &str) -> String {
    if role.starts_with("Role.") {
        role.to_string()
    } else {
        format!("Role.{role}")
    }
}

pub(crate) fn cf_edit_serialized_text(text: &str, source_text: &str) -> String {
    let mut output = lxml_tree_serialized_text_like_source(text, source_text);
    let source_has_trailing_newline = source_text.ends_with('\n') || source_text.ends_with('\r');
    if !source_has_trailing_newline {
        if output.ends_with("\r\n") {
            output.truncate(output.len() - 2);
        } else if output.ends_with('\n') {
            output.pop();
        }
    }
    output
}

pub(crate) fn cf_edit_child_objects(text: &str) -> Result<Vec<(String, String)>, String> {
    let Some((_, _, body_range)) = cf_edit_element_range(text, "ChildObjects") else {
        return Err("No <ChildObjects> element found".to_string());
    };
    let Some((start, end)) = body_range else {
        return Ok(Vec::new());
    };
    let body = &text[start..end];
    let mut result = Vec::new();
    let mut offset = 0usize;
    while let Some(rel_start) = body[offset..].find('<') {
        let tag_start = offset + rel_start;
        if body[tag_start + 1..].starts_with('/') {
            offset = tag_start + 1;
            continue;
        }
        let Some(gt_rel) = body[tag_start..].find('>') else {
            break;
        };
        let tag_end = tag_start + gt_rel;
        let tag_name = body[tag_start + 1..tag_end].trim();
        if tag_name.is_empty() || tag_name.contains(' ') || tag_name.ends_with('/') {
            offset = tag_end + 1;
            continue;
        }
        let close = format!("</{tag_name}>");
        let value_start = tag_end + 1;
        let Some(close_rel) = body[value_start..].find(&close) else {
            break;
        };
        let value_end = value_start + close_rel;
        result.push((
            tag_name.to_string(),
            unescape_xml(body[value_start..value_end].trim()),
        ));
        offset = value_end + close.len();
    }
    Ok(result)
}

#[derive(Debug, Clone)]
pub(crate) struct CfEditChildObjectEntry {
    pub(crate) type_name: String,
    pub(crate) object_name: String,
    pub(crate) range: std::ops::Range<usize>,
    pub(crate) line_range: std::ops::Range<usize>,
}

pub(crate) fn cf_edit_child_object_entries(
    text: &str,
) -> Result<Vec<CfEditChildObjectEntry>, String> {
    cf_edit_root_child_objects(text).map(|(_, entries)| entries)
}

fn cf_edit_root_child_objects(
    text: &str,
) -> Result<(CfEditElementRange, Vec<CfEditChildObjectEntry>), String> {
    let source = text.trim_start_matches('\u{feff}');
    let source_offset = text.len() - source.len();
    let doc = Document::parse(source).map_err(|err| format!("XML parse error: {err}"))?;
    let root = doc.root_element();
    if root.tag_name().name() != "MetaDataObject" {
        return Err("Root element must be <MetaDataObject>".to_string());
    }

    let mut direct_lists = root
        .children()
        .filter(|node| node.is_element())
        .filter_map(|owner| {
            owner
                .children()
                .find(|node| node.is_element() && node.tag_name().name() == "ChildObjects")
        });
    let child_objects = direct_lists.next().ok_or_else(|| {
        "No <ChildObjects> element found directly under the root metadata object".to_string()
    })?;
    if direct_lists.next().is_some() {
        return Err("Multiple root metadata <ChildObjects> elements found".to_string());
    }

    let node_range = child_objects.range();
    let child_start = source_offset + node_range.start;
    let child_end = source_offset + node_range.end;
    let child_text = &text[child_start..child_end];
    let open_end = child_text
        .find('>')
        .ok_or_else(|| "Malformed root metadata <ChildObjects> element".to_string())?;
    let body_range = if child_text[..=open_end].trim_end().ends_with("/>") {
        None
    } else {
        let close_start = child_text
            .rfind("</ChildObjects>")
            .ok_or_else(|| "Malformed root metadata </ChildObjects> element".to_string())?;
        Some((child_start + open_end + 1, child_start + close_start))
    };

    let mut entries = Vec::new();
    for child in child_objects.children().filter(|node| node.is_element()) {
        let raw_range = child.range();
        let range = (source_offset + raw_range.start)..(source_offset + raw_range.end);
        let line_start = cf_edit_line_start(text, range.start);
        let prefix_is_indent = text[line_start..range.start]
            .chars()
            .all(|ch| ch == '\t' || ch == ' ');
        let start = if prefix_is_indent {
            line_start
        } else {
            range.start
        };
        let end = if text[range.end..].starts_with("\r\n") {
            range.end + 2
        } else if matches!(text[range.end..].chars().next(), Some('\n' | '\r')) {
            range.end + 1
        } else {
            range.end
        };
        entries.push(CfEditChildObjectEntry {
            type_name: child.tag_name().name().to_string(),
            object_name: unescape_xml(child.text().unwrap_or("").trim()),
            range,
            line_range: start..end,
        });
    }
    Ok(((child_start, child_end, body_range), entries))
}

pub(crate) fn cf_edit_line_indent(text: &str, index: usize) -> Option<String> {
    let line_start = cf_edit_line_start(text, index);
    let indent = &text[line_start..index];
    if indent.chars().all(|ch| ch == '\t' || ch == ' ') {
        Some(indent.to_string())
    } else {
        None
    }
}

fn cf_edit_line_start(text: &str, index: usize) -> usize {
    text[..index]
        .rfind(['\r', '\n'])
        .map_or(0, |position| position + 1)
}

pub(crate) fn cf_edit_child_name_key(value: &str) -> String {
    value.chars().flat_map(char::to_lowercase).collect()
}

pub(crate) fn cf_edit_child_name_cmp(left: &str, right: &str) -> std::cmp::Ordering {
    cf_edit_child_name_key(left)
        .cmp(&cf_edit_child_name_key(right))
        .then_with(|| left.cmp(right))
}

pub(crate) fn cf_edit_line_ending(text: &str) -> &'static str {
    let bytes = text.as_bytes();
    let Some(index) = bytes.iter().position(|byte| matches!(byte, b'\r' | b'\n')) else {
        return "\n";
    };
    if bytes[index] == b'\r' && bytes.get(index + 1) == Some(&b'\n') {
        "\r\n"
    } else if bytes[index] == b'\r' {
        "\r"
    } else {
        "\n"
    }
}

pub(crate) fn cf_edit_child_object_line(
    type_name: &str,
    object_name: &str,
    indent: &str,
    line_ending: &str,
) -> String {
    format!(
        "{indent}<{type_name}>{}</{type_name}>{line_ending}",
        escape_xml(object_name),
    )
}

pub(crate) fn cf_edit_remove_child_object_text(
    text: &mut String,
    type_name: &str,
    object_name: &str,
) -> Result<bool, String> {
    if metadata_kind(type_name).is_none() {
        return Err(format!("Unknown type '{type_name}'"));
    }
    let entries = cf_edit_child_object_entries(text)?;
    let Some(entry) = entries
        .into_iter()
        .find(|entry| entry.type_name == type_name && entry.object_name == object_name)
    else {
        return Ok(false);
    };
    text.replace_range(entry.line_range, "");
    Ok(true)
}

pub(crate) fn cf_edit_add_child_object_text(
    text: &mut String,
    type_name: &str,
    object_name: &str,
) -> Result<bool, String> {
    let new_type_index =
        metadata_kind_index(type_name).ok_or_else(|| format!("Unknown type '{type_name}'"))?;
    let ((child_start, child_end, body_range), entries) = cf_edit_root_child_objects(text)?;
    let line_ending = cf_edit_line_ending(text);
    if entries
        .iter()
        .any(|entry| entry.type_name == type_name && entry.object_name == object_name)
    {
        return Ok(false);
    }

    let target = entries.iter().find(|entry| {
        let entry_type_index =
            cf_validate_child_object_type_index(&entry.type_name).unwrap_or(usize::MAX);
        entry_type_index > new_type_index
            || (entry_type_index == new_type_index
                && cf_edit_child_name_cmp(object_name, &entry.object_name).is_lt())
    });

    if let Some(target) = target {
        let indent =
            cf_edit_line_indent(text, target.range.start).unwrap_or_else(|| "\t\t\t".to_string());
        let line = cf_edit_child_object_line(type_name, object_name, &indent, line_ending);
        text.insert_str(target.line_range.start, &line);
        return Ok(true);
    }

    if let Some(last) = entries.last() {
        let indent =
            cf_edit_line_indent(text, last.range.start).unwrap_or_else(|| "\t\t\t".to_string());
        let line = cf_edit_child_object_line(type_name, object_name, &indent, line_ending);
        text.insert_str(last.line_range.end, &line);
        return Ok(true);
    }

    let Some((body_start, body_end)) = body_range else {
        let open_indent = cf_edit_line_indent(text, child_start).unwrap_or_default();
        let child_indent = format!("{open_indent}\t");
        let close_indent = open_indent;
        let replacement = format!(
            "<ChildObjects>{line_ending}{}{}</ChildObjects>",
            cf_edit_child_object_line(type_name, object_name, &child_indent, line_ending),
            close_indent
        );
        text.replace_range(child_start..child_end, &replacement);
        return Ok(true);
    };

    let close_line_start = cf_edit_line_start(text, body_end);
    let close_indent = if text[close_line_start..body_end]
        .chars()
        .all(|ch| ch == '\t' || ch == ' ')
    {
        text[close_line_start..body_end].to_string()
    } else {
        cf_edit_line_indent(text, child_start).unwrap_or_default()
    };
    let child_indent = format!("{close_indent}\t");
    let line = cf_edit_child_object_line(type_name, object_name, &child_indent, line_ending);
    if body_start == body_end {
        text.insert_str(body_start, &format!("{line_ending}{line}{close_indent}"));
    } else {
        text.insert_str(close_line_start, &line);
    }
    Ok(true)
}

pub(crate) fn cf_edit_replace_child_objects(
    text: &str,
    children: &[(String, String)],
) -> Result<String, String> {
    let Some((start, end, _)) = cf_edit_element_range(text, "ChildObjects") else {
        return Err("No <ChildObjects> element found".to_string());
    };
    let replacement = cf_edit_child_objects_xml(children);
    Ok(format!("{}{}{}", &text[..start], replacement, &text[end..]))
}

pub(crate) fn cf_edit_child_objects_xml(children: &[(String, String)]) -> String {
    if children.is_empty() {
        return "<ChildObjects/>".to_string();
    }
    let mut body = String::from("<ChildObjects>\n");
    for (index, (type_name, obj_name)) in children.iter().enumerate() {
        body.push_str(&format!(
            "\t\t\t<{type_name}>{}</{type_name}>",
            escape_xml(obj_name)
        ));
        if index + 1 == children.len() {
            body.push('\n');
        } else {
            body.push_str("\r\n");
        }
    }
    body.push_str("\t\t</ChildObjects>");
    body
}

pub(crate) fn cf_edit_child_object_cmp(
    left: &(String, String),
    right: &(String, String),
) -> std::cmp::Ordering {
    let left_idx = cf_validate_child_object_type_index(&left.0).unwrap_or(usize::MAX);
    let right_idx = cf_validate_child_object_type_index(&right.0).unwrap_or(usize::MAX);
    left_idx.cmp(&right_idx).then_with(|| left.1.cmp(&right.1))
}

pub(crate) fn cf_edit_properties_body_range(text: &str) -> Result<(usize, usize), String> {
    let Some((_, _, Some(body))) = cf_edit_element_range(text, "Properties") else {
        return Err("No <Properties> element found".to_string());
    };
    Ok(body)
}

type CfEditElementRange = (usize, usize, Option<(usize, usize)>);

pub(crate) fn cf_edit_element_range(text: &str, tag: &str) -> Option<CfEditElementRange> {
    let needle = format!("<{tag}");
    let mut offset = 0usize;
    while let Some(rel_start) = text[offset..].find(&needle) {
        let start = offset + rel_start;
        let boundary_index = start + needle.len();
        let boundary = text[boundary_index..].chars().next();
        let is_boundary = match boundary {
            Some('>') | Some('/') => true,
            Some(ch) => ch.is_whitespace(),
            None => false,
        };
        if !is_boundary {
            offset = boundary_index;
            continue;
        }
        let gt = start + text[start..].find('>')?;
        let open_tag = &text[start..=gt];
        if open_tag.trim_end().ends_with("/>") {
            return Some((start, gt + 1, None));
        }
        let close = format!("</{tag}>");
        let body_start = gt + 1;
        let close_start = body_start + text[body_start..].find(&close)?;
        let end = close_start + close.len();
        return Some((start, end, Some((body_start, close_start))));
    }
    None
}

pub(crate) fn cf_edit_set_panels(value: &Value, config_dir: &Path) -> Result<PathBuf, String> {
    let layout = cf_edit_json_object(value, "set-panels value must be valid JSON object")?;
    if layout.is_empty() {
        return Err("set-panels value must be non-empty object".to_string());
    }
    let sides = ["top", "left", "right", "bottom"];
    for key in layout.keys() {
        if !sides.contains(&key.as_str()) {
            return Err(format!(
                "Unknown side '{key}'. Allowed: {}",
                sides.join(", ")
            ));
        }
    }
    let mut body_parts = Vec::new();
    for side in sides {
        let Some(entries) = layout.get(side) else {
            continue;
        };
        let entry_values = if let Some(items) = entries.as_array() {
            items.clone()
        } else {
            vec![entries.clone()]
        };
        for entry in entry_values {
            let entry_xml = cf_edit_panel_entry_xml(&entry, "\t\t")?;
            body_parts.push(format!("\t<{side}>\r\n{entry_xml}\r\n\t</{side}>"));
        }
    }
    let body = body_parts.join("\r\n");
    let body_block = if body.is_empty() {
        String::new()
    } else {
        format!("{body}\r\n")
    };
    let declarations = concat!(
        "\t<panelDef id=\"b553047f-c9aa-4157-978d-448ecad24248\"/>\r\n",
        "\t<panelDef id=\"13322b22-3960-4d68-93a6-fe2dd7f28ca3\"/>\r\n",
        "\t<panelDef id=\"c933ac92-92cd-459d-81cc-e0c8a83ced99\"/>\r\n",
        "\t<panelDef id=\"cbab57f2-a0f3-4f0a-89ea-4cb19570ab75\"/>\r\n",
        "\t<panelDef id=\"b2735bd3-d822-4430-ba59-c9e869693b24\"/>",
    );
    let cai_xml = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\r\n\
         <ClientApplicationInterface xmlns=\"http://v8.1c.ru/8.2/managed-application/core\" \
         xmlns:xs=\"http://www.w3.org/2001/XMLSchema\" \
         xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\" \
         xsi:type=\"InterfaceLayouter\">\r\n\
         {body_block}{declarations}\r\n\
         </ClientApplicationInterface>"
    );
    let ext_dir = config_dir.join("Ext");
    fs::create_dir_all(&ext_dir)
        .map_err(|err| format!("failed to create {}: {err}", ext_dir.display()))?;
    let path = ext_dir.join("ClientApplicationInterface.xml");
    write_utf8_bom(&path, &cai_xml)?;
    Ok(path)
}

pub(crate) fn cf_edit_panel_entry_xml(entry: &Value, indent: &str) -> Result<String, String> {
    if let Some(alias) = entry.as_str() {
        let key = cf_edit_panel_alias(alias);
        let Some(uuid) = cf_edit_panel_uuid(key) else {
            return Err(format!(
                "Unknown panel alias '{alias}'. Allowed: favorites, functions, history, open, sections"
            ));
        };
        let inst = fresh_uuid();
        return Ok(format!(
            "{indent}<panel id=\"{inst}\">\r\n{indent}\t<uuid>{uuid}</uuid>\r\n{indent}</panel>"
        ));
    }
    if let Some(group) = entry.as_object().and_then(|obj| obj.get("group")) {
        let Some(children) = group.as_array() else {
            return Err("group must contain at least one entry".to_string());
        };
        if children.is_empty() {
            return Err("group must contain at least one entry".to_string());
        }
        let gid = fresh_uuid();
        let mut inner = String::new();
        for child in children {
            let child_xml = cf_edit_panel_entry_xml(child, &format!("{indent}\t\t"))?;
            inner.push_str(&format!(
                "{indent}\t<group>\r\n{child_xml}\r\n{indent}\t</group>\r\n"
            ));
        }
        return Ok(format!(
            "{indent}<group id=\"{gid}\">\r\n{inner}{indent}</group>"
        ));
    }
    Err(format!(
        "Panel entry must be string alias or {{group:[...]}}, got: {entry}"
    ))
}

pub(crate) fn cf_edit_panel_alias(alias: &str) -> &str {
    match alias.to_lowercase().as_str() {
        "разделов" | "разделы" => "sections",
        "открытых" | "открытые" => "open",
        "избранного" | "избранное" => "favorites",
        "истории" | "история" => "history",
        "функций" | "функции" => "functions",
        _ => alias,
    }
}

pub(crate) fn cf_edit_panel_uuid(alias: &str) -> Option<&'static str> {
    match alias {
        "sections" => Some("b553047f-c9aa-4157-978d-448ecad24248"),
        "open" => Some("cbab57f2-a0f3-4f0a-89ea-4cb19570ab75"),
        "favorites" => Some("13322b22-3960-4d68-93a6-fe2dd7f28ca3"),
        "history" => Some("c933ac92-92cd-459d-81cc-e0c8a83ced99"),
        "functions" => Some("b2735bd3-d822-4430-ba59-c9e869693b24"),
        _ => None,
    }
}

pub(crate) fn cf_edit_set_home_page(value: &Value, config_dir: &Path) -> Result<PathBuf, String> {
    let layout = cf_edit_json_object(value, "set-home-page value must be valid JSON object")?;
    if layout.is_empty() {
        return Err("set-home-page value must be non-empty object".to_string());
    }
    for key in layout.keys() {
        if !matches!(
            key.as_str(),
            "template" | "WorkingAreaTemplate" | "left" | "LeftColumn" | "right" | "RightColumn"
        ) {
            return Err(format!(
                "Unknown key '{key}'. Allowed: template, left, right"
            ));
        }
    }
    let template = cf_edit_object_field(&layout, &["template", "WorkingAreaTemplate"])
        .and_then(Value::as_str)
        .unwrap_or("TwoColumnsEqualWidth");
    if !matches!(
        template,
        "OneColumn" | "TwoColumnsEqualWidth" | "TwoColumnsVariableWidth"
    ) {
        return Err(format!(
            "Unknown template '{template}'. Allowed: OneColumn, TwoColumnsEqualWidth, TwoColumnsVariableWidth"
        ));
    }
    let left_items = cf_edit_object_field(&layout, &["left", "LeftColumn"]);
    let right_items = cf_edit_object_field(&layout, &["right", "RightColumn"]);
    if template == "OneColumn" && right_items.is_some_and(cf_edit_truthy_value) {
        return Err("Template 'OneColumn' cannot have items in 'right' column".to_string());
    }
    let left_xml = cf_edit_home_page_column_xml("LeftColumn", left_items)?;
    let right_xml = cf_edit_home_page_column_xml("RightColumn", right_items)?;
    let hp_xml = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\r\n\
         <HomePageWorkArea xmlns=\"http://v8.1c.ru/8.3/xcf/extrnprops\" \
         xmlns:xr=\"http://v8.1c.ru/8.3/xcf/readable\" \
         xmlns:xs=\"http://www.w3.org/2001/XMLSchema\" \
         xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\" version=\"2.17\">\r\n\
         \t<WorkingAreaTemplate>{template}</WorkingAreaTemplate>\r\n\
         {left_xml}\r\n\
         {right_xml}\r\n\
         </HomePageWorkArea>"
    );
    let ext_dir = config_dir.join("Ext");
    fs::create_dir_all(&ext_dir)
        .map_err(|err| format!("failed to create {}: {err}", ext_dir.display()))?;
    let path = ext_dir.join("HomePageWorkArea.xml");
    write_utf8_bom(&path, &hp_xml)?;
    Ok(path)
}

pub(crate) fn cf_edit_home_page_column_xml(
    tag: &str,
    items: Option<&Value>,
) -> Result<String, String> {
    let Some(items) = items.filter(|value| cf_edit_truthy_value(value)) else {
        return Ok(format!("\t<{tag}/>"));
    };
    let values = if let Some(array) = items.as_array() {
        if array.is_empty() {
            return Ok(format!("\t<{tag}/>"));
        }
        array.clone()
    } else {
        vec![items.clone()]
    };
    let blocks = values
        .iter()
        .map(|item| cf_edit_home_page_item_xml(item, "\t\t"))
        .collect::<Result<Vec<_>, _>>()?
        .join("\r\n");
    Ok(format!("\t<{tag}>\r\n{blocks}\r\n\t</{tag}>"))
}

pub(crate) fn cf_edit_home_page_item_xml(entry: &Value, indent: &str) -> Result<String, String> {
    let (form_ref, height, common, roles) = if let Some(form_ref) = entry.as_str() {
        (cf_edit_normalize_form_ref(form_ref), 10i64, true, None)
    } else if let Some(obj) = entry.as_object() {
        let form_raw = cf_edit_object_field(obj, &["form", "Form"])
            .and_then(Value::as_str)
            .ok_or_else(|| format!("Home page item: 'form' is required, got: {entry}"))?;
        let height = cf_edit_object_field(obj, &["height", "Height"])
            .and_then(cf_edit_i64_value)
            .unwrap_or(10);
        let common = cf_edit_object_field(obj, &["visibility", "Visibility"])
            .map(cf_edit_truthy_value)
            .unwrap_or(true);
        (
            cf_edit_normalize_form_ref(form_raw),
            height,
            common,
            obj.get("roles"),
        )
    } else {
        return Err(format!(
            "Home page item must be string or object, got: {entry}"
        ));
    };

    let mut vis_parts = vec![format!("{indent}\t\t<xr:Common>{common}</xr:Common>")];
    if let Some(roles) = roles.and_then(Value::as_object) {
        for (role_name, value) in roles {
            let role_name = if role_name.starts_with("Role.") || cf_edit_uuid_like(role_name) {
                role_name.clone()
            } else {
                format!("Role.{role_name}")
            };
            vis_parts.push(format!(
                "{indent}\t\t<xr:Value name=\"{}\">{}</xr:Value>",
                escape_xml(&role_name),
                cf_edit_truthy_value(value)
            ));
        }
    }
    let vis_block = vis_parts.join("\r\n");
    Ok(format!(
        "{indent}<Item>\r\n\
         {indent}\t<Form>{}</Form>\r\n\
         {indent}\t<Height>{height}</Height>\r\n\
         {indent}\t<Visibility>\r\n\
         {vis_block}\r\n\
         {indent}\t</Visibility>\r\n\
         {indent}</Item>",
        escape_xml(&form_ref)
    ))
}

pub(crate) fn cf_edit_normalize_form_ref(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.is_empty() || cf_edit_uuid_like(trimmed) {
        return trimmed.to_string();
    }
    if trimmed.contains('/') || trimmed.contains('\\') {
        let mut parts = trimmed
            .replace('\\', "/")
            .split('/')
            .filter(|part| !part.is_empty() && part.to_lowercase() != "ext")
            .map(ToOwned::to_owned)
            .collect::<Vec<_>>();
        if parts
            .last()
            .is_some_and(|part| part.to_lowercase() == "form.xml")
        {
            parts.pop();
        }
        if parts.len() >= 2 {
            if let Some(type_singular) = cf_edit_dir_to_type(&parts[0]) {
                if type_singular == "CommonForm" {
                    return format!("CommonForm.{}", parts[1]);
                }
                if parts.len() >= 4 && parts[2].eq_ignore_ascii_case("Forms") {
                    return format!("{}.{}.Form.{}", type_singular, parts[1], parts[3]);
                }
            }
        }
        return trimmed.to_string();
    }
    let mut parts = trimmed
        .split('.')
        .map(ToOwned::to_owned)
        .collect::<Vec<_>>();
    if let Some(head) = parts.first_mut() {
        if let Some(normalized) = cf_edit_ru_type(head) {
            *head = normalized.to_string();
        }
    }
    for part in parts.iter_mut().skip(1) {
        if part == "Форма" {
            *part = "Form".to_string();
        }
    }
    if parts.len() == 3
        && parts[0] != "CommonForm"
        && cf_validate_child_object_type_index(&parts[0]).is_some()
    {
        parts.insert(2, "Form".to_string());
    }
    parts.join(".")
}

pub(crate) fn cf_edit_ru_type(value: &str) -> Option<&'static str> {
    match value.to_lowercase().as_str() {
        "справочник" => Some("Catalog"),
        "документ" => Some("Document"),
        "перечисление" => Some("Enum"),
        "отчёт" | "отчет" => Some("Report"),
        "обработка" => Some("DataProcessor"),
        "общаяформа" => Some("CommonForm"),
        "журналдокументов" => Some("DocumentJournal"),
        "планвидовхарактеристик" => Some("ChartOfCharacteristicTypes"),
        "плансчетов" => Some("ChartOfAccounts"),
        "планвидоврасчета" | "планвидоврасчёта" => {
            Some("ChartOfCalculationTypes")
        }
        "регистрсведений" => Some("InformationRegister"),
        "регистрнакопления" => Some("AccumulationRegister"),
        "регистрбухгалтерии" => Some("AccountingRegister"),
        "регистррасчета" | "регистррасчёта" => {
            Some("CalculationRegister")
        }
        "бизнеспроцесс" => Some("BusinessProcess"),
        "задача" => Some("Task"),
        "планобмена" => Some("ExchangePlan"),
        "хранилищенастроек" => Some("SettingsStorage"),
        _ => None,
    }
}

pub(crate) fn cf_edit_dir_to_type(value: &str) -> Option<&'static str> {
    metadata_kind_by_directory(value).map(|kind| kind.tag)
}

pub(crate) fn cf_edit_json_object(
    value: &Value,
    string_parse_error: &str,
) -> Result<Map<String, Value>, String> {
    if let Value::String(text) = value {
        let parsed: Value =
            serde_json::from_str(text).map_err(|_| string_parse_error.to_string())?;
        return Ok(parsed
            .as_object()
            .ok_or_else(|| string_parse_error.to_string())?
            .clone());
    }
    value
        .as_object()
        .cloned()
        .ok_or_else(|| string_parse_error.to_string())
}

pub(crate) fn cf_edit_object_field<'a>(
    object: &'a Map<String, Value>,
    keys: &[&str],
) -> Option<&'a Value> {
    keys.iter().find_map(|key| object.get(*key))
}

pub(crate) fn cf_edit_truthy_value(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(value) => *value,
        Value::Number(value) => value.as_i64().is_some_and(|number| number != 0),
        Value::String(value) => !value.is_empty(),
        Value::Array(value) => !value.is_empty(),
        Value::Object(value) => !value.is_empty(),
    }
}

pub(crate) fn cf_edit_i64_value(value: &Value) -> Option<i64> {
    value
        .as_i64()
        .or_else(|| value.as_str().and_then(|value| value.parse::<i64>().ok()))
}

pub(crate) fn cf_edit_uuid_like(value: &str) -> bool {
    let parts = value.split('-').collect::<Vec<_>>();
    [8usize, 4, 4, 4, 12].iter().zip(parts.iter()).count() == 5
        && parts.len() == 5
        && parts
            .iter()
            .zip([8usize, 4, 4, 4, 12])
            .all(|(part, len)| part.len() == len && part.chars().all(|ch| ch.is_ascii_hexdigit()))
}

pub(crate) fn create_configuration_scaffold(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> AdapterOutcome {
    let name = string_arg(args, &["name", "Name"]).unwrap_or("");
    if name.is_empty() {
        return AdapterOutcome {
            ok: false,
            summary: "unica.cf.init failed in native XML scaffold writer".to_string(),
            changes: Vec::new(),
            warnings: Vec::new(),
            errors: vec!["missing required Name argument".to_string()],
            artifacts: Vec::new(),
            stdout: None,
            stderr: Some("missing required Name argument\n".to_string()),
            command: None,
        };
    }
    let synonym = string_arg(args, &["synonym", "Synonym"]).unwrap_or(name);
    let out_dir = output_dir_arg(args, context, &["outputDir", "OutputDir"], "src");
    let config = out_dir.join("Configuration.xml");
    let languages = out_dir.join("Languages");
    let language = languages.join("Русский.xml");
    let ext = out_dir.join("Ext");
    let cai = ext.join("ClientApplicationInterface.xml");

    let write_result = (|| -> Result<(), String> {
        if config.exists() {
            return Err(format!(
                "Configuration.xml already exists: {}",
                config.display()
            ));
        }

        let uuid_cfg = stable_uuid(0);
        let uuid_lang = stable_uuid(1);
        let contained_object_ids = (2..9).map(stable_uuid).collect::<Vec<_>>();
        let open_panel_inst = stable_uuid(9);
        let sections_panel_inst = stable_uuid(10);
        let compatibility = string_arg(args, &["compatibilityMode", "CompatibilityMode"])
            .unwrap_or("Version8_3_24");
        let vendor_xml = string_arg(args, &["vendor", "Vendor"])
            .map(escape_xml)
            .unwrap_or_default();
        let version_xml = string_arg(args, &["version", "Version"])
            .map(escape_xml)
            .unwrap_or_default();
        let synonym_xml = format!(
            "\r\n\t\t\t\t<v8:item>\r\n\t\t\t\t\t<v8:lang>ru</v8:lang>\r\n\t\t\t\t\t<v8:content>{}</v8:content>\r\n\t\t\t\t</v8:item>\r\n\t\t\t",
            escape_xml(synonym)
        );
        let mobile_xml = mobile_functionality_xml();
        let contained_objects = contained_objects_xml(&contained_object_ids);

        fs::create_dir_all(&languages)
            .map_err(|err| format!("failed to create {}: {err}", languages.display()))?;
        fs::create_dir_all(&ext)
            .map_err(|err| format!("failed to create {}: {err}", ext.display()))?;

        write_utf8_bom(
            &config,
            &format!(
                r#"<?xml version="1.0" encoding="UTF-8"?>
<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" xmlns:app="http://v8.1c.ru/8.2/managed-application/core" xmlns:cfg="http://v8.1c.ru/8.1/data/enterprise/current-config" xmlns:cmi="http://v8.1c.ru/8.2/managed-application/cmi" xmlns:ent="http://v8.1c.ru/8.1/data/enterprise" xmlns:lf="http://v8.1c.ru/8.2/managed-application/logform" xmlns:style="http://v8.1c.ru/8.1/data/ui/style" xmlns:sys="http://v8.1c.ru/8.1/data/ui/fonts/system" xmlns:v8="http://v8.1c.ru/8.1/data/core" xmlns:v8ui="http://v8.1c.ru/8.1/data/ui" xmlns:web="http://v8.1c.ru/8.1/data/ui/colors/web" xmlns:win="http://v8.1c.ru/8.1/data/ui/colors/windows" xmlns:xen="http://v8.1c.ru/8.3/xcf/enums" xmlns:xpr="http://v8.1c.ru/8.3/xcf/predef" xmlns:xr="http://v8.1c.ru/8.3/xcf/readable" xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" version="2.17">
	<Configuration uuid="{uuid_cfg}">
		<InternalInfo>
{contained_objects}		</InternalInfo>
		<Properties>
			<Name>{name}</Name>
			<Synonym>{synonym_xml}</Synonym>
			<Comment/>
			<NamePrefix/>
			<ConfigurationExtensionCompatibilityMode>{compatibility}</ConfigurationExtensionCompatibilityMode>
			<DefaultRunMode>ManagedApplication</DefaultRunMode>
			<UsePurposes>
				<v8:Value xsi:type="app:ApplicationUsePurpose">PlatformApplication</v8:Value>
			</UsePurposes>
			<ScriptVariant>Russian</ScriptVariant>
			<DefaultRoles/>
			<Vendor>{vendor_xml}</Vendor>
			<Version>{version_xml}</Version>
			<UpdateCatalogAddress/>
			<IncludeHelpInContents>false</IncludeHelpInContents>
			<UseManagedFormInOrdinaryApplication>false</UseManagedFormInOrdinaryApplication>
			<UseOrdinaryFormInManagedApplication>false</UseOrdinaryFormInManagedApplication>
			<AdditionalFullTextSearchDictionaries/>
			<CommonSettingsStorage/>
			<ReportsUserSettingsStorage/>
			<ReportsVariantsStorage/>
			<FormDataSettingsStorage/>
			<DynamicListsUserSettingsStorage/>
			<URLExternalDataStorage/>
			<Content/>
			<DefaultReportForm/>
			<DefaultReportVariantForm/>
			<DefaultReportSettingsForm/>
			<DefaultReportAppearanceTemplate/>
			<DefaultDynamicListSettingsForm/>
			<DefaultSearchForm/>
			<DefaultDataHistoryChangeHistoryForm/>
			<DefaultDataHistoryVersionDataForm/>
			<DefaultDataHistoryVersionDifferencesForm/>
			<DefaultCollaborationSystemUsersChoiceForm/>
			<RequiredMobileApplicationPermissions/>
			<UsedMobileApplicationFunctionalities>{mobile_xml}
			</UsedMobileApplicationFunctionalities>
			<StandaloneConfigurationRestrictionRoles/>
			<MobileApplicationURLs/>
			<AllowedIncomingShareRequestTypes/>
			<MainClientApplicationWindowMode>Normal</MainClientApplicationWindowMode>
			<DefaultInterface/>
			<DefaultStyle/>
			<DefaultLanguage>Language.Русский</DefaultLanguage>
			<BriefInformation/>
			<DetailedInformation/>
			<Copyright/>
			<VendorInformationAddress/>
			<ConfigurationInformationAddress/>
			<DataLockControlMode>Managed</DataLockControlMode>
			<ObjectAutonumerationMode>NotAutoFree</ObjectAutonumerationMode>
			<ModalityUseMode>DontUse</ModalityUseMode>
			<SynchronousPlatformExtensionAndAddInCallUseMode>DontUse</SynchronousPlatformExtensionAndAddInCallUseMode>
			<InterfaceCompatibilityMode>TaxiEnableVersion8_2</InterfaceCompatibilityMode>
			<DatabaseTablespacesUseMode>DontUse</DatabaseTablespacesUseMode>
			<CompatibilityMode>{compatibility}</CompatibilityMode>
			<DefaultConstantsForm/>
		</Properties>
		<ChildObjects>
			<Language>Русский</Language>
		</ChildObjects>
	</Configuration>
</MetaDataObject>"#,
                name = escape_xml(name),
            ),
        )?;
        write_utf8_bom(
            &language,
            &format!(
                r#"<?xml version="1.0" encoding="UTF-8"?>
<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" xmlns:app="http://v8.1c.ru/8.2/managed-application/core" xmlns:cfg="http://v8.1c.ru/8.1/data/enterprise/current-config" xmlns:cmi="http://v8.1c.ru/8.2/managed-application/cmi" xmlns:ent="http://v8.1c.ru/8.1/data/enterprise" xmlns:lf="http://v8.1c.ru/8.2/managed-application/logform" xmlns:style="http://v8.1c.ru/8.1/data/ui/style" xmlns:sys="http://v8.1c.ru/8.1/data/ui/fonts/system" xmlns:v8="http://v8.1c.ru/8.1/data/core" xmlns:v8ui="http://v8.1c.ru/8.1/data/ui" xmlns:web="http://v8.1c.ru/8.1/data/ui/colors/web" xmlns:win="http://v8.1c.ru/8.1/data/ui/colors/windows" xmlns:xen="http://v8.1c.ru/8.3/xcf/enums" xmlns:xpr="http://v8.1c.ru/8.3/xcf/predef" xmlns:xr="http://v8.1c.ru/8.3/xcf/readable" xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" version="2.17">
	<Language uuid="{uuid_lang}">
		<Properties>
			<Name>Русский</Name>
			<Synonym>
				<v8:item>
					<v8:lang>ru</v8:lang>
					<v8:content>Русский</v8:content>
				</v8:item>
			</Synonym>
			<Comment/>
			<LanguageCode>ru</LanguageCode>
		</Properties>
	</Language>
</MetaDataObject>"#
            ),
        )?;
        write_utf8_bom(
            &cai,
            &format!(
                r#"<?xml version="1.0" encoding="UTF-8"?>
<ClientApplicationInterface xmlns="http://v8.1c.ru/8.2/managed-application/core" xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" xsi:type="InterfaceLayouter">
	<top>
		<panel id="{open_panel_inst}">
			<uuid>cbab57f2-a0f3-4f0a-89ea-4cb19570ab75</uuid>
		</panel>
	</top>
	<left>
		<panel id="{sections_panel_inst}">
			<uuid>b553047f-c9aa-4157-978d-448ecad24248</uuid>
		</panel>
	</left>
	<panelDef id="b553047f-c9aa-4157-978d-448ecad24248"/>
	<panelDef id="13322b22-3960-4d68-93a6-fe2dd7f28ca3"/>
	<panelDef id="c933ac92-92cd-459d-81cc-e0c8a83ced99"/>
	<panelDef id="cbab57f2-a0f3-4f0a-89ea-4cb19570ab75"/>
	<panelDef id="b2735bd3-d822-4430-ba59-c9e869693b24"/>
</ClientApplicationInterface>"#
            ),
        )?;
        Ok(())
    })();

    match write_result {
        Ok(()) => AdapterOutcome {
            ok: true,
            summary: "unica.cf.init completed with native XML scaffold writer".to_string(),
            changes: vec![
                format!("created {}", config.display()),
                format!("created {}", language.display()),
                format!("created {}", cai.display()),
            ],
            warnings: Vec::new(),
            errors: Vec::new(),
            artifacts: vec![
                config.display().to_string(),
                language.display().to_string(),
                cai.display().to_string(),
            ],
            stdout: Some(format!(
                "[OK] Создана конфигурация: {name}\n     Каталог:            {}\n     Configuration.xml:  {}\n     Languages:          {}\n     Ext/CAI:            {}\n",
                out_dir.display(),
                config.display(),
                language.display(),
                cai.display()
            )),
            stderr: None,
            command: None,
        },
        Err(error) => AdapterOutcome {
            ok: false,
            summary: "unica.cf.init failed in native XML scaffold writer".to_string(),
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

pub(crate) fn mobile_functionality_xml() -> String {
    const MOBILE_FUNCTIONS: &[(&str, &str)] = &[
        ("Biometrics", "true"),
        ("Location", "false"),
        ("BackgroundLocation", "false"),
        ("BluetoothPrinters", "false"),
        ("WiFiPrinters", "false"),
        ("Contacts", "false"),
        ("Calendars", "false"),
        ("PushNotifications", "false"),
        ("LocalNotifications", "false"),
        ("InAppPurchases", "false"),
        ("PersonalComputerFileExchange", "false"),
        ("Ads", "false"),
        ("NumberDialing", "false"),
        ("CallProcessing", "false"),
        ("CallLog", "false"),
        ("AutoSendSMS", "false"),
        ("ReceiveSMS", "false"),
        ("SMSLog", "false"),
        ("Camera", "false"),
        ("Microphone", "false"),
        ("MusicLibrary", "false"),
        ("PictureAndVideoLibraries", "false"),
        ("AudioPlaybackAndVibration", "false"),
        ("BackgroundAudioPlaybackAndVibration", "false"),
        ("InstallPackages", "false"),
        ("OSBackup", "true"),
        ("ApplicationUsageStatistics", "false"),
        ("BarcodeScanning", "false"),
        ("BackgroundAudioRecording", "false"),
        ("AllFilesAccess", "false"),
        ("Videoconferences", "false"),
        ("NFC", "false"),
        ("DocumentScanning", "false"),
        ("SpeechToText", "false"),
        ("Geofences", "false"),
        ("IncomingShareRequests", "false"),
        ("AllIncomingShareRequestsTypesProcessing", "false"),
    ];

    let mut xml = String::new();
    for (name, enabled) in MOBILE_FUNCTIONS {
        xml.push_str(&format!(
            "\r\n\t\t\t\t<app:functionality>\r\n\t\t\t\t\t<app:functionality>{name}</app:functionality>\r\n\t\t\t\t\t<app:use>{enabled}</app:use>\r\n\t\t\t\t</app:functionality>"
        ));
    }
    xml
}

pub(crate) fn contained_objects_xml(object_ids: &[String]) -> String {
    const CLASS_IDS: &[&str] = &[
        "9cd510cd-abfc-11d4-9434-004095e12fc7",
        "9fcd25a0-4822-11d4-9414-008048da11f9",
        "e3687481-0a87-462c-a166-9f34594f9bba",
        "9de14907-ec23-4a07-96f0-85521cb6b53b",
        "51f2d5d8-ea4d-4064-8892-82951750031e",
        "e68182ea-4237-4383-967f-90c1e3370bc7",
        "fb282519-d103-4dd3-bc12-cb271d631dfc",
    ];

    let mut xml = String::new();
    for (class_id, object_id) in CLASS_IDS.iter().zip(object_ids) {
        xml.push_str(&format!(
            "\t\t\t<xr:ContainedObject>\n\t\t\t\t<xr:ClassId>{class_id}</xr:ClassId>\n\t\t\t\t<xr:ObjectId>{object_id}</xr:ObjectId>\n\t\t\t</xr:ContainedObject>\n"
        ));
    }
    xml
}

pub(crate) fn invoke_read(
    operation: &str,
    _tool_name: &str,
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> Option<Result<AdapterOutcome, String>> {
    match operation {
        "cf-info" => Some(Ok(analyze_cf_info(args, context))),
        "cf-validate" => Some(Ok(validate_cf(args, context))),
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
        "cf-init" => Some(create_configuration_scaffold(args, context)),
        "cf-edit" => Some(edit_cf(args, context)),
        _ => None,
    }
}
