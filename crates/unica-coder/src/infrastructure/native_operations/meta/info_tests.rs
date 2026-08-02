#![allow(dead_code, unused_imports)]

use super::internal::*;
use std::time::{SystemTime, UNIX_EPOCH};

fn workspace(name: &str) -> WorkspaceContext {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!("unica-meta-info-logical-{name}-{nanos}"));
    fs::create_dir_all(root.join("src/Catalogs/Items/Ext")).unwrap();
    fs::write(
        root.join("v8project.yaml"),
        "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
    )
    .unwrap();
    fs::write(
            root.join("src/Configuration.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Configuration/></MetaDataObject>"#,
        )
        .unwrap();
    fs::write(
            root.join("src/Catalogs/Items.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Catalog><Properties><Name>Items</Name><Synonym><v8:item xmlns:v8="http://v8.1c.ru/8.1/data/core"><v8:lang>ru</v8:lang><v8:content>Номенклатура</v8:content></v8:item></Synonym></Properties></Catalog></MetaDataObject>"#,
        )
        .unwrap();
    fs::write(
        root.join("src/Catalogs/Items/Ext/ObjectModule.bsl"),
        "Procedure BeforeWrite()\nEndProcedure\n",
    )
    .unwrap();
    WorkspaceContext {
        cwd: root.clone(),
        workspace_root: root.clone(),
        cache_root: root.join(".build/unica"),
        workspace_epoch: 1,
    }
}

fn info_args(address: &str) -> Map<String, Value> {
    Map::from_iter([
        ("sourceSet".to_string(), json!("main")),
        ("metadataPath".to_string(), json!(address)),
    ])
}

#[test]
fn meta_info_reads_the_descriptor_named_by_a_logical_address() {
    let context = workspace("reads");

    let execution = analyze_meta_info_with_data(&info_args("Catalog.Items"), &context);

    assert!(execution.outcome.ok, "{:?}", execution.outcome);
    let data = execution.data.expect("a resolved target is reported");
    assert_eq!(data.kind, "Catalog");
    assert_eq!(data.name, "Items");
    assert_eq!(data.target.source_set, "main");
    assert_eq!(
        data.target.metadata_path.as_ref().map(|path| path.as_str()),
        Some("Catalog.Items")
    );
    let _ = fs::remove_dir_all(&context.workspace_root);
}

/// The profile accepts a Russian kind alias and answers with the canonical
/// English address, so the answer can be fed back to any logical tool.
#[test]
fn meta_info_accepts_a_russian_kind_alias_and_answers_with_the_canonical_address() {
    let context = workspace("alias");

    let execution = analyze_meta_info_with_data(&info_args("Справочник.Items"), &context);

    assert!(execution.outcome.ok, "{:?}", execution.outcome);
    assert_eq!(
        execution
            .data
            .and_then(|data| data.target.metadata_path)
            .map(|path| path.as_str().to_string()),
        Some("Catalog.Items".to_string())
    );
    let _ = fs::remove_dir_all(&context.workspace_root);
}

/// Subordination is the structural fact that separates one catalog from
/// another; reading it required opening the raw XML before.
#[test]
fn meta_info_reports_owners_and_their_absence() {
    let context = workspace("owners");
    let subordinate = context.workspace_root.join("src/Catalogs/Series.xml");
    fs::write(
            &subordinate,
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" xmlns:xr="http://v8.1c.ru/8.3/xcf/readable" version="2.20"><Catalog><Properties><Name>Series</Name><Owners><xr:Item>Catalog.Items</xr:Item><xr:Item>Catalog.Kinds</xr:Item></Owners></Properties></Catalog></MetaDataObject>"#,
        )
        .unwrap();
    fs::write(
            context.workspace_root.join("src/Catalogs/Plain.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Catalog><Properties><Name>Plain</Name><Owners/></Properties></Catalog></MetaDataObject>"#,
        )
        .unwrap();

    let subordinate = analyze_meta_info_with_data(&info_args("Catalog.Series"), &context);
    let plain = analyze_meta_info_with_data(&info_args("Catalog.Plain"), &context);

    assert!(subordinate.outcome.ok, "{:?}", subordinate.outcome);
    assert_eq!(
        subordinate
            .data
            .expect("meta.info answers with data")
            .owners,
        vec!["Catalog.Items".to_string(), "Catalog.Kinds".to_string()]
    );
    assert!(plain.outcome.ok, "{:?}", plain.outcome);
    // An empty list is the answer: the catalog is not subordinate.
    assert!(plain
        .data
        .expect("meta.info answers with data")
        .owners
        .is_empty());
    let _ = fs::remove_dir_all(&context.workspace_root);
}

/// A silent property cannot be told apart from an unreported one, which is
/// what forced a reader to the XML to learn a catalog is flat.
#[test]
fn meta_info_states_catalog_properties_including_their_negatives() {
    let context = workspace("catalog-properties");
    fs::write(
            context.workspace_root.join("src/Catalogs/Flat.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Catalog><Properties><Name>Flat</Name><Hierarchical>false</Hierarchical><CodeLength>0</CodeLength><DescriptionLength>150</DescriptionLength><DefaultPresentation>AsCode</DefaultPresentation></Properties><ChildObjects><Form>ФормаЭлемента</Form></ChildObjects></Catalog></MetaDataObject>"#,
        )
        .unwrap();

    let execution = analyze_meta_info_with_data(&info_args("Catalog.Flat"), &context);

    assert!(execution.outcome.ok, "{:?}", execution.outcome);
    let data = execution.data.expect("meta.info answers with data");
    // Properties keep the platform's own names and values, so a negative is
    // stated rather than left out.
    let property = |name: &str| {
        data.properties
            .iter()
            .find(|property| property.name == name)
            .map(|property| property.value.as_str())
    };
    assert_eq!(property("Hierarchical"), Some("false"), "{data:?}");
    assert_eq!(property("CodeLength"), Some("0"), "{data:?}");
    assert_eq!(property("DescriptionLength"), Some("150"), "{data:?}");
    assert_eq!(property("DefaultPresentation"), Some("AsCode"), "{data:?}");
    // Forms used to appear in overview only for reports and data processors.
    assert_eq!(data.forms, vec!["ФормаЭлемента".to_string()]);
    let _ = fs::remove_dir_all(&context.workspace_root);
}

/// Reading a module is `unica.code.*` work. Quietly reading the owner
/// instead would answer a question the caller did not ask.
#[test]
fn meta_info_refuses_a_module_terminal_by_name() {
    let context = workspace("module");

    let outcome = analyze_meta_info(&info_args("Catalog.Items.ObjectModule"), &context);

    assert!(!outcome.ok);
    assert!(
        outcome.errors[0].contains("names a module terminal"),
        "{:?}",
        outcome.errors
    );
    let _ = fs::remove_dir_all(&context.workspace_root);
}

#[test]
fn meta_info_reports_an_unknown_address_without_naming_a_path() {
    let context = workspace("unknown");

    let outcome = analyze_meta_info(&info_args("Catalog.Missing"), &context);

    assert!(!outcome.ok);
    assert!(
        outcome.errors[0].contains("Catalog.Missing"),
        "{:?}",
        outcome.errors
    );
    assert!(
        !outcome.errors[0].contains("Catalogs/"),
        "{:?}",
        outcome.errors
    );
    let _ = fs::remove_dir_all(&context.workspace_root);
}

#[test]
fn meta_info_requires_a_source_set() {
    let context = workspace("no-source-set");

    let outcome = analyze_meta_info(
        &Map::from_iter([("metadataPath".to_string(), json!("Catalog.Items"))]),
        &context,
    );

    assert!(!outcome.ok);
    assert!(
        outcome.errors[0].contains("sourceSet"),
        "{:?}",
        outcome.errors
    );
    let _ = fs::remove_dir_all(&context.workspace_root);
}
