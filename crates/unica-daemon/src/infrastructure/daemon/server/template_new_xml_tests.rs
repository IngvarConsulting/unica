use super::actor_capacity_tests::direct_v5;
use super::*;
use unica_execution::invocation_store::ToolIdentity;

use crate::infrastructure::metadata_operations::MetadataOperations;
use serde_json::json;
use std::collections::{BTreeMap, BTreeSet};
use std::ops::Range;
use std::path::{Path, PathBuf};
use unica_address::source_target::{MetadataAddress, PLATFORM_XML_8_3_27_FORMAT_2_20};
use unica_metadata::{
    MetaCollection, MetaEditOperation, MetaElementInput, MetaElementUpdateInput, MetaTemplateKind,
    MetaValidationStatus,
};
use unica_source_contract::workspace::WorkspaceContext;
use unica_work_control::cancellation::CancellationToken;
use unica_workspace::test_support::tree_snapshot;

const OWNER: &str = "Reports/CorpusReport.xml";
const DESCRIPTOR: &str = "Reports/CorpusReport/Templates/CorpusTemplate.xml";
const CONTENT: &str = "Reports/CorpusReport/Templates/CorpusTemplate/Ext/Template.xml";
const AT: &str = "main:Report.CorpusReport";
const TEMPLATE_AT: &str = "main:Report.CorpusReport.Template.CorpusTemplate";
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

fn workspace(with_cache: bool) -> tempfile::TempDir {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("src");
    let seed: &[(&str, &[u8])] = &[
        ("Configuration.xml", native!("seed/Configuration.xml")),
        ("ConfigDumpInfo.xml", native!("seed/ConfigDumpInfo.xml")),
        (
            "Ext/ClientApplicationInterface.xml",
            native!("seed/Ext/ClientApplicationInterface.xml"),
        ),
        (
            "Languages/Русский.xml",
            native!("seed/Languages/Русский.xml"),
        ),
        (OWNER, native!("seed/Reports/CorpusReport.xml")),
        (
            "Reports/CorpusReport/Ext/ObjectModule.bsl",
            native!("seed/Reports/CorpusReport/Ext/ObjectModule.bsl"),
        ),
        (
            "Reports/CorpusReport/Ext/ManagerModule.bsl",
            native!("seed/Reports/CorpusReport/Ext/ManagerModule.bsl"),
        ),
    ];
    for (relative, bytes) in seed {
        let path = source.join(relative);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, bytes).unwrap();
    }
    std::fs::write(
        root.path().join("v8project.yaml"),
        "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
    )
    .unwrap();
    std::fs::create_dir_all(source.join("Adjacent/empty")).unwrap();
    std::fs::write(source.join("Adjacent/opaque.bin"), b"\0\xffadjacent\r\n").unwrap();
    if with_cache {
        let cache = root.path().join(".build/unica");
        std::fs::create_dir_all(cache.join("empty")).unwrap();
        std::fs::write(cache.join("preview-sentinel"), b"retained cache\r\n").unwrap();
    }
    root
}

fn directories(root: &Path) -> BTreeSet<PathBuf> {
    fn visit(root: &Path, current: &Path, result: &mut BTreeSet<PathBuf>) {
        for entry in std::fs::read_dir(current).unwrap() {
            let entry = entry.unwrap();
            if entry.file_type().unwrap().is_dir() {
                result.insert(entry.path().strip_prefix(root).unwrap().to_path_buf());
                visit(root, &entry.path(), result);
            }
        }
    }
    let mut result = BTreeSet::new();
    visit(root, root, &mut result);
    result
}

fn publish_template(kind: &str, with_cache: bool) -> tempfile::TempDir {
    let fixture = workspace(with_cache);
    let root = std::fs::canonicalize(fixture.path()).unwrap();
    let source = root.join("src");
    let before = tree_snapshot(&source);
    let before_dirs = directories(&source);
    let cache = root.join(".build/unica");
    let cache_before = cache
        .exists()
        .then(|| (tree_snapshot(&cache), directories(&cache)));
    let runtime = V5CanonicalInvocationRuntime::new(
        Arc::new(crate::infrastructure::daemon::v13_service::CanonicalV13ReadService::default()),
        Arc::new(TokioClock),
    );
    let invoke = |tool, arguments| {
        direct_v5(
            &runtime,
            InvocationRequest::new(tool, arguments, root.to_string_lossy(), 7_000).unwrap(),
        )
        .unwrap()
    };
    let can = invoke(
        ToolIdentity::View,
        json!({"at":format!("{AT}.Can.template.add")}),
    );
    assert!(can.ok, "actual template.add declaration failed: {can:?}");
    let declaration = can.data.as_ref().unwrap();
    assert_eq!(declaration["kind"], "Can");
    assert_eq!(declaration["props"]["op"], "template.add");
    assert!(declaration["props"].get("implemented").is_none());
    let planned = invoke(
        ToolIdentity::Apply,
        json!({"at":AT,"ops":[{
            "op":"template.add","args":{"items":[{"name":"CorpusTemplate","templateType":kind}]}
        }]}),
    );
    assert!(planned.ok, "canonical template preview failed: {planned:?}");
    let plan = planned.data.as_ref().unwrap();
    assert_eq!(plan["mode"], "preview");
    assert!(plan["executionToken"]
        .as_str()
        .is_some_and(|value| !value.is_empty()));
    assert!(plan["planHash"]
        .as_str()
        .is_some_and(|value| !value.is_empty()));
    assert_eq!(
        (tree_snapshot(&source), directories(&source)),
        (before.clone(), before_dirs.clone()),
        "preview changed source files or directories"
    );
    assert_eq!(
        cache
            .exists()
            .then(|| (tree_snapshot(&cache), directories(&cache))),
        cache_before,
        "preview changed cache files, directories or existence"
    );
    let executed = invoke(
        ToolIdentity::Apply,
        json!({"executionToken":plan["executionToken"]}),
    );
    assert!(
        executed.ok,
        "canonical template publication failed: {executed:?}"
    );
    let publication = executed.data.as_ref().unwrap();
    assert_eq!(publication["mode"], "published");
    assert_eq!(publication["planHash"], plan["planHash"]);
    let after = tree_snapshot(&source);
    let created = after
        .keys()
        .filter(|path| !before.contains_key(*path))
        .cloned()
        .collect::<BTreeSet<_>>();
    assert_eq!(
        created,
        BTreeSet::from([PathBuf::from(DESCRIPTOR), PathBuf::from(CONTENT)])
    );
    assert!(
        before.keys().all(|path| after.contains_key(path)),
        "publication removed a source file"
    );
    let modified = before
        .iter()
        .filter(|(path, bytes)| after.get(*path) != Some(*bytes))
        .map(|(path, _)| path.clone())
        .collect::<BTreeSet<_>>();
    assert_eq!(modified, BTreeSet::from([PathBuf::from(OWNER)]));
    let after_dirs = directories(&source);
    assert!(before_dirs.is_subset(&after_dirs));
    assert_eq!(
        after_dirs
            .difference(&before_dirs)
            .cloned()
            .collect::<BTreeSet<_>>(),
        BTreeSet::from([
            PathBuf::from("Reports/CorpusReport/Templates"),
            PathBuf::from("Reports/CorpusReport/Templates/CorpusTemplate"),
            PathBuf::from("Reports/CorpusReport/Templates/CorpusTemplate/Ext"),
        ])
    );
    let owner_text = std::str::from_utf8(&after[Path::new(OWNER)]).unwrap();
    let owner = roxmltree::Document::parse(owner_text.strip_prefix('\u{feff}').unwrap()).unwrap();
    let children = owner
        .descendants()
        .find(|node| node.has_tag_name((MD, "ChildObjects")))
        .unwrap();
    let references = children
        .children()
        .filter(|node| node.is_element())
        .collect::<Vec<_>>();
    assert_eq!(references.len(), 1);
    assert!(references[0].has_tag_name((MD, "Template")));
    assert_eq!(references[0].text(), Some("CorpusTemplate"));
    let viewed = invoke(ToolIdentity::View, json!({"at":TEMPLATE_AT}));
    assert!(viewed.ok, "published template is unreadable: {viewed:?}");
    assert_eq!(viewed.data.as_ref().unwrap()["kind"], "Template");
    assert_eq!(viewed.data.as_ref().unwrap()["at"], TEMPLATE_AT);
    assert_eq!(
        tree_snapshot(&source),
        after,
        "post-publication view changed source"
    );
    fixture
}

fn template_uuid(bytes: &[u8], kind: &str) -> String {
    let text = std::str::from_utf8(bytes).unwrap();
    let document =
        roxmltree::Document::parse(text.strip_prefix('\u{feff}').unwrap_or(text)).unwrap();
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
    let root = publish_template("SpreadsheetDocument", false);
    let published = std::fs::read(root.path().join("src").join(DESCRIPTOR)).unwrap();
    assert_eq!(
        published,
        bind_template_uuid(
            native!("native-spreadsheet-descriptor.xml"),
            &published,
            "SpreadsheetDocument"
        ),
        "canonical template descriptor must match raw native bytes except for its one UUID"
    );
}

#[test]
fn canonical_spreadsheet_template_content_matches_native_bytes() {
    let root = publish_template("SpreadsheetDocument", true);
    let published = std::fs::read(root.path().join("src").join(CONTENT)).unwrap();
    assert_eq!(
        published,
        native!("native-spreadsheet-content.xml"),
        "canonical MXL scaffold must match raw native bytes without normalization"
    );
}

#[test]
fn canonical_dcs_template_descriptor_matches_native_bytes_without_replacing_its_settings() {
    let root = publish_template("DataCompositionSchema", false);
    let content = std::fs::read_to_string(root.path().join("src").join(CONTENT)).unwrap();
    let xml = roxmltree::Document::parse(content.strip_prefix('\u{feff}').unwrap()).unwrap();
    let settings_ns = "http://v8.1c.ru/8.1/data-composition-system/settings";
    let settings = xml
        .descendants()
        .find(|node| node.has_tag_name((settings_ns, "settings")))
        .unwrap();
    let children = settings
        .children()
        .filter(|node| node.is_element())
        .collect::<Vec<_>>();
    assert_eq!(children.len(), 1, "native canonical DCS retains its structure while eliding the empty top selection; retained empty settings are a different scaffold");
    assert!(children[0].has_tag_name((settings_ns, "item")));
    assert_eq!(
        children[0].attribute(("http://www.w3.org/2001/XMLSchema-instance", "type")),
        Some("dcsset:StructureItemGroup")
    );
    let group_children = children[0]
        .children()
        .filter(|node| node.is_element())
        .collect::<Vec<_>>();
    assert_eq!(group_children.len(), 1, "native empty order is absent");
    assert!(group_children[0].has_tag_name((settings_ns, "selection")));
    assert!(children[0].descendants().any(|node| node
        .attribute(("http://www.w3.org/2001/XMLSchema-instance", "type"))
        == Some("dcsset:SelectedItemAuto")));
    // The retained DCS payload is a different scaffold and is not this test's oracle.
    let published = std::fs::read(root.path().join("src").join(DESCRIPTOR)).unwrap();
    assert_eq!(published, bind_template_uuid(native!("native-dcs-descriptor.xml"), &published, "DataCompositionSchema"), "canonical DCS descriptor must match raw native bytes; canonical payload native acceptance is separate");
}

#[test]
fn canonical_dcs_template_content_matches_native_bytes() {
    let root = publish_template("DataCompositionSchema", true);
    let published = std::fs::read(root.path().join("src").join(CONTENT)).unwrap();
    assert_eq!(
        published,
        native!("native-canonical-dcs-content.xml"),
        "canonical DCS scaffold must match its own stable native export, preserving Group/Auto and raw bytes"
    );
}

const V8: &str = "http://v8.1c.ru/8.1/data/core";
const FINAL_VALUE: &str = "Final\n\t& <value> \"quoted\" 'single'";
// An independent literal, not the production XML escape/format helper.
const FINAL_XML_VALUE: &str = "Final\n\t&amp; &lt;value&gt; &quot;quoted&quot; 'single'";

fn one_child<'a, 'input>(
    parent: roxmltree::Node<'a, 'input>,
    namespace: &str,
    name: &str,
) -> roxmltree::Node<'a, 'input> {
    let matches = parent
        .children()
        .filter(|node| node.has_tag_name((namespace, name)))
        .collect::<Vec<_>>();
    assert_eq!(matches.len(), 1, "expected one direct {name}");
    matches[0]
}

fn template_fields(bytes: &[u8], name: &str, kind: MetaTemplateKind) -> (String, String, String) {
    let text = std::str::from_utf8(bytes).unwrap();
    let doc = roxmltree::Document::parse(text.strip_prefix('\u{feff}').unwrap_or(text)).unwrap();
    let root = doc.root_element();
    assert!(root.has_tag_name((MD, "MetaDataObject")));
    assert_eq!(root.attribute("version"), Some("2.20"));
    assert_eq!(root.children().filter(|node| node.is_element()).count(), 1);
    let template = one_child(root, MD, "Template");
    let props = one_child(template, MD, "Properties");
    assert_eq!(one_child(props, MD, "Name").text(), Some(name));
    assert_eq!(
        one_child(props, MD, "TemplateType").text(),
        Some(kind.as_str())
    );
    let identity = template.attribute("uuid").unwrap();
    assert!(!uuid::Uuid::parse_str(identity).unwrap().is_nil());
    assert_eq!(text.matches(identity).count(), 1);
    let synonym = one_child(props, MD, "Synonym");
    let item = one_child(synonym, V8, "item");
    assert_eq!(
        synonym.children().filter(|node| node.is_element()).count(),
        1
    );
    assert_eq!(one_child(item, V8, "lang").text(), Some("ru"));
    (
        identity.to_owned(),
        one_child(props, MD, "Comment")
            .text()
            .unwrap_or("")
            .to_owned(),
        one_child(item, V8, "content")
            .text()
            .unwrap_or("")
            .to_owned(),
    )
}

// Ranges come only from the preimage (or the independent native fixture).
fn template_property_range(bytes: &[u8], property: &str) -> Range<usize> {
    let text = std::str::from_utf8(bytes).unwrap();
    let body = text.strip_prefix('\u{feff}').unwrap_or(text);
    let offset = text.len() - body.len();
    let doc = roxmltree::Document::parse(body).unwrap();
    let template = one_child(doc.root_element(), MD, "Template");
    let props = one_child(template, MD, "Properties");
    let node = if property == "SynonymContent" {
        one_child(
            one_child(one_child(props, MD, "Synonym"), V8, "item"),
            V8,
            "content",
        )
    } else {
        one_child(props, MD, property)
    };
    let range = node.range();
    range.start + offset..range.end + offset
}

fn patch_preimage(bytes: &[u8], mut edits: Vec<(Range<usize>, String)>) -> Vec<u8> {
    edits.sort_by_key(|(range, _)| range.start);
    for pair in edits.windows(2) {
        assert!(pair[0].0.end <= pair[1].0.start, "declared edits overlap");
    }
    let mut expected = bytes.to_vec();
    for (range, replacement) in edits.into_iter().rev() {
        assert!(range.start < range.end && range.end <= expected.len());
        expected.splice(range, replacement.bytes());
    }
    expected
}

fn retained_paths(name: &str) -> (PathBuf, PathBuf) {
    (
        PathBuf::from(format!("Reports/CorpusReport/Templates/{name}.xml")),
        PathBuf::from(format!(
            "Reports/CorpusReport/Templates/{name}/Ext/Template.xml"
        )),
    )
}

fn add_template(name: &str, kind: MetaTemplateKind) -> MetaEditOperation {
    MetaEditOperation::add(
        MetaCollection::Templates,
        None,
        vec![MetaElementInput {
            name: name.into(),
            template_type: Some(kind),
            synonym: Some("Initial caption".into()),
            comment: Some("Initial comment".into()),
            ..MetaElementInput::default()
        }],
    )
    .unwrap()
}

fn retained_publish(
    root: &Path,
    operations: Vec<MetaEditOperation>,
    name: &str,
    kind: MetaTemplateKind,
) -> (Vec<u8>, Vec<u8>) {
    let root = std::fs::canonicalize(root).unwrap();
    let context = WorkspaceContext {
        cwd: root.clone(),
        workspace_root: root.clone(),
        cache_root: root.join(".build/unica"),
        workspace_epoch: 0,
    };
    let target =
        MetadataAddress::parse(PLATFORM_XML_8_3_27_FORMAT_2_20, "Report.CorpusReport").unwrap();
    let request =
        unica_metadata::request::MetadataRequest::Edit(unica_metadata::request::MetaEditRequest {
            source_set: "main".into(),
            metadata_path: target.clone(),
            operations,
            dry_run: false,
        });
    let before = (tree_snapshot(&root), directories(&root));
    let cancellation = CancellationToken::new();
    let prepared = MetadataOperations::prepare_mutation(&request, &context, &cancellation)
        .expect("retained request must prepare before publication");
    assert_eq!(
        (tree_snapshot(&root), directories(&root)),
        before,
        "prepare changed source/cache bytes or membership"
    );
    let validation =
        MetadataOperations::validate(prepared.validation_subject(), &context, &cancellation);
    assert_eq!(
        validation.status,
        MetaValidationStatus::Passed,
        "{:?}",
        validation.diagnostics
    );
    assert_eq!(
        (tree_snapshot(&root), directories(&root)),
        before,
        "validation changed source/cache bytes or membership"
    );
    let resources = &prepared.validation_subject().resources;
    let descriptors = resources
        .iter()
        .filter(|resource| {
            matches!(
                &resource.role,
                unica_metadata::validation_subject::MetadataResourceRole::Template {owner, name: resource_name}
                    if owner == &target && resource_name == name
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(
        descriptors.len(),
        1,
        "one prepared final Template descriptor"
    );
    let expected_type =
        unica_metadata::validation_subject::MetadataTemplateType::from_descriptor_value(
            kind.as_str(),
        )
        .unwrap();
    let child_address = format!("Report.CorpusReport.Template.{name}");
    let payloads = resources
        .iter()
        .filter(|resource| {
            matches!(
                &resource.role,
                unica_metadata::validation_subject::MetadataResourceRole::ChildResource {
                    child,
                    kind: unica_metadata::validation_subject::MetadataChildResourceKind::TemplateContent {
                        template_type,
                        part: unica_metadata::validation_subject::MetadataTemplateResourcePart::Primary,
                    },
                    ordinal: 0,
                } if child.as_str() == child_address && *template_type == expected_type
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(payloads.len(), 1, "one prepared final Template payload");
    let descriptor = descriptors[0].bytes.clone();
    let payload = payloads[0].bytes.clone();
    template_fields(&descriptor, name, kind);
    prepared
        .publish(&cancellation)
        .expect("retained publication must succeed");
    let (descriptor_path, payload_path) = retained_paths(name);
    assert_eq!(
        std::fs::read(root.join("src").join(descriptor_path)).unwrap(),
        descriptor,
        "published descriptor must equal the WHOLE prepared validation image"
    );
    assert_eq!(
        std::fs::read(root.join("src").join(payload_path)).unwrap(),
        payload,
        "published payload must equal the WHOLE prepared validation image"
    );
    (descriptor, payload)
}

fn owner_children_range(bytes: &[u8], expected_name: Option<&str>) -> Range<usize> {
    let text = std::str::from_utf8(bytes).unwrap();
    let body = text.strip_prefix('\u{feff}').unwrap_or(text);
    let doc = roxmltree::Document::parse(body).unwrap();
    let owner = one_child(doc.root_element(), MD, "Report");
    let children = one_child(owner, MD, "ChildObjects");
    let refs = children
        .children()
        .filter(|node| node.is_element())
        .collect::<Vec<_>>();
    if let Some(name) = expected_name {
        assert_eq!(refs.len(), 1);
        assert!(refs[0].has_tag_name((MD, "Template")));
        assert_eq!(refs[0].attributes().len(), 0);
        assert_eq!(
            refs[0].children().filter(|node| node.is_element()).count(),
            0
        );
        assert_eq!(refs[0].text(), Some(name));
    } else {
        assert!(refs.is_empty());
    }
    let offset = text.len() - body.len();
    let range = children.range();
    range.start + offset..range.end + offset
}

fn assert_only_template_created(
    source: &Path,
    before: &BTreeMap<PathBuf, Vec<u8>>,
    before_dirs: &BTreeSet<PathBuf>,
    name: &str,
) {
    let after = tree_snapshot(source);
    let (descriptor, payload) = retained_paths(name);
    let mut expected_paths = before.keys().cloned().collect::<BTreeSet<_>>();
    expected_paths.extend([descriptor, payload]);
    assert_eq!(
        after.keys().cloned().collect::<BTreeSet<_>>(),
        expected_paths
    );
    for (path, bytes) in before {
        if path != Path::new(OWNER) {
            assert_eq!(&after[path], bytes, "creation changed unrelated {path:?}");
        }
    }
    let old = &before[Path::new(OWNER)];
    let new = &after[Path::new(OWNER)];
    let old_range = owner_children_range(old, None);
    let new_range = owner_children_range(new, Some(name));
    assert_eq!(&old[..old_range.start], &new[..new_range.start]);
    assert_eq!(&old[old_range.end..], &new[new_range.end..]);
    let mut expected_dirs = before_dirs.clone();
    expected_dirs.extend([
        PathBuf::from("Reports/CorpusReport/Templates"),
        PathBuf::from(format!("Reports/CorpusReport/Templates/{name}")),
        PathBuf::from(format!("Reports/CorpusReport/Templates/{name}/Ext")),
    ]);
    assert_eq!(directories(source), expected_dirs);
}

fn retained_batch_keeps_final_fields(kind: MetaTemplateKind, native_descriptor: &[u8]) {
    let root = workspace(true);
    let source = root.path().join("src");
    let before = tree_snapshot(&source);
    let before_dirs = directories(&source);
    let update = MetaEditOperation::update(
        MetaCollection::Templates,
        None,
        vec![MetaElementUpdateInput {
            name: "Draft".into(),
            new_name: Some("FinalTemplate".into()),
            synonym: Some(FINAL_VALUE.into()),
            comment: Some(FINAL_VALUE.into()),
            ..MetaElementUpdateInput::default()
        }],
    )
    .unwrap();
    let (descriptor, _) = retained_publish(
        root.path(),
        vec![add_template("Draft", kind), update],
        "FinalTemplate",
        kind,
    );
    let (identity, comment, synonym) = template_fields(&descriptor, "FinalTemplate", kind);
    assert_eq!(comment, FINAL_VALUE, "literal LF/TAB comment must survive");
    assert_eq!(synonym, FINAL_VALUE, "literal LF/TAB synonym must survive");
    assert_only_template_created(&source, &before, &before_dirs, "FinalTemplate");
    let old_identity = template_uuid(native_descriptor, kind.as_str());
    let native_text = std::str::from_utf8(native_descriptor).unwrap();
    let uuid_start = native_text.find(&old_identity).unwrap();
    let name_range = template_property_range(native_descriptor, "Name");
    let comment_range = template_property_range(native_descriptor, "Comment");
    let synonym_range = template_property_range(native_descriptor, "SynonymContent");
    assert_eq!(
        &native_descriptor[name_range.clone()],
        b"<Name>CorpusTemplate</Name>"
    );
    assert_eq!(&native_descriptor[comment_range.clone()], b"<Comment/>");
    assert_eq!(
        &native_descriptor[synonym_range.clone()],
        b"<v8:content>Corpus template</v8:content>"
    );
    let expected = patch_preimage(
        native_descriptor,
        vec![
            (uuid_start..uuid_start + old_identity.len(), identity),
            (name_range, "<Name>FinalTemplate</Name>".into()),
            (
                comment_range,
                format!("<Comment>{FINAL_XML_VALUE}</Comment>"),
            ),
            (
                synonym_range,
                format!("<v8:content>{FINAL_XML_VALUE}</v8:content>"),
            ),
        ],
    );
    assert_eq!(descriptor, expected,
        "only UUID, Name, Comment and ru content may differ from the native envelope; LF/TAB inside values are literal");
}

#[test]
fn retained_spreadsheet_add_update_batch_keeps_final_fields_and_prepared_bytes() {
    retained_batch_keeps_final_fields(
        MetaTemplateKind::SpreadsheetDocument,
        native!("native-spreadsheet-descriptor.xml"),
    );
}

#[test]
fn retained_dcs_add_update_batch_keeps_final_fields_and_prepared_bytes() {
    retained_batch_keeps_final_fields(
        MetaTemplateKind::DataCompositionSchema,
        native!("native-dcs-descriptor.xml"),
    );
}

fn retained_second_update_preserves_bytes(kind: MetaTemplateKind) {
    let root = workspace(true);
    let source = root.path().join("src");
    let initial_files = tree_snapshot(&source);
    let initial_dirs = directories(&source);
    let (descriptor, payload) = retained_publish(
        root.path(),
        vec![add_template("Original", kind)],
        "Original",
        kind,
    );
    assert_only_template_created(&source, &initial_files, &initial_dirs, "Original");
    let (identity, comment, synonym) = template_fields(&descriptor, "Original", kind);
    assert_eq!(comment, "Initial comment");
    assert_eq!(synonym, "Initial caption");
    let before = tree_snapshot(&source);
    let before_dirs = directories(&source);
    let update = MetaEditOperation::update(
        MetaCollection::Templates,
        None,
        vec![MetaElementUpdateInput {
            name: "Original".into(),
            new_name: Some("Renamed".into()),
            comment: Some("Changed & <value>".into()),
            ..MetaElementUpdateInput::default()
        }],
    )
    .unwrap();
    let (updated, updated_payload) = retained_publish(root.path(), vec![update], "Renamed", kind);
    assert_eq!(
        updated_payload, payload,
        "rename must retain raw new-scaffold payload"
    );
    assert_eq!(
        template_fields(&updated, "Renamed", kind),
        (identity, "Changed & <value>".into(), synonym)
    );
    let expected = patch_preimage(
        &descriptor,
        vec![
            (
                template_property_range(&descriptor, "Name"),
                "<Name>Renamed</Name>".into(),
            ),
            (
                template_property_range(&descriptor, "Comment"),
                "<Comment>Changed &amp; &lt;value&gt;</Comment>".into(),
            ),
        ],
    );
    assert_eq!(
        updated, expected,
        "second update may change only the declared Name/Comment ranges"
    );
    let mut expected_files = before;
    let (old_descriptor, old_payload) = retained_paths("Original");
    let (new_descriptor, new_payload) = retained_paths("Renamed");
    expected_files.remove(&old_descriptor).unwrap();
    expected_files.remove(&old_payload).unwrap();
    expected_files.insert(new_descriptor, updated);
    expected_files.insert(new_payload, payload);
    let owner = expected_files.get_mut(Path::new(OWNER)).unwrap();
    owner_children_range(owner, Some("Original"));
    let text = std::str::from_utf8(owner).unwrap();
    let reference = "<Template>Original</Template>";
    assert_eq!(text.matches(reference).count(), 1);
    let start = text.find(reference).unwrap();
    *owner = patch_preimage(
        owner,
        vec![(
            start..start + reference.len(),
            "<Template>Renamed</Template>".into(),
        )],
    );
    assert_eq!(
        tree_snapshot(&source),
        expected_files,
        "rename touched unrelated source bytes or membership"
    );
    let mut expected_dirs = before_dirs;
    for suffix in ["", "/Ext"] {
        assert!(expected_dirs.remove(&PathBuf::from(format!(
            "Reports/CorpusReport/Templates/Original{suffix}"
        ))));
        expected_dirs.insert(PathBuf::from(format!(
            "Reports/CorpusReport/Templates/Renamed{suffix}"
        )));
    }
    assert_eq!(directories(&source), expected_dirs);
    owner_children_range(&std::fs::read(source.join(OWNER)).unwrap(), Some("Renamed"));
}

#[test]
fn retained_spreadsheet_second_update_preserves_payload_and_unedited_bytes() {
    retained_second_update_preserves_bytes(MetaTemplateKind::SpreadsheetDocument);
}

#[test]
fn retained_dcs_second_update_preserves_payload_and_unedited_bytes() {
    retained_second_update_preserves_bytes(MetaTemplateKind::DataCompositionSchema);
}

fn canonical_set_preserves_new_template(kind: MetaTemplateKind) {
    let root = publish_template(kind.as_str(), true);
    let root_path = std::fs::canonicalize(root.path()).unwrap();
    let source = root_path.join("src");
    let runtime = V5CanonicalInvocationRuntime::new(
        Arc::new(crate::infrastructure::daemon::v13_service::CanonicalV13ReadService::default()),
        Arc::new(TokioClock),
    );
    let invoke = |tool, args| {
        direct_v5(
            &runtime,
            InvocationRequest::new(tool, args, root_path.to_string_lossy(), 7_000).unwrap(),
        )
        .unwrap()
    };
    let before = tree_snapshot(&source);
    let before_all = (tree_snapshot(&root_path), directories(&root_path));
    let source_dirs = directories(&source);
    let descriptor = &before[Path::new(DESCRIPTOR)];
    let (identity, old_comment, _) = template_fields(descriptor, "CorpusTemplate", kind);
    assert_eq!(old_comment, "");
    let comment_range = template_property_range(descriptor, "Comment");
    assert_eq!(&descriptor[comment_range.clone()], b"<Comment/>");
    let expected = patch_preimage(
        descriptor,
        vec![
            (
                comment_range,
                "<Comment>Changed &amp; &lt;value&gt;</Comment>".into(),
            ),
            (
                template_property_range(descriptor, "SynonymContent"),
                "<v8:content>Changed caption</v8:content>".into(),
            ),
        ],
    );
    let can = invoke(
        ToolIdentity::View,
        json!({"at":format!("{TEMPLATE_AT}.Can.template.set")}),
    );
    assert!(can.ok, "template.set declaration failed: {can:?}");
    let declaration = can.data.as_ref().unwrap();
    assert_eq!(declaration["kind"], "Can");
    assert_eq!(declaration["props"]["op"], "template.set");
    assert!(declaration["props"].get("implemented").is_none());
    let planned = invoke(
        ToolIdentity::Apply,
        json!({"at":TEMPLATE_AT,"ops":[{
            "op":"template.set","args":{"values":{"comment":"Changed & <value>","synonym":"Changed caption"}}
        }]}),
    );
    assert!(planned.ok, "template.set preview failed: {planned:?}");
    let plan = planned.data.as_ref().unwrap();
    assert_eq!(plan["mode"], "preview");
    assert_eq!(plan["effects"], 1);
    assert_eq!(
        planned.changed,
        vec![json!({"at":TEMPLATE_AT,"event":"TemplateChanged"})]
    );
    for key in ["executionToken", "planHash"] {
        assert!(
            plan[key].as_str().is_some_and(|value| !value.is_empty()),
            "missing {key}"
        );
    }
    assert_eq!(
        (tree_snapshot(&root_path), directories(&root_path)),
        before_all,
        "view/preview changed source, cache, or membership"
    );
    let executed = invoke(
        ToolIdentity::Apply,
        json!({"executionToken":plan["executionToken"]}),
    );
    assert!(executed.ok, "template.set publication failed: {executed:?}");
    let publication = executed.data.as_ref().unwrap();
    assert_eq!(publication["mode"], "published");
    assert_eq!(publication["planHash"], plan["planHash"]);
    assert_eq!(publication["effects"], plan["effects"]);
    assert_eq!(executed.changed, planned.changed);
    let mut expected_files = before;
    expected_files.insert(PathBuf::from(DESCRIPTOR), expected);
    let after = tree_snapshot(&source);
    assert_eq!(
        after, expected_files,
        "template.set must retain payload, owner, and all other source bytes"
    );
    assert_eq!(directories(&source), source_dirs);
    assert_eq!(
        template_fields(&after[Path::new(DESCRIPTOR)], "CorpusTemplate", kind),
        (
            identity,
            "Changed & <value>".into(),
            "Changed caption".into()
        )
    );
}

#[test]
fn canonical_spreadsheet_template_set_preserves_new_payload_and_unedited_bytes() {
    canonical_set_preserves_new_template(MetaTemplateKind::SpreadsheetDocument);
}

#[test]
fn canonical_dcs_template_set_preserves_new_payload_and_unedited_bytes() {
    canonical_set_preserves_new_template(MetaTemplateKind::DataCompositionSchema);
}
