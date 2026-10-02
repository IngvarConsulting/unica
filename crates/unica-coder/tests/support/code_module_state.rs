use super::McpProcess;
use serde_json::{json, Value};
use std::fs;

fn call(mcp: &mut McpProcess, id: u64, args: Value) -> Value {
    let response = mcp.exchange(json!({"jsonrpc":"2.0", "id":id, "method":"tools/call", "params":{"name":"unica.apply", "arguments":args}}));
    let result = response["result"]["structuredContent"].clone();
    assert_eq!(result["ok"], true, "{response}");
    result
}

#[test]
fn canonical_stdio_code_insert_publishes_borrowed_module_and_state() {
    let root = tempfile::tempdir().unwrap();
    let workspace = root.path().join("workspace");
    let source = workspace.join("ext");
    let state = root.path().join("state");
    fs::create_dir_all(source.join("CommonModules/Fix/Ext")).unwrap();
    fs::create_dir_all(source.join("Ext")).unwrap();
    fs::create_dir(&state).unwrap();
    fs::write(
        workspace.join("v8project.yaml"),
        "format: DESIGNER\nsource-set:\n  - name: ext\n    type: EXTENSION\n    path: ext\n",
    )
    .unwrap();
    fs::write(source.join("Configuration.xml"), r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Configuration uuid="66666666-6666-6666-6666-666666666666"><InternalInfo/><Properties><ObjectBelonging>Adopted</ObjectBelonging><Name>Extension</Name><ConfigurationExtensionPurpose>Customization</ConfigurationExtensionPurpose><NamePrefix>E_</NamePrefix></Properties><ChildObjects><CommonModule>Fix</CommonModule></ChildObjects></Configuration></MetaDataObject>"#).unwrap();
    let descriptor = source.join("CommonModules/Fix.xml");
    let before = r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" xmlns:xr="http://v8.1c.ru/8.3/xcf/readable" version="2.20"><CommonModule uuid="77777777-7777-7777-7777-777777777777"><InternalInfo/><Properties><ObjectBelonging>Adopted</ObjectBelonging><Name>Fix</Name><ExtendedConfigurationObject>88888888-8888-8888-8888-888888888888</ExtendedConfigurationObject><Global>false</Global><Server>true</Server><ClientManagedApplication>false</ClientManagedApplication><ExternalConnection>false</ExternalConnection><ClientOrdinaryApplication>false</ClientOrdinaryApplication><ServerCall>false</ServerCall><Privileged>false</Privileged><ReturnValuesReuse>DontUse</ReturnValuesReuse></Properties><ChildObjects/></CommonModule></MetaDataObject>"#;
    fs::write(&descriptor, before).unwrap();
    let mut mcp = McpProcess::start(&workspace, &state);
    mcp.exchange(json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"module-state-test","version":"1"}}}));
    mcp.notify(json!({"jsonrpc":"2.0","method":"notifications/initialized"}));
    let args = json!({"at":"ext:CommonModule.Fix","ops":[{"op":"code.insert","args":{"at":"ext:CommonModule.Fix","text":"Procedure Added() Export\nEndProcedure"}}]});
    let preview = call(&mut mcp, 2, args.clone());
    assert!(preview["data"]["executionToken"]
        .as_str()
        .is_some_and(|token| !token.is_empty()));
    assert!(preview["data"]["planHash"]
        .as_str()
        .is_some_and(|hash| !hash.is_empty()));
    assert_eq!(fs::read_to_string(&descriptor).unwrap(), before);
    let module = source.join("CommonModules/Fix/Ext/Module.bsl");
    assert!(!module.exists());
    let execute = json!({"executionToken":preview["data"]["executionToken"]});
    let applied = call(&mut mcp, 3, execute.clone());
    let after = fs::read_to_string(&descriptor).unwrap();
    assert!(
        after.contains("<xr:Property>Module</xr:Property>"),
        "{after}"
    );
    assert!(after.contains("<xr:State>Extended</xr:State>"), "{after}");
    assert!(fs::read_to_string(&module)
        .unwrap()
        .contains("Procedure Added()"));
    assert_eq!(preview["data"]["planHash"], applied["data"]["planHash"]);
    assert!(applied["rev"].as_str().is_some_and(|rev| !rev.is_empty()));
    let repeated = call(&mut mcp, 4, execute);
    assert_eq!(applied["rev"], repeated["rev"]);
    let repeat_preview = call(&mut mcp, 5, args);
    assert!(repeat_preview["data"]["planHash"]
        .as_str()
        .is_some_and(|hash| !hash.is_empty()));
    assert!(repeat_preview["changed"]
        .as_array()
        .is_none_or(Vec::is_empty));
    assert_eq!(fs::read_to_string(&descriptor).unwrap(), after);
    let no_op = call(
        &mut mcp,
        6,
        json!({"executionToken":repeat_preview["data"]["executionToken"]}),
    );
    assert_eq!(
        repeat_preview["data"]["planHash"],
        no_op["data"]["planHash"]
    );
    assert!(no_op["changed"].as_array().is_none_or(Vec::is_empty));
    assert_eq!(fs::read_to_string(&descriptor).unwrap(), after);
    mcp.finish();
}

#[test]
fn canonical_stdio_root_modules_publish_configuration_state_and_repeat() {
    let root = tempfile::tempdir().unwrap();
    let workspace = root.path().join("workspace");
    let source = workspace.join("ext");
    let state = root.path().join("state");
    fs::create_dir_all(source.join("Ext")).unwrap();
    fs::create_dir(&state).unwrap();
    fs::write(
        workspace.join("v8project.yaml"),
        "format: DESIGNER\nsource-set:\n  - name: ext\n    type: EXTENSION\n    path: ext\n",
    )
    .unwrap();
    let descriptor = source.join("Configuration.xml");
    fs::write(&descriptor, r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Configuration uuid="66666666-6666-6666-6666-666666666666"><InternalInfo/><Properties><ObjectBelonging>Adopted</ObjectBelonging><Name>Extension</Name><ConfigurationExtensionPurpose>Customization</ConfigurationExtensionPurpose><NamePrefix>E_</NamePrefix></Properties><ChildObjects/></Configuration></MetaDataObject>"#).unwrap();
    let mut mcp = McpProcess::start(&workspace, &state);
    mcp.exchange(json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"root-module-state-test","version":"1"}}}));
    mcp.notify(json!({"jsonrpc":"2.0","method":"notifications/initialized"}));
    for (index, role) in [
        "ManagedApplication",
        "OrdinaryApplication",
        "Session",
        "ExternalConnection",
    ]
    .iter()
    .enumerate()
    {
        let before = fs::read_to_string(&descriptor).unwrap();
        let at = format!("ext:Module.{role}");
        let args = json!({"at":at,"ops":[{"op":"code.insert","args":{"at":at,"text":"Procedure Added() Export\nEndProcedure"}}]});
        let id = 2 + index as u64 * 5;
        let preview = call(&mut mcp, id, args.clone());
        assert!(preview["data"]["executionToken"]
            .as_str()
            .is_some_and(|token| !token.is_empty()));
        assert!(preview["data"]["planHash"]
            .as_str()
            .is_some_and(|hash| !hash.is_empty()));
        assert_eq!(fs::read_to_string(&descriptor).unwrap(), before);
        let module = source.join(format!("Ext/{role}Module.bsl"));
        assert!(!module.exists());
        let execute = json!({"executionToken":preview["data"]["executionToken"]});
        let applied = call(&mut mcp, id + 1, execute.clone());
        assert_eq!(preview["data"]["planHash"], applied["data"]["planHash"]);
        assert!(applied["rev"].as_str().is_some_and(|rev| !rev.is_empty()));
        let after = fs::read_to_string(&descriptor).unwrap();
        let document = roxmltree::Document::parse(&after).unwrap();
        let property = format!("{role}Module");
        let matching: Vec<_> = document
            .descendants()
            .filter(|node| {
                node.has_tag_name(("http://v8.1c.ru/8.3/xcf/readable", "PropertyState"))
                    && node.children().any(|child| {
                        child.has_tag_name(("http://v8.1c.ru/8.3/xcf/readable", "Property"))
                            && child.text() == Some(property.as_str())
                    })
            })
            .collect();
        assert_eq!(matching.len(), 1, "{after}");
        assert!(matching[0]
            .parent()
            .unwrap()
            .has_tag_name(("http://v8.1c.ru/8.3/MDClasses", "InternalInfo")));
        assert!(matching[0].children().any(|child| child
            .has_tag_name(("http://v8.1c.ru/8.3/xcf/readable", "State"))
            && child.text() == Some("Extended")));
        let bsl = fs::read(&module).unwrap();
        assert!(String::from_utf8_lossy(&bsl).contains("Procedure Added()"));
        let repeated = call(&mut mcp, id + 2, execute);
        assert_eq!(repeated["rev"], applied["rev"]);
        let repeat_preview = call(&mut mcp, id + 3, args);
        assert!(repeat_preview["data"]["planHash"]
            .as_str()
            .is_some_and(|hash| !hash.is_empty()));
        assert!(repeat_preview["changed"]
            .as_array()
            .is_none_or(Vec::is_empty));
        assert_eq!(fs::read_to_string(&descriptor).unwrap(), after);
        assert_eq!(fs::read(&module).unwrap(), bsl);
        let no_op = call(
            &mut mcp,
            id + 4,
            json!({"executionToken":repeat_preview["data"]["executionToken"]}),
        );
        assert_eq!(
            repeat_preview["data"]["planHash"],
            no_op["data"]["planHash"]
        );
        assert!(no_op["changed"].as_array().is_none_or(Vec::is_empty));
        assert_eq!(fs::read_to_string(&descriptor).unwrap(), after);
        assert_eq!(fs::read(&module).unwrap(), bsl);
    }
    mcp.finish();
}
