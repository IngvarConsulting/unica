//! Closed command facts; malformed properties retain independent observations.
use super::*;

const XR_NS: &str = "http://v8.1c.ru/8.3/xcf/readable";
const XSI_NS: &str = "http://www.w3.org/2001/XMLSchema-instance";
const XS_NS: &str = "http://www.w3.org/2001/XMLSchema";

fn damage(limits: &mut Vec<String>, name: &str) {
    let message = format!("{name} is unreadable: malformed or duplicate command property");
    if !limits.contains(&message) {
        limits.push(message);
    }
}

fn property<'a>(
    node: roxmltree::Node<'a, 'a>,
    name: &str,
    ns: &str,
    limits: &mut Vec<String>,
) -> Option<roxmltree::Node<'a, 'a>> {
    let values = node
        .children()
        .filter(|child| child.is_element() && child.tag_name().name() == name)
        .collect::<Vec<_>>();
    match values.as_slice() {
        [] => None,
        [value] if value.tag_name().namespace() == Some(ns) => Some(*value),
        _ => {
            damage(limits, name);
            None
        }
    }
}

fn text(
    node: roxmltree::Node<'_, '_>,
    name: &str,
    ns: &str,
    limits: &mut Vec<String>,
) -> Option<String> {
    let value = property(node, name, ns, limits)?;
    let attributes_valid = value.attributes().all(|attribute| {
        if name != "AssociatedTableElementId"
            || attribute.namespace() != Some(XSI_NS)
            || attribute.name() != "type"
        {
            return false;
        }
        let Some((prefix, local)) = attribute.value().split_once(':') else {
            return false;
        };
        local == "string" && value.lookup_namespace_uri(Some(prefix)) == Some(XS_NS)
    });
    if !attributes_valid || value.children().any(|child| child.is_element()) {
        damage(limits, name);
        return None;
    }
    Some(
        unica_xml_format::read::direct_text(value)
            .trim()
            .to_string(),
    )
}

fn flag(
    node: roxmltree::Node<'_, '_>,
    name: &str,
    ns: &str,
    limits: &mut Vec<String>,
) -> Option<bool> {
    match text(node, name, ns, limits)?.as_str() {
        "true" | "1" => Some(true),
        "false" | "0" => Some(false),
        _ => {
            damage(limits, name);
            None
        }
    }
}

fn options(node: roxmltree::Node<'_, '_>, limits: &mut Vec<String>) -> Option<Vec<String>> {
    let container = property(node, "FunctionalOptions", FORM_LOGFORM_NS, limits)?;
    if container.attributes().len() != 0
        || !unica_xml_format::read::direct_text(container)
            .trim()
            .is_empty()
    {
        damage(limits, "FunctionalOptions");
        return None;
    }
    let mut references = Vec::new();
    for item in container.children().filter(|child| child.is_element()) {
        let value = unica_xml_format::read::direct_text(item);
        let value = value.trim();
        if !item.has_tag_name((FORM_LOGFORM_NS, "Item"))
            || item.attributes().len() != 0
            || item.children().any(|child| child.is_element())
            || !value
                .strip_prefix("FunctionalOption.")
                .is_some_and(unica_metadata::identifier::is_1c_identifier)
        {
            damage(limits, "FunctionalOptions");
        } else {
            references.push(value.to_string());
        }
    }
    Some(references)
}

pub(super) fn read(command: roxmltree::Node<'_, '_>) -> FormInfoCommand {
    let mut limits = Vec::new();
    let current_row_use =
        text(command, "CurrentRowUse", FORM_LOGFORM_NS, &mut limits).and_then(|value| {
            if matches!(value.as_str(), "Auto" | "DontUse" | "Use") {
                Some(value)
            } else {
                damage(&mut limits, "CurrentRowUse");
                None
            }
        });
    let modifies_saved_data = flag(command, "ModifiesSavedData", FORM_LOGFORM_NS, &mut limits);
    let associated_table_element_id = text(
        command,
        "AssociatedTableElementId",
        FORM_LOGFORM_NS,
        &mut limits,
    );
    let functional_options = options(command, &mut limits);
    let picture = property(command, "Picture", FORM_LOGFORM_NS, &mut limits).filter(|picture| {
        if picture.attributes().len() != 0
            || !unica_xml_format::read::direct_text(*picture)
                .trim()
                .is_empty()
        {
            damage(&mut limits, "Picture");
            false
        } else {
            true
        }
    });
    let picture_ref = picture
        .and_then(|picture| text(picture, "Ref", XR_NS, &mut limits))
        .filter(|value| !value.is_empty());
    let picture_load_transparent =
        picture.and_then(|picture| flag(picture, "LoadTransparent", XR_NS, &mut limits));
    FormInfoCommand {
        name: command.attribute("name").unwrap_or("").to_string(),
        actions: form_children(command, "Action")
            .into_iter()
            .map(|action| FormInfoEvent {
                name: "Execute".to_string(),
                handler: action.text().unwrap_or("").to_string(),
                call_type: action.attribute("callType").map(str::to_string),
            })
            .collect(),
        title: form_child_text(command, "Title"),
        tooltip: form_child_text(command, "ToolTip"),
        shortcut: form_child_text(command, "Shortcut"),
        picture: picture_ref,
        representation: form_child_text(command, "Representation"),
        current_row_use,
        modifies_saved_data,
        associated_table_element_id,
        functional_options,
        picture_load_transparent,
        property_limits: limits,
    }
}
