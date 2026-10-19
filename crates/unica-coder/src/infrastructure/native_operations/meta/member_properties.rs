use super::info_projection::{
    direct_text_content, strict_localized_meta_info_property_value, MD_CLASSES_NAMESPACE,
};
use crate::domain::metadata::{
    MetaDiagnostic, MetaDiagnosticCode, MetaMemberReadProfile, MetaMemberValueKind,
    MetaObservedProperty,
};
use crate::domain::source_target::MetadataAddress;

/// Optional facts fail independently. Their failure never removes the member.
pub(super) fn observe(
    node: roxmltree::Node<'_, '_>,
    profile: MetaMemberReadProfile,
    field: &str,
    target: &MetadataAddress,
    diagnostics: &mut Vec<MetaDiagnostic>,
) -> (Vec<MetaObservedProperty>, Vec<String>) {
    let Some(context) = profile
        .context_for_field(field)
        .filter(|context| context.xml_tag() == node.tag_name().name())
    else {
        return (Vec::new(), Vec::new());
    };
    let containers = node
        .children()
        .filter(|child| child.is_element() && child.tag_name().name() == "Properties")
        .collect::<Vec<_>>();
    let properties = match containers.as_slice() {
        [properties]
            if properties.tag_name().namespace() == Some(MD_CLASSES_NAMESPACE)
                && properties.attributes().len() == 0 =>
        {
            Some(*properties)
        }
        _ => None,
    };
    let mut observed = Vec::new();
    let mut limits = Vec::new();
    for spec in profile.properties(context) {
        let nodes = properties
            .into_iter()
            .flat_map(|properties| properties.children())
            .filter(|child| child.is_element() && child.tag_name().name() == spec.name)
            .collect::<Vec<_>>();
        if properties.is_some() && nodes.is_empty() {
            continue;
        }
        let value = match nodes.as_slice() {
            [value]
                if value.tag_name().namespace() == Some(MD_CLASSES_NAMESPACE)
                    && value.attributes().len() == 0 =>
            {
                match spec.value_kind() {
                    MetaMemberValueKind::LocalizedString => {
                        strict_localized_meta_info_property_value(*value)
                    }
                    _ if !value.children().any(|child| child.is_element()) => {
                        spec.parse_text(&direct_text_content(*value))
                    }
                    _ => None,
                }
            }
            _ => None,
        };
        if let Some(value) = value {
            observed.push(MetaObservedProperty {
                name: spec.name.into(),
                value,
            });
        } else {
            let property_field = format!("{field}.properties.{}", spec.name);
            let message =
                format!("{property_field} is malformed, duplicated, or has a foreign namespace");
            limits.push(message.clone());
            diagnostics.push(
                MetaDiagnostic::error(MetaDiagnosticCode::ValidationFailed, message)
                    .with_metadata_path(target.clone())
                    .with_field(property_field),
            );
        }
    }
    (observed, limits)
}

#[cfg(test)]
mod tests;
