use super::*;
use serde_json::json;
use std::sync::atomic::{AtomicU64, Ordering};

static NONCE: AtomicU64 = AtomicU64::new(0);

/// A catalog form and a common form, both registered so both are
/// addressable, and both reachable by their `Ext/Form.xml` path.
fn workspace(name: &str) -> WorkspaceContext {
    let root = std::env::temp_dir().join(format!(
        "unica-form-read-bridge-{name}-{}-{}",
        std::process::id(),
        NONCE.fetch_add(1, Ordering::Relaxed)
    ));
    let src = root.join("src");
    fs::create_dir_all(src.join("Catalogs/Items/Forms/Order/Ext")).unwrap();
    fs::create_dir_all(src.join("CommonForms/Settings/Ext")).unwrap();
    fs::write(
        root.join("v8project.yaml"),
        "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
    )
    .unwrap();
    fs::write(
            src.join("Configuration.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Configuration><Properties><Name>Demo</Name></Properties><ChildObjects><Catalog>Items</Catalog><CommonForm>Settings</CommonForm></ChildObjects></Configuration></MetaDataObject>"#,
        )
        .unwrap();
    fs::write(
            src.join("Catalogs/Items.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Catalog><Properties><Name>Items</Name></Properties><ChildObjects><Form>Order</Form></ChildObjects></Catalog></MetaDataObject>"#,
        )
        .unwrap();
    fs::write(
            src.join("Catalogs/Items/Forms/Order.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Form><Properties><Name>Order</Name></Properties></Form></MetaDataObject>"#,
        )
        .unwrap();
    fs::write(
            src.join("CommonForms/Settings.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><CommonForm><Properties><Name>Settings</Name></Properties></CommonForm></MetaDataObject>"#,
        )
        .unwrap();
    for relative in [
        "Catalogs/Items/Forms/Order/Ext/Form.xml",
        "CommonForms/Settings/Ext/Form.xml",
    ] {
        fs::write(
                src.join(relative),
                r#"<?xml version="1.0" encoding="UTF-8"?><Form xmlns="http://v8.1c.ru/8.3/xcf/logform" xmlns:xr="http://v8.1c.ru/8.3/xcf/readable"><AutoCommandBar name="ФормаКоманднаяПанель" id="-1"/></Form>"#,
            )
            .unwrap();
    }
    WorkspaceContext {
        cwd: root.clone(),
        workspace_root: root.clone(),
        cache_root: root.join(".build/unica"),
        workspace_epoch: 1,
    }
}

#[test]
pub(crate) fn form_info_answers_identically_for_a_logical_and_a_physical_selector() {
    let context = workspace("info");

    let physical = analyze_form_info_with_data(
        &Map::from_iter([(
            "FormPath".to_string(),
            json!("src/Catalogs/Items/Forms/Order/Ext/Form.xml"),
        )]),
        &context,
        &WorkspaceSupportStateReader::new(&context),
    );
    let logical = analyze_form_info_with_data(
        &Map::from_iter([
            ("sourceSet".to_string(), json!("main")),
            (
                "metadataPath".to_string(),
                json!("Catalog.Items.Form.Order"),
            ),
        ]),
        &context,
        &WorkspaceSupportStateReader::new(&context),
    );

    assert!(logical.outcome.ok, "{:?}", logical.outcome);
    assert_eq!(
        serde_json::to_value(&logical.data).unwrap(),
        serde_json::to_value(&physical.data).unwrap()
    );
    let _ = fs::remove_dir_all(&context.workspace_root);
}

#[test]
fn form_info_reads_a_common_form_by_its_top_level_address() {
    let context = workspace("common-form");

    let logical = analyze_form_info_with_data(
        &Map::from_iter([
            ("sourceSet".to_string(), json!("main")),
            ("metadataPath".to_string(), json!("CommonForm.Settings")),
        ]),
        &context,
        &WorkspaceSupportStateReader::new(&context),
    );

    assert!(logical.outcome.ok, "{:?}", logical.outcome);
    let _ = fs::remove_dir_all(&context.workspace_root);
}

#[test]
pub(crate) fn form_validate_answers_identically_for_a_logical_and_a_physical_selector() {
    let context = workspace("validate");

    let physical = validate_form(
        &Map::from_iter([(
            "FormPath".to_string(),
            json!("src/Catalogs/Items/Forms/Order/Ext/Form.xml"),
        )]),
        &context,
    );
    let logical = validate_form(
        &Map::from_iter([
            ("sourceSet".to_string(), json!("main")),
            (
                "metadataPath".to_string(),
                json!("Catalog.Items.Form.Order"),
            ),
        ]),
        &context,
    );

    assert_eq!(logical.ok, physical.ok, "{logical:?} vs {physical:?}");
    assert_eq!(logical.errors, physical.errors);
    let _ = fs::remove_dir_all(&context.workspace_root);
}
