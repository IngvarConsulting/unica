//! Local type replacement for an existing ValueTable data column.
use roxmltree::{Document, Node};

use crate::root::FORM_LOGFORM_NS as FORM;
const CORE: &str = "http://v8.1c.ru/8.1/data/core";

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum ColumnTypeError {
    Missing(String),
    Invalid(String),
}

fn invalid(message: impl Into<String>) -> ColumnTypeError {
    ColumnTypeError::Invalid(message.into())
}

fn child<'a, 'input>(
    parent: Node<'a, 'input>,
    name: &str,
) -> Result<Node<'a, 'input>, ColumnTypeError> {
    let mut found = parent
        .children()
        .filter(|n| n.is_element() && n.tag_name().name() == name);
    let node = found.next().ok_or_else(|| {
        invalid(format!(
            "{name} is absent from the existing form declaration"
        ))
    })?;
    if found.next().is_some() || node.tag_name().namespace() != Some(FORM) {
        return Err(invalid(format!(
            "{name} is duplicated or has an invalid namespace"
        )));
    }
    Ok(node)
}

fn named<'a, 'input>(
    parent: Node<'a, 'input>,
    kind: &str,
    name: &str,
) -> Result<Node<'a, 'input>, ColumnTypeError> {
    let mut found = parent.children().filter(|n| {
        n.is_element() && n.tag_name().name() == kind && n.attribute("name") == Some(name)
    });
    let node = found
        .next()
        .ok_or_else(|| ColumnTypeError::Missing(format!("{kind} `{name}` was not found")))?;
    if found.next().is_some() || node.tag_name().namespace() != Some(FORM) {
        return Err(invalid(format!(
            "{kind} `{name}` is duplicated or has an invalid namespace"
        )));
    }
    Ok(node)
}

fn container<'a, 'input>(
    parent: Node<'a, 'input>,
    name: &str,
) -> Result<Node<'a, 'input>, ColumnTypeError> {
    if !parent
        .children()
        .any(|node| node.is_element() && node.tag_name().name() == name)
    {
        return Err(ColumnTypeError::Missing(format!(
            "{name} contains no declared target"
        )));
    }
    child(parent, name)
}

fn with_namespaces(body: &str) -> String {
    let mut opening = format!("<Type xmlns=\"{FORM}\"");
    for (prefix, uri) in crate::codec::form_edit_emitter_namespaces() {
        if body.contains(&format!("{prefix}:")) {
            opening.push_str(&format!(" xmlns:{prefix}=\"{uri}\""));
        }
    }
    opening.push('>');
    body.replacen("<Type>", &opening, 1)
}

fn emit(type_name: &str) -> Result<String, ColumnTypeError> {
    if type_name.trim().is_empty() {
        return Err(invalid("column type must not be empty"));
    }
    let mut lines = Vec::new();
    crate::codec::emit_form_type(&mut lines, type_name, "").map_err(invalid)?;
    Ok(with_namespaces(&lines.join("\n")))
}

pub(crate) fn validate_type(type_name: &str) -> Result<(), ColumnTypeError> {
    emit(type_name).map(|_| ())
}

fn canonical_qname(node: Node<'_, '_>, value: &str) -> Result<String, ColumnTypeError> {
    let (prefix, local) = value
        .split_once(':')
        .filter(|(prefix, local)| {
            unica_xml_format::names::xml_ncname_is_valid(prefix)
                && unica_xml_format::names::xml_ncname_is_valid(local)
        })
        .ok_or_else(|| invalid("existing column type contains an invalid QName"))?;
    let uri = node
        .lookup_namespace_uri(Some(prefix))
        .ok_or_else(|| invalid("existing column type contains an unbound QName"))?;
    let canonical = crate::codec::form_edit_emitter_namespaces()
        .into_iter()
        .find(|(_, candidate)| *candidate == uri)
        .map(|(prefix, _)| prefix)
        .ok_or_else(|| invalid("existing column type uses an unsupported QName namespace"))?;
    Ok(format!("{canonical}:{local}"))
}

/// Canonical spelling is only for comparing facts. It is never written back
/// over the old declaration merely to normalize its prefixes or whitespace.
fn canonical_node(node: Node<'_, '_>) -> Result<String, ColumnTypeError> {
    if node.tag_name().namespace() != Some(CORE) || node.attributes().len() != 0 {
        return Err(invalid(
            "existing column type has foreign nodes or unsupported attributes",
        ));
    }
    let name = node.tag_name().name();
    let elements: Vec<_> = node.children().filter(Node::is_element).collect();
    let text = unica_xml_format::read::direct_text(node);
    let content = if elements.is_empty() {
        let value = text.trim();
        let value = if matches!(name, "Type" | "TypeSet") {
            canonical_qname(node, value)?
        } else if matches!(name, "Digits" | "FractionDigits" | "Length") {
            value
                .parse::<u32>()
                .map_err(|_| invalid("existing column type has a malformed numeric qualifier"))?
                .to_string()
        } else {
            value.to_string()
        };
        unica_xml_format::text::escape_xml(&value)
    } else {
        if !text.trim().is_empty() {
            return Err(invalid("existing column type has mixed content"));
        }
        elements
            .into_iter()
            .map(canonical_node)
            .collect::<Result<Vec<_>, _>>()?
            .join("")
    };
    Ok(format!("<v8:{name}>{content}</v8:{name}>"))
}

fn canonical_type(node: Node<'_, '_>) -> Result<(String, Vec<String>), ColumnTypeError> {
    if !node.has_tag_name((FORM, "Type"))
        || node.attributes().len() != 0
        || !unica_xml_format::read::direct_text(node).trim().is_empty()
    {
        return Err(invalid("existing column Type has an invalid shape"));
    }
    let mut last_rank = 0;
    let mut children = Vec::new();
    for item in node.children().filter(Node::is_element) {
        let rank = match item.tag_name().name() {
            "Type" => 0,
            "TypeSet" => 1,
            "TypeId" => 2,
            "NumberQualifiers" => 3,
            "StringQualifiers" => 4,
            "DateQualifiers" => 5,
            _ => {
                return Err(invalid(
                    "existing column type contains an unsupported member",
                ))
            }
        };
        if rank < last_rank {
            return Err(invalid("existing column type has invalid member order"));
        }
        last_rank = rank;
        children.push(canonical_node(item)?);
    }
    if children.is_empty() {
        return Err(invalid("existing column type is empty"));
    }
    let normalized = with_namespaces(&format!("<Type>{}</Type>", children.join("")));
    children.sort();
    Ok((normalized, children))
}

fn supported_shape(node: Node<'_, '_>) -> Result<Vec<String>, ColumnTypeError> {
    let (normalized, shape) = canonical_type(node)?;
    let normalized_doc =
        Document::parse(&normalized).map_err(|error| invalid(error.to_string()))?;
    let type_name = crate::codec::form_format_type(normalized_doc.root_element());
    let emitted = emit(&type_name)?;
    let emitted_doc = Document::parse(&emitted).map_err(|error| invalid(error.to_string()))?;
    let (_, emitted_shape) = canonical_type(emitted_doc.root_element())?;
    if shape != emitted_shape {
        return Err(invalid("existing column type cannot be rewritten without losing unsupported content or qualifiers"));
    }
    Ok(shape)
}

pub(crate) fn apply(
    xml: &mut String,
    attribute: &str,
    column: &str,
    type_name: &str,
) -> Result<(), ColumnTypeError> {
    let replacement = emit(type_name)?;
    let replacement_doc =
        Document::parse(&replacement).map_err(|error| invalid(error.to_string()))?;
    let new_shape = supported_shape(replacement_doc.root_element())?;
    let source = xml.trim_start_matches('\u{feff}');
    let offset = xml.len() - source.len();
    let doc = Document::parse(source).map_err(|_| invalid("form is not valid XML"))?;
    let root = doc.root_element();
    if !root.has_tag_name((FORM, "Form")) {
        return Err(invalid("expected a managed form"));
    }
    let attributes = container(root, "Attributes")?;
    let attribute_node = named(attributes, "Attribute", attribute)?;
    let attribute_type = child(attribute_node, "Type")?;
    let shape = supported_shape(attribute_type)?;
    if shape != vec!["<v8:Type>v8:ValueTable</v8:Type>".to_string()] {
        return Err(invalid(
            "column type changes require a ValueTable attribute",
        ));
    }
    let columns = container(attribute_node, "Columns")?;
    let column_node = named(columns, "Column", column)?;
    let old_type = child(column_node, "Type")?;
    let old_shape = supported_shape(old_type)?;
    if old_shape == new_shape {
        return Ok(());
    }
    let range = old_type.range();
    let eol = if source.contains("\r\n") {
        "\r\n"
    } else {
        "\n"
    };
    let replacement = replacement.replace('\n', eol);
    drop(doc);
    xml.replace_range(range.start + offset..range.end + offset, &replacement);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    const OLD: &str = "<Type><v8:TypeSet>cfg:DefinedType.Existing</v8:TypeSet></Type>";

    fn form(value_type: &str) -> String {
        format!("\u{feff}<Form xmlns=\"{FORM}\" xmlns:v8=\"{CORE}\" xmlns:cfg=\"http://v8.1c.ru/8.1/data/enterprise/current-config\"><ChildItems><InputField name=\"Bound\" id=\"7\"><DataPath>Rows.Value</DataPath></InputField></ChildItems>\r\n<Attributes><Attribute name=\"Rows\" id=\"1\"><Type><v8:Type>v8:ValueTable</v8:Type></Type><Columns><Column name=\"Value\" id=\"2\">{value_type}<Title>untouched</Title></Column><Column name=\"Other\" id=\"3\">{OLD}</Column></Columns></Attribute></Attributes></Form>")
    }

    #[test]
    fn adds_catalog_reference_without_changing_any_other_bytes() {
        let before = form(OLD);
        let mut after = before.clone();
        apply(
            &mut after,
            "Rows",
            "Value",
            "DefinedType.Existing | CatalogRef.Additional",
        )
        .unwrap();
        let replacement = emit("DefinedType.Existing | CatalogRef.Additional")
            .unwrap()
            .replace('\n', "\r\n");
        assert_eq!(after, before.replacen(OLD, &replacement, 1));
        let doc = Document::parse(after.trim_start_matches('\u{feff}')).unwrap();
        let type_node = doc
            .descendants()
            .find(|n| n.has_tag_name((FORM, "Column")) && n.attribute("name") == Some("Value"))
            .unwrap()
            .children()
            .find(|n| n.has_tag_name((FORM, "Type")))
            .unwrap();
        assert_eq!(
            crate::codec::form_format_type(type_node),
            "CatalogRef.Additional | DefinedType.Existing"
        );
    }

    #[test]
    fn equivalent_composition_and_alias_prefixes_are_a_byte_noop() {
        let old = "<Type xmlns:c=\"http://v8.1c.ru/8.1/data/enterprise/current-config\" xmlns:b=\"http://v8.1c.ru/8.1/data/core\"><b:Type>c:CatalogRef.B</b:Type><b:Type>c:CatalogRef.A</b:Type><b:TypeSet>c:DefinedType.Existing</b:TypeSet></Type>";
        let before = form(old);
        let mut after = before.clone();
        apply(
            &mut after,
            "Rows",
            "Value",
            "DefinedType.Existing | CatalogRef.A | CatalogRef.B",
        )
        .unwrap();
        assert_eq!(after, before);
    }

    #[test]
    fn changing_a_type_shadows_conflicting_prefixes_only_inside_the_replacement() {
        let before = form("<Type xmlns:c=\"http://v8.1c.ru/8.1/data/enterprise/current-config\"><v8:TypeSet>c:DefinedType.Existing</v8:TypeSet></Type>").replace("xmlns:cfg=\"http://v8.1c.ru/8.1/data/enterprise/current-config\"", "xmlns:cfg=\"urn:foreign\"");
        let mut after = before.clone();
        apply(
            &mut after,
            "Rows",
            "Value",
            "DefinedType.Existing | CatalogRef.Additional",
        )
        .unwrap();
        let doc = Document::parse(after.trim_start_matches('\u{feff}')).unwrap();
        let added = doc
            .descendants()
            .find(|n| {
                n.has_tag_name((CORE, "Type")) && n.text() == Some("cfg:CatalogRef.Additional")
            })
            .unwrap();
        assert_eq!(
            added.lookup_namespace_uri(Some("cfg")),
            Some("http://v8.1c.ru/8.1/data/enterprise/current-config")
        );
        assert!(after.contains("xmlns:cfg=\"urn:foreign\""));
    }

    #[test]
    fn refuses_damaged_or_unsupported_old_types_without_writing() {
        for old in [
            "<Type/>",
            "<Type><v8:TypeSet>missing:DefinedType.Existing</v8:TypeSet></Type>",
            "<Type xmlns:cfg=\"urn:foreign\"><v8:TypeSet>cfg:DefinedType.Existing</v8:TypeSet></Type>",
            "<Type><v8:Unknown>value</v8:Unknown></Type>",
            "<Type><TypeSet>cfg:DefinedType.Existing</TypeSet></Type>",
            "<Type><v8:TypeSet unexpected=\"true\">cfg:DefinedType.Existing</v8:TypeSet></Type>",
            "<Type><v8:TypeSet>cfg:DefinedType.Existing</v8:TypeSet>unexpected</Type>",
            "<Type><v8:TypeSet>cfg:DefinedType.Existing</v8:TypeSet><v8:TypeSet>cfg:DefinedType.Existing</v8:TypeSet></Type>",
            "<Type xmlns:xs=\"http://www.w3.org/2001/XMLSchema\"><v8:Type>xs:string</v8:Type><v8:StringQualifiers><v8:Length>broken</v8:Length><v8:AllowedLength>Variable</v8:AllowedLength></v8:StringQualifiers></Type>",
            "<Type xmlns:xs=\"http://www.w3.org/2001/XMLSchema\"><v8:Type>xs:string</v8:Type><v8:StringQualifiers><v8:Length>10</v8:Length><v8:AllowedLength>Unknown</v8:AllowedLength></v8:StringQualifiers></Type>",
        ] {
            let before=form(old); let mut after=before.clone();
            assert!(matches!(apply(&mut after,"Rows","Value","CatalogRef.Additional"),Err(ColumnTypeError::Invalid(_))),"{old}");
            assert_eq!(after,before);
        }
    }

    #[test]
    fn qualifier_duplicates_and_unknown_children_cannot_be_silently_discarded() {
        let original = emit("decimal(10,2) | string(20)").unwrap();
        for damaged in [
            original.replace(
                "<v8:Digits>10</v8:Digits>",
                "<v8:Digits>10</v8:Digits><v8:Digits>10</v8:Digits>",
            ),
            original.replace(
                "</v8:StringQualifiers>",
                "<v8:FutureQualifier>true</v8:FutureQualifier></v8:StringQualifiers>",
            ),
            original.replace(
                "<v8:AllowedSign>Any</v8:AllowedSign>",
                "<v8:AllowedSign>Unknown</v8:AllowedSign>",
            ),
        ] {
            assert_ne!(damaged, original);
            let before = form(&damaged);
            let mut after = before.clone();
            assert!(matches!(
                apply(&mut after, "Rows", "Value", "CatalogRef.Additional"),
                Err(ColumnTypeError::Invalid(_))
            ));
            assert_eq!(before, after);
        }
        let before = form(&original);
        let mut after = before.clone();
        apply(&mut after, "Rows", "Value", "string(20) | decimal(10,2)").unwrap();
        assert_eq!(before, after);
    }

    #[test]
    fn exact_target_is_required_and_duplicate_declarations_are_invalid() {
        let before = form(OLD);
        for (attribute, column) in [("Absent", "Value"), ("Rows", "Absent")] {
            let mut after = before.clone();
            assert!(matches!(
                apply(&mut after, attribute, column, "CatalogRef.Additional"),
                Err(ColumnTypeError::Missing(_))
            ));
            assert_eq!(after, before);
        }
        for source in [
            before.replace("name=\"Other\"", "name=\"Value\""),
            before.replacen(OLD, &OLD.repeat(2), 1),
            before.replace("v8:ValueTable", "v8:ValueTree"),
        ] {
            let mut after = source.clone();
            assert!(matches!(
                apply(&mut after, "Rows", "Value", "CatalogRef.Additional"),
                Err(ColumnTypeError::Invalid(_))
            ));
            assert_eq!(after, source);
        }
    }

    #[test]
    fn new_type_validation_uses_the_existing_form_type_contract() {
        validate_type("DefinedType.Existing | CatalogRef.Additional").unwrap();
        for value in [
            "",
            "CatalogRef.Additional |",
            "CatalogRef.A | CatalogRef.A",
            "string(2000)",
            "Unknown.Type",
        ] {
            assert!(validate_type(value).is_err(), "{value}");
        }
    }

    #[test]
    fn absent_containers_are_missing_targets_but_duplicate_containers_are_invalid() {
        for before in [
            format!("<Form xmlns=\"{FORM}\"/>"),
            format!("<Form xmlns=\"{FORM}\" xmlns:v8=\"{CORE}\"><Attributes><Attribute name=\"Rows\"><Type><v8:Type>v8:ValueTable</v8:Type></Type></Attribute></Attributes></Form>"),
        ] {
            let mut after=before.clone();
            assert!(matches!(apply(&mut after,"Rows","Value","CatalogRef.Additional"),Err(ColumnTypeError::Missing(_))));
            assert_eq!(after,before);
        }
        for before in [
            form(OLD).replace("</Attributes>", "</Attributes><Attributes/>"),
            form(OLD).replace("</Columns>", "</Columns><Columns/>"),
        ] {
            let mut after = before.clone();
            assert!(matches!(
                apply(&mut after, "Rows", "Value", "CatalogRef.Additional"),
                Err(ColumnTypeError::Invalid(_))
            ));
            assert_eq!(after, before);
        }
    }
}
