use super::*;

fn fixture() -> tempfile::TempDir {
    let root = tempfile::tempdir().unwrap();
    copy_tree(
        &std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/acceptance/workspace-metadata-commands"),
        root.path(),
    );
    root
}

fn apply(mcp: &mut McpProcess, id: u64, ops: Value) -> Value {
    mcp.completed_tool_call(call_tool(
        id,
        "unica.apply",
        json!({"at":"main:Catalog.Items", "ops":ops}),
    ))
}

fn execute(mcp: &mut McpProcess, id: u64, plan: &Value) -> Value {
    assert_eq!(plan["ok"], true, "{plan:#}");
    mcp.completed_tool_call(call_tool(
        id,
        "unica.apply",
        json!({"executionToken":plan["data"]["executionToken"]}),
    ))
}

fn assert_bsl_index_invalidated(result: &Value) {
    assert_eq!(result["ok"], true, "{result:#}");
    assert!(
        result["data"]["cache"]["invalidated"]
            .as_array()
            .unwrap()
            .iter()
            .any(|name| name == "bsl_index"),
        "{result:#}"
    );
}

fn exact_module_route(root: &std::path::Path, relative: &str) -> bool {
    let mut parent = root.to_path_buf();
    for component in std::path::Path::new(relative).components() {
        if !std::fs::read_dir(&parent)
            .unwrap()
            .any(|entry| entry.unwrap().file_name() == component.as_os_str())
        {
            return false;
        }
        parent.push(component);
    }
    true
}

fn source_snapshot(
    root: &std::path::Path,
) -> std::collections::BTreeMap<std::path::PathBuf, Vec<u8>> {
    fn visit(
        root: &std::path::Path,
        path: &std::path::Path,
        result: &mut std::collections::BTreeMap<std::path::PathBuf, Vec<u8>>,
    ) {
        for entry in std::fs::read_dir(path).unwrap() {
            let entry = entry.unwrap();
            if entry.file_type().unwrap().is_dir() {
                visit(root, &entry.path(), result);
            } else {
                result.insert(
                    entry.path().strip_prefix(root).unwrap().to_path_buf(),
                    std::fs::read(entry.path()).unwrap(),
                );
            }
        }
    }
    let mut result = std::collections::BTreeMap::new();
    visit(root, root, &mut result);
    result
}

#[test]
fn public_command_lifecycle_never_publishes_a_module_under_a_case_alias() {
    for case in [
        "direct",
        "old-directory",
        "orphan-directory",
        "ext-directory",
    ] {
        let root = fixture();
        let source = root.path().join("src");
        let commands = source.join("Catalogs/Items/Commands");
        let case_insensitive = commands.join("refresh").exists();
        let mut mcp = McpProcess::start(root.path());
        initialize_body_mcp(&mut mcp);
        let mut old_name = "Refresh";
        if case == "old-directory" {
            let plan = apply(
                &mut mcp,
                2,
                json!([{"op":"command.set", "args":{
                    "at":"main:Catalog.Items.Command.Refresh", "values":{"newName":"Temp"}
                }}]),
            );
            assert_bsl_index_invalidated(&execute(&mut mcp, 3, &plan));
            old_name = "Temp";
        }
        let new_name = match case {
            "orphan-directory" => {
                std::fs::create_dir_all(commands.join("updated/Ext")).unwrap();
                std::fs::write(commands.join("updated/Ext/unknown.bin"), b"foreign\0\xff").unwrap();
                "Updated"
            }
            "ext-directory" => {
                std::fs::create_dir_all(commands.join("Renamed/ext")).unwrap();
                std::fs::write(commands.join("Renamed/ext/unknown.bin"), b"foreign\0\xff").unwrap();
                "Renamed"
            }
            _ => "refresh",
        };
        let before = source_snapshot(&source);
        let old_module = commands.join(old_name).join("Ext/CommandModule.bsl");
        let module_bytes = std::fs::read(&old_module).unwrap();
        let selector = if case == "direct" {
            "refresh"
        } else {
            old_name
        };
        let plan = apply(
            &mut mcp,
            4,
            json!([{"op":"command.set", "args":{
                "at":format!("main:Catalog.Items.Command.{selector}"), "values":{"newName":new_name, "comment":"updated"}
            }}]),
        );
        let result = if plan["ok"] == true {
            execute(&mut mcp, 5, &plan)
        } else {
            plan
        };
        if case_insensitive {
            assert_eq!(result["ok"], false, "{case}: {result:#}");
            assert_eq!(source_snapshot(&source), before, "{case}");
        } else {
            assert_bsl_index_invalidated(&result);
            let relative = format!("Catalogs/Items/Commands/{new_name}/Ext/CommandModule.bsl");
            assert!(exact_module_route(&source, &relative), "{case}: {relative}");
            assert_eq!(std::fs::read(source.join(relative)).unwrap(), module_bytes);
            assert!(!old_module.exists());
            for (relative, bytes) in before {
                if relative != std::path::Path::new("Catalogs/Items.xml")
                    && source.join(&relative) != old_module
                {
                    assert_eq!(
                        std::fs::read(source.join(relative)).unwrap(),
                        bytes,
                        "{case}"
                    );
                }
            }
        }
        mcp.finish();
    }
}

#[test]
fn public_command_lifecycle_saved_plan_refuses_changed_destination_spelling() {
    let root = fixture();
    let source = root.path().join("src");
    let destination = source.join("Catalogs/Items/Commands/Renamed/Ext");
    std::fs::create_dir_all(&destination).unwrap();
    std::fs::write(destination.join("unknown.bin"), b"neighbor").unwrap();
    let mut mcp = McpProcess::start(root.path());
    initialize_body_mcp(&mut mcp);
    let plan = apply(
        &mut mcp,
        2,
        json!([{"op":"command.set", "args":{
            "at":"main:Catalog.Items.Command.Refresh", "values":{"newName":"Renamed"}
        }}]),
    );
    assert_eq!(plan["ok"], true, "{plan:#}");
    std::fs::rename(&destination, destination.with_file_name("ext")).unwrap();
    let foreign_state = source_snapshot(&source);

    let refused = execute(&mut mcp, 3, &plan);

    assert_eq!(refused["ok"], false, "{refused:#}");
    assert_eq!(source_snapshot(&source), foreign_state);
    mcp.finish();
}

#[cfg(unix)]
#[test]
fn public_command_lifecycle_saved_plan_ignores_unrelated_neighbor_links() {
    let root = fixture();
    let source = root.path().join("src");
    let destination = source.join("Catalogs/Items/Commands/Renamed/Ext");
    std::fs::create_dir_all(&destination).unwrap();
    let outside = root.path().join("outside.bin");
    std::fs::write(&outside, b"foreign\0\xff").unwrap();
    let mut mcp = McpProcess::start(root.path());
    initialize_body_mcp(&mut mcp);
    let plan = apply(
        &mut mcp,
        2,
        json!([{"op":"command.set", "args":{
            "at":"main:Catalog.Items.Command.Refresh", "values":{"newName":"Renamed"}
        }}]),
    );
    assert_eq!(plan["ok"], true, "{plan:#}");
    let link = destination.join("unrelated-link");
    std::os::unix::fs::symlink(&outside, &link).unwrap();

    let result = execute(&mut mcp, 3, &plan);

    assert_bsl_index_invalidated(&result);
    assert_eq!(std::fs::read_link(&link).unwrap(), outside);
    assert_eq!(std::fs::read(&outside).unwrap(), b"foreign\0\xff");
    assert!(exact_module_route(
        &source,
        "Catalogs/Items/Commands/Renamed/Ext/CommandModule.bsl"
    ));
    mcp.finish();
}

#[test]
fn public_command_lifecycle_preserves_bytes_and_allows_reusing_its_old_directory() {
    let root = fixture();
    let source = root.path().join("src");
    let old = source.join("Catalogs/Items/Commands/Refresh/Ext/CommandModule.bsl");
    let new = source.join("Catalogs/Items/Commands/Renamed/Ext/CommandModule.bsl");
    let neighbor = source.join("Catalogs/Items/Commands/Keep/Ext/CommandModule.bsl");
    let opaque = source.join("Catalogs/Items/Commands/Refresh/Ext/unrelated.txt");
    std::fs::write(
        &old,
        "// Refresh module bytes must survive\r\nProcedure Refresh() Export\r\nEndProcedure\r\n",
    )
    .unwrap();
    let old_bytes = std::fs::read(&old).unwrap();
    let neighbor_bytes = std::fs::read(&neighbor).unwrap();
    let opaque_bytes = std::fs::read(&opaque).unwrap();
    let mut mcp = McpProcess::start(root.path());
    initialize_body_mcp(&mut mcp);
    let plan = apply(
        &mut mcp,
        2,
        json!([
            {"op":"command.set", "args":{
                "at":"main:Catalog.Items.Command.Refresh",
                "values":{"newName":"Renamed"}
            }}
        ]),
    );
    assert_eq!(std::fs::read(&old).unwrap(), old_bytes);
    assert!(!new.exists());
    let result = execute(&mut mcp, 3, &plan);
    assert_bsl_index_invalidated(&result);
    assert!(!old.exists());
    let moved = std::fs::read(&new).unwrap();
    assert_eq!(moved, old_bytes);

    let back = apply(
        &mut mcp,
        4,
        json!([{ "op":"command.set", "args":{
            "at":"main:Catalog.Items.Command.Renamed", "values":{"newName":"Refresh"}
        }}]),
    );
    let result = execute(&mut mcp, 5, &back);
    assert_bsl_index_invalidated(&result);
    assert_eq!(std::fs::read(&old).unwrap(), moved);
    assert!(!new.exists());

    let remove = apply(
        &mut mcp,
        6,
        json!([{ "op":"command.remove", "args":{
            "at":"main:Catalog.Items.Command.Refresh"
        }}]),
    );
    let result = execute(&mut mcp, 7, &remove);
    assert_bsl_index_invalidated(&result);
    assert!(!old.exists());
    assert_eq!(std::fs::read(neighbor).unwrap(), neighbor_bytes);
    assert_eq!(std::fs::read(opaque).unwrap(), opaque_bytes);
    mcp.finish();
}

#[test]
fn public_command_lifecycle_saved_plan_preserves_concurrent_source_and_destination_bytes() {
    for changed in ["source", "destination"] {
        let root = fixture();
        let source = root.path().join("src");
        let descriptor = source.join("Catalogs/Items.xml");
        let old = source.join("Catalogs/Items/Commands/Refresh/Ext/CommandModule.bsl");
        let new = source.join("Catalogs/Items/Commands/Renamed/Ext/CommandModule.bsl");
        let owner_before = std::fs::read(&descriptor).unwrap();
        let module_before = std::fs::read(&old).unwrap();
        let mut mcp = McpProcess::start(root.path());
        initialize_body_mcp(&mut mcp);
        let plan = apply(
            &mut mcp,
            2,
            json!([{ "op":"command.set", "args":{
                "at":"main:Catalog.Items.Command.Refresh", "values":{"newName":"Renamed"}
            }}]),
        );
        assert_eq!(plan["ok"], true, "{plan:#}");
        let raced = if changed == "source" { &old } else { &new };
        std::fs::create_dir_all(raced.parent().unwrap()).unwrap();
        std::fs::write(raced, b"// concurrent owner bytes\r\n").unwrap();

        let refused = execute(&mut mcp, 3, &plan);

        assert_eq!(refused["ok"], false, "{changed}: {refused:#}");
        assert_eq!(std::fs::read(&descriptor).unwrap(), owner_before);
        assert_eq!(
            std::fs::read(raced).unwrap(),
            b"// concurrent owner bytes\r\n"
        );
        if changed == "source" {
            assert!(!new.exists());
        } else {
            assert_eq!(std::fs::read(&old).unwrap(), module_before);
        }
        mcp.finish();
    }
}
