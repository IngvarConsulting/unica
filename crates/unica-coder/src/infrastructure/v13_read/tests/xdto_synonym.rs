use super::*;

const PACKAGE: &str = "main:XDTOPackage.EnterpriseData_1_17_3";

fn install_synonym(fixture: &RealReaderFixture, synonym: &str) -> PathBuf {
    let path = fixture
        .source
        .join("XDTOPackages/EnterpriseData_1_17_3.xml");
    let text = fs::read_to_string(&path).unwrap();
    assert!(text.contains("<Synonym/>"));
    write(&path, &text.replace("<Synonym/>", synonym));
    path
}

/// rule: INV.SOURCE.METADATA-OBJECT-FACTS
#[test]
fn xdto_synonym_reaches_public_view_from_the_descriptor_without_writes() {
    for (xml, expected) in [
        ("<Synonym><v8:item><v8:lang>en</v8:lang><v8:content>Contracts</v8:content></v8:item><v8:item><v8:lang>ru</v8:lang><v8:content>  Договоры &amp; акты<![CDATA[.]]></v8:content></v8:item></Synonym>", "en = 'Contracts'; ru = '  Договоры & акты.'"),
        ("<Synonym><v8:item><v8:lang>en</v8:lang><v8:content>Contracts</v8:content></v8:item></Synonym>", "en = 'Contracts'"),
        ("<Synonym/>", ""),
        ("<Synonym></Synonym>", ""),
        ("<Synonym><v8:item><v8:lang>ru</v8:lang><v8:content/></v8:item></Synonym>", "ru = ''"),
    ] {
        let fixture = RealReaderFixture::new();
        let descriptor = install_synonym(&fixture, xml);
        let before = fs::read(&descriptor).unwrap();
        let package = fixture.source.join("XDTOPackages/EnterpriseData_1_17_3/Ext/Package.bin");
        let package_before = fs::read(&package).unwrap();
        let service = fixture.view_service();
        let result = service.view(ViewRequest::new(PACKAGE).unwrap());
        assert!(result.ok, "XDTO root must remain readable");
        let data = result.data.unwrap();
        assert_eq!(data["props"].get("Synonym").and_then(serde_json::Value::as_str), (!expected.is_empty()).then_some(expected));
        assert_eq!(data["props"]["Comment"], "");
        assert!(data["props"]["targetNamespace"].is_string());
        assert!(data["props"]["total"].is_number());
        assert!(data.get("limits").is_none(), "{data}");
        let types = service.view(ViewRequest::new(&format!("{PACKAGE}.Type")).unwrap());
        assert!(types.ok);
        assert!(!types.data.unwrap()["items"].as_array().unwrap().is_empty());
        assert_eq!(fs::read(&descriptor).unwrap(), before);
        assert_eq!(fs::read(&package).unwrap(), package_before);
    }
}

/// rule: INV.SOURCE.METADATA-OBJECT-FACTS
#[test]
fn xdto_synonym_absence_and_damage_do_not_hide_independent_package_facts() {
    for (xml, damaged) in [
        ("", false),
        ("<Synonym/><Synonym/>", true),
        ("<Synonym xmlns=\"urn:foreign\"/>", true),
        ("<Synonym><broken/></Synonym>", true),
        ("<Synonym>not localized</Synonym>", true),
        (
            "<Synonym><v8:item><v8:lang>ru</v8:lang></v8:item></Synonym>",
            true,
        ),
    ] {
        let fixture = RealReaderFixture::new();
        install_synonym(&fixture, xml);
        let result = fixture
            .view_service()
            .view(ViewRequest::new(PACKAGE).unwrap());
        assert!(result.ok, "one damaged property must not hide the package");
        let data = result.data.unwrap();
        assert!(data["props"].get("Synonym").is_none());
        assert!(data["props"]["targetNamespace"].is_string());
        assert_eq!(
            data.get("limits").is_some_and(|limits| limits
                .as_array()
                .unwrap()
                .iter()
                .any(|limit| limit.as_str().unwrap().contains("Synonym"))),
            damaged,
            "{data}"
        );
    }
}

#[test]
fn xdto_synonym_compact_budget_preserves_whole_property_reading() {
    for size in [1020, 1021] {
        let fixture = RealReaderFixture::new();
        let text = "Я".repeat(size);
        install_synonym(&fixture, &format!("<Synonym><v8:item><v8:lang>ru</v8:lang><v8:content>{text}</v8:content></v8:item></Synonym>"));
        let service = fixture.view_service();
        let result = service.view(ViewRequest::new(PACKAGE).unwrap());
        assert!(result.ok);
        let data = result.data.unwrap();
        if size == 1020 {
            assert_eq!(data["props"]["Synonym"], format!("ru = '{text}'"));
        } else {
            assert!(data["props"].get("Synonym").is_none());
            assert!(data["limits"]
                .as_array()
                .unwrap()
                .iter()
                .any(|limit| limit.as_str().unwrap().contains(".Props.Synonym")));
        }
        let whole = crate::infrastructure::v13_read_projection::with_whole_prop("Synonym", || {
            service.view(ViewRequest::new(PACKAGE).unwrap())
        });
        assert!(whole.ok);
        assert_eq!(whole.data.unwrap()["props"]["Synonym"], format!("ru = '{text}'"));
    }
}

#[test]
fn xdto_synonym_read_keeps_registration_and_descriptor_identity_required() {
    for damage in ["registration", "owner", "namespace", "name"] {
        let fixture = RealReaderFixture::new();
        let descriptor = install_synonym(&fixture, "<Synonym><v8:item><v8:lang>ru</v8:lang><v8:content>Договоры</v8:content></v8:item></Synonym>");
        let original = fs::read_to_string(&descriptor).unwrap();
        match damage {
            "registration" => {
                let configuration = fixture.source.join("Configuration.xml");
                let text = fs::read_to_string(&configuration).unwrap();
                write(
                    &configuration,
                    &text.replace("<XDTOPackage>EnterpriseData_1_17_3</XDTOPackage>", ""),
                );
            }
            "owner" => write(
                &descriptor,
                &original.replace("http://v8.1c.ru/8.3/MDClasses", "urn:foreign"),
            ),
            "namespace" => write(
                &descriptor,
                &original.replace(
                    "http://v8.1c.ru/edi/edi_stnd/EnterpriseData/1.17.3",
                    "urn:other",
                ),
            ),
            "name" => write(
                &descriptor,
                &original.replace("<Name>EnterpriseData_1_17_3</Name>", "<Name>Other</Name>"),
            ),
            _ => unreachable!(),
        }
        let result = fixture
            .view_service()
            .view(ViewRequest::new(PACKAGE).unwrap());
        assert!(
            !result.ok,
            "{damage} must not become successful Synonym reading"
        );
    }
}
