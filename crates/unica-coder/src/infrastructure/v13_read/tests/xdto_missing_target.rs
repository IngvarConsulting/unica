use super::{refusal_codes, RealReaderFixture, ViewRequest};
use std::fs;

const PACKAGE_AT: &str = "main:XDTOPackage.EnterpriseData_1_17_3";
const PACKAGE_PATH: &str = "XDTOPackages/EnterpriseData_1_17_3/Ext/Package.bin";

#[test]
fn missing_xdto_type_and_its_descendant_are_not_found() {
    let fixture = RealReaderFixture::new();
    let path = fixture.source.join(PACKAGE_PATH);
    let before = fs::read(&path).unwrap();
    let service = fixture.view_service();
    for suffix in ["Type.AbsentType", "Type.AbsentType.Property.AbsentProperty"] {
        let at = format!("{PACKAGE_AT}.{suffix}");
        let result = service.view(ViewRequest::new(&at).unwrap());
        assert!(!result.ok);
        assert_eq!(result.at.as_deref(), Some(at.as_str()));
        assert_eq!(refusal_codes(&result), vec!["not_found"]);
        assert_eq!(result.diagnostics[0]["outcome"], "fixCall");
        assert!(result.diagnostics[0].get("detailCode").is_none());
        assert!(result.data.is_none());
    }
    assert_eq!(fs::read(path).unwrap(), before);
}

#[test]
fn missing_xdto_type_does_not_hide_malformed_source() {
    let fixture = RealReaderFixture::new();
    fs::write(fixture.source.join(PACKAGE_PATH), b"<package").unwrap();
    let service = fixture.view_service();
    let result = service.view(ViewRequest::new(&format!("{PACKAGE_AT}.Type.AbsentType")).unwrap());
    assert_eq!(refusal_codes(&result), vec!["provider_unavailable"]);
    assert_eq!(result.diagnostics[0]["detailCode"], "source_unreadable");
    assert!(result.data.is_none());
}

#[test]
fn missing_xdto_type_does_not_hide_namespace_mismatch() {
    let fixture = RealReaderFixture::new();
    let path = fixture.source.join(PACKAGE_PATH);
    let before = fs::read_to_string(&path).unwrap();
    let after = before.replacen(
        "targetNamespace=\"http://v8.1c.ru/edi/edi_stnd/EnterpriseData/1.17.3\"",
        "targetNamespace=\"urn:other\"",
        1,
    );
    assert_ne!(before, after);
    fs::write(path, after).unwrap();
    let service = fixture.view_service();
    let result = service.view(ViewRequest::new(&format!("{PACKAGE_AT}.Type.AbsentType")).unwrap());
    assert_eq!(refusal_codes(&result), vec!["provider_unavailable"]);
    assert_eq!(result.diagnostics[0]["detailCode"], "source_unreadable");
    assert!(result.data.is_none());
}

#[test]
fn ambiguous_xdto_type_is_unreadable_and_package_is_still_readable() {
    let fixture = RealReaderFixture::new();
    let path = fixture.source.join(PACKAGE_PATH);
    let before = fs::read_to_string(&path).unwrap();
    let after = before.replace(
        "</package>",
        "<objectType name=\"СоставнойЛюбойОбъект\"/></package>",
    );
    assert_ne!(before, after);
    fs::write(path, after).unwrap();
    let service = fixture.view_service();
    let result =
        service.view(ViewRequest::new(&format!("{PACKAGE_AT}.Type.СоставнойЛюбойОбъект")).unwrap());
    assert_eq!(refusal_codes(&result), vec!["provider_unavailable"]);
    assert_eq!(result.diagnostics[0]["detailCode"], "source_unreadable");
    let package = service.view(ViewRequest::new(PACKAGE_AT).unwrap());
    assert!(package.ok, "{:?}", package.diagnostics);
}

#[test]
fn existing_xdto_type_and_absent_property_keep_their_results() {
    let fixture = RealReaderFixture::new();
    let service = fixture.view_service();
    let at = format!("{PACKAGE_AT}.Type.EnterpriseData");
    let existing = service.view(ViewRequest::new(&at).unwrap());
    assert!(existing.ok, "{:?}", existing.diagnostics);
    let missing = service.view(ViewRequest::new(&format!("{at}.Property.AbsentProperty")).unwrap());
    assert_eq!(refusal_codes(&missing), vec!["not_found"]);
}
