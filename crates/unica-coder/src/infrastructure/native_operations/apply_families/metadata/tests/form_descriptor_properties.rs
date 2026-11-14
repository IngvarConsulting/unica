use super::*;
use unica_source_contract::format_identity::PlatformLine;
use unica_workspace::test_support::tree_snapshot;

const LIST_FORM: &str = "Catalogs/Валюты/Forms/ФормаСписка.xml";
const COMMON_FORM: &str = "CommonForms/MainA.xml";
const LIST_FORM_AT: &str = "main:Catalog.Валюты.Form.ФормаСписка.Metadata";
const COMMON_FORM_AT: &str = "main:CommonForm.MainA.Metadata";

fn export(format: &str) -> PathBuf {
    let root = match format {
        "2.20" => "../../tests/fixtures/formats/platform_xml/2.20/all_kinds/src",
        "2.21" => "../../tests/fixtures/formats/platform_xml/2.21/all_kinds/src",
        other => panic!("no all_kinds export of format {other}"),
    };
    Path::new(env!("CARGO_MANIFEST_DIR")).join(root)
}

fn read(format: &str, relative: &str) -> String {
    std::fs::read_to_string(export(format).join(relative)).unwrap()
}

/// The configuration root of an `all_kinds` export of `format` with one
/// catalog, its list form and a common form.
fn fixture(format: &str, line: PlatformLine) -> MetadataFixture {
    fixture_with(format, line, &[])
}

fn fixture_with(format: &str, line: PlatformLine, replaced: &[(&str, String)]) -> MetadataFixture {
    let files = [
        "Configuration.xml",
        "Languages/Русский.xml",
        "Languages/English.xml",
        "Catalogs/Валюты.xml",
        LIST_FORM,
        COMMON_FORM,
    ]
    .into_iter()
    .map(|relative| {
        let text = replaced
            .iter()
            .find(|(path, _)| *path == relative)
            .map(|(_, text)| text.clone())
            .unwrap_or_else(|| read(format, relative));
        (relative, text)
    })
    .collect::<Vec<_>>();
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

/// The refusal of `props.set` at `at`, by the argument decoder or by the
/// planner that reads the descriptor.
fn refusal(
    fixture: &MetadataFixture,
    at: &str,
    values: serde_json::Value,
) -> (ApplyPlanErrorKind, Option<String>, String) {
    let args = json!({"at":at,"values":values});
    let error = match parse_metadata_plan_operation("props.set", &args, 0, &fixture.binding) {
        Err(error) => error,
        Ok(_) => plan_all(fixture, &[("props.set", args.clone())])
            .err()
            .unwrap_or_else(|| panic!("{args} must be refused")),
    };
    (
        error.kind(),
        error.path().map(str::to_string),
        error.to_string(),
    )
}

/// rule: INV.SOURCE.PROFILE-CAPABILITIES
#[test]
fn a_set_of_line_8_5_1_writes_the_descriptor_properties_of_its_forms() {
    let fixture = fixture("2.21", PlatformLine::new(8, 5, 1));
    let staged = plan_all(
        &fixture,
        &[
            (
                "props.set",
                json!({"at":LIST_FORM_AT,"values":{
                    "Comment":"Список",
                    "includeHelpInContents":true,
                    "UsePurposes":["MobilePlatformApplication"],
                    "UseInInterfaceCompatibilityMode":"Taxi",
                }}),
            ),
            (
                "props.set",
                json!({"at":COMMON_FORM_AT,"values":{
                    "usePurposes":[],
                    "useInInterfaceCompatibilityMode":"Version85",
                }}),
            ),
        ],
    )
    .unwrap();
    let texts = staged_texts(&staged);
    let list = read("2.21", LIST_FORM)
        .replace("<Comment/>", "<Comment>Список</Comment>")
        .replace(
            "<IncludeHelpInContents>false</IncludeHelpInContents>",
            "<IncludeHelpInContents>true</IncludeHelpInContents>",
        )
        .replace(
            "\r\n\t\t\t\t<v8:Value xsi:type=\"app:ApplicationUsePurpose\">PlatformApplication</v8:Value>",
            "",
        )
        .replace(
            "<UseInInterfaceCompatibilityMode>Any</UseInInterfaceCompatibilityMode>",
            "<UseInInterfaceCompatibilityMode>Taxi</UseInInterfaceCompatibilityMode>",
        );
    assert_eq!(texts[Path::new(LIST_FORM)], list);
    let common = read("2.21", COMMON_FORM);
    let start = common.find("<UsePurposes>").unwrap();
    let end = common.find("</UsePurposes>").unwrap() + "</UsePurposes>".len();
    let mut expected = common.clone();
    expected.replace_range(start..end, "<UsePurposes/>");
    let expected = expected.replace(
        "<UseInInterfaceCompatibilityMode>Any</UseInInterfaceCompatibilityMode>",
        "<UseInInterfaceCompatibilityMode>Version85</UseInInterfaceCompatibilityMode>",
    );
    assert_eq!(texts[Path::new(COMMON_FORM)], expected);
    assert_eq!(texts.len(), 2, "{:?}", texts.keys().collect::<Vec<_>>());
}

/// rule: INV.SOURCE.PROFILE-CAPABILITIES
#[test]
fn a_set_of_line_8_3_27_refuses_the_compatibility_mode_of_a_form_without_writing() {
    let fixture = fixture("2.20", PlatformLine::new(8, 3, 27));
    let before = tree_snapshot(fixture.binding.source_root());
    for at in [LIST_FORM_AT, COMMON_FORM_AT] {
        for name in [
            "UseInInterfaceCompatibilityMode",
            "useInInterfaceCompatibilityMode",
        ] {
            assert_eq!(
                refusal(&fixture, at, json!({"IncludeHelpInContents":true, name:"Any"})),
                (
                    ApplyPlanErrorKind::BadValue,
                    Some(format!("ops[0].args.values.{name}")),
                    "Property `UseInInterfaceCompatibilityMode` exists from platform 8.5.1 (format 2.21); source set `main` is in format 2.20 (platform 8.3.27).".to_string()
                ),
                "{at}"
            );
        }
    }
    // The properties of the set's own line still edit.
    let staged = plan_all(
        &fixture,
        &[(
            "props.set",
            json!({"at":LIST_FORM_AT,"values":{"IncludeHelpInContents":true,"UsePurposes":["PlatformApplication"]}}),
        )],
    )
    .unwrap();
    assert!(staged_texts(&staged)[Path::new(LIST_FORM)]
        .contains("<IncludeHelpInContents>true</IncludeHelpInContents>"));
    assert_eq!(tree_snapshot(fixture.binding.source_root()), before);
}

#[test]
fn the_type_of_a_form_is_read_only() {
    let fixture = fixture("2.21", PlatformLine::new(8, 5, 1));
    let before = tree_snapshot(fixture.binding.source_root());
    for at in [LIST_FORM_AT, COMMON_FORM_AT] {
        for name in ["FormType", "formType"] {
            assert_eq!(
                refusal(&fixture, at, json!({"Comment":"x", name:"Ordinary"})),
                (
                    ApplyPlanErrorKind::BadValue,
                    Some(format!("ops[0].args.values.{name}")),
                    "property `FormType` is read only here: a form's type changes together with its content".to_string()
                ),
                "{at}"
            );
        }
    }
    assert_eq!(tree_snapshot(fixture.binding.source_root()), before);
}

#[test]
fn values_equal_to_the_present_ones_change_no_byte() {
    let fixture = fixture("2.21", PlatformLine::new(8, 5, 1));
    let staged = plan_all(
        &fixture,
        &[(
            "props.set",
            json!({"at":LIST_FORM_AT,"values":{
                "Comment":"",
                "IncludeHelpInContents":false,
                "UsePurposes":["MobilePlatformApplication","PlatformApplication"],
                "UseInInterfaceCompatibilityMode":"Any",
            }}),
        )],
    )
    .unwrap();
    assert!(staged_texts(&staged).is_empty());
}

#[test]
fn closed_values_and_repeated_purposes_are_refused() {
    let fixture = fixture("2.21", PlatformLine::new(8, 5, 1));
    for (values, path) in [
        (
            json!({"UsePurposes":["Desktop"]}),
            "ops[0].args.values.UsePurposes",
        ),
        (
            json!({"UsePurposes":["PlatformApplication","PlatformApplication"]}),
            "ops[0].args.values.UsePurposes",
        ),
        (
            json!({"UsePurposes":"PlatformApplication"}),
            "ops[0].args.values.UsePurposes",
        ),
        (
            json!({"UseInInterfaceCompatibilityMode":"Version8_5"}),
            "ops[0].args.values.UseInInterfaceCompatibilityMode",
        ),
        (
            json!({"IncludeHelpInContents":"true"}),
            "ops[0].args.values.IncludeHelpInContents",
        ),
        (
            json!({"Comment":"a","comment":"b"}),
            "ops[0].args.values.comment",
        ),
    ] {
        let (kind, error_path, message) = refusal(&fixture, LIST_FORM_AT, values.clone());
        assert_eq!(kind, ApplyPlanErrorKind::BadValue, "{values}: {message}");
        assert!(
            error_path
                .as_deref()
                .is_some_and(|error_path| error_path.starts_with(path)),
            "{values}: {error_path:?} {message}"
        );
    }
}

#[test]
fn a_form_its_owner_does_not_register_is_not_found() {
    let owner =
        read("2.21", "Catalogs/Валюты.xml").replace("\t\t\t<Form>ФормаСписка</Form>\r\n", "");
    let fixture = fixture_with(
        "2.21",
        PlatformLine::new(8, 5, 1),
        &[("Catalogs/Валюты.xml", owner)],
    );
    let (kind, _, message) = refusal(
        &fixture,
        LIST_FORM_AT,
        json!({"IncludeHelpInContents":true}),
    );
    assert_eq!(
        (kind, message.as_str()),
        (ApplyPlanErrorKind::NotFound, "form is not registered")
    );
}

#[test]
fn a_form_of_an_extension_changes_only_its_comment() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/formats/platform_xml/2.21/empty_extension/ext");
    let files = ["Configuration.xml", "Languages/Русский.xml"].map(|relative| {
        (
            relative,
            std::fs::read_to_string(root.join(relative)).unwrap(),
        )
    });
    let files = files
        .iter()
        .map(|(relative, text)| (*relative, text.as_str()))
        .collect::<Vec<_>>();
    let fixture = MetadataFixture::with_profile_and_files(
        SourceSetKind::Extension,
        SourceProfile::platform_xml_of_line(PlatformLine::new(8, 5, 1)).unwrap(),
        &files,
    );
    for at in [LIST_FORM_AT, COMMON_FORM_AT] {
        assert!(parse_metadata_plan_operation(
            "props.set",
            &json!({"at":at,"values":{"comment":"x"}}),
            0,
            &fixture.binding,
        )
        .is_ok());
        for name in [
            "UsePurposes",
            "IncludeHelpInContents",
            "UseInInterfaceCompatibilityMode",
        ] {
            let error = parse_metadata_plan_operation(
                "props.set",
                &json!({"at":at,"values":{name:[]}}),
                0,
                &fixture.binding,
            )
            .unwrap_err();
            assert_eq!(error.kind(), ApplyPlanErrorKind::BadValue);
            assert_eq!(
                error.path(),
                Some(format!("ops[0].args.values.{name}").as_str())
            );
            assert_eq!(
                error.to_string(),
                format!("property `{name}` is not written for a form of an extension; only `Comment` is")
            );
        }
    }
}

/// The workspace plans no metadata write in an external processor, so
/// `props.set` is not an operation of its form descriptors.
#[test]
fn a_form_of_an_external_processor_has_no_property_edit() {
    let processor = std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join(
        "../../tests/fixtures/formats/platform_xml/2.20/all_kinds/src/DataProcessors/Обработка.xml",
    ))
    .unwrap();
    let fixture = MetadataFixture::with_profile_and_files(
        SourceSetKind::ExternalProcessor,
        SourceProfile::platform_xml_8_3_27_format_2_20(),
        &[("Обработка.xml", processor.as_str())],
    );
    let error = parse_metadata_plan_operation(
        "props.set",
        &json!({"at":"main:ExternalDataProcessor.Обработка.Form.Форма.Metadata","values":{"IncludeHelpInContents":true}}),
        0,
        &fixture.binding,
    )
    .unwrap_err();
    assert_eq!(
        error.kind(),
        ApplyPlanErrorKind::ProviderUnavailable,
        "{error}"
    );
}
