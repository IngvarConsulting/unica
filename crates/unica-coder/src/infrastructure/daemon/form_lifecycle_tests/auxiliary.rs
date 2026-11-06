use super::*;
use serde_json::json;

fn auxiliary_fixture() -> Fixture {
    let fixture = Fixture::new();
    fixture.replace(
        "Reports/Totals.xml",
        "<DefaultSettingsForm/>",
        "<AuxiliaryForm/><DefaultSettingsForm/>",
    );
    fixture
}

#[test]
fn owned_form_assignment_requires_an_existing_matching_descriptor() {
    for replacement in [
        None,
        Some("<Template><Properties><Name>MainA</Name></Properties></Template>"),
        Some("<Form><Properties><Name>Other</Name></Properties></Form>"),
    ] {
        let f = auxiliary_fixture();
        let runtime = runtime();
        // The registered owned form is Own, independent of the CommonForms.
        let path = f.source.join("Reports/Totals/Forms/Own.xml");
        if let Some(body) = replacement {
            std::fs::write(&path, format!("<MetaDataObject xmlns=\"http://v8.1c.ru/8.3/MDClasses\" version=\"2.20\">{body}</MetaDataObject>")).unwrap();
        } else {
            std::fs::remove_file(&path).unwrap();
        }
        let before = unica_workspace::test_support::tree_snapshot(&f.source);
        let plan = f.preview(&runtime, "main:Report.Totals", "auxiliaryForm", "Own");
        assert!(
            !plan.ok,
            "missing or mismatched owned form accepted: {plan:?}"
        );
        assert_eq!(
            unica_workspace::test_support::tree_snapshot(&f.source),
            before
        );
    }
}

#[test]
fn owned_form_assignment_binds_the_descriptor_before_publication() {
    for remove in [false, true] {
        let f = auxiliary_fixture();
        let runtime = runtime();
        let plan = f.preview(&runtime, "main:Report.Totals", "auxiliaryForm", "Own");
        assert!(plan.ok, "{plan:?}");
        let descriptor = f.source.join("Reports/Totals/Forms/Own.xml");
        if remove {
            std::fs::remove_file(descriptor).unwrap();
        } else {
            let mut bytes = std::fs::read(&descriptor).unwrap();
            bytes.extend_from_slice(b"\n<!-- external -->\n");
            std::fs::write(descriptor, bytes).unwrap();
        }
        let before = unica_workspace::test_support::tree_snapshot(&f.source);
        let published = f.call(
            &runtime,
            ToolIdentity::Apply,
            json!({"executionToken":plan.data.unwrap()["executionToken"]}),
        );
        assert!(!published.ok, "changed form accepted: {published:?}");
        assert_eq!(published.diagnostics[0]["code"], "stale_revision");
        assert_eq!(
            unica_workspace::test_support::tree_snapshot(&f.source),
            before
        );
    }
}

#[test]
fn owned_form_assignment_reads_staged_descriptors_and_accepts_ordinary_forms() {
    let f = auxiliary_fixture();
    let runtime = runtime();
    let before = unica_workspace::test_support::tree_snapshot(&f.source);
    let plan = f.call(
        &runtime,
        ToolIdentity::Apply,
        json!({"at":"main:Report.Totals","ops":[
            {"op":"form.add","args":{"items":[{"name":"Added","type":"ObjectForm"}]}},
            {"op":"form.set","args":{"values":{"auxiliaryForm":"Added"}}}
        ]}),
    );
    assert!(plan.ok, "{plan:?}");
    assert_eq!(
        unica_workspace::test_support::tree_snapshot(&f.source),
        before
    );
    let applied = f.call(
        &runtime,
        ToolIdentity::Apply,
        json!({"executionToken":plan.data.unwrap()["executionToken"]}),
    );
    assert!(applied.ok, "{applied:?}");
    f.replace(
        "Reports/Totals/Forms/Own.xml",
        "<FormType>Managed</FormType>",
        "<FormType>Ordinary</FormType>",
    );
    let plan = f.preview(&runtime, "main:Report.Totals", "auxiliaryForm", "Own");
    assert!(
        plan.ok,
        "an ordinary owned form remains assignable: {plan:?}"
    );
}
