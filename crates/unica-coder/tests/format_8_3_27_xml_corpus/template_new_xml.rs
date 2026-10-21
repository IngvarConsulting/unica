use super::*;

const DESCRIPTOR: &str = "Reports/CorpusReport/Templates/CorpusTemplate.xml";
const CONTENT: &str = "Reports/CorpusReport/Templates/CorpusTemplate/Ext/Template.xml";
const MD: &str = "http://v8.1c.ru/8.3/MDClasses";

macro_rules! native {
    ($name:literal) => {
        include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../tests/fixtures/platform_8_3_27/template-new-xml/",
            $name
        ))
    };
}

fn published_case(case_id: &str) -> tempfile::TempDir {
    let root = tempfile::tempdir().unwrap();
    let case = EXECUTABLE_CASES
        .iter()
        .find(|case| case.id == case_id)
        .unwrap();
    assert_eq!(case.tool, "unica.meta.edit");
    let mut gate = SequentialCallGate::default();
    let generated = run_corpus_case(root.path(), case, &mut gate).unwrap();
    let manifest = serde_json::to_value(generated).unwrap();
    let prefix = format!("cases/{case_id}/workspace/src/");
    let mut created = BTreeSet::new();
    let mut modified = BTreeSet::new();
    for section in ["files", "nonXmlFiles"] {
        for entry in manifest[section].as_array().unwrap() {
            let path = entry["path"].as_str().unwrap();
            assert_eq!(
                format!(
                    "{:x}",
                    Sha256::digest(fs::read(root.path().join(path)).unwrap())
                ),
                entry["sha256"].as_str().unwrap(),
                "manifest must bind actual published bytes: {path}"
            );
            match entry["delta"].as_str().unwrap() {
                "created" => {
                    assert_eq!(entry["seed"], false);
                    assert!(created.insert(path.strip_prefix(&prefix).unwrap().to_owned()));
                }
                "modified" => {
                    assert_eq!(entry["seed"], true);
                    modified.insert(path.strip_prefix(&prefix).unwrap().to_owned());
                }
                "unchanged" => assert_eq!(entry["seed"], true),
                other => panic!("unexpected file delta: {other}"),
            }
        }
    }
    assert!(manifest["removedPaths"].as_array().unwrap().is_empty());
    assert!(manifest["removedNonXmlPaths"]
        .as_array()
        .unwrap()
        .is_empty());
    assert_eq!(
        modified,
        BTreeSet::from(["Reports/CorpusReport.xml".to_owned()])
    );
    let mut expected = BTreeSet::from([DESCRIPTOR.to_owned()]);
    let workspace = root.path().join(format!("cases/{case_id}/workspace/src"));
    match case.branch {
        "SpreadsheetDocument" | "DataCompositionSchema" => {
            expected.insert(CONTENT.to_owned());
        }
        "HTMLDocument" => {
            expected.insert(CONTENT.to_owned());
            let page = "Reports/CorpusReport/Templates/CorpusTemplate/Ext/Template/ru.html";
            expected.insert(page.to_owned());
            assert_eq!(
                fs::read(workspace.join(page)).unwrap(),
                include_bytes!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/../../tests/fixtures/platform_8_3_27/html-template/native-ru.html"
                )),
                "the already-native HTML page must remain byte exact"
            );
        }
        "TextDocument" | "BinaryData" => {
            let extension = if case.branch == "TextDocument" {
                "txt"
            } else {
                "bin"
            };
            let payload =
                format!("Reports/CorpusReport/Templates/CorpusTemplate/Ext/Template.{extension}");
            assert_eq!(fs::read(workspace.join(&payload)).unwrap(), b"");
            expected.insert(payload);
        }
        other => panic!("unreviewed template kind: {other}"),
    }
    assert_eq!(created, expected);
    root
}

fn template_uuid(bytes: &[u8], kind: &str) -> String {
    let text = std::str::from_utf8(bytes).unwrap();
    let document = Document::parse(text.strip_prefix('\u{feff}').unwrap_or(text)).unwrap();
    let root = document.root_element();
    assert!(root.has_tag_name((MD, "MetaDataObject")));
    assert_eq!(root.attribute("version"), Some("2.20"));
    let children = root
        .children()
        .filter(|node| node.is_element())
        .collect::<Vec<_>>();
    assert_eq!(children.len(), 1);
    let template = children[0];
    assert!(template.has_tag_name((MD, "Template")));
    let properties = template
        .children()
        .find(|node| node.has_tag_name((MD, "Properties")))
        .unwrap();
    for (name, value) in [("Name", "CorpusTemplate"), ("TemplateType", kind)] {
        assert_eq!(
            properties
                .children()
                .find(|node| node.has_tag_name((MD, name)))
                .unwrap()
                .text(),
            Some(value)
        );
    }
    let identity = template.attribute("uuid").unwrap();
    assert!(!uuid::Uuid::parse_str(identity).unwrap().is_nil());
    assert_eq!(identity.len(), 36);
    assert_eq!(
        text.matches(identity).count(),
        1,
        "only the declared Template UUID may vary"
    );
    identity.to_owned()
}

fn bind_template_uuid(native: &[u8], published: &[u8], kind: &str) -> Vec<u8> {
    assert!(native.starts_with(b"\xef\xbb\xbf"));
    let expected = template_uuid(native, kind);
    let actual = template_uuid(published, kind);
    let mut result = std::str::from_utf8(native).unwrap().to_owned();
    let offset = result.find(&expected).unwrap();
    result.replace_range(offset..offset + expected.len(), &actual);
    result.into_bytes()
}

fn assert_descriptor(case_id: &str, kind: &str, native: &[u8]) {
    let root = published_case(case_id);
    let published = fs::read(
        root.path()
            .join(format!("cases/{case_id}/workspace/src/{DESCRIPTOR}")),
    )
    .unwrap();
    assert_eq!(published, bind_template_uuid(native, &published, kind), "retained template descriptor must match the raw native export except for its one generated UUID");
}

fn assert_content(case_id: &str, native: &[u8]) {
    let root = published_case(case_id);
    let published = fs::read(
        root.path()
            .join(format!("cases/{case_id}/workspace/src/{CONTENT}")),
    )
    .unwrap();
    assert_eq!(
        published, native,
        "retained template payload must match the raw native export without normalization"
    );
}

#[test]
fn retained_spreadsheet_template_descriptor_matches_native_bytes() {
    assert_descriptor(
        "template-add-spreadsheet-document",
        "SpreadsheetDocument",
        native!("native-spreadsheet-descriptor.xml"),
    );
}

#[test]
fn retained_dcs_template_descriptor_matches_native_bytes() {
    assert_descriptor(
        "template-add-data-composition-schema",
        "DataCompositionSchema",
        native!("native-dcs-descriptor.xml"),
    );
}

#[test]
fn retained_html_template_descriptor_matches_native_bytes() {
    assert_descriptor(
        "template-add-html-document",
        "HTMLDocument",
        native!("native-html-descriptor.xml"),
    );
}

#[test]
fn retained_text_template_descriptor_matches_native_bytes() {
    assert_descriptor(
        "template-add-text-document",
        "TextDocument",
        native!("native-text-descriptor.xml"),
    );
}

#[test]
fn retained_binary_template_descriptor_matches_native_bytes() {
    assert_descriptor(
        "template-add-binary-data",
        "BinaryData",
        native!("native-binary-descriptor.xml"),
    );
}

#[test]
fn retained_spreadsheet_template_content_matches_native_bytes() {
    assert_content(
        "template-add-spreadsheet-document",
        native!("native-spreadsheet-content.xml"),
    );
}

#[test]
fn retained_dcs_template_content_matches_its_native_empty_settings_bytes() {
    assert_content(
        "template-add-data-composition-schema",
        native!("native-dcs-content.xml"),
    );
}

#[test]
fn retained_html_template_page_descriptor_matches_native_bytes() {
    assert_content(
        "template-add-html-document",
        native!("native-html-pages.xml"),
    );
}
