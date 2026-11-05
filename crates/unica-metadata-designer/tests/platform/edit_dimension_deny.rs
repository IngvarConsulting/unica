use super::apply_typed_operations_to_image_with_seed;
use unica_metadata::{MetaCollection, MetaDiagnosticCode, MetaEditOperation, MetaElementUpdate};

/// The 8.3.27 export of `workspace-register-dimensions`.
fn register() -> String {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tests/fixtures/acceptance/workspace-register-dimensions/src/AccumulationRegisters/Остатки.xml"
    );
    String::from_utf8(std::fs::read(path).unwrap()).unwrap()
}

fn deny(name: &str, value: bool) -> MetaEditOperation {
    MetaEditOperation::Update {
        collection: MetaCollection::Dimensions,
        scope: None,
        elements: vec![MetaElementUpdate {
            name: name.to_string(),
            new_name: None,
            synonym: None,
            comment: None,
            r#type: None,
            required: None,
            fill_value: None,
            position: None,
            indexing: None,
            deny_incomplete_values: Some(value),
            use_in_totals: None,
            master: None,
            main_filter: None,
            type_reduction_mode: None,
        }],
    }
}

fn apply(
    xml: &str,
    operation: MetaEditOperation,
) -> Result<String, (MetaDiagnosticCode, Option<String>)> {
    let mut text = xml.trim_start_matches('\u{feff}').to_string();
    apply_typed_operations_to_image_with_seed(&mut text, &[operation], "", b"deny")
        .map(|()| text)
        .map_err(|failure| {
            let diagnostic = failure.diagnostics.into_iter().next().unwrap();
            (diagnostic.code, diagnostic.field)
        })
}

#[test]
fn deny_incomplete_values_changes_only_its_own_value_of_the_named_dimension() {
    let before = register().trim_start_matches('\u{feff}').to_string();
    let after = apply(&before, deny("Товар", true)).unwrap();
    let first = before
        .find("<DenyIncompleteValues>false</DenyIncompleteValues>")
        .unwrap();
    assert!(first < before.find("<Name>Склад</Name>").unwrap());
    let mut expected = before.clone();
    expected.replace_range(
        first..first + "<DenyIncompleteValues>false</DenyIncompleteValues>".len(),
        "<DenyIncompleteValues>true</DenyIncompleteValues>",
    );
    assert_eq!(after, expected);
    assert_eq!(apply(&after, deny("Товар", false)).unwrap(), before);
    assert_eq!(apply(&before, deny("Товар", false)).unwrap(), before);
}

#[test]
fn a_dimension_without_deny_incomplete_values_is_refused_without_a_guessed_place() {
    let before = register().trim_start_matches('\u{feff}').to_string();
    let line = before.find("<DenyIncompleteValues>").unwrap();
    let start = before[..line].rfind('\n').unwrap();
    let end = line + before[line..].find('\n').unwrap();
    let mut without = before.clone();
    without.replace_range(start..end, "");
    let (code, field) = apply(&without, deny("Товар", true)).unwrap_err();
    assert_eq!(code, MetaDiagnosticCode::InvalidArguments);
    assert_eq!(
        field.as_deref(),
        Some("operations[0].elements[0].denyIncompleteValues")
    );
}

#[test]
fn a_damaged_or_unfamiliar_deny_incomplete_values_is_refused_instead_of_overwritten() {
    let before = register().trim_start_matches('\u{feff}').to_string();
    let original = "<DenyIncompleteValues>false</DenyIncompleteValues>";
    for replacement in [
        "<DenyIncompleteValues>garbage</DenyIncompleteValues>",
        "<DenyIncompleteValues></DenyIncompleteValues>",
        "<DenyIncompleteValues>1</DenyIncompleteValues>",
        "<DenyIncompleteValues/>",
    ] {
        let input = before.replacen(original, replacement, 1);
        let (code, field) = apply(&input, deny("Товар", true)).unwrap_err();
        assert_eq!(code, MetaDiagnosticCode::InvalidArguments, "{replacement}");
        assert_eq!(
            field.as_deref(),
            Some("operations[0].elements[0].denyIncompleteValues"),
            "{replacement}"
        );
    }
}

#[test]
fn deny_rejects_duplicate_foreign_and_nested_boolean_nodes() {
    let before = register();
    let original = "<DenyIncompleteValues>false</DenyIncompleteValues>";
    for replacement in [
        "<DenyIncompleteValues>false</DenyIncompleteValues><DenyIncompleteValues>true</DenyIncompleteValues>",
        "<DenyIncompleteValues xmlns=\"urn:foreign\">false</DenyIncompleteValues>",
        "<DenyIncompleteValues><Nested>false</Nested></DenyIncompleteValues>",
        "<DenyIncompleteValues extra=\"value\">false</DenyIncompleteValues>",
    ] {
        assert!(apply(&before.replacen(original, replacement, 1), deny("Товар", true)).is_err(), "{replacement}");
    }
}

fn information_register() -> String {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../tests/fixtures/acceptance/workspace-information-register-dimension-flags/src/InformationRegisters/Facts.xml");
    let mut xml = String::from_utf8(std::fs::read(path).unwrap()).unwrap();
    for tag in ["Master", "MainFilter", "DenyIncompleteValues"] {
        xml = xml.replace(
            &format!("<{tag}>true</{tag}>"),
            &format!("<{tag}>false</{tag}>"),
        );
    }
    xml
}

fn flag(name: &str, field: &str, value: bool) -> MetaEditOperation {
    let mut operation = deny(name, value);
    let MetaEditOperation::Update { elements, .. } = &mut operation else {
        unreachable!()
    };
    let element = &mut elements[0];
    element.deny_incomplete_values = None;
    match field {
        "Master" => element.master = Some(value),
        "MainFilter" => element.main_filter = Some(value),
        "DenyIncompleteValues" => element.deny_incomplete_values = Some(value),
        _ => unreachable!(),
    }
    operation
}

#[test]
fn information_flags_edit_only_selected_literal_and_preserve_noop_bytes() {
    for tag in ["Master", "MainFilter", "DenyIncompleteValues"] {
        let before = information_register()
            .trim_start_matches('\u{feff}')
            .to_string();
        let old = format!("<{tag}>false</{tag}>");
        let decorated =
            format!("<md:{tag} xmlns:md=\"http://v8.1c.ru/8.3/MDClasses\">false</md:{tag}>");
        let before = before.replacen(&old, &decorated, 1);
        let expected = before.replacen(&decorated, &decorated.replace(">false<", ">true<"), 1);
        assert_eq!(
            apply(&before, flag("StringDimension", tag, true)).unwrap(),
            expected
        );
        assert_eq!(
            apply(&before, flag("StringDimension", tag, false)).unwrap(),
            before
        );
        assert_eq!(
            apply(&expected, flag("StringDimension", tag, false)).unwrap(),
            before
        );
    }
}

#[test]
fn every_information_flag_refuses_missing_foreign_duplicate_and_nonliteral_values() {
    for tag in ["Master", "MainFilter", "DenyIncompleteValues"] {
        let original = format!("<{tag}>false</{tag}>");
        for replacement in [
            String::new(),
            format!("<{tag} xmlns=\"urn:foreign\">false</{tag}>"),
            format!("{original}{original}"),
            format!("<{tag}><Nested>false</Nested></{tag}>"),
            format!("<{tag}>1</{tag}>"),
            format!("<{tag}>\u{00a0}false\u{00a0}</{tag}>"),
            format!("<{tag}>&#xA0;false&#160;</{tag}>"),
            format!("<{tag} extra=\"value\">false</{tag}>"),
            format!("<{tag}/>"),
        ] {
            let before = information_register().replacen(&original, &replacement, 1);
            let (code, field) = apply(&before, flag("StringDimension", tag, true)).unwrap_err();
            let field_name = match tag {
                "Master" => "master",
                "MainFilter" => "mainFilter",
                _ => "denyIncompleteValues",
            };
            assert_eq!(
                code,
                MetaDiagnosticCode::InvalidArguments,
                "{tag}: {replacement}"
            );
            assert!(
                apply(&before, flag("StringDimension", tag, false)).is_err(),
                "invalid no-op: {tag}: {replacement}"
            );
            assert_eq!(
                field.as_deref(),
                Some(format!("operations[0].elements[0].{field_name}").as_str()),
                "{tag}: {replacement}"
            );
        }
    }
}

#[test]
fn adding_information_dimension_emits_requested_flags_and_false_defaults() {
    use unica_metadata::{MetaElementDefinition, MetadataType, MetadataTypeVariant};
    for value in [None, Some(false), Some(true)] {
        let element = MetaElementDefinition {
            name: "NewDimension".into(),
            synonym: None,
            comment: None,
            r#type: Some(MetadataType {
                variants: vec![MetadataTypeVariant::Boolean],
            }),
            required: None,
            fill_value: None,
            attributes: vec![],
            position: None,
            template_type: None,
            indexing: None,
            master: value,
            main_filter: value,
            deny_incomplete_values: value,
        };
        let rendered = super::render_typed_element(
            "InformationRegister",
            "Facts",
            MetaCollection::Dimensions,
            false,
            &element,
            "ru",
            &mut || "22910000-0000-4000-8000-000000000001".into(),
        )
        .unwrap()
        .join("\n");
        for tag in ["Master", "MainFilter", "DenyIncompleteValues"] {
            assert!(
                rendered.contains(&format!("<{tag}>{}</{tag}>", value.unwrap_or(false))),
                "{rendered}"
            );
        }
        assert!(rendered.contains("<v8:Type>xs:boolean</v8:Type>"));
    }
}

#[test]
fn all_flags_in_one_update_keep_original_offsets_and_neighbor_bytes() {
    let before = information_register()
        .trim_start_matches('\u{feff}')
        .to_string();
    let mut operation = flag("StringDimension", "Master", true);
    let MetaEditOperation::Update { elements, .. } = &mut operation else {
        unreachable!()
    };
    elements[0].main_filter = Some(true);
    elements[0].deny_incomplete_values = Some(true);
    let mut expected = before.clone();
    for tag in ["Master", "MainFilter", "DenyIncompleteValues"] {
        expected = expected.replacen(
            &format!("<{tag}>false</{tag}>"),
            &format!("<{tag}>true</{tag}>"),
            1,
        );
    }
    assert_eq!(apply(&before, operation.clone()).unwrap(), expected);
    assert_eq!(apply(&expected, operation.clone()).unwrap(), expected);
    let mut decorated = before.clone();
    let mut expected_decorated = before.clone();
    for (tag, input, changed) in [
        ("Master", "fa<!--keep-->lse", "true<!--keep-->"),
        ("MainFilter", "f<![CDATA[al]]>se", "true<![CDATA[]]>"),
        (
            "DenyIncompleteValues",
            "&#32;f&#97;lse&#x20;",
            "&#32;true&#x20;",
        ),
    ] {
        let original = format!("<{tag}>false</{tag}>");
        decorated = decorated.replacen(&original, &format!("<{tag}>{input}</{tag}>"), 1);
        expected_decorated =
            expected_decorated.replacen(&original, &format!("<{tag}>{changed}</{tag}>"), 1);
    }
    assert_eq!(
        apply(&decorated, operation.clone()).unwrap(),
        expected_decorated
    );
    assert_eq!(
        apply(&expected_decorated, operation).unwrap(),
        expected_decorated
    );
}

#[test]
fn medium_information_boolean_edits_preserve_split_text_markup_and_reader_values() {
    for tag in ["Master", "MainFilter", "DenyIncompleteValues"] {
        for (input, changed) in [
            ("false<!-- trailing -->", "true<!-- trailing -->"),
            ("<!-- </Properties> -->false", "<!-- </Properties> -->true"),
            (
                "<?keep </Properties> ?>false",
                "<?keep </Properties> ?>true",
            ),
            ("fa<!-- middle -->lse", "true<!-- middle -->"),
            ("f<?keep value?>alse", "true<?keep value?>"),
            ("<![CDATA[false]]>", "<![CDATA[true]]>"),
            ("f<![CDATA[al]]>se", "true<![CDATA[]]>"),
            ("<![CDATA[f]]>alse", "<![CDATA[true]]>"),
            ("f&#97;lse", "true"),
            ("&#x20;f&#97;lse&#32;", "&#x20;true&#32;"),
            (
                "<![CDATA[fa]]><![CDATA[lse]]>",
                "<![CDATA[true]]><![CDATA[]]>",
            ),
            (
                " \n<![CDATA[ fa]]><!--keep-->lse\t",
                " \n<![CDATA[ true]]><!--keep-->\t",
            ),
        ] {
            let before = information_register()
                .trim_start_matches('\u{feff}')
                .replacen(
                    &format!("<{tag}>false</{tag}>"),
                    &format!("<{tag}>{input}</{tag}>"),
                    1,
                );
            let expected = before.replacen(
                &format!("<{tag}>{input}</{tag}>"),
                &format!("<{tag}>{changed}</{tag}>"),
                1,
            );
            let read = |xml: &str| {
                let target = unica_address::source_target::MetadataAddress::parse(
                    unica_address::source_target::PLATFORM_XML_8_3_27_FORMAT_2_20,
                    "InformationRegister.Facts",
                )
                .unwrap();
                let info = crate::info::parse_typed_meta_local_info(
                    xml.as_bytes(),
                    &target,
                    unica_metadata::MetaSupportStatus::Unsupported,
                )
                .unwrap();
                let dimension = info
                    .collections
                    .dimensions
                    .iter()
                    .find(|dimension| dimension.name == "StringDimension")
                    .unwrap();
                match tag {
                    "Master" => dimension.master,
                    "MainFilter" => dimension.main_filter,
                    _ => dimension.deny_incomplete_values,
                }
            };
            assert_eq!(read(&before), Some(false), "{tag}: {input}");
            assert_eq!(
                apply(&before, flag("StringDimension", tag, false))
                    .unwrap_or_else(|error| panic!("no-op {tag}: {input}: {error:?}")),
                before,
                "reader-valid no-op: {tag}: {input}"
            );
            let after = apply(&before, flag("StringDimension", tag, true))
                .unwrap_or_else(|error| panic!("change {tag}: {input}: {error:?}"));
            assert_eq!(after, expected, "only boolean text changes: {tag}: {input}");
            assert_eq!(read(&after), Some(true), "{tag}: {input}");
            assert_eq!(
                apply(&after, flag("StringDimension", tag, true)).unwrap(),
                after
            );
            let reverted = apply(&after, flag("StringDimension", tag, false)).unwrap();
            assert_eq!(read(&reverted), Some(false), "{tag}: {input}");
        }
    }
}

#[test]
fn a_flag_and_missing_comment_share_the_full_properties_container() {
    let mut before = information_register()
        .trim_start_matches('\u{feff}')
        .to_string();
    let dimension = before.find("<Name>StringDimension</Name>").unwrap();
    let comment = dimension + before[dimension..].find("<Comment/>").unwrap();
    before.replace_range(comment..comment + "<Comment/>".len(), "");
    before = before.replacen(
        "<MetaDataObject ",
        "<MetaDataObject xmlns:m=\"http://v8.1c.ru/8.3/MDClasses\" ",
        1,
    );
    before = before.replacen(
        "<Master>false</Master>",
        "<m:Master><!-- </Properties> -->false</m:Master>",
        1,
    );
    let mut operation = flag("StringDimension", "Master", true);
    let MetaEditOperation::Update { elements, .. } = &mut operation else {
        unreachable!()
    };
    elements[0].comment = Some("inserted comment".into());
    let after = apply(&before, operation).unwrap();
    assert!(after.contains("<m:Master><!-- </Properties> -->true</m:Master>"));
    let document = roxmltree::Document::parse(&after).unwrap();
    let properties = document
        .descendants()
        .find(|node| node.has_tag_name("Name") && node.text() == Some("StringDimension"))
        .unwrap()
        .parent()
        .unwrap();
    let comments: Vec<_> = properties
        .children()
        .filter(|node| node.has_tag_name("Comment"))
        .collect();
    assert_eq!(comments.len(), 1);
    assert_eq!(comments[0].text(), Some("inserted comment"));
    let expected_flag_only = before.replacen(
        "<m:Master><!-- </Properties> -->false</m:Master>",
        "<m:Master><!-- </Properties> -->true</m:Master>",
        1,
    );
    let inserted = "\t\t\t\t\t<Comment>inserted comment</Comment>\n\t\t\t\t\t";
    assert_eq!(after.matches(inserted).count(), 1);
    // Remove only the new property and its own indentation, preserving every old byte.
    assert_eq!(after.replacen(inserted, "", 1), expected_flag_only);
}

#[test]
fn flags_and_reduction_mode_share_original_ranges_in_either_xml_order() {
    for mode_first in [false, true] {
        for old_flag in [false, true] {
            let old_mode = if old_flag { "Deny" } else { "TransformValues" };
            let new_mode = if old_flag { "TransformValues" } else { "Deny" };
            let mut before = information_register()
                .trim_start_matches('\u{feff}')
                .to_string();
            if mode_first {
                let property = "<TypeReductionMode>TransformValues</TypeReductionMode>";
                before = before.replacen(property, "", 1).replacen(
                    "<Master>false</Master>",
                    &format!("{property}<Master>false</Master>"),
                    1,
                );
            }
            if old_flag {
                for tag in ["Master", "MainFilter", "DenyIncompleteValues"] {
                    before = before.replacen(
                        &format!("<{tag}>false</{tag}>"),
                        &format!("<{tag}>true</{tag}>"),
                        1,
                    );
                }
                before = before.replacen(
                    "<TypeReductionMode>TransformValues</TypeReductionMode>",
                    "<TypeReductionMode>Deny</TypeReductionMode>",
                    1,
                );
            }
            let mut operation = flag("StringDimension", "Master", !old_flag);
            let MetaEditOperation::Update { elements, .. } = &mut operation else {
                unreachable!()
            };
            elements[0].main_filter = Some(!old_flag);
            elements[0].deny_incomplete_values = Some(!old_flag);
            elements[0].type_reduction_mode = Some(new_mode.into());
            let mut expected = before.replacen(
                &format!("<TypeReductionMode>{old_mode}</TypeReductionMode>"),
                &format!("<TypeReductionMode>{new_mode}</TypeReductionMode>"),
                1,
            );
            for tag in ["Master", "MainFilter", "DenyIncompleteValues"] {
                expected = expected.replacen(
                    &format!("<{tag}>{old_flag}</{tag}>"),
                    &format!("<{tag}>{}</{tag}>", !old_flag),
                    1,
                );
            }
            let after = apply(&before, operation.clone()).unwrap();
            assert_eq!(
                after, expected,
                "mode_first={mode_first}, old_flag={old_flag}"
            );
            assert_eq!(apply(&after, operation.clone()).unwrap(), after);
            let MetaEditOperation::Update { elements, .. } = &mut operation else {
                unreachable!()
            };
            elements[0].master = Some(old_flag);
            elements[0].main_filter = Some(old_flag);
            elements[0].deny_incomplete_values = Some(old_flag);
            elements[0].type_reduction_mode = Some(old_mode.into());
            assert_eq!(apply(&after, operation).unwrap(), before);
        }
    }
}
