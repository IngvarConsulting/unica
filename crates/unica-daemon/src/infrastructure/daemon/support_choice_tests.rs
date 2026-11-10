//! `unica.apply` спрашивает человека о запрете поставщика и по его выбору
//! готовит один план из изменения поддержки и правки.

use super::actor_capacity_tests::direct_v5;
use super::*;
use unica_execution::invocation_store::ToolIdentity;
use unica_execution_contract::invocation::{DomainResult, HUMAN_CONFIRMATION_KEY};

const OBJECT_UUID: &str = "11111111-1111-4111-8111-111111111111";

/// A configuration on vendor support whose `Document.First` carries `rule`.
fn vendor_workspace(rule: u8, editing_enabled: bool) -> tempfile::TempDir {
    let workspace = tempfile::tempdir().unwrap();
    let source = workspace.path().join("src");
    for directory in ["Documents", "Languages", "Ext"] {
        std::fs::create_dir_all(source.join(directory)).unwrap();
    }
    std::fs::write(
        workspace.path().join("v8project.yaml"),
        "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
    )
    .unwrap();
    std::fs::write(
        source.join("Configuration.xml"),
        r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Configuration><Properties><Name>Store</Name><DefaultLanguage>Language.Русский</DefaultLanguage></Properties><ChildObjects><Language>Русский</Language><Document>First</Document></ChildObjects></Configuration></MetaDataObject>"#,
    )
    .unwrap();
    std::fs::write(
        source.join("Languages/Русский.xml"),
        r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Language uuid="33333333-3333-4333-8333-333333333333"><Properties><Name>Русский</Name><Comment/><LanguageCode>ru</LanguageCode></Properties></Language></MetaDataObject>"#,
    )
    .unwrap();
    std::fs::write(
        source.join("Documents/First.xml"),
        format!(
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Document uuid="{OBJECT_UUID}"><Properties><Name>First</Name><Synonym/><Comment/></Properties><ChildObjects/></Document></MetaDataObject>"#
        ),
    )
    .unwrap();
    std::fs::write(
        source.join("Ext/ParentConfigurations.bin"),
        format!(
            "\u{feff}{{6,{},1,dddddddd-dddd-4ddd-8ddd-dddddddddddd,0,33333333-3333-4333-8333-333333333333,\"1.0\",\"Vendor\",\"VendorConf\",3,{rule},0,{OBJECT_UUID},{OBJECT_UUID}}}",
            u8::from(!editing_enabled)
        ),
    )
    .unwrap();
    workspace
}

fn runtime() -> V5CanonicalInvocationRuntime {
    V5CanonicalInvocationRuntime::new(
        Arc::new(crate::infrastructure::daemon::v13_service::CanonicalV13ReadService::default()),
        Arc::new(TokioClock),
    )
}

fn apply(
    runtime: &V5CanonicalInvocationRuntime,
    workspace: &std::path::Path,
    arguments: serde_json::Value,
) -> DomainResult {
    let hint = std::fs::canonicalize(workspace).unwrap();
    let request = InvocationRequest::new(
        ToolIdentity::Apply,
        arguments,
        hint.to_string_lossy(),
        7_000,
    )
    .unwrap();
    direct_v5(runtime, request).unwrap()
}

/// The agreed text of the vendor-lock questions: the heading, the labels
/// and what each answer loses. Labels change only together with this test.
/// rule: INV.SAFETY.APPLY-SUPPORT-GATE
/// rule: INV.SOURCE.VENDOR-OBJECT-REMOVAL
/// rule: INV.MCP.HUMAN-CHOICE
#[test]
fn a_locked_vendor_object_asks_the_person_in_the_agreed_words() {
    let runtime = runtime();
    let workspace = vendor_workspace(0, true);
    let source = workspace.path().join("src");
    let before = unica_workspace::test_support::tree_snapshot(&source);

    let edit = apply(
        &runtime,
        workspace.path(),
        serde_json::json!({"at": "main:Document.First", "ops": [{"op": "props.set", "args": {"values": {"Comment": "local"}}}]}),
    );
    assert!(!edit.ok, "{edit:?}");
    assert_eq!(edit.diagnostics[0]["code"], "invalid_state", "{edit:?}");
    let choice = edit.human_choice.as_deref().expect("deny asks the person");
    assert_eq!(choice.code, "support.object_locked");
    assert_eq!(
        choice.question,
        "Объект «Document.First» набора «main» на поддержке поставщика: правка запрещена."
    );
    let answers = choice
        .options
        .iter()
        .map(|option| (option.title.as_str(), option.description.as_str()))
        .collect::<Vec<_>>();
    assert_eq!(
        answers,
        [
            (
                "Разрешить правку с сохранением поддержки",
                "при обновлении от поставщика по объекту возможны конфликты"
            ),
            (
                "Снять объект с поддержки",
                "обновления поставщика по объекту перестанут приходить"
            ),
        ]
    );
    assert_eq!(
        choice.cancel_description.as_deref(),
        Some("доработку можно внести в расширение")
    );
    assert!(!choice.proceeds_unasked);

    let removal = apply(
        &runtime,
        workspace.path(),
        serde_json::json!({"at": "main:Document.First", "ops": [{"op": "object.remove"}]}),
    );
    let choice = removal
        .human_choice
        .as_deref()
        .expect("deny asks the person");
    assert_eq!(choice.code, "support.remove_locked");
    assert_eq!(
        choice.question,
        "Объект «Document.First» на поддержке поставщика: удалить его можно только после снятия с поддержки."
    );
    assert_eq!(choice.options.len(), 1);
    assert_eq!(choice.options[0].title, "Снять с поддержки и удалить");
    assert_eq!(
        choice.options[0].description,
        "обновления поставщика по объекту прекратятся, объект будет удалён"
    );
    assert_eq!(choice.cancel_description, None);
    assert_eq!(
        unica_workspace::test_support::tree_snapshot(&source),
        before
    );

    let disabled = vendor_workspace(1, false);
    let edit = apply(
        &runtime,
        disabled.path(),
        serde_json::json!({"at": "main:Document.First", "ops": [{"op": "props.set", "args": {"values": {"Comment": "local"}}}]}),
    );
    let choice = edit.human_choice.as_deref().expect("deny asks the person");
    assert_eq!(choice.code, "support.configuration_locked");
    assert_eq!(
        choice.question,
        "Конфигурация набора «main» на поддержке поставщика без возможности изменения."
    );
    assert_eq!(choice.options.len(), 2);
    assert_eq!(
        choice.cancel_description.as_deref(),
        Some("доработку можно внести в расширение")
    );
}

/// The answer is the same call with the door's confirmation: it prepares one
/// plan that changes the object's support and makes the edit, publishes both
/// by its execution token, and another answer or a changed call is asked
/// again instead of planned.
/// rule: INV.SOURCE.SUPPORT-SEPARATE-APPLY
/// rule: INV.SAFETY.APPLY-SUPPORT-GATE
#[test]
fn the_persons_answer_plans_the_support_change_and_the_edit_together() {
    let runtime = runtime();
    let workspace = vendor_workspace(0, true);
    let source = workspace.path().join("src");
    let marker = std::fs::read(source.join("Ext/ParentConfigurations.bin")).unwrap();
    let before = unica_workspace::test_support::tree_snapshot(&source);
    let call = serde_json::json!({"at": "main:Document.First", "ops": [{"op": "object.remove"}]});

    let refused = apply(&runtime, workspace.path(), call.clone());
    let choice = refused.human_choice.expect("deny asks the person");
    let answer = choice.options[0].arguments.clone();
    assert!(answer.contains_key(HUMAN_CONFIRMATION_KEY));

    // A mark that is not the offered one is asked again, never planned.
    let mut forged = answer.clone();
    forged.insert(
        HUMAN_CONFIRMATION_KEY.to_string(),
        serde_json::json!("unica-confirmation-1:forged"),
    );
    let asked_again = apply(
        &runtime,
        workspace.path(),
        serde_json::Value::Object(forged),
    );
    assert!(!asked_again.ok, "{asked_again:?}");
    assert!(asked_again.human_choice.is_some(), "{asked_again:?}");
    // The mark of one call does not stand for another.
    let mut other_call = answer.clone();
    other_call.insert(
        "at".to_string(),
        serde_json::json!("main:Document.First.Form.Card"),
    );
    let other = apply(
        &runtime,
        workspace.path(),
        serde_json::Value::Object(other_call),
    );
    assert!(!other.ok, "{other:?}");
    assert_eq!(
        unica_workspace::test_support::tree_snapshot(&source),
        before
    );

    let planned = apply(
        &runtime,
        workspace.path(),
        serde_json::Value::Object(answer),
    );
    assert!(planned.ok, "{planned:?}");
    let data = planned.data.as_ref().unwrap();
    assert_eq!(data["mode"], "preview");
    let token = data["executionToken"].clone();
    assert!(token.is_string(), "{planned:?}");
    assert_eq!(
        unica_workspace::test_support::tree_snapshot(&source),
        before
    );

    let published = apply(
        &runtime,
        workspace.path(),
        serde_json::json!({"executionToken": token}),
    );
    assert!(published.ok, "{published:?}");
    assert!(!source.join("Documents/First.xml").exists());
    let changed = std::fs::read(source.join("Ext/ParentConfigurations.bin")).unwrap();
    assert_ne!(changed, marker, "the plan takes the object off support");
}

/// The plan the person's answer prepares is checked like any other: a
/// support marker changed on disk between planning and execution refuses
/// the execution with `stale_revision`, writes nothing and leaves the
/// foreign change in place.
/// rule: INV.SOURCE.REVISION-PROJECTION-CAPTURE-EQUALITY
/// rule: INV.SOURCE.SUPPORT-SEPARATE-APPLY
#[test]
fn a_support_marker_changed_after_the_answer_makes_its_plan_stale() {
    let runtime = runtime();
    let workspace = vendor_workspace(0, true);
    let source = workspace.path().join("src");
    let call = serde_json::json!({"at": "main:Document.First", "ops": [{"op": "object.remove"}]});

    let refused = apply(&runtime, workspace.path(), call);
    let answer = refused.human_choice.expect("deny asks the person").options[0]
        .arguments
        .clone();
    let planned = apply(
        &runtime,
        workspace.path(),
        serde_json::Value::Object(answer),
    );
    assert!(planned.ok, "{planned:?}");
    let token = planned.data.as_ref().unwrap()["executionToken"].clone();

    let marker = source.join("Ext/ParentConfigurations.bin");
    let foreign = std::fs::read_to_string(&marker)
        .unwrap()
        .replace("\"1.0\"", "\"1.1\"");
    std::fs::write(&marker, &foreign).unwrap();
    let changed = unica_workspace::test_support::tree_snapshot(&source);

    let stale = apply(
        &runtime,
        workspace.path(),
        serde_json::json!({"executionToken": token}),
    );
    assert!(!stale.ok, "{stale:?}");
    assert_eq!(stale.diagnostics[0]["code"], "stale_revision", "{stale:?}");
    assert_eq!(
        unica_workspace::test_support::tree_snapshot(&source),
        changed
    );
    assert!(source.join("Documents/First.xml").exists());
    assert_eq!(std::fs::read_to_string(&marker).unwrap(), foreign);
}

/// The same vendor workspace with the configuration in compatibility `mode`.
fn workspace_in_mode(rule: u8, mode: &str) -> tempfile::TempDir {
    let workspace = vendor_workspace(rule, true);
    let configuration = workspace.path().join("src/Configuration.xml");
    let xml = std::fs::read_to_string(&configuration).unwrap().replace(
        "<Name>Store</Name>",
        &format!("<Name>Store</Name><CompatibilityMode>{mode}</CompatibilityMode>"),
    );
    std::fs::write(&configuration, xml).unwrap();
    workspace
}

fn comment_edit(comment: &str) -> serde_json::Value {
    serde_json::json!({"at": "main:Document.First", "ops": [{"op": "props.set", "args": {"values": {"Comment": comment}}}]})
}

/// A set read-only by its mode asks the person in the agreed words; the
/// answer raises the mode to 8.3.8 and makes the edit in one plan.
/// rule: INV.SOURCE.WRITABLE-PROFILE
/// rule: INV.MCP.HUMAN-CHOICE
#[test]
fn a_read_only_mode_asks_the_person_and_the_answer_raises_it_with_the_edit() {
    let runtime = runtime();
    let workspace = workspace_in_mode(1, "Version8_2_13");
    let source = workspace.path().join("src");
    let before = unica_workspace::test_support::tree_snapshot(&source);

    let refused = apply(&runtime, workspace.path(), comment_edit("local"));
    assert!(!refused.ok, "{refused:?}");
    assert_eq!(
        refused.diagnostics[0]["code"], "invalid_source",
        "{refused:?}"
    );
    assert_eq!(
        refused.diagnostics[1]["code"], "compatibilityModeUnsupported",
        "{refused:?}"
    );
    let choice = refused.human_choice.expect("the person is asked");
    assert_eq!(choice.code, "source.compatibility_mode");
    assert_eq!(
        choice.question,
        "Набор «main» в режиме совместимости 8.2.13 можно только читать. Правки доступны с режима 8.3.8."
    );
    assert_eq!(choice.options.len(), 1);
    assert_eq!(
        choice.options[0].title,
        "Повысить режим до 8.3.8 и внести правку"
    );
    assert_eq!(
        choice.options[0].description,
        "поведение конфигурации может измениться"
    );
    assert_eq!(choice.cancel_description, None);
    assert!(!choice.proceeds_unasked);
    assert_eq!(
        unica_workspace::test_support::tree_snapshot(&source),
        before
    );

    let planned = apply(
        &runtime,
        workspace.path(),
        serde_json::Value::Object(choice.options[0].arguments.clone()),
    );
    assert!(planned.ok, "{planned:?}");
    assert_eq!(
        unica_workspace::test_support::tree_snapshot(&source),
        before
    );
    let token = planned.data.as_ref().unwrap()["executionToken"].clone();
    let published = apply(
        &runtime,
        workspace.path(),
        serde_json::json!({"executionToken": token}),
    );
    assert!(published.ok, "{published:?}");
    let configuration = std::fs::read_to_string(source.join("Configuration.xml")).unwrap();
    assert!(configuration.contains("<CompatibilityMode>Version8_3_8</CompatibilityMode>"));
    let document = std::fs::read_to_string(source.join("Documents/First.xml")).unwrap();
    assert!(document.contains("local"), "{document}");
}

/// A set both read-only by its mode and locked by vendor support asks two
/// questions one after another; the second keeps the first answer, and the
/// answers make one plan that raises the mode, unlocks the object and edits
/// it. Until the last answer nothing is written.
/// rule: INV.SOURCE.WRITABLE-PROFILE
/// rule: INV.SOURCE.SUPPORT-SEPARATE-APPLY
/// rule: INV.RUNTIME.DESTRUCTIVE-CONFIRMATION
#[test]
fn a_read_only_mode_and_a_vendor_lock_are_asked_in_turn_and_planned_once() {
    let runtime = runtime();
    let workspace = workspace_in_mode(0, "Version8_3_7");
    let source = workspace.path().join("src");
    let marker = std::fs::read(source.join("Ext/ParentConfigurations.bin")).unwrap();
    let before = unica_workspace::test_support::tree_snapshot(&source);

    let first = apply(&runtime, workspace.path(), comment_edit("local"));
    let mode = first.human_choice.expect("the mode is asked first");
    assert_eq!(mode.code, "source.compatibility_mode");
    let after_mode = mode.options[0].arguments.clone();

    let second = apply(
        &runtime,
        workspace.path(),
        serde_json::Value::Object(after_mode.clone()),
    );
    assert!(!second.ok, "{second:?}");
    // Refusing the second question leaves everything as it was: no plan,
    // no written byte.
    assert!(second
        .data
        .as_ref()
        .is_none_or(|data| data.get("executionToken").is_none()));
    assert_eq!(
        unica_workspace::test_support::tree_snapshot(&source),
        before
    );
    let support = second.human_choice.expect("the vendor lock is asked next");
    assert_eq!(support.code, "support.object_locked");
    let answer = support.options[0].arguments.clone();
    let marks = answer[HUMAN_CONFIRMATION_KEY].as_str().unwrap();
    assert!(
        marks.contains(after_mode[HUMAN_CONFIRMATION_KEY].as_str().unwrap()),
        "the second answer keeps the first: {marks}"
    );

    // A call changed between the questions keeps none of the answers: the
    // first question is asked again.
    let mut changed = answer.clone();
    changed["ops"][0]["args"]["values"]["Comment"] = serde_json::json!("other");
    let asked_again = apply(
        &runtime,
        workspace.path(),
        serde_json::Value::Object(changed),
    );
    assert_eq!(
        asked_again.human_choice.expect("asked again").code,
        "source.compatibility_mode"
    );
    assert_eq!(
        unica_workspace::test_support::tree_snapshot(&source),
        before
    );

    let planned = apply(
        &runtime,
        workspace.path(),
        serde_json::Value::Object(answer),
    );
    assert!(planned.ok, "{planned:?}");
    assert_eq!(
        unica_workspace::test_support::tree_snapshot(&source),
        before
    );
    let token = planned.data.as_ref().unwrap()["executionToken"].clone();
    let published = apply(
        &runtime,
        workspace.path(),
        serde_json::json!({"executionToken": token}),
    );
    assert!(published.ok, "{published:?}");
    let configuration = std::fs::read_to_string(source.join("Configuration.xml")).unwrap();
    assert!(configuration.contains("<CompatibilityMode>Version8_3_8</CompatibilityMode>"));
    assert_ne!(
        std::fs::read(source.join("Ext/ParentConfigurations.bin")).unwrap(),
        marker
    );
    let document = std::fs::read_to_string(source.join("Documents/First.xml")).unwrap();
    assert!(document.contains("local"), "{document}");
}
