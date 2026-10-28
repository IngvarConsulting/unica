use super::*;

#[test]
fn main_dcs_property_selects_a_registered_schema_in_the_staged_report() {
    let owner = r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Report uuid="11111111-1111-4111-8111-111111111111"><Properties><Name>Sales</Name><MainDataCompositionSchema/><Comment>keep</Comment></Properties><ChildObjects/></Report></MetaDataObject>"#;
    let config =
        RU_CONFIGURATION_XML.replace("<Document>Order</Document>", "<Report>Sales</Report>");
    let fixture = MetadataFixture::with_files(&[
        ("Configuration.xml", &config),
        ("Reports/Sales.xml", owner),
        ("Languages/Русский.xml", &language_xml("Русский", "ru")),
    ]);
    let operations = [
        fixture.parse("template.add", json!({"at":"main:Report.Sales","items":[{"name":"MainSchema","templateType":"DataCompositionSchema"}]}),0),
        fixture.parse("props.set", json!({"at":"main:Report.Sales","values":{"MainDataCompositionSchema":"Report.Sales.Template.MainSchema"}}),1),
    ];
    let admission = fixture.admission();
    let authority = admission
        .metadata_planning_authority(&fixture.binding)
        .unwrap();
    let (staged, _) = plan_metadata_batch_owned_for_test(
        admission.staged_state().unwrap(),
        authority,
        &operations,
    )
    .unwrap_or_else(|error| panic!("{error:?} at {:?}", error.path()));
    let change = staged
        .planned_changes()
        .into_iter()
        .find(|change| change.relative_path == Path::new("Reports/Sales.xml"))
        .unwrap();
    let StagedFileState::Bytes(bytes) = change.current else {
        panic!("report must stay present")
    };
    let xml = std::str::from_utf8(&bytes).unwrap();
    let doc = roxmltree::Document::parse(xml).unwrap();
    assert_eq!(
        doc.descendants()
            .find(|n| n.has_tag_name("MainDataCompositionSchema"))
            .and_then(|n| n.text()),
        Some("Report.Sales.Template.MainSchema")
    );
    assert!(xml.contains("<Comment>keep</Comment>"));
    assert_eq!(
        std::fs::read_to_string(fixture.binding.source_root().join("Reports/Sales.xml")).unwrap(),
        owner
    );
}

const REGISTERED_REPORT: &str = r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Report uuid="11111111-1111-4111-8111-111111111111"><Properties><Name>Sales</Name><MainDataCompositionSchema>Report.Sales.Template.MainSchema</MainDataCompositionSchema><Comment>keep</Comment></Properties><ChildObjects><Template>MainSchema</Template><Template>Other</Template></ChildObjects></Report></MetaDataObject>"#;
const SCHEMA_DESCRIPTOR: &str = r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Template uuid="22222222-2222-4222-8222-222222222222"><Properties><Name>MainSchema</Name><Comment/><TemplateType>DataCompositionSchema</TemplateType></Properties></Template></MetaDataObject>"#;
const SCHEMA_BODY: &str =
    r#"<DataCompositionSchema xmlns="http://v8.1c.ru/8.1/data-composition-system/schema"/>"#;

fn registered_fixture() -> MetadataFixture {
    let config =
        RU_CONFIGURATION_XML.replace("<Document>Order</Document>", "<Report>Sales</Report>");
    let other = SCHEMA_DESCRIPTOR.replace("MainSchema", "Other");
    MetadataFixture::with_files(&[
        ("Configuration.xml", &config),
        ("Reports/Sales.xml", REGISTERED_REPORT),
        ("Reports/Sales/Templates/MainSchema.xml", SCHEMA_DESCRIPTOR),
        (
            "Reports/Sales/Templates/MainSchema/Ext/Template.xml",
            SCHEMA_BODY,
        ),
        ("Reports/Sales/Templates/Other.xml", &other),
        (
            "Reports/Sales/Templates/Other/Ext/Template.xml",
            SCHEMA_BODY,
        ),
    ])
}

fn plan_final(
    fixture: &MetadataFixture,
    requests: &[(&str, serde_json::Value)],
) -> Result<unica_workspace::apply::ApplyStagedState, unica_source_contract::proposal::ApplyPlanError>
{
    let operations = requests
        .iter()
        .enumerate()
        .map(|(index, (op, args))| {
            parse_metadata_plan_operation(op, args, index, &fixture.binding)
                .map(|operation| IndexedPlanOperation::new(index, operation))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let admission = fixture.admission();
    let authority = admission.metadata_planning_authority(&fixture.binding)?;
    let (mut staged, _) = plan_metadata_batch_owned_for_test(
        admission.staged_state().unwrap(),
        authority,
        &operations,
    )?;
    unica_metadata_designer::planner::validate_final_object_integrity(
        &mut staged.proposal(),
        "main",
        SourceSetKind::Configuration,
    )?;
    Ok(staged)
}

#[test]
fn main_dcs_rejects_a_foreign_missing_unregistered_or_non_dcs_template() {
    for selected in [
        "Report.Other.Template.MainSchema",
        "Report.Sales.Template.Missing",
        "main:Report.Sales.Template.MainSchema",
        "Report.Sales.Template.MainSchema.DataSet.Items",
    ] {
        let fixture = registered_fixture();
        let error = plan_final(
            &fixture,
            &[(
                "props.set",
                json!({"at":"main:Report.Sales","values":{"MainDataCompositionSchema":selected}}),
            )],
        )
        .unwrap_err();
        assert_eq!(
            error.kind(),
            ApplyPlanErrorKind::BadValue,
            "{selected}: {error:?}"
        );
        assert_eq!(
            std::fs::read_to_string(fixture.binding.source_root().join("Reports/Sales.xml"))
                .unwrap(),
            REGISTERED_REPORT
        );
    }
    for descriptor in [
        SCHEMA_DESCRIPTOR.replace("DataCompositionSchema", "SpreadsheetDocument"),
        SCHEMA_DESCRIPTOR.replace("<Name>MainSchema</Name>", "<Name>Foreign</Name>"),
        SCHEMA_DESCRIPTOR.replace("22222222-2222-4222-8222-222222222222", "invalid"),
    ] {
        let fixture = registered_fixture();
        std::fs::write(
            fixture
                .binding
                .source_root()
                .join("Reports/Sales/Templates/MainSchema.xml"),
            descriptor,
        )
        .unwrap();
        assert!(plan_final(&fixture,&[("props.set",json!({"at":"main:Report.Sales","values":{"MainDataCompositionSchema":"Report.Sales.Template.MainSchema"}}))]).is_err());
    }
    let fixture = registered_fixture();
    let owner = REGISTERED_REPORT.replace("<Template>MainSchema</Template>", "");
    std::fs::write(
        fixture.binding.source_root().join("Reports/Sales.xml"),
        owner,
    )
    .unwrap();
    assert!(plan_final(&fixture,&[("props.set",json!({"at":"main:Report.Sales","values":{"MainDataCompositionSchema":"Report.Sales.Template.MainSchema"}}))]).is_err());
}

#[test]
fn main_dcs_replacement_clear_and_noop_keep_guarded_schema_inputs() {
    let fixture = registered_fixture();
    let staged=plan_final(&fixture,&[("props.set",json!({"at":"main:Report.Sales","values":{"MainDataCompositionSchema":"Report.Sales.Template.MainSchema"}}))]).unwrap();
    assert!(staged.planned_changes().is_empty());
    // Even a no-op assignment validates the selected descriptor and resource.
    assert!(staged
        .original_image(Path::new("Reports/Sales/Templates/MainSchema.xml"))
        .is_some());
    assert!(staged
        .original_image(Path::new(
            "Reports/Sales/Templates/MainSchema/Ext/Template.xml"
        ))
        .is_some());
    for selected in ["Report.Sales.Template.Other", ""] {
        let staged = plan_final(
            &fixture,
            &[(
                "props.set",
                json!({"at":"main:Report.Sales","values":{"MainDataCompositionSchema":selected}}),
            )],
        )
        .unwrap();
        let change = staged
            .planned_changes()
            .into_iter()
            .find(|c| c.relative_path == Path::new("Reports/Sales.xml"))
            .unwrap();
        let StagedFileState::Bytes(bytes) = change.current else {
            panic!("report stays present")
        };
        let doc = roxmltree::Document::parse(std::str::from_utf8(&bytes).unwrap()).unwrap();
        assert_eq!(
            doc.descendants()
                .find(|n| n.has_tag_name("MainDataCompositionSchema"))
                .and_then(|n| n.text())
                .unwrap_or(""),
            selected
        );
    }
    assert!(plan_final(
        &fixture,
        &[(
            "props.set",
            json!({"at":"main:Report.Sales","values":{"MainDataCompositionSchema":null}})
        )]
    )
    .is_err());
}

#[test]
fn final_main_dcs_validation_observes_template_type_changes_and_removal() {
    let fixture = registered_fixture();
    let changed_type = (
        "template.set",
        json!({"at":"main:Report.Sales.Template.MainSchema","values":{"templateType":"SpreadsheetDocument"}}),
    );
    assert!(
        plan_final(&fixture, std::slice::from_ref(&changed_type)).is_err(),
        "changing only the selected template still validates its owner"
    );
    assert!(plan_final(&fixture,&[("props.set",json!({"at":"main:Report.Sales","values":{"MainDataCompositionSchema":"Report.Sales.Template.MainSchema"}})),changed_type]).is_err());
    let staged=plan_final(&fixture,&[("props.set",json!({"at":"main:Report.Sales","values":{"MainDataCompositionSchema":"Report.Sales.Template.MainSchema"}})),("template.remove",json!({"at":"main:Report.Sales.Template.MainSchema"}))]).unwrap();
    let change = staged
        .planned_changes()
        .into_iter()
        .find(|c| c.relative_path == Path::new("Reports/Sales.xml"))
        .unwrap();
    let StagedFileState::Bytes(bytes) = change.current else {
        panic!("report stays present")
    };
    let doc = roxmltree::Document::parse(std::str::from_utf8(&bytes).unwrap()).unwrap();
    assert!(doc
        .descendants()
        .find(|n| n.has_tag_name("MainDataCompositionSchema"))
        .and_then(|n| n.text())
        .unwrap_or("")
        .is_empty());
    assert!(plan_final(&fixture,&[("template.remove",json!({"at":"main:Report.Sales.Template.MainSchema"})),("props.set",json!({"at":"main:Report.Sales","values":{"MainDataCompositionSchema":"Report.Sales.Template.MainSchema"}}))]).is_err());
}

#[test]
fn main_dcs_missing_or_damaged_primary_resource_cannot_be_selected() {
    for replacement in [None, Some("<wrong/>"), Some("not XML")] {
        let fixture = registered_fixture();
        let path = fixture
            .binding
            .source_root()
            .join("Reports/Sales/Templates/MainSchema/Ext/Template.xml");
        match replacement {
            None => std::fs::remove_file(path).unwrap(),
            Some(text) => std::fs::write(path, text).unwrap(),
        }
        assert!(plan_final(&fixture, &[("props.set", json!({"at":"main:Report.Sales","values":{"MainDataCompositionSchema":"Report.Sales.Template.MainSchema"}}))]).is_err());
        assert_eq!(
            std::fs::read_to_string(fixture.binding.source_root().join("Reports/Sales.xml"))
                .unwrap(),
            REGISTERED_REPORT
        );
    }
}

#[test]
fn unrelated_report_edits_do_not_take_ownership_of_an_old_bad_schema_reference() {
    let fixture = registered_fixture();
    let owner = REGISTERED_REPORT.replace(
        "Report.Sales.Template.MainSchema",
        "Report.Sales.Template.Missing",
    );
    std::fs::write(
        fixture.binding.source_root().join("Reports/Sales.xml"),
        &owner,
    )
    .unwrap();
    let staged = plan_final(
        &fixture,
        &[
            (
                "props.set",
                json!({"at":"main:Report.Sales","values":{"Comment":"updated"}}),
            ),
            (
                "template.set",
                json!({"at":"main:Report.Sales.Template.Other","values":{"comment":"updated"}}),
            ),
        ],
    )
    .unwrap();
    let change = staged
        .planned_changes()
        .into_iter()
        .find(|c| c.relative_path == Path::new("Reports/Sales.xml"))
        .unwrap();
    let StagedFileState::Bytes(bytes) = change.current else {
        panic!("report stays present")
    };
    let xml = std::str::from_utf8(&bytes).unwrap();
    assert!(xml.contains("Report.Sales.Template.Missing"));
    assert!(xml.contains("<Comment>updated</Comment>"));
}

#[test]
fn main_dcs_assignment_preserves_the_namespace_of_a_prefixed_report() {
    let fixture = registered_fixture();
    let prefixed = REGISTERED_REPORT
        .replace("<", "<md:")
        .replace("<md:/", "</md:")
        .replace("xmlns=", "xmlns:md=");
    std::fs::write(
        fixture.binding.source_root().join("Reports/Sales.xml"),
        &prefixed,
    )
    .unwrap();
    let staged=plan_final(&fixture,&[("props.set",json!({"at":"main:Report.Sales","values":{"MainDataCompositionSchema":"Report.Sales.Template.Other"}}))]).unwrap();
    let change = staged
        .planned_changes()
        .into_iter()
        .find(|c| c.relative_path == Path::new("Reports/Sales.xml"))
        .unwrap();
    let StagedFileState::Bytes(bytes) = change.current else {
        panic!("report stays present")
    };
    let doc = roxmltree::Document::parse(std::str::from_utf8(&bytes).unwrap()).unwrap();
    assert_eq!(doc.descendants().find(|n|n.has_tag_name(("http://v8.1c.ru/8.3/MDClasses","MainDataCompositionSchema"))).and_then(|n|n.text()),Some("Report.Sales.Template.Other"));
    assert!(std::str::from_utf8(&bytes)
        .unwrap()
        .contains("<md:Comment>keep</md:Comment>"));
}

fn main_dcs_namespace_cases() -> [(String, &'static str); 3] {
    let prefixed = REGISTERED_REPORT
        .replace("<", "<md:")
        .replace("<md:/", "</md:")
        .replace("xmlns=", "xmlns:md=");
    let foreign_default =
        prefixed.replace("xmlns:md=", "xmlns=\"urn:unrelated-default\" xmlns:md=");
    [
        (REGISTERED_REPORT.to_string(), ""),
        (prefixed, " xmlns=\"http://v8.1c.ru/8.3/MDClasses\""),
        (foreign_default, " xmlns=\"http://v8.1c.ru/8.3/MDClasses\""),
    ]
}

#[test]
fn main_dcs_assignment_writes_a_trimmed_reference_with_only_the_required_namespace() {
    for (owner, namespace) in main_dcs_namespace_cases() {
        let fixture = registered_fixture();
        std::fs::write(
            fixture.binding.source_root().join("Reports/Sales.xml"),
            &owner,
        )
        .unwrap();
        let staged = plan_final(&fixture, &[("props.set", json!({"at":"main:Report.Sales","values":{"MainDataCompositionSchema":" \tReport.Sales.Template.Other\r\n "}}))]).unwrap();
        let changed = staged
            .planned_changes()
            .into_iter()
            .find(|change| change.relative_path == Path::new("Reports/Sales.xml"))
            .unwrap();
        let StagedFileState::Bytes(bytes) = changed.current else {
            panic!("report stays present")
        };
        let xml = std::str::from_utf8(&bytes).unwrap();
        let document = roxmltree::Document::parse(xml).unwrap();
        let reference = document
            .descendants()
            .find(|node| {
                node.has_tag_name(("http://v8.1c.ru/8.3/MDClasses", "MainDataCompositionSchema"))
            })
            .unwrap();
        assert_eq!(
            &xml[reference.range()],
            format!("<MainDataCompositionSchema{namespace}>Report.Sales.Template.Other</MainDataCompositionSchema>")
        );
        assert_eq!(
            std::fs::read_to_string(fixture.binding.source_root().join("Reports/Sales.xml"))
                .unwrap(),
            owner,
            "planning must not publish the canonicalized image"
        );
    }
}

#[test]
fn main_dcs_clear_writes_a_self_closing_reference_in_its_original_namespace() {
    for (owner, namespace) in main_dcs_namespace_cases() {
        for value in ["", " \t\r\n "] {
            let fixture = registered_fixture();
            std::fs::write(
                fixture.binding.source_root().join("Reports/Sales.xml"),
                &owner,
            )
            .unwrap();
            let staged = plan_final(
                &fixture,
                &[(
                    "props.set",
                    json!({"at":"main:Report.Sales","values":{"MainDataCompositionSchema":value}}),
                )],
            )
            .unwrap();
            let changed = staged
                .planned_changes()
                .into_iter()
                .find(|change| change.relative_path == Path::new("Reports/Sales.xml"))
                .unwrap();
            let StagedFileState::Bytes(bytes) = changed.current else {
                panic!("report stays present")
            };
            let xml = std::str::from_utf8(&bytes).unwrap();
            let document = roxmltree::Document::parse(xml).unwrap();
            let reference = document
                .descendants()
                .find(|node| {
                    node.has_tag_name((
                        "http://v8.1c.ru/8.3/MDClasses",
                        "MainDataCompositionSchema",
                    ))
                })
                .unwrap();
            assert_eq!(
                &xml[reference.range()],
                format!("<MainDataCompositionSchema{namespace}/>")
            );
        }
    }
}

#[test]
fn main_dcs_whitespace_equivalent_assignment_keeps_the_existing_owner_bytes() {
    for (owner, _) in main_dcs_namespace_cases() {
        let fixture = registered_fixture();
        std::fs::write(
            fixture.binding.source_root().join("Reports/Sales.xml"),
            &owner,
        )
        .unwrap();
        let staged = plan_final(&fixture, &[("props.set", json!({"at":"main:Report.Sales","values":{"MainDataCompositionSchema":" \tReport.Sales.Template.MainSchema\r\n "}}))]).unwrap();
        assert!(staged.planned_changes().is_empty());
        assert!(staged
            .original_image(Path::new("Reports/Sales/Templates/MainSchema.xml"))
            .is_some());
        assert_eq!(
            std::fs::read_to_string(fixture.binding.source_root().join("Reports/Sales.xml"))
                .unwrap(),
            owner
        );
    }
}
