use super::McpProcess;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

fn snapshot(root: &Path) -> BTreeMap<PathBuf, Option<Vec<u8>>> {
    fn visit(root: &Path, dir: &Path, result: &mut BTreeMap<PathBuf, Option<Vec<u8>>>) {
        if !dir.exists() {
            return;
        }
        for entry in fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            let directory = path.is_dir();
            result.insert(
                path.strip_prefix(root).unwrap().to_path_buf(),
                if directory {
                    None
                } else {
                    Some(fs::read(&path).unwrap())
                },
            );
            if directory {
                visit(root, &path, result);
            }
        }
    }
    let mut result = BTreeMap::new();
    visit(root, root, &mut result);
    result
}

fn apply(mcp: &mut McpProcess, id: &mut u64, args: Value) -> Value {
    *id += 1;
    let response = mcp.exchange(json!({"jsonrpc":"2.0","id":*id,"method":"tools/call","params":{"name":"unica.apply","arguments":args}}));
    response["result"]["structuredContent"].clone()
}

fn configuration(name: &str) -> String {
    format!(
        r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Configuration uuid="11111111-1111-4111-8111-111111111111"><Properties><Name>{name}</Name><DefaultLanguage>Language.Русский</DefaultLanguage></Properties><ChildObjects><Language>Русский</Language><Catalog>Items</Catalog></ChildObjects></Configuration></MetaDataObject>"#
    )
}

fn configure(kind: &str, at: &str) -> Value {
    if kind == "Role" {
        json!({"op":"right.add","args":{"at":at,"items":[{"object":"Catalog.Items","rights":{"Read":true,"View":true}}]}})
    } else {
        json!({"op":"content.add","args":{"at":at,"items":[{"object":"Catalog.Items"}]}})
    }
}

fn create(kind: &str, raw: &str) -> Value {
    let op = if kind == "Role" {
        "role.create"
    } else {
        "subsystem.create"
    };
    json!({"op":op,"args":{"values":{"name":raw}}})
}

fn created_name_transaction(kind: &str) {
    let temp = tempfile::tempdir().unwrap();
    let workspace = temp.path().join("workspace");
    let state = temp.path().join("state");
    fs::create_dir_all(&state).unwrap();
    fs::create_dir_all(&workspace).unwrap();
    fs::write(workspace.join("v8project.yaml"), "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n  - name: other\n    type: CONFIGURATION\n    path: other\n").unwrap();
    for directory in ["src", "other"] {
        let source = workspace.join(directory);
        fs::create_dir_all(source.join("Catalogs")).unwrap();
        fs::create_dir_all(source.join("Languages")).unwrap();
        fs::write(source.join("Configuration.xml"), configuration(directory)).unwrap();
        fs::write(source.join("Languages/Русский.xml"), r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Language uuid="33333333-3333-4333-8333-333333333333"><Properties><Name>Русский</Name><LanguageCode>ru</LanguageCode></Properties></Language></MetaDataObject>"#).unwrap();
        fs::write(source.join("Catalogs/Items.xml"), r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Catalog uuid="22222222-2222-4222-8222-222222222222"><Properties><Name>Items</Name></Properties><ChildObjects/></Catalog></MetaDataObject>"#).unwrap();
    }
    let mut mcp = McpProcess::start(&workspace, &state);
    mcp.exchange(json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"creation-name-test","version":"1"}}}));
    mcp.notify(json!({"jsonrpc":"2.0","method":"notifications/initialized"}));
    let mut id = 1;
    for (source_set, directory, sibling) in [("main", "src", "other"), ("other", "other", "src")] {
        for (raw, name, configured) in [
            ("Canonical", "Canonical", true),
            (" Padded ", "Padded", true),
            ("\u{a0}Читатель\u{2003}", "Читатель", true),
            ("\u{2003}Standalone\u{a0}", "Standalone", false),
        ] {
            let at = format!("{source_set}:{kind}.{name}");
            let root_at = format!("{source_set}:Configuration");
            let ops = if configured {
                vec![create(kind, raw), configure(kind, &at)]
            } else {
                vec![create(kind, raw)]
            };
            let args = json!({"at":root_at,"ops":ops});
            let before = snapshot(&workspace);
            let sibling_before = snapshot(&workspace.join(sibling));
            let preview = apply(&mut mcp, &mut id, args.clone());
            assert_eq!(
                preview["ok"], true,
                "{kind} {raw:?} {source_set}: {preview}"
            );
            assert!(preview["data"]["executionToken"]
                .as_str()
                .is_some_and(|token| !token.is_empty()));
            assert_eq!(
                snapshot(&workspace),
                before,
                "preview changes source or cache"
            );
            let published = apply(
                &mut mcp,
                &mut id,
                json!({"executionToken":preview["data"]["executionToken"]}),
            );
            assert_eq!(published["ok"], true, "{published}");
            assert_eq!(published["data"]["planHash"], preview["data"]["planHash"]);
            let source = workspace.join(directory);
            let owner = fs::read_to_string(source.join("Configuration.xml")).unwrap();
            let owner = roxmltree::Document::parse(&owner).unwrap();
            assert_eq!(
                owner
                    .descendants()
                    .filter(|n| n.has_tag_name(kind) && n.text() == Some(name))
                    .count(),
                1
            );
            let descriptor = source.join(format!("{kind}s/{name}.xml"));
            let xml = fs::read_to_string(descriptor).unwrap();
            let doc = roxmltree::Document::parse(&xml).unwrap();
            assert!(doc
                .descendants()
                .any(|n| n.has_tag_name("Name") && n.text() == Some(name)));
            if configured && kind == "Role" {
                let rights =
                    fs::read_to_string(source.join(format!("Roles/{name}/Ext/Rights.xml")))
                        .unwrap();
                let doc = roxmltree::Document::parse(&rights).unwrap();
                let object = doc
                    .descendants()
                    .find(|n| {
                        n.has_tag_name("object")
                            && n.children().any(|c| {
                                c.has_tag_name("name") && c.text() == Some("Catalog.Items")
                            })
                    })
                    .unwrap();
                for right in ["Read", "View"] {
                    assert!(object
                        .children()
                        .filter(|n| n.has_tag_name("right"))
                        .any(|n| n
                            .children()
                            .any(|c| c.has_tag_name("name") && c.text() == Some(right))
                            && n.children()
                                .any(|c| c.has_tag_name("value") && c.text() == Some("true"))));
                }
            } else if configured {
                assert!(doc
                    .descendants()
                    .any(|n| n.has_tag_name("Item") && n.text() == Some("Catalog.Items")));
            }
            assert!(!source.join(format!("{kind}s/{raw}.xml")).exists() || raw == name);
            assert_eq!(snapshot(&workspace.join(sibling)), sibling_before);
            // A failed creation cannot authorize changes to the existing object.
            let before = snapshot(&workspace);
            let collision = apply(&mut mcp, &mut id, args);
            assert_eq!(collision["ok"], false, "{collision}");
            assert_eq!(collision["at"], "ops[0].args.values.name", "{collision}");
            assert_eq!(collision["diagnostics"][0]["code"], "invalid_state");
            assert_eq!(snapshot(&workspace), before);
        }
        let root_at = format!("{source_set}:Configuration");
        for raw in ["", "   ", "Bad Name", "Bad.Name", "Bad/Name", "Bad-Name"] {
            let before = snapshot(&workspace);
            let refused = apply(
                &mut mcp,
                &mut id,
                json!({"at":root_at,"ops":[create(kind,raw)]}),
            );
            assert_eq!(refused["ok"], false, "{raw}: {refused}");
            assert_eq!(snapshot(&workspace), before);
        }
        let before = snapshot(&workspace);
        let early = apply(
            &mut mcp,
            &mut id,
            json!({"at":root_at,"ops":[configure(kind,&format!("{source_set}:{kind}.Early")),create(kind," Early ")]}),
        );
        assert_eq!(early["ok"], false, "{early}");
        assert_eq!(snapshot(&workspace), before);
        let before = snapshot(&workspace);
        let strict = apply(
            &mut mcp,
            &mut id,
            json!({"at":root_at,"ops":[{"op":"object.create","args":{"values":{"kind":"Constant","name":" Strict "}}}]}),
        );
        assert_eq!(strict["ok"], false, "{strict}");
        assert_eq!(snapshot(&workspace), before);
        let failed_at = format!("{source_set}:{kind}.Failed");
        let late = create(kind, "Failed");
        let before = snapshot(&workspace);
        let refused = apply(
            &mut mcp,
            &mut id,
            json!({"at":root_at,"ops":[create(kind," Failed "),configure(kind,&failed_at),late]}),
        );
        assert_eq!(refused["ok"], false, "{refused}");
        assert_eq!(refused["at"], "ops[2].args.values.name", "{refused}");
        assert_eq!(refused["diagnostics"][0]["code"], "invalid_state");
        assert_eq!(
            snapshot(&workspace),
            before,
            "a later failure publishes no files or cache"
        );
        for target in [
            format!("{source_set}:{kind}.Padded"),
            format!(
                "{}:{kind}.New",
                if source_set == "main" {
                    "other"
                } else {
                    "main"
                }
            ),
        ] {
            let before = snapshot(&workspace);
            let refused = apply(
                &mut mcp,
                &mut id,
                json!({"at":root_at,"ops":[create(kind," New "),configure(kind,&target)]}),
            );
            assert_eq!(refused["ok"], false, "{refused}");
            assert_eq!(snapshot(&workspace), before);
        }
    }
    mcp.finish();
}

#[test]
fn canonical_stdio_role_creation_name_matches_batch_scope() {
    created_name_transaction("Role");
}
#[test]
fn canonical_stdio_subsystem_creation_name_matches_batch_scope() {
    created_name_transaction("Subsystem");
}
