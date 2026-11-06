//! Real publication of equivalent BSL, DCS and spreadsheet edits.
use super::publication_invariant_tests::{
    assert_noop, plan, prepare_publication, publication_input, request, Snapshot,
};
use super::tests::ApplySeamFixture;
use crate::domain::apply::ApplyRequest;
use serde_json::json;
use unica_source_contract::proposal::ApplyStagingErrorKind;
use unica_workspace::actor::ApplyPublicationErrorKind;
use unica_workspace::test_support::tree_snapshot;

fn commit(fixture: &ApplySeamFixture, input: &ApplyRequest) {
    let before = tree_snapshot(&fixture.source_dir());
    let result = fixture
        .actor()
        .publish_prepared_apply(prepare_publication(fixture, input))
        .unwrap();
    assert_eq!(result.commit_count_for_test(), 1);
    assert!(!result.effects().events().is_empty());
    assert_ne!(tree_snapshot(&fixture.source_dir()), before);
}

fn code_fixture() -> (ApplySeamFixture, ApplyRequest) {
    let f = ApplySeamFixture::new();
    let source = f.source_dir();
    let root = source.join("Configuration.xml");
    let text = std::fs::read_to_string(&root).unwrap();
    std::fs::write(
        root,
        text.replace(
            "<ChildObjects>",
            "<ChildObjects><CommonModule>Sample</CommonModule>",
        ),
    )
    .unwrap();
    std::fs::create_dir_all(source.join("CommonModules/Sample/Ext")).unwrap();
    std::fs::write(source.join("CommonModules/Sample.xml"), r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><CommonModule uuid="bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb"><Properties><Name>Sample</Name></Properties></CommonModule></MetaDataObject>"#).unwrap();
    std::fs::write(
        source.join("CommonModules/Sample/Ext/Module.bsl"),
        b"\xef\xbb\xbfProcedure Base()\r\nEndProcedure\r\n// untouched\r\n",
    )
    .unwrap();
    let change = request(
        "main:CommonModule.Sample",
        json!([{"op":"code.insert","args":{"text":"Procedure Added()\nEndProcedure"}}]),
    );
    (f, change)
}

fn template_fixture(spreadsheet: bool) -> (ApplySeamFixture, ApplyRequest) {
    let f = ApplySeamFixture::new();
    let kind = if spreadsheet {
        "SpreadsheetDocument"
    } else {
        "DataCompositionSchema"
    };
    commit(
        &f,
        &request(
            "main:Document.First",
            json!([{"op":"template.add","args":{"items":[{"name":"Print","templateType":kind}]}}]),
        ),
    );
    let at = "main:Document.First.Template.Print";
    if spreadsheet {
        commit(
            &f,
            &request(
                at,
                json!([{"op":"area.add","args":{"items":[{"name":"Header","cells":{"R1C1":"before"}}]}}]),
            ),
        );
        (
            f,
            request(
                "main:Document.First.Template.Print.Area.Header",
                json!([{"op":"mxl.set","args":{"values":{"cells":{"R1C1":"after"}}}}]),
            ),
        )
    } else {
        let xml = r#"<DataCompositionSchema xmlns="http://v8.1c.ru/8.1/data-composition-system/schema" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance"><dataSource><name>Local</name><dataSourceType>Local</dataSourceType></dataSource><dataSet xsi:type="DataSetQuery"><name>Data</name><dataSource>Local</dataSource><query>ВЫБРАТЬ 1 КАК Amount</query></dataSet></DataCompositionSchema>"#;
        std::fs::write(
            f.source_dir()
                .join("Documents/First/Templates/Print/Ext/Template.xml"),
            xml,
        )
        .unwrap();
        (
            f,
            request(
                "main:Document.First.Template.Print.DataSet.Data.Query",
                json!([{"op":"query.set","args":{"values":{"query":"ВЫБРАТЬ 2 КАК Amount"}}}]),
            ),
        )
    }
}

/// rule: INV.SOURCE.EMPTY-APPLY-PUBLICATION
#[test]
fn bsl_dcs_and_mxl_publish_first_edit_then_fresh_equivalent_plan_without_writes() {
    for (fixture, change) in [
        code_fixture(),
        template_fixture(false),
        template_fixture(true),
    ] {
        commit(&fixture, &change);
        assert!(
            fixture.actor().context().cache_root.exists(),
            "the second publication must preserve an existing cache/state"
        );
        assert_noop(&fixture, &change);
    }
}

fn drift(fixture: &ApplySeamFixture, input: &ApplyRequest, relative: &str, after_prepare: bool) {
    let (actual, mut admission) = publication_input(fixture, input);
    assert!(!actual.dry_run());
    let (staged, effects) = plan(fixture, &actual, &mut admission);
    assert!(!staged.planned_changes().is_empty());
    let absent = !fixture.source_dir().join(relative).exists();
    let mutate = || {
        let path = fixture.source_dir().join(relative);
        let mut bytes = std::fs::read(&path).unwrap_or_else(|_| b"<foreign/>".to_vec());
        if relative.ends_with(".bsl") {
            bytes.extend_from_slice(b"\r\n// foreign edit\r\n");
        } else {
            bytes.extend_from_slice(b"<!-- foreign edit -->");
        }
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, bytes).unwrap();
        Snapshot::capture(fixture)
    };
    if after_prepare {
        let prepared = admission.prepare_with_effects(staged, effects).unwrap();
        let before = mutate();
        let error = fixture
            .actor()
            .publish_prepared_apply(prepared)
            .unwrap_err();
        assert_eq!(
            error.kind(),
            if absent {
                ApplyPublicationErrorKind::ContainmentIdentity
            } else {
                ApplyPublicationErrorKind::ConcurrentRevision
            },
            "{relative}: {error}"
        );
        before.assert_unchanged();
    } else {
        let before = mutate();
        match admission.prepare_with_effects(staged, effects) {
            Err(error) => assert_eq!(
                error.kind(),
                if absent {
                    ApplyStagingErrorKind::AbsentChainOccupied
                } else {
                    ApplyStagingErrorKind::ConcurrentRevision
                },
                "{relative}: {error}"
            ),
            Ok(prepared) => {
                let error = fixture
                    .actor()
                    .publish_prepared_apply(prepared)
                    .unwrap_err();
                assert_eq!(
                    error.kind(),
                    if absent {
                        ApplyPublicationErrorKind::ContainmentIdentity
                    } else {
                        ApplyPublicationErrorKind::ConcurrentRevision
                    },
                    "{relative}: {error}"
                );
            }
        }
        before.assert_unchanged();
    }
}

/// rule: INV.SOURCE.BOUND-PREIMAGES
#[test]
fn medium_bsl_dcs_and_mxl_revalidate_actual_bodies_and_owners_in_both_publication_windows() {
    for after_prepare in [false, true] {
        for relative in [
            "CommonModules/Sample.xml",
            "CommonModules/Sample/Ext/Module.bsl",
        ] {
            let (f, input) = code_fixture();
            drift(&f, &input, relative, after_prepare);
        }
        for spreadsheet in [false, true] {
            for relative in [
                "Documents/First.xml",
                "Documents/First/Templates/Print.xml",
                "Documents/First/Templates/Print/Ext/Template.xml",
            ] {
                let (f, input) = template_fixture(spreadsheet);
                drift(&f, &input, relative, after_prepare);
            }
        }
    }
}

/// rule: INV.SOURCE.BOUND-PREIMAGES
#[test]
fn dcs_and_mxl_creation_revalidate_absent_descriptor_and_body_without_partial_publication() {
    for after_prepare in [false, true] {
        for kind in ["DataCompositionSchema", "SpreadsheetDocument"] {
            for relative in [
                "Documents/First/Templates/Print.xml",
                "Documents/First/Templates/Print/Ext/Template.xml",
            ] {
                let f = ApplySeamFixture::new();
                std::fs::create_dir_all(f.source_dir().join("Documents/First/Templates/Print/Ext"))
                    .unwrap();
                let input = request(
                    "main:Document.First",
                    json!([{"op":"template.add","args":{"items":[{"name":"Print","templateType":kind}]}}]),
                );
                drift(&f, &input, relative, after_prepare);
            }
        }
    }
}

/// rule: INV.SOURCE.BOUND-PREIMAGES
#[test]
fn dcs_and_mxl_creation_revalidate_the_consumed_owner_and_default_language() {
    for after_prepare in [false, true] {
        for kind in ["DataCompositionSchema", "SpreadsheetDocument"] {
            for relative in [
                "Configuration.xml",
                "Documents/First.xml",
                "Languages/Русский.xml",
            ] {
                let f = ApplySeamFixture::new();
                let input = request(
                    "main:Document.First",
                    json!([{"op":"template.add","args":{"items":[{"name":"Print","templateType":kind}]}}]),
                );
                drift(&f, &input, relative, after_prepare);
            }
        }
    }
}
