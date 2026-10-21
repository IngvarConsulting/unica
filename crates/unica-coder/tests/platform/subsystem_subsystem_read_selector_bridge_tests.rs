use super::*;
use serde_json::json;
use std::fs;
use std::sync::atomic::{AtomicU64, Ordering};

static NONCE: AtomicU64 = AtomicU64::new(0);

/// One registered top-level subsystem with a registered child, so both the
/// whole-tree and the single-node reads have something to answer.
fn workspace(name: &str) -> WorkspaceContext {
    let root = std::env::temp_dir().join(format!(
        "unica-subsystem-read-bridge-{name}-{}-{}",
        std::process::id(),
        NONCE.fetch_add(1, Ordering::Relaxed)
    ));
    let src = root.join("src");
    fs::create_dir_all(src.join("Subsystems/Parent/Subsystems")).unwrap();
    fs::write(
        root.join("v8project.yaml"),
        "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
    )
    .unwrap();
    fs::write(
            src.join("Configuration.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Configuration><Properties><Name>Demo</Name></Properties><ChildObjects><Subsystem>Parent</Subsystem></ChildObjects></Configuration></MetaDataObject>"#,
        )
        .unwrap();
    fs::write(
            src.join("Subsystems/Parent.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Subsystem uuid="aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa"><Properties><Name>Parent</Name><IncludeInCommandInterface>true</IncludeInCommandInterface><Content/></Properties><ChildObjects><Subsystem>Child</Subsystem></ChildObjects></Subsystem></MetaDataObject>"#,
        )
        .unwrap();
    fs::write(
            src.join("Subsystems/Parent/Subsystems/Child.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Subsystem uuid="bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb"><Properties><Name>Child</Name><IncludeInCommandInterface>true</IncludeInCommandInterface><Content/></Properties><ChildObjects/></Subsystem></MetaDataObject>"#,
        )
        .unwrap();
    // The topology walk compares a physical path against the configured
    // source roots. On macOS the temp directory is a symlink, so a
    // non-canonical workspace root makes that comparison fail for reasons
    // that have nothing to do with the tool under test.
    let physical_root = root.canonicalize().unwrap();
    WorkspaceContext {
        cwd: physical_root.clone(),
        workspace_root: physical_root.clone(),
        cache_root: physical_root.join(".build/unica"),
        workspace_epoch: 1,
    }
}

#[test]
pub(crate) fn subsystem_info_answers_identically_for_a_logical_and_a_physical_selector() {
    let context = workspace("info");

    let physical = analyze_subsystem_info(
        &Map::from_iter([(
            "SubsystemPath".to_string(),
            json!("src/Subsystems/Parent.xml"),
        )]),
        &context,
    );
    let logical = analyze_subsystem_info(
        &Map::from_iter([
            ("sourceSet".to_string(), json!("main")),
            ("metadataPath".to_string(), json!("Subsystem.Parent")),
        ]),
        &context,
    );

    assert!(logical.outcome.ok, "{:?}", logical.outcome);
    assert!(physical.outcome.ok, "{:?}", physical.outcome);
    assert_eq!(
        serde_json::to_value(&logical.data).unwrap(),
        serde_json::to_value(&physical.data).unwrap()
    );
    let _ = fs::remove_dir_all(&context.workspace_root);
}

/// `sourceSet` with no address is the source root, and the whole registered
/// tree is exactly what the `Subsystems` directory path answers today.
#[test]
fn subsystem_info_reads_the_whole_registered_tree_from_the_source_set_alone() {
    let context = workspace("root");

    let by_path = analyze_subsystem_info(
        &Map::from_iter([("SubsystemPath".to_string(), json!("src/Subsystems"))]),
        &context,
    );
    let by_set = analyze_subsystem_info(
        &Map::from_iter([("sourceSet".to_string(), json!("main"))]),
        &context,
    );

    assert!(by_set.outcome.ok, "{:?}", by_set.outcome);
    assert_eq!(
        serde_json::to_value(&by_set.data).unwrap(),
        serde_json::to_value(&by_path.data).unwrap()
    );
    let _ = fs::remove_dir_all(&context.workspace_root);
}

/// `metadataPath` is not type-checked as a string, so a null value reaches
/// the handler. The shared resolver reads it with `string_arg` and sees
/// nothing; a raw key-presence check here would see something, and the two
/// would disagree about which branch the call took.
#[test]
fn subsystem_info_treats_a_null_address_as_no_address() {
    let context = workspace("null-address");

    let by_set = analyze_subsystem_info(
        &Map::from_iter([("sourceSet".to_string(), json!("main"))]),
        &context,
    );
    let with_null = analyze_subsystem_info(
        &Map::from_iter([
            ("sourceSet".to_string(), json!("main")),
            ("metadataPath".to_string(), Value::Null),
        ]),
        &context,
    );

    assert!(with_null.outcome.ok, "{:?}", with_null.outcome);
    assert_eq!(
        serde_json::to_value(&with_null.data).unwrap(),
        serde_json::to_value(&by_set.data).unwrap()
    );
    let _ = fs::remove_dir_all(&context.workspace_root);
}

/// `contains_key` says a selector was sent; `string_arg` says whether it
/// can be used. An empty or non-string `sourceSet` satisfies the first and
/// not the second, and the gap between them was an `expect` — a panic on a
/// schema-valid call.
#[test]
fn subsystem_info_refuses_an_unusable_source_set_instead_of_panicking() {
    let context = workspace("unusable-source-set");

    for value in [json!(""), json!("   "), json!(7), Value::Null] {
        let outcome = analyze_subsystem_info(
            &Map::from_iter([("sourceSet".to_string(), value.clone())]),
            &context,
        )
        .outcome;

        assert!(!outcome.ok, "{value}: {outcome:?}");
        let text = outcome
            .errors
            .iter()
            .cloned()
            .chain(std::iter::once(outcome.summary.clone()))
            .collect::<Vec<_>>()
            .join(" | ");
        assert!(text.contains("source_set_unknown"), "{value}: {text}");
    }
    let _ = fs::remove_dir_all(&context.workspace_root);
}

/// ADR-0036 keeps the nested node outside `unica.source.*`, so the grammar
/// must refuse it rather than resolve a plausible-looking path.
#[test]
fn subsystem_info_refuses_a_nested_address_instead_of_guessing() {
    let context = workspace("nested");

    let outcome = analyze_subsystem_info(
        &Map::from_iter([
            ("sourceSet".to_string(), json!("main")),
            ("metadataPath".to_string(), json!("Subsystem.Parent.Child")),
        ]),
        &context,
    )
    .outcome;

    assert!(!outcome.ok);
    let text = outcome
        .errors
        .iter()
        .cloned()
        .chain(std::iter::once(outcome.summary.clone()))
        .collect::<Vec<_>>()
        .join(" | ");
    assert!(text.contains("target_kind_unsupported"), "{text}");
    let _ = fs::remove_dir_all(&context.workspace_root);
}

#[test]
pub(crate) fn subsystem_validate_answers_identically_for_a_logical_and_a_physical_selector() {
    let context = workspace("validate");

    let physical = validate_subsystem(
        &Map::from_iter([(
            "SubsystemPath".to_string(),
            json!("src/Subsystems/Parent.xml"),
        )]),
        &context,
    );
    let logical = validate_subsystem(
        &Map::from_iter([
            ("sourceSet".to_string(), json!("main")),
            ("metadataPath".to_string(), json!("Subsystem.Parent")),
        ]),
        &context,
    );

    assert_eq!(logical.ok, physical.ok, "{logical:?} vs {physical:?}");
    assert_eq!(logical.errors, physical.errors);
    let _ = fs::remove_dir_all(&context.workspace_root);
}
