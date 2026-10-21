use roxmltree::{Document, Node};

use super::xml_model::MD_CLASSES_NS;
use unica_metadata::{
    meta_object_integrity_rules, MetaCollection, MetaDiagnostic, MetaDiagnosticCode,
    MetaObjectRequirement, MetaPropertyKey, MetadataKind,
};

/// Платформа принимает такой дескриптор как документ и отвергает как объект
/// конфигурации, поэтому условие проверяется здесь, а не схемой.
pub fn check_meta_object_integrity(
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
        .find(|node| node.has_tag_name((MD_CLASSES_NS, kind.as_str())))
        .ok_or_else(|| {
            validation_failed(format!(
                "descriptor has no {} metadata object",
                kind.as_str()
            ))
        })?;

    for rule in rules {
        let satisfied = match rule.requirement {
            MetaObjectRequirement::AnyCollectionNonEmpty(collections) => {
                metadata_child(object, "ChildObjects").is_some_and(|children| {
                    collections.iter().any(|collection| {
                        collection_element_name(*collection).is_some_and(|element_name| {
                            children
                                .children()
                                .any(|child| child.has_tag_name((MD_CLASSES_NS, element_name)))
                        })
                    })
                })
            }
            MetaObjectRequirement::PropertyNonEmpty(property) => {
                metadata_child(object, "Properties")
                    .and_then(|properties| {
                        metadata_child(properties, property_element_name(property))
                    })
                    .and_then(|node| node.text())
                    .is_some_and(|value| !value.trim().is_empty())
            }
        };
        if !satisfied {
            return Err(validation_failed(format!(
                "{} rejects this object: {}. Provide the required fields in the same apply batch: {}",
                kind.as_str(),
                rule.platform_message,
                requirement_hint(rule.requirement)
            )));
        }
    }
    Ok(())
}

pub fn metadata_child<'a, 'input>(node: Node<'a, 'input>, name: &str) -> Option<Node<'a, 'input>> {
    node.children()
        .find(|child| child.has_tag_name((MD_CLASSES_NS, name)))
}

pub fn collection_element_name(collection: MetaCollection) -> Option<&'static str> {
    match collection {
        MetaCollection::Dimensions => Some("Dimension"),
        MetaCollection::Resources => Some("Resource"),
        MetaCollection::Attributes => Some("Attribute"),
        MetaCollection::TabularSections => Some("TabularSection"),
        MetaCollection::EnumValues => Some("EnumValue"),
        MetaCollection::Columns => Some("Column"),
        MetaCollection::Forms => Some("Form"),
        MetaCollection::Templates => Some("Template"),
        MetaCollection::Commands => Some("Command"),
        // Эта коллекция хранится в отдельном `Predefined.xml`, а проверка
        // целостности читает только итоговый дескриптор объекта.
        MetaCollection::PredefinedItems => None,
    }
}

pub fn property_element_name(property: MetaPropertyKey) -> &'static str {
    match property {
        MetaPropertyKey::Namespace => "Namespace",
        // Таблица содержит только `Namespace`; новый вид свойства заводится
        // вместе со своим именем элемента, а не молчаливым умолчанием.
        other => panic!("integrity rule references a property without an element name: {other:?}"),
    }
}

pub fn requirement_hint(requirement: MetaObjectRequirement) -> String {
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

pub fn validation_failed(message: impl Into<String>) -> MetaDiagnostic {
    MetaDiagnostic::error(MetaDiagnosticCode::ValidationFailed, message)
}

#[cfg(test)]
mod tests {
    use super::*;
    use unica_metadata::{MetaDiagnosticCode, MetadataKind};

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
    fn a_foreign_namespace_member_does_not_satisfy_required_composition() {
        let foreign = std::str::from_utf8(FILLED_REGISTER)
            .unwrap()
            .replace("<Resource>", "<Resource xmlns=\"urn:foreign\">");
        let error =
            check_meta_object_integrity(MetadataKind::InformationRegister, foreign.as_bytes())
                .expect_err("a same-named foreign element is not a register resource");
        assert_eq!(error.code, MetaDiagnosticCode::ValidationFailed);
        assert!(error
            .message
            .contains("Register without dimensions, resources, and attributes"));
    }

    #[test]
    fn foreign_namespace_containers_and_properties_do_not_satisfy_integrity() {
        for foreign in [
            std::str::from_utf8(FILLED_REGISTER)
                .unwrap()
                .replace("<ChildObjects>", "<ChildObjects xmlns=\"urn:foreign\">"),
            std::str::from_utf8(FILLED_NAMESPACE)
                .unwrap()
                .replace("<Namespace>", "<Namespace xmlns=\"urn:foreign\">"),
            std::str::from_utf8(FILLED_NAMESPACE)
                .unwrap()
                .replace("<Properties>", "<Properties xmlns=\"urn:foreign\">"),
        ] {
            let kind = if foreign.contains("<WebService>") {
                MetadataKind::WebService
            } else {
                MetadataKind::InformationRegister
            };
            check_meta_object_integrity(kind, foreign.as_bytes())
                .expect_err("foreign metadata is not proof of the required composition");
        }
    }

    #[test]
    fn a_different_prefix_for_the_metadata_namespace_preserves_integrity() {
        let prefixed = r#"<m:MetaDataObject xmlns:m="http://v8.1c.ru/8.3/MDClasses"><m:InformationRegister><m:Properties><m:Name>R</m:Name></m:Properties><m:ChildObjects><m:Resource><m:Properties><m:Name>Price</m:Name></m:Properties></m:Resource></m:ChildObjects></m:InformationRegister></m:MetaDataObject>"#;
        check_meta_object_integrity(MetadataKind::InformationRegister, prefixed.as_bytes())
            .unwrap();
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
