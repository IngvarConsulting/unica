#[path = "metadata_can_tests/property_name_parity.rs"]
mod property_name_parity;
#[path = "metadata_can_tests/relation_diagnostic_path.rs"]
mod relation_diagnostic_path;

use super::actor_capacity_tests::direct_v5;
use super::*;
use crate::domain::apply::ApplyRequest;
use unica_execution::invocation_store::ToolIdentity;
use unica_source_contract::{SourceFormat, SourceProfile, SourceSetKind};

use crate::infrastructure::native_operations::apply_families::parse_family_operation_for_test;
use unica_source_contract::proposal::ApplyPlanError;
use unica_source_contract::workspace::WorkspaceContext;
use unica_workspace::actor::{ProviderRootBinding, WorkspaceIdentity, WorkspaceSourceSetInput};

use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use unica_execution_contract::invocation::DomainResult;

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
    for (relative, bytes) in unica_workspace::test_support::tree_snapshot(from) {
        let path = to.join(relative);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, bytes).unwrap();
    }
}

impl Fixture {
    fn new(name: &str) -> Self {
        let root = tempfile::tempdir().unwrap();
        let workspace =
            unica_workspace::test_support::canonical_path(root.path()).join("workspace");
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

    /// Минимальная конфигурация с одним языком: объекты создаёт сам тест
    /// каноническим `apply`, без прежнего `cf.init`/`meta.add`.
    fn blank() -> Self {
        let root = tempfile::tempdir().unwrap();
        let workspace =
            unica_workspace::test_support::canonical_path(root.path()).join("workspace");
        let source = workspace.join("src");
        std::fs::create_dir_all(source.join("Languages")).unwrap();
        std::fs::write(
            workspace.join("v8project.yaml"),
            "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
        )
        .unwrap();
        std::fs::write(
            source.join("Configuration.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Configuration uuid="f3317000-0000-4000-8000-0000000000c1"><Properties><Name>Store</Name><DefaultLanguage>Language.Русский</DefaultLanguage></Properties><ChildObjects><Language>Русский</Language></ChildObjects></Configuration></MetaDataObject>"#,
        )
        .unwrap();
        std::fs::write(
            source.join("Languages/Русский.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Language uuid="33333333-3333-4333-8333-333333333333"><Properties><Name>Русский</Name><Comment/><LanguageCode>ru</LanguageCode></Properties></Language></MetaDataObject>"#,
        )
        .unwrap();
        Self {
            _root: root,
            source,
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
        crate::infrastructure::source_workspace::new_actor(identity, context)
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

/// Канонический цикл `apply`: план без записи, затем исполнение по токену
/// с тем же хешем плана.
fn publish(runtime: &V5CanonicalInvocationRuntime, fixture: &Fixture, args: Value) -> DomainResult {
    let before = unica_workspace::test_support::tree_snapshot(&fixture.workspace);
    let planned = call(runtime, fixture, ToolIdentity::Apply, args);
    assert!(planned.ok, "{planned:?}");
    assert_eq!(
        unica_workspace::test_support::tree_snapshot(&fixture.workspace),
        before,
        "planning writes nothing"
    );
    let planned = planned.data.unwrap();
    assert_eq!(planned["mode"], "preview");
    let published = call(
        runtime,
        fixture,
        ToolIdentity::Apply,
        json!({"executionToken":planned["executionToken"]}),
    );
    assert!(published.ok, "{published:?}");
    assert_eq!(published.data.as_ref().unwrap()["mode"], "published");
    assert_eq!(
        published.data.as_ref().unwrap()["planHash"],
        planned["planHash"]
    );
    published
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
    assert!(data["props"].get("implemented").is_none());
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
    let before = unica_workspace::test_support::tree_snapshot(&fixture.source);
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
    assert!(data["props"].get("implemented").is_none());
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
    assert_eq!(
        unica_workspace::test_support::tree_snapshot(&fixture.source),
        before
    );
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
    assert_eq!(
        unica_workspace::test_support::tree_snapshot(&fixture.source),
        before
    );
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
            &unica_address::address::QualifiedAddress::parse(at).unwrap(),
            unica_bsl::profile::PlatformProfile::v8_3_27(),
        ),
        Some(unica_source_contract::route::RouteClass::Module)
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
    let before = unica_workspace::test_support::tree_snapshot(&fixture.source);
    let extension_before =
        unica_workspace::test_support::tree_snapshot(&fixture.workspace.join("src-ext"));
    let cases = [
        (
            "main:Configuration",
            "object.create",
            json!({"values":{"kind":"Catalog","name":"CanContractObject"}}),
        ),
        (
            "main:Report.АнализВерсийОбъектов",
            "object.remove",
            json!({}),
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
        assert_eq!(
            unica_workspace::test_support::tree_snapshot(&fixture.source),
            before
        );
        assert_eq!(
            unica_workspace::test_support::tree_snapshot(&fixture.workspace.join("src-ext")),
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
                (json!({"force":false}), false),
                (json!({"force":true}), false),
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
    let before = unica_workspace::test_support::tree_snapshot(&fixture.source);
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
    assert_eq!(
        unica_workspace::test_support::tree_snapshot(&fixture.source),
        before
    );
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
    let before = unica_workspace::test_support::tree_snapshot(&fixture.source);
    let event_before = unica_workspace::test_support::tree_snapshot(&event.source);
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
    assert_eq!(
        unica_workspace::test_support::tree_snapshot(&fixture.source),
        before
    );
    assert_eq!(
        unica_workspace::test_support::tree_snapshot(&event.source),
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
    let before = unica_workspace::test_support::tree_snapshot(&fixture.source);
    let planned = call(&runtime, &fixture, ToolIdentity::Apply, args.clone());
    assert!(planned.ok, "{planned:?}");
    assert_eq!(planned.data.as_ref().unwrap()["mode"], "preview");
    assert_eq!(
        unica_workspace::test_support::tree_snapshot(&fixture.source),
        before
    );

    // An external writer's edit invalidates the saved plan. Its bytes survive the
    // refusal, and neither help file nor a form edit may leak from the plan.
    let mut edited_owner = std::fs::read(&owner).unwrap();
    edited_owner.extend_from_slice(b"\n<!-- changed after help preview -->\n");
    std::fs::write(&owner, edited_owner).unwrap();
    let externally_changed = unica_workspace::test_support::tree_snapshot(&fixture.source);
    let stale = call(
        &runtime,
        &fixture,
        ToolIdentity::Apply,
        json!({"executionToken":planned.data.as_ref().unwrap()["executionToken"]}),
    );
    assert!(!stale.ok, "{stale:?}");
    assert_eq!(stale.diagnostics[0]["code"], "stale_revision");
    assert_eq!(
        unica_workspace::test_support::tree_snapshot(&fixture.source),
        externally_changed
    );

    let planned = call(&runtime, &fixture, ToolIdentity::Apply, args.clone());
    assert!(planned.ok, "{planned:?}");
    assert_eq!(
        unica_workspace::test_support::tree_snapshot(&fixture.source),
        externally_changed
    );
    // The help plan does not read this module. An independent edit must not
    // invalidate the token or be overwritten by publication.
    let module = fixture.source.join("Catalogs/Items/Ext/ObjectModule.bsl");
    let mut edited = std::fs::read(&module).unwrap();
    edited.extend_from_slice(b"\n// independent edit after fresh help preview\n");
    std::fs::write(&module, edited).unwrap();
    let externally_changed = unica_workspace::test_support::tree_snapshot(&fixture.source);
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
    let after = unica_workspace::test_support::tree_snapshot(&fixture.source);
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
    assert_eq!(
        unica_workspace::test_support::tree_snapshot(&fixture.source),
        after
    );
}

/// Задаёт состав определяемого типа: член с точкой и без префикса
/// `DefinedType.` пишется как `Type`, семейство и определяемый тип — как `TypeSet`.
fn set_defined_type_members(source: &Path, name: &str, members: &[&str]) {
    let path = source.join(format!("DefinedTypes/{name}.xml"));
    let descriptor = std::fs::read_to_string(&path).unwrap();
    let body = members
        .iter()
        .map(|member| {
            let tag = if member.starts_with("DefinedType.") || !member.contains('.') {
                "TypeSet"
            } else {
                "Type"
            };
            format!("<v8:{tag}>cfg:{member}</v8:{tag}>")
        })
        .collect::<String>();
    let (start, end) = match descriptor.find("<Type/>") {
        Some(start) => (start, start + "<Type/>".len()),
        None => {
            let start = descriptor.find("<Type>").expect("DefinedType Type");
            let close = start + descriptor[start..].find("</Type>").unwrap();
            (start, close + "</Type>".len())
        }
    };
    let mut changed = descriptor.clone();
    changed.replace_range(start..end, &format!("<Type>{body}</Type>"));
    assert_ne!(changed, descriptor, "{name}: members must change");
    std::fs::write(path, changed).unwrap();
}

/// Заменяет единственное вхождение фрагмента в файле исходников.
fn rewrite(path: &Path, from: &str, to: &str) {
    let text = std::fs::read_to_string(path).unwrap();
    assert_eq!(text.matches(from).count(), 1, "{}: {from}", path.display());
    std::fs::write(path, text.replacen(from, to, 1)).unwrap();
}

fn diagnostics(result: &DomainResult) -> Vec<Value> {
    assert!(result.ok, "{result:?}");
    result.data.as_ref().unwrap()["diagnostics"]
        .as_array()
        .expect("check diagnostics")
        .clone()
}

fn check_status(result: &DomainResult) -> &str {
    result.data.as_ref().unwrap()["status"].as_str().unwrap()
}

/// INV.SOURCE.OBSERVED-TYPE-CAPABILITY: UUID записывается каноническим
/// `attribute.add` как `v8:UUID` и читается `view` тем же изменяемым вариантом.
#[test]
fn canonical_uuid_attribute_round_trips_as_an_editable_observed_type() {
    let fixture = Fixture::blank();
    let runtime = runtime();
    publish(
        &runtime,
        &fixture,
        json!({"at":"main:Configuration","ops":[{"op":"object.create","args":{"values":{"kind":"Catalog","name":"Products"}}}]}),
    );
    publish(
        &runtime,
        &fixture,
        json!({"at":"main:Catalog.Products","ops":[{"op":"attribute.add","args":{"items":[
            {"name":"ExternalId","type":{"variants":[{"kind":"uuid"}]}}
        ]}}]}),
    );

    let xml = std::fs::read_to_string(fixture.source.join("Catalogs/Products.xml")).unwrap();
    let document = roxmltree::Document::parse(xml.trim_start_matches('\u{feff}')).unwrap();
    let attribute = document
        .descendants()
        .find(|node| {
            node.has_tag_name("Attribute")
                && node
                    .descendants()
                    .any(|name| name.has_tag_name("Name") && name.text() == Some("ExternalId"))
        })
        .expect("the attribute is written");
    let types = attribute
        .descendants()
        .filter(|node| node.has_tag_name(("http://v8.1c.ru/8.1/data/core", "Type")))
        .collect::<Vec<_>>();
    assert_eq!(types.len(), 1, "{xml}");
    let (prefix, local) = types[0].text().unwrap().split_once(':').unwrap();
    assert_eq!(local, "UUID");
    assert_eq!(
        types[0].lookup_namespace_uri(Some(prefix)),
        Some("http://v8.1c.ru/8.1/data/core")
    );

    let viewed = call(
        &runtime,
        &fixture,
        ToolIdentity::View,
        json!({"at":"main:Catalog.Products.Attribute.ExternalId"}),
    );
    assert!(viewed.ok, "{viewed:?}");
    let props = &viewed.data.as_ref().unwrap()["props"];
    assert!(props.get("incomplete").is_none(), "{props}");
    assert_eq!(
        props["type"],
        json!({"variants":[{"kind":"uuid"}],"mutationCapability":"editable"})
    );
    let checked = call(
        &runtime,
        &fixture,
        ToolIdentity::Check,
        json!({"at":"main:Catalog.Products"}),
    );
    assert_eq!(check_status(&checked), "passed", "{checked:?}");
    assert!(diagnostics(&checked).is_empty(), "{checked:?}");
}

/// INV.APP.META-OBSERVATION: посторонний файл в каталоге формы не скрывает
/// справочник и его форму в `view`, а `check` того же узла называет форму
/// ошибкой `provider_unavailable` логическим адресом.
#[test]
fn canonical_foreign_form_file_keeps_catalog_structure_and_reports_provider_unavailable() {
    let fixture = Fixture::blank();
    let runtime = runtime();
    publish(
        &runtime,
        &fixture,
        json!({"at":"main:Configuration","ops":[{"op":"object.create","args":{"values":{"kind":"Catalog","name":"Products"}}}]}),
    );
    publish(
        &runtime,
        &fixture,
        json!({"at":"main:Catalog.Products","ops":[{"op":"form.add","args":{"items":[{"name":"Main","type":"ObjectForm"}]}}]}),
    );
    let at = json!({"at":"main:Catalog.Products"});
    let clean = call(&runtime, &fixture, ToolIdentity::Check, at.clone());
    assert_eq!(check_status(&clean), "passed", "{clean:?}");

    std::fs::write(
        fixture
            .source
            .join("Catalogs/Products/Forms/Main/unexpected.bin"),
        b"unexpected",
    )
    .unwrap();

    let viewed = call(&runtime, &fixture, ToolIdentity::View, at.clone());
    assert!(viewed.ok, "{viewed:?}");
    let data = viewed.data.as_ref().unwrap();
    assert_eq!(data["title"], "Products");
    assert_eq!(data["props"]["Synonym"], "Products");
    assert!(
        data["branches"]
            .as_array()
            .unwrap()
            .contains(&json!({"at":"main:Catalog.Products.Form","count":1})),
        "{data}"
    );
    let forms = call(
        &runtime,
        &fixture,
        ToolIdentity::View,
        json!({"at":"main:Catalog.Products.Form"}),
    );
    assert!(forms.ok, "{forms:?}");
    assert_eq!(
        forms.data.as_ref().unwrap()["items"],
        json!([{"at":"main:Catalog.Products.Form.Main","kind":"Form","title":"Main"}])
    );

    let checked = call(&runtime, &fixture, ToolIdentity::Check, at);
    assert_eq!(check_status(&checked), "failed", "{checked:?}");
    let found = diagnostics(&checked);
    let unavailable = found
        .iter()
        .find(|diagnostic| diagnostic["code"] == "provider_unavailable")
        .unwrap_or_else(|| panic!("{found:?}"));
    assert_eq!(unavailable["severity"], "error");
    assert_eq!(unavailable["metadataPath"], "Catalog.Products.Form.Main");
    let serialized = serde_json::to_string(&checked).unwrap();
    assert!(
        !serialized.contains(&fixture.workspace.display().to_string()),
        "diagnostics expose logical identities only: {serialized}"
    );
}

/// INV.APP.META-FINDINGS: `check` справочника проверяет длину текста команды
/// по непустому представлению списка, а не по синониму, и предупреждает о
/// представлении, повторяющем синоним. Обе находки — предупреждения с полем и
/// языком, вердикт остаётся `passed`.
#[test]
fn canonical_check_reports_list_presentation_findings_with_field_and_language() {
    let long = "Текст длиной не менее тридцати девяти символов";
    assert!(long.chars().count() > 38);
    let presentation = |text: &str| {
        format!("<ListPresentation><v8:item><v8:lang>ru</v8:lang><v8:content>{text}</v8:content></v8:item></ListPresentation>")
    };
    let command_text = |diagnostic: &Value| {
        diagnostic["code"]
            .as_str()
            .is_some_and(|code| code.starts_with("command_text_"))
    };
    for (synonym, list, expected) in [
        (
            "Products",
            Some("Products"),
            Some(("redundant_list_presentation", "properties.ListPresentation")),
        ),
        (
            "Products",
            Some(long),
            Some(("command_text_upper_limit", "properties.ListPresentation")),
        ),
        (long, Some("Товары"), None),
        (
            long,
            None,
            Some(("command_text_upper_limit", "properties.Synonym")),
        ),
    ] {
        let fixture = Fixture::blank();
        let runtime = runtime();
        publish(
            &runtime,
            &fixture,
            json!({"at":"main:Configuration","ops":[{"op":"object.create","args":{"values":{"kind":"Catalog","name":"Products"}}}]}),
        );
        let descriptor = fixture.source.join("Catalogs/Products.xml");
        if synonym != "Products" {
            rewrite(
                &descriptor,
                "<v8:content>Products</v8:content>",
                &format!("<v8:content>{synonym}</v8:content>"),
            );
        }
        if let Some(list) = list {
            rewrite(&descriptor, "<ListPresentation/>", &presentation(list));
        }
        let case = format!("synonym={synonym} list={list:?}");
        let checked = call(
            &runtime,
            &fixture,
            ToolIdentity::Check,
            json!({"at":"main:Catalog.Products"}),
        );
        assert_eq!(check_status(&checked), "passed", "{case}: {checked:?}");
        let found = diagnostics(&checked);
        let relevant = found
            .iter()
            .filter(|diagnostic| {
                command_text(diagnostic) || diagnostic["code"] == "redundant_list_presentation"
            })
            .collect::<Vec<_>>();
        match expected {
            None => assert!(relevant.is_empty(), "{case}: {found:?}"),
            Some((code, field)) => {
                assert_eq!(relevant.len(), 1, "{case}: {found:?}");
                let warning = relevant[0];
                assert_eq!(warning["code"], code, "{case}");
                assert_eq!(warning["severity"], "warning", "{case}");
                assert_eq!(warning["metadataPath"], "Catalog.Products", "{case}");
                assert_eq!(warning["field"], field, "{case}");
                assert_eq!(warning["language"], "ru", "{case}");
                assert!(
                    warning["message"].as_str().is_some_and(|m| !m.is_empty()),
                    "{case}"
                );
            }
        }
    }
}

/// INV.SOURCE.EVENT-DEFINED-TYPE: `check` подписки раскрывает определяемый
/// тип источника вместе с вложенным определяемым типом до классов событий;
/// цикл и неподходящий член остаются ошибками у `relations.source`.
#[test]
fn canonical_check_expands_nested_defined_type_event_sources() {
    let fixture = Fixture::new("workspace-event-source-order");
    let runtime = runtime();
    let at = "main:EventSubscription.CorpusEventSubscription";
    publish(
        &runtime,
        &fixture,
        json!({"at":"main:Configuration","ops":[
            {"op":"object.create","args":{"values":{"kind":"DefinedType","name":"Inner"}}},
            {"op":"object.create","args":{"values":{"kind":"DefinedType","name":"Outer"}}}
        ]}),
    );
    set_defined_type_members(&fixture.source, "Inner", &["CatalogObject.ПримерБ"]);
    set_defined_type_members(&fixture.source, "Outer", &["DefinedType.Inner"]);
    publish(
        &runtime,
        &fixture,
        json!({"at":at,"ops":[{"op":"relation.add","args":{"values":{"relation":"source","targets":[
            {"kind":"definedType","metadataPath":"DefinedType.Outer"}
        ]}}}]}),
    );
    let relations = call(
        &runtime,
        &fixture,
        ToolIdentity::View,
        json!({"at":format!("{at}.Relation")}),
    );
    assert!(relations.ok, "{relations:?}");
    assert!(
        relations.data.as_ref().unwrap()["items"]
            .as_array()
            .unwrap()
            .contains(
                &json!({"relation":"source","at":"main:DefinedType.Outer","kind":"DefinedType",
                    "target":{"kind":"definedType","metadataPath":"DefinedType.Outer"}})
            ),
        "{relations:?}"
    );
    let check = || call(&runtime, &fixture, ToolIdentity::Check, json!({"at":at}));
    let consistent = check();
    assert_eq!(check_status(&consistent), "passed", "{consistent:?}");

    // A document class reachable only through the nested type changes the
    // verdict: the handler signature of BeforeWrite no longer fits both classes.
    set_defined_type_members(
        &fixture.source,
        "Inner",
        &["CatalogObject.ПримерБ", "DocumentObject"],
    );
    let nested = check();
    assert_eq!(check_status(&nested), "failed", "{nested:?}");
    assert!(
        diagnostics(&nested).iter().any(|diagnostic| {
            diagnostic["severity"] == "error"
                && diagnostic["field"] == "properties.event"
                && diagnostic["message"]
                    .as_str()
                    .is_some_and(|message| message.contains("documentObject"))
        }),
        "{nested:?}"
    );

    let inner = fixture.source.join("DefinedTypes/Inner.xml");
    let unfit_type = |_: &Path| {
        set_defined_type_members(&fixture.source, "Inner", &["CatalogObject.ПримерБ"]);
        rewrite(
            &inner,
            "<v8:Type>cfg:CatalogObject.ПримерБ</v8:Type>",
            "<v8:Type>cfg:String</v8:Type>",
        );
    };
    let cycle =
        |_: &Path| set_defined_type_members(&fixture.source, "Inner", &["DefinedType.Outer"]);
    let unfit_family = |_: &Path| set_defined_type_members(&fixture.source, "Inner", &["String"]);
    for (corrupt, cause) in [
        (&cycle as &dyn Fn(&Path), "cycle"),
        (&unfit_family, "class family cfg:String is unsupported"),
        (&unfit_type, "configuration type cfg:String is malformed"),
    ] {
        corrupt(&inner);
        let refused = check();
        assert_eq!(check_status(&refused), "failed", "{cause}: {refused:?}");
        assert!(
            diagnostics(&refused).iter().any(|diagnostic| {
                diagnostic["severity"] == "error"
                    && diagnostic["field"]
                        .as_str()
                        .is_some_and(|field| field.starts_with("relations.source"))
                    && diagnostic["message"]
                        .as_str()
                        .is_some_and(|message| message.contains(cause))
            }),
            "{cause}: {refused:?}"
        );
    }
}

/// Подписка из корпуса без единого справочника: источник — семейство
/// документов, конфигурация не содержит `Catalog`.
fn event_source_without_catalogs() -> Fixture {
    let fixture = Fixture::new("workspace-event-source-order");
    std::fs::remove_dir_all(fixture.source.join("Catalogs")).unwrap();
    let configuration = fixture.source.join("Configuration.xml");
    for name in ["ПримерВ", "ПримерА", "ПримерБ"] {
        rewrite(
            &configuration,
            &format!("\t\t\t<Catalog>{name}</Catalog>\n"),
            "",
        );
    }
    rewrite(
        &fixture.source.join("EventSubscriptions/CorpusEventSubscription.xml"),
        "\t\t\t\t<v8:Type>cfg:CatalogObject.ПримерВ</v8:Type>\n\t\t\t\t<v8:Type>cfg:CatalogObject.ПримерА</v8:Type>\n",
        "\t\t\t\t<v8:TypeSet>cfg:DocumentObject</v8:TypeSet>\n",
    );
    fixture
}

fn subscription_source(fixture: &Fixture) -> Vec<(String, String)> {
    let xml = std::fs::read_to_string(
        fixture
            .source
            .join("EventSubscriptions/CorpusEventSubscription.xml"),
    )
    .unwrap();
    let document = roxmltree::Document::parse(xml.trim_start_matches('\u{feff}')).unwrap();
    let source = document
        .descendants()
        .find(|node| node.has_tag_name("Source"))
        .expect("Source");
    source
        .children()
        .filter(|node| node.is_element())
        .map(|node| {
            let (prefix, local) = node.text().unwrap().split_once(':').unwrap();
            assert_eq!(
                node.lookup_namespace_uri(Some(prefix)),
                Some("http://v8.1c.ru/8.1/data/enterprise/current-config")
            );
            (node.tag_name().name().to_owned(), local.to_owned())
        })
        .collect()
}

/// INV.APP.EVENT-SOURCE: семейство `catalogObject` записывается без
/// справочников в конфигурации; явный выбор класса менеджера константы
/// сохраняется в XML. Повторная та же замена — пустой план: типизированный
/// читатель возвращает записанный источник.
#[test]
fn canonical_event_source_family_and_constant_manager_choice_round_trip() {
    let fixture = event_source_without_catalogs();
    let runtime = runtime();
    let at = "main:EventSubscription.CorpusEventSubscription";
    assert!(!fixture.source.join("Catalogs").exists());
    let replace = |target: Value| json!({"at":at,"ops":[{"op":"relation.replace","args":{"values":{"relation":"source","targets":[target]}}}]});
    let assert_noop = |target: Value| {
        let before = unica_workspace::test_support::tree_snapshot(&fixture.source);
        let planned = call(
            &runtime,
            &fixture,
            ToolIdentity::Apply,
            replace(target.clone()),
        );
        assert!(planned.ok, "{target}: {planned:?}");
        assert_eq!(
            planned.data.as_ref().unwrap()["effects"],
            0,
            "{target}: {planned:?}"
        );
        assert_eq!(
            unica_workspace::test_support::tree_snapshot(&fixture.source),
            before
        );
    };

    let relation = |fixture: &Fixture| {
        let view = call(
            &runtime,
            fixture,
            ToolIdentity::View,
            json!({"at":format!("{at}.Relation")}),
        );
        assert!(view.ok, "{view:?}");
        view.data.unwrap()["items"].clone()
    };
    let family = json!({"kind":"family","sourceClass":"catalogObject"});
    publish(&runtime, &fixture, replace(family.clone()));
    assert_eq!(
        relation(&fixture),
        json!([{"relation":"source","kind":"family","target":family}])
    );
    assert_eq!(
        subscription_source(&fixture),
        [("TypeSet".to_owned(), "CatalogObject".to_owned())]
    );
    assert!(!fixture.source.join("Catalogs").exists());
    let checked = call(&runtime, &fixture, ToolIdentity::Check, json!({"at":at}));
    assert_eq!(check_status(&checked), "passed", "{checked:?}");
    assert_noop(family);

    publish(
        &runtime,
        &fixture,
        json!({"at":"main:Configuration","ops":[{"op":"object.create","args":{"values":{"kind":"Constant","name":"Sample"}}}]}),
    );
    // Each manager class has its own events; the plan checks the source
    // against the event, so the event that fits is set in the same batch.
    for (class, event, written) in [
        (
            "constantManager",
            "FormGetProcessing",
            "ConstantManager.Sample",
        ),
        (
            "constantValueManager",
            "BeforeWrite",
            "ConstantValueManager.Sample",
        ),
    ] {
        let target = json!({"kind":"manager","metadataPath":"Constant.Sample","sourceClass":class});
        let mut batch = replace(target.clone());
        batch["ops"]
            .as_array_mut()
            .unwrap()
            .push(json!({"op":"props.set","args":{"values":{"Event":event}}}));
        let published = publish(&runtime, &fixture, batch);
        assert_eq!(published.data.as_ref().unwrap()["effects"], 1, "{class}");
        assert_eq!(
            subscription_source(&fixture),
            [("Type".to_owned(), written.to_owned())],
            "{class}"
        );
        assert_eq!(
            relation(&fixture),
            json!([{"relation":"source","at":"main:Constant.Sample","kind":"Constant","target":target}]),
            "{class}"
        );
        assert_noop(target);
    }
}

/// INV.SOURCE.WRITABLE-PROFILE: канонический `apply` принимает только
/// буквальную версию `2.20`; старый формат и численно равные написания
/// отклоняются до плана без записи, событий и артефактов, а отказ по старому
/// формату предлагает повторную выгрузку платформой 8.3.27.
#[test]
fn canonical_apply_refuses_inexact_export_format_and_offers_platform_reexport() {
    let args = json!({"at":"main:Catalog.Products","ops":[{"op":"attribute.add","args":{"items":[{"name":"Extra"}]}}]});
    let runtime = runtime();
    let created = |raw: &str| {
        let fixture = Fixture::blank();
        publish(
            &runtime,
            &fixture,
            json!({"at":"main:Configuration","ops":[{"op":"object.create","args":{"values":{"kind":"Catalog","name":"Products"}}}]}),
        );
        rewrite(
            &fixture.source.join("Catalogs/Products.xml"),
            r#"version="2.20""#,
            &format!(r#"version="{raw}""#),
        );
        fixture
    };
    let exact = created("2.20");
    let planned = call(&runtime, &exact, ToolIdentity::Apply, args.clone());
    assert!(planned.ok, "the exact profile is writable: {planned:?}");

    for raw in ["2.19", "2.20.0", "02.20", "2.020", "2.&#50;0"] {
        let fixture = created(raw);
        let before = unica_workspace::test_support::tree_snapshot(&fixture.workspace);
        let refused = call(&runtime, &fixture, ToolIdentity::Apply, args.clone());
        assert!(!refused.ok, "{raw}: {refused:?}");
        assert_eq!(refused.diagnostics.len(), 1, "{raw}: {refused:?}");
        assert_eq!(refused.diagnostics[0]["code"], "invalid_source", "{raw}");
        let message = refused.diagnostics[0]["message"].as_str().unwrap();
        assert!(
            message.contains("Catalogs/Products.xml"),
            "{raw}: {message}"
        );
        // Отказ называет несовместимый формат владельца как он записан.
        assert!(message.contains(raw), "{raw}: {message}");
        if raw == "2.19" {
            for expected in [
                "2.19",
                "2.20",
                "1C 8.3.27",
                "re-export",
                "with the platform",
            ] {
                assert!(message.contains(expected), "{raw}: {message}");
            }
        }
        assert!(
            refused
                .data
                .as_ref()
                .is_none_or(|data| data.get("executionToken").is_none()),
            "{raw}: {refused:?}"
        );
        assert!(refused.changed.is_empty(), "{raw}: {refused:?}");
        assert!(refused.artifacts.is_empty(), "{raw}: {refused:?}");
        assert_eq!(
            unica_workspace::test_support::tree_snapshot(&fixture.workspace),
            before,
            "{raw}"
        );
    }
}

// --- #1933: канонические `check` и `view` сообщают диагностики читателя
// метаданных — немоделируемый тип, битый квалификатор и голый `FillValue`.
// Прежде их доказывал только путь v0.12.

/// Справочник `Products` с реквизитом `Article` и константа `Rate`, оба
/// строкового типа длиной 10.
fn catalog_and_constant(runtime: &V5CanonicalInvocationRuntime) -> Fixture {
    let fixture = Fixture::blank();
    for kind in ["Catalog", "Constant"] {
        let name = if kind == "Catalog" {
            "Products"
        } else {
            "Rate"
        };
        publish(
            runtime,
            &fixture,
            json!({"at":"main:Configuration","ops":[{"op":"object.create","args":{"values":{"kind":kind,"name":name}}}]}),
        );
    }
    publish(
        runtime,
        &fixture,
        json!({"at":"main:Catalog.Products","ops":[{"op":"attribute.add","args":{"items":[
            {"name":"Article","type":{"variants":[{"kind":"string","length":10,"allowedLength":"variable"}]}}
        ]}}]}),
    );
    publish(
        runtime,
        &fixture,
        json!({"at":"main:Constant.Rate","ops":[{"op":"props.set","args":{"values":{
            "Type":{"variants":[{"kind":"string","length":10,"allowedLength":"variable"}]}
        }}}]}),
    );
    fixture
}

/// Делает строковый тип немоделируемым, но синтаксически допустимым: имя
/// платформенного типа без квалификаторов строки.
fn make_type_unmodelled(path: &Path) {
    rewrite(
        path,
        "<v8:Type>xs:string</v8:Type>",
        "<v8:Type>v8:FutureOpaque</v8:Type>",
    );
    let text = std::fs::read_to_string(path).unwrap();
    let start = text
        .find("<v8:StringQualifiers>")
        .expect("string qualifiers");
    let end = text[start..].find("</v8:StringQualifiers>").unwrap()
        + start
        + "</v8:StringQualifiers>".len();
    let mut text = text;
    text.replace_range(start..end, "");
    std::fs::write(path, text).unwrap();
}

fn view(runtime: &V5CanonicalInvocationRuntime, fixture: &Fixture, at: &str) -> Value {
    let viewed = call(runtime, fixture, ToolIdentity::View, json!({"at": at}));
    assert!(viewed.ok, "{at}: {viewed:?}");
    viewed.data.unwrap()
}

fn check(
    runtime: &V5CanonicalInvocationRuntime,
    fixture: &Fixture,
    at: &str,
) -> (String, Vec<Value>) {
    let checked = call(runtime, fixture, ToolIdentity::Check, json!({"at": at}));
    assert!(checked.ok, "{at}: {checked:?}");
    let data = checked.data.unwrap();
    (
        data["status"].as_str().unwrap().to_owned(),
        data["diagnostics"].as_array().unwrap().clone(),
    )
}

fn limits(node: &Value) -> Vec<String> {
    node["limits"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::to_owned)
        .collect()
}

/// Единственная находка с кодом у поля: читатель и валидатор не называют
/// одну поломку дважды.
fn finding<'a>(diagnostics: &'a [Value], code: &str, field: &str) -> &'a Value {
    let found = diagnostics
        .iter()
        .filter(|diagnostic| diagnostic["field"] == field)
        .collect::<Vec<_>>();
    assert_eq!(found.len(), 1, "{field}: {diagnostics:?}");
    assert_eq!(found[0]["code"], code, "{field}: {diagnostics:?}");
    found[0]
}

/// INV.ANALYSIS.METADATA-DIAGNOSTIC-VERDICT, INV.SOURCE.OBSERVED-TYPE-CAPABILITY:
/// синтаксически допустимый, но немоделируемый тип реквизита и константы —
/// предупреждение `validation_warning` у своего поля при итоге `passed`;
/// `view` называет его в пределах узла, остальные сведения сохраняются.
#[test]
fn canonical_unmodelled_type_is_a_localized_warning_in_check_and_view() {
    let runtime = runtime();
    let fixture = catalog_and_constant(&runtime);
    make_type_unmodelled(&fixture.source.join("Catalogs/Products.xml"));
    make_type_unmodelled(&fixture.source.join("Constants/Rate.xml"));

    for (at, path, field) in [
        (
            "main:Catalog.Products",
            "Catalog.Products",
            "collections.attributes[0].type",
        ),
        ("main:Constant.Rate", "Constant.Rate", "details.type"),
    ] {
        let (status, diagnostics) = check(&runtime, &fixture, at);
        assert_eq!(status, "passed", "{at}: {diagnostics:?}");
        let warning = finding(&diagnostics, "validation_warning", field);
        assert_eq!(warning["severity"], "warning");
        assert_eq!(warning["metadataPath"], path);
        assert!(
            diagnostics
                .iter()
                .all(|diagnostic| diagnostic["severity"] == "warning"),
            "{at}: {diagnostics:?}"
        );
    }

    let attribute = view(
        &runtime,
        &fixture,
        "main:Catalog.Products.Attribute.Article",
    );
    assert_eq!(attribute["props"]["synonym"], "Article");
    assert_eq!(attribute["props"]["incomplete"], true);
    assert!(attribute["props"].get("type").is_none(), "{attribute}");
    assert!(
        limits(&attribute)
            .iter()
            .any(|limit| limit.starts_with("collections.attributes[0].type: validation_warning: ")),
        "{attribute}"
    );
    let constant = view(&runtime, &fixture, "main:Constant.Rate");
    assert_eq!(constant["props"]["Synonym"], "Rate");
    // Тип не прочитан: значения нет, причина — в пределах узла.
    assert!(constant["props"]["Type"].is_null(), "{constant}");
    assert!(
        limits(&constant).iter().any(
            |limit| limit.starts_with("details.type") && limit.contains("validation_warning: ")
        ),
        "{constant}"
    );
}

/// INV.SOURCE.OBSERVED-TYPE-CAPABILITY: повреждённый квалификатор остаётся
/// ошибкой, а не превращается в предупреждение о неподдержанном типе.
#[test]
fn canonical_broken_qualifier_stays_an_error_in_check_and_view() {
    let runtime = runtime();
    let fixture = catalog_and_constant(&runtime);
    for file in ["Catalogs/Products.xml", "Constants/Rate.xml"] {
        rewrite(
            &fixture.source.join(file),
            "<v8:Length>10</v8:Length>",
            "<v8:Length>abc</v8:Length>",
        );
    }

    for (at, field) in [
        ("main:Catalog.Products", "collections.attributes[0].type"),
        ("main:Constant.Rate", "details.type"),
    ] {
        let (status, diagnostics) = check(&runtime, &fixture, at);
        assert_eq!(status, "failed", "{at}: {diagnostics:?}");
        assert_eq!(
            finding(&diagnostics, "validation_failed", field)["severity"],
            "error"
        );
        assert!(
            diagnostics
                .iter()
                .all(|diagnostic| diagnostic["code"] != "validation_warning"),
            "{at}: {diagnostics:?}"
        );
    }
    let constant = view(&runtime, &fixture, "main:Constant.Rate");
    assert!(constant["props"]["Type"].is_null(), "{constant}");
    assert!(
        limits(&constant)
            .iter()
            .any(|limit| limit.starts_with("details.type") && !limit.contains("validation_warning")),
        "{constant}"
    );

    let attribute = view(
        &runtime,
        &fixture,
        "main:Catalog.Products.Attribute.Article",
    );
    assert_eq!(attribute["props"]["incomplete"], true);
    assert!(attribute["props"].get("type").is_none(), "{attribute}");
    assert!(
        limits(&attribute)
            .iter()
            .any(|limit| limit.starts_with("collections.attributes[0].type: validation_failed: ")),
        "{attribute}"
    );
}

/// INV.SOURCE.METADATA-OBJECT-FACTS: пустой нетипизированный `FillValue` —
/// диагностика, а не молчаливое «значение не задано»; тип реквизита
/// по-прежнему читается.
#[test]
fn canonical_bare_fill_value_is_a_diagnostic_not_an_absent_value() {
    let runtime = runtime();
    let fixture = catalog_and_constant(&runtime);
    rewrite(
        &fixture.source.join("Catalogs/Products.xml"),
        "<FillValue xsi:nil=\"true\"/>",
        "<FillValue/>",
    );

    let (status, diagnostics) = check(&runtime, &fixture, "main:Catalog.Products");
    assert_eq!(status, "failed", "{diagnostics:?}");
    assert_eq!(
        finding(
            &diagnostics,
            "validation_failed",
            "collections.attributes[0].fillValue"
        )["severity"],
        "error"
    );

    let attribute = view(
        &runtime,
        &fixture,
        "main:Catalog.Products.Attribute.Article",
    );
    assert_eq!(attribute["props"]["incomplete"], true);
    assert!(attribute["props"].get("fillValue").is_none(), "{attribute}");
    assert!(attribute["props"]["type"].is_object(), "{attribute}");
    assert!(
        limits(&attribute)
            .iter()
            .any(|limit| limit
                .starts_with("collections.attributes[0].fillValue: validation_failed: ")),
        "{attribute}"
    );
}

/// INV.SOURCE.EVENT-DEFINED-TYPE: канонический план раскрывает определяемый
/// тип источника вместе с вложенным до классов событий. Цикл, неподходящий
/// член и класс, несовместимый с событием, отклоняют план без записи;
/// описания определяемых типов входят во входы плана, и их правка после
/// плана делает токен устаревшим.
#[test]
fn canonical_apply_expands_defined_type_event_sources_before_publication() {
    let at = "main:EventSubscription.CorpusEventSubscription";
    let add = json!({"at":at,"ops":[{"op":"relation.add","args":{"values":{"relation":"source","targets":[
        {"kind":"definedType","metadataPath":"DefinedType.Outer"}
    ]}}}]});
    let prepared = || {
        let fixture = Fixture::new("workspace-event-source-order");
        let runtime = runtime();
        publish(
            &runtime,
            &fixture,
            json!({"at":"main:Configuration","ops":[
                {"op":"object.create","args":{"values":{"kind":"DefinedType","name":"Inner"}}},
                {"op":"object.create","args":{"values":{"kind":"DefinedType","name":"Outer"}}}
            ]}),
        );
        set_defined_type_members(&fixture.source, "Outer", &["DefinedType.Inner"]);
        (fixture, runtime)
    };

    for (members, cause) in [
        (
            &["DefinedType.Outer"][..],
            "cycle contains `DefinedType.Outer`",
        ),
        (&["String"][..], "class family cfg:String is unsupported"),
        (
            &["CatalogObject.ПримерБ", "DocumentObject"][..],
            "`catalogObject` (Cancel) versus `documentObject`",
        ),
    ] {
        let (fixture, runtime) = prepared();
        set_defined_type_members(&fixture.source, "Inner", members);
        let before = unica_workspace::test_support::tree_snapshot(&fixture.workspace);
        let refused = call(&runtime, &fixture, ToolIdentity::Apply, add.clone());
        assert!(!refused.ok, "{cause}: {refused:?}");
        assert_eq!(refused.diagnostics.len(), 1, "{cause}: {refused:?}");
        assert_eq!(refused.diagnostics[0]["code"], "bad_value", "{cause}");
        let message = refused.diagnostics[0]["message"].as_str().unwrap();
        assert!(message.contains(cause), "{cause}: {message}");
        assert_eq!(
            unica_workspace::test_support::tree_snapshot(&fixture.workspace),
            before,
            "{cause}: a refused plan writes nothing"
        );
    }

    // A nested class that fits the event plans; the descriptors it was
    // expanded from bind the saved plan, so changing one makes it stale.
    let (fixture, runtime) = prepared();
    set_defined_type_members(&fixture.source, "Inner", &["CatalogObject.ПримерБ"]);
    let planned = call(&runtime, &fixture, ToolIdentity::Apply, add.clone());
    assert!(planned.ok, "{planned:?}");
    let token = planned.data.unwrap()["executionToken"].clone();
    set_defined_type_members(
        &fixture.source,
        "Inner",
        &["CatalogObject.ПримерБ", "DocumentObject"],
    );
    let subscription = fixture
        .source
        .join("EventSubscriptions/CorpusEventSubscription.xml");
    let unchanged = std::fs::read(&subscription).unwrap();
    let stale = call(
        &runtime,
        &fixture,
        ToolIdentity::Apply,
        json!({"executionToken": token}),
    );
    assert!(!stale.ok, "{stale:?}");
    assert_eq!(stale.diagnostics[0]["code"], "stale_revision", "{stale:?}");
    assert!(
        stale.diagnostics[0]["message"]
            .as_str()
            .unwrap()
            .contains("DefinedTypes/Inner.xml"),
        "{stale:?}"
    );
    assert_eq!(std::fs::read(&subscription).unwrap(), unchanged);

    // The same nested source, planned afresh over the fitting type, publishes.
    set_defined_type_members(&fixture.source, "Inner", &["CatalogObject.ПримерБ"]);
    publish(&runtime, &fixture, add);
}

/// INV.SOURCE.EVENT-DEFINED-TYPE: проверка плана касается только изменённого
/// источника или события. Заимствованная подписка своего источника не несёт,
/// а правка другого свойства подписки расширения с источником `v8:TypeId`
/// сохраняет его как есть: оба плана публикуются.
#[test]
fn canonical_borrowed_and_unchanged_event_sources_publish_without_a_recheck() {
    let runtime = runtime();

    // The acceptance extension root without its own objects, over the
    // configuration whose subscription is borrowed.
    let fixture = Fixture::new("workspace-event-source-order");
    let template = repo_root().join("tests/fixtures/acceptance/workspace-format/src-ext");
    let extension = fixture.workspace.join("src-ext");
    copy_image(&template.join("Languages"), &extension.join("Languages"));
    let root = std::fs::read_to_string(template.join("Configuration.xml"))
        .unwrap()
        .lines()
        .filter(|line| {
            ![
                "<Role>",
                "<CommonModule>",
                "<EventSubscription>",
                "<Catalog>",
            ]
            .iter()
            .any(|tag| line.trim_start().starts_with(tag))
        })
        .collect::<Vec<_>>()
        .join("\n");
    std::fs::write(extension.join("Configuration.xml"), root).unwrap();
    std::fs::write(
        fixture.workspace.join("v8project.yaml"),
        "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n  - name: ext\n    type: EXTENSION\n    path: src-ext\n",
    )
    .unwrap();
    publish(
        &runtime,
        &fixture,
        json!({"at":"ext:Configuration","ops":[
            {"op":"object.borrow","args":{"from":"main:EventSubscription.CorpusEventSubscription"}}
        ]}),
    );
    let borrowed =
        std::fs::read_to_string(extension.join("EventSubscriptions/CorpusEventSubscription.xml"))
            .unwrap();
    assert!(
        borrowed.contains("<ExtendedConfigurationObject>"),
        "{borrowed}"
    );
    assert!(!borrowed.contains("<Source>"), "{borrowed}");

    let fixture = Fixture::with_extension();
    let own = fixture
        .workspace
        .join("src-ext/EventSubscriptions/ПР_ПередЗаписьюНастроек.xml");
    assert!(std::fs::read_to_string(&own)
        .unwrap()
        .contains("<v8:TypeId>"));
    publish(
        &runtime,
        &fixture,
        json!({"at":"ext:EventSubscription.ПР_ПередЗаписьюНастроек","ops":[
            {"op":"props.set","args":{"values":{"Comment":"Проверено"}}}
        ]}),
    );
    assert!(std::fs::read_to_string(&own)
        .unwrap()
        .contains("<v8:TypeId>"));
}

/// Отказ предпросмотра идёт через проводной `apply`, а не только через
/// планировщик: недопустимый итог не получает токена, а попытка исполнить
/// те же входы без предпросмотра (`dryRun: false`) отклоняется. В обоих
/// случаях исходники не меняются (#959).
#[test]
fn canonical_wire_apply_refuses_removing_the_last_required_register_field_without_writes() {
    let runtime = runtime();
    let fixture = Fixture::blank();
    publish(
        &runtime,
        &fixture,
        json!({"at":"main:Configuration","ops":[
            {"op":"object.create","args":{"values":{"kind":"InformationRegister","name":"Target"}}},
            {"op":"dimension.add","args":{"at":"main:InformationRegister.Target","items":[{"name":"Only"}]}}
        ]}),
    );
    let ops = json!([{"op":"dimension.remove","args":{}}]);
    let at = "main:InformationRegister.Target.Dimension.Only";
    let before = unica_workspace::test_support::tree_snapshot(&fixture.workspace);

    let refused = call(
        &runtime,
        &fixture,
        ToolIdentity::Apply,
        json!({"at":at,"ops":ops}),
    );
    assert!(!refused.ok, "{refused:?}");
    assert!(
        refused
            .data
            .as_ref()
            .and_then(|data| data.get("executionToken"))
            .is_none(),
        "a refused preview issues no execution token: {refused:?}"
    );
    let reasons = serde_json::to_string(&refused.diagnostics).unwrap();
    assert!(
        reasons.contains("Register without dimensions, resources, and attributes"),
        "the refusal names the subject reason: {reasons}"
    );
    assert_eq!(
        unica_workspace::test_support::tree_snapshot(&fixture.workspace),
        before
    );

    let direct = call(
        &runtime,
        &fixture,
        ToolIdentity::Apply,
        json!({"at":at,"ops":ops,"dryRun":false,"ifRev":"unica-source-sha256-v1:1:any"}),
    );
    assert!(
        !direct.ok,
        "execution without a saved preview is refused: {direct:?}"
    );
    assert_eq!(
        unica_workspace::test_support::tree_snapshot(&fixture.workspace),
        before,
        "the refused direct execution changes no source byte"
    );
}

/// Состав плана обмена чтение показывает связью `content`, а меняет только
/// `content.set`: `relation.*` с `relation: "content"` на плане обмена
/// отказывает и ведёт к контракту `content.set` того же плана (#1923).
#[test]
fn exchange_plan_content_relation_leads_to_content_set() {
    let fixture = Fixture::new("workspace-exchange-content");
    let binding = fixture.binding(false);
    let runtime = runtime();
    let plan = "main:ExchangePlan.CorpusExchangePlan";
    let args =
        json!({"values":{"relation":"content","targets":[{"metadataPath":"Catalog.Пример"}]}});
    let before = unica_workspace::test_support::tree_snapshot(&fixture.workspace);
    for op in ["relation.add", "relation.remove", "relation.replace"] {
        let error = parse(&binding, plan, op, &args).unwrap_err();
        assert_eq!(error.path(), Some("ops[3].args.values.relation"), "{op}");
        assert!(error.to_string().contains("content.set"), "{op}: {error}");
        // Подсказка не зависит от остальных аргументов связи.
        let without_targets = json!({"values":{"relation":"content"}});
        let error = parse(&binding, plan, op, &without_targets).unwrap_err();
        assert!(error.to_string().contains("content.set"), "{op}: {error}");

        let refused = call(
            &runtime,
            &fixture,
            ToolIdentity::Apply,
            json!({"at":plan,"ops":[{"op":op,"args":args.clone()}]}),
        );
        assert!(!refused.ok, "{op}: {refused:?}");
        assert_eq!(
            refused.diagnostics[0]["code"], "bad_value",
            "{op}: {refused:?}"
        );
        let next = &refused.next[0];
        assert_eq!(next["tool"], "unica.view", "{op}: {refused:?}");
        assert_eq!(next["args"]["at"], format!("{plan}.Can.content.set"));
        // `next` открывается: контракт `content.set` существует на этом плане.
        contract(&runtime, &fixture, plan, "content.set");

        // Для объекта другого вида отказ прежний и в `content.set` не ведёт.
        let other = parse(&binding, "main:Catalog.Пример", op, &args).unwrap_err();
        assert!(!other.to_string().contains("content.set"), "{op}: {other}");
    }
    // Словарь не обещает `content` для `relation.*`.
    let schema = contract(&runtime, &fixture, plan, "relation.add")["argsSchema"].clone();
    let oracle = jsonschema::validator_for(&schema).unwrap();
    assert!(!oracle.is_valid(&args), "{schema}");
    assert_eq!(
        unica_workspace::test_support::tree_snapshot(&fixture.workspace),
        before,
        "refusals write nothing"
    );
}

/// INV.SOURCE.OBSERVED-TYPE-CAPABILITY: набор типов платформы
/// (`cfg:AnyIBRef` — любая ссылка базы) допустим, но не моделируется; это
/// предупреждение у поля при итоге `passed`, а не повреждение исходника.
#[test]
fn canonical_platform_type_set_is_an_unmodelled_warning_not_damage() {
    let runtime = runtime();
    let fixture = catalog_and_constant(&runtime);
    let catalog = fixture.source.join("Catalogs/Products.xml");
    make_type_unmodelled(&catalog);
    rewrite(
        &catalog,
        "<v8:Type>v8:FutureOpaque</v8:Type>",
        "<v8:TypeSet>cfg:AnyIBRef</v8:TypeSet>",
    );

    let (status, diagnostics) = check(&runtime, &fixture, "main:Catalog.Products");
    assert_eq!(status, "passed", "{diagnostics:?}");
    assert_eq!(
        finding(
            &diagnostics,
            "validation_warning",
            "collections.attributes[0].type"
        )["severity"],
        "warning"
    );
    let attribute = view(
        &runtime,
        &fixture,
        "main:Catalog.Products.Attribute.Article",
    );
    assert!(
        limits(&attribute)
            .iter()
            .any(|limit| limit.starts_with("collections.attributes[0].type: validation_warning: ")),
        "{attribute}"
    );
}

/// INV.SOURCE.METADATA-OBJECT-FACTS: непустые параметры выбора стандартного
/// реквизита — допустимая выгрузка платформы, которую профиль не моделирует.
/// Неподдержанная конструкция не доказывает повреждение: `check` проходит с
/// предупреждением у поля, а чтение соседних деклараций продолжается, так что
/// настоящее повреждение после неё по-прежнему даёт ошибку.
#[test]
fn canonical_unmodelled_standard_attribute_choice_parameters_do_not_fail_check() {
    let runtime = runtime();
    let fixture = catalog_and_constant(&runtime);
    let catalog = fixture.source.join("Catalogs/Products.xml");
    let text = std::fs::read_to_string(&catalog).unwrap();
    let empty = "<xr:ChoiceParameters/>";
    let at = text
        .find(empty)
        .expect("standard attribute ChoiceParameters");
    let mut changed = text.clone();
    changed.replace_range(
        at..at + empty.len(),
        "<xr:ChoiceParameters><app:item xmlns:app=\"http://v8.1c.ru/8.2/managed-application/core\" name=\"Отбор.Код\"><app:value xsi:type=\"xs:string\">A</app:value></app:item></xr:ChoiceParameters>",
    );
    std::fs::write(&catalog, &changed).unwrap();

    let (status, diagnostics) = check(&runtime, &fixture, "main:Catalog.Products");
    assert_eq!(status, "passed", "{diagnostics:?}");
    let field = diagnostics
        .iter()
        .filter_map(|diagnostic| diagnostic["field"].as_str())
        .find(|field| field.ends_with(".properties.ChoiceParameters"))
        .unwrap_or_else(|| panic!("{diagnostics:?}"))
        .to_owned();
    assert!(field.starts_with("standardAttributes["), "{field}");
    assert_eq!(
        finding(&diagnostics, "validation_warning", &field)["severity"],
        "warning"
    );
    let catalog_view = view(&runtime, &fixture, "main:Catalog.Products");
    assert!(
        limits(&catalog_view)
            .iter()
            .all(|limit| !limit.contains("ChoiceParameters")),
        "{catalog_view}"
    );

    // Повреждение у следующего стандартного реквизита не прячется за
    // немоделируемой декларацией перед ним.
    let next = changed[at..]
        .find("<xr:Synonym/>")
        .expect("a later synonym")
        + at;
    let mut damaged = changed.clone();
    damaged.replace_range(
        next..next + "<xr:Synonym/>".len(),
        "<xr:Synonym>broken</xr:Synonym>",
    );
    std::fs::write(&catalog, damaged).unwrap();
    let (status, diagnostics) = check(&runtime, &fixture, "main:Catalog.Products");
    assert_eq!(status, "failed", "{diagnostics:?}");
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic["severity"] == "error"
                && diagnostic["field"]
                    .as_str()
                    .is_some_and(|field| field.starts_with("standardAttributes["))),
        "{diagnostics:?}"
    );
}

/// INV.SOURCE.OBSERVED-TYPE-CAPABILITY: набор типов, которого платформа не
/// выгружает (`cfg:CommonModuleRef`), остаётся ошибкой, а не превращается в
/// предупреждение о неподдержанном типе.
#[test]
fn canonical_unknown_type_set_stays_an_error() {
    let runtime = runtime();
    let fixture = catalog_and_constant(&runtime);
    let catalog = fixture.source.join("Catalogs/Products.xml");
    make_type_unmodelled(&catalog);
    rewrite(
        &catalog,
        "<v8:Type>v8:FutureOpaque</v8:Type>",
        "<v8:TypeSet>cfg:CommonModuleRef</v8:TypeSet>",
    );
    let (status, diagnostics) = check(&runtime, &fixture, "main:Catalog.Products");
    assert_eq!(status, "failed", "{diagnostics:?}");
    assert_eq!(
        finding(
            &diagnostics,
            "validation_failed",
            "collections.attributes[0].type"
        )["severity"],
        "error"
    );
}

/// #1941, INV.SOURCE.METADATA-OBJECT-FACTS: повреждённое свойство табличной
/// части и её реквизита из профиля их свойств `check` называет ошибкой у поля
/// элемента так же, как `view` называет её в пределах узла.
#[test]
fn canonical_check_reports_a_duplicated_tabular_attribute_property() {
    let runtime = runtime();
    let fixture = catalog_and_constant(&runtime);
    publish(
        &runtime,
        &fixture,
        json!({"at":"main:Catalog.Products","ops":[{"op":"tabularSection.add","args":{"items":[{"name":"Goods"}]}}]}),
    );
    publish(
        &runtime,
        &fixture,
        json!({"at":"main:Catalog.Products.TabularSection.Goods","ops":[{"op":"attribute.add","args":{"items":[
            {"name":"Price","type":{"variants":[{"kind":"string","length":10,"allowedLength":"variable"}]}}
        ]}}]}),
    );
    let (status, diagnostics) = check(&runtime, &fixture, "main:Catalog.Products");
    assert!(
        diagnostics
            .iter()
            .all(|diagnostic| diagnostic["severity"] != "error"),
        "{status}: {diagnostics:?}"
    );

    let catalog = fixture.source.join("Catalogs/Products.xml");
    let text = std::fs::read_to_string(&catalog).unwrap();
    let section = text
        .find("<TabularSection uuid=")
        .expect("the tabular section");
    let attribute = text[section..]
        .find("<Attribute uuid=")
        .expect("its attribute")
        + section;
    let indexing = text[attribute..]
        .find("<Indexing>")
        .expect("attribute Indexing")
        + attribute;
    let end = text[indexing..].find("</Indexing>").unwrap() + indexing + "</Indexing>".len();
    let line_number = text[section..]
        .find("<LineNumberLength>")
        .expect("section LineNumberLength")
        + section;
    assert!(
        line_number < attribute,
        "the section property precedes its attributes"
    );
    let line_end = text[line_number..].find("</LineNumberLength>").unwrap()
        + line_number
        + "</LineNumberLength>".len();
    let mut duplicated = text.clone();
    duplicated.insert_str(end, &text[indexing..end]);
    duplicated.insert_str(line_end, &text[line_number..line_end]);
    std::fs::write(&catalog, duplicated).unwrap();

    let section_field = "collections.tabularSections[0].properties.LineNumberLength";
    let field = "collections.tabularSections[0].attributes[0].properties.Indexing";
    let (status, diagnostics) = check(&runtime, &fixture, "main:Catalog.Products");
    assert_eq!(status, "failed", "{diagnostics:?}");
    assert_eq!(
        finding(&diagnostics, "validation_failed", field)["severity"],
        "error"
    );
    assert_eq!(
        finding(&diagnostics, "validation_failed", section_field)["severity"],
        "error"
    );
    let section_view = view(
        &runtime,
        &fixture,
        "main:Catalog.Products.TabularSection.Goods",
    );
    assert!(
        limits(&section_view)
            .iter()
            .any(|limit| limit.contains(section_field)),
        "{section_view}"
    );
    let viewed = view(
        &runtime,
        &fixture,
        "main:Catalog.Products.TabularSection.Goods.Attribute.Price",
    );
    assert!(
        limits(&viewed).iter().any(|limit| limit.contains(field)),
        "{viewed}"
    );
}

/// #2027: макет двоичных данных без содержимого платформа выгружает одним
/// описанием, без `Ext/Template.bin` (так его загружает и выгружает ibcmd
/// 8.3.27). `check` не считает его непроверяемым ресурсом, а частичный каталог
/// и макет другого вида без содержимого по-прежнему остаются ошибкой.
#[test]
fn canonical_check_accepts_a_binary_template_without_data() {
    let runtime = runtime();
    let fixture = catalog_and_constant(&runtime);
    publish(
        &runtime,
        &fixture,
        json!({"at":"main:Catalog.Products","ops":[{"op":"template.add","args":{"items":[
            {"name":"Sheet","templateType":"SpreadsheetDocument"}
        ]}}]}),
    );
    // Двоичный макет планировщик не создаёт: описание — из выгрузки платформы.
    let templates = fixture.source.join("Catalogs/Products/Templates");
    let descriptor = std::fs::read_to_string(
        repo_root()
            .join("tests/fixtures/platform_8_3_27/template-new-xml/native-binary-descriptor.xml"),
    )
    .unwrap()
    .replace("CorpusTemplate", "Empty")
    .replace("Corpus template", "Empty");
    std::fs::write(templates.join("Empty.xml"), descriptor).unwrap();
    std::fs::create_dir_all(templates.join("Empty/Ext")).unwrap();
    std::fs::write(templates.join("Empty/Ext/Template.bin"), b"data").unwrap();
    rewrite(
        &fixture.source.join("Catalogs/Products.xml"),
        "<Template>Sheet</Template>",
        "<Template>Sheet</Template>\n\t\t\t<Template>Empty</Template>",
    );
    let (status, diagnostics) = check(&runtime, &fixture, "main:Catalog.Products");
    assert!(
        diagnostics
            .iter()
            .all(|diagnostic| diagnostic["severity"] != "error"),
        "{status}: {diagnostics:?}"
    );

    std::fs::remove_dir_all(templates.join("Empty")).unwrap();
    let (status, diagnostics) = check(&runtime, &fixture, "main:Catalog.Products");
    assert!(
        diagnostics
            .iter()
            .all(|diagnostic| diagnostic["severity"] != "error"),
        "{status}: {diagnostics:?}"
    );
    let catalog = view(&runtime, &fixture, "main:Catalog.Products");
    assert!(
        catalog["branches"]
            .as_array()
            .unwrap()
            .contains(&json!({"at":"main:Catalog.Products.Template","count":2})),
        "{catalog}"
    );

    // Частичный каталог без файла — не «макет без данных».
    std::fs::create_dir_all(templates.join("Empty/Ext")).unwrap();
    let (status, diagnostics) = check(&runtime, &fixture, "main:Catalog.Products");
    assert_eq!(status, "failed", "{diagnostics:?}");
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic["code"] == "provider_unavailable"
                && diagnostic["metadataPath"] == "Catalog.Products.Template.Empty"),
        "{diagnostics:?}"
    );
    std::fs::remove_dir_all(templates.join("Empty")).unwrap();

    // Владелец с таким макетом правится: макет удаляется без отказа.
    publish(
        &runtime,
        &fixture,
        json!({"at":"main:Catalog.Products.Template.Empty","ops":[{"op":"template.remove","args":{}}]}),
    );
    assert!(!templates.join("Empty.xml").exists());
    assert!(
        !std::fs::read_to_string(fixture.source.join("Catalogs/Products.xml"))
            .unwrap()
            .contains("<Template>Empty</Template>")
    );

    // Табличному макету содержимое обязательно.
    std::fs::remove_dir_all(templates.join("Sheet")).unwrap();
    let (status, diagnostics) = check(&runtime, &fixture, "main:Catalog.Products");
    assert_eq!(status, "failed", "{diagnostics:?}");
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic["code"] == "provider_unavailable"
                && diagnostic["metadataPath"] == "Catalog.Products.Template.Sheet"),
        "{diagnostics:?}"
    );
}

/// #2027: общий макет двоичных данных без содержимого — то же описание без
/// `Ext/Template.bin`, которое платформа загружает и выгружает; `check` его
/// принимает.
#[test]
fn canonical_check_accepts_a_common_binary_template_without_data() {
    let runtime = runtime();
    let fixture = catalog_and_constant(&runtime);
    let descriptor = std::fs::read_to_string(
        repo_root()
            .join("tests/fixtures/platform_8_3_27/template-new-xml/native-binary-descriptor.xml"),
    )
    .unwrap()
    .replace("CorpusTemplate", "Empty")
    .replace("Corpus template", "Empty")
    .replace("<Template uuid=", "<CommonTemplate uuid=")
    .replace("</Template>", "</CommonTemplate>");
    std::fs::create_dir_all(fixture.source.join("CommonTemplates")).unwrap();
    std::fs::write(fixture.source.join("CommonTemplates/Empty.xml"), descriptor).unwrap();
    rewrite(
        &fixture.source.join("Configuration.xml"),
        "<Language>Русский</Language>",
        "<Language>Русский</Language><CommonTemplate>Empty</CommonTemplate>",
    );
    let (status, diagnostics) = check(&runtime, &fixture, "main:CommonTemplate.Empty");
    assert!(
        diagnostics
            .iter()
            .all(|diagnostic| diagnostic["severity"] != "error"),
        "{status}: {diagnostics:?}"
    );
    let viewed = view(&runtime, &fixture, "main:CommonTemplate.Empty");
    assert_eq!(viewed["kind"], "CommonTemplate", "{viewed}");
}

/// Собственные факты Unica в `props` корня и плоские поля составных
/// фактов: они не свойства 1С и потому публикуются в camelCase.
const UNICA_ROOT_FACTS: &[&str] = &[
    "kind",
    "support",
    "directEditSafe",
    "libraryRoot",
    "format",
    "content",
    "extends",
    "handlerModule",
    "handlerMethod",
    "scheduleRegister",
    "scheduleValueField",
    "scheduleDateField",
    "fontKind",
    "fontRef",
    "fontFaceName",
    "fontHeight",
    "fontScale",
    "fontBold",
    "fontItalic",
    "fontUnderline",
    "fontStrikeout",
    "borderStyle",
    "borderWidth",
    "belonging",
    "parentId",
    "parentStatus",
    "overridesCount",
    "overridesComplete",
    "overrides",
    "totalallowed",
    "totaldenied",
];

/// Виды, свойства которых не записывает ни одна операция.
const WITHOUT_PROPS_SET: &[&str] = &["Language", "CommonTemplate", "StyleItem"];

/// Адреса корней набора: объекты верхнего уровня и вложенные подсистемы.
fn metadata_roots(set: &str, root: &Path) -> Vec<(&'static str, String)> {
    fn sorted(directory: &Path, keep: impl Fn(&Path) -> bool) -> Vec<PathBuf> {
        let mut paths = std::fs::read_dir(directory)
            .map(|entries| entries.map(|entry| entry.unwrap().path()).collect())
            .unwrap_or_else(|_| Vec::new());
        paths.retain(|path: &PathBuf| keep(path));
        paths.sort();
        paths
    }
    fn descriptors(directory: &Path) -> Vec<PathBuf> {
        sorted(directory, |path| {
            path.extension().is_some_and(|ext| ext == "xml")
        })
    }
    let mut roots = Vec::new();
    for directory in sorted(root, Path::is_dir) {
        let Some(layout) = unica_source_designer::layout::metadata_kind_by_directory(
            directory.file_name().unwrap().to_str().unwrap(),
        ) else {
            continue;
        };
        // Общий модуль, форма и XDTO переходят на имена платформы
        // своими срезами.
        if matches!(layout.tag, "CommonModule" | "CommonForm" | "XDTOPackage") {
            continue;
        }
        let mut pending = descriptors(&directory)
            .into_iter()
            .map(|descriptor| (descriptor, format!("{set}:{}", layout.tag)))
            .collect::<Vec<_>>();
        while let Some((descriptor, prefix)) = pending.pop() {
            let name = descriptor.file_stem().unwrap().to_str().unwrap();
            let at = format!("{prefix}.{name}");
            if layout.tag == "Subsystem" {
                let nested = descriptor.with_extension("").join("Subsystems");
                pending.extend(
                    descriptors(&nested)
                        .into_iter()
                        .map(|child| (child, format!("{at}.Subsystem"))),
                );
            }
            roots.push((layout.tag, at));
        }
    }
    roots
}

/// Имя в `props` корня или в адресе `<узел>.Props.<имя>` его предела:
/// свойство платформы — ровно под именем `props.set`, если запись его
/// принимает, иначе под именем XML; camelCase — только факт Unica.
fn assert_platform_name(at: &str, key: &str, accepted: &BTreeSet<String>) {
    if key.starts_with(|first: char| first.is_ascii_uppercase()) {
        assert!(
            accepted.contains(key) || !accepted.iter().any(|name| name.eq_ignore_ascii_case(key)),
            "{at}: props.{key} differs from its props.set name"
        );
    } else {
        assert!(
            UNICA_ROOT_FACTS.contains(&key),
            "{at}: props.{key} names a platform property in camelCase"
        );
    }
}

/// Корень объекта метаданных называет свойство 1С так же, как его
/// принимает `props.set` того же адреса (#2160).
#[test]
fn metadata_root_props_use_the_names_props_set_accepts() {
    let runtime = runtime();
    let mut seen = BTreeSet::new();
    let mut published = BTreeSet::new();
    for (image, sets) in [
        ("workspace", &[("main", "src")][..]),
        ("workspace-common-template", &[("main", "src")]),
        ("workspace-styleitem-values", &[("main", "src")]),
        ("workspace-metadata-values", &[("main", "src")]),
        ("workspace-subsystem-presence", &[("main", "src")]),
        (
            "workspace-borrowed",
            &[("main", "src/cf"), ("ext", "src/ext")],
        ),
    ] {
        let fixture = Fixture::new(image);
        for (set, path) in sets {
            for (kind, at) in metadata_roots(set, &fixture.workspace.join(path)) {
                let data = view(&runtime, &fixture, &at);
                let accepted: BTreeSet<String> = if WITHOUT_PROPS_SET.contains(&kind) {
                    BTreeSet::new()
                } else {
                    let can = contract(&runtime, &fixture, &at, "props.set");
                    can["argsSchema"]["properties"]["values"]["properties"]
                        .as_object()
                        .unwrap_or_else(|| panic!("{at}: props.set lists no values: {can}"))
                        .keys()
                        .cloned()
                        .collect()
                };
                for key in data["props"].as_object().unwrap().keys() {
                    assert_platform_name(&at, key, &accepted);
                    published.insert(key.clone());
                }
                let whole = format!("{at}.Props.");
                for limit in data["limits"].as_array().into_iter().flatten() {
                    let limit = limit.as_str().unwrap();
                    if let Some((_, key)) = limit.split_once(&whole) {
                        assert_platform_name(&at, key, &accepted);
                    }
                }
                seen.insert(kind);
                if kind == "Subsystem" && at.matches(".Subsystem.").count() > 0 {
                    seen.insert("nested Subsystem");
                }
            }
        }
    }
    for kind in [
        "Catalog",
        "Document",
        "Enum",
        "Role",
        "Subsystem",
        "nested Subsystem",
        "CommonTemplate",
        "StyleItem",
    ] {
        assert!(seen.contains(kind), "the guard saw no {kind}: {seen:?}");
    }
    for property in ["Synonym", "Comment", "Explanation", "CodeLength", "Type"] {
        assert!(
            published.contains(property),
            "the guard saw no {property}: {published:?}"
        );
    }
}

/// Слишком длинное свойство подсистемы не пропадает молча: предел
/// называет его адрес под именем платформы, и по адресу оно читается
/// целиком.
#[test]
fn oversized_subsystem_property_is_named_by_its_platform_name() {
    let fixture = Fixture::new("workspace-subsystem-presence");
    let runtime = runtime();
    let at = "main:Subsystem.Parent";
    let descriptor = fixture.source.join("Subsystems/Parent.xml");
    let comment = "к".repeat(4096);
    let xml = std::fs::read_to_string(&descriptor).unwrap();
    let start = xml.find("<Comment").unwrap();
    let end = xml[start..]
        .find("</Comment>")
        .map(|end| start + end + "</Comment>".len())
        .unwrap_or_else(|| start + xml[start..].find("/>").unwrap() + 2);
    let mut rewritten = xml.clone();
    rewritten.replace_range(start..end, &format!("<Comment>{comment}</Comment>"));
    std::fs::write(&descriptor, rewritten).unwrap();
    let data = view(&runtime, &fixture, at);
    assert!(data["props"].get("Comment").is_none(), "{data}");
    let limits = data["limits"].to_string();
    assert!(limits.contains(&format!("{at}.Props.Comment")), "{limits}");
    let whole = view(&runtime, &fixture, &format!("{at}.Props.Comment"));
    assert_eq!(whole["props"]["Comment"], comment, "{whole}");
}

/// #2217: a nested subsystem is written at the address `view` reads it by,
/// in its own descriptor; its owners stay byte-identical.
#[test]
fn props_set_writes_a_nested_subsystem_at_its_own_address() {
    let fixture = Fixture::new("workspace-subsystem-presence");
    let runtime = runtime();
    let at = "main:Subsystem.Parent.Subsystem.Child.Subsystem.Leaf";
    let can = contract(&runtime, &fixture, at, "props.set");
    assert!(
        can["argsSchema"]["properties"]["values"]["properties"]
            .get("IncludeInCommandInterface")
            .is_some(),
        "{can}"
    );
    let owners = [
        "Subsystems/Parent.xml",
        "Subsystems/Parent/Subsystems/Child.xml",
    ]
    .map(|relative| std::fs::read(fixture.source.join(relative)).unwrap());
    publish(
        &runtime,
        &fixture,
        json!({"at": at, "ops": [{"op": "props.set", "args": {"values": {
            "Comment": "Nested 2217",
            "IncludeInCommandInterface": false,
        }}}]}),
    );
    let leaf = std::fs::read_to_string(
        fixture
            .source
            .join("Subsystems/Parent/Subsystems/Child/Subsystems/Leaf.xml"),
    )
    .unwrap();
    assert!(leaf.contains("<Comment>Nested 2217</Comment>"), "{leaf}");
    assert_eq!(
        [
            "Subsystems/Parent.xml",
            "Subsystems/Parent/Subsystems/Child.xml"
        ]
        .map(|relative| std::fs::read(fixture.source.join(relative)).unwrap()),
        owners
    );
    let viewed = view(&runtime, &fixture, at);
    assert_eq!(viewed["props"]["Comment"], "Nested 2217", "{viewed}");
}

/// #2218: a role synonym longer than the property limit is named in
/// `limits` by its platform address and reads whole there.
#[test]
fn oversized_role_synonym_is_named_by_its_platform_name() {
    let fixture = Fixture::new("workspace");
    let runtime = runtime();
    let at = "main:Role.SalesReader";
    let descriptor = fixture.source.join("Roles/SalesReader.xml");
    let synonym = "р".repeat(4096);
    let xml = std::fs::read_to_string(&descriptor)
        .unwrap()
        .replace("Читатель продаж", &synonym);
    std::fs::write(&descriptor, xml).unwrap();
    let data = view(&runtime, &fixture, at);
    assert!(
        data["props"].get("Synonym").is_none(),
        "an oversized synonym stays out of props"
    );
    let named = data["limits"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .filter(|limit| limit.contains(".Props."))
        .collect::<Vec<_>>();
    assert_eq!(named.len(), 1, "one limit names the dropped property");
    assert!(
        named[0].ends_with(&format!("{at}.Props.Synonym")),
        "the limit names the synonym by its platform address"
    );
    let whole = view(&runtime, &fixture, &format!("{at}.Props.Synonym"));
    assert!(
        whole["props"]["Synonym"] == synonym,
        "the synonym reads whole at its address"
    );
    let plain = view(&runtime, &Fixture::new("workspace"), at);
    assert!(
        plain.get("limits").is_none(),
        "a short synonym leaves no limit"
    );
}

/// #2246: a synonym longer than the property limit does not inflate the
/// title of the node or of its collection row: the title is cut and marked,
/// and the synonym reads whole at `<node>.Props.Synonym`.
#[test]
fn an_oversized_synonym_leaves_a_bounded_marked_title() {
    for (at, collection, descriptor, short) in [
        (
            "main:Role.SalesReader",
            "main:Role",
            "Roles/SalesReader.xml",
            "Читатель продаж",
        ),
        (
            "main:Catalog.Валюты",
            "main:Catalog",
            "Catalogs/Валюты.xml",
            "<v8:content>Валюты</v8:content>",
        ),
    ] {
        let fixture = Fixture::new("workspace");
        let runtime = runtime();
        let synonym = "р".repeat(4096);
        let path = fixture.source.join(descriptor);
        let xml = std::fs::read_to_string(&path).unwrap();
        let replacement = if short.starts_with("<v8:content>") {
            format!("<v8:content>{synonym}</v8:content>")
        } else {
            synonym.clone()
        };
        std::fs::write(&path, xml.replacen(short, &replacement, 1)).unwrap();

        let bounded = |title: &Value| {
            title.as_str().is_some_and(|title| {
                title.len() <= crate::domain::node_view::MAX_TITLE_BYTES
                    && title.strip_suffix('…').is_some_and(|kept| {
                        kept.len() + 4 > crate::domain::node_view::MAX_TITLE_BYTES - '…'.len_utf8()
                            && synonym.starts_with(kept)
                    })
            })
        };
        let data = view(&runtime, &fixture, at);
        assert!(bounded(&data["title"]), "{at}: the title is cut and marked");
        assert!(
            data["limits"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .any(|limit| limit.ends_with(&format!("{at}.Props.Synonym"))),
            "{at}: a limit names the synonym address"
        );
        let whole = view(&runtime, &fixture, &format!("{at}.Props.Synonym"));
        assert!(
            whole["props"]["Synonym"] == synonym,
            "{at}: the synonym reads whole at its address"
        );
        let rows = view(&runtime, &fixture, collection);
        let row = rows["items"]
            .as_array()
            .unwrap()
            .iter()
            .find(|row| row["at"] == at)
            .unwrap_or_else(|| panic!("{at}: the collection lists the node"));
        assert!(
            row["title"]
                .as_str()
                .is_some_and(|title| title.len() <= crate::domain::node_view::MAX_TITLE_BYTES),
            "{at}: the collection row title stays bounded"
        );
    }
}

/// #2246: a form item title longer than the property limit leaves a bounded
/// marked title and a limit naming the address it reads whole at.
#[test]
fn an_oversized_form_item_title_is_named_by_a_limit() {
    let fixture = Fixture::new("workspace");
    let runtime = runtime();
    let at = "main:Catalog.Валюты.Form.ФормаЭлемента.Item.ПолеНаименование";
    let path = fixture
        .source
        .join("Catalogs/Валюты/Forms/ФормаЭлемента/Ext/Form.xml");
    let title = "з".repeat(4096);
    let xml = std::fs::read_to_string(&path).unwrap().replacen(
        "<v8:content>Наименование</v8:content>",
        &format!("<v8:content>{title}</v8:content>"),
        1,
    );
    std::fs::write(&path, xml).unwrap();
    let data = view(&runtime, &fixture, at);
    assert!(
        data["title"].as_str().is_some_and(|shown| shown.len()
            <= crate::domain::node_view::MAX_TITLE_BYTES
            && shown.ends_with('…')),
        "the item title is cut and marked"
    );
    assert!(
        data["props"].get("title").is_none(),
        "an oversized title stays out of props"
    );
    assert!(
        data["limits"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .any(|limit| limit.ends_with(&format!("{at}.Props.title"))),
        "a limit names the title address"
    );
    let whole = view(&runtime, &fixture, &format!("{at}.Props.title"));
    assert!(
        whole["props"]["title"] == title,
        "the title reads whole at its address"
    );
}
