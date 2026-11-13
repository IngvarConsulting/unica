use super::*;
use unica_source_contract::format_identity::PlatformLine;
use unica_workspace::test_support::tree_snapshot;

/// The root of an `all_kinds` export at `source` with its languages.
fn all_kinds_root(source: &str) -> Vec<(&'static str, String)> {
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join(source);
    [
        "Configuration.xml",
        "Languages/Русский.xml",
        "Languages/English.xml",
    ]
    .into_iter()
    .map(|relative| {
        (
            relative,
            std::fs::read_to_string(source.join(relative)).unwrap(),
        )
    })
    .collect()
}

fn fixture(source: &str, line: PlatformLine) -> MetadataFixture {
    let files = all_kinds_root(source);
    let files = files
        .iter()
        .map(|(relative, text)| (*relative, text.as_str()))
        .collect::<Vec<_>>();
    MetadataFixture::with_profile_and_files(
        SourceSetKind::Configuration,
        SourceProfile::platform_xml_of_line(line).unwrap(),
        &files,
    )
}

/// rule: INV.SOURCE.PROFILE-CAPABILITIES
#[test]
fn a_set_of_line_8_5_1_edits_the_root_properties_that_line_adds() {
    let fixture = fixture(
        "../../tests/fixtures/formats/platform_xml/2.21/all_kinds/src",
        PlatformLine::new(8, 5, 1),
    );
    let staged = plan_all(
        &fixture,
        &[(
            "props.set",
            json!({"at":"main:Configuration","values":{
                "ClientApplicationTheme":"Dark",
                "mainClientApplicationWindowInterfaceVariant":"NavigationTop",
                "ClientApplicationWindowsOpenVariant":"OpenDataInTabs",
                "Version85InterfaceMigrationMode":"Use",
                "InterfaceCompatibilityMode":"Version8_5",
                "Caption":"Учёт",
            }}),
        )],
    )
    .unwrap();
    let text = &staged_texts(&staged)[Path::new("Configuration.xml")];
    for element in [
        "<ClientApplicationTheme>Dark</ClientApplicationTheme>",
        "<MainClientApplicationWindowInterfaceVariant>NavigationTop</MainClientApplicationWindowInterfaceVariant>",
        "<ClientApplicationWindowsOpenVariant>OpenDataInTabs</ClientApplicationWindowsOpenVariant>",
        "<Version85InterfaceMigrationMode>Use</Version85InterfaceMigrationMode>",
        "<InterfaceCompatibilityMode>Version8_5</InterfaceCompatibilityMode>",
        "<ShortCaption/>",
    ] {
        assert!(text.contains(element), "{element}");
    }
    assert_eq!(
        ml_values(text, "LanguageAwareParity", "Caption"),
        vec![("ru".to_string(), "Учёт".to_string())]
    );
}

/// rule: INV.SOURCE.PROFILE-CAPABILITIES
#[test]
fn a_set_of_line_8_3_27_refuses_what_a_later_line_adds_without_writing() {
    let fixture = fixture(
        "../../tests/fixtures/formats/platform_xml/2.20/all_kinds/src",
        PlatformLine::new(8, 3, 27),
    );
    let before = tree_snapshot(fixture.binding.source_root());
    for (values, path, message) in [
        (
            json!({"Comment":"x","ClientApplicationTheme":"Dark"}),
            "ops[0].args.values.ClientApplicationTheme",
            "Property `ClientApplicationTheme` exists from platform 8.5.1 (format 2.21); source set `main` is in format 2.20 (platform 8.3.27).",
        ),
        (
            json!({"interfaceCompatibilityMode":"Version8_5"}),
            "ops[0].args.values.interfaceCompatibilityMode",
            "Value `Version8_5` of `InterfaceCompatibilityMode` exists from platform 8.5.1 (format 2.21); source set `main` is in format 2.20 (platform 8.3.27).",
        ),
    ] {
        let error = parse_metadata_plan_operation(
            "props.set",
            &json!({"at":"main:Configuration","values":values}),
            0,
            &fixture.binding,
        )
        .unwrap_err();
        assert_eq!(error.kind(), ApplyPlanErrorKind::BadValue);
        assert_eq!(error.path(), Some(path));
        assert_eq!(error.to_string(), message);
    }
    // A value of the set's own line still edits.
    assert!(plan_all(
        &fixture,
        &[(
            "props.set",
            json!({"at":"main:Configuration","values":{"InterfaceCompatibilityMode":"Taxi"}}),
        )],
    )
    .is_ok());
    assert_eq!(tree_snapshot(fixture.binding.source_root()), before);
}
