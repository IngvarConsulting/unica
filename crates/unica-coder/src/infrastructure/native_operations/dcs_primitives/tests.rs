use super::{apply, Primitive, Target, C, S, T, V, X};
use crate::domain::operation_contract::OperationContract;
use roxmltree::{Document, Node};
use serde_json::{json, Value};

const COMMON: &str = "http://v8.1c.ru/8.1/data-composition-system/common";

fn schema(body: &str) -> String {
    format!("<DataCompositionSchema xmlns=\"{S}\" xmlns:s=\"{S}\" xmlns:t=\"{T}\" xmlns:c=\"{C}\" xmlns:v8=\"{V}\" xmlns:xsi=\"{X}\" xmlns:xs=\"http://www.w3.org/2001/XMLSchema\" xmlns:com=\"{COMMON}\">{body}</DataCompositionSchema>")
}

fn query_schema(query: &str) -> String {
    schema(&format!("<dataSource><name>Local</name><dataSourceType>Local</dataSourceType></dataSource><dataSet xsi:type=\"s:DataSetQuery\"><name>Data</name><field xsi:type=\"s:DataSetFieldField\"><dataPath>Amount</dataPath><field>Amount</field></field><dataSource>Local</dataSource><query>{query}</query></dataSet><settingsVariant><t:name>Main</t:name><t:settings><t:selection/><t:filter/><t:order/></t:settings></settingsVariant>"))
}

fn target(dataset: &[&str], variant: &str) -> Target {
    Target {
        datasets: dataset.iter().map(|name| (*name).to_string()).collect(),
        variant: variant.to_string(),
        terminal: None,
    }
}

fn run(xml: &mut String, operation: Primitive, args: Value, target: &Target) -> Result<(), String> {
    apply(xml, operation.as_str(), args.as_object().unwrap(), target)
}

fn normalized(operation: Primitive, args: Value) -> Value {
    OperationContract::dcs(operation.as_str())
        .unwrap()
        .normalize(&args, "args")
        .unwrap_or_else(|error| panic!("{}: {error:?}", operation.as_str()))
}

fn direct<'a, 'i>(parent: Node<'a, 'i>, ns: &str, name: &str) -> Node<'a, 'i> {
    parent
        .children()
        .find(|node| node.has_tag_name((ns, name)))
        .unwrap_or_else(|| panic!("missing direct child {name}"))
}

fn direct_text(parent: Node<'_, '_>, ns: &str, name: &str) -> String {
    direct(parent, ns, name)
        .children()
        .filter(Node::is_text)
        .filter_map(|node| node.text())
        .collect()
}

fn names(parent: Node<'_, '_>) -> Vec<String> {
    parent
        .children()
        .filter(Node::is_element)
        .map(|node| node.tag_name().name().to_string())
        .collect()
}

fn named<'a, 'i>(parent: Node<'a, 'i>, ns: &str, tag: &str, key: &str, name: &str) -> Node<'a, 'i> {
    parent
        .children()
        .filter(|node| node.has_tag_name((ns, tag)))
        .find(|node| direct_text(*node, ns, key) == name)
        .unwrap_or_else(|| panic!("missing {tag} {name}"))
}

fn variant<'a, 'i>(root: Node<'a, 'i>, name: &str) -> Node<'a, 'i> {
    root.children()
        .filter(|node| node.has_tag_name((S, "settingsVariant")))
        .find(|node| direct_text(*node, T, "name") == name)
        .unwrap()
}

fn assert_refuses(xml: &str, operation: Primitive, args: Value, selected: &Target) {
    let mut tentative = xml.to_string();
    assert!(
        run(&mut tentative, operation, args, selected).is_err(),
        "{} must reject this input",
        operation.as_str()
    );
}

#[test]
fn prefixed_root_and_shadowed_prefixes_keep_untouched_nodes_exact() {
    let untouched = "<u:future xmlns:u=\"urn:future\" xmlns:v8=\"urn:shadow\" xmlns:t=\"urn:shadow-settings\" a='1'>\r\n  <!-- original --><?keep exact?><v8:value>  left &amp; right  </v8:value>\n</u:future>";
    let mut xml = format!("\u{feff}<s:DataCompositionSchema xmlns:s=\"{S}\" xmlns:xsi=\"{X}\">\r\n<s:dataSet xsi:type=\"s:DataSetQuery\"><s:name>Data</s:name><s:query>SELECT 1</s:query></s:dataSet>\n{untouched}\r\n</s:DataCompositionSchema>");
    run(
        &mut xml,
        Primitive::FieldAdd,
        json!({"items":[{"dataPath":"Added","title":{"ru":"Добавлено","en":"Added"}}]}),
        &target(&["Data"], ""),
    )
    .unwrap();
    assert!(xml.starts_with('\u{feff}'));
    assert!(
        xml.contains(untouched),
        "unowned bytes must not be reserialized"
    );
    let doc = Document::parse(&xml).unwrap();
    let field = named(
        direct(doc.root_element(), S, "dataSet"),
        S,
        "field",
        "dataPath",
        "Added",
    );
    let title = direct(field, S, "title");
    assert_eq!(title.lookup_namespace_uri(Some("v8")), Some(V));
    let translations = title
        .children()
        .filter(|node| node.has_tag_name((V, "item")))
        .map(|node| {
            (
                direct_text(node, V, "lang"),
                direct_text(node, V, "content"),
            )
        })
        .collect::<Vec<_>>();
    assert!(translations.contains(&("ru".to_string(), "Добавлено".to_string())));
    assert!(translations.contains(&("en".to_string(), "Added".to_string())));
}

#[test]
fn field_alias_normalization_and_query_derived_type_reach_the_writer() {
    let args = normalized(
        Primitive::FieldAdd,
        json!({"items":[{"dataPath":"Added","name":"Wrong","type":"String","title":"Added"}]}),
    );
    let mut xml = query_schema("SELECT 1 AS Amount");
    run(&mut xml, Primitive::FieldAdd, args, &target(&["Data"], "")).unwrap();
    let doc = Document::parse(&xml).unwrap();
    let dataset = direct(doc.root_element(), S, "dataSet");
    assert_eq!(
        names(dataset),
        ["name", "field", "field", "dataSource", "query"]
    );
    let field = named(dataset, S, "field", "dataPath", "Added");
    assert_eq!(names(field), ["dataPath", "field", "title"]);
    assert!(!xml.contains("Wrong"));
}

#[test]
fn object_and_union_field_types_have_platform_qualifiers_and_order() {
    for kind in ["Object", "Union"] {
        let payload = if kind == "Object" {
            "<objectName>Catalog.Items</objectName>"
        } else {
            "<item xsi:type=\"s:DataSetQuery\"><name>Inner</name><query>SELECT 1</query></item>"
        };
        let mut xml = schema(&format!(
            "<dataSet xsi:type=\"s:DataSet{kind}\"><name>Data</name>{payload}</dataSet>"
        ));
        run(
            &mut xml,
            Primitive::FieldAdd,
            json!({"items":[{"dataPath":"Added","type":"String"}]}),
            &target(&["Data"], ""),
        )
        .unwrap();
        let doc = Document::parse(&xml).unwrap();
        let field = named(
            direct(doc.root_element(), S, "dataSet"),
            S,
            "field",
            "dataPath",
            "Added",
        );
        assert_eq!(names(field), ["dataPath", "field", "valueType"]);
        let value_type = direct(field, S, "valueType");
        assert_eq!(names(value_type), ["Type", "StringQualifiers"]);
        assert_eq!(direct_text(value_type, V, "Type"), "xs:string");
        let qualifiers = direct(value_type, V, "StringQualifiers");
        assert_eq!(names(qualifiers), ["Length", "AllowedLength"]);
        assert_eq!(direct_text(qualifiers, V, "Length"), "0");
        assert_eq!(direct_text(qualifiers, V, "AllowedLength"), "Variable");
    }
}

#[test]
fn invalid_explicit_types_are_rejected_even_on_query_fields() {
    let source = query_schema("SELECT 1 AS Amount");
    let selected = target(&["Data"], "");
    for invalid in [
        "string(1025)",
        "decimal(39,0)",
        "decimal(10,11)",
        "MysteryRef.Items",
        "CatalogRef.Items.Extra",
        "typeid:not-a-uuid",
        "string|String(10)",
        "DefinedType.NamedType",
    ] {
        for operation in [Primitive::FieldAdd, Primitive::FieldSet] {
            let args = if matches!(operation, Primitive::FieldAdd) {
                json!({"items":[{"dataPath":"Bad","type":invalid}]})
            } else {
                json!({"values":{"field":"Amount","type":invalid}})
            };
            assert_refuses(&source, operation, args, &selected);
        }
    }
}

#[test]
fn mixed_types_publish_xsd_type_groups_and_qualifiers_in_order() {
    let mut xml = schema("<dataSet xsi:type=\"s:DataSetObject\"><name>Data</name><objectName>Catalog.Items</objectName></dataSet>");
    run(&mut xml, Primitive::FieldAdd, json!({"items":[{"dataPath":"Value","type":"ВидыСубконтоХозрасчетные|typeid:00112233-4455-6677-8899-aabbccddeeff|date|string(12)|decimal(15,2,nonneg)|CatalogRef.Items|boolean"}]}), &target(&["Data"], "")).unwrap();
    let doc = Document::parse(&xml).unwrap();
    let value_type = direct(
        direct(direct(doc.root_element(), S, "dataSet"), S, "field"),
        S,
        "valueType",
    );
    assert_eq!(
        names(value_type),
        [
            "Type",
            "Type",
            "Type",
            "Type",
            "Type",
            "TypeSet",
            "TypeId",
            "NumberQualifiers",
            "StringQualifiers",
            "DateQualifiers"
        ]
    );
    let wire_types = value_type
        .children()
        .filter(|node| node.has_tag_name((V, "Type")))
        .map(|node| node.text().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(&wire_types[..3], ["xs:dateTime", "xs:string", "xs:decimal"]);
    let reference = value_type
        .children()
        .find(|node| {
            node.has_tag_name((V, "Type"))
                && node
                    .text()
                    .is_some_and(|value| value.ends_with(":CatalogRef.Items"))
        })
        .unwrap();
    let prefix = reference.text().unwrap().split(':').next().unwrap();
    assert_eq!(
        reference.lookup_namespace_uri(Some(prefix)),
        Some("http://v8.1c.ru/8.1/data/enterprise/current-config")
    );
    assert_eq!(
        direct_text(value_type, V, "TypeId"),
        "00112233-4455-6677-8899-aabbccddeeff"
    );
}

#[test]
fn field_patch_keeps_extended_children_and_inserts_in_full_xsd_sequence() {
    let preserved = "<attributeUseRestriction><condition>true</condition></attributeUseRestriction><presentationExpression>Amount</presentationExpression>";
    let trailing = "<appearance/><availableValue/><inputParameters/><u:future xmlns:u=\"urn:future\">unchanged</u:future>";
    let mut xml = schema(&format!("<dataSet xsi:type=\"s:DataSetObject\"><name>Data</name><field><dataPath>Amount</dataPath><field>Amount</field><useRestriction><field>true</field></useRestriction>{preserved}{trailing}</field><objectName>Catalog.Items</objectName></dataSet>"));
    run(
        &mut xml,
        Primitive::FieldSet,
        json!({"values":{"field":"Amount","title":"Updated","type":"string"}}),
        &target(&["Data"], ""),
    )
    .unwrap();
    run(
        &mut xml,
        Primitive::FieldRoleSet,
        json!({"values":{"field":"Amount","role":{"dimension":true}}}),
        &target(&["Data"], ""),
    )
    .unwrap();
    for element in [
        "<attributeUseRestriction><condition>true</condition></attributeUseRestriction>",
        "<presentationExpression>Amount</presentationExpression>",
    ] {
        assert!(xml.contains(element), "changed preserved element {element}");
    }
    assert!(xml.contains(trailing));
    let doc = Document::parse(&xml).unwrap();
    let field = direct(direct(doc.root_element(), S, "dataSet"), S, "field");
    assert_eq!(
        names(field),
        [
            "dataPath",
            "field",
            "title",
            "useRestriction",
            "attributeUseRestriction",
            "role",
            "presentationExpression",
            "valueType",
            "appearance",
            "availableValue",
            "inputParameters",
            "future"
        ]
    );
}

#[test]
fn query_field_title_patch_removes_old_value_type_without_touching_other_children() {
    let source = query_schema("SELECT 1 AS Amount").replace(
        "<field>Amount</field>",
        "<field>Amount</field><valueType><v8:Type>xs:string</v8:Type></valueType><appearance/>",
    );
    let mut xml = source;
    run(
        &mut xml,
        Primitive::FieldSet,
        json!({"values":{"field":"Amount","title":"Updated"}}),
        &target(&["Data"], ""),
    )
    .unwrap();
    let doc = Document::parse(&xml).unwrap();
    let field = direct(direct(doc.root_element(), S, "dataSet"), S, "field");
    assert_eq!(names(field), ["dataPath", "field", "title", "appearance"]);
}

#[test]
fn field_role_uses_common_namespace_canonical_order_and_replaces_previous_entries() {
    let mut xml = query_schema("SELECT 1 AS Amount").replace("<field>Amount</field>", "<field>Amount</field><role><com:account>true</com:account><com:required>false</com:required></role>");
    let args = normalized(
        Primitive::FieldRoleSet,
        json!({"values":{"field":"Amount","role":{"required":true,"balanceType":"OpeningBalance","dimension":true,"periodType":"Additional","periodNumber":"2"}}}),
    );
    run(
        &mut xml,
        Primitive::FieldRoleSet,
        args,
        &target(&["Data"], ""),
    )
    .unwrap();
    let doc = Document::parse(&xml).unwrap();
    let role = direct(
        direct(direct(doc.root_element(), S, "dataSet"), S, "field"),
        S,
        "role",
    );
    assert_eq!(
        names(role),
        [
            "periodNumber",
            "periodType",
            "dimension",
            "balanceType",
            "required"
        ]
    );
    for node in role.children().filter(Node::is_element) {
        assert_eq!(node.tag_name().namespace(), Some(COMMON));
    }
    assert_eq!(direct_text(role, COMMON, "periodNumber"), "2");
    assert_eq!(direct_text(role, COMMON, "periodType"), "Additional");
    assert_eq!(direct_text(role, COMMON, "required"), "true");
}

#[test]
fn parameter_patch_preserves_existing_values_and_advanced_properties_in_order() {
    let preserved = "<expression>&amp;Source.Choice</expression><availableValue><value xsi:type=\"xs:string\">A</value><presentation>Alpha</presentation></availableValue><valueListAllowed>true</valueListAllowed><availableAsField>false</availableAsField><denyIncompleteValues>true</denyIncompleteValues><use>Always</use>";
    let mut xml = schema(&format!("<parameter><name>P</name><valueType><v8:Type>xs:string</v8:Type></valueType><value xsi:type=\"xs:string\">A</value>{preserved}</parameter>"));
    run(
        &mut xml,
        Primitive::ParameterSet,
        json!({"values":{"name":"P","title":"Choice","useRestriction":true}}),
        &target(&[], ""),
    )
    .unwrap();
    assert!(xml.contains(preserved));
    let doc = Document::parse(&xml).unwrap();
    assert_eq!(
        names(direct(doc.root_element(), S, "parameter")),
        [
            "name",
            "title",
            "valueType",
            "value",
            "useRestriction",
            "expression",
            "availableValue",
            "valueListAllowed",
            "availableAsField",
            "denyIncompleteValues",
            "use"
        ]
    );
}

#[test]
fn schema_parameter_value_obeys_its_declared_type_without_separate_value_type() {
    let mut xml = schema("");
    run(
        &mut xml,
        Primitive::ParameterAdd,
        json!({"items":[{"name":"Count","type":"decimal(10,2)","value":"12.50"}]}),
        &target(&[], ""),
    )
    .unwrap();
    let doc = Document::parse(&xml).unwrap();
    let value = direct(direct(doc.root_element(), S, "parameter"), S, "value");
    assert_eq!(value.attribute((X, "type")), Some("xs:decimal"));
    assert_eq!(value.text(), Some("12.50"));
    for args in [
        json!({"items":[{"name":"Bad","type":"decimal(10,2)","value":"abc"}]}),
        json!({"items":[{"name":"Bad","type":"date","value":"2024-99-99T00:00:00"}]}),
    ] {
        assert_refuses(&schema(""), Primitive::ParameterAdd, args, &target(&[], ""));
    }
}

#[test]
fn standard_period_parameter_value_has_nested_variant_without_custom_only_dates() {
    let mut xml = schema("");
    run(
        &mut xml,
        Primitive::ParameterAdd,
        json!({"items":[{"name":"Period","type":"StandardPeriod","value":"LastMonth"}]}),
        &target(&[], ""),
    )
    .unwrap();
    let doc = Document::parse(&xml).unwrap();
    let value = direct(direct(doc.root_element(), S, "parameter"), S, "value");
    assert_eq!(value.attribute((X, "type")), Some("v8:StandardPeriod"));
    assert_eq!(direct_text(value, V, "variant"), "LastMonth");
    assert_eq!(
        direct(value, V, "variant").attribute((X, "type")),
        Some("v8:StandardPeriodVariant")
    );
    assert_eq!(names(value), ["variant"]);
    assert_refuses(
        &schema(""),
        Primitive::ParameterAdd,
        json!({"items":[{"name":"Period","type":"StandardPeriod","value":"NotAStandardPeriod"}]}),
        &target(&[], ""),
    );
}

#[test]
fn custom_standard_period_settings_value_has_validated_dates() {
    let mut xml = query_schema("SELECT 1 AS Amount");
    run(
        &mut xml,
        Primitive::DataParameterSet,
        json!({"values":{"name":"Period","valueType":"v8:StandardPeriod","value":{"variant":"Custom","startDate":"2026-01-01T00:00:00","endDate":"2026-01-31T23:59:59"}}}),
        &target(&[], "Main"),
    )
    .unwrap();
    let doc = Document::parse(&xml).unwrap();
    let settings = direct(variant(doc.root_element(), "Main"), T, "settings");
    let item = direct(direct(settings, T, "dataParameters"), C, "item");
    let value = direct(item, C, "value");
    assert_eq!(value.attribute((X, "type")), Some("v8:StandardPeriod"));
    assert_eq!(names(value), ["variant", "startDate", "endDate"]);
    assert_eq!(direct_text(value, V, "variant"), "Custom");
    assert_eq!(
        direct(value, V, "variant").attribute((X, "type")),
        Some("v8:StandardPeriodVariant")
    );
    assert_eq!(direct_text(value, V, "startDate"), "2026-01-01T00:00:00");
    assert_eq!(direct_text(value, V, "endDate"), "2026-01-31T23:59:59");
    assert_refuses(
        &xml,
        Primitive::DataParameterSet,
        json!({"values":{"name":"Period","valueType":"v8:StandardPeriod","value":{"variant":"Custom","startDate":"2026-99-01T00:00:00","endDate":"2026-01-31T23:59:59"}}}),
        &target(&[], "Main"),
    );
}

#[test]
fn parameter_value_patch_uses_existing_declared_type() {
    let source = schema("<parameter><name>Count</name><valueType><v8:Type>xs:decimal</v8:Type><v8:NumberQualifiers><v8:Digits>10</v8:Digits><v8:FractionDigits>2</v8:FractionDigits><v8:AllowedSign>Any</v8:AllowedSign></v8:NumberQualifiers></valueType><value xsi:type=\"xs:decimal\">1</value></parameter>");
    let mut xml = source.clone();
    run(
        &mut xml,
        Primitive::ParameterSet,
        json!({"values":{"name":"Count","value":"12.50"}}),
        &target(&[], ""),
    )
    .unwrap();
    let doc = Document::parse(&xml).unwrap();
    let value = direct(direct(doc.root_element(), S, "parameter"), S, "value");
    assert_eq!(value.attribute((X, "type")), Some("xs:decimal"));
    assert_eq!(value.text(), Some("12.50"));
    assert_refuses(
        &source,
        Primitive::ParameterSet,
        json!({"values":{"name":"Count","value":"abc"}}),
        &target(&[], ""),
    );
}

#[test]
fn calculated_field_add_places_expression_before_title_and_type() {
    let mut xml = query_schema("SELECT 1 AS Amount");
    run(&mut xml, Primitive::CalculatedFieldAdd, json!({"items":[{"name":"Double","expression":"Amount * 2","title":"Double","type":"decimal(15,2)"}]}), &target(&["Data"], "")).unwrap();
    let doc = Document::parse(&xml).unwrap();
    let field = direct(doc.root_element(), S, "calculatedField");
    assert_eq!(
        names(field),
        ["dataPath", "expression", "title", "valueType"]
    );
    assert_eq!(direct_text(field, S, "expression"), "Amount * 2");
}

#[test]
fn setting_collections_keep_singletons_and_platform_order_with_spaces() {
    let mut xml = schema("<settingsVariant><t:name>Main</t:name><t:settings>\n  <t:selection/>\n  <t:filter>\n    <t:item xsi:type=\"t:FilterItemComparison\"><t:left xsi:type=\"c:Field\">Amount</t:left><t:comparisonType>Greater</t:comparisonType><t:right xsi:type=\"xs:decimal\">0</t:right></t:item>\n  </t:filter>\n  <t:outputParameters/>\n  <t:item xsi:type=\"t:StructureItemGroup\"/>\n</t:settings></settingsVariant>");
    let selected = target(&[], "Main");
    run(
        &mut xml,
        Primitive::FilterAdd,
        json!({"items":[{"field":"Amount","comparison":"Less","value":100}]}),
        &selected,
    )
    .unwrap();
    run(
        &mut xml,
        Primitive::DataParameterAdd,
        json!({"items":[{"name":"Period","value":"Today","valueType":"v8:StandardPeriod"}]}),
        &selected,
    )
    .unwrap();
    run(
        &mut xml,
        Primitive::OrderAdd,
        json!({"items":[{"field":"Amount","direction":"Desc"}]}),
        &selected,
    )
    .unwrap();
    run(
        &mut xml,
        Primitive::ConditionalAppearanceAdd,
        json!({"items":[{"fields":["Amount"],"appearance":{"TextColor":"web:Red"}}]}),
        &selected,
    )
    .unwrap();
    let doc = Document::parse(&xml).unwrap();
    let settings = direct(variant(doc.root_element(), "Main"), T, "settings");
    assert_eq!(
        names(settings),
        [
            "selection",
            "filter",
            "dataParameters",
            "order",
            "conditionalAppearance",
            "outputParameters",
            "item"
        ]
    );
    assert_eq!(
        settings
            .children()
            .filter(|node| node.has_tag_name((T, "filter")))
            .count(),
        1
    );
    assert_eq!(
        direct(settings, T, "filter")
            .children()
            .filter(|node| node.has_tag_name((T, "item")))
            .count(),
        2
    );
    let item = direct(direct(settings, T, "dataParameters"), C, "item");
    assert_eq!(direct_text(item, C, "parameter"), "Period");
    let period = direct(item, C, "value");
    assert_eq!(direct_text(period, V, "variant"), "Today");
}

#[test]
fn filter_patch_preserves_unprovided_presentation_application_and_user_settings() {
    let untouched = "<t:presentation>Existing</t:presentation><t:application>Items</t:application><t:viewMode>Normal</t:viewMode><t:userSettingID>fixed-id</t:userSettingID><t:userSettingPresentation>Saved</t:userSettingPresentation><u:future xmlns:u='urn:future' a='1'> untouched </u:future>";
    let mut xml = schema(&format!("<settingsVariant><t:name>Main</t:name><t:settings><t:filter><t:item xsi:type=\"t:FilterItemComparison\"><t:left xsi:type=\"c:Field\">Amount</t:left><t:comparisonType>Equal</t:comparisonType>{untouched}</t:item></t:filter></t:settings></settingsVariant>"));
    run(
        &mut xml,
        Primitive::FilterSet,
        json!({"values":{"field":"Amount","comparison":"GreaterOrEqual","value":1}}),
        &target(&[], "Main"),
    )
    .unwrap();
    assert!(
        xml.contains(untouched),
        "patch must preserve unspecified properties"
    );
    let doc = Document::parse(&xml).unwrap();
    let item = direct(
        direct(
            direct(variant(doc.root_element(), "Main"), T, "settings"),
            T,
            "filter",
        ),
        T,
        "item",
    );
    assert_eq!(
        names(item),
        [
            "left",
            "comparisonType",
            "right",
            "presentation",
            "application",
            "viewMode",
            "userSettingID",
            "userSettingPresentation",
            "future"
        ]
    );
    assert_eq!(
        direct(item, T, "right").attribute((X, "type")),
        Some("xs:decimal")
    );
}

#[test]
fn unprovided_view_mode_is_absent_and_filter_defaults_come_from_contract() {
    let mut xml = query_schema("SELECT 1 AS Amount");
    let args = normalized(
        Primitive::FilterAdd,
        json!({"items":[{"dataPath":"Amount","value":0}]}),
    );
    run(&mut xml, Primitive::FilterAdd, args, &target(&[], "Main")).unwrap();
    let doc = Document::parse(&xml).unwrap();
    let item = direct(
        direct(
            direct(variant(doc.root_element(), "Main"), T, "settings"),
            T,
            "filter",
        ),
        T,
        "item",
    );
    assert_eq!(names(item), ["left", "comparisonType", "right"]);
    assert_eq!(direct_text(item, T, "left"), "Amount");
    assert_eq!(direct_text(item, T, "comparisonType"), "Equal");
    for args in [
        json!({"items":[{"field":"Amount","comparison":"Equal","value":"1.2.3","valueType":"xs:decimal"}]}),
        json!({"items":[{"field":"Amount","comparison":"Equal","value":"2024-99-99T00:00:00","valueType":"xs:dateTime"}]}),
    ] {
        assert_refuses(&xml, Primitive::FilterAdd, args, &target(&[], "Main"));
    }
}

#[test]
fn settings_parameter_patch_preserves_explicit_user_settings_and_core_namespace() {
    let untouched = "<t:viewMode>Normal</t:viewMode><t:userSettingID>fixed</t:userSettingID><t:userSettingPresentation>Saved</t:userSettingPresentation><u:future xmlns:u='urn:future' a='1'> untouched </u:future>";
    let mut xml = schema(&format!("<settingsVariant><t:name>Main</t:name><t:settings><t:dataParameters><c:item xsi:type=\"t:SettingsParameterValue\"><c:parameter>Period</c:parameter><c:value xsi:type=\"xs:string\">old</c:value>{untouched}</c:item></t:dataParameters></t:settings></settingsVariant>"));
    run(
        &mut xml,
        Primitive::DataParameterSet,
        json!({"values":{"name":"Period","value":"Today","valueType":"v8:StandardPeriod"}}),
        &target(&[], "Main"),
    )
    .unwrap();
    assert!(xml.contains(untouched));
    let doc = Document::parse(&xml).unwrap();
    let item = direct(
        direct(
            direct(variant(doc.root_element(), "Main"), T, "settings"),
            T,
            "dataParameters",
        ),
        C,
        "item",
    );
    assert_eq!(
        names(item),
        [
            "parameter",
            "value",
            "viewMode",
            "userSettingID",
            "userSettingPresentation",
            "future"
        ]
    );
    assert_eq!(direct_text(item, C, "parameter"), "Period");
    assert_eq!(direct_text(direct(item, C, "value"), V, "variant"), "Today");
}

#[test]
fn query_patch_decodes_entities_cdata_and_literal_empty_replacement() {
    for encoded in [
        "ВЫБРАТЬ \" X \" КАК Category, 1 &lt; 2",
        "ВЫБРАТЬ &quot; X &quot; КАК Category, 1 &lt; 2",
        "<![CDATA[ВЫБРАТЬ \" X \" КАК Category, 1 < 2]]>",
        "ВЫБРАТЬ <![CDATA[\" X \"]]> КАК Category, 1 &lt; 2",
    ] {
        let mut xml = query_schema(encoded);
        let args = normalized(
            Primitive::QueryPatch,
            json!({"values":{"find":" X ","once":true}}),
        );
        run(
            &mut xml,
            Primitive::QueryPatch,
            args,
            &target(&["Data"], ""),
        )
        .unwrap();
        let doc = Document::parse(&xml).unwrap();
        assert_eq!(
            direct_text(direct(doc.root_element(), S, "dataSet"), S, "query"),
            "ВЫБРАТЬ \"\" КАК Category, 1 < 2"
        );
    }
}

#[test]
fn query_patch_keeps_comment_pi_bytes_and_once_refuses_multiple_matches() {
    let original = "А<!-- before -->Б<![CDATA[В]]><?keep original?>Г<!-- after -->Д";
    for (replacement, expected) in [
        ("Ж", "А<!-- before --><?keep original?>Ж<!-- after -->Д"),
        ("", "А<!-- before --><?keep original?><!-- after -->Д"),
    ] {
        let mut xml = query_schema(original);
        run(
            &mut xml,
            Primitive::QueryPatch,
            json!({"values":{"find":"БВГ","replace":replacement,"once":true}}),
            &target(&["Data"], ""),
        )
        .unwrap();
        assert_eq!(xml, query_schema(expected));
    }
    let source = query_schema("SELECT Code AS Code");
    let mut xml = source.clone();
    assert!(run(
        &mut xml,
        Primitive::QueryPatch,
        json!({"values":{"find":"Code","replace":"Item","once":true}}),
        &target(&["Data"], "")
    )
    .is_err());
    assert_eq!(xml, source);
}

#[test]
fn query_operands_are_literal_text_without_control_marker_grammar() {
    let mut xml = query_schema("SELECT Code  =>  Item @once AS Value");
    run(&mut xml, Primitive::QueryPatch, json!({"values":{"find":"Code  =>  Item @once","replace":"Other @once => Value","once":true}}), &target(&["Data"], "")).unwrap();
    let doc = Document::parse(&xml).unwrap();
    assert_eq!(
        direct_text(direct(doc.root_element(), S, "dataSet"), S, "query"),
        "SELECT Other @once => Value AS Value"
    );
}

#[test]
fn nested_union_operations_do_not_select_outer_or_descendant_fields_implicitly() {
    let outer = "<field><dataPath>Value</dataPath><field>Value</field><title>Outer</title></field>";
    let mut xml = schema(&format!("<dataSet xsi:type=\"s:DataSetUnion\"><name>Union</name>{outer}<item xsi:type=\"s:DataSetQuery\"><name>Inner</name><field><dataPath>Value</dataPath><field>Value</field></field><query>SELECT 1 AS Value</query></item></dataSet>"));
    run(
        &mut xml,
        Primitive::FieldSet,
        json!({"values":{"field":"Value","title":"Inner updated","type":"String"}}),
        &target(&["Union", "Inner"], ""),
    )
    .unwrap();
    assert!(xml.contains(outer));
    let doc = Document::parse(&xml).unwrap();
    let inner = named(
        direct(doc.root_element(), S, "dataSet"),
        S,
        "item",
        "name",
        "Inner",
    );
    assert_eq!(
        names(direct(inner, S, "field")),
        ["dataPath", "field", "title"]
    );
    assert_refuses(
        &xml,
        Primitive::QuerySet,
        json!({"values":{"query":"SELECT 2"}}),
        &target(&["Union"], ""),
    );
    assert_refuses(
        &xml,
        Primitive::FieldSet,
        json!({"values":{"field":"Value","title":"must not guess"}}),
        &target(&[], ""),
    );
}

#[test]
fn structure_patch_targets_parent_direct_group_items_and_preserves_child() {
    let child_xml = "<t:item xsi:type=\"t:StructureItemGroup\"><t:name>Child</t:name><t:groupItems><t:item xsi:type=\"t:GroupItemField\"><t:field>Quantity</t:field></t:item></t:groupItems></t:item>";
    let mut xml = schema(&format!("<settingsVariant><t:name>Main</t:name><t:settings><t:item xsi:type=\"t:StructureItemGroup\"><t:name>Parent</t:name>{child_xml}</t:item></t:settings></settingsVariant>"));
    run(
        &mut xml,
        Primitive::StructurePatch,
        json!({"values":{"name":"Parent","groupBy":["Price"]}}),
        &target(&[], "Main"),
    )
    .unwrap();
    assert!(xml.contains(child_xml));
    let doc = Document::parse(&xml).unwrap();
    let parent = direct(
        direct(variant(doc.root_element(), "Main"), T, "settings"),
        T,
        "item",
    );
    assert_eq!(names(parent), ["name", "groupItems", "item"]);
    assert_eq!(
        direct_text(
            direct(direct(parent, T, "groupItems"), T, "item"),
            T,
            "field"
        ),
        "Price"
    );
    assert_refuses(
        &xml,
        Primitive::StructurePatch,
        json!({"values":{"name":"Missing","groupBy":["Amount"]}}),
        &target(&[], "Main"),
    );
    let ambiguous = xml.replace("<t:name>Child</t:name>", "<t:name>Parent</t:name>");
    assert_refuses(
        &ambiguous,
        Primitive::StructurePatch,
        json!({"values":{"name":"Parent","groupBy":["Amount"]}}),
        &target(&[], "Main"),
    );
}

#[test]
fn details_structure_has_no_empty_group_items_and_keeps_settings_before_items() {
    let mut xml = query_schema("SELECT 1 AS Amount");
    run(
        &mut xml,
        Primitive::StructureSet,
        json!({"values":{"groupBy":[],"details":false}}),
        &target(&[], "Main"),
    )
    .unwrap();
    let doc = Document::parse(&xml).unwrap();
    let settings = direct(variant(doc.root_element(), "Main"), T, "settings");
    assert_eq!(names(settings), ["selection", "filter", "order", "item"]);
    assert!(!direct(settings, T, "item")
        .children()
        .any(|node| node.has_tag_name((T, "groupItems"))));
}

#[test]
fn variant_selection_is_explicit_and_other_variant_bytes_remain_exact() {
    let untouched = "<settingsVariant><t:name>Main</t:name><t:settings><t:selection/><t:item xsi:type=\"t:StructureItemGroup\"><t:viewMode>Normal</t:viewMode></t:item></t:settings></settingsVariant>";
    let mut xml = schema(&format!("{untouched}<settingsVariant><t:name>Other</t:name><t:settings><t:selection/></t:settings></settingsVariant>"));
    run(
        &mut xml,
        Primitive::SelectionAdd,
        json!({"items":[{"field":"Amount"}]}),
        &target(&[], "Other"),
    )
    .unwrap();
    assert!(xml.contains(untouched));
    let doc = Document::parse(&xml).unwrap();
    let other = direct(variant(doc.root_element(), "Other"), T, "settings");
    assert_eq!(
        direct_text(direct(direct(other, T, "selection"), T, "item"), T, "field"),
        "Amount"
    );
    assert_refuses(
        &xml,
        Primitive::SelectionAdd,
        json!({"items":[{"field":"Amount"}]}),
        &target(&[], ""),
    );
    assert_refuses(
        &xml,
        Primitive::SelectionAdd,
        json!({"items":[{"field":"Amount"}]}),
        &target(&[], "Missing"),
    );
}

#[test]
fn table_and_chart_children_are_composed_under_the_requested_axes() {
    let mut xml = query_schema("SELECT 1 AS Amount");
    let selected = target(&[], "Main");
    run(&mut xml, Primitive::StructureAdd, json!({"items":[{"name":"Balances","kind":"table","viewMode":"QuickAccess"},{"name":"Chart","kind":"chart"}]}), &selected).unwrap();
    run(&mut xml, Primitive::StructureAdd, json!({"items":[{"name":"ByItem","kind":"group","parent":"Balances","axis":"row","groupBy":["Item"]},{"name":"ByPeriod","kind":"group","parent":"Balances","axis":"column","groupBy":["Period"]},{"name":"ByPoint","kind":"group","parent":"Chart","axis":"point","groupBy":["Item"]},{"name":"BySeries","kind":"group","parent":"Chart","axis":"series","groupBy":["Account"]}]}), &selected).unwrap();
    run(
        &mut xml,
        Primitive::StructureAdd,
        json!({"items":[{"name":"Nested","parent":"ByItem","groupBy":["Account"]}]}),
        &selected,
    )
    .unwrap();
    let doc = Document::parse(&xml).unwrap();
    let settings = direct(variant(doc.root_element(), "Main"), T, "settings");
    let table = named(settings, T, "item", "name", "Balances");
    // A platform axis is itself a structure item. Its groupItems/order/
    // selection are direct children; it does not wrap another group item.
    let row = direct(table, T, "row");
    let column = direct(table, T, "column");
    assert_eq!(direct_text(row, T, "name"), "ByItem");
    assert_eq!(direct_text(column, T, "name"), "ByPeriod");
    assert_eq!(
        direct_text(direct(direct(row, T, "groupItems"), T, "item"), T, "field"),
        "Item"
    );
    assert_eq!(
        direct_text(
            direct(direct(column, T, "groupItems"), T, "item"),
            T,
            "field"
        ),
        "Period"
    );
    assert_eq!(direct_text(direct(row, T, "item"), T, "name"), "Nested");
    assert_eq!(direct_text(table, T, "viewMode"), "QuickAccess");
    let chart = named(settings, T, "item", "name", "Chart");
    assert_eq!(direct_text(direct(chart, T, "point"), T, "name"), "ByPoint");
    assert_eq!(
        direct_text(direct(chart, T, "series"), T, "name"),
        "BySeries"
    );
    assert_refuses(
        &xml,
        Primitive::StructureAdd,
        json!({"items":[{"name":"WrongAxis","parent":"Balances","axis":"point","groupBy":["Amount"]}]}),
        &selected,
    );
}

#[test]
fn root_additions_precede_nested_schema_and_settings_variants() {
    let mut xml = query_schema("SELECT 1 AS Amount")
        .replace("<settingsVariant>", "<nestedSchema/><settingsVariant>");
    run(&mut xml, Primitive::ParameterAdd, json!({"items":[{"name":"P","type":"string","value":"A","useRestriction":true,"expression":"&Source.Choice","valueListAllowed":true,"availableAsField":false}]}), &target(&[], "")).unwrap();
    run(
        &mut xml,
        Primitive::CalculatedFieldAdd,
        json!({"items":[{"name":"Double","expression":"Amount * 2"}]}),
        &target(&[], ""),
    )
    .unwrap();
    run(
        &mut xml,
        Primitive::TotalAdd,
        json!({"items":[{"field":"Amount","expression":"SUM(Amount)","group":"Item"}]}),
        &target(&[], ""),
    )
    .unwrap();
    let doc = Document::parse(&xml).unwrap();
    assert_eq!(
        names(doc.root_element()),
        [
            "dataSource",
            "dataSet",
            "calculatedField",
            "totalField",
            "parameter",
            "nestedSchema",
            "settingsVariant"
        ]
    );
    assert_eq!(
        names(direct(doc.root_element(), S, "parameter")),
        [
            "name",
            "valueType",
            "value",
            "useRestriction",
            "expression",
            "valueListAllowed",
            "availableAsField"
        ]
    );
    assert_eq!(
        direct_text(direct(doc.root_element(), S, "totalField"), S, "group"),
        "Item"
    );
}

#[test]
fn parameter_reorder_moves_only_complete_parameter_images_between_original_slots() {
    let first =
        "<parameter a='1'><name>First</name><value xsi:type=\"xs:string\">A</value></parameter>";
    let second =
        "<parameter a='2'><name>Second</name><value xsi:type=\"xs:string\">B</value></parameter>";
    let marker = "\r\n<!-- between --><?keep original?>\n";
    let original = schema(&format!("{first}{marker}{second}<nestedSchema/>"));
    let mut xml = original.clone();
    run(
        &mut xml,
        Primitive::ParameterReorder,
        json!({"items":[{"name":"Second"},{"name":"First"}]}),
        &target(&[], ""),
    )
    .unwrap();
    assert_eq!(
        xml,
        schema(&format!("{second}{marker}{first}<nestedSchema/>"))
    );
    for args in [
        json!({"items":[{"name":"Second"}]}),
        json!({"items":[{"name":"Second"},{"name":"Second"}]}),
        json!({"items":[{"name":"Second"},{"name":"Missing"}]}),
    ] {
        let mut candidate = original.clone();
        assert!(run(
            &mut candidate,
            Primitive::ParameterReorder,
            args,
            &target(&[], "")
        )
        .is_err());
        assert_eq!(
            candidate, original,
            "reorder preflight must precede replacement"
        );
    }
}

#[test]
fn namespace_spoofs_cannot_be_selected_as_schema_dataset_or_query_type() {
    let foreign = schema("<dataSet xmlns=\"urn:foreign\" xsi:type=\"s:DataSetQuery\"><name>Data</name><query>SELECT 1</query></dataSet>");
    assert_refuses(
        &foreign,
        Primitive::QuerySet,
        json!({"values":{"query":"SELECT 2"}}),
        &target(&["Data"], ""),
    );
    let shadowed_type = schema("<dataSet xmlns:s=\"urn:foreign\" xsi:type=\"s:DataSetQuery\"><name>Data</name><query>SELECT 1</query></dataSet>");
    assert_refuses(
        &shadowed_type,
        Primitive::QuerySet,
        json!({"values":{"query":"SELECT 2"}}),
        &target(&["Data"], ""),
    );
    let wrong_root =
        query_schema("SELECT 1").replacen(&format!("xmlns=\"{S}\""), "xmlns=\"urn:foreign\"", 1);
    assert_refuses(
        &wrong_root,
        Primitive::FieldAdd,
        json!({"items":[{"dataPath":"Added"}]}),
        &target(&["Data"], ""),
    );
}

#[test]
fn field_names_with_old_marker_characters_stay_literal_and_do_not_add_selection() {
    let name = "Amount [шт] @dimension #noField";
    let args = normalized(
        Primitive::FieldAdd,
        json!({"items":[{"dataPath":name,"title":"Literal"}]}),
    );
    let mut xml = query_schema("SELECT 1 AS Amount");
    run(&mut xml, Primitive::FieldAdd, args, &target(&["Data"], "")).unwrap();
    let doc = Document::parse(&xml).unwrap();
    let field = named(
        direct(doc.root_element(), S, "dataSet"),
        S,
        "field",
        "dataPath",
        name,
    );
    assert_eq!(direct_text(field, S, "field"), name);
    assert_eq!(names(field), ["dataPath", "field", "title"]);
    let selection = direct(
        direct(variant(doc.root_element(), "Main"), T, "settings"),
        T,
        "selection",
    );
    assert_eq!(selection.children().filter(Node::is_element).count(), 0);
}

#[test]
fn mapped_field_add_emits_restrictions_and_presentation_in_full_sequence() {
    let mut xml = schema("<dataSet xsi:type=\"s:DataSetObject\"><name>Data</name><objectName>Catalog.Items</objectName></dataSet>");
    let args = normalized(
        Primitive::FieldAdd,
        json!({"items":[{"dataPath":"Amount","field":"Source.Amount","title":"Amount title","useRestriction":{"order":true,"field":false,"group":true,"condition":false},"attributeUseRestriction":{"condition":true},"presentationExpression":"String(Source.Amount)","type":"decimal(15,2)"}]}),
    );
    run(&mut xml, Primitive::FieldAdd, args, &target(&["Data"], "")).unwrap();
    let doc = Document::parse(&xml).unwrap();
    let field = direct(direct(doc.root_element(), S, "dataSet"), S, "field");
    assert_eq!(
        names(field),
        [
            "dataPath",
            "field",
            "title",
            "useRestriction",
            "attributeUseRestriction",
            "presentationExpression",
            "valueType"
        ]
    );
    assert_eq!(direct_text(field, S, "dataPath"), "Amount");
    assert_eq!(direct_text(field, S, "field"), "Source.Amount");
    assert_eq!(
        direct_text(field, S, "presentationExpression"),
        "String(Source.Amount)"
    );
    let restriction = direct(field, S, "useRestriction");
    assert_eq!(names(restriction), ["field", "condition", "group", "order"]);
    assert_eq!(direct_text(restriction, S, "field"), "false");
    assert_eq!(direct_text(restriction, S, "group"), "true");
    assert_eq!(
        direct_text(direct(field, S, "attributeUseRestriction"), S, "condition"),
        "true"
    );
}

#[test]
fn field_mapping_patch_preserves_type_title_roles_and_unprovided_restrictions_exactly() {
    let preserved = "<title xsi:type=\"v8:LocalStringType\"><v8:item><v8:lang>ru</v8:lang><v8:content>Amount</v8:content></v8:item></title><useRestriction><field>true</field></useRestriction><attributeUseRestriction><condition>false</condition></attributeUseRestriction><role><com:dimension>true</com:dimension></role>";
    let typed_tail = "<valueType><v8:Type>xs:decimal</v8:Type></valueType><appearance/><u:future xmlns:u='urn:future'> exact </u:future>";
    let mut xml = schema(&format!("<dataSet xsi:type=\"s:DataSetObject\"><name>Data</name><field><dataPath>Amount</dataPath><field>Old.Amount</field>{preserved}<presentationExpression>Old.Amount</presentationExpression>{typed_tail}</field><objectName>Catalog.Items</objectName></dataSet>"));
    let args = normalized(
        Primitive::FieldSet,
        json!({"values":{"field":"Amount","sourceField":"New.Amount","presentationExpression":"String(New.Amount)"}}),
    );
    run(&mut xml, Primitive::FieldSet, args, &target(&["Data"], "")).unwrap();
    assert!(xml.contains(preserved));
    assert!(xml.contains(typed_tail));
    let doc = Document::parse(&xml).unwrap();
    let field = direct(direct(doc.root_element(), S, "dataSet"), S, "field");
    assert_eq!(direct_text(field, S, "dataPath"), "Amount");
    assert_eq!(direct_text(field, S, "field"), "New.Amount");
    assert_eq!(
        direct_text(field, S, "presentationExpression"),
        "String(New.Amount)"
    );
    assert_eq!(
        names(field),
        [
            "dataPath",
            "field",
            "title",
            "useRestriction",
            "attributeUseRestriction",
            "role",
            "presentationExpression",
            "valueType",
            "appearance",
            "future"
        ]
    );
}

#[test]
fn parameter_multiple_defaults_and_available_values_follow_full_sequence() {
    let mut xml = schema("");
    let args = normalized(
        Primitive::ParameterAdd,
        json!({"items":[{"name":"Choice","title":"Choice","type":"string(20)","values":["A","B"],"useRestriction":true,"expression":"&Source.Choice","availableValues":[{"value":"A","presentation":"Alpha"},{"value":"B","presentation":{"ru":"Бета","en":"Beta"}}],"valueListAllowed":true,"availableAsField":false,"denyIncompleteValues":true,"use":"Always"}]}),
    );
    run(&mut xml, Primitive::ParameterAdd, args, &target(&[], "")).unwrap();
    let doc = Document::parse(&xml).unwrap();
    let parameter = direct(doc.root_element(), S, "parameter");
    assert_eq!(
        names(parameter),
        [
            "name",
            "title",
            "valueType",
            "value",
            "value",
            "useRestriction",
            "expression",
            "availableValue",
            "availableValue",
            "valueListAllowed",
            "availableAsField",
            "denyIncompleteValues",
            "use"
        ]
    );
    let values = parameter
        .children()
        .filter(|node| node.has_tag_name((S, "value")))
        .collect::<Vec<_>>();
    assert_eq!(
        values
            .iter()
            .map(|node| node.text().unwrap())
            .collect::<Vec<_>>(),
        ["A", "B"]
    );
    assert!(values
        .iter()
        .all(|node| node.attribute((X, "type")) == Some("xs:string")));
    let available = parameter
        .children()
        .filter(|node| node.has_tag_name((S, "availableValue")))
        .collect::<Vec<_>>();
    assert_eq!(available.len(), 2);
    for node in &available {
        assert_eq!(names(*node), ["value", "presentation"]);
    }
    assert_eq!(direct_text(available[0], S, "value"), "A");
    assert_eq!(direct_text(parameter, S, "use"), "Always");
    assert_eq!(direct_text(parameter, S, "denyIncompleteValues"), "true");
    assert!(OperationContract::dcs("parameter.add")
        .unwrap()
        .normalize(
            &json!({"items":[{"name":"Bad","availableValues":[{"presentation":"Missing value"}]}]}),
            "args"
        )
        .is_err());
}

#[test]
fn parameter_defaults_patch_and_clear_leave_available_values_and_other_properties_exact() {
    let preserved = "<useRestriction>true</useRestriction><expression>&amp;Source.Choice</expression><availableValue><value xsi:type=\"xs:string\">A</value><presentation>Alpha</presentation></availableValue><valueListAllowed>true</valueListAllowed><availableAsField>false</availableAsField><denyIncompleteValues>true</denyIncompleteValues><use>Always</use><u:future xmlns:u='urn:future'> exact </u:future>";
    let declaration = "<name>Choice</name><title xsi:type=\"xs:string\">Choice</title><valueType><v8:Type>xs:string</v8:Type></valueType>";
    let mut xml = schema(&format!(
        "<parameter>{declaration}<value xsi:type=\"xs:string\">old</value>{preserved}</parameter>"
    ));
    run(
        &mut xml,
        Primitive::ParameterSet,
        normalized(
            Primitive::ParameterSet,
            json!({"values":{"name":"Choice","values":["B","A"]}}),
        ),
        &target(&[], ""),
    )
    .unwrap();
    assert!(xml.contains(declaration));
    assert!(xml.contains(preserved));
    let doc = Document::parse(&xml).unwrap();
    let parameter = direct(doc.root_element(), S, "parameter");
    assert_eq!(
        parameter
            .children()
            .filter(|node| node.has_tag_name((S, "value")))
            .map(|node| node.text().unwrap())
            .collect::<Vec<_>>(),
        ["B", "A"]
    );
    run(
        &mut xml,
        Primitive::ParameterSet,
        normalized(
            Primitive::ParameterSet,
            json!({"values":{"name":"Choice","values":[]}}),
        ),
        &target(&[], ""),
    )
    .unwrap();
    assert!(xml.contains(declaration));
    assert!(xml.contains(preserved));
    let doc = Document::parse(&xml).unwrap();
    let parameter = direct(doc.root_element(), S, "parameter");
    assert_eq!(
        names(parameter),
        [
            "name",
            "title",
            "valueType",
            "useRestriction",
            "expression",
            "availableValue",
            "valueListAllowed",
            "availableAsField",
            "denyIncompleteValues",
            "use",
            "future"
        ]
    );
}

#[test]
fn query_patch_finds_content_after_quoted_greater_than_attributes() {
    let original = query_schema("SELECT 1 AS Amount")
        .replace("<query>", "<query note='a > b' other=\"c > d\">");
    let mut xml = original.clone();
    run(
        &mut xml,
        Primitive::QueryPatch,
        json!({"values":{"find":"1 AS Amount","replace":"2 AS Amount","once":true}}),
        &target(&["Data"], ""),
    )
    .unwrap();
    assert_eq!(
        xml,
        original.replace("SELECT 1 AS Amount", "SELECT 2 AS Amount")
    );
}

#[test]
fn self_closing_prefixed_container_with_tab_and_quoted_attribute_expands_safely() {
    let mut xml = schema("<settingsVariant><t:name>Main</t:name><t:settings><t:selection\tflag='a > b'\t/><!-- exact --></t:settings></settingsVariant>");
    run(
        &mut xml,
        Primitive::SelectionAdd,
        json!({"items":[{"field":"Amount"}]}),
        &target(&[], "Main"),
    )
    .unwrap();
    assert!(xml.contains("<t:selection\tflag='a > b'\t>"));
    assert!(xml.contains("</t:selection><!-- exact -->"));
    let doc = Document::parse(&xml).unwrap();
    let selection = direct(
        direct(variant(doc.root_element(), "Main"), T, "settings"),
        T,
        "selection",
    );
    assert_eq!(selection.attribute("flag"), Some("a > b"));
    assert_eq!(
        direct_text(direct(selection, T, "item"), T, "field"),
        "Amount"
    );
}

fn assert_value_qname(node: Node<'_, '_>, namespace: &str, local: &str) {
    let raw = node.attribute((X, "type")).expect("typed XML value");
    let (prefix, name) = raw.split_once(':').unwrap_or(("", raw));
    assert_eq!(name, local);
    assert_eq!(
        node.lookup_namespace_uri(if prefix.is_empty() {
            None
        } else {
            Some(prefix)
        }),
        Some(namespace)
    );
}

fn assert_translations(node: Node<'_, '_>, expected: &[(&str, &str)]) {
    assert_value_qname(node, V, "LocalStringType");
    let actual = node
        .children()
        .filter(|child| child.has_tag_name((V, "item")))
        .map(|item| {
            (
                direct_text(item, V, "lang"),
                direct_text(item, V, "content"),
            )
        })
        .collect::<std::collections::BTreeMap<_, _>>();
    let expected = expected
        .iter()
        .map(|(lang, text)| (lang.to_string(), text.to_string()))
        .collect::<std::collections::BTreeMap<_, _>>();
    assert_eq!(actual, expected);
}

#[test]
fn repeated_table_rows_are_independent_direct_group_axes() {
    let mut xml = query_schema("SELECT 1 AS Amount");
    let selected = target(&[], "Main");
    for item in [
        json!({"name":"Table","kind":"table"}),
        json!({"name":"FirstRow","parent":"Table","axis":"row","groupBy":["Amount"]}),
        json!({"name":"SecondRow","parent":"Table","axis":"row","groupBy":["Quantity"]}),
    ] {
        let args = normalized(Primitive::StructureAdd, json!({"items":[item]}));
        run(&mut xml, Primitive::StructureAdd, args, &selected).unwrap();
    }
    let doc = Document::parse(&xml).unwrap();
    let settings = direct(variant(doc.root_element(), "Main"), T, "settings");
    let table = named(settings, T, "item", "name", "Table");
    let rows = table
        .children()
        .filter(|n| n.has_tag_name((T, "row")))
        .collect::<Vec<_>>();
    assert_eq!(rows.len(), 2);
    for (row, name, field) in [
        (rows[0], "FirstRow", "Amount"),
        (rows[1], "SecondRow", "Quantity"),
    ] {
        assert_eq!(direct_text(row, T, "name"), name);
        assert_eq!(
            direct_text(direct(direct(row, T, "groupItems"), T, "item"), T, "field"),
            field
        );
        assert!(
            !row.children().any(|n| n.has_tag_name((T, "item"))),
            "axis must not wrap another structure item"
        );
    }
}

#[test]
fn structure_patch_updates_settings_and_empty_group_by_removes_only_own_group_items() {
    let nested = "<t:item xsi:type='t:StructureItemGroup'><t:name>Child</t:name><t:groupItems><t:item xsi:type='t:GroupItemField'><t:field>Quantity</t:field></t:item></t:groupItems></t:item>";
    let unknown = "<!-- exact group note --><u:future xmlns:u='urn:future'> untouched </u:future>";
    let mut xml = schema(&format!("<settingsVariant><t:name>Main</t:name><t:settings><t:item xsi:type='t:StructureItemGroup'><t:name>Group</t:name><t:groupItems><t:item xsi:type='t:GroupItemField'><t:field>Amount</t:field></t:item></t:groupItems>{nested}{unknown}</t:item></t:settings></settingsVariant>"));
    let args = normalized(
        Primitive::StructurePatch,
        json!({"values":{"name":"Group","groupBy":[],"use":false,"viewMode":"QuickAccess","userSettingID":"11111111-1111-4111-8111-111111111111","itemsViewMode":"Inaccessible","userSettingPresentation":{"ru":"По сумме","en":"By amount"}}}),
    );
    run(
        &mut xml,
        Primitive::StructurePatch,
        args,
        &target(&[], "Main"),
    )
    .unwrap();
    assert!(xml.contains(nested));
    assert!(xml.contains(unknown));
    let doc = Document::parse(&xml).unwrap();
    let group = named(
        direct(variant(doc.root_element(), "Main"), T, "settings"),
        T,
        "item",
        "name",
        "Group",
    );
    assert!(!group.children().any(|n| n.has_tag_name((T, "groupItems"))));
    assert_eq!(direct_text(group, T, "use"), "false");
    assert_eq!(direct_text(group, T, "viewMode"), "QuickAccess");
    assert_eq!(direct_text(group, T, "itemsViewMode"), "Inaccessible");
    assert_eq!(
        direct_text(group, T, "userSettingID"),
        "11111111-1111-4111-8111-111111111111"
    );
    assert_translations(
        direct(group, T, "userSettingPresentation"),
        &[("ru", "По сумме"), ("en", "By amount")],
    );
    let known = names(group)
        .into_iter()
        .filter(|name| name != "future")
        .collect::<Vec<_>>();
    assert_eq!(
        known,
        [
            "use",
            "name",
            "item",
            "viewMode",
            "userSettingID",
            "itemsViewMode",
            "userSettingPresentation"
        ]
    );
}

#[test]
fn foreign_qname_cannot_spoof_a_table_axis_parent_or_patchable_group() {
    let original = schema("<settingsVariant><t:name>Main</t:name><t:settings><t:item xmlns:evil='urn:foreign' xsi:type='evil:StructureItemTable'><t:name>Spoof</t:name><t:selection/></t:item></t:settings></settingsVariant>");
    for (op, args) in [
        (
            Primitive::StructureAdd,
            json!({"items":[{"name":"Row","parent":"Spoof","axis":"row","groupBy":["Amount"]}]}),
        ),
        (
            Primitive::StructurePatch,
            json!({"values":{"name":"Spoof","groupBy":["Amount"]}}),
        ),
    ] {
        let mut xml = original.clone();
        let args = normalized(op, args);
        assert!(run(&mut xml, op, args, &target(&[], "Main")).is_err());
        assert_eq!(
            xml, original,
            "a foreign type must never authorize an XML patch"
        );
    }
}

#[test]
fn equal_parameter_values_preserve_original_prefixes_attributes_comments_and_whitespace() {
    let property = format!("<value xmlns:q='http://www.w3.org/2001/XMLSchema' xmlns:z='{X}' z:type = 'q:decimal'>3<!-- original numeric note --></value>");
    let original = schema(&format!("<parameter><name>Count</name><valueType><v8:Type>xs:decimal</v8:Type></valueType>{property}<use>Auto</use></parameter>"));
    let mut xml = original.clone();
    let args = normalized(
        Primitive::ParameterSet,
        json!({"values":{"name":"Count","value":3}}),
    );
    run(&mut xml, Primitive::ParameterSet, args, &target(&[], "")).unwrap();
    assert_eq!(
        xml, original,
        "semantic equality must not normalize the existing property"
    );

    for (operation, container) in [
        (Primitive::DataParameterSet, "dataParameters"),
        (Primitive::OutputParameterSet, "outputParameters"),
    ] {
        let property = format!("<c:value xmlns:q='http://www.w3.org/2001/XMLSchema' xmlns:z='{X}' z:type = 'q:decimal'>3<!-- original numeric note --></c:value>");
        let original = schema(&format!("<settingsVariant><t:name>Main</t:name><t:settings><t:{container}><c:item xsi:type='t:SettingsParameterValue'><c:parameter>Count</c:parameter>{property}<t:viewMode>Normal</t:viewMode></c:item></t:{container}></t:settings></settingsVariant>"));
        let mut xml = original.clone();
        let args = normalized(operation, json!({"values":{"name":"Count","value":3}}));
        run(&mut xml, operation, args, &target(&[], "Main")).unwrap();
        assert_eq!(
            xml,
            original,
            "{} must preserve equal typed property bytes",
            operation.as_str()
        );
    }
}

#[test]
fn data_source_remove_detects_decoded_reference_split_by_comment_and_entity() {
    let original = query_schema("SELECT 1 AS Amount").replace(
        "<dataSource>Local</dataSource>",
        "<dataSource>Lo<!-- reference -->c&#97;l</dataSource>",
    );
    let mut xml = original.clone();
    let args = normalized(
        Primitive::DataSourceRemove,
        json!({"values":{"name":"Local"}}),
    );
    assert!(run(
        &mut xml,
        Primitive::DataSourceRemove,
        args,
        &target(&[], "")
    )
    .is_err());
    assert_eq!(xml, original);
}

#[test]
fn null_parameter_value_is_nil_instead_of_an_empty_string_or_literal_null() {
    let mut xml = schema("");
    let args = normalized(
        Primitive::ParameterAdd,
        json!({"items":[{"name":"Optional","type":"string","value":null}]}),
    );
    run(&mut xml, Primitive::ParameterAdd, args, &target(&[], "")).unwrap();
    let doc = Document::parse(&xml).unwrap();
    let value = direct(
        named(doc.root_element(), S, "parameter", "name", "Optional"),
        S,
        "value",
    );
    assert_eq!(value.attribute((X, "nil")), Some("true"));
    assert_eq!(value.text(), None);
    assert!(!value.children().any(|node| node.is_element()));
}

#[test]
fn catalog_reference_default_is_design_time_value_with_declared_reference_type() {
    let mut xml = schema("");
    let args = normalized(
        Primitive::ParameterAdd,
        json!({"items":[{"name":"Item","type":"CatalogRef.Items","value":"Catalog.Items.EmptyRef"}]}),
    );
    run(&mut xml, Primitive::ParameterAdd, args, &target(&[], "")).unwrap();
    let doc = Document::parse(&xml).unwrap();
    let parameter = named(doc.root_element(), S, "parameter", "name", "Item");
    let type_node = direct(direct(parameter, S, "valueType"), V, "Type");
    let declared = type_node.text().unwrap();
    let (prefix, local) = declared.split_once(':').expect("reference QName");
    assert_eq!(local, "CatalogRef.Items");
    assert_eq!(
        type_node.lookup_namespace_uri(Some(prefix)),
        Some("http://v8.1c.ru/8.1/data/enterprise/current-config")
    );
    let value = direct(parameter, S, "value");
    assert_value_qname(value, C, "DesignTimeValue");
    assert_eq!(value.text(), Some("Catalog.Items.EmptyRef"));
    assert_eq!(value.attribute((X, "nil")), None);
}

#[test]
fn conditional_appearance_emits_use_filter_color_and_multilingual_format_values() {
    const UI: &str = "http://v8.1c.ru/8.1/data/ui";
    const WEB: &str = "http://v8.1c.ru/8.1/data/ui/colors/web";
    let mut xml = query_schema("SELECT 1 AS Amount");
    let args = normalized(
        Primitive::ConditionalAppearanceAdd,
        json!({"items":[{"fields":["Amount"],"use":false,"filter":[{"field":"Amount","comparison":"Less","value":0}],"appearance":{"ЦветТекста":{"valueType":"v8ui:Color","value":"web:Red"},"Формат":{"valueType":"v8:LocalStringType","value":{"ru":"ЧДЦ=2","en":"ND=2"}}}}]}),
    );
    run(
        &mut xml,
        Primitive::ConditionalAppearanceAdd,
        args,
        &target(&[], "Main"),
    )
    .unwrap();
    let doc = Document::parse(&xml).unwrap();
    let settings = direct(variant(doc.root_element(), "Main"), T, "settings");
    let item = direct(direct(settings, T, "conditionalAppearance"), T, "item");
    assert_eq!(names(item), ["use", "selection", "filter", "appearance"]);
    assert_eq!(direct_text(item, T, "use"), "false");
    let selection = direct(direct(item, T, "selection"), T, "item");
    assert_eq!(direct_text(selection, T, "field"), "Amount");
    let comparison = direct(direct(item, T, "filter"), T, "item");
    assert_value_qname(comparison, T, "FilterItemComparison");
    assert_eq!(direct_text(comparison, T, "left"), "Amount");
    assert_value_qname(direct(comparison, T, "left"), C, "Field");
    assert_eq!(direct_text(comparison, T, "comparisonType"), "Less");
    let right = direct(comparison, T, "right");
    assert_value_qname(right, "http://www.w3.org/2001/XMLSchema", "decimal");
    assert_eq!(right.text(), Some("0"));
    let appearance = direct(item, T, "appearance");
    let color_item = named(appearance, C, "item", "parameter", "ЦветТекста");
    assert_value_qname(color_item, T, "SettingsParameterValue");
    let color = direct(color_item, C, "value");
    assert_value_qname(color, UI, "Color");
    let (prefix, name) = color.text().unwrap().split_once(':').unwrap();
    assert_eq!(name, "Red");
    assert_eq!(color.lookup_namespace_uri(Some(prefix)), Some(WEB));
    assert_translations(
        direct(
            named(appearance, C, "item", "parameter", "Формат"),
            C,
            "value",
        ),
        &[("ru", "ЧДЦ=2"), ("en", "ND=2")],
    );
}

#[test]
fn output_title_uses_core_parameter_item_and_localized_string_instead_of_json_text() {
    let mut xml = query_schema("SELECT 1 AS Amount");
    let args = normalized(
        Primitive::OutputParameterSet,
        json!({"values":{"name":"Заголовок","use":false,"valueType":"v8:LocalStringType","value":{"ru":"Отчёт по остаткам","en":"Balances report"}}}),
    );
    run(
        &mut xml,
        Primitive::OutputParameterSet,
        args,
        &target(&[], "Main"),
    )
    .unwrap();
    let doc = Document::parse(&xml).unwrap();
    let settings = direct(variant(doc.root_element(), "Main"), T, "settings");
    let item = named(
        direct(settings, T, "outputParameters"),
        C,
        "item",
        "parameter",
        "Заголовок",
    );
    assert_value_qname(item, T, "SettingsParameterValue");
    assert_eq!(names(item), ["use", "parameter", "value"]);
    assert_eq!(direct_text(item, C, "use"), "false");
    assert_translations(
        direct(item, C, "value"),
        &[("ru", "Отчёт по остаткам"), ("en", "Balances report")],
    );
}

#[test]
fn empty_string_patches_clear_parameter_value_expression_and_field_presentation_expression() {
    let untouched = "<u:future xmlns:u='urn:future'> untouched </u:future>";
    let mut xml = query_schema("SELECT 1 AS Amount")
        .replace("<field>Amount</field></field>", "<field>Amount</field><presentationExpression>Amount + 1</presentationExpression></field>")
        .replace("<settingsVariant>", &format!("<parameter><name>Choice</name><valueType><v8:Type>xs:string</v8:Type></valueType><value xsi:type='xs:string'>old default</value><expression>old expression</expression>{untouched}</parameter><settingsVariant>"));
    let args = normalized(
        Primitive::ParameterSet,
        json!({"values":{"name":"Choice","value":"","expression":""}}),
    );
    run(&mut xml, Primitive::ParameterSet, args, &target(&[], "")).unwrap();
    {
        let doc = Document::parse(&xml).unwrap();
        let parameter = named(doc.root_element(), S, "parameter", "name", "Choice");
        let value = direct(parameter, S, "value");
        assert_value_qname(value, "http://www.w3.org/2001/XMLSchema", "string");
        assert_eq!(direct_text(parameter, S, "value"), "");
        assert_eq!(direct_text(parameter, S, "expression"), "");
        assert_eq!(
            direct_text(direct(parameter, S, "valueType"), V, "Type"),
            "xs:string"
        );
    }
    let args = normalized(
        Primitive::FieldSet,
        json!({"values":{"field":"Amount","presentationExpression":""}}),
    );
    run(&mut xml, Primitive::FieldSet, args, &target(&["Data"], "")).unwrap();
    let doc = Document::parse(&xml).unwrap();
    let field = named(
        direct(doc.root_element(), S, "dataSet"),
        S,
        "field",
        "dataPath",
        "Amount",
    );
    assert_eq!(direct_text(field, S, "presentationExpression"), "");
    assert_eq!(direct_text(field, S, "field"), "Amount");
    assert_eq!(
        direct_text(
            named(doc.root_element(), S, "parameter", "name", "Choice"),
            S,
            "value"
        ),
        "",
        "later field patch must retain the earlier parameter edit"
    );
    assert!(xml.contains(untouched));
    for old in ["old default", "old expression", "Amount + 1"] {
        assert!(!xml.contains(old));
    }
}

#[test]
fn equal_typed_string_preserves_split_text_cdata_comment_bytes_but_spaces_are_not_empty() {
    for (old_content, requested, same) in [
        ("a<![CDATA[b]]><!--note-->c", "abc", true),
        (" <!--note--> ", "", false),
    ] {
        let original = schema(&format!("<parameter><name>Text</name><valueType><v8:Type>xs:string</v8:Type></valueType><value xmlns:q='http://www.w3.org/2001/XMLSchema' xsi:type = 'q:string'>{old_content}</value></parameter>"));
        let mut xml = original.clone();
        let args = normalized(
            Primitive::ParameterSet,
            json!({"values":{"name":"Text","value":requested}}),
        );
        run(&mut xml, Primitive::ParameterSet, args, &target(&[], "")).unwrap();
        if same {
            assert_eq!(xml, original, "equal decoded text must retain original CDATA, comment, prefix and attribute bytes");
        } else {
            assert_ne!(
                xml, original,
                "two spaces are a nonempty string and must actually be cleared"
            );
        }
        let doc = Document::parse(&xml).unwrap();
        let parameter = named(doc.root_element(), S, "parameter", "name", "Text");
        let value = direct(parameter, S, "value");
        assert_value_qname(value, "http://www.w3.org/2001/XMLSchema", "string");
        assert_eq!(direct_text(parameter, S, "value"), requested);
    }
}

#[test]
fn dcs_component_writer_refuses_wrong_existing_root_without_a_postimage() {
    let original = "<garbage/>";
    let mut candidate = original.to_string();
    let error = apply(
        &mut candidate,
        "dataSource.add",
        json!({"items":[{"name":"Data","kind":"Local"}]})
            .as_object()
            .unwrap(),
        &Target {
            datasets: Vec::new(),
            variant: String::new(),
            terminal: None,
        },
    )
    .expect_err("a DCS component writer must refuse a foreign XML root");
    assert!(error.contains("DataCompositionSchema"), "{error}");
    assert_eq!(
        candidate, original,
        "a refusal must not return a publishable postimage"
    );
}

fn link_schema(body: &str) -> String {
    schema(&format!("<dataSource><name>Local</name><dataSourceType>Local</dataSourceType></dataSource><dataSet xsi:type='s:DataSetQuery'><name>Left</name><dataSource>Local</dataSource><query>SELECT 1 AS Amount</query></dataSet><dataSet xsi:type='s:DataSetQuery'><name>Right</name><dataSource>Local</dataSource><query>SELECT 1 AS Amount</query></dataSet>{body}"))
}

fn assert_link_read_flags(xml: &str, list_allowed: Option<bool>, required: Option<bool>) {
    let data = crate::infrastructure::native_operations::dcs::parse_dcs_info_xml(
        xml,
        crate::domain::support_state::ObjectSupportData {
            state: crate::domain::support_state::ObjectSupportState::NotSupported,
            direct_edit_safe: None,
        },
    )
    .unwrap();
    let facts = serde_json::to_value(data).unwrap();
    assert_eq!(facts["links"].as_array().unwrap().len(), 1);
    assert_eq!(
        facts["links"][0]["parameterListAllowed"],
        json!(list_allowed)
    );
    assert_eq!(facts["links"][0]["required"], json!(required));
}

#[test]
fn dataset_link_add_keeps_platform_defaults_implicit_and_nondefaults_in_xsd_order() {
    for (flags, expected_flags, read_flags) in [
        (json!({}), false, (None, None)),
        (
            json!({"parameterListAllowed":false,"required":true}),
            false,
            (None, None),
        ),
        (
            json!({"parameterListAllowed":true,"required":false}),
            true,
            (Some(true), Some(false)),
        ),
    ] {
        let mut item = json!({"source":"Left","destination":"Right","sourceExpression":"Amount","destinationExpression":"Amount","parameter":"P","condition":"Amount > 0","startExpression":"Amount"});
        item.as_object_mut()
            .unwrap()
            .extend(flags.as_object().unwrap().clone());
        let mut xml = link_schema("");
        let args = normalized(Primitive::DataSetLinkAdd, json!({"items":[item]}));
        run(&mut xml, Primitive::DataSetLinkAdd, args, &target(&[], "")).unwrap();
        let doc = Document::parse(&xml).unwrap();
        let link = direct(doc.root_element(), S, "dataSetLink");
        let expected = if expected_flags {
            vec![
                "sourceDataSet",
                "destinationDataSet",
                "sourceExpression",
                "destinationExpression",
                "parameter",
                "parameterListAllowed",
                "linkConditionExpression",
                "startExpression",
                "required",
            ]
        } else {
            vec![
                "sourceDataSet",
                "destinationDataSet",
                "sourceExpression",
                "destinationExpression",
                "parameter",
                "linkConditionExpression",
                "startExpression",
            ]
        };
        assert_eq!(names(link), expected);
        if expected_flags {
            assert_eq!(direct_text(link, S, "parameterListAllowed"), "true");
            assert_eq!(direct_text(link, S, "required"), "false");
        }
        assert_eq!(
            direct_text(link, S, "linkConditionExpression"),
            "Amount > 0"
        );
        assert_link_read_flags(&xml, read_flags.0, read_flags.1);
    }
}

#[test]
fn dataset_link_set_removes_only_supplied_nondefault_flag_and_keeps_adjacent_bytes() {
    let list_flag = "<s:parameterListAllowed data-preserve='exact'>true<!-- list flag note --></s:parameterListAllowed>";
    let untouched =
        "<!-- link note --><?keep exact?><u:future xmlns:u='urn:future'>  unowned </u:future>";
    let original = link_schema(&format!("<dataSetLink><sourceDataSet>Left</sourceDataSet><destinationDataSet>Right</destinationDataSet><sourceExpression>Amount</sourceExpression><destinationExpression>Amount</destinationExpression><parameter>P</parameter>{list_flag}<linkConditionExpression>Amount &gt; 0</linkConditionExpression>{untouched}<startExpression>Amount</startExpression><required>false</required></dataSetLink>"));
    let selector = json!({"source":"Left","destination":"Right","sourceExpression":"Amount","destinationExpression":"Amount"});
    let mut xml = original.clone();
    let args = normalized(
        Primitive::DataSetLinkSet,
        json!({"values":{"selector":selector,"required":true}}),
    );
    run(&mut xml, Primitive::DataSetLinkSet, args, &target(&[], "")).unwrap();
    assert_eq!(xml, original.replace("<required>false</required>", ""));
    assert!(
        xml.contains(list_flag),
        "unsupplied flag is not a reset to default"
    );
    assert_link_read_flags(&xml, Some(true), None);
    let before_list_reset = xml.clone();
    let args = normalized(
        Primitive::DataSetLinkSet,
        json!({"values":{"selector":selector,"parameterListAllowed":false}}),
    );
    run(&mut xml, Primitive::DataSetLinkSet, args, &target(&[], "")).unwrap();
    assert_eq!(xml, before_list_reset.replace(list_flag, ""));
    assert!(xml.contains(untouched));
    assert_link_read_flags(&xml, None, None);
    let after_reset = xml.clone();
    let args = normalized(
        Primitive::DataSetLinkSet,
        json!({"values":{"selector":selector,"parameterListAllowed":false,"required":true}}),
    );
    run(&mut xml, Primitive::DataSetLinkSet, args, &target(&[], "")).unwrap();
    assert_eq!(xml, after_reset, "repeated defaults must be a no-op");
}

#[test]
fn dataset_link_semantically_equal_explicit_default_flags_preserve_literal_bytes() {
    for (list, required) in [
        ("false", "true"),
        ("0", "1"),
        (" <!--note-->0", "tr<![CDATA[ue]]>"),
        ("f<![CDATA[al]]><!--note-->se", " <!--note-->1 "),
    ] {
        let original = link_schema(&format!("<dataSetLink><sourceDataSet>Left</sourceDataSet><destinationDataSet>Right</destinationDataSet><sourceExpression>Amount</sourceExpression><destinationExpression>Amount</destinationExpression><parameterListAllowed>{list}<!-- explicit default note --></parameterListAllowed><required xmlns:q='urn:unowned' q:keep='exact'>{required}</required><!-- tail --></dataSetLink>"));
        let mut xml = original.clone();
        let args = normalized(
            Primitive::DataSetLinkSet,
            json!({"values":{"selector":{"source":"Left","destination":"Right","sourceExpression":"Amount","destinationExpression":"Amount"},"parameterListAllowed":false,"required":true}}),
        );
        run(&mut xml, Primitive::DataSetLinkSet, args, &target(&[], "")).unwrap();
        assert_eq!(
            xml, original,
            "a semantic no-op must not canonicalize explicit defaults"
        );
        assert_link_read_flags(&xml, Some(false), Some(true));
    }
}

#[test]
fn dataset_link_read_preserves_complete_literal_selectors_for_set_and_noop() {
    let source = "<sourceDataSet>Le<![CDATA[ft]]></sourceDataSet>";
    let destination = "<destinationDataSet>Ri<!-- endpoint -->ght</destinationDataSet>";
    let source_expression = "<sourceExpression> A<![CDATA[mount]]> </sourceExpression>";
    let destination_expression =
        "<destinationExpression> A<?keep expression?>mount </destinationExpression>";
    let parameter = "<parameter> P<![CDATA[eriod]]> </parameter>";
    let start_expression = "<startExpression> A<![CDATA[mount]]> </startExpression>";
    let condition =
        "<linkConditionExpression> Amount &gt; <!-- comparison -->0 </linkConditionExpression>";
    let original = link_schema(&format!("<dataSetLink>{source}{destination}{source_expression}{destination_expression}{parameter}<parameterListAllowed> f<!-- false -->alse </parameterListAllowed>{condition}{start_expression}<required>tr<?keep bool?>ue</required></dataSetLink>"));
    let facts = crate::infrastructure::native_operations::dcs::parse_dcs_info_xml(
        &original,
        crate::domain::support_state::ObjectSupportData {
            state: crate::domain::support_state::ObjectSupportState::NotSupported,
            direct_edit_safe: None,
        },
    )
    .unwrap();
    let facts = serde_json::to_value(facts).unwrap();
    let link = &facts["links"][0];
    for (key, expected) in [
        ("source", "Left"),
        ("destination", "Right"),
        ("sourceExpression", " Amount "),
        ("destinationExpression", " Amount "),
        ("parameter", " Period "),
        ("condition", " Amount > 0 "),
        ("startExpression", " Amount "),
    ] {
        assert_eq!(link[key], json!(expected), "complete literal {key}");
    }
    assert_eq!(link["parameterListAllowed"], json!(false));
    assert_eq!(link["required"], json!(true));
    let selector = json!({
        "source":link["source"],
        "destination":link["destination"],
        "sourceExpression":link["sourceExpression"],
        "destinationExpression":link["destinationExpression"]
    });
    let mut xml = original.clone();
    let args = normalized(
        Primitive::DataSetLinkSet,
        json!({"values":{"selector":selector,"parameterListAllowed":false,"required":true,"condition":" Amount > 0 "}}),
    );
    run(&mut xml, Primitive::DataSetLinkSet, args, &target(&[], "")).unwrap();
    assert_eq!(
        xml, original,
        "facts must locate the link without changing equal bytes"
    );
    let args = normalized(
        Primitive::DataSetLinkSet,
        json!({"values":{"selector":selector,"parameterListAllowed":true,"condition":" Amount > 1 "}}),
    );
    run(&mut xml, Primitive::DataSetLinkSet, args, &target(&[], "")).unwrap();
    let document = Document::parse(&xml).unwrap();
    let link = direct(document.root_element(), S, "dataSetLink");
    assert_eq!(direct_text(link, S, "parameterListAllowed"), "true");
    assert_eq!(
        direct_text(link, S, "linkConditionExpression"),
        " Amount > 1 "
    );
    for untouched in [
        source,
        destination,
        source_expression,
        destination_expression,
        parameter,
        start_expression,
    ] {
        assert!(
            xml.contains(untouched),
            "unselected scalar XML must stay exact"
        );
    }
    let after_change = xml.clone();
    let args = normalized(
        Primitive::DataSetLinkSet,
        json!({"values":{"selector":selector,"parameterListAllowed":true,"condition":" Amount > 1 "}}),
    );
    run(&mut xml, Primitive::DataSetLinkSet, args, &target(&[], "")).unwrap();
    assert_eq!(
        xml, after_change,
        "repeated targeted set must preserve the postimage"
    );
}
