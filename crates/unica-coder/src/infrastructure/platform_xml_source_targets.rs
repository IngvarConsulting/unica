use crate::application::source_navigation::{
    page_bounds, NavigationCompleteness, SourceChildrenRequest, SourceChildrenResult,
    SourceLocation, SourceMatchKind, SourceNavigationMode, SourceNode, SourceNodeAddressability,
    SourceNodeKind, SourceResolveCandidate, SourceResolveRequest, SourceResolveResult,
};
use crate::domain::cancellation::{cancelled_error, CancellationToken};
use crate::domain::project_sources::{
    classify_already_read_config_dump_info_xml, ConfigDumpInfoXmlKind,
};
use crate::domain::project_sources::{SourceFormat, SourceSetKind};
use crate::domain::source_target::{
    MetadataAddress, MetadataAddressPrefix, ResolvedTarget, SourceTarget, SourceTargetError,
    SourceTargetErrorCode, TargetKind,
};
use crate::domain::workspace::WorkspaceContext;
use crate::infrastructure::metadata_kinds::{
    metadata_kind, metadata_kind_by_directory, supports_direct_module_role,
    supports_nested_form_or_command, MetadataKind, METADATA_KINDS,
};
use crate::infrastructure::path_policy::WorkspacePathPolicy;
use crate::infrastructure::platform::filesystem::metadata_is_link_or_reparse_point;
use crate::infrastructure::source_roots::{
    normalize_path_identity, resolve_named_source_set, NamedSourceSetError,
    NamedSourceSetErrorKind, ResolvedNamedSourceSet,
};
#[cfg(test)]
use std::cell::RefCell;
use std::collections::BTreeSet;
use std::fmt;
use std::fs;
use std::path::{Component, Path, PathBuf};

const MD_CLASSES_NS: &str = "http://v8.1c.ru/8.3/MDClasses";
const MAX_NAVIGATION_INVENTORY_ENTRIES: usize = 4_096;
const MAX_NAVIGATION_DESCRIPTOR_BYTES: u64 = 8 * 1024 * 1024;

#[cfg(test)]
thread_local! {
    static NAVIGATION_PROVIDER_ENTRY_HOOK: RefCell<Option<Box<dyn FnOnce()>>> =
        RefCell::new(None);
}

#[cfg(test)]
fn set_navigation_provider_entry_hook_for_test(hook: impl FnOnce() + 'static) {
    NAVIGATION_PROVIDER_ENTRY_HOOK.with(|slot| {
        let previous = slot.replace(Some(Box::new(hook)));
        assert!(previous.is_none(), "navigation provider entry hook leaked");
    });
}

#[cfg(test)]
fn run_navigation_provider_entry_hook_for_test() {
    NAVIGATION_PROVIDER_ENTRY_HOOK.with(|slot| {
        if let Some(hook) = slot.borrow_mut().take() {
            hook();
        }
    });
}

fn check_navigation_cancellation(cancellation: &CancellationToken) -> Result<(), String> {
    if cancellation.is_cancelled() {
        Err(cancelled_error("source navigation stopped"))
    } else {
        Ok(())
    }
}

#[derive(Debug, Clone)]
struct NavigableItem {
    address: MetadataAddress,
    target_kind: TargetKind,
    display_name: String,
}

#[derive(Debug, Clone)]
struct ObservedItem {
    display_name: String,
    relative_path: String,
}

#[derive(Debug)]
struct NavigationInventory {
    source_set: String,
    source_set_kind: SourceSetKind,
    items: Vec<NavigableItem>,
    root_collections: BTreeSet<String>,
    observed_root_items: Vec<ObservedItem>,
    completeness: NavigationCompleteness,
}

pub(crate) fn resolve_platform_xml_source_navigation(
    context: &WorkspaceContext,
    request: &SourceResolveRequest,
    cancellation: &CancellationToken,
) -> Result<SourceResolveResult, String> {
    check_navigation_cancellation(cancellation)?;
    let inventory = navigation_inventory(context, &request.source_set, cancellation)?;
    let query = match request.mode {
        SourceNavigationMode::Exact => MetadataAddress::parse(
            crate::domain::source_target::PLATFORM_XML_8_3_27_FORMAT_2_20,
            request.query.trim(),
        )
        .map(|address| address.as_str().to_string()),
        SourceNavigationMode::Prefix => MetadataAddressPrefix::parse(
            crate::domain::source_target::PLATFORM_XML_8_3_27_FORMAT_2_20,
            request.query.trim(),
        )
        .map(|prefix| prefix.as_str().to_string()),
    }
    .map_err(|error| error.to_string())?;
    let mut matches = Vec::new();
    for item in &inventory.items {
        check_navigation_cancellation(cancellation)?;
        if request
            .target_kind
            .is_none_or(|target_kind| item.target_kind == target_kind)
            && match request.mode {
                SourceNavigationMode::Exact => item.address.as_str() == query,
                SourceNavigationMode::Prefix => item.address.as_str().starts_with(&query),
            }
        {
            matches.push(item.clone());
        }
    }
    check_navigation_cancellation(cancellation)?;
    matches.sort_by(|left, right| left.address.as_str().cmp(right.address.as_str()));
    check_navigation_cancellation(cancellation)?;

    if request.mode == SourceNavigationMode::Exact {
        if inventory.completeness != NavigationCompleteness::Complete {
            return Ok(SourceResolveResult {
                candidates: Vec::new(),
                completeness: NavigationCompleteness::Partial,
                next_cursor: None,
            });
        }
        if matches.len() > 1 {
            return Err(format!(
                "exact logical address is ambiguous in sourceSet `{}`",
                inventory.source_set
            ));
        }
    }

    let cursor_key = format!(
        "resolve:{}:{:?}:{:?}:{}",
        inventory.source_set, request.mode, request.target_kind, query
    );
    let (start, end, next_cursor) = page_bounds(
        request.cursor.as_deref(),
        &cursor_key,
        request.limit,
        matches.len(),
    )?;
    let candidates = matches[start..end]
        .iter()
        .map(|item| SourceResolveCandidate {
            metadata_path: item.address.clone(),
            target_kind: item.target_kind,
            display_name: item.display_name.clone(),
            match_kind: match request.mode {
                SourceNavigationMode::Exact => SourceMatchKind::Exact,
                SourceNavigationMode::Prefix => SourceMatchKind::Prefix,
            },
            location: addressed_location(
                &inventory.source_set,
                Some(item.address.clone()),
                item.target_kind,
            ),
        })
        .collect();
    let completeness =
        if inventory.completeness == NavigationCompleteness::Complete && next_cursor.is_none() {
            NavigationCompleteness::Complete
        } else {
            NavigationCompleteness::Partial
        };
    Ok(SourceResolveResult {
        candidates,
        completeness,
        next_cursor,
    })
}

pub(crate) fn children_platform_xml_source_navigation(
    context: &WorkspaceContext,
    request: &SourceChildrenRequest,
    cancellation: &CancellationToken,
) -> Result<SourceChildrenResult, String> {
    check_navigation_cancellation(cancellation)?;
    let inventory = navigation_inventory(context, &request.source_set, cancellation)?;
    let external_artifact_children_unscanned = request.metadata_path.is_some()
        && matches!(
            inventory.source_set_kind,
            SourceSetKind::ExternalProcessor | SourceSetKind::ExternalReport
        );
    let mut children = if let Some(parent) = request.metadata_path.as_ref() {
        children_of_address(&inventory, parent, cancellation)?
    } else {
        root_children(&inventory, cancellation)?
    };
    check_navigation_cancellation(cancellation)?;
    children.sort_by(|left, right| {
        left.display_name.cmp(&right.display_name).then_with(|| {
            left.metadata_path
                .as_ref()
                .map(MetadataAddress::as_str)
                .cmp(&right.metadata_path.as_ref().map(MetadataAddress::as_str))
        })
    });
    check_navigation_cancellation(cancellation)?;
    let parent_key = request
        .metadata_path
        .as_ref()
        .map(MetadataAddress::as_str)
        .unwrap_or("<root>");
    let cursor_key = format!("children:{}:{parent_key}", inventory.source_set);
    let (start, end, next_cursor) = page_bounds(
        request.cursor.as_deref(),
        &cursor_key,
        request.limit,
        children.len(),
    )?;
    let children = children[start..end].to_vec();
    let completeness = if !external_artifact_children_unscanned
        && inventory.completeness == NavigationCompleteness::Complete
        && next_cursor.is_none()
    {
        NavigationCompleteness::Complete
    } else {
        NavigationCompleteness::Partial
    };
    Ok(SourceChildrenResult {
        children,
        completeness,
        next_cursor,
    })
}

fn navigation_inventory(
    context: &WorkspaceContext,
    source_set: &str,
    cancellation: &CancellationToken,
) -> Result<NavigationInventory, String> {
    check_navigation_cancellation(cancellation)?;
    let selected = resolve_named_source_set(context, source_set)
        .map_err(|error| public_source_set_error(source_set, error).to_string())?;
    check_navigation_cancellation(cancellation)?;
    if selected.source_set.source_format != SourceFormat::PlatformXml {
        return Err(format!(
            "sourceSet `{source_set}` is not addressable by the Platform XML source provider"
        ));
    }
    let mut inventory = NavigationInventory {
        source_set: selected.source_set.name.clone(),
        source_set_kind: selected.source_set.kind,
        items: Vec::new(),
        root_collections: BTreeSet::new(),
        observed_root_items: Vec::new(),
        completeness: NavigationCompleteness::Complete,
    };
    match selected.source_set.kind {
        SourceSetKind::Configuration | SourceSetKind::Extension => {
            check_navigation_cancellation(cancellation)?;
            validate_navigation_owner(context, &selected)?;
            check_navigation_cancellation(cancellation)?;
            collect_configuration_navigation(
                &selected.path,
                context,
                &mut inventory,
                cancellation,
            )?;
        }
        SourceSetKind::ExternalProcessor | SourceSetKind::ExternalReport => {
            collect_external_navigation(&selected.path, &mut inventory, cancellation)?;
        }
    }
    check_navigation_cancellation(cancellation)?;
    inventory
        .items
        .sort_by(|left, right| left.address.as_str().cmp(right.address.as_str()));
    check_navigation_cancellation(cancellation)?;
    for pair in inventory.items.windows(2) {
        check_navigation_cancellation(cancellation)?;
        if pair[0].address == pair[1].address {
            return Err(format!(
                "ambiguous descriptor identity `{}` in sourceSet `{}`",
                pair[0].address.as_str(),
                inventory.source_set
            ));
        }
    }
    Ok(inventory)
}

fn validate_navigation_owner(
    context: &WorkspaceContext,
    selected: &ResolvedNamedSourceSet,
) -> Result<(), String> {
    let owners = crate::infrastructure::platform_xml_owner::resolve_platform_xml_owners(
        &selected.path,
        context,
    )
    .map_err(|_| {
        format!(
            "Platform XML owner evidence is unavailable for sourceSet `{}`",
            selected.source_set.name
        )
    })?;
    if owners.is_empty()
        || owners
            .iter()
            .any(|owner| owner.version.as_deref() != Some("2.20"))
    {
        return Err(format!(
            "sourceSet `{}` does not prove the Platform XML 2.20 address profile",
            selected.source_set.name
        ));
    }
    Ok(())
}

fn collect_configuration_navigation(
    source_root: &Path,
    context: &WorkspaceContext,
    inventory: &mut NavigationInventory,
    cancellation: &CancellationToken,
) -> Result<(), String> {
    #[cfg(test)]
    run_navigation_provider_entry_hook_for_test();
    check_navigation_cancellation(cancellation)?;
    let mut inspected_metadata_entries = 0;
    for kind in METADATA_KINDS {
        check_navigation_cancellation(cancellation)?;
        let directory = source_root.join(kind.directory);
        let metadata = match fs::symlink_metadata(&directory) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(_) => {
                inventory.completeness = NavigationCompleteness::Partial;
                continue;
            }
        };
        if metadata_is_link_or_reparse_point(&metadata) || !metadata.is_dir() {
            inventory.completeness = NavigationCompleteness::Partial;
            continue;
        }
        inventory
            .root_collections
            .insert(kind.directory.to_string());
        let entries = match fs::read_dir(&directory) {
            Ok(entries) => entries,
            Err(_) => {
                inventory.completeness = NavigationCompleteness::Partial;
                continue;
            }
        };
        for entry in entries {
            check_navigation_cancellation(cancellation)?;
            if inspected_metadata_entries >= MAX_NAVIGATION_INVENTORY_ENTRIES {
                inventory.completeness = NavigationCompleteness::Partial;
                break;
            }
            inspected_metadata_entries += 1;
            let Ok(entry) = entry else {
                inventory.completeness = NavigationCompleteness::Partial;
                continue;
            };
            let path = entry.path();
            if path.extension().and_then(|value| value.to_str()) != Some("xml") {
                continue;
            }
            check_navigation_cancellation(cancellation)?;
            match descriptor_name(&path, kind.tag) {
                Ok(name) => add_address(
                    inventory,
                    format!("{}.{name}", kind.tag),
                    name,
                    TargetKind::MetadataObject,
                ),
                Err(()) => inventory.completeness = NavigationCompleteness::Partial,
            }
            check_navigation_cancellation(cancellation)?;
        }
    }

    let mut files = Vec::new();
    let mut inspected_file_entries = 0;
    collect_regular_files_bounded(
        source_root,
        source_root,
        &mut files,
        &mut inspected_file_entries,
        inventory,
        cancellation,
    )?;
    for relative in files {
        check_navigation_cancellation(cancellation)?;
        if relative.extension().and_then(|value| value.to_str()) != Some("bsl") {
            continue;
        }
        let identity = match platform_xml_module_identity(&relative) {
            Ok(identity) => identity,
            Err(_) => {
                inventory.completeness = NavigationCompleteness::Partial;
                continue;
            }
        };
        if !navigation_module_descriptors_are_regular(
            context,
            source_root,
            &identity,
            cancellation,
        )? {
            inventory.completeness = NavigationCompleteness::Partial;
            continue;
        }
        if !module_descriptor_identity_is_proven(source_root, &identity, cancellation)? {
            inventory.completeness = NavigationCompleteness::Partial;
            continue;
        }
        check_navigation_cancellation(cancellation)?;
        let module_address = identity.address.clone();
        let module_display = module_address
            .as_str()
            .split('.')
            .next_back()
            .unwrap_or(module_address.as_str())
            .to_string();
        if module_address.as_str().split('.').count() == 5 {
            if let Some((object, _)) = module_address.as_str().rsplit_once('.') {
                add_address(
                    inventory,
                    object.to_string(),
                    object.rsplit('.').next().unwrap_or(object).to_string(),
                    TargetKind::MetadataObject,
                );
            }
        }
        inventory.items.push(NavigableItem {
            address: module_address,
            target_kind: TargetKind::Module,
            display_name: module_display,
        });
    }
    Ok(())
}

fn navigation_module_descriptors_are_regular(
    context: &WorkspaceContext,
    source_root: &Path,
    identity: &PlatformXmlModuleIdentity,
    cancellation: &CancellationToken,
) -> Result<bool, String> {
    for descriptor in &identity.descriptors {
        check_navigation_cancellation(cancellation)?;
        if validate_platform_xml_module_descriptors(
            context,
            source_root,
            std::slice::from_ref(descriptor),
        )
        .is_err()
        {
            return Ok(false);
        }
        check_navigation_cancellation(cancellation)?;
    }
    Ok(true)
}

fn module_descriptor_identity_is_proven(
    source_root: &Path,
    identity: &PlatformXmlModuleIdentity,
    cancellation: &CancellationToken,
) -> Result<bool, String> {
    let parts = identity.address.segments().collect::<Vec<_>>();
    if parts.len() == 1 {
        return Ok(identity.descriptors.len() == 1);
    }
    let proven = match (parts.as_slice(), identity.descriptors.as_slice()) {
        ([owner_kind, owner_name, _], [owner_descriptor]) => descriptor_identity_matches(
            source_root,
            owner_descriptor,
            owner_kind,
            owner_name,
            cancellation,
        )?,
        (
            [owner_kind, owner_name, child_kind @ ("Form" | "Command"), child_name, _],
            [owner_descriptor, child_descriptor],
        ) => {
            descriptor_identity_matches(
                source_root,
                owner_descriptor,
                owner_kind,
                owner_name,
                cancellation,
            )? && descriptor_identity_matches(
                source_root,
                child_descriptor,
                child_kind,
                child_name,
                cancellation,
            )?
        }
        _ => false,
    };
    Ok(proven)
}

fn descriptor_identity_matches(
    source_root: &Path,
    descriptor: &Path,
    expected_kind: &str,
    expected_name: &str,
    cancellation: &CancellationToken,
) -> Result<bool, String> {
    check_navigation_cancellation(cancellation)?;
    let matches = descriptor_name(&source_root.join(descriptor), expected_kind)
        .is_ok_and(|name| name == expected_name);
    check_navigation_cancellation(cancellation)?;
    Ok(matches)
}

fn collect_external_navigation(
    source_root: &Path,
    inventory: &mut NavigationInventory,
    cancellation: &CancellationToken,
) -> Result<(), String> {
    check_navigation_cancellation(cancellation)?;
    let entries = fs::read_dir(source_root).map_err(|_| {
        format!(
            "external sourceSet `{}` root is unavailable",
            inventory.source_set
        )
    })?;
    let expected_kind = match inventory.source_set_kind {
        SourceSetKind::ExternalProcessor => "ExternalDataProcessor",
        SourceSetKind::ExternalReport => "ExternalReport",
        _ => unreachable!("external collector requires an external source-set kind"),
    };
    let mut candidates = Vec::new();
    for (inspected, entry) in entries.enumerate() {
        check_navigation_cancellation(cancellation)?;
        if inspected >= MAX_NAVIGATION_INVENTORY_ENTRIES {
            inventory.completeness = NavigationCompleteness::Partial;
            break;
        }
        let entry = match entry {
            Ok(entry) => entry,
            Err(_) => {
                inventory.completeness = NavigationCompleteness::Partial;
                continue;
            }
        };
        let path = entry.path();
        if path.extension().and_then(|value| value.to_str()) == Some("xml") {
            candidates.push(path);
        }
    }
    candidates.sort();
    for path in candidates {
        check_navigation_cancellation(cancellation)?;
        let raw = match read_navigation_descriptor(&path) {
            Ok(raw) => raw,
            Err(()) => {
                observe_external_path(inventory, &path);
                continue;
            }
        };
        check_navigation_cancellation(cancellation)?;
        if path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.eq_ignore_ascii_case("ConfigDumpInfo.xml"))
            && classify_already_read_config_dump_info_xml(&raw)
                == ConfigDumpInfoXmlKind::RuntimeSidecar
        {
            continue;
        }
        match descriptor_name_from_bytes(&raw, expected_kind) {
            Ok(name) => add_address(
                inventory,
                format!("{expected_kind}.{name}"),
                name,
                TargetKind::MetadataObject,
            ),
            Err(()) => observe_external_path(inventory, &path),
        }
        check_navigation_cancellation(cancellation)?;
    }
    Ok(())
}

fn collect_regular_files_bounded(
    source_root: &Path,
    directory: &Path,
    files: &mut Vec<PathBuf>,
    inspected_entries: &mut usize,
    inventory: &mut NavigationInventory,
    cancellation: &CancellationToken,
) -> Result<(), String> {
    check_navigation_cancellation(cancellation)?;
    if *inspected_entries >= MAX_NAVIGATION_INVENTORY_ENTRIES {
        inventory.completeness = NavigationCompleteness::Partial;
        return Ok(());
    }
    let entries = match fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(_) => {
            inventory.completeness = NavigationCompleteness::Partial;
            return Ok(());
        }
    };
    let mut paths = Vec::new();
    for entry in entries {
        check_navigation_cancellation(cancellation)?;
        if *inspected_entries >= MAX_NAVIGATION_INVENTORY_ENTRIES {
            inventory.completeness = NavigationCompleteness::Partial;
            break;
        }
        *inspected_entries += 1;
        match entry {
            Ok(entry) => paths.push(entry.path()),
            Err(_) => inventory.completeness = NavigationCompleteness::Partial,
        }
    }
    paths.sort();
    for path in paths {
        check_navigation_cancellation(cancellation)?;
        let metadata = match fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(_) => {
                inventory.completeness = NavigationCompleteness::Partial;
                continue;
            }
        };
        if metadata_is_link_or_reparse_point(&metadata) {
            inventory.completeness = NavigationCompleteness::Partial;
        } else if metadata.is_dir() {
            collect_regular_files_bounded(
                source_root,
                &path,
                files,
                inspected_entries,
                inventory,
                cancellation,
            )?;
        } else if metadata.is_file() {
            match path.strip_prefix(source_root) {
                Ok(relative) => files.push(relative.to_path_buf()),
                Err(_) => inventory.completeness = NavigationCompleteness::Partial,
            }
        }
    }
    Ok(())
}

fn descriptor_name(path: &Path, expected_kind: &str) -> Result<String, ()> {
    let raw = read_navigation_descriptor(path)?;
    descriptor_name_from_bytes(&raw, expected_kind)
}

fn read_navigation_descriptor(path: &Path) -> Result<Vec<u8>, ()> {
    let metadata = fs::symlink_metadata(path).map_err(|_| ())?;
    if metadata_is_link_or_reparse_point(&metadata)
        || !metadata.is_file()
        || metadata.len() > MAX_NAVIGATION_DESCRIPTOR_BYTES
    {
        return Err(());
    }
    fs::read(path).map_err(|_| ())
}

fn descriptor_name_from_bytes(raw: &[u8], expected_kind: &str) -> Result<String, ()> {
    let text = std::str::from_utf8(raw).map_err(|_| ())?;
    let document =
        roxmltree::Document::parse(text.trim_start_matches('\u{feff}')).map_err(|_| ())?;
    let root = document.root_element();
    if root.tag_name().namespace() != Some(MD_CLASSES_NS)
        || root.tag_name().name() != "MetaDataObject"
        || root.attribute("version") != Some("2.20")
    {
        return Err(());
    }
    let artifacts = root
        .children()
        .filter(|node| node.is_element())
        .collect::<Vec<_>>();
    let [artifact] = artifacts.as_slice() else {
        return Err(());
    };
    if artifact.tag_name().namespace() != Some(MD_CLASSES_NS)
        || artifact.tag_name().name() != expected_kind
    {
        return Err(());
    }
    let properties = artifact
        .children()
        .find(|node| node.is_element() && node.tag_name().name() == "Properties")
        .ok_or(())?;
    let names = properties
        .children()
        .filter(|node| node.is_element() && node.tag_name().name() == "Name")
        .collect::<Vec<_>>();
    let [name] = names.as_slice() else {
        return Err(());
    };
    let name = name
        .text()
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .ok_or(())?;
    Ok(name.to_string())
}

fn add_address(
    inventory: &mut NavigationInventory,
    raw_address: String,
    display_name: String,
    target_kind: TargetKind,
) {
    if inventory.items.len() >= MAX_NAVIGATION_INVENTORY_ENTRIES {
        inventory.completeness = NavigationCompleteness::Partial;
        return;
    }
    match MetadataAddress::parse(
        crate::domain::source_target::PLATFORM_XML_8_3_27_FORMAT_2_20,
        &raw_address,
    ) {
        Ok(address) => inventory.items.push(NavigableItem {
            address,
            target_kind,
            display_name,
        }),
        Err(_) => inventory.completeness = NavigationCompleteness::Partial,
    }
}

fn observe_external_path(inventory: &mut NavigationInventory, path: &Path) {
    inventory.completeness = NavigationCompleteness::Partial;
    let Some(file_name) = path.file_name().and_then(|name| name.to_str()) else {
        return;
    };
    inventory.observed_root_items.push(ObservedItem {
        display_name: path
            .file_stem()
            .and_then(|name| name.to_str())
            .unwrap_or(file_name)
            .to_string(),
        relative_path: file_name.to_string(),
    });
}

fn root_children(
    inventory: &NavigationInventory,
    cancellation: &CancellationToken,
) -> Result<Vec<SourceNode>, String> {
    let mut children = Vec::new();
    match inventory.source_set_kind {
        SourceSetKind::Configuration | SourceSetKind::Extension => {
            for collection in &inventory.root_collections {
                check_navigation_cancellation(cancellation)?;
                children.push(SourceNode {
                    display_name: collection.clone(),
                    node_kind: SourceNodeKind::Collection,
                    addressability: SourceNodeAddressability::Unaddressable,
                    completeness: inventory.completeness,
                    metadata_path: None,
                    target_kind: None,
                    location: None,
                });
            }
            for item in &inventory.items {
                check_navigation_cancellation(cancellation)?;
                if item.address.as_str().split('.').count() == 1 {
                    children.push(addressed_node(inventory, item));
                }
            }
        }
        SourceSetKind::ExternalProcessor | SourceSetKind::ExternalReport => {
            for item in &inventory.items {
                check_navigation_cancellation(cancellation)?;
                if item.target_kind == TargetKind::MetadataObject {
                    children.push(addressed_node(inventory, item));
                }
            }
            for item in &inventory.observed_root_items {
                check_navigation_cancellation(cancellation)?;
                children.push(SourceNode {
                    display_name: item.display_name.clone(),
                    node_kind: SourceNodeKind::Item,
                    addressability: SourceNodeAddressability::Unaddressable,
                    completeness: NavigationCompleteness::Partial,
                    metadata_path: None,
                    target_kind: None,
                    location: Some(SourceLocation::Unaddressable {
                        source_set: inventory.source_set.clone(),
                        owner_metadata_path: None,
                        path: item.relative_path.clone(),
                    }),
                });
            }
        }
    }
    Ok(children)
}

fn children_of_address(
    inventory: &NavigationInventory,
    parent: &MetadataAddress,
    cancellation: &CancellationToken,
) -> Result<Vec<SourceNode>, String> {
    let mut parent_item = None;
    for item in &inventory.items {
        check_navigation_cancellation(cancellation)?;
        if &item.address == parent {
            parent_item = Some(item);
            break;
        }
    }
    let Some(parent_item) = parent_item else {
        return Err(format!(
            "metadataPath `{}` was not found in sourceSet `{}`",
            parent.as_str(),
            inventory.source_set
        ));
    };
    if parent_item.target_kind == TargetKind::Module {
        return Ok(Vec::new());
    }
    if matches!(
        inventory.source_set_kind,
        SourceSetKind::ExternalProcessor | SourceSetKind::ExternalReport
    ) {
        return Ok(Vec::new());
    }

    let parent_parts = parent.segments().collect::<Vec<_>>();
    let mut collections = BTreeSet::new();
    let mut children = Vec::new();
    for item in &inventory.items {
        check_navigation_cancellation(cancellation)?;
        let parts = item.address.segments().collect::<Vec<_>>();
        if parts.len() <= parent_parts.len() || parts[..parent_parts.len()] != parent_parts[..] {
            continue;
        }
        if parts.len() == parent_parts.len() + 1 {
            children.push(addressed_node(inventory, item));
        } else if parts.len() >= parent_parts.len() + 2 {
            collections.insert(match parts[parent_parts.len()] {
                "Form" => "Forms".to_string(),
                "Command" => "Commands".to_string(),
                other => format!("{other}s"),
            });
        }
    }
    for collection in collections {
        check_navigation_cancellation(cancellation)?;
        children.push(SourceNode {
            display_name: collection,
            node_kind: SourceNodeKind::Collection,
            addressability: SourceNodeAddressability::Unaddressable,
            completeness: inventory.completeness,
            metadata_path: None,
            target_kind: None,
            location: None,
        });
    }
    Ok(children)
}

fn addressed_node(inventory: &NavigationInventory, item: &NavigableItem) -> SourceNode {
    SourceNode {
        display_name: item.display_name.clone(),
        node_kind: SourceNodeKind::Item,
        addressability: SourceNodeAddressability::Addressable,
        completeness: NavigationCompleteness::Complete,
        metadata_path: Some(item.address.clone()),
        target_kind: Some(item.target_kind),
        location: Some(addressed_location(
            &inventory.source_set,
            Some(item.address.clone()),
            item.target_kind,
        )),
    }
}

fn addressed_location(
    source_set: &str,
    metadata_path: Option<MetadataAddress>,
    target_kind: TargetKind,
) -> SourceLocation {
    SourceLocation::Addressed {
        source_set: source_set.to_string(),
        metadata_path,
        target_kind,
    }
}

#[derive(Debug)]
pub(crate) struct PlatformXmlResolution {
    pub(crate) resolved: ResolvedTarget,
    pub(crate) handle: ClosedPlatformXmlTarget,
}

#[derive(Clone)]
pub(crate) struct ClosedPlatformXmlTarget {
    source_target: SourceTarget,
    workspace_root: PathBuf,
    source_root_lexical: PathBuf,
    source_root: PathBuf,
    source_set_kind: SourceSetKind,
    source_format: SourceFormat,
    target_path: PathBuf,
}

impl fmt::Debug for ClosedPlatformXmlTarget {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ClosedPlatformXmlTarget")
            .field("source_target", &self.source_target)
            .field("source_set_kind", &self.source_set_kind)
            .field("source_format", &self.source_format)
            .field("physical_handle", &"<closed>")
            .finish()
    }
}

#[derive(Debug)]
pub(crate) struct RevalidatedPlatformXmlTarget {
    pub(crate) path: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PlatformXmlModuleIdentity {
    pub(crate) owner: String,
    pub(crate) address: MetadataAddress,
    pub(crate) role: PlatformXmlModuleRole,
    pub(crate) descriptors: Vec<PathBuf>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PlatformXmlModuleRole {
    Module,
    ObjectModule,
    ManagerModule,
    RecordSetModule,
    ValueManagerModule,
    FormModule,
    CommandModule,
    ManagedApplicationModule,
    OrdinaryApplicationModule,
    SessionModule,
    ExternalConnectionModule,
}

impl PlatformXmlModuleRole {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Module => "Module",
            Self::ObjectModule => "ObjectModule",
            Self::ManagerModule => "ManagerModule",
            Self::RecordSetModule => "RecordSetModule",
            Self::ValueManagerModule => "ValueManagerModule",
            Self::FormModule => "FormModule",
            Self::CommandModule => "CommandModule",
            Self::ManagedApplicationModule => "ManagedApplicationModule",
            Self::OrdinaryApplicationModule => "OrdinaryApplicationModule",
            Self::SessionModule => "SessionModule",
            Self::ExternalConnectionModule => "ExternalConnectionModule",
        }
    }
}

pub(crate) fn resolve_platform_xml_target(
    context: &WorkspaceContext,
    target: &SourceTarget,
) -> Result<PlatformXmlResolution, SourceTargetError> {
    if target.source_set.is_empty() {
        return Err(SourceTargetError::new(
            SourceTargetErrorCode::SourceSetRequired,
            "sourceSet must name an exact project source set",
        ));
    }
    let selected = resolve_named_source_set(context, &target.source_set)
        .map_err(|error| public_source_set_error(&target.source_set, error))?;
    validate_source_set(&selected)?;
    if target.metadata_path.is_none() {
        return resolve_platform_xml_root(context, target, selected);
    }
    let address = target.metadata_path.as_ref().expect("checked above");
    if address.target_kind() != TargetKind::Module {
        return Err(SourceTargetError::new(
            SourceTargetErrorCode::TargetKindMismatch,
            "metadataPath does not identify a module terminal",
        ));
    }

    let relative_path = module_path_for_address(address).map_err(|error| {
        SourceTargetError::new(SourceTargetErrorCode::MetadataAddressNotFound, error)
    })?;
    let identity = platform_xml_module_identity(&relative_path).map_err(|error| {
        SourceTargetError::new(SourceTargetErrorCode::MetadataAddressNotFound, error)
    })?;
    if &identity.address != address {
        return Err(SourceTargetError::new(
            SourceTargetErrorCode::MetadataAddressNotFound,
            "module layout did not round-trip to the requested metadata address",
        ));
    }
    validate_platform_xml_module_descriptors(context, &selected.path, &identity.descriptors)
        .map_err(|error| public_evidence_error(target, error))?;

    let target_path = selected.path.join(&relative_path);
    let target_path = WorkspacePathPolicy::new(context)
        .resolve_write(target_path)
        .map_err(|_| target_containment_error(&target.source_set))?;
    ensure_no_link_components(&selected.path, &target_path)
        .map_err(|_| target_containment_error(&target.source_set))?;
    validate_regular_module(&target_path, &target.source_set)?;
    let target_identity = normalize_path_identity(&target_path)
        .map_err(|_| target_containment_error(&target.source_set))?;
    if !target_identity.starts_with(&selected.path) {
        return Err(target_containment_error(&target.source_set));
    }

    let workspace_root = normalize_path_identity(&context.workspace_root)
        .map_err(|_| target_containment_error(&target.source_set))?;
    Ok(PlatformXmlResolution {
        resolved: ResolvedTarget {
            source_set: selected.source_set.name.clone(),
            metadata_path: Some(identity.address),
            target_kind: TargetKind::Module,
        },
        handle: ClosedPlatformXmlTarget {
            source_target: target.clone(),
            workspace_root,
            source_root_lexical: selected.lexical_path,
            source_root: selected.path,
            source_set_kind: selected.source_set.kind,
            source_format: selected.source_set.source_format,
            target_path,
        },
    })
}

fn resolve_platform_xml_root(
    context: &WorkspaceContext,
    target: &SourceTarget,
    selected: ResolvedNamedSourceSet,
) -> Result<PlatformXmlResolution, SourceTargetError> {
    let root_path = WorkspacePathPolicy::new(context)
        .resolve_write(selected.lexical_path.clone())
        .map_err(|_| source_root_containment_error(&target.source_set))?;
    let metadata = fs::symlink_metadata(&root_path)
        .map_err(|_| source_root_containment_error(&target.source_set))?;
    if metadata_is_link_or_reparse_point(&metadata) || !metadata.is_dir() {
        return Err(source_root_containment_error(&target.source_set));
    }
    let root_identity = normalize_path_identity(&root_path)
        .map_err(|_| source_root_containment_error(&target.source_set))?;
    if root_identity != selected.path {
        return Err(source_root_containment_error(&target.source_set));
    }
    let workspace_root = normalize_path_identity(&context.workspace_root)
        .map_err(|_| source_root_containment_error(&target.source_set))?;
    Ok(PlatformXmlResolution {
        resolved: ResolvedTarget {
            source_set: selected.source_set.name.clone(),
            metadata_path: None,
            target_kind: TargetKind::SourceRoot,
        },
        handle: ClosedPlatformXmlTarget {
            source_target: target.clone(),
            workspace_root,
            source_root_lexical: selected.lexical_path,
            source_root: selected.path.clone(),
            source_set_kind: selected.source_set.kind,
            source_format: selected.source_set.source_format,
            target_path: selected.path,
        },
    })
}

pub(crate) fn revalidate_platform_xml_target(
    context: &WorkspaceContext,
    handle: &ClosedPlatformXmlTarget,
) -> Result<RevalidatedPlatformXmlTarget, SourceTargetError> {
    let workspace_root = normalize_path_identity(&context.workspace_root)
        .map_err(|_| source_map_rebind_error(&handle.source_target.source_set))?;
    if workspace_root != handle.workspace_root {
        return Err(source_map_rebind_error(&handle.source_target.source_set));
    }
    let selected = resolve_named_source_set(context, &handle.source_target.source_set)
        .map_err(|_| source_map_rebind_error(&handle.source_target.source_set))?;
    if selected.lexical_path != handle.source_root_lexical
        || selected.path != handle.source_root
        || selected.source_set.kind != handle.source_set_kind
        || selected.source_set.source_format != handle.source_format
    {
        return Err(source_map_rebind_error(&handle.source_target.source_set));
    }

    let current = resolve_platform_xml_target(context, &handle.source_target)?;
    if current.handle.target_path != handle.target_path {
        return Err(source_map_rebind_error(&handle.source_target.source_set));
    }
    Ok(RevalidatedPlatformXmlTarget {
        path: current.handle.target_path,
    })
}

fn public_source_set_error(source_set: &str, error: NamedSourceSetError) -> SourceTargetError {
    match error.kind {
        NamedSourceSetErrorKind::NotFound => SourceTargetError::new(
            SourceTargetErrorCode::SourceSetNotFound,
            format!("sourceSet `{source_set}` was not found"),
        ),
        NamedSourceSetErrorKind::Containment => source_root_containment_error(source_set),
        NamedSourceSetErrorKind::Ambiguous => SourceTargetError::new(
            SourceTargetErrorCode::SourceRootNotAddressable,
            format!("sourceSet `{source_set}` is ambiguous"),
        ),
        NamedSourceSetErrorKind::Discovery => SourceTargetError::new(
            SourceTargetErrorCode::SourceRootNotAddressable,
            format!("sourceSet `{source_set}` is not addressable"),
        ),
    }
}

fn public_evidence_error(
    target: &SourceTarget,
    error: PlatformXmlEvidenceError,
) -> SourceTargetError {
    match error.kind {
        PlatformXmlEvidenceErrorKind::Containment => target_containment_error(&target.source_set),
        PlatformXmlEvidenceErrorKind::Unavailable | PlatformXmlEvidenceErrorKind::NotRegular => {
            SourceTargetError::new(
                SourceTargetErrorCode::MetadataAddressNotFound,
                format!(
                    "metadata owner evidence is unavailable for `{}` in sourceSet `{}`",
                    target
                        .metadata_path
                        .as_ref()
                        .map(MetadataAddress::as_str)
                        .unwrap_or("<root>"),
                    target.source_set
                ),
            )
        }
    }
}

fn target_containment_error(source_set: &str) -> SourceTargetError {
    SourceTargetError::new(
        SourceTargetErrorCode::ContainmentDenied,
        format!("resolved target containment was denied for sourceSet `{source_set}`"),
    )
}

fn source_root_containment_error(source_set: &str) -> SourceTargetError {
    SourceTargetError::new(
        SourceTargetErrorCode::ContainmentDenied,
        format!("selected source root containment was denied for sourceSet `{source_set}`"),
    )
}

fn source_map_rebind_error(source_set: &str) -> SourceTargetError {
    SourceTargetError::new(
        SourceTargetErrorCode::ContainmentDenied,
        format!("source-map binding changed for sourceSet `{source_set}`"),
    )
}

fn validate_source_set(selected: &ResolvedNamedSourceSet) -> Result<(), SourceTargetError> {
    if !matches!(
        selected.source_set.kind,
        SourceSetKind::Configuration | SourceSetKind::Extension
    ) || selected.source_set.source_format != SourceFormat::PlatformXml
    {
        return Err(SourceTargetError::new(
            SourceTargetErrorCode::SourceRootNotAddressable,
            format!(
                "source set `{}` must be a Platform XML Configuration or Extension",
                selected.source_set.name
            ),
        ));
    }
    Ok(())
}

fn validate_regular_module(path: &Path, source_set: &str) -> Result<(), SourceTargetError> {
    let metadata = fs::symlink_metadata(path).map_err(|_| {
        SourceTargetError::new(
            SourceTargetErrorCode::MetadataAddressNotFound,
            format!("module target is unavailable in sourceSet `{source_set}`"),
        )
    })?;
    if metadata_is_link_or_reparse_point(&metadata) {
        return Err(target_containment_error(source_set));
    }
    if !metadata.is_file() {
        return Err(SourceTargetError::new(
            SourceTargetErrorCode::MetadataAddressNotFound,
            format!("module target is not a regular file in sourceSet `{source_set}`"),
        ));
    }
    Ok(())
}

pub(crate) fn platform_xml_module_identity(
    relative: &Path,
) -> Result<PlatformXmlModuleIdentity, String> {
    module_layout_for_relative(relative).map(|(_, identity)| identity)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PlatformXmlModuleLayoutFamily {
    RootApplication,
    OwnerModule,
    CommonForm,
    CommonCommand,
    DirectMetadata,
    NestedForm,
    NestedCommand,
}

const OWNER_MODULE_KINDS: &[&str] = &[
    "CommonModule",
    "HTTPService",
    "WebService",
    "IntegrationService",
];

#[derive(Debug, Clone, Copy)]
enum ModuleLayoutToken {
    Literal(&'static str),
    MetadataKind,
    MetadataDirectory,
    OwnerName,
    ChildName,
    Role(ModuleRoleClass),
    RoleFile(ModuleRoleClass),
}

#[derive(Debug, Clone, Copy)]
enum ModuleRoleClass {
    Root,
    Direct,
}

#[derive(Debug, Clone, Copy)]
enum ModuleLayoutCapability {
    Any,
    OwnerModule,
    DirectModule,
    NestedFormOrCommand,
}

#[derive(Debug, Clone, Copy)]
enum ModuleDescriptorRule {
    Root,
    Common {
        kind: &'static str,
        directory: &'static str,
    },
    Owner,
    Nested {
        child_kind: &'static str,
        child_directory: &'static str,
    },
}

#[derive(Debug, Clone, Copy)]
struct PlatformXmlModuleLayoutDescriptor {
    family: PlatformXmlModuleLayoutFamily,
    physical: &'static [ModuleLayoutToken],
    logical: &'static [ModuleLayoutToken],
    capability: ModuleLayoutCapability,
    fixed_role: Option<PlatformXmlModuleRole>,
    descriptor_rule: ModuleDescriptorRule,
}

const PLATFORM_XML_MODULE_LAYOUT_FAMILIES: &[PlatformXmlModuleLayoutDescriptor] = &[
    PlatformXmlModuleLayoutDescriptor {
        family: PlatformXmlModuleLayoutFamily::RootApplication,
        physical: &[
            ModuleLayoutToken::Literal("Ext"),
            ModuleLayoutToken::RoleFile(ModuleRoleClass::Root),
        ],
        logical: &[ModuleLayoutToken::Role(ModuleRoleClass::Root)],
        capability: ModuleLayoutCapability::Any,
        fixed_role: None,
        descriptor_rule: ModuleDescriptorRule::Root,
    },
    PlatformXmlModuleLayoutDescriptor {
        family: PlatformXmlModuleLayoutFamily::OwnerModule,
        physical: &[
            ModuleLayoutToken::MetadataDirectory,
            ModuleLayoutToken::OwnerName,
            ModuleLayoutToken::Literal("Ext"),
            ModuleLayoutToken::Literal("Module.bsl"),
        ],
        logical: &[
            ModuleLayoutToken::MetadataKind,
            ModuleLayoutToken::OwnerName,
            ModuleLayoutToken::Literal("Module"),
        ],
        capability: ModuleLayoutCapability::OwnerModule,
        fixed_role: Some(PlatformXmlModuleRole::Module),
        descriptor_rule: ModuleDescriptorRule::Owner,
    },
    PlatformXmlModuleLayoutDescriptor {
        family: PlatformXmlModuleLayoutFamily::CommonForm,
        physical: &[
            ModuleLayoutToken::Literal("CommonForms"),
            ModuleLayoutToken::OwnerName,
            ModuleLayoutToken::Literal("Ext"),
            ModuleLayoutToken::Literal("Form"),
            ModuleLayoutToken::Literal("Module.bsl"),
        ],
        logical: &[
            ModuleLayoutToken::Literal("CommonForm"),
            ModuleLayoutToken::OwnerName,
            ModuleLayoutToken::Literal("FormModule"),
        ],
        capability: ModuleLayoutCapability::Any,
        fixed_role: Some(PlatformXmlModuleRole::FormModule),
        descriptor_rule: ModuleDescriptorRule::Common {
            kind: "CommonForm",
            directory: "CommonForms",
        },
    },
    PlatformXmlModuleLayoutDescriptor {
        family: PlatformXmlModuleLayoutFamily::CommonCommand,
        physical: &[
            ModuleLayoutToken::Literal("CommonCommands"),
            ModuleLayoutToken::OwnerName,
            ModuleLayoutToken::Literal("Ext"),
            ModuleLayoutToken::Literal("CommandModule.bsl"),
        ],
        logical: &[
            ModuleLayoutToken::Literal("CommonCommand"),
            ModuleLayoutToken::OwnerName,
            ModuleLayoutToken::Literal("CommandModule"),
        ],
        capability: ModuleLayoutCapability::Any,
        fixed_role: Some(PlatformXmlModuleRole::CommandModule),
        descriptor_rule: ModuleDescriptorRule::Common {
            kind: "CommonCommand",
            directory: "CommonCommands",
        },
    },
    PlatformXmlModuleLayoutDescriptor {
        family: PlatformXmlModuleLayoutFamily::DirectMetadata,
        physical: &[
            ModuleLayoutToken::MetadataDirectory,
            ModuleLayoutToken::OwnerName,
            ModuleLayoutToken::Literal("Ext"),
            ModuleLayoutToken::RoleFile(ModuleRoleClass::Direct),
        ],
        logical: &[
            ModuleLayoutToken::MetadataKind,
            ModuleLayoutToken::OwnerName,
            ModuleLayoutToken::Role(ModuleRoleClass::Direct),
        ],
        capability: ModuleLayoutCapability::DirectModule,
        fixed_role: None,
        descriptor_rule: ModuleDescriptorRule::Owner,
    },
    PlatformXmlModuleLayoutDescriptor {
        family: PlatformXmlModuleLayoutFamily::NestedForm,
        physical: &[
            ModuleLayoutToken::MetadataDirectory,
            ModuleLayoutToken::OwnerName,
            ModuleLayoutToken::Literal("Forms"),
            ModuleLayoutToken::ChildName,
            ModuleLayoutToken::Literal("Ext"),
            ModuleLayoutToken::Literal("Form"),
            ModuleLayoutToken::Literal("Module.bsl"),
        ],
        logical: &[
            ModuleLayoutToken::MetadataKind,
            ModuleLayoutToken::OwnerName,
            ModuleLayoutToken::Literal("Form"),
            ModuleLayoutToken::ChildName,
            ModuleLayoutToken::Literal("FormModule"),
        ],
        capability: ModuleLayoutCapability::NestedFormOrCommand,
        fixed_role: Some(PlatformXmlModuleRole::FormModule),
        descriptor_rule: ModuleDescriptorRule::Nested {
            child_kind: "Form",
            child_directory: "Forms",
        },
    },
    PlatformXmlModuleLayoutDescriptor {
        family: PlatformXmlModuleLayoutFamily::NestedCommand,
        physical: &[
            ModuleLayoutToken::MetadataDirectory,
            ModuleLayoutToken::OwnerName,
            ModuleLayoutToken::Literal("Commands"),
            ModuleLayoutToken::ChildName,
            ModuleLayoutToken::Literal("Ext"),
            ModuleLayoutToken::Literal("CommandModule.bsl"),
        ],
        logical: &[
            ModuleLayoutToken::MetadataKind,
            ModuleLayoutToken::OwnerName,
            ModuleLayoutToken::Literal("Command"),
            ModuleLayoutToken::ChildName,
            ModuleLayoutToken::Literal("CommandModule"),
        ],
        capability: ModuleLayoutCapability::NestedFormOrCommand,
        fixed_role: Some(PlatformXmlModuleRole::CommandModule),
        descriptor_rule: ModuleDescriptorRule::Nested {
            child_kind: "Command",
            child_directory: "Commands",
        },
    },
];

impl PlatformXmlModuleLayoutDescriptor {
    fn identity_from_parts(
        &self,
        parts: &[&str],
    ) -> Option<Result<PlatformXmlModuleIdentity, String>> {
        self.capture(self.physical, parts)
            .map(|captures| self.identity(&captures))
    }

    fn path_from_address(&self, parts: &[&str]) -> Option<Result<PathBuf, String>> {
        self.capture(self.logical, parts).map(|captures| {
            let mut path = PathBuf::new();
            for segment in self.render(self.physical, &captures) {
                path.push(segment);
            }
            Ok(path)
        })
    }

    fn capture<'a>(
        &self,
        template: &[ModuleLayoutToken],
        parts: &[&'a str],
    ) -> Option<ModuleLayoutCaptures<'a>> {
        if template.len() != parts.len() {
            return None;
        }
        let mut captures = ModuleLayoutCaptures::default();
        for (token, value) in template.iter().zip(parts.iter().copied()) {
            match token {
                ModuleLayoutToken::Literal(expected) if *expected == value => {}
                ModuleLayoutToken::Literal(_) => return None,
                ModuleLayoutToken::MetadataKind => {
                    captures.kind = Some(metadata_kind(value)?);
                }
                ModuleLayoutToken::MetadataDirectory => {
                    captures.kind = Some(metadata_kind_by_directory(value)?);
                }
                ModuleLayoutToken::OwnerName => captures.owner_name = Some(value),
                ModuleLayoutToken::ChildName => captures.child_name = Some(value),
                ModuleLayoutToken::Role(class) => {
                    captures.role = Some(class.parse(value)?);
                }
                ModuleLayoutToken::RoleFile(class) => {
                    captures.role = Some(class.parse(value.strip_suffix(".bsl")?)?);
                }
            }
        }
        self.accepts(&captures).then_some(captures)
    }

    fn accepts(&self, captures: &ModuleLayoutCaptures<'_>) -> bool {
        match self.capability {
            ModuleLayoutCapability::Any => true,
            ModuleLayoutCapability::OwnerModule => captures
                .kind
                .is_some_and(|kind| OWNER_MODULE_KINDS.contains(&kind.tag)),
            ModuleLayoutCapability::DirectModule => captures
                .kind
                .zip(captures.role)
                .is_some_and(|(kind, role)| supports_direct_module_role(kind.tag, role.as_str())),
            ModuleLayoutCapability::NestedFormOrCommand => captures
                .kind
                .is_some_and(|kind| supports_nested_form_or_command(kind.tag)),
        }
    }

    fn render(
        &self,
        template: &[ModuleLayoutToken],
        captures: &ModuleLayoutCaptures<'_>,
    ) -> Vec<String> {
        template
            .iter()
            .map(|token| match token {
                ModuleLayoutToken::Literal(value) => (*value).to_string(),
                ModuleLayoutToken::MetadataKind => captures
                    .kind
                    .expect("accepted layout must capture metadata kind")
                    .tag
                    .to_string(),
                ModuleLayoutToken::MetadataDirectory => captures
                    .kind
                    .expect("accepted layout must capture metadata kind")
                    .directory
                    .to_string(),
                ModuleLayoutToken::OwnerName => captures
                    .owner_name
                    .expect("accepted layout must capture owner name")
                    .to_string(),
                ModuleLayoutToken::ChildName => captures
                    .child_name
                    .expect("accepted layout must capture child name")
                    .to_string(),
                ModuleLayoutToken::Role(_) => captures
                    .role
                    .expect("accepted layout must capture module role")
                    .as_str()
                    .to_string(),
                ModuleLayoutToken::RoleFile(_) => format!(
                    "{}.bsl",
                    captures
                        .role
                        .expect("accepted layout must capture module role")
                        .as_str()
                ),
            })
            .collect()
    }

    fn identity(
        &self,
        captures: &ModuleLayoutCaptures<'_>,
    ) -> Result<PlatformXmlModuleIdentity, String> {
        let address = self.render(self.logical, captures).join(".");
        let role = self
            .fixed_role
            .or(captures.role)
            .ok_or_else(unsupported_module_layout)?;
        let (owner, descriptors) = match self.descriptor_rule {
            ModuleDescriptorRule::Root => (
                "Configuration".to_string(),
                vec![PathBuf::from("Configuration.xml")],
            ),
            ModuleDescriptorRule::Common { kind, directory } => {
                let name = captures.owner_name.ok_or_else(unsupported_module_layout)?;
                (
                    format!("{kind}.{name}"),
                    vec![metadata_descriptor(directory, name)],
                )
            }
            ModuleDescriptorRule::Owner => {
                let kind = captures.kind.ok_or_else(unsupported_module_layout)?;
                let name = captures.owner_name.ok_or_else(unsupported_module_layout)?;
                (
                    format!("{}.{name}", kind.tag),
                    vec![metadata_descriptor(kind.directory, name)],
                )
            }
            ModuleDescriptorRule::Nested {
                child_kind,
                child_directory,
            } => {
                let kind = captures.kind.ok_or_else(unsupported_module_layout)?;
                let name = captures.owner_name.ok_or_else(unsupported_module_layout)?;
                let child_name = captures.child_name.ok_or_else(unsupported_module_layout)?;
                (
                    format!("{}.{name}", kind.tag),
                    vec![
                        metadata_descriptor(kind.directory, name),
                        PathBuf::from(kind.directory)
                            .join(name)
                            .join(child_directory)
                            .join(child_name)
                            .join("Ext")
                            .join(format!("{child_kind}.xml")),
                    ],
                )
            }
        };
        module_identity(address, owner, role, descriptors)
    }

    #[cfg(test)]
    fn family(&self) -> PlatformXmlModuleLayoutFamily {
        self.family
    }

    #[cfg(test)]
    fn render_physical(&self, address: &MetadataAddress) -> Result<PathBuf, String> {
        let parts = address.segments().collect::<Vec<_>>();
        self.path_from_address(&parts)
            .ok_or_else(unsupported_module_layout)?
    }

    #[cfg(test)]
    fn parse_physical(&self, relative: &Path) -> Result<MetadataAddress, String> {
        let components = relative_module_path_components(relative)?;
        let parts = components.iter().map(String::as_str).collect::<Vec<_>>();
        self.identity_from_parts(&parts)
            .ok_or_else(unsupported_module_layout)?
            .map(|identity| identity.address)
    }
}

#[derive(Debug, Default)]
struct ModuleLayoutCaptures<'a> {
    kind: Option<&'static MetadataKind>,
    owner_name: Option<&'a str>,
    child_name: Option<&'a str>,
    role: Option<PlatformXmlModuleRole>,
}

impl ModuleRoleClass {
    fn parse(self, raw: &str) -> Option<PlatformXmlModuleRole> {
        let role = match raw {
            "ObjectModule" => PlatformXmlModuleRole::ObjectModule,
            "ManagerModule" => PlatformXmlModuleRole::ManagerModule,
            "RecordSetModule" => PlatformXmlModuleRole::RecordSetModule,
            "ValueManagerModule" => PlatformXmlModuleRole::ValueManagerModule,
            "ManagedApplicationModule" => PlatformXmlModuleRole::ManagedApplicationModule,
            "OrdinaryApplicationModule" => PlatformXmlModuleRole::OrdinaryApplicationModule,
            "SessionModule" => PlatformXmlModuleRole::SessionModule,
            "ExternalConnectionModule" => PlatformXmlModuleRole::ExternalConnectionModule,
            _ => return None,
        };
        match self {
            Self::Root
                if matches!(
                    role,
                    PlatformXmlModuleRole::ManagedApplicationModule
                        | PlatformXmlModuleRole::OrdinaryApplicationModule
                        | PlatformXmlModuleRole::SessionModule
                        | PlatformXmlModuleRole::ExternalConnectionModule
                ) =>
            {
                Some(role)
            }
            Self::Direct
                if matches!(
                    role,
                    PlatformXmlModuleRole::ObjectModule
                        | PlatformXmlModuleRole::ManagerModule
                        | PlatformXmlModuleRole::RecordSetModule
                        | PlatformXmlModuleRole::ValueManagerModule
                ) =>
            {
                Some(role)
            }
            _ => None,
        }
    }
}

fn module_layout_for_relative(
    relative: &Path,
) -> Result<(PlatformXmlModuleLayoutFamily, PlatformXmlModuleIdentity), String> {
    let components = relative_module_path_components(relative)?;
    let parts = components.iter().map(String::as_str).collect::<Vec<_>>();
    for family in PLATFORM_XML_MODULE_LAYOUT_FAMILIES {
        if let Some(identity) = family.identity_from_parts(&parts) {
            return identity.map(|identity| (family.family, identity));
        }
    }
    Err(unsupported_module_layout())
}

fn relative_module_path_components(relative: &Path) -> Result<Vec<String>, String> {
    relative
        .components()
        .map(|component| match component {
            Component::Normal(value) => value
                .to_str()
                .map(str::to_string)
                .ok_or_else(|| "BSL module path is not valid UTF-8".to_string()),
            _ => Err("BSL module path must be relative to its source set".to_string()),
        })
        .collect()
}

#[cfg(test)]
fn module_layout_descriptors_for_test() -> &'static [PlatformXmlModuleLayoutDescriptor] {
    PLATFORM_XML_MODULE_LAYOUT_FAMILIES
}

fn module_identity(
    address: String,
    owner: String,
    role: PlatformXmlModuleRole,
    descriptors: Vec<PathBuf>,
) -> Result<PlatformXmlModuleIdentity, String> {
    let address = MetadataAddress::parse(
        crate::domain::source_target::PLATFORM_XML_8_3_27_FORMAT_2_20,
        &address,
    )
    .map_err(|error| error.to_string())?;
    Ok(PlatformXmlModuleIdentity {
        owner,
        address,
        role,
        descriptors,
    })
}

fn module_path_for_address(address: &MetadataAddress) -> Result<PathBuf, String> {
    let parts = address.segments().collect::<Vec<_>>();
    for family in PLATFORM_XML_MODULE_LAYOUT_FAMILIES {
        if let Some(path) = family.path_from_address(&parts) {
            return path;
        }
    }
    Err(unsupported_module_layout())
}

fn metadata_descriptor(directory: &str, name: &str) -> PathBuf {
    PathBuf::from(directory).join(format!("{name}.xml"))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PlatformXmlEvidenceErrorKind {
    Containment,
    Unavailable,
    NotRegular,
}

#[derive(Debug)]
pub(crate) struct PlatformXmlEvidenceError {
    kind: PlatformXmlEvidenceErrorKind,
    private_diagnostic: String,
}

impl PlatformXmlEvidenceError {
    fn new(kind: PlatformXmlEvidenceErrorKind, private_diagnostic: impl Into<String>) -> Self {
        Self {
            kind,
            private_diagnostic: private_diagnostic.into(),
        }
    }
}

impl fmt::Display for PlatformXmlEvidenceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.private_diagnostic)
    }
}

impl std::error::Error for PlatformXmlEvidenceError {}

pub(crate) fn validate_platform_xml_module_descriptors(
    context: &WorkspaceContext,
    source_root: &Path,
    descriptors: &[PathBuf],
) -> Result<(), PlatformXmlEvidenceError> {
    for descriptor in descriptors {
        let path = WorkspacePathPolicy::new(context)
            .resolve_write(source_root.join(descriptor))
            .map_err(|error| {
                PlatformXmlEvidenceError::new(
                    PlatformXmlEvidenceErrorKind::Containment,
                    format!("BSL module descriptor containment denied: {error}"),
                )
            })?;
        ensure_no_link_components(source_root, &path).map_err(|error| {
            PlatformXmlEvidenceError::new(
                PlatformXmlEvidenceErrorKind::Containment,
                format!("BSL module descriptor containment denied: {error}"),
            )
        })?;
        let metadata = fs::symlink_metadata(&path).map_err(|error| {
            PlatformXmlEvidenceError::new(
                PlatformXmlEvidenceErrorKind::Unavailable,
                format!(
                    "BSL module metadata descriptor is unavailable {}: {error}",
                    path.display()
                ),
            )
        })?;
        if metadata_is_link_or_reparse_point(&metadata) || !metadata.is_file() {
            return Err(PlatformXmlEvidenceError::new(
                PlatformXmlEvidenceErrorKind::NotRegular,
                format!(
                    "BSL module metadata descriptor must be a regular file: {}",
                    path.display()
                ),
            ));
        }
    }
    Ok(())
}

fn ensure_no_link_components(source_root: &Path, path: &Path) -> Result<(), String> {
    let relative = path
        .strip_prefix(source_root)
        .map_err(|_| "path is outside its selected source set".to_string())?;
    let mut current = source_root.to_path_buf();
    for component in relative.components() {
        current.push(component.as_os_str());
        let metadata = match fs::symlink_metadata(&current) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => break,
            Err(error) => {
                return Err(format!(
                    "failed to inspect path component {}: {error}",
                    current.display()
                ));
            }
        };
        if metadata_is_link_or_reparse_point(&metadata) {
            return Err(format!(
                "path component must not be a symbolic link or reparse point: {}",
                current.display()
            ));
        }
    }
    Ok(())
}

fn unsupported_module_layout() -> String {
    "unica.code.patch v1 accepts only a supported canonical platform XML BSL module path"
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::{
        children_platform_xml_source_navigation as children_platform_xml_source_navigation_cancellable,
        resolve_platform_xml_source_navigation as resolve_platform_xml_source_navigation_cancellable,
        resolve_platform_xml_target, revalidate_platform_xml_target,
        set_navigation_provider_entry_hook_for_test,
    };
    use crate::application::source_navigation::{
        NavigationCompleteness, SourceChildrenRequest, SourceLocation, SourceMatchKind,
        SourceNavigationMode, SourceNodeAddressability, SourceNodeKind, SourceResolveRequest,
    };
    use crate::domain::cancellation::{CancellationToken, CANCELLED_PREFIX};
    use crate::domain::source_target::{
        MetadataAddress, SourceTarget, SourceTargetErrorCode, PLATFORM_XML_8_3_27_FORMAT_2_20,
    };
    use crate::domain::workspace::WorkspaceContext;
    use crate::infrastructure::platform::filesystem::create_dir_symlink_for_test;
    use crate::infrastructure::platform::testing::{
        create_file_link_fixture_for_test, FileLinkFixtureOutcome,
    };
    use crate::infrastructure::source_roots::normalize_path_identity;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    static TEMP_NONCE: AtomicU64 = AtomicU64::new(0);

    fn resolve_platform_xml_source_navigation(
        context: &WorkspaceContext,
        request: &SourceResolveRequest,
    ) -> Result<crate::application::source_navigation::SourceResolveResult, String> {
        resolve_platform_xml_source_navigation_cancellable(
            context,
            request,
            &CancellationToken::new(),
        )
    }

    fn children_platform_xml_source_navigation(
        context: &WorkspaceContext,
        request: &SourceChildrenRequest,
    ) -> Result<crate::application::source_navigation::SourceChildrenResult, String> {
        children_platform_xml_source_navigation_cancellable(
            context,
            request,
            &CancellationToken::new(),
        )
    }

    #[test]
    fn platform_xml_source_targets_resolve_identical_addresses_in_configuration_and_extension() {
        let context = fixture(
            "config-extension",
            "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: cfg\n  - name: addOn\n    type: EXTENSION\n    path: ext\n",
        );
        for root in ["cfg", "ext"] {
            write_module_fixture(
                &context.workspace_root.join(root),
                "CommonModules/Shared.xml",
                "CommonModules/Shared/Ext/Module.bsl",
            );
        }

        let configuration =
            resolve_platform_xml_target(&context, &target("main", "CommonModule.Shared.Module"))
                .unwrap();
        let extension =
            resolve_platform_xml_target(&context, &target("addOn", "CommonModule.Shared.Module"))
                .unwrap();

        assert_eq!(
            configuration
                .resolved
                .metadata_path
                .as_ref()
                .unwrap()
                .as_str(),
            extension.resolved.metadata_path.as_ref().unwrap().as_str()
        );
        assert_eq!(
            configuration.handle.target_path,
            normalize_path_identity(
                &context
                    .workspace_root
                    .join("cfg/CommonModules/Shared/Ext/Module.bsl")
            )
            .unwrap()
        );
        assert_eq!(
            extension.handle.target_path,
            normalize_path_identity(
                &context
                    .workspace_root
                    .join("ext/CommonModules/Shared/Ext/Module.bsl")
            )
            .unwrap()
        );
        cleanup(&context);
    }

    #[test]
    fn platform_xml_source_target_handle_debug_does_not_disclose_physical_paths() {
        let context = fixture("closed-debug", project_yaml("main", "CONFIGURATION", "src"));
        write_module_fixture(
            &context.workspace_root.join("src"),
            "CommonModules/Shared.xml",
            "CommonModules/Shared/Ext/Module.bsl",
        );
        let resolution =
            resolve_platform_xml_target(&context, &target("main", "CommonModule.Shared.Module"))
                .unwrap();

        let debug = format!("{:?}", resolution.handle);

        assert!(!debug.contains(&context.workspace_root.display().to_string()));
        assert!(!debug.contains("Module.bsl"));
        cleanup(&context);
    }

    #[test]
    fn platform_xml_source_targets_reject_wrong_source_set_kind_and_format() {
        let context = fixture(
            "wrong-kind-format",
            "source-set:\n  - name: external\n    type: EXTERNAL_DATA_PROCESSORS\n    path: external\n  - name: edt\n    type: CONFIGURATION\n    path: edt\n",
        );
        fs::create_dir_all(context.workspace_root.join("external")).unwrap();
        fs::create_dir_all(context.workspace_root.join("edt/Configuration")).unwrap();
        fs::write(context.workspace_root.join("edt/.project"), "edt").unwrap();

        for source_set in ["external", "edt"] {
            let error = resolve_platform_xml_target(
                &context,
                &target(source_set, "CommonModule.Shared.Module"),
            )
            .unwrap_err();
            assert_eq!(
                error.code,
                SourceTargetErrorCode::SourceRootNotAddressable,
                "{source_set}: {error}"
            );
        }
        cleanup(&context);
    }

    #[test]
    fn platform_xml_source_targets_require_descriptor_evidence() {
        let context = fixture(
            "missing-descriptor",
            project_yaml("main", "CONFIGURATION", "src"),
        );
        let module = context
            .workspace_root
            .join("src/CommonModules/Missing/Ext/Module.bsl");
        fs::create_dir_all(module.parent().unwrap()).unwrap();
        fs::write(&module, "Procedure Run()\nEndProcedure\n").unwrap();

        let error =
            resolve_platform_xml_target(&context, &target("main", "CommonModule.Missing.Module"))
                .unwrap_err();

        assert_eq!(error.code, SourceTargetErrorCode::MetadataAddressNotFound);
        assert_eq!(
            error.message,
            "metadata owner evidence is unavailable for `CommonModule.Missing.Module` in sourceSet `main`"
        );
        let serialized = serde_json::to_string(&error).unwrap();
        assert!(!serialized.contains(&context.workspace_root.display().to_string()));
        assert!(!serialized.contains("CommonModules/Missing.xml"));
        cleanup(&context);
    }

    #[test]
    fn platform_xml_source_targets_serialize_missing_modules_without_io_details() {
        let context = fixture(
            "missing-module",
            project_yaml("main", "CONFIGURATION", "src"),
        );
        let descriptor = context.workspace_root.join("src/CommonModules/Missing.xml");
        fs::create_dir_all(descriptor.parent().unwrap()).unwrap();
        fs::write(&descriptor, "<MetaDataObject/>").unwrap();

        let error =
            resolve_platform_xml_target(&context, &target("main", "CommonModule.Missing.Module"))
                .unwrap_err();

        assert_eq!(error.code, SourceTargetErrorCode::MetadataAddressNotFound);
        assert_eq!(
            error.message,
            "module target is unavailable in sourceSet `main`"
        );
        let serialized = serde_json::to_string(&error).unwrap();
        assert!(!serialized.contains(&context.workspace_root.display().to_string()));
        assert!(!serialized.contains("os error"));
        cleanup(&context);
    }

    #[test]
    fn platform_xml_source_targets_never_fall_back_from_the_named_source_set() {
        let context = fixture("exact-name", project_yaml("main", "CONFIGURATION", "src"));
        write_module_fixture(
            &context.workspace_root.join("src"),
            "CommonModules/Shared.xml",
            "CommonModules/Shared/Ext/Module.bsl",
        );

        let error =
            resolve_platform_xml_target(&context, &target("missing", "CommonModule.Shared.Module"))
                .unwrap_err();

        assert_eq!(error.code, SourceTargetErrorCode::SourceSetNotFound);
        cleanup(&context);
    }

    #[test]
    fn platform_xml_source_targets_reject_duplicate_exact_source_set_names() {
        let context = fixture(
            "duplicate-name",
            "format: DESIGNER\nsource-set:\n  - name: duplicate\n    type: CONFIGURATION\n    path: cfg\n  - name: duplicate\n    type: EXTENSION\n    path: ext\n",
        );
        for root in ["cfg", "ext"] {
            write_module_fixture(
                &context.workspace_root.join(root),
                "CommonModules/Shared.xml",
                "CommonModules/Shared/Ext/Module.bsl",
            );
        }

        let error = resolve_platform_xml_target(
            &context,
            &target("duplicate", "CommonModule.Shared.Module"),
        )
        .unwrap_err();

        assert_eq!(error.code, SourceTargetErrorCode::SourceRootNotAddressable);
        assert!(error.message.contains("ambiguous"));
        cleanup(&context);
    }

    #[test]
    fn platform_xml_source_targets_cover_every_registered_module_layout_family() {
        let context = fixture(
            "layout-families",
            project_yaml("main", "CONFIGURATION", "src"),
        );
        let root = context.workspace_root.join("src");
        for descriptor in [
            "Configuration.xml",
            "CommonModules/Shared.xml",
            "Catalogs/Items.xml",
            "InformationRegisters/Prices.xml",
            "Constants/Mode.xml",
            "CommonForms/Main.xml",
            "CommonCommands/Print.xml",
            "Catalogs/Items/Forms/List/Ext/Form.xml",
            "Catalogs/Items/Commands/Open/Ext/Command.xml",
        ] {
            let path = root.join(descriptor);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, "<MetaDataObject/>").unwrap();
        }
        let cases = [
            (
                "ManagedApplicationModule",
                "Ext/ManagedApplicationModule.bsl",
            ),
            (
                "CommonModule.Shared.Module",
                "CommonModules/Shared/Ext/Module.bsl",
            ),
            (
                "Catalog.Items.ObjectModule",
                "Catalogs/Items/Ext/ObjectModule.bsl",
            ),
            (
                "Catalog.Items.ManagerModule",
                "Catalogs/Items/Ext/ManagerModule.bsl",
            ),
            (
                "InformationRegister.Prices.RecordSetModule",
                "InformationRegisters/Prices/Ext/RecordSetModule.bsl",
            ),
            (
                "Constant.Mode.ValueManagerModule",
                "Constants/Mode/Ext/ValueManagerModule.bsl",
            ),
            (
                "CommonForm.Main.FormModule",
                "CommonForms/Main/Ext/Form/Module.bsl",
            ),
            (
                "CommonCommand.Print.CommandModule",
                "CommonCommands/Print/Ext/CommandModule.bsl",
            ),
            (
                "Catalog.Items.Form.List.FormModule",
                "Catalogs/Items/Forms/List/Ext/Form/Module.bsl",
            ),
            (
                "Catalog.Items.Command.Open.CommandModule",
                "Catalogs/Items/Commands/Open/Ext/CommandModule.bsl",
            ),
        ];
        for (address, relative) in cases {
            let module = root.join(relative);
            fs::create_dir_all(module.parent().unwrap()).unwrap();
            fs::write(&module, "Procedure Run()\nEndProcedure\n").unwrap();

            let resolution =
                resolve_platform_xml_target(&context, &target("main", address)).unwrap();

            assert_eq!(
                resolution.handle.target_path,
                normalize_path_identity(&module).unwrap(),
                "{address}"
            );
            assert_eq!(
                resolution.resolved.metadata_path.unwrap().as_str(),
                address,
                "{relative}"
            );
        }
        cleanup(&context);
    }

    #[test]
    fn platform_xml_module_layouts_share_one_family_registry_in_both_directions() {
        let cases = [
            (
                super::PlatformXmlModuleLayoutFamily::RootApplication,
                "ManagedApplicationModule",
                "Ext/ManagedApplicationModule.bsl",
            ),
            (
                super::PlatformXmlModuleLayoutFamily::OwnerModule,
                "CommonModule.Shared.Module",
                "CommonModules/Shared/Ext/Module.bsl",
            ),
            (
                super::PlatformXmlModuleLayoutFamily::CommonForm,
                "CommonForm.Main.FormModule",
                "CommonForms/Main/Ext/Form/Module.bsl",
            ),
            (
                super::PlatformXmlModuleLayoutFamily::CommonCommand,
                "CommonCommand.Print.CommandModule",
                "CommonCommands/Print/Ext/CommandModule.bsl",
            ),
            (
                super::PlatformXmlModuleLayoutFamily::DirectMetadata,
                "Catalog.Items.ManagerModule",
                "Catalogs/Items/Ext/ManagerModule.bsl",
            ),
            (
                super::PlatformXmlModuleLayoutFamily::NestedForm,
                "Catalog.Items.Form.List.FormModule",
                "Catalogs/Items/Forms/List/Ext/Form/Module.bsl",
            ),
            (
                super::PlatformXmlModuleLayoutFamily::NestedCommand,
                "Catalog.Items.Command.Open.CommandModule",
                "Catalogs/Items/Commands/Open/Ext/CommandModule.bsl",
            ),
        ];

        let descriptors = super::module_layout_descriptors_for_test();
        assert_eq!(descriptors.len(), cases.len());
        for ((expected_family, address, relative), descriptor) in cases.into_iter().zip(descriptors)
        {
            let address = MetadataAddress::parse(PLATFORM_XML_8_3_27_FORMAT_2_20, address).unwrap();

            assert_eq!(descriptor.family(), expected_family);
            assert_eq!(
                descriptor.render_physical(&address).unwrap(),
                Path::new(relative)
            );
            assert_eq!(
                descriptor.parse_physical(Path::new(relative)).unwrap(),
                address
            );
        }
    }

    #[test]
    fn platform_xml_source_targets_reject_unregistered_module_roles() {
        let context = fixture(
            "unregistered-role",
            project_yaml("main", "CONFIGURATION", "src"),
        );
        write_module_fixture(
            &context.workspace_root.join("src"),
            "Languages/Russian.xml",
            "Languages/Russian/Ext/ManagerModule.bsl",
        );

        let error = resolve_platform_xml_target(
            &context,
            &target("main", "Language.Russian.ManagerModule"),
        )
        .unwrap_err();

        assert_eq!(error.code, SourceTargetErrorCode::MetadataAddressNotFound);
        cleanup(&context);
    }

    #[test]
    fn platform_xml_source_targets_revalidation_rejects_a_replaced_symlink_target() {
        let context = fixture(
            "revalidate-symlink",
            project_yaml("main", "CONFIGURATION", "src"),
        );
        let root = context.workspace_root.join("src");
        write_module_fixture(
            &root,
            "CommonModules/Shared.xml",
            "CommonModules/Shared/Ext/Module.bsl",
        );
        let resolution =
            resolve_platform_xml_target(&context, &target("main", "CommonModule.Shared.Module"))
                .unwrap();
        let target = root.join("CommonModules/Shared/Ext/Module.bsl");
        let real = root.join("CommonModules/Shared/Ext/Replacement.bsl");
        fs::write(&real, "Procedure Changed()\nEndProcedure\n").unwrap();
        fs::remove_file(&target).unwrap();
        let outcome = create_file_link_fixture_for_test(&real, &target)
            .expect("unexpected file-link creation error must fail the fixture test");
        if outcome != FileLinkFixtureOutcome::Created {
            cleanup(&context);
            return;
        }

        let error = revalidate_platform_xml_target(&context, &resolution.handle).unwrap_err();

        assert_eq!(error.code, SourceTargetErrorCode::ContainmentDenied);
        cleanup(&context);
    }

    #[test]
    fn platform_xml_source_targets_reject_symlinked_target() {
        let context = fixture(
            "symlink-target",
            project_yaml("main", "CONFIGURATION", "src"),
        );
        let root = context.workspace_root.join("src");
        fs::create_dir_all(root.join("CommonModules/Shared/Ext")).unwrap();
        fs::write(root.join("CommonModules/Shared.xml"), "<MetaDataObject/>").unwrap();
        let real = root.join("CommonModules/Shared/Ext/RealModule.bsl");
        let target = root.join("CommonModules/Shared/Ext/Module.bsl");
        fs::write(&real, "Procedure Run()\nEndProcedure\n").unwrap();
        let outcome = create_file_link_fixture_for_test(&real, &target)
            .expect("unexpected file-link creation error must fail the fixture test");
        if outcome != FileLinkFixtureOutcome::Created {
            cleanup(&context);
            return;
        }

        let error = resolve_platform_xml_target(
            &context,
            &target_for("main", "CommonModule.Shared.Module"),
        )
        .unwrap_err();

        assert_eq!(error.code, SourceTargetErrorCode::ContainmentDenied);
        cleanup(&context);
    }

    #[test]
    fn platform_xml_source_targets_reject_symlinked_layout_ancestor() {
        let context = fixture(
            "symlink-ancestor",
            project_yaml("main", "CONFIGURATION", "src"),
        );
        let root = context.workspace_root.join("src");
        write_module_fixture(
            &root,
            "RealCommonModules/Shared.xml",
            "RealCommonModules/Shared/Ext/Module.bsl",
        );
        let Some(result) =
            create_dir_symlink_for_test(root.join("RealCommonModules"), root.join("CommonModules"))
        else {
            cleanup(&context);
            return;
        };
        result.unwrap();

        let error =
            resolve_platform_xml_target(&context, &target("main", "CommonModule.Shared.Module"))
                .unwrap_err();

        assert_eq!(error.code, SourceTargetErrorCode::ContainmentDenied);
        assert_eq!(
            error.message,
            "resolved target containment was denied for sourceSet `main`"
        );
        let serialized = serde_json::to_string(&error).unwrap();
        assert!(!serialized.contains(&context.workspace_root.display().to_string()));
        assert!(!serialized.contains("CommonModules"));
        cleanup(&context);
    }

    #[test]
    fn platform_xml_source_targets_resolve_and_revalidate_configuration_and_extension_roots() {
        let context = fixture(
            "source-roots",
            "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: cfg\n  - name: addOn\n    type: EXTENSION\n    path: ext\n",
        );
        for root in ["cfg", "ext"] {
            fs::create_dir_all(context.workspace_root.join(root)).unwrap();
        }

        for (source_set, root) in [("main", "cfg"), ("addOn", "ext")] {
            let resolution =
                resolve_platform_xml_target(&context, &root_target(source_set)).unwrap();

            assert_eq!(resolution.resolved.source_set, source_set);
            assert_eq!(resolution.resolved.metadata_path, None);
            assert_eq!(
                resolution.resolved.target_kind,
                crate::domain::source_target::TargetKind::SourceRoot
            );
            assert_eq!(
                revalidate_platform_xml_target(&context, &resolution.handle)
                    .unwrap()
                    .path,
                normalize_path_identity(&context.workspace_root.join(root)).unwrap()
            );
        }
        cleanup(&context);
    }

    #[test]
    fn platform_xml_source_targets_reject_a_configured_source_root_symlink() {
        let context = fixture(
            "symlink-source-root",
            project_yaml("main", "CONFIGURATION", "src"),
        );
        let real = context.workspace_root.join("real-src");
        fs::create_dir_all(&real).unwrap();
        fs::write(real.join("Configuration.xml"), "<MetaDataObject/>").unwrap();
        let Some(result) = create_dir_symlink_for_test(&real, context.workspace_root.join("src"))
        else {
            cleanup(&context);
            return;
        };
        result.unwrap();

        let error = resolve_platform_xml_target(&context, &root_target("main")).unwrap_err();

        assert_eq!(error.code, SourceTargetErrorCode::ContainmentDenied);
        assert_eq!(
            error.message,
            "selected source root containment was denied for sourceSet `main`"
        );
        let serialized = serde_json::to_string(&error).unwrap();
        assert!(!serialized.contains(&context.workspace_root.display().to_string()));
        assert!(!serialized.contains("real-src"));
        cleanup(&context);
    }

    #[test]
    fn platform_xml_source_targets_reject_a_source_root_replaced_by_a_symlink() {
        let context = fixture(
            "revalidate-source-root-symlink",
            project_yaml("main", "CONFIGURATION", "src"),
        );
        let root = context.workspace_root.join("src");
        fs::create_dir_all(&root).unwrap();
        let resolution = resolve_platform_xml_target(&context, &root_target("main")).unwrap();
        let replacement = context.workspace_root.join("replacement");
        fs::rename(&root, &replacement).unwrap();
        let Some(result) = create_dir_symlink_for_test(&replacement, &root) else {
            cleanup(&context);
            return;
        };
        result.unwrap();

        let error = revalidate_platform_xml_target(&context, &resolution.handle).unwrap_err();

        assert_eq!(error.code, SourceTargetErrorCode::ContainmentDenied);
        assert_eq!(
            error.message,
            "source-map binding changed for sourceSet `main`"
        );
        let serialized = serde_json::to_string(&error).unwrap();
        assert!(!serialized.contains(&context.workspace_root.display().to_string()));
        assert!(!serialized.contains("replacement"));
        cleanup(&context);
    }

    #[test]
    fn platform_xml_source_targets_revalidate_source_map_binding() {
        let context = fixture(
            "source-map-rebind",
            project_yaml("main", "EXTENSION", "ext"),
        );
        write_module_fixture(
            &context.workspace_root.join("ext"),
            "CommonModules/Shared.xml",
            "CommonModules/Shared/Ext/Module.bsl",
        );
        let resolution =
            resolve_platform_xml_target(&context, &target("main", "CommonModule.Shared.Module"))
                .unwrap();
        fs::write(
            context.workspace_root.join("v8project.yaml"),
            project_yaml("main", "EXTENSION", "other"),
        )
        .unwrap();
        fs::create_dir_all(context.workspace_root.join("other")).unwrap();

        let error = revalidate_platform_xml_target(&context, &resolution.handle).unwrap_err();

        assert_eq!(error.code, SourceTargetErrorCode::ContainmentDenied);
        assert!(error.message.contains("source-map"));
        cleanup(&context);
    }

    #[test]
    fn source_navigation_resolves_exact_prefix_russian_aliases_for_configuration_and_extension() {
        let context = fixture(
            "navigation-config-extension",
            "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: cfg\n  - name: addOn\n    type: EXTENSION\n    path: ext\n",
        );
        for root in ["cfg", "ext"] {
            write_configuration_descriptor(&context.workspace_root.join(root), root == "ext");
            write_real_module_fixture(&context.workspace_root.join(root), "CommonModule", "Shared");
            write_real_module_fixture(
                &context.workspace_root.join(root),
                "CommonModule",
                "Shipping",
            );
        }

        for source_set in ["main", "addOn"] {
            let exact = resolve_platform_xml_source_navigation(
                &context,
                &SourceResolveRequest {
                    source_set: source_set.to_string(),
                    query: "ОбщийМодуль.Shared.Module".to_string(),
                    mode: SourceNavigationMode::Exact,
                    target_kind: None,
                    limit: 10,
                    cursor: None,
                },
            )
            .unwrap();
            assert_eq!(exact.completeness, NavigationCompleteness::Complete);
            assert_eq!(exact.candidates.len(), 1);
            assert_eq!(
                exact.candidates[0].metadata_path.as_str(),
                "CommonModule.Shared.Module"
            );
            assert_eq!(exact.candidates[0].match_kind, SourceMatchKind::Exact);

            let prefix = resolve_platform_xml_source_navigation(
                &context,
                &SourceResolveRequest {
                    source_set: source_set.to_string(),
                    query: "ОбщийМодуль.Sh".to_string(),
                    mode: SourceNavigationMode::Prefix,
                    target_kind: Some(crate::domain::source_target::TargetKind::Module),
                    limit: 10,
                    cursor: None,
                },
            )
            .unwrap();
            assert_eq!(
                prefix
                    .candidates
                    .iter()
                    .map(|candidate| candidate.metadata_path.as_str())
                    .collect::<Vec<_>>(),
                vec!["CommonModule.Shared.Module", "CommonModule.Shipping.Module"],
                "prefix ambiguity must be returned, never heuristically selected"
            );
            assert!(prefix
                .candidates
                .iter()
                .all(|candidate| candidate.match_kind == SourceMatchKind::Prefix));
        }
        cleanup(&context);
    }

    #[test]
    fn source_navigation_rejects_nested_modules_with_mismatched_child_descriptor_identity() {
        let context = fixture(
            "navigation-nested-child-identity",
            project_yaml("main", "CONFIGURATION", "src"),
        );
        let root = context.workspace_root.join("src");
        write_configuration_descriptor(&root, false);
        write_metadata_descriptor(&root, "Catalogs", "Catalog", "Items", "Items");
        write_nested_module_fixture(
            &root,
            "Catalogs/Items/Forms/Order/Ext/Form/Module.bsl",
            "Catalogs/Items/Forms/Order/Ext/Form.xml",
            "Form",
            "DifferentForm",
        );
        write_nested_module_fixture(
            &root,
            "Catalogs/Items/Commands/Print/Ext/CommandModule.bsl",
            "Catalogs/Items/Commands/Print/Ext/Command.xml",
            "Form",
            "Print",
        );
        write_nested_module_fixture_xml(
            &root,
            "Catalogs/Items/Forms/Legacy/Ext/Form/Module.bsl",
            "Catalogs/Items/Forms/Legacy/Ext/Form.xml",
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.19"><Form><Properties><Name>Legacy</Name></Properties></Form></MetaDataObject>"#,
        );
        write_nested_module_fixture_xml(
            &root,
            "Catalogs/Items/Commands/NoNamespace/Ext/CommandModule.bsl",
            "Catalogs/Items/Commands/NoNamespace/Ext/Command.xml",
            r#"<MetaDataObject version="2.20"><Command><Properties><Name>NoNamespace</Name></Properties></Command></MetaDataObject>"#,
        );

        for metadata_path in [
            "Catalog.Items.Form.Order.FormModule",
            "Catalog.Items.Command.Print.CommandModule",
            "Catalog.Items.Form.Legacy.FormModule",
            "Catalog.Items.Command.NoNamespace.CommandModule",
        ] {
            let result = resolve_platform_xml_source_navigation(
                &context,
                &SourceResolveRequest {
                    source_set: "main".to_string(),
                    query: metadata_path.to_string(),
                    mode: SourceNavigationMode::Exact,
                    target_kind: Some(crate::domain::source_target::TargetKind::Module),
                    limit: 50,
                    cursor: None,
                },
            )
            .unwrap();

            assert!(
                result.candidates.is_empty(),
                "mismatched nested descriptor must not prove {metadata_path}"
            );
            assert_eq!(result.completeness, NavigationCompleteness::Partial);
        }
        cleanup(&context);
    }

    #[test]
    fn source_navigation_prefix_accepts_typed_kind_and_module_prefixes_without_fuzzy_choice() {
        let context = fixture(
            "navigation-typed-prefix",
            project_yaml("main", "CONFIGURATION", "src"),
        );
        let root = context.workspace_root.join("src");
        write_configuration_descriptor(&root, false);
        write_metadata_descriptor(&root, "Catalogs", "Catalog", "Items", "Items");
        fs::create_dir_all(root.join("Catalogs/Items/Ext")).unwrap();
        fs::write(
            root.join("Catalogs/Items/Ext/ManagerModule.bsl"),
            "Procedure Run()\nEndProcedure\n",
        )
        .unwrap();

        let kind = resolve_platform_xml_source_navigation(
            &context,
            &SourceResolveRequest {
                source_set: "main".to_string(),
                query: "Catalog".to_string(),
                mode: SourceNavigationMode::Prefix,
                target_kind: None,
                limit: 50,
                cursor: None,
            },
        )
        .unwrap();
        assert_eq!(
            kind.candidates
                .iter()
                .map(|candidate| candidate.metadata_path.as_str())
                .collect::<Vec<_>>(),
            vec!["Catalog.Items", "Catalog.Items.ManagerModule"]
        );

        for query in [
            "Catalog.Items.Man",
            "Catalogs.Items.Man",
            "Справочники.Items.Man",
        ] {
            let result = resolve_platform_xml_source_navigation(
                &context,
                &SourceResolveRequest {
                    source_set: "main".to_string(),
                    query: query.to_string(),
                    mode: SourceNavigationMode::Prefix,
                    target_kind: Some(crate::domain::source_target::TargetKind::Module),
                    limit: 50,
                    cursor: None,
                },
            )
            .unwrap();
            assert_eq!(
                result
                    .candidates
                    .iter()
                    .map(|candidate| candidate.metadata_path.as_str())
                    .collect::<Vec<_>>(),
                vec!["Catalog.Items.ManagerModule"],
                "{query} must return the canonical candidate without fuzzy ranking"
            );
        }

        for malformed in [
            "Catalog..Man",
            "Unknown.Items",
            "Catalog.Items.Unknown",
            "Catalog.Items.Form.Order.Unknown",
        ] {
            assert!(
                resolve_platform_xml_source_navigation(
                    &context,
                    &SourceResolveRequest {
                        source_set: "main".to_string(),
                        query: malformed.to_string(),
                        mode: SourceNavigationMode::Prefix,
                        target_kind: None,
                        limit: 50,
                        cursor: None,
                    },
                )
                .is_err(),
                "{malformed} must fail typed prefix validation"
            );
        }
        cleanup(&context);
    }

    #[test]
    fn source_navigation_cancels_after_provider_entry_before_traversal_continues() {
        let context = fixture(
            "navigation-cancel-after-entry",
            project_yaml("main", "CONFIGURATION", "src"),
        );
        let root = context.workspace_root.join("src");
        write_configuration_descriptor(&root, false);
        write_real_module_fixture(&root, "CommonModule", "Shared");
        let cancellation = CancellationToken::new();
        let cancel_after_entry = cancellation.clone();
        set_navigation_provider_entry_hook_for_test(move || cancel_after_entry.cancel());

        let error = resolve_platform_xml_source_navigation_cancellable(
            &context,
            &SourceResolveRequest {
                source_set: "main".to_string(),
                query: "CommonModule".to_string(),
                mode: SourceNavigationMode::Prefix,
                target_kind: None,
                limit: 50,
                cursor: None,
            },
            &cancellation,
        )
        .unwrap_err();

        assert!(error.starts_with(CANCELLED_PREFIX), "{error}");
        cleanup(&context);
    }

    #[test]
    fn source_navigation_children_walk_one_level_and_distinguish_collections_from_items() {
        let context = fixture(
            "navigation-one-level",
            project_yaml("main", "CONFIGURATION", "src"),
        );
        let root = context.workspace_root.join("src");
        write_configuration_descriptor(&root, false);
        write_real_module_fixture(&root, "CommonModule", "Shared");
        fs::create_dir_all(root.join("Ext")).unwrap();
        fs::write(
            root.join("Ext/ManagedApplicationModule.bsl"),
            "Procedure Start()\nEndProcedure\n",
        )
        .unwrap();

        let root_page = children_platform_xml_source_navigation(
            &context,
            &SourceChildrenRequest {
                source_set: "main".to_string(),
                metadata_path: None,
                limit: 50,
                cursor: None,
            },
        )
        .unwrap();
        let collection = root_page
            .children
            .iter()
            .find(|child| child.display_name == "CommonModules")
            .expect("root exposes the CommonModules collection");
        assert_eq!(collection.node_kind, SourceNodeKind::Collection);
        assert_eq!(
            collection.addressability,
            SourceNodeAddressability::Unaddressable
        );
        assert!(collection.metadata_path.is_none());
        let root_module = root_page
            .children
            .iter()
            .find(|child| child.display_name == "ManagedApplicationModule")
            .expect("root module is a direct item");
        assert_eq!(root_module.node_kind, SourceNodeKind::Item);
        assert_eq!(
            root_module.addressability,
            SourceNodeAddressability::Addressable
        );

        let object_page = children_platform_xml_source_navigation(
            &context,
            &SourceChildrenRequest {
                source_set: "main".to_string(),
                metadata_path: Some(
                    MetadataAddress::parse(PLATFORM_XML_8_3_27_FORMAT_2_20, "CommonModule.Shared")
                        .unwrap(),
                ),
                limit: 50,
                cursor: None,
            },
        )
        .unwrap();
        assert_eq!(object_page.children.len(), 1);
        assert_eq!(object_page.children[0].node_kind, SourceNodeKind::Item);
        assert_eq!(
            object_page.children[0]
                .metadata_path
                .as_ref()
                .unwrap()
                .as_str(),
            "CommonModule.Shared.Module"
        );
        cleanup(&context);
    }

    #[test]
    fn source_navigation_external_virtual_roots_enumerate_two_artifacts_of_each_kind() {
        let context = fixture(
            "navigation-external-multi-root",
            "format: DESIGNER\nsource-set:\n  - name: processors\n    type: EXTERNAL_DATA_PROCESSORS\n    path: epf\n  - name: reports\n    type: EXTERNAL_REPORTS\n    path: erf\n",
        );
        for (directory, kind, names) in [
            ("epf", "ExternalDataProcessor", ["Import", "Export"]),
            ("erf", "ExternalReport", ["Sales", "Stock"]),
        ] {
            for name in names {
                write_external_descriptor(
                    &context.workspace_root.join(directory),
                    kind,
                    name,
                    name,
                );
            }
        }

        for (source_set, expected) in [
            (
                "processors",
                vec![
                    "ExternalDataProcessor.Export",
                    "ExternalDataProcessor.Import",
                ],
            ),
            (
                "reports",
                vec!["ExternalReport.Sales", "ExternalReport.Stock"],
            ),
        ] {
            let page = children_platform_xml_source_navigation(
                &context,
                &SourceChildrenRequest {
                    source_set: source_set.to_string(),
                    metadata_path: None,
                    limit: 50,
                    cursor: None,
                },
            )
            .unwrap();
            assert_eq!(
                page.children
                    .iter()
                    .map(|child| child.metadata_path.as_ref().unwrap().as_str())
                    .collect::<Vec<_>>(),
                expected
            );
            assert!(page.children.iter().all(|child| {
                child.node_kind == SourceNodeKind::Item
                    && child.addressability == SourceNodeAddressability::Addressable
            }));

            let artifact_page = children_platform_xml_source_navigation(
                &context,
                &SourceChildrenRequest {
                    source_set: source_set.to_string(),
                    metadata_path: Some(
                        MetadataAddress::parse(PLATFORM_XML_8_3_27_FORMAT_2_20, expected[0])
                            .unwrap(),
                    ),
                    limit: 50,
                    cursor: None,
                },
            )
            .unwrap();
            assert!(artifact_page.children.is_empty());
            assert_eq!(
                artifact_page.completeness,
                NavigationCompleteness::Partial,
                "descriptor-root enumeration must not claim knowledge of unscanned artifact children"
            );
        }
        cleanup(&context);
    }

    #[test]
    fn source_navigation_external_root_fails_closed_on_duplicate_descriptor_identity() {
        let context = fixture(
            "navigation-external-ambiguous",
            project_yaml("processors", "EXTERNAL_DATA_PROCESSORS", "epf"),
        );
        write_external_descriptor(
            &context.workspace_root.join("epf"),
            "ExternalDataProcessor",
            "First",
            "Shared",
        );
        write_external_descriptor(
            &context.workspace_root.join("epf"),
            "ExternalDataProcessor",
            "Second",
            "Shared",
        );

        let error = children_platform_xml_source_navigation(
            &context,
            &SourceChildrenRequest {
                source_set: "processors".to_string(),
                metadata_path: None,
                limit: 50,
                cursor: None,
            },
        )
        .unwrap_err();
        assert!(error.contains("ambiguous descriptor identity"), "{error}");
        assert!(!error.contains(&context.workspace_root.display().to_string()));
        assert!(!error.contains("First.xml"));
        assert!(!error.contains("Second.xml"));

        let resolve_error = resolve_platform_xml_source_navigation(
            &context,
            &SourceResolveRequest {
                source_set: "processors".to_string(),
                query: "ExternalDataProcessor.Shared".to_string(),
                mode: SourceNavigationMode::Exact,
                target_kind: None,
                limit: 50,
                cursor: None,
            },
        )
        .unwrap_err();
        assert!(resolve_error.contains("ambiguous descriptor identity"));
        assert!(!resolve_error.contains("First.xml"));
        cleanup(&context);
    }

    #[test]
    fn source_navigation_module_requires_descriptor_identity_not_only_a_matching_filename() {
        let context = fixture(
            "navigation-descriptor-identity",
            project_yaml("main", "CONFIGURATION", "src"),
        );
        let root = context.workspace_root.join("src");
        write_configuration_descriptor(&root, false);
        write_real_module_fixture(&root, "CommonModule", "Shared");
        fs::write(
            root.join("CommonModules/Shared.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><CommonModule><Properties><Name>Different</Name></Properties></CommonModule></MetaDataObject>"#,
        )
        .unwrap();

        let result = resolve_platform_xml_source_navigation(
            &context,
            &SourceResolveRequest {
                source_set: "main".to_string(),
                query: "CommonModule.Shared.Module".to_string(),
                mode: SourceNavigationMode::Exact,
                target_kind: Some(crate::domain::source_target::TargetKind::Module),
                limit: 50,
                cursor: None,
            },
        )
        .unwrap();

        assert!(result.candidates.is_empty());
        assert_eq!(result.completeness, NavigationCompleteness::Partial);
        cleanup(&context);
    }

    #[test]
    fn source_navigation_observed_paths_are_relative_and_only_unaddressable() {
        let context = fixture(
            "navigation-observed-external",
            project_yaml("processors", "EXTERNAL_DATA_PROCESSORS", "epf"),
        );
        fs::create_dir_all(context.workspace_root.join("epf")).unwrap();
        fs::write(context.workspace_root.join("epf/Broken.xml"), "<broken").unwrap();

        let page = children_platform_xml_source_navigation(
            &context,
            &SourceChildrenRequest {
                source_set: "processors".to_string(),
                metadata_path: None,
                limit: 50,
                cursor: None,
            },
        )
        .unwrap();

        assert_eq!(page.children.len(), 1);
        assert_eq!(
            page.children[0].addressability,
            SourceNodeAddressability::Unaddressable
        );
        assert!(page.children[0].metadata_path.is_none());
        assert!(matches!(
            page.children[0].location.as_ref(),
            Some(SourceLocation::Unaddressable {
                path,
                owner_metadata_path: None,
                ..
            }) if path == "Broken.xml"
        ));
        cleanup(&context);
    }

    fn target(source_set: &str, metadata_path: &str) -> SourceTarget {
        SourceTarget {
            source_set: source_set.to_string(),
            metadata_path: Some(
                MetadataAddress::parse(PLATFORM_XML_8_3_27_FORMAT_2_20, metadata_path).unwrap(),
            ),
        }
    }

    fn root_target(source_set: &str) -> SourceTarget {
        SourceTarget {
            source_set: source_set.to_string(),
            metadata_path: None,
        }
    }

    fn target_for(source_set: &str, metadata_path: &str) -> SourceTarget {
        target(source_set, metadata_path)
    }

    fn fixture(name: &str, yaml: impl AsRef<str>) -> WorkspaceContext {
        let root = temp_root(name);
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("v8project.yaml"), yaml.as_ref()).unwrap();
        WorkspaceContext {
            cwd: root.clone(),
            workspace_root: root.clone(),
            cache_root: root.join(".build/unica"),
            workspace_epoch: 1,
        }
    }

    fn project_yaml(name: &str, kind: &str, path: &str) -> String {
        format!(
            "format: DESIGNER\nsource-set:\n  - name: {name}\n    type: {kind}\n    path: {path}\n"
        )
    }

    fn write_module_fixture(root: &Path, descriptor: &str, module: &str) {
        let descriptor = root.join(descriptor);
        let module = root.join(module);
        fs::create_dir_all(descriptor.parent().unwrap()).unwrap();
        fs::create_dir_all(module.parent().unwrap()).unwrap();
        fs::write(descriptor, "<MetaDataObject/>").unwrap();
        fs::write(module, "Procedure Run()\nEndProcedure\n").unwrap();
    }

    fn write_configuration_descriptor(root: &Path, extension: bool) {
        fs::create_dir_all(root).unwrap();
        let purpose = if extension {
            "<ConfigurationExtensionPurpose>Customization</ConfigurationExtensionPurpose>"
        } else {
            ""
        };
        fs::write(
            root.join("Configuration.xml"),
            format!(
                r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Configuration><Properties><Name>NavigationFixture</Name>{purpose}</Properties></Configuration></MetaDataObject>"#
            ),
        )
        .unwrap();
    }

    fn write_real_module_fixture(root: &Path, kind: &str, name: &str) {
        assert_eq!(kind, "CommonModule");
        let directory = root.join("CommonModules");
        fs::create_dir_all(directory.join(name).join("Ext")).unwrap();
        fs::write(
            directory.join(format!("{name}.xml")),
            format!(
                r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><CommonModule><Properties><Name>{name}</Name></Properties></CommonModule></MetaDataObject>"#
            ),
        )
        .unwrap();
        fs::write(
            directory.join(name).join("Ext/Module.bsl"),
            "Procedure Run()\nEndProcedure\n",
        )
        .unwrap();
    }

    fn write_metadata_descriptor(
        root: &Path,
        directory: &str,
        kind: &str,
        file_stem: &str,
        name: &str,
    ) {
        let directory = root.join(directory);
        fs::create_dir_all(&directory).unwrap();
        fs::write(
            directory.join(format!("{file_stem}.xml")),
            format!(
                r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><{kind}><Properties><Name>{name}</Name></Properties></{kind}></MetaDataObject>"#
            ),
        )
        .unwrap();
    }

    fn write_nested_module_fixture(
        root: &Path,
        module: &str,
        descriptor: &str,
        child_kind: &str,
        descriptor_name: &str,
    ) {
        write_nested_module_fixture_xml(
            root,
            module,
            descriptor,
            &format!(
                r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><{child_kind}><Properties><Name>{descriptor_name}</Name></Properties></{child_kind}></MetaDataObject>"#
            ),
        );
    }

    fn write_nested_module_fixture_xml(
        root: &Path,
        module: &str,
        descriptor: &str,
        descriptor_xml: &str,
    ) {
        let module = root.join(module);
        let descriptor = root.join(descriptor);
        fs::create_dir_all(module.parent().unwrap()).unwrap();
        fs::create_dir_all(descriptor.parent().unwrap()).unwrap();
        fs::write(module, "Procedure Run()\nEndProcedure\n").unwrap();
        fs::write(descriptor, descriptor_xml).unwrap();
    }

    fn write_external_descriptor(root: &Path, kind: &str, file_stem: &str, name: &str) {
        fs::create_dir_all(root).unwrap();
        fs::write(
            root.join(format!("{file_stem}.xml")),
            format!(
                r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><{kind}><Properties><Name>{name}</Name></Properties><ChildObjects/></{kind}></MetaDataObject>"#
            ),
        )
        .unwrap();
    }

    fn temp_root(name: &str) -> PathBuf {
        let nonce = TEMP_NONCE.fetch_add(1, Ordering::Relaxed);
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!(
            "unica-platform-xml-targets-{name}-{}-{nanos}-{nonce}",
            std::process::id()
        ))
    }

    fn cleanup(context: &WorkspaceContext) {
        let _ = fs::remove_dir_all(&context.workspace_root);
    }
}
