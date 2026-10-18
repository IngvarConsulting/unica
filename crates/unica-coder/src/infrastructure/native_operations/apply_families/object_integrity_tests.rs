use super::plan_hidden_v13_apply;
use crate::domain::apply::ApplyRequest;
use crate::domain::cancellation::CancellationToken;
use crate::domain::code_intelligence::ProviderDeadline;
use crate::domain::invocation::DomainResult;
use crate::domain::project_sources::{SourceFormat, SourceProfile, SourceSetKind};
use crate::domain::workspace::WorkspaceContext;
use crate::infrastructure::native_operations::apply::{ApplyPlanErrorKind, StagedFileState};
use crate::infrastructure::workspace_actor::{
    ApplyAdmission, PreparedApplyBatch, ProviderRootBinding, WorkspaceActor, WorkspaceIdentity,
    WorkspaceSourceSetInput,
};
use crate::test_support::tree_snapshot;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::time::Duration;

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
        let actor = WorkspaceActor::new(identity, context).unwrap();
        let binding = actor.bind_provider_root("main", &source).unwrap();
        Self {
            _root: root,
            source,
            actor,
            binding,
        }
    }

    fn put(&self, kind: &str, directory: &str, properties: &str, children: &str) {
        let configuration = self.source.join("Configuration.xml");
        let original = std::fs::read_to_string(&configuration).unwrap();
        std::fs::write(
            configuration,
            original.replace(
                "<ChildObjects>",
                &format!("<ChildObjects><{kind}>Target</{kind}>"),
            ),
        )
        .unwrap();
        std::fs::create_dir_all(self.source.join(directory)).unwrap();
        std::fs::write(
            self.source.join(directory).join("Target.xml"),
            descriptor(kind, "Target", properties, children),
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
        admission.bind_request(request);
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
