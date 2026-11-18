use super::*;

#[test]
fn role_right_rls_consumes_the_complete_suffix() {
    let fixture = RealReaderFixture::new();
    let service = fixture.view_service();

    let result =
        service.view(ViewRequest::new("main:Role.SalesReader.Right.Products.RLS.View").unwrap());

    assert!(result.ok, "{} {:?}", result.summary, result.diagnostics);
    let data = result.data.as_ref().unwrap();
    assert_eq!(
        data["at"],
        "main:Role.SalesReader.Right.Catalog_Products.RLS.View"
    );
    assert_eq!(data["kind"], "RLS");
    assert_eq!(data["title"], "View");
}

#[test]
fn dcs_dataset_field_and_parameter_consume_the_complete_suffix() {
    let fixture = RealReaderFixture::new();
    let service = fixture.view_service();
    for (at, kind, title) in [
        (
            "main:Report.ParityReport.Template.MainSchema.DataSet.MainData.Field.Code",
            "Field",
            "Code",
        ),
        (
            "main:Report.ParityReport.Template.MainSchema.DataSet.MainData.Parameter.Period",
            "Parameter",
            "Period",
        ),
    ] {
        let result = service.view(ViewRequest::new(at).unwrap());
        assert!(
            result.ok,
            "{at}: {} {:?}",
            result.summary, result.diagnostics
        );
        let data = result.data.as_ref().unwrap();
        assert_eq!(data["at"], at);
        assert_eq!(data["kind"], kind);
        assert_eq!(data["title"], title);
    }
}

/// Имя с точкой — один сегмент адреса: читатель публикует его с `\.`, и по
/// опубликованному адресу узел читается. Составной `dataPath` поля схемы,
/// группа `CommandGroup.X` интерфейса, право на вложенный объект роли и
/// область с точкой в имени прежде получали адрес, который грамматика
/// не разбирала, или не получали его вовсе.
/// rule: CTR.SOURCE.QUALIFIED-LOGICAL-ADDRESS
#[test]
fn names_with_a_dot_are_published_escaped_and_read_back() {
    let fixture = RealReaderFixture::new();
    let schema = fixture
        .source
        .join("Reports/ParityReport/Templates/MainSchema/Ext/Template.xml");
    let text = fs::read_to_string(&schema).unwrap();
    write(
        &schema,
        &text.replacen(
            "<field xsi:type=\"DataSetFieldField\">",
            "<field xsi:type=\"DataSetFieldField\"><dataPath>Owner.Code</dataPath><field>Owner.Code</field></field>\n\t\t<field xsi:type=\"DataSetFieldField\">",
            1,
        ),
    );
    let rights = fixture.source.join("Roles/SalesReader/Ext/Rights.xml");
    let text = fs::read_to_string(&rights).unwrap();
    write(
        &rights,
        &text.replacen(
            "    <object>",
            "    <object>\n        <name>Catalog.Products.Command.Print</name>\n        <right>\n            <name>View</name>\n            <value>true</value>\n        </right>\n    </object>\n    <object>",
            1,
        ),
    );
    let print = fixture
        .source
        .join("Reports/ParityReport/Templates/Print/Ext/Template.xml");
    let text = fs::read_to_string(&print).unwrap();
    write(
        &print,
        &text.replace("<name>Header</name>", "<name>Header.Total</name>"),
    );
    write(
        &fixture
            .source
            .join("Subsystems/Sales/Ext/CommandInterface.xml"),
        r#"<?xml version="1.0" encoding="utf-8"?>
<CommandInterface xmlns="http://v8.1c.ru/8.3/xcf/extrnprops" xmlns:xr="http://v8.1c.ru/8.3/xcf/readable" version="2.20">
	<CommandsOrder>
		<Command name="Catalog.Items.StandardCommand.OpenList">
			<CommandGroup>CommandGroup.Pricing</CommandGroup>
		</Command>
	</CommandsOrder>
	<GroupsOrder>
		<Group>NavigationPanelImportant</Group>
		<Group>CommandGroup.Pricing</Group>
	</GroupsOrder>
</CommandInterface>
"#,
    );
    let service = fixture.view_service();
    // Член коллекции по опубликованному адресу: имя и вид совпадают.
    let member = |branch: &str, title: &str| -> String {
        let listed = service.view(ViewRequest::new(branch).unwrap());
        assert!(listed.ok, "{branch}: {:?}", refusal_codes(&listed));
        let items = listed.data.as_ref().unwrap()["items"]
            .as_array()
            .unwrap()
            .clone();
        let at = items
            .iter()
            .find(|item| item["title"] == title)
            .and_then(|item| item["at"].as_str())
            .unwrap_or_else(|| panic!("{branch} lists `{title}` with an address"))
            .to_string();
        let read = service.view(ViewRequest::new(&at).unwrap());
        assert!(read.ok, "{at}: {:?}", refusal_codes(&read));
        assert_eq!(read.data.as_ref().unwrap()["at"], at.as_str());
        at
    };
    assert_eq!(
        member(
            "main:Report.ParityReport.Template.MainSchema.DataSet.MainData.Field",
            "Owner.Code"
        ),
        r"main:Report.ParityReport.Template.MainSchema.DataSet.MainData.Field.Owner\.Code"
    );
    assert_eq!(
        member("main:Role.SalesReader.Right", "Products.Command.Print"),
        r"main:Role.SalesReader.Right.Catalog_Products\.Command\.Print"
    );
    assert_eq!(
        member(
            "main:Report.ParityReport.Template.Print.Area",
            "Header.Total"
        ),
        r"main:Report.ParityReport.Template.Print.Area.Header\.Total"
    );
    assert_eq!(
        member(
            "main:Subsystem.Sales.Interface.Group",
            "CommandGroup.Pricing"
        ),
        r"main:Subsystem.Sales.Interface.Group.CommandGroup\.Pricing"
    );
    let group = service.view(
        ViewRequest::new(r"main:Subsystem.Sales.Interface.Group.CommandGroup\.Pricing").unwrap(),
    );
    assert_eq!(
        group.data.as_ref().unwrap()["items"],
        json!([{"order": 1, "command": "Catalog.Items.StandardCommand.OpenList"}])
    );
}

/// Свойство XDTO любой вложенности читается по цепочке `Property`: узел
/// свойства со вложенным анонимным типом объявляет ветвь его свойств, и
/// каждый опубликованный адрес читается.
/// rule: INV.SOURCE.XDTO-PROPERTY-PATH
#[test]
fn xdto_properties_are_read_at_any_depth() {
    let fixture = RealReaderFixture::new();
    let service = fixture.view_service();
    let owner = "main:XDTOPackage.EnterpriseData_1_17_3.Type.ЛюбаяСсылка.Property.СсылкаНаОбъект";
    let node = service.view(ViewRequest::new(owner).unwrap());
    assert!(node.ok, "{:?}", refusal_codes(&node));
    assert_eq!(
        node.data.as_ref().unwrap()["branches"],
        json!([{"at": format!("{owner}.Property"), "count": 1}])
    );
    let nested = service.view(ViewRequest::new(&format!("{owner}.Property")).unwrap());
    assert!(nested.ok, "{:?}", refusal_codes(&nested));
    let leaf = format!("{owner}.Property.Документ_ЗаказКлиента");
    assert_eq!(
        nested.data.as_ref().unwrap()["items"][0]["at"],
        leaf.as_str()
    );
    let read = service.view(ViewRequest::new(&leaf).unwrap());
    assert!(read.ok, "{:?}", refusal_codes(&read));
    assert_eq!(read.data.as_ref().unwrap()["kind"], "Property");
    assert!(read
        .data
        .as_ref()
        .unwrap()
        .get("branches")
        .is_none_or(|value| value.as_array().is_some_and(Vec::is_empty)));
    // У свойства без вложенного типа детей нет.
    let missing = service.view(ViewRequest::new(&format!("{leaf}.Property.Нет")).unwrap());
    assert_eq!(refusal_codes(&missing), vec!["not_found"]);
}

#[test]
fn mxl_area_parameter_consumes_the_complete_suffix() {
    let fixture = RealReaderFixture::new();
    let service = fixture.view_service();
    let at = "main:Report.ParityReport.Template.Print.Area.Header.Parameter.Title";

    let result = service.view(ViewRequest::new(at).unwrap());

    assert!(result.ok, "{} {:?}", result.summary, result.diagnostics);
    let data = result.data.as_ref().unwrap();
    assert_eq!(data["at"], at);
    assert_eq!(data["kind"], "Parameter");
    assert_eq!(data["title"], "Title");
}

/// rule: INV.SOURCE.LOGICAL-QUESTION-COMPLETENESS
#[test]
fn every_reader_rejects_an_extra_unconsumed_address_tail() {
    let cases = [
        (
            "main:XDTOPackage.EnterpriseData_1_17_3.Type.Order.Property.Id.Event.Change",
            json!({
                "targetNamespace": "urn:test",
                "types": [{"name": "Order", "properties": [{"name": "Id"}]}],
                "globalProperties": [],
                "counts": {},
                "imports": [],
                "findings": []
            }),
        ),
        (
            "main:Subsystem.Sales.Interface.Command.Open.Event.Change",
            json!({
                "name": "Sales",
                "content": [],
                "children": [],
                "commandInterface": {
                    "visibility": [{"command": "Open", "visible": true}],
                    "placement": []
                }
            }),
        ),
    ];
    for (at, payload) in cases {
        let address = QualifiedAddress::parse(at).unwrap();
        let route = route_logical_address(&address, PlatformProfile::v8_3_27()).unwrap();
        let error = project_typed_payload(&route, payload).unwrap_err();
        assert_eq!(error.code().as_str(), "not_found", "{at}");
    }

    let fixture = RealReaderFixture::new();
    let module = fixture.view_service().view(
        ViewRequest::new(
            "main:Report.ParityReport.Form.MainForm.Module.Form.Method.OnOpen.Body.source.Event.Change",
        )
        .unwrap(),
    );
    assert!(!module.ok, "{:?}", module.data);
    assert_eq!(module.diagnostics[0]["code"], "not_found");
}
