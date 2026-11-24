//! Read-back facts for properties written by the external data source profile.

use roxmltree::Node;
use unica_address::address::NodeKind;
use unica_address::source_target::MetadataAddress;
use unica_metadata::{MetaInfoPropertyValueKind, MetaObservedProperty, MetaObservedPropertyValue};

use crate::info_projection::{
    direct_meta_info_property, direct_text_content, meta_info_property_value_is_valid,
};

pub(crate) fn inline(
    target: &MetadataAddress,
    node: Node<'_, '_>,
    properties: Node<'_, '_>,
    path: &str,
) -> (Vec<MetaObservedProperty>, Vec<String>) {
    if !target.as_str().starts_with("ExternalDataSource.") {
        return (Vec::new(), Vec::new());
    }
    let Ok(kind) = NodeKind::parse(node.tag_name().name()) else {
        return (Vec::new(), Vec::new());
    };
    let mut values = Vec::new();
    let mut limits = Vec::new();
    for &name in unica_metadata::external_data_source::inline_read_facts(kind) {
        let observed = (|| {
            let Some(property) = direct_meta_info_property(properties, kind.as_str(), name)
                .map_err(|error| error.message)?
            else {
                return Ok(None);
            };
            let text = direct_text_content(property);
            let value = if matches!(name, "ReturnValue" | "ReadOnly" | "AllowNull") {
                if !meta_info_property_value_is_valid(property, MetaInfoPropertyValueKind::Boolean)
                {
                    return Err(format!("{name} must be canonical boolean text"));
                }
                Some(MetaObservedPropertyValue::Boolean {
                    value: text == "true",
                })
            } else {
                if !meta_info_property_value_is_valid(property, MetaInfoPropertyValueKind::String) {
                    return Err(format!("{name} must be canonical scalar text"));
                }
                (!text.is_empty()).then_some(MetaObservedPropertyValue::Text { value: text })
            };
            Ok(value)
        })();
        match observed {
            Ok(Some(value)) => values.push(MetaObservedProperty {
                name: name.into(),
                value,
            }),
            Ok(None) => {}
            Err(message) => limits.push(format!("source_unreadable: {path}.{name}: {message}")),
        }
    }
    (values, limits)
}
