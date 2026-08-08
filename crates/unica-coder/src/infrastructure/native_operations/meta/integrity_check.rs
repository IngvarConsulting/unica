use roxmltree::{Document, Node};

use super::xml_model::meta_info_child;
use crate::domain::metadata::{
    meta_object_integrity_rules, MetaCollection, MetaDiagnostic, MetaDiagnosticCode,
    MetaObjectRequirement, MetaPropertyKey, MetadataKind,
};

/// Платформа принимает такой дескриптор как документ и отвергает как объект
/// конфигурации, поэтому условие проверяется здесь, а не схемой (ADR-0030).
pub(super) fn check_meta_object_integrity(
    kind: MetadataKind,
    descriptor: &[u8],
) -> Result<(), MetaDiagnostic> {
    let mut rules = meta_object_integrity_rules(kind).peekable();
    if rules.peek().is_none() {
        return Ok(());
    }
    let text = std::str::from_utf8(descriptor)
        .map_err(|error| validation_failed(format!("descriptor is not UTF-8: {error}")))?;
    let document = Document::parse(text.trim_start_matches('\u{feff}'))
        .map_err(|error| validation_failed(format!("descriptor is not valid XML: {error}")))?;
    let object = document
        .root_element()
        .children()
        .find(Node::is_element)
        .ok_or_else(|| validation_failed("descriptor has no metadata object"))?;

    for rule in rules {
        let satisfied = match rule.requirement {
            MetaObjectRequirement::AnyCollectionNonEmpty(collections) => {
                meta_info_child(object, "ChildObjects").is_some_and(|children| {
                    collections.iter().any(|collection| {
                        children.children().any(|child| {
                            child.is_element()
                                && child.tag_name().name() == collection_element_name(*collection)
                        })
                    })
                })
            }
            MetaObjectRequirement::PropertyNonEmpty(property) => {
                meta_info_child(object, "Properties")
                    .and_then(|properties| {
                        meta_info_child(properties, property_element_name(property))
                    })
                    .and_then(|node| node.text())
                    .is_some_and(|value| !value.trim().is_empty())
            }
        };
        if !satisfied {
            return Err(validation_failed(format!(
                "{} rejects this object: {}. Provide it through `operations` on the same call: {}",
                kind.as_str(),
                rule.platform_message,
                requirement_hint(rule.requirement)
            )));
        }
    }
    Ok(())
}

fn collection_element_name(collection: MetaCollection) -> &'static str {
    match collection {
        MetaCollection::Dimensions => "Dimension",
        MetaCollection::Resources => "Resource",
        MetaCollection::Attributes => "Attribute",
        MetaCollection::TabularSections => "TabularSection",
        MetaCollection::EnumValues => "EnumValue",
        MetaCollection::Columns => "Column",
        MetaCollection::Forms => "Form",
        MetaCollection::Templates => "Template",
        MetaCollection::Commands => "Command",
    }
}

fn property_element_name(property: MetaPropertyKey) -> &'static str {
    match property {
        MetaPropertyKey::Namespace => "Namespace",
        // Таблица содержит только `Namespace`; новый вид свойства заводится
        // вместе со своим именем элемента, а не молчаливым умолчанием.
        other => panic!("integrity rule references a property without an element name: {other:?}"),
    }
}

fn requirement_hint(requirement: MetaObjectRequirement) -> String {
    match requirement {
        MetaObjectRequirement::AnyCollectionNonEmpty(collections) => format!(
            "add at least one of {}",
            collections
                .iter()
                .map(|collection| collection.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ),
        MetaObjectRequirement::PropertyNonEmpty(property) => {
            format!("set a non-empty `{}`", property_element_name(property))
        }
    }
}

fn validation_failed(message: impl Into<String>) -> MetaDiagnostic {
    MetaDiagnostic::error(MetaDiagnosticCode::ValidationFailed, message)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::metadata::{MetaDiagnosticCode, MetadataKind};

    const EMPTY_REGISTER: &[u8] = br#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses"><InformationRegister><Properties><Name>R</Name></Properties><ChildObjects/></InformationRegister></MetaDataObject>"#;
    const FILLED_REGISTER: &[u8] = br#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses"><InformationRegister><Properties><Name>R</Name></Properties><ChildObjects><Resource><Properties><Name>Price</Name></Properties></Resource></ChildObjects></InformationRegister></MetaDataObject>"#;
    const EMPTY_NAMESPACE: &[u8] = br#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses"><WebService><Properties><Name>S</Name><Namespace/></Properties></WebService></MetaDataObject>"#;
    const FILLED_NAMESPACE: &[u8] = br#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses"><WebService><Properties><Name>S</Name><Namespace>urn:corpus</Namespace></Properties></WebService></MetaDataObject>"#;

    #[test]
    fn register_without_any_child_collection_is_refused_with_the_platform_message() {
        let error = check_meta_object_integrity(MetadataKind::InformationRegister, EMPTY_REGISTER)
            .expect_err("empty register must be refused");

        assert_eq!(error.code, MetaDiagnosticCode::ValidationFailed);
        assert!(
            error
                .message
                .contains("Register without dimensions, resources, and attributes"),
            "{error:?}"
        );
        assert!(error.message.contains("dimensions"), "{error:?}");
    }

    #[test]
    fn register_with_one_resource_is_accepted() {
        check_meta_object_integrity(MetadataKind::InformationRegister, FILLED_REGISTER).unwrap();
    }

    #[test]
    fn web_service_namespace_must_be_non_empty() {
        check_meta_object_integrity(MetadataKind::WebService, EMPTY_NAMESPACE)
            .expect_err("empty namespace must be refused");
        check_meta_object_integrity(MetadataKind::WebService, FILLED_NAMESPACE).unwrap();
    }

    #[test]
    fn kinds_without_a_rule_are_never_refused() {
        check_meta_object_integrity(MetadataKind::Catalog, EMPTY_REGISTER).unwrap();
    }
}
