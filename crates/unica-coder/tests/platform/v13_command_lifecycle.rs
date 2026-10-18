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
