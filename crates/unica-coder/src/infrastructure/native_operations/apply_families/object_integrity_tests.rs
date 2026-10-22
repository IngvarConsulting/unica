use super::plan_hidden_v13_apply;
use crate::domain::apply::ApplyRequest;
use unica_source_contract::{SourceFormat, SourceProfile, SourceSetKind};

use crate::infrastructure::source_workspace::WorkspaceActor;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::time::Duration;
use unica_execution_contract::invocation::DomainResult;
use unica_source_contract::proposal::{ApplyPlanErrorKind, StagedFileState};
use unica_source_contract::workspace::WorkspaceContext;
use unica_work_control::cancellation::CancellationToken;
use unica_work_control::provider_deadline::ProviderDeadline;
use unica_workspace::actor::{
    ApplyAdmission, ApplyPublicationError, ApplyPublicationErrorKind, PreparedApplyBatch,
    ProviderRootBinding, WorkspaceIdentity, WorkspaceSourceSetInput,
};
use unica_workspace::test_support::tree_snapshot;

const MD: &str = "http://v8.1c.ru/8.3/MDClasses";

struct Fixture {
    _root: tempfile::TempDir,
    source: PathBuf,
    actor: WorkspaceActor,
    binding: ProviderRootBinding,
}

fn descriptor(kind: &str, name: &str, properties: &str, children: &str) -> String {
    format!(
        r#"<MetaDataObject xmlns="{MD}" xmlns:v8="http://v8.1c.ru/8.1/data/core" xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" xmlns:xr="http://v8.1c.ru/8.3/xcf/readable" version="2.20"><{kind} uuid="11111111-1111-4111-8111-111111111111"><Properties><Name>{name}</Name><Comment/>{properties}</Properties><ChildObjects>{children}</ChildObjects></{kind}></MetaDataObject>"#
    )
}

fn member(kind: &str, name: &str) -> String {
    format!(
        r#"<{kind} uuid="22222222-2222-4222-8222-222222222222"><Properties><Name>{name}</Name><Comment/></Properties></{kind}>"#
    )
}

impl Fixture {
    fn new(kind: SourceSetKind) -> Self {
        let root = tempfile::tempdir().unwrap();
        let workspace = std::fs::canonicalize(root.path()).unwrap();
        let source = workspace.join("src");
        std::fs::create_dir_all(&source).unwrap();
        let project_kind = if kind == SourceSetKind::Extension {
            "EXTENSION"
        } else {
            "CONFIGURATION"
        };
        std::fs::write(
            workspace.join("v8project.yaml"),
            format!("format: DESIGNER\nsource-set:\n  - name: main\n    type: {project_kind}\n    path: src\n"),
        )
        .unwrap();
        let configuration = descriptor(
            "Configuration",
            "Main",
            "<DefaultLanguage>Language.English</DefaultLanguage>",
            "<Language>English</Language><Document>Records</Document><ChartOfAccounts>Accounts</ChartOfAccounts><ChartOfCalculationTypes>Calculations</ChartOfCalculationTypes>",
        );
        for (relative, bytes) in [
            ("Configuration.xml", configuration),
            (
                "Languages/English.xml",
                descriptor("Language", "English", "<LanguageCode>en</LanguageCode>", ""),
            ),
            (
                "Documents/Records.xml",
                descriptor("Document", "Records", "<RegisterRecords/>", ""),
            ),
            (
                "ChartsOfAccounts/Accounts.xml",
                descriptor("ChartOfAccounts", "Accounts", "", ""),
            ),
            (
                "ChartsOfCalculationTypes/Calculations.xml",
                descriptor("ChartOfCalculationTypes", "Calculations", "", ""),
            ),
        ] {
            let path = source.join(relative);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, bytes).unwrap();
        }
        let context = WorkspaceContext {
            cwd: workspace.clone(),
            workspace_root: workspace.clone(),
            cache_root: workspace.join(".build/unica"),
            workspace_epoch: 1,
        };
        let identity = WorkspaceIdentity::new(
            &context,
            [WorkspaceSourceSetInput::new(
                "main",
                &source,
                kind,
                SourceFormat::PlatformXml,
                SourceProfile::platform_xml_8_3_27_format_2_20(),
            )],
            "final-object-integrity-test",
        )
        .unwrap();
        let actor = crate::infrastructure::source_workspace::new_actor(identity, context).unwrap();
        let binding = actor.bind_provider_root("main", &source).unwrap();
        Self {
            _root: root,
            source,
            actor,
            binding,
        }
    }

    fn put(&self, kind: &str, directory: &str, properties: &str, children: &str) {
        self.put_named(kind, directory, "Target", properties, children);
    }

    fn put_named(&self, kind: &str, directory: &str, name: &str, properties: &str, children: &str) {
        let configuration = self.source.join("Configuration.xml");
        let original = std::fs::read_to_string(&configuration).unwrap();
        std::fs::write(
            configuration,
            original.replace(
                "<ChildObjects>",
                &format!("<ChildObjects><{kind}>{name}</{kind}>"),
            ),
        )
        .unwrap();
        std::fs::create_dir_all(self.source.join(directory)).unwrap();
        std::fs::write(
            self.source.join(directory).join(format!("{name}.xml")),
            descriptor(kind, name, properties, children),
        )
        .unwrap();
    }

    fn admission(&self, request: &ApplyRequest) -> ApplyAdmission {
        let mut admission = self
            .actor
            .admit_apply(
                &self.binding,
                request.if_rev(),
                request.dry_run(),
                ProviderDeadline::from_budget(Duration::from_secs(5)),
                &CancellationToken::new(),
            )
            .unwrap();
        admission.bind_request_identity(
            &request.at().to_string(),
            request.ops().iter().map(|operation| {
                (
                    operation.name(),
                    operation.at().to_string(),
                    operation.args(),
                )
            }),
        );
        admission
    }

    fn prepare(&self, request: &ApplyRequest) -> PreparedApplyBatch {
        let admission = self.admission(request);
        let (staged, effects) = plan_hidden_v13_apply(request, &self.binding, &admission)
            .unwrap_or_else(|error| panic!("{error:?} at {:?}", error.path()));
        admission.prepare_with_effects(staged, effects).unwrap()
    }

    fn preview_and_publish(&self, at: &str, ops: Value) {
        let before = tree_snapshot(&self.source);
        let request = request(at, ops);
        let batch = self.prepare(&request);
        let projected = self.actor.preview_prepared_apply(&batch).unwrap();
        assert_eq!(projected.commit_count_for_test(), 0);
        assert_eq!(
            tree_snapshot(&self.source),
            before,
            "preview writes no source"
        );
        let mut preview = DomainResult::success("preview");
        preview.rev = Some(projected.rev().to_owned());
        let token = self.actor.save_prepared_apply(batch, preview).unwrap();
        let committed = self
            .actor
            .execute_saved_apply(
                &token,
                ProviderDeadline::from_budget(Duration::from_secs(5)),
                &CancellationToken::new(),
                |mut result, publication| {
                    let publication = publication.expect("the exact saved preview must publish");
                    result.rev = Some(publication.rev().to_owned());
                    result.data = Some(json!({"commits": publication.commit_count_for_test()}));
                    result
                },
            )
            .unwrap();
        assert!(committed.ok, "{}", committed.summary);
        assert_eq!(committed.data.as_ref().unwrap()["commits"], 1);
        assert_eq!(committed.rev.as_deref(), Some(projected.rev()));
    }

    fn refuse_preview(&self, at: &str, ops: Value, owner: &str, reason: &str) {
        let before = tree_snapshot(&self.source);
        let request = request(at, ops);
        let result = plan_hidden_v13_apply(&request, &self.binding, &self.admission(&request));
        let error = match result {
            Ok(_) => panic!("{owner}: invalid final object was accepted"),
            Err(error) => error,
        };
        assert_eq!(error.kind(), ApplyPlanErrorKind::BadValue, "{error:?}");
        assert_eq!(error.path(), Some(owner), "the refusal names the object");
        assert!(error.to_string().contains(reason), "{error:?}");
        assert_eq!(
            tree_snapshot(&self.source),
            before,
            "refusal writes no source"
        );
    }
}

fn request(at: &str, ops: Value) -> ApplyRequest {
    let args = json!({"at":at,"ops":ops,"dryRun":true});
    ApplyRequest::parse(args.as_object().unwrap(), &["main"]).unwrap()
}

fn create(kind: &str) -> Value {
    json!({"op":"object.create","args":{"values":{"kind":kind,"name":"Target"}}})
}

fn assert_member_names(path: &Path, kind: &str, names: &[&str]) {
    let text = std::fs::read_to_string(path).unwrap();
    let document = roxmltree::Document::parse(&text).unwrap();
    let children = document
        .root_element()
        .children()
        .find(|node| node.is_element())
        .unwrap()
        .children()
        .find(|node| node.has_tag_name((MD, "ChildObjects")))
        .unwrap();
    let actual = children
        .children()
        .filter(|node| node.has_tag_name((MD, kind)))
        .map(|node| {
            node.children()
                .find(|node| node.has_tag_name((MD, "Properties")))
                .unwrap()
                .children()
                .find(|node| node.has_tag_name((MD, "Name")))
                .unwrap()
                .text()
                .unwrap()
        })
        .collect::<Vec<_>>();
    assert_eq!(actual, names);
}

#[test]
fn canonical_preview_refuses_creating_objects_without_the_required_final_composition() {
    for (kind, reason) in [
        (
            "InformationRegister",
            "Register without dimensions, resources, and attributes",
        ),
        (
            "AccumulationRegister",
            "Register without dimensions, resources, and attributes",
        ),
        (
            "AccountingRegister",
            "The register doesn't have any resources specified",
        ),
        ("WebService", "Empty name space"),
    ] {
        let fixture = Fixture::new(SourceSetKind::Configuration);
        fixture.refuse_preview(
            "main:Configuration",
            json!([create(kind)]),
            &format!("main:{kind}.Target"),
            reason,
        );
    }
}

#[test]
fn canonical_preview_refuses_removing_the_last_required_register_field() {
    for (kind, directory, child, operation, reason) in [
        (
            "InformationRegister",
            "InformationRegisters",
            "Dimension",
            "dimension.remove",
            "Register without dimensions, resources, and attributes",
        ),
        (
            "AccumulationRegister",
            "AccumulationRegisters",
            "Attribute",
            "attribute.remove",
            "Register without dimensions, resources, and attributes",
        ),
        (
            "AccountingRegister",
            "AccountingRegisters",
            "Resource",
            "resource.remove",
            "The register doesn't have any resources specified",
        ),
    ] {
        let fixture = Fixture::new(SourceSetKind::Configuration);
        fixture.put(kind, directory, "", &member(child, "Only"));
        let owner = format!("main:{kind}.Target");
        fixture.refuse_preview(
            &format!("{owner}.{child}.Only"),
            json!([{"op":operation,"args":{}}]),
            &owner,
            reason,
        );
    }
}

#[test]
fn canonical_preview_refuses_clearing_a_web_service_namespace() {
    let fixture = Fixture::new(SourceSetKind::Configuration);
    fixture.put(
        "WebService",
        "WebServices",
        "<Namespace>urn:before</Namespace>",
        "",
    );
    fixture.refuse_preview(
        "main:WebService.Target",
        json!([{"op":"props.set","args":{"values":{"Namespace":"   "}}}]),
        "main:WebService.Target",
        "Empty name space",
    );
}

#[test]
fn canonical_create_then_complete_registers_previews_and_publishes_minimal_composition() {
    for (kind, directory, operation, child) in [
        (
            "InformationRegister",
            "InformationRegisters",
            "dimension.add",
            "Dimension",
        ),
        (
            "AccumulationRegister",
            "AccumulationRegisters",
            "attribute.add",
            "Attribute",
        ),
        (
            "AccountingRegister",
            "AccountingRegisters",
            "resource.add",
            "Resource",
        ),
    ] {
        let fixture = Fixture::new(SourceSetKind::Configuration);
        fixture.preview_and_publish("main:Configuration", json!([create(kind), {"op":operation,"args":{"at":format!("main:{kind}.Target"),"items":[{"name":"Only"}]}}]));
        assert_member_names(
            &fixture.source.join(directory).join("Target.xml"),
            child,
            &["Only"],
        );
    }
}

#[test]
fn canonical_create_then_set_namespace_previews_and_publishes_a_web_service() {
    let fixture = Fixture::new(SourceSetKind::Configuration);
    fixture.preview_and_publish("main:Configuration", json!([create("WebService"), {"op":"props.set","args":{"at":"main:WebService.Target","values":{"Namespace":"urn:accepted"}}}]));
    let text = std::fs::read_to_string(fixture.source.join("WebServices/Target.xml")).unwrap();
    let document = roxmltree::Document::parse(&text).unwrap();
    assert_eq!(
        document
            .descendants()
            .find(|node| node.has_tag_name((MD, "Namespace")))
            .unwrap()
            .text(),
        Some("urn:accepted")
    );
}

#[test]
fn canonical_integrity_uses_the_final_batch_after_an_intervening_other_family() {
    let fixture = Fixture::new(SourceSetKind::Configuration);
    fixture.put(
        "InformationRegister",
        "InformationRegisters",
        "",
        &member("Dimension", "Only"),
    );
    let module = fixture
        .source
        .join("InformationRegisters/Target/Ext/RecordSetModule.bsl");
    std::fs::create_dir_all(module.parent().unwrap()).unwrap();
    std::fs::write(&module, "// original\n").unwrap();
    fixture.preview_and_publish("main:InformationRegister.Target", json!([
        {"op":"dimension.remove","args":{"at":"main:InformationRegister.Target.Dimension.Only"}},
        {"op":"code.insert","args":{"at":"main:InformationRegister.Target.Module.RecordSet","text":"\n// intervening family\n"}},
        {"op":"dimension.add","args":{"at":"main:InformationRegister.Target","items":[{"name":"Replacement"}]}}
    ]));
    assert_member_names(
        &fixture.source.join("InformationRegisters/Target.xml"),
        "Dimension",
        &["Replacement"],
    );
    assert!(std::fs::read_to_string(module)
        .unwrap()
        .contains("intervening family"));
}

#[test]
fn canonical_creation_of_a_calculation_register_without_a_resource_remains_allowed() {
    let fixture = Fixture::new(SourceSetKind::Configuration);
    fixture.preview_and_publish("main:Configuration", json!([create("CalculationRegister")]));
    assert_member_names(
        &fixture.source.join("CalculationRegisters/Target.xml"),
        "Resource",
        &[],
    );
}

#[test]
fn canonical_object_removal_does_not_validate_a_deleted_descriptor() {
    let fixture = Fixture::new(SourceSetKind::Configuration);
    fixture.put("InformationRegister", "InformationRegisters", "", "");
    fixture.preview_and_publish(
        "main:InformationRegister.Target",
        json!([
            {"op":"object.remove","args":{}}
        ]),
    );
    assert!(!fixture
        .source
        .join("InformationRegisters/Target.xml")
        .exists());
}

#[test]
fn canonical_integrity_checks_own_extension_objects_but_preserves_inherited_composition() {
    for (kind, directory) in [
        ("InformationRegister", "InformationRegisters"),
        ("AccountingRegister", "AccountingRegisters"),
        ("WebService", "WebServices"),
    ] {
        let fixture = Fixture::new(SourceSetKind::Extension);
        fixture.put(kind, directory, "<ObjectBelonging>Adopted</ObjectBelonging><ExtendedConfigurationObject>aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa</ExtendedConfigurationObject>", "");
        fixture.preview_and_publish(
            &format!("main:{kind}.Target"),
            json!([{"op":"props.set","args":{"values":{"Comment":"local comment"}}}]),
        );
        let bytes = std::fs::read(fixture.source.join(directory).join("Target.xml")).unwrap();
        assert!(String::from_utf8(bytes).unwrap().contains("local comment"));
    }
    let own = Fixture::new(SourceSetKind::Extension);
    own.put(
        "InformationRegister",
        "InformationRegisters",
        "<ObjectBelonging>Adopted</ObjectBelonging>",
        &member("Dimension", "Only"),
    );
    own.refuse_preview(
        "main:InformationRegister.Target.Dimension.Only",
        json!([{"op":"dimension.remove","args":{}}]),
        "main:InformationRegister.Target",
        "Register without dimensions, resources, and attributes",
    );
}

#[test]
fn canonical_integrity_rejects_a_foreign_borrowing_marker_without_rejecting_a_metadata_prefix() {
    let own = Fixture::new(SourceSetKind::Extension);
    own.put(
        "InformationRegister",
        "InformationRegisters",
        "<x:ExtendedConfigurationObject xmlns:x=\"urn:foreign\">aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa</x:ExtendedConfigurationObject>",
        &member("Dimension", "Only"),
    );
    own.refuse_preview(
        "main:InformationRegister.Target.Dimension.Only",
        json!([{"op":"dimension.remove","args":{}}]),
        "main:InformationRegister.Target",
        "Register without dimensions, resources, and attributes",
    );

    let inherited = Fixture::new(SourceSetKind::Extension);
    inherited.put(
        "InformationRegister",
        "InformationRegisters",
        "<m:ExtendedConfigurationObject xmlns:m=\"http://v8.1c.ru/8.3/MDClasses\">aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa</m:ExtendedConfigurationObject>",
        "",
    );
    inherited.preview_and_publish(
        "main:InformationRegister.Target",
        json!([{"op":"props.set","args":{"values":{"Comment":"inherited through a namespace prefix"}}}]),
    );
}

#[test]
fn canonical_integrity_does_not_scan_unrelated_incomplete_objects() {
    let fixture = Fixture::new(SourceSetKind::Configuration);
    fixture.put("InformationRegister", "InformationRegisters", "", "");
    let preview = request(
        "main:Document.Records",
        json!([{"op":"props.set","args":{"values":{"Comment":"unrelated"}}}]),
    );
    let admission = fixture.admission(&preview);
    let (staged, _) = plan_hidden_v13_apply(&preview, &fixture.binding, &admission).unwrap();
    assert!(staged
        .planned_changes()
        .iter()
        .any(
            |change| change.relative_path == Path::new("Documents/Records.xml")
                && matches!(change.current, StagedFileState::Bytes(_))
        ));
    assert!(staged
        .planned_changes()
        .iter()
        .all(|change| change.relative_path != Path::new("InformationRegisters/Target.xml")));
}

/// Every top-level kind the canonical `object.create` writes, with the
/// directory of its descriptor.
const CREATABLE_KINDS: &[(&str, &str)] = &[
    ("Catalog", "Catalogs"),
    ("Document", "Documents"),
    ("Enum", "Enums"),
    ("Constant", "Constants"),
    ("InformationRegister", "InformationRegisters"),
    ("AccumulationRegister", "AccumulationRegisters"),
    ("AccountingRegister", "AccountingRegisters"),
    ("CalculationRegister", "CalculationRegisters"),
    ("ChartOfAccounts", "ChartsOfAccounts"),
    ("ChartOfCharacteristicTypes", "ChartsOfCharacteristicTypes"),
    ("ChartOfCalculationTypes", "ChartsOfCalculationTypes"),
    ("BusinessProcess", "BusinessProcesses"),
    ("Task", "Tasks"),
    ("ExchangePlan", "ExchangePlans"),
    ("DocumentJournal", "DocumentJournals"),
    ("Report", "Reports"),
    ("DataProcessor", "DataProcessors"),
    ("CommonModule", "CommonModules"),
    ("ScheduledJob", "ScheduledJobs"),
    ("EventSubscription", "EventSubscriptions"),
    ("HTTPService", "HTTPServices"),
    ("WebService", "WebServices"),
    ("DefinedType", "DefinedTypes"),
];

/// A vendor-support marker with changes disabled for the whole configuration.
const SUPPORT_LOCKED_MARKER: &str = concat!(
    "\u{feff}{6,1,1,dddddddd-dddd-dddd-dddd-dddddddddddd,0,",
    "eeeeeeee-eeee-eeee-eeee-eeeeeeeeeeee,\"1.0\",\"Vendor\",",
    "\"VendorConf\",0,0,0}"
);

impl Fixture {
    /// The prerequisites creation resolves from the source set: a task for a
    /// business process, a catalog as an event source and a common module
    /// whose exported procedures fit a scheduled job and an event handler.
    fn with_creation_prerequisites(kind: SourceSetKind, module: &str) -> Self {
        let fixture = Self::new(kind);
        fixture.put_named("Task", "Tasks", "Work", "", "");
        fixture.put_named("Catalog", "Catalogs", "Source", "", "");
        fixture.put_named(
            "CommonModule",
            "CommonModules",
            "Handlers",
            "<Server>true</Server>",
            "",
        );
        let module_path = fixture.source.join("CommonModules/Handlers/Ext/Module.bsl");
        std::fs::create_dir_all(module_path.parent().unwrap()).unwrap();
        std::fs::write(module_path, module).unwrap();
        fixture
    }

    fn save_preview(&self, at: &str, ops: Value) -> String {
        let batch = self.prepare(&request(at, ops));
        let projected = self.actor.preview_prepared_apply(&batch).unwrap();
        let mut preview = DomainResult::success("preview");
        preview.rev = Some(projected.rev().to_owned());
        self.actor.save_prepared_apply(batch, preview).unwrap()
    }

    fn execute_refused(&self, token: &str) -> ApplyPublicationError {
        let mut refusal = None;
        self.actor
            .execute_saved_apply(
                token,
                ProviderDeadline::from_budget(Duration::from_secs(5)),
                &CancellationToken::new(),
                |result, publication| {
                    refusal = Some(
                        publication
                            .map(|_| ())
                            .expect_err("a stale support decision must not publish"),
                    );
                    result
                },
            )
            .unwrap();
        refusal.unwrap()
    }
}

const CALLABLE_HANDLERS: &str = concat!(
    "Procedure Run() Export\n",
    "EndProcedure\n\n",
    "Procedure Handle(Source, Cancel) Export\n",
    "EndProcedure\n",
);

fn create_named(kind: &str, name: &str) -> Value {
    json!({"op":"object.create","args":{"values":{"kind":kind,"name":name}}})
}

/// The minimal same-batch completion a kind needs to be a valid final object;
/// kinds without a composition requirement need nothing.
fn completion(kind: &str, name: &str) -> Vec<Value> {
    let at = format!("main:{kind}.{name}");
    match kind {
        "InformationRegister" | "AccumulationRegister" | "AccountingRegister" => {
            vec![json!({"op":"resource.add","args":{"at":at,"items":[{"name":"Value"}]}})]
        }
        "WebService" => {
            vec![json!({"op":"props.set","args":{"at":at,"values":{"Namespace":"urn:unica:test"}}})]
        }
        _ => Vec::new(),
    }
}

fn create_ops(kind: &str, name: &str) -> Value {
    let mut ops = vec![create_named(kind, name)];
    ops.extend(completion(kind, name));
    Value::Array(ops)
}

#[test]
fn canonical_preview_of_every_creatable_kind_stages_descriptor_and_registration_without_writes() {
    // Список видов совпадает с перечнем модели: новый вид не выпадет молча.
    assert_eq!(
        CREATABLE_KINDS
            .iter()
            .map(|(kind, _)| *kind)
            .collect::<std::collections::BTreeSet<_>>(),
        unica_metadata::MetadataKind::ALL
            .iter()
            .map(|kind| kind.as_str())
            .collect::<std::collections::BTreeSet<_>>(),
    );
    let mut failures = Vec::new();
    for (kind, directory) in CREATABLE_KINDS {
        let fixture =
            Fixture::with_creation_prerequisites(SourceSetKind::Configuration, CALLABLE_HANDLERS);
        let name = format!("Preview{kind}");
        let before = tree_snapshot(&fixture.source);
        let request = request("main:Configuration", create_ops(kind, &name));
        let admission = fixture.admission(&request);
        let (staged, effects) = match plan_hidden_v13_apply(&request, &fixture.binding, &admission)
        {
            Ok(planned) => planned,
            Err(error) => {
                failures.push(format!("{kind}: {error:?}"));
                continue;
            }
        };
        let descriptor = PathBuf::from(directory).join(format!("{name}.xml"));
        let changed = staged
            .planned_changes()
            .into_iter()
            .map(|change| change.relative_path)
            .collect::<Vec<_>>();
        assert!(changed.contains(&descriptor), "{kind}: {changed:?}");
        assert!(
            changed.contains(&PathBuf::from("Configuration.xml")),
            "{kind}: the owner registration is planned: {changed:?}"
        );
        let batch = admission.prepare_with_effects(staged, effects).unwrap();
        let projected = fixture.actor.preview_prepared_apply(&batch).unwrap();
        assert_eq!(projected.commit_count_for_test(), 0, "{kind}");
        assert_eq!(
            tree_snapshot(&fixture.source),
            before,
            "{kind}: preview writes no source"
        );
        assert!(!fixture.source.join(&descriptor).exists(), "{kind}");
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

fn root_object(text: &str) -> roxmltree::Document<'_> {
    roxmltree::Document::parse(text.trim_start_matches('\u{feff}')).unwrap()
}

#[test]
fn canonical_publish_of_every_creatable_kind_writes_its_descriptor_and_refuses_a_duplicate_without_writes(
) {
    let fixture =
        Fixture::with_creation_prerequisites(SourceSetKind::Configuration, CALLABLE_HANDLERS);
    for (kind, directory) in CREATABLE_KINDS {
        let name = format!("Applied{kind}");
        fixture.preview_and_publish("main:Configuration", create_ops(kind, &name));
        let text =
            std::fs::read_to_string(fixture.source.join(directory).join(format!("{name}.xml")))
                .unwrap();
        let document = root_object(&text);
        let object = document
            .root_element()
            .children()
            .find(|node| node.is_element())
            .unwrap();
        assert_eq!(object.tag_name().name(), *kind);
        assert_eq!(
            object
                .descendants()
                .find(|node| node.has_tag_name((MD, "Name")))
                .and_then(|node| node.text()),
            Some(name.as_str()),
            "{kind}"
        );
        let property = |name: &str| {
            object
                .descendants()
                .find(|node| node.has_tag_name((MD, name)))
                .and_then(|node| node.text())
        };
        match *kind {
            "CommonModule" => assert_eq!(property("Server"), Some("true"), "{text}"),
            "ScheduledJob" => assert_eq!(
                property("MethodName"),
                Some("CommonModule.Handlers.Run"),
                "{text}"
            ),
            "EventSubscription" => {
                assert_eq!(
                    property("Handler"),
                    Some("CommonModule.Handlers.Handle"),
                    "{text}"
                );
                assert!(
                    text.contains("<v8:Type>cfg:CatalogObject."),
                    "the event source uses the object wire type: {text}"
                );
                assert!(!text.contains("cfg:CatalogRef."), "{text}");
            }
            _ => {}
        }
        let owner = std::fs::read_to_string(fixture.source.join("Configuration.xml")).unwrap();
        assert!(
            owner.contains(&format!("<{kind}>{name}</{kind}>")),
            "{kind}"
        );

        let before_duplicate = tree_snapshot(&fixture.source);
        let duplicate = request("main:Configuration", json!([create_named(kind, &name)]));
        let error =
            plan_hidden_v13_apply(&duplicate, &fixture.binding, &fixture.admission(&duplicate))
                .err()
                .unwrap_or_else(|| panic!("{kind}: duplicate creation was accepted"));
        assert_eq!(
            error.kind(),
            ApplyPlanErrorKind::InvalidState,
            "{kind}: {error:?}"
        );
        let message = error.to_string();
        assert!(
            message.contains(&name),
            "{kind}: the duplicate refusal names the existing object: {message}"
        );
        assert_eq!(
            tree_snapshot(&fixture.source),
            before_duplicate,
            "{kind}: a refused duplicate changes no source byte"
        );
    }
}

#[test]
fn canonical_preview_refuses_a_handler_kind_whose_common_module_method_is_missing() {
    for (kind, reason) in [
        (
            "ScheduledJob",
            "exported common-module procedure callable without arguments",
        ),
        (
            "EventSubscription",
            "exported common-module procedure with exactly 2 parameters",
        ),
    ] {
        let fixture = Fixture::with_creation_prerequisites(SourceSetKind::Configuration, "");
        let before = tree_snapshot(&fixture.source);
        let request = request("main:Configuration", json!([create_named(kind, "Denied")]));
        let error = plan_hidden_v13_apply(&request, &fixture.binding, &fixture.admission(&request))
            .err()
            .unwrap_or_else(|| panic!("{kind}: a dangling handler dependency was accepted"));
        assert_eq!(
            error.kind(),
            ApplyPlanErrorKind::BadValue,
            "{kind}: {error:?}"
        );
        assert_eq!(error.path(), Some("ops[0].args"), "{kind}");
        assert!(error.to_string().contains(reason), "{kind}: {error}");
        assert_eq!(tree_snapshot(&fixture.source), before, "{kind}");
    }
}

#[test]
fn canonical_preview_refuses_creation_in_a_support_locked_configuration_without_writes() {
    let fixture = Fixture::new(SourceSetKind::Configuration);
    let marker = fixture.source.join("Ext/ParentConfigurations.bin");
    std::fs::create_dir_all(marker.parent().unwrap()).unwrap();
    std::fs::write(&marker, SUPPORT_LOCKED_MARKER).unwrap();
    let before = tree_snapshot(&fixture.source);
    let request = request(
        "main:Configuration",
        json!([create_named("Catalog", "Locked")]),
    );
    let error = plan_hidden_v13_apply(&request, &fixture.binding, &fixture.admission(&request))
        .expect_err("a support-locked configuration must refuse creation");
    assert_eq!(error.kind(), ApplyPlanErrorKind::InvalidState, "{error:?}");
    assert!(error.to_string().contains("changes disabled"), "{error}");
    assert_eq!(tree_snapshot(&fixture.source), before);
}

/// A support lock that appears between the saved preview and its execution
/// invalidates the authorization the preview was planned under: execution
/// refuses, and the only byte that differs is the lock itself.
#[test]
fn canonical_saved_create_and_edit_refuse_a_support_lock_that_appears_before_execution() {
    for (at, ops, created) in [
        (
            "main:Configuration",
            json!([create_named("Catalog", "AuthorizationDrift")]),
            Some("Catalogs/AuthorizationDrift.xml"),
        ),
        (
            "main:Document.Records",
            json!([{"op":"props.set","args":{"values":{"Comment":"denied"}}}]),
            None,
        ),
    ] {
        for existing_ext in [false, true] {
            let fixture = Fixture::new(SourceSetKind::Configuration);
            let marker = fixture.source.join("Ext/ParentConfigurations.bin");
            if existing_ext {
                std::fs::create_dir_all(marker.parent().unwrap()).unwrap();
            }
            let token = fixture.save_preview(at, ops.clone());
            std::fs::create_dir_all(marker.parent().unwrap()).unwrap();
            std::fs::write(&marker, SUPPORT_LOCKED_MARKER).unwrap();
            let expected = tree_snapshot(&fixture.source);

            let refusal = fixture.execute_refused(&token);

            assert!(
                refusal.contains("Ext/ParentConfigurations.bin"),
                "{at}/{existing_ext}: the refusal names the changed support input: {refusal}"
            );
            assert_eq!(
                refusal.kind(),
                ApplyPublicationErrorKind::ContainmentIdentity,
                "{at}/{existing_ext}: {refusal}"
            );
            assert_eq!(tree_snapshot(&fixture.source), expected, "{at}");
            if let Some(created) = created {
                assert!(!fixture.source.join(created).exists());
            }
        }
    }
}

/// A support policy that changes after the saved preview is not silently
/// honoured by the old plan: execution refuses and a fresh plan decides under
/// the new policy.
#[test]
fn canonical_saved_edit_refuses_a_support_policy_changed_before_execution() {
    let fixture = Fixture::new(SourceSetKind::Configuration);
    let token = fixture.save_preview(
        "main:Document.Records",
        json!([{"op":"props.set","args":{"values":{"Comment":"warned"}}}]),
    );
    let marker = fixture.source.join("Ext/ParentConfigurations.bin");
    std::fs::create_dir_all(marker.parent().unwrap()).unwrap();
    std::fs::write(&marker, SUPPORT_LOCKED_MARKER).unwrap();
    std::fs::write(
        fixture.source.parent().unwrap().join(".v8-project.json"),
        r#"{"editingAllowedCheck":"warn"}"#,
    )
    .unwrap();
    let expected = tree_snapshot(&fixture.source);

    let refusal = fixture.execute_refused(&token);

    assert_eq!(
        refusal.kind(),
        ApplyPublicationErrorKind::ContainmentIdentity,
        "{refusal}"
    );
    assert!(refusal.contains("support-policy"), "{refusal}");
    assert_eq!(tree_snapshot(&fixture.source), expected);

    let fresh = request(
        "main:Document.Records",
        json!([{"op":"props.set","args":{"values":{"Comment":"warned"}}}]),
    );
    let (staged, _) = plan_hidden_v13_apply(&fresh, &fixture.binding, &fixture.admission(&fresh))
        .expect("the warn policy permits editing a locked configuration");
    assert!(!staged.planned_changes().is_empty());
}

#[test]
fn canonical_create_refuses_a_property_the_new_kind_does_not_have_without_writes() {
    let fixture = Fixture::new(SourceSetKind::Configuration);
    let before = tree_snapshot(&fixture.source);
    let request = request(
        "main:Configuration",
        json!([
            create_named("Catalog", "RejectedProperty"),
            {"op":"props.set","args":{"at":"main:Catalog.RejectedProperty","values":{"NumberLength":12}}}
        ]),
    );
    let error = plan_hidden_v13_apply(&request, &fixture.binding, &fixture.admission(&request))
        .expect_err("a property foreign to Catalog must refuse");
    assert_eq!(error.kind(), ApplyPlanErrorKind::BadValue, "{error:?}");
    assert!(error.to_string().contains("NumberLength"), "{error}");
    assert_eq!(tree_snapshot(&fixture.source), before);
}
