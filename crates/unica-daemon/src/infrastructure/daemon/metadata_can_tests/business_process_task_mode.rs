use super::*;

#[test]
fn business_process_task_mode_round_trips_without_changing_other_source_bytes() {
    let fixture = Fixture::blank();
    let runtime = runtime();
    for (kind, name) in [("Task", "Work"), ("BusinessProcess", "Approval")] {
        publish(
            &runtime,
            &fixture,
            json!({"at":"main:Configuration","ops":[{"op":"object.create","args":{"values":{"kind":kind,"name":name}}}]}),
        );
    }
    let at = "main:BusinessProcess.Approval";
    let can = contract(&runtime, &fixture, at, "props.set");
    assert_eq!(
        can["argsSchema"]["properties"]["values"]["properties"]["CreateTaskInPrivilegedMode"]
            ["type"],
        "boolean"
    );
    assert_parity(
        &can["argsSchema"],
        &fixture.binding(false),
        at,
        "props.set",
        &[
            (json!({"values":{"CreateTaskInPrivilegedMode":false}}), true),
            (
                json!({"values":{"CreateTaskInPrivilegedMode":"false"}}),
                false,
            ),
        ],
    );
    let original = unica_workspace::test_support::tree_snapshot(&fixture.source);
    let owner = PathBuf::from("BusinessProcesses/Approval.xml");
    let initial = String::from_utf8(original[&owner].clone()).unwrap();
    assert!(initial.contains("<CreateTaskInPrivilegedMode>true</CreateTaskInPrivilegedMode>"));
    for value in [false, false, true] {
        publish(
            &runtime,
            &fixture,
            json!({"at":at,"ops":[{"op":"props.set","args":{"values":{"CreateTaskInPrivilegedMode":value}}}]}),
        );
        let mut expected = original.clone();
        expected.insert(
            owner.clone(),
            initial
                .replace(
                    "<CreateTaskInPrivilegedMode>true</CreateTaskInPrivilegedMode>",
                    &format!("<CreateTaskInPrivilegedMode>{value}</CreateTaskInPrivilegedMode>"),
                )
                .into_bytes(),
        );
        assert_eq!(
            unica_workspace::test_support::tree_snapshot(&fixture.source),
            expected
        );
        assert_eq!(
            view(&runtime, &fixture, at)["props"]["CreateTaskInPrivilegedMode"],
            value
        );
    }
    for (target, value) in [
        (at, json!("false")),
        ("main:Task.Work", json!(false)),
        ("main:BusinessProcess.Missing", json!(false)),
    ] {
        let before = unica_workspace::test_support::tree_snapshot(&fixture.source);
        let refused = call(
            &runtime,
            &fixture,
            ToolIdentity::Apply,
            json!({"at":target,"ops":[{"op":"props.set","args":{"values":{"CreateTaskInPrivilegedMode":value}}}]}),
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
        json!({"at":at,"ops":[{"op":"props.set","args":{"values":{"CreateTaskInPrivilegedMode":false}}}]}),
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
        initial.replace(
            "<CreateTaskInPrivilegedMode>true</CreateTaskInPrivilegedMode>",
            "",
        ),
    )
    .unwrap();
    let before = unica_workspace::test_support::tree_snapshot(&fixture.source);
    let refused = call(
        &runtime,
        &fixture,
        ToolIdentity::Apply,
        json!({"at":at,"ops":[{"op":"props.set","args":{"values":{"CreateTaskInPrivilegedMode":false}}}]}),
    );
    assert!(!refused.ok, "{refused:?}");
    assert_eq!(
        unica_workspace::test_support::tree_snapshot(&fixture.source),
        before
    );
}
