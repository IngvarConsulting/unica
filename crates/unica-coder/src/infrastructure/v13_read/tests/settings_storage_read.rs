use super::*;
use unica_workspace::test_support::tree_snapshot;

const OWNER: &str = "SettingsStorage.Probe";
const SAVE: &str = "SettingsStorage.Probe.Form.Save";
const LOAD: &str = "SettingsStorage.Probe.Form.Load";

fn descriptor() -> String {
    fixture_text("acceptance/workspace-settings-storage-read/src/SettingsStorages/Probe.xml")
}

fn fixture_with_storage(xml: &str) -> RealReaderFixture {
    let fixture = RealReaderFixture::new();
    let configuration = fixture.source.join("Configuration.xml");
    write(
        &configuration,
        &fs::read_to_string(&configuration).unwrap().replace(
            "</ChildObjects>",
            "<SettingsStorage>Probe</SettingsStorage></ChildObjects>",
        ),
    );
    write(&fixture.source.join("SettingsStorages/Probe.xml"), xml);
    for form in ["Save", "Load"] {
        for suffix in [".xml", "/Ext/Form.xml", "/Ext/Form/Module.bsl"] {
            let relative = format!("SettingsStorages/Probe/Forms/{form}{suffix}");
            write(
                &fixture.source.join(&relative),
                &fixture_text(&format!(
                    "acceptance/workspace-settings-storage-read/src/{relative}"
                )),
            );
        }
    }
    fixture
}

fn read(fixture: &RealReaderFixture, source_set: &str, path: &str) -> serde_json::Value {
    let root = Arc::new(unica_workspace::read::open_source_directory(&fixture.source).unwrap());
    let authority = LogicalViewReadAuthority::new(
        &fixture.cancellation,
        source_set,
        "settings-storage-read-fixture",
        SourceSetKind::Configuration,
        root,
        PlatformProfile::v8_3_27(),
    );
    let service = ViewService::new(authority, ViewCursorStore::default());
    let result = service.view(ViewRequest::new(&format!("{source_set}:{path}")).unwrap());
    assert!(result.ok, "{result:?}");
    result.data.unwrap()
}

fn has_limit(node: &serde_json::Value, field: &str) -> bool {
    node.get("limits").is_some_and(|limits| {
        limits.as_array().unwrap().iter().any(|limit| {
            limit
                .as_str()
                .is_some_and(|message| message.contains(field))
        })
    })
}

/// rule: INV.SOURCE.METADATA-NODE-PROPERTIES
/// rule: INV.SOURCE.METADATA-OBJECT-FACTS
#[test]
fn settings_storage_view_qualifies_both_form_assignments_without_writes() {
    for source_set in ["main", "archive"] {
        for swapped in [false, true] {
            let xml = if swapped {
                descriptor()
                    .replace(SAVE, "__save__")
                    .replace(LOAD, SAVE)
                    .replace("__save__", LOAD)
            } else {
                descriptor()
            };
            let fixture = fixture_with_storage(&xml);
            let before = tree_snapshot(&fixture.source);
            let collection = read(&fixture, source_set, "SettingsStorage");
            assert_eq!(
                collection["items"][0]["at"],
                format!("{source_set}:{OWNER}")
            );
            let node = read(&fixture, source_set, OWNER);
            let (save, load) = if swapped { (LOAD, SAVE) } else { (SAVE, LOAD) };
            assert_eq!(
                node["props"]["DefaultSaveForm"],
                format!("{source_set}:{save}")
            );
            assert_eq!(
                node["props"]["DefaultLoadForm"],
                format!("{source_set}:{load}")
            );
            assert_eq!(node["props"]["kind"], "SettingsStorage");
            assert_eq!(node["props"]["Comment"], "storage-read-facts");
            assert!(!has_limit(&node, "DefaultSaveForm"));
            assert!(!has_limit(&node, "DefaultLoadForm"));
            assert_eq!(tree_snapshot(&fixture.source), before);
        }
    }
}

/// rule: INV.SOURCE.INCOMPLETE-METADATA-READ
#[test]
fn settings_storage_form_absence_and_empty_values_do_not_invent_assignments() {
    for field in ["DefaultSaveForm", "DefaultLoadForm"] {
        let reference = if field == "DefaultSaveForm" {
            SAVE
        } else {
            LOAD
        };
        let original = format!("<{field}>{reference}</{field}>");
        for raw in [
            String::new(),
            format!("<{field}/>"),
            format!("<{field}></{field}>"),
            format!("<{field} xsi:nil=\"true\"/>"),
        ] {
            let fixture = fixture_with_storage(&descriptor().replace(&original, &raw));
            let before = tree_snapshot(&fixture.source);
            let node = read(&fixture, "main", OWNER);
            assert!(node["props"].get(field).is_none(), "{raw}");
            assert!(!has_limit(&node, field), "absence is allowed: {raw}");
            let (sibling, expected) = if field == "DefaultSaveForm" {
                ("DefaultLoadForm", LOAD)
            } else {
                ("DefaultSaveForm", SAVE)
            };
            assert_eq!(node["props"][sibling], format!("main:{expected}"));
            assert_eq!(tree_snapshot(&fixture.source), before);
        }
    }
    let xml = descriptor()
        .replace(&format!("<DefaultSaveForm>{SAVE}</DefaultSaveForm>"), "")
        .replace(&format!("<DefaultLoadForm>{LOAD}</DefaultLoadForm>"), "");
    let fixture = fixture_with_storage(&xml);
    let node = read(&fixture, "main", OWNER);
    for field in ["DefaultSaveForm", "DefaultLoadForm"] {
        assert!(node["props"].get(field).is_none());
        assert!(!has_limit(&node, field));
    }
}

/// rule: INV.SOURCE.INCOMPLETE-METADATA-READ
#[test]
fn settings_storage_damaged_form_reference_keeps_the_other_assignment() {
    for field in ["DefaultSaveForm", "DefaultLoadForm"] {
        let reference = if field == "DefaultSaveForm" {
            SAVE
        } else {
            LOAD
        };
        let original = format!("<{field}>{reference}</{field}>");
        for raw in [
            format!("<{field}>not a form address</{field}>"),
            format!("<{field}><Nested>{reference}</Nested></{field}>"),
            format!("<{field} extra=\"bad\">{reference}</{field}>"),
            format!("{original}{original}"),
            format!("<{field} xmlns=\"urn:foreign\">{reference}</{field}>"),
            format!("<{field}>unbound:Save</{field}>"),
            format!("<{field} xsi:nil=\"true\">{reference}</{field}>"),
        ] {
            let fixture = fixture_with_storage(&descriptor().replace(&original, &raw));
            let before = tree_snapshot(&fixture.source);
            let node = read(&fixture, "main", OWNER);
            assert!(node["props"].get(field).is_none(), "{raw}");
            assert!(
                has_limit(&node, field),
                "damaged field needs a limit: {raw}"
            );
            let (sibling, expected) = if field == "DefaultSaveForm" {
                ("DefaultLoadForm", LOAD)
            } else {
                ("DefaultSaveForm", SAVE)
            };
            assert_eq!(node["props"][sibling], format!("main:{expected}"));
            assert_eq!(node["props"]["Comment"], "storage-read-facts");
            assert!(!has_limit(&node, sibling));
            assert_eq!(tree_snapshot(&fixture.source), before);
        }
    }
}

#[test]
fn settings_storage_form_property_qnames_use_expanded_names() {
    let xml = descriptor()
        .replace(
            "<DefaultSaveForm>",
            "<md:DefaultSaveForm xmlns:md=\"http://v8.1c.ru/8.3/MDClasses\">",
        )
        .replace("</DefaultSaveForm>", "</md:DefaultSaveForm>")
        .replace(
            "<DefaultLoadForm>",
            "<other:DefaultLoadForm xmlns:other=\"http://v8.1c.ru/8.3/MDClasses\">",
        )
        .replace("</DefaultLoadForm>", "</other:DefaultLoadForm>");
    let fixture = fixture_with_storage(&xml);
    let before = tree_snapshot(&fixture.source);
    let node = read(&fixture, "main", OWNER);
    assert_eq!(node["props"]["DefaultSaveForm"], format!("main:{SAVE}"));
    assert_eq!(node["props"]["DefaultLoadForm"], format!("main:{LOAD}"));
    assert!(!has_limit(&node, "DefaultSaveForm"));
    assert!(!has_limit(&node, "DefaultLoadForm"));
    assert_eq!(tree_snapshot(&fixture.source), before);
}

#[test]
fn settings_storage_read_observes_reference_text_without_reading_target_files() {
    let xml = descriptor().replace(SAVE, "SettingsStorage.Missing.Form.Save");
    let fixture = fixture_with_storage(&xml);
    let before = tree_snapshot(&fixture.source);
    let node = read(&fixture, "main", OWNER);
    assert_eq!(
        node["props"]["DefaultSaveForm"],
        "main:SettingsStorage.Missing.Form.Save"
    );
    assert_eq!(node["props"]["DefaultLoadForm"], format!("main:{LOAD}"));
    assert!(!has_limit(&node, "DefaultSaveForm"));
    assert_eq!(tree_snapshot(&fixture.source), before);
}
