use super::integrity_check::check_meta_object_integrity;
use crate::application::metadata::{MetaEditRequest, MetaFailure};
use crate::application::ports::{
    MetadataChildDirectoryKind, MetadataChildFootprintEvidence, MetadataChildProfile,
    MetadataChildResourceKind, MetadataResourceImage, MetadataResourceRole,
    MetadataTemplateResourcePart, MetadataTemplateType, PreparedMetadataMutation,
};
use crate::domain::cancellation::CancellationToken;
use crate::domain::format_profile::ACTIVE_FORMAT_PROFILE;
use crate::domain::metadata::{
    metadata_decimal_shape, metadata_name_eq, metadata_xs_datetime_is_valid,
    validate_metadata_element_value_profile, validate_metadata_operation_capabilities,
    validate_metadata_relation_target_profile, MetaCollection, MetaDiagnostic, MetaDiagnosticCode,
    MetaEditOperation, MetaElementDefinition, MetaElementUpdate, MetaFillValue, MetaMutationEffect,
    MetaPosition, MetaPropertyKey, MetaPropertyValue, MetaPublicationAction,
    MetaPublicationPlanEntry, MetaPublicationResource, MetaRelation, MetaValueProfileContext,
    MetadataKind, MetadataType, RelationEditMode, METADATA_PROPERTY_SPECS,
};
use crate::domain::source_target::{
    MetadataAddress, ResolvedTarget, SourceTarget, TargetKind, PLATFORM_XML_8_3_27_FORMAT_2_20,
};
use crate::domain::workspace::WorkspaceContext;
use crate::infrastructure::platform_xml_source_targets::{
    platform_xml_resource_evidence, resolve_platform_xml_target, ClosedPlatformXmlTarget,
    TargetKindPolicy,
};
use crate::infrastructure::support_guard::{
    evaluate_resolved_support_guard, ResolvedSupportGuardCheck,
};
use roxmltree::Document;
use serde_json::{Map as JsonMap, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

use super::super::common::{escape_xml, is_1c_identifier};
use super::super::compile_transaction::{
    snapshot_directory_membership, DirectoryMembershipSelector, DirectoryMembershipSnapshot,
};
use super::super::text_snapshot::{
    resolve_observed_line_ending, LineEnding, SourceTextSnapshot, Utf8Bom,
};
use super::format_contract::{
    validate_metadata_8_3_27_boolean_contract, validate_metadata_8_3_27_enum_contract,
};
use super::publisher::{fresh_metadata_uuid, PreparedMetaEdit};
use super::template_catalog::{
    emit_meta_attribute, emit_meta_enum_value, emit_meta_register_field, emit_meta_tabular_section,
    meta_attribute_context, metadata_standard_attribute_names, split_meta_camel_case,
    MetadataAttributeTemplate, MetadataEnumValueTemplate, MetadataTabularSectionTemplate,
};
use super::validation_context::{
    event_source_dependency_contract, validate_event_source_dependency_descriptor,
    validate_event_source_registration,
};
use super::xml_model::{
    canonical_meta_event_sources, emit_meta_event_subscription_source, emit_meta_mltext,
    emit_meta_typed_fill_value, emit_meta_typed_value_type, event_source_generated_prefix,
    meta_event_subscription_source_node, meta_info_child, meta_info_child_text, meta_info_children,
    meta_info_inner_text, parse_defined_type_event_sources, parse_meta_event_subscription_source,
};

#[derive(Debug, Default)]
pub(super) struct MetaEditCounts {
    pub(crate) added: usize,
    pub(crate) modified: usize,
    pub(crate) removed: usize,
    pub(crate) effects: Vec<MetaMutationEffect>,
}

#[derive(Clone, Copy)]
struct MetaEditSourceFormat {
    has_bom: bool,
    eol: LineEnding,
}

fn meta_edit_preserve_source_format(text: &str, format: MetaEditSourceFormat) -> Vec<u8> {
    let normalized = text
        .trim_start_matches('\u{feff}')
        .replace("\r\n", "\n")
        .replace('\r', "\n");
    let serialized = normalized.replace('\n', format.eol.as_str());
    let mut bytes = Vec::with_capacity(serialized.len() + usize::from(format.has_bom) * 3);
    if format.has_bom {
        bytes.extend_from_slice(b"\xef\xbb\xbf");
    }
    bytes.extend_from_slice(serialized.as_bytes());
    bytes
}

fn meta_edit_lf_to_eol(text: &str, eol: LineEnding) -> String {
    text.replace('\n', eol.as_str())
}

fn meta_edit_preserve_existing_eol(text: &str, has_bom: bool) -> Vec<u8> {
    let source = text.trim_start_matches('\u{feff}');
    let mut bytes = Vec::with_capacity(source.len() + usize::from(has_bom) * 3);
    if has_bom {
        bytes.extend_from_slice(b"\xef\xbb\xbf");
    }
    bytes.extend_from_slice(source.as_bytes());
    bytes
}

fn meta_edit_line_indent(text: &str, position: usize) -> String {
    let line_start = text[..position]
        .rfind(['\r', '\n'])
        .map_or(0, |index| index + 1);
    text[line_start..position]
        .chars()
        .take_while(|character| matches!(character, '\t' | ' '))
        .collect()
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
            if meta_edit_child_object_name(child)
                .is_some_and(|candidate| metadata_name_eq(&candidate, name))
            {
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
        .find(|section| {
            meta_edit_child_object_name(*section)
                .is_some_and(|candidate| metadata_name_eq(&candidate, section_name))
        })
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
            if meta_edit_child_object_name(child)
                .is_some_and(|candidate| metadata_name_eq(&candidate, name))
            {
                return Err(format!("{tag} '{section_name}.{name}' already exists"));
            }
        }
    }
    Ok(())
}

/// The 8.3.27 export orders register children `Resource` before `Attribute`
/// before `Dimension` (uniform across the reference dump), so appended
/// collections must land in their canonical slot or the platform reorders the
/// file on the first roundtrip.
fn register_collection_slot(tag: &str) -> Option<u8> {
    match tag {
        "Resource" => Some(0),
        "Attribute" => Some(1),
        "Dimension" => Some(2),
        _ => None,
    }
}

fn inserted_tag(lines: &[String]) -> Option<String> {
    let first = lines.first()?.trim_start();
    let rest = first.strip_prefix('<')?;
    let end = rest.find([' ', '>', '/'])?;
    Some(rest[..end].to_string())
}

pub(super) fn meta_edit_insert_top_child_object(
    xml_text: &mut String,
    lines: &[String],
) -> Result<(), String> {
    let doc = Document::parse(xml_text.trim_start_matches('\u{feff}'))
        .map_err(|err| format!("XML parse error: {err}"))?;
    let object = meta_edit_object_node(&doc)?;
    if let Some(child_objects) = meta_info_child(object, "ChildObjects") {
        if let Some(slot) = inserted_tag(lines)
            .as_deref()
            .and_then(register_collection_slot)
        {
            let anchor = child_objects
                .children()
                .filter(|child| child.is_element())
                .find(|child| {
                    register_collection_slot(child.tag_name().name())
                        .is_some_and(|existing| existing > slot)
                })
                .map(|child| child.range());
            if let Some(anchor_range) = anchor {
                drop(doc);
                meta_edit_insert_lines_near_node(xml_text, anchor_range, false, lines);
                return Ok(());
            }
        }
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
        .find(|child| {
            meta_edit_child_object_name(*child)
                .is_some_and(|candidate| metadata_name_eq(&candidate, target_name))
        })
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
        .find(|child| {
            meta_edit_child_object_name(*child)
                .is_some_and(|candidate| metadata_name_eq(&candidate, target_name))
        })
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
        xml_text.insert_str(line_start, &format!("{content}\n"));
    } else {
        xml_text.insert_str(close_pos, &format!("{content}\n{close_indent}"));
    }
    Ok(())
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
            xml_text.insert_str(insert_pos, &format!("{content}\n"));
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

pub(crate) struct ResolvedMetadataObject {
    pub(super) handle: ClosedPlatformXmlTarget,
    pub(super) resolved_target: ResolvedTarget,
    pub(super) metadata_path: MetadataAddress,
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
    pub(super) absent_path_guards: Vec<PathBuf>,
    pub(super) exact_file_guards: Vec<(PathBuf, Vec<u8>)>,
    pub(super) directory_guards: Vec<(
        PathBuf,
        DirectoryMembershipSelector,
        DirectoryMembershipSnapshot,
    )>,
    pub(super) publication_plan: Vec<MetaPublicationPlanEntry>,
    pub(super) validation_resources: Vec<MetadataResourceImage>,
    pub(super) validation_footprints: Vec<MetadataChildFootprintEvidence>,
    pub(super) relation_dependencies: Vec<TypedRelationDependency>,
    pub(super) expected_post_images: Vec<(PathBuf, Vec<u8>)>,
}

#[derive(Debug, Default)]
pub(super) struct TypedChildObservation {
    pub(super) validation_resources: Vec<MetadataResourceImage>,
    pub(super) validation_footprints: Vec<MetadataChildFootprintEvidence>,
    pub(super) diagnostics: Vec<MetaDiagnostic>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TypedChildDescriptorStorage {
    Standalone,
    InlineOwner,
}

#[derive(Debug, Clone, Copy)]
struct TypedChildStorageProfile {
    directory: &'static str,
    resource: MetaPublicationResource,
    descriptor: TypedChildDescriptorStorage,
}

#[derive(Debug)]
pub(super) struct TypedRelationDependency {
    pub(super) handle: ClosedPlatformXmlTarget,
    pub(super) path: PathBuf,
    pub(super) bytes: Vec<u8>,
    pub(super) target: MetadataAddress,
    pub(super) modules: Vec<TypedDependencyModule>,
}

#[derive(Debug)]
pub(super) struct TypedDependencyModule {
    pub(super) handle: ClosedPlatformXmlTarget,
    pub(super) path: PathBuf,
    pub(super) bytes: Vec<u8>,
}

pub(super) struct TypedOperationPostImage {
    pub(super) descriptor: Vec<u8>,
    pub(super) child_resources: TypedChildResourcePlan,
    pub(super) effects: Vec<MetaMutationEffect>,
}

pub(super) struct TypedOperationDependencyScope<'a> {
    source_set: &'a str,
    owner_source_root: &'a Path,
    owner_registration_path: &'a Path,
    owner_registration_preimage: &'a [u8],
    include_handler_dependency: bool,
}

impl<'a> TypedOperationDependencyScope<'a> {
    pub(super) fn new(
        source_set: &'a str,
        owner_source_root: &'a Path,
        owner_registration_path: &'a Path,
        owner_registration_preimage: &'a [u8],
        include_handler_dependency: bool,
    ) -> Self {
        Self {
            source_set,
            owner_source_root,
            owner_registration_path,
            owner_registration_preimage,
            include_handler_dependency,
        }
    }
}

/// Build the complete descriptor and child-resource plan privately. No
/// transaction mutation is registered until this function returns success.
pub(super) fn build_typed_operation_post_image(
    dependency_scope: TypedOperationDependencyScope<'_>,
    descriptor_path: &Path,
    target: &MetadataAddress,
    descriptor_preimage: &[u8],
    operations: &[MetaEditOperation],
    context: &WorkspaceContext,
) -> Result<TypedOperationPostImage, MetaFailure> {
    let source_only = !operations.is_empty()
        && operations.iter().all(|operation| {
            matches!(
                operation,
                MetaEditOperation::EditRelations {
                    relation: MetaRelation::Source,
                    mode: RelationEditMode::Replace,
                    ..
                }
            )
        });
    let snapshot = SourceTextSnapshot::from_bytes(descriptor_preimage).map_err(|error| {
        MetaFailure::from(
            typed_diagnostic(
                MetaDiagnosticCode::ProviderUnavailable,
                format!("metadata descriptor snapshot failed: {error}"),
                None,
            )
            .with_metadata_path(target.clone()),
        )
    })?;
    // Смешанный профиль — дефект содержимого источника, а не отказ провайдера:
    // байты прочитаны и декодированы, но их вид не проходит проверку writer-а,
    // как и неканоничные fill-значения ниже по файлу.
    let eol = resolve_observed_line_ending(&snapshot, None).map_err(|error| {
        MetaFailure::from(
            typed_diagnostic(
                MetaDiagnosticCode::ValidationFailed,
                format!("metadata descriptor EOL policy failed: {error}"),
                None,
            )
            .with_metadata_path(target.clone()),
        )
    })?;
    let source_format = MetaEditSourceFormat {
        has_bom: snapshot.bom() == Utf8Bom::Present,
        eol,
    };
    let mut xml = snapshot.text().to_string();
    let normalized_preimage = xml.clone();
    let applied = apply_typed_operations(&mut xml, operations).map_err(|mut failure| {
        for diagnostic in &mut failure.diagnostics {
            diagnostic.metadata_path = Some(target.clone());
        }
        failure
    })?;
    restore_initial_empty_top_child_objects(&normalized_preimage, &mut xml);
    let (object_kind, object_name) = meta_edit_object_identity(&xml).map_err(|_| {
        MetaFailure::from(
            typed_diagnostic(
                MetaDiagnosticCode::ProviderUnavailable,
                "typed metadata post-image identity is unavailable",
                None,
            )
            .with_metadata_path(target.clone()),
        )
    })?;
    let mut child_resources = plan_typed_child_resources(
        descriptor_path,
        target,
        &object_kind,
        &object_name,
        operations,
        &xml,
    )?;
    // Вид вне закрытого набора по-прежнему пропускает проверки, которым нужен
    // typed-профиль (ADR-0030); жёстко он требуется только предопределённым
    // данным ниже.
    let owner_kind = MetadataKind::parse(&object_kind).ok();
    // Разрешение набора исходников нужно только companion-writer-у
    // `Ext/Predefined.xml`; правка, не касающаяся предопределённых данных и
    // `CodeType`, не обязана его требовать.
    let planned_predefined =
        if super::predefined::descriptor_edit_requires_predefined_planning(operations) {
            let owner_kind = owner_kind.ok_or_else(|| {
                MetaFailure::from(
                    typed_diagnostic(
                        MetaDiagnosticCode::UnsupportedKind,
                        format!("metadata kind `{object_kind}` has no typed operation profile"),
                        None,
                    )
                    .with_metadata_path(target.clone()),
                )
            })?;
            let source_root = crate::infrastructure::source_roots::resolve_named_source_set(
                context,
                dependency_scope.source_set,
            )
            .map_err(|_| {
                MetaFailure::from(
                    typed_diagnostic(
                        MetaDiagnosticCode::ProviderUnavailable,
                        "metadata source set cannot be resolved for predefined data",
                        None,
                    )
                    .with_metadata_path(target.clone()),
                )
            })?
            .path;
            super::predefined::plan_predefined_resource_after_descriptor_edit(
                &source_root,
                descriptor_path,
                target,
                owner_kind,
                &normalized_preimage,
                &xml,
                operations,
            )?
        } else {
            super::predefined::PlannedPredefinedResource {
                resources: TypedChildResourcePlan::default(),
                effects: Vec::new(),
            }
        };
    let predefined_resources = planned_predefined.resources;
    child_resources
        .file_mutations
        .extend(predefined_resources.file_mutations);
    child_resources
        .absent_path_guards
        .extend(predefined_resources.absent_path_guards);
    child_resources
        .exact_file_guards
        .extend(predefined_resources.exact_file_guards);
    child_resources
        .directory_guards
        .extend(predefined_resources.directory_guards);
    child_resources
        .publication_plan
        .extend(predefined_resources.publication_plan);
    child_resources
        .validation_resources
        .extend(predefined_resources.validation_resources);
    child_resources
        .validation_footprints
        .extend(predefined_resources.validation_footprints);
    child_resources
        .expected_post_images
        .extend(predefined_resources.expected_post_images);
    // ADR-0072: справка — фасет владельца рядом с дескриптором; её файлы
    // планируются тем же post-image построением, что и остальные ресурсы.
    let help_resources = super::help_facet::plan_help_resource_after_descriptor_edit(
        descriptor_path,
        target,
        &object_name,
        operations,
    )?;
    child_resources
        .file_mutations
        .extend(help_resources.file_mutations);
    child_resources
        .absent_path_guards
        .extend(help_resources.absent_path_guards);
    child_resources
        .exact_file_guards
        .extend(help_resources.exact_file_guards);
    child_resources
        .publication_plan
        .extend(help_resources.publication_plan);
    child_resources
        .expected_post_images
        .extend(help_resources.expected_post_images);
    child_resources.relation_dependencies =
        resolve_typed_relation_dependencies(&dependency_scope, target, operations, context, &xml)?;
    if target.segments().next() == Some("EventSubscription") {
        let directory = descriptor_path.parent().ok_or_else(|| {
            MetaFailure::from(
                typed_diagnostic(
                    MetaDiagnosticCode::ProviderUnavailable,
                    "EventSubscription collection topology is unavailable",
                    Some("relations.source"),
                )
                .with_metadata_path(target.clone()),
            )
        })?;
        let snapshot =
            snapshot_directory_membership(directory, DirectoryMembershipSelector::XmlFiles)
                .map_err(|_| {
                    MetaFailure::from(
                        typed_diagnostic(
                            MetaDiagnosticCode::ProviderUnavailable,
                            "EventSubscription collection topology is unavailable",
                            Some("relations.source"),
                        )
                        .with_metadata_path(target.clone()),
                    )
                })?;
        child_resources.directory_guards.push((
            directory.to_path_buf(),
            DirectoryMembershipSelector::XmlFiles,
            snapshot,
        ));
    }
    // Итоговое состояние вызова, а не каждая операция по отдельности: замена
    // единственного измерения через `remove` вместе с `add` остаётся законной,
    // потому что промежуточная пустота здесь не наблюдается (ADR-0030). Вид,
    // которого нет в закрытом наборе, условий не имеет и проверку пропускает.
    if let Some(owner_kind) = owner_kind {
        check_meta_object_integrity(owner_kind, xml.as_bytes()).map_err(|diagnostic| {
            MetaFailure::from(diagnostic.with_metadata_path(target.clone()))
        })?;
    }
    let descriptor = if xml == normalized_preimage {
        descriptor_preimage.to_vec()
    } else if source_only {
        meta_edit_preserve_existing_eol(&xml, source_format.has_bom)
    } else {
        meta_edit_preserve_source_format(&xml, source_format)
    };
    let mut effects = applied.effects;
    effects.extend(planned_predefined.effects);
    effects.sort_by_key(|effect| effect.operation_index);
    Ok(TypedOperationPostImage {
        descriptor,
        child_resources,
        effects,
    })
}

pub(super) fn plan_typed_child_resources(
    descriptor_path: &Path,
    owner: &MetadataAddress,
    object_kind: &str,
    object_name: &str,
    operations: &[MetaEditOperation],
    post_image: &str,
) -> Result<TypedChildResourcePlan, MetaFailure> {
    #[derive(Debug)]
    enum Origin {
        Existing(String),
        Added,
    }
    #[derive(Debug)]
    struct State {
        collection: MetaCollection,
        origin: Origin,
        current_name: Option<String>,
        touched: bool,
    }

    let mut states = Vec::<State>::new();
    let mut active = BTreeMap::<(String, String), usize>::new();
    let mut touched_collections = BTreeSet::<String>::new();
    for operation in operations {
        match operation {
            MetaEditOperation::Add {
                collection,
                elements,
                ..
            } if typed_physical_collection(*collection).is_some() => {
                touched_collections.insert(collection.as_str().to_string());
                for element in elements {
                    let index = states.len();
                    states.push(State {
                        collection: *collection,
                        origin: Origin::Added,
                        current_name: Some(element.name.clone()),
                        touched: true,
                    });
                    active.insert(
                        (collection.as_str().to_string(), element.name.clone()),
                        index,
                    );
                }
            }
            MetaEditOperation::Update {
                collection,
                elements,
                ..
            } if typed_physical_collection(*collection).is_some() => {
                touched_collections.insert(collection.as_str().to_string());
                for element in elements {
                    let key = (collection.as_str().to_string(), element.name.clone());
                    let index = active.remove(&key).unwrap_or_else(|| {
                        let index = states.len();
                        states.push(State {
                            collection: *collection,
                            origin: Origin::Existing(element.name.clone()),
                            current_name: Some(element.name.clone()),
                            touched: true,
                        });
                        index
                    });
                    let next_name = element
                        .new_name
                        .clone()
                        .unwrap_or_else(|| element.name.clone());
                    states[index].current_name = Some(next_name.clone());
                    states[index].touched = true;
                    active.insert((collection.as_str().to_string(), next_name), index);
                }
            }
            MetaEditOperation::Remove {
                collection, names, ..
            } if typed_physical_collection(*collection).is_some() => {
                touched_collections.insert(collection.as_str().to_string());
                for name in names {
                    let key = (collection.as_str().to_string(), name.clone());
                    let index = active.remove(&key).unwrap_or_else(|| {
                        let index = states.len();
                        states.push(State {
                            collection: *collection,
                            origin: Origin::Existing(name.clone()),
                            current_name: Some(name.clone()),
                            touched: true,
                        });
                        index
                    });
                    states[index].current_name = None;
                    states[index].touched = true;
                }
            }
            _ => {}
        }
    }

    let final_document =
        Document::parse(post_image.trim_start_matches('\u{feff}')).map_err(|_| {
            MetaFailure::from(
                typed_diagnostic(
                    MetaDiagnosticCode::ProviderUnavailable,
                    "typed owner post-image is not valid XML",
                    None,
                )
                .with_metadata_path(owner.clone()),
            )
        })?;
    let final_object = meta_edit_object_node(&final_document).map_err(|_| {
        MetaFailure::from(
            typed_diagnostic(
                MetaDiagnosticCode::ProviderUnavailable,
                "typed owner post-image object is unavailable",
                None,
            )
            .with_metadata_path(owner.clone()),
        )
    })?;
    if let Some(children) = meta_info_child(final_object, "ChildObjects") {
        for (collection, tag) in [
            (MetaCollection::Forms, "Form"),
            (MetaCollection::Templates, "Template"),
            (MetaCollection::Commands, "Command"),
        ] {
            for child in meta_info_children(children, tag) {
                let Some(name) = typed_physical_owner_child_name(child) else {
                    continue;
                };
                if states.iter().any(|state| {
                    state.collection == collection
                        && state.current_name.as_deref() == Some(name.as_str())
                }) {
                    continue;
                }
                touched_collections.insert(collection.as_str().to_string());
                states.push(State {
                    collection,
                    origin: Origin::Existing(name.clone()),
                    current_name: Some(name),
                    touched: false,
                });
            }
        }
    }

    let object_dir = descriptor_path.with_extension("");
    let mut initial_files = BTreeMap::<PathBuf, Vec<u8>>::new();
    let mut final_files = BTreeMap::<PathBuf, Vec<u8>>::new();
    let mut initial_directories = BTreeSet::<PathBuf>::new();
    let mut final_directories = BTreeSet::<PathBuf>::new();
    let mut collection_snapshots = Vec::new();
    let mut child_topology_roots = Vec::<(PathBuf, MetadataAddress)>::new();
    let mut plan = TypedChildResourcePlan::default();

    for collection_name in touched_collections {
        let collection = states
            .iter()
            .find(|state| state.collection.as_str() == collection_name)
            .map(|state| state.collection)
            .expect("a touched physical collection has at least one state");
        let profile = typed_child_storage_profile(collection).unwrap();
        let collection_dir = object_dir.join(profile.directory);
        let snapshot = snapshot_directory_membership(
            &collection_dir,
            DirectoryMembershipSelector::AllDirectEntries,
        )
        .map_err(|_| typed_child_collection_topology_failure(owner, collection))?;
        if matches!(snapshot, DirectoryMembershipSnapshot::Present(_)) {
            initial_directories.insert(collection_dir.clone());
        }
        collection_snapshots.push((collection, collection_dir, snapshot));
    }

    for state in &states {
        let profile = typed_child_storage_profile(state.collection).unwrap();
        let collection_dir = object_dir.join(profile.directory);
        let resource = profile.resource;
        let mut initial_descriptor = None;
        let mut initial_payload = Vec::new();
        let mut initial_payload_directories = Vec::new();
        if let Origin::Existing(initial_name) = &state.origin {
            let initial_child = typed_child_logical_address(
                owner,
                object_kind,
                object_name,
                state.collection,
                initial_name,
            )?;
            if profile.descriptor == TypedChildDescriptorStorage::Standalone {
                let descriptor = collection_dir.join(format!("{initial_name}.xml"));
                let bytes = read_typed_child_file(&descriptor, owner)?;
                initial_descriptor = Some(bytes.clone());
                initial_files.insert(descriptor, bytes);
            }
            let payload_root = collection_dir.join(initial_name);
            child_topology_roots.push((payload_root.clone(), initial_child.clone()));
            if payload_root.exists() {
                let (files, directories) =
                    read_typed_child_tree(&payload_root).map_err(|error| {
                        typed_child_topology_failure(&initial_child, error.public_message())
                    })?;
                for (relative, bytes) in files {
                    initial_files.insert(payload_root.join(&relative), bytes.clone());
                    initial_payload.push((relative, bytes));
                }
                for relative in directories {
                    let path = payload_root.join(&relative);
                    initial_directories.insert(path);
                    initial_payload_directories.push(relative);
                }
            }
        }

        let final_descriptor = if let Some(final_name) = &state.current_name {
            let bytes = typed_child_descriptor_image(
                post_image,
                collection_tag(state.collection),
                final_name,
            )
            .or_else(|failure| match &state.origin {
                Origin::Existing(initial_name) if initial_name == final_name => {
                    initial_descriptor.clone().ok_or(failure)
                }
                Origin::Existing(_) | Origin::Added => Err(failure),
            })?;
            if profile.descriptor == TypedChildDescriptorStorage::Standalone {
                final_files.insert(
                    collection_dir.join(format!("{final_name}.xml")),
                    bytes.clone(),
                );
            }
            plan.validation_resources.push(MetadataResourceImage {
                role: typed_child_role(state.collection, owner, final_name),
                bytes: bytes.clone(),
            });
            let child_address = typed_child_logical_address(
                owner,
                object_kind,
                object_name,
                state.collection,
                final_name,
            )?;
            child_topology_roots.push((collection_dir.join(final_name), child_address.clone()));
            let template_type = (state.collection == MetaCollection::Templates)
                .then(|| typed_template_type_from_descriptor(&bytes, &child_address))
                .transpose()?;
            let mut final_payload = Vec::<(PathBuf, Vec<u8>)>::new();
            let mut final_payload_directories = Vec::<PathBuf>::new();
            match &state.origin {
                Origin::Existing(_) => {
                    final_payload.clone_from(&initial_payload);
                    final_payload_directories.clone_from(&initial_payload_directories);
                }
                Origin::Added => match state.collection {
                    MetaCollection::Forms => {
                        final_payload_directories.push(PathBuf::new());
                        final_payload_directories.push(PathBuf::from("Ext"));
                        let content =
                            minimal_typed_form_content(object_kind, object_name).into_bytes();
                        final_payload.push((PathBuf::from("Ext/Form.xml"), content));
                    }
                    MetaCollection::Templates => {
                        final_payload_directories.push(PathBuf::new());
                        final_payload_directories.push(PathBuf::from("Ext"));
                        let template_type = template_type
                            .expect("a planned template child publishes its TemplateType");
                        final_payload.push((
                            PathBuf::from(typed_template_primary_path(template_type)),
                            typed_template_initial_content(template_type),
                        ));
                        if template_type == MetadataTemplateType::HtmlDocument {
                            final_payload_directories.push(PathBuf::from("Ext/Template"));
                            final_payload.push((
                                typed_template_page_path("ru"),
                                super::super::template::html_template_page()
                                    .as_bytes()
                                    .to_vec(),
                            ));
                        }
                    }
                    MetaCollection::Commands => {}
                    _ => unreachable!(),
                },
            }
            let footprint = validate_typed_child_footprint(
                state.collection,
                template_type,
                &child_address,
                &final_payload,
                &final_payload_directories,
            )?;
            let retained = footprint.retained.clone();
            plan.validation_footprints.push(footprint);
            let payload_root = collection_dir.join(final_name);
            for relative in &final_payload_directories {
                final_directories.insert(payload_root.join(relative));
            }
            for (relative, bytes) in &final_payload {
                final_files.insert(payload_root.join(relative), bytes.clone());
                plan.validation_resources.push(MetadataResourceImage {
                    role: typed_child_payload_role(
                        state.collection,
                        &child_address,
                        relative,
                        template_type,
                        &retained,
                    )?,
                    bytes: bytes.clone(),
                });
            }
            Some(bytes)
        } else {
            None
        };

        match (&state.origin, &state.current_name) {
            (Origin::Existing(initial_name), None) => {
                let replaced_at_same_name = states.iter().any(|candidate| {
                    candidate.collection == state.collection
                        && candidate.current_name.as_deref() == Some(initial_name.as_str())
                });
                if !replaced_at_same_name {
                    plan.publication_plan.push(MetaPublicationPlanEntry {
                        action: MetaPublicationAction::Remove,
                        resource,
                        metadata_path: Some(typed_child_logical_address(
                            owner,
                            object_kind,
                            object_name,
                            state.collection,
                            initial_name,
                        )?),
                    });
                }
            }
            (Origin::Existing(initial_name), Some(final_name)) => {
                if state.touched
                    && (initial_name != final_name
                        || profile.descriptor == TypedChildDescriptorStorage::InlineOwner
                        || initial_descriptor.as_ref() != final_descriptor.as_ref())
                {
                    plan.publication_plan.push(MetaPublicationPlanEntry {
                        action: MetaPublicationAction::Update,
                        resource,
                        metadata_path: Some(typed_child_logical_address(
                            owner,
                            object_kind,
                            object_name,
                            state.collection,
                            final_name,
                        )?),
                    });
                }
            }
            (Origin::Added, Some(final_name)) => {
                let replaces_initial = states.iter().any(|candidate| {
                    candidate.collection == state.collection
                        && matches!(
                            &candidate.origin,
                            Origin::Existing(initial_name) if initial_name == final_name
                        )
                });
                plan.publication_plan.push(MetaPublicationPlanEntry {
                    action: if replaces_initial {
                        MetaPublicationAction::Update
                    } else {
                        MetaPublicationAction::Create
                    },
                    resource,
                    metadata_path: Some(typed_child_logical_address(
                        owner,
                        object_kind,
                        object_name,
                        state.collection,
                        final_name,
                    )?),
                });
            }
            (Origin::Added, None) => {}
        }
    }

    for (collection, collection_dir, snapshot) in collection_snapshots {
        let collection_existed = matches!(snapshot, DirectoryMembershipSnapshot::Present(_));
        let touched_initial_entries = states
            .iter()
            .filter(|state| state.collection == collection)
            .filter_map(|state| match &state.origin {
                Origin::Existing(name) => Some(name),
                Origin::Added => None,
            })
            .flat_map(|name| [name.clone(), format!("{name}.xml")])
            .collect::<BTreeSet<_>>();
        let has_unrelated_initial_entry = match snapshot {
            DirectoryMembershipSnapshot::Absent => false,
            DirectoryMembershipSnapshot::Present(entries) => entries.iter().any(|entry| {
                !touched_initial_entries.contains(&entry.name.to_string_lossy().into_owned())
            }),
        };
        let has_final_child_footprint = final_files
            .keys()
            .any(|path| path.starts_with(&collection_dir))
            || final_directories
                .iter()
                .any(|path| path.starts_with(&collection_dir));
        let removed_existing_child = states.iter().any(|state| {
            state.collection == collection
                && matches!(state.origin, Origin::Existing(_))
                && state.current_name.is_none()
        });
        if has_unrelated_initial_entry
            || has_final_child_footprint
            || (collection_existed && !removed_existing_child)
        {
            final_directories.insert(collection_dir);
        }
    }

    let removed_directories = initial_directories
        .difference(&final_directories)
        .filter(|candidate| {
            !initial_directories
                .difference(&final_directories)
                .any(|parent| parent != *candidate && candidate.starts_with(parent))
        })
        .cloned()
        .collect::<Vec<_>>();
    for directory in &removed_directories {
        plan.file_mutations.push(TypedChildFileMutation {
            path: directory.clone(),
            pre_image: Some(Vec::new()),
            post_image: None,
        });
    }
    let all_file_paths = initial_files
        .keys()
        .chain(final_files.keys())
        .cloned()
        .collect::<BTreeSet<_>>();
    for path in &all_file_paths {
        if plan
            .file_mutations
            .iter()
            .any(|mutation| mutation.post_image.is_none() && path.starts_with(&mutation.path))
        {
            continue;
        }
        match (initial_files.get(path), final_files.get(path)) {
            (Some(before), Some(after)) if before == after => {
                plan.exact_file_guards.push((path.clone(), before.clone()));
            }
            (Some(before), Some(after)) => plan.file_mutations.push(TypedChildFileMutation {
                path: path.clone(),
                pre_image: Some(before.clone()),
                post_image: Some(after.clone()),
            }),
            (Some(before), None) => plan.file_mutations.push(TypedChildFileMutation {
                path: path.clone(),
                pre_image: Some(before.clone()),
                post_image: None,
            }),
            (None, Some(after)) => plan.file_mutations.push(TypedChildFileMutation {
                path: path.clone(),
                pre_image: None,
                post_image: Some(after.clone()),
            }),
            (None, None) => unreachable!(),
        }
    }

    let guarded_directories = initial_directories
        .union(&final_directories)
        .filter(|directory| {
            !removed_directories
                .iter()
                .any(|removed| directory.starts_with(removed))
        })
        .cloned()
        .collect::<Vec<_>>();
    for directory in guarded_directories {
        let snapshot = snapshot_directory_membership(
            &directory,
            DirectoryMembershipSelector::AllDirectEntries,
        )
        .map_err(|_| {
            child_topology_roots
                .iter()
                .filter(|(root, _)| directory.starts_with(root))
                .max_by_key(|(root, _)| root.components().count())
                .map(|(_, child)| {
                    typed_child_topology_failure(
                        child,
                        "typed child resource topology is unavailable",
                    )
                })
                .unwrap_or_else(|| typed_child_collection_guard_failure(owner))
        })?;
        plan.directory_guards.push((
            directory,
            DirectoryMembershipSelector::AllDirectEntries,
            snapshot,
        ));
    }
    Ok(plan)
}

fn typed_physical_owner_child_name(node: roxmltree::Node<'_, '_>) -> Option<String> {
    meta_edit_child_object_name(node).or_else(|| {
        (!node.children().any(|child| child.is_element()))
            .then(|| meta_info_inner_text(node).trim().to_string())
            .filter(|name| !name.is_empty())
    })
}

fn typed_physical_collection(
    collection: MetaCollection,
) -> Option<(&'static str, MetaPublicationResource)> {
    typed_child_storage_profile(collection).map(|profile| (profile.directory, profile.resource))
}

fn typed_child_storage_profile(collection: MetaCollection) -> Option<TypedChildStorageProfile> {
    match collection {
        MetaCollection::Forms => Some(TypedChildStorageProfile {
            directory: "Forms",
            resource: MetaPublicationResource::Form,
            descriptor: TypedChildDescriptorStorage::Standalone,
        }),
        MetaCollection::Templates => Some(TypedChildStorageProfile {
            directory: "Templates",
            resource: MetaPublicationResource::Template,
            descriptor: TypedChildDescriptorStorage::Standalone,
        }),
        MetaCollection::Commands => Some(TypedChildStorageProfile {
            directory: "Commands",
            resource: MetaPublicationResource::Command,
            descriptor: TypedChildDescriptorStorage::InlineOwner,
        }),
        _ => None,
    }
}

pub(super) fn observe_typed_child_resources(
    descriptor_path: &Path,
    owner: &MetadataAddress,
    object_kind: &str,
    object_name: &str,
    owner_xml: &str,
) -> Result<TypedChildObservation, MetaFailure> {
    let document = Document::parse(owner_xml.trim_start_matches('\u{feff}')).map_err(|_| {
        MetaFailure::from(
            typed_diagnostic(
                MetaDiagnosticCode::ProviderUnavailable,
                "typed owner observation image is not valid XML",
                None,
            )
            .with_metadata_path(owner.clone()),
        )
    })?;
    let object = meta_edit_object_node(&document).map_err(|_| {
        MetaFailure::from(
            typed_diagnostic(
                MetaDiagnosticCode::ProviderUnavailable,
                "typed owner observation object is unavailable",
                None,
            )
            .with_metadata_path(owner.clone()),
        )
    })?;
    let Some(children) = meta_info_child(object, "ChildObjects") else {
        return Ok(TypedChildObservation::default());
    };

    let object_dir = descriptor_path.with_extension("");
    let mut observation = TypedChildObservation::default();
    for (collection, tag) in [
        (MetaCollection::Forms, "Form"),
        (MetaCollection::Templates, "Template"),
        (MetaCollection::Commands, "Command"),
    ] {
        let profile = typed_child_storage_profile(collection)
            .expect("closed observed child collection has a storage profile");
        let collection_dir = object_dir.join(profile.directory);
        for child_node in meta_info_children(children, tag) {
            let Some(name) = typed_physical_owner_child_name(child_node) else {
                continue;
            };
            let child = match typed_child_logical_address(
                owner,
                object_kind,
                object_name,
                collection,
                &name,
            ) {
                Ok(child) => child,
                Err(failure) => {
                    observation.diagnostics.extend(failure.diagnostics);
                    continue;
                }
            };
            let descriptor = match profile.descriptor {
                TypedChildDescriptorStorage::Standalone => {
                    let path = collection_dir.join(format!("{name}.xml"));
                    match fs::read(&path) {
                        Ok(bytes) => bytes,
                        Err(_) => {
                            observation.diagnostics.push(
                                typed_diagnostic(
                                    MetaDiagnosticCode::ProviderUnavailable,
                                    format!("typed child descriptor is unavailable: {child}"),
                                    Some("resources.child.descriptor"),
                                )
                                .with_metadata_path(child.clone()),
                            );
                            continue;
                        }
                    }
                }
                TypedChildDescriptorStorage::InlineOwner => {
                    match typed_child_descriptor_image(owner_xml, tag, &name) {
                        Ok(bytes) => bytes,
                        Err(failure) => {
                            observation.diagnostics.extend(failure.diagnostics);
                            continue;
                        }
                    }
                }
            };

            observation
                .validation_resources
                .push(MetadataResourceImage {
                    role: typed_child_role(collection, owner, &name),
                    bytes: descriptor.clone(),
                });
            let template_type = (collection == MetaCollection::Templates)
                .then(|| typed_template_type_from_descriptor(&descriptor, &child))
                .transpose();
            let template_type = match template_type {
                Ok(value) => value,
                Err(failure) => {
                    observation.diagnostics.extend(failure.diagnostics);
                    continue;
                }
            };

            let payload_root = collection_dir.join(&name);
            let (payload, directories) = if payload_root.exists() {
                match read_typed_child_tree(&payload_root) {
                    Ok(value) => value,
                    Err(error) => {
                        observation.diagnostics.push(
                            typed_diagnostic(
                                MetaDiagnosticCode::ProviderUnavailable,
                                format!(
                                    "typed child resource topology is unavailable: {child}: {}",
                                    error.public_message()
                                ),
                                Some("resources.child.topology"),
                            )
                            .with_metadata_path(child.clone()),
                        );
                        continue;
                    }
                }
            } else {
                (Vec::new(), Vec::new())
            };
            let footprint = match validate_typed_child_footprint(
                collection,
                template_type,
                &child,
                &payload,
                &directories,
            ) {
                Ok(footprint) => footprint,
                Err(failure) => {
                    observation.diagnostics.extend(failure.diagnostics);
                    continue;
                }
            };
            let retained = footprint.retained.clone();
            observation.validation_footprints.push(footprint);
            for (relative, bytes) in payload {
                let role = match typed_child_payload_role(
                    collection,
                    &child,
                    &relative,
                    template_type,
                    &retained,
                ) {
                    Ok(role) => role,
                    Err(failure) => {
                        observation.diagnostics.extend(failure.diagnostics);
                        continue;
                    }
                };
                observation
                    .validation_resources
                    .push(MetadataResourceImage { role, bytes });
            }
        }
    }
    Ok(observation)
}

#[derive(Debug)]
struct TypedChildFootprintProfile {
    logical_profile: MetadataChildProfile,
    required_files: BTreeSet<PathBuf>,
    optional_files: BTreeSet<PathBuf>,
}

fn typed_child_footprint_profile(
    collection: MetaCollection,
    template_type: Option<MetadataTemplateType>,
    child: &MetadataAddress,
) -> Result<TypedChildFootprintProfile, MetaFailure> {
    let paths = |values: &[&str]| values.iter().map(PathBuf::from).collect::<BTreeSet<_>>();
    match collection {
        MetaCollection::Forms => Ok(TypedChildFootprintProfile {
            logical_profile: MetadataChildProfile::Form,
            required_files: paths(&[TYPED_FORM_CONTENT_PATH]),
            optional_files: paths(&[TYPED_FORM_MODULE_PATH]),
        }),
        MetaCollection::Commands => Ok(TypedChildFootprintProfile {
            logical_profile: MetadataChildProfile::Command,
            required_files: BTreeSet::new(),
            optional_files: paths(&[TYPED_COMMAND_MODULE_PATH]),
        }),
        MetaCollection::Templates => {
            let template_type = template_type.ok_or_else(|| {
                typed_child_resource_failure(
                    child,
                    "template payload has no closed TemplateType evidence",
                )
            })?;
            Ok(TypedChildFootprintProfile {
                logical_profile: MetadataChildProfile::Template(template_type),
                required_files: paths(&[typed_template_primary_path(template_type)]),
                optional_files: BTreeSet::new(),
            })
        }
        _ => unreachable!(),
    }
}

pub(super) const TYPED_FORM_CONTENT_PATH: &str = "Ext/Form.xml";
/// A managed form's module. Designer nests it one level below the other `Ext`
/// members — `Ext/Form/Module.bsl`, not `Ext/Module.bsl`, which is where an
/// *object* module lives. Every other reader in the crate already uses this
/// path; the footprint contract used to disagree with them (issue #360).
pub(super) const TYPED_FORM_MODULE_PATH: &str = "Ext/Form/Module.bsl";
pub(super) const TYPED_COMMAND_MODULE_PATH: &str = "Ext/CommandModule.bsl";

pub(super) fn typed_template_primary_path(template_type: MetadataTemplateType) -> &'static str {
    match template_type {
        MetadataTemplateType::HtmlDocument
        | MetadataTemplateType::SpreadsheetDocument
        | MetadataTemplateType::DataCompositionSchema => "Ext/Template.xml",
        MetadataTemplateType::TextDocument => "Ext/Template.txt",
        MetadataTemplateType::BinaryData => "Ext/Template.bin",
    }
}

/// ADR-0072: начальное содержимое нового макета по виду — то, что порождал
/// снимаемый `template.add`; байты канала сохраняют прежнее typed-поведение
/// (без BOM), текстовый и бинарный виды пусты.
pub(super) fn typed_template_initial_content(template_type: MetadataTemplateType) -> Vec<u8> {
    match template_type {
        MetadataTemplateType::SpreadsheetDocument => {
            super::super::mxl::empty_spreadsheet_document_xml().into_bytes()
        }
        // Платформенная форма HTMLDocument: primary — дескриптор страниц
        // `<Help>` (extrnprops), сами страницы лежат в Ext/Template/*.html.
        // Снимаемый template.add писал сюда сырой HTML — typed-канал пишет
        // форму, которую закрепил валидатор по реальным дампам 8.3.27.
        MetadataTemplateType::HtmlDocument => {
            crate::infrastructure::native_operations::help::help_metadata_xml(
                "ru",
                crate::domain::format_profile::ACTIVE_FORMAT_PROFILE.export_format,
            )
            .into_bytes()
        }
        MetadataTemplateType::DataCompositionSchema => {
            super::super::template::template_content_xml("DataCompositionSchema", "")
                .expect("closed template kind")
                .into_bytes()
        }
        MetadataTemplateType::TextDocument | MetadataTemplateType::BinaryData => Vec::new(),
    }
}

pub(super) fn typed_template_page_path(page: &str) -> PathBuf {
    PathBuf::from(format!("Ext/Template/{page}.html"))
}

/// The payload members Unica models for a child: the ones it parses, rewrites
/// and can address. Everything else a real payload holds is retained instead
/// (see [`typed_child_retained_payload`]). Both the planner and the validator
/// derive their expectations from here, so neither can drift from the other.
pub(super) fn typed_child_modelled_payload(
    profile: MetadataChildProfile,
    has_module: bool,
    html_pages: &[String],
) -> BTreeSet<PathBuf> {
    let mut files = BTreeSet::new();
    match profile {
        MetadataChildProfile::Form => {
            files.insert(PathBuf::from(TYPED_FORM_CONTENT_PATH));
            if has_module {
                files.insert(PathBuf::from(TYPED_FORM_MODULE_PATH));
            }
        }
        MetadataChildProfile::Command => {
            if has_module {
                files.insert(PathBuf::from(TYPED_COMMAND_MODULE_PATH));
            }
        }
        MetadataChildProfile::Template(template_type) => {
            files.insert(PathBuf::from(typed_template_primary_path(template_type)));
            if template_type == MetadataTemplateType::HtmlDocument {
                files.extend(html_pages.iter().map(|page| typed_template_page_path(page)));
            }
        }
    }
    files
}

/// Payload members the platform writes beside a child that Unica carries
/// verbatim instead of interpreting. Measured against a real 8.3.27 Designer
/// dump: form help and its assets, per-item pictures, HTML template assets.
///
/// The shapes stay closed so a mutation still refuses to touch a child whose
/// directory holds bytes this contract does not recognise.
pub(super) fn typed_child_retained_payload(profile: MetadataChildProfile, relative: &Path) -> bool {
    match profile {
        MetadataChildProfile::Form => {
            relative == Path::new("Ext/Help.xml")
                || relative.starts_with("Ext/Help")
                || relative.starts_with("Ext/Form/Items")
        }
        MetadataChildProfile::Template(MetadataTemplateType::HtmlDocument) => {
            relative.starts_with("Ext/Template/_files")
        }
        MetadataChildProfile::Command | MetadataChildProfile::Template(_) => false,
    }
}

/// The directory topology of a child payload is exactly the parent closure of
/// its files: every directory holds something, and nothing holds nothing. That
/// single rule reproduces each shape the platform writes and still rejects a
/// stray empty directory.
pub(super) fn typed_child_payload_directories<'a>(
    files: impl IntoIterator<Item = &'a PathBuf>,
) -> BTreeSet<PathBuf> {
    let mut directories = BTreeSet::new();
    for file in files {
        let mut next = file.parent();
        while let Some(directory) = next {
            directories.insert(directory.to_path_buf());
            if directory.as_os_str().is_empty() {
                break;
            }
            next = directory.parent();
        }
    }
    directories
}

pub(super) fn typed_child_directory_kind(relative: &Path) -> MetadataChildDirectoryKind {
    if relative.as_os_str().is_empty() {
        MetadataChildDirectoryKind::Root
    } else if relative == Path::new("Ext") {
        MetadataChildDirectoryKind::Extension
    } else if relative == Path::new("Ext/Template") {
        MetadataChildDirectoryKind::HtmlPages
    } else {
        MetadataChildDirectoryKind::Nested
    }
}

fn validate_typed_child_footprint(
    collection: MetaCollection,
    template_type: Option<MetadataTemplateType>,
    child: &MetadataAddress,
    files: &[(PathBuf, Vec<u8>)],
    directories: &[PathBuf],
) -> Result<MetadataChildFootprintEvidence, MetaFailure> {
    let profile = typed_child_footprint_profile(collection, template_type, child)?;
    let mut required_files = profile.required_files.clone();
    if template_type == Some(MetadataTemplateType::HtmlDocument) {
        let descriptor = files
            .iter()
            .find(|(relative, _)| relative == Path::new("Ext/Template.xml"))
            .ok_or_else(|| {
                typed_child_resource_failure(child, "HTML template descriptor is unavailable")
            })?;
        let pages = super::validation::html_template_page_names(&descriptor.1)
            .map_err(|message| typed_child_resource_failure(child, &message))?;
        required_files.extend(
            pages
                .into_iter()
                .map(|page| PathBuf::from(format!("Ext/Template/{page}.html"))),
        );
    }
    let observed_files = files
        .iter()
        .map(|(relative, _)| relative.clone())
        .collect::<BTreeSet<_>>();
    let modelled_files = required_files
        .union(&profile.optional_files)
        .cloned()
        .collect::<BTreeSet<_>>();
    let retained = observed_files
        .difference(&modelled_files)
        .cloned()
        .collect::<Vec<_>>();
    if !required_files.is_subset(&observed_files)
        || observed_files.len() != files.len()
        || !retained
            .iter()
            .all(|relative| typed_child_retained_payload(profile.logical_profile, relative))
    {
        return Err(typed_child_resource_failure(
            child,
            "child payload does not contain its exact required resource set",
        ));
    }

    let observed_directories = directories.iter().cloned().collect::<BTreeSet<_>>();
    let expected_directories = typed_child_payload_directories(&observed_files);
    if observed_directories != expected_directories
        || observed_directories.len() != directories.len()
    {
        return Err(typed_child_topology_failure(
            child,
            "child payload does not contain its exact required directory topology",
        ));
    }

    Ok(MetadataChildFootprintEvidence {
        child: child.clone(),
        profile: profile.logical_profile,
        directories: observed_directories
            .iter()
            .map(|relative| typed_child_directory_kind(relative))
            .collect(),
        retained,
    })
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

fn typed_child_payload_role(
    collection: MetaCollection,
    child: &MetadataAddress,
    relative_path: &Path,
    template_type: Option<MetadataTemplateType>,
    retained: &[PathBuf],
) -> Result<MetadataResourceRole, MetaFailure> {
    if let Some(ordinal) = retained
        .iter()
        .position(|candidate| candidate == relative_path)
    {
        return Ok(MetadataResourceRole::ChildResource {
            child: child.clone(),
            kind: MetadataChildResourceKind::Retained,
            ordinal,
        });
    }
    let kind = match collection {
        MetaCollection::Forms if relative_path == Path::new("Ext/Form.xml") => {
            MetadataChildResourceKind::FormContent
        }
        MetaCollection::Forms if relative_path == Path::new(TYPED_FORM_MODULE_PATH) => {
            MetadataChildResourceKind::Module
        }
        MetaCollection::Commands if relative_path == Path::new("Ext/CommandModule.bsl") => {
            MetadataChildResourceKind::Module
        }
        MetaCollection::Templates => {
            let template_type = template_type.ok_or_else(|| {
                typed_child_resource_failure(
                    child,
                    "template payload has no closed TemplateType evidence",
                )
            })?;
            let part = match (template_type, relative_path) {
                (
                    MetadataTemplateType::SpreadsheetDocument
                    | MetadataTemplateType::DataCompositionSchema,
                    path,
                ) if path == Path::new("Ext/Template.xml") => MetadataTemplateResourcePart::Primary,
                (MetadataTemplateType::TextDocument, path)
                    if path == Path::new("Ext/Template.txt") =>
                {
                    MetadataTemplateResourcePart::Primary
                }
                (MetadataTemplateType::BinaryData, path)
                    if path == Path::new("Ext/Template.bin") =>
                {
                    MetadataTemplateResourcePart::Primary
                }
                (MetadataTemplateType::HtmlDocument, path)
                    if path == Path::new("Ext/Template.xml") =>
                {
                    MetadataTemplateResourcePart::Primary
                }
                (MetadataTemplateType::HtmlDocument, path)
                    if path.parent() == Some(Path::new("Ext/Template"))
                        && path.extension().and_then(|extension| extension.to_str())
                            == Some("html") =>
                {
                    MetadataTemplateResourcePart::HtmlPage
                }
                _ => {
                    return Err(typed_child_resource_failure(
                        child,
                        "template payload footprint does not match its TemplateType",
                    ))
                }
            };
            MetadataChildResourceKind::TemplateContent {
                template_type,
                part,
            }
        }
        _ => {
            return Err(typed_child_resource_failure(
                child,
                "child payload contains an unsupported resource role",
            ))
        }
    };
    let ordinal = match (collection, kind) {
        (MetaCollection::Forms, MetadataChildResourceKind::FormContent) => 0,
        (MetaCollection::Forms, MetadataChildResourceKind::Module) => 1,
        (MetaCollection::Commands, MetadataChildResourceKind::Module) => 0,
        (
            MetaCollection::Templates,
            MetadataChildResourceKind::TemplateContent {
                part: MetadataTemplateResourcePart::Primary,
                ..
            },
        ) => 0,
        (
            MetaCollection::Templates,
            MetadataChildResourceKind::TemplateContent {
                part: MetadataTemplateResourcePart::HtmlPage,
                ..
            },
        ) => 1,
        _ => unreachable!("closed child resource kind is owned by its collection"),
    };
    Ok(MetadataResourceRole::ChildResource {
        child: child.clone(),
        kind,
        ordinal,
    })
}

fn typed_template_type_from_descriptor(
    descriptor: &[u8],
    child: &MetadataAddress,
) -> Result<MetadataTemplateType, MetaFailure> {
    let text = std::str::from_utf8(descriptor)
        .map_err(|_| typed_child_resource_failure(child, "template descriptor is not UTF-8"))?;
    let document = Document::parse(text.trim_start_matches('\u{feff}'))
        .map_err(|_| typed_child_resource_failure(child, "template descriptor is malformed XML"))?;
    let values = document
        .descendants()
        .filter(|node| node.is_element() && node.tag_name().name() == "TemplateType")
        .filter_map(|node| node.text())
        .map(str::trim)
        .collect::<Vec<_>>();
    let [value] = values.as_slice() else {
        return Err(typed_child_resource_failure(
            child,
            "template descriptor must contain exactly one TemplateType",
        ));
    };
    MetadataTemplateType::from_descriptor_value(value).ok_or_else(|| {
        typed_child_resource_failure(
            child,
            "template descriptor uses an unsupported TemplateType",
        )
    })
}

fn typed_child_resource_failure(child: &MetadataAddress, message: &str) -> MetaFailure {
    typed_diagnostic(
        MetaDiagnosticCode::ProviderUnavailable,
        format!("{message}: {child}"),
        Some("resources.child.payload"),
    )
    .with_metadata_path(child.clone())
    .into()
}

fn typed_child_topology_failure(child: &MetadataAddress, message: &str) -> MetaFailure {
    typed_diagnostic(
        MetaDiagnosticCode::ProviderUnavailable,
        format!("{message}: {child}"),
        Some("resources.child.topology"),
    )
    .with_metadata_path(child.clone())
    .into()
}

fn typed_child_collection_topology_failure(
    owner: &MetadataAddress,
    collection: MetaCollection,
) -> MetaFailure {
    typed_diagnostic(
        MetaDiagnosticCode::ProviderUnavailable,
        format!(
            "typed child collection topology is unavailable: {}.{}",
            owner,
            collection.as_str()
        ),
        Some("resources.child.collection"),
    )
    .with_metadata_path(owner.clone())
    .into()
}

fn typed_child_collection_guard_failure(owner: &MetadataAddress) -> MetaFailure {
    typed_diagnostic(
        MetaDiagnosticCode::ProviderUnavailable,
        format!("typed child collection topology is unavailable: {owner}"),
        Some("resources.child.collection"),
    )
    .with_metadata_path(owner.clone())
    .into()
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
        MetaCollection::PredefinedItems => {
            unreachable!("predefined items are not descriptor child objects")
        }
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

#[derive(Debug, Clone, Copy)]
enum TypedChildTreeError {
    Unavailable,
    SymbolicLink,
    UnsupportedNode,
}

impl TypedChildTreeError {
    fn public_message(self) -> &'static str {
        match self {
            Self::Unavailable => "typed child resource topology is unavailable",
            Self::SymbolicLink => "typed child resource topology contains a symbolic link",
            Self::UnsupportedNode => {
                "typed child resource topology contains an unsupported filesystem node"
            }
        }
    }
}

type TypedChildTree = (Vec<(PathBuf, Vec<u8>)>, Vec<PathBuf>);

const TYPED_CHILD_TREE_MAX_DEPTH: usize = 64;
const TYPED_CHILD_TREE_MAX_ENTRIES: usize = 20_000;
const TYPED_CHILD_TREE_MAX_FILES: usize = 20_000;
const TYPED_CHILD_TREE_MAX_BYTES: usize = 64 * 1024 * 1024;
#[derive(Default)]
struct TypedChildTreeBudget {
    entries: usize,
    files: usize,
    bytes: usize,
}

fn read_typed_child_tree(root: &Path) -> Result<TypedChildTree, TypedChildTreeError> {
    fn visit(
        root: &Path,
        current: &Path,
        depth: usize,
        files: &mut Vec<(PathBuf, Vec<u8>)>,
        directories: &mut Vec<PathBuf>,
        budget: &mut TypedChildTreeBudget,
    ) -> Result<(), TypedChildTreeError> {
        if depth > TYPED_CHILD_TREE_MAX_DEPTH {
            return Err(TypedChildTreeError::Unavailable);
        }
        let entries = fs::read_dir(current).map_err(|_| TypedChildTreeError::Unavailable)?;
        for entry in entries {
            let entry = entry.map_err(|_| TypedChildTreeError::Unavailable)?;
            budget.entries = budget
                .entries
                .checked_add(1)
                .filter(|count| *count <= TYPED_CHILD_TREE_MAX_ENTRIES)
                .ok_or(TypedChildTreeError::Unavailable)?;
            let file_type = entry
                .file_type()
                .map_err(|_| TypedChildTreeError::Unavailable)?;
            if file_type.is_symlink() {
                return Err(TypedChildTreeError::SymbolicLink);
            }
            if file_type.is_dir() {
                directories.push(entry.path().strip_prefix(root).unwrap().to_path_buf());
                visit(root, &entry.path(), depth + 1, files, directories, budget)?;
            } else if file_type.is_file() {
                budget.files = budget
                    .files
                    .checked_add(1)
                    .filter(|count| *count <= TYPED_CHILD_TREE_MAX_FILES)
                    .ok_or(TypedChildTreeError::Unavailable)?;
                let path = entry.path();
                let remaining = TYPED_CHILD_TREE_MAX_BYTES
                    .checked_sub(budget.bytes)
                    .ok_or(TypedChildTreeError::Unavailable)?;
                let mut bytes = Vec::new();
                fs::File::open(&path)
                    .map_err(|_| TypedChildTreeError::Unavailable)?
                    .take((remaining as u64).saturating_add(1))
                    .read_to_end(&mut bytes)
                    .map_err(|_| TypedChildTreeError::Unavailable)?;
                if bytes.len() > remaining {
                    return Err(TypedChildTreeError::Unavailable);
                }
                budget.bytes += bytes.len();
                files.push((path.strip_prefix(root).unwrap().to_path_buf(), bytes));
            } else {
                return Err(TypedChildTreeError::UnsupportedNode);
            }
        }
        Ok(())
    }
    let mut files = Vec::new();
    let mut directories = vec![PathBuf::new()];
    visit(
        root,
        root,
        0,
        &mut files,
        &mut directories,
        &mut TypedChildTreeBudget::default(),
    )?;
    files.sort_by(|left, right| left.0.cmp(&right.0));
    directories.sort();
    Ok((files, directories))
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
    resolve_typed_metadata_object(
        &request.source_set,
        &request.metadata_path,
        "edit",
        context,
        cancellation,
    )
}

pub(crate) fn resolve_typed_metadata_object(
    source_set: &str,
    metadata_path: &MetadataAddress,
    operation: &str,
    context: &WorkspaceContext,
    cancellation: &CancellationToken,
) -> Result<ResolvedMetadataObject, MetaFailure> {
    if cancellation.is_cancelled() {
        return Err(typed_diagnostic(
            MetaDiagnosticCode::ProviderUnavailable,
            format!("metadata {operation} was cancelled before source resolution"),
            None,
        )
        .with_metadata_path(metadata_path.clone())
        .into());
    }
    let target = SourceTarget {
        source_set: source_set.to_string(),
        metadata_path: Some(metadata_path.clone()),
    };
    let resolution = resolve_platform_xml_target(context, &target, TargetKindPolicy::Any)
        .map_err(|error| typed_resolution_failure(metadata_path, operation, error.code))?;
    if resolution.resolved.target_kind != TargetKind::MetadataObject {
        return Err(typed_diagnostic(
            MetaDiagnosticCode::InvalidArguments,
            "metadataPath must identify one existing metadata object",
            Some("metadataPath"),
        )
        .with_metadata_path(metadata_path.clone())
        .into());
    }
    let resolved_metadata_path = resolution.resolved.metadata_path.clone().ok_or_else(|| {
        MetaFailure::from(
            typed_diagnostic(
                MetaDiagnosticCode::ProviderUnavailable,
                "resolved metadata object has no logical identity",
                Some("metadataPath"),
            )
            .with_metadata_path(metadata_path.clone()),
        )
    })?;
    let evidence = platform_xml_resource_evidence(context, &resolution.handle)
        .map_err(|error| typed_resolution_failure(metadata_path, operation, error.code))?;
    let descriptor_preimage = fs::read(&evidence.target_path).map_err(|_| {
        MetaFailure::from(
            typed_diagnostic(
                MetaDiagnosticCode::ProviderUnavailable,
                "metadata descriptor image is unavailable",
                None,
            )
            .with_metadata_path(metadata_path.clone()),
        )
    })?;
    let owner_preimage = fs::read(&evidence.registration_path).map_err(|_| {
        MetaFailure::from(
            typed_diagnostic(
                MetaDiagnosticCode::ProviderUnavailable,
                "metadata owner image is unavailable",
                None,
            )
            .with_metadata_path(metadata_path.clone()),
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
                .with_metadata_path(metadata_path.clone()),
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
        .with_metadata_path(metadata_path.clone())
        .into());
    }
    Ok(ResolvedMetadataObject {
        handle: resolution.handle,
        resolved_target: resolution.resolved,
        metadata_path: resolved_metadata_path,
        descriptor_path: evidence.target_path,
        descriptor_preimage,
        source_root: evidence.source_root,
        owner_path: evidence.registration_path,
        owner_preimage,
    })
}

fn typed_resolution_failure(
    metadata_path: &MetadataAddress,
    operation: &str,
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
            MetaDiagnosticCode::InvalidArguments => "metadata target is not a mutable object",
            MetaDiagnosticCode::ProviderUnavailable => {
                "metadata target could not be resolved safely"
            }
            _ => match operation {
                "remove" => "metadata target does not provide typed Platform XML removal",
                _ => "metadata target does not provide typed Platform XML editing",
            },
        },
        Some("metadataPath"),
    )
    .with_metadata_path(metadata_path.clone())
    .into()
}

pub(crate) fn prepare_typed_edit(
    request: &MetaEditRequest,
    resolved: ResolvedMetadataObject,
    context: &WorkspaceContext,
) -> Result<Box<dyn PreparedMetadataMutation>, MetaFailure> {
    let target = request.metadata_path.clone();
    let diagnostics = Vec::new();
    match evaluate_resolved_support_guard(
        &resolved.descriptor_path,
        crate::application::SupportGuardRequirement::Editable,
        context,
    ) {
        ResolvedSupportGuardCheck::Allow | ResolvedSupportGuardCheck::Warn(_) => {}
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
    let TypedOperationPostImage {
        descriptor: post_image,
        child_resources,
        effects,
    } = build_typed_operation_post_image(
        TypedOperationDependencyScope::new(
            &request.source_set,
            &resolved.source_root,
            &resolved.owner_path,
            &resolved.owner_preimage,
            true,
        ),
        &resolved.descriptor_path,
        &request.metadata_path,
        &resolved.descriptor_preimage,
        &request.operations,
        context,
    )?;
    PreparedMetaEdit::prepare(
        request,
        resolved,
        context,
        post_image,
        diagnostics,
        child_resources,
        effects,
    )
}

fn restore_initial_empty_top_child_objects(pre_image: &str, post_image: &mut String) {
    let Ok(pre_document) = Document::parse(pre_image) else {
        return;
    };
    let Ok(pre_object) = meta_edit_object_node(&pre_document) else {
        return;
    };
    let Some(pre_children) = meta_info_child(pre_object, "ChildObjects") else {
        return;
    };
    let pre_range = pre_children.range();
    let pre_source = &pre_image[pre_range.clone()];
    if pre_children.children().any(|child| child.is_element()) {
        return;
    }

    let Ok(post_document) = Document::parse(post_image.as_str()) else {
        return;
    };
    let Ok(post_object) = meta_edit_object_node(&post_document) else {
        return;
    };
    let Some(post_children) = meta_info_child(post_object, "ChildObjects") else {
        return;
    };
    if post_children.children().any(|child| child.is_element()) {
        return;
    }
    let post_range = post_children.range();
    drop(post_document);
    post_image.replace_range(post_range, pre_source);
}

fn resolve_typed_relation_dependencies(
    dependency_scope: &TypedOperationDependencyScope<'_>,
    metadata_path: &MetadataAddress,
    operations: &[MetaEditOperation],
    context: &WorkspaceContext,
    owner_post_image: &str,
) -> Result<Vec<TypedRelationDependency>, MetaFailure> {
    let mut dependencies = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let mut request_origins = BTreeMap::new();
    for (operation_index, operation) in operations.iter().enumerate() {
        let MetaEditOperation::EditRelations {
            relation, targets, ..
        } = operation
        else {
            continue;
        };
        for (target_index, target) in targets.iter().enumerate() {
            request_origins.insert(
                (relation.as_str(), target.wire_value().to_string()),
                (operation_index, target_index),
            );
        }
    }
    let final_graph = parse_typed_final_relation_graph(owner_post_image, metadata_path)?;
    let mut source_prefixes = BTreeMap::<String, BTreeSet<String>>::new();
    for (_, relation_index, target) in &final_graph {
        let crate::domain::metadata::MetaRelationTarget::EventSource(source) = target else {
            continue;
        };
        let Some(dependency) = source.metadata_path() else {
            continue;
        };
        let generated_prefix = event_source_generated_prefix(source).map_err(|message| {
            MetaFailure::from(
                typed_diagnostic(
                    MetaDiagnosticCode::InvalidArguments,
                    message,
                    Some(&format!("relations.source[{relation_index}]")),
                )
                .with_metadata_path(metadata_path.clone()),
            )
        })?;
        source_prefixes
            .entry(dependency.as_str().to_string())
            .or_default()
            .insert(generated_prefix.to_string());
    }
    for (relation, relation_index, target) in final_graph {
        let origin = request_origins
            .get(&(relation.as_str(), target.wire_value().to_string()))
            .copied();
        let final_field = format!("relations.{}[{relation_index}]", relation.as_str());
        let diagnostic_field = origin.map_or_else(
            || final_field.clone(),
            |(operation_index, target_index)| {
                format!("operations[{operation_index}].targets[{target_index}]")
            },
        );
        validate_typed_relation_target(metadata_path, owner_post_image, relation, &target)
            .map_err(|mut diagnostic| {
                diagnostic.operation_index = origin.map(|(operation_index, _)| operation_index);
                diagnostic.field = Some(diagnostic_field.clone());
                MetaFailure::from(diagnostic.with_metadata_path(metadata_path.clone()))
            })?;
        let Some(dependency) = target.dependency() else {
            continue;
        };
        if dependency == metadata_path || !seen.insert(dependency.clone()) {
            continue;
        }
        let source_target = SourceTarget {
            source_set: dependency_scope.source_set.to_string(),
            metadata_path: Some(dependency.clone()),
        };
        let resolution =
            resolve_platform_xml_target(context, &source_target, TargetKindPolicy::Any).map_err(
                |_| {
                    let mut diagnostic = typed_diagnostic(
                        MetaDiagnosticCode::TargetNotFound,
                        "relation target does not resolve in the selected source set",
                        Some(&diagnostic_field),
                    );
                    diagnostic.operation_index = origin.map(|(index, _)| index);
                    MetaFailure::from(diagnostic.with_metadata_path(metadata_path.clone()))
                },
            )?;
        if resolution.resolved.target_kind != TargetKind::MetadataObject {
            let mut diagnostic = typed_diagnostic(
                MetaDiagnosticCode::InvalidArguments,
                "relation target must identify a metadata object",
                Some(&diagnostic_field),
            );
            diagnostic.operation_index = origin.map(|(index, _)| index);
            return Err(MetaFailure::from(
                diagnostic.with_metadata_path(metadata_path.clone()),
            ));
        }
        let evidence =
            platform_xml_resource_evidence(context, &resolution.handle).map_err(|_| {
                let mut diagnostic = typed_diagnostic(
                    MetaDiagnosticCode::ProviderUnavailable,
                    "relation target evidence is unavailable",
                    Some(&diagnostic_field),
                );
                diagnostic.operation_index = origin.map(|(index, _)| index);
                MetaFailure::from(diagnostic.with_metadata_path(metadata_path.clone()))
            })?;
        let bytes = fs::read(&evidence.target_path).map_err(|_| {
            let mut diagnostic = typed_diagnostic(
                MetaDiagnosticCode::ProviderUnavailable,
                "relation target pre-image is unavailable",
                Some(&diagnostic_field),
            );
            diagnostic.operation_index = origin.map(|(index, _)| index);
            MetaFailure::from(diagnostic.with_metadata_path(metadata_path.clone()))
        })?;
        if relation == MetaRelation::Source {
            if evidence.source_root != dependency_scope.owner_source_root
                || evidence.registration_path != dependency_scope.owner_registration_path
            {
                let mut diagnostic = typed_diagnostic(
                    MetaDiagnosticCode::InvalidArguments,
                    "EventSubscription Source dependency must belong to the exact owner source set",
                    Some(&diagnostic_field),
                );
                diagnostic.operation_index = origin.map(|(index, _)| index);
                return Err(MetaFailure::from(
                    diagnostic.with_metadata_path(metadata_path.clone()),
                ));
            }
            let (
                registration_kind,
                registration_name,
                descriptor_kind,
                descriptor_name,
                generated_name,
            ) = event_source_dependency_contract(dependency).map_err(|_| {
                let mut diagnostic = typed_diagnostic(
                    MetaDiagnosticCode::InvalidArguments,
                    "EventSubscription Source dependency must identify one metadata object",
                    Some(&diagnostic_field),
                );
                diagnostic.operation_index = origin.map(|(index, _)| index);
                MetaFailure::from(diagnostic.with_metadata_path(metadata_path.clone()))
            })?;
            validate_event_source_registration(
                dependency_scope.owner_registration_preimage,
                registration_kind,
                registration_name,
            )
            .and_then(|()| {
                let prefixes = source_prefixes
                    .get(dependency.as_str())
                    .map(|prefixes| prefixes.iter().cloned().collect::<Vec<_>>())
                    .unwrap_or_default();
                validate_event_source_dependency_descriptor(
                    &bytes,
                    descriptor_kind,
                    descriptor_name,
                    generated_name.as_deref(),
                    &prefixes,
                )
            })
            .map_err(|message| {
                let mut diagnostic = typed_diagnostic(
                    MetaDiagnosticCode::ValidationFailed,
                    message,
                    Some(&diagnostic_field),
                );
                diagnostic.operation_index = origin.map(|(index, _)| index);
                MetaFailure::from(diagnostic.with_metadata_path(metadata_path.clone()))
            })?;
        }
        dependencies.push(TypedRelationDependency {
            handle: resolution.handle,
            path: evidence.target_path,
            bytes,
            target: dependency.clone(),
            modules: Vec::new(),
        });
    }
    let defined_types = dependencies
        .iter()
        .filter(|dependency| dependency.target.segments().next() == Some("DefinedType"))
        .map(|dependency| dependency.target.clone())
        .collect::<Vec<_>>();
    let mut expanded = std::collections::HashSet::new();
    for defined_type in defined_types {
        expand_defined_type_event_dependencies(
            dependency_scope,
            metadata_path,
            context,
            &defined_type,
            &mut dependencies,
            &mut seen,
            &mut expanded,
            &mut std::collections::HashSet::new(),
        )?;
    }
    if dependency_scope.include_handler_dependency
        && metadata_path.segments().next() == Some("EventSubscription")
    {
        let handler = resolve_typed_event_subscription_handler_dependency(
            dependency_scope.source_set,
            dependency_scope.owner_source_root,
            dependency_scope.owner_registration_path,
            metadata_path,
            owner_post_image,
            context,
        )?;
        if !seen.contains(&handler.target) {
            dependencies.push(handler);
        }
    }
    Ok(dependencies)
}

#[allow(clippy::too_many_arguments)]
fn expand_defined_type_event_dependencies(
    dependency_scope: &TypedOperationDependencyScope<'_>,
    owner: &MetadataAddress,
    context: &WorkspaceContext,
    defined_type: &MetadataAddress,
    dependencies: &mut Vec<TypedRelationDependency>,
    seen: &mut std::collections::HashSet<MetadataAddress>,
    expanded: &mut std::collections::HashSet<MetadataAddress>,
    visiting: &mut std::collections::HashSet<MetadataAddress>,
) -> Result<(), MetaFailure> {
    if expanded.contains(defined_type) {
        return Ok(());
    }
    if !visiting.insert(defined_type.clone()) {
        return Err(defined_type_event_failure(
            owner,
            format!("DefinedType event source cycle contains `{defined_type}`"),
            MetaDiagnosticCode::ValidationFailed,
        ));
    }
    let bytes = dependencies
        .iter()
        .find(|dependency| &dependency.target == defined_type)
        .map(|dependency| dependency.bytes.clone())
        .ok_or_else(|| {
            defined_type_event_failure(
                owner,
                format!("DefinedType event source `{defined_type}` evidence is unavailable"),
                MetaDiagnosticCode::ProviderUnavailable,
            )
        })?;
    let members = parse_defined_type_event_sources(&bytes).map_err(|message| {
        defined_type_event_failure(owner, message, MetaDiagnosticCode::ValidationFailed)
    })?;
    for member in members {
        let source_class = member.event_source_class().map_err(|message| {
            defined_type_event_failure(owner, message, MetaDiagnosticCode::ValidationFailed)
        })?;
        let Some(target) = member.metadata_path().cloned() else {
            if source_class.is_some() {
                continue;
            }
            return Err(defined_type_event_failure(
                owner,
                "DefinedType member has no event source class",
                MetaDiagnosticCode::ValidationFailed,
            ));
        };
        if visiting.contains(&target) {
            return Err(defined_type_event_failure(
                owner,
                format!("DefinedType event source cycle contains `{target}`"),
                MetaDiagnosticCode::ValidationFailed,
            ));
        }
        if seen.insert(target.clone()) {
            let source_target = SourceTarget {
                source_set: dependency_scope.source_set.to_string(),
                metadata_path: Some(target.clone()),
            };
            let resolution =
                resolve_platform_xml_target(context, &source_target, TargetKindPolicy::Any)
                    .map_err(|_| {
                        defined_type_event_failure(
                            owner,
                            format!(
                        "DefinedType member `{target}` does not resolve in the selected source set"
                    ),
                            MetaDiagnosticCode::TargetNotFound,
                        )
                    })?;
            let evidence =
                platform_xml_resource_evidence(context, &resolution.handle).map_err(|_| {
                    defined_type_event_failure(
                        owner,
                        format!("DefinedType member `{target}` evidence is unavailable"),
                        MetaDiagnosticCode::ProviderUnavailable,
                    )
                })?;
            if evidence.source_root != dependency_scope.owner_source_root
                || evidence.registration_path != dependency_scope.owner_registration_path
            {
                return Err(defined_type_event_failure(
                    owner,
                    "DefinedType member must belong to the exact owner source set",
                    MetaDiagnosticCode::InvalidArguments,
                ));
            }
            let member_bytes = fs::read(&evidence.target_path).map_err(|_| {
                defined_type_event_failure(
                    owner,
                    format!("DefinedType member `{target}` pre-image is unavailable"),
                    MetaDiagnosticCode::ProviderUnavailable,
                )
            })?;
            let (
                registration_kind,
                registration_name,
                descriptor_kind,
                descriptor_name,
                generated_name,
            ) = event_source_dependency_contract(&target).map_err(|message| {
                defined_type_event_failure(owner, message, MetaDiagnosticCode::InvalidArguments)
            })?;
            let generated_prefix = event_source_generated_prefix(&member).map_err(|message| {
                defined_type_event_failure(owner, message, MetaDiagnosticCode::InvalidArguments)
            })?;
            validate_event_source_registration(
                dependency_scope.owner_registration_preimage,
                registration_kind,
                registration_name,
            )
            .and_then(|()| {
                validate_event_source_dependency_descriptor(
                    &member_bytes,
                    descriptor_kind,
                    descriptor_name,
                    generated_name.as_deref(),
                    &[generated_prefix.to_string()],
                )
            })
            .map_err(|message| {
                defined_type_event_failure(owner, message, MetaDiagnosticCode::ValidationFailed)
            })?;
            dependencies.push(TypedRelationDependency {
                handle: resolution.handle,
                path: evidence.target_path,
                bytes: member_bytes,
                target: target.clone(),
                modules: Vec::new(),
            });
        }
        if target.segments().next() == Some("DefinedType") {
            expand_defined_type_event_dependencies(
                dependency_scope,
                owner,
                context,
                &target,
                dependencies,
                seen,
                expanded,
                visiting,
            )?;
        }
    }
    visiting.remove(defined_type);
    expanded.insert(defined_type.clone());
    Ok(())
}

fn defined_type_event_failure(
    owner: &MetadataAddress,
    message: impl Into<String>,
    code: MetaDiagnosticCode,
) -> MetaFailure {
    MetaFailure::from(
        typed_diagnostic(code, message, Some("relations.source")).with_metadata_path(owner.clone()),
    )
}

fn resolve_typed_event_subscription_handler_dependency(
    source_set: &str,
    owner_source_root: &Path,
    owner_registration_path: &Path,
    metadata_path: &MetadataAddress,
    owner_post_image: &str,
    context: &WorkspaceContext,
) -> Result<TypedRelationDependency, MetaFailure> {
    let document =
        Document::parse(owner_post_image.trim_start_matches('\u{feff}')).map_err(|_| {
            MetaFailure::from(
                typed_diagnostic(
                    MetaDiagnosticCode::ProviderUnavailable,
                    "EventSubscription post-image is not valid XML",
                    Some("properties.handler"),
                )
                .with_metadata_path(metadata_path.clone()),
            )
        })?;
    let source = meta_event_subscription_source_node(&document).map_err(|message| {
        MetaFailure::from(
            typed_diagnostic(
                MetaDiagnosticCode::ProviderUnavailable,
                message,
                Some("properties.handler"),
            )
            .with_metadata_path(metadata_path.clone()),
        )
    })?;
    let properties = source.parent().ok_or_else(|| {
        MetaFailure::from(
            typed_diagnostic(
                MetaDiagnosticCode::ProviderUnavailable,
                "EventSubscription Properties are unavailable",
                Some("properties.handler"),
            )
            .with_metadata_path(metadata_path.clone()),
        )
    })?;
    let handler = meta_info_child_text(properties, "Handler")
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| {
            MetaFailure::from(
                typed_diagnostic(
                    MetaDiagnosticCode::ValidationFailed,
                    "EventSubscription Handler is empty",
                    Some("properties.handler"),
                )
                .with_metadata_path(metadata_path.clone()),
            )
        })?;
    let parts = handler.trim().split('.').collect::<Vec<_>>();
    let module_name = match parts.as_slice() {
        ["CommonModule", module, _] => *module,
        _ => {
            return Err(MetaFailure::from(
                typed_diagnostic(
                    MetaDiagnosticCode::ValidationFailed,
                    "EventSubscription Handler must identify a CommonModule procedure",
                    Some("properties.handler"),
                )
                .with_metadata_path(metadata_path.clone()),
            ))
        }
    };
    let target = MetadataAddress::parse(
        PLATFORM_XML_8_3_27_FORMAT_2_20,
        &format!("CommonModule.{module_name}"),
    )
    .map_err(|_| {
        MetaFailure::from(
            typed_diagnostic(
                MetaDiagnosticCode::ValidationFailed,
                "EventSubscription Handler CommonModule is invalid",
                Some("properties.handler"),
            )
            .with_metadata_path(metadata_path.clone()),
        )
    })?;
    let source_target = SourceTarget {
        source_set: source_set.to_string(),
        metadata_path: Some(target.clone()),
    };
    let resolution = resolve_platform_xml_target(context, &source_target, TargetKindPolicy::Any)
        .map_err(|_| {
            MetaFailure::from(
                typed_diagnostic(
                    MetaDiagnosticCode::TargetNotFound,
                    "EventSubscription Handler CommonModule does not resolve in the selected source set",
                    Some("properties.handler"),
                )
                .with_metadata_path(metadata_path.clone()),
            )
        })?;
    if resolution.resolved.target_kind != TargetKind::MetadataObject {
        return Err(MetaFailure::from(
            typed_diagnostic(
                MetaDiagnosticCode::InvalidArguments,
                "EventSubscription Handler must resolve to a metadata object",
                Some("properties.handler"),
            )
            .with_metadata_path(metadata_path.clone()),
        ));
    }
    let evidence = platform_xml_resource_evidence(context, &resolution.handle).map_err(|_| {
        MetaFailure::from(
            typed_diagnostic(
                MetaDiagnosticCode::ProviderUnavailable,
                "EventSubscription Handler evidence is unavailable",
                Some("properties.handler"),
            )
            .with_metadata_path(metadata_path.clone()),
        )
    })?;
    if evidence.source_root != owner_source_root
        || evidence.registration_path != owner_registration_path
    {
        return Err(MetaFailure::from(
            typed_diagnostic(
                MetaDiagnosticCode::InvalidArguments,
                "EventSubscription Handler must belong to the exact owner source set",
                Some("properties.handler"),
            )
            .with_metadata_path(metadata_path.clone()),
        ));
    }
    let bytes = fs::read(&evidence.target_path).map_err(|_| {
        MetaFailure::from(
            typed_diagnostic(
                MetaDiagnosticCode::ProviderUnavailable,
                "EventSubscription Handler descriptor pre-image is unavailable",
                Some("properties.handler"),
            )
            .with_metadata_path(metadata_path.clone()),
        )
    })?;
    let module_address = MetadataAddress::parse(
        PLATFORM_XML_8_3_27_FORMAT_2_20,
        &format!("CommonModule.{module_name}.Module"),
    )
    .expect("a registered common module has a canonical module address");
    let module_source_target = SourceTarget {
        source_set: source_set.to_string(),
        metadata_path: Some(module_address),
    };
    let module_resolution =
        resolve_platform_xml_target(context, &module_source_target, TargetKindPolicy::ModuleOnly)
            .map_err(|_| {
            MetaFailure::from(
                typed_diagnostic(
                    MetaDiagnosticCode::TargetNotFound,
                    "EventSubscription Handler module does not resolve in the selected source set",
                    Some("properties.handler"),
                )
                .with_metadata_path(metadata_path.clone()),
            )
        })?;
    let module_evidence = platform_xml_resource_evidence(context, &module_resolution.handle)
        .map_err(|_| {
            MetaFailure::from(
                typed_diagnostic(
                    MetaDiagnosticCode::ProviderUnavailable,
                    "EventSubscription Handler module evidence is unavailable",
                    Some("properties.handler"),
                )
                .with_metadata_path(metadata_path.clone()),
            )
        })?;
    if module_evidence.source_root != owner_source_root
        || module_evidence.registration_path != owner_registration_path
        || evidence.module_paths.as_slice() != [module_evidence.target_path.as_path()]
    {
        return Err(MetaFailure::from(
            typed_diagnostic(
                MetaDiagnosticCode::InvalidArguments,
                "EventSubscription Handler must have exactly one module in the owner source set",
                Some("properties.handler"),
            )
            .with_metadata_path(metadata_path.clone()),
        ));
    }
    let module_bytes = fs::read(&module_evidence.target_path).map_err(|_| {
        MetaFailure::from(
            typed_diagnostic(
                MetaDiagnosticCode::ProviderUnavailable,
                "EventSubscription Handler module pre-image is unavailable",
                Some("properties.handler"),
            )
            .with_metadata_path(metadata_path.clone()),
        )
    })?;
    let modules = vec![TypedDependencyModule {
        handle: module_resolution.handle,
        path: module_evidence.target_path,
        bytes: module_bytes,
    }];
    Ok(TypedRelationDependency {
        handle: resolution.handle,
        path: evidence.target_path,
        bytes,
        target,
        modules,
    })
}

fn parse_typed_final_relation_graph(
    owner_post_image: &str,
    owner: &MetadataAddress,
) -> Result<
    Vec<(
        MetaRelation,
        usize,
        crate::domain::metadata::MetaRelationTarget,
    )>,
    MetaFailure,
> {
    let document =
        Document::parse(owner_post_image.trim_start_matches('\u{feff}')).map_err(|_| {
            MetaFailure::from(
                typed_diagnostic(
                    MetaDiagnosticCode::ProviderUnavailable,
                    "owner post-image is not valid XML",
                    Some("relations"),
                )
                .with_metadata_path(owner.clone()),
            )
        })?;
    let object = meta_edit_object_node(&document).map_err(|_| {
        MetaFailure::from(
            typed_diagnostic(
                MetaDiagnosticCode::ProviderUnavailable,
                "owner post-image object is unavailable",
                Some("relations"),
            )
            .with_metadata_path(owner.clone()),
        )
    })?;
    let properties = meta_info_child(object, "Properties").ok_or_else(|| {
        MetaFailure::from(
            typed_diagnostic(
                MetaDiagnosticCode::ProviderUnavailable,
                "owner post-image properties are unavailable",
                Some("relations"),
            )
            .with_metadata_path(owner.clone()),
        )
    })?;
    let mut graph = Vec::new();
    for (relation, tag) in [
        (MetaRelation::Owners, "Owners"),
        (MetaRelation::RegisterRecords, "RegisterRecords"),
        (MetaRelation::BasedOn, "BasedOn"),
        (MetaRelation::InputByString, "InputByString"),
    ] {
        let Some(container) = meta_info_child(properties, tag) else {
            continue;
        };
        for (index, item) in container
            .children()
            .filter(|node| node.is_element())
            .enumerate()
        {
            let value = item.text().unwrap_or_default().trim();
            let target = if relation == MetaRelation::InputByString {
                crate::domain::metadata::MetaRelationTarget::Field(
                    crate::domain::metadata::MetadataFieldPath::parse(value).map_err(
                        |mut diagnostic| {
                            diagnostic.field =
                                Some(format!("relations.{}[{index}]", relation.as_str()));
                            MetaFailure::from(diagnostic.with_metadata_path(owner.clone()))
                        },
                    )?,
                )
            } else {
                let metadata_path = MetadataAddress::parse(PLATFORM_XML_8_3_27_FORMAT_2_20, value)
                    .map_err(|_| {
                        MetaFailure::from(
                            typed_diagnostic(
                                MetaDiagnosticCode::ValidationFailed,
                                "final relation target is not a metadata address",
                                Some(&format!("relations.{}[{index}]", relation.as_str())),
                            )
                            .with_metadata_path(owner.clone()),
                        )
                    })?;
                crate::domain::metadata::MetaRelationTarget::Object(
                    crate::domain::metadata::MetadataReference { metadata_path },
                )
            };
            graph.push((relation, index, target));
        }
    }
    if owner.segments().next() == Some("EventSubscription") {
        let source = meta_event_subscription_source_node(&document)
            .and_then(parse_meta_event_subscription_source)
            .map_err(|message| {
                MetaFailure::from(
                    typed_diagnostic(
                        MetaDiagnosticCode::ValidationFailed,
                        message,
                        Some("relations.source"),
                    )
                    .with_metadata_path(owner.clone()),
                )
            })?;
        for (index, source) in source.into_iter().enumerate() {
            graph.push((
                MetaRelation::Source,
                index,
                crate::domain::metadata::MetaRelationTarget::EventSource(source),
            ));
        }
    }
    Ok(graph)
}

fn validate_typed_relation_target(
    owner: &MetadataAddress,
    owner_post_image: &str,
    relation: MetaRelation,
    target: &crate::domain::metadata::MetaRelationTarget,
) -> Result<(), MetaDiagnostic> {
    let owner_kind = owner
        .segments()
        .next()
        .and_then(|kind| MetadataKind::parse(kind).ok())
        .ok_or_else(|| {
            typed_diagnostic(
                MetaDiagnosticCode::ProviderUnavailable,
                "metadata owner kind is unavailable",
                Some("targets"),
            )
        })?;
    validate_metadata_relation_target_profile(owner_kind, owner, relation, target)?;
    if relation == MetaRelation::InputByString {
        let crate::domain::metadata::MetaRelationTarget::Field(field) = target else {
            unreachable!("domain relation profile rejects non-field inputByString targets")
        };
        let target_kind = owner_kind.as_str();
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
        } else if !metadata_standard_attribute_names(target_kind).contains(&field.name.as_str()) {
            return Err(typed_diagnostic(
                MetaDiagnosticCode::TargetNotFound,
                "inputByString standard attribute does not exist for the owner kind",
                Some("targets"),
            ));
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
        if matches!(
            operation,
            MetaEditOperation::AddPredefinedItems { .. }
                | MetaEditOperation::UpdatePredefinedItems { .. }
                | MetaEditOperation::RemovePredefinedItems { .. }
        ) {
            continue;
        }
        // Capture before applying, but let the writer own invalid-operation
        // diagnostics. A missing remove/update target must stay target_not_found
        // rather than being replaced by an effect-projection failure.
        let before = match typed_operation_effect_value(&working, operation, false) {
            Err(_)
                if matches!(
                    operation,
                    MetaEditOperation::EditRelations {
                        relation: MetaRelation::Source,
                        mode: RelationEditMode::Replace,
                        ..
                    }
                ) =>
            {
                Ok(Some(Value::Null))
            }
            other => other,
        };
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
        let before = before.map_err(|diagnostic| {
            MetaFailure::from(diagnostic.with_operation_index(operation_index))
        })?;
        let after =
            typed_operation_effect_value(&working, operation, true).map_err(|diagnostic| {
                MetaFailure::from(diagnostic.with_operation_index(operation_index))
            })?;
        counts.effects.push(MetaMutationEffect {
            operation_index: Some(operation_index as u64),
            operation: typed_operation_name(operation).to_string(),
            target: typed_operation_target(&working, operation)?,
            before,
            after,
        });
    }
    *xml_text = working;
    Ok(counts)
}

fn typed_operation_name(operation: &MetaEditOperation) -> &'static str {
    match operation {
        MetaEditOperation::SetProperties { .. } => "setProperties",
        MetaEditOperation::Add { .. } => "add",
        MetaEditOperation::Update { .. } => "update",
        MetaEditOperation::Remove { .. } => "remove",
        MetaEditOperation::EditRelations { .. } => "editRelations",
        MetaEditOperation::AddHelp { .. } => "addHelp",
        MetaEditOperation::AddPredefinedItems { .. } => "add",
        MetaEditOperation::UpdatePredefinedItems { .. } => "update",
        MetaEditOperation::RemovePredefinedItems { .. } => "remove",
    }
}

fn typed_operation_target(xml: &str, operation: &MetaEditOperation) -> Result<String, MetaFailure> {
    let (kind, name) = meta_edit_object_identity(xml).map_err(|_| {
        MetaFailure::from(typed_diagnostic(
            MetaDiagnosticCode::ProviderUnavailable,
            "metadata descriptor identity is unavailable for semantic effect",
            None,
        ))
    })?;
    let base = format!("{kind}.{name}");
    Ok(match operation {
        MetaEditOperation::SetProperties { .. } => format!("{base}.properties"),
        MetaEditOperation::Add {
            collection, scope, ..
        }
        | MetaEditOperation::Update {
            collection, scope, ..
        }
        | MetaEditOperation::Remove {
            collection, scope, ..
        } => match scope {
            Some(scope) => format!(
                "{base}.collections.tabularSections.{}.{}",
                scope.tabular_section,
                collection.as_str()
            ),
            None => format!("{base}.collections.{}", collection.as_str()),
        },
        MetaEditOperation::EditRelations { relation, .. } => {
            format!("{base}.relations.{}", relation.as_str())
        }
        MetaEditOperation::AddHelp { .. } => format!("{base}.help"),
        MetaEditOperation::AddPredefinedItems { .. }
        | MetaEditOperation::UpdatePredefinedItems { .. }
        | MetaEditOperation::RemovePredefinedItems { .. } => {
            format!("{base}.collections.predefinedItems")
        }
    })
}

fn typed_operation_effect_value(
    xml: &str,
    operation: &MetaEditOperation,
    after: bool,
) -> Result<Option<Value>, MetaDiagnostic> {
    match operation {
        MetaEditOperation::Add { .. } if !after => return Ok(None),
        MetaEditOperation::Remove { .. } if after => return Ok(None),
        _ => {}
    }
    let (kind_name, object_name) = meta_edit_object_identity(xml).map_err(|_| {
        typed_diagnostic(
            MetaDiagnosticCode::ProviderUnavailable,
            "metadata descriptor identity is unavailable for semantic effect",
            None,
        )
    })?;
    let kind = crate::domain::metadata::MetadataKind::parse(&kind_name).map_err(|_| {
        typed_diagnostic(
            MetaDiagnosticCode::ProviderUnavailable,
            "metadata descriptor kind is unavailable for semantic effect",
            None,
        )
    })?;
    let target = MetadataAddress::parse(
        PLATFORM_XML_8_3_27_FORMAT_2_20,
        &format!("{kind_name}.{object_name}"),
    )
    .map_err(|_| {
        typed_diagnostic(
            MetaDiagnosticCode::ProviderUnavailable,
            "metadata descriptor address is unavailable for semantic effect",
            None,
        )
    })?;
    let document = Document::parse(xml.trim_start_matches('\u{feff}')).map_err(|_| {
        typed_diagnostic(
            MetaDiagnosticCode::ProviderUnavailable,
            "metadata descriptor is not valid XML",
            None,
        )
    })?;
    let object = meta_edit_object_node(&document).map_err(|_| {
        typed_diagnostic(
            MetaDiagnosticCode::ProviderUnavailable,
            "metadata object is unavailable for semantic effect",
            None,
        )
    })?;
    let properties = meta_info_child(object, "Properties");
    let value = match operation {
        MetaEditOperation::AddHelp { lang } => serde_json::json!({"lang": lang}),
        MetaEditOperation::SetProperties { values } => {
            let observed = super::info::typed_properties(properties, kind);
            let mut selected = JsonMap::new();
            for (key, _) in values.entries() {
                let public_key = serde_json::to_value(key)
                    .ok()
                    .and_then(|value| value.as_str().map(str::to_string))
                    .ok_or_else(|| {
                        typed_diagnostic(
                            MetaDiagnosticCode::ProviderUnavailable,
                            "metadata property name cannot be normalized",
                            Some("values"),
                        )
                    })?;
                let property = observed
                    .iter()
                    .find(|property| property.key == public_key)
                    .ok_or_else(|| {
                        typed_diagnostic(
                            MetaDiagnosticCode::ProviderUnavailable,
                            "metadata property is unavailable for semantic effect",
                            Some("values"),
                        )
                    })?;
                selected.insert(
                    public_key,
                    serde_json::to_value(&property.value).map_err(|_| {
                        typed_diagnostic(
                            MetaDiagnosticCode::ProviderUnavailable,
                            "metadata property value cannot be normalized",
                            Some("values"),
                        )
                    })?,
                );
            }
            Value::Object(selected)
        }
        MetaEditOperation::Add {
            collection,
            scope,
            elements,
        } => typed_collection_effect_value(
            xml,
            object,
            *collection,
            scope.as_ref(),
            elements.iter().map(|element| element.name.as_str()),
            &target,
        )?,
        MetaEditOperation::Update {
            collection,
            scope,
            elements,
        } => typed_collection_effect_value(
            xml,
            object,
            *collection,
            scope.as_ref(),
            elements.iter().map(|element| {
                if after {
                    element.new_name.as_deref().unwrap_or(&element.name)
                } else {
                    &element.name
                }
            }),
            &target,
        )?,
        MetaEditOperation::Remove {
            collection,
            scope,
            names,
        } => typed_collection_effect_value(
            xml,
            object,
            *collection,
            scope.as_ref(),
            names.iter().map(String::as_str),
            &target,
        )?,
        MetaEditOperation::EditRelations { relation, .. } => {
            let mut diagnostics = Vec::new();
            let relations = super::info::typed_relations(
                &document,
                properties,
                kind,
                &target,
                &mut diagnostics,
            );
            if let Some(diagnostic) = diagnostics.into_iter().next() {
                return Err(diagnostic);
            }
            match relation {
                MetaRelation::Owners => serde_json::to_value(relations.owners),
                MetaRelation::RegisterRecords => serde_json::to_value(relations.register_records),
                MetaRelation::BasedOn => serde_json::to_value(relations.based_on),
                MetaRelation::InputByString => serde_json::to_value(relations.input_by_string),
                MetaRelation::Source => serde_json::to_value(relations.source),
            }
            .map_err(|_| {
                typed_diagnostic(
                    MetaDiagnosticCode::ProviderUnavailable,
                    "metadata relations cannot be normalized",
                    Some("relation"),
                )
            })?
        }
        MetaEditOperation::AddPredefinedItems { .. }
        | MetaEditOperation::UpdatePredefinedItems { .. }
        | MetaEditOperation::RemovePredefinedItems { .. } => {
            return Err(typed_diagnostic(
                MetaDiagnosticCode::ProviderUnavailable,
                "predefined effects are projected by the predefined resource writer",
                None,
            ))
        }
    };
    Ok(Some(value))
}

fn typed_collection_effect_value<'a>(
    xml: &str,
    object: roxmltree::Node<'a, 'a>,
    collection: MetaCollection,
    scope: Option<&crate::domain::metadata::MetaScope>,
    names: impl Iterator<Item = &'a str>,
    target: &MetadataAddress,
) -> Result<Value, MetaDiagnostic> {
    let top = meta_info_child(object, "ChildObjects");
    let parent = if let Some(scope) = scope {
        let section = top
            .into_iter()
            .flat_map(|node| meta_info_children(node, "TabularSection"))
            .find(|node| {
                meta_edit_child_object_name(*node).is_some_and(|candidate| {
                    metadata_name_eq(&candidate, scope.tabular_section.as_str())
                })
            })
            .ok_or_else(|| {
                typed_diagnostic(
                    MetaDiagnosticCode::TargetNotFound,
                    "metadata effect scope was not found",
                    Some("scope"),
                )
            })?;
        meta_info_child(section, "ChildObjects")
    } else {
        top
    };
    let requested = names.collect::<Vec<_>>();
    let mut diagnostics = Vec::new();
    let elements = super::info::typed_elements_with_diagnostics(
        xml,
        parent,
        collection_tag(collection),
        collection == MetaCollection::TabularSections,
        "effect",
        target,
        &mut diagnostics,
    );
    if let Some(diagnostic) = diagnostics.into_iter().find(|diagnostic| {
        diagnostic.severity == crate::domain::metadata::MetaDiagnosticSeverity::Error
    }) {
        return Err(diagnostic);
    }
    let selected = elements
        .into_iter()
        .filter(|element| {
            requested
                .iter()
                .any(|requested| metadata_name_eq(element.name.as_str(), requested))
        })
        .collect::<Vec<_>>();
    if selected.len() != requested.len() {
        return Err(typed_diagnostic(
            MetaDiagnosticCode::ProviderUnavailable,
            "metadata element is unavailable for semantic effect",
            Some("elements"),
        ));
    }
    serde_json::to_value(selected).map_err(|_| {
        typed_diagnostic(
            MetaDiagnosticCode::ProviderUnavailable,
            "metadata elements cannot be normalized",
            Some("elements"),
        )
    })
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
    let owner_kind = MetadataKind::parse(&object_kind).map_err(|_| {
        typed_diagnostic(
            MetaDiagnosticCode::UnsupportedKind,
            format!("metadata kind `{object_kind}` has no typed operation profile"),
            None,
        )
    })?;
    let owner = MetadataAddress::parse(
        PLATFORM_XML_8_3_27_FORMAT_2_20,
        &format!("{object_kind}.{object_name}"),
    )
    .map_err(|_| {
        typed_diagnostic(
            MetaDiagnosticCode::ProviderUnavailable,
            "metadata descriptor identity is not a canonical logical address",
            None,
        )
    })?;
    validate_metadata_operation_capabilities(owner_kind, &owner, operation)?;
    match operation {
        // ADR-0072: справка живёт рядом с дескриптором, сам текст дескриптора
        // не меняется; файлы планирует `help_facet` из того же построения
        // post-image.
        MetaEditOperation::AddHelp { .. } => {
            counts.added += 1;
        }
        MetaEditOperation::SetProperties { values } => {
            apply_typed_properties(xml_text, values)?;
            counts.modified += values.entries().len();
        }
        MetaEditOperation::Add {
            collection,
            scope,
            elements,
        } => {
            ensure_typed_scope_exists(
                xml_text,
                scope.as_ref().map(|scope| scope.tabular_section.as_str()),
            )?;
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
            ensure_typed_scope_exists(
                xml_text,
                scope.as_ref().map(|scope| scope.tabular_section.as_str()),
            )?;
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
            ensure_typed_scope_exists(
                xml_text,
                scope.as_ref().map(|scope| scope.tabular_section.as_str()),
            )?;
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
            apply_typed_relations(xml_text, *relation, *mode, targets)?;
            counts.modified += 1;
        }
        MetaEditOperation::AddPredefinedItems { .. }
        | MetaEditOperation::UpdatePredefinedItems { .. }
        | MetaEditOperation::RemovePredefinedItems { .. } => {
            unreachable!("predefined operations are applied by their companion-resource writer")
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

fn ensure_typed_scope_exists(xml_text: &str, scope: Option<&str>) -> Result<(), MetaDiagnostic> {
    let Some(scope) = scope else {
        return Ok(());
    };
    let document = Document::parse(xml_text.trim_start_matches('\u{feff}')).map_err(|_| {
        typed_diagnostic(
            MetaDiagnosticCode::ProviderUnavailable,
            "metadata descriptor is not valid XML",
            None,
        )
    })?;
    let object = meta_edit_object_node(&document).map_err(|_| {
        typed_diagnostic(
            MetaDiagnosticCode::ProviderUnavailable,
            "metadata object is unavailable",
            None,
        )
    })?;
    if meta_edit_find_tabular_section(object, scope).is_some() {
        Ok(())
    } else {
        Err(typed_diagnostic(
            MetaDiagnosticCode::TargetNotFound,
            "tabular section scope was not found",
            Some("scope.tabularSection"),
        ))
    }
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
    let kind = MetadataKind::parse(object.tag_name().name()).map_err(|_| {
        typed_diagnostic(
            MetaDiagnosticCode::ProviderUnavailable,
            "metadata descriptor kind is unavailable",
            None,
        )
    })?;
    let range = properties.range();
    drop(doc);
    let mut text = xml_text[range.clone()].to_string();
    let indent = meta_edit_property_child_indent(&text);
    for (key, value) in changes.entries() {
        let tag = METADATA_PROPERTY_SPECS
            .iter()
            .find(|spec| spec.key == *key && spec.allowed_kinds.contains(&kind))
            .map(|spec| spec.xml_name)
            .ok_or_else(|| {
                typed_diagnostic(
                    MetaDiagnosticCode::ProviderUnavailable,
                    "metadata property registry is unavailable for this kind",
                    Some("values"),
                )
            })?;
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
        MetaCollection::PredefinedItems => {
            unreachable!("predefined items are not descriptor child objects")
        }
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
    let mut lines = render_typed_element(object_kind, object_name, collection, element)?;
    if matches!(
        collection,
        MetaCollection::Dimensions | MetaCollection::Resources
    ) {
        inherit_register_field_settings(xml_text, tag, &mut lines);
    }
    let lines = lines;
    let result = match scope {
        Some(section) => meta_edit_insert_tabular_child_object_with_position(
            xml_text, section, tag, &position, &lines,
        ),
        None => meta_edit_insert_top_child_object_with_position(xml_text, tag, &position, &lines),
    };
    result.map_err(|_| {
        let field = if element.position.is_some() {
            "position"
        } else if scope.is_some() {
            "scope"
        } else {
            "position"
        };
        typed_diagnostic(
            MetaDiagnosticCode::TargetNotFound,
            "metadata position or scope target was not found",
            Some(field),
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

/// Settings a new register field takes from the fields already in the register.
///
/// There is no single platform value to default to: a 8.3.27 vendor dump has
/// 2169 of 2490 information-register resources on `Use`/`Use`, 139 on
/// `DontUse`/`Use`, and 42 registers that disagree inside themselves. So the
/// value is inherited only where every sibling of the same tag already agrees,
/// and the template default stands otherwise. What #323 reported is the case
/// where the siblings do agree and the new field silently did not join them.
const INHERITED_REGISTER_FIELD_SETTINGS: &[&str] = &["Indexing", "FullTextSearch", "DataHistory"];

fn inherit_register_field_settings(xml: &str, tag: &str, rendered: &mut [String]) {
    for property in INHERITED_REGISTER_FIELD_SETTINGS {
        let Some(agreed) = agreed_sibling_setting(xml, tag, property) else {
            continue;
        };
        let open = format!("<{property}>");
        let close = format!("</{property}>");
        for line in rendered.iter_mut() {
            let Some(current) = element_text_between(line, &open, &close) else {
                continue;
            };
            if current != agreed {
                *line = line.replace(
                    &format!("{open}{current}{close}"),
                    &format!("{open}{agreed}{close}"),
                );
            }
        }
    }
}

/// The value every existing `<tag>` carries for `property`, or `None` when
/// there is no sibling or the siblings disagree.
fn agreed_sibling_setting(xml: &str, tag: &str, property: &str) -> Option<String> {
    let open_tag = format!("<{tag} uuid=");
    let close_tag = format!("</{tag}>");
    let open = format!("<{property}>");
    let close = format!("</{property}>");
    let mut agreed: Option<String> = None;
    let mut rest = xml;
    while let Some(start) = rest.find(&open_tag) {
        let block = &rest[start..];
        let end = block.find(&close_tag)?;
        let value = element_text_between(&block[..end], &open, &close)?;
        match &agreed {
            Some(seen) if seen != &value => return None,
            Some(_) => {}
            None => agreed = Some(value),
        }
        rest = &block[end + close_tag.len()..];
    }
    agreed
}

fn element_text_between(haystack: &str, open: &str, close: &str) -> Option<String> {
    let start = haystack.find(open)? + open.len();
    let end = haystack[start..].find(close)? + start;
    Some(haystack[start..end].to_string())
}

fn render_typed_element(
    object_kind: &str,
    object_name: &str,
    collection: MetaCollection,
    element: &MetaElementDefinition,
) -> Result<Vec<String>, MetaDiagnostic> {
    let mut lines = Vec::new();
    let mut next_uuid = fresh_metadata_uuid;
    let attr = || MetadataAttributeTemplate {
        name: element.name.clone(),
        synonym: element
            .synonym
            .clone()
            .unwrap_or_else(|| split_meta_camel_case(&element.name)),
        required: element.required.unwrap_or(false),
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
                .map(|attribute| MetadataAttributeTemplate {
                    name: attribute.name.clone(),
                    synonym: attribute
                        .synonym
                        .clone()
                        .unwrap_or_else(|| split_meta_camel_case(&attribute.name)),
                    required: attribute.required.unwrap_or(false),
                })
                .collect();
            emit_meta_tabular_section(
                &mut lines,
                "\t\t\t",
                &MetadataTabularSectionTemplate {
                    name: element.name.clone(),
                    columns,
                },
                object_kind,
                object_name,
                &mut next_uuid,
            );
            let mut rendered = lines.join("\n");
            apply_typed_element_fields(&mut rendered, element);
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
            &MetadataEnumValueTemplate {
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
        MetaCollection::PredefinedItems => {
            unreachable!("predefined items are rendered in Ext/Predefined.xml")
        }
    }
    let mut rendered = lines.join("\n");
    if collection == MetaCollection::Templates {
        if let Some(kind) = element.template_type {
            rendered = rendered.replace(
                "<TemplateType>SpreadsheetDocument</TemplateType>",
                &format!("<TemplateType>{}</TemplateType>", kind.as_str()),
            );
        }
    }
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
        .find(|child| {
            meta_edit_child_object_name(*child)
                .is_some_and(|candidate| metadata_name_eq(&candidate, name))
        })
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
            None => parse_typed_fill_value_at_element(xml_text, range.clone())?,
        };
        validate_metadata_element_value_profile(
            Some(metadata_type),
            fill_value.as_ref(),
            MetaValueProfileContext::NewElement,
        )
        .map_err(|mut diagnostic| {
            if update.fill_value.is_none() && diagnostic.field.as_deref() == Some("fillValue") {
                diagnostic.field = Some("type".to_string());
            }
            diagnostic
        })?;
    } else if let Some(fill_value) = &update.fill_value {
        let metadata_type = parse_typed_metadata_type_at_element(xml_text, range.clone())?;
        validate_metadata_element_value_profile(
            Some(&metadata_type),
            Some(fill_value),
            MetaValueProfileContext::NewElement,
        )?;
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

#[cfg(test)]
pub(super) fn parse_typed_fill_value(
    properties_text: &str,
) -> Result<Option<MetaFillValue>, MetaDiagnostic> {
    const WRAPPER_START: &str = r#"<Root xmlns="http://v8.1c.ru/8.3/MDClasses" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:xr="http://v8.1c.ru/8.3/xcf/readable" xmlns:v8="http://v8.1c.ru/8.1/data/core" xmlns:cfg="http://v8.1c.ru/8.1/data/enterprise/current-config">"#;
    let wrapped = format!("{WRAPPER_START}{properties_text}</Root>");
    let document = Document::parse(&wrapped).map_err(|_| {
        typed_diagnostic(
            MetaDiagnosticCode::ProviderUnavailable,
            "metadata element properties are not valid XML",
            Some("fillValue"),
        )
    })?;
    parse_typed_fill_value_node(document.root_element())
}

pub(super) fn parse_typed_fill_value_node(
    properties: roxmltree::Node<'_, '_>,
) -> Result<Option<MetaFillValue>, MetaDiagnostic> {
    const MD_NAMESPACE: &str = "http://v8.1c.ru/8.3/MDClasses";
    const XSI_NAMESPACE: &str = "http://www.w3.org/2001/XMLSchema-instance";
    const XSD_NAMESPACE: &str = "http://www.w3.org/2001/XMLSchema";
    const READABLE_NAMESPACE: &str = "http://v8.1c.ru/8.3/xcf/readable";
    let properties = if properties.tag_name().namespace() == Some(MD_NAMESPACE)
        && properties.tag_name().name() == "Properties"
    {
        properties
    } else {
        properties
            .children()
            .find(|node| {
                node.is_element()
                    && node.tag_name().namespace() == Some(MD_NAMESPACE)
                    && node.tag_name().name() == "Properties"
            })
            .unwrap_or(properties)
    };
    let fills = properties
        .children()
        .filter(|node| {
            node.is_element()
                && node.tag_name().namespace() == Some(MD_NAMESPACE)
                && node.tag_name().name() == "FillValue"
        })
        .collect::<Vec<_>>();
    let ([] | [_]) = fills.as_slice() else {
        return Err(typed_diagnostic(
            MetaDiagnosticCode::ProviderUnavailable,
            "metadata properties contain duplicate FillValue elements",
            Some("fillValue"),
        ));
    };
    let Some(fill) = fills.first().copied() else {
        return Ok(None);
    };
    if fill.children().any(|child| child.is_element()) {
        return Err(typed_diagnostic(
            MetaDiagnosticCode::ProviderUnavailable,
            "existing fill value contains unsupported nested markup",
            Some("fillValue"),
        ));
    }
    let nil = fill.attribute((XSI_NAMESPACE, "nil"));
    let value_type = fill.attribute((XSI_NAMESPACE, "type"));
    let value = super::info_projection::direct_text_content(fill);
    if nil == Some("true") {
        if fill.attributes().len() != 1 || value_type.is_some() || !value.trim().is_empty() {
            return Err(typed_diagnostic(
                MetaDiagnosticCode::ProviderUnavailable,
                "existing nil fill value contains unsupported structure",
                Some("fillValue"),
            ));
        }
        return Ok(None);
    }
    if fill.attributes().len() != 1 {
        return Err(typed_diagnostic(
            MetaDiagnosticCode::ProviderUnavailable,
            "existing fill value must contain exactly one xsi:type attribute",
            Some("fillValue"),
        ));
    }
    let value_type = value_type.ok_or_else(|| {
        typed_diagnostic(
            MetaDiagnosticCode::ProviderUnavailable,
            "existing fill value has no xsi:type attribute",
            Some("fillValue"),
        )
    })?;
    let (prefix, local_name) = value_type.split_once(':').ok_or_else(|| {
        typed_diagnostic(
            MetaDiagnosticCode::ValidationFailed,
            "existing fill value type is not a qualified name",
            Some("fillValue"),
        )
    })?;
    let namespace = fill.lookup_namespace_uri(Some(prefix)).ok_or_else(|| {
        typed_diagnostic(
            MetaDiagnosticCode::ValidationFailed,
            "existing fill value type prefix is not bound",
            Some("fillValue"),
        )
    })?;
    match (namespace, local_name) {
        (XSD_NAMESPACE, "string") => Ok(Some(MetaFillValue::String(value))),
        (XSD_NAMESPACE, "decimal") if metadata_decimal_shape(&value).is_some() => {
            Ok(Some(MetaFillValue::Number(value)))
        }
        (XSD_NAMESPACE, "decimal") => Err(typed_diagnostic(
            MetaDiagnosticCode::ValidationFailed,
            "existing numeric fill value is not canonical",
            Some("fillValue"),
        )),
        (XSD_NAMESPACE, "boolean") => match value.as_str() {
            "true" => Ok(Some(MetaFillValue::Boolean(true))),
            "false" => Ok(Some(MetaFillValue::Boolean(false))),
            _ => Err(typed_diagnostic(
                MetaDiagnosticCode::ValidationFailed,
                "existing boolean fill value is not canonical",
                Some("fillValue"),
            )),
        },
        (XSD_NAMESPACE, "dateTime") if metadata_xs_datetime_is_valid(&value) => {
            Ok(Some(MetaFillValue::DateTime(value)))
        }
        (XSD_NAMESPACE, "dateTime") => Err(typed_diagnostic(
            MetaDiagnosticCode::ValidationFailed,
            "existing date-time fill value is not canonical",
            Some("fillValue"),
        )),
        (READABLE_NAMESPACE, "DesignTimeRef") => Ok(Some(MetaFillValue::Reference(
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
        _ => Err(typed_diagnostic(
            MetaDiagnosticCode::ValidationFailed,
            "existing fill value type is unsupported by typed metadata edit",
            Some("fillValue"),
        )),
    }
}

fn parse_typed_fill_value_at_element(
    xml_text: &str,
    element_range: std::ops::Range<usize>,
) -> Result<Option<MetaFillValue>, MetaDiagnostic> {
    let document = Document::parse(xml_text.trim_start_matches('\u{feff}')).map_err(|_| {
        typed_diagnostic(
            MetaDiagnosticCode::ProviderUnavailable,
            "metadata descriptor is not valid XML",
            Some("fillValue"),
        )
    })?;
    let element = document
        .descendants()
        .find(|node| node.is_element() && node.range() == element_range)
        .ok_or_else(|| {
            typed_diagnostic(
                MetaDiagnosticCode::ProviderUnavailable,
                "metadata element is unavailable",
                Some("fillValue"),
            )
        })?;
    let properties =
        super::info_projection::direct_md_child(element, "Properties").ok_or_else(|| {
            typed_diagnostic(
                MetaDiagnosticCode::ProviderUnavailable,
                "metadata element has no Properties",
                Some("fillValue"),
            )
        })?;
    parse_typed_fill_value_node(properties)
}

#[cfg(test)]
pub(super) fn parse_typed_metadata_type(
    properties_text: &str,
) -> Result<MetadataType, MetaDiagnostic> {
    let observed = super::info_projection::parse_observed_metadata_type(properties_text)?;
    narrow_observed_metadata_type(observed)
}

fn parse_typed_metadata_type_at_element(
    xml_text: &str,
    element_range: std::ops::Range<usize>,
) -> Result<MetadataType, MetaDiagnostic> {
    let document = Document::parse(xml_text.trim_start_matches('\u{feff}')).map_err(|_| {
        typed_diagnostic(
            MetaDiagnosticCode::ProviderUnavailable,
            "metadata descriptor is not valid XML",
            Some("type"),
        )
    })?;
    let element = document
        .descendants()
        .find(|node| node.is_element() && node.range() == element_range)
        .ok_or_else(|| {
            typed_diagnostic(
                MetaDiagnosticCode::ProviderUnavailable,
                "metadata element is unavailable",
                Some("type"),
            )
        })?;
    let properties =
        super::info_projection::direct_md_child(element, "Properties").ok_or_else(|| {
            typed_diagnostic(
                MetaDiagnosticCode::ProviderUnavailable,
                "metadata element has no Properties",
                Some("type"),
            )
        })?;
    let observed = super::info_projection::parse_observed_metadata_type_node(properties)?;
    narrow_observed_metadata_type(observed)
}

fn narrow_observed_metadata_type(
    observed: crate::domain::metadata::ObservedMetadataType,
) -> Result<MetadataType, MetaDiagnostic> {
    MetadataType::try_from(observed).map_err(|mut diagnostic| {
        diagnostic.code = MetaDiagnosticCode::ValidationFailed;
        diagnostic.message =
            "existing metadata type is outside the public mutation algebra".to_string();
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
    relation: MetaRelation,
    mode: RelationEditMode,
    targets: &[crate::domain::metadata::MetaRelationTarget],
) -> Result<(), MetaDiagnostic> {
    if relation == MetaRelation::Source {
        return apply_typed_event_subscription_source(xml_text, mode, targets);
    }
    let tag = match relation {
        MetaRelation::Owners => "Owners",
        MetaRelation::RegisterRecords => "RegisterRecords",
        MetaRelation::BasedOn => "BasedOn",
        MetaRelation::InputByString => "InputByString",
        MetaRelation::Source => {
            return Err(typed_diagnostic(
                MetaDiagnosticCode::InvalidArguments,
                "EventSubscription Source requires replace mode",
                Some("relation"),
            ))
        }
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

fn apply_typed_event_subscription_source(
    xml_text: &mut String,
    mode: RelationEditMode,
    targets: &[crate::domain::metadata::MetaRelationTarget],
) -> Result<(), MetaDiagnostic> {
    if mode != RelationEditMode::Replace {
        return Err(typed_diagnostic(
            MetaDiagnosticCode::InvalidArguments,
            "EventSubscription Source supports replace mode only",
            Some("mode"),
        ));
    }
    let requested = targets
        .iter()
        .enumerate()
        .map(|(index, target)| match target {
            crate::domain::metadata::MetaRelationTarget::EventSource(source) => Ok(source.clone()),
            _ => Err(typed_diagnostic(
                MetaDiagnosticCode::InvalidArguments,
                "Source relation target has the wrong typed shape",
                Some(&format!("targets[{index}]")),
            )),
        })
        .collect::<Result<Vec<_>, _>>()?;
    let source_text = xml_text.trim_start_matches('\u{feff}');
    let source_offset = xml_text.len() - source_text.len();
    let document = Document::parse(source_text).map_err(|_| {
        typed_diagnostic(
            MetaDiagnosticCode::ProviderUnavailable,
            "metadata descriptor is not valid XML",
            None,
        )
    })?;
    let source_node = meta_event_subscription_source_node(&document).map_err(|message| {
        typed_diagnostic(
            MetaDiagnosticCode::ProviderUnavailable,
            message,
            Some("relation"),
        )
    })?;
    // A replace operation is also the repair path for a syntactically readable
    // legacy TypeDescription source. Failure to project the old wire value into
    // the logical algebra must not prevent replacing it with a valid target.
    let current = parse_meta_event_subscription_source(source_node).ok();
    let requested = canonical_meta_event_sources(&requested);
    if current
        .as_deref()
        .is_some_and(|current| canonical_meta_event_sources(current) == requested)
    {
        return Ok(());
    }
    let range = source_node.range();
    let range = range.start + source_offset..range.end + source_offset;
    let indent = meta_edit_line_indent(xml_text, range.start);
    let current_source = &xml_text[range.clone()];
    let replacement_eol_source = if current_source
        .as_bytes()
        .iter()
        .any(|byte| matches!(*byte, b'\r' | b'\n'))
    {
        current_source
    } else {
        xml_text.as_str()
    };
    let replacement_snapshot = SourceTextSnapshot::from_bytes(replacement_eol_source.as_bytes())
        .map_err(|error| {
            typed_diagnostic(
                MetaDiagnosticCode::ProviderUnavailable,
                format!("metadata descriptor snapshot failed: {error}"),
                Some("relation"),
            )
        })?;
    let replacement_eol =
        resolve_observed_line_ending(&replacement_snapshot, None).map_err(|error| {
            typed_diagnostic(
                MetaDiagnosticCode::ValidationFailed,
                format!("metadata descriptor EOL policy failed: {error}"),
                Some("relation"),
            )
        })?;
    let replacement = emit_meta_event_subscription_source(&indent, &requested, source_node)
        .map_err(|message| {
            typed_diagnostic(
                MetaDiagnosticCode::InvalidArguments,
                message,
                Some("relation"),
            )
        })?;
    let replacement = meta_edit_lf_to_eol(replacement.trim_start(), replacement_eol);
    drop(document);
    xml_text.replace_range(range, &replacement);
    Ok(())
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::domain::metadata::{
        DateFractions, MetaCollection, MetaEditOperation, MetaElementInput, MetaElementUpdateInput,
        MetaEventSource, MetaFillValue, MetaPosition, MetaPropertyChanges, MetaPropertyInput,
        MetaPropertyValue, MetaRelation, MetaRelationTarget, MetaScope, MetadataReference,
        MetadataType, MetadataTypeVariant, NumberSign, RelationEditMode, StringLengthMode,
    };
    use crate::domain::source_target::{MetadataAddress, PLATFORM_XML_8_3_27_FORMAT_2_20};

    fn object_xml(kind: &str, name: &str, properties: &str) -> String {
        format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" xmlns:app="http://v8.1c.ru/8.2/managed-application/core" xmlns:cfg="http://v8.1c.ru/8.1/data/enterprise/current-config" xmlns:v8="http://v8.1c.ru/8.1/data/core" xmlns:xr="http://v8.1c.ru/8.3/xcf/readable" xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" version="2.20">
	<{kind} uuid="11111111-1111-4111-8111-111111111111">
		<Properties><Name>{name}</Name><Synonym/><Comment/>{properties}</Properties>
		<ChildObjects/>
	</{kind}>
</MetaDataObject>
"#
        )
    }

    fn event_subscription_xml(source: &str) -> String {
        format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" xmlns:cfg="http://v8.1c.ru/8.1/data/enterprise/current-config" xmlns:v8="http://v8.1c.ru/8.1/data/core" xmlns:xr="http://v8.1c.ru/8.3/xcf/readable" xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" version="2.20">
	<EventSubscription uuid="11111111-1111-4111-8111-111111111111">
		<Properties>
			<Name>Events</Name>
			<Synonym/>
			<Comment/>
			{source}
			<Event>BeforeWrite</Event>
			<Handler>CommonModule.Handler.OnEvent</Handler>
		</Properties>
	</EventSubscription>
</MetaDataObject>
"#
        )
    }

    fn event_subscription_alias_xml(source: &str) -> String {
        format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" xmlns:c="http://v8.1c.ru/8.1/data/enterprise/current-config" xmlns:d="http://v8.1c.ru/8.1/data/core" xmlns:s="http://www.w3.org/2001/XMLSchema" version="2.20">
	<EventSubscription uuid="11111111-1111-4111-8111-111111111111">
		<Properties>
			<Name>Events</Name>
			<Synonym/>
			<Comment/>
			{source}
			<Event>BeforeWrite</Event>
			<Handler>CommonModule.Handler.OnEvent</Handler>
		</Properties>
	</EventSubscription>
</MetaDataObject>
"#
        )
    }

    fn source_replace(sources: Vec<MetaEventSource>) -> MetaEditOperation {
        MetaEditOperation::edit_relation_targets(
            MetaRelation::Source,
            RelationEditMode::Replace,
            sources
                .into_iter()
                .map(MetaRelationTarget::EventSource)
                .collect(),
        )
        .unwrap()
    }

    fn catalog_family() -> MetaEventSource {
        MetaEventSource::Family {
            source_class: crate::domain::metadata::EventSourceClass::CatalogObject,
        }
    }

    fn metadata_reference(path: &str) -> MetadataReference {
        MetadataReference {
            metadata_path: MetadataAddress::parse(PLATFORM_XML_8_3_27_FORMAT_2_20, path).unwrap(),
        }
    }

    #[test]
    fn register_collections_insert_into_their_canonical_platform_slot() {
        // The 8.3.27 export orders register children Resource → Attribute →
        // Dimension (773/773 registers in the reference dump). Appending in
        // operation order makes the platform reorder the file on roundtrip,
        // which the exact gate reports as accepted-normalized.
        fn register_xml(children: &[(&str, &str)]) -> String {
            let mut body = String::new();
            for (index, (tag, name)) in children.iter().enumerate() {
                body.push_str(&format!(
                    concat!(
                        "\t\t\t<{tag} uuid=\"22222222-2222-4222-8222-22222222222{n}\">\n",
                        "\t\t\t\t<Properties><Name>{name}</Name></Properties>\n",
                        "\t\t\t</{tag}>\n",
                    ),
                    tag = tag,
                    name = name,
                    n = index,
                ));
            }
            format!(
                concat!(
                    "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n",
                    "<MetaDataObject xmlns=\"http://v8.1c.ru/8.3/MDClasses\" version=\"2.20\">\n",
                    "\t<InformationRegister uuid=\"11111111-1111-4111-8111-111111111111\">\n",
                    "\t\t<Properties><Name>Corpus</Name></Properties>\n",
                    "\t\t<ChildObjects>\n{body}\t\t</ChildObjects>\n",
                    "\t</InformationRegister>\n",
                    "</MetaDataObject>\n",
                ),
                body = body,
            )
        }

        // (existing children, inserted tag, expected child-name order)
        type SlotCase = (
            &'static [(&'static str, &'static str)],
            &'static str,
            &'static [&'static str],
        );
        let matrix: &[SlotCase] = &[
            // every earlier slot lands before every later slot already present
            (&[("Dimension", "Item")], "Resource", &["New", "Item"]),
            (&[("Attribute", "Note")], "Resource", &["New", "Note"]),
            (&[("Dimension", "Item")], "Attribute", &["New", "Item"]),
            (
                &[("Attribute", "Note"), ("Dimension", "Item")],
                "Resource",
                &["New", "Note", "Item"],
            ),
            // the last slot and same-slot siblings keep append order
            (&[("Resource", "Price")], "Dimension", &["Price", "New"]),
            (&[("Dimension", "Item")], "Dimension", &["Item", "New"]),
            (&[("Resource", "Price")], "Resource", &["Price", "New"]),
        ];
        for (existing, inserted, expected) in matrix {
            let mut xml = register_xml(existing);
            meta_edit_insert_top_child_object(
                &mut xml,
                &[
                    format!("\t\t\t<{inserted} uuid=\"33333333-3333-4333-8333-333333333333\">"),
                    "\t\t\t\t<Properties><Name>New</Name></Properties>".to_string(),
                    format!("\t\t\t</{inserted}>"),
                ],
            )
            .unwrap();

            let order: Vec<&str> = expected
                .iter()
                .map(|name| (xml.find(&format!("<Name>{name}</Name>")).unwrap(), *name))
                .collect::<std::collections::BTreeMap<_, _>>()
                .into_values()
                .collect();
            assert_eq!(
                &order, expected,
                "insert {inserted} into {existing:?}: {xml}"
            );
        }
    }

    #[test]
    fn typed_fill_parser_uses_the_exact_xsd_timezone_boundary() {
        for (timezone, valid) in [
            ("+13:59", true),
            ("-13:59", true),
            ("+14:00", true),
            ("-14:00", true),
            ("+14:01", false),
            ("-14:01", false),
            ("+14:59", false),
            ("-14:59", false),
        ] {
            let properties = format!(
                r#"<FillValue xsi:type="xs:dateTime">2026-01-01T12:00:00{timezone}</FillValue>"#
            );
            let result = parse_typed_fill_value(&properties);
            assert_eq!(
                result.is_ok(),
                valid,
                "writer fill parser disagrees at timezone {timezone}: {result:?}"
            );
        }
    }

    #[test]
    fn typed_fill_parser_uses_complete_direct_text_and_rejects_nil_suffix() {
        assert_eq!(
            parse_typed_fill_value(
                r#"<FillValue xsi:type="xs:string">ab<!--split-->cd</FillValue>"#,
            )
            .unwrap(),
            Some(MetaFillValue::String("abcd".to_string()))
        );
        assert!(parse_typed_fill_value(
            r#"<FillValue xsi:nil="true"><!--split-->garbage</FillValue>"#,
        )
        .is_err());
    }

    #[test]
    fn typed_metadata_type_uses_the_complete_direct_text_value() {
        let parsed = parse_typed_metadata_type(concat!(
            "<v8:Type>xs:str<!--future-separator-->ing</v8:Type>",
            "<v8:StringQualifiers><v8:Length>0</v8:Length>",
            "<v8:AllowedLength>Variable</v8:AllowedLength></v8:StringQualifiers>"
        ))
        .unwrap();
        assert!(matches!(
            parsed.variants.as_slice(),
            [MetadataTypeVariant::String { .. }]
        ));

        let invalid = parse_typed_metadata_type(concat!(
            "<v8:Type>xs:string<!--future-suffix-->garbage</v8:Type>",
            "<v8:StringQualifiers><v8:Length>0</v8:Length>",
            "<v8:AllowedLength>Variable</v8:AllowedLength></v8:StringQualifiers>"
        ));
        assert!(invalid.is_err());

        let qualified = parse_typed_metadata_type(concat!(
            "<v8:Type>xs:string</v8:Type>",
            "<v8:StringQualifiers>",
            "<v8:Length>1<!--future-separator-->0</v8:Length>",
            "<v8:AllowedLength>Fi<!--future-separator-->xed</v8:AllowedLength>",
            "</v8:StringQualifiers>"
        ))
        .unwrap();
        assert!(matches!(
            qualified.variants.as_slice(),
            [MetadataTypeVariant::String {
                length: 10,
                allowed_length: StringLengthMode::Fixed,
            }]
        ));

        let invalid_qualifier = parse_typed_metadata_type(concat!(
            "<v8:Type>xs:string</v8:Type>",
            "<v8:StringQualifiers>",
            "<v8:Length>10<!--future-suffix-->garbage</v8:Length>",
            "</v8:StringQualifiers>"
        ));
        assert!(invalid_qualifier.is_err());

        let container =
            parse_typed_metadata_type("<Type><v8:Type>xs:boolean</v8:Type></Type>").unwrap();
        assert!(matches!(
            container.variants.as_slice(),
            [MetadataTypeVariant::Boolean]
        ));

        let scoped = parse_typed_metadata_type(concat!(
            "<Properties>",
            "<future:Type xmlns:future=\"urn:future\"><v8:Type>xs:boolean</v8:Type></future:Type>",
            "<Type><v8:Type>xs:string</v8:Type>",
            "<v8:StringQualifiers><v8:Length>0</v8:Length>",
            "<v8:AllowedLength>Variable</v8:AllowedLength></v8:StringQualifiers></Type>",
            "</Properties>"
        ))
        .unwrap();
        assert!(matches!(
            scoped.variants.as_slice(),
            [MetadataTypeVariant::String {
                length: 0,
                allowed_length: StringLengthMode::Variable,
            }]
        ));

        assert!(parse_typed_metadata_type(concat!(
            "<Type><v8:Type>xs:string</v8:Type>",
            "<future:Extension xmlns:future=\"urn:future\"/>",
            "</Type>"
        ))
        .is_err());
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
    fn retired_dsl_scalar_typed_properties_round_trip_through_template_writer_and_info() {
        use crate::domain::metadata::MetadataKind::*;

        let cases: &[(
            &str,
            &str,
            MetaPropertyValue,
            &[crate::domain::metadata::MetadataKind],
        )] = &[
            (
                "ActionPeriod",
                "ActionPeriod",
                MetaPropertyValue::Boolean(true),
                &[CalculationRegister],
            ),
            (
                "ActionPeriodUse",
                "ActionPeriodUse",
                MetaPropertyValue::Boolean(true),
                &[ChartOfCalculationTypes],
            ),
            (
                "AutoOrderByCode",
                "AutoOrderByCode",
                MetaPropertyValue::Boolean(false),
                &[ChartOfAccounts],
            ),
            (
                "BasePeriod",
                "BasePeriod",
                MetaPropertyValue::Boolean(true),
                &[CalculationRegister],
            ),
            (
                "ChoiceMode",
                "ChoiceMode",
                MetaPropertyValue::String("FromForm".into()),
                &[Catalog],
            ),
            (
                "ClientManagedApplication",
                "ClientManagedApplication",
                MetaPropertyValue::Boolean(true),
                &[CommonModule],
            ),
            (
                "CodeAllowedLength",
                "CodeAllowedLength",
                MetaPropertyValue::String("Fixed".into()),
                &[Catalog],
            ),
            (
                "CodeLength",
                "CodeLength",
                MetaPropertyValue::UnsignedInteger(12),
                &[
                    Catalog,
                    ChartOfAccounts,
                    ChartOfCharacteristicTypes,
                    ChartOfCalculationTypes,
                    ExchangePlan,
                ],
            ),
            (
                "CodeMask",
                "CodeMask",
                MetaPropertyValue::String("@@@.@@".into()),
                &[ChartOfAccounts],
            ),
            (
                "CodeType",
                "CodeType",
                MetaPropertyValue::String("Number".into()),
                &[Catalog],
            ),
            (
                "Correspondence",
                "Correspondence",
                MetaPropertyValue::Boolean(true),
                &[AccountingRegister],
            ),
            (
                "DefaultPresentation",
                "DefaultPresentation",
                MetaPropertyValue::String("AsCode".into()),
                &[Catalog],
            ),
            (
                "DependenceOnCalculationTypes",
                "DependenceOnCalculationTypes",
                MetaPropertyValue::String("OnActionPeriod".into()),
                &[ChartOfCalculationTypes],
            ),
            (
                "Description",
                "Description",
                MetaPropertyValue::String("Night import".into()),
                &[ScheduledJob],
            ),
            (
                "DescriptionLength",
                "DescriptionLength",
                MetaPropertyValue::UnsignedInteger(120),
                &[
                    Catalog,
                    ChartOfAccounts,
                    ChartOfCharacteristicTypes,
                    ChartOfCalculationTypes,
                    Task,
                    ExchangePlan,
                ],
            ),
            (
                "DistributedInfoBase",
                "DistributedInfoBase",
                MetaPropertyValue::Boolean(true),
                &[ExchangePlan],
            ),
            (
                "EnableTotalsSplitting",
                "EnableTotalsSplitting",
                MetaPropertyValue::Boolean(false),
                &[AccumulationRegister],
            ),
            (
                "ExternalConnection",
                "ExternalConnection",
                MetaPropertyValue::Boolean(true),
                &[CommonModule],
            ),
            (
                "FoldersOnTop",
                "FoldersOnTop",
                MetaPropertyValue::Boolean(false),
                &[Catalog],
            ),
            (
                "Global",
                "Global",
                MetaPropertyValue::Boolean(true),
                &[CommonModule],
            ),
            (
                "HierarchyType",
                "HierarchyType",
                MetaPropertyValue::String("HierarchyOfItems".into()),
                &[Catalog],
            ),
            (
                "LevelCount",
                "LevelCount",
                MetaPropertyValue::UnsignedInteger(4),
                &[Catalog],
            ),
            (
                "LimitLevelCount",
                "LimitLevelCount",
                MetaPropertyValue::Boolean(true),
                &[Catalog],
            ),
            (
                "MainFilterOnPeriod",
                "MainFilterOnPeriod",
                MetaPropertyValue::Boolean(true),
                &[InformationRegister],
            ),
            (
                "MaxExtDimensionCount",
                "MaxExtDimensionCount",
                MetaPropertyValue::UnsignedInteger(4),
                &[ChartOfAccounts],
            ),
            (
                "Namespace",
                "Namespace",
                MetaPropertyValue::String("urn:unica:test".into()),
                &[WebService],
            ),
            (
                "NumberAllowedLength",
                "NumberAllowedLength",
                MetaPropertyValue::String("Fixed".into()),
                &[Document],
            ),
            (
                "NumberLength",
                "NumberLength",
                MetaPropertyValue::UnsignedInteger(15),
                &[Document, BusinessProcess, Task],
            ),
            (
                "NumberPeriodicity",
                "NumberPeriodicity",
                MetaPropertyValue::String("Month".into()),
                &[Document],
            ),
            (
                "NumberType",
                "NumberType",
                MetaPropertyValue::String("Number".into()),
                &[Document, BusinessProcess, Task],
            ),
            (
                "OrderLength",
                "OrderLength",
                MetaPropertyValue::UnsignedInteger(6),
                &[ChartOfAccounts],
            ),
            (
                "PeriodAdjustmentLength",
                "PeriodAdjustmentLength",
                MetaPropertyValue::UnsignedInteger(2),
                &[AccountingRegister],
            ),
            (
                "Periodicity",
                "InformationRegisterPeriodicity",
                MetaPropertyValue::String("Quarter".into()),
                &[InformationRegister],
            ),
            (
                "Periodicity",
                "Periodicity",
                MetaPropertyValue::String("Quarter".into()),
                &[CalculationRegister],
            ),
            (
                "PostInPrivilegedMode",
                "PostInPrivilegedMode",
                MetaPropertyValue::Boolean(false),
                &[Document],
            ),
            (
                "Posting",
                "Posting",
                MetaPropertyValue::String("Deny".into()),
                &[Document],
            ),
            (
                "Predefined",
                "Predefined",
                MetaPropertyValue::Boolean(true),
                &[ScheduledJob],
            ),
            (
                "Privileged",
                "Privileged",
                MetaPropertyValue::Boolean(true),
                &[CommonModule],
            ),
            (
                "QuickChoice",
                "QuickChoice",
                MetaPropertyValue::Boolean(true),
                &[Catalog],
            ),
            (
                "RealTimePosting",
                "RealTimePosting",
                MetaPropertyValue::String("Allow".into()),
                &[Document],
            ),
            (
                "RegisterRecordsDeletion",
                "RegisterRecordsDeletion",
                MetaPropertyValue::String("AutoDeleteOff".into()),
                &[Document],
            ),
            (
                "RegisterRecordsWritingOnPost",
                "RegisterRecordsWritingOnPost",
                MetaPropertyValue::String("WriteAll".into()),
                &[Document],
            ),
            (
                "RestartCountOnFailure",
                "RestartCountOnFailure",
                MetaPropertyValue::UnsignedInteger(5),
                &[ScheduledJob],
            ),
            (
                "RestartIntervalOnFailure",
                "RestartIntervalOnFailure",
                MetaPropertyValue::UnsignedInteger(30),
                &[ScheduledJob],
            ),
            (
                "ReturnValuesReuse",
                "ReturnValuesReuse",
                MetaPropertyValue::String("DuringRequest".into()),
                &[CommonModule],
            ),
            (
                "ReuseSessions",
                "ReuseSessions",
                MetaPropertyValue::String("AutoUse".into()),
                &[HTTPService, WebService],
            ),
            (
                "RootURL",
                "RootURL",
                MetaPropertyValue::String("v2".into()),
                &[HTTPService],
            ),
            (
                "Server",
                "Server",
                MetaPropertyValue::Boolean(false),
                &[CommonModule],
            ),
            (
                "ServerCall",
                "ServerCall",
                MetaPropertyValue::Boolean(true),
                &[CommonModule],
            ),
            (
                "SessionMaxAge",
                "SessionMaxAge",
                MetaPropertyValue::UnsignedInteger(45),
                &[HTTPService, WebService],
            ),
            (
                "SubordinationUse",
                "SubordinationUse",
                MetaPropertyValue::String("ToFoldersAndItems".into()),
                &[Catalog],
            ),
            (
                "UnpostInPrivilegedMode",
                "UnpostInPrivilegedMode",
                MetaPropertyValue::Boolean(false),
                &[Document],
            ),
            (
                "Use",
                "Use",
                MetaPropertyValue::Boolean(true),
                &[ScheduledJob],
            ),
            (
                "WriteMode",
                "WriteMode",
                MetaPropertyValue::String("RecorderSubordinate".into()),
                &[InformationRegister],
            ),
        ];

        for (public_name, xml_tag, value, kinds) in cases {
            for kind in *kinds {
                let (mut xml, _) = super::super::template_catalog::minimal_metadata_xml_for_tests(
                    *kind, "Evidence",
                )
                .unwrap();
                let changes = MetaPropertyChanges::convert(
                    *kind,
                    vec![MetaPropertyInput::new(*public_name, value.clone())],
                )
                .unwrap_or_else(|diagnostic| {
                    panic!("{public_name} for {}: {diagnostic:?}", kind.as_str())
                });

                apply_typed_operations(
                    &mut xml,
                    &[MetaEditOperation::SetProperties { values: changes }],
                )
                .unwrap_or_else(|failure| {
                    panic!("{public_name} for {}: {failure:?}", kind.as_str())
                });

                let expected_xml = match value {
                    MetaPropertyValue::String(value) => {
                        format!("<{xml_tag}>{}</{xml_tag}>", escape_xml(value))
                    }
                    MetaPropertyValue::Boolean(value) => {
                        format!("<{xml_tag}>{value}</{xml_tag}>")
                    }
                    MetaPropertyValue::UnsignedInteger(value) => {
                        format!("<{xml_tag}>{value}</{xml_tag}>")
                    }
                };
                assert!(
                    xml.contains(&expected_xml),
                    "{public_name} for {}: {xml}",
                    kind.as_str()
                );

                let document = roxmltree::Document::parse(&xml).unwrap();
                let object = meta_edit_object_node(&document).unwrap();
                let properties = meta_info_child(object, "Properties");
                let observed = super::super::info::typed_properties(properties, *kind);
                let observed = observed
                    .iter()
                    .find(|property| property.key == *public_name)
                    .unwrap_or_else(|| {
                        panic!("{public_name} missing from info for {}", kind.as_str())
                    });
                assert_eq!(
                    serde_json::to_value(&observed.value).unwrap(),
                    serde_json::to_value(value).unwrap(),
                    "{public_name} for {}",
                    kind.as_str()
                );
            }
        }
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
    fn typed_collection_targets_are_case_insensitive() {
        let mut xml = object_xml("Document", "Order", "<RegisterRecords/>");
        apply_typed_operations(
            &mut xml,
            &[MetaEditOperation::add(
                MetaCollection::Attributes,
                None,
                vec![MetaElementInput::named("Товар")],
            )
            .unwrap()],
        )
        .unwrap();

        let duplicate = apply_typed_operations(
            &mut xml,
            &[MetaEditOperation::add(
                MetaCollection::Attributes,
                None,
                vec![MetaElementInput::named("товар")],
            )
            .unwrap()],
        )
        .unwrap_err();
        assert_eq!(
            duplicate.diagnostics[0].code,
            MetaDiagnosticCode::AlreadyExists
        );

        apply_typed_operations(
            &mut xml,
            &[MetaEditOperation::update(
                MetaCollection::Attributes,
                None,
                vec![MetaElementUpdateInput {
                    name: "тОвАр".into(),
                    new_name: Some("Позиция".into()),
                    ..MetaElementUpdateInput::default()
                }],
            )
            .unwrap()],
        )
        .unwrap();
        assert!(xml.contains("<Name>Позиция</Name>"));

        apply_typed_operations(
            &mut xml,
            &[
                MetaEditOperation::remove(MetaCollection::Attributes, None, vec!["пОзИцИя".into()])
                    .unwrap(),
            ],
        )
        .unwrap();
        assert!(!xml.contains("<Attribute uuid="));
    }

    #[test]
    fn typed_resource_add_keeps_digits_apart_in_the_generated_synonym() {
        let mut xml = object_xml("InformationRegister", "PaymentTerms", "");
        apply_typed_operations(
            &mut xml,
            &[MetaEditOperation::add(
                MetaCollection::Resources,
                None,
                vec![
                    MetaElementInput::named("СуммаЗакупокЗа30Дней"),
                    MetaElementInput {
                        name: "СуммаПродажЗа30Дней".into(),
                        synonym: Some("Сумма продаж за месяц".into()),
                        ..MetaElementInput::default()
                    },
                ],
            )
            .unwrap()],
        )
        .unwrap();

        assert!(
            xml.contains("<v8:content>Сумма закупок за 30 дней</v8:content>"),
            "{xml}"
        );
        assert!(
            xml.contains("<v8:content>Сумма продаж за месяц</v8:content>"),
            "{xml}"
        );
    }

    /// #323, first defect. A CRLF descriptor must not grow a literal `&#13;`
    /// entity: the platform writes none — the 8.3.27 dump has zero occurrences
    /// across 31 009 structural XML files — so an inserted element carries the
    /// file's own EOL rather than an escaped carriage return in content.
    #[test]
    fn typed_resource_add_does_not_escape_the_carriage_return_of_a_crlf_descriptor() {
        let mut xml = object_xml("InformationRegister", "PaymentTerms", "").replace('\n', "\r\n");

        apply_typed_operations(
            &mut xml,
            &[MetaEditOperation::add(
                MetaCollection::Resources,
                None,
                vec![MetaElementInput::named("СуммаЗакупокЗа30Дней")],
            )
            .unwrap()],
        )
        .unwrap();

        assert!(!xml.contains("&#13;"), "{xml}");
        assert!(xml.contains("<Resource uuid="), "{xml}");
    }

    /// #323, second defect. A new register field must not silently disagree
    /// with the fields already in the register. The corpus gives no single
    /// standard value — a 8.3.27 vendor dump has 2169 of 2490 information
    /// register resources on `Use/Use` and 42 registers mixed inside
    /// themselves — so the new field takes what its siblings agree on and
    /// falls back to the template default only when they do not.
    #[test]
    fn typed_resource_add_inherits_search_and_history_from_its_siblings() {
        let existing = rendered_resource("СуммаПродаж")
            .replace(
                "<FullTextSearch>Use</FullTextSearch>",
                "<FullTextSearch>DontUse</FullTextSearch>",
            )
            .replace(
                "<DataHistory>Use</DataHistory>",
                "<DataHistory>DontUse</DataHistory>",
            );
        let mut xml = object_xml("InformationRegister", "PaymentTerms", &existing);

        apply_typed_operations(
            &mut xml,
            &[MetaEditOperation::add(
                MetaCollection::Resources,
                None,
                vec![MetaElementInput::named("СуммаЗакупок")],
            )
            .unwrap()],
        )
        .unwrap();

        assert_eq!(
            xml.matches("<FullTextSearch>DontUse</FullTextSearch>")
                .count(),
            2,
            "the new resource follows its sibling: {xml}"
        );
        assert_eq!(
            xml.matches("<DataHistory>DontUse</DataHistory>").count(),
            2,
            "the new resource follows its sibling: {xml}"
        );
    }

    /// Review of #444: the change covers three settings and two collections,
    /// so the regression has to prove `Indexing` and the `Dimension` branch as
    /// well — otherwise either could break with the suite still green.
    #[test]
    fn typed_register_field_add_inherits_indexing_and_covers_dimensions() {
        let existing = rendered_resource("СуммаПродаж").replace(
            "<Indexing>DontIndex</Indexing>",
            "<Indexing>Index</Indexing>",
        );
        let mut xml = object_xml("InformationRegister", "PaymentTerms", &existing);

        apply_typed_operations(
            &mut xml,
            &[MetaEditOperation::add(
                MetaCollection::Resources,
                None,
                vec![MetaElementInput::named("СуммаЗакупок")],
            )
            .unwrap()],
        )
        .unwrap();

        assert_eq!(
            xml.matches("<Indexing>Index</Indexing>").count(),
            2,
            "the new resource follows its sibling indexing: {xml}"
        );

        let dimension = rendered_dimension("Организация")
            .replace(
                "<FullTextSearch>Use</FullTextSearch>",
                "<FullTextSearch>DontUse</FullTextSearch>",
            )
            .replace(
                "<Indexing>DontIndex</Indexing>",
                "<Indexing>Index</Indexing>",
            );
        let mut xml = object_xml("InformationRegister", "PaymentTerms", &dimension);

        apply_typed_operations(
            &mut xml,
            &[MetaEditOperation::add(
                MetaCollection::Dimensions,
                None,
                vec![MetaElementInput::named("Склад")],
            )
            .unwrap()],
        )
        .unwrap();

        assert_eq!(
            xml.matches("<FullTextSearch>DontUse</FullTextSearch>")
                .count(),
            2,
            "the dimension branch inherits too: {xml}"
        );
        assert_eq!(
            xml.matches("<Indexing>Index</Indexing>").count(),
            2,
            "the dimension branch inherits indexing too: {xml}"
        );
    }

    #[test]
    fn typed_child_tree_rejects_excessive_depth_before_capture() {
        let root = std::env::temp_dir().join(format!(
            "unica-meta-edit-child-depth-{}",
            uuid::Uuid::new_v4()
        ));
        let mut current = root.clone();
        for depth in 0..65 {
            current = current.join(format!("d{depth}"));
        }
        std::fs::create_dir_all(&current).unwrap();
        std::fs::write(current.join("payload.bin"), b"payload").unwrap();

        let result = read_typed_child_tree(&root);

        assert!(matches!(result, Err(TypedChildTreeError::Unavailable)));
        let _ = std::fs::remove_dir_all(root);
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
            MetadataTypeVariant::Uuid,
            MetadataTypeVariant::Date {
                fractions: DateFractions::DateTime,
            },
            MetadataTypeVariant::BinaryData {
                length: 512,
                allowed_length: StringLengthMode::Fixed,
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
        assert!(xml.contains("<v8:Type>v8:UUID</v8:Type>"));
        assert!(xml.contains("<v8:Type>xs:dateTime</v8:Type>"));
        assert!(xml.contains("<v8:Type>cfg:CatalogRef.Items</v8:Type>"));
        assert!(xml.contains("<v8:TypeSet>cfg:DefinedType.ExternalCode</v8:TypeSet>"));
        assert!(xml.contains("<FillChecking>ShowError</FillChecking>"));
    }

    #[test]
    fn tracked_platform_uuid_fixture_round_trips_through_the_typed_emitter() {
        let xml = include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../tests/fixtures/unica_mcp_script_parity/",
            "meta-validate-subordinate-register/InformationRegisters/SubordinateRegister.xml"
        ));
        let document = Document::parse(xml).unwrap();
        let properties = document
            .descendants()
            .filter(|node| node.is_element() && node.tag_name().name() == "Properties")
            .find(|properties| {
                meta_info_child(*properties, "Type").is_some_and(|type_node| {
                    meta_info_children(type_node, "Type")
                        .iter()
                        .any(|node| node.text() == Some("v8:UUID"))
                })
            })
            .expect("tracked fixture contains a UUID metadata field");
        let parsed = parse_typed_metadata_type(&xml[properties.range()]).unwrap();
        assert_eq!(parsed.variants, vec![MetadataTypeVariant::Uuid]);

        let mut emitted = Vec::new();
        emit_meta_typed_value_type(&mut emitted, "\t", &parsed);
        let properties = format!("<Properties>{}</Properties>", emitted.join("\n"));
        assert_eq!(parse_typed_metadata_type(&properties).unwrap(), parsed);
    }

    fn information_register_xml_with_data_cr() -> String {
        object_xml(
            "InformationRegister",
            "Sample",
            "<InformationRegisterPeriodicity>Nonperiodical</InformationRegisterPeriodicity>",
        )
        .replace("<Comment/>", "<Comment>First&#13;Second</Comment>")
    }

    fn rendered_dimension(name: &str) -> String {
        let element = MetaElementDefinition::convert(
            MetaCollection::Dimensions,
            MetaElementInput::named(name),
        )
        .unwrap();
        render_typed_element(
            "InformationRegister",
            "Sample",
            MetaCollection::Dimensions,
            &element,
        )
        .unwrap()
        .join("\n")
    }

    fn rendered_resource(name: &str) -> String {
        let element = MetaElementDefinition::convert(
            MetaCollection::Resources,
            MetaElementInput::named(name),
        )
        .unwrap();
        render_typed_element(
            "InformationRegister",
            "Sample",
            MetaCollection::Resources,
            &element,
        )
        .unwrap()
        .join("\n")
    }

    fn assert_structural_resource_separators(xml: &str, expected: usize) {
        assert!(
            xml.contains("<Comment>First&#13;Second</Comment>"),
            "data CR entity was not preserved: {xml}"
        );
        assert!(
            !xml.contains("</Resource>&#13;"),
            "structural CR entity was emitted: {xml}"
        );
        assert_eq!(
            xml.matches("</Resource>\n\t\t\t<Resource").count(),
            expected,
            "resource elements are not separated by newlines: {xml}"
        );
        Document::parse(xml).unwrap_or_else(|error| panic!("{error}\n{xml}"));
    }

    #[test]
    fn typed_child_append_uses_a_newline_instead_of_a_carriage_return_entity() {
        let first_resource = rendered_resource("First");
        let mut xml = information_register_xml_with_data_cr().replace(
            "<ChildObjects/>",
            &format!("<ChildObjects>\n{first_resource}\n\t\t</ChildObjects>"),
        );
        let operation = MetaEditOperation::add(
            MetaCollection::Resources,
            None,
            vec![MetaElementInput::named("Second")],
        )
        .unwrap();

        apply_typed_operations(&mut xml, &[operation]).unwrap();

        assert_structural_resource_separators(&xml, 1);
        let first = xml.find("<Name>First</Name>").unwrap();
        let second = xml.find("<Name>Second</Name>").unwrap();
        assert!(first < second, "{xml}");
    }

    #[test]
    fn typed_child_position_after_uses_a_newline_instead_of_a_carriage_return_entity() {
        let resources = [rendered_resource("First"), rendered_resource("Last")].join("\n");
        let mut xml = information_register_xml_with_data_cr().replace(
            "<ChildObjects/>",
            &format!("<ChildObjects>\n{resources}\n\t\t</ChildObjects>"),
        );
        let operation = MetaEditOperation::add(
            MetaCollection::Resources,
            None,
            vec![MetaElementInput {
                name: "Second".into(),
                position: Some(MetaPosition::new(None, Some("First".into())).unwrap()),
                ..MetaElementInput::default()
            }],
        )
        .unwrap();

        apply_typed_operations(&mut xml, &[operation]).unwrap();

        assert_structural_resource_separators(&xml, 2);
        let first = xml.find("<Name>First</Name>").unwrap();
        let second = xml.find("<Name>Second</Name>").unwrap();
        let last = xml.find("<Name>Last</Name>").unwrap();
        assert!(first < second && second < last, "{xml}");
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
    fn typed_event_source_replaces_only_the_exact_direct_source_and_is_semantically_idempotent() {
        let nested_decoy = "<Wrapper><Source><v8:Type>xs:boolean</v8:Type></Source></Wrapper>";
        let mut xml = event_subscription_xml(&format!("<Source/>{nested_decoy}"));
        let operation = source_replace(vec![catalog_family()]);

        let result = apply_typed_operations(&mut xml, std::slice::from_ref(&operation)).unwrap();

        assert_eq!(result.effects.len(), 1);
        assert_eq!(result.effects[0].before, Some(serde_json::json!([])));
        assert_eq!(
            result.effects[0].after,
            Some(serde_json::json!([{
                "kind": "family",
                "sourceClass": "catalogObject",
            }]))
        );
        assert!(xml.contains(
            "<Source>\n\t\t\t\t<v8:TypeSet>cfg:CatalogObject</v8:TypeSet>\n\t\t\t</Source>"
        ));
        assert!(xml.contains(nested_decoy));

        let post_image = xml.clone();
        apply_typed_operations(&mut xml, &[operation]).unwrap();
        assert_eq!(xml, post_image);
    }

    #[test]
    fn typed_event_source_semantic_noop_is_order_insensitive_and_preserves_exact_bytes() {
        let source = concat!(
            "<Source>",
            "<v8:Type>cfg:CatalogObject.Items</v8:Type>",
            "<v8:Type>cfg:DocumentObject.Sale</v8:Type>",
            "</Source>"
        );
        let mut xml = event_subscription_xml(source);
        let before = xml.clone();
        let operation = source_replace(vec![
            MetaEventSource::Object {
                metadata_path: MetadataAddress::parse(
                    PLATFORM_XML_8_3_27_FORMAT_2_20,
                    "Document.Sale",
                )
                .unwrap(),
            },
            MetaEventSource::Object {
                metadata_path: MetadataAddress::parse(
                    PLATFORM_XML_8_3_27_FORMAT_2_20,
                    "Catalog.Items",
                )
                .unwrap(),
            },
        ]);

        apply_typed_operations(&mut xml, &[operation]).unwrap();

        assert_eq!(xml, before);
    }

    #[test]
    fn typed_event_source_alias_qname_noop_is_byte_exact_and_changes_reuse_aliases() {
        let source = concat!(
            "<Source>",
            "<d:Type>c:CatalogObject.Items</d:Type>",
            "<d:TypeSet>c:CatalogObject</d:TypeSet>",
            "</Source>"
        );
        let mut xml = event_subscription_alias_xml(source);
        let before = xml.clone();
        let object = MetaEventSource::Object {
            metadata_path: MetadataAddress::parse(PLATFORM_XML_8_3_27_FORMAT_2_20, "Catalog.Items")
                .unwrap(),
        };

        apply_typed_operations(
            &mut xml,
            &[source_replace(vec![object.clone(), catalog_family()])],
        )
        .unwrap();
        assert_eq!(xml, before);

        apply_typed_operations(&mut xml, &[source_replace(vec![object])]).unwrap();
        assert!(xml.contains("<d:Type>c:CatalogObject.Items</d:Type>"));
        assert!(!xml.contains("<v8:") && !xml.contains("cfg:") && !xml.contains("xs:"));
        roxmltree::Document::parse(&xml).unwrap();
    }

    #[test]
    fn typed_event_source_change_declares_missing_canonical_namespaces_locally() {
        let mut xml = event_subscription_alias_xml("<Source/>")
            .replace(
                r#" xmlns:c="http://v8.1c.ru/8.1/data/enterprise/current-config""#,
                "",
            )
            .replace(r#" xmlns:s="http://www.w3.org/2001/XMLSchema""#, "");
        let target = MetaEventSource::Object {
            metadata_path: MetadataAddress::parse(PLATFORM_XML_8_3_27_FORMAT_2_20, "Catalog.Items")
                .unwrap(),
        };

        apply_typed_operations(&mut xml, &[source_replace(vec![target])]).unwrap();

        assert!(xml.contains(r#"xmlns:cfg="http://v8.1c.ru/8.1/data/enterprise/current-config""#));
        assert!(xml.contains("<d:Type>cfg:CatalogObject.Items</d:Type>"));
        roxmltree::Document::parse(&xml).unwrap();
    }

    #[test]
    fn typed_event_source_change_does_not_reuse_declarations_on_replaced_source() {
        let local_source = concat!(
            r#"<Source xmlns:d="http://v8.1c.ru/8.1/data/core" "#,
            r#"xmlns:c="http://v8.1c.ru/8.1/data/enterprise/current-config" "#,
            r#"xmlns:s="http://www.w3.org/2001/XMLSchema">"#,
            "<d:Type>s:boolean</d:Type>",
            "</Source>"
        );
        let mut xml = event_subscription_alias_xml(local_source)
            .replacen(
                r#" xmlns:c="http://v8.1c.ru/8.1/data/enterprise/current-config""#,
                "",
                1,
            )
            .replacen(r#" xmlns:d="http://v8.1c.ru/8.1/data/core""#, "", 1)
            .replacen(r#" xmlns:s="http://www.w3.org/2001/XMLSchema""#, "", 1);
        let target = MetaEventSource::Object {
            metadata_path: MetadataAddress::parse(PLATFORM_XML_8_3_27_FORMAT_2_20, "Catalog.Items")
                .unwrap(),
        };

        apply_typed_operations(&mut xml, &[source_replace(vec![target])]).unwrap();

        assert!(xml.contains(
            r#"<Source xmlns:v8="http://v8.1c.ru/8.1/data/core" xmlns:cfg="http://v8.1c.ru/8.1/data/enterprise/current-config">"#
        ));
        roxmltree::Document::parse(&xml).unwrap();
    }

    #[test]
    fn typed_event_source_build_preserves_bom_and_crlf_for_change_and_noop() {
        let lf = event_subscription_xml("<Source/>");
        let mut preimage = b"\xef\xbb\xbf".to_vec();
        preimage.extend_from_slice(lf.replace('\n', "\r\n").as_bytes());
        let operation = source_replace(vec![catalog_family()]);
        let target =
            MetadataAddress::parse(PLATFORM_XML_8_3_27_FORMAT_2_20, "EventSubscription.Events")
                .unwrap();
        let context = crate::domain::workspace::WorkspaceContext {
            cwd: std::path::PathBuf::from("/unused"),
            workspace_root: std::path::PathBuf::from("/unused"),
            cache_root: std::path::PathBuf::from("/unused/.build"),
            workspace_epoch: 0,
        };

        let changed = build_typed_operation_post_image(
            TypedOperationDependencyScope::new(
                "main",
                std::path::Path::new("/unused"),
                std::path::Path::new("/unused/Configuration.xml"),
                b"",
                false,
            ),
            std::path::Path::new("/unused/EventSubscriptions/Events.xml"),
            &target,
            &preimage,
            std::slice::from_ref(&operation),
            &context,
        )
        .unwrap()
        .descriptor;

        assert!(changed.starts_with(b"\xef\xbb\xbf"));
        assert!(changed.windows(2).any(|pair| pair == b"\r\n"));
        assert!(!changed
            .iter()
            .enumerate()
            .any(|(index, byte)| *byte == b'\n' && (index == 0 || changed[index - 1] != b'\r')));

        let noop = build_typed_operation_post_image(
            TypedOperationDependencyScope::new(
                "main",
                std::path::Path::new("/unused"),
                std::path::Path::new("/unused/Configuration.xml"),
                b"",
                false,
            ),
            std::path::Path::new("/unused/EventSubscriptions/Events.xml"),
            &target,
            &changed,
            std::slice::from_ref(&operation),
            &context,
        )
        .unwrap()
        .descriptor;
        assert_eq!(noop, changed);
    }

    #[test]
    fn typed_event_source_build_preserves_cr_only_indentation() {
        let preimage = event_subscription_xml("<Source/>")
            .replace('\n', "\r")
            .into_bytes();
        let operation = source_replace(vec![catalog_family()]);
        let target =
            MetadataAddress::parse(PLATFORM_XML_8_3_27_FORMAT_2_20, "EventSubscription.Events")
                .unwrap();
        let context = crate::domain::workspace::WorkspaceContext {
            cwd: std::path::PathBuf::from("/unused"),
            workspace_root: std::path::PathBuf::from("/unused"),
            cache_root: std::path::PathBuf::from("/unused/.build"),
            workspace_epoch: 0,
        };

        let changed = build_typed_operation_post_image(
            TypedOperationDependencyScope::new(
                "main",
                std::path::Path::new("/unused"),
                std::path::Path::new("/unused/Configuration.xml"),
                b"",
                false,
            ),
            std::path::Path::new("/unused/EventSubscriptions/Events.xml"),
            &target,
            &preimage,
            &[operation],
            &context,
        )
        .unwrap()
        .descriptor;
        let changed = String::from_utf8(changed).unwrap();

        assert!(!changed.contains('\n'), "{changed:?}");
        assert!(
            changed
                .contains("\t\t\t<Source>\r\t\t\t\t<v8:TypeSet>cfg:CatalogObject</v8:TypeSet>\r\t\t\t</Source>"),
            "{changed:?}"
        );
    }

    #[test]
    fn typed_event_source_build_rejects_mixed_eol_before_source_only_serialization() {
        let mixed = event_subscription_xml("<Source/>").replacen('\n', "\r\n", 1);
        let operation = source_replace(vec![catalog_family()]);
        let target =
            MetadataAddress::parse(PLATFORM_XML_8_3_27_FORMAT_2_20, "EventSubscription.Events")
                .unwrap();
        let context = crate::domain::workspace::WorkspaceContext {
            cwd: std::path::PathBuf::from("/unused"),
            workspace_root: std::path::PathBuf::from("/unused"),
            cache_root: std::path::PathBuf::from("/unused/.build"),
            workspace_epoch: 0,
        };

        let failure = build_typed_operation_post_image(
            TypedOperationDependencyScope::new(
                "main",
                std::path::Path::new("/unused"),
                std::path::Path::new("/unused/Configuration.xml"),
                b"",
                false,
            ),
            std::path::Path::new("/unused/EventSubscriptions/Events.xml"),
            &target,
            mixed.as_bytes(),
            &[operation],
            &context,
        )
        .err()
        .expect("mixed EOL must be rejected before publication planning");

        assert_eq!(
            failure.diagnostics[0].code,
            MetaDiagnosticCode::ValidationFailed
        );
        assert!(
            failure.diagnostics[0]
                .message
                .contains("mixed line endings"),
            "{:?}",
            failure.diagnostics
        );
    }

    #[test]
    fn typed_relation_registry_matches_template_nodes_and_guards_private_images() {
        use crate::domain::metadata::{
            metadata_relation_spec, MetaRelationTarget, MetaRelationTargetPolicy,
        };

        for kind in MetadataKind::ALL {
            for relation in MetaRelation::ALL {
                let (template, _) =
                    super::super::template_catalog::minimal_metadata_xml_for_tests(*kind, "Owner")
                        .unwrap();
                let tag = match relation {
                    MetaRelation::Owners => "Owners",
                    MetaRelation::RegisterRecords => "RegisterRecords",
                    MetaRelation::BasedOn => "BasedOn",
                    MetaRelation::InputByString => "InputByString",
                    MetaRelation::Source => "Source",
                };
                let document = Document::parse(&template).unwrap();
                let has_node = document.descendants().any(|node| {
                    node.is_element()
                        && node.tag_name().name() == tag
                        && node.parent().is_some_and(|parent| {
                            parent.is_element() && parent.tag_name().name() == "Properties"
                        })
                });
                let spec = metadata_relation_spec(*kind, *relation);
                assert_eq!(has_node, spec.is_some(), "{} {tag}", kind.as_str());

                let owner = format!("{}.Owner", kind.as_str());
                let target = match spec.map(|spec| spec.target_policy) {
                    Some(MetaRelationTargetPolicy::MetadataKinds(kinds)) => {
                        MetaRelationTarget::Object(metadata_reference(&format!(
                            "{}.Target",
                            kinds[0].as_str()
                        )))
                    }
                    Some(MetaRelationTargetPolicy::SameOwnerField) => {
                        let field = metadata_standard_attribute_names(kind.as_str())
                            .first()
                            .copied()
                            .expect("every inputByString owner has a standard field");
                        MetaRelationTarget::Field(
                            crate::domain::metadata::MetadataFieldPath::parse(&format!(
                                "{owner}.StandardAttribute.{field}"
                            ))
                            .unwrap(),
                        )
                    }
                    Some(MetaRelationTargetPolicy::EventSources) => {
                        MetaRelationTarget::EventSource(catalog_family())
                    }
                    None => match relation {
                        MetaRelation::InputByString => MetaRelationTarget::Field(
                            crate::domain::metadata::MetadataFieldPath::parse(&format!(
                                "{owner}.StandardAttribute.Name"
                            ))
                            .unwrap(),
                        ),
                        MetaRelation::Owners => {
                            MetaRelationTarget::Object(metadata_reference("Catalog.Target"))
                        }
                        MetaRelation::RegisterRecords => MetaRelationTarget::Object(
                            metadata_reference("InformationRegister.Target"),
                        ),
                        MetaRelation::BasedOn => MetaRelationTarget::Object(metadata_reference(
                            &format!("{}.Target", kind.as_str()),
                        )),
                        MetaRelation::Source => MetaRelationTarget::EventSource(catalog_family()),
                    },
                };
                let operation = MetaEditOperation::edit_relation_targets(
                    *relation,
                    RelationEditMode::Replace,
                    vec![target],
                )
                .unwrap();
                let mut working = template.clone();
                let result = apply_typed_operations(&mut working, &[operation]);
                if spec.is_some() {
                    result.unwrap_or_else(|failure| {
                        panic!("{} {tag}: {:?}", kind.as_str(), failure.diagnostics)
                    });
                    assert_ne!(working, template, "{} {tag}", kind.as_str());
                } else {
                    let failure = result.expect_err(&format!("{} {tag}", kind.as_str()));
                    assert_eq!(
                        failure.diagnostics[0].field.as_deref(),
                        Some("operations[0].relation")
                    );
                    assert_eq!(working, template, "{} {tag}", kind.as_str());
                }
            }
        }
    }

    #[test]
    fn typed_fill_value_registry_guards_every_public_field_context_before_mutation() {
        use crate::domain::metadata::{metadata_fill_value_is_allowed, MetaElementScope};

        let filled = |name: &str| MetaElementInput {
            name: name.into(),
            r#type: Some(
                MetadataType::new(vec![MetadataTypeVariant::String {
                    length: 20,
                    allowed_length: StringLengthMode::Variable,
                }])
                .unwrap(),
            ),
            fill_value: Some(MetaFillValue::String("value".into())),
            ..MetaElementInput::default()
        };

        for kind in MetadataKind::ALL {
            for collection in [
                MetaCollection::Attributes,
                MetaCollection::Dimensions,
                MetaCollection::Resources,
            ] {
                if !crate::domain::metadata::metadata_kind_collections(*kind).contains(&collection)
                {
                    continue;
                }
                let (template, _) =
                    super::super::template_catalog::minimal_metadata_xml_for_tests(*kind, "Owner")
                        .unwrap();
                let operation =
                    MetaEditOperation::add(collection, None, vec![filled("Filled")]).unwrap();
                let mut working = template.clone();
                let result = apply_typed_operations(&mut working, &[operation]);
                let allowed =
                    metadata_fill_value_is_allowed(*kind, collection, MetaElementScope::TopLevel);
                if allowed {
                    result.unwrap_or_else(|failure| {
                        panic!(
                            "{} {}: {:?}",
                            kind.as_str(),
                            collection.as_str(),
                            failure.diagnostics
                        )
                    });
                    assert!(working.contains("<FillValue xsi:type=\"xs:string\">value</FillValue>"));
                } else {
                    let failure =
                        result.expect_err(&format!("{} {}", kind.as_str(), collection.as_str()));
                    assert_eq!(
                        failure.diagnostics[0].field.as_deref(),
                        Some("operations[0].elements[0].fillValue")
                    );
                    assert_eq!(working, template);
                }
            }

            if !crate::domain::metadata::metadata_kind_collections(*kind)
                .contains(&MetaCollection::TabularSections)
            {
                continue;
            }
            let (template, _) =
                super::super::template_catalog::minimal_metadata_xml_for_tests(*kind, "Owner")
                    .unwrap();
            let operation = MetaEditOperation::add(
                MetaCollection::TabularSections,
                None,
                vec![MetaElementInput {
                    name: "Lines".into(),
                    attributes: Some(vec![filled("Filled")]),
                    ..MetaElementInput::default()
                }],
            )
            .unwrap();
            let mut working = template.clone();
            let result = apply_typed_operations(&mut working, &[operation]);
            let allowed = metadata_fill_value_is_allowed(
                *kind,
                MetaCollection::Attributes,
                MetaElementScope::TabularSection,
            );
            if allowed {
                result.unwrap_or_else(|failure| {
                    panic!("{} scoped: {:?}", kind.as_str(), failure.diagnostics)
                });
                assert!(working.contains("<FillValue xsi:type=\"xs:string\">value</FillValue>"));
            } else {
                let failure = result.expect_err(&format!("{} scoped", kind.as_str()));
                assert_eq!(
                    failure.diagnostics[0].field.as_deref(),
                    Some("operations[0].elements[0].attributes[0].fillValue")
                );
                assert_eq!(working, template);
            }
        }
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
        assert_eq!(
            failure.diagnostics[0].field.as_deref(),
            Some("operations[0].scope.tabularSection")
        );
    }

    #[test]
    fn typed_scoped_add_missing_anchor_reports_the_exact_element_position() {
        let mut xml = object_xml("Document", "Order", "<RegisterRecords/>");
        let operations = [
            MetaEditOperation::add(
                MetaCollection::TabularSections,
                None,
                vec![MetaElementInput {
                    name: "Lines".into(),
                    attributes: Some(vec![MetaElementInput::named("Existing")]),
                    ..MetaElementInput::default()
                }],
            )
            .unwrap(),
            MetaEditOperation::add(
                MetaCollection::Attributes,
                Some(MetaScope {
                    tabular_section: "Lines".into(),
                }),
                vec![MetaElementInput {
                    name: "Inserted".into(),
                    position: Some(MetaPosition::new(Some("Missing".into()), None).unwrap()),
                    ..MetaElementInput::default()
                }],
            )
            .unwrap(),
        ];

        let failure = apply_typed_operations(&mut xml, &operations).unwrap_err();

        assert_eq!(failure.diagnostics[0].operation_index, Some(1));
        assert_eq!(
            failure.diagnostics[0].field.as_deref(),
            Some("operations[1].elements[0].position")
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
        let mut xml = object_xml("Report", "Sales", "");
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
    fn typed_composite_type_canonicalizes_qualifiers_for_variant_permutations() {
        let metadata_type = MetadataType::new(vec![
            MetadataTypeVariant::Date {
                fractions: DateFractions::DateTime,
            },
            MetadataTypeVariant::BinaryData {
                length: 512,
                allowed_length: StringLengthMode::Fixed,
            },
            MetadataTypeVariant::String {
                length: 40,
                allowed_length: StringLengthMode::Variable,
            },
            MetadataTypeVariant::Number {
                digits: 12,
                fraction: 2,
                sign: NumberSign::Any,
            },
        ])
        .unwrap();
        let mut lines = Vec::new();

        emit_meta_typed_value_type(&mut lines, "", &metadata_type);

        let xml = lines.join("\n");
        let number = xml.find("<v8:NumberQualifiers>").unwrap();
        let string = xml.find("<v8:StringQualifiers>").unwrap();
        let date = xml.find("<v8:DateQualifiers>").unwrap();
        let binary = xml.find("<v8:BinaryDataQualifiers>").unwrap();
        assert!(number < string && string < date && date < binary, "{xml}");
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
    fn typed_fill_value_legality_follows_owner_and_scope_context_for_add_and_update() {
        let string_type = || {
            MetadataType::new(vec![MetadataTypeVariant::String {
                length: 20,
                allowed_length: StringLengthMode::Variable,
            }])
            .unwrap()
        };
        let filled = |name: &str| MetaElementInput {
            name: name.into(),
            r#type: Some(string_type()),
            fill_value: Some(MetaFillValue::String("value".into())),
            ..MetaElementInput::default()
        };

        let mut report = object_xml("Report", "Sales", "");
        let failure = apply_typed_operations(
            &mut report,
            &[
                MetaEditOperation::add(MetaCollection::Attributes, None, vec![filled("Top")])
                    .unwrap(),
            ],
        )
        .unwrap_err();
        assert_eq!(
            failure.diagnostics[0].field.as_deref(),
            Some("operations[0].elements[0].fillValue")
        );
        assert!(!report.contains("<FillValue xsi:type=\"xs:string\">value</FillValue>"));

        let mut catalog = object_xml("Catalog", "Items", "");
        apply_typed_operations(
            &mut catalog,
            &[
                MetaEditOperation::add(MetaCollection::Attributes, None, vec![filled("Top")])
                    .unwrap(),
            ],
        )
        .unwrap();
        assert!(catalog.contains("<FillValue xsi:type=\"xs:string\">value</FillValue>"));

        let mut stored_tabular = object_xml("Catalog", "Items", "");
        let failure = apply_typed_operations(
            &mut stored_tabular,
            &[MetaEditOperation::add(
                MetaCollection::TabularSections,
                None,
                vec![MetaElementInput {
                    name: "Lines".into(),
                    attributes: Some(vec![filled("Value")]),
                    ..MetaElementInput::default()
                }],
            )
            .unwrap()],
        )
        .unwrap_err();
        assert_eq!(
            failure.diagnostics[0].field.as_deref(),
            Some("operations[0].elements[0].attributes[0].fillValue")
        );
        assert!(!stored_tabular.contains("<FillValue xsi:type=\"xs:string\">value</FillValue>"));

        let mut report_tabular = object_xml("Report", "Sales", "");
        apply_typed_operations(
            &mut report_tabular,
            &[MetaEditOperation::add(
                MetaCollection::TabularSections,
                None,
                vec![MetaElementInput {
                    name: "Lines".into(),
                    attributes: Some(vec![filled("Value")]),
                    ..MetaElementInput::default()
                }],
            )
            .unwrap()],
        )
        .unwrap();
        assert!(report_tabular.contains("<FillValue xsi:type=\"xs:string\">value</FillValue>"));

        let mut report_update = object_xml("Report", "Sales", "");
        let operations = [
            MetaEditOperation::add(
                MetaCollection::Attributes,
                None,
                vec![MetaElementInput {
                    name: "Top".into(),
                    r#type: Some(string_type()),
                    ..MetaElementInput::default()
                }],
            )
            .unwrap(),
            MetaEditOperation::update(
                MetaCollection::Attributes,
                None,
                vec![MetaElementUpdateInput {
                    name: "Top".into(),
                    fill_value: Some(MetaFillValue::String("value".into())),
                    ..MetaElementUpdateInput::default()
                }],
            )
            .unwrap(),
        ];
        let failure = apply_typed_operations(&mut report_update, &operations).unwrap_err();
        assert_eq!(
            failure.diagnostics[0].field.as_deref(),
            Some("operations[1].elements[0].fillValue")
        );
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
    fn fill_value_only_update_preserves_inherited_type_namespace_aliases() {
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
        apply_typed_operations(&mut xml, &[add]).unwrap();
        xml = xml
            .replace(
                "xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\"",
                "xmlns:schema=\"http://www.w3.org/2001/XMLSchema\" xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\"",
            )
            .replace(
                "xmlns:v8=\"http://v8.1c.ru/8.1/data/core\"",
                "xmlns:core=\"http://v8.1c.ru/8.1/data/core\"",
            )
            .replace("<v8:", "<core:")
            .replace("</v8:", "</core:")
            .replace(">xs:string<", ">schema:string<");
        let document = roxmltree::Document::parse(&xml).unwrap();
        let observed_type = document
            .descendants()
            .find(|node| {
                node.is_element()
                    && node.tag_name().namespace() == Some("http://v8.1c.ru/8.1/data/core")
                    && node.tag_name().name() == "Type"
            })
            .unwrap();
        assert_eq!(
            observed_type.lookup_namespace_uri(Some("schema")),
            Some("http://www.w3.org/2001/XMLSchema")
        );
        let update = MetaEditOperation::update(
            MetaCollection::Attributes,
            None,
            vec![MetaElementUpdateInput {
                name: "CodeText".into(),
                fill_value: Some(MetaFillValue::String("ok".into())),
                ..MetaElementUpdateInput::default()
            }],
        )
        .unwrap();

        apply_typed_operations(&mut xml, &[update]).unwrap();

        assert!(xml.contains("schema:string"));
        assert!(xml.contains("xsi:type=\"xs:string\">ok</FillValue>"));
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
        assert_eq!(resources.validation_resources.len(), 2);

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
    fn typed_command_add_does_not_invent_a_standalone_descriptor() {
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

        assert!(
            resources.file_mutations.is_empty(),
            "platform commands are inline owner descriptors: {:?}",
            resources.file_mutations
        );
        assert_eq!(resources.publication_plan.len(), 1);
        assert_eq!(
            resources.publication_plan[0]
                .metadata_path
                .as_ref()
                .unwrap()
                .as_str(),
            "Document.Order.Command.Open"
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn child_observation_localizes_an_invalid_address_and_keeps_valid_siblings() {
        let root = std::env::temp_dir().join(format!(
            "unica-meta-observe-invalid-child-address-{}",
            uuid::Uuid::new_v4()
        ));
        let descriptor_path = root.join("Catalogs/Items.xml");
        let owner = metadata_reference("Catalog.Items").metadata_path;
        let owner_xml = object_xml("Catalog", "Items", "").replace(
            "<ChildObjects/>",
            r#"<ChildObjects>
			<Form>Bad..Name</Form>
			<Command uuid="22222222-2222-4222-8222-222222222222">
				<Properties><Name>Refresh</Name></Properties>
			</Command>
		</ChildObjects>"#,
        );

        let observation =
            observe_typed_child_resources(&descriptor_path, &owner, "Catalog", "Items", &owner_xml)
                .unwrap();

        assert!(observation.diagnostics.iter().any(|diagnostic| {
            diagnostic.message == "typed child logical address cannot be represented"
        }));
        assert!(observation.validation_resources.iter().any(|resource| {
            matches!(
                &resource.role,
                MetadataResourceRole::Command { owner, name }
                    if owner.as_str() == "Catalog.Items" && name == "Refresh"
            )
        }));
    }

    #[test]
    fn typed_last_form_remove_plans_one_whole_collection_tree_removal() {
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

        assert_eq!(resources.file_mutations.len(), 1);
        assert_eq!(
            resources.file_mutations[0].path,
            root.join("Documents/Order/Forms")
        );
        assert!(resources.file_mutations[0].post_image.is_none());
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
    fn typed_child_footprint_profile_matrix_rejects_every_missing_or_extra_member() {
        let child = metadata_reference("Catalog.Editable.Template.Main").metadata_path;
        let cases = vec![
            (
                "form",
                MetaCollection::Forms,
                None,
                vec!["Ext/Form.xml"],
                vec!["", "Ext"],
                "Ext/Template.xml",
            ),
            (
                "command-module",
                MetaCollection::Commands,
                None,
                vec!["Ext/CommandModule.bsl"],
                vec!["", "Ext"],
                "Ext/Form.xml",
            ),
            (
                "spreadsheet",
                MetaCollection::Templates,
                Some(MetadataTemplateType::SpreadsheetDocument),
                vec!["Ext/Template.xml"],
                vec!["", "Ext"],
                "Ext/Template.txt",
            ),
            (
                "schema",
                MetaCollection::Templates,
                Some(MetadataTemplateType::DataCompositionSchema),
                vec!["Ext/Template.xml"],
                vec!["", "Ext"],
                "Ext/Template.bin",
            ),
            (
                "text",
                MetaCollection::Templates,
                Some(MetadataTemplateType::TextDocument),
                vec!["Ext/Template.txt"],
                vec!["", "Ext"],
                "Ext/Template.xml",
            ),
            (
                "binary",
                MetaCollection::Templates,
                Some(MetadataTemplateType::BinaryData),
                vec!["Ext/Template.bin"],
                vec!["", "Ext"],
                "Ext/Template.xml",
            ),
            (
                "html",
                MetaCollection::Templates,
                Some(MetadataTemplateType::HtmlDocument),
                vec!["Ext/Template.xml", "Ext/Template/ru.html"],
                vec!["", "Ext", "Ext/Template"],
                "Ext/Template/de.html",
            ),
        ];

        for (label, collection, template_type, file_names, directory_names, wrong_part) in cases {
            let mut files = file_names
                .iter()
                .map(|path| (PathBuf::from(path), Vec::new()))
                .collect::<Vec<_>>();
            if template_type == Some(MetadataTemplateType::HtmlDocument) {
                files
                    .iter_mut()
                    .find(|(path, _)| path == Path::new("Ext/Template.xml"))
                    .unwrap()
                    .1 = br#"<Help xmlns="http://v8.1c.ru/8.3/xcf/extrnprops" version="2.20"><Page>ru</Page></Help>"#
                    .to_vec();
            }
            let directories = directory_names
                .iter()
                .map(PathBuf::from)
                .collect::<Vec<_>>();
            validate_typed_child_footprint(collection, template_type, &child, &files, &directories)
                .unwrap_or_else(|failure| panic!("valid {label} footprint failed: {failure:?}"));

            let mut missing_file = files.clone();
            missing_file.pop();
            assert!(
                validate_typed_child_footprint(
                    collection,
                    template_type,
                    &child,
                    &missing_file,
                    &directories,
                )
                .is_err(),
                "{label} accepted a missing file"
            );

            let mut extra_file = files.clone();
            extra_file.push((PathBuf::from("Ext/Unexpected.bin"), Vec::new()));
            assert!(
                validate_typed_child_footprint(
                    collection,
                    template_type,
                    &child,
                    &extra_file,
                    &directories,
                )
                .is_err(),
                "{label} accepted an extra file"
            );

            let mut wrong_file_role = files.clone();
            *wrong_file_role.last_mut().unwrap() = (PathBuf::from(wrong_part), Vec::new());
            assert!(
                validate_typed_child_footprint(
                    collection,
                    template_type,
                    &child,
                    &wrong_file_role,
                    &directories,
                )
                .is_err(),
                "{label} accepted a wrong file role or part"
            );

            let mut missing_directory = directories.clone();
            missing_directory.pop();
            assert!(
                validate_typed_child_footprint(
                    collection,
                    template_type,
                    &child,
                    &files,
                    &missing_directory,
                )
                .is_err(),
                "{label} accepted a missing directory"
            );

            let mut extra_directory = directories.clone();
            extra_directory.push(PathBuf::from("Ext/Unexpected"));
            assert!(
                validate_typed_child_footprint(
                    collection,
                    template_type,
                    &child,
                    &files,
                    &extra_directory,
                )
                .is_err(),
                "{label} accepted an extra directory"
            );
        }

        validate_typed_child_footprint(MetaCollection::Commands, None, &child, &[], &[])
            .expect("descriptor-only Command is a valid closed footprint");
    }

    #[test]
    fn html_template_footprint_follows_all_declared_pages() {
        let child = metadata_reference("Catalog.Editable.Template.Main").metadata_path;
        let files = vec![
            (
                PathBuf::from("Ext/Template.xml"),
                br#"<Help xmlns="http://v8.1c.ru/8.3/xcf/extrnprops" version="2.20"><Page>ru</Page><Page>en</Page></Help>"#.to_vec(),
            ),
            (
                PathBuf::from("Ext/Template/ru.html"),
                b"<html><body>ru</body></html>".to_vec(),
            ),
            (
                PathBuf::from("Ext/Template/en.html"),
                b"<html><body>en</body></html>".to_vec(),
            ),
        ];
        let directories = vec![
            PathBuf::new(),
            PathBuf::from("Ext"),
            PathBuf::from("Ext/Template"),
        ];

        validate_typed_child_footprint(
            MetaCollection::Templates,
            Some(MetadataTemplateType::HtmlDocument),
            &child,
            &files,
            &directories,
        )
        .unwrap();

        let mut missing_page = files;
        missing_page.pop();
        assert!(validate_typed_child_footprint(
            MetaCollection::Templates,
            Some(MetadataTemplateType::HtmlDocument),
            &child,
            &missing_page,
            &directories,
        )
        .is_err());
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
        assert_eq!(resources.file_mutations.len(), 1);
        assert_eq!(
            resources.file_mutations[0].path,
            root.join("Documents/Order/Forms")
        );
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
        assert_eq!(resources.file_mutations.len(), 1);
        assert_eq!(
            resources.file_mutations[0].path,
            root.join("Documents/Order/Forms")
        );
        assert_eq!(
            resources.publication_plan[0].action,
            MetaPublicationAction::Remove
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn typed_existing_form_remove_then_add_is_one_replacement_not_create_on_existing() {
        let root = std::env::temp_dir().join(format!(
            "unica-meta-edit-form-remove-add-{}",
            uuid::Uuid::new_v4()
        ));
        let descriptor_path = root.join("Documents/Order.xml");
        let child_descriptor = root.join("Documents/Order/Forms/A.xml");
        let form_content = root.join("Documents/Order/Forms/A/Ext/Form.xml");
        std::fs::create_dir_all(form_content.parent().unwrap()).unwrap();
        let mut pre_image = object_xml("Document", "Order", "<RegisterRecords/>");
        let initial_add = MetaEditOperation::add(
            MetaCollection::Forms,
            None,
            vec![MetaElementInput::named("A")],
        )
        .unwrap();
        apply_typed_operations(&mut pre_image, &[initial_add]).unwrap();
        std::fs::write(
            &child_descriptor,
            typed_child_descriptor_image(&pre_image, "Form", "A").unwrap(),
        )
        .unwrap();
        std::fs::write(&form_content, b"<Form version=\"old\"/>").unwrap();
        let operations = [
            MetaEditOperation::remove(MetaCollection::Forms, None, vec!["A".into()]).unwrap(),
            MetaEditOperation::add(
                MetaCollection::Forms,
                None,
                vec![MetaElementInput {
                    name: "A".into(),
                    comment: Some("replacement".into()),
                    ..MetaElementInput::default()
                }],
            )
            .unwrap(),
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
            MetaPublicationAction::Update
        );
        assert!(resources.file_mutations.iter().any(|mutation| {
            mutation.path == child_descriptor
                && mutation.pre_image.is_some()
                && mutation.post_image.is_some()
        }));
        assert!(resources.file_mutations.iter().any(|mutation| {
            mutation.path == form_content
                && mutation.pre_image.is_some()
                && mutation.post_image.is_some()
        }));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn typed_existing_form_remove_add_remove_deletes_only_the_initial_tree() {
        let root = std::env::temp_dir().join(format!(
            "unica-meta-edit-form-remove-add-remove-{}",
            uuid::Uuid::new_v4()
        ));
        let descriptor_path = root.join("Documents/Order.xml");
        let child_descriptor = root.join("Documents/Order/Forms/A.xml");
        let payload_dir = root.join("Documents/Order/Forms/A");
        std::fs::create_dir_all(payload_dir.join("Ext")).unwrap();
        let mut pre_image = object_xml("Document", "Order", "<RegisterRecords/>");
        let initial_add = MetaEditOperation::add(
            MetaCollection::Forms,
            None,
            vec![MetaElementInput::named("A")],
        )
        .unwrap();
        apply_typed_operations(&mut pre_image, &[initial_add]).unwrap();
        std::fs::write(
            &child_descriptor,
            typed_child_descriptor_image(&pre_image, "Form", "A").unwrap(),
        )
        .unwrap();
        std::fs::write(payload_dir.join("Ext/Form.xml"), b"<Form/>").unwrap();
        let operations = [
            MetaEditOperation::remove(MetaCollection::Forms, None, vec!["A".into()]).unwrap(),
            MetaEditOperation::add(
                MetaCollection::Forms,
                None,
                vec![MetaElementInput::named("A")],
            )
            .unwrap(),
            MetaEditOperation::remove(MetaCollection::Forms, None, vec!["A".into()]).unwrap(),
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
        assert_eq!(resources.file_mutations.len(), 1);
        assert_eq!(
            resources.file_mutations[0].path,
            root.join("Documents/Order/Forms")
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    pub(crate) fn typed_exact_noop_form_update_preserves_resource_bytes_and_identities() {
        use crate::infrastructure::platform::testing::file_identity_for_test;

        let root = std::env::temp_dir().join(format!(
            "unica-meta-edit-form-noop-{}",
            uuid::Uuid::new_v4()
        ));
        let descriptor_path = root.join("Documents/Order.xml");
        let child_descriptor = root.join("Documents/Order/Forms/A.xml");
        std::fs::create_dir_all(child_descriptor.parent().unwrap()).unwrap();
        let mut pre_image = object_xml("Document", "Order", "<RegisterRecords/>");
        let initial_add = MetaEditOperation::add(
            MetaCollection::Forms,
            None,
            vec![MetaElementInput::named("A")],
        )
        .unwrap();
        apply_typed_operations(&mut pre_image, &[initial_add]).unwrap();
        let child_bytes = typed_child_descriptor_image(&pre_image, "Form", "A").unwrap();
        std::fs::write(&child_descriptor, &child_bytes).unwrap();
        let form_content = root.join("Documents/Order/Forms/A/Ext/Form.xml");
        std::fs::create_dir_all(form_content.parent().unwrap()).unwrap();
        std::fs::write(
            &form_content,
            minimal_typed_form_content("Document", "Order").as_bytes(),
        )
        .unwrap();
        let child_before = std::fs::read(&child_descriptor).unwrap();
        let content_before = std::fs::read(&form_content).unwrap();
        let child_identity = file_identity_for_test(&child_descriptor).unwrap();
        let content_identity = file_identity_for_test(&form_content).unwrap();
        let operation = MetaEditOperation::update(
            MetaCollection::Forms,
            None,
            vec![MetaElementUpdateInput {
                name: "A".into(),
                comment: Some(String::new()),
                ..MetaElementUpdateInput::default()
            }],
        )
        .unwrap();
        let mut post_image = pre_image;
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

        assert!(resources.file_mutations.is_empty());
        assert!(resources.publication_plan.is_empty());
        assert_eq!(std::fs::read(&child_descriptor).unwrap(), child_before);
        assert_eq!(std::fs::read(&form_content).unwrap(), content_before);
        assert_eq!(
            file_identity_for_test(&child_descriptor).unwrap(),
            child_identity
        );
        assert_eq!(
            file_identity_for_test(&form_content).unwrap(),
            content_identity
        );
        let _ = std::fs::remove_dir_all(root);
    }

    /// Every payload member the platform actually writes beside a managed form,
    /// measured over a real 8.3.27 Designer dump. `Ext/Form/Module.bsl` is the
    /// module (not `Ext/Module.bsl`); `Ext/Help*` is form help; the pictures
    /// under `Ext/Form/Items` belong to individual form items.
    const REAL_FORM_PAYLOADS: &[(&str, &[&str])] = &[
        ("bare", &[]),
        ("module", &["Ext/Form/Module.bsl"]),
        (
            "module-and-help",
            &["Ext/Form/Module.bsl", "Ext/Help.xml", "Ext/Help/ru.html"],
        ),
        (
            "help-assets",
            &[
                "Ext/Help.xml",
                "Ext/Help/ru.html",
                "Ext/Help/_files/note.png",
            ],
        ),
        (
            "item-pictures",
            &[
                "Ext/Form/Module.bsl",
                "Ext/Form/Items/Список/Picture.png",
                "Ext/Form/Items/Список/RowsPicture.png",
            ],
        ),
    ];

    fn write_designer_form_object(root: &Path, extra_payload: &[&str]) -> String {
        let owner_image = object_xml("Catalog", "Users", "").replace(
            "<ChildObjects/>",
            "<ChildObjects><Form>ItemForm</Form></ChildObjects>",
        );
        let forms = root.join("Catalogs/Users/Forms");
        std::fs::create_dir_all(forms.join("ItemForm/Ext")).unwrap();
        std::fs::write(root.join("Catalogs/Users.xml"), &owner_image).unwrap();
        std::fs::write(
            forms.join("ItemForm.xml"),
            object_xml("Form", "ItemForm", "<FormType>Managed</FormType>")
                .replace("\n\t\t<ChildObjects/>", ""),
        )
        .unwrap();
        std::fs::write(
            forms.join("ItemForm/Ext/Form.xml"),
            concat!(
                "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n",
                "<Form xmlns=\"http://v8.1c.ru/8.3/xcf/logform\" version=\"2.20\">\n",
                "\t<AutoCommandBar name=\"ФормаКоманднаяПанель\" id=\"-1\"/>\n",
                "</Form>"
            ),
        )
        .unwrap();
        for relative in extra_payload {
            let path = forms.join("ItemForm").join(relative);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, b"payload").unwrap();
        }
        owner_image
    }

    fn designer_form_child_diagnostics(root: &Path, owner_image: &str) -> Vec<String> {
        let owner = metadata_reference("Catalog.Users").metadata_path;
        let plan = match plan_typed_child_resources(
            &root.join("Catalogs/Users.xml"),
            &owner,
            "Catalog",
            "Users",
            &[],
            owner_image,
        ) {
            Ok(plan) => plan,
            Err(failure) => {
                return failure
                    .diagnostics
                    .iter()
                    .map(|diagnostic| diagnostic.message.clone())
                    .collect()
            }
        };
        let mut resources = vec![
            crate::application::ports::MetadataResourceImage {
                role: MetadataResourceRole::Descriptor,
                bytes: owner_image.as_bytes().to_vec(),
            },
            crate::application::ports::MetadataResourceImage {
                role: MetadataResourceRole::Registration,
                bytes: br#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20">
<Configuration uuid="22222222-2222-4222-8222-222222222222">
<Properties><Name>Owner</Name></Properties><ChildObjects><Catalog>Users</Catalog></ChildObjects>
</Configuration></MetaDataObject>"#
                    .to_vec(),
            },
        ];
        resources.extend(plan.validation_resources);
        let subject = crate::application::ports::MetadataValidationSubject {
            target: owner,
            resources,
            child_footprints: plan.validation_footprints,
            registrar_evidence: Default::default(),
            subsystem_evidence: Default::default(),
        };
        let context = WorkspaceContext {
            cwd: root.to_path_buf(),
            workspace_root: root.to_path_buf(),
            cache_root: root.join(".unica/cache"),
            workspace_epoch: 1,
        };

        // Keep only what this regression is about: the object itself is a
        // hand-written stub, so its own completeness diagnostics are noise.
        super::super::validation::MetadataValidator
            .validate(&subject, &context)
            .diagnostics
            .iter()
            .filter(|diagnostic| {
                diagnostic
                    .metadata_path
                    .as_ref()
                    .is_some_and(|path| path.as_str().contains(".Form."))
            })
            .map(|diagnostic| diagnostic.message.clone())
            .collect()
    }

    /// Regression for #360. A Designer dump registers a form in the owner's
    /// `ChildObjects` as a bare `<Form>Name</Form>` element and keeps the real
    /// descriptor in `Forms/Name.xml`. Reading such an object must succeed for
    /// every payload shape the platform actually writes.
    #[test]
    fn real_designer_form_payloads_validate_without_child_diagnostics() {
        for (label, extra_payload) in REAL_FORM_PAYLOADS {
            let root = std::env::temp_dir().join(format!(
                "unica-meta-issue-360-{label}-{}",
                uuid::Uuid::new_v4()
            ));
            std::fs::create_dir_all(&root).unwrap();
            let owner_image = write_designer_form_object(&root, extra_payload);

            let diagnostics = designer_form_child_diagnostics(&root, &owner_image);

            let _ = std::fs::remove_dir_all(&root);
            assert_eq!(diagnostics, Vec::<String>::new(), "{label}");
        }
    }

    /// The closed payload guard still has to reject bytes the platform never
    /// writes, otherwise a mutation would silently clobber them.
    #[test]
    fn unmodelled_form_payload_members_are_still_rejected() {
        for (label, extra_payload) in [
            ("stray-root-file", "stray.txt"),
            ("legacy-module-path", "Ext/Module.bsl"),
            ("stray-ext-file", "Ext/Unexpected.xml"),
        ] {
            let root = std::env::temp_dir().join(format!(
                "unica-meta-issue-360-reject-{label}-{}",
                uuid::Uuid::new_v4()
            ));
            std::fs::create_dir_all(&root).unwrap();
            let owner_image = write_designer_form_object(&root, &[extra_payload]);

            let diagnostics = designer_form_child_diagnostics(&root, &owner_image);

            let _ = std::fs::remove_dir_all(&root);
            assert!(!diagnostics.is_empty(), "{label} was accepted");
        }
    }
}
