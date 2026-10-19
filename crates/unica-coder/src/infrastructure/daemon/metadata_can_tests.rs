use super::actor_capacity_tests::direct_v5;
use super::*;
use crate::application::invocation_store::ToolIdentity;
use crate::domain::apply::ApplyRequest;
use crate::domain::invocation::DomainResult;
use crate::domain::project_sources::{SourceFormat, SourceProfile, SourceSetKind};
use crate::domain::workspace::WorkspaceContext;
use crate::infrastructure::native_operations::apply::ApplyPlanError;
use crate::infrastructure::native_operations::apply_families::parse_family_operation_for_test;
use crate::infrastructure::workspace_actor::{
    ProviderRootBinding, WorkspaceActor, WorkspaceIdentity, WorkspaceSourceSetInput,
};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

type SourceImage = BTreeMap<PathBuf, Vec<u8>>;

struct Fixture {
    _root: tempfile::TempDir,
    workspace: PathBuf,
    source: PathBuf,
}

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_owned()
}

fn copy_image(from: &Path, to: &Path) {
    for (relative, bytes) in crate::test_support::tree_snapshot(from) {
        let path = to.join(relative);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, bytes).unwrap();
    }
}

impl Fixture {
    fn new(name: &str) -> Self {
        let root = tempfile::tempdir().unwrap();
        let workspace = crate::test_support::canonical_path(root.path()).join("workspace");
        copy_image(
            &repo_root().join("tests/fixtures/acceptance").join(name),
            &workspace,
        );
        Self {
            _root: root,
            source: workspace.join("src"),
            workspace,
        }
    }

    fn with_extension() -> Self {
        let fixture = Self::new("workspace");
        copy_image(
            &repo_root().join("tests/fixtures/acceptance/workspace-format/src-ext"),
            &fixture.workspace.join("src-ext"),
        );
        std::fs::write(
            fixture.workspace.join("v8project.yaml"),
            "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n  - name: ext\n    type: EXTENSION\n    path: src-ext\n",
        )
        .unwrap();
        fixture
    }

    fn binding(&self, extension: bool) -> ProviderRootBinding {
        let (name, kind, source) = if extension {
            (
                "ext",
                SourceSetKind::Extension,
                self.workspace.join("src-ext"),
            )
        } else {
            ("main", SourceSetKind::Configuration, self.source.clone())
        };
        let context = WorkspaceContext {
            cwd: self.workspace.clone(),
            workspace_root: self.workspace.clone(),
            cache_root: self.workspace.join(".build/parser-test"),
            workspace_epoch: 1,
        };
        let identity = WorkspaceIdentity::new(
            &context,
            [WorkspaceSourceSetInput::new(
                name,
                &source,
                kind,
                SourceFormat::PlatformXml,
                SourceProfile::platform_xml_8_3_27_format_2_20(),
            )],
            "metadata-can-parser-test",
        )
        .unwrap();
        WorkspaceActor::new(identity, context)
            .unwrap()
            .bind_provider_root(name, &source)
            .unwrap()
    }
}

fn runtime() -> V5CanonicalInvocationRuntime {
    V5CanonicalInvocationRuntime::new(
        Arc::new(crate::infrastructure::daemon::v13_service::CanonicalV13ReadService::default()),
        Arc::new(TokioClock),
    )
}

fn call(
    runtime: &V5CanonicalInvocationRuntime,
    fixture: &Fixture,
    tool: ToolIdentity,
    args: Value,
) -> DomainResult {
    direct_v5(
        runtime,
        InvocationRequest::new(tool, args, fixture.workspace.to_string_lossy(), 7_000).unwrap(),
    )
    .unwrap()
}

fn contract(
    runtime: &V5CanonicalInvocationRuntime,
    fixture: &Fixture,
    at: &str,
    op: &str,
) -> Value {
    let detailed = call(
        runtime,
        fixture,
        ToolIdentity::View,
        json!({"at":format!("{at}.Can.{op}")}),
    );
    assert!(detailed.ok, "{at}.Can.{op}: {detailed:?}");
    let data = detailed.data.unwrap();
    assert_eq!(data["kind"], "Can");
    assert_eq!(data["props"]["op"], op);
    assert_eq!(data["props"]["implemented"], true);
    let contract = data["props"]["contract"].clone();
    assert_eq!(contract["argsSchema"]["additionalProperties"], false);
    assert!(contract["exampleArgs"].get("at").is_none());
    contract
}

fn parse(
    binding: &ProviderRootBinding,
    at: &str,
    op: &str,
    args: &Value,
) -> Result<(), ApplyPlanError> {
    let input = json!({"at":at,"ops":[{"op":op,"args":args}],"dryRun":true});
    let request = ApplyRequest::parse(input.as_object().unwrap(), &["main", "ext"]).unwrap();
    parse_family_operation_for_test(3, &request.ops()[0], binding)
}

fn assert_parity(
    schema: &Value,
    binding: &ProviderRootBinding,
    at: &str,
    op: &str,
    samples: &[(Value, bool)],
) {
    let oracle =
        jsonschema::validator_for(schema).expect("published Can schema is valid JSON Schema");
    for (args, expected) in samples {
        assert_eq!(
            oracle.is_valid(args),
            *expected,
            "schema {op} at {at}: {args}"
        );
        let parsed = parse(binding, at, op, args);
        assert_eq!(
            parsed.is_ok(),
            *expected,
            "productive parser {op} at {at}: {args}: {parsed:?}"
        );
    }
}

fn assert_props_can_and_preview(at: &str, preview_args: Value, samples: &[(Value, bool)]) {
    let fixture = Fixture::new("workspace");
    let runtime = runtime();
    let before = crate::test_support::tree_snapshot(&fixture.source);
    let project_before = std::fs::read(fixture.workspace.join("v8project.yaml")).unwrap();
    let operation_at = format!("{at}.Can.props.set");
    let detailed = call(
        &runtime,
        &fixture,
        ToolIdentity::View,
        json!({"at":operation_at}),
    );
    assert!(detailed.ok, "{operation_at}: {detailed:?}");
    assert_eq!(detailed.at.as_deref(), Some(operation_at.as_str()));
    let data = detailed.data.unwrap();
    assert_eq!(data["at"], operation_at);
    assert_eq!(data["kind"], "Can");
    assert_eq!(data["props"]["op"], "props.set");
    assert_eq!(data["props"]["implemented"], true);
    let contract = &data["props"]["contract"];
    let schema = &contract["argsSchema"];
    assert_eq!(schema["additionalProperties"], false);
    assert_eq!(
        schema["properties"]["values"]["additionalProperties"],
        false
    );
    assert!(contract["exampleArgs"].get("at").is_none());
    let oracle = jsonschema::validator_for(schema).expect("Props Can publishes valid JSON Schema");
    assert!(oracle.is_valid(&preview_args), "{at}: {preview_args}");
    for (args, expected) in samples {
        assert_eq!(oracle.is_valid(args), *expected, "{at}: {args}");
    }
    for args in [
        json!({}),
        json!({"values":null}),
        json!({"values":[]}),
        json!({"values":{}}),
        json!({"values":{"Comment":null}}),
        json!({"values":{"Comment":false}}),
        json!({"values":{"UnknownProperty":"x"}}),
        json!({"values":{"Comment":"x"},"unknown":true}),
        json!({"values":{"Comment":"x"},"at":at}),
    ] {
        assert!(!oracle.is_valid(&args), "{at}: {args}");
    }
    assert_eq!(crate::test_support::tree_snapshot(&fixture.source), before);
    assert_eq!(
        std::fs::read(fixture.workspace.join("v8project.yaml")).unwrap(),
        project_before
    );

    let preview = call(
        &runtime,
        &fixture,
        ToolIdentity::Apply,
        json!({"at":at,"ops":[{"op":"props.set","args":preview_args}]}),
    );
    assert!(preview.ok, "real props.set preview at {at}: {preview:?}");
    let planned = preview.data.unwrap();
    assert_eq!(planned["mode"], "preview");
    assert!(planned["executionToken"]
        .as_str()
        .is_some_and(|token| !token.is_empty()));
    assert!(planned["effects"]
        .as_u64()
        .is_some_and(|effects| effects > 0));
    assert_eq!(crate::test_support::tree_snapshot(&fixture.source), before);
    assert_eq!(
        std::fs::read(fixture.workspace.join("v8project.yaml")).unwrap(),
        project_before
    );
}

#[test]
fn props_set_common_module_can_is_contextual_and_previews_without_writes() {
    let at = "main:CommonModule.GoogleПереводчик";
    assert_eq!(
        crate::domain::logical_route::classify_route(
            &crate::domain::address::QualifiedAddress::parse(at).unwrap(),
            crate::domain::platform_profile::PlatformProfile::v8_3_27(),
        ),
        Some(crate::domain::logical_route::RouteClass::Module)
    );
    assert_props_can_and_preview(
        at,
        json!({"values":{"Comment":"Properties CommonModule preview","ClientOrdinaryApplication":false}}),
        &[
            (json!({"values":{"Comment":"","Synonym":""}}), true),
            (json!({"values":{"ClientOrdinaryApplication":true}}), true),
            (json!({"values":{"ClientOrdinaryApplication":false}}), true),
            (
                json!({"values":{"ReturnValuesReuse":"DuringRequest"}}),
                true,
            ),
            (
                json!({"values":{"ClientOrdinaryApplication":"false"}}),
                false,
            ),
            (json!({"values":{"ReturnValuesReuse":"Unknown"}}), false),
            (
                json!({"values":{"comment":"lowercase is not a Metadata alias"}}),
                false,
            ),
            (json!({"values":{"IncludeInCommandInterface":false}}), false),
            (
                json!({"values":{"Type":{"variants":[{"kind":"boolean"}]}}}),
                false,
            ),
        ],
    );
}

#[test]
fn props_set_role_can_is_contextual_and_previews_without_writes() {
    assert_props_can_and_preview(
        "main:Role.SalesReader",
        json!({"values":{"synonym":"Properties role preview","Comment":"Independent alias groups"}}),
        &[
            (json!({"values":{"Synonym":"","comment":""}}), true),
            (json!({"values":{"synonym":"","Comment":""}}), true),
            (json!({"values":{"Synonym":"a","synonym":"a"}}), false),
            (json!({"values":{"Comment":"a","comment":"b"}}), false),
            (json!({"values":{"ClientOrdinaryApplication":false}}), false),
            (json!({"values":{"IncludeInCommandInterface":false}}), false),
            (json!({"values":{"Explanation":"Subsystem only"}}), false),
        ],
    );
}

#[test]
fn props_set_subsystem_can_is_contextual_and_previews_without_writes() {
    assert_props_can_and_preview(
        "main:Subsystem.Администрирование",
        json!({"values":{"Comment":"Properties subsystem preview","explanation":"","IncludeInCommandInterface":false}}),
        &[
            (
                json!({"values":{"synonym":"","Comment":"","explanation":""}}),
                true,
            ),
            (
                json!({"values":{"Synonym":"","comment":"","Explanation":""}}),
                true,
            ),
            (
                json!({"values":{"IncludeHelpInContents":true,"includeInCommandInterface":false,"UseOneCommand":true}}),
                true,
            ),
            (
                json!({"values":{"includeHelpInContents":false,"IncludeInCommandInterface":true,"useOneCommand":false}}),
                true,
            ),
            (json!({"values":{"IncludeHelpInContents":"true"}}), false),
            (
                json!({"values":{"includeInCommandInterface":"false"}}),
                false,
            ),
            (json!({"values":{"UseOneCommand":0}}), false),
            (json!({"values":{"Comment":"a","comment":"a"}}), false),
            (
                json!({"values":{"IncludeInCommandInterface":true,"includeInCommandInterface":false}}),
                false,
            ),
            (json!({"values":{"ClientOrdinaryApplication":true}}), false),
            (json!({"values":{"Content":[]}}), false),
        ],
    );
}

#[test]
fn seven_metadata_can_contracts_publish_and_real_arguments_preview_without_writes() {
    let fixture = Fixture::with_extension();
    let runtime = runtime();
    let before = crate::test_support::tree_snapshot(&fixture.source);
    let extension_before = crate::test_support::tree_snapshot(&fixture.workspace.join("src-ext"));
    let cases = [
        (
            "main:Configuration",
            "object.create",
            json!({"values":{"kind":"Catalog","name":"CanContractObject"}}),
        ),
        (
            "main:Report.АнализВерсийОбъектов",
            "object.remove",
            json!({"force":true}),
        ),
        (
            "ext:Configuration",
            "object.borrow",
            json!({"from":"main:CommonModule.GoogleПереводчик"}),
        ),
        (
            "main:Catalog.Валюты",
            "help.create",
            json!({"values":{"lang":"ru"}}),
        ),
        (
            "main:Catalog.Валюты",
            "relation.add",
            json!({"values":{"relation":"inputByString","targets":[{"fieldPath":"Catalog.Валюты.Attribute.НаименованиеПолное"}]}}),
        ),
        (
            "main:Catalog.Валюты",
            "relation.remove",
            json!({"values":{"relation":"inputByString","targets":[{"fieldPath":"Catalog.Валюты.StandardAttribute.Description"}]}}),
        ),
        (
            "main:Catalog.Валюты",
            "relation.replace",
            json!({"values":{"relation":"inputByString","targets":[{"fieldPath":"Catalog.Валюты.StandardAttribute.Description"}]}}),
        ),
    ];
    for (at, op, args) in cases {
        let detailed = contract(&runtime, &fixture, at, op);
        assert!(
            jsonschema::validator_for(&detailed["argsSchema"])
                .unwrap()
                .is_valid(&args),
            "{op}: {args}"
        );
        let preview = call(
            &runtime,
            &fixture,
            ToolIdentity::Apply,
            json!({"at":at,"ops":[{"op":op,"args":args}]}),
        );
        assert!(preview.ok, "real {op} preview: {preview:?}");
        assert_eq!(preview.data.as_ref().unwrap()["mode"], "preview");
        assert!(preview.data.as_ref().unwrap()["executionToken"]
            .as_str()
            .is_some_and(|token| !token.is_empty()));
        assert_eq!(crate::test_support::tree_snapshot(&fixture.source), before);
        assert_eq!(
            crate::test_support::tree_snapshot(&fixture.workspace.join("src-ext")),
            extension_before
        );
    }
}

#[test]
fn scalar_and_nested_can_fields_agree_with_productive_readers() {
    let fixture = Fixture::with_extension();
    let runtime = runtime();
    let main = fixture.binding(false);
    let extension = fixture.binding(true);
    let cases = [
        (
            "main:Configuration",
            "object.create",
            &main,
            vec![
                (
                    json!({"values":{"kind":"Catalog","name":"NewCatalog"}}),
                    true,
                ),
                (json!({}), false),
                (json!({"values":false}), false),
                (json!({"values":{"name":"A"}}), false),
                (json!({"values":{"kind":"Catalog"}}), false),
                (json!({"values":{"kind":false,"name":"A"}}), false),
                (json!({"values":{"kind":"Unknown","name":"A"}}), false),
                (json!({"values":{"kind":"Catalog","name":""}}), false),
                (json!({"values":{"kind":"Catalog","name":false}}), false),
                (
                    json!({"values":{"kind":"Catalog","name":"A","unknown":1}}),
                    false,
                ),
                (
                    json!({"values":{"kind":"Catalog","name":"A"},"unknown":1}),
                    false,
                ),
            ],
        ),
        (
            "main:Catalog.Валюты",
            "object.remove",
            &main,
            vec![
                (json!({}), true),
                (json!({"force":false}), true),
                (json!({"force":true}), true),
                (json!({"force":"true"}), false),
                (json!({"unknown":true}), false),
            ],
        ),
        (
            "main:Catalog.Валюты",
            "help.create",
            &main,
            vec![
                (json!({"values":{"lang":"ru"}}), true),
                (json!({}), false),
                (json!({"values":false}), false),
                (json!({"values":{}}), false),
                (json!({"values":{"lang":false}}), false),
                (json!({"values":{"lang":""}}), false),
                (json!({"values":{"lang":"ru-RU"}}), false),
                (json!({"values":{"lang":"ru","unknown":1}}), false),
            ],
        ),
        (
            "ext:Configuration",
            "object.borrow",
            &extension,
            vec![
                (json!({"from":"main:Catalog.Валюты"}), true),
                (json!({"from":"main:Catalog.Валюты","overrides":[]}), true),
                (
                    json!({"from":"main:Catalog.Валюты","overrides":false}),
                    false,
                ),
                (
                    json!({"from":"main:Catalog.Валюты","overrides":[false]}),
                    false,
                ),
                (
                    json!({"from":"main:Catalog.Валюты","overrides":[""]}),
                    false,
                ),
                (json!({"from":"main:Catalog.Валюты","unknown":1}), false),
            ],
        ),
    ];
    let before = crate::test_support::tree_snapshot(&fixture.source);
    for (at, op, binding, samples) in cases {
        let detailed = contract(&runtime, &fixture, at, op);
        assert_parity(&detailed["argsSchema"], binding, at, op, &samples);
    }
    // The create parser historically accepts a nonempty name; identifier
    // validity belongs to staging. Can can explain the complete grammar
    // without moving that refusal before a later operation's parse error.
    let invalid_name = json!({"values":{"kind":"Catalog","name":"bad name"}});
    let detailed = contract(&runtime, &fixture, "main:Configuration", "object.create");
    assert!(!jsonschema::validator_for(&detailed["argsSchema"])
        .unwrap()
        .is_valid(&invalid_name));
    assert!(parse(&main, "main:Configuration", "object.create", &invalid_name).is_ok());
    let refused = call(
        &runtime,
        &fixture,
        ToolIdentity::Apply,
        json!({"at":"main:Configuration","ops":[
            {"op":"object.create","args":invalid_name},
            {"op":"object.create","args":{"values":{"kind":false,"name":"ValidName"}}}
        ]}),
    );
    assert!(!refused.ok, "{refused:?}");
    assert_eq!(refused.at.as_deref(), Some("ops[1].args.values.kind"));
    assert_eq!(
        refused.summary,
        "object.create requires `values.kind`, a top-level metadata kind"
    );
    assert_eq!(refused.diagnostics[0]["code"], "bad_value");
    assert_eq!(crate::test_support::tree_snapshot(&fixture.source), before);
}

#[test]
fn relation_can_is_contextual_and_a_relationless_owner_has_no_accepted_sample() {
    let fixture = Fixture::new("workspace");
    let event = Fixture::new("workspace-event-source-order");
    let runtime = runtime();
    let main = fixture.binding(false);
    let event_binding = event.binding(false);
    let catalog = "main:Catalog.Валюты";
    let document = "main:Document.АктОбУничтоженииПерсональныхДанных";
    let report = "main:Report.АнализВерсийОбъектов";
    let event_at = "main:EventSubscription.CorpusEventSubscription";
    let samples = [
        ("owners", json!([{"metadataPath":"Catalog.Валюты"}])),
        (
            "registerRecords",
            json!([{"metadataPath":"InformationRegister.АдминистративнаяИерархия"}]),
        ),
        ("basedOn", json!([{"metadataPath":"Catalog.Валюты"}])),
        (
            "inputByString",
            json!([{"fieldPath":"Catalog.Валюты.StandardAttribute.Description"}]),
        ),
        (
            "source",
            json!([{"kind":"object","metadataPath":"Catalog.ПримерА"}]),
        ),
    ];
    let before = crate::test_support::tree_snapshot(&fixture.source);
    let event_before = crate::test_support::tree_snapshot(&event.source);
    for op in ["relation.add", "relation.remove", "relation.replace"] {
        for (workspace, binding, at, accepted) in [
            (
                &fixture,
                &main,
                catalog,
                &["owners", "basedOn", "inputByString"][..],
            ),
            (
                &fixture,
                &main,
                document,
                &["registerRecords", "basedOn", "inputByString"][..],
            ),
            (&event, &event_binding, event_at, &["source"][..]),
            (&fixture, &main, report, &[][..]),
        ] {
            let detailed = contract(&runtime, workspace, at, op);
            let schema = &detailed["argsSchema"];
            let oracle = jsonschema::validator_for(schema).unwrap();
            if accepted.is_empty() {
                assert_eq!(
                    schema["properties"]["values"]["properties"]["relation"]["not"],
                    json!({})
                );
                assert_eq!(detailed["exampleArgs"], json!({}));
                assert!(detailed["notes"]
                    .as_str()
                    .is_some_and(|notes| !notes.is_empty()));
            }
            let mut accepted_samples = 0;
            for (relation, targets) in &samples {
                let mut targets = targets.clone();
                if *relation == "inputByString" && at == document {
                    targets[0]["fieldPath"] = json!(format!(
                        "{}.StandardAttribute.Number",
                        at.strip_prefix("main:").unwrap()
                    ));
                }
                let args = json!({"values":{"relation":relation,"targets":targets}});
                let expected = accepted.contains(relation);
                let parsed = parse(binding, at, op, &args);
                assert_eq!(oracle.is_valid(&args), expected, "schema {op} {at}: {args}");
                assert_eq!(
                    parsed.is_ok(),
                    expected,
                    "parser {op} {at}: {args}: {parsed:?}"
                );
                accepted_samples += usize::from(oracle.is_valid(&args));
                if at == report {
                    let error = parsed.unwrap_err();
                    assert_eq!(error.path(), Some("ops[3].args.values.relation"));
                    assert_eq!(
                        error.to_string(),
                        format!("relation `{relation}` is not supported for Report")
                    );
                }
            }
            assert_eq!(accepted_samples, accepted.len());
            for args in [
                json!({}),
                json!({"values":{}}),
                json!({"values":{"relation":false,"targets":[]}}),
                json!({"values":{"relation":"owners","targets":false}}),
                json!({"values":{"relation":"owners","targets":[]}}),
                json!({"values":{"relation":"owners","targets":[{"metadataPath":false}]}}),
                json!({"values":{"relation":"owners","targets":[{"metadataPath":"Catalog.Валюты","unknown":1}]}}),
                json!({"values":{"relation":"owners","targets":[{"metadataPath":"Catalog.Валюты"}],"unknown":1}}),
            ] {
                assert!(!oracle.is_valid(&args), "{op} {at}: {args}");
                assert!(parse(binding, at, op, &args).is_err(), "{op} {at}: {args}");
            }
        }
    }
    assert_eq!(crate::test_support::tree_snapshot(&fixture.source), before);
    assert_eq!(
        crate::test_support::tree_snapshot(&event.source),
        event_before
    );
}

#[test]
fn event_source_can_preserves_both_manager_variants_and_raw_diagnostic_order() {
    let fixture = Fixture::new("workspace-event-source-order");
    let runtime = runtime();
    let binding = fixture.binding(false);
    let at = "main:EventSubscription.CorpusEventSubscription";
    for op in ["relation.add", "relation.remove", "relation.replace"] {
        let detailed = contract(&runtime, &fixture, at, op);
        let samples = [
            (json!({"kind":"manager","metadataPath":"Catalog.ПримерА"}), true),
            (json!({"kind":"manager","metadataPath":"Constant.Sample","sourceClass":"constantManager"}), true),
            (json!({"kind":"manager","metadataPath":"Constant.Sample","sourceClass":"constantValueManager"}), true),
            (json!({"kind":"manager","metadataPath":"Constant.Sample"}), false),
            (json!({"kind":"manager","metadataPath":"Constant.Sample","sourceClass":"catalogManager"}), false),
            (json!({"kind":"manager","metadataPath":"Catalog.ПримерА","sourceClass":"catalogManager"}), false),
            (json!({"kind":"manager","metadataPath":"Catalog.ПримерА","sourceClass":false}), false),
            (json!({"kind":"object","metadataPath":"Catalog.ПримерА"}), true),
            (json!({"kind":"recordSet","metadataPath":"InformationRegister.Sample"}), true),
            (json!({"kind":"definedType","metadataPath":"DefinedType.Sample"}), true),
            (json!({"kind":"family","sourceClass":"catalogObject"}), true),
            (json!({"kind":"family","sourceClass":"unknown"}), false),
            (json!({"kind":"family","sourceClass":false}), false),
            (json!({"kind":"unknown","metadataPath":"Catalog.ПримерА"}), false),
            (json!({"kind":"object","metadataPath":"Catalog.ПримерА","unknown":1}), false),
        ].map(|(target, accepted)| (json!({"values":{"relation":"source","targets":[target]}}), accepted));
        assert_parity(&detailed["argsSchema"], &binding, at, op, &samples);
    }
    for (target, suffix) in [
        (
            json!({"kind":"manager","metadataPath":false,"unknown":1}),
            "metadataPath",
        ),
        (
            json!({"kind":"manager","metadataPath":"Catalog.ПримерА","sourceClass":false,"unknown":1}),
            "sourceClass",
        ),
        (
            json!({"kind":"manager","metadataPath":"Catalog.ПримерА","sourceClass":"unknown","unknown":1}),
            "sourceClass",
        ),
        (
            json!({"kind":"family","sourceClass":false,"unknown":1}),
            "sourceClass",
        ),
        (
            json!({"kind":"object","metadataPath":false,"unknown":1}),
            "metadataPath",
        ),
        (
            json!({"kind":"recordSet","metadataPath":false,"unknown":1}),
            "metadataPath",
        ),
        (
            json!({"kind":"definedType","metadataPath":false,"unknown":1}),
            "metadataPath",
        ),
    ] {
        let error = parse(
            &binding,
            at,
            "relation.replace",
            &json!({"values":{"relation":"source","targets":[target]}}),
        )
        .unwrap_err();
        assert_eq!(
            error.path(),
            Some(format!("ops[3].args.values.targets[0].{suffix}").as_str())
        );
    }
}

fn changed_paths(
    before: &SourceImage,
    after: &SourceImage,
) -> (BTreeSet<PathBuf>, BTreeSet<PathBuf>, BTreeSet<PathBuf>) {
    (
        after
            .keys()
            .filter(|path| !before.contains_key(*path))
            .cloned()
            .collect(),
        after
            .iter()
            .filter(|(path, bytes)| before.get(*path).is_some_and(|old| old != *bytes))
            .map(|(path, _)| path.clone())
            .collect(),
        before
            .keys()
            .filter(|path| !after.contains_key(*path))
            .cloned()
            .collect(),
    )
}

#[test]
fn canonical_help_saved_execute_creates_the_facet_and_preserves_neighbors_and_refusals() {
    let fixture = Fixture::new("workspace-resolve");
    let owner = fixture.source.join("Catalogs/Items.xml");
    let owner_xml = std::fs::read_to_string(&owner)
        .unwrap()
        .replace("<Form>Main</Form>", "<Form>Main</Form><Form>Kept</Form>");
    std::fs::write(&owner, owner_xml).unwrap();
    std::fs::write(fixture.source.join("Catalogs/Items/Forms/Kept.xml"), r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Form uuid="f3317000-0000-4000-8000-000000000099"><Properties><Name>Kept</Name><FormType>Managed</FormType><IncludeHelpInContents>true</IncludeHelpInContents></Properties></Form></MetaDataObject>"#).unwrap();
    let runtime = runtime();
    let args = json!({"at":"main:Catalog.Items","ops":[{"op":"help.create","args":{"values":{"lang":"ru"}}}]});
    let before = crate::test_support::tree_snapshot(&fixture.source);
    let planned = call(&runtime, &fixture, ToolIdentity::Apply, args.clone());
    assert!(planned.ok, "{planned:?}");
    assert_eq!(planned.data.as_ref().unwrap()["mode"], "preview");
    assert_eq!(crate::test_support::tree_snapshot(&fixture.source), before);

    // An external writer's edit invalidates the saved plan. Its bytes survive the
    // refusal, and neither help file nor a form edit may leak from the plan.
    let mut edited_owner = std::fs::read(&owner).unwrap();
    edited_owner.extend_from_slice(b"\n<!-- changed after help preview -->\n");
    std::fs::write(&owner, edited_owner).unwrap();
    let externally_changed = crate::test_support::tree_snapshot(&fixture.source);
    let stale = call(
        &runtime,
        &fixture,
        ToolIdentity::Apply,
        json!({"executionToken":planned.data.as_ref().unwrap()["executionToken"]}),
    );
    assert!(!stale.ok, "{stale:?}");
    assert_eq!(stale.diagnostics[0]["code"], "stale_revision");
    assert_eq!(
        crate::test_support::tree_snapshot(&fixture.source),
        externally_changed
    );

    let planned = call(&runtime, &fixture, ToolIdentity::Apply, args.clone());
    assert!(planned.ok, "{planned:?}");
    assert_eq!(
        crate::test_support::tree_snapshot(&fixture.source),
        externally_changed
    );
    // The help plan does not read this module. An independent edit must not
    // invalidate the token or be overwritten by publication.
    let module = fixture.source.join("Catalogs/Items/Ext/ObjectModule.bsl");
    let mut edited = std::fs::read(&module).unwrap();
    edited.extend_from_slice(b"\n// independent edit after fresh help preview\n");
    std::fs::write(&module, edited).unwrap();
    let externally_changed = crate::test_support::tree_snapshot(&fixture.source);
    let published = call(
        &runtime,
        &fixture,
        ToolIdentity::Apply,
        json!({"executionToken":planned.data.as_ref().unwrap()["executionToken"]}),
    );
    assert!(published.ok, "{published:?}");
    assert_eq!(published.data.as_ref().unwrap()["mode"], "published");
    assert_eq!(
        published.data.as_ref().unwrap()["planHash"],
        planned.data.as_ref().unwrap()["planHash"]
    );
    let after = crate::test_support::tree_snapshot(&fixture.source);
    assert_eq!(
        changed_paths(&externally_changed, &after),
        (
            [
                "Catalogs/Items/Ext/Help.xml",
                "Catalogs/Items/Ext/Help/ru.html"
            ]
            .map(PathBuf::from)
            .into_iter()
            .collect(),
            ["Catalogs/Items/Forms/Main.xml"]
                .map(PathBuf::from)
                .into_iter()
                .collect(),
            BTreeSet::new(),
        )
    );
    let help = std::str::from_utf8(&after[Path::new("Catalogs/Items/Ext/Help.xml")]).unwrap();
    let help = roxmltree::Document::parse(help.trim_start_matches('\u{feff}')).unwrap();
    assert_eq!(help.root_element().tag_name().name(), "Help");
    assert_eq!(
        help.root_element().tag_name().namespace(),
        Some("http://v8.1c.ru/8.3/xcf/extrnprops")
    );
    assert_eq!(help.root_element().attribute("version"), Some("2.20"));
    let pages = help
        .root_element()
        .children()
        .filter(|n| n.is_element())
        .map(|n| (n.tag_name().name(), n.text()))
        .collect::<Vec<_>>();
    assert_eq!(pages, [("Page", Some("ru"))]);
    let html = &after[Path::new("Catalogs/Items/Ext/Help/ru.html")];
    assert!(html.starts_with(&[0xef, 0xbb, 0xbf]));
    let html = std::str::from_utf8(html).unwrap();
    assert!(html.contains("<h1>Items</h1>"));
    assert!(html.contains("text/html; charset=utf-8"));
    assert!(html.contains("v8help://service_book/service_style"));
    let form = std::str::from_utf8(&after[Path::new("Catalogs/Items/Forms/Main.xml")]).unwrap();
    let form = roxmltree::Document::parse(form.trim_start_matches('\u{feff}')).unwrap();
    let flags = form
        .descendants()
        .filter(|n| n.has_tag_name("IncludeHelpInContents"))
        .map(|n| n.text())
        .collect::<Vec<_>>();
    assert_eq!(flags, [Some("false")]);
    assert!(form.descendants().any(|n| n.has_tag_name("Form")
        && n.attribute("uuid") == Some("f3317000-0000-4000-8000-000000000004")));
    let repeated = call(&runtime, &fixture, ToolIdentity::Apply, args);
    assert!(!repeated.ok, "{repeated:?}");
    assert_eq!(repeated.diagnostics[0]["code"], "invalid_state");
    assert_eq!(repeated.at.as_deref(), Some("ops[0].args"));
    assert_eq!(
        repeated.summary,
        "embedded help already exists; addHelp is create-only"
    );
    assert!(repeated
        .data
        .as_ref()
        .is_none_or(|data| data.get("executionToken").is_none()));
    assert_eq!(crate::test_support::tree_snapshot(&fixture.source), after);
}
