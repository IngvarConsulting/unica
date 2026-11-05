use super::*;

fn verify_modes(prefix: &str, periods: &[&str]) {
    let fixture = Fixture::new("workspace-register-slice-totals");
    let runtime = runtime();
    for period in periods {
        let name = if prefix == "Ind" && *period == "RecorderPosition" {
            "IndNonperiodical".to_owned()
        } else {
            format!("{prefix}{period}")
        };
        let at = format!("main:InformationRegister.{name}");
        let owner = PathBuf::from(format!("InformationRegisters/{name}.xml"));
        let path = fixture.source.join(&owner);
        // Designer 8.3.27.2214 also preserves this inactive combination.
        if prefix == "Ind" && *period == "RecorderPosition" {
            let text = std::fs::read_to_string(&path).unwrap();
            std::fs::write(&path, text.replace("<InformationRegisterPeriodicity>Nonperiodical</InformationRegisterPeriodicity>", "<InformationRegisterPeriodicity>RecorderPosition</InformationRegisterPeriodicity>")).unwrap();
        }
        let original = unica_workspace::test_support::tree_snapshot(&fixture.source);
        let initial = String::from_utf8(original[&owner].clone()).unwrap();
        let can = contract(&runtime, &fixture, &at, "props.set");
        for key in ["EnableTotalsSliceFirst", "EnableTotalsSliceLast"] {
            assert_eq!(
                can["argsSchema"]["properties"]["values"]["properties"][key]["type"],
                "boolean"
            );
        }
        let mut previous = (false, false);
        for (first, last) in [
            (true, false),
            (false, false),
            (false, true),
            (true, true),
            (true, true),
            (false, false),
        ] {
            let values = if previous.1 == last {
                json!({"EnableTotalsSliceFirst": first})
            } else if previous.0 == first {
                json!({"EnableTotalsSliceLast": last})
            } else {
                json!({"EnableTotalsSliceFirst": first, "EnableTotalsSliceLast": last})
            };
            previous = (first, last);
            publish(
                &runtime,
                &fixture,
                json!({"at":at,"ops":[{"op":"props.set","args":{"values":values}}]}),
            );
            let mut expected = original.clone();
            expected.insert(
                owner.clone(),
                initial
                    .replace(
                        "<EnableTotalsSliceFirst>false</EnableTotalsSliceFirst>",
                        &format!("<EnableTotalsSliceFirst>{first}</EnableTotalsSliceFirst>"),
                    )
                    .replace(
                        "<EnableTotalsSliceLast>false</EnableTotalsSliceLast>",
                        &format!("<EnableTotalsSliceLast>{last}</EnableTotalsSliceLast>"),
                    )
                    .into_bytes(),
            );
            assert_eq!(
                unica_workspace::test_support::tree_snapshot(&fixture.source),
                expected
            );
            let props = view(&runtime, &fixture, &at);
            assert_eq!(props["props"]["EnableTotalsSliceFirst"], first);
            assert_eq!(props["props"]["EnableTotalsSliceLast"], last);
        }
        for key in ["EnableTotalsSliceFirst", "EnableTotalsSliceLast"] {
            for value in [json!("true"), json!(1), Value::Null] {
                let values = json!({key:value});
                let refused = call(
                    &runtime,
                    &fixture,
                    ToolIdentity::Apply,
                    json!({"at":at,"ops":[{"op":"props.set","args":{"values":values}}]}),
                );
                assert!(!refused.ok, "{refused:?}");
                assert_eq!(
                    unica_workspace::test_support::tree_snapshot(&fixture.source),
                    original
                );
            }
        }
        // At least one missing target property is checked in each native mode.
        std::fs::write(
            &path,
            initial.replace("<EnableTotalsSliceFirst>false</EnableTotalsSliceFirst>", ""),
        )
        .unwrap();
        let before = unica_workspace::test_support::tree_snapshot(&fixture.source);
        let refused = call(
            &runtime,
            &fixture,
            ToolIdentity::Apply,
            json!({"at":at,"ops":[{"op":"props.set","args":{"values":{"EnableTotalsSliceFirst":true}}}]}),
        );
        assert!(!refused.ok, "{refused:?}");
        assert_eq!(
            unica_workspace::test_support::tree_snapshot(&fixture.source),
            before
        );
        std::fs::write(&path, initial).unwrap();
    }
}

#[test]
fn medium_slice_totals_preserve_independent_registers_including_inactive_nonperiodical() {
    verify_modes(
        "Ind",
        &[
            "Nonperiodical",
            "Second",
            "Day",
            "Month",
            "Quarter",
            "Year",
            "RecorderPosition",
        ],
    );
}

#[test]
fn slice_totals_preserve_recorder_registers_including_recorder_position() {
    verify_modes(
        "Sub",
        &[
            "Nonperiodical",
            "Second",
            "Day",
            "Month",
            "Quarter",
            "Year",
            "RecorderPosition",
        ],
    );
}

#[test]
fn slice_totals_refuse_damaged_flags_and_changed_retained_settings() {
    let fixture = Fixture::new("workspace-register-slice-totals");
    let runtime = runtime();
    let at = "main:InformationRegister.IndDay";
    let path = fixture.source.join("InformationRegisters/IndDay.xml");
    let initial = std::fs::read_to_string(&path).unwrap();
    for key in ["EnableTotalsSliceFirst", "EnableTotalsSliceLast"] {
        let old = format!("<{key}>false</{key}>");
        for damaged in [
            format!("<{key}>Auto</{key}>"),
            format!("<{key}/>",),
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
                json!({"at":at,"ops":[{"op":"props.set","args":{"values":{key:true}}}]}),
            );
            assert!(!refused.ok, "{damaged}: {refused:?}");
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
        json!({"at":at,"ops":[{"op":"props.set","args":{"values":{"EnableTotalsSliceFirst":true}}}]}),
    );
    assert!(plan.ok, "{plan:?}");
    std::fs::write(
        &path,
        initial.replace(
            "<InformationRegisterPeriodicity>Day</InformationRegisterPeriodicity>",
            "<InformationRegisterPeriodicity>Month</InformationRegisterPeriodicity>",
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
