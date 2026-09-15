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
    let mut args = json!({"at":"ext:CommonModule.Fix","ops":[{"op":"code.insert","args":{"at":"ext:CommonModule.Fix","text":"Procedure Added() Export\nEndProcedure"}}],"dryRun":true});
    let preview = call(&mut mcp, 2, args.clone());
    assert_eq!(fs::read_to_string(&descriptor).unwrap(), before);
    let module = source.join("CommonModules/Fix/Ext/Module.bsl");
    assert!(!module.exists());
    args["dryRun"] = json!(false);
    args["ifRev"] = preview["rev"].clone();
    let applied = call(&mut mcp, 3, args.clone());
    let after = fs::read_to_string(&descriptor).unwrap();
    assert!(
        after.contains("<xr:Property>Module</xr:Property>"),
        "{after}"
    );
    assert!(after.contains("<xr:State>Extended</xr:State>"), "{after}");
    assert!(fs::read_to_string(&module)
        .unwrap()
        .contains("Procedure Added()"));
    assert_ne!(preview["rev"], applied["rev"]);
    args["ifRev"] = applied["rev"].clone();
    let repeated = call(&mut mcp, 4, args);
    assert_eq!(applied["rev"], repeated["rev"]);
    assert_eq!(fs::read_to_string(&descriptor).unwrap(), after);
    mcp.finish();
}
