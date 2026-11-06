//! Saved public apply plans must retain the native planner's publication invariants.
use super::actor_capacity_tests::direct_v5;
use super::*;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use unica_execution::invocation_store::ToolIdentity;
use unica_workspace::compile_transaction::take_retained_apply_observed_events;
use unica_workspace::test_support::path_identity_for_test;

type WorkspaceImage = BTreeMap<PathBuf, (Option<Vec<u8>>, Option<String>)>;

fn workspace_image(root: &Path) -> WorkspaceImage {
    let mut image = BTreeMap::new();
    let mut pending = vec![root.to_path_buf()];
    while let Some(path) = pending.pop() {
        let metadata = std::fs::symlink_metadata(&path).unwrap();
        assert!(
            !metadata.file_type().is_symlink(),
            "fixture symlink: {path:?}"
        );
        let bytes = if metadata.is_dir() {
            pending.extend(
                std::fs::read_dir(&path)
                    .unwrap()
                    .map(|entry| entry.unwrap().path()),
            );
            None
        } else {
            Some(std::fs::read(&path).unwrap())
        };
        image.insert(
            path.strip_prefix(root).unwrap().to_path_buf(),
            (bytes, path_identity_for_test(&path).unwrap()),
        );
    }
    image
}

/// rule: INV.SOURCE.BOUND-PREIMAGES
/// rule: INV.WIRE.APPLY-REQUIRES-THE-FENCE
/// rule: INV.APP.REQUEST-LEVEL-APPLY-EFFECT-RECONCILIATION
#[test]
fn fresh_public_noop_token_and_stale_plan_preserve_source_cache_state_and_file_identity() {
    let workspace = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(workspace.path()).unwrap();
    let source = root.join("src");
    std::fs::create_dir_all(source.join("Documents")).unwrap();
    std::fs::create_dir_all(source.join("Languages")).unwrap();
    std::fs::write(
        root.join("v8project.yaml"),
        "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
    )
    .unwrap();
    std::fs::write(source.join("Configuration.xml"), r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Configuration><Properties><Name>Main</Name><DefaultLanguage>Language.Русский</DefaultLanguage></Properties><ChildObjects><Language>Русский</Language><Document>Order</Document></ChildObjects></Configuration></MetaDataObject>"#).unwrap();
    std::fs::write(source.join("Languages/Русский.xml"), r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Language uuid="33333333-3333-4333-8333-333333333333"><Properties><Name>Русский</Name><Comment/><LanguageCode>ru</LanguageCode></Properties></Language></MetaDataObject>"#).unwrap();
    let descriptor = source.join("Documents/Order.xml");
    std::fs::write(&descriptor, r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Document uuid="11111111-1111-4111-8111-111111111111"><Properties><Name>Order</Name><Synonym/><Comment/></Properties><ChildObjects/></Document></MetaDataObject>"#).unwrap();
    std::fs::write(source.join("opaque.bin"), b"\0\xffuntouched\r\n").unwrap();

    let runtime = V5CanonicalInvocationRuntime::new(
        Arc::new(crate::infrastructure::daemon::v13_service::CanonicalV13ReadService::default()),
        Arc::new(TokioClock),
    );
    let call = |arguments: Value| {
        direct_v5(
            &runtime,
            InvocationRequest::new(
                ToolIdentity::Apply,
                arguments,
                root.to_string_lossy(),
                7_000,
            )
            .unwrap(),
        )
        .unwrap()
    };
    let preview = |comment: &str| {
        let result = call(
            json!({"at":"main:Document.Order","ops":[{"op":"props.set","args":{"values":{"Comment":comment}}}]}),
        );
        assert!(result.ok, "preview: {result:?}");
        assert_eq!(result.data.as_ref().unwrap()["mode"], "preview");
        result.data.as_ref().unwrap()["executionToken"]
            .as_str()
            .unwrap()
            .to_string()
    };

    let original = std::fs::read(&descriptor).unwrap();
    let first_token = preview("First publication");
    take_retained_apply_observed_events();
    let first = call(json!({"executionToken":first_token}));
    assert!(first.ok, "first execute: {first:?}");
    assert_eq!(first.data.as_ref().unwrap()["mode"], "published");
    assert!(first.data.as_ref().unwrap()["effects"].as_u64().unwrap() > 0);
    assert!(
        !first.changed.is_empty(),
        "first execute must change the source: {first:?}"
    );
    assert!(
        !take_retained_apply_observed_events().is_empty(),
        "first execute must physically publish"
    );
    let published = std::fs::read(&descriptor).unwrap();
    assert_ne!(published, original);
    let document = roxmltree::Document::parse(
        std::str::from_utf8(&published)
            .unwrap()
            .trim_start_matches('\u{feff}'),
    )
    .unwrap();
    assert_eq!(
        document
            .descendants()
            .find(|node| node.has_tag_name(("http://v8.1c.ru/8.3/MDClasses", "Comment")))
            .unwrap()
            .text(),
        Some("First publication")
    );

    // This is a newly planned equivalent operation, never replay of the first token.
    let before_noop = workspace_image(&root);
    let noop_token = preview("First publication");
    assert_ne!(noop_token, first_token);
    assert_eq!(
        workspace_image(&root),
        before_noop,
        "noop preview must not write source/cache/state"
    );
    take_retained_apply_observed_events();
    let noop = call(json!({"executionToken":noop_token}));
    assert!(noop.ok, "new noop token execute: {noop:?}");
    assert_eq!(noop.data.as_ref().unwrap()["mode"], "published");
    assert_eq!(noop.data.as_ref().unwrap()["effects"], 0);
    assert!(noop.changed.is_empty(), "{noop:?}");
    assert!(
        take_retained_apply_observed_events().is_empty(),
        "noop entered physical publication"
    );
    assert_eq!(
        workspace_image(&root),
        before_noop,
        "noop changed source/cache/state bytes or identities"
    );

    let pending_token = preview("Must not publish");
    assert_ne!(pending_token, noop_token);
    let mut foreign = std::fs::read(&descriptor).unwrap();
    foreign.extend_from_slice(b"<!-- concurrent descriptor input -->");
    std::fs::write(&descriptor, &foreign).unwrap();
    let concurrent = workspace_image(&root);
    take_retained_apply_observed_events();
    let stale = call(json!({"executionToken":pending_token}));
    assert!(
        !stale.ok,
        "changed real input accepted stale saved plan: {stale:?}"
    );
    assert_eq!(stale.diagnostics[0]["code"], "stale_revision");
    assert!(stale.changed.is_empty(), "{stale:?}");
    assert!(
        take_retained_apply_observed_events().is_empty(),
        "stale plan entered physical publication"
    );
    assert_eq!(std::fs::read(&descriptor).unwrap(), foreign);
    assert_eq!(
        workspace_image(&root),
        concurrent,
        "stale refusal lost foreign bytes/identities or published partial source/cache/state"
    );
}
