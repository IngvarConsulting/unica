//! Strict observation for the leaf changed by ScheduledJob.Key edits.

use roxmltree::Node;
use unica_metadata::{MetaDiagnostic, MetaDiagnosticCode, MetaInfoPropertyValueKind};

use crate::info_projection::{
    direct_meta_info_property, direct_text_content, meta_info_property_value_is_valid,
};

const MD: &str = "http://v8.1c.ru/8.3/MDClasses";

fn invalid(message: impl Into<String>) -> MetaDiagnostic {
    MetaDiagnostic::error(MetaDiagnosticCode::InvalidArguments, message).with_field("values.Key")
}

pub(crate) fn properties<'a, 'input>(
    object: Node<'a, 'input>,
) -> Result<Node<'a, 'input>, MetaDiagnostic> {
    let root = object
        .parent_element()
        .ok_or_else(|| invalid("Key owner has no root"))?;
    if !root.has_tag_name((MD, "MetaDataObject"))
        || root.children().filter(Node::is_element).count() != 1
        || !object.has_tag_name((MD, "ScheduledJob"))
    {
        return Err(invalid(
            "Key requires one ScheduledJob owner in the metadata namespace",
        ));
    }
    let containers = object
        .children()
        .filter(|node| node.is_element() && node.tag_name().name() == "Properties")
        .collect::<Vec<_>>();
    match containers.as_slice() {
        [node] if node.has_tag_name((MD, "Properties")) => Ok(*node),
        _ => Err(invalid(
            "Key requires one direct Properties container in the metadata namespace",
        )),
    }
}

pub(crate) fn leaf<'a, 'input>(
    properties: Node<'a, 'input>,
) -> Result<(Node<'a, 'input>, String), MetaDiagnostic> {
    let node = direct_meta_info_property(properties, "ScheduledJob", "Key")
        .map_err(|error| invalid(error.message))?
        .ok_or_else(|| invalid("property `Key` is not present on this metadata object"))?;
    if !meta_info_property_value_is_valid(node, MetaInfoPropertyValueKind::String) {
        return Err(invalid("ScheduledJob contains a malformed Key"));
    }
    Ok((node, direct_text_content(node)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::edit::apply_typed_properties;
    use unica_metadata::{MetaPropertyChanges, MetaPropertyInput, MetaPropertyValue, MetadataKind};

    fn change(value: &str) -> MetaPropertyChanges {
        MetaPropertyChanges::convert(
            MetadataKind::ScheduledJob,
            vec![MetaPropertyInput::new(
                "Key",
                MetaPropertyValue::String(value.to_string()),
            )],
        )
        .unwrap()
    }
    fn descriptor(key: &str) -> String {
        format!("<MetaDataObject xmlns=\"{MD}\"><ScheduledJob uuid=\"16670000-0000-4000-8000-000000000001\"><Properties><Name>Probe</Name><MethodName>EventHandlers.Run</MethodName>{key}<Use>true</Use></Properties></ScheduledJob></MetaDataObject>")
    }

    #[test]
    fn scheduled_job_key_edit_preserves_equivalent_lexical_bytes() {
        for (raw, value) in [
            ("<Key><![CDATA[a & b]]></Key>", "a & b"),
            ("<Key>a&#13;b</Key>", "a\rb"),
            ("<Key/>", ""),
            ("<Key></Key>", ""),
        ] {
            let mut source = descriptor(raw);
            let original = source.clone();
            apply_typed_properties(&mut source, &change(value), "ru").unwrap();
            assert_eq!(source, original);
        }
    }

    #[test]
    fn scheduled_job_key_edit_preserves_exact_text_and_neighbors() {
        let before = descriptor("<Key/>");
        let mut source = before.clone();
        let value = " Обмен & <очередь>\r\n\t\"ключ\" ";
        apply_typed_properties(&mut source, &change(value), "ru").unwrap();
        let doc = roxmltree::Document::parse(&source).unwrap();
        let key = doc
            .descendants()
            .find(|node| node.has_tag_name((MD, "Key")))
            .unwrap();
        assert_eq!(crate::info_projection::direct_text_content(key), value);
        assert_eq!(source.replace(&source[key.range()], "<Key/>"), before);
    }

    #[test]
    fn scheduled_job_key_edit_refuses_ambiguous_or_damaged_input_without_changes() {
        for raw in [
            "<Key/><Key/>",
            "<Key xmlns=\"urn:foreign\"/>",
            "<Key><Nested/></Key>",
            "<Key extra=\"bad\"/>",
        ] {
            let mut source = descriptor(raw);
            let original = source.clone();
            assert!(
                apply_typed_properties(&mut source, &change("new"), "ru").is_err(),
                "{raw}"
            );
            assert_eq!(source, original);
        }
        for extra in ["<Properties/>", "<Properties xmlns=\"urn:foreign\"/>"] {
            let mut source =
                descriptor("<Key/>").replace("</ScheduledJob>", &format!("{extra}</ScheduledJob>"));
            let original = source.clone();
            assert!(apply_typed_properties(&mut source, &change("new"), "ru").is_err());
            assert_eq!(source, original);
        }
    }
}

#[cfg(test)]
mod namespace_tests {
    use super::MD;
    use crate::edit::apply_typed_properties;
    use unica_metadata::{MetaPropertyChanges, MetaPropertyInput, MetaPropertyValue, MetadataKind};

    #[test]
    fn scheduled_job_key_edit_keeps_namespace_for_set_and_clear() {
        for (default, key) in [
            ("", "<m:Key>old</m:Key>"),
            (" xmlns=\"urn:other\"", "<m:Key>old</m:Key>"),
            ("", "<Key xmlns=\"http://v8.1c.ru/8.3/MDClasses\">old</Key>"),
        ] {
            for value in ["new", ""] {
                let mut source = format!("<m:MetaDataObject xmlns:m=\"{MD}\"{default}><m:ScheduledJob uuid=\"16670000-0000-4000-8000-000000000001\"><m:Properties><m:Name>Probe</m:Name>{key}</m:Properties></m:ScheduledJob></m:MetaDataObject>");
                let original = source.clone();
                let changes = MetaPropertyChanges::convert(
                    MetadataKind::ScheduledJob,
                    vec![MetaPropertyInput::new(
                        "Key",
                        MetaPropertyValue::String(value.to_string()),
                    )],
                )
                .unwrap();
                apply_typed_properties(&mut source, &changes, "ru").unwrap();
                let doc = roxmltree::Document::parse(&source).unwrap();
                let key = doc
                    .descendants()
                    .find(|node| node.has_tag_name((MD, "Key")))
                    .expect("edited Key must remain in the metadata namespace");
                assert_eq!(crate::info_projection::direct_text_content(key), value);
                let before = roxmltree::Document::parse(&original).unwrap();
                let old = before
                    .descendants()
                    .find(|node| node.has_tag_name((MD, "Key")))
                    .unwrap();
                assert_eq!(
                    source.replace(&source[key.range()], "<Key/>"),
                    original.replace(&original[old.range()], "<Key/>")
                );
            }
        }
    }
}
