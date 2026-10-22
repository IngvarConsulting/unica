use super::*;

const VENDOR_UUID: &str = "11111111-1111-4111-8111-111111111111";

/// A configuration on vendor support whose `Document.First` carries the
/// given support rule, under the given project support policy.
fn supported_workspace(policy: &str, rule: u8, editing_enabled: bool) -> tempfile::TempDir {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("src");
    std::fs::create_dir_all(source.join("Documents")).unwrap();
    std::fs::create_dir_all(source.join("Languages")).unwrap();
    std::fs::create_dir_all(source.join("Ext")).unwrap();
    std::fs::write(
        root.path().join("v8project.yaml"),
        "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
    )
    .unwrap();
    std::fs::write(
        root.path().join(".v8-project.json"),
        json!({"editingAllowedCheck": policy}).to_string(),
    )
    .unwrap();
    std::fs::write(
        source.join("Configuration.xml"),
        r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Configuration><Properties><Name>Main</Name><DefaultLanguage>Language.Русский</DefaultLanguage></Properties><ChildObjects><Language>Русский</Language><Document>First</Document></ChildObjects></Configuration></MetaDataObject>"#,
    )
    .unwrap();
    std::fs::write(
        source.join("Languages/Русский.xml"),
        r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Language uuid="33333333-3333-4333-8333-333333333333"><Properties><Name>Русский</Name><Comment/><LanguageCode>ru</LanguageCode></Properties></Language></MetaDataObject>"#,
    )
    .unwrap();
    std::fs::write(
        source.join("Documents/First.xml"),
        format!(
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" xmlns:v8="http://v8.1c.ru/8.1/data/core" version="2.20"><Document uuid="{VENDOR_UUID}"><Properties><Name>First</Name><Synonym/><Comment/></Properties><ChildObjects/></Document></MetaDataObject>"#
        ),
    )
    .unwrap();
    let global_flag = u8::from(!editing_enabled);
    std::fs::write(
        source.join("Ext/ParentConfigurations.bin"),
        format!(
            "\u{feff}{{6,{global_flag},1,dddddddd-dddd-4ddd-8ddd-dddddddddddd,0,33333333-3333-4333-8333-333333333333,\"1.0\",\"Vendor\",\"VendorConf\",3,{rule},0,{VENDOR_UUID},{VENDOR_UUID}}}"
        ),
    )
    .unwrap();
    root
}

fn support_warnings(result: &Value) -> Vec<Value> {
    result["warnings"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|warning| warning["code"] == "support_locked")
        .cloned()
        .collect()
}

/// `warn` permits editing a locked vendor object and says so in the plan and
/// in the published result; `off` permits it silently; `deny` refuses it.
/// Both lock sources count: the object rule `locked` and a configuration
/// whose vendor support disables changes. An `editable` vendor object is not
/// locked: every policy edits it without a warning.
#[test]
fn public_apply_on_a_locked_vendor_object_follows_the_support_policy() {
    for (rule, editing_enabled) in [(0, true), (2, false), (1, true)] {
        let locked = rule != 1;
        for policy in ["deny", "warn", "off"] {
            let case = format!("{policy}, rule {rule}, editing enabled {editing_enabled}");
            let root = supported_workspace(policy, rule, editing_enabled);
            let descriptor = root.path().join("src/Documents/First.xml");
            let original = std::fs::read(&descriptor).unwrap();
            let mut mcp = McpProcess::start(root.path());
            initialize_body_mcp(&mut mcp);
            let plan = mcp.completed_tool_call(call_tool(
                2,
                "unica.apply",
                json!({
                    "at": "main:Document.First",
                    "ops": [{"op": "props.set", "args": {"values": {"Comment": "vendor edit"}}}]
                }),
            ));
            assert_eq!(
                std::fs::read(&descriptor).unwrap(),
                original,
                "{case}: plan writes nothing"
            );
            if policy == "deny" && locked {
                assert_eq!(plan["ok"], false, "{case}: {plan:#}");
                assert!(
                    plan.to_string().contains("supportRule.set")
                        || plan.to_string().contains("supportCapability.set"),
                    "{case}: {plan:#}"
                );
                mcp.finish();
                continue;
            }
            assert_eq!(plan["ok"], true, "{case}: {plan:#}");
            let expected = if policy == "warn" && locked {
                vec![json!("main:Document.First")]
            } else {
                Vec::new()
            };
            let planned = support_warnings(&plan);
            assert_eq!(
                planned
                    .iter()
                    .map(|warning| warning["at"].clone())
                    .collect::<Vec<_>>(),
                expected,
                "{case}: {plan:#}"
            );
            for warning in &planned {
                assert_eq!(warning["severity"], "warning", "{case}: {warning:#}");
                assert!(
                    warning["message"].as_str().unwrap().contains("`warn`"),
                    "{case}: {warning:#}"
                );
            }
            let executed = mcp.completed_tool_call(call_tool(
                3,
                "unica.apply",
                json!({"executionToken": plan["data"]["executionToken"]}),
            ));
            assert_eq!(executed["ok"], true, "{case}: {executed:#}");
            assert_eq!(
                executed["data"]["mode"], "published",
                "{case}: {executed:#}"
            );
            assert_eq!(support_warnings(&executed), planned, "{case}: {executed:#}");
            assert!(
                std::fs::read_to_string(&descriptor)
                    .unwrap()
                    .contains("<Comment>vendor edit</Comment>"),
                "{case}: the edit is published"
            );
            mcp.finish();
        }
    }
}
