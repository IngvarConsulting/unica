use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
use unica_address::source_target::{MetadataAddress, PLATFORM_XML_8_3_27_FORMAT_2_20};
use unica_metadata::MetadataKind;
use unica_metadata_designer::usage_scan::{
    scan_local_enrichment as scan_local_enrichment_by_address, LocalEnrichment, LocalSection,
};
use unica_work_control::cancellation::CancellationToken;

fn temp_root(label: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    // The secure reader opens every path component without following links, and
    // the macOS temp directory sits behind `/var` -> `/private/var`.
    let root = std::env::temp_dir()
        .canonicalize()
        .unwrap()
        .join(format!("unica-usage-scan-{label}-{nanos}"));
    fs::create_dir_all(&root).unwrap();
    root
}

fn write(root: &Path, relative: &str, body: &str) {
    let path = root.join(relative);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, body).unwrap();
}

fn scan_local_enrichment(
    source_root: &Path,
    kind: MetadataKind,
    name: &str,
    sections: &[LocalSection],
    limit: usize,
    cancellation: &CancellationToken,
) -> LocalEnrichment {
    let metadata_path = MetadataAddress::parse(
        PLATFORM_XML_8_3_27_FORMAT_2_20,
        &format!("{}.{name}", kind.as_str()),
    )
    .unwrap();
    scan_local_enrichment_by_address(
        &unica_workspace::discovery::ProjectDiscoveryReader::new(source_root),
        &super::predefined::PredefinedResourceReadBinding,
        source_root,
        unica_metadata_designer::usage_scan::MetadataUsageTarget {
            kind,
            metadata_path: &metadata_path,
            predefined_code_type: Some("String"),
        },
        sections,
        limit,
        cancellation,
    )
}

fn role(root: &Path, name: &str, subjects: &[&str]) {
    let objects = subjects
        .iter()
        .map(|subject| format!("<object><name>{subject}</name></object>"))
        .collect::<String>();
    write(
        root,
        &format!("Roles/{name}/Ext/Rights.xml"),
        &format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<Rights xmlns="http://v8.1c.ru/8.2/roles" version="2.20">{objects}</Rights>"#
        ),
    );
}

fn names(items: &[serde_json::Value]) -> Vec<&str> {
    items
        .iter()
        .map(|item| item["name"].as_str().unwrap())
        .collect()
}

#[test]
fn roles_match_the_object_and_anything_beneath_it_but_not_a_longer_name() {
    let root = temp_root("roles");
    role(&root, "Direct", &["Catalog.Goods"]);
    role(&root, "Attribute", &["Catalog.Goods.Attribute.Price"]);
    // A right on `Catalog.GoodsArchive` is a different object whose name merely
    // starts with the same characters.
    role(&root, "Neighbour", &["Catalog.GoodsArchive"]);
    role(&root, "Unrelated", &["Document.Order"]);

    let found = scan_local_enrichment(
        &root,
        MetadataKind::Catalog,
        "Goods",
        &[LocalSection::Roles],
        50,
        &CancellationToken::new(),
    );

    assert_eq!(
        names(found.usage.roles.as_ref().unwrap()),
        vec!["Role.Attribute", "Role.Direct"]
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn functional_options_read_content_and_never_the_location() {
    let root = temp_root("functional-options");
    // `Location` says where the option stores its own value; it does not mean
    // the option controls that object. Reading it would answer a different
    // question while looking right.
    write(
        &root,
        "FunctionalOptions/StoredInGoods.xml",
        r#"<?xml version="1.0" encoding="UTF-8"?>
<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" xmlns:xr="http://v8.1c.ru/8.3/xcf/readable" version="2.20">
  <FunctionalOption><Properties><Location>Catalog.Goods</Location>
    <Content><xr:Object>Document.Order</xr:Object></Content>
  </Properties></FunctionalOption></MetaDataObject>"#,
    );
    write(
        &root,
        "FunctionalOptions/ControlsGoods.xml",
        r#"<?xml version="1.0" encoding="UTF-8"?>
<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" xmlns:xr="http://v8.1c.ru/8.3/xcf/readable" version="2.20">
  <FunctionalOption><Properties><Location>Constant.UseGoods</Location>
    <Content><xr:Object>Catalog.Goods.Attribute.Rating</xr:Object></Content>
  </Properties></FunctionalOption></MetaDataObject>"#,
    );

    let found = scan_local_enrichment(
        &root,
        MetadataKind::Catalog,
        "Goods",
        &[LocalSection::FunctionalOptions],
        50,
        &CancellationToken::new(),
    );

    assert_eq!(
        names(found.usage.functional_options.as_ref().unwrap()),
        vec!["FunctionalOption.ControlsGoods"]
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn subscriptions_match_directly_and_through_a_defined_type() {
    let root = temp_root("subscriptions");
    // A source names types, not metadata paths, and it may reach the object
    // through a defined type. On an 8.3.27 vendor dump 80 of 305 subscriptions
    // do exactly that, so an indirect match is part of the answer, not an edge
    // case — and it is reported as indirect rather than silently flattened.
    write(
        &root,
        "DefinedTypes/GoodsLike.xml",
        r#"<?xml version="1.0" encoding="UTF-8"?>
<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" xmlns:v8="http://v8.1c.ru/8.1/data/core" version="2.20">
  <DefinedType><Properties><Name>GoodsLike</Name>
    <Type><v8:Type>cfg:CatalogRef.Goods</v8:Type></Type>
  </Properties></DefinedType></MetaDataObject>"#,
    );
    let subscription = |name: &str, source: &str| {
        format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" xmlns:v8="http://v8.1c.ru/8.1/data/core" version="2.20">
  <EventSubscription><Properties><Name>{name}</Name>
    <Source>{source}</Source><Event>BeforeWrite</Event>
    <Handler>CommonModule.Handlers.OnWrite</Handler>
  </Properties></EventSubscription></MetaDataObject>"#
        )
    };
    write(
        &root,
        "EventSubscriptions/DirectHit.xml",
        &subscription("DirectHit", "<v8:Type>cfg:CatalogObject.Goods</v8:Type>"),
    );
    write(
        &root,
        "EventSubscriptions/ThroughDefinedType.xml",
        &subscription(
            "ThroughDefinedType",
            "<v8:TypeSet>cfg:DefinedType.GoodsLike</v8:TypeSet>",
        ),
    );
    write(
        &root,
        "EventSubscriptions/Unrelated.xml",
        &subscription("Unrelated", "<v8:Type>cfg:DocumentObject.Order</v8:Type>"),
    );

    let found = scan_local_enrichment(
        &root,
        MetadataKind::Catalog,
        "Goods",
        &[LocalSection::Subscriptions],
        50,
        &CancellationToken::new(),
    );

    let subscriptions = found.usage.subscriptions.as_ref().unwrap();
    assert_eq!(
        names(subscriptions),
        vec![
            "EventSubscription.DirectHit",
            "EventSubscription.ThroughDefinedType"
        ]
    );
    assert_eq!(subscriptions[0]["event"], "BeforeWrite");
    assert_eq!(subscriptions[0]["handler"], "CommonModule.Handlers.OnWrite");
    assert!(
        subscriptions[0].get("via").is_none(),
        "a direct source must not claim indirection"
    );
    assert_eq!(subscriptions[1]["via"], "DefinedType.GoodsLike");
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn a_kind_that_cannot_be_a_source_collects_no_subscriptions() {
    let root = temp_root("no-source-kind");
    write(
        &root,
        "EventSubscriptions/Any.xml",
        r#"<?xml version="1.0" encoding="UTF-8"?>
<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" xmlns:v8="http://v8.1c.ru/8.1/data/core" version="2.20">
  <EventSubscription><Properties><Name>Any</Name>
    <Source><v8:Type>cfg:CatalogObject.Goods</v8:Type></Source>
  </Properties></EventSubscription></MetaDataObject>"#,
    );

    let found = scan_local_enrichment(
        &root,
        MetadataKind::CommonModule,
        "Shared",
        &[LocalSection::Subscriptions],
        50,
        &CancellationToken::new(),
    );

    assert_eq!(found.usage.subscriptions.as_deref(), Some(&[][..]));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn predefined_items_report_the_exact_total_and_their_own_truncation() {
    let root = temp_root("predefined");
    let items = (0..5)
        .map(|index| {
            format!(
                "<Item id=\"00000000-0000-0000-0000-00000000000{index}\"><Name>Item{index}</Name></Item>"
            )
        })
        .collect::<String>();
    write(
        &root,
        "Catalogs/Goods/Ext/Predefined.xml",
        &format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<PredefinedData xmlns="http://v8.1c.ru/8.3/xcf/predef" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" xsi:type="CatalogPredefinedItems" version="2.20">{items}</PredefinedData>"#
        ),
    );

    let page = scan_local_enrichment(
        &root,
        MetadataKind::Catalog,
        "Goods",
        &[LocalSection::PredefinedItems],
        2,
        &CancellationToken::new(),
    )
    .predefined_items
    .unwrap();

    assert_eq!(page.total, 5);
    assert_eq!(page.returned, 2);
    assert!(page.truncated);
    assert_eq!(
        page.items
            .iter()
            .map(|item| item.name.as_str())
            .collect::<Vec<_>>(),
        vec!["Item0", "Item1"]
    );

    let whole = scan_local_enrichment(
        &root,
        MetadataKind::Catalog,
        "Goods",
        &[LocalSection::PredefinedItems],
        50,
        &CancellationToken::new(),
    )
    .predefined_items
    .unwrap();
    assert_eq!(whole.returned, 5);
    assert!(!whole.truncated);

    // An object without the file has no predefined items, which is a fact, not
    // a failure.
    let empty = scan_local_enrichment(
        &root,
        MetadataKind::Catalog,
        "Absent",
        &[LocalSection::PredefinedItems],
        50,
        &CancellationToken::new(),
    )
    .predefined_items
    .unwrap();
    assert_eq!(empty.total, 0);
    assert!(!empty.truncated);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn requested_malformed_predefined_data_reports_validation_diagnostic() {
    let root = temp_root("malformed-predefined");
    write(
        &root,
        "Catalogs/Goods/Ext/Predefined.xml",
        r#"<PredefinedData xmlns="http://v8.1c.ru/8.3/xcf/predef" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" xsi:type="ChartOfAccountsPredefinedItems" version="2.20"/>"#,
    );
    let found = scan_local_enrichment(
        &root,
        MetadataKind::Catalog,
        "Goods",
        &[LocalSection::PredefinedItems],
        50,
        &CancellationToken::new(),
    );
    assert!(found.predefined_items.is_none());
    assert_eq!(found.diagnostics.len(), 1);
    assert_eq!(
        found.diagnostics[0].code,
        unica_metadata::MetaDiagnosticCode::ValidationFailed
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn predefined_items_are_validated_against_the_owner_code_type() {
    let root = temp_root("numeric-predefined");
    write(
        &root,
        "Catalogs/Goods/Ext/Predefined.xml",
        r#"<PredefinedData xmlns="http://v8.1c.ru/8.3/xcf/predef" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" xsi:type="CatalogPredefinedItems" version="2.20"><Item id="a7d2e6fc-3824-4b56-b4be-ae6be4944c0e"><Name>Main</Name><Code>ABC</Code></Item></PredefinedData>"#,
    );
    let metadata_path =
        MetadataAddress::parse(PLATFORM_XML_8_3_27_FORMAT_2_20, "Catalog.Goods").unwrap();
    let found = scan_local_enrichment_by_address(
        &unica_workspace::discovery::ProjectDiscoveryReader::new(&root),
        &super::predefined::PredefinedResourceReadBinding,
        &root,
        unica_metadata_designer::usage_scan::MetadataUsageTarget {
            kind: MetadataKind::Catalog,
            metadata_path: &metadata_path,
            predefined_code_type: Some("Number"),
        },
        &[LocalSection::PredefinedItems],
        50,
        &CancellationToken::new(),
    );
    assert!(found.predefined_items.is_none());
    assert_eq!(found.diagnostics.len(), 1);
    assert_eq!(
        found.diagnostics[0].code,
        unica_metadata::MetaDiagnosticCode::ValidationFailed
    );
    assert!(found.diagnostics[0].message.contains("numeric Code"));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn predefined_section_on_unsupported_owner_reports_unsupported_kind() {
    let root = temp_root("unsupported-predefined-owner");
    let found = scan_local_enrichment(
        &root,
        MetadataKind::Document,
        "Order",
        &[LocalSection::PredefinedItems],
        50,
        &CancellationToken::new(),
    );
    assert!(found.predefined_items.is_none());
    assert_eq!(found.diagnostics.len(), 1);
    assert_eq!(
        found.diagnostics[0].code,
        unica_metadata::MetaDiagnosticCode::UnsupportedKind
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn an_unrequested_section_is_absent_rather_than_empty() {
    // An empty list claims the object is used nowhere; absence says nobody
    // asked. The two must not be spelled the same way.
    let root = temp_root("absent-sections");
    role(&root, "Any", &["Catalog.Goods"]);

    let found = scan_local_enrichment(
        &root,
        MetadataKind::Catalog,
        "Goods",
        &[LocalSection::Roles],
        50,
        &CancellationToken::new(),
    );

    assert!(found.usage.roles.is_some());
    assert!(found.usage.subscriptions.is_none());
    assert!(found.usage.functional_options.is_none());
    assert!(found.predefined_items.is_none());
    fs::remove_dir_all(root).unwrap();
}

/// #1119: a section is not cut at a fixed count of descriptors; every role in a
/// group larger than the former 8192-entry cap is read.
#[test]
fn roles_past_the_former_section_cap_are_all_found() {
    let root = temp_root("roles-many");
    let count = 8_200;
    for index in 0..count {
        role(&root, &format!("Role{index:05}"), &["Catalog.Goods"]);
    }

    let found = scan_local_enrichment(
        &root,
        MetadataKind::Catalog,
        "Goods",
        &[LocalSection::Roles],
        50,
        &CancellationToken::new(),
    );

    assert_eq!(found.usage.roles.as_ref().unwrap().len(), count);
    assert!(found.diagnostics.is_empty(), "{:?}", found.diagnostics);
    fs::remove_dir_all(root).unwrap();
}

/// #1119: a descriptor larger than the former 8 MiB read limit is read whole
/// instead of silently dropped from the section.
#[test]
fn a_functional_option_larger_than_eight_mebibytes_is_still_read() {
    let root = temp_root("functional-option-large");
    write(
        &root,
        "FunctionalOptions/Large.xml",
        &format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" xmlns:xr="http://v8.1c.ru/8.3/xcf/readable" version="2.20">
  <!--{}-->
  <FunctionalOption><Properties><Location>Constant.UseGoods</Location>
    <Content><xr:Object>Catalog.Goods</xr:Object></Content>
  </Properties></FunctionalOption></MetaDataObject>"#,
            "x".repeat(9 * 1024 * 1024)
        ),
    );

    let found = scan_local_enrichment(
        &root,
        MetadataKind::Catalog,
        "Goods",
        &[LocalSection::FunctionalOptions],
        50,
        &CancellationToken::new(),
    );

    assert_eq!(
        names(found.usage.functional_options.as_ref().unwrap()),
        vec!["FunctionalOption.Large"]
    );
    assert!(found.diagnostics.is_empty(), "{:?}", found.diagnostics);
    fs::remove_dir_all(root).unwrap();
}

/// #1119: a descriptor the scan cannot read or parse is named as an error of
/// its section; the rest of the list is still returned.
#[test]
fn an_unreadable_descriptor_is_reported_as_a_gap_of_its_section() {
    let root = temp_root("roles-gap");
    role(&root, "Direct", &["Catalog.Goods"]);
    write(&root, "Roles/Broken/Ext/Rights.xml", "<Rights");

    let found = scan_local_enrichment(
        &root,
        MetadataKind::Catalog,
        "Goods",
        &[LocalSection::Roles],
        50,
        &CancellationToken::new(),
    );

    assert_eq!(
        names(found.usage.roles.as_ref().unwrap()),
        vec!["Role.Direct"]
    );
    assert_eq!(found.diagnostics.len(), 1, "{:?}", found.diagnostics);
    let gap = &found.diagnostics[0];
    assert_eq!(gap.severity, unica_metadata::MetaDiagnosticSeverity::Error);
    assert_eq!(
        gap.code,
        unica_metadata::MetaDiagnosticCode::ProviderUnavailable
    );
    assert_eq!(gap.field.as_deref(), Some("roles"));
    assert!(
        gap.message.contains("Roles/Broken/Ext/Rights.xml"),
        "{}",
        gap.message
    );
    fs::remove_dir_all(root).unwrap();
}

/// An absent group or a role without rights has nothing to say: neither is a
/// gap of the section.
#[test]
fn an_absent_group_or_rights_file_is_not_a_gap() {
    let root = temp_root("roles-absent");
    fs::create_dir_all(root.join("Roles/NoRights")).unwrap();
    role(&root, "Direct", &["Catalog.Goods"]);

    let found = scan_local_enrichment(
        &root,
        MetadataKind::Catalog,
        "Goods",
        &[LocalSection::Roles, LocalSection::FunctionalOptions],
        50,
        &CancellationToken::new(),
    );

    assert_eq!(
        names(found.usage.roles.as_ref().unwrap()),
        vec!["Role.Direct"]
    );
    assert!(found.usage.functional_options.as_ref().unwrap().is_empty());
    assert!(found.diagnostics.is_empty(), "{:?}", found.diagnostics);
    fs::remove_dir_all(root).unwrap();
}
