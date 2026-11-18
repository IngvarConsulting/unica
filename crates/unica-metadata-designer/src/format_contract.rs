use roxmltree::{Document, Node};

use unica_metadata::MetadataKind;

use crate::validation::{meta_validate_property_values, meta_validate_valid_types};

fn format_contract_object_node<'a>(document: &'a Document<'a>) -> Result<Node<'a, 'a>, String> {
    document
        .root_element()
        .children()
        .find(|node| {
            node.is_element() && meta_validate_valid_types().contains(&node.tag_name().name())
        })
        .ok_or_else(|| "metadata object node was not found".to_string())
}

/// Whether a kind declares a root `ChildObjects` container in the export
/// format the codec writes.
pub fn kind_declares_child_objects(kind: MetadataKind) -> bool {
    (crate::dialect::writable().kind_declares_child_objects)(kind)
}

/// The dialect that reads `document`, by the version its root records.
fn document_dialect(
    source: &str,
    document: &Document<'_>,
) -> &'static crate::dialect::MetadataDialect {
    crate::dialect::for_version(
        unica_source_designer::roots::root_version_literal(source, document.root_element())
            .as_deref(),
    )
}

fn validate_boolean_property_value(
    dialect: &crate::dialect::MetadataDialect,
    context: &str,
    object_type: &str,
    property_name: &str,
    value: &str,
) -> Result<(), String> {
    if !(dialect.boolean_properties)(object_type).contains(&property_name) {
        return Ok(());
    }
    if matches!(value, "true" | "false") {
        Ok(())
    } else {
        Err(format!(
            "{context} property {object_type}.{property_name} value '{value}' is not a canonical xs:boolean for the fixed {} contract; expected true or false",
            dialect.platform_line
        ))
    }
}

pub fn validate_metadata_boolean_contract(xml_text: &str, context: &str) -> Result<(), String> {
    let source = xml_text.trim_start_matches('\u{feff}');
    let document = Document::parse(source).map_err(|error| format!("XML parse error: {error}"))?;
    let dialect = document_dialect(source, &document);
    let root_object = format_contract_object_node(&document)?;
    if !meta_validate_valid_types().contains(&root_object.tag_name().name()) {
        return Ok(());
    }

    for object in root_object
        .descendants()
        .filter(roxmltree::Node::is_element)
    {
        let object_type = object.tag_name().name();
        let boolean_properties = (dialect.boolean_properties)(object_type);
        if boolean_properties.is_empty() {
            continue;
        }
        let Some(properties) = unica_xml_format::read::child_element(object, "Properties") else {
            continue;
        };
        for property in properties.children().filter(roxmltree::Node::is_element) {
            let property_name = property.tag_name().name();
            if boolean_properties.contains(&property_name) {
                let dimension_flag = object_type == "Dimension"
                    && matches!(
                        property_name,
                        "Master" | "MainFilter" | "DenyIncompleteValues"
                    );
                if dimension_flag {
                    if crate::info::dimension_boolean_value(property).is_none() {
                        return Err(format!(
                            "{context} property {object_type}.{property_name} is not a boolean text leaf"
                        ));
                    }
                } else {
                    validate_boolean_property_value(
                        dialect,
                        context,
                        object_type,
                        property_name,
                        property.text().unwrap_or(""),
                    )?;
                }
            }
        }
    }

    Ok(())
}

pub fn validate_metadata_enum_contract(xml_text: &str, context: &str) -> Result<(), String> {
    let source = xml_text.trim_start_matches('\u{feff}');
    let document = Document::parse(source).map_err(|error| format!("XML parse error: {error}"))?;
    let dialect = document_dialect(source, &document);
    let root_object = format_contract_object_node(&document)?;
    if !meta_validate_valid_types().contains(&root_object.tag_name().name()) {
        return Ok(());
    }

    for object in root_object
        .descendants()
        .filter(roxmltree::Node::is_element)
    {
        let Some(properties) = unica_xml_format::read::child_element(object, "Properties") else {
            continue;
        };
        for (property_name, allowed) in meta_validate_property_values() {
            let Some(value) = unica_xml_format::read::child_direct_text(properties, property_name)
                .filter(|value| !value.is_empty())
            else {
                continue;
            };
            if !allowed.contains(&value.as_str()) {
                return Err(format!(
                    "{context} property {}.{property_name} value '{value}' is not valid for the fixed {} contract; expected one of: {}",
                    object.tag_name().name(),
                    dialect.platform_line,
                    allowed.join(", ")
                ));
            }
        }
    }

    Ok(())
}

/// Observe the same XML root name used by the product's Configuration/Subsystem dispatcher.
pub fn metadata_owner_object_name(xml_text: &str) -> Result<String, String> {
    let document = Document::parse(xml_text.trim_start_matches('\u{feff}'))
        .map_err(|error| format!("XML parse error: {error}"))?;
    let root_object = crate::edit::meta_edit_object_node(&document)?;
    Ok(root_object.tag_name().name().to_string())
}
