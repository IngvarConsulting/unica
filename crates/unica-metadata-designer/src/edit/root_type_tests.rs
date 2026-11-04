use super::*;
use crate::info_projection::MD_CLASSES_NAMESPACE;
use unica_metadata::descriptor_operations::TypePropertyOwner;

fn aliased_descriptor(owner: TypePropertyOwner, line_ending: &str, bom: bool) -> String {
    format!(
        "{}<m:MetaDataObject xmlns:m=\"http://v8.1c.ru/8.3/MDClasses\" xmlns:d=\"http://v8.1c.ru/8.1/data/core\" xmlns:a=\"http://www.w3.org/2001/XMLSchema\" xmlns:v8=\"urn:shadow-core\" xmlns:cfg=\"urn:shadow-config\" xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\" version=\"2.20\">\n  <m:{} uuid=\"12345678-1234-4123-8123-123456789abc\">\n    <m:Properties><m:Name>Existing</m:Name><m:Comment>keep &amp; this</m:Comment>\n      <m:Type><d:Type>a:string</d:Type><d:StringQualifiers><d:Length>17</d:Length><d:AllowedLength>Variable</d:AllowedLength></d:StringQualifiers></m:Type>\n      <m:Other xsi:type=\"v8:Shadow\">unchanged</m:Other></m:Properties>\n  </m:{}>\n</m:MetaDataObject>",
        if bom { "\u{feff}" } else { "" }, owner.node_kind().as_str(), owner.node_kind().as_str(),
    ).replace('\n', line_ending)
}

fn parsed_type(value: Value) -> MetadataType {
    unica_metadata::request::parse_metadata_type(&value, "values.Type").unwrap()
}

#[test]
fn root_type_image_preserves_equivalent_aliases_and_exact_bytes() {
    let metadata_type = parsed_type(
        serde_json::json!({"variants":[{"kind":"string","length":17,"allowedLength":"variable"}]}),
    );
    for owner in [
        TypePropertyOwner::CommonAttribute,
        TypePropertyOwner::SessionParameter,
        TypePropertyOwner::FilterCriterion,
    ] {
        for eol in ["\n", "\r\n", "\r"] {
            for bom in [false, true] {
                let original = aliased_descriptor(owner, eol, bom);
                let mut current = original.clone();
                apply_descriptor_type(&mut current, owner, &metadata_type).unwrap();
                assert_eq!(current, original);
            }
        }
    }
}

#[test]
fn root_type_image_owns_new_qnames_without_changing_neighbor_bindings() {
    let metadata_type = parsed_type(
        serde_json::json!({"variants":[{"kind":"reference","metadataPath":"Catalog.Items"}]}),
    );
    for owner in [
        TypePropertyOwner::CommonAttribute,
        TypePropertyOwner::SessionParameter,
        TypePropertyOwner::FilterCriterion,
    ] {
        let original = aliased_descriptor(owner, "\n", true);
        let mut current = original.clone();
        apply_descriptor_type(&mut current, owner, &metadata_type).unwrap();
        assert_eq!(
            MetadataType::try_from(crate::root_type::observe(current.as_bytes()).unwrap()).unwrap(),
            metadata_type
        );
        let replace_type = |text: &str| {
            let body = text.trim_start_matches('\u{feff}');
            let document = Document::parse(body).unwrap();
            let node = document
                .descendants()
                .find(|node| node.has_tag_name((MD_CLASSES_NAMESPACE, "Type")))
                .unwrap();
            let mut result = text.to_string();
            let prefix = text.len() - body.len();
            result.replace_range(
                node.range().start + prefix..node.range().end + prefix,
                "<Type/>",
            );
            result
        };
        assert_eq!(replace_type(&current), replace_type(&original));
    }
}

#[test]
fn root_type_image_refuses_missing_duplicate_and_foreign_type_without_changes() {
    let metadata_type = parsed_type(serde_json::json!({"variants":[{"kind":"boolean"}]}));
    let owner = TypePropertyOwner::CommonAttribute;
    let original = aliased_descriptor(owner, "\n", false);
    for current in [
        original
            .replace("<m:Type>", "<foreign:Type xmlns:foreign=\"urn:foreign\">")
            .replace("</m:Type>", "</foreign:Type>"),
        original.replace(
            "</m:Type>",
            "</m:Type><m:Type><d:Type>a:boolean</d:Type></m:Type>",
        ),
        original.replace("<m:Type>", "<m:Type xsi:nil=\"true\">"),
        original.replace("<m:Properties>", "<m:Properties xsi:nil=\"true\">"),
        original.replace(
            "<m:CommonAttribute uuid=",
            "<m:CommonAttribute xsi:nil=\"true\" uuid=",
        ),
    ] {
        let mut after = current.clone();
        assert!(
            apply_descriptor_type(&mut after, owner, &metadata_type).is_err(),
            "{current}"
        );
        assert_eq!(after, current);
    }
}
