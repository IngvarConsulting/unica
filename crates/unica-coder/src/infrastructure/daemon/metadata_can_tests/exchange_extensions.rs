use super::*;

#[test]
fn exchange_extensions_validate_final_pair_and_preserve_content() {
    let fixture = Fixture::new("workspace-exchange-extensions");
    let runtime = runtime();
    let at = "main:ExchangePlan.Codes";
    let owner = PathBuf::from("ExchangePlans/Codes.xml");
    let path = fixture.source.join(&owner);
    let original = unica_workspace::test_support::tree_snapshot(&fixture.source);
    let initial = String::from_utf8(original[&owner].clone()).unwrap();
    let can = contract(&runtime, &fixture, at, "props.set");
    assert_eq!(
        can["argsSchema"]["properties"]["values"]["properties"]["IncludeConfigurationExtensions"]
            ["type"],
        "boolean"
    );
    assert_parity(
        &can["argsSchema"],
        &fixture.binding(false),
        at,
        "props.set",
        &[
            (
                json!({"values":{"IncludeConfigurationExtensions":true}}),
                true,
            ),
            (
                json!({"values":{"IncludeConfigurationExtensions":"true"}}),
                false,
            ),
            (
                json!({"values":{"IncludeConfigurationExtensions":1}}),
                false,
            ),
            (
                json!({"values":{"IncludeConfigurationExtensions":null}}),
                false,
            ),
        ],
    );
    for (distributed, include) in [
        (true, true),
        (true, true),
        (true, false),
        (false, false),
        (true, true),
        (false, false),
        (true, false),
    ] {
        publish(
            &runtime,
            &fixture,
            json!({"at":at,"ops":[{"op":"props.set","args":{"values":{"IncludeConfigurationExtensions":include,"DistributedInfoBase":distributed}}}]}),
        );
        let mut expected = original.clone();
        expected.insert(owner.clone(),initial.replace("<DistributedInfoBase>true</DistributedInfoBase>",&format!("<DistributedInfoBase>{distributed}</DistributedInfoBase>")).replace("<IncludeConfigurationExtensions>false</IncludeConfigurationExtensions>",&format!("<IncludeConfigurationExtensions>{include}</IncludeConfigurationExtensions>")).into_bytes());
        assert_eq!(
            unica_workspace::test_support::tree_snapshot(&fixture.source),
            expected
        );
        let props = view(&runtime, &fixture, at);
        assert_eq!(props["props"]["DistributedInfoBase"], distributed);
        assert_eq!(props["props"]["IncludeConfigurationExtensions"], include);
    }
    let refused = call(
        &runtime,
        &fixture,
        ToolIdentity::Apply,
        json!({"at":at,"ops":[{"op":"props.set","args":{"values":{"IncludeConfigurationExtensions":true,"DistributedInfoBase":false}}}]}),
    );
    assert!(!refused.ok, "{refused:?}");
    assert_eq!(
        unica_workspace::test_support::tree_snapshot(&fixture.source),
        original
    );
    // Turning off the already supported flag must also preserve the invariant.
    publish(
        &runtime,
        &fixture,
        json!({"at":at,"ops":[{"op":"props.set","args":{"values":{"IncludeConfigurationExtensions":true}}}]}),
    );
    let before = unica_workspace::test_support::tree_snapshot(&fixture.source);
    let refused = call(
        &runtime,
        &fixture,
        ToolIdentity::Apply,
        json!({"at":at,"ops":[{"op":"props.set","args":{"values":{"DistributedInfoBase":false}}}]}),
    );
    assert!(!refused.ok, "{refused:?}");
    assert_eq!(
        unica_workspace::test_support::tree_snapshot(&fixture.source),
        before
    );
    std::fs::write(&path, &initial).unwrap();
    for key in ["DistributedInfoBase", "IncludeConfigurationExtensions"] {
        let old = if key == "DistributedInfoBase" {
            format!("<{key}>true</{key}>")
        } else {
            format!("<{key}>false</{key}>")
        };
        for damaged in [
            String::new(),
            format!("<{key}/>"),
            format!("<{key}>unknown</{key}>"),
            format!("<{key} xmlns=\"foreign\">false</{key}>"),
            format!("<{key}><value>false</value></{key}>"),
            format!("{old}{old}"),
        ] {
            std::fs::write(&path, initial.replace(&old, &damaged)).unwrap();
            let before = unica_workspace::test_support::tree_snapshot(&fixture.source);
            let refused = call(
                &runtime,
                &fixture,
                ToolIdentity::Apply,
                json!({"at":at,"ops":[{"op":"props.set","args":{"values":{"IncludeConfigurationExtensions":true}}}]}),
            );
            assert!(!refused.ok, "{key} {damaged}: {refused:?}");
            assert_eq!(
                unica_workspace::test_support::tree_snapshot(&fixture.source),
                before
            );
        }
    }
    std::fs::write(&path, &initial).unwrap();
    let plan = call(
        &runtime,
        &fixture,
        ToolIdentity::Apply,
        json!({"at":at,"ops":[{"op":"props.set","args":{"values":{"IncludeConfigurationExtensions":true}}}]}),
    );
    assert!(plan.ok, "{plan:?}");
    std::fs::write(
        &path,
        initial.replace(
            "<DistributedInfoBase>true</DistributedInfoBase>",
            "<DistributedInfoBase>false</DistributedInfoBase>",
        ),
    )
    .unwrap();
    let before = unica_workspace::test_support::tree_snapshot(&fixture.source);
    let refused = call(
        &runtime,
        &fixture,
        ToolIdentity::Apply,
        json!({"executionToken":plan.data.unwrap()["executionToken"]}),
    );
    assert!(!refused.ok, "{refused:?}");
    assert_eq!(
        unica_workspace::test_support::tree_snapshot(&fixture.source),
        before
    );
}
