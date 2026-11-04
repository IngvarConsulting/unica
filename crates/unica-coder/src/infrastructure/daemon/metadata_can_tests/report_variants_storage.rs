use super::*;
const AT: &str = "main:Report.SyntheticReport";
const DESCRIPTOR: &str = "Reports/SyntheticReport.xml";
fn edit(value: Value) -> Value {
    json!({"at":AT,"ops":[{"op":"props.set","args":{"values":{"VariantsStorage":value}}}]})
}
fn outside_field(bytes: &[u8]) -> Vec<u8> {
    let text = std::str::from_utf8(bytes).unwrap();
    let body = text.trim_start_matches('\u{feff}');
    let prefix = text.len() - body.len();
    let doc = roxmltree::Document::parse(body).unwrap();
    let field = doc
        .descendants()
        .find(|n| n.has_tag_name(("http://v8.1c.ru/8.3/MDClasses", "VariantsStorage")))
        .unwrap();
    let range = field.range();
    [
        &bytes[..range.start + prefix],
        b"<VariantsStorage/>",
        &bytes[range.end + prefix..],
    ]
    .concat()
}
#[test]
fn report_variant_storage_assign_replace_and_noop_preserve_all_neighbors() {
    let fixture = Fixture::new("workspace-report-variants-storage");
    let runtime = runtime();
    let baseline = unica_workspace::test_support::tree_snapshot(&fixture.source);
    let can = contract(&runtime, &fixture, AT, "props.set");
    let schema = &can["argsSchema"]["properties"]["values"]["properties"]["VariantsStorage"];
    assert_eq!(schema["type"], "string");
    assert!(schema["pattern"]
        .as_str()
        .unwrap()
        .starts_with("^SettingsStorage"));
    for (index, name) in ["StorageA", "StorageB", "StorageB"].into_iter().enumerate() {
        let before = unica_workspace::test_support::tree_snapshot(&fixture.source);
        let value = format!("SettingsStorage.{name}");
        publish(&runtime, &fixture, edit(json!(value)));
        let viewed = view(&runtime, &fixture, AT);
        assert_eq!(viewed["props"]["VariantsStorage"], value, "{viewed}");
        assert_eq!(
            viewed["props"]["SettingsStorage"],
            "SettingsStorage.StorageB"
        );
        let relation = view(&runtime, &fixture, &format!("{AT}.Relation"));
        assert!(
            relation["items"]
                .as_array()
                .unwrap()
                .iter()
                .any(|item| item["relation"] == "variantsStorage"
                    && item["at"] == format!("main:SettingsStorage.{name}")),
            "{relation}"
        );
        let mut after = unica_workspace::test_support::tree_snapshot(&fixture.source);
        let changed = after.remove(&PathBuf::from(DESCRIPTOR)).unwrap();
        let mut expected = baseline.clone();
        let old = expected.remove(&PathBuf::from(DESCRIPTOR)).unwrap();
        assert_eq!(
            after, expected,
            "storage/module/form/template/registration bytes never change"
        );
        assert_eq!(
            outside_field(&changed),
            outside_field(&old),
            "Report.SettingsStorage, DCS, forms, identity and commands stay byte-identical"
        );
        if index == 2 {
            assert_eq!(
                before,
                unica_workspace::test_support::tree_snapshot(&fixture.source),
                "equivalent publication preserves the complete source image"
            );
        }
    }
    publish(
        &runtime,
        &fixture,
        json!({"at":AT,"ops":[
            {"op":"props.set","args":{"values":{"VariantsStorage":"SettingsStorage.StorageA"}}},
            {"op":"props.set","args":{"values":{"VariantsStorage":"SettingsStorage.StorageB"}}}
        ]}),
    );
    assert_eq!(
        view(&runtime, &fixture, AT)["props"]["VariantsStorage"],
        "SettingsStorage.StorageB"
    );
}
#[test]
fn report_variant_storage_rejects_wrong_targets_and_damaged_existing_fields_without_writes() {
    for value in [
        json!(false),
        json!(null),
        json!(""),
        json!("Catalog.StorageA"),
        json!("main:SettingsStorage.StorageA"),
        json!("other:SettingsStorage.StorageA"),
        json!("SettingsStorage.StorageA.Form.Main"),
        json!("SettingsStorage.Missing"),
    ] {
        let fixture = Fixture::new("workspace-report-variants-storage");
        let runtime = runtime();
        let before = unica_workspace::test_support::tree_snapshot(&fixture.source);
        let refused = call(&runtime, &fixture, ToolIdentity::Apply, edit(value));
        assert!(!refused.ok, "{refused:?}");
        assert_eq!(
            unica_workspace::test_support::tree_snapshot(&fixture.source),
            before
        );
    }
    for field in [
        "",
        "<VariantsStorage/><VariantsStorage/>",
        "<VariantsStorage xsi:nil=\"true\"/>",
        "<VariantsStorage>Catalog.StorageA</VariantsStorage>",
        "<VariantsStorage xmlns=\"urn:foreign\"/>",
        "<VariantsStorage><Nested/></VariantsStorage>",
    ] {
        let fixture = Fixture::new("workspace-report-variants-storage");
        let runtime = runtime();
        let path = fixture.source.join(DESCRIPTOR);
        let old = std::fs::read_to_string(&path).unwrap();
        assert!(old.contains("<VariantsStorage/>"));
        std::fs::write(path, old.replace("<VariantsStorage/>", field)).unwrap();
        let before = unica_workspace::test_support::tree_snapshot(&fixture.source);
        let refused = call(
            &runtime,
            &fixture,
            ToolIdentity::Apply,
            edit(json!("SettingsStorage.StorageA")),
        );
        assert!(!refused.ok, "{field}: {refused:?}");
        assert_eq!(
            unica_workspace::test_support::tree_snapshot(&fixture.source),
            before
        );
    }
}
#[test]
fn report_variant_storage_saved_plan_guards_report_root_and_selected_storage() {
    for relative in [
        DESCRIPTOR,
        "Configuration.xml",
        "SettingsStorages/StorageA.xml",
    ] {
        let fixture = Fixture::new("workspace-report-variants-storage");
        let runtime = runtime();
        let planned = call(
            &runtime,
            &fixture,
            ToolIdentity::Apply,
            edit(json!("SettingsStorage.StorageA")),
        );
        assert!(planned.ok, "{planned:?}");
        let token = planned.data.unwrap()["executionToken"].clone();
        let path = fixture.source.join(relative);
        let mut bytes = std::fs::read(&path).unwrap();
        bytes.extend_from_slice(b"\n");
        std::fs::write(path, bytes).unwrap();
        let before = unica_workspace::test_support::tree_snapshot(&fixture.source);
        let stale = call(
            &runtime,
            &fixture,
            ToolIdentity::Apply,
            json!({"executionToken":token}),
        );
        assert!(!stale.ok, "{relative}: {stale:?}");
        assert_eq!(stale.diagnostics[0]["code"], "stale_revision", "{stale:?}");
        assert_eq!(
            unica_workspace::test_support::tree_snapshot(&fixture.source),
            before
        );
    }
}

#[test]
fn report_variant_storage_noop_preserves_lexical_values_and_other_sources_stay_closed() {
    let fixture = Fixture::new("workspace-report-variants-storage");
    let runtime = runtime();
    let path = fixture.source.join(DESCRIPTOR);
    let old = std::fs::read_to_string(&path).unwrap();
    std::fs::write(&path,old.replace("<VariantsStorage/>","<m:VariantsStorage xmlns:m=\"http://v8.1c.ru/8.3/MDClasses\">SettingsStorage.Storage&#66;<!--keep--></m:VariantsStorage>")).unwrap();
    let before = unica_workspace::test_support::tree_snapshot(&fixture.source);
    publish(&runtime, &fixture, edit(json!("SettingsStorage.StorageB")));
    assert_eq!(
        unica_workspace::test_support::tree_snapshot(&fixture.source),
        before
    );
    let fixture = Fixture::with_extension();
    let runtime = super::runtime();
    publish(
        &runtime,
        &fixture,
        json!({"at":"ext:Configuration","ops":[{"op":"object.create","args":{"values":{"kind":"Report","name":"Boundary"}}}]}),
    );
    let can = contract(&runtime, &fixture, "ext:Report.Boundary", "props.set");
    assert!(can["argsSchema"]["properties"]["values"]["properties"]
        .get("VariantsStorage")
        .is_none());
    publish(
        &runtime,
        &fixture,
        json!({"at":"ext:Report.Boundary","ops":[{"op":"props.set","args":{"values":{"Comment":"independent"}}}]}),
    );
    let before = unica_workspace::test_support::tree_snapshot(&fixture.workspace.join("src-ext"));
    let refused = call(
        &runtime,
        &fixture,
        ToolIdentity::Apply,
        json!({"at":"ext:Report.Boundary","ops":[{"op":"props.set","args":{"values":{"VariantsStorage":"SettingsStorage.StorageA"}}}]}),
    );
    assert!(!refused.ok, "{refused:?}");
    assert_eq!(
        unica_workspace::test_support::tree_snapshot(&fixture.workspace.join("src-ext")),
        before
    );
}

#[test]
fn report_variant_storage_refuses_substituted_targets_and_keeps_unrelated_edits_independent() {
    for replacement in [
        "<Name>Other</Name>",
        "<Name>StorageA</Name><Name>Other</Name>",
        "<Name>StorageA<Nested/></Name>",
        "<Name> StorageA </Name>",
    ] {
        let fixture = Fixture::new("workspace-report-variants-storage");
        let runtime = runtime();
        let path = fixture.source.join("SettingsStorages/StorageA.xml");
        let old = std::fs::read_to_string(&path).unwrap();
        std::fs::write(path, old.replace("<Name>StorageA</Name>", replacement)).unwrap();
        let before = unica_workspace::test_support::tree_snapshot(&fixture.source);
        let refused = call(
            &runtime,
            &fixture,
            ToolIdentity::Apply,
            edit(json!("SettingsStorage.StorageA")),
        );
        assert!(!refused.ok, "{replacement}: {refused:?}");
        assert_eq!(
            unica_workspace::test_support::tree_snapshot(&fixture.source),
            before
        );
    }
    let fixture = Fixture::new("workspace-report-variants-storage");
    let runtime = runtime();
    let path = fixture.source.join(DESCRIPTOR);
    let old = std::fs::read_to_string(&path).unwrap();
    std::fs::write(
        path,
        old.replace(
            "<VariantsStorage/>",
            "<VariantsStorage>SettingsStorage.AlreadyMissing</VariantsStorage>",
        ),
    )
    .unwrap();
    publish(
        &runtime,
        &fixture,
        json!({"at":AT,"ops":[{"op":"props.set","args":{"values":{"Comment":"independent"}}}]}),
    );
    assert_eq!(
        view(&runtime, &fixture, AT)["props"]["VariantsStorage"],
        "SettingsStorage.AlreadyMissing"
    );
}

#[test]
fn report_variant_storage_absent_local_scalar_keeps_independent_facts_and_relations() {
    let fixture = Fixture::new("workspace-report-variants-storage");
    let runtime = runtime();
    let baseline = view(&runtime, &fixture, AT);
    let baseline_relations = view(&runtime, &fixture, &format!("{AT}.Relation"));
    let path = fixture.source.join(DESCRIPTOR);
    let old = std::fs::read_to_string(&path).unwrap();
    assert!(old.contains("<VariantsStorage/>"));
    std::fs::write(path, old.replace("<VariantsStorage/>", "")).unwrap();
    let before = unica_workspace::test_support::tree_snapshot(&fixture.source);
    let observed = call(&runtime, &fixture, ToolIdentity::View, json!({"at":AT}));
    assert!(observed.ok, "{observed:?}");
    assert!(observed.diagnostics.is_empty(), "{observed:?}");
    let observed = observed.data.unwrap();
    assert!(
        observed["props"].get("VariantsStorage").is_none(),
        "{observed}"
    );
    let mut expected_props = baseline["props"].clone();
    expected_props
        .as_object_mut()
        .unwrap()
        .remove("VariantsStorage");
    assert_eq!(observed["props"], expected_props);
    let relations = view(&runtime, &fixture, &format!("{AT}.Relation"));
    assert_eq!(relations, baseline_relations);
    assert_eq!(
        unica_workspace::test_support::tree_snapshot(&fixture.source),
        before
    );
}

#[test]
fn report_variant_storage_reader_keeps_present_damage_and_owner_damage_unknown() {
    use unica_address::source_target::{MetadataAddress, PLATFORM_XML_8_3_27_FORMAT_2_20};
    use unica_metadata::{MetaDiagnosticCode, MetaSupportStatus};
    let target = MetadataAddress::parse(PLATFORM_XML_8_3_27_FORMAT_2_20, "Report.Local").unwrap();
    for field in [
        "<VariantsStorage xmlns=\"urn:foreign\"/>",
        "<VariantsStorage/><VariantsStorage xmlns=\"urn:foreign\"/>",
        "<VariantsStorage/><VariantsStorage/>",
        "<VariantsStorage xsi:nil=\"true\"/>",
        "<VariantsStorage><Nested/></VariantsStorage>",
        "<VariantsStorage>Catalog.Wrong</VariantsStorage>",
    ] {
        let xml = format!(
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" version="2.20"><Report uuid="16970000-0000-4000-8000-000000000009"><Properties><Name>Local</Name>{field}</Properties><ChildObjects/></Report></MetaDataObject>"#
        );
        let observed = unica_metadata_designer::info::parse_typed_meta_local_info(
            xml.as_bytes(),
            &target,
            MetaSupportStatus::Supported,
        )
        .unwrap();
        assert_eq!(observed.relations.variants_storage, Some(None), "{field}");
        assert!(
            observed
                .diagnostics
                .iter()
                .any(|d| d.code == MetaDiagnosticCode::ValidationFailed
                    && d.field.as_deref() == Some("relations.variantsStorage")),
            "{field}: {:?}",
            observed.diagnostics
        );
    }
    for owner in [
        "<Report><Name>Local</Name></Report>",
        "<Report><Properties xmlns=\"urn:foreign\"><Name>Local</Name></Properties></Report>",
        "<Report><Properties><Name>Local</Name></Properties><Properties><Name>Local</Name></Properties></Report>",
        "<DataProcessor><Properties><Name>Local</Name></Properties></DataProcessor>",
    ] {
        let xml = format!(r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20">{owner}</MetaDataObject>"#);
        assert!(unica_metadata_designer::info::parse_typed_meta_local_info(xml.as_bytes(), &target, MetaSupportStatus::Supported).is_err(), "{owner}");
    }
}

#[test]
fn report_variant_storage_borrowed_local_absence_does_not_inherit_parent_reference() {
    let fixture = Fixture::new("workspace-report-variants-storage");
    let path = fixture.source.join(DESCRIPTOR);
    let old = std::fs::read_to_string(&path).unwrap();
    std::fs::write(
        path,
        old.replace(
            "<VariantsStorage/>",
            "<VariantsStorage>SettingsStorage.StorageA</VariantsStorage>",
        ),
    )
    .unwrap();
    let extension = fixture.workspace.join("src-ext");
    std::fs::create_dir_all(extension.join("Reports")).unwrap();
    std::fs::write(extension.join("Configuration.xml"), r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Configuration uuid="16970000-0000-4000-8000-000000000010"><Properties><ObjectBelonging>Adopted</ObjectBelonging><Name>Extension</Name><NamePrefix>Ext_</NamePrefix></Properties><ChildObjects><Report>SyntheticReport</Report></ChildObjects></Configuration></MetaDataObject>"#).unwrap();
    std::fs::write(extension.join(DESCRIPTOR), r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Report uuid="16970000-0000-4000-8000-000000000011"><Properties><ObjectBelonging>Adopted</ObjectBelonging><Name>SyntheticReport</Name><ExtendedConfigurationObject>f30c7939-01a6-57a4-95d9-3c5f812e32ab</ExtendedConfigurationObject><Comment>local</Comment></Properties><ChildObjects/></Report></MetaDataObject>"#).unwrap();
    std::fs::write(fixture.workspace.join("v8project.yaml"), "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n  - name: ext\n    type: EXTENSION\n    path: src-ext\n").unwrap();
    let before = unica_workspace::test_support::tree_snapshot(&fixture.source);
    let before_ext = unica_workspace::test_support::tree_snapshot(&extension);
    let runtime = runtime();
    let local = view(&runtime, &fixture, "ext:Report.SyntheticReport");
    assert_eq!(local["props"]["belonging"], "borrowed", "{local}");
    assert!(
        local
            .get("limits")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .all(|limit| !limit
                .as_str()
                .unwrap()
                .to_ascii_lowercase()
                .contains("variantsstorage")),
        "{local}"
    );
    assert_eq!(local["props"]["parentStatus"], "resolved", "{local}");
    assert_eq!(local["props"]["extends"], AT, "{local}");
    assert_eq!(local["props"]["Comment"], "local", "{local}");
    assert!(local["props"].get("VariantsStorage").is_none(), "{local}");
    let local_relations = view(&runtime, &fixture, "ext:Report.SyntheticReport.Relation");
    assert!(local_relations.get("limits").is_none(), "{local_relations}");
    assert!(
        !local_relations["items"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["relation"] == "variantsStorage"),
        "{local_relations}"
    );
    assert_eq!(
        view(&runtime, &fixture, AT)["props"]["VariantsStorage"],
        "SettingsStorage.StorageA"
    );
    let parent_relations = view(&runtime, &fixture, &format!("{AT}.Relation"));
    assert!(
        parent_relations["items"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["relation"] == "variantsStorage"
                && item["at"] == "main:SettingsStorage.StorageA"),
        "{parent_relations}"
    );
    assert_eq!(
        unica_workspace::test_support::tree_snapshot(&fixture.source),
        before
    );
    assert_eq!(
        unica_workspace::test_support::tree_snapshot(&extension),
        before_ext
    );
}
