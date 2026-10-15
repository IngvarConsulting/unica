use super::*;

fn resource_resolve_fixture() -> tempfile::TempDir {
    let root = tempfile::tempdir().expect("resource resolve workspace");
    copy_tree(
        &std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/acceptance/workspace-resolve-resources"),
        root.path(),
    );
    root
}

fn resource_resolve_mcp(workspace: &std::path::Path) -> McpProcess {
    let mut mcp = McpProcess::start(workspace);
    let initialized = mcp.exchange(json!({
        "jsonrpc":"2.0", "id":1, "method":"initialize",
        "params":{"protocolVersion":"2025-06-18", "capabilities":{},
            "clientInfo":{"name":"resource-resolve", "version":"1"}}
    }));
    assert_eq!(initialized["result"]["serverInfo"]["name"], "unica");
    mcp.notify(json!({"jsonrpc":"2.0", "method":"notifications/initialized", "params":{}}));
    mcp
}

#[test]
fn resolve_form_and_module_paths_preserve_the_exact_resource_on_public_mcp() {
    for source_kind in ["CONFIGURATION", "EXTENSION"] {
        let root = resource_resolve_fixture();
        let manifest = root.path().join("v8project.yaml");
        let text = std::fs::read_to_string(&manifest)
            .unwrap()
            .replace("CONFIGURATION", source_kind);
        std::fs::write(manifest, text).unwrap();
        let mut mcp = resource_resolve_mcp(root.path());
        let mut id = 2;
        for (kind, name) in [("DataProcessor", "Processor"), ("Document", "Invoice")] {
            for (resource, suffix, expected_kind) in [
                ("Forms/Main/Ext/Form.xml", ".Form.Main", "Form"),
                (
                    "Forms/Main/Ext/Form/Module.bsl",
                    ".Form.Main.Module.Form",
                    "Module",
                ),
                ("Ext/ManagerModule.bsl", ".Module.Manager", "Module"),
            ] {
                let path = format!("{kind}s/{name}/{resource}");
                let address = format!("main:{kind}.{name}{suffix}");
                for query in [
                    path.clone(),
                    format!("src/{path}"),
                    std::fs::canonicalize(root.path().join("src").join(&path))
                        .unwrap()
                        .to_str()
                        .unwrap()
                        .to_string(),
                ] {
                    let result = mcp.completed_tool_call(call_tool(
                        id,
                        "unica.resolve",
                        json!({"path":query}),
                    ));
                    id += 1;
                    assert_eq!(result["ok"], true, "{query}: {result:#}");
                    assert_eq!(result["data"]["at"], address);
                    assert_eq!(result["data"]["path"], path);
                    assert_eq!(result["data"]["kind"], expected_kind);
                    assert_eq!(result["data"]["lines"]["state"], "notLineBased");
                    let viewed =
                        mcp.completed_tool_call(call_tool(id, "unica.view", json!({"at":address})));
                    id += 1;
                    assert_eq!(viewed["ok"], true, "{viewed:#}");
                }
            }
        }
        let descriptor = mcp.completed_tool_call(call_tool(
            id,
            "unica.resolve",
            json!({"at":"main:DataProcessor.Processor.Form.Main"}),
        ));
        assert_eq!(descriptor["ok"], true, "{descriptor:#}");
        assert_eq!(
            descriptor["data"]["path"],
            "DataProcessors/Processor/Forms/Main.xml"
        );
        mcp.finish();
    }
}

#[test]
fn resolve_resources_refuse_unregistered_and_absent_targets_on_public_mcp() {
    let root = resource_resolve_fixture();
    std::fs::remove_file(
        root.path()
            .join("src/Documents/Invoice/Ext/ManagerModule.bsl"),
    )
    .unwrap();
    let mut mcp = resource_resolve_mcp(root.path());
    for (index, path) in [
        "DataProcessors/Orphan/Ext/ManagerModule.bsl",
        "DataProcessors/Orphan/Forms/Main/Ext/Form.xml",
        "DataProcessors/Processor/Forms/Ghost/Ext/Form.xml",
        "DataProcessors/Processor/Forms/Ghost/Ext/Form/Module.bsl",
        "Documents/Invoice/Ext/ManagerModule.bsl",
        "DataProcessors/Absent/Ext/ManagerModule.bsl",
    ]
    .iter()
    .enumerate()
    {
        let result = mcp.completed_tool_call(call_tool(
            index as u64 + 2,
            "unica.resolve",
            json!({"path":path}),
        ));
        assert_eq!(result["ok"], false, "{path}: {result:#}");
        assert_eq!(
            result["diagnostics"][0]["code"], "not_found",
            "{path}: {result:#}"
        );
    }
    mcp.finish();
}

#[test]
fn resolve_resources_keep_source_prefix_and_deepest_root_selection_on_public_mcp() {
    let root = resource_resolve_fixture();
    copy_tree(&root.path().join("src"), root.path());
    std::fs::write(root.path().join("v8project.yaml"), "format: DESIGNER\nsource-set:\n  - name: outer\n    type: CONFIGURATION\n    path: .\n  - name: inner\n    type: CONFIGURATION\n    path: src\n").unwrap();
    let mut mcp = resource_resolve_mcp(root.path());
    let path = "DataProcessors/Processor/Forms/Main/Ext/Form/Module.bsl";
    for (index, query) in [
        format!("src/{path}"),
        std::fs::canonicalize(root.path().join("src").join(path))
            .unwrap()
            .to_str()
            .unwrap()
            .to_string(),
    ]
    .iter()
    .enumerate()
    {
        let result = mcp.completed_tool_call(call_tool(
            index as u64 + 2,
            "unica.resolve",
            json!({"path":query}),
        ));
        assert_eq!(result["ok"], true, "{result:#}");
        assert_eq!(
            result["data"]["at"],
            "inner:DataProcessor.Processor.Form.Main.Module.Form"
        );
        assert_eq!(result["data"]["path"], path);
    }
    let ambiguous = mcp.completed_tool_call(call_tool(5, "unica.resolve", json!({"path":path})));
    assert_eq!(ambiguous["ok"], false, "{ambiguous:#}");
    assert_eq!(ambiguous["diagnostics"][0]["code"], "bad_value");
    // An unregistered physical double must still refuse ambiguity, rather than
    // selecting an arbitrary source before typed registration is established.
    let root_descriptor = root.path().join("Configuration.xml");
    let text = std::fs::read_to_string(&root_descriptor)
        .unwrap()
        .replace("<DataProcessor>Processor</DataProcessor>", "");
    std::fs::write(root_descriptor, text).unwrap();
    let ambiguous = mcp.completed_tool_call(call_tool(6, "unica.resolve", json!({"path":path})));
    assert_eq!(ambiguous["ok"], false, "{ambiguous:#}");
    assert_eq!(ambiguous["diagnostics"][0]["code"], "bad_value");
    mcp.finish();
}

#[cfg(unix)]
#[test]
fn resolve_resources_do_not_follow_linked_files_or_directories_on_public_mcp() {
    use std::os::unix::fs::symlink;
    for relative in [
        "DataProcessors/Processor/Forms/Main/Ext/Form/Module.bsl",
        "DataProcessors/Processor/Forms/Main/Ext",
        "DataProcessors/Processor/Forms/Main",
        "DataProcessors/Processor/Forms/Main.xml",
    ] {
        let root = resource_resolve_fixture();
        let target = root.path().join("src").join(relative);
        let foreign = root.path().join("foreign");
        std::fs::rename(&target, &foreign).unwrap();
        symlink(&foreign, &target).unwrap();
        let mut mcp = resource_resolve_mcp(root.path());
        let result = mcp.completed_tool_call(call_tool(
            2,
            "unica.resolve",
            json!({"path":"src/DataProcessors/Processor/Forms/Main/Ext/Form/Module.bsl"}),
        ));
        assert_eq!(result["ok"], false, "{relative}: {result:#}");
        assert!(
            ["invalid_source", "provider_unavailable"]
                .contains(&result["diagnostics"][0]["code"].as_str().unwrap()),
            "{relative}: {result:#}"
        );
        mcp.finish();
    }
}

#[cfg(unix)]
#[test]
fn resolve_resource_prefix_skips_a_foreign_linked_collection_on_public_mcp() {
    use std::os::unix::fs::symlink;
    let root = resource_resolve_fixture();
    let other = root.path().join("other");
    std::fs::create_dir_all(&other).unwrap();
    std::fs::copy(
        root.path().join("src/Configuration.xml"),
        other.join("Configuration.xml"),
    )
    .unwrap();
    symlink(
        root.path().join("src/DataProcessors"),
        other.join("DataProcessors"),
    )
    .unwrap();
    std::fs::write(root.path().join("v8project.yaml"), "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n  - name: foreign\n    type: CONFIGURATION\n    path: other\n").unwrap();
    let mut mcp = resource_resolve_mcp(root.path());
    let result = mcp.completed_tool_call(call_tool(
        2,
        "unica.resolve",
        json!({"path":"src/DataProcessors/Processor/Forms/Main/Ext/Form/Module.bsl"}),
    ));
    assert_eq!(result["ok"], true, "{result:#}");
    assert_eq!(
        result["data"]["at"],
        "main:DataProcessor.Processor.Form.Main.Module.Form"
    );
    mcp.finish();
}

#[test]
fn resolve_resource_refuses_mismatched_descriptor_identity_on_public_mcp() {
    for (relative, name) in [
        ("DataProcessors/Processor.xml", "Processor"),
        ("DataProcessors/Processor/Forms/Main.xml", "Main"),
    ] {
        let root = resource_resolve_fixture();
        let path = root.path().join("src").join(relative);
        let text = std::fs::read_to_string(&path)
            .unwrap()
            .replace(&format!("<Name>{name}</Name>"), "<Name>Other</Name>");
        std::fs::write(path, text).unwrap();
        let mut mcp = resource_resolve_mcp(root.path());
        let result = mcp.completed_tool_call(call_tool(
            2,
            "unica.resolve",
            json!({"path":"src/DataProcessors/Processor/Forms/Main/Ext/Form.xml"}),
        ));
        assert_eq!(result["ok"], false, "{relative}: {result:#}");
        assert_eq!(
            result["diagnostics"][0]["code"], "invalid_source",
            "{result:#}"
        );
        mcp.finish();
    }
}
