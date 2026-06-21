#![allow(dead_code, unused_imports)]

use crate::domain::workspace::WorkspaceContext;
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
    cf::*, cfe::*, form::*, interface::*, mxl::*, role::*, skd::*, subsystem::*, template::*,
};
#[derive(Clone)]
pub(crate) struct MetaInfoAttr<'a, 'input> {
    pub(crate) name: String,
    pub(crate) type_name: String,
    pub(crate) flags: String,
    pub(crate) _marker: std::marker::PhantomData<roxmltree::Node<'a, 'input>>,
}

pub(crate) struct MetaInfoTabularSection<'a, 'input> {
    pub(crate) name: String,
    pub(crate) columns: Vec<MetaInfoAttr<'a, 'input>>,
}

pub(crate) struct MetaInfoHttpMethod {
    pub(crate) http_method: String,
    pub(crate) handler: String,
}

pub(crate) struct MetaInfoHttpEndpoint {
    pub(crate) name: String,
    pub(crate) template: String,
    pub(crate) methods: Vec<MetaInfoHttpMethod>,
}

pub(crate) struct MetaInfoWsOperation {
    pub(crate) name: String,
    pub(crate) params: String,
    pub(crate) return_type: String,
    pub(crate) proc_name: String,
}

pub(crate) struct MetaValidationReporter {
    pub(crate) errors: usize,
    pub(crate) warnings: usize,
    pub(crate) ok_count: usize,
    pub(crate) stopped: bool,
    pub(crate) max_errors: usize,
    pub(crate) detailed: bool,
    pub(crate) lines: Vec<String>,
    pub(crate) md_type: String,
    pub(crate) obj_name: String,
}

pub(crate) struct MetaValidationRun {
    pub(crate) ok: bool,
    pub(crate) stdout: String,
    pub(crate) out_file: Option<PathBuf>,
    pub(crate) artifact: PathBuf,
    pub(crate) errors: Vec<String>,
}

impl MetaValidationReporter {
    pub(crate) fn new(max_errors: usize, detailed: bool) -> Self {
        Self {
            errors: 0,
            warnings: 0,
            ok_count: 0,
            stopped: false,
            max_errors,
            detailed,
            lines: vec![String::new()],
            md_type: "(unknown)".to_string(),
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
                    "=== Validation OK: {}.{} ({checks} checks) ===",
                    self.md_type, self.obj_name
                ),
                Vec::new(),
            );
        }
        self.lines.insert(
            0,
            format!("=== Validation: {}.{} ===", self.md_type, self.obj_name),
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
        (ok, self.lines.join("\n"), errors)
    }
}

pub(crate) fn validate_meta(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> AdapterOutcome {
    const MD_NS: &str = "http://v8.1c.ru/8.3/MDClasses";

    let result = (|| -> Result<MetaValidationRun, String> {
        let raw_path = required_path(
            args,
            &["objectPath", "ObjectPath", "path", "Path"],
            "ObjectPath",
        )?;
        let raw_path_text = raw_path.to_string_lossy();
        if raw_path_text.contains('|') {
            return Err(
                "[ERROR] Batch validation is not implemented in native meta.validate yet"
                    .to_string(),
            );
        }
        let object_path = resolve_meta_info_path(absolutize(raw_path, &context.cwd))?;
        let resolved_path = object_path
            .canonicalize()
            .unwrap_or_else(|_| object_path.clone());
        let config_dir = meta_validate_config_dir(&resolved_path);
        let out_file_label = string_arg(args, &["outFile", "OutFile"]).map(ToOwned::to_owned);
        let out_file = out_file_label
            .as_ref()
            .map(|path| absolutize(PathBuf::from(path), &context.cwd));
        let detailed = bool_arg(args, &["detailed", "Detailed"]);
        let max_errors = int_arg(args, &["maxErrors", "MaxErrors"])
            .and_then(|value| usize::try_from(value).ok())
            .filter(|value| *value > 0)
            .unwrap_or(30);

        let text = read_utf8_sig(&resolved_path)?;
        let doc = match Document::parse(text.trim_start_matches('\u{feff}')) {
            Ok(doc) => doc,
            Err(err) => {
                let mut report = MetaValidationReporter::new(max_errors, detailed);
                report.md_type = "(parse failed)".to_string();
                report.obj_name.clear();
                report.error(format!("1. XML parse failed: {err}"));
                return meta_validate_finish(report, out_file, out_file_label, resolved_path);
            }
        };

        let root = doc.root_element();
        let mut report = MetaValidationReporter::new(max_errors, detailed);
        let mut check1_ok = true;

        if root.tag_name().name() != "MetaDataObject" {
            report.error(format!(
                "1. Root element is '{}', expected 'MetaDataObject'",
                root.tag_name().name()
            ));
            return meta_validate_finish(report, out_file, out_file_label, resolved_path);
        }

        let root_ns = root.tag_name().namespace().unwrap_or("");
        if root_ns != MD_NS {
            report.error(format!(
                "1. Root namespace is '{root_ns}', expected '{MD_NS}'"
            ));
            check1_ok = false;
        }

        let version = root.attribute("version").unwrap_or("");
        if version.is_empty() {
            report.warn("1. Missing version attribute on MetaDataObject");
        } else if !matches!(version, "2.17" | "2.20") {
            report.warn(format!(
                "1. Unusual version '{version}' (expected 2.17 or 2.20)"
            ));
        }

        let child_elements = root
            .children()
            .filter(|child| child.is_element() && child.tag_name().namespace() == Some(MD_NS))
            .collect::<Vec<_>>();
        if child_elements.is_empty() {
            report.error("1. No metadata type element found inside MetaDataObject");
            return meta_validate_finish(report, out_file, out_file_label, resolved_path);
        }
        if child_elements.len() > 1 {
            let names = child_elements
                .iter()
                .map(|child| format!("'{}'", child.tag_name().name()))
                .collect::<Vec<_>>();
            report.error(format!(
                "1. Multiple type elements found: [{}]",
                names.join(", ")
            ));
            check1_ok = false;
        }

        let type_node = child_elements[0];
        let md_type = type_node.tag_name().name();
        report.md_type = md_type.to_string();
        if !meta_validate_valid_types().contains(&md_type) {
            report.error(format!("1. Unrecognized metadata type: {md_type}"));
            return meta_validate_finish(report, out_file, out_file_label, resolved_path);
        }

        let type_uuid = type_node.attribute("uuid").unwrap_or("");
        if type_uuid.is_empty() {
            report.error(format!("1. Missing uuid on <{md_type}> element"));
            check1_ok = false;
        } else if !is_guid(type_uuid) {
            report.error(format!("1. Invalid uuid '{type_uuid}' on <{md_type}>"));
            check1_ok = false;
        }

        let props_node = meta_info_child(type_node, "Properties");
        let name_node = props_node.and_then(|props| meta_info_child(props, "Name"));
        let obj_name = name_node
            .map(meta_info_inner_text)
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| "(unknown)".to_string());
        report.obj_name = obj_name.clone();

        if check1_ok {
            report.ok(format!(
                "1. Root structure: MetaDataObject/{md_type}, version {version}"
            ));
        }
        if report.stopped {
            return meta_validate_finish(report, out_file, out_file_label, resolved_path);
        }

        meta_validate_check_internal_info(&mut report, md_type, type_node, &obj_name);
        if report.stopped {
            return meta_validate_finish(report, out_file, out_file_label, resolved_path);
        }
        meta_validate_check_properties(&mut report, props_node, name_node, &obj_name);
        if report.stopped {
            return meta_validate_finish(report, out_file, out_file_label, resolved_path);
        }
        meta_validate_check_property_values(&mut report, props_node);
        if report.stopped {
            return meta_validate_finish(report, out_file, out_file_label, resolved_path);
        }
        meta_validate_check_standard_attributes(&mut report, md_type, props_node);
        if report.stopped {
            return meta_validate_finish(report, out_file, out_file_label, resolved_path);
        }

        let child_obj_node = meta_info_child(type_node, "ChildObjects");
        meta_validate_check_child_objects(&mut report, md_type, child_obj_node);
        if report.stopped {
            return meta_validate_finish(report, out_file, out_file_label, resolved_path);
        }
        meta_validate_check_child_elements(&mut report, child_obj_node);
        if report.stopped {
            return meta_validate_finish(report, out_file, out_file_label, resolved_path);
        }
        meta_validate_check_reserved_attr_names(&mut report, child_obj_node);
        if report.stopped {
            return meta_validate_finish(report, out_file, out_file_label, resolved_path);
        }
        meta_validate_check_uniqueness(&mut report, child_obj_node);
        if report.stopped {
            return meta_validate_finish(report, out_file, out_file_label, resolved_path);
        }
        meta_validate_check_tabular_sections(&mut report, child_obj_node);
        if report.stopped {
            return meta_validate_finish(report, out_file, out_file_label, resolved_path);
        }
        meta_validate_check_cross_properties(
            &mut report,
            md_type,
            props_node,
            child_obj_node,
            config_dir.as_deref(),
            &obj_name,
        );
        if report.stopped {
            return meta_validate_finish(report, out_file, out_file_label, resolved_path);
        }
        meta_validate_check_services(&mut report, md_type, child_obj_node);
        if report.stopped {
            return meta_validate_finish(report, out_file, out_file_label, resolved_path);
        }
        meta_validate_check_forbidden_properties(&mut report, md_type, props_node);
        if report.stopped {
            return meta_validate_finish(report, out_file, out_file_label, resolved_path);
        }
        meta_validate_check_method_reference(
            &mut report,
            md_type,
            props_node,
            config_dir.as_deref(),
        );
        if report.stopped {
            return meta_validate_finish(report, out_file, out_file_label, resolved_path);
        }
        meta_validate_check_document_journal_columns(&mut report, md_type, child_obj_node);

        meta_validate_finish(report, out_file, out_file_label, resolved_path)
    })();

    match result {
        Ok(run) => {
            let mut artifacts = vec![run.artifact.display().to_string()];
            if let Some(out_file) = &run.out_file {
                artifacts.push(out_file.display().to_string());
            }
            AdapterOutcome {
                ok: run.ok,
                summary: if run.ok {
                    "unica.meta.validate completed with native metadata validator".to_string()
                } else {
                    "unica.meta.validate failed in native metadata validator".to_string()
                },
                changes: Vec::new(),
                warnings: Vec::new(),
                errors: run.errors,
                artifacts,
                stdout: Some(run.stdout),
                stderr: Some(String::new()),
                command: None,
            }
        }
        Err(error) => AdapterOutcome {
            ok: false,
            summary: "unica.meta.validate failed in native metadata validator".to_string(),
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

pub(crate) fn meta_validate_finish(
    report: MetaValidationReporter,
    out_file: Option<PathBuf>,
    out_file_label: Option<String>,
    artifact: PathBuf,
) -> Result<MetaValidationRun, String> {
    let (ok, result_text, errors) = report.finalize();
    let stdout = if let Some(out_file) = &out_file {
        write_utf8_bom(out_file, &result_text)?;
        let label = out_file_label
            .as_deref()
            .unwrap_or_else(|| out_file.to_str().unwrap_or(""));
        format!("{result_text}\nWritten to: {label}\n")
    } else {
        format!("{result_text}\n")
    };
    Ok(MetaValidationRun {
        ok,
        stdout,
        out_file,
        artifact,
        errors,
    })
}

pub(crate) fn meta_validate_config_dir(resolved_path: &Path) -> Option<PathBuf> {
    let mut probe = resolved_path.parent();
    for _ in 0..4 {
        let Some(dir) = probe else {
            break;
        };
        if dir.join("Configuration.xml").exists() {
            return Some(dir.to_path_buf());
        }
        probe = dir.parent();
    }
    None
}

pub(crate) fn meta_validate_check_internal_info(
    report: &mut MetaValidationReporter,
    md_type: &str,
    type_node: roxmltree::Node<'_, '_>,
    obj_name: &str,
) {
    let internal_info = meta_info_child(type_node, "InternalInfo");
    if meta_validate_types_without_internal_info().contains(&md_type) {
        if let Some(internal_info) = internal_info {
            let gen_types = meta_info_children(internal_info, "GeneratedType");
            if gen_types.is_empty() {
                report.ok(format!(
                    "2. InternalInfo: absent or empty (correct for {md_type})"
                ));
            } else {
                report.warn(format!(
                    "2. InternalInfo: {md_type} should not have GeneratedType entries, found {}",
                    gen_types.len()
                ));
            }
        } else {
            report.ok(format!("2. InternalInfo: absent (correct for {md_type})"));
        }
        return;
    }

    let Some(expected_categories) = meta_validate_generated_categories(md_type) else {
        return;
    };
    let Some(internal_info) = internal_info else {
        report.error(format!(
            "2. InternalInfo: missing (expected {} GeneratedType)",
            expected_categories.len()
        ));
        return;
    };
    let gen_types = meta_info_children(internal_info, "GeneratedType");
    let mut check_ok = true;
    let mut found_categories = Vec::<String>::new();
    for generated_type in &gen_types {
        let name = generated_type.attribute("name").unwrap_or("");
        let category = generated_type.attribute("category").unwrap_or("");
        found_categories.push(category.to_string());
        if !name.is_empty() && obj_name != "(unknown)" && !name.ends_with(&format!(".{obj_name}")) {
            report.error(format!(
                "2. GeneratedType name '{name}' does not end with '.{obj_name}'"
            ));
            check_ok = false;
        }
        if !expected_categories.contains(&category) {
            report.warn(format!(
                "2. Unexpected GeneratedType category '{category}' for {md_type}"
            ));
        }
        if let Some(type_id) = meta_info_child(*generated_type, "TypeId") {
            if !is_guid(&meta_info_inner_text(type_id)) {
                report.error(format!(
                    "2. Invalid TypeId UUID in GeneratedType '{category}'"
                ));
                check_ok = false;
            }
        }
        if let Some(value_id) = meta_info_child(*generated_type, "ValueId") {
            if !is_guid(&meta_info_inner_text(value_id)) {
                report.error(format!(
                    "2. Invalid ValueId UUID in GeneratedType '{category}'"
                ));
                check_ok = false;
            }
        }
    }

    if md_type == "ExchangePlan" {
        if let Some(this_node) = meta_info_child(internal_info, "ThisNode") {
            if !is_guid(&meta_info_inner_text(this_node)) {
                report.error("2. ExchangePlan xr:ThisNode has invalid UUID");
                check_ok = false;
            }
        } else {
            report.warn("2. ExchangePlan missing xr:ThisNode in InternalInfo");
        }
    }

    let missing_categories = expected_categories
        .iter()
        .filter(|category| !found_categories.iter().any(|found| found == **category))
        .copied()
        .collect::<Vec<_>>();
    if !missing_categories.is_empty() {
        report.warn(format!(
            "2. Missing GeneratedType categories: {}",
            missing_categories.join(", ")
        ));
    }
    if check_ok {
        found_categories.sort();
        report.ok(format!(
            "2. InternalInfo: {} GeneratedType ({})",
            gen_types.len(),
            found_categories.join(", ")
        ));
    }
}

pub(crate) fn meta_validate_check_properties(
    report: &mut MetaValidationReporter,
    props_node: Option<roxmltree::Node<'_, '_>>,
    name_node: Option<roxmltree::Node<'_, '_>>,
    obj_name: &str,
) {
    let Some(props_node) = props_node else {
        report.error("3. Properties block missing");
        return;
    };
    let mut check_ok = true;
    if name_node.is_none() || obj_name.is_empty() {
        report.error("3. Properties: Name is missing or empty");
        check_ok = false;
    } else {
        if !is_1c_identifier(obj_name) {
            report.error(format!(
                "3. Properties: Name '{obj_name}' is not a valid 1C identifier"
            ));
            check_ok = false;
        }
        if obj_name.chars().count() > 80 {
            report.warn(format!(
                "3. Properties: Name '{obj_name}' is longer than 80 characters ({})",
                obj_name.chars().count()
            ));
        }
    }
    let syn_present = meta_info_child(props_node, "Synonym")
        .and_then(|node| meta_info_child(node, "item"))
        .and_then(|node| meta_info_child(node, "content"))
        .map(meta_info_inner_text)
        .is_some_and(|value| !value.is_empty());
    if check_ok {
        let syn_info = if syn_present {
            "Synonym present"
        } else {
            "no Synonym"
        };
        report.ok(format!("3. Properties: Name=\"{obj_name}\", {syn_info}"));
    }
}

pub(crate) fn meta_validate_check_property_values(
    report: &mut MetaValidationReporter,
    props_node: Option<roxmltree::Node<'_, '_>>,
) {
    let Some(props_node) = props_node else {
        report.warn("4. No Properties block to check");
        return;
    };
    let mut enum_checked = 0usize;
    let mut check_ok = true;
    for (prop_name, allowed) in meta_validate_property_values() {
        if let Some(value) =
            meta_info_child_text(props_node, prop_name).filter(|value| !value.is_empty())
        {
            if !allowed.contains(&value.as_str()) {
                report.error(format!(
                    "4. Property '{prop_name}' has invalid value '{value}' (allowed: {})",
                    allowed.join(", ")
                ));
                check_ok = false;
            }
            enum_checked += 1;
        }
    }
    if check_ok {
        report.ok(format!(
            "4. Property values: {enum_checked} enum properties checked"
        ));
    }
}

pub(crate) fn meta_validate_check_standard_attributes(
    report: &mut MetaValidationReporter,
    md_type: &str,
    props_node: Option<roxmltree::Node<'_, '_>>,
) {
    if !meta_validate_types_with_std_attrs().contains(&md_type) {
        return;
    }
    let Some(props_node) = props_node else {
        return;
    };
    let Some(std_attr_node) = meta_info_child(props_node, "StandardAttributes") else {
        report.ok(format!(
            "5. StandardAttributes: absent (optional for {md_type})"
        ));
        return;
    };
    let std_attrs = meta_info_children(std_attr_node, "StandardAttribute");
    let expected_std_attrs = meta_validate_standard_attributes(md_type).unwrap_or_default();
    let mut check_ok = true;
    let mut found_names = Vec::<String>::new();
    for standard_attr in &std_attrs {
        let name = standard_attr.attribute("name").unwrap_or("");
        if name.is_empty() {
            report.error("5. StandardAttribute without 'name' attribute");
            check_ok = false;
            continue;
        }
        found_names.push(name.to_string());
        if !expected_std_attrs.contains(&name)
            && !meta_validate_dynamic_standard_attr(md_type, name)
        {
            report.warn(format!(
                "5. Unexpected StandardAttribute '{name}' for {md_type}"
            ));
        }
    }
    let missing_attrs = expected_std_attrs
        .iter()
        .filter(|attr| !found_names.iter().any(|found| found == **attr))
        .copied()
        .collect::<Vec<_>>();
    if !missing_attrs.is_empty() {
        report.warn(format!(
            "5. Missing StandardAttributes: {}",
            missing_attrs.join(", ")
        ));
    }
    if check_ok {
        report.ok(format!(
            "5. StandardAttributes: {} entries",
            std_attrs.len()
        ));
    }
}

pub(crate) fn meta_validate_check_child_objects(
    report: &mut MetaValidationReporter,
    md_type: &str,
    child_obj_node: Option<roxmltree::Node<'_, '_>>,
) {
    let allowed_children = meta_validate_child_rules(md_type).unwrap_or_default();
    if let Some(child_obj_node) = child_obj_node {
        let mut check_ok = true;
        let mut child_counts = BTreeMap::<String, usize>::new();
        for child in child_obj_node.children().filter(|child| child.is_element()) {
            let child_tag = child.tag_name().name();
            if !allowed_children.contains(&child_tag) {
                report.error(format!(
                    "6. ChildObjects: disallowed element '{child_tag}' for {md_type}"
                ));
                check_ok = false;
            }
            *child_counts.entry(child_tag.to_string()).or_default() += 1;
        }
        if check_ok {
            if child_counts.is_empty() {
                report.ok(format!("6. ChildObjects: empty (valid for {md_type})"));
            } else {
                let summary = child_counts
                    .iter()
                    .map(|(name, count)| format!("{name}({count})"))
                    .collect::<Vec<_>>()
                    .join(", ");
                report.ok(format!("6. ChildObjects types: {summary}"));
            }
        }
    } else if allowed_children.is_empty() {
        report.ok(format!("6. ChildObjects: absent (correct for {md_type})"));
    } else {
        report.ok("6. ChildObjects: absent");
    }
}

pub(crate) fn meta_validate_check_child_elements(
    report: &mut MetaValidationReporter,
    child_obj_node: Option<roxmltree::Node<'_, '_>>,
) {
    let Some(child_obj_node) = child_obj_node else {
        return;
    };
    let mut check_ok = true;
    let mut count = 0usize;
    for kind in ["Attribute", "Dimension", "Resource", "EnumValue", "Column"] {
        let require_type = !matches!(kind, "EnumValue" | "Column");
        for element in meta_info_children(child_obj_node, kind) {
            if !meta_validate_check_child_element(report, element, kind, require_type) {
                check_ok = false;
            }
            count += 1;
            if report.stopped {
                break;
            }
        }
    }
    if check_ok && count > 0 {
        report.ok(format!(
            "7. Child elements: {count} items checked (UUID, Name, Type)"
        ));
    } else if count == 0 {
        report.ok("7. Child elements: none to check");
    }
}

pub(crate) fn meta_validate_check_child_element(
    report: &mut MetaValidationReporter,
    node: roxmltree::Node<'_, '_>,
    kind: &str,
    require_type: bool,
) -> bool {
    let uuid = node.attribute("uuid").unwrap_or("");
    if uuid.is_empty() {
        report.error(format!("7. {kind} missing uuid"));
        return false;
    }
    if !is_guid(uuid) {
        report.error(format!("7. {kind} has invalid uuid '{uuid}'"));
        return false;
    }
    let Some(props) = meta_info_child(node, "Properties") else {
        report.error(format!("7. {kind} (uuid={uuid}) missing Properties"));
        return false;
    };
    let name = meta_info_child_text(props, "Name").unwrap_or_default();
    if name.is_empty() {
        report.error(format!("7. {kind} (uuid={uuid}) missing or empty Name"));
        return false;
    }
    if !is_1c_identifier(&name) {
        report.error(format!("7. {kind} '{name}' has invalid identifier"));
        return false;
    }
    if require_type {
        let Some(type_el) = meta_info_child(props, "Type") else {
            report.error(format!("7. {kind} '{name}' missing Type block"));
            return false;
        };
        if meta_info_children(type_el, "Type").is_empty()
            && meta_info_children(type_el, "TypeSet").is_empty()
        {
            report.error(format!(
                "7. {kind} '{name}' Type block has no v8:Type or v8:TypeSet"
            ));
            return false;
        }
    }
    true
}

pub(crate) fn meta_validate_check_reserved_attr_names(
    report: &mut MetaValidationReporter,
    child_obj_node: Option<roxmltree::Node<'_, '_>>,
) {
    let Some(child_obj_node) = child_obj_node else {
        return;
    };
    let mut check_ok = true;
    for attr_node in meta_info_children(child_obj_node, "Attribute") {
        if let Some(name) = meta_info_child(attr_node, "Properties")
            .and_then(|props| meta_info_child_text(props, "Name"))
            .filter(|value| meta_validate_reserved_attr_names().contains(&value.as_str()))
        {
            report.warn(format!(
                "7b. Attribute '{name}' conflicts with a standard attribute name"
            ));
            check_ok = false;
        }
    }
    if check_ok {
        report.ok("7b. Reserved attribute names: no conflicts");
    }
}

pub(crate) fn meta_validate_check_uniqueness(
    report: &mut MetaValidationReporter,
    child_obj_node: Option<roxmltree::Node<'_, '_>>,
) {
    let Some(child_obj_node) = child_obj_node else {
        return;
    };
    let mut check_ok = true;
    for kind in [
        "Attribute",
        "TabularSection",
        "Dimension",
        "Resource",
        "EnumValue",
        "Column",
        "URLTemplate",
        "Operation",
    ] {
        if !meta_validate_names_unique(report, meta_info_children(child_obj_node, kind), kind) {
            check_ok = false;
        }
    }
    if check_ok {
        report.ok("8. Name uniqueness: all names unique");
    }
}

pub(crate) fn meta_validate_names_unique(
    report: &mut MetaValidationReporter,
    nodes: Vec<roxmltree::Node<'_, '_>>,
    kind: &str,
) -> bool {
    let mut names = HashSet::<String>::new();
    let mut ok = true;
    for node in nodes {
        let Some(name) = meta_info_child(node, "Properties")
            .and_then(|props| meta_info_child_text(props, "Name"))
            .filter(|value| !value.is_empty())
        else {
            continue;
        };
        if !names.insert(name.clone()) {
            report.error(format!("8. Duplicate {kind} name: '{name}'"));
            ok = false;
        }
    }
    ok
}

pub(crate) fn meta_validate_check_tabular_sections(
    report: &mut MetaValidationReporter,
    child_obj_node: Option<roxmltree::Node<'_, '_>>,
) {
    let Some(child_obj_node) = child_obj_node else {
        return;
    };
    let sections = meta_info_children(child_obj_node, "TabularSection");
    if sections.is_empty() {
        report.ok("9. TabularSections: none present");
        return;
    }
    let mut check_ok = true;
    for (index, section) in sections.iter().enumerate() {
        let count = index + 1;
        let uuid = section.attribute("uuid").unwrap_or("");
        if uuid.is_empty() || !is_guid(uuid) {
            report.error(format!(
                "9. TabularSection #{count}: invalid or missing uuid"
            ));
            check_ok = false;
        }
        let props = meta_info_child(*section, "Properties");
        let section_name = props
            .and_then(|node| meta_info_child_text(node, "Name"))
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| "(unnamed)".to_string());
        if section_name == "(unnamed)" {
            report.error(format!("9. TabularSection #{count}: missing or empty Name"));
            check_ok = false;
        }
        if let Some(internal_info) = meta_info_child(*section, "InternalInfo") {
            let generated = meta_info_children(internal_info, "GeneratedType");
            if generated.len() < 2 {
                report.warn(format!(
                    "9. TabularSection '{section_name}': expected 2 GeneratedType, found {}",
                    generated.len()
                ));
            }
        }
        if let Some(ts_child_obj) = meta_info_child(*section, "ChildObjects") {
            let mut names = HashSet::<String>::new();
            for attr in meta_info_children(ts_child_obj, "Attribute") {
                if !meta_validate_check_child_element(
                    report,
                    attr,
                    &format!("TabularSection '{section_name}'.Attribute"),
                    true,
                ) {
                    check_ok = false;
                }
                if let Some(name) = meta_info_child(attr, "Properties")
                    .and_then(|node| meta_info_child_text(node, "Name"))
                    .filter(|value| !value.is_empty())
                {
                    if !names.insert(name.clone()) {
                        report.error(format!(
                            "9. Duplicate attribute '{name}' in TabularSection '{section_name}'"
                        ));
                        check_ok = false;
                    }
                }
            }
            if let Some(props) = props {
                if let Some(std_attr) = meta_info_child(props, "StandardAttributes") {
                    let has_line_number = meta_info_children(std_attr, "StandardAttribute")
                        .iter()
                        .any(|attr| attr.attribute("name") == Some("LineNumber"));
                    if !has_line_number {
                        report.warn(format!(
                            "9. TabularSection '{section_name}': missing LineNumber StandardAttribute"
                        ));
                    }
                }
            }
        }
    }
    if check_ok {
        report.ok(format!(
            "9. TabularSections: {} sections, structure valid",
            sections.len()
        ));
    }
}

pub(crate) fn meta_validate_check_cross_properties(
    report: &mut MetaValidationReporter,
    md_type: &str,
    props_node: Option<roxmltree::Node<'_, '_>>,
    child_obj_node: Option<roxmltree::Node<'_, '_>>,
    config_dir: Option<&Path>,
    obj_name: &str,
) {
    let Some(props_node) = props_node else {
        return;
    };
    let mut check_ok = true;
    let mut issues = 0usize;
    if meta_info_child_text(props_node, "Hierarchical").as_deref() == Some("false") {
        if let Some(hierarchy_type) =
            meta_info_child_text(props_node, "HierarchyType").filter(|value| !value.is_empty())
        {
            report.warn(format!(
                "10. HierarchyType='{hierarchy_type}' but Hierarchical=false"
            ));
            issues += 1;
        }
    }
    if md_type == "CommonModule" {
        let any_enabled = [
            "Server",
            "ClientManagedApplication",
            "ClientOrdinaryApplication",
            "ExternalConnection",
            "ServerCall",
            "Global",
        ]
        .iter()
        .any(|name| meta_info_child_text(props_node, name).as_deref() == Some("true"));
        if !any_enabled {
            report.warn("10. CommonModule: no execution context enabled");
            issues += 1;
        }
    }
    if md_type == "EventSubscription" {
        if meta_info_child_text(props_node, "Handler").is_none_or(|value| value.trim().is_empty()) {
            report.error("10. EventSubscription: empty Handler");
            check_ok = false;
            issues += 1;
        }
        let has_source = meta_info_child(props_node, "Source")
            .map(|node| !meta_info_children(node, "Type").is_empty())
            .unwrap_or(false);
        if !has_source {
            report.warn("10. EventSubscription: no Source types specified");
            issues += 1;
        }
    }
    if md_type == "ScheduledJob"
        && meta_info_child_text(props_node, "MethodName")
            .is_none_or(|value| value.trim().is_empty())
    {
        report.error("10. ScheduledJob: empty MethodName");
        check_ok = false;
        issues += 1;
    }
    for (type_name, property, message) in [
        (
            "AccountingRegister",
            "ChartOfAccounts",
            "10. AccountingRegister: empty ChartOfAccounts",
        ),
        (
            "CalculationRegister",
            "ChartOfCalculationTypes",
            "10. CalculationRegister: empty ChartOfCalculationTypes",
        ),
    ] {
        if md_type == type_name
            && meta_info_child_text(props_node, property)
                .is_none_or(|value| value.trim().is_empty())
        {
            report.error(message);
            check_ok = false;
            issues += 1;
        }
    }
    if md_type == "BusinessProcess"
        && meta_info_child_text(props_node, "Task").is_none_or(|value| value.trim().is_empty())
    {
        report.warn("10. BusinessProcess: empty Task reference");
        issues += 1;
    }
    if md_type == "CalculationRegister"
        && meta_info_child_text(props_node, "ActionPeriod").as_deref() == Some("true")
        && meta_info_child_text(props_node, "Schedule").is_none_or(|value| value.trim().is_empty())
    {
        report.warn(
            "10. CalculationRegister: ActionPeriod=true but Schedule is empty — platform requires a schedule register",
        );
        issues += 1;
    }
    if md_type == "DocumentJournal" {
        let has_registered = meta_info_child(props_node, "RegisteredDocuments")
            .map(|node| !meta_info_children(node, "Type").is_empty())
            .unwrap_or(false);
        if !has_registered {
            report.warn("10. DocumentJournal: no RegisteredDocuments specified");
            issues += 1;
        }
    }
    if md_type == "ChartOfAccounts" {
        let max_ext_dim = meta_info_child_text(props_node, "MaxExtDimensionCount")
            .and_then(|value| value.parse::<i64>().ok())
            .unwrap_or(0);
        if max_ext_dim > 0
            && meta_info_child_text(props_node, "ExtDimensionTypes")
                .is_none_or(|value| value.trim().is_empty())
        {
            report
                .warn("10. ChartOfAccounts: MaxExtDimensionCount>0 but ExtDimensionTypes is empty");
            issues += 1;
        }
    }
    if matches!(
        md_type,
        "AccumulationRegister"
            | "AccountingRegister"
            | "CalculationRegister"
            | "InformationRegister"
    ) {
        if let Some(child_obj_node) = child_obj_node {
            let count = meta_info_children(child_obj_node, "Dimension").len()
                + meta_info_children(child_obj_node, "Resource").len()
                + meta_info_children(child_obj_node, "Attribute").len();
            if count == 0 {
                report.warn(format!(
                    "10. {md_type}: no Dimensions, Resources, or Attributes — platform will reject"
                ));
                issues += 1;
            }
        }
    }
    meta_validate_check_document_register_records(
        report,
        md_type,
        props_node,
        config_dir,
        &mut issues,
    );
    meta_validate_check_register_registrar(
        report,
        md_type,
        props_node,
        config_dir,
        obj_name,
        &mut issues,
    );
    if check_ok && issues == 0 {
        report.ok("10. Cross-property consistency");
    }
}

pub(crate) fn meta_validate_check_document_register_records(
    report: &mut MetaValidationReporter,
    md_type: &str,
    props_node: roxmltree::Node<'_, '_>,
    config_dir: Option<&Path>,
    issues: &mut usize,
) {
    if md_type != "Document" {
        return;
    }
    let Some(config_dir) = config_dir else {
        return;
    };
    let Some(register_records) = meta_info_child(props_node, "RegisterRecords") else {
        return;
    };
    for item in meta_info_children(register_records, "Item") {
        let ref_value = meta_info_inner_text(item).trim().to_string();
        let Some((ref_type, ref_name)) = ref_value.split_once('.') else {
            continue;
        };
        let ref_dir = match ref_type {
            "AccumulationRegister" => "AccumulationRegisters",
            "InformationRegister" => "InformationRegisters",
            "AccountingRegister" => "AccountingRegisters",
            "CalculationRegister" => "CalculationRegisters",
            _ => continue,
        };
        let ref_path = config_dir.join(ref_dir).join(ref_name);
        let ref_xml = config_dir.join(ref_dir).join(format!("{ref_name}.xml"));
        if !ref_path.exists() && !ref_xml.exists() {
            report.warn(format!(
                "10. Document.RegisterRecords references '{ref_value}' but object not found in config"
            ));
            *issues += 1;
        }
    }
}

pub(crate) fn meta_validate_check_register_registrar(
    report: &mut MetaValidationReporter,
    md_type: &str,
    props_node: roxmltree::Node<'_, '_>,
    config_dir: Option<&Path>,
    obj_name: &str,
    issues: &mut usize,
) {
    if !matches!(
        md_type,
        "AccumulationRegister"
            | "AccountingRegister"
            | "CalculationRegister"
            | "InformationRegister"
    ) || obj_name == "(unknown)"
    {
        return;
    }
    if md_type == "InformationRegister"
        && meta_info_child_text(props_node, "WriteMode").as_deref() != Some("RecorderSubordinate")
    {
        return;
    }
    let Some(config_dir) = config_dir else {
        return;
    };
    let docs_dir = config_dir.join("Documents");
    let reg_ref = format!("{md_type}.{obj_name}");
    let mut has_registrar = false;
    if docs_dir.is_dir() {
        if let Ok(entries) = fs::read_dir(&docs_dir) {
            for entry in entries.filter_map(Result::ok) {
                let path = entry.path();
                if path.extension().and_then(|ext| ext.to_str()) != Some("xml") || !path.is_file() {
                    continue;
                }
                if read_utf8_sig(&path)
                    .map(|content| content.contains(&reg_ref))
                    .unwrap_or(false)
                {
                    has_registrar = true;
                    break;
                }
            }
        }
    }
    if !has_registrar {
        report.warn(format!(
            "10. {md_type}: no registrar document found (none references '{reg_ref}' in RegisterRecords)"
        ));
        *issues += 1;
    }
}

pub(crate) fn meta_validate_check_services(
    report: &mut MetaValidationReporter,
    md_type: &str,
    child_obj_node: Option<roxmltree::Node<'_, '_>>,
) {
    let Some(child_obj_node) = child_obj_node else {
        return;
    };
    if md_type == "HTTPService" {
        let templates = meta_info_children(child_obj_node, "URLTemplate");
        let mut check_ok = true;
        let mut method_count = 0usize;
        for template in &templates {
            let props = meta_info_child(*template, "Properties");
            let name = props
                .and_then(|node| meta_info_child_text(node, "Name"))
                .unwrap_or_else(|| "(unnamed)".to_string());
            if props
                .and_then(|node| meta_info_child_text(node, "Template"))
                .is_none_or(|value| value.trim().is_empty())
            {
                report.error(format!(
                    "11. HTTPService URLTemplate '{name}': empty Template"
                ));
                check_ok = false;
            }
            if let Some(child_objects) = meta_info_child(*template, "ChildObjects") {
                for method in meta_info_children(child_objects, "Method") {
                    method_count += 1;
                    let props = meta_info_child(method, "Properties");
                    let http_method =
                        props.and_then(|node| meta_info_child_text(node, "HTTPMethod"));
                    if let Some(http_method) = http_method.filter(|value| !value.is_empty()) {
                        if !meta_validate_valid_http_methods().contains(&http_method.as_str()) {
                            report.error(format!(
                                "11. HTTPService URLTemplate '{name}': invalid HTTPMethod '{http_method}'"
                            ));
                            check_ok = false;
                        }
                    } else {
                        report.error(format!(
                            "11. HTTPService URLTemplate '{name}': Method missing HTTPMethod"
                        ));
                        check_ok = false;
                    }
                }
            }
        }
        if check_ok {
            report.ok(format!(
                "11. HTTPService: {} URLTemplate(s), {method_count} method(s)",
                templates.len()
            ));
        }
    } else if md_type == "WebService" {
        let operations = meta_info_children(child_obj_node, "Operation");
        let mut check_ok = true;
        let mut param_count = 0usize;
        for operation in &operations {
            let props = meta_info_child(*operation, "Properties");
            let name = props
                .and_then(|node| meta_info_child_text(node, "Name"))
                .unwrap_or_else(|| "(unnamed)".to_string());
            if props
                .and_then(|node| meta_info_child_text(node, "XDTOReturningValueType"))
                .is_none_or(|value| value.trim().is_empty())
            {
                report.warn(format!(
                    "11. WebService Operation '{name}': no XDTOReturningValueType"
                ));
            }
            if let Some(child_objects) = meta_info_child(*operation, "ChildObjects") {
                for param in meta_info_children(child_objects, "Parameter") {
                    param_count += 1;
                    let direction = meta_info_child(param, "Properties")
                        .and_then(|node| meta_info_child_text(node, "TransferDirection"));
                    if let Some(direction) = direction.filter(|value| !value.is_empty()) {
                        if !["In", "Out", "InOut"].contains(&direction.as_str()) {
                            report.error(format!(
                                "11. WebService Operation '{name}': Parameter has invalid TransferDirection '{direction}'"
                            ));
                            check_ok = false;
                        }
                    }
                }
            }
        }
        if check_ok {
            report.ok(format!(
                "11. WebService: {} operation(s), {param_count} parameter(s)",
                operations.len()
            ));
        }
    }
}

pub(crate) fn meta_validate_check_forbidden_properties(
    report: &mut MetaValidationReporter,
    md_type: &str,
    props_node: Option<roxmltree::Node<'_, '_>>,
) {
    let Some(props_node) = props_node else {
        return;
    };
    let Some(forbidden) = meta_validate_forbidden_properties(md_type) else {
        return;
    };
    let mut check_ok = true;
    for property in forbidden {
        if meta_info_child(props_node, property).is_some() {
            report.error(format!(
                "12. Forbidden property '{property}' present in {md_type} (will fail on LoadConfigFromFiles)"
            ));
            check_ok = false;
        }
    }
    if check_ok {
        report.ok("12. Forbidden properties: none found");
    }
}

pub(crate) fn meta_validate_check_method_reference(
    report: &mut MetaValidationReporter,
    md_type: &str,
    props_node: Option<roxmltree::Node<'_, '_>>,
    config_dir: Option<&Path>,
) {
    if !matches!(md_type, "EventSubscription" | "ScheduledJob") {
        return;
    }
    let (Some(props_node), Some(config_dir)) = (props_node, config_dir) else {
        return;
    };
    let (property, method_ref) = if md_type == "EventSubscription" {
        ("Handler", meta_info_child_text(props_node, "Handler"))
    } else {
        ("MethodName", meta_info_child_text(props_node, "MethodName"))
    };
    let Some(method_ref) = method_ref.filter(|value| !value.is_empty()) else {
        return;
    };
    let parts = method_ref.split('.').collect::<Vec<_>>();
    let parsed = if parts.len() == 3 && parts[0] == "CommonModule" {
        Some((parts[1], parts[2]))
    } else if parts.len() == 2 {
        Some((parts[0], parts[1]))
    } else {
        None
    };
    let Some((module_name, proc_name)) = parsed else {
        report.error(format!(
            "13. {md_type}.{property} = '{method_ref}': expected format 'CommonModule.ModuleName.ProcedureName'"
        ));
        return;
    };
    let module_xml = config_dir
        .join("CommonModules")
        .join(format!("{module_name}.xml"));
    if !module_xml.exists() {
        report.error(format!(
            "13. {md_type}.{property}: CommonModule '{module_name}' not found (expected {})",
            module_xml.display()
        ));
        return;
    }
    let bsl_path = config_dir
        .join("CommonModules")
        .join(module_name)
        .join("Ext")
        .join("Module.bsl");
    if bsl_path.exists() {
        if let Ok(content) = read_utf8_sig(&bsl_path) {
            if !meta_validate_bsl_has_export(&content, proc_name) {
                report.warn(format!(
                    "13. {md_type}.{property}: procedure '{proc_name}' not found as exported in CommonModule '{module_name}'"
                ));
                return;
            }
        }
    } else {
        report.warn(format!(
            "13. {md_type}.{property}: BSL file not found ({}), cannot verify procedure",
            bsl_path.display()
        ));
        return;
    }
    report.ok(format!("13. Method reference: {property} = '{method_ref}'"));
}

pub(crate) fn meta_validate_check_document_journal_columns(
    report: &mut MetaValidationReporter,
    md_type: &str,
    child_obj_node: Option<roxmltree::Node<'_, '_>>,
) {
    if md_type != "DocumentJournal" {
        return;
    }
    let Some(child_obj_node) = child_obj_node else {
        return;
    };
    let columns = meta_info_children(child_obj_node, "Column");
    let mut check_ok = true;
    let mut empty_ref_count = 0usize;
    for column in &columns {
        let props = meta_info_child(*column, "Properties");
        let name = props
            .and_then(|node| meta_info_child_text(node, "Name"))
            .unwrap_or_else(|| "(unnamed)".to_string());
        let has_items = props
            .and_then(|node| meta_info_child(node, "References"))
            .map(|node| !meta_info_children(node, "Item").is_empty())
            .unwrap_or(false);
        if !has_items {
            report.error(format!(
                "14. DocumentJournal Column '{name}': empty References (will fail on LoadConfigFromFiles)"
            ));
            check_ok = false;
            empty_ref_count += 1;
        }
    }
    if check_ok && !columns.is_empty() {
        report.ok(format!(
            "14. DocumentJournal Columns: {} column(s), all have References",
            columns.len()
        ));
    } else if columns.is_empty() && empty_ref_count == 0 {
        report.ok("14. DocumentJournal Columns: none");
    }
}

pub(crate) fn meta_validate_bsl_has_export(content: &str, proc_name: &str) -> bool {
    content.lines().any(|line| {
        let trimmed = line.trim_start();
        let starts = ["Procedure", "Function", "Процедура", "Функция"]
            .iter()
            .any(|prefix| trimmed.starts_with(prefix));
        starts
            && trimmed.contains(proc_name)
            && (trimmed.contains(" Export") || trimmed.contains(" Экспорт"))
    })
}

pub(crate) fn is_guid(value: &str) -> bool {
    let bytes = value.as_bytes();
    value.len() == 36
        && [8, 13, 18, 23].iter().all(|index| bytes[*index] == b'-')
        && value
            .chars()
            .enumerate()
            .all(|(index, ch)| [8, 13, 18, 23].contains(&index) || ch.is_ascii_hexdigit())
}

pub(crate) fn meta_validate_valid_types() -> &'static [&'static str] {
    &[
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
    ]
}

pub(crate) fn meta_validate_generated_categories(md_type: &str) -> Option<&'static [&'static str]> {
    match md_type {
        "Catalog" | "Document" => Some(&["Object", "Ref", "Selection", "List", "Manager"]),
        "Enum" => Some(&["Ref", "Manager", "List"]),
        "Constant" => Some(&["Manager", "ValueManager", "ValueKey"]),
        "InformationRegister" => Some(&[
            "Record",
            "Manager",
            "Selection",
            "List",
            "RecordSet",
            "RecordKey",
            "RecordManager",
        ]),
        "AccumulationRegister" => Some(&[
            "Record",
            "Manager",
            "Selection",
            "List",
            "RecordSet",
            "RecordKey",
        ]),
        "AccountingRegister" => Some(&[
            "Record",
            "Manager",
            "Selection",
            "List",
            "RecordSet",
            "RecordKey",
            "ExtDimensions",
        ]),
        "CalculationRegister" => Some(&[
            "Record",
            "Manager",
            "Selection",
            "List",
            "RecordSet",
            "RecordKey",
            "Recalcs",
        ]),
        "ChartOfAccounts" => Some(&[
            "Object",
            "Ref",
            "Selection",
            "List",
            "Manager",
            "ExtDimensionTypes",
            "ExtDimensionTypesRow",
        ]),
        "ChartOfCharacteristicTypes" => Some(&[
            "Object",
            "Ref",
            "Selection",
            "List",
            "Manager",
            "Characteristic",
        ]),
        "ChartOfCalculationTypes" => Some(&[
            "Object",
            "Ref",
            "Selection",
            "List",
            "Manager",
            "DisplacingCalculationTypes",
            "DisplacingCalculationTypesRow",
            "BaseCalculationTypes",
            "BaseCalculationTypesRow",
            "LeadingCalculationTypes",
            "LeadingCalculationTypesRow",
        ]),
        "BusinessProcess" => Some(&[
            "Object",
            "Ref",
            "Selection",
            "List",
            "Manager",
            "RoutePointRef",
        ]),
        "Task" | "ExchangePlan" => Some(&["Object", "Ref", "Selection", "List", "Manager"]),
        "DocumentJournal" => Some(&["Selection", "List", "Manager"]),
        "Report" | "DataProcessor" => Some(&["Object", "Manager"]),
        "DefinedType" => Some(&["DefinedType"]),
        _ => None,
    }
}

pub(crate) fn meta_validate_types_without_internal_info() -> &'static [&'static str] {
    &["CommonModule", "ScheduledJob", "EventSubscription"]
}

pub(crate) fn meta_validate_types_with_std_attrs() -> &'static [&'static str] {
    &[
        "Catalog",
        "Document",
        "Enum",
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
    ]
}

pub(crate) fn meta_validate_standard_attributes(md_type: &str) -> Option<&'static [&'static str]> {
    match md_type {
        "Catalog" => Some(&[
            "PredefinedDataName",
            "Predefined",
            "Ref",
            "DeletionMark",
            "IsFolder",
            "Owner",
            "Parent",
            "Description",
            "Code",
        ]),
        "Document" => Some(&["Posted", "Ref", "DeletionMark", "Date", "Number"]),
        "Enum" => Some(&["Order", "Ref"]),
        "InformationRegister" => Some(&["Active", "LineNumber", "Recorder", "Period"]),
        "AccumulationRegister" => {
            Some(&["Active", "LineNumber", "Recorder", "Period", "RecordType"])
        }
        "AccountingRegister" => Some(&["Active", "Period", "Recorder", "LineNumber", "Account"]),
        "CalculationRegister" => Some(&[
            "Active",
            "Recorder",
            "LineNumber",
            "RegistrationPeriod",
            "CalculationType",
            "ReversingEntry",
            "ActionPeriod",
            "BegOfActionPeriod",
            "EndOfActionPeriod",
            "BegOfBasePeriod",
            "EndOfBasePeriod",
        ]),
        "ChartOfAccounts" => Some(&[
            "PredefinedDataName",
            "Predefined",
            "Ref",
            "DeletionMark",
            "Description",
            "Code",
            "Parent",
            "Order",
            "Type",
            "OffBalance",
        ]),
        "ChartOfCharacteristicTypes" => Some(&[
            "PredefinedDataName",
            "Predefined",
            "Ref",
            "DeletionMark",
            "Description",
            "Code",
            "Parent",
            "IsFolder",
            "ValueType",
        ]),
        "ChartOfCalculationTypes" => Some(&[
            "PredefinedDataName",
            "Predefined",
            "Ref",
            "DeletionMark",
            "Description",
            "Code",
            "ActionPeriodIsBasic",
        ]),
        "BusinessProcess" => Some(&[
            "Ref",
            "DeletionMark",
            "Date",
            "Number",
            "Started",
            "Completed",
            "HeadTask",
        ]),
        "Task" => Some(&[
            "Ref",
            "DeletionMark",
            "Date",
            "Number",
            "Executed",
            "Description",
            "RoutePoint",
            "BusinessProcess",
        ]),
        "ExchangePlan" => Some(&[
            "Ref",
            "DeletionMark",
            "Code",
            "Description",
            "ThisNode",
            "SentNo",
            "ReceivedNo",
        ]),
        "DocumentJournal" => Some(&["Type", "Ref", "Date", "Posted", "DeletionMark", "Number"]),
        _ => None,
    }
}

pub(crate) fn meta_validate_dynamic_standard_attr(md_type: &str, name: &str) -> bool {
    (md_type == "AccountingRegister"
        && (name == "PeriodAdjustment"
            || name
                .strip_prefix("ExtDimension")
                .is_some_and(|rest| rest.chars().all(|ch| ch.is_ascii_digit()))
            || name
                .strip_prefix("ExtDimensionType")
                .is_some_and(|rest| rest.chars().all(|ch| ch.is_ascii_digit()))))
        || (md_type == "CalculationRegister"
            && matches!(
                name,
                "ActionPeriod"
                    | "BegOfActionPeriod"
                    | "EndOfActionPeriod"
                    | "BegOfBasePeriod"
                    | "EndOfBasePeriod"
            ))
}

pub(crate) fn meta_validate_child_rules(md_type: &str) -> Option<&'static [&'static str]> {
    match md_type {
        "Catalog"
        | "Document"
        | "ExchangePlan"
        | "ChartOfCharacteristicTypes"
        | "ChartOfCalculationTypes"
        | "BusinessProcess"
        | "Report"
        | "DataProcessor" => Some(&["Attribute", "TabularSection", "Form", "Template", "Command"]),
        "ChartOfAccounts" => Some(&[
            "Attribute",
            "TabularSection",
            "Form",
            "Template",
            "Command",
            "AccountingFlag",
            "ExtDimensionAccountingFlag",
        ]),
        "Task" => Some(&[
            "Attribute",
            "TabularSection",
            "Form",
            "Template",
            "Command",
            "AddressingAttribute",
        ]),
        "Enum" => Some(&["EnumValue", "Form", "Template", "Command"]),
        "InformationRegister" | "AccumulationRegister" | "AccountingRegister" => Some(&[
            "Dimension",
            "Resource",
            "Attribute",
            "Form",
            "Template",
            "Command",
        ]),
        "CalculationRegister" => Some(&[
            "Dimension",
            "Resource",
            "Attribute",
            "Form",
            "Template",
            "Command",
            "Recalculation",
        ]),
        "DocumentJournal" => Some(&["Column", "Form", "Template", "Command"]),
        "HTTPService" => Some(&["URLTemplate"]),
        "WebService" => Some(&["Operation"]),
        "Constant" => Some(&["Form"]),
        "DefinedType" | "CommonModule" | "ScheduledJob" | "EventSubscription" => Some(&[]),
        _ => None,
    }
}

pub(crate) fn meta_validate_property_values() -> &'static [(&'static str, &'static [&'static str])]
{
    &[
        ("CodeType", &["String", "Number"]),
        ("CodeAllowedLength", &["Variable", "Fixed"]),
        ("NumberType", &["String", "Number"]),
        ("NumberAllowedLength", &["Variable", "Fixed"]),
        ("Posting", &["Allow", "Deny"]),
        ("RealTimePosting", &["Allow", "Deny"]),
        (
            "RegisterRecordsDeletion",
            &["AutoDelete", "AutoDeleteOnUnpost", "AutoDeleteOff"],
        ),
        (
            "RegisterRecordsWritingOnPost",
            &["WriteModified", "WriteSelected", "WriteAll"],
        ),
        ("DataLockControlMode", &["Automatic", "Managed"]),
        ("FullTextSearch", &["Use", "DontUse"]),
        ("DefaultPresentation", &["AsDescription", "AsCode"]),
        (
            "HierarchyType",
            &["HierarchyFoldersAndItems", "HierarchyItemsOnly"],
        ),
        ("EditType", &["InDialog", "InList", "BothWays"]),
        ("WriteMode", &["Independent", "RecorderSubordinate"]),
        (
            "InformationRegisterPeriodicity",
            &[
                "Nonperiodical",
                "Second",
                "Day",
                "Month",
                "Quarter",
                "Year",
                "RecorderPosition",
            ],
        ),
        ("RegisterType", &["Balance", "Turnovers"]),
        (
            "ReturnValuesReuse",
            &["DontUse", "DuringRequest", "DuringSession"],
        ),
        ("ReuseSessions", &["DontUse", "AutoUse"]),
        ("FillChecking", &["DontCheck", "ShowError", "ShowWarning"]),
        (
            "Indexing",
            &["DontIndex", "Index", "IndexWithAdditionalOrder"],
        ),
        ("DataHistory", &["Use", "DontUse"]),
        (
            "DependenceOnCalculationTypes",
            &["DontUse", "OnActionPeriod"],
        ),
    ]
}

pub(crate) fn meta_validate_reserved_attr_names() -> &'static [&'static str] {
    &[
        "Ref",
        "DeletionMark",
        "Code",
        "Description",
        "Date",
        "Number",
        "Posted",
        "Parent",
        "Owner",
        "IsFolder",
        "Predefined",
        "PredefinedDataName",
        "Recorder",
        "Period",
        "LineNumber",
        "Active",
        "Order",
        "Type",
        "OffBalance",
        "Started",
        "Completed",
        "HeadTask",
        "Executed",
        "RoutePoint",
        "BusinessProcess",
        "ThisNode",
        "SentNo",
        "ReceivedNo",
        "CalculationType",
        "RegistrationPeriod",
        "ReversingEntry",
        "Account",
        "ValueType",
        "ActionPeriodIsBasic",
    ]
}

pub(crate) fn meta_validate_valid_http_methods() -> &'static [&'static str] {
    &[
        "GET", "POST", "PUT", "DELETE", "PATCH", "HEAD", "OPTIONS", "MERGE", "CONNECT",
    ]
}

pub(crate) fn meta_validate_forbidden_properties(md_type: &str) -> Option<&'static [&'static str]> {
    match md_type {
        "ChartOfCharacteristicTypes" => Some(&["CodeType"]),
        "ChartOfAccounts" => Some(&["Autonumbering", "Hierarchical"]),
        "ChartOfCalculationTypes" => Some(&["CheckUnique", "Autonumbering"]),
        "ExchangePlan" => Some(&["CodeType", "CheckUnique", "Autonumbering"]),
        _ => None,
    }
}

pub(crate) fn analyze_meta_info(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> AdapterOutcome {
    const MD_NS: &str = "http://v8.1c.ru/8.3/MDClasses";

    let result = (|| -> Result<(String, Option<PathBuf>, PathBuf), String> {
        let raw_path = required_path(
            args,
            &["objectPath", "ObjectPath", "path", "Path"],
            "ObjectPath",
        )?;
        let object_path = resolve_meta_info_path(absolutize(raw_path, &context.cwd))?;
        let text = read_utf8_sig(&object_path)?;
        let doc = Document::parse(text.trim_start_matches('\u{feff}'))
            .map_err(|err| format!("XML parse error in {}: {err}", object_path.display()))?;
        let root = doc.root_element();
        if root.tag_name().name() != "MetaDataObject" {
            return Err("[ERROR] Not a valid 1C metadata XML file".to_string());
        }

        let Some(type_node) = root
            .children()
            .find(|child| child.is_element() && child.tag_name().namespace() == Some(MD_NS))
        else {
            return Err("[ERROR] Cannot detect metadata type".to_string());
        };
        let md_type = type_node.tag_name().name();
        let props = meta_info_child(type_node, "Properties");
        let child_objs = meta_info_child(type_node, "ChildObjects");
        let obj_name = props
            .and_then(|node| meta_info_child_text(node, "Name"))
            .unwrap_or_default();
        let synonym = props
            .and_then(|node| meta_info_child(node, "Synonym"))
            .map(meta_info_ml_text)
            .unwrap_or_default();
        let mode = string_arg(args, &["mode", "Mode"]).unwrap_or("overview");
        let drill_name = string_arg(args, &["name", "Name"]).unwrap_or("");
        let out_file =
            path_arg(args, &["outFile", "OutFile"]).map(|path| absolutize(path, &context.cwd));

        let mut lines = if drill_name.is_empty() {
            meta_info_main_lines(md_type, props, child_objs, &obj_name, &synonym, mode)?
        } else {
            meta_info_drill_lines(md_type, child_objs, drill_name, &obj_name)?
        };
        if drill_name.is_empty() {
            lines.insert(
                1,
                format!("Поддержка: {}", support_status_for_path(&object_path)),
            );
        }
        let output_text = meta_info_paginate(lines, args);
        let stdout = if let Some(out_file) = &out_file {
            write_utf8_bom(out_file, &output_text)?;
            format!("Output written to {}\n", out_file.display())
        } else {
            format!("{output_text}\n")
        };

        Ok((stdout, out_file, object_path))
    })();

    match result {
        Ok((stdout, out_file, artifact)) => {
            let mut artifacts = vec![artifact.display().to_string()];
            if let Some(out_file) = out_file {
                artifacts.push(out_file.display().to_string());
            }
            AdapterOutcome {
                ok: true,
                summary: "unica.meta.info completed with native metadata analyzer".to_string(),
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
            summary: "unica.meta.info failed in native metadata analyzer".to_string(),
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

pub(crate) fn resolve_meta_info_path(mut object_path: PathBuf) -> Result<PathBuf, String> {
    if object_path.is_dir() {
        let dir_name = object_path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default()
            .to_string();
        let candidate = object_path.join(format!("{dir_name}.xml"));
        let sibling = object_path
            .parent()
            .unwrap_or_else(|| Path::new(""))
            .join(format!("{dir_name}.xml"));
        if candidate.is_file() {
            object_path = candidate;
        } else if sibling.is_file() {
            object_path = sibling;
        } else {
            let xml_file = fs::read_dir(&object_path)
                .map_err(|err| format!("failed to read {}: {err}", object_path.display()))?
                .filter_map(Result::ok)
                .map(|entry| entry.path())
                .find(|path| {
                    path.extension()
                        .and_then(|ext| ext.to_str())
                        .is_some_and(|ext| ext.eq_ignore_ascii_case("xml"))
                });
            if let Some(xml_file) = xml_file {
                object_path = xml_file;
            } else {
                return Err(format!(
                    "[ERROR] No XML file found in directory: {}",
                    object_path.display()
                ));
            }
        }
    }

    if !object_path.exists() {
        let file_name = object_path.file_stem().and_then(|name| name.to_str());
        let parent_dir = object_path.parent();
        let parent_dir_name = parent_dir
            .and_then(|path| path.file_name())
            .and_then(|name| name.to_str());
        if file_name == parent_dir_name {
            if let (Some(parent_dir), Some(file_name)) = (parent_dir, file_name) {
                let candidate = parent_dir
                    .parent()
                    .unwrap_or_else(|| Path::new(""))
                    .join(format!("{file_name}.xml"));
                if candidate.exists() {
                    object_path = candidate;
                }
            }
        }
    }

    if !object_path.exists() {
        return Err(format!("[ERROR] File not found: {}", object_path.display()));
    }
    Ok(object_path)
}

pub(crate) fn meta_info_main_lines(
    md_type: &str,
    props: Option<roxmltree::Node<'_, '_>>,
    child_objs: Option<roxmltree::Node<'_, '_>>,
    obj_name: &str,
    synonym: &str,
    mode: &str,
) -> Result<Vec<String>, String> {
    let mut lines = Vec::<String>::new();
    let ru_type_name = meta_info_type_ru(md_type);
    let mut header = format!("=== {ru_type_name}: {obj_name}");
    if !synonym.is_empty() && synonym != obj_name {
        header.push_str(&format!(" — \"{synonym}\""));
    }
    header.push_str(" ===");
    lines.push(header);

    if meta_info_is_reference_metadata_type(md_type) {
        let object_presentation = meta_info_ml_child_text(props, "ObjectPresentation");
        let extended_object_presentation =
            meta_info_ml_child_text(props, "ExtendedObjectPresentation");
        let list_presentation = meta_info_ml_child_text(props, "ListPresentation");
        let extended_list_presentation = meta_info_ml_child_text(props, "ExtendedListPresentation");
        let type_presentation = object_presentation
            .as_deref()
            .filter(|value| !value.is_empty())
            .or_else(|| (!synonym.is_empty()).then_some(synonym))
            .unwrap_or(obj_name);
        lines.push(format!("Представление типа: {type_presentation}"));
        if mode == "full" {
            if let Some(value) = object_presentation.filter(|value| !value.is_empty()) {
                lines.push(format!("Представление объекта: {value}"));
            }
            if let Some(value) = extended_object_presentation.filter(|value| !value.is_empty()) {
                lines.push(format!("Расширенное представление объекта: {value}"));
            }
            if let Some(value) = list_presentation.filter(|value| !value.is_empty()) {
                lines.push(format!("Представление списка: {value}"));
            }
            if let Some(value) = extended_list_presentation.filter(|value| !value.is_empty()) {
                lines.push(format!("Расширенное представление списка: {value}"));
            }
        }
    }

    if mode == "brief" {
        meta_info_append_brief(&mut lines, md_type, props, child_objs);
    } else if mode == "overview" || mode == "full" {
        meta_info_append_overview_or_full(&mut lines, md_type, props, child_objs, mode);
    } else {
        return Err(format!(
            "argument -Mode: invalid choice: '{mode}' (choose from 'overview', 'brief', 'full')"
        ));
    }
    Ok(lines)
}

pub(crate) fn meta_info_append_brief(
    lines: &mut Vec<String>,
    md_type: &str,
    props: Option<roxmltree::Node<'_, '_>>,
    child_objs: Option<roxmltree::Node<'_, '_>>,
) {
    let attrs = meta_info_attributes(child_objs, "Attribute", false);
    if !attrs.is_empty() {
        lines.push(format!(
            "Реквизиты ({}): {}",
            attrs.len(),
            attrs
                .iter()
                .map(|attr| attr.name.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }

    if md_type.ends_with("Register") {
        let dims = meta_info_attributes(child_objs, "Dimension", true);
        if !dims.is_empty() {
            lines.push(format!(
                "Измерения ({}): {}",
                dims.len(),
                dims.iter()
                    .map(|attr| attr.name.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        let resources = meta_info_attributes(child_objs, "Resource", false);
        if !resources.is_empty() {
            lines.push(format!(
                "Ресурсы ({}): {}",
                resources.len(),
                resources
                    .iter()
                    .map(|attr| attr.name.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
    }

    let tabular_sections = meta_info_tabular_sections(child_objs);
    if !tabular_sections.is_empty() {
        let parts = tabular_sections
            .iter()
            .map(|section| format!("{}({})", section.name, section.columns.len()))
            .collect::<Vec<_>>();
        lines.push(format!(
            "ТЧ ({}): {}",
            tabular_sections.len(),
            parts.join(", ")
        ));
    }

    if md_type == "Enum" {
        let values = meta_info_enum_values(child_objs);
        if !values.is_empty() {
            lines.push(format!(
                "Значения ({}): {}",
                values.len(),
                values.join(", ")
            ));
        }
    }

    if md_type == "DefinedType" {
        if let Some(type_node) = props.and_then(|node| meta_info_child(node, "Type")) {
            let types = meta_info_children(type_node, "Type")
                .into_iter()
                .map(|node| meta_info_format_single_type(meta_info_inner_text(node), type_node))
                .collect::<Vec<_>>();
            if !types.is_empty() {
                lines.push(format!("Типы ({}): {}", types.len(), types.join(", ")));
            }
        }
    }

    if md_type == "CommonModule" {
        let flags = meta_info_common_module_flags(props);
        if !flags.is_empty() {
            lines.push(flags.join(" | "));
        }
    }

    if md_type == "ScheduledJob" {
        meta_info_append_scheduled_job(lines, props);
    }

    if md_type == "EventSubscription" {
        meta_info_append_event_subscription_brief(lines, props);
    }

    if md_type == "HTTPService" {
        if let Some(root_url) = props.and_then(|node| meta_info_child_text(node, "RootURL")) {
            if !root_url.is_empty() {
                lines.push(format!("Корневой URL: /{root_url}"));
            }
        }
        let endpoints = meta_info_http_endpoints(child_objs);
        if !endpoints.is_empty() {
            let total_methods = endpoints
                .iter()
                .map(|endpoint| endpoint.methods.len())
                .sum::<usize>();
            lines.push(format!(
                "Шаблоны: {} | Методы: {total_methods}",
                endpoints.len()
            ));
        }
    }

    if md_type == "WebService" {
        if let Some(namespace) = props.and_then(|node| meta_info_child_text(node, "Namespace")) {
            if !namespace.is_empty() {
                lines.push(format!("Пространство имён: {namespace}"));
            }
        }
        let operations = meta_info_ws_operations(child_objs);
        if !operations.is_empty() {
            lines.push(format!("Операции: {}", operations.len()));
        }
    }
}

pub(crate) fn meta_info_append_overview_or_full(
    lines: &mut Vec<String>,
    md_type: &str,
    props: Option<roxmltree::Node<'_, '_>>,
    child_objs: Option<roxmltree::Node<'_, '_>>,
    mode: &str,
) {
    if md_type == "Document" {
        meta_info_append_document_header(lines, props);
    }
    if md_type == "Catalog" {
        meta_info_append_catalog_header(lines, props);
    }
    if md_type.ends_with("Register") {
        meta_info_append_register_header(lines, md_type, props);
    }
    if md_type == "Constant" {
        if let Some(type_node) = props.and_then(|node| meta_info_child(node, "Type")) {
            let type_name = meta_info_format_type(type_node);
            if !type_name.is_empty() {
                lines.push(format!("Тип: {type_name}"));
            }
        }
    }
    if md_type == "Report" {
        if let Some(main_dcs) =
            props.and_then(|node| meta_info_child_text(node, "MainDataCompositionSchema"))
        {
            if !main_dcs.is_empty() {
                let dcs_name = main_dcs
                    .rsplit_once(".Template.")
                    .map(|(_, name)| name)
                    .unwrap_or(&main_dcs);
                lines.push(format!("Основная СКД: {dcs_name}"));
            }
        }
    }
    if md_type == "DefinedType" {
        meta_info_append_defined_type(lines, props);
    }
    if md_type == "CommonModule" {
        let flags = meta_info_common_module_flags(props);
        if !flags.is_empty() {
            lines.push(flags.join(" | "));
        }
    }
    if md_type == "ScheduledJob" {
        meta_info_append_scheduled_job(lines, props);
    }
    if md_type == "EventSubscription" {
        meta_info_append_event_subscription(lines, props, mode);
    }
    if md_type == "HTTPService" {
        meta_info_append_http_service(lines, props, child_objs);
    }
    if md_type == "WebService" {
        meta_info_append_web_service(lines, props, child_objs);
    }
    if md_type == "Enum" {
        meta_info_append_enum_values(lines, child_objs);
    }
    if md_type.ends_with("Register") {
        meta_info_append_attribute_section(lines, "Измерения", child_objs, "Dimension", true);
        meta_info_append_attribute_section(lines, "Ресурсы", child_objs, "Resource", false);
    }
    if md_type != "Enum" {
        meta_info_append_attribute_section(lines, "Реквизиты", child_objs, "Attribute", false);
        meta_info_append_tabular_sections(lines, child_objs, mode);
    }
    if mode == "overview" && matches!(md_type, "Report" | "DataProcessor") {
        meta_info_append_simple_children(lines, child_objs);
    }
    if mode == "full" {
        meta_info_append_full_tail(lines, md_type, props, child_objs);
    }
}

pub(crate) fn meta_info_drill_lines(
    md_type: &str,
    child_objs: Option<roxmltree::Node<'_, '_>>,
    drill_name: &str,
    obj_name: &str,
) -> Result<Vec<String>, String> {
    let Some(child_objs) = child_objs else {
        return Err(format!("[ERROR] '{drill_name}' not found in {obj_name}"));
    };
    for (tag, label, is_dimension) in [
        ("Attribute", "Реквизит", false),
        ("Dimension", "Измерение", true),
        ("Resource", "Ресурс", false),
    ] {
        for attr in meta_info_children(child_objs, tag) {
            let Some(props) = meta_info_child(attr, "Properties") else {
                continue;
            };
            let name = meta_info_child_text(props, "Name").unwrap_or_default();
            if name == drill_name {
                return Ok(meta_info_drill_attr_lines(
                    label,
                    &name,
                    props,
                    is_dimension,
                ));
            }
        }
    }

    for section in meta_info_children(child_objs, "TabularSection") {
        let props = meta_info_child(section, "Properties");
        let section_name = props
            .and_then(|node| meta_info_child_text(node, "Name"))
            .unwrap_or_default();
        if section_name == drill_name {
            let section_child_objs = meta_info_child(section, "ChildObjects");
            let columns = meta_info_attributes(section_child_objs, "Attribute", false);
            let mut lines = vec![format!(
                "ТЧ: {section_name} ({} {}):",
                columns.len(),
                meta_info_decline_cols(columns.len())
            )];
            if !columns.is_empty() {
                let width = meta_info_max_name_len(&columns);
                for column in columns {
                    lines.push(meta_info_format_attr_line(&column, width));
                }
            }
            return Ok(lines);
        }
    }

    for value in meta_info_children(child_objs, "EnumValue") {
        let props = meta_info_child(value, "Properties");
        let value_name = props
            .and_then(|node| meta_info_child_text(node, "Name"))
            .unwrap_or_default();
        if value_name == drill_name {
            let mut lines = vec![format!("Значение перечисления: {value_name}")];
            if let Some(synonym) = props
                .and_then(|node| meta_info_child(node, "Synonym"))
                .map(meta_info_ml_text)
                .filter(|value| !value.is_empty())
            {
                lines.push(format!("  Синоним: \"{synonym}\""));
            }
            if let Some(comment) = props
                .and_then(|node| meta_info_child_text(node, "Comment"))
                .filter(|value| !value.is_empty())
            {
                lines.push(format!("  Комментарий: {comment}"));
            }
            return Ok(lines);
        }
    }

    if md_type == "HTTPService" {
        for endpoint in meta_info_http_endpoints(Some(child_objs)) {
            if endpoint.name == drill_name {
                let mut lines = vec![
                    format!("Шаблон URL: {drill_name}"),
                    format!("  Путь: {}", endpoint.template),
                ];
                for method in endpoint.methods {
                    lines.push(format!("  {} → {}", method.http_method, method.handler));
                }
                return Ok(lines);
            }
        }
    }

    if md_type == "WebService" {
        for operation in meta_info_ws_operations(Some(child_objs)) {
            if operation.name == drill_name {
                let mut lines = vec![
                    format!("Операция: {drill_name}"),
                    format!("  Возвращает: {}", operation.return_type),
                ];
                if !operation.proc_name.is_empty() {
                    lines.push(format!("  Процедура: {}", operation.proc_name));
                }
                return Ok(lines);
            }
        }
    }

    Err(format!("[ERROR] '{drill_name}' not found in {obj_name}"))
}

pub(crate) fn meta_info_drill_attr_lines(
    label: &str,
    name: &str,
    props: roxmltree::Node<'_, '_>,
    is_dimension: bool,
) -> Vec<String> {
    let type_name = meta_info_child(props, "Type")
        .map(meta_info_format_type)
        .unwrap_or_default();
    let fill_checking = meta_info_child_text(props, "FillChecking").unwrap_or_default();
    let indexing = meta_info_child_text(props, "Indexing").unwrap_or_default();
    let indexing_ru = match indexing.as_str() {
        "" | "DontIndex" => "нет".to_string(),
        "Index" => "Индекс".to_string(),
        "IndexWithAdditionalOrder" => "Индекс с доп. упорядочиванием".to_string(),
        other => other.to_string(),
    };
    let mut lines = vec![
        format!("{label}: {name}"),
        format!("  Тип: {type_name}"),
        format!(
            "  Обязательный: {}",
            if fill_checking == "ShowError" {
                "да"
            } else {
                "нет"
            }
        ),
        format!("  Индексирование: {indexing_ru}"),
    ];
    if meta_info_child_text(props, "MultiLine").as_deref() == Some("true") {
        lines.push("  Многострочный: да".to_string());
    }
    if let Some(use_value) = meta_info_child_text(props, "Use") {
        if use_value != "ForItem" {
            let use_ru = match use_value.as_str() {
                "ForFolder" => "для папок",
                "ForFolderAndItem" => "для папок и элементов",
                _ => &use_value,
            };
            lines.push(format!("  Использование: {use_ru}"));
        }
    }
    if let Some(fill_value) = meta_info_child(props, "FillValue") {
        let value = meta_info_inner_text(fill_value);
        if meta_info_attr_by_local(fill_value, "nil") != Some("true") && !value.is_empty() {
            let value = match value.as_str() {
                "false" => "Ложь".to_string(),
                "true" => "Истина".to_string(),
                other if other.ends_with(".EmptyRef") => "Пустая ссылка".to_string(),
                other => other.to_string(),
            };
            lines.push(format!("  Значение заполнения: {value}"));
        } else {
            lines.push("  Значение заполнения: —".to_string());
        }
    } else {
        lines.push("  Значение заполнения: —".to_string());
    }
    if is_dimension {
        lines.push(format!(
            "  Ведущее: {}",
            if meta_info_child_text(props, "Master").as_deref() == Some("true") {
                "да"
            } else {
                "нет"
            }
        ));
        lines.push(format!(
            "  Основной отбор: {}",
            if meta_info_child_text(props, "MainFilter").as_deref() == Some("true") {
                "да"
            } else {
                "нет"
            }
        ));
    }
    if let Some(synonym) = meta_info_child(props, "Synonym")
        .map(meta_info_ml_text)
        .filter(|value| !value.is_empty() && value != name)
    {
        lines.push(format!("  Синоним: {synonym}"));
    }
    lines
}

pub(crate) fn meta_info_append_document_header(
    lines: &mut Vec<String>,
    props: Option<roxmltree::Node<'_, '_>>,
) {
    let Some(props) = props else {
        return;
    };
    let mut parts = Vec::new();
    let number_type = meta_info_child_text(props, "NumberType");
    let number_length = meta_info_child_text(props, "NumberLength");
    if let (Some(number_type), Some(number_length)) = (number_type, number_length) {
        let type_name = if number_type == "String" {
            "Строка"
        } else {
            "Число"
        };
        let mut piece = format!("Номер: {type_name}({number_length})");
        if let Some(periodicity) = meta_info_child_text(props, "NumberPeriodicity") {
            piece.push_str(&format!(", {}", meta_info_number_period_ru(&periodicity)));
        }
        if meta_info_child_text(props, "Autonumbering").as_deref() == Some("true") {
            piece.push_str(", авто");
        }
        parts.push(piece);
    }
    if let Some(posting) = meta_info_child_text(props, "Posting") {
        parts.push(format!(
            "Проведение: {}",
            if posting == "Allow" { "да" } else { "нет" }
        ));
    }
    if !parts.is_empty() {
        lines.push(parts.join(" | "));
    }
}

pub(crate) fn meta_info_append_catalog_header(
    lines: &mut Vec<String>,
    props: Option<roxmltree::Node<'_, '_>>,
) {
    let Some(props) = props else {
        return;
    };
    let mut parts = Vec::new();
    if meta_info_child_text(props, "Hierarchical").as_deref() == Some("true") {
        let mut hierarchy_type = if meta_info_child_text(props, "HierarchyType").as_deref()
            == Some("HierarchyFoldersAndItems")
        {
            "группы и элементы".to_string()
        } else {
            "элементы".to_string()
        };
        if meta_info_child_text(props, "LimitLevelCount").as_deref() == Some("true") {
            if let Some(level_count) = meta_info_child_text(props, "LevelCount") {
                hierarchy_type.push_str(&format!(", уровней: {level_count}"));
            }
        } else {
            hierarchy_type.push_str(", без ограничения уровней");
        }
        parts.push(format!("Иерархический: {hierarchy_type}"));
    }
    if let Some(code_length) = meta_info_child_text(props, "CodeLength") {
        if code_length.parse::<i64>().unwrap_or(0) > 0 {
            parts.push(format!("Код({code_length})"));
        }
    }
    if let Some(description_length) = meta_info_child_text(props, "DescriptionLength") {
        if description_length.parse::<i64>().unwrap_or(0) > 0 {
            parts.push(format!("Наименование({description_length})"));
        }
    }
    if !parts.is_empty() {
        lines.push(parts.join(" | "));
    }
}

pub(crate) fn meta_info_append_register_header(
    lines: &mut Vec<String>,
    md_type: &str,
    props: Option<roxmltree::Node<'_, '_>>,
) {
    let Some(props) = props else {
        return;
    };
    let mut parts = Vec::new();
    if md_type == "InformationRegister" {
        if let Some(periodicity) = meta_info_child_text(props, "InformationRegisterPeriodicity") {
            parts.push(format!(
                "Периодичность: {}",
                meta_info_period_ru(&periodicity)
            ));
        }
        if let Some(write_mode) = meta_info_child_text(props, "WriteMode") {
            parts.push(format!("Запись: {}", meta_info_write_mode_ru(&write_mode)));
        }
    }
    if md_type == "AccumulationRegister" {
        if let Some(register_type) = meta_info_child_text(props, "RegisterType") {
            let register_type = match register_type.as_str() {
                "Balances" => "остатки",
                "Turnovers" => "обороты",
                _ => &register_type,
            };
            parts.push(format!("Вид: {register_type}"));
        }
    }
    if !parts.is_empty() {
        lines.push(parts.join(" | "));
    }
}

pub(crate) fn meta_info_append_defined_type(
    lines: &mut Vec<String>,
    props: Option<roxmltree::Node<'_, '_>>,
) {
    let Some(type_node) = props.and_then(|node| meta_info_child(node, "Type")) else {
        return;
    };
    let types = meta_info_children(type_node, "Type")
        .into_iter()
        .map(|node| meta_info_format_single_type(meta_info_inner_text(node), type_node))
        .collect::<Vec<_>>();
    if types.is_empty() {
        return;
    }
    lines.push(format!("Типы ({}):", types.len()));
    for type_name in types {
        lines.push(format!("  {type_name}"));
    }
}

pub(crate) fn meta_info_append_scheduled_job(
    lines: &mut Vec<String>,
    props: Option<roxmltree::Node<'_, '_>>,
) {
    let Some(props) = props else {
        return;
    };
    if let Some(method) =
        meta_info_child_text(props, "MethodName").filter(|value| !value.is_empty())
    {
        lines.push(format!(
            "Метод: {}",
            method.strip_prefix("CommonModule.").unwrap_or(&method)
        ));
    }
    let mut parts = Vec::new();
    parts.push(format!(
        "Использование: {}",
        if meta_info_child_text(props, "Use").as_deref() == Some("true") {
            "да"
        } else {
            "нет"
        }
    ));
    parts.push(format!(
        "Предопределённое: {}",
        if meta_info_child_text(props, "Predefined").as_deref() == Some("true") {
            "да"
        } else {
            "нет"
        }
    ));
    let restart_count = meta_info_child_text(props, "RestartCountOnFailure")
        .and_then(|value| value.parse::<i64>().ok())
        .unwrap_or(0);
    if restart_count > 0 {
        let interval = meta_info_child_text(props, "RestartIntervalOnFailure").unwrap_or_default();
        parts.push(format!(
            "Перезапуск: {restart_count} (через {interval} сек)"
        ));
    }
    lines.push(parts.join(" | "));
}

pub(crate) fn meta_info_append_event_subscription_brief(
    lines: &mut Vec<String>,
    props: Option<roxmltree::Node<'_, '_>>,
) {
    let Some(props) = props else {
        return;
    };
    let mut parts = Vec::new();
    if let Some(event) = meta_info_child_text(props, "Event").filter(|value| !value.is_empty()) {
        parts.push(format!("Событие: {}", meta_info_event_ru(&event)));
    }
    if let Some(handler) = meta_info_child_text(props, "Handler").filter(|value| !value.is_empty())
    {
        parts.push(format!(
            "Обработчик: {}",
            handler.strip_prefix("CommonModule.").unwrap_or(&handler)
        ));
    }
    if let Some(source) = meta_info_child(props, "Source") {
        let source_count = meta_info_children(source, "Type").len();
        if source_count > 0 {
            parts.push(format!("Источники: {source_count}"));
        }
    }
    if !parts.is_empty() {
        lines.push(parts.join(" | "));
    }
}

pub(crate) fn meta_info_append_event_subscription(
    lines: &mut Vec<String>,
    props: Option<roxmltree::Node<'_, '_>>,
    mode: &str,
) {
    let Some(props) = props else {
        return;
    };
    if let Some(event) = meta_info_child_text(props, "Event").filter(|value| !value.is_empty()) {
        lines.push(format!("Событие: {}", meta_info_event_ru(&event)));
    }
    if let Some(handler) = meta_info_child_text(props, "Handler").filter(|value| !value.is_empty())
    {
        lines.push(format!(
            "Обработчик: {}",
            handler.strip_prefix("CommonModule.").unwrap_or(&handler)
        ));
    }
    if let Some(source) = meta_info_child(props, "Source") {
        let source_types = meta_info_children(source, "Type")
            .into_iter()
            .map(|node| meta_info_format_source_type(&meta_info_inner_text(node)))
            .collect::<Vec<_>>();
        if !source_types.is_empty() {
            if mode == "full" {
                lines.push(format!("Источники ({}):", source_types.len()));
                for source_type in source_types {
                    lines.push(format!("  {source_type}"));
                }
            } else {
                lines.push(format!("Источники ({})", source_types.len()));
            }
        }
    }
}

pub(crate) fn meta_info_append_http_service(
    lines: &mut Vec<String>,
    props: Option<roxmltree::Node<'_, '_>>,
    child_objs: Option<roxmltree::Node<'_, '_>>,
) {
    if let Some(root_url) = props.and_then(|node| meta_info_child_text(node, "RootURL")) {
        if !root_url.is_empty() {
            lines.push(format!("Корневой URL: /{root_url}"));
        }
    }
    let endpoints = meta_info_http_endpoints(child_objs);
    if endpoints.is_empty() {
        return;
    }
    lines.push(String::new());
    lines.push(format!("Шаблоны URL ({}):", endpoints.len()));
    for endpoint in endpoints {
        lines.push(format!("  {}", endpoint.template));
        for method in endpoint.methods {
            lines.push(format!(
                "    {:<6} → {}",
                method.http_method, method.handler
            ));
        }
    }
}

pub(crate) fn meta_info_append_web_service(
    lines: &mut Vec<String>,
    props: Option<roxmltree::Node<'_, '_>>,
    child_objs: Option<roxmltree::Node<'_, '_>>,
) {
    if let Some(namespace) = props.and_then(|node| meta_info_child_text(node, "Namespace")) {
        if !namespace.is_empty() {
            lines.push(format!("Пространство имён: {namespace}"));
        }
    }
    let operations = meta_info_ws_operations(child_objs);
    if operations.is_empty() {
        return;
    }
    lines.push(String::new());
    lines.push(format!("Операции ({}):", operations.len()));
    for operation in operations {
        lines.push(format!(
            "  {}({}) → {}",
            operation.name, operation.params, operation.return_type
        ));
    }
}

pub(crate) fn meta_info_append_enum_values(
    lines: &mut Vec<String>,
    child_objs: Option<roxmltree::Node<'_, '_>>,
) {
    let Some(child_objs) = child_objs else {
        return;
    };
    let values = meta_info_children(child_objs, "EnumValue")
        .into_iter()
        .filter_map(|value| {
            let props = meta_info_child(value, "Properties")?;
            let name = meta_info_child_text(props, "Name").unwrap_or_default();
            let synonym = meta_info_child(props, "Synonym")
                .map(meta_info_ml_text)
                .unwrap_or_default();
            Some((name, synonym))
        })
        .collect::<Vec<_>>();
    if values.is_empty() {
        return;
    }
    lines.push(String::new());
    lines.push(format!("Значения ({}):", values.len()));
    let max_len = values
        .iter()
        .map(|(name, _)| name.chars().count())
        .max()
        .unwrap_or(10)
        .max(10)
        + 2;
    for (name, synonym) in values {
        let synonym_text = if !synonym.is_empty() && synonym != name {
            format!("\"{synonym}\"")
        } else {
            String::new()
        };
        lines.push(format!("  {name:<max_len$} {synonym_text}"));
    }
}

pub(crate) fn meta_info_append_attribute_section(
    lines: &mut Vec<String>,
    header: &str,
    child_objs: Option<roxmltree::Node<'_, '_>>,
    child_tag: &str,
    is_dimension: bool,
) {
    let attrs = meta_info_attributes(child_objs, child_tag, is_dimension);
    if attrs.is_empty() {
        return;
    }
    lines.push(String::new());
    lines.push(format!("{header} ({}):", attrs.len()));
    let sorted_attrs = meta_info_sort_attrs_ref_first(attrs);
    let width = meta_info_max_name_len(&sorted_attrs);
    for attr in sorted_attrs {
        lines.push(meta_info_format_attr_line(&attr, width));
    }
}

pub(crate) fn meta_info_append_tabular_sections(
    lines: &mut Vec<String>,
    child_objs: Option<roxmltree::Node<'_, '_>>,
    mode: &str,
) {
    let tabular_sections = meta_info_tabular_sections(child_objs);
    if tabular_sections.is_empty() {
        return;
    }
    if mode == "full" {
        for section in tabular_sections {
            lines.push(String::new());
            lines.push(format!(
                "ТЧ {} ({} {}):",
                section.name,
                section.columns.len(),
                meta_info_decline_cols(section.columns.len())
            ));
            if !section.columns.is_empty() {
                let sorted_cols = meta_info_sort_attrs_ref_first(section.columns);
                let width = meta_info_max_name_len(&sorted_cols);
                for column in sorted_cols {
                    lines.push(meta_info_format_attr_line(&column, width));
                }
            }
        }
    } else {
        lines.push(String::new());
        let parts = tabular_sections
            .iter()
            .map(|section| format!("{}({})", section.name, section.columns.len()))
            .collect::<Vec<_>>();
        lines.push(format!(
            "ТЧ ({}): {}",
            tabular_sections.len(),
            parts.join(", ")
        ));
    }
}

pub(crate) fn meta_info_append_simple_children(
    lines: &mut Vec<String>,
    child_objs: Option<roxmltree::Node<'_, '_>>,
) {
    let forms = meta_info_simple_children(child_objs, "Form");
    if !forms.is_empty() {
        lines.push(format!("Формы: {}", forms.join(", ")));
    }
    let templates = meta_info_simple_children(child_objs, "Template");
    if !templates.is_empty() {
        lines.push(format!("Макеты: {}", templates.join(", ")));
    }
    let commands = meta_info_simple_children(child_objs, "Command");
    if !commands.is_empty() {
        lines.push(format!("Команды: {}", commands.join(", ")));
    }
}

pub(crate) fn meta_info_append_full_tail(
    lines: &mut Vec<String>,
    md_type: &str,
    props: Option<roxmltree::Node<'_, '_>>,
    child_objs: Option<roxmltree::Node<'_, '_>>,
) {
    if md_type == "Document" {
        let Some(props) = props else {
            return;
        };
        let register_records = meta_info_child(props, "RegisterRecords")
            .map(|node| {
                meta_info_children(node, "Item")
                    .into_iter()
                    .map(|item| {
                        let raw = meta_info_inner_text(item);
                        if let Some((prefix, name)) = raw.split_once('.') {
                            format!("{}.{}", meta_info_register_short(prefix), name)
                        } else {
                            raw
                        }
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        if !register_records.is_empty() {
            lines.push(String::new());
            lines.push(format!(
                "Движения ({}): {}",
                register_records.len(),
                register_records.join(", ")
            ));
        }
        let based_on = meta_info_child(props, "BasedOn")
            .map(|node| {
                meta_info_children(node, "Item")
                    .into_iter()
                    .map(|item| {
                        let raw = meta_info_inner_text(item);
                        raw.split_once('.')
                            .map(|(_, name)| name.to_string())
                            .unwrap_or(raw)
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        if !based_on.is_empty() {
            lines.push(format!("Ввод на основании: {}", based_on.join(", ")));
        }
    }
    meta_info_append_simple_children(lines, child_objs);
}

pub(crate) fn meta_info_attributes<'a, 'input>(
    parent_node: Option<roxmltree::Node<'a, 'input>>,
    child_tag: &str,
    is_dimension: bool,
) -> Vec<MetaInfoAttr<'a, 'input>> {
    let Some(parent_node) = parent_node else {
        return Vec::new();
    };
    meta_info_children(parent_node, child_tag)
        .into_iter()
        .filter_map(|attr| {
            let props = meta_info_child(attr, "Properties")?;
            let name = meta_info_child_text(props, "Name").unwrap_or_default();
            let type_name = meta_info_child(props, "Type")
                .map(meta_info_format_type)
                .unwrap_or_default();
            let flags = meta_info_format_flags(props, is_dimension);
            Some(MetaInfoAttr {
                name,
                type_name,
                flags,
                _marker: std::marker::PhantomData,
            })
        })
        .collect()
}

pub(crate) fn meta_info_tabular_sections<'a, 'input>(
    parent_node: Option<roxmltree::Node<'a, 'input>>,
) -> Vec<MetaInfoTabularSection<'a, 'input>> {
    let Some(parent_node) = parent_node else {
        return Vec::new();
    };
    meta_info_children(parent_node, "TabularSection")
        .into_iter()
        .map(|section| {
            let props = meta_info_child(section, "Properties");
            let name = props
                .and_then(|node| meta_info_child_text(node, "Name"))
                .unwrap_or_default();
            let columns =
                meta_info_attributes(meta_info_child(section, "ChildObjects"), "Attribute", false);
            MetaInfoTabularSection { name, columns }
        })
        .collect()
}

pub(crate) fn meta_info_http_endpoints(
    child_objs: Option<roxmltree::Node<'_, '_>>,
) -> Vec<MetaInfoHttpEndpoint> {
    let Some(child_objs) = child_objs else {
        return Vec::new();
    };
    meta_info_children(child_objs, "URLTemplate")
        .into_iter()
        .map(|template| {
            let props = meta_info_child(template, "Properties");
            let name = props
                .and_then(|node| meta_info_child_text(node, "Name"))
                .unwrap_or_default();
            let template_path = props
                .and_then(|node| meta_info_child_text(node, "Template"))
                .unwrap_or_default();
            let methods = meta_info_child(template, "ChildObjects")
                .map(|node| {
                    meta_info_children(node, "Method")
                        .into_iter()
                        .map(|method| {
                            let props = meta_info_child(method, "Properties");
                            MetaInfoHttpMethod {
                                http_method: props
                                    .and_then(|node| meta_info_child_text(node, "HTTPMethod"))
                                    .unwrap_or_default(),
                                handler: props
                                    .and_then(|node| meta_info_child_text(node, "Handler"))
                                    .unwrap_or_default(),
                            }
                        })
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            MetaInfoHttpEndpoint {
                name,
                template: template_path,
                methods,
            }
        })
        .collect()
}

pub(crate) fn meta_info_ws_operations(
    child_objs: Option<roxmltree::Node<'_, '_>>,
) -> Vec<MetaInfoWsOperation> {
    let Some(child_objs) = child_objs else {
        return Vec::new();
    };
    meta_info_children(child_objs, "Operation")
        .into_iter()
        .map(|operation| {
            let props = meta_info_child(operation, "Properties");
            let params = meta_info_child(operation, "ChildObjects")
                .map(|node| {
                    meta_info_children(node, "Parameter")
                        .into_iter()
                        .map(|param| {
                            let props = meta_info_child(param, "Properties");
                            let name = props
                                .and_then(|node| meta_info_child_text(node, "Name"))
                                .unwrap_or_default();
                            let type_name = props
                                .and_then(|node| meta_info_child_text(node, "XDTOValueType"))
                                .filter(|value| !value.is_empty())
                                .unwrap_or_else(|| "?".to_string());
                            let direction = props
                                .and_then(|node| meta_info_child_text(node, "TransferDirection"))
                                .filter(|value| value != "In")
                                .map(|value| format!(" [{}]", value.to_lowercase()))
                                .unwrap_or_default();
                            format!("{name}: {type_name}{direction}")
                        })
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default()
                .join(", ");
            let return_type = props
                .and_then(|node| meta_info_child_text(node, "XDTOReturningValueType"))
                .filter(|value| !value.is_empty())
                .unwrap_or_else(|| "void".to_string());
            MetaInfoWsOperation {
                name: props
                    .and_then(|node| meta_info_child_text(node, "Name"))
                    .unwrap_or_default(),
                params,
                return_type,
                proc_name: props
                    .and_then(|node| meta_info_child_text(node, "ProcedureName"))
                    .unwrap_or_default(),
            }
        })
        .collect()
}

pub(crate) fn meta_info_common_module_flags(props: Option<roxmltree::Node<'_, '_>>) -> Vec<String> {
    let Some(props) = props else {
        return Vec::new();
    };
    let mut flags = Vec::new();
    for (flag_name, flag_label) in [
        ("Global", "Глобальный"),
        ("Server", "Сервер"),
        ("ServerCall", "Вызов сервера"),
        ("ClientManagedApplication", "Клиент управляемое"),
        ("ClientOrdinaryApplication", "Обычный клиент"),
        ("ExternalConnection", "Внешнее соединение"),
        ("Privileged", "Привилегированный"),
    ] {
        if meta_info_child_text(props, flag_name).as_deref() == Some("true") {
            flags.push(flag_label.to_string());
        }
    }
    if let Some(reuse) =
        meta_info_child_text(props, "ReturnValuesReuse").filter(|value| value != "DontUse")
    {
        flags.push(format!(
            "Повторное использование: {}",
            meta_info_reuse_ru(&reuse)
        ));
    }
    flags
}

pub(crate) fn meta_info_format_type(type_node: roxmltree::Node<'_, '_>) -> String {
    let mut types = Vec::new();
    for type_item in meta_info_children(type_node, "Type") {
        types.push(meta_info_format_single_type(
            meta_info_inner_text(type_item),
            type_node,
        ));
    }
    for type_set in meta_info_children(type_node, "TypeSet") {
        let raw = meta_info_inner_text(type_set);
        if let Some(name) = raw.strip_prefix("cfg:DefinedType.") {
            types.push(format!("ОпределяемыйТип.{name}"));
        } else if let Some(name) = raw.strip_prefix("cfg:Characteristic.") {
            types.push(format!("Характеристика.{name}"));
        } else {
            types.push(raw);
        }
    }
    types.join(" | ")
}

pub(crate) fn meta_info_format_single_type(
    raw: String,
    parent_node: roxmltree::Node<'_, '_>,
) -> String {
    match raw.as_str() {
        "xs:string" => {
            let length = meta_info_child(parent_node, "StringQualifiers")
                .and_then(|node| meta_info_child_text(node, "Length"))
                .unwrap_or_default();
            if length.is_empty() {
                "Строка".to_string()
            } else {
                format!("Строка({length})")
            }
        }
        "xs:decimal" => {
            let qualifiers = meta_info_child(parent_node, "NumberQualifiers");
            let digits = qualifiers
                .and_then(|node| meta_info_child_text(node, "Digits"))
                .unwrap_or_default();
            let fraction = qualifiers
                .and_then(|node| meta_info_child_text(node, "FractionDigits"))
                .unwrap_or_else(|| "0".to_string());
            if digits.is_empty() {
                "Число".to_string()
            } else {
                format!("Число({digits},{fraction})")
            }
        }
        "xs:boolean" => "Булево".to_string(),
        "xs:dateTime" => {
            let date_fraction = meta_info_child(parent_node, "DateQualifiers")
                .and_then(|node| meta_info_child_text(node, "DateFractions"));
            match date_fraction.as_deref() {
                Some("Date") => "Дата".to_string(),
                Some("Time") => "Время".to_string(),
                Some("DateTime") => "ДатаВремя".to_string(),
                Some(_) => "Дата".to_string(),
                None => "ДатаВремя".to_string(),
            }
        }
        "v8:ValueStorage" => "ХранилищеЗначения".to_string(),
        "v8:UUID" => "УникальныйИдентификатор".to_string(),
        "v8:Null" => "Null".to_string(),
        _ => meta_info_format_cfg_type(&raw),
    }
}

pub(crate) fn meta_info_format_cfg_type(raw: &str) -> String {
    let normalized = meta_info_normalize_cfg_prefix(raw);
    if let Some(rest) = normalized.strip_prefix("cfg:") {
        if let Some((prefix, name)) = rest.split_once('.') {
            if let Some(ref_type) = meta_info_ref_type_ru(prefix) {
                return format!("{ref_type}.{name}");
            }
            if prefix == "Characteristic" {
                return format!("Характеристика.{name}");
            }
            if prefix == "DefinedType" {
                return format!("ОпределяемыйТип.{name}");
            }
        }
        return rest.to_string();
    }
    normalized
}

pub(crate) fn meta_info_format_flags(props: roxmltree::Node<'_, '_>, is_dimension: bool) -> String {
    let mut flags = Vec::new();
    if meta_info_child_text(props, "FillChecking").as_deref() == Some("ShowError") {
        flags.push("обязательный");
    }
    if let Some(indexing) = meta_info_child_text(props, "Indexing") {
        match indexing.as_str() {
            "Index" => flags.push("индекс"),
            "IndexWithAdditionalOrder" => flags.push("индекс+доп"),
            _ => {}
        }
    }
    if is_dimension && meta_info_child_text(props, "Master").as_deref() == Some("true") {
        flags.push("ведущее");
    }
    if meta_info_child_text(props, "MultiLine").as_deref() == Some("true") {
        flags.push("многострочный");
    }
    if let Some(use_value) = meta_info_child_text(props, "Use") {
        match use_value.as_str() {
            "ForFolder" => flags.push("для папок"),
            "ForFolderAndItem" => flags.push("для папок и элементов"),
            _ => {}
        }
    }
    if flags.is_empty() {
        String::new()
    } else {
        format!("  [{}]", flags.join(", "))
    }
}

pub(crate) fn meta_info_sort_attrs_ref_first<'a, 'input>(
    attrs: Vec<MetaInfoAttr<'a, 'input>>,
) -> Vec<MetaInfoAttr<'a, 'input>> {
    let mut refs = Vec::new();
    let mut prims = Vec::new();
    for attr in attrs {
        if meta_info_type_is_reference(&attr.type_name) {
            refs.push(attr);
        } else {
            prims.push(attr);
        }
    }
    refs.extend(prims);
    refs
}

pub(crate) fn meta_info_type_is_reference(type_name: &str) -> bool {
    type_name.contains("Ссылка.")
        || type_name.contains("Характеристика.")
        || type_name.contains("ОпределяемыйТип.")
        || type_name.contains("ПланСчетовСсылка")
        || type_name.contains("ПВХСсылка")
        || type_name.contains("ПВРСсылка")
}

pub(crate) fn meta_info_format_attr_line(attr: &MetaInfoAttr<'_, '_>, width: usize) -> String {
    format!("  {:<width$} {}{}", attr.name, attr.type_name, attr.flags)
}

pub(crate) fn meta_info_max_name_len(attrs: &[MetaInfoAttr<'_, '_>]) -> usize {
    let max_len = attrs
        .iter()
        .map(|attr| attr.name.chars().count())
        .max()
        .unwrap_or(10)
        .max(10);
    (max_len + 2).min(40)
}

pub(crate) fn meta_info_simple_children(
    parent_node: Option<roxmltree::Node<'_, '_>>,
    tag: &str,
) -> Vec<String> {
    let Some(parent_node) = parent_node else {
        return Vec::new();
    };
    meta_info_children(parent_node, tag)
        .into_iter()
        .map(meta_info_inner_text)
        .collect()
}

pub(crate) fn meta_info_enum_values(parent_node: Option<roxmltree::Node<'_, '_>>) -> Vec<String> {
    let Some(parent_node) = parent_node else {
        return Vec::new();
    };
    meta_info_children(parent_node, "EnumValue")
        .into_iter()
        .filter_map(|value| {
            meta_info_child(value, "Properties")
                .and_then(|props| meta_info_child_text(props, "Name"))
        })
        .collect()
}

pub(crate) fn meta_info_paginate(lines: Vec<String>, args: &Map<String, Value>) -> String {
    let total_lines = lines.len();
    let offset = int_arg(args, &["offset", "Offset"]).unwrap_or(0).max(0) as usize;
    let limit = int_arg(args, &["limit", "Limit"]).unwrap_or(150).max(0) as usize;
    if offset >= total_lines && offset > 0 {
        return format!(
            "[INFO] Offset {offset} exceeds total lines ({total_lines}). Nothing to show."
        );
    }
    let mut out_lines = if offset > 0 {
        lines.into_iter().skip(offset).collect::<Vec<_>>()
    } else {
        lines
    };
    if limit > 0 && out_lines.len() > limit {
        let mut shown = out_lines.drain(..limit).collect::<Vec<_>>();
        shown.push(String::new());
        shown.push(format!(
            "[ОБРЕЗАНО] Показано {limit} из {total_lines} строк. Используйте -Offset {} для продолжения.",
            offset + limit
        ));
        out_lines = shown;
    }
    out_lines.join("\n")
}

pub(crate) fn meta_info_child<'a, 'input>(
    node: roxmltree::Node<'a, 'input>,
    local_name: &str,
) -> Option<roxmltree::Node<'a, 'input>> {
    node.children()
        .find(|child| role_info_element(*child, local_name, None))
}

pub(crate) fn meta_info_children<'a, 'input>(
    node: roxmltree::Node<'a, 'input>,
    local_name: &str,
) -> Vec<roxmltree::Node<'a, 'input>> {
    node.children()
        .filter(|child| role_info_element(*child, local_name, None))
        .collect()
}

pub(crate) fn meta_info_child_text(
    node: roxmltree::Node<'_, '_>,
    local_name: &str,
) -> Option<String> {
    meta_info_child(node, local_name).map(meta_info_inner_text)
}

pub(crate) fn meta_info_inner_text(node: roxmltree::Node<'_, '_>) -> String {
    node.text().unwrap_or("").to_string()
}

pub(crate) fn meta_info_ml_text(node: roxmltree::Node<'_, '_>) -> String {
    let value = multilang_text(node);
    if value.is_empty() {
        node.text().unwrap_or("").trim().to_string()
    } else {
        value
    }
}

pub(crate) fn meta_info_ml_child_text(
    node: Option<roxmltree::Node<'_, '_>>,
    local_name: &str,
) -> Option<String> {
    node.and_then(|node| meta_info_child(node, local_name))
        .map(meta_info_ml_text)
}

pub(crate) fn meta_info_attr_by_local<'a>(
    node: roxmltree::Node<'a, '_>,
    local_name: &str,
) -> Option<&'a str> {
    node.attributes()
        .find(|attr| attr.name() == local_name)
        .map(|attr| attr.value())
}

pub(crate) fn meta_info_normalize_cfg_prefix(raw: &str) -> String {
    let Some((prefix, rest)) = raw.split_once(':') else {
        return raw.to_string();
    };
    if prefix.starts_with('d')
        && prefix[1..]
            .chars()
            .all(|ch| ch.is_ascii_digit() || ch == 'p')
    {
        format!("cfg:{rest}")
    } else {
        raw.to_string()
    }
}

pub(crate) fn meta_info_format_source_type(raw: &str) -> String {
    let normalized = meta_info_normalize_cfg_prefix(raw);
    let Some(rest) = normalized.strip_prefix("cfg:") else {
        return normalized;
    };
    let Some((prefix, name)) = rest.split_once('.') else {
        return rest.to_string();
    };
    if let Some(object_type) = meta_info_object_type_ru(prefix) {
        format!("{object_type}.{name}")
    } else {
        rest.to_string()
    }
}

pub(crate) fn meta_info_type_ru(md_type: &str) -> String {
    match md_type {
        "Catalog" => "Справочник",
        "Document" => "Документ",
        "Enum" => "Перечисление",
        "Constant" => "Константа",
        "InformationRegister" => "Регистр сведений",
        "AccumulationRegister" => "Регистр накопления",
        "AccountingRegister" => "Регистр бухгалтерии",
        "CalculationRegister" => "Регистр расчёта",
        "ChartOfAccounts" => "План счетов",
        "ChartOfCharacteristicTypes" => "План видов характеристик",
        "ChartOfCalculationTypes" => "План видов расчёта",
        "BusinessProcess" => "Бизнес-процесс",
        "Task" => "Задача",
        "ExchangePlan" => "План обмена",
        "DocumentJournal" => "Журнал документов",
        "Report" => "Отчёт",
        "DataProcessor" => "Обработка",
        "DefinedType" => "Определяемый тип",
        "CommonModule" => "Общий модуль",
        "ScheduledJob" => "Регламентное задание",
        "EventSubscription" => "Подписка на событие",
        "HTTPService" => "HTTP-сервис",
        "WebService" => "Веб-сервис",
        _ => md_type,
    }
    .to_string()
}

pub(crate) fn meta_info_is_reference_metadata_type(md_type: &str) -> bool {
    matches!(
        md_type,
        "Catalog"
            | "Document"
            | "Enum"
            | "ChartOfAccounts"
            | "ChartOfCharacteristicTypes"
            | "ChartOfCalculationTypes"
            | "ExchangePlan"
            | "BusinessProcess"
            | "Task"
    )
}

pub(crate) fn meta_info_ref_type_ru(prefix: &str) -> Option<&'static str> {
    match prefix {
        "CatalogRef" => Some("СправочникСсылка"),
        "DocumentRef" => Some("ДокументСсылка"),
        "EnumRef" => Some("ПеречислениеСсылка"),
        "ChartOfAccountsRef" => Some("ПланСчетовСсылка"),
        "ChartOfCharacteristicTypesRef" => Some("ПВХСсылка"),
        "ChartOfCalculationTypesRef" => Some("ПВРСсылка"),
        "ExchangePlanRef" => Some("ПланОбменаСсылка"),
        "BusinessProcessRef" => Some("БизнесПроцессСсылка"),
        "TaskRef" => Some("ЗадачаСсылка"),
        _ => None,
    }
}

pub(crate) fn meta_info_object_type_ru(prefix: &str) -> Option<&'static str> {
    match prefix {
        "CatalogObject" => Some("СправочникОбъект"),
        "DocumentObject" => Some("ДокументОбъект"),
        "ChartOfAccountsObject" => Some("ПланСчетовОбъект"),
        "ChartOfCharacteristicTypesObject" => Some("ПВХОбъект"),
        "BusinessProcessObject" => Some("БизнесПроцессОбъект"),
        "TaskObject" => Some("ЗадачаОбъект"),
        "ExchangePlanObject" => Some("ПланОбменаОбъект"),
        "InformationRegisterRecordSet" => Some("НаборЗаписейРС"),
        "AccumulationRegisterRecordSet" => Some("НаборЗаписейРН"),
        "AccountingRegisterRecordSet" => Some("НаборЗаписейРБ"),
        _ => None,
    }
}

pub(crate) fn meta_info_period_ru(value: &str) -> &str {
    match value {
        "Nonperiodical" => "Непериодический",
        "Day" => "День",
        "Month" => "Месяц",
        "Quarter" => "Квартал",
        "Year" => "Год",
        "Second" => "Секунда",
        _ => value,
    }
}

pub(crate) fn meta_info_write_mode_ru(value: &str) -> &str {
    match value {
        "Independent" => "независимая",
        "RecorderSubordinate" => "подчинение регистратору",
        _ => value,
    }
}

pub(crate) fn meta_info_reuse_ru(value: &str) -> &str {
    match value {
        "DontUse" => "нет",
        "DuringRequest" => "на время вызова",
        "DuringSession" => "на время сеанса",
        _ => value,
    }
}

pub(crate) fn meta_info_event_ru(value: &str) -> &str {
    match value {
        "BeforeWrite" => "ПередЗаписью",
        "OnWrite" => "ПриЗаписи",
        "AfterWrite" => "ПослеЗаписи",
        "BeforeDelete" => "ПередУдалением",
        "Posting" => "ОбработкаПроведения",
        "UndoPosting" => "ОбработкаУдаленияПроведения",
        "OnReadAtServer" => "ПриЧтенииНаСервере",
        "FillCheckProcessing" => "ОбработкаПроверкиЗаполнения",
        _ => value,
    }
}

pub(crate) fn meta_info_number_period_ru(value: &str) -> &str {
    match value {
        "Year" => "по году",
        "Quarter" => "по кварталу",
        "Month" => "по месяцу",
        "Day" => "по дню",
        "WholeCatalog" => "сквозная",
        _ => value,
    }
}

pub(crate) fn meta_info_register_short(value: &str) -> &str {
    match value {
        "AccumulationRegister" => "РН",
        "AccountingRegister" => "РБ",
        "CalculationRegister" => "РР",
        "InformationRegister" => "РС",
        _ => value,
    }
}

pub(crate) fn meta_info_decline_cols(n: usize) -> &'static str {
    let m = n % 10;
    let h = n % 100;
    if (11..=19).contains(&h) {
        "колонок"
    } else if m == 1 {
        "колонка"
    } else if (2..=4).contains(&m) {
        "колонки"
    } else {
        "колонок"
    }
}

pub(crate) struct MetaRemoveError {
    pub(crate) stdout: String,
    pub(crate) stderr: String,
    pub(crate) message: String,
}

pub(crate) fn meta_remove_stdout_error(message: String) -> MetaRemoveError {
    MetaRemoveError {
        stdout: format!("{message}\n"),
        stderr: String::new(),
        message,
    }
}

pub(crate) fn remove_metadata_object(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> AdapterOutcome {
    let result = (|| -> Result<(String, Vec<String>, Vec<String>), MetaRemoveError> {
        let config_dir_raw = required_string(args, &["configDir", "ConfigDir"], "ConfigDir")
            .map_err(|err| meta_remove_stdout_error(format!("[ERROR] {err}")))?;
        let object = required_string(args, &["object", "Object"], "Object")
            .map_err(|err| meta_remove_stdout_error(format!("[ERROR] {err}")))?;

        let config_dir_display = PathBuf::from(config_dir_raw);
        let config_dir = absolutize(config_dir_display.clone(), &context.cwd);
        if !config_dir.is_dir() {
            return Err(meta_remove_stdout_error(format!(
                "[ERROR] Config directory not found: {}",
                config_dir.display()
            )));
        }

        let config_xml = config_dir.join("Configuration.xml");
        if !config_xml.is_file() {
            return Err(meta_remove_stdout_error(format!(
                "[ERROR] Configuration.xml not found in: {}",
                config_dir.display()
            )));
        }

        let Some((obj_type, obj_name)) = object.split_once('.') else {
            return Err(meta_remove_stdout_error(format!(
                "[ERROR] Invalid object format '{object}'. Expected: Type.Name (e.g. Catalog.Товары)"
            )));
        };
        if obj_type.is_empty() || obj_name.is_empty() {
            return Err(meta_remove_stdout_error(format!(
                "[ERROR] Invalid object format '{object}'. Expected: Type.Name (e.g. Catalog.Товары)"
            )));
        }
        let Some(type_plural) = meta_remove_type_plural(obj_type) else {
            return Err(meta_remove_stdout_error(format!(
                "[ERROR] Unknown type '{obj_type}'. Supported: {}",
                meta_remove_supported_types().join(", ")
            )));
        };

        let dry_run = bool_arg(args, &["DryRun"]);
        let keep_files = bool_arg(args, &["KeepFiles", "keepFiles"]);
        let force = bool_arg(args, &["Force", "force"]);

        let type_dir = config_dir.join(type_plural);
        let obj_xml = type_dir.join(format!("{obj_name}.xml"));
        let obj_dir = type_dir.join(obj_name);
        let has_xml = obj_xml.is_file();
        let has_dir = obj_dir.is_dir();

        let mut stdout = String::new();
        stdout.push_str(&format!("=== meta-remove: {obj_type}.{obj_name} ===\n\n"));
        if dry_run {
            stdout.push_str("[DRY-RUN] No changes will be made\n\n");
        }

        let mut changes = Vec::new();
        let mut artifacts = vec![config_xml.display().to_string()];
        let mut actions = 0usize;

        if !has_xml && !has_dir {
            if !metadata_object_registered(&config_xml, obj_type, obj_name) {
                stdout.push_str(&format!(
                    "[ERROR] Object not found: {type_plural}/{obj_name}.xml and not registered in Configuration.xml\n"
                ));
                return Err(MetaRemoveError {
                    message: stdout.trim().to_string(),
                    stdout,
                    stderr: String::new(),
                });
            }
            stdout.push_str(&format!(
                "[WARN]  Object files not found: {type_plural}/{obj_name}.xml\n"
            ));
            stdout.push_str("        Proceeding with deregistration only...\n");
        } else {
            if has_xml {
                stdout.push_str(&format!("[FOUND] {type_plural}/{obj_name}.xml\n"));
                artifacts.push(obj_xml.display().to_string());
            }
            if has_dir {
                let file_count = count_files_recursive(&obj_dir);
                stdout.push_str(&format!(
                    "[FOUND] {type_plural}/{obj_name}/ ({file_count} files)\n"
                ));
                artifacts.push(obj_dir.display().to_string());
            }
        }

        stdout.push('\n');
        stdout.push_str("--- Reference check ---\n");
        let references = meta_remove_references(
            &config_dir,
            obj_type,
            obj_name,
            type_plural,
            &obj_xml,
            &obj_dir,
            has_xml,
            has_dir,
        );
        if references.is_empty() {
            stdout.push_str("[OK]    No references found\n");
        } else {
            stdout.push_str(&format!(
                "[WARN]  Found {} reference(s) to {obj_type}.{obj_name}:\n\n",
                references.len()
            ));
            for (index, reference) in references.iter().take(20).enumerate() {
                stdout.push_str(&format!("        {}\n", reference.file));
                stdout.push_str(&format!("          pattern: {}\n", reference.pattern));
                if index == 19 && references.len() > 20 {
                    stdout.push_str(&format!("        ... and {} more\n", references.len() - 20));
                }
            }
            stdout.push('\n');
            if !force {
                stdout.push_str(&format!(
                    "[ERROR] Cannot remove: object has {} reference(s).\n",
                    references.len()
                ));
                stdout.push_str("        Use -Force to remove anyway, or fix references first.\n");
                return Err(MetaRemoveError {
                    message: stdout.trim().to_string(),
                    stdout,
                    stderr: String::new(),
                });
            }
            stdout.push_str("[WARN]  -Force specified, proceeding despite references\n");
        }

        stdout.push('\n');
        stdout.push_str("--- Configuration.xml ---\n");
        let config_text = read_utf8_sig(&config_xml).map_err(meta_remove_stdout_error)?;
        let (next_config_text, removed_from_config) =
            remove_metadata_child_text_with_flag(&config_text, obj_type, obj_name);
        if removed_from_config {
            stdout.push_str(&format!(
                "[OK]    Removed <{obj_type}>{obj_name}</{obj_type}> from ChildObjects\n"
            ));
            actions += 1;
            if !dry_run {
                write_utf8_bom(&config_xml, &ensure_trailing_newline(next_config_text))
                    .map_err(meta_remove_stdout_error)?;
                stdout.push_str("[OK]    Configuration.xml saved\n");
                changes.push(format!("updated {}", config_xml.display()));
            }
        } else {
            stdout.push_str(&format!(
                "[WARN]  <{obj_type}>{obj_name}</{obj_type}> not found in ChildObjects\n"
            ));
        }

        stdout.push('\n');
        stdout.push_str("--- Subsystems ---\n");
        let subsystems_dir = config_dir.join("Subsystems");
        let mut subsystems_cleaned = 0usize;
        if subsystems_dir.is_dir() {
            remove_object_from_subsystems(
                &subsystems_dir,
                obj_type,
                obj_name,
                dry_run,
                &mut stdout,
                &mut subsystems_cleaned,
                &mut changes,
            )
            .map_err(meta_remove_stdout_error)?;
            if subsystems_cleaned == 0 {
                stdout.push_str("[OK]    Not referenced in any subsystem\n");
            }
        } else {
            stdout.push_str("[OK]    No Subsystems directory\n");
        }

        stdout.push('\n');
        stdout.push_str("--- Files ---\n");
        if !keep_files {
            if has_dir && !dry_run {
                fs::remove_dir_all(&obj_dir).map_err(|err| {
                    meta_remove_stdout_error(format!(
                        "failed to remove {}: {err}",
                        obj_dir.display()
                    ))
                })?;
                stdout.push_str(&format!(
                    "[OK]    Deleted directory: {type_plural}/{obj_name}/\n"
                ));
                changes.push(format!("removed directory {}", obj_dir.display()));
                actions += 1;
            } else if has_dir {
                stdout.push_str(&format!(
                    "[DRY]   Would delete directory: {type_plural}/{obj_name}/\n"
                ));
                actions += 1;
            }

            if has_xml && !dry_run {
                fs::remove_file(&obj_xml).map_err(|err| {
                    meta_remove_stdout_error(format!(
                        "failed to remove {}: {err}",
                        obj_xml.display()
                    ))
                })?;
                stdout.push_str(&format!(
                    "[OK]    Deleted file: {type_plural}/{obj_name}.xml\n"
                ));
                changes.push(format!("removed file {}", obj_xml.display()));
                actions += 1;
            } else if has_xml {
                stdout.push_str(&format!(
                    "[DRY]   Would delete file: {type_plural}/{obj_name}.xml\n"
                ));
                actions += 1;
            }

            if !has_xml && !has_dir {
                stdout.push_str("[OK]    No files to delete\n");
            }
        } else {
            stdout.push_str("[SKIP]  File deletion skipped (-KeepFiles)\n");
        }

        stdout.push('\n');
        let total_actions = actions + subsystems_cleaned;
        if dry_run {
            stdout.push_str(&format!(
                "=== Dry run complete: {total_actions} actions would be performed ===\n"
            ));
        } else {
            stdout.push_str(&format!(
                "=== Done: {total_actions} actions performed ({subsystems_cleaned} subsystem references removed) ===\n"
            ));
        }

        Ok((stdout, changes, artifacts))
    })();

    match result {
        Ok((stdout, changes, artifacts)) => AdapterOutcome {
            ok: true,
            summary: "unica.meta.remove completed with native metadata remover".to_string(),
            changes,
            warnings: Vec::new(),
            errors: Vec::new(),
            artifacts,
            stdout: Some(stdout),
            stderr: Some(String::new()),
            command: None,
        },
        Err(error) => AdapterOutcome {
            ok: false,
            summary: "unica.meta.remove failed in native metadata remover".to_string(),
            changes: Vec::new(),
            warnings: Vec::new(),
            errors: if error.message.is_empty() {
                Vec::new()
            } else {
                vec![error.message]
            },
            artifacts: Vec::new(),
            stdout: Some(error.stdout),
            stderr: Some(error.stderr),
            command: None,
        },
    }
}

pub(crate) fn remove_metadata_child_text_lxml(
    xml_text: &str,
    local_name: &str,
    item_name: &str,
) -> String {
    let plain = format!("<{local_name}>{item_name}</{local_name}>");
    let prefixed = format!("<md:{local_name}>{item_name}</md:{local_name}>");
    for (open, target) in [
        ("<ChildObjects>", plain.as_str()),
        ("<md:ChildObjects>", prefixed.as_str()),
    ] {
        let Some(open_idx) = xml_text.find(open) else {
            continue;
        };
        let after_open = open_idx + open.len();
        if !xml_text[after_open..].starts_with('\n') {
            continue;
        }
        let child_indent_start = after_open + 1;
        let child_start = child_indent_start
            + xml_text[child_indent_start..]
                .chars()
                .take_while(|ch| *ch == '\t' || *ch == ' ')
                .map(char::len_utf8)
                .sum::<usize>();
        if !xml_text[child_start..].starts_with(target) {
            continue;
        }
        let after_child = child_start + target.len();
        if !xml_text[after_child..].starts_with('\n') {
            continue;
        }
        let next_line_start = after_child + 1;
        let next_content_start = next_line_start
            + xml_text[next_line_start..]
                .chars()
                .take_while(|ch| *ch == '\t' || *ch == ' ')
                .map(char::len_utf8)
                .sum::<usize>();
        let mut result = String::with_capacity(xml_text.len());
        result.push_str(&xml_text[..after_open]);
        result.push_str(&xml_text[next_content_start..]);
        return result;
    }
    remove_metadata_child_text(xml_text, local_name, item_name)
}

pub(crate) fn remove_metadata_child_text(
    xml_text: &str,
    local_name: &str,
    item_name: &str,
) -> String {
    remove_metadata_child_text_with_flag(xml_text, local_name, item_name).0
}

pub(crate) fn remove_metadata_child_text_with_flag(
    xml_text: &str,
    local_name: &str,
    item_name: &str,
) -> (String, bool) {
    let plain = format!("<{local_name}>{item_name}</{local_name}>");
    let prefixed = format!("<md:{local_name}>{item_name}</md:{local_name}>");
    let mut removed = false;
    let mut result = String::with_capacity(xml_text.len());
    for line in xml_text.split_inclusive('\n') {
        let trimmed = line.trim();
        if !removed && (trimmed == plain || trimmed == prefixed) {
            removed = true;
            continue;
        }
        result.push_str(line);
    }
    if removed {
        (result, true)
    } else if let Some(index) = xml_text.find(&plain) {
        let mut result = xml_text.to_string();
        result.replace_range(index..index + plain.len(), "");
        (result, true)
    } else if let Some(index) = xml_text.find(&prefixed) {
        let mut result = xml_text.to_string();
        result.replace_range(index..index + prefixed.len(), "");
        (result, true)
    } else {
        (xml_text.to_string(), false)
    }
}

pub(crate) struct MetaRemoveReference {
    pub(crate) file: String,
    pub(crate) pattern: String,
}

pub(crate) fn metadata_object_registered(
    config_xml: &Path,
    obj_type: &str,
    obj_name: &str,
) -> bool {
    let Ok(text) = read_utf8_sig(config_xml) else {
        return false;
    };
    text.contains(&format!("<{obj_type}>{obj_name}</{obj_type}>"))
        || text.contains(&format!("<md:{obj_type}>{obj_name}</md:{obj_type}>"))
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn meta_remove_references(
    config_dir: &Path,
    obj_type: &str,
    obj_name: &str,
    type_plural: &str,
    obj_xml: &Path,
    obj_dir: &Path,
    has_xml: bool,
    has_dir: bool,
) -> Vec<MetaRemoveReference> {
    let patterns = meta_remove_search_patterns(obj_type, obj_name, type_plural);
    let mut references = Vec::new();
    let mut already_found = HashSet::new();
    let files = metadata_files_recursive(config_dir);

    for file in files.iter().filter(|file| {
        matches!(
            file.extension().and_then(|ext| ext.to_str()).map(str::to_ascii_lowercase),
            Some(ext) if ext == "xml" || ext == "bsl"
        )
    }) {
        if meta_remove_should_skip_file(file, config_dir, obj_xml, obj_dir, has_xml, has_dir) {
            continue;
        }
        let Ok(content) = read_utf8_sig(file) else {
            continue;
        };
        let rel = relative_display(file, config_dir);
        for pattern in &patterns {
            if content.contains(pattern) {
                already_found.insert(rel.clone());
                references.push(MetaRemoveReference {
                    file: rel,
                    pattern: pattern.clone(),
                });
                break;
            }
        }
    }

    let type_name_ref = format!("{obj_type}.{obj_name}");
    for file in files.iter().filter(|file| {
        file.extension()
            .and_then(|ext| ext.to_str())
            .is_some_and(|ext| ext.eq_ignore_ascii_case("xml"))
    }) {
        if meta_remove_should_skip_file(file, config_dir, obj_xml, obj_dir, has_xml, has_dir) {
            continue;
        }
        let rel = relative_display(file, config_dir);
        if already_found.contains(&rel) {
            continue;
        }
        let Ok(content) = read_utf8_sig(file) else {
            continue;
        };
        if content.contains(&type_name_ref) {
            references.push(MetaRemoveReference {
                file: rel,
                pattern: type_name_ref.clone(),
            });
        }
    }

    references
}

pub(crate) fn metadata_files_recursive(root: &Path) -> Vec<PathBuf> {
    let mut result = Vec::new();
    let Ok(entries) = fs::read_dir(root) else {
        return result;
    };
    let mut entries = entries.filter_map(Result::ok).collect::<Vec<_>>();
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        let path = entry.path();
        if path.is_dir() {
            result.extend(metadata_files_recursive(&path));
        } else if path.is_file() {
            result.push(path);
        }
    }
    result
}

pub(crate) fn meta_remove_should_skip_file(
    file: &Path,
    config_dir: &Path,
    obj_xml: &Path,
    obj_dir: &Path,
    has_xml: bool,
    has_dir: bool,
) -> bool {
    if has_xml && file == obj_xml {
        return true;
    }
    if has_dir && (file == obj_dir || file.starts_with(obj_dir)) {
        return true;
    }
    let rel = relative_display(file, config_dir);
    rel == "Configuration.xml" || rel == "ConfigDumpInfo.xml" || rel.starts_with("Subsystems")
}

pub(crate) fn meta_remove_search_patterns(
    obj_type: &str,
    obj_name: &str,
    type_plural: &str,
) -> Vec<String> {
    let mut patterns = Vec::new();
    if let Some(ref_names) = meta_remove_type_ref_names(obj_type) {
        patterns.extend(ref_names.iter().map(|name| format!("{name}.{obj_name}")));
    }
    if let Some(manager) = meta_remove_ru_manager(obj_type) {
        patterns.push(format!("{manager}.{obj_name}"));
    }
    patterns.push(format!("{type_plural}.{obj_name}"));
    if obj_type == "CommonModule" {
        patterns.push(format!("{obj_name}."));
        patterns.push(format!("<Handler>{obj_name}."));
        patterns.push(format!("<MethodName>{obj_name}."));
    }
    patterns
}

pub(crate) fn meta_remove_supported_types() -> &'static [&'static str] {
    &[
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
        "Role",
        "Subsystem",
        "CommonForm",
        "CommonTemplate",
        "CommonPicture",
        "CommonAttribute",
        "SessionParameter",
        "FunctionalOption",
        "FunctionalOptionsParameter",
        "Sequence",
        "FilterCriterion",
        "SettingsStorage",
        "XDTOPackage",
        "WSReference",
        "StyleItem",
        "Language",
    ]
}

pub(crate) fn meta_remove_type_plural(obj_type: &str) -> Option<&'static str> {
    match obj_type {
        "Catalog" => Some("Catalogs"),
        "Document" => Some("Documents"),
        "Enum" => Some("Enums"),
        "Constant" => Some("Constants"),
        "InformationRegister" => Some("InformationRegisters"),
        "AccumulationRegister" => Some("AccumulationRegisters"),
        "AccountingRegister" => Some("AccountingRegisters"),
        "CalculationRegister" => Some("CalculationRegisters"),
        "ChartOfAccounts" => Some("ChartsOfAccounts"),
        "ChartOfCharacteristicTypes" => Some("ChartsOfCharacteristicTypes"),
        "ChartOfCalculationTypes" => Some("ChartsOfCalculationTypes"),
        "BusinessProcess" => Some("BusinessProcesses"),
        "Task" => Some("Tasks"),
        "ExchangePlan" => Some("ExchangePlans"),
        "DocumentJournal" => Some("DocumentJournals"),
        "Report" => Some("Reports"),
        "DataProcessor" => Some("DataProcessors"),
        "CommonModule" => Some("CommonModules"),
        "ScheduledJob" => Some("ScheduledJobs"),
        "EventSubscription" => Some("EventSubscriptions"),
        "HTTPService" => Some("HTTPServices"),
        "WebService" => Some("WebServices"),
        "DefinedType" => Some("DefinedTypes"),
        "Role" => Some("Roles"),
        "Subsystem" => Some("Subsystems"),
        "CommonForm" => Some("CommonForms"),
        "CommonTemplate" => Some("CommonTemplates"),
        "CommonPicture" => Some("CommonPictures"),
        "CommonAttribute" => Some("CommonAttributes"),
        "SessionParameter" => Some("SessionParameters"),
        "FunctionalOption" => Some("FunctionalOptions"),
        "FunctionalOptionsParameter" => Some("FunctionalOptionsParameters"),
        "Sequence" => Some("Sequences"),
        "FilterCriterion" => Some("FilterCriteria"),
        "SettingsStorage" => Some("SettingsStorages"),
        "XDTOPackage" => Some("XDTOPackages"),
        "WSReference" => Some("WSReferences"),
        "StyleItem" => Some("StyleItems"),
        "Language" => Some("Languages"),
        _ => None,
    }
}

pub(crate) fn meta_remove_type_ref_names(obj_type: &str) -> Option<&'static [&'static str]> {
    match obj_type {
        "Catalog" => Some(&["CatalogRef", "CatalogObject"]),
        "Document" => Some(&["DocumentRef", "DocumentObject"]),
        "Enum" => Some(&["EnumRef"]),
        "ExchangePlan" => Some(&["ExchangePlanRef", "ExchangePlanObject"]),
        "ChartOfAccounts" => Some(&["ChartOfAccountsRef", "ChartOfAccountsObject"]),
        "ChartOfCharacteristicTypes" => Some(&[
            "ChartOfCharacteristicTypesRef",
            "ChartOfCharacteristicTypesObject",
        ]),
        "ChartOfCalculationTypes" => Some(&[
            "ChartOfCalculationTypesRef",
            "ChartOfCalculationTypesObject",
        ]),
        "BusinessProcess" => Some(&["BusinessProcessRef", "BusinessProcessObject"]),
        "Task" => Some(&["TaskRef", "TaskObject"]),
        _ => None,
    }
}

pub(crate) fn meta_remove_ru_manager(obj_type: &str) -> Option<&'static str> {
    match obj_type {
        "Catalog" => Some("Справочники"),
        "Document" => Some("Документы"),
        "Enum" => Some("Перечисления"),
        "Constant" => Some("Константы"),
        "InformationRegister" => Some("РегистрыСведений"),
        "AccumulationRegister" => Some("РегистрыНакопления"),
        "AccountingRegister" => Some("РегистрыБухгалтерии"),
        "CalculationRegister" => Some("РегистрыРасчета"),
        "ChartOfAccounts" => Some("ПланыСчетов"),
        "ChartOfCharacteristicTypes" => Some("ПланыВидовХарактеристик"),
        "ChartOfCalculationTypes" => Some("ПланыВидовРасчета"),
        "BusinessProcess" => Some("БизнесПроцессы"),
        "Task" => Some("Задачи"),
        "ExchangePlan" => Some("ПланыОбмена"),
        "Report" => Some("Отчеты"),
        "DataProcessor" => Some("Обработки"),
        "DocumentJournal" => Some("ЖурналыДокументов"),
        _ => None,
    }
}

pub(crate) fn compile_meta(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> AdapterOutcome {
    let write_result = (|| -> Result<(String, Vec<PathBuf>), String> {
        let json_path_raw = required_path(args, &["jsonPath", "JsonPath"], "JsonPath")?;
        let output_dir_label = string_arg(args, &["outputDir", "OutputDir"])
            .ok_or_else(|| "missing required OutputDir argument".to_string())?
            .to_string();
        let output_dir = absolutize(PathBuf::from(&output_dir_label), &context.cwd);
        let json_path = absolutize(json_path_raw.clone(), &context.cwd);
        if !json_path.is_file() {
            return Err(format!("File not found: {}", json_path_raw.display()));
        }

        let json_text = fs::read_to_string(&json_path)
            .map_err(|err| format!("failed to read {}: {err}", json_path.display()))?;
        let mut defn: Value = serde_json::from_str(json_text.trim_start_matches('\u{feff}'))
            .map_err(|err| format!("failed to parse metadata JSON: {err}"))?;
        if defn.is_array() {
            return Err(
                "native meta compiler currently supports one metadata object per call".to_string(),
            );
        }
        if defn.get("type").is_none() {
            if let Some(object_type) = defn.get("objectType").cloned() {
                defn.as_object_mut()
                    .ok_or_else(|| "metadata JSON must be an object".to_string())?
                    .insert("type".to_string(), object_type);
            }
        }
        let object = defn
            .as_object()
            .ok_or_else(|| "metadata JSON must be an object".to_string())?;
        let raw_type = object
            .get("type")
            .and_then(Value::as_str)
            .ok_or_else(|| "JSON must have 'type' field".to_string())?;
        let obj_type = normalize_meta_object_type(raw_type);
        if obj_type != "Catalog" {
            return Err(format!(
                "Unsupported type: {obj_type}. Valid: Catalog, Document, Enum, Constant, InformationRegister, AccumulationRegister, AccountingRegister, CalculationRegister, ChartOfAccounts, ChartOfCharacteristicTypes, ChartOfCalculationTypes, BusinessProcess, Task, ExchangePlan, DocumentJournal, Report, DataProcessor, CommonModule, ScheduledJob, EventSubscription, HTTPService, WebService, DefinedType"
            ));
        }
        let obj_name = object
            .get("name")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| "JSON must have 'name' field".to_string())?;
        let type_plural = "Catalogs";
        let type_dir = output_dir.join(type_plural);
        let main_xml_path = type_dir.join(format!("{obj_name}.xml"));
        let obj_sub_dir = type_dir.join(obj_name);
        let ext_dir = obj_sub_dir.join("Ext");

        fs::create_dir_all(&obj_sub_dir)
            .map_err(|err| format!("failed to create {}: {err}", obj_sub_dir.display()))?;
        let format_version = detect_format_version(&output_dir);
        let metadata_xml = meta_compile_catalog_xml(object, obj_name, &format_version)?;
        write_utf8_bom(&main_xml_path, &metadata_xml)?;

        let mut artifacts = vec![main_xml_path.clone()];
        let mut modules_created = Vec::<PathBuf>::new();
        let module_path = ext_dir.join("ObjectModule.bsl");
        if !module_path.is_file() {
            fs::create_dir_all(&ext_dir)
                .map_err(|err| format!("failed to create {}: {err}", ext_dir.display()))?;
            write_utf8_bom(&module_path, "")?;
            modules_created.push(module_path.clone());
            artifacts.push(module_path.clone());
        }

        let reg_result = register_compiled_meta_in_configuration(&output_dir, "Catalog", obj_name)?;

        let attr_count = object
            .get("attributes")
            .and_then(Value::as_array)
            .map_or(0, Vec::len);
        let ts_count = object
            .get("tabularSections")
            .map(meta_compile_collection_count)
            .unwrap_or(0);
        let uid = "00000000-0000-0000-0000-000000000001";
        let mut stdout = format!(
            "[OK] Catalog '{obj_name}' compiled\n     UUID: {uid}\n     File: {}/{type_plural}/{obj_name}.xml\n",
            output_dir_label.trim_end_matches(['/', '\\'])
        );
        let mut details = Vec::new();
        if attr_count > 0 {
            details.push(format!("Attributes: {attr_count}"));
        }
        if ts_count > 0 {
            details.push(format!("TabularSections: {ts_count}"));
        }
        if !details.is_empty() {
            stdout.push_str(&format!("     {}\n", details.join(", ")));
        }
        for module in modules_created {
            stdout.push_str(&format!(
                "     Module: {}/{type_plural}/{obj_name}/Ext/{}\n",
                output_dir_label.trim_end_matches(['/', '\\']),
                module
                    .file_name()
                    .and_then(|value| value.to_str())
                    .unwrap_or("ObjectModule.bsl")
            ));
        }
        match reg_result.as_deref() {
            Some("added") => stdout.push_str(&format!(
                "     Configuration.xml: <Catalog>{obj_name}</Catalog> added to ChildObjects\n"
            )),
            Some("already") => stdout.push_str(&format!(
                "     Configuration.xml: <Catalog>{obj_name}</Catalog> already registered\n"
            )),
            Some("no-childobj") => {}
            _ => stdout.push_str(&format!(
                "     Configuration.xml: not found at {}/Configuration.xml (register manually)\n",
                output_dir_label.trim_end_matches(['/', '\\'])
            )),
        }

        Ok((stdout, artifacts))
    })();

    match write_result {
        Ok((stdout, artifacts)) => AdapterOutcome {
            ok: true,
            summary: "unica.meta.compile completed with native metadata compiler".to_string(),
            changes: artifacts
                .iter()
                .map(|path| format!("updated {}", path.display()))
                .collect(),
            warnings: Vec::new(),
            errors: Vec::new(),
            artifacts: artifacts
                .iter()
                .map(|path| path.display().to_string())
                .collect(),
            stdout: Some(stdout),
            stderr: None,
            command: None,
        },
        Err(error) => AdapterOutcome {
            ok: false,
            summary: "unica.meta.compile failed in native metadata compiler".to_string(),
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

pub(crate) fn meta_compile_collection_count(value: &Value) -> usize {
    value
        .as_array()
        .map(Vec::len)
        .or_else(|| value.as_object().map(Map::len))
        .unwrap_or(0)
}

pub(crate) fn normalize_meta_object_type(raw: &str) -> String {
    match raw {
        "Справочник" | "Каталог" => "Catalog",
        "Документ" => "Document",
        "Перечисление" => "Enum",
        "Константа" => "Constant",
        "РегистрСведений" => "InformationRegister",
        "РегистрНакопления" => "AccumulationRegister",
        "РегистрБухгалтерии" => "AccountingRegister",
        "РегистрРасчёта" | "РегистрРасчета" => "CalculationRegister",
        "ПланСчетов" => "ChartOfAccounts",
        "ПланВидовХарактеристик" => "ChartOfCharacteristicTypes",
        "ПланВидовРасчёта" | "ПланВидовРасчета" => {
            "ChartOfCalculationTypes"
        }
        "БизнесПроцесс" => "BusinessProcess",
        "Задача" => "Task",
        "ПланОбмена" => "ExchangePlan",
        "ЖурналДокументов" => "DocumentJournal",
        "Отчёт" | "Отчет" => "Report",
        "Обработка" => "DataProcessor",
        "ОбщийМодуль" => "CommonModule",
        "РегламентноеЗадание" => "ScheduledJob",
        "ПодпискаНаСобытие" => "EventSubscription",
        "HTTPСервис" => "HTTPService",
        "ВебСервис" => "WebService",
        "ОпределяемыйТип" => "DefinedType",
        other => other,
    }
    .to_string()
}

pub(crate) fn meta_compile_catalog_xml(
    defn: &Map<String, Value>,
    obj_name: &str,
    format_version: &str,
) -> Result<String, String> {
    let mut uuid_counter = 1usize;
    let mut next_uuid = || {
        let uuid = stable_uuid(uuid_counter);
        uuid_counter += 1;
        uuid
    };
    let obj_uuid = next_uuid();
    let synonym = defn
        .get("synonym")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| split_meta_camel_case(obj_name));

    let mut lines = Vec::<String>::new();
    lines.push("<?xml version=\"1.0\" encoding=\"UTF-8\"?>".to_string());
    lines.push(format!(
        "<MetaDataObject {} version=\"{format_version}\">",
        meta_xmlns_decl()
    ));
    lines.push(format!("\t<Catalog uuid=\"{obj_uuid}\">"));
    emit_meta_internal_info(&mut lines, "\t\t", "Catalog", obj_name, &mut next_uuid);
    lines.push("\t\t<Properties>".to_string());
    emit_meta_catalog_properties(&mut lines, "\t\t\t", defn, obj_name, &synonym);
    lines.push("\t\t</Properties>".to_string());

    let attrs = meta_compile_attributes(defn.get("attributes"));
    let tabular_sections = meta_compile_tabular_sections(defn.get("tabularSections"))?;
    if attrs.is_empty() && tabular_sections.is_empty() {
        lines.push("\t\t<ChildObjects/>".to_string());
    } else {
        lines.push("\t\t<ChildObjects>".to_string());
        for attr in &attrs {
            emit_meta_attribute(&mut lines, "\t\t\t", attr, "catalog", &mut next_uuid);
        }
        for section in &tabular_sections {
            emit_meta_tabular_section(
                &mut lines,
                "\t\t\t",
                section,
                "Catalog",
                obj_name,
                &mut next_uuid,
            );
        }
        lines.push("\t\t</ChildObjects>".to_string());
    }

    lines.push("\t</Catalog>".to_string());
    lines.push("</MetaDataObject>".to_string());
    Ok(format!("{}\n", lines.join("\n")))
}

pub(crate) fn meta_xmlns_decl() -> &'static str {
    "xmlns=\"http://v8.1c.ru/8.3/MDClasses\" xmlns:app=\"http://v8.1c.ru/8.2/managed-application/core\" xmlns:cfg=\"http://v8.1c.ru/8.1/data/enterprise/current-config\" xmlns:cmi=\"http://v8.1c.ru/8.2/managed-application/cmi\" xmlns:ent=\"http://v8.1c.ru/8.1/data/enterprise\" xmlns:lf=\"http://v8.1c.ru/8.2/managed-application/logform\" xmlns:style=\"http://v8.1c.ru/8.1/data/ui/style\" xmlns:sys=\"http://v8.1c.ru/8.1/data/ui/fonts/system\" xmlns:v8=\"http://v8.1c.ru/8.1/data/core\" xmlns:v8ui=\"http://v8.1c.ru/8.1/data/ui\" xmlns:web=\"http://v8.1c.ru/8.1/data/ui/colors/web\" xmlns:win=\"http://v8.1c.ru/8.1/data/ui/colors/windows\" xmlns:xen=\"http://v8.1c.ru/8.3/xcf/enums\" xmlns:xpr=\"http://v8.1c.ru/8.3/xcf/predef\" xmlns:xr=\"http://v8.1c.ru/8.3/xcf/readable\" xmlns:xs=\"http://www.w3.org/2001/XMLSchema\" xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\""
}

pub(crate) fn emit_meta_internal_info<F>(
    lines: &mut Vec<String>,
    indent: &str,
    object_type: &str,
    object_name: &str,
    next_uuid: &mut F,
) where
    F: FnMut() -> String,
{
    let generated = match object_type {
        "Catalog" => vec![
            ("CatalogObject", "Object"),
            ("CatalogRef", "Ref"),
            ("CatalogSelection", "Selection"),
            ("CatalogList", "List"),
            ("CatalogManager", "Manager"),
        ],
        _ => Vec::new(),
    };
    if generated.is_empty() {
        return;
    }
    lines.push(format!("{indent}<InternalInfo>"));
    for (prefix, category) in generated {
        lines.push(format!(
            "{indent}\t<xr:GeneratedType name=\"{prefix}.{object_name}\" category=\"{category}\">"
        ));
        lines.push(format!(
            "{indent}\t\t<xr:TypeId>{}</xr:TypeId>",
            next_uuid()
        ));
        lines.push(format!(
            "{indent}\t\t<xr:ValueId>{}</xr:ValueId>",
            next_uuid()
        ));
        lines.push(format!("{indent}\t</xr:GeneratedType>"));
    }
    lines.push(format!("{indent}</InternalInfo>"));
}

pub(crate) fn emit_meta_catalog_properties(
    lines: &mut Vec<String>,
    indent: &str,
    defn: &Map<String, Value>,
    obj_name: &str,
    synonym: &str,
) {
    lines.push(format!("{indent}<Name>{}</Name>", escape_xml(obj_name)));
    emit_meta_mltext(lines, indent, "Synonym", synonym);
    lines.push(format!("{indent}<Comment/>"));
    let hierarchical = defn.get("hierarchical").and_then(Value::as_bool) == Some(true);
    lines.push(format!(
        "{indent}<Hierarchical>{hierarchical}</Hierarchical>"
    ));
    lines.push(format!(
        "{indent}<HierarchyType>{}</HierarchyType>",
        meta_enum_prop(defn, "hierarchyType", "HierarchyFoldersAndItems")
    ));
    let limit_level_count = defn.get("limitLevelCount").and_then(Value::as_bool) == Some(true);
    let level_count = defn.get("levelCount").and_then(json_i64_value).unwrap_or(2);
    let folders_on_top = defn.get("foldersOnTop").and_then(Value::as_bool) != Some(false);
    lines.push(format!(
        "{indent}<LimitLevelCount>{limit_level_count}</LimitLevelCount>"
    ));
    lines.push(format!("{indent}<LevelCount>{level_count}</LevelCount>"));
    lines.push(format!(
        "{indent}<FoldersOnTop>{folders_on_top}</FoldersOnTop>"
    ));
    lines.push(format!(
        "{indent}<UseStandardCommands>true</UseStandardCommands>"
    ));
    lines.push(format!("{indent}<Owners/>"));
    lines.push(format!(
        "{indent}<SubordinationUse>{}</SubordinationUse>",
        meta_enum_prop(defn, "subordinationUse", "ToItems")
    ));
    let code_length = defn.get("codeLength").and_then(json_i64_value).unwrap_or(9);
    let description_length = defn
        .get("descriptionLength")
        .and_then(json_i64_value)
        .unwrap_or(25);
    lines.push(format!("{indent}<CodeLength>{code_length}</CodeLength>"));
    lines.push(format!(
        "{indent}<DescriptionLength>{description_length}</DescriptionLength>"
    ));
    lines.push(format!(
        "{indent}<CodeType>{}</CodeType>",
        meta_enum_prop(defn, "codeType", "String")
    ));
    lines.push(format!(
        "{indent}<CodeAllowedLength>{}</CodeAllowedLength>",
        meta_enum_prop(defn, "codeAllowedLength", "Variable")
    ));
    lines.push(format!(
        "{indent}<CodeSeries>{}</CodeSeries>",
        meta_enum_prop(defn, "codeSeries", "WholeCatalog")
    ));
    let check_unique = defn.get("checkUnique").and_then(Value::as_bool) == Some(true);
    let autonumbering = defn.get("autonumbering").and_then(Value::as_bool) != Some(false);
    lines.push(format!("{indent}<CheckUnique>{check_unique}</CheckUnique>"));
    lines.push(format!(
        "{indent}<Autonumbering>{autonumbering}</Autonumbering>"
    ));
    lines.push(format!(
        "{indent}<DefaultPresentation>{}</DefaultPresentation>",
        meta_enum_prop(defn, "defaultPresentation", "AsDescription")
    ));
    emit_meta_standard_attributes(lines, indent, "Catalog");
    lines.push(format!("{indent}<Characteristics/>"));
    lines.push(format!(
        "{indent}<PredefinedDataUpdate>Auto</PredefinedDataUpdate>"
    ));
    lines.push(format!("{indent}<EditType>InDialog</EditType>"));
    let quick_choice = defn.get("quickChoice").and_then(Value::as_bool) != Some(false);
    lines.push(format!("{indent}<QuickChoice>{quick_choice}</QuickChoice>"));
    lines.push(format!(
        "{indent}<ChoiceMode>{}</ChoiceMode>",
        meta_enum_prop(defn, "choiceMode", "BothWays")
    ));
    lines.push(format!("{indent}<InputByString>"));
    lines.push(format!(
        "{indent}\t<xr:Field>Catalog.{obj_name}.StandardAttribute.Description</xr:Field>"
    ));
    lines.push(format!(
        "{indent}\t<xr:Field>Catalog.{obj_name}.StandardAttribute.Code</xr:Field>"
    ));
    lines.push(format!("{indent}</InputByString>"));
    lines.push(format!(
        "{indent}<SearchStringModeOnInputByString>Begin</SearchStringModeOnInputByString>"
    ));
    lines.push(format!(
        "{indent}<FullTextSearchOnInputByString>DontUse</FullTextSearchOnInputByString>"
    ));
    lines.push(format!(
        "{indent}<ChoiceDataGetModeOnInputByString>Directly</ChoiceDataGetModeOnInputByString>"
    ));
    for tag in [
        "DefaultObjectForm",
        "DefaultFolderForm",
        "DefaultListForm",
        "DefaultChoiceForm",
        "DefaultFolderChoiceForm",
        "AuxiliaryObjectForm",
        "AuxiliaryFolderForm",
        "AuxiliaryListForm",
        "AuxiliaryChoiceForm",
        "AuxiliaryFolderChoiceForm",
    ] {
        lines.push(format!("{indent}<{tag}/>"));
    }
    lines.push(format!(
        "{indent}<IncludeHelpInContents>false</IncludeHelpInContents>"
    ));
    for line in [
        "<BasedOn/>",
        "<DataLockFields/>",
        "<DataLockControlMode>Automatic</DataLockControlMode>",
        "<FullTextSearch>Use</FullTextSearch>",
        "<ObjectPresentation/>",
        "<ExtendedObjectPresentation/>",
        "<ListPresentation/>",
        "<ExtendedListPresentation/>",
        "<Explanation/>",
        "<CreateOnInput>DontUse</CreateOnInput>",
        "<ChoiceHistoryOnInput>Auto</ChoiceHistoryOnInput>",
        "<DataHistory>DontUse</DataHistory>",
        "<UpdateDataHistoryImmediatelyAfterWrite>false</UpdateDataHistoryImmediatelyAfterWrite>",
        "<ExecuteAfterWriteDataHistoryVersionProcessing>false</ExecuteAfterWriteDataHistoryVersionProcessing>",
    ] {
        lines.push(format!("{indent}{line}"));
    }
}

pub(crate) fn meta_enum_prop(defn: &Map<String, Value>, field_name: &str, default: &str) -> String {
    defn.get(field_name)
        .and_then(Value::as_str)
        .map(normalize_meta_enum_value)
        .unwrap_or_else(|| default.to_string())
}

pub(crate) fn normalize_meta_enum_value(value: &str) -> String {
    match value {
        "Balances" => "Balance",
        "Остатки" => "Balance",
        "Обороты" => "Turnovers",
        "RecordSubordinate" | "Subordinate" | "ПодчинениеРегистратору" => {
            "RecorderSubordinate"
        }
        "Независимый" => "Independent",
        "Автоматический" => "Automatic",
        "Управляемый" => "Managed",
        "ВВидеНаименования" => "AsDescription",
        "ВВидеКода" => "AsCode",
        "ВДиалоге" => "InDialog",
        "ВСписке" => "InList",
        "ОбаСпособа" => "BothWays",
        other => other,
    }
    .to_string()
}

pub(crate) fn emit_meta_standard_attributes(
    lines: &mut Vec<String>,
    indent: &str,
    object_type: &str,
) {
    let attrs = match object_type {
        "Catalog" => vec![
            "PredefinedDataName",
            "Predefined",
            "Ref",
            "DeletionMark",
            "IsFolder",
            "Owner",
            "Parent",
            "Description",
            "Code",
        ],
        "TabularSection" => vec!["LineNumber"],
        _ => Vec::new(),
    };
    if attrs.is_empty() {
        return;
    }
    lines.push(format!("{indent}<StandardAttributes>"));
    for attr in attrs {
        emit_meta_standard_attribute(lines, &format!("{indent}\t"), attr);
    }
    lines.push(format!("{indent}</StandardAttributes>"));
}

pub(crate) fn emit_meta_standard_attribute(lines: &mut Vec<String>, indent: &str, attr_name: &str) {
    lines.push(format!(
        "{indent}<xr:StandardAttribute name=\"{attr_name}\">"
    ));
    for line in [
        "<xr:LinkByType/>",
        "<xr:FillChecking>DontCheck</xr:FillChecking>",
        "<xr:MultiLine>false</xr:MultiLine>",
        "<xr:FillFromFillingValue>false</xr:FillFromFillingValue>",
        "<xr:CreateOnInput>Auto</xr:CreateOnInput>",
        "<xr:MaxValue xsi:nil=\"true\"/>",
        "<xr:ToolTip/>",
        "<xr:ExtendedEdit>false</xr:ExtendedEdit>",
        "<xr:Format/>",
        "<xr:ChoiceForm/>",
        "<xr:QuickChoice>Auto</xr:QuickChoice>",
        "<xr:ChoiceHistoryOnInput>Auto</xr:ChoiceHistoryOnInput>",
        "<xr:EditFormat/>",
        "<xr:PasswordMode>false</xr:PasswordMode>",
        "<xr:DataHistory>Use</xr:DataHistory>",
        "<xr:MarkNegatives>false</xr:MarkNegatives>",
        "<xr:MinValue xsi:nil=\"true\"/>",
        "<xr:Synonym/>",
        "<xr:Comment/>",
        "<xr:FullTextSearch>Use</xr:FullTextSearch>",
        "<xr:ChoiceParameterLinks/>",
        "<xr:FillValue xsi:nil=\"true\"/>",
        "<xr:Mask/>",
        "<xr:ChoiceParameters/>",
    ] {
        lines.push(format!("{indent}\t{line}"));
    }
    lines.push(format!("{indent}</xr:StandardAttribute>"));
}

#[derive(Clone)]
pub(crate) struct MetaCompileAttr {
    pub(crate) name: String,
    pub(crate) type_name: String,
    pub(crate) synonym: String,
    pub(crate) flags: Vec<String>,
    pub(crate) fill_checking: String,
    pub(crate) indexing: String,
    pub(crate) multi_line: bool,
    pub(crate) choice_history_on_input: String,
}

pub(crate) struct MetaCompileTabularSection {
    pub(crate) name: String,
    pub(crate) columns: Vec<MetaCompileAttr>,
}

pub(crate) fn meta_compile_attributes(value: Option<&Value>) -> Vec<MetaCompileAttr> {
    let Some(value) = value else {
        return Vec::new();
    };
    if let Some(object) = value.as_object() {
        return object
            .iter()
            .map(|(key, value)| {
                meta_compile_parse_attr(&Value::String(format!(
                    "{key}:{}",
                    json_value_to_python_string(value)
                )))
            })
            .collect();
    }
    value
        .as_array()
        .map(|items| items.iter().map(meta_compile_parse_attr).collect())
        .unwrap_or_default()
}

pub(crate) fn meta_compile_tabular_sections(
    value: Option<&Value>,
) -> Result<Vec<MetaCompileTabularSection>, String> {
    let Some(value) = value else {
        return Ok(Vec::new());
    };
    let mut result = Vec::new();
    if let Some(items) = value.as_array() {
        for item in items {
            let object = item
                .as_object()
                .ok_or_else(|| "tabular section must be an object".to_string())?;
            let name = object
                .get("name")
                .and_then(Value::as_str)
                .ok_or_else(|| "tabular section is missing name".to_string())?
                .to_string();
            result.push(MetaCompileTabularSection {
                name,
                columns: meta_compile_attributes(object.get("attributes")),
            });
        }
    } else if let Some(object) = value.as_object() {
        for (name, columns) in object {
            result.push(MetaCompileTabularSection {
                name: name.to_string(),
                columns: meta_compile_attributes(Some(columns)),
            });
        }
    }
    Ok(result)
}

pub(crate) fn meta_compile_parse_attr(value: &Value) -> MetaCompileAttr {
    if let Some(text) = value.as_str() {
        let mut pieces = text.splitn(2, '|');
        let main = pieces.next().unwrap_or_default().trim();
        let flags = pieces
            .next()
            .map(|part| {
                part.split(',')
                    .map(str::trim)
                    .filter(|flag| !flag.is_empty())
                    .map(|flag| flag.to_lowercase())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let mut colon = main.splitn(2, ':');
        let name = colon.next().unwrap_or_default().trim().to_string();
        let type_name = colon.next().unwrap_or_default().trim().to_string();
        let synonym = split_meta_camel_case(&name);
        return MetaCompileAttr {
            name,
            type_name,
            synonym,
            flags,
            fill_checking: String::new(),
            indexing: String::new(),
            multi_line: false,
            choice_history_on_input: String::new(),
        };
    }
    let object = value.as_object();
    let name = object
        .and_then(|object| object.get("name"))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let type_name = object.map(meta_compile_build_type).unwrap_or_default();
    let synonym = object
        .and_then(|object| object.get("synonym"))
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| split_meta_camel_case(&name));
    let flags = object
        .and_then(|object| object.get("flags"))
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(ToOwned::to_owned)
                .collect()
        })
        .unwrap_or_default();
    MetaCompileAttr {
        name,
        type_name,
        synonym,
        flags,
        fill_checking: object
            .and_then(|object| object.get("fillChecking"))
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        indexing: object
            .and_then(|object| object.get("indexing"))
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        multi_line: object
            .and_then(|object| object.get("multiLine"))
            .and_then(Value::as_bool)
            == Some(true),
        choice_history_on_input: object
            .and_then(|object| object.get("choiceHistoryOnInput"))
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
    }
}

pub(crate) fn meta_compile_build_type(object: &Map<String, Value>) -> String {
    let mut type_name = object
        .get("valueType")
        .or_else(|| object.get("type"))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    if !type_name.is_empty() && !type_name.contains('(') {
        if type_name == "String" {
            if let Some(length) = object.get("length").and_then(json_i64_value) {
                type_name = format!("String({length})");
            }
        } else if type_name == "Number" {
            if let Some(length) = object.get("length").and_then(json_i64_value) {
                let precision = object
                    .get("precision")
                    .and_then(json_i64_value)
                    .unwrap_or(0);
                let nn = if object.get("nonneg").and_then(Value::as_bool) == Some(true)
                    || object.get("nonnegative").and_then(Value::as_bool) == Some(true)
                {
                    ",nonneg"
                } else {
                    ""
                };
                type_name = format!("Number({length},{precision}{nn})");
            }
        }
    }
    type_name
}

pub(crate) fn emit_meta_attribute<F>(
    lines: &mut Vec<String>,
    indent: &str,
    attr: &MetaCompileAttr,
    context: &str,
    next_uuid: &mut F,
) where
    F: FnMut() -> String,
{
    lines.push(format!("{indent}<Attribute uuid=\"{}\">", next_uuid()));
    lines.push(format!("{indent}\t<Properties>"));
    lines.push(format!(
        "{indent}\t\t<Name>{}</Name>",
        escape_xml(&attr.name)
    ));
    emit_meta_mltext(lines, &format!("{indent}\t\t"), "Synonym", &attr.synonym);
    lines.push(format!("{indent}\t\t<Comment/>"));
    if attr.type_name.is_empty() {
        lines.push(format!("{indent}\t\t<Type>"));
        lines.push(format!("{indent}\t\t\t<v8:Type>xs:string</v8:Type>"));
        lines.push(format!("{indent}\t\t</Type>"));
    } else {
        emit_meta_value_type(lines, &format!("{indent}\t\t"), &attr.type_name);
    }
    lines.push(format!("{indent}\t\t<PasswordMode>false</PasswordMode>"));
    lines.push(format!("{indent}\t\t<Format/>"));
    lines.push(format!("{indent}\t\t<EditFormat/>"));
    lines.push(format!("{indent}\t\t<ToolTip/>"));
    lines.push(format!("{indent}\t\t<MarkNegatives>false</MarkNegatives>"));
    lines.push(format!("{indent}\t\t<Mask/>"));
    let multi_line = attr.multi_line || attr.flags.iter().any(|flag| flag == "multiline");
    lines.push(format!("{indent}\t\t<MultiLine>{multi_line}</MultiLine>"));
    lines.push(format!("{indent}\t\t<ExtendedEdit>false</ExtendedEdit>"));
    lines.push(format!("{indent}\t\t<MinValue xsi:nil=\"true\"/>"));
    lines.push(format!("{indent}\t\t<MaxValue xsi:nil=\"true\"/>"));
    if !matches!(
        context,
        "tabular" | "processor" | "chart" | "register-other"
    ) {
        lines.push(format!(
            "{indent}\t\t<FillFromFillingValue>false</FillFromFillingValue>"
        ));
    }
    if !matches!(
        context,
        "tabular" | "processor" | "chart" | "register-other"
    ) {
        emit_meta_fill_value(lines, &format!("{indent}\t\t"), &attr.type_name);
    }
    let fill_checking = if !attr.fill_checking.is_empty() {
        attr.fill_checking.as_str()
    } else if attr.flags.iter().any(|flag| flag == "req") {
        "ShowError"
    } else {
        "DontCheck"
    };
    lines.push(format!(
        "{indent}\t\t<FillChecking>{fill_checking}</FillChecking>"
    ));
    for line in [
        "<ChoiceFoldersAndItems>Items</ChoiceFoldersAndItems>",
        "<ChoiceParameterLinks/>",
        "<ChoiceParameters/>",
        "<QuickChoice>Auto</QuickChoice>",
        "<CreateOnInput>Auto</CreateOnInput>",
        "<ChoiceForm/>",
        "<LinkByType/>",
    ] {
        lines.push(format!("{indent}\t\t{line}"));
    }
    let choice_history_on_input = if attr.choice_history_on_input.is_empty() {
        "Auto"
    } else {
        attr.choice_history_on_input.as_str()
    };
    lines.push(format!(
        "{indent}\t\t<ChoiceHistoryOnInput>{choice_history_on_input}</ChoiceHistoryOnInput>"
    ));
    if context == "catalog" {
        lines.push(format!("{indent}\t\t<Use>ForItem</Use>"));
    }
    if !matches!(context, "processor" | "processor-tabular") {
        let indexing = if !attr.indexing.is_empty() {
            attr.indexing.as_str()
        } else if attr.flags.iter().any(|flag| flag == "indexadditional") {
            "IndexWithAdditionalOrder"
        } else if attr.flags.iter().any(|flag| flag == "index") {
            "Index"
        } else {
            "DontIndex"
        };
        lines.push(format!("{indent}\t\t<Indexing>{indexing}</Indexing>"));
        lines.push(format!("{indent}\t\t<FullTextSearch>Use</FullTextSearch>"));
        if !matches!(context, "chart" | "register-other") {
            lines.push(format!("{indent}\t\t<DataHistory>Use</DataHistory>"));
        }
    }
    lines.push(format!("{indent}\t</Properties>"));
    lines.push(format!("{indent}</Attribute>"));
}

pub(crate) fn emit_meta_tabular_section<F>(
    lines: &mut Vec<String>,
    indent: &str,
    section: &MetaCompileTabularSection,
    object_type: &str,
    object_name: &str,
    next_uuid: &mut F,
) where
    F: FnMut() -> String,
{
    lines.push(format!("{indent}<TabularSection uuid=\"{}\">", next_uuid()));
    let type_prefix = format!("{object_type}TabularSection");
    let row_prefix = format!("{object_type}TabularSectionRow");
    lines.push(format!("{indent}\t<InternalInfo>"));
    lines.push(format!(
        "{indent}\t\t<xr:GeneratedType name=\"{type_prefix}.{object_name}.{}\" category=\"TabularSection\">",
        section.name
    ));
    lines.push(format!(
        "{indent}\t\t\t<xr:TypeId>{}</xr:TypeId>",
        next_uuid()
    ));
    lines.push(format!(
        "{indent}\t\t\t<xr:ValueId>{}</xr:ValueId>",
        next_uuid()
    ));
    lines.push(format!("{indent}\t\t</xr:GeneratedType>"));
    lines.push(format!(
        "{indent}\t\t<xr:GeneratedType name=\"{row_prefix}.{object_name}.{}\" category=\"TabularSectionRow\">",
        section.name
    ));
    lines.push(format!(
        "{indent}\t\t\t<xr:TypeId>{}</xr:TypeId>",
        next_uuid()
    ));
    lines.push(format!(
        "{indent}\t\t\t<xr:ValueId>{}</xr:ValueId>",
        next_uuid()
    ));
    lines.push(format!("{indent}\t\t</xr:GeneratedType>"));
    lines.push(format!("{indent}\t</InternalInfo>"));
    lines.push(format!("{indent}\t<Properties>"));
    lines.push(format!(
        "{indent}\t\t<Name>{}</Name>",
        escape_xml(&section.name)
    ));
    emit_meta_mltext(
        lines,
        &format!("{indent}\t\t"),
        "Synonym",
        &split_meta_camel_case(&section.name),
    );
    lines.push(format!("{indent}\t\t<Comment/>"));
    lines.push(format!("{indent}\t\t<ToolTip/>"));
    lines.push(format!(
        "{indent}\t\t<FillChecking>DontCheck</FillChecking>"
    ));
    emit_meta_standard_attributes(lines, &format!("{indent}\t\t"), "TabularSection");
    if object_type == "Catalog" {
        lines.push(format!("{indent}\t\t<Use>ForItem</Use>"));
    }
    lines.push(format!("{indent}\t</Properties>"));
    lines.push(format!("{indent}\t<ChildObjects>"));
    for column in &section.columns {
        emit_meta_attribute(
            lines,
            &format!("{indent}\t\t"),
            column,
            "tabular",
            next_uuid,
        );
    }
    lines.push(format!("{indent}\t</ChildObjects>"));
    lines.push(format!("{indent}</TabularSection>"));
}

pub(crate) fn emit_meta_mltext(lines: &mut Vec<String>, indent: &str, tag: &str, text: &str) {
    if text.is_empty() {
        lines.push(format!("{indent}<{tag}/>"));
        return;
    }
    lines.push(format!("{indent}<{tag}>"));
    lines.push(format!("{indent}\t<v8:item>"));
    lines.push(format!("{indent}\t\t<v8:lang>ru</v8:lang>"));
    lines.push(format!(
        "{indent}\t\t<v8:content>{}</v8:content>",
        escape_xml(text)
    ));
    lines.push(format!("{indent}\t</v8:item>"));
    lines.push(format!("{indent}</{tag}>"));
}

pub(crate) fn emit_meta_value_type(lines: &mut Vec<String>, indent: &str, type_name: &str) {
    lines.push(format!("{indent}<Type>"));
    emit_meta_type_content(lines, &format!("{indent}\t"), type_name);
    lines.push(format!("{indent}</Type>"));
}

pub(crate) fn emit_meta_type_content(lines: &mut Vec<String>, indent: &str, type_name: &str) {
    if type_name.is_empty() {
        return;
    }
    if type_name.contains(" + ") {
        for part in type_name.split('+').map(str::trim) {
            emit_meta_type_content(lines, indent, part);
        }
        return;
    }
    let resolved = resolve_meta_type(type_name);
    if resolved == "Boolean" {
        lines.push(format!("{indent}<v8:Type>xs:boolean</v8:Type>"));
    } else if let Some(length) = resolved
        .strip_prefix("String(")
        .and_then(|rest| rest.strip_suffix(')'))
    {
        lines.push(format!("{indent}<v8:Type>xs:string</v8:Type>"));
        lines.push(format!("{indent}<v8:StringQualifiers>"));
        lines.push(format!("{indent}\t<v8:Length>{length}</v8:Length>"));
        lines.push(format!(
            "{indent}\t<v8:AllowedLength>Variable</v8:AllowedLength>"
        ));
        lines.push(format!("{indent}</v8:StringQualifiers>"));
    } else if resolved == "String" {
        lines.push(format!("{indent}<v8:Type>xs:string</v8:Type>"));
        lines.push(format!("{indent}<v8:StringQualifiers>"));
        lines.push(format!("{indent}\t<v8:Length>10</v8:Length>"));
        lines.push(format!(
            "{indent}\t<v8:AllowedLength>Variable</v8:AllowedLength>"
        ));
        lines.push(format!("{indent}</v8:StringQualifiers>"));
    } else if resolved == "Number" {
        lines.push(format!("{indent}<v8:Type>xs:decimal</v8:Type>"));
        lines.push(format!("{indent}<v8:NumberQualifiers>"));
        lines.push(format!("{indent}\t<v8:Digits>10</v8:Digits>"));
        lines.push(format!(
            "{indent}\t<v8:FractionDigits>0</v8:FractionDigits>"
        ));
        lines.push(format!("{indent}\t<v8:AllowedSign>Any</v8:AllowedSign>"));
        lines.push(format!("{indent}</v8:NumberQualifiers>"));
    } else if let Some((digits, fraction, nonnegative)) = parse_meta_number_type(&resolved) {
        lines.push(format!("{indent}<v8:Type>xs:decimal</v8:Type>"));
        lines.push(format!("{indent}<v8:NumberQualifiers>"));
        lines.push(format!("{indent}\t<v8:Digits>{digits}</v8:Digits>"));
        lines.push(format!(
            "{indent}\t<v8:FractionDigits>{fraction}</v8:FractionDigits>"
        ));
        lines.push(format!(
            "{indent}\t<v8:AllowedSign>{}</v8:AllowedSign>",
            if nonnegative { "Nonnegative" } else { "Any" }
        ));
        lines.push(format!("{indent}</v8:NumberQualifiers>"));
    } else {
        lines.push(format!(
            "{indent}<v8:Type>{}</v8:Type>",
            escape_xml(&resolved)
        ));
    }
}

pub(crate) fn emit_meta_fill_value(lines: &mut Vec<String>, indent: &str, type_name: &str) {
    if type_name.is_empty() {
        lines.push(format!("{indent}<FillValue xsi:nil=\"true\"/>"));
        return;
    }
    let resolved = resolve_meta_type(type_name);
    if resolved == "Boolean" {
        lines.push(format!(
            "{indent}<FillValue xsi:type=\"xs:boolean\">false</FillValue>"
        ));
    } else if resolved.starts_with("String") {
        lines.push(format!("{indent}<FillValue xsi:type=\"xs:string\"/>"));
    } else if resolved.starts_with("Number") {
        lines.push(format!(
            "{indent}<FillValue xsi:type=\"xs:decimal\">0</FillValue>"
        ));
    } else {
        lines.push(format!("{indent}<FillValue xsi:nil=\"true\"/>"));
    }
}

pub(crate) fn resolve_meta_type(type_name: &str) -> String {
    if let Some(open) = type_name.find('(') {
        if type_name.ends_with(')') {
            let base = type_name[..open].trim();
            let params = &type_name[open + 1..type_name.len() - 1];
            if let Some(resolved) = meta_type_synonym(base) {
                return format!("{resolved}({params})");
            }
        }
    }
    if let Some(dot) = type_name.find('.') {
        let prefix = &type_name[..dot];
        let suffix = &type_name[dot..];
        if let Some(resolved) = meta_type_synonym(prefix) {
            return format!("{resolved}{suffix}");
        }
    }
    meta_type_synonym(type_name)
        .unwrap_or(type_name)
        .to_string()
}

pub(crate) fn meta_type_synonym(value: &str) -> Option<&'static str> {
    match value.to_lowercase().as_str() {
        "число" | "number" => Some("Number"),
        "строка" | "string" => Some("String"),
        "булево" | "boolean" | "bool" => Some("Boolean"),
        "дата" | "date" => Some("Date"),
        "датавремя" | "datetime" => Some("DateTime"),
        "справочникссылка" | "catalogref" => Some("CatalogRef"),
        "документссылка" | "documentref" => Some("DocumentRef"),
        "перечислениессылка" | "enumref" => Some("EnumRef"),
        "определяемыйтип" | "definedtype" => Some("DefinedType"),
        _ => None,
    }
}

pub(crate) fn parse_meta_number_type(value: &str) -> Option<(&str, &str, bool)> {
    let rest = value.strip_prefix("Number(")?.strip_suffix(')')?;
    let parts = rest.split(',').map(str::trim).collect::<Vec<_>>();
    if parts.len() < 2 {
        return None;
    }
    Some((parts[0], parts[1], parts.get(2) == Some(&"nonneg")))
}

pub(crate) fn split_meta_camel_case(name: &str) -> String {
    if name.is_empty() {
        return String::new();
    }
    let mut result = String::new();
    let mut previous_lower = false;
    for ch in name.chars() {
        if previous_lower && ch.is_uppercase() {
            result.push(' ');
        }
        result.push(ch);
        previous_lower = ch.is_lowercase();
    }
    let mut chars = result.chars();
    match chars.next() {
        Some(first) => format!("{}{}", first, chars.as_str().to_lowercase()),
        None => result,
    }
}

pub(crate) fn register_compiled_meta_in_configuration(
    output_dir: &Path,
    child_tag: &str,
    obj_name: &str,
) -> Result<Option<String>, String> {
    let config_xml_path = output_dir.join("Configuration.xml");
    if !config_xml_path.is_file() {
        return Ok(Some("no-config".to_string()));
    }
    let mut raw_text = fs::read_to_string(&config_xml_path)
        .map_err(|err| format!("failed to read {}: {err}", config_xml_path.display()))?;
    if raw_text.contains(&format!("<{child_tag}>{obj_name}</{child_tag}>")) {
        return Ok(Some("already".to_string()));
    }
    if raw_text.contains("</ChildObjects>") {
        raw_text = raw_text.replacen(
            "</ChildObjects>",
            &format!("\t\t\t<{child_tag}>{obj_name}</{child_tag}>\n\t\t</ChildObjects>"),
            1,
        );
        write_utf8_bom(&config_xml_path, &raw_text)?;
        Ok(Some("added".to_string()))
    } else {
        Ok(Some("no-childobj".to_string()))
    }
}

pub(crate) fn edit_meta(args: &Map<String, Value>, context: &WorkspaceContext) -> AdapterOutcome {
    let edit_result = (|| -> Result<(String, PathBuf, usize), String> {
        let definition_file = path_arg(args, &["definitionFile", "DefinitionFile"]);
        let operation = string_arg(args, &["operation", "Operation"]);
        if definition_file.is_some() && operation.is_some() {
            return Err("Cannot use both -DefinitionFile and -Operation".to_string());
        }
        if definition_file.is_none() && operation.is_none() {
            return Err("Either -DefinitionFile or -Operation is required".to_string());
        }
        if let Some(definition_file) = definition_file {
            let definition_path = absolutize(definition_file.clone(), &context.cwd);
            if !definition_path.exists() {
                return Err(format!(
                    "Definition file not found: {}",
                    definition_file.display()
                ));
            }
            return Err(
                "native meta-edit currently supports inline -Operation mode only".to_string(),
            );
        }

        let object_path_raw = required_path(
            args,
            &["objectPath", "ObjectPath", "path", "Path"],
            "ObjectPath",
        )?;
        let object_path = resolve_meta_edit_object_path(&object_path_raw, &context.cwd)?;
        let operation = operation.expect("checked above");
        let value = string_arg(args, &["value", "Value"]).unwrap_or_default();

        if operation != "modify-property" {
            return Err(format!(
                "native meta-edit currently supports modify-property only, got: {operation}"
            ));
        }

        let mut xml_text = fs::read_to_string(&object_path)
            .map_err(|err| format!("failed to read {}: {err}", object_path.display()))?;
        let (object_type, object_name) = meta_edit_object_identity(&xml_text)?;
        let mut modified = 0usize;
        for pair in value
            .split(";;")
            .map(str::trim)
            .filter(|part| !part.is_empty())
        {
            let Some((key, raw_value)) = pair.split_once('=') else {
                continue;
            };
            let key = key.trim();
            let raw_value = raw_value.trim();
            let normalized = normalize_meta_edit_property_value(key, raw_value);
            if replace_first_xml_element_text(&mut xml_text, key, &normalized) {
                modified += 1;
            } else {
                insert_meta_property_before_child_objects(&mut xml_text, key, &normalized)?;
                modified += 1;
            }
        }
        write_utf8_bom(&object_path, &xml_text)?;
        let stdout = format!(
            "\n=== meta-edit summary ===\n  Object:   {object_type}.{object_name}\n  Added:    0\n  Removed:  0\n  Modified: {modified}\n"
        );
        Ok((stdout, object_path, modified))
    })();

    match edit_result {
        Ok((stdout, object_path, _modified)) => AdapterOutcome {
            ok: true,
            summary: "unica.meta.edit completed with native metadata editor".to_string(),
            changes: vec![format!("updated {}", object_path.display())],
            warnings: Vec::new(),
            errors: Vec::new(),
            artifacts: vec![object_path.display().to_string()],
            stdout: Some(stdout),
            stderr: None,
            command: None,
        },
        Err(error) => AdapterOutcome {
            ok: false,
            summary: "unica.meta.edit failed in native metadata editor".to_string(),
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

pub(crate) fn resolve_meta_edit_object_path(raw: &Path, cwd: &Path) -> Result<PathBuf, String> {
    let mut path = absolutize(raw.to_path_buf(), cwd);
    if path.is_dir() {
        let dir_name = path
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or_default()
            .to_string();
        let candidate = path.join(format!("{dir_name}.xml"));
        let sibling = path
            .parent()
            .map(|parent| parent.join(format!("{dir_name}.xml")));
        if candidate.exists() {
            path = candidate;
        } else if let Some(sibling) = sibling.filter(|candidate| candidate.exists()) {
            path = sibling;
        } else {
            return Err(format!(
                "Directory given but no {dir_name}.xml found inside or as sibling"
            ));
        }
    }

    if !path.exists() {
        let file_name = path
            .file_stem()
            .and_then(|value| value.to_str())
            .unwrap_or_default()
            .to_string();
        let parent_dir = path.parent();
        let parent_dir_name = parent_dir
            .and_then(|parent| parent.file_name())
            .and_then(|value| value.to_str())
            .unwrap_or_default();
        if file_name == parent_dir_name {
            if let Some(grandparent) = parent_dir.and_then(Path::parent) {
                let candidate = grandparent.join(format!("{file_name}.xml"));
                if candidate.exists() {
                    path = candidate;
                }
            }
        }
    }

    if !path.exists() {
        return Err(format!("Object file not found: {}", raw.display()));
    }
    Ok(path)
}

pub(crate) fn meta_edit_object_identity(xml_text: &str) -> Result<(String, String), String> {
    let doc = Document::parse(xml_text.trim_start_matches('\u{feff}'))
        .map_err(|err| format!("XML parse error: {err}"))?;
    let root = doc.root_element();
    if root.tag_name().name() != "MetaDataObject" {
        return Err(format!(
            "Root element must be MetaDataObject, got: {}",
            root.tag_name().name()
        ));
    }
    let object = root
        .children()
        .find(|node| node.is_element())
        .ok_or_else(|| "No object element found under MetaDataObject".to_string())?;
    let object_type = object.tag_name().name().to_string();
    let object_name = object
        .descendants()
        .find(|node| node.is_element() && node.tag_name().name() == "Name")
        .and_then(|node| node.text())
        .unwrap_or("")
        .to_string();
    Ok((object_type, object_name))
}

pub(crate) fn normalize_meta_edit_property_value(key: &str, value: &str) -> String {
    match key {
        "HierarchyType" => normalize_meta_enum_value(value),
        "DefaultPresentation" => normalize_meta_enum_value(value),
        "DataLockControlMode" => normalize_meta_enum_value(value),
        "FullTextSearch" => normalize_meta_enum_value(value),
        "Posting" => normalize_meta_enum_value(value),
        "EditType" => normalize_meta_enum_value(value),
        _ => value.to_string(),
    }
}

pub(crate) fn invoke_read(
    operation: &str,
    _tool_name: &str,
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> Option<Result<AdapterOutcome, String>> {
    match operation {
        "meta-info" => Some(Ok(analyze_meta_info(args, context))),
        "meta-validate" => Some(Ok(validate_meta(args, context))),
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
        "meta-compile" => Some(compile_meta(args, context)),
        "meta-edit" => Some(edit_meta(args, context)),
        "meta-remove" => Some(remove_metadata_object(args, context)),
        _ => None,
    }
}
