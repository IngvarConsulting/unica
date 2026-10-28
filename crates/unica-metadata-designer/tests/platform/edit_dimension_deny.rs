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
            base_dimension: None,
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
