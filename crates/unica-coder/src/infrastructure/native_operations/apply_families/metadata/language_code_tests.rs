use super::*;

fn make_fixture(extra: &[(&str, &str)]) -> MetadataFixture {
    let configuration = RU_CONFIGURATION_XML.replace(
        "<Language>Русский</Language>",
        "<Language>Русский</Language><Language>English</Language>",
    );
    let english = language_xml("English", "en").replace(
        "33333333-3333-4333-8333-333333333333",
        "16520000-0000-4000-8000-000000000002",
    );
    let russian = language_xml("Русский", "ru");
    let mut files = vec![
        ("Configuration.xml", configuration.as_str()),
        ("Languages/Русский.xml", russian.as_str()),
        ("Languages/English.xml", english.as_str()),
        ("Documents/Order.xml", ORDER_XML),
    ];
    files.extend_from_slice(extra);
    MetadataFixture::with_files(&files)
}
fn prepare(fixture: &MetadataFixture, code: &str) -> unica_workspace::apply::ApplyStagedState {
    plan_all(
        fixture,
        &[(
            "props.set",
            json!({"at":"main:Language.English","values":{"LanguageCode":code}}),
        )],
    )
    .unwrap()
}
fn validate(
    staged: &mut unica_workspace::apply::ApplyStagedState,
) -> Result<(), unica_source_contract::proposal::ApplyPlanError> {
    unica_metadata_designer::planner::validate_final_object_integrity(
        &mut staged.proposal(),
        "main",
        SourceSetKind::Configuration,
    )
}

#[test]
fn language_code_changes_only_the_unused_registered_additional_language() {
    let fixture = make_fixture(&[]);
    let before =
        std::fs::read(fixture.binding.source_root().join("Languages/English.xml")).unwrap();
    let mut staged = prepare(&fixture, "de");
    validate(&mut staged).unwrap();
    let texts = staged_texts(&staged);
    let updated = &texts[Path::new("Languages/English.xml")];
    assert_eq!(
        updated,
        &String::from_utf8(before.clone()).unwrap().replace(
            "<LanguageCode>en</LanguageCode>",
            "<LanguageCode>de</LanguageCode>"
        )
    );
    assert_eq!(
        std::fs::read(fixture.binding.source_root().join("Languages/English.xml")).unwrap(),
        before
    );
    let mut unchanged = prepare(&fixture, "en");
    validate(&mut unchanged).unwrap();
    assert!(unchanged.planned_changes().is_empty());
}

#[test]
fn language_code_refuses_xml_nstr_and_help_dependencies_before_publication() {
    for (path, text) in [
        (
            "CommonModules/Texts/Ext/Module.bsl",
            "Procedure Run()\nA = НСтр(\"ru='текст'; en='text'\");\nEndProcedure",
        ),
        (
            "Catalogs/Unused.xml",
            "<Title xmlns:v8=\"http://v8.1c.ru/8.1/data/core\"><v8:item><v8:lang>en</v8:lang><v8:content>text</v8:content></v8:item></Title>",
        ),
        (
            "Catalogs/Unused/Ext/Help.xml",
            "<Help xmlns=\"http://v8.1c.ru/8.3/xcf/extrnprops\"><Page>en</Page></Help>",
        ),
        ("Catalogs/Unused/Ext/Help/en.html", "<html>Text</html>"),
    ] {
        let fixture = make_fixture(&[(path, text)]);
        let mut staged = prepare(&fixture, "de");
        let error = validate(&mut staged).unwrap_err();
        assert_eq!(error.kind(), ApplyPlanErrorKind::BadValue);
        assert!(
            std::fs::read_to_string(fixture.binding.source_root().join("Languages/English.xml"))
                .unwrap()
                .contains("<LanguageCode>en</LanguageCode>")
        );
    }
}

#[test]
fn language_code_final_guard_sees_late_created_sources_and_original_dependencies() {
    let fixture = make_fixture(&[]);
    let mut staged = prepare(&fixture, "de");
    staged
        .proposal()
        .create(
            "CommonModules/Late/Ext/Module.bsl",
            b"Procedure Run()\nA = NStr(\"en='late'\");\nEndProcedure".to_vec(),
        )
        .unwrap();
    assert!(
        validate(&mut staged)
            .unwrap_err()
            .to_string()
            .contains("NStr")
    );
    let xml = "<Title xmlns:v8=\"http://v8.1c.ru/8.1/data/core\"><v8:item><v8:lang>en</v8:lang><v8:content>text</v8:content></v8:item></Title>";
    let used = make_fixture(&[("Catalogs/Unused.xml", xml)]);
    let mut staged = prepare(&used, "de");
    staged
        .proposal()
        .replace("Catalogs/Unused.xml", xml.as_bytes(), b"<Title/>".to_vec())
        .unwrap();
    assert!(
        validate(&mut staged)
            .unwrap_err()
            .to_string()
            .contains("XML translations")
    );
    let independent = make_fixture(&[]);
    let mut staged = prepare(&independent, "de");
    staged
        .proposal()
        .create(
            "CommonModules/Late/Ext/Module.bsl",
            b"Procedure Run()\nA = NStr(\"ru='independent'\");\nEndProcedure".to_vec(),
        )
        .unwrap();
    validate(&mut staged).unwrap();
}

#[test]
fn language_code_refuses_default_conflict_or_unregistered_owner() {
    let fixture = make_fixture(&[]);
    let mut conflicting = prepare(&fixture, "RU");
    assert!(
        validate(&mut conflicting)
            .unwrap_err()
            .to_string()
            .contains("conflicts")
    );
    let configuration = RU_CONFIGURATION_XML.to_string();
    let orphan = MetadataFixture::with_files(&[
        ("Configuration.xml", &configuration),
        ("Languages/Русский.xml", &language_xml("Русский", "ru")),
        ("Languages/English.xml", &language_xml("English", "en")),
    ]);
    assert!(
        validate(&mut prepare(&orphan, "de"))
            .unwrap_err()
            .to_string()
            .contains("not registered")
    );
    let mut default = plan_all(
        &fixture,
        &[(
            "props.set",
            json!({"at":"main:Language.Русский","values":{"LanguageCode":"de"}}),
        )],
    )
    .unwrap();
    assert!(
        validate(&mut default)
            .unwrap_err()
            .to_string()
            .contains("DefaultLanguage")
    );
}
