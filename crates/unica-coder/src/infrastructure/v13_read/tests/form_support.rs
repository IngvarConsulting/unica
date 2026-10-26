use super::*;
use serde_json::Value;

#[test]
fn canonical_form_support_keeps_own_rule_and_owner_prohibition() {
    use unica_metadata_designer::support::support_root_uuid_from_bytes;
    for (owner_rule, form_rule, editing, support, safe) in [
        (1, 0, 0, "locked", false),
        (0, 1, 0, "editableWithSupport", false),
        (1, 1, 0, "editableWithSupport", true),
        (1, 2, 0, "removedFromSupport", true),
        (1, 1, 1, "configurationReadOnly", false),
    ] {
        let fixture = RealReaderFixture::new();
        let uuid = |path: &str| {
            support_root_uuid_from_bytes(&fs::read(fixture.source.join(path)).unwrap()).unwrap()
        };
        let owner = uuid("Reports/ParityReport.xml");
        let form = uuid("Reports/ParityReport/Forms/MainForm.xml");
        write(
            &fixture.source.join("Ext/ParentConfigurations.bin"),
            &format!("{{6,{editing},1,{owner_rule},0,{owner},{form_rule},0,{form}}}"),
        );
        let service = fixture.view_service();
        for _ in 0..2 {
            let result =
                service.view(ViewRequest::new("main:Report.ParityReport.Form.MainForm").unwrap());
            assert!(result.ok);
            let props = &result.data.as_ref().unwrap()["props"];
            assert_eq!(props["support"], support);
            assert_eq!(
                props["supportRule"],
                ["locked", "editableWithSupport", "removedFromSupport"][form_rule]
            );
            assert_eq!(props["directEditSafe"], safe);
            assert!(props.get("ownerSupport").is_none(), "{props}");
            // The owner's own state is read from the owner in the same exact
            // vocabulary as the form (#2158).
            let owner = service.view(ViewRequest::new("main:Report.ParityReport").unwrap());
            assert!(owner.ok);
            assert_eq!(
                owner.data.as_ref().unwrap()["props"]["support"],
                if editing == 1 {
                    "configurationReadOnly"
                } else {
                    ["locked", "editableWithSupport"][owner_rule]
                }
            );
        }
    }
}

#[test]
fn canonical_form_support_unknown_is_not_safe_and_absence_has_a_reason() {
    for (marker, support, reason, safe) in [
        (None, "notSupported", "markerAbsent", None),
        (
            Some("{6,0,1,1,0,33333333-3333-4333-8333-333333333333}"),
            "notSupported",
            "ruleAbsent",
            None,
        ),
        (
            Some(""),
            "removedFromSupport",
            "configurationRemoved",
            Some(true),
        ),
    ] {
        let fixture = RealReaderFixture::new();
        if let Some(marker) = marker {
            write(&fixture.source.join("Ext/ParentConfigurations.bin"), marker);
        }
        let result = fixture
            .view_service()
            .view(ViewRequest::new("main:Report.ParityReport.Form.MainForm").unwrap());
        assert!(result.ok);
        let props = &result.data.as_ref().unwrap()["props"];
        assert_eq!(props["support"], support);
        assert_eq!(props["supportRule"], Value::Null);
        assert_eq!(props["supportRuleReason"], reason);
        assert_eq!(props["directEditSafe"], json!(safe));
    }
}

#[test]
fn canonical_form_support_common_form_has_no_invented_owner_in_configuration_or_extension() {
    for extension in [false, true] {
        let fixture = RealReaderFixture::new();
        install_form_metadata_test_sources(&fixture);
        write(
            &fixture.source.join("Ext/ParentConfigurations.bin"),
            "{6,0,1,1,0,e9455333-3333-4333-8333-333333333333}",
        );
        let authority = if extension {
            fixture.extension_read_authority()
        } else {
            fixture.read_authority()
        };
        let at = QualifiedAddress::parse("main:CommonForm.ОбщаяФорма").unwrap();
        let admitted = authority.snapshot(&at).unwrap();
        let node = authority.read_exact(&at, &admitted).unwrap();
        let node = serde_json::to_value(node).unwrap();
        assert_eq!(node["props"]["supportRule"], "editableWithSupport");
        assert!(node["props"].get("ownerSupport").is_none());
        assert_eq!(node["props"]["directEditSafe"], true);
        assert_eq!(authority.support_state_read_count(), 1);
    }
}

#[test]
fn canonical_form_support_refuses_invalid_marker_and_missing_uuid_even_when_globally_locked() {
    for case in ["invalid", "directory", "missingUuid"] {
        let fixture = RealReaderFixture::new();
        let marker = fixture.source.join("Ext/ParentConfigurations.bin");
        match case {
            "invalid" => write(&marker, "{9,0,1}, invalid support header"),
            "directory" => fs::create_dir_all(&marker).unwrap(),
            _ => {
                write(&marker, "{6,1,1,1,0,33333333-3333-4333-8333-333333333333}");
                let descriptor = fixture
                    .source
                    .join("Reports/ParityReport/Forms/MainForm.xml");
                let text = fs::read_to_string(&descriptor).unwrap();
                let uuid =
                    unica_metadata_designer::support::support_root_uuid_from_bytes(text.as_bytes())
                        .unwrap();
                write(&descriptor, &text.replace(&format!("uuid=\"{uuid}\""), ""));
            }
        }
        let result = fixture
            .view_service()
            .view(ViewRequest::new("main:Report.ParityReport.Form.MainForm").unwrap());
        assert!(!result.ok, "{case}");
        assert!(result.data.is_none());
    }
}

#[test]
fn canonical_form_support_facts_do_not_depend_on_editing_policy() {
    for policy in ["deny", "warn", "off"] {
        let fixture = RealReaderFixture::new();
        install_form_metadata_test_sources(&fixture);
        write(
            &fixture.source.join("Ext/ParentConfigurations.bin"),
            "{6,0,1,0,0,e9455333-3333-4333-8333-333333333333}",
        );
        write(
            &fixture.context.workspace_root.join(".v8-project.json"),
            &format!(r#"{{"editingAllowedCheck":"{policy}"}}"#),
        );
        let result = fixture
            .view_service()
            .view(ViewRequest::new("main:CommonForm.ОбщаяФорма").unwrap());
        assert!(result.ok);
        let props = &result.data.as_ref().unwrap()["props"];
        assert_eq!(props["supportRule"], "locked");
        assert_eq!(props["directEditSafe"], false);
    }
}

#[test]
fn canonical_form_support_unknown_owner_does_not_turn_editable_child_into_safe() {
    let fixture = RealReaderFixture::new();
    let descriptor = fs::read(
        fixture
            .source
            .join("Reports/ParityReport/Forms/MainForm.xml"),
    )
    .unwrap();
    let uuid = unica_metadata_designer::support::support_root_uuid_from_bytes(&descriptor).unwrap();
    write(
        &fixture.source.join("Ext/ParentConfigurations.bin"),
        &format!("{{6,0,1,1,0,{uuid}}}"),
    );
    let result = fixture
        .view_service()
        .view(ViewRequest::new("main:Report.ParityReport.Form.MainForm").unwrap());
    assert!(result.ok);
    let props = &result.data.as_ref().unwrap()["props"];
    assert_eq!(props["supportRule"], "editableWithSupport");
    assert_eq!(props["directEditSafe"], Value::Null);
    let owner = fixture
        .view_service()
        .view(ViewRequest::new("main:Report.ParityReport").unwrap());
    assert!(owner.ok);
    // The owner has no rule of its own: its exact state is `notSupported`;
    // the unknown owner is the form's `directEditSafe: null` above.
    assert_eq!(
        owner.data.as_ref().unwrap()["props"]["support"],
        "notSupported"
    );
}

#[test]
fn canonical_form_support_refuses_linked_marker_and_ext() {
    use unica_workspace::test_support::{
        create_directory_link_fixture_for_test, create_file_link_fixture_for_test,
        FileLinkFixtureOutcome,
    };
    for directory in [false, true] {
        let fixture = RealReaderFixture::new();
        let foreign = fixture.source.join("foreign");
        write(
            &foreign.join("ParentConfigurations.bin"),
            "{6,0,1,1,0,e9455333-3333-4333-8333-333333333333}",
        );
        let ext = fixture.source.join("Ext");
        let outcome = if directory {
            if ext.exists() {
                fs::remove_dir_all(&ext).unwrap();
            }
            create_directory_link_fixture_for_test(&foreign, &ext).unwrap()
        } else {
            fs::create_dir_all(&ext).unwrap();
            create_file_link_fixture_for_test(
                foreign.join("ParentConfigurations.bin"),
                ext.join("ParentConfigurations.bin"),
            )
            .unwrap()
        };
        if outcome != FileLinkFixtureOutcome::Created {
            unica_process::test_support::skip_without_host_capability(
                unica_process::test_support::HostCapability::Symlink,
                "creating a link fixture needs the symlink privilege",
            );
            return;
        }
        let result = fixture
            .view_service()
            .view(ViewRequest::new("main:Report.ParityReport.Form.MainForm").unwrap());
        assert!(!result.ok);
        assert!(result.data.is_none());
    }
}

#[test]
fn canonical_form_support_details_do_not_change_legacy_payload() {
    use unica_metadata::support_state::{ObjectSupportData, ObjectSupportState};
    let data = unica_forms_designer::codec::parse_form_info_xml(
        r#"<Form xmlns="http://v8.1c.ru/8.3/xcf/logform" version="2.20"><ChildItems/></Form>"#,
        "Main".to_string(),
        String::new(),
        ObjectSupportData {
            state: ObjectSupportState::Locked,
            direct_edit_safe: Some(false),
        },
    )
    .unwrap();
    let payload = serde_json::to_value(data).unwrap();
    assert!(payload.get("supportDetails").is_none());
    assert_eq!(
        payload["support"],
        json!({"state": "locked", "directEditSafe": false})
    );
}
