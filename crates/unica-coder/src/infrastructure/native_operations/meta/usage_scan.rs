//! Who uses a metadata object, read from the source tree itself.
//!
//! Roles, event subscriptions and functional options are ordinary Platform XML
//! in the configuration, and predefined items belong to the object's own
//! directory. None of them needs a code index, so none of them can be stale or
//! unavailable relative to the rest of a `meta.info` answer: they are read from
//! the same tree in the same call. Only "which modules mention this object"
//! genuinely requires an index, and that stays with the index provider.
//!
//! Measured on an 8.3.27 vendor-class dump (2172 role right-sets over 177 MB,
//! 557 functional options, 305 subscriptions, 432 defined types): the three
//! usage scans together cost about 0.38 s, and the largest section any single
//! object collects is a few dozen entries.

use crate::domain::cancellation::CancellationToken;
use crate::domain::metadata::{
    MetaDiagnostic, MetaDiagnosticCode, MetaPredefinedItemsData, MetaUsageData, MetadataKind,
};
use crate::domain::source_target::MetadataAddress;
use crate::infrastructure::metadata_kinds::{metadata_kind_value_types, metadata_layout};
use crate::infrastructure::platform::secure_read::read_root_relative_regular_file;
use roxmltree::Document;
use serde_json::{json, Value};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// Largest single descriptor the usage scan will read.
const USAGE_FILE_MAX_BYTES: usize = 8 * 1024 * 1024;

/// Largest number of descriptors one section will open.
///
/// The dump this was measured against holds 2172 role right-sets, so the bound
/// is set well above a real configuration and exists to stop a pathological
/// tree, not to shape ordinary answers. A section that hits it reports what it
/// found rather than failing: an incomplete usage list is still useful, and the
/// caller can see the count.
const USAGE_SECTION_MAX_FILES: usize = 8192;

/// What a local read adds to a metadata answer.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct LocalEnrichment {
    pub(crate) usage: MetaUsageData,
    pub(crate) predefined_items: Option<MetaPredefinedItemsData>,
    pub(crate) diagnostics: Vec<MetaDiagnostic>,
}

/// Sections a local read can serve, named as the public contract names them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LocalSection {
    Roles,
    Subscriptions,
    FunctionalOptions,
    PredefinedItems,
}

pub(crate) fn scan_local_enrichment(
    source_root: &Path,
    kind: MetadataKind,
    metadata_path: &MetadataAddress,
    sections: &[LocalSection],
    limit: usize,
    cancellation: &CancellationToken,
) -> LocalEnrichment {
    let target = metadata_path.as_str();
    let name = metadata_path
        .segments()
        .last()
        .expect("a parsed metadata address has an object name");
    let mut enrichment = LocalEnrichment::default();
    if sections.contains(&LocalSection::Roles) && !cancellation.is_cancelled() {
        enrichment.usage.roles = Some(scan_roles(source_root, target));
    }
    if sections.contains(&LocalSection::FunctionalOptions) && !cancellation.is_cancelled() {
        enrichment.usage.functional_options = Some(scan_functional_options(source_root, target));
    }
    if sections.contains(&LocalSection::Subscriptions) && !cancellation.is_cancelled() {
        enrichment.usage.subscriptions = Some(scan_subscriptions(source_root, kind, name));
    }
    if sections.contains(&LocalSection::PredefinedItems) && !cancellation.is_cancelled() {
        if !crate::domain::metadata::metadata_kind_collections(kind)
            .contains(&crate::domain::metadata::MetaCollection::PredefinedItems)
        {
            enrichment.diagnostics.push(
                MetaDiagnostic::error(
                    MetaDiagnosticCode::UnsupportedKind,
                    format!(
                        "predefinedItems is not supported for metadata kind `{}`",
                        kind.as_str()
                    ),
                )
                .with_metadata_path(metadata_path.clone())
                .with_field("predefinedItems"),
            );
        } else {
            match read_predefined_items(source_root, kind, name, metadata_path, limit) {
                Ok(items) => enrichment.predefined_items = Some(items),
                Err(diagnostic) => enrichment.diagnostics.push(diagnostic),
            }
        }
    }
    enrichment
}

/// Roles that grant any right on the object.
///
/// A right set names its subject as `Kind.Name`, and rights on an attribute or
/// tabular section extend that with a suffix, so an object is referenced by the
/// exact name or by anything beneath it.
fn scan_roles(source_root: &Path, target: &str) -> Vec<Value> {
    let mut found = BTreeSet::new();
    for (role, path) in child_directories(source_root, "Roles") {
        let rights = path.join("Ext").join("Rights.xml");
        let Some(text) = read_descriptor(source_root, &rights) else {
            continue;
        };
        let Ok(document) = Document::parse(text.trim_start_matches('\u{feff}')) else {
            continue;
        };
        let referenced = document
            .descendants()
            .filter(|node| node.is_element() && node.tag_name().name() == "name")
            .filter_map(|node| node.text())
            .any(|value| names_target(value.trim(), target));
        if referenced {
            found.insert(role);
        }
    }
    found
        .into_iter()
        .map(|role| json!({"name": format!("Role.{role}")}))
        .collect()
}

/// Functional options whose content includes the object.
///
/// `Location` names where the option's own value is stored and says nothing
/// about what the option controls; `Content` is the list of objects and fields
/// it switches. Reading the former instead of the latter answers a different
/// question while looking correct.
fn scan_functional_options(source_root: &Path, target: &str) -> Vec<Value> {
    let mut found = BTreeSet::new();
    for (option, path) in child_descriptors(source_root, "FunctionalOptions") {
        let Some(text) = read_descriptor(source_root, &path) else {
            continue;
        };
        let Ok(document) = Document::parse(text.trim_start_matches('\u{feff}')) else {
            continue;
        };
        let controls = document
            .descendants()
            .filter(|node| node.is_element() && node.tag_name().name() == "Content")
            .flat_map(|content| content.descendants())
            .filter(|node| node.is_element() && node.tag_name().name() == "Object")
            .filter_map(|node| node.text())
            .any(|value| names_target(value.trim(), target));
        if controls {
            found.insert(option);
        }
    }
    found
        .into_iter()
        .map(|option| json!({"name": format!("FunctionalOption.{option}")}))
        .collect()
}

/// Event subscriptions whose source includes the object.
///
/// A source names types, not metadata paths, and it may name them indirectly
/// through a defined type. On the measured dump 80 of 305 subscriptions reach
/// their source that way, so a reader that matches only direct spellings loses
/// about a quarter of the answer without reporting a gap.
fn scan_subscriptions(source_root: &Path, kind: MetadataKind, name: &str) -> Vec<Value> {
    let spellings = metadata_kind_value_types(kind);
    if spellings.is_empty() {
        return Vec::new();
    }
    let direct = spellings
        .iter()
        .map(|spelling| format!("{spelling}.{name}"))
        .collect::<BTreeSet<_>>();
    let indirect = defined_types_including(source_root, &direct);

    let mut found = Vec::new();
    for (subscription, path) in child_descriptors(source_root, "EventSubscriptions") {
        let Some(text) = read_descriptor(source_root, &path) else {
            continue;
        };
        let Ok(document) = Document::parse(text.trim_start_matches('\u{feff}')) else {
            continue;
        };
        let Some(source) = document
            .descendants()
            .find(|node| node.is_element() && node.tag_name().name() == "Source")
        else {
            continue;
        };
        let mut via = None;
        let mut matched = false;
        for value in source
            .descendants()
            .filter(|node| node.is_element())
            .filter(|node| matches!(node.tag_name().name(), "Type" | "TypeSet"))
            .filter_map(|node| node.text())
            .map(|value| strip_configuration_prefix(value.trim()))
        {
            if direct.contains(value) {
                matched = true;
                via = None;
                break;
            }
            if let Some(defined) = value.strip_prefix("DefinedType.") {
                if indirect.contains(defined) {
                    matched = true;
                    via = Some(format!("DefinedType.{defined}"));
                }
            }
        }
        if !matched {
            continue;
        }
        let mut item = json!({"name": format!("EventSubscription.{subscription}")});
        let object = item
            .as_object_mut()
            .expect("subscription item is always an object");
        if let Some(event) = child_text(&document, "Event") {
            object.insert("event".into(), Value::String(event));
        }
        if let Some(handler) = child_text(&document, "Handler") {
            object.insert("handler".into(), Value::String(handler));
        }
        // An indirect match is reported, not hidden: the subscription does not
        // name this object, and a reader deciding whether an edit is safe needs
        // to know the link runs through a defined type.
        if let Some(via) = via {
            object.insert("via".into(), Value::String(via));
        }
        found.push(item);
    }
    found.sort_by(|left, right| left["name"].as_str().cmp(&right["name"].as_str()));
    found
}

/// Defined types whose own type list contains one of the object's spellings.
fn defined_types_including(source_root: &Path, direct: &BTreeSet<String>) -> BTreeSet<String> {
    let mut including = BTreeSet::new();
    for (defined, path) in child_descriptors(source_root, "DefinedTypes") {
        let Some(text) = read_descriptor(source_root, &path) else {
            continue;
        };
        let Ok(document) = Document::parse(text.trim_start_matches('\u{feff}')) else {
            continue;
        };
        let contains = document
            .descendants()
            .filter(|node| node.is_element() && node.tag_name().name() == "Type")
            .filter_map(|node| node.text())
            .map(|value| strip_configuration_prefix(value.trim()))
            .any(|value| direct.contains(value));
        if contains {
            including.insert(defined);
        }
    }
    including
}

/// Predefined items declared by the object itself.
fn read_predefined_items(
    source_root: &Path,
    kind: MetadataKind,
    name: &str,
    metadata_path: &MetadataAddress,
    limit: usize,
) -> Result<MetaPredefinedItemsData, MetaDiagnostic> {
    let path = source_root
        .join(metadata_layout(kind).directory)
        .join(name)
        .join("Ext")
        .join("Predefined.xml");
    let bytes =
        match read_root_relative_regular_file(source_root, &path, USAGE_FILE_MAX_BYTES, |_| {}) {
            Ok(read) => read.bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(MetaPredefinedItemsData {
                    total: 0,
                    returned: 0,
                    truncated: false,
                    items: Vec::new(),
                })
            }
            Err(_) => {
                return Err(MetaDiagnostic::error(
                    MetaDiagnosticCode::ProviderUnavailable,
                    "predefined data cannot be read through the source-root boundary",
                )
                .with_metadata_path(metadata_path.clone())
                .with_field("predefinedItems"))
            }
        };
    super::predefined::read_predefined_items(&bytes, kind, limit).map_err(|message| {
        MetaDiagnostic::error(MetaDiagnosticCode::ValidationFailed, message)
            .with_metadata_path(metadata_path.clone())
            .with_field("predefinedItems")
    })
}

/// `Kind.Name` itself, or anything nested beneath it.
fn names_target(value: &str, target: &str) -> bool {
    value == target
        || value
            .strip_prefix(target)
            .is_some_and(|rest| rest.starts_with('.'))
}

/// Platform XML writes configuration-local types as `cfg:CatalogRef.Name`.
fn strip_configuration_prefix(value: &str) -> &str {
    value.strip_prefix("cfg:").unwrap_or(value)
}

fn child_text(document: &Document<'_>, tag: &str) -> Option<String> {
    document
        .descendants()
        .find(|node| node.is_element() && node.tag_name().name() == tag)
        .and_then(|node| node.text())
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

/// `<root>/<group>/<Name>.xml` descriptors, by name.
fn child_descriptors(source_root: &Path, group: &str) -> Vec<(String, PathBuf)> {
    let mut entries = Vec::new();
    let Ok(listing) = std::fs::read_dir(source_root.join(group)) else {
        return entries;
    };
    for entry in listing.flatten().take(USAGE_SECTION_MAX_FILES) {
        let path = entry.path();
        if path.extension().and_then(|value| value.to_str()) != Some("xml") {
            continue;
        }
        let Some(name) = path.file_stem().and_then(|value| value.to_str()) else {
            continue;
        };
        entries.push((name.to_string(), path));
    }
    entries
}

/// `<root>/<group>/<Name>/` directories, by name.
fn child_directories(source_root: &Path, group: &str) -> Vec<(String, PathBuf)> {
    let mut entries = Vec::new();
    let Ok(listing) = std::fs::read_dir(source_root.join(group)) else {
        return entries;
    };
    for entry in listing.flatten().take(USAGE_SECTION_MAX_FILES) {
        let path = entry.path();
        if !entry.file_type().is_ok_and(|kind| kind.is_dir()) {
            continue;
        }
        let Some(name) = path.file_name().and_then(|value| value.to_str()) else {
            continue;
        };
        entries.push((name.to_string(), path));
    }
    entries
}

/// Read one descriptor bound to the source root, or skip it.
///
/// A usage scan reports what the configuration says; it does not gate a
/// mutation, so a descriptor that cannot be read is skipped rather than
/// failing the whole answer. The read is still root-relative and bounded, so a
/// link pointing out of the tree contributes nothing instead of being followed.
fn read_descriptor(source_root: &Path, path: &Path) -> Option<String> {
    let read =
        read_root_relative_regular_file(source_root, path, USAGE_FILE_MAX_BYTES, |_| {}).ok()?;
    String::from_utf8(read.bytes).ok()
}
