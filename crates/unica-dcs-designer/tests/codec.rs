//! Чтение и проверка схемы через семейный порт [`DcsCodec`].
mod fixtures;

use fixtures::{BASE_SCHEMA, COMPLETE_SCHEMA};
use serde_json::json;
use unica_dcs::codec::{DcsCheckOptions, DcsCheckReport, DcsCodec};
use unica_dcs::facts::DcsFacts;
use unica_dcs_designer::DesignerDcsCodec;
use unica_metadata::support_state::{ObjectSupportData, ObjectSupportState};

fn not_supported() -> ObjectSupportData {
    ObjectSupportData {
        state: ObjectSupportState::NotSupported,
        direct_edit_safe: None,
    }
}

fn read(xml: &str) -> DcsFacts {
    DesignerDcsCodec.read(xml, not_supported()).unwrap()
}

fn check(xml: &str) -> DcsCheckReport {
    let options = DcsCheckOptions {
        detailed: false,
        max_errors: 20,
    };
    DesignerDcsCodec.check(xml, options, "Template.xml")
}

#[test]
fn dcs_info_rejects_wrong_root_namespace() {
    let error = DesignerDcsCodec
        .read(
            &BASE_SCHEMA.replace(
                "http://v8.1c.ru/8.1/data-composition-system/schema",
                "urn:not-dcs",
            ),
            not_supported(),
        )
        .unwrap_err();
    assert!(
        error.contains("urn:not-dcs") && error.contains("DataCompositionSchema"),
        "{error}"
    );
}

#[test]
fn dcs_info_raw_returns_exact_unpaginated_query_text() {
    let query = "  ВЫБРАТЬ 1 КАК Значение  \n|ОБЪЕДИНИТЬ ВСЕ\n|ВЫБРАТЬ 2  ";
    // `Raw` existed because pagination mangled the query; data carries the
    // exact text always, and reading takes no pagination at all.
    let data = read(&BASE_SCHEMA.replace(
        "<query>ВЫБРАТЬ Amount КАК Amount</query>",
        &format!("<query>{query}</query>"),
    ));
    let queries = data
        .data_sets
        .iter()
        .filter_map(|data_set| data_set.query.as_deref())
        .collect::<Vec<_>>();
    assert_eq!(queries, vec![query], "{data:?}");
}

/// Support has its own structured field and is not duplicated in overview prose.
#[test]
fn dcs_info_overview_does_not_duplicate_structured_support() {
    let doc = roxmltree::Document::parse(COMPLETE_SCHEMA).unwrap();
    let mut lines = Vec::new();

    unica_dcs_designer::schema::dcs_info_overview(
        doc.root_element(),
        std::path::Path::new("Template.xml"),
        COMPLETE_SCHEMA,
        &mut lines,
        unica_dcs_designer::schema::DCS_SCHEMA_NS,
        unica_dcs_designer::schema::DCS_SETTINGS_NS,
    );

    assert!(
        lines.iter().all(|line| !line.starts_with("Поддержка:")),
        "support belongs to the typed `support` field, not overview prose: {lines:?}"
    );
}

/// rule: INV.SOURCE.DCS-READ-FACTS
#[test]
fn dcs_info_data_carries_every_fact_the_retired_modes_printed() {
    // The owner's support state is an input of the read, so the fixture passes
    // one no codec could produce by default.
    let support = ObjectSupportData {
        state: ObjectSupportState::Locked,
        direct_edit_safe: Some(false),
    };
    let data = DesignerDcsCodec.read(COMPLETE_SCHEMA, support).unwrap();

    // `overview` printed the owner's support state and the data sources.
    assert_eq!(
        (data.support.state, data.support.direct_edit_safe),
        (ObjectSupportState::Locked, Some(false)),
        "{data:?}"
    );
    assert_eq!(
        data.data_sources
            .iter()
            .map(|source| (source.name.as_str(), source.kind.as_deref()))
            .collect::<Vec<_>>(),
        vec![("ИсточникДанных1", Some("Local"))],
        "{data:?}"
    );

    // `overview` distinguished an Object dataset by its objectName, and
    // every dataset named the source it reads.
    let object_set = data
        .data_sets
        .iter()
        .find(|set| set.kind == "Object")
        .expect("the fixture declares an Object dataset");
    assert_eq!(object_set.object_name.as_deref(), Some("Справочник.Товары"));
    assert_eq!(
        data.data_sets[0].data_source.as_deref(),
        Some("ИсточникДанных1")
    );

    // `resources` printed the grouping a total belongs to; without it an
    // overall total and a per-group total look identical.
    assert_eq!(
        data.total_fields
            .iter()
            .map(|total| (total.data_path.as_str(), total.group.as_deref()))
            .collect::<Vec<_>>(),
        vec![("Сумма", Some("Товар")), ("Количество", None)],
        "{data:?}"
    );

    // `calculated -Name` printed which uses a field is barred from; the
    // element is structured, so a boolean cannot carry that answer.
    assert_eq!(
        data.calculated_fields[0].restrictions.as_deref(),
        Some(&["condition".to_string(), "order".to_string()][..]),
        "{data:?}"
    );

    // `params` marked a parameter that cannot be used as a field.
    assert_eq!(
        data.parameters[0].available_as_field,
        Some(false),
        "{data:?}"
    );

    // `variant` printed the name, presentation, selection, filters and the
    // structure with its grouping fields.
    let variant = &data.variants[0];
    assert_eq!(variant.name, "Основной");
    assert_eq!(variant.presentation.as_deref(), Some("Основной вариант"));
    assert_eq!(variant.selection, vec!["Товар".to_string()]);
    assert_eq!(variant.filters, 1);
    assert_eq!(
        variant
            .order
            .iter()
            .map(|item| (item.field.as_str(), item.direction.as_deref()))
            .collect::<Vec<_>>(),
        vec![("Сумма", Some("Desc"))]
    );
    assert_eq!(
        variant
            .structure
            .iter()
            .map(|item| (item.kind.as_str(), item.group_by.clone()))
            .collect::<Vec<_>>(),
        vec![("Group", vec!["Товар".to_string()])]
    );
}

#[test]
fn dcs_info_structure_items_preserve_names_unnamed_parents_duplicates_and_axes() {
    let xml = r#"<DataCompositionSchema xmlns="http://v8.1c.ru/8.1/data-composition-system/schema" xmlns:s="http://v8.1c.ru/8.1/data-composition-system/settings" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance">
    <settingsVariant><s:name>Variant</s:name><s:settings>
      <s:item xsi:type="s:StructureItemGroup"><s:item xsi:type="s:StructureItemGroup"><s:name>Nested</s:name></s:item></s:item>
      <s:item xsi:type="s:StructureItemGroup"/>
      <s:item xsi:type="s:StructureItemTable"><s:row><s:item xsi:type="s:StructureItemGroup"><s:name>Repeated</s:name></s:item></s:row><s:column><s:item xsi:type="s:StructureItemGroup"><s:name>Repeated</s:name></s:item></s:column></s:item>
      <s:item xsi:type="s:StructureItemChart"><s:point><s:item xsi:type="s:StructureItemGroup"><s:name>name with spaces</s:name></s:item></s:point><s:series><s:item xsi:type="s:StructureItemGroup"><s:name>Series</s:name></s:item></s:series></s:item>
      <s:selection><s:item xsi:type="s:SelectedItemField"><s:name>Decoy</s:name></s:item></s:selection>
      <s:item xsi:type="s:StructureItemNestedObject"><s:name>NestedObject</s:name><s:item xsi:type="s:StructureItemGroup"><s:name>NestedChild</s:name></s:item></s:item>
      <s:item xsi:type="s:StructureItemFuture"><s:name>Unknown</s:name><s:item xsi:type="s:StructureItemGroup"><s:name>FutureChild</s:name></s:item></s:item>
    </s:settings></settingsVariant></DataCompositionSchema>"#;
    let data = read(xml);
    let value = serde_json::to_value(data).unwrap();
    let rows = value["variants"][0]["structureItems"]
        .as_array()
        .expect("structure names must be reachable without reading XML");
    assert_eq!(
        rows.iter()
            .map(|row| row["name"].clone())
            .collect::<Vec<_>>(),
        vec![
            json!(null),
            json!("Nested"),
            json!(null),
            json!(null),
            json!("Repeated"),
            json!("Repeated"),
            json!(null),
            json!("name with spaces"),
            json!("Series"),
            json!("NestedObject"),
            json!("NestedChild"),
            json!("Unknown"),
            json!("FutureChild")
        ]
    );
    assert_eq!(rows[1]["parentIndex"], json!(0));
    assert_eq!(rows[4]["parentIndex"], json!(3));
    assert_eq!(rows[4]["axis"], json!("row"));
    assert_eq!(rows[5]["axis"], json!("column"));
    assert_eq!(rows[7]["axis"], json!("point"));
    assert_eq!(rows[8]["axis"], json!("series"));
    assert_eq!(rows[9]["kind"], json!("NestedObject"));
    assert_eq!(rows[10]["parentIndex"], json!(9));
    assert_eq!(rows[11]["kind"], json!("StructureItemFuture"));
    assert_eq!(rows[12]["parentIndex"], json!(11));
    assert!(rows.iter().all(|row| row.get("at").is_none()));
}

/// #311. A `SettingsParameterValue` with an empty `<dcscor:parameter>` is
/// well-formed XML that names no parameter, so the variant carries no value
/// for it. Validation reported `Validation OK` on exactly that shape.
#[test]
fn native_dcs_validate_rejects_a_data_parameter_without_a_name() {
    let report = check(&BASE_SCHEMA.replace(
        "\t\t\t<dcsset:order>",
        "\t\t\t<dcsset:dataParameters>\n\t\t\t\t<dcscor:item xsi:type=\"dcsset:SettingsParameterValue\">\n\t\t\t\t\t<dcscor:parameter></dcscor:parameter>\n\t\t\t\t</dcscor:item>\n\t\t\t</dcsset:dataParameters>\n\t\t\t<dcsset:order>",
    ));
    let stdout = report.report;
    assert!(!report.ok, "{stdout}");
    assert!(
        stdout.contains("SettingsParameterValue has empty parameter name"),
        "{stdout}"
    );
}

#[test]
fn native_dcs_validate_rejects_ref_type_bound_to_unexpected_namespace() {
    let report = check(&BASE_SCHEMA.replace(
        "<field>Amount</field>",
        "<field>Amount</field>\n\t\t\t<valueType>\n\t\t\t\t<v8:Type xmlns:bad=\"http://example.com\">bad:CatalogRef.X</v8:Type>\n\t\t\t</valueType>",
    ));
    let stdout = report.report;
    assert!(!report.ok, "{stdout}");
    assert!(
        stdout.contains("uses prefix 'bad' bound to unexpected namespace 'http://example.com'"),
        "{stdout}"
    );
}

#[test]
fn nested_setting_filters_keep_ownership_negation_and_typed_zero() {
    let xml = r#"<DataCompositionSchema xmlns="http://v8.1c.ru/8.1/data-composition-system/schema" xmlns:s="http://v8.1c.ru/8.1/data-composition-system/settings" xmlns:c="http://v8.1c.ru/8.1/data-composition-system/core" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" xmlns:xs="http://www.w3.org/2001/XMLSchema">
    <settingsVariant><s:name>Variant</s:name><s:settings>
      <s:filter><s:item xsi:type="s:FilterItemComparison"><s:left xsi:type="c:Field">Root</s:left><s:comparisonType>Filled</s:comparisonType></s:item></s:filter>
      <s:item xsi:type="s:StructureItemGroup"><s:filter><s:item xsi:type="s:FilterItemGroup"><s:groupType>NotGroup</s:groupType><s:item xsi:type="s:FilterItemComparison"><s:left xsi:type="c:Field">Серия</s:left><s:comparisonType>NotFilled</s:comparisonType></s:item><s:item xsi:type="s:FilterItemComparison"><s:left xsi:type="c:Field">НачальныйОстаток</s:left><s:comparisonType>Equal</s:comparisonType><s:right xsi:type="xs:decimal">0</s:right></s:item></s:item></s:filter><s:order><s:item xsi:type="s:OrderItemField"><s:field>Серия</s:field><s:orderType>Asc</s:orderType></s:item></s:order></s:item>
    </s:settings></settingsVariant></DataCompositionSchema>"#;
    let data = read(xml);
    let variant = &data.variants[0];
    let rows = &variant.setting_items;
    let group = rows
        .iter()
        .find(|row| row["itemType"] == "FilterItemGroup")
        .unwrap();
    assert_eq!(group["groupType"], "NotGroup");
    assert_eq!(group["scope"], "structure");
    assert_eq!(group["structureIndex"], variant.structure_items[0].index);
    let zero = rows
        .iter()
        .find(|row| row["field"] == "НачальныйОстаток")
        .unwrap();
    assert_eq!(zero["parentIndexes"], json!([0]));
    assert_eq!(zero["value"], "0");
    assert_eq!(
        zero["typedValue"],
        json!({"type":{"namespace":"http://www.w3.org/2001/XMLSchema","localName":"decimal"},"value":"0"})
    );
    assert_eq!(zero["structureIndex"], 0);
    let root = rows.iter().find(|row| row["field"] == "Root").unwrap();
    assert_eq!(root["scope"], "root");
    assert!(root.get("structureIndex").is_none());
    let order = rows.iter().find(|row| row["kind"] == "Order").unwrap();
    assert_eq!(order["scope"], "structure");
    assert_eq!(order["structureIndex"], 0);
    assert!(rows.iter().all(|row| row.get("at").is_none()));
}

const NESTED_SETTINGS: &str = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../tests/fixtures/acceptance/workspace-dcs-nested-setting-facts/src/Reports/Probe/Templates/Schema/Ext/Template.xml"));

#[test]
fn nested_setting_fixture_keeps_all_five_conditions_and_appearance_widths() {
    let data = read(NESTED_SETTINGS);
    let root = data.variants.iter().find(|v| v.name == "Root").unwrap();
    let filters: Vec<_> = root
        .setting_items
        .iter()
        .filter(|row| row["itemType"] == "FilterItemComparison")
        .collect();
    assert_eq!(filters.len(), 9);
    let nested: Vec<_> = filters
        .iter()
        .filter(|row| row["scope"] == "structure")
        .collect();
    assert_eq!(nested.len(), 5);
    assert!(nested
        .iter()
        .all(|row| row["parentIndexes"] == json!([0]) && row["structureIndex"] == 0));
    assert_eq!(nested[0]["field"], "Серия");
    assert_eq!(nested[0]["comparison"], "NotFilled");
    for row in &nested[1..] {
        assert_eq!(row["typedValue"]["value"], "0");
        assert_eq!(row["typedValue"]["type"]["localName"], "decimal");
    }
    let mobile = data.variants.iter().find(|v| v.name == "Mobile").unwrap();
    let appearances: Vec<_> = mobile
        .setting_items
        .iter()
        .filter(|row| row["kind"] == "ConditionalAppearance")
        .collect();
    assert_eq!(appearances.len(), 4);
    assert!(appearances
        .iter()
        .all(|row| row["filterState"] == "empty" && row.get("fields").is_some()));
    let parameters: Vec<_> = mobile
        .setting_items
        .iter()
        .filter(|row| row["kind"] == "AppearanceParameter")
        .collect();
    assert_eq!(
        parameters
            .iter()
            .map(|row| row["typedValue"]["value"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["20", "6", "12", "12", "4"]
    );
    assert_eq!(
        parameters
            .iter()
            .map(|row| row["appearanceIndex"].as_u64().unwrap())
            .collect::<Vec<_>>(),
        [0, 1, 2, 2, 3]
    );
    assert!(mobile
        .setting_items
        .iter()
        .any(|row| row["kind"] == "FilterContainer"
            && row["scope"] == "structure"
            && row["state"] == "absent"));
    assert!(mobile
        .setting_items
        .iter()
        .filter(|row| row["kind"] == "FilterContainer" && row.get("appearanceIndex").is_some())
        .all(|row| row["state"] == "empty"));
    assert!(
        data.variants
            .iter()
            .flat_map(|v| v.setting_items.iter())
            .all(|row| row.get("at").is_none() && row.get("unreadable").is_none()),
        "{data:?}"
    );
}

#[test]
fn nested_setting_damage_is_local_and_never_a_known_empty_filter() {
    for (from, to) in [
        ("dcsset:FilterItemGroup", "xs:FilterItemGroup"),
        (
            "<dcsset:groupType>NotGroup</dcsset:groupType>",
            "<dcsset:groupType>NotGroup</dcsset:groupType><dcsset:groupType>OrGroup</dcsset:groupType>",
        ),
        (">0</dcsset:right>", ">bad</dcsset:right>"),
        ("<dcsset:filter/>", "<dcsset:filter/><dcsset:filter/>"),
        ("dcsset:SettingsParameterValue", "xs:SettingsParameterValue"),
    ] {
        assert!(NESTED_SETTINGS.contains(from), "{from}");
        let data = read(&NESTED_SETTINGS.replacen(from, to, 1));
        assert!(
            data.variants
                .iter()
                .flat_map(|v| v.setting_items.iter())
                .any(|row| row.get("unreadable").is_some()),
            "{from}"
        );
        assert!(data
            .variants
            .iter()
            .flat_map(|v| v.setting_items.iter())
            .any(|row| row["kind"] == "Order"));
    }
}

#[test]
fn setting_fact_damage_and_string_whitespace_survive_the_closed_projection() {
    let spaced = NESTED_SETTINGS
        .replacen("xs:decimal", "xs:string", 1)
        .replacen(">0</dcsset:right>", "> 0 </dcsset:right>", 1);
    let data = read(&spaced);
    let row = data.variants[0]
        .setting_items
        .iter()
        .find(|row| row["kind"] == "Filter")
        .unwrap();
    assert_eq!(row["value"], "0");
    assert_eq!(row["typedValue"]["value"], " 0 ");
    let nested = NESTED_SETTINGS.replacen(
        ">НачальныйОстаток</dcsset:left>",
        "><dcsset:garbage>НачальныйОстаток</dcsset:garbage></dcsset:left>",
        1,
    );
    let data = read(&nested);
    let row = data.variants[0]
        .setting_items
        .iter()
        .find(|row| row["kind"] == "Filter")
        .unwrap();
    assert!(row.get("unreadable").is_some());
    assert!(row.get("field").is_none());
    let duplicate = NESTED_SETTINGS.replacen(
        "</dcsset:selection>",
        "</dcsset:selection><dcsset:selection/>",
        1,
    );
    let data = read(&duplicate);
    let row = data.variants[1]
        .setting_items
        .iter()
        .find(|row| row["kind"] == "ConditionalAppearance")
        .unwrap();
    assert!(row.get("unreadable").is_some());
    assert!(row.get("fields").is_none());
}

#[test]
fn nested_object_settings_use_the_published_structure_item_index() {
    let xml = NESTED_SETTINGS.replacen(
        "dcsset:StructureItemGroup",
        "dcsset:StructureItemNestedObject",
        1,
    );
    let data = read(&xml);
    assert_eq!(data.variants[0].structure_items[0].kind, "NestedObject");
    let group = data.variants[0]
        .setting_items
        .iter()
        .find(|row| row["itemType"] == "FilterItemGroup")
        .unwrap();
    assert_eq!(
        group["structureIndex"],
        data.variants[0].structure_items[0].index
    );
    assert_eq!(group["groupType"], "NotGroup");
}

#[test]
fn setting_scopes_follow_item_indices_across_axis_wrappers() {
    let filter = |field: &str| {
        format!("<s:filter><s:item xsi:type=\"s:FilterItemComparison\"><s:left>{field}</s:left><s:comparisonType>Filled</s:comparisonType></s:item></s:filter>")
    };
    let xml = format!(
        r#"<DataCompositionSchema xmlns="http://v8.1c.ru/8.1/data-composition-system/schema" xmlns:s="http://v8.1c.ru/8.1/data-composition-system/settings" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance"><settingsVariant><s:name>Axes</s:name><s:settings>{}<s:item xsi:type="s:StructureItemTable"><s:row>{}<s:item xsi:type="s:StructureItemGroup"><s:name>Named</s:name>{}</s:item></s:row><s:column><s:item xsi:type="s:StructureItemGroup">{}</s:item></s:column></s:item></s:settings></settingsVariant></DataCompositionSchema>"#,
        filter("RootF"),
        filter("RowF"),
        filter("NestedF"),
        filter("ColumnF")
    );
    let data = read(&xml);
    let variant = &data.variants[0];
    assert_eq!(variant.structure_items.len(), 4);
    for (field, index, axis) in [
        ("RowF", 1, "row"),
        ("NestedF", 2, "row"),
        ("ColumnF", 3, "column"),
    ] {
        let row = variant
            .setting_items
            .iter()
            .find(|row| row["field"] == field)
            .unwrap();
        assert_eq!(row["index"], 0);
        assert_eq!(row["scope"], "structure");
        assert_eq!(row["structureIndex"], variant.structure_items[index].index);
        assert_eq!(row["axis"], axis);
        if field == "NestedF" {
            assert_eq!(row["group"], "Named");
        }
    }
    assert!(variant
        .setting_items
        .iter()
        .any(|row| row["kind"] == "FilterContainer"
            && row["axis"] == "column"
            && row["structureIndex"] == 0
            && row["state"] == "absent"));
    let root = variant
        .setting_items
        .iter()
        .find(|row| row["field"] == "RootF")
        .unwrap();
    assert_eq!(root["scope"], "root");
    assert!(root.get("structureIndex").is_none());
}

#[test]
fn filter_container_metadata_is_not_an_extra_condition() {
    let xml=NESTED_SETTINGS.replacen("<dcsset:filter>","<dcsset:filter><dcsset:viewMode>Normal</dcsset:viewMode><dcsset:userSettingID>id</dcsset:userSettingID>",1);
    let base = read(NESTED_SETTINGS);
    let data = read(&xml);
    assert_eq!(
        data.variants[0].setting_items,
        base.variants[0].setting_items
    );
}

#[test]
fn appearance_container_view_mode_is_not_an_extra_rule() {
    let xml = NESTED_SETTINGS.replace(
        "<dcsset:conditionalAppearance>",
        "<dcsset:conditionalAppearance><dcsset:viewMode>Normal</dcsset:viewMode><dcsset:userSettingID>id</dcsset:userSettingID><dcsset:userSettingPresentation>Title</dcsset:userSettingPresentation>",
    );
    assert_ne!(xml, NESTED_SETTINGS);
    assert_eq!(
        read(&xml).variants[1].setting_items,
        read(NESTED_SETTINGS).variants[1].setting_items
    );
}

fn scalar_setting_fact(type_attributes: &str, value: &str) -> serde_json::Value {
    let xml = NESTED_SETTINGS.replacen(
        r#"xsi:type="xs:decimal">0</dcsset:right>"#,
        &format!("{type_attributes}>{value}</dcsset:right>"),
        1,
    );
    assert_ne!(xml, NESTED_SETTINGS);
    read(&xml).variants[0]
        .setting_items
        .iter()
        .find(|row| row["kind"] == "Filter")
        .unwrap()
        .clone()
}

#[test]
fn scalar_setting_qnames_use_local_default_bindings_and_xml_whitespace() {
    for attrs in [
        r#"xmlns="http://www.w3.org/2001/XMLSchema" xsi:type="decimal""#,
        r#"xsi:type=" &#x9;xs:decimal&#xA; ""#,
        r#"xmlns:t="http://www.w3.org/2001/XMLSchema" xsi:type="t:decimal""#,
    ] {
        let row = scalar_setting_fact(attrs, " +000.00 ");
        assert_eq!(
            row["typedValue"],
            json!({
                "type":{"namespace":"http://www.w3.org/2001/XMLSchema","localName":"decimal"},
                "value":" +000.00 "
            }),
            "{attrs}"
        );
        assert!(row.get("unreadable").is_none(), "{row}");
    }
}

#[test]
fn scalar_setting_non_xml_whitespace_is_not_a_valid_number_or_boolean() {
    for (kind, value) in [("boolean", "false"), ("decimal", "0"), ("integer", "0")] {
        let row = scalar_setting_fact(
            &format!(r#"xsi:type="xs:{kind}""#),
            &format!("&#160;{value}&#160;"),
        );
        assert!(row.get("typedValue").is_none(), "{row}");
        assert!(row.get("unreadable").is_some(), "{row}");
    }
    let row = scalar_setting_fact(r#"xsi:type="xs:string""#, "&#160;false&#160;");
    assert_eq!(row["typedValue"]["value"], "\u{a0}false\u{a0}");
    assert!(row.get("unreadable").is_none());
}

#[test]
fn scalar_setting_int_range_differs_from_unbounded_integer() {
    for value in [
        "-2147483648",
        "2147483647",
        "+0002147483647",
        "-0002147483648",
    ] {
        let row = scalar_setting_fact(r#"xsi:type="xs:int""#, value);
        assert_eq!(row["typedValue"]["value"], value, "{row}");
        assert!(row.get("unreadable").is_none(), "{row}");
    }
    for value in ["-2147483649", "2147483648"] {
        let row = scalar_setting_fact(r#"xsi:type="xs:int""#, value);
        assert!(row.get("typedValue").is_none(), "{row}");
        assert!(row.get("unreadable").is_some(), "{row}");
    }
    let value = "1234567890123456789012345678901234567890";
    let row = scalar_setting_fact(r#"xsi:type="xs:integer""#, value);
    assert_eq!(row["typedValue"]["value"], value);
    assert!(row.get("unreadable").is_none());
}
