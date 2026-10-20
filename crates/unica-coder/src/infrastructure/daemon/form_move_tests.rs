use super::actor_capacity_tests::direct_v5;
use super::*;
use crate::application::invocation_store::ToolIdentity;

#[test]
fn canonical_form_move_composes_tabs_events_and_preserves_atomic_noops() {
    let workspace = tempfile::tempdir().unwrap();
    let source = workspace.path().join("src");
    std::fs::create_dir_all(source.join("Documents")).unwrap();
    std::fs::write(
        workspace.path().join("v8project.yaml"),
        "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
    )
    .unwrap();
    std::fs::write(
            source.join("Configuration.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Configuration><Properties><Name>Store</Name><DefaultLanguage>Language.Русский</DefaultLanguage></Properties><ChildObjects><Language>Русский</Language><Document>Order</Document></ChildObjects></Configuration></MetaDataObject>"#,
        )
        .unwrap();
    std::fs::create_dir_all(source.join("Languages")).unwrap();
    std::fs::write(
            source.join("Languages/Русский.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Language uuid="33333333-3333-4333-8333-333333333333"><Properties><Name>Русский</Name><Comment/><LanguageCode>ru</LanguageCode></Properties></Language></MetaDataObject>"#,
        )
        .unwrap();
    std::fs::write(
            source.join("Documents/Order.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" xmlns:app="http://v8.1c.ru/8.2/managed-application/core" xmlns:cfg="http://v8.1c.ru/8.1/data/enterprise/current-config" xmlns:v8="http://v8.1c.ru/8.1/data/core" xmlns:xr="http://v8.1c.ru/8.3/xcf/readable" xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" version="2.20"><Document uuid="11111111-1111-4111-8111-111111111111"><Properties><Name>Order</Name><Synonym/><Comment/></Properties><ChildObjects/></Document></MetaDataObject>"#,
        )
        .unwrap();
    let runtime = V5CanonicalInvocationRuntime::new(
        Arc::new(crate::infrastructure::daemon::v13_service::CanonicalV13ReadService::default()),
        Arc::new(TokioClock),
    );
    let workspace_hint = std::fs::canonicalize(workspace.path()).unwrap();
    let call = |tool, arguments| {
        let request =
            InvocationRequest::new(tool, arguments, workspace_hint.to_string_lossy(), 7_000)
                .unwrap();
        direct_v5(&runtime, request).unwrap()
    };
    let apply_pair = |at: &str, ops: serde_json::Value| {
        let before = crate::test_support::tree_snapshot(&source);
        let view_before = call(ToolIdentity::View, serde_json::json!({"at": at}));
        let preview = call(
            ToolIdentity::Apply,
            serde_json::json!({"at": at, "ops": ops}),
        );
        assert!(preview.ok, "preview failed: {preview:?}");
        assert_eq!(crate::test_support::tree_snapshot(&source), before);
        if view_before.ok {
            let view_after = call(ToolIdentity::View, serde_json::json!({"at": at}));
            assert!(view_after.ok, "{view_after:?}");
            assert_eq!(
                view_after.data, view_before.data,
                "preview changed the read model"
            );
        }
        let token = &preview.data.as_ref().unwrap()["executionToken"];
        assert!(token.is_string(), "{preview:?}");
        let published = call(
            ToolIdentity::Apply,
            serde_json::json!({"executionToken": token}),
        );
        assert!(published.ok, "publication failed: {published:?}");
        let plan_hash = &preview.data.as_ref().unwrap()["planHash"];
        assert!(plan_hash.is_string(), "{preview:?}");
        assert_eq!(plan_hash, &published.data.as_ref().unwrap()["planHash"]);
    };
    let at = "main:Document.Order.Form.Main";
    apply_pair(
        "main:Document.Order",
        serde_json::json!([{"op":"form.add","args":{"items":[{"name":"Main","type":"ObjectForm"}]}}]),
    );
    apply_pair(
        at,
        serde_json::json!([
            {"op":"attribute.add","args":{"items":[{"name":"Text","type":"String"}]}},
            {"op":"item.add","args":{"items":[{"name":"Existing","type":"Group"}]}},
            {"op":"item.add","args":{"at":format!("{at}.Item.Existing"),"items":[{"name":"Input","type":"InputField","path":"Text"}]}},
            {"op":"event.bind","args":{"at":format!("{at}.Item.Existing.Item.Input.Event.OnChange"),"values":{"handler":"InputПриИзменении"}}}
        ]),
    );
    let form_path = source.join("Documents/Order/Forms/Main/Ext/Form.xml");
    let before = std::fs::read_to_string(&form_path).unwrap();
    let before_doc = roxmltree::Document::parse(before.trim_start_matches('\u{feff}')).unwrap();
    let original_ids: std::collections::BTreeMap<_, _> = before_doc
        .descendants()
        .filter_map(|n| {
            Some((
                n.attribute("name")?.to_owned(),
                n.attribute("id")?.to_owned(),
            ))
        })
        .collect();
    apply_pair(
        at,
        serde_json::json!([
            {"op":"item.add","args":{"items":[{"name":"Tabs","type":"Pages"}]}},
            {"op":"item.add","args":{"at":format!("{at}.Item.Tabs"),"items":[{"name":"Primary","type":"Page"},{"name":"Secondary","type":"Page"}]}},
            {"op":"item.add","args":{"at":format!("{at}.Item.Tabs.Item.Secondary"),"items":[{"name":"Stationary","type":"Group"}]}},
            {"op":"item.move","args":{"at":format!("{at}.Item.Existing"),"values":{"into":"Primary"}}},
            {"op":"event.bind","args":{"at":format!("{at}.Item.Tabs.Item.Primary.Item.Existing.Item.Input.Event.StartChoice"),"values":{"handler":"MovedInputChoice"}}}
        ]),
    );
    let old_item = call(
        ToolIdentity::View,
        serde_json::json!({"at":format!("{at}.Item.Existing")}),
    );
    assert!(
        !old_item.ok,
        "old item path survived publication: {old_item:?}"
    );
    let moved_item_at = format!("{at}.Item.Tabs.Item.Primary.Item.Existing");
    let moved_item = call(ToolIdentity::View, serde_json::json!({"at":moved_item_at}));
    assert!(
        moved_item.ok,
        "new item path was not published: {moved_item:?}"
    );
    let after = std::fs::read_to_string(&form_path).unwrap();
    let doc = roxmltree::Document::parse(after.trim_start_matches('\u{feff}')).unwrap();
    let group = doc
        .descendants()
        .find(|n| n.attribute("name") == Some("Existing"))
        .unwrap();
    assert_eq!(
        group
            .parent_element()
            .unwrap()
            .parent_element()
            .unwrap()
            .attribute("name"),
        Some("Primary")
    );
    for (name, id) in &original_ids {
        let node = doc
            .descendants()
            .find(|n| n.attribute("name") == Some(name))
            .unwrap();
        assert_eq!(
            node.attribute("id"),
            Some(id.as_str()),
            "ID changed for {name}"
        );
    }
    let input = doc
        .descendants()
        .find(|n| n.attribute("name") == Some("Input"))
        .unwrap();
    assert_eq!(
        input.parent_element().unwrap().parent_element(),
        Some(group)
    );
    assert!(input.descendants().any(|n| n.has_tag_name("Event")
        && n.attribute("name") == Some("OnChange")
        && n.text() == Some("InputПриИзменении")));
    assert!(input.descendants().any(|n| n.has_tag_name("Event")
        && n.attribute("name") == Some("StartChoice")
        && n.text() == Some("MovedInputChoice")));
    for tag in ["ContextMenu", "ExtendedTooltip"] {
        assert_eq!(input.children().filter(|n| n.has_tag_name(tag)).count(), 1);
    }
    let checked = call(ToolIdentity::Check, serde_json::json!({"at":at}));
    assert!(checked.ok, "{checked:?}");
    assert_eq!(checked.data.as_ref().unwrap()["diagnosticCount"], 0);

    let cyclic = call(
        ToolIdentity::Apply,
        serde_json::json!({
            "at": format!("{at}.Item.Tabs"),
            "ops": [{"op":"item.move","args":{"values":{"into":"Primary"}}}]
        }),
    );
    assert!(!cyclic.ok, "cyclic move was accepted: {cyclic:?}");
    assert_eq!(cyclic.at.as_deref(), Some("ops[0].args.values.into"));
    assert_eq!(cyclic.diagnostics[0]["code"], "bad_value", "{cyclic:?}");
    assert_eq!(
        cyclic.diagnostics[0]["message"],
        "an element cannot be moved into itself or its descendant",
        "{cyclic:?}"
    );

    // Materialize the read model, then fail after a valid move: neither disk
    // contents nor the subsequent cached logical view may expose the move.
    let viewed_before = call(ToolIdentity::View, serde_json::json!({"at":at}));
    assert!(viewed_before.ok, "{viewed_before:?}");
    let tree_before = crate::test_support::tree_snapshot(&source);
    let refused = call(
        ToolIdentity::Apply,
        serde_json::json!({"at":at,"ops":[
            {"op":"item.move","args":{"at":format!("{at}.Item.Tabs.Item.Primary.Item.Existing"),"values":{"into":"Secondary"}}},
            {"op":"item.move","args":{"at":format!("{at}.Item.Missing"),"values":{"into":"Secondary"}}}
        ]}),
    );
    assert!(!refused.ok, "{refused:?}");
    assert!(refused
        .data
        .as_ref()
        .is_none_or(|d| d.get("executionToken").is_none()));
    assert!(refused.changed.is_empty(), "{refused:?}");
    assert_eq!(crate::test_support::tree_snapshot(&source), tree_before);
    let viewed_after = call(ToolIdentity::View, serde_json::json!({"at":at}));
    assert!(viewed_after.ok, "{viewed_after:?}");
    assert_eq!(viewed_after.data, viewed_before.data);

    // Exercise the element-address contract and preserve deliberately BOM-less
    // source bytes when the element already occupies the requested position.
    let no_bom = after.trim_start_matches('\u{feff}');
    std::fs::write(&form_path, no_bom).unwrap();
    let item_at = "main:Document.Order.Form.Main.Item.Tabs.Item.Primary.Item.Existing";
    let item_view = call(ToolIdentity::View, serde_json::json!({"at":item_at}));
    assert!(item_view.ok, "{item_view:?}");
    for wrong_at in [
        "main:Document.Order.Form.Main.Item.Existing",
        "main:Document.Order.Form.Main.Item.Tabs.Item.Secondary.Item.Existing",
    ] {
        let refused = call(
            ToolIdentity::Apply,
            serde_json::json!({"at":wrong_at,"ops":[
                {"op":"item.move","args":{"values":{"into":"Primary"}}}
            ]}),
        );
        assert!(!refused.ok, "incorrect source path accepted: {refused:?}");
        assert_eq!(std::fs::read(&form_path).unwrap(), no_bom.as_bytes());
    }
    let stale = call(
        ToolIdentity::Apply,
        serde_json::json!({"at":at,"ops":[
            {"op":"item.move","args":{"at":format!("{at}.Item.Tabs.Item.Primary.Item.Existing"),"values":{"into":"Secondary"}}},
            {"op":"item.move","args":{"at":item_at,"values":{"into":"Primary"}}}
        ]}),
    );
    assert!(!stale.ok, "stale source path accepted: {stale:?}");
    assert_eq!(stale.at.as_deref(), Some("ops[1].args.at"));
    assert_eq!(std::fs::read(&form_path).unwrap(), no_bom.as_bytes());
    for args in [
        serde_json::json!({"values":{}}),
        serde_json::json!({"values":{"into":false}}),
        serde_json::json!({"values":{"into":"MisspelledContainer"}}),
        serde_json::json!({"items":[{"name":"Existing","into":"Primary"}]}),
    ] {
        let refused = call(
            ToolIdentity::Apply,
            serde_json::json!({"at":item_at,"ops":[{"op":"item.move","args":args}]}),
        );
        assert!(!refused.ok, "invalid destination accepted: {refused:?}");
        assert_eq!(std::fs::read(&form_path).unwrap(), no_bom.as_bytes());
    }
    // The root must be requested explicitly. The second move addresses the
    // new path, proving that each operation observes preceding staged moves.
    apply_pair(
        at,
        serde_json::json!([
            {"op":"item.move","args":{"at":item_at,"values":{"into":null}}},
            {"op":"item.move","args":{"at":format!("{at}.Item.Existing"),"values":{"into":"Primary"}}}
        ]),
    );
    assert_eq!(std::fs::read(&form_path).unwrap(), no_bom.as_bytes());
    let noop = call(
        ToolIdentity::Apply,
        serde_json::json!({"at":item_at,"ops":[
            {"op":"item.move","args":{"values":{"into":"Primary"}}}
        ]}),
    );
    assert!(noop.ok, "{noop:?}");
    assert_eq!(std::fs::read(&form_path).unwrap(), no_bom.as_bytes());
    let published = call(
        ToolIdentity::Apply,
        serde_json::json!({"executionToken":noop.data.as_ref().unwrap()["executionToken"]}),
    );
    assert!(published.ok, "{published:?}");
    assert!(published.changed.is_empty(), "{published:?}");
    assert_eq!(published.data.as_ref().unwrap()["effects"], 0);
    assert_eq!(std::fs::read(&form_path).unwrap(), no_bom.as_bytes());

    // Publication derives events from the final file delta, not intermediate
    // moves. Both items consume the preceding item's staged XML.
    let restored = call(
        ToolIdentity::Apply,
        serde_json::json!({"at":at,"ops":[
            {"op":"item.move","args":{"at":item_at,"values":{"into":"Secondary"}}},
            {"op":"item.move","args":{"at":format!("{at}.Item.Tabs.Item.Secondary.Item.Existing"),"values":{"into":"Primary"}}}
        ]}),
    );
    assert!(restored.ok, "{restored:?}");
    let restored = call(
        ToolIdentity::Apply,
        serde_json::json!({
            "executionToken": restored.data.as_ref().unwrap()["executionToken"]
        }),
    );
    assert!(restored.ok, "{restored:?}");
    assert_eq!(restored.data.as_ref().unwrap()["effects"], 0);
    assert!(restored.changed.is_empty(), "{restored:?}");
    assert_eq!(std::fs::read(&form_path).unwrap(), no_bom.as_bytes());

    // A position no-op must still validate the existing document: duplicate
    // form element IDs cannot be hidden by returning early from the move.
    let group_id = original_ids.get("Existing").unwrap();
    let input_id = original_ids.get("Input").unwrap();
    let invalid = no_bom.replace(
        &format!("name=\"Input\" id=\"{input_id}\""),
        &format!("name=\"Input\" id=\"{group_id}\""),
    );
    assert_ne!(invalid, no_bom, "fixture must introduce a duplicate ID");
    std::fs::write(&form_path, &invalid).unwrap();
    let invalid_noop = call(
        ToolIdentity::Apply,
        serde_json::json!({"at":at,"ops":[
            {"op":"item.move","args":{"at":item_at,"values":{"into":"Primary"}}}
        ]}),
    );
    assert!(
        !invalid_noop.ok,
        "invalid form accepted by no-op: {invalid_noop:?}"
    );
    assert!(invalid_noop
        .data
        .as_ref()
        .is_none_or(|d| d.get("executionToken").is_none()));
    assert_eq!(std::fs::read(&form_path).unwrap(), invalid.as_bytes());
}

#[test]
fn column_group_and_its_columns_are_addressable_through_the_wire() {
    let workspace = tempfile::tempdir().unwrap();
    let source = workspace.path().join("src");
    std::fs::create_dir_all(source.join("Documents")).unwrap();
    std::fs::write(
        workspace.path().join("v8project.yaml"),
        "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
    )
    .unwrap();
    std::fs::write(
            source.join("Configuration.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Configuration><Properties><Name>Store</Name><DefaultLanguage>Language.Русский</DefaultLanguage></Properties><ChildObjects><Language>Русский</Language><Document>Order</Document></ChildObjects></Configuration></MetaDataObject>"#,
        )
        .unwrap();
    std::fs::create_dir_all(source.join("Languages")).unwrap();
    std::fs::write(
            source.join("Languages/Русский.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Language uuid="33333333-3333-4333-8333-333333333333"><Properties><Name>Русский</Name><Comment/><LanguageCode>ru</LanguageCode></Properties></Language></MetaDataObject>"#,
        )
        .unwrap();
    std::fs::write(
            source.join("Documents/Order.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" xmlns:app="http://v8.1c.ru/8.2/managed-application/core" xmlns:cfg="http://v8.1c.ru/8.1/data/enterprise/current-config" xmlns:v8="http://v8.1c.ru/8.1/data/core" xmlns:xr="http://v8.1c.ru/8.3/xcf/readable" xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" version="2.20"><Document uuid="11111111-1111-4111-8111-111111111111"><Properties><Name>Order</Name><Synonym/><Comment/></Properties><ChildObjects/></Document></MetaDataObject>"#,
        )
        .unwrap();
    let runtime = V5CanonicalInvocationRuntime::new(
        Arc::new(crate::infrastructure::daemon::v13_service::CanonicalV13ReadService::default()),
        Arc::new(TokioClock),
    );
    let workspace_hint = std::fs::canonicalize(workspace.path()).unwrap();
    let call = |tool, arguments| {
        let request =
            InvocationRequest::new(tool, arguments, workspace_hint.to_string_lossy(), 7_000)
                .unwrap();
        direct_v5(&runtime, request).unwrap()
    };
    let added = call(
        ToolIdentity::Apply,
        serde_json::json!({
            "at": "main:Document.Order",
            "ops": [{"op":"form.add","args":{"items":[{"name":"Main","type":"ObjectForm"}]}}]
        }),
    );
    assert!(added.ok, "{added:?}");
    let published = call(
        ToolIdentity::Apply,
        serde_json::json!({"executionToken":added.data.as_ref().unwrap()["executionToken"]}),
    );
    assert!(published.ok, "{published:?}");

    let form_path = source.join("Documents/Order/Forms/Main/Ext/Form.xml");
    let generated = std::fs::read_to_string(&form_path).unwrap();
    let with_columns = generated.replace(
        "\t<Attributes>\r\n",
        "\t<ChildItems>\r\n\t\t<Table name=\"Rows\" id=\"1\">\r\n\t\t\t<ContextMenu name=\"RowsMenu\" id=\"5\"/>\r\n\t\t\t<AutoCommandBar name=\"RowsBar\" id=\"6\"/>\r\n\t\t\t<SearchStringAddition name=\"RowsSearch\" id=\"7\"/>\r\n\t\t\t<ViewStatusAddition name=\"RowsStatus\" id=\"8\"/>\r\n\t\t\t<SearchControlAddition name=\"RowsControl\" id=\"9\"/>\r\n\t\t\t<ChildItems>\r\n\t\t\t\t<ColumnGroup name=\"First\" id=\"2\">\r\n\t\t\t\t\t<ChildItems>\r\n\t\t\t\t\t\t<InputField name=\"Column\" id=\"3\">\r\n\t\t\t\t\t\t\t<ContextMenu name=\"ColumnMenu\" id=\"10\"/>\r\n\t\t\t\t\t\t\t<ExtendedTooltip name=\"ColumnTip\" id=\"11\"/>\r\n\t\t\t\t\t\t</InputField>\r\n\t\t\t\t\t</ChildItems>\r\n\t\t\t\t</ColumnGroup>\r\n\t\t\t\t<ColumnGroup name=\"Second\" id=\"4\"/>\r\n\t\t\t</ChildItems>\r\n\t\t</Table>\r\n\t</ChildItems>\r\n\t<Attributes>\r\n",
    );
    assert_ne!(with_columns, generated, "the generated form has ChildItems");
    std::fs::write(&form_path, &with_columns).unwrap();

    let form_at = "main:Document.Order.Form.Main";
    let first_at = format!("{form_at}.Item.Rows.Item.First");
    let column_at = format!("{first_at}.Item.Column");
    let first = call(ToolIdentity::View, serde_json::json!({"at":first_at}));
    assert!(first.ok, "ColumnGroup is absent from view: {first:?}");
    assert_eq!(
        first.data.as_ref().unwrap()["props"]["tag"],
        "[ColumnGroup]"
    );
    let can = first.data.as_ref().unwrap()["can"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|entry| entry["op"].as_str())
        .collect::<std::collections::BTreeSet<_>>();
    assert!(can.contains("item.add"), "{first:?}");
    assert!(can.contains("item.move"), "{first:?}");
    assert!(can.contains("item.remove"), "{first:?}");
    let column = call(ToolIdentity::View, serde_json::json!({"at":column_at}));
    assert!(column.ok, "column is absent from its full path: {column:?}");
    let can = column.data.as_ref().unwrap()["can"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|entry| entry["op"].as_str())
        .collect::<std::collections::BTreeSet<_>>();
    assert!(can.contains("item.move"), "{column:?}");
    assert!(can.contains("item.remove"), "{column:?}");
    assert!(!can.contains("item.add"), "{column:?}");

    let moved = call(
        ToolIdentity::Apply,
        serde_json::json!({
            "at": column_at,
            "ops": [{"op":"item.move","args":{"values":{"into":"Second"}}}]
        }),
    );
    assert!(
        moved.ok,
        "move through the full wire address failed: {moved:?}"
    );
    let published = call(
        ToolIdentity::Apply,
        serde_json::json!({"executionToken":moved.data.as_ref().unwrap()["executionToken"]}),
    );
    assert!(published.ok, "{published:?}");
    let old = call(ToolIdentity::View, serde_json::json!({"at":column_at}));
    assert!(!old.ok, "the old column path survived: {old:?}");
    let new_at = format!("{form_at}.Item.Rows.Item.Second.Item.Column");
    let new = call(ToolIdentity::View, serde_json::json!({"at":new_at}));
    assert!(new.ok, "the new column path is absent: {new:?}");
    let xml = std::fs::read_to_string(&form_path).unwrap();
    let doc = roxmltree::Document::parse(xml.trim_start_matches('\u{feff}')).unwrap();
    let first = doc
        .descendants()
        .find(|node| node.attribute("name") == Some("First"))
        .unwrap();
    assert!(
        first
            .children()
            .all(|node| !node.has_tag_name("ChildItems")),
        "{xml}"
    );
}
