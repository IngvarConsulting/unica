use super::{McpProcess, RESPONSE_DEADLINE};
use serde_json::{json, Value};
use std::fs;
use std::path::Path;
use std::time::Instant;

const MD: &str = "http://v8.1c.ru/8.3/MDClasses";
const XR: &str = "http://v8.1c.ru/8.3/xcf/readable";
const EXTENDED: &str = "<xr:PropertyState><xr:Property>ObjectModule</xr:Property><xr:State>Extended</xr:State></xr:PropertyState>";

fn start(workspace: &Path, state: &Path) -> McpProcess {
    fs::create_dir(state).unwrap();
    let mut mcp = McpProcess::start(workspace, state);
    let initialized = mcp.exchange(json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"cfe-structure-test","version":"1"}}}));
    assert_eq!(initialized["result"]["serverInfo"]["name"], "unica");
    mcp.notify(json!({"jsonrpc":"2.0","method":"notifications/initialized"}));
    mcp
}

fn call(mcp: &mut McpProcess, name: &str, args: Value) -> Value {
    let context = format!("{name} {args}");
    let mut response = mcp.exchange(json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":name,"arguments":args}}));
    let deadline = Instant::now() + RESPONSE_DEADLINE;
    loop {
        let result = &response["result"]["structuredContent"];
        assert_eq!(result["ok"], true, "{context}: {response:#}");
        let Some(task_id) = result["data"]["task"]["taskId"].as_str() else {
            return result.clone();
        };
        assert!(
            Instant::now() < deadline,
            "{context}: task did not finish: {response:#}"
        );
        response = mcp.exchange(json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"unica.task.result","arguments":{"taskId":task_id,"waitMs":7000}}}));
    }
}

fn configuration(children: &str, extension: bool) -> String {
    let properties = if extension {
        "<ObjectBelonging>Adopted</ObjectBelonging><Name>GuardedExtension</Name><ConfigurationExtensionPurpose>Customization</ConfigurationExtensionPurpose><NamePrefix>GE_</NamePrefix>"
    } else {
        "<Name>Parent</Name><CompatibilityMode>Version8_3_27</CompatibilityMode>"
    };
    format!(
        r#"<MetaDataObject xmlns="{MD}" version="2.20"><Configuration uuid="66666666-6666-4666-8666-666666666666"><InternalInfo/><Properties>{properties}</Properties><ChildObjects>{children}</ChildObjects></Configuration></MetaDataObject>"#
    )
}

// Ported from #621 to the current check/task protocol. Inputs are deliberately
// damaged on disk: the checker must diagnose existing sources without repairing them.
#[test]
fn canonical_stdio_checks_borrowed_cfe_structure_without_writes() {
    let root = tempfile::tempdir().unwrap();
    let workspace = root.path().join("workspace");
    let source = workspace.join("ext");
    fs::create_dir_all(source.join("Reports/PriceList/Ext")).unwrap();
    fs::write(
        workspace.join("v8project.yaml"),
        "format: DESIGNER\nsource-set:\n  - name: ext\n    type: EXTENSION\n    path: ext\n",
    )
    .unwrap();
    let config = configuration("<Report>PriceList</Report>", true);
    fs::write(source.join("Configuration.xml"), &config).unwrap();
    let descriptor = source.join("Reports/PriceList.xml");
    let module = source.join("Reports/PriceList/Ext/ObjectModule.bsl");
    let mut mcp = start(&workspace, &root.path().join("state"));
    let duplicate_state = EXTENDED.repeat(2);
    let cases = [
        ("missing", "", "", "", false, false),
        ("foreign", "", "<ChildObjects xmlns=\"urn:foreign\"/>", "", false, false),
        ("duplicate", "", "<ChildObjects/><ChildObjects/>", "", false, false),
        ("order", "<ChildObjects/>", "", "", false, false),
        ("empty", "", "<ChildObjects/>", "", false, true),
        ("prefixed", "", "<md:ChildObjects xmlns:md=\"http://v8.1c.ru/8.3/MDClasses\"/>", "", false, true),
        ("missing-state", "", "<ChildObjects/>", "", true, false),
        ("extended", "", "<ChildObjects/>", EXTENDED, true, true),
        ("prefixed-state", "", "<ChildObjects/>", "<state:PropertyState xmlns:state=\"http://v8.1c.ru/8.3/xcf/readable\"><state:Property>ObjectModule</state:Property><state:State>Extended</state:State></state:PropertyState>", true, true),
        ("notify-without-module", "", "<ChildObjects/>", "<xr:PropertyState><xr:Property>ObjectModule</xr:Property><xr:State>Notify</xr:State></xr:PropertyState>", false, true),
        ("foreign-state", "", "<ChildObjects/>", "<xr:PropertyState xmlns:xr=\"urn:foreign\"><xr:Property>ObjectModule</xr:Property><xr:State>Extended</xr:State></xr:PropertyState>", true, false),
        ("reversed-state", "", "<ChildObjects/>", "<xr:PropertyState><xr:State>Extended</xr:State><xr:Property>ObjectModule</xr:Property></xr:PropertyState>", true, false),
        ("conflicting-state", "", "<ChildObjects/>", "<xr:PropertyState><xr:Property>ObjectModule</xr:Property><xr:State>Notify</xr:State><xr:State>Extended</xr:State></xr:PropertyState>", true, false),
        ("duplicate-state", "", "<ChildObjects/>", duplicate_state.as_str(), true, false),
        ("mixed-text", "", "<ChildObjects/>", "<xr:PropertyState>junk<xr:Property>ObjectModule</xr:Property><xr:State>Extended</xr:State></xr:PropertyState>", true, false),
        ("nested-property", "", "<ChildObjects/>", "<xr:PropertyState><xr:Property>ObjectModule<xr:Extra/></xr:Property><xr:State>Extended</xr:State></xr:PropertyState>", true, false),
        ("nested-state", "", "<ChildObjects/>", "<xr:PropertyState><xr:Property>ObjectModule</xr:Property><xr:State>Extended<xr:Extra/></xr:State></xr:PropertyState>", true, false),
    ];
    for (label, before, after, states, has_module, passed) in cases {
        let xml = format!(
            r#"<MetaDataObject xmlns="{MD}" xmlns:xr="{XR}" version="2.20"><Report uuid="77777777-7777-4777-8777-777777777777"><InternalInfo>{states}</InternalInfo>{before}<Properties><ObjectBelonging>Adopted</ObjectBelonging><Name>PriceList</Name><Comment/><ExtendedConfigurationObject>88888888-8888-4888-8888-888888888888</ExtendedConfigurationObject></Properties>{after}</Report></MetaDataObject>"#
        );
        fs::write(&descriptor, &xml).unwrap();
        if has_module {
            fs::write(&module, "// connected module\n").unwrap();
        } else if module.exists() {
            fs::remove_file(&module).unwrap();
        }
        let result = call(&mut mcp, "unica.check", json!({"at":"ext:Configuration"}));
        let data = &result["data"];
        assert_eq!(data["validators"], json!(["cfe"]), "{label}: {result:#}");
        assert_eq!(
            data["status"],
            if passed { "passed" } else { "failed" },
            "{label}: {result:#}"
        );
        if !passed {
            assert!(
                data["diagnostics"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|finding| finding["validator"] == "cfe"),
                "{label}: {result:#}"
            );
        }
        assert_eq!(
            fs::read_to_string(&descriptor).unwrap(),
            xml,
            "{label}: check changed XML"
        );
        assert_eq!(
            fs::read_to_string(source.join("Configuration.xml")).unwrap(),
            config
        );
        if has_module {
            assert_eq!(
                fs::read_to_string(&module).unwrap(),
                "// connected module\n"
            );
        } else {
            assert!(!module.exists());
        }
    }
    mcp.finish();
}

#[test]
fn canonical_stdio_borrow_refresh_preserves_module_and_identity() {
    let root = tempfile::tempdir().unwrap();
    let workspace = root.path().join("workspace");
    let parent = workspace.join("src");
    let extension = workspace.join("ext");
    fs::create_dir_all(&parent).unwrap();
    // Retain the existing support-marker directory, as in code-module fixtures.
    fs::create_dir_all(extension.join("Ext")).unwrap();
    fs::write(workspace.join("v8project.yaml"), "format: DESIGNER\nsource-set:\n  - name: parent\n    type: CONFIGURATION\n    path: src\n  - name: ext\n    type: EXTENSION\n    path: ext\n").unwrap();
    fs::write(
        parent.join("Configuration.xml"),
        configuration(
            "<Report>PriceList</Report><DataProcessor>PriceList</DataProcessor><Catalog>PriceList</Catalog>",
            false,
        ),
    )
    .unwrap();
    fs::write(extension.join("Configuration.xml"), configuration("", true)).unwrap();
    let mut mcp = start(&workspace, &root.path().join("state"));
    for (kind, directory) in [
        ("Report", "Reports"),
        ("DataProcessor", "DataProcessors"),
        ("Catalog", "Catalogs"),
    ] {
        fs::create_dir_all(parent.join(directory)).unwrap();
        let parent_path = parent.join(directory).join("PriceList.xml");
        let controls = if kind == "Catalog" {
            "<CodeLength>3</CodeLength><Hierarchical>false</Hierarchical>"
        } else {
            ""
        };
        let parent_xml = format!(
            r#"<MetaDataObject xmlns="{MD}" version="2.20"><{kind} uuid="88888888-8888-4888-8888-888888888888"><InternalInfo/><Properties><Name>PriceList</Name><Comment/>{controls}</Properties><ChildObjects/></{kind}></MetaDataObject>"#
        );
        fs::write(&parent_path, &parent_xml).unwrap();
        let descriptor = extension.join(directory).join("PriceList.xml");
        let module = extension
            .join(directory)
            .join("PriceList/Ext/ObjectModule.bsl");
        let owner_before = fs::read(extension.join("Configuration.xml")).unwrap();
        let mut borrow_args = json!({"at":"ext:Configuration","dryRun":true,"ops":[{"op":"object.borrow","args":{"at":"ext:Configuration","from":format!("parent:{kind}.PriceList")}}]});
        let preview = call(&mut mcp, "unica.apply", borrow_args.clone());
        assert!(!descriptor.exists());
        assert!(!module.exists());
        assert_eq!(
            fs::read(extension.join("Configuration.xml")).unwrap(),
            owner_before
        );
        borrow_args["dryRun"] = json!(false);
        borrow_args["ifRev"] = preview["rev"].clone();
        call(&mut mcp, "unica.apply", borrow_args.clone());
        let borrowed = fs::read_to_string(&descriptor).unwrap();
        let doc = roxmltree::Document::parse(&borrowed).unwrap();
        let object = doc
            .root_element()
            .children()
            .find(|node| node.has_tag_name((MD, kind)))
            .unwrap();
        let uuid = object.attribute("uuid").unwrap().to_owned();
        assert_ne!(uuid, "88888888-8888-4888-8888-888888888888");
        let children: Vec<_> = object
            .children()
            .filter(|node| node.has_tag_name((MD, "ChildObjects")))
            .collect();
        assert_eq!(
            children.len(),
            1,
            "{kind}: missing mandatory empty ChildObjects: {borrowed}"
        );
        assert!(!children[0].children().any(|node| node.is_element()));
        let properties = object
            .children()
            .find(|node| node.has_tag_name((MD, "Properties")))
            .unwrap();
        assert!(children[0].range().start > properties.range().end);
        // code.insert creates a module leaf under an existing directory; it
        // does not own source-tree topology (staged_code_rejects_absent_leaf_below_missing_parent_topology).
        fs::create_dir_all(module.parent().unwrap()).unwrap();
        let at = format!("ext:{kind}.PriceList.Module.Object");
        let mut code_args = json!({"at":at,"dryRun":true,"ops":[{"op":"code.insert","args":{"at":at,"text":"Procedure Added() Export\nEndProcedure"}}]});
        let preview = call(&mut mcp, "unica.apply", code_args.clone());
        assert_eq!(fs::read_to_string(&descriptor).unwrap(), borrowed);
        assert!(!module.exists());
        code_args["dryRun"] = json!(false);
        code_args["ifRev"] = preview["rev"].clone();
        call(&mut mcp, "unica.apply", code_args);
        let connected = fs::read_to_string(&descriptor).unwrap();
        let doc = roxmltree::Document::parse(&connected).unwrap();
        let states: Vec<_> = doc
            .descendants()
            .filter(|node| node.has_tag_name((XR, "PropertyState")))
            .collect();
        assert_eq!(states.len(), 1, "{connected}");
        assert!(
            states[0]
                .children()
                .any(|node| node.has_tag_name((XR, "Property"))
                    && node.text() == Some("ObjectModule"))
        );
        assert!(states[0]
            .children()
            .any(|node| node.has_tag_name((XR, "State")) && node.text() == Some("Extended")));
        let bsl = fs::read(&module).unwrap();
        assert!(String::from_utf8_lossy(&bsl).contains("Procedure Added()"));
        // Catalog exercises an actual controlled-property refresh; the other
        // objects prove that an extension-owned Comment is not overwritten.
        let mut expected_refresh = connected.clone();
        let changed_parent = if kind == "Catalog" {
            let code_length = doc
                .descendants()
                .find(|node| node.has_tag_name((MD, "CodeLength")))
                .unwrap();
            assert_eq!(code_length.text(), Some("3"));
            let value = code_length.children().find(|node| node.is_text()).unwrap();
            expected_refresh.replace_range(value.range(), "5");
            parent_xml.replace("<CodeLength>3</CodeLength>", "<CodeLength>5</CodeLength>")
        } else {
            parent_xml.replace("<Comment/>", "<Comment>parent-only change</Comment>")
        };
        fs::write(&parent_path, changed_parent).unwrap();
        borrow_args["dryRun"] = json!(true);
        borrow_args.as_object_mut().unwrap().remove("ifRev");
        let repeated_preview = call(&mut mcp, "unica.apply", borrow_args.clone());
        assert_eq!(fs::read_to_string(&descriptor).unwrap(), connected);
        assert_eq!(fs::read(&module).unwrap(), bsl);
        borrow_args["dryRun"] = json!(false);
        borrow_args["ifRev"] = repeated_preview["rev"].clone();
        let refreshed = call(&mut mcp, "unica.apply", borrow_args.clone());
        if kind == "Catalog" {
            assert_ne!(refreshed["rev"], repeated_preview["rev"]);
        } else {
            assert_eq!(refreshed["rev"], repeated_preview["rev"]);
        }
        assert_eq!(
            fs::read_to_string(&descriptor).unwrap(),
            expected_refresh,
            "refresh must change only the transferred property, keeping UUID, generated type IDs and module state bytes"
        );
        assert_eq!(fs::read(&module).unwrap(), bsl);
        // After applying the parent change, a true repeat has no new revision
        // and leaves both metadata and module bytes identical.
        borrow_args["dryRun"] = json!(true);
        borrow_args["ifRev"] = refreshed["rev"].clone();
        let noop_preview = call(&mut mcp, "unica.apply", borrow_args.clone());
        assert_eq!(noop_preview["rev"], refreshed["rev"]);
        assert_eq!(fs::read_to_string(&descriptor).unwrap(), expected_refresh);
        assert_eq!(fs::read(&module).unwrap(), bsl);
        borrow_args["dryRun"] = json!(false);
        borrow_args["ifRev"] = noop_preview["rev"].clone();
        let noop = call(&mut mcp, "unica.apply", borrow_args);
        assert_eq!(noop["rev"], refreshed["rev"]);
        assert_eq!(fs::read_to_string(&descriptor).unwrap(), expected_refresh);
        assert_eq!(fs::read(&module).unwrap(), bsl);
        let checked = call(&mut mcp, "unica.check", json!({"at":"ext:Configuration"}));
        assert_eq!(checked["data"]["status"], "passed", "{checked:#}");
        // The checker must also detect damage to a state created by the real
        // writer, retaining both the damaged descriptor and the BSL untouched.
        let state_range = roxmltree::Document::parse(&expected_refresh)
            .unwrap()
            .descendants()
            .find(|node| node.has_tag_name((XR, "PropertyState")))
            .unwrap()
            .range();
        let mut damaged = expected_refresh.clone();
        damaged.replace_range(state_range, "");
        fs::write(&descriptor, &damaged).unwrap();
        let rejected = call(&mut mcp, "unica.check", json!({"at":"ext:Configuration"}));
        assert_eq!(rejected["data"]["status"], "failed", "{rejected:#}");
        assert!(rejected["data"]["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .any(|finding| finding["validator"] == "cfe"));
        assert_eq!(fs::read_to_string(&descriptor).unwrap(), damaged);
        assert_eq!(fs::read(&module).unwrap(), bsl);
        fs::write(&descriptor, &expected_refresh).unwrap();
    }
    mcp.finish();
}
