use super::*;

const NUMERATOR: &str = concat!(
    "\u{feff}<MetaDataObject xmlns=\"http://v8.1c.ru/8.3/MDClasses\" version=\"2.20\">\r\n",
    "<DocumentNumerator uuid=\"16760000-0000-4000-8000-000000000001\"><Properties>",
    "<Name>Sales</Name><Synonym/><Comment>retained</Comment>",
    "<NumberType>String</NumberType><NumberLength>11</NumberLength>",
    "<NumberAllowedLength>Fixed</NumberAllowedLength><NumberPeriodicity>Year</NumberPeriodicity>",
    "<CheckUnique>true</CheckUnique></Properties></DocumentNumerator>\r\n</MetaDataObject>\r\n"
);

fn fixture() -> MetadataFixture {
    let configuration = RU_CONFIGURATION_XML.replace("</ChildObjects>",
        "<DocumentNumerator>Sales</DocumentNumerator></ChildObjects>");
    let order = ORDER_XML.replace("<BasedOn/>", "<BasedOn/><Numerator>DocumentNumerator.Sales</Numerator>");
    MetadataFixture::with_files(&[
        ("Configuration.xml", &configuration),
        ("Languages/Русский.xml", &language_xml("Русский", "ru")),
        ("Documents/Order.xml", &order),
        ("DocumentNumerators/Sales.xml", NUMERATOR),
    ])
}

#[test]
fn numerator_properties_stage_only_the_requested_scalars_and_preserve_references() {
    let fixture = fixture();
    let before = unica_workspace::test_support::tree_snapshot(fixture.binding.source_root());
    let staged = plan_all(&fixture, &[("props.set", json!({
        "at":"main:DocumentNumerator.Sales", "values":{
            "NumberType":"Number", "NumberLength":9, "NumberAllowedLength":"Variable",
            "NumberPeriodicity":"Month", "CheckUnique":false
        }
    }))]).unwrap();
    let texts = staged_texts(&staged);
    assert_eq!(texts.len(), 1);
    let expected = NUMERATOR.replace("<NumberType>String</NumberType>", "<NumberType>Number</NumberType>")
        .replace("<NumberLength>11</NumberLength>", "<NumberLength>9</NumberLength>")
        .replace("<NumberAllowedLength>Fixed</NumberAllowedLength>", "<NumberAllowedLength>Variable</NumberAllowedLength>")
        .replace("<NumberPeriodicity>Year</NumberPeriodicity>", "<NumberPeriodicity>Month</NumberPeriodicity>")
        .replace("<CheckUnique>true</CheckUnique>", "<CheckUnique>false</CheckUnique>");
    assert_eq!(texts[Path::new("DocumentNumerators/Sales.xml")], expected);
    assert_eq!(unica_workspace::test_support::tree_snapshot(fixture.binding.source_root()), before);
}

#[test]
fn equivalent_numerator_properties_do_not_stage_a_write() {
    let staged = plan_all(&fixture(), &[("props.set", json!({
        "at":"main:DocumentNumerator.Sales", "values":{"CheckUnique":true,"NumberLength":11}
    }))]).unwrap();
    assert!(staged.planned_changes().is_empty());
}

#[test]
fn numerator_properties_require_a_registered_configuration_owner() {
    for registration in [
        "",
        "<DocumentNumerator>Other</DocumentNumerator>",
        "<DocumentNumerator xmlns=\"urn:foreign\">Sales</DocumentNumerator>",
        "<Subsystem><DocumentNumerator>Sales</DocumentNumerator></Subsystem>",
    ] {
        for check_unique in [false, true] {
            let fixture = fixture();
            let root = fixture.binding.source_root().join("Configuration.xml");
            let original = std::fs::read_to_string(&root).unwrap();
            std::fs::write(
                root,
                original.replace("<DocumentNumerator>Sales</DocumentNumerator>", registration),
            )
            .unwrap();
            let before = unica_workspace::test_support::tree_snapshot(fixture.binding.source_root());
            let error = plan_all(&fixture, &[("props.set", json!({
                "at":"main:DocumentNumerator.Sales", "values":{"CheckUnique":check_unique}
            }))])
            .expect_err("an orphan numerator must refuse both a change and an equivalent no-op");
            assert_eq!(error.kind(), ApplyPlanErrorKind::InvalidSource);
            assert_eq!(error.path(), Some("ops[0].args.at"));
            assert!(error.to_string().contains("not registered"), "{error:?}");
            assert_eq!(
                unica_workspace::test_support::tree_snapshot(fixture.binding.source_root()),
                before,
                "a registration refusal writes no source files"
            );
        }
    }
}

#[test]
fn each_numerator_property_changes_only_its_own_scalar() {
    for (field, value, old, new) in [
        ("NumberType", json!("Number"), "String", "Number"),
        ("NumberLength", json!(0), "11", "0"),
        ("NumberAllowedLength", json!("Variable"), "Fixed", "Variable"),
        ("NumberPeriodicity", json!("Day"), "Year", "Day"),
        ("CheckUnique", json!(false), "true", "false"),
    ] {
        let staged = plan_all(&fixture(), &[("props.set", json!({
            "at":"main:DocumentNumerator.Sales", "values":{(field):value}
        }))]).unwrap();
        let texts = staged_texts(&staged);
        assert_eq!(texts.len(), 1);
        assert_eq!(texts[Path::new("DocumentNumerators/Sales.xml")],
            NUMERATOR.replace(&format!("<{field}>{old}</{field}>"), &format!("<{field}>{new}</{field}>")));
    }
}
