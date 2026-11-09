use super::*;

fn check_number_property(
    runtime: &V5CanonicalInvocationRuntime,
    fixture: &Fixture,
    kind: &str,
    directory: &str,
    name: &str,
    property: &str,
    values: &[&str],
) {
    let at = format!("main:{kind}.{name}");
    let schema = contract(runtime, fixture, &at, "props.set");
    let enumeration = &schema["argsSchema"]["properties"]["values"]["properties"][property]["enum"];
    for value in values {
        assert!(enumeration.as_array().unwrap().contains(&json!(value)));
    }
    let owner = PathBuf::from(format!("{directory}/{name}.xml"));
    let path = fixture.source.join(&owner);
    let original = unica_workspace::test_support::tree_snapshot(&fixture.source);
    let initial = String::from_utf8(original[&owner].clone()).unwrap();
    let observed = view(runtime, fixture, &at)["props"][property]
        .as_str()
        .unwrap()
        .to_string();
    let old = format!("<{property}>{observed}</{property}>");
    assert!(initial.contains(&old));
    for value in values.iter().chain(values.last()) {
        publish(
            runtime,
            fixture,
            json!({"at":at,"ops":[{"op":"props.set","args":{"values":{property:value}}}]}),
        );
        let mut expected = original.clone();
        expected.insert(
            owner.clone(),
            initial
                .replace(&old, &format!("<{property}>{value}</{property}>"))
                .into_bytes(),
        );
        assert_eq!(
            unica_workspace::test_support::tree_snapshot(&fixture.source),
            expected
        );
        assert_eq!(view(runtime, fixture, &at)["props"][property], *value);
    }
    std::fs::write(&path, &initial).unwrap();
    for value in [json!("Unknown"), json!(false)] {
        let result = call(
            runtime,
            fixture,
            ToolIdentity::Apply,
            json!({"at":at,"ops":[{"op":"props.set","args":{"values":{property:value}}}]}),
        );
        assert!(!result.ok, "{result:?}");
        assert_eq!(
            unica_workspace::test_support::tree_snapshot(&fixture.source),
            original
        );
    }
    if property == "NumberAllowedLength" && kind != "Document" {
        let numeric = initial.replace(
            "<NumberType>String</NumberType>",
            "<NumberType>Number</NumberType>",
        );
        assert_ne!(numeric, initial);
        std::fs::write(&path, &numeric).unwrap();
        let mut expected = unica_workspace::test_support::tree_snapshot(&fixture.source);
        expected.insert(
            owner.clone(),
            numeric
                .replace(&old, &format!("<{property}>{}</{property}>", values[0]))
                .into_bytes(),
        );
        publish(
            runtime,
            fixture,
            json!({"at":at,"ops":[{"op":"props.set","args":{"values":{property:values[0]}}}]}),
        );
        assert_eq!(
            unica_workspace::test_support::tree_snapshot(&fixture.source),
            expected
        );
        assert_eq!(view(runtime, fixture, &at)["props"]["NumberType"], "Number");
        std::fs::write(&path, &initial).unwrap();
    }
    if kind == "Document" {
        return;
    }
    for damaged in [
        String::new(),
        format!("<{property}>Unknown</{property}>"),
        format!("<{property}><value>{observed}</value></{property}>"),
        format!("{old}{old}"),
    ] {
        std::fs::write(&path, initial.replace(&old, &damaged)).unwrap();
        let before = unica_workspace::test_support::tree_snapshot(&fixture.source);
        let result = call(
            runtime,
            fixture,
            ToolIdentity::Apply,
            json!({"at":at,"ops":[{"op":"props.set","args":{"values":{property:values[0]}}}]}),
        );
        assert!(!result.ok, "{kind}.{property} {damaged}: {result:?}");
        assert_eq!(
            unica_workspace::test_support::tree_snapshot(&fixture.source),
            before
        );
    }
    std::fs::write(&path, &initial).unwrap();
    let plan = call(
        runtime,
        fixture,
        ToolIdentity::Apply,
        json!({"at":at,"ops":[{"op":"props.set","args":{"values":{property:values[0]}}}]}),
    );
    assert!(plan.ok, "{plan:?}");
    std::fs::write(
        &path,
        initial.replace("<Comment/>", "<Comment>Changed</Comment>"),
    )
    .unwrap();
    let before = unica_workspace::test_support::tree_snapshot(&fixture.source);
    let result = call(
        runtime,
        fixture,
        ToolIdentity::Apply,
        json!({"executionToken":plan.data.unwrap()["executionToken"]}),
    );
    assert!(!result.ok, "{result:?}");
    assert_eq!(
        unica_workspace::test_support::tree_snapshot(&fixture.source),
        before
    );
    std::fs::write(&path, initial).unwrap();
}

#[test]
fn business_process_and_task_number_settings_preserve_source_and_refuse_damage() {
    let fixture = Fixture::blank();
    let runtime = runtime();
    for (kind, name) in [
        ("Document", "Document"),
        ("Task", "Work"),
        ("BusinessProcess", "Approval"),
    ] {
        publish(
            &runtime,
            &fixture,
            json!({"at":"main:Configuration","ops":[{"op":"object.create","args":{"values":{"kind":kind,"name":name}}}]}),
        );
    }
    for (kind, dir, name) in [
        ("Document", "Documents", "Document"),
        ("BusinessProcess", "BusinessProcesses", "Approval"),
        ("Task", "Tasks", "Work"),
    ] {
        check_number_property(
            &runtime,
            &fixture,
            kind,
            dir,
            name,
            "NumberAllowedLength",
            &["Fixed", "Variable", "Fixed"],
        );
        if kind != "Task" {
            check_number_property(
                &runtime,
                &fixture,
                kind,
                dir,
                name,
                "NumberPeriodicity",
                &["Day", "Month", "Quarter", "Year", "Nonperiodical"],
            );
        }
    }
    let can = contract(&runtime, &fixture, "main:Task.Work", "props.set");
    assert!(can["argsSchema"]["properties"]["values"]["properties"]
        .get("NumberPeriodicity")
        .is_none());
    let before = unica_workspace::test_support::tree_snapshot(&fixture.source);
    let result = call(
        &runtime,
        &fixture,
        ToolIdentity::Apply,
        json!({"at":"main:Task.Work","ops":[{"op":"props.set","args":{"values":{"NumberPeriodicity":"Day"}}}]}),
    );
    assert!(!result.ok, "{result:?}");
    assert_eq!(
        unica_workspace::test_support::tree_snapshot(&fixture.source),
        before
    );
}
