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
