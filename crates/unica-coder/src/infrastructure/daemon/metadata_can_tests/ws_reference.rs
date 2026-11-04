use super::*;

const AT: &str = "main:WSReference.Probe";
fn edit(value: Value) -> Value {
    json!({"at":AT,"ops":[{"op":"props.set","args":{"values":{"LocationURL":value}}}]})
}

#[test]
fn ws_reference_url_can_preview_publication_preserve_every_neighbor_and_noop() {
    let fixture = Fixture::new("workspace-wsreference-location-url");
    let runtime = runtime();
    let before = unica_workspace::test_support::tree_snapshot(&fixture.source);
    let can = contract(&runtime, &fixture, AT, "props.set");
    let keys: BTreeSet<_> = can["argsSchema"]["properties"]["values"]["properties"]
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(keys, BTreeSet::from(["LocationURL", "locationURL"]));
    let initial = view(&runtime, &fixture, AT);
    assert!(initial["can"].as_array().unwrap().iter().all(|v| !matches!(
        v["op"].as_str(),
        Some("object.create" | "object.remove" | "relation.add" | "attribute.add")
    )));
    assert_eq!(
        view(&runtime, &fixture, &format!("{AT}.Props.LocationURL"))["props"]["LocationURL"],
        ""
    );
    let value = "https://example.invalid/?a=1&b=<ю>&quote=\"x\"";
    publish(&runtime, &fixture, edit(json!(value)));
    assert_eq!(view(&runtime, &fixture, AT)["props"]["LocationURL"], value);
    let mut after = unica_workspace::test_support::tree_snapshot(&fixture.source);
    let path = PathBuf::from("WSReferences/Probe.xml");
    let descriptor = String::from_utf8(after[&path].clone()).unwrap();
    let edited = format!(
        "<LocationURL>{}</LocationURL>",
        unica_xml_format::text::escape_xml_text(value)
    );
    assert!(descriptor.contains(&edited));
    after.insert(
        path,
        descriptor.replace(&edited, "<LocationURL/>").into_bytes(),
    );
    assert_eq!(
        after, before,
        "all bytes outside the edited property, including WSDL/UUID/registration, survive"
    );
    let before = unica_workspace::test_support::tree_snapshot(&fixture.source);
    let noop = call(&runtime, &fixture, ToolIdentity::Apply, edit(json!(value)));
    assert!(noop.ok, "{noop:?}");
    assert_eq!(noop.data.unwrap()["effects"], 0);
    assert_eq!(
        unica_workspace::test_support::tree_snapshot(&fixture.source),
        before
    );
    publish(&runtime, &fixture, edit(json!("")));
    assert_eq!(
        view(&runtime, &fixture, &format!("{AT}.Props.LocationURL"))["props"]["LocationURL"],
        ""
    );
}

#[test]
fn ws_reference_argument_errors_and_damaged_or_absent_fields_leave_no_writes() {
    for values in [
        json!({"LocationURL":false}),
        json!({"LocationURL":null}),
        json!({"LocationURL":"one","locationURL":"two"}),
        json!({"Name":"Other"}),
        json!({"locationURL":"bad\u{000b}"}),
    ] {
        let fixture = Fixture::new("workspace-wsreference-location-url");
        let runtime = runtime();
        let before = unica_workspace::test_support::tree_snapshot(&fixture.source);
        let refused = call(
            &runtime,
            &fixture,
            ToolIdentity::Apply,
            json!({"at":AT,"ops":[{"op":"props.set","args":{"values":values}}]}),
        );
        assert!(!refused.ok, "{refused:?}");
        assert_eq!(refused.diagnostics[0]["code"], "bad_value", "{refused:?}");
        assert_eq!(
            unica_workspace::test_support::tree_snapshot(&fixture.source),
            before
        );
    }
    for field in [
        "",
        "<LocationURL/><LocationURL/>",
        "<LocationURL xmlns=\"urn:foreign\"/>",
        "<LocationURL xsi:nil=\"true\"/>",
        "<LocationURL><Nested/></LocationURL>",
    ] {
        let fixture = Fixture::new("workspace-wsreference-location-url");
        let runtime = runtime();
        let path = fixture.source.join("WSReferences/Probe.xml");
        let old = std::fs::read_to_string(&path).unwrap();
        std::fs::write(&path, old.replace("<LocationURL/>", field)).unwrap();
        let before = unica_workspace::test_support::tree_snapshot(&fixture.source);
        let readable = view(&runtime, &fixture, AT);
        assert_eq!(readable["kind"], "WSReference");
        if !field.is_empty() {
            assert!(readable.get("limits").is_some(), "{field}: {readable}");
        }
        let refused = call(&runtime, &fixture, ToolIdentity::Apply, edit(json!("new")));
        assert!(!refused.ok, "{field}: {refused:?}");
        assert_eq!(
            unica_workspace::test_support::tree_snapshot(&fixture.source),
            before
        );
    }
}

#[test]
fn ws_reference_saved_preview_guards_descriptor_and_configuration_registration() {
    for registration in [false, true] {
        let fixture = Fixture::new("workspace-wsreference-location-url");
        let runtime = runtime();
        let planned = call(&runtime, &fixture, ToolIdentity::Apply, edit(json!("new")));
        assert!(planned.ok, "{planned:?}");
        let token = planned.data.unwrap()["executionToken"].clone();
        let path = fixture.source.join(if registration {
            "Configuration.xml"
        } else {
            "WSReferences/Probe.xml"
        });
        let old = std::fs::read_to_string(&path).unwrap();
        let new = if registration {
            old.replace("<WSReference>Probe</WSReference>", "")
        } else {
            old.replace("Preserve &amp; verify", "Concurrent comment")
        };
        assert_ne!(old, new);
        std::fs::write(path, new).unwrap();
        let before = unica_workspace::test_support::tree_snapshot(&fixture.source);
        let stale = call(
            &runtime,
            &fixture,
            ToolIdentity::Apply,
            json!({"executionToken":token}),
        );
        assert!(!stale.ok, "{registration}: {stale:?}");
        assert_eq!(stale.diagnostics[0]["code"], "stale_revision", "{stale:?}");
        assert_eq!(
            unica_workspace::test_support::tree_snapshot(&fixture.source),
            before,
            "a stale plan never leaves a partial edit"
        );
    }
}

#[test]
fn ws_reference_location_url_refuses_foreign_owners_and_unregistered_or_substituted_descriptors() {
    for at in [
        "main:Configuration",
        "main:Language.Русский",
        "main:WSReference.Probe.Attribute.Foreign",
    ] {
        let fixture = Fixture::new("workspace-wsreference-location-url");
        let runtime = runtime();
        let before = unica_workspace::test_support::tree_snapshot(&fixture.source);
        let mut request = edit(json!("new"));
        request["at"] = json!(at);
        let refused = call(&runtime, &fixture, ToolIdentity::Apply, request);
        assert!(!refused.ok, "{at}: {refused:?}");
        assert_eq!(
            unica_workspace::test_support::tree_snapshot(&fixture.source),
            before
        );
    }
    for registration in [false, true] {
        let fixture = Fixture::new("workspace-wsreference-location-url");
        let runtime = runtime();
        let path = fixture.source.join(if registration {
            "Configuration.xml"
        } else {
            "WSReferences/Probe.xml"
        });
        let old = std::fs::read_to_string(&path).unwrap();
        let new = if registration {
            old.replace("<WSReference>Probe</WSReference>", "")
        } else {
            old.replace("<Name>Probe</Name>", "<Name>Other</Name>")
        };
        assert_ne!(old, new);
        std::fs::write(path, new).unwrap();
        let before = unica_workspace::test_support::tree_snapshot(&fixture.source);
        let refused = call(&runtime, &fixture, ToolIdentity::Apply, edit(json!("new")));
        assert!(!refused.ok, "{refused:?}");
        assert_eq!(
            unica_workspace::test_support::tree_snapshot(&fixture.source),
            before
        );
    }
}

#[test]
fn ws_reference_writer_refuses_ambiguous_or_normalized_name_without_hiding_url_read() {
    for name in [
        "<Name>Probe</Name><Name>Other</Name>",
        "<Name>Probe<Nested/></Name>",
        "<Name> Probe </Name>",
        "<Name extra=\"value\">Probe</Name>",
        "<Name>Probe</Name><Name xmlns=\"urn:foreign\">Other</Name>",
    ] {
        let fixture = Fixture::new("workspace-wsreference-location-url");
        let runtime = runtime();
        let path = fixture.source.join("WSReferences/Probe.xml");
        let old = std::fs::read_to_string(&path).unwrap();
        std::fs::write(path, old.replace("<Name>Probe</Name>", name)).unwrap();
        let before = unica_workspace::test_support::tree_snapshot(&fixture.source);
        assert_eq!(
            view(&runtime, &fixture, &format!("{AT}.Props.LocationURL"))["props"]["LocationURL"],
            ""
        );
        let refused = call(&runtime, &fixture, ToolIdentity::Apply, edit(json!("new")));
        assert!(
            !refused.ok,
            "ambiguous Name must not authorize a write: {name}: {refused:?}"
        );
        assert!(refused
            .data
            .as_ref()
            .is_none_or(|d| d.get("executionToken").is_none()));
        assert_eq!(
            unica_workspace::test_support::tree_snapshot(&fixture.source),
            before
        );
    }
}
