use super::*;

#[test]
fn xdto_explicit_bounds_keep_their_xml_names_and_all_observed_facts() {
    let fixture = RealReaderFixture::new();
    let path = fixture
        .source
        .join("XDTOPackages/EnterpriseData_1_17_3/Ext/Package.bin");
    let xml = fs::read_to_string(&path).unwrap().replace("</package>", r#"<objectType name="BoundsProbe">
<property name="Absent" type="xs:string"/>
<property name="Explicit" type="xs:string" lowerBound="1" upperBound="1"/>
<property name="Huge" type="xs:string" lowerBound="0" upperBound="999999999999999999999999999999999999"/>
</objectType></package>"#);
    fs::write(&path, &xml).unwrap();
    let service = fixture.view_service();
    for (name, explicit) in [("Absent", false), ("Explicit", true)] {
        let at = format!("main:XDTOPackage.EnterpriseData_1_17_3.Type.BoundsProbe.Property.{name}");
        let data = service.view(ViewRequest::new(&at).unwrap()).data.unwrap();
        assert_eq!(
            data["props"]["bounds"],
            json!({"lower":"1","upper":"1","lowerExplicit":explicit,"upperExplicit":explicit,"unbounded":false})
        );
        for key in ["lowerBound", "upperBound"] {
            assert_eq!(data["props"].get(key), explicit.then_some(&json!("1")));
        }
    }
    let at = "main:XDTOPackage.EnterpriseData_1_17_3.Type.BoundsProbe.Property.Huge";
    let data = service.view(ViewRequest::new(at).unwrap()).data.unwrap();
    assert_eq!(
        data["props"]["upperBound"],
        "999999999999999999999999999999999999"
    );
    assert_eq!(
        data["props"]["bounds"]["upper"],
        data["props"]["upperBound"]
    );
    assert_eq!(fs::read_to_string(&path).unwrap(), xml);
}
