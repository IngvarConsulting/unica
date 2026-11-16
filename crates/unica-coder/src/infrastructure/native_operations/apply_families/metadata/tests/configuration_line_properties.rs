use super::*;
use unica_source_contract::format_identity::PlatformLine;
use unica_workspace::test_support::tree_snapshot;

/// The root of an `all_kinds` export at `source` with its languages and one
/// enumeration.
fn all_kinds_root(source: &str) -> Vec<(&'static str, String)> {
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join(source);
    [
        "Configuration.xml",
        "Languages/Русский.xml",
        "Languages/English.xml",
        "Enums/СпособыУстановкиКурсаВалюты.xml",
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

const ENUM_VALUE: &str = "main:Enum.СпособыУстановкиКурсаВалюты.EnumValue.ЗагрузкаИзИнтернета";

/// rule: INV.SOURCE.PROFILE-CAPABILITIES
#[test]
fn a_set_of_line_8_5_1_sets_the_color_of_an_enumeration_value() {
    let fixture = fixture(
        "../../tests/fixtures/formats/platform_xml/2.21/all_kinds/src",
        PlatformLine::new(8, 5, 1),
    );
    let staged = plan_all(
        &fixture,
        &[(
            "enumValue.set",
            json!({"at":ENUM_VALUE,"values":{"color":"#ff00aa"}}),
        )],
    )
    .unwrap();
    let texts = staged_texts(&staged);
    let text = &texts[Path::new("Enums/СпособыУстановкиКурсаВалюты.xml")];
    let value = &text[text.find("<Name>ЗагрузкаИзИнтернета</Name>").unwrap()..];
    let value = &value[..value.find("</Properties>").unwrap()];
    assert!(value.contains("<Color>#FF00AA</Color>"), "{value}");
    // The other value keeps its own color.
    assert_eq!(text.matches("<Color>auto</Color>").count(), 3);
    // The platform refuses a color it does not know (checked on 8.5.1).
    for color in ["win:WindowColor", "web:Unknown"] {
        let error = parse_metadata_plan_operation(
            "enumValue.set",
            &json!({"at":ENUM_VALUE,"values":{"color":color}}),
            0,
            &fixture.binding,
        )
        .unwrap_err();
        assert_eq!(error.kind(), ApplyPlanErrorKind::BadValue, "{color}");
    }
}

/// rule: INV.SOURCE.PROFILE-CAPABILITIES
#[test]
fn a_set_of_line_8_3_27_refuses_the_color_of_an_enumeration_value() {
    let fixture = fixture(
        "../../tests/fixtures/formats/platform_xml/2.20/all_kinds/src",
        PlatformLine::new(8, 3, 27),
    );
    let before = tree_snapshot(fixture.binding.source_root());
    let error = parse_metadata_plan_operation(
        "enumValue.set",
        &json!({"at":ENUM_VALUE,"values":{"color":"auto"}}),
        0,
        &fixture.binding,
    )
    .unwrap_err();
    assert_eq!(error.kind(), ApplyPlanErrorKind::BadValue);
    assert_eq!(error.path(), Some("ops[0].args.values.color"));
    assert_eq!(
        error.to_string(),
        "Property `Color` exists from platform 8.5.1 (format 2.21); source set `main` is in format 2.20 (platform 8.3.27)."
    );
    assert_eq!(tree_snapshot(fixture.binding.source_root()), before);
}

/// The root of an `empty_extension` export at `source` as an extension set
/// of platform `line`.
fn extension_fixture(source: &str, line: PlatformLine) -> MetadataFixture {
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join(source);
    let files = ["Configuration.xml", "Languages/Русский.xml"]
        .into_iter()
        .map(|relative| {
            (
                relative,
                std::fs::read_to_string(source.join(relative)).unwrap(),
            )
        })
        .collect::<Vec<_>>();
    let files = files
        .iter()
        .map(|(relative, text)| (*relative, text.as_str()))
        .collect::<Vec<_>>();
    MetadataFixture::with_profile_and_files(
        SourceSetKind::Extension,
        SourceProfile::platform_xml_of_line(line).unwrap(),
        &files,
    )
}

/// rule: INV.SOURCE.PROFILE-CAPABILITIES
#[test]
fn an_extension_of_line_8_5_1_sets_its_caption() {
    let fixture = extension_fixture(
        "../../tests/fixtures/formats/platform_xml/2.21/empty_extension/ext",
        PlatformLine::new(8, 5, 1),
    );
    let staged = plan_all(
        &fixture,
        &[(
            "props.set",
            json!({"at":"main:Configuration","values":{"Caption":"Доработки","shortCaption":"Д"}}),
        )],
    )
    .unwrap();
    let text = &staged_texts(&staged)[Path::new("Configuration.xml")];
    assert_eq!(
        ml_values(text, "CorpusExtension", "Caption"),
        vec![("ru".to_string(), "Доработки".to_string())]
    );
    assert_eq!(
        ml_values(text, "CorpusExtension", "ShortCaption"),
        vec![("ru".to_string(), "Д".to_string())]
    );
}

/// rule: INV.SOURCE.PROFILE-CAPABILITIES
#[test]
fn an_extension_of_line_8_3_27_refuses_a_caption() {
    let fixture = extension_fixture(
        "../../tests/fixtures/formats/platform_xml/2.20/empty_extension/ext",
        PlatformLine::new(8, 3, 27),
    );
    let error = parse_metadata_plan_operation(
        "props.set",
        &json!({"at":"main:Configuration","values":{"Caption":"Доработки"}}),
        0,
        &fixture.binding,
    )
    .unwrap_err();
    assert_eq!(error.kind(), ApplyPlanErrorKind::BadValue);
    assert_eq!(
        error.to_string(),
        "Property `Caption` exists from platform 8.5.1 (format 2.21); source set `main` is in format 2.20 (platform 8.3.27)."
    );
}

/// The answer of `form.set` of the report's auxiliary variant form in a copy
/// of the `all_kinds` export at `source`, set of platform `line`, the report
/// given a form of its own.
fn set_report_variant_form(
    source: &str,
    line: PlatformLine,
) -> Result<
    std::collections::BTreeMap<PathBuf, String>,
    unica_source_contract::proposal::ApplyPlanError,
> {
    hidden_apply(
        source,
        line,
        |main| {
            let report = main.join("Reports/АнализВерсийОбъектов.xml");
            let text = std::fs::read_to_string(&report).unwrap();
            let text = text.replacen(
                "<ChildObjects>",
                "<ChildObjects>\r\n\t\t\t<Form>Вариант</Form>",
                1,
            );
            std::fs::write(&report, text).unwrap();
        },
        json!({"at":"main:Report.АнализВерсийОбъектов","dryRun":true,"ops":[
            {"op":"form.set","args":{"values":{"auxiliaryVariantForm":"Вариант"}}}]}),
    )
}

/// The answer of a dry-run `apply` of `args` in a copy of the export at
/// `source`, set `main` of platform `line`, after `prepare` edits the copy.
fn hidden_apply(
    source: &str,
    line: PlatformLine,
    prepare: impl FnOnce(&Path),
    args: serde_json::Value,
) -> Result<
    std::collections::BTreeMap<PathBuf, String>,
    unica_source_contract::proposal::ApplyPlanError,
> {
    fn copy_tree(from: &Path, to: &Path) {
        std::fs::create_dir_all(to).unwrap();
        for entry in std::fs::read_dir(from).unwrap() {
            let entry = entry.unwrap();
            let target = to.join(entry.file_name());
            if entry.file_type().unwrap().is_dir() {
                copy_tree(&entry.path(), &target);
            } else {
                std::fs::copy(entry.path(), target).unwrap();
            }
        }
    }
    let root = tempfile::tempdir().unwrap();
    let workspace_root = std::fs::canonicalize(root.path()).unwrap();
    let main = workspace_root.join("src");
    copy_tree(&Path::new(env!("CARGO_MANIFEST_DIR")).join(source), &main);
    prepare(&main);
    std::fs::write(
        workspace_root.join("v8project.yaml"),
        "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
    )
    .unwrap();
    let context = WorkspaceContext {
        cwd: workspace_root.clone(),
        workspace_root: workspace_root.clone(),
        cache_root: workspace_root.join(".build/unica"),
        workspace_epoch: 1,
    };
    let identity = WorkspaceIdentity::new(
        &context,
        [WorkspaceSourceSetInput::new(
            "main",
            &main,
            SourceSetKind::Configuration,
            SourceFormat::PlatformXml,
            SourceProfile::platform_xml_of_line(line).unwrap(),
        )],
        "hidden-apply",
    )
    .unwrap();
    let actor = crate::infrastructure::source_workspace::new_actor(identity, context).unwrap();
    let binding = actor.bind_provider_root("main", &main).unwrap();
    let admission = actor
        .admit_apply(
            &binding,
            None,
            true,
            ProviderDeadline::from_budget(Duration::from_secs(5)),
            &CancellationToken::new(),
        )
        .unwrap();
    let request =
        crate::domain::apply::ApplyRequest::parse(args.as_object().unwrap(), &["main"]).unwrap();
    let (staged, _) = super::super::super::plan_hidden_v13_apply(&request, &binding, &admission)?;
    Ok(staged_texts(&staged))
}

/// rule: INV.SOURCE.PROFILE-CAPABILITIES
#[test]
fn a_report_of_line_8_5_1_takes_an_auxiliary_variant_form() {
    let texts = set_report_variant_form(
        "../../tests/fixtures/formats/platform_xml/2.21/all_kinds/src",
        PlatformLine::new(8, 5, 1),
    )
    .unwrap();
    assert!(
        texts[Path::new("Reports/АнализВерсийОбъектов.xml")].contains(
            "<AuxiliaryVariantForm>Report.АнализВерсийОбъектов.Form.Вариант</AuxiliaryVariantForm>"
        )
    );
}

/// rule: INV.SOURCE.PROFILE-CAPABILITIES
#[test]
fn a_report_of_line_8_3_27_refuses_an_auxiliary_variant_form() {
    let error = set_report_variant_form(
        "../../tests/fixtures/formats/platform_xml/2.20/all_kinds/src",
        PlatformLine::new(8, 3, 27),
    )
    .unwrap_err();
    assert_eq!(error.kind(), ApplyPlanErrorKind::BadValue);
    assert_eq!(
        error.to_string(),
        "Property `AuxiliaryVariantForm` exists from platform 8.5.1 (format 2.21); source set `main` is in format 2.20 (platform 8.3.27)."
    );
}

/// `form.set` of `values` at the root of a copy of the `all_kinds` export
/// at `source`, set of platform `line`.
fn set_root_forms(
    source: &str,
    line: PlatformLine,
    values: serde_json::Value,
) -> Result<
    std::collections::BTreeMap<PathBuf, String>,
    unica_source_contract::proposal::ApplyPlanError,
> {
    hidden_apply(
        source,
        line,
        |_| {},
        json!({"at":"main:Configuration","dryRun":true,"ops":[
            {"op":"form.set","args":{"values":values}}]}),
    )
}

/// The configuration root points its default forms, and from 8.5.1 its
/// auxiliary ones, at common forms of the set, and clears them.
/// rule: INV.SOURCE.PROFILE-CAPABILITIES
#[test]
fn a_configuration_of_line_8_5_1_points_its_root_form_slots() {
    let source = "../../tests/fixtures/formats/platform_xml/2.21/all_kinds/src";
    let line = PlatformLine::new(8, 5, 1);
    let texts = set_root_forms(
        source,
        line,
        json!({
            "defaultSearchForm":"MainA",
            "auxiliaryReportForm":"CommonForm.MainA",
            "defaultConstantsForm":"main:CommonForm.MainA",
        }),
    )
    .unwrap();
    let text = &texts[Path::new("Configuration.xml")];
    for element in [
        "<DefaultSearchForm>CommonForm.MainA</DefaultSearchForm>",
        "<AuxiliaryReportForm>CommonForm.MainA</AuxiliaryReportForm>",
        "<DefaultConstantsForm>CommonForm.MainA</DefaultConstantsForm>",
        "<DefaultReportForm/>",
    ] {
        assert!(text.contains(element), "{element}");
    }
    // An empty name clears the slot.
    let texts = hidden_apply(
        source,
        line,
        |main| {
            let path = main.join("Configuration.xml");
            let text = std::fs::read_to_string(&path).unwrap().replacen(
                "<DefaultSearchForm/>",
                "<DefaultSearchForm>CommonForm.MainA</DefaultSearchForm>",
                1,
            );
            std::fs::write(&path, text).unwrap();
        },
        json!({"at":"main:Configuration","dryRun":true,"ops":[
            {"op":"form.set","args":{"values":{"defaultSearchForm":""}}}]}),
    )
    .unwrap();
    assert!(texts[Path::new("Configuration.xml")].contains("<DefaultSearchForm/>"));
}

/// A configuration of 8.3.27 refuses the auxiliary forms 8.5.1 adds, and a
/// root slot takes only a common form registered in the set.
/// rule: INV.SOURCE.PROFILE-CAPABILITIES
#[test]
fn a_configuration_root_refuses_slots_and_forms_it_does_not_have() {
    let source = "../../tests/fixtures/formats/platform_xml/2.20/all_kinds/src";
    let line = PlatformLine::new(8, 3, 27);
    let error = set_root_forms(source, line, json!({"auxiliaryReportForm":"MainA"})).unwrap_err();
    assert_eq!(error.kind(), ApplyPlanErrorKind::BadValue);
    assert_eq!(
        error.to_string(),
        "Property `AuxiliaryReportForm` exists from platform 8.5.1 (format 2.21); source set `main` is in format 2.20 (platform 8.3.27)."
    );
    let texts = set_root_forms(source, line, json!({"defaultReportForm":"MainA"})).unwrap();
    assert!(texts[Path::new("Configuration.xml")]
        .contains("<DefaultReportForm>CommonForm.MainA</DefaultReportForm>"));
    for (values, kind) in [
        (
            json!({"defaultReportForm":"Missing"}),
            ApplyPlanErrorKind::NotFound,
        ),
        (
            json!({"defaultReportForm":"Catalog.Валюты"}),
            ApplyPlanErrorKind::BadValue,
        ),
        (
            json!({"defaultObjectForm":"MainA"}),
            ApplyPlanErrorKind::BadValue,
        ),
    ] {
        let error = set_root_forms(source, line, values.clone()).unwrap_err();
        assert_eq!(error.kind(), kind, "{values}");
    }
    // An argument the contract does not name is refused at its own key
    // before the slots are looked at.
    let error = hidden_apply(
        source,
        line,
        |_| {},
        json!({"at":"main:Configuration","dryRun":true,"ops":[
            {"op":"form.set","args":{"values":{"defaultReportForm":"Missing"},
                "unicaUnknownArgument":true}}]}),
    )
    .unwrap_err();
    assert_eq!(error.kind(), ApplyPlanErrorKind::BadValue);
    assert_eq!(error.path(), Some("ops[0].args.unicaUnknownArgument"));
}

/// An extension root of 8.5.4 sets what it does when the configuration lacks
/// one of its objects, how it checks extended properties and reduces types;
/// a composite property changes only the parts given.
/// rule: INV.SOURCE.PROFILE-CAPABILITIES
#[test]
fn an_extension_of_line_8_5_4_sets_its_root_actions() {
    let fixture = extension_fixture(
        "../../tests/fixtures/formats/platform_xml/2.22/empty_extension/ext",
        PlatformLine::new(8, 5, 4),
    );
    let staged = plan_all(
        &fixture,
        &[(
            "props.set",
            json!({"at":"main:Configuration","values":{
                "ActionOnConfigurationObjectMissing":"Warning",
                "defaultExtensionPropertyCheckAction":"Ignore",
                "TypeReductionByConfigurationExtensionMode":"DeleteData",
                "ActionOnExtendedConfigurationObjectsMissing":{
                    "AdoptedObject":"Warning","checkedObject":"Auto"
                },
            }}),
        )],
    )
    .unwrap();
    let text = &staged_texts(&staged)[Path::new("Configuration.xml")];
    for element in [
        "<ActionOnConfigurationObjectMissing>Warning</ActionOnConfigurationObjectMissing>",
        "<DefaultExtensionPropertyCheckAction>Ignore</DefaultExtensionPropertyCheckAction>",
        "<TypeReductionByConfigurationExtensionMode>DeleteData</TypeReductionByConfigurationExtensionMode>",
        "<xr:AdoptedObject>Warning</xr:AdoptedObject>",
        "<xr:NotifiedObject>Error</xr:NotifiedObject>",
        "<xr:CheckedObject>Auto</xr:CheckedObject>",
        "<xr:ExtendedObject>Error</xr:ExtendedObject>",
    ] {
        assert!(text.contains(element), "{element}");
    }
}

/// An extension root of an earlier line refuses the actions 8.5.4 adds, and
/// a composite value names only its own parts and values.
/// rule: INV.SOURCE.PROFILE-CAPABILITIES
#[test]
fn an_extension_refuses_root_actions_its_line_lacks_or_does_not_name() {
    let fixture = extension_fixture(
        "../../tests/fixtures/formats/platform_xml/2.21/empty_extension/ext",
        PlatformLine::new(8, 5, 1),
    );
    let error = parse_metadata_plan_operation(
        "props.set",
        &json!({"at":"main:Configuration","values":{
            "ActionOnExtendedConfigurationObjectsMissing":{"AdoptedObject":"Warning"}
        }}),
        0,
        &fixture.binding,
    )
    .unwrap_err();
    assert_eq!(error.kind(), ApplyPlanErrorKind::BadValue);
    assert_eq!(
        error.to_string(),
        "Property `ActionOnExtendedConfigurationObjectsMissing` exists from platform 8.5.4 (format 2.22); source set `main` is in format 2.21 (platform 8.5.1)."
    );
    let fixture = extension_fixture(
        "../../tests/fixtures/formats/platform_xml/2.22/empty_extension/ext",
        PlatformLine::new(8, 5, 4),
    );
    for values in [
        json!({"ActionOnExtendedConfigurationObjectsMissing":{"ExtendDataObject":"Error"}}),
        json!({"ActionOnExtendedConfigurationObjectsMissing":{"AdoptedObject":"Deny"}}),
        json!({"ActionOnExtendedConfigurationObjectsMissing":{}}),
        json!({"TypeReductionByConfigurationExtensionMode":"Auto"}),
    ] {
        let error = parse_metadata_plan_operation(
            "props.set",
            &json!({"at":"main:Configuration","values":values}),
            0,
            &fixture.binding,
        )
        .unwrap_err();
        assert_eq!(error.kind(), ApplyPlanErrorKind::BadValue, "{values}");
    }
}

/// A configuration root of 8.5.4 has none of the extension root actions.
/// rule: INV.SOURCE.PROFILE-CAPABILITIES
#[test]
fn a_configuration_of_line_8_5_4_has_no_extension_root_actions() {
    let fixture = fixture(
        "../../tests/fixtures/formats/platform_xml/2.22/all_kinds/src",
        PlatformLine::new(8, 5, 4),
    );
    let error = parse_metadata_plan_operation(
        "props.set",
        &json!({"at":"main:Configuration","values":{"ActionOnConfigurationObjectMissing":"Warning"}}),
        0,
        &fixture.binding,
    )
    .unwrap_err();
    assert_eq!(error.kind(), ApplyPlanErrorKind::BadValue);
}
