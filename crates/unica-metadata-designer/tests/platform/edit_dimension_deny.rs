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
            master: None,
            main_filter: None,
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
        "<DenyIncompleteValues>false<!-- comment --></DenyIncompleteValues>",
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
            format!("<{tag}>false<!-- extra --></{tag}>"),
            format!("<{tag}><Nested>false</Nested></{tag}>"),
            format!("<{tag}>1</{tag}>"),
            format!("<{tag}/>"),
            format!("<{tag}><![CDATA[false]]></{tag}>"),
            format!("<{tag}>f&#97;lse</{tag}>"),
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
    assert_eq!(apply(&expected, operation).unwrap(), expected);
}
