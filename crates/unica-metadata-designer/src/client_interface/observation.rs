//! Read explicit instances and definitions without interpreting standard panels.
use super::{named, CF_CAI_NS};
use std::collections::BTreeSet;
use unica_metadata::configuration::{
    cf_validate_guid, CfPanelArea, CfPanelDefinition, CfPanelItem, CfPanelLayoutData,
};

const XSI: &str = "http://www.w3.org/2001/XMLSchema-instance";

fn attributes(
    node: roxmltree::Node<'_, '_>,
    allowed: &[(&str, Option<&str>)],
) -> Result<(), String> {
    if let Some(attribute) = node
        .attributes()
        .find(|attribute| !allowed.contains(&(attribute.name(), attribute.namespace())))
    {
        return Err(format!(
            "unsupported attribute {} on {}",
            attribute.name(),
            node.tag_name().name()
        ));
    }
    Ok(())
}

fn whitespace(node: roxmltree::Node<'_, '_>) -> Result<(), String> {
    if node
        .children()
        .any(|child| child.is_text() && child.text().is_some_and(|text| !text.trim().is_empty()))
    {
        return Err(format!("unexpected text in {}", node.tag_name().name()));
    }
    Ok(())
}

fn identifier(
    node: roxmltree::Node<'_, '_>,
    required: bool,
    ids: &mut BTreeSet<String>,
) -> Result<Option<String>, String> {
    let Some(id) = node.attribute("id") else {
        return if required {
            Err(format!("{} has no id", node.tag_name().name()))
        } else {
            Ok(None)
        };
    };
    if !cf_validate_guid(id) {
        return Err(format!("invalid {} id", node.tag_name().name()));
    }
    if !ids.insert(id.to_ascii_lowercase()) {
        return Err(format!("duplicate {} id {id}", node.tag_name().name()));
    }
    Ok(Some(id.to_string()))
}

fn area(node: roxmltree::Node<'_, '_>, ids: &mut BTreeSet<String>) -> Result<CfPanelArea, String> {
    attributes(node, &[])?;
    whitespace(node)?;
    let mut items = Vec::new();
    let mut pending: Vec<_> = node
        .children()
        .rev()
        .filter(roxmltree::Node::is_element)
        .map(|child| (child, None))
        .collect();
    while let Some((node, parent)) = pending.pop() {
        attributes(node, &[("id", None)])?;
        whitespace(node)?;
        if named(node, "group") {
            let id = identifier(node, false, ids)?;
            let index = items.len();
            items.push(CfPanelItem::Group { id, parent });
            pending.extend(
                node.children()
                    .rev()
                    .filter(roxmltree::Node::is_element)
                    .map(|child| (child, Some(index))),
            );
        } else if named(node, "panel") {
            let id = identifier(node, true, ids)?.expect("required panel identifier");
            let children: Vec<_> = node
                .children()
                .filter(roxmltree::Node::is_element)
                .collect();
            let [uuid] = children.as_slice() else {
                return Err("panel must contain exactly one uuid".into());
            };
            if !named(*uuid, "uuid") || uuid.children().any(|child| child.is_element()) {
                return Err("panel must contain one scalar uuid".into());
            }
            attributes(*uuid, &[])?;
            let text: String = uuid
                .children()
                .filter(roxmltree::Node::is_text)
                .filter_map(|node| node.text())
                .collect();
            let definition_id = text.trim();
            if !cf_validate_guid(definition_id) {
                return Err("invalid panel definition reference".into());
            }
            items.push(CfPanelItem::Panel {
                id,
                definition_id: definition_id.to_string(),
                resolved: false,
                parent,
            });
        } else {
            return Err(format!(
                "unsupported element {} in a panel area",
                node.tag_name().name()
            ));
        }
    }
    Ok(CfPanelArea {
        side: node.tag_name().name().to_string(),
        items,
    })
}

fn profile(root: roxmltree::Node<'_, '_>) -> Result<CfPanelLayoutData, String> {
    attributes(root, &[("type", Some(XSI))])?;
    whitespace(root)?;
    if let Some(value) = root.attribute((XSI, "type")) {
        let (prefix, local) = value
            .split_once(':')
            .map_or((None, value), |(prefix, local)| (Some(prefix), local));
        if local != "InterfaceLayouter" || root.lookup_namespace_uri(prefix) != Some(CF_CAI_NS) {
            return Err("unsupported client application interface type".into());
        }
    }
    let mut areas = Vec::new();
    let mut definitions = Vec::new();
    let mut instance_ids = BTreeSet::new();
    let mut definition_ids = BTreeSet::new();
    for child in root.children().filter(roxmltree::Node::is_element) {
        if super::SIDES.iter().any(|side| named(child, side)) {
            areas.push(area(child, &mut instance_ids)?);
        } else if named(child, "panelDef") {
            attributes(child, &[("id", None)])?;
            whitespace(child)?;
            if child.children().any(|node| node.is_element()) {
                return Err("non-empty panel definition is outside the read profile".into());
            }
            let id = identifier(child, true, &mut definition_ids)?
                .expect("required definition identifier");
            definitions.push(CfPanelDefinition { id });
        } else {
            return Err(format!(
                "unsupported element {} in client application interface",
                child.tag_name().name()
            ));
        }
    }
    let mut limits = Vec::new();
    for area in &mut areas {
        for item in &mut area.items {
            if let CfPanelItem::Panel {
                definition_id,
                resolved,
                ..
            } = item
            {
                *resolved = definition_ids.contains(&definition_id.to_ascii_lowercase());
                if !*resolved {
                    limits.push(format!("panel definition {definition_id} is not declared"));
                }
            }
        }
    }
    if limits.is_empty() {
        Ok(CfPanelLayoutData::Complete {
            areas,
            definitions,
            limits,
        })
    } else {
        Ok(CfPanelLayoutData::Limited {
            areas,
            definitions,
            limits,
        })
    }
}

/// Malformed XML or a foreign document is unreadable. Unsupported profile
/// constructs remain explicit observations rather than an empty layout.
pub fn parse_panel_layout_observation(text: &str) -> Result<CfPanelLayoutData, String> {
    let document = roxmltree::Document::parse(text.trim_start_matches('\u{feff}'))
        .map_err(|error| format!("ClientApplicationInterface.xml is malformed: {error}"))?;
    let root = document.root_element();
    if !named(root, "ClientApplicationInterface") {
        return Err("ClientApplicationInterface.xml has a wrong root or namespace".into());
    }
    Ok(
        profile(root).unwrap_or_else(|reason| CfPanelLayoutData::Unsupported {
            limits: vec![reason],
        }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};

    const DEFINITION: &str = "cf162400-0000-4000-8000-000000000001";
    fn document(body: &str) -> String {
        format!(
            r#"<ClientApplicationInterface xmlns="{CF_CAI_NS}" xmlns:xsi="{XSI}" xsi:type="InterfaceLayouter">{body}</ClientApplicationInterface>"#
        )
    }
    fn observed(body: &str) -> Value {
        serde_json::to_value(parse_panel_layout_observation(&document(body)).unwrap()).unwrap()
    }
    fn panel(id: &str, definition: &str) -> String {
        format!("<panel id=\"{id}\"><uuid>{definition}</uuid></panel>")
    }

    #[test]
    fn definitions_resolve_by_identity_and_repeated_placements_keep_distinct_instances() {
        let second = "CF162400-0000-4000-8000-000000000002";
        let value = observed(&format!("<top>{}<group>{}</group><group/></top><top><group>{}</group></top><left/><panelDef id=\"{DEFINITION}\"/>", panel(second, &DEFINITION.to_ascii_uppercase()), panel("cf162400-0000-4000-8000-000000000003", DEFINITION), panel("cf162400-0000-4000-8000-000000000004", "cf162400-0000-4000-8000-000000000005")));
        assert_eq!(value["state"], "limited");
        assert_eq!(
            value["areas"][0]["items"],
            json!([
                {"kind":"panel","id":second,"definitionId":DEFINITION.to_ascii_uppercase(),"resolved":true,"parent":null},
                {"kind":"group","parent":null},
                {"kind":"panel","id":"cf162400-0000-4000-8000-000000000003","definitionId":DEFINITION,"resolved":true,"parent":1},
                {"kind":"group","parent":null}
            ])
        );
        assert_eq!(value["areas"][1]["items"][0]["parent"], Value::Null);
        assert_eq!(value["areas"][1]["items"][1]["parent"], 0);
        assert_eq!(value["areas"][1]["items"][1]["resolved"], false);
        assert_eq!(value["areas"][2], json!({"side":"left","items":[]}));
        assert_eq!(value["definitions"], json!([{"id":DEFINITION}]));
        assert_eq!(value["limits"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn unsupported_profile_does_not_masquerade_as_an_empty_complete_layout() {
        for body in [
            "<unknown/>", "text", "<top>text</top>", "<top other=\"x\"/>",
            "<top><foreign:group xmlns:foreign=\"urn:foreign\"/></top>",
            "<top><group id=\"bad\"/></top>", "<top><panel/></top>",
            "<top><panel id=\"cf162400-0000-4000-8000-000000000003\"><uuid>bad</uuid></panel></top>",
            "<panelDef/>", "<panelDef id=\"cf162400-0000-4000-8000-000000000001\"><content/></panelDef>",
        ] {
            let value = observed(body);
            assert_eq!(value["state"], "unsupported", "{body}: {value}");
            assert!(value.get("areas").is_none(), "{body}");
            assert!(value.get("definitions").is_none(), "{body}");
            assert!(!value["limits"].as_array().unwrap().is_empty());
        }
        for xml in [
            document("").replace("InterfaceLayouter", "Unknown"),
            document("").replace("xsi:type=", "other="),
        ] {
            let value =
                serde_json::to_value(parse_panel_layout_observation(&xml).unwrap()).unwrap();
            assert_eq!(value["state"], "unsupported");
        }
    }

    #[test]
    fn malformed_documents_and_foreign_roots_remain_unreadable() {
        for xml in [
            "",
            "<bad",
            "<Other/>",
            "<ClientApplicationInterface xmlns=\"urn:other\"/>",
        ] {
            assert!(parse_panel_layout_observation(xml).is_err(), "{xml}");
        }
        assert_eq!(
            observed(""),
            json!({"state":"complete","areas":[],"definitions":[],"limits":[]})
        );
    }

    #[test]
    fn duplicate_ids_and_duplicate_or_structured_uuid_are_explicitly_unsupported() {
        for body in [
            format!("<panelDef id=\"{DEFINITION}\"/><panelDef id=\"{}\"/>", DEFINITION.to_ascii_uppercase()),
            format!("<top>{}{}</top>", panel(DEFINITION, DEFINITION), panel(DEFINITION, DEFINITION)),
            format!("<top><group id=\"{DEFINITION}\">{}</group></top>", panel(DEFINITION, DEFINITION)),
            format!("<top><panel id=\"{DEFINITION}\"><uuid>{DEFINITION}</uuid><uuid>{DEFINITION}</uuid></panel></top>"),
            format!("<top><panel id=\"{DEFINITION}\"><uuid><nested>{DEFINITION}</nested></uuid></panel></top>"),
            format!("<top><panel id=\"{DEFINITION}\"><uuid>{DEFINITION}<!-- split -->junk</uuid></panel></top>"),
        ] {
            assert_eq!(observed(&body)["state"], "unsupported", "{body}");
        }
        // Definition and instance identities have separate roles, even if
        // their literal identifiers happen to match.
        assert_eq!(
            observed(&format!(
                "<top>{}</top><panelDef id=\"{DEFINITION}\"/>",
                panel(DEFINITION, DEFINITION)
            ))["state"],
            "complete"
        );
    }

    #[test]
    fn the_existing_writer_can_repeat_areas_without_losing_its_topology() {
        use super::super::{render_panels, Panel, PanelEntry, PanelLayout};
        let layout = PanelLayout {
            sides: [
                vec![
                    PanelEntry::Panel(Panel::Open),
                    PanelEntry::Group(vec![Panel::Favorites, Panel::History]),
                ],
                vec![],
                vec![],
                vec![],
            ],
        };
        let xml = render_panels(&layout, None);
        let value = serde_json::to_value(parse_panel_layout_observation(&xml).unwrap()).unwrap();
        assert_eq!(value["state"], "complete");
        assert_eq!(value["areas"].as_array().unwrap().len(), 2);
        assert_eq!(value["areas"][0]["side"], "top");
        assert_eq!(value["areas"][1]["side"], "top");
        assert_eq!(value["areas"][0]["items"][0]["parent"], Value::Null);
        let items = value["areas"][1]["items"].as_array().unwrap();
        assert_eq!(items[0]["kind"], "group");
        assert_eq!(items[1]["parent"], 0);
        assert_eq!(items[2]["parent"], 1);
        assert_eq!(items[3]["parent"], 0);
        assert_eq!(items[4]["parent"], 3);
    }
}
