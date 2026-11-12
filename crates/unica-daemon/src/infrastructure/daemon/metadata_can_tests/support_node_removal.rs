//! Удаление подчинённого узла и запертые узлы под ним (#2918).

use super::support_node_records::declare_vendor_nodes;
use super::*;

fn support(data: &Value) -> &Value {
    &data["props"]["support"]
}

fn marks(data: &Value) -> Vec<(String, Option<String>)> {
    data["can"]
        .as_array()
        .expect("computed can")
        .iter()
        .map(|entry| {
            (
                entry["op"].as_str().unwrap().to_string(),
                entry["blocked"]["reason"].as_str().map(str::to_string),
            )
        })
        .collect()
}

const LINES: &str = "main:Catalog.Items.TabularSection.Lines";

/// The vendor tabular section `Lines` editable with support and its own
/// attribute `Quantity` locked.
fn lines_with_a_locked_attribute() -> Fixture {
    const LINES_UUID: &str = "66666666-6666-4666-8666-666666666666";
    const QUANTITY_UUID: &str = "77777777-7777-4777-8777-777777777777";
    let fixture = Fixture::new("workspace-support-form");
    declare_vendor_nodes(
        &fixture,
        &format!("<TabularSection uuid=\"{LINES_UUID}\"><Properties><Name>Lines</Name></Properties><ChildObjects><Attribute uuid=\"{QUANTITY_UUID}\"><Properties><Name>Quantity</Name></Properties></Attribute></ChildObjects></TabularSection>"),
        &[(LINES_UUID, 1), (QUANTITY_UUID, 0)],
    );
    fixture
}

/// Removing a node below the object answers for the locked nodes under it,
/// as removing the object does: under `deny` the removal of a tabular
/// section editable with support refuses while its own attribute is locked,
/// naming that attribute, also when the operation reaches the section from
/// the catalog through `args.at`; `view` marks the removal, not the
/// section's edits; once the section is off support it is removed.
/// rule: INV.SOURCE.VENDOR-OBJECT-REMOVAL
/// rule: INV.SAFETY.APPLY-SUPPORT-GATE
#[test]
fn removing_a_tabular_section_answers_for_its_locked_attribute() {
    let fixture = lines_with_a_locked_attribute();
    let runtime = runtime();
    let named = format!(
        "`{LINES}` has vendor child objects with the support rule `locked`: `{LINES}.Attribute.Quantity`"
    );

    let section = view(&runtime, &fixture, LINES);
    assert_eq!(support(&section)["editable"], true);
    let marked = marks(&section);
    assert!(
        marked.iter().any(|(op, _)| op == "tabularSection.remove"),
        "{marked:?}"
    );
    for (op, blocked) in marked {
        let expected = (op == "tabularSection.remove").then_some("locked");
        assert_eq!(blocked.as_deref(), expected, "{op}");
    }

    for request in [
        json!({"at":LINES,"ops":[{"op":"tabularSection.remove","args":{}}]}),
        json!({"at":"main:Catalog.Items","ops":[{"op":"tabularSection.remove","args":{"at":LINES}}]}),
    ] {
        let before = unica_workspace::test_support::tree_snapshot(&fixture.workspace);
        let refused = call(&runtime, &fixture, ToolIdentity::Apply, request);
        assert!(!refused.ok, "{refused:?}");
        assert_eq!(refused.diagnostics[0]["code"], "invalid_state");
        assert!(refused.summary.contains(&named), "{}", refused.summary);
        assert!(
            refused.summary.contains("off-support"),
            "{}",
            refused.summary
        );
        let choice = refused
            .human_choice
            .as_deref()
            .expect("deny asks the person");
        assert_eq!(choice.code, "support.remove_locked");
        assert_eq!(
            choice.data.as_ref().unwrap()["object"],
            "Catalog.Items.TabularSection.Lines"
        );
        assert_eq!(
            unica_workspace::test_support::tree_snapshot(&fixture.workspace),
            before
        );
    }

    publish(
        &runtime,
        &fixture,
        json!({"at":LINES,"ops":[{"op":"supportRule.set","args":{"values":{"rule":"off-support"}}}]}),
    );
    let section = view(&runtime, &fixture, LINES);
    assert!(marks(&section)
        .iter()
        .all(|(_, blocked)| blocked.is_none()));
    publish(
        &runtime,
        &fixture,
        json!({"at":LINES,"ops":[{"op":"tabularSection.remove","args":{}}]}),
    );
    let catalog = std::fs::read_to_string(fixture.source.join("Catalogs/Items.xml")).unwrap();
    assert!(!catalog.contains("<Name>Lines</Name>"), "{catalog}");
}

/// Under `warn` the removal of a tabular section with a locked attribute
/// goes ahead with the warning that names the attribute.
/// rule: INV.SOURCE.VENDOR-OBJECT-REMOVAL
#[test]
fn a_warn_policy_removes_a_tabular_section_with_a_locked_attribute() {
    let fixture = lines_with_a_locked_attribute();
    std::fs::write(
        fixture.workspace.join(".v8-project.json"),
        r#"{"editingAllowedCheck":"warn"}"#,
    )
    .unwrap();
    let runtime = runtime();

    let section = view(&runtime, &fixture, LINES);
    assert!(marks(&section)
        .iter()
        .all(|(_, blocked)| blocked.is_none()));
    let published = publish(
        &runtime,
        &fixture,
        json!({"at":LINES,"ops":[{"op":"tabularSection.remove","args":{}}]}),
    );
    assert_eq!(published.warnings.len(), 1, "{published:?}");
    assert_eq!(published.warnings[0]["code"], "support_locked");
    assert_eq!(published.warnings[0]["at"], LINES);
    let message = published.warnings[0]["message"].as_str().unwrap();
    assert!(
        message.contains(&format!("`{LINES}.Attribute.Quantity`")),
        "{message}"
    );
}

/// A locked form is removed only once it is off support: its removal asks
/// to take it off support, as the removal of a locked object does, and
/// `view` marks it.
/// rule: INV.SOURCE.VENDOR-OBJECT-REMOVAL
#[test]
fn removing_a_locked_form_asks_to_take_it_off_support() {
    let fixture = Fixture::new("workspace-support-form");
    let runtime = runtime();
    let form = "main:Catalog.Items.Form.Main";

    let refused = call(
        &runtime,
        &fixture,
        ToolIdentity::Apply,
        json!({"at":form,"ops":[{"op":"form.remove","args":{}}]}),
    );
    assert!(!refused.ok, "{refused:?}");
    assert!(
        refused.summary.contains(&format!(
            "`{form}` is a vendor object with the support rule `locked`"
        )),
        "{}",
        refused.summary
    );
    assert!(
        refused.summary.contains("off-support"),
        "{}",
        refused.summary
    );
    let choice = refused
        .human_choice
        .as_deref()
        .expect("deny asks the person");
    assert_eq!(choice.code, "support.remove_locked");
    let node = view(&runtime, &fixture, form);
    assert!(marks(&node)
        .iter()
        .any(|(op, blocked)| op == "form.remove" && blocked.as_deref() == Some("locked")));
}

/// A nested subsystem editable with support under an editable parent is not
/// removed under `deny` while a subsystem nested in it is locked; the
/// refusal names that one.
/// rule: INV.SOURCE.VENDOR-OBJECT-REMOVAL
#[test]
fn removing_a_nested_subsystem_answers_for_its_locked_subsystems() {
    let fixture = Fixture::new("workspace-support-view");
    let main = fixture.workspace.join("main");
    let marker = main.join("Ext/ParentConfigurations.bin");
    let text = std::fs::read_to_string(&marker).unwrap();
    let changed = text
        .replacen(
            "0,0,11111111-1111-4111-8111-111111111111",
            "1,0,11111111-1111-4111-8111-111111111111",
            1,
        )
        .replacen('}', ",0,0,33333333-3333-4333-8333-333333333333}", 1);
    assert_ne!(changed, text);
    std::fs::write(&marker, changed).unwrap();
    let child = main.join("Subsystems/Parent/Subsystems/Child.xml");
    let text = std::fs::read_to_string(&child).unwrap();
    let changed = text.replace(
        "<ChildObjects></ChildObjects>",
        "<ChildObjects><Subsystem>Grandchild</Subsystem></ChildObjects>",
    );
    assert_ne!(changed, text);
    std::fs::write(&child, changed).unwrap();
    let grandchild = main.join("Subsystems/Parent/Subsystems/Child/Subsystems/Grandchild.xml");
    std::fs::create_dir_all(grandchild.parent().unwrap()).unwrap();
    std::fs::write(
        &grandchild,
        r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Subsystem uuid="33333333-3333-4333-8333-333333333333"><Properties><Name>Grandchild</Name><Synonym/><Comment/><IncludeHelpInContents>false</IncludeHelpInContents><IncludeInCommandInterface>true</IncludeInCommandInterface><UseOneCommand>false</UseOneCommand><Explanation/><Picture/><Content/></Properties><ChildObjects></ChildObjects></Subsystem></MetaDataObject>"#,
    )
    .unwrap();
    let runtime = runtime();
    let child = "main:Subsystem.Parent.Subsystem.Child";

    let node = view(&runtime, &fixture, child);
    assert_eq!(support(&node)["editable"], true, "{node}");
    assert!(marks(&node).iter().any(|(op, blocked)| {
        op == "childSubsystem.remove" && blocked.as_deref() == Some("locked")
    }));
    let refused = call(
        &runtime,
        &fixture,
        ToolIdentity::Apply,
        json!({"at":child,"ops":[{"op":"childSubsystem.remove","args":{}}]}),
    );
    assert!(!refused.ok, "{refused:?}");
    assert!(
        refused.summary.contains(&format!(
            "`{child}` has vendor child objects with the support rule `locked`: `{child}.Subsystem.Grandchild`"
        )),
        "{}",
        refused.summary
    );
}
