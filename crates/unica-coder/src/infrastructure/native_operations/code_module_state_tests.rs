use super::{parse_code_plan_operation, plan_code_batch, CodePlanOperation};
use crate::domain::{
    cancellation::CancellationToken,
    code_intelligence::ProviderDeadline,
    events::DomainEventKind,
    project_sources::{SourceFormat, SourceProfile, SourceSetKind},
    workspace::WorkspaceContext,
};
use crate::infrastructure::{
    native_operations::apply::{ApplyPlanError, ApplyStagedState, PlannedApplyEffects},
    workspace_actor::{
        ApplyAdmission, ProviderRootBinding, WorkspaceActor, WorkspaceIdentity,
        WorkspaceSourceSetInput,
    },
};
use serde_json::json;
use std::{fs, path::PathBuf, time::Duration};

const MD: &str = "http://v8.1c.ru/8.3/MDClasses";
const XR: &str = "http://v8.1c.ru/8.3/xcf/readable";
const BEFORE: &[u8] = b"Procedure Base()\nEndProcedure\n";

struct Fixture {
    _temp: tempfile::TempDir,
    source: PathBuf,
    actor: WorkspaceActor,
    binding: ProviderRootBinding,
    descriptor: PathBuf,
    module: PathBuf,
    at: String,
}

impl Fixture {
    fn new(
        kind: &str,
        directory: &str,
        terminal: &str,
        source_kind: SourceSetKind,
        belonging: &str,
    ) -> Self {
        let temp = tempfile::tempdir().unwrap();
        let root = fs::canonicalize(temp.path()).unwrap();
        let source = root.join("src");
        let descriptor = PathBuf::from(format!("{directory}/Sample.xml"));
        let module = PathBuf::from(format!("{directory}/Sample/Ext/{terminal}.bsl"));
        fs::create_dir_all(source.join(module.parent().unwrap())).unwrap();
        fs::create_dir_all(source.join("Ext")).unwrap();
        let extension = source_kind == SourceSetKind::Extension;
        fs::write(
            root.join("v8project.yaml"),
            format!(
                "format: DESIGNER\nsource-set:\n  - name: ext\n    type: {}\n    path: src\n",
                if extension {
                    "EXTENSION"
                } else {
                    "CONFIGURATION"
                }
            ),
        )
        .unwrap();
        fs::write(source.join("Configuration.xml"), format!(
            "<MetaDataObject xmlns=\"{MD}\" version=\"2.20\"><Configuration uuid=\"aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa\"><InternalInfo/><Properties><Name>Extension</Name>{}</Properties><ChildObjects><{kind}>Sample</{kind}></ChildObjects></Configuration></MetaDataObject>",
            if extension { "<ObjectBelonging>Adopted</ObjectBelonging><ConfigurationExtensionPurpose>Customization</ConfigurationExtensionPurpose><NamePrefix>E_</NamePrefix>" } else { "" }
        )).unwrap();
        fs::write(source.join(&descriptor), format!(
            "<MetaDataObject xmlns=\"{MD}\" xmlns:xr=\"{XR}\" version=\"2.20\"><{kind} uuid=\"bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb\"><InternalInfo/><Properties><Name>Sample</Name><ObjectBelonging>{belonging}</ObjectBelonging><ExtendedConfigurationObject>cccccccc-cccc-cccc-cccc-cccccccccccc</ExtendedConfigurationObject></Properties><ChildObjects/></{kind}></MetaDataObject>"
        )).unwrap();
        fs::write(source.join(&module), BEFORE).unwrap();
        let context = WorkspaceContext {
            cwd: root.clone(),
            workspace_root: root.clone(),
            cache_root: root.join(".build/unica"),
            workspace_epoch: 1,
        };
        let identity = WorkspaceIdentity::new(
            &context,
            [WorkspaceSourceSetInput::new(
                "ext",
                &source,
                source_kind,
                SourceFormat::PlatformXml,
                SourceProfile::platform_xml_8_3_27_format_2_20(),
            )],
            "module-state-test",
        )
        .unwrap();
        let actor = WorkspaceActor::new(identity, context).unwrap();
        let binding = actor.bind_provider_root("ext", &source).unwrap();
        let suffix = match terminal {
            "Module" => String::new(),
            "CommandModule" => ".Module.Command".to_owned(),
            other => format!(".Module.{}", other.strip_suffix("Module").unwrap()),
        };
        Self {
            _temp: temp,
            source,
            actor,
            binding,
            descriptor,
            module,
            at: format!("ext:{kind}.Sample{suffix}"),
        }
    }

    fn common() -> Self {
        Self::new(
            "CommonModule",
            "CommonModules",
            "Module",
            SourceSetKind::Extension,
            "Adopted",
        )
    }

    fn operation(&self, replace: bool) -> CodePlanOperation {
        let mut args = json!({"at":self.at,"text":"Procedure Added()\nEndProcedure"});
        if replace {
            args["selector"] = json!({"method":"Base"});
        }
        parse_code_plan_operation(
            if replace {
                "code.replace"
            } else {
                "code.insert"
            },
            &args,
            0,
            &self.binding,
        )
        .unwrap()
    }

    fn admission(&self, dry_run: bool) -> ApplyAdmission {
        self.actor
            .admit_apply(
                &self.binding,
                None,
                dry_run,
                ProviderDeadline::from_budget(Duration::from_secs(10)),
                &CancellationToken::new(),
            )
            .unwrap()
    }

    fn plan(
        &self,
        admission: &ApplyAdmission,
        ops: &[CodePlanOperation],
    ) -> Result<(ApplyStagedState, PlannedApplyEffects), ApplyPlanError> {
        plan_code_batch(
            admission.staged_state().unwrap(),
            admission.code_planning_authority(&self.binding)?,
            ops,
        )
    }

    fn descriptor_bytes(&self) -> Vec<u8> {
        fs::read(self.source.join(&self.descriptor)).unwrap()
    }
}

fn assert_state(bytes: &[u8], property: &str) {
    let text = std::str::from_utf8(bytes).unwrap();
    let doc = roxmltree::Document::parse(text).unwrap();
    let states: Vec<_> = doc
        .descendants()
        .filter(|n| n.has_tag_name((XR, "PropertyState")))
        .collect();
    assert_eq!(
        states.len(),
        1,
        "successful BSL write must connect the borrowed module: {text}"
    );
    assert_eq!(
        states[0]
            .children()
            .find(|n| n.has_tag_name((XR, "Property")))
            .and_then(|n| n.text()),
        Some(property)
    );
    assert_eq!(
        states[0]
            .children()
            .find(|n| n.has_tag_name((XR, "State")))
            .and_then(|n| n.text()),
        Some("Extended")
    );
}

#[test]
fn borrowed_code_insert_and_replace_stage_module_state_atomically() {
    for replace in [false, true] {
        let fixture = Fixture::common();
        let before = fixture.descriptor_bytes();
        let admission = fixture.admission(false);
        let (mut staged, effects) = fixture
            .plan(&admission, &[fixture.operation(replace)])
            .unwrap();
        assert_state(
            &staged.read(&fixture.descriptor).unwrap().unwrap(),
            "Module",
        );
        assert_eq!(staged.planned_changes().len(), 2);
        assert!(effects
            .events()
            .iter()
            .any(|e| e.kind == DomainEventKind::MetadataChanged));
        assert_eq!(fixture.descriptor_bytes(), before);
        fixture
            .actor
            .publish_prepared_apply(admission.prepare_with_effects(staged, effects).unwrap())
            .unwrap();
        assert_state(&fixture.descriptor_bytes(), "Module");
        assert_ne!(
            fs::read(fixture.source.join(&fixture.module)).unwrap(),
            BEFORE
        );
    }
}

#[test]
fn borrowed_code_direct_roles_and_common_command_use_their_property() {
    for (kind, directory, terminal) in [
        ("Document", "Documents", "ObjectModule"),
        ("Catalog", "Catalogs", "ManagerModule"),
        (
            "InformationRegister",
            "InformationRegisters",
            "RecordSetModule",
        ),
        ("Constant", "Constants", "ValueManagerModule"),
        ("CommonCommand", "CommonCommands", "CommandModule"),
    ] {
        let fixture = Fixture::new(
            kind,
            directory,
            terminal,
            SourceSetKind::Extension,
            "Adopted",
        );
        let admission = fixture.admission(true);
        let (mut staged, _) = fixture
            .plan(&admission, &[fixture.operation(false)])
            .unwrap();
        assert_state(
            &staged.read(&fixture.descriptor).unwrap().unwrap(),
            terminal,
        );
    }
}

#[test]
fn borrowed_code_preview_is_write_free_and_repeat_keeps_one_state() {
    let fixture = Fixture::common();
    fs::remove_file(fixture.source.join(&fixture.module)).unwrap();
    let before = fixture.descriptor_bytes();
    let operation = fixture.operation(false);
    let preview = fixture.admission(true);
    let (mut state, effects) = fixture
        .plan(&preview, std::slice::from_ref(&operation))
        .unwrap();
    let expected = state.read(&fixture.descriptor).unwrap().unwrap();
    assert_state(&expected, "Module");
    fixture
        .actor
        .publish_prepared_apply(preview.prepare_with_effects(state, effects).unwrap())
        .unwrap();
    assert_eq!(fixture.descriptor_bytes(), before);
    assert!(!fixture.source.join(&fixture.module).exists());
    for repeat in [false, true] {
        let admission = fixture.admission(false);
        let (state, effects) = fixture
            .plan(&admission, std::slice::from_ref(&operation))
            .unwrap();
        assert_eq!(state.planned_changes().is_empty(), repeat);
        fixture
            .actor
            .publish_prepared_apply(admission.prepare_with_effects(state, effects).unwrap())
            .unwrap();
        assert_eq!(fixture.descriptor_bytes(), expected);
    }
}

#[test]
fn borrowed_code_publication_failure_restores_bsl_descriptor_cache_and_revision() {
    use crate::infrastructure::native_operations::compile_transaction::{
        set_retained_apply_failpoint, RetainedApplyFailpoint,
    };
    let fixture = Fixture::common();
    let before = fixture.descriptor_bytes();
    let service = fixture
        .actor
        .source_revision_service(&fixture.binding)
        .unwrap();
    let machine_before = service.machine_state_for_test();
    let admission = fixture.admission(false);
    let (mut staged, effects) = fixture
        .plan(&admission, &[fixture.operation(false)])
        .unwrap();
    assert_state(
        &staged.read(&fixture.descriptor).unwrap().unwrap(),
        "Module",
    );
    let prepared = admission.prepare_with_effects(staged, effects).unwrap();
    set_retained_apply_failpoint(RetainedApplyFailpoint::AfterAllPostimages);
    assert!(fixture.actor.publish_prepared_apply(prepared).is_err());
    assert_eq!(fixture.descriptor_bytes(), before);
    assert_eq!(
        fs::read(fixture.source.join(&fixture.module)).unwrap(),
        BEFORE
    );
    assert_eq!(service.machine_state_for_test(), machine_before);
    assert!(!fixture
        .source
        .parent()
        .unwrap()
        .join(".build/unica/state.json")
        .exists());
}

#[test]
fn borrowed_code_rejects_incompatible_state_without_writes() {
    let fixture = Fixture::common();
    let text = String::from_utf8(fixture.descriptor_bytes()).unwrap().replace("<InternalInfo/>", "<InternalInfo><xr:PropertyState><xr:Property>Module</xr:Property><xr:State>Notify</xr:State></xr:PropertyState></InternalInfo>");
    fs::write(fixture.source.join(&fixture.descriptor), &text).unwrap();
    for dry_run in [true, false] {
        let admission = fixture.admission(dry_run);
        assert!(fixture
            .plan(&admission, &[fixture.operation(false)])
            .is_err());
        assert_eq!(fixture.descriptor_bytes(), text.as_bytes());
        assert_eq!(
            fs::read(fixture.source.join(&fixture.module)).unwrap(),
            BEFORE
        );
    }
}

#[test]
fn code_does_not_mark_configuration_or_owned_extension_objects() {
    for (kind, belonging) in [
        (SourceSetKind::Configuration, "Adopted"),
        (SourceSetKind::Extension, "Own"),
    ] {
        let fixture = Fixture::new("CommonModule", "CommonModules", "Module", kind, belonging);
        let before = fixture.descriptor_bytes();
        let admission = fixture.admission(false);
        let (state, effects) = fixture
            .plan(&admission, &[fixture.operation(false)])
            .unwrap();
        assert_eq!(state.planned_changes().len(), 1);
        fixture
            .actor
            .publish_prepared_apply(admission.prepare_with_effects(state, effects).unwrap())
            .unwrap();
        assert_eq!(fixture.descriptor_bytes(), before);
    }
}

#[test]
fn borrowed_code_repairs_missing_state_when_bsl_is_already_present() {
    let fixture = Fixture::common();
    fs::write(
        fixture.source.join(&fixture.module),
        b"Procedure Added()\nEndProcedure\n",
    )
    .unwrap();
    let admission = fixture.admission(false);
    let (state, effects) = fixture
        .plan(&admission, &[fixture.operation(false)])
        .unwrap();
    assert_eq!(state.planned_changes().len(), 1);
    assert_eq!(effects.events().len(), 1);
    assert_eq!(effects.events()[0].kind, DomainEventKind::MetadataChanged);
    fixture
        .actor
        .publish_prepared_apply(admission.prepare_with_effects(state, effects).unwrap())
        .unwrap();
    assert_state(&fixture.descriptor_bytes(), "Module");
}

#[test]
fn borrowed_code_nested_form_and_command_mark_the_child_descriptor() {
    for (kind, directory, role, module_tail, property) in [
        ("Form", "Forms", "Form", "Ext/Form/Module.bsl", "Form"),
        (
            "Command",
            "Commands",
            "Command",
            "Ext/CommandModule.bsl",
            "CommandModule",
        ),
    ] {
        let mut fixture = Fixture::new(
            "Document",
            "Documents",
            "ObjectModule",
            SourceSetKind::Extension,
            "Adopted",
        );
        let parent = fixture.descriptor_bytes();
        let parent_text = String::from_utf8(parent).unwrap().replace(
            "<ChildObjects/>",
            &format!("<ChildObjects><{kind}>Main</{kind}></ChildObjects>"),
        );
        fs::write(fixture.source.join(&fixture.descriptor), &parent_text).unwrap();
        let parent_path = fixture.descriptor.clone();
        fixture.descriptor = PathBuf::from(format!("Documents/Sample/{directory}/Main.xml"));
        fixture.module = PathBuf::from(format!("Documents/Sample/{directory}/Main/{module_tail}"));
        fs::create_dir_all(fixture.source.join(fixture.module.parent().unwrap())).unwrap();
        fs::write(fixture.source.join(&fixture.descriptor), format!("<MetaDataObject xmlns=\"{MD}\" version=\"2.20\"><{kind} uuid=\"dddddddd-dddd-dddd-dddd-dddddddddddd\"><InternalInfo/><Properties><Name>Main</Name><ObjectBelonging>Adopted</ObjectBelonging></Properties></{kind}></MetaDataObject>")).unwrap();
        fs::write(fixture.source.join(&fixture.module), BEFORE).unwrap();
        fixture.at = format!("ext:Document.Sample.{kind}.Main.Module.{role}");
        let admission = fixture.admission(false);
        let (mut state, effects) = fixture
            .plan(&admission, &[fixture.operation(false)])
            .unwrap();
        assert_state(&state.read(&fixture.descriptor).unwrap().unwrap(), property);
        assert!(effects
            .events()
            .iter()
            .any(|e| e.kind == DomainEventKind::MetadataChanged
                && e.artifact == format!("ext:Document.Sample.{kind}.Main")));
        fixture
            .actor
            .publish_prepared_apply(admission.prepare_with_effects(state, effects).unwrap())
            .unwrap();
        assert_eq!(
            fs::read(fixture.source.join(parent_path)).unwrap(),
            parent_text.as_bytes()
        );
    }
}

#[test]
fn borrowed_code_existing_state_is_byte_preserved_and_descriptor_races_refuse() {
    let fixture = Fixture::common();
    let before = String::from_utf8(fixture.descriptor_bytes()).unwrap().replace("<InternalInfo/>", "<InternalInfo><xr:PropertyState><xr:Property>Module</xr:Property><xr:State>Extended</xr:State></xr:PropertyState></InternalInfo>");
    fs::write(fixture.source.join(&fixture.descriptor), &before).unwrap();
    let admission = fixture.admission(false);
    let (state, effects) = fixture
        .plan(&admission, &[fixture.operation(false)])
        .unwrap();
    assert_eq!(state.planned_changes().len(), 1);
    assert!(effects
        .events()
        .iter()
        .all(|e| e.kind == DomainEventKind::ModuleChanged));
    let prepared = admission.prepare_with_effects(state, effects).unwrap();
    let concurrent = before.replace(
        "<Name>Sample</Name>",
        "<Name>Sample</Name><Comment>concurrent</Comment>",
    );
    fs::write(fixture.source.join(&fixture.descriptor), &concurrent).unwrap();
    assert!(fixture.actor.publish_prepared_apply(prepared).is_err());
    assert_eq!(fixture.descriptor_bytes(), concurrent.as_bytes());
    assert_eq!(
        fs::read(fixture.source.join(&fixture.module)).unwrap(),
        BEFORE
    );
}
