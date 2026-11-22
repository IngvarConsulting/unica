use super::*;

const NATIVE_MODULE: &str = "main:CommonModule.EventHandlers";
const NATIVE_VARIABLES: &str = "main:CommonModule.EventHandlers.Variable";
const NATIVE_BSL: &str = "CommonModules/EventHandlers/Ext/Module.bsl";
const NATIVE_FORMATS: [&str; 2] = ["2.21", "2.22"];

pub(super) fn write_requirements(directory: &Path) {
    let mut lines = vec![
        required_branch(
            "2.20",
            "main:CommonModule.Probe.Variable",
            "tests/fixtures/acceptance/workspace-module-variables/src/CommonModules/Probe/Ext/Module.bsl",
        ),
        required_branch(
            "2.20",
            "main:Catalog.Команды.Form.ФормаСписка.Module.Form.Method.Step.Binding",
            "tests/fixtures/acceptance/workspace-notification-binding/src/Catalogs/Команды/Forms/ФормаСписка/Ext/Form/Module.bsl",
        ),
    ];
    for format in NATIVE_FORMATS {
        lines.push(required_branch(
            format,
            NATIVE_VARIABLES,
            &format!("tests/fixtures/formats/platform_xml/{format}/all_kinds/src/{NATIVE_BSL}"),
        ));
    }
    write_shard(&directory.join("required_bsl_projection_keys.jsonl"), lines);
}

fn required_branch(format: &str, at: &str, origin: &str) -> String {
    let address = parsed(at);
    let (branch, owner_segments) = address.segments().split_last().unwrap();
    assert!(branch.name().is_none());
    let owner = address.prefix(owner_segments.len());
    let line = export_format_line(format).unwrap().to_string();
    let key = evidence_key(
        &owner,
        branch.kind(),
        None,
        &line,
        SourceSetKind::Configuration,
    );
    json!({
        "schema_version": 1,
        "record_type": "required",
        "key": key,
        "requirement": {
            "kind": "exact",
            "observations": [{
                "owner": owner.to_string(),
                "origin": origin,
                "addresses": [at],
                "level": "read",
                "test_case": EVIDENCE_TEST_CASE,
            }],
        },
    })
    .to_string()
}

pub(super) fn check_native_empty_variables(evidence: &mut EvidenceRows) {
    const NAME: &str = "EventHandlers";
    for format in NATIVE_FORMATS {
        let fixture = fixture_with_profile(
            &format!("../formats/platform_xml/{format}/all_kinds"),
            unica_workspace::source_profile::platform_xml_source_profile,
        );
        let configuration = fixture_xml(&fixture, NATIVE_MODULE, "Configuration.xml");
        let configuration =
            roxmltree::Document::parse(configuration.trim_start_matches('\u{feff}')).unwrap();
        let root = configuration.root_element();
        assert_eq!(root.attribute("version"), Some(format));
        let registered = metadata_child(metadata_child(root, "Configuration"), "ChildObjects");
        assert_eq!(
            registered
                .children()
                .filter(|node| node.has_tag_name((MD_CLASSES_NS, "CommonModule")))
                .filter(|node| node.text() == Some(NAME))
                .count(),
            1
        );
        let descriptor = fixture_xml(&fixture, NATIVE_MODULE, "CommonModules/EventHandlers.xml");
        let descriptor =
            roxmltree::Document::parse(descriptor.trim_start_matches('\u{feff}')).unwrap();
        let root = descriptor.root_element();
        assert_eq!(root.attribute("version"), Some(format));
        check_metadata_identity(root, "CommonModule", NAME);

        let raw = fixture_xml(&fixture, NATIVE_MODULE, NATIVE_BSL);
        let raw = raw.trim_start_matches('\u{feff}');
        assert!(raw.lines().any(|line| line == "Procedure Run() Export"));
        assert!(!raw.lines().any(|line| {
            line.split_whitespace()
                .next()
                .is_some_and(|token| matches!(token.to_lowercase().as_str(), "var" | "перем"))
        }));

        let runtime = V5CanonicalInvocationRuntime::new(
            Arc::new(CanonicalV13ReadService::default()),
            Arc::new(TokioClock),
        );
        let (owner, _) = read_view(&runtime, &fixture.workspace, NATIVE_MODULE);
        assert_eq!(owner["kind"], "Module");
        assert_eq!(owner["props"]["ownerKind"], "CommonModule");
        assert_eq!(
            published_branch_counts(&owner, NATIVE_VARIABLES).collect::<Vec<_>>(),
            [0]
        );
        let (collection, rows) = read_view(&runtime, &fixture.workspace, NATIVE_VARIABLES);
        assert_eq!(collection["kind"], "Variable");
        assert_eq!(collection["items"], json!([]));
        assert!(rows.is_empty());
        check_refusal(
            &runtime,
            &fixture.workspace,
            &format!("{NATIVE_VARIABLES}.Invented"),
            "not_found",
        );
        evidence.record_fixture_evidence(
            &fixture,
            &format!("tests/fixtures/formats/platform_xml/{format}/all_kinds"),
            NATIVE_BSL,
            Evidence::Read {
                at: NATIVE_VARIABLES,
                external_owner_descriptor: None,
            },
        );
    }
}

pub(super) fn check_refusal(
    runtime: &V5CanonicalInvocationRuntime,
    workspace: &Path,
    at: &str,
    code: &str,
) {
    let response = invoke_on(runtime, workspace, ToolIdentity::View, json!({"at": at}));
    assert!(!response.ok, "{at}: {response:?}");
    assert_eq!(response.diagnostics[0]["code"], code, "{at}: {response:?}");
}
