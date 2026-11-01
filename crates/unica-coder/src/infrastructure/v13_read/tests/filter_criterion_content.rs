use super::*;
const REFERENCES: &[&str] = &[
    "Document.Встреча.TabularSection.Участники.Attribute.Контакт",
    "Document.ЗапланированноеВзаимодействие.TabularSection.Участники.Attribute.Контакт",
    "Document.ТелефонныйЗвонок.Attribute.АбонентКонтакт",
    "Document.ЭлектронноеПисьмоВходящее.Attribute.ОтправительКонтакт",
    "Document.ЭлектронноеПисьмоИсходящее.TabularSection.ПолучателиПисьма.Attribute.Контакт",
    "Catalog.СделкиСКлиентами.Attribute.Партнер",
    "Catalog.СделкиСКлиентами.TabularSection.ПартнерыИКонтактныеЛица.Attribute.Партнер",
    "Catalog.СделкиСКлиентами.TabularSection.ПартнерыИКонтактныеЛица.Attribute.КонтактноеЛицо",
];
fn content(refs: &[&str]) -> String {
    format!(
        "<Content>{}</Content>",
        refs.iter()
            .map(|r| format!("<r:Item xsi:type=\"r:MDObjectRef\">{r}</r:Item>"))
            .collect::<String>()
    )
}
fn install(f: &RealReaderFixture, name: &str, properties: &str) -> PathBuf {
    let config = f.source.join("Configuration.xml");
    write(
        &config,
        &fs::read_to_string(&config).unwrap().replace(
            "</ChildObjects>",
            &format!("<FilterCriterion>{name}</FilterCriterion></ChildObjects>"),
        ),
    );
    let path = f.source.join("FilterCriteria").join(format!("{name}.xml"));
    write(
        &path,
        &format!(
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" xmlns:r="http://v8.1c.ru/8.3/xcf/readable" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" version="2.20"><FilterCriterion uuid="16150000-0000-4000-8000-000000000001"><Properties><Name>{name}</Name>{properties}<Vendor>hidden</Vendor></Properties></FilterCriterion></MetaDataObject>"#
        ),
    );
    path
}
#[test]
fn filter_criterion_content_preserves_eight_full_declarations_and_false_true() {
    for (name, refs, flag) in [
        ("Contacts", &REFERENCES[..5], true),
        ("Deals", &REFERENCES[5..], false),
    ] {
        let f = RealReaderFixture::new();
        let path = install(
            &f,
            name,
            &format!(
                "<UseStandardCommands>{flag}</UseStandardCommands>{}",
                content(refs)
            ),
        );
        let before = fs::read(&path).unwrap();
        let service = f.view_service();
        let owner = format!("main:FilterCriterion.{name}");
        let node = service
            .view(ViewRequest::new(&owner).unwrap())
            .data
            .unwrap();
        assert_eq!(node["props"]["UseStandardCommands"], flag);
        assert!(node["props"].get("Vendor").is_none());
        let branch = node["branches"]
            .as_array()
            .unwrap()
            .iter()
            .find(|b| b["at"].as_str().unwrap().ends_with(".Relation"))
            .unwrap();
        assert_eq!(branch["count"], refs.len());
        let rows = service
            .view(ViewRequest::new(branch["at"].as_str().unwrap()).unwrap())
            .data
            .unwrap();
        assert_eq!(
            rows["items"],
            json!(refs
                .iter()
                .map(|r| json!({"relation":"content","at":format!("main:{r}"),"kind":"Attribute"}))
                .collect::<Vec<_>>())
        );
        assert!(rows.get("limits").is_none(), "{rows}");
        assert_eq!(fs::read(path).unwrap(), before);
    }
}

#[test]
fn filter_criterion_content_distinguishes_empty_missing_and_partial_declarations() {
    let valid = "Catalog.Items.Attribute.Article";
    for (xml,count,limited) in [
        ("<Content/>".to_string(),0,false),
        ("".to_string(),0,true),
        ("<Content/><Content/>".to_string(),0,true),
        (format!("<Content><r:Item xsi:type=\"r:MDObjectRef\">{valid}</r:Item><r:Item xsi:type=\"r:MDObjectRef\">bad</r:Item><r:Item xsi:type=\"r:MDObjectRef\">{valid}</r:Item></Content>"),2,true),
        (content(&[valid,valid]),2,false),
    ] {
        let f=RealReaderFixture::new();install(&f,"Probe",&format!("<UseStandardCommands>false</UseStandardCommands>{xml}"));let service=f.view_service();
        let owner=service.view(ViewRequest::new("main:FilterCriterion.Probe").unwrap()).data.unwrap();assert_eq!(owner["props"]["UseStandardCommands"],false);
        let branch=owner["branches"].as_array().unwrap().iter().find(|b|b["at"]=="main:FilterCriterion.Probe.Relation").unwrap();assert_eq!(branch["count"],count);
        let relation=service.view(ViewRequest::new(branch["at"].as_str().unwrap()).unwrap()).data.unwrap();assert_eq!(relation["items"].as_array().unwrap().len(),count);assert_eq!(relation.get("limits").is_some(),limited,"{xml}: {relation}");assert_eq!(owner.get("limits").is_some(),limited);
    }
}

#[test]
fn filter_criterion_boolean_damage_does_not_hide_content_or_invent_false() {
    for raw in ["", "<UseStandardCommands/>","<UseStandardCommands>bad</UseStandardCommands>","<UseStandardCommands><Nested/></UseStandardCommands>","<UseStandardCommands>false</UseStandardCommands><UseStandardCommands>true</UseStandardCommands>","<UseStandardCommands xmlns=\"urn:foreign\">false</UseStandardCommands>"] {
        let f=RealReaderFixture::new();install(&f,"Probe",&format!("{raw}{}",content(&["Catalog.Items.Attribute.Article"])));let service=f.view_service();
        let owner=service.view(ViewRequest::new("main:FilterCriterion.Probe").unwrap()).data.unwrap();assert!(owner["props"].get("UseStandardCommands").is_none());assert_eq!(owner.get("limits").is_some(),!raw.is_empty(),"{owner}");
        let relation=service.view(ViewRequest::new("main:FilterCriterion.Probe.Relation").unwrap()).data.unwrap();assert_eq!(relation["items"].as_array().unwrap().len(),1);assert!(relation.get("limits").is_none(),"{relation}");
    }
}

#[test]
fn filter_criterion_declarations_keep_dangling_targets_and_follow_real_nested_targets() {
    let f = RealReaderFixture::new();
    let refs = [
        "Catalog.Items.Attribute.Code",
        "Catalog.Items.TabularSection.Lines.Attribute.Quantity",
        "Catalog.Gone.Attribute.Missing",
        "Catalog.Items.Attribute.Missing",
    ];
    install(&f, "Probe", &content(&refs));
    let service = f.view_service();
    let relation = service
        .view(ViewRequest::new("main:FilterCriterion.Probe.Relation").unwrap())
        .data
        .unwrap();
    assert_eq!(relation["items"].as_array().unwrap().len(), 4);
    assert!(relation.get("limits").is_none());
    for (i, row) in relation["items"].as_array().unwrap().iter().enumerate() {
        let result = service.view(ViewRequest::new(row["at"].as_str().unwrap()).unwrap());
        assert_eq!(result.ok, i < 2, "{row}: {result:?}");
    }
    for suffix in [".Entry", ".Entry.Attribute.Other"] {
        let rejected = service.view(
            ViewRequest::new(&format!("main:FilterCriterion.Probe.Relation{suffix}")).unwrap(),
        );
        assert!(!rejected.ok);
        assert_eq!(rejected.diagnostics[0]["code"], "not_found");
    }
}

#[test]
fn filter_criterion_reads_current_owner_bytes_and_registration() {
    let f = RealReaderFixture::new();
    let path = install(
        &f,
        "Probe",
        &format!(
            "<UseStandardCommands>false</UseStandardCommands>{}",
            content(&["Catalog.Items.Attribute.Article"])
        ),
    );
    let service = f.view_service();
    assert_eq!(
        service
            .view(ViewRequest::new("main:FilterCriterion.Probe").unwrap())
            .data
            .unwrap()["props"]["UseStandardCommands"],
        false
    );
    write(
        &path,
        &fs::read_to_string(&path)
            .unwrap()
            .replace("<UseStandardCommands>false", "<UseStandardCommands>true")
            .replace(
                "Catalog.Items.Attribute.Article",
                "Catalog.Items.TabularSection.Lines.Attribute.Quantity",
            ),
    );
    let owner = service
        .view(ViewRequest::new("main:FilterCriterion.Probe").unwrap())
        .data
        .unwrap();
    assert_eq!(owner["props"]["UseStandardCommands"], true);
    let relation = service
        .view(ViewRequest::new("main:FilterCriterion.Probe.Relation").unwrap())
        .data
        .unwrap();
    assert_eq!(
        relation["items"][0]["at"],
        "main:Catalog.Items.TabularSection.Lines.Attribute.Quantity"
    );
    let config = f.source.join("Configuration.xml");
    write(
        &config,
        &fs::read_to_string(&config)
            .unwrap()
            .replace("<FilterCriterion>Probe</FilterCriterion>", ""),
    );
    assert!(
        !service
            .view(ViewRequest::new("main:FilterCriterion.Probe").unwrap())
            .ok
    );
}

#[test]
fn filter_criterion_closed_parent_damage_preserves_identity_without_fabricating_facts() {
    let f = RealReaderFixture::new();
    let path = install(
        &f,
        "Probe",
        "<UseStandardCommands>false</UseStandardCommands><Content/>",
    );
    write(
        &path,
        &fs::read_to_string(&path)
            .unwrap()
            .replace("<Properties>", "<Properties vendor=\"hidden\">"),
    );
    let service = f.view_service();
    let result = service.view(ViewRequest::new("main:FilterCriterion.Probe").unwrap());
    assert!(result.ok);
    let node = result.data.unwrap();
    assert_eq!(node["props"]["kind"], "FilterCriterion");
    assert!(node["props"].get("UseStandardCommands").is_none());
    assert!(node["limits"]
        .to_string()
        .contains("unsupported attributes"));
    let relation = service
        .view(ViewRequest::new("main:FilterCriterion.Probe.Relation").unwrap())
        .data
        .unwrap();
    assert!(relation["items"].as_array().unwrap().is_empty());
    assert!(relation["limits"]
        .to_string()
        .contains("unsupported attributes"));
}
