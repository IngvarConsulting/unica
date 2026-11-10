use super::*;

fn support(data: &Value) -> &Value {
    &data["props"]["support"]
}

fn blocked_ops(data: &Value) -> Vec<(String, Option<String>)> {
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

/// A locked vendor object and its own nodes show one editability, the one
/// `apply` acts on: not editable under `deny`, with every edit in `can`
/// marked and the support operations left open; the old support fields are
/// gone.
/// rule: INV.APP.SUPPORT-STATE
/// rule: INV.WIRE.V13-CAN-DICTIONARY
#[test]
fn a_locked_object_and_its_nodes_show_the_editability_apply_acts_on() {
    let fixture = Fixture::new("workspace-support-view");
    let runtime = runtime();

    let object = view(&runtime, &fixture, "main:Catalog.Items");
    assert_eq!(
        support(&object),
        &json!({"editable": false, "reason": "locked", "policy": "deny", "rule": "locked"})
    );
    for old in ["supportRule", "supportRuleReason", "directEditSafe"] {
        assert!(object["props"].get(old).is_none(), "{old}");
    }
    let ops = blocked_ops(&object);
    assert!(!ops.is_empty());
    for (op, blocked) in &ops {
        if op == "supportRule.set" || op == "supportCapability.set" {
            assert_eq!(blocked, &None, "{op}");
        } else {
            assert_eq!(blocked.as_deref(), Some("locked"), "{op}");
        }
    }
    assert!(ops.iter().any(|(op, _)| op == "supportRule.set"));

    // A node under the object takes the object's editability; it has no own
    // record here, so it names no rule.
    let attributes = view(&runtime, &fixture, "main:Catalog.Items.Attribute");
    assert_eq!(
        support(&attributes),
        &json!({"editable": false, "reason": "locked", "policy": "deny"})
    );

    // The tree's lock without reading the object.
    let catalogs = view(&runtime, &fixture, "main:Catalog");
    let items = catalogs["items"].as_array().unwrap();
    let row = items
        .iter()
        .find(|item| item["at"] == "main:Catalog.Items")
        .unwrap();
    assert_eq!(row["support"], "locked");
    assert!(row["props"].get("support").is_none());
}

/// Without a marker nothing is on support; an unreadable marker is not
/// "not on support" but `unknown`, which `deny` does not let edit.
/// rule: INV.APP.SUPPORT-STATE
#[test]
fn absent_and_unreadable_support_are_told_apart() {
    let fixture = Fixture::new("workspace-support-view");
    let runtime = runtime();

    let free = view(&runtime, &fixture, "absent:Catalog.Items");
    assert_eq!(
        support(&free),
        &json!({"editable": true, "reason": "notSupported", "policy": "deny"})
    );
    assert!(blocked_ops(&free).iter().all(|(_, blocked)| blocked.is_none()));

    // A set whose support state cannot be read refuses the view of its
    // objects: unreadable is not "not on support".
    let unreadable = call(
        &runtime,
        &fixture,
        ToolIdentity::View,
        json!({"at": "unreadable:Catalog.Items"}),
    );
    assert!(!unreadable.ok, "{unreadable:?}");
}

/// The project policy `warn` lets `apply` edit a locked object with a
/// warning, so the node is editable and nothing in `can` is marked.
/// rule: INV.APP.SUPPORT-STATE
/// rule: INV.SAFETY.SUPPORT-POLICY-DOWNGRADE
#[test]
fn a_warn_policy_makes_a_locked_object_editable() {
    let fixture = Fixture::new("workspace-support-view");
    std::fs::write(
        fixture.workspace.join(".v8-project.json"),
        r#"{"editingAllowedCheck":"warn"}"#,
    )
    .unwrap();
    let runtime = runtime();

    let object = view(&runtime, &fixture, "main:Catalog.Items");
    assert_eq!(
        support(&object),
        &json!({"editable": true, "reason": "locked", "policy": "warn", "rule": "locked"})
    );
    assert!(blocked_ops(&object).iter().all(|(_, blocked)| blocked.is_none()));
}
