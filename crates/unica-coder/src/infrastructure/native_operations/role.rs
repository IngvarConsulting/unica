#![allow(dead_code, unused_imports)]

use crate::application::AdapterOutcome;
use crate::domain::workspace::WorkspaceContext;
use roxmltree::Document;
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use super::common::*;
use super::compile_transaction::{CompileTransaction, RegistrationStatus};
use super::{
    cf::*, cfe::*, dcs::*, form::*, interface::*, meta::*, mxl::*, subsystem::*, template::*,
};
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

pub(crate) fn analyze_role_info(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> AdapterOutcome {
    let result = (|| -> Result<(String, Option<PathBuf>, PathBuf), String> {
        let rights_path_raw = required_path(
            args,
            &["rightsPath", "RightsPath", "path", "Path"],
            "RightsPath",
        )?;
        let rights_path = absolutize(rights_path_raw, &context.cwd);
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

        let mut lines = Vec::<String>::new();
        let mut header = format!("=== Role: {role_name}");
        if !role_synonym.is_empty() {
            header.push_str(&format!(" --- \"{role_synonym}\""));
        }
        header.push_str(" ===");
        lines.push(header);
        lines.push(format!(
            "Поддержка: {}",
            support_status_for_path(&rights_path)
        ));
        lines.push(String::new());
        lines.push(format!(
            "Properties: setForNewObjects={set_for_new}, setForAttributesByDefault={set_for_attrs}, independentRightsOfChildObjects={independent_child}"
        ));
        lines.push(String::new());

        if !allowed.is_empty() {
            lines.push("Allowed rights:".to_string());
            lines.push(String::new());
            for group in &allowed {
                lines.push(format!(
                    "  {} ({}):",
                    group.type_prefix,
                    group.objects.len()
                ));
                append_role_info_group(&mut lines, &group.objects, false);
                lines.push(String::new());
            }
        } else {
            lines.push("(no allowed rights)".to_string());
            lines.push(String::new());
        }

        let show_denied = bool_arg(args, &["showDenied", "ShowDenied"]);
        if show_denied && !denied.is_empty() {
            lines.push("Denied rights:".to_string());
            lines.push(String::new());
            for group in &denied {
                lines.push(format!(
                    "  {} ({}):",
                    group.type_prefix,
                    group.objects.len()
                ));
                append_role_info_group(&mut lines, &group.objects, true);
                lines.push(String::new());
            }
        } else if total_denied > 0 {
            lines.push(format!(
                "Denied: {total_denied} rights (use -ShowDenied to list)"
            ));
            lines.push(String::new());
        }

        if !rls_objects.is_empty() {
            lines.push(format!("RLS: {} restrictions", rls_objects.len()));
        }
        if !templates.is_empty() {
            lines.push(format!("Templates: {}", templates.join(", ")));
        }

        lines.push(String::new());
        lines.push("---".to_string());
        lines.push(format!(
            "Total: {total_allowed} allowed, {total_denied} denied"
        ));

        let total_lines = lines.len();
        let offset = int_arg(args, &["offset", "Offset"]).unwrap_or(0);
        let limit = int_arg(args, &["limit", "Limit"]).unwrap_or(150);

        let mut out_lines = lines;
        if offset > 0 {
            if offset as usize >= total_lines {
                return Ok((
                    format!(
                        "[INFO] Offset {offset} exceeds total lines ({total_lines}). Nothing to show.\n"
                    ),
                    None,
                    rights_path,
                ));
            }
            out_lines = out_lines[offset as usize..].to_vec();
        }

        if limit > 0 && out_lines.len() > limit as usize {
            let mut shown = out_lines[..limit as usize].to_vec();
            shown.push(String::new());
            shown.push(format!(
                "[TRUNCATED] Shown {limit} of {total_lines} lines. Use -Offset {} to continue.",
                offset + limit
            ));
            out_lines = shown;
        }

        if let Some(out_file) = path_arg(args, &["outFile", "OutFile"]) {
            let out_file = absolutize(out_file, &context.cwd);
            write_utf8_bom(&out_file, &out_lines.join("\n"))?;
            Ok((
                format!("Output written to {}\n", out_file.display()),
                Some(out_file),
                rights_path,
            ))
        } else {
            Ok((format!("{}\n", out_lines.join("\n")), None, rights_path))
        }
    })();

    match result {
        Ok((stdout, out_file, rights_path)) => {
            let mut artifacts = vec![rights_path.display().to_string()];
            if let Some(out_file) = out_file {
                artifacts.push(out_file.display().to_string());
            }
            AdapterOutcome {
                ok: true,
                summary: "unica.role.info completed with native role analyzer".to_string(),
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
            summary: "unica.role.info failed in native role analyzer".to_string(),
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

pub(crate) fn role_info_metadata(rights_path: &Path) -> (String, String) {
    let ext_dir = rights_path.parent().unwrap_or_else(|| Path::new(""));
    let role_dir = ext_dir.parent().unwrap_or_else(|| Path::new(""));
    let roles_dir = role_dir.parent().unwrap_or_else(|| Path::new(""));
    let role_folder_name = role_dir
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("")
        .to_string();
    let meta_path = roles_dir.join(format!("{role_folder_name}.xml"));

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
    let result = (|| -> Result<(bool, String, PathBuf, Option<PathBuf>, String), String> {
        let rights_path_raw = required_path(
            args,
            &["rightsPath", "RightsPath", "path", "Path"],
            "RightsPath",
        )?;
        let rights_path =
            resolve_role_validate_rights_path(absolutize(rights_path_raw, &context.cwd));
        let out_file =
            path_arg(args, &["outFile", "OutFile"]).map(|path| absolutize(path, &context.cwd));
        let detailed = bool_arg(args, &["detailed", "Detailed"]);

        let ext_dir = rights_path.parent().unwrap_or_else(|| Path::new(""));
        let role_dir = ext_dir.parent().unwrap_or_else(|| Path::new(""));
        let roles_dir = role_dir.parent().unwrap_or_else(|| Path::new(""));
        let role_dir_name = role_dir
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or("")
            .to_string();
        let metadata_path = roles_dir.join(format!("{role_dir_name}.xml"));

        let mut report = RoleValidationReport::new(detailed);
        if !rights_path.exists() {
            report.error(format!("File not found: {}", rights_path.display()));
            let text = report.lines.join("\n");
            return Ok((false, text, rights_path, out_file, String::new()));
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
                return Ok((false, text, rights_path, out_file, String::new()));
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
                    let valid = if obj_name.contains(".Command.") {
                        &["View"][..]
                    } else if obj_name.contains(".IntegrationServiceChannel.") {
                        &["Use"][..]
                    } else {
                        &["View", "Edit"][..]
                    };
                    if !valid.contains(&right_name) {
                        if obj_name.contains(".Command.") {
                            report.warn(format!(
                                "{obj_name}: '{right_name}' not valid for commands (only: View)"
                            ));
                        } else if obj_name.contains(".IntegrationServiceChannel.") {
                            report.warn(format!(
                                "{obj_name}: '{right_name}' not valid for channels (only: Use)"
                            ));
                        } else {
                            report.warn(format!(
                                "{obj_name}: '{right_name}' not valid for nested objects (only: View, Edit)"
                            ));
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

        let config_dir = roles_dir.parent().unwrap_or_else(|| Path::new(""));
        let config_xml_path = config_dir.join("Configuration.xml");
        if inferred_role_name.is_empty() {
            inferred_role_name = role_dir
                .file_name()
                .and_then(|value| value.to_str())
                .unwrap_or("")
                .to_string();
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
        Ok((ok, text, rights_path, out_file, String::new()))
    })();

    match result {
        Ok((ok, text, rights_path, out_file, error_slot)) => {
            let stdout = if let Some(out_file) = &out_file {
                if let Some(parent) = out_file.parent() {
                    if let Err(err) = fs::create_dir_all(parent) {
                        return AdapterOutcome {
                            ok: false,
                            summary: "unica.role.validate failed in native role validator"
                                .to_string(),
                            changes: Vec::new(),
                            warnings: Vec::new(),
                            errors: vec![format!("failed to create {}: {err}", parent.display())],
                            artifacts: Vec::new(),
                            stdout: None,
                            stderr: None,
                            command: None,
                        };
                    }
                }
                if let Err(error) = write_utf8_bom(out_file, &text) {
                    return AdapterOutcome {
                        ok: false,
                        summary: "unica.role.validate failed in native role validator".to_string(),
                        changes: Vec::new(),
                        warnings: Vec::new(),
                        errors: vec![error.clone()],
                        artifacts: Vec::new(),
                        stdout: None,
                        stderr: Some(format!("{error}\n")),
                        command: None,
                    };
                }
                format!("Written to: {}\n", out_file.display())
            } else {
                format!("{text}\n")
            };

            let mut artifacts = vec![rights_path.display().to_string()];
            if let Some(out_file) = out_file {
                artifacts.push(out_file.display().to_string());
            }
            AdapterOutcome {
                ok,
                summary: if ok {
                    "unica.role.validate completed with native role validator".to_string()
                } else {
                    "unica.role.validate failed in native role validator".to_string()
                },
                changes: Vec::new(),
                warnings: Vec::new(),
                errors: if ok { Vec::new() } else { vec![error_slot] },
                artifacts,
                stdout: Some(stdout),
                stderr: Some(String::new()),
                command: None,
            }
        }
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
    match object_type {
        "Configuration" => &[
            "Administration",
            "DataAdministration",
            "UpdateDataBaseConfiguration",
            "ConfigurationExtensionsAdministration",
            "ActiveUsers",
            "EventLog",
            "ExclusiveMode",
            "ThinClient",
            "ThickClient",
            "WebClient",
            "MobileClient",
            "ExternalConnection",
            "Automation",
            "Output",
            "SaveUserData",
            "TechnicalSpecialistMode",
            "InteractiveOpenExtDataProcessors",
            "InteractiveOpenExtReports",
            "AnalyticsSystemClient",
            "CollaborationSystemInfoBaseRegistration",
            "MainWindowModeNormal",
            "MainWindowModeWorkplace",
            "MainWindowModeEmbeddedWorkplace",
            "MainWindowModeFullscreenWorkplace",
            "MainWindowModeKiosk",
        ],
        "Catalog" => &[
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
            "InteractiveDelete",
            "InteractiveDeleteMarked",
            "InteractiveDeletePredefinedData",
            "InteractiveSetDeletionMarkPredefinedData",
            "InteractiveClearDeletionMarkPredefinedData",
            "InteractiveDeleteMarkedPredefinedData",
            "ReadDataHistory",
            "ViewDataHistory",
            "UpdateDataHistory",
            "UpdateDataHistoryOfMissingData",
            "ReadDataHistoryOfMissingData",
            "UpdateDataHistorySettings",
            "UpdateDataHistoryVersionComment",
            "EditDataHistoryVersionComment",
            "SwitchToDataHistoryVersion",
        ],
        "Document" => &[
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
            "InteractiveDelete",
            "InteractiveDeleteMarked",
            "InteractivePosting",
            "InteractivePostingRegular",
            "InteractiveUndoPosting",
            "InteractiveChangeOfPosted",
            "ReadDataHistory",
            "ViewDataHistory",
            "UpdateDataHistory",
            "UpdateDataHistoryOfMissingData",
            "ReadDataHistoryOfMissingData",
            "UpdateDataHistorySettings",
            "UpdateDataHistoryVersionComment",
            "EditDataHistoryVersionComment",
            "SwitchToDataHistoryVersion",
        ],
        "InformationRegister" => &[
            "Read",
            "Update",
            "View",
            "Edit",
            "TotalsControl",
            "ReadDataHistory",
            "ViewDataHistory",
            "UpdateDataHistory",
            "UpdateDataHistoryOfMissingData",
            "ReadDataHistoryOfMissingData",
            "UpdateDataHistorySettings",
            "UpdateDataHistoryVersionComment",
            "EditDataHistoryVersionComment",
            "SwitchToDataHistoryVersion",
        ],
        "AccumulationRegister" | "AccountingRegister" => {
            &["Read", "Update", "View", "Edit", "TotalsControl"]
        }
        "CalculationRegister" => &["Read", "View"],
        "Constant" => &[
            "Read",
            "Update",
            "View",
            "Edit",
            "ReadDataHistory",
            "ViewDataHistory",
            "UpdateDataHistory",
            "UpdateDataHistorySettings",
            "UpdateDataHistoryVersionComment",
            "EditDataHistoryVersionComment",
            "SwitchToDataHistoryVersion",
        ],
        "ChartOfAccounts" => &[
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
            "InteractiveDelete",
            "InteractiveDeletePredefinedData",
            "InteractiveSetDeletionMarkPredefinedData",
            "InteractiveClearDeletionMarkPredefinedData",
            "InteractiveDeleteMarkedPredefinedData",
            "ReadDataHistory",
            "ReadDataHistoryOfMissingData",
            "UpdateDataHistory",
            "UpdateDataHistoryOfMissingData",
            "UpdateDataHistorySettings",
            "UpdateDataHistoryVersionComment",
        ],
        "ChartOfCharacteristicTypes" => &[
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
            "InteractiveDelete",
            "InteractiveDeleteMarked",
            "InteractiveDeletePredefinedData",
            "InteractiveSetDeletionMarkPredefinedData",
            "InteractiveClearDeletionMarkPredefinedData",
            "InteractiveDeleteMarkedPredefinedData",
            "ReadDataHistory",
            "ViewDataHistory",
            "UpdateDataHistory",
            "ReadDataHistoryOfMissingData",
            "UpdateDataHistoryOfMissingData",
            "UpdateDataHistorySettings",
            "UpdateDataHistoryVersionComment",
            "EditDataHistoryVersionComment",
            "SwitchToDataHistoryVersion",
        ],
        "ChartOfCalculationTypes" => &[
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
            "InteractiveDelete",
            "InteractiveDeletePredefinedData",
            "InteractiveSetDeletionMarkPredefinedData",
            "InteractiveClearDeletionMarkPredefinedData",
            "InteractiveDeleteMarkedPredefinedData",
        ],
        "ExchangePlan" => &[
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
            "InteractiveDelete",
            "InteractiveDeleteMarked",
            "ReadDataHistory",
            "ViewDataHistory",
            "UpdateDataHistory",
            "ReadDataHistoryOfMissingData",
            "UpdateDataHistoryOfMissingData",
            "UpdateDataHistorySettings",
            "UpdateDataHistoryVersionComment",
            "EditDataHistoryVersionComment",
            "SwitchToDataHistoryVersion",
        ],
        "BusinessProcess" => &[
            "Read",
            "Insert",
            "Update",
            "Delete",
            "View",
            "Edit",
            "InputByString",
            "Start",
            "InteractiveInsert",
            "InteractiveSetDeletionMark",
            "InteractiveClearDeletionMark",
            "InteractiveDelete",
            "InteractiveActivate",
            "InteractiveStart",
        ],
        "Task" => &[
            "Read",
            "Insert",
            "Update",
            "Delete",
            "View",
            "Edit",
            "InputByString",
            "Execute",
            "InteractiveInsert",
            "InteractiveSetDeletionMark",
            "InteractiveClearDeletionMark",
            "InteractiveDelete",
            "InteractiveActivate",
            "InteractiveExecute",
        ],
        "DataProcessor" | "Report" => &["Use", "View"],
        "CommonForm" | "CommonCommand" | "Subsystem" | "FilterCriterion" => &["View"],
        "DocumentJournal" => &["Read", "View"],
        "Sequence" => &["Read", "Update"],
        "WebService" | "HTTPService" | "IntegrationService" => &["Use"],
        "SessionParameter" => &["Get", "Set"],
        "CommonAttribute" => &["View", "Edit"],
        _ => &[],
    }
}

struct RoleCompileResult {
    stdout: String,
    stderr: String,
    artifacts: Vec<PathBuf>,
    changes: Vec<String>,
    warnings: Vec<String>,
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
        let mut transaction = CompileTransaction::new();
        let json_path_raw = required_path(args, &["jsonPath", "JsonPath"], "JsonPath")?;
        let output_dir_raw = required_path(args, &["outputDir", "OutputDir"], "OutputDir")?;
        let json_path = absolutize(json_path_raw, &context.cwd);
        if !json_path.exists() {
            return Err(format!("File not found: {}", json_path.display()));
        }
        let json_text = fs::read_to_string(&json_path)
            .map_err(|err| format!("failed to read {}: {err}", json_path.display()))?;
        let mut defn: Value = serde_json::from_str(json_text.trim_start_matches('\u{feff}'))
            .map_err(|err| format!("failed to parse role JSON: {err}"))?;

        let role_name = json_string_field(&defn, "name")
            .filter(|value| !value.is_empty())
            .ok_or_else(|| "JSON must have 'name' field (role programmatic name)".to_string())?;
        let synonym = json_string_field(&defn, "synonym").unwrap_or_else(|| role_name.clone());
        let comment = json_string_field(&defn, "comment").unwrap_or_default();

        if !truthy_json_field(&defn, "objects") && truthy_json_field(&defn, "rights") {
            let rights = defn.get("rights").cloned().unwrap_or(Value::Null);
            if let Some(object) = defn.as_object_mut() {
                object.insert("objects".to_string(), rights);
            }
        }

        let output_dir = absolutize(output_dir_raw.clone(), &context.cwd);
        let format_version = detect_format_version(&output_dir);
        let mut stderr = String::new();
        let mut parsed_objects = Vec::<RoleObject>::new();
        if let Some(objects) = defn.get("objects").and_then(Value::as_array) {
            for entry in objects {
                if let Some(parsed) = parse_role_object_entry(entry, &mut stderr) {
                    parsed_objects.push(parsed);
                }
            }
        }

        let sfno = defn
            .get("setForNewObjects")
            .map(json_value_to_python_lower)
            .unwrap_or_else(|| "false".to_string());
        let sfab = defn
            .get("setForAttributesByDefault")
            .map(json_value_to_python_lower)
            .unwrap_or_else(|| "true".to_string());
        let irco = defn
            .get("independentRightsOfChildObjects")
            .map(json_value_to_python_lower)
            .unwrap_or_else(|| "false".to_string());

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
            rights_lines.push(format!("        <name>{}</name>", object.name));
            for right in &object.rights {
                rights_lines.push("        <right>".to_string());
                rights_lines.push(format!("            <name>{}</name>", right.name));
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
        let uid = fresh_meta_compile_uuid();
        let metadata_xml = role_metadata_xml(&role_name, &synonym, &comment, &format_version, &uid);
        transaction.create_utf8_bom_text(&metadata_path, &metadata_xml)?;
        transaction.create_utf8_bom_text(&rights_path, &rights_xml)?;

        let config_xml_path = config_dir.join("Configuration.xml");
        let reg_result =
            transaction.register_canonical_child(&config_xml_path, "Role", &role_name)?;

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
            (
                Vec::new(),
                transaction.dry_run_changes(),
                Vec::new(),
                transaction.dry_run_stdout(),
            )
        } else {
            let report = transaction.commit()?;
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
    lines.push(format!("            <Name>{role_name}</Name>"));
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
        "role-info" => Some(Ok(analyze_role_info(args, context))),
        "role-validate" => Some(Ok(validate_role(args, context))),
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
        "role-compile" => Some(compile_role(args, context)),
        _ => None,
    }
}
