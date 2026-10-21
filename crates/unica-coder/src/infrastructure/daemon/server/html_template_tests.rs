use super::actor_capacity_tests::direct_v5;
use super::*;
use serde_json::json;
use std::path::Path;
use unica_execution::invocation_store::ToolIdentity;
use unica_workspace::test_support::tree_snapshot;

const PAGE: &str = "Reports/Report/Templates/Html/Ext/Template/ru.html";
const DESCRIPTOR: &str = "Reports/Report/Templates/Html.xml";
const PRIMARY: &str = "Reports/Report/Templates/Html/Ext/Template.xml";
const OWNER: &str = "Reports/Report.xml";
const TEMPLATE_AT: &str = "main:Report.Report.Template.Html";
fn workspace(page: &[u8]) -> tempfile::TempDir {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("src");
    for directory in ["Reports", "Languages", "Adjacent"] {
        std::fs::create_dir_all(source.join(directory)).unwrap();
    }
    std::fs::write(
        root.path().join("v8project.yaml"),
        "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
    )
    .unwrap();
    std::fs::write(source.join("Configuration.xml"), r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Configuration><Properties><Name>Main</Name><DefaultLanguage>Language.Русский</DefaultLanguage></Properties><ChildObjects><Language>Русский</Language><Report>Report</Report></ChildObjects></Configuration></MetaDataObject>"#).unwrap();
    std::fs::write(source.join("Languages/Русский.xml"), r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Language uuid="33333333-3333-4333-8333-333333333333"><Properties><Name>Русский</Name><Comment/><LanguageCode>ru</LanguageCode></Properties></Language></MetaDataObject>"#).unwrap();
    let children = "<Template>Html</Template>";
    std::fs::write(source.join(OWNER), format!(r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" xmlns:v8="http://v8.1c.ru/8.1/data/core" xmlns:xr="http://v8.1c.ru/8.3/xcf/readable" xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" version="2.20"><Report uuid="11111111-1111-4111-8111-111111111111"><Properties><Name>Report</Name><Synonym/><Comment/></Properties><ChildObjects>{children}</ChildObjects></Report></MetaDataObject>"#)).unwrap();
    std::fs::write(source.join("Adjacent/opaque.bin"), b"\0\xffadjacent\r\n").unwrap();
    {
        std::fs::create_dir_all(source.join(PAGE).parent().unwrap()).unwrap();
        std::fs::write(source.join(DESCRIPTOR), r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" xmlns:v8="http://v8.1c.ru/8.1/data/core" version="2.20"><Template uuid="22222222-2222-4222-8222-222222222222"><Properties><Name>Html</Name><Synonym/><Comment/><TemplateType>HTMLDocument</TemplateType></Properties></Template></MetaDataObject>"#).unwrap();
        std::fs::write(source.join(PRIMARY), r#"<Help xmlns="http://v8.1c.ru/8.3/xcf/extrnprops" version="2.20"><Page>ru</Page></Help>"#).unwrap();
        std::fs::write(source.join(PAGE), page).unwrap();
    }
    root
}

fn apply_pair(
    runtime: &V5CanonicalInvocationRuntime,
    root: &Path,
    at: &str,
    ops: serde_json::Value,
) {
    let root = std::fs::canonicalize(root).unwrap();
    let source = root.join("src");
    let before = tree_snapshot(&source);
    let cache = root.join(".build/unica");
    let cache_before = cache.exists().then(|| tree_snapshot(&cache));
    let call = |arguments| {
        let request = InvocationRequest::new(
            ToolIdentity::Apply,
            arguments,
            root.to_string_lossy(),
            7_000,
        )
        .unwrap();
        direct_v5(runtime, request).unwrap()
    };
    let planned = call(json!({"at":at,"ops":ops}));
    assert!(planned.ok, "preview failed: {planned:?}");
    let planned_data = planned.data.as_ref().unwrap();
    assert_eq!(planned_data["mode"], "preview");
    assert!(planned_data["executionToken"].is_string());
    assert_eq!(
        tree_snapshot(&source),
        before,
        "preview changed source bytes"
    );
    assert_eq!(
        cache.exists().then(|| tree_snapshot(&cache)),
        cache_before,
        "preview changed workspace cache existence or bytes",
    );
    let published = call(json!({"executionToken":planned_data["executionToken"]}));
    assert!(published.ok, "publication failed: {published:?}");
    assert_eq!(
        planned_data["planHash"],
        published.data.as_ref().unwrap()["planHash"]
    );
}

fn runtime() -> V5CanonicalInvocationRuntime {
    V5CanonicalInvocationRuntime::new(
        Arc::new(crate::infrastructure::daemon::v13_service::CanonicalV13ReadService::default()),
        Arc::new(TokioClock),
    )
}

fn assert_comment(source: &Path, relative: &str, expected: &str) {
    let bytes = std::fs::read_to_string(source.join(relative)).unwrap();
    let xml = roxmltree::Document::parse(bytes.trim_start_matches('\u{feff}')).unwrap();
    assert!(xml
        .descendants()
        .any(|node| node.has_tag_name("Comment") && node.text() == Some(expected)));
}

#[test]
fn canonical_html_property_edits_preserve_authored_page_bytes_with_and_without_bom() {
    let authored = "<!DOCTYPE HTML PUBLIC \"-//W3C//DTD HTML 4.0 Transitional//EN\">\r\n<html><head><title>Справка&nbsp;1С</title></head>\n<body><p>Текст<br></body></html>";
    for prefix in ["", "\u{feff}"] {
        let page = format!("{prefix}{authored}").into_bytes();
        let root = workspace(&page);
        let source = root.path().join("src");
        let runtime = runtime();
        let before = tree_snapshot(&source);
        apply_pair(
            &runtime,
            root.path(),
            TEMPLATE_AT,
            json!([
                {"op":"template.set","args":{"values":{"comment":"template comment"}}}
            ]),
        );
        assert_comment(&source, DESCRIPTOR, "template comment");
        let after_template = tree_snapshot(&source);
        for (path, bytes) in &before {
            if path != Path::new(DESCRIPTOR) {
                assert_eq!(
                    &after_template[path], bytes,
                    "template edit changed {path:?}"
                );
            }
        }
        assert_eq!(after_template.len(), before.len());
        apply_pair(
            &runtime,
            root.path(),
            "main:Report.Report",
            json!([
                {"op":"props.set","args":{"values":{"Comment":"owner comment"}}}
            ]),
        );
        assert_comment(&source, OWNER, "owner comment");
        let after_owner = tree_snapshot(&source);
        for (path, bytes) in &after_template {
            if path != Path::new(OWNER) {
                assert_eq!(&after_owner[path], bytes, "owner edit changed {path:?}");
            }
        }
        assert_eq!(after_owner.len(), after_template.len());
        assert_eq!(after_owner[Path::new(PAGE)], page);
    }
}
