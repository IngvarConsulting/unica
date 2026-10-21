//! Точечное заимствование EnumValue из описания перечисления-владельца.

use roxmltree::{Document, Node};
use std::collections::BTreeMap;

const MD: &str = "http://v8.1c.ru/8.3/MDClasses";

fn child<'a, 'input>(node: Node<'a, 'input>, tag: &str) -> Result<Node<'a, 'input>, String> {
    let nodes = node
        .children()
        .filter(|n| n.has_tag_name((MD, tag)))
        .collect::<Vec<_>>();
    match nodes.as_slice() {
        [node] => Ok(*node),
        _ => Err(format!("Enum borrowing requires exactly one {tag}")),
    }
}

fn text(node: Node<'_, '_>, tag: &str) -> Result<String, String> {
    let node = child(node, tag)?;
    if node.children().any(|n| n.is_element()) {
        return Err(format!("Invalid scalar {tag}"));
    }
    Ok(node.text().unwrap_or_default().to_string())
}

fn guid(value: &str) -> Result<(), String> {
    if !unica_metadata::configuration::cf_validate_guid(value)
        || value.eq_ignore_ascii_case("00000000-0000-0000-0000-000000000000")
    {
        return Err("Enum borrowing requires a valid non-nil UUID".to_string());
    }
    Ok(())
}

fn owner<'a, 'input>(document: &'a Document<'input>) -> Result<Node<'a, 'input>, String> {
    let root = document.root_element();
    if !root.has_tag_name((MD, "MetaDataObject")) || root.attribute("version") != Some("2.20") {
        return Err("Enum borrowing requires a 2.20 metadata descriptor".to_string());
    }
    let elements = root
        .children()
        .filter(|n| n.is_element())
        .collect::<Vec<_>>();
    let [node] = elements.as_slice() else {
        return Err("Enum descriptor must have one owner".to_string());
    };
    if !node.has_tag_name((MD, "Enum")) {
        return Err("EnumValue must belong to Enum".to_string());
    }
    guid(node.attribute("uuid").unwrap_or_default())?;
    child(*node, "Properties")?;
    child(*node, "ChildObjects")?;
    Ok(*node)
}

fn selected<'a, 'input>(owner: Node<'a, 'input>, name: &str) -> Result<Node<'a, 'input>, String> {
    let value = named_value(owner, name)?
        .ok_or_else(|| format!("EnumValue '{name}' is missing or ambiguous in parent Enum"))?;
    let values = child(owner, "ChildObjects")?;
    let uuid = value.attribute("uuid").unwrap_or_default();
    guid(uuid)?;
    if uuid.eq_ignore_ascii_case(owner.attribute("uuid").unwrap()) {
        return Err("Parent EnumValue UUID collides with its owner".to_string());
    }
    if values
        .children()
        .filter(|n| {
            n.has_tag_name((MD, "EnumValue"))
                && n.attribute("uuid")
                    .is_some_and(|id| id.eq_ignore_ascii_case(uuid))
        })
        .count()
        != 1
    {
        return Err("Parent EnumValue UUID is ambiguous".to_string());
    }
    Ok(value)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnumValueLookupErrorKind {
    NotFound,
    InvalidSource,
}

#[derive(Debug)]
pub struct EnumValueLookupError {
    pub kind: EnumValueLookupErrorKind,
    message: String,
}

impl EnumValueLookupError {
    fn not_found(message: impl Into<String>) -> Self {
        Self {
            kind: EnumValueLookupErrorKind::NotFound,
            message: message.into(),
        }
    }

    fn invalid_source(message: impl Into<String>) -> Self {
        Self {
            kind: EnumValueLookupErrorKind::InvalidSource,
            message: message.into(),
        }
    }
}

impl std::fmt::Display for EnumValueLookupError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for EnumValueLookupError {}

fn named_value<'a, 'input>(
    owner: Node<'a, 'input>,
    name: &str,
) -> Result<Option<Node<'a, 'input>>, String> {
    if name.is_empty() {
        return Err("EnumValue name must not be empty".to_string());
    }
    let values = child(owner, "ChildObjects")?;
    let mut matches = Vec::new();
    for value in values
        .children()
        .filter(|node| node.has_tag_name((MD, "EnumValue")))
    {
        if text(child(value, "Properties")?, "Name")? == name {
            matches.push(value);
        }
    }
    match matches.as_slice() {
        [] => Ok(None),
        [value] => Ok(Some(*value)),
        _ => Err(format!("EnumValue '{name}' is ambiguous in Enum")),
    }
}

fn optional_property<'a, 'input>(
    properties: Node<'a, 'input>,
    tag: &str,
) -> Result<Option<Node<'a, 'input>>, String> {
    let nodes = properties
        .children()
        .filter(|n| n.has_tag_name((MD, tag)))
        .collect::<Vec<_>>();
    match nodes.as_slice() {
        [] => Ok(None),
        [node] => Ok(Some(*node)),
        _ => Err(format!("Duplicate EnumValue property {tag}")),
    }
}

fn canonical_element(node: Node<'_, '_>, depth: usize, newline: &str) -> Result<String, String> {
    let indent = "\t".repeat(depth);
    let tag = crate::extension::cfe_borrow_prefixed_name(
        node.tag_name().namespace(),
        node.tag_name().name(),
    );
    let attributes = node
        .attributes()
        .map(|attribute| {
            let name =
                crate::extension::cfe_borrow_prefixed_name(attribute.namespace(), attribute.name());
            format!(
                " {name}=\"{}\"",
                unica_xml_format::text::escape_xml(attribute.value())
            )
        })
        .collect::<String>();
    let elements = node
        .children()
        .filter(|child| child.is_element())
        .collect::<Vec<_>>();
    if elements.is_empty() {
        let value = node.text().unwrap_or_default();
        return if value.is_empty() {
            Ok(format!("{indent}<{tag}{attributes}/>"))
        } else {
            Ok(format!(
                "{indent}<{tag}{attributes}>{}</{tag}>",
                unica_xml_format::text::escape_xml(value)
            ))
        };
    }
    if node
        .children()
        .filter(|child| child.is_text())
        .any(|child| !child.text().unwrap_or_default().trim().is_empty())
    {
        return Err(format!("Mixed content is not supported in EnumValue {tag}"));
    }
    let children = elements
        .into_iter()
        .map(|child| canonical_element(child, depth + 1, newline))
        .collect::<Result<Vec<_>, _>>()?
        .join(newline);
    Ok(format!(
        "{indent}<{tag}{attributes}>{newline}{children}{newline}{indent}</{tag}>"
    ))
}

fn canonical_property(
    source: Option<Node<'_, '_>>,
    tag: &str,
    depth: usize,
    newline: &str,
) -> Result<String, String> {
    source.map_or_else(
        || Ok(format!("{}<{tag}/>", "\t".repeat(depth))),
        |node| canonical_element(node, depth, newline),
    )
}

pub fn enum_value_source_uuid(parent: &[u8], name: &str) -> Result<String, EnumValueLookupError> {
    let raw = std::str::from_utf8(parent)
        .map_err(|error| EnumValueLookupError::invalid_source(error.to_string()))?
        .trim_start_matches('\u{feff}');
    let document = Document::parse(raw)
        .map_err(|error| EnumValueLookupError::invalid_source(error.to_string()))?;
    let owner = owner(&document).map_err(EnumValueLookupError::invalid_source)?;
    let value = named_value(owner, name)
        .map_err(EnumValueLookupError::invalid_source)?
        .ok_or_else(|| {
            EnumValueLookupError::not_found(format!("EnumValue '{name}' is missing in parent Enum"))
        })?;
    let uuid = value.attribute("uuid").unwrap_or_default();
    guid(uuid).map_err(EnumValueLookupError::invalid_source)?;
    if uuid.eq_ignore_ascii_case(owner.attribute("uuid").unwrap()) {
        return Err(EnumValueLookupError::invalid_source(
            "Parent EnumValue UUID collides with its owner",
        ));
    }
    let values = child(owner, "ChildObjects").map_err(EnumValueLookupError::invalid_source)?;
    if values
        .children()
        .filter(|node| {
            node.has_tag_name((MD, "EnumValue"))
                && node
                    .attribute("uuid")
                    .is_some_and(|id| id.eq_ignore_ascii_case(uuid))
        })
        .count()
        != 1
    {
        return Err(EnumValueLookupError::invalid_source(
            "Parent EnumValue UUID is ambiguous",
        ));
    }
    Ok(uuid.to_string())
}

pub fn enum_value_properties(
    descriptor: &[u8],
    name: &str,
) -> Result<Option<BTreeMap<String, String>>, EnumValueLookupError> {
    let raw = std::str::from_utf8(descriptor)
        .map_err(|error| EnumValueLookupError::invalid_source(error.to_string()))?
        .trim_start_matches('\u{feff}');
    let document = Document::parse(raw)
        .map_err(|error| EnumValueLookupError::invalid_source(error.to_string()))?;
    let owner = owner(&document).map_err(EnumValueLookupError::invalid_source)?;
    let Some(value) = named_value(owner, name).map_err(EnumValueLookupError::invalid_source)?
    else {
        return Ok(None);
    };
    let properties = child(value, "Properties").map_err(EnumValueLookupError::invalid_source)?;
    Ok(Some(
        properties
            .children()
            .filter(|node| node.is_element())
            .map(|node| {
                (
                    node.tag_name().name().to_string(),
                    raw[node.range()].to_string(),
                )
            })
            .collect(),
    ))
}

pub fn plan_borrow_enum_value(
    parent: &[u8],
    existing_enum: &[u8],
    name: &str,
    value_uuid: &str,
) -> Result<Vec<u8>, String> {
    let parent_text = std::str::from_utf8(parent)
        .map_err(|e| e.to_string())?
        .trim_start_matches('\u{feff}');
    let existing_text = std::str::from_utf8(existing_enum)
        .map_err(|e| e.to_string())?
        .trim_start_matches('\u{feff}');
    let parent_doc = Document::parse(parent_text).map_err(|e| e.to_string())?;
    let existing_doc = Document::parse(existing_text).map_err(|e| e.to_string())?;
    let parent_owner = owner(&parent_doc)?;
    let destination_owner = owner(&existing_doc)?;
    let source = selected(parent_owner, name)?;
    let source_uuid = source.attribute("uuid").unwrap();
    let props = child(destination_owner, "Properties")?;
    if text(props, "ObjectBelonging")? != "Adopted"
        || !text(props, "ExtendedConfigurationObject")?
            .eq_ignore_ascii_case(parent_owner.attribute("uuid").unwrap())
        || text(props, "Name")? != text(child(parent_owner, "Properties")?, "Name")?
    {
        return Err("Destination Enum is not adopted from the requested parent".to_string());
    }
    let children = child(destination_owner, "ChildObjects")?;
    let mut same_name = Vec::new();
    let mut local_ids = std::collections::HashSet::from([destination_owner
        .attribute("uuid")
        .unwrap()
        .to_lowercase()]);
    for value in children
        .children()
        .filter(|n| n.has_tag_name((MD, "EnumValue")))
    {
        let local_uuid = value.attribute("uuid").unwrap_or_default();
        guid(local_uuid)?;
        if !local_ids.insert(local_uuid.to_lowercase()) {
            return Err("Destination EnumValue UUID is ambiguous".to_string());
        }
        let properties = child(value, "Properties")?;
        let local_name = text(properties, "Name")?;
        let links = properties
            .children()
            .filter(|n| n.has_tag_name((MD, "ExtendedConfigurationObject")))
            .collect::<Vec<_>>();
        if links.len() > 1 {
            return Err("Duplicate EnumValue parent link".to_string());
        }
        if local_name == name {
            if local_uuid.eq_ignore_ascii_case(source_uuid) {
                return Err("Borrowed EnumValue must have a distinct extension UUID".to_string());
            }
            if text(properties, "ObjectBelonging")? != "Adopted"
                || !text(properties, "ExtendedConfigurationObject")?
                    .eq_ignore_ascii_case(source_uuid)
            {
                return Err(format!("EnumValue '{name}' conflicts with an extension-owned or differently borrowed value"));
            }
            same_name.push(value);
        } else if links
            .first()
            .and_then(|n| n.text())
            .is_some_and(|id| id.eq_ignore_ascii_case(source_uuid))
        {
            return Err("Source EnumValue is already borrowed under another name".to_string());
        }
        if local_name != name && local_uuid.eq_ignore_ascii_case(value_uuid) {
            return Err("EnumValue UUID collides with an existing value".to_string());
        }
    }
    match same_name.len() {
        1 => {
            let properties = child(same_name[0], "Properties")?;
            for tag in ["Synonym", "Comment"] {
                optional_property(properties, tag)?;
            }
            return Ok(existing_enum.to_vec());
        }
        0 => (),
        _ => return Err("Destination EnumValue name is ambiguous".to_string()),
    }
    guid(value_uuid)?;
    if value_uuid.eq_ignore_ascii_case(source_uuid)
        || value_uuid.eq_ignore_ascii_case(destination_owner.attribute("uuid").unwrap())
    {
        return Err("Borrowed EnumValue must have a distinct extension UUID".to_string());
    }
    let source_props = child(source, "Properties")?;
    let newline = if existing_text.contains("\r\n") {
        "\r\n"
    } else {
        "\n"
    };
    let synonym = canonical_property(
        optional_property(source_props, "Synonym")?,
        "Synonym",
        5,
        newline,
    )?;
    let fragment = [
        format!(
            "\t\t\t<EnumValue uuid=\"{}\">",
            unica_xml_format::text::escape_xml(value_uuid)
        ),
        "\t\t\t\t<InternalInfo/>".to_string(),
        "\t\t\t\t<Properties>".to_string(),
        "\t\t\t\t\t<ObjectBelonging>Adopted</ObjectBelonging>".to_string(),
        format!(
            "\t\t\t\t\t<Name>{}</Name>",
            unica_xml_format::text::escape_xml(name)
        ),
        synonym,
        "\t\t\t\t\t<Comment/>".to_string(),
        format!(
            "\t\t\t\t\t<ExtendedConfigurationObject>{}</ExtendedConfigurationObject>",
            unica_xml_format::text::escape_xml(source_uuid)
        ),
        "\t\t\t\t</Properties>".to_string(),
        "\t\t\t</EnumValue>".to_string(),
    ]
    .join(newline);
    if let Some(offset) =
        unica_source_designer::registration::platform_child_object_insertion_offset(
            existing_text,
            "EnumValue",
        )
    {
        let mut result = existing_text.to_string();
        result.insert_str(offset, &format!("{fragment}{newline}"));
        Document::parse(&result).map_err(|e| e.to_string())?;
        let mut bytes = Vec::new();
        if existing_enum.starts_with(&[0xef, 0xbb, 0xbf]) {
            bytes.extend_from_slice(&[0xef, 0xbb, 0xbf]);
        }
        bytes.extend_from_slice(result.as_bytes());
        return Ok(bytes);
    }
    let range = children.range();
    let raw = &existing_text[range.clone()];
    let replacement = if let Some(open) = raw.strip_suffix("/>") {
        let qname = raw[1..]
            .split(|ch: char| ch.is_whitespace() || ch == '/' || ch == '>')
            .next()
            .unwrap();
        format!("{}>{newline}{fragment}{newline}\t\t</{qname}>", open)
    } else {
        let end = raw.rfind("</").ok_or("ChildObjects has no closing tag")?;
        let line_start = raw[..end].rfind('\n').map_or(end, |index| index + 1);
        if raw[line_start..end]
            .chars()
            .all(|character| character == '\t' || character == ' ' || character == '\r')
        {
            format!(
                "{}{fragment}{newline}{}",
                &raw[..line_start],
                &raw[line_start..]
            )
        } else {
            format!(
                "{}{newline}{fragment}{newline}\t\t{}",
                &raw[..end],
                &raw[end..]
            )
        }
    };
    let mut result = existing_text.to_string();
    result.replace_range(range, &replacement);
    Document::parse(&result).map_err(|e| e.to_string())?;
    let mut bytes = Vec::new();
    if existing_enum.starts_with(&[0xef, 0xbb, 0xbf]) {
        bytes.extend_from_slice(&[0xef, 0xbb, 0xbf]);
    }
    bytes.extend_from_slice(result.as_bytes());
    Ok(bytes)
}

#[cfg(test)]
#[path = "../tests/platform/borrowed_enum_value.rs"]
mod tests;
