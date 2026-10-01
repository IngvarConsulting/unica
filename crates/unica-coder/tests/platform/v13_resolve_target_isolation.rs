#![cfg(unix)]

use super::*;

#[test]
fn resolve_path_checks_only_target_and_necessary_owner_on_public_mcp() {
    use std::os::unix::fs::symlink;

    let root = tempfile::tempdir().expect("resolve workspace");
    let workspace = root.path();
    std::fs::create_dir_all(workspace.join("Catalogs/Visible/Forms"))
        .expect("form source directory");
    std::fs::create_dir_all(workspace.join("CommonModules/Main/Ext"))
        .expect("main module source directory");
    std::fs::write(
        workspace.join("v8project.yaml"),
        "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: .\n",
    )
    .expect("workspace manifest");
    std::fs::write(
        workspace.join("Configuration.xml"),
        r#"<MetaDataObject><Configuration><Properties><Name>Main</Name></Properties><ChildObjects><Catalog>Visible</Catalog><Catalog>Broken</Catalog></ChildObjects></Configuration></MetaDataObject>"#,
    )
    .expect("configuration descriptor");
    let owner = workspace.join("Catalogs/Visible.xml");
    std::fs::write(
        &owner,
        r#"<MetaDataObject><Catalog><Properties><Name>Visible</Name></Properties><ChildObjects><Form>Form</Form></ChildObjects></Catalog></MetaDataObject>"#,
    )
    .expect("visible catalog descriptor");
    std::fs::write(
        workspace.join("Catalogs/Visible/Forms/Form.xml"),
        r#"<MetaDataObject><Form><Properties><Name>Form</Name></Properties></Form></MetaDataObject>"#,
    )
    .expect("form descriptor");
    std::fs::write(
        workspace.join("CommonModules/Main.xml"),
        r#"<MetaDataObject><CommonModule><Properties><Name>Main</Name></Properties></CommonModule></MetaDataObject>"#,
    )
    .expect("common module owner");
    let module = workspace.join("CommonModules/Main/Ext/Module.bsl");
    std::fs::write(&module, "// module\n").expect("common module source");
    let foreign = workspace.join("physical-broken.xml");
    std::fs::write(
        &foreign,
        r#"<MetaDataObject><Catalog><Properties><Name>Broken</Name></Properties></Catalog></MetaDataObject>"#,
    )
    .expect("foreign descriptor target");
    symlink(&foreign, workspace.join("Catalogs/Broken.xml")).expect("linked unrelated descriptor");

    let mut mcp = McpProcess::start(workspace);
    let initialized = mcp.exchange(json!({
        "jsonrpc": "2.0", "id": 1, "method": "initialize",
        "params": {"protocolVersion": "2025-06-18", "capabilities": {},
                   "clientInfo": {"name": "resolve-proof", "version": "1"}}
    }));
    assert_eq!(initialized["result"]["serverInfo"]["name"], "unica");
    mcp.notify(json!({"jsonrpc":"2.0", "method":"notifications/initialized", "params":{}}));

    let resolved = mcp.completed_tool_call(call_tool(
        2,
        "unica.resolve",
        json!({"path": "Catalogs/Visible/Forms/Form.xml"}),
    ));
    assert_eq!(resolved["ok"], true, "{resolved:#}");
    assert_eq!(resolved["data"]["at"], "main:Catalog.Visible.Form.Form");

    let absolute_form = std::fs::canonicalize(workspace.join("Catalogs/Visible/Forms/Form.xml"))
        .expect("absolute form path");
    let absolute_resolved = mcp.completed_tool_call(call_tool(
        5,
        "unica.resolve",
        json!({"path": absolute_form}),
    ));
    assert_eq!(absolute_resolved["ok"], true, "{absolute_resolved:#}");
    assert_eq!(
        absolute_resolved["data"]["at"],
        "main:Catalog.Visible.Form.Form"
    );

    let absolute_module = std::fs::canonicalize(&module).expect("absolute module path");
    let alias = mcp.completed_tool_call(call_tool(
        6,
        "unica.resolve",
        json!({"path": absolute_module}),
    ));
    assert_eq!(alias["ok"], true, "{alias:#}");
    assert_eq!(alias["data"]["at"], "main:CommonModule.Main");
    assert_eq!(alias["data"]["path"], "CommonModules/Main/Ext/Module.bsl");

    let outside = tempfile::tempdir().expect("outside path prefix");
    let false_path = outside.path().join("CommonModules/Main/Ext/Module.bsl");
    let false_match =
        mcp.completed_tool_call(call_tool(8, "unica.resolve", json!({"path": false_path})));
    assert_eq!(false_match["ok"], false, "{false_match:#}");
    assert_eq!(false_match["diagnostics"][0]["code"], "not_found");

    let names = mcp.completed_tool_call(call_tool(
        3,
        "unica.search",
        json!({"query": "Visible", "corpus": "names"}),
    ));
    assert_eq!(
        names["ok"], false,
        "full name search must expose the fault: {names:#}"
    );
    assert_eq!(names["diagnostics"][0]["code"], "invalid_source");

    let physical_owner = workspace.join("physical-visible.xml");
    std::fs::rename(&owner, &physical_owner).expect("move the required owner descriptor");
    symlink(&physical_owner, &owner).expect("link the required owner descriptor");
    let refused = mcp.completed_tool_call(call_tool(
        4,
        "unica.resolve",
        json!({"path": "Catalogs/Visible/Forms/Form.xml"}),
    ));
    assert_eq!(refused["ok"], false, "{refused:#}");
    assert_eq!(refused["diagnostics"][0]["code"], "invalid_source");
    mcp.finish();
}

#[test]
fn resolve_relative_source_prefix_skips_foreign_linked_collections_on_public_mcp() {
    use std::os::unix::fs::symlink;

    let root = tempfile::tempdir().expect("resolve workspace");
    let workspace = root.path();
    let extension = workspace.join("src/extension");
    std::fs::create_dir_all(extension.join("Catalogs/Visible/Forms"))
        .expect("extension form source directory");
    std::fs::create_dir_all(extension.join("CommonModules/Main/Ext"))
        .expect("extension module source directory");
    std::fs::write(
        workspace.join("v8project.yaml"),
        "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: .\n  - name: extension\n    type: EXTENSION\n    path: src/extension\n",
    )
    .expect("workspace manifest");
    std::fs::write(
        workspace.join("Configuration.xml"),
        r#"<MetaDataObject><Configuration><Properties><Name>Main</Name></Properties></Configuration></MetaDataObject>"#,
    )
    .expect("main descriptor");
    std::fs::write(
        extension.join("Configuration.xml"),
        r#"<MetaDataObject><Configuration><Properties><Name>Extension</Name></Properties></Configuration></MetaDataObject>"#,
    )
    .expect("extension descriptor");
    std::fs::write(
        extension.join("Catalogs/Visible.xml"),
        r#"<MetaDataObject><Catalog><Properties><Name>Visible</Name></Properties><ChildObjects><Form>Form</Form></ChildObjects></Catalog></MetaDataObject>"#,
    )
    .expect("catalog owner");
    std::fs::write(
        extension.join("Catalogs/Visible/Forms/Form.xml"),
        r#"<MetaDataObject><Form><Properties><Name>Form</Name></Properties></Form></MetaDataObject>"#,
    )
    .expect("form descriptor");
    std::fs::write(
        extension.join("CommonModules/Main.xml"),
        r#"<MetaDataObject><CommonModule><Properties><Name>Main</Name></Properties></CommonModule></MetaDataObject>"#,
    )
    .expect("common module owner");
    std::fs::write(
        extension.join("CommonModules/Main/Ext/Module.bsl"),
        "// module\n",
    )
    .expect("module source");
    symlink(extension.join("Catalogs"), workspace.join("Catalogs"))
        .expect("foreign linked Catalogs collection");
    symlink(
        extension.join("CommonModules"),
        workspace.join("CommonModules"),
    )
    .expect("foreign linked CommonModules collection");

    let mut mcp = McpProcess::start(workspace);
    let initialized = mcp.exchange(json!({
        "jsonrpc": "2.0", "id": 1, "method": "initialize",
        "params": {"protocolVersion": "2025-06-18", "capabilities": {},
                   "clientInfo": {"name": "resolve-source-proof", "version": "1"}}
    }));
    assert_eq!(initialized["result"]["serverInfo"]["name"], "unica");
    mcp.notify(json!({"jsonrpc":"2.0", "method":"notifications/initialized", "params":{}}));

    let catalog = mcp.completed_tool_call(call_tool(
        6,
        "unica.resolve",
        json!({"path": "src/extension/Catalogs/Visible.xml"}),
    ));
    assert_eq!(catalog["ok"], true, "{catalog:#}");
    assert_eq!(catalog["data"]["at"], "extension:Catalog.Visible");

    let form = mcp.completed_tool_call(call_tool(
        2,
        "unica.resolve",
        json!({"path": "src/extension/Catalogs/Visible/Forms/Form.xml"}),
    ));
    assert_eq!(form["ok"], true, "{form:#}");
    assert_eq!(form["data"]["at"], "extension:Catalog.Visible.Form.Form");

    let module = mcp.completed_tool_call(call_tool(
        3,
        "unica.resolve",
        json!({"path": "src/extension/CommonModules/Main/Ext/Module.bsl"}),
    ));
    assert_eq!(module["ok"], true, "{module:#}");
    assert_eq!(module["data"]["at"], "extension:CommonModule.Main");
    assert_eq!(module["data"]["path"], "CommonModules/Main/Ext/Module.bsl");

    let absolute_module =
        std::fs::canonicalize(extension.join("CommonModules/Main/Ext/Module.bsl"))
            .expect("absolute extension module path");
    let absolute = mcp.completed_tool_call(call_tool(
        5,
        "unica.resolve",
        json!({"path": absolute_module}),
    ));
    assert_eq!(absolute["ok"], true, "{absolute:#}");
    assert_eq!(absolute["data"]["at"], "extension:CommonModule.Main");

    let ambiguous = mcp.completed_tool_call(call_tool(
        4,
        "unica.resolve",
        json!({"path": "Catalogs/Visible/Forms/Form.xml"}),
    ));
    assert_eq!(ambiguous["ok"], false, "{ambiguous:#}");
    assert_eq!(ambiguous["diagnostics"][0]["code"], "invalid_source");
    mcp.finish();
}

#[test]
fn resolve_path_does_not_inherit_the_search_source_set_limit() {
    let root = tempfile::tempdir().expect("resolve workspace");
    let workspace = root.path();
    let mut manifest = String::from("format: DESIGNER\nsource-set:\n");
    for index in 0..65 {
        let name = format!("source{index:02}");
        let source = workspace.join("src").join(&name);
        std::fs::create_dir_all(&source).expect("source root");
        std::fs::write(
            source.join("Configuration.xml"),
            r#"<MetaDataObject><Configuration><Properties><Name>Main</Name></Properties></Configuration></MetaDataObject>"#,
        )
        .expect("source descriptor");
        let kind = if index == 0 {
            "CONFIGURATION"
        } else {
            "EXTENSION"
        };
        manifest.push_str(&format!(
            "  - name: {name}\n    type: {kind}\n    path: src/{name}\n"
        ));
    }
    std::fs::write(workspace.join("v8project.yaml"), manifest).expect("workspace manifest");
    let target = workspace.join("src/source64/Catalogs/Requested.xml");
    std::fs::create_dir_all(target.parent().unwrap()).expect("target collection");
    std::fs::write(
        &target,
        r#"<MetaDataObject><Catalog><Properties><Name>Requested</Name></Properties></Catalog></MetaDataObject>"#,
    )
    .expect("target descriptor");

    let mut mcp = McpProcess::start(workspace);
    let initialized = mcp.exchange(json!({
        "jsonrpc": "2.0", "id": 1, "method": "initialize",
        "params": {"protocolVersion": "2025-06-18", "capabilities": {},
                   "clientInfo": {"name": "resolve-many-sources", "version": "1"}}
    }));
    assert_eq!(initialized["result"]["serverInfo"]["name"], "unica");
    mcp.notify(json!({"jsonrpc":"2.0", "method":"notifications/initialized", "params":{}}));

    let search = mcp.completed_tool_call(call_tool(
        2,
        "unica.search",
        json!({"query": "Requested", "corpus": "names"}),
    ));
    assert_eq!(search["ok"], false, "{search:#}");
    assert_eq!(search["diagnostics"][0]["code"], "provider_limit_exceeded");

    let resolved = mcp.completed_tool_call(call_tool(
        3,
        "unica.resolve",
        json!({"path": std::fs::canonicalize(&target).expect("absolute admitted target")}),
    ));
    assert_eq!(resolved["ok"], true, "{resolved:#}");
    assert_eq!(resolved["data"]["at"], "source64:Catalog.Requested");
    mcp.finish();
}

#[test]
fn resolve_absolute_xml_ignores_a_broken_foreign_target_and_reports_ambiguous_alias() {
    use std::os::unix::fs::symlink;

    let root = tempfile::tempdir().expect("resolve workspace");
    let workspace = root.path();
    let first = workspace.join("src/A");
    let second = workspace.join("src/B");
    for source in [&first, &second] {
        std::fs::create_dir_all(source.join("Catalogs")).expect("catalog collection");
        std::fs::create_dir_all(source.join("CommonModules/Main/Ext")).expect("module directory");
        std::fs::write(
            source.join("Configuration.xml"),
            r#"<MetaDataObject><Configuration><Properties><Name>Main</Name></Properties></Configuration></MetaDataObject>"#,
        )
        .expect("configuration descriptor");
        std::fs::write(
            source.join("CommonModules/Main.xml"),
            r#"<MetaDataObject><CommonModule><Properties><Name>Main</Name></Properties></CommonModule></MetaDataObject>"#,
        )
        .expect("common module owner");
        std::fs::write(
            source.join("CommonModules/Main/Ext/Module.bsl"),
            "// module\n",
        )
        .expect("module source");
    }
    std::fs::write(
        workspace.join("v8project.yaml"),
        "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src/A\n  - name: other\n    type: EXTENSION\n    path: src/B\n",
    )
    .expect("workspace manifest");
    std::fs::write(
        first.join("Catalogs/X.xml"),
        r#"<MetaDataObject><Catalog><Properties><Name>X</Name></Properties></Catalog></MetaDataObject>"#,
    )
    .expect("target catalog descriptor");
    std::fs::write(
        first.join("Catalogs/Bad.Name.xml"),
        r#"<MetaDataObject><Catalog><Properties><Name>Bad.Name</Name></Properties></Catalog></MetaDataObject>"#,
    )
    .expect("invalid logical name descriptor");
    let foreign = workspace.join("physical-broken.xml");
    std::fs::write(
        &foreign,
        r#"<MetaDataObject><Catalog><Properties><Name>X</Name></Properties></Catalog></MetaDataObject>"#,
    )
    .expect("foreign descriptor target");
    symlink(&foreign, second.join("Catalogs/X.xml")).expect("linked foreign target");

    let mut mcp = McpProcess::start(workspace);
    let initialized = mcp.exchange(json!({
        "jsonrpc": "2.0", "id": 1, "method": "initialize",
        "params": {"protocolVersion": "2025-06-18", "capabilities": {},
                   "clientInfo": {"name": "resolve-absolute-proof", "version": "1"}}
    }));
    assert_eq!(initialized["result"]["serverInfo"]["name"], "unica");
    mcp.notify(json!({"jsonrpc":"2.0", "method":"notifications/initialized", "params":{}}));

    let canonical_first = std::fs::canonicalize(&first).expect("retained first source root");
    let absolute = canonical_first.join("Catalogs/X.xml");
    let catalog = mcp.completed_tool_call(call_tool(2, "unica.resolve", json!({"path": absolute})));
    assert_eq!(catalog["ok"], true, "{catalog:#}");
    assert_eq!(catalog["data"]["at"], "main:Catalog.X");

    let requested_broken = mcp.completed_tool_call(call_tool(
        7,
        "unica.resolve",
        json!({"path": std::fs::canonicalize(&second).unwrap().join("Catalogs/X.xml")}),
    ));
    assert_eq!(requested_broken["ok"], false, "{requested_broken:#}");
    assert_eq!(requested_broken["diagnostics"][0]["code"], "invalid_source");

    let outside = workspace.join("outside/Catalogs/X.xml");
    std::fs::create_dir_all(outside.parent().unwrap()).expect("outside collection");
    std::fs::write(
        &outside,
        r#"<MetaDataObject><Catalog><Properties><Name>X</Name></Properties></Catalog></MetaDataObject>"#,
    )
    .expect("outside catalog descriptor");
    let outside_result = mcp.completed_tool_call(call_tool(
        8,
        "unica.resolve",
        json!({"path": std::fs::canonicalize(&outside).unwrap()}),
    ));
    assert_eq!(outside_result["diagnostics"][0]["code"], "not_found");

    let missing_collection_member = mcp.completed_tool_call(call_tool(
        5,
        "unica.resolve",
        json!({"path": "src/A/Catalogs/Configuration.xml"}),
    ));
    assert_eq!(
        missing_collection_member["diagnostics"][0]["code"], "not_found",
        "{missing_collection_member:#}"
    );
    let missing_absolute = mcp.completed_tool_call(call_tool(
        6,
        "unica.resolve",
        json!({"path": canonical_first.join("Catalogs/Configuration.xml")}),
    ));
    assert_eq!(
        missing_absolute["diagnostics"][0]["code"], "not_found",
        "{missing_absolute:#}"
    );

    let invalid_name = mcp.completed_tool_call(call_tool(
        3,
        "unica.resolve",
        json!({"path": canonical_first.join("Catalogs/Bad.Name.xml")}),
    ));
    assert_eq!(invalid_name["ok"], false, "{invalid_name:#}");
    assert_eq!(invalid_name["diagnostics"][0]["code"], "invalid_source");

    let ambiguous_alias = mcp.completed_tool_call(call_tool(
        4,
        "unica.resolve",
        json!({"path": "CommonModules/Main/Ext/Module.bsl"}),
    ));
    assert_eq!(ambiguous_alias["ok"], false, "{ambiguous_alias:#}");
    assert_eq!(ambiguous_alias["diagnostics"][0]["code"], "bad_value");

    let absolute_module = mcp.completed_tool_call(call_tool(
        9,
        "unica.resolve",
        json!({"path": canonical_first.join("CommonModules/Main/Ext/Module.bsl")}),
    ));
    assert_eq!(absolute_module["ok"], true, "{absolute_module:#}");
    assert_eq!(absolute_module["data"]["at"], "main:CommonModule.Main");
    mcp.finish();
}

#[test]
fn resolve_absolute_path_uses_the_deepest_admitted_source_root() {
    let root = tempfile::tempdir().expect("nested source workspace");
    let workspace = root.path();
    let nested = workspace.join("Catalogs");
    std::fs::create_dir_all(&nested).expect("nested source root");
    std::fs::write(
        workspace.join("v8project.yaml"),
        "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: .\n  - name: nested\n    type: EXTENSION\n    path: Catalogs\n",
    )
    .expect("source manifest");
    for source in [workspace, nested.as_path()] {
        std::fs::write(
            source.join("Configuration.xml"),
            r#"<MetaDataObject><Configuration><Properties><Name>Main</Name></Properties></Configuration></MetaDataObject>"#,
        )
        .expect("source descriptor");
    }
    std::fs::write(
        nested.join("X.xml"),
        r#"<MetaDataObject><Catalog><Properties><Name>X</Name></Properties></Catalog></MetaDataObject>"#,
    )
    .expect("parent-visible catalog descriptor");

    let mut mcp = McpProcess::start(workspace);
    let initialized = mcp.exchange(json!({
        "jsonrpc": "2.0", "id": 1, "method": "initialize",
        "params": {"protocolVersion": "2025-06-18", "capabilities": {},
                   "clientInfo": {"name": "resolve-deepest-root", "version": "1"}}
    }));
    assert_eq!(initialized["result"]["serverInfo"]["name"], "unica");
    mcp.notify(json!({"jsonrpc":"2.0", "method":"notifications/initialized", "params":{}}));
    let absolute = std::fs::canonicalize(nested.join("X.xml")).expect("absolute nested target");
    let result = mcp.completed_tool_call(call_tool(2, "unica.resolve", json!({"path": absolute})));
    assert_eq!(result["ok"], false, "{result:#}");
    assert_eq!(result["diagnostics"][0]["code"], "not_found");
    mcp.finish();
}

#[test]
fn resolve_path_succeeds_above_the_full_directory_byte_budget() {
    let root = tempfile::tempdir().expect("large resolve workspace");
    let workspace = root.path();
    let catalogs = workspace.join("Catalogs");
    std::fs::create_dir(&catalogs).expect("catalog collection");
    std::fs::write(
        workspace.join("v8project.yaml"),
        "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: .\n",
    )
    .expect("source manifest");
    std::fs::write(
        workspace.join("Configuration.xml"),
        r#"<MetaDataObject><Configuration><Properties><Name>Main</Name></Properties></Configuration></MetaDataObject>"#,
    )
    .expect("configuration descriptor");
    std::fs::write(
        catalogs.join("Requested.xml"),
        r#"<MetaDataObject><Catalog><Properties><Name>Requested</Name></Properties></Catalog></MetaDataObject>"#,
    )
    .expect("requested descriptor");

    // The full directory charges the synonym once as the title and again as
    // a fact. These unrelated valid objects alone exceed its 16 MiB budget.
    const UNRELATED: usize = 1_200;
    const SYNONYM_BYTES: usize = 7_200;
    const _: () = assert!(UNRELATED * SYNONYM_BYTES * 2 > 16 * 1024 * 1024);
    let synonym = "Q".repeat(SYNONYM_BYTES);
    for index in 0..UNRELATED {
        let name = format!("U{index:04}");
        std::fs::write(
            catalogs.join(format!("{name}.xml")),
            format!(
                "<MetaDataObject><Catalog><Properties><Name>{name}</Name><Synonym><v8:item><v8:lang>ru</v8:lang><v8:content>{synonym}</v8:content></v8:item></Synonym></Properties></Catalog></MetaDataObject>"
            ),
        )
        .expect("unrelated catalog descriptor");
    }
    std::fs::write(catalogs.join("ZZBroken.xml"), "<broken").expect("damaged foreign descriptor");

    let mut mcp = McpProcess::start(workspace);
    let initialized = mcp.exchange(json!({
        "jsonrpc": "2.0", "id": 1, "method": "initialize",
        "params": {"protocolVersion": "2025-06-18", "capabilities": {},
                   "clientInfo": {"name": "resolve-large-proof", "version": "1"}}
    }));
    assert_eq!(initialized["result"]["serverInfo"]["name"], "unica");
    mcp.notify(json!({"jsonrpc":"2.0", "method":"notifications/initialized", "params":{}}));

    let full_search = mcp.completed_tool_call(call_tool(
        2,
        "unica.search",
        json!({"query": "Requested", "corpus": "names"}),
    ));
    assert_eq!(full_search["ok"], false, "{full_search:#}");
    assert_eq!(
        full_search["diagnostics"][0]["code"], "provider_limit_exceeded",
        "{full_search:#}"
    );

    let resolved = mcp.completed_tool_call(call_tool(
        3,
        "unica.resolve",
        json!({"path": "Catalogs/Requested.xml"}),
    ));
    assert_eq!(resolved["ok"], true, "{resolved:#}");
    assert_eq!(resolved["data"]["at"], "main:Catalog.Requested");
    mcp.finish();
}
