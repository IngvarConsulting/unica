use super::*;

#[test]
fn document_sequence_filling_preserves_posting_movements_forms_and_modules() {
    let fixture = Fixture::new("workspace-register-dimensions");
    let runtime = runtime();
    let at = "main:Document.Приход";
    publish(
        &runtime,
        &fixture,
        json!({"at":at,"ops":[{"op":"form.add","args":{"items":[{"name":"Main","type":"ObjectForm"}]}}]}),
    );
    let can = contract(&runtime, &fixture, at, "props.set");
    assert_eq!(
        can["argsSchema"]["properties"]["values"]["properties"]["SequenceFilling"]["type"],
        "string"
    );
    assert_eq!(
        can["argsSchema"]["properties"]["values"]["properties"]["SequenceFilling"]["enum"],
        json!(["AutoFill", "AutoFillOff"])
    );
    assert_parity(
        &can["argsSchema"],
        &fixture.binding(false),
        at,
        "props.set",
        &[
            (json!({"values":{"SequenceFilling":"AutoFillOff"}}), true),
            (json!({"values":{"SequenceFilling":false}}), false),
        ],
    );
    let original = unica_workspace::test_support::tree_snapshot(&fixture.source);
    let owner = PathBuf::from("Documents/Приход.xml");
    let initial = String::from_utf8(original[&owner].clone()).unwrap();
    assert!(initial.contains("<SequenceFilling>AutoFill</SequenceFilling>"));
    for value in ["AutoFillOff", "AutoFillOff", "AutoFill"] {
        publish(
            &runtime,
            &fixture,
            json!({"at":at,"ops":[{"op":"props.set","args":{"values":{"SequenceFilling":value}}}]}),
        );
        let mut expected = original.clone();
        expected.insert(
            owner.clone(),
            initial
                .replace(
                    "<SequenceFilling>AutoFill</SequenceFilling>",
                    &format!("<SequenceFilling>{value}</SequenceFilling>"),
                )
                .into_bytes(),
        );
        assert_eq!(
            unica_workspace::test_support::tree_snapshot(&fixture.source),
            expected
        );
        assert_eq!(
            view(&runtime, &fixture, at)["props"]["SequenceFilling"],
            value
        );
    }
    for (target, value) in [
        (at, json!("Unknown")),
        ("main:InformationRegister.Цены", json!("AutoFillOff")),
        ("main:Document.Missing", json!("AutoFillOff")),
    ] {
        let before = unica_workspace::test_support::tree_snapshot(&fixture.source);
        let refused = call(
            &runtime,
            &fixture,
            ToolIdentity::Apply,
            json!({"at":target,"ops":[{"op":"props.set","args":{"values":{"SequenceFilling":value}}}]}),
        );
        assert!(!refused.ok, "{refused:?}");
        assert_eq!(
            unica_workspace::test_support::tree_snapshot(&fixture.source),
            before
        );
    }
    let planned = call(
        &runtime,
        &fixture,
        ToolIdentity::Apply,
        json!({"at":at,"ops":[{"op":"props.set","args":{"values":{"SequenceFilling":"AutoFillOff"}}}]}),
    );
    assert!(planned.ok, "{planned:?}");
    let path = fixture.source.join(&owner);
    std::fs::write(
        &path,
        initial.replace("<Comment/>", "<Comment>External edit</Comment>"),
    )
    .unwrap();
    let before = unica_workspace::test_support::tree_snapshot(&fixture.source);
    let refused = call(
        &runtime,
        &fixture,
        ToolIdentity::Apply,
        json!({"executionToken":planned.data.unwrap()["executionToken"]}),
    );
    assert!(!refused.ok, "{refused:?}");
    assert_eq!(
        unica_workspace::test_support::tree_snapshot(&fixture.source),
        before
    );
    std::fs::write(
        &path,
        initial.replace("<SequenceFilling>AutoFill</SequenceFilling>", ""),
    )
    .unwrap();
    let before = unica_workspace::test_support::tree_snapshot(&fixture.source);
    let refused = call(
        &runtime,
        &fixture,
        ToolIdentity::Apply,
        json!({"at":at,"ops":[{"op":"props.set","args":{"values":{"SequenceFilling":"AutoFillOff"}}}]}),
    );
    assert!(!refused.ok, "{refused:?}");
    assert_eq!(
        unica_workspace::test_support::tree_snapshot(&fixture.source),
        before
    );
}
