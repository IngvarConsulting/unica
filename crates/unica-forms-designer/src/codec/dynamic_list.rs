//! Observed design-time settings; absence never supplies a runtime default.
use roxmltree::Node;
use unica_forms::facts::FormInfoDynamicList;

use crate::root::FORM_LOGFORM_NS as FORM;
const XSI: &str = "http://www.w3.org/2001/XMLSchema-instance";

type Read<T> = Result<T, String>;

fn one<'a, 'i>(parent: Node<'a, 'i>, name: &str) -> Read<Option<Node<'a, 'i>>> {
    let mut nodes = parent.children().filter(|n| n.has_tag_name((FORM, name)));
    let first = nodes.next();
    if nodes.next().is_some() {
        return Err(format!("DynamicList {name}: duplicate setting"));
    }
    Ok(first)
}

fn text(node: Node<'_, '_>) -> Read<String> {
    if node.children().any(|n| n.is_element()) || node.attributes().len() != 0 {
        return Err(format!(
            "DynamicList {}: expected scalar text",
            node.tag_name().name()
        ));
    }
    Ok(node
        .children()
        .filter(Node::is_text)
        .filter_map(|n| n.text())
        .collect())
}

fn scalar(settings: Node<'_, '_>, name: &str) -> Read<Option<String>> {
    one(settings, name)?.map(text).transpose()
}

fn flag(settings: Node<'_, '_>, name: &str) -> Read<Option<bool>> {
    scalar(settings, name)?
        .map(|raw| match raw.trim() {
            "true" | "1" => Ok(true),
            "false" | "0" => Ok(false),
            _ => Err(format!("DynamicList {name}: invalid boolean")),
        })
        .transpose()
}

fn observed<T>(value: Read<Option<T>>, limits: &mut Vec<String>) -> Option<T> {
    match value {
        Ok(value) => value,
        Err(reason) => {
            limits.push(reason);
            None
        }
    }
}

/// Confirm this profile without relying on the legacy type display formatter.
pub(super) fn is_dynamic_list(attribute: Node<'_, '_>) -> bool {
    if !attribute.has_tag_name((FORM, "Attribute")) {
        return false;
    }
    let Ok(Some(type_node)) = one(attribute, "Type") else {
        return false;
    };
    let mut children = type_node.children().filter(Node::is_element);
    let Some(value) = children.next() else {
        return false;
    };
    if !value.has_tag_name((super::FORM_V8_NS, "Type")) || children.next().is_some() {
        return false;
    }
    let Ok(raw) = text(value) else {
        return false;
    };
    let raw = raw.trim();
    let (prefix, local) = raw
        .split_once(':')
        .map_or((None, raw), |(p, n)| (Some(p), n));
    local == "DynamicList"
        && value.lookup_namespace_uri(prefix)
            == Some("http://v8.1c.ru/8.1/data/enterprise/current-config")
}

pub(super) fn read(attribute: Node<'_, '_>) -> FormInfoDynamicList {
    let mut out = FormInfoDynamicList::default();
    let settings = match one(attribute, "Settings") {
        Ok(Some(settings)) => settings,
        Ok(None) => {
            out.key_fields_applicable = Some(true);
            return out;
        }
        Err(reason) => {
            out.limits.push(reason);
            return out;
        }
    };
    let valid_type = settings.attribute((XSI, "type")).is_some_and(|raw| {
        let (prefix, local) = raw
            .split_once(':')
            .map_or((None, raw), |(p, n)| (Some(p), n));
        local == "DynamicList" && settings.lookup_namespace_uri(prefix) == Some(FORM)
    });
    if !valid_type {
        out.limits
            .push("DynamicList Settings: missing or unsupported xsi:type".into());
        return out;
    }
    let main_table = scalar(settings, "MainTable");
    out.key_fields_applicable = main_table
        .as_ref()
        .ok()
        .map(|value| value.as_ref().is_none_or(|text| text.trim().is_empty()));
    out.main_table = observed(main_table, &mut out.limits);
    out.manual_query = observed(flag(settings, "ManualQuery"), &mut out.limits);
    out.query_text = observed(scalar(settings, "QueryText"), &mut out.limits);
    out.dynamic_data_read = observed(flag(settings, "DynamicDataRead"), &mut out.limits);
    out.key_type = observed(
        scalar(settings, "KeyType").and_then(|value| {
            value
                .map(|raw| match raw.trim() {
                    value @ ("Auto" | "FieldValue" | "RowKey" | "RowNumber") => {
                        Ok(value.to_string())
                    }
                    _ => Err("DynamicList KeyType: unsupported key type".into()),
                })
                .transpose()
        }),
        &mut out.limits,
    );
    let fields = settings
        .children()
        .filter(|n| n.has_tag_name((FORM, "KeyField")))
        .map(text)
        .collect::<Read<Vec<_>>>();
    out.key_fields = observed(
        fields.map(|fields| (!fields.is_empty()).then_some(fields)),
        &mut out.limits,
    );
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(settings: &str) -> FormInfoDynamicList {
        let xml = format!(
            r#"<Attribute xmlns="{FORM}" xmlns:xsi="{XSI}" xmlns:f="{FORM}" xmlns:other="urn:other">{settings}</Attribute>"#
        );
        let doc = roxmltree::Document::parse(&xml).unwrap();
        read(doc.root_element())
    }

    #[test]
    fn settings_preserve_text_false_empty_and_order_without_defaults() {
        let list = parse(
            r#"<Settings xsi:type="f:DynamicList"><ManualQuery>0</ManualQuery><QueryText>  SELECT
A &amp; B<!-- split -->
  </QueryText><DynamicDataRead>1</DynamicDataRead><MainTable/><KeyType>Auto</KeyType><KeyField>Part</KeyField><KeyField>Code</KeyField><KeyField>Part</KeyField></Settings>"#,
        );
        assert_eq!(list.manual_query, Some(false));
        assert_eq!(list.dynamic_data_read, Some(true));
        assert_eq!(list.query_text.as_deref(), Some("  SELECT\nA & B\n  "));
        assert_eq!(list.main_table.as_deref(), Some(""));
        assert_eq!(list.key_fields_applicable, Some(true));
        assert_eq!(list.key_fields.unwrap(), ["Part", "Code", "Part"]);
        assert!(list.limits.is_empty());
        let empty = parse(r#"<Settings xsi:type="DynamicList"><QueryText/></Settings>"#);
        assert_eq!(empty.query_text.as_deref(), Some(""));
        assert!(empty.manual_query.is_none());
        assert!(empty.key_type.is_none());
        assert!(empty.key_fields.is_none());
    }

    #[test]
    fn settings_accept_all_documented_key_types_and_preserve_inapplicable_observations() {
        for key in ["Auto", "FieldValue", "RowKey", "RowNumber"] {
            let list = parse(&format!(
                r#"<Settings xsi:type="DynamicList"><MainTable>Catalog.Items</MainTable><KeyType>{key}</KeyType><KeyField>Code</KeyField></Settings>"#
            ));
            assert_eq!(list.key_type.as_deref(), Some(key));
            assert_eq!(list.key_fields_applicable, Some(false));
            assert_eq!(list.key_fields.unwrap(), ["Code"]);
        }
    }

    #[test]
    fn settings_damage_is_local_and_does_not_invent_values() {
        for bad in [
            "<ManualQuery>yes</ManualQuery>",
            "<ManualQuery>true</ManualQuery><ManualQuery>false</ManualQuery>",
            "<ManualQuery><Nested>true</Nested></ManualQuery>",
        ] {
            let list = parse(&format!(
                r#"<Settings xsi:type="DynamicList">{bad}<QueryText>SELECT 1</QueryText><KeyType>RowNumber</KeyType></Settings>"#
            ));
            assert_eq!(list.manual_query, None);
            assert_eq!(list.query_text.as_deref(), Some("SELECT 1"));
            assert_eq!(list.key_type.as_deref(), Some("RowNumber"));
            assert_eq!(list.limits.len(), 1);
            assert!(list.limits[0].contains("ManualQuery"));
        }
        for bad in [
            "<MainTable><Nested/></MainTable>",
            "<MainTable>A</MainTable><MainTable>B</MainTable>",
        ] {
            let list = parse(&format!(
                r#"<Settings xsi:type="DynamicList">{bad}<ManualQuery>false</ManualQuery></Settings>"#
            ));
            assert_eq!(list.main_table, None);
            assert_eq!(list.key_fields_applicable, None);
            assert_eq!(list.manual_query, Some(false));
            assert_eq!(list.limits.len(), 1);
        }
        let bad = parse(
            r#"<Settings xsi:type="DynamicList"><QueryText><Part>hidden</Part></QueryText><KeyType>FutureKey</KeyType><KeyField><Field>hidden</Field></KeyField><DynamicDataRead>false</DynamicDataRead></Settings>"#,
        );
        assert_eq!(bad.query_text, None);
        assert_eq!(bad.key_type, None);
        assert_eq!(bad.key_fields, None);
        assert_eq!(bad.dynamic_data_read, Some(false));
        assert_eq!(bad.limits.len(), 3);
    }

    #[test]
    fn settings_namespace_and_cardinality_are_checked() {
        let list = parse(
            r#"<other:Settings xsi:type="DynamicList"><ManualQuery>true</ManualQuery></other:Settings><Settings xsi:type="DynamicList"><other:ManualQuery>true</other:ManualQuery><other:KeyField>secret</other:KeyField></Settings>"#,
        );
        assert_eq!(list.manual_query, None);
        assert_eq!(list.key_fields, None);
        assert!(list.limits.is_empty());
        for settings in [
            r#"<Settings xsi:type="other:DynamicList"/>"#,
            r#"<Settings/>"#,
            r#"<Settings xsi:type="DynamicList"/><Settings xsi:type="DynamicList"/>"#,
        ] {
            let list = parse(settings);
            assert_eq!(list.key_fields_applicable, None);
            assert_eq!(list.manual_query, None);
            assert_eq!(list.limits.len(), 1);
        }
    }
}
