use crate::infrastructure::native_operations::common::Utf8TextSnapshot;
use crate::infrastructure::platform::secure_read::read_root_relative_regular_file;
use roxmltree::Document;
use std::collections::HashSet;
use std::ffi::OsString;
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use super::super::common::{file_stem_string, read_utf8_sig_snapshot, relative_display};
use super::super::compile_transaction::DirectoryTopologyEntryKind;
use super::super::role::role_info_element;

#[cfg(test)]
use super::{
    run_before_meta_remove_subsystem_child_inspection_hook, META_REMOVE_FORCED_REPARSE_PATHS,
};

pub(crate) struct MetaRemoveSubsystemReplacement {
    pub(crate) path: PathBuf,
    pub(crate) original: Vec<u8>,
    pub(crate) replacement: Vec<u8>,
}

pub(crate) struct MetaRemoveTextRead {
    path: PathBuf,
    text: String,
}

struct MetaRemoveReferenceScan {
    references: Vec<MetaRemoveReference>,
}

pub(crate) struct MetaRemoveTraversal {
    pub(crate) files: Vec<PathBuf>,
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

pub(crate) fn plan_meta_remove_subsystem_replacements(
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
        let (updated, removed_references) =
            remove_subsystem_content_items(&snapshot.text, qualified_object_name)?;
        descriptor_reads.push(MetaRemoveTextRead {
            path: path.clone(),
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

/// Source files that still mention `Kind.Name`, for planners that stage a
/// removal without the legacy transaction. Same scan, same patterns; only the
/// file list travels back.
#[allow(clippy::too_many_arguments)]
pub(crate) fn typed_remove_reference_files(
    config_dir: &Path,
    obj_type: &str,
    obj_name: &str,
    type_plural: &str,
    obj_xml: &Path,
    obj_dir: &Path,
    has_xml: bool,
    has_dir: bool,
) -> Result<Vec<String>, String> {
    let scan = meta_remove_reference_scan(
        config_dir,
        obj_type,
        obj_name,
        type_plural,
        obj_xml,
        obj_dir,
        has_xml,
        has_dir,
    )?;
    Ok(scan
        .references
        .into_iter()
        .map(|reference| reference.file)
        .collect())
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

    Ok(MetaRemoveReferenceScan { references })
}

pub(crate) fn metadata_files_recursive(root: &Path) -> Result<MetaRemoveTraversal, String> {
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
        return Ok(MetaRemoveTraversal { files: Vec::new() });
    };
    let mut result = MetaRemoveTraversal { files: Vec::new() };

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
