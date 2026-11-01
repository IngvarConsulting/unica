//! Append columns without decoding or rewriting retained column types.
use std::collections::HashSet;

use roxmltree::{Document, Node};

use crate::root::FORM_LOGFORM_NS as FORM;
const CORE: &str = "http://v8.1c.ru/8.1/data/core";

#[derive(Debug, Clone)]
pub struct ColumnAddition {
    pub name: String,
    pub type_name: String,
    pub title: Option<String>,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum ColumnAddError {
    Missing(String),
    Invalid(String),
}

fn invalid(message: impl Into<String>) -> ColumnAddError {
    ColumnAddError::Invalid(message.into())
}

fn child<'a, 'input>(
    node: Node<'a, 'input>,
    name: &str,
) -> Result<Option<Node<'a, 'input>>, ColumnAddError> {
    let mut matches = node
        .children()
        .filter(|n| n.is_element() && n.tag_name().name() == name);
    let found = matches.next();
    if matches.next().is_some() || found.is_some_and(|n| n.tag_name().namespace() != Some(FORM)) {
        return Err(invalid(format!(
            "{name} is duplicated or has a foreign namespace"
        )));
    }
    Ok(found)
}

fn verify_table(attribute: Node<'_, '_>) -> Result<(), ColumnAddError> {
    let ty = child(attribute, "Type")?.ok_or_else(|| invalid("Attribute Type is missing"))?;
    let members: Vec<_> = ty.children().filter(Node::is_element).collect();
    if ty.attributes().len() != 0
        || !unica_xml_format::read::direct_text(ty).trim().is_empty()
        || members.len() != 1
    {
        return Err(invalid("Attribute must have exactly the ValueTable type"));
    }
    let member = members[0];
    let value = unica_xml_format::read::direct_text(member);
    let value = value.trim();
    let (prefix, local) = value
        .split_once(':')
        .map_or((None, value), |(prefix, local)| (Some(prefix), local));
    if !member.has_tag_name((CORE, "Type"))
        || member.attributes().len() != 0
        || member.children().any(|n| n.is_element())
        || local != "ValueTable"
        || member.lookup_namespace_uri(prefix) != Some(CORE)
    {
        return Err(invalid("Attribute must have exactly the ValueTable type"));
    }
    Ok(())
}

/// The column-add route recognizes the table by QName URI, independently of
/// the prefix used by the source. Other observed types keep their read profile.
pub(crate) fn observed_value_table_type(attribute: Node<'_, '_>) -> Option<String> {
    verify_table(attribute)
        .ok()
        .map(|()| "ValueTable".to_string())
}

fn emit(
    addition: &ColumnAddition,
    id: usize,
    lang: Option<&str>,
) -> Result<String, ColumnAddError> {
    if !unica_xml_format::names::xml_ncname_is_valid(&addition.name)
        || addition.type_name.trim().is_empty()
    {
        return Err(invalid("Column requires a valid name and a nonempty type"));
    }
    let mut lines = Vec::new();
    if let Some(title) = &addition.title {
        crate::codec::emit_form_mltext(&mut lines, "\t", "Title", title, lang).map_err(invalid)?;
    }
    crate::codec::emit_form_type(&mut lines, &addition.type_name, "\t").map_err(invalid)?;
    let body = lines.join("\n");
    let mut opening = format!(
        "<Column xmlns=\"{FORM}\" name=\"{}\" id=\"{id}\"",
        unica_xml_format::text::escape_xml(&addition.name)
    );
    for (prefix, uri) in crate::codec::form_edit_emitter_namespaces() {
        if body.contains(&format!("{prefix}:")) {
            opening.push_str(&format!(" xmlns:{prefix}=\"{uri}\""));
        }
    }
    let rendered = format!("{opening}>\n{body}\n</Column>");
    Document::parse(&rendered).map_err(|_| invalid("New column is not valid XML"))?;
    Ok(rendered)
}

/// Insert immediately before a closing tag; expand an empty tag only when needed.
fn append(
    source: &str,
    node: Node<'_, '_>,
    content: &str,
) -> Result<(std::ops::Range<usize>, String), ColumnAddError> {
    let range = node.range();
    let fragment = &source[range.clone()];
    if fragment.ends_with("/>") {
        let name = fragment[1..]
            .split(|ch: char| ch.is_whitespace() || ch == '/' || ch == '>')
            .next()
            .ok_or_else(|| invalid("Malformed container"))?;
        return Ok((range.end - 2..range.end, format!(">{content}</{name}>")));
    }
    let closing = fragment
        .rfind("</")
        .ok_or_else(|| invalid("Container closing tag is missing"))?;
    Ok((
        range.start + closing..range.start + closing,
        content.to_owned(),
    ))
}

pub(crate) fn apply(
    xml: &mut String,
    attribute: &str,
    additions: &[ColumnAddition],
    lang: Option<&str>,
) -> Result<(), ColumnAddError> {
    if additions.is_empty() {
        return Err(invalid("At least one column is required"));
    }
    let source = xml.trim_start_matches('\u{feff}');
    let offset = xml.len() - source.len();
    let doc = Document::parse(source).map_err(|_| invalid("Form is not valid XML"))?;
    let root = doc.root_element();
    if !root.has_tag_name((FORM, "Form")) {
        return Err(invalid("Expected a managed form"));
    }
    let attributes = child(root, "Attributes")?
        .ok_or_else(|| ColumnAddError::Missing(format!("Attribute `{attribute}` was not found")))?;
    let mut matches = attributes.children().filter(|n| {
        n.is_element()
            && n.tag_name().name() == "Attribute"
            && n.attribute("name") == Some(attribute)
    });
    let target = matches
        .next()
        .ok_or_else(|| ColumnAddError::Missing(format!("Attribute `{attribute}` was not found")))?;
    if matches.next().is_some() || target.tag_name().namespace() != Some(FORM) {
        return Err(invalid(
            "Attribute is duplicated or has a foreign namespace",
        ));
    }
    verify_table(target)?;
    let columns = child(target, "Columns")?;
    let mut names = HashSet::new();
    let mut ids = HashSet::new();
    let mut max_id = 0usize;
    if let Some(columns) = columns {
        if !unica_xml_format::read::direct_text(columns)
            .trim()
            .is_empty()
        {
            return Err(invalid("Columns has mixed content"));
        }
        for column in columns.children().filter(Node::is_element) {
            if !column.has_tag_name((FORM, "Column")) {
                return Err(invalid("Columns contains a foreign or unknown declaration"));
            }
            let name = column
                .attribute("name")
                .ok_or_else(|| invalid("Existing column has no name"))?;
            let id = column
                .attribute("id")
                .filter(|id| !id.is_empty() && id.bytes().all(|b| b.is_ascii_digit()))
                .and_then(|id| id.parse::<usize>().ok())
                .ok_or_else(|| invalid("Existing column has an invalid id"))?;
            if !unica_xml_format::names::xml_ncname_is_valid(name)
                || !names.insert(name.to_owned())
                || !ids.insert(id)
            {
                return Err(invalid(
                    "Existing columns contain invalid or duplicate names or ids",
                ));
            }
            max_id = max_id.max(id);
        }
    }
    let mut rendered = Vec::new();
    for addition in additions {
        if !names.insert(addition.name.clone()) {
            return Err(invalid(format!(
                "Column `{}` already exists",
                addition.name
            )));
        }
        max_id = max_id
            .checked_add(1)
            .ok_or_else(|| invalid("Column id range is exhausted"))?;
        rendered.push(emit(addition, max_id, lang)?);
    }
    let body = rendered.join("\n");
    let content = if columns.is_some() {
        body
    } else {
        format!("<Columns xmlns=\"{FORM}\">\n{body}\n</Columns>")
    };
    let content = if source.contains("\r\n") {
        content.replace('\n', "\r\n")
    } else {
        content
    };
    let (range, replacement) = append(source, columns.unwrap_or(target), &content)?;
    drop(doc);
    xml.replace_range(range.start + offset..range.end + offset, &replacement);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn addition(name: &str) -> ColumnAddition {
        ColumnAddition {
            name: name.into(),
            type_name: "DefinedType.Existing | CatalogRef.Additional".into(),
            title: None,
        }
    }
    fn table(name: &str, columns: &str) -> String {
        format!("<Attribute name=\"{name}\" id=\"9\"><Type><v8:Type>v8:ValueTable</v8:Type></Type>{columns}</Attribute>")
    }
    fn form(attributes: &str) -> String {
        format!("\u{feff}<Form xmlns=\"{FORM}\" xmlns:v8=\"{CORE}\" xmlns:cfg=\"urn:conflicting\"><ChildItems><InputField id=\"2\"><DataPath>A.Old</DataPath></InputField></ChildItems>\r\n<Attributes>{attributes}</Attributes></Form>")
    }
    const OLD: &str = "<Column name=\"Old\" id=\"7\"><Type><v8:FutureType strange=\"true\">opaque</v8:FutureType></Type></Column>";
    #[test]
    fn observed_valuetable_type_resolves_only_a_valid_core_qname() {
        let plain = form(&table("Rows", ""));
        let alias = plain
            .replace("xmlns:v8=", "xmlns:core=")
            .replace("v8:", "core:");
        let observed = crate::codec::parse_form_attributes_xml(&alias).unwrap();
        assert_eq!(observed[0].type_name.as_deref(), Some("ValueTable"));
        for xml in [
            alias.replace(CORE, "urn:foreign"),
            alias.replace("core:ValueTable", "core:ValueTree"),
            alias.replace(
                "<core:Type>core:ValueTable</core:Type>",
                "<core:Type>core:ValueTable</core:Type><core:Unknown/>",
            ),
            alias.replace(
                "<core:Type>core:ValueTable</core:Type>",
                "<core:Type>core:ValueTable<core:Nested/></core:Type>",
            ),
        ] {
            let document = Document::parse(xml.trim_start_matches('\u{feff}')).unwrap();
            let attribute = document
                .descendants()
                .find(|node| node.has_tag_name((FORM, "Attribute")))
                .unwrap();
            assert_eq!(observed_value_table_type(attribute), None, "{xml}");
        }
    }

    #[test]
    fn form_reader_preserves_healthy_collection_columns_across_qname_spellings() {
        let mut observations = Vec::new();
        for value_type in [
            "<v8:Type>v8:ValueTable</v8:Type>",
            "<core:Type xmlns:core=\"http://v8.1c.ru/8.1/data/core\">core:ValueTable</core:Type>",
            "<Type xmlns=\"http://v8.1c.ru/8.1/data/core\">ValueTable</Type>",
        ] {
            let xml = format!(
                r#"<Form xmlns="{FORM}" xmlns:v8="{CORE}" xmlns:cfg="http://v8.1c.ru/8.1/data/enterprise/current-config" xmlns:xs="http://www.w3.org/2001/XMLSchema" version="2.20"><Attributes><Attribute name="Rows" id="1"><Type>{value_type}</Type><Columns><Column name="Ref" id="1"><Type><v8:Type>cfg:CatalogRef.Items</v8:Type></Type></Column><Column name="Amount" id="2"><Type><v8:Type>xs:decimal</v8:Type><v8:NumberQualifiers><v8:Digits>10</v8:Digits><v8:FractionDigits>2</v8:FractionDigits><v8:AllowedSign>Any</v8:AllowedSign></v8:NumberQualifiers></Type></Column></Columns></Attribute><Attribute name="Tree" id="2"><Type><v8:Type>v8:ValueTree</v8:Type></Type><Columns><Column name="Text" id="1"><Type><v8:Type>xs:string</v8:Type><v8:StringQualifiers><v8:Length>20</v8:Length><v8:AllowedLength>Variable</v8:AllowedLength></v8:StringQualifiers></Type></Column></Columns></Attribute></Attributes></Form>"#
            );
            let data = crate::codec::parse_form_info_xml(
                &xml,
                "Main".into(),
                String::new(),
                unica_metadata::support_state::ObjectSupportData {
                    state: unica_metadata::support_state::ObjectSupportState::NotSupported,
                    direct_edit_safe: Some(true),
                },
            )
            .unwrap();
            let observed = serde_json::to_value(data.attributes).unwrap();
            assert_eq!(
                observed,
                serde_json::json!([
                    {"name":"Rows","type":"ValueTable","isMain":false,"mainTable":null,"columns":[
                        {"name":"Ref","type":"CatalogRef.Items"},
                        {"name":"Amount","type":"decimal(10,2)"}]},
                    {"name":"Tree","type":"ValueTree","isMain":false,"mainTable":null,"columns":[
                        {"name":"Text","type":"string(20)"}]}
                ])
            );
            observations.push(observed);
        }
        assert!(observations.windows(2).all(|pair| pair[0] == pair[1]));
    }

    #[test]
    fn two_tables_have_independent_ids_and_retain_unsupported_bytes() {
        let before = form(
            &(table("A", &format!("<Columns>{OLD}</Columns>"))
                + &table("B", "<Columns><Column name=\"Other\" id=\"2\"/></Columns>")),
        );
        let mut after = before.clone();
        apply(&mut after, "A", &[addition("NewA")], None).unwrap();
        apply(&mut after, "B", &[addition("NewB")], None).unwrap();
        assert!(after.contains(OLD));
        assert!(after.starts_with('\u{feff}'));
        assert!(!after.replace("\r\n", "").contains('\n'));
        let expected = before.replacen(
            "</Columns>",
            &(emit(&addition("NewA"), 8, None)
                .unwrap()
                .replace('\n', "\r\n")
                + "</Columns>"),
            1,
        );
        let expected = expected.replacen(
            "<Column name=\"Other\" id=\"2\"/></Columns>",
            &("<Column name=\"Other\" id=\"2\"/>".to_owned()
                + &emit(&addition("NewB"), 3, None)
                    .unwrap()
                    .replace('\n', "\r\n")
                + "</Columns>"),
            1,
        );
        assert_eq!(after, expected);
        let doc = Document::parse(after.trim_start_matches('\u{feff}')).unwrap();
        let new = doc
            .descendants()
            .find(|n| n.attribute("name") == Some("NewA"))
            .unwrap();
        assert_eq!(
            new.lookup_namespace_uri(Some("cfg")),
            Some("http://v8.1c.ru/8.1/data/enterprise/current-config")
        );
    }
    #[test]
    fn creates_missing_and_expands_empty_columns() {
        for columns in [
            "",
            "<Columns/>",
            "<f:Columns xmlns:f=\"http://v8.1c.ru/8.3/xcf/logform\" />",
        ] {
            let mut xml = form(&table("A", columns));
            apply(&mut xml, "A", &[addition("Новая")], None).unwrap();
            let doc = Document::parse(xml.trim_start_matches('\u{feff}')).unwrap();
            assert_eq!(
                doc.descendants()
                    .filter(|n| n.has_tag_name((FORM, "Columns")))
                    .count(),
                1
            );
            assert_eq!(
                doc.descendants()
                    .find(|n| n.attribute("name") == Some("Новая"))
                    .unwrap()
                    .attribute("id"),
                Some("1")
            );
        }
    }
    #[test]
    fn rejects_duplicates_bad_ids_overflow_and_foreign_containers_atomically() {
        for columns in [
            format!("<Columns>{OLD}{OLD}</Columns>"),
            format!("<Columns>{OLD}{}</Columns>", OLD.replace("Old", "Other")),
            format!(
                "<Columns>{}</Columns>",
                OLD.replace("7", &usize::MAX.to_string())
            ),
            format!("<Columns>{}</Columns>", OLD.replace("7", "-1")),
            "<Columns xmlns=\"urn:foreign\"/>".into(),
            "<Columns/><Columns/>".into(),
        ] {
            let before = form(&table("A", &columns));
            let mut after = before.clone();
            assert!(matches!(
                apply(&mut after, "A", &[addition("New")], None),
                Err(ColumnAddError::Invalid(_))
            ));
            assert_eq!(after, before);
        }
        let before = form(&table("A", &format!("<Columns>{OLD}</Columns>")));
        for additions in [
            vec![addition("New"), addition("Old")],
            vec![addition("New"), addition("New")],
            vec![
                addition("New"),
                ColumnAddition {
                    type_name: "Unsupported.Type".into(),
                    ..addition("Later")
                },
            ],
        ] {
            let mut after = before.clone();
            assert!(apply(&mut after, "A", &additions, None).is_err());
            assert_eq!(after, before);
        }
    }
    #[test]
    fn missing_parent_and_wrong_parent_type_are_distinct() {
        let before = form(&table("A", ""));
        let mut after = before.clone();
        assert!(matches!(
            apply(&mut after, "Missing", &[addition("New")], None),
            Err(ColumnAddError::Missing(_))
        ));
        assert_eq!(after, before);
        for before in [
            before.replace("v8:ValueTable", "v8:ValueTree"),
            before.replace(CORE, "urn:foreign"),
        ] {
            let mut after = before.clone();
            assert!(matches!(
                apply(&mut after, "A", &[addition("New")], None),
                Err(ColumnAddError::Invalid(_))
            ));
            assert_eq!(after, before);
        }
    }
    #[test]
    fn title_is_escaped_and_requires_resolved_language() {
        let before = form(&table("A", ""));
        let mut after = before.clone();
        let addition = ColumnAddition {
            title: Some("A & <B>".into()),
            ..addition("New")
        };
        assert!(apply(&mut after, "A", std::slice::from_ref(&addition), None).is_err());
        assert_eq!(after, before);
        apply(&mut after, "A", &[addition], Some("ru")).unwrap();
        assert!(after.contains("A &amp; &lt;B&gt;"));
    }

    #[test]
    fn aliases_and_zero_ids_are_preserved_and_invalid_xml_text_is_refused() {
        let before = form(&table(
            "A",
            &format!("<Columns>{}</Columns>", OLD.replace("7", "0")),
        ))
        .replace("xmlns:v8=", "xmlns:core=")
        .replace("v8:", "core:");
        let mut after = before.clone();
        apply(&mut after, "A", &[addition("New")], None).unwrap();
        assert!(after.contains("name=\"Old\" id=\"0\""));
        assert!(after.contains("name=\"New\" id=\"1\""));
        let mut unchanged = before.clone();
        let bad = ColumnAddition {
            title: Some("bad\u{1}".into()),
            ..addition("Bad")
        };
        assert!(apply(&mut unchanged, "A", &[addition("Good"), bad], Some("ru")).is_err());
        assert_eq!(unchanged, before);
    }
}
