use crate::application::metadata::MetaFailure;
use crate::application::source_navigation::{
    page_bounds, NavigationCompleteness, SourceChildrenRequest, SourceChildrenResult,
    SourceLocation, SourceMatchKind, SourceNavigationMode, SourceNode, SourceNodeAddressability,
    SourceNodeKind, SourceResolveCandidate, SourceResolveRequest, SourceResolveResult,
};
use crate::application::source_navigation::{
    LocateRejection, SourceLocateRequest, SourceLocateResult,
};
use crate::domain::cancellation::{cancelled_error, CancellationToken};
use crate::domain::metadata::{MetaDiagnostic, MetaDiagnosticCode};
use crate::domain::project_sources::{
    classify_already_read_config_dump_info_xml, ConfigDumpInfoXmlKind, ProjectSourceSet,
};
use crate::domain::project_sources::{SourceFormat, SourceSetKind};
use crate::domain::source_target::{
    MetadataAddress, MetadataAddressPrefix, ResolvedTarget, SourceTarget, SourceTargetError,
    SourceTargetErrorCode, TargetKind,
};
use crate::domain::workspace::WorkspaceContext;
use crate::infrastructure::metadata_kinds::{
    metadata_kind, metadata_kind_by_directory, supports_direct_module_role,
    supports_nested_form_or_command, MetadataLayout, METADATA_KINDS,
};
use crate::infrastructure::native_operations::compile_transaction::CompileTransaction;
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
    static OBJECT_DESCRIPTOR_CONTENT_READ_HOOK: RefCell<Option<Box<dyn FnOnce()>>> =
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

#[cfg(test)]
fn set_object_descriptor_content_read_hook_for_test(hook: impl FnOnce() + 'static) {
    OBJECT_DESCRIPTOR_CONTENT_READ_HOOK.with(|slot| {
        let previous = slot.replace(Some(Box::new(hook)));
        assert!(
            previous.is_none(),
            "object descriptor content-read hook leaked"
        );
    });
}

#[cfg(test)]
fn run_object_descriptor_content_read_hook_for_test() {
    OBJECT_DESCRIPTOR_CONTENT_READ_HOOK.with(|slot| {
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
    if request.mode == SourceNavigationMode::Exact {
        if let Some(result) = resolve_exact_by_rendering(context, request, cancellation)? {
            return Ok(result);
        }
    } else if let Some(result) = resolve_prefix_by_scoped_scan(context, request, cancellation)? {
        return Ok(result);
    }
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

/// Exact resolution never needs an inventory: a canonical address already names
/// its own physical candidate through the layout table, so the provider renders
/// that one path and proves it. Enumerating the source set first made the answer
/// depend on how far a bounded walk happened to get, which on a production-size
/// configuration stops long before most collections. Returns `None` for source
/// sets whose roots are virtual rather than a metadata layout, so those keep the
/// enumerating path.
fn resolve_exact_by_rendering(
    context: &WorkspaceContext,
    request: &SourceResolveRequest,
    cancellation: &CancellationToken,
) -> Result<Option<SourceResolveResult>, String> {
    let address = MetadataAddress::parse(
        crate::domain::source_target::PLATFORM_XML_8_3_27_FORMAT_2_20,
        request.query.trim(),
    )
    .map_err(|error| error.to_string())?;
    let selected = resolve_named_source_set(context, &request.source_set)
        .map_err(|error| public_source_set_error(&request.source_set, error).to_string())?;
    if selected.source_set.source_format != SourceFormat::PlatformXml {
        return Err(format!(
            "sourceSet `{}` is not addressable by the Platform XML source provider",
            request.source_set
        ));
    }
    if matches!(
        selected.source_set.kind,
        SourceSetKind::ExternalProcessor | SourceSetKind::ExternalReport
    ) {
        return Ok(None);
    }
    check_navigation_cancellation(cancellation)?;
    validate_navigation_owner(context, &selected)?;
    check_navigation_cancellation(cancellation)?;

    let target_kind = address.target_kind();
    let outcome = if request
        .target_kind
        .is_some_and(|requested| requested != target_kind)
    {
        ExactCandidate::Absent
    } else {
        match target_kind {
            TargetKind::Module => {
                exact_module_outcome(context, &selected, &request.source_set, &address)?
            }
            TargetKind::MetadataObject => {
                exact_object_outcome(&selected.path, &address, cancellation)?
            }
            TargetKind::SourceRoot => ExactCandidate::Absent,
        }
    };
    check_navigation_cancellation(cancellation)?;

    // An address whose candidate exists but cannot be proven keeps `partial`:
    // the provider saw something it could not classify, so its empty answer is
    // not authoritative. Only a candidate that is simply not there is reported
    // as a complete absence.
    if outcome == ExactCandidate::Unproven {
        return Ok(Some(SourceResolveResult {
            candidates: Vec::new(),
            completeness: NavigationCompleteness::Partial,
            next_cursor: None,
        }));
    }

    let candidates = if outcome == ExactCandidate::Proven {
        let display_name = address
            .as_str()
            .split('.')
            .next_back()
            .unwrap_or(address.as_str())
            .to_string();
        vec![SourceResolveCandidate {
            metadata_path: address.clone(),
            target_kind,
            display_name,
            match_kind: SourceMatchKind::Exact,
            location: addressed_location(&selected.source_set.name, Some(address), target_kind),
        }]
    } else {
        Vec::new()
    };
    Ok(Some(SourceResolveResult {
        candidates,
        completeness: NavigationCompleteness::Complete,
        next_cursor: None,
    }))
}

/// Recovers the logical address that owns a source path. Every other public
/// producer of physical paths — `unica.code.search` hits, a git diff, a build
/// log — otherwise dead-ends, because the logical tools accept addresses only.
pub(crate) fn locate_platform_xml_source_path(
    context: &WorkspaceContext,
    request: &SourceLocateRequest,
    cancellation: &CancellationToken,
) -> Result<SourceLocateResult, String> {
    locate_platform_xml_source_path_with_policy(
        context,
        request,
        cancellation,
        DescriptorVersionPolicy::ActiveProfile,
    )
}

/// The temporary subject-reader bridge needs the same exact inverse mapping as
/// `source.locate`, but it runs before the format guard has classified an old
/// or new dump. The resolver after this locator still proves registration and
/// exact owner/target version equality; this policy only avoids misreporting a
/// coherent non-active version as an unknown physical owner.
pub(crate) fn locate_platform_xml_reader_path(
    context: &WorkspaceContext,
    request: &SourceLocateRequest,
    cancellation: &CancellationToken,
) -> Result<SourceLocateResult, String> {
    locate_platform_xml_source_path_with_policy(
        context,
        request,
        cancellation,
        DescriptorVersionPolicy::AnyExact,
    )
}

fn locate_platform_xml_source_path_with_policy(
    context: &WorkspaceContext,
    request: &SourceLocateRequest,
    cancellation: &CancellationToken,
    version_policy: DescriptorVersionPolicy,
) -> Result<SourceLocateResult, String> {
    check_navigation_cancellation(cancellation)?;
    let selected = resolve_named_source_set(context, &request.source_set)
        .map_err(|error| public_source_set_error(&request.source_set, error).to_string())?;
    locate_platform_xml_source_path_in_with_policy(
        context,
        &selected.source_set,
        &selected.path,
        &request.path,
        cancellation,
        version_policy,
    )
}

/// The same inverse mapping as `unica.source.locate`, against a source set the
/// caller has already resolved exactly. Diagnostics uses this entry point so a
/// provider result cannot silently trigger the legacy default-source fallback.
pub(crate) fn locate_platform_xml_source_path_in(
    context: &WorkspaceContext,
    source_set: &ProjectSourceSet,
    source_root: &Path,
    raw_path: &str,
    cancellation: &CancellationToken,
) -> Result<SourceLocateResult, String> {
    locate_platform_xml_source_path_in_with_policy(
        context,
        source_set,
        source_root,
        raw_path,
        cancellation,
        DescriptorVersionPolicy::ActiveProfile,
    )
}

fn locate_platform_xml_source_path_in_with_policy(
    context: &WorkspaceContext,
    source_set: &ProjectSourceSet,
    source_root: &Path,
    raw_path: &str,
    cancellation: &CancellationToken,
    version_policy: DescriptorVersionPolicy,
) -> Result<SourceLocateResult, String> {
    if source_set.source_format != SourceFormat::PlatformXml {
        return Err(format!(
            "sourceSet `{}` is not addressable by the Platform XML source provider",
            source_set.name
        ));
    }
    let reject = |relative: String, rejection: LocateRejection| SourceLocateResult {
        source_set: source_set.name.clone(),
        relative_path: relative,
        metadata_path: None,
        target_kind: None,
        owner_metadata_path: None,
        rejection: Some(rejection),
    };

    let raw = Path::new(raw_path.trim());
    let Some(relative) = source_set_relative_path(context, source_root, raw) else {
        // There is no source-set-relative form for a path outside the set, and
        // echoing a stripped one back would read as relative while naming
        // somewhere else entirely.
        return Ok(reject(String::new(), LocateRejection::OutsideSourceSet));
    };
    check_navigation_cancellation(cancellation)?;
    let relative_text = portable_relative(&relative);

    // A module is addressable in its own right; everything else can at best
    // name the metadata object that owns it.
    if relative.extension().and_then(|value| value.to_str()) == Some("bsl") {
        if let Ok(identity) = platform_xml_module_identity(&relative) {
            let proven = validate_platform_xml_module_descriptors(
                context,
                source_root,
                &identity.descriptors,
            )
            .is_ok()
                && module_descriptor_identity_is_proven(source_root, &identity, cancellation)?;
            if !proven {
                return Ok(reject(relative_text, LocateRejection::OwnerUnproven));
            }
            let owner = MetadataAddress::parse(
                crate::domain::source_target::PLATFORM_XML_8_3_27_FORMAT_2_20,
                &identity.owner,
            )
            .ok();
            return Ok(SourceLocateResult {
                source_set: source_set.name.clone(),
                relative_path: relative_text,
                metadata_path: Some(identity.address),
                target_kind: Some(TargetKind::Module),
                owner_metadata_path: owner,
                rejection: None,
            });
        }
        return Ok(reject(relative_text, LocateRejection::NotAddressable));
    }

    let parts = relative
        .components()
        .map(|component| match component {
            Component::Normal(value) => value.to_str(),
            _ => None,
        })
        .collect::<Option<Vec<_>>>();
    let Some(parts) = parts else {
        return Ok(reject(relative_text, LocateRejection::NotAddressable));
    };
    let Some(kind) = parts
        .first()
        .and_then(|first| metadata_kind_by_directory(first))
    else {
        return Ok(reject(relative_text, LocateRejection::NotAddressable));
    };
    let Some(name) = parts.get(1).map(|name| name.trim_end_matches(".xml")) else {
        return Ok(reject(relative_text, LocateRejection::NotAddressable));
    };
    let object = format!("{}.{name}", kind.tag);
    let Ok(object_address) = MetadataAddress::parse(
        crate::domain::source_target::PLATFORM_XML_8_3_27_FORMAT_2_20,
        &object,
    ) else {
        return Ok(reject(relative_text, LocateRejection::NotAddressable));
    };
    if exact_object_outcome_with_policy(source_root, &object_address, cancellation, version_policy)?
        != ExactCandidate::Proven
    {
        return Ok(reject(relative_text, LocateRejection::OwnerUnproven));
    }
    check_navigation_cancellation(cancellation)?;

    // A nested Form, Template, or Command owns everything beneath its own directory.
    let nested = match parts.get(2).copied() {
        Some(directory @ ("Forms" | "Templates" | "Commands")) => parts.get(3).map(|child| {
            let child_kind = match directory {
                "Forms" => "Form",
                "Templates" => "Template",
                "Commands" => "Command",
                _ => unreachable!("nested metadata directory match is closed"),
            };
            // The same child appears twice under the collection: once as its
            // `<Name>.xml` descriptor and once as the `<Name>/` content tree.
            format!("{object}.{child_kind}.{}", child.trim_end_matches(".xml"))
        }),
        _ => None,
    };
    if let Some(nested) = nested {
        if let Ok(nested_address) = MetadataAddress::parse(
            crate::domain::source_target::PLATFORM_XML_8_3_27_FORMAT_2_20,
            &nested,
        ) {
            if exact_object_outcome_with_policy(
                source_root,
                &nested_address,
                cancellation,
                version_policy,
            )? == ExactCandidate::Proven
            {
                let is_own_descriptor = parts.len() == 4 && parts[3].ends_with(".xml");
                return Ok(SourceLocateResult {
                    source_set: source_set.name.clone(),
                    relative_path: relative_text,
                    metadata_path: is_own_descriptor.then(|| nested_address.clone()),
                    target_kind: is_own_descriptor.then_some(TargetKind::MetadataObject),
                    owner_metadata_path: Some(nested_address),
                    rejection: (!is_own_descriptor).then_some(LocateRejection::NotAddressable),
                });
            }
        }
    }

    let is_object_descriptor = parts.len() == 2 && parts[1].ends_with(".xml");
    Ok(SourceLocateResult {
        source_set: source_set.name.clone(),
        relative_path: relative_text,
        metadata_path: is_object_descriptor.then(|| object_address.clone()),
        target_kind: is_object_descriptor.then_some(TargetKind::MetadataObject),
        owner_metadata_path: Some(object_address),
        rejection: (!is_object_descriptor).then_some(LocateRejection::NotAddressable),
    })
}

/// Accepts an absolute path, a workspace-relative path or a source-set-relative
/// path, and returns it relative to the source root when it is contained there.
pub(crate) fn source_set_relative_path(
    context: &WorkspaceContext,
    source_root: &Path,
    raw: &Path,
) -> Option<PathBuf> {
    let mut candidates = Vec::new();
    if raw.is_absolute() {
        candidates.push(raw.to_path_buf());
    } else {
        candidates.push(context.workspace_root.join(raw));
        candidates.push(source_root.join(raw));
    }
    for candidate in candidates {
        let Ok(normalized) = normalize_path_identity(&candidate) else {
            continue;
        };
        if let Ok(relative) = normalized.strip_prefix(source_root) {
            if relative.as_os_str().is_empty() {
                continue;
            }
            return Some(relative.to_path_buf());
        }
    }
    None
}

pub(crate) fn portable_relative(path: &Path) -> String {
    path.components()
        .filter_map(|component| match component {
            Component::Normal(value) => value.to_str(),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("/")
}

/// A prefix query pins its leading segments, so the scan is scoped to the
/// collections those segments select instead of walking the whole source set.
/// Names are filtered by file name before any descriptor is parsed, so the cost
/// tracks the answer rather than the size of the configuration. Returns `None`
/// when the shape is not one of the scoped forms, which keeps the enumerating
/// path for the remainder.
fn resolve_prefix_by_scoped_scan(
    context: &WorkspaceContext,
    request: &SourceResolveRequest,
    cancellation: &CancellationToken,
) -> Result<Option<SourceResolveResult>, String> {
    let prefix = MetadataAddressPrefix::parse(
        crate::domain::source_target::PLATFORM_XML_8_3_27_FORMAT_2_20,
        request.query.trim(),
    )
    .map_err(|error| error.to_string())?;
    let query = prefix.as_str().to_string();
    let parts = query.split('.').collect::<Vec<_>>();
    if parts.len() > 5 {
        return Ok(None);
    }
    let selected = resolve_named_source_set(context, &request.source_set)
        .map_err(|error| public_source_set_error(&request.source_set, error).to_string())?;
    if selected.source_set.source_format != SourceFormat::PlatformXml {
        return Err(format!(
            "sourceSet `{}` is not addressable by the Platform XML source provider",
            request.source_set
        ));
    }
    if matches!(
        selected.source_set.kind,
        SourceSetKind::ExternalProcessor | SourceSetKind::ExternalReport
    ) {
        return Ok(None);
    }
    check_navigation_cancellation(cancellation)?;
    validate_navigation_owner(context, &selected)?;
    // The scoped scan is the traversal for a prefix query, so it carries the
    // provider-entry observation the enumerating path used to own.
    #[cfg(test)]
    run_navigation_provider_entry_hook_for_test();
    check_navigation_cancellation(cancellation)?;

    // A leading segment that is already a complete kind pins one collection;
    // otherwise it is a partial canonical token that can only select whole
    // collections whose tag it prefixes.
    let kinds = match metadata_kind(parts[0]) {
        Some(kind) if parts.len() >= 2 => vec![kind],
        _ if parts.len() == 1 => METADATA_KINDS
            .iter()
            .filter(|kind| kind.tag.starts_with(parts[0]))
            .collect::<Vec<_>>(),
        _ => return Ok(None),
    };

    let mut partial = false;
    let mut matches = Vec::new();
    for kind in kinds {
        check_navigation_cancellation(cancellation)?;
        let name_filter = if parts.len() >= 2 {
            Some(parts[1])
        } else {
            None
        };
        for name in proven_object_names(
            &selected.path,
            kind,
            name_filter,
            &mut partial,
            cancellation,
        )? {
            check_navigation_cancellation(cancellation)?;
            let object = format!("{}.{name}", kind.tag);
            if parts.len() < 3 && object.starts_with(&query) {
                if let Ok(address) = MetadataAddress::parse(
                    crate::domain::source_target::PLATFORM_XML_8_3_27_FORMAT_2_20,
                    &object,
                ) {
                    matches.push((address, TargetKind::MetadataObject, name.clone()));
                }
            }
            for terminal in crate::domain::source_target::module_terminals() {
                check_navigation_cancellation(cancellation)?;
                push_module_candidate(
                    context,
                    &selected,
                    &format!("{object}.{terminal}"),
                    &query,
                    terminal,
                    &mut matches,
                )?;
            }
            // A nested Form, Template, or Command is addressable in its own right, so a
            // prefix that can reach one must not be answered without it. The
            // descent is affordable only once a name prefix bounds the outer
            // loop; a bare-kind query says so instead of claiming completeness.
            if parts.len() < 2 {
                continue;
            }
            for (child_kind, child_directory, module_terminal) in [
                ("Form", "Forms", Some("FormModule")),
                ("Template", "Templates", None),
                ("Command", "Commands", Some("CommandModule")),
            ] {
                check_navigation_cancellation(cancellation)?;
                if !supports_nested_form_or_command(kind.tag) {
                    break;
                }
                for child in proven_child_names(
                    &selected.path,
                    kind,
                    &name,
                    child_kind,
                    child_directory,
                    &mut partial,
                    cancellation,
                )? {
                    check_navigation_cancellation(cancellation)?;
                    let nested = format!("{object}.{child_kind}.{child}");
                    if nested.starts_with(&query) {
                        if let Ok(address) = MetadataAddress::parse(
                            crate::domain::source_target::PLATFORM_XML_8_3_27_FORMAT_2_20,
                            &nested,
                        ) {
                            matches.push((address, TargetKind::MetadataObject, child.clone()));
                        }
                    }
                    if let Some(terminal) = module_terminal {
                        push_module_candidate(
                            context,
                            &selected,
                            &format!("{nested}.{terminal}"),
                            &query,
                            terminal,
                            &mut matches,
                        )?;
                    }
                }
            }
        }
        if parts.len() < 2 {
            // Nested descendants were not enumerated for this query shape.
            partial = true;
        }
    }
    if parts.len() == 1 {
        for terminal in crate::domain::source_target::root_module_terminals() {
            check_navigation_cancellation(cancellation)?;
            if !terminal.starts_with(&query) {
                continue;
            }
            if rendered_module_node(context, &selected, terminal)?.is_some() {
                if let Ok(address) = MetadataAddress::parse(
                    crate::domain::source_target::PLATFORM_XML_8_3_27_FORMAT_2_20,
                    terminal,
                ) {
                    matches.push((address, TargetKind::Module, (*terminal).to_string()));
                }
            }
        }
    }

    matches.retain(|(_, target_kind, _)| {
        request
            .target_kind
            .is_none_or(|requested| requested == *target_kind)
    });
    matches.sort_by(|left, right| left.0.as_str().cmp(right.0.as_str()));
    check_navigation_cancellation(cancellation)?;

    let cursor_key = format!(
        "resolve:{}:{:?}:{:?}:{query}",
        selected.source_set.name, request.mode, request.target_kind
    );
    let (start, end, next_cursor) = page_bounds(
        request.cursor.as_deref(),
        &cursor_key,
        request.limit,
        matches.len(),
    )?;
    let candidates = matches[start..end]
        .iter()
        .map(
            |(address, target_kind, display_name)| SourceResolveCandidate {
                metadata_path: address.clone(),
                target_kind: *target_kind,
                display_name: display_name.clone(),
                match_kind: SourceMatchKind::Prefix,
                location: addressed_location(
                    &selected.source_set.name,
                    Some(address.clone()),
                    *target_kind,
                ),
            },
        )
        .collect();
    let completeness = if partial || next_cursor.is_some() {
        NavigationCompleteness::Partial
    } else {
        NavigationCompleteness::Complete
    };
    Ok(Some(SourceResolveResult {
        candidates,
        completeness,
        next_cursor,
    }))
}

fn push_module_candidate(
    context: &WorkspaceContext,
    selected: &ResolvedNamedSourceSet,
    candidate: &str,
    query: &str,
    display_name: &str,
    matches: &mut Vec<(MetadataAddress, TargetKind, String)>,
) -> Result<(), String> {
    if !candidate.starts_with(query) {
        return Ok(());
    }
    if rendered_module_node(context, selected, candidate)?.is_none() {
        return Ok(());
    }
    if let Ok(address) = MetadataAddress::parse(
        crate::domain::source_target::PLATFORM_XML_8_3_27_FORMAT_2_20,
        candidate,
    ) {
        matches.push((address, TargetKind::Module, display_name.to_string()));
    }
    Ok(())
}

/// Reads one nested `Forms`/`Templates`/`Commands` collection of a proven owner and returns
/// the child names whose own descriptor proves them.
fn proven_child_names(
    source_root: &Path,
    kind: &MetadataLayout,
    owner: &str,
    child_kind: &str,
    child_directory: &str,
    partial: &mut bool,
    cancellation: &CancellationToken,
) -> Result<Vec<String>, String> {
    let directory = source_root
        .join(kind.directory)
        .join(owner)
        .join(child_directory);
    let entries = match fs::read_dir(&directory) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(_) => {
            *partial = true;
            return Ok(Vec::new());
        }
    };
    let mut names = Vec::new();
    for entry in entries {
        check_navigation_cancellation(cancellation)?;
        let Ok(entry) = entry else {
            *partial = true;
            continue;
        };
        let path = entry.path();
        if path.extension().and_then(|value| value.to_str()) != Some("xml") {
            continue;
        }
        let Some(stem) = path.file_stem().and_then(|value| value.to_str()) else {
            *partial = true;
            continue;
        };
        match descriptor_name(&path, child_kind) {
            Ok(name) if name == stem => names.push(name),
            Ok(_) => *partial = true,
            Err(()) => *partial = true,
        }
    }
    Ok(names)
}

/// Reads one collection directory and returns the names whose descriptor
/// identity is proven. The file-name filter runs before the descriptor is read,
/// so a narrow query never pays to parse the whole collection.
fn proven_object_names(
    source_root: &Path,
    kind: &MetadataLayout,
    name_prefix: Option<&str>,
    partial: &mut bool,
    cancellation: &CancellationToken,
) -> Result<Vec<String>, String> {
    let directory = source_root.join(kind.directory);
    let metadata = match fs::symlink_metadata(&directory) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(_) => {
            *partial = true;
            return Ok(Vec::new());
        }
    };
    if metadata_is_link_or_reparse_point(&metadata) || !metadata.is_dir() {
        *partial = true;
        return Ok(Vec::new());
    }
    let entries = match fs::read_dir(&directory) {
        Ok(entries) => entries,
        Err(_) => {
            *partial = true;
            return Ok(Vec::new());
        }
    };
    let mut names = Vec::new();
    let mut inspected = 0_usize;
    for entry in entries {
        check_navigation_cancellation(cancellation)?;
        let Ok(entry) = entry else {
            *partial = true;
            continue;
        };
        let path = entry.path();
        if path.extension().and_then(|value| value.to_str()) != Some("xml") {
            continue;
        }
        let Some(stem) = path.file_stem().and_then(|value| value.to_str()) else {
            *partial = true;
            continue;
        };
        if name_prefix.is_some_and(|prefix| !stem.starts_with(prefix)) {
            continue;
        }
        // Only a candidate that survives the cheap filters counts against the
        // bound: skipping a sibling directory costs nothing and must not make
        // the provider claim its answer is truncated.
        if inspected >= MAX_NAVIGATION_INVENTORY_ENTRIES {
            *partial = true;
            break;
        }
        inspected += 1;
        match descriptor_name(&path, kind.tag) {
            Ok(name) if name == stem => names.push(name),
            Ok(_) => *partial = true,
            Err(()) => *partial = true,
        }
    }
    Ok(names)
}

/// `children` descends exactly one level, so it reads the directory the target
/// maps to instead of materialising a recursive inventory of the whole source
/// set. Returns `None` for source sets whose roots are virtual, which keep the
/// enumerating path.
fn children_by_rendering(
    context: &WorkspaceContext,
    request: &SourceChildrenRequest,
    cancellation: &CancellationToken,
) -> Result<Option<SourceChildrenResult>, String> {
    let selected = resolve_named_source_set(context, &request.source_set)
        .map_err(|error| public_source_set_error(&request.source_set, error).to_string())?;
    if selected.source_set.source_format != SourceFormat::PlatformXml {
        return Err(format!(
            "sourceSet `{}` is not addressable by the Platform XML source provider",
            request.source_set
        ));
    }
    if matches!(
        selected.source_set.kind,
        SourceSetKind::ExternalProcessor | SourceSetKind::ExternalReport
    ) {
        return Ok(None);
    }
    check_navigation_cancellation(cancellation)?;
    validate_navigation_owner(context, &selected)?;
    check_navigation_cancellation(cancellation)?;

    let mut children = match request.metadata_path.as_ref() {
        None => rendered_root_children(context, &selected, cancellation)?,
        Some(parent) => rendered_address_children(
            context,
            &selected,
            &request.source_set,
            parent,
            cancellation,
        )?,
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
    let parent_key = request
        .metadata_path
        .as_ref()
        .map(MetadataAddress::as_str)
        .unwrap_or("<root>");
    let cursor_key = format!("children:{}:{parent_key}", selected.source_set.name);
    let (start, end, next_cursor) = page_bounds(
        request.cursor.as_deref(),
        &cursor_key,
        request.limit,
        children.len(),
    )?;
    let completeness = if next_cursor.is_none() {
        NavigationCompleteness::Complete
    } else {
        NavigationCompleteness::Partial
    };
    Ok(Some(SourceChildrenResult {
        children: children[start..end].to_vec(),
        completeness,
        next_cursor,
    }))
}

fn rendered_root_children(
    context: &WorkspaceContext,
    selected: &ResolvedNamedSourceSet,
    cancellation: &CancellationToken,
) -> Result<Vec<SourceNode>, String> {
    let mut children = Vec::new();
    for kind in METADATA_KINDS {
        check_navigation_cancellation(cancellation)?;
        let directory = selected.path.join(kind.directory);
        let Ok(metadata) = fs::symlink_metadata(&directory) else {
            continue;
        };
        if metadata_is_link_or_reparse_point(&metadata) || !metadata.is_dir() {
            continue;
        }
        children.push(collection_node(kind.directory.to_string()));
    }
    for terminal in crate::domain::source_target::root_module_terminals() {
        check_navigation_cancellation(cancellation)?;
        if let Some(node) = rendered_module_node(context, selected, terminal)? {
            children.push(node);
        }
    }
    Ok(children)
}

fn rendered_address_children(
    context: &WorkspaceContext,
    selected: &ResolvedNamedSourceSet,
    source_set: &str,
    parent: &MetadataAddress,
    cancellation: &CancellationToken,
) -> Result<Vec<SourceNode>, String> {
    if parent.target_kind() == TargetKind::Module {
        return Ok(Vec::new());
    }
    match exact_object_outcome(&selected.path, parent, cancellation)? {
        ExactCandidate::Proven => {}
        ExactCandidate::Unproven => {
            return Err(format!(
                "metadataPath `{}` in sourceSet `{source_set}` could not be proven; its descriptor identity does not match",
                parent.as_str()
            ))
        }
        ExactCandidate::Absent => {
            return Err(format!(
                "metadataPath `{}` was not found in sourceSet `{source_set}`",
                parent.as_str()
            ))
        }
    }

    let parts = parent.segments().collect::<Vec<_>>();
    let mut children = Vec::new();
    for terminal in crate::domain::source_target::module_terminals() {
        check_navigation_cancellation(cancellation)?;
        let candidate = format!("{}.{terminal}", parent.as_str());
        if let Some(node) = rendered_module_node(context, selected, &candidate)? {
            children.push(node);
        }
    }
    if let [kind, name] = parts.as_slice() {
        if let Some(kind) = metadata_kind(kind) {
            if supports_nested_form_or_command(kind.tag) {
                for child_directory in ["Forms", "Templates", "Commands"] {
                    check_navigation_cancellation(cancellation)?;
                    let directory = selected
                        .path
                        .join(kind.directory)
                        .join(name)
                        .join(child_directory);
                    let Ok(metadata) = fs::symlink_metadata(&directory) else {
                        continue;
                    };
                    if metadata_is_link_or_reparse_point(&metadata) || !metadata.is_dir() {
                        continue;
                    }
                    children.push(collection_node(child_directory.to_string()));
                }
            }
        }
    }
    Ok(children)
}

/// Renders one candidate module address and returns its node only when the
/// provider can prove it, so probing never invents a child.
fn rendered_module_node(
    context: &WorkspaceContext,
    selected: &ResolvedNamedSourceSet,
    candidate: &str,
) -> Result<Option<SourceNode>, String> {
    let Ok(address) = MetadataAddress::parse(
        crate::domain::source_target::PLATFORM_XML_8_3_27_FORMAT_2_20,
        candidate,
    ) else {
        return Ok(None);
    };
    // Most probed roles do not exist for a given owner. One stat rules them out
    // before paying for the full containment and descriptor proof.
    let Ok(relative) = module_path_for_address(&address) else {
        return Ok(None);
    };
    if fs::symlink_metadata(selected.path.join(&relative)).is_err() {
        return Ok(None);
    }
    if exact_module_outcome(context, selected, &selected.source_set.name, &address)?
        != ExactCandidate::Proven
    {
        return Ok(None);
    }
    let display_name = address
        .as_str()
        .split('.')
        .next_back()
        .unwrap_or(address.as_str())
        .to_string();
    Ok(Some(SourceNode {
        display_name,
        node_kind: SourceNodeKind::Item,
        addressability: SourceNodeAddressability::Addressable,
        completeness: NavigationCompleteness::Complete,
        metadata_path: Some(address.clone()),
        target_kind: Some(TargetKind::Module),
        location: Some(addressed_location(
            &selected.source_set.name,
            Some(address),
            TargetKind::Module,
        )),
    }))
}

fn collection_node(display_name: String) -> SourceNode {
    SourceNode {
        display_name,
        node_kind: SourceNodeKind::Collection,
        addressability: SourceNodeAddressability::Unaddressable,
        completeness: NavigationCompleteness::Complete,
        metadata_path: None,
        target_kind: None,
        location: None,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ExactCandidate {
    /// The rendered candidate exists and its descriptor identity is proven.
    Proven,
    /// Something occupies the rendered candidate but it cannot be classified.
    Unproven,
    /// Nothing occupies the rendered candidate.
    Absent,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DescriptorVersionPolicy {
    ActiveProfile,
    AnyExact,
}

/// Proves a module address through the same resolver `unica.source.resources`
/// uses, so navigation and access agree on what exists. A containment denial is
/// surfaced rather than reported as a plain absence.
fn exact_module_outcome(
    context: &WorkspaceContext,
    selected: &ResolvedNamedSourceSet,
    source_set: &str,
    address: &MetadataAddress,
) -> Result<ExactCandidate, String> {
    let target = SourceTarget {
        source_set: source_set.to_string(),
        metadata_path: Some(address.clone()),
    };
    match resolve_platform_xml_target_in(
        context,
        &target,
        TargetKindPolicy::ModuleOnly,
        selected.clone(),
    ) {
        Ok(_) => Ok(ExactCandidate::Proven),
        Err(error) if error.code == SourceTargetErrorCode::ContainmentDenied => {
            Err(error.to_string())
        }
        Err(_) => {
            let occupied = module_path_for_address(address)
                .ok()
                .map(|relative| selected.path.join(relative))
                .is_some_and(|path| fs::symlink_metadata(path).is_ok());
            Ok(if occupied {
                ExactCandidate::Unproven
            } else {
                ExactCandidate::Absent
            })
        }
    }
}

fn exact_object_outcome(
    source_root: &Path,
    address: &MetadataAddress,
    cancellation: &CancellationToken,
) -> Result<ExactCandidate, String> {
    exact_object_outcome_with_policy(
        source_root,
        address,
        cancellation,
        DescriptorVersionPolicy::ActiveProfile,
    )
}

fn exact_object_outcome_with_policy(
    source_root: &Path,
    address: &MetadataAddress,
    cancellation: &CancellationToken,
    version_policy: DescriptorVersionPolicy,
) -> Result<ExactCandidate, String> {
    check_navigation_cancellation(cancellation)?;
    let parts = address.segments().collect::<Vec<_>>();
    // A nested child is only as real as the owner that declares it, so the
    // owner descriptor is proven first and its verdict wins when it fails.
    if parts.len() == 4 {
        let Some(owner) = object_descriptor_evidence(&parts[..2]) else {
            return Ok(ExactCandidate::Absent);
        };
        let outcome = descriptor_outcome_with_policy(
            source_root,
            &owner.path,
            &owner.kind,
            &owner.name,
            version_policy,
        );
        if outcome != ExactCandidate::Proven {
            return Ok(outcome);
        }
        check_navigation_cancellation(cancellation)?;
    }
    let Some(evidence) = object_descriptor_evidence(&parts) else {
        return Ok(ExactCandidate::Absent);
    };
    Ok(descriptor_outcome_with_policy(
        source_root,
        &evidence.path,
        &evidence.kind,
        &evidence.name,
        version_policy,
    ))
}

fn descriptor_outcome_with_policy(
    source_root: &Path,
    descriptor: &Path,
    expected_kind: &str,
    expected_name: &str,
    version_policy: DescriptorVersionPolicy,
) -> ExactCandidate {
    if version_policy == DescriptorVersionPolicy::ActiveProfile {
        return descriptor_outcome(source_root, descriptor, expected_kind, expected_name);
    }
    let path = source_root.join(descriptor);
    match read_navigation_descriptor(&path)
        .and_then(|raw| descriptor_version_and_name_from_bytes(&raw, expected_kind))
    {
        Ok((_version, name)) if name == expected_name => ExactCandidate::Proven,
        Ok(_) => ExactCandidate::Unproven,
        Err(()) if fs::symlink_metadata(&path).is_ok() => ExactCandidate::Unproven,
        Err(()) => ExactCandidate::Absent,
    }
}

struct ObjectDescriptorEvidence {
    path: PathBuf,
    kind: String,
    name: String,
}

/// The one place that renders a metadata object address into the descriptor
/// that proves it. Navigation, resolution and resource access all read this
/// mapping so they cannot disagree about what a logical object is.
fn object_descriptor_evidence(parts: &[&str]) -> Option<ObjectDescriptorEvidence> {
    match parts {
        [kind, name] => {
            let kind = metadata_kind(kind)?;
            Some(ObjectDescriptorEvidence {
                path: metadata_descriptor(kind.directory, name),
                kind: kind.tag.to_string(),
                name: (*name).to_string(),
            })
        }
        [kind, name, child_kind, child_name]
            if matches!(
                *child_kind,
                "Form" | "Template" | "Command" | "Recalculation"
            ) =>
        {
            let kind = metadata_kind(kind)?;
            let child_directory = match *child_kind {
                "Form" => "Forms",
                "Template" => "Templates",
                "Command" => "Commands",
                "Recalculation" if kind.tag == "CalculationRegister" => "Recalculations",
                "Recalculation" => return None,
                _ => unreachable!("nested metadata kind match is closed"),
            };
            Some(ObjectDescriptorEvidence {
                path: PathBuf::from(kind.directory)
                    .join(name)
                    .join(child_directory)
                    .join(format!("{child_name}.xml")),
                kind: (*child_kind).to_string(),
                name: (*child_name).to_string(),
            })
        }
        _ => None,
    }
}

fn descriptor_outcome(
    source_root: &Path,
    descriptor: &Path,
    expected_kind: &str,
    expected_name: &str,
) -> ExactCandidate {
    let path = source_root.join(descriptor);
    match descriptor_name(&path, expected_kind) {
        Ok(name) if name == expected_name => ExactCandidate::Proven,
        Ok(_) => ExactCandidate::Unproven,
        Err(()) => {
            if fs::symlink_metadata(&path).is_ok() {
                ExactCandidate::Unproven
            } else {
                ExactCandidate::Absent
            }
        }
    }
}

pub(crate) fn children_platform_xml_source_navigation(
    context: &WorkspaceContext,
    request: &SourceChildrenRequest,
    cancellation: &CancellationToken,
) -> Result<SourceChildrenResult, String> {
    check_navigation_cancellation(cancellation)?;
    if let Some(result) = children_by_rendering(context, request, cancellation)? {
        return Ok(result);
    }
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
    let (version, name) = descriptor_version_and_name_from_bytes(raw, expected_kind)?;
    if version.as_deref() != Some("2.20") {
        return Err(());
    }
    Ok(name)
}

fn descriptor_version_and_name_from_bytes(
    raw: &[u8],
    expected_kind: &str,
) -> Result<(Option<String>, String), ()> {
    let text = std::str::from_utf8(raw).map_err(|_| ())?;
    let source = text.trim_start_matches('\u{feff}');
    let document = roxmltree::Document::parse(source).map_err(|_| ())?;
    let root = document.root_element();
    if root.tag_name().namespace() != Some(MD_CLASSES_NS)
        || root.tag_name().name() != "MetaDataObject"
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
    Ok((
        crate::infrastructure::platform_xml_owner::root_version_literal(source, root),
        name.to_string(),
    ))
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
    target_kind: TargetKind,
    target_path: PathBuf,
    module_owner: Option<String>,
    /// Whether this handle was issued under a policy that tolerates a module
    /// file the platform never exported. Revalidation reproduces the same
    /// decision, so a handle can neither gain nor lose that tolerance.
    module_absence_allowed: bool,
}

/// Closed provider handle for creation in one Platform XML source set. Public
/// and domain results receive only logical addresses; physical state remains
/// inside infrastructure.
pub(crate) struct ResolvedSourceSet {
    handle: ClosedPlatformXmlTarget,
    pub(crate) source_root: PathBuf,
    pub(crate) owner_path: PathBuf,
    pub(crate) owner_preimage: Vec<u8>,
    pub(crate) format_version: String,
}

impl fmt::Debug for ResolvedSourceSet {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ResolvedSourceSet")
            .field("source_set", &self.handle.source_target.source_set)
            .field("format_version", &self.format_version)
            .field("physical_handle", &"<closed>")
            .finish()
    }
}

pub(crate) fn resolve_metadata_add_source(
    context: &WorkspaceContext,
    source_set: &str,
) -> Result<ResolvedSourceSet, MetaFailure> {
    let target = SourceTarget {
        source_set: source_set.to_string(),
        metadata_path: None,
    };
    let resolution = resolve_platform_xml_target(context, &target, TargetKindPolicy::Any)
        .map_err(|error| metadata_source_failure(source_set, error))?;
    let evidence = platform_xml_resource_evidence(context, &resolution.handle)
        .map_err(|error| metadata_source_failure(source_set, error))?;
    let format_version = crate::infrastructure::native_operations::common::detect_format_version(
        &evidence.registration_path,
        context,
    )
    .map_err(|_| {
        MetaFailure::from(MetaDiagnostic::error(
            MetaDiagnosticCode::CapabilityUnavailable,
            format!(
                "sourceSet `{source_set}` is outside the supported Platform XML format profile"
            ),
        ))
    })?
    .to_string();
    let owner_preimage = fs::read(&evidence.registration_path).map_err(|_| {
        MetaFailure::from(MetaDiagnostic::error(
            MetaDiagnosticCode::ProviderUnavailable,
            format!("sourceSet `{source_set}` owner image is unavailable"),
        ))
    })?;
    Ok(ResolvedSourceSet {
        handle: resolution.handle,
        source_root: evidence.source_root,
        owner_path: evidence.registration_path,
        owner_preimage,
        format_version,
    })
}

pub(crate) fn revalidate_metadata_add_source(
    context: &WorkspaceContext,
    source: &ResolvedSourceSet,
) -> Result<(), MetaFailure> {
    revalidate_platform_xml_target(context, &source.handle)
        .map(|_| ())
        .map_err(|_| {
            MetaFailure::from(MetaDiagnostic::error(
                MetaDiagnosticCode::ConcurrentModification,
                format!(
                    "sourceSet `{}` changed after metadata creation was prepared",
                    source.handle.source_target.source_set
                ),
            ))
        })
}

pub(crate) fn bind_metadata_add_source_evidence(
    transaction: &mut CompileTransaction,
    context: &WorkspaceContext,
    source: &ResolvedSourceSet,
) -> Result<(), MetaFailure> {
    transaction
        .guard_or_verify_exact_preimage(&source.owner_path, &source.owner_preimage)
        .map_err(metadata_concurrent_failure)?;
    crate::infrastructure::native_operations::common::guard_resolved_platform_xml_target_dependencies(
        transaction,
        &source.handle,
        context,
    )
    .map(|_| ())
    .map_err(metadata_concurrent_failure)
}

fn metadata_concurrent_failure(_internal: String) -> MetaFailure {
    MetaDiagnostic::error(
        MetaDiagnosticCode::ConcurrentModification,
        "sourceSet evidence changed after metadata creation was authorized",
    )
    .into()
}

fn metadata_source_failure(source_set: &str, error: SourceTargetError) -> MetaFailure {
    let code = match error.code {
        SourceTargetErrorCode::SourceSetNotFound => MetaDiagnosticCode::TargetNotFound,
        SourceTargetErrorCode::ContainmentDenied => MetaDiagnosticCode::ProviderUnavailable,
        _ => MetaDiagnosticCode::CapabilityUnavailable,
    };
    let message = match code {
        MetaDiagnosticCode::TargetNotFound => format!("sourceSet `{source_set}` was not found"),
        MetaDiagnosticCode::ProviderUnavailable => {
            format!("sourceSet `{source_set}` could not be resolved safely")
        }
        _ => format!("sourceSet `{source_set}` does not provide Platform XML metadata creation"),
    };
    MetaDiagnostic::error(code, message).into()
}

impl ClosedPlatformXmlTarget {
    pub(crate) fn target_kind(&self) -> TargetKind {
        self.target_kind
    }

    pub(crate) fn search_filters(
        &self,
    ) -> Result<Vec<crate::domain::code_intelligence::RelativeSearchFilter>, SourceTargetError>
    {
        use crate::domain::code_intelligence::RelativeSearchFilter;
        if self.target_kind == TargetKind::SourceRoot {
            return Ok(Vec::new());
        }
        let relative = self
            .target_path
            .strip_prefix(&self.source_root)
            .map_err(|_| target_containment_error(&self.source_target.source_set))?;
        match self.target_kind {
            TargetKind::Module => Ok(vec![RelativeSearchFilter::Exact(relative.to_path_buf())]),
            TargetKind::MetadataObject => Ok(vec![
                RelativeSearchFilter::Exact(relative.to_path_buf()),
                RelativeSearchFilter::Subtree(relative.with_extension("")),
            ]),
            TargetKind::SourceRoot => unreachable!("source roots return before path projection"),
        }
    }
}

/// Which target kinds a caller is prepared to receive. The write surface passes
/// `ModuleOnly` so that widening the resolver can never hand a descriptor or a
/// source root to a writer; read-only callers pass `Any`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TargetKindPolicy {
    ModuleOnly,
    /// Like `ModuleOnly`, but a module whose `*.bsl` does not exist yet still
    /// resolves. The platform omits that file when the module is empty, so its
    /// absence is a fact about the export, not about the address: whether the
    /// role is legitimate is already decided by the kind registry. Only a
    /// writer that materialises the file may ask for this.
    ModuleOnlyAllowingAbsent,
    Any,
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

/// Revalidated resource paths whose field meaning follows the closed handle's
/// target kind:
///
/// - `Module`: `target_path` is the BSL module and `descriptor_paths` contains
///   the descriptors required by its layout; `module_paths` is empty.
/// - `MetadataObject`: `target_path` is the object descriptor and
///   `module_paths` contains its proven owned BSL modules; `descriptor_paths`
///   is empty.
/// - `SourceRoot`: `target_path` is the source root and `descriptor_paths`
///   contains `Configuration.xml`; `module_paths` is empty.
pub(crate) struct PlatformXmlResourceEvidence {
    pub(crate) target_path: PathBuf,
    pub(crate) source_root: PathBuf,
    pub(crate) descriptor_paths: Vec<PathBuf>,
    /// Proven BSL modules owned by a metadata object target. Empty for module
    /// and source-root targets, which are not module owners themselves.
    pub(crate) module_paths: Vec<PathBuf>,
    pub(crate) registration_path: PathBuf,
    pub(crate) module_owner: Option<String>,
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
    policy: TargetKindPolicy,
) -> Result<PlatformXmlResolution, SourceTargetError> {
    if target.source_set.is_empty() {
        return Err(SourceTargetError::new(
            SourceTargetErrorCode::SourceSetRequired,
            "sourceSet must name an exact project source set",
        ));
    }
    let selected = resolve_named_source_set(context, &target.source_set)
        .map_err(|error| public_source_set_error(&target.source_set, error))?;
    resolve_platform_xml_target_in(context, target, policy, selected)
}

/// Resolves another logical observation inside the exact source set already
/// selected for a diagnostics call. The source root is a closed internal
/// handle; no physical identity is added to the public target.
pub(crate) fn resolve_platform_xml_target_in_diagnostic_context(
    context: &WorkspaceContext,
    target: &SourceTarget,
    policy: TargetKindPolicy,
    source_set: &ProjectSourceSet,
    source_root: &Path,
) -> Result<PlatformXmlResolution, SourceTargetError> {
    let selected = ResolvedNamedSourceSet {
        source_set: source_set.clone(),
        lexical_path: source_root.to_path_buf(),
        path: source_root.to_path_buf(),
    };
    resolve_platform_xml_target_in(context, target, policy, selected)
}

/// The same resolution against an already-resolved source set.
///
/// A prefix scan proves thousands of candidates against one source set, and
/// re-reading the project config for each of them was the whole cost of a
/// bare-kind query (#277). The containment chain is unchanged — this takes the
/// value the caller already proved instead of proving it again.
pub(crate) fn resolve_platform_xml_target_in(
    context: &WorkspaceContext,
    target: &SourceTarget,
    policy: TargetKindPolicy,
    selected: ResolvedNamedSourceSet,
) -> Result<PlatformXmlResolution, SourceTargetError> {
    validate_source_set(&selected)?;
    // The policy decides every target kind, including the source root a missing
    // `metadataPath` names. Answering the root before the policy is consulted
    // would make `ModuleOnly` a filter over addresses rather than the
    // fail-closed declaration a writer relies on.
    let address = target.metadata_path.as_ref();
    let target_kind = address.map_or(TargetKind::SourceRoot, MetadataAddress::target_kind);
    match (target_kind, policy) {
        (TargetKind::Module, _) => resolve_platform_xml_module(
            context,
            target,
            selected,
            address.expect("a module kind comes from an address"),
            policy,
        ),
        (TargetKind::MetadataObject, TargetKindPolicy::Any) => resolve_platform_xml_object(
            context,
            target,
            selected,
            address.expect("a metadata object kind comes from an address"),
        ),
        (TargetKind::SourceRoot, TargetKindPolicy::Any) => {
            resolve_platform_xml_root(context, target, selected)
        }
        (TargetKind::SourceRoot, _) => Err(SourceTargetError::new(
            SourceTargetErrorCode::TargetKindMismatch,
            "sourceSet root is not a module terminal",
        )),
        _ => Err(SourceTargetError::new(
            SourceTargetErrorCode::TargetKindMismatch,
            "metadataPath does not identify a module terminal",
        )),
    }
}

fn resolve_platform_xml_module(
    context: &WorkspaceContext,
    target: &SourceTarget,
    selected: ResolvedNamedSourceSet,
    address: &MetadataAddress,
    policy: TargetKindPolicy,
) -> Result<PlatformXmlResolution, SourceTargetError> {
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
    validate_regular_module(
        &target_path,
        &target.source_set,
        matches!(policy, TargetKindPolicy::ModuleOnlyAllowingAbsent),
    )?;
    let target_identity = normalize_path_identity(&target_path)
        .map_err(|_| target_containment_error(&target.source_set))?;
    if !target_identity.starts_with(&selected.path) {
        return Err(target_containment_error(&target.source_set));
    }
    let identity_proven =
        module_descriptor_identity_is_proven(&selected.path, &identity, &CancellationToken::new())
            .map_err(|_| metadata_owner_evidence_error(target))?;
    if !identity_proven {
        return Err(metadata_owner_evidence_error(target));
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
            module_absence_allowed: matches!(policy, TargetKindPolicy::ModuleOnlyAllowingAbsent),
            source_target: target.clone(),
            workspace_root,
            source_root_lexical: selected.lexical_path,
            source_root: selected.path,
            source_set_kind: selected.source_set.kind,
            source_format: selected.source_set.source_format,
            target_kind: TargetKind::Module,
            target_path,
            module_owner: Some(identity.owner),
        },
    })
}

/// Resolves a metadata object to the descriptor that proves it. The descriptor
/// is evidence, never a write target: `TargetKindPolicy::ModuleOnly` keeps every
/// writer out of this branch.
fn resolve_platform_xml_object(
    context: &WorkspaceContext,
    target: &SourceTarget,
    selected: ResolvedNamedSourceSet,
    address: &MetadataAddress,
) -> Result<PlatformXmlResolution, SourceTargetError> {
    let parts = address.segments().collect::<Vec<_>>();
    let relative_path = object_descriptor_evidence(&parts)
        .map(|evidence| evidence.path)
        .ok_or_else(|| {
            SourceTargetError::new(
                SourceTargetErrorCode::MetadataAddressNotFound,
                "metadataPath does not identify a known metadata object",
            )
        })?;
    let target_path = WorkspacePathPolicy::new(context)
        .resolve_write(selected.path.join(&relative_path))
        .map_err(|_| target_containment_error(&target.source_set))?;
    ensure_no_link_components(&selected.path, &target_path)
        .map_err(|_| target_containment_error(&target.source_set))?;
    let target_identity = normalize_path_identity(&target_path)
        .map_err(|_| target_containment_error(&target.source_set))?;
    if !target_identity.starts_with(&selected.path) {
        return Err(target_containment_error(&target.source_set));
    }
    validate_platform_xml_module_descriptors(
        context,
        &selected.path,
        std::slice::from_ref(&relative_path),
    )
    .map_err(|error| public_evidence_error(target, error))?;

    let Some(owner_version) = object_registration_evidence(context, &selected, address)? else {
        return Err(SourceTargetError::new(
            SourceTargetErrorCode::MetadataAddressNotFound,
            format!(
                "metadataPath `{address}` was not found in sourceSet `{}`",
                target.source_set
            ),
        ));
    };
    #[cfg(test)]
    run_object_descriptor_content_read_hook_for_test();
    let descriptor = read_navigation_descriptor(&target_path)
        .map_err(|_| metadata_owner_evidence_error(target))?;
    let parts = address.segments().collect::<Vec<_>>();
    let expected =
        object_descriptor_evidence(&parts).ok_or_else(|| metadata_owner_evidence_error(target))?;
    let (version, name) = descriptor_version_and_name_from_bytes(&descriptor, &expected.kind)
        .map_err(|_| metadata_owner_evidence_error(target))?;
    if name != expected.name {
        return Err(metadata_owner_evidence_error(target));
    }
    if let ObjectOwnerVersionEvidence::Exact(owner_version) = owner_version {
        if version != owner_version {
            return Err(SourceTargetError::source_format_unsupported(format!(
                "metadataPath `{address}` format does not match its proven metadata owner in sourceSet `{}`",
                target.source_set
            )));
        }
    }

    let workspace_root = normalize_path_identity(&context.workspace_root)
        .map_err(|_| target_containment_error(&target.source_set))?;
    Ok(PlatformXmlResolution {
        resolved: ResolvedTarget {
            source_set: selected.source_set.name.clone(),
            metadata_path: Some(address.clone()),
            target_kind: TargetKind::MetadataObject,
        },
        handle: ClosedPlatformXmlTarget {
            module_absence_allowed: false,
            source_target: target.clone(),
            workspace_root,
            source_root_lexical: selected.lexical_path,
            source_root: selected.path,
            source_set_kind: selected.source_set.kind,
            source_format: selected.source_set.source_format,
            target_kind: TargetKind::MetadataObject,
            target_path,
            module_owner: None,
        },
    })
}

enum ObjectOwnerVersionEvidence {
    /// External processors and reports are their own source-set owner, so
    /// there is no distinct descriptor version to compare with the target.
    SelfOwned,
    /// Configuration and extension objects must use the exact raw format
    /// literal declared by the canonical owner. `None` is evidence for a
    /// recognized versionless owner (legacy format 1.0), not a missing owner.
    Exact(Option<String>),
}

fn object_registration_evidence(
    context: &WorkspaceContext,
    selected: &ResolvedNamedSourceSet,
    address: &MetadataAddress,
) -> Result<Option<ObjectOwnerVersionEvidence>, SourceTargetError> {
    if matches!(
        selected.source_set.kind,
        SourceSetKind::ExternalProcessor | SourceSetKind::ExternalReport
    ) {
        return Ok(Some(ObjectOwnerVersionEvidence::SelfOwned));
    }
    let logical_target = SourceTarget {
        source_set: selected.source_set.name.clone(),
        metadata_path: Some(address.clone()),
    };
    let source_owner_relative = PathBuf::from("Configuration.xml");
    validate_platform_xml_module_descriptors(
        context,
        &selected.path,
        std::slice::from_ref(&source_owner_relative),
    )
    .map_err(|error| public_evidence_error(&logical_target, error))?;
    let source_owner_path = selected.path.join(source_owner_relative);
    let source_owner = read_navigation_descriptor(&source_owner_path)
        .map_err(|_| metadata_owner_evidence_error(&logical_target))?;
    let source_owner_evidence =
        crate::infrastructure::platform_xml_owner::prove_already_read_source_set_owner(
            &source_owner_path,
            &source_owner,
            selected.source_set.kind,
        )
        .map_err(|_| metadata_owner_evidence_error(&logical_target))?;
    let source_owner_version = source_owner_evidence.version().map(str::to_owned);

    let parts = address.segments().collect::<Vec<_>>();
    let [owner_kind, owner_name, child_kind @ ("Form" | "Template" | "Command" | "Recalculation"), child_name] =
        parts.as_slice()
    else {
        return match parts.as_slice() {
            [kind, name] if source_owner_evidence.registers(kind, name) => Ok(Some(
                ObjectOwnerVersionEvidence::Exact(source_owner_version),
            )),
            [_, _] => Ok(None),
            _ => Ok(None),
        };
    };
    let owner = object_descriptor_evidence(&[*owner_kind, *owner_name])
        .ok_or_else(|| metadata_owner_evidence_error(&logical_target))?;
    if !source_owner_evidence.registers(&owner.kind, &owner.name) {
        return Ok(None);
    }
    validate_platform_xml_module_descriptors(
        context,
        &selected.path,
        std::slice::from_ref(&owner.path),
    )
    .map_err(|error| public_evidence_error(&logical_target, error))?;
    let owner_path = selected.path.join(&owner.path);
    let owner = read_navigation_descriptor(&owner_path)
        .map_err(|_| metadata_owner_evidence_error(&logical_target))?;
    let (owner_version, actual_name) =
        descriptor_version_and_name_from_bytes(&owner, owner_kind)
            .map_err(|_| metadata_owner_evidence_error(&logical_target))?;
    if actual_name != *owner_name {
        return Err(metadata_owner_evidence_error(&logical_target));
    }
    if owner_version != source_owner_version {
        return Err(SourceTargetError::source_format_unsupported(format!(
            "metadata owner `{}.{}` format does not match sourceSet `{}` owner",
            owner_kind, owner_name, selected.source_set.name
        )));
    }
    let owner = std::str::from_utf8(&owner)
        .map_err(|_| metadata_owner_evidence_error(&logical_target))?
        .trim_start_matches('\u{feff}');
    let document = roxmltree::Document::parse(owner)
        .map_err(|_| metadata_owner_evidence_error(&logical_target))?;
    let owner_node = document
        .root_element()
        .children()
        .find(|node| {
            node.is_element()
                && node.tag_name().namespace() == Some(MD_CLASSES_NS)
                && node.tag_name().name() == *owner_kind
        })
        .ok_or_else(|| metadata_owner_evidence_error(&logical_target))?;
    if owner_node.children().any(|child_objects| {
        child_objects.is_element()
            && child_objects.tag_name().namespace() == Some(MD_CLASSES_NS)
            && child_objects.tag_name().name() == "ChildObjects"
            && child_objects.children().any(|node| {
                node.is_element()
                    && node.tag_name().namespace() == Some(MD_CLASSES_NS)
                    && node.tag_name().name() == *child_kind
                    && node.text().is_some_and(|text| text.trim() == *child_name)
            })
    }) {
        Ok(Some(ObjectOwnerVersionEvidence::Exact(
            source_owner_version,
        )))
    } else {
        Ok(None)
    }
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
            module_absence_allowed: false,
            source_target: target.clone(),
            workspace_root,
            source_root_lexical: selected.lexical_path,
            source_root: selected.path.clone(),
            source_set_kind: selected.source_set.kind,
            source_format: selected.source_set.source_format,
            target_kind: TargetKind::SourceRoot,
            target_path: selected.path,
            module_owner: None,
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

    // A handle revalidates under the policy it was issued under: an object
    // handle can never widen into a module write target, and a module handle
    // never accepts an object in its place.
    let policy = match handle.target_kind {
        TargetKind::Module if handle.module_absence_allowed => {
            TargetKindPolicy::ModuleOnlyAllowingAbsent
        }
        TargetKind::Module => TargetKindPolicy::ModuleOnly,
        TargetKind::MetadataObject | TargetKind::SourceRoot => TargetKindPolicy::Any,
    };
    let current = resolve_platform_xml_target(context, &handle.source_target, policy)?;
    if current.handle.target_path != handle.target_path
        || current.handle.module_owner != handle.module_owner
        || current.handle.target_kind != handle.target_kind
    {
        return Err(source_map_rebind_error(&handle.source_target.source_set));
    }
    Ok(RevalidatedPlatformXmlTarget {
        path: current.handle.target_path,
    })
}

pub(crate) fn platform_xml_resource_evidence(
    context: &WorkspaceContext,
    handle: &ClosedPlatformXmlTarget,
) -> Result<PlatformXmlResourceEvidence, SourceTargetError> {
    let current = revalidate_platform_xml_target(context, handle)?;
    let mut module_paths = Vec::new();
    let descriptor_paths = match handle.target_kind {
        TargetKind::Module => {
            let relative = current
                .path
                .strip_prefix(&handle.source_root)
                .map_err(|_| target_containment_error(&handle.source_target.source_set))?;
            platform_xml_module_identity(relative)
                .map_err(|_| {
                    SourceTargetError::new(
                        SourceTargetErrorCode::MetadataAddressNotFound,
                        "module resource evidence is unavailable",
                    )
                })?
                .descriptors
                .into_iter()
                .map(|path| handle.source_root.join(path))
                .collect()
        }
        TargetKind::MetadataObject => {
            let address = handle
                .source_target
                .metadata_path
                .as_ref()
                .expect("an object handle carries its address");
            module_paths = object_module_paths(context, &handle.source_root, address);
            Vec::new()
        }
        TargetKind::SourceRoot => vec![handle.source_root.join("Configuration.xml")],
    };
    Ok(PlatformXmlResourceEvidence {
        module_paths,
        target_path: current.path,
        source_root: handle.source_root.clone(),
        descriptor_paths,
        registration_path: handle.source_root.join("Configuration.xml"),
        module_owner: handle.module_owner.clone(),
    })
}

/// Module roles that a metadata object can own. Root modules are addressed by a
/// single segment and never hang off an object address, so they are absent here.
const PLATFORM_XML_OBJECT_MODULE_ROLES: &[PlatformXmlModuleRole] = &[
    PlatformXmlModuleRole::Module,
    PlatformXmlModuleRole::ObjectModule,
    PlatformXmlModuleRole::ManagerModule,
    PlatformXmlModuleRole::RecordSetModule,
    PlatformXmlModuleRole::ValueManagerModule,
    PlatformXmlModuleRole::FormModule,
    PlatformXmlModuleRole::CommandModule,
];

/// Renders each module role the object could own, keeps the ones whose layout
/// round-trips back to the same address, and returns only contained regular
/// files. Nothing here scans a directory: an unproven file cannot appear.
fn object_module_paths(
    context: &WorkspaceContext,
    source_root: &Path,
    address: &MetadataAddress,
) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    for role in PLATFORM_XML_OBJECT_MODULE_ROLES {
        let Ok(candidate) = MetadataAddress::parse(
            crate::domain::source_target::PLATFORM_XML_8_3_27_FORMAT_2_20,
            &format!("{address}.{}", role.as_str()),
        ) else {
            continue;
        };
        let Ok(relative) = module_path_for_address(&candidate) else {
            continue;
        };
        let Ok(identity) = platform_xml_module_identity(&relative) else {
            continue;
        };
        if identity.address != candidate {
            continue;
        }
        let Ok(resolved) =
            WorkspacePathPolicy::new(context).resolve_write(source_root.join(&relative))
        else {
            continue;
        };
        if ensure_no_link_components(source_root, &resolved).is_err() {
            continue;
        }
        let Ok(metadata) = fs::symlink_metadata(&resolved) else {
            continue;
        };
        if metadata_is_link_or_reparse_point(&metadata) || !metadata.is_file() {
            continue;
        }
        paths.push(resolved);
    }
    paths
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
            metadata_owner_evidence_error(target)
        }
    }
}

fn metadata_owner_evidence_error(target: &SourceTarget) -> SourceTargetError {
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
        return Err(SourceTargetError::source_format_unsupported(format!(
            "source set `{}` must be a Platform XML Configuration or Extension",
            selected.source_set.name
        )));
    }
    Ok(())
}

fn validate_regular_module(
    path: &Path,
    source_set: &str,
    allow_absent: bool,
) -> Result<(), SourceTargetError> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        // Absent is not malformed: nothing exists to be a link, a directory or
        // the wrong kind of file, so the remaining checks have no subject.
        Err(error) if allow_absent && error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(_) => {
            return Err(SourceTargetError::new(
                SourceTargetErrorCode::MetadataAddressNotFound,
                format!("module target is unavailable in sourceSet `{source_set}`"),
            ))
        }
    };
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
        /// Collection directory holding the child; the child's kind is derived
        /// from the logical address, so it is not repeated here.
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
                .is_some_and(|kind| super::metadata_kinds::supports_owner_module(kind.tag)),
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
            ModuleDescriptorRule::Nested { child_directory } => {
                let kind = captures.kind.ok_or_else(unsupported_module_layout)?;
                let name = captures.owner_name.ok_or_else(unsupported_module_layout)?;
                let child_name = captures.child_name.ok_or_else(unsupported_module_layout)?;
                (
                    format!("{}.{name}", kind.tag),
                    vec![
                        metadata_descriptor(kind.directory, name),
                        // The child's metadata descriptor sits beside its
                        // directory as `<Forms>/<Name>.xml`. The file under
                        // `<Name>/Ext/Form.xml` is the form's own content in the
                        // logform schema, not a `MetaDataObject` descriptor.
                        PathBuf::from(kind.directory)
                            .join(name)
                            .join(child_directory)
                            .join(format!("{child_name}.xml")),
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
    kind: Option<&'static MetadataLayout>,
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
        revalidate_platform_xml_target, set_navigation_provider_entry_hook_for_test,
        set_object_descriptor_content_read_hook_for_test,
    };
    use crate::application::source_navigation::{
        NavigationCompleteness, SourceChildrenRequest, SourceLocation, SourceMatchKind,
        SourceNavigationMode, SourceNodeAddressability, SourceNodeKind, SourceResolveRequest,
    };
    use crate::domain::cancellation::{CancellationToken, CANCELLED_PREFIX};
    use crate::domain::code_intelligence::RelativeSearchFilter;
    use crate::domain::source_target::{
        MetadataAddress, SourceTarget, SourceTargetErrorCode, SourceTargetErrorReason,
        PLATFORM_XML_8_3_27_FORMAT_2_20,
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

    /// The write surface's policy is the default under test: every case that
    /// wants an object target says so explicitly with `TargetKindPolicy::Any`.
    fn resolve_platform_xml_target(
        context: &WorkspaceContext,
        target: &SourceTarget,
    ) -> Result<super::PlatformXmlResolution, crate::domain::source_target::SourceTargetError> {
        super::resolve_platform_xml_target(context, target, super::TargetKindPolicy::ModuleOnly)
    }

    fn resolve_platform_xml_object_target(
        context: &WorkspaceContext,
        target: &SourceTarget,
    ) -> Result<super::PlatformXmlResolution, crate::domain::source_target::SourceTargetError> {
        super::resolve_platform_xml_target(context, target, super::TargetKindPolicy::Any)
    }

    /// A source root is a read-only target: only `Any` admits it, so a root
    /// test that reached for the writer's policy would be testing nothing it
    /// claims to test.
    fn resolve_platform_xml_root_target(
        context: &WorkspaceContext,
        target: &SourceTarget,
    ) -> Result<super::PlatformXmlResolution, crate::domain::source_target::SourceTargetError> {
        super::resolve_platform_xml_target(context, target, super::TargetKindPolicy::Any)
    }

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
                "CommonModule",
                "Shared",
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
            "CommonModule",
            "Shared",
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
            assert_eq!(
                error.reason(),
                SourceTargetErrorReason::SourceFormatUnsupported,
                "{source_set}: {error}"
            );
            assert!(
                error.to_string().starts_with("SourceRootNotAddressable:"),
                "{source_set}: {error}"
            );
            let serialized = serde_json::to_string(&error).unwrap();
            assert!(!serialized.contains("reason"), "{source_set}: {serialized}");
            assert!(
                !serialized.contains("SourceFormatUnsupported"),
                "{source_set}: {serialized}"
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
    fn platform_xml_source_targets_require_exact_owner_descriptor_identity() {
        let context = fixture(
            "exact-owner-descriptor-identity",
            project_yaml("main", "CONFIGURATION", "src"),
        );
        let root = context.workspace_root.join("src");
        write_real_module_fixture(&root, "CommonModule", "Shared");
        write_metadata_descriptor(
            &root,
            "CommonModules",
            "CommonModule",
            "Shared",
            "Different",
        );

        let error =
            resolve_platform_xml_target(&context, &target("main", "CommonModule.Shared.Module"))
                .unwrap_err();

        assert_eq!(error.code, SourceTargetErrorCode::MetadataAddressNotFound);
        assert!(!error
            .message
            .contains(&context.workspace_root.display().to_string()));
        cleanup(&context);
    }

    #[test]
    fn platform_xml_source_targets_require_exact_nested_descriptor_identity() {
        let context = fixture(
            "exact-nested-descriptor-identity",
            project_yaml("main", "CONFIGURATION", "src"),
        );
        let root = context.workspace_root.join("src");
        write_metadata_descriptor(&root, "Catalogs", "Catalog", "Items", "Items");
        write_nested_module_fixture(
            &root,
            "Catalogs/Items/Forms/Order/Ext/Form/Module.bsl",
            "Catalogs/Items/Forms/Order.xml",
            "Form",
            "Different",
        );

        let error = resolve_platform_xml_target(
            &context,
            &target("main", "Catalog.Items.Form.Order.FormModule"),
        )
        .unwrap_err();

        assert_eq!(error.code, SourceTargetErrorCode::MetadataAddressNotFound);
        cleanup(&context);
    }

    #[test]
    fn platform_xml_source_target_revalidation_rejects_changed_descriptor_identity() {
        let context = fixture(
            "changed-descriptor-identity",
            project_yaml("main", "CONFIGURATION", "src"),
        );
        let root = context.workspace_root.join("src");
        write_real_module_fixture(&root, "CommonModule", "Shared");
        let resolution =
            resolve_platform_xml_target(&context, &target("main", "CommonModule.Shared.Module"))
                .unwrap();
        write_metadata_descriptor(
            &root,
            "CommonModules",
            "CommonModule",
            "Shared",
            "Different",
        );

        let error = revalidate_platform_xml_target(&context, &resolution.handle).unwrap_err();

        assert_eq!(error.code, SourceTargetErrorCode::MetadataAddressNotFound);
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
            "CommonModule",
            "Shared",
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
                "CommonModule",
                "Shared",
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
        write_configuration_descriptor(&root, false);
        for (directory, kind, name) in [
            ("CommonModules", "CommonModule", "Shared"),
            ("Catalogs", "Catalog", "Items"),
            ("InformationRegisters", "InformationRegister", "Prices"),
            ("Constants", "Constant", "Mode"),
            ("CommonForms", "CommonForm", "Main"),
            ("CommonCommands", "CommonCommand", "Print"),
        ] {
            write_metadata_descriptor(&root, directory, kind, name, name);
        }
        write_nested_module_fixture(
            &root,
            "Catalogs/Items/Forms/List/Ext/Form/Module.bsl",
            "Catalogs/Items/Forms/List.xml",
            "Form",
            "List",
        );
        write_nested_module_fixture(
            &root,
            "Catalogs/Items/Commands/Open/Ext/CommandModule.bsl",
            "Catalogs/Items/Commands/Open.xml",
            "Command",
            "Open",
        );
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
            "Language",
            "Russian",
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
            "CommonModule",
            "Shared",
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
            "CommonModule",
            "Shared",
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
                resolve_platform_xml_root_target(&context, &root_target(source_set)).unwrap();

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

    /// `ModuleOnly` is a fail-closed declaration, not a filter on addresses that
    /// happen to carry a `metadataPath`: a source-root target must be refused by
    /// the same barrier that refuses a descriptor.
    #[test]
    fn platform_xml_module_only_policy_refuses_a_source_root_target() {
        let context = fixture(
            "module-only-source-root",
            project_yaml("main", "CONFIGURATION", "src"),
        );
        fs::create_dir_all(context.workspace_root.join("src")).unwrap();

        let allowed = resolve_platform_xml_object_target(&context, &root_target("main"))
            .expect("a read-only caller still resolves the source root under `Any`");
        assert_eq!(
            allowed.resolved.target_kind,
            crate::domain::source_target::TargetKind::SourceRoot
        );

        let error = resolve_platform_xml_target(&context, &root_target("main")).unwrap_err();

        assert_eq!(error.code, SourceTargetErrorCode::TargetKindMismatch);
        assert_eq!(error.message, "sourceSet root is not a module terminal");
        let serialized = serde_json::to_string(&error).unwrap();
        assert!(!serialized.contains(&context.workspace_root.display().to_string()));
        assert!(!serialized.contains("src"));
        cleanup(&context);
    }

    /// The complete `(target kind, policy)` table, so widening one cell cannot
    /// pass unnoticed because only its own scenario was covered.
    #[test]
    fn platform_xml_target_kind_policy_table_is_closed() {
        let context = fixture(
            "target-kind-policy-table",
            project_yaml("main", "CONFIGURATION", "src"),
        );
        let root = context.workspace_root.join("src");
        write_module_fixture(
            &root,
            "CommonModules/Shared.xml",
            "CommonModules/Shared/Ext/Module.bsl",
            "CommonModule",
            "Shared",
        );
        write_metadata_descriptor(&root, "Catalogs", "Catalog", "Items", "Items");

        let root_target = root_target("main");
        let object_target = target("main", "Catalog.Items");
        let module_target = target("main", "CommonModule.Shared.Module");
        let cases = [
            (
                "source root",
                &root_target,
                super::TargetKindPolicy::Any,
                true,
            ),
            (
                "source root",
                &root_target,
                super::TargetKindPolicy::ModuleOnly,
                false,
            ),
            (
                "source root",
                &root_target,
                super::TargetKindPolicy::ModuleOnlyAllowingAbsent,
                false,
            ),
            (
                "metadata object",
                &object_target,
                super::TargetKindPolicy::Any,
                true,
            ),
            (
                "metadata object",
                &object_target,
                super::TargetKindPolicy::ModuleOnly,
                false,
            ),
            (
                "metadata object",
                &object_target,
                super::TargetKindPolicy::ModuleOnlyAllowingAbsent,
                false,
            ),
            ("module", &module_target, super::TargetKindPolicy::Any, true),
            (
                "module",
                &module_target,
                super::TargetKindPolicy::ModuleOnly,
                true,
            ),
            (
                "module",
                &module_target,
                super::TargetKindPolicy::ModuleOnlyAllowingAbsent,
                true,
            ),
        ];

        for (label, source_target, policy, accepted) in cases {
            let outcome = super::resolve_platform_xml_target(&context, source_target, policy);
            match (accepted, outcome) {
                (true, Ok(_)) => {}
                (false, Err(error)) => assert_eq!(
                    error.code,
                    SourceTargetErrorCode::TargetKindMismatch,
                    "{label} under {policy:?} must fail as a kind mismatch"
                ),
                (true, Err(error)) => {
                    panic!("{label} under {policy:?} must resolve, got {error:?}")
                }
                (false, Ok(_)) => panic!("{label} under {policy:?} must not resolve"),
            }
        }
        cleanup(&context);
    }

    /// A handle revalidates under the policy that issued it, so a source-root
    /// handle keeps resolving while a module handle can never widen into one.
    #[test]
    fn platform_xml_source_root_handle_revalidates_without_widening() {
        let context = fixture(
            "source-root-rebind",
            project_yaml("main", "CONFIGURATION", "src"),
        );
        let root = context.workspace_root.join("src");
        write_module_fixture(
            &root,
            "CommonModules/Shared.xml",
            "CommonModules/Shared/Ext/Module.bsl",
            "CommonModule",
            "Shared",
        );

        let root_handle = resolve_platform_xml_object_target(&context, &root_target("main"))
            .unwrap()
            .handle;
        assert_eq!(
            revalidate_platform_xml_target(&context, &root_handle)
                .unwrap()
                .path,
            normalize_path_identity(&root).unwrap()
        );

        let module_handle =
            resolve_platform_xml_target(&context, &target("main", "CommonModule.Shared.Module"))
                .unwrap()
                .handle;
        assert_eq!(
            module_handle.target_kind(),
            crate::domain::source_target::TargetKind::Module
        );
        assert_ne!(
            revalidate_platform_xml_target(&context, &module_handle)
                .unwrap()
                .path,
            normalize_path_identity(&root).unwrap()
        );
        cleanup(&context);
    }

    #[test]
    fn write_target_kind_and_revalidation_contract_is_complete() {
        platform_xml_target_kind_policy_table_is_closed();
        platform_xml_source_root_handle_revalidates_without_widening();
        platform_xml_source_target_revalidation_rejects_changed_descriptor_identity();
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

        let error = resolve_platform_xml_root_target(&context, &root_target("main")).unwrap_err();

        // The source set itself is refused before the target-kind policy is
        // consulted, so the message names the root, not the terminal.
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
        let resolution = resolve_platform_xml_root_target(&context, &root_target("main")).unwrap();
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
            "CommonModule",
            "Shared",
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
            "Catalogs/Items/Forms/Order.xml",
            "Form",
            "DifferentForm",
        );
        write_nested_module_fixture(
            &root,
            "Catalogs/Items/Commands/Print/Ext/CommandModule.bsl",
            "Catalogs/Items/Commands/Print.xml",
            "Form",
            "Print",
        );
        write_nested_module_fixture_xml(
            &root,
            "Catalogs/Items/Forms/Legacy/Ext/Form/Module.bsl",
            "Catalogs/Items/Forms/Legacy.xml",
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.19"><Form><Properties><Name>Legacy</Name></Properties></Form></MetaDataObject>"#,
        );
        write_nested_module_fixture_xml(
            &root,
            "Catalogs/Items/Commands/NoNamespace/Ext/CommandModule.bsl",
            "Catalogs/Items/Commands/NoNamespace.xml",
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

    /// #277. A bare-kind prefix query proves every candidate of the kind, and
    /// each proof used to re-read the project config. On a vendor-class
    /// configuration that is thousands of re-parses and 2.5-4.4 s per query.
    /// The source set is resolved once per call, whatever the candidate count.
    #[test]
    fn prefix_scan_resolves_the_source_set_once_per_call() {
        let context = fixture(
            "navigation-prefix-source-map",
            project_yaml("main", "CONFIGURATION", "src"),
        );
        let root = context.workspace_root.join("src");
        write_configuration_descriptor(&root, false);
        for name in ["Items", "Goods", "Partners", "Orders"] {
            write_metadata_descriptor(&root, "Catalogs", "Catalog", name, name);
            fs::create_dir_all(root.join(format!("Catalogs/{name}/Ext"))).unwrap();
            fs::write(
                root.join(format!("Catalogs/{name}/Ext/ManagerModule.bsl")),
                "Procedure Run()\nEndProcedure\n",
            )
            .unwrap();
        }

        crate::infrastructure::source_roots::NAMED_SOURCE_SET_RESOLUTIONS
            .with(|count| count.set(0));
        let answer = resolve_platform_xml_source_navigation(
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
        let resolutions = crate::infrastructure::source_roots::NAMED_SOURCE_SET_RESOLUTIONS
            .with(|count| count.get());

        assert!(
            answer.candidates.len() >= 4,
            "the scan still answers: {:?}",
            answer.candidates
        );
        assert_eq!(
            resolutions, 1,
            "the project config is read once per call, not once per candidate"
        );
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
    fn exact_resolution_answers_without_enumerating_the_source_set() {
        let context = fixture(
            "navigation-exact-no-walk",
            project_yaml("main", "CONFIGURATION", "src"),
        );
        let root = context.workspace_root.join("src");
        write_configuration_descriptor(&root, false);
        write_real_module_fixture(&root, "CommonModule", "Shared");

        let enumerated = std::rc::Rc::new(std::cell::Cell::new(false));
        let observer = std::rc::Rc::clone(&enumerated);
        set_navigation_provider_entry_hook_for_test(move || observer.set(true));

        for (query, expected) in [
            ("CommonModule.Shared.Module", 1),
            ("CommonModule.Shared", 1),
        ] {
            let result = resolve_platform_xml_source_navigation(
                &context,
                &SourceResolveRequest {
                    source_set: "main".to_string(),
                    query: query.to_string(),
                    mode: SourceNavigationMode::Exact,
                    target_kind: None,
                    limit: 50,
                    cursor: None,
                },
            )
            .unwrap();
            assert_eq!(result.candidates.len(), expected, "{query}");
            assert_eq!(
                result.completeness,
                NavigationCompleteness::Complete,
                "{query}"
            );
        }

        assert!(
            !enumerated.get(),
            "exact resolution must render its candidate, not enumerate the source set"
        );
        cleanup(&context);
    }

    #[test]
    fn nested_child_descriptor_sits_beside_its_content_directory() {
        // A real Designer export keeps the form's `MetaDataObject` descriptor at
        // `Forms/<Name>.xml`; `Forms/<Name>/Ext/Form.xml` is the form content in
        // the logform schema. Reading the content file as the descriptor made
        // every form and command module unaddressable in real configurations.
        let context = fixture(
            "navigation-nested-descriptor",
            project_yaml("main", "CONFIGURATION", "src"),
        );
        let root = context.workspace_root.join("src");
        write_configuration_descriptor(&root, false);
        fs::create_dir_all(root.join("Catalogs/Items/Forms/Order/Ext/Form")).unwrap();
        fs::write(
            root.join("Catalogs/Items.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Catalog><Properties><Name>Items</Name></Properties></Catalog></MetaDataObject>"#,
        )
        .unwrap();
        fs::write(
            root.join("Catalogs/Items/Forms/Order.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Form><Properties><Name>Order</Name></Properties></Form></MetaDataObject>"#,
        )
        .unwrap();
        // The content file must not be mistaken for the descriptor.
        fs::write(
            root.join("Catalogs/Items/Forms/Order/Ext/Form.xml"),
            r#"<Form xmlns="http://v8.1c.ru/8.3/xcf/logform"><Items/></Form>"#,
        )
        .unwrap();
        fs::write(
            root.join("Catalogs/Items/Forms/Order/Ext/Form/Module.bsl"),
            "Procedure Run()\nEndProcedure\n",
        )
        .unwrap();

        let result = resolve_platform_xml_source_navigation(
            &context,
            &SourceResolveRequest {
                source_set: "main".to_string(),
                query: "Catalog.Items.Form.Order.FormModule".to_string(),
                mode: SourceNavigationMode::Exact,
                target_kind: None,
                limit: 50,
                cursor: None,
            },
        )
        .unwrap();

        assert_eq!(result.completeness, NavigationCompleteness::Complete);
        assert_eq!(
            result
                .candidates
                .iter()
                .map(|candidate| candidate.metadata_path.as_str())
                .collect::<Vec<_>>(),
            ["Catalog.Items.Form.Order.FormModule"]
        );
        cleanup(&context);
    }

    #[test]
    fn prefix_scan_reaches_nested_children_and_says_when_it_did_not() {
        let context = fixture(
            "navigation-prefix-nested",
            project_yaml("main", "CONFIGURATION", "src"),
        );
        let root = context.workspace_root.join("src");
        write_configuration_descriptor(&root, false);
        fs::create_dir_all(root.join("Catalogs/Items/Forms/Order/Ext/Form")).unwrap();
        fs::write(
            root.join("Catalogs/Items.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Catalog><Properties><Name>Items</Name></Properties></Catalog></MetaDataObject>"#,
        )
        .unwrap();
        fs::write(
            root.join("Catalogs/Items/Forms/Order.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Form><Properties><Name>Order</Name></Properties></Form></MetaDataObject>"#,
        )
        .unwrap();
        fs::create_dir_all(root.join("Catalogs/Items/Templates")).unwrap();
        fs::write(
            root.join("Catalogs/Items/Templates/Print.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Template><Properties><Name>Print</Name></Properties></Template></MetaDataObject>"#,
        )
        .unwrap();
        fs::write(
            root.join("Catalogs/Items/Forms/Order/Ext/Form/Module.bsl"),
            "Procedure Run()\nEndProcedure\n",
        )
        .unwrap();

        let resolve = |query: &str| {
            resolve_platform_xml_source_navigation(
                &context,
                &SourceResolveRequest {
                    source_set: "main".to_string(),
                    query: query.to_string(),
                    mode: SourceNavigationMode::Prefix,
                    target_kind: None,
                    limit: 50,
                    cursor: None,
                },
            )
            .unwrap()
        };

        // A prefix that can reach a nested Form must return it.
        let owner = resolve("Catalog.Items");
        let found = owner
            .candidates
            .iter()
            .map(|candidate| candidate.metadata_path.as_str())
            .collect::<Vec<_>>();
        assert!(found.contains(&"Catalog.Items.Form.Order"), "{found:?}");
        assert!(found.contains(&"Catalog.Items.Template.Print"), "{found:?}");
        assert!(
            found.contains(&"Catalog.Items.Form.Order.FormModule"),
            "{found:?}"
        );
        assert_eq!(owner.completeness, NavigationCompleteness::Complete);

        let nested = resolve("Catalog.Items.Form.Ord");
        assert_eq!(
            nested
                .candidates
                .iter()
                .map(|candidate| candidate.metadata_path.as_str())
                .collect::<Vec<_>>(),
            [
                "Catalog.Items.Form.Order",
                "Catalog.Items.Form.Order.FormModule"
            ]
        );

        let template = resolve("Catalog.Items.Template.Pr");
        assert_eq!(
            template
                .candidates
                .iter()
                .map(|candidate| candidate.metadata_path.as_str())
                .collect::<Vec<_>>(),
            ["Catalog.Items.Template.Print"]
        );

        // A bare-kind prefix does not descend, and says so instead of claiming
        // its answer is complete.
        assert_eq!(
            resolve("Catalog").completeness,
            NavigationCompleteness::Partial
        );
        cleanup(&context);
    }

    #[test]
    fn locate_recovers_the_address_that_owns_a_source_path() {
        let context = fixture(
            "navigation-locate",
            project_yaml("main", "CONFIGURATION", "src"),
        );
        let root = context.workspace_root.join("src");
        write_configuration_descriptor(&root, false);
        write_real_module_fixture(&root, "CommonModule", "Shared");

        let locate = |path: &str| {
            super::locate_platform_xml_source_path(
                &context,
                &crate::application::source_navigation::SourceLocateRequest {
                    source_set: "main".to_string(),
                    path: path.to_string(),
                },
                &CancellationToken::new(),
            )
            .unwrap()
        };

        let module = locate("src/CommonModules/Shared/Ext/Module.bsl");
        assert_eq!(
            module.metadata_path.as_ref().map(MetadataAddress::as_str),
            Some("CommonModule.Shared.Module")
        );
        assert_eq!(
            module
                .owner_metadata_path
                .as_ref()
                .map(MetadataAddress::as_str),
            Some("CommonModule.Shared")
        );
        assert!(module.rejection.is_none());
        // The same file addressed relative to the source set resolves alike.
        assert_eq!(
            locate("CommonModules/Shared/Ext/Module.bsl").metadata_path,
            module.metadata_path
        );

        let descriptor = locate("src/CommonModules/Shared.xml");
        assert_eq!(
            descriptor
                .metadata_path
                .as_ref()
                .map(MetadataAddress::as_str),
            Some("CommonModule.Shared")
        );

        let outside = locate("/etc/passwd");
        assert_eq!(
            outside.rejection,
            Some(crate::application::source_navigation::LocateRejection::OutsideSourceSet)
        );
        assert!(outside.metadata_path.is_none());
        assert!(outside.owner_metadata_path.is_none());
        assert_eq!(
            outside.relative_path, "",
            "a path outside the source set has no relative form to echo"
        );
        cleanup(&context);
    }

    #[test]
    fn reader_locator_preserves_old_format_identity_without_widening_public_locate() {
        let context = fixture(
            "reader-locate-consistent-old-format",
            project_yaml("main", "CONFIGURATION", "src"),
        );
        let root = context.workspace_root.join("src");
        fs::create_dir_all(root.join("Catalogs")).unwrap();
        fs::write(
            root.join("Configuration.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.19"><Configuration><ChildObjects><Catalog>Items</Catalog></ChildObjects></Configuration></MetaDataObject>"#,
        )
        .unwrap();
        fs::write(
            root.join("Catalogs/Items.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.19"><Catalog><Properties><Name>Items</Name></Properties></Catalog></MetaDataObject>"#,
        )
        .unwrap();
        let request = crate::application::source_navigation::SourceLocateRequest {
            source_set: "main".to_string(),
            path: "src/Catalogs/Items.xml".to_string(),
        };

        let public =
            super::locate_platform_xml_source_path(&context, &request, &CancellationToken::new())
                .unwrap();
        assert_eq!(
            public.rejection,
            Some(crate::application::source_navigation::LocateRejection::OwnerUnproven)
        );

        let reader =
            super::locate_platform_xml_reader_path(&context, &request, &CancellationToken::new())
                .unwrap();
        assert_eq!(
            reader.metadata_path.as_ref().map(MetadataAddress::as_str),
            Some("Catalog.Items")
        );
        assert!(reader.rejection.is_none());
        cleanup(&context);
    }

    #[test]
    fn children_answer_one_level_without_enumerating_the_source_set() {
        let context = fixture(
            "navigation-children-no-walk",
            project_yaml("main", "CONFIGURATION", "src"),
        );
        let root = context.workspace_root.join("src");
        write_configuration_descriptor(&root, false);
        write_real_module_fixture(&root, "CommonModule", "Shared");

        let enumerated = std::rc::Rc::new(std::cell::Cell::new(false));
        let observer = std::rc::Rc::clone(&enumerated);
        set_navigation_provider_entry_hook_for_test(move || observer.set(true));

        let result = children_platform_xml_source_navigation(
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

        assert_eq!(result.completeness, NavigationCompleteness::Complete);
        assert_eq!(
            result
                .children
                .iter()
                .filter_map(|child| child.metadata_path.as_ref().map(MetadataAddress::as_str))
                .collect::<Vec<_>>(),
            ["CommonModule.Shared.Module"]
        );
        assert!(
            !enumerated.get(),
            "children descends one level and must not enumerate the source set"
        );
        cleanup(&context);
    }

    #[test]
    fn children_separate_an_absent_parent_from_an_unprovable_one() {
        let context = fixture(
            "navigation-children-absent",
            project_yaml("main", "CONFIGURATION", "src"),
        );
        let root = context.workspace_root.join("src");
        write_configuration_descriptor(&root, false);
        write_real_module_fixture(&root, "CommonModule", "Shared");

        let absent = children_platform_xml_source_navigation(
            &context,
            &SourceChildrenRequest {
                source_set: "main".to_string(),
                metadata_path: Some(
                    MetadataAddress::parse(PLATFORM_XML_8_3_27_FORMAT_2_20, "CommonModule.Missing")
                        .unwrap(),
                ),
                limit: 50,
                cursor: None,
            },
        )
        .unwrap_err();
        assert!(absent.contains("was not found"), "{absent}");

        fs::write(
            root.join("CommonModules/Shared.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><CommonModule><Properties><Name>Different</Name></Properties></CommonModule></MetaDataObject>"#,
        )
        .unwrap();
        let unproven = children_platform_xml_source_navigation(
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
        .unwrap_err();
        assert!(
            unproven.contains("could not be proven"),
            "an unprovable parent must not be reported as absent: {unproven}"
        );
        cleanup(&context);
    }

    #[test]
    fn exact_resolution_reports_a_missing_address_as_a_complete_absence() {
        let context = fixture(
            "navigation-exact-absent",
            project_yaml("main", "CONFIGURATION", "src"),
        );
        let root = context.workspace_root.join("src");
        write_configuration_descriptor(&root, false);
        write_real_module_fixture(&root, "CommonModule", "Shared");

        let result = resolve_platform_xml_source_navigation(
            &context,
            &SourceResolveRequest {
                source_set: "main".to_string(),
                query: "CommonModule.Missing.Module".to_string(),
                mode: SourceNavigationMode::Exact,
                target_kind: None,
                limit: 50,
                cursor: None,
            },
        )
        .unwrap();

        assert!(result.candidates.is_empty());
        assert_eq!(
            result.completeness,
            NavigationCompleteness::Complete,
            "an address that is simply not there is a definitive answer"
        );
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

    fn write_module_fixture(
        root: &Path,
        descriptor: &str,
        module: &str,
        descriptor_kind: &str,
        descriptor_name: &str,
    ) {
        let descriptor = root.join(descriptor);
        let module = root.join(module);
        fs::create_dir_all(descriptor.parent().unwrap()).unwrap();
        fs::create_dir_all(module.parent().unwrap()).unwrap();
        fs::write(
            descriptor,
            format!(
                r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><{descriptor_kind}><Properties><Name>{descriptor_name}</Name></Properties></{descriptor_kind}></MetaDataObject>"#
            ),
        )
        .unwrap();
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

    #[test]
    fn platform_xml_object_target_resolves_to_its_descriptor() {
        let context = fixture(
            "object-target",
            project_yaml("main", "CONFIGURATION", "src"),
        );
        let root = context.workspace_root.join("src");
        write_metadata_descriptor(&root, "Catalogs", "Catalog", "Items", "Items");

        let resolution =
            resolve_platform_xml_object_target(&context, &target("main", "Catalog.Items")).unwrap();

        assert_eq!(
            resolution.resolved.target_kind,
            crate::domain::source_target::TargetKind::MetadataObject
        );
        assert_eq!(
            resolution.resolved.metadata_path.unwrap().as_str(),
            "Catalog.Items"
        );
        assert_eq!(
            resolution.handle.target_path,
            normalize_path_identity(&root.join("Catalogs/Items.xml")).unwrap()
        );
        assert!(resolution.handle.module_owner.is_none());
        cleanup(&context);
    }

    #[test]
    fn platform_xml_recalculation_target_resolves_to_nested_descriptor() {
        let context = fixture(
            "recalculation-object-target",
            project_yaml("main", "CONFIGURATION", "src"),
        );
        let root = context.workspace_root.join("src");
        fs::create_dir_all(root.join("CalculationRegisters/Payroll/Recalculations")).unwrap();
        fs::write(
            root.join("Configuration.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Configuration><ChildObjects><CalculationRegister>Payroll</CalculationRegister></ChildObjects></Configuration></MetaDataObject>"#,
        )
        .unwrap();
        fs::write(
            root.join("CalculationRegisters/Payroll.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><CalculationRegister><Properties><Name>Payroll</Name></Properties><ChildObjects><Recalculation>Main</Recalculation></ChildObjects></CalculationRegister></MetaDataObject>"#,
        )
        .unwrap();
        fs::write(
            root.join("CalculationRegisters/Payroll/Recalculations/Main.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Recalculation><Properties><Name>Main</Name></Properties></Recalculation></MetaDataObject>"#,
        )
        .unwrap();

        let resolution = resolve_platform_xml_object_target(
            &context,
            &target("main", "CalculationRegister.Payroll.Recalculation.Main"),
        )
        .unwrap();

        assert_eq!(
            resolution.handle.target_path,
            normalize_path_identity(
                &root.join("CalculationRegisters/Payroll/Recalculations/Main.xml")
            )
            .unwrap()
        );
        cleanup(&context);
    }

    #[test]
    fn platform_xml_object_target_is_refused_by_the_module_only_policy() {
        let context = fixture(
            "object-target-module-only",
            project_yaml("main", "CONFIGURATION", "src"),
        );
        let root = context.workspace_root.join("src");
        write_metadata_descriptor(&root, "Catalogs", "Catalog", "Items", "Items");

        let error =
            resolve_platform_xml_target(&context, &target("main", "Catalog.Items")).unwrap_err();

        assert_eq!(error.code, SourceTargetErrorCode::TargetKindMismatch);
        assert!(
            error.message.contains("module terminal"),
            "{}",
            error.message
        );
        cleanup(&context);
    }

    #[test]
    fn platform_xml_target_kind_policy_applies_to_every_target_kind() {
        let context = fixture(
            "policy-covers-root",
            project_yaml("main", "CONFIGURATION", "src"),
        );
        let root = context.workspace_root.join("src");
        write_module_fixture(
            &root,
            "CommonModules/Shared.xml",
            "CommonModules/Shared/Ext/Module.bsl",
            "CommonModule",
            "Shared",
        );
        write_metadata_descriptor(&root, "Catalogs", "Catalog", "Items", "Items");

        let root_target = root_target("main");
        let object = target("main", "Catalog.Items");
        let module = target("main", "CommonModule.Shared.Module");

        // `Any` is the read policy: every kind resolves.
        for candidate in [&root_target, &object, &module] {
            assert!(
                super::resolve_platform_xml_target(
                    &context,
                    candidate,
                    super::TargetKindPolicy::Any
                )
                .is_ok(),
                "Any must resolve every target kind"
            );
        }

        // `ModuleOnly` is a fail-closed declaration: it admits the module
        // terminal and refuses every other kind, the source root included.
        assert!(super::resolve_platform_xml_target(
            &context,
            &module,
            super::TargetKindPolicy::ModuleOnly
        )
        .is_ok());
        for candidate in [&root_target, &object] {
            let error = super::resolve_platform_xml_target(
                &context,
                candidate,
                super::TargetKindPolicy::ModuleOnly,
            )
            .expect_err("ModuleOnly must refuse a non-module target");
            assert_eq!(error.code, SourceTargetErrorCode::TargetKindMismatch);
            assert!(
                !error.message.contains(std::path::MAIN_SEPARATOR),
                "a refusal must not disclose a physical path: {}",
                error.message
            );
        }
        cleanup(&context);
    }

    #[test]
    fn platform_xml_nested_object_target_resolves_to_its_child_descriptor() {
        let context = fixture(
            "object-target-nested",
            project_yaml("main", "CONFIGURATION", "src"),
        );
        let root = context.workspace_root.join("src");
        write_metadata_descriptor(&root, "Catalogs", "Catalog", "Items", "Items");
        write_nested_module_fixture(
            &root,
            "Catalogs/Items/Forms/List/Ext/Form/Module.bsl",
            "Catalogs/Items/Forms/List.xml",
            "Form",
            "List",
        );

        let resolution = resolve_platform_xml_object_target(
            &context,
            &target("main", "Catalog.Items.Form.List"),
        )
        .unwrap();

        assert_eq!(
            resolution.handle.target_path,
            normalize_path_identity(&root.join("Catalogs/Items/Forms/List.xml")).unwrap()
        );
        cleanup(&context);
    }

    #[test]
    fn platform_xml_nested_object_targets_accept_canonical_form_and_command_owners() {
        for (label, child_kind, collection, child_name) in [
            ("form", "Form", "Forms", "List"),
            ("command", "Command", "Commands", "Post"),
        ] {
            let context = fixture(
                &format!("object-target-nested-canonical-{label}"),
                project_yaml("main", "CONFIGURATION", "src"),
            );
            let root = context.workspace_root.join("src");
            write_metadata_descriptor(&root, "Catalogs", "Catalog", "Items", "Items");
            let descriptor = format!("Catalogs/Items/{collection}/{child_name}.xml");
            write_nested_module_fixture(
                &root,
                &format!("Catalogs/Items/{collection}/{child_name}/Ext/Module.bsl"),
                &descriptor,
                child_kind,
                child_name,
            );

            let resolution = resolve_platform_xml_object_target(
                &context,
                &target("main", &format!("Catalog.Items.{child_kind}.{child_name}")),
            )
            .unwrap();

            assert_eq!(
                resolution.handle.target_path,
                normalize_path_identity(&root.join(descriptor)).unwrap()
            );
            cleanup(&context);
        }
    }

    #[test]
    fn platform_xml_nested_object_target_requires_a_proven_owner() {
        let context = fixture(
            "object-target-nested-owner",
            project_yaml("main", "CONFIGURATION", "src"),
        );
        let root = context.workspace_root.join("src");
        write_nested_module_fixture(
            &root,
            "Catalogs/Items/Forms/List/Ext/Form/Module.bsl",
            "Catalogs/Items/Forms/List.xml",
            "Form",
            "List",
        );

        let error = resolve_platform_xml_object_target(
            &context,
            &target("main", "Catalog.Items.Form.List"),
        )
        .unwrap_err();

        assert_eq!(error.code, SourceTargetErrorCode::MetadataAddressNotFound);
        cleanup(&context);
    }

    #[test]
    fn platform_xml_object_target_rejects_a_descriptor_naming_another_object() {
        let context = fixture(
            "object-target-mismatch",
            project_yaml("main", "CONFIGURATION", "src"),
        );
        let root = context.workspace_root.join("src");
        write_metadata_descriptor(&root, "Catalogs", "Catalog", "Items", "Goods");

        let error = resolve_platform_xml_object_target(&context, &target("main", "Catalog.Items"))
            .unwrap_err();

        assert_eq!(error.code, SourceTargetErrorCode::MetadataAddressNotFound);
        cleanup(&context);
    }

    #[test]
    fn platform_xml_object_target_reports_a_missing_descriptor_as_not_found() {
        let context = fixture(
            "object-target-missing",
            project_yaml("main", "CONFIGURATION", "src"),
        );
        let root = context.workspace_root.join("src");
        write_metadata_descriptor(&root, "Catalogs", "Catalog", "Goods", "Goods");

        let error = resolve_platform_xml_object_target(&context, &target("main", "Catalog.Items"))
            .unwrap_err();

        assert_eq!(error.code, SourceTargetErrorCode::MetadataAddressNotFound);
        cleanup(&context);
    }

    #[test]
    fn platform_xml_object_target_does_not_classify_an_unregistered_wrong_format_partial() {
        let context = fixture(
            "object-target-unregistered-wrong-format",
            project_yaml("main", "CONFIGURATION", "src"),
        );
        let descriptor = context.workspace_root.join("src/Catalogs/Items.xml");
        fs::create_dir_all(descriptor.parent().unwrap()).unwrap();
        fs::write(
            &descriptor,
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.19"><Catalog><Properties><Name>Items</Name></Properties></Catalog></MetaDataObject>"#,
        )
        .unwrap();
        let content_read = std::rc::Rc::new(std::cell::Cell::new(false));
        let observer = std::rc::Rc::clone(&content_read);
        set_object_descriptor_content_read_hook_for_test(move || observer.set(true));

        let error = resolve_platform_xml_object_target(&context, &target("main", "Catalog.Items"))
            .unwrap_err();

        assert_eq!(error.code, SourceTargetErrorCode::MetadataAddressNotFound);
        assert_eq!(error.reason(), SourceTargetErrorReason::General);
        assert!(
            !content_read.get(),
            "unregistered descriptor content was read before owner registration proof"
        );
        super::OBJECT_DESCRIPTOR_CONTENT_READ_HOOK.with(|slot| {
            slot.borrow_mut().take();
        });
        cleanup(&context);
    }

    #[test]
    fn platform_xml_object_target_proves_a_regular_file_before_any_descriptor_content_read() {
        let context = fixture(
            "object-target-non-file-call-order",
            project_yaml("main", "CONFIGURATION", "src"),
        );
        let descriptor = context.workspace_root.join("src/Catalogs/Items.xml");
        fs::create_dir_all(&descriptor).unwrap();
        let content_read = std::rc::Rc::new(std::cell::Cell::new(false));
        let observer = std::rc::Rc::clone(&content_read);
        set_object_descriptor_content_read_hook_for_test(move || observer.set(true));

        let error = resolve_platform_xml_object_target(&context, &target("main", "Catalog.Items"))
            .unwrap_err();

        assert_eq!(error.code, SourceTargetErrorCode::MetadataAddressNotFound);
        assert!(
            !content_read.get(),
            "descriptor content was read before regular-file evidence"
        );
        super::OBJECT_DESCRIPTOR_CONTENT_READ_HOOK.with(|slot| {
            slot.borrow_mut().take();
        });
        cleanup(&context);
    }

    #[test]
    fn platform_xml_object_target_rejects_a_registration_lookalike_outside_the_exact_owner() {
        let context = fixture(
            "object-target-registration-lookalike",
            project_yaml("main", "CONFIGURATION", "src"),
        );
        let root = context.workspace_root.join("src");
        fs::create_dir_all(root.join("Catalogs")).unwrap();
        fs::write(
            root.join("Catalogs/Items.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Catalog><Properties><Name>Items</Name></Properties></Catalog></MetaDataObject>"#,
        )
        .unwrap();
        fs::write(
            root.join("Configuration.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Bogus><ChildObjects><Catalog>Items</Catalog></ChildObjects></Bogus></MetaDataObject>"#,
        )
        .unwrap();

        let error = resolve_platform_xml_object_target(&context, &target("main", "Catalog.Items"))
            .unwrap_err();

        assert_eq!(error.code, SourceTargetErrorCode::MetadataAddressNotFound);
        cleanup(&context);
    }

    #[test]
    fn platform_xml_object_target_rejects_a_multi_artifact_source_set_owner() {
        let context = fixture(
            "object-target-multi-artifact-owner",
            project_yaml("main", "CONFIGURATION", "src"),
        );
        let root = context.workspace_root.join("src");
        write_metadata_descriptor(&root, "Catalogs", "Catalog", "Items", "Items");
        fs::write(
            root.join("Configuration.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Bogus/><Configuration><ChildObjects><Catalog>Items</Catalog></ChildObjects></Configuration></MetaDataObject>"#,
        )
        .unwrap();
        let content_read = std::rc::Rc::new(std::cell::Cell::new(false));
        let observer = std::rc::Rc::clone(&content_read);
        set_object_descriptor_content_read_hook_for_test(move || observer.set(true));

        let error = resolve_platform_xml_object_target(&context, &target("main", "Catalog.Items"))
            .unwrap_err();

        assert_eq!(error.code, SourceTargetErrorCode::MetadataAddressNotFound);
        assert_eq!(error.reason(), SourceTargetErrorReason::General);
        assert_eq!(
            error.message,
            "metadata owner evidence is unavailable for `Catalog.Items` in sourceSet `main`"
        );
        assert!(
            !content_read.get(),
            "target content read preceded owner proof"
        );
        assert!(!error.message.contains(root.to_string_lossy().as_ref()));
        super::OBJECT_DESCRIPTOR_CONTENT_READ_HOOK.with(|slot| {
            slot.borrow_mut().take();
        });
        cleanup(&context);
    }

    #[test]
    fn platform_xml_object_target_rejects_an_owner_version_mismatching_the_target() {
        let context = fixture(
            "object-target-owner-version-mismatch",
            project_yaml("main", "CONFIGURATION", "src"),
        );
        let root = context.workspace_root.join("src");
        write_metadata_descriptor(&root, "Catalogs", "Catalog", "Items", "Items");
        fs::write(
            root.join("Configuration.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.19"><Configuration><ChildObjects><Catalog>Items</Catalog></ChildObjects></Configuration></MetaDataObject>"#,
        )
        .unwrap();
        let content_read = std::rc::Rc::new(std::cell::Cell::new(false));
        let observer = std::rc::Rc::clone(&content_read);
        set_object_descriptor_content_read_hook_for_test(move || observer.set(true));

        let error = resolve_platform_xml_object_target(&context, &target("main", "Catalog.Items"))
            .unwrap_err();

        assert_eq!(error.code, SourceTargetErrorCode::SourceRootNotAddressable);
        assert_eq!(
            error.reason(),
            SourceTargetErrorReason::SourceFormatUnsupported
        );
        assert_eq!(
            error.message,
            "metadataPath `Catalog.Items` format does not match its proven metadata owner in sourceSet `main`"
        );
        assert!(
            content_read.get(),
            "owner/target compatibility must use bounded target evidence"
        );
        assert!(!error.message.contains(root.to_string_lossy().as_ref()));
        super::OBJECT_DESCRIPTOR_CONTENT_READ_HOOK.with(|slot| {
            slot.borrow_mut().take();
        });
        cleanup(&context);
    }

    #[test]
    fn platform_xml_object_target_accepts_a_consistent_older_owner_and_target_for_format_guard() {
        let context = fixture(
            "object-target-consistent-older-format",
            project_yaml("main", "CONFIGURATION", "src"),
        );
        let root = context.workspace_root.join("src");
        fs::create_dir_all(root.join("Catalogs")).unwrap();
        fs::write(
            root.join("Configuration.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.19"><Configuration><ChildObjects><Catalog>Items</Catalog></ChildObjects></Configuration></MetaDataObject>"#,
        )
        .unwrap();
        fs::write(
            root.join("Catalogs/Items.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.19"><Catalog><Properties><Name>Items</Name></Properties></Catalog></MetaDataObject>"#,
        )
        .unwrap();

        let resolution =
            resolve_platform_xml_object_target(&context, &target("main", "Catalog.Items")).unwrap();

        assert_eq!(
            resolution.handle.target_path,
            normalize_path_identity(&root.join("Catalogs/Items.xml")).unwrap()
        );
        cleanup(&context);
    }

    #[test]
    fn platform_xml_object_target_accepts_consistent_versionless_evidence_for_format_guard() {
        let context = fixture(
            "object-target-consistent-versionless",
            project_yaml("main", "CONFIGURATION", "src"),
        );
        let root = context.workspace_root.join("src");
        fs::create_dir_all(root.join("Catalogs")).unwrap();
        fs::write(
            root.join("Configuration.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses"><Configuration><ChildObjects><Catalog>Items</Catalog></ChildObjects></Configuration></MetaDataObject>"#,
        )
        .unwrap();
        fs::write(
            root.join("Catalogs/Items.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses"><Catalog><Properties><Name>Items</Name></Properties></Catalog></MetaDataObject>"#,
        )
        .unwrap();

        let resolution =
            resolve_platform_xml_object_target(&context, &target("main", "Catalog.Items")).unwrap();

        assert_eq!(
            resolution.handle.target_path,
            normalize_path_identity(&root.join("Catalogs/Items.xml")).unwrap()
        );
        cleanup(&context);
    }

    #[test]
    fn platform_xml_object_target_rejects_xml_decoded_version_equality() {
        let context = fixture(
            "object-target-raw-version-mismatch",
            project_yaml("main", "CONFIGURATION", "src"),
        );
        let root = context.workspace_root.join("src");
        fs::create_dir_all(root.join("Catalogs")).unwrap();
        fs::write(
            root.join("Configuration.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Configuration><ChildObjects><Catalog>Items</Catalog></ChildObjects></Configuration></MetaDataObject>"#,
        )
        .unwrap();
        fs::write(
            root.join("Catalogs/Items.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.&#50;0"><Catalog><Properties><Name>Items</Name></Properties></Catalog></MetaDataObject>"#,
        )
        .unwrap();

        let error = resolve_platform_xml_object_target(&context, &target("main", "Catalog.Items"))
            .unwrap_err();

        assert_eq!(error.code, SourceTargetErrorCode::SourceRootNotAddressable);
        assert_eq!(
            error.reason(),
            SourceTargetErrorReason::SourceFormatUnsupported
        );
        assert_eq!(
            error.message,
            "metadataPath `Catalog.Items` format does not match its proven metadata owner in sourceSet `main`"
        );
        assert!(!error.message.contains(root.to_string_lossy().as_ref()));
        cleanup(&context);
    }

    #[test]
    fn platform_xml_object_target_accepts_identical_entity_spelled_raw_versions_for_format_guard() {
        let context = fixture(
            "object-target-identical-entity-version",
            project_yaml("main", "CONFIGURATION", "src"),
        );
        let root = context.workspace_root.join("src");
        fs::create_dir_all(root.join("Catalogs")).unwrap();
        fs::write(
            root.join("Configuration.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.&#50;0"><Configuration><ChildObjects><Catalog>Items</Catalog></ChildObjects></Configuration></MetaDataObject>"#,
        )
        .unwrap();
        fs::write(
            root.join("Catalogs/Items.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.&#50;0"><Catalog><Properties><Name>Items</Name></Properties></Catalog></MetaDataObject>"#,
        )
        .unwrap();

        let resolution =
            resolve_platform_xml_object_target(&context, &target("main", "Catalog.Items")).unwrap();

        assert_eq!(
            resolution.handle.target_path,
            normalize_path_identity(&root.join("Catalogs/Items.xml")).unwrap()
        );
        cleanup(&context);
    }

    #[test]
    fn platform_xml_nested_object_target_accepts_consistent_versionless_evidence() {
        let context = fixture(
            "nested-object-target-consistent-versionless",
            project_yaml("main", "CONFIGURATION", "src"),
        );
        let root = context.workspace_root.join("src");
        fs::create_dir_all(root.join("Catalogs/Items/Forms")).unwrap();
        fs::write(
            root.join("Configuration.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses"><Configuration><ChildObjects><Catalog>Items</Catalog></ChildObjects></Configuration></MetaDataObject>"#,
        )
        .unwrap();
        fs::write(
            root.join("Catalogs/Items.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses"><Catalog><Properties><Name>Items</Name></Properties><ChildObjects><Form>List</Form></ChildObjects></Catalog></MetaDataObject>"#,
        )
        .unwrap();
        fs::write(
            root.join("Catalogs/Items/Forms/List.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses"><Form><Properties><Name>List</Name></Properties></Form></MetaDataObject>"#,
        )
        .unwrap();

        let resolution = resolve_platform_xml_object_target(
            &context,
            &target("main", "Catalog.Items.Form.List"),
        )
        .unwrap();

        assert_eq!(
            resolution.handle.target_path,
            normalize_path_identity(&root.join("Catalogs/Items/Forms/List.xml")).unwrap()
        );
        cleanup(&context);
    }

    #[test]
    fn platform_xml_nested_object_target_accepts_identical_entity_spelled_raw_versions() {
        let context = fixture(
            "nested-object-target-identical-entity-version",
            project_yaml("main", "CONFIGURATION", "src"),
        );
        let root = context.workspace_root.join("src");
        fs::create_dir_all(root.join("Catalogs/Items/Forms")).unwrap();
        fs::write(
            root.join("Configuration.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.&#50;0"><Configuration><ChildObjects><Catalog>Items</Catalog></ChildObjects></Configuration></MetaDataObject>"#,
        )
        .unwrap();
        fs::write(
            root.join("Catalogs/Items.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.&#50;0"><Catalog><Properties><Name>Items</Name></Properties><ChildObjects><Form>List</Form></ChildObjects></Catalog></MetaDataObject>"#,
        )
        .unwrap();
        fs::write(
            root.join("Catalogs/Items/Forms/List.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.&#50;0"><Form><Properties><Name>List</Name></Properties></Form></MetaDataObject>"#,
        )
        .unwrap();

        let resolution = resolve_platform_xml_object_target(
            &context,
            &target("main", "Catalog.Items.Form.List"),
        )
        .unwrap();

        assert_eq!(
            resolution.handle.target_path,
            normalize_path_identity(&root.join("Catalogs/Items/Forms/List.xml")).unwrap()
        );
        cleanup(&context);
    }

    #[test]
    fn platform_xml_object_target_refuses_a_linked_descriptor() {
        let context = fixture(
            "object-target-symlink",
            project_yaml("main", "CONFIGURATION", "src"),
        );
        let root = context.workspace_root.join("src");
        write_metadata_descriptor(&root, "Catalogs", "Catalog", "Items", "Items");
        let descriptor = root.join("Catalogs/Items.xml");
        let real = root.join("Catalogs/Replacement.xml");
        fs::rename(&descriptor, &real).unwrap();
        let outcome = create_file_link_fixture_for_test(&real, &descriptor)
            .expect("unexpected file-link creation error must fail the fixture test");
        if outcome != FileLinkFixtureOutcome::Created {
            cleanup(&context);
            return;
        }

        let error = resolve_platform_xml_object_target(&context, &target("main", "Catalog.Items"))
            .unwrap_err();

        assert_eq!(error.code, SourceTargetErrorCode::ContainmentDenied);
        cleanup(&context);
    }

    #[test]
    fn platform_xml_object_evidence_lists_only_proven_modules() {
        let context = fixture(
            "object-evidence",
            project_yaml("main", "CONFIGURATION", "src"),
        );
        let root = context.workspace_root.join("src");
        write_metadata_descriptor(&root, "Catalogs", "Catalog", "Items", "Items");
        for module in [
            "Catalogs/Items/Ext/ObjectModule.bsl",
            "Catalogs/Items/Ext/ManagerModule.bsl",
        ] {
            let path = root.join(module);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(&path, "Procedure Run()\nEndProcedure\n").unwrap();
        }

        let resolution =
            resolve_platform_xml_object_target(&context, &target("main", "Catalog.Items")).unwrap();
        let evidence = super::platform_xml_resource_evidence(&context, &resolution.handle).unwrap();

        assert_eq!(
            evidence.target_path,
            normalize_path_identity(&root.join("Catalogs/Items.xml")).unwrap()
        );
        assert!(evidence.descriptor_paths.is_empty());
        assert_eq!(
            evidence
                .module_paths
                .iter()
                .map(|path| path.file_name().unwrap().to_string_lossy().to_string())
                .collect::<Vec<_>>(),
            vec![
                "ObjectModule.bsl".to_string(),
                "ManagerModule.bsl".to_string()
            ]
        );
        cleanup(&context);
    }

    #[test]
    fn metadata_object_search_filters_exclude_sibling_objects() {
        let context = fixture(
            "object-search-filters",
            project_yaml("main", "CONFIGURATION", "src"),
        );
        let root = context.workspace_root.join("src");
        write_metadata_descriptor(&root, "Catalogs", "Catalog", "Items", "Items");
        let resolution =
            resolve_platform_xml_object_target(&context, &target("main", "Catalog.Items")).unwrap();

        assert_eq!(
            resolution.handle.search_filters().unwrap(),
            vec![
                RelativeSearchFilter::Exact(PathBuf::from("Catalogs/Items.xml")),
                RelativeSearchFilter::Subtree(PathBuf::from("Catalogs/Items")),
            ]
        );
        cleanup(&context);
    }

    #[test]
    fn search_filters_reject_a_target_outside_its_source_root() {
        let context = fixture(
            "outside-root-search-filters",
            project_yaml("main", "CONFIGURATION", "src"),
        );
        let root = context.workspace_root.join("src");
        write_metadata_descriptor(&root, "Catalogs", "Catalog", "Items", "Items");
        let resolution =
            resolve_platform_xml_object_target(&context, &target("main", "Catalog.Items")).unwrap();
        let mut inconsistent_handle = resolution.handle;
        inconsistent_handle.source_root = context.workspace_root.join("another-source-root");

        let error = inconsistent_handle
            .search_filters()
            .expect_err("an uncontained target must not become an empty all-source filter");

        assert_eq!(error.code, SourceTargetErrorCode::ContainmentDenied);
        cleanup(&context);
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
        register_fixture_item(&root.join("Configuration.xml"), "Configuration", kind, name);
    }

    fn register_fixture_item(owner: &Path, owner_kind: &str, kind: &str, name: &str) {
        let registration = format!("<{kind}>{name}</{kind}>");
        let mut image = fs::read_to_string(owner).unwrap_or_else(|_| {
            format!(
                r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><{owner_kind}><ChildObjects></ChildObjects></{owner_kind}></MetaDataObject>"#
            )
        });
        if !image.contains(&registration) {
            if let Some(insertion) = image.rfind("</ChildObjects>") {
                image.insert_str(insertion, &registration);
            } else if let Some(self_closing) = image.find("<ChildObjects/>") {
                image.replace_range(
                    self_closing..self_closing + "<ChildObjects/>".len(),
                    &format!("<ChildObjects>{registration}</ChildObjects>"),
                );
            } else {
                let closing = format!("</{owner_kind}>");
                let insertion = image
                    .rfind(&closing)
                    .expect("fixture owner has its declared closing element");
                image.insert_str(
                    insertion,
                    &format!("<ChildObjects>{registration}</ChildObjects>"),
                );
            }
            fs::write(owner, image).unwrap();
        }
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
        fs::write(&descriptor, descriptor_xml).unwrap();
        let descriptor_relative = descriptor
            .strip_prefix(root)
            .expect("fixture descriptor is below its source root")
            .components()
            .filter_map(|component| component.as_os_str().to_str())
            .collect::<Vec<_>>();
        if let [owner_directory, owner_name, child_collection, child_file] =
            descriptor_relative.as_slice()
        {
            let child_kind = match *child_collection {
                "Forms" => Some("Form"),
                "Commands" => Some("Command"),
                _ => None,
            };
            if let Some(child_kind) = child_kind {
                let owner = root.join(owner_directory).join(format!("{owner_name}.xml"));
                if owner.is_file() {
                    let owner_kind = super::metadata_kind_by_directory(owner_directory)
                        .expect("fixture owner directory is registered")
                        .tag;
                    register_fixture_item(
                        &owner,
                        owner_kind,
                        child_kind,
                        child_file.trim_end_matches(".xml"),
                    );
                }
            }
        }
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
