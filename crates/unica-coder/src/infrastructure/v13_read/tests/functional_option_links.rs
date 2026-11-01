use super::*;
const CONTENT: &[&str] = &[
    "Catalog.ЛицензииПоставщиковАлкогольнойПродукции",
    "CommonCommand.ОтчетЖурналУчетаРозничнойПродажиАлкогольнойПродукции",
    "Report.ЖурналУчетаРозничнойПродажиАлкогольнойПродукции",
    "CommonCommand.ОткрытьКаталогРасширений",
    "InformationRegister.ДокументыФизическихЛиц.Command.ДокументыКонтрагентаФизическогоЛица",
    "InformationRegister.ДокументыФизическихЛиц.Command.ДокументыПартнераФизическогоЛица",
];
const USE: &[&str] = &[
    "Catalog.Склады",
    "InformationRegister.НастройкиАдресныхСкладов.Dimension.Склад",
    "Catalog.Организации",
    "InformationRegister.УчетнаяПолитикаФинансовогоУчета.Dimension.Организация",
    "InformationRegister.НастройкиСистемыНалогообложения.Dimension.Организация",
    "InformationRegister.НастройкиУчетаНДС.Dimension.Организация",
    "InformationRegister.НастройкиУчетаУСН.Dimension.Организация",
];
fn declaration(kind: &str, refs: &[&str]) -> String {
    let (field, entry, typed) = if kind == "FunctionalOption" {
        ("Content", "Object", false)
    } else {
        ("Use", "Item", true)
    };
    format!(
        "<{field}>{}</{field}>",
        refs.iter()
            .map(|r| format!(
                "<r:{entry}{ty}>{r}</r:{entry}>",
                ty = if typed {
                    " xsi:type=\"r:MDObjectRef\""
                } else {
                    ""
                }
            ))
            .collect::<String>()
    )
}
fn install(f: &RealReaderFixture, kind: &str, name: &str, properties: &str) -> PathBuf {
    let config = f.source.join("Configuration.xml");
    write(
        &config,
        &fs::read_to_string(&config).unwrap().replace(
            "</ChildObjects>",
            &format!("<{kind}>{name}</{kind}></ChildObjects>"),
        ),
    );
    let dir = if kind == "FunctionalOption" {
        "FunctionalOptions"
    } else {
        "FunctionalOptionsParameters"
    };
    let path = f.source.join(dir).join(format!("{name}.xml"));
    write(
        &path,
        &format!(
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" xmlns:r="http://v8.1c.ru/8.3/xcf/readable" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" version="2.20"><{kind} uuid="16000000-0000-4000-8000-000000000001"><Properties><Name>{name}</Name>{properties}<Vendor>hidden</Vendor></Properties></{kind}></MetaDataObject>"#
        ),
    );
    path
}
#[test]
fn functional_option_links_preserve_all_thirteen_declarations_and_target_kinds() {
    for (kind, name, refs, role) in [
        ("FunctionalOption", "Alcohol", &CONTENT[..3], "content"),
        ("FunctionalOption", "Extensions", &CONTENT[3..4], "content"),
        ("FunctionalOption", "Documents", &CONTENT[4..], "content"),
        ("FunctionalOptionsParameter", "Warehouse", &USE[..2], "use"),
        (
            "FunctionalOptionsParameter",
            "Organization",
            &USE[2..],
            "use",
        ),
    ] {
        let f = RealReaderFixture::new();
        let path = install(&f, kind, name, &declaration(kind, refs));
        let before = fs::read(&path).unwrap();
        let service = f.view_service();
        let at = format!("main:{kind}.{name}");
        let node = service.view(ViewRequest::new(&at).unwrap()).data.unwrap();
        let relation = node["branches"]
            .as_array()
            .expect("new declaration route")
            .iter()
            .find(|b| b["at"].as_str().unwrap().ends_with(".Relation"))
            .unwrap();
        assert_eq!(relation["count"], refs.len());
        let result = service
            .view(ViewRequest::new(relation["at"].as_str().unwrap()).unwrap())
            .data
            .unwrap();
        let expected=refs.iter().map(|r|{let parts=r.split('.').collect::<Vec<_>>();json!({"relation":role,"at":format!("main:{r}"),"kind":if parts.len()==2 {parts[0]}else{parts[2]}})}).collect::<Vec<_>>();
        assert_eq!(result["items"], json!(expected));
        assert!(result.get("limits").is_none(), "{result}");
        assert!(node["props"].get("Vendor").is_none());
        assert!(node["props"].get("Content").is_none());
        assert!(node["props"].get("Use").is_none());
        assert_eq!(fs::read(path).unwrap(), before);
    }
}

#[test]
fn functional_option_empty_content_and_unavailable_parameter_use_are_distinct() {
    let f = RealReaderFixture::new();
    install(&f, "FunctionalOption", "Empty", "<Content/>");
    let service = f.view_service();
    let node = service
        .view(ViewRequest::new("main:FunctionalOption.Empty").unwrap())
        .data
        .unwrap();
    assert_eq!(node["branches"][0]["count"], 0);
    assert!(node.get("limits").is_none());
    let rows = service
        .view(ViewRequest::new("main:FunctionalOption.Empty.Relation").unwrap())
        .data
        .unwrap();
    assert_eq!(rows["items"], json!([]));
    assert!(rows.get("limits").is_none());
    for (name, xml, reason) in [
        ("Empty", "<Use/>", "explicitly empty"),
        ("Missing", "", "absent"),
        (
            "Bad",
            "<Use><r:Item xsi:type=\"r:MDObjectRef\">bad</r:Item></Use>",
            "unsupported",
        ),
        ("Duplicate", "<Use/><Use/>", "duplicate"),
    ] {
        let f = RealReaderFixture::new();
        install(&f, "FunctionalOptionsParameter", name, xml);
        let service = f.view_service();
        let at = format!("main:FunctionalOptionsParameter.{name}");
        let result = service.view(ViewRequest::new(&at).unwrap());
        assert!(result.ok, "{result:?}");
        let node = result.data.unwrap();
        assert_eq!(node["props"]["kind"], "FunctionalOptionsParameter");
        assert!(node["limits"].to_string().contains(reason), "{node}");
        assert!(node
            .get("branches")
            .is_none_or(|b| b
                .as_array()
                .unwrap()
                .iter()
                .all(|b| !b["at"].as_str().unwrap().ends_with(".Relation"))));
        let rejected = service.view(ViewRequest::new(&format!("{at}.Relation")).unwrap());
        assert!(!rejected.ok);
        assert_eq!(rejected.diagnostics[0]["code"], "provider_unavailable");
    }
}

#[test]
fn functional_declaration_entry_grammars_and_partial_lists_stay_independent() {
    for (kind,xml,role) in [
        ("FunctionalOption","<Content><r:Object>Catalog.Items</r:Object><r:Item xsi:type=\"r:MDObjectRef\">Catalog.Hidden</r:Item><r:Object>Catalog.Items</r:Object></Content>","content"),
        ("FunctionalOptionsParameter","<Use><r:Item xsi:type=\"r:MDObjectRef\">Catalog.Items</r:Item><r:Object>Catalog.Hidden</r:Object><r:Item xsi:type=\"r:MDObjectRef\">Catalog.Items</r:Item></Use>","use"),
    ] {
        let f=RealReaderFixture::new();install(&f,kind,"Probe",xml);let service=f.view_service();let at=format!("main:{kind}.Probe");let node=service.view(ViewRequest::new(&at).unwrap()).data.unwrap();assert_eq!(node["branches"][0]["count"],2);assert!(node["limits"].to_string().contains("[1]"));let relation=service.view(ViewRequest::new(&format!("{at}.Relation")).unwrap()).data.unwrap();assert_eq!(relation["items"],json!([{ "relation":role,"at":"main:Catalog.Items","kind":"Catalog"},{"relation":role,"at":"main:Catalog.Items","kind":"Catalog"}]));assert!(relation["limits"].to_string().contains("[1]"));
    }
}

#[test]
fn functional_declarations_preserve_dangling_targets_and_reject_extra_relation_tail() {
    for (kind, reference) in [
        ("FunctionalOption", "CommonCommand.Missing"),
        (
            "FunctionalOptionsParameter",
            "InformationRegister.Missing.Dimension.Unknown",
        ),
    ] {
        let f = RealReaderFixture::new();
        install(&f, kind, "Probe", &declaration(kind, &[reference]));
        let service = f.view_service();
        let at = format!("main:{kind}.Probe.Relation");
        let rows = service.view(ViewRequest::new(&at).unwrap()).data.unwrap();
        assert_eq!(rows["items"][0]["at"], format!("main:{reference}"));
        assert!(rows.get("limits").is_none());
        assert!(
            !service
                .view(ViewRequest::new(rows["items"][0]["at"].as_str().unwrap()).unwrap())
                .ok
        );
        for suffix in [".Entry", ".Entry.Attribute.Extra"] {
            let rejected = service.view(ViewRequest::new(&format!("{at}{suffix}")).unwrap());
            assert!(!rejected.ok);
            assert_eq!(rejected.diagnostics[0]["code"], "not_found");
        }
    }
}

#[test]
fn functional_declarations_refresh_owner_bytes_and_registration() {
    for kind in ["FunctionalOption", "FunctionalOptionsParameter"] {
        let f = RealReaderFixture::new();
        let path = install(&f, kind, "Probe", &declaration(kind, &["Catalog.Items"]));
        let service = f.view_service();
        let at = format!("main:{kind}.Probe.Relation");
        let first = service.view(ViewRequest::new(&at).unwrap()).data.unwrap();
        assert_eq!(first["items"].as_array().unwrap().len(), 1);
        write(
            &path,
            &fs::read_to_string(&path)
                .unwrap()
                .replace("Catalog.Items", "Catalog.Other"),
        );
        let next = service.view(ViewRequest::new(&at).unwrap()).data.unwrap();
        assert_eq!(next["items"][0]["at"], "main:Catalog.Other");
        let config = f.source.join("Configuration.xml");
        write(
            &config,
            &fs::read_to_string(&config)
                .unwrap()
                .replace(&format!("<{kind}>Probe</{kind}>"), ""),
        );
        assert!(!service.view(ViewRequest::new(&at).unwrap()).ok);
    }
}
