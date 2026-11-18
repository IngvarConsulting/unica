use super::*;

const ROOT: &str = "main:Subsystem.Sales";
const NESTED: &str = "main:Subsystem.Sales.Subsystem.Orders";

fn install_help(
    fixture: &RealReaderFixture,
    root: &str,
    nested: &str,
    prefixed: bool,
) -> [PathBuf; 2] {
    let prefix = if prefixed { "md:" } else { "" };
    let paths = [
        fixture.source.join("Subsystems/Sales.xml"),
        fixture
            .source
            .join("Subsystems/Sales/Subsystems/Orders.xml"),
    ];
    for ((path, name, children, uuid), fragment) in [
        (
            &paths[0],
            "Sales",
            "<Subsystem>Orders</Subsystem>",
            "11111111-1111-4111-8111-111111111111",
        ),
        (
            &paths[1],
            "Orders",
            "",
            "22222222-2222-4222-8222-222222222222",
        ),
    ]
    .into_iter()
    .zip([root, nested])
    {
        write(
            path,
            &format!(
                r#"<{prefix}MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" xmlns:md="http://v8.1c.ru/8.3/MDClasses" version="2.20"><{prefix}Subsystem uuid="{uuid}"><{prefix}Properties><{prefix}Name>{name}</{prefix}Name><{prefix}Synonym/><{prefix}Comment>Независимый комментарий</{prefix}Comment>{fragment}<{prefix}IncludeInCommandInterface>true</{prefix}IncludeInCommandInterface><{prefix}UseOneCommand>false</{prefix}UseOneCommand><{prefix}Content/></{prefix}Properties><{prefix}ChildObjects>{children}</{prefix}ChildObjects></{prefix}Subsystem></{prefix}MetaDataObject>"#
            ),
        );
    }
    paths
}

fn has_help_limit(data: &serde_json::Value) -> bool {
    data.get("limits").is_some_and(|limits| {
        limits.as_array().unwrap().iter().any(|limit| {
            limit
                .as_str()
                .is_some_and(|text| text.contains("IncludeHelpInContents"))
        })
    })
}

/// rule: INV.SOURCE.METADATA-OBJECT-FACTS
/// rule: INV.SOURCE.METADATA-NODE-PROPERTIES
#[test]
fn subsystem_help_reads_true_and_false_at_each_depth_without_changing_sources() {
    for prefixed in [false, true] {
        for (root, nested) in [(true, false), (false, true)] {
            let fixture = RealReaderFixture::new();
            let paths = install_help(
                &fixture,
                &format!("<IncludeHelpInContents>{root}</IncludeHelpInContents>"),
                &format!("<md:IncludeHelpInContents>{nested}</md:IncludeHelpInContents>"),
                prefixed,
            );
            let before = paths.each_ref().map(|path| fs::read(path).unwrap());
            let service = fixture.view_service();
            for (at, expected) in [(ROOT, root), (NESTED, nested)] {
                let result = service.view(ViewRequest::new(at).unwrap());
                assert!(result.ok, "{at}: {:?}", result.diagnostics);
                let data = result.data.unwrap();
                assert_eq!(data["props"]["IncludeHelpInContents"], json!(expected));
                assert!(data["props"]["IncludeHelpInContents"].is_boolean());
                assert_eq!(data["props"]["Comment"], "Независимый комментарий");
                assert_eq!(data["props"]["IncludeInCommandInterface"], "true");
                assert!(!has_help_limit(&data), "{data}");
            }
            for (path, bytes) in paths.into_iter().zip(before) {
                assert_eq!(fs::read(path).unwrap(), bytes);
            }
        }
    }
}

/// rule: INV.SOURCE.METADATA-OBJECT-FACTS
#[test]
fn subsystem_help_separates_absence_from_empty_or_damaged_values_at_each_depth() {
    for (fragment, damaged) in [
        ("", false),
        ("<IncludeHelpInContents/>", true),
        ("<IncludeHelpInContents></IncludeHelpInContents>", true),
        ("<IncludeHelpInContents>1</IncludeHelpInContents>", true),
        ("<IncludeHelpInContents> true </IncludeHelpInContents>", true),
        ("<IncludeHelpInContents>broken</IncludeHelpInContents>", true),
        ("<IncludeHelpInContents>true</IncludeHelpInContents><IncludeHelpInContents>false</IncludeHelpInContents>", true),
        ("<IncludeHelpInContents xmlns=\"urn:foreign\">true</IncludeHelpInContents>", true),
        ("<IncludeHelpInContents extra=\"x\">true</IncludeHelpInContents>", true),
        ("<IncludeHelpInContents><Nested>true</Nested></IncludeHelpInContents>", true),
    ] {
        let fixture = RealReaderFixture::new();
        install_help(&fixture, fragment, fragment, false);
        let service = fixture.view_service();
        for at in [ROOT, NESTED] {
            let result = service.view(ViewRequest::new(at).unwrap());
            assert!(result.ok, "one unreadable flag must not hide {at}");
            let data = result.data.unwrap();
            assert!(data["props"].get("IncludeHelpInContents").is_none());
            assert_eq!(data["props"]["Comment"], "Независимый комментарий");
            assert_eq!(has_help_limit(&data), damaged, "{at}: {fragment}: {data}");
        }
    }
}

/// rule: INV.SOURCE.SUBSYSTEM-REGISTRATION
#[test]
fn subsystem_help_keeps_the_registered_parent_chain_and_nested_identity_required() {
    for damage in ["registration", "name", "namespace"] {
        let fixture = RealReaderFixture::new();
        let paths = install_help(
            &fixture,
            "<IncludeHelpInContents>true</IncludeHelpInContents>",
            "<IncludeHelpInContents>false</IncludeHelpInContents>",
            false,
        );
        let path = if damage == "registration" {
            &paths[0]
        } else {
            &paths[1]
        };
        let original = fs::read_to_string(path).unwrap();
        let changed = match damage {
            "registration" => original.replace("<Subsystem>Orders</Subsystem>", ""),
            "name" => original.replace("<Name>Orders</Name>", "<Name>Impostor</Name>"),
            "namespace" => original.replace("http://v8.1c.ru/8.3/MDClasses", "urn:foreign"),
            _ => unreachable!(),
        };
        write(path, &changed);
        let result = fixture
            .view_service()
            .view(ViewRequest::new(NESTED).unwrap());
        assert!(
            !result.ok,
            "{damage} must not become a successful nested flag read"
        );
    }
}
