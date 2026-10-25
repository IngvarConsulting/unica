//! Canonical metadata descriptor/resource planning over an issued proposal and read observations.
use crate::decode::*;
use crate::edit::apply_typed_operations_to_image_with_seed;
use crate::layout::metadata_layout;
use crate::template_catalog::MetadataTemplateSource;
use serde_json::{json, Map, Value};
use sha2::Digest;
use std::path::{Path, PathBuf};
use unica_address::address::NodeKind;
use unica_address::source_target::MetadataAddress;
use unica_metadata::property_input::DescriptorPropertyValue;
use unica_metadata::{MetaCollection, MetaDiagnosticCode, MetaEditOperation, MetadataKind};
use unica_source_contract::discovery::ProjectDiscoveryRead;
use unica_source_contract::events::{DomainEvent, DomainEventKind};
use unica_source_contract::proposal::{
    empty_apply_family_batch, ApplyPlanError, ApplyPlanErrorKind, ApplyProposal,
    ApplySourceAuthority, IndexedPlanOperation, ProvisionalApplyEffect, StagedFileState,
};
use unica_source_contract::SourceSetKind;
use unica_source_designer::layout::{metadata_descriptor_relative, metadata_kind_by_directory};
use unica_xml_format::text::{LineEnding, LineEndingProfile, SourceTextSnapshot};

/// Source selection remains a trusted, lazy read binding at the original resource phase.
/// The callback borrows the existing template-source view; it cannot issue write authority.
/// Required representation emitter bound by trusted composition. No fallback is provided.
#[derive(Clone, Copy)]
pub struct TemplateSeedEmitter(pub fn() -> String);
type CreationSourceConsumer<'a, G> =
    &'a mut (dyn FnMut(&MetadataTemplateSource<'_, G>) -> Result<(), ApplyPlanError> + 'a);

pub trait MetadataPlanningRead {
    type Guard;
    fn with_creation_source(
        &self,
        op_index: usize,
        consume: CreationSourceConsumer<'_, Self::Guard>,
    ) -> Result<(), ApplyPlanError>;
    fn ordinary_read(&self) -> &dyn ProjectDiscoveryRead;
    fn removal_observations(&self) -> crate::remove::RemoveObservationHooks {
        crate::remove::RemoveObservationHooks::NONE
    }
}
/// Local composition belongs to the final descriptor, after every operation
/// family has contributed its image. A borrowed descriptor contains only the
/// extension's contribution and cannot establish the parent's composition.
pub fn validate_final_object_integrity(
    staged: &mut ApplyProposal<'_>,
    source_set_name: &str,
    source_kind: SourceSetKind,
) -> Result<(), ApplyPlanError> {
    for change in staged.planned_changes() {
        let StagedFileState::Bytes(bytes) = &change.current else {
            continue;
        };
        let relative = &change.relative_path;
        let Some(directory) = relative
            .parent()
            .filter(|parent| parent.components().count() == 1)
            .and_then(Path::to_str)
            .and_then(metadata_kind_by_directory)
        else {
            continue;
        };
        let Ok(kind) = MetadataKind::parse(directory.tag) else {
            continue;
        };
        if kind == MetadataKind::EventSubscription
            && relative
                .extension()
                .and_then(|extension| extension.to_str())
                == Some("xml")
        {
            if let Some(name) = relative.file_stem().and_then(|name| name.to_str()) {
                let at = format!("{source_set_name}:EventSubscription.{name}");
                validate_final_event_subscription(staged, relative, bytes, &at, source_kind)?;
            }
        }
        if unica_metadata::meta_object_integrity_rules(kind)
            .next()
            .is_none()
            || relative
                .extension()
                .and_then(|extension| extension.to_str())
                != Some("xml")
        {
            continue;
        }
        let Some(name) = relative.file_stem().and_then(|name| name.to_str()) else {
            continue;
        };
        let at = format!("{}:{}.{}", source_set_name, kind.as_str(), name);
        staged
            .checkpoint("final metadata object integrity")
            .map_err(|error| ApplyPlanError::staging(error, at.clone()))?;
        if source_kind == SourceSetKind::Extension && final_object_inherits_composition(kind, bytes)
        {
            continue;
        }
        crate::integrity_check::check_meta_object_integrity(kind, bytes).map_err(|diagnostic| {
            ApplyPlanError::new(ApplyPlanErrorKind::BadValue, diagnostic.message).at_path(at)
        })?;
    }
    Ok(())
}

/// The source classes and the event a subscription descriptor binds.
fn event_subscription_binding(
    bytes: &[u8],
) -> Result<(Vec<unica_metadata::MetaEventSource>, String), (&'static str, String)> {
    let (_, document) = unica_xml_format::read::parse_metadata_image(bytes)
        .map_err(|message| ("resources", message))?;
    let source = crate::xml_model::meta_event_subscription_source_node(&document)
        .map_err(|message| ("relations.source", message))?;
    let sources = crate::xml_model::parse_meta_event_subscription_source(source)
        .map_err(|message| ("relations.source", message))?
        .sources;
    let event = source
        .parent_element()
        .and_then(|properties| unica_xml_format::read::child_direct_text(properties, "Event"))
        .unwrap_or_default();
    Ok((sources, event))
}

/// INV.SOURCE.EVENT-DEFINED-TYPE: a changed source or event is checked against
/// the classes it names, with every defined type expanded down to its event
/// classes, nested ones included. A cycle or an unfit member refuses the plan.
/// Defined types are read through the proposal: their descriptors become
/// inputs of the plan, so changing one after planning makes the token stale.
/// The handler contract stays with `check` (#931).
fn validate_final_event_subscription(
    staged: &mut ApplyProposal<'_>,
    relative: &Path,
    bytes: &[u8],
    at: &str,
    source_kind: SourceSetKind,
) -> Result<(), ApplyPlanError> {
    let refuse = |field: &str, message: String| {
        ApplyPlanError::new(ApplyPlanErrorKind::BadValue, format!("{field}: {message}")).at_path(at)
    };
    // A borrowed subscription carries no source of its own: the parent's
    // applies, and the extension only adds to it.
    if final_object_inherits_composition(MetadataKind::EventSubscription, bytes) {
        return Ok(());
    }
    let binding = event_subscription_binding(bytes);
    // Only a changed source or event is checked; an edit of another property
    // keeps whatever the descriptor held, including a source this reader
    // cannot parse.
    if staged
        .original_image(relative)
        .is_some_and(|original| event_subscription_binding(original) == binding)
    {
        return Ok(());
    }
    let (sources, event) = binding.map_err(|(field, message)| refuse(field, message))?;
    if sources.is_empty() || event.is_empty() {
        return Ok(());
    }
    let mut classes = Vec::new();
    let mut visiting = std::collections::HashSet::new();
    for (index, source) in sources.iter().enumerate() {
        expand_staged_event_source(staged, source_kind, source, &mut classes, &mut visiting)
            .map_err(|message| refuse(&format!("relations.source[{index}]"), message))?;
    }
    unica_metadata::validate_event_subscription_sources(&classes, &event).map_err(|error| {
        let (field, message) = crate::validation::event_binding_error_message(error);
        refuse(field, message)
    })?;
    Ok(())
}

fn expand_staged_event_source(
    staged: &mut ApplyProposal<'_>,
    source_kind: SourceSetKind,
    source: &unica_metadata::MetaEventSource,
    classes: &mut Vec<unica_metadata::EventSourceClass>,
    visiting: &mut std::collections::HashSet<MetadataAddress>,
) -> Result<(), String> {
    if let Some(class) = source.event_source_class()? {
        classes.push(class);
        return Ok(());
    }
    let target = source
        .metadata_path()
        .cloned()
        .ok_or_else(|| "event source class is unavailable".to_string())?;
    if !visiting.insert(target.clone()) {
        return Err(format!(
            "DefinedType event source cycle contains `{target}`"
        ));
    }
    let relative = metadata_descriptor_relative(&target, source_kind)?;
    let descriptor = staged
        .read(&relative)
        .map_err(|error| error.to_string())?
        .ok_or_else(|| format!("DefinedType event source `{target}` was not found"))?;
    for member in crate::xml_model::parse_defined_type_event_sources(&descriptor)? {
        expand_staged_event_source(staged, source_kind, &member, classes, visiting)?;
    }
    visiting.remove(&target);
    Ok(())
}

fn final_object_inherits_composition(kind: MetadataKind, bytes: &[u8]) -> bool {
    const MD: &str = "http://v8.1c.ru/8.3/MDClasses";
    let Ok(text) = std::str::from_utf8(bytes) else {
        return false;
    };
    let Ok(document) = roxmltree::Document::parse(text.trim_start_matches('\u{feff}')) else {
        return false;
    };
    let root = document.root_element();
    if !root.has_tag_name((MD, "MetaDataObject")) {
        return false;
    }
    root.children()
        .find(|node| node.has_tag_name((MD, kind.as_str())))
        .and_then(|object| {
            object
                .children()
                .find(|node| node.has_tag_name((MD, "Properties")))
        })
        .and_then(|properties| {
            properties
                .children()
                .find(|node| node.has_tag_name((MD, "ExtendedConfigurationObject")))
        })
        .and_then(|marker| marker.text())
        .is_some_and(|id| uuid::Uuid::parse_str(id.trim()).is_ok())
}

pub fn plan_metadata_batch<R: MetadataPlanningRead>(
    reader: &R,
    template_seed: TemplateSeedEmitter,
    staged: &mut ApplyProposal<'_>,
    authority: &dyn ApplySourceAuthority,
    operations: &[IndexedPlanOperation<MetadataPlanOperation>],
) -> Result<Vec<ProvisionalApplyEffect>, ApplyPlanError> {
    if operations.is_empty() {
        return Err(empty_apply_family_batch());
    }
    if !authority.owns_staged_state(staged) {
        return Err(ApplyPlanError::new(
            ApplyPlanErrorKind::InvalidState,
            "metadata planner authority does not own the staged state",
        )
        .at_path("ops"));
    }
    let mut provisional = Vec::new();
    for operation in operations {
        let op_index = operation.index();
        let (target, edit, addressed) = match &operation.operation().kind {
            MetadataPlanKind::ExchangeContent { owner, items } => {
                stage_exchange_content(
                    staged,
                    authority,
                    owner,
                    items,
                    op_index,
                    &mut provisional,
                )?;
                continue;
            }
            MetadataPlanKind::Edit {
                target,
                operation: edit,
                addressed,
            } => (target, edit, *addressed),
            MetadataPlanKind::Predefined {
                owner,
                kind,
                change,
            } => {
                stage_predefined(
                    staged,
                    authority,
                    owner,
                    *kind,
                    change,
                    op_index,
                    &mut provisional,
                )?;
                continue;
            }
            MetadataPlanKind::Create { kind, name } => {
                stage_object_create(
                    reader,
                    staged,
                    authority.source_kind(),
                    authority.source_set_name(),
                    authority.source_root(),
                    *kind,
                    name,
                    op_index,
                    &mut provisional,
                )?;
                continue;
            }
            MetadataPlanKind::Remove { target, kind, name } => {
                stage_object_remove(
                    reader,
                    staged,
                    authority,
                    target,
                    *kind,
                    name,
                    op_index,
                    &mut provisional,
                )?;
                continue;
            }
            MetadataPlanKind::TemplateAdd { owner, items } => {
                stage_template_add(
                    template_seed,
                    staged,
                    authority,
                    owner,
                    items,
                    op_index,
                    &mut provisional,
                )?;
                continue;
            }
            MetadataPlanKind::TemplateSet {
                owner,
                name,
                values,
            } => {
                stage_template_set(
                    staged,
                    authority,
                    owner,
                    name,
                    values,
                    op_index,
                    &mut provisional,
                )?;
                continue;
            }
            MetadataPlanKind::TemplateRemove { owner, name } => {
                stage_template_remove(staged, authority, owner, name, op_index, &mut provisional)?;
                continue;
            }
            MetadataPlanKind::DescriptorProperties {
                name,
                relative,
                changes,
            } => {
                stage_descriptor_properties(
                    staged,
                    authority.source_kind(),
                    name,
                    relative,
                    changes,
                    op_index,
                    &mut provisional,
                )?;
                continue;
            }
            MetadataPlanKind::HelpCreate { target, name, lang } => {
                stage_help_create(
                    reader,
                    staged,
                    authority,
                    target,
                    HelpPage {
                        name,
                        language: lang,
                    },
                    op_index,
                    &mut provisional,
                )?;
                continue;
            }
        };
        let relative =
            metadata_descriptor_relative(target, authority.source_kind()).map_err(|message| {
                ApplyPlanError::new(ApplyPlanErrorKind::BadValue, message)
                    .at_path(format!("ops[{op_index}].args.at"))
            })?;
        let preimage = staged
            .read(&relative)
            .map_err(|error| ApplyPlanError::staging(error, format!("ops[{op_index}].args.at")))?
            .ok_or_else(|| {
                ApplyPlanError::new(
                    ApplyPlanErrorKind::NotFound,
                    "metadata descriptor was not found",
                )
                .at_path(format!("ops[{op_index}].args.at"))
            })?;
        // The typed image transform addresses byte offsets of the parsed
        // document, so the byte-order mark stays outside the text it edits
        // and is restored on the way out.
        let (bom, body) = match preimage.strip_prefix(b"\xef\xbb\xbf") {
            Some(body) => (&b"\xef\xbb\xbf"[..], body),
            None => (&b""[..], preimage.as_slice()),
        };
        let source = std::str::from_utf8(body).map_err(|_| {
            ApplyPlanError::new(
                ApplyPlanErrorKind::InvalidSource,
                "metadata descriptor is not UTF-8",
            )
            .at_path(format!("ops[{op_index}].args.at"))
        })?;
        let mut postimage = source.to_string();
        let (actual_kind, actual_name) = crate::edit::meta_edit_object_identity(&postimage)
            .map_err(|message| {
                ApplyPlanError::new(ApplyPlanErrorKind::InvalidSource, message)
                    .at_path(format!("ops[{op_index}].args.at"))
            })?;
        let expected = target.as_str().split('.').collect::<Vec<_>>();
        if expected.as_slice() != [actual_kind.as_str(), actual_name.as_str()] {
            return Err(ApplyPlanError::new(
                ApplyPlanErrorKind::InvalidSource,
                "metadata descriptor identity does not match its logical target",
            )
            .at_path(format!("ops[{op_index}].args.at")));
        }
        let uuid_seed = format!(
            "{}\0{}\0{}\0{:?}\0{:x}",
            authority.source_set_name(),
            target.as_str(),
            op_index,
            edit,
            sha2::Sha256::digest(&preimage)
        );
        let lang =
            if crate::edit::typed_operations_write_multilingual_text(std::slice::from_ref(edit)) {
                unica_source_designer::language::staged_text_language(
                    staged,
                    authority.source_kind(),
                    op_index,
                )?
            } else {
                String::new()
            };
        apply_typed_operations_to_image_with_seed(
            &mut postimage,
            std::slice::from_ref(edit),
            &lang,
            uuid_seed.as_bytes(),
        )
        .map_err(|failure| {
            let diagnostic = failure
                .diagnostics
                .into_iter()
                .next()
                .expect("typed metadata failures contain a diagnostic");
            let kind = match diagnostic.code {
                MetaDiagnosticCode::InvalidArguments
                | MetaDiagnosticCode::UnsupportedKind
                | MetaDiagnosticCode::CapabilityUnavailable => ApplyPlanErrorKind::BadValue,
                MetaDiagnosticCode::TargetNotFound => ApplyPlanErrorKind::NotFound,
                MetaDiagnosticCode::ValidationFailed => ApplyPlanErrorKind::Postcondition,
                MetaDiagnosticCode::AlreadyExists
                | MetaDiagnosticCode::SupportLocked
                | MetaDiagnosticCode::ValidationWarning
                | MetaDiagnosticCode::RedundantListPresentation
                | MetaDiagnosticCode::CommandTextRecommendedLimit
                | MetaDiagnosticCode::CommandTextUpperLimit
                | MetaDiagnosticCode::ConcurrentModification
                | MetaDiagnosticCode::ProviderUnavailable
                | MetaDiagnosticCode::RollbackFailed => ApplyPlanErrorKind::ProviderUnavailable,
            };
            // The member named by the address is the operation's own node:
            // when the writer cannot find it, the refusal points at `at`. Other
            // fields map back to where the caller wrote them.
            let path = match (addressed, diagnostic.field) {
                (Some(fields), field) => {
                    let field = field.map(|field| {
                        field
                            .strip_prefix("operations[0].")
                            .map(str::to_string)
                            .unwrap_or(field)
                    });
                    match field.as_deref().map(|field| fields.caller_field(field)) {
                        None | Some(None) if kind == ApplyPlanErrorKind::NotFound => {
                            format!("ops[{op_index}].args.at")
                        }
                        Some(Some(field)) => format!("ops[{op_index}].args.{field}"),
                        _ => format!("ops[{op_index}].args"),
                    }
                }
                (None, None) => format!("ops[{op_index}].args"),
                (None, Some(field)) => format!("ops[{op_index}].args.{field}"),
            };
            ApplyPlanError::new(kind, diagnostic.message).at_path(path)
        })?;
        stage_command_module_lifecycle(
            staged,
            authority,
            target,
            &relative,
            source,
            edit,
            op_index,
            &mut provisional,
        )?;
        // The typed writer inserts `\n`-separated lines. A descriptor with one
        // observed line ending keeps it (a platform export writes CRLF); a
        // mixed or line-free one is left as written, never normalized
        // (INV.SOURCE.OBSERVED-BYTES).
        let postimage = match SourceTextSnapshot::from_bytes(&preimage)
            .map(|snapshot| snapshot.line_endings())
        {
            Ok(LineEndingProfile::Uniform(LineEnding::CrLf)) => {
                postimage.replace("\r\n", "\n").replace('\n', "\r\n")
            }
            Ok(LineEndingProfile::Uniform(LineEnding::Cr)) => postimage.replace('\n', "\r"),
            _ => postimage,
        };
        let mut postimage = {
            let mut bytes = Vec::with_capacity(bom.len() + postimage.len());
            bytes.extend_from_slice(bom);
            bytes.extend_from_slice(postimage.as_bytes());
            bytes
        };
        postimage.shrink_to_fit();
        if postimage != preimage {
            staged
                .replace(&relative, &preimage, postimage)
                .map_err(|error| {
                    ApplyPlanError::staging(error, format!("ops[{op_index}].args.at"))
                })?;
            provisional.push(ProvisionalApplyEffect::single(
                relative,
                DomainEvent::new(
                    DomainEventKind::MetadataChanged,
                    target.as_str().to_string(),
                ),
                op_index,
            ));
        }
    }
    Ok(provisional)
}

#[allow(clippy::too_many_arguments)]
fn stage_command_module_lifecycle(
    staged: &mut ApplyProposal<'_>,
    authority: &dyn ApplySourceAuthority,
    owner: &MetadataAddress,
    descriptor_relative: &Path,
    owner_preimage: &str,
    edit: &MetaEditOperation,
    op_index: usize,
    provisional: &mut Vec<ProvisionalApplyEffect>,
) -> Result<(), ApplyPlanError> {
    let canonical_name = |name: &str| {
        crate::edit::canonical_typed_command_name(owner_preimage, name)
            .map_err(|failure| meta_failure_to_plan_error(failure, op_index))
    };
    let changes = match edit {
        MetaEditOperation::Update {
            collection: MetaCollection::Commands,
            elements,
            ..
        } => elements
            .iter()
            .filter_map(|element| {
                element.new_name.as_ref().map(|new_name| {
                    canonical_name(&element.name).map(|old_name| (old_name, Some(new_name.clone())))
                })
            })
            .collect::<Result<Vec<_>, _>>()?,
        MetaEditOperation::Remove {
            collection: MetaCollection::Commands,
            names,
            ..
        } => names
            .iter()
            .map(|name| canonical_name(name).map(|name| (name, None)))
            .collect::<Result<Vec<_>, _>>()?,
        _ => return Ok(()),
    };
    let module_event = |name: &str| {
        DomainEvent::new(
            DomainEventKind::ModuleChanged,
            format!(
                "{}:{}.Command.{name}.Module.Command",
                authority.source_set_name(),
                owner.as_str()
            ),
        )
    };
    for (old_name, new_name) in changes {
        if new_name.as_deref() == Some(old_name.as_str()) {
            continue;
        }
        let at_path = format!("ops[{op_index}].args.at");
        let old_relative = crate::edit::typed_command_module_path(descriptor_relative, &old_name);
        let module = staged
            .read(&old_relative)
            .map_err(|error| ApplyPlanError::staging(error, &at_path))?;
        if let Some(bytes) = &module {
            let effect_path = staged
                .retained_relative_path(&old_relative)
                .map_err(|error| ApplyPlanError::staging(error, &at_path))?;
            staged
                .remove(&old_relative, bytes)
                .map_err(|error| ApplyPlanError::staging(error, &at_path))?;
            provisional.push(ProvisionalApplyEffect::single(
                effect_path,
                module_event(&old_name),
                op_index,
            ));
        }
        if let Some(new_name) = new_name {
            let new_relative =
                crate::edit::typed_command_module_path(descriptor_relative, &new_name);
            let new_name_path = format!("ops[{op_index}].args.values.newName");
            // Probe after the staged removal: aliases under the retained
            // filesystem's name policy share that absent overlay entry.
            // A distinct orphan module must still refuse even when this
            // command had no module to move.
            if staged
                .read(&new_relative)
                .map_err(|error| ApplyPlanError::staging(error, &new_name_path))?
                .is_some()
            {
                return Err(ApplyPlanError::new(
                    ApplyPlanErrorKind::InvalidState,
                    format!(
                        "the command module destination already exists: {}",
                        new_relative.to_string_lossy().replace('\\', "/")
                    ),
                )
                .at_path(new_name_path));
            }
            if let Some(bytes) = module {
                staged
                    .create(&new_relative, bytes)
                    .map_err(|error| ApplyPlanError::staging(error, &new_name_path))?;
                staged
                    .require_exact_path_spelling(&new_relative)
                    .map_err(|error| ApplyPlanError::staging(error, &new_name_path))?;
                let effect_path = staged
                    .retained_relative_path(&new_relative)
                    .map_err(|error| ApplyPlanError::staging(error, new_name_path))?;
                provisional.push(ProvisionalApplyEffect::single(
                    effect_path,
                    module_event(&new_name),
                    op_index,
                ));
                if staged
                    .read(&old_relative)
                    .map_err(|error| ApplyPlanError::staging(error, &at_path))?
                    .is_some()
                {
                    // A case alias can change the logical module address
                    // without moving bytes on this filesystem. The owner's
                    // final delta still invalidates the BSL index.
                    provisional.push(ProvisionalApplyEffect::single(
                        descriptor_relative,
                        module_event(&new_name),
                        op_index,
                    ));
                }
            }
        }
    }
    Ok(())
}

pub fn meta_failure_to_plan_error(
    failure: unica_metadata::request::MetaFailure,
    op_index: usize,
) -> ApplyPlanError {
    let diagnostic = failure
        .diagnostics
        .into_iter()
        .next()
        .expect("typed metadata failures contain a diagnostic");
    let kind = match diagnostic.code {
        MetaDiagnosticCode::InvalidArguments
        | MetaDiagnosticCode::UnsupportedKind
        | MetaDiagnosticCode::CapabilityUnavailable => ApplyPlanErrorKind::BadValue,
        MetaDiagnosticCode::TargetNotFound => ApplyPlanErrorKind::NotFound,
        MetaDiagnosticCode::ValidationFailed => ApplyPlanErrorKind::Postcondition,
        MetaDiagnosticCode::AlreadyExists | MetaDiagnosticCode::SupportLocked => {
            ApplyPlanErrorKind::InvalidState
        }
        MetaDiagnosticCode::ValidationWarning
        | MetaDiagnosticCode::RedundantListPresentation
        | MetaDiagnosticCode::CommandTextRecommendedLimit
        | MetaDiagnosticCode::CommandTextUpperLimit
        | MetaDiagnosticCode::ConcurrentModification
        | MetaDiagnosticCode::ProviderUnavailable
        | MetaDiagnosticCode::RollbackFailed => ApplyPlanErrorKind::ProviderUnavailable,
    };
    let path = diagnostic.field.map_or_else(
        || format!("ops[{op_index}].args"),
        |field| format!("ops[{op_index}].args.values.{field}"),
    );
    ApplyPlanError::new(kind, diagnostic.message).at_path(path)
}

pub fn staged_relative(
    root: &std::path::Path,
    absolute: &std::path::Path,
    op_index: usize,
) -> Result<PathBuf, ApplyPlanError> {
    absolute
        .strip_prefix(root)
        .map(std::path::Path::to_path_buf)
        .map_err(|_| {
            ApplyPlanError::new(
                ApplyPlanErrorKind::ProviderUnavailable,
                "planned metadata file lies outside the admitted source root",
            )
            .at_path(format!("ops[{op_index}].args.at"))
        })
}

/// Stages a new top-level object from the platform template catalog: its
/// descriptor, modules and auxiliary files plus the owner registration.
/// Shared by the metadata family (`object.create`) and the families whose
/// creation operations are the same template (`role.create`,
/// `subsystem.create`).
#[allow(clippy::too_many_arguments)]
pub fn stage_object_create<R: MetadataPlanningRead>(
    reader: &R,
    staged: &mut ApplyProposal<'_>,
    source_kind: SourceSetKind,
    source_set_name: &str,
    root: &std::path::Path,
    kind: MetadataKind,
    name: &str,
    op_index: usize,
    provisional: &mut Vec<ProvisionalApplyEffect>,
) -> Result<(), ApplyPlanError> {
    reader.with_creation_source(op_index, &mut |source| {
        // The new descriptor's synonym is written in the source set's default
        // language, read from the staged image like every other edit (#909).
        let language =
            unica_source_designer::language::staged_text_language(staged, source_kind, op_index)?;
        let post_image = crate::template_catalog::minimal_object(
            source,
            kind,
            name,
            crate::template_catalog::MetadataTemplateOperationOverrides {
                source: false,
                handler: false,
            },
            source_set_name,
            &language,
        )
        .map_err(|failure| meta_failure_to_plan_error(failure, op_index))?;
        let owner_relative = staged_relative(root, source.owner_path, op_index)?;
        let descriptor_relative =
            PathBuf::from(metadata_layout(kind).directory).join(format!("{name}.xml"));
        let existing = staged.read(&descriptor_relative).map_err(|error| {
            ApplyPlanError::staging(error, format!("ops[{op_index}].args.values.name"))
        })?;
        if existing.is_some() {
            return Err(ApplyPlanError::new(
                ApplyPlanErrorKind::InvalidState,
                format!("metadata object `{}.{name}` already exists", kind.as_str()),
            )
            .at_path(format!("ops[{op_index}].args.values.name")));
        }
        let mut touched = Vec::new();
        for file in &post_image.files {
            let relative = file.relative_path.clone();
            let at_path = format!("ops[{op_index}].args.values.name");
            match file.mode {
                crate::template_catalog::MetadataTemplateFileMode::Create => {
                    staged
                        .create(&relative, file.bytes.clone())
                        .map_err(|error| ApplyPlanError::staging(error, at_path))?;
                    touched.push(relative);
                }
                crate::template_catalog::MetadataTemplateFileMode::Guard => {
                    let current = staged
                        .read(&relative)
                        .map_err(|error| ApplyPlanError::staging(error, at_path.clone()))?;
                    let expected = file.preimage.as_deref().unwrap_or(&file.bytes);
                    // An earlier operation of this batch may have staged its own
                    // change of the guarded prerequisite (a register adds itself
                    // to the registrar a journal then guards). The guard binds the
                    // on-disk image the template read, so it holds while that
                    // image is still the one this plan retained (#1332).
                    let unchanged_on_disk =
                        current.is_some() && staged.original_image(&relative) == Some(expected);
                    if current.as_deref() != Some(expected) && !unchanged_on_disk {
                        return Err(ApplyPlanError::new(
                            ApplyPlanErrorKind::InvalidState,
                            format!(
                                "prerequisite `{}` changed while planning the new object",
                                relative.display()
                            ),
                        )
                        .at_path(at_path));
                    }
                }
                crate::template_catalog::MetadataTemplateFileMode::Replace => {
                    let preimage = file.preimage.as_deref().unwrap_or(&file.bytes);
                    let current = staged
                        .read(&relative)
                        .map_err(|error| ApplyPlanError::staging(error, at_path.clone()))?;
                    // The template derived its replacement from the file on disk.
                    // An earlier operation of this batch may already have staged
                    // its own change of the same prerequisite (#1332): two new
                    // registers share one registrar document. Re-derive the edit
                    // from that staged image, but only while the on-disk image the
                    // template read is still the one this plan retained.
                    let replacement = match (current.as_deref(), &file.edit) {
                        (Some(current), _) if current == preimage => file.bytes.clone(),
                        (Some(current), Some(edit))
                            if staged.original_image(&relative) == Some(preimage) =>
                        {
                            edit.apply(current).map_err(|_| {
                                ApplyPlanError::new(
                                    ApplyPlanErrorKind::InvalidState,
                                    format!(
                                    "prerequisite `{}` cannot accept the change of the new object",
                                    relative.display()
                                ),
                                )
                                .at_path(at_path.clone())
                            })?
                        }
                        _ => {
                            return Err(ApplyPlanError::new(
                                ApplyPlanErrorKind::InvalidState,
                                format!(
                                    "prerequisite `{}` changed while planning the new object",
                                    relative.display()
                                ),
                            )
                            .at_path(at_path));
                        }
                    };
                    let current = current.expect("a matched prerequisite image is present");
                    staged
                        .replace(&relative, &current, replacement)
                        .map_err(|error| ApplyPlanError::staging(error, at_path))?;
                    touched.push(relative);
                }
            }
        }
        // 8.3.27 lists a registrar's records in the order of the registers'
        // UUIDs; the new record moves to its place once every file of the new
        // register is staged and its UUID can be read.
        for file in &post_image.files {
            let Some(crate::template_catalog::MetadataDependencyEdit::AddRegisterRecord(register)) =
                &file.edit
            else {
                continue;
            };
            let relative = file.relative_path.clone();
            let Some(current) = staged.read(&relative).map_err(|error| {
                ApplyPlanError::staging(error, format!("ops[{op_index}].args.at"))
            })?
            else {
                continue;
            };
            let mut uuid_of = |reference: &str| -> Option<String> {
                let (kind, name) = reference.split_once('.')?;
                let layout = unica_source_designer::layout::metadata_kind(kind)?;
                let descriptor = PathBuf::from(layout.directory).join(format!("{name}.xml"));
                let bytes = staged.read(&descriptor).ok()??;
                crate::template_catalog::descriptor_object_uuid(&bytes)
            };
            if let Some(ordered) =
                crate::template_catalog::place_register_record(&current, register, &mut uuid_of)
            {
                staged
                    .replace(&relative, &current, ordered)
                    .map_err(|error| {
                        ApplyPlanError::staging(error, format!("ops[{op_index}].args.at"))
                    })?;
            }
        }
        let owner_preimage = staged
            .read(&owner_relative)
            .map_err(|error| ApplyPlanError::staging(error, format!("ops[{op_index}].args.at")))?
            .ok_or_else(|| {
                ApplyPlanError::new(
                    ApplyPlanErrorKind::NotFound,
                    "the configuration descriptor was not found",
                )
                .at_path(format!("ops[{op_index}].args.at"))
            })?;
        let Some(owner_postimage) = unica_source_designer::registration::owner_registration_image(
            &owner_preimage,
            kind.as_str(),
            name,
            true,
            op_index,
        )?
        else {
            return Err(ApplyPlanError::new(
                ApplyPlanErrorKind::InvalidState,
                format!(
                    "the configuration already registers `{}.{name}`",
                    kind.as_str()
                ),
            )
            .at_path(format!("ops[{op_index}].args.values.name")));
        };
        staged
            .replace(&owner_relative, &owner_preimage, owner_postimage)
            .map_err(|error| ApplyPlanError::staging(error, format!("ops[{op_index}].args.at")))?;
        touched.push(owner_relative);
        provisional.push(ProvisionalApplyEffect::spanning(
            touched,
            DomainEvent::new(
                DomainEventKind::MetadataChanged,
                post_image.metadata_path.as_str().to_string(),
            ),
            op_index,
        ));
        Ok(())
    })
}

#[allow(clippy::too_many_arguments)]
fn stage_object_remove<R: MetadataPlanningRead>(
    reader: &R,
    staged: &mut ApplyProposal<'_>,
    authority: &dyn ApplySourceAuthority,
    target: &MetadataAddress,
    kind: MetadataKind,
    name: &str,
    op_index: usize,
    provisional: &mut Vec<ProvisionalApplyEffect>,
) -> Result<(), ApplyPlanError> {
    use crate::remove::{
        meta_remove_composition, meta_remove_composition_image, meta_remove_references,
        meta_remove_referrer, meta_remove_search_patterns, meta_remove_should_skip_file,
        plan_meta_remove_subsystem_replacements, remove_composition_entries, MetaRemoveReference,
        META_REMOVE_REFERENCE_FILE_MAX_BYTES,
    };
    let at_path = format!("ops[{op_index}].args.at");
    unica_source_designer::registration::require_untouched_staged_state(
        staged,
        "object.remove",
        &at_path,
    )?;
    let root = authority.source_root();
    let layout = metadata_layout(kind);
    let descriptor_relative = PathBuf::from(layout.directory).join(format!("{name}.xml"));
    let descriptor_preimage = staged
        .read(&descriptor_relative)
        .map_err(|error| ApplyPlanError::staging(error, at_path.clone()))?
        .ok_or_else(|| {
            ApplyPlanError::new(
                ApplyPlanErrorKind::NotFound,
                "metadata descriptor was not found",
            )
            .at_path(at_path.clone())
        })?;
    let descriptor_absolute = root.join(&descriptor_relative);
    let object_dir = root.join(layout.directory).join(name);
    let files = staged
        .enumerate_tree(Path::new(""))
        .map_err(|error| ApplyPlanError::staging(error, at_path.clone()))?;
    let code_patterns = meta_remove_search_patterns(kind.as_str(), name, layout.directory);
    // The bare `X.` of a common module is a call in code; in XML it would
    // match a synonym such as «X.Печать» rather than the module.
    let bare = format!("{name}.");
    let mut metadata_patterns = code_patterns
        .iter()
        .filter(|pattern| **pattern != bare)
        .cloned()
        .collect::<Vec<_>>();
    metadata_patterns.push(format!("{}.{name}", kind.as_str()));
    // A query names a table by the kind in either language: `Catalog.X`,
    // `Справочник.X`.
    let spellings = unica_address::source_target::metadata_address_kind_spellings(kind.as_str())
        .unwrap_or_default();
    let mut query_patterns = metadata_patterns.clone();
    query_patterns.extend(
        spellings
            .iter()
            .map(|spelling| format!("{spelling}.{name}")),
    );
    // Module code names the object the same way and also by its Russian
    // type names, the kind spelling followed by `Ссылка` or `Объект`.
    let mut code_patterns = code_patterns;
    code_patterns.extend(query_patterns.iter().cloned());
    for reference in crate::remove::meta_remove_type_ref_names(kind.as_str()).unwrap_or_default() {
        let suffix = match reference.strip_prefix(kind.as_str()) {
            Some("Ref") => "Ссылка",
            Some("Object") => "Объект",
            _ => continue,
        };
        code_patterns.extend(
            spellings
                .iter()
                .skip(1)
                .map(|spelling| format!("{spelling}{suffix}.{name}")),
        );
    }
    code_patterns.sort();
    code_patterns.dedup();
    let source_set = authority.source_set_name();
    let removed = format!("{source_set}:{}", target.as_str());
    let qualified = format!("{}.{name}", kind.as_str());
    let mut metadata_references = Vec::new();
    let mut code_references = Vec::new();
    // Compositions that list the object — role rights, command interfaces,
    // exchange plan and functional option content — lose its entries with it.
    let mut cleaned = Vec::new();
    let mut cleaned_files = Vec::new();
    for relative in &files {
        let extension = relative
            .extension()
            .and_then(|ext| ext.to_str())
            .unwrap_or("");
        let patterns = if extension.eq_ignore_ascii_case("xml") {
            &metadata_patterns
        } else if extension.eq_ignore_ascii_case("bsl") {
            &code_patterns
        } else {
            continue;
        };
        let absolute = root.join(relative);
        let skipped = meta_remove_should_skip_file(
            &absolute,
            root,
            &descriptor_absolute,
            &object_dir,
            true,
            true,
        );
        let composition =
            meta_remove_composition(relative).filter(|_| !absolute.starts_with(&object_dir));
        if skipped && composition.is_none() {
            continue;
        }
        let bytes = staged
            .read_guarded_bounded(relative, META_REMOVE_REFERENCE_FILE_MAX_BYTES)
            .map_err(|error| ApplyPlanError::staging(error, at_path.clone()))?
            .ok_or_else(|| {
                ApplyPlanError::new(
                    ApplyPlanErrorKind::InvalidState,
                    "reference input disappeared",
                )
            })?;
        let text = std::str::from_utf8(&bytes).map_err(|error| {
            ApplyPlanError::new(
                ApplyPlanErrorKind::InvalidSource,
                format!("reference input is not UTF-8: {error}"),
            )
        })?;
        let mut text = text.trim_start_matches('\u{feff}').to_string();
        if let Some((rules, property)) = composition {
            let (updated, removed_entries) = remove_composition_entries(&text, rules, &qualified)
                .map_err(|error| {
                ApplyPlanError::new(ApplyPlanErrorKind::InvalidSource, error)
                    .at_path(at_path.clone())
            })?;
            if removed_entries > 0 {
                let (referrer, _) = meta_remove_referrer(relative);
                cleaned.push((format!("{source_set}:{referrer}"), property));
                cleaned_files.push((
                    relative.clone(),
                    bytes.clone(),
                    meta_remove_composition_image(&bytes, updated.clone()),
                ));
                text = updated;
            }
        }
        if skipped {
            continue;
        }
        for reference in
            meta_remove_references(source_set, relative, &text, patterns, &query_patterns)
        {
            match reference {
                MetaRemoveReference::Metadata { at, property } => {
                    metadata_references.push(format!("`{at}` → {property}"));
                }
                MetaRemoveReference::Code { at, line } => code_references.push((at, line)),
            }
        }
    }
    if !metadata_references.is_empty() {
        let mut shown = metadata_references
            .iter()
            .take(10)
            .cloned()
            .collect::<Vec<_>>()
            .join(", ");
        if metadata_references.len() > 10 {
            shown.push_str(&format!(" and {} more", metadata_references.len() - 10));
        }
        return Err(ApplyPlanError::new(
            ApplyPlanErrorKind::InvalidState,
            format!(
                "`{removed}` is referenced by other metadata: {shown}; remove these references first"
            ),
        )
        .at_path(at_path));
    }
    // References only in module code or query-language text do not block
    // the removal: the plan names them, and executing its token confirms it.
    let code_warning = (!code_references.is_empty()).then(|| {
        let places = code_references
            .iter()
            .take(20)
            .map(|(at, line)| serde_json::json!({"at": at, "line": line}))
            .collect::<Vec<_>>();
        let mut shown = code_references
            .iter()
            .take(5)
            .map(|(at, line)| format!("`{at}` line {line}"))
            .collect::<Vec<_>>()
            .join(", ");
        if code_references.len() > 5 {
            shown.push_str(&format!(" and {} more", code_references.len() - 5));
        }
        serde_json::json!({
            "code": "code_references",
            "at": at_path,
            "count": code_references.len(),
            "references": places,
            "message": format!(
                "module code or query text still refers to `{removed}`: {shown}; the removal leaves this text as it is"
            ),
        })
    });
    let mut touched = Vec::new();
    for relative in files
        .iter()
        .filter(|path| root.join(path).starts_with(&object_dir))
    {
        let preimage = staged
            .read(relative)
            .map_err(|error| ApplyPlanError::staging(error, at_path.clone()))?
            .ok_or_else(|| {
                ApplyPlanError::new(
                    ApplyPlanErrorKind::InvalidState,
                    "payload input disappeared",
                )
            })?;
        staged
            .remove(relative, &preimage)
            .map_err(|error| ApplyPlanError::staging(error, at_path.clone()))?;
        touched.push(relative.clone());
    }
    let subsystems_dir = root.join("Subsystems");
    if unica_source_designer::read::path_is_dir(reader.ordinary_read(), &subsystems_dir) {
        let mut replacements = Vec::new();
        let mut reads = Vec::new();
        plan_meta_remove_subsystem_replacements(
            reader.ordinary_read(),
            reader.removal_observations(),
            &subsystems_dir,
            target.as_str(),
            &mut replacements,
            &mut reads,
        )
        .map_err(|error| {
            ApplyPlanError::new(ApplyPlanErrorKind::ProviderUnavailable, error)
                .at_path(at_path.clone())
        })?;
        for read in reads {
            let relative = staged_relative(root, &read.path, op_index)?;
            let observed = staged
                .read(&relative)
                .map_err(|error| ApplyPlanError::staging(error, at_path.clone()))?;
            if observed.as_deref() != Some(read.raw.as_slice()) {
                return Err(ApplyPlanError::new(
                    ApplyPlanErrorKind::InvalidState,
                    "subsystem input changed while planning",
                ));
            }
        }
        for replacement in replacements {
            let relative = staged_relative(root, &replacement.path, op_index)?;
            let (referrer, _) = meta_remove_referrer(&relative);
            cleaned.push((format!("{source_set}:{referrer}"), "Content"));
            staged
                .replace(&relative, &replacement.original, replacement.replacement)
                .map_err(|error| ApplyPlanError::staging(error, at_path.clone()))?;
            touched.push(relative);
        }
    }
    for (relative, original, replacement) in cleaned_files {
        staged
            .replace(&relative, &original, replacement)
            .map_err(|error| ApplyPlanError::staging(error, at_path.clone()))?;
        touched.push(relative);
    }
    let owner_relative = PathBuf::from("Configuration.xml");
    let owner_preimage = staged
        .read(&owner_relative)
        .map_err(|error| ApplyPlanError::staging(error, at_path.clone()))?
        .ok_or_else(|| {
            ApplyPlanError::new(
                ApplyPlanErrorKind::NotFound,
                "the configuration descriptor was not found",
            )
            .at_path(at_path.clone())
        })?;
    let Some(owner_postimage) = unica_source_designer::registration::owner_registration_image(
        &owner_preimage,
        kind.as_str(),
        name,
        false,
        op_index,
    )?
    else {
        return Err(ApplyPlanError::new(
            ApplyPlanErrorKind::InvalidSource,
            "metadata object is not registered by its owner",
        )
        .at_path(at_path));
    };
    staged
        .replace(&owner_relative, &owner_preimage, owner_postimage)
        .map_err(|error| ApplyPlanError::staging(error, at_path.clone()))?;
    touched.push(owner_relative);
    staged
        .remove(&descriptor_relative, &descriptor_preimage)
        .map_err(|error| ApplyPlanError::staging(error, at_path))?;
    staged
        .remove_owned_directories(
            &PathBuf::from(layout.directory).join(name),
            &descriptor_relative,
            Path::new(layout.directory),
        )
        .map_err(|error| ApplyPlanError::staging(error, format!("ops[{op_index}].args.at")))?;
    touched.push(descriptor_relative);
    let effect = ProvisionalApplyEffect::spanning(
        touched,
        DomainEvent::new(
            DomainEventKind::MetadataChanged,
            target.as_str().to_string(),
        ),
        op_index,
    );
    let mut effect = match code_warning {
        Some(warning) => effect.with_warning(warning),
        None => effect,
    };
    if !cleaned.is_empty() {
        let mut shown = cleaned
            .iter()
            .take(10)
            .map(|(at, property)| format!("`{at}` → {property}"))
            .collect::<Vec<_>>()
            .join(", ");
        if cleaned.len() > 10 {
            shown.push_str(&format!(" and {} more", cleaned.len() - 10));
        }
        effect = effect.with_warning(serde_json::json!({
            "code": "composition_cleaned",
            "at": format!("ops[{op_index}].args.at"),
            "count": cleaned.len(),
            "entries": cleaned
                .iter()
                .take(20)
                .map(|(at, property)| serde_json::json!({"at": at, "property": property}))
                .collect::<Vec<_>>(),
            "message": format!("the removal also takes `{removed}` out of {shown}"),
        }));
    }
    provisional.push(effect);
    Ok(())
}

/// Sets the `lang` item of a multilingual property of the descriptor's own
/// `<Properties>`, keeping the other languages byte for byte; an empty
/// element becomes a one-item block. `None` when the descriptor is not XML
/// or has no such property.
pub fn set_ml_property(
    text: &str,
    tag: &str,
    indent: &str,
    lang: &str,
    value: &str,
) -> Option<String> {
    edit_descriptor_property(text, tag, |source, property| {
        unica_xml_format::localized::replace_language_value(source, property, indent, lang, value)
    })
}

/// Sets a plain-text property of the descriptor's own `<Properties>`,
/// spelled with the element name the source uses.
fn set_plain_property(text: &str, tag: &str, value: &str) -> Option<String> {
    edit_descriptor_property(text, tag, |source, property| {
        let open = &source[property.range().start + 1..];
        let name = &open[..open
            .find(|ch: char| ch.is_whitespace() || ch == '>' || ch == '/')
            .unwrap_or(open.len())];
        if value.is_empty() {
            format!("<{name}/>")
        } else {
            format!(
                "<{name}>{}</{name}>",
                value
                    .replace('&', "&amp;")
                    .replace('<', "&lt;")
                    .replace('>', "&gt;")
            )
        }
    })
}

/// Rewrites the property `tag` of the first `<Properties>` of the document,
/// which is the descriptor object's own; `replace` builds the new element
/// from the source text and the property node.
fn edit_descriptor_property(
    text: &str,
    tag: &str,
    replace: impl FnOnce(&str, roxmltree::Node<'_, '_>) -> String,
) -> Option<String> {
    let document = roxmltree::Document::parse(text).ok()?;
    let property = document
        .descendants()
        .find(|node| node.is_element() && node.tag_name().name() == "Properties")?
        .children()
        .find(|node| node.is_element() && node.tag_name().name() == tag)?;
    let replacement = replace(text, property);
    let mut updated = text.to_string();
    updated.replace_range(property.range(), &replacement);
    Some(updated)
}

/// Replaces the first `<Tag>…</Tag>` or `<Tag/>` inside `<Properties>`.
fn replace_property_block(text: &str, tag: &str, replacement: &str) -> Option<String> {
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    let empty = format!("<{tag}/>");
    let properties_start = text.find("<Properties>")?;
    let properties_end = text[properties_start..].find("</Properties>")? + properties_start;
    let scope = &text[properties_start..properties_end];
    let (start, end) = if let Some(index) = scope.find(&empty) {
        (index, index + empty.len())
    } else {
        let index = scope.find(&open)?;
        let close_index = scope[index..].find(&close)? + index + close.len();
        (index, close_index)
    };
    let mut updated = text.to_string();
    updated.replace_range(
        properties_start + start..properties_start + end,
        replacement.trim_start(),
    );
    Some(updated)
}

#[allow(clippy::too_many_arguments)]
fn stage_descriptor_properties(
    staged: &mut ApplyProposal<'_>,
    source_kind: SourceSetKind,
    name: &str,
    relative: &std::path::Path,
    changes: &DescriptorPropertyChanges,
    op_index: usize,
    provisional: &mut Vec<ProvisionalApplyEffect>,
) -> Result<(), ApplyPlanError> {
    let at_path = format!("ops[{op_index}].args.at");
    let preimage = staged
        .read(relative)
        .map_err(|error| ApplyPlanError::staging(error, at_path.clone()))?
        .ok_or_else(|| {
            ApplyPlanError::new(
                ApplyPlanErrorKind::NotFound,
                "metadata descriptor was not found",
            )
            .at_path(at_path.clone())
        })?;
    let (_, body) = match preimage.strip_prefix(b"\xef\xbb\xbf") {
        Some(body) => (&b"\xef\xbb\xbf"[..], body),
        None => (&b""[..], preimage.as_slice()),
    };
    let source = String::from_utf8(body.to_vec()).map_err(|_| {
        ApplyPlanError::new(
            ApplyPlanErrorKind::InvalidSource,
            "metadata descriptor is not UTF-8",
        )
        .at_path(at_path.clone())
    })?;
    let (kind, postimage_text) = match changes {
        DescriptorPropertyChanges::Role(unica_metadata::property_input::RolePropertyChanges {
            synonym,
            comment,
        }) => {
            let mut text = source.clone();
            let indent = "\t\t\t";
            if let Some(synonym) = synonym {
                let lang = unica_source_designer::language::staged_text_language(
                    staged,
                    source_kind,
                    op_index,
                )?;
                text =
                    set_ml_property(&text, "Synonym", indent, &lang, synonym).ok_or_else(|| {
                        ApplyPlanError::new(
                            ApplyPlanErrorKind::InvalidSource,
                            "the role descriptor has no Synonym property",
                        )
                        .at_path(at_path.clone())
                    })?;
            }
            if let Some(comment) = comment {
                let block = if comment.is_empty() {
                    format!("{indent}<Comment/>")
                } else {
                    format!(
                        "{indent}<Comment>{}</Comment>",
                        comment
                            .replace('&', "&amp;")
                            .replace('<', "&lt;")
                            .replace('>', "&gt;")
                    )
                };
                text = replace_property_block(&text, "Comment", &block).ok_or_else(|| {
                    ApplyPlanError::new(
                        ApplyPlanErrorKind::InvalidSource,
                        "the role descriptor has no Comment property",
                    )
                    .at_path(at_path.clone())
                })?;
            }
            (NodeKind::Role, text)
        }
        DescriptorPropertyChanges::Subsystem(
            unica_metadata::property_input::SubsystemPropertyChanges {
                synonym,
                comment,
                explanation,
                include_help_in_contents,
                include_in_command_interface,
                use_one_command,
            },
        ) => {
            // Prove the descriptor before its existing local XML edits. The
            // typed patch changes only the configuration's default language.
            crate::subsystem::parse_subsystem_edit_model(&source, &relative.display().to_string())
                .map_err(|message| {
                    ApplyPlanError::new(ApplyPlanErrorKind::InvalidSource, message)
                        .at_path(at_path.clone())
                })?;
            let mut text = source.clone();
            let indent = "\t\t\t";
            let missing = |tag: &str| {
                ApplyPlanError::new(
                    ApplyPlanErrorKind::InvalidSource,
                    format!("the subsystem descriptor has no {tag} property"),
                )
                .at_path(at_path.clone())
            };
            let lang = if synonym.is_some() || explanation.is_some() {
                Some(unica_source_designer::language::staged_text_language(
                    staged,
                    source_kind,
                    op_index,
                )?)
            } else {
                None
            };
            if let Some(synonym) = synonym {
                text = set_ml_property(
                    &text,
                    "Synonym",
                    indent,
                    lang.as_deref()
                        .expect("localized patch retained its language"),
                    synonym,
                )
                .ok_or_else(|| missing("Synonym"))?;
            }
            if let Some(comment) = comment {
                text = set_plain_property(&text, "Comment", comment)
                    .ok_or_else(|| missing("Comment"))?;
            }
            if let Some(explanation) = explanation {
                text = set_ml_property(
                    &text,
                    "Explanation",
                    indent,
                    lang.as_deref()
                        .expect("localized patch retained its language"),
                    explanation,
                )
                .ok_or_else(|| missing("Explanation"))?;
            }
            if let Some(value) = include_help_in_contents {
                text = set_plain_property(&text, "IncludeHelpInContents", &value.to_string())
                    .ok_or_else(|| missing("IncludeHelpInContents"))?;
            }
            if let Some(value) = include_in_command_interface {
                text = set_plain_property(&text, "IncludeInCommandInterface", &value.to_string())
                    .ok_or_else(|| missing("IncludeInCommandInterface"))?;
            }
            if let Some(value) = use_one_command {
                text = set_plain_property(&text, "UseOneCommand", &value.to_string())
                    .ok_or_else(|| missing("UseOneCommand"))?;
            }
            (NodeKind::Subsystem, text)
        }
        DescriptorPropertyChanges::Configuration(
            unica_metadata::property_input::ConfigurationPropertyChanges { changes, .. },
        ) => {
            let mut text = source.clone();
            let missing = |tag: &str| {
                ApplyPlanError::new(
                    ApplyPlanErrorKind::InvalidSource,
                    format!("the configuration root has no {tag} property"),
                )
                .at_path(at_path.clone())
            };
            let lang = if changes.iter().any(|(property, _)| property.localized()) {
                Some(unica_source_designer::language::staged_text_language(
                    staged,
                    source_kind,
                    op_index,
                )?)
            } else {
                None
            };
            for (property, value) in changes {
                let tag = property.names()[0];
                let value = match value {
                    DescriptorPropertyValue::Text(value) => value.clone(),
                    DescriptorPropertyValue::Boolean(value) => value.to_string(),
                };
                text = if property.localized() {
                    set_ml_property(
                        &text,
                        tag,
                        "\t\t\t",
                        lang.as_deref()
                            .expect("localized patch retained its language"),
                        &value,
                    )
                } else {
                    set_plain_property(&text, tag, &value)
                }
                .ok_or_else(|| missing(tag))?;
            }
            (NodeKind::Configuration, text)
        }
    };
    let postimage = unica_source_designer::registration::preserve_descriptor_image(
        &preimage,
        &source,
        &postimage_text,
    );
    if postimage == preimage {
        return Ok(());
    }
    staged
        .replace(relative, &preimage, postimage)
        .map_err(|error| ApplyPlanError::staging(error, at_path))?;
    provisional.push(ProvisionalApplyEffect::single(
        relative.to_path_buf(),
        DomainEvent::new(
            match kind {
                NodeKind::Role => DomainEventKind::RoleChanged,
                NodeKind::Subsystem => DomainEventKind::SubsystemChanged,
                _ if source_kind == SourceSetKind::Extension => DomainEventKind::CfeChanged,
                _ => DomainEventKind::ConfigXmlChanged,
            },
            if kind == NodeKind::Configuration {
                name.to_string()
            } else {
                format!("{}.{name}", kind.as_str())
            },
        ),
        op_index,
    ));
    Ok(())
}

fn template_descriptor_xml(
    name: &str,
    kind: unica_metadata::MetaTemplateKind,
    format_version: &str,
    uuid: &str,
    lang: &str,
) -> String {
    let synonym = unica_metadata::identifier::split_camel_case(name)
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;");
    let synonym_xml = if synonym.is_empty() {
        "\t\t\t<Synonym/>".to_owned()
    } else {
        format!(
            "\t\t\t<Synonym>\r\n\t\t\t\t<v8:item>\r\n\t\t\t\t\t<v8:lang>{lang}</v8:lang>\r\n\t\t\t\t\t<v8:content>{synonym}</v8:content>\r\n\t\t\t\t</v8:item>\r\n\t\t\t</Synonym>"
        )
    };
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\r\n<MetaDataObject {} version=\"{format_version}\">\r\n\t<Template uuid=\"{uuid}\">\r\n\t\t<Properties>\r\n\t\t\t<Name>{name}</Name>\r\n{synonym_xml}\r\n\t\t\t<Comment/>\r\n\t\t\t<TemplateType>{}</TemplateType>\r\n\t\t</Properties>\r\n\t</Template>\r\n</MetaDataObject>",
        unica_source_designer::layout::full_md_namespace_declarations(),
        kind.as_str()
    )
}

fn empty_data_composition_schema_xml() -> String {
    concat!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\r\n",
        "<DataCompositionSchema xmlns=\"http://v8.1c.ru/8.1/data-composition-system/schema\" xmlns:dcscom=\"http://v8.1c.ru/8.1/data-composition-system/common\" xmlns:dcscor=\"http://v8.1c.ru/8.1/data-composition-system/core\" xmlns:dcsset=\"http://v8.1c.ru/8.1/data-composition-system/settings\" xmlns:v8=\"http://v8.1c.ru/8.1/data/core\" xmlns:v8ui=\"http://v8.1c.ru/8.1/data/ui\" xmlns:xs=\"http://www.w3.org/2001/XMLSchema\" xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\">\r\n",
        "\t<dataSource>\r\n\t\t<name>ИсточникДанных1</name>\r\n\t\t<dataSourceType>Local</dataSourceType>\r\n\t</dataSource>\r\n",
        "\t<settingsVariant>\r\n\t\t<dcsset:name>Основной</dcsset:name>\r\n\t\t<dcsset:presentation xsi:type=\"xs:string\">Основной</dcsset:presentation>\r\n\t\t<dcsset:settings xmlns:style=\"http://v8.1c.ru/8.1/data/ui/style\" xmlns:sys=\"http://v8.1c.ru/8.1/data/ui/fonts/system\" xmlns:web=\"http://v8.1c.ru/8.1/data/ui/colors/web\" xmlns:win=\"http://v8.1c.ru/8.1/data/ui/colors/windows\">\r\n\t\t\t<dcsset:item xsi:type=\"dcsset:StructureItemGroup\">\r\n\t\t\t\t<dcsset:selection>\r\n\t\t\t\t\t<dcsset:item xsi:type=\"dcsset:SelectedItemAuto\"/>\r\n\t\t\t\t</dcsset:selection>\r\n\t\t\t</dcsset:item>\r\n\t\t</dcsset:settings>\r\n\t</settingsVariant>\r\n",
        "</DataCompositionSchema>",
    )
    .to_string()
}

/// Adds `<Tag>Name</Tag>` to the owner's outer `ChildObjects` (the block
/// closed at two tabs) in the platform slot of its kind (#1385): before the
/// first entry of a kind the export writes later, else after the last
/// sibling of the same tag when there is one.
fn register_owner_child_ref(text: &str, tag: &str, name: &str) -> Option<String> {
    let entry = format!("\t\t\t<{tag}>{name}</{tag}>\n");
    let mut updated = text.to_string();
    if let Some(offset) =
        unica_source_designer::registration::platform_child_object_insertion_offset(text, tag)
    {
        updated.insert_str(offset, &entry);
        return Some(updated);
    }
    if let Some(index) = text.rfind(&format!("\n\t\t\t<{tag}>")) {
        let line_end = text[index + 1..].find('\n')? + index + 2;
        updated.insert_str(line_end, &entry);
        return Some(updated);
    }
    if let Some(close) = text.rfind("\n\t\t</ChildObjects>") {
        updated.insert_str(close + 1, &entry);
        return Some(updated);
    }
    // Compact layouts: an empty `<ChildObjects/>` or a close on the same
    // line as its content.
    if let Some(empty) = text.rfind("<ChildObjects/>") {
        updated.replace_range(
            empty..empty + "<ChildObjects/>".len(),
            &format!("<ChildObjects>\n{entry}\t\t</ChildObjects>"),
        );
        return Some(updated);
    }
    let close = text.rfind("</ChildObjects>")?;
    updated.insert_str(close, &format!("\n{entry}\t\t"));
    Some(updated)
}

fn stage_exchange_content(
    staged: &mut ApplyProposal<'_>,
    authority: &dyn ApplySourceAuthority,
    owner: &MetadataAddress,
    items: &[unica_metadata::MetaExchangeContentItem],
    op_index: usize,
    provisional: &mut Vec<ProvisionalApplyEffect>,
) -> Result<(), ApplyPlanError> {
    let path = format!("ops[{op_index}].args.items");
    let configuration = staged
        .read(Path::new("Configuration.xml"))
        .map_err(|e| ApplyPlanError::staging(e, path.clone()))?
        .ok_or_else(|| {
            ApplyPlanError::new(
                ApplyPlanErrorKind::InvalidSource,
                "Configuration.xml is absent",
            )
            .at_path(path.clone())
        })?;
    let invalid_configuration = |message: String| {
        ApplyPlanError::new(ApplyPlanErrorKind::InvalidSource, message).at_path(path.clone())
    };
    let configuration_text = std::str::from_utf8(&configuration)
        .map_err(|_| invalid_configuration("Configuration.xml is not UTF-8".into()))?;
    let configuration_doc = roxmltree::Document::parse(configuration_text)
        .map_err(|e| invalid_configuration(format!("invalid Configuration.xml: {e}")))?;
    const MD_NS: &str = "http://v8.1c.ru/8.3/MDClasses";
    let root = configuration_doc.root_element();
    if !root.has_tag_name((MD_NS, "MetaDataObject")) {
        return Err(invalid_configuration(
            "expected MetaDataObject in Configuration.xml".into(),
        ));
    }
    let mut configurations = root
        .children()
        .filter(|n| n.has_tag_name((MD_NS, "Configuration")));
    let configuration_node = configurations.next().ok_or_else(|| {
        invalid_configuration("Configuration.xml has no Configuration element".into())
    })?;
    if configurations.next().is_some() {
        return Err(invalid_configuration(
            "Configuration.xml has repeated Configuration elements".into(),
        ));
    }
    let mut compositions = configuration_node
        .children()
        .filter(|n| n.has_tag_name((MD_NS, "ChildObjects")));
    let composition = compositions.next().ok_or_else(|| {
        invalid_configuration("Configuration.xml has no ChildObjects element".into())
    })?;
    if compositions.next().is_some() {
        return Err(invalid_configuration(
            "Configuration.xml has repeated ChildObjects elements".into(),
        ));
    }
    let targets = std::iter::once((owner, format!("ops[{op_index}].args.at"))).chain(
        items.iter().enumerate().map(|(index, item)| {
            (
                &item.target,
                format!("ops[{op_index}].args.items[{index}].at"),
            )
        }),
    );
    for (target, target_path) in targets {
        let (_, _, text) = owner_descriptor_and_text(staged, authority, target, op_index)
            .map_err(|e| e.at_path(target_path.clone()))?;
        let (kind, name) = crate::edit::meta_edit_object_identity(&text).map_err(|e| {
            ApplyPlanError::new(ApplyPlanErrorKind::InvalidSource, e).at_path(target_path.clone())
        })?;
        if !unica_metadata::metadata_name_eq(target.as_str(), &format!("{kind}.{name}")) {
            return Err(ApplyPlanError::new(
                ApplyPlanErrorKind::InvalidSource,
                "content target descriptor identity does not match its address",
            )
            .at_path(target_path.clone()));
        }
        let registered = composition.children().any(|entry| {
            entry.has_tag_name((MD_NS, kind.as_str()))
                && entry.attributes().len() == 0
                && !entry.children().any(|n| n.is_element())
                && unica_metadata::metadata_name_eq(
                    &entry
                        .children()
                        .filter(|n| n.is_text())
                        .filter_map(|n| n.text())
                        .collect::<String>(),
                    &name,
                )
        });
        if !registered {
            return Err(ApplyPlanError::new(
                ApplyPlanErrorKind::NotFound,
                "content target is not registered in Configuration.xml",
            )
            .at_path(target_path.clone()));
        }
    }
    let relative = metadata_descriptor_relative(owner, authority.source_kind())
        .map_err(|e| ApplyPlanError::new(ApplyPlanErrorKind::BadValue, e).at_path(path.clone()))?
        .with_extension("")
        .join("Ext/Content.xml");
    // The platform omits Content.xml for an empty composition; the first
    // added item creates the file in the platform layout.
    let before = staged
        .read(&relative)
        .map_err(|e| ApplyPlanError::staging(e, path.clone()))?;
    let template = crate::exchange_content::EMPTY_CONTENT.as_bytes();
    let text = std::str::from_utf8(before.as_deref().unwrap_or(template)).map_err(|_| {
        ApplyPlanError::new(
            ApplyPlanErrorKind::InvalidSource,
            "ExchangePlan Content.xml is not UTF-8",
        )
        .at_path(path.clone())
    })?;
    let after = crate::exchange_content::update_content(text, items)
        .map_err(|e| {
            ApplyPlanError::new(ApplyPlanErrorKind::InvalidSource, e).at_path(path.clone())
        })?
        .into_bytes();
    let changed = match before {
        Some(before) if before != after => staged.replace(&relative, &before, after).map(|()| true),
        None if after != template => staged.create(&relative, after).map(|()| true),
        _ => Ok(false),
    };
    let changed = changed.map_err(|e| ApplyPlanError::staging(e, path))?;
    if changed {
        provisional.push(ProvisionalApplyEffect::single(
            relative,
            DomainEvent::new(DomainEventKind::MetadataChanged, owner.as_str().to_owned()),
            op_index,
        ));
    }
    Ok(())
}

/// The final composition of every ExchangePlan named by the batch, read
/// from the staged image after all families planned their changes.
pub fn final_exchange_content(
    staged: &mut ApplyProposal<'_>,
    source_set_name: &str,
    source_kind: SourceSetKind,
    owners: impl Iterator<Item = MetadataAddress>,
) -> Result<Vec<unica_metadata::ExchangeContentPlanDetail>, ApplyPlanError> {
    use unica_metadata::{ExchangeContentPlanDetail, ExchangeContentPlanItem};
    let mut seen = std::collections::HashSet::new();
    let mut details = Vec::new();
    for owner in owners {
        if !seen.insert(owner.as_str().to_lowercase()) {
            continue;
        }
        let fail = |message| {
            ApplyPlanError::new(ApplyPlanErrorKind::InvalidSource, message).at_path("ops")
        };
        let relative = metadata_descriptor_relative(&owner, source_kind).map_err(fail)?;
        if staged
            .read(&relative)
            .map_err(|e| ApplyPlanError::staging(e, "ops"))?
            .is_none()
        {
            continue;
        }
        let content = relative.with_extension("").join("Ext/Content.xml");
        let bytes = staged
            .read(&content)
            .map_err(|e| ApplyPlanError::staging(e, "ops"))?;
        // An absent file is the platform's empty composition.
        let bytes =
            bytes.unwrap_or_else(|| crate::exchange_content::EMPTY_CONTENT.as_bytes().to_vec());
        let text = std::str::from_utf8(&bytes).map_err(|e| fail(e.to_string()))?;
        let observed = crate::exchange_content::read_content(text).map_err(fail)?;
        if !observed.errors.is_empty() {
            return Err(fail(observed.errors.join("; ")));
        }
        details.push(ExchangeContentPlanDetail {
            at: format!("{}:{}", source_set_name, owner.as_str()),
            items: observed
                .items
                .into_iter()
                .map(|item| ExchangeContentPlanItem {
                    at: format!("{}:{}", source_set_name, item.target.as_str()),
                    auto_record: item.auto_record,
                })
                .collect(),
        });
    }
    Ok(details)
}

fn owner_descriptor_and_text(
    staged: &mut ApplyProposal<'_>,
    authority: &dyn ApplySourceAuthority,
    owner: &MetadataAddress,
    op_index: usize,
) -> Result<(PathBuf, Vec<u8>, String), ApplyPlanError> {
    let at_path = format!("ops[{op_index}].args.at");
    let relative =
        metadata_descriptor_relative(owner, authority.source_kind()).map_err(|message| {
            ApplyPlanError::new(ApplyPlanErrorKind::BadValue, message).at_path(at_path.clone())
        })?;
    let preimage = staged
        .read(&relative)
        .map_err(|error| ApplyPlanError::staging(error, at_path.clone()))?
        .ok_or_else(|| {
            ApplyPlanError::new(
                ApplyPlanErrorKind::NotFound,
                "metadata descriptor was not found",
            )
            .at_path(at_path.clone())
        })?;
    let text = String::from_utf8(
        preimage
            .strip_prefix(b"\xef\xbb\xbf")
            .unwrap_or(&preimage)
            .to_vec(),
    )
    .map_err(|_| {
        ApplyPlanError::new(
            ApplyPlanErrorKind::InvalidSource,
            "metadata descriptor is not UTF-8",
        )
        .at_path(at_path)
    })?;
    Ok((relative, preimage, text))
}

fn bom_bytes(text: &str) -> Vec<u8> {
    let mut bytes = b"\xef\xbb\xbf".to_vec();
    bytes.extend_from_slice(text.as_bytes());
    bytes
}

fn stage_template_add(
    template_seed: TemplateSeedEmitter,
    staged: &mut ApplyProposal<'_>,
    authority: &dyn ApplySourceAuthority,
    owner: &MetadataAddress,
    items: &[(String, unica_metadata::MetaTemplateKind)],
    op_index: usize,
    provisional: &mut Vec<ProvisionalApplyEffect>,
) -> Result<(), ApplyPlanError> {
    use unica_metadata::MetaTemplateKind;
    let (owner_relative, owner_preimage, owner_source) =
        owner_descriptor_and_text(staged, authority, owner, op_index)?;
    let mut owner_text = owner_source.clone();
    let templates_dir = owner_relative.with_extension("").join("Templates");
    let lang = unica_source_designer::language::staged_text_language(
        staged,
        authority.source_kind(),
        op_index,
    )?;
    let mut touched = Vec::new();
    for (index, (name, kind)) in items.iter().enumerate() {
        let name_path = format!("ops[{op_index}].args.items[{index}].name");
        if owner_text.contains(&format!("<Template>{name}</Template>")) {
            return Err(ApplyPlanError::new(
                ApplyPlanErrorKind::InvalidState,
                format!("template `{name}` already exists on `{}`", owner.as_str()),
            )
            .at_path(name_path));
        }
        let content = match kind {
            MetaTemplateKind::SpreadsheetDocument => {
                (template_seed.0)()
            }
            MetaTemplateKind::DataCompositionSchema => empty_data_composition_schema_xml(),
            MetaTemplateKind::TextDocument | MetaTemplateKind::HtmlDocument | MetaTemplateKind::BinaryData => {
                return Err(ApplyPlanError::new(
                    ApplyPlanErrorKind::BadValue,
                    format!(
                        "template type `{}` needs content the planner cannot synthesize; create a SpreadsheetDocument or DataCompositionSchema template",
                        kind.as_str()
                    ),
                )
                .at_path(format!("ops[{op_index}].args.items[{index}].templateType")))
            }
        };
        let uuid = {
            use sha2::Digest;
            let digest = sha2::Sha256::digest(format!(
                "unica-v13-template-uuid-v1\0{}\0{}.Template.{name}",
                authority.source_set_name(),
                owner.as_str()
            ));
            let mut bytes = [0_u8; 16];
            bytes.copy_from_slice(&digest[..16]);
            bytes[6] = (bytes[6] & 0x0f) | 0x40;
            bytes[8] = (bytes[8] & 0x3f) | 0x80;
            uuid::Uuid::from_bytes(bytes).to_string()
        };
        let descriptor =
            template_descriptor_xml(name, *kind, authority.expected_format(), &uuid, &lang);
        let descriptor_relative = templates_dir.join(format!("{name}.xml"));
        let content_relative = templates_dir.join(name).join("Ext/Template.xml");
        for (relative, text) in [
            (&descriptor_relative, &descriptor),
            (&content_relative, &content),
        ] {
            if staged
                .read(relative)
                .map_err(|error| ApplyPlanError::staging(error, name_path.clone()))?
                .is_some()
            {
                return Err(ApplyPlanError::new(
                    ApplyPlanErrorKind::InvalidState,
                    format!("`{}` already exists", relative.display()),
                )
                .at_path(name_path));
            }
            staged
                .create(relative, bom_bytes(text))
                .map_err(|error| ApplyPlanError::staging(error, name_path.clone()))?;
            touched.push(relative.clone());
        }
        owner_text = register_owner_child_ref(&owner_text, "Template", name).ok_or_else(|| {
            ApplyPlanError::new(
                ApplyPlanErrorKind::InvalidSource,
                "the owner descriptor has no ChildObjects block to register the template",
            )
            .at_path(format!("ops[{op_index}].args.at"))
        })?;
    }
    let owner_postimage = unica_source_designer::registration::preserve_descriptor_image(
        &owner_preimage,
        &owner_source,
        &owner_text,
    );
    staged
        .replace(&owner_relative, &owner_preimage, owner_postimage)
        .map_err(|error| ApplyPlanError::staging(error, format!("ops[{op_index}].args.at")))?;
    touched.push(owner_relative);
    provisional.push(ProvisionalApplyEffect::spanning(
        touched,
        DomainEvent::new(DomainEventKind::TemplateChanged, owner.as_str().to_string()),
        op_index,
    ));
    Ok(())
}

fn stage_template_set(
    staged: &mut ApplyProposal<'_>,
    authority: &dyn ApplySourceAuthority,
    owner: &MetadataAddress,
    name: &str,
    values: &Map<String, Value>,
    op_index: usize,
    provisional: &mut Vec<ProvisionalApplyEffect>,
) -> Result<(), ApplyPlanError> {
    let at_path = format!("ops[{op_index}].args.at");
    let (owner_relative, _, owner_source) =
        owner_descriptor_and_text(staged, authority, owner, op_index)?;
    if !owner_source.contains(&format!("<Template>{name}</Template>")) {
        return Err(ApplyPlanError::new(
            ApplyPlanErrorKind::NotFound,
            format!("`{}` has no template `{name}`", owner.as_str()),
        )
        .at_path(at_path));
    }
    let relative = owner_relative
        .with_extension("")
        .join("Templates")
        .join(format!("{name}.xml"));
    let preimage = staged
        .read(&relative)
        .map_err(|error| ApplyPlanError::staging(error, at_path.clone()))?
        .ok_or_else(|| {
            ApplyPlanError::new(
                ApplyPlanErrorKind::NotFound,
                "the template descriptor was not found",
            )
            .at_path(at_path.clone())
        })?;
    let (_, body) = match preimage.strip_prefix(b"\xef\xbb\xbf") {
        Some(body) => (&b"\xef\xbb\xbf"[..], body),
        None => (&b""[..], preimage.as_slice()),
    };
    let mut text = String::from_utf8(body.to_vec()).map_err(|_| {
        ApplyPlanError::new(
            ApplyPlanErrorKind::InvalidSource,
            "the template descriptor is not UTF-8",
        )
        .at_path(at_path.clone())
    })?;
    let source_text = text.clone();
    let string_value = |key: &str| -> Result<Option<String>, ApplyPlanError> {
        match values.get(key) {
            None => Ok(None),
            Some(Value::String(text)) => Ok(Some(text.clone())),
            Some(_) => Err(ApplyPlanError::new(
                ApplyPlanErrorKind::BadValue,
                format!("`{key}` must be a string"),
            )
            .at_path(format!("ops[{op_index}].args.values.{key}"))),
        }
    };
    let indent = "\t\t\t";
    if let Some(synonym) = string_value("synonym")? {
        let lang = unica_source_designer::language::staged_text_language(
            staged,
            authority.source_kind(),
            op_index,
        )?;
        text = set_ml_property(&text, "Synonym", indent, &lang, &synonym).ok_or_else(|| {
            ApplyPlanError::new(
                ApplyPlanErrorKind::InvalidSource,
                "the template descriptor has no Synonym property",
            )
            .at_path(at_path.clone())
        })?;
    }
    if let Some(comment) = string_value("comment")? {
        let block = if comment.is_empty() {
            format!("{indent}<Comment/>")
        } else {
            format!(
                "{indent}<Comment>{}</Comment>",
                comment
                    .replace('&', "&amp;")
                    .replace('<', "&lt;")
                    .replace('>', "&gt;")
            )
        };
        text = replace_property_block(&text, "Comment", &block).ok_or_else(|| {
            ApplyPlanError::new(
                ApplyPlanErrorKind::InvalidSource,
                "the template descriptor has no Comment property",
            )
            .at_path(at_path.clone())
        })?;
    }
    if let Some(kind) = string_value("templateType")? {
        let kind = unica_metadata::MetaTemplateKind::parse(&kind).map_err(|diagnostic| {
            ApplyPlanError::new(ApplyPlanErrorKind::BadValue, diagnostic.message)
                .at_path(format!("ops[{op_index}].args.values.templateType"))
        })?;
        text = replace_property_block(
            &text,
            "TemplateType",
            &format!("{indent}<TemplateType>{}</TemplateType>", kind.as_str()),
        )
        .ok_or_else(|| {
            ApplyPlanError::new(
                ApplyPlanErrorKind::InvalidSource,
                "the template descriptor has no TemplateType property",
            )
            .at_path(at_path.clone())
        })?;
    }
    let postimage = unica_source_designer::registration::preserve_descriptor_image(
        &preimage,
        &source_text,
        &text,
    );
    if postimage == preimage {
        return Ok(());
    }
    staged
        .replace(&relative, &preimage, postimage)
        .map_err(|error| ApplyPlanError::staging(error, at_path))?;
    provisional.push(ProvisionalApplyEffect::single(
        relative,
        DomainEvent::new(
            DomainEventKind::TemplateChanged,
            format!("{}.Template.{name}", owner.as_str()),
        ),
        op_index,
    ));
    Ok(())
}

/// Include indentation and its newline only when the confirmed registration
/// is the sole token on that line; compact XML and comments keep their bytes.
fn template_removal_registration_span(
    source: &str,
    range: std::ops::Range<usize>,
) -> std::ops::Range<usize> {
    let line_start = source[..range.start].rfind('\n').map_or(0, |at| at + 1);
    let only_indent = source[line_start..range.start]
        .chars()
        .all(|character| matches!(character, ' ' | '\t' | '\r'));
    if only_indent && source[range.end..].starts_with("\r\n") {
        line_start..range.end + 2
    } else if only_indent && source[range.end..].starts_with('\n') {
        line_start..range.end + 1
    } else {
        range
    }
}

fn stage_template_remove(
    staged: &mut ApplyProposal<'_>,
    authority: &dyn ApplySourceAuthority,
    owner: &MetadataAddress,
    name: &str,
    op_index: usize,
    provisional: &mut Vec<ProvisionalApplyEffect>,
) -> Result<(), ApplyPlanError> {
    let at_path = format!("ops[{op_index}].args.at");
    unica_source_designer::registration::require_untouched_staged_state(
        staged,
        "template.remove",
        &at_path,
    )?;
    let (owner_relative, owner_preimage, owner_source) =
        owner_descriptor_and_text(staged, authority, owner, op_index)?;
    let owner_source = owner_source.trim_start_matches('\u{feff}').to_string();
    let templates_dir = owner_relative.with_extension("").join("Templates");
    let descriptor_relative = templates_dir.join(format!("{name}.xml"));
    let descriptor_preimage = staged
        .read(&descriptor_relative)
        .map_err(|error| ApplyPlanError::staging(error, at_path.clone()))?
        .ok_or_else(|| {
            ApplyPlanError::new(
                ApplyPlanErrorKind::InvalidSource,
                "registered template descriptor is missing",
            )
            .at_path(at_path.clone())
        })?;
    let payload_relative = templates_dir.join(name);
    let (files, payload_absent) = staged
        .enumerate_tree_with_root_absence(&payload_relative)
        .map_err(|error| ApplyPlanError::staging(error, at_path.clone()))?;
    let inherited = if payload_absent && authority.source_kind() == SourceSetKind::Extension {
        let configuration = staged
            .read(Path::new("Configuration.xml"))
            .map_err(|error| ApplyPlanError::staging(error, at_path.clone()))?
            .ok_or_else(|| {
                ApplyPlanError::new(
                    ApplyPlanErrorKind::InvalidSource,
                    "extension root descriptor is missing",
                )
                .at_path(at_path.clone())
            })?;
        crate::validation::template_removal_has_inherited_owners(
            &configuration,
            &owner_preimage,
            &descriptor_preimage,
        )
        .map_err(|message| {
            ApplyPlanError::new(ApplyPlanErrorKind::InvalidSource, message).at_path(at_path.clone())
        })?
    } else {
        false
    };
    let identity = crate::validation::validate_template_removal_identity(
        &owner_source,
        &descriptor_preimage,
        owner,
        name,
        inherited,
    )
    .map_err(|message| {
        ApplyPlanError::new(ApplyPlanErrorKind::InvalidSource, message).at_path(at_path.clone())
    })?;
    if !inherited && files.is_empty() {
        return Err(ApplyPlanError::new(
            ApplyPlanErrorKind::InvalidSource,
            "template content is missing",
        )
        .at_path(at_path.clone()));
    }
    if let Some(kind) = identity.template_type.as_deref() {
        if let Some(primary) = crate::validation::template_removal_primary_path(kind) {
            let bytes = staged
                .read(&payload_relative.join(primary))
                .map_err(|error| ApplyPlanError::staging(error, at_path.clone()))?;
            match bytes {
                Some(bytes) => {
                    crate::validation::validate_template_removal_primary_image(kind, &bytes)
                        .map_err(|message| {
                            ApplyPlanError::new(ApplyPlanErrorKind::InvalidSource, message)
                                .at_path(at_path.clone())
                        })?
                }
                None if inherited => {}
                None => {
                    return Err(ApplyPlanError::new(
                        ApplyPlanErrorKind::InvalidSource,
                        "template primary resource is missing",
                    )
                    .at_path(at_path.clone()))
                }
            }
        }
    }
    let reference = format!("{}.Template.{name}", owner.as_str());
    let registration = template_removal_registration_span(&owner_source, identity.registration);
    let mut owner_edits = identity.reference_edits;
    owner_edits.push((registration, String::new()));
    owner_edits.sort_by_key(|edit| std::cmp::Reverse(edit.0.start));
    let mut owner_text = owner_source.clone();
    for (range, replacement) in owner_edits {
        owner_text.replace_range(range, &replacement);
    }
    let mut touched = Vec::new();
    staged
        .remove(&descriptor_relative, &descriptor_preimage)
        .map_err(|error| ApplyPlanError::staging(error, at_path.clone()))?;
    touched.push(descriptor_relative.clone());
    for relative in files {
        if let Some(preimage) = staged
            .read(&relative)
            .map_err(|error| ApplyPlanError::staging(error, at_path.clone()))?
        {
            staged
                .remove(&relative, &preimage)
                .map_err(|error| ApplyPlanError::staging(error, at_path.clone()))?;
            touched.push(relative);
        }
    }
    // The emptied payload and, when nothing else remains there, its
    // `Templates` collection go in the same publication: the platform does
    // not export empty template directories (#1997).
    staged
        .remove_owned_directories(&payload_relative, &descriptor_relative, &templates_dir)
        .map_err(|error| ApplyPlanError::staging(error, at_path.clone()))?;
    staged
        .replace(
            &owner_relative,
            &owner_preimage,
            unica_source_designer::registration::preserve_descriptor_image(
                &owner_preimage,
                &owner_source,
                &owner_text,
            ),
        )
        .map_err(|error| ApplyPlanError::staging(error, at_path))?;
    touched.push(owner_relative);
    provisional.push(ProvisionalApplyEffect::spanning(
        touched,
        DomainEvent::new(DomainEventKind::TemplateChanged, reference),
        op_index,
    ));
    Ok(())
}

/// Stage one predefined-item change in the owner's `Ext/Predefined.xml`.
///
/// The file is read from the staged state, so an item added earlier in the
/// batch is visible to a later `set` or `remove` of the same batch. `set` and
/// `remove` find the item by the name in their address; the platform keeps
/// predefined names unique within the owner, nested items included.
fn stage_predefined(
    staged: &mut ApplyProposal<'_>,
    authority: &dyn ApplySourceAuthority,
    owner: &MetadataAddress,
    kind: MetadataKind,
    change: &PredefinedChange,
    op_index: usize,
    provisional: &mut Vec<ProvisionalApplyEffect>,
) -> Result<(), ApplyPlanError> {
    let at_path = format!("ops[{op_index}].args.at");
    let (owner_relative, _, owner_source) =
        owner_descriptor_and_text(staged, authority, owner, op_index)?;
    // Коды предопределённых проверяются по `CodeType` владельца; описание
    // без него (заимствованный справочник платформа выгружает так) не даёт
    // вида кодов, и запись отказывает тем же текстом, что и чтение.
    crate::predefined::require_predefined_code_type(kind, &owner_source).map_err(|message| {
        ApplyPlanError::new(ApplyPlanErrorKind::InvalidSource, message).at_path(at_path.clone())
    })?;
    let relative = owner_relative.with_extension("").join("Ext/Predefined.xml");
    let original = staged
        .read(&relative)
        .map_err(|error| ApplyPlanError::staging(error, at_path.clone()))?;
    let item_id = |name: &str| -> Result<String, ApplyPlanError> {
        let found = match &original {
            Some(bytes) => {
                crate::predefined::predefined_item_id(bytes, kind, name).map_err(|message| {
                    ApplyPlanError::new(ApplyPlanErrorKind::InvalidSource, message)
                        .at_path(at_path.clone())
                })?
            }
            None => None,
        };
        found.ok_or_else(|| {
            ApplyPlanError::new(
                ApplyPlanErrorKind::NotFound,
                format!("`{}` has no predefined item `{name}`", owner.as_str()),
            )
            .at_path(at_path.clone())
        })
    };
    let operation = match change {
        PredefinedChange::Add(operation) => operation.clone(),
        PredefinedChange::Set { name, values } => predefined_update(
            authority.source_set_name(),
            owner,
            kind,
            &item_id(name)?,
            values,
            op_index,
        )?,
        PredefinedChange::Remove { name } => edit_operation(parse_legacy_edit(
            authority.source_set_name(),
            owner.clone(),
            kind,
            json!({"op": "remove", "collection": "predefinedItems", "ids": [item_id(name)?]}),
            |field| format!("ops[{op_index}].args.{field}"),
            op_index,
        )?),
    };
    let post_image = crate::predefined::predefined_post_image(
        kind,
        owner,
        &owner_source,
        original.as_deref(),
        &operation,
    )
    .map_err(|failure| {
        // A name or id already taken is the caller's to change: the
        // refusal asks for another value rather than for a human.
        let duplicate = failure
            .diagnostics
            .first()
            .filter(|diagnostic| diagnostic.code == MetaDiagnosticCode::AlreadyExists)
            .map(|diagnostic| diagnostic.message.clone());
        let error = meta_failure_to_plan_error(failure, op_index);
        let error = match (duplicate, error.path().map(str::to_string)) {
            (Some(message), Some(path)) => {
                ApplyPlanError::new(ApplyPlanErrorKind::BadValue, message).at_path(path)
            }
            _ => error,
        };
        // The typed writer names fields of its own request (`elements[0]`,
        // `ids[0]`); the caller wrote `items`, `values` or the address.
        let path = error
            .path()
            .map(|path| match change {
                PredefinedChange::Add(_) => {
                    path.replacen("args.values.elements[", "args.items[", 1)
                }
                PredefinedChange::Set { .. } => path
                    .replacen("args.values.elements[0].name", "args.values.newName", 1)
                    .replacen("args.values.elements[0].id", "args.at", 1)
                    .replacen("args.values.elements[0]", "args.values", 1),
                PredefinedChange::Remove { .. } => {
                    path.replacen("args.values.ids[0]", "args.at", 1)
                }
            })
            .unwrap_or_else(|| format!("ops[{op_index}].args"));
        error.at_path(path)
    })?;
    let changed = match (&original, post_image) {
        (None, None) => false,
        (None, Some(post)) => {
            staged
                .create(&relative, post)
                .map_err(|error| ApplyPlanError::staging(error, at_path.clone()))?;
            true
        }
        (Some(pre), Some(post)) if *pre != post => {
            staged
                .replace(&relative, pre, post)
                .map_err(|error| ApplyPlanError::staging(error, at_path.clone()))?;
            true
        }
        (Some(_), Some(_)) => false,
        (Some(pre), None) => {
            staged
                .remove(&relative, pre)
                .map_err(|error| ApplyPlanError::staging(error, at_path.clone()))?;
            true
        }
    };
    if changed {
        provisional.push(ProvisionalApplyEffect::single(
            relative,
            DomainEvent::new(DomainEventKind::MetadataChanged, owner.as_str().to_string()),
            op_index,
        ));
    }
    Ok(())
}

struct HelpPage<'a> {
    name: &'a str,
    language: &'a str,
}

fn stage_help_create<R: MetadataPlanningRead>(
    reader: &R,
    staged: &mut ApplyProposal<'_>,
    authority: &dyn ApplySourceAuthority,
    target: &MetadataAddress,
    page: HelpPage<'_>,
    op_index: usize,
    provisional: &mut Vec<ProvisionalApplyEffect>,
) -> Result<(), ApplyPlanError> {
    let HelpPage {
        name,
        language: lang,
    } = page;

    let at_path = format!("ops[{op_index}].args.at");
    // The help facet planner inspects the owner's forms on the physical root.
    unica_source_designer::registration::require_untouched_staged_state(
        staged,
        "help.create",
        &at_path,
    )?;
    let root = authority.source_root();
    let descriptor_relative = metadata_descriptor_relative(target, authority.source_kind())
        .map_err(|message| {
            ApplyPlanError::new(ApplyPlanErrorKind::BadValue, message).at_path(at_path.clone())
        })?;
    if staged
        .read(&descriptor_relative)
        .map_err(|error| ApplyPlanError::staging(error, at_path.clone()))?
        .is_none()
    {
        return Err(ApplyPlanError::new(
            ApplyPlanErrorKind::NotFound,
            "metadata descriptor was not found",
        )
        .at_path(at_path));
    }
    let changes = crate::help::plan_help_facet_files(
        reader.ordinary_read(),
        &root.join(&descriptor_relative),
        target,
        name,
        lang,
    )
    .map_err(|failure| meta_failure_to_plan_error(failure, op_index))?;
    let mut touched = Vec::new();
    for (path, preimage, postimage) in changes {
        let relative = staged_relative(root, &path, op_index)?;
        match (preimage, postimage) {
            (None, Some(bytes)) => staged
                .create(&relative, bytes)
                .map_err(|error| ApplyPlanError::staging(error, at_path.clone()))?,
            (Some(expected), Some(bytes)) => staged
                .replace(&relative, &expected, bytes)
                .map_err(|error| ApplyPlanError::staging(error, at_path.clone()))?,
            (Some(expected), None) => staged
                .remove(&relative, &expected)
                .map_err(|error| ApplyPlanError::staging(error, at_path.clone()))?,
            (None, None) => continue,
        }
        touched.push(relative);
    }
    provisional.push(ProvisionalApplyEffect::spanning(
        touched,
        DomainEvent::new(
            DomainEventKind::MetadataChanged,
            target.as_str().to_string(),
        ),
        op_index,
    ));
    Ok(())
}
