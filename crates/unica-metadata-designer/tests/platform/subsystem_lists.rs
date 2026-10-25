use crate::subsystem::{parse_subsystem_edit_model, patch_subsystem_lists};
const MD: &str = "http://v8.1c.ru/8.3/MDClasses";
const XR: &str = "http://v8.1c.ru/8.3/xcf/readable";
const XSI: &str = "http://www.w3.org/2001/XMLSchema-instance";

#[test]
fn subsystem_list_patch_preserves_raw_entries_and_foreign_nodes() {
    let retained = "<ref:Item xsi:type=\"ref:MDObjectRef\" extra=\"keep\">Document.&#70;irst</ref:Item><!--keep--><foreign:Item>Document.Foreign</foreign:Item>";
    let source = format!("\u{feff}<md:MetaDataObject xmlns:md=\"{MD}\" xmlns:ref=\"{XR}\" xmlns:xsi=\"{XSI}\" xmlns:foreign=\"urn:other\" version=\"2.20\">\r\n<md:Subsystem><md:Properties><md:Name>Sales</md:Name><md:Content>{retained}<ref:Item xsi:type=\"ref:MDObjectRef\">Document.Remove</ref:Item></md:Content></md:Properties><md:ChildObjects/></md:Subsystem></md:MetaDataObject>");
    let mut model = parse_subsystem_edit_model(&source, "test").unwrap();
    assert_eq!(model.content, ["Document.First", "Document.Remove"]);
    assert_eq!(
        patch_subsystem_lists(&source, &model, None).unwrap(),
        source
    );
    model.content = vec!["Document.First".into(), "Document.Added".into()];
    let actual = patch_subsystem_lists(&source, &model, None).unwrap();
    let expected = source.replace(
        "<ref:Item xsi:type=\"ref:MDObjectRef\">Document.Remove</ref:Item>",
        "<ref:Item xsi:type=\"ref:MDObjectRef\">Document.Added</ref:Item>",
    );
    assert_eq!(actual, expected);
    model.content.clear();
    let actual = patch_subsystem_lists(&source, &model, None).unwrap();
    assert!(actual.contains("<!--keep--><foreign:Item>Document.Foreign</foreign:Item>"));
    assert!(!actual.contains("<ref:Item"));
}

#[test]
fn subsystem_list_patch_binds_missing_or_rebound_prefixes() {
    for declarations in [
        "",
        "xmlns:xr=\"urn:foreign\" xmlns:xsi=\"urn:foreign-instance\"",
        "xmlns:xsi=\"http://v8.1c.ru/8.3/xcf/readable\"",
        "xmlns:xr=\"http://www.w3.org/2001/XMLSchema-instance\"",
    ] {
        let source = format!("<MetaDataObject xmlns=\"{MD}\" {declarations} version=\"2.20\"><Subsystem><Properties><Name>Sales</Name><Content /></Properties><ChildObjects /></Subsystem></MetaDataObject>");
        let mut model = parse_subsystem_edit_model(&source, "test").unwrap();
        model.content.push("Document.First".into());
        model.children.push("Orders".into());
        let actual = patch_subsystem_lists(&source, &model, None).unwrap();
        let doc = roxmltree::Document::parse(&actual).unwrap();
        let item = doc
            .descendants()
            .find(|n| n.has_tag_name((XR, "Item")))
            .unwrap();
        assert_eq!(item.text(), Some("Document.First"));
        let ty = item.attribute((XSI, "type")).unwrap();
        let (prefix, local) = ty.split_once(':').unwrap();
        assert_eq!(item.lookup_namespace_uri(Some(prefix)), Some(XR));
        assert_eq!(local, "MDObjectRef");
        assert!(doc
            .descendants()
            .any(|n| n.has_tag_name((MD, "Subsystem")) && n.text() == Some("Orders")));
    }
}

#[test]
fn subsystem_list_patch_refuses_ambiguous_direct_containers() {
    for duplicate in [
        "<Properties/><Properties/>",
        "<Properties><Content/><Content/></Properties>",
        "<Properties/><ChildObjects/>",
    ] {
        let source = format!("<MetaDataObject xmlns=\"{MD}\" version=\"2.20\"><Subsystem>{duplicate}<ChildObjects/></Subsystem></MetaDataObject>");
        let model = parse_subsystem_edit_model(&source, "test").unwrap();
        assert!(patch_subsystem_lists(&source, &model, None)
            .unwrap_err()
            .contains("ambiguous"));
    }
}

/// Выгрузка 8.3.27 (BOM, CRLF, табуляции): удалённый элемент уходит вместе со
/// строкой, добавленный встаёт своей строкой с отступом соседей, опустевший
/// список становится пустым тегом, как пишет платформа (#1889).
#[test]
fn subsystem_list_patch_keeps_the_platform_layout_of_a_dump() {
    let source = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tests/fixtures/acceptance/workspace/src/Subsystems/Администрирование.xml"
    ))
    .unwrap();
    assert!(source.starts_with('\u{feff}') && source.contains("\r\n"));
    let valuta = "\t\t\t\t<xr:Item xsi:type=\"xr:MDObjectRef\">Catalog.Валюты</xr:Item>\r\n";
    let child = "\t\t<ChildObjects>\r\n\t\t\t<Subsystem>КонтрольРаботыПользователей</Subsystem>\r\n\t\t</ChildObjects>\r\n";
    assert!(source.contains(valuta) && source.contains(child));

    let mut model = parse_subsystem_edit_model(&source, "test").unwrap();
    model.content.retain(|item| item != "Catalog.Валюты");
    let actual = patch_subsystem_lists(&source, &model, None).unwrap();
    assert_eq!(actual, source.replacen(valuta, "", 1));

    let mut model = parse_subsystem_edit_model(&source, "test").unwrap();
    model.children.push("НастройкиПользователей".into());
    let actual = patch_subsystem_lists(&source, &model, None).unwrap();
    assert_eq!(
        actual,
        source.replacen(
            child,
            "\t\t<ChildObjects>\r\n\t\t\t<Subsystem>КонтрольРаботыПользователей</Subsystem>\r\n\t\t\t<Subsystem>НастройкиПользователей</Subsystem>\r\n\t\t</ChildObjects>\r\n",
            1
        )
    );

    let mut model = parse_subsystem_edit_model(&source, "test").unwrap();
    model.children.clear();
    let actual = patch_subsystem_lists(&source, &model, None).unwrap();
    assert_eq!(actual, source.replacen(child, "\t\t<ChildObjects/>\r\n", 1));

    // Пустой тег раскрывается в многострочный список, а обратное удаление
    // возвращает исходные байты.
    let emptied = actual;
    let mut model = parse_subsystem_edit_model(&emptied, "test").unwrap();
    model.children.push("КонтрольРаботыПользователей".into());
    assert_eq!(
        patch_subsystem_lists(&emptied, &model, None).unwrap(),
        source
    );
}
