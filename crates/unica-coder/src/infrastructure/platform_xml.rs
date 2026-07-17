use crate::domain::discovery_registry::metadata_kind;
use roxmltree::{Document, Node};
use std::collections::BTreeSet;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct RootRegistration {
    pub(crate) kind: String,
    pub(crate) directory: String,
    pub(crate) name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct NestedRegistrations {
    pub(crate) forms: Vec<String>,
    pub(crate) templates: Vec<String>,
    pub(crate) commands: Vec<String>,
}

pub(crate) fn parse_configuration_registrations(
    bytes: &[u8],
) -> Result<Vec<RootRegistration>, String> {
    let text = std::str::from_utf8(bytes)
        .map_err(|_| "malformed_registration: Configuration.xml is not UTF-8")?;
    let document = Document::parse(text)
        .map_err(|error| format!("malformed_registration: Configuration.xml: {error}"))?;
    let metadata_root = document.root_element();
    require_local_name(metadata_root, "MetaDataObject", "Configuration.xml root")?;
    let configuration = exactly_one_direct_element(metadata_root, "Configuration")?;
    let child_objects = exactly_one_direct_element(configuration, "ChildObjects")?;
    let mut registrations = Vec::new();
    let mut paths = BTreeSet::new();
    for node in child_objects.children().filter(Node::is_element) {
        let tag = node.tag_name().name();
        let kind = metadata_kind(tag).ok_or_else(|| {
            format!("unknown_registration_kind: Configuration.xml ChildObjects/{tag}")
        })?;
        let name = registration_value(node, "Configuration.xml")?;
        let folded_path = format!("{}/{}.xml", kind.directory, name).to_lowercase();
        if !paths.insert(folded_path) {
            return Err(format!(
                "duplicate_registration: Configuration.xml contains duplicate {tag}/{name}"
            ));
        }
        registrations.push(RootRegistration {
            kind: tag.to_string(),
            directory: kind.directory.to_string(),
            name,
        });
    }
    registrations.sort();
    Ok(registrations)
}

pub(crate) fn parse_registered_descriptor(
    bytes: &[u8],
    registration: &RootRegistration,
) -> Result<NestedRegistrations, String> {
    let text = std::str::from_utf8(bytes).map_err(|_| {
        format!(
            "malformed_registered_object: {}/{}.xml is not UTF-8",
            registration.directory, registration.name
        )
    })?;
    let document = Document::parse(text).map_err(|error| {
        format!(
            "malformed_registered_object: {}/{}.xml: {error}",
            registration.directory, registration.name
        )
    })?;
    let metadata_root = document.root_element();
    require_local_name(
        metadata_root,
        "MetaDataObject",
        "registered descriptor root",
    )?;
    let object = exactly_one_direct_element(metadata_root, &registration.kind)?;
    let properties = exactly_one_direct_element(object, "Properties")?;
    let name_node = exactly_one_direct_element(properties, "Name")?;
    let actual_name = registration_value(name_node, "registered descriptor Name")?;
    if actual_name != registration.name {
        return Err(format!(
            "registered_object_identity_mismatch: expected {} {}, descriptor names {}",
            registration.kind, registration.name, actual_name
        ));
    }

    let child_objects = optional_one_direct_element(object, "ChildObjects")?;
    let mut nested = NestedRegistrations::default();
    let mut forms = BTreeSet::new();
    let mut templates = BTreeSet::new();
    let mut commands = BTreeSet::new();
    if let Some(child_objects) = child_objects {
        for child in child_objects.children().filter(Node::is_element) {
            let value = registration_value(child, "registered descriptor ChildObjects")?;
            match child.tag_name().name() {
                "Form" => {
                    if !forms.insert(value.to_lowercase()) {
                        return Err(format!("duplicate_nested_registration: Form/{value}"));
                    }
                    nested.forms.push(value);
                }
                "Template" => {
                    if !templates.insert(value.to_lowercase()) {
                        return Err(format!("duplicate_nested_registration: Template/{value}"));
                    }
                    nested.templates.push(value);
                }
                "Command" => {
                    if !commands.insert(value.to_lowercase()) {
                        return Err(format!("duplicate_nested_registration: Command/{value}"));
                    }
                    nested.commands.push(value);
                }
                _ => {}
            }
        }
    }
    nested.forms.sort_by_key(|name| name.to_lowercase());
    nested.templates.sort_by_key(|name| name.to_lowercase());
    nested.commands.sort_by_key(|name| name.to_lowercase());
    Ok(nested)
}

fn registration_value(node: Node<'_, '_>, context: &str) -> Result<String, String> {
    if node.children().any(|child| child.is_element()) {
        return Err(format!(
            "invalid_registration_value: {context}: element content is forbidden"
        ));
    }
    let mut semantic_text = node
        .children()
        .filter_map(|child| child.text())
        .filter(|text| !text.is_empty());
    let text = semantic_text.next().unwrap_or_default();
    if semantic_text.next().is_some() {
        return Err(format!(
            "invalid_registration_value: {context}: ambiguous text content"
        ));
    }
    if text.is_empty()
        || text.trim() != text
        || matches!(text, "." | "..")
        || text.contains('/')
        || text.contains('\\')
        || text.contains(':')
        || text.chars().any(char::is_control)
        || !text
            .chars()
            .all(|character| character.is_alphanumeric() || character == '_')
    {
        return Err(format!("invalid_registration_value: {context}: {text:?}"));
    }
    Ok(text.to_string())
}

fn require_local_name(node: Node<'_, '_>, expected: &str, context: &str) -> Result<(), String> {
    if node.tag_name().name() != expected {
        return Err(format!(
            "malformed_registration: {context} must be {expected}, got {}",
            node.tag_name().name()
        ));
    }
    Ok(())
}

fn exactly_one_direct_element<'a, 'input>(
    parent: Node<'a, 'input>,
    name: &str,
) -> Result<Node<'a, 'input>, String> {
    optional_one_direct_element(parent, name)?.ok_or_else(|| {
        format!(
            "malformed_registration: {} must contain direct {name}",
            parent.tag_name().name()
        )
    })
}

fn optional_one_direct_element<'a, 'input>(
    parent: Node<'a, 'input>,
    name: &str,
) -> Result<Option<Node<'a, 'input>>, String> {
    let mut matches = parent
        .children()
        .filter(Node::is_element)
        .filter(|node| node.tag_name().name() == name);
    let first = matches.next();
    if matches.next().is_some() {
        return Err(format!(
            "malformed_registration: {} contains duplicate direct {name}",
            parent.tag_name().name()
        ));
    }
    Ok(first)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn configuration_parser_requires_direct_known_safe_unique_registrations() {
        let valid = br#"<MetaDataObject><Configuration><ChildObjects><CommonModule>Safe_Name1</CommonModule></ChildObjects></Configuration></MetaDataObject>"#;
        assert_eq!(
            parse_configuration_registrations(valid).unwrap(),
            [RootRegistration {
                kind: "CommonModule".into(),
                directory: "CommonModules".into(),
                name: "Safe_Name1".into(),
            }]
        );

        for invalid in [
            "<MetaDataObject><Configuration><ChildObjects><Unknown>X</Unknown></ChildObjects></Configuration></MetaDataObject>",
            "<MetaDataObject><Configuration><ChildObjects><CommonModule>../X</CommonModule></ChildObjects></Configuration></MetaDataObject>",
            "<MetaDataObject><Configuration><Wrapper><ChildObjects><CommonModule>X</CommonModule></ChildObjects></Wrapper></Configuration></MetaDataObject>",
            "<MetaDataObject><Configuration><ChildObjects><CommonModule>X</CommonModule><CommonModule>x</CommonModule></ChildObjects></Configuration></MetaDataObject>",
        ] {
            assert!(parse_configuration_registrations(invalid.as_bytes()).is_err(), "accepted {invalid}");
        }
    }

    #[test]
    fn configuration_parser_accepts_utf8_bom_and_namespace_prefixes() {
        let prefixed = b"\xef\xbb\xbf<md:MetaDataObject xmlns:md=\"urn:1c\"><md:Configuration><md:ChildObjects><md:CommonModule>Safe</md:CommonModule></md:ChildObjects></md:Configuration></md:MetaDataObject>";
        assert_eq!(
            parse_configuration_registrations(prefixed).unwrap(),
            [RootRegistration {
                kind: "CommonModule".into(),
                directory: "CommonModules".into(),
                name: "Safe".into(),
            }]
        );
    }

    #[test]
    fn parsers_reject_mixed_registration_content() {
        let root_mixed = br#"<MetaDataObject><Configuration><ChildObjects><CommonModule>Safe<Trap/>Name</CommonModule></ChildObjects></Configuration></MetaDataObject>"#;
        assert!(parse_configuration_registrations(root_mixed).is_err());

        let registration = RootRegistration {
            kind: "Document".into(),
            directory: "Documents".into(),
            name: "Sale".into(),
        };
        let nested_mixed = br#"<MetaDataObject><Document><Properties><Name>Sale</Name></Properties><ChildObjects><Form>Main<Trap/>Form</Form></ChildObjects></Document></MetaDataObject>"#;
        assert!(parse_registered_descriptor(nested_mixed, &registration).is_err());
    }

    #[test]
    fn descriptor_parser_binds_kind_name_and_nested_form_template_registration() {
        let registration = RootRegistration {
            kind: "Document".into(),
            directory: "Documents".into(),
            name: "Sale".into(),
        };
        let valid = br#"<MetaDataObject><Document><Properties><Name>Sale</Name></Properties><ChildObjects><Form>Main</Form><Template>Print</Template><Command>Post</Command><Attribute>Number</Attribute></ChildObjects></Document></MetaDataObject>"#;
        let nested = parse_registered_descriptor(valid, &registration).unwrap();
        assert_eq!(nested.forms, ["Main"]);
        assert_eq!(nested.templates, ["Print"]);
        assert_eq!(nested.commands, ["Post"]);

        let wrong_kind = br#"<MetaDataObject><Catalog><Properties><Name>Sale</Name></Properties><ChildObjects/></Catalog></MetaDataObject>"#;
        let wrong_name = br#"<MetaDataObject><Document><Properties><Name>Other</Name></Properties><ChildObjects/></Document></MetaDataObject>"#;
        assert!(parse_registered_descriptor(wrong_kind, &registration).is_err());
        assert!(parse_registered_descriptor(wrong_name, &registration).is_err());
    }
}
