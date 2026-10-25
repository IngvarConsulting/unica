//! Existing typed child resource plan and observation algorithms over issued reads.
use crate::edit::*;
use crate::resource_plan::{TypedChildFileMutation, TypedChildObservation, TypedChildResourcePlan};
use roxmltree::Document;
use std::collections::{BTreeMap, BTreeSet};
use std::io::Read;
use std::path::{Path, PathBuf};
use unica_address::source_target::{MetadataAddress, PLATFORM_XML_8_3_27_FORMAT_2_20};
use unica_metadata::{
    MetaCollection, MetaDiagnosticCode, MetaEditOperation, MetaPublicationAction,
    MetaPublicationPlanEntry, MetaPublicationResource,
};
use unica_source_contract::discovery::ProjectDiscoveryRead;
use unica_source_contract::proposal::{DirectoryMembershipSelector, DirectoryMembershipSnapshot};
/// Actual IO topology observation, issued by composition at the original callpoint.
pub type DirectoryMembershipObserver =
    fn(&Path, DirectoryMembershipSelector) -> Result<DirectoryMembershipSnapshot, String>;
/// Required existing representation emitters. No fallback or registration exists.
#[derive(Clone, Copy)]
pub struct TemplateSeedEmitters {
    pub spreadsheet: crate::planner::TemplateSeedEmitter,
    pub html_descriptor: fn(&str) -> String,
    pub html_page: fn() -> &'static str,
    pub content_xml: fn(&str, &str) -> Result<String, String>,
}
/// Required existing observation and seed bindings; construction does not invoke them.
#[derive(Clone, Copy)]
pub struct ChildResourceBindings {
    pub observe_membership: DirectoryMembershipObserver,
    pub templates: TemplateSeedEmitters,
    pub form_seed: fn(&str, &str) -> String,
}

pub fn plan_typed_child_resources<G>(
    reader: &dyn ProjectDiscoveryRead,
    bindings: ChildResourceBindings,
    descriptor: crate::resource_plan::MetadataDescriptorTarget<'_>,
    object_kind: &str,
    object_name: &str,
    operations: &[MetaEditOperation],
    post_image: &str,
) -> Result<TypedChildResourcePlan<G>, unica_metadata::request::MetaFailure> {
    let ChildResourceBindings {
        observe_membership,
        templates,
        form_seed,
    } = bindings;
    let crate::resource_plan::MetadataDescriptorTarget {
        path: descriptor_path,
        address: owner,
    } = descriptor;

    #[derive(Debug)]
    enum Origin {
        Existing(String),
        Added,
    }
    #[derive(Debug)]
    struct State {
        collection: MetaCollection,
        origin: Origin,
        current_name: Option<String>,
        touched: bool,
    }

    let mut states = Vec::<State>::new();
    let mut active = BTreeMap::<(String, String), usize>::new();
    let mut touched_collections = BTreeSet::<String>::new();
    for operation in operations {
        match operation {
            MetaEditOperation::Add {
                collection,
                elements,
                ..
            } if typed_physical_collection(*collection).is_some() => {
                touched_collections.insert(collection.as_str().to_string());
                for element in elements {
                    let index = states.len();
                    states.push(State {
                        collection: *collection,
                        origin: Origin::Added,
                        current_name: Some(element.name.clone()),
                        touched: true,
                    });
                    active.insert(
                        (collection.as_str().to_string(), element.name.clone()),
                        index,
                    );
                }
            }
            MetaEditOperation::Update {
                collection,
                elements,
                ..
            } if typed_physical_collection(*collection).is_some() => {
                touched_collections.insert(collection.as_str().to_string());
                for element in elements {
                    let key = (collection.as_str().to_string(), element.name.clone());
                    let index = active.remove(&key).unwrap_or_else(|| {
                        let index = states.len();
                        states.push(State {
                            collection: *collection,
                            origin: Origin::Existing(element.name.clone()),
                            current_name: Some(element.name.clone()),
                            touched: true,
                        });
                        index
                    });
                    let next_name = element
                        .new_name
                        .clone()
                        .unwrap_or_else(|| element.name.clone());
                    states[index].current_name = Some(next_name.clone());
                    states[index].touched = true;
                    active.insert((collection.as_str().to_string(), next_name), index);
                }
            }
            MetaEditOperation::Remove {
                collection, names, ..
            } if typed_physical_collection(*collection).is_some() => {
                touched_collections.insert(collection.as_str().to_string());
                for name in names {
                    let key = (collection.as_str().to_string(), name.clone());
                    let index = active.remove(&key).unwrap_or_else(|| {
                        let index = states.len();
                        states.push(State {
                            collection: *collection,
                            origin: Origin::Existing(name.clone()),
                            current_name: Some(name.clone()),
                            touched: true,
                        });
                        index
                    });
                    states[index].current_name = None;
                    states[index].touched = true;
                }
            }
            _ => {}
        }
    }

    let final_document =
        Document::parse(post_image.trim_start_matches('\u{feff}')).map_err(|_| {
            unica_metadata::request::MetaFailure::from(
                crate::edit::typed_diagnostic(
                    MetaDiagnosticCode::ProviderUnavailable,
                    "typed owner post-image is not valid XML",
                    None,
                )
                .with_metadata_path(owner.clone()),
            )
        })?;
    let final_object = crate::edit::meta_edit_object_node(&final_document).map_err(|_| {
        unica_metadata::request::MetaFailure::from(
            crate::edit::typed_diagnostic(
                MetaDiagnosticCode::ProviderUnavailable,
                "typed owner post-image object is unavailable",
                None,
            )
            .with_metadata_path(owner.clone()),
        )
    })?;
    if let Some(children) = unica_xml_format::read::child_element(final_object, "ChildObjects") {
        for (collection, tag) in [
            (MetaCollection::Forms, "Form"),
            (MetaCollection::Templates, "Template"),
            (MetaCollection::Commands, "Command"),
        ] {
            for child in unica_xml_format::read::child_elements(children, tag) {
                let Some(name) = crate::edit::typed_physical_owner_child_name(child) else {
                    continue;
                };
                if states.iter().any(|state| {
                    state.collection == collection
                        && state.current_name.as_deref() == Some(name.as_str())
                }) {
                    continue;
                }
                touched_collections.insert(collection.as_str().to_string());
                states.push(State {
                    collection,
                    origin: Origin::Existing(name.clone()),
                    current_name: Some(name),
                    touched: false,
                });
            }
        }
    }

    let object_dir = descriptor_path.with_extension("");
    let mut initial_files = BTreeMap::<PathBuf, Vec<u8>>::new();
    let mut final_files = BTreeMap::<PathBuf, Vec<u8>>::new();
    let mut initial_directories = BTreeSet::<PathBuf>::new();
    let mut final_directories = BTreeSet::<PathBuf>::new();
    let mut collection_snapshots = Vec::new();
    let mut child_topology_roots = Vec::<(PathBuf, MetadataAddress)>::new();
    let mut plan = TypedChildResourcePlan::default();

    for collection_name in touched_collections {
        let collection = states
            .iter()
            .find(|state| state.collection.as_str() == collection_name)
            .map(|state| state.collection)
            .expect("a touched physical collection has at least one state");
        let profile = typed_child_storage_profile(collection).unwrap();
        let collection_dir = object_dir.join(profile.directory);
        let snapshot = observe_membership(
            &collection_dir,
            DirectoryMembershipSelector::AllDirectEntries,
        )
        .map_err(|_| typed_child_collection_topology_failure(owner, collection))?;
        if matches!(snapshot, DirectoryMembershipSnapshot::Present(_)) {
            initial_directories.insert(collection_dir.clone());
        }
        collection_snapshots.push((collection, collection_dir, snapshot));
    }

    for state in &states {
        let profile = typed_child_storage_profile(state.collection).unwrap();
        let collection_dir = object_dir.join(profile.directory);
        let resource = profile.resource;
        let mut initial_descriptor = None;
        // The payload of an existing child is held once, in `initial_files`;
        // these are its paths relative to `initial_payload_root` (#1119).
        let mut initial_payload_root = None;
        let mut initial_payload = Vec::<PathBuf>::new();
        let mut initial_payload_directories = Vec::new();
        if let Origin::Existing(initial_name) = &state.origin {
            let initial_child = typed_child_logical_address(
                owner,
                object_kind,
                object_name,
                state.collection,
                initial_name,
            )?;
            if profile.descriptor == crate::edit::TypedChildDescriptorStorage::Standalone {
                let descriptor = collection_dir.join(format!("{initial_name}.xml"));
                let bytes = read_typed_child_file(reader, &descriptor, owner)?;
                initial_descriptor = Some(bytes.clone());
                initial_files.insert(descriptor, bytes);
            }
            let payload_root = collection_dir.join(initial_name);
            initial_payload_root = Some(payload_root.clone());
            child_topology_roots.push((payload_root.clone(), initial_child.clone()));
            if unica_source_designer::read::path_exists(reader, &payload_root) {
                let (files, directories) =
                    read_typed_child_tree(reader, &payload_root).map_err(|error| {
                        typed_child_topology_failure(&initial_child, error.public_message())
                    })?;
                for (relative, bytes) in files {
                    initial_files.insert(payload_root.join(&relative), bytes);
                    initial_payload.push(relative);
                }
                for relative in directories {
                    let path = payload_root.join(&relative);
                    initial_directories.insert(path);
                    initial_payload_directories.push(relative);
                }
            }
        }

        let final_descriptor = if let Some(final_name) = &state.current_name {
            let tag = crate::edit::collection_tag(state.collection);
            let bytes = typed_child_element_xml(post_image, tag, final_name)
                .and_then(|child_xml| {
                    if matches!(&state.origin, Origin::Added)
                        && state.collection == MetaCollection::Templates
                    {
                        return new_template_descriptor_image(&child_xml);
                    }
                    Ok(initial_descriptor
                        .as_deref()
                        .and_then(|initial| splice_standalone_descriptor(initial, tag, &child_xml))
                        .unwrap_or_else(|| wrap_standalone_descriptor(&child_xml)))
                })
                .or_else(|failure| match &state.origin {
                    Origin::Existing(initial_name) if initial_name == final_name => {
                        initial_descriptor.clone().ok_or(failure)
                    }
                    Origin::Existing(_) | Origin::Added => Err(failure),
                })?;
            if profile.descriptor == crate::edit::TypedChildDescriptorStorage::Standalone {
                final_files.insert(
                    collection_dir.join(format!("{final_name}.xml")),
                    bytes.clone(),
                );
            }
            plan.validation_resources.push(
                unica_metadata::validation_subject::MetadataResourceImage {
                    role: typed_child_role(state.collection, owner, final_name),
                    bytes: bytes.clone(),
                },
            );
            let child_address = typed_child_logical_address(
                owner,
                object_kind,
                object_name,
                state.collection,
                final_name,
            )?;
            child_topology_roots.push((collection_dir.join(final_name), child_address.clone()));
            let template_type = (state.collection == MetaCollection::Templates)
                .then(|| typed_template_type_from_descriptor(&bytes, &child_address))
                .transpose()?;
            let mut final_payload = Vec::<(PathBuf, Vec<u8>)>::new();
            let mut final_payload_directories = Vec::<PathBuf>::new();
            match &state.origin {
                Origin::Existing(_) => {
                    if let Some(root) = &initial_payload_root {
                        final_payload = initial_payload
                            .iter()
                            .map(|relative| {
                                (
                                    relative.clone(),
                                    initial_files[&root.join(relative)].clone(),
                                )
                            })
                            .collect();
                    }
                    final_payload_directories.clone_from(&initial_payload_directories);
                }
                Origin::Added => match state.collection {
                    MetaCollection::Forms => {
                        final_payload_directories.push(PathBuf::new());
                        final_payload_directories.push(PathBuf::from("Ext"));
                        let content = (form_seed)(object_kind, object_name).into_bytes();
                        final_payload.push((PathBuf::from("Ext/Form.xml"), content));
                    }
                    MetaCollection::Templates => {
                        final_payload_directories.push(PathBuf::new());
                        final_payload_directories.push(PathBuf::from("Ext"));
                        let template_type = template_type
                            .expect("a planned template child publishes its TemplateType");
                        final_payload.push((
                            PathBuf::from(typed_template_primary_path(template_type)),
                            typed_template_initial_content(templates, template_type),
                        ));
                        if template_type == unica_metadata::validation_subject::MetadataTemplateType::HtmlDocument {
                            final_payload_directories.push(PathBuf::from("Ext/Template"));
                            final_payload.push((
                                typed_template_page_path("ru"),
                                (templates.html_page)()
                                    .as_bytes()
                                    .to_vec(),
                            ));
                        }
                    }
                    MetaCollection::Commands => {}
                    _ => unreachable!(),
                },
            }
            let footprint = validate_typed_child_footprint(
                state.collection,
                template_type,
                &child_address,
                &final_payload,
                &final_payload_directories,
            )?;
            let retained = footprint.retained.clone();
            plan.validation_footprints.push(footprint);
            let payload_root = collection_dir.join(final_name);
            for relative in &final_payload_directories {
                final_directories.insert(payload_root.join(relative));
            }
            for (relative, bytes) in final_payload {
                plan.validation_resources.push(
                    unica_metadata::validation_subject::MetadataResourceImage {
                        role: typed_child_payload_role(
                            state.collection,
                            &child_address,
                            &relative,
                            template_type,
                            &retained,
                        )?,
                        bytes: bytes.clone(),
                    },
                );
                final_files.insert(payload_root.join(relative), bytes);
            }
            Some(bytes)
        } else {
            None
        };

        match (&state.origin, &state.current_name) {
            (Origin::Existing(initial_name), None) => {
                let replaced_at_same_name = states.iter().any(|candidate| {
                    candidate.collection == state.collection
                        && candidate.current_name.as_deref() == Some(initial_name.as_str())
                });
                if !replaced_at_same_name {
                    plan.publication_plan.push(MetaPublicationPlanEntry {
                        action: MetaPublicationAction::Remove,
                        resource,
                        metadata_path: Some(typed_child_logical_address(
                            owner,
                            object_kind,
                            object_name,
                            state.collection,
                            initial_name,
                        )?),
                    });
                }
            }
            (Origin::Existing(initial_name), Some(final_name)) => {
                if state.touched
                    && (initial_name != final_name
                        || profile.descriptor
                            == crate::edit::TypedChildDescriptorStorage::InlineOwner
                        || initial_descriptor.as_ref() != final_descriptor.as_ref())
                {
                    plan.publication_plan.push(MetaPublicationPlanEntry {
                        action: MetaPublicationAction::Update,
                        resource,
                        metadata_path: Some(typed_child_logical_address(
                            owner,
                            object_kind,
                            object_name,
                            state.collection,
                            final_name,
                        )?),
                    });
                }
            }
            (Origin::Added, Some(final_name)) => {
                let replaces_initial = states.iter().any(|candidate| {
                    candidate.collection == state.collection
                        && matches!(
                            &candidate.origin,
                            Origin::Existing(initial_name) if initial_name == final_name
                        )
                });
                plan.publication_plan.push(MetaPublicationPlanEntry {
                    action: if replaces_initial {
                        MetaPublicationAction::Update
                    } else {
                        MetaPublicationAction::Create
                    },
                    resource,
                    metadata_path: Some(typed_child_logical_address(
                        owner,
                        object_kind,
                        object_name,
                        state.collection,
                        final_name,
                    )?),
                });
            }
            (Origin::Added, None) => {}
        }
    }

    for (collection, collection_dir, snapshot) in collection_snapshots {
        let collection_existed = matches!(snapshot, DirectoryMembershipSnapshot::Present(_));
        let touched_initial_entries = states
            .iter()
            .filter(|state| state.collection == collection)
            .filter_map(|state| match &state.origin {
                Origin::Existing(name) => Some(name),
                Origin::Added => None,
            })
            .flat_map(|name| [name.clone(), format!("{name}.xml")])
            .collect::<BTreeSet<_>>();
        let has_unrelated_initial_entry = match snapshot {
            DirectoryMembershipSnapshot::Absent => false,
            DirectoryMembershipSnapshot::Present(entries) => entries.iter().any(|entry| {
                !touched_initial_entries.contains(&entry.name.to_string_lossy().into_owned())
            }),
        };
        let has_final_child_footprint = final_files
            .keys()
            .any(|path| path.starts_with(&collection_dir))
            || final_directories
                .iter()
                .any(|path| path.starts_with(&collection_dir));
        let removed_existing_child = states.iter().any(|state| {
            state.collection == collection
                && matches!(state.origin, Origin::Existing(_))
                && state.current_name.is_none()
        });
        if has_unrelated_initial_entry
            || has_final_child_footprint
            || (collection_existed && !removed_existing_child)
        {
            final_directories.insert(collection_dir);
        }
    }

    let removed_directories = initial_directories
        .difference(&final_directories)
        .filter(|candidate| {
            !initial_directories
                .difference(&final_directories)
                .any(|parent| parent != *candidate && candidate.starts_with(parent))
        })
        .cloned()
        .collect::<Vec<_>>();
    for directory in &removed_directories {
        plan.file_mutations.push(TypedChildFileMutation {
            path: directory.clone(),
            pre_image: Some(Vec::new()),
            post_image: None,
        });
    }
    let all_file_paths = initial_files
        .keys()
        .chain(final_files.keys())
        .cloned()
        .collect::<BTreeSet<_>>();
    for path in &all_file_paths {
        if plan
            .file_mutations
            .iter()
            .any(|mutation| mutation.post_image.is_none() && path.starts_with(&mutation.path))
        {
            continue;
        }
        match (initial_files.get(path), final_files.get(path)) {
            (Some(before), Some(after)) if before == after => {
                plan.exact_file_guards.push((path.clone(), before.clone()));
            }
            (Some(before), Some(after)) => plan.file_mutations.push(TypedChildFileMutation {
                path: path.clone(),
                pre_image: Some(before.clone()),
                post_image: Some(after.clone()),
            }),
            (Some(before), None) => plan.file_mutations.push(TypedChildFileMutation {
                path: path.clone(),
                pre_image: Some(before.clone()),
                post_image: None,
            }),
            (None, Some(after)) => plan.file_mutations.push(TypedChildFileMutation {
                path: path.clone(),
                pre_image: None,
                post_image: Some(after.clone()),
            }),
            (None, None) => unreachable!(),
        }
    }

    let guarded_directories = initial_directories
        .union(&final_directories)
        .filter(|directory| {
            !removed_directories
                .iter()
                .any(|removed| directory.starts_with(removed))
        })
        .cloned()
        .collect::<Vec<_>>();
    for directory in guarded_directories {
        let snapshot =
            observe_membership(&directory, DirectoryMembershipSelector::AllDirectEntries).map_err(
                |_| {
                    child_topology_roots
                        .iter()
                        .filter(|(root, _)| directory.starts_with(root))
                        .max_by_key(|(root, _)| root.components().count())
                        .map(|(_, child)| {
                            typed_child_topology_failure(
                                child,
                                "typed child resource topology is unavailable",
                            )
                        })
                        .unwrap_or_else(|| typed_child_collection_guard_failure(owner))
                },
            )?;
        plan.directory_guards.push((
            directory,
            DirectoryMembershipSelector::AllDirectEntries,
            snapshot,
        ));
    }
    Ok(plan)
}

fn typed_physical_collection(
    collection: MetaCollection,
) -> Option<(&'static str, MetaPublicationResource)> {
    typed_child_storage_profile(collection).map(|profile| (profile.directory, profile.resource))
}

pub fn observe_typed_child_resources(
    reader: &dyn ProjectDiscoveryRead,
    descriptor_path: &Path,
    owner: &MetadataAddress,
    object_kind: &str,
    object_name: &str,
    owner_xml: &str,
) -> Result<TypedChildObservation, unica_metadata::request::MetaFailure> {
    let document = Document::parse(owner_xml.trim_start_matches('\u{feff}')).map_err(|_| {
        unica_metadata::request::MetaFailure::from(
            crate::edit::typed_diagnostic(
                MetaDiagnosticCode::ProviderUnavailable,
                "typed owner observation image is not valid XML",
                None,
            )
            .with_metadata_path(owner.clone()),
        )
    })?;
    let object = crate::edit::meta_edit_object_node(&document).map_err(|_| {
        unica_metadata::request::MetaFailure::from(
            crate::edit::typed_diagnostic(
                MetaDiagnosticCode::ProviderUnavailable,
                "typed owner observation object is unavailable",
                None,
            )
            .with_metadata_path(owner.clone()),
        )
    })?;
    let Some(children) = unica_xml_format::read::child_element(object, "ChildObjects") else {
        return Ok(TypedChildObservation::default());
    };

    let object_dir = descriptor_path.with_extension("");
    let mut observation = TypedChildObservation::default();
    for (collection, tag) in [
        (MetaCollection::Forms, "Form"),
        (MetaCollection::Templates, "Template"),
        (MetaCollection::Commands, "Command"),
    ] {
        let profile = typed_child_storage_profile(collection)
            .expect("closed observed child collection has a storage profile");
        let collection_dir = object_dir.join(profile.directory);
        for child_node in unica_xml_format::read::child_elements(children, tag) {
            let Some(name) = crate::edit::typed_physical_owner_child_name(child_node) else {
                continue;
            };
            let child = match typed_child_logical_address(
                owner,
                object_kind,
                object_name,
                collection,
                &name,
            ) {
                Ok(child) => child,
                Err(failure) => {
                    observation.diagnostics.extend(failure.diagnostics);
                    continue;
                }
            };
            let descriptor = match profile.descriptor {
                crate::edit::TypedChildDescriptorStorage::Standalone => {
                    let path = collection_dir.join(format!("{name}.xml"));
                    match unica_source_designer::read::read_bytes(reader, &path) {
                        Ok(bytes) => bytes,
                        Err(_) => {
                            observation.diagnostics.push(
                                crate::edit::typed_diagnostic(
                                    MetaDiagnosticCode::ProviderUnavailable,
                                    format!("typed child descriptor is unavailable: {child}"),
                                    Some("resources.child.descriptor"),
                                )
                                .with_metadata_path(child.clone()),
                            );
                            continue;
                        }
                    }
                }
                crate::edit::TypedChildDescriptorStorage::InlineOwner => {
                    match typed_child_descriptor_image(owner_xml, tag, &name) {
                        Ok(bytes) => bytes,
                        Err(failure) => {
                            observation.diagnostics.extend(failure.diagnostics);
                            continue;
                        }
                    }
                }
            };

            observation.validation_resources.push(
                unica_metadata::validation_subject::MetadataResourceImage {
                    role: typed_child_role(collection, owner, &name),
                    bytes: descriptor.clone(),
                },
            );
            let template_type = (collection == MetaCollection::Templates)
                .then(|| typed_template_type_from_descriptor(&descriptor, &child))
                .transpose();
            let template_type = match template_type {
                Ok(value) => value,
                Err(failure) => {
                    observation.diagnostics.extend(failure.diagnostics);
                    continue;
                }
            };

            let payload_root = collection_dir.join(&name);
            let (payload, directories) =
                if unica_source_designer::read::path_exists(reader, &payload_root) {
                    match read_typed_child_tree(reader, &payload_root) {
                        Ok(value) => value,
                        Err(error) => {
                            observation.diagnostics.push(
                                crate::edit::typed_diagnostic(
                                    MetaDiagnosticCode::ProviderUnavailable,
                                    format!(
                                        "typed child resource topology is unavailable: {child}: {}",
                                        error.public_message()
                                    ),
                                    Some("resources.child.topology"),
                                )
                                .with_metadata_path(child.clone()),
                            );
                            continue;
                        }
                    }
                } else {
                    (Vec::new(), Vec::new())
                };
            // Заимствованного ребёнка платформа выгружает без своего
            // содержимого (`Form.xml`, `Template.xml`), если расширение его не
            // меняет. Наблюдение не требует у него файлов; доказать
            // заимствование владельца и ребёнка обязан валидатор.
            let inherited = payload.is_empty()
                && directories.is_empty()
                && crate::xml_model::descriptor_extends_parent(&descriptor);
            // Макет двоичных данных без содержимого платформа выгружает одним
            // описанием, без `Ext/Template.bin`: так его загружает и выгружает
            // ibcmd 8.3.27 (#2027). Это макет без данных, а не потерянный файл.
            let empty_binary_template = payload.is_empty()
                && directories.is_empty()
                && template_type
                    == Some(unica_metadata::validation_subject::MetadataTemplateType::BinaryData);
            let footprint_profile = (|| {
                let mut profile = typed_child_footprint_profile(collection, template_type, &child)?;
                if inherited || empty_binary_template {
                    profile.required_files.clear();
                }
                if collection == MetaCollection::Forms {
                    let parsed = crate::validation::parse_child_profile_from_bytes(
                        &descriptor,
                        owner,
                        crate::validation::MetadataAdmission::Read,
                    )
                    .map_err(|message| typed_child_resource_failure(&child, &message))?;
                    let Some((parsed_child, parsed_profile)) = parsed else {
                        return Err(typed_child_resource_failure(
                            &child,
                            "form descriptor has a different child kind",
                        ));
                    };
                    if parsed_child != child {
                        return Err(typed_child_resource_failure(
                            &child,
                            "form descriptor names another child",
                        ));
                    }
                    // Observation accepts opaque ordinary forms. Mutation planning
                    // still uses its managed-only footprint profile.
                    if parsed_profile
                        == unica_metadata::validation_subject::MetadataChildProfile::OrdinaryForm
                    {
                        profile = TypedChildFootprintProfile {
                            logical_profile: parsed_profile,
                            required_files: typed_child_modelled_payload(
                                parsed_profile,
                                false,
                                &[],
                            ),
                            optional_files: BTreeSet::new(),
                        };
                    }
                }
                Ok(profile)
            })();
            let footprint = match footprint_profile.and_then(|profile| {
                validate_child_footprint_profile(profile, &child, &payload, &directories)
            }) {
                Ok(footprint) => footprint,
                Err(failure) => {
                    observation.diagnostics.extend(failure.diagnostics);
                    continue;
                }
            };
            let retained = footprint.retained.clone();
            observation.validation_footprints.push(footprint);
            for (relative, bytes) in payload {
                let role = match typed_child_payload_role(
                    collection,
                    &child,
                    &relative,
                    template_type,
                    &retained,
                ) {
                    Ok(role) => role,
                    Err(failure) => {
                        observation.diagnostics.extend(failure.diagnostics);
                        continue;
                    }
                };
                observation.validation_resources.push(
                    unica_metadata::validation_subject::MetadataResourceImage { role, bytes },
                );
            }
        }
    }
    Ok(observation)
}

/// New XML payloads have the native BOM and structural profile. Existing
/// payloads are retained separately; text and binary initial payloads are empty.
pub fn typed_template_initial_content(
    templates: TemplateSeedEmitters,
    template_type: unica_metadata::validation_subject::MetadataTemplateType,
) -> Vec<u8> {
    let xml = match template_type {
        unica_metadata::validation_subject::MetadataTemplateType::SpreadsheetDocument => {
            (templates.spreadsheet.0)()
        }
        // Платформенная форма HTMLDocument: primary — дескриптор страниц
        // `<Help>` (extrnprops), сами страницы лежат в Ext/Template/*.html.
        // Снимаемый template.add писал сюда сырой HTML — typed-канал пишет
        // форму, которую закрепил валидатор по реальным дампам 8.3.27.
        unica_metadata::validation_subject::MetadataTemplateType::HtmlDocument => (templates.html_descriptor)(
            unica_source_designer::format_profile::ACTIVE_FORMAT_PROFILE.export_format,
        ),
        unica_metadata::validation_subject::MetadataTemplateType::DataCompositionSchema => {
            (templates.content_xml)("DataCompositionSchema", "")
                .expect("closed template kind")
        }
        unica_metadata::validation_subject::MetadataTemplateType::TextDocument
        | unica_metadata::validation_subject::MetadataTemplateType::BinaryData
        | unica_metadata::validation_subject::MetadataTemplateType::AddIn
        | unica_metadata::validation_subject::MetadataTemplateType::DataCompositionAppearanceTemplate => return Vec::new(),
    };
    let mut bytes = b"\xef\xbb\xbf".to_vec();
    bytes.extend_from_slice(xml.as_bytes());
    bytes
}

fn typed_child_role(
    collection: MetaCollection,
    owner: &MetadataAddress,
    name: &str,
) -> unica_metadata::validation_subject::MetadataResourceRole {
    match collection {
        MetaCollection::Forms => unica_metadata::validation_subject::MetadataResourceRole::Form {
            owner: owner.clone(),
            name: name.to_string(),
        },
        MetaCollection::Templates => {
            unica_metadata::validation_subject::MetadataResourceRole::Template {
                owner: owner.clone(),
                name: name.to_string(),
            }
        }
        MetaCollection::Commands => {
            unica_metadata::validation_subject::MetadataResourceRole::Command {
                owner: owner.clone(),
                name: name.to_string(),
            }
        }
        _ => unreachable!(),
    }
}

fn typed_child_payload_role(
    collection: MetaCollection,
    child: &MetadataAddress,
    relative_path: &Path,
    template_type: Option<unica_metadata::validation_subject::MetadataTemplateType>,
    retained: &[PathBuf],
) -> Result<
    unica_metadata::validation_subject::MetadataResourceRole,
    unica_metadata::request::MetaFailure,
> {
    if let Some(ordinal) = retained
        .iter()
        .position(|candidate| candidate == relative_path)
    {
        return Ok(
            unica_metadata::validation_subject::MetadataResourceRole::ChildResource {
                child: child.clone(),
                kind: unica_metadata::validation_subject::MetadataChildResourceKind::Retained,
                ordinal,
            },
        );
    }
    let kind = match collection {
        MetaCollection::Forms if relative_path == Path::new(TYPED_ORDINARY_FORM_CONTENT_PATH) => {
            unica_metadata::validation_subject::MetadataChildResourceKind::OrdinaryFormContent
        }
        MetaCollection::Forms if relative_path == Path::new("Ext/Form.xml") => {
            unica_metadata::validation_subject::MetadataChildResourceKind::FormContent
        }
        MetaCollection::Forms if relative_path == Path::new(TYPED_FORM_MODULE_PATH) => {
            unica_metadata::validation_subject::MetadataChildResourceKind::Module
        }
        MetaCollection::Commands if relative_path == Path::new("Ext/CommandModule.bsl") => {
            unica_metadata::validation_subject::MetadataChildResourceKind::Module
        }
        MetaCollection::Templates => {
            let template_type = template_type.ok_or_else(|| {
                typed_child_resource_failure(
                    child,
                    "template payload has no closed TemplateType evidence",
                )
            })?;
            let part = match (template_type, relative_path) {
                (
                    unica_metadata::validation_subject::MetadataTemplateType::SpreadsheetDocument
                    | unica_metadata::validation_subject::MetadataTemplateType::DataCompositionSchema,
                    path,
                ) if path == Path::new("Ext/Template.xml") => unica_metadata::validation_subject::MetadataTemplateResourcePart::Primary,
                (unica_metadata::validation_subject::MetadataTemplateType::TextDocument, path)
                    if path == Path::new("Ext/Template.txt") =>
                {
                    unica_metadata::validation_subject::MetadataTemplateResourcePart::Primary
                }
                (unica_metadata::validation_subject::MetadataTemplateType::BinaryData | unica_metadata::validation_subject::MetadataTemplateType::AddIn, path)
                    if path == Path::new("Ext/Template.bin") =>
                {
                    unica_metadata::validation_subject::MetadataTemplateResourcePart::Primary
                }
                (unica_metadata::validation_subject::MetadataTemplateType::DataCompositionAppearanceTemplate, path)
                    if path == Path::new("Ext/Template.xml") =>
                {
                    unica_metadata::validation_subject::MetadataTemplateResourcePart::Primary
                }
                (unica_metadata::validation_subject::MetadataTemplateType::HtmlDocument, path)
                    if path == Path::new("Ext/Template.xml") =>
                {
                    unica_metadata::validation_subject::MetadataTemplateResourcePart::Primary
                }
                (unica_metadata::validation_subject::MetadataTemplateType::HtmlDocument, path)
                    if path.parent() == Some(Path::new("Ext/Template"))
                        && path.extension().and_then(|extension| extension.to_str())
                            == Some("html") =>
                {
                    unica_metadata::validation_subject::MetadataTemplateResourcePart::HtmlPage
                }
                _ => {
                    return Err(typed_child_resource_failure(
                        child,
                        "template payload footprint does not match its TemplateType",
                    ))
                }
            };
            unica_metadata::validation_subject::MetadataChildResourceKind::TemplateContent {
                template_type,
                part,
            }
        }
        _ => {
            return Err(typed_child_resource_failure(
                child,
                "child payload contains an unsupported resource role",
            ))
        }
    };
    let ordinal = match (collection, kind) {
        (
            MetaCollection::Forms,
            unica_metadata::validation_subject::MetadataChildResourceKind::FormContent
            | unica_metadata::validation_subject::MetadataChildResourceKind::OrdinaryFormContent,
        ) => 0,
        (
            MetaCollection::Forms,
            unica_metadata::validation_subject::MetadataChildResourceKind::Module,
        ) => 1,
        (
            MetaCollection::Commands,
            unica_metadata::validation_subject::MetadataChildResourceKind::Module,
        ) => 0,
        (
            MetaCollection::Templates,
            unica_metadata::validation_subject::MetadataChildResourceKind::TemplateContent {
                part: unica_metadata::validation_subject::MetadataTemplateResourcePart::Primary,
                ..
            },
        ) => 0,
        (
            MetaCollection::Templates,
            unica_metadata::validation_subject::MetadataChildResourceKind::TemplateContent {
                part: unica_metadata::validation_subject::MetadataTemplateResourcePart::HtmlPage,
                ..
            },
        ) => 1,
        _ => unreachable!("closed child resource kind is owned by its collection"),
    };
    Ok(
        unica_metadata::validation_subject::MetadataResourceRole::ChildResource {
            child: child.clone(),
            kind,
            ordinal,
        },
    )
}

fn typed_template_type_from_descriptor(
    descriptor: &[u8],
    child: &MetadataAddress,
) -> Result<
    unica_metadata::validation_subject::MetadataTemplateType,
    unica_metadata::request::MetaFailure,
> {
    let text = std::str::from_utf8(descriptor)
        .map_err(|_| typed_child_resource_failure(child, "template descriptor is not UTF-8"))?;
    let document = Document::parse(text.trim_start_matches('\u{feff}'))
        .map_err(|_| typed_child_resource_failure(child, "template descriptor is malformed XML"))?;
    let values = document
        .descendants()
        .filter(|node| node.is_element() && node.tag_name().name() == "TemplateType")
        .filter_map(|node| node.text())
        .map(str::trim)
        .collect::<Vec<_>>();
    let [value] = values.as_slice() else {
        return Err(typed_child_resource_failure(
            child,
            "template descriptor must contain exactly one TemplateType",
        ));
    };
    unica_metadata::validation_subject::MetadataTemplateType::from_descriptor_value(value)
        .ok_or_else(|| {
            typed_child_resource_failure(
                child,
                "template descriptor uses an unsupported TemplateType",
            )
        })
}

fn typed_child_collection_topology_failure(
    owner: &MetadataAddress,
    collection: MetaCollection,
) -> unica_metadata::request::MetaFailure {
    crate::edit::typed_diagnostic(
        MetaDiagnosticCode::ProviderUnavailable,
        format!(
            "typed child collection topology is unavailable: {}.{}",
            owner,
            collection.as_str()
        ),
        Some("resources.child.collection"),
    )
    .with_metadata_path(owner.clone())
    .into()
}

fn typed_child_collection_guard_failure(
    owner: &MetadataAddress,
) -> unica_metadata::request::MetaFailure {
    crate::edit::typed_diagnostic(
        MetaDiagnosticCode::ProviderUnavailable,
        format!("typed child collection topology is unavailable: {owner}"),
        Some("resources.child.collection"),
    )
    .with_metadata_path(owner.clone())
    .into()
}

fn typed_child_logical_address(
    owner: &MetadataAddress,
    object_kind: &str,
    object_name: &str,
    collection: MetaCollection,
    name: &str,
) -> Result<MetadataAddress, unica_metadata::request::MetaFailure> {
    let child_kind = match collection {
        MetaCollection::Forms => "Form",
        MetaCollection::Templates => "Template",
        MetaCollection::Commands => "Command",
        MetaCollection::PredefinedItems => {
            unreachable!("predefined items are not descriptor child objects")
        }
        _ => unreachable!(),
    };
    MetadataAddress::parse(
        PLATFORM_XML_8_3_27_FORMAT_2_20,
        &format!("{object_kind}.{object_name}.{child_kind}.{name}"),
    )
    .map_err(|_| {
        unica_metadata::request::MetaFailure::from(
            crate::edit::typed_diagnostic(
                MetaDiagnosticCode::ProviderUnavailable,
                "typed child logical address cannot be represented",
                Some("collection"),
            )
            .with_metadata_path(owner.clone()),
        )
    })
}

fn read_typed_child_file(
    reader: &dyn ProjectDiscoveryRead,
    path: &Path,
    owner: &MetadataAddress,
) -> Result<Vec<u8>, unica_metadata::request::MetaFailure> {
    unica_source_designer::read::read_bytes(reader, path).map_err(|_| {
        unica_metadata::request::MetaFailure::from(
            crate::edit::typed_diagnostic(
                MetaDiagnosticCode::ProviderUnavailable,
                "typed child descriptor pre-image is unavailable",
                None,
            )
            .with_metadata_path(owner.clone()),
        )
    })
}

pub fn read_typed_child_tree(
    reader: &dyn ProjectDiscoveryRead,
    root: &Path,
) -> Result<TypedChildTree, TypedChildTreeError> {
    fn visit(
        reader: &dyn ProjectDiscoveryRead,
        root: &Path,
        current: &Path,
        depth: usize,
        files: &mut Vec<(PathBuf, Vec<u8>)>,
        directories: &mut Vec<PathBuf>,
    ) -> Result<(), TypedChildTreeError> {
        if depth > TYPED_CHILD_TREE_MAX_DEPTH {
            return Err(TypedChildTreeError::Unavailable);
        }
        let entries = reader
            .read_entries(current)
            .map_err(|_| TypedChildTreeError::Unavailable)?;
        for entry in entries {
            let entry = entry.map_err(|_| TypedChildTreeError::Unavailable)?;
            let file_type = entry
                .file_type()
                .map_err(|_| TypedChildTreeError::Unavailable)?;
            if file_type.is_symlink() {
                return Err(TypedChildTreeError::SymbolicLink);
            }
            if file_type.is_dir() {
                directories.push(entry.path().strip_prefix(root).unwrap().to_path_buf());
                visit(reader, root, &entry.path(), depth + 1, files, directories)?;
            } else if file_type.is_file() {
                let path = entry.path();
                let mut bytes = Vec::new();
                reader
                    .open_input(&path)
                    .map_err(|_| TypedChildTreeError::Unavailable)?
                    .read_to_end(&mut bytes)
                    .map_err(|_| TypedChildTreeError::Unavailable)?;
                files.push((path.strip_prefix(root).unwrap().to_path_buf(), bytes));
            } else {
                return Err(TypedChildTreeError::UnsupportedNode);
            }
        }
        Ok(())
    }
    let mut files = Vec::new();
    let mut directories = vec![PathBuf::new()];
    visit(reader, root, root, 0, &mut files, &mut directories)?;
    files.sort_by(|left, right| left.0.cmp(&right.0));
    directories.sort();
    Ok((files, directories))
}

/// Format only the structural gaps of a newly added Template, after all
/// operations have supplied its final fields. Leaf markup is copied verbatim,
/// including literal LF/TAB in comments and multilingual text.
fn new_template_descriptor_image(
    child_xml: &str,
) -> Result<Vec<u8>, unica_metadata::request::MetaFailure> {
    /// Append one template subtree, preserving leaf markup and validating structural gaps.
    fn append(
        source: &str,
        node: roxmltree::Node<'_, '_>,
        depth: usize,
        output: &mut String,
    ) -> Option<()> {
        const MD: &str = "http://v8.1c.ru/8.3/MDClasses";
        const V8: &str = "http://v8.1c.ru/8.1/data/core";
        output.push_str(&"\t".repeat(depth));
        let structural = node.has_tag_name((MD, "Template"))
            || node.has_tag_name((MD, "Properties"))
            || node.has_tag_name((MD, "Synonym"))
            || node.has_tag_name((V8, "item"));
        let children = node
            .children()
            .filter(|child| child.is_element())
            .collect::<Vec<_>>();
        if !structural || children.is_empty() {
            output.push_str(&source[node.range()]);
            return Some(());
        }
        // Only emitter-owned whitespace may be replaced. Authored leaf text
        // never takes this branch, and unexpected markup is not discarded.
        if node.children().any(|child| {
            !(child.is_element()
                || (child.is_text()
                    && child.text().is_some_and(|text| {
                        text.bytes()
                            .all(|byte| matches!(byte, b' ' | b'\t' | b'\r' | b'\n'))
                    })))
        }) {
            return None;
        }
        let first = children.first()?.range().start;
        let last = children.last()?.range().end;
        output
            .push_str(source[node.range().start..first].trim_end_matches([' ', '\t', '\r', '\n']));
        for child in children {
            output.push_str("\r\n");
            append(source, child, depth + 1, output)?;
        }
        output.push_str("\r\n");
        output.push_str(&"\t".repeat(depth));
        output.push_str(source[last..node.range().end].trim_start_matches([' ', '\t', '\r', '\n']));
        Some(())
    }

    let invalid = || {
        unica_metadata::request::MetaFailure::from(crate::edit::typed_diagnostic(
            MetaDiagnosticCode::ProviderUnavailable,
            "new template descriptor has unexpected structural content",
            Some("elements"),
        ))
    };
    let wrapped = wrap_standalone_descriptor(child_xml);
    let source = std::str::from_utf8(&wrapped).map_err(|_| invalid())?;
    let document = Document::parse(source).map_err(|_| invalid())?;
    let template = document
        .root_element()
        .children()
        .find(|node| node.has_tag_name(("http://v8.1c.ru/8.3/MDClasses", "Template")))
        .ok_or_else(invalid)?;
    let mut output = format!(
        "\u{feff}<?xml version=\"1.0\" encoding=\"UTF-8\"?>\r\n<MetaDataObject {} version=\"2.20\">\r\n",
        crate::template_catalog::meta_xmlns_decl()
    );
    append(source, template, 1, &mut output).ok_or_else(invalid)?;
    output.push_str("\r\n</MetaDataObject>");
    Ok(output.into_bytes())
}

/// Replace the child element of an existing standalone descriptor and keep
/// its preamble, root declarations and trailer byte-for-byte.
fn splice_standalone_descriptor(initial: &[u8], tag: &str, child_xml: &str) -> Option<Vec<u8>> {
    let text = std::str::from_utf8(initial).ok()?;
    let body_start = if text.starts_with('\u{feff}') { 3 } else { 0 };
    let body = &text[body_start..];
    let document = Document::parse(body).ok()?;
    let child = document
        .root_element()
        .children()
        .find(|node| node.is_element() && node.tag_name().name() == tag)?;
    let range = child.range();
    let mut spliced = String::with_capacity(text.len() + child_xml.len());
    spliced.push_str(&text[..body_start + range.start]);
    spliced.push_str(child_xml);
    spliced.push_str(&body[range.end..]);
    Some(spliced.into_bytes())
}

#[derive(Debug, Clone, Copy)]
pub enum TypedChildTreeError {
    Unavailable,
    SymbolicLink,
    UnsupportedNode,
}

impl TypedChildTreeError {
    fn public_message(self) -> &'static str {
        match self {
            Self::Unavailable => "typed child resource topology is unavailable",
            Self::SymbolicLink => "typed child resource topology contains a symbolic link",
            Self::UnsupportedNode => {
                "typed child resource topology contains an unsupported filesystem node"
            }
        }
    }
}

pub type TypedChildTree = (Vec<(PathBuf, Vec<u8>)>, Vec<PathBuf>);

const TYPED_CHILD_TREE_MAX_DEPTH: usize = 64;
