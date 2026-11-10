//! Closed standard-list profile: the selected row has a statically known reference type.
use roxmltree::{Document, Node};

const FORM: &str = "http://v8.1c.ru/8.3/xcf/logform";
const CORE: &str = "http://v8.1c.ru/8.1/data/core";
const CONFIG: &str = "http://v8.1c.ru/8.1/data/enterprise/current-config";

fn one<'a, 'i>(parent: Node<'a, 'i>, ns: &str, name: &str) -> Result<Node<'a, 'i>, String> {
    let mut nodes = parent
        .children()
        .filter(|n| n.is_element() && n.tag_name().name() == name);
    let node = nodes
        .next()
        .ok_or_else(|| format!("standard choice form requires {name}"))?;
    if nodes.next().is_some() || !node.has_tag_name((ns, name)) {
        return Err(format!(
            "standard choice form requires one qualified {name}"
        ));
    }
    Ok(node)
}

fn scalar<'a, 'i>(node: Node<'a, 'i>) -> Result<&'a str, String> {
    if node.attributes().len() != 0 || node.children().any(|n| !n.is_text()) {
        return Err(format!("{} must be a scalar", node.tag_name().name()));
    }
    Ok(node.text().unwrap_or_default())
}

fn configured_type<'a, 'i>(parent: Node<'a, 'i>, ns: &str) -> Result<&'a str, String> {
    let ty = one(parent, ns, "Type")?;
    let node = one(ty, CORE, "Type")?;
    if ty.attributes().len() != 0 || ty.children().filter(|n| n.is_element()).count() != 1 {
        return Err("choiceForm requires a single reference type".into());
    }
    let text = scalar(node)?;
    qualified_name(node, text, CONFIG)
}

fn qualified_name<'a>(
    node: Node<'_, '_>,
    text: &'a str,
    namespace: &str,
) -> Result<&'a str, String> {
    let (prefix, name) = text
        .split_once(':')
        .map_or((None, text), |(prefix, name)| (Some(prefix), name));
    if node.lookup_namespace_uri(prefix) != Some(namespace) {
        return Err("unexpected type namespace in standard choice form".into());
    }
    Ok(name)
}

pub fn validate(payload: &[u8], table: &str, module: Option<&[u8]>) -> Result<(), String> {
    let payload = std::str::from_utf8(payload)
        .map_err(|e| e.to_string())?
        .trim_start_matches('\u{feff}');
    let doc = Document::parse(payload).map_err(|e| e.to_string())?;
    let root = doc.root_element();
    if !root.has_tag_name((FORM, "Form")) {
        return Err("expected managed form payload".into());
    }
    for node in root.descendants().filter(|n| n.is_element()) {
        match node.tag_name().name() {
            "Event" | "Action" | "Command" => return Err(
                "custom form handlers and commands are outside the standard choice form profile"
                    .into(),
            ),
            "CommandName" if !scalar(node)?.starts_with("Form.StandardCommand.") => {
                return Err(
                    "external form commands are outside the standard choice form profile".into(),
                )
            }
            "Query" | "QueryText" | "CustomQuery" if !scalar(node)?.trim().is_empty() => {
                return Err("custom queries are outside the standard choice form profile".into())
            }
            "MultipleChoice" if scalar(node)? != "false" => {
                return Err("multiple choice is outside the standard choice form profile".into())
            }
            _ => {}
        }
    }
    let attrs = one(root, FORM, "Attributes")?;
    let attr = one(attrs, FORM, "Attribute")?;
    if attrs.children().filter(|n| n.is_element()).count() != 1
        || scalar(one(attr, FORM, "MainAttribute")?)? != "true"
        || configured_type(attr, FORM)? != "DynamicList"
    {
        return Err("standard choice form requires one main DynamicList attribute".into());
    }
    let settings = one(attr, FORM, "Settings")?;
    let xsi_type = settings.attribute(("http://www.w3.org/2001/XMLSchema-instance", "type"));
    if qualified_name(settings, xsi_type.unwrap_or_default(), FORM)? != "DynamicList"
        || scalar(one(settings, FORM, "ManualQuery")?)? != "false"
    {
        return Err("standard choice form requires a non-custom DynamicList".into());
    }
    let main_table = scalar(one(settings, FORM, "MainTable")?)?;
    if !unica_metadata::metadata_name_eq(main_table, table) {
        return Err(
            "standard choice form main table does not match the field reference type".into(),
        );
    }
    let tables = root
        .descendants()
        .filter(|n| n.is_element() && n.tag_name().name() == "Table")
        .collect::<Vec<_>>();
    let [list] = tables.as_slice() else {
        return Err("standard choice form requires one list table".into());
    };
    if !list.has_tag_name((FORM, "Table"))
        || Some(scalar(one(*list, FORM, "DataPath")?)?) != attr.attribute("name")
    {
        return Err("standard choice form table must use its main DynamicList".into());
    }
    if scalar(one(*list, FORM, "ChoiceMode")?)? != "true" {
        return Err("standard choice form table requires ChoiceMode=true".into());
    }
    if let Some(module) = module {
        let text = std::str::from_utf8(module).map_err(|e| e.to_string())?;
        if text.contains(['\u{2028}', '\u{2029}'])
            || text
                .chars()
                .any(|c| c.is_control() && !matches!(c, '\r' | '\n' | '\t'))
        {
            return Err("unsupported line separator in choice form BSL".into());
        }
        for line in text.trim_start_matches('\u{feff}').split(['\r', '\n']) {
            let line = line.trim();
            let region = ["#Область", "#Region"].iter().any(|prefix| {
                line.strip_prefix(prefix)
                    .is_some_and(|tail| tail.starts_with(char::is_whitespace))
            });
            if !(line.is_empty()
                || line.starts_with("//")
                || region
                || ["#КонецОбласти", "#EndRegion"].contains(&line))
            {
                return Err("executable BSL is outside the standard choice form profile".into());
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    const LIST: &str = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../tests/fixtures/acceptance/workspace-member-choice/src/Catalogs/Валюты/Forms/ФормаСписка/Ext/Form.xml"));

    #[test]
    fn standard_list_profile_proves_row_type_and_rejects_custom_selection_routes() {
        validate(LIST.as_bytes(), "Catalog.Валюты", None).unwrap();
        validate(
            LIST.as_bytes(),
            "Catalog.Валюты",
            Some("// comment\n#Область Helpers\n#КонецОбласти\n".as_bytes()),
        )
        .unwrap();
        assert!(validate(LIST.as_bytes(), "Document.Приход", None).is_err());
        assert!(validate(
            LIST.as_bytes(),
            "Catalog.Валюты",
            Some(b"NotifyChoice(Other());")
        )
        .is_err());
        assert!(validate(
            LIST.as_bytes(),
            "Catalog.Валюты",
            Some(b"// comment\rNotifyChoice(Other());")
        )
        .is_err());
        for payload in [
            LIST.replace("<ChoiceMode>true</ChoiceMode>", ""),
            LIST.replace(
                "<ChoiceMode>true</ChoiceMode>",
                "<ChoiceMode>false</ChoiceMode>",
            ),
            LIST.replace(
                "<ManualQuery>false</ManualQuery>",
                "<ManualQuery>true</ManualQuery>",
            ),
            LIST.replace(
                "</Settings>",
                "<Query>SELECT Ref FROM Catalog.Codes</Query></Settings>",
            ),
            LIST.replace(
                "</Settings>",
                "<MainTable>Catalog.Codes</MainTable></Settings>",
            ),
            LIST.replace("<MainTable>", "<MainTable xmlns=\"foreign\">"),
            LIST.replace(
                "</Form>",
                "<Events><Event name=\"ChoiceProcessing\">ChooseOther</Event></Events></Form>",
            ),
            LIST.replace(
                "</Form>",
                "<CommandName>CommonCommand.ChooseOther</CommandName></Form>",
            ),
            LIST.replace(
                "</Attributes>",
                "<Attribute name=\"Other\" id=\"2\"/></Attributes>",
            ),
            LIST.replace(
                "</Form>",
                "<Table name=\"Other\" id=\"100\"><DataPath>Other</DataPath></Table></Form>",
            ),
            LIST.replace("<DataPath>List</DataPath>", "<DataPath>Other</DataPath>"),
            LIST.replace(
                "<MainAttribute>true</MainAttribute>",
                "<MainAttribute>true</MainAttribute><MainAttribute>true</MainAttribute>",
            ),
            LIST.replace(
                "<Settings xsi:type=\"DynamicList\">",
                "<Settings xmlns=\"foreign\" xsi:type=\"DynamicList\">",
            ),
        ] {
            assert!(
                validate(payload.as_bytes(), "Catalog.Валюты", None).is_err(),
                "accepted {payload}"
            );
        }
    }
}
