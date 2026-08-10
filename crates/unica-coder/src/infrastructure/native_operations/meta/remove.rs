use crate::application::metadata::{MetaFailure, MetaRemoveRequest};
use crate::application::ports::{
    MetadataResourceImage, MetadataResourceRole, MetadataValidationSubject,
};
use crate::application::source_navigation::SourceLocateRequest;
use crate::application::SupportGuardRequirement;
use crate::domain::cancellation::CancellationToken;
use crate::domain::metadata::{
    MetaDiagnostic, MetaDiagnosticCode, MetaMutationData, MetaMutationEffect,
    MetaPublicationAction, MetaPublicationPlanEntry, MetaPublicationResource, MetaValidationData,
    MetaValidationStatus, MetadataKind,
};
use crate::domain::source_target::{MetadataAddress, PLATFORM_XML_8_3_27_FORMAT_2_20};
use crate::domain::workspace::WorkspaceContext;
use crate::infrastructure::metadata_kinds::metadata_layout;
use crate::infrastructure::native_operations::common::Utf8TextSnapshot;
use crate::infrastructure::platform::secure_read::read_root_relative_regular_file;
use crate::infrastructure::platform_xml_source_targets::locate_platform_xml_source_path;
use crate::infrastructure::support_guard::{
    bind_resolved_support_guard_evidence, evaluate_resolved_support_guard,
    ResolvedSupportGuardCheck,
};
use roxmltree::Document;
use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::ffi::OsString;
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use super::super::common::{
    file_stem_string, first_tag_text_in_xml, guard_active_format_dependencies_and_xml_trees,
    read_utf8_sig_snapshot, relative_display,
};
use super::super::compile_transaction::{
    CompileTransaction, DirectoryMembershipSnapshot, DirectoryTopologyEntry,
    DirectoryTopologyEntryKind,
};
use super::super::form::form_is_xml_ncname;
use super::super::role::role_info_element;
use super::super::subsystem::subsystem_validation_format_dependency_paths;
use super::edit::ResolvedMetadataObject;

#[cfg(test)]
thread_local! {
    static META_REMOVE_BEFORE_REAUTHORIZATION_HOOK: std::cell::RefCell<Option<Box<dyn FnOnce()>>> =
        const { std::cell::RefCell::new(None) };
}

#[cfg(test)]
pub(crate) fn with_meta_remove_before_reauthorization_hook<T>(
    hook: impl FnOnce() + 'static,
    action: impl FnOnce() -> T,
) -> T {
    struct Reset;
    impl Drop for Reset {
        fn drop(&mut self) {
            META_REMOVE_BEFORE_REAUTHORIZATION_HOOK.with(|slot| {
                slot.borrow_mut().take();
            });
        }
    }
    META_REMOVE_BEFORE_REAUTHORIZATION_HOOK.with(|slot| {
        assert!(slot.borrow_mut().replace(Box::new(hook)).is_none());
    });
    let _reset = Reset;
    action()
}

#[cfg(test)]
fn run_meta_remove_before_reauthorization_hook() {
    META_REMOVE_BEFORE_REAUTHORIZATION_HOOK.with(|slot| {
        if let Some(hook) = slot.borrow_mut().take() {
            hook();
        }
    });
}

#[cfg(test)]
use super::{
    run_before_meta_remove_subsystem_child_inspection_hook, META_REMOVE_FORCED_REPARSE_PATHS,
};

fn validate_meta_remove_object_name(name: &str) -> Result<(), String> {
    let mut components = Path::new(name).components();
    let is_single_path_component = matches!(
        components.next(),
        Some(std::path::Component::Normal(component))
            if component == std::ffi::OsStr::new(name)
    ) && components.next().is_none();

    if form_is_xml_ncname(name) && is_single_path_component {
        Ok(())
    } else {
        Err(format!(
            "Object name must be a valid Unicode XML NCName and a single path component: {name:?}"
        ))
    }
}

pub(super) struct MetaRemoveSubsystemReplacement {
    path: PathBuf,
    original: Vec<u8>,
    replacement: Vec<u8>,
    subsystem_name: String,
}

pub(super) struct MetaRemoveTextRead {
    path: PathBuf,
    raw: Vec<u8>,
    text: String,
}

struct MetaRemoveReferenceScan {
    references: Vec<MetaRemoveReference>,
    reads: Vec<MetaRemoveTextRead>,
    directory_reads: Vec<MetaRemoveDirectoryRead>,
}

struct MetaRemoveDirectoryRead {
    path: PathBuf,
    direct_entries: Vec<DirectoryTopologyEntry>,
}

pub(super) struct MetaRemoveTraversal {
    files: Vec<PathBuf>,
    directories: Vec<MetaRemoveDirectoryRead>,
}

const META_REMOVE_MAX_TRAVERSAL_DEPTH: usize = 256;
const META_REMOVE_MAX_TRAVERSAL_ENTRIES: usize = 1_000_000;

#[derive(Clone, Copy)]
pub(super) struct MetaRemoveTraversalLimits {
    pub(crate) max_depth: usize,
    pub(crate) max_entries: usize,
}

fn meta_remove_preserved_utf8_image(original: &[u8], updated: String) -> Vec<u8> {
    let mut image = Vec::with_capacity(original.len());
    if original.starts_with(b"\xef\xbb\xbf") {
        image.extend_from_slice(b"\xef\xbb\xbf");
    }
    image.extend_from_slice(updated.as_bytes());
    image
}

fn remove_subsystem_content_items(
    xml_text: &str,
    qualified_object_name: &str,
) -> Result<(String, usize), String> {
    const MD_NS: &str = "http://v8.1c.ru/8.3/MDClasses";
    let document = Document::parse(xml_text.trim_start_matches('\u{feff}'))
        .map_err(|error| format!("XML parse error: {error}"))?;
    let content = document
        .descendants()
        .find(|node| role_info_element(*node, "Content", Some(MD_NS)));
    let Some(content) = content else {
        return Ok((xml_text.to_string(), 0));
    };
    let mut ranges = content
        .children()
        .filter(|node| node.is_element() && node.tag_name().name() == "Item")
        .filter(|node| {
            node.text()
                .is_some_and(|text| text.trim() == qualified_object_name)
        })
        .map(|node| node.range())
        .collect::<Vec<_>>();
    if ranges.is_empty() {
        return Ok((xml_text.to_string(), 0));
    }
    ranges.sort_by_key(|range| range.start);
    let removed = ranges.len();
    let mut updated = xml_text.to_string();
    for range in ranges.into_iter().rev() {
        let line_start = updated[..range.start]
            .rfind('\n')
            .map_or(0, |index| index + 1);
        let leading_is_indent = updated[line_start..range.start]
            .chars()
            .all(|character| matches!(character, ' ' | '\t' | '\r'));
        let removal = if leading_is_indent && updated[range.end..].starts_with("\r\n") {
            line_start..range.end + 2
        } else if leading_is_indent && updated[range.end..].starts_with('\n') {
            line_start..range.end + 1
        } else {
            range
        };
        updated.replace_range(removal, "");
    }
    Ok((updated, removed))
}

fn meta_remove_path_is_link_or_reparse_point(_path: &Path, metadata: &fs::Metadata) -> bool {
    if crate::infrastructure::platform::filesystem::metadata_is_link_or_reparse_point(metadata) {
        return true;
    }
    #[cfg(test)]
    {
        META_REMOVE_FORCED_REPARSE_PATHS.with(|slot| slot.borrow().contains(_path))
    }
    #[cfg(not(test))]
    false
}

fn require_meta_remove_real_path(
    path: &Path,
    metadata: &fs::Metadata,
    role: &str,
) -> Result<(), String> {
    if meta_remove_path_is_link_or_reparse_point(path, metadata) {
        Err(format!(
            "{role} must not be a symbolic link or reparse point: {}",
            path.display()
        ))
    } else {
        Ok(())
    }
}

#[derive(Clone, Copy)]
struct MetaRemoveDirectoryWalkPolicy {
    label: &'static str,
    allow_absent_root: bool,
}

type InspectedMetaRemoveDirectoryEntry = (PathBuf, OsString, DirectoryTopologyEntryKind);

fn inspect_meta_remove_directory(
    dir: &Path,
    depth: usize,
    limits: MetaRemoveTraversalLimits,
    policy: MetaRemoveDirectoryWalkPolicy,
    visited_directories: &mut HashSet<PathBuf>,
    visited_entries: &mut usize,
) -> Result<Option<Vec<InspectedMetaRemoveDirectoryEntry>>, String> {
    if depth > limits.max_depth {
        return Err(format!(
            "{} traversal exceeded the maximum depth of {}: {}",
            policy.label,
            limits.max_depth,
            dir.display()
        ));
    }
    let metadata = match fs::symlink_metadata(dir) {
        Ok(metadata) => metadata,
        Err(error)
            if policy.allow_absent_root && depth == 0 && error.kind() == ErrorKind::NotFound =>
        {
            return Ok(None);
        }
        Err(error) => {
            return Err(format!(
                "failed to inspect {} directory {}: {error}",
                policy.label,
                dir.display()
            ));
        }
    };
    require_meta_remove_real_path(dir, &metadata, &format!("{} directory", policy.label))?;
    if !metadata.is_dir() {
        return Err(format!(
            "{} path is not a directory: {}",
            policy.label,
            dir.display()
        ));
    }
    let directory_identity = fs::canonicalize(dir).map_err(|error| {
        format!(
            "failed to resolve {} directory identity {}: {error}",
            policy.label,
            dir.display()
        )
    })?;
    if !visited_directories.insert(directory_identity) {
        return Err(format!(
            "{} traversal directory cycle or duplicate identity detected before traversal: {}",
            policy.label,
            dir.display()
        ));
    }

    let directory_entries = fs::read_dir(dir).map_err(|error| {
        format!(
            "failed to read {} directory {}: {error}",
            policy.label,
            dir.display()
        )
    })?;
    let mut entries = Vec::new();
    for entry in directory_entries {
        let entry = entry.map_err(|error| {
            format!(
                "failed to read an entry in {} directory {}: {error}",
                policy.label,
                dir.display()
            )
        })?;
        if *visited_entries >= limits.max_entries {
            return Err(format!(
                "{} traversal exceeded the maximum of {} entries: {}",
                policy.label,
                limits.max_entries,
                dir.display()
            ));
        }
        *visited_entries += 1;
        entries.push(entry);
    }
    entries.sort_by_key(|entry| entry.file_name());

    entries
        .into_iter()
        .map(|entry| {
            let path = entry.path();
            let metadata = fs::symlink_metadata(&path).map_err(|error| {
                format!(
                    "failed to inspect {} entry {}: {error}",
                    policy.label,
                    path.display()
                )
            })?;
            require_meta_remove_real_path(&path, &metadata, &format!("{} entry", policy.label))?;
            let kind = if metadata.is_dir() {
                DirectoryTopologyEntryKind::Directory
            } else if metadata.is_file() {
                DirectoryTopologyEntryKind::File
            } else {
                return Err(format!(
                    "{} entry has an unsupported filesystem type: {}",
                    policy.label,
                    path.display()
                ));
            };
            Ok((path, entry.file_name(), kind))
        })
        .collect::<Result<Vec<_>, _>>()
        .map(Some)
}

pub(super) fn plan_meta_remove_subsystem_replacements(
    dir: &Path,
    qualified_object_name: &str,
    replacements: &mut Vec<MetaRemoveSubsystemReplacement>,
    descriptor_reads: &mut Vec<MetaRemoveTextRead>,
) -> Result<(), String> {
    let mut visited_directories = HashSet::new();
    let mut visited_entries = 0usize;
    plan_meta_remove_subsystem_replacements_bounded(
        dir,
        qualified_object_name,
        replacements,
        descriptor_reads,
        0,
        MetaRemoveTraversalLimits {
            max_depth: META_REMOVE_MAX_TRAVERSAL_DEPTH,
            max_entries: META_REMOVE_MAX_TRAVERSAL_ENTRIES,
        },
        &mut visited_directories,
        &mut visited_entries,
    )
}

#[allow(clippy::too_many_arguments)]
pub(super) fn plan_meta_remove_subsystem_replacements_bounded(
    dir: &Path,
    qualified_object_name: &str,
    replacements: &mut Vec<MetaRemoveSubsystemReplacement>,
    descriptor_reads: &mut Vec<MetaRemoveTextRead>,
    depth: usize,
    limits: MetaRemoveTraversalLimits,
    visited_directories: &mut HashSet<PathBuf>,
    visited_entries: &mut usize,
) -> Result<(), String> {
    let Some(entries) = inspect_meta_remove_directory(
        dir,
        depth,
        limits,
        MetaRemoveDirectoryWalkPolicy {
            label: "subsystem",
            allow_absent_root: false,
        },
        visited_directories,
        visited_entries,
    )?
    else {
        return Err(format!(
            "subsystem directory disappeared before traversal: {}",
            dir.display()
        ));
    };
    let descriptors = entries
        .into_iter()
        .filter_map(|(path, _, kind)| {
            let is_xml = path
                .extension()
                .and_then(|ext| ext.to_str())
                .is_some_and(|ext| ext.eq_ignore_ascii_case("xml"));
            (is_xml && kind == DirectoryTopologyEntryKind::File).then_some(path)
        })
        .collect::<Vec<_>>();

    for path in descriptors {
        let snapshot = read_utf8_sig_snapshot(&path)?;
        let subsystem_name = first_tag_text_in_xml(&snapshot.text, "Name")
            .unwrap_or_else(|| file_stem_string(&path));
        let (updated, removed_references) =
            remove_subsystem_content_items(&snapshot.text, qualified_object_name)?;
        descriptor_reads.push(MetaRemoveTextRead {
            path: path.clone(),
            raw: snapshot.raw.clone(),
            text: snapshot.text.clone(),
        });

        let child_dir = path
            .parent()
            .unwrap_or(dir)
            .join(file_stem_string(&path))
            .join("Subsystems");
        if removed_references > 0 {
            let replacement = meta_remove_preserved_utf8_image(&snapshot.raw, updated);
            replacements.push(MetaRemoveSubsystemReplacement {
                path: path.clone(),
                original: snapshot.raw,
                replacement,
                subsystem_name,
            });
        }

        #[cfg(test)]
        run_before_meta_remove_subsystem_child_inspection_hook(&child_dir);
        match fs::symlink_metadata(&child_dir) {
            Ok(metadata) => {
                require_meta_remove_real_path(&child_dir, &metadata, "subsystem directory")?;
                if metadata.is_dir() {
                    if depth >= limits.max_depth {
                        return Err(format!(
                            "subsystem traversal exceeded the maximum depth of {}: {}",
                            limits.max_depth,
                            child_dir.display()
                        ));
                    }
                    plan_meta_remove_subsystem_replacements_bounded(
                        &child_dir,
                        qualified_object_name,
                        replacements,
                        descriptor_reads,
                        depth + 1,
                        limits,
                        visited_directories,
                        visited_entries,
                    )?;
                }
            }
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
            Err(err) => {
                return Err(format!("failed to inspect {}: {err}", child_dir.display()));
            }
        }
    }

    Ok(())
}

fn meta_remove_payload_file_count(path: &Path) -> Result<Option<usize>, String> {
    match fs::symlink_metadata(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(format!(
            "failed to inspect metadata payload directory {}: {error}",
            path.display()
        )),
        Ok(metadata)
            if crate::infrastructure::platform::filesystem::metadata_is_link_or_reparse_point(
                &metadata,
            ) =>
        {
            Err(format!(
                "metadata payload directory must not be a symbolic link or reparse point: {}",
                path.display()
            ))
        }
        Ok(metadata) if metadata.is_dir() => Ok(Some(metadata_files_recursive(path)?.files.len())),
        Ok(_) => Err(format!(
            "metadata payload path is not a directory: {}",
            path.display()
        )),
    }
}

pub(super) struct TypedMetaRemovePlan {
    pub(super) preview: MetaMutationData,
    pub(super) validation_subject: MetadataValidationSubject,
    pub(super) transaction: CompileTransaction,
    pub(super) resolved: ResolvedMetadataObject,
    pub(super) expected_post_images: Vec<(PathBuf, Vec<u8>)>,
    pub(super) expected_absent: Vec<PathBuf>,
}

pub(crate) fn plan_typed_remove(
    request: &MetaRemoveRequest,
    resolved: ResolvedMetadataObject,
    context: &WorkspaceContext,
    cancellation: &CancellationToken,
) -> Result<TypedMetaRemovePlan, MetaFailure> {
    let target = &request.metadata_path;
    if cancellation.is_cancelled() {
        return Err(typed_remove_failure(
            MetaDiagnosticCode::ProviderUnavailable,
            target,
            "metadata remove was cancelled before planning",
            None,
        ));
    }
    if (request.force && !request.confirm) || (request.confirm && !request.force) {
        return Err(typed_remove_failure(
            MetaDiagnosticCode::InvalidArguments,
            target,
            "forced metadata removal requires force=true and confirm=true",
            Some("force"),
        ));
    }

    let resolved_target = &resolved.metadata_path;
    let segments = resolved_target.segments().collect::<Vec<_>>();
    let [object_kind, object_name] = segments.as_slice() else {
        return Err(typed_remove_failure(
            MetaDiagnosticCode::InvalidArguments,
            target,
            "metadata remove accepts one top-level metadata object",
            Some("metadataPath"),
        ));
    };
    let kind = MetadataKind::parse(object_kind).map_err(|_| {
        typed_remove_failure(
            MetaDiagnosticCode::CapabilityUnavailable,
            target,
            "metadata target does not provide typed Platform XML removal",
            Some("metadataPath"),
        )
    })?;
    validate_meta_remove_object_name(object_name).map_err(|_| {
        typed_remove_failure(
            MetaDiagnosticCode::InvalidArguments,
            target,
            "metadataPath contains an invalid object name",
            Some("metadataPath"),
        )
    })?;

    let mut diagnostics = Vec::new();
    match evaluate_resolved_support_guard(
        &resolved.descriptor_path,
        SupportGuardRequirement::Removed,
        context,
    ) {
        ResolvedSupportGuardCheck::Allow | ResolvedSupportGuardCheck::Warn(_) => {}
        ResolvedSupportGuardCheck::Block(_) => {
            return Err(typed_remove_failure(
                MetaDiagnosticCode::SupportLocked,
                target,
                "metadata support policy blocks object removal",
                None,
            ));
        }
    }

    let type_dir = resolved.source_root.join(metadata_layout(kind).directory);
    let object_dir = type_dir.join(object_name);
    let has_payload = meta_remove_payload_file_count(&object_dir)
        .map_err(|_| typed_remove_provider_failure(target))?
        .is_some();
    let reference_scan = meta_remove_reference_scan(
        &resolved.source_root,
        object_kind,
        object_name,
        metadata_layout(kind).directory,
        &resolved.descriptor_path,
        &object_dir,
        true,
        has_payload,
    )
    .map_err(|_| typed_remove_provider_failure(target))?;
    let logical_referrers =
        typed_remove_logical_referrers(request, &reference_scan.references, context, cancellation)?;
    if !logical_referrers.is_empty() && !request.force {
        return Err(MetaFailure {
            diagnostics: logical_referrers
                .iter()
                .map(|referrer| {
                    MetaDiagnostic::error(
                        MetaDiagnosticCode::ReferenceConflict,
                        format!(
                            "`{}` refers to `{target}`; use the confirmed force gate to remove it",
                            referrer.as_str()
                        ),
                    )
                    .with_metadata_path(referrer.clone())
                    .with_field("force")
                })
                .collect(),
        });
    }
    for referrer in &logical_referrers {
        diagnostics.push(
            MetaDiagnostic::warning(
                MetaDiagnosticCode::ReferenceConflict,
                format!(
                    "confirmed removal leaves the logical reference from `{}` to `{target}`",
                    referrer.as_str()
                ),
            )
            .with_metadata_path(referrer.clone())
            .with_field("force"),
        );
    }

    let subsystems_dir = resolved.source_root.join("Subsystems");
    let mut subsystem_replacements = Vec::new();
    let mut subsystem_descriptor_reads = Vec::new();
    match fs::symlink_metadata(&subsystems_dir) {
        Ok(_) => plan_meta_remove_subsystem_replacements(
            &subsystems_dir,
            target.as_str(),
            &mut subsystem_replacements,
            &mut subsystem_descriptor_reads,
        )
        .map_err(|_| typed_remove_provider_failure(target))?,
        Err(error) if error.kind() == ErrorKind::NotFound => {}
        Err(_) => return Err(typed_remove_provider_failure(target)),
    }
    for replacement in &subsystem_replacements {
        match evaluate_resolved_support_guard(
            &replacement.path,
            SupportGuardRequirement::Editable,
            context,
        ) {
            ResolvedSupportGuardCheck::Allow | ResolvedSupportGuardCheck::Warn(_) => {}
            ResolvedSupportGuardCheck::Block(_) => {
                return Err(typed_remove_failure(
                    MetaDiagnosticCode::SupportLocked,
                    target,
                    format!(
                        "subsystem `{}` support policy blocks reference cleanup",
                        replacement.subsystem_name
                    ),
                    Some("dependencies"),
                ));
            }
        }
    }

    let owner_text = std::str::from_utf8(&resolved.owner_preimage)
        .map_err(|_| typed_remove_provider_failure(target))?
        .trim_start_matches('\u{feff}');
    let (owner_post_text, deregistered) =
        remove_metadata_child_text_with_flag(owner_text, object_kind, object_name);
    if !deregistered {
        return Err(typed_remove_failure(
            MetaDiagnosticCode::TargetNotFound,
            target,
            "metadata object is not registered by its owner",
            Some("metadataPath"),
        ));
    }
    let owner_post_image =
        meta_remove_preserved_utf8_image(&resolved.owner_preimage, owner_post_text);

    #[cfg(test)]
    run_meta_remove_before_reauthorization_hook();
    if cancellation.is_cancelled() {
        return Err(typed_remove_failure(
            MetaDiagnosticCode::ProviderUnavailable,
            target,
            "metadata remove was cancelled before guarded transaction preparation",
            None,
        ));
    }

    let mut transaction = CompileTransaction::new();
    bind_resolved_support_guard_evidence(&mut transaction, &resolved.descriptor_path, context)
        .map_err(|_| typed_remove_provider_failure(target))?;
    for replacement in &subsystem_replacements {
        bind_resolved_support_guard_evidence(&mut transaction, &replacement.path, context)
            .map_err(|_| typed_remove_provider_failure(target))?;
    }
    match evaluate_resolved_support_guard(
        &resolved.descriptor_path,
        SupportGuardRequirement::Removed,
        context,
    ) {
        ResolvedSupportGuardCheck::Allow => {}
        ResolvedSupportGuardCheck::Warn(_) => diagnostics.push(
            MetaDiagnostic::warning(
                MetaDiagnosticCode::SupportLocked,
                "metadata support policy permits removal with a warning",
            )
            .with_metadata_path(target.clone()),
        ),
        ResolvedSupportGuardCheck::Block(_) => {
            return Err(typed_remove_failure(
                MetaDiagnosticCode::SupportLocked,
                target,
                "metadata support policy blocks object removal",
                None,
            ));
        }
    }
    for replacement in &subsystem_replacements {
        match evaluate_resolved_support_guard(
            &replacement.path,
            SupportGuardRequirement::Editable,
            context,
        ) {
            ResolvedSupportGuardCheck::Allow => {}
            ResolvedSupportGuardCheck::Warn(_) => diagnostics.push(
                MetaDiagnostic::warning(
                    MetaDiagnosticCode::SupportLocked,
                    format!(
                        "subsystem `{}` support policy permits cleanup with a warning",
                        replacement.subsystem_name
                    ),
                )
                .with_metadata_path(target.clone())
                .with_field("dependencies"),
            ),
            ResolvedSupportGuardCheck::Block(_) => {
                return Err(typed_remove_failure(
                    MetaDiagnosticCode::SupportLocked,
                    target,
                    format!(
                        "subsystem `{}` support policy blocks reference cleanup",
                        replacement.subsystem_name
                    ),
                    Some("dependencies"),
                ));
            }
        }
    }
    transaction
        .replace_bytes(
            &resolved.owner_path,
            &resolved.owner_preimage,
            owner_post_image.clone(),
        )
        .map_err(|_| typed_remove_provider_failure(target))?;
    for replacement in &subsystem_replacements {
        transaction
            .replace_bytes(
                &replacement.path,
                &replacement.original,
                replacement.replacement.clone(),
            )
            .map_err(|_| typed_remove_provider_failure(target))?;
    }

    let mut removal_targets = vec![resolved.descriptor_path.as_path()];
    if has_payload {
        removal_targets.push(object_dir.as_path());
    }
    let remove_collection = transaction
        .remove_directory_if_only_direct_entries(
            &type_dir,
            removal_targets
                .iter()
                .map(|path| {
                    path.file_name()
                        .expect("typed metadata removal target has one file name")
                        .to_os_string()
                })
                .collect(),
        )
        .map_err(|_| typed_remove_provider_failure(target))?;
    let mut removed_paths = Vec::new();
    if remove_collection {
        removed_paths.push(type_dir.clone());
    } else {
        if has_payload {
            transaction
                .remove_path(&object_dir)
                .map_err(|_| typed_remove_provider_failure(target))?;
            removed_paths.push(object_dir.clone());
        } else {
            transaction
                .guard_path_absent(&object_dir)
                .map_err(|_| typed_remove_provider_failure(target))?;
        }
        transaction
            .remove_path(&resolved.descriptor_path)
            .map_err(|_| typed_remove_provider_failure(target))?;
        removed_paths.push(resolved.descriptor_path.clone());
    }

    for read in reference_scan
        .reads
        .iter()
        .chain(subsystem_descriptor_reads.iter())
    {
        transaction
            .guard_or_verify_exact_preimage(&read.path, &read.raw)
            .map_err(|_| typed_remove_provider_failure(target))?;
    }
    let mut dependency_paths = vec![resolved.owner_path.clone()];
    dependency_paths.extend(
        reference_scan
            .reads
            .iter()
            .filter(|read| {
                read.path
                    .extension()
                    .and_then(|extension| extension.to_str())
                    .is_some_and(|extension| extension.eq_ignore_ascii_case("xml"))
            })
            .map(|read| read.path.clone()),
    );
    dependency_paths.extend(
        subsystem_descriptor_reads
            .iter()
            .map(|read| read.path.clone()),
    );
    let subsystem_descriptors = subsystem_replacements
        .iter()
        .map(|replacement| replacement.path.as_path())
        .collect::<Vec<_>>();
    dependency_paths.extend(subsystem_validation_format_dependency_paths(
        &subsystem_descriptors,
    ));
    dependency_paths.sort();
    dependency_paths.dedup();
    let dependencies = dependency_paths
        .iter()
        .map(PathBuf::as_path)
        .collect::<Vec<_>>();
    let mut trees = vec![resolved.descriptor_path.as_path()];
    if has_payload {
        trees.push(object_dir.as_path());
    }
    guard_active_format_dependencies_and_xml_trees(
        &mut transaction,
        &dependencies,
        &trees,
        context,
    )
    .map_err(|_| {
        typed_remove_failure(
            MetaDiagnosticCode::CapabilityUnavailable,
            target,
            "metadata removal is outside the supported Platform XML format profile",
            None,
        )
    })?;
    super::super::common::guard_resolved_platform_xml_target_dependencies(
        &mut transaction,
        &resolved.handle,
        context,
    )
    .map_err(|_| typed_remove_provider_failure(target))?;
    let mut guarded_directories = BTreeSet::new();
    for directory_read in &reference_scan.directory_reads {
        if removed_paths
            .iter()
            .any(|removed| directory_read.path.starts_with(removed))
            || !guarded_directories.insert(directory_read.path.clone())
        {
            continue;
        }
        transaction
            .guard_or_verify_directory_topology(
                &directory_read.path,
                DirectoryMembershipSnapshot::Present(directory_read.direct_entries.clone()),
            )
            .map_err(|_| typed_remove_provider_failure(target))?;
    }

    let mut validation_resources = vec![MetadataResourceImage {
        role: MetadataResourceRole::Registration,
        bytes: owner_post_image.clone(),
    }];
    let mut expected_post_images = vec![(resolved.owner_path.clone(), owner_post_image)];
    let mut publication_plan = vec![MetaPublicationPlanEntry {
        action: MetaPublicationAction::Remove,
        resource: MetaPublicationResource::Descriptor,
        metadata_path: Some(target.clone()),
    }];
    if has_payload {
        publication_plan.extend(typed_remove_payload_publication_plan(target, &object_dir)?);
    }
    publication_plan.push(MetaPublicationPlanEntry {
        action: MetaPublicationAction::Update,
        resource: MetaPublicationResource::Registration,
        metadata_path: Some(target.clone()),
    });
    for replacement in subsystem_replacements {
        let subsystem_target = MetadataAddress::parse(
            PLATFORM_XML_8_3_27_FORMAT_2_20,
            &format!("Subsystem.{}", replacement.subsystem_name),
        )
        .map_err(|_| typed_remove_provider_failure(target))?;
        validation_resources.push(MetadataResourceImage {
            role: MetadataResourceRole::Dependency {
                target: subsystem_target.clone(),
            },
            bytes: replacement.replacement.clone(),
        });
        expected_post_images.push((replacement.path, replacement.replacement));
        publication_plan.push(MetaPublicationPlanEntry {
            action: MetaPublicationAction::Update,
            resource: MetaPublicationResource::Dependency,
            metadata_path: Some(subsystem_target),
        });
    }

    let mut expected_absent = vec![resolved.descriptor_path.clone(), object_dir];
    if remove_collection {
        expected_absent.push(type_dir);
    }
    Ok(TypedMetaRemovePlan {
        preview: MetaMutationData {
            metadata_path: resolved_target.clone(),
            changed: true,
            publication_plan,
            effects: vec![MetaMutationEffect {
                operation_index: None,
                operation: "removeObject".to_string(),
                target: resolved_target.as_str().to_string(),
                before: Some(serde_json::json!({
                    "metadataPath": resolved_target.as_str(),
                    "kind": kind.as_str(),
                    "name": object_name,
                })),
                after: None,
            }],
            validation: MetaValidationData {
                status: MetaValidationStatus::Passed,
                diagnostics: Vec::new(),
            },
            diagnostics,
        },
        validation_subject: MetadataValidationSubject {
            target: target.clone(),
            resources: validation_resources,
            child_footprints: Vec::new(),
            registrar_evidence: Default::default(),
        },
        transaction,
        resolved,
        expected_post_images,
        expected_absent,
    })
}

fn typed_remove_logical_referrers(
    request: &MetaRemoveRequest,
    references: &[MetaRemoveReference],
    context: &WorkspaceContext,
    cancellation: &CancellationToken,
) -> Result<Vec<MetadataAddress>, MetaFailure> {
    let mut logical = BTreeMap::new();
    for reference in references {
        if cancellation.is_cancelled() {
            return Err(typed_remove_provider_failure(&request.metadata_path));
        }
        let located = locate_platform_xml_source_path(
            context,
            &SourceLocateRequest {
                source_set: request.source_set.clone(),
                path: reference.file.clone(),
            },
            cancellation,
        )
        .map_err(|_| typed_remove_provider_failure(&request.metadata_path))?;
        let referrer = located
            .metadata_path
            .or(located.owner_metadata_path)
            .ok_or_else(|| typed_remove_provider_failure(&request.metadata_path))?;
        logical
            .entry(referrer.as_str().to_string())
            .or_insert(referrer);
    }
    Ok(logical.into_values().collect())
}

fn typed_remove_payload_publication_plan(
    owner: &MetadataAddress,
    object_dir: &Path,
) -> Result<Vec<MetaPublicationPlanEntry>, MetaFailure> {
    let traversal =
        metadata_files_recursive(object_dir).map_err(|_| typed_remove_provider_failure(owner))?;
    let mut plan = Vec::new();
    for path in traversal.files {
        let relative = path
            .strip_prefix(object_dir)
            .map_err(|_| typed_remove_provider_failure(owner))?;
        let components = relative
            .components()
            .filter_map(|component| component.as_os_str().to_str())
            .collect::<Vec<_>>();
        let (resource, metadata_path) = match components.as_slice() {
            ["Ext", file] if file.ends_with(".bsl") => {
                (MetaPublicationResource::Module, owner.clone())
            }
            [collection, child, ..]
                if matches!(*collection, "Forms" | "Templates" | "Commands") =>
            {
                let child_name = child.strip_suffix(".xml").unwrap_or(child);
                let (resource, kind) = match *collection {
                    "Forms" => (MetaPublicationResource::Form, "Form"),
                    "Templates" => (MetaPublicationResource::Template, "Template"),
                    "Commands" => (MetaPublicationResource::Command, "Command"),
                    _ => unreachable!(),
                };
                let child_path = MetadataAddress::parse(
                    PLATFORM_XML_8_3_27_FORMAT_2_20,
                    &format!(
                        "{}.{}.{kind}.{child_name}",
                        owner.segments().next().unwrap(),
                        owner.segments().nth(1).unwrap()
                    ),
                )
                .map_err(|_| typed_remove_provider_failure(owner))?;
                (resource, child_path)
            }
            _ => (MetaPublicationResource::Dependency, owner.clone()),
        };
        let entry = MetaPublicationPlanEntry {
            action: MetaPublicationAction::Remove,
            resource,
            metadata_path: Some(metadata_path),
        };
        if !plan.contains(&entry) {
            plan.push(entry);
        }
    }
    if plan.is_empty() {
        plan.push(MetaPublicationPlanEntry {
            action: MetaPublicationAction::Remove,
            resource: MetaPublicationResource::Dependency,
            metadata_path: Some(owner.clone()),
        });
    }
    Ok(plan)
}

fn typed_remove_provider_failure(target: &MetadataAddress) -> MetaFailure {
    typed_remove_failure(
        MetaDiagnosticCode::ProviderUnavailable,
        target,
        "metadata provider could not prepare the guarded removal",
        None,
    )
}

fn typed_remove_failure(
    code: MetaDiagnosticCode,
    target: &MetadataAddress,
    message: impl Into<String>,
    field: Option<&str>,
) -> MetaFailure {
    let mut diagnostic = MetaDiagnostic::error(code, message).with_metadata_path(target.clone());
    if let Some(field) = field {
        diagnostic = diagnostic.with_field(field);
    }
    diagnostic.into()
}

pub(crate) fn remove_metadata_child_text_with_flag(
    xml_text: &str,
    local_name: &str,
    item_name: &str,
) -> (String, bool) {
    let plain = format!("<{local_name}>{item_name}</{local_name}>");
    let prefixed = format!("<md:{local_name}>{item_name}</md:{local_name}>");
    let mut removed = false;
    let mut result = String::with_capacity(xml_text.len());
    for line in xml_text.split_inclusive('\n') {
        let trimmed = line.trim();
        if !removed && (trimmed == plain || trimmed == prefixed) {
            removed = true;
            continue;
        }
        result.push_str(line);
    }
    if removed {
        (result, true)
    } else if let Some(index) = xml_text.find(&plain) {
        let mut result = xml_text.to_string();
        result.replace_range(index..index + plain.len(), "");
        (result, true)
    } else if let Some(index) = xml_text.find(&prefixed) {
        let mut result = xml_text.to_string();
        result.replace_range(index..index + prefixed.len(), "");
        (result, true)
    } else {
        (xml_text.to_string(), false)
    }
}

pub(super) struct MetaRemoveReference {
    pub(crate) file: String,
}

/// Largest single source file the reference scan will read.
///
/// The scan decides whether an object may be removed, so it reads every XML and
/// BSL file under the source root. Without a per-file bound one oversized file
/// would size the whole operation, and the traversal limits above only bound
/// how many entries are visited, not how big each one is.
pub(super) const META_REMOVE_REFERENCE_FILE_MAX_BYTES: usize = 16 * 1024 * 1024;

/// Read one reference-scan file bound to the source root.
///
/// The scan runs against a tree another writer may be touching, and its verdict
/// gates a destructive publication. Reading through a directory-relative
/// no-follow handle keeps a symlink swapped in mid-scan from redirecting the
/// read outside the root, which a plain path read would follow.
pub(super) fn read_reference_scan_snapshot(
    root: &Path,
    path: &Path,
) -> Result<Utf8TextSnapshot, String> {
    let read =
        read_root_relative_regular_file(root, path, META_REMOVE_REFERENCE_FILE_MAX_BYTES, |_| {})
            .map_err(|error| format!("failed to read {}: {error}", path.display()))?;
    let text = std::str::from_utf8(&read.bytes)
        .map_err(|error| format!("{} is not valid UTF-8: {error}", path.display()))?
        .trim_start_matches('\u{feff}')
        .to_string();
    Ok(Utf8TextSnapshot {
        raw: read.bytes,
        text,
    })
}

#[allow(clippy::too_many_arguments)]
fn meta_remove_reference_scan(
    config_dir: &Path,
    obj_type: &str,
    obj_name: &str,
    type_plural: &str,
    obj_xml: &Path,
    obj_dir: &Path,
    has_xml: bool,
    has_dir: bool,
) -> Result<MetaRemoveReferenceScan, String> {
    let patterns = meta_remove_search_patterns(obj_type, obj_name, type_plural);
    let mut references = Vec::new();
    let mut already_found = HashSet::new();
    let mut reads = Vec::new();
    let traversal = metadata_files_recursive(config_dir)?;

    for file in traversal.files.iter().filter(|file| {
        matches!(
            file.extension().and_then(|ext| ext.to_str()).map(str::to_ascii_lowercase),
            Some(ext) if ext == "xml" || ext == "bsl"
        )
    }) {
        if meta_remove_should_skip_file(file, config_dir, obj_xml, obj_dir, has_xml, has_dir) {
            continue;
        }
        let snapshot = read_reference_scan_snapshot(config_dir, file)?;
        let content = snapshot.text.clone();
        reads.push(MetaRemoveTextRead {
            path: file.clone(),
            raw: snapshot.raw,
            text: snapshot.text,
        });
        let rel = relative_display(file, config_dir);
        for pattern in &patterns {
            if content.contains(pattern) {
                already_found.insert(rel.clone());
                references.push(MetaRemoveReference { file: rel });
                break;
            }
        }
    }

    let type_name_ref = format!("{obj_type}.{obj_name}");
    for read in reads.iter().filter(|read| {
        read.path
            .extension()
            .and_then(|ext| ext.to_str())
            .is_some_and(|ext| ext.eq_ignore_ascii_case("xml"))
    }) {
        let rel = relative_display(&read.path, config_dir);
        if already_found.contains(&rel) {
            continue;
        }
        if read.text.contains(&type_name_ref) {
            references.push(MetaRemoveReference { file: rel });
        }
    }

    Ok(MetaRemoveReferenceScan {
        references,
        reads,
        directory_reads: traversal.directories,
    })
}

fn metadata_files_recursive(root: &Path) -> Result<MetaRemoveTraversal, String> {
    metadata_files_recursive_with_limits(
        root,
        MetaRemoveTraversalLimits {
            max_depth: META_REMOVE_MAX_TRAVERSAL_DEPTH,
            max_entries: META_REMOVE_MAX_TRAVERSAL_ENTRIES,
        },
    )
}

pub(super) fn metadata_files_recursive_with_limits(
    root: &Path,
    limits: MetaRemoveTraversalLimits,
) -> Result<MetaRemoveTraversal, String> {
    let mut visited_directories = HashSet::new();
    let mut visited_entries = 0usize;
    metadata_files_recursive_bounded(
        root,
        0,
        limits,
        &mut visited_directories,
        &mut visited_entries,
    )
}

pub(super) fn metadata_files_recursive_bounded(
    root: &Path,
    depth: usize,
    limits: MetaRemoveTraversalLimits,
    visited_directories: &mut HashSet<PathBuf>,
    visited_entries: &mut usize,
) -> Result<MetaRemoveTraversal, String> {
    // The format guard calls this helper before the operation can render its
    // normal "Config directory not found" outcome. Preserve that public error
    // path for an initially absent root, while retaining fail-closed traversal
    // once a root or child has been observed.
    let Some(inspected_entries) = inspect_meta_remove_directory(
        root,
        depth,
        limits,
        MetaRemoveDirectoryWalkPolicy {
            label: "reference scan",
            allow_absent_root: true,
        },
        visited_directories,
        visited_entries,
    )?
    else {
        return Ok(MetaRemoveTraversal {
            files: Vec::new(),
            directories: Vec::new(),
        });
    };
    let direct_entries = inspected_entries
        .iter()
        .map(|(_, name, kind)| DirectoryTopologyEntry {
            name: name.clone(),
            kind: *kind,
        })
        .collect();
    let mut result = MetaRemoveTraversal {
        files: Vec::new(),
        directories: vec![MetaRemoveDirectoryRead {
            path: root.to_path_buf(),
            direct_entries,
        }],
    };

    for (path, _, kind) in inspected_entries {
        if kind == DirectoryTopologyEntryKind::Directory {
            if depth >= limits.max_depth {
                return Err(format!(
                    "reference scan traversal exceeded the maximum depth of {}: {}",
                    limits.max_depth,
                    path.display()
                ));
            }
            let nested = metadata_files_recursive_bounded(
                &path,
                depth + 1,
                limits,
                visited_directories,
                visited_entries,
            )?;
            result.files.extend(nested.files);
            result.directories.extend(nested.directories);
        } else {
            result.files.push(path);
        }
    }
    Ok(result)
}

pub(super) fn meta_remove_should_skip_file(
    file: &Path,
    config_dir: &Path,
    obj_xml: &Path,
    obj_dir: &Path,
    has_xml: bool,
    has_dir: bool,
) -> bool {
    if has_xml && file == obj_xml {
        return true;
    }
    if has_dir && (file == obj_dir || file.starts_with(obj_dir)) {
        return true;
    }
    let relative = file.strip_prefix(config_dir).ok();
    let rel = relative_display(file, config_dir);
    rel == "Configuration.xml"
        || rel == "ConfigDumpInfo.xml"
        || relative.is_some_and(|path| {
            path.components().next()
                == Some(std::path::Component::Normal(std::ffi::OsStr::new(
                    "Subsystems",
                )))
        })
}

pub(super) fn meta_remove_search_patterns(
    obj_type: &str,
    obj_name: &str,
    type_plural: &str,
) -> Vec<String> {
    let mut patterns = Vec::new();
    if let Some(ref_names) = meta_remove_type_ref_names(obj_type) {
        patterns.extend(ref_names.iter().map(|name| format!("{name}.{obj_name}")));
    }
    if let Some(manager) = meta_remove_ru_manager(obj_type) {
        patterns.push(format!("{manager}.{obj_name}"));
    }
    patterns.push(format!("{type_plural}.{obj_name}"));
    if obj_type == "CommonModule" {
        patterns.push(format!("{obj_name}."));
        patterns.push(format!("<Handler>{obj_name}."));
        patterns.push(format!("<MethodName>{obj_name}."));
    }
    patterns
}

pub(super) fn meta_remove_type_ref_names(obj_type: &str) -> Option<&'static [&'static str]> {
    match obj_type {
        "Catalog" => Some(&["CatalogRef", "CatalogObject"]),
        "Document" => Some(&["DocumentRef", "DocumentObject"]),
        "Enum" => Some(&["EnumRef"]),
        "ExchangePlan" => Some(&["ExchangePlanRef", "ExchangePlanObject"]),
        "ChartOfAccounts" => Some(&["ChartOfAccountsRef", "ChartOfAccountsObject"]),
        "ChartOfCharacteristicTypes" => Some(&[
            "ChartOfCharacteristicTypesRef",
            "ChartOfCharacteristicTypesObject",
        ]),
        "ChartOfCalculationTypes" => Some(&[
            "ChartOfCalculationTypesRef",
            "ChartOfCalculationTypesObject",
        ]),
        "BusinessProcess" => Some(&["BusinessProcessRef", "BusinessProcessObject"]),
        "Task" => Some(&["TaskRef", "TaskObject"]),
        _ => None,
    }
}

pub(super) fn meta_remove_ru_manager(obj_type: &str) -> Option<&'static str> {
    match obj_type {
        "Catalog" => Some("Справочники"),
        "Document" => Some("Документы"),
        "Enum" => Some("Перечисления"),
        "Constant" => Some("Константы"),
        "InformationRegister" => Some("РегистрыСведений"),
        "AccumulationRegister" => Some("РегистрыНакопления"),
        "AccountingRegister" => Some("РегистрыБухгалтерии"),
        "CalculationRegister" => Some("РегистрыРасчета"),
        "ChartOfAccounts" => Some("ПланыСчетов"),
        "ChartOfCharacteristicTypes" => Some("ПланыВидовХарактеристик"),
        "ChartOfCalculationTypes" => Some("ПланыВидовРасчета"),
        "BusinessProcess" => Some("БизнесПроцессы"),
        "Task" => Some("Задачи"),
        "ExchangePlan" => Some("ПланыОбмена"),
        "Report" => Some("Отчеты"),
        "DataProcessor" => Some("Обработки"),
        "DocumentJournal" => Some("ЖурналыДокументов"),
        _ => None,
    }
}
