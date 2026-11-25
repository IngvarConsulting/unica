use super::*;

const ROOT: &str = "main:Configuration";
const INTERFACE: &str = "main:OrdinaryInterface.CovTemplate";

fn root_reference_fixture() -> Fixture {
    let fixture = Fixture::new("workspace-root-comments");
    let interfaces = fixture.source.join("Interfaces");
    std::fs::create_dir_all(&interfaces).unwrap();
    std::fs::copy(
        acceptance_fixtures().join("workspace-ordinary-interface/src/Interfaces/CovTemplate.xml"),
        interfaces.join("CovTemplate.xml"),
    )
    .unwrap();
    let root = fixture.source.join("Configuration.xml");
    let xml = std::fs::read_to_string(&root).unwrap();
    let prepared = xml
        .replacen(
            "<DefaultRunMode>ManagedApplication</DefaultRunMode>",
            "<DefaultRunMode>OrdinaryApplication</DefaultRunMode>",
            1,
        )
        .replacen(
            "</ChildObjects>",
            "<Interface>CovTemplate</Interface></ChildObjects>",
            1,
        );
    assert_ne!(prepared, xml);
    std::fs::write(&root, prepared).unwrap();
    fixture
}

fn set_root_references(values: Value) -> Value {
    json!({"at":ROOT,"ops":[{"op":"props.set","args":{"values":values}}]})
}

#[test]
fn configuration_references_can_assign_and_clear_only_their_platform_spans() {
    let fixture = root_reference_fixture();
    let runtime = runtime();
    let before = source_snapshot(&fixture);
    let can = view(&runtime, &fixture, "main:Configuration.Can.props.set");
    let fields = &can["props"]["contract"]["argsSchema"]["properties"]["values"]["properties"];
    for name in [
        "defaultStyle",
        "defaultInterface",
        "defaultReportAppearanceTemplate",
    ] {
        assert_eq!(fields[name]["type"], "string");
    }

    publish(
        &runtime,
        &fixture,
        set_root_references(json!({
            "DefaultStyle": "main:Style.Probe",
            "defaultInterface": INTERFACE,
            "defaultReportAppearanceTemplate": "main:CommonTemplate.Probe",
        })),
    );

    let original = std::str::from_utf8(&before[Path::new("Configuration.xml")]).unwrap();
    let edited = original
        .replace("<DefaultStyle/>", "<DefaultStyle>Style.Probe</DefaultStyle>")
        .replace(
            "<DefaultInterface/>",
            "<DefaultInterface>Interface.CovTemplate</DefaultInterface>",
        )
        .replace(
            "<DefaultReportAppearanceTemplate/>",
            "<DefaultReportAppearanceTemplate>CommonTemplate.Probe</DefaultReportAppearanceTemplate>",
        );
    assert_ne!(edited, original);
    let mut assigned = before.clone();
    assigned.insert(PathBuf::from("Configuration.xml"), edited.into_bytes());
    assert_eq!(source_snapshot(&fixture), assigned);

    let relation_at = "main:Configuration.Relation";
    let node = view(&runtime, &fixture, ROOT);
    assert!(node["branches"].as_array().unwrap().iter().any(|branch|
        branch["at"] == relation_at && branch["count"] == 3), "{node}");
    let relations = view(&runtime, &fixture, relation_at);
    let items = relations["items"].as_array().unwrap();
    assert_eq!(items.len(), 3);
    for (relation, target, kind) in [
        ("defaultStyle", "main:Style.Probe", "Style"),
        ("defaultInterface", INTERFACE, "OrdinaryInterface"),
        ("defaultReportAppearanceTemplate", "main:CommonTemplate.Probe", "CommonTemplate"),
    ] {
        let row = items.iter().find(|row| row["relation"] == relation).unwrap();
        assert_eq!(row["at"], target);
        assert_eq!(row["kind"], kind);
        assert_eq!(view(&runtime, &fixture, target)["at"], target);
    }

    let refused = call(
        &runtime,
        &fixture,
        ToolIdentity::Apply,
        json!({"at":"main:CommonTemplate.Probe","ops":[{"op":"object.remove"}]}),
    );
    assert!(!refused.ok, "{refused:?}");
    assert!(
        refused.summary.contains("DefaultReportAppearanceTemplate"),
        "{refused:?}"
    );
    assert_eq!(source_snapshot(&fixture), assigned);

    publish(
        &runtime,
        &fixture,
        set_root_references(json!({
            "defaultStyle": "",
            "DefaultInterface": "",
            "defaultReportAppearanceTemplate": "",
        })),
    );
    assert_eq!(source_snapshot(&fixture), before);
    assert_eq!(view(&runtime, &fixture, relation_at)["items"], json!([]));
}

#[test]
fn configuration_reference_preview_refuses_missing_wrong_kind_and_foreign_targets_atomically() {
    let fixture = root_reference_fixture();
    let runtime = runtime();
    let before = source_snapshot(&fixture);
    for target in [
        "main:OrdinaryInterface.NotRegistered",
        "main:Style.Probe",
        "foreign:OrdinaryInterface.CovTemplate",
    ] {
        let refused = call(
            &runtime,
            &fixture,
            ToolIdentity::Apply,
            set_root_references(json!({
                "defaultStyle": "main:Style.Probe",
                "defaultInterface": target,
            })),
        );
        assert!(!refused.ok, "{target}: {refused:?}");
        assert!(refused
            .data
            .as_ref()
            .and_then(|data| data.get("executionToken"))
            .is_none());
        assert_eq!(source_snapshot(&fixture), before);
    }

    std::fs::remove_file(fixture.source.join("Interfaces/CovTemplate.xml")).unwrap();
    let damaged = source_snapshot(&fixture);
    let refused = call(
        &runtime,
        &fixture,
        ToolIdentity::Apply,
        set_root_references(json!({"defaultInterface": INTERFACE})),
    );
    assert!(!refused.ok, "{refused:?}");
    assert!(refused.summary.contains("descriptor"), "{refused:?}");
    assert_eq!(source_snapshot(&fixture), damaged);
}

#[test]
fn configuration_reference_publication_refuses_a_target_lost_after_preview() {
    let fixture = root_reference_fixture();
    let runtime = runtime();
    let request = set_root_references(json!({"defaultInterface":INTERFACE}));
    let preview = call(&runtime, &fixture, ToolIdentity::Apply, request);
    assert!(preview.ok, "{preview:?}");
    let token = preview.data.unwrap()["executionToken"].clone();

    std::fs::remove_file(fixture.source.join("Interfaces/CovTemplate.xml")).unwrap();
    let before = source_snapshot(&fixture);
    let failed = call(
        &runtime,
        &fixture,
        ToolIdentity::Apply,
        json!({"executionToken":token}),
    );
    assert!(!failed.ok, "{failed:?}");
    assert_eq!(source_snapshot(&fixture), before);
}

/// rule: INV.CACHE.RETAINED-APPLY-REVISION-ROLLBACK
#[test]
fn configuration_reference_publication_restores_the_root_after_a_late_failure() {
    use unica_workspace::compile_transaction::{
        set_retained_apply_failpoint, RetainedApplyFailpoint,
    };
    let fixture = root_reference_fixture();
    let runtime = runtime();
    let before = source_snapshot(&fixture);
    let request = set_root_references(json!({"defaultInterface":INTERFACE}));
    let preview = call(&runtime, &fixture, ToolIdentity::Apply, request.clone());
    assert!(preview.ok, "{preview:?}");
    assert_eq!(source_snapshot(&fixture), before);
    let data = preview.data.unwrap();
    set_retained_apply_failpoint(RetainedApplyFailpoint::AfterAllPostimages);
    let failed = call(
        &runtime,
        &fixture,
        ToolIdentity::Apply,
        json!({"executionToken":data["executionToken"]}),
    );
    assert!(!failed.ok, "{failed:?}");
    assert_eq!(source_snapshot(&fixture), before);
    publish(&runtime, &fixture, request);
    assert_ne!(source_snapshot(&fixture), before);
}

fn source_snapshot(fixture: &Fixture) -> SourceImage {
    unica_workspace::test_support::tree_snapshot(&fixture.source)
}

/// rule: INV.SOURCE.RELATIONS-ANSWER-IN-ONE-BRANCH
#[test]
fn configuration_relation_reports_broken_unsupported_and_missing_targets() {
    let fixture = root_reference_fixture();
    let path = fixture.source.join("Configuration.xml");
    let xml = std::fs::read_to_string(&path).unwrap()
        .replace("<DefaultStyle/>", "<DefaultStyle><Broken/></DefaultStyle>")
        .replace("<DefaultInterface/>", "<DefaultInterface>Role.Bad</DefaultInterface>")
        .replace("<DefaultReportAppearanceTemplate/>", "<DefaultReportAppearanceTemplate>CommonTemplate.Absent</DefaultReportAppearanceTemplate>");
    std::fs::write(path, xml).unwrap();
    let node = view(&runtime(), &fixture, "main:Configuration.Relation");
    assert_eq!(node["items"], json!([]));
    let limits=node["limits"].as_array().unwrap();
    for diagnostic in ["malformed DefaultStyle", "unsupported reference kind in DefaultInterface", "main:CommonTemplate.Absent"] {
        assert!(limits.iter().any(|limit| limit.as_str().is_some_and(|text| text.contains(diagnostic))), "{node}");
    }
}

/// rule: INV.SOURCE.LARGE-CONFIGURATION-ROOT-VIEW
/// rule: INV.SOURCE.RELATIONS-ANSWER-IN-ONE-BRANCH
#[test]
fn medium_configuration_relation_count_matches_verified_rows_for_both_readers() {
    for large in [false, true] {
        let fixture = root_reference_fixture();
        let path = fixture.source.join("Configuration.xml");
        let mut xml = std::fs::read_to_string(&path).unwrap();
        if large {
            xml = xml.replace("</MetaDataObject>", &format!("<!--{}--></MetaDataObject>", "x".repeat(8 * 1024 * 1024)));
        }
        for assigned in [false, true] {
            if assigned {
                xml = xml.replace("<DefaultStyle/>", "<DefaultStyle>Style.Probe</DefaultStyle>")
                    .replace("<DefaultInterface/>", "<DefaultInterface>Interface.Absent</DefaultInterface>");
            }
            std::fs::write(&path, &xml).unwrap();
            let runtime = runtime();
            let root = view(&runtime, &fixture, ROOT);
            let branch = root["branches"].as_array().unwrap().iter()
                .find(|row| row["at"] == "main:Configuration.Relation").unwrap();
            let relations = view(&runtime, &fixture, branch["at"].as_str().unwrap());
            let items = relations["items"].as_array().unwrap();
            assert_eq!(items.len(), usize::from(assigned));
            assert_eq!(branch["count"], items.len());
            if assigned {
                assert_eq!(items[0]["at"], "main:Style.Probe");
                assert!(limits(&relations).iter().any(|text| text.contains("OrdinaryInterface.Absent")));
            }
        }
    }
}

#[test]
fn configuration_relation_ignores_unrelated_command_interface() {
    let fixture = root_reference_fixture();
    let descriptor = fixture.source.join("Configuration.xml");
    let xml = std::fs::read_to_string(&descriptor).unwrap();
    std::fs::write(descriptor, xml.replace("<DefaultStyle/>", "<DefaultStyle>Style.Probe</DefaultStyle>")).unwrap();
    let ext = fixture.source.join("Ext");
    std::fs::create_dir_all(&ext).unwrap();
    std::fs::write(ext.join("CommandInterface.xml"), b"<broken").unwrap();
    let node = view(&runtime(), &fixture, "main:Configuration.Relation");
    assert!(node["items"].as_array().unwrap().iter().any(|row| row["at"]=="main:Style.Probe"), "{node}");
}
