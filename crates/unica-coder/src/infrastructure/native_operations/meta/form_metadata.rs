use super::info_projection::{
    direct_md_child, meta_info_property_value_is_valid, parsed_localized_meta_info_property_value,
};
use super::validation_context::inspect_form_metadata_image_identity;
use super::xml_model::{meta_info_inner_text, parse_metadata_image, MD_CLASSES_NS};
use crate::domain::metadata::MetaInfoPropertyValueKind;
use crate::domain::source_target::MetadataAddress;
use serde_json::{json, Map, Value};
use std::collections::HashSet;

/// Свойства локального описания формы. Файл содержимого здесь не читается;
/// общий разбор XML, identity и значений совпадает с читателем метаданных.
pub(crate) fn parse_form_metadata_properties(
    bytes: &[u8],
    target: &MetadataAddress,
) -> Result<(Map<String, Value>, Vec<String>), String> {
    let identity =
        inspect_form_metadata_image_identity(bytes).map_err(|error| error.to_string())?;
    let parts = target.as_str().split('.').collect::<Vec<_>>();
    let expected = &parts[parts.len().saturating_sub(2)..];
    if expected != [identity.object_type.as_str(), identity.object_name.as_str()] {
        return Err(format!(
            "form metadata descriptor identity differs from `{target}`"
        ));
    }
    let (_, document) = parse_metadata_image(bytes)?;
    let artifact = document
        .root_element()
        .children()
        .find(roxmltree::Node::is_element)
        .expect("the shared identity reader proved a unique artifact");
    let properties = direct_md_child(artifact, "Properties")
        .expect("the shared identity reader proved one Properties");
    let mut result = Map::new();
    result.insert("uuid".into(), json!(identity.object_uuid.to_string()));
    let mut seen = HashSet::new();
    let mut limits = Vec::new();
    for property in properties.children().filter(roxmltree::Node::is_element) {
        let name = property.tag_name().name();
        if property.tag_name().namespace() != Some(MD_CLASSES_NS) || !seen.insert(name) {
            return Err(format!(
                "form metadata property `{name}` is foreign or duplicated"
            ));
        }
        match observe_form_property(property) {
            Ok(Some(value)) => {
                result.insert(name.to_string(), value);
            }
            Ok(None) => {}
            Err(error) => limits.push(format!("properties.{name}: {error}")),
        }
    }
    Ok((result, limits))
}

fn direct_text_is_formatting(node: roxmltree::Node<'_, '_>) -> bool {
    node.children()
        .filter(roxmltree::Node::is_text)
        .all(|child| child.text().is_none_or(|text| text.trim().is_empty()))
}

fn observe_form_property(property: roxmltree::Node<'_, '_>) -> Result<Option<Value>, String> {
    let name = property.tag_name().name();
    let value = match name {
        "Synonym" | "ExtendedPresentation" | "Explanation" => {
            if !direct_text_is_formatting(property)
                || !meta_info_property_value_is_valid(
                    property,
                    MetaInfoPropertyValueKind::LocalizedString,
                )
            {
                return Err("malformed localized text".into());
            }
            serde_json::to_value(parsed_localized_meta_info_property_value(property))
                .map_err(|error| error.to_string())?
        }
        "UsePurposes" => {
            if property.attributes().len() != 0 || !direct_text_is_formatting(property) {
                return Err("UsePurposes contains attributes or direct non-whitespace text".into());
            }
            let mut purposes = Vec::new();
            for value in property.children().filter(roxmltree::Node::is_element) {
                if value.tag_name().namespace() != Some("http://v8.1c.ru/8.1/data/core")
                    || value.tag_name().name() != "Value"
                    || value.children().any(|child| child.is_element())
                {
                    return Err("UsePurposes contains an invalid value".into());
                }
                purposes.push(meta_info_inner_text(value));
            }
            json!(purposes)
        }
        "UseStandardCommands" | "IncludeHelpInContents" => {
            if !meta_info_property_value_is_valid(property, MetaInfoPropertyValueKind::Boolean) {
                return Err("value is not a boolean".into());
            }
            json!(meta_info_inner_text(property) == "true")
        }
        "Name" | "Comment" | "FormType" => {
            if !meta_info_property_value_is_valid(property, MetaInfoPropertyValueKind::String) {
                return Err("value is not a scalar string".into());
            }
            json!(meta_info_inner_text(property))
        }
        // Their typed meaning is added by the shared borrowing reader.
        "ObjectBelonging" | "ExtendedConfigurationObject" => return Ok(None),
        _ => return Err("form metadata property has no supported projection".into()),
    };
    Ok(Some(value))
}

#[cfg(test)]
mod tests {
    use super::parse_form_metadata_properties;
    use crate::domain::source_target::{MetadataAddress, PLATFORM_XML_8_3_27_FORMAT_2_20};
    use serde_json::json;

    #[test]
    fn form_use_purposes_preserves_absence_empty_and_malformed_observations() {
        let target =
            MetadataAddress::parse(PLATFORM_XML_8_3_27_FORMAT_2_20, "CommonForm.Main").unwrap();
        let parse = |property: &str| {
            let descriptor = format!(
                r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" xmlns:v8="http://v8.1c.ru/8.1/data/core" version="2.20"><CommonForm uuid="e9455333-3333-4333-8333-333333333333"><Properties><Name>Main</Name>{property}</Properties></CommonForm></MetaDataObject>"#
            );
            parse_form_metadata_properties(descriptor.as_bytes(), &target)
        };
        let (absent, limits) = parse("").unwrap();
        assert!(!absent.contains_key("UsePurposes"));
        assert!(limits.is_empty());
        let (empty, limits) = parse("<UsePurposes/>").unwrap();
        assert_eq!(empty["UsePurposes"], json!([]));
        assert!(limits.is_empty());
        let (valid, limits) = parse("<UsePurposes>\n <v8:Value>PlatformApplication</v8:Value>\n <v8:Value>FuturePurpose</v8:Value>\n</UsePurposes>").unwrap();
        assert_eq!(
            valid["UsePurposes"],
            json!(["PlatformApplication", "FuturePurpose"])
        );
        assert!(
            limits.is_empty(),
            "observing an enum literal must not invent a platform default"
        );
        for malformed in [
            "<UsePurposes>PlatformApplication</UsePurposes>",
            "<UsePurposes>garbage<v8:Value>PlatformApplication</v8:Value></UsePurposes>",
            "<UsePurposes><v8:Value>PlatformApplication</v8:Value>garbage</UsePurposes>",
            "<UsePurposes><Value>PlatformApplication</Value></UsePurposes>",
            "<UsePurposes><v8:Value><Nested/></v8:Value></UsePurposes>",
        ] {
            let (properties, limits) =
                parse(malformed).expect("damaged property must preserve independent identity");
            assert_eq!(properties["Name"], "Main");
            assert_eq!(properties["uuid"], "e9455333-3333-4333-8333-333333333333");
            assert!(!properties.contains_key("UsePurposes") && !limits.is_empty(),
                "malformed UsePurposes must be unknown, never empty/success: {malformed}, {properties:?}, {limits:?}");
        }
        assert!(
            parse("<UsePurposes/><UsePurposes/>").is_err(),
            "duplicate containers remain a structural refusal"
        );
        for (property, key) in [
            (
                "<UseStandardCommands>maybe</UseStandardCommands>",
                "UseStandardCommands",
            ),
            ("<Synonym>not-localized</Synonym>", "Synonym"),
            (
                "<Explanation><v8:item><v8:lang>ru</v8:lang></v8:item></Explanation>",
                "Explanation",
            ),
        ] {
            let (properties, limits) = parse(property).unwrap();
            assert_eq!(properties["Name"], "Main");
            assert_eq!(properties["uuid"], "e9455333-3333-4333-8333-333333333333");
            assert!(
                !properties.contains_key(key),
                "damaged {key} was published: {properties:?}"
            );
            assert!(
                limits.iter().any(|limit| limit.contains(key)),
                "damaged {key} has no limit: {limits:?}"
            );
        }
    }
}
