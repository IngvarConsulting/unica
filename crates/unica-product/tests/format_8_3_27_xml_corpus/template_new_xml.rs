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

/// Кейс корпуса `template-add-spreadsheet-document`, опубликованный
/// каноническим `template.add`: манифест связывает настоящие байты, а новыми
/// становятся ровно дескриптор и содержимое макета.
fn published_case(case_id: &str) -> tempfile::TempDir {
    let root = tempfile::tempdir().unwrap();
    let case = EXECUTABLE_CASES
        .iter()
        .find(|case| case.id == case_id)
        .unwrap();
    assert_eq!(case.tool, CANONICAL_APPLY_TOOL);
    assert_eq!(case.branch, "SpreadsheetDocument");
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
    assert_eq!(
        created,
        BTreeSet::from([DESCRIPTOR.to_owned(), CONTENT.to_owned()])
    );
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

#[test]
fn canonical_spreadsheet_template_descriptor_matches_native_bytes() {
    let case_id = "template-add-spreadsheet-document";
    let root = published_case(case_id);
    let published = fs::read(
        root.path()
            .join(format!("cases/{case_id}/workspace/src/{DESCRIPTOR}")),
    )
    .unwrap();
    assert_eq!(
        published,
        bind_template_uuid(
            native!("native-spreadsheet-descriptor.xml"),
            &published,
            "SpreadsheetDocument"
        ),
        "the template descriptor must match the raw native export except for its one generated UUID"
    );
    let content = fs::read(
        root.path()
            .join(format!("cases/{case_id}/workspace/src/{CONTENT}")),
    )
    .unwrap();
    assert_eq!(
        content,
        native!("native-spreadsheet-content.xml"),
        "the template payload must match the raw native export without normalization"
    );
}
