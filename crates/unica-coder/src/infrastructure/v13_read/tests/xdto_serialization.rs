use super::*;
const ROOT: &str = "main:XDTOPackage.EnterpriseData_1_17_3";
fn fixture() -> RealReaderFixture {
    let f = RealReaderFixture::new();
    let xml = fixture_text(
        "acceptance/workspace-xdto-serialization/src/XDTOPackages/Serialization/Ext/Package.bin",
    )
    .replace(
        "urn:unica:serialization",
        "http://v8.1c.ru/edi/edi_stnd/EnterpriseData/1.17.3",
    );
    write(
        &f.source
            .join("XDTOPackages/EnterpriseData_1_17_3/Ext/Package.bin"),
        &xml,
    );
    f
}
fn read(f: &RealReaderFixture, tail: &str) -> serde_json::Value {
    let at = if tail.is_empty() {
        ROOT.to_owned()
    } else {
        format!("{ROOT}.{tail}")
    };
    let r = f.view_service().view(ViewRequest::new(&at).unwrap());
    assert!(r.ok, "{at}: {:?}", refusal_codes(&r));
    r.data.unwrap()
}
#[test]
fn xdto_serialization_facts_survive_native_public_collection_and_deep_detail() {
    let f = fixture();
    let root = read(&f, "");
    assert_eq!(root["props"]["elementFormQualified"], false);
    assert_eq!(root["props"]["attributeFormQualified"], true);
    let rows = read(&f, "Type");
    let row = rows["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["title"] == "Probe")
        .unwrap();
    let detail = read(&f, "Type.Probe");
    for data in [row, &detail] {
        assert_eq!(data["props"]["open"], false);
        assert_eq!(data["props"]["mixed"], true);
    }
    let collection = read(&f, "Type.Probe.Property");
    let f2 = &collection["items"][0];
    let detail = read(&f, "Type.Probe.Property.F2RegId");
    for data in [f2, &detail] {
        assert_eq!(data["props"]["form"], "Element");
        assert_eq!(data["props"]["nillable"], false);
        assert_eq!(data["props"]["fixed"], false);
        assert_eq!(data["props"]["default"], "FIRSTSHIPMENT");
        assert_eq!(
            data["props"]["bounds"],
            json!({"lower":"0","upper":"-1","lowerExplicit":true,"upperExplicit":true,"unbounded":true})
        );
    }
    assert_eq!(
        read(&f, "Type.Probe.Property.Nested.Property.contentType")["props"]["form"],
        "Attribute"
    );
    assert_eq!(
        read(&f, "Type.Probe.Property.Nested.Property.__content")["props"]["form"],
        "Text"
    );
}

#[test]
fn xdto_serialization_empty_false_absent_and_damage_stay_distinct() {
    let f = fixture();
    let absent = read(&f, "Type.Probe.Property.Absent");
    for key in ["form", "nillable", "fixed", "default"] {
        assert!(absent["props"].get(key).is_none());
    }
    let empty = read(&f, "Type.Probe.Property.Empty");
    assert_eq!(empty["props"]["default"], "");
    assert_eq!(empty["props"]["nillable"], false);
    assert_eq!(empty["props"]["fixed"], true);
    assert_eq!(
        read(&f, "Type.Probe.Property.Whitespace")["props"]["default"],
        "  keep \t & \"  "
    );
    let broken = read(&f, "Type.Probe.Property.Broken");
    for key in ["form", "nillable", "fixed"] {
        assert!(broken["props"].get(key).is_none());
        assert!(broken["limits"]
            .as_array()
            .unwrap()
            .iter()
            .any(|l| l.as_str().unwrap().contains(key)));
    }
    assert_eq!(broken["props"]["default"], "keep");
    assert_eq!(broken["props"]["type"]["local"], "string");
    let rows = read(&f, "Type.Probe.Property");
    let row = rows["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r.get("at").is_none() && r.get("ref").is_some())
        .unwrap();
    assert!(row.get("nillable").is_none());
    assert!(row["limits"]
        .as_array()
        .unwrap()
        .iter()
        .any(|l| l.as_str().unwrap().contains("nillable")));
    assert_eq!(row["ref"]["local"], "Global");
    let foreign = read(&f, "Type.Probe.Property.Foreign");
    for key in ["form", "nillable", "default"] {
        assert!(foreign["props"].get(key).is_none());
    }
    assert!(foreign["limits"]
        .as_array()
        .unwrap()
        .iter()
        .any(|l| l.as_str().unwrap().contains("urn:foreign")));
    assert_eq!(
        read(&f, "Type.Probe.Property.Dual")["props"]["form"],
        "Element"
    );
    let opaque = read(&f, "Type.Probe.Property.Opaque");
    for key in ["open", "mixed"] {
        assert!(opaque["props"].get(key).is_none());
    }
    assert_eq!(
        read(&f, "Type.Probe.Property.Opaque.Property.Child")["props"]["form"],
        "Text"
    );
}

#[test]
fn xdto_serialization_root_and_object_absence_and_invalid_flags_keep_other_facts() {
    for case in ["absent", "invalid", "foreign", "wrong-owner"] {
        let f = fixture();
        let path = f
            .source
            .join("XDTOPackages/EnterpriseData_1_17_3/Ext/Package.bin");
        let original = fs::read_to_string(&path).unwrap();
        let xml = match case {
            "absent" => original
                .replace(" elementFormQualified=\"false\"", "")
                .replace(" attributeFormQualified=\"true\"", "")
                .replace(" open=\"false\"", "")
                .replace(" mixed=\"true\"", ""),
            "invalid" => original
                .replace(
                    "elementFormQualified=\"false\"",
                    "elementFormQualified=\"invalid\"",
                )
                .replace("open=\"false\"", "open=\"invalid\""),
            "foreign" => original.replace(
                "elementFormQualified=\"false\"",
                "xmlns:f=\"urn:foreign\" f:elementFormQualified=\"false\"",
            ),
            _ => original.replace(
                "<valueType name=\"String\"",
                "<valueType open=\"false\" mixed=\"true\" name=\"String\"",
            ),
        };
        write(&path, &xml);
        let root = read(&f, "");
        let probe = read(&f, "Type.Probe");
        if case == "absent" {
            for key in ["elementFormQualified", "attributeFormQualified"] {
                assert!(root["props"].get(key).is_none());
            }
            for key in ["open", "mixed"] {
                assert!(probe["props"].get(key).is_none());
            }
        }
        if case == "invalid" {
            assert!(root["props"].get("elementFormQualified").is_none());
            assert_eq!(root["props"]["attributeFormQualified"], true);
            assert!(root["limits"]
                .as_array()
                .unwrap()
                .iter()
                .any(|x| x.as_str().unwrap().contains("elementFormQualified")));
            assert!(probe["props"].get("open").is_none());
            assert_eq!(probe["props"]["mixed"], true);
            assert!(probe["limits"]
                .as_array()
                .unwrap()
                .iter()
                .any(|x| x.as_str().unwrap().contains("open")));
        }
        if case == "foreign" {
            assert!(root["props"].get("elementFormQualified").is_none());
            assert_eq!(root["props"]["attributeFormQualified"], true);
            assert!(root["limits"]
                .as_array()
                .unwrap()
                .iter()
                .any(|x| x.as_str().unwrap().contains("urn:foreign")));
        }
        if case == "wrong-owner" {
            let value = read(&f, "Type.String");
            assert_eq!(value["props"]["kind"], "valueType");
            for key in ["open", "mixed"] {
                assert!(value["props"].get(key).is_none());
            }
        }
        assert_eq!(probe["props"]["base"]["local"], "Base");
        assert_eq!(
            read(&f, "Type.Probe.Property.Nested.Property.__content")["props"]["form"],
            "Text"
        );
        assert_eq!(fs::read_to_string(&path).unwrap(), xml);
    }
}

#[test]
fn xdto_long_default_names_the_whole_property_and_keeps_sibling_facts() {
    let f = fixture();
    let path = f
        .source
        .join("XDTOPackages/EnterpriseData_1_17_3/Ext/Package.bin");
    let value = "я".repeat(1025);
    let xml = fs::read_to_string(&path).unwrap().replacen(
        "default=\"FIRSTSHIPMENT\"",
        &format!("default=\"{value}\""),
        1,
    );
    write(&path, &xml);
    let detail = read(&f, "Type.Probe.Property.F2RegId");
    assert!(detail["props"].get("default").is_none());
    assert_eq!(detail["props"]["fixed"], false);
    assert_eq!(detail["props"]["form"], "Element");
    assert!(detail["limits"].as_array().unwrap().iter().any(|x| x
        .as_str()
        .unwrap()
        .contains("Property.F2RegId.Props.default")));
    let rows = read(&f, "Type.Probe.Property");
    assert!(rows["items"][0]["props"].get("default").is_none());
    assert!(rows["items"][0]["limits"]
        .as_array()
        .unwrap()
        .iter()
        .any(|x| x
            .as_str()
            .unwrap()
            .contains("Property.F2RegId.Props.default")));
    assert_eq!(fs::read_to_string(&path).unwrap(), xml);
}
