//! rule: CTR.SOURCE.EMPTY-BRANCHES

use super::{install_form_metadata_test_sources, write, RealReaderFixture, ViewRequest};
use serde_json::{json, Value};
use std::fs;
use unica_workspace::test_support::tree_snapshot;

fn read(fixture: &RealReaderFixture, at: &str) -> Value {
    // A branch count covers its whole collection, not the default 20-row
    // page. These fixtures fit within one explicit public 50-row page.
    let result = fixture
        .view_service()
        .view(ViewRequest::new(at).unwrap().with_limit(50).unwrap());
    assert!(result.ok, "{at}: {result:?}");
    result.data.unwrap()
}

fn branch(fixture: &RealReaderFixture, owner: &str, kind: &str, expected: &[&str]) {
    let items = branch_items(fixture, owner, kind, expected.len());
    let addresses = items
        .iter()
        .map(|row| {
            row["at"]
                .as_str()
                .unwrap_or_else(|| panic!("addressable row expected: {row}"))
        })
        .collect::<Vec<_>>();
    assert_eq!(addresses, expected, "{owner}.{kind}: {items:?}");
    for child in expected {
        assert_eq!(read(fixture, child)["at"], *child);
    }
}

fn branch_items(
    fixture: &RealReaderFixture,
    owner: &str,
    kind: &str,
    expected_count: usize,
) -> Vec<Value> {
    let at = format!("{owner}.{kind}");
    let node = read(fixture, owner);
    let matching = node["branches"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|row| row["at"] == at)
        .collect::<Vec<_>>();
    assert_eq!(
        matching.len(),
        1,
        "{at}: missing or duplicate branch: {node}"
    );
    assert_eq!(matching[0]["count"], expected_count, "{at}: {node}");
    let collection = read(fixture, &at);
    let items = collection["items"].as_array().expect("branch items");
    assert_eq!(items.len(), expected_count, "{at}: {collection}");
    items.clone()
}

fn branch_rows(fixture: &RealReaderFixture, owner: &str, kind: &str, expected: &[Value]) {
    let items = branch_items(fixture, owner, kind, expected.len());
    assert_eq!(items, expected, "{owner}.{kind}");
    for at in expected.iter().filter_map(|row| row["at"].as_str()) {
        assert_eq!(read(fixture, at)["at"], at);
    }
}

fn absent_branch(fixture: &RealReaderFixture, owner: &str, kind: &str) {
    let at = format!("{owner}.{kind}");
    let node = read(fixture, owner);
    assert!(
        node["branches"]
            .as_array()
            .into_iter()
            .flatten()
            .all(|row| row["at"] != at),
        "{node}"
    );
    if let Ok(request) = ViewRequest::new(&at) {
        let result = fixture.view_service().view(request);
        assert!(
            !result.ok,
            "inapplicable branch succeeded: {at}: {result:?}"
        );
    }
}

// Reduce the existing real-reader XML fixture without replacing it by a
// provider payload. Removing whole elements preserves their admitted owner,
// namespaces and source registration. Nested matching elements are removed
// only with their outermost matching ancestor.
fn remove_elements(fixture: &RealReaderFixture, relative: &str, tags: &[&str]) {
    let path = fixture.source.join(relative);
    let mut xml = fs::read_to_string(&path).unwrap();
    let ranges = {
        let document = roxmltree::Document::parse(&xml).unwrap();
        document
            .descendants()
            .filter(|node| {
                node.is_element()
                    && tags.contains(&node.tag_name().name())
                    && !node.ancestors().skip(1).any(|parent| {
                        parent.is_element() && tags.contains(&parent.tag_name().name())
                    })
            })
            .map(|node| node.range())
            .collect::<Vec<_>>()
    };
    assert!(
        !ranges.is_empty(),
        "fixture contains none of {tags:?}: {relative}"
    );
    for range in ranges.into_iter().rev() {
        xml.replace_range(range, "");
    }
    write(&path, &xml);
}

/// rule: INV.SOURCE.LOGICAL-BRANCH-COUNT
#[test]
fn metadata_tabular_section_keeps_empty_attribute_and_standard_attribute_branches() {
    let fixture = RealReaderFixture::new();
    let owner = "main:Catalog.Items.TabularSection.Lines";
    branch(
        &fixture,
        owner,
        "Attribute",
        &["main:Catalog.Items.TabularSection.Lines.Attribute.Quantity"],
    );
    // The source fixture omits StandardAttributes altogether. That is not
    // evidence of an admitted empty collection. Explicitly declare the empty
    // container before asserting count=0; corrupt declarations have their
    // own real-reader coverage in metadata_declaration_diagnostics.
    absent_branch(&fixture, owner, "StandardAttribute");
    let path = fixture.source.join("Catalogs/Items.xml");
    let xml = fs::read_to_string(&path).unwrap();
    let name = "<Name>Lines</Name>";
    assert_eq!(
        xml.matches(name).count(),
        1,
        "tabular section fixture owner"
    );
    write(
        &path,
        &xml.replace(name, "<Name>Lines</Name><StandardAttributes/>"),
    );
    remove_elements(&fixture, "Catalogs/Items.xml", &["Attribute"]);
    let before = tree_snapshot(&fixture.source);
    branch(&fixture, owner, "Attribute", &[]);
    branch(&fixture, owner, "StandardAttribute", &[]);
    assert_eq!(tree_snapshot(&fixture.source), before);
}

/// rule: INV.SOURCE.LOGICAL-BRANCH-COUNT
#[test]
fn admitted_standard_tabular_section_keeps_its_empty_standard_attribute_branch() {
    let fixture = RealReaderFixture::new();
    let owner = "main:ChartOfAccounts.Accounts.StandardTabularSection.ExtDimensionTypes";
    branch(&fixture, owner, "StandardAttribute", &["main:ChartOfAccounts.Accounts.StandardTabularSection.ExtDimensionTypes.StandardAttribute.ExtDimensionType"]);
    remove_elements(
        &fixture,
        "ChartsOfAccounts/Accounts.xml",
        &["StandardAttribute"],
    );
    branch(&fixture, owner, "StandardAttribute", &[]);
    absent_branch(&fixture, owner, "Parameter");
}

/// rule: INV.SOURCE.LOGICAL-BRANCH-COUNT
#[test]
fn http_url_template_keeps_its_empty_method_branch() {
    let fixture = RealReaderFixture::new();
    let owner = "main:HTTPService.ExternalAPI.URLTemplate.Metrics";
    branch(
        &fixture,
        owner,
        "Method",
        &["main:HTTPService.ExternalAPI.URLTemplate.Metrics.Method.Get"],
    );
    remove_elements(&fixture, "HTTPServices/ExternalAPI.xml", &["Method"]);
    branch(&fixture, owner, "Method", &[]);
    absent_branch(&fixture, owner, "Parameter");
}

/// rule: INV.SOURCE.LOGICAL-BRANCH-COUNT
#[test]
fn web_service_operation_keeps_its_empty_parameter_branch() {
    let fixture = RealReaderFixture::new();
    let owner = "main:WebService.Exchange.Operation.Send";
    branch(
        &fixture,
        owner,
        "Parameter",
        &["main:WebService.Exchange.Operation.Send.Parameter.Payload"],
    );
    remove_elements(&fixture, "WebServices/Exchange.xml", &["Parameter"]);
    branch(&fixture, owner, "Parameter", &[]);
    absent_branch(&fixture, owner, "Method");
}

/// rule: INV.SOURCE.LOGICAL-BRANCH-COUNT
#[test]
fn form_and_common_form_keep_empty_owned_collections() {
    let fixture = RealReaderFixture::new();
    install_form_metadata_test_sources(&fixture);
    for owner in [
        "main:CommonForm.ОбщаяФорма",
        "main:DataProcessor.Обработка.Form.Форма",
    ] {
        for kind in ["Item", "Attribute", "Parameter", "Command"] {
            branch(&fixture, owner, kind, &[]);
        }
        // The actual descriptor remains available independently of the empty
        // managed-form content. Event applicability comes from its catalog,
        // not from the absence of explicit XML event bindings.
        assert_eq!(
            read(&fixture, &format!("{owner}.Metadata"))["at"],
            format!("{owner}.Metadata")
        );
        absent_branch(&fixture, owner, "Area");
    }
}

/// rule: INV.SOURCE.LOGICAL-BRANCH-COUNT
#[test]
fn form_nested_table_keeps_empty_children_and_nonempty_traversal_order() {
    let fixture = RealReaderFixture::new();
    let owner = "main:Report.ParityReport.Form.MainForm.Item.Goods";
    branch(
        &fixture,
        owner,
        "Item",
        &["main:Report.ParityReport.Form.MainForm.Item.Goods.Item.Quantity"],
    );
    remove_elements(
        &fixture,
        "Reports/ParityReport/Forms/MainForm/Ext/Form.xml",
        &["InputField"],
    );
    branch(&fixture, owner, "Item", &[]);
}

/// rule: INV.SOURCE.LOGICAL-BRANCH-COUNT
#[test]
fn dcs_template_keeps_all_empty_admitted_collections() {
    let fixture = RealReaderFixture::new();
    let owner = "main:Report.ParityReport.Template.MainSchema";
    branch(
        &fixture,
        owner,
        "DataSet",
        &["main:Report.ParityReport.Template.MainSchema.DataSet.MainData"],
    );
    branch(
        &fixture,
        owner,
        "Parameter",
        &["main:Report.ParityReport.Template.MainSchema.Parameter.Period"],
    );
    branch(
        &fixture,
        owner,
        "Setting",
        &["main:Report.ParityReport.Template.MainSchema.Setting.Основной"],
    );
    remove_elements(
        &fixture,
        "Reports/ParityReport/Templates/MainSchema/Ext/Template.xml",
        &["dataSource", "dataSet", "parameter", "settingsVariant"],
    );
    for kind in [
        "DataSource",
        "DataSet",
        "CalculatedField",
        "Total",
        "Parameter",
        "Setting",
        "Item",
    ] {
        branch(&fixture, owner, kind, &[]);
    }
    absent_branch(&fixture, owner, "Area");
}

/// rule: INV.SOURCE.LOGICAL-BRANCH-COUNT
#[test]
fn dcs_query_dataset_keeps_empty_fields_and_parameters_without_inventing_children() {
    let fixture = RealReaderFixture::new();
    let owner = "main:Report.ParityReport.Template.MainSchema.DataSet.MainData";
    branch(
        &fixture,
        owner,
        "Field",
        &["main:Report.ParityReport.Template.MainSchema.DataSet.MainData.Field.Code"],
    );
    remove_elements(
        &fixture,
        "Reports/ParityReport/Templates/MainSchema/Ext/Template.xml",
        &["field", "parameter"],
    );
    branch(&fixture, owner, "Field", &[]);
    branch(&fixture, owner, "Parameter", &[]);
    // Query is a singleton source text, not an empty collection.
    assert_eq!(
        read(&fixture, &format!("{owner}.Query"))["at"],
        format!("{owner}.Query")
    );
    absent_branch(&fixture, owner, "DataSet");
}

/// rule: INV.SOURCE.LOGICAL-BRANCH-COUNT
#[test]
fn dcs_setting_keeps_its_empty_structural_item_branch() {
    let fixture = RealReaderFixture::new();
    branch(
        &fixture,
        "main:Report.ParityReport.Template.MainSchema.Setting.Основной",
        "Item",
        &[],
    );
}

/// rule: INV.SOURCE.LOGICAL-BRANCH-COUNT
#[test]
fn mxl_template_and_area_keep_empty_area_and_parameter_branches() {
    let fixture = RealReaderFixture::new();
    let owner = "main:Report.ParityReport.Template.Print";
    branch(
        &fixture,
        owner,
        "Area",
        &["main:Report.ParityReport.Template.Print.Area.Header"],
    );
    branch_rows(
        &fixture,
        "main:Report.ParityReport.Template.Print.Area.Header",
        "Parameter",
        &[
            json!({"at":"main:Report.ParityReport.Template.Print.Area.Header.Parameter.Title", "kind":"Parameter", "title":"Title"}),
            json!({"kind":"Parameter", "title":"Since [tpl]", "addressable":false}),
        ],
    );
    branch_rows(
        &fixture,
        "main:Report.ParityReport.Template.Print.Area.Header",
        "Body",
        &[
            json!({"index":1,"template":false,"text":"Sales report"}),
            json!({"index":2,"template":true,"text":"Period: [Since]"}),
        ],
    );
    remove_elements(
        &fixture,
        "Reports/ParityReport/Templates/Print/Ext/Template.xml",
        &["parameter", "tl"],
    );
    branch(
        &fixture,
        "main:Report.ParityReport.Template.Print.Area.Header",
        "Parameter",
        &[],
    );
    branch_rows(
        &fixture,
        "main:Report.ParityReport.Template.Print.Area.Header",
        "Body",
        &[],
    );
    assert_eq!(
        read(
            &fixture,
            "main:Report.ParityReport.Template.Print.Area.Header.Body"
        )["at"],
        "main:Report.ParityReport.Template.Print.Area.Header.Body"
    );
    remove_elements(
        &fixture,
        "Reports/ParityReport/Templates/Print/Ext/Template.xml",
        &["namedItem"],
    );
    branch(&fixture, owner, "Area", &[]);
}

/// rule: INV.SOURCE.LOGICAL-BRANCH-COUNT
#[test]
fn xdto_object_type_keeps_empty_properties_but_value_type_does_not_acquire_them() {
    let fixture = RealReaderFixture::new();
    branch(&fixture, "main:XDTOPackage.EnterpriseData_1_17_3.Type.Документ_ЗаказКлиента", "Property", &["main:XDTOPackage.EnterpriseData_1_17_3.Type.Документ_ЗаказКлиента.Property.Идентификаторы"]);
    // This empty ObjectType is already present in the enterprise-data fixture.
    branch(
        &fixture,
        "main:XDTOPackage.EnterpriseData_1_17_3.Type.СоставнойЛюбойОбъект",
        "Property",
        &[],
    );
    absent_branch(
        &fixture,
        "main:XDTOPackage.EnterpriseData_1_17_3.Type.ВерсияФормата",
        "Property",
    );
}

/// rule: INV.SOURCE.LOGICAL-BRANCH-COUNT
#[test]
fn xdto_inline_object_property_keeps_its_empty_property_branch() {
    let fixture = RealReaderFixture::new();
    let owner = "main:XDTOPackage.EnterpriseData_1_17_3.Type.ЛюбаяСсылка.Property.СсылкаНаОбъект";
    branch(&fixture, owner, "Property", &["main:XDTOPackage.EnterpriseData_1_17_3.Type.ЛюбаяСсылка.Property.СсылкаНаОбъект.Property.Документ_ЗаказКлиента"]);
    let path = fixture
        .source
        .join("XDTOPackages/EnterpriseData_1_17_3/Ext/Package.bin");
    let mut xml = fs::read_to_string(&path).unwrap();
    let range = {
        let document = roxmltree::Document::parse(&xml).unwrap();
        document
            .descendants()
            .find(|node| {
                node.has_tag_name(("http://v8.1c.ru/8.1/xdto", "property"))
                    && node
                        .parent()
                        .is_some_and(|parent| parent.tag_name().name() == "typeDef")
            })
            .unwrap()
            .range()
    };
    xml.replace_range(range, "");
    write(&path, &xml);
    branch(&fixture, owner, "Property", &[]);
    absent_branch(
        &fixture,
        "main:XDTOPackage.EnterpriseData_1_17_3.Type.Документ_ЗаказКлиента.Property.Идентификаторы",
        "Property",
    );
}

/// rule: INV.SOURCE.LOGICAL-BRANCH-COUNT
#[test]
fn role_right_keeps_empty_rls_branch_after_the_last_condition_is_removed() {
    let fixture = RealReaderFixture::new();
    let owner = "main:Role.SalesReader.Right.Catalog_Products";
    branch(
        &fixture,
        owner,
        "RLS",
        &["main:Role.SalesReader.Right.Catalog_Products.RLS.View"],
    );
    remove_elements(
        &fixture,
        "Roles/SalesReader/Ext/Rights.xml",
        &["restrictionByCondition"],
    );
    branch(&fixture, owner, "RLS", &[]);
    absent_branch(&fixture, owner, "Attribute");
}

/// rule: INV.SOURCE.LOGICAL-BRANCH-COUNT
#[test]
fn nested_subsystem_keeps_empty_children_and_relations_without_inventing_an_interface() {
    let fixture = RealReaderFixture::new();
    write(
        &fixture
            .source
            .join("Subsystems/Sales/Ext/CommandInterface.xml"),
        r#"<CommandInterface xmlns="http://v8.1c.ru/8.3/xcf/extrnprops"/>"#,
    );
    // Derived from the existing Sales descriptor and its logical-tree fixture:
    // register a real nested owner, then leave its two collections empty.
    let parent = fixture.source.join("Subsystems/Sales.xml");
    let original = fs::read_to_string(&parent).unwrap();
    write(
        &parent,
        &original.replace(
            "<ChildObjects/>",
            "<ChildObjects><Subsystem>Orders</Subsystem></ChildObjects>",
        ),
    );
    write(
        &fixture
            .source
            .join("Subsystems/Sales/Subsystems/Orders.xml"),
        &super::subsystem_descriptor("Orders", &[], &[]),
    );
    branch(
        &fixture,
        "main:Subsystem.Sales",
        "Subsystem",
        &["main:Subsystem.Sales.Subsystem.Orders"],
    );
    let owner = "main:Subsystem.Sales.Subsystem.Orders";
    branch(&fixture, owner, "Subsystem", &[]);
    branch(&fixture, owner, "Relation", &[]);
    absent_branch(&fixture, owner, "Interface");
}

/// rule: INV.SOURCE.LOGICAL-BRANCH-COUNT
#[test]
fn interface_keeps_all_three_empty_ordered_collections() {
    let fixture = RealReaderFixture::new();
    write(
        &fixture
            .source
            .join("Subsystems/Sales/Ext/CommandInterface.xml"),
        r#"<CommandInterface xmlns="http://v8.1c.ru/8.3/xcf/extrnprops"/>"#,
    );
    // An admitted empty document is distinct from a missing artifact and
    // from the old metadata-removal fixture's NavigationPanel placeholder.
    for kind in ["Command", "Group", "Subsystem"] {
        branch(&fixture, "main:Subsystem.Sales.Interface", kind, &[]);
    }
    absent_branch(&fixture, "main:Subsystem.Sales.Interface", "Parameter");
}

/// rule: INV.SOURCE.LOGICAL-BRANCH-COUNT
#[test]
fn bsl_region_keeps_its_empty_nested_region_branch() {
    let fixture = RealReaderFixture::new();
    let module = fixture
        .source
        .join("CommonModules/РеактивныйСервер/Ext/Module.bsl");
    // The registered real-reader module is reduced to legal nested regions;
    // no provider facts or symbol payloads are fabricated.
    write(&module, "#Область Outer\n#Область First\n#КонецОбласти\n#Область Second\n#КонецОбласти\n#КонецОбласти\n");
    let owner = "main:CommonModule.РеактивныйСервер.Region.Outer";
    branch(
        &fixture,
        owner,
        "Region",
        &[
            "main:CommonModule.РеактивныйСервер.Region.Outer.Region.First",
            "main:CommonModule.РеактивныйСервер.Region.Outer.Region.Second",
        ],
    );
    branch(
        &fixture,
        "main:CommonModule.РеактивныйСервер.Region.Outer.Region.First",
        "Region",
        &[],
    );
    write(&module, "#Область Outer\n#КонецОбласти\n");
    branch(&fixture, owner, "Region", &[]);
}

/// rule: INV.SOURCE.LOGICAL-BRANCH-COUNT
#[test]
fn corrupt_service_and_unknown_owner_do_not_become_empty_collections() {
    let fixture = RealReaderFixture::new();
    let path = fixture.source.join("HTTPServices/ExternalAPI.xml");
    let xml = fs::read_to_string(&path).unwrap();
    write(&path, &xml.replace("<Name>Get</Name>", "<Name/>"));
    let result = fixture
        .view_service()
        .view(ViewRequest::new("main:HTTPService.ExternalAPI.URLTemplate.Metrics.Method").unwrap());
    assert!(
        !result.ok,
        "damaged identity became an empty collection: {result:?}"
    );
    let missing = fixture
        .view_service()
        .view(ViewRequest::new("main:HTTPService.ExternalAPI.URLTemplate.Missing.Method").unwrap());
    assert!(
        !missing.ok,
        "missing owner became an empty collection: {missing:?}"
    );
}

/// rule: INV.SOURCE.LOGICAL-BRANCH-COUNT
#[test]
fn inherited_form_content_does_not_claim_empty_own_collections() {
    let fixture = RealReaderFixture::new();
    // A real adopted descriptor without its inherited content is not an own
    // empty form. Existing extension-reader authority retains that distinction.
    fixture.borrow_catalog("Catalogs/Items.xml", None);
    let content = fixture
        .source
        .join("Catalogs/Items/Forms/ItemForm/Ext/Form.xml");
    fs::remove_file(content).unwrap();
    let result = fixture
        .extension_view_service()
        .view(ViewRequest::new("main:Catalog.Items.Form.ItemForm.Item").unwrap());
    assert!(
        !result.ok,
        "inherited/unknown content became an own empty branch: {result:?}"
    );
}

fn catalog_event_branch(fixture: &RealReaderFixture, owner: &str, known_event: &str) {
    let at = format!("{owner}.Event");
    let events = read(fixture, &at);
    let rows = events["items"].as_array().unwrap();
    let addresses = rows
        .iter()
        .map(|row| row["at"].as_str().unwrap())
        .collect::<Vec<_>>();
    assert!(
        addresses.contains(&format!("{at}.{known_event}").as_str()),
        "{events}"
    );
    assert!(
        addresses.windows(2).all(|pair| pair[0] < pair[1]),
        "catalog order or duplicate: {events}"
    );
    branch(fixture, owner, "Event", &addresses);
}

/// rule: INV.SOURCE.LOGICAL-BRANCH-COUNT
#[test]
fn real_form_nonempty_collections_keep_order_and_catalog_event_applicability() {
    let fixture = RealReaderFixture::new();
    fixture.install_accepted_profile_sources();
    let owner = "main:Document.Заказ.Form.ФормаДокумента";
    branch(
        &fixture,
        owner,
        "Item",
        &[
            "main:Document.Заказ.Form.ФормаДокумента.Item.Склад",
            "main:Document.Заказ.Form.ФормаДокумента.Item.Товары",
        ],
    );
    branch(
        &fixture,
        owner,
        "Attribute",
        &["main:Document.Заказ.Form.ФормаДокумента.Attribute.Объект"],
    );
    branch(
        &fixture,
        owner,
        "Parameter",
        &["main:Document.Заказ.Form.ФормаДокумента.Parameter.Режим"],
    );
    branch(
        &fixture,
        owner,
        "Command",
        &["main:Document.Заказ.Form.ФормаДокумента.Command.Пересчитать"],
    );
    branch(
        &fixture,
        "main:Document.Заказ.Form.ФормаДокумента.Command.Пересчитать",
        "Event",
        &["main:Document.Заказ.Form.ФормаДокумента.Command.Пересчитать.Event.Execute"],
    );
    catalog_event_branch(&fixture, owner, "OnOpen");
    catalog_event_branch(
        &fixture,
        "main:Document.Заказ.Form.ФормаДокумента.Item.Склад",
        "OnChange",
    );
    // The scalar attribute does not acquire table columns just because the
    // grammar can spell a Column suffix.
    absent_branch(
        &fixture,
        "main:Document.Заказ.Form.ФормаДокумента.Attribute.Объект",
        "Column",
    );
    assert_eq!(
        read(&fixture, &format!("{owner}.Metadata"))["kind"],
        "Metadata"
    );
    absent_branch(&fixture, owner, "Setting");
}

/// rule: INV.SOURCE.LOGICAL-BRANCH-COUNT
#[test]
fn real_form_tabular_attribute_keeps_empty_columns_after_removing_the_last_column() {
    let fixture = RealReaderFixture::new();
    fixture.install_accepted_profile_sources();
    let relative = "Documents/Заказ/Forms/ФормаДокумента/Ext/Form.xml";
    let path = fixture.source.join(relative);
    let xml = fs::read_to_string(&path).unwrap();
    // Derive a ValueTable attribute from the accepted-profile form using the
    // Columns/Column shape in references/specs/1c-form-spec.md §9.3.
    write(&path, &xml.replace("<Attribute name=\"Объект\" id=\"1\"><Type><v8:Type>xs:string</v8:Type></Type></Attribute>", "<Attribute name=\"Объект\" id=\"1\"><Type><v8:Type>v8:ValueTable</v8:Type></Type><Columns><Column name=\"First\" id=\"1\"><Type><v8:Type>xs:string</v8:Type></Type></Column><Column name=\"Second\" id=\"2\"><Type><v8:Type>xs:decimal</v8:Type></Type></Column></Columns></Attribute>"));
    let owner = "main:Document.Заказ.Form.ФормаДокумента.Attribute.Объект";
    branch(
        &fixture,
        owner,
        "Column",
        &[
            "main:Document.Заказ.Form.ФормаДокумента.Attribute.Объект.Column.First",
            "main:Document.Заказ.Form.ФормаДокумента.Attribute.Объект.Column.Second",
        ],
    );
    remove_elements(&fixture, relative, &["Column"]);
    branch(&fixture, owner, "Column", &[]);
}

/// rule: INV.SOURCE.LOGICAL-BRANCH-COUNT
#[test]
fn form_group_keeps_empty_children_without_inventing_catalog_events() {
    let fixture = RealReaderFixture::new();
    let relative = "Reports/ParityReport/Forms/MainForm/Ext/Form.xml";
    let path = fixture.source.join(relative);
    let xml = fs::read_to_string(&path).unwrap();
    // UsualGroup is the container used by the existing real form event matrix.
    write(
        &path,
        &xml.replace("<Table name=\"Goods\">", "<UsualGroup name=\"Goods\">")
            .replace("</Table>", "</UsualGroup>"),
    );
    let owner = "main:Report.ParityReport.Form.MainForm.Item.Goods";
    branch(
        &fixture,
        owner,
        "Item",
        &["main:Report.ParityReport.Form.MainForm.Item.Goods.Item.Quantity"],
    );
    absent_branch(&fixture, owner, "Event");
    remove_elements(&fixture, relative, &["InputField"]);
    branch(&fixture, owner, "Item", &[]);
}

/// rule: INV.SOURCE.LOGICAL-BRANCH-COUNT
#[test]
fn present_conditional_appearance_keeps_empty_items_but_absence_is_not_a_setting() {
    let fixture = RealReaderFixture::new();
    install_form_metadata_test_sources(&fixture);
    let relative = "CommonForms/ОбщаяФорма/Ext/Form.xml";
    let xml = include_str!("../../../../../../tests/fixtures/acceptance/form-conditional-appearance/src/CommonForms/Appearance/Ext/Form.xml");
    write(&fixture.source.join(relative), xml);
    let form = "main:CommonForm.ОбщаяФорма";
    let setting = "main:CommonForm.ОбщаяФорма.Setting.ConditionalAppearance";
    branch(&fixture, form, "Setting", &[setting]);
    branch(
        &fixture,
        setting,
        "Item",
        &["main:CommonForm.ОбщаяФорма.Setting.ConditionalAppearance.Item.Rule1"],
    );
    let start = xml.find("<ConditionalAppearance>").unwrap();
    let end = xml.find("</ConditionalAppearance>").unwrap() + "</ConditionalAppearance>".len();
    let mut empty = xml.to_string();
    empty.replace_range(start..end, "<ConditionalAppearance/>");
    write(&fixture.source.join(relative), &empty);
    branch(&fixture, setting, "Item", &[]);
    remove_elements(&fixture, relative, &["ConditionalAppearance"]);
    absent_branch(&fixture, form, "Setting");
}

/// rule: INV.SOURCE.LOGICAL-BRANCH-COUNT
#[test]
fn dcs_object_and_union_empty_collections_follow_the_dataset_variant() {
    for variant in ["DataSetObject", "DataSetUnion"] {
        let fixture = RealReaderFixture::new();
        let relative = "Reports/ParityReport/Templates/MainSchema/Ext/Template.xml";
        remove_elements(&fixture, relative, &["field", "query"]);
        let path = fixture.source.join(relative);
        let xml = fs::read_to_string(&path).unwrap();
        // The variants and objectName follow 1c-dcs-spec.md §4.2–4.3;
        // source, owner and template registration remain the existing fixture.
        let xml = xml.replace(
            "xsi:type=\"DataSetQuery\"",
            &format!("xsi:type=\"{variant}\""),
        );
        let xml = if variant == "DataSetObject" {
            xml.replace("</dataSet>", "<objectName>Rows</objectName></dataSet>")
        } else {
            xml
        };
        write(&path, &xml);
        let owner = "main:Report.ParityReport.Template.MainSchema.DataSet.MainData";
        branch(&fixture, owner, "Field", &[]);
        if variant == "DataSetUnion" {
            branch(&fixture, owner, "DataSet", &[]);
        } else {
            absent_branch(&fixture, owner, "DataSet");
        }
        absent_branch(&fixture, owner, "Query");
    }
}

/// rule: INV.SOURCE.LOGICAL-BRANCH-COUNT
#[test]
fn role_root_keeps_its_empty_right_collection() {
    let fixture = RealReaderFixture::new();
    branch(
        &fixture,
        "main:Role.SalesReader",
        "Right",
        &[
            "main:Role.SalesReader.Right.Catalog_Products",
            "main:Role.SalesReader.Right.Document_SalesOrder",
        ],
    );
    remove_elements(&fixture, "Roles/SalesReader/Ext/Rights.xml", &["object"]);
    branch(&fixture, "main:Role.SalesReader", "Right", &[]);
}

/// rule: INV.SOURCE.LOGICAL-BRANCH-COUNT
#[test]
fn xdto_package_keeps_empty_type_and_global_property_collections() {
    let fixture = RealReaderFixture::new();
    let package = "main:XDTOPackage.EnterpriseData_1_17_3";
    // The existing package provides actual object/value types and one global
    // property before reduction; its namespace and import remain intact.
    assert_eq!(
        read(&fixture, &format!("{package}.Type.ВерсияФормата"))["kind"],
        "Type"
    );
    assert_eq!(
        read(&fixture, &format!("{package}.Property.EnterpriseData"))["kind"],
        "Property"
    );
    remove_elements(
        &fixture,
        "XDTOPackages/EnterpriseData_1_17_3/Ext/Package.bin",
        &["objectType", "valueType", "property"],
    );
    branch(&fixture, package, "Type", &[]);
    branch(&fixture, package, "Property", &[]);
    assert_eq!(
        read(&fixture, &format!("{package}.Namespace"))["kind"],
        "Namespace"
    );
}

/// rule: INV.SOURCE.LOGICAL-BRANCH-COUNT
#[test]
fn dcs_query_count_is_the_number_of_reachable_text_lines() {
    for (query, expected) in [
        ("", vec![]),
        (
            "a\nb",
            vec![json!({"line":1,"text":"a"}), json!({"line":2,"text":"b"})],
        ),
        (
            "a\nb\n",
            vec![json!({"line":1,"text":"a"}), json!({"line":2,"text":"b"})],
        ),
        ("a\n", vec![json!({"line":1,"text":"a"})]),
    ] {
        let fixture = RealReaderFixture::new();
        let path = fixture
            .source
            .join("Reports/ParityReport/Templates/MainSchema/Ext/Template.xml");
        let xml = fs::read_to_string(&path).unwrap();
        let old = "<query>SELECT 1 AS Code</query>";
        assert_eq!(xml.matches(old).count(), 1, "query fixture anchor");
        write(&path, &xml.replace(old, &format!("<query>{query}</query>")));
        branch_rows(
            &fixture,
            "main:Report.ParityReport.Template.MainSchema.DataSet.MainData",
            "Query",
            &expected,
        );
    }
}

fn install_dcs_structure(fixture: &RealReaderFixture, structure: &str) {
    let path = fixture
        .source
        .join("Reports/ParityReport/Templates/MainSchema/Ext/Template.xml");
    let xml = fs::read_to_string(&path).unwrap();
    assert_eq!(xml.matches("</dcsset:settings>").count(), 1);
    // Same named settings-item shape as the F05Group in the existing
    // acceptance/workspace-dcs-parameter-expression fixture.
    write(
        &path,
        &xml.replace(
            "</dcsset:settings>",
            &format!("{structure}</dcsset:settings>"),
        ),
    );
}

/// rule: INV.SOURCE.LOGICAL-BRANCH-COUNT
#[test]
fn confirmed_dcs_structure_containers_keep_empty_children_and_named_order() {
    for kind in ["Group", "Table", "Chart"] {
        let fixture = RealReaderFixture::new();
        install_dcs_structure(&fixture, &format!("<dcsset:item xsi:type=\"dcsset:StructureItem{kind}\"><dcsset:name>Container</dcsset:name></dcsset:item>"));
        let setting = "main:Report.ParityReport.Template.MainSchema.Setting.Основной";
        let owner = "main:Report.ParityReport.Template.MainSchema.Setting.Основной.Item.Container";
        branch(&fixture, setting, "Item", &[owner]);
        branch(&fixture, owner, "Item", &[]);
    }
    let fixture = RealReaderFixture::new();
    install_dcs_structure(&fixture, "<dcsset:item xsi:type=\"dcsset:StructureItemGroup\"><dcsset:name>Container</dcsset:name><dcsset:item xsi:type=\"dcsset:StructureItemGroup\"><dcsset:name>Zeta</dcsset:name></dcsset:item><dcsset:item xsi:type=\"dcsset:StructureItemGroup\"><dcsset:name>Alpha</dcsset:name></dcsset:item></dcsset:item>");
    branch(&fixture, "main:Report.ParityReport.Template.MainSchema.Setting.Основной.Item.Container", "Item", &[
        "main:Report.ParityReport.Template.MainSchema.Setting.Основной.Item.Container.Item.Zeta",
        "main:Report.ParityReport.Template.MainSchema.Setting.Основной.Item.Container.Item.Alpha",
    ]);
}

/// rule: INV.SOURCE.LOGICAL-BRANCH-COUNT
#[test]
fn dcs_unconfirmed_structure_kinds_do_not_claim_empty_children_or_lose_observed_children() {
    for kind in ["NestedObject", "Future"] {
        let fixture = RealReaderFixture::new();
        install_dcs_structure(&fixture, &format!("<dcsset:item xsi:type=\"dcsset:StructureItem{kind}\"><dcsset:name>Container</dcsset:name></dcsset:item>"));
        absent_branch(
            &fixture,
            "main:Report.ParityReport.Template.MainSchema.Setting.Основной.Item.Container",
            "Item",
        );
    }
    let fixture = RealReaderFixture::new();
    install_dcs_structure(&fixture, "<dcsset:item xsi:type=\"dcsset:StructureItemNestedObject\"><dcsset:name>Container</dcsset:name><dcsset:item xsi:type=\"dcsset:StructureItemGroup\"><dcsset:name>Observed</dcsset:name></dcsset:item></dcsset:item>");
    branch(&fixture, "main:Report.ParityReport.Template.MainSchema.Setting.Основной.Item.Container", "Item", &["main:Report.ParityReport.Template.MainSchema.Setting.Основной.Item.Container.Item.Observed"]);
}

/// rule: INV.SOURCE.LOGICAL-BRANCH-COUNT
#[test]
fn interface_preserves_nonempty_group_and_subsystem_order() {
    let fixture = RealReaderFixture::new();
    write(
        &fixture.source.join("Subsystems/Sales.xml"),
        &super::subsystem_descriptor("Sales", &[], &["Orders"]),
    );
    write(
        &fixture
            .source
            .join("Subsystems/Sales/Subsystems/Orders.xml"),
        &super::subsystem_descriptor("Orders", &[], &[]),
    );
    // Derive the two ordered lists from the existing interface order fixture;
    // every subsystem reference points at an actually installed descriptor.
    write(
        &fixture
            .source
            .join("Subsystems/Sales/Ext/CommandInterface.xml"),
        r#"<CommandInterface xmlns="http://v8.1c.ru/8.3/xcf/extrnprops" version="2.20"><GroupsOrder><Group>NavigationPanelSeeAlso</Group><Group>NavigationPanelImportant</Group></GroupsOrder><SubsystemsOrder><Subsystem>Subsystem.Sales.Subsystem.Orders</Subsystem><Subsystem>Subsystem.Sales</Subsystem></SubsystemsOrder></CommandInterface>"#,
    );
    let owner = "main:Subsystem.Sales.Interface";
    branch(
        &fixture,
        owner,
        "Group",
        &[
            "main:Subsystem.Sales.Interface.Group.NavigationPanelSeeAlso",
            "main:Subsystem.Sales.Interface.Group.NavigationPanelImportant",
        ],
    );
    branch_rows(
        &fixture,
        owner,
        "Subsystem",
        &[
            json!({"order":1,"at":"main:Subsystem.Sales.Subsystem.Orders"}),
            json!({"order":2,"at":"main:Subsystem.Sales"}),
        ],
    );
    branch(&fixture, owner, "Command", &[]);
}

/// rule: INV.SOURCE.LOGICAL-BRANCH-COUNT
#[test]
fn unknown_form_container_preserves_observed_child_items() {
    let fixture = RealReaderFixture::new();
    // A forward-version widget is deliberately unknown, but its standard
    // ChildItems container remains observable in actual Form.xml. Derive the
    // owner/child shape from the existing real form container fixture.
    fixture.install_main_form_sources(
        r#"<Form xmlns="http://v8.1c.ru/8.3/xcf/logform" version="2.20"><ChildItems><FutureWidget name="Container" id="1"><ChildItems><Button name="Observed" id="2"/></ChildItems></FutureWidget></ChildItems></Form>"#,
        "",
    );
    let owner = "main:Report.ParityReport.Form.MainForm.Item.Container";
    let child = "main:Report.ParityReport.Form.MainForm.Item.Container.Item.Observed";
    assert_eq!(read(&fixture, child)["kind"], "Item");
    branch(&fixture, owner, "Item", &[child]);
}

/// rule: INV.SOURCE.LOGICAL-BRANCH-COUNT
#[test]
fn real_form_value_tree_attribute_exposes_confirmed_empty_columns() {
    let fixture = RealReaderFixture::new();
    fixture.install_accepted_profile_sources();
    let path = fixture
        .source
        .join("Documents/Заказ/Forms/ФормаДокумента/Ext/Form.xml");
    let xml = fs::read_to_string(&path).unwrap();
    let original =
        "<Attribute name=\"Объект\" id=\"1\"><Type><v8:Type>xs:string</v8:Type></Type></Attribute>";
    assert_eq!(xml.matches(original).count(), 1);
    // Both ValueTable and ValueTree own Columns in the form fact profile and
    // 1c-form-spec.md §9.3; a scalar attribute remains the negative control.
    write(&path, &xml.replace(original, "<Attribute name=\"Объект\" id=\"1\"><Type><v8:Type>v8:ValueTree</v8:Type></Type><Columns/></Attribute>"));
    branch(
        &fixture,
        "main:Document.Заказ.Form.ФормаДокумента.Attribute.Объект",
        "Column",
        &[],
    );
}

/// rule: INV.SOURCE.LOGICAL-BRANCH-COUNT
#[test]
fn command_interface_corruption_is_not_published_as_confirmed_empty_collections() {
    const NS: &str = "http://v8.1c.ru/8.3/xcf/extrnprops";
    let cases = [
        ("foreign root", format!("<Foreign xmlns=\"{NS}\"/>")),
        (
            "foreign namespace",
            "<CommandInterface xmlns=\"urn:unica:foreign\"/>".to_string(),
        ),
        (
            "malformed XML",
            format!("<CommandInterface xmlns=\"{NS}\"><GroupsOrder>"),
        ),
    ];
    for (case, xml) in cases {
        let fixture = RealReaderFixture::new();
        // Identity failures are well-formed XML; a parse failure is a separate
        // refusal. Local damage inside an identified document is tested below.
        if case != "malformed XML" {
            roxmltree::Document::parse(&xml).unwrap();
        }
        write(
            &fixture
                .source
                .join("Subsystems/Sales/Ext/CommandInterface.xml"),
            &xml,
        );
        let before = tree_snapshot(&fixture.source);
        let result = fixture
            .view_service()
            .view(ViewRequest::new("main:Subsystem.Sales.Interface").unwrap());
        assert!(
            !result.ok,
            "{case}: corrupt input became known empty: {result:?}"
        );
        assert_eq!(
            result.diagnostics[0]["code"], "provider_unavailable",
            "{case}: {result:?}"
        );
        assert!(
            result.data.is_none(),
            "{case}: corrupt input published branches"
        );
        assert_eq!(tree_snapshot(&fixture.source), before, "{case}");
    }
    let fixture = RealReaderFixture::new();
    // Existing valid documents do not require version. Namespace prefixes,
    // formatting whitespace and comments do not change the five known lists.
    let valid = format!("<ci:CommandInterface xmlns:ci=\"{NS}\">\n<!-- empty but observed -->\n<ci:CommandsVisibility/>\n<ci:CommandsPlacement/>\n<ci:CommandsOrder/>\n<ci:SubsystemsOrder/>\n<ci:GroupsOrder/>\n</ci:CommandInterface>");
    write(
        &fixture
            .source
            .join("Subsystems/Sales/Ext/CommandInterface.xml"),
        &valid,
    );
    for kind in ["Command", "Group", "Subsystem"] {
        branch(&fixture, "main:Subsystem.Sales.Interface", kind, &[]);
    }
}

/// rule: INV.SOURCE.INCOMPLETE-METADATA-READ
/// rule: INV.SOURCE.LOGICAL-BRANCH-COUNT
#[test]
fn command_interface_unknown_sibling_preserves_known_group_with_a_public_limit() {
    let fixture = RealReaderFixture::new();
    write(
        &fixture
            .source
            .join("Subsystems/Sales/Ext/CommandInterface.xml"),
        r#"<CommandInterface xmlns="http://v8.1c.ru/8.3/xcf/extrnprops"><GroupsOrder><Group>NavigationPanelImportant</Group></GroupsOrder><FutureSection/></CommandInterface>"#,
    );
    let before = tree_snapshot(&fixture.source);
    let owner = "main:Subsystem.Sales.Interface";
    let node = read(&fixture, owner);
    assert!(
        node["limits"]
            .as_array()
            .is_some_and(|limits| !limits.is_empty()),
        "unknown sibling must limit the observation: {node}"
    );
    // Interface has no public scalar properties. Private section-observation
    // carriers must not become invented public props while reporting limits.
    assert!(
        node.get("props")
            .is_none_or(|props| props.as_object().is_some_and(|props| props.is_empty())),
        "private reader state leaked into props: {node}"
    );
    branch(
        &fixture,
        owner,
        "Group",
        &["main:Subsystem.Sales.Interface.Group.NavigationPanelImportant"],
    );
    assert_eq!(
        read(
            &fixture,
            "main:Subsystem.Sales.Interface.Group.NavigationPanelImportant"
        )["items"],
        json!([])
    );
    unconfirmed_empty_branch(&fixture, owner, "Command");
    unconfirmed_empty_branch(&fixture, owner, "Subsystem");
    assert_eq!(tree_snapshot(&fixture.source), before);
}

fn partial_interface_fixture(body: &str) -> RealReaderFixture {
    let fixture = RealReaderFixture::new();
    write(&fixture.source.join("Subsystems/Sales/Ext/CommandInterface.xml"), &format!("<CommandInterface xmlns=\"http://v8.1c.ru/8.3/xcf/extrnprops\" xmlns:xr=\"http://v8.1c.ru/8.3/xcf/readable\">{body}</CommandInterface>"));
    fixture
}

fn limited_interface(fixture: &RealReaderFixture) -> Value {
    let node = read(fixture, "main:Subsystem.Sales.Interface");
    assert!(
        node["limits"]
            .as_array()
            .is_some_and(|limits| !limits.is_empty()),
        "local damage disappeared: {node}"
    );
    assert!(
        node.get("props")
            .is_none_or(|props| props.as_object().is_some_and(|props| props.is_empty())),
        "private reader state leaked: {node}"
    );
    node
}

fn unconfirmed_empty_branch(fixture: &RealReaderFixture, owner: &str, kind: &str) {
    let at = format!("{owner}.{kind}");
    let node = read(fixture, owner);
    assert!(
        node["branches"]
            .as_array()
            .into_iter()
            .flatten()
            .all(|row| row["at"] != at),
        "unconfirmed collection was advertised as known empty: {node}"
    );
    let result = fixture.view_service().view(ViewRequest::new(&at).unwrap());
    if result.ok {
        let collection = result.data.unwrap();
        assert_eq!(collection["items"], json!([]));
        assert!(
            collection["limits"]
                .as_array()
                .is_some_and(|limits| !limits.is_empty()),
            "partial empty collection lost its limit: {collection}"
        );
    } else {
        assert_eq!(
            result.diagnostics[0]["code"], "provider_unavailable",
            "{at}: {result:?}"
        );
        assert!(result.data.is_none());
    }
}

/// rule: INV.SOURCE.INCOMPLETE-METADATA-READ
/// rule: INV.SOURCE.LOGICAL-BRANCH-COUNT
#[test]
fn duplicate_interface_groups_do_not_choose_a_winner_or_erase_independent_lists() {
    let fixture = partial_interface_fixture("<CommandsOrder/><SubsystemsOrder><Subsystem>Subsystem.Sales</Subsystem></SubsystemsOrder><GroupsOrder><Group>NavigationPanelImportant</Group></GroupsOrder><GroupsOrder><Group>NavigationPanelSeeAlso</Group></GroupsOrder>");
    let before = tree_snapshot(&fixture.source);
    limited_interface(&fixture);
    let owner = "main:Subsystem.Sales.Interface";
    let groups = branch_items(&fixture, owner, "Group", 2);
    let mut identities = groups
        .iter()
        .map(|row| row["at"].as_str().unwrap())
        .collect::<Vec<_>>();
    identities.sort_unstable();
    assert_eq!(
        identities,
        vec![
            "main:Subsystem.Sales.Interface.Group.NavigationPanelImportant",
            "main:Subsystem.Sales.Interface.Group.NavigationPanelSeeAlso"
        ]
    );
    for row in groups {
        assert!(
            row.get("order").is_none_or(Value::is_null),
            "ambiguous order: {row}"
        );
        assert!(
            row["limits"]
                .as_array()
                .is_some_and(|limits| !limits.is_empty()),
            "{row}"
        );
        read(&fixture, row["at"].as_str().unwrap());
    }
    branch(&fixture, owner, "Subsystem", &["main:Subsystem.Sales"]);
    branch(&fixture, owner, "Command", &[]);
    assert_eq!(tree_snapshot(&fixture.source), before);
}

/// rule: INV.SOURCE.INCOMPLETE-METADATA-READ
/// rule: INV.SOURCE.LOGICAL-BRANCH-COUNT
#[test]
fn bad_interface_rows_keep_valid_groups_in_source_order() {
    for invalid in ["<FutureEntry/>", "<Group/>", "unexpected text"] {
        let fixture = partial_interface_fixture(&format!("<GroupsOrder><Group>NavigationPanelImportant</Group>{invalid}<Group>NavigationPanelSeeAlso</Group></GroupsOrder>"));
        limited_interface(&fixture);
        branch(
            &fixture,
            "main:Subsystem.Sales.Interface",
            "Group",
            &[
                "main:Subsystem.Sales.Interface.Group.NavigationPanelImportant",
                "main:Subsystem.Sales.Interface.Group.NavigationPanelSeeAlso",
            ],
        );
    }
    for invalid in ["<FutureEntry/>", "<Group/>", "unexpected text"] {
        let fixture = partial_interface_fixture(&format!("<GroupsOrder>{invalid}</GroupsOrder>"));
        limited_interface(&fixture);
        unconfirmed_empty_branch(&fixture, "main:Subsystem.Sales.Interface", "Group");
    }
    let fixture = partial_interface_fixture(
        "unexpected text<GroupsOrder><Group>NavigationPanelImportant</Group></GroupsOrder>",
    );
    limited_interface(&fixture);
    branch(
        &fixture,
        "main:Subsystem.Sales.Interface",
        "Group",
        &["main:Subsystem.Sales.Interface.Group.NavigationPanelImportant"],
    );
    unconfirmed_empty_branch(&fixture, "main:Subsystem.Sales.Interface", "Subsystem");
}

/// rule: INV.SOURCE.INCOMPLETE-METADATA-READ
/// rule: INV.SOURCE.LOGICAL-BRANCH-COUNT
#[test]
fn foreign_interface_group_sections_do_not_hide_native_sections_or_claim_empty_groups() {
    for native in [false, true] {
        let known = if native {
            "<GroupsOrder><Group>NavigationPanelImportant</Group></GroupsOrder>"
        } else {
            ""
        };
        let fixture = partial_interface_fixture(&format!("<SubsystemsOrder><Subsystem>Subsystem.Sales</Subsystem></SubsystemsOrder><foreign:GroupsOrder xmlns:foreign=\"urn:unica:foreign\"><foreign:Group>NavigationPanelSeeAlso</foreign:Group></foreign:GroupsOrder>{known}"));
        limited_interface(&fixture);
        branch(
            &fixture,
            "main:Subsystem.Sales.Interface",
            "Subsystem",
            &["main:Subsystem.Sales"],
        );
        if native {
            branch(
                &fixture,
                "main:Subsystem.Sales.Interface",
                "Group",
                &["main:Subsystem.Sales.Interface.Group.NavigationPanelImportant"],
            );
        } else {
            unconfirmed_empty_branch(&fixture, "main:Subsystem.Sales.Interface", "Group");
        }
    }
}

fn interface_command(fixture: &RealReaderFixture) -> Value {
    let collection = read(fixture, "main:Subsystem.Sales.Interface.Command");
    let items = collection["items"].as_array().unwrap();
    assert_eq!(items.len(), 1, "one known identity survives: {collection}");
    assert_eq!(
        items[0]["at"], "main:Catalog.Items.Command.Refresh",
        "{collection}"
    );
    items[0].clone()
}

/// rule: INV.SOURCE.INCOMPLETE-METADATA-READ
/// rule: INV.SOURCE.LOGICAL-BRANCH-COUNT
#[test]
fn damaged_interface_visibility_keeps_command_identity_placement_and_independent_sections() {
    for common in [
        "<xr:Common>not-a-boolean</xr:Common>",
        "<xr:Common>true</xr:Common><xr:Common>false</xr:Common>",
    ] {
        let fixture = partial_interface_fixture(&format!("<CommandsVisibility><Command name=\"Catalog.Items.Command.Refresh\"><Visibility>{common}</Visibility></Command></CommandsVisibility><CommandsPlacement><Command name=\"Catalog.Items.Command.Refresh\"><CommandGroup>NavigationPanelImportant</CommandGroup><Placement>Auto</Placement></Command></CommandsPlacement><SubsystemsOrder><Subsystem>Subsystem.Sales</Subsystem></SubsystemsOrder><GroupsOrder><Group>NavigationPanelImportant</Group></GroupsOrder>"));
        limited_interface(&fixture);
        let command = interface_command(&fixture);
        assert!(
            command.pointer("/props/visible").is_none_or(Value::is_null),
            "a damaged scalar became factual visibility: {command}"
        );
        assert!(
            command["limits"]
                .as_array()
                .is_some_and(|limits| !limits.is_empty()),
            "command row lost the scalar limit: {command}"
        );
        assert_eq!(command["props"]["group"], "NavigationPanelImportant");
        assert_eq!(command["props"]["placement"], "Auto");
        branch(
            &fixture,
            "main:Subsystem.Sales.Interface",
            "Group",
            &["main:Subsystem.Sales.Interface.Group.NavigationPanelImportant"],
        );
        branch(
            &fixture,
            "main:Subsystem.Sales.Interface",
            "Subsystem",
            &["main:Subsystem.Sales"],
        );
    }
}

/// rule: INV.SOURCE.INCOMPLETE-METADATA-READ
#[test]
fn conflicting_interface_command_rows_preserve_identity_without_selecting_visibility() {
    let fixture = partial_interface_fixture("<CommandsVisibility><Command name=\"Catalog.Items.Command.Refresh\"><Visibility><xr:Common>true</xr:Common></Visibility></Command><Command name=\"Catalog.Items.Command.Refresh\"><Visibility><xr:Common>false</xr:Common></Visibility></Command></CommandsVisibility><CommandsPlacement><Command name=\"Catalog.Items.Command.Refresh\"><CommandGroup>NavigationPanelImportant</CommandGroup></Command></CommandsPlacement>");
    limited_interface(&fixture);
    let command = interface_command(&fixture);
    assert!(
        command.pointer("/props/visible").is_none_or(Value::is_null),
        "duplicate rows selected a winner: {command}"
    );
    assert!(
        command["limits"]
            .as_array()
            .is_some_and(|limits| !limits.is_empty()),
        "command row lost duplicate observation: {command}"
    );
    assert_eq!(command["props"]["group"], "NavigationPanelImportant");
}

/// rule: INV.SOURCE.LOGICAL-BRANCH-COUNT
#[test]
fn interface_command_order_alone_preserves_the_command_identity() {
    let fixture = partial_interface_fixture("<CommandsOrder><Command name=\"Catalog.Items.Command.Refresh\"><CommandGroup>NavigationPanelImportant</CommandGroup></Command></CommandsOrder>");
    let command = interface_command(&fixture);
    assert!(
        command.pointer("/props/visible").is_none_or(Value::is_null),
        "order invented visibility: {command}"
    );
}

/// rule: INV.SOURCE.INCOMPLETE-METADATA-READ
#[test]
fn malformed_interface_command_identity_does_not_discard_a_valid_row() {
    let fixture = partial_interface_fixture("<CommandsVisibility><Command name=\"Catalog.Items.Command.Refresh\"><Visibility><xr:Common>true</xr:Common></Visibility></Command><Command><Visibility><xr:Common>false</xr:Common></Visibility></Command></CommandsVisibility>");
    limited_interface(&fixture);
    let command = interface_command(&fixture);
    assert_eq!(
        command["props"]["visible"], true,
        "a skipped unnamed row changed a known scalar: {command}"
    );
    assert!(
        command["limits"]
            .as_array()
            .is_some_and(|limits| !limits.is_empty()),
        "command row lost incomplete observation: {command}"
    );
}

/// rule: INV.SOURCE.INCOMPLETE-METADATA-READ
/// rule: INV.SOURCE.LOGICAL-BRANCH-COUNT
#[test]
fn incomplete_command_order_never_becomes_a_known_empty_group_composition() {
    for damaged in [
        "<CommandsOrder><Command/></CommandsOrder>",
        "<CommandsOrder/><CommandsOrder><Command name=\"Catalog.Items.Command.Refresh\"><CommandGroup>NavigationPanelImportant</CommandGroup></Command></CommandsOrder>",
    ] {
        let fixture = partial_interface_fixture(&format!("{damaged}<GroupsOrder><Group>NavigationPanelImportant</Group></GroupsOrder>"));
        limited_interface(&fixture);
        let groups = read(&fixture, "main:Subsystem.Sales.Interface.Group");
        let rows = groups["items"].as_array().unwrap();
        assert_eq!(rows.len(), 1, "{groups}");
        assert_eq!(rows[0]["at"], "main:Subsystem.Sales.Interface.Group.NavigationPanelImportant");
        let recovered = damaged.contains("Catalog.Items.Command.Refresh");
        if recovered { assert_eq!(rows[0]["commands"], 1, "{groups}"); }
        else { assert!(rows[0].get("commands").is_none_or(Value::is_null), "unknown composition was reported as a factual count: {groups}"); }
        assert!(rows[0]["limits"].as_array().is_some_and(|limits| !limits.is_empty()), "group row lost the observation limit: {groups}");
        let detail = read(&fixture, "main:Subsystem.Sales.Interface.Group.NavigationPanelImportant");
        if recovered {
            assert_eq!(detail["items"], json!([{"command":"Catalog.Items.Command.Refresh"}]));
            assert!(detail["items"][0].get("order").is_none_or(Value::is_null));
        } else { assert_eq!(detail["items"], json!([])); }
        assert!(detail["limits"].as_array().is_some_and(|limits| !limits.is_empty()), "empty partial composition was reported as complete: {detail}");
    }
    // The same empty rows are factual when all command sections are known.
    let fixture = partial_interface_fixture(
        "<CommandsOrder/><GroupsOrder><Group>NavigationPanelImportant</Group></GroupsOrder>",
    );
    let groups = read(&fixture, "main:Subsystem.Sales.Interface.Group");
    assert_eq!(groups["items"][0]["commands"], 0);
    let detail = read(
        &fixture,
        "main:Subsystem.Sales.Interface.Group.NavigationPanelImportant",
    );
    assert_eq!(detail["items"], json!([]));
    assert!(
        detail
            .get("limits")
            .is_none_or(|limits| limits.as_array().is_some_and(Vec::is_empty)),
        "valid empty composition was limited: {detail}"
    );
}

/// rule: INV.SOURCE.LOGICAL-BRANCH-COUNT
#[test]
fn valid_interface_command_membership_preserves_both_groups_and_their_order() {
    let fixture = partial_interface_fixture("<CommandsOrder><Command name=\"Catalog.Items.Command.Refresh\"><CommandGroup>NavigationPanelImportant</CommandGroup></Command><Command name=\"Catalog.Items.Command.Refresh\"><CommandGroup>NavigationPanelSeeAlso</CommandGroup></Command></CommandsOrder><GroupsOrder><Group>NavigationPanelSeeAlso</Group><Group>NavigationPanelImportant</Group></GroupsOrder>");
    let before = tree_snapshot(&fixture.source);
    let owner = "main:Subsystem.Sales.Interface";
    let node = read(&fixture, owner);
    assert!(
        node.get("limits")
            .is_none_or(|limits| limits.as_array().is_some_and(Vec::is_empty)),
        "valid platform membership was limited: {node}"
    );
    // One command identity can legitimately occur in more than one group.
    // GroupsOrder remains independent of the order of CommandsOrder rows.
    let command = interface_command(&fixture);
    assert!(
        command
            .get("limits")
            .is_none_or(|limits| limits.as_array().is_some_and(Vec::is_empty)),
        "valid repeated identity was limited: {command}"
    );
    branch(
        &fixture,
        owner,
        "Group",
        &[
            "main:Subsystem.Sales.Interface.Group.NavigationPanelSeeAlso",
            "main:Subsystem.Sales.Interface.Group.NavigationPanelImportant",
        ],
    );
    let groups = read(&fixture, "main:Subsystem.Sales.Interface.Group");
    for (index, name) in ["NavigationPanelSeeAlso", "NavigationPanelImportant"]
        .iter()
        .enumerate()
    {
        assert_eq!(groups["items"][index]["order"], index + 1);
        assert_eq!(groups["items"][index]["commands"], 1);
        let group = read(&fixture, &format!("{owner}.Group.{name}"));
        assert_eq!(
            group["items"],
            json!([{"order":1,"command":"Catalog.Items.Command.Refresh"}])
        );
        assert!(
            group
                .get("limits")
                .is_none_or(|limits| limits.as_array().is_some_and(Vec::is_empty)),
            "known membership was normalized away: {group}"
        );
    }
    assert_eq!(tree_snapshot(&fixture.source), before);
}

fn collection_without_false_zero(fixture: &RealReaderFixture, owner: &str, kind: &str) {
    let at = format!("{owner}.{kind}");
    let node = read(fixture, owner);
    for branch in node["branches"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|branch| branch["at"] == at)
    {
        assert_ne!(
            branch["count"], 0,
            "unread data became confirmed empty: {node}"
        );
    }
    let result = fixture.view_service().view(ViewRequest::new(&at).unwrap());
    if result.ok {
        let data = result.data.unwrap();
        let items = data["items"].as_array().unwrap();
        if items.is_empty() {
            assert!(
                data["limits"]
                    .as_array()
                    .is_some_and(|limits| !limits.is_empty()),
                "unread collection became complete empty: {data}"
            );
        }
    } else {
        assert_eq!(
            result.diagnostics[0]["code"], "provider_unavailable",
            "{at}: {result:?}"
        );
        assert!(result.data.is_none());
    }
}

fn partial_role_fixture(objects: &str) -> RealReaderFixture {
    let fixture = RealReaderFixture::new();
    let path = fixture.source.join("Roles/SalesReader/Ext/Rights.xml");
    let original = fs::read_to_string(&path).unwrap();
    let first_object = original.find("<object>").unwrap();
    // Keep the existing admitted Rights namespace, version and root flags.
    write(
        &path,
        &format!("{}{objects}</Rights>", &original[..first_object]),
    );
    fixture
}

/// rule: INV.SOURCE.INCOMPLETE-METADATA-READ
/// rule: INV.SOURCE.LOGICAL-BRANCH-COUNT
#[test]
fn role_rights_reader_distinguishes_wrong_root_and_incomplete_rows_from_empty() {
    let fixture = RealReaderFixture::new();
    write(
        &fixture.source.join("Roles/SalesReader/Ext/Rights.xml"),
        "<Foreign xmlns=\"http://v8.1c.ru/8.2/roles\"/>",
    );
    let before = tree_snapshot(&fixture.source);
    let result = fixture
        .view_service()
        .view(ViewRequest::new("main:Role.SalesReader").unwrap());
    assert!(
        !result.ok,
        "foreign rights root became a known empty role: {result:?}"
    );
    assert_eq!(result.diagnostics[0]["code"], "provider_unavailable");
    assert!(result.data.is_none());
    assert_eq!(tree_snapshot(&fixture.source), before);

    let bad_rows = [
        "<object><right><name>Read</name><value>true</value></right></object>",
        "<object><name>Catalog.Damaged</name><right><name>Read</name><value>not-a-boolean</value></right></object>",
        "<object><name>Catalog.Damaged</name><right><value>true</value></right></object>",
    ];
    for bad in bad_rows {
        let fixture = partial_role_fixture(bad);
        let before = tree_snapshot(&fixture.source);
        collection_without_false_zero(&fixture, "main:Role.SalesReader", "Right");
        let role = read(&fixture, "main:Role.SalesReader");
        assert!(
            role["limits"]
                .as_array()
                .is_some_and(|limits| !limits.is_empty()),
            "damaged rights rows lost their limit: {role}"
        );
        assert_eq!(tree_snapshot(&fixture.source), before);
    }

    // An unread right identity can hide RLS even when another known right
    // keeps its object addressable. It cannot prove RLS.count=0.
    let fixture = partial_role_fixture("<object><name>Catalog.Products</name><right><name>Read</name><value>true</value></right><right><value>true</value><restrictionByCondition><condition>Products.Owner = &amp;CurrentUser</condition></restrictionByCondition></right></object>");
    let before = tree_snapshot(&fixture.source);
    collection_without_false_zero(
        &fixture,
        "main:Role.SalesReader.Right.Catalog_Products",
        "RLS",
    );
    assert_eq!(tree_snapshot(&fixture.source), before);

    let fixture = RealReaderFixture::new();
    let path = fixture.source.join("Roles/SalesReader/Ext/Rights.xml");
    let original = fs::read_to_string(&path).unwrap();
    write(
        &path,
        &original.replace(
            "</Rights>",
            &format!("{}{}</Rights>", bad_rows[0], bad_rows[1]),
        ),
    );
    let before = tree_snapshot(&fixture.source);
    let role = read(&fixture, "main:Role.SalesReader");
    assert!(
        role["limits"]
            .as_array()
            .is_some_and(|limits| !limits.is_empty()),
        "{role}"
    );
    let rights = read(&fixture, "main:Role.SalesReader.Right");
    let healthy = rights["items"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|row| row["at"].as_str())
        .filter(|at| {
            matches!(
                *at,
                "main:Role.SalesReader.Right.Catalog_Products"
                    | "main:Role.SalesReader.Right.Document_SalesOrder"
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(
        healthy,
        [
            "main:Role.SalesReader.Right.Catalog_Products",
            "main:Role.SalesReader.Right.Document_SalesOrder"
        ],
        "healthy rights were discarded: {rights}"
    );
    branch(
        &fixture,
        "main:Role.SalesReader.Right.Catalog_Products",
        "RLS",
        &["main:Role.SalesReader.Right.Catalog_Products.RLS.View"],
    );
    assert_eq!(tree_snapshot(&fixture.source), before);
}

fn dcs_fields_fixture(fields: &str) -> RealReaderFixture {
    let fixture = RealReaderFixture::new();
    let path = fixture
        .source
        .join("Reports/ParityReport/Templates/MainSchema/Ext/Template.xml");
    let mut xml = fs::read_to_string(&path).unwrap();
    let range = {
        let document = roxmltree::Document::parse(&xml).unwrap();
        document
            .descendants()
            .find(|node| {
                node.is_element()
                    && node.tag_name().name() == "field"
                    && node
                        .parent()
                        .is_some_and(|parent| parent.tag_name().name() == "dataSet")
            })
            .unwrap()
            .range()
    };
    xml.replace_range(range, fields);
    write(&path, &xml);
    fixture
}

/// rule: INV.SOURCE.INCOMPLETE-METADATA-READ
/// rule: INV.SOURCE.LOGICAL-BRANCH-COUNT
#[test]
fn dcs_query_fields_keep_independent_query_and_valid_fields_when_a_field_is_damaged() {
    let owner = "main:Report.ParityReport.Template.MainSchema.DataSet.MainData";
    for bad in [
        "<field/>",
        "<field xsi:type=\"DataSetFieldField\"><field>MissingPath</field></field>",
    ] {
        let fixture = dcs_fields_fixture(bad);
        let before = tree_snapshot(&fixture.source);
        assert_eq!(
            read(&fixture, &format!("{owner}.Query"))["items"],
            json!([{"line":1,"text":"SELECT 1 AS Code"}])
        );
        collection_without_false_zero(&fixture, owner, "Field");
        let dataset = read(&fixture, owner);
        assert!(
            dataset["limits"]
                .as_array()
                .is_some_and(|limits| !limits.is_empty()),
            "damaged field became absent: {dataset}"
        );
        assert_eq!(tree_snapshot(&fixture.source), before);
    }
    // These are the original fixture's direct field names and types, with an
    // unread field between two valid identities. Their order is observable.
    let fixture = dcs_fields_fixture("<field xsi:type=\"DataSetFieldField\"><dataPath>Code</dataPath><field>Code</field><valueType><v8:Type>xs:string</v8:Type></valueType></field><field/><field xsi:type=\"DataSetFieldField\"><dataPath>Later</dataPath><field>Later</field><valueType><v8:Type>xs:string</v8:Type></valueType></field>");
    let before = tree_snapshot(&fixture.source);
    let dataset = read(&fixture, owner);
    assert!(
        dataset["limits"]
            .as_array()
            .is_some_and(|limits| !limits.is_empty()),
        "{dataset}"
    );
    branch(
        &fixture,
        owner,
        "Field",
        &[
            "main:Report.ParityReport.Template.MainSchema.DataSet.MainData.Field.Code",
            "main:Report.ParityReport.Template.MainSchema.DataSet.MainData.Field.Later",
        ],
    );
    assert_eq!(
        read(&fixture, &format!("{owner}.Query"))["items"],
        json!([{"line":1,"text":"SELECT 1 AS Code"}])
    );
    assert_eq!(tree_snapshot(&fixture.source), before);
}

fn edit_xml_element(
    fixture: &RealReaderFixture,
    relative: &str,
    tag: &str,
    parent: &str,
    edit: impl FnOnce(&str) -> String,
) {
    let path = fixture.source.join(relative);
    let mut xml = fs::read_to_string(&path).unwrap();
    let range = {
        let document = roxmltree::Document::parse(&xml).unwrap();
        document
            .descendants()
            .find(|node| {
                node.is_element()
                    && node.tag_name().name() == tag
                    && node
                        .parent()
                        .is_some_and(|owner| owner.tag_name().name() == parent)
            })
            .unwrap()
            .range()
    };
    let replacement = edit(&xml[range.clone()]);
    xml.replace_range(range, &replacement);
    write(&path, &xml);
}

/// rule: INV.SOURCE.INCOMPLETE-METADATA-READ
/// rule: INV.SOURCE.LOGICAL-BRANCH-COUNT
#[test]
fn duplicate_form_containers_do_not_discard_independent_known_collections() {
    for damaged in ["Attributes", "ChildItems"] {
        let fixture = RealReaderFixture::new();
        fixture.install_accepted_profile_sources();
        edit_xml_element(
            &fixture,
            "Documents/Заказ/Forms/ФормаДокумента/Ext/Form.xml",
            damaged,
            "Form",
            |original| format!("<{damaged}/>{original}"),
        );
        let before = tree_snapshot(&fixture.source);
        let owner = "main:Document.Заказ.Form.ФормаДокумента";
        if damaged == "Attributes" {
            branch(
                &fixture,
                owner,
                "Item",
                &[
                    "main:Document.Заказ.Form.ФормаДокумента.Item.Склад",
                    "main:Document.Заказ.Form.ФормаДокумента.Item.Товары",
                ],
            );
            collection_without_false_zero(&fixture, owner, "Attribute");
        } else {
            branch(
                &fixture,
                owner,
                "Attribute",
                &["main:Document.Заказ.Form.ФормаДокумента.Attribute.Объект"],
            );
            collection_without_false_zero(&fixture, owner, "Item");
        }
        let node = read(&fixture, owner);
        assert!(
            node["limits"]
                .as_array()
                .is_some_and(|limits| !limits.is_empty()),
            "duplicate container was silently selected: {node}"
        );
        assert_eq!(tree_snapshot(&fixture.source), before);
    }
}

/// rule: INV.SOURCE.INCOMPLETE-METADATA-READ
/// rule: INV.SOURCE.LOGICAL-BRANCH-COUNT
#[test]
fn mxl_missing_area_and_unread_row_index_do_not_prove_empty_collections() {
    let relative = "Reports/ParityReport/Templates/Print/Ext/Template.xml";
    let template = "main:Report.ParityReport.Template.Print";
    let fixture = RealReaderFixture::new();
    edit_xml_element(&fixture, relative, "area", "namedItem", |_| String::new());
    let before = tree_snapshot(&fixture.source);
    collection_without_false_zero(&fixture, template, "Area");
    assert_eq!(
        read(&fixture, template)["props"]["format"],
        "SpreadsheetDocument"
    );
    assert_eq!(tree_snapshot(&fixture.source), before);

    let fixture = RealReaderFixture::new();
    let path = fixture.source.join(relative);
    let mut xml = fs::read_to_string(&path).unwrap();
    let root_end = xml.rfind("</").unwrap();
    xml.insert_str(root_end, "<namedItem xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\" xsi:type=\"NamedItemCells\"><name>Damaged</name></namedItem>");
    write(&path, &xml);
    let before = tree_snapshot(&fixture.source);
    assert_eq!(
        read(&fixture, &format!("{template}.Area.Header.Parameter.Title"))["title"],
        "Title"
    );
    assert_eq!(
        read(&fixture, &format!("{template}.Area.Header.Body"))["items"],
        json!([{"index":1,"template":false,"text":"Sales report"},{"index":2,"template":true,"text":"Period: [Since]"}])
    );
    assert!(read(&fixture, template)["limits"]
        .as_array()
        .is_some_and(|limits| !limits.is_empty()));
    assert_eq!(tree_snapshot(&fixture.source), before);

    let fixture = RealReaderFixture::new();
    edit_xml_element(&fixture, relative, "index", "rowsItem", |_| {
        "<index>not-an-index</index>".to_string()
    });
    let before = tree_snapshot(&fixture.source);
    collection_without_false_zero(&fixture, &format!("{template}.Area.Header"), "Body");
    collection_without_false_zero(&fixture, &format!("{template}.Area.Header"), "Parameter");
    assert!(read(&fixture, template)["limits"]
        .as_array()
        .is_some_and(|limits| !limits.is_empty()));
    assert_eq!(tree_snapshot(&fixture.source), before);
}

/// rule: INV.SOURCE.INCOMPLETE-METADATA-READ
/// rule: INV.SOURCE.LOGICAL-BRANCH-COUNT
#[test]
fn foreign_xdto_properties_do_not_become_known_empty_native_property_collections() {
    let package = "main:XDTOPackage.EnterpriseData_1_17_3";
    for global in [false, true] {
        let fixture = RealReaderFixture::new();
        let path = fixture
            .source
            .join("XDTOPackages/EnterpriseData_1_17_3/Ext/Package.bin");
        let original = fs::read_to_string(&path).unwrap();
        let root_open_end = original.find('>').unwrap() + 1;
        let foreign = "<foreign:property xmlns:foreign=\"urn:unica:foreign\" name=\"Unread\" type=\"xs:string\"/>";
        let globals = if global {
            foreign
        } else {
            "<property name=\"Root\" type=\"xs:string\"/>"
        };
        let empty_properties = if global { "" } else { foreign };
        // Preserve the actual package identity/namespace. The native primitive
        // property shape is reduced from its existing object-type fixture.
        write(&path, &format!("{}{globals}<objectType name=\"Known\"><property name=\"Code\" type=\"xs:string\"/></objectType><objectType name=\"Empty\">{empty_properties}</objectType></package>", &original[..root_open_end]));
        let before = tree_snapshot(&fixture.source);
        branch(
            &fixture,
            &format!("{package}.Type.Known"),
            "Property",
            &["main:XDTOPackage.EnterpriseData_1_17_3.Type.Known.Property.Code"],
        );
        let owner = if global {
            package.to_string()
        } else {
            format!("{package}.Type.Empty")
        };
        collection_without_false_zero(&fixture, &owner, "Property");
        assert!(
            read(&fixture, &owner)["limits"]
                .as_array()
                .is_some_and(|limits| !limits.is_empty()),
            "foreign property was silently erased"
        );
        assert_eq!(tree_snapshot(&fixture.source), before);
    }
}

/// rule: INV.SOURCE.INCOMPLETE-METADATA-READ
/// rule: INV.SOURCE.LOGICAL-BRANCH-COUNT
#[test]
fn damaged_subsystem_content_and_child_identity_preserve_independent_facts() {
    for damaged in ["Content", "ChildObjects"] {
        let fixture = RealReaderFixture::new();
        write(
            &fixture
                .source
                .join("Subsystems/Sales/Ext/CommandInterface.xml"),
            "<CommandInterface xmlns=\"http://v8.1c.ru/8.3/xcf/extrnprops\"/>",
        );
        if damaged == "Content" {
            edit_xml_element(
                &fixture,
                "Subsystems/Sales.xml",
                "Content",
                "Properties",
                |_| "<Content><xr:Item xsi:type=\"xr:MDObjectRef\"/></Content>".to_string(),
            );
            edit_xml_element(
                &fixture,
                "Subsystems/Sales.xml",
                "ChildObjects",
                "Subsystem",
                |_| "<ChildObjects><Subsystem>Orders</Subsystem></ChildObjects>".to_string(),
            );
            write(
                &fixture
                    .source
                    .join("Subsystems/Sales/Subsystems/Orders.xml"),
                &super::subsystem_descriptor("Orders", &[], &[]),
            );
        } else {
            edit_xml_element(
                &fixture,
                "Subsystems/Sales.xml",
                "ChildObjects",
                "Subsystem",
                |_| "<ChildObjects><Subsystem/></ChildObjects>".to_string(),
            );
        }
        let before = tree_snapshot(&fixture.source);
        let owner = "main:Subsystem.Sales";
        if damaged == "Content" {
            branch(
                &fixture,
                owner,
                "Subsystem",
                &["main:Subsystem.Sales.Subsystem.Orders"],
            );
            collection_without_false_zero(&fixture, owner, "Relation");
        } else {
            branch(&fixture, owner, "Relation", &["main:Catalog.Items"]);
            collection_without_false_zero(&fixture, owner, "Subsystem");
        }
        assert!(read(&fixture, owner)["limits"]
            .as_array()
            .is_some_and(|limits| !limits.is_empty()));
        assert_eq!(tree_snapshot(&fixture.source), before);
    }
}

/// rule: INV.SOURCE.INCOMPLETE-METADATA-READ
/// rule: INV.SOURCE.LOGICAL-BRANCH-COUNT
#[test]
fn tabular_duplicate_child_objects_do_not_prove_no_attributes() {
    let fixture = RealReaderFixture::new();
    let path = fixture.source.join("Catalogs/Items.xml");
    let xml = fs::read_to_string(&path).unwrap();
    let anchor = "<Name>Lines</Name>";
    assert_eq!(xml.matches(anchor).count(), 1);
    write(
        &path,
        &xml.replace(anchor, "<Name>Lines</Name><StandardAttributes/>"),
    );
    edit_xml_element(
        &fixture,
        "Catalogs/Items.xml",
        "ChildObjects",
        "TabularSection",
        |original| format!("<ChildObjects/>{original}"),
    );
    let before = tree_snapshot(&fixture.source);
    branch(
        &fixture,
        "main:Catalog.Items",
        "Attribute",
        &["main:Catalog.Items.Attribute.Code"],
    );
    let owner = "main:Catalog.Items.TabularSection.Lines";
    collection_without_false_zero(&fixture, owner, "Attribute");
    branch(&fixture, owner, "StandardAttribute", &[]);
    let section = read(&fixture, owner);
    assert_eq!(section["title"], "Lines");
    assert!(section["limits"]
        .as_array()
        .is_some_and(|limits| !limits.is_empty()));
    assert_eq!(tree_snapshot(&fixture.source), before);
    for damage in ["true", "false", "0", "text", "childtext"] {
        let fixture = RealReaderFixture::new();
        let path = fixture.source.join("Catalogs/Items.xml");
        let xml = fs::read_to_string(&path).unwrap().replace(
            "<Name>Lines</Name>",
            "<Name>Lines</Name><StandardAttributes/>",
        );
        write(&path, &xml);
        edit_xml_element(
            &fixture,
            "Catalogs/Items.xml",
            "ChildObjects",
            "TabularSection",
            |_| {
                if damage == "childtext" {
                    "<ChildObjects>damaged</ChildObjects>".to_string()
                } else {
                    "<ChildObjects/>".to_string()
                }
            },
        );
        edit_xml_element(
            &fixture,
            "Catalogs/Items.xml",
            "TabularSection",
            "ChildObjects",
            |original| {
                if matches!(damage, "true" | "false" | "0") {
                    original.replacen("<TabularSection", &format!("<TabularSection xmlns:sourceNil=\"http://www.w3.org/2001/XMLSchema-instance\" sourceNil:nil=\"{damage}\""), 1)
                } else if damage == "text" {
                    let mut damaged = original.to_string();
                    damaged.insert_str(original.find('>').unwrap() + 1, "unread direct text");
                    damaged
                } else {
                    original.to_string()
                }
            },
        );
        let before = tree_snapshot(&fixture.source);
        branch(
            &fixture,
            "main:Catalog.Items",
            "Attribute",
            &["main:Catalog.Items.Attribute.Code"],
        );
        let owner = "main:Catalog.Items.TabularSection.Lines";
        assert_eq!(read(&fixture, owner)["title"], "Lines");
        for kind in ["Attribute", "StandardAttribute"] {
            if matches!(damage, "false" | "0")
                || (damage == "childtext" && kind == "StandardAttribute")
            {
                branch(&fixture, owner, kind, &[]);
            } else {
                collection_without_false_zero(&fixture, owner, kind);
            }
        }
        if matches!(damage, "true" | "text" | "childtext") {
            assert!(
                read(&fixture, owner)["limits"]
                    .as_array()
                    .is_some_and(|limits| !limits.is_empty()),
                "{damage}"
            );
        }
        assert_eq!(tree_snapshot(&fixture.source), before);
    }
}

fn xml_with_nil(xml: &str, nil: &str) -> String {
    let root_start = roxmltree::Document::parse(xml)
        .unwrap()
        .root_element()
        .range()
        .start;
    let offset = root_start + xml[root_start..].find('>').unwrap();
    let offset = if xml.as_bytes()[offset - 1] == b'/' {
        offset - 1
    } else {
        offset
    };
    let mut result = xml.to_string();
    result.insert_str(offset, &format!(" xmlns:sourceNil=\"http://www.w3.org/2001/XMLSchema-instance\" sourceNil:nil=\"{nil}\""));
    result
}

/// rule: INV.SOURCE.INCOMPLETE-METADATA-READ
/// rule: INV.SOURCE.LOGICAL-BRANCH-COUNT
#[test]
fn nil_native_source_roots_are_not_evidence_of_empty_owned_collections() {
    for (family, relative, owner, kinds) in [
        (
            "Role",
            "Roles/SalesReader/Ext/Rights.xml",
            "main:Role.SalesReader",
            &["Right"][..],
        ),
        (
            "DCS",
            "Reports/ParityReport/Templates/MainSchema/Ext/Template.xml",
            "main:Report.ParityReport.Template.MainSchema",
            &["DataSet", "Parameter"][..],
        ),
        (
            "Form",
            "CommonForms/ОбщаяФорма/Ext/Form.xml",
            "main:CommonForm.ОбщаяФорма",
            &["Item", "Attribute", "Parameter", "Command"][..],
        ),
        (
            "MXL",
            "Reports/ParityReport/Templates/Print/Ext/Template.xml",
            "main:Report.ParityReport.Template.Print",
            &["Area"][..],
        ),
        (
            "XDTO",
            "XDTOPackages/EnterpriseData_1_17_3/Ext/Package.bin",
            "main:XDTOPackage.EnterpriseData_1_17_3",
            &["Type", "Property"][..],
        ),
    ] {
        for nil in ["true", "false"] {
            let fixture = RealReaderFixture::new();
            match family {
                "Role" => remove_elements(&fixture, relative, &["object"]),
                "DCS" => remove_elements(
                    &fixture,
                    relative,
                    &["dataSource", "dataSet", "parameter", "settingsVariant"],
                ),
                "Form" => install_form_metadata_test_sources(&fixture),
                "MXL" => remove_elements(&fixture, relative, &["namedItem"]),
                "XDTO" => {
                    remove_elements(&fixture, relative, &["objectType", "valueType", "property"])
                }
                _ => unreachable!(),
            }
            let path = fixture.source.join(relative);
            write(
                &path,
                &xml_with_nil(&fs::read_to_string(&path).unwrap(), nil),
            );
            let before = tree_snapshot(&fixture.source);
            if nil == "true" {
                let result = fixture
                    .view_service()
                    .view(ViewRequest::new(owner).unwrap());
                assert!(!result.ok, "nil {family} root became readable: {result:?}");
                assert!(result.data.is_none(), "{family}: {result:?}");
                assert_eq!(
                    result.diagnostics[0]["code"], "provider_unavailable",
                    "{family}: {result:?}"
                );
            } else {
                for kind in kinds {
                    branch(&fixture, owner, kind, &[]);
                }
            }
            assert_eq!(
                tree_snapshot(&fixture.source),
                before,
                "{family}, nil={nil}"
            );
        }
    }
}

/// rule: INV.SOURCE.INCOMPLETE-METADATA-READ
/// rule: INV.SOURCE.LOGICAL-BRANCH-COUNT
#[test]
fn nil_or_foreign_mxl_cells_do_not_prove_empty_overlapping_area_content() {
    let relative = "Reports/ParityReport/Templates/Print/Ext/Template.xml";
    let template = "main:Report.ParityReport.Template.Print";
    for damage in [
        "cell",
        "foreign-wrapper",
        "row",
        "rowsItem",
        "namedItem",
        "area",
        "foreign-namedItem",
    ] {
        let fixture = RealReaderFixture::new();
        let path = fixture.source.join(relative);
        let mut xml = fs::read_to_string(&path).unwrap();
        let document = roxmltree::Document::parse(&xml).unwrap();
        let row = document
            .descendants()
            .find(|node| node.has_tag_name("rowsItem"))
            .unwrap();
        let independent = xml[row.range()]
            .replace("<index>0</index>", "<index>1</index>")
            .replace("Title", "OtherTitle");
        let end = xml.rfind("</document>").unwrap();
        xml.insert_str(end, &format!("{independent}<namedItem xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\" xsi:type=\"NamedItemCells\"><name>Independent</name><area><type>Rows</type><beginRow>1</beginRow><endRow>1</endRow><beginColumn>-1</beginColumn><endColumn>-1</endColumn></area></namedItem>"));
        write(&path, &xml);
        let independent_owner = format!("{template}.Area.Independent");
        let expected_body = read(&fixture, &format!("{independent_owner}.Body"))["items"].clone();
        let expected_parameters =
            read(&fixture, &format!("{independent_owner}.Parameter"))["items"].clone();
        assert_eq!(
            expected_parameters,
            json!([
                {"at":"main:Report.ParityReport.Template.Print.Area.Independent.Parameter.OtherTitle", "kind":"Parameter", "title":"OtherTitle"},
                {"kind":"Parameter", "title":"Since [tpl]", "addressable":false}
            ])
        );
        match damage {
            "cell" => edit_xml_element(&fixture, relative, "row", "rowsItem", |_| {
                "<row><c><c xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\" xsi:nil=\"true\"/></c></row>".to_string()
            }),
            "foreign-wrapper" => edit_xml_element(&fixture, relative, "row", "rowsItem", |_| {
                "<row><f:c xmlns:f=\"urn:unica:foreign\"><c/></f:c></row>".to_string()
            }),
            "row" => edit_xml_element(&fixture, relative, "row", "rowsItem", |original| {
                xml_with_nil(original, "true")
            }),
            "rowsItem" => {
                edit_xml_element(&fixture, relative, "rowsItem", "document", |original| {
                    xml_with_nil(original, "true")
                })
            }
            "namedItem" => {
                edit_xml_element(&fixture, relative, "namedItem", "document", |original| {
                    xml_with_nil(original, "true")
                })
            }
            "area" => edit_xml_element(&fixture, relative, "area", "namedItem", |original| {
                xml_with_nil(original, "true")
            }),
            "foreign-namedItem" => {
                edit_xml_element(&fixture, relative, "namedItem", "document", |original| {
                    original
                        .replacen(
                            "<namedItem",
                            "<foreign:namedItem xmlns:foreign=\"urn:unica:foreign\"",
                            1,
                        )
                        .replace("</namedItem>", "</foreign:namedItem>")
                })
            }
            _ => unreachable!(),
        }
        let before = tree_snapshot(&fixture.source);
        let header = format!("{template}.Area.Header");
        if damage == "foreign-namedItem" {
            collection_without_false_zero(&fixture, template, "Area");
            assert!(read(&fixture, template)["limits"]
                .as_array()
                .is_some_and(|limits| !limits.is_empty()));
        } else {
            let result = fixture
                .view_service()
                .view(ViewRequest::new(&header).unwrap());
            if result.ok {
                for kind in ["Parameter", "Body"] {
                    collection_without_false_zero(&fixture, &header, kind);
                }
                assert!(
                    result.data.unwrap()["limits"]
                        .as_array()
                        .is_some_and(|limits| !limits.is_empty()),
                    "{damage}"
                );
            } else {
                assert!(
                    matches!(damage, "namedItem" | "area"),
                    "{damage}: {result:?}"
                );
                assert!(result.data.is_none());
                assert_eq!(
                    result.diagnostics[0]["code"], "provider_unavailable",
                    "{damage}: {result:?}"
                );
            }
        }
        let parameters = branch_items(&fixture, &independent_owner, "Parameter", 2);
        assert_eq!(
            parameters,
            expected_parameters.as_array().unwrap().clone(),
            "{damage}"
        );
        read(&fixture, parameters[0]["at"].as_str().unwrap());
        assert_eq!(
            read(&fixture, &format!("{independent_owner}.Body"))["items"],
            expected_body,
            "{damage}"
        );
        branch_items(&fixture, &independent_owner, "Body", 2);
        assert_eq!(tree_snapshot(&fixture.source), before, "{damage}");
    }
}

/// rule: INV.SOURCE.INCOMPLETE-METADATA-READ
/// rule: INV.SOURCE.LOGICAL-BRANCH-COUNT
#[test]
fn nil_named_and_inline_xdto_object_types_do_not_prove_empty_properties() {
    let package = "main:XDTOPackage.EnterpriseData_1_17_3";
    for inline in [false, true] {
        for nil in ["true", "false", "0"] {
            let fixture = RealReaderFixture::new();
            let path = fixture
                .source
                .join("XDTOPackages/EnterpriseData_1_17_3/Ext/Package.bin");
            let original = fs::read_to_string(&path).unwrap();
            let root_end = original.find('>').unwrap() + 1;
            let damaged = if inline {
                format!("<objectType name=\"Container\"><property name=\"Inline\"><typeDef xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\" xsi:type=\"ObjectType\" xsi:nil=\"{nil}\"/></property><property name=\"Code\" type=\"xs:string\"/></objectType>")
            } else {
                format!("<objectType xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\" name=\"Empty\" xsi:nil=\"{nil}\"/>")
            };
            write(&path, &format!("{}<property name=\"Root\" type=\"xs:string\"/><objectType name=\"Known\"><property name=\"Code\" type=\"xs:string\"/></objectType>{damaged}</package>", &original[..root_end]));
            let before = tree_snapshot(&fixture.source);
            let owner = if inline {
                format!("{package}.Type.Container.Property.Inline")
            } else {
                format!("{package}.Type.Empty")
            };
            assert_eq!(
                read(&fixture, &owner)["title"],
                if inline { "Inline" } else { "Empty" }
            );
            branch(
                &fixture,
                &format!("{package}.Type.Known"),
                "Property",
                &["main:XDTOPackage.EnterpriseData_1_17_3.Type.Known.Property.Code"],
            );
            branch(
                &fixture,
                package,
                "Property",
                &["main:XDTOPackage.EnterpriseData_1_17_3.Property.Root"],
            );
            if inline {
                assert_eq!(
                    read(&fixture, &format!("{package}.Type.Container.Property.Code"))["title"],
                    "Code"
                );
            }
            if nil == "true" {
                collection_without_false_zero(&fixture, &owner, "Property");
                assert!(
                    read(&fixture, &owner)["limits"]
                        .as_array()
                        .is_some_and(|limits| !limits.is_empty()),
                    "{owner}"
                );
            } else {
                branch(&fixture, &owner, "Property", &[]);
            }
            assert_eq!(
                tree_snapshot(&fixture.source),
                before,
                "inline={inline}, nil={nil}"
            );
        }
    }
}

/// rule: INV.SOURCE.INCOMPLETE-METADATA-READ
#[test]
fn duplicate_interface_visibility_sections_recover_independent_command_identities_and_scalars() {
    let fixture = partial_interface_fixture("<CommandsVisibility><Command name=\"Catalog.Items.Command.Refresh\"><Visibility><xr:Common>true</xr:Common></Visibility></Command></CommandsVisibility><CommandsVisibility><Command name=\"CommonCommand.ОткрытьНастройки\"><Visibility><xr:Common>false</xr:Common></Visibility></Command></CommandsVisibility>");
    fixture.install_module_matrix_sources();
    let before = tree_snapshot(&fixture.source);
    limited_interface(&fixture);
    let rows = branch_items(&fixture, "main:Subsystem.Sales.Interface", "Command", 2);
    for (reference, visible) in [
        ("Catalog.Items.Command.Refresh", true),
        ("CommonCommand.ОткрытьНастройки", false),
    ] {
        let row = rows
            .iter()
            .find(|row| {
                row["props"]["name"] == reference
                    || row["title"] == reference
                    || row["command"] == reference
                    || row["at"].as_str().is_some_and(|at| at.ends_with(reference))
            })
            .unwrap_or_else(|| panic!("lost {reference}: {rows:?}"));
        let command = row;
        read(&fixture, row["at"].as_str().unwrap());
        assert_eq!(command["props"]["visible"], visible, "{command}");
        assert!(
            command["limits"]
                .as_array()
                .is_some_and(|limits| !limits.is_empty()),
            "{command}"
        );
    }
    assert_eq!(tree_snapshot(&fixture.source), before);
}

/// rule: INV.SOURCE.INCOMPLETE-METADATA-READ
/// rule: INV.SOURCE.LOGICAL-BRANCH-COUNT
#[test]
fn nil_dcs_query_dataset_preserves_identity_without_proving_empty_fields_or_query() {
    let relative = "Reports/ParityReport/Templates/MainSchema/Ext/Template.xml";
    let template = "main:Report.ParityReport.Template.MainSchema";
    let owner = format!("{template}.DataSet.MainData");
    for nil in ["true", "false", "0"] {
        let fixture = RealReaderFixture::new();
        remove_elements(&fixture, relative, &["field", "parameter"]);
        edit_xml_element(&fixture, relative, "query", "dataSet", |_| {
            "<query/>".to_string()
        });
        edit_xml_element(
            &fixture,
            relative,
            "dataSet",
            "DataCompositionSchema",
            |original| {
                original.replacen("<dataSet", &format!("<dataSet xmlns:sourceNil=\"http://www.w3.org/2001/XMLSchema-instance\" sourceNil:nil=\"{nil}\""), 1)
            },
        );
        let before = tree_snapshot(&fixture.source);
        assert_eq!(read(&fixture, &owner)["title"], "MainData");
        branch(&fixture, template, "Parameter", &[]);
        if nil == "true" {
            collection_without_false_zero(&fixture, &owner, "Field");
            collection_without_false_zero(&fixture, &owner, "Query");
            assert!(read(&fixture, &owner)["limits"]
                .as_array()
                .is_some_and(|limits| !limits.is_empty()));
        } else {
            branch(&fixture, &owner, "Field", &[]);
            branch_rows(&fixture, &owner, "Query", &[]);
        }
        assert_eq!(tree_snapshot(&fixture.source), before, "nil={nil}");
    }
    for (variant, kind) in [("DataSetObject", "Object"), ("DataSetUnion", "Union")] {
        let fixture = RealReaderFixture::new();
        remove_elements(&fixture, relative, &["field", "query"]);
        let path = fixture.source.join(relative);
        let xml = fs::read_to_string(&path).unwrap().replace(
            "xsi:type=\"DataSetQuery\"",
            &format!("xsi:type=\"{variant}\""),
        );
        let xml = if variant == "DataSetObject" {
            xml.replace("</dataSet>", "<objectName>Rows</objectName></dataSet>")
        } else {
            xml
        };
        write(&path, &xml);
        edit_xml_element(
            &fixture,
            relative,
            "dataSet",
            "DataCompositionSchema",
            |original| {
                original.replacen("<dataSet", "<dataSet xmlns:sourceNil=\"http://www.w3.org/2001/XMLSchema-instance\" sourceNil:nil=\"true\"", 1)
            },
        );
        let before = tree_snapshot(&fixture.source);
        let dataset = read(&fixture, &owner);
        assert_eq!(dataset["title"], "MainData");
        assert_eq!(dataset["props"]["kind"], kind, "{dataset}");
        let result = fixture
            .view_service()
            .view(ViewRequest::new(&format!("{owner}.Query")).unwrap());
        assert!(!result.ok, "{variant}: {result:?}");
        assert!(result.data.is_none());
        assert_eq!(
            result.diagnostics[0]["code"], "not_found",
            "inapplicable Query became unread-source refusal: {result:?}"
        );
        assert_eq!(tree_snapshot(&fixture.source), before);
    }
}

/// rule: INV.SOURCE.INCOMPLETE-METADATA-READ
/// rule: INV.SOURCE.LOGICAL-BRANCH-COUNT
#[test]
fn invalid_present_auto_command_bar_name_is_not_evidence_of_no_form_items() {
    let fixture = RealReaderFixture::new();
    fixture.install_accepted_profile_sources();
    let relative = "Documents/Заказ/Forms/ФормаДокумента/Ext/Form.xml";
    edit_xml_element(&fixture, relative, "ChildItems", "Form", |_| {
        "<AutoCommandBar name=\"bad name\" id=\"-1\"/><ChildItems/>".to_string()
    });
    let before = tree_snapshot(&fixture.source);
    let owner = "main:Document.Заказ.Form.ФормаДокумента";
    collection_without_false_zero(&fixture, owner, "Item");
    branch(
        &fixture,
        owner,
        "Attribute",
        &["main:Document.Заказ.Form.ФормаДокумента.Attribute.Объект"],
    );
    branch(
        &fixture,
        owner,
        "Command",
        &["main:Document.Заказ.Form.ФормаДокумента.Command.Пересчитать"],
    );
    assert!(read(&fixture, owner)["limits"]
        .as_array()
        .is_some_and(|limits| !limits.is_empty()));
    assert_eq!(tree_snapshot(&fixture.source), before);
}

/// rule: INV.SOURCE.INCOMPLETE-METADATA-READ
/// rule: INV.SOURCE.LOGICAL-BRANCH-COUNT
#[test]
fn direct_root_text_does_not_turn_unread_form_dcs_or_mxl_content_into_known_empty() {
    for family in ["Form", "DCS", "MXL"] {
        for empty in [false, true] {
            for content in ["damaged", "<![CDATA[damaged]]>"] {
                let fixture = RealReaderFixture::new();
                let (relative, owner, kinds, positive) = match family {
                    "Form" => {
                        fixture.install_accepted_profile_sources();
                        let relative = "Documents/Заказ/Forms/ФормаДокумента/Ext/Form.xml";
                        if empty {
                            remove_elements(
                                &fixture,
                                relative,
                                &["ChildItems", "Attributes", "Parameters", "Commands"],
                            );
                        }
                        (
                            relative,
                            "main:Document.Заказ.Form.ФормаДокумента",
                            &["Item", "Attribute", "Parameter", "Command"][..],
                            "Item",
                        )
                    }
                    "DCS" => {
                        let relative = "Reports/ParityReport/Templates/MainSchema/Ext/Template.xml";
                        if empty {
                            remove_elements(
                                &fixture,
                                relative,
                                &["dataSource", "dataSet", "parameter", "settingsVariant"],
                            );
                        }
                        (
                            relative,
                            "main:Report.ParityReport.Template.MainSchema",
                            &[
                                "DataSource",
                                "DataSet",
                                "CalculatedField",
                                "Total",
                                "Parameter",
                                "Setting",
                                "Item",
                            ][..],
                            "DataSet",
                        )
                    }
                    "MXL" => {
                        let relative = "Reports/ParityReport/Templates/Print/Ext/Template.xml";
                        if empty {
                            remove_elements(&fixture, relative, &["namedItem"]);
                        }
                        (
                            relative,
                            "main:Report.ParityReport.Template.Print",
                            &["Area"][..],
                            "Area",
                        )
                    }
                    _ => unreachable!(),
                };
                let collection_at = format!("{owner}.{positive}");
                let expected = if empty {
                    Vec::new()
                } else {
                    read(&fixture, &collection_at)["items"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(|row| row["at"].as_str().unwrap().to_string())
                        .collect::<Vec<_>>()
                };
                let path = fixture.source.join(relative);
                let mut xml = fs::read_to_string(&path).unwrap();
                let root_start = roxmltree::Document::parse(&xml)
                    .unwrap()
                    .root_element()
                    .range()
                    .start;
                let offset = root_start + xml[root_start..].find('>').unwrap() + 1;
                xml.insert_str(offset, content);
                write(&path, &xml);
                let before = tree_snapshot(&fixture.source);
                let node = read(&fixture, owner);
                assert!(
                    node["limits"]
                        .as_array()
                        .is_some_and(|limits| !limits.is_empty()),
                    "{family}: {node}"
                );
                for kind in kinds {
                    collection_without_false_zero(&fixture, owner, kind);
                }
                if !empty {
                    let collection = read(&fixture, &collection_at);
                    let actual = collection["items"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(|row| row["at"].as_str().unwrap().to_string())
                        .collect::<Vec<_>>();
                    assert_eq!(actual, expected, "{family}: {collection}");
                    for at in actual {
                        read(&fixture, &at);
                    }
                }
                assert_eq!(
                    tree_snapshot(&fixture.source),
                    before,
                    "{family}, empty={empty}, {content}"
                );
            }
        }
    }
}

/// rule: INV.SOURCE.INCOMPLETE-METADATA-READ
/// rule: INV.SOURCE.LOGICAL-BRANCH-COUNT
#[test]
fn only_native_known_mxl_cells_type_proves_empty_area_content() {
    let relative = "Reports/ParityReport/Templates/Print/Ext/Template.xml";
    let owner = "main:Report.ParityReport.Template.Print.Area.Header";
    for cell_type in [
        "NamedItemCells",
        "NamedItemCellsFuture",
        "foreign:NamedItemCells",
    ] {
        for empty in [false, true] {
            let fixture = RealReaderFixture::new();
            if empty {
                remove_elements(&fixture, relative, &["parameter", "tl"]);
            }
            let expected_parameters =
                read(&fixture, &format!("{owner}.Parameter"))["items"].clone();
            let expected_body = read(&fixture, &format!("{owner}.Body"))["items"].clone();
            let path = fixture.source.join(relative);
            let xml = fs::read_to_string(&path).unwrap();
            assert_eq!(xml.matches("xsi:type=\"NamedItemCells\"").count(), 1);
            write(
                &path,
                &xml.replace(
                    "xsi:type=\"NamedItemCells\"",
                    &format!("xmlns:foreign=\"urn:unica:foreign\" xsi:type=\"{cell_type}\""),
                ),
            );
            let before = tree_snapshot(&fixture.source);
            let node = read(&fixture, owner);
            assert_eq!(node["title"], "Header");
            assert_eq!(node["props"]["beginRow"], 0);
            assert_eq!(node["props"]["endRow"], 0);
            if cell_type == "NamedItemCells" {
                if empty {
                    branch(&fixture, owner, "Parameter", &[]);
                    branch_rows(&fixture, owner, "Body", &[]);
                } else {
                    assert_eq!(
                        branch_items(&fixture, owner, "Parameter", 2),
                        expected_parameters.as_array().unwrap().clone()
                    );
                    assert_eq!(
                        branch_items(&fixture, owner, "Body", 2),
                        expected_body.as_array().unwrap().clone()
                    );
                }
            } else {
                for kind in ["Parameter", "Body"] {
                    collection_without_false_zero(&fixture, owner, kind);
                }
                assert!(
                    node["limits"]
                        .as_array()
                        .is_some_and(|limits| !limits.is_empty()),
                    "unknown type lost its read limit: {node}"
                );
                if !empty {
                    assert_eq!(
                        read(&fixture, &format!("{owner}.Parameter"))["items"],
                        expected_parameters
                    );
                    assert_eq!(
                        read(&fixture, &format!("{owner}.Body"))["items"],
                        expected_body
                    );
                    read(&fixture, &format!("{owner}.Parameter.Title"));
                }
            }
            assert_eq!(
                tree_snapshot(&fixture.source),
                before,
                "{cell_type}, empty={empty}"
            );
        }
    }
}

/// rule: INV.SOURCE.INCOMPLETE-METADATA-READ
/// rule: INV.SOURCE.LOGICAL-BRANCH-COUNT
#[test]
fn damaged_mxl_payload_limits_only_affected_facts_and_preserves_neighboring_area_content() {
    let relative = "Reports/ParityReport/Templates/Print/Ext/Template.xml";
    let template = "main:Report.ParityReport.Template.Print";
    for damage in ["parameter", "text", "locale-content"] {
        let fixture = RealReaderFixture::new();
        let path = fixture.source.join(relative);
        let mut xml = fs::read_to_string(&path).unwrap();
        let row_range = roxmltree::Document::parse(&xml)
            .unwrap()
            .descendants()
            .find(|node| node.has_tag_name("rowsItem"))
            .unwrap()
            .range();
        let neighbor = xml[row_range]
            .replace("<index>0</index>", "<index>1</index>")
            .replace("Title", "NeighborTitle");
        xml.insert_str(xml.rfind("</document>").unwrap(), &format!("{neighbor}<namedItem xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\" xsi:type=\"NamedItemCells\"><name>Neighbor</name><area><type>Rows</type><beginRow>1</beginRow><endRow>1</endRow><beginColumn>-1</beginColumn><endColumn>-1</endColumn></area></namedItem>"));
        write(&path, &xml);
        let neighbor_owner = format!("{template}.Area.Neighbor");
        let neighbor_parameters =
            read(&fixture, &format!("{neighbor_owner}.Parameter"))["items"].clone();
        let neighbor_body = read(&fixture, &format!("{neighbor_owner}.Body"))["items"].clone();
        let payload = if damage == "parameter" {
            "<row><c><c><parameter/><parameter>Known</parameter></c></c></row>"
        } else if damage == "text" {
            "<row><c><c><parameter>Explicit</parameter><tl/><tl><content>Hidden [Token]</content></tl></c></c></row>"
        } else {
            "<row><c><c><parameter>Explicit</parameter><tl><v8:item><v8:lang>ru</v8:lang><v8:content/><v8:content>Hidden [Token]</v8:content></v8:item></tl></c></c></row>"
        };
        edit_xml_element(&fixture, relative, "row", "rowsItem", |_| {
            payload.to_string()
        });
        let before = tree_snapshot(&fixture.source);
        let header = format!("{template}.Area.Header");
        collection_without_false_zero(&fixture, &header, "Parameter");
        if damage == "parameter" {
            branch_rows(&fixture, &header, "Body", &[]);
        } else {
            collection_without_false_zero(&fixture, &header, "Body");
            let parameters = read(&fixture, &format!("{header}.Parameter"));
            assert!(
                parameters["items"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|row| row["at"] == format!("{header}.Parameter.Explicit")),
                "known explicit parameter lost: {parameters}"
            );
            read(&fixture, &format!("{header}.Parameter.Explicit"));
        }
        assert!(read(&fixture, &header)["limits"]
            .as_array()
            .is_some_and(|limits| !limits.is_empty()));
        assert_eq!(
            read(&fixture, &format!("{neighbor_owner}.Parameter"))["items"],
            neighbor_parameters
        );
        assert_eq!(
            read(&fixture, &format!("{neighbor_owner}.Body"))["items"],
            neighbor_body
        );
        branch_items(&fixture, &neighbor_owner, "Parameter", 2);
        branch_items(&fixture, &neighbor_owner, "Body", 2);
        assert_eq!(tree_snapshot(&fixture.source), before, "{damage}");
    }
}
