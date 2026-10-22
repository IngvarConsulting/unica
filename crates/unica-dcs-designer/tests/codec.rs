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
