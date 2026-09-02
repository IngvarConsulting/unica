pub(crate) mod dcs_mxl;
pub(crate) mod form_resource;
pub(crate) mod metadata;
mod request;

use crate::domain::apply::{ApplyRequest, OperationFamily};
use crate::domain::project_sources::{SourceFormat, SourceSetKind};
use crate::infrastructure::native_operations::apply::{
    hidden_apply_family_unimplemented, ApplyPlanError, ApplyPlanErrorKind, ApplyStagedState,
    PlannedApplyEffects,
};
use crate::infrastructure::workspace_actor::{ApplyAdmission, ProviderRootBinding};
use request::{reconcile_effects, IndexedPlanOperation, ProvisionalApplyEffect};

/// The writable platform XML profile is single (`DEC.2026-08-21.SINGLE-WRITABLE-PLATFORM-XML-PROFILE`):
/// a write outside it refuses before the first byte. The retired v0.12
/// mutators proved this through the dispatcher's format guard; the canonical
/// planner proves it here, over the owner chain of the addressed node, read
/// through the staged state so no ambient file is touched after admission.
fn refuse_targets_outside_writable_profile(
    request: &ApplyRequest,
    binding: &ProviderRootBinding,
    staged: &mut ApplyStagedState,
) -> Result<(), ApplyPlanError> {
    let at = request.at().to_string();
    for relative in writable_profile_owner_chain(request, binding.source_kind()) {
        let Some(bytes) = staged
            .read(&relative)
            .map_err(|error| ApplyPlanError::staging(error, at.clone()))?
        else {
            continue;
        };
        if let Some(finding) =
            crate::infrastructure::format_guard::classify_staged_platform_xml_root(
                &relative, &bytes,
            )
        {
            return Err(
                ApplyPlanError::new(ApplyPlanErrorKind::InvalidSource, finding.message).at_path(at),
            );
        }
    }
    Ok(())
}

/// The platform XML documents whose root version decides whether the
/// addressed node may be written: the source-set root descriptor, the owner
/// descriptor of every metadata segment, the wrapper of a nested form,
/// template or command, and the attached content the kind owns.
fn writable_profile_owner_chain(
    request: &ApplyRequest,
    source_kind: SourceSetKind,
) -> Vec<std::path::PathBuf> {
    use crate::domain::address::NodeKind;
    use crate::domain::source_target::{MetadataAddress, PLATFORM_XML_8_3_27_FORMAT_2_20};
    use crate::infrastructure::logical_event_source::{
        attached_resource_relative, metadata_descriptor_relative,
    };

    let mut chain = Vec::new();
    if matches!(
        source_kind,
        SourceSetKind::Configuration | SourceSetKind::Extension
    ) {
        chain.push(std::path::PathBuf::from("Configuration.xml"));
    }
    let segments = request.at().segments();
    let Some(owner) = segments.first() else {
        return chain;
    };
    let Some(owner_name) = owner.name() else {
        return chain;
    };
    if owner.kind() == NodeKind::Configuration || !owner.kind().is_metadata_kind() {
        return chain;
    }
    let owner_path = format!("{}.{}", owner.kind().as_str(), owner_name);
    let Ok(owner_address) = MetadataAddress::parse(PLATFORM_XML_8_3_27_FORMAT_2_20, &owner_path)
    else {
        return chain;
    };
    if let Ok(descriptor) = metadata_descriptor_relative(&owner_address, source_kind) {
        chain.push(descriptor);
    }
    let owner_resource = match owner.kind() {
        NodeKind::Role => Some("Rights.xml"),
        NodeKind::Subsystem => Some("CommandInterface.xml"),
        NodeKind::CommonForm => Some("Form.xml"),
        _ => None,
    };
    if let Some(resource) = owner_resource {
        if let Ok(content) = attached_resource_relative(&owner_address, resource, source_kind) {
            chain.push(content);
        }
    }
    if let Some(nested) = segments.get(1) {
        let nested_resource = match nested.kind() {
            NodeKind::Form => Some("Form.xml"),
            NodeKind::Template => Some("Template.xml"),
            _ => None,
        };
        if let (Some(resource), Some(nested_name)) = (nested_resource, nested.name()) {
            let nested_path = format!("{owner_path}.{}.{nested_name}", nested.kind().as_str());
            if let Ok(nested_address) =
                MetadataAddress::parse(PLATFORM_XML_8_3_27_FORMAT_2_20, &nested_path)
            {
                if let Ok(wrapper) = metadata_descriptor_relative(&nested_address, source_kind) {
                    chain.push(wrapper);
                }
                if let Ok(content) =
                    attached_resource_relative(&nested_address, resource, source_kind)
                {
                    chain.push(content);
                }
            }
        }
    }
    chain
}

enum ParsedApplyOperation {
    Metadata(IndexedPlanOperation<metadata::MetadataPlanOperation>),
    FormResource(IndexedPlanOperation<form_resource::FormResourcePlanOperation>),
    DcsMxl(IndexedPlanOperation<dcs_mxl::DcsMxlPlanOperation>),
    Code(IndexedPlanOperation<crate::infrastructure::native_operations::code::CodePlanOperation>),
    Xdto(IndexedPlanOperation<crate::infrastructure::native_operations::xdto::XdtoPlanOperation>),
    Event(
        IndexedPlanOperation<crate::infrastructure::native_operations::event::EventImplementArgs>,
    ),
    Unsupported(usize),
}

impl ParsedApplyOperation {
    /// Consecutive operations of one family plan as a single batch.
    fn same_family(&self, other: &Self) -> bool {
        std::mem::discriminant(self) == std::mem::discriminant(other)
    }
}

/// A planner that already reconciled its events against its own staged
/// changes reports them tied to every path the batch changed: the request
/// finalizer then keeps an event only while all of those paths stay changed
/// in the final postimage.
/// Object and manager events by their Russian names, as developers write them.
const RUSSIAN_EVENT_NAMES: &[(&str, &str)] = &[
    ("ПередЗаписью", "BeforeWrite"),
    ("ПриЗаписи", "OnWrite"),
    ("ПередУдалением", "BeforeDelete"),
    ("ОбработкаЗаполнения", "Filling"),
    ("ОбработкаПроверкиЗаполнения", "FillCheckProcessing"),
    ("ПриКопировании", "OnCopy"),
    ("ПриУстановкеНовогоНомера", "OnSetNewNumber"),
    ("ПриУстановкеНовогоКода", "OnSetNewCode"),
    ("ОбработкаПроведения", "Posting"),
    ("ОбработкаУдаленияПроведения", "UndoPosting"),
    ("ОбработкаПолученияДанныхВыбора", "ChoiceDataGetProcessing"),
    ("ОбработкаПолученияФормы", "FormGetProcessing"),
    (
        "ОбработкаПолученияПолейПредставления",
        "PresentationFieldsGetProcessing",
    ),
    (
        "ОбработкаПолученияПредставления",
        "PresentationGetProcessing",
    ),
    (
        "ОбработкаФормированияИзВерсииИсторииДанных",
        "GenerateFromDataHistoryVersionProcessing",
    ),
    (
        "ОбработкаПослеЗаписиВерсийИсторииДанных",
        "AfterWriteDataHistoryVersionsProcessing",
    ),
];

const MANAGER_EVENTS: &[&str] = &[
    "ChoiceDataGetProcessing",
    "FormGetProcessing",
    "PresentationFieldsGetProcessing",
    "PresentationGetProcessing",
    "AfterWriteDataHistoryVersionsProcessing",
];

/// `Owner.Name.Event.ПриЗаписи` is how a developer names an object event; the
/// event planner addresses the module that hosts it
/// (`Owner.Name.Module.Object.Event.OnWrite`). Both spellings are accepted:
/// the module role follows the owner kind and the event, and Russian event
/// names map onto the registry. Anything else passes through unchanged.
fn canonical_event_args(args: serde_json::Value) -> serde_json::Value {
    use crate::domain::address::{NodeKind, QualifiedAddress};
    let Some(raw) = args.get("at").and_then(serde_json::Value::as_str) else {
        return args;
    };
    let Ok(address) = QualifiedAddress::parse(raw) else {
        return args;
    };
    let segments = address.segments();
    let english = |name: &str| -> String {
        RUSSIAN_EVENT_NAMES
            .iter()
            .find(|(russian, _)| *russian == name)
            .map_or_else(|| name.to_string(), |(_, english)| (*english).to_string())
    };
    let rewritten = match segments {
        [owner, event] if event.kind() == NodeKind::Event => {
            let (Some(owner_name), Some(event_name)) = (owner.name(), event.name()) else {
                return args;
            };
            let event_name = english(event_name);
            let role = if MANAGER_EVENTS.contains(&event_name.as_str()) {
                "Manager"
            } else {
                match owner.kind() {
                    NodeKind::InformationRegister
                    | NodeKind::AccumulationRegister
                    | NodeKind::AccountingRegister
                    | NodeKind::CalculationRegister => "RecordSet",
                    NodeKind::Constant => "ValueManager",
                    _ => "Object",
                }
            };
            format!(
                "{}:{}.{owner_name}.Module.{role}.Event.{event_name}",
                address.source_set(),
                owner.kind().as_str()
            )
        }
        [owner, module, role, event]
            if module.kind() == NodeKind::Module && event.kind() == NodeKind::Event =>
        {
            let (Some(owner_name), Some(role_name), Some(event_name)) = (
                owner.name(),
                role.name().or(Some(role.kind().as_str())),
                event.name(),
            ) else {
                return args;
            };
            format!(
                "{}:{}.{owner_name}.Module.{role_name}.Event.{}",
                address.source_set(),
                owner.kind().as_str(),
                english(event_name)
            )
        }
        _ => return args,
    };
    let mut args = args;
    if let Some(object) = args.as_object_mut() {
        object.insert("at".to_string(), serde_json::Value::String(rewritten));
    }
    args
}

/// The event planner reports through its own error type; the apply seam
/// speaks one refusal vocabulary, so the kinds map one to one.
fn event_plan_error_to_apply(
    error: crate::infrastructure::native_operations::event::EventPlanError,
) -> ApplyPlanError {
    use crate::infrastructure::native_operations::event::EventPlanErrorKind;
    let kind = match error.kind() {
        EventPlanErrorKind::BadValue => ApplyPlanErrorKind::BadValue,
        EventPlanErrorKind::NotFound => ApplyPlanErrorKind::NotFound,
        EventPlanErrorKind::ProviderUnavailable => ApplyPlanErrorKind::ProviderUnavailable,
        EventPlanErrorKind::InvalidState => ApplyPlanErrorKind::InvalidState,
        EventPlanErrorKind::InvalidSource => ApplyPlanErrorKind::InvalidSource,
        EventPlanErrorKind::Staging(kind) => ApplyPlanErrorKind::Staging(kind),
        EventPlanErrorKind::Postcondition => ApplyPlanErrorKind::Postcondition,
    };
    let path = error.path().map(str::to_string);
    let planned = ApplyPlanError::new(kind, error.to_string());
    match path {
        Some(path) => planned.at_path(path),
        None => planned,
    }
}

fn batch_effects_as_provisional(
    before: &[std::path::PathBuf],
    staged: &ApplyStagedState,
    effects: PlannedApplyEffects,
    op_index: usize,
) -> Vec<ProvisionalApplyEffect> {
    let changed = staged
        .planned_changes()
        .into_iter()
        .map(|change| change.relative_path)
        .filter(|path| !before.contains(path))
        .collect::<Vec<_>>();
    effects
        .into_events_with_paths()
        .into_iter()
        .map(|(event, paths)| {
            // A planner that named the file behind its event keeps that
            // association; only an anonymous event spans the whole batch.
            if paths.is_empty() {
                ProvisionalApplyEffect::spanning(changed.clone(), event, op_index)
            } else {
                ProvisionalApplyEffect::spanning(paths, event, op_index)
            }
        })
        .collect()
}

fn validate_platform_xml_binding(
    binding: &ProviderRootBinding,
    op_index: usize,
) -> Result<(), ApplyPlanError> {
    if binding.source_format() != SourceFormat::PlatformXml
        || !matches!(
            binding.source_kind(),
            SourceSetKind::Configuration | SourceSetKind::Extension
        )
        || binding.source_profile().platform_profile().is_none()
        || binding.source_profile().serialization_format().is_none()
    {
        return Err(ApplyPlanError::new(
            ApplyPlanErrorKind::ProviderUnavailable,
            "admitted source has no writable exact Platform XML profile",
        )
        .at_path(format!("ops[{op_index}].op")));
    }
    Ok(())
}

pub(crate) fn plan_hidden_v13_apply(
    request: &ApplyRequest,
    binding: &ProviderRootBinding,
    admission: &ApplyAdmission,
) -> Result<(ApplyStagedState, PlannedApplyEffects), ApplyPlanError> {
    let parsed = request
        .ops()
        .iter()
        .enumerate()
        .map(|(op_index, operation)| {
            let Some(family) = crate::application::v13::apply::dispatch_family(operation.name())
            else {
                return Err(hidden_apply_family_unimplemented(op_index));
            };
            let args = serde_json::Value::Object(operation.args().clone());
            let parsed = match family {
                OperationFamily::Metadata | OperationFamily::Properties => {
                    let parsed = metadata::parse_metadata_plan_operation(
                        operation.name(),
                        &args,
                        op_index,
                        binding,
                    )?;
                    ParsedApplyOperation::Metadata(IndexedPlanOperation::new(op_index, parsed))
                }
                OperationFamily::Form
                | OperationFamily::Role
                | OperationFamily::Subsystem
                | OperationFamily::Support => {
                    let parsed = form_resource::parse_form_resource_plan_operation(
                        operation.name(),
                        &args,
                        op_index,
                        binding,
                    )?;
                    ParsedApplyOperation::FormResource(IndexedPlanOperation::new(op_index, parsed))
                }
                OperationFamily::Dcs | OperationFamily::Mxl => {
                    let parsed = dcs_mxl::parse_dcs_mxl_plan_operation(
                        operation.name(),
                        &args,
                        op_index,
                        binding,
                    )?;
                    ParsedApplyOperation::DcsMxl(IndexedPlanOperation::new(op_index, parsed))
                }
                OperationFamily::Code => {
                    let parsed =
                        crate::infrastructure::native_operations::code::parse_code_plan_operation(
                            operation.name(),
                            &args,
                            op_index,
                            binding,
                        )?;
                    ParsedApplyOperation::Code(IndexedPlanOperation::new(op_index, parsed))
                }
                OperationFamily::Xdto => {
                    let parsed =
                        crate::infrastructure::native_operations::xdto::parse_xdto_plan_operation(
                            operation.name(),
                            &args,
                            op_index,
                            binding,
                        )?;
                    ParsedApplyOperation::Xdto(IndexedPlanOperation::new(op_index, parsed))
                }
                OperationFamily::Event => {
                    let args = canonical_event_args(args);
                    let parsed =
                        crate::infrastructure::native_operations::event::parse_event_implement_args(
                            &args,
                            &format!("ops[{op_index}].args"),
                        )
                        .map_err(event_plan_error_to_apply)?;
                    ParsedApplyOperation::Event(IndexedPlanOperation::new(op_index, parsed))
                }
            };
            Ok(parsed)
        })
        .collect::<Result<Vec<_>, ApplyPlanError>>()?;

    let mut staged = admission
        .staged_state()
        .map_err(|error| ApplyPlanError::staging(error, "ops"))?;
    refuse_targets_outside_writable_profile(request, binding, &mut staged)?;
    let mut provisional = Vec::new();
    let mut cursor = 0;
    while cursor < parsed.len() {
        if let ParsedApplyOperation::Unsupported(index) = &parsed[cursor] {
            return Err(hidden_apply_family_unimplemented(*index));
        }
        let end = parsed[cursor..]
            .iter()
            .take_while(|operation| operation.same_family(&parsed[cursor]))
            .count()
            + cursor;
        match &parsed[cursor] {
            ParsedApplyOperation::Code(_) => {
                let operations = parsed[cursor..end]
                    .iter()
                    .map(|operation| match operation {
                        ParsedApplyOperation::Code(operation) => operation.operation().clone(),
                        _ => unreachable!("the selected run is code-only"),
                    })
                    .collect::<Vec<_>>();
                let before = staged
                    .planned_changes()
                    .into_iter()
                    .map(|change| change.relative_path)
                    .collect::<Vec<_>>();
                let authority = admission.code_planning_authority(binding)?;
                let planned = crate::infrastructure::native_operations::code::plan_code_batch(
                    staged,
                    authority,
                    &operations,
                )?;
                staged = planned.0;
                provisional.extend(batch_effects_as_provisional(
                    &before, &staged, planned.1, cursor,
                ));
            }
            ParsedApplyOperation::Event(_) => {
                let operations = parsed[cursor..end]
                    .iter()
                    .map(|operation| match operation {
                        ParsedApplyOperation::Event(operation) => operation.operation().clone(),
                        _ => unreachable!("the selected run is event-only"),
                    })
                    .collect::<Vec<_>>();
                let before = staged
                    .planned_changes()
                    .into_iter()
                    .map(|change| change.relative_path)
                    .collect::<Vec<_>>();
                let authority = admission.metadata_planning_authority(binding)?;
                let planned =
                    crate::infrastructure::native_operations::event::plan_event_implement_batch(
                        staged,
                        authority.source_set_name(),
                        authority.source_kind(),
                        authority.profile(),
                        &operations,
                    )
                    .map_err(event_plan_error_to_apply)?;
                staged = planned.0;
                provisional.extend(batch_effects_as_provisional(
                    &before, &staged, planned.1, cursor,
                ));
            }
            ParsedApplyOperation::Xdto(_) => {
                let operations = parsed[cursor..end]
                    .iter()
                    .map(|operation| match operation {
                        ParsedApplyOperation::Xdto(operation) => operation.operation().clone(),
                        _ => unreachable!("the selected run is XDTO-only"),
                    })
                    .collect::<Vec<_>>();
                let before = staged
                    .planned_changes()
                    .into_iter()
                    .map(|change| change.relative_path)
                    .collect::<Vec<_>>();
                let authority = admission.xdto_planning_authority(binding)?;
                let planned = crate::infrastructure::native_operations::xdto::plan_xdto_batch(
                    staged,
                    authority,
                    &operations,
                )?;
                staged = planned.0;
                provisional.extend(batch_effects_as_provisional(
                    &before, &staged, planned.1, cursor,
                ));
            }
            ParsedApplyOperation::Metadata(_) => {
                let operations = parsed[cursor..end]
                    .iter()
                    .map(|operation| match operation {
                        ParsedApplyOperation::Metadata(operation) => operation.clone(),
                        _ => unreachable!("the selected run is metadata-only"),
                    })
                    .collect::<Vec<_>>();
                let authority = admission.metadata_planning_authority(binding)?;
                let planned = metadata::plan_metadata_batch(staged, authority, &operations)?;
                staged = planned.0;
                provisional.extend(planned.1);
            }
            ParsedApplyOperation::FormResource(_) => {
                let operations = parsed[cursor..end]
                    .iter()
                    .map(|operation| match operation {
                        ParsedApplyOperation::FormResource(operation) => operation.clone(),
                        _ => unreachable!("the selected run is form/resource-only"),
                    })
                    .collect::<Vec<_>>();
                let authority = admission.form_resource_planning_authority(binding)?;
                let planned =
                    form_resource::plan_form_resource_batch(staged, authority, &operations)?;
                staged = planned.0;
                provisional.extend(planned.1);
            }
            ParsedApplyOperation::DcsMxl(_) => {
                let operations = parsed[cursor..end]
                    .iter()
                    .map(|operation| match operation {
                        ParsedApplyOperation::DcsMxl(operation) => operation.clone(),
                        _ => unreachable!("the selected run is DCS/MXL-only"),
                    })
                    .collect::<Vec<_>>();
                let authority = admission.dcs_mxl_planning_authority(binding)?;
                let planned = dcs_mxl::plan_dcs_mxl_batch(staged, authority, &operations)?;
                staged = planned.0;
                provisional.extend(planned.1);
            }
            ParsedApplyOperation::Unsupported(_) => unreachable!("unsupported run handled above"),
        }
        cursor = end;
    }

    let effects = reconcile_effects(&staged, provisional);
    Ok((staged, effects))
}

#[cfg(test)]
mod tests {
    use super::request::{reconcile_effects, ProvisionalApplyEffect};
    use crate::domain::cancellation::CancellationToken;
    use crate::domain::code_intelligence::ProviderDeadline;
    use crate::domain::project_sources::{SourceFormat, SourceProfile, SourceSetKind};
    use crate::domain::workspace::WorkspaceContext;
    use crate::infrastructure::workspace_actor::{
        ApplyAdmission, ProviderRootBinding, WorkspaceActor, WorkspaceIdentity,
        WorkspaceSourceSetInput,
    };
    use std::sync::Arc;
    use std::time::Duration;

    pub(super) struct ApplySeamFixture {
        _root: tempfile::TempDir,
        actor: Arc<WorkspaceActor>,
        pub(super) binding: ProviderRootBinding,
    }

    impl ApplySeamFixture {
        /// The physical `src` root, for tests that lay out extra resources.
        pub(super) fn source_dir(&self) -> std::path::PathBuf {
            std::fs::canonicalize(self._root.path().join("src")).unwrap()
        }

        pub(super) fn new() -> Self {
            let root = tempfile::tempdir().unwrap();
            let source = root.path().join("src");
            std::fs::create_dir_all(source.join("Documents")).unwrap();
            std::fs::write(
                root.path().join("v8project.yaml"),
                "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
            )
            .unwrap();
            std::fs::write(
                source.join("Configuration.xml"),
                r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Configuration><Properties><Name>Main</Name></Properties><ChildObjects><Document>First</Document><Document>Second</Document></ChildObjects></Configuration></MetaDataObject>"#,
            )
            .unwrap();
            for (name, comment) in [("First", ""), ("Second", "")] {
                std::fs::write(
                    source.join(format!("Documents/{name}.xml")),
                    format!(
                        r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" xmlns:v8="http://v8.1c.ru/8.1/data/core" version="2.20"><Document uuid="11111111-1111-4111-8111-111111111111"><Properties><Name>{name}</Name><Synonym/><Comment>{comment}</Comment></Properties><ChildObjects><Attribute uuid="22222222-2222-4222-8222-222222222222"><Properties><Name>Total</Name><Comment>original</Comment></Properties><ChildObjects/></Attribute></ChildObjects></Document></MetaDataObject>"#
                    ),
                )
                .unwrap();
            }
            let workspace_root = std::fs::canonicalize(root.path()).unwrap();
            let source = std::fs::canonicalize(source).unwrap();
            let context = WorkspaceContext {
                cwd: workspace_root.clone(),
                workspace_root: workspace_root.clone(),
                cache_root: workspace_root.join(".build/unica"),
                workspace_epoch: 1,
            };
            let identity = WorkspaceIdentity::new(
                &context,
                [WorkspaceSourceSetInput::new(
                    "main",
                    &source,
                    SourceSetKind::Configuration,
                    SourceFormat::PlatformXml,
                    SourceProfile::platform_xml_8_3_27_format_2_20(),
                )],
                "apply-family-seam-test",
            )
            .unwrap();
            let actor = Arc::new(WorkspaceActor::new(identity, context).unwrap());
            let binding = actor.bind_provider_root("main", &source).unwrap();
            Self {
                _root: root,
                actor,
                binding,
            }
        }

        pub(super) fn admission(&self) -> ApplyAdmission {
            self.actor
                .admit_apply(
                    &self.binding,
                    None,
                    true,
                    ProviderDeadline::from_budget(Duration::from_secs(5)),
                    &CancellationToken::new(),
                )
                .unwrap()
        }
    }

    #[test]
    fn batch_effects_keep_each_event_with_its_own_file() {
        use crate::domain::events::{DomainEvent, DomainEventKind};
        use crate::infrastructure::native_operations::apply::PlannedApplyEffects;

        let fixture = ApplySeamFixture::new();
        let admission = fixture.admission();
        let mut staged = admission.staged_state().unwrap();
        let first = std::path::Path::new("Documents/First.xml");
        let second = std::path::Path::new("Documents/Second.xml");
        let first_preimage = staged.read(first).unwrap().unwrap();
        let second_preimage = staged.read(second).unwrap().unwrap();
        let edited = |preimage: &[u8]| {
            String::from_utf8(preimage.to_vec())
                .unwrap()
                .replace("<Comment></Comment>", "<Comment>batch</Comment>")
                .into_bytes()
        };
        staged
            .replace(first, &first_preimage, edited(&first_preimage))
            .unwrap();
        staged
            .replace(second, &second_preimage, edited(&second_preimage))
            .unwrap();

        // The planner names the file behind each event, as the code, XDTO
        // and event planners do.
        let mut effects = PlannedApplyEffects::default();
        effects.append_at(
            DomainEvent::new(
                DomainEventKind::ModuleChanged,
                "main:Document.First.Module.Object",
            ),
            vec![first.to_path_buf()],
        );
        effects.append_at(
            DomainEvent::new(
                DomainEventKind::ModuleChanged,
                "main:Document.Second.Module.Object",
            ),
            vec![second.to_path_buf()],
        );
        let provisional = super::batch_effects_as_provisional(&[], &staged, effects, 0);

        // A later operation restores the first file; only the second event
        // may survive request-level reconciliation.
        let first_current = staged.read(first).unwrap().unwrap();
        staged
            .replace(first, &first_current, first_preimage.clone())
            .unwrap();
        let reconciled = reconcile_effects(&staged, provisional);
        assert_eq!(
            reconciled
                .events()
                .iter()
                .map(|event| event.artifact.as_str())
                .collect::<Vec<_>>(),
            ["main:Document.Second.Module.Object"]
        );
    }

    #[test]
    fn request_level_reconciliation_drops_cancelled_effect_before_deduplication() {
        let fixture = ApplySeamFixture::new();
        let admission = fixture.admission();
        let mut staged = admission.staged_state().unwrap();
        let second_path = std::path::Path::new("Documents/Second.xml");
        let second_preimage = staged.read(second_path).unwrap().unwrap();
        let second_postimage = String::from_utf8(second_preimage.clone())
            .unwrap()
            .replace("<Comment></Comment>", "<Comment>survives</Comment>")
            .into_bytes();
        staged
            .replace(second_path, &second_preimage, second_postimage)
            .unwrap();

        assert_eq!(
            staged
                .planned_changes()
                .iter()
                .map(|change| change.relative_path.to_string_lossy().into_owned())
                .collect::<Vec<_>>(),
            ["Documents/Second.xml"]
        );
        let effects = reconcile_effects(
            &staged,
            vec![
                ProvisionalApplyEffect::single(
                    "Documents/First.xml",
                    crate::domain::events::DomainEvent::new(
                        crate::domain::events::DomainEventKind::MetadataChanged,
                        "shared",
                    ),
                    0,
                ),
                ProvisionalApplyEffect::single(
                    second_path,
                    crate::domain::events::DomainEvent::new(
                        crate::domain::events::DomainEventKind::SourceSetChanged,
                        "shared",
                    ),
                    1,
                ),
            ],
        );
        assert_eq!(
            effects
                .events()
                .iter()
                .map(|event| (event.kind, event.artifact.as_str()))
                .collect::<Vec<_>>(),
            [(
                crate::domain::events::DomainEventKind::SourceSetChanged,
                "shared"
            )]
        );
    }
}
