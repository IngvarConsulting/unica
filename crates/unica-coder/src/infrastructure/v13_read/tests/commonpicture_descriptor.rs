use super::*;
const OWNER: &str = "main:CommonPicture.Probe";
fn install(f: &RealReaderFixture, flag: &str, picture: &str) -> PathBuf {
    let config = f.source.join("Configuration.xml");
    write(
        &config,
        &fs::read_to_string(&config).unwrap().replace(
            "</ChildObjects>",
            "<CommonPicture>Probe</CommonPicture></ChildObjects>",
        ),
    );
    write(
        &f.source.join("CommonPictures/Probe.xml"),
        &format!(
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><CommonPicture uuid="16180000-0000-4000-8000-000000000001"><Properties><Name>Probe</Name>{flag}<Vendor>hidden</Vendor></Properties></CommonPicture></MetaDataObject>"#
        ),
    );
    let path = f.source.join("CommonPictures/Probe/Ext/Picture.xml");
    write(
        &path,
        &format!(
            r#"<ExtPicture xmlns="http://v8.1c.ru/8.3/xcf/extrnprops" xmlns:r="http://v8.1c.ru/8.3/xcf/readable" version="2.20"><Picture>{picture}</Picture></ExtPicture>"#
        ),
    );
    path
}
#[test]
fn commonpicture_descriptor_preserves_explicit_xml_facts_without_payload() {
    for (reference, flag) in [
        ("Picture.png", false),
        ("Picture.zip", true),
        ("../opaque.custom", false),
    ] {
        let f = RealReaderFixture::new();
        let path = install(
            &f,
            &format!("<AvailabilityForAppearance>{flag}</AvailabilityForAppearance>"),
            &format!("<r:Abs>{reference}</r:Abs><r:LoadTransparent>{flag}</r:LoadTransparent>"),
        );
        let before = fs::read(&path).unwrap();
        let result = f.view_service().view(ViewRequest::new(OWNER).unwrap());
        assert!(result.ok, "{result:?}");
        let node = result.data.unwrap();
        assert_eq!(node["props"]["AvailabilityForAppearance"], flag);
        assert_eq!(
            node["props"]["Picture"],
            json!({"Abs":reference,"LoadTransparent":flag})
        );
        assert!(node["props"].get("Vendor").is_none());
        assert!(node.get("limits").is_none(), "{node}");
        assert_eq!(fs::read(path).unwrap(), before);
    }
}

#[test]
fn commonpicture_descriptor_keeps_independent_fields_on_local_damage() {
    for (raw, expected, damaged) in [
        ("<r:Abs>Picture.png</r:Abs>", json!({"Abs":"Picture.png"}), false),
        ("<r:Abs/><r:LoadTransparent>false</r:LoadTransparent>", json!({"LoadTransparent":false}), true),
        ("<r:LoadTransparent>true</r:LoadTransparent>", json!({"LoadTransparent":true}), true),
        ("<r:Abs>one</r:Abs><r:Abs>two</r:Abs><r:LoadTransparent>false</r:LoadTransparent>", json!({"LoadTransparent":false}), true),
        ("<r:Abs>Picture.zip</r:Abs><r:LoadTransparent>unknown</r:LoadTransparent>", json!({"Abs":"Picture.zip"}), true),
        ("<r:Abs>Picture.zip</r:Abs><r:LoadTransparent>false</r:LoadTransparent><r:LoadTransparent>true</r:LoadTransparent>", json!({"Abs":"Picture.zip"}), true),
        ("<r:Abs xmlns:r=\"urn:foreign\">hidden</r:Abs><r:LoadTransparent>false</r:LoadTransparent>", json!({"LoadTransparent":false}), true),
        ("<r:Abs><r:Nested/></r:Abs><r:LoadTransparent>false</r:LoadTransparent>", json!({"LoadTransparent":false}), true),
        ("<r:Abs vendor=\"bad\">hidden</r:Abs><r:LoadTransparent>false</r:LoadTransparent>", json!({"LoadTransparent":false}), true),
        ("<r:Abs>Picture.png</r:Abs><Vendor>hidden</Vendor>", json!({"Abs":"Picture.png"}), true),
    ] {
        let f = RealReaderFixture::new();
        install(&f, "<AvailabilityForAppearance>false</AvailabilityForAppearance>", raw);
        let result = f.view_service().view(ViewRequest::new(OWNER).unwrap());
        assert!(result.ok);
        let node = result.data.unwrap();
        assert_eq!(node["props"]["AvailabilityForAppearance"], false);
        assert_eq!(node["props"]["Picture"], expected, "{raw}: {node}");
        assert_eq!(node.get("limits").is_some(), damaged, "{raw}: {node}");
    }
}

#[test]
fn commonpicture_descriptor_retains_identity_and_root_flag_without_companion() {
    for variant in ["missing", "malformed", "foreign", "duplicate", "directory"] {
        let f = RealReaderFixture::new();
        let path = install(
            &f,
            "<AvailabilityForAppearance>false</AvailabilityForAppearance>",
            "<r:Abs>Picture.png</r:Abs>",
        );
        match variant {
            "missing" => fs::remove_file(&path).unwrap(),
            "malformed" => write(&path, "<ExtPicture>"),
            "foreign" => write(
                &path,
                "<ExtPicture xmlns=\"urn:foreign\"><Picture/></ExtPicture>",
            ),
            "duplicate" => write(
                &path,
                &fs::read_to_string(&path)
                    .unwrap()
                    .replace("</ExtPicture>", "<Picture/></ExtPicture>"),
            ),
            "directory" => {
                fs::remove_file(&path).unwrap();
                fs::create_dir(&path).unwrap();
            }
            _ => unreachable!(),
        }
        let result = f.view_service().view(ViewRequest::new(OWNER).unwrap());
        assert!(result.ok, "{variant}: {result:?}");
        let node = result.data.unwrap();
        assert_eq!(node["at"], OWNER);
        assert_eq!(node["props"]["AvailabilityForAppearance"], false);
        assert!(node["props"].get("Picture").is_none(), "{node}");
        assert!(node["limits"].to_string().contains("Picture"), "{node}");
    }
}

#[test]
fn commonpicture_descriptor_root_damage_does_not_hide_companion() {
    for flag in ["", "<AvailabilityForAppearance>bad</AvailabilityForAppearance>", "<AvailabilityForAppearance>true</AvailabilityForAppearance><AvailabilityForAppearance>false</AvailabilityForAppearance>", "<AvailabilityForAppearance xmlns=\"urn:foreign\">false</AvailabilityForAppearance>"] {
        let f = RealReaderFixture::new();
        install(&f, flag, "<r:Abs>Picture.zip</r:Abs><r:LoadTransparent>false</r:LoadTransparent>");
        let result = f.view_service().view(ViewRequest::new(OWNER).unwrap());
        assert!(result.ok);
        let node = result.data.unwrap();
        assert!(node["props"].get("AvailabilityForAppearance").is_none());
        assert_eq!(node["props"]["Picture"], json!({"Abs":"Picture.zip","LoadTransparent":false}));
        assert_eq!(node.get("limits").is_some(), !flag.is_empty(), "{node}");
    }
}

#[test]
fn commonpicture_descriptor_whole_picture_retains_the_compound_reference() {
    let f = RealReaderFixture::new();
    let reference = "x".repeat(2050);
    install(
        &f,
        "",
        &format!("<r:Abs>{reference}</r:Abs><r:LoadTransparent>false</r:LoadTransparent>"),
    );
    let service = f.view_service();
    let result = service.view(ViewRequest::new(OWNER).unwrap());
    let node = result.data.unwrap();
    assert!(node["props"].get("Picture").is_none());
    assert!(node["limits"].to_string().contains(".Props.Picture"));
    let whole = crate::infrastructure::v13_read_projection::with_whole_prop("Picture", || {
        service.view(ViewRequest::new(OWNER).unwrap())
    });
    assert!(whole.ok);
    assert_eq!(
        whole.data.unwrap()["props"]["Picture"],
        json!({"Abs":reference,"LoadTransparent":false})
    );
}

#[test]
fn commonpicture_descriptor_refreshes_companion_under_the_same_owner_identity() {
    let f = RealReaderFixture::new();
    let path = install(
        &f,
        "<AvailabilityForAppearance>false</AvailabilityForAppearance>",
        "<r:Abs>Picture.png</r:Abs><r:LoadTransparent>false</r:LoadTransparent>",
    );
    let service = f.view_service();
    let first = service.view(ViewRequest::new(OWNER).unwrap());
    assert_eq!(
        first.data.unwrap()["props"]["Picture"]["LoadTransparent"],
        false
    );
    write(
        &path,
        &fs::read_to_string(&path)
            .unwrap()
            .replace("<r:LoadTransparent>false", "<r:LoadTransparent>true"),
    );
    let second = service.view(ViewRequest::new(OWNER).unwrap());
    assert_eq!(
        second.data.unwrap()["props"]["Picture"]["LoadTransparent"],
        true
    );
}

#[test]
fn commonpicture_descriptor_observes_companion_recovery_without_recreating_service() {
    let f = RealReaderFixture::new();
    let path = install(
        &f,
        "<AvailabilityForAppearance>false</AvailabilityForAppearance>",
        "<r:Abs>Picture.png</r:Abs><r:LoadTransparent>false</r:LoadTransparent>",
    );
    let original = fs::read_to_string(&path).unwrap();
    let service = f.view_service();
    for content in [
        None,
        Some(original.as_str()),
        Some("<ExtPicture>"),
        Some(original.as_str()),
    ] {
        match content {
            Some(text) => write(&path, text),
            None => fs::remove_file(&path).unwrap(),
        }
        let result = service.view(ViewRequest::new(OWNER).unwrap());
        assert!(result.ok);
        let node = result.data.unwrap();
        assert_eq!(node["props"]["AvailabilityForAppearance"], false);
        assert_eq!(
            node["props"].get("Picture").is_some(),
            content == Some(original.as_str()),
            "{node}"
        );
    }
}
