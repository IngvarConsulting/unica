//! Public calls whose format gate must block before any handler runs. The
//! integration target runs them as tests; the format guard's own unit test
//! calls them as part of its proof that every native mutator enters the gate.
//! Each includer gives `super::application()`: the application on the
//! infrastructure ports.

use serde_json::{json, Map, Value};

pub(crate) fn incompatible_format_blocks_before_native_handler() {
    let root = test_workspace_root("unica-application-format-guard");
    let src = root.join("src");
    std::fs::create_dir_all(&src).unwrap();
    std::fs::write(
        root.join("v8project.yaml"),
        "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
    )
    .unwrap();
    let config = src.join("Configuration.xml");
    let before = r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.19"><Configuration/></MetaDataObject>"#;
    std::fs::write(&config, before).unwrap();
    let mut args = Map::new();
    args.insert("cwd".into(), Value::String(root.display().to_string()));
    args.insert(
        "ConfigPath".into(),
        Value::String(config.display().to_string()),
    );
    args.insert("dryRun".into(), Value::Bool(false));

    let result = super::application()
        .call_tool("unica.cf.edit", &args)
        .unwrap();

    assert!(!result.ok, "{result:?}");
    assert_eq!(
        result.diagnostics.as_ref().unwrap()["formatCompatibility"]["code"],
        "formatMigrationAvailable"
    );
    assert_eq!(std::fs::read_to_string(config).unwrap(), before);
    let _ = std::fs::remove_dir_all(root);
}

pub(crate) fn public_metadata_mutators_refuse_old_and_new_profiles_without_side_effects() {
    for version in ["2.19", "2.21"] {
        for tool in ["unica.meta.add", "unica.meta.edit"] {
            let root = test_workspace_root(&format!(
                "unica-meta-common-format-gate-{}-{version}",
                tool.replace('.', "-")
            ));
            let workspace = root.join("workspace");
            let source = workspace.join("src");
            std::fs::create_dir_all(source.join("Catalogs")).unwrap();
            std::fs::write(
                workspace.join("v8project.yaml"),
                "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
            )
            .unwrap();
            std::fs::write(
                source.join("Configuration.xml"),
                format!(
                    r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="{version}"><Configuration><Properties><Name>Test</Name></Properties><ChildObjects><Catalog>Items</Catalog></ChildObjects></Configuration></MetaDataObject>"#
                ),
            )
            .unwrap();
            std::fs::write(
                source.join("Catalogs/Items.xml"),
                support_test_catalog_xml("bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb"),
            )
            .unwrap();
            let before = unica_workspace::test_support::tree_snapshot(&source);
            let mut args = match tool {
                "unica.meta.add" => Map::from_iter([
                    ("sourceSet".to_string(), json!("main")),
                    ("kind".to_string(), json!("Catalog")),
                    ("name".to_string(), json!("Future")),
                ]),
                "unica.meta.edit" => Map::from_iter([
                    ("sourceSet".to_string(), json!("main")),
                    ("metadataPath".to_string(), json!("Catalog.Items")),
                    (
                        "operations".to_string(),
                        json!([{
                            "op": "setProperties",
                            "values": {"Comment": "changed"}
                        }]),
                    ),
                ]),
                _ => unreachable!(),
            };
            args.insert(
                "cwd".to_string(),
                Value::String(workspace.display().to_string()),
            );
            args.insert("dryRun".to_string(), Value::Bool(false));

            let result = super::application().call_tool(tool, &args).unwrap();

            assert!(!result.ok, "{tool}, version={version}: {result:?}");
            let diagnostic = result
                .diagnostics
                .as_ref()
                .and_then(Value::as_array)
                .and_then(|diagnostics| diagnostics.first())
                .expect("metadata refusal has one typed diagnostic");
            assert_eq!(diagnostic["code"], "capability_unavailable");
            assert_eq!(
                unica_workspace::test_support::tree_snapshot(&source),
                before,
                "{tool}, version={version} changed source bytes"
            );
            assert!(result.changes.is_empty(), "{tool}: {result:?}");
            assert!(result.artifacts.is_empty(), "{tool}: {result:?}");
            assert!(result.cache.events.is_empty(), "{tool}: {result:?}");
            std::fs::remove_dir_all(root).unwrap();
        }
    }
}

pub(crate) fn test_workspace_root(prefix: &str) -> std::path::PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "{prefix}-{}-{nanos}-{}",
        std::process::id(),
        uuid::Uuid::new_v4()
    ));
    std::fs::create_dir_all(&root).unwrap();
    root
}

pub(crate) fn support_test_catalog_xml(uuid: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" xmlns:v8="http://v8.1c.ru/8.1/data/core" version="2.20">
  <Catalog uuid="{uuid}">
<Properties>
  <Name>Items</Name>
  <Synonym><v8:item><v8:lang>ru</v8:lang><v8:content>Items</v8:content></v8:item></Synonym>
</Properties>
<ChildObjects/>
  </Catalog>
</MetaDataObject>"#
    )
}
