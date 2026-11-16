use super::*;

fn refusal(runtime: &V5CanonicalInvocationRuntime, fixture: &Fixture, args: Value) -> String {
    let refused = call(runtime, fixture, ToolIdentity::Apply, args);
    assert!(!refused.ok, "{refused:?}");
    assert_eq!(
        refused.diagnostics[0]["code"], "invalid_state",
        "{refused:?}"
    );
    refused.summary
}

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

/// A form with its own `locked` record is closed under an editable catalog:
/// `apply` refuses its edits naming the form, `view` shows it locked with
/// every edit marked, and its own `editable` rule opens it again.
/// rule: INV.SAFETY.APPLY-SUPPORT-GATE
/// rule: INV.APP.SUPPORT-STATE
#[test]
fn a_locked_form_is_closed_under_an_editable_object() {
    let fixture = Fixture::new("workspace-support-form");
    let runtime = runtime();

    let form = view(&runtime, &fixture, "main:Catalog.Items.Form.Main");
    assert_eq!(
        support(&form),
        &json!({"editable": false, "reason": "locked", "policy": "deny", "rule": "locked"})
    );
    for (op, blocked) in marks(&form) {
        if op != "supportRule.set" {
            assert_eq!(blocked.as_deref(), Some("locked"), "{op}");
        }
    }
    let message = refusal(
        &runtime,
        &fixture,
        json!({"at":"main:Catalog.Items.Form.Main.Module.Form","ops":[{"op":"code.insert","args":{"text":"// edit\n"}}]}),
    );
    assert!(
        message.contains(
            "`main:Catalog.Items.Form.Main` is a vendor object with the support rule `locked`"
        ),
        "{message}"
    );

    publish(
        &runtime,
        &fixture,
        json!({"at":"main:Catalog.Items.Form.Main","ops":[{"op":"supportRule.set","args":{"values":{"rule":"editable"}}}]}),
    );
    let form = view(&runtime, &fixture, "main:Catalog.Items.Form.Main");
    assert_eq!(support(&form)["reason"], "editableWithSupport");
    assert_eq!(support(&form)["editable"], true);
    let edited = call(
        &runtime,
        &fixture,
        ToolIdentity::Apply,
        json!({"at":"main:Catalog.Items.Form.Main.Module.Form","ops":[{"op":"code.insert","args":{"text":"// edit
"}}]}),
    );
    assert!(edited.ok, "{edited:?}");
    // The other form keeps its own lock.
    let other = view(&runtime, &fixture, "main:Catalog.Items.Form.Other");
    assert_eq!(support(&other)["reason"], "locked");
}

/// An attribute with its own `locked` record is closed under an editable
/// catalog: `apply` refuses its edit naming the attribute, also when the
/// request addresses the catalog and the operation the attribute, `view`
/// shows it locked with its edits marked, and `supportRule.set` changes that
/// record, not its catalog's.
/// rule: INV.SAFETY.APPLY-SUPPORT-GATE
#[test]
fn an_attribute_answers_for_its_own_support_record() {
    const ATTRIBUTE: &str = "55555555-5555-4555-8555-555555555555";
    let fixture = Fixture::new("workspace-support-form");
    // The vendor declares the attribute and puts it on support, locked.
    declare_vendor_nodes(
        &fixture,
        &format!("<Attribute uuid=\"{ATTRIBUTE}\"><Properties><Name>Code</Name></Properties></Attribute>"),
        &[(ATTRIBUTE, 0)],
    );
    let marker = fixture.source.join("Ext/ParentConfigurations.bin");
    let runtime = runtime();
    let attribute = "main:Catalog.Items.Attribute.Code";
    let edit =
        json!({"at":attribute,"ops":[{"op":"attribute.set","args":{"values":{"comment":"Note"}}}]});

    let message = refusal(&runtime, &fixture, edit.clone());
    assert!(
        message.contains(
            "`main:Catalog.Items.Attribute.Code` is a vendor object with the support rule `locked`"
        ),
        "{message}"
    );
    // The same edit addressed from the catalog through `args.at`.
    let message = refusal(
        &runtime,
        &fixture,
        json!({"at":"main:Catalog.Items","ops":[{"op":"attribute.set","args":{"at":attribute,"values":{"comment":"Note"}}}]}),
    );
    assert!(
        message.contains(
            "`main:Catalog.Items.Attribute.Code` is a vendor object with the support rule `locked`"
        ),
        "{message}"
    );
    let node = view(&runtime, &fixture, attribute);
    assert_eq!(
        support(&node),
        &json!({"editable": false, "reason": "locked", "policy": "deny", "rule": "locked"})
    );
    for (op, blocked) in marks(&node) {
        if op != "supportRule.set" {
            assert_eq!(blocked.as_deref(), Some("locked"), "{op}");
        }
    }
    // The catalog itself stays editable.
    let catalog = view(&runtime, &fixture, "main:Catalog.Items");
    assert_eq!(support(&catalog)["reason"], "editableWithSupport");

    publish(
        &runtime,
        &fixture,
        json!({"at":attribute,"ops":[{"op":"supportRule.set","args":{"values":{"rule":"editable"}}}]}),
    );
    let marker = std::fs::read_to_string(&marker).unwrap();
    assert!(marker.contains(&format!("1,0,{ATTRIBUTE}")), "{marker}");
    assert!(
        marker.contains("1,0,44444444-4444-4444-8444-444444444444"),
        "{marker}"
    );
    let node = view(&runtime, &fixture, attribute);
    assert_eq!(
        support(&node),
        &json!({"editable": true, "reason": "editableWithSupport", "policy": "deny", "rule": "editableWithSupport"})
    );
    assert!(marks(&node).iter().all(|(_, blocked)| blocked.is_none()));
    let unlocked = call(&runtime, &fixture, ToolIdentity::Apply, edit);
    assert!(unlocked.ok, "{unlocked:?}");
}

/// Declares vendor nodes in the catalog of the fixture (`child_objects`)
/// and registers their own records `(uuid, rule)` in the marker.
pub(super) fn declare_vendor_nodes(fixture: &Fixture, child_objects: &str, records: &[(&str, u8)]) {
    let descriptor = fixture.source.join("Catalogs/Items.xml");
    let text = std::fs::read_to_string(&descriptor).unwrap();
    std::fs::write(
        &descriptor,
        text.replace("<ChildObjects>", &format!("<ChildObjects>{child_objects}")),
    )
    .unwrap();
    let marker = fixture.source.join("Ext/ParentConfigurations.bin");
    let text = std::fs::read_to_string(&marker).unwrap();
    let mut entries = String::new();
    for (uuid, rule) in records {
        entries.push_str(&format!(",{rule},0,{uuid},{uuid}"));
    }
    let text = text
        .replacen(",3,1,0,", &format!(",{},1,0,", 3 + records.len()), 1)
        .replacen('}', &format!("{entries}}}"), 1);
    std::fs::write(&marker, text).unwrap();
}

/// A tabular section editable with support keeps its edits open while its
/// own attribute is `locked`: the attribute alone is closed, also when the
/// operation reaches it from the tabular section through `args.at`.
/// rule: INV.SAFETY.APPLY-SUPPORT-GATE
/// rule: INV.APP.SUPPORT-STATE
#[test]
fn a_locked_attribute_of_an_editable_tabular_section_is_closed_alone() {
    const LINES: &str = "66666666-6666-4666-8666-666666666666";
    const QUANTITY: &str = "77777777-7777-4777-8777-777777777777";
    let fixture = Fixture::new("workspace-support-form");
    declare_vendor_nodes(
        &fixture,
        &format!("<TabularSection uuid=\"{LINES}\"><Properties><Name>Lines</Name></Properties><ChildObjects><Attribute uuid=\"{QUANTITY}\"><Properties><Name>Quantity</Name></Properties></Attribute></ChildObjects></TabularSection>"),
        &[(LINES, 1), (QUANTITY, 0)],
    );
    let runtime = runtime();
    let lines = "main:Catalog.Items.TabularSection.Lines";
    let quantity = "main:Catalog.Items.TabularSection.Lines.Attribute.Quantity";

    let section = view(&runtime, &fixture, lines);
    assert_eq!(support(&section)["reason"], "editableWithSupport");
    assert_eq!(support(&section)["rule"], "editableWithSupport");
    let node = view(&runtime, &fixture, quantity);
    assert_eq!(
        support(&node),
        &json!({"editable": false, "reason": "locked", "policy": "deny", "rule": "locked"})
    );

    let message = refusal(
        &runtime,
        &fixture,
        json!({"at":lines,"ops":[{"op":"attribute.set","args":{"at":quantity,"values":{"comment":"Note"}}}]}),
    );
    assert!(
        message.contains(&format!(
            "`{quantity}` is a vendor object with the support rule `locked`"
        )),
        "{message}"
    );
    let edited = call(
        &runtime,
        &fixture,
        ToolIdentity::Apply,
        json!({"at":lines,"ops":[{"op":"tabularSection.set","args":{"values":{"comment":"Note"}}}]}),
    );
    assert!(edited.ok, "{edited:?}");
}

/// A nested subsystem carries its own rule; `supportRule.set` changes it and
/// leaves its parent's rule as it is.
/// rule: INV.APP.SUPPORT-STATE
#[test]
fn a_nested_subsystem_rule_is_set_on_its_own_record() {
    let fixture = Fixture::new("workspace-support-view");
    let runtime = runtime();
    let child = "main:Subsystem.Parent.Subsystem.Child";
    assert_eq!(
        support(&view(&runtime, &fixture, child))["rule"],
        "editableWithSupport"
    );

    publish(
        &runtime,
        &fixture,
        json!({"at":child,"ops":[{"op":"supportRule.set","args":{"values":{"rule":"off-support"}}}]}),
    );
    assert_eq!(
        support(&view(&runtime, &fixture, child))["rule"],
        "removedFromSupport"
    );
    let parent = view(&runtime, &fixture, "main:Subsystem.Parent");
    assert_eq!(support(&parent)["rule"], "locked");
}

/// `supportRule.set` names the nodes that carry a record; a node without one
/// is not its target.
/// rule: INV.WIRE.V13-CAN-DICTIONARY
#[test]
fn support_rule_targets_every_node_with_its_own_record() {
    use unica_address::address::QualifiedAddress;
    for (at, applies) in [
        ("main:Catalog.Items", true),
        ("main:Catalog.Items.Form.Main", true),
        ("main:Catalog.Items.Template.Print", true),
        ("main:Catalog.Items.Command.Refresh", true),
        ("main:Catalog.Items.Attribute.Code", true),
        ("main:Catalog.Items.TabularSection.Lines", true),
        (
            "main:Catalog.Items.TabularSection.Lines.Attribute.Quantity",
            true,
        ),
        ("main:InformationRegister.Rates.Dimension.Currency", true),
        ("main:InformationRegister.Rates.Resource.Rate", true),
        ("main:Subsystem.Parent.Subsystem.Child", true),
        ("main:Subsystem.Parent.Subsystem.Child.Subsystem.Leaf", true),
        ("main:Catalog.Items.Form.Main.Item.Field", false),
        ("main:Catalog.Items.Module.Object", false),
        ("main:Catalog.Items.Attribute", false),
        ("main:Catalog.Items.Subsystem.Child", false),
        ("main:Subsystem.Parent.Attribute.Code", false),
    ] {
        let at = QualifiedAddress::parse(at).unwrap();
        assert_eq!(
            unica_metadata::applicability::support_record_path(at.segments()),
            applies,
            "{at}"
        );
    }
}
