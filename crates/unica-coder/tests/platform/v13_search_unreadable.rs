#![cfg(unix)]

use super::*;

#[test]
fn unreadable_name_descriptor_is_reported_without_losing_proven_matches() {
    use std::os::unix::fs::PermissionsExt;

    struct RestorePermissions(std::path::PathBuf, std::fs::Permissions);
    impl Drop for RestorePermissions {
        fn drop(&mut self) {
            let _ = std::fs::set_permissions(&self.0, self.1.clone());
        }
    }

    let root = tempfile::tempdir().expect("name search workspace");
    let workspace = root.path();
    std::fs::create_dir_all(workspace.join("CommonModules")).expect("module collection");
    std::fs::create_dir_all(workspace.join("Catalogs")).expect("catalog collection");
    std::fs::write(
        workspace.join("v8project.yaml"),
        "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: .\n",
    )
    .expect("workspace manifest");
    std::fs::write(
        workspace.join("Configuration.xml"),
        r#"<?xml version="1.0" encoding="UTF-8"?><MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Configuration uuid="aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa"><Properties><Name>Main</Name></Properties><ChildObjects><CommonModule>Visible</CommonModule><Catalog>Hidden</Catalog></ChildObjects></Configuration></MetaDataObject>"#,
    )
    .expect("configuration descriptor");
    let visible = r#"<?xml version="1.0" encoding="UTF-8"?><MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><CommonModule uuid="bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb"><Properties><Name>Visible</Name><Global>false</Global><ClientManagedApplication>true</ClientManagedApplication><Server>true</Server><ExternalConnection>false</ExternalConnection><ClientOrdinaryApplication>false</ClientOrdinaryApplication><ServerCall>false</ServerCall><Privileged>false</Privileged><ReturnValuesReuse>DontUse</ReturnValuesReuse></Properties></CommonModule></MetaDataObject>"#;
    std::fs::write(workspace.join("CommonModules/Visible.xml"), visible)
        .expect("visible descriptor");
    let hidden = workspace.join("Catalogs/Hidden.xml");
    std::fs::write(
        &hidden,
        r#"<?xml version="1.0" encoding="UTF-8"?><MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" xmlns:v8="http://v8.1c.ru/8.1/data/core" version="2.20"><Catalog uuid="cccccccc-cccc-4ccc-8ccc-cccccccccccc"><Properties><Name>Hidden</Name><Synonym><v8:item><v8:lang>en</v8:lang><v8:content>Concealed</v8:content></v8:item></Synonym></Properties></Catalog></MetaDataObject>"#,
    )
    .expect("hidden descriptor");
    let permissions = std::fs::metadata(&hidden)
        .expect("hidden metadata")
        .permissions();
    let restore = RestorePermissions(hidden.clone(), permissions);
    std::fs::set_permissions(&hidden, std::fs::Permissions::from_mode(0o0))
        .expect("make one descriptor unreadable");
    if std::fs::File::open(&hidden).is_ok() {
        // Privileged local runners can bypass mode bits. The injected unit
        // test exercises this fault deterministically on every platform.
        return;
    }

    let mut mcp = McpProcess::start(workspace);
    let initialized = mcp.exchange(json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {
            "protocolVersion": "2025-06-18",
            "capabilities": {},
            "clientInfo": {"name": "v13-names-partial-ci", "version": "1"}
        }
    }));
    assert_eq!(initialized["result"]["serverInfo"]["name"], "unica");
    mcp.notify(json!({
        "jsonrpc": "2.0",
        "method": "notifications/initialized",
        "params": {}
    }));

    let found = domain_result(&mcp.exchange(call_tool(
        2,
        "unica.search",
        json!({"query": "Visible", "corpus": "names"}),
    )));
    assert_eq!(found["ok"], true, "{found:#}");
    assert_eq!(
        found["data"]["matches"][0]["at"],
        "main:CommonModule.Visible"
    );
    assert_eq!(found["data"]["approximate"], false);
    assert_eq!(found["data"]["sourceCoverage"]["complete"], false);
    assert_eq!(found["data"]["sourceCoverage"]["omitted"], 1);
    assert_eq!(found["data"]["sourceCoverage"]["detailsTruncated"], false);
    assert_eq!(
        found["data"]["sourceCoverage"]["details"],
        json!([{"sourceSet": "main", "reason": "descriptor_unreadable"}])
    );
    assert!(
        !found.to_string().contains("Hidden.xml"),
        "unproven paths must not enter the public response: {found:#}"
    );

    let approximate = domain_result(&mcp.exchange(call_tool(
        5,
        "unica.search",
        json!({"query": "Visibke", "corpus": "names"}),
    )));
    assert_eq!(approximate["ok"], true, "{approximate:#}");
    assert_eq!(
        approximate["data"]["matches"][0]["at"],
        "main:CommonModule.Visible"
    );
    assert_eq!(approximate["data"]["approximate"], true);
    assert_eq!(approximate["data"]["sourceCoverage"]["complete"], false);

    let empty = domain_result(&mcp.exchange(call_tool(
        3,
        "unica.search",
        json!({"query": "DefinitelyAbsentWord", "corpus": "names", "kind": "Catalog"}),
    )));
    assert_eq!(empty["ok"], true, "{empty:#}");
    assert_eq!(empty["data"]["matches"], json!([]));
    assert_eq!(empty["data"]["approximate"], false);
    assert_eq!(empty["data"]["sourceCoverage"]["complete"], false);
    assert_eq!(empty["data"]["sourceCoverage"]["omitted"], 1);

    let unsupported = domain_result(&mcp.exchange(call_tool(
        6,
        "unica.search",
        json!({"query": "Visible", "corpus": "names", "scope": "main:Form"}),
    )));
    assert_eq!(unsupported["diagnostics"][0]["code"], "unsupported_scope");

    let resolve = domain_result(&mcp.exchange(call_tool(
        4,
        "unica.resolve",
        json!({"path": "CommonModules/Visible.xml"}),
    )));
    assert_eq!(resolve["ok"], false, "{resolve:#}");
    assert_eq!(
        resolve["diagnostics"][0]["code"], "provider_unavailable",
        "{resolve:#}"
    );
    assert_eq!(resolve["diagnostics"][0]["detailCode"], "source_unreadable");
    std::fs::set_permissions(&hidden, restore.1.clone()).expect("restore descriptor access");
    let complete = domain_result(&mcp.exchange(call_tool(
        7,
        "unica.search",
        json!({"query": "Hidden", "corpus": "names"}),
    )));
    assert_eq!(complete["ok"], true, "{complete:#}");
    assert_eq!(complete["data"]["matches"][0]["at"], "main:Catalog.Hidden");
    assert_eq!(complete["data"]["sourceCoverage"]["complete"], true);
    assert_eq!(complete["data"]["sourceCoverage"]["omitted"], 0);
    let synonym = domain_result(&mcp.exchange(call_tool(
        8,
        "unica.search",
        json!({"query": "Concealed", "corpus": "names"}),
    )));
    assert_eq!(synonym["ok"], true, "{synonym:#}");
    assert_eq!(synonym["data"]["matches"][0]["at"], "main:Catalog.Hidden");
    assert_eq!(synonym["data"]["matches"][0]["title"], "Concealed");
    assert_eq!(synonym["data"]["sourceCoverage"]["complete"], true);
    mcp.finish();
}
