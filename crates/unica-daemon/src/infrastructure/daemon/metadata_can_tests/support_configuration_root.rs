//! Public support operations on the configuration's own Platform XML record.
use super::*;

const ROOT: &str = "main:Configuration";
const MARKER: &str = "Ext/ParentConfigurations.bin";
const ROOT_UUID: &str = "bf5222b1-a591-40d2-aabf-ea9a296792d6";
const LOCKED: &[u8] = include_bytes!(
    "../../../../../unica-metadata-designer/tests/fixtures/support-markers/locked.bin"
);
const DISABLED: &[u8] =
    include_bytes!("../../../../../unica-metadata-designer/tests/fixtures/support-markers/off.bin");

fn root_with_support(marker: &[u8]) -> Fixture {
    // These dumps were produced from this same configuration by platform 8.3.27.
    let fixture = Fixture::new("workspace-exchange-content");
    let relative = fixture.source.join(MARKER);
    std::fs::create_dir_all(relative.parent().unwrap()).unwrap();
    std::fs::write(relative, marker).unwrap();
    fixture
}

fn ordinary_edit() -> Value {
    json!({
        "at": ROOT,
        "ops": [{"op":"props.set","args":{"values":{"Comment":"Edited with vendor support"}}}],
    })
}

/// rule: INV.APP.SUPPORT-STATE
/// rule: INV.SAFETY.APPLY-SUPPORT-GATE
/// rule: INV.SOURCE.SUPPORT-SEPARATE-APPLY
#[test]
fn configuration_rule_unlocks_only_the_root_and_allows_a_separate_edit() {
    let fixture = root_with_support(LOCKED);
    // The wrapper can carry a different UUID; only the direct Configuration
    // element owns the root record.
    let descriptor = fixture.source.join("Configuration.xml");
    let source = std::fs::read_to_string(&descriptor).unwrap();
    let source = source.replacen(
        "<MetaDataObject ",
        "<MetaDataObject uuid=\"00000000-0000-0000-0000-000000000001\" ",
        1,
    );
    std::fs::write(descriptor, source).unwrap();
    let runtime = runtime();
    let before = unica_workspace::test_support::tree_snapshot(&fixture.source);
    let root = view(&runtime, &fixture, ROOT);
    assert_eq!(root["props"]["support"]["rule"], "locked");
    assert_eq!(root["props"]["support"]["editable"], false);
    let contract = view(&runtime, &fixture, "main:Configuration.Can.supportRule.set");
    assert_eq!(contract["props"]["op"], "supportRule.set");

    publish(
        &runtime,
        &fixture,
        json!({"at":ROOT,"ops":[{"op":"supportCapability.set","args":{"values":{"enabled":true}}}]}),
    );
    assert_eq!(
        unica_workspace::test_support::tree_snapshot(&fixture.source),
        before
    );

    let refused = call(&runtime, &fixture, ToolIdentity::Apply, ordinary_edit());
    assert!(!refused.ok, "{refused:?}");
    assert!(
        refused.summary.contains("own vendor support rule"),
        "{refused:?}"
    );
    let next = &refused.next[0];
    assert_eq!(next["tool"], "unica.apply");
    assert_eq!(
        next["args"],
        json!({"at":ROOT,"ops":[{"op":"supportRule.set","args":{"values":{"rule":"editable"}}}]})
    );
    assert_eq!(
        unica_workspace::test_support::tree_snapshot(&fixture.source),
        before
    );

    let published = publish(&runtime, &fixture, next["args"].clone());
    assert!(published.ok, "{published:?}");
    let mut expected = before;
    let marker = String::from_utf8(expected[Path::new(MARKER)].clone()).unwrap();
    let after = marker.replacen(
        &format!(",0,0,{ROOT_UUID},"),
        &format!(",1,0,{ROOT_UUID},"),
        1,
    );
    assert_ne!(
        after, marker,
        "native marker carries the configuration's locked record"
    );
    expected.insert(PathBuf::from(MARKER), after.into_bytes());
    assert_eq!(
        unica_workspace::test_support::tree_snapshot(&fixture.source),
        expected
    );

    let root = view(&runtime, &fixture, ROOT);
    assert_eq!(root["props"]["support"]["rule"], "editableWithSupport");
    assert_eq!(root["props"]["support"]["editable"], true);
    publish(&runtime, &fixture, ordinary_edit());
    let edited = view(&runtime, &fixture, ROOT);
    assert_eq!(
        edited["props"]["properties"]["comment"],
        "Edited with vendor support"
    );
    assert_eq!(
        std::fs::read(fixture.source.join(MARKER)).unwrap(),
        expected[Path::new(MARKER)],
    );
}

/// rule: INV.SAFETY.APPLY-SUPPORT-GATE
#[test]
fn configuration_with_global_lock_distinguishes_the_capability_from_the_rule() {
    let fixture = root_with_support(DISABLED);
    let runtime = runtime();
    let before = unica_workspace::test_support::tree_snapshot(&fixture.source);
    let refused = call(&runtime, &fixture, ToolIdentity::Apply, ordinary_edit());
    assert!(!refused.ok, "{refused:?}");
    assert!(
        refused.summary.contains("with changes disabled"),
        "{refused:?}"
    );
    assert!(
        refused.summary.contains("supportCapability.set"),
        "{refused:?}"
    );
    assert!(refused.next.is_empty(), "{refused:?}");
    assert_eq!(
        unica_workspace::test_support::tree_snapshot(&fixture.source),
        before
    );
}
