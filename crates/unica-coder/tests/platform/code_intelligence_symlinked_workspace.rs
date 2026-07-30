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

/// ADR-0020: the public envelope of `unica.code.outline` describes the file on
/// disk and claims no cache. Nothing here builds an index, so a passing outline
/// also proves the tool never needed one.
#[test]
fn code_outline_answers_from_the_current_file_without_touching_the_index() {
    use serde_json::{Map, Value};
    use unica_coder::application::UnicaApplication;

    let root = std::env::temp_dir().join(format!(
        "unica-code-outline-current-source-{}-{}",
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
    // method while losing the real one.
    std::fs::write(
        &module,
        concat!(
            "\u{feff}#Область ПрограммныйИнтерфейс\n",
            "\n",
            "// Пример вызова:\n",
            "// Процедура ОпределитьНастройки(Форма) Экспорт\n",
            "// КонецПроцедуры\n",
            "Процедура НастроитьВарианты(Настройки) Экспорт\n",
            "\tВозврат;\n",
            "КонецПроцедуры\n",
            "\n",
            "#КонецОбласти\n",
        ),
    )
    .unwrap();

    let mut args = Map::new();
    args.insert(
        "cwd".to_string(),
        Value::String(workspace.display().to_string()),
    );
    args.insert(
        "path".to_string(),
        Value::String("CommonModules/Демо/Ext/Module.bsl".to_string()),
    );

    let outcome = UnicaApplication::new()
        .call_tool("unica.code.outline", &args)
        .expect("the outline is proved from the current file");

    assert!(outcome.ok, "{:?}", outcome.errors);
    assert!(outcome.errors.is_empty(), "{:?}", outcome.errors);
    let stdout = outcome.stdout.clone().expect("outline stdout");
    assert_eq!(
        stdout,
        concat!(
            "=== bsl-outline ===\n",
            "module: CommonModules/Демо/Ext/Module.bsl\n",
            "object: Демо\n",
            "category: CommonModules\n",
            "moduleType: Module\n",
            "totals: methods=1 exports=1 regions=1 loc=3\n",
            "region ПрограммныйИнтерфейс: 1-10\n",
            "  Процедура НастроитьВарианты(Настройки) export at 6-8"
        )
    );
    assert!(
        !stdout.contains("ОпределитьНастройки"),
        "a commented-out declaration reached the public outline:\n{stdout}"
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
