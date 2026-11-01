use super::*;
const COMMAND: &str = "main:CommonCommand.Probe";
fn install(f: &RealReaderFixture, kind: &str, name: &str, properties: &str) -> PathBuf {
    let config = f.source.join("Configuration.xml");
    write(
        &config,
        &fs::read_to_string(&config).unwrap().replace(
            "</ChildObjects>",
            &format!("<{kind}>{name}</{kind}></ChildObjects>"),
        ),
    );
    let directory = match kind {
        "CommonCommand" => "CommonCommands",
        "CommandGroup" => "CommandGroups",
        "CommonPicture" => "CommonPictures",
        "DefinedType" => "DefinedTypes",
        _ => panic!("unsupported fixture kind"),
    };
    let path = f.source.join(directory).join(format!("{name}.xml"));
    write(
        &path,
        &format!(
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" xmlns:v="http://v8.1c.ru/8.1/data/core" xmlns:r="http://v8.1c.ru/8.3/xcf/readable" xmlns:c="http://v8.1c.ru/8.1/data/enterprise/current-config" xmlns:x="http://www.w3.org/2001/XMLSchema" version="2.20"><{kind} uuid="16160000-0000-4000-8000-000000000001"><Properties><Name>{name}</Name>{properties}<Vendor>hidden</Vendor></Properties></{kind}></MetaDataObject>"#
        ),
    );
    path
}
#[test]
fn command_root_facts_preserve_kind_specific_xml_without_default_values() {
    let f = RealReaderFixture::new();
    install(
        &f,
        "CommonCommand",
        "Probe",
        r#"<Representation>PictureAndText</Representation><ToolTip><v:item><v:lang>ru</v:lang><v:content>Подсказка</v:content></v:item><v:item><v:lang>en</v:lang><v:content>Hint</v:content></v:item></ToolTip><Picture><r:Ref>StdPicture.Print</r:Ref><r:LoadTransparent>false</r:LoadTransparent></Picture><CommandParameterType><v:Type>x:boolean</v:Type></CommandParameterType><ParameterUseMode>Multiple</ParameterUseMode><ModifiesData>true</ModifiesData><Comment>not applicable</Comment>"#,
    );
    let result = f.view_service().view(ViewRequest::new(COMMAND).unwrap());
    assert!(result.ok, "{result:?}");
    let node = result.data.unwrap();
    assert_eq!(node["props"]["Representation"], "PictureAndText");
    assert_eq!(node["props"]["ParameterUseMode"], "Multiple");
    assert_eq!(node["props"]["ModifiesData"], true);
    assert_eq!(
        node["props"]["Picture"],
        json!({"Ref":"StdPicture.Print","LoadTransparent":false})
    );
    assert_eq!(
        node["props"]["ToolTip"],
        json!([{"language":"ru","content":"Подсказка"},{"language":"en","content":"Hint"}])
    );
    assert_eq!(
        node["props"]["CommandParameterType"]["variants"],
        json!([{"kind":"boolean"}])
    );
    assert!(node["props"].get("Comment").is_none());
    assert!(node["props"].get("Vendor").is_none());
    assert!(node.get("limits").is_none(), "{node}");
}

#[test]
fn command_group_profile_preserves_comment_and_rejects_command_only_fields() {
    for representation in ["Auto", "Text", "Picture", "PictureAndText"] {
        let f = RealReaderFixture::new();
        install(&f,"CommandGroup","Group",&format!("<Comment>  Печать\n</Comment><Representation>{representation}</Representation><ToolTip/><Picture/><ModifiesData>true</ModifiesData><ParameterUseMode>Single</ParameterUseMode>"));
        let node = f
            .view_service()
            .view(ViewRequest::new("main:CommandGroup.Group").unwrap())
            .data
            .unwrap();
        assert_eq!(node["props"]["Comment"], "  Печать\n");
        assert_eq!(node["props"]["Representation"], representation);
        for absent in ["ModifiesData", "ParameterUseMode", "Picture", "ToolTip"] {
            assert!(node["props"].get(absent).is_none(), "{node}");
        }
        assert!(node.get("limits").is_none(), "{node}");
    }
}

#[test]
fn command_root_facts_keep_independent_picture_fields_and_type_failures_local() {
    for (picture,expected) in [
        ("<r:Ref>invalid</r:Ref><r:LoadTransparent>false</r:LoadTransparent>",json!({"LoadTransparent":false})),
        ("<r:Ref>StdPicture.Print</r:Ref><r:LoadTransparent>bad</r:LoadTransparent>",json!({"Ref":"StdPicture.Print"})),
        ("<r:Ref xmlns:r=\"urn:foreign\">CommonPicture.Hidden</r:Ref><r:LoadTransparent>true</r:LoadTransparent>",json!({"LoadTransparent":true})),
        ("<r:Ref>one</r:Ref><r:Ref>two</r:Ref><r:LoadTransparent>false</r:LoadTransparent>",json!({"LoadTransparent":false})),
    ] {
        let f=RealReaderFixture::new();
        install(&f,"CommonCommand","Probe",&format!("<Representation>TextPicture</Representation><ParameterUseMode>Single</ParameterUseMode><ModifiesData>false</ModifiesData><Picture>{picture}</Picture><CommandParameterType><v:Type>x:futureType</v:Type></CommandParameterType>"));
        let node=f.view_service().view(ViewRequest::new(COMMAND).unwrap()).data.unwrap();
        assert_eq!(node["props"]["Picture"],expected);assert_eq!(node["props"]["ModifiesData"],false);assert_eq!(node["props"]["ParameterUseMode"],"Single");
        assert!(node["props"].get("Representation").is_none());assert!(node["props"].get("CommandParameterType").is_none());
        assert!(node["limits"].to_string().contains("validation_warning"));assert!(node["limits"].to_string().contains("Representation"));
    }
}

#[test]
fn command_root_relations_prove_picture_and_parameter_targets_without_companion_dependency() {
    let f = RealReaderFixture::new();
    let picture = install(&f, "CommonPicture", "Icon", "");
    install(
        &f,
        "DefinedType",
        "Option",
        "<Type><v:Type>x:boolean</v:Type></Type>",
    );
    install(&f,"CommonCommand","Probe","<Picture><r:Ref>CommonPicture.Icon</r:Ref><r:LoadTransparent>false</r:LoadTransparent></Picture><CommandParameterType><v:TypeSet>c:DefinedType.Option</v:TypeSet><v:Type>c:CatalogRef.Items</v:Type></CommandParameterType>");
    let service = f.view_service();
    let relation = service.view(ViewRequest::new(&format!("{COMMAND}.Relation")).unwrap());
    assert!(relation.ok, "{relation:?}");
    assert_eq!(
        relation.data.unwrap()["items"],
        json!([
            {"relation":"picture","at":"main:CommonPicture.Icon","kind":"CommonPicture"},
            {"relation":"parameterType","at":"main:DefinedType.Option","kind":"DefinedType"},
            {"relation":"parameterType","at":"main:Catalog.Items","kind":"Catalog"}
        ])
    );
    let original = fs::read_to_string(&picture).unwrap();
    for changed in [
        original.replace("<Name>Icon</Name>", "<Name>Other</Name>"),
        original.replace("CommonPicture", "DefinedType"),
        original.replace("http://v8.1c.ru/8.3/MDClasses", "urn:foreign"),
    ] {
        write(&picture, &changed);
        let renamed = service
            .view(ViewRequest::new(&format!("{COMMAND}.Relation")).unwrap())
            .data
            .unwrap();
        assert_eq!(
            renamed["items"].as_array().unwrap().len(),
            2,
            "stale proof after target change: {renamed}"
        );
        assert!(renamed["limits"].to_string().contains("Icon"));
    }
    fs::remove_file(picture).unwrap();
    let node = service
        .view(ViewRequest::new(COMMAND).unwrap())
        .data
        .unwrap();
    assert_eq!(node["props"]["Picture"]["Ref"], "CommonPicture.Icon");
    assert!(node["limits"]
        .to_string()
        .contains("relations.commandPicture"));
    let relation = service
        .view(ViewRequest::new(&format!("{COMMAND}.Relation")).unwrap())
        .data
        .unwrap();
    assert_eq!(relation["items"].as_array().unwrap().len(), 2);
    assert!(relation["limits"].to_string().contains("Icon"));
}

#[test]
fn command_root_facts_refresh_properties_and_return_whole_compounds() {
    let f = RealReaderFixture::new();
    let path = install(
        &f,
        "CommonCommand",
        "Probe",
        "<Picture/><CommandParameterType/><ModifiesData>false</ModifiesData>",
    );
    let service = f.view_service();
    assert_eq!(
        service
            .view(ViewRequest::new(COMMAND).unwrap())
            .data
            .unwrap()["props"]["ModifiesData"],
        false
    );
    let long = "я".repeat(1200);
    write(&path,&fs::read_to_string(&path).unwrap().replace("<ModifiesData>false","<ModifiesData>true").replace("<Picture/>",&format!("<Picture><r:Ref>StdPicture.{long}</r:Ref><r:LoadTransparent>false</r:LoadTransparent></Picture>")));
    let node = service
        .view(ViewRequest::new(COMMAND).unwrap())
        .data
        .unwrap();
    assert_eq!(node["props"]["ModifiesData"], true);
    assert!(node["props"].get("Picture").is_none());
    assert!(node["limits"].to_string().contains(".Props.Picture"));
    let whole = crate::infrastructure::v13_read_projection::with_whole_prop("Picture", || {
        service.view(ViewRequest::new(COMMAND).unwrap())
    });
    assert_eq!(
        whole.data.unwrap()["props"]["Picture"]["Ref"],
        format!("StdPicture.{long}")
    );
}

#[test]
fn command_root_relations_refresh_registration_and_recovery_in_the_same_service() {
    let f = RealReaderFixture::new();
    install(&f, "CommonPicture", "Icon", "");
    install(
        &f,
        "CommonCommand",
        "Probe",
        "<Picture><r:Ref>CommonPicture.Icon</r:Ref></Picture><CommandParameterType/>",
    );
    let config = f.source.join("Configuration.xml");
    let original = fs::read_to_string(&config).unwrap();
    let service = f.view_service();
    let first = service
        .view(ViewRequest::new(&format!("{COMMAND}.Relation")).unwrap())
        .data
        .unwrap();
    assert_eq!(first["items"].as_array().unwrap().len(), 1);
    write(
        &config,
        &original.replace("<CommonPicture>Icon</CommonPicture>", ""),
    );
    let removed = service
        .view(ViewRequest::new(&format!("{COMMAND}.Relation")).unwrap())
        .data
        .unwrap();
    assert!(
        removed["items"].as_array().unwrap().is_empty(),
        "stale target registration: {removed}"
    );
    assert!(removed["limits"].to_string().contains("Icon"));
    write(&config, &original);
    let restored = service
        .view(ViewRequest::new(&format!("{COMMAND}.Relation")).unwrap())
        .data
        .unwrap();
    assert_eq!(restored["items"].as_array().unwrap().len(), 1);
    write(
        &config,
        &original.replace("<CommonCommand>Probe</CommonCommand>", ""),
    );
    assert!(
        !service.view(ViewRequest::new(COMMAND).unwrap()).ok,
        "an unregistered command must not remain readable"
    );
}
