use std::{
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

use unica_adapter_platform_xml::PlatformXmlAdapterFactory;
use unica_format_core::{
    commands::*,
    ports::{
        CompatibilityIssueKind, CompatibilityRequest, OperationCancellation, OwnerResolutionMode,
        WriterRequest,
    },
};

static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone, Copy)]
enum StandaloneFamily {
    DataComposition,
    Spreadsheet,
}

fn text<T>(value: &str, constructor: impl FnOnce(String) -> Result<T, SemanticValueError>) -> T {
    constructor(value.to_string()).unwrap()
}

fn root(label: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "unica-task8-fix4-{label}-{}-{}",
        std::process::id(),
        NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed)
    ))
}

fn family_xml(
    family: StandaloneFamily,
    version: Option<&str>,
    root_name: Option<&str>,
    namespace: Option<&str>,
) -> String {
    let (default_root, default_namespace) = match family {
        StandaloneFamily::DataComposition => (
            "DataCompositionSchema",
            "http://v8.1c.ru/8.1/data-composition-system/schema",
        ),
        StandaloneFamily::Spreadsheet => ("document", "http://v8.1c.ru/8.2/data/spreadsheet"),
    };
    let version = version
        .map(|value| format!(" version=\"{value}\""))
        .unwrap_or_default();
    format!(
        "<{} xmlns=\"{}\"{version}/>",
        root_name.unwrap_or(default_root),
        namespace.unwrap_or(default_namespace),
    )
}

fn compatibility_issue(root: &Path, target: &Path) -> Option<CompatibilityIssueKind> {
    let session = PlatformXmlAdapterFactory::new().capture_unscoped_source(
        target,
        root,
        OwnerResolutionMode::Existing,
    );
    PlatformXmlAdapterFactory::new()
        .operational_registration()
        .compatibility()
        .inspect(&CompatibilityRequest::new(vec![session]).unwrap())
        .unwrap()
        .issue()
        .map(|issue| issue.kind())
}

fn data_composition_definition() -> DataCompositionDefinition {
    DataCompositionDefinition::new(vec![DataCompositionDataSet::Query(
        DataCompositionQueryDataSet::new(
            text("Round4Data", DataSetName::new),
            text("SELECT 1 AS Round4Value", DataCompositionQueryText::new),
        ),
    )])
    .unwrap()
}

fn spreadsheet_document() -> SpreadsheetDocument {
    let cell = SpreadsheetCell::new(
        1,
        SpreadsheetCellValue::Text(text("Round 4", SpreadsheetCellText::new)),
    )
    .unwrap();
    let area = SpreadsheetArea::new(
        text("Round4Area", SpreadsheetAreaName::new),
        vec![SpreadsheetRow::new(vec![cell])],
    )
    .unwrap();
    SpreadsheetDocument::new(vec![area]).unwrap()
}

fn family_command(family: StandaloneFamily) -> WriterCommand {
    match family {
        StandaloneFamily::DataComposition => WriterCommand::DataCompositionCreate(
            DataCompositionCreate::new(data_composition_definition()),
        ),
        StandaloneFamily::Spreadsheet => {
            WriterCommand::SpreadsheetCreate(SpreadsheetCreate::new(spreadsheet_document()))
        }
    }
}

fn execute(
    root: &Path,
    command: WriterCommand,
    sources: Vec<(WriterSourceRole, PathBuf)>,
    mode: MutationMode,
) -> WriterResult {
    let session = PlatformXmlAdapterFactory::new()
        .capture_writer_session(
            sources,
            root,
            root,
            &root.join(".cache"),
            NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed),
        )
        .unwrap();
    PlatformXmlAdapterFactory::new()
        .operational_registration()
        .writer()
        .execute(&WriterRequest::new(
            session,
            command,
            mode,
            OperationCancellation::new(),
        ))
        .unwrap()
}

fn execute_family(
    root: &Path,
    family: StandaloneFamily,
    target: &Path,
    mode: MutationMode,
) -> WriterResult {
    let role = match family {
        StandaloneFamily::DataComposition | StandaloneFamily::Spreadsheet => {
            WriterSourceRole::DestinationArtifact
        }
    };
    execute(
        root,
        family_command(family),
        vec![(role, target.to_path_buf())],
        mode,
    )
}

fn root_version(path: &Path) -> Option<String> {
    let xml = fs::read_to_string(path).unwrap();
    let document = roxmltree::Document::parse(&xml).unwrap();
    document
        .root_element()
        .attribute("version")
        .map(str::to_owned)
}

#[test]
fn dcs_and_mxl_use_complete_family_revision_policy_with_dry_run_apply_parity() {
    let versions = [
        (Some("0.9"), Some(CompatibilityIssueKind::Older)),
        (Some("1.0"), None),
        (Some("1.1"), Some(CompatibilityIssueKind::Newer)),
        (Some("9.0"), Some(CompatibilityIssueKind::Newer)),
        (None, Some(CompatibilityIssueKind::Malformed)),
        (Some("1.x"), Some(CompatibilityIssueKind::Malformed)),
    ];

    for family in [
        StandaloneFamily::DataComposition,
        StandaloneFamily::Spreadsheet,
    ] {
        for (version, expected_issue) in versions {
            let fixture = root(&format!("{family:?}-{version:?}"));
            fs::create_dir_all(&fixture).unwrap();
            let target = fixture.join("Template.xml");
            let initial = family_xml(family, version, None, None);
            fs::write(&target, initial.as_bytes()).unwrap();

            assert_eq!(
                compatibility_issue(&fixture, &target),
                expected_issue,
                "{family:?} {version:?}"
            );

            let dry_run = execute_family(&fixture, family, &target, MutationMode::Preview);
            assert_eq!(fs::read_to_string(&target).unwrap(), initial);
            let apply = execute_family(&fixture, family, &target, MutationMode::Apply);

            match expected_issue {
                None => {
                    assert!(matches!(dry_run.lifecycle(), WriterLifecycle::Previewed));
                    assert!(matches!(apply.lifecycle(), WriterLifecycle::Applied));
                    assert_eq!(root_version(&target).as_deref(), Some("1.0"));
                }
                Some(_) => {
                    assert!(matches!(dry_run.lifecycle(), WriterLifecycle::Rejected(_)));
                    assert!(matches!(apply.lifecycle(), WriterLifecycle::Rejected(_)));
                    assert_eq!(fs::read_to_string(&target).unwrap(), initial);
                }
            }
        }
    }
}

#[test]
fn dcs_and_mxl_reject_wrong_root_or_namespace_as_malformed() {
    for family in [
        StandaloneFamily::DataComposition,
        StandaloneFamily::Spreadsheet,
    ] {
        for (root_name, namespace) in [(Some("WrongRoot"), None), (None, Some("urn:wrong-family"))]
        {
            let fixture = root(&format!("{family:?}-{root_name:?}-{namespace:?}"));
            fs::create_dir_all(&fixture).unwrap();
            let target = fixture.join("Template.xml");
            fs::write(
                &target,
                family_xml(family, Some("1.0"), root_name, namespace),
            )
            .unwrap();
            assert_eq!(
                compatibility_issue(&fixture, &target),
                Some(CompatibilityIssueKind::Malformed)
            );
        }
    }
}

fn initialize_configuration(root: &Path) {
    let result = execute(
        root,
        WriterCommand::ConfigurationInitialize(ConfigurationInitialize::new(text(
            "Round4Configuration",
            ConfigurationName::new,
        ))),
        vec![(WriterSourceRole::DestinationDirectory, root.join("src"))],
        MutationMode::Apply,
    );
    assert!(matches!(result.lifecycle(), WriterLifecycle::Applied));
}

fn metadata_definition(
    name: &str,
    kind: MetadataKind,
    property: MetadataKindProperty,
) -> MetadataDefinition {
    MetadataDefinition::new(
        MetadataCommonDefinition::new(text(name, MetadataChildName::new)),
        MetadataKindDefinition::new(kind, vec![property]).unwrap(),
    )
}

fn direct_child_text(path: &Path, child: &str) -> Option<String> {
    let xml = fs::read_to_string(path).unwrap();
    let document = roxmltree::Document::parse(&xml).unwrap();
    let properties = document
        .descendants()
        .find(|node| node.is_element() && node.tag_name().name() == "Properties")
        .unwrap();
    properties
        .children()
        .find(|node| node.is_element() && node.tag_name().name() == child)
        .and_then(|node| node.text())
        .map(str::to_owned)
}

#[test]
fn typed_metadata_contract_maps_accounting_totals_and_calculation_periodicity_exactly() {
    let fixture = root("metadata-contract");
    initialize_configuration(&fixture);

    let accounting = metadata_definition(
        "Ledger",
        MetadataKind::AccountingRegister,
        MetadataKindProperty::new(
            MetadataKindPropertyName::EnableTotalsSplitting,
            MetadataPropertyValue::Boolean(true),
        )
        .unwrap(),
    );
    let accounting_result = execute(
        &fixture,
        WriterCommand::MetadataCreate(MetadataCreate::new(accounting)),
        vec![(WriterSourceRole::DestinationDirectory, fixture.join("src"))],
        MutationMode::Apply,
    );
    assert!(matches!(
        accounting_result.lifecycle(),
        WriterLifecycle::Applied
    ));
    assert_eq!(
        direct_child_text(
            &fixture.join("src/AccountingRegisters/Ledger.xml"),
            "EnableTotalsSplitting"
        )
        .as_deref(),
        Some("true")
    );

    let calculation = metadata_definition(
        "Payroll",
        MetadataKind::CalculationRegister,
        MetadataKindProperty::new(
            MetadataKindPropertyName::Periodicity,
            MetadataPropertyValue::Periodicity(MetadataPeriodicity::Day),
        )
        .unwrap(),
    );
    let calculation_result = execute(
        &fixture,
        WriterCommand::MetadataCreate(MetadataCreate::new(calculation)),
        vec![(WriterSourceRole::DestinationDirectory, fixture.join("src"))],
        MutationMode::Apply,
    );
    assert!(matches!(
        calculation_result.lifecycle(),
        WriterLifecycle::Applied
    ));

    let replacement = metadata_definition(
        "Payroll",
        MetadataKind::CalculationRegister,
        MetadataKindProperty::new(
            MetadataKindPropertyName::Periodicity,
            MetadataPropertyValue::Periodicity(MetadataPeriodicity::Year),
        )
        .unwrap(),
    );
    let calculation_path = fixture.join("src/CalculationRegisters/Payroll.xml");
    let edit_result = execute(
        &fixture,
        WriterCommand::MetadataEdit(MetadataEdit::new(
            text("CalculationRegister.Payroll", MetadataObjectReference::new),
            MetadataPatch::Replace(replacement),
        )),
        vec![(WriterSourceRole::Object, calculation_path.clone())],
        MutationMode::Apply,
    );
    assert!(matches!(edit_result.lifecycle(), WriterLifecycle::Applied));
    assert_eq!(
        direct_child_text(&calculation_path, "Periodicity").as_deref(),
        Some("Year")
    );
    assert_eq!(
        direct_child_text(&calculation_path, "InformationRegisterPeriodicity"),
        None
    );
}

#[test]
fn independent_metadata_oracle_detects_wrong_native_target_mutations() {
    fn legacy_2_20_oracle(xml: &str) -> bool {
        let document = roxmltree::Document::parse(xml).unwrap();
        let root = document.root_element();
        let owner = root.tag_name().name();
        let expected = match owner {
            "AccountingRegister" => ("EnableTotalsSplitting", "true"),
            "CalculationRegister" => ("Periodicity", "Year"),
            _ => return false,
        };
        root.children().any(|properties| {
            properties.is_element()
                && properties.tag_name().name() == "Properties"
                && properties.children().any(|child| {
                    child.is_element()
                        && child.tag_name().name() == expected.0
                        && child.text() == Some(expected.1)
                })
        })
    }

    let valid_accounting = concat!(
        "<AccountingRegister><Properties>",
        "<EnableTotalsSplitting>true</EnableTotalsSplitting>",
        "</Properties></AccountingRegister>"
    );
    let valid_calculation = concat!(
        "<CalculationRegister><Properties>",
        "<Periodicity>Year</Periodicity>",
        "</Properties></CalculationRegister>"
    );
    assert!(legacy_2_20_oracle(valid_accounting));
    assert!(legacy_2_20_oracle(valid_calculation));
    assert!(!legacy_2_20_oracle(
        &valid_accounting.replace("true", "false")
    ));
    assert!(!legacy_2_20_oracle(
        &valid_calculation.replace("Periodicity", "InformationRegisterPeriodicity")
    ));
}
