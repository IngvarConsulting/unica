use super::*;

const AT: &str = "main:IntegrationService.Bus.Channel.ReceiveTrue";
const DESCRIPTOR: &str = "IntegrationServices/Bus.xml";
fn fixture() -> Fixture {
    Fixture::new("workspace-integration-channel")
}
fn edit(at: &str, values: Value) -> Value {
    json!({"at":at,"ops":[{"op":"props.set","args":{"values":values}}]})
}

#[test]
fn integration_channel_discovery_publication_preserves_neighbors_and_equivalent_bytes() {
    let fixture = fixture();
    let runtime = runtime();
    let owner = view(&runtime, &fixture, "main:IntegrationService.Bus");
    assert!(owner["branches"]
        .as_array()
        .unwrap()
        .contains(&json!({"at":"main:IntegrationService.Bus.Channel","count":8})));
    let collection = view(&runtime, &fixture, "main:IntegrationService.Bus.Channel");
    assert_eq!(collection["items"].as_array().unwrap().len(), 8);
    let can = contract(&runtime, &fixture, AT, "props.set");
    assert_eq!(
        can["argsSchema"]["properties"]["values"]["properties"]["MessageDirection"]["enum"],
        json!(["Receive", "Send"])
    );
    let before = unica_workspace::test_support::tree_snapshot(&fixture.source);
    publish(
        &runtime,
        &fixture,
        edit(
            AT,
            json!({"ExternalIntegrationServiceChannelName":"bus/<new>&"}),
        ),
    );
    assert_eq!(
        view(&runtime, &fixture, AT)["props"]["ExternalIntegrationServiceChannelName"],
        "bus/<new>&"
    );
    let mut after = unica_workspace::test_support::tree_snapshot(&fixture.source);
    let path = PathBuf::from(DESCRIPTOR);
    let text = String::from_utf8(after[&path].clone()).unwrap();
    after.insert(
        path,
        text.replace("bus/&lt;new&gt;&amp;", "test.ReceiveTrue")
            .into_bytes(),
    );
    assert_eq!(
        after, before,
        "UUID, InternalInfo, neighbors, registration, module and all other bytes survive"
    );
    for values in [
        json!({"ReceiveMessageProcessing":"Alternate"}),
        json!({"Transactioned":false}),
        json!({"MessageDirection":"Send"}),
        json!({"ReceiveMessageProcessing":""}),
        json!({"MessageDirection":"Receive"}),
    ] {
        publish(&runtime, &fixture, edit(AT, values.clone()));
        let before = unica_workspace::test_support::tree_snapshot(&fixture.source);
        let noop = call(&runtime, &fixture, ToolIdentity::Apply, edit(AT, values));
        assert!(noop.ok, "{noop:?}");
        assert_eq!(noop.data.unwrap()["effects"], 0);
        assert_eq!(
            unica_workspace::test_support::tree_snapshot(&fixture.source),
            before
        );
    }
    let send = "main:IntegrationService.Bus.Channel.SendTrueEmpty";
    publish(
        &runtime,
        &fixture,
        edit(
            send,
            json!({"ExternalIntegrationServiceChannelName":"renamed"}),
        ),
    );
    let send = view(&runtime, &fixture, send);
    assert_eq!(send["props"]["MessageDirection"], "Send");
    assert_eq!(send["props"]["Transactioned"], true);
    assert_eq!(send["props"]["ReceiveMessageProcessing"], "");
}

#[test]
fn integration_channel_invalid_values_targets_and_bindings_refuse_without_writes() {
    let fixture = fixture();
    let runtime = runtime();
    for values in [
        json!({"MessageDirection":"Both"}),
        json!({"Transactioned":"false"}),
        json!({"Transactioned":null}),
        json!({"Name":"Other"}),
        json!({"ReceiveMessageProcessing":"Missing"}),
        json!({"ReceiveMessageProcessing":"WrongArity"}),
        json!({"ReceiveMessageProcessing":"WrongFunction"}),
        json!({"ExternalIntegrationServiceChannelName":"bad\u{b}"}),
    ] {
        let before = unica_workspace::test_support::tree_snapshot(&fixture.source);
        let refused = call(&runtime, &fixture, ToolIdentity::Apply, edit(AT, values));
        assert!(!refused.ok, "{refused:?}");
        assert_eq!(
            unica_workspace::test_support::tree_snapshot(&fixture.source),
            before
        );
    }
    for at in [
        "main:IntegrationService.Bus",
        "main:IntegrationService.Bus.Channel.Missing",
        "main:IntegrationService.Bus.Channel",
        "main:Catalog.Bus.Channel.ReceiveTrue",
    ] {
        let before = unica_workspace::test_support::tree_snapshot(&fixture.source);
        let refused = call(
            &runtime,
            &fixture,
            ToolIdentity::Apply,
            edit(at, json!({"Transactioned":true})),
        );
        assert!(!refused.ok, "{at}: {refused:?}");
        assert_eq!(
            unica_workspace::test_support::tree_snapshot(&fixture.source),
            before
        );
    }
    let before = unica_workspace::test_support::tree_snapshot(&fixture.source);
    let refused = call(
        &runtime,
        &fixture,
        ToolIdentity::Apply,
        json!({"at":AT,"ops":[
            {"op":"props.set","args":{"values":{"Transactioned":false}}},
            {"op":"props.set","args":{"values":{"ReceiveMessageProcessing":"Missing"}}}
        ]}),
    );
    assert!(!refused.ok, "{refused:?}");
    assert_eq!(
        unica_workspace::test_support::tree_snapshot(&fixture.source),
        before
    );
}

#[test]
fn integration_channel_saved_token_guards_descriptor_module_and_registration() {
    for path in [
        DESCRIPTOR,
        "Configuration.xml",
        "IntegrationServices/Bus/Ext/Module.bsl",
    ] {
        let fixture = fixture();
        let runtime = runtime();
        let planned = call(
            &runtime,
            &fixture,
            ToolIdentity::Apply,
            edit(AT, json!({"ReceiveMessageProcessing":"Alternate"})),
        );
        assert!(planned.ok, "{planned:?}");
        let token = planned.data.unwrap()["executionToken"].clone();
        let path = fixture.source.join(path);
        let mut bytes = std::fs::read(&path).unwrap();
        bytes.extend_from_slice(b"\r\n");
        std::fs::write(path, bytes).unwrap();
        let before = unica_workspace::test_support::tree_snapshot(&fixture.source);
        let stale = call(
            &runtime,
            &fixture,
            ToolIdentity::Apply,
            json!({"executionToken":token}),
        );
        assert!(!stale.ok, "{stale:?}");
        assert_eq!(stale.diagnostics[0]["code"], "stale_revision", "{stale:?}");
        assert_eq!(
            unica_workspace::test_support::tree_snapshot(&fixture.source),
            before
        );
    }
}

#[test]
fn integration_channel_refuses_ambiguous_source_and_preserves_inactive_saved_handler() {
    for mutation in ["registration", "owner", "duplicate", "namespace", "format"] {
        let fixture = fixture();
        let runtime = runtime();
        let path = fixture.source.join(if mutation == "registration" {
            "Configuration.xml"
        } else {
            DESCRIPTOR
        });
        let text = std::fs::read_to_string(&path).unwrap();
        let changed = match mutation {
            "registration" => text.replace("<IntegrationService>Bus</IntegrationService>", ""),
            "owner" => text.replace("<Name>Bus</Name>", "<Name>Other</Name>"),
            "duplicate" => text.replace("<Name>ReceiveFalse</Name>", "<Name>ReceiveTrue</Name>"),
            "namespace" => text.replace("<Transactioned>", "<Transactioned xmlns=\"foreign\">"),
            _ => text.replace("version=\"2.20\"", "version=\"2.21\""),
        };
        assert_ne!(changed, text);
        std::fs::write(path, changed).unwrap();
        let before = unica_workspace::test_support::tree_snapshot(&fixture.source);
        let result = call(
            &runtime,
            &fixture,
            ToolIdentity::Apply,
            edit(AT, json!({"Transactioned":false})),
        );
        assert!(!result.ok, "{mutation}: {result:?}");
        assert_eq!(
            unica_workspace::test_support::tree_snapshot(&fixture.source),
            before
        );
    }
    let fixture = fixture();
    let runtime = runtime();
    let module = fixture
        .source
        .join("IntegrationServices/Bus/Ext/Module.bsl");
    std::fs::write(module, "// old handler unavailable\n").unwrap();
    let at = "main:IntegrationService.Bus.Channel.SendTrueHandler";
    publish(
        &runtime,
        &fixture,
        edit(at, json!({"ExternalIntegrationServiceChannelName":"saved"})),
    );
    let result = call(
        &runtime,
        &fixture,
        ToolIdentity::Apply,
        edit(at, json!({"MessageDirection":"Receive"})),
    );
    assert!(!result.ok, "{result:?}");
}
