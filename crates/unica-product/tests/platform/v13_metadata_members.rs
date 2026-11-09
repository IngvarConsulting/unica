use super::*;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

struct MemberCase {
    family: &'static str,
    leaf: &'static str,
    owner: &'static str,
    file: &'static str,
    item: Value,
}

fn cases() -> Vec<MemberCase> {
    vec![
        MemberCase {
            family: "tabularSection",
            leaf: "TabularSection",
            owner: "Catalog.Items",
            file: "Catalogs/Items.xml",
            item: json!({"name":"NewSection", "attributes":[{"name":"Amount", "type":{"variants":[{"kind":"number","digits":10,"fraction":2,"sign":"any"}]}}]}),
        },
        MemberCase {
            family: "dimension",
            leaf: "Dimension",
            owner: "InformationRegister.CorpusRegister",
            file: "InformationRegisters/CorpusRegister.xml",
            item: json!({"name":"NewDimension","type":{"variants":[{"kind":"string","length":20,"allowedLength":"variable"}]}}),
        },
        MemberCase {
            family: "resource",
            leaf: "Resource",
            owner: "InformationRegister.CorpusRegister",
            file: "InformationRegisters/CorpusRegister.xml",
            item: json!({"name":"NewResource","type":{"variants":[{"kind":"number","digits":10,"fraction":2,"sign":"any"}]}}),
        },
        MemberCase {
            family: "enumValue",
            leaf: "EnumValue",
            owner: "Enum.LanguageAware",
            file: "Enums/LanguageAware.xml",
            item: json!({"name":"NewValue"}),
        },
        MemberCase {
            family: "column",
            leaf: "Column",
            owner: "DocumentJournal.ДокументыОрганизации",
            file: "DocumentJournals/ДокументыОрганизации.xml",
            item: json!({"name":"NewColumn"}),
        },
    ]
}

fn fixture() -> tempfile::TempDir {
    let root = tempfile::tempdir().unwrap();
    copy_tree(
        &Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/acceptance/workspace-metadata-members"),
        root.path(),
    );
    root
}

fn snapshot(root: &Path) -> BTreeMap<PathBuf, Option<Vec<u8>>> {
    let mut pending = vec![root.to_path_buf()];
    let mut observed = BTreeMap::new();
    while let Some(path) = pending.pop() {
        if path.is_dir() {
            observed.insert(path.strip_prefix(root).unwrap().to_path_buf(), None);
            pending.extend(
                std::fs::read_dir(&path)
                    .unwrap()
                    .map(|entry| entry.unwrap().path()),
            );
        } else {
            observed.insert(
                path.strip_prefix(root).unwrap().to_path_buf(),
                Some(std::fs::read(path).unwrap()),
            );
        }
    }
    observed
}

fn preview(mcp: &mut McpProcess, case: &MemberCase, id: u64, verb: &str, args: Value) -> Value {
    mcp.completed_tool_call(call_tool(
        id,
        "unica.apply",
        json!({
            "at":format!("main:{}",case.owner),
            "ops":[{"op":format!("{}.{}",case.family,verb), "args":args}]
        }),
    ))
}

fn execute(mcp: &mut McpProcess, id: u64, plan: &Value) -> Value {
    assert_eq!(plan["ok"], true, "{plan:#}");
    mcp.completed_tool_call(call_tool(
        id,
        "unica.apply",
        json!({"executionToken":plan["data"]["executionToken"]}),
    ))
}

fn assert_metadata_event(result: &Value) {
    assert_eq!(result["ok"], true, "{result:#}");
    assert_eq!(
        result["data"]["cache"]["events"],
        json!(["MetadataChanged"]),
        "{result:#}"
    );
}

fn member_range(text: &str, leaf: &str, name: &str) -> std::ops::Range<usize> {
    let body = text.trim_start_matches('\u{feff}');
    let shift = text.len() - body.len();
    let doc = roxmltree::Document::parse(body).unwrap();
    let member = doc
        .descendants()
        .find(|node| {
            node.is_element()
                && node.tag_name().name() == leaf
                && node.children().any(|props| {
                    props.is_element()
                        && props.tag_name().name() == "Properties"
                        && props.children().any(|child| {
                            child.is_element()
                                && child.tag_name().name() == "Name"
                                && child.text() == Some(name)
                        })
                })
        })
        .unwrap_or_else(|| panic!("{leaf}.{name} is absent"));
    let range = member.range();
    shift + range.start..shift + range.end
}

#[test]
fn public_metadata_members_keep_new_identity_neighbors_and_exact_rename_bytes() {
    for case in cases() {
        let root = fixture();
        let source = root.path().join("src");
        let owner = source.join(case.file);
        let original = snapshot(&source);
        let mut mcp = McpProcess::start(root.path());
        initialize_body_mcp(&mut mcp);
        let add = preview(&mut mcp, &case, 2, "add", json!({"items":[case.item]}));
        assert_eq!(snapshot(&source), original, "{} preview", case.family);
        assert_metadata_event(&execute(&mut mcp, 3, &add));
        let added = std::fs::read_to_string(&owner).unwrap();
        let after_add = snapshot(&source);
        assert_eq!(
            after_add.keys().collect::<Vec<_>>(),
            original.keys().collect::<Vec<_>>()
        );
        for (path, bytes) in &original {
            if path != Path::new(case.file) {
                assert_eq!(
                    after_add.get(path),
                    Some(bytes),
                    "{} add neighbor {path:?}",
                    case.family
                );
            }
        }
        let name = case.item["name"].as_str().unwrap();
        let range = member_range(&added, case.leaf, name);
        let new_name = format!("{name}Renamed");
        let before_node = &added[range.clone()];
        // Only the member name and its two generated type names may change.
        // Every UUID, generated TypeId/ValueId and nested attribute stays byte-identical.
        let renamed_node = before_node
            .replacen(
                &format!("<Name>{name}</Name>"),
                &format!("<Name>{new_name}</Name>"),
                1,
            )
            .replace(&format!(".{name}\""), &format!(".{new_name}\""));
        assert_ne!(before_node, renamed_node);
        let mut expected = added.clone();
        expected.replace_range(range, &renamed_node);
        let rename = preview(
            &mut mcp,
            &case,
            4,
            "set",
            json!({
                "at":format!("main:{}.{}.{name}",case.owner,case.leaf),
                "values":{"newName":new_name}
            }),
        );
        assert_eq!(
            snapshot(&source),
            after_add,
            "{} rename preview",
            case.family
        );
        assert_metadata_event(&execute(&mut mcp, 5, &rename));
        assert_eq!(
            std::fs::read(&owner).unwrap(),
            expected.as_bytes(),
            "{} exact rename",
            case.family
        );
        let mut expected_tree = after_add;
        expected_tree.insert(PathBuf::from(case.file), Some(expected.into_bytes()));
        assert_eq!(
            snapshot(&source),
            expected_tree,
            "{} neighbors",
            case.family
        );

        let remove = preview(
            &mut mcp,
            &case,
            6,
            "remove",
            json!({
                "at":format!("main:{}.{}.{new_name}",case.owner,case.leaf)
            }),
        );
        assert_eq!(
            snapshot(&source),
            expected_tree,
            "{} removal preview",
            case.family
        );
        assert_metadata_event(&execute(&mut mcp, 7, &remove));
        assert_eq!(
            snapshot(&source),
            original,
            "{} complete lifecycle",
            case.family
        );
        mcp.finish();
    }
}

#[test]
fn public_metadata_member_cancelled_batch_has_no_source_or_cache_effects() {
    for case in cases() {
        let root = fixture();
        let source = root.path().join("src");
        let original = snapshot(&source);
        let mut mcp = McpProcess::start(root.path());
        initialize_body_mcp(&mut mcp);
        let name = case.item["name"].as_str().unwrap();
        let plan = mcp.completed_tool_call(call_tool(2, "unica.apply", json!({
            "at":format!("main:{}",case.owner),
            "ops":[
                {"op":format!("{}.add",case.family),"args":{"items":[case.item]}},
                {"op":format!("{}.remove",case.family),"args":{"at":format!("main:{}.{}.{name}",case.owner,case.leaf)}}
            ]
        })));
        assert_eq!(plan["ok"], true, "{}: {plan:#}", case.family);
        assert_eq!(plan["data"]["effects"], 0, "{plan:#}");
        assert_eq!(plan["data"]["cache"]["events"], json!([]), "{plan:#}");
        assert_eq!(snapshot(&source), original, "{} no-op preview", case.family);

        let applied = execute(&mut mcp, 3, &plan);

        assert_eq!(applied["ok"], true, "{applied:#}");
        assert_eq!(applied["data"]["effects"], 0, "{applied:#}");
        assert_eq!(
            applied["data"]["cache"], plan["data"]["cache"],
            "{applied:#}"
        );
        assert_eq!(snapshot(&source), original, "{} no-op execute", case.family);
        mcp.finish();
    }
}

#[test]
fn public_metadata_member_saved_plan_preserves_a_concurrent_owner_edit() {
    for case in cases() {
        let root = fixture();
        let source = root.path().join("src");
        let owner = source.join(case.file);
        let mut mcp = McpProcess::start(root.path());
        initialize_body_mcp(&mut mcp);
        let plan = preview(&mut mcp, &case, 2, "add", json!({"items":[case.item]}));
        assert_eq!(plan["ok"], true, "{plan:#}");
        let mut changed = std::fs::read(&owner).unwrap();
        changed.extend_from_slice(b"\n<!-- concurrent owner bytes -->\n");
        std::fs::write(&owner, changed).unwrap();
        let foreign = snapshot(&source);

        let refused = execute(&mut mcp, 3, &plan);

        assert_eq!(refused["ok"], false, "{}: {refused:#}", case.family);
        assert_eq!(
            refused["diagnostics"][0]["code"], "stale_revision",
            "{}: {refused:#}",
            case.family
        );
        assert_eq!(
            snapshot(&source),
            foreign,
            "{} concurrent owner",
            case.family
        );
        mcp.finish();
    }
}
