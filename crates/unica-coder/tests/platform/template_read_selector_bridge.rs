use super::*;
use crate::infrastructure::native_operations::dcs::{analyze_dcs_info_with_data, validate_dcs};
use serde_json::json;
use std::sync::atomic::{AtomicU64, Ordering};

static NONCE: AtomicU64 = AtomicU64::new(0);

/// A report template reachable both ways, plus a binary common template
/// whose descriptor exists and whose XML body does not.
pub(crate) fn workspace(name: &str) -> WorkspaceContext {
    let root = std::env::temp_dir().join(format!(
        "unica-template-read-bridge-{name}-{}-{}",
        std::process::id(),
        NONCE.fetch_add(1, Ordering::Relaxed)
    ));
    let src = root.join("src");
    fs::create_dir_all(src.join("Reports/Sales/Templates/Main/Ext")).unwrap();
    fs::create_dir_all(src.join("CommonTemplates/Logo/Ext")).unwrap();
    fs::write(
        root.join("v8project.yaml"),
        "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
    )
    .unwrap();
    fs::write(
            src.join("Configuration.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Configuration><Properties><Name>Demo</Name></Properties><ChildObjects><Report>Sales</Report><CommonTemplate>Logo</CommonTemplate></ChildObjects></Configuration></MetaDataObject>"#,
        )
        .unwrap();
    fs::write(
            src.join("Reports/Sales.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Report><Properties><Name>Sales</Name></Properties><ChildObjects><Template>Main</Template><Template>Dcs</Template></ChildObjects></Report></MetaDataObject>"#,
        )
        .unwrap();
    fs::write(
            src.join("Reports/Sales/Templates/Main.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Template><Properties><Name>Main</Name><TemplateType>SpreadsheetDocument</TemplateType></Properties></Template></MetaDataObject>"#,
        )
        .unwrap();
    fs::write(
            src.join("Reports/Sales/Templates/Main/Ext/Template.xml"),
            r#"<?xml version="1.0" encoding="UTF-8"?><document xmlns="http://v8.1c.ru/8.2/data/spreadsheet" xmlns:v8="http://v8.1c.ru/8.1/data/core"><columnsID>0</columnsID><format><formatIndex>0</formatIndex></format></document>"#,
        )
        .unwrap();
    fs::write(
            src.join("CommonTemplates/Logo.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><CommonTemplate><Properties><Name>Logo</Name><TemplateType>BinaryData</TemplateType></Properties></CommonTemplate></MetaDataObject>"#,
        )
        .unwrap();
    fs::write(src.join("CommonTemplates/Logo/Ext/Template.bin"), [0u8, 1]).unwrap();
    // The DCS readers parse a composition schema, not a spreadsheet. Giving
    // them the MXL body would make both sides of a parity test fail the
    // same way and compare two errors as if they were an answer.
    fs::create_dir_all(src.join("Reports/Sales/Templates/Dcs/Ext")).unwrap();
    fs::write(
            src.join("Reports/Sales/Templates/Dcs.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Template><Properties><Name>Dcs</Name><TemplateType>DataCompositionSchema</TemplateType></Properties></Template></MetaDataObject>"#,
        )
        .unwrap();
    fs::write(
            src.join("Reports/Sales/Templates/Dcs/Ext/Template.xml"),
            r#"<?xml version="1.0" encoding="UTF-8"?><DataCompositionSchema xmlns="http://v8.1c.ru/8.1/data-composition-system/schema" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance"><dataSource><name>ИсточникДанных1</name><dataSourceType>Local</dataSourceType></dataSource><dataSet xsi:type="DataSetQuery"><name>НаборДанных1</name><field xsi:type="DataSetFieldField"><dataPath>Код</dataPath><field>Код</field></field><dataSource>ИсточникДанных1</dataSource><query>ВЫБРАТЬ Код ИЗ Справочник.Товары</query></dataSet></DataCompositionSchema>"#,
        )
        .unwrap();
    WorkspaceContext {
        cwd: root.clone(),
        workspace_root: root.clone(),
        cache_root: root.join(".build/unica"),
        workspace_epoch: 1,
    }
}

pub(crate) fn physical_args() -> Map<String, Value> {
    Map::from_iter([(
        "TemplatePath".to_string(),
        json!("src/Reports/Sales/Templates/Main/Ext/Template.xml"),
    )])
}

pub(crate) fn logical_args() -> Map<String, Value> {
    Map::from_iter([
        ("sourceSet".to_string(), json!("main")),
        (
            "metadataPath".to_string(),
            json!("Report.Sales.Template.Main"),
        ),
    ])
}

#[test]
pub(crate) fn mxl_info_answers_identically_for_a_logical_and_a_physical_selector() {
    let context = workspace("mxl-info");

    let support_reader = WorkspaceSupportStateReader::new(&context);
    let physical = analyze_mxl_info(&physical_args(), &context, &support_reader);
    let logical = analyze_mxl_info(&logical_args(), &context, &support_reader);

    assert!(logical.outcome.ok, "{:?}", logical.outcome);
    assert_eq!(
        serde_json::to_value(&logical.data).unwrap(),
        serde_json::to_value(&physical.data).unwrap()
    );
    let _ = fs::remove_dir_all(&context.workspace_root);
}

#[test]
pub(crate) fn mxl_validate_answers_identically_for_a_logical_and_a_physical_selector() {
    let context = workspace("mxl-validate");

    let physical = validate_mxl(&physical_args(), &context);
    let logical = validate_mxl(&logical_args(), &context);

    assert_eq!(logical.ok, physical.ok, "{logical:?} vs {physical:?}");
    assert_eq!(logical.errors, physical.errors);
    let _ = fs::remove_dir_all(&context.workspace_root);
}

fn dcs_physical_args() -> Map<String, Value> {
    Map::from_iter([(
        "TemplatePath".to_string(),
        json!("src/Reports/Sales/Templates/Dcs/Ext/Template.xml"),
    )])
}

fn dcs_logical_args() -> Map<String, Value> {
    Map::from_iter([
        ("sourceSet".to_string(), json!("main")),
        (
            "metadataPath".to_string(),
            json!("Report.Sales.Template.Dcs"),
        ),
    ])
}

#[test]
pub(crate) fn dcs_info_answers_identically_for_a_logical_and_a_physical_selector() {
    let context = workspace("dcs-info");

    let support_reader = WorkspaceSupportStateReader::new(&context);
    let physical = analyze_dcs_info_with_data(&dcs_physical_args(), &context, &support_reader);
    let logical = analyze_dcs_info_with_data(&dcs_logical_args(), &context, &support_reader);

    // Both must succeed: comparing two identical failures would pass while
    // proving nothing about the bridge.
    assert!(physical.outcome.ok, "{:?}", physical.outcome);
    assert!(logical.outcome.ok, "{:?}", logical.outcome);
    assert!(
        logical.data.is_some(),
        "a successful read answers with data"
    );
    assert_eq!(
        serde_json::to_value(&logical.data).unwrap(),
        serde_json::to_value(&physical.data).unwrap()
    );
    let _ = fs::remove_dir_all(&context.workspace_root);
}

#[test]
pub(crate) fn dcs_validate_answers_identically_for_a_logical_and_a_physical_selector() {
    let context = workspace("dcs-validate");

    let physical = validate_dcs(&dcs_physical_args(), &context);
    let logical = validate_dcs(&dcs_logical_args(), &context);

    assert!(physical.ok, "{physical:?}");
    assert_eq!(logical.ok, physical.ok, "{logical:?} vs {physical:?}");
    assert_eq!(logical.errors, physical.errors);
    let _ = fs::remove_dir_all(&context.workspace_root);
}

/// A `TemplateType` other than an XML one writes `Template.bin`. The object
/// is registered and addressable, so "not found" would be the wrong answer:
/// the target exists, this body does not.
#[test]
fn dcs_info_reports_a_binary_template_as_an_absent_resource_not_a_missing_target() {
    let context = workspace("dcs-binary");

    let outcome = analyze_dcs_info_with_data(
        &Map::from_iter([
            ("sourceSet".to_string(), json!("main")),
            ("metadataPath".to_string(), json!("CommonTemplate.Logo")),
        ]),
        &context,
        &WorkspaceSupportStateReader::new(&context),
    )
    .outcome;

    assert!(!outcome.ok);
    let text = outcome
        .errors
        .iter()
        .cloned()
        .chain(std::iter::once(outcome.summary.clone()))
        .collect::<Vec<_>>()
        .join(" | ");
    assert!(text.contains("resource_absent"), "{text}");
    assert!(!text.contains("target_not_found"), "{text}");
    let _ = fs::remove_dir_all(&context.workspace_root);
}
