//! Exact top-level attribute removal and the XML references that can prevent it.
use roxmltree::{Document, Node};
use unica_source_contract::proposal::ApplyPlanErrorKind;

use super::{
    is_opaque_form_binding, resolve_form_binding_path, strip_form_binding_prefixes,
    FormBindingPathResolution, FormBindingTablePath, FORM_BINDING_PATH_PROPERTIES,
};
use crate::root::{require_form_root, FORM_LOGFORM_NS as FORM};

const CORE: &str = "http://v8.1c.ru/8.1/data-composition-system/core";
const XSI: &str = "http://www.w3.org/2001/XMLSchema-instance";

type Result<T> = std::result::Result<T, RemovalError>;
#[derive(Debug)]
pub(crate) struct RemovalError {
    pub kind: ApplyPlanErrorKind,
    pub message: String,
}

fn error(kind: ApplyPlanErrorKind, message: impl Into<String>) -> RemovalError {
    RemovalError {
        kind,
        message: message.into(),
    }
}

fn invalid(message: impl Into<String>) -> RemovalError {
    error(ApplyPlanErrorKind::InvalidSource, message)
}

fn one<'a, 'i>(parent: Node<'a, 'i>, name: &str) -> Result<Option<Node<'a, 'i>>> {
    let mut nodes = parent.children().filter(|n| n.has_tag_name((FORM, name)));
    let value = nodes.next();
    if nodes.next().is_some() {
        return Err(invalid(format!("attribute.remove: duplicate {name}")));
    }
    Ok(value)
}

fn text(node: Node<'_, '_>) -> Result<String> {
    if node.children().any(|n| n.is_element()) {
        return Err(invalid(format!(
            "attribute.remove: {} is not scalar text",
            node.tag_name().name()
        )));
    }
    Ok(node
        .children()
        .filter(Node::is_text)
        .filter_map(|n| n.text())
        .collect())
}

fn qname_is(node: Node<'_, '_>, namespace: &str, local: &str) -> bool {
    node.attribute((XSI, "type")).is_some_and(|raw| {
        let (prefix, name) = raw
            .trim()
            .split_once(':')
            .map_or((None, raw.trim()), |(p, n)| (Some(p), n));
        name == local && node.lookup_namespace_uri(prefix) == Some(namespace)
    })
}

fn location(node: Node<'_, '_>) -> String {
    let property = node.tag_name().name();
    node.ancestors()
        .skip(1)
        .find_map(|n| {
            n.attribute("name")
                .map(|name| format!("{} `{name}`/{property}", n.tag_name().name()))
        })
        .unwrap_or_else(|| format!("Form/{property}"))
}

fn blocked(node: Node<'_, '_>, name: &str, value: &str) -> RemovalError {
    error(ApplyPlanErrorKind::BadValue, format!("attribute.remove: {} still references attribute `{name}` through `{value}`; remove the XML reference first", location(node)))
}

fn unsupported(node: Node<'_, '_>, value: &str) -> RemovalError {
    error(ApplyPlanErrorKind::InvalidState, format!("attribute.remove: cannot prove safe removal with binding `{value}` at {}; this binding representation is not resolved", location(node)))
}

fn check_path(root: Node<'_, '_>, node: Node<'_, '_>, name: &str, value: &str) -> Result<()> {
    let value = value.trim();
    if value.is_empty() {
        return Ok(());
    }
    if is_opaque_form_binding(&strip_form_binding_prefixes(value)) {
        return Err(unsupported(node, value));
    }
    let result = resolve_form_binding_path(value, |table_name| {
        let tables = root
            .children()
            .filter(|n| n.has_tag_name((FORM, "ChildItems")))
            .flat_map(|n| n.descendants())
            .filter(|n| n.has_tag_name((FORM, "Table")) && n.attribute("name") == Some(table_name))
            .collect::<Vec<_>>();
        let [table] = tables.as_slice() else {
            return FormBindingTablePath::Missing;
        };
        match one(*table, "DataPath").and_then(|n| n.map(text).transpose()) {
            Ok(Some(path))
                if !is_opaque_form_binding(&strip_form_binding_prefixes(path.trim())) =>
            {
                FormBindingTablePath::Bound(path)
            }
            _ => FormBindingTablePath::Missing,
        }
    });
    match result {
        FormBindingPathResolution::Attribute(attribute) if attribute == name => {
            Err(blocked(node, name, value))
        }
        FormBindingPathResolution::MissingTable(_)
        | FormBindingPathResolution::UnknownItemsShape => Err(unsupported(node, value)),
        _ => Ok(()),
    }
}

fn check_references(
    root: Node<'_, '_>,
    attributes: Node<'_, '_>,
    removed: Node<'_, '_>,
    name: &str,
) -> Result<()> {
    // All surviving live XML is checked, including borrowed items. BaseForm
    // is evidence of the baseline, not part of the edited tree.
    for node in root.descendants().filter(|n| n.is_element()) {
        if node
            .ancestors()
            .any(|ancestor| ancestor == removed || ancestor.has_tag_name((FORM, "BaseForm")))
        {
            continue;
        }
        if node.tag_name().namespace() == Some(FORM)
            && FORM_BINDING_PATH_PROPERTIES
                .iter()
                .any(|(_, tag)| *tag == node.tag_name().name())
        {
            check_path(root, node, name, &text(node)?)?;
        }
    }
    for property in ["ReportResult", "DetailsData", "VariantAppearance"] {
        if let Some(node) = one(root, property)? {
            let value = text(node)?;
            let value = value.trim();
            if !value.is_empty() && is_opaque_form_binding(&strip_form_binding_prefixes(value)) {
                return Err(unsupported(node, value));
            }
            if strip_form_binding_prefixes(value) == name {
                return Err(blocked(node, name, value));
            }
        }
    }
    if let Some(appearance) = one(attributes, "ConditionalAppearance")? {
        for node in appearance
            .descendants()
            .filter(|n| n.is_element() && qname_is(*n, CORE, "Field"))
        {
            check_path(root, node, name, &text(node)?)?;
        }
    }
    Ok(())
}

pub(crate) fn remove(xml: &str, name: &str) -> Result<String> {
    let doc = Document::parse(xml)
        .map_err(|e| invalid(format!("attribute.remove: invalid form XML: {e}")))?;
    let root = doc.root_element();
    require_form_root(root).map_err(invalid)?;
    let attributes = one(root, "Attributes")?.ok_or_else(|| {
        error(
            ApplyPlanErrorKind::NotFound,
            format!("form attribute `{name}` was not found"),
        )
    })?;
    let matching = attributes
        .children()
        .filter(|n| n.has_tag_name((FORM, "Attribute")) && n.attribute("name") == Some(name))
        .collect::<Vec<_>>();
    let attribute = match matching.as_slice() {
        [] => {
            return Err(error(
                ApplyPlanErrorKind::NotFound,
                format!("form attribute `{name}` was not found"),
            ))
        }
        [attribute] => *attribute,
        _ => {
            return Err(invalid(format!(
                "attribute.remove: form attribute `{name}` is ambiguous"
            )))
        }
    };
    let id = attribute
        .attribute("id")
        .and_then(|s| s.parse::<u64>().ok())
        .filter(|id| *id > 0)
        .ok_or_else(|| {
            invalid(format!(
                "attribute.remove: attribute `{name}` has no valid id"
            ))
        })?;
    if attributes
        .children()
        .filter(|n| {
            n.has_tag_name((FORM, "Attribute"))
                && n.attribute("id").and_then(|s| s.parse::<u64>().ok()) == Some(id)
        })
        .count()
        != 1
    {
        return Err(invalid(format!(
            "attribute.remove: attribute `{name}` has an ambiguous id"
        )));
    }
    if let Some(main) = one(attribute, "MainAttribute")? {
        match text(main)?.trim() {
            "true" | "1" => {
                return Err(error(
                    ApplyPlanErrorKind::BadValue,
                    format!("attribute.remove: main attribute `{name}` is protected"),
                ))
            }
            "false" | "0" => {}
            _ => {
                return Err(invalid(format!(
                    "attribute.remove: attribute `{name}` has an invalid MainAttribute"
                )))
            }
        }
    }
    if let Some(base) = one(root, "BaseForm")? {
        if id < 1_000_000
            || base.descendants().any(|n| {
                n.has_tag_name((FORM, "Attribute"))
                    && (n.attribute("name") == Some(name)
                        || n.attribute("id").and_then(|s| s.parse::<u64>().ok()) == Some(id))
            })
        {
            return Err(error(
                ApplyPlanErrorKind::BadValue,
                format!("attribute.remove: inherited attribute `{name}` is protected"),
            ));
        }
    }
    check_references(root, attributes, attribute, name)?;
    let mut result = xml.to_string();
    result.replace_range(attribute.range(), "");
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    const OPEN: &str = r#"<Form xmlns="http://v8.1c.ru/8.3/xcf/logform" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" xmlns:c="http://v8.1c.ru/8.1/data-composition-system/core" xmlns:s="http://v8.1c.ru/8.1/data-composition-system/settings" xmlns:xs="http://www.w3.org/2001/XMLSchema" version="2.20">"#;
    fn form(content: &str) -> String {
        format!("{OPEN}{content}</Form>")
    }
    fn attr(name: &str, id: &str, content: &str) -> String {
        format!(r#"<Attribute name="{name}" id="{id}">{content}</Attribute>"#)
    }
    fn fixture(extra: &str) -> String {
        form(&format!(
            "<Attributes>{}{}</Attributes>{extra}",
            attr("Unused", "2", ""),
            attr("UnusedExtra", "3", "")
        ))
    }
    fn failure(xml: &str, kind: ApplyPlanErrorKind, contains: &str) {
        let err = remove(xml, "Unused").expect_err("removal must refuse");
        assert_eq!(err.kind, kind, "{}", err.message);
        assert!(err.message.contains(contains), "{}", err.message);
    }

    #[test]
    fn attribute_removal_changes_only_the_exact_node_bytes() {
        for eol in ["\n", "\r\n"] {
            for bom in ["", "\u{feff}"] {
                let target = attr(
                    "Unused",
                    "2",
                    "<Title>removed</Title><Columns><Column name=\"Inner\" id=\"1\"/></Columns>",
                );
                let xml = format!("{bom}{OPEN}{eol}\t<Attributes>{eol}\t\t{target}{eol}\t\t{}{eol}\t</Attributes>{eol}</Form>",attr("UnusedExtra","3",""));
                assert_eq!(
                    remove(&xml, "Unused").unwrap(),
                    xml.replacen(&target, "", 1)
                );
            }
        }
    }

    #[test]
    fn attribute_removal_targets_only_live_top_level_names_and_exact_namespaces() {
        let xml = fixture("");
        assert_eq!(
            remove(&xml, " Unused").err().unwrap().kind,
            ApplyPlanErrorKind::NotFound
        );
        let removed = remove(&xml, "Unused").unwrap();
        assert_eq!(
            remove(&removed, "Unused").err().unwrap().kind,
            ApplyPlanErrorKind::NotFound
        );
        let xml = form(
            r#"<Attributes><Attribute name="Table" id="1"><Columns><Column name="Unused" id="2"/></Columns></Attribute></Attributes><BaseForm><Attributes><Attribute name="Unused" id="2"/></Attributes></BaseForm>"#,
        );
        failure(&xml, ApplyPlanErrorKind::NotFound, "not found");
        let xml = form(
            r#"<Attributes xmlns:o="urn:foreign"><o:Attribute name="Unused" id="2"/></Attributes>"#,
        );
        failure(&xml, ApplyPlanErrorKind::NotFound, "not found");
        let prefixed = fixture("")
            .replace(
                "<Attributes>",
                "<f:Attributes xmlns:f=\"http://v8.1c.ru/8.3/xcf/logform\">",
            )
            .replace("</Attributes>", "</f:Attributes>");
        assert!(remove(&prefixed, "Unused").is_ok());
    }

    #[test]
    fn attribute_removal_rejects_ambiguous_and_protected_attributes() {
        for extra in [attr("Unused", "3", ""), attr("Another", "2", "")] {
            failure(
                &form(&format!(
                    "<Attributes>{}{extra}</Attributes>",
                    attr("Unused", "2", "")
                )),
                ApplyPlanErrorKind::InvalidSource,
                "ambiguous",
            );
        }
        failure(
            &form("<Attributes/><Attributes/>"),
            ApplyPlanErrorKind::InvalidSource,
            "duplicate Attributes",
        );
        for value in ["true", "1"] {
            failure(
                &form(&format!(
                    "<Attributes>{}</Attributes>",
                    attr(
                        "Unused",
                        "2",
                        &format!("<MainAttribute>{value}</MainAttribute>")
                    )
                )),
                ApplyPlanErrorKind::BadValue,
                "main attribute",
            );
        }
        for main in [
            "<MainAttribute/>",
            "<MainAttribute>maybe</MainAttribute>",
            "<MainAttribute>false</MainAttribute><MainAttribute>false</MainAttribute>",
        ] {
            failure(
                &form(&format!(
                    "<Attributes>{}</Attributes>",
                    attr("Unused", "2", main)
                )),
                ApplyPlanErrorKind::InvalidSource,
                "MainAttribute",
            );
        }
        for id in ["", "x", "0"] {
            failure(
                &form(&format!(
                    "<Attributes>{}</Attributes>",
                    attr("Unused", id, "")
                )),
                ApplyPlanErrorKind::InvalidSource,
                "valid id",
            );
        }
    }

    #[test]
    fn attribute_removal_preserves_base_form_but_checks_all_surviving_live_items() {
        let baseline = "<BaseForm><Attributes><Attribute name=\"Borrowed\" id=\"1\"/></Attributes><ChildItems><InputField name=\"Old\" id=\"1\"><DataPath>Unused</DataPath></InputField></ChildItems></BaseForm>";
        let xml = form(&format!(
            "<Attributes>{}</Attributes>{baseline}",
            attr("Unused", "1000000", "")
        ));
        let result = remove(&xml, "Unused").unwrap();
        assert!(result.contains(baseline));
        for target in [attr("Unused", "2", ""), attr("Borrowed", "1000000", "")] {
            let xml = form(&format!("<Attributes>{target}</Attributes>{baseline}"));
            let name = if target.contains("Borrowed") {
                "Borrowed"
            } else {
                "Unused"
            };
            assert!(remove(&xml, name)
                .err()
                .unwrap()
                .message
                .contains("inherited"));
        }
        let xml = form(&format!("<Attributes>{}</Attributes>{baseline}<ChildItems><InputField name=\"LiveBorrowed\" id=\"2\"><DataPath>Unused</DataPath></InputField></ChildItems>",attr("Unused","1000000","")));
        failure(&xml, ApplyPlanErrorKind::BadValue, "LiveBorrowed");
    }

    #[test]
    fn attribute_removal_checks_binding_paths_and_report_attribute_slots() {
        for (_, tag) in FORM_BINDING_PATH_PROPERTIES {
            let xml = fixture(&format!("<ChildItems><InputField name=\"UsesAttribute\" id=\"1\"><{tag}>~Unused[0].Field</{tag}></InputField></ChildItems>"));
            failure(&xml, ApplyPlanErrorKind::BadValue, tag);
        }
        for tag in ["ReportResult", "DetailsData", "VariantAppearance"] {
            failure(
                &fixture(&format!("<{tag}>Unused</{tag}>")),
                ApplyPlanErrorKind::BadValue,
                tag,
            );
        }
        let xml = fixture("<ChildItems><Table name=\"Rows\" id=\"1\"><DataPath>Unused</DataPath></Table><InputField name=\"Current\" id=\"2\"><DataPath>Items.Rows.CurrentData.Field</DataPath></InputField></ChildItems>");
        failure(&xml, ApplyPlanErrorKind::BadValue, "still references");
        // The indirect resolver is used even when the table itself is not in
        // the surviving tree's first binding slot.
        let xml = fixture("<ChildItems><InputField name=\"Current\" id=\"2\"><DataPath>Items.Rows.CurrentData.Field</DataPath></InputField><Table name=\"Rows\" id=\"1\"><DataPath>Unused</DataPath></Table></ChildItems>");
        failure(&xml, ApplyPlanErrorKind::BadValue, "Current");
    }

    #[test]
    fn attribute_removal_distinguishes_typed_fields_from_literals_and_item_references() {
        let conditional = |value: &str| {
            form(&format!("<Attributes>{}<ConditionalAppearance><s:item><s:filter><s:left xsi:type=\"{value}\">Unused.Field</s:left></s:filter><s:selection><s:item><s:field>Unused</s:field></s:item></s:selection></s:item></ConditionalAppearance></Attributes>",attr("Unused","2","")))
        };
        failure(
            &conditional("c:Field"),
            ApplyPlanErrorKind::BadValue,
            "still references",
        );
        assert!(remove(&conditional("xs:string"), "Unused").is_ok());
        let xml = fixture("<Title>Unused</Title><GroupList>Unused</GroupList><CustomSettingsFolder>Unused</CustomSettingsFolder><SettingsStorage>Unused</SettingsStorage><ChildItems><InputField name=\"Unused\" id=\"1\"><DataPath>UnusedExtra</DataPath><Title>Unused</Title></InputField><SearchStringAddition name=\"Search\" id=\"2\"><AdditionSource><Item>Unused</Item></AdditionSource></SearchStringAddition></ChildItems><Commands><Command name=\"C\" id=\"1\"><AssociatedTableElementId>Unused</AssociatedTableElementId></Command></Commands>");
        assert!(remove(&xml, "Unused").is_ok());
        let xml = form(&format!(
            "<Attributes>{}{}</Attributes>",
            attr("Unused", "2", ""),
            attr(
                "Other",
                "3",
                "<Settings><Value xsi:type=\"c:Field\">Unused</Value></Settings>"
            )
        ));
        assert!(
            remove(&xml, "Unused").is_ok(),
            "query fields of another dynamic list are not form attributes"
        );
    }

    #[test]
    fn attribute_removal_refuses_unresolved_bindings_instead_of_claiming_safety() {
        for slot in ["ReportResult", "DetailsData", "VariantAppearance"] {
            for value in [
                "2",
                "~2",
                "2[1]",
                "~1/2:11111111-1111-4111-8111-111111111111",
            ] {
                failure(
                    &fixture(&format!("<{slot}>{value}</{slot}>")),
                    ApplyPlanErrorKind::InvalidState,
                    "cannot prove safe removal",
                );
            }
        }
        for value in [
            "2",
            "~2",
            "1/2:11111111-1111-4111-8111-111111111111",
            "Items.Missing.CurrentData.Code",
            "Items.Unknown",
        ] {
            failure(&fixture(&format!("<ChildItems><InputField name=\"Opaque\" id=\"1\"><DataPath>{value}</DataPath></InputField></ChildItems>")),ApplyPlanErrorKind::InvalidState,"cannot prove safe removal");
        }
    }
}
