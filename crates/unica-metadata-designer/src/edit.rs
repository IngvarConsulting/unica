use roxmltree::Document;
use serde_json::{Map as JsonMap, Value};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use unica_address::source_target::{MetadataAddress, PLATFORM_XML_8_3_27_FORMAT_2_20};
use unica_metadata::*;
use unica_xml_format::text::{resolve_observed_line_ending, LineEnding, SourceTextSnapshot};

use super::format_contract::*;
use super::template_catalog::*;
use super::xml_model::*;
#[derive(Debug, Default)]
pub struct MetaEditCounts {
    pub added: usize,
    pub modified: usize,
    pub removed: usize,
    pub effects: Vec<MetaMutationEffect>,
}
pub fn meta_edit_lf_to_eol(text: &str, eol: LineEnding) -> String {
    text.replace('\n', eol.as_str())
}
pub fn meta_edit_line_indent(text: &str, position: usize) -> String {
    let line_start = text[..position]
        .rfind(['\r', '\n'])
        .map_or(0, |index| index + 1);
    text[line_start..position]
        .chars()
        .take_while(|character| matches!(character, '\t' | ' '))
        .collect()
}
pub fn meta_edit_object_identity(xml_text: &str) -> Result<(String, String), String> {
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
pub struct MetaEditInsertPosition {
    pub before: Option<String>,
    pub after: Option<String>,
}
impl MetaEditInsertPosition {
    pub fn is_empty(&self) -> bool {
        self.before.is_none() && self.after.is_none()
    }

    pub fn target(&self) -> Option<(&str, bool)> {
        if let Some(after) = self.after.as_deref() {
            Some((after, true))
        } else {
            self.before.as_deref().map(|before| (before, false))
        }
    }
}
pub fn emit_meta_simple_child<F>(
    lines: &mut Vec<String>,
    indent: &str,
    tag: &str,
    name: &str,
    lang: &str,
    next_uuid: &mut F,
) where
    F: FnMut() -> String,
{
    lines.push(format!("{indent}<{tag} uuid=\"{}\">", next_uuid()));
    lines.push(format!("{indent}\t<Properties>"));
    lines.push(format!(
        "{indent}\t\t<Name>{}</Name>",
        unica_xml_format::text::escape_xml(name)
    ));
    emit_meta_mltext(
        lines,
        &format!("{indent}\t\t"),
        "Synonym",
        lang,
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
            // The full property set of an owner command in the 8.3.27
            // export: a load fills the omitted ones in (#1385).
            lines.push(format!(
                "{indent}\t\t<Group>FormNavigationPanelGoTo</Group>"
            ));
            lines.push(format!("{indent}\t\t<CommandParameterType/>"));
            lines.push(format!(
                "{indent}\t\t<ParameterUseMode>Single</ParameterUseMode>"
            ));
            lines.push(format!("{indent}\t\t<ModifiesData>false</ModifiesData>"));
            lines.push(format!("{indent}\t\t<Representation>Auto</Representation>"));
            lines.push(format!("{indent}\t\t<ToolTip/>"));
            lines.push(format!("{indent}\t\t<Picture/>"));
            lines.push(format!("{indent}\t\t<Shortcut/>"));
            lines.push(format!(
                "{indent}\t\t<OnMainServerUnavalableBehavior>Auto</OnMainServerUnavalableBehavior>"
            ));
        }
        _ => {}
    }
    lines.push(format!("{indent}\t</Properties>"));
    lines.push(format!("{indent}</{tag}>"));
}
pub fn meta_edit_property_child_indent(properties: &str) -> String {
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
pub fn meta_edit_replace_or_insert_property(
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
/// Sets the `lang` value of the multilingual `tag` inside a `<Properties>`
/// text, keeping its other languages byte for byte; a missing property is
/// inserted with the single `lang` item.
pub fn meta_edit_set_mltext_property(
    properties: &mut String,
    tag: &str,
    child_indent: &str,
    lang: &str,
    value: &str,
) -> Result<(), String> {
    if let Some(range) = meta_edit_xml_element_range(properties, tag)? {
        let replacement =
            meta_mltext_with_language_value(&properties[range.clone()], child_indent, lang, value)?;
        properties.replace_range(range, &replacement);
        return Ok(());
    }
    let mut lines = Vec::new();
    emit_meta_mltext(&mut lines, child_indent, tag, lang, value);
    meta_edit_replace_or_insert_property(properties, tag, &lines.join("\n"), child_indent)
}
pub fn meta_edit_xml_element_range(
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
pub fn meta_edit_object_node<'a, 'input>(
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
pub fn meta_edit_child_object_name(node: roxmltree::Node<'_, '_>) -> Option<String> {
    unica_xml_format::read::child_element(node, "Properties")
        .and_then(|props| unica_xml_format::read::child_direct_text(props, "Name"))
}
pub fn meta_edit_ensure_top_child_name_free(
    xml_text: &str,
    tag: &str,
    name: &str,
) -> Result<(), String> {
    let doc = Document::parse(xml_text.trim_start_matches('\u{feff}'))
        .map_err(|err| format!("XML parse error: {err}"))?;
    let object = meta_edit_object_node(&doc)?;
    if let Some(child_objects) = unica_xml_format::read::child_element(object, "ChildObjects") {
        for child in unica_xml_format::read::child_elements(child_objects, tag) {
            // Forms and templates may be registered by name only (#1325).
            if typed_physical_owner_child_name(child)
                .is_some_and(|candidate| metadata_name_eq(&candidate, name))
            {
                return Err(format!("{tag} '{name}' already exists"));
            }
        }
    }
    Ok(())
}
pub fn meta_edit_find_tabular_section<'a, 'input>(
    object: roxmltree::Node<'a, 'input>,
    section_name: &str,
) -> Option<roxmltree::Node<'a, 'input>> {
    let child_objects = unica_xml_format::read::child_element(object, "ChildObjects")?;
    unica_xml_format::read::child_elements(child_objects, "TabularSection")
        .into_iter()
        .find(|section| {
            meta_edit_child_object_name(*section)
                .is_some_and(|candidate| metadata_name_eq(&candidate, section_name))
        })
}
pub fn meta_edit_ensure_tabular_child_name_free(
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
    if let Some(child_objects) = unica_xml_format::read::child_element(section, "ChildObjects") {
        for child in unica_xml_format::read::child_elements(child_objects, tag) {
            if meta_edit_child_object_name(child)
                .is_some_and(|candidate| metadata_name_eq(&candidate, name))
            {
                return Err(format!("{tag} '{section_name}.{name}' already exists"));
            }
        }
    }
    Ok(())
}
pub fn inserted_tag(lines: &[String]) -> Option<String> {
    let first = lines.first()?.trim_start();
    let rest = first.strip_prefix('<')?;
    let end = rest.find([' ', '>', '/'])?;
    Some(rest[..end].to_string())
}
pub fn meta_edit_insert_top_child_object(
    xml_text: &mut String,
    lines: &[String],
) -> Result<(), String> {
    let doc = Document::parse(xml_text.trim_start_matches('\u{feff}'))
        .map_err(|err| format!("XML parse error: {err}"))?;
    let object = meta_edit_object_node(&doc)?;
    if let Some(child_objects) = unica_xml_format::read::child_element(object, "ChildObjects") {
        // A new entry lands in its kind's platform slot: before the first
        // entry of a kind the export writes later, else after all of them.
        let anchor = inserted_tag(lines).and_then(|tag| {
            unica_source_designer::registration::platform_child_object_successor(
                object.tag_name().name(),
                child_objects,
                &tag,
            )
            .map(|child| child.range())
        });
        if let Some(anchor_range) = anchor {
            drop(doc);
            meta_edit_insert_lines_near_node(xml_text, anchor_range, false, lines);
            return Ok(());
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
pub fn meta_edit_insert_top_child_object_with_position(
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
    let child_objects = unica_xml_format::read::child_element(object, "ChildObjects")
        .ok_or_else(|| "ChildObjects not found for positional insert".to_string())?;
    let (target_name, after) = position
        .target()
        .ok_or_else(|| "Position target must be non-empty".to_string())?;
    // A form or template anchor may be registered by name only (#1325).
    let target = unica_xml_format::read::child_elements(child_objects, tag)
        .into_iter()
        .find(|child| {
            typed_physical_owner_child_name(*child)
                .is_some_and(|candidate| metadata_name_eq(&candidate, target_name))
        })
        .ok_or_else(|| format!("{tag} '{target_name}' not found for positional insert"))?;
    let range = target.range();
    drop(doc);
    meta_edit_insert_lines_near_node(xml_text, range, after, lines);
    Ok(())
}
pub fn meta_edit_insert_tabular_child_object(
    xml_text: &mut String,
    section_name: &str,
    lines: &[String],
) -> Result<(), String> {
    let doc = Document::parse(xml_text.trim_start_matches('\u{feff}'))
        .map_err(|err| format!("XML parse error: {err}"))?;
    let object = meta_edit_object_node(&doc)?;
    let section = meta_edit_find_tabular_section(object, section_name)
        .ok_or_else(|| format!("TabularSection '{section_name}' not found"))?;
    // Как и у владельца, новый член без позиции встаёт после существующих:
    // платформа выгружает реквизиты части в порядке добавления.
    if let Some(child_objects) = unica_xml_format::read::child_element(section, "ChildObjects") {
        let range = child_objects.range();
        drop(doc);
        return meta_edit_insert_lines_into_child_objects(xml_text, range, "\t\t\t\t", lines);
    }
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
pub fn meta_edit_insert_tabular_child_object_with_position(
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
    let child_objects = unica_xml_format::read::child_element(section, "ChildObjects")
        .ok_or_else(|| format!("TabularSection '{section_name}' has no ChildObjects"))?;
    let (target_name, after) = position
        .target()
        .ok_or_else(|| "Position target must be non-empty".to_string())?;
    let target = unica_xml_format::read::child_elements(child_objects, tag)
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
pub fn meta_edit_insert_lines_into_child_objects(
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
    // A self-closing `<ChildObjects/>` whose parsed range stops short of its
    // terminator (fresh template descriptors): expand the tag in place.
    if !section_text.contains("</ChildObjects>") {
        if let Some(relative_end) = xml_text[range.start..].find("/>") {
            let end = range.start + relative_end + 2;
            if !xml_text[range.start + 1..end].contains('<') {
                xml_text.replace_range(
                    range.start..end,
                    &format!("<ChildObjects>\n{content}\n{close_indent}</ChildObjects>"),
                );
                return Ok(());
            }
        }
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
pub fn meta_edit_insert_lines_near_node(
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
pub fn meta_edit_insert_child_objects_into_node(
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
pub fn meta_edit_insert_lines_into_node_child_objects(
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
pub fn meta_edit_remove_xml_node_range(xml_text: &mut String, range: std::ops::Range<usize>) {
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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TypedChildDescriptorStorage {
    Standalone,
    InlineOwner,
}
#[derive(Debug, Clone, Copy)]
pub struct TypedChildStorageProfile {
    pub directory: &'static str,
    pub resource: MetaPublicationResource,
    pub descriptor: TypedChildDescriptorStorage,
}
const TYPED_COMMAND_STORAGE_PROFILE: TypedChildStorageProfile = TypedChildStorageProfile {
    directory: "Commands",
    resource: MetaPublicationResource::Command,
    descriptor: TypedChildDescriptorStorage::InlineOwner,
};
/// Whether applying `operations` writes a multilingual value: a synonym set
/// through properties or an element update, or any new element, whose
/// template always carries a synonym.
pub fn typed_operations_write_multilingual_text(operations: &[MetaEditOperation]) -> bool {
    operations.iter().any(|operation| match operation {
        MetaEditOperation::SetProperties { values } => values
            .entries()
            .iter()
            .any(|(key, _)| *key == MetaPropertyKey::Synonym),
        MetaEditOperation::Add { .. } => true,
        MetaEditOperation::Update { elements, .. } => {
            elements.iter().any(|element| element.synonym.is_some())
        }
        // A copy carries the source's own translations, never new text.
        MetaEditOperation::Remove { .. }
        | MetaEditOperation::Copy { .. }
        | MetaEditOperation::EditRelations { .. }
        | MetaEditOperation::AddHelp { .. }
        | MetaEditOperation::AddPredefinedItems { .. }
        | MetaEditOperation::UpdatePredefinedItems { .. }
        | MetaEditOperation::RemovePredefinedItems { .. } => false,
    })
}
pub fn typed_physical_owner_child_name(node: roxmltree::Node<'_, '_>) -> Option<String> {
    meta_edit_child_object_name(node).or_else(|| {
        (!node.children().any(|child| child.is_element()))
            .then(|| unica_xml_format::read::direct_text(node).trim().to_string())
            .filter(|name| !name.is_empty())
    })
}
pub fn typed_command_module_path(descriptor_path: &Path, name: &str) -> PathBuf {
    descriptor_path
        .with_extension("")
        .join(TYPED_COMMAND_STORAGE_PROFILE.directory)
        .join(name)
        .join(TYPED_COMMAND_MODULE_PATH)
}
pub fn canonical_typed_command_name(
    xml_text: &str,
    name: &str,
) -> Result<String, unica_metadata::request::MetaFailure> {
    find_typed_element_identity(xml_text, "Command", None, name)
        .map(|(_, name)| name)
        .map_err(unica_metadata::request::MetaFailure::from)
}
pub const TYPED_COMMAND_MODULE_PATH: &str = "Ext/CommandModule.bsl";
pub fn apply_typed_operations_with_uuid<F>(
    xml_text: &mut String,
    operations: &[MetaEditOperation],
    lang: &str,
    next_uuid: &mut F,
) -> Result<MetaEditCounts, unica_metadata::request::MetaFailure>
where
    F: FnMut() -> String,
{
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
        if let Err(diagnostic) =
            apply_typed_operation(&mut working, operation, &mut counts, lang, next_uuid)
        {
            let mut diagnostic = diagnostic.with_operation_index(operation_index);
            if let Some(field) = diagnostic.field.as_mut() {
                if !field.starts_with("operations[") {
                    *field = format!("operations[{operation_index}].{field}");
                }
            }
            return Err(diagnostic.into());
        }
        Document::parse(working.trim_start_matches('\u{feff}')).map_err(|_| {
            unica_metadata::request::MetaFailure::from(
                typed_diagnostic(
                    MetaDiagnosticCode::ValidationFailed,
                    "typed metadata operation produced invalid XML",
                    None,
                )
                .with_operation_index(operation_index),
            )
        })?;
        validate_metadata_8_3_27_boolean_contract(&working, "meta.edit").map_err(|_| {
            unica_metadata::request::MetaFailure::from(
                typed_diagnostic(
                    MetaDiagnosticCode::ValidationFailed,
                    "typed metadata operation violates the boolean format contract",
                    None,
                )
                .with_operation_index(operation_index),
            )
        })?;
        validate_metadata_8_3_27_enum_contract(&working, "meta.edit").map_err(|_| {
            unica_metadata::request::MetaFailure::from(
                typed_diagnostic(
                    MetaDiagnosticCode::ValidationFailed,
                    "typed metadata operation violates the enum format contract",
                    None,
                )
                .with_operation_index(operation_index),
            )
        })?;
        let before = before.map_err(|diagnostic| {
            unica_metadata::request::MetaFailure::from(
                diagnostic.with_operation_index(operation_index),
            )
        })?;
        let after =
            typed_operation_effect_value(&working, operation, true).map_err(|diagnostic| {
                unica_metadata::request::MetaFailure::from(
                    diagnostic.with_operation_index(operation_index),
                )
            })?;
        counts.effects.push(MetaMutationEffect {
            operation_index: Some(operation_index as u64),
            operation: typed_operation_name(operation).to_string(),
            target: typed_operation_target(&working, operation)?,
            before,
            after,
        });
    }
    validate_typed_event_source_final_state(&working, operations)?;
    *xml_text = working;
    Ok(counts)
}
pub fn validate_typed_event_source_final_state(
    xml: &str,
    operations: &[MetaEditOperation],
) -> Result<(), unica_metadata::request::MetaFailure> {
    if !operations.iter().any(|operation| {
        matches!(
            operation,
            MetaEditOperation::EditRelations {
                relation: MetaRelation::Source,
                ..
            }
        )
    }) {
        return Ok(());
    }
    let document = Document::parse(xml.trim_start_matches('\u{feff}')).map_err(|_| {
        unica_metadata::request::MetaFailure::from(typed_diagnostic(
            MetaDiagnosticCode::ProviderUnavailable,
            "EventSubscription post-image is not valid XML",
            Some("relations.source"),
        ))
    })?;
    let source = meta_event_subscription_source_node(&document).map_err(|message| {
        unica_metadata::request::MetaFailure::from(typed_diagnostic(
            MetaDiagnosticCode::ProviderUnavailable,
            message,
            Some("relations.source"),
        ))
    })?;
    let observed = parse_meta_event_subscription_source(source).map_err(|message| {
        unica_metadata::request::MetaFailure::from(typed_diagnostic(
            MetaDiagnosticCode::InvalidArguments,
            message,
            Some("relations.source"),
        ))
    })?;
    if observed.sources.is_empty() {
        return Err(unica_metadata::request::MetaFailure::from(
            typed_diagnostic(
                MetaDiagnosticCode::InvalidArguments,
                "EventSubscription Source must retain at least one logical event source",
                Some("relations.source"),
            ),
        ));
    }
    Ok(())
}
pub fn typed_operation_name(operation: &MetaEditOperation) -> &'static str {
    match operation {
        MetaEditOperation::SetProperties { .. } => "setProperties",
        MetaEditOperation::Add { .. } => "add",
        MetaEditOperation::Update { .. } => "update",
        MetaEditOperation::Remove { .. } => "remove",
        MetaEditOperation::Copy { .. } => "copy",
        MetaEditOperation::EditRelations { .. } => "editRelations",
        MetaEditOperation::AddHelp { .. } => "addHelp",
        MetaEditOperation::AddPredefinedItems { .. } => "add",
        MetaEditOperation::UpdatePredefinedItems { .. } => "update",
        MetaEditOperation::RemovePredefinedItems { .. } => "remove",
    }
}
pub fn typed_operation_target(
    xml: &str,
    operation: &MetaEditOperation,
) -> Result<String, unica_metadata::request::MetaFailure> {
    let (kind, name) = meta_edit_object_identity(xml).map_err(|_| {
        unica_metadata::request::MetaFailure::from(typed_diagnostic(
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
        }
        | MetaEditOperation::Copy {
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
pub fn typed_operation_effect_value(
    xml: &str,
    operation: &MetaEditOperation,
    after: bool,
) -> Result<Option<Value>, MetaDiagnostic> {
    match operation {
        MetaEditOperation::Add { .. } | MetaEditOperation::Copy { .. } if !after => {
            return Ok(None)
        }
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
    let kind = unica_metadata::MetadataKind::parse(&kind_name).map_err(|_| {
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
    let properties = unica_xml_format::read::child_element(object, "Properties");
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
                if *key == MetaPropertyKey::Type {
                    let properties = properties.ok_or_else(|| {
                        typed_diagnostic(
                            MetaDiagnosticCode::ProviderUnavailable,
                            "metadata Type has no Properties",
                            Some("values.Type"),
                        )
                    })?;
                    let observed_type =
                        super::info_projection::parse_observed_metadata_type_node(properties)?;
                    selected.insert(
                        public_key,
                        serde_json::to_value(observed_type).map_err(|_| {
                            typed_diagnostic(
                                MetaDiagnosticCode::ProviderUnavailable,
                                "metadata Type cannot be normalized",
                                Some("values.Type"),
                            )
                        })?,
                    );
                    continue;
                }
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
        } => {
            let value = typed_collection_effect_value(
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
            )?;
            if elements
                .iter()
                .any(|element| element.base_dimension.is_some())
            {
                crate::dimension_base::effect(
                    xml,
                    object,
                    value,
                    &elements
                        .iter()
                        .filter(|element| element.base_dimension.is_some())
                        .map(|element| {
                            if after {
                                element.new_name.as_deref().unwrap_or(&element.name)
                            } else {
                                &element.name
                            }
                        })
                        .collect::<Vec<_>>(),
                )?
            } else {
                value
            }
        }
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
        MetaEditOperation::Copy {
            collection,
            scope,
            new_name,
            ..
        } => typed_collection_effect_value(
            xml,
            object,
            *collection,
            scope.as_ref(),
            std::iter::once(new_name.as_str()),
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
pub fn typed_collection_effect_value<'a>(
    xml: &str,
    object: roxmltree::Node<'a, 'a>,
    collection: MetaCollection,
    scope: Option<&unica_metadata::MetaScope>,
    names: impl Iterator<Item = &'a str>,
    target: &MetadataAddress,
) -> Result<Value, MetaDiagnostic> {
    let top = unica_xml_format::read::child_element(object, "ChildObjects");
    let parent = if let Some(scope) = scope {
        let section = top
            .into_iter()
            .flat_map(|node| unica_xml_format::read::child_elements(node, "TabularSection"))
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
        unica_xml_format::read::child_element(section, "ChildObjects")
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
    if let Some(diagnostic) = diagnostics
        .into_iter()
        .find(|diagnostic| diagnostic.severity == unica_metadata::MetaDiagnosticSeverity::Error)
    {
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
pub fn apply_typed_operation<F>(
    xml_text: &mut String,
    operation: &MetaEditOperation,
    counts: &mut MetaEditCounts,
    lang: &str,
    next_uuid: &mut F,
) -> Result<(), MetaDiagnostic>
where
    F: FnMut() -> String,
{
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
            apply_typed_properties(xml_text, values, lang)?;
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
                    lang,
                    next_uuid,
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
                    lang,
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
        MetaEditOperation::Copy {
            collection,
            scope,
            name,
            new_name,
        } => {
            let section = scope.as_ref().map(|scope| scope.tabular_section.as_str());
            ensure_typed_scope_exists(xml_text, section)?;
            copy_typed_element(
                xml_text,
                &object_kind,
                &object_name,
                *collection,
                section,
                name,
                new_name,
                next_uuid,
            )?;
            counts.added += 1;
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
pub fn qualify_element_diagnostic(mut diagnostic: MetaDiagnostic, index: usize) -> MetaDiagnostic {
    diagnostic.field = Some(match diagnostic.field.take() {
        Some(field) if field.starts_with("scope") || field.starts_with("collection") => field,
        Some(field) => format!("elements[{index}].{field}"),
        None => format!("elements[{index}]"),
    });
    diagnostic
}
pub fn ensure_typed_scope_exists(
    xml_text: &str,
    scope: Option<&str>,
) -> Result<(), MetaDiagnostic> {
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
pub fn typed_diagnostic(
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
pub fn apply_typed_properties(
    xml_text: &mut String,
    changes: &unica_metadata::MetaPropertyChanges,
    lang: &str,
) -> Result<(), MetaDiagnostic> {
    let parsed_body = xml_text.trim_start_matches('\u{feff}');
    let prefix_len = xml_text.len() - parsed_body.len();
    let doc = Document::parse(parsed_body).map_err(|_| {
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
    let properties =
        unica_xml_format::read::child_element(object, "Properties").ok_or_else(|| {
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
    let previous_type = if changes
        .entries()
        .iter()
        .any(|(key, _)| *key == MetaPropertyKey::Type)
    {
        Some(super::info_projection::parse_observed_metadata_type_node(
            properties,
        )?)
    } else {
        None
    };
    let range = properties.range();
    let mut text = parsed_body[range.clone()].to_string();
    let indent = meta_edit_property_child_indent(&text);
    let mut replacements = Vec::new();
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
        let property = unica_xml_format::read::child_element(properties, tag).ok_or_else(|| {
            typed_diagnostic(
                MetaDiagnosticCode::InvalidArguments,
                format!("property `{tag}` is not present on this metadata object"),
                Some("values"),
            )
        })?;
        let replacement = match (key, value) {
            (MetaPropertyKey::Type, MetaPropertyValue::Type(metadata_type)) => {
                if previous_type
                    .as_ref()
                    .and_then(|observed| MetadataType::try_from(observed.clone()).ok())
                    .as_ref()
                    == Some(metadata_type)
                {
                    continue;
                }
                let mut lines = Vec::new();
                emit_meta_typed_value_type(&mut lines, &indent, metadata_type);
                // The replacement owns these QName bindings even when the
                // original descriptor uses aliases or shadows canonical names.
                lines[0] = format!(
                    r#"{indent}<Type xmlns="http://v8.1c.ru/8.3/MDClasses" xmlns:v8="http://v8.1c.ru/8.1/data/core" xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:cfg="http://v8.1c.ru/8.1/data/enterprise/current-config">"#
                );
                lines.join("\n")
            }
            (_, MetaPropertyValue::Type(_)) => {
                return Err(typed_diagnostic(
                    MetaDiagnosticCode::InvalidArguments,
                    "structured Type is only a Constant Type property",
                    Some("values.Type"),
                ))
            }
            (MetaPropertyKey::Synonym, MetaPropertyValue::String(value)) => {
                unica_xml_format::localized::replace_language_value(
                    parsed_body,
                    property,
                    &indent,
                    lang,
                    value,
                )
            }
            (MetaPropertyKey::Comment, MetaPropertyValue::String(value)) if value.is_empty() => {
                format!("{indent}<{tag}/>")
            }
            (_, MetaPropertyValue::String(value)) => {
                format!(
                    "{indent}<{tag}>{}</{tag}>",
                    unica_xml_format::text::escape_xml(value)
                )
            }
            (_, MetaPropertyValue::Boolean(value)) => {
                format!("{indent}<{tag}>{value}</{tag}>")
            }
            (_, MetaPropertyValue::UnsignedInteger(value)) => {
                format!("{indent}<{tag}>{value}</{tag}>")
            }
        };
        let property_range = property.range();
        replacements.push((
            property_range.start - range.start..property_range.end - range.start,
            replacement.trim_start().to_string(),
        ));
    }
    // XML node ranges include nested elements with the same local name.
    // Apply from the end so earlier replacements keep their original offsets.
    replacements.sort_by_key(|(range, _)| std::cmp::Reverse(range.start));
    for (property_range, replacement) in replacements {
        text.replace_range(property_range, &replacement);
    }
    drop(doc);
    xml_text.replace_range(range.start + prefix_len..range.end + prefix_len, &text);
    Ok(())
}
pub fn collection_tag(collection: MetaCollection) -> &'static str {
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
#[allow(clippy::too_many_arguments)]
pub fn add_typed_element<F>(
    xml_text: &mut String,
    object_kind: &str,
    object_name: &str,
    collection: MetaCollection,
    scope: Option<&str>,
    element: &MetaElementDefinition,
    lang: &str,
    next_uuid: &mut F,
) -> Result<(), MetaDiagnostic>
where
    F: FnMut() -> String,
{
    if !unica_metadata::identifier::is_1c_identifier(&element.name) {
        return Err(typed_diagnostic(
            MetaDiagnosticCode::InvalidArguments,
            "metadata element name is invalid",
            Some("name"),
        ));
    }
    let tag = collection_tag(collection);
    ensure_typed_name_free(xml_text, tag, scope, &element.name)?;
    let position = typed_insert_position(element.position.as_ref());
    let mut lines = render_typed_element(
        object_kind,
        object_name,
        collection,
        scope.is_some(),
        element,
        lang,
        next_uuid,
    )?;
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
pub fn typed_insert_position(position: Option<&MetaPosition>) -> MetaEditInsertPosition {
    position.map_or_else(MetaEditInsertPosition::default, |position| {
        MetaEditInsertPosition {
            before: position.before.clone(),
            after: position.after.clone(),
        }
    })
}
pub fn ensure_typed_name_free(
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
pub fn inherit_register_field_settings(xml: &str, tag: &str, rendered: &mut [String]) {
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
pub fn agreed_sibling_setting(xml: &str, tag: &str, property: &str) -> Option<String> {
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
pub fn element_text_between(haystack: &str, open: &str, close: &str) -> Option<String> {
    let start = haystack.find(open)? + open.len();
    let end = haystack[start..].find(close)? + start;
    Some(haystack[start..end].to_string())
}
pub fn render_typed_element<F>(
    object_kind: &str,
    object_name: &str,
    collection: MetaCollection,
    in_tabular_section: bool,
    element: &MetaElementDefinition,
    lang: &str,
    next_uuid: &mut F,
) -> Result<Vec<String>, MetaDiagnostic>
where
    F: FnMut() -> String,
{
    let mut lines = Vec::new();
    let attr = || MetadataAttributeTemplate {
        name: element.name.clone(),
        synonym: element
            .synonym
            .clone()
            .unwrap_or_else(|| split_meta_camel_case(&element.name)),
        required: element.required.unwrap_or(false),
    };
    match collection {
        // A tabular-section column sits two levels deeper
        // (`TabularSection/ChildObjects`) and has its own platform profile.
        MetaCollection::Attributes if in_tabular_section => emit_meta_attribute(
            &mut lines,
            "\t\t\t\t\t",
            &attr(),
            meta_tabular_attribute_context(object_kind),
            lang,
            next_uuid,
        ),
        MetaCollection::Attributes => emit_meta_attribute(
            &mut lines,
            "\t\t\t",
            &attr(),
            meta_attribute_context(object_kind),
            lang,
            next_uuid,
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
                lang,
                next_uuid,
            );
            let mut rendered = lines.join("\n");
            apply_typed_element_fields(&mut rendered, element, lang);
            apply_typed_nested_attribute_fields(&mut rendered, &ordered_attributes, lang)?;
            return Ok(rendered.lines().map(ToOwned::to_owned).collect());
        }
        MetaCollection::Dimensions | MetaCollection::Resources => emit_meta_register_field(
            &mut lines,
            "\t\t\t",
            collection_tag(collection),
            &attr(),
            object_kind,
            lang,
            next_uuid,
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
            lang,
            next_uuid,
        ),
        MetaCollection::Columns
        | MetaCollection::Forms
        | MetaCollection::Templates
        | MetaCollection::Commands => emit_meta_simple_child(
            &mut lines,
            "\t\t\t",
            collection_tag(collection),
            &element.name,
            lang,
            next_uuid,
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
    apply_typed_element_fields(&mut rendered, element, lang);
    Ok(rendered.lines().map(ToOwned::to_owned).collect())
}
pub fn order_typed_nested_attributes(
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
pub fn apply_typed_nested_attribute_fields(
    xml: &mut String,
    attributes: &[MetaElementDefinition],
    lang: &str,
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
        apply_typed_element_fields(&mut attribute_xml, attribute, lang);
        xml.replace_range(range, &attribute_xml);
    }
    Ok(())
}
pub fn apply_typed_element_fields(xml: &mut String, element: &MetaElementDefinition, lang: &str) {
    let Some(range) = typed_properties_text_range(xml) else {
        return;
    };
    let mut text = xml[range.clone()].to_string();
    let indent = meta_edit_property_child_indent(&text);
    if let Some(synonym) = &element.synonym {
        let _ = meta_edit_set_mltext_property(&mut text, "Synonym", &indent, lang, synonym);
    }
    if let Some(comment) = &element.comment {
        let replacement = if comment.is_empty() {
            format!("{indent}<Comment/>")
        } else {
            format!(
                "{indent}<Comment>{}</Comment>",
                unica_xml_format::text::escape_xml(comment)
            )
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
    if let Some(indexing) = &element.indexing {
        let replacement = format!(
            "{indent}<Indexing>{}</Indexing>",
            unica_xml_format::text::escape_xml(indexing)
        );
        let _ = meta_edit_replace_or_insert_property(&mut text, "Indexing", &replacement, &indent);
    }
    xml.replace_range(range, &text);
}
pub fn typed_properties_text_range(text: &str) -> Option<std::ops::Range<usize>> {
    let start = text.find("<Properties>")?;
    let close = "</Properties>";
    let relative_end = text[start..].find(close)?;
    Some(start..start + relative_end + close.len())
}
pub fn find_typed_element_range(
    xml_text: &str,
    tag: &str,
    scope: Option<&str>,
    name: &str,
) -> Result<std::ops::Range<usize>, MetaDiagnostic> {
    find_typed_element_identity(xml_text, tag, scope, name).map(|(range, _)| range)
}
pub fn find_typed_element_identity(
    xml_text: &str,
    tag: &str,
    scope: Option<&str>,
    name: &str,
) -> Result<(std::ops::Range<usize>, String), MetaDiagnostic> {
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
        unica_xml_format::read::child_element(section, "ChildObjects")
    } else {
        unica_xml_format::read::child_element(object, "ChildObjects")
    }
    .ok_or_else(|| {
        typed_diagnostic(
            MetaDiagnosticCode::TargetNotFound,
            "metadata collection is empty",
            Some("name"),
        )
    })?;
    unica_xml_format::read::child_elements(parent, tag)
        .into_iter()
        .find_map(|node| {
            let canonical_name = meta_edit_child_object_name(node)?;
            metadata_name_eq(&canonical_name, name).then(|| (node.range(), canonical_name))
        })
        .ok_or_else(|| {
            typed_diagnostic(
                MetaDiagnosticCode::TargetNotFound,
                format!("element `{name}` was not found"),
                Some("name"),
            )
        })
}
pub fn update_typed_element(
    xml_text: &mut String,
    collection: MetaCollection,
    scope: Option<&str>,
    update: &MetaElementUpdate,
    lang: &str,
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
    if let Some(value) = update.base_dimension {
        if let Some((edit, token)) = crate::dimension_base::edit(xml_text, range.clone(), value)? {
            let start = range.start + properties_range.start;
            properties_text.replace_range(edit.start - start..edit.end - start, &token);
        }
    }

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
        let replacement = format!(
            "{indent}<Name>{}</Name>",
            unica_xml_format::text::escape_xml(new_name)
        );
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
        meta_edit_set_mltext_property(&mut properties_text, "Synonym", &indent, lang, synonym)
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
            format!(
                "{indent}<Comment>{}</Comment>",
                unica_xml_format::text::escape_xml(comment)
            )
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
    if let Some(indexing) = &update.indexing {
        let replacement = format!(
            "{indent}<Indexing>{}</Indexing>",
            unica_xml_format::text::escape_xml(indexing)
        );
        meta_edit_replace_or_insert_property(
            &mut properties_text,
            "Indexing",
            &replacement,
            &indent,
        )
        .map_err(|_| {
            typed_diagnostic(
                MetaDiagnosticCode::ProviderUnavailable,
                "metadata indexing could not be updated",
                Some("indexing"),
            )
        })?;
    }
    if let Some(deny) = update.deny_incomplete_values {
        // 8.3.27 writes the flag on every dimension of these registers as
        // `<DenyIncompleteValues>true|false</…>`; any other shape or value
        // has no proven meaning to keep, so it is refused, not overwritten.
        let refused = |message: &str| {
            typed_diagnostic(
                MetaDiagnosticCode::InvalidArguments,
                message,
                Some("denyIncompleteValues"),
            )
        };
        let open = "<DenyIncompleteValues>";
        let close = "</DenyIncompleteValues>";
        let start = match properties_text.find(open) {
            Some(start) => start + open.len(),
            None if properties_text.contains("<DenyIncompleteValues") => {
                return Err(refused(
                    "the dimension's DenyIncompleteValues has a form other than an element with a boolean value",
                ))
            }
            None => return Err(refused("the dimension has no DenyIncompleteValues to change")),
        };
        let end = start
            + properties_text[start..]
                .find(close)
                .ok_or_else(|| refused("the dimension's DenyIncompleteValues is not closed"))?;
        let current = properties_text[start..end].trim();
        if !matches!(current, "true" | "false") {
            return Err(refused(
                "the dimension's DenyIncompleteValues holds a value other than true or false",
            ));
        }
        let value = if deny { "true" } else { "false" };
        if current != value {
            properties_text.replace_range(start..end, value);
        }
    }
    let mut updated_node = node_xml;
    updated_node.replace_range(properties_range, &properties_text);
    xml_text.replace_range(range, &updated_node);
    if let Some(new_name) = update
        .new_name
        .as_deref()
        .filter(|name| *name != update.name)
    {
        if collection == MetaCollection::TabularSections {
            rename_tabular_section_generated_types(xml_text, new_name)?;
        }
    }
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
/// A tabular section's generated types carry its name
/// (`CatalogTabularSection.<Owner>.<Name>`, `CatalogTabularSectionRow.<Owner>.<Name>`).
/// The platform renames them with the section, so a rename that kept the old
/// names would be rewritten by the first load and export (#1385). Their
/// `TypeId`/`ValueId` stay: the types are the same, only their names change.
pub fn rename_tabular_section_generated_types(
    xml_text: &mut String,
    section_name: &str,
) -> Result<(), MetaDiagnostic> {
    let body = xml_text.trim_start_matches('\u{feff}');
    let shift = xml_text.len() - body.len();
    let unavailable = || {
        typed_diagnostic(
            MetaDiagnosticCode::ProviderUnavailable,
            "tabular section generated types could not be renamed",
            Some("newName"),
        )
    };
    let doc = Document::parse(body).map_err(|_| unavailable())?;
    let object = meta_edit_object_node(&doc).map_err(|_| unavailable())?;
    let section = meta_edit_find_tabular_section(object, section_name).ok_or_else(unavailable)?;
    let mut edits = Vec::new();
    if let Some(internal_info) = unica_xml_format::read::child_element(section, "InternalInfo") {
        for generated in internal_info
            .children()
            .filter(|node| node.is_element() && node.tag_name().name() == "GeneratedType")
        {
            if !matches!(
                generated.attribute("category"),
                Some("TabularSection" | "TabularSectionRow")
            ) {
                continue;
            }
            let Some(name) = generated
                .attributes()
                .find(|attribute| attribute.name() == "name" && attribute.namespace().is_none())
            else {
                continue;
            };
            let Some((prefix, _)) = name.value().rsplit_once('.') else {
                continue;
            };
            let range = name.range_value();
            edits.push((
                shift + range.start..shift + range.end,
                unica_xml_format::text::escape_xml(&format!("{prefix}.{section_name}")),
            ));
        }
    }
    drop(doc);
    for (range, value) in edits.into_iter().rev() {
        xml_text.replace_range(range, &value);
    }
    Ok(())
}
pub fn parse_typed_fill_value_node(
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
        (READABLE_NAMESPACE, "DesignTimeRef") => {
            // Platform dumps encode an unset reference as a typed empty node.
            // Structural and namespace checks above still reject malformed XML.
            if value.is_empty() {
                return Ok(None);
            }
            // Real platform dumps carry item designators after the metadata
            // address — `Catalog.Валюты.EmptyRef` or a predefined item name.
            // The address prefix is validated; the designator round-trips
            // verbatim so re-emission cannot corrupt the value.
            let parts = value.split('.').collect::<Vec<_>>();
            if parts.first() == Some(&"Enum") && parts.get(2) == Some(&"EnumValue") {
                if parts.len() != 4
                    || !unica_metadata::identifier::is_1c_identifier(parts[1])
                    || !unica_metadata::identifier::is_1c_identifier(parts[3])
                {
                    return Err(typed_diagnostic(
                        MetaDiagnosticCode::ValidationFailed,
                        "existing enum fill value is not a canonical item address",
                        Some("fillValue"),
                    ));
                }
                let metadata_path = MetadataAddress::parse(
                    PLATFORM_XML_8_3_27_FORMAT_2_20,
                    &format!("Enum.{}", parts[1]),
                )
                .map_err(|error| {
                    typed_diagnostic(
                        MetaDiagnosticCode::ValidationFailed,
                        error.to_string(),
                        Some("fillValue"),
                    )
                })?;
                return Ok(Some(MetaFillValue::DesignTimeItemRef {
                    reference: unica_metadata::MetadataReference { metadata_path },
                    item: format!("EnumValue.{}", parts[3]),
                }));
            }
            if let Ok(metadata_path) =
                MetadataAddress::parse(PLATFORM_XML_8_3_27_FORMAT_2_20, &value)
            {
                return Ok(Some(MetaFillValue::Reference(
                    unica_metadata::MetadataReference { metadata_path },
                )));
            }
            let Some((prefix, item)) = value.rsplit_once('.') else {
                return Err(typed_diagnostic(
                    MetaDiagnosticCode::ValidationFailed,
                    "existing reference fill value is not a metadata address",
                    Some("fillValue"),
                ));
            };
            let metadata_path = MetadataAddress::parse(PLATFORM_XML_8_3_27_FORMAT_2_20, prefix)
                .map_err(|_| {
                    typed_diagnostic(
                        MetaDiagnosticCode::ValidationFailed,
                        "existing reference fill value is not a metadata address",
                        Some("fillValue"),
                    )
                })?;
            if item.is_empty() {
                return Err(typed_diagnostic(
                    MetaDiagnosticCode::ValidationFailed,
                    "existing reference fill value is not a metadata address",
                    Some("fillValue"),
                ));
            }
            Ok(Some(MetaFillValue::DesignTimeItemRef {
                reference: unica_metadata::MetadataReference { metadata_path },
                item: item.to_string(),
            }))
        }
        _ => Err(typed_diagnostic(
            MetaDiagnosticCode::ValidationFailed,
            "existing fill value type is unsupported by typed metadata edit",
            Some("fillValue"),
        )),
    }
}
pub fn parse_typed_fill_value_at_element(
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
pub fn parse_typed_metadata_type_at_element(
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
pub fn narrow_observed_metadata_type(
    observed: unica_metadata::ObservedMetadataType,
) -> Result<MetadataType, MetaDiagnostic> {
    MetadataType::try_from(observed).map_err(|mut diagnostic| {
        diagnostic.code = MetaDiagnosticCode::ValidationFailed;
        diagnostic.message =
            "existing metadata type is outside the public mutation algebra".to_string();
        diagnostic.field = Some("type".to_string());
        diagnostic
    })
}
/// Insert a copy of member `name` after the last member of its collection.
///
/// The copy keeps the source text byte for byte, so every setting and every
/// translation stays, and changes only its own `uuid` and `Name`. A source
/// whose text declares further identities or names its own UUID or path
/// cannot be copied without guessing which reference should follow.
#[allow(clippy::too_many_arguments)]
pub fn copy_typed_element<F>(
    xml_text: &mut String,
    object_kind: &str,
    object_name: &str,
    collection: MetaCollection,
    scope: Option<&str>,
    name: &str,
    new_name: &str,
    next_uuid: &mut F,
) -> Result<(), MetaDiagnostic>
where
    F: FnMut() -> String,
{
    if !unica_metadata::identifier::is_1c_identifier(new_name) {
        return Err(typed_diagnostic(
            MetaDiagnosticCode::InvalidArguments,
            "metadata element name is invalid",
            Some("values.name"),
        ));
    }
    let tag = collection_tag(collection);
    let (source_range, source_name) = find_typed_element_identity(xml_text, tag, scope, name)
        .map_err(|mut diagnostic| {
            diagnostic.field = None;
            diagnostic
        })?;
    // A taken name is the caller's to change, not a provider failure.
    ensure_typed_name_free(xml_text, tag, scope, new_name).map_err(|_| {
        typed_diagnostic(
            MetaDiagnosticCode::InvalidArguments,
            format!("`{new_name}` is already the name of a member of this collection"),
            Some("values.name"),
        )
    })?;
    let unsupported =
        |message: &str| typed_diagnostic(MetaDiagnosticCode::InvalidArguments, message, Some("at"));
    let document = Document::parse(xml_text.trim_start_matches('\u{feff}')).map_err(|_| {
        typed_diagnostic(
            MetaDiagnosticCode::ProviderUnavailable,
            "metadata descriptor is not valid XML",
            None,
        )
    })?;
    let offset = xml_text.len() - xml_text.trim_start_matches('\u{feff}').len();
    let source = document
        .descendants()
        .find(|node| node.is_element() && node.range().start == source_range.start)
        .expect("the found member is an element of the same text");
    let elements = source
        .children()
        .filter(|child| child.is_element())
        .collect::<Vec<_>>();
    let properties = match elements.as_slice() {
        [properties] if properties.tag_name().name() == "Properties" => *properties,
        _ => {
            return Err(unsupported(
                "the source member must hold exactly one Properties element",
            ))
        }
    };
    let uuid = source
        .attribute("uuid")
        .ok_or_else(|| unsupported("the source member has no uuid"))?;
    if source
        .attributes()
        .any(|attribute| attribute.name() != "uuid")
        || properties.descendants().skip(1).any(|node| {
            node.is_element()
                && node
                    .attributes()
                    .any(|attribute| matches!(attribute.name(), "uuid" | "id"))
        })
    {
        return Err(unsupported(
            "the source member declares nested identities; this copy supports only a member without them",
        ));
    }
    // References name the canonical declaration, so the path is built from
    // the names written in the descriptor, not from the caller's spelling.
    let declared_name = |node: roxmltree::Node<'_, '_>| {
        unica_xml_format::read::child_element(node, "Properties")
            .and_then(|properties| unica_xml_format::read::child_element(properties, "Name"))
            .and_then(|name| name.text())
            .map(str::trim)
            .map(str::to_string)
    };
    let section_name = match scope {
        Some(_) => Some(
            source
                .parent()
                .and_then(|children| children.parent())
                .and_then(declared_name)
                .ok_or_else(|| unsupported("the source member's tabular section has no name"))?,
        ),
        None => None,
    };
    let own_path = match &section_name {
        Some(section) => {
            format!("{object_kind}.{object_name}.TabularSection.{section}.{tag}.{source_name}")
        }
        None => format!("{object_kind}.{object_name}.{tag}.{source_name}"),
    };
    // A longer name such as `…Attribute.СвойствоДоп` is a neighbour.
    let names_itself = |value: &str| {
        value.match_indices(own_path.as_str()).any(|(at, _)| {
            !value[at + own_path.len()..]
                .chars()
                .next()
                .is_some_and(|next| next.is_alphanumeric() || next == '_')
        })
    };
    let source_range = (source_range.start + offset)..(source_range.end + offset);
    let text = &xml_text[source_range.clone()];
    let refers_to_itself = properties.descendants().any(|node| {
        node.text().is_some_and(|value| {
            let value = value.trim();
            value.eq_ignore_ascii_case(uuid) || names_itself(value)
        })
    });
    if refers_to_itself {
        return Err(unsupported(
            "the source member refers to its own name or uuid; such references cannot be copied unambiguously",
        ));
    }
    let name_node = unica_xml_format::read::child_element(properties, "Name")
        .and_then(|node| node.first_child().filter(|child| child.is_text()))
        .ok_or_else(|| unsupported("the source member has no Name"))?;
    let local = |range: std::ops::Range<usize>| {
        (range.start + offset - source_range.start)..(range.end + offset - source_range.start)
    };
    let uuid_range = local(
        source
            .attributes()
            .find(|attribute| attribute.name() == "uuid")
            .expect("checked above")
            .range_value(),
    );
    let name_range = local(name_node.range());
    let mut copy = String::with_capacity(text.len() + new_name.len());
    copy.push_str(&text[..uuid_range.start]);
    copy.push_str(&next_uuid());
    copy.push_str(&text[uuid_range.end..name_range.start]);
    copy.push_str(new_name);
    copy.push_str(&text[name_range.end..]);
    // The copy follows the last member of the collection with the layout
    // observed in front of it.
    let parent = source.parent().expect("a member has its collection");
    let last = unica_xml_format::read::child_elements(parent, tag)
        .into_iter()
        .last()
        .expect("the source is one of the members");
    let last_start = last.range().start + offset;
    let line_start = xml_text[..last_start]
        .rfind('\n')
        .map_or(0, |index| index + 1);
    let indent = &xml_text[line_start..last_start];
    if !indent
        .chars()
        .all(|character| matches!(character, ' ' | '\t'))
    {
        return Err(unsupported("the member layout is not one element per line"));
    }
    let eol = if xml_text[..line_start].ends_with("\r\n") {
        "\r\n"
    } else {
        "\n"
    };
    let insertion = last.range().end + offset;
    let fragment = format!("{eol}{indent}{copy}");
    xml_text.insert_str(insertion, &fragment);
    Ok(())
}
pub fn remove_typed_element(
    xml_text: &mut String,
    collection: MetaCollection,
    scope: Option<&str>,
    name: &str,
) -> Result<(), MetaDiagnostic> {
    let tag = collection_tag(collection);
    let range = find_typed_element_range(xml_text, tag, scope, name)?;
    if let Some(updated) = unica_source_designer::registration::empty_child_objects_after_removal(
        xml_text,
        range.clone(),
    ) {
        *xml_text = updated;
    } else {
        meta_edit_remove_xml_node_range(xml_text, range);
    }
    Ok(())
}
pub fn apply_typed_relations(
    xml_text: &mut String,
    relation: MetaRelation,
    mode: RelationEditMode,
    targets: &[unica_metadata::MetaRelationTarget],
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
    let properties =
        unica_xml_format::read::child_element(object, "Properties").ok_or_else(|| {
            typed_diagnostic(
                MetaDiagnosticCode::ProviderUnavailable,
                "metadata descriptor has no Properties",
                None,
            )
        })?;
    let relation_node =
        unica_xml_format::read::child_element(properties, tag).ok_or_else(|| {
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
                unica_metadata::MetaRelationTarget::Field(_)
            ) | (
                MetaRelation::Owners | MetaRelation::RegisterRecords | MetaRelation::BasedOn,
                unica_metadata::MetaRelationTarget::Object(_)
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
                    format!(
                        "{indent}\t<{item_tag}>{}</{item_tag}>",
                        unica_xml_format::text::escape_xml(value)
                    )
                } else {
                    format!(
                        "{indent}\t<{item_tag} xsi:type=\"xr:MDObjectRef\">{}</{item_tag}>",
                        unica_xml_format::text::escape_xml(value)
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
pub fn apply_typed_event_subscription_source(
    xml_text: &mut String,
    mode: RelationEditMode,
    targets: &[unica_metadata::MetaRelationTarget],
) -> Result<(), MetaDiagnostic> {
    let requested = targets
        .iter()
        .enumerate()
        .map(|(index, target)| match target {
            unica_metadata::MetaRelationTarget::EventSource(source) => Ok(source.clone()),
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
    if mode != RelationEditMode::Replace {
        let range = source_node.range();
        let indent = meta_edit_line_indent(source_text, range.start);
        let replacement = super::xml_model::edit_meta_event_subscription_members(
            source_text,
            source_node,
            mode,
            &requested,
            &indent,
        )
        .map_err(|message| {
            typed_diagnostic(
                MetaDiagnosticCode::InvalidArguments,
                message,
                Some("relation"),
            )
        })?;
        let range = range.start + source_offset..range.end + source_offset;
        drop(document);
        xml_text.replace_range(range, &replacement);
        return Ok(());
    }
    // A replace operation is also the repair path for a syntactically readable
    // legacy TypeDescription source. Failure to project the old wire value into
    // the logical algebra must not prevent replacing it with a valid target.
    let current = parse_meta_event_subscription_source(source_node).ok();
    // `v8:TypeId` members name types of the extended configuration that the
    // logical algebra cannot express; replace acts on logical members only
    // and keeps them verbatim (#1361).
    let kept_type_ids = current
        .as_ref()
        .map(|current| current.type_ids.clone())
        .unwrap_or_default();
    let requested = canonical_meta_event_sources(&requested);
    if current
        .as_ref()
        .is_some_and(|current| canonical_meta_event_sources(&current.sources) == requested)
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
    let replacement =
        emit_meta_event_subscription_source(&indent, &requested, &kept_type_ids, source_node)
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

pub fn typed_template_primary_path(
    template_type: unica_metadata::validation_subject::MetadataTemplateType,
) -> &'static str {
    match template_type {
        unica_metadata::validation_subject::MetadataTemplateType::HtmlDocument
        | unica_metadata::validation_subject::MetadataTemplateType::SpreadsheetDocument
        | unica_metadata::validation_subject::MetadataTemplateType::DataCompositionSchema
        | unica_metadata::validation_subject::MetadataTemplateType::DataCompositionAppearanceTemplate => "Ext/Template.xml",
        unica_metadata::validation_subject::MetadataTemplateType::TextDocument => "Ext/Template.txt",
        unica_metadata::validation_subject::MetadataTemplateType::BinaryData | unica_metadata::validation_subject::MetadataTemplateType::AddIn => "Ext/Template.bin",
    }
}

pub fn apply_typed_operations_to_image_with_seed(
    xml_text: &mut String,
    operations: &[unica_metadata::MetaEditOperation],
    lang: &str,
    seed: &[u8],
) -> Result<(), unica_metadata::request::MetaFailure> {
    use sha2::{Digest, Sha256};

    let mut counter = 0_u64;
    let mut next_uuid = || {
        let mut hasher = Sha256::new();
        hasher.update(b"unica-v13-metadata-uuid-v1\0");
        hasher.update(seed);
        hasher.update(counter.to_be_bytes());
        counter = counter.saturating_add(1);
        let digest = hasher.finalize();
        let mut bytes = [0_u8; 16];
        bytes.copy_from_slice(&digest[..16]);
        bytes[6] = (bytes[6] & 0x0f) | 0x40;
        bytes[8] = (bytes[8] & 0x3f) | 0x80;
        uuid::Uuid::from_bytes(bytes).to_string()
    };
    apply_typed_operations_with_uuid(xml_text, operations, lang, &mut next_uuid).map(|_| ())
}

pub fn typed_child_storage_profile(collection: MetaCollection) -> Option<TypedChildStorageProfile> {
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
        MetaCollection::Commands => Some(TYPED_COMMAND_STORAGE_PROFILE),
        _ => None,
    }
}

#[derive(Debug)]
pub struct TypedChildFootprintProfile {
    pub logical_profile: unica_metadata::validation_subject::MetadataChildProfile,
    pub required_files: BTreeSet<PathBuf>,
    pub optional_files: BTreeSet<PathBuf>,
}

pub fn typed_child_footprint_profile(
    collection: MetaCollection,
    template_type: Option<unica_metadata::validation_subject::MetadataTemplateType>,
    child: &MetadataAddress,
) -> Result<TypedChildFootprintProfile, unica_metadata::request::MetaFailure> {
    let paths = |values: &[&str]| values.iter().map(PathBuf::from).collect::<BTreeSet<_>>();
    match collection {
        MetaCollection::Forms => Ok(TypedChildFootprintProfile {
            logical_profile: unica_metadata::validation_subject::MetadataChildProfile::Form,
            required_files: paths(&[TYPED_FORM_CONTENT_PATH]),
            optional_files: paths(&[TYPED_FORM_MODULE_PATH]),
        }),
        MetaCollection::Commands => Ok(TypedChildFootprintProfile {
            logical_profile: unica_metadata::validation_subject::MetadataChildProfile::Command,
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
                logical_profile: unica_metadata::validation_subject::MetadataChildProfile::Template(
                    template_type,
                ),
                required_files: paths(&[typed_template_primary_path(template_type)]),
                optional_files: BTreeSet::new(),
            })
        }
        _ => unreachable!(),
    }
}

pub const TYPED_FORM_CONTENT_PATH: &str = "Ext/Form.xml";

pub const TYPED_ORDINARY_FORM_CONTENT_PATH: &str = "Ext/Form.bin";

/// A managed form's module. Designer nests it one level below the other `Ext`
/// members — `Ext/Form/Module.bsl`, not `Ext/Module.bsl`, which is where an
/// *object* module lives. Every other reader in the crate already uses this
/// path; the footprint contract used to disagree with them (issue #360).
pub const TYPED_FORM_MODULE_PATH: &str = "Ext/Form/Module.bsl";

pub fn typed_template_page_path(page: &str) -> PathBuf {
    PathBuf::from(format!("Ext/Template/{page}.html"))
}

/// The payload members Unica models for a child: the ones it parses, rewrites
/// and can address. Everything else a real payload holds is retained instead
/// (see [`typed_child_retained_payload`]). Both the planner and the validator
/// derive their expectations from here, so neither can drift from the other.
pub fn typed_child_modelled_payload(
    profile: unica_metadata::validation_subject::MetadataChildProfile,
    has_module: bool,
    html_pages: &[String],
) -> BTreeSet<PathBuf> {
    let mut files = BTreeSet::new();
    match profile {
        unica_metadata::validation_subject::MetadataChildProfile::OrdinaryForm => {
            files.insert(PathBuf::from(TYPED_ORDINARY_FORM_CONTENT_PATH));
        }
        unica_metadata::validation_subject::MetadataChildProfile::Form => {
            files.insert(PathBuf::from(TYPED_FORM_CONTENT_PATH));
            if has_module {
                files.insert(PathBuf::from(TYPED_FORM_MODULE_PATH));
            }
        }
        unica_metadata::validation_subject::MetadataChildProfile::Command => {
            if has_module {
                files.insert(PathBuf::from(TYPED_COMMAND_MODULE_PATH));
            }
        }
        unica_metadata::validation_subject::MetadataChildProfile::Template(template_type) => {
            files.insert(PathBuf::from(typed_template_primary_path(template_type)));
            if template_type
                == unica_metadata::validation_subject::MetadataTemplateType::HtmlDocument
            {
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
pub fn typed_child_retained_payload(
    profile: unica_metadata::validation_subject::MetadataChildProfile,
    relative: &Path,
) -> bool {
    match profile {
        unica_metadata::validation_subject::MetadataChildProfile::OrdinaryForm => {
            relative == Path::new("Ext/Help.xml") || relative.starts_with("Ext/Help")
        }
        unica_metadata::validation_subject::MetadataChildProfile::Form => {
            relative == Path::new("Ext/Help.xml")
                || relative.starts_with("Ext/Help")
                || relative.starts_with("Ext/Form/Items")
        }
        unica_metadata::validation_subject::MetadataChildProfile::Template(
            unica_metadata::validation_subject::MetadataTemplateType::HtmlDocument,
        ) => relative.starts_with("Ext/Template/_files"),
        unica_metadata::validation_subject::MetadataChildProfile::Command
        | unica_metadata::validation_subject::MetadataChildProfile::Template(_) => false,
    }
}

/// The directory topology of a child payload is exactly the parent closure of
/// its files: every directory holds something, and nothing holds nothing. That
/// single rule reproduces each shape the platform writes and still rejects a
/// stray empty directory.
pub fn typed_child_payload_directories<'a>(
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

pub fn typed_child_directory_kind(
    relative: &Path,
) -> unica_metadata::validation_subject::MetadataChildDirectoryKind {
    if relative.as_os_str().is_empty() {
        unica_metadata::validation_subject::MetadataChildDirectoryKind::Root
    } else if relative == Path::new("Ext") {
        unica_metadata::validation_subject::MetadataChildDirectoryKind::Extension
    } else if relative == Path::new("Ext/Template") {
        unica_metadata::validation_subject::MetadataChildDirectoryKind::HtmlPages
    } else {
        unica_metadata::validation_subject::MetadataChildDirectoryKind::Nested
    }
}

pub fn validate_typed_child_footprint(
    collection: MetaCollection,
    template_type: Option<unica_metadata::validation_subject::MetadataTemplateType>,
    child: &MetadataAddress,
    files: &[(PathBuf, Vec<u8>)],
    directories: &[PathBuf],
) -> Result<
    unica_metadata::validation_subject::MetadataChildFootprintEvidence,
    unica_metadata::request::MetaFailure,
> {
    let profile = typed_child_footprint_profile(collection, template_type, child)?;
    validate_child_footprint_profile(profile, child, files, directories)
}

pub fn validate_child_footprint_profile(
    profile: TypedChildFootprintProfile,
    child: &MetadataAddress,
    files: &[(PathBuf, Vec<u8>)],
    directories: &[PathBuf],
) -> Result<
    unica_metadata::validation_subject::MetadataChildFootprintEvidence,
    unica_metadata::request::MetaFailure,
> {
    let mut required_files = profile.required_files.clone();
    if profile.logical_profile
        == unica_metadata::validation_subject::MetadataChildProfile::Template(
            unica_metadata::validation_subject::MetadataTemplateType::HtmlDocument,
        )
    {
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

    Ok(
        unica_metadata::validation_subject::MetadataChildFootprintEvidence {
            child: child.clone(),
            profile: profile.logical_profile,
            directories: observed_directories
                .iter()
                .map(|relative| typed_child_directory_kind(relative))
                .collect(),
            retained,
        },
    )
}

pub fn typed_child_resource_failure(
    child: &MetadataAddress,
    message: &str,
) -> unica_metadata::request::MetaFailure {
    typed_diagnostic(
        MetaDiagnosticCode::ProviderUnavailable,
        format!("{message}: {child}"),
        Some("resources.child.payload"),
    )
    .with_metadata_path(child.clone())
    .into()
}

pub fn typed_child_topology_failure(
    child: &MetadataAddress,
    message: &str,
) -> unica_metadata::request::MetaFailure {
    typed_diagnostic(
        MetaDiagnosticCode::ProviderUnavailable,
        format!("{message}: {child}"),
        Some("resources.child.topology"),
    )
    .with_metadata_path(child.clone())
    .into()
}

pub fn typed_child_descriptor_image(
    owner_xml: &str,
    tag: &str,
    name: &str,
) -> Result<Vec<u8>, unica_metadata::request::MetaFailure> {
    typed_child_element_xml(owner_xml, tag, name)
        .map(|child_xml| wrap_standalone_descriptor(&child_xml))
}

pub fn wrap_standalone_descriptor(child_xml: &str) -> Vec<u8> {
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<MetaDataObject {} version=\"2.20\">\n{}\n</MetaDataObject>",
        crate::template_catalog::meta_xmlns_decl(),
        child_xml
    )
    .into_bytes()
}

pub fn typed_child_element_xml(
    owner_xml: &str,
    tag: &str,
    name: &str,
) -> Result<String, unica_metadata::request::MetaFailure> {
    let document = Document::parse(owner_xml.trim_start_matches('\u{feff}')).map_err(|_| {
        unica_metadata::request::MetaFailure::from(typed_diagnostic(
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
            unica_metadata::request::MetaFailure::from(typed_diagnostic(
                MetaDiagnosticCode::ProviderUnavailable,
                "typed child descriptor is missing from owner post-image",
                Some("elements"),
            ))
        })?;
    Ok(owner_xml[child.range()].to_string())
}

pub fn restore_initial_empty_top_child_objects(pre_image: &str, post_image: &mut String) {
    let Ok(pre_document) = Document::parse(pre_image) else {
        return;
    };
    let Ok(pre_object) = meta_edit_object_node(&pre_document) else {
        return;
    };
    let Some(pre_children) = unica_xml_format::read::child_element(pre_object, "ChildObjects")
    else {
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
    let Some(post_children) = unica_xml_format::read::child_element(post_object, "ChildObjects")
    else {
        return;
    };
    if post_children.children().any(|child| child.is_element()) {
        return;
    }
    let post_range = post_children.range();
    drop(post_document);
    post_image.replace_range(post_range, pre_source);
}

/// Top-level owner `ChildObjects` children whose descriptors live in their
/// own files (`Forms/<Name>.xml`, `Templates/<Name>.xml`).
pub const STANDALONE_CHILD_TAGS: [(MetaCollection, &str); 2] = [
    (MetaCollection::Forms, "Form"),
    (MetaCollection::Templates, "Template"),
];

/// Names of standalone children that an owner image describes inline.
pub fn inline_standalone_child_names(owner_xml: &str) -> BTreeSet<(&'static str, String)> {
    let mut names = BTreeSet::new();
    let Ok(document) = Document::parse(owner_xml.trim_start_matches('\u{feff}')) else {
        return names;
    };
    let Ok(object) = meta_edit_object_node(&document) else {
        return names;
    };
    let Some(children) = unica_xml_format::read::child_element(object, "ChildObjects") else {
        return names;
    };
    for (_, tag) in STANDALONE_CHILD_TAGS {
        for child in unica_xml_format::read::child_elements(children, tag) {
            if let Some(name) = meta_edit_child_object_name(child) {
                names.insert((tag, name));
            }
        }
    }
    names
}

/// The platform registers forms and templates in the owner by name only. A
/// typed update or removal edits the element's `Properties`, so the working
/// image temporarily carries the standalone descriptor in place of the name;
/// [`collapse_standalone_child_descriptors`] restores the reference after the
/// child resources are planned. A child whose descriptor cannot be read or
/// spliced keeps its reference and fails later as an unknown element.
pub fn expand_standalone_children_for_operations(
    reader: &dyn unica_source_contract::discovery::ProjectDiscoveryRead,
    xml: &mut String,
    descriptor_path: &Path,
    operations: &[MetaEditOperation],
) {
    let mut targets = Vec::<(MetaCollection, String)>::new();
    for operation in operations {
        match operation {
            MetaEditOperation::Update {
                collection,
                scope: None,
                elements,
            } => targets.extend(
                elements
                    .iter()
                    .map(|element| (*collection, element.name.clone())),
            ),
            MetaEditOperation::Remove {
                collection,
                scope: None,
                names,
            } => targets.extend(names.iter().map(|name| (*collection, name.clone()))),
            _ => {}
        }
    }
    let object_dir = descriptor_path.with_extension("");
    for (collection, name) in targets {
        let Some((_, tag)) = STANDALONE_CHILD_TAGS
            .iter()
            .find(|(candidate, _)| *candidate == collection)
        else {
            continue;
        };
        let Some(profile) = typed_child_storage_profile(collection) else {
            continue;
        };
        let reference = {
            let Ok(document) = Document::parse(xml.as_str()) else {
                return;
            };
            let Ok(object) = meta_edit_object_node(&document) else {
                return;
            };
            let Some(children) = unica_xml_format::read::child_element(object, "ChildObjects")
            else {
                return;
            };
            unica_xml_format::read::child_elements(children, tag)
                .into_iter()
                .filter(|child| !child.children().any(|node| node.is_element()))
                .find_map(|child| {
                    let text = unica_xml_format::read::direct_text(child)
                        .trim()
                        .to_string();
                    metadata_name_eq(&text, &name).then(|| (child.range(), text))
                })
        };
        let Some((range, physical_name)) = reference else {
            continue;
        };
        let path = object_dir
            .join(profile.directory)
            .join(format!("{physical_name}.xml"));
        let Ok(bytes) = unica_source_designer::read::read_bytes(reader, &path) else {
            continue;
        };
        let Ok(text) = String::from_utf8(bytes) else {
            continue;
        };
        let text = text.trim_start_matches('\u{feff}');
        let element = {
            let Ok(document) = Document::parse(text) else {
                continue;
            };
            let Some(child) = document
                .root_element()
                .children()
                .find(|node| node.is_element() && node.tag_name().name() == *tag)
            else {
                continue;
            };
            if meta_edit_child_object_name(child)
                .is_none_or(|candidate| !metadata_name_eq(&candidate, &physical_name))
            {
                continue;
            }
            text[child.range()].to_string()
        };
        let mut expanded = xml.clone();
        expanded.replace_range(range, &element);
        if Document::parse(&expanded).is_ok() {
            *xml = expanded;
        }
    }
}

/// Replace every inline form or template that the initial owner image did not
/// already describe inline with the platform's by-name registration.
pub fn collapse_standalone_child_descriptors(pre_image: &str, post_image: &mut String) {
    let initially_inline = inline_standalone_child_names(pre_image);
    collapse_children(post_image, &|tag, name, _| {
        !initially_inline.iter().any(|(inline_tag, inline_name)| {
            *inline_tag == tag && metadata_name_eq(inline_name, name)
        })
    });
}

/// Legacy inline form and template registrations of an edited owner (#1370):
/// before #1325 the owner carried the whole description next to the
/// standalone `Forms/<Name>.xml` or `Templates/<Name>.xml`, and 1C 8.3.27
/// then imports the configuration endlessly. Each inline registration whose
/// standalone descriptor `has_descriptor` becomes the by-name reference; the
/// collapsed `(tag, name)` pairs are returned for the plan's warning.
pub fn collapse_legacy_inline_children(
    image: &mut String,
    has_descriptor: &dyn Fn(&'static str, &str) -> bool,
) -> Vec<(&'static str, String)> {
    collapse_children(image, &|tag, name, inline| {
        inline && has_descriptor(tag, name)
    })
}

/// Rewrite to `<Tag>Name</Tag>` each form or template child that `collapse`
/// selects by tag, name and whether it is described inline; return the
/// inline ones rewritten.
fn collapse_children(
    post_image: &mut String,
    collapse: &dyn Fn(&'static str, &str, bool) -> bool,
) -> Vec<(&'static str, String)> {
    let mut collapsed = Vec::new();
    let replacements = {
        let Ok(document) = Document::parse(post_image.as_str()) else {
            return collapsed;
        };
        let Ok(object) = meta_edit_object_node(&document) else {
            return collapsed;
        };
        let Some(children) = unica_xml_format::read::child_element(object, "ChildObjects") else {
            return collapsed;
        };
        let mut replacements = Vec::new();
        for (_, tag) in STANDALONE_CHILD_TAGS {
            for child in unica_xml_format::read::child_elements(children, tag) {
                let Some(name) = meta_edit_child_object_name(child) else {
                    continue;
                };
                let inline = child.children().any(|node| node.is_element());
                if !collapse(tag, &name, inline) {
                    continue;
                }
                if inline {
                    collapsed.push((tag, name.clone()));
                }
                // Keep the element's own qualified name, so an owner that
                // spells MDClasses elements with a prefix stays in that
                // namespace.
                let range = child.range();
                let open = &post_image[range.start + 1..range.end];
                let qname_end = open
                    .find(|ch: char| ch.is_whitespace() || ch == '>' || ch == '/')
                    .unwrap_or(open.len());
                let qname = &open[..qname_end];
                replacements.push((
                    range,
                    format!(
                        "<{qname}>{}</{qname}>",
                        unica_xml_format::text::escape_xml(&name)
                    ),
                ));
            }
        }
        replacements
    };
    let mut replacements = replacements;
    replacements.sort_by_key(|(range, _)| std::cmp::Reverse(range.start));
    for (range, reference) in replacements {
        post_image.replace_range(range, &reference);
    }
    collapsed
}

#[cfg(test)]
mod legacy_inline_collapse_tests {
    use super::collapse_legacy_inline_children;

    /// An inline form or template with a standalone descriptor becomes the
    /// by-name reference in the owner's own namespace prefix; one without a
    /// descriptor, a by-name reference and an inline command stay (#1370).
    #[test]
    fn only_described_inline_forms_and_templates_are_collapsed() {
        let owner = concat!(
            "<md:MetaDataObject xmlns:md=\"http://v8.1c.ru/8.3/MDClasses\" version=\"2.20\">",
            "<md:Catalog uuid=\"1\"><md:Properties><md:Name>Items</md:Name></md:Properties>",
            "<md:ChildObjects>",
            "<md:Form uuid=\"2\"><md:Properties><md:Name>Main</md:Name></md:Properties></md:Form>",
            "<md:Form uuid=\"3\"><md:Properties><md:Name>Orphan</md:Name></md:Properties></md:Form>",
            "<md:Form>List</md:Form>",
            "<md:Template uuid=\"4\"><md:Properties><md:Name>Print</md:Name></md:Properties></md:Template>",
            "<md:Command uuid=\"5\"><md:Properties><md:Name>Open</md:Name></md:Properties></md:Command>",
            "</md:ChildObjects></md:Catalog></md:MetaDataObject>"
        );
        let mut image = owner.to_string();
        let collapsed = collapse_legacy_inline_children(&mut image, &|tag, name| {
            matches!(
                (tag, name),
                ("Form", "Main") | ("Form", "List") | ("Template", "Print")
            )
        });
        assert_eq!(
            collapsed,
            vec![
                ("Form", "Main".to_string()),
                ("Template", "Print".to_string())
            ]
        );
        assert!(image.contains("<md:Form>Main</md:Form>"), "{image}");
        assert!(
            image.contains("<md:Template>Print</md:Template>"),
            "{image}"
        );
        assert!(image.contains("<md:Name>Orphan</md:Name>"), "{image}");
        assert!(image.contains("<md:Form>List</md:Form>"), "{image}");
        assert!(image.contains("<md:Name>Open</md:Name>"), "{image}");
    }
}

#[cfg(test)]
#[path = "../tests/platform/edit_empty_child_objects.rs"]
mod empty_child_objects_tests;

#[cfg(test)]
#[path = "../tests/platform/dump_parity.rs"]
mod dump_parity_tests;

#[cfg(test)]
#[path = "../tests/platform/edit_dimension_deny.rs"]
mod dimension_deny_tests;

#[cfg(test)]
#[path = "../tests/platform/edit_attribute_copy.rs"]
mod attribute_copy_tests;
