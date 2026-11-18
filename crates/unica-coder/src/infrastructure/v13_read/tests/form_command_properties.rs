use super::*;
const FORM: &str = "main:Report.ParityReport.Form.MainForm";

fn install(fixture: &RealReaderFixture, commands: &str) -> PathBuf {
    let path = fixture
        .source
        .join("Reports/ParityReport/Forms/MainForm/Ext/Form.xml");
    write(
        &path,
        &format!(
            r#"<Form xmlns="http://v8.1c.ru/8.3/xcf/logform" xmlns:xr="http://v8.1c.ru/8.3/xcf/readable" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" xmlns:xs="http://www.w3.org/2001/XMLSchema" version="2.20"><ChildItems><Table name="Товары" id="3"/><Button name="Fill" id="4"><CommandName>Form.Command.Fill</CommandName></Button></ChildItems><Commands>{commands}</Commands></Form>"#
        ),
    );
    path
}

#[test]
fn form_command_properties_reach_collection_and_detail_without_changing_buttons() {
    let fixture = RealReaderFixture::new();
    let commands = [false,true].iter().enumerate().map(|(index, transparent)| format!(r#"<Command name="Fill{index}" id="{index}"><CurrentRowUse>DontUse</CurrentRowUse><ModifiesSavedData>{transparent}</ModifiesSavedData><AssociatedTableElementId xsi:type="xs:string">Товары</AssociatedTableElementId><FunctionalOptions><Item>FunctionalOption.Prices</Item><Item>FunctionalOption.Taxes</Item></FunctionalOptions><Picture><xr:Ref>CommonPicture.Fill</xr:Ref><xr:LoadTransparent>{transparent}</xr:LoadTransparent></Picture><Action>ExecuteFill</Action><VendorFlag>true</VendorFlag></Command>"#)).collect::<String>();
    let path = install(&fixture, &commands);
    let before = fs::read(&path).unwrap();
    let service = fixture.view_service();
    let collection = service.view(ViewRequest::new(&format!("{FORM}.Command")).unwrap());
    assert!(collection.ok, "{collection:?}");
    let rows = collection.data.as_ref().unwrap()["items"]
        .as_array()
        .unwrap();
    for (index, flag) in [false, true].iter().enumerate() {
        let at = format!("{FORM}.Command.Fill{index}");
        let detail = service.view(ViewRequest::new(&at).unwrap());
        assert!(detail.ok, "{detail:?}");
        for node in [&rows[index], detail.data.as_ref().unwrap()] {
            let props = &node["props"];
            assert_eq!(props["currentRowUse"], "DontUse");
            assert_eq!(props["modifiesSavedData"], *flag);
            assert_eq!(props["associatedTableElementId"], "Товары");
            assert_eq!(props["picture"], "CommonPicture.Fill");
            assert_eq!(props["pictureLoadTransparent"], *flag);
            assert_eq!(
                props["functionalOptions"],
                json!(["FunctionalOption.Prices", "FunctionalOption.Taxes"])
            );
            assert!(props.get("VendorFlag").is_none());
        }
    }
    let button = service.view(ViewRequest::new(&format!("{FORM}.Item.Fill")).unwrap());
    assert!(button.ok);
    assert!(button.data.as_ref().unwrap()["props"]
        .get("modifiesSavedData")
        .is_none());
    assert_eq!(fs::read(path).unwrap(), before);
}

#[test]
fn form_command_string_identity_and_qname_variants_reach_collection_and_detail() {
    for property in [
        r#"<AssociatedTableElementId xsi:type=" xs:string "> Товары </AssociatedTableElementId>"#,
        r#"<lf:AssociatedTableElementId xmlns:lf="http://v8.1c.ru/8.3/xcf/logform" xmlns="http://www.w3.org/2001/XMLSchema" xsi:type="string"> Товары </lf:AssociatedTableElementId>"#,
    ] {
        let fixture = RealReaderFixture::new();
        let path = install(&fixture, &format!("<Command name=\"Fill\" id=\"1\">{property}<ModifiesSavedData> \tfalse\r\n</ModifiesSavedData><Action>Keep</Action></Command>"));
        let before = fs::read(&path).unwrap();
        let service = fixture.view_service();
        let collection = service.view(ViewRequest::new(&format!("{FORM}.Command")).unwrap());
        assert!(collection.ok, "{collection:?}");
        let row = &collection.data.as_ref().unwrap()["items"][0];
        let detail = service.view(ViewRequest::new(row["at"].as_str().unwrap()).unwrap());
        assert!(detail.ok, "{detail:?}");
        for node in [row, detail.data.as_ref().unwrap()] {
            assert_eq!(node["props"]["associatedTableElementId"], " Товары ");
            assert_eq!(node["props"]["modifiesSavedData"], false);
            assert!(node.get("limits").is_none(), "{node}");
        }
        assert_eq!(fs::read(path).unwrap(), before);
    }
}

#[test]
fn form_command_nbsp_boolean_damage_preserves_independent_public_facts() {
    let fixture = RealReaderFixture::new();
    let path = install(
        &fixture,
        r#"<Command name="Fill" id="1"><CurrentRowUse>DontUse</CurrentRowUse><ModifiesSavedData>&#160;false&#160;</ModifiesSavedData><AssociatedTableElementId xsi:type="xs:string">Товары</AssociatedTableElementId><Picture><xr:Ref>CommonPicture.Fill</xr:Ref><xr:LoadTransparent>&#160;false&#160;</xr:LoadTransparent></Picture><Action>Keep</Action></Command>"#,
    );
    let before = fs::read(&path).unwrap();
    let service = fixture.view_service();
    let collection = service.view(ViewRequest::new(&format!("{FORM}.Command")).unwrap());
    assert!(collection.ok, "{collection:?}");
    let row = &collection.data.as_ref().unwrap()["items"][0];
    let detail = service.view(ViewRequest::new(row["at"].as_str().unwrap()).unwrap());
    assert!(detail.ok, "{detail:?}");
    for node in [row, detail.data.as_ref().unwrap()] {
        assert!(node["props"]["modifiesSavedData"].is_null());
        assert!(node["props"]["pictureLoadTransparent"].is_null());
        assert_eq!(node["props"]["currentRowUse"], "DontUse");
        assert_eq!(node["props"]["associatedTableElementId"], "Товары");
        assert_eq!(node["props"]["picture"], "CommonPicture.Fill");
        assert_eq!(
            node["limits"],
            json!([
                "ModifiesSavedData is unreadable: malformed or duplicate command property",
                "LoadTransparent is unreadable: malformed or duplicate command property"
            ])
        );
    }
    assert_eq!(fs::read(path).unwrap(), before);
}

#[test]
fn form_command_option_limit_preserves_the_whole_property_projection() {
    let fixture = RealReaderFixture::new();
    let options = (0..180)
        .map(|index| format!("FunctionalOption.Option{index}"))
        .collect::<Vec<_>>();
    let xml = options
        .iter()
        .map(|option| format!("<Item>{option}</Item>"))
        .collect::<String>();
    let path=install(&fixture,&format!("<Command name=\"Fill\" id=\"1\"><CurrentRowUse>Use</CurrentRowUse><FunctionalOptions>{xml}</FunctionalOptions></Command>"));
    let before = fs::read(&path).unwrap();
    let service = fixture.view_service();
    let at = format!("{FORM}.Command.Fill");
    let detail = service.view(ViewRequest::new(&at).unwrap());
    assert!(detail.ok, "{detail:?}");
    let data = detail.data.as_ref().unwrap();
    assert!(data["props"].get("functionalOptions").is_none());
    assert_eq!(data["props"]["currentRowUse"], "Use");
    assert!(data["limits"]
        .to_string()
        .contains(&format!("{at}.Props.functionalOptions")));
    let whole =
        crate::infrastructure::v13_read_projection::with_whole_prop("functionalOptions", || {
            service.view(ViewRequest::new(&at).unwrap())
        });
    assert!(whole.ok, "{whole:?}");
    assert_eq!(
        whole.data.as_ref().unwrap()["props"]["functionalOptions"],
        json!(options)
    );
    assert_eq!(fs::read(path).unwrap(), before);
}
