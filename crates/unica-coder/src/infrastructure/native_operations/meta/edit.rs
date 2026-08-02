#![allow(dead_code, unused_imports)]

use super::*;

#[derive(Default)]
pub(crate) struct MetaEditCounts {
    pub(crate) added: usize,
    pub(crate) modified: usize,
    pub(crate) removed: usize,
}

#[derive(Clone, Copy)]
enum MetaEditEol {
    Lf,
    CrLf,
    Cr,
}

#[derive(Clone, Copy)]
struct MetaEditSourceFormat {
    has_bom: bool,
    eol: MetaEditEol,
}

#[derive(Clone, Copy)]
pub(crate) enum MetaEditLineNumberLengthPolicy {
    Editable,
    FixedFive,
    NotApplicable,
    UnknownCompatibility,
}

struct MetaEditLineNumberLengthAuthorization {
    policy: MetaEditLineNumberLengthPolicy,
    provenance: Option<PlatformXmlOwnerProvenance>,
}

/// Typed answer of `unica.meta.edit` (ADR-0023). The projected diff stays a
/// string because a unified diff is a format, not a rendered report -- the same
/// choice `unica.code.patch` makes.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MetaEditData {
    pub(crate) object_kind: String,
    pub(crate) object_name: String,
    pub(crate) changed: bool,
    pub(crate) counts: MetaEditCountsData,
    pub(crate) diff: Option<String>,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MetaEditCountsData {
    pub(crate) added: usize,
    pub(crate) removed: usize,
    pub(crate) modified: usize,
}

pub(crate) struct MetaEditExecution {
    pub(crate) outcome: AdapterOutcome,
    pub(crate) data: Option<MetaEditData>,
}

pub(crate) fn edit_meta(args: &Map<String, Value>, context: &WorkspaceContext) -> AdapterOutcome {
    edit_meta_with_data(args, context).outcome
}

pub(crate) fn edit_meta_with_data(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> MetaEditExecution {
    edit_meta_with_mode(args, context, false)
}

/// Validates a metadata edit and reports its planned effects without writing files.
pub(crate) fn preview_meta_edit(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> AdapterOutcome {
    preview_meta_edit_with_data(args, context).outcome
}

pub(crate) fn preview_meta_edit_with_data(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> MetaEditExecution {
    edit_meta_with_mode(args, context, true)
}

/// Runs the shared metadata-edit workflow in either preview or apply mode.
fn edit_meta_with_mode(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
    dry_run: bool,
) -> MetaEditExecution {
    let edit_result = (|| -> Result<(MetaEditData, PathBuf, bool, Vec<String>), String> {
        let definition_file = path_arg(args, &["definitionFile", "DefinitionFile"]);
        let operation = string_arg(args, &["operation", "Operation"]);
        if definition_file.is_some() && operation.is_some() {
            return Err("Cannot use both -DefinitionFile and -Operation".to_string());
        }
        if definition_file.is_none() && operation.is_none() {
            return Err("Either -DefinitionFile or -Operation is required".to_string());
        }
        let object_path_raw = required_path(args, OBJECT_PATH, "ObjectPath")?;
        let object_path = resolve_meta_edit_object_path(&object_path_raw, &context.cwd)?;
        let value = string_arg(args, &["value", "Value"]).unwrap_or_default();

        let original_bytes = fs::read(&object_path)
            .map_err(|err| format!("failed to read {}: {err}", object_path.display()))?;
        let mut xml_text = String::from_utf8(original_bytes.clone())
            .map_err(|err| format!("failed to read {}: {err}", object_path.display()))?;
        let source_format = MetaEditSourceFormat {
            has_bom: original_bytes.starts_with(b"\xef\xbb\xbf"),
            eol: meta_edit_source_eol(&xml_text),
        };
        if xml_text.starts_with('\u{feff}') {
            xml_text = xml_text.trim_start_matches('\u{feff}').to_string();
        }
        let (object_type, object_name) = meta_edit_object_identity(&xml_text)?;
        validate_metadata_8_3_27_enum_contract(&xml_text, "meta.edit")?;

        let mut counts = MetaEditCounts::default();
        // The old report echoed the operation and value the caller had just
        // sent, plus the path that `artifacts` already names. Data keeps what
        // only the tool knows: whether anything changed, how much, and the diff.
        let mut projected_diff = None;
        let mut transaction = CompileTransaction::new();
        let line_number_length_provenance = if let Some(definition_file) = definition_file {
            let definition_path = absolutize(definition_file.clone(), &context.cwd);
            if !definition_path.exists() {
                return Err(format!(
                    "Definition file not found: {}",
                    definition_file.display()
                ));
            }
            let definition = FileBackedJson::read(&definition_path, |err| {
                format!("DefinitionFile JSON parse error: {err}")
            })?
            .bind_to(&mut transaction)?;
            let authorization = if meta_edit_definition_requests_line_number_length(&definition) {
                meta_edit_line_number_length_policy(
                    &object_type,
                    &object_path,
                    context,
                    &mut transaction,
                )?
            } else {
                MetaEditLineNumberLengthAuthorization {
                    policy: MetaEditLineNumberLengthPolicy::UnknownCompatibility,
                    provenance: None,
                }
            };
            meta_edit_apply_definition(
                &mut xml_text,
                &object_type,
                &object_name,
                &definition,
                authorization.policy,
                &mut counts,
            )?;
            authorization.provenance
        } else {
            let operation = operation.expect("checked above");
            let authorization = if meta_edit_inline_requests_line_number_length(operation, value) {
                meta_edit_line_number_length_policy(
                    &object_type,
                    &object_path,
                    context,
                    &mut transaction,
                )?
            } else {
                MetaEditLineNumberLengthAuthorization {
                    policy: MetaEditLineNumberLengthPolicy::UnknownCompatibility,
                    provenance: None,
                }
            };
            meta_edit_apply_inline_operation(
                &mut xml_text,
                &object_type,
                &object_name,
                operation,
                value,
                authorization.policy,
                &mut counts,
            )?;
            authorization.provenance
        };

        #[cfg(test)]
        run_meta_edit_after_line_number_length_policy_hook();

        Document::parse(xml_text.trim_start_matches('\u{feff}'))
            .map_err(|err| format!("XML parse error after meta-edit: {err}"))?;
        validate_metadata_8_3_27_boolean_contract(&xml_text, "meta.edit")?;
        let serialized_bytes = meta_edit_preserve_source_format(&xml_text, source_format);
        let changed = serialized_bytes != original_bytes;
        let mut warnings = Vec::new();
        if changed && !dry_run {
            transaction.replace_bytes(&object_path, &original_bytes, serialized_bytes)?;
            if let Some(provenance) = line_number_length_provenance {
                provenance.bind_to(&mut transaction)?;
            }
            guard_active_format_owner(&mut transaction, &object_path, context)?;
            let validation_path = object_path.clone();
            warnings = transaction
                .commit_with_post_validation(move || {
                    let published = read_utf8_sig(&validation_path)?;
                    validate_metadata_8_3_27_boolean_contract(&published, "meta.edit")?;
                    validate_metadata_8_3_27_enum_contract(&published, "meta.edit")
                })?
                .cleanup_warnings;
        } else if changed {
            let diff_path = object_path
                .strip_prefix(&context.cwd)
                .unwrap_or(&object_path)
                .display()
                .to_string();
            let before = String::from_utf8(original_bytes.clone())
                .map_err(|err| format!("failed to preview {}: {err}", object_path.display()))?;
            let after = String::from_utf8(serialized_bytes)
                .map_err(|err| format!("failed to preview {}: {err}", object_path.display()))?;
            projected_diff = meta_edit_projected_diff(
                &mut warnings,
                meta_edit_unified_diff(&diff_path, &before, &after),
            );
        } else {
            counts = MetaEditCounts::default();
        }
        let data = MetaEditData {
            object_kind: object_type.to_string(),
            object_name: object_name.to_string(),
            changed,
            counts: MetaEditCountsData {
                added: counts.added,
                removed: counts.removed,
                modified: counts.modified,
            },
            diff: projected_diff,
        };
        Ok((data, object_path, changed, warnings))
    })();

    match edit_result {
        Ok((data, object_path, changed, warnings)) => MetaEditExecution {
            outcome: AdapterOutcome {
                ok: true,
                summary: if dry_run {
                    if changed {
                        "dry run: unica.meta.edit planned native metadata edit".to_string()
                    } else {
                        "dry run: unica.meta.edit found no metadata changes".to_string()
                    }
                } else {
                    "unica.meta.edit completed with native metadata editor".to_string()
                },
                changes: if changed {
                    vec![if dry_run {
                        format!("would update {}", object_path.display())
                    } else {
                        format!("updated {}", object_path.display())
                    }]
                } else {
                    Vec::new()
                },
                warnings,
                errors: Vec::new(),
                artifacts: vec![object_path.display().to_string()],
                stdout: None,
                stderr: None,
                command: None,
            },
            data: Some(data),
        },
        Err(error) => MetaEditExecution {
            outcome: AdapterOutcome {
                ok: false,
                summary: if dry_run {
                    "dry run: unica.meta.edit failed in native metadata editor".to_string()
                } else {
                    "unica.meta.edit failed in native metadata editor".to_string()
                },
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

/// Returns a verified diff or records a non-fatal renderer diagnostic. A
/// renderer fault must not turn a valid edit into a failure.
pub(crate) fn meta_edit_projected_diff(
    warnings: &mut Vec<String>,
    rendered: Result<String, String>,
) -> Option<String> {
    match rendered {
        Ok(diff) => Some(diff),
        Err(error) => {
            warnings.push(format!(
                "projected diff could not be rendered safely: {error}"
            ));
            None
        }
    }
}

/// Renders and verifies an exact unified diff for the projected metadata bytes.
fn meta_edit_unified_diff(path: &str, before: &str, after: &str) -> Result<String, String> {
    let mut options = DiffOptions::new();
    options
        .set_original_filename(format!("a/{path}"))
        .set_modified_filename(format!("b/{path}"));
    let rendered = options.create_patch(before, after).to_string();
    let reparsed = Patch::from_str(&rendered)
        .map_err(|error| format!("generated meta-edit diff cannot be parsed: {error}"))?;
    let rebuilt = apply(before, &reparsed)
        .map_err(|error| format!("generated meta-edit diff cannot be applied: {error}"))?;
    if rebuilt.as_bytes() != after.as_bytes() {
        return Err(
            "generated meta-edit diff does not reproduce the exact projected XML".to_string(),
        );
    }
    Ok(rendered)
}

fn meta_edit_source_eol(text: &str) -> MetaEditEol {
    let bytes = text.as_bytes();
    if let Some(index) = bytes.iter().position(|byte| *byte == b'\n') {
        return if index > 0 && bytes[index - 1] == b'\r' {
            MetaEditEol::CrLf
        } else {
            MetaEditEol::Lf
        };
    }
    if bytes.contains(&b'\r') {
        MetaEditEol::Cr
    } else {
        MetaEditEol::Lf
    }
}

fn meta_edit_preserve_source_format(text: &str, format: MetaEditSourceFormat) -> Vec<u8> {
    let normalized = text
        .trim_start_matches('\u{feff}')
        .replace("\r\n", "\n")
        .replace('\r', "\n");
    let serialized = match format.eol {
        MetaEditEol::Lf => normalized,
        MetaEditEol::CrLf => normalized.replace('\n', "\r\n"),
        MetaEditEol::Cr => normalized.replace('\n', "\r"),
    };
    let mut bytes = Vec::with_capacity(serialized.len() + usize::from(format.has_bom) * 3);
    if format.has_bom {
        bytes.extend_from_slice(b"\xef\xbb\xbf");
    }
    bytes.extend_from_slice(serialized.as_bytes());
    bytes
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

fn meta_edit_line_number_length_policy(
    object_type: &str,
    object_path: &Path,
    context: &WorkspaceContext,
    transaction: &mut CompileTransaction,
) -> Result<MetaEditLineNumberLengthAuthorization, String> {
    if !meta_line_number_length_is_applicable(object_type) {
        return Ok(MetaEditLineNumberLengthAuthorization {
            policy: MetaEditLineNumberLengthPolicy::NotApplicable,
            provenance: None,
        });
    }

    let resolution = match resolve_platform_xml_owners_with_provenance(object_path, context) {
        Ok(resolution) => resolution,
        Err(_) => {
            return Ok(MetaEditLineNumberLengthAuthorization {
                policy: MetaEditLineNumberLengthPolicy::UnknownCompatibility,
                provenance: None,
            })
        }
    };
    let Some(owner) = resolution.owners.iter().find(|owner| {
        matches!(
            owner.kind,
            PlatformXmlOwnerKind::Configuration | PlatformXmlOwnerKind::Extension
        )
    }) else {
        return Ok(MetaEditLineNumberLengthAuthorization {
            policy: MetaEditLineNumberLengthPolicy::UnknownCompatibility,
            provenance: None,
        });
    };
    if owner.path != object_path {
        transaction.guard_or_verify_exact_preimage(&owner.path, &owner.raw)?;
    }
    let property_name = match owner.kind {
        PlatformXmlOwnerKind::Configuration => "CompatibilityMode",
        PlatformXmlOwnerKind::Extension => "ConfigurationExtensionCompatibilityMode",
        _ => unreachable!("configuration-like owners were filtered above"),
    };
    let owner_text = std::str::from_utf8(&owner.raw)
        .map_err(|error| format!("failed to read {}: {error}", owner.path.display()))?;
    let document = Document::parse(owner_text.trim_start_matches('\u{feff}'))
        .map_err(|error| format!("failed to parse {}: {error}", owner.path.display()))?;
    let Some(mode) = document
        .descendants()
        .find(|node| node.is_element() && node.tag_name().name() == property_name)
        .and_then(|node| node.text())
        .map(str::trim)
        .filter(|mode| !mode.is_empty())
    else {
        return Ok(MetaEditLineNumberLengthAuthorization {
            policy: MetaEditLineNumberLengthPolicy::UnknownCompatibility,
            provenance: None,
        });
    };

    Ok(MetaEditLineNumberLengthAuthorization {
        policy: meta_edit_line_number_length_policy_from_mode(mode),
        provenance: Some(resolution.provenance),
    })
}

pub(crate) fn meta_edit_line_number_length_policy_from_mode(
    mode: &str,
) -> MetaEditLineNumberLengthPolicy {
    meta_edit_line_number_length_policy_for_platform(mode, ACTIVE_FORMAT_PROFILE.platform_line)
}

pub(crate) fn meta_edit_line_number_length_policy_for_platform(
    mode: &str,
    platform_line: &str,
) -> MetaEditLineNumberLengthPolicy {
    if !cf_validate_enum_allowed("CompatibilityMode").contains(&mode) {
        return MetaEditLineNumberLengthPolicy::UnknownCompatibility;
    }
    let version = if mode == "DontUse" {
        meta_edit_parse_platform_line(platform_line)
    } else {
        mode.strip_prefix("Version")
            .and_then(meta_edit_parse_compatibility_version)
    };
    match version {
        Some(version) if version > (8, 3, 26) => MetaEditLineNumberLengthPolicy::Editable,
        Some(_) => MetaEditLineNumberLengthPolicy::FixedFive,
        None => MetaEditLineNumberLengthPolicy::UnknownCompatibility,
    }
}

pub(crate) fn meta_edit_parse_platform_line(value: &str) -> Option<(u32, u32, u32)> {
    let mut parts = value.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next()?.parse().ok()?;
    let patch = parts.next()?.parse().ok()?;
    if parts.next().is_some() {
        return None;
    }
    Some((major, minor, patch))
}

pub(crate) fn meta_edit_parse_compatibility_version(value: &str) -> Option<(u32, u32, u32)> {
    let mut parts = value.split('_');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next()?.parse().ok()?;
    let patch = parts.next().map(str::parse).transpose().ok()?.unwrap_or(0);
    if parts.next().is_some() {
        return None;
    }
    Some((major, minor, patch))
}

pub(crate) fn meta_edit_is_line_number_length_key(key: &str) -> bool {
    matches!(
        key.trim().to_ascii_lowercase().as_str(),
        "linenumberlength" | "line_number_length" | "line-number-length"
    )
}

pub(crate) fn meta_edit_changes_request_line_number_length(raw_changes: &str) -> bool {
    split_meta_edit_commas_outside_parens(raw_changes)
        .into_iter()
        .filter_map(|change| change.split_once('='))
        .any(|(key, _)| meta_edit_is_line_number_length_key(key))
}

pub(crate) fn meta_edit_inline_requests_line_number_length(operation: &str, value: &str) -> bool {
    if !operation.eq_ignore_ascii_case("modify-ts") {
        return false;
    }
    split_meta_edit_batch_items(value, operation)
        .unwrap_or_default()
        .into_iter()
        .filter_map(|item| item.split_once(':'))
        .any(|(_, changes)| meta_edit_changes_request_line_number_length(changes))
}

pub(crate) fn meta_edit_definition_requests_line_number_length(definition: &Value) -> bool {
    let Some(definition) = definition.as_object() else {
        return false;
    };
    definition.iter().any(|(operation, operation_value)| {
        if meta_edit_operation_key(operation).as_deref() != Some("modify") {
            return false;
        }
        let Some(modify) = operation_value.as_object() else {
            return false;
        };
        modify.iter().any(|(child_type, child_value)| {
            if meta_edit_child_type_key(child_type) != Some("tabularSections") {
                return false;
            }
            child_value
                .as_object()
                .into_iter()
                .flat_map(|sections| sections.values())
                .filter_map(Value::as_object)
                .flat_map(|changes| changes.keys())
                .any(|key| meta_edit_is_line_number_length_key(key))
        })
    })
}

pub(crate) fn split_meta_edit_batch_items<'a>(
    raw_value: &'a str,
    operation: &str,
) -> Result<Vec<&'a str>, String> {
    let items = raw_value
        .split(";;")
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .collect::<Vec<_>>();
    if items.is_empty() {
        return Err(format!("{operation} requires non-empty Value"));
    }
    Ok(items)
}

pub(crate) fn meta_edit_apply_inline_operation(
    xml_text: &mut String,
    object_type: &str,
    object_name: &str,
    operation: &str,
    value: &str,
    line_number_length_policy: MetaEditLineNumberLengthPolicy,
    counts: &mut MetaEditCounts,
) -> Result<(), String> {
    let (action, target) = operation
        .split_once('-')
        .ok_or_else(|| format!("Invalid meta-edit Operation: {operation}"))?;

    if let Some(property) = meta_edit_complex_property_from_inline_target(target) {
        meta_edit_apply_complex_property_action(
            xml_text,
            object_type,
            object_name,
            action,
            property,
            meta_edit_split_values(value),
            counts,
        )?;
        return Ok(());
    }

    if target == "ts-attribute" {
        match action {
            "add" => {
                for item in split_meta_edit_batch_items(value, operation)? {
                    meta_edit_add_tabular_section_attribute(xml_text, item)?;
                    counts.added += 1;
                }
            }
            "remove" => {
                counts.removed += meta_edit_remove_tabular_section_attribute(xml_text, value)?
            }
            "modify" => {
                counts.modified += meta_edit_modify_tabular_section_attribute(xml_text, value)?
            }
            _ => return Err(format!("Unsupported meta-edit Operation: {operation}")),
        }
        return Ok(());
    }

    if target == "property" {
        if action != "modify" {
            return Err(format!("Unsupported meta-edit Operation: {operation}"));
        }
        counts.modified += meta_edit_modify_object_properties_from_pairs(xml_text, value)?;
        return Ok(());
    }

    let Some(child_type) = meta_edit_child_type_from_inline_target(target) else {
        return Err(format!("Unsupported meta-edit Operation: {operation}"));
    };

    match action {
        "add" => {
            for item in split_meta_edit_batch_items(value, operation)? {
                let item_value = Value::String(item.to_string());
                meta_edit_add_child_value(
                    xml_text,
                    object_type,
                    object_name,
                    child_type,
                    &item_value,
                )?;
                counts.added += 1;
            }
        }
        "remove" => {
            for item in split_meta_edit_batch_items(value, operation)? {
                meta_edit_remove_child_value(
                    xml_text,
                    child_type,
                    &Value::String(item.to_string()),
                )?;
                counts.removed += 1;
            }
        }
        "modify" => {
            for item in split_meta_edit_batch_items(value, operation)? {
                let (name, raw_changes) = item
                    .split_once(':')
                    .ok_or_else(|| format!("{operation} requires Value like Name: key=value"))?;
                counts.modified += meta_edit_modify_top_child(
                    xml_text,
                    child_type,
                    name.trim(),
                    raw_changes.trim(),
                    line_number_length_policy,
                )?;
            }
        }
        _ => return Err(format!("Unsupported meta-edit Operation: {operation}")),
    }

    Ok(())
}

pub(crate) fn meta_edit_apply_definition(
    xml_text: &mut String,
    object_type: &str,
    object_name: &str,
    definition: &Value,
    line_number_length_policy: MetaEditLineNumberLengthPolicy,
    counts: &mut MetaEditCounts,
) -> Result<(), String> {
    let definition = definition
        .as_object()
        .ok_or_else(|| "DefinitionFile root must be a JSON object".to_string())?;

    if let Some(Value::Array(items)) = definition.get("_complex") {
        for item in items {
            let object = item
                .as_object()
                .ok_or_else(|| "_complex item must be an object".to_string())?;
            let action = object
                .get("action")
                .and_then(Value::as_str)
                .ok_or_else(|| "_complex item is missing action".to_string())?;
            let property = object
                .get("property")
                .and_then(Value::as_str)
                .ok_or_else(|| "_complex item is missing property".to_string())?;
            let values = meta_edit_values_from_json(object.get("values"));
            meta_edit_apply_complex_property_action(
                xml_text,
                object_type,
                object_name,
                action,
                property,
                values,
                counts,
            )?;
        }
    }

    for (raw_key, value) in definition {
        if raw_key == "_complex" {
            continue;
        }
        match meta_edit_operation_key(raw_key).as_deref() {
            Some("add") => {
                meta_edit_apply_definition_add(xml_text, object_type, object_name, value, counts)?
            }
            Some("remove") => meta_edit_apply_definition_remove(xml_text, value, counts)?,
            Some("modify") => meta_edit_apply_definition_modify(
                xml_text,
                object_type,
                object_name,
                value,
                line_number_length_policy,
                counts,
            )?,
            Some(other) => return Err(format!("Unsupported definition operation: {other}")),
            None => return Err(format!("Unknown definition operation: {raw_key}")),
        }
    }

    Ok(())
}

pub(crate) fn meta_edit_apply_definition_add(
    xml_text: &mut String,
    object_type: &str,
    object_name: &str,
    value: &Value,
    counts: &mut MetaEditCounts,
) -> Result<(), String> {
    let object = value
        .as_object()
        .ok_or_else(|| "definition add must be an object".to_string())?;
    for (raw_child_type, items) in object {
        let child_type = meta_edit_child_type_key(raw_child_type)
            .ok_or_else(|| format!("Unknown add child type: {raw_child_type}"))?;
        for item in meta_edit_definition_items(items) {
            meta_edit_add_child_value(xml_text, object_type, object_name, child_type, &item)?;
            counts.added += 1;
        }
    }
    Ok(())
}

pub(crate) fn meta_edit_apply_definition_remove(
    xml_text: &mut String,
    value: &Value,
    counts: &mut MetaEditCounts,
) -> Result<(), String> {
    let object = value
        .as_object()
        .ok_or_else(|| "definition remove must be an object".to_string())?;
    for (raw_child_type, items) in object {
        let child_type = meta_edit_child_type_key(raw_child_type)
            .ok_or_else(|| format!("Unknown remove child type: {raw_child_type}"))?;
        for item in meta_edit_definition_items(items) {
            meta_edit_remove_child_value(xml_text, child_type, &item)?;
            counts.removed += 1;
        }
    }
    Ok(())
}

pub(crate) fn meta_edit_apply_definition_modify(
    xml_text: &mut String,
    object_type: &str,
    object_name: &str,
    value: &Value,
    line_number_length_policy: MetaEditLineNumberLengthPolicy,
    counts: &mut MetaEditCounts,
) -> Result<(), String> {
    let object = value
        .as_object()
        .ok_or_else(|| "definition modify must be an object".to_string())?;
    for (raw_child_type, items) in object {
        let child_type = meta_edit_child_type_key(raw_child_type)
            .ok_or_else(|| format!("Unknown modify child type: {raw_child_type}"))?;
        if child_type == "properties" {
            meta_edit_modify_object_properties_from_map(
                xml_text,
                object_type,
                object_name,
                items,
                counts,
            )?;
        } else if child_type == "tabularSections" {
            meta_edit_modify_tabular_sections_from_definition(
                xml_text,
                items,
                line_number_length_policy,
                counts,
            )?;
        } else {
            let item_object = items
                .as_object()
                .ok_or_else(|| format!("modify {child_type} must be an object"))?;
            for (name, changes) in item_object {
                let raw_changes = meta_edit_changes_to_inline(changes)?;
                counts.modified += meta_edit_modify_top_child(
                    xml_text,
                    child_type,
                    name,
                    &raw_changes,
                    line_number_length_policy,
                )?;
            }
        }
    }
    Ok(())
}

pub(crate) fn meta_edit_definition_info_lines(definition: &Value) -> Vec<String> {
    let mut lines = Vec::new();
    let Some(object) = definition.as_object() else {
        return lines;
    };

    for (raw_key, value) in object {
        if raw_key == "_complex" {
            continue;
        }
        match meta_edit_operation_key(raw_key).as_deref() {
            Some("add") => lines.extend(meta_edit_definition_add_info_lines(value)),
            Some("remove") => lines.extend(meta_edit_definition_remove_info_lines(value)),
            Some("modify") => lines.extend(meta_edit_definition_modify_info_lines(value)),
            _ => {}
        }
    }

    lines
}

pub(crate) fn meta_edit_definition_add_info_lines(value: &Value) -> Vec<String> {
    let mut lines = Vec::new();
    let Some(object) = value.as_object() else {
        return lines;
    };

    for (raw_child_type, items) in object {
        let Some(child_type) = meta_edit_child_type_key(raw_child_type) else {
            continue;
        };
        for item in meta_edit_definition_items(items) {
            if let Some(name) = meta_edit_log_child_name(child_type, &item) {
                lines.push(format!(
                    "[INFO] Added {}: {name}",
                    meta_edit_added_child_log_label(child_type)
                ));
            }
        }
    }

    lines
}

pub(crate) fn meta_edit_definition_remove_info_lines(value: &Value) -> Vec<String> {
    let mut lines = Vec::new();
    let Some(object) = value.as_object() else {
        return lines;
    };

    for (raw_child_type, items) in object {
        let Some(child_type) = meta_edit_child_type_key(raw_child_type) else {
            continue;
        };
        let label = meta_edit_child_xml_tag(child_type)
            .map(|tag| tag.to_ascii_lowercase())
            .unwrap_or_else(|| child_type.to_string());
        for item in meta_edit_definition_items(items) {
            if let Some(name) = meta_edit_value_name(&item) {
                lines.push(format!("[INFO] Removed {label}: {name}"));
            }
        }
    }

    lines
}

pub(crate) fn meta_edit_definition_modify_info_lines(value: &Value) -> Vec<String> {
    let mut lines = Vec::new();
    let Some(object) = value.as_object() else {
        return lines;
    };

    for (raw_child_type, items) in object {
        let Some(child_type) = meta_edit_child_type_key(raw_child_type) else {
            continue;
        };
        if child_type == "properties" {
            if let Some(properties) = items.as_object() {
                for (key, value) in properties {
                    if meta_edit_complex_property_kind(key).is_none() {
                        lines.push(format!(
                            "[INFO] Modified property: {key} = {}",
                            json_value_to_python_string(value)
                        ));
                    }
                }
            }
        } else if child_type == "tabularSections" {
            lines.extend(meta_edit_tabular_section_definition_info_lines(items));
        } else if let Some(item_object) = items.as_object() {
            if let Some(tag) = meta_edit_child_xml_tag(child_type) {
                for (name, changes) in item_object {
                    lines.extend(meta_edit_modify_child_info_lines(tag, name, changes));
                }
            }
        }
    }

    lines
}

pub(crate) fn meta_edit_tabular_section_definition_info_lines(value: &Value) -> Vec<String> {
    let mut lines = Vec::new();
    let Some(object) = value.as_object() else {
        return lines;
    };

    for (section_name, changes) in object {
        let Some(changes) = changes.as_object() else {
            continue;
        };
        for (raw_key, change_value) in changes {
            match meta_edit_operation_key(raw_key).as_deref() {
                Some("add") => {
                    for item in meta_edit_definition_items(change_value) {
                        let attr = meta_compile_parse_attr(&item);
                        if !attr.name.is_empty() {
                            lines.push(format!(
                                "[INFO] Added attribute to TS '{section_name}': {}",
                                attr.name
                            ));
                        }
                    }
                }
                Some("remove") => {
                    for item in meta_edit_definition_items(change_value) {
                        if let Some(attr_name) = meta_edit_value_name(&item) {
                            lines.push(format!(
                                "[INFO] Removed attribute from TS '{section_name}': {attr_name}"
                            ));
                        }
                    }
                }
                Some("modify") => {
                    if let Some(attrs) = change_value.as_object() {
                        for (attr_name, attr_changes) in attrs {
                            lines.extend(meta_edit_modify_child_info_lines(
                                "Attribute",
                                attr_name,
                                attr_changes,
                            ));
                        }
                    }
                }
                _ => {
                    let mut scalar_change = Map::new();
                    scalar_change.insert(raw_key.to_string(), change_value.clone());
                    lines.extend(meta_edit_modify_child_info_lines(
                        "TabularSection",
                        section_name,
                        &Value::Object(scalar_change),
                    ));
                }
            }
        }
    }

    lines
}

pub(crate) fn meta_edit_modify_child_info_lines(
    xml_tag: &str,
    child_name: &str,
    changes: &Value,
) -> Vec<String> {
    let mut lines = Vec::new();
    for (key, value) in meta_edit_log_change_items(changes) {
        match key.as_str() {
            "name" => lines.push(format!("[INFO] Renamed {xml_tag}: {child_name} -> {value}")),
            "type" => lines.push(format!(
                "[INFO] Changed type of {xml_tag} '{child_name}': {value}"
            )),
            "synonym" => lines.push(format!(
                "[INFO] Changed synonym of {xml_tag} '{child_name}': {value}"
            )),
            _ => lines.push(format!(
                "[INFO] Modified {xml_tag} '{child_name}'.{key} = {value}"
            )),
        }
    }
    lines
}

pub(crate) fn meta_edit_log_change_items(value: &Value) -> Vec<(String, String)> {
    if let Some(text) = value.as_str() {
        return split_meta_edit_commas_outside_parens(text)
            .into_iter()
            .filter_map(|change| {
                let (key, value) = change.split_once('=')?;
                Some((key.trim().to_string(), value.trim().to_string()))
            })
            .collect();
    }
    let Some(object) = value.as_object() else {
        return Vec::new();
    };
    object
        .iter()
        .map(|(key, value)| (key.to_string(), json_value_to_python_string(value)))
        .collect()
}

pub(crate) fn meta_edit_added_child_log_label(child_type: &str) -> &'static str {
    match child_type {
        "attributes" => "attribute",
        "tabularSections" => "tabular section",
        "dimensions" => "dimension",
        "resources" => "resource",
        "enumValues" => "enum value",
        "columns" => "column",
        "forms" => "form",
        "templates" => "template",
        "commands" => "command",
        _ => "item",
    }
}

pub(crate) fn meta_edit_log_child_name(child_type: &str, value: &Value) -> Option<String> {
    let name = match child_type {
        "attributes" | "dimensions" | "resources" => meta_compile_parse_attr(value).name,
        "tabularSections" => meta_edit_tabular_section_from_value(value).ok()?.name,
        "enumValues" => meta_edit_enum_value_from_value(value).ok()?.name,
        "columns" => meta_edit_value_name(&meta_edit_column_value(value))?,
        "forms" | "templates" | "commands" => meta_edit_value_name(value)?,
        _ => return None,
    };
    if name.is_empty() {
        None
    } else {
        Some(name)
    }
}

pub(crate) fn meta_edit_modify_object_properties_from_pairs(
    xml_text: &mut String,
    value: &str,
) -> Result<usize, String> {
    let mut modified = 0usize;
    for pair in value
        .split(";;")
        .map(str::trim)
        .filter(|part| !part.is_empty())
    {
        let Some((key, raw_value)) = pair.split_once('=') else {
            return Err(format!("modify-property requires Key=Value, got: {pair}"));
        };
        meta_edit_set_scalar_property(xml_text, key.trim(), raw_value.trim())?;
        modified += 1;
    }
    if modified == 0 {
        return Err("modify-property requires non-empty Value".to_string());
    }
    Ok(modified)
}

pub(crate) fn meta_edit_modify_object_properties_from_map(
    xml_text: &mut String,
    object_type: &str,
    object_name: &str,
    value: &Value,
    counts: &mut MetaEditCounts,
) -> Result<(), String> {
    let object = value
        .as_object()
        .ok_or_else(|| "modify.properties must be an object".to_string())?;
    for (key, value) in object {
        if meta_edit_complex_property_kind(key).is_some() {
            meta_edit_apply_complex_property_action(
                xml_text,
                object_type,
                object_name,
                "set",
                key,
                meta_edit_values_from_json(Some(value)),
                counts,
            )?;
        } else {
            meta_edit_set_scalar_property(xml_text, key, &json_value_to_python_string(value))?;
            counts.modified += 1;
        }
    }
    Ok(())
}

pub(crate) fn meta_edit_set_scalar_property(
    xml_text: &mut String,
    key: &str,
    raw_value: &str,
) -> Result<(), String> {
    let key = key.trim();
    if key.is_empty() {
        return Err("modify-property requires non-empty key".to_string());
    }
    let doc = Document::parse(xml_text.trim_start_matches('\u{feff}'))
        .map_err(|err| format!("XML parse error: {err}"))?;
    let object = meta_edit_object_node(&doc)?;
    let object_type = object.tag_name().name();
    let normalized = normalize_meta_edit_scalar_property_value(object_type, key, raw_value);
    if key == "Name" {
        validate_meta_compile_name("meta.edit object", &normalized)?;
    }
    let properties = meta_info_child(object, "Properties")
        .ok_or_else(|| "Object has no Properties".to_string())?;
    let matching_properties = meta_info_children(properties, key).len();
    if matching_properties == 0 {
        return Err(format!(
            "direct scalar property <{key}> does not exist in object Properties"
        ));
    }
    if matching_properties > 1 {
        return Err(format!(
            "Properties contains {matching_properties} direct <{key}> elements; expected at most one"
        ));
    }
    validate_meta_8_3_27_property_value("meta.edit", key, &normalized)?;
    validate_meta_8_3_27_boolean_property_value("meta.edit", object_type, key, &normalized)?;
    let range = properties.range();
    drop(doc);

    let mut properties_text = xml_text[range.clone()].to_string();
    let child_indent = meta_edit_property_child_indent(&properties_text);
    let replacement = format!("{child_indent}<{key}>{}</{key}>", escape_xml(&normalized));
    meta_edit_replace_or_insert_property(&mut properties_text, key, &replacement, &child_indent)?;
    xml_text.replace_range(range, &properties_text);
    Ok(())
}

pub(crate) fn meta_edit_add_child_value(
    xml_text: &mut String,
    object_type: &str,
    object_name: &str,
    child_type: &str,
    value: &Value,
) -> Result<(), String> {
    let (value, position) = meta_edit_extract_insert_position(value)?;
    match child_type {
        "attributes" => {
            let attr = meta_compile_parse_attr(&value);
            if attr.name.is_empty() {
                return Err("add-attribute requires Value like Name: Type".to_string());
            }
            validate_meta_compile_attr_type(&attr, "meta.edit add attribute")?;
            meta_edit_ensure_top_child_name_free(xml_text, "Attribute", &attr.name)?;
            let context = meta_edit_attribute_context(object_type);
            let mut lines = Vec::new();
            let mut next_uuid = fresh_meta_compile_uuid;
            emit_meta_attribute(&mut lines, "\t\t\t", &attr, context, &mut next_uuid);
            meta_edit_insert_top_child_object_with_position(
                xml_text,
                "Attribute",
                &position,
                &lines,
            )
        }
        "tabularSections" => {
            let section = meta_edit_tabular_section_from_value(&value)?;
            meta_edit_ensure_top_child_name_free(xml_text, "TabularSection", &section.name)?;
            let mut lines = Vec::new();
            let mut next_uuid = fresh_meta_compile_uuid;
            emit_meta_tabular_section(
                &mut lines,
                "\t\t\t",
                &section,
                object_type,
                object_name,
                &mut next_uuid,
            );
            meta_edit_insert_top_child_object_with_position(
                xml_text,
                "TabularSection",
                &position,
                &lines,
            )
        }
        "dimensions" | "resources" => {
            let attr = meta_compile_parse_attr(&value);
            if attr.name.is_empty() {
                return Err(format!("add-{child_type} requires Value like Name: Type"));
            }
            validate_meta_compile_attr_type(&attr, "meta.edit add register field")?;
            let tag = if child_type == "dimensions" {
                "Dimension"
            } else {
                "Resource"
            };
            meta_edit_ensure_top_child_name_free(xml_text, tag, &attr.name)?;
            let mut lines = Vec::new();
            let mut next_uuid = fresh_meta_compile_uuid;
            emit_meta_register_field(
                &mut lines,
                "\t\t\t",
                tag,
                &attr,
                object_type,
                &mut next_uuid,
            );
            meta_edit_insert_top_child_object_with_position(xml_text, tag, &position, &lines)
        }
        "enumValues" => {
            let enum_value = meta_edit_enum_value_from_value(&value)?;
            validate_meta_compile_name("meta.edit enum value", &enum_value.name)?;
            meta_edit_ensure_top_child_name_free(xml_text, "EnumValue", &enum_value.name)?;
            let mut lines = Vec::new();
            let mut next_uuid = fresh_meta_compile_uuid;
            emit_meta_enum_value(&mut lines, "\t\t\t", &enum_value, &mut next_uuid);
            meta_edit_insert_top_child_object_with_position(
                xml_text,
                "EnumValue",
                &position,
                &lines,
            )
        }
        "columns" => {
            let column_value = meta_edit_column_value(&value);
            let column_name = meta_edit_value_name(&column_value)
                .ok_or_else(|| "add-column requires non-empty name".to_string())?;
            validate_meta_compile_name("meta.edit column", &column_name)?;
            meta_edit_ensure_top_child_name_free(xml_text, "Column", &column_name)?;
            let mut lines = Vec::new();
            let mut next_uuid = fresh_meta_compile_uuid;
            emit_meta_column(&mut lines, "\t\t\t", &column_value, &mut next_uuid);
            meta_edit_insert_top_child_object_with_position(xml_text, "Column", &position, &lines)
        }
        "forms" | "templates" | "commands" => {
            let tag = match child_type {
                "forms" => "Form",
                "templates" => "Template",
                _ => "Command",
            };
            let name = meta_edit_value_name(&value)
                .ok_or_else(|| format!("add-{child_type} requires non-empty name"))?;
            validate_meta_compile_name(&format!("meta.edit {child_type}"), &name)?;
            meta_edit_ensure_top_child_name_free(xml_text, tag, &name)?;
            let mut lines = Vec::new();
            let mut next_uuid = fresh_meta_compile_uuid;
            emit_meta_simple_child(&mut lines, "\t\t\t", tag, &name, &mut next_uuid);
            meta_edit_insert_top_child_object_with_position(xml_text, tag, &position, &lines)
        }
        other => Err(format!("Unsupported add child type: {other}")),
    }
}

pub(crate) fn meta_edit_remove_child_value(
    xml_text: &mut String,
    child_type: &str,
    value: &Value,
) -> Result<(), String> {
    let tag = meta_edit_child_xml_tag(child_type)
        .ok_or_else(|| format!("Unsupported remove child type: {child_type}"))?;
    let name = meta_edit_value_name(value)
        .ok_or_else(|| format!("remove {child_type} requires non-empty name"))?;
    meta_edit_remove_top_child_by_name(xml_text, tag, &name)
}

pub(crate) fn meta_edit_modify_top_child(
    xml_text: &mut String,
    child_type: &str,
    name: &str,
    raw_changes: &str,
    line_number_length_policy: MetaEditLineNumberLengthPolicy,
) -> Result<usize, String> {
    let tag = meta_edit_child_xml_tag(child_type)
        .ok_or_else(|| format!("Unsupported modify child type: {child_type}"))?;
    let target = match child_type {
        "attributes" => {
            let (object_type, _) = meta_edit_object_identity(xml_text)?;
            MetaEditModifyTarget::Attribute {
                fill_value_allowed: !matches!(object_type.as_str(), "DataProcessor" | "Report"),
            }
        }
        "dimensions" | "resources" => MetaEditModifyTarget::RegisterField,
        "enumValues" => MetaEditModifyTarget::EnumValue,
        "columns" => MetaEditModifyTarget::Column,
        "tabularSections" => MetaEditModifyTarget::TabularSection {
            line_number_length: line_number_length_policy,
        },
        _ => return Err(format!("Unsupported modify child type: {child_type}")),
    };
    meta_edit_modify_top_child_properties(xml_text, tag, name, raw_changes, target)
}

pub(crate) fn meta_edit_modify_tabular_sections_from_definition(
    xml_text: &mut String,
    value: &Value,
    line_number_length_policy: MetaEditLineNumberLengthPolicy,
    counts: &mut MetaEditCounts,
) -> Result<(), String> {
    let object = value
        .as_object()
        .ok_or_else(|| "modify.tabularSections must be an object".to_string())?;
    for (section_name, changes) in object {
        let changes = changes
            .as_object()
            .ok_or_else(|| "tabular section modify entry must be an object".to_string())?;
        let mut section_property_changes = Map::new();
        for (raw_key, change_value) in changes {
            match meta_edit_operation_key(raw_key).as_deref() {
                Some("add") => {
                    for item in meta_edit_definition_items(change_value) {
                        meta_edit_add_tabular_section_attribute_value(
                            xml_text,
                            section_name,
                            &item,
                        )?;
                        counts.added += 1;
                    }
                }
                Some("remove") => {
                    for item in meta_edit_definition_items(change_value) {
                        let attr_name = meta_edit_value_name(&item).ok_or_else(|| {
                            format!("remove attribute from TS '{section_name}' requires name")
                        })?;
                        meta_edit_remove_tabular_child_by_name(
                            xml_text,
                            section_name,
                            "Attribute",
                            &attr_name,
                        )?;
                        counts.removed += 1;
                    }
                }
                Some("modify") => {
                    let attrs = change_value.as_object().ok_or_else(|| {
                        format!("modify attributes in TS '{section_name}' must be an object")
                    })?;
                    for (attr_name, attr_changes) in attrs {
                        let raw_changes = meta_edit_changes_to_inline(attr_changes)?;
                        let modified = meta_edit_modify_tabular_attribute_properties(
                            xml_text,
                            section_name,
                            attr_name,
                            &raw_changes,
                        )?;
                        counts.modified += modified;
                    }
                }
                _ => {
                    section_property_changes.insert(raw_key.to_string(), change_value.clone());
                }
            }
        }
        if !section_property_changes.is_empty() {
            let raw_changes =
                meta_edit_changes_to_inline(&Value::Object(section_property_changes))?;
            meta_edit_modify_tabular_section_properties(
                xml_text,
                section_name,
                &raw_changes,
                line_number_length_policy,
            )?;
            counts.modified += 1;
        }
    }
    Ok(())
}

pub(crate) fn meta_edit_apply_complex_property_action(
    xml_text: &mut String,
    object_type: &str,
    object_name: &str,
    action: &str,
    property: &str,
    raw_values: Vec<String>,
    counts: &mut MetaEditCounts,
) -> Result<(), String> {
    let property = meta_edit_complex_property_kind(property)
        .ok_or_else(|| format!("Unsupported complex property: {property}"))?;
    if property == "RegisterRecords" && object_type != "Document" {
        return Err(format!(
            "RegisterRecords is supported for Document only, got: {object_type}"
        ));
    }
    let values = raw_values
        .into_iter()
        .map(|value| {
            meta_edit_normalize_complex_property_value(property, object_type, object_name, &value)
        })
        .collect::<Vec<_>>();
    if property == "RegisterRecords" {
        for value in &values {
            if !matches!(
                value.split('.').next().unwrap_or_default(),
                "AccumulationRegister"
                    | "InformationRegister"
                    | "AccountingRegister"
                    | "CalculationRegister"
            ) {
                return Err(format!(
                    "RegisterRecords value must be a register reference, got: {value}"
                ));
            }
        }
    }
    let existing = meta_edit_complex_property_values(xml_text, property)?;
    match action {
        "add" => {
            let mut next = existing;
            for value in values {
                if next.iter().any(|existing| existing == &value) {
                    return Err(format!("{property} item '{value}' already exists"));
                }
                next.push(value);
                counts.added += 1;
            }
            meta_edit_replace_complex_property(xml_text, property, &next)
        }
        "remove" => {
            let mut next = existing;
            for value in values {
                let Some(index) = next.iter().position(|existing| existing == &value) else {
                    return Err(format!("{property} item '{value}' not found"));
                };
                next.remove(index);
                counts.removed += 1;
            }
            meta_edit_replace_complex_property(xml_text, property, &next)
        }
        "set" => {
            meta_edit_replace_complex_property(xml_text, property, &values)?;
            counts.modified += 1;
            Ok(())
        }
        other => Err(format!("Unsupported complex property action: {other}")),
    }
}

pub(crate) fn meta_edit_complex_property_values(
    xml_text: &str,
    property: &str,
) -> Result<Vec<String>, String> {
    let doc = Document::parse(xml_text.trim_start_matches('\u{feff}'))
        .map_err(|err| format!("XML parse error: {err}"))?;
    let object = meta_edit_object_node(&doc)?;
    let Some(properties) = meta_info_child(object, "Properties") else {
        return Ok(Vec::new());
    };
    let Some(property_node) = meta_info_child(properties, property) else {
        return Ok(Vec::new());
    };
    Ok(property_node
        .children()
        .filter(|node| node.is_element())
        .filter_map(|node| node.text().map(str::trim).map(ToOwned::to_owned))
        .filter(|value| !value.is_empty())
        .collect())
}

pub(crate) fn meta_edit_replace_complex_property(
    xml_text: &mut String,
    property: &str,
    values: &[String],
) -> Result<(), String> {
    let replacement = meta_edit_complex_property_xml(xml_text, property, values)?;
    if let Some(range) = meta_edit_xml_element_range(xml_text, property)? {
        xml_text.replace_range(range, &replacement);
        return Ok(());
    }
    let Some(close_pos) = xml_text.find("</Properties>") else {
        return Err("No closing </Properties> found".to_string());
    };
    xml_text.insert_str(close_pos, &format!("{replacement}\n\t\t\t"));
    Ok(())
}

pub(crate) fn meta_edit_complex_property_xml(
    xml_text: &str,
    property: &str,
    values: &[String],
) -> Result<String, String> {
    let indent = if let Some(range) = meta_edit_xml_element_range(xml_text, property)? {
        meta_edit_line_indent(xml_text, range.start)
    } else {
        "\t\t\t".to_string()
    };
    if values.is_empty() {
        return Ok(format!("{indent}<{property}/>"));
    }
    let child_indent = format!("{indent}\t");
    let mut lines = vec![format!("{indent}<{property}>")];
    for value in values {
        if property == "InputByString" {
            lines.push(format!(
                "{child_indent}<xr:Field>{}</xr:Field>",
                escape_xml(value)
            ));
        } else {
            lines.push(format!(
                "{child_indent}<xr:Item xsi:type=\"xr:MDObjectRef\">{}</xr:Item>",
                escape_xml(value)
            ));
        }
    }
    lines.push(format!("{indent}</{property}>"));
    Ok(lines.join("\n"))
}

pub(crate) fn meta_edit_line_indent(text: &str, pos: usize) -> String {
    let line_start = text[..pos].rfind('\n').map_or(0, |index| index + 1);
    text[line_start..pos]
        .chars()
        .take_while(|ch| *ch == '\t' || *ch == ' ')
        .collect()
}

#[derive(Clone, Debug, Default)]
pub(crate) struct MetaEditInsertPosition {
    pub(crate) before: Option<String>,
    pub(crate) after: Option<String>,
}

impl MetaEditInsertPosition {
    pub(crate) fn is_empty(&self) -> bool {
        self.before.is_none() && self.after.is_none()
    }

    pub(crate) fn target(&self) -> Option<(&str, bool)> {
        if let Some(after) = self.after.as_deref() {
            Some((after, true))
        } else {
            self.before.as_deref().map(|before| (before, false))
        }
    }
}

pub(crate) fn meta_edit_extract_insert_position(
    value: &Value,
) -> Result<(Value, MetaEditInsertPosition), String> {
    if let Some(text) = value.as_str() {
        let (cleaned, position) = meta_edit_extract_insert_position_from_text(text)?;
        return Ok((Value::String(cleaned), position));
    }
    if let Some(object) = value.as_object() {
        let mut object = object.clone();
        let before = object
            .remove("before")
            .and_then(|value| value.as_str().map(str::trim).map(ToOwned::to_owned))
            .filter(|value| !value.is_empty());
        let after = object
            .remove("after")
            .and_then(|value| value.as_str().map(str::trim).map(ToOwned::to_owned))
            .filter(|value| !value.is_empty());
        if before.is_some() && after.is_some() {
            return Err("Use either before or after, not both".to_string());
        }
        return Ok((
            Value::Object(object),
            MetaEditInsertPosition { before, after },
        ));
    }
    Ok((value.clone(), MetaEditInsertPosition::default()))
}

pub(crate) fn meta_edit_extract_insert_position_from_text(
    text: &str,
) -> Result<(String, MetaEditInsertPosition), String> {
    let after_marker = ">> after ";
    let before_marker = "<< before ";
    let after_pos = text.rfind(after_marker);
    let before_pos = text.rfind(before_marker);
    let Some((marker_pos, marker, is_after)) = (match (after_pos, before_pos) {
        (Some(_), Some(_)) => {
            return Err("Use either >> after or << before, not both".to_string());
        }
        (Some(pos), None) => Some((pos, after_marker, true)),
        (None, Some(pos)) => Some((pos, before_marker, false)),
        (None, None) => None,
    }) else {
        return Ok((text.trim().to_string(), MetaEditInsertPosition::default()));
    };

    let cleaned = text[..marker_pos].trim().to_string();
    let target = text[marker_pos + marker.len()..].trim();
    if target.is_empty() {
        return Err("Position target must be non-empty".to_string());
    }
    let position = if is_after {
        MetaEditInsertPosition {
            before: None,
            after: Some(target.to_string()),
        }
    } else {
        MetaEditInsertPosition {
            before: Some(target.to_string()),
            after: None,
        }
    };
    Ok((cleaned, position))
}

pub(crate) fn meta_edit_normalize_complex_property_value(
    property: &str,
    object_type: &str,
    object_name: &str,
    value: &str,
) -> String {
    let value = value.trim();
    if property != "InputByString" {
        return normalize_meta_object_ref(value);
    }
    let first = value.split('.').next().unwrap_or_default();
    let is_prefixed = matches!(
        first,
        "Catalog"
            | "Document"
            | "InformationRegister"
            | "AccumulationRegister"
            | "AccountingRegister"
            | "CalculationRegister"
            | "ChartOfCharacteristicTypes"
            | "ChartOfCalculationTypes"
            | "ChartOfAccounts"
            | "ExchangePlan"
            | "BusinessProcess"
            | "Task"
            | "Enum"
            | "Report"
            | "DataProcessor"
    );
    if is_prefixed {
        value.to_string()
    } else {
        format!("{object_type}.{object_name}.{value}")
    }
}

pub(crate) fn meta_edit_complex_property_from_inline_target(target: &str) -> Option<&'static str> {
    match target {
        "owner" | "owners" => Some("Owners"),
        "registerRecord" | "registerRecords" => Some("RegisterRecords"),
        "basedOn" => Some("BasedOn"),
        "inputByString" => Some("InputByString"),
        _ => None,
    }
}

pub(crate) fn meta_edit_complex_property_kind(property: &str) -> Option<&'static str> {
    match property {
        "Owners" | "owners" => Some("Owners"),
        "RegisterRecords" | "registerRecords" => Some("RegisterRecords"),
        "BasedOn" | "basedOn" => Some("BasedOn"),
        "InputByString" | "inputByString" => Some("InputByString"),
        _ => None,
    }
}

pub(crate) fn meta_edit_operation_key(key: &str) -> Option<String> {
    match key.to_lowercase().as_str() {
        "add" | "добавить" => Some("add".to_string()),
        "remove" | "удалить" => Some("remove".to_string()),
        "modify" | "изменить" => Some("modify".to_string()),
        _ => None,
    }
}

pub(crate) fn meta_edit_child_type_from_inline_target(target: &str) -> Option<&'static str> {
    match target {
        "attribute" => Some("attributes"),
        "ts" => Some("tabularSections"),
        "dimension" => Some("dimensions"),
        "resource" => Some("resources"),
        "enumValue" => Some("enumValues"),
        "column" => Some("columns"),
        "form" => Some("forms"),
        "template" => Some("templates"),
        "command" => Some("commands"),
        _ => None,
    }
}

pub(crate) fn meta_edit_child_type_key(key: &str) -> Option<&'static str> {
    match key.to_lowercase().as_str() {
        "attributes" | "реквизиты" | "attrs" => Some("attributes"),
        "tabularsections" | "табличныечасти" | "тч" | "ts" => {
            Some("tabularSections")
        }
        "dimensions" | "измерения" | "dims" => Some("dimensions"),
        "resources" | "ресурсы" | "res" => Some("resources"),
        "enumvalues" | "значения" | "values" => Some("enumValues"),
        "columns" | "графы" | "колонки" => Some("columns"),
        "forms" | "формы" => Some("forms"),
        "templates" | "макеты" => Some("templates"),
        "commands" | "команды" => Some("commands"),
        "properties" | "свойства" => Some("properties"),
        _ => None,
    }
}

pub(crate) fn meta_edit_child_xml_tag(child_type: &str) -> Option<&'static str> {
    match child_type {
        "attributes" => Some("Attribute"),
        "tabularSections" => Some("TabularSection"),
        "dimensions" => Some("Dimension"),
        "resources" => Some("Resource"),
        "enumValues" => Some("EnumValue"),
        "columns" => Some("Column"),
        "forms" => Some("Form"),
        "templates" => Some("Template"),
        "commands" => Some("Command"),
        _ => None,
    }
}

pub(crate) fn meta_edit_split_values(value: &str) -> Vec<String> {
    value
        .split(";;")
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}

pub(crate) fn meta_edit_values_from_json(value: Option<&Value>) -> Vec<String> {
    match value {
        Some(Value::Array(items)) => items
            .iter()
            .map(json_value_to_python_string)
            .filter(|value| !value.trim().is_empty())
            .collect(),
        Some(Value::String(text)) => meta_edit_split_values(text),
        Some(value) => vec![json_value_to_python_string(value)],
        None => Vec::new(),
    }
}

pub(crate) fn meta_edit_definition_items(value: &Value) -> Vec<Value> {
    match value {
        Value::Array(items) => items.clone(),
        Value::String(_) => vec![value.clone()],
        Value::Object(object) if object.contains_key("name") => vec![value.clone()],
        Value::Object(object) => object
            .iter()
            .map(|(name, item)| {
                if let Some(mut item_object) = item.as_object().cloned() {
                    item_object
                        .entry("name".to_string())
                        .or_insert_with(|| Value::String(name.clone()));
                    Value::Object(item_object)
                } else if let Some(type_text) = item.as_str() {
                    Value::String(format!("{name}: {type_text}"))
                } else {
                    Value::String(name.clone())
                }
            })
            .collect(),
        _ => Vec::new(),
    }
}

pub(crate) fn meta_edit_value_name(value: &Value) -> Option<String> {
    value.as_str().map(ToOwned::to_owned).or_else(|| {
        value
            .as_object()
            .and_then(|object| object.get("name"))
            .and_then(Value::as_str)
            .map(ToOwned::to_owned)
    })
}

pub(crate) fn meta_edit_changes_to_inline(value: &Value) -> Result<String, String> {
    if let Some(text) = value.as_str() {
        return Ok(text.to_string());
    }
    let object = value
        .as_object()
        .ok_or_else(|| "modify changes must be an object or string".to_string())?;
    Ok(object
        .iter()
        .map(|(key, value)| format!("{key}={}", json_value_to_python_string(value)))
        .collect::<Vec<_>>()
        .join(", "))
}

pub(crate) fn meta_edit_tabular_section_from_value(
    value: &Value,
) -> Result<MetaCompileTabularSection, String> {
    if let Some(text) = value.as_str() {
        return meta_edit_parse_tabular_section(text);
    }
    let object = value
        .as_object()
        .ok_or_else(|| "tabular section must be a string or object".to_string())?;
    let name = object
        .get("name")
        .and_then(Value::as_str)
        .filter(|name| !name.is_empty())
        .ok_or_else(|| "tabular section is missing name".to_string())?;
    let columns_value = object
        .get("attrs")
        .or_else(|| object.get("attributes"))
        .or_else(|| object.get("реквизиты"));
    let section = MetaCompileTabularSection {
        name: name.to_string(),
        columns: meta_compile_attributes(columns_value),
    };
    validate_meta_compile_tabular_section_types(&section, "meta.edit tabular section")?;
    Ok(section)
}

pub(crate) fn meta_edit_enum_value_from_value(
    value: &Value,
) -> Result<MetaCompileEnumValue, String> {
    let mut values = meta_compile_enum_values(Some(&Value::Array(vec![value.clone()])))?;
    values
        .pop()
        .ok_or_else(|| "enum value is missing name".to_string())
}

pub(crate) fn meta_edit_column_value(value: &Value) -> Value {
    if let Some(text) = value.as_str() {
        if let Some((name, reference)) = text.split_once(':') {
            let mut object = Map::new();
            object.insert("name".to_string(), Value::String(name.trim().to_string()));
            object.insert(
                "references".to_string(),
                Value::Array(vec![Value::String(reference.trim().to_string())]),
            );
            return Value::Object(object);
        }
    }
    value.clone()
}

pub(crate) fn emit_meta_simple_child<F>(
    lines: &mut Vec<String>,
    indent: &str,
    tag: &str,
    name: &str,
    next_uuid: &mut F,
) where
    F: FnMut() -> String,
{
    lines.push(format!("{indent}<{tag} uuid=\"{}\">", next_uuid()));
    lines.push(format!("{indent}\t<Properties>"));
    lines.push(format!("{indent}\t\t<Name>{}</Name>", escape_xml(name)));
    emit_meta_mltext(
        lines,
        &format!("{indent}\t\t"),
        "Synonym",
        &split_meta_camel_case(name),
    );
    lines.push(format!("{indent}\t\t<Comment/>"));
    match tag {
        "Form" => {
            lines.push(format!("{indent}\t\t<FormType>Ordinary</FormType>"));
            lines.push(format!(
                "{indent}\t\t<IncludeHelpInContents>false</IncludeHelpInContents>"
            ));
            lines.push(format!("{indent}\t\t<UsePurposes/>"));
        }
        "Template" => {
            lines.push(format!(
                "{indent}\t\t<TemplateType>SpreadsheetDocument</TemplateType>"
            ));
        }
        "Command" => {
            lines.push(format!(
                "{indent}\t\t<Group>FormNavigationPanelGoTo</Group>"
            ));
            lines.push(format!("{indent}\t\t<Representation>Auto</Representation>"));
            lines.push(format!("{indent}\t\t<ToolTip/>"));
            lines.push(format!("{indent}\t\t<Picture/>"));
            lines.push(format!("{indent}\t\t<Shortcut/>"));
        }
        _ => {}
    }
    lines.push(format!("{indent}\t</Properties>"));
    lines.push(format!("{indent}</{tag}>"));
}

pub(crate) fn meta_edit_add_register_record(
    xml_text: &mut String,
    object_type: &str,
    raw_value: &str,
) -> Result<(), String> {
    if object_type != "Document" {
        return Err(format!(
            "add-registerRecord is supported for Document only, got: {object_type}"
        ));
    }
    let value = normalize_meta_object_ref(raw_value.trim());
    if value.is_empty() {
        return Err("add-registerRecord requires non-empty Value".to_string());
    }
    if !value.starts_with("AccumulationRegister.")
        && !value.starts_with("InformationRegister.")
        && !value.starts_with("AccountingRegister.")
        && !value.starts_with("CalculationRegister.")
    {
        return Err(format!(
            "add-registerRecord Value must be a register reference, got: {value}"
        ));
    }
    if meta_edit_register_record_exists(xml_text, &value)? {
        return Err(format!("Register record '{value}' already exists"));
    }
    let item = format!(
        "<xr:Item xsi:type=\"xr:MDObjectRef\">{}</xr:Item>",
        escape_xml(&value)
    );
    if xml_text.contains(&item) {
        return Err(format!("Register record '{value}' already exists"));
    }

    if xml_text.contains("<RegisterRecords/>") {
        *xml_text = xml_text.replacen(
            "<RegisterRecords/>",
            &format!("<RegisterRecords>\n\t\t\t{item}\n\t\t</RegisterRecords>"),
            1,
        );
        return Ok(());
    }
    if let Some(close_pos) = xml_text.find("</RegisterRecords>") {
        xml_text.insert_str(close_pos, &format!("\t\t\t{item}\n\t\t"));
        return Ok(());
    }
    if let Some(pos) = xml_text.find("<PostInPrivilegedMode>") {
        xml_text.insert_str(
            pos,
            &format!("<RegisterRecords>\n\t\t\t{item}\n\t\t</RegisterRecords>\n\t\t"),
        );
        return Ok(());
    }
    let Some(pos) = xml_text.find("</Properties>") else {
        return Err("No <Properties> section found in metadata object".to_string());
    };
    xml_text.insert_str(
        pos,
        &format!("\t\t<RegisterRecords>\n\t\t\t{item}\n\t\t</RegisterRecords>\n"),
    );
    Ok(())
}

pub(crate) fn meta_edit_register_record_exists(
    xml_text: &str,
    value: &str,
) -> Result<bool, String> {
    let doc = Document::parse(xml_text.trim_start_matches('\u{feff}'))
        .map_err(|err| format!("XML parse error: {err}"))?;
    let object = meta_edit_object_node(&doc)?;
    let Some(properties) = meta_info_child(object, "Properties") else {
        return Ok(false);
    };
    let Some(register_records) = meta_info_child(properties, "RegisterRecords") else {
        return Ok(false);
    };
    Ok(meta_info_children(register_records, "Item")
        .into_iter()
        .any(|item| item.text().unwrap_or("").trim() == value))
}

pub(crate) fn meta_edit_add_attribute(
    xml_text: &mut String,
    object_type: &str,
    raw_value: &str,
) -> Result<(), String> {
    let attr = meta_compile_parse_attr(&Value::String(raw_value.trim().to_string()));
    if attr.name.is_empty() {
        return Err("add-attribute requires Value like Name: Type".to_string());
    }
    validate_meta_compile_attr_type(&attr, "meta.edit add-attribute")?;
    meta_edit_ensure_top_child_name_free(xml_text, "Attribute", &attr.name)?;
    let context = meta_edit_attribute_context(object_type);
    let mut lines = Vec::new();
    let mut next_uuid = fresh_meta_compile_uuid;
    emit_meta_attribute(&mut lines, "\t\t\t", &attr, context, &mut next_uuid);
    meta_edit_insert_top_child_object(xml_text, &lines)
}

pub(crate) fn meta_edit_add_tabular_section(
    xml_text: &mut String,
    object_type: &str,
    object_name: &str,
    raw_value: &str,
) -> Result<(), String> {
    let section = meta_edit_parse_tabular_section(raw_value)?;
    meta_edit_ensure_top_child_name_free(xml_text, "TabularSection", &section.name)?;
    let mut lines = Vec::new();
    let mut next_uuid = fresh_meta_compile_uuid;
    emit_meta_tabular_section(
        &mut lines,
        "\t\t\t",
        &section,
        object_type,
        object_name,
        &mut next_uuid,
    );
    meta_edit_insert_top_child_object(xml_text, &lines)
}

pub(crate) fn meta_edit_parse_tabular_section(
    raw_value: &str,
) -> Result<MetaCompileTabularSection, String> {
    let value = raw_value.trim();
    if value.is_empty() {
        return Err("add-ts requires non-empty Value".to_string());
    }

    let Some((name, raw_columns)) = value.split_once(':') else {
        let section = MetaCompileTabularSection {
            name: value.to_string(),
            columns: Vec::new(),
        };
        validate_meta_compile_tabular_section_types(&section, "meta.edit add-ts")?;
        return Ok(section);
    };

    let name = name.trim();
    if name.is_empty() {
        return Err("add-ts requires non-empty tabular section name".to_string());
    }

    let columns = meta_edit_parse_tabular_section_columns(raw_columns)?;
    let section = MetaCompileTabularSection {
        name: name.to_string(),
        columns,
    };
    validate_meta_compile_tabular_section_types(&section, "meta.edit add-ts")?;
    Ok(section)
}

pub(crate) fn meta_edit_parse_tabular_section_columns(
    raw_columns: &str,
) -> Result<Vec<MetaCompileAttr>, String> {
    let mut column_defs = Vec::new();
    let mut current = String::new();

    for part in split_meta_edit_commas_outside_parens(raw_columns) {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        if !current.is_empty() && meta_edit_looks_like_attr_definition(part) {
            column_defs.push(current);
            current = part.to_string();
        } else if current.is_empty() {
            current = part.to_string();
        } else {
            current.push_str(", ");
            current.push_str(part);
        }
    }
    if !current.is_empty() {
        column_defs.push(current);
    }

    column_defs
        .into_iter()
        .map(|column| {
            let attr = meta_compile_parse_attr(&Value::String(column.clone()));
            if attr.name.is_empty() || attr.type_name.is_empty() {
                return Err(format!(
                    "add-ts column requires Value like Name: Type, got: {column}"
                ));
            }
            validate_meta_compile_attr_type(&attr, "meta.edit add-ts column")?;
            Ok(attr)
        })
        .collect()
}

pub(crate) fn split_meta_edit_commas_outside_parens(value: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut depth = 0usize;
    let mut start = 0usize;

    for (index, ch) in value.char_indices() {
        match ch {
            '(' => depth += 1,
            ')' => depth = depth.saturating_sub(1),
            ',' if depth == 0 => {
                parts.push(&value[start..index]);
                start = index + ch.len_utf8();
            }
            _ => {}
        }
    }
    parts.push(&value[start..]);
    parts
}

pub(crate) fn meta_edit_looks_like_attr_definition(value: &str) -> bool {
    value
        .split_once(':')
        .map(|(name, _)| !name.trim().is_empty())
        .unwrap_or(false)
}

pub(crate) fn meta_edit_add_tabular_section_attribute(
    xml_text: &mut String,
    raw_value: &str,
) -> Result<(), String> {
    let (section_name, attr_text) = raw_value.trim().split_once('.').ok_or_else(|| {
        "add-ts-attribute requires Value like Section.Attribute: Type".to_string()
    })?;
    let section_name = section_name.trim();
    meta_edit_add_tabular_section_attribute_value(
        xml_text,
        section_name,
        &Value::String(attr_text.trim().to_string()),
    )
}

pub(crate) fn meta_edit_add_tabular_section_attribute_value(
    xml_text: &mut String,
    section_name: &str,
    value: &Value,
) -> Result<(), String> {
    let (value, position) = meta_edit_extract_insert_position(value)?;
    let attr = meta_compile_parse_attr(&value);
    if section_name.is_empty() || attr.name.is_empty() {
        return Err("add-ts-attribute requires Value like Section.Attribute: Type".to_string());
    }
    validate_meta_compile_attr_type(&attr, "meta.edit add-ts-attribute")?;
    meta_edit_ensure_tabular_child_name_free(xml_text, section_name, "Attribute", &attr.name)?;
    let mut lines = Vec::new();
    let mut next_uuid = fresh_meta_compile_uuid;
    emit_meta_attribute(&mut lines, "\t\t\t\t\t", &attr, "tabular", &mut next_uuid);
    meta_edit_insert_tabular_child_object_with_position(
        xml_text,
        section_name,
        "Attribute",
        &position,
        &lines,
    )
}

pub(crate) fn meta_edit_remove_tabular_section_attribute(
    xml_text: &mut String,
    raw_value: &str,
) -> Result<usize, String> {
    let mut removed = 0usize;
    for item in raw_value
        .split(";;")
        .map(str::trim)
        .filter(|part| !part.is_empty())
    {
        let (section_name, attr_name) = item.split_once('.').ok_or_else(|| {
            "remove-ts-attribute requires Value like Section.Attribute".to_string()
        })?;
        let section_name = section_name.trim();
        let attr_name = attr_name.trim();
        if section_name.is_empty() || attr_name.is_empty() {
            return Err("remove-ts-attribute requires Value like Section.Attribute".to_string());
        }
        meta_edit_remove_tabular_child_by_name(xml_text, section_name, "Attribute", attr_name)?;
        removed += 1;
    }

    if removed == 0 {
        return Err("remove-ts-attribute requires non-empty Value".to_string());
    }
    Ok(removed)
}

pub(crate) fn meta_edit_modify_attribute(
    xml_text: &mut String,
    raw_value: &str,
) -> Result<usize, String> {
    let mut modified = 0usize;
    for item in raw_value
        .split(";;")
        .map(str::trim)
        .filter(|part| !part.is_empty())
    {
        let (attr_name, raw_changes) = item.split_once(':').ok_or_else(|| {
            "modify-attribute requires Value like Attribute: key=value".to_string()
        })?;
        let attr_name = attr_name.trim();
        if attr_name.is_empty() || raw_changes.trim().is_empty() {
            return Err("modify-attribute requires Value like Attribute: key=value".to_string());
        }
        modified += meta_edit_modify_top_attribute_properties(xml_text, attr_name, raw_changes)?;
    }
    if modified == 0 {
        return Err("modify-attribute requires non-empty Value".to_string());
    }
    Ok(modified)
}

pub(crate) fn meta_edit_modify_tabular_section(
    xml_text: &mut String,
    raw_value: &str,
    line_number_length_policy: MetaEditLineNumberLengthPolicy,
) -> Result<usize, String> {
    let mut modified = 0usize;
    for item in raw_value
        .split(";;")
        .map(str::trim)
        .filter(|part| !part.is_empty())
    {
        let (section_name, raw_changes) = item
            .split_once(':')
            .ok_or_else(|| "modify-ts requires Value like TabularSection: key=value".to_string())?;
        let section_name = section_name.trim();
        if section_name.is_empty() || raw_changes.trim().is_empty() {
            return Err("modify-ts requires Value like TabularSection: key=value".to_string());
        }
        modified += meta_edit_modify_tabular_section_properties(
            xml_text,
            section_name,
            raw_changes,
            line_number_length_policy,
        )?;
    }
    if modified == 0 {
        return Err("modify-ts requires non-empty Value".to_string());
    }
    Ok(modified)
}

pub(crate) fn meta_edit_modify_tabular_section_attribute(
    xml_text: &mut String,
    raw_value: &str,
) -> Result<usize, String> {
    let mut modified = 0usize;
    for item in raw_value
        .split(";;")
        .map(str::trim)
        .filter(|part| !part.is_empty())
    {
        let (target, raw_changes) = item.split_once(':').ok_or_else(|| {
            "modify-ts-attribute requires Value like Section.Attribute: key=value".to_string()
        })?;
        let (section_name, attr_name) = target.trim().split_once('.').ok_or_else(|| {
            "modify-ts-attribute requires Value like Section.Attribute: key=value".to_string()
        })?;
        let section_name = section_name.trim();
        let attr_name = attr_name.trim();
        if section_name.is_empty() || attr_name.is_empty() || raw_changes.trim().is_empty() {
            return Err(
                "modify-ts-attribute requires Value like Section.Attribute: key=value".to_string(),
            );
        }
        modified += meta_edit_modify_tabular_attribute_properties(
            xml_text,
            section_name,
            attr_name,
            raw_changes,
        )?;
    }
    if modified == 0 {
        return Err("modify-ts-attribute requires non-empty Value".to_string());
    }
    Ok(modified)
}

pub(crate) fn meta_edit_modify_top_attribute_properties(
    xml_text: &mut String,
    attr_name: &str,
    raw_changes: &str,
) -> Result<usize, String> {
    let doc = Document::parse(xml_text.trim_start_matches('\u{feff}'))
        .map_err(|err| format!("XML parse error: {err}"))?;
    let object = meta_edit_object_node(&doc)?;
    let target_kind = MetaEditModifyTarget::Attribute {
        fill_value_allowed: !matches!(object.tag_name().name(), "DataProcessor" | "Report"),
    };
    let child_objects = meta_info_child(object, "ChildObjects")
        .ok_or_else(|| format!("Attribute '{attr_name}' not found"))?;
    let target = meta_info_children(child_objects, "Attribute")
        .into_iter()
        .find(|child| meta_edit_child_object_name(*child).as_deref() == Some(attr_name))
        .ok_or_else(|| format!("Attribute '{attr_name}' not found"))?;
    if let Some(new_name) = meta_edit_requested_name(raw_changes, target_kind)? {
        meta_edit_ensure_sibling_name_free(
            child_objects,
            "Attribute",
            target.range(),
            &new_name,
            None,
        )?;
    }
    let props = meta_info_child(target, "Properties")
        .ok_or_else(|| format!("Attribute '{attr_name}' has no Properties"))?;
    let range = props.range();
    drop(doc);
    meta_edit_modify_properties_range(xml_text, range, raw_changes, target_kind)
}

pub(crate) fn meta_edit_modify_top_child_properties(
    xml_text: &mut String,
    tag: &str,
    child_name: &str,
    raw_changes: &str,
    target_kind: MetaEditModifyTarget,
) -> Result<usize, String> {
    let doc = Document::parse(xml_text.trim_start_matches('\u{feff}'))
        .map_err(|err| format!("XML parse error: {err}"))?;
    let object = meta_edit_object_node(&doc)?;
    let child_objects = meta_info_child(object, "ChildObjects")
        .ok_or_else(|| format!("{tag} '{child_name}' not found"))?;
    let target = meta_info_children(child_objects, tag)
        .into_iter()
        .find(|child| meta_edit_child_object_name(*child).as_deref() == Some(child_name))
        .ok_or_else(|| format!("{tag} '{child_name}' not found"))?;
    if let Some(new_name) = meta_edit_requested_name(raw_changes, target_kind)? {
        meta_edit_ensure_sibling_name_free(child_objects, tag, target.range(), &new_name, None)?;
    }
    let props = meta_info_child(target, "Properties")
        .ok_or_else(|| format!("{tag} '{child_name}' has no Properties"))?;
    let range = props.range();
    drop(doc);
    meta_edit_modify_properties_range(xml_text, range, raw_changes, target_kind)
}

pub(crate) fn meta_edit_modify_tabular_section_properties(
    xml_text: &mut String,
    section_name: &str,
    raw_changes: &str,
    line_number_length_policy: MetaEditLineNumberLengthPolicy,
) -> Result<usize, String> {
    let doc = Document::parse(xml_text.trim_start_matches('\u{feff}'))
        .map_err(|err| format!("XML parse error: {err}"))?;
    let object = meta_edit_object_node(&doc)?;
    let child_objects = meta_info_child(object, "ChildObjects")
        .ok_or_else(|| format!("TabularSection '{section_name}' not found"))?;
    let section = meta_info_children(child_objects, "TabularSection")
        .into_iter()
        .find(|section| meta_edit_child_object_name(*section).as_deref() == Some(section_name))
        .ok_or_else(|| format!("TabularSection '{section_name}' not found"))?;
    let target = MetaEditModifyTarget::TabularSection {
        line_number_length: line_number_length_policy,
    };
    if let Some(new_name) = meta_edit_requested_name(raw_changes, target)? {
        meta_edit_ensure_sibling_name_free(
            child_objects,
            "TabularSection",
            section.range(),
            &new_name,
            None,
        )?;
    }
    let props = meta_info_child(section, "Properties")
        .ok_or_else(|| format!("TabularSection '{section_name}' has no Properties"))?;
    let range = props.range();
    drop(doc);
    meta_edit_modify_properties_range(xml_text, range, raw_changes, target)
}

pub(crate) fn meta_edit_modify_tabular_attribute_properties(
    xml_text: &mut String,
    section_name: &str,
    attr_name: &str,
    raw_changes: &str,
) -> Result<usize, String> {
    let doc = Document::parse(xml_text.trim_start_matches('\u{feff}'))
        .map_err(|err| format!("XML parse error: {err}"))?;
    let object = meta_edit_object_node(&doc)?;
    let target_kind = MetaEditModifyTarget::Attribute {
        fill_value_allowed: matches!(object.tag_name().name(), "DataProcessor" | "Report"),
    };
    let section = meta_edit_find_tabular_section(object, section_name)
        .ok_or_else(|| format!("TabularSection '{section_name}' not found"))?;
    let child_objects = meta_info_child(section, "ChildObjects")
        .ok_or_else(|| format!("Attribute '{section_name}.{attr_name}' not found"))?;
    let target = meta_info_children(child_objects, "Attribute")
        .into_iter()
        .find(|child| meta_edit_child_object_name(*child).as_deref() == Some(attr_name))
        .ok_or_else(|| format!("Attribute '{section_name}.{attr_name}' not found"))?;
    if let Some(new_name) = meta_edit_requested_name(raw_changes, target_kind)? {
        meta_edit_ensure_sibling_name_free(
            child_objects,
            "Attribute",
            target.range(),
            &new_name,
            Some(section_name),
        )?;
    }
    let props = meta_info_child(target, "Properties")
        .ok_or_else(|| format!("Attribute '{section_name}.{attr_name}' has no Properties"))?;
    let range = props.range();
    drop(doc);
    meta_edit_modify_properties_range(xml_text, range, raw_changes, target_kind)
}

#[derive(Clone, Copy)]
pub(crate) enum MetaEditModifyTarget {
    Attribute {
        fill_value_allowed: bool,
    },
    RegisterField,
    EnumValue,
    Column,
    TabularSection {
        line_number_length: MetaEditLineNumberLengthPolicy,
    },
}

pub(crate) fn meta_edit_modify_properties_range(
    xml_text: &mut String,
    range: std::ops::Range<usize>,
    raw_changes: &str,
    target: MetaEditModifyTarget,
) -> Result<usize, String> {
    let mut properties = xml_text[range.clone()].to_string();
    let child_indent = meta_edit_property_child_indent(&properties);
    let mut modified = 0usize;

    for change in split_meta_edit_commas_outside_parens(raw_changes) {
        let change = change.trim();
        if change.is_empty() {
            continue;
        }
        let (raw_key, raw_value) = change
            .split_once('=')
            .ok_or_else(|| format!("modify attribute change requires key=value, got: {change}"))?;
        let key = raw_key.trim();
        let value = raw_value.trim();
        let canonical = meta_edit_canonical_attribute_property(key, target)?;
        match canonical.as_str() {
            "Name" => {
                validate_meta_compile_name("meta.edit rename", value)?;
                let replacement = format!("{child_indent}<Name>{}</Name>", escape_xml(value));
                meta_edit_replace_or_insert_property(
                    &mut properties,
                    "Name",
                    &replacement,
                    &child_indent,
                )?;
            }
            "Synonym" => {
                let mut lines = Vec::new();
                emit_meta_mltext(&mut lines, &child_indent, "Synonym", value);
                meta_edit_replace_or_insert_property(
                    &mut properties,
                    "Synonym",
                    &lines.join("\n"),
                    &child_indent,
                )?;
            }
            "Comment" => {
                let replacement = if value.is_empty() {
                    format!("{child_indent}<Comment/>")
                } else {
                    format!("{child_indent}<Comment>{}</Comment>", escape_xml(value))
                };
                meta_edit_replace_or_insert_property(
                    &mut properties,
                    "Comment",
                    &replacement,
                    &child_indent,
                )?;
            }
            "Type" => {
                validate_meta_type_union(std::iter::once(value)).map_err(|error| {
                    format!("invalid 8.3.27 type for meta.edit modify: {error}")
                })?;
                let mut lines = Vec::new();
                emit_meta_value_type(&mut lines, &child_indent, value);
                meta_edit_replace_or_insert_property(
                    &mut properties,
                    "Type",
                    &lines.join("\n"),
                    &child_indent,
                )?;
                if meta_edit_property_exists(&properties, "FillValue")? {
                    let mut fill_lines = Vec::new();
                    emit_meta_fill_value(&mut fill_lines, &child_indent, value);
                    meta_edit_replace_or_insert_property(
                        &mut properties,
                        "FillValue",
                        &fill_lines.join("\n"),
                        &child_indent,
                    )?;
                }
            }
            "FillValue" => {
                if !meta_edit_property_exists(&properties, "FillValue")? {
                    return Err(
                        "Property 'FillValue' is not available for this attribute".to_string()
                    );
                }
                let replacement = meta_edit_fill_value_xml(&child_indent, value);
                meta_edit_replace_or_insert_property(
                    &mut properties,
                    "FillValue",
                    &replacement,
                    &child_indent,
                )?;
            }
            "v8:AllowedSign" => {
                meta_edit_replace_or_insert_nested_v8_property(
                    &mut properties,
                    "NumberQualifiers",
                    "AllowedSign",
                    value,
                    &child_indent,
                )?;
            }
            "LineNumberLength" => {
                if !meta_edit_property_exists(&properties, "LineNumberLength")? {
                    return Err(
                        "Property 'LineNumberLength' is not available in this tabular section"
                            .to_string(),
                    );
                }
                let value = meta_edit_line_number_length_value(value)?;
                let replacement =
                    format!("{child_indent}<LineNumberLength>{value}</LineNumberLength>");
                meta_edit_replace_or_insert_property(
                    &mut properties,
                    "LineNumberLength",
                    &replacement,
                    &child_indent,
                )?;
            }
            _ => {
                let replacement = format!(
                    "{child_indent}<{canonical}>{}</{canonical}>",
                    escape_xml(value)
                );
                meta_edit_replace_or_insert_property(
                    &mut properties,
                    &canonical,
                    &replacement,
                    &child_indent,
                )?;
            }
        }
        modified += 1;
    }

    xml_text.replace_range(range, &properties);
    Ok(modified)
}

pub(crate) fn meta_edit_canonical_attribute_property(
    key: &str,
    target: MetaEditModifyTarget,
) -> Result<String, String> {
    let trimmed = key.trim();
    let normalized = trimmed.to_ascii_lowercase();
    if meta_edit_is_line_number_length_key(trimmed) {
        return match target {
            MetaEditModifyTarget::TabularSection {
                line_number_length: MetaEditLineNumberLengthPolicy::Editable,
            } => Ok("LineNumberLength".to_string()),
            MetaEditModifyTarget::TabularSection {
                line_number_length: MetaEditLineNumberLengthPolicy::FixedFive,
            } => Err(
                "LineNumberLength is fixed at 5 when CompatibilityMode is Version8_3_26 or earlier"
                    .to_string(),
            ),
            MetaEditModifyTarget::TabularSection {
                line_number_length: MetaEditLineNumberLengthPolicy::NotApplicable,
            } => Err(
                "LineNumberLength is not applicable to Report, DataProcessor, ExternalReport, or ExternalDataProcessor tabular sections".to_string(),
            ),
            MetaEditModifyTarget::TabularSection {
                line_number_length: MetaEditLineNumberLengthPolicy::UnknownCompatibility,
            } => Err(
                "LineNumberLength cannot be changed because CompatibilityMode cannot be determined"
                    .to_string(),
            ),
            _ => Err(format!("Unsupported modify property key '{trimmed}'")),
        };
    }
    let canonical = match normalized.as_str() {
        "name" | "имя" => Ok("Name".to_string()),
        "synonym" | "синоним" => Ok("Synonym".to_string()),
        "comment" | "комментарий" => Ok("Comment".to_string()),
        "fillchecking" | "fill_checking" | "fill-checking"
            if matches!(
                target,
                MetaEditModifyTarget::Attribute { .. }
                    | MetaEditModifyTarget::RegisterField
                    | MetaEditModifyTarget::TabularSection { .. }
            ) =>
        {
            Ok("FillChecking".to_string())
        }
        "use" | "использование"
            if matches!(
                target,
                MetaEditModifyTarget::Attribute { .. }
                    | MetaEditModifyTarget::RegisterField
                    | MetaEditModifyTarget::TabularSection { .. }
            ) =>
        {
            Ok("Use".to_string())
        }
        "type" | "тип"
            if matches!(
                target,
                MetaEditModifyTarget::Attribute { .. } | MetaEditModifyTarget::RegisterField
            ) =>
        {
            Ok("Type".to_string())
        }
        "fillvalue" | "fill_value" | "fill-value" | "значениезаполнения"
            if matches!(
                target,
                MetaEditModifyTarget::Attribute {
                    fill_value_allowed: true
                }
            ) =>
        {
            Ok("FillValue".to_string())
        }
        "indexing" | "индексирование"
            if matches!(
                target,
                MetaEditModifyTarget::Attribute { .. }
                    | MetaEditModifyTarget::RegisterField
                    | MetaEditModifyTarget::Column
            ) =>
        {
            Ok("Indexing".to_string())
        }
        "allowedsign" | "allowed_sign" | "allowed-sign" | "v8:allowedsign"
            if matches!(
                target,
                MetaEditModifyTarget::Attribute { .. } | MetaEditModifyTarget::RegisterField
            ) =>
        {
            Ok("v8:AllowedSign".to_string())
        }
        _ => Err(format!("Unsupported modify property key '{trimmed}'")),
    }?;
    Ok(canonical)
}

pub(crate) fn meta_edit_line_number_length_value(raw_value: &str) -> Result<String, String> {
    let parsed = raw_value.parse::<u8>().map_err(|_| {
        format!(
            "LineNumberLength must be an integer in 5..=9, got '{}'",
            raw_value
        )
    })?;
    if !(5..=9).contains(&parsed) {
        return Err(format!(
            "LineNumberLength must be an integer in 5..=9, got '{raw_value}'"
        ));
    }
    Ok(parsed.to_string())
}

pub(crate) fn meta_edit_fill_value_xml(indent: &str, raw_value: &str) -> String {
    let value = raw_value.trim();
    if value.is_empty() || value.eq_ignore_ascii_case("nil") {
        return format!("{indent}<FillValue xsi:nil=\"true\"/>");
    }

    let value_type = if meta_edit_is_design_time_ref(value) {
        "xr:DesignTimeRef"
    } else if value.eq_ignore_ascii_case("true") || value.eq_ignore_ascii_case("false") {
        "xs:boolean"
    } else if meta_edit_is_decimal_literal(value) {
        "xs:decimal"
    } else if meta_edit_is_date_time_literal(value) {
        "xs:dateTime"
    } else {
        "xs:string"
    };
    let normalized_value = if value_type == "xs:boolean" {
        value.to_ascii_lowercase()
    } else {
        value.to_string()
    };
    format!(
        "{indent}<FillValue xsi:type=\"{value_type}\">{}</FillValue>",
        escape_xml(&normalized_value)
    )
}

fn meta_edit_is_design_time_ref(value: &str) -> bool {
    let parts = value.split('.').collect::<Vec<_>>();
    matches!(parts.as_slice(), ["Enum", name, "EnumValue", item] if !name.is_empty() && !item.is_empty())
        || matches!(
            parts.as_slice(),
            [kind, name, "EmptyRef"]
                if !name.is_empty()
                    && matches!(
                        *kind,
                        "Catalog"
                            | "Document"
                            | "ExchangePlan"
                            | "ChartOfAccounts"
                            | "ChartOfCharacteristicTypes"
                            | "ChartOfCalculationTypes"
                            | "BusinessProcess"
                            | "Task"
                    )
        )
}

fn meta_edit_is_decimal_literal(value: &str) -> bool {
    let unsigned = value
        .strip_prefix('-')
        .or_else(|| value.strip_prefix('+'))
        .unwrap_or(value);
    let mut parts = unsigned.split('.');
    let integer = parts.next().unwrap_or_default();
    let fraction = parts.next();
    let has_digit = !integer.is_empty() || fraction.is_some_and(|part| !part.is_empty());
    has_digit
        && integer.chars().all(|ch| ch.is_ascii_digit())
        && fraction.is_none_or(|part| part.chars().all(|ch| ch.is_ascii_digit()))
        && parts.next().is_none()
}

fn meta_edit_is_date_time_literal(value: &str) -> bool {
    if value.len() != 19 || !value.is_ascii() {
        return false;
    }
    let bytes = value.as_bytes();
    if bytes[4] != b'-'
        || bytes[7] != b'-'
        || bytes[10] != b'T'
        || bytes[13] != b':'
        || bytes[16] != b':'
        || bytes
            .iter()
            .enumerate()
            .any(|(index, byte)| !matches!(index, 4 | 7 | 10 | 13 | 16) && !byte.is_ascii_digit())
    {
        return false;
    }

    let parse = |start: usize, end: usize| value[start..end].parse::<u32>().ok();
    let (Some(year), Some(month), Some(day), Some(hour), Some(minute), Some(second)) = (
        parse(0, 4),
        parse(5, 7),
        parse(8, 10),
        parse(11, 13),
        parse(14, 16),
        parse(17, 19),
    ) else {
        return false;
    };
    if year == 0 || !(1..=12).contains(&month) || hour > 23 || minute > 59 || second > 59 {
        return false;
    }
    let leap_year = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    let max_day = match month {
        2 if leap_year => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    };
    (1..=max_day).contains(&day)
}

pub(crate) fn meta_edit_requested_name(
    raw_changes: &str,
    target: MetaEditModifyTarget,
) -> Result<Option<String>, String> {
    for change in split_meta_edit_commas_outside_parens(raw_changes) {
        let change = change.trim();
        if change.is_empty() {
            continue;
        }
        let (raw_key, raw_value) = change
            .split_once('=')
            .ok_or_else(|| format!("modify attribute change requires key=value, got: {change}"))?;
        if meta_edit_canonical_attribute_property(raw_key, target)?.as_str() == "Name" {
            let name = raw_value.trim();
            if name.is_empty() {
                return Err("modify name requires non-empty value".to_string());
            }
            validate_meta_compile_name("meta.edit rename", name)?;
            return Ok(Some(name.to_string()));
        }
    }
    Ok(None)
}

pub(crate) fn meta_edit_ensure_sibling_name_free(
    child_objects: roxmltree::Node<'_, '_>,
    tag: &str,
    current_range: std::ops::Range<usize>,
    new_name: &str,
    parent_name: Option<&str>,
) -> Result<(), String> {
    for child in meta_info_children(child_objects, tag) {
        if child.range() == current_range {
            continue;
        }
        if meta_edit_child_object_name(child).as_deref() == Some(new_name) {
            return Err(match parent_name {
                Some(parent_name) => format!("{tag} '{parent_name}.{new_name}' already exists"),
                None => format!("{tag} '{new_name}' already exists"),
            });
        }
    }
    Ok(())
}

pub(crate) fn meta_edit_property_child_indent(properties: &str) -> String {
    for tag in ["Name", "Synonym", "Comment", "Type"] {
        let needle = format!("<{tag}");
        if let Some(pos) = properties.find(&needle) {
            let line_start = properties[..pos]
                .rfind('\n')
                .map(|idx| idx + 1)
                .unwrap_or(0);
            let indent = &properties[line_start..pos];
            if indent.chars().all(|ch| ch == '\t' || ch == ' ') {
                return indent.to_string();
            }
        }
    }
    "\t\t\t\t\t".to_string()
}

pub(crate) fn meta_edit_replace_or_insert_property(
    properties: &mut String,
    tag: &str,
    replacement: &str,
    child_indent: &str,
) -> Result<(), String> {
    if let Some(range) = meta_edit_xml_element_range(properties, tag)? {
        properties.replace_range(range, replacement.trim_start());
        return Ok(());
    }

    let Some(close_pos) = properties.rfind("</Properties>") else {
        return Err("No closing </Properties> found".to_string());
    };
    properties.insert_str(close_pos, &format!("{replacement}\n{child_indent}"));
    Ok(())
}

pub(crate) fn meta_edit_property_exists(properties: &str, tag: &str) -> Result<bool, String> {
    meta_edit_xml_element_range(properties, tag).map(|range| range.is_some())
}

pub(crate) fn meta_edit_xml_element_range(
    text: &str,
    tag: &str,
) -> Result<Option<std::ops::Range<usize>>, String> {
    let needle = format!("<{tag}");
    let close = format!("</{tag}>");
    let mut search_start = 0usize;

    while let Some(relative_start) = text[search_start..].find(&needle) {
        let start = search_start + relative_start;
        let after_tag = text[start + needle.len()..].chars().next();
        if after_tag.is_some_and(|ch| ch != '>' && ch != '/' && !ch.is_whitespace()) {
            search_start = start + needle.len();
            continue;
        }
        let Some(relative_open_end) = text[start..].find('>') else {
            return Err(format!("No closing > found for <{tag}>"));
        };
        let open_end = start + relative_open_end;
        let opening = &text[start..=open_end];
        if opening.trim_end().ends_with("/>") {
            return Ok(Some(start..open_end + 1));
        }
        let content_start = open_end + 1;
        let Some(relative_end) = text[content_start..].find(&close) else {
            return Err(format!("No closing </{tag}> found"));
        };
        let end = content_start + relative_end + close.len();
        return Ok(Some(start..end));
    }

    Ok(None)
}

pub(crate) fn meta_edit_replace_or_insert_nested_v8_property(
    properties: &mut String,
    parent_tag: &str,
    child_tag: &str,
    value: &str,
    child_indent: &str,
) -> Result<(), String> {
    let parent_open = format!("<v8:{parent_tag}>");
    let parent_close = format!("</v8:{parent_tag}>");
    let Some(parent_start) = properties.find(&parent_open) else {
        return Err(format!("No <v8:{parent_tag}> found"));
    };
    let parent_content_start = parent_start + parent_open.len();
    let Some(relative_parent_end) = properties[parent_content_start..].find(&parent_close) else {
        return Err(format!("No </v8:{parent_tag}> found"));
    };
    let parent_end = parent_content_start + relative_parent_end;
    let parent_range = parent_start..parent_end + parent_close.len();
    let mut parent = properties[parent_range.clone()].to_string();
    let nested_indent = format!("{child_indent}\t\t");
    let replacement = format!(
        "{nested_indent}<v8:{child_tag}>{}</v8:{child_tag}>",
        escape_xml(value)
    );

    let self_closing = format!("<v8:{child_tag}/>");
    if let Some(pos) = parent.find(&self_closing) {
        parent.replace_range(pos..pos + self_closing.len(), replacement.trim_start());
    } else {
        let open = format!("<v8:{child_tag}>");
        let close = format!("</v8:{child_tag}>");
        if let Some(start) = parent.find(&open) {
            let Some(relative_end) = parent[start + open.len()..].find(&close) else {
                return Err(format!("No </v8:{child_tag}> found"));
            };
            let end = start + open.len() + relative_end + close.len();
            parent.replace_range(start..end, replacement.trim_start());
        } else {
            let Some(close_pos) = parent.rfind(&parent_close) else {
                return Err(format!("No </v8:{parent_tag}> found"));
            };
            parent.insert_str(close_pos, &format!("{replacement}\n{child_indent}\t"));
        }
    }
    properties.replace_range(parent_range, &parent);
    Ok(())
}

pub(crate) fn meta_edit_remove_tabular_child_by_name(
    xml_text: &mut String,
    section_name: &str,
    tag: &str,
    name: &str,
) -> Result<(), String> {
    let doc = Document::parse(xml_text.trim_start_matches('\u{feff}'))
        .map_err(|err| format!("XML parse error: {err}"))?;
    let object = meta_edit_object_node(&doc)?;
    let section = meta_edit_find_tabular_section(object, section_name)
        .ok_or_else(|| format!("TabularSection '{section_name}' not found"))?;
    let child_objects = meta_info_child(section, "ChildObjects")
        .ok_or_else(|| format!("{tag} '{section_name}.{name}' not found"))?;
    let target = meta_info_children(child_objects, tag)
        .into_iter()
        .find(|child| meta_edit_child_object_name(*child).as_deref() == Some(name))
        .ok_or_else(|| format!("{tag} '{section_name}.{name}' not found"))?;
    let range = target.range();
    drop(doc);
    meta_edit_remove_xml_node_range(xml_text, range);
    Ok(())
}

pub(crate) fn meta_edit_remove_top_child_by_name(
    xml_text: &mut String,
    tag: &str,
    name: &str,
) -> Result<(), String> {
    let doc = Document::parse(xml_text.trim_start_matches('\u{feff}'))
        .map_err(|err| format!("XML parse error: {err}"))?;
    let object = meta_edit_object_node(&doc)?;
    let child_objects = meta_info_child(object, "ChildObjects")
        .ok_or_else(|| format!("{tag} '{name}' not found"))?;
    let target = meta_info_children(child_objects, tag)
        .into_iter()
        .find(|child| meta_edit_child_object_name(*child).as_deref() == Some(name))
        .ok_or_else(|| format!("{tag} '{name}' not found"))?;
    let range = target.range();
    drop(doc);
    meta_edit_remove_xml_node_range(xml_text, range);
    Ok(())
}

pub(crate) fn meta_edit_attribute_context(object_type: &str) -> &str {
    match object_type {
        "Catalog" => "catalog",
        "DataProcessor" | "Report" => "processor",
        "InformationRegister"
        | "AccumulationRegister"
        | "AccountingRegister"
        | "CalculationRegister" => "register-other",
        _ => "object",
    }
}

pub(crate) fn meta_edit_object_node<'a, 'input>(
    doc: &'a Document<'input>,
) -> Result<roxmltree::Node<'a, 'input>, String> {
    let root = doc.root_element();
    if root.tag_name().name() != "MetaDataObject" {
        return Err(format!(
            "Root element must be MetaDataObject, got: {}",
            root.tag_name().name()
        ));
    }
    root.children()
        .find(|node| node.is_element())
        .ok_or_else(|| "No object element found under MetaDataObject".to_string())
}

pub(crate) fn meta_edit_child_object_name(node: roxmltree::Node<'_, '_>) -> Option<String> {
    meta_info_child(node, "Properties").and_then(|props| meta_info_child_text(props, "Name"))
}

pub(crate) fn meta_edit_ensure_top_child_name_free(
    xml_text: &str,
    tag: &str,
    name: &str,
) -> Result<(), String> {
    let doc = Document::parse(xml_text.trim_start_matches('\u{feff}'))
        .map_err(|err| format!("XML parse error: {err}"))?;
    let object = meta_edit_object_node(&doc)?;
    if let Some(child_objects) = meta_info_child(object, "ChildObjects") {
        for child in meta_info_children(child_objects, tag) {
            if meta_edit_child_object_name(child).as_deref() == Some(name) {
                return Err(format!("{tag} '{name}' already exists"));
            }
        }
    }
    Ok(())
}

pub(crate) fn meta_edit_find_tabular_section<'a, 'input>(
    object: roxmltree::Node<'a, 'input>,
    section_name: &str,
) -> Option<roxmltree::Node<'a, 'input>> {
    let child_objects = meta_info_child(object, "ChildObjects")?;
    meta_info_children(child_objects, "TabularSection")
        .into_iter()
        .find(|section| meta_edit_child_object_name(*section).as_deref() == Some(section_name))
}

pub(crate) fn meta_edit_ensure_tabular_child_name_free(
    xml_text: &str,
    section_name: &str,
    tag: &str,
    name: &str,
) -> Result<(), String> {
    let doc = Document::parse(xml_text.trim_start_matches('\u{feff}'))
        .map_err(|err| format!("XML parse error: {err}"))?;
    let object = meta_edit_object_node(&doc)?;
    let section = meta_edit_find_tabular_section(object, section_name)
        .ok_or_else(|| format!("TabularSection '{section_name}' not found"))?;
    if let Some(child_objects) = meta_info_child(section, "ChildObjects") {
        for child in meta_info_children(child_objects, tag) {
            if meta_edit_child_object_name(child).as_deref() == Some(name) {
                return Err(format!("{tag} '{section_name}.{name}' already exists"));
            }
        }
    }
    Ok(())
}

pub(crate) fn meta_edit_insert_top_child_object(
    xml_text: &mut String,
    lines: &[String],
) -> Result<(), String> {
    let doc = Document::parse(xml_text.trim_start_matches('\u{feff}'))
        .map_err(|err| format!("XML parse error: {err}"))?;
    let object = meta_edit_object_node(&doc)?;
    if let Some(child_objects) = meta_info_child(object, "ChildObjects") {
        let range = child_objects.range();
        drop(doc);
        return meta_edit_insert_lines_into_child_objects(xml_text, range, "\t\t", lines);
    }
    let range = object.range();
    let tag = object.tag_name().name().to_string();
    drop(doc);
    meta_edit_insert_child_objects_into_node(xml_text, range, &tag, "\t\t", lines)
}

pub(crate) fn meta_edit_insert_top_child_object_with_position(
    xml_text: &mut String,
    tag: &str,
    position: &MetaEditInsertPosition,
    lines: &[String],
) -> Result<(), String> {
    if position.is_empty() {
        return meta_edit_insert_top_child_object(xml_text, lines);
    }
    let doc = Document::parse(xml_text.trim_start_matches('\u{feff}'))
        .map_err(|err| format!("XML parse error: {err}"))?;
    let object = meta_edit_object_node(&doc)?;
    let child_objects = meta_info_child(object, "ChildObjects")
        .ok_or_else(|| "ChildObjects not found for positional insert".to_string())?;
    let (target_name, after) = position
        .target()
        .ok_or_else(|| "Position target must be non-empty".to_string())?;
    let target = meta_info_children(child_objects, tag)
        .into_iter()
        .find(|child| meta_edit_child_object_name(*child).as_deref() == Some(target_name))
        .ok_or_else(|| format!("{tag} '{target_name}' not found for positional insert"))?;
    let range = target.range();
    drop(doc);
    meta_edit_insert_lines_near_node(xml_text, range, after, lines);
    Ok(())
}

pub(crate) fn meta_edit_insert_tabular_child_object(
    xml_text: &mut String,
    section_name: &str,
    lines: &[String],
) -> Result<(), String> {
    let doc = Document::parse(xml_text.trim_start_matches('\u{feff}'))
        .map_err(|err| format!("XML parse error: {err}"))?;
    let object = meta_edit_object_node(&doc)?;
    let section = meta_edit_find_tabular_section(object, section_name)
        .ok_or_else(|| format!("TabularSection '{section_name}' not found"))?;
    let range = section.range();
    drop(doc);
    meta_edit_insert_lines_into_node_child_objects(
        xml_text,
        range,
        "TabularSection",
        "\t\t\t\t",
        lines,
    )
}

pub(crate) fn meta_edit_insert_tabular_child_object_with_position(
    xml_text: &mut String,
    section_name: &str,
    tag: &str,
    position: &MetaEditInsertPosition,
    lines: &[String],
) -> Result<(), String> {
    if position.is_empty() {
        return meta_edit_insert_tabular_child_object(xml_text, section_name, lines);
    }
    let doc = Document::parse(xml_text.trim_start_matches('\u{feff}'))
        .map_err(|err| format!("XML parse error: {err}"))?;
    let object = meta_edit_object_node(&doc)?;
    let section = meta_edit_find_tabular_section(object, section_name)
        .ok_or_else(|| format!("TabularSection '{section_name}' not found"))?;
    let child_objects = meta_info_child(section, "ChildObjects")
        .ok_or_else(|| format!("TabularSection '{section_name}' has no ChildObjects"))?;
    let (target_name, after) = position
        .target()
        .ok_or_else(|| "Position target must be non-empty".to_string())?;
    let target = meta_info_children(child_objects, tag)
        .into_iter()
        .find(|child| meta_edit_child_object_name(*child).as_deref() == Some(target_name))
        .ok_or_else(|| {
            format!("{tag} '{section_name}.{target_name}' not found for positional insert")
        })?;
    let range = target.range();
    drop(doc);
    meta_edit_insert_lines_near_node(xml_text, range, after, lines);
    Ok(())
}

pub(crate) fn meta_edit_insert_lines_into_child_objects(
    xml_text: &mut String,
    range: std::ops::Range<usize>,
    close_indent: &str,
    lines: &[String],
) -> Result<(), String> {
    let content = lines.join("\n");
    let section_text = &xml_text[range.clone()];
    if section_text.trim_end().ends_with("/>") {
        xml_text.replace_range(
            range,
            &format!("<ChildObjects>\n{content}\n{close_indent}</ChildObjects>"),
        );
        return Ok(());
    }
    let Some(relative_pos) = section_text.rfind("</ChildObjects>") else {
        if section_text.trim_end().ends_with('>') {
            xml_text.insert_str(range.end, &format!("\n{content}\n{close_indent}"));
            return Ok(());
        }
        return Err("No closing </ChildObjects> found".to_string());
    };
    let close_pos = range.start + relative_pos;
    let line_start = xml_text[..close_pos]
        .rfind('\n')
        .map_or(close_pos, |index| index + 1);
    let insert_at_closing_indent = xml_text[line_start..close_pos]
        .chars()
        .all(|ch| ch == '\t' || ch == ' ');
    if insert_at_closing_indent {
        let insert_pos = meta_edit_mark_lxml_append_tail(xml_text, line_start);
        xml_text.insert_str(insert_pos, &format!("{content}\n"));
    } else {
        xml_text.insert_str(close_pos, &format!("{content}\n{close_indent}"));
    }
    Ok(())
}

pub(crate) fn meta_edit_mark_lxml_append_tail(xml_text: &mut String, insert_pos: usize) -> usize {
    if insert_pos == 0 || xml_text[..insert_pos].ends_with("&#13;\n") {
        return insert_pos;
    }
    if insert_pos >= 2 && &xml_text[insert_pos - 2..insert_pos] == "\r\n" {
        xml_text.replace_range(insert_pos - 2..insert_pos, "&#13;\n");
        return insert_pos + 4;
    }
    if insert_pos >= 1 && &xml_text[insert_pos - 1..insert_pos] == "\n" {
        xml_text.replace_range(insert_pos - 1..insert_pos, "&#13;\n");
        return insert_pos + 5;
    }
    insert_pos
}

pub(crate) fn meta_edit_insert_lines_near_node(
    xml_text: &mut String,
    range: std::ops::Range<usize>,
    after: bool,
    lines: &[String],
) {
    let content = lines.join("\n");
    if after {
        if let Some(relative_newline) = xml_text[range.end..].find('\n') {
            let insert_pos = range.end + relative_newline + 1;
            xml_text.insert_str(insert_pos, &format!("{content}&#13;\n"));
        } else {
            xml_text.insert_str(range.end, &format!("\n{content}"));
        }
        return;
    }

    let line_start = xml_text[..range.start]
        .rfind('\n')
        .map_or(0, |index| index + 1);
    let insert_pos = if xml_text[line_start..range.start]
        .chars()
        .all(|ch| ch == '\t' || ch == ' ')
    {
        line_start
    } else {
        range.start
    };
    xml_text.insert_str(insert_pos, &format!("{content}\n"));
}

pub(crate) fn meta_edit_insert_child_objects_into_node(
    xml_text: &mut String,
    range: std::ops::Range<usize>,
    tag: &str,
    close_indent: &str,
    lines: &[String],
) -> Result<(), String> {
    let content = lines.join("\n");
    let node_text = &xml_text[range.clone()];
    if let Some(relative_pos) = node_text.rfind("/>") {
        let pos = range.start + relative_pos;
        xml_text.replace_range(
            pos..pos + 2,
            &format!(">\n{close_indent}<ChildObjects>\n{content}\n{close_indent}</ChildObjects>\n\t</{tag}>"),
        );
        return Ok(());
    }
    let close = format!("</{tag}>");
    let Some(relative_pos) = node_text.rfind(&close) else {
        return Err(format!("No closing </{tag}> found"));
    };
    xml_text.insert_str(
        range.start + relative_pos,
        &format!("{close_indent}<ChildObjects>\n{content}\n{close_indent}</ChildObjects>\n"),
    );
    Ok(())
}

pub(crate) fn meta_edit_insert_lines_into_node_child_objects(
    xml_text: &mut String,
    range: std::ops::Range<usize>,
    tag: &str,
    close_indent: &str,
    lines: &[String],
) -> Result<(), String> {
    let content = lines.join("\n");
    let node_text = &xml_text[range.clone()];
    if let Some(relative_pos) = node_text.find("<ChildObjects/>") {
        let pos = range.start + relative_pos;
        xml_text.replace_range(
            pos..pos + "<ChildObjects/>".len(),
            &format!("<ChildObjects>\n{content}\n{close_indent}</ChildObjects>"),
        );
        return Ok(());
    }
    if let Some(relative_pos) = node_text.find("<ChildObjects>") {
        let pos = range.start + relative_pos + "<ChildObjects>".len();
        xml_text.insert_str(pos, &format!("\n{content}"));
        return Ok(());
    }
    meta_edit_insert_child_objects_into_node(xml_text, range, tag, close_indent, lines)
}

pub(crate) fn meta_edit_remove_xml_node_range(
    xml_text: &mut String,
    range: std::ops::Range<usize>,
) {
    let mut start = range.start;
    let mut end = range.end;

    if let Some(line_start) = xml_text[..start].rfind('\n') {
        let prefix = &xml_text[line_start + 1..start];
        if prefix.trim().is_empty() {
            start = line_start + 1;
        }
    }

    if end < xml_text.len() {
        if let Some(line_end) = xml_text[end..].find('\n') {
            let suffix_end = end + line_end;
            let suffix = &xml_text[end..suffix_end];
            if suffix.trim().is_empty() {
                end = suffix_end + 1;
            }
        }
    }

    xml_text.replace_range(start..end, "");
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

fn normalize_meta_edit_scalar_property_value(object_type: &str, key: &str, value: &str) -> String {
    let normalized = normalize_meta_edit_property_value(key, value);
    if meta_8_3_27_boolean_properties(object_type).contains(&key) {
        if normalized.eq_ignore_ascii_case("true") {
            return "true".to_string();
        }
        if normalized.eq_ignore_ascii_case("false") {
            return "false".to_string();
        }
    }
    normalized
}
