//! Exact direct-child XML observations for form event context.
use crate::root::FORM_LOGFORM_NS;
use roxmltree::Node;
use unica_forms::events::*;
const FORM_V8_NS: &str = "http://v8.1c.ru/8.1/data/core";
pub fn context_from_root(root: Node<'_, '_>) -> FormEventContext {
    let base_form = direct_logform_child(root, "BaseForm");
    let definition = if base_form.is_some() {
        FormDefinitionKind::Extension
    } else {
        FormDefinitionKind::Regular
    };
    let direct_part_writable = definition == FormDefinitionKind::Regular
        || root.children().any(|child| {
            child.is_element()
                && child.tag_name().namespace() == Some(FORM_LOGFORM_NS)
                && child.tag_name().name() != "BaseForm"
        });
    let form_main_attribute = direct_main_attribute(root);
    let base_main_attribute = base_form.and_then(direct_main_attribute);
    let (main_attribute_type, main_attribute_name, main_attribute_provenance) =
        if let Some(main_attribute) = form_main_attribute {
            (
                main_attribute_type(main_attribute),
                main_attribute.attribute("name").map(str::to_string),
                MainAttributeProvenance::DirectForm,
            )
        } else if let Some(main_attribute) = base_main_attribute {
            (
                main_attribute_type(main_attribute),
                main_attribute.attribute("name").map(str::to_string),
                MainAttributeProvenance::DirectBaseForm,
            )
        } else if definition == FormDefinitionKind::Extension {
            (
                None,
                None,
                MainAttributeProvenance::InheritedBaseFormUnavailable,
            )
        } else {
            (None, None, MainAttributeProvenance::Missing)
        };
    let main_attribute = main_attribute_type
        .as_deref()
        .map(MainAttributeKind::from_type_name)
        .unwrap_or(MainAttributeKind::Unknown);

    FormEventContext {
        definition,
        direct_part_writable,
        main_attribute,
        main_attribute_type,
        main_attribute_provenance,
        main_attribute_name,
        metadata_owner: None,
    }
}

fn direct_logform_child<'a, 'input>(
    node: Node<'a, 'input>,
    local_name: &str,
) -> Option<Node<'a, 'input>> {
    node.children().find(|child| {
        child.is_element()
            && child.tag_name().name() == local_name
            && child.tag_name().namespace() == Some(FORM_LOGFORM_NS)
    })
}

fn direct_main_attribute<'a, 'input>(container: Node<'a, 'input>) -> Option<Node<'a, 'input>> {
    let attributes = direct_logform_child(container, "Attributes")?;
    attributes.children().find(|attribute| {
        attribute.is_element()
            && attribute.tag_name().name() == "Attribute"
            && attribute.tag_name().namespace() == Some(FORM_LOGFORM_NS)
            && direct_logform_child(*attribute, "MainAttribute")
                .and_then(|flag| flag.text())
                .is_some_and(|flag| flag.trim() == "true")
    })
}

fn main_attribute_type(main_attribute: Node<'_, '_>) -> Option<String> {
    let type_node = direct_logform_child(main_attribute, "Type")?;

    let v8_type_nodes = type_node
        .descendants()
        .skip(1)
        .filter(|node| {
            node.is_element()
                && node.tag_name().name() == "Type"
                && node.tag_name().namespace() == Some(FORM_V8_NS)
        })
        .collect::<Vec<_>>();
    if !v8_type_nodes.is_empty() {
        let v8_types = v8_type_nodes
            .into_iter()
            .map(trimmed_text)
            .collect::<Option<Vec<_>>>()?;
        return Some(v8_types.join("|"));
    }

    type_node
        .children()
        .filter(|node| node.is_text())
        .find_map(trimmed_text)
}

fn trimmed_text(node: Node<'_, '_>) -> Option<String> {
    node.text()
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(ToOwned::to_owned)
}
