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
            // The outline is proved from the current file (ADR-0020), so the point
            // of this test is only that it never fails on path containment.
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

#[cfg(unix)]
#[test]
fn code_intelligence_accepts_an_absolute_source_path_through_a_symlinked_workspace() {
    use serde_json::{Map, Value};
    use std::os::unix::fs::symlink;
    use unica_coder::application::UnicaApplication;

    let root = std::env::temp_dir().join(format!(
        "unica-code-intelligence-absolute-symlink-{}-{}",
        std::process::id(),
        uuid::Uuid::new_v4()
    ));
    let workspace = root.join("real");
    let relative_module = "src/CommonModules/Тест/Ext/Module.bsl";
    let module = workspace.join(relative_module);
    std::fs::create_dir_all(module.parent().unwrap()).unwrap();
    std::fs::write(&module, "Процедура Тест() КонецПроцедуры\n").unwrap();
    std::fs::write(
        workspace.join("v8project.yaml"),
        "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
    )
    .unwrap();

    let linked_workspace = root.join("linked");
    symlink(&workspace, &linked_workspace).unwrap();

    let mut args = Map::new();
    args.insert(
        "cwd".to_string(),
        Value::String(linked_workspace.display().to_string()),
    );
    args.insert(
        "path".to_string(),
        Value::String(linked_workspace.join(relative_module).display().to_string()),
    );

    let result = UnicaApplication::new().call_tool("unica.code.outline", &args);

    match result {
        Ok(outcome) => {
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

#[cfg(unix)]
#[test]
fn code_intelligence_rejects_a_source_path_through_a_symlink_outside_the_source_root() {
    use serde_json::{Map, Value};
    use std::os::unix::fs::symlink;
    use unica_coder::application::UnicaApplication;

    let root = std::env::temp_dir().join(format!(
        "unica-code-intelligence-symlink-escape-{}-{}",
        std::process::id(),
        uuid::Uuid::new_v4()
    ));
    let workspace = root.join("workspace");
    let source_root = workspace.join("src");
    let outside = root.join("outside");
    std::fs::create_dir_all(&source_root).unwrap();
    std::fs::create_dir_all(&outside).unwrap();
    std::fs::write(
        workspace.join("v8project.yaml"),
        "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
    )
    .unwrap();
    std::fs::write(
        outside.join("Module.bsl"),
        "Процедура Снаружи() КонецПроцедуры\n",
    )
    .unwrap();
    symlink(&outside, source_root.join("escape")).unwrap();

    let mut args = Map::new();
    args.insert(
        "cwd".to_string(),
        Value::String(workspace.display().to_string()),
    );
    args.insert(
        "path".to_string(),
        Value::String("escape/Module.bsl".to_string()),
    );

    let error = UnicaApplication::new()
        .call_tool("unica.code.outline", &args)
        .expect_err("a source path through an escaping symlink must be rejected");

    assert!(error.contains("outside resolved source root"), "{error}");

    let _ = std::fs::remove_dir_all(&root);
}

fn current_source_outline_fixture(
    label: &str,
) -> (
    std::path::PathBuf,
    std::path::PathBuf,
    serde_json::Map<String, serde_json::Value>,
) {
    use serde_json::Value;

    let root = std::env::temp_dir().join(format!(
        "unica-code-outline-{label}-{}-{}",
        std::process::id(),
        uuid::Uuid::new_v4()
    ));
    let workspace = root.join("workspace");
    let module = workspace.join("src/CommonModules/Демо/Ext/Module.bsl");
    std::fs::create_dir_all(module.parent().unwrap()).unwrap();
    std::fs::write(
        workspace.join("v8project.yaml"),
        "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
    )
    .unwrap();
    // The BSP header shape that the shipped index reads as a real exported
    // method while losing the real one. The real declaration also proves that
    // source spelling and line wrapping do not leak into the public contract.
    std::fs::write(
        &module,
        concat!(
            "\u{feff}#Область ПрограммныйИнтерфейс\n",
            "\n",
            "// Пример вызова:\n",
            "// Процедура ОпределитьНастройки(Форма) Экспорт\n",
            "// КонецПроцедуры\n",
            "пРоЦеДуРа НастроитьВарианты(\n",
            "    Знач\n",
            "    Настройки,\n",
            "    Необязательный =\n",
            "        1 + 2) Экспорт\n",
            "\tВозврат;\n",
            "КонецПроцедуры\n",
            "\n",
            "#КонецОбласти\n",
        ),
    )
    .unwrap();

    let mut args = serde_json::Map::new();
    args.insert(
        "cwd".to_string(),
        Value::String(workspace.display().to_string()),
    );
    args.insert(
        "path".to_string(),
        Value::String("CommonModules/Демо/Ext/Module.bsl".to_string()),
    );
    (root, workspace, args)
}

/// ADR-0021: the public envelope of `unica.code.outline` describes the file on
/// disk as typed data and claims no cache. Nothing here builds an index, so a
/// passing outline also proves the tool never needed one.
#[test]
fn code_outline_answers_from_the_current_file_without_touching_the_index() {
    use serde_json::json;
    use unica_coder::application::UnicaApplication;

    let (root, workspace, args) = current_source_outline_fixture("typed-data");

    let outcome = UnicaApplication::new()
        .call_tool("unica.code.outline", &args)
        .expect("the outline is proved from the current file");

    assert!(outcome.ok, "{:?}", outcome.errors);
    assert!(outcome.errors.is_empty(), "{:?}", outcome.errors);
    assert!(
        outcome.stdout.is_none(),
        "typed outline must not be duplicated in stdout: {:?}",
        outcome.stdout
    );
    assert_eq!(
        outcome.data,
        Some(json!({
            "module": "CommonModules/Демо/Ext/Module.bsl",
            "identity": {
                "category": "CommonModules",
                "object": "Демо",
                "moduleType": "Module"
            },
            "totals": {
                "methods": 1,
                "exports": 1,
                "regions": 1,
                "loc": 7
            },
            "regions": [{
                "name": "ПрограммныйИнтерфейс",
                "line": 1,
                "endLine": 14,
                "regions": [],
                "methods": [{
                    "name": "НастроитьВарианты",
                    "kind": "procedure",
                    "parameters": [
                        {
                            "name": "Настройки",
                            "byValue": true,
                            "defaultValue": null
                        },
                        {
                            "name": "Необязательный",
                            "byValue": false,
                            "defaultValue": "1 + 2"
                        }
                    ],
                    "export": true,
                    "line": 6,
                    "endLine": 12
                }]
            }],
            "methods": []
        }))
    );
    assert!(
        !outcome.cache.fresh.iter().any(|name| name == "bsl_index"),
        "the outline must not claim the index fresh: {:?}",
        outcome.cache
    );
    assert!(
        !workspace.join(".build/unica/rlm-tools-bsl").exists(),
        "the outline must not create index state"
    );

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn code_outline_without_methods_keeps_totals_in_typed_data() {
    use serde_json::{json, Value};
    use unica_coder::application::UnicaApplication;

    let (root, _workspace, mut args) =
        current_source_outline_fixture("typed-data-without-methods");
    args.insert("includeMethods".to_string(), Value::Bool(false));

    let outcome = UnicaApplication::new()
        .call_tool("unica.code.outline", &args)
        .expect("the outline is proved from the current file");

    assert!(outcome.ok, "{:?}", outcome.errors);
    assert!(outcome.stdout.is_none(), "{:?}", outcome.stdout);
    let data = outcome.data.expect("typed outline data");
    assert_eq!(data["totals"]["methods"], json!(1));
    assert_eq!(data["totals"]["exports"], json!(1));
    assert_eq!(data["regions"][0]["methods"], json!([]));
    assert_eq!(data["methods"], json!([]));

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn code_outline_failure_publishes_neither_typed_nor_text_partial_result() {
    use unica_coder::application::UnicaApplication;

    let (root, workspace, args) = current_source_outline_fixture("typed-data-failure");
    std::fs::write(
        workspace.join("src/CommonModules/Демо/Ext/Module.bsl"),
        "Процедура Сломана(\nКонецПроцедуры\nЕсли Тогда\n",
    )
    .unwrap();

    let outcome = UnicaApplication::new()
        .call_tool("unica.code.outline", &args)
        .expect("a proved provider failure uses the public result envelope");

    assert!(!outcome.ok);
    assert!(outcome.data.is_none(), "{:?}", outcome.data);
    assert!(outcome.stdout.is_none(), "{:?}", outcome.stdout);
    assert!(
        outcome.errors.iter().any(|error| error.contains("parser reported")),
        "{:?}",
        outcome.errors
    );
    assert!(
        !workspace.join(".build/unica").exists(),
        "a failed outline must not create workspace state"
    );

    let _ = std::fs::remove_dir_all(&root);
}
