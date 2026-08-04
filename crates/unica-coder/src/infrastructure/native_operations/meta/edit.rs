use crate::application::metadata::{MetaEditRequest, MetaFailure};
use crate::application::ports::{
    MetadataChildDirectoryKind, MetadataChildFootprintEvidence, MetadataChildProfile,
    MetadataChildResourceKind, MetadataResourceImage, MetadataResourceRole,
    MetadataTemplateResourcePart, MetadataTemplateType, PreparedMetadataMutation,
};
use crate::domain::cancellation::CancellationToken;
use crate::domain::format_profile::ACTIVE_FORMAT_PROFILE;
use crate::domain::metadata::{
    DateFractions, MetaCollection, MetaDiagnostic, MetaDiagnosticCode, MetaEditOperation,
    MetaElementDefinition, MetaElementUpdate, MetaFillValue, MetaMutationEffect, MetaPosition,
    MetaPropertyKey, MetaPropertyValue, MetaPublicationAction, MetaPublicationPlanEntry,
    MetaPublicationResource, MetaRelation, MetadataType, MetadataTypeVariant, NumberSign,
    RelationEditMode, StringLengthMode,
};
use crate::domain::source_target::{
    MetadataAddress, SourceTarget, TargetKind, PLATFORM_XML_8_3_27_FORMAT_2_20,
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
use std::path::{Path, PathBuf};

use super::super::common::{absolutize, escape_xml, is_1c_identifier};
use super::super::compile_transaction::{
    snapshot_directory_membership, DirectoryMembershipSelector, DirectoryMembershipSnapshot,
};
use super::format_contract::{
    validate_metadata_8_3_27_boolean_contract, validate_metadata_8_3_27_enum_contract,
};
use super::publisher::{fresh_metadata_uuid, PreparedMetaEdit};
use super::template_catalog::{
    emit_meta_attribute, emit_meta_enum_value, emit_meta_register_field, emit_meta_tabular_section,
    meta_attribute_context, metadata_generated_types_8_3_27, metadata_standard_attribute_names,
    split_meta_camel_case, MetadataAttributeTemplate, MetadataEnumValueTemplate,
    MetadataTabularSectionTemplate,
};
use super::xml_model::{
    emit_meta_mltext, emit_meta_typed_fill_value, emit_meta_typed_value_type, meta_info_child,
    meta_info_child_text, meta_info_children, meta_info_inner_text,
};

#[derive(Debug, Default)]
pub(super) struct MetaEditCounts {
    pub(crate) added: usize,
    pub(crate) modified: usize,
    pub(crate) removed: usize,
    pub(crate) effects: Vec<MetaMutationEffect>,
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

pub(crate) struct ResolvedMetadataObject {
    pub(super) handle: ClosedPlatformXmlTarget,
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
    pub(super) exact_file_guards: Vec<(PathBuf, Vec<u8>)>,
    pub(super) directory_guards: Vec<(PathBuf, DirectoryMembershipSnapshot)>,
    pub(super) publication_plan: Vec<MetaPublicationPlanEntry>,
    pub(super) validation_resources: Vec<MetadataResourceImage>,
    pub(super) validation_footprints: Vec<MetadataChildFootprintEvidence>,
    pub(super) relation_dependencies: Vec<TypedRelationDependency>,
}

#[derive(Debug)]
pub(super) struct TypedRelationDependency {
    pub(super) handle: ClosedPlatformXmlTarget,
    pub(super) path: PathBuf,
    pub(super) bytes: Vec<u8>,
    pub(super) target: MetadataAddress,
}

pub(super) struct TypedOperationPostImage {
    pub(super) descriptor: Vec<u8>,
    pub(super) child_resources: TypedChildResourcePlan,
    pub(super) effects: Vec<MetaMutationEffect>,
}

/// Build the complete descriptor and child-resource plan privately. No
/// transaction mutation is registered until this function returns success.
pub(super) fn build_typed_operation_post_image(
    source_set: &str,
    descriptor_path: &Path,
    target: &MetadataAddress,
    descriptor_preimage: &[u8],
    operations: &[MetaEditOperation],
    context: &WorkspaceContext,
) -> Result<TypedOperationPostImage, MetaFailure> {
    let mut xml = String::from_utf8(descriptor_preimage.to_vec()).map_err(|_| {
        MetaFailure::from(
            typed_diagnostic(
                MetaDiagnosticCode::ProviderUnavailable,
                "metadata descriptor image is not UTF-8",
                None,
            )
            .with_metadata_path(target.clone()),
        )
    })?;
    let source_format = MetaEditSourceFormat {
        has_bom: descriptor_preimage.starts_with(b"\xef\xbb\xbf"),
        eol: meta_edit_source_eol(&xml),
    };
    if xml.starts_with('\u{feff}') {
        xml = xml.trim_start_matches('\u{feff}').to_string();
    }
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
    child_resources.relation_dependencies =
        resolve_typed_relation_dependencies(source_set, target, operations, context, &xml)?;
    Ok(TypedOperationPostImage {
        descriptor: meta_edit_preserve_source_format(&xml, source_format),
        child_resources,
        effects: applied.effects,
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
                        });
                        index
                    });
                    let next_name = element
                        .new_name
                        .clone()
                        .unwrap_or_else(|| element.name.clone());
                    states[index].current_name = Some(next_name.clone());
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
                        });
                        index
                    });
                    states[index].current_name = None;
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
        let (directory, _) = typed_physical_collection(collection).unwrap();
        let collection_dir = object_dir.join(directory);
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
        let (directory, resource) = typed_physical_collection(state.collection).unwrap();
        let collection_dir = object_dir.join(directory);
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
            let descriptor = collection_dir.join(format!("{initial_name}.xml"));
            let bytes = read_typed_child_file(&descriptor, owner)?;
            initial_descriptor = Some(bytes.clone());
            initial_files.insert(descriptor, bytes);
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
            final_files.insert(
                collection_dir.join(format!("{final_name}.xml")),
                bytes.clone(),
            );
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
                        let content =
                            super::super::mxl::empty_spreadsheet_document_xml().into_bytes();
                        final_payload.push((PathBuf::from("Ext/Template.xml"), content));
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
                if initial_name != final_name
                    || initial_descriptor.as_ref() != final_descriptor.as_ref()
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
        plan.directory_guards.push((directory, snapshot));
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
    match collection {
        MetaCollection::Forms => Some(("Forms", MetaPublicationResource::Form)),
        MetaCollection::Templates => Some(("Templates", MetaPublicationResource::Template)),
        MetaCollection::Commands => Some(("Commands", MetaPublicationResource::Command)),
        _ => None,
    }
}

#[derive(Debug)]
struct TypedChildFootprintProfile {
    logical_profile: MetadataChildProfile,
    required_files: BTreeSet<PathBuf>,
    optional_files: BTreeSet<PathBuf>,
    required_directories: BTreeSet<PathBuf>,
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
            required_files: paths(&["Ext/Form.xml"]),
            optional_files: paths(&["Ext/Module.bsl"]),
            required_directories: paths(&["", "Ext"]),
        }),
        MetaCollection::Commands => Ok(TypedChildFootprintProfile {
            logical_profile: MetadataChildProfile::Command,
            required_files: BTreeSet::new(),
            optional_files: paths(&["Ext/CommandModule.bsl"]),
            required_directories: BTreeSet::new(),
        }),
        MetaCollection::Templates => {
            let template_type = template_type.ok_or_else(|| {
                typed_child_resource_failure(
                    child,
                    "template payload has no closed TemplateType evidence",
                )
            })?;
            let (required_files, required_directories) = match template_type {
                MetadataTemplateType::HtmlDocument => (
                    paths(&["Ext/Template.xml", "Ext/Template/ru.html"]),
                    paths(&["", "Ext", "Ext/Template"]),
                ),
                MetadataTemplateType::TextDocument => {
                    (paths(&["Ext/Template.txt"]), paths(&["", "Ext"]))
                }
                MetadataTemplateType::SpreadsheetDocument
                | MetadataTemplateType::DataCompositionSchema => {
                    (paths(&["Ext/Template.xml"]), paths(&["", "Ext"]))
                }
                MetadataTemplateType::BinaryData => {
                    (paths(&["Ext/Template.bin"]), paths(&["", "Ext"]))
                }
            };
            Ok(TypedChildFootprintProfile {
                logical_profile: MetadataChildProfile::Template(template_type),
                required_files,
                optional_files: BTreeSet::new(),
                required_directories,
            })
        }
        _ => unreachable!(),
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
    let observed_files = files
        .iter()
        .map(|(relative, _)| relative.clone())
        .collect::<BTreeSet<_>>();
    let allowed_files = profile
        .required_files
        .union(&profile.optional_files)
        .cloned()
        .collect::<BTreeSet<_>>();
    if !profile.required_files.is_subset(&observed_files)
        || !observed_files.is_subset(&allowed_files)
        || observed_files.len() != files.len()
    {
        return Err(typed_child_resource_failure(
            child,
            "child payload does not contain its exact required resource set",
        ));
    }

    let observed_directories = directories.iter().cloned().collect::<BTreeSet<_>>();
    let expected_directories = if collection == MetaCollection::Commands
        && observed_files.contains(Path::new("Ext/CommandModule.bsl"))
    {
        [PathBuf::new(), PathBuf::from("Ext")]
            .into_iter()
            .collect::<BTreeSet<_>>()
    } else {
        profile.required_directories.clone()
    };
    if observed_directories != expected_directories
        || observed_directories.len() != directories.len()
    {
        return Err(typed_child_topology_failure(
            child,
            "child payload does not contain its exact required directory topology",
        ));
    }

    let directories = observed_directories
        .iter()
        .map(|relative| {
            if relative.as_os_str().is_empty() {
                MetadataChildDirectoryKind::Root
            } else if relative == Path::new("Ext") {
                MetadataChildDirectoryKind::Extension
            } else if relative == Path::new("Ext/Template") {
                MetadataChildDirectoryKind::HtmlPages
            } else {
                unreachable!("closed directory footprint was checked above")
            }
        })
        .collect();
    Ok(MetadataChildFootprintEvidence {
        child: child.clone(),
        profile: profile.logical_profile,
        directories,
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
) -> Result<MetadataResourceRole, MetaFailure> {
    let kind = match collection {
        MetaCollection::Forms if relative_path == Path::new("Ext/Form.xml") => {
            MetadataChildResourceKind::FormContent
        }
        MetaCollection::Forms if relative_path == Path::new("Ext/Module.bsl") => {
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
                    if path == Path::new("Ext/Template/ru.html") =>
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

fn read_typed_child_tree(root: &Path) -> Result<TypedChildTree, TypedChildTreeError> {
    fn visit(
        root: &Path,
        current: &Path,
        files: &mut Vec<(PathBuf, Vec<u8>)>,
        directories: &mut Vec<PathBuf>,
    ) -> Result<(), TypedChildTreeError> {
        let entries = fs::read_dir(current).map_err(|_| TypedChildTreeError::Unavailable)?;
        for entry in entries {
            let entry = entry.map_err(|_| TypedChildTreeError::Unavailable)?;
            let file_type = entry
                .file_type()
                .map_err(|_| TypedChildTreeError::Unavailable)?;
            if file_type.is_symlink() {
                return Err(TypedChildTreeError::SymbolicLink);
            }
            if file_type.is_dir() {
                directories.push(entry.path().strip_prefix(root).unwrap().to_path_buf());
                visit(root, &entry.path(), files, directories)?;
            } else if file_type.is_file() {
                let path = entry.path();
                let bytes = fs::read(&path).map_err(|_| TypedChildTreeError::Unavailable)?;
                files.push((path.strip_prefix(root).unwrap().to_path_buf(), bytes));
            } else {
                return Err(TypedChildTreeError::UnsupportedNode);
            }
        }
        Ok(())
    }
    let mut files = Vec::new();
    let mut directories = vec![PathBuf::new()];
    visit(root, root, &mut files, &mut directories)?;
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
    let TypedOperationPostImage {
        descriptor: post_image,
        child_resources,
        effects,
    } = build_typed_operation_post_image(
        &request.source_set,
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
    source_set: &str,
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
    for (relation, relation_index, target) in
        parse_typed_final_relation_graph(owner_post_image, metadata_path)?
    {
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
        let dependency = target.dependency();
        if dependency == metadata_path || !seen.insert(dependency.clone()) {
            continue;
        }
        let source_target = SourceTarget {
            source_set: source_set.to_string(),
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
        dependencies.push(TypedRelationDependency {
            handle: resolution.handle,
            path: evidence.target_path,
            bytes,
            target: dependency.clone(),
        });
    }
    Ok(dependencies)
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
    Ok(graph)
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
        // Capture before applying, but let the writer own invalid-operation
        // diagnostics. A missing remove/update target must stay target_not_found
        // rather than being replaced by an effect-projection failure.
        let before = typed_operation_effect_value(&working, operation, false);
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
        MetaEditOperation::SetProperties { values } => {
            let observed = super::info::typed_properties(properties, kind);
            let mut selected = JsonMap::new();
            for (key, _) in values.entries() {
                let property = observed
                    .iter()
                    .find(|property| &property.key == key)
                    .ok_or_else(|| {
                        typed_diagnostic(
                            MetaDiagnosticCode::ProviderUnavailable,
                            "metadata property is unavailable for semantic effect",
                            Some("values"),
                        )
                    })?;
                let key = serde_json::to_value(property.key)
                    .ok()
                    .and_then(|value| value.as_str().map(str::to_string))
                    .ok_or_else(|| {
                        typed_diagnostic(
                            MetaDiagnosticCode::ProviderUnavailable,
                            "metadata property name cannot be normalized",
                            Some("values"),
                        )
                    })?;
                selected.insert(
                    key,
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
            let relations = super::info::typed_relations(properties, &target, &mut diagnostics);
            if let Some(diagnostic) = diagnostics.into_iter().next() {
                return Err(diagnostic);
            }
            serde_json::to_value(match relation {
                MetaRelation::Owners => relations.owners,
                MetaRelation::RegisterRecords => relations.register_records,
                MetaRelation::BasedOn => relations.based_on,
                MetaRelation::InputByString => relations.input_by_string,
            })
            .map_err(|_| {
                typed_diagnostic(
                    MetaDiagnosticCode::ProviderUnavailable,
                    "metadata relations cannot be normalized",
                    Some("relation"),
                )
            })?
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
                meta_edit_child_object_name(*node).as_deref()
                    == Some(scope.tabular_section.as_str())
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
    let requested = names.collect::<BTreeSet<_>>();
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
        .filter(|element| requested.contains(element.name.as_str()))
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
            ensure_typed_collection_allowed(&object_kind, *collection)?;
            ensure_typed_scope_exists(
                xml_text,
                scope.as_ref().map(|scope| scope.tabular_section.as_str()),
            )?;
            for (index, element) in elements.iter().enumerate() {
                update_typed_element(
                    xml_text,
                    &object_kind,
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
    validate_typed_fill_value_context(object_kind, collection, scope.is_some(), element)?;
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

fn validate_typed_fill_value_context(
    object_kind: &str,
    collection: MetaCollection,
    scoped: bool,
    element: &MetaElementDefinition,
) -> Result<(), MetaDiagnostic> {
    if element.fill_value.is_some() && !typed_fill_value_is_allowed(object_kind, collection, scoped)
    {
        return Err(typed_diagnostic(
            MetaDiagnosticCode::InvalidArguments,
            "fillValue is not available for this metadata field context",
            Some("fillValue"),
        ));
    }
    if collection == MetaCollection::TabularSections {
        for (index, attribute) in element.attributes.iter().enumerate() {
            validate_typed_fill_value_context(
                object_kind,
                MetaCollection::Attributes,
                true,
                attribute,
            )
            .map_err(|mut diagnostic| {
                diagnostic.field = Some(match diagnostic.field.take() {
                    Some(field) => format!("attributes[{index}].{field}"),
                    None => format!("attributes[{index}]"),
                });
                diagnostic
            })?;
        }
    }
    Ok(())
}

fn typed_fill_value_is_allowed(
    object_kind: &str,
    collection: MetaCollection,
    scoped: bool,
) -> bool {
    match collection {
        MetaCollection::Attributes if scoped => matches!(
            object_kind,
            "Report" | "DataProcessor" | "ExternalReport" | "ExternalDataProcessor"
        ),
        MetaCollection::Attributes => !matches!(
            meta_attribute_context(object_kind),
            "processor" | "chart" | "register-other" | "tabular"
        ),
        MetaCollection::Dimensions | MetaCollection::Resources => {
            object_kind == "InformationRegister"
        }
        MetaCollection::TabularSections
        | MetaCollection::EnumValues
        | MetaCollection::Columns
        | MetaCollection::Forms
        | MetaCollection::Templates
        | MetaCollection::Commands => false,
    }
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
    object_kind: &str,
    collection: MetaCollection,
    scope: Option<&str>,
    update: &MetaElementUpdate,
) -> Result<(), MetaDiagnostic> {
    if update.fill_value.is_some()
        && !typed_fill_value_is_allowed(object_kind, collection, scope.is_some())
    {
        return Err(typed_diagnostic(
            MetaDiagnosticCode::InvalidArguments,
            "fillValue is not available for this metadata field context",
            Some("fillValue"),
        ));
    }
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

pub(super) fn parse_typed_fill_value(
    properties_text: &str,
) -> Result<Option<MetaFillValue>, MetaDiagnostic> {
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
        "xs:decimal" if typed_decimal_shape(&value).is_some() => {
            Ok(Some(MetaFillValue::Number(value)))
        }
        "xs:decimal" => Err(typed_diagnostic(
            MetaDiagnosticCode::ValidationFailed,
            "existing numeric fill value is not canonical",
            Some("fillValue"),
        )),
        "xs:boolean" => match value.as_str() {
            "true" => Ok(Some(MetaFillValue::Boolean(true))),
            "false" => Ok(Some(MetaFillValue::Boolean(false))),
            _ => Err(typed_diagnostic(
                MetaDiagnosticCode::ValidationFailed,
                "existing boolean fill value is not canonical",
                Some("fillValue"),
            )),
        },
        "xs:dateTime" if typed_xs_datetime_is_valid(&value) => {
            Ok(Some(MetaFillValue::DateTime(value)))
        }
        "xs:dateTime" => Err(typed_diagnostic(
            MetaDiagnosticCode::ValidationFailed,
            "existing date-time fill value is not canonical",
            Some("fillValue"),
        )),
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
        "" if value.is_empty() => Err(typed_diagnostic(
            MetaDiagnosticCode::ValidationFailed,
            "existing fill value has no typed value or xsi:nil marker",
            Some("fillValue"),
        )),
        _ => Err(typed_diagnostic(
            MetaDiagnosticCode::ValidationFailed,
            "existing fill value type is unsupported by typed metadata edit",
            Some("fillValue"),
        )),
    }
}

pub(super) fn parse_typed_metadata_type(
    properties_text: &str,
) -> Result<MetadataType, MetaDiagnostic> {
    const WRAPPER_START: &str = r#"<Root xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:xr="http://v8.1c.ru/8.3/xcf/readable" xmlns:v8="http://v8.1c.ru/8.1/data/core" xmlns:cfg="http://v8.1c.ru/8.1/data/enterprise/current-config">"#;
    let wrapped = format!("{WRAPPER_START}{properties_text}</Root>");
    let document = Document::parse(&wrapped).map_err(|_| {
        typed_diagnostic(
            MetaDiagnosticCode::ProviderUnavailable,
            "metadata element properties are not valid XML",
            Some("type"),
        )
    })?;
    let qualifier_text = |container: &str, name: &str| -> Option<&str> {
        document
            .descendants()
            .find(|node| node.is_element() && node.tag_name().name() == container)
            .and_then(|node| meta_info_child(node, name))
            .and_then(|node| node.text())
    };
    let qualifier_u32 = |container: &str, name: &str| -> Result<u32, MetaDiagnostic> {
        qualifier_text(container, name).map_or(Ok(0), |value| {
            value.parse().map_err(|_| {
                typed_diagnostic(
                    MetaDiagnosticCode::ValidationFailed,
                    format!("existing metadata type has malformed {container}.{name}"),
                    Some("type"),
                )
            })
        })
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
                    length: qualifier_u32("StringQualifiers", "Length")?,
                    allowed_length: match qualifier_text("StringQualifiers", "AllowedLength") {
                        Some("Fixed") => StringLengthMode::Fixed,
                        Some("Variable") | None => StringLengthMode::Variable,
                        Some(_) => {
                            return Err(typed_diagnostic(
                                MetaDiagnosticCode::ValidationFailed,
                                "existing string length mode is unsupported",
                                Some("type"),
                            ))
                        }
                    },
                },
                "xs:decimal" => MetadataTypeVariant::Number {
                    digits: qualifier_u32("NumberQualifiers", "Digits")?,
                    fraction: qualifier_u32("NumberQualifiers", "FractionDigits")?,
                    sign: match qualifier_text("NumberQualifiers", "AllowedSign") {
                        Some("Nonnegative") => NumberSign::NonNegative,
                        Some("Any") | None => NumberSign::Any,
                        Some(_) => {
                            return Err(typed_diagnostic(
                                MetaDiagnosticCode::ValidationFailed,
                                "existing number sign mode is unsupported",
                                Some("type"),
                            ))
                        }
                    },
                },
                "xs:boolean" => MetadataTypeVariant::Boolean,
                "xs:dateTime" => MetadataTypeVariant::Date {
                    fractions: match qualifier_text("DateQualifiers", "DateFractions") {
                        Some("Date") => DateFractions::Date,
                        Some("Time") => DateFractions::Time,
                        Some("DateTime") | None => DateFractions::DateTime,
                        Some(_) => {
                            return Err(typed_diagnostic(
                                MetaDiagnosticCode::ValidationFailed,
                                "existing date fractions mode is unsupported",
                                Some("type"),
                            ))
                        }
                    },
                },
                "xs:binary" => MetadataTypeVariant::BinaryData {
                    length: qualifier_u32("BinaryDataQualifiers", "Length")?,
                    allowed_length: match qualifier_text("BinaryDataQualifiers", "AllowedLength") {
                        Some("Fixed") => StringLengthMode::Fixed,
                        Some("Variable") | None => StringLengthMode::Variable,
                        Some(_) => {
                            return Err(typed_diagnostic(
                                MetaDiagnosticCode::ValidationFailed,
                                "existing binary length mode is unsupported",
                                Some("type"),
                            ))
                        }
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
            let files = file_names
                .iter()
                .map(|path| (PathBuf::from(path), Vec::new()))
                .collect::<Vec<_>>();
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
    fn typed_exact_noop_form_update_has_no_resource_mutation_or_event() {
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
        let _ = std::fs::remove_dir_all(root);
    }
}
