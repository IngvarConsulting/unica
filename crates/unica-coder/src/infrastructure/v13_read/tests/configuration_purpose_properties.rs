use super::*;

fn purpose_configuration_xml() -> String {
    fixture_text("acceptance/workspace-configuration-purpose/src/Configuration.xml")
}

fn read_purpose_configuration(
    xml: &str,
    line: unica_source_contract::format_identity::PlatformLine,
) -> unica_execution_contract::invocation::DomainResult {
    let copy = tempfile::tempdir().unwrap();
    let source = fs::canonicalize(copy.path()).unwrap();
    fs::write(source.join("Configuration.xml"), xml).unwrap();
    let before = fs::read(source.join("Configuration.xml")).unwrap();
    let cancellation = CancellationToken::new();
    let source_root = Arc::new(unica_workspace::read::open_source_directory(&source).unwrap());
    let authority = LogicalViewReadAuthority::new(
        &cancellation,
        "main",
        "actor-purpose",
        SourceSetKind::Configuration,
        source_root,
        PlatformProfile::for_line(line).unwrap(),
    );
    let service = ViewService::new(authority, ViewCursorStore::default());
    let result = service.view(ViewRequest::new("main:Configuration").unwrap());
    assert_eq!(fs::read(source.join("Configuration.xml")).unwrap(), before);
    result
}

fn view_purpose_configuration(
    xml: &str,
    line: unica_source_contract::format_identity::PlatformLine,
) -> serde_json::Value {
    let result = read_purpose_configuration(xml, line);
    assert!(result.ok, "{}: {:?}", result.summary, result.diagnostics);
    result.data.unwrap()
}

fn assert_purpose_values(data: &serde_json::Value) {
    let properties = &data["props"]["properties"];
    assert_eq!(
        properties["usePurposes"],
        json!(["MobilePlatformApplication", "PlatformApplication"])
    );
    assert_eq!(
        properties["defaultRoles"],
        json!(["main:Role.Operator", "main:Role.Administrator"])
    );
    assert_eq!(
        properties["usedMobileApplicationFunctionalities"],
        json!(["Biometrics", "OSBackup"])
    );
    assert_eq!(properties["includeHelpInContents"], false);
}

/// XML values survive the codec, typed model and public root projection.
/// rule: INV.SOURCE.METADATA-OBJECT-FACTS
/// rule: INV.SOURCE.PROFILE-CAPABILITIES
#[test]
fn configuration_purpose_values_are_read_from_xml_in_each_supported_line() {
    use unica_source_contract::format_identity::PlatformLine;
    for (format, line) in [
        ("2.20", PlatformLine::new(8, 3, 27)),
        ("2.21", PlatformLine::new(8, 5, 1)),
        ("2.22", PlatformLine::new(8, 5, 4)),
    ] {
        let xml = purpose_configuration_xml()
            .replace("version=\"2.20\"", &format!("version=\"{format}\""));
        assert_purpose_values(&view_purpose_configuration(&xml, line));
    }
}

/// Streaming retains the same lists, false flag and qualified role references.
/// rule: INV.SOURCE.LARGE-CONFIGURATION-ROOT-VIEW
#[test]
fn medium_large_configuration_preserves_purpose_values() {
    let xml = purpose_configuration_xml();
    let line = unica_source_contract::format_identity::PlatformLine::new(8, 3, 27);
    let ordinary = view_purpose_configuration(&xml, line);
    let large = xml.replace(
        "</MetaDataObject>",
        &format!("<!--{}--></MetaDataObject>", "x".repeat(8 * 1024 * 1024)),
    );
    let streamed = view_purpose_configuration(&large, line);
    assert_purpose_values(&streamed);
    assert_eq!(streamed, ordinary);
}

/// Only mobile capabilities have platform defaults for absent values.
/// rule: INV.SOURCE.METADATA-OBJECT-FACTS
#[test]
fn configuration_purpose_properties_distinguish_empty_and_absent_values() {
    let line = unica_source_contract::format_identity::PlatformLine::new(8, 3, 27);
    for (values, expected) in [
        (
            "<UsePurposes/><DefaultRoles/><UsedMobileApplicationFunctionalities/><IncludeHelpInContents>true</IncludeHelpInContents>",
            json!([[], [], ["Biometrics", "OSBackup"], true]),
        ),
        ("", json!([null, null, ["Biometrics", "OSBackup"], null])),
    ] {
        let xml = format!(
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Configuration uuid="30930000-0000-4000-8000-000000000001"><Properties><Name>EmptyPurpose</Name>{values}</Properties><ChildObjects/></Configuration></MetaDataObject>"#
        );
        for padding in [0, 8 * 1024 * 1024] {
        let xml = xml.replace("</MetaDataObject>", &format!("<!--{}--></MetaDataObject>", "x".repeat(padding)));
        let data = view_purpose_configuration(&xml, line);
        let props = data["props"]["properties"].as_object().unwrap();
        for (index, key) in [
            "usePurposes",
            "defaultRoles",
            "usedMobileApplicationFunctionalities",
            "includeHelpInContents",
        ]
        .iter()
        .enumerate()
        {
            assert_eq!(props.get(*key), Some(&expected[index]), "{key}");
        }
        }
    }
}

/// A damaged fact must not become a successful false flag or empty list.
/// rule: INV.SOURCE.METADATA-OBJECT-FACTS
#[test]
fn medium_configuration_purpose_damage_is_visible_in_both_readers() {
    let line = unica_source_contract::format_identity::PlatformLine::new(8, 3, 27);
    for (from, to) in [
        (
            "<IncludeHelpInContents>false</IncludeHelpInContents>",
            "<IncludeHelpInContents>oops</IncludeHelpInContents>",
        ),
        ("<app:use>false</app:use>", ""),
        ("<app:use>false</app:use>", "<app:use>oops</app:use>"),
        (
            "<app:functionality><app:functionality>Location",
            "<app:functionality>unexpected<app:functionality>Location",
        ),
        ("Role.Operator", "Catalog.Operator"),
        ("<UsePurposes>", "<UsePurposes/><UsePurposes>"),
        (
            "<v8:Value xsi:type",
            "<v8:Value xmlns:v8=\"urn:foreign\" xsi:type",
        ),
        (
            "<xr:Item xsi:type",
            "<xr:Item xmlns:xr=\"urn:foreign\" xsi:type",
        ),
    ] {
        let xml = purpose_configuration_xml().replacen(from, to, 1);
        for padding in [0, 8 * 1024 * 1024] {
            let xml = xml.replace(
                "</MetaDataObject>",
                &format!("<!--{}--></MetaDataObject>", "x".repeat(padding)),
            );
            let result = read_purpose_configuration(&xml, line);
            let visible_limit = result
                .data
                .as_ref()
                .and_then(|data| data["limits"].as_array())
                .is_some_and(|limits| !limits.is_empty());
            assert!(
                !result.ok || !result.diagnostics.is_empty() || visible_limit,
                "damaged {from}, padding {padding}, silently accepted: {:?}",
                result.data
            );
        }
    }
}

/// A large list cannot bypass the existing canonical response limit.
/// rule: INV.WIRE.READ-PAGE-BUDGET
#[test]
fn medium_configuration_default_roles_respect_the_transport_budget() {
    let name = "R".repeat(64);
    let limit = unica_execution_contract::result_size::MAX_CANONICAL_RESULT_BYTES;
    let roles = (0..limit / 64)
        .map(|index| format!("<xr:Item xsi:type=\"xr:MDObjectRef\">Role.{name}{index}</xr:Item>"))
        .collect::<String>();
    let xml = purpose_configuration_xml();
    let start = xml.find("<DefaultRoles>").unwrap() + "<DefaultRoles>".len();
    let end = xml.find("</DefaultRoles>").unwrap();
    let xml = format!("{}{}{}", &xml[..start], roles, &xml[end..]);
    let result = read_purpose_configuration(
        &xml,
        unica_source_contract::format_identity::PlatformLine::new(8, 3, 27),
    );
    assert!(!result.ok, "oversized role list was returned successfully");
    assert_eq!(result.diagnostics[0]["code"], "result_too_large");
    assert!(result.cursor.is_none());
}

/// Namespace URIs, rather than prefix spelling, identify list members.
#[test]
fn configuration_purpose_values_accept_alternative_namespace_prefixes() {
    let xml = purpose_configuration_xml()
        .replace("v8:", "core:")
        .replace("xmlns:v8=", "xmlns:core=")
        .replace("xr:", "refs:")
        .replace("xmlns:xr=", "xmlns:refs=")
        .replace("app:", "mobile:")
        .replace("xmlns:app=", "xmlns:mobile=");
    for padding in [0, 8 * 1024 * 1024] {
        let xml = xml.replace(
            "</MetaDataObject>",
            &format!("<!--{}--></MetaDataObject>", "x".repeat(padding)),
        );
        assert_purpose_values(&view_purpose_configuration(
            &xml,
            unica_source_contract::format_identity::PlatformLine::new(8, 3, 27),
        ));
    }
}
