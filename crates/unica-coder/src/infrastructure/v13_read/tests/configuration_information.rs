use super::*;

fn set_information(fixture: &RealReaderFixture, extra: &str, large: bool) {
    let path = fixture.source.join("Configuration.xml");
    let text = fs::read_to_string(&path).unwrap();
    let mut text = text.trim_start_matches('\u{feff}').to_owned();
    let document = roxmltree::Document::parse(&text).unwrap();
    let properties = document
        .root_element()
        .children()
        .find(|n| n.is_element())
        .unwrap()
        .children()
        .find(|n| n.has_tag_name(("http://v8.1c.ru/8.3/MDClasses", "Properties")))
        .unwrap();
    let mut ranges = properties
        .children()
        .filter(|node| {
            node.is_element()
                && unica_metadata::configuration::CONFIGURATION_INFORMATION_PROPERTIES
                    .contains(&node.tag_name().name())
        })
        .map(|node| node.range())
        .collect::<Vec<_>>();
    ranges.reverse();
    for range in ranges {
        text.replace_range(range, "");
    }
    text = text.replacen("</Properties>", &format!("{extra}</Properties>"), 1);
    if large {
        text = text.replacen(
            "</Properties>",
            &format!("<!--{}--></Properties>", "x".repeat(9 * 1024 * 1024)),
            1,
        );
    }
    fs::write(path, text).unwrap();
}

#[test]
fn configuration_information_reaches_both_public_read_routes_without_writes() {
    let long_text = "Подробности &amp; &lt;&gt; &#13;&#xD;\n<![CDATA[<&>]]> ".repeat(5000);
    let extra = format!(
        r#"<UseManagedFormInOrdinaryApplication>true</UseManagedFormInOrdinaryApplication>
<UseOrdinaryFormInManagedApplication>false</UseOrdinaryFormInManagedApplication>
<BriefInformation><v8:item><v8:lang>ru</v8:lang><v8:content>Кратко</v8:content></v8:item><v8:item><v8:lang>en</v8:lang><v8:content>Brief</v8:content></v8:item></BriefInformation>
<DetailedInformation><v8:item><v8:lang>ru</v8:lang><v8:content>{long_text}</v8:content></v8:item></DetailedInformation>
<Copyright><v8:item><v8:lang>ru</v8:lang><v8:content>Автор</v8:content></v8:item></Copyright>
<VendorInformationAddress><v8:item><v8:lang>en</v8:lang><v8:content>https://example.org/vendor</v8:content></v8:item></VendorInformationAddress>
<ConfigurationInformationAddress><v8:item><v8:lang>en</v8:lang><v8:content>https://example.org/configuration</v8:content></v8:item></ConfigurationInformationAddress>"#
    );
    let mut previous = None;
    for large in [false, true] {
        let fixture = RealReaderFixture::new();
        set_information(&fixture, &extra, large);
        let before = fs::read(fixture.source.join("Configuration.xml")).unwrap();
        let service = fixture.view_service();
        let result = service.view(ViewRequest::new("main:Configuration").unwrap());
        assert!(result.ok, "{result:?}");
        let data = result.data.unwrap();
        let properties = &data["props"]["properties"];
        assert_eq!(properties["useManagedFormInOrdinaryApplication"], true);
        assert_eq!(properties["useOrdinaryFormInManagedApplication"], false);
        assert_eq!(
            properties["briefInformation"],
            json!([
            {"language":"ru","content":"Кратко"},{"language":"en","content":"Brief"}])
        );
        assert_eq!(
            properties["detailedInformation"][0]["content"],
            "Подробности & <> \r\r\n<&> ".repeat(5000)
        );
        assert_eq!(properties["copyright"][0]["content"], "Автор");
        assert_eq!(
            properties["vendorInformationAddress"][0]["content"],
            "https://example.org/vendor"
        );
        assert_eq!(
            properties["configurationInformationAddress"][0]["content"],
            "https://example.org/configuration"
        );
        assert_eq!(properties["defaultRunMode"], "ManagedApplication");
        if let Some(previous) = previous {
            assert_eq!(data, previous);
        }
        previous = Some(data);
        assert_eq!(
            fs::read(fixture.source.join("Configuration.xml")).unwrap(),
            before
        );
    }
}

#[test]
fn configuration_information_damage_is_local_and_absence_is_not_a_default() {
    for large in [false, true] {
        for (extra, damaged) in [
            ("", false),
            ("<UseManagedFormInOrdinaryApplication/>", true),
            ("<UseManagedFormInOrdinaryApplication>junk</UseManagedFormInOrdinaryApplication>", true),
            ("<UseManagedFormInOrdinaryApplication a=\"1\">true</UseManagedFormInOrdinaryApplication>", true),
            ("<UseManagedFormInOrdinaryApplication><value>true</value></UseManagedFormInOrdinaryApplication>", true),
            ("<UseManagedFormInOrdinaryApplication>true</UseManagedFormInOrdinaryApplication><UseManagedFormInOrdinaryApplication>false</UseManagedFormInOrdinaryApplication>", true),
            ("<UseManagedFormInOrdinaryApplication xmlns=\"foreign\">true</UseManagedFormInOrdinaryApplication>", true),
            (r#"<UseManagedFormInOrdinaryApplication xmlns="">true</UseManagedFormInOrdinaryApplication>"#, true),
            (r#"<BriefInformation xmlns=""><v8:item><v8:lang>ru</v8:lang><v8:content>A</v8:content></v8:item></BriefInformation>"#, true),
            ("<BriefInformation/>", false),
            ("<BriefInformation>plain text</BriefInformation>", true),
            ("<BriefInformation><v8:item><v8:lang>ru</v8:lang><v8:content>A</v8:content></v8:item><v8:item><v8:lang>ru</v8:lang><v8:content>B</v8:content></v8:item></BriefInformation>", true),
            ("<BriefInformation/><BriefInformation/>", true),
        ] {
            let fixture = RealReaderFixture::new();
            set_information(&fixture, &format!("{extra}<UseOrdinaryFormInManagedApplication>false</UseOrdinaryFormInManagedApplication><Copyright><v8:item><v8:lang>ru</v8:lang><v8:content/></v8:item></Copyright>"), large);
            let service = fixture.view_service();
            let result = service.view(ViewRequest::new("main:Configuration").unwrap());
            assert!(result.ok, "{extra}: {result:?}");
            let data = result.data.unwrap();
            let properties = &data["props"]["properties"];
            assert!(properties["useManagedFormInOrdinaryApplication"].is_null());
            assert!(properties["briefInformation"].is_null());
            assert_eq!(properties["useOrdinaryFormInManagedApplication"], false);
            assert_eq!(properties["copyright"], json!([{"language":"ru","content":""}]));
            assert_eq!(data["limits"].as_array().is_some_and(|v| !v.is_empty()), damaged, "{extra}: {data}");
        }
    }
}
