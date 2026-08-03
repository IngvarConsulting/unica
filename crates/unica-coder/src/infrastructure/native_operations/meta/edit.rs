#![allow(dead_code, unused_imports)]

use crate::application::metadata::{MetaEditRequest, MetaFailure};
use crate::application::operation_descriptors::OBJECT_PATH;
use crate::application::ports::{
    MetadataResourceImage, MetadataResourceRole, MetadataValidationSubject,
    PreparedMetadataMutation,
};
use crate::application::AdapterOutcome;
use crate::domain::cancellation::CancellationToken;
use crate::domain::format_profile::ACTIVE_FORMAT_PROFILE;
use crate::domain::metadata::{
    DateFractions, MetaCollection, MetaDiagnostic, MetaDiagnosticCode, MetaEditOperation,
    MetaElementDefinition, MetaElementUpdate, MetaFillValue, MetaPosition, MetaPropertyKey,
    MetaPropertyValue, MetaPublicationAction, MetaPublicationPlanEntry, MetaPublicationResource,
    MetaRelation, MetadataType, MetadataTypeVariant, NumberSign, RelationEditMode,
    StringLengthMode,
};
use crate::domain::source_target::{
    MetadataAddress, SourceTarget, TargetKind, PLATFORM_XML_8_3_27_FORMAT_2_20,
};
use crate::domain::workspace::WorkspaceContext;
use crate::infrastructure::platform_xml_owner::{
    resolve_platform_xml_owners_with_provenance, PlatformXmlOwnerKind, PlatformXmlOwnerProvenance,
};
use crate::infrastructure::platform_xml_source_targets::{
    platform_xml_resource_evidence, resolve_platform_xml_target, ClosedPlatformXmlTarget,
    TargetKindPolicy,
};
use crate::infrastructure::support_guard::{
    bind_resolved_support_guard_evidence, evaluate_resolved_support_guard,
    ResolvedSupportGuardCheck,
};
use diffy::{apply, DiffOptions, Patch};
use roxmltree::Document;
use serde_json::{Map, Value};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use super::super::cf::cf_validate_enum_allowed;
use super::super::common::{
    absolutize, escape_xml, guard_active_format_owner, is_1c_identifier,
    json_value_to_python_string, read_utf8_sig, required_path,
};
use super::super::compile_transaction::CompileTransaction;
use super::format_contract::{
    meta_8_3_27_boolean_properties, validate_meta_8_3_27_boolean_property_value,
    validate_metadata_8_3_27_boolean_contract, validate_metadata_8_3_27_enum_contract,
};
use super::legacy_dsl::{
    meta_edit_apply_definition, meta_edit_apply_inline_operation,
    meta_edit_definition_requests_line_number_length, meta_edit_inline_requests_line_number_length,
    parse_meta_edit_dsl_input, read_meta_edit_definition, validate_meta_8_3_27_property_value,
    validate_meta_compile_attr_type, validate_meta_compile_name,
    validate_meta_compile_tabular_section_types, MetaEditDslInput,
};
use super::publisher::{fresh_meta_compile_uuid, PreparedMetaEdit};
use super::template_catalog::{
    emit_meta_attribute, emit_meta_column, emit_meta_enum_value, emit_meta_register_field,
    emit_meta_tabular_section, meta_attribute_context, meta_compile_attributes,
    meta_compile_enum_values, meta_compile_parse_attr, meta_line_number_length_is_applicable,
    metadata_generated_types_8_3_27, metadata_standard_attribute_names, normalize_meta_enum_value,
    normalize_meta_object_ref, split_meta_camel_case, MetaCompileAttr, MetaCompileEnumValue,
    MetaCompileTabularSection,
};
use super::xml_model::{
    emit_meta_fill_value, emit_meta_mltext, emit_meta_typed_fill_value, emit_meta_typed_value_type,
    emit_meta_value_type, meta_info_child, meta_info_child_text, meta_info_children,
    validate_meta_type_union,
};

#[cfg(test)]
use super::run_meta_edit_after_line_number_length_policy_hook;

#[derive(Debug, Default)]
pub(super) struct MetaEditCounts {
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
pub(super) enum MetaEditLineNumberLengthPolicy {
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

pub(super) fn edit_meta(args: &Map<String, Value>, context: &WorkspaceContext) -> AdapterOutcome {
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
        let input = parse_meta_edit_dsl_input(args)?;
        let object_path_raw = required_path(args, OBJECT_PATH, "ObjectPath")?;
        let object_path = resolve_meta_edit_object_path(&object_path_raw, &context.cwd)?;

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
        let line_number_length_provenance = match input {
            MetaEditDslInput::File(definition_file) => {
                let definition =
                    read_meta_edit_definition(&definition_file, context, &mut transaction)?;
                let authorization = if meta_edit_definition_requests_line_number_length(&definition)
                {
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
            }
            MetaEditDslInput::Inline { operation, value } => {
                let authorization =
                    if meta_edit_inline_requests_line_number_length(&operation, &value) {
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
                    &operation,
                    &value,
                    authorization.policy,
                    &mut counts,
                )?;
                authorization.provenance
            }
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
pub(super) fn meta_edit_projected_diff(
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

pub(super) fn meta_edit_object_identity(xml_text: &str) -> Result<(String, String), String> {
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

pub(super) fn meta_edit_line_number_length_policy_from_mode(
    mode: &str,
) -> MetaEditLineNumberLengthPolicy {
    meta_edit_line_number_length_policy_for_platform(mode, ACTIVE_FORMAT_PROFILE.platform_line)
}

pub(super) fn meta_edit_line_number_length_policy_for_platform(
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

pub(super) fn meta_edit_parse_platform_line(value: &str) -> Option<(u32, u32, u32)> {
    let mut parts = value.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next()?.parse().ok()?;
    let patch = parts.next()?.parse().ok()?;
    if parts.next().is_some() {
        return None;
    }
    Some((major, minor, patch))
}

pub(super) fn meta_edit_parse_compatibility_version(value: &str) -> Option<(u32, u32, u32)> {
    let mut parts = value.split('_');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next()?.parse().ok()?;
    let patch = parts.next().map(str::parse).transpose().ok()?.unwrap_or(0);
    if parts.next().is_some() {
        return None;
    }
    Some((major, minor, patch))
}

pub(super) fn meta_edit_is_line_number_length_key(key: &str) -> bool {
    matches!(
        key.trim().to_ascii_lowercase().as_str(),
        "linenumberlength" | "line_number_length" | "line-number-length"
    )
}

pub(super) fn meta_edit_definition_info_lines(definition: &Value) -> Vec<String> {
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

pub(super) fn meta_edit_definition_add_info_lines(value: &Value) -> Vec<String> {
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

pub(super) fn meta_edit_definition_remove_info_lines(value: &Value) -> Vec<String> {
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

pub(super) fn meta_edit_definition_modify_info_lines(value: &Value) -> Vec<String> {
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

pub(super) fn meta_edit_tabular_section_definition_info_lines(value: &Value) -> Vec<String> {
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

pub(super) fn meta_edit_modify_child_info_lines(
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

pub(super) fn meta_edit_log_change_items(value: &Value) -> Vec<(String, String)> {
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

pub(super) fn meta_edit_added_child_log_label(child_type: &str) -> &'static str {
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

pub(super) fn meta_edit_log_child_name(child_type: &str, value: &Value) -> Option<String> {
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

pub(super) fn meta_edit_modify_object_properties_from_pairs(
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

pub(super) fn meta_edit_modify_object_properties_from_map(
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

pub(super) fn meta_edit_set_scalar_property(
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

pub(super) fn meta_edit_add_child_value(
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

pub(super) fn meta_edit_remove_child_value(
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

pub(super) fn meta_edit_modify_top_child(
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

pub(super) fn meta_edit_modify_tabular_sections_from_definition(
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

pub(super) fn meta_edit_apply_complex_property_action(
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

pub(super) fn meta_edit_complex_property_values(
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

pub(super) fn meta_edit_replace_complex_property(
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

pub(super) fn meta_edit_complex_property_xml(
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

pub(super) fn meta_edit_line_indent(text: &str, pos: usize) -> String {
    let line_start = text[..pos].rfind('\n').map_or(0, |index| index + 1);
    text[line_start..pos]
        .chars()
        .take_while(|ch| *ch == '\t' || *ch == ' ')
        .collect()
}

#[derive(Clone, Debug, Default)]
pub(super) struct MetaEditInsertPosition {
    pub(crate) before: Option<String>,
    pub(crate) after: Option<String>,
}

impl MetaEditInsertPosition {
    pub(super) fn is_empty(&self) -> bool {
        self.before.is_none() && self.after.is_none()
    }

    pub(super) fn target(&self) -> Option<(&str, bool)> {
        if let Some(after) = self.after.as_deref() {
            Some((after, true))
        } else {
            self.before.as_deref().map(|before| (before, false))
        }
    }
}

pub(super) fn meta_edit_extract_insert_position(
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

pub(super) fn meta_edit_extract_insert_position_from_text(
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

pub(super) fn meta_edit_normalize_complex_property_value(
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

pub(super) fn meta_edit_complex_property_from_inline_target(target: &str) -> Option<&'static str> {
    match target {
        "owner" | "owners" => Some("Owners"),
        "registerRecord" | "registerRecords" => Some("RegisterRecords"),
        "basedOn" => Some("BasedOn"),
        "inputByString" => Some("InputByString"),
        _ => None,
    }
}

pub(super) fn meta_edit_complex_property_kind(property: &str) -> Option<&'static str> {
    match property {
        "Owners" | "owners" => Some("Owners"),
        "RegisterRecords" | "registerRecords" => Some("RegisterRecords"),
        "BasedOn" | "basedOn" => Some("BasedOn"),
        "InputByString" | "inputByString" => Some("InputByString"),
        _ => None,
    }
}

pub(super) fn meta_edit_operation_key(key: &str) -> Option<String> {
    match key.to_lowercase().as_str() {
        "add" | "добавить" => Some("add".to_string()),
        "remove" | "удалить" => Some("remove".to_string()),
        "modify" | "изменить" => Some("modify".to_string()),
        _ => None,
    }
}

pub(super) fn meta_edit_child_type_from_inline_target(target: &str) -> Option<&'static str> {
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

pub(super) fn meta_edit_child_type_key(key: &str) -> Option<&'static str> {
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

pub(super) fn meta_edit_child_xml_tag(child_type: &str) -> Option<&'static str> {
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

pub(super) fn meta_edit_split_values(value: &str) -> Vec<String> {
    value
        .split(";;")
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}

pub(super) fn meta_edit_values_from_json(value: Option<&Value>) -> Vec<String> {
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

pub(super) fn meta_edit_definition_items(value: &Value) -> Vec<Value> {
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

pub(super) fn meta_edit_value_name(value: &Value) -> Option<String> {
    value.as_str().map(ToOwned::to_owned).or_else(|| {
        value
            .as_object()
            .and_then(|object| object.get("name"))
            .and_then(Value::as_str)
            .map(ToOwned::to_owned)
    })
}

pub(super) fn meta_edit_changes_to_inline(value: &Value) -> Result<String, String> {
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

pub(super) fn meta_edit_tabular_section_from_value(
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

pub(super) fn meta_edit_enum_value_from_value(
    value: &Value,
) -> Result<MetaCompileEnumValue, String> {
    let mut values = meta_compile_enum_values(Some(&Value::Array(vec![value.clone()])))?;
    values
        .pop()
        .ok_or_else(|| "enum value is missing name".to_string())
}

pub(super) fn meta_edit_column_value(value: &Value) -> Value {
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

pub(super) fn emit_meta_simple_child<F>(
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

pub(super) fn meta_edit_add_register_record(
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

pub(super) fn meta_edit_register_record_exists(
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

pub(super) fn meta_edit_add_attribute(
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

pub(super) fn meta_edit_add_tabular_section(
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

pub(super) fn meta_edit_parse_tabular_section(
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

pub(super) fn meta_edit_parse_tabular_section_columns(
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

pub(super) fn split_meta_edit_commas_outside_parens(value: &str) -> Vec<&str> {
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

pub(super) fn meta_edit_looks_like_attr_definition(value: &str) -> bool {
    value
        .split_once(':')
        .map(|(name, _)| !name.trim().is_empty())
        .unwrap_or(false)
}

pub(super) fn meta_edit_add_tabular_section_attribute(
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

pub(super) fn meta_edit_add_tabular_section_attribute_value(
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

pub(super) fn meta_edit_remove_tabular_section_attribute(
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

pub(super) fn meta_edit_modify_attribute(
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

pub(super) fn meta_edit_modify_tabular_section(
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

pub(super) fn meta_edit_modify_tabular_section_attribute(
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

pub(super) fn meta_edit_modify_top_attribute_properties(
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

pub(super) fn meta_edit_modify_top_child_properties(
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

pub(super) fn meta_edit_modify_tabular_section_properties(
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

pub(super) fn meta_edit_modify_tabular_attribute_properties(
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
pub(super) enum MetaEditModifyTarget {
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

pub(super) fn meta_edit_modify_properties_range(
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

pub(super) fn meta_edit_canonical_attribute_property(
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

pub(super) fn meta_edit_line_number_length_value(raw_value: &str) -> Result<String, String> {
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

pub(super) fn meta_edit_fill_value_xml(indent: &str, raw_value: &str) -> String {
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

pub(super) fn meta_edit_requested_name(
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

pub(super) fn meta_edit_ensure_sibling_name_free(
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

pub(super) fn meta_edit_property_child_indent(properties: &str) -> String {
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

pub(super) fn meta_edit_replace_or_insert_property(
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

pub(super) fn meta_edit_property_exists(properties: &str, tag: &str) -> Result<bool, String> {
    meta_edit_xml_element_range(properties, tag).map(|range| range.is_some())
}

pub(super) fn meta_edit_xml_element_range(
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

pub(super) fn meta_edit_replace_or_insert_nested_v8_property(
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

pub(super) fn meta_edit_remove_tabular_child_by_name(
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

pub(super) fn meta_edit_remove_top_child_by_name(
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

pub(super) fn meta_edit_attribute_context(object_type: &str) -> &str {
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

pub(super) fn meta_edit_object_node<'a, 'input>(
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

pub(super) fn meta_edit_child_object_name(node: roxmltree::Node<'_, '_>) -> Option<String> {
    meta_info_child(node, "Properties").and_then(|props| meta_info_child_text(props, "Name"))
}

pub(super) fn meta_edit_ensure_top_child_name_free(
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

pub(super) fn meta_edit_find_tabular_section<'a, 'input>(
    object: roxmltree::Node<'a, 'input>,
    section_name: &str,
) -> Option<roxmltree::Node<'a, 'input>> {
    let child_objects = meta_info_child(object, "ChildObjects")?;
    meta_info_children(child_objects, "TabularSection")
        .into_iter()
        .find(|section| meta_edit_child_object_name(*section).as_deref() == Some(section_name))
}

pub(super) fn meta_edit_ensure_tabular_child_name_free(
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

pub(super) fn meta_edit_insert_top_child_object(
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

pub(super) fn meta_edit_insert_top_child_object_with_position(
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

pub(super) fn meta_edit_insert_tabular_child_object(
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

pub(super) fn meta_edit_insert_tabular_child_object_with_position(
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

pub(super) fn meta_edit_insert_lines_into_child_objects(
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

pub(super) fn meta_edit_mark_lxml_append_tail(xml_text: &mut String, insert_pos: usize) -> usize {
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

pub(super) fn meta_edit_insert_lines_near_node(
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

pub(super) fn meta_edit_insert_child_objects_into_node(
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

pub(super) fn meta_edit_insert_lines_into_node_child_objects(
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

pub(super) fn meta_edit_remove_xml_node_range(
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

pub(super) fn normalize_meta_edit_property_value(key: &str, value: &str) -> String {
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

pub(crate) struct ResolvedMetadataObject {
    pub(super) handle: ClosedPlatformXmlTarget,
    pub(super) descriptor_path: PathBuf,
    pub(super) descriptor_preimage: Vec<u8>,
    pub(super) source_root: PathBuf,
    pub(super) owner_path: PathBuf,
    pub(super) owner_preimage: Vec<u8>,
}

#[derive(Debug)]
pub(super) struct TypedChildFileMutation {
    pub(super) path: PathBuf,
    pub(super) pre_image: Option<Vec<u8>>,
    pub(super) post_image: Option<Vec<u8>>,
}

#[derive(Debug, Default)]
pub(super) struct TypedChildResourcePlan {
    pub(super) file_mutations: Vec<TypedChildFileMutation>,
    pub(super) publication_plan: Vec<MetaPublicationPlanEntry>,
    pub(super) validation_resources: Vec<MetadataResourceImage>,
    pub(super) relation_dependencies: Vec<TypedRelationDependency>,
}

#[derive(Debug)]
pub(super) struct TypedRelationDependency {
    pub(super) handle: ClosedPlatformXmlTarget,
    pub(super) path: PathBuf,
    pub(super) bytes: Vec<u8>,
    pub(super) target: MetadataAddress,
}

fn plan_typed_child_resources(
    descriptor_path: &Path,
    owner: &MetadataAddress,
    object_kind: &str,
    object_name: &str,
    operations: &[MetaEditOperation],
    post_image: &str,
) -> Result<TypedChildResourcePlan, MetaFailure> {
    let mut plan = TypedChildResourcePlan::default();
    let object_dir = descriptor_path.with_extension("");
    let mut request_created_names = std::collections::HashSet::new();
    let mut request_renamed_destinations = std::collections::HashSet::new();
    for operation in operations {
        match operation {
            MetaEditOperation::Add {
                collection,
                elements,
                ..
            } => {
                for element in elements {
                    request_created_names.insert((collection.as_str(), element.name.clone()));
                }
            }
            MetaEditOperation::Update {
                collection,
                elements,
                ..
            } => {
                for element in elements {
                    if let Some(new_name) = &element.new_name {
                        request_renamed_destinations
                            .insert((collection.as_str(), new_name.clone()));
                    }
                    if request_created_names.contains(&(collection.as_str(), element.name.clone()))
                    {
                        if let Some(new_name) = &element.new_name {
                            request_created_names.insert((collection.as_str(), new_name.clone()));
                        }
                    }
                }
            }
            _ => {}
        }
    }
    let mut planned_existing_names = std::collections::HashSet::new();
    for (operation_index, operation) in operations.iter().enumerate() {
        if let MetaEditOperation::Update {
            collection,
            elements,
            ..
        } = operation
        {
            let Some((directory, resource)) = typed_physical_collection(*collection) else {
                continue;
            };
            for element in elements {
                let old_name = &element.name;
                if request_created_names.contains(&(collection.as_str(), old_name.clone()))
                    || request_renamed_destinations
                        .contains(&(collection.as_str(), old_name.clone()))
                {
                    continue;
                }
                if !planned_existing_names.insert((collection.as_str(), old_name.clone())) {
                    continue;
                }
                let immediate_name = element.new_name.as_deref().unwrap_or(old_name);
                let final_name = typed_new_child_final_name(
                    *collection,
                    immediate_name,
                    &operations[operation_index + 1..],
                );
                let collection_dir = object_dir.join(directory);
                let old_descriptor_path = collection_dir.join(format!("{old_name}.xml"));
                let old_descriptor = read_typed_child_file(&old_descriptor_path, owner)?;
                let Some(new_name) = final_name else {
                    plan.file_mutations.push(TypedChildFileMutation {
                        path: old_descriptor_path,
                        pre_image: Some(old_descriptor),
                        post_image: None,
                    });
                    let old_payload_dir = collection_dir.join(old_name);
                    if old_payload_dir.exists() {
                        plan.file_mutations.push(TypedChildFileMutation {
                            path: old_payload_dir,
                            pre_image: Some(Vec::new()),
                            post_image: None,
                        });
                    }
                    plan.publication_plan.push(MetaPublicationPlanEntry {
                        action: MetaPublicationAction::Remove,
                        resource,
                        metadata_path: Some(typed_child_logical_address(
                            owner,
                            object_kind,
                            object_name,
                            *collection,
                            old_name,
                        )?),
                    });
                    continue;
                };
                let new_descriptor = typed_child_descriptor_image(
                    post_image,
                    collection_tag(*collection),
                    &new_name,
                )?;
                if old_name == &new_name {
                    plan.file_mutations.push(TypedChildFileMutation {
                        path: old_descriptor_path,
                        pre_image: Some(old_descriptor),
                        post_image: Some(new_descriptor.clone()),
                    });
                } else {
                    plan.file_mutations.push(TypedChildFileMutation {
                        path: old_descriptor_path,
                        pre_image: Some(old_descriptor),
                        post_image: None,
                    });
                    let old_payload_dir = collection_dir.join(old_name);
                    if old_payload_dir.exists() {
                        let payload_files = read_typed_child_tree(&old_payload_dir)?;
                        plan.file_mutations.push(TypedChildFileMutation {
                            path: old_payload_dir,
                            pre_image: Some(Vec::new()),
                            post_image: None,
                        });
                        for (relative, bytes) in payload_files {
                            plan.file_mutations.push(TypedChildFileMutation {
                                path: collection_dir.join(&new_name).join(relative),
                                pre_image: None,
                                post_image: Some(bytes),
                            });
                        }
                    }
                    plan.file_mutations.push(TypedChildFileMutation {
                        path: collection_dir.join(format!("{new_name}.xml")),
                        pre_image: None,
                        post_image: Some(new_descriptor.clone()),
                    });
                }
                plan.validation_resources.push(MetadataResourceImage {
                    role: typed_child_role(*collection, owner, &new_name),
                    bytes: new_descriptor,
                });
                plan.publication_plan.push(MetaPublicationPlanEntry {
                    action: MetaPublicationAction::Update,
                    resource,
                    metadata_path: Some(typed_child_logical_address(
                        owner,
                        object_kind,
                        object_name,
                        *collection,
                        &new_name,
                    )?),
                });
            }
            continue;
        }
        if let MetaEditOperation::Remove {
            collection, names, ..
        } = operation
        {
            let Some((directory, resource)) = typed_physical_collection(*collection) else {
                continue;
            };
            for name in names {
                if request_created_names.contains(&(collection.as_str(), name.clone())) {
                    continue;
                }
                if request_renamed_destinations.contains(&(collection.as_str(), name.clone())) {
                    continue;
                }
                if !planned_existing_names.insert((collection.as_str(), name.clone())) {
                    continue;
                }
                let collection_dir = object_dir.join(directory);
                let descriptor = collection_dir.join(format!("{name}.xml"));
                let pre_image = read_typed_child_file(&descriptor, owner)?;
                plan.file_mutations.push(TypedChildFileMutation {
                    path: descriptor,
                    pre_image: Some(pre_image),
                    post_image: None,
                });
                let payload_dir = collection_dir.join(name);
                if payload_dir.exists() {
                    plan.file_mutations.push(TypedChildFileMutation {
                        path: payload_dir,
                        pre_image: Some(Vec::new()),
                        post_image: None,
                    });
                }
                plan.publication_plan.push(MetaPublicationPlanEntry {
                    action: MetaPublicationAction::Remove,
                    resource,
                    metadata_path: Some(typed_child_logical_address(
                        owner,
                        object_kind,
                        object_name,
                        *collection,
                        name,
                    )?),
                });
            }
            continue;
        }
        let MetaEditOperation::Add {
            collection,
            elements,
            ..
        } = operation
        else {
            continue;
        };
        let (directory, resource) = match collection {
            MetaCollection::Forms => ("Forms", MetaPublicationResource::Form),
            MetaCollection::Templates => ("Templates", MetaPublicationResource::Template),
            MetaCollection::Commands => ("Commands", MetaPublicationResource::Command),
            _ => continue,
        };
        for element in elements {
            let Some(final_name) = typed_new_child_final_name(
                *collection,
                &element.name,
                &operations[operation_index + 1..],
            ) else {
                continue;
            };
            let child_xml =
                typed_child_descriptor_image(post_image, collection_tag(*collection), &final_name)?;
            let collection_dir = object_dir.join(directory);
            plan.file_mutations.push(TypedChildFileMutation {
                path: collection_dir.join(format!("{final_name}.xml")),
                pre_image: None,
                post_image: Some(child_xml.clone()),
            });
            match collection {
                MetaCollection::Forms => {
                    let content = minimal_typed_form_content(object_kind, object_name);
                    plan.file_mutations.push(TypedChildFileMutation {
                        path: collection_dir.join(&final_name).join("Ext/Form.xml"),
                        pre_image: None,
                        post_image: Some(content.into_bytes()),
                    });
                }
                MetaCollection::Templates => {
                    plan.file_mutations.push(TypedChildFileMutation {
                        path: collection_dir.join(&final_name).join("Ext/Template.xml"),
                        pre_image: None,
                        post_image: Some(
                            super::super::mxl::empty_spreadsheet_document_xml().into_bytes(),
                        ),
                    });
                }
                MetaCollection::Commands => {}
                _ => unreachable!(),
            }
            let role = match collection {
                MetaCollection::Forms => MetadataResourceRole::Form {
                    owner: owner.clone(),
                    name: final_name.clone(),
                },
                MetaCollection::Templates => MetadataResourceRole::Template {
                    owner: owner.clone(),
                    name: final_name.clone(),
                },
                MetaCollection::Commands => MetadataResourceRole::Command {
                    owner: owner.clone(),
                    name: final_name.clone(),
                },
                _ => unreachable!(),
            };
            plan.validation_resources.push(MetadataResourceImage {
                role,
                bytes: child_xml,
            });
            let child_kind = match collection {
                MetaCollection::Forms => "Form",
                MetaCollection::Templates => "Template",
                MetaCollection::Commands => "Command",
                _ => unreachable!(),
            };
            let child_path = MetadataAddress::parse(
                PLATFORM_XML_8_3_27_FORMAT_2_20,
                &format!(
                    "{}.{}.{}.{}",
                    object_kind, object_name, child_kind, final_name
                ),
            )
            .map_err(|_| {
                MetaFailure::from(
                    typed_diagnostic(
                        MetaDiagnosticCode::ProviderUnavailable,
                        "typed child logical address cannot be represented",
                        Some("collection"),
                    )
                    .with_metadata_path(owner.clone()),
                )
            })?;
            plan.publication_plan.push(MetaPublicationPlanEntry {
                action: MetaPublicationAction::Create,
                resource,
                metadata_path: Some(child_path),
            });
        }
    }
    Ok(plan)
}

fn typed_physical_collection(
    collection: MetaCollection,
) -> Option<(&'static str, MetaPublicationResource)> {
    match collection {
        MetaCollection::Forms => Some(("Forms", MetaPublicationResource::Form)),
        MetaCollection::Templates => Some(("Templates", MetaPublicationResource::Template)),
        MetaCollection::Commands => Some(("Commands", MetaPublicationResource::Command)),
        _ => None,
    }
}

fn typed_new_child_final_name(
    collection: MetaCollection,
    initial_name: &str,
    later_operations: &[MetaEditOperation],
) -> Option<String> {
    let mut name = initial_name.to_string();
    for operation in later_operations {
        match operation {
            MetaEditOperation::Update {
                collection: candidate,
                elements,
                ..
            } if *candidate == collection => {
                if let Some(update) = elements.iter().find(|element| element.name == name) {
                    if let Some(new_name) = &update.new_name {
                        name = new_name.clone();
                    }
                }
            }
            MetaEditOperation::Remove {
                collection: candidate,
                names,
                ..
            } if *candidate == collection && names.contains(&name) => return None,
            _ => {}
        }
    }
    Some(name)
}

fn typed_child_role(
    collection: MetaCollection,
    owner: &MetadataAddress,
    name: &str,
) -> MetadataResourceRole {
    match collection {
        MetaCollection::Forms => MetadataResourceRole::Form {
            owner: owner.clone(),
            name: name.to_string(),
        },
        MetaCollection::Templates => MetadataResourceRole::Template {
            owner: owner.clone(),
            name: name.to_string(),
        },
        MetaCollection::Commands => MetadataResourceRole::Command {
            owner: owner.clone(),
            name: name.to_string(),
        },
        _ => unreachable!(),
    }
}

fn typed_child_logical_address(
    owner: &MetadataAddress,
    object_kind: &str,
    object_name: &str,
    collection: MetaCollection,
    name: &str,
) -> Result<MetadataAddress, MetaFailure> {
    let child_kind = match collection {
        MetaCollection::Forms => "Form",
        MetaCollection::Templates => "Template",
        MetaCollection::Commands => "Command",
        _ => unreachable!(),
    };
    MetadataAddress::parse(
        PLATFORM_XML_8_3_27_FORMAT_2_20,
        &format!("{object_kind}.{object_name}.{child_kind}.{name}"),
    )
    .map_err(|_| {
        MetaFailure::from(
            typed_diagnostic(
                MetaDiagnosticCode::ProviderUnavailable,
                "typed child logical address cannot be represented",
                Some("collection"),
            )
            .with_metadata_path(owner.clone()),
        )
    })
}

fn read_typed_child_file(path: &Path, owner: &MetadataAddress) -> Result<Vec<u8>, MetaFailure> {
    fs::read(path).map_err(|_| {
        MetaFailure::from(
            typed_diagnostic(
                MetaDiagnosticCode::ProviderUnavailable,
                "typed child descriptor pre-image is unavailable",
                None,
            )
            .with_metadata_path(owner.clone()),
        )
    })
}

fn read_typed_child_tree(root: &Path) -> Result<Vec<(PathBuf, Vec<u8>)>, MetaFailure> {
    fn visit(
        root: &Path,
        current: &Path,
        files: &mut Vec<(PathBuf, Vec<u8>)>,
    ) -> Result<(), MetaFailure> {
        let entries = fs::read_dir(current).map_err(|_| {
            MetaFailure::from(typed_diagnostic(
                MetaDiagnosticCode::ProviderUnavailable,
                "typed child payload tree is unavailable",
                None,
            ))
        })?;
        for entry in entries {
            let entry = entry.map_err(|_| {
                MetaFailure::from(typed_diagnostic(
                    MetaDiagnosticCode::ProviderUnavailable,
                    "typed child payload tree is unavailable",
                    None,
                ))
            })?;
            let file_type = entry.file_type().map_err(|_| {
                MetaFailure::from(typed_diagnostic(
                    MetaDiagnosticCode::ProviderUnavailable,
                    "typed child payload entry type is unavailable",
                    None,
                ))
            })?;
            if file_type.is_symlink() {
                return Err(MetaFailure::from(typed_diagnostic(
                    MetaDiagnosticCode::ProviderUnavailable,
                    "typed child payload tree contains a symbolic link",
                    None,
                )));
            }
            if file_type.is_dir() {
                visit(root, &entry.path(), files)?;
            } else if file_type.is_file() {
                let path = entry.path();
                let bytes = fs::read(&path).map_err(|_| {
                    MetaFailure::from(typed_diagnostic(
                        MetaDiagnosticCode::ProviderUnavailable,
                        "typed child payload file is unavailable",
                        None,
                    ))
                })?;
                files.push((path.strip_prefix(root).unwrap().to_path_buf(), bytes));
            }
        }
        Ok(())
    }
    let mut files = Vec::new();
    visit(root, root, &mut files)?;
    files.sort_by(|left, right| left.0.cmp(&right.0));
    Ok(files)
}

fn typed_child_descriptor_image(
    owner_xml: &str,
    tag: &str,
    name: &str,
) -> Result<Vec<u8>, MetaFailure> {
    let document = Document::parse(owner_xml.trim_start_matches('\u{feff}')).map_err(|_| {
        MetaFailure::from(typed_diagnostic(
            MetaDiagnosticCode::ProviderUnavailable,
            "typed owner post-image is not valid XML",
            None,
        ))
    })?;
    let child = document
        .descendants()
        .filter(|node| node.is_element() && node.tag_name().name() == tag)
        .find(|node| meta_edit_child_object_name(*node).as_deref() == Some(name))
        .ok_or_else(|| {
            MetaFailure::from(typed_diagnostic(
                MetaDiagnosticCode::ProviderUnavailable,
                "typed child descriptor is missing from owner post-image",
                Some("elements"),
            ))
        })?;
    let child_xml = &owner_xml[child.range()];
    Ok(format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<MetaDataObject {} version=\"2.20\">\n{}\n</MetaDataObject>",
        super::template_catalog::meta_xmlns_decl(),
        child_xml
    )
    .into_bytes())
}

fn minimal_typed_form_content(_object_kind: &str, _object_name: &str) -> String {
    concat!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n",
        "<Form xmlns=\"http://v8.1c.ru/8.3/xcf/logform\" version=\"2.20\">\n",
        "\t<AutoCommandBar name=\"ФормаКоманднаяПанель\" id=\"-1\"/>\n",
        "</Form>"
    )
    .to_string()
}

pub(crate) fn resolve_typed_edit_object(
    request: &MetaEditRequest,
    context: &WorkspaceContext,
    cancellation: &CancellationToken,
) -> Result<ResolvedMetadataObject, MetaFailure> {
    if cancellation.is_cancelled() {
        return Err(typed_diagnostic(
            MetaDiagnosticCode::ProviderUnavailable,
            "metadata edit was cancelled before source resolution",
            None,
        )
        .with_metadata_path(request.metadata_path.clone())
        .into());
    }
    let target = SourceTarget {
        source_set: request.source_set.clone(),
        metadata_path: Some(request.metadata_path.clone()),
    };
    let resolution = resolve_platform_xml_target(context, &target, TargetKindPolicy::Any)
        .map_err(|error| typed_resolution_failure(request, error.code))?;
    if resolution.resolved.target_kind != TargetKind::MetadataObject {
        return Err(typed_diagnostic(
            MetaDiagnosticCode::InvalidArguments,
            "metadataPath must identify one existing metadata object",
            Some("metadataPath"),
        )
        .with_metadata_path(request.metadata_path.clone())
        .into());
    }
    let evidence = platform_xml_resource_evidence(context, &resolution.handle)
        .map_err(|error| typed_resolution_failure(request, error.code))?;
    let descriptor_preimage = fs::read(&evidence.target_path).map_err(|_| {
        MetaFailure::from(
            typed_diagnostic(
                MetaDiagnosticCode::ProviderUnavailable,
                "metadata descriptor image is unavailable",
                None,
            )
            .with_metadata_path(request.metadata_path.clone()),
        )
    })?;
    let owner_preimage = fs::read(&evidence.registration_path).map_err(|_| {
        MetaFailure::from(
            typed_diagnostic(
                MetaDiagnosticCode::ProviderUnavailable,
                "metadata owner image is unavailable",
                None,
            )
            .with_metadata_path(request.metadata_path.clone()),
        )
    })?;
    let (_, descriptor) =
        super::xml_model::parse_metadata_image(&descriptor_preimage).map_err(|_| {
            MetaFailure::from(
                typed_diagnostic(
                    MetaDiagnosticCode::ProviderUnavailable,
                    "metadata descriptor image is unreadable",
                    None,
                )
                .with_metadata_path(request.metadata_path.clone()),
            )
        })?;
    if descriptor.root_element().attribute("version") != Some(ACTIVE_FORMAT_PROFILE.export_format) {
        return Err(typed_diagnostic(
            MetaDiagnosticCode::CapabilityUnavailable,
            format!(
                "metadata object is outside the supported {} export format",
                ACTIVE_FORMAT_PROFILE.export_format
            ),
            None,
        )
        .with_metadata_path(request.metadata_path.clone())
        .into());
    }
    Ok(ResolvedMetadataObject {
        handle: resolution.handle,
        descriptor_path: evidence.target_path,
        descriptor_preimage,
        source_root: evidence.source_root,
        owner_path: evidence.registration_path,
        owner_preimage,
    })
}

fn typed_resolution_failure(
    request: &MetaEditRequest,
    code: crate::domain::source_target::SourceTargetErrorCode,
) -> MetaFailure {
    use crate::domain::source_target::SourceTargetErrorCode;
    let diagnostic_code = match code {
        SourceTargetErrorCode::SourceSetNotFound
        | SourceTargetErrorCode::MetadataAddressNotFound => MetaDiagnosticCode::TargetNotFound,
        SourceTargetErrorCode::MetadataAddressInvalid
        | SourceTargetErrorCode::TargetKindMismatch
        | SourceTargetErrorCode::SourceSetRequired => MetaDiagnosticCode::InvalidArguments,
        SourceTargetErrorCode::ContainmentDenied => MetaDiagnosticCode::ProviderUnavailable,
        _ => MetaDiagnosticCode::CapabilityUnavailable,
    };
    typed_diagnostic(
        diagnostic_code,
        match diagnostic_code {
            MetaDiagnosticCode::TargetNotFound => "metadata target was not found",
            MetaDiagnosticCode::InvalidArguments => "metadata target is not an editable object",
            MetaDiagnosticCode::ProviderUnavailable => {
                "metadata target could not be resolved safely"
            }
            _ => "metadata target does not provide typed Platform XML editing",
        },
        Some("metadataPath"),
    )
    .with_metadata_path(request.metadata_path.clone())
    .into()
}

pub(crate) fn prepare_typed_edit(
    request: &MetaEditRequest,
    resolved: ResolvedMetadataObject,
    context: &WorkspaceContext,
) -> Result<Box<dyn PreparedMetadataMutation>, MetaFailure> {
    let target = request.metadata_path.clone();
    let mut diagnostics = Vec::new();
    match evaluate_resolved_support_guard(
        &resolved.descriptor_path,
        crate::application::SupportGuardRequirement::Editable,
        context,
    ) {
        ResolvedSupportGuardCheck::Allow => {}
        ResolvedSupportGuardCheck::Warn(_) => diagnostics.push(MetaDiagnostic {
            code: MetaDiagnosticCode::SupportLocked,
            severity: crate::domain::metadata::MetaDiagnosticSeverity::Warning,
            message: "metadata source support policy permits editing with a warning".to_string(),
            metadata_path: Some(target.clone()),
            operation_index: None,
            field: None,
        }),
        ResolvedSupportGuardCheck::Block(_) => {
            return Err(typed_diagnostic(
                MetaDiagnosticCode::SupportLocked,
                "metadata source support policy blocks object editing",
                None,
            )
            .with_metadata_path(target)
            .into())
        }
    }
    let mut xml = String::from_utf8(resolved.descriptor_preimage.clone()).map_err(|_| {
        MetaFailure::from(
            typed_diagnostic(
                MetaDiagnosticCode::ProviderUnavailable,
                "metadata descriptor image is not UTF-8",
                None,
            )
            .with_metadata_path(request.metadata_path.clone()),
        )
    })?;
    let source_format = MetaEditSourceFormat {
        has_bom: resolved.descriptor_preimage.starts_with(b"\xef\xbb\xbf"),
        eol: meta_edit_source_eol(&xml),
    };
    if xml.starts_with('\u{feff}') {
        xml = xml.trim_start_matches('\u{feff}').to_string();
    }
    apply_typed_operations(&mut xml, &request.operations).map_err(|mut failure| {
        for diagnostic in &mut failure.diagnostics {
            diagnostic.metadata_path = Some(request.metadata_path.clone());
        }
        failure
    })?;
    let (object_kind, object_name) = meta_edit_object_identity(&xml).map_err(|_| {
        MetaFailure::from(
            typed_diagnostic(
                MetaDiagnosticCode::ProviderUnavailable,
                "typed metadata post-image identity is unavailable",
                None,
            )
            .with_metadata_path(request.metadata_path.clone()),
        )
    })?;
    let child_resources = plan_typed_child_resources(
        &resolved.descriptor_path,
        &request.metadata_path,
        &object_kind,
        &object_name,
        &request.operations,
        &xml,
    )?;
    let mut child_resources = child_resources;
    child_resources.relation_dependencies =
        resolve_typed_relation_dependencies(request, context, &xml)?;
    let post_image = meta_edit_preserve_source_format(&xml, source_format);
    PreparedMetaEdit::prepare(
        request,
        resolved,
        context,
        post_image,
        diagnostics,
        child_resources,
    )
}

fn resolve_typed_relation_dependencies(
    request: &MetaEditRequest,
    context: &WorkspaceContext,
    owner_post_image: &str,
) -> Result<Vec<TypedRelationDependency>, MetaFailure> {
    let mut dependencies = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for (operation_index, operation) in request.operations.iter().enumerate() {
        let MetaEditOperation::EditRelations {
            relation, targets, ..
        } = operation
        else {
            continue;
        };
        for (target_index, target) in targets.iter().enumerate() {
            validate_typed_relation_target(
                &request.metadata_path,
                owner_post_image,
                *relation,
                target,
            )
            .map_err(|mut diagnostic| {
                diagnostic.operation_index = Some(operation_index);
                diagnostic.field = Some(format!(
                    "operations[{operation_index}].targets[{target_index}]"
                ));
                MetaFailure::from(diagnostic.with_metadata_path(request.metadata_path.clone()))
            })?;
            let dependency = target.dependency();
            if dependency == &request.metadata_path || !seen.insert(dependency.clone()) {
                continue;
            }
            let source_target = SourceTarget {
                source_set: request.source_set.clone(),
                metadata_path: Some(dependency.clone()),
            };
            let resolution =
                resolve_platform_xml_target(context, &source_target, TargetKindPolicy::Any)
                    .map_err(|_| {
                        MetaFailure::from(
                            typed_diagnostic(
                                MetaDiagnosticCode::TargetNotFound,
                                "relation target does not resolve in the selected source set",
                                Some(&format!(
                                    "operations[{operation_index}].targets[{target_index}]"
                                )),
                            )
                            .with_operation_index(operation_index)
                            .with_metadata_path(request.metadata_path.clone()),
                        )
                    })?;
            if resolution.resolved.target_kind != TargetKind::MetadataObject {
                return Err(MetaFailure::from(
                    typed_diagnostic(
                        MetaDiagnosticCode::InvalidArguments,
                        "relation target must identify a metadata object",
                        Some(&format!(
                            "operations[{operation_index}].targets[{target_index}]"
                        )),
                    )
                    .with_operation_index(operation_index)
                    .with_metadata_path(request.metadata_path.clone()),
                ));
            }
            let evidence =
                platform_xml_resource_evidence(context, &resolution.handle).map_err(|_| {
                    MetaFailure::from(
                        typed_diagnostic(
                            MetaDiagnosticCode::ProviderUnavailable,
                            "relation target evidence is unavailable",
                            Some(&format!(
                                "operations[{operation_index}].targets[{target_index}]"
                            )),
                        )
                        .with_operation_index(operation_index)
                        .with_metadata_path(request.metadata_path.clone()),
                    )
                })?;
            let bytes = fs::read(&evidence.target_path).map_err(|_| {
                MetaFailure::from(
                    typed_diagnostic(
                        MetaDiagnosticCode::ProviderUnavailable,
                        "relation target pre-image is unavailable",
                        Some(&format!(
                            "operations[{operation_index}].targets[{target_index}]"
                        )),
                    )
                    .with_operation_index(operation_index)
                    .with_metadata_path(request.metadata_path.clone()),
                )
            })?;
            dependencies.push(TypedRelationDependency {
                handle: resolution.handle,
                path: evidence.target_path,
                bytes,
                target: dependency.clone(),
            });
        }
    }
    Ok(dependencies)
}

fn validate_typed_relation_target(
    owner: &MetadataAddress,
    owner_post_image: &str,
    relation: MetaRelation,
    target: &crate::domain::metadata::MetaRelationTarget,
) -> Result<(), MetaDiagnostic> {
    let target_kind = target.dependency().segments().next().unwrap_or_default();
    match relation {
        MetaRelation::Owners => {
            if owner.segments().next() != Some("Catalog") || target_kind != "Catalog" {
                return Err(typed_diagnostic(
                    MetaDiagnosticCode::InvalidArguments,
                    "owners requires Catalog owner and Catalog targets",
                    Some("targets"),
                ));
            }
        }
        MetaRelation::RegisterRecords => {
            if owner.segments().next() != Some("Document")
                || !matches!(
                    target_kind,
                    "InformationRegister"
                        | "AccumulationRegister"
                        | "AccountingRegister"
                        | "CalculationRegister"
                )
            {
                return Err(typed_diagnostic(
                    MetaDiagnosticCode::InvalidArguments,
                    "registerRecords requires Document owner and register targets",
                    Some("targets"),
                ));
            }
        }
        MetaRelation::BasedOn => {
            if owner.segments().next() != Some(target_kind) {
                return Err(typed_diagnostic(
                    MetaDiagnosticCode::InvalidArguments,
                    "basedOn target kind must match the edited object kind",
                    Some("targets"),
                ));
            }
        }
        MetaRelation::InputByString => {
            let crate::domain::metadata::MetaRelationTarget::Field(field) = target else {
                return Err(typed_diagnostic(
                    MetaDiagnosticCode::InvalidArguments,
                    "inputByString requires typed field paths",
                    Some("targets"),
                ));
            };
            if &field.owner != owner {
                return Err(typed_diagnostic(
                    MetaDiagnosticCode::InvalidArguments,
                    "inputByString field must belong to the edited object",
                    Some("targets"),
                ));
            }
            if field.kind == crate::domain::metadata::MetadataFieldKind::Attribute {
                let document = Document::parse(owner_post_image).map_err(|_| {
                    typed_diagnostic(
                        MetaDiagnosticCode::ProviderUnavailable,
                        "owner post-image is not valid XML",
                        Some("targets"),
                    )
                })?;
                let found = document
                    .descendants()
                    .filter(|node| node.is_element() && node.tag_name().name() == "Attribute")
                    .any(|node| meta_edit_child_object_name(node).as_deref() == Some(&field.name));
                if !found {
                    return Err(typed_diagnostic(
                        MetaDiagnosticCode::TargetNotFound,
                        "inputByString attribute does not exist in the owner post-image",
                        Some("targets"),
                    ));
                }
            } else if !metadata_standard_attribute_names(target_kind).contains(&field.name.as_str())
            {
                return Err(typed_diagnostic(
                    MetaDiagnosticCode::TargetNotFound,
                    "inputByString standard attribute does not exist for the owner kind",
                    Some("targets"),
                ));
            }
        }
    }
    Ok(())
}

/// Apply a closed sequence to one private working image. The caller observes
/// either the complete post-image or the exact preimage; partial operation
/// results never escape this function.
pub(crate) fn apply_typed_operations(
    xml_text: &mut String,
    operations: &[MetaEditOperation],
) -> Result<MetaEditCounts, MetaFailure> {
    let mut working = xml_text.clone();
    let mut counts = MetaEditCounts::default();
    for (operation_index, operation) in operations.iter().enumerate() {
        if let Err(diagnostic) = apply_typed_operation(&mut working, operation, &mut counts) {
            let mut diagnostic = diagnostic.with_operation_index(operation_index);
            if let Some(field) = diagnostic.field.as_mut() {
                if !field.starts_with("operations[") {
                    *field = format!("operations[{operation_index}].{field}");
                }
            }
            return Err(diagnostic.into());
        }
        Document::parse(working.trim_start_matches('\u{feff}')).map_err(|_| {
            MetaFailure::from(
                typed_diagnostic(
                    MetaDiagnosticCode::ValidationFailed,
                    "typed metadata operation produced invalid XML",
                    None,
                )
                .with_operation_index(operation_index),
            )
        })?;
        validate_metadata_8_3_27_boolean_contract(&working, "meta.edit").map_err(|_| {
            MetaFailure::from(
                typed_diagnostic(
                    MetaDiagnosticCode::ValidationFailed,
                    "typed metadata operation violates the boolean format contract",
                    None,
                )
                .with_operation_index(operation_index),
            )
        })?;
        validate_metadata_8_3_27_enum_contract(&working, "meta.edit").map_err(|_| {
            MetaFailure::from(
                typed_diagnostic(
                    MetaDiagnosticCode::ValidationFailed,
                    "typed metadata operation violates the enum format contract",
                    None,
                )
                .with_operation_index(operation_index),
            )
        })?;
    }
    *xml_text = working;
    Ok(counts)
}

fn apply_typed_operation(
    xml_text: &mut String,
    operation: &MetaEditOperation,
    counts: &mut MetaEditCounts,
) -> Result<(), MetaDiagnostic> {
    let (object_kind, object_name) = meta_edit_object_identity(xml_text).map_err(|_| {
        typed_diagnostic(
            MetaDiagnosticCode::ProviderUnavailable,
            "metadata descriptor identity is unavailable",
            None,
        )
    })?;
    match operation {
        MetaEditOperation::SetProperties { values } => {
            apply_typed_properties(xml_text, values)?;
            counts.modified += values.entries().len();
        }
        MetaEditOperation::Add {
            collection,
            scope,
            elements,
        } => {
            ensure_typed_collection_allowed(&object_kind, *collection)?;
            for (index, element) in elements.iter().enumerate() {
                add_typed_element(
                    xml_text,
                    &object_kind,
                    &object_name,
                    *collection,
                    scope.as_ref().map(|scope| scope.tabular_section.as_str()),
                    element,
                )
                .map_err(|diagnostic| qualify_element_diagnostic(diagnostic, index))?;
                counts.added += 1;
            }
        }
        MetaEditOperation::Update {
            collection,
            scope,
            elements,
        } => {
            ensure_typed_collection_allowed(&object_kind, *collection)?;
            for (index, element) in elements.iter().enumerate() {
                update_typed_element(
                    xml_text,
                    *collection,
                    scope.as_ref().map(|scope| scope.tabular_section.as_str()),
                    element,
                )
                .map_err(|diagnostic| qualify_element_diagnostic(diagnostic, index))?;
                counts.modified += 1;
            }
        }
        MetaEditOperation::Remove {
            collection,
            scope,
            names,
        } => {
            ensure_typed_collection_allowed(&object_kind, *collection)?;
            for (index, name) in names.iter().enumerate() {
                remove_typed_element(
                    xml_text,
                    *collection,
                    scope.as_ref().map(|scope| scope.tabular_section.as_str()),
                    name,
                )
                .map_err(|mut diagnostic| {
                    diagnostic.field = Some(format!("names[{index}]"));
                    diagnostic
                })?;
                counts.removed += 1;
            }
        }
        MetaEditOperation::EditRelations {
            relation,
            mode,
            targets,
        } => {
            apply_typed_relations(xml_text, &object_kind, *relation, *mode, targets)?;
            counts.modified += 1;
        }
    }
    Ok(())
}

fn qualify_element_diagnostic(mut diagnostic: MetaDiagnostic, index: usize) -> MetaDiagnostic {
    diagnostic.field = Some(match diagnostic.field.take() {
        Some(field) if field.starts_with("scope") || field.starts_with("collection") => field,
        Some(field) => format!("elements[{index}].{field}"),
        None => format!("elements[{index}]"),
    });
    diagnostic
}

fn typed_diagnostic(
    code: MetaDiagnosticCode,
    message: impl Into<String>,
    field: Option<&str>,
) -> MetaDiagnostic {
    let diagnostic = MetaDiagnostic::error(code, message);
    match field {
        Some(field) => diagnostic.with_field(field),
        None => diagnostic,
    }
}

fn ensure_typed_collection_allowed(
    object_kind: &str,
    collection: MetaCollection,
) -> Result<(), MetaDiagnostic> {
    let kind = crate::domain::metadata::MetadataKind::parse(object_kind).map_err(|_| {
        typed_diagnostic(
            MetaDiagnosticCode::UnsupportedKind,
            format!("metadata kind `{object_kind}` has no typed collection profile"),
            Some("collection"),
        )
    })?;
    let allowed = crate::domain::metadata::metadata_kind_collections(kind).contains(&collection);
    if allowed {
        Ok(())
    } else {
        Err(typed_diagnostic(
            MetaDiagnosticCode::UnsupportedKind,
            format!(
                "collection `{}` is not supported for {object_kind}",
                collection.as_str()
            ),
            Some("collection"),
        ))
    }
}

fn apply_typed_properties(
    xml_text: &mut String,
    changes: &crate::domain::metadata::MetaPropertyChanges,
) -> Result<(), MetaDiagnostic> {
    let doc = Document::parse(xml_text.trim_start_matches('\u{feff}')).map_err(|_| {
        typed_diagnostic(
            MetaDiagnosticCode::ProviderUnavailable,
            "metadata descriptor is not valid XML",
            None,
        )
    })?;
    let object = meta_edit_object_node(&doc).map_err(|_| {
        typed_diagnostic(
            MetaDiagnosticCode::ProviderUnavailable,
            "metadata descriptor object is unavailable",
            None,
        )
    })?;
    let properties = meta_info_child(object, "Properties").ok_or_else(|| {
        typed_diagnostic(
            MetaDiagnosticCode::ProviderUnavailable,
            "metadata descriptor has no Properties",
            None,
        )
    })?;
    let range = properties.range();
    drop(doc);
    let mut text = xml_text[range.clone()].to_string();
    let indent = meta_edit_property_child_indent(&text);
    for (key, value) in changes.entries() {
        let tag = match key {
            MetaPropertyKey::Synonym => "Synonym",
            MetaPropertyKey::Comment => "Comment",
            MetaPropertyKey::NumberLength => "NumberLength",
            MetaPropertyKey::CheckUnique => "CheckUnique",
            MetaPropertyKey::CodeLength => "CodeLength",
            MetaPropertyKey::DescriptionLength => "DescriptionLength",
            MetaPropertyKey::Hierarchical => "Hierarchical",
            MetaPropertyKey::Autonumbering => "Autonumbering",
            MetaPropertyKey::UseStandardCommands => "UseStandardCommands",
        };
        if meta_edit_xml_element_range(&text, tag)
            .map_err(|_| {
                typed_diagnostic(
                    MetaDiagnosticCode::ProviderUnavailable,
                    "metadata property image is malformed",
                    Some("values"),
                )
            })?
            .is_none()
        {
            return Err(typed_diagnostic(
                MetaDiagnosticCode::InvalidArguments,
                format!("property `{tag}` is not present on this metadata object"),
                Some("values"),
            ));
        }
        let replacement = match (key, value) {
            (MetaPropertyKey::Synonym, MetaPropertyValue::String(value)) => {
                let mut lines = Vec::new();
                emit_meta_mltext(&mut lines, &indent, tag, value);
                lines.join("\n")
            }
            (MetaPropertyKey::Comment, MetaPropertyValue::String(value)) if value.is_empty() => {
                format!("{indent}<{tag}/>")
            }
            (_, MetaPropertyValue::String(value)) => {
                format!("{indent}<{tag}>{}</{tag}>", escape_xml(value))
            }
            (_, MetaPropertyValue::Boolean(value)) => {
                format!("{indent}<{tag}>{value}</{tag}>")
            }
            (_, MetaPropertyValue::UnsignedInteger(value)) => {
                format!("{indent}<{tag}>{value}</{tag}>")
            }
        };
        meta_edit_replace_or_insert_property(&mut text, tag, &replacement, &indent).map_err(
            |_| {
                typed_diagnostic(
                    MetaDiagnosticCode::ProviderUnavailable,
                    "metadata property could not be updated",
                    Some("values"),
                )
            },
        )?;
    }
    xml_text.replace_range(range, &text);
    Ok(())
}

fn collection_tag(collection: MetaCollection) -> &'static str {
    match collection {
        MetaCollection::Attributes => "Attribute",
        MetaCollection::TabularSections => "TabularSection",
        MetaCollection::Dimensions => "Dimension",
        MetaCollection::Resources => "Resource",
        MetaCollection::EnumValues => "EnumValue",
        MetaCollection::Columns => "Column",
        MetaCollection::Forms => "Form",
        MetaCollection::Templates => "Template",
        MetaCollection::Commands => "Command",
    }
}

fn add_typed_element(
    xml_text: &mut String,
    object_kind: &str,
    object_name: &str,
    collection: MetaCollection,
    scope: Option<&str>,
    element: &MetaElementDefinition,
) -> Result<(), MetaDiagnostic> {
    if !is_1c_identifier(&element.name) {
        return Err(typed_diagnostic(
            MetaDiagnosticCode::InvalidArguments,
            "metadata element name is invalid",
            Some("name"),
        ));
    }
    let tag = collection_tag(collection);
    ensure_typed_name_free(xml_text, tag, scope, &element.name)?;
    let position = typed_insert_position(element.position.as_ref());
    let lines = render_typed_element(object_kind, object_name, collection, element)?;
    let result = match scope {
        Some(section) => meta_edit_insert_tabular_child_object_with_position(
            xml_text, section, tag, &position, &lines,
        ),
        None => meta_edit_insert_top_child_object_with_position(xml_text, tag, &position, &lines),
    };
    result.map_err(|_| {
        typed_diagnostic(
            MetaDiagnosticCode::TargetNotFound,
            "metadata position or scope target was not found",
            Some(if scope.is_some() { "scope" } else { "position" }),
        )
    })
}

fn typed_insert_position(position: Option<&MetaPosition>) -> MetaEditInsertPosition {
    position.map_or_else(MetaEditInsertPosition::default, |position| {
        MetaEditInsertPosition {
            before: position.before.clone(),
            after: position.after.clone(),
        }
    })
}

fn ensure_typed_name_free(
    xml_text: &str,
    tag: &str,
    scope: Option<&str>,
    name: &str,
) -> Result<(), MetaDiagnostic> {
    let result = match scope {
        Some(section) => meta_edit_ensure_tabular_child_name_free(xml_text, section, tag, name),
        None => meta_edit_ensure_top_child_name_free(xml_text, tag, name),
    };
    result.map_err(|message| {
        let code = if message.contains("already exists") {
            MetaDiagnosticCode::AlreadyExists
        } else {
            MetaDiagnosticCode::TargetNotFound
        };
        typed_diagnostic(
            code,
            "metadata element identity is not available",
            Some("name"),
        )
    })
}

fn render_typed_element(
    object_kind: &str,
    object_name: &str,
    collection: MetaCollection,
    element: &MetaElementDefinition,
) -> Result<Vec<String>, MetaDiagnostic> {
    validate_typed_element_value_profile(element)?;
    let mut lines = Vec::new();
    let mut next_uuid = fresh_meta_compile_uuid;
    let attr = || MetaCompileAttr {
        name: element.name.clone(),
        type_name: String::new(),
        synonym: element
            .synonym
            .clone()
            .unwrap_or_else(|| split_meta_camel_case(&element.name)),
        flags: element
            .required
            .is_some_and(|required| required)
            .then(|| "req".to_string())
            .into_iter()
            .collect(),
        fill_checking: String::new(),
        indexing: String::new(),
        multi_line: false,
        choice_history_on_input: String::new(),
    };
    match collection {
        MetaCollection::Attributes => emit_meta_attribute(
            &mut lines,
            "\t\t\t",
            &attr(),
            meta_attribute_context(object_kind),
            &mut next_uuid,
        ),
        MetaCollection::TabularSections => {
            let ordered_attributes = order_typed_nested_attributes(&element.attributes)?;
            let columns = ordered_attributes
                .iter()
                .map(|attribute| MetaCompileAttr {
                    name: attribute.name.clone(),
                    type_name: String::new(),
                    synonym: attribute
                        .synonym
                        .clone()
                        .unwrap_or_else(|| split_meta_camel_case(&attribute.name)),
                    flags: Vec::new(),
                    fill_checking: String::new(),
                    indexing: String::new(),
                    multi_line: false,
                    choice_history_on_input: String::new(),
                })
                .collect();
            emit_meta_tabular_section(
                &mut lines,
                "\t\t\t",
                &MetaCompileTabularSection {
                    name: element.name.clone(),
                    columns,
                },
                object_kind,
                object_name,
                &mut next_uuid,
            );
            let mut rendered = lines.join("\n");
            apply_typed_nested_attribute_fields(&mut rendered, &ordered_attributes)?;
            return Ok(rendered.lines().map(ToOwned::to_owned).collect());
        }
        MetaCollection::Dimensions | MetaCollection::Resources => emit_meta_register_field(
            &mut lines,
            "\t\t\t",
            collection_tag(collection),
            &attr(),
            object_kind,
            &mut next_uuid,
        ),
        MetaCollection::EnumValues => emit_meta_enum_value(
            &mut lines,
            "\t\t\t",
            &MetaCompileEnumValue {
                name: element.name.clone(),
                synonym: element
                    .synonym
                    .clone()
                    .unwrap_or_else(|| split_meta_camel_case(&element.name)),
                comment: element.comment.clone().unwrap_or_default(),
            },
            &mut next_uuid,
        ),
        MetaCollection::Columns
        | MetaCollection::Forms
        | MetaCollection::Templates
        | MetaCollection::Commands => emit_meta_simple_child(
            &mut lines,
            "\t\t\t",
            collection_tag(collection),
            &element.name,
            &mut next_uuid,
        ),
    }
    let mut rendered = lines.join("\n");
    if collection == MetaCollection::Forms {
        rendered = rendered
            .replace("<FormType>Ordinary</FormType>", "<FormType>Managed</FormType>")
            .replace(
                "<UsePurposes/>",
                concat!(
                    "<UsePurposes>\n",
                    "\t\t\t\t\t<v8:Value xsi:type=\"app:ApplicationUsePurpose\">PlatformApplication</v8:Value>\n",
                    "\t\t\t\t\t<v8:Value xsi:type=\"app:ApplicationUsePurpose\">MobilePlatformApplication</v8:Value>\n",
                    "\t\t\t\t</UsePurposes>"
                ),
            );
    }
    apply_typed_element_fields(&mut rendered, element);
    Ok(rendered.lines().map(ToOwned::to_owned).collect())
}

fn order_typed_nested_attributes(
    attributes: &[MetaElementDefinition],
) -> Result<Vec<MetaElementDefinition>, MetaDiagnostic> {
    let mut ordered = Vec::<MetaElementDefinition>::with_capacity(attributes.len());
    for (index, attribute) in attributes.iter().cloned().enumerate() {
        let position = attribute.position.clone();
        ordered.push(attribute);
        let Some(position) = position else {
            continue;
        };
        let anchor = position
            .before
            .as_ref()
            .or(position.after.as_ref())
            .unwrap();
        let Some(anchor_index) = ordered
            .iter()
            .position(|candidate| &candidate.name == anchor)
        else {
            return Err(typed_diagnostic(
                MetaDiagnosticCode::TargetNotFound,
                "nested attribute position target was not found",
                Some(&format!("attributes[{index}].position")),
            ));
        };
        let current = ordered.pop().unwrap();
        let insertion = if position.before.is_some() {
            anchor_index
        } else {
            anchor_index + 1
        };
        ordered.insert(insertion, current);
    }
    Ok(ordered)
}

fn apply_typed_nested_attribute_fields(
    xml: &mut String,
    attributes: &[MetaElementDefinition],
) -> Result<(), MetaDiagnostic> {
    const WRAPPER: &str = "<MetaDataObject xmlns=\"http://v8.1c.ru/8.3/MDClasses\" xmlns:v8=\"http://v8.1c.ru/8.1/data/core\" xmlns:xr=\"http://v8.1c.ru/8.3/xcf/readable\" xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\">";
    for attribute in attributes {
        validate_typed_element_value_profile(attribute)?;
        let wrapped = format!("{WRAPPER}{xml}</MetaDataObject>");
        let document = Document::parse(&wrapped).map_err(|_| {
            typed_diagnostic(
                MetaDiagnosticCode::ProviderUnavailable,
                "rendered tabular section is not valid XML",
                None,
            )
        })?;
        let range = document
            .descendants()
            .filter(|node| node.is_element() && node.tag_name().name() == "Attribute")
            .find(|node| meta_edit_child_object_name(*node).as_deref() == Some(&attribute.name))
            .map(|node| {
                let range = node.range();
                range.start - WRAPPER.len()..range.end - WRAPPER.len()
            })
            .ok_or_else(|| {
                typed_diagnostic(
                    MetaDiagnosticCode::ProviderUnavailable,
                    "rendered nested attribute is unavailable",
                    Some("attributes"),
                )
            })?;
        drop(document);
        let mut attribute_xml = xml[range.clone()].to_string();
        apply_typed_element_fields(&mut attribute_xml, attribute);
        xml.replace_range(range, &attribute_xml);
    }
    Ok(())
}

fn validate_typed_element_value_profile(
    element: &MetaElementDefinition,
) -> Result<(), MetaDiagnostic> {
    let Some(metadata_type) = element.r#type.as_ref() else {
        return if element.fill_value.is_some() {
            Err(typed_diagnostic(
                MetaDiagnosticCode::InvalidArguments,
                "fillValue requires an explicit metadata type",
                Some("fillValue"),
            ))
        } else {
            Ok(())
        };
    };
    for (index, variant) in metadata_type.variants.iter().enumerate() {
        match variant {
            MetadataTypeVariant::Reference { metadata_path } => {
                let kind = metadata_path.segments().next().unwrap_or_default();
                let supports_ref = metadata_generated_types_8_3_27(kind)
                    .is_some_and(|types| types.iter().any(|(_, category)| *category == "Ref"));
                if !supports_ref {
                    return Err(typed_diagnostic(
                        MetaDiagnosticCode::InvalidArguments,
                        format!("metadata kind `{kind}` does not define a reference type"),
                        Some(&format!("type.variants[{index}].metadataPath")),
                    ));
                }
            }
            MetadataTypeVariant::DefinedType { metadata_path }
                if metadata_path.segments().next() != Some("DefinedType") =>
            {
                return Err(typed_diagnostic(
                    MetaDiagnosticCode::InvalidArguments,
                    "defined-type variant must target DefinedType",
                    Some(&format!("type.variants[{index}].metadataPath")),
                ));
            }
            _ => {}
        }
    }
    let Some(fill_value) = element.fill_value.as_ref() else {
        return Ok(());
    };
    let compatible = match fill_value {
        MetaFillValue::String(value) => metadata_type.variants.iter().any(|variant| {
            matches!(
                variant,
                MetadataTypeVariant::String { length, .. }
                    if *length == 0 || value.chars().count() <= *length as usize
            )
        }),
        MetaFillValue::Number(value) => metadata_type.variants.iter().any(|variant| {
            let MetadataTypeVariant::Number {
                digits,
                fraction,
                sign,
            } = variant
            else {
                return false;
            };
            typed_decimal_shape(value).is_some_and(|(negative, integer_digits, fraction_digits)| {
                (*sign == NumberSign::Any || !negative)
                    && integer_digits + fraction_digits <= *digits as usize
                    && fraction_digits <= *fraction as usize
            })
        }),
        MetaFillValue::Boolean(_) => metadata_type
            .variants
            .iter()
            .any(|variant| matches!(variant, MetadataTypeVariant::Boolean)),
        MetaFillValue::DateTime(value) => {
            typed_xs_datetime_is_valid(value)
                && metadata_type
                    .variants
                    .iter()
                    .any(|variant| matches!(variant, MetadataTypeVariant::Date { .. }))
        }
        MetaFillValue::Reference(reference) => metadata_type.variants.iter().any(|variant| {
            matches!(
                variant,
                MetadataTypeVariant::Reference { metadata_path }
                    if metadata_path == &reference.metadata_path
            )
        }),
    };
    if compatible {
        Ok(())
    } else {
        Err(typed_diagnostic(
            MetaDiagnosticCode::InvalidArguments,
            "fillValue is not lexically valid and compatible with the metadata type",
            Some("fillValue"),
        ))
    }
}

fn typed_decimal_shape(value: &str) -> Option<(bool, usize, usize)> {
    let (negative, unsigned) = match value.as_bytes().first() {
        Some(b'-') => (true, &value[1..]),
        Some(b'+') => (false, &value[1..]),
        _ => (false, value),
    };
    let (integer, fraction) = unsigned
        .split_once('.')
        .map_or((unsigned, ""), |parts| parts);
    if integer.is_empty()
        || !integer.bytes().all(|byte| byte.is_ascii_digit())
        || !fraction.bytes().all(|byte| byte.is_ascii_digit())
        || (unsigned.contains('.') && fraction.is_empty())
    {
        return None;
    }
    Some((
        negative,
        integer.trim_start_matches('0').len().max(1),
        fraction.len(),
    ))
}

fn typed_xs_datetime_is_valid(value: &str) -> bool {
    let core = if let Some(core) = value.strip_suffix('Z') {
        core
    } else if value.len() >= 6 {
        let split = value.len() - 6;
        let timezone = &value[split..];
        if matches!(timezone.as_bytes().first(), Some(b'+') | Some(b'-'))
            && timezone.as_bytes().get(3) == Some(&b':')
            && timezone[1..3].bytes().all(|byte| byte.is_ascii_digit())
            && timezone[4..].bytes().all(|byte| byte.is_ascii_digit())
            && timezone[1..3].parse::<u8>().is_ok_and(|hour| hour <= 14)
            && timezone[4..].parse::<u8>().is_ok_and(|minute| minute <= 59)
        {
            &value[..split]
        } else {
            value
        }
    } else {
        value
    };
    let Some((date, time)) = core.split_once('T') else {
        return false;
    };
    let mut date_parts = date.split('-');
    let (Some(year), Some(month), Some(day), None) = (
        date_parts.next(),
        date_parts.next(),
        date_parts.next(),
        date_parts.next(),
    ) else {
        return false;
    };
    let (Ok(year), Ok(month), Ok(day)) =
        (year.parse::<i32>(), month.parse::<u8>(), day.parse::<u8>())
    else {
        return false;
    };
    let max_day = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if year % 400 == 0 || (year % 4 == 0 && year % 100 != 0) => 29,
        2 => 28,
        _ => return false,
    };
    if day == 0 || day > max_day {
        return false;
    }
    let mut time_parts = time.split(':');
    let (Some(hour), Some(minute), Some(second), None) = (
        time_parts.next(),
        time_parts.next(),
        time_parts.next(),
        time_parts.next(),
    ) else {
        return false;
    };
    let (second, fraction) = second
        .split_once('.')
        .map_or((second, None), |(second, fraction)| {
            (second, Some(fraction))
        });
    hour.parse::<u8>().is_ok_and(|hour| hour <= 23)
        && minute.parse::<u8>().is_ok_and(|minute| minute <= 59)
        && second.parse::<u8>().is_ok_and(|second| second <= 59)
        && fraction.is_none_or(|fraction| {
            !fraction.is_empty() && fraction.bytes().all(|byte| byte.is_ascii_digit())
        })
}

fn apply_typed_element_fields(xml: &mut String, element: &MetaElementDefinition) {
    let Some(range) = typed_properties_text_range(xml) else {
        return;
    };
    let mut text = xml[range.clone()].to_string();
    let indent = meta_edit_property_child_indent(&text);
    if let Some(synonym) = &element.synonym {
        let mut lines = Vec::new();
        emit_meta_mltext(&mut lines, &indent, "Synonym", synonym);
        let _ =
            meta_edit_replace_or_insert_property(&mut text, "Synonym", &lines.join("\n"), &indent);
    }
    if let Some(comment) = &element.comment {
        let replacement = if comment.is_empty() {
            format!("{indent}<Comment/>")
        } else {
            format!("{indent}<Comment>{}</Comment>", escape_xml(comment))
        };
        let _ = meta_edit_replace_or_insert_property(&mut text, "Comment", &replacement, &indent);
    }
    if let Some(metadata_type) = &element.r#type {
        let mut lines = Vec::new();
        emit_meta_typed_value_type(&mut lines, &indent, metadata_type);
        let _ = meta_edit_replace_or_insert_property(&mut text, "Type", &lines.join("\n"), &indent);
    }
    if element.fill_value.is_some() {
        let mut lines = Vec::new();
        emit_meta_typed_fill_value(&mut lines, &indent, element.fill_value.as_ref());
        let _ = meta_edit_replace_or_insert_property(
            &mut text,
            "FillValue",
            &lines.join("\n"),
            &indent,
        );
    }
    if let Some(required) = element.required {
        let replacement = format!(
            "{indent}<FillChecking>{}</FillChecking>",
            if required { "ShowError" } else { "DontCheck" }
        );
        let _ =
            meta_edit_replace_or_insert_property(&mut text, "FillChecking", &replacement, &indent);
    }
    xml.replace_range(range, &text);
}

fn typed_properties_text_range(text: &str) -> Option<std::ops::Range<usize>> {
    let start = text.find("<Properties>")?;
    let close = "</Properties>";
    let relative_end = text[start..].find(close)?;
    Some(start..start + relative_end + close.len())
}

fn find_typed_element_range(
    xml_text: &str,
    tag: &str,
    scope: Option<&str>,
    name: &str,
) -> Result<std::ops::Range<usize>, MetaDiagnostic> {
    let doc = Document::parse(xml_text.trim_start_matches('\u{feff}')).map_err(|_| {
        typed_diagnostic(
            MetaDiagnosticCode::ProviderUnavailable,
            "metadata descriptor is not valid XML",
            None,
        )
    })?;
    let object = meta_edit_object_node(&doc).map_err(|_| {
        typed_diagnostic(
            MetaDiagnosticCode::ProviderUnavailable,
            "metadata object is unavailable",
            None,
        )
    })?;
    let parent = if let Some(section_name) = scope {
        let section = meta_edit_find_tabular_section(object, section_name).ok_or_else(|| {
            typed_diagnostic(
                MetaDiagnosticCode::TargetNotFound,
                "tabular section scope was not found",
                Some("scope.tabularSection"),
            )
        })?;
        meta_info_child(section, "ChildObjects")
    } else {
        meta_info_child(object, "ChildObjects")
    }
    .ok_or_else(|| {
        typed_diagnostic(
            MetaDiagnosticCode::TargetNotFound,
            "metadata collection is empty",
            Some("name"),
        )
    })?;
    meta_info_children(parent, tag)
        .into_iter()
        .find(|child| meta_edit_child_object_name(*child).as_deref() == Some(name))
        .map(|node| node.range())
        .ok_or_else(|| {
            typed_diagnostic(
                MetaDiagnosticCode::TargetNotFound,
                format!("element `{name}` was not found"),
                Some("name"),
            )
        })
}

fn update_typed_element(
    xml_text: &mut String,
    collection: MetaCollection,
    scope: Option<&str>,
    update: &MetaElementUpdate,
) -> Result<(), MetaDiagnostic> {
    let tag = collection_tag(collection);
    let range = find_typed_element_range(xml_text, tag, scope, &update.name)?;
    if let Some(new_name) = update
        .new_name
        .as_deref()
        .filter(|name| *name != update.name)
    {
        ensure_typed_name_free(xml_text, tag, scope, new_name)?;
    }
    let node_xml = xml_text[range.clone()].to_string();
    let properties_range = typed_properties_text_range(&node_xml).ok_or_else(|| {
        typed_diagnostic(
            MetaDiagnosticCode::ProviderUnavailable,
            "metadata element has no Properties",
            None,
        )
    })?;
    let mut properties_text = node_xml[properties_range.clone()].to_string();
    if let Some(metadata_type) = &update.r#type {
        let fill_value = match &update.fill_value {
            Some(fill_value) => Some(fill_value.clone()),
            None => parse_typed_fill_value(&properties_text)?,
        };
        let candidate = MetaElementDefinition {
            name: update
                .new_name
                .clone()
                .unwrap_or_else(|| update.name.clone()),
            synonym: None,
            comment: None,
            r#type: Some(metadata_type.clone()),
            required: None,
            fill_value,
            attributes: Vec::new(),
            position: None,
        };
        validate_typed_element_value_profile(&candidate).map_err(|mut diagnostic| {
            if update.fill_value.is_none() && diagnostic.field.as_deref() == Some("fillValue") {
                diagnostic.field = Some("type".to_string());
            }
            diagnostic
        })?;
    } else if let Some(fill_value) = &update.fill_value {
        let metadata_type = parse_typed_metadata_type(&properties_text)?;
        let candidate = MetaElementDefinition {
            name: update.name.clone(),
            synonym: None,
            comment: None,
            r#type: Some(metadata_type),
            required: None,
            fill_value: Some(fill_value.clone()),
            attributes: Vec::new(),
            position: None,
        };
        validate_typed_element_value_profile(&candidate)?;
    }
    let indent = meta_edit_property_child_indent(&properties_text);
    if let Some(new_name) = &update.new_name {
        let replacement = format!("{indent}<Name>{}</Name>", escape_xml(new_name));
        meta_edit_replace_or_insert_property(&mut properties_text, "Name", &replacement, &indent)
            .map_err(|_| {
            typed_diagnostic(
                MetaDiagnosticCode::ProviderUnavailable,
                "metadata element name could not be updated",
                Some("newName"),
            )
        })?;
    }
    if let Some(synonym) = &update.synonym {
        let mut lines = Vec::new();
        emit_meta_mltext(&mut lines, &indent, "Synonym", synonym);
        meta_edit_replace_or_insert_property(
            &mut properties_text,
            "Synonym",
            &lines.join("\n"),
            &indent,
        )
        .map_err(|_| {
            typed_diagnostic(
                MetaDiagnosticCode::ProviderUnavailable,
                "metadata synonym could not be updated",
                Some("synonym"),
            )
        })?;
    }
    if let Some(comment) = &update.comment {
        let replacement = if comment.is_empty() {
            format!("{indent}<Comment/>")
        } else {
            format!("{indent}<Comment>{}</Comment>", escape_xml(comment))
        };
        meta_edit_replace_or_insert_property(
            &mut properties_text,
            "Comment",
            &replacement,
            &indent,
        )
        .map_err(|_| {
            typed_diagnostic(
                MetaDiagnosticCode::ProviderUnavailable,
                "metadata comment could not be updated",
                Some("comment"),
            )
        })?;
    }
    if let Some(metadata_type) = &update.r#type {
        let mut lines = Vec::new();
        emit_meta_typed_value_type(&mut lines, &indent, metadata_type);
        meta_edit_replace_or_insert_property(
            &mut properties_text,
            "Type",
            &lines.join("\n"),
            &indent,
        )
        .map_err(|_| {
            typed_diagnostic(
                MetaDiagnosticCode::ProviderUnavailable,
                "metadata type could not be updated",
                Some("type"),
            )
        })?;
    }
    if let Some(fill_value) = &update.fill_value {
        let mut lines = Vec::new();
        emit_meta_typed_fill_value(&mut lines, &indent, Some(fill_value));
        meta_edit_replace_or_insert_property(
            &mut properties_text,
            "FillValue",
            &lines.join("\n"),
            &indent,
        )
        .map_err(|_| {
            typed_diagnostic(
                MetaDiagnosticCode::ProviderUnavailable,
                "metadata fill value could not be updated",
                Some("fillValue"),
            )
        })?;
    }
    if let Some(required) = update.required {
        let replacement = format!(
            "{indent}<FillChecking>{}</FillChecking>",
            if required { "ShowError" } else { "DontCheck" }
        );
        meta_edit_replace_or_insert_property(
            &mut properties_text,
            "FillChecking",
            &replacement,
            &indent,
        )
        .map_err(|_| {
            typed_diagnostic(
                MetaDiagnosticCode::ProviderUnavailable,
                "metadata required flag could not be updated",
                Some("required"),
            )
        })?;
    }
    let mut updated_node = node_xml;
    updated_node.replace_range(properties_range, &properties_text);
    xml_text.replace_range(range, &updated_node);
    if let Some(position) = &update.position {
        let current_name = update.new_name.as_deref().unwrap_or(&update.name);
        let range = find_typed_element_range(xml_text, tag, scope, current_name)?;
        let node = xml_text[range.clone()].to_string();
        meta_edit_remove_xml_node_range(xml_text, range);
        let lines = node.lines().map(ToOwned::to_owned).collect::<Vec<_>>();
        let position = typed_insert_position(Some(position));
        let result = match scope {
            Some(section) => meta_edit_insert_tabular_child_object_with_position(
                xml_text, section, tag, &position, &lines,
            ),
            None => {
                meta_edit_insert_top_child_object_with_position(xml_text, tag, &position, &lines)
            }
        };
        result.map_err(|_| {
            typed_diagnostic(
                MetaDiagnosticCode::TargetNotFound,
                "metadata position anchor was not found",
                Some("position"),
            )
        })?;
    }
    Ok(())
}

fn parse_typed_fill_value(properties_text: &str) -> Result<Option<MetaFillValue>, MetaDiagnostic> {
    const WRAPPER_START: &str = r#"<Root xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:xr="http://v8.1c.ru/8.3/xcf/readable" xmlns:v8="http://v8.1c.ru/8.1/data/core" xmlns:cfg="http://v8.1c.ru/8.1/data/enterprise/current-config">"#;
    let wrapped = format!("{WRAPPER_START}{properties_text}</Root>");
    let document = Document::parse(&wrapped).map_err(|_| {
        typed_diagnostic(
            MetaDiagnosticCode::ProviderUnavailable,
            "metadata element properties are not valid XML",
            Some("fillValue"),
        )
    })?;
    let Some(fill) = document
        .descendants()
        .find(|node| node.is_element() && node.tag_name().name() == "FillValue")
    else {
        return Ok(None);
    };
    if fill
        .attributes()
        .any(|attribute| attribute.name() == "nil" && attribute.value() == "true")
    {
        return Ok(None);
    }
    let value = fill.text().unwrap_or_default().to_string();
    let value_type = fill
        .attributes()
        .find(|attribute| attribute.name() == "type")
        .map(|attribute| attribute.value())
        .unwrap_or_default();
    match value_type {
        "xs:string" => Ok(Some(MetaFillValue::String(value))),
        "xs:decimal" => Ok(Some(MetaFillValue::Number(value))),
        "xs:boolean" => match value.as_str() {
            "true" => Ok(Some(MetaFillValue::Boolean(true))),
            "false" => Ok(Some(MetaFillValue::Boolean(false))),
            _ => Err(typed_diagnostic(
                MetaDiagnosticCode::ValidationFailed,
                "existing boolean fill value is not canonical",
                Some("fillValue"),
            )),
        },
        "xs:dateTime" => Ok(Some(MetaFillValue::DateTime(value))),
        "xr:DesignTimeRef" => Ok(Some(MetaFillValue::Reference(
            crate::domain::metadata::MetadataReference {
                metadata_path: MetadataAddress::parse(PLATFORM_XML_8_3_27_FORMAT_2_20, &value)
                    .map_err(|_| {
                        typed_diagnostic(
                            MetaDiagnosticCode::ValidationFailed,
                            "existing reference fill value is not a metadata address",
                            Some("fillValue"),
                        )
                    })?,
            },
        ))),
        "" if value.is_empty() => Ok(None),
        _ => Err(typed_diagnostic(
            MetaDiagnosticCode::ValidationFailed,
            "existing fill value type is unsupported by typed metadata edit",
            Some("fillValue"),
        )),
    }
}

fn parse_typed_metadata_type(properties_text: &str) -> Result<MetadataType, MetaDiagnostic> {
    const WRAPPER_START: &str = r#"<Root xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:xr="http://v8.1c.ru/8.3/xcf/readable" xmlns:v8="http://v8.1c.ru/8.1/data/core" xmlns:cfg="http://v8.1c.ru/8.1/data/enterprise/current-config">"#;
    let wrapped = format!("{WRAPPER_START}{properties_text}</Root>");
    let document = Document::parse(&wrapped).map_err(|_| {
        typed_diagnostic(
            MetaDiagnosticCode::ProviderUnavailable,
            "metadata element properties are not valid XML",
            Some("type"),
        )
    })?;
    let text = |name: &str| {
        document
            .descendants()
            .find(|node| node.is_element() && node.tag_name().name() == name)
            .and_then(|node| node.text())
            .unwrap_or_default()
    };
    let mut variants = Vec::new();
    for node in document.descendants().filter(|node| {
        node.is_element()
            && node.tag_name().namespace() == Some("http://v8.1c.ru/8.1/data/core")
            && matches!(node.tag_name().name(), "Type" | "TypeSet")
    }) {
        let value = node.text().unwrap_or_default();
        let variant = if node.tag_name().name() == "TypeSet" {
            let raw = value.strip_prefix("cfg:").unwrap_or(value);
            MetadataTypeVariant::DefinedType {
                metadata_path: MetadataAddress::parse(PLATFORM_XML_8_3_27_FORMAT_2_20, raw)
                    .map_err(|_| {
                        typed_diagnostic(
                            MetaDiagnosticCode::ValidationFailed,
                            "existing defined type is not a metadata address",
                            Some("type"),
                        )
                    })?,
            }
        } else {
            match value {
                "xs:string" => MetadataTypeVariant::String {
                    length: text("Length").parse().unwrap_or(0),
                    allowed_length: if text("AllowedLength") == "Fixed" {
                        StringLengthMode::Fixed
                    } else {
                        StringLengthMode::Variable
                    },
                },
                "xs:decimal" => MetadataTypeVariant::Number {
                    digits: text("Digits").parse().unwrap_or(0),
                    fraction: text("FractionDigits").parse().unwrap_or(0),
                    sign: if text("AllowedSign") == "Nonnegative" {
                        NumberSign::NonNegative
                    } else {
                        NumberSign::Any
                    },
                },
                "xs:boolean" => MetadataTypeVariant::Boolean,
                "xs:dateTime" => MetadataTypeVariant::Date {
                    fractions: match text("DateFractions") {
                        "Date" => DateFractions::Date,
                        "Time" => DateFractions::Time,
                        _ => DateFractions::DateTime,
                    },
                },
                "v8:ValueStorage" => MetadataTypeVariant::ValueStorage,
                raw if raw.starts_with("cfg:") => {
                    let raw = raw.trim_start_matches("cfg:");
                    let Some((generated, name)) = raw.split_once('.') else {
                        return Err(typed_diagnostic(
                            MetaDiagnosticCode::ValidationFailed,
                            "existing reference type is malformed",
                            Some("type"),
                        ));
                    };
                    let kind = generated.strip_suffix("Ref").ok_or_else(|| {
                        typed_diagnostic(
                            MetaDiagnosticCode::ValidationFailed,
                            "existing generated type is not a reference",
                            Some("type"),
                        )
                    })?;
                    MetadataTypeVariant::Reference {
                        metadata_path: MetadataAddress::parse(
                            PLATFORM_XML_8_3_27_FORMAT_2_20,
                            &format!("{kind}.{name}"),
                        )
                        .map_err(|_| {
                            typed_diagnostic(
                                MetaDiagnosticCode::ValidationFailed,
                                "existing reference type is not a metadata address",
                                Some("type"),
                            )
                        })?,
                    }
                }
                _ => {
                    return Err(typed_diagnostic(
                        MetaDiagnosticCode::ValidationFailed,
                        "existing metadata type is unsupported by typed edit",
                        Some("type"),
                    ))
                }
            }
        };
        variants.push(variant);
    }
    MetadataType::new(variants).map_err(|mut diagnostic| {
        diagnostic.field = Some("type".to_string());
        diagnostic
    })
}

fn remove_typed_element(
    xml_text: &mut String,
    collection: MetaCollection,
    scope: Option<&str>,
    name: &str,
) -> Result<(), MetaDiagnostic> {
    let tag = collection_tag(collection);
    let range = find_typed_element_range(xml_text, tag, scope, name)?;
    meta_edit_remove_xml_node_range(xml_text, range);
    Ok(())
}

fn apply_typed_relations(
    xml_text: &mut String,
    object_kind: &str,
    relation: MetaRelation,
    mode: RelationEditMode,
    targets: &[crate::domain::metadata::MetaRelationTarget],
) -> Result<(), MetaDiagnostic> {
    if relation == MetaRelation::RegisterRecords && object_kind != "Document" {
        return Err(typed_diagnostic(
            MetaDiagnosticCode::UnsupportedKind,
            "registerRecords is supported for Document only",
            Some("relation"),
        ));
    }
    let tag = match relation {
        MetaRelation::Owners => "Owners",
        MetaRelation::RegisterRecords => "RegisterRecords",
        MetaRelation::BasedOn => "BasedOn",
        MetaRelation::InputByString => "InputByString",
    };
    let item_tag = if relation == MetaRelation::InputByString {
        "xr:Field"
    } else {
        "xr:Item"
    };
    let doc = Document::parse(xml_text.trim_start_matches('\u{feff}')).map_err(|_| {
        typed_diagnostic(
            MetaDiagnosticCode::ProviderUnavailable,
            "metadata descriptor is not valid XML",
            None,
        )
    })?;
    let object = meta_edit_object_node(&doc).map_err(|_| {
        typed_diagnostic(
            MetaDiagnosticCode::ProviderUnavailable,
            "metadata object is unavailable",
            None,
        )
    })?;
    let properties = meta_info_child(object, "Properties").ok_or_else(|| {
        typed_diagnostic(
            MetaDiagnosticCode::ProviderUnavailable,
            "metadata descriptor has no Properties",
            None,
        )
    })?;
    let relation_node = meta_info_child(properties, tag).ok_or_else(|| {
        typed_diagnostic(
            MetaDiagnosticCode::InvalidArguments,
            format!(
                "relation `{}` is not present on this metadata object",
                relation.as_str()
            ),
            Some("relation"),
        )
    })?;
    let mut values = relation_node
        .children()
        .filter(|child| child.is_element())
        .filter_map(|child| child.text())
        .map(ToOwned::to_owned)
        .collect::<Vec<_>>();
    let range = properties.range();
    drop(doc);
    for (index, target) in targets.iter().enumerate() {
        let valid_shape = matches!(
            (relation, target),
            (
                MetaRelation::InputByString,
                crate::domain::metadata::MetaRelationTarget::Field(_)
            ) | (
                MetaRelation::Owners | MetaRelation::RegisterRecords | MetaRelation::BasedOn,
                crate::domain::metadata::MetaRelationTarget::Object(_)
            )
        );
        if !valid_shape {
            return Err(typed_diagnostic(
                MetaDiagnosticCode::InvalidArguments,
                "relation target has the wrong typed shape",
                Some(&format!("targets[{index}]")),
            ));
        }
    }
    let requested = targets
        .iter()
        .map(|target| target.wire_value().to_string())
        .collect::<Vec<_>>();
    match mode {
        RelationEditMode::Add => {
            for (target_index, target) in requested.into_iter().enumerate() {
                if values.contains(&target) {
                    return Err(typed_diagnostic(
                        MetaDiagnosticCode::AlreadyExists,
                        "metadata relation target already exists",
                        Some(&format!("targets[{target_index}]")),
                    ));
                }
                values.push(target);
            }
        }
        RelationEditMode::Remove => {
            for (target_index, target) in requested.into_iter().enumerate() {
                let Some(index) = values.iter().position(|value| value == &target) else {
                    return Err(typed_diagnostic(
                        MetaDiagnosticCode::TargetNotFound,
                        "metadata relation target was not found",
                        Some(&format!("targets[{target_index}]")),
                    ));
                };
                values.remove(index);
            }
        }
        RelationEditMode::Replace => {
            let mut unique = std::collections::HashSet::new();
            for (target_index, target) in requested.iter().enumerate() {
                if !unique.insert(target) {
                    return Err(typed_diagnostic(
                        MetaDiagnosticCode::InvalidArguments,
                        "metadata relation targets contain duplicates",
                        Some(&format!("targets[{target_index}]")),
                    ));
                }
            }
            values = requested;
        }
    }
    let mut properties_text = xml_text[range.clone()].to_string();
    let indent = meta_edit_property_child_indent(&properties_text);
    let replacement = if values.is_empty() {
        format!("{indent}<{tag}/>")
    } else {
        let items = values
            .iter()
            .map(|value| {
                if relation == MetaRelation::InputByString {
                    format!("{indent}\t<{item_tag}>{}</{item_tag}>", escape_xml(value))
                } else {
                    format!(
                        "{indent}\t<{item_tag} xsi:type=\"xr:MDObjectRef\">{}</{item_tag}>",
                        escape_xml(value)
                    )
                }
            })
            .collect::<Vec<_>>()
            .join("\n");
        format!("{indent}<{tag}>\n{items}\n{indent}</{tag}>")
    };
    meta_edit_replace_or_insert_property(&mut properties_text, tag, &replacement, &indent)
        .map_err(|_| {
            typed_diagnostic(
                MetaDiagnosticCode::ProviderUnavailable,
                "metadata relation could not be updated",
                Some("relation"),
            )
        })?;
    xml_text.replace_range(range, &properties_text);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::metadata::{
        DateFractions, MetaCollection, MetaEditOperation, MetaElementInput, MetaElementUpdateInput,
        MetaFillValue, MetaPosition, MetaPropertyChanges, MetaPropertyInput, MetaPropertyValue,
        MetaRelation, MetaScope, MetadataReference, MetadataType, MetadataTypeVariant, NumberSign,
        RelationEditMode, StringLengthMode,
    };
    use crate::domain::source_target::{MetadataAddress, PLATFORM_XML_8_3_27_FORMAT_2_20};

    fn object_xml(kind: &str, name: &str, properties: &str) -> String {
        format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" xmlns:app="http://v8.1c.ru/8.2/managed-application/core" xmlns:cfg="http://v8.1c.ru/8.1/data/enterprise/current-config" xmlns:v8="http://v8.1c.ru/8.1/data/core" xmlns:xr="http://v8.1c.ru/8.3/xcf/readable" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" version="2.20">
	<{kind} uuid="11111111-1111-4111-8111-111111111111">
		<Properties><Name>{name}</Name><Synonym/><Comment/>{properties}</Properties>
		<ChildObjects/>
	</{kind}>
</MetaDataObject>
"#
        )
    }

    fn metadata_reference(path: &str) -> MetadataReference {
        MetadataReference {
            metadata_path: MetadataAddress::parse(PLATFORM_XML_8_3_27_FORMAT_2_20, path).unwrap(),
        }
    }

    #[test]
    fn typed_set_properties_and_relations_apply_without_legacy_dsl() {
        let mut xml = object_xml(
            "Document",
            "Order",
            "<NumberLength>9</NumberLength><CheckUnique>true</CheckUnique><RegisterRecords/><BasedOn/><InputByString/>",
        );
        let operations = vec![
            MetaEditOperation::SetProperties {
                values: MetaPropertyChanges::convert(
                    crate::domain::metadata::MetadataKind::Document,
                    vec![
                        MetaPropertyInput::new(
                            "Synonym",
                            MetaPropertyValue::String("Customer order".into()),
                        ),
                        MetaPropertyInput::new(
                            "NumberLength",
                            MetaPropertyValue::UnsignedInteger(12),
                        ),
                        MetaPropertyInput::new("CheckUnique", MetaPropertyValue::Boolean(false)),
                    ],
                )
                .unwrap(),
            },
            MetaEditOperation::edit_relations(
                MetaRelation::RegisterRecords,
                RelationEditMode::Replace,
                vec![metadata_reference("InformationRegister.OrderFacts")],
            )
            .unwrap(),
            MetaEditOperation::edit_relations(
                MetaRelation::BasedOn,
                RelationEditMode::Add,
                vec![metadata_reference("Document.Quote")],
            )
            .unwrap(),
            MetaEditOperation::edit_relation_targets(
                MetaRelation::InputByString,
                RelationEditMode::Replace,
                vec![crate::domain::metadata::MetaRelationTarget::Field(
                    crate::domain::metadata::MetadataFieldPath::parse(
                        "Document.Order.StandardAttribute.Number",
                    )
                    .unwrap(),
                )],
            )
            .unwrap(),
        ];

        apply_typed_operations(&mut xml, &operations).unwrap();

        assert!(xml.contains("<v8:content>Customer order</v8:content>"));
        assert!(xml.contains("<NumberLength>12</NumberLength>"));
        assert!(xml.contains("<CheckUnique>false</CheckUnique>"));
        assert!(xml.contains(
            "<xr:Item xsi:type=\"xr:MDObjectRef\">InformationRegister.OrderFacts</xr:Item>"
        ));
        assert!(xml.contains("<xr:Item xsi:type=\"xr:MDObjectRef\">Document.Quote</xr:Item>"));
        assert!(xml.contains("<xr:Field>Document.Order.StandardAttribute.Number</xr:Field>"));
    }

    #[test]
    fn typed_collection_matrix_adds_updates_and_removes_every_supported_collection() {
        let cases = [
            (MetaCollection::Attributes, "Document", "Attribute"),
            (
                MetaCollection::TabularSections,
                "Document",
                "TabularSection",
            ),
            (
                MetaCollection::Dimensions,
                "InformationRegister",
                "Dimension",
            ),
            (MetaCollection::Resources, "InformationRegister", "Resource"),
            (MetaCollection::EnumValues, "Enum", "EnumValue"),
            (MetaCollection::Columns, "DocumentJournal", "Column"),
            (MetaCollection::Forms, "Document", "Form"),
            (MetaCollection::Templates, "Document", "Template"),
            (MetaCollection::Commands, "Document", "Command"),
        ];

        for (collection, kind, tag) in cases {
            let mut xml = object_xml(kind, "Sample", "");
            let mut added = MetaElementInput::named("First");
            if matches!(
                collection,
                MetaCollection::Attributes
                    | MetaCollection::Dimensions
                    | MetaCollection::Resources
                    | MetaCollection::Columns
            ) {
                added.r#type = Some(
                    MetadataType::new(vec![MetadataTypeVariant::String {
                        length: 20,
                        allowed_length: StringLengthMode::Variable,
                    }])
                    .unwrap(),
                );
            }
            let operations = vec![
                MetaEditOperation::add(collection, None, vec![added]).unwrap(),
                MetaEditOperation::update(
                    collection,
                    None,
                    vec![MetaElementUpdateInput {
                        name: "First".into(),
                        new_name: Some("Second".into()),
                        ..MetaElementUpdateInput::default()
                    }],
                )
                .unwrap(),
                MetaEditOperation::remove(collection, None, vec!["Second".into()]).unwrap(),
            ];

            apply_typed_operations(&mut xml, &operations).unwrap();
            assert!(!xml.contains(&format!("<{tag} uuid=")), "{collection:?}");
        }
    }

    #[test]
    fn typed_operations_are_ordered_support_scoped_attributes_and_emit_structured_types() {
        let mut xml = object_xml("Document", "Order", "<RegisterRecords/>");
        let structured_types = vec![
            MetadataTypeVariant::String {
                length: 40,
                allowed_length: StringLengthMode::Fixed,
            },
            MetadataTypeVariant::Number {
                digits: 15,
                fraction: 3,
                sign: NumberSign::NonNegative,
            },
            MetadataTypeVariant::Boolean,
            MetadataTypeVariant::Date {
                fractions: DateFractions::DateTime,
            },
            MetadataTypeVariant::Reference {
                metadata_path: metadata_reference("Catalog.Items").metadata_path,
            },
            MetadataTypeVariant::DefinedType {
                metadata_path: metadata_reference("DefinedType.ExternalCode").metadata_path,
            },
        ];
        let operations = vec![
            MetaEditOperation::add(
                MetaCollection::TabularSections,
                None,
                vec![MetaElementInput::named("Lines")],
            )
            .unwrap(),
            MetaEditOperation::add(
                MetaCollection::Attributes,
                Some(MetaScope {
                    tabular_section: "Lines".into(),
                }),
                vec![MetaElementInput {
                    name: "Value".into(),
                    r#type: Some(MetadataType::new(structured_types).unwrap()),
                    required: Some(true),
                    ..MetaElementInput::default()
                }],
            )
            .unwrap(),
        ];

        apply_typed_operations(&mut xml, &operations).unwrap();

        assert!(xml.contains("<v8:Type>xs:string</v8:Type>"));
        assert!(xml.contains("<v8:AllowedLength>Fixed</v8:AllowedLength>"));
        assert!(xml.contains("<v8:Type>xs:decimal</v8:Type>"));
        assert!(xml.contains("<v8:Type>xs:boolean</v8:Type>"));
        assert!(xml.contains("<v8:Type>xs:dateTime</v8:Type>"));
        assert!(xml.contains("<v8:Type>cfg:CatalogRef.Items</v8:Type>"));
        assert!(xml.contains("<v8:TypeSet>cfg:DefinedType.ExternalCode</v8:TypeSet>"));
        assert!(xml.contains("<FillChecking>ShowError</FillChecking>"));
    }

    #[test]
    fn typed_positions_and_fill_values_preserve_requested_order_and_wire_types() {
        let mut xml = object_xml("Document", "Order", "<RegisterRecords/>");
        let typed_attribute =
            |name: &str,
             variant: MetadataTypeVariant,
             fill_value: Option<MetaFillValue>,
             position: Option<MetaPosition>| MetaElementInput {
                name: name.into(),
                r#type: Some(MetadataType::new(vec![variant]).unwrap()),
                fill_value,
                position,
                ..MetaElementInput::default()
            };
        let operations = vec![
            MetaEditOperation::add(
                MetaCollection::Attributes,
                None,
                vec![
                    typed_attribute(
                        "StringValue",
                        MetadataTypeVariant::String {
                            length: 30,
                            allowed_length: StringLengthMode::Variable,
                        },
                        Some(MetaFillValue::String("A&B".into())),
                        None,
                    ),
                    typed_attribute(
                        "BooleanValue",
                        MetadataTypeVariant::Boolean,
                        Some(MetaFillValue::Boolean(true)),
                        Some(MetaPosition::new(Some("StringValue".into()), None).unwrap()),
                    ),
                    typed_attribute(
                        "NumberValue",
                        MetadataTypeVariant::Number {
                            digits: 10,
                            fraction: 2,
                            sign: NumberSign::Any,
                        },
                        Some(MetaFillValue::Number("12.50".into())),
                        Some(MetaPosition::new(None, Some("StringValue".into())).unwrap()),
                    ),
                    typed_attribute(
                        "DateValue",
                        MetadataTypeVariant::Date {
                            fractions: DateFractions::DateTime,
                        },
                        Some(MetaFillValue::DateTime("2026-08-03T10:11:12".into())),
                        None,
                    ),
                    typed_attribute(
                        "ReferenceValue",
                        MetadataTypeVariant::Reference {
                            metadata_path: metadata_reference("Catalog.Items").metadata_path,
                        },
                        Some(MetaFillValue::Reference(metadata_reference(
                            "Catalog.Items",
                        ))),
                        None,
                    ),
                    typed_attribute(
                        "StorageValue",
                        MetadataTypeVariant::ValueStorage,
                        None,
                        None,
                    ),
                ],
            )
            .unwrap(),
            MetaEditOperation::update(
                MetaCollection::Attributes,
                None,
                vec![MetaElementUpdateInput {
                    name: "DateValue".into(),
                    position: Some(MetaPosition::new(Some("BooleanValue".into()), None).unwrap()),
                    ..MetaElementUpdateInput::default()
                }],
            )
            .unwrap(),
        ];

        apply_typed_operations(&mut xml, &operations).unwrap();

        let boolean = xml.find("<Name>BooleanValue</Name>").unwrap();
        let date = xml.find("<Name>DateValue</Name>").unwrap();
        let string = xml.find("<Name>StringValue</Name>").unwrap();
        let number = xml.find("<Name>NumberValue</Name>").unwrap();
        assert!(
            date < boolean && boolean < string && string < number,
            "unexpected typed attribute order: date={date}, boolean={boolean}, string={string}, number={number}\n{xml}"
        );
        assert!(xml.contains("<FillValue xsi:type=\"xs:string\">A&amp;B</FillValue>"));
        assert!(xml.contains("<FillValue xsi:type=\"xs:boolean\">true</FillValue>"));
        assert!(xml.contains("<FillValue xsi:type=\"xs:decimal\">12.50</FillValue>"));
        assert!(xml.contains("<FillValue xsi:type=\"xs:dateTime\">2026-08-03T10:11:12</FillValue>"));
        assert!(xml.contains("<FillValue xsi:type=\"xr:DesignTimeRef\">Catalog.Items</FillValue>"));
        assert!(xml.contains("<v8:Type>v8:ValueStorage</v8:Type>"));
    }

    #[test]
    fn typed_relations_cover_all_relations_and_edit_modes() {
        let mut catalog = object_xml("Catalog", "Child", "<Owners/>");
        let owners = vec![
            MetaEditOperation::edit_relations(
                MetaRelation::Owners,
                RelationEditMode::Replace,
                vec![metadata_reference("Catalog.Parent")],
            )
            .unwrap(),
            MetaEditOperation::edit_relations(
                MetaRelation::Owners,
                RelationEditMode::Add,
                vec![metadata_reference("Catalog.SecondParent")],
            )
            .unwrap(),
            MetaEditOperation::edit_relations(
                MetaRelation::Owners,
                RelationEditMode::Remove,
                vec![metadata_reference("Catalog.Parent")],
            )
            .unwrap(),
        ];
        apply_typed_operations(&mut catalog, &owners).unwrap();
        assert!(!catalog.contains("Catalog.Parent</xr:Item>"));
        assert!(catalog.contains("Catalog.SecondParent</xr:Item>"));

        let mut document = object_xml(
            "Document",
            "Order",
            "<RegisterRecords/><BasedOn/><InputByString/>",
        );
        for (relation, target) in [
            (
                MetaRelation::RegisterRecords,
                "InformationRegister.OrderFacts",
            ),
            (MetaRelation::BasedOn, "Document.Quote"),
        ] {
            let operations = [
                MetaEditOperation::edit_relations(
                    relation,
                    RelationEditMode::Replace,
                    vec![metadata_reference(target)],
                )
                .unwrap(),
                MetaEditOperation::edit_relations(
                    relation,
                    RelationEditMode::Remove,
                    vec![metadata_reference(target)],
                )
                .unwrap(),
            ];
            apply_typed_operations(&mut document, &operations).unwrap();
        }
        let field = crate::domain::metadata::MetadataFieldPath::parse(
            "Document.Order.StandardAttribute.Number",
        )
        .unwrap();
        let input_operations = [
            MetaEditOperation::edit_relation_targets(
                MetaRelation::InputByString,
                RelationEditMode::Replace,
                vec![crate::domain::metadata::MetaRelationTarget::Field(
                    field.clone(),
                )],
            )
            .unwrap(),
            MetaEditOperation::edit_relation_targets(
                MetaRelation::InputByString,
                RelationEditMode::Remove,
                vec![crate::domain::metadata::MetaRelationTarget::Field(field)],
            )
            .unwrap(),
        ];
        apply_typed_operations(&mut document, &input_operations).unwrap();
        assert!(document.contains("<RegisterRecords/>"));
        assert!(document.contains("<BasedOn/>"));
        assert!(document.contains("<InputByString/>"));
    }

    #[test]
    fn typed_failures_are_indexed_and_never_upsert_missing_targets() {
        let mut xml = object_xml("Document", "Order", "<RegisterRecords/>");
        let original = xml.clone();
        let operations = vec![
            MetaEditOperation::add(
                MetaCollection::Attributes,
                None,
                vec![MetaElementInput::named("Created")],
            )
            .unwrap(),
            MetaEditOperation::update(
                MetaCollection::Attributes,
                None,
                vec![MetaElementUpdateInput {
                    name: "Missing".into(),
                    synonym: Some("must not upsert".into()),
                    ..MetaElementUpdateInput::default()
                }],
            )
            .unwrap(),
        ];

        let failure = apply_typed_operations(&mut xml, &operations).unwrap_err();

        assert_eq!(failure.diagnostics[0].operation_index, Some(1));
        assert_eq!(
            failure.diagnostics[0].code,
            crate::domain::metadata::MetaDiagnosticCode::TargetNotFound
        );
        assert_eq!(
            xml, original,
            "failed ordered edits must leave the caller image unchanged"
        );
    }

    #[test]
    fn typed_duplicate_rename_collision_and_invalid_scope_have_stable_diagnostics() {
        let mut duplicate_xml = object_xml("Document", "Order", "<RegisterRecords/>");
        let duplicates = [
            MetaEditOperation::add(
                MetaCollection::Attributes,
                None,
                vec![MetaElementInput::named("Same")],
            )
            .unwrap(),
            MetaEditOperation::add(
                MetaCollection::Attributes,
                None,
                vec![MetaElementInput::named("Same")],
            )
            .unwrap(),
        ];
        let failure = apply_typed_operations(&mut duplicate_xml, &duplicates).unwrap_err();
        assert_eq!(failure.diagnostics[0].operation_index, Some(1));
        assert_eq!(
            failure.diagnostics[0].code,
            MetaDiagnosticCode::AlreadyExists
        );

        let mut collision_xml = object_xml("Document", "Order", "<RegisterRecords/>");
        let operations = [
            MetaEditOperation::add(
                MetaCollection::Attributes,
                None,
                vec![
                    MetaElementInput::named("First"),
                    MetaElementInput::named("Second"),
                ],
            )
            .unwrap(),
            MetaEditOperation::update(
                MetaCollection::Attributes,
                None,
                vec![MetaElementUpdateInput {
                    name: "First".into(),
                    new_name: Some("Second".into()),
                    ..MetaElementUpdateInput::default()
                }],
            )
            .unwrap(),
        ];
        let failure = apply_typed_operations(&mut collision_xml, &operations).unwrap_err();
        assert_eq!(failure.diagnostics[0].operation_index, Some(1));
        assert_eq!(
            failure.diagnostics[0].code,
            MetaDiagnosticCode::AlreadyExists
        );

        let mut scope_xml = object_xml("Document", "Order", "<RegisterRecords/>");
        let invalid_scope = MetaEditOperation::add(
            MetaCollection::Attributes,
            Some(MetaScope {
                tabular_section: "Missing".into(),
            }),
            vec![MetaElementInput::named("Value")],
        )
        .unwrap();
        let failure = apply_typed_operations(&mut scope_xml, &[invalid_scope]).unwrap_err();
        assert_eq!(failure.diagnostics[0].operation_index, Some(0));
        assert_eq!(
            failure.diagnostics[0].code,
            MetaDiagnosticCode::TargetNotFound
        );
    }

    #[test]
    fn typed_invalid_property_is_rejected_before_xml_mutation() {
        let diagnostic = MetaPropertyChanges::convert(
            crate::domain::metadata::MetadataKind::Catalog,
            vec![MetaPropertyInput::new(
                "NumberLength",
                MetaPropertyValue::UnsignedInteger(10),
            )],
        )
        .unwrap_err();

        assert_eq!(diagnostic.code, MetaDiagnosticCode::UnsupportedKind);
        assert_eq!(diagnostic.field.as_deref(), Some("values.NumberLength"));
    }

    #[test]
    fn typed_kind_collection_matrix_is_exact_and_reports_the_full_operation_field() {
        use crate::domain::metadata::MetadataKind;

        let all = [
            MetaCollection::Attributes,
            MetaCollection::TabularSections,
            MetaCollection::Dimensions,
            MetaCollection::Resources,
            MetaCollection::EnumValues,
            MetaCollection::Columns,
            MetaCollection::Forms,
            MetaCollection::Templates,
            MetaCollection::Commands,
        ];
        let cases: &[(MetadataKind, &[MetaCollection])] = &[
            (
                MetadataKind::Catalog,
                &[all[0], all[1], all[6], all[7], all[8]],
            ),
            (
                MetadataKind::Document,
                &[all[0], all[1], all[6], all[7], all[8]],
            ),
            (MetadataKind::Enum, &[all[4], all[6], all[7], all[8]]),
            (MetadataKind::Constant, &[all[6]]),
            (
                MetadataKind::InformationRegister,
                &[all[0], all[2], all[3], all[6], all[7], all[8]],
            ),
            (
                MetadataKind::AccumulationRegister,
                &[all[0], all[2], all[3], all[6], all[7], all[8]],
            ),
            (
                MetadataKind::AccountingRegister,
                &[all[0], all[2], all[3], all[6], all[7], all[8]],
            ),
            (
                MetadataKind::CalculationRegister,
                &[all[0], all[2], all[3], all[6], all[7], all[8]],
            ),
            (
                MetadataKind::ChartOfAccounts,
                &[all[0], all[1], all[6], all[7], all[8]],
            ),
            (
                MetadataKind::ChartOfCharacteristicTypes,
                &[all[0], all[1], all[6], all[7], all[8]],
            ),
            (
                MetadataKind::ChartOfCalculationTypes,
                &[all[0], all[1], all[6], all[7], all[8]],
            ),
            (
                MetadataKind::BusinessProcess,
                &[all[0], all[1], all[6], all[7], all[8]],
            ),
            (
                MetadataKind::Task,
                &[all[0], all[1], all[6], all[7], all[8]],
            ),
            (
                MetadataKind::ExchangePlan,
                &[all[0], all[1], all[6], all[7], all[8]],
            ),
            (
                MetadataKind::DocumentJournal,
                &[all[5], all[6], all[7], all[8]],
            ),
            (
                MetadataKind::Report,
                &[all[0], all[1], all[6], all[7], all[8]],
            ),
            (
                MetadataKind::DataProcessor,
                &[all[0], all[1], all[6], all[7], all[8]],
            ),
            (MetadataKind::CommonModule, &[]),
            (MetadataKind::ScheduledJob, &[]),
            (MetadataKind::EventSubscription, &[]),
            (MetadataKind::HTTPService, &[]),
            (MetadataKind::WebService, &[]),
            (MetadataKind::DefinedType, &[]),
        ];

        assert_eq!(cases.len(), MetadataKind::ALL.len());
        for (kind, allowed) in cases {
            for collection in all {
                let mut xml = object_xml(kind.as_str(), "Sample", "");
                let operation = MetaEditOperation::add(
                    collection,
                    None,
                    vec![MetaElementInput::named("Child")],
                )
                .unwrap();
                let result = apply_typed_operations(&mut xml, &[operation]);
                if allowed.contains(&collection) {
                    assert!(result.is_ok(), "{kind:?}.{collection:?}: {result:?}");
                } else {
                    let failure = result.unwrap_err();
                    assert_eq!(failure.diagnostics[0].operation_index, Some(0));
                    assert_eq!(
                        failure.diagnostics[0].code,
                        MetaDiagnosticCode::UnsupportedKind,
                        "{kind:?}.{collection:?}"
                    );
                    assert_eq!(
                        failure.diagnostics[0].field.as_deref(),
                        Some("operations[0].collection"),
                        "{kind:?}.{collection:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn typed_inline_tabular_attributes_keep_fields_and_apply_nested_positions() {
        let mut xml = object_xml("Document", "Order", "<RegisterRecords/>");
        let operation = MetaEditOperation::add(
            MetaCollection::TabularSections,
            None,
            vec![MetaElementInput {
                name: "Lines".into(),
                attributes: Some(vec![
                    MetaElementInput::named("Second"),
                    MetaElementInput {
                        name: "First".into(),
                        comment: Some("typed nested field".into()),
                        r#type: Some(
                            MetadataType::new(vec![MetadataTypeVariant::Number {
                                digits: 12,
                                fraction: 2,
                                sign: NumberSign::NonNegative,
                            }])
                            .unwrap(),
                        ),
                        required: Some(true),
                        fill_value: Some(MetaFillValue::Number("3.50".into())),
                        position: Some(MetaPosition::new(Some("Second".into()), None).unwrap()),
                        ..MetaElementInput::default()
                    },
                ]),
                ..MetaElementInput::default()
            }],
        )
        .unwrap();

        apply_typed_operations(&mut xml, &[operation]).unwrap();

        let first = xml.find("<Name>First</Name>").unwrap();
        let second = xml.find("<Name>Second</Name>").unwrap();
        assert!(first < second, "nested position.before was ignored\n{xml}");
        let first_xml = &xml[xml[..first].rfind("<Attribute ").unwrap()
            ..xml[first..].find("</Attribute>").unwrap() + first + "</Attribute>".len()];
        assert!(first_xml.contains("<Comment>typed nested field</Comment>"));
        assert!(first_xml.contains("<v8:Type>xs:decimal</v8:Type>"));
        assert!(first_xml.contains("<v8:AllowedSign>Nonnegative</v8:AllowedSign>"));
        assert!(first_xml.contains("<FillValue xsi:type=\"xs:decimal\">3.50</FillValue>"));
        assert!(first_xml.contains("<FillChecking>ShowError</FillChecking>"));
    }

    #[test]
    fn typed_top_level_attributes_use_the_kind_aware_platform_profile() {
        for (kind, forbidden) in [
            (
                "Report",
                &[
                    "<FillFromFillingValue>",
                    "<FillValue ",
                    "<Indexing>",
                    "<FullTextSearch>",
                    "<DataHistory>",
                ][..],
            ),
            (
                "AccumulationRegister",
                &["<FillFromFillingValue>", "<FillValue ", "<DataHistory>"][..],
            ),
        ] {
            let mut xml = object_xml(kind, "Sample", "");
            let operation = MetaEditOperation::add(
                MetaCollection::Attributes,
                None,
                vec![MetaElementInput::named("Profiled")],
            )
            .unwrap();
            apply_typed_operations(&mut xml, &[operation]).unwrap();
            for tag in forbidden {
                assert!(!xml.contains(tag), "{kind} emitted forbidden {tag}\n{xml}");
            }
        }
    }

    #[test]
    fn typed_composite_type_uses_platform_node_order() {
        let mut xml = object_xml("Document", "Order", "<RegisterRecords/>");
        let operation = MetaEditOperation::add(
            MetaCollection::Attributes,
            None,
            vec![MetaElementInput {
                name: "Composite".into(),
                r#type: Some(
                    MetadataType::new(vec![
                        MetadataTypeVariant::DefinedType {
                            metadata_path: metadata_reference("DefinedType.ExternalCode")
                                .metadata_path,
                        },
                        MetadataTypeVariant::Number {
                            digits: 12,
                            fraction: 2,
                            sign: NumberSign::Any,
                        },
                        MetadataTypeVariant::String {
                            length: 20,
                            allowed_length: StringLengthMode::Variable,
                        },
                    ])
                    .unwrap(),
                ),
                ..MetaElementInput::default()
            }],
        )
        .unwrap();

        apply_typed_operations(&mut xml, &[operation]).unwrap();

        let decimal = xml.find("<v8:Type>xs:decimal</v8:Type>").unwrap();
        let string = xml.find("<v8:Type>xs:string</v8:Type>").unwrap();
        let type_set = xml
            .find("<v8:TypeSet>cfg:DefinedType.ExternalCode</v8:TypeSet>")
            .unwrap();
        let number_qualifiers = xml.find("<v8:NumberQualifiers>").unwrap();
        let string_qualifiers = xml.find("<v8:StringQualifiers>").unwrap();
        assert!(decimal < string && string < type_set);
        assert!(type_set < number_qualifiers && number_qualifiers < string_qualifiers);
    }

    #[test]
    fn typed_type_and_fill_validation_rejects_invalid_profile_values_with_full_fields() {
        let cases = [
            (
                MetadataType::new(vec![MetadataTypeVariant::Reference {
                    metadata_path: metadata_reference("Report.Sales").metadata_path,
                }])
                .unwrap(),
                None,
                "operations[0].elements[0].type.variants[0].metadataPath",
            ),
            (
                MetadataType::new(vec![MetadataTypeVariant::DefinedType {
                    metadata_path: metadata_reference("Catalog.Items").metadata_path,
                }])
                .unwrap(),
                None,
                "operations[0].elements[0].type.variants[0].metadataPath",
            ),
            (
                MetadataType::new(vec![MetadataTypeVariant::Number {
                    digits: 10,
                    fraction: 2,
                    sign: NumberSign::Any,
                }])
                .unwrap(),
                Some(MetaFillValue::Number("1e3".into())),
                "operations[0].elements[0].fillValue",
            ),
            (
                MetadataType::new(vec![MetadataTypeVariant::Date {
                    fractions: DateFractions::DateTime,
                }])
                .unwrap(),
                Some(MetaFillValue::DateTime("2026-02-30T25:61:00".into())),
                "operations[0].elements[0].fillValue",
            ),
            (
                MetadataType::new(vec![MetadataTypeVariant::String {
                    length: 10,
                    allowed_length: StringLengthMode::Variable,
                }])
                .unwrap(),
                Some(MetaFillValue::Boolean(true)),
                "operations[0].elements[0].fillValue",
            ),
        ];

        for (metadata_type, fill_value, expected_field) in cases {
            let mut xml = object_xml("Document", "Order", "<RegisterRecords/>");
            let operation = MetaEditOperation::add(
                MetaCollection::Attributes,
                None,
                vec![MetaElementInput {
                    name: "Invalid".into(),
                    r#type: Some(metadata_type),
                    fill_value,
                    ..MetaElementInput::default()
                }],
            )
            .unwrap();
            let failure = apply_typed_operations(&mut xml, &[operation]).unwrap_err();
            assert_eq!(
                failure.diagnostics[0].code,
                MetaDiagnosticCode::InvalidArguments
            );
            assert_eq!(
                failure.diagnostics[0].field.as_deref(),
                Some(expected_field)
            );
        }
    }

    #[test]
    fn typed_update_validates_new_type_against_the_existing_fill_value() {
        let mut xml = object_xml("Document", "Order", "<RegisterRecords/>");
        let add = MetaEditOperation::add(
            MetaCollection::Attributes,
            None,
            vec![MetaElementInput {
                name: "CodeText".into(),
                r#type: Some(
                    MetadataType::new(vec![MetadataTypeVariant::String {
                        length: 10,
                        allowed_length: StringLengthMode::Variable,
                    }])
                    .unwrap(),
                ),
                fill_value: Some(MetaFillValue::String("ABC".into())),
                ..MetaElementInput::default()
            }],
        )
        .unwrap();
        let update = MetaEditOperation::update(
            MetaCollection::Attributes,
            None,
            vec![MetaElementUpdateInput {
                name: "CodeText".into(),
                r#type: Some(
                    MetadataType::new(vec![MetadataTypeVariant::Number {
                        digits: 10,
                        fraction: 0,
                        sign: NumberSign::NonNegative,
                    }])
                    .unwrap(),
                ),
                ..MetaElementUpdateInput::default()
            }],
        )
        .unwrap();

        let failure = apply_typed_operations(&mut xml, &[add, update]).unwrap_err();

        assert_eq!(
            failure.diagnostics[0].code,
            MetaDiagnosticCode::InvalidArguments
        );
        assert_eq!(
            failure.diagnostics[0].field.as_deref(),
            Some("operations[1].elements[0].type")
        );
    }

    #[test]
    fn typed_update_validates_new_fill_value_against_the_existing_type() {
        let mut xml = object_xml("Document", "Order", "<RegisterRecords/>");
        let add = MetaEditOperation::add(
            MetaCollection::Attributes,
            None,
            vec![MetaElementInput {
                name: "CodeText".into(),
                r#type: Some(
                    MetadataType::new(vec![MetadataTypeVariant::String {
                        length: 10,
                        allowed_length: StringLengthMode::Variable,
                    }])
                    .unwrap(),
                ),
                ..MetaElementInput::default()
            }],
        )
        .unwrap();
        let update = MetaEditOperation::update(
            MetaCollection::Attributes,
            None,
            vec![MetaElementUpdateInput {
                name: "CodeText".into(),
                fill_value: Some(MetaFillValue::Boolean(true)),
                ..MetaElementUpdateInput::default()
            }],
        )
        .unwrap();

        let failure = apply_typed_operations(&mut xml, &[add, update]).unwrap_err();

        assert_eq!(
            failure.diagnostics[0].code,
            MetaDiagnosticCode::InvalidArguments
        );
        assert_eq!(
            failure.diagnostics[0].field.as_deref(),
            Some("operations[1].elements[0].fillValue")
        );
    }

    #[test]
    fn typed_form_add_plans_owner_descriptor_and_both_physical_form_images() {
        let root = std::env::temp_dir().join(format!(
            "unica-meta-edit-form-plan-{}",
            uuid::Uuid::new_v4()
        ));
        let descriptor_path = root.join("Documents/Order.xml");
        std::fs::create_dir_all(descriptor_path.parent().unwrap()).unwrap();
        let mut post_image = object_xml("Document", "Order", "<RegisterRecords/>");
        let operation = MetaEditOperation::add(
            MetaCollection::Forms,
            None,
            vec![MetaElementInput {
                name: "ObjectForm".into(),
                ..MetaElementInput::default()
            }],
        )
        .unwrap();
        apply_typed_operations(&mut post_image, std::slice::from_ref(&operation)).unwrap();

        let resources = plan_typed_child_resources(
            &descriptor_path,
            &metadata_reference("Document.Order").metadata_path,
            "Document",
            "Order",
            &[operation],
            &post_image,
        )
        .unwrap();
        let create_paths = resources
            .file_mutations
            .iter()
            .filter_map(|mutation| mutation.post_image.as_ref().map(|_| &mutation.path))
            .collect::<Vec<_>>();

        assert_eq!(create_paths.len(), 2);
        assert!(create_paths.contains(&&root.join("Documents/Order/Forms/ObjectForm.xml")));
        assert!(create_paths.contains(&&root.join("Documents/Order/Forms/ObjectForm/Ext/Form.xml")));
        assert_eq!(resources.publication_plan.len(), 1);
        assert_eq!(resources.validation_resources.len(), 1);

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn typed_form_rename_moves_the_descriptor_and_payload_as_one_guarded_resource() {
        let root = std::env::temp_dir().join(format!(
            "unica-meta-edit-form-rename-{}",
            uuid::Uuid::new_v4()
        ));
        let descriptor_path = root.join("Documents/Order.xml");
        let old_descriptor = root.join("Documents/Order/Forms/Old.xml");
        let old_payload = root.join("Documents/Order/Forms/Old/Ext/Form.xml");
        std::fs::create_dir_all(old_payload.parent().unwrap()).unwrap();
        let mut pre_image = object_xml("Document", "Order", "<RegisterRecords/>");
        let add = MetaEditOperation::add(
            MetaCollection::Forms,
            None,
            vec![MetaElementInput {
                name: "Old".into(),
                ..MetaElementInput::default()
            }],
        )
        .unwrap();
        apply_typed_operations(&mut pre_image, &[add]).unwrap();
        let child = typed_child_descriptor_image(&pre_image, "Form", "Old").unwrap();
        std::fs::write(&old_descriptor, child).unwrap();
        std::fs::write(&old_payload, b"<Form version=\"2.20\"/>").unwrap();
        let rename = MetaEditOperation::update(
            MetaCollection::Forms,
            None,
            vec![MetaElementUpdateInput {
                name: "Old".into(),
                new_name: Some("New".into()),
                ..MetaElementUpdateInput::default()
            }],
        )
        .unwrap();
        let mut post_image = pre_image;
        apply_typed_operations(&mut post_image, std::slice::from_ref(&rename)).unwrap();

        let resources = plan_typed_child_resources(
            &descriptor_path,
            &metadata_reference("Document.Order").metadata_path,
            "Document",
            "Order",
            &[rename],
            &post_image,
        )
        .unwrap();

        assert!(resources
            .file_mutations
            .iter()
            .any(|mutation| { mutation.path == old_descriptor && mutation.post_image.is_none() }));
        assert!(resources.file_mutations.iter().any(|mutation| {
            mutation.path == old_payload.parent().unwrap().parent().unwrap()
                && mutation.post_image.is_none()
        }));
        assert!(resources.file_mutations.iter().any(|mutation| {
            mutation.path == root.join("Documents/Order/Forms/New.xml")
                && mutation.post_image.is_some()
        }));
        assert!(resources.file_mutations.iter().any(|mutation| {
            mutation.path == root.join("Documents/Order/Forms/New/Ext/Form.xml")
                && mutation.post_image.as_deref() == Some(b"<Form version=\"2.20\"/>".as_slice())
        }));

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn typed_two_template_preview_entries_have_distinct_exact_logical_addresses() {
        let root = std::env::temp_dir().join(format!(
            "unica-meta-edit-template-plan-{}",
            uuid::Uuid::new_v4()
        ));
        let descriptor_path = root.join("Reports/Sales.xml");
        std::fs::create_dir_all(descriptor_path.parent().unwrap()).unwrap();
        let operation = MetaEditOperation::add(
            MetaCollection::Templates,
            None,
            vec![
                MetaElementInput {
                    name: "A".into(),
                    ..MetaElementInput::default()
                },
                MetaElementInput {
                    name: "B".into(),
                    ..MetaElementInput::default()
                },
            ],
        )
        .unwrap();
        let mut post_image = object_xml("Report", "Sales", "");
        apply_typed_operations(&mut post_image, std::slice::from_ref(&operation)).unwrap();

        let resources = plan_typed_child_resources(
            &descriptor_path,
            &metadata_reference("Report.Sales").metadata_path,
            "Report",
            "Sales",
            &[operation],
            &post_image,
        )
        .unwrap();
        let addresses = resources
            .publication_plan
            .iter()
            .map(|entry| entry.metadata_path.as_ref().unwrap().as_str())
            .collect::<Vec<_>>();
        assert_eq!(
            addresses,
            ["Report.Sales.Template.A", "Report.Sales.Template.B"]
        );

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn typed_command_add_plans_only_its_required_descriptor() {
        let root = std::env::temp_dir().join(format!(
            "unica-meta-edit-command-plan-{}",
            uuid::Uuid::new_v4()
        ));
        let descriptor_path = root.join("Documents/Order.xml");
        std::fs::create_dir_all(descriptor_path.parent().unwrap()).unwrap();
        let operation = MetaEditOperation::add(
            MetaCollection::Commands,
            None,
            vec![MetaElementInput {
                name: "Open".into(),
                ..MetaElementInput::default()
            }],
        )
        .unwrap();
        let mut post_image = object_xml("Document", "Order", "<RegisterRecords/>");
        apply_typed_operations(&mut post_image, std::slice::from_ref(&operation)).unwrap();

        let resources = plan_typed_child_resources(
            &descriptor_path,
            &metadata_reference("Document.Order").metadata_path,
            "Document",
            "Order",
            &[operation],
            &post_image,
        )
        .unwrap();

        assert_eq!(resources.file_mutations.len(), 1);
        assert_eq!(
            resources.file_mutations[0].path,
            root.join("Documents/Order/Commands/Open.xml")
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn typed_form_remove_plans_descriptor_and_whole_payload_tree_removal() {
        let root = std::env::temp_dir().join(format!(
            "unica-meta-edit-form-remove-{}",
            uuid::Uuid::new_v4()
        ));
        let descriptor_path = root.join("Documents/Order.xml");
        let child_descriptor = root.join("Documents/Order/Forms/Old.xml");
        let payload_dir = root.join("Documents/Order/Forms/Old");
        std::fs::create_dir_all(payload_dir.join("Ext")).unwrap();
        let mut pre_image = object_xml("Document", "Order", "<RegisterRecords/>");
        let add = MetaEditOperation::add(
            MetaCollection::Forms,
            None,
            vec![MetaElementInput {
                name: "Old".into(),
                ..MetaElementInput::default()
            }],
        )
        .unwrap();
        apply_typed_operations(&mut pre_image, &[add]).unwrap();
        std::fs::write(
            &child_descriptor,
            typed_child_descriptor_image(&pre_image, "Form", "Old").unwrap(),
        )
        .unwrap();
        std::fs::write(payload_dir.join("Ext/Form.xml"), b"<Form/>").unwrap();
        let remove =
            MetaEditOperation::remove(MetaCollection::Forms, None, vec!["Old".into()]).unwrap();
        let mut post_image = pre_image;
        apply_typed_operations(&mut post_image, std::slice::from_ref(&remove)).unwrap();

        let resources = plan_typed_child_resources(
            &descriptor_path,
            &metadata_reference("Document.Order").metadata_path,
            "Document",
            "Order",
            &[remove],
            &post_image,
        )
        .unwrap();

        assert!(resources
            .file_mutations
            .iter()
            .any(|item| item.path == child_descriptor && item.post_image.is_none()));
        assert!(resources
            .file_mutations
            .iter()
            .any(|item| item.path == payload_dir && item.post_image.is_none()));
        assert_eq!(
            resources.publication_plan[0].action,
            MetaPublicationAction::Remove
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn typed_relation_wire_uses_md_object_refs_and_field_paths() {
        let mut xml = object_xml("Document", "Order", "<RegisterRecords/><InputByString/>");
        let operations = [
            MetaEditOperation::edit_relations(
                MetaRelation::RegisterRecords,
                RelationEditMode::Add,
                vec![metadata_reference("InformationRegister.OrderFacts")],
            )
            .unwrap(),
            MetaEditOperation::edit_relation_targets(
                MetaRelation::InputByString,
                RelationEditMode::Add,
                vec![crate::domain::metadata::MetaRelationTarget::Field(
                    crate::domain::metadata::MetadataFieldPath::parse(
                        "Document.Order.StandardAttribute.Number",
                    )
                    .unwrap(),
                )],
            )
            .unwrap(),
        ];

        apply_typed_operations(&mut xml, &operations).unwrap();

        assert!(xml.contains(
            "<xr:Item xsi:type=\"xr:MDObjectRef\">InformationRegister.OrderFacts</xr:Item>"
        ));
        assert!(xml.contains("<xr:Field>Document.Order.StandardAttribute.Number</xr:Field>"));
    }

    #[test]
    fn typed_form_add_then_rename_collapses_to_one_new_multi_image_resource() {
        let root = std::env::temp_dir().join(format!(
            "unica-meta-edit-form-add-rename-{}",
            uuid::Uuid::new_v4()
        ));
        let descriptor_path = root.join("Documents/Order.xml");
        std::fs::create_dir_all(descriptor_path.parent().unwrap()).unwrap();
        let add = MetaEditOperation::add(
            MetaCollection::Forms,
            None,
            vec![MetaElementInput {
                name: "Old".into(),
                ..MetaElementInput::default()
            }],
        )
        .unwrap();
        let rename = MetaEditOperation::update(
            MetaCollection::Forms,
            None,
            vec![MetaElementUpdateInput {
                name: "Old".into(),
                new_name: Some("New".into()),
                ..MetaElementUpdateInput::default()
            }],
        )
        .unwrap();
        let operations = [add, rename];
        let mut post_image = object_xml("Document", "Order", "<RegisterRecords/>");
        apply_typed_operations(&mut post_image, &operations).unwrap();

        let resources = plan_typed_child_resources(
            &descriptor_path,
            &metadata_reference("Document.Order").metadata_path,
            "Document",
            "Order",
            &operations,
            &post_image,
        )
        .unwrap();

        assert!(resources
            .file_mutations
            .iter()
            .all(|item| !item.path.to_string_lossy().contains("/Old")));
        assert_eq!(resources.publication_plan.len(), 1);
        assert_eq!(
            resources.publication_plan[0].action,
            MetaPublicationAction::Create
        );
        assert_eq!(
            resources.publication_plan[0]
                .metadata_path
                .as_ref()
                .unwrap()
                .as_str(),
            "Document.Order.Form.New"
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn typed_form_add_then_remove_collapses_to_a_resource_noop() {
        let root = std::env::temp_dir().join(format!(
            "unica-meta-edit-form-add-remove-{}",
            uuid::Uuid::new_v4()
        ));
        let descriptor_path = root.join("Documents/Order.xml");
        std::fs::create_dir_all(descriptor_path.parent().unwrap()).unwrap();
        let operations = [
            MetaEditOperation::add(
                MetaCollection::Forms,
                None,
                vec![MetaElementInput {
                    name: "Gone".into(),
                    ..MetaElementInput::default()
                }],
            )
            .unwrap(),
            MetaEditOperation::remove(MetaCollection::Forms, None, vec!["Gone".into()]).unwrap(),
        ];
        let mut post_image = object_xml("Document", "Order", "<RegisterRecords/>");
        apply_typed_operations(&mut post_image, &operations).unwrap();

        let resources = plan_typed_child_resources(
            &descriptor_path,
            &metadata_reference("Document.Order").metadata_path,
            "Document",
            "Order",
            &operations,
            &post_image,
        )
        .unwrap();

        assert!(resources.file_mutations.is_empty());
        assert!(resources.publication_plan.is_empty());
        assert!(resources.validation_resources.is_empty());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn typed_form_rename_then_remove_removes_only_the_original_resource_tree() {
        let root = std::env::temp_dir().join(format!(
            "unica-meta-edit-form-rename-remove-{}",
            uuid::Uuid::new_v4()
        ));
        let descriptor_path = root.join("Documents/Order.xml");
        let child_descriptor = root.join("Documents/Order/Forms/Old.xml");
        let payload_dir = root.join("Documents/Order/Forms/Old");
        std::fs::create_dir_all(payload_dir.join("Ext")).unwrap();
        let mut pre_image = object_xml("Document", "Order", "<RegisterRecords/>");
        let add = MetaEditOperation::add(
            MetaCollection::Forms,
            None,
            vec![MetaElementInput {
                name: "Old".into(),
                ..MetaElementInput::default()
            }],
        )
        .unwrap();
        apply_typed_operations(&mut pre_image, &[add]).unwrap();
        std::fs::write(
            &child_descriptor,
            typed_child_descriptor_image(&pre_image, "Form", "Old").unwrap(),
        )
        .unwrap();
        std::fs::write(payload_dir.join("Ext/Form.xml"), b"<Form/>").unwrap();
        let operations = [
            MetaEditOperation::update(
                MetaCollection::Forms,
                None,
                vec![MetaElementUpdateInput {
                    name: "Old".into(),
                    new_name: Some("New".into()),
                    ..MetaElementUpdateInput::default()
                }],
            )
            .unwrap(),
            MetaEditOperation::remove(MetaCollection::Forms, None, vec!["New".into()]).unwrap(),
        ];
        let mut post_image = pre_image;
        apply_typed_operations(&mut post_image, &operations).unwrap();

        let resources = plan_typed_child_resources(
            &descriptor_path,
            &metadata_reference("Document.Order").metadata_path,
            "Document",
            "Order",
            &operations,
            &post_image,
        )
        .unwrap();

        assert_eq!(resources.publication_plan.len(), 1);
        assert_eq!(
            resources.publication_plan[0].action,
            MetaPublicationAction::Remove
        );
        assert!(resources
            .file_mutations
            .iter()
            .all(|item| !item.path.to_string_lossy().contains("/New")));
        assert!(resources
            .file_mutations
            .iter()
            .any(|item| item.path == child_descriptor));
        assert!(resources
            .file_mutations
            .iter()
            .any(|item| item.path == payload_dir));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn typed_form_update_then_remove_plans_the_original_resource_once() {
        let root = std::env::temp_dir().join(format!(
            "unica-meta-edit-form-update-remove-{}",
            uuid::Uuid::new_v4()
        ));
        let descriptor_path = root.join("Documents/Order.xml");
        let child_descriptor = root.join("Documents/Order/Forms/Old.xml");
        let payload_dir = root.join("Documents/Order/Forms/Old");
        std::fs::create_dir_all(payload_dir.join("Ext")).unwrap();
        let mut pre_image = object_xml("Document", "Order", "<RegisterRecords/>");
        let add = MetaEditOperation::add(
            MetaCollection::Forms,
            None,
            vec![MetaElementInput {
                name: "Old".into(),
                ..MetaElementInput::default()
            }],
        )
        .unwrap();
        apply_typed_operations(&mut pre_image, &[add]).unwrap();
        std::fs::write(
            &child_descriptor,
            typed_child_descriptor_image(&pre_image, "Form", "Old").unwrap(),
        )
        .unwrap();
        std::fs::write(payload_dir.join("Ext/Form.xml"), b"<Form/>").unwrap();
        let operations = [
            MetaEditOperation::update(
                MetaCollection::Forms,
                None,
                vec![MetaElementUpdateInput {
                    name: "Old".into(),
                    comment: Some("changed".into()),
                    ..MetaElementUpdateInput::default()
                }],
            )
            .unwrap(),
            MetaEditOperation::remove(MetaCollection::Forms, None, vec!["Old".into()]).unwrap(),
        ];
        let mut post_image = pre_image;
        apply_typed_operations(&mut post_image, &operations).unwrap();

        let resources = plan_typed_child_resources(
            &descriptor_path,
            &metadata_reference("Document.Order").metadata_path,
            "Document",
            "Order",
            &operations,
            &post_image,
        )
        .unwrap();

        assert_eq!(resources.publication_plan.len(), 1);
        assert_eq!(resources.file_mutations.len(), 2);
        assert_eq!(
            resources.publication_plan[0].action,
            MetaPublicationAction::Remove
        );
        let _ = std::fs::remove_dir_all(root);
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
