use super::apply_typed_operations_to_image_with_seed;
use unica_metadata::{MetaCollection, MetaDiagnosticCode, MetaEditOperation, MetaScope};

/// The 8.3.27 export of `workspace-tabular-attribute-copy`: a catalog whose
/// tabular attribute carries ru/en synonym and tooltip and a composite type.
fn catalog() -> String {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tests/fixtures/acceptance/workspace-tabular-attribute-copy/src/Catalogs/Валюты.xml"
    );
    String::from_utf8(std::fs::read(path).unwrap()).unwrap()
}

fn copy(name: &str, new_name: &str) -> MetaEditOperation {
    MetaEditOperation::Copy {
        collection: MetaCollection::Attributes,
        scope: Some(MetaScope {
            tabular_section: "Представления".to_string(),
        }),
        name: name.to_string(),
        new_name: new_name.to_string(),
    }
}

fn apply(
    xml: &str,
    operation: MetaEditOperation,
) -> Result<String, (MetaDiagnosticCode, Option<String>)> {
    let mut text = xml.to_string();
    apply_typed_operations_to_image_with_seed(&mut text, &[operation], "", b"copy-seed")
        .map(|()| text)
        .map_err(|failure| {
            let diagnostic = failure.diagnostics.into_iter().next().unwrap();
            let field = diagnostic
                .field
                .map(|field| field.trim_start_matches("operations[0].").to_string());
            (diagnostic.code, field)
        })
}

/// The element text of the tabular attribute `name`.
fn tabular_attribute(xml: &str, name: &str) -> String {
    let section = xml.find("<TabularSection").unwrap();
    let at = section
        + xml[section..]
            .find(&format!("<Name>{name}</Name>"))
            .unwrap();
    let start = xml[..at].rfind("<Attribute uuid=").unwrap();
    let end = at + xml[at..].find("</Attribute>").unwrap() + "</Attribute>".len();
    xml[start..end].to_string()
}

fn uuid_of(element: &str) -> &str {
    let start = element.find("uuid=\"").unwrap() + "uuid=\"".len();
    &element[start..start + 36]
}

#[test]
fn a_tabular_attribute_copy_keeps_every_setting_and_translation_under_a_new_identity() {
    let before = catalog();
    let source = tabular_attribute(&before, "ПараметрыПрописи");
    let after = apply(&before, copy("ПараметрыПрописи", "ПараметрыПрописиКопия")).unwrap();
    let copied = tabular_attribute(&after, "ПараметрыПрописиКопия");
    assert_ne!(uuid_of(&copied), uuid_of(&source));
    assert_eq!(
        copied,
        source
            .replacen(uuid_of(&source), uuid_of(&copied), 1)
            .replacen(
                "<Name>ПараметрыПрописи</Name>",
                "<Name>ПараметрыПрописиКопия</Name>",
                1
            )
    );
    for text in [
        "Amount in words settings",
        "How the amount is spelled out",
        "<v8:Type>xs:decimal</v8:Type>",
        "<FillChecking>ShowError</FillChecking>",
    ] {
        assert!(copied.contains(text), "{text}");
    }
    // The copy follows the last member with the line ending in front of it;
    // the rest of the descriptor, the source included, keeps its bytes.
    let last_end = before.rfind(&source).unwrap() + source.len();
    let inserted = format!("\r\n\t\t\t\t\t{copied}");
    assert_eq!(
        after,
        format!("{}{inserted}{}", &before[..last_end], &before[last_end..])
    );
    roxmltree::Document::parse(after.trim_start_matches('\u{feff}')).unwrap();
    // The copy is seeded: the same plan gives the same UUID.
    assert_eq!(
        apply(&before, copy("ПараметрыПрописи", "ПараметрыПрописиКопия")).unwrap(),
        after
    );
}

#[test]
fn a_tabular_attribute_copy_refuses_without_writing_what_it_cannot_copy_unambiguously() {
    let before = catalog();
    let source = tabular_attribute(&before, "ПараметрыПрописи");
    let with = |replacement: &str| before.replacen(&source, replacement, 1);
    let cases = [
        (
            before.clone(),
            copy("ПараметрыПрописи", "КодЯзыка"),
            MetaDiagnosticCode::InvalidArguments,
            Some("values.name"),
        ),
        (
            before.clone(),
            copy("Нет", "Копия"),
            MetaDiagnosticCode::TargetNotFound,
            None,
        ),
        (
            with(
                &source
                    .replacen("<Comment/>", "<Comment>Мой uuid ".to_string().as_str(), 1)
                    .replacen(
                        "<Comment>Мой uuid ",
                        &format!("<Comment>{}</Comment>", uuid_of(&source)),
                        1,
                    ),
            ),
            copy("ПараметрыПрописи", "Копия"),
            MetaDiagnosticCode::InvalidArguments,
            Some("at"),
        ),
        (
            with(&source.replacen(
                "<ChoiceParameters/>",
                "<ChoiceParameters><app:item xmlns:app=\"urn:x\" id=\"1\"/></ChoiceParameters>",
                1,
            )),
            copy("ПараметрыПрописи", "Копия"),
            MetaDiagnosticCode::InvalidArguments,
            Some("at"),
        ),
    ];
    for (input, operation, code, field) in cases {
        let (actual, actual_field) = apply(&input, operation.clone()).unwrap_err();
        assert_eq!(actual, code, "{operation:?}");
        assert_eq!(actual_field.as_deref(), field, "{operation:?}");
    }
}

#[test]
fn a_copy_of_a_member_that_is_not_last_follows_the_last_and_keeps_references_to_neighbours() {
    let before = catalog();
    let first = tabular_attribute(&before, "КодЯзыка");
    let last = tabular_attribute(&before, "ПараметрыПрописи");
    // The neighbour path and a longer name that starts with the source's own
    // are references to other members and stay as written.
    let neighbour = "Catalog.Валюты.TabularSection.Представления.Attribute.ПараметрыПрописи";
    let longer = "Catalog.Валюты.TabularSection.Представления.Attribute.КодЯзыкаДоп";
    let source = first.replacen(
        "<Comment/>",
        &format!("<Comment>{neighbour} {longer}</Comment>"),
        1,
    );
    let input = before.replacen(&first, &source, 1);
    let after = apply(&input, copy("кодязыка", "КодЯзыкаКопия")).unwrap();
    let copied = tabular_attribute(&after, "КодЯзыкаКопия");
    assert!(copied.contains(&format!("<Comment>{neighbour} {longer}</Comment>")));
    let last_end = input.rfind(&last).unwrap() + last.len();
    assert_eq!(
        after,
        format!(
            "{}\r\n\t\t\t\t\t{copied}{}",
            &input[..last_end],
            &input[last_end..]
        )
    );

    // The own path, spelled in another case by the caller, is still the
    // source's own reference.
    let own = "Catalog.Валюты.TabularSection.Представления.Attribute.КодЯзыка";
    let input = before.replacen(
        &first,
        &first.replacen("<Comment/>", &format!("<Comment>{own}</Comment>"), 1),
        1,
    );
    let mut lowered = copy("КодЯзыка", "Копия");
    if let MetaEditOperation::Copy { scope, .. } = &mut lowered {
        *scope = Some(MetaScope {
            tabular_section: "представления".to_string(),
        });
    }
    let (code, field) = apply(&input, lowered).unwrap_err();
    assert_eq!(code, MetaDiagnosticCode::InvalidArguments);
    assert_eq!(field.as_deref(), Some("at"));
}
