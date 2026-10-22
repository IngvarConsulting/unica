//! Exact Designer child registration and byte-preserving proposal edits.
use crate::layout::metadata_kind_index;
use roxmltree::Document;
use std::path::PathBuf;
use unica_address::source_target::MetadataAddress;
use unica_source_contract::portable_path::PortablePathExt;
use unica_source_contract::proposal::input::bad;
use unica_source_contract::proposal::ApplySourceAuthority;
use unica_source_contract::proposal::{ApplyPlanError, ApplyPlanErrorKind, ApplyProposal};
const UTF8_BOM: &[u8] = b"\xef\xbb\xbf";

#[derive(Debug, Clone)]
pub struct CfEditChildObjectEntry {
    pub type_name: String,
    pub object_name: String,
    pub range: std::ops::Range<usize>,
    pub line_range: std::ops::Range<usize>,
}
pub type CfEditElementRange = (usize, usize, Option<(usize, usize)>);
pub fn cf_validate_child_object_type_index(type_name: &str) -> Option<usize> {
    metadata_kind_index(type_name)
}
pub fn cf_edit_child_object_entries(text: &str) -> Result<Vec<CfEditChildObjectEntry>, String> {
    cf_edit_root_child_objects(text).map(|(_, entries)| entries)
}
pub fn cf_edit_root_child_objects(
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
            object_name: unica_xml_format::text::unescape_xml(child.text().unwrap_or("").trim()),
            range,
            line_range: start..end,
        });
    }
    Ok(((child_start, child_end, body_range), entries))
}
pub fn cf_edit_line_indent(text: &str, index: usize) -> Option<String> {
    let line_start = cf_edit_line_start(text, index);
    let indent = &text[line_start..index];
    if indent.chars().all(|ch| ch == '\t' || ch == ' ') {
        Some(indent.to_string())
    } else {
        None
    }
}
pub fn cf_edit_line_start(text: &str, index: usize) -> usize {
    text[..index]
        .rfind(['\r', '\n'])
        .map_or(0, |position| position + 1)
}
pub fn cf_edit_child_name_key(value: &str) -> String {
    value.chars().flat_map(char::to_lowercase).collect()
}
pub fn cf_edit_child_name_cmp(left: &str, right: &str) -> std::cmp::Ordering {
    cf_edit_child_name_key(left)
        .cmp(&cf_edit_child_name_key(right))
        .then_with(|| left.cmp(right))
}
pub fn cf_edit_line_ending(text: &str) -> &'static str {
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
pub fn cf_edit_child_object_line(
    type_name: &str,
    object_name: &str,
    indent: &str,
    line_ending: &str,
) -> String {
    format!(
        "{indent}<{type_name}>{}</{type_name}>{line_ending}",
        unica_xml_format::text::escape_xml(object_name),
    )
}
pub fn cf_edit_add_child_object_text(
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
pub fn remove_metadata_child_text_with_flag(
    xml_text: &str,
    local_name: &str,
    item_name: &str,
) -> (String, bool) {
    let plain = format!("<{local_name}>{item_name}</{local_name}>");
    let prefixed = format!("<md:{local_name}>{item_name}</md:{local_name}>");
    let mut offset = 0;
    let own_line = xml_text.split_inclusive('\n').find_map(|line| {
        let line_start = offset;
        offset += line.len();
        let trimmed = line.trim();
        if trimmed == plain || trimmed == prefixed {
            let start = line_start + line.len() - line.trim_start().len();
            Some((start..start + trimmed.len(), line_start..offset))
        } else {
            None
        }
    });
    let inline = || {
        [&plain, &prefixed].into_iter().find_map(|needle| {
            xml_text.find(needle).map(|start| {
                let range = start..start + needle.len();
                (range.clone(), range)
            })
        })
    };
    let Some((element_range, removal_range)) = own_line.or_else(inline) else {
        return (xml_text.to_string(), false);
    };
    if let Some(result) = empty_child_objects_after_removal(xml_text, element_range) {
        return (result, true);
    }
    let mut result = xml_text.to_string();
    result.replace_range(removal_range, "");
    (result, true)
}
/// Use the native empty spelling only for the direct collection emptied by this removal.
pub fn empty_child_objects_after_removal(
    xml_text: &str,
    removed_range: std::ops::Range<usize>,
) -> Option<String> {
    const MD_NS: &str = "http://v8.1c.ru/8.3/MDClasses";
    let source = xml_text.strip_prefix('\u{feff}').unwrap_or(xml_text);
    let bom_len = xml_text.len() - source.len();
    let removed_range =
        removed_range.start.checked_sub(bom_len)?..removed_range.end.checked_sub(bom_len)?;
    let document = Document::parse(source).ok()?;
    let removed = document.descendants().find(|node| {
        node.is_element()
            && node.tag_name().namespace() == Some(MD_NS)
            && node.range() == removed_range
    })?;
    let parent = removed.parent_element()?;
    if !parent.has_tag_name((MD_NS, "ChildObjects")) {
        return None;
    }

    let parent_range = parent.range();
    let parent_source = &source[parent_range.clone()];
    let opening_end = parent_range.start + parent_source.find('>')?;
    let closing_start = parent_range.start + parent_source.rfind("</")?;
    let opening = &source[parent_range.start + 1..opening_end];
    let xml_space = |byte: u8| matches!(byte, b' ' | b'\t' | b'\r' | b'\n');
    let name_end = opening.bytes().position(xml_space).unwrap_or(opening.len());
    // Inspect raw bytes, not decoded text: attributes, xmlns, comments, CDATA,
    // entities and non-XML whitespace must survive even when they look empty.
    if !opening[name_end..].bytes().all(xml_space)
        || opening_end >= removed_range.start
        || removed_range.end > closing_start
        || !source[opening_end + 1..removed_range.start]
            .bytes()
            .all(xml_space)
        || !source[removed_range.end..closing_start]
            .bytes()
            .all(xml_space)
    {
        return None;
    }

    let replacement = format!("{}/>", &source[parent_range.start..opening_end]);
    let mut result = xml_text.to_string();
    result.replace_range(
        parent_range.start + bom_len..parent_range.end + bom_len,
        &replacement,
    );
    Some(result)
}
/// Order in which the 8.3.27 export writes the kinds of an owner's top-level
/// `ChildObjects` entries. The export groups the entries by kind in this order
/// and keeps the order of entries inside a kind, so an entry written in any
/// other slot is moved by the first load and export (#1385).
///
/// Settled by loading descriptors with the kinds in reverse order into 8.3.27
/// (`ibcmd infobase create --import`, then `config export`) and by the
/// reference configuration dumps. The kinds differ per owner: a document puts
/// its forms before its tabular sections, an accounting register starts with
/// its dimensions. `None` leaves the placement to the caller.
pub fn platform_child_object_order(owner_kind: &str) -> Option<&'static [&'static str]> {
    const OBJECT: &[&str] = &["Attribute", "TabularSection", "Form", "Template", "Command"];
    Some(match owner_kind {
        "Catalog"
        | "ChartOfCharacteristicTypes"
        | "ChartOfCalculationTypes"
        | "ExchangePlan"
        | "BusinessProcess"
        | "Report"
        | "DataProcessor"
        | "ExternalReport"
        | "ExternalDataProcessor" => OBJECT,
        "Document" => &["Attribute", "Form", "TabularSection", "Template", "Command"],
        "ChartOfAccounts" => &[
            "Attribute",
            "TabularSection",
            "AccountingFlag",
            "ExtDimensionAccountingFlag",
            "Form",
            "Template",
            "Command",
        ],
        "Task" => &[
            "Attribute",
            "TabularSection",
            "Form",
            "Template",
            "AddressingAttribute",
            "Command",
        ],
        "InformationRegister" | "AccumulationRegister" => &[
            "Resource",
            "Attribute",
            "Dimension",
            "Form",
            "Template",
            "Command",
        ],
        "CalculationRegister" => &[
            "Resource",
            "Attribute",
            "Dimension",
            "Recalculation",
            "Form",
            "Template",
            "Command",
        ],
        "AccountingRegister" => &[
            "Dimension",
            "Resource",
            "Attribute",
            "Form",
            "Template",
            "Command",
        ],
        "Enum" => &["EnumValue", "Form", "Template", "Command"],
        "DocumentJournal" => &["Column", "Form", "Template", "Command"],
        _ => return None,
    })
}
/// The first top-level entry of `child_objects` whose kind the export writes
/// after `tag`: a new `tag` entry goes right before it. `None` means the new
/// entry belongs at the end. Entries of kinds outside the owner's order are
/// never an anchor.
pub fn platform_child_object_successor<'a, 'input>(
    owner_kind: &str,
    child_objects: roxmltree::Node<'a, 'input>,
    tag: &str,
) -> Option<roxmltree::Node<'a, 'input>> {
    let order = platform_child_object_order(owner_kind)?;
    let slot = order.iter().position(|kind| *kind == tag)?;
    child_objects
        .children()
        .filter(|child| child.is_element())
        .find(|child| {
            order
                .iter()
                .position(|kind| *kind == child.tag_name().name())
                .is_some_and(|existing| existing > slot)
        })
}
/// Text form of [`platform_child_object_successor`] for the registrars that
/// edit a descriptor as text: the byte offset in `text` where the line of a
/// new top-level `tag` entry is inserted, when an entry of a later kind
/// exists.
pub fn platform_child_object_insertion_offset(text: &str, tag: &str) -> Option<usize> {
    top_child_object_line_offset(text, |owner_kind, child_objects| {
        platform_child_object_successor(owner_kind, child_objects, tag)
    })
}
/// Byte offset of the line of the first top-level `ChildObjects` entry named
/// `tag`, for owners whose platform order is not known.
pub fn first_top_child_object_offset(text: &str, tag: &str) -> Option<usize> {
    top_child_object_line_offset(text, |_, child_objects| {
        child_objects
            .children()
            .find(|child| child.is_element() && child.tag_name().name() == tag)
    })
}
pub fn top_child_object_line_offset<'t>(
    text: &'t str,
    anchor: impl for<'a> Fn(&str, roxmltree::Node<'a, 't>) -> Option<roxmltree::Node<'a, 't>>,
) -> Option<usize> {
    let body = text.trim_start_matches('\u{feff}');
    let shift = text.len() - body.len();
    let doc = roxmltree::Document::parse(body).ok()?;
    let root = doc.root_element();
    if root.tag_name().name() != "MetaDataObject" {
        return None;
    }
    let object = root.children().find(|node| node.is_element())?;
    let child_objects = unica_xml_format::read::child_element(object, "ChildObjects")?;
    let start = shift
        + anchor(object.tag_name().name(), child_objects)?
            .range()
            .start;
    let line_start = text[..start].rfind('\n').map_or(0, |index| index + 1);
    Some(
        if text[line_start..start]
            .chars()
            .all(|ch| ch == '\t' || ch == ' ')
        {
            line_start
        } else {
            start
        },
    )
}
/// The owner descriptor (`Configuration.xml`) with `<Kind>Name</Kind>` added
/// to or removed from `ChildObjects`, keeping the byte-order mark and the
/// line endings of the original image.
pub fn owner_registration_image(
    owner: &[u8],
    kind: &str,
    name: &str,
    register: bool,
    op_index: usize,
) -> Result<Option<Vec<u8>>, ApplyPlanError> {
    use unica_xml_format::text::{preserve_inserted_line_endings, split_utf8_bom_prefix};
    let (bom, payload) = split_utf8_bom_prefix(owner);
    let source = std::str::from_utf8(payload).map_err(|_| {
        ApplyPlanError::new(
            ApplyPlanErrorKind::InvalidSource,
            "the configuration descriptor is not UTF-8",
        )
        .at_path(format!("ops[{op_index}].args.at"))
    })?;
    let (updated, changed) = if register {
        let mut updated = source.to_string();
        let changed = cf_edit_add_child_object_text(&mut updated, kind, name).map_err(|error| {
            ApplyPlanError::new(ApplyPlanErrorKind::InvalidSource, error)
                .at_path(format!("ops[{op_index}].args.at"))
        })?;
        (updated, changed)
    } else {
        remove_metadata_child_text_with_flag(source, kind, name)
    };
    if !changed {
        return Ok(None);
    }
    let updated = preserve_inserted_line_endings(source, &updated);
    let mut image = Vec::with_capacity(bom.len() + updated.len());
    image.extend_from_slice(bom);
    image.extend_from_slice(updated.as_bytes());
    Ok(Some(image))
}
/// The descriptor image after a text edit: the observed byte-order mark and
/// the source's line-ending profile are kept, as the typed engine does.
pub fn preserve_descriptor_image(preimage: &[u8], source: &str, updated: &str) -> Vec<u8> {
    use unica_xml_format::text::{preserve_inserted_line_endings, split_utf8_bom_prefix};
    let (bom, _) = split_utf8_bom_prefix(preimage);
    let updated = preserve_inserted_line_endings(source, updated);
    let mut image = Vec::with_capacity(bom.len() + updated.len());
    image.extend_from_slice(bom);
    image.extend_from_slice(updated.as_bytes());
    image
}
/// Removal planners list payload files and scan references on the physical
/// source root, so they only see the truth when nothing was staged before
/// them. A batch that edited files first must run the removal in its own
/// apply call; refusing here keeps a stale scan from committing dangling
/// references or leaving staged payload behind.
pub fn require_untouched_staged_state(
    staged: &ApplyProposal<'_>,
    operation: &str,
    at_path: &str,
) -> Result<(), ApplyPlanError> {
    if staged.planned_changes().is_empty() {
        return Ok(());
    }
    Err(ApplyPlanError::new(
        ApplyPlanErrorKind::InvalidState,
        format!(
            "`{operation}` inspects the source root and must run in its own apply call before other changes"
        ),
    )
    .at_path(at_path.to_string()))
}
/// A version-4 shaped uuid derived from the plan identity, so a preview and
/// its publication produce the same descriptor bytes.
pub fn seeded_uuid(seed: &str) -> String {
    use sha2::Digest;
    let digest = sha2::Sha256::digest(format!("unica-v13-form-resource-uuid-v1\0{seed}"));
    let mut bytes = [0_u8; 16];
    bytes.copy_from_slice(&digest[..16]);
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    uuid::Uuid::from_bytes(bytes).to_string()
}
pub fn stage_new_file(
    staged: &mut ApplyProposal<'_>,
    relative: &std::path::Path,
    text: &[u8],
    at_path: &str,
) -> Result<(), ApplyPlanError> {
    let existing = staged
        .read(relative)
        .map_err(|error| ApplyPlanError::staging(error, at_path.to_string()))?;
    if existing.is_some() {
        return Err(ApplyPlanError::new(
            ApplyPlanErrorKind::InvalidState,
            format!("`{}` already exists", relative.portable()),
        )
        .at_path(at_path.to_string()));
    }
    let mut bytes = UTF8_BOM.to_vec();
    bytes.extend_from_slice(text.strip_prefix(UTF8_BOM).unwrap_or(text));
    staged
        .create(relative, bytes)
        .map_err(|error| ApplyPlanError::staging(error, at_path.to_string()))
}
/// Adds `<Kind>Name</Kind>` to the configuration's child objects.
pub fn register_child(
    staged: &mut ApplyProposal<'_>,
    kind: &str,
    name: &str,
    op_index: usize,
) -> Result<PathBuf, ApplyPlanError> {
    let at_path = format!("ops[{op_index}].args.at");
    let owner = PathBuf::from("Configuration.xml");
    let preimage = staged
        .read(&owner)
        .map_err(|error| ApplyPlanError::staging(error, at_path.clone()))?
        .ok_or_else(|| {
            ApplyPlanError::new(
                ApplyPlanErrorKind::NotFound,
                "the configuration descriptor was not found",
            )
            .at_path(at_path.clone())
        })?;
    let Some(postimage) = owner_registration_image(&preimage, kind, name, true, op_index)? else {
        return Err(ApplyPlanError::new(
            ApplyPlanErrorKind::InvalidState,
            format!("the configuration already registers `{kind}.{name}`"),
        )
        .at_path(format!("ops[{op_index}].args.values.name")));
    };
    staged
        .replace(&owner, &preimage, postimage)
        .map_err(|error| ApplyPlanError::staging(error, at_path))?;
    Ok(owner)
}

/// `Kind.Name` → `Kinds/Name.xml`, the owner descriptor of a form.
pub fn owner_descriptor_relative(
    owner: &MetadataAddress,
    authority: &dyn ApplySourceAuthority,
    op_index: usize,
) -> Result<PathBuf, ApplyPlanError> {
    crate::layout::metadata_descriptor_relative(owner, authority.source_kind())
        .map_err(|message| bad(op_index, "at", message))
}
pub fn read_required(
    staged: &mut ApplyProposal<'_>,
    relative: &std::path::Path,
    what: &str,
    at_path: &str,
) -> Result<Vec<u8>, ApplyPlanError> {
    staged
        .read(relative)
        .map_err(|error| ApplyPlanError::staging(error, at_path.to_string()))?
        .ok_or_else(|| {
            ApplyPlanError::new(
                ApplyPlanErrorKind::NotFound,
                format!("{what} was not found"),
            )
            .at_path(at_path.to_string())
        })
}
pub fn utf8_body(bytes: &[u8], what: &str, at_path: &str) -> Result<String, ApplyPlanError> {
    String::from_utf8(bytes.strip_prefix(UTF8_BOM).unwrap_or(bytes).to_vec()).map_err(|_| {
        ApplyPlanError::new(
            ApplyPlanErrorKind::InvalidSource,
            format!("{what} is not UTF-8"),
        )
        .at_path(at_path.to_string())
    })
}
pub fn with_bom(text: &str) -> Vec<u8> {
    let mut bytes = UTF8_BOM.to_vec();
    bytes.extend_from_slice(text.as_bytes());
    bytes
}
