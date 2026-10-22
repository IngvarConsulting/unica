//! Actual Designer subsystem parsing, read projections, validation and in-memory edit algorithms.
use roxmltree::Document;
use serde_json::{Map, Value};
use std::collections::{BTreeMap, HashSet};
use std::ffi::OsStr;
use std::path::{Component, Path, PathBuf};
use unica_metadata::subsystem::*;
use unica_metadata::subsystem_topology::*;
use unica_metadata::support_state::ObjectSupportData as DomainObjectSupportData;
use unica_source_contract::discovery::{FollowMode, ProjectDiscoveryRead};
use unica_source_designer::format_profile::{
    classify_root_version, FormatCompatibility, ACTIVE_FORMAT_PROFILE,
};
use unica_source_designer::layout::full_md_namespace_declarations;
use unica_source_designer::read::{path_exists, path_is_dir, read_bytes, read_string};
use unica_source_designer::roots::root_version_literal;

pub struct SubsystemValidationReport {
    pub errors: usize,
    pub warnings: usize,
    pub ok_count: usize,
    pub detailed: bool,
    pub lines: Vec<String>,
}

impl SubsystemValidationReport {
    pub fn new(detailed: bool) -> Self {
        Self {
            errors: 0,
            warnings: 0,
            ok_count: 0,
            detailed,
            lines: Vec::new(),
        }
    }

    pub fn out(&mut self, msg: impl Into<String>) {
        self.lines.push(msg.into());
    }

    pub fn ok(&mut self, msg: impl AsRef<str>) {
        self.ok_count += 1;
        if self.detailed {
            self.lines.push(format!("[OK]    {}", msg.as_ref()));
        }
    }

    pub fn error(&mut self, msg: impl AsRef<str>) {
        self.errors += 1;
        self.lines.push(format!("[ERROR] {}", msg.as_ref()));
    }

    pub fn warn(&mut self, msg: impl AsRef<str>) {
        self.warnings += 1;
        self.lines.push(format!("[WARN]  {}", msg.as_ref()));
    }

    pub fn finish(mut self, sub_name: &str) -> String {
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

pub fn validate_subsystem_metadata_name(argument: &str, value: &str) -> Result<(), String> {
    let mut components = Path::new(value).components();
    let is_single_path_component = matches!(
        components.next(),
        Some(Component::Normal(component)) if component == OsStr::new(value)
    ) && components.next().is_none();

    if unica_xml_format::names::xml_ncname_is_valid(value) && is_single_path_component {
        Ok(())
    } else {
        Err(format!(
            "{argument} must be a valid Unicode XML NCName and a single path component: {value:?}"
        ))
    }
}

pub fn canonical_subsystem_boolean(value: &Value, property: &str) -> Result<String, String> {
    match value {
        Value::Bool(value) => Ok(value.to_string()),
        Value::String(value) if value == "true" || value == "false" => Ok(value.clone()),
        _ => Err(format!(
            "{property} must be a JSON boolean or the canonical string true or false"
        )),
    }
}

pub fn subsystem_command_interface_path(subsystem_xml: &Path) -> PathBuf {
    subsystem_dir_for_xml(subsystem_xml)
        .join("Ext")
        .join("CommandInterface.xml")
}

/// Return the exact version-owning XML candidates read by subsystem semantic
/// validation for the supplied descriptor paths. A subsystem validator reads
/// its descriptor and, when present, the direct Ext/CommandInterface.xml
/// sidecar. Configuration.xml uses the configuration validator instead.
pub fn subsystem_validation_format_dependency_paths(descriptor_paths: &[&Path]) -> Vec<PathBuf> {
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
/// A subsystem descriptor is itself what the reader parses, so the address maps
/// to the descriptor rather than to something under an `Ext/` directory.
pub const SUBSYSTEM_KINDS: &[&str] = &["Subsystem"];

pub fn subsystem_edit_operations(
    reader: &dyn ProjectDiscoveryRead,
    args: &Map<String, Value>,
    cwd: &Path,
    operation: Option<&str>,
    definition_file: Option<PathBuf>,
) -> Result<Vec<(String, Value)>, String> {
    if let Some(definition_file) = definition_file {
        let definition_file = unica_operation_contract::input::absolutize(definition_file, cwd);
        let text = read_string(reader, &definition_file)
            .map_err(|err| format!("failed to read {}: {err}", definition_file.display()))?;
        let parsed: Value = serde_json::from_str(text.trim_start_matches('\u{feff}'))
            .map_err(|err| format!("failed to parse {}: {err}", definition_file.display()))?;
        Ok(subsystem_edit_operations_from_value(parsed, operation))
    } else {
        Ok(vec![(
            operation.unwrap_or("").to_string(),
            Value::String(
                unica_operation_contract::input::string_arg(args, &["value", "Value"])
                    .unwrap_or_default()
                    .to_string(),
            ),
        )])
    }
}

pub fn subsystem_edit_operations_from_value(
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

pub fn subsystem_edit_ml_text(props: roxmltree::Node<'_, '_>, tag: &str) -> String {
    unica_xml_format::read::child_element(props, tag)
        .and_then(|node| {
            node.descendants()
                .find(|child| unica_xml_format::names::element_has_name(*child, "content", None))
                .and_then(|child| child.text())
                .or_else(|| node.text())
        })
        .map(str::trim)
        .unwrap_or("")
        .to_string()
}

pub fn subsystem_edit_picture_text(props: roxmltree::Node<'_, '_>) -> String {
    unica_xml_format::read::child_element(props, "Picture")
        .and_then(|node| {
            node.children()
                .find(|child| unica_xml_format::names::element_has_name(*child, "Ref", None))
                .and_then(|child| child.text())
                .or_else(|| node.text())
        })
        .map(str::trim)
        .unwrap_or("")
        .to_string()
}

pub fn subsystem_edit_value_list(value: &Value) -> Result<Vec<String>, String> {
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
        _ => Ok(vec![
            unica_operation_contract::input::json_value_to_python_string(value),
        ]),
    }
}

pub fn subsystem_edit_array_strings(value: &Value) -> Result<Vec<String>, String> {
    let Some(items) = value.as_array() else {
        return Err("value must be an array".to_string());
    };
    Ok(items
        .iter()
        .map(unica_operation_contract::input::json_value_to_python_string)
        .collect())
}

pub fn subsystem_edit_object(value: &Value) -> Result<Value, String> {
    if value.is_object() {
        Ok(value.clone())
    } else if let Some(text) = value.as_str() {
        serde_json::from_str(text).map_err(|err| format!("failed to parse JSON value: {err}"))
    } else {
        Err("value must be a JSON object".to_string())
    }
}

pub fn subsystem_edit_add_content(
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

pub fn subsystem_edit_remove_content(
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

/// Borrows existing mutation outputs; it does not own additional staged state.
pub struct SubsystemChildEffects<'a> {
    pub counters: &'a mut SubsystemEditCounters,
    pub log: &'a mut SubsystemEditLog,
    pub child_stubs: &'a mut BTreeMap<PathBuf, String>,
    pub reused_child_subsystems: &'a mut BTreeMap<PathBuf, Vec<u8>>,
}

pub fn subsystem_edit_add_child(
    reader: &dyn ProjectDiscoveryRead,
    model: &mut SubsystemEditModel,
    resolved_path: &Path,
    value: &Value,
    effects: SubsystemChildEffects<'_>,
    fresh_uuid: fn() -> String,
) -> Result<(), String> {
    let SubsystemChildEffects {
        counters,
        log,
        child_stubs,
        reused_child_subsystems,
    } = effects;

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
    match reader.inspect_input(&child_xml, FollowMode::NoFollow) {
        Ok(metadata) if metadata.is_file() && !metadata.file_type().is_symlink() => {
            let preimage = read_bytes(reader, &child_xml).map_err(|error| {
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
                child_subsystem_stub_xml(&child_name, &model.version, fresh_uuid),
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

pub fn subsystem_edit_remove_child(
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

pub fn subsystem_edit_set_property(
    model: &mut SubsystemEditModel,
    value: &Value,
    counters: &mut SubsystemEditCounters,
    log: &mut SubsystemEditLog,
) -> Result<(), String> {
    let value = subsystem_edit_object(value)?;
    let prop_name = value
        .get("name")
        .map(unica_operation_contract::input::json_value_to_python_string)
        .ok_or_else(|| "set-property requires {name, value}".to_string())?;
    let raw_prop_value = value.get("value");
    let prop_value = raw_prop_value
        .map(unica_operation_contract::input::json_value_to_python_string)
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

/// Validate captured XML with dependencies supplied by the caller's source authority.
/// Paths label dependencies; this function never opens them itself.
pub fn validate_subsystem_content(
    text: &str,
    xml_path: PathBuf,
    detailed: bool,
    mut exists: impl FnMut(&Path) -> Result<bool, String>,
    mut read_optional: impl FnMut(&Path) -> Result<Option<String>, String>,
) -> Result<(bool, String, PathBuf), String> {
    let mut report = SubsystemValidationReport::new(detailed);
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
        Ok(compatibility) => report.warn(
            unica_source_designer::format_profile::format_compatibility_warning(&compatibility),
        ),
        Err(error) => report.error(error.to_string()),
    }
    let Some(sub) = root.children().find(|node| {
        unica_xml_format::names::element_has_name(
            *node,
            "Subsystem",
            Some("http://v8.1c.ru/8.3/MDClasses"),
        )
    }) else {
        report.error("1. Root structure: expected MetaDataObject/Subsystem, not found");
        let result = report.finish("");
        return Ok((false, result, xml_path));
    };
    let uuid_val = sub.attribute("uuid").unwrap_or("");
    if !uuid_val.is_empty() && unica_xml_format::names::is_valid_uuid(uuid_val) {
        report.ok(format!(
            "1. Root structure: MetaDataObject/Subsystem, uuid={uuid_val}, version {version}"
        ));
    } else {
        report.error("1. Root structure: invalid or missing uuid");
    }

    let Some(props) = sub.children().find(|node| {
        unica_xml_format::names::element_has_name(
            *node,
            "Properties",
            Some("http://v8.1c.ru/8.3/MDClasses"),
        )
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
                !unica_xml_format::names::element_has_name(
                    node,
                    prop,
                    Some("http://v8.1c.ru/8.3/MDClasses"),
                )
            })
        })
        .copied()
        .collect::<Vec<_>>();
    if missing.is_empty() {
        report.ok("2. Properties: all 9 required properties present");
    } else {
        report.error(format!("2. Properties: missing: {}", missing.join(", ")));
    }

    let sub_name =
        unica_xml_format::read::child_text(props, "Name", Some("http://v8.1c.ru/8.3/MDClasses"))
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

    if let Some(syn) = props.children().find(|node| {
        unica_xml_format::names::element_has_name(
            *node,
            "Synonym",
            Some("http://v8.1c.ru/8.3/MDClasses"),
        )
    }) {
        let items = syn
            .children()
            .filter(|node| unica_xml_format::names::element_has_name(*node, "item", None));
        let mut item_count = 0usize;
        let mut first_content = String::new();
        for item in items {
            item_count += 1;
            if first_content.is_empty() {
                if let Some(content) = item
                    .children()
                    .find(|node| unica_xml_format::names::element_has_name(*node, "content", None))
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
        if let Some(node) = props.children().find(|node| {
            unica_xml_format::names::element_has_name(
                *node,
                prop,
                Some("http://v8.1c.ru/8.3/MDClasses"),
            )
        }) {
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
    if let Some(content) = props.children().find(|node| {
        unica_xml_format::names::element_has_name(
            *node,
            "Content",
            Some("http://v8.1c.ru/8.3/MDClasses"),
        )
    }) {
        let xr_items = content
            .children()
            .filter(|node| unica_xml_format::names::element_has_name(*node, "Item", None))
            .collect::<Vec<_>>();
        if xr_items.is_empty() {
            report.ok("6. Content: empty (no items)");
        } else {
            let mut content_ok = true;
            for item in &xr_items {
                let type_attr =
                    unica_xml_format::read::attribute_by_local_name(*item, "type").unwrap_or("");
                let text = item.text().unwrap_or("").trim().to_string();
                content_items.push(text.clone());
                if type_attr != "xr:MDObjectRef" {
                    report.error(format!(
                            "6. Content item \"{text}\": xsi:type=\"{type_attr}\" (expected xr:MDObjectRef)"
                        ));
                    content_ok = false;
                }
                if !is_subsystem_content_ref(&text)
                    && !unica_xml_format::names::is_valid_uuid(&text)
                {
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
        let duplicates =
            unica_operation_contract::collections::duplicates_preserve_order(&content_items);
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
        unica_xml_format::names::element_has_name(
            *node,
            "ChildObjects",
            Some("http://v8.1c.ru/8.3/MDClasses"),
        )
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
        let duplicates =
            unica_operation_contract::collections::duplicates_preserve_order(&child_names);
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
            .map(|name| {
                exists(&subs_dir.join(format!("{name}.xml")))
                    .map(|present| (!present).then(|| name.clone()))
            })
            .collect::<Result<Vec<_>, String>>()?
            .into_iter()
            .flatten()
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
    match read_optional(&ci_path).and_then(|text| {
        text.map(|text| {
            Document::parse(text.trim_start_matches('\u{feff}'))
                .map(|_| ())
                .map_err(|error| error.to_string())
        })
        .transpose()
    }) {
        Ok(Some(())) => report.ok("11. CommandInterface: exists, well-formed"),
        Ok(None) => report.ok("11. CommandInterface: not present"),
        Err(error) => report.warn(format!(
            "11. CommandInterface: exists but NOT well-formed: {error}"
        )),
    }

    if let Some(picture) = props.children().find(|node| {
        unica_xml_format::names::element_has_name(
            *node,
            "Picture",
            Some("http://v8.1c.ru/8.3/MDClasses"),
        )
    }) {
        let children = picture
            .children()
            .filter(|node| node.is_element())
            .collect::<Vec<_>>();
        if children.is_empty() {
            report.ok("12. Picture: empty (not set)");
        } else if let Some(pic_ref) = children
            .iter()
            .find(|node| unica_xml_format::names::element_has_name(**node, "Ref", None))
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
}

pub fn build_subsystem_info_result(
    data: SubsystemInfoData,
    tree: Option<Vec<SubsystemTreeNode>>,
    command_interface: Option<SubsystemCommandInterfaceData>,
    support: DomainObjectSupportData,
) -> SubsystemInfoResult {
    SubsystemInfoResult {
        name: data.name,
        property_limits: data.property_limits,
        synonym: subsystem_optional(data.synonym),
        comment: data.comment,
        explanation: data.explanation,
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
    }
}

pub fn topology_format_documents(
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

pub fn reject_subsystem_info_parent_components(path: &Path) -> Result<(), String> {
    if path
        .components()
        .any(|component| matches!(component, Component::ParentDir))
    {
        return Err("SubsystemPath contains a parent path component (`..`)".to_string());
    }
    Ok(())
}

fn subsystem_optional(value: String) -> Option<String> {
    (!value.is_empty()).then_some(value)
}

pub fn subsystem_tree_node(node: &SubsystemTopologyNode) -> SubsystemTreeNode {
    SubsystemTreeNode {
        name: node.name.clone(),
        content: node.content.len(),
        children: node.children.iter().map(subsystem_tree_node).collect(),
    }
}

pub fn focused_subsystem_tree_from_topology(
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

pub fn subsystem_node_by_address<'a>(
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

pub fn focused_subsystem_tree_node(
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

/// Reads the command interface as data. `None` means the file is absent; an
/// interface that declares nothing yields empty lists instead.
pub fn subsystem_command_interface_data(
    reader: &dyn ProjectDiscoveryRead,
    subsystem_path: &Path,
) -> Result<Option<SubsystemCommandInterfaceData>, String> {
    let ci_path = subsystem_command_interface_path(subsystem_path);
    if !reader.path_is_file(&ci_path) {
        return Ok(None);
    }
    let text = read_string(reader, &ci_path)
        .map_err(|err| format!("failed to read {}: {err}", ci_path.display()))?;
    parse_subsystem_command_interface_data(&ci_path, &text).map(Some)
}

pub fn parse_subsystem_command_interface_data(
    ci_path: &Path,
    text: &str,
) -> Result<SubsystemCommandInterfaceData, String> {
    let doc = Document::parse(text.trim_start_matches('\u{feff}'))
        .map_err(|err| format!("XML parse error in {}: {err}", ci_path.display()))?;
    let root = doc.root_element();
    const CI_NS: &str = "http://v8.1c.ru/8.3/xcf/extrnprops";

    let mut visibility = Vec::new();
    if let Some(section) = root.children().find(|node| {
        unica_xml_format::names::element_has_name(*node, "CommandsVisibility", Some(CI_NS))
    }) {
        for cmd in section
            .children()
            .filter(|node| unica_xml_format::names::element_has_name(*node, "Command", Some(CI_NS)))
        {
            let common = cmd
                .descendants()
                .find(|node| unica_xml_format::names::element_has_name(*node, "Common", None))
                .and_then(|node| node.text());
            let role_overrides = cmd
                .descendants()
                .filter(|node| unica_xml_format::names::element_has_name(*node, "Value", None))
                .filter(|node| node.attribute("name").is_some_and(|name| !name.is_empty()))
                .count();
            visibility.push(SubsystemCommandVisibilityData {
                command: cmd.attribute("name").unwrap_or("").to_string(),
                visible: common != Some("false"),
                role_overrides,
            });
        }
    }

    let mut placement = Vec::new();
    if let Some(section) = root.children().find(|node| {
        unica_xml_format::names::element_has_name(*node, "CommandsPlacement", Some(CI_NS))
    }) {
        for cmd in section
            .children()
            .filter(|node| unica_xml_format::names::element_has_name(*node, "Command", Some(CI_NS)))
        {
            placement.push(SubsystemCommandPlacementData {
                command: cmd.attribute("name").unwrap_or("").to_string(),
                group: subsystem_optional(unica_xml_format::read::child_text(
                    cmd,
                    "CommandGroup",
                    Some(CI_NS),
                )),
                placement: subsystem_optional(unica_xml_format::read::child_text(
                    cmd,
                    "Placement",
                    Some(CI_NS),
                )),
            });
        }
    }

    let mut order = Vec::<(String, Vec<String>)>::new();
    if let Some(section) = root
        .children()
        .find(|node| unica_xml_format::names::element_has_name(*node, "CommandsOrder", Some(CI_NS)))
    {
        for cmd in section
            .children()
            .filter(|node| unica_xml_format::names::element_has_name(*node, "Command", Some(CI_NS)))
        {
            let group = unica_xml_format::read::child_text(cmd, "CommandGroup", Some(CI_NS));
            unica_operation_contract::collections::push_group_item(
                &mut order,
                if group.is_empty() { "?" } else { &group },
                cmd.attribute("name").unwrap_or("").to_string(),
            );
        }
    }

    let listed = |section: &str, item: &str| -> Vec<String> {
        root.children()
            .find(|node| unica_xml_format::names::element_has_name(*node, section, Some(CI_NS)))
            .into_iter()
            .flat_map(|section| section.children())
            .filter(|node| unica_xml_format::names::element_has_name(*node, item, Some(CI_NS)))
            .filter_map(|node| node.text())
            .map(|text| text.trim().to_string())
            .filter(|value| !value.is_empty())
            .collect()
    };

    Ok(SubsystemCommandInterfaceData {
        visibility,
        placement,
        order: order
            .into_iter()
            .map(|(name, items)| SubsystemGroupData { name, items })
            .collect(),
        groups_order: listed("GroupsOrder", "Group"),
        subsystem_order: listed("SubsystemsOrder", "Subsystem"),
    })
}

pub fn subsystem_info_ci_lines(
    reader: &dyn ProjectDiscoveryRead,
    sub_name: &str,
    subsystem_path: &Path,
) -> Result<Vec<String>, String> {
    let ci_path = subsystem_command_interface_path(subsystem_path);
    let mut lines = vec![format!("Командный интерфейс: {sub_name}"), String::new()];
    if !reader.path_is_file(&ci_path) {
        lines.push("Файл CommandInterface.xml не найден.".to_string());
        lines.push(format!("Путь: {}", ci_path.display()));
        return Ok(lines);
    }

    let text = read_string(reader, &ci_path)
        .map_err(|err| format!("failed to read {}: {err}", ci_path.display()))?;
    let doc = Document::parse(text.trim_start_matches('\u{feff}'))
        .map_err(|err| format!("XML parse error in {}: {err}", ci_path.display()))?;
    let root = doc.root_element();
    const CI_NS: &str = "http://v8.1c.ru/8.3/xcf/extrnprops";

    if let Some(section) = root.children().find(|node| {
        unica_xml_format::names::element_has_name(*node, "CommandsVisibility", Some(CI_NS))
    }) {
        let mut hidden = Vec::new();
        let mut shown = Vec::new();
        for cmd in section
            .children()
            .filter(|node| unica_xml_format::names::element_has_name(*node, "Command", Some(CI_NS)))
        {
            let name = cmd.attribute("name").unwrap_or("").to_string();
            let common = cmd
                .descendants()
                .find(|node| unica_xml_format::names::element_has_name(*node, "Common", None))
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

    if let Some(section) = root.children().find(|node| {
        unica_xml_format::names::element_has_name(*node, "CommandsPlacement", Some(CI_NS))
    }) {
        let placements = section
            .children()
            .filter(|node| unica_xml_format::names::element_has_name(*node, "Command", Some(CI_NS)))
            .map(|cmd| {
                let name = cmd.attribute("name").unwrap_or("");
                let group = unica_xml_format::read::child_text(cmd, "CommandGroup", Some(CI_NS));
                let placement = unica_xml_format::read::child_text(cmd, "Placement", Some(CI_NS));
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
        .find(|node| unica_xml_format::names::element_has_name(*node, "CommandsOrder", Some(CI_NS)))
    {
        let mut groups = Vec::<(String, Vec<String>)>::new();
        for cmd in section
            .children()
            .filter(|node| unica_xml_format::names::element_has_name(*node, "Command", Some(CI_NS)))
        {
            let name = cmd.attribute("name").unwrap_or("").to_string();
            let group = unica_xml_format::read::child_text(cmd, "CommandGroup", Some(CI_NS));
            unica_operation_contract::collections::push_group_item(
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

pub fn subsystem_info_tree(
    reader: &dyn ProjectDiscoveryRead,
    path: &Path,
    name_filter: &str,
) -> Result<(Vec<String>, PathBuf), String> {
    let mut lines = Vec::<String>::new();
    if path_is_dir(reader, path) {
        let label = path
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or("");
        lines.push(format!("Дерево подсистем от: {label}/"));
        lines.push(String::new());
        let mut files = reader
            .read_entries(path)
            .map_err(|err| format!("failed to read {}: {err}", path.display()))?
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|entry| {
                reader.path_is_file(entry)
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
            build_subsystem_tree_entry(
                reader,
                file,
                "",
                index == files.len() - 1,
                true,
                &mut lines,
            )?;
        }
        Ok((lines, path.to_path_buf()))
    } else {
        if !reader.path_is_file(path) {
            return Err(format!("[ERROR] File not found: {}", path.display()));
        }
        build_subsystem_tree_entry(reader, path, "", true, true, &mut lines)?;
        Ok((lines, path.to_path_buf()))
    }
}

pub fn subsystem_dir_for_xml(xml_path: &Path) -> PathBuf {
    let dir = xml_path.parent().unwrap_or_else(|| Path::new(""));
    let base = xml_path
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("");
    dir.join(base)
}

pub fn subsystem_content_items(props: roxmltree::Node<'_, '_>) -> Vec<String> {
    props
        .children()
        .find(|node| {
            unica_xml_format::names::element_has_name(
                *node,
                "Content",
                Some("http://v8.1c.ru/8.3/MDClasses"),
            )
        })
        .map(|content| {
            content
                .children()
                .filter(|node| unica_xml_format::names::element_has_name(*node, "Item", None))
                .filter_map(|node| node.text().map(ToOwned::to_owned))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default()
}

pub fn subsystem_child_names(sub: roxmltree::Node<'_, '_>) -> Vec<String> {
    sub.children()
        .find(|node| {
            unica_xml_format::names::element_has_name(
                *node,
                "ChildObjects",
                Some("http://v8.1c.ru/8.3/MDClasses"),
            )
        })
        .map(|children| {
            children
                .children()
                .filter(|node| unica_xml_format::names::element_has_name(*node, "Subsystem", None))
                .map(|node| node.text().unwrap_or("").to_string())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default()
}

pub fn subsystem_group_content(items: &[String]) -> Vec<(String, Vec<String>)> {
    let mut groups = Vec::<(String, Vec<String>)>::new();
    for item in items {
        let (type_name, name) = if let Some((type_name, name)) = item.split_once('.') {
            (type_name.to_string(), name.to_string())
        } else if unica_xml_format::names::looks_like_uuid_prefix(item) {
            ("[UUID]".to_string(), item.clone())
        } else {
            ("[Other]".to_string(), item.clone())
        };
        unica_operation_contract::collections::push_group_item(&mut groups, &type_name, name);
    }
    groups
}

pub fn subsystem_known_plural_types() -> &'static [&'static str] {
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

pub fn child_subsystem_stub_xml(
    child_name: &str,
    format_version: &str,
    fresh_uuid: fn() -> String,
) -> String {
    let subsystem_uuid = fresh_uuid();
    let mut lines = Vec::new();
    lines.push("<?xml version=\"1.0\" encoding=\"UTF-8\"?>".to_string());
    lines.push(format!(
        "<MetaDataObject xmlns=\"http://v8.1c.ru/8.3/MDClasses\" xmlns:app=\"http://v8.1c.ru/8.2/managed-application/core\" xmlns:cfg=\"http://v8.1c.ru/8.1/data/enterprise/current-config\" xmlns:cmi=\"http://v8.1c.ru/8.2/managed-application/cmi\" xmlns:ent=\"http://v8.1c.ru/8.1/data/enterprise\" xmlns:lf=\"http://v8.1c.ru/8.2/managed-application/logform\" xmlns:style=\"http://v8.1c.ru/8.1/data/ui/style\" xmlns:sys=\"http://v8.1c.ru/8.1/data/ui/fonts/system\" xmlns:v8=\"http://v8.1c.ru/8.1/data/core\" xmlns:v8ui=\"http://v8.1c.ru/8.1/data/ui\" xmlns:web=\"http://v8.1c.ru/8.1/data/ui/colors/web\" xmlns:win=\"http://v8.1c.ru/8.1/data/ui/colors/windows\" xmlns:xen=\"http://v8.1c.ru/8.3/xcf/enums\" xmlns:xpr=\"http://v8.1c.ru/8.3/xcf/predef\" xmlns:xr=\"http://v8.1c.ru/8.3/xcf/readable\" xmlns:xs=\"http://www.w3.org/2001/XMLSchema\" xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\" version=\"{format_version}\">"
    ));
    lines.push(format!("\t<Subsystem uuid=\"{}\">", subsystem_uuid));
    lines.push("\t\t<Properties>".to_string());
    lines.push(format!(
        "\t\t\t<Name>{}</Name>",
        unica_xml_format::text::escape_xml(child_name)
    ));
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

pub fn normalize_subsystem_content_ref(raw: &str) -> String {
    let Some(dot_idx) = raw.find('.') else {
        return raw.to_string();
    };
    let type_part = &raw[..dot_idx];
    let name_part = &raw[dot_idx + 1..];
    let normalized = subsystem_content_type(type_part).unwrap_or(type_part);
    format!("{normalized}.{name_part}")
}

pub fn subsystem_content_type(type_part: &str) -> Option<&'static str> {
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

pub fn resolve_subsystem_edit_xml(
    reader: &dyn ProjectDiscoveryRead,
    mut path: PathBuf,
) -> Result<PathBuf, String> {
    if path_is_dir(reader, &path) {
        let dir_name = path
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or("")
            .to_string();
        let candidate = path.join(format!("{dir_name}.xml"));
        let sibling = path
            .parent()
            .map(|parent| parent.join(format!("{dir_name}.xml")))
            .unwrap_or_else(|| PathBuf::from(format!("{dir_name}.xml")));
        if reader.path_is_file(&candidate) {
            path = candidate;
        } else if reader.path_is_file(&sibling) {
            path = sibling;
        } else {
            return Err(format!(
                "No {dir_name}.xml found in directory or as sibling"
            ));
        }
    }

    if !reader.path_is_file(&path) {
        let stem = path
            .file_stem()
            .and_then(|value| value.to_str())
            .unwrap_or("");
        let parent = path.parent().unwrap_or_else(|| Path::new(""));
        if stem
            == parent
                .file_name()
                .and_then(|value| value.to_str())
                .unwrap_or("")
        {
            if let Some(grand) = parent.parent() {
                let candidate = grand.join(format!("{stem}.xml"));
                if reader.path_is_file(&candidate) {
                    path = candidate;
                }
            }
        }
    }

    if !reader.path_is_file(&path) {
        return Err(format!("File not found: {}", path.display()));
    }
    Ok(reader.canonicalize(&path).unwrap_or(path))
}
pub fn load_subsystem_edit_model(
    reader: &dyn ProjectDiscoveryRead,
    path: &Path,
) -> Result<SubsystemEditModel, String> {
    let text = read_string(reader, path)
        .map_err(|err| format!("failed to read {}: {err}", path.display()))?;
    parse_subsystem_edit_model(&text, &path.display().to_string())
}
/// Parses a subsystem descriptor already in memory; `label` names the source
/// in error messages.
pub fn parse_subsystem_edit_model(text: &str, label: &str) -> Result<SubsystemEditModel, String> {
    let doc = Document::parse(text.trim_start_matches('\u{feff}'))
        .map_err(|err| format!("XML parse error in {label}: {err}"))?;
    let root = doc.root_element();
    if root.tag_name().name() != "MetaDataObject" {
        return Err(format!(
            "Expected <MetaDataObject> root element, got <{}>",
            root.tag_name().name()
        ));
    }
    let Some(sub) = root.children().find(|node| {
        unica_xml_format::names::element_has_name(
            *node,
            "Subsystem",
            Some("http://v8.1c.ru/8.3/MDClasses"),
        )
    }) else {
        return Err("No <Subsystem> element found".to_string());
    };
    let Some(props) = unica_xml_format::read::child_element(sub, "Properties") else {
        return Err("No <Properties> element found".to_string());
    };
    let Some(child_objects) = unica_xml_format::read::child_element(sub, "ChildObjects") else {
        return Err("No <ChildObjects> element found".to_string());
    };

    let content = unica_xml_format::read::child_element(props, "Content")
        .map(|content| {
            content
                .children()
                .filter(|node| {
                    unica_xml_format::names::element_has_name(
                        *node,
                        "Item",
                        Some("http://v8.1c.ru/8.3/xcf/readable"),
                    )
                })
                .filter_map(|node| node.text())
                .map(str::trim)
                .map(ToOwned::to_owned)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let children = child_objects
        .children()
        .filter(|node| {
            unica_xml_format::names::element_has_name(
                *node,
                "Subsystem",
                Some("http://v8.1c.ru/8.3/MDClasses"),
            )
        })
        .filter_map(|node| node.text())
        .map(str::trim)
        .map(ToOwned::to_owned)
        .collect::<Vec<_>>();

    Ok(SubsystemEditModel {
        version: root
            .attribute("version")
            .unwrap_or(ACTIVE_FORMAT_PROFILE.export_format)
            .to_string(),
        uuid: sub
            .attribute("uuid")
            .map(ToOwned::to_owned)
            .unwrap_or_else(|| unica_xml_format::names::stable_uuid(70)),
        name: unica_xml_format::read::child_direct_text(props, "Name").unwrap_or_default(),
        synonym: subsystem_edit_ml_text(props, "Synonym"),
        comment: unica_xml_format::read::child_direct_text(props, "Comment").unwrap_or_default(),
        include_help: unica_xml_format::read::child_direct_text(props, "IncludeHelpInContents")
            .unwrap_or_else(|| "true".to_string()),
        include_ci: unica_xml_format::read::child_direct_text(props, "IncludeInCommandInterface")
            .unwrap_or_else(|| "true".to_string()),
        use_one_command: unica_xml_format::read::child_direct_text(props, "UseOneCommand")
            .unwrap_or_else(|| "false".to_string()),
        explanation: subsystem_edit_ml_text(props, "Explanation"),
        picture: subsystem_edit_picture_text(props),
        picture_load_transparent: unica_xml_format::read::child_element(props, "Picture")
            .and_then(|picture| {
                picture.children().find(|child| {
                    unica_xml_format::names::element_has_name(*child, "LoadTransparent", None)
                })
            })
            .and_then(|node| node.text())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned),
        content,
        children,
    })
}
/// Change only subsystem membership. Existing entries and all outside XML stay raw.
pub fn patch_subsystem_lists(
    text: &str,
    model: &SubsystemEditModel,
    original: Option<&str>,
) -> Result<String, String> {
    const MD: &str = "http://v8.1c.ru/8.3/MDClasses";
    const XR: &str = "http://v8.1c.ru/8.3/xcf/readable";
    const XSI: &str = "http://www.w3.org/2001/XMLSchema-instance";
    fn child<'a, 'input>(
        node: roxmltree::Node<'a, 'input>,
        name: &str,
    ) -> Result<Option<roxmltree::Node<'a, 'input>>, String> {
        let mut found = node.children().filter(|n| n.has_tag_name((MD, name)));
        let first = found.next();
        if found.next().is_some() {
            return Err(format!("ambiguous subsystem {name}"));
        }
        Ok(first)
    }
    fn lexical_name<'a>(body: &'a str, node: roxmltree::Node<'_, '_>) -> &'a str {
        body[node.range().start + 1..]
            .split([' ', '\t', '\r', '\n', '/', '>'])
            .next()
            .unwrap()
    }
    fn insert(
        body: &str,
        node: roxmltree::Node<'_, '_>,
        value: &str,
    ) -> Result<(std::ops::Range<usize>, String), String> {
        let range = node.range();
        let raw = &body[range.clone()];
        if raw.ends_with("/>") {
            let at = range.end - 2;
            Ok((
                at..range.end,
                format!(">{value}</{}>", lexical_name(body, node)),
            ))
        } else {
            let at = raw
                .rfind("</")
                .ok_or("subsystem container has no closing tag")?
                + range.start;
            Ok((at..at, value.to_owned()))
        }
    }
    let body = text.strip_prefix('\u{feff}').unwrap_or(text);
    let doc = Document::parse(body).map_err(|e| format!("XML parse error: {e}"))?;
    if !doc.root_element().has_tag_name((MD, "MetaDataObject")) {
        return Err("Expected MDClasses MetaDataObject root".to_owned());
    }
    let sub = child(doc.root_element(), "Subsystem")?.ok_or("No Subsystem element")?;
    let props = child(sub, "Properties")?.ok_or("No Properties element")?;
    let children = child(sub, "ChildObjects")?.ok_or("No ChildObjects element")?;
    let content = child(props, "Content")?;
    let original_body = original.map(|s| s.strip_prefix('\u{feff}').unwrap_or(s));
    let original_doc = original_body
        .map(Document::parse)
        .transpose()
        .map_err(|e| format!("XML parse error: {e}"))?;
    let mut edits = Vec::new();
    for (container, parent, name, item_namespace, item_name, desired) in [
        (content, props, "Content", XR, "Item", &model.content),
        (
            Some(children),
            sub,
            "ChildObjects",
            MD,
            "Subsystem",
            &model.children,
        ),
    ] {
        let entries: Vec<_> = container
            .into_iter()
            .flat_map(|n| n.children())
            .filter(|n| n.has_tag_name((item_namespace, item_name)))
            .collect();
        if let (Some(base_doc), Some(base_body)) = (&original_doc, original_body) {
            let base_sub = child(base_doc.root_element(), "Subsystem")?
                .ok_or("No original Subsystem element")?;
            let base_parent = if name == "Content" {
                child(base_sub, "Properties")?.ok_or("No original Properties element")?
            } else {
                base_sub
            };
            let base_container = child(base_parent, name)?;
            let base_items: Vec<_> = base_container
                .into_iter()
                .flat_map(|n| n.children())
                .filter(|n| n.has_tag_name((item_namespace, item_name)))
                .map(|n| n.text().unwrap_or("").trim())
                .collect();
            // An intervening editor may have changed inherited namespace
            // bindings. Never transplant raw prefixed XML into a different
            // context; edit the current list normally in that case.
            let bindings_compatible = base_container.is_none_or(|base| {
                let current_scope = container.unwrap_or(parent);
                base.namespaces()
                    .all(|ns| current_scope.lookup_namespace_uri(ns.name()) == Some(ns.uri()))
            });
            if bindings_compatible
                && base_items
                    .iter()
                    .copied()
                    .eq(desired.iter().map(String::as_str))
            {
                if let Some(node) = container {
                    let raw = base_container.map_or("", |n| &base_body[n.range()]);
                    edits.push((node.range(), raw.to_owned()));
                }
                continue;
            }
        }
        let current: Vec<_> = entries
            .iter()
            .map(|n| n.text().unwrap_or("").trim())
            .collect();
        if current
            .iter()
            .copied()
            .eq(desired.iter().map(String::as_str))
        {
            continue;
        }
        for entry in &entries {
            if !desired
                .iter()
                .any(|v| v == entry.text().unwrap_or("").trim())
            {
                edits.push((entry.range(), String::new()));
            }
        }
        let scope = container.unwrap_or(parent);
        let prefix = scope.lookup_prefix(item_namespace);
        let tag = prefix.map_or_else(|| item_name.to_owned(), |p| format!("{p}:{item_name}"));
        let mut added = String::new();
        for value in desired.iter().filter(|v| !current.contains(&v.as_str())) {
            if item_namespace == XR {
                // A QName in xsi:type must bind to the readable namespace too.
                let xsi_prefix = scope.lookup_prefix(XSI);
                let xr = prefix.unwrap_or(if xsi_prefix == Some("xr") {
                    "xr1"
                } else {
                    "xr"
                });
                let xsi = xsi_prefix.unwrap_or(if xr == "xsi" { "xsi1" } else { "xsi" });
                let xr_decl = if prefix.is_none() {
                    format!(" xmlns:{xr}=\"{XR}\"")
                } else {
                    String::new()
                };
                let xsi_decl = if scope.lookup_prefix(XSI).is_none() {
                    format!(" xmlns:{xsi}=\"{XSI}\"")
                } else {
                    String::new()
                };
                added.push_str(&format!(
                    "<{xr}:Item{xr_decl}{xsi_decl} {xsi}:type=\"{xr}:MDObjectRef\">{}</{xr}:Item>",
                    unica_xml_format::text::escape_xml(value)
                ));
            } else {
                added.push_str(&format!(
                    "<{tag}>{}</{tag}>",
                    unica_xml_format::text::escape_xml(value)
                ));
            }
        }
        if !added.is_empty() {
            if let Some(node) = container {
                edits.push(insert(body, node, &added)?);
            } else {
                let prefix = parent.lookup_prefix(MD);
                let tag = prefix.map_or_else(|| name.to_owned(), |p| format!("{p}:{name}"));
                edits.push(insert(body, parent, &format!("<{tag}>{added}</{tag}>"))?);
            }
        }
    }
    edits.sort_by_key(|a| std::cmp::Reverse(a.0.start));
    let mut updated = body.to_owned();
    for (range, replacement) in edits {
        updated.replace_range(range, &replacement);
    }
    if body.len() != text.len() {
        updated.insert(0, '\u{feff}');
    }
    Ok(updated)
}
pub fn emit_subsystem_edit_model(model: &SubsystemEditModel) -> String {
    let mut lines = Vec::new();
    lines.push("<?xml version=\"1.0\" encoding=\"UTF-8\"?>".to_string());
    lines.push(format!(
        "<MetaDataObject {} version=\"{}\">",
        full_md_namespace_declarations(),
        unica_xml_format::text::escape_xml(&model.version)
    ));
    lines.push(format!(
        "\t<Subsystem uuid=\"{}\">",
        unica_xml_format::text::escape_xml(&model.uuid)
    ));
    lines.push("\t\t<Properties>".to_string());
    lines.push(format!(
        "\t\t\t<Name>{}</Name>",
        unica_xml_format::text::escape_xml(&model.name)
    ));
    emit_subsystem_edit_ml(&mut lines, "\t\t\t", "Synonym", &model.synonym);
    if model.comment.is_empty() {
        lines.push("\t\t\t<Comment/>".to_string());
    } else {
        lines.push(format!(
            "\t\t\t<Comment>{}</Comment>",
            unica_xml_format::text::escape_xml(&model.comment)
        ));
    }
    lines.push(format!(
        "\t\t\t<IncludeHelpInContents>{}</IncludeHelpInContents>",
        unica_xml_format::text::escape_xml(&model.include_help)
    ));
    lines.push(format!(
        "\t\t\t<IncludeInCommandInterface>{}</IncludeInCommandInterface>",
        unica_xml_format::text::escape_xml(&model.include_ci)
    ));
    lines.push(format!(
        "\t\t\t<UseOneCommand>{}</UseOneCommand>",
        unica_xml_format::text::escape_xml(&model.use_one_command)
    ));
    emit_subsystem_edit_ml(&mut lines, "\t\t\t", "Explanation", &model.explanation);
    if model.picture.is_empty() && model.picture_load_transparent.is_none() {
        lines.push("\t\t\t<Picture/>".to_string());
    } else {
        lines.push("\t\t\t<Picture>".to_string());
        if !model.picture.is_empty() {
            lines.push(format!(
                "\t\t\t\t<xr:Ref>{}</xr:Ref>",
                unica_xml_format::text::escape_xml(&model.picture)
            ));
        }
        if let Some(load_transparent) = &model.picture_load_transparent {
            lines.push(format!(
                "\t\t\t\t<xr:LoadTransparent>{}</xr:LoadTransparent>",
                unica_xml_format::text::escape_xml(load_transparent)
            ));
        }
        lines.push("\t\t\t</Picture>".to_string());
    }
    if model.content.is_empty() {
        lines.push("\t\t\t<Content/>".to_string());
    } else {
        lines.push("\t\t\t<Content>".to_string());
        for item in &model.content {
            lines.push(format!(
                "\t\t\t\t<xr:Item xsi:type=\"xr:MDObjectRef\">{}</xr:Item>",
                unica_xml_format::text::escape_xml(item)
            ));
        }
        lines.push("\t\t\t</Content>".to_string());
    }
    lines.push("\t\t</Properties>".to_string());
    if model.children.is_empty() {
        lines.push("\t\t<ChildObjects/>".to_string());
    } else {
        lines.push("\t\t<ChildObjects>".to_string());
        for child in &model.children {
            lines.push(format!(
                "\t\t\t<Subsystem>{}</Subsystem>",
                unica_xml_format::text::escape_xml(child)
            ));
        }
        lines.push("\t\t</ChildObjects>".to_string());
    }
    lines.push("\t</Subsystem>".to_string());
    lines.push("</MetaDataObject>".to_string());
    format!("{}\n", lines.join("\n"))
}
pub fn emit_subsystem_edit_ml(lines: &mut Vec<String>, indent: &str, tag: &str, text: &str) {
    if text.is_empty() {
        lines.push(format!("{indent}<{tag}/>"));
        return;
    }
    lines.push(format!("{indent}<{tag}>"));
    lines.push(format!("{indent}\t<v8:item>"));
    lines.push(format!("{indent}\t\t<v8:lang>ru</v8:lang>"));
    lines.push(format!(
        "{indent}\t\t<v8:content>{}</v8:content>",
        unica_xml_format::text::escape_xml(text)
    ));
    lines.push(format!("{indent}\t</v8:item>"));
    lines.push(format!("{indent}</{tag}>"));
}
pub fn resolve_subsystem_validate_xml(
    reader: &dyn ProjectDiscoveryRead,
    mut path: PathBuf,
) -> Result<PathBuf, String> {
    if path_is_dir(reader, &path) {
        let dir_name = path
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or("");
        let candidate = path.join(format!("{dir_name}.xml"));
        let sibling = path
            .parent()
            .map(|parent| parent.join(format!("{dir_name}.xml")))
            .unwrap_or_else(|| PathBuf::from(format!("{dir_name}.xml")));
        if path_exists(reader, &candidate) {
            path = candidate;
        } else if path_exists(reader, &sibling) {
            path = sibling;
        } else {
            return Err(format!(
                "[ERROR] No {dir_name}.xml found in directory: {}",
                path.display()
            ));
        }
    }

    if !path_exists(reader, &path) {
        let stem = path
            .file_stem()
            .and_then(|value| value.to_str())
            .unwrap_or("");
        let parent = path.parent().unwrap_or_else(|| Path::new(""));
        if stem
            == parent
                .file_name()
                .and_then(|value| value.to_str())
                .unwrap_or("")
        {
            if let Some(grand) = parent.parent() {
                let candidate = grand.join(format!("{stem}.xml"));
                if path_exists(reader, &candidate) {
                    path = candidate;
                }
            }
        }
    }

    if !path_exists(reader, &path) {
        return Err(format!("[ERROR] File not found: {}", path.display()));
    }
    Ok(path)
}
pub fn load_subsystem_info_data(
    reader: &dyn ProjectDiscoveryRead,
    path: &Path,
) -> Result<(SubsystemInfoData, Vec<String>), String> {
    let text = read_string(reader, path)
        .map_err(|err| format!("failed to read {}: {err}", path.display()))?;
    parse_subsystem_info_data(reader, path, &text)
}
pub fn parse_subsystem_info_data(
    reader: &dyn ProjectDiscoveryRead,
    path: &Path,
    text: &str,
) -> Result<(SubsystemInfoData, Vec<String>), String> {
    let has_ci = reader.path_is_file(
        &subsystem_dir_for_xml(path)
            .join("Ext")
            .join("CommandInterface.xml"),
    );
    parse_subsystem_info_xml(path, text, has_ci)
}
pub fn parse_subsystem_info_xml(
    path: &Path,
    text: &str,
    has_ci: bool,
) -> Result<(SubsystemInfoData, Vec<String>), String> {
    let doc = Document::parse(text.trim_start_matches('\u{feff}'))
        .map_err(|err| format!("XML parse error in {}: {err}", path.display()))?;
    let root = doc.root_element();
    let Some(sub) = root.children().find(|node| {
        unica_xml_format::names::element_has_name(
            *node,
            "Subsystem",
            Some("http://v8.1c.ru/8.3/MDClasses"),
        )
    }) else {
        return Err(format!(
            "[ERROR] Not a valid subsystem XML: {}",
            path.display()
        ));
    };
    let Some(props) = sub.children().find(|node| {
        unica_xml_format::names::element_has_name(
            *node,
            "Properties",
            Some("http://v8.1c.ru/8.3/MDClasses"),
        )
    }) else {
        return Err(format!(
            "[ERROR] Not a valid subsystem XML: {}",
            path.display()
        ));
    };

    let name =
        unica_xml_format::read::child_text(props, "Name", Some("http://v8.1c.ru/8.3/MDClasses"));
    let synonym = props
        .children()
        .find(|node| {
            unica_xml_format::names::element_has_name(
                *node,
                "Synonym",
                Some("http://v8.1c.ru/8.3/MDClasses"),
            )
        })
        .map(unica_xml_format::read::multilang_text)
        .unwrap_or_default();
    let mut property_limits = Vec::new();
    let comment = subsystem_text_property(props, "Comment", false, &mut property_limits);
    let include_ci = unica_xml_format::read::child_text(
        props,
        "IncludeInCommandInterface",
        Some("http://v8.1c.ru/8.3/MDClasses"),
    );
    let use_one_command = unica_xml_format::read::child_text(
        props,
        "UseOneCommand",
        Some("http://v8.1c.ru/8.3/MDClasses"),
    );
    let explanation = subsystem_text_property(props, "Explanation", true, &mut property_limits);
    let picture = props
        .children()
        .find(|node| {
            unica_xml_format::names::element_has_name(
                *node,
                "Picture",
                Some("http://v8.1c.ru/8.3/MDClasses"),
            )
        })
        .and_then(|node| {
            node.children()
                .find(|child| unica_xml_format::names::element_has_name(*child, "Ref", None))
                .and_then(|child| child.text())
        })
        .unwrap_or("")
        .to_string();
    let content_items = subsystem_content_items(props);
    let groups = subsystem_group_content(&content_items);
    let child_names = subsystem_child_names(sub);
    Ok((
        SubsystemInfoData {
            name,
            synonym,
            comment,
            include_ci,
            use_one_command,
            explanation,
            picture,
            content_items,
            groups,
            child_names,
            has_ci,
            property_limits,
        },
        Vec::new(),
    ))
}

fn subsystem_text_property(
    properties: roxmltree::Node<'_, '_>,
    name: &str,
    localized: bool,
    limits: &mut Vec<String>,
) -> Option<String> {
    use crate::info_projection::{direct_text_content, localized_string_is_valid};
    let nodes = properties
        .children()
        .filter(|node| node.is_element() && node.tag_name().name() == name)
        .collect::<Vec<_>>();
    if nodes.is_empty() {
        return None;
    }
    if let [node] = nodes.as_slice() {
        let direct_text = direct_text_content(*node);
        let valid = node.tag_name().namespace() == Some("http://v8.1c.ru/8.3/MDClasses")
            && node.attributes().len() == 0
            && if localized {
                direct_text.trim().is_empty() && localized_string_is_valid(*node)
            } else {
                !node.children().any(|child| child.is_element())
            };
        if valid {
            return Some(if localized {
                subsystem_localized_text(*node)
            } else {
                direct_text
            });
        }
    }
    limits.push(format!("source_unreadable: Subsystem properties.{name} is malformed, duplicated or has a foreign namespace"));
    None
}

fn subsystem_localized_text(node: roxmltree::Node<'_, '_>) -> String {
    use crate::info_projection::direct_text_content;
    let values = node
        .children()
        .filter(roxmltree::Node::is_element)
        .map(|item| {
            let text = |name| {
                item.children()
                    .find(|child| child.is_element() && child.tag_name().name() == name)
                    .map(direct_text_content)
                    .unwrap_or_default()
            };
            (text("lang"), text("content"))
        })
        .collect::<Vec<_>>();
    values
        .iter()
        .find(|(language, content)| language == "ru" && !content.is_empty())
        .or_else(|| values.iter().find(|(_, content)| !content.is_empty()))
        .map(|(_, content)| content.clone())
        .unwrap_or_default()
}

pub fn append_subsystem_overview(lines: &mut Vec<String>, data: &SubsystemInfoData) {
    lines.push(format!("Подсистема: {}", data.name));
    if !data.synonym.is_empty() && data.synonym != data.name {
        lines.push(format!("Синоним: {}", data.synonym));
    }
    if let Some(comment) = data.comment.as_deref().filter(|value| !value.is_empty()) {
        lines.push(format!("Комментарий: {comment}"));
    }
    lines.push(format!("ВключатьВКомандныйИнтерфейс: {}", data.include_ci));
    lines.push(format!("ИспользоватьОднуКоманду: {}", data.use_one_command));
    if let Some(explanation) = data
        .explanation
        .as_deref()
        .filter(|value| !value.is_empty())
    {
        lines.push(format!("Пояснение: {explanation}"));
    }
    if !data.picture.is_empty() {
        lines.push(format!("Картинка: {}", data.picture));
    }
    if data.content_items.is_empty() {
        lines.push("Состав: пусто".to_string());
    } else {
        let parts = data
            .groups
            .iter()
            .map(|(type_name, items)| format!("{type_name}: {}", items.len()))
            .collect::<Vec<_>>();
        lines.push(format!(
            "Состав: {} объектов ({})",
            data.content_items.len(),
            parts.join(", ")
        ));
    }
    if !data.child_names.is_empty() {
        lines.push(format!(
            "Дочерние подсистемы ({}): {}",
            data.child_names.len(),
            data.child_names.join(", ")
        ));
    }
    if data.has_ci {
        lines.push("Командный интерфейс: есть".to_string());
    }
}
pub fn append_subsystem_content(
    lines: &mut Vec<String>,
    data: &SubsystemInfoData,
    name_filter: &str,
) {
    lines.push(format!(
        "Состав подсистемы {} ({} объектов):",
        data.name,
        data.content_items.len()
    ));
    lines.push(String::new());
    if !name_filter.is_empty() {
        if let Some((_, items)) = data
            .groups
            .iter()
            .find(|(type_name, _)| type_name == name_filter)
        {
            lines.push(format!("{name_filter} ({}):", items.len()));
            for item in items {
                lines.push(format!("  {item}"));
            }
        } else {
            lines.push(format!("[INFO] Тип '{name_filter}' не найден в составе."));
            lines.push(format!(
                "Доступные типы: {}",
                data.groups
                    .iter()
                    .map(|(type_name, _)| type_name.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
    } else {
        for (type_name, items) in &data.groups {
            lines.push(format!("{type_name} ({}):", items.len()));
            for item in items {
                lines.push(format!("  {item}"));
            }
            lines.push(String::new());
        }
    }
}
pub fn build_subsystem_tree_entry(
    reader: &dyn ProjectDiscoveryRead,
    xml_path: &Path,
    prefix: &str,
    is_last: bool,
    is_root: bool,
    lines: &mut Vec<String>,
) -> Result<(), String> {
    let (data, _) = load_subsystem_info_data(reader, xml_path)?;
    let mut markers = Vec::new();
    if data.has_ci {
        markers.push("CI");
    }
    if data.use_one_command == "true" {
        markers.push("OneCmd");
    }
    if data.include_ci == "false" {
        markers.push("Скрыт");
    }
    let marker = if markers.is_empty() {
        String::new()
    } else {
        format!(" [{}]", markers.join(", "))
    };
    let child_str = if data.child_names.is_empty() {
        String::new()
    } else {
        format!(", {} дочерних", data.child_names.len())
    };
    let connector = if is_root {
        ""
    } else if is_last {
        "└── "
    } else {
        "├── "
    };
    lines.push(format!(
        "{prefix}{connector}{}{} ({} объектов{child_str})",
        data.name,
        marker,
        data.content_items.len()
    ));

    if !data.child_names.is_empty() {
        let child_prefix = if is_root {
            String::new()
        } else if is_last {
            format!("{prefix}    ")
        } else {
            format!("{prefix}│   ")
        };
        let subs_dir = subsystem_dir_for_xml(xml_path).join("Subsystems");
        for (idx, child_name) in data.child_names.iter().enumerate() {
            let child_xml = subs_dir.join(format!("{child_name}.xml"));
            let child_is_last = idx == data.child_names.len() - 1;
            if reader.path_is_file(&child_xml) {
                build_subsystem_tree_entry(
                    reader,
                    &child_xml,
                    &child_prefix,
                    child_is_last,
                    false,
                    lines,
                )?;
            } else {
                let conn = if child_is_last {
                    "└── "
                } else {
                    "├── "
                };
                lines.push(format!("{child_prefix}{conn}{child_name} [NOT FOUND]"));
            }
        }
    }
    Ok(())
}
pub fn paginate_subsystem_info(
    lines: &mut Vec<String>,
    args: &Map<String, Value>,
) -> Option<String> {
    let total_lines = lines.len();
    let offset = unica_operation_contract::input::int_arg(args, &["offset", "Offset"]).unwrap_or(0);
    let limit = unica_operation_contract::input::int_arg(args, &["limit", "Limit"]).unwrap_or(150);
    if offset > 0 {
        if offset as usize >= total_lines {
            return Some(format!(
                "[INFO] Offset {offset} exceeds total lines ({total_lines}). Nothing to show.\n"
            ));
        }
        *lines = lines[offset as usize..].to_vec();
    }
    if limit > 0 && lines.len() > limit as usize {
        let mut shown = lines[..limit as usize].to_vec();
        shown.push(String::new());
        shown.push(format!(
            "[ОБРЕЗАНО] Показано {limit} из {total_lines} строк. Используйте -Offset {} для продолжения.",
            offset + limit
        ));
        *lines = shown;
    }
    None
}
pub fn is_subsystem_content_ref(value: &str) -> bool {
    let Some((prefix, rest)) = value.split_once('.') else {
        return false;
    };
    !prefix.is_empty() && !rest.is_empty() && prefix.chars().all(|ch| ch.is_ascii_alphabetic())
}

#[cfg(test)]
#[path = "../tests/platform/subsystem_lists.rs"]
mod subsystem_list_patch_tests;

#[cfg(test)]
#[path = "../tests/platform/subsystem_codec.rs"]
mod tests;
