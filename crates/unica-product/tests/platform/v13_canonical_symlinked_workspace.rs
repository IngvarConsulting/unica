/// The module outline the retired `unica.code.outline` answered lives in the
/// module node of `unica.view`: through a symlinked workspace it resolves the
/// methods of the module from the current file, and no BSL index state is
/// created for it.
#[cfg(unix)]
#[test]
fn canonical_stdio_views_a_module_through_a_symlinked_workspace() {
    let root = tempfile::tempdir().expect("symlinked workspace root");
    let real = root.path().join("real-workspace");
    let link = root.path().join("linked-workspace");
    let state = root.path().join("state");
    std::fs::create_dir_all(real.join("src/CommonModules/Demo/Ext")).unwrap();
    std::fs::create_dir(&state).unwrap();
    std::fs::write(
        real.join("v8project.yaml"),
        "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
    )
    .unwrap();
    std::fs::write(
        real.join("src/Configuration.xml"),
        r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Configuration><Properties><Name>Outline</Name></Properties><ChildObjects><CommonModule>Demo</CommonModule></ChildObjects></Configuration></MetaDataObject>"#,
    )
    .unwrap();
    std::fs::write(
        real.join("src/CommonModules/Demo.xml"),
        r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" xmlns:v8="http://v8.1c.ru/8.1/data/core" version="2.20"><CommonModule uuid="ac847dc9-e222-45cf-af4a-6fa863c919a8"><Properties><Name>Demo</Name><Synonym/><Comment/><Global>false</Global><ClientManagedApplication>false</ClientManagedApplication><Server>true</Server><ExternalConnection>false</ExternalConnection><ClientOrdinaryApplication>false</ClientOrdinaryApplication><ServerCall>true</ServerCall><Privileged>false</Privileged><ReturnValuesReuse>DontUse</ReturnValuesReuse></Properties></CommonModule></MetaDataObject>"#,
    )
    .unwrap();
    std::fs::write(
        real.join("src/CommonModules/Demo/Ext/Module.bsl"),
        "#Область Служебный\nПроцедура Служебная()\nКонецПроцедуры\n#КонецОбласти\n\nФункция Экспортная() Экспорт\n\tВозврат 1;\nКонецФункции\n",
    )
    .unwrap();
    std::os::unix::fs::symlink(&real, &link).unwrap();

    let mut mcp = McpProcess::start_at(&link, &state);
    mcp.exchange(json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {
            "protocolVersion": "2025-11-25",
            "capabilities": {},
            "clientInfo": {"name": "v13-outline-ci", "version": "1"}
        }
    }));
    mcp.notify(json!({
        "jsonrpc": "2.0",
        "method": "notifications/initialized",
        "params": {}
    }));
    let node = mcp.exchange(json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "tools/call",
        "params": {"name": "unica.view", "arguments": {"at": "main:CommonModule.Demo"}}
    }));
    let result = &node["result"]["structuredContent"];
    assert_eq!(result["ok"], true, "{node:#}");
    assert_eq!(result["data"]["kind"], "Module");
    let branches = result["data"]["branches"].as_array().unwrap();
    let method_branch = branches
        .iter()
        .find(|branch| branch["at"] == "main:CommonModule.Demo.Method")
        .expect("the module node counts its methods");
    assert_eq!(method_branch["count"], 2);
    let methods = mcp.exchange(json!({
        "jsonrpc": "2.0",
        "id": 3,
        "method": "tools/call",
        "params": {"name": "unica.view", "arguments": {"at": "main:CommonModule.Demo.Method"}}
    }));
    let items = methods["result"]["structuredContent"]["data"]["items"]
        .as_array()
        .unwrap();
    let exported = items
        .iter()
        .find(|item| item["title"] == "Экспортная")
        .expect("the exported function is listed");
    assert_eq!(exported["props"]["export"], true);
    assert_eq!(exported["props"]["methodKind"], "function");
    mcp.finish();
    assert!(
        !real.join(".build/unica/bsl_index").exists(),
        "viewing a module must not create BSL index state"
    );
}
