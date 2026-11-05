use super::*;

#[test]
fn configuration_exposes_explicit_panel_instances_and_definitions() {
    let fixture = RealReaderFixture::new();
    let path = fixture.source.join("Ext/ClientApplicationInterface.xml");
    let xml = r#"<ClientApplicationInterface xmlns="http://v8.1c.ru/8.2/managed-application/core" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" xsi:type="InterfaceLayouter"><top><group id="00000000-0000-0000-0000-000000000001"><group><panel id="00000000-0000-0000-0000-000000000002"><uuid>cbab57f2-a0f3-4f0a-89ea-4cb19570ab75</uuid></panel></group></group></top><left/><panelDef id="cbab57f2-a0f3-4f0a-89ea-4cb19570ab75"/><panelDef id="13322b22-3960-4d68-93a6-fe2dd7f28ca3"/></ClientApplicationInterface>"#;
    write(&path, xml);
    let before = fs::read(&path).unwrap();
    let service = fixture.view_service();
    let result = service.view(ViewRequest::new("main:Configuration").unwrap());
    assert!(result.ok, "{:?}", refusal_codes(&result));
    let data = result.data.unwrap();
    assert_eq!(
        data["props"]["panelLayout"],
        json!({
            "state": "complete",
            "areas": [
                {"side":"top", "items":[{"kind":"group", "id":"00000000-0000-0000-0000-000000000001", "parent":null}, {"kind":"group", "parent":0}, {"kind":"panel", "id":"00000000-0000-0000-0000-000000000002", "definitionId":"cbab57f2-a0f3-4f0a-89ea-4cb19570ab75", "resolved":true, "parent":1}]},
                {"side":"left", "items":[]}
            ],
            "definitions":[{"id":"cbab57f2-a0f3-4f0a-89ea-4cb19570ab75"},{"id":"13322b22-3960-4d68-93a6-fe2dd7f28ca3"}],
            "limits":[]
        })
    );
    assert_eq!(data["props"]["panels"], json!({"top":[["open"]]}));
    assert_eq!(fs::read(path).unwrap(), before);
}

fn install_layout_fixture(fixture: &RealReaderFixture) {
    for relative in [
        "Configuration.xml",
        "Languages/Русский.xml",
        "Ext/ClientApplicationInterface.xml",
        "Ext/CommandInterface.xml",
        "Ext/HomePageWorkArea.xml",
    ] {
        copy_fixture(
            &format!("acceptance/workspace-panel-layout/src/{relative}"),
            &fixture.source.join(relative),
        );
    }
}

#[test]
fn panel_topology_preserves_all_root_order_and_home_page_facts_in_both_readers() {
    let expected: serde_json::Value = serde_json::from_str(&fixture_text(
        "acceptance/workspace-panel-layout/expected-root-props.json",
    ))
    .unwrap();
    for streamed in [false, true] {
        let fixture = RealReaderFixture::new();
        install_layout_fixture(&fixture);
        if streamed {
            let path = fixture.source.join("Configuration.xml");
            let xml = fs::read_to_string(&path).unwrap();
            write(
                &path,
                &xml.replace(
                    "</MetaDataObject>",
                    &format!("<!--{}--></MetaDataObject>", "x".repeat(8 * 1024 * 1024)),
                ),
            );
        }
        let before = unica_workspace::test_support::tree_snapshot(&fixture.source);
        let service = fixture.view_service();
        let read = service.view(ViewRequest::new("main:Configuration").unwrap());
        assert!(read.ok, "streamed={streamed}: {:?}", refusal_codes(&read));
        let data = read.data.unwrap();
        for key in ["panelLayout", "panels", "interface", "homePage"] {
            assert_eq!(
                data["props"][key], expected[key],
                "streamed={streamed} {key}"
            );
        }
        assert_eq!(
            unica_workspace::test_support::tree_snapshot(&fixture.source),
            before
        );
    }
}

#[test]
fn panel_observation_distinguishes_missing_empty_unreadable_and_unsupported_documents() {
    let fixture = RealReaderFixture::new();
    let path = fixture.source.join("Ext/ClientApplicationInterface.xml");
    if path.exists() {
        fs::remove_file(&path).unwrap();
    }
    let missing = fixture
        .view_service()
        .view(ViewRequest::new("main:Configuration").unwrap());
    assert!(missing.ok);
    assert!(missing.data.unwrap()["props"].get("panelLayout").is_none());
    write(
        &path,
        r#"<ClientApplicationInterface xmlns="http://v8.1c.ru/8.2/managed-application/core"/>"#,
    );
    let empty = fixture
        .view_service()
        .view(ViewRequest::new("main:Configuration").unwrap());
    assert!(empty.ok);
    assert_eq!(
        empty.data.unwrap()["props"]["panelLayout"],
        json!({"state":"complete","areas":[],"definitions":[],"limits":[]})
    );
    write(
        &path,
        r#"<ClientApplicationInterface xmlns="http://v8.1c.ru/8.2/managed-application/core"><unknown/></ClientApplicationInterface>"#,
    );
    let unsupported = fixture
        .view_service()
        .view(ViewRequest::new("main:Configuration").unwrap());
    assert!(unsupported.ok);
    let value = unsupported.data.unwrap()["props"]["panelLayout"].clone();
    assert_eq!(value["state"], "unsupported");
    assert!(value.get("areas").is_none());
    assert!(!value["limits"].as_array().unwrap().is_empty());
    for xml in [
        "",
        "<bad",
        r#"<ClientApplicationInterface xmlns="urn:foreign"/>"#,
        "<Other/>",
    ] {
        write(&path, xml);
        let unreadable = fixture
            .view_service()
            .view(ViewRequest::new("main:Configuration").unwrap());
        assert!(!unreadable.ok, "{xml}: {unreadable:?}");
        assert!(!unreadable.diagnostics.is_empty());
    }
}

#[test]
fn deeply_nested_panel_groups_reach_public_view_without_a_depth_quota() {
    let fixture = RealReaderFixture::new();
    let depth = 256;
    let xml = format!(
        r#"<ClientApplicationInterface xmlns="http://v8.1c.ru/8.2/managed-application/core"><top>{}</top><top><group/></top></ClientApplicationInterface>"#,
        "<group>".repeat(depth) + &"</group>".repeat(depth)
    );
    write(
        &fixture.source.join("Ext/ClientApplicationInterface.xml"),
        &xml,
    );
    let service = fixture.view_service();
    let read = service.view(ViewRequest::new("main:Configuration").unwrap());
    assert!(read.ok, "{:?}", refusal_codes(&read));
    let data = read.data.unwrap();
    let layout = &data["props"]["panelLayout"];
    assert_eq!(layout["state"], "complete");
    let items = layout["areas"][0]["items"].as_array().unwrap();
    assert_eq!(items.len(), depth);
    for (index, item) in items.iter().enumerate() {
        assert_eq!(item["kind"], "group");
        assert_eq!(
            item["parent"],
            index
                .checked_sub(1)
                .map_or(serde_json::Value::Null, |value| json!(value))
        );
        assert!(item.get("id").is_none());
    }
    assert_eq!(
        layout["areas"][1]["items"],
        json!([{"kind":"group","parent":null}])
    );
    // The old writable summary cannot represent this topology; the complete
    // read does not pretend that flattening it would be a safe write input.
    assert!(data["props"].get("panels").is_none());
}
