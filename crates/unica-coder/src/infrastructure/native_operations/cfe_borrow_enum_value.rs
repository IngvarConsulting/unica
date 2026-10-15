//! Точечное заимствование EnumValue из описания перечисления-владельца.
use super::common::escape_xml;
use roxmltree::{Document, Node};

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
    if !super::cf::cf_validate_guid(value)
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
    if name.is_empty() {
        return Err("EnumValue name must not be empty".to_string());
    }
    let mut matches = Vec::new();
    let values = child(owner, "ChildObjects")?;
    for value in values
        .children()
        .filter(|n| n.has_tag_name((MD, "EnumValue")))
    {
        if text(child(value, "Properties")?, "Name")? == name {
            matches.push(value);
        }
    }
    let [value] = matches.as_slice() else {
        return Err(format!(
            "EnumValue '{name}' is missing or ambiguous in parent Enum"
        ));
    };
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
    Ok(*value)
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

fn desired_property(source: Option<Node<'_, '_>>, tag: &str) -> String {
    source
        .map(super::cfe_borrow_object::standalone_fragment)
        .unwrap_or_else(|| format!("<{tag} xmlns=\"{MD}\"/>"))
}

fn update_transferred_properties(
    existing_text: &str,
    existing_value: Node<'_, '_>,
    source_value: Node<'_, '_>,
) -> Result<Option<String>, String> {
    let existing_properties = child(existing_value, "Properties")?;
    let source_properties = child(source_value, "Properties")?;
    let mut edits = Vec::new();
    let mut missing = Vec::new();

    for tag in ["Synonym", "Comment"] {
        let existing = optional_property(existing_properties, tag)?;
        let desired = desired_property(optional_property(source_properties, tag)?, tag);
        let desired_doc = Document::parse(&desired).map_err(|e| e.to_string())?;
        if let Some(current) = existing {
            if !super::cfe_borrow_object::equivalent(current, desired_doc.root_element()) {
                edits.push((current.range(), desired));
            }
        } else {
            missing.push(desired);
        }
    }
    if edits.is_empty() && missing.is_empty() {
        return Ok(None);
    }

    if !missing.is_empty() {
        let properties_range = existing_properties.range();
        let closing = properties_range.start
            + existing_text[properties_range.clone()]
                .rfind("</")
                .ok_or("EnumValue Properties has no closing tag")?;
        let newline = if existing_text.contains("\r\n") {
            "\r\n"
        } else {
            "\n"
        };
        let insertion = missing
            .into_iter()
            .map(|property| format!("\t\t\t\t\t{property}{newline}"))
            .collect::<String>();
        edits.push((closing..closing, insertion));
    }

    edits.sort_by_key(|(range, _)| std::cmp::Reverse(range.start));
    let mut result = existing_text.to_string();
    for (range, replacement) in edits {
        result.replace_range(range, &replacement);
    }
    Ok(Some(result))
}

pub(crate) fn enum_value_source_uuid(parent: &[u8], name: &str) -> Result<String, String> {
    let raw = std::str::from_utf8(parent)
        .map_err(|e| e.to_string())?
        .trim_start_matches('\u{feff}');
    let document = Document::parse(raw).map_err(|e| e.to_string())?;
    Ok(selected(owner(&document)?, name)?
        .attribute("uuid")
        .unwrap()
        .to_string())
}

pub(crate) fn plan_borrow_enum_value(
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
            let Some(result) = update_transferred_properties(existing_text, same_name[0], source)?
            else {
                return Ok(existing_enum.to_vec());
            };
            Document::parse(&result).map_err(|e| e.to_string())?;
            let mut bytes = Vec::new();
            if existing_enum.starts_with(&[0xef, 0xbb, 0xbf]) {
                bytes.extend_from_slice(&[0xef, 0xbb, 0xbf]);
            }
            bytes.extend_from_slice(result.as_bytes());
            return Ok(bytes);
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
    let synonym = desired_property(optional_property(source_props, "Synonym")?, "Synonym");
    let comment = desired_property(optional_property(source_props, "Comment")?, "Comment");
    let newline = if existing_text.contains("\r\n") {
        "\r\n"
    } else {
        "\n"
    };
    let fragment = format!("\t\t\t<EnumValue xmlns=\"{MD}\" uuid=\"{}\">{newline}\t\t\t\t<Properties>{newline}\t\t\t\t\t<ObjectBelonging>Adopted</ObjectBelonging>{newline}\t\t\t\t\t<Name>{}</Name>{newline}\t\t\t\t\t<ExtendedConfigurationObject>{}</ExtendedConfigurationObject>{newline}\t\t\t\t\t{synonym}{newline}\t\t\t\t\t{comment}{newline}\t\t\t\t</Properties>{newline}\t\t\t</EnumValue>", escape_xml(value_uuid),escape_xml(name),escape_xml(source_uuid));
    if let Some(offset) =
        super::meta::platform_child_object_insertion_offset(existing_text, "EnumValue")
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
        format!(
            "{}{newline}{fragment}{newline}\t\t{}",
            &raw[..end],
            &raw[end..]
        )
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
pub(crate) mod tests {
    use super::*;
    pub(crate) const PARENT: &str = r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" xmlns:core="http://v8.1c.ru/8.1/data/core" version="2.20"><Enum uuid="11111111-1111-4111-8111-111111111111"><Properties><Name>Kinds</Name><Synonym/><Comment/></Properties><ChildObjects><EnumValue uuid="22222222-2222-4222-8222-222222222222"><Properties><Name>First</Name><Synonym><core:item><core:lang>ru</core:lang><core:content>Первое &amp; значение</core:content></core:item></Synonym><Comment>Источник</Comment></Properties></EnumValue><EnumValue uuid="33333333-3333-4333-8333-333333333333"><Properties><Name>Second</Name><Synonym/><Comment/></Properties></EnumValue></ChildObjects></Enum></MetaDataObject>"#;
    const VALUE_UUID: &str = "44444444-4444-4444-8444-444444444444";

    fn shell() -> Vec<u8> {
        super::super::cfe_borrow_object::plan_borrow_object(PARENT.as_bytes(), None, None)
            .unwrap()
            .bytes
    }

    #[test]
    fn enum_value_borrow_preserves_siblings_namespaces_and_noop_bytes() {
        let original = shell();
        let first =
            plan_borrow_enum_value(PARENT.as_bytes(), &original, "First", VALUE_UUID).unwrap();
        let second = plan_borrow_enum_value(
            PARENT.as_bytes(),
            &first,
            "Second",
            "55555555-5555-4555-8555-555555555555",
        )
        .unwrap();
        assert_eq!(
            plan_borrow_enum_value(
                PARENT.as_bytes(),
                &second,
                "First",
                "66666666-6666-4666-8666-666666666666"
            )
            .unwrap(),
            second
        );
        let xml_text = std::str::from_utf8(&second)
            .unwrap()
            .trim_start_matches('\u{feff}');
        let doc = Document::parse(xml_text).unwrap();
        let values = child(owner(&doc).unwrap(), "ChildObjects")
            .unwrap()
            .children()
            .filter(|n| n.has_tag_name((MD, "EnumValue")))
            .collect::<Vec<_>>();
        assert_eq!(values.len(), 2);
        assert_eq!(values[0].attribute("uuid"), Some(VALUE_UUID));
        assert_eq!(
            text(
                child(values[0], "Properties").unwrap(),
                "ExtendedConfigurationObject"
            )
            .unwrap(),
            "22222222-2222-4222-8222-222222222222"
        );
        assert!(values[0].descendants().any(|n| n
            .has_tag_name(("http://v8.1c.ru/8.1/data/core", "content"))
            && n.text() == Some("Первое & значение")));

        let refreshed_parent = PARENT
            .replace("Первое &amp; значение", "Обновлённое значение")
            .replace(
                "<Comment>Источник</Comment>",
                "<Comment>Новый комментарий</Comment>",
            );
        let refreshed = plan_borrow_enum_value(
            refreshed_parent.as_bytes(),
            &second,
            "First",
            "77777777-7777-4777-8777-777777777777",
        )
        .unwrap();
        let refreshed_text = std::str::from_utf8(&refreshed)
            .unwrap()
            .trim_start_matches('\u{feff}');
        let refreshed_doc = Document::parse(refreshed_text).unwrap();
        let refreshed_values = child(owner(&refreshed_doc).unwrap(), "ChildObjects")
            .unwrap()
            .children()
            .filter(|n| n.has_tag_name((MD, "EnumValue")))
            .collect::<Vec<_>>();
        assert_eq!(refreshed_values.len(), 2);
        assert_eq!(refreshed_values[0].attribute("uuid"), Some(VALUE_UUID));
        assert_eq!(
            text(child(refreshed_values[0], "Properties").unwrap(), "Comment").unwrap(),
            "Новый комментарий"
        );
        assert!(refreshed_values[0].descendants().any(|n| n
            .has_tag_name(("http://v8.1c.ru/8.1/data/core", "content"))
            && n.text() == Some("Обновлённое значение")));
        assert_eq!(
            refreshed_values[1].attribute("uuid"),
            Some("55555555-5555-4555-8555-555555555555")
        );
        assert_eq!(
            plan_borrow_enum_value(
                refreshed_parent.as_bytes(),
                &refreshed,
                "First",
                "88888888-8888-4888-8888-888888888888",
            )
            .unwrap(),
            refreshed
        );
    }

    #[test]
    fn enum_value_borrow_uses_the_platform_slot_before_later_child_kinds() {
        let shell = String::from_utf8(shell()).unwrap();
        let with_command = shell.replace(
            "<ChildObjects/>",
            "<ChildObjects>\n\t\t\t<Command>LocalCommand</Command>\n\t\t</ChildObjects>",
        );

        let borrowed = plan_borrow_enum_value(
            PARENT.as_bytes(),
            with_command.as_bytes(),
            "First",
            VALUE_UUID,
        )
        .unwrap();
        let text = std::str::from_utf8(&borrowed)
            .unwrap()
            .trim_start_matches('\u{feff}');
        let document = Document::parse(text).unwrap();
        let tags = child(owner(&document).unwrap(), "ChildObjects")
            .unwrap()
            .children()
            .filter(|node| node.is_element())
            .map(|node| node.tag_name().name())
            .collect::<Vec<_>>();
        assert_eq!(tags, ["EnumValue", "Command"]);
        assert!(text.contains("<Command>LocalCommand</Command>"));
    }

    #[test]
    fn enum_value_borrow_refuses_missing_duplicate_and_foreign_values() {
        let original = shell();
        assert!(
            plan_borrow_enum_value(PARENT.as_bytes(), &original, "Missing", VALUE_UUID).is_err()
        );
        let duplicate = PARENT.replace("<Name>Second</Name>", "<Name>First</Name>");
        assert!(
            plan_borrow_enum_value(duplicate.as_bytes(), &original, "First", VALUE_UUID).is_err()
        );
        let first =
            plan_borrow_enum_value(PARENT.as_bytes(), &original, "First", VALUE_UUID).unwrap();
        let original_text = String::from_utf8(first.clone()).unwrap();
        let damaged = original_text.replace(
            &format!("uuid=\"{VALUE_UUID}\""),
            "uuid=\"22222222-2222-4222-8222-222222222222\"",
        );
        assert!(
            plan_borrow_enum_value(PARENT.as_bytes(), damaged.as_bytes(), "First", VALUE_UUID)
                .is_err()
        );
        let own = original_text.replace(
            "<ObjectBelonging>Adopted</ObjectBelonging>",
            "<ObjectBelonging>Own</ObjectBelonging>",
        );
        assert!(
            plan_borrow_enum_value(PARENT.as_bytes(), own.as_bytes(), "First", VALUE_UUID).is_err()
        );
        let foreign = original_text.replace(
            "22222222-2222-4222-8222-222222222222",
            "99999999-9999-4999-8999-999999999999",
        );
        assert!(
            plan_borrow_enum_value(PARENT.as_bytes(), foreign.as_bytes(), "First", VALUE_UUID)
                .is_err()
        );
        assert!(plan_borrow_enum_value(PARENT.as_bytes(), &first, "Second", VALUE_UUID).is_err());
        assert!(plan_borrow_enum_value(
            PARENT.as_bytes(),
            &original,
            "First",
            "22222222-2222-4222-8222-222222222222"
        )
        .is_err());
    }
}
