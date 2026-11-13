//! Ответ человека снимает свой запрет, остальные цели пакета проверяются
//! (#2924).

use super::support_node_records::declare_vendor_nodes;
use super::*;

const ATTRIBUTES: [(&str, &str); 3] = [
    ("Code", "55555555-5555-4555-8555-555555555555"),
    ("Name", "66666666-6666-4666-8666-666666666666"),
    ("Note", "77777777-7777-4777-8777-777777777777"),
];

/// The catalog editable with support and its first `count` own attributes,
/// each locked.
fn locked_attributes(count: usize) -> Fixture {
    let fixture = Fixture::new("workspace-support-form");
    let declared = ATTRIBUTES[..count]
        .iter()
        .map(|(name, uuid)| {
            format!("<Attribute uuid=\"{uuid}\"><Properties><Name>{name}</Name></Properties></Attribute>")
        })
        .collect::<String>();
    let records = ATTRIBUTES[..count]
        .iter()
        .map(|(_, uuid)| (*uuid, 0))
        .collect::<Vec<_>>();
    declare_vendor_nodes(&fixture, &declared, &records);
    fixture
}

fn answer(refused: &DomainResult, code: &str, object: &str, option: &str) -> Value {
    assert!(!refused.ok, "{refused:?}");
    let choice = refused
        .human_choice
        .as_deref()
        .unwrap_or_else(|| panic!("deny asks the person: {refused:?}"));
    assert_eq!(choice.code, code);
    assert_eq!(choice.data.as_ref().unwrap()["object"], object);
    let option = choice
        .options
        .iter()
        .find(|candidate| candidate.id == option)
        .expect("the offered answer");
    Value::Object(option.arguments.clone())
}

/// One `apply` edits `count` locked attributes: each answer lifts only the
/// lock it was asked about, the next target asks its own question, and the
/// call that carries every answer plans the support changes with the edits
/// and publishes them together.
fn answers_lift_one_lock_each(count: usize) {
    let fixture = locked_attributes(count);
    let runtime = runtime();
    let before = unica_workspace::test_support::tree_snapshot(&fixture.workspace);
    let ops = ATTRIBUTES[..count]
        .iter()
        .map(|(name, _)| {
            json!({"op":"attribute.set","args":{
                "at":format!("main:Catalog.Items.Attribute.{name}"),
                "values":{"comment":format!("Edited {name}")}}})
        })
        .collect::<Vec<_>>();
    let mut request = json!({"at":"main:Catalog.Items","ops":ops});

    for (name, _) in &ATTRIBUTES[..count] {
        let refused = call(&runtime, &fixture, ToolIdentity::Apply, request);
        assert!(
            refused.summary.contains(&format!(
                "`main:Catalog.Items.Attribute.{name}` is a vendor object with the support rule `locked`"
            )),
            "{}",
            refused.summary
        );
        request = answer(
            &refused,
            "support.object_locked",
            &format!("Catalog.Items.Attribute.{name}"),
            "editable",
        );
        assert_eq!(
            unica_workspace::test_support::tree_snapshot(&fixture.workspace),
            before
        );
    }

    let planned = call(&runtime, &fixture, ToolIdentity::Apply, request);
    assert!(planned.ok, "{planned:?}");
    let data = planned.data.as_ref().unwrap();
    assert_eq!(data["mode"], "preview");
    let published = call(
        &runtime,
        &fixture,
        ToolIdentity::Apply,
        json!({"executionToken": data["executionToken"]}),
    );
    assert!(published.ok, "{published:?}");
    let marker =
        std::fs::read_to_string(fixture.source.join("Ext/ParentConfigurations.bin")).unwrap();
    let catalog = std::fs::read_to_string(fixture.source.join("Catalogs/Items.xml")).unwrap();
    for (name, uuid) in &ATTRIBUTES[..count] {
        assert!(marker.contains(&format!("1,0,{uuid}")), "{marker}");
        assert!(catalog.contains(&format!("Edited {name}")), "{catalog}");
    }
}

/// An answer about the first of two targets does not let the second, also
/// locked, through: it asks its own question.
/// rule: INV.SOURCE.SUPPORT-SEPARATE-APPLY
/// rule: INV.SAFETY.APPLY-SUPPORT-GATE
#[test]
fn an_answer_lifts_its_own_lock_and_the_next_target_asks_anew() {
    answers_lift_one_lock_each(2);
}

/// Three locked targets take three answers in turn; each lock is asked about
/// once, so the questions end. The limit of questions per call is the MCP
/// door's (`a_fourth_question_keeps_the_refusal` in `unica-mcp`).
/// rule: INV.SOURCE.SUPPORT-SEPARATE-APPLY
#[test]
fn three_locked_targets_take_three_answers_in_turn() {
    answers_lift_one_lock_each(3);
}

/// Under a configuration whose changes are disabled, enabling them puts
/// every object rule back to `locked`: an object the answer did not open
/// stays closed, and its edit in the same `apply` asks its own question.
/// rule: INV.SOURCE.SUPPORT-SEPARATE-APPLY
/// rule: INV.SAFETY.APPLY-SUPPORT-GATE
#[test]
fn enabling_changes_for_one_object_leaves_the_others_closed() {
    let fixture = locked_attributes(1);
    // Changes of the whole configuration are disabled.
    let marker = fixture.source.join("Ext/ParentConfigurations.bin");
    let text = std::fs::read_to_string(&marker).unwrap();
    let changed = text.replacen("{6,0,1,", "{6,1,1,", 1);
    assert_ne!(changed, text);
    std::fs::write(&marker, changed).unwrap();
    let runtime = runtime();
    let edits = json!({"at":"main:Catalog.Items","ops":[
        {"op":"attribute.set","args":{"at":"main:Catalog.Items.Attribute.Code","values":{"comment":"First"}}}
    ]});

    let refused = call(&runtime, &fixture, ToolIdentity::Apply, edits);
    let first = answer(
        &refused,
        "support.configuration_locked",
        "Catalog.Items",
        "editable",
    );
    // The catalog is opened, its own locked attribute is not.
    let refused = call(&runtime, &fixture, ToolIdentity::Apply, first);
    assert!(
        refused.summary.contains(
            "`main:Catalog.Items.Attribute.Code` is a vendor object with the support rule `locked`"
        ),
        "{}",
        refused.summary
    );
    let both = answer(
        &refused,
        "support.object_locked",
        "Catalog.Items.Attribute.Code",
        "editable",
    );
    let planned = call(&runtime, &fixture, ToolIdentity::Apply, both);
    assert!(planned.ok, "{planned:?}");
}
