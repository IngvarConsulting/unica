use super::*;

fn install(fixture: &RealReaderFixture, properties: &str) -> PathBuf {
    let path = fixture.source.join("Configuration.xml");
    let mut xml = fs::read_to_string(&path).unwrap();
    for name in [
        "UseManagedFormInOrdinaryApplication",
        "UseOrdinaryFormInManagedApplication",
        "BriefInformation",
        "DetailedInformation",
        "Copyright",
        "VendorInformationAddress",
        "ConfigurationInformationAddress",
    ] {
        let pattern = regex::Regex::new(&format!(
            r"(?s)<{name}(?:\s[^>]*)?>(.*?)</{name}>|<{name}\s*/>"
        ))
        .unwrap();
        xml = pattern.replace_all(&xml, "").into_owned();
    }
    write(
        &path,
        &xml.replacen("</Properties>", &format!("{properties}</Properties>"), 1),
    );
    path
}

#[test]
fn configuration_information_reaches_public_properties_without_changing_sources() {
    for (managed, ordinary) in [(true, false), (false, true)] {
        let fixture = RealReaderFixture::new();
        let mut properties = format!("<UseManagedFormInOrdinaryApplication>{managed}</UseManagedFormInOrdinaryApplication><UseOrdinaryFormInManagedApplication>{ordinary}</UseOrdinaryFormInManagedApplication>");
        for (xml_name, text) in [
            ("BriefInformation", "Кратко"),
            ("DetailedInformation", "Подробно"),
            ("Copyright", "Автор 2026"),
            ("VendorInformationAddress", "https://vendor.example"),
            ("ConfigurationInformationAddress", "https://product.example"),
        ] {
            properties.push_str(&format!("<{xml_name} xmlns:v8=\"http://v8.1c.ru/8.1/data/core\"><v8:item><v8:lang>ru</v8:lang><v8:content>{text}</v8:content></v8:item><v8:item><v8:lang>en</v8:lang><v8:content/></v8:item></{xml_name}>"));
        }
        let path = install(&fixture, &properties);
        let before = fs::read(&path).unwrap();
        let result = fixture
            .view_service()
            .view(ViewRequest::new("main:Configuration").unwrap());
        assert!(result.ok);
        let data = result.data.as_ref().unwrap();
        let props = &data["props"]["properties"];
        assert_eq!(props["useManagedFormInOrdinaryApplication"], managed);
        assert_eq!(props["useOrdinaryFormInManagedApplication"], ordinary);
        for (key, text) in [
            ("briefInformation", "Кратко"),
            ("detailedInformation", "Подробно"),
            ("copyright", "Автор 2026"),
            ("vendorInformationAddress", "https://vendor.example"),
            ("configurationInformationAddress", "https://product.example"),
        ] {
            assert_eq!(
                props[key],
                json!([{"language":"ru", "content":text}, {"language":"en", "content":""}])
            );
        }
        assert_eq!(data["at"], "main:Configuration");
        assert!(!data["branches"].as_array().unwrap().is_empty());
        assert_eq!(fs::read(&path).unwrap(), before);
    }
}

#[test]
fn configuration_information_damage_is_local_and_distinct_from_absence() {
    for (property, public, value) in [
        (
            "UseManagedFormInOrdinaryApplication",
            "useManagedFormInOrdinaryApplication",
            "false",
        ),
        (
            "UseOrdinaryFormInManagedApplication",
            "useOrdinaryFormInManagedApplication",
            "true",
        ),
        (
            "BriefInformation",
            "briefInformation",
            "<v8:item><v8:lang>ru</v8:lang><v8:content>Text</v8:content></v8:item>",
        ),
        (
            "DetailedInformation",
            "detailedInformation",
            "<v8:item><v8:lang>ru</v8:lang><v8:content>Text</v8:content></v8:item>",
        ),
        (
            "Copyright",
            "copyright",
            "<v8:item><v8:lang>ru</v8:lang><v8:content>Text</v8:content></v8:item>",
        ),
        (
            "VendorInformationAddress",
            "vendorInformationAddress",
            "<v8:item><v8:lang>ru</v8:lang><v8:content>Text</v8:content></v8:item>",
        ),
        (
            "ConfigurationInformationAddress",
            "configurationInformationAddress",
            "<v8:item><v8:lang>ru</v8:lang><v8:content>Text</v8:content></v8:item>",
        ),
    ] {
        for raw in [
            format!("<{property}>bad</{property}>"),
            format!("<{property}/>"),
            format!("<{property}><Nested/></{property}>"),
            format!("<{property}>{value}</{property}><{property}>{value}</{property}>"),
            format!("<{property} xmlns=\"urn:foreign\">{value}</{property}>"),
            format!("<{property} extra=\"yes\">{value}</{property}>"),
        ] {
            if raw == format!("<{property}/>")
                && property != "UseManagedFormInOrdinaryApplication"
                && property != "UseOrdinaryFormInManagedApplication"
            {
                continue;
            }
            let fixture = RealReaderFixture::new();
            let independent = if property == "Copyright" {
                "BriefInformation"
            } else {
                "Copyright"
            };
            install(&fixture, &format!("<{independent} xmlns:v8=\"http://v8.1c.ru/8.1/data/core\"><v8:item><v8:lang>en</v8:lang><v8:content>Independent</v8:content></v8:item></{independent}>{raw}"));
            let result = fixture
                .view_service()
                .view(ViewRequest::new("main:Configuration").unwrap());
            assert!(result.ok, "{raw}: {:?}", result.diagnostics);
            let data = result.data.as_ref().unwrap();
            assert!(data["props"]["properties"][public].is_null());
            assert!(data["limits"]
                .as_array()
                .unwrap()
                .iter()
                .any(|limit| limit.as_str().is_some_and(|text| text.contains(property))));
            {
                let independent_key = if independent == "Copyright" {
                    "copyright"
                } else {
                    "briefInformation"
                };
                assert_eq!(
                    data["props"]["properties"][independent_key][0]["content"],
                    "Independent"
                );
            }
            assert_eq!(data["at"], "main:Configuration");
        }
    }
}

#[test]
fn configuration_information_absence_and_empty_translations_do_not_invent_values() {
    for raw in [
        "",
        "<BriefInformation/>",
        "<BriefInformation>  </BriefInformation>",
    ] {
        let fixture = RealReaderFixture::new();
        install(&fixture, raw);
        let result = fixture
            .view_service()
            .view(ViewRequest::new("main:Configuration").unwrap());
        assert!(result.ok);
        let data = result.data.as_ref().unwrap();
        for key in [
            "useManagedFormInOrdinaryApplication",
            "useOrdinaryFormInManagedApplication",
            "briefInformation",
            "detailedInformation",
            "copyright",
            "vendorInformationAddress",
            "configurationInformationAddress",
        ] {
            assert!(data["props"]["properties"][key].is_null());
        }
        assert!(data
            .get("limits")
            .is_none_or(|limits| limits.as_array().is_some_and(Vec::is_empty)));
    }
}
