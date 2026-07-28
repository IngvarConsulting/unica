#[cfg(unix)]
#[test]
fn code_intelligence_accepts_source_paths_reached_through_a_symlinked_workspace() {
    use serde_json::{Map, Value};
    use std::os::unix::fs::symlink;
    use unica_coder::application::UnicaApplication;

    let root = std::env::temp_dir().join(format!(
        "unica-code-intelligence-symlink-{}-{}",
        std::process::id(),
        uuid::Uuid::new_v4()
    ));
    let workspace = root.join("real");
    let module = workspace.join("src/CommonModules/Тест/Ext/Module.bsl");
    std::fs::create_dir_all(module.parent().unwrap()).unwrap();
    std::fs::write(&module, "Процедура Тест() КонецПроцедуры\n").unwrap();
    std::fs::write(
        workspace.join("v8project.yaml"),
        "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
    )
    .unwrap();

    // The caller stands in a symlink to the workspace. `resolve_source_root`
    // canonicalizes the source root, so unless cwd and workspace root are put in
    // the same identity class the module below reads as living outside the root.
    let linked_workspace = root.join("linked");
    symlink(&workspace, &linked_workspace).unwrap();

    let mut args = Map::new();
    args.insert(
        "cwd".to_string(),
        Value::String(linked_workspace.display().to_string()),
    );
    args.insert(
        "path".to_string(),
        Value::String("CommonModules/Тест/Ext/Module.bsl".to_string()),
    );

    let result = UnicaApplication::new().call_tool("unica.code.outline", &args);

    match result {
        Ok(outcome) => {
            // Without a built RLM index the call reports a typed index failure. The
            // point of the test is that it never fails on path containment.
            let rendered = format!("{} {:?}", outcome.summary, outcome.errors);
            assert!(
                !rendered.contains("outside resolved source root"),
                "{rendered}"
            );
        }
        Err(error) => {
            assert!(!error.contains("outside resolved source root"), "{error}");
        }
    }

    let _ = std::fs::remove_dir_all(&root);
}
