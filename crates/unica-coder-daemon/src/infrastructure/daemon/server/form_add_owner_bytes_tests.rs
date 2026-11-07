use super::actor_capacity_tests::direct_v5;
use super::*;
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use unica_execution::invocation_store::ToolIdentity;
use unica_workspace::test_support::{canonical_path, tree_snapshot};

const AT: &str = "main:Document.Order";
const OWNER: &str = "Documents/Order.xml";
const MD: &str = "http://v8.1c.ru/8.3/MDClasses";
const DEFAULT_SLOT: &str = "<DefaultObjectForm/>";
const INSERTION_SUFFIX: &str = "\t\t</ChildObjects>";

#[derive(Clone, Copy)]
enum Endings {
    Lf,
    CrLf,
    Mixed,
}

fn authored_owner(bom: bool, endings: Endings, final_newline: bool, default: bool) -> Vec<u8> {
    // Synthetic input derived from the established canonical form lifecycle
    // fixture. None of these bytes are represented as a native platform dump.
    let text = concat!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n",
        "<MetaDataObject xmlns=\"http://v8.1c.ru/8.3/MDClasses\" xmlns:v8='http://v8.1c.ru/8.1/data/core' xmlns:xs=\"http://www.w3.org/2001/XMLSchema\" xmlns:xr='http://v8.1c.ru/8.3/xcf/readable' version=\"2.20\">\n",
        "<Document uuid='11111111-1111-4111-8111-111111111111'>\n",
        "<Properties>\n<Name>Order</Name>\n",
        "<Synonym><v8:item><v8:lang>ru</v8:lang><v8:content>Keep &amp; &#65; &#x42; &quot;Q&quot;</v8:content></v8:item></Synonym>\n",
        "<Comment />\n",
        "__DEFAULT_SLOT__",
        "<!-- untouched between default and registration: 'single' &quot;double&quot; -->\n",
        "<?keep-owner spelling='unchanged'?>\n",
        "</Properties>\n<ChildObjects>\n",
        "<Attribute uuid='22222222-2222-4222-8222-222222222222'><Properties><Name>Witness</Name><Type><v8:Type>xs:string</v8:Type><v8:StringQualifiers><v8:Length>40</v8:Length><v8:AllowedLength>Variable</v8:AllowedLength></v8:StringQualifiers></Type><Comment>Keep &amp; &#67; &#x44;</Comment></Properties></Attribute>\n",
        "<!-- untouched insertion neighbor -->\n",
        "\t\t</ChildObjects>\n</Document>\n</MetaDataObject>"
    )
    .replace("__DEFAULT_SLOT__", if default { "<DefaultObjectForm/>\n" } else { "" });
    let mut bytes = if bom {
        b"\xef\xbb\xbf".to_vec()
    } else {
        Vec::new()
    };
    let lines = text.split('\n').collect::<Vec<_>>();
    for (index, line) in lines.iter().enumerate() {
        bytes.extend_from_slice(line.as_bytes());
        if index + 1 == lines.len() && !final_newline {
            continue;
        }
        let ending = match endings {
            Endings::Lf => "\n",
            Endings::CrLf => "\r\n",
            // The insertion has unambiguous local CRLF context; earlier
            // untouched lines deliberately include both LF and bare CR.
            Endings::Mixed
                if lines
                    .get(index + 1)
                    .is_some_and(|next| next.starts_with(INSERTION_SUFFIX)) =>
            {
                "\r\n"
            }
            Endings::Mixed => ["\r\n", "\n", "\r"][index % 3],
        };
        bytes.extend_from_slice(ending.as_bytes());
    }
    bytes
}

struct Fixture {
    _root: tempfile::TempDir,
    workspace: PathBuf,
    runtime: V5CanonicalInvocationRuntime,
}

impl Fixture {
    fn new(owner: &[u8], existing_cache: bool) -> Self {
        let root = tempfile::tempdir().unwrap();
        let workspace = canonical_path(root.path()).join("workspace");
        let source = workspace.join("src");
        for directory in ["Documents", "Languages", "Adjacent/empty"] {
            std::fs::create_dir_all(source.join(directory)).unwrap();
        }
        std::fs::write(workspace.join("v8project.yaml"), "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n").unwrap();
        std::fs::write(source.join("Configuration.xml"), r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Configuration><Properties><Name>Store</Name><DefaultLanguage>Language.Русский</DefaultLanguage></Properties><ChildObjects><Language>Русский</Language><Document>Order</Document><Document>Other</Document></ChildObjects></Configuration></MetaDataObject>"#).unwrap();
        std::fs::write(source.join("Languages/Русский.xml"), r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Language uuid="33333333-3333-4333-8333-333333333333"><Properties><Name>Русский</Name><Comment/><LanguageCode>ru</LanguageCode></Properties></Language></MetaDataObject>"#).unwrap();
        std::fs::write(source.join("Documents/Other.xml"), r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Document uuid="44444444-4444-4444-8444-444444444444"><Properties><Name>Other</Name><Comment /></Properties><ChildObjects/></Document></MetaDataObject>"#).unwrap();
        std::fs::write(source.join("Adjacent/opaque.bin"), b"\0\xffunrelated\r\n").unwrap();
        std::fs::write(source.join(OWNER), owner).unwrap();
        if existing_cache {
            let cache = workspace.join(".build/unica");
            std::fs::create_dir_all(cache.join("untouched-empty")).unwrap();
            std::fs::write(cache.join("unrelated.bin"), b"\0\xffcache\r\n").unwrap();
        }
        Self {
            _root: root,
            workspace,
            runtime: V5CanonicalInvocationRuntime::new(
                Arc::new(
                    crate::infrastructure::daemon::v13_service::CanonicalV13ReadService::default(),
                ),
                Arc::new(TokioClock),
            ),
        }
    }

    fn source(&self) -> PathBuf {
        self.workspace.join("src")
    }

    fn call(
        &self,
        tool: ToolIdentity,
        arguments: Value,
    ) -> unica_execution_contract::invocation::DomainResult {
        direct_v5(
            &self.runtime,
            InvocationRequest::new(tool, arguments, self.workspace.to_string_lossy(), 7_000)
                .unwrap(),
        )
        .unwrap()
    }

    fn preview(&self, ops: Value) -> unica_execution_contract::invocation::DomainResult {
        let before = snapshot(&self.workspace);
        let result = self.call(ToolIdentity::Apply, json!({"at":AT,"ops":ops}));
        assert_eq!(
            snapshot(&self.workspace),
            before,
            "preview changed source/cache files or directory membership"
        );
        result
    }

    fn publish(&self, ops: Value, names: &[&str]) -> Vec<u8> {
        let before = tree_snapshot(&self.source());
        let preview = self.preview(ops);
        assert!(
            preview.ok,
            "fixture must reach canonical publication: {preview:?}"
        );
        let data = preview.data.as_ref().unwrap();
        assert_eq!(data["mode"], "preview");
        assert!(data["executionToken"].is_string(), "{preview:?}");
        let published = self.call(
            ToolIdentity::Apply,
            json!({"executionToken":data["executionToken"]}),
        );
        assert!(published.ok, "canonical publication failed: {published:?}");
        assert_eq!(
            data["planHash"],
            published.data.as_ref().unwrap()["planHash"]
        );
        let after = tree_snapshot(&self.source());
        let mut expected_paths = before.keys().cloned().collect::<BTreeSet<_>>();
        for name in names {
            for relative in [
                format!("Documents/Order/Forms/{name}.xml"),
                format!("Documents/Order/Forms/{name}/Ext/Form.xml"),
                format!("Documents/Order/Forms/{name}/Ext/Form/Module.bsl"),
            ] {
                assert!(!before.contains_key(Path::new(&relative)));
                expected_paths.insert(PathBuf::from(relative));
            }
        }
        assert_eq!(
            after.keys().cloned().collect::<BTreeSet<_>>(),
            expected_paths,
            "only the requested form source files may be created"
        );
        for (path, bytes) in &before {
            if path != Path::new(OWNER) {
                assert_eq!(
                    &after[path], bytes,
                    "form.add changed unrelated source {path:?}"
                );
            }
        }
        for name in names {
            let view = self.call(
                ToolIdentity::View,
                json!({"at":format!("{AT}.Form.{name}")}),
            );
            assert!(view.ok, "new form must be readable: {view:?}");
            assert_eq!(
                view.at.as_deref(),
                Some(format!("{AT}.Form.{name}").as_str())
            );
        }
        after[Path::new(OWNER)].clone()
    }
}

fn snapshot(root: &Path) -> (BTreeMap<PathBuf, Vec<u8>>, BTreeSet<PathBuf>) {
    fn directories(root: &Path, current: &Path, result: &mut BTreeSet<PathBuf>) {
        for entry in std::fs::read_dir(current).unwrap() {
            let entry = entry.unwrap();
            if entry.file_type().unwrap().is_dir() {
                let path = entry.path();
                result.insert(path.strip_prefix(root).unwrap().to_path_buf());
                directories(root, &path, result);
            }
        }
    }
    let mut found = BTreeSet::new();
    directories(root, root, &mut found);
    (tree_snapshot(root), found)
}

fn form_ops(names: &[&str], separate_ops: bool) -> Value {
    let items = names
        .iter()
        .map(|name| json!({"name":name,"type":"ObjectForm"}))
        .collect::<Vec<_>>();
    if separate_ops {
        Value::Array(
            items
                .into_iter()
                .map(|item| json!({"op":"form.add","args":{"items":[item]}}))
                .collect(),
        )
    } else {
        json!([{"op":"form.add","args":{"items":items}}])
    }
}

fn strip_exact_prefix<'a>(actual: &'a [u8], expected: &[u8], label: &str) -> &'a [u8] {
    actual.strip_prefix(expected).unwrap_or_else(|| {
        let first_difference = actual.iter().zip(expected).position(|(a, b)| a != b).unwrap_or(actual.len().min(expected.len()));
        panic!("{label} changed outside the edit at byte {first_difference}; expected prefix length {}", expected.len())
    })
}

fn assert_owner_edits(before: &[u8], after: &[u8], names: &[&str], default: bool) {
    let text = std::str::from_utf8(before).unwrap();
    assert_eq!(text.matches(INSERTION_SUFFIX).count(), 1);
    let insertion = text.find(INSERTION_SUFFIX).unwrap();
    let mut rest = after;
    if default {
        assert_eq!(text.matches(DEFAULT_SLOT).count(), 1);
        let slot = text.find(DEFAULT_SLOT).unwrap();
        assert!(slot < insertion);
        rest = strip_exact_prefix(rest, &before[..slot], "bytes before default slot");
        let replacement = format!(
            "<DefaultObjectForm>Document.Order.Form.{}</DefaultObjectForm>",
            names[0]
        );
        rest = strip_exact_prefix(
            rest,
            replacement.as_bytes(),
            "only the empty default slot may change",
        );
        rest = strip_exact_prefix(
            rest,
            &before[slot + DEFAULT_SLOT.len()..insertion],
            "untouched bytes between default and registration",
        );
    } else {
        rest = strip_exact_prefix(
            rest,
            &before[..insertion],
            "bytes before registration insertion",
        );
    }
    let gap = rest
        .strip_suffix(&before[insertion..])
        .expect("bytes after insertion, including final newline, must stay exact");
    // Only inserted form refs/local whitespace may occupy this gap. The
    // original indentation and surrounding newlines are in unchanged slices.
    let wrapper = format!(
        "<Root xmlns='{MD}'>{}</Root>",
        std::str::from_utf8(gap).unwrap()
    );
    let document = roxmltree::Document::parse(&wrapper).unwrap();
    let mut observed = Vec::new();
    for node in document.root_element().children() {
        if node.is_text() {
            assert!(node
                .text()
                .unwrap()
                .bytes()
                .all(|byte| matches!(byte, b' ' | b'\t' | b'\r' | b'\n')));
        } else {
            assert!(
                node.has_tag_name((MD, "Form")),
                "unexpected content in inserted gap: {wrapper}"
            );
            assert_eq!(node.attributes().len(), 0);
            assert!(!node.children().any(|child| child.is_element()));
            observed.push(node.text().unwrap().to_owned());
        }
    }
    assert_eq!(
        observed, names,
        "the insertion must contain exactly the requested form references"
    );
    let full = std::str::from_utf8(after)
        .unwrap()
        .trim_start_matches('\u{feff}');
    let parsed = roxmltree::Document::parse(full).unwrap();
    let owner = parsed
        .root_element()
        .children()
        .find(|node| node.has_tag_name((MD, "Document")))
        .unwrap();
    let children = owner
        .children()
        .find(|node| node.has_tag_name((MD, "ChildObjects")))
        .unwrap();
    let forms = children
        .children()
        .filter(|node| node.has_tag_name((MD, "Form")))
        .map(|node| node.text().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(forms, names);
}

fn preservation_case(bom: bool, endings: Endings, final_newline: bool) {
    let before = authored_owner(bom, endings, final_newline, false);
    let fixture = Fixture::new(&before, bom);
    let after = fixture.publish(form_ops(&["Main"], false), &["Main"]);
    assert_owner_edits(&before, &after, &["Main"], false);
}

macro_rules! preservation_case {
    ($name:ident, $bom:expr, $eol:ident, $final:expr) => {
        #[test]
        fn $name() {
            preservation_case($bom, Endings::$eol, $final);
        }
    };
}

preservation_case!(canonical_form_add_owner_plain_lf_final, false, Lf, true);
preservation_case!(canonical_form_add_owner_plain_lf_no_final, false, Lf, false);
preservation_case!(canonical_form_add_owner_plain_crlf_final, false, CrLf, true);
preservation_case!(
    canonical_form_add_owner_plain_crlf_no_final,
    false,
    CrLf,
    false
);
preservation_case!(
    canonical_form_add_owner_plain_mixed_final,
    false,
    Mixed,
    true
);
preservation_case!(
    canonical_form_add_owner_plain_mixed_no_final,
    false,
    Mixed,
    false
);
preservation_case!(canonical_form_add_owner_bom_lf_final, true, Lf, true);
preservation_case!(canonical_form_add_owner_bom_lf_no_final, true, Lf, false);
preservation_case!(canonical_form_add_owner_bom_crlf_final, true, CrLf, true);
preservation_case!(
    canonical_form_add_owner_bom_crlf_no_final,
    true,
    CrLf,
    false
);
preservation_case!(canonical_form_add_owner_bom_mixed_final, true, Mixed, true);
preservation_case!(
    canonical_form_add_owner_bom_mixed_no_final,
    true,
    Mixed,
    false
);

#[test]
fn canonical_form_add_two_items_preserve_existing_owner_bytes() {
    let before = authored_owner(true, Endings::Mixed, false, false);
    let fixture = Fixture::new(&before, false);
    let after = fixture.publish(form_ops(&["First", "Second"], false), &["First", "Second"]);
    assert_owner_edits(&before, &after, &["First", "Second"], false);
}

#[test]
fn canonical_form_add_two_operations_preserve_existing_owner_bytes() {
    let before = authored_owner(true, Endings::Mixed, true, false);
    let fixture = Fixture::new(&before, true);
    let after = fixture.publish(form_ops(&["First", "Second"], true), &["First", "Second"]);
    assert_owner_edits(&before, &after, &["First", "Second"], false);
}

#[test]
fn canonical_form_add_keeps_mixed_bytes_between_default_and_registration_edits() {
    let before = authored_owner(true, Endings::Mixed, false, true);
    let fixture = Fixture::new(&before, true);
    let after = fixture.publish(form_ops(&["Main"], false), &["Main"]);
    assert_owner_edits(&before, &after, &["Main"], true);
}

#[test]
fn canonical_form_add_refuses_malformed_owner_without_source_or_cache_writes() {
    let fixture = Fixture::new(b"<MetaDataObject", true);
    let refused = fixture.preview(form_ops(&["Main"], false));
    assert!(!refused.ok, "{refused:?}");
    assert_eq!(refused.diagnostics[0]["code"], "bad_value", "{refused:?}");
    assert_eq!(refused.diagnostics[0]["outcome"], "fixCall", "{refused:?}");
    let cause = refused.diagnostics[0]["message"].as_str().unwrap();
    assert!(
        cause.contains("XML parse error") && cause.contains("root node"),
        "an unrelated bad_value is not proof of malformed-source refusal: {refused:?}"
    );
    assert!(refused.changed.is_empty());
    assert!(refused
        .data
        .as_ref()
        .is_none_or(|data| data.get("executionToken").is_none()));
}

#[test]
fn canonical_form_add_refuses_old_owner_profile_without_source_or_cache_writes() {
    let owner = String::from_utf8(authored_owner(true, Endings::CrLf, false, false))
        .unwrap()
        .replace("version=\"2.20\"", "version=\"2.19\"");
    let fixture = Fixture::new(owner.as_bytes(), false);
    let refused = fixture.preview(form_ops(&["Main"], false));
    assert!(!refused.ok, "{refused:?}");
    assert_eq!(
        refused.diagnostics[0]["code"], "invalid_source",
        "{refused:?}"
    );
    assert!(refused.changed.is_empty());
    assert!(refused
        .data
        .as_ref()
        .is_none_or(|data| data.get("executionToken").is_none()));
}

#[test]
fn canonical_form_add_saved_plan_preserves_external_owner_change() {
    let before = authored_owner(true, Endings::CrLf, false, false);
    let fixture = Fixture::new(&before, true);
    let preview = fixture.preview(form_ops(&["Main"], false));
    assert!(preview.ok, "{preview:?}");
    let external = String::from_utf8(before)
        .unwrap()
        .replace("<Comment />", "<Comment>external edit</Comment>");
    std::fs::write(fixture.source().join(OWNER), external).unwrap();
    let after_external = snapshot(&fixture.source());
    let refused = fixture.call(
        ToolIdentity::Apply,
        json!({"executionToken":preview.data.as_ref().unwrap()["executionToken"]}),
    );
    assert!(!refused.ok, "{refused:?}");
    assert_eq!(
        refused.diagnostics[0]["code"], "stale_revision",
        "{refused:?}"
    );
    assert!(refused.changed.is_empty());
    assert_eq!(
        snapshot(&fixture.source()),
        after_external,
        "saved plan overwrote external bytes or published form sources"
    );
}

fn prefixed_owner(default: &str, children: &str) -> String {
    format!(
        "\u{feff}<?xml version='1.0' encoding='UTF-8'?>\r\n<md:MetaDataObject xmlns:md='{MD}' xmlns='urn:foreign' version='2.20'><md:Document uuid='11111111-1111-4111-8111-111111111111'><md:Properties><md:Name>Order</md:Name><md:Comment />{default}</md:Properties>\r<!-- keep between edits -->\n{children}</md:Document></md:MetaDataObject>"
    )
}

#[test]
fn canonical_form_add_splices_prefixed_self_closing_container_and_default() {
    let default = "<md:DefaultObjectForm xmlns:keep='urn:kept' />";
    let children = "<md:ChildObjects xmlns:keep='urn:kept' />";
    let before = prefixed_owner(default, children);
    let fixture = Fixture::new(before.as_bytes(), true);
    let after = fixture.publish(form_ops(&["Main"], false), &["Main"]);
    let expected = before
        .replace(default, "<md:DefaultObjectForm xmlns:keep='urn:kept' >Document.Order.Form.Main</md:DefaultObjectForm>")
        .replace(children, "<md:ChildObjects xmlns:keep='urn:kept' ><md:Form>Main</md:Form></md:ChildObjects>");
    assert_eq!(after, expected.as_bytes());
}

#[test]
fn canonical_form_add_splices_prefixed_expanded_nodes_without_touching_foreign_namesakes() {
    let default = "<DefaultObjectForm/><md:DefaultObjectForm> \r\n\t</md:DefaultObjectForm>";
    let children = "<md:ChildObjects><Template>foreign &amp; untouched</Template><md:TabularSection uuid='22222222-2222-4222-8222-222222222222'><md:Properties><md:Name>Rows</md:Name></md:Properties><md:ChildObjects/></md:TabularSection></md:ChildObjects>";
    let before = prefixed_owner(default, children);
    let fixture = Fixture::new(before.as_bytes(), false);
    let after = fixture.publish(form_ops(&["Main"], false), &["Main"]);
    let expected = before
        .replace(
            "<md:DefaultObjectForm> \r\n\t</md:DefaultObjectForm>",
            "<md:DefaultObjectForm>Document.Order.Form.Main</md:DefaultObjectForm>",
        )
        .replace(
            "<md:TabularSection uuid=",
            "<md:Form>Main</md:Form><md:TabularSection uuid=",
        );
    assert_eq!(after, expected.as_bytes());
}

#[test]
fn canonical_form_add_keeps_nonempty_default_and_foreign_empty_default() {
    let before = String::from_utf8(authored_owner(true, Endings::Mixed, false, true))
        .unwrap()
        .replace(DEFAULT_SLOT, "<DefaultObjectForm xmlns='urn:foreign'/><DefaultObjectForm>Document.Order.Form.Existing</DefaultObjectForm>");
    let fixture = Fixture::new(before.as_bytes(), false);
    let after = fixture.publish(form_ops(&["Main"], false), &["Main"]);
    assert_owner_edits(before.as_bytes(), &after, &["Main"], false);
}

#[test]
fn canonical_form_add_keeps_comment_and_processing_instruction_in_default() {
    let before = String::from_utf8(authored_owner(false, Endings::Mixed, true, true))
        .unwrap()
        .replace(DEFAULT_SLOT, "<DefaultObjectForm><!-- preserve empty-looking slot --><?keep default='yes'?></DefaultObjectForm>");
    let fixture = Fixture::new(before.as_bytes(), true);
    let after = fixture.publish(form_ops(&["Main"], false), &["Main"]);
    assert_owner_edits(before.as_bytes(), &after, &["Main"], false);
}

fn assert_owner_structure_refused(owner: &str, cause: &str) {
    let fixture = Fixture::new(owner.as_bytes(), true);
    let refused = fixture.preview(form_ops(&["Main"], false));
    assert!(!refused.ok, "{refused:?}");
    assert!(
        refused
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic["message"]
                .as_str()
                .is_some_and(|message| message.contains(cause))),
        "refusal must identify {cause}: {refused:?}"
    );
    assert!(refused.changed.is_empty());
    assert!(refused
        .data
        .as_ref()
        .is_none_or(|data| data.get("executionToken").is_none()));
}

#[test]
fn canonical_form_add_refuses_missing_childobjects_without_orphan_files() {
    assert_owner_structure_refused(&prefixed_owner("", ""), "ChildObjects");
}

#[test]
fn canonical_form_add_refuses_ambiguous_childobjects_without_orphan_files() {
    assert_owner_structure_refused(
        &prefixed_owner("", "<md:ChildObjects/><md:ChildObjects/>"),
        "ChildObjects",
    );
}

#[test]
fn canonical_form_add_refuses_prefixed_entity_spelled_duplicate_registration() {
    assert_owner_structure_refused(
        &prefixed_owner(
            "",
            "<md:ChildObjects><md:Form>M&#97;in</md:Form></md:ChildObjects>",
        ),
        "already exists",
    );
}

#[test]
fn canonical_form_add_refuses_prefixed_legacy_inline_duplicate_registration() {
    assert_owner_structure_refused(
        &prefixed_owner("", "<md:ChildObjects><md:Form uuid='22222222-2222-4222-8222-222222222222'><md:Properties><md:Name>M&#97;in</md:Name><md:Comment /></md:Properties></md:Form></md:ChildObjects>"),
        "already exists",
    );
}

// The inline form has no standalone descriptor here, so the #1370 collapse
// of an edited owner leaves it as it is.
#[test]
fn canonical_form_add_preserves_unrelated_legacy_inline_registration() {
    let old_form = "<md:Form uuid='22222222-2222-4222-8222-222222222222'><md:Properties><md:Name>Existing</md:Name><md:Comment /></md:Properties></md:Form>";
    let before = prefixed_owner(
        "",
        &format!("<md:ChildObjects>{old_form}</md:ChildObjects>"),
    );
    let fixture = Fixture::new(before.as_bytes(), false);
    let after = fixture.publish(form_ops(&["Main"], false), &["Main"]);
    let expected = before.replace(
        "</md:ChildObjects>",
        "<md:Form>Main</md:Form></md:ChildObjects>",
    );
    assert_eq!(after, expected.as_bytes());
}
