use super::*;
use unica_metadata::{MetaChoiceParameter, MetaChoiceParameterValue};

const APP_NS: &str = "http://v8.1c.ru/8.2/managed-application/core";
const FIELD: &str = "details.choiceParameters";

pub(super) fn parse(
    container: roxmltree::Node<'_, '_>,
) -> Result<Vec<MetaChoiceParameter>, ProjectionError> {
    ensure_container_shape(container, FIELD)?;
    container.children().filter(roxmltree::Node::is_element).enumerate().map(|(index, item)| {
        let field = format!("{FIELD}[{index}]");
        if !item.has_tag_name((APP_NS,"item")) || item.attributes().len()!=1
            || item.attribute("name").is_none_or(|name| name.trim().is_empty())
            || !node_has_only_whitespace_text(item)
        { return Err(ProjectionError::malformed(&field,"choice parameter must be a named app:item")); }
        let children=item.children().filter(roxmltree::Node::is_element).collect::<Vec<_>>();
        let [value]=children.as_slice() else { return Err(ProjectionError::malformed(&field,"choice parameter must contain one app:value")); };
        if !value.has_tag_name((APP_NS,"value")) || value.attributes().len()!=1 || value.children().any(|child|child.is_element()) {
            return Err(ProjectionError::malformed(&field,"choice parameter value has an unexpected structure"));
        }
        let raw_type=value.attribute((XML_SCHEMA_INSTANCE_NAMESPACE,"type")).ok_or_else(|| ProjectionError::malformed(&field,"choice parameter value has no xsi:type"))?;
        let expanded = expand_qname(*value, raw_type, &field)?;
        if expanded.namespace != READABLE_NAMESPACE || expanded.local_name != "DesignTimeRef" {
            return Err(ProjectionError::unsupported(&field,"choice parameter type is outside the DesignTimeRef profile"));
        }
        let reference=unica_xml_format::read::direct_text(*value).trim().to_string();
        let parts=reference.split('.').collect::<Vec<_>>();
        if !matches!(parts.as_slice(),["Enum", owner, "EnumValue", name] if unica_metadata::identifier::is_1c_identifier(owner) && unica_metadata::identifier::is_1c_identifier(name)) {
            return Err(ProjectionError::unsupported(&field,"choice parameter reference must name a concrete EnumValue"));
        }
        Ok(MetaChoiceParameter { name:item.attribute("name").unwrap().to_string(), value:MetaChoiceParameterValue { r#type:expanded,reference } })
    }).collect()
}
