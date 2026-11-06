use super::info::parse_xdto_info_bytes;
use serde_json::json;
use unica_address::source_target::{MetadataAddress, PLATFORM_XML_8_3_27_FORMAT_2_20};
const PACKAGE: &[u8] = include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"),"/../../tests/fixtures/acceptance/workspace-xdto-serialization/src/XDTOPackages/Serialization/Ext/Package.bin"));
const DESCRIPTOR: &[u8] = include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"),"/../../tests/fixtures/acceptance/workspace-xdto-serialization/src/XDTOPackages/Serialization.xml"));
fn info(type_name: Option<&str>) -> serde_json::Value {
    let at = MetadataAddress::parse(PLATFORM_XML_8_3_27_FORMAT_2_20, "XDTOPackage.Serialization")
        .unwrap();
    serde_json::to_value(
        parse_xdto_info_bytes(PACKAGE, DESCRIPTOR, "main", &at, type_name).unwrap(),
    )
    .unwrap()
}
#[test]
fn xdto_serialization_values_are_typed_observations_not_defaults() {
    let root = info(None);
    assert_eq!(root["elementFormQualified"], false);
    assert_eq!(root["attributeFormQualified"], true);
    assert_eq!(root["globalProperties"][0]["form"], "Attribute");
    assert_eq!(root["globalProperties"][0]["default"], "");
    let detail = info(Some("Probe"));
    let t = &detail["typeDetail"];
    assert_eq!(t["open"], false);
    assert_eq!(t["mixed"], true);
    let p = &t["properties"][0];
    assert_eq!(p["form"], "Element");
    assert_eq!(p["nillable"], false);
    assert_eq!(p["fixed"], false);
    assert_eq!(p["default"], "FIRSTSHIPMENT");
    assert_eq!(
        p["bounds"],
        json!({"lower":"0","upper":"-1","lowerExplicit":true,"upperExplicit":true,"unbounded":true})
    );
    assert_eq!(p["type"]["namespace"], "http://www.w3.org/2001/XMLSchema");
    assert_eq!(t["base"]["namespace"], "urn:unica:serialization");
    for field in ["form", "nillable", "fixed", "default"] {
        assert!(t["properties"][1].get(field).is_none());
    }
    assert_eq!(t["properties"][2]["nillable"], false);
    assert_eq!(t["properties"][2]["fixed"], true);
    assert_eq!(t["properties"][2]["default"], "");
    assert_eq!(
        t["properties"][4]["typeDefs"][0]["properties"][0]["form"],
        "Attribute"
    );
    assert_eq!(
        t["properties"][4]["typeDefs"][0]["properties"][1]["form"],
        "Text"
    );
}

#[test]
fn xdto_serialization_boolean_lexemes_and_owner_slots_share_one_observer() {
    let at = MetadataAddress::parse(PLATFORM_XML_8_3_27_FORMAT_2_20, "XDTOPackage.Serialization")
        .unwrap();
    for (owner, name, old) in [
        ("package", "elementFormQualified", "false"),
        ("package", "attributeFormQualified", "true"),
        ("objectType", "open", "false"),
        ("objectType", "mixed", "true"),
        ("property", "nillable", "false"),
        ("property", "fixed", "false"),
    ] {
        for (literal, wanted) in [
            ("true", Some(true)),
            ("false", Some(false)),
            ("1", Some(true)),
            ("0", Some(false)),
            (" &#x9;true ", Some(true)),
            ("", None),
            ("False", None),
            ("yes", None),
            ("&#xA0;false", None),
        ] {
            let source = std::str::from_utf8(PACKAGE).unwrap();
            let needle = format!("{name}=\"{old}\"");
            let start = if owner == "property" {
                source.find("name=\"F2RegId\"").unwrap()
            } else {
                0
            };
            let offset = source[start..].find(&needle).unwrap() + start;
            let mut xml = source.to_owned();
            xml.replace_range(
                offset..offset + needle.len(),
                &format!("{name}=\"{literal}\""),
            );
            let info = serde_json::to_value(
                parse_xdto_info_bytes(xml.as_bytes(), DESCRIPTOR, "main", &at, Some("Probe"))
                    .unwrap(),
            )
            .unwrap();
            let target = match owner {
                "package" => &info,
                "objectType" => &info["typeDetail"],
                _ => &info["typeDetail"]["properties"][0],
            };
            assert_eq!(
                target.get(name),
                wanted.map(serde_json::Value::Bool).as_ref(),
                "{owner}/{name}/{literal}"
            );
            if wanted.is_none() {
                assert!(target["serializationLimits"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|x| x.as_str().unwrap().contains(name)));
            }
        }
    }
}

#[test]
fn xdto_serialization_reader_expansion_does_not_make_flagged_property_an_exact_writer_repeat() {
    use crate::writer::plan;
    let xml = r#"<package xmlns="http://v8.1c.ru/8.1/xdto" xmlns:xs="http://www.w3.org/2001/XMLSchema" targetNamespace="urn:test">
 <objectType name="Owner"><property name="Value" type="xs:string" form="Attribute" nillable="false" fixed="false" default="keep"/></objectType>
</package>"#;
    let args = serde_json::from_value(
        json!({"typeName":"Owner","property":{"name":"Value","type":"xs:string"}}),
    )
    .unwrap();
    let p = plan(xml, &args, "add-property").unwrap();
    assert!(p.blocks());
    assert_eq!(p.after, xml);
    let p = crate::writer::plan_typed(
        xml,
        crate::writer::TypedWriterOperation::RenameProperty {
            type_name: "Owner",
            name: "Value",
            property_path: None,
            new_name: "Renamed",
        },
    )
    .unwrap();
    assert!(!p.blocks());
    assert_eq!(p.after, xml.replace("name=\"Value\"", "name=\"Renamed\""));
    let operation=unica_xdto::operations::XdtoOperation::AddProperty.decode(&json!({"at":"main:XDTOPackage.Test.Type.Owner","values":{"property":{"name":"Value","type":"xs:string"}}}),"args","main").unwrap();
    let unica_xdto::operations::XdtoPlanOperation::AddProperty(args) = operation else {
        panic!("property operation")
    };
    let p = crate::writer::plan_typed(
        xml,
        crate::writer::TypedWriterOperation::AddPropertyTree {
            type_name: "Owner",
            property: &args.property,
            property_path: None,
        },
    )
    .unwrap();
    assert!(p.blocks());
    assert_eq!(p.after, xml);
}

#[test]
fn xdto_read_serialization_fields_are_not_mutation_operands_and_old_base_patch_stays_local() {
    use unica_xdto::operations::XdtoOperation;
    for key in ["form", "nillable", "fixed", "default"] {
        let mut args = json!({"at":"main:XDTOPackage.Test.Type.Owner","values":{"property":{"name":"Value","type":"xs:string"}}});
        args["values"]["property"][key] = json!(false);
        assert!(
            XdtoOperation::AddProperty
                .decode(&args, "args", "main")
                .is_err(),
            "{key}"
        );
    }
    for key in ["open", "mixed"] {
        let mut args =
            json!({"at":"main:XDTOPackage.Test.Type.Owner","values":{"base":"tns:Base"}});
        args["values"][key] = json!(false);
        assert!(
            XdtoOperation::SetType
                .decode(&args, "args", "main")
                .is_err(),
            "{key}"
        );
    }
    let xml = r#"<package xmlns="http://v8.1c.ru/8.1/xdto" xmlns:tns="urn:test" targetNamespace="urn:test" elementFormQualified="false" attributeFormQualified="true">
 <objectType name="Base"/>
 <objectType name="Owner" open="false" mixed="true"/>
</package>"#;
    let p = crate::writer::plan_typed(
        xml,
        crate::writer::TypedWriterOperation::SetTypeBase {
            name: "Owner",
            base: "tns:Base",
        },
    )
    .unwrap();
    assert!(!p.blocks());
    assert_eq!(
        p.after,
        xml.replace("name=\"Owner\"", "name=\"Owner\" base=\"tns:Base\"")
    );
}
