use crate::application::v13::find::{
    normalize_layout_path, FindDocument, FindFact, FindFactKind, FindIndex, FindPathAlias,
};
use crate::domain::address::{NodeKind, QualifiedAddress};
use crate::domain::cancellation::CancellationToken;
use crate::domain::code_intelligence::ProviderDeadline;
use crate::domain::project_sources::SourceSetKind;
use crate::domain::refusal::{RefusalCode, RefusalDetail};
use crate::infrastructure::capacity_observation::CapacityObserver;
use crate::infrastructure::metadata_kinds::metadata_kind_by_directory;
use crate::infrastructure::platform::filesystem::{
    FileIdentity, RetainedChildCapability, RetainedDirectoryCapability,
    RetainedRegularFileCapability,
};
use std::collections::{HashMap, HashSet};
use std::ffi::OsStr;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;

const MAX_SOURCE_SETS: usize = 64;
const DEFAULT_MAX_DOCUMENTS: usize = 65_536;
const DEFAULT_MAX_FACT_BYTES: usize = 16 * 1024 * 1024;
/// Enough of a descriptor head to carry `Name` and `Synonym`. The directory
/// never needs the rest of the file.
const DESCRIPTOR_HEAD_BYTES: usize = 8 * 1024;
const MAX_COLLECTION_ENTRIES: usize = 65_536;
const MAX_OMISSION_DETAILS: usize = 20;
/// Physical child families an object owns as its own files or directories.
const NESTED_FAMILIES: [(&str, NodeKind); 3] = [
    ("Forms", NodeKind::Form),
    ("Templates", NodeKind::Template),
    ("Commands", NodeKind::Command),
];

/// One admitted source-set root. The directory is read through the retained
/// no-follow capability the actor owns; no path is reopened by name.
pub(crate) struct LayoutFindSource<'a> {
    name: &'a str,
    kind: SourceSetKind,
    root: &'a RetainedDirectoryCapability,
}

impl<'a> LayoutFindSource<'a> {
    pub(crate) const fn new(
        name: &'a str,
        kind: SourceSetKind,
        root: &'a RetainedDirectoryCapability,
    ) -> Self {
        Self { name, kind, root }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FindBuildError {
    code: RefusalCode,
    detail: Option<RefusalDetail>,
    message: String,
}

impl FindBuildError {
    fn new(code: RefusalCode, message: impl Into<String>) -> Self {
        Self {
            code,
            detail: None,
            message: message.into(),
        }
    }

    fn with_detail(detail: RefusalDetail, message: impl Into<String>) -> Self {
        Self {
            code: detail.code(),
            detail: Some(detail),
            message: message.into(),
        }
    }

    pub(crate) const fn code(&self) -> RefusalCode {
        self.code
    }

    pub(crate) const fn detail(&self) -> Option<RefusalDetail> {
        self.detail
    }

    fn is_local_unreadable(&self) -> bool {
        self.detail == Some(RefusalDetail::SourceUnreadable)
    }
}

impl std::fmt::Display for FindBuildError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl From<io::Error> for FindBuildError {
    fn from(_error: io::Error) -> Self {
        Self::with_detail(
            RefusalDetail::SourceUnreadable,
            "resolve could not enumerate the retained source directory",
        )
    }
}

/// Builds the two-way directory between qualified logical addresses and where
/// objects live in the source layout. It reads that layout only: no typed
/// projection, no module source, no revision lease.
pub(crate) struct WorkspaceFindDirectoryBuilder {
    max_documents: usize,
    max_total_fact_bytes: usize,
    max_collection_entries: usize,
    read_head: Arc<HeadReader>,
    capacity_observer: Option<Arc<CapacityObserver>>,
}

type HeadReader = dyn Fn(&RetainedRegularFileCapability, &Path) -> Result<Vec<u8>, DescriptorHeadReadError>
    + Send
    + Sync;

#[derive(Clone, Copy)]
enum DescriptorHeadReadError {
    /// The retained capability could not be cloned. This is not evidence that
    /// just one descriptor is unreadable, so the whole build refuses.
    Capability,
    /// I/O failed while reading this already retained regular file.
    Local(io::ErrorKind),
}

#[derive(Debug)]
pub(crate) struct FindBuildOutcome {
    pub(crate) index: FindIndex,
    pub(crate) omissions: FindOmissions,
}

#[derive(Debug, Default)]
pub(crate) struct FindOmissions {
    pub(crate) total: usize,
    pub(crate) details: Vec<FindOmission>,
}

#[derive(Debug)]
pub(crate) struct FindOmission {
    pub(crate) source_set: String,
    pub(crate) reason: &'static str,
}

impl Default for WorkspaceFindDirectoryBuilder {
    fn default() -> Self {
        Self::with_limits(DEFAULT_MAX_DOCUMENTS, DEFAULT_MAX_FACT_BYTES)
    }
}

struct DirectoryBuild {
    documents: Vec<FindDocument>,
    fact_bytes: usize,
    attempted_entries: usize,
    attempted_fact_bytes: usize,
    omissions: FindOmissions,
    include_module_aliases: bool,
}

impl DirectoryBuild {
    fn record_omission(&mut self, source_set: &str, reason: &'static str) {
        self.omissions.total = self.omissions.total.saturating_add(1);
        if self.omissions.details.len() < MAX_OMISSION_DETAILS {
            self.omissions.details.push(FindOmission {
                source_set: source_set.to_string(),
                reason,
            });
        }
    }
}

impl WorkspaceFindDirectoryBuilder {
    fn with_limits(max_documents: usize, max_total_fact_bytes: usize) -> Self {
        Self {
            max_documents,
            max_total_fact_bytes,
            max_collection_entries: MAX_COLLECTION_ENTRIES,
            read_head: Arc::new(read_descriptor_head_prefix),
            capacity_observer: None,
        }
    }

    pub(crate) fn with_capacity_observer(mut self, observer: Arc<CapacityObserver>) -> Self {
        self.capacity_observer = Some(observer);
        self
    }

    #[cfg(test)]
    fn with_head_reader(mut self, reader: Arc<HeadReader>) -> Self {
        self.read_head = reader;
        self
    }

    #[cfg(test)]
    pub(crate) fn with_local_read_fault_for_test(
        self,
        relative: &'static str,
        kind: io::ErrorKind,
    ) -> Self {
        self.with_head_reader(Arc::new(move |file, path| {
            if path == Path::new(relative) {
                Err(DescriptorHeadReadError::Local(kind))
            } else {
                read_descriptor_head_prefix(file, path)
            }
        }))
    }

    #[cfg(test)]
    pub(crate) fn with_document_limit(max_documents: usize) -> Self {
        Self::with_limits(max_documents, DEFAULT_MAX_FACT_BYTES)
    }

    #[cfg(test)]
    pub(crate) fn with_fact_byte_limit_for_test(max_fact_bytes: usize) -> Self {
        Self::with_limits(DEFAULT_MAX_DOCUMENTS, max_fact_bytes)
    }

    #[cfg(test)]
    fn with_collection_limit_for_test(mut self, max_collection_entries: usize) -> Self {
        self.max_collection_entries = max_collection_entries;
        self
    }

    pub(crate) fn build(
        &self,
        sources: &[LayoutFindSource<'_>],
        deadline: ProviderDeadline,
        cancellation: &CancellationToken,
    ) -> Result<FindIndex, FindBuildError> {
        self.build_exact(sources, deadline, cancellation, false)
    }

    pub(crate) fn build_for_path(
        &self,
        sources: &[LayoutFindSource<'_>],
        deadline: ProviderDeadline,
        cancellation: &CancellationToken,
    ) -> Result<FindIndex, FindBuildError> {
        self.build_exact(sources, deadline, cancellation, true)
    }

    fn build_exact(
        &self,
        sources: &[LayoutFindSource<'_>],
        deadline: ProviderDeadline,
        cancellation: &CancellationToken,
        include_module_aliases: bool,
    ) -> Result<FindIndex, FindBuildError> {
        let outcome =
            self.build_internal(sources, deadline, cancellation, include_module_aliases)?;
        if outcome.omissions.total != 0 {
            return Err(FindBuildError::with_detail(
                RefusalDetail::SourceUnreadable,
                format!(
                    "resolve cannot prove a complete source layout: {} descriptor reads failed",
                    outcome.omissions.total
                ),
            ));
        }
        Ok(outcome.index)
    }

    pub(crate) fn build_for_search(
        &self,
        sources: &[LayoutFindSource<'_>],
        deadline: ProviderDeadline,
        cancellation: &CancellationToken,
    ) -> Result<FindBuildOutcome, FindBuildError> {
        self.build_internal(sources, deadline, cancellation, false)
    }

    fn build_internal(
        &self,
        sources: &[LayoutFindSource<'_>],
        deadline: ProviderDeadline,
        cancellation: &CancellationToken,
        include_module_aliases: bool,
    ) -> Result<FindBuildOutcome, FindBuildError> {
        if sources.len() > MAX_SOURCE_SETS {
            return Err(FindBuildError::new(
                RefusalCode::ProviderLimitExceeded,
                "find source-set count exceeds the bounded workspace limit",
            ));
        }
        let mut build = DirectoryBuild {
            documents: Vec::new(),
            fact_bytes: 0,
            attempted_entries: 0,
            attempted_fact_bytes: 0,
            omissions: FindOmissions::default(),
            include_module_aliases,
        };
        let result = (|| {
            for source in sources {
                find_checkpoint(deadline, cancellation)?;
                self.add_source(source, &mut build, deadline, cancellation)?;
            }
            Ok::<(), FindBuildError>(())
        })();
        if let Some(observer) = &self.capacity_observer {
            let capacity_refusal = result
                .as_ref()
                .err()
                .is_some_and(|error| error.code() == RefusalCode::ProviderLimitExceeded);
            if result.is_ok() || capacity_refusal {
                observer.record_find(
                    build.attempted_fact_bytes as u64,
                    build.attempted_entries as u64,
                    result.is_ok() && build.omissions.total == 0,
                );
            }
        }
        result?;
        Ok(FindBuildOutcome {
            index: FindIndex::new(build.documents),
            omissions: build.omissions,
        })
    }

    /// Resolve only paths that could be the requested object. The name-search
    /// directory still owns its aggregate fact budget; a point lookup must not
    /// read unrelated descriptors or charge their facts before answering.
    pub(crate) fn locate_path(
        &self,
        sources: &[LayoutFindSource<'_>],
        path: &str,
        deadline: ProviderDeadline,
        cancellation: &CancellationToken,
    ) -> Result<Option<FindDocument>, FindBuildError> {
        let normalized = normalize_layout_path(path);
        let parts = normalized.split('/').collect::<Vec<_>>();
        let absolute_query = layout_path_is_absolute(path, &normalized);
        // Bind a path carrying a retained source-root prefix once for the
        // whole lookup. Recomputing the owner at each shorter suffix would
        // let an unrelated source supply a fallback after the target misses.
        let mut absolute_witnesses = Vec::with_capacity(sources.len());
        let mut source_matches = Vec::with_capacity(sources.len());
        if absolute_query {
            let root_matches = sources
                .iter()
                .map(|source| absolute_query_relative_path(source, path, deadline, cancellation))
                .collect::<Result<Vec<_>, _>>()?;
            let deepest = sources
                .iter()
                .zip(&root_matches)
                .filter(|(_, relative)| relative.is_some())
                .map(|(source, _)| source.root.path().components().count())
                .max();
            for (source, relative) in sources.iter().zip(root_matches) {
                let selected =
                    relative.is_some() && deepest == Some(source.root.path().components().count());
                source_matches.push(selected);
                absolute_witnesses.push(if selected {
                    retain_absolute_target(
                        source,
                        relative.as_deref().ok_or_else(unsafe_layout_entry)?,
                        deadline,
                        cancellation,
                    )?
                } else {
                    None
                });
            }
        } else {
            for source in sources {
                source_matches.push((1..=parts.len().min(4)).any(|depth| {
                    let tail = parts[parts.len() - depth..].join("/");
                    normalized != tail && source_path_ends_with_query(source, &normalized, &tail)
                }));
                absolute_witnesses.push(None);
            }
        }
        let source_is_bound = source_matches.iter().any(|matched| *matched);
        if absolute_query && !source_is_bound {
            find_checkpoint(deadline, cancellation)?;
            return Ok(None);
        }
        // The full index chooses the longest stored path. Probe equal-length
        // candidates in every source set before trying shorter fallbacks, so
        // a damaged, unrelated fallback cannot block an exact target.
        for depth in [4, 3, 2, 1] {
            let tail = parts
                .len()
                .checked_sub(depth)
                .map(|start| parts[start..].join("/"));
            let mut located = None;
            let mut ambiguous_alias = false;
            for ((source, source_matches), witness) in
                sources.iter().zip(&source_matches).zip(&absolute_witnesses)
            {
                find_checkpoint(deadline, cancellation)?;
                if source_is_bound && !source_matches {
                    continue;
                }
                let Some(tail) = tail.as_deref() else {
                    continue;
                };
                if absolute_query
                    && witness
                        .as_ref()
                        .is_none_or(|witness| witness.relative.components().count() != depth)
                {
                    continue;
                }
                // A relative path with a source prefix names this exact
                // layout entry. An absolute path already has a retained
                // physical witness; string spelling may differ by Unicode
                // normalization on the same filesystem object.
                if !absolute_query && !source_path_matches_query(source, &normalized, tail) {
                    continue;
                }
                source
                    .root
                    .validate_named_identity()
                    .map_err(|_| unsafe_layout_entry())?;
                self.locate_path_in_source(
                    source,
                    path,
                    &parts,
                    witness.as_ref().map(|witness| witness.relative.as_path()),
                    depth,
                    deadline,
                    cancellation,
                    &mut located,
                    &mut ambiguous_alias,
                )?;
                source
                    .root
                    .validate_named_identity()
                    .map_err(|_| unsafe_layout_entry())?;
            }
            // A shared relative alias names two real owners; report the
            // ambiguity instead of claiming that the module is absent.
            if ambiguous_alias {
                return Err(FindBuildError::new(
                    RefusalCode::BadValue,
                    "resolve path matches multiple equally specific objects; use their logical address",
                ));
            }
            if located.is_some() {
                // The exact physical target must still have the identity that
                // selected this source before its descriptor was inspected.
                if absolute_query {
                    for ((source, matched), witness) in
                        sources.iter().zip(&source_matches).zip(&absolute_witnesses)
                    {
                        if *matched
                            && absolute_query_witness(source, path, deadline, cancellation)?
                                != *witness
                        {
                            return Err(FindBuildError::new(
                                RefusalCode::ConcurrentChange,
                                "resolve target changed during path lookup",
                            ));
                        }
                    }
                    let found = located.as_ref().ok_or_else(unsafe_layout_entry)?;
                    let address =
                        QualifiedAddress::parse(found.at()).map_err(|_| unsafe_layout_entry())?;
                    let Some((source, witness)) = sources
                        .iter()
                        .zip(&absolute_witnesses)
                        .find(|(source, _)| source.name == address.source_set())
                    else {
                        return Err(unsafe_layout_entry());
                    };
                    let Some(witness) = witness else {
                        return Err(unsafe_layout_entry());
                    };
                    let Some(placed_path) = found.placed_path() else {
                        return Err(unsafe_layout_entry());
                    };
                    if !absolute_query_matches_placement(
                        source,
                        witness,
                        placed_path,
                        deadline,
                        cancellation,
                    )? {
                        return Ok(None);
                    }
                }
                find_checkpoint(deadline, cancellation)?;
                return Ok(located);
            }
        }
        find_checkpoint(deadline, cancellation)?;
        Ok(None)
    }

    #[allow(clippy::too_many_arguments)]
    fn locate_path_in_source(
        &self,
        source: &LayoutFindSource<'_>,
        query: &str,
        parts: &[&str],
        absolute_relative: Option<&Path>,
        depth: usize,
        deadline: ProviderDeadline,
        cancellation: &CancellationToken,
        located: &mut Option<FindDocument>,
        ambiguous_alias: &mut bool,
    ) -> Result<(), FindBuildError> {
        let physical_names = absolute_relative.map(|relative| {
            relative
                .components()
                .filter_map(|component| match component {
                    std::path::Component::Normal(name) => Some(name),
                    _ => None,
                })
                .collect::<Vec<_>>()
        });
        let physical_name = |index| {
            physical_names
                .as_ref()
                .and_then(|names| names.get(index).copied())
        };
        if matches!(
            source.kind,
            SourceSetKind::ExternalProcessor | SourceSetKind::ExternalReport
        ) {
            let kind = if source.kind == SourceSetKind::ExternalProcessor {
                NodeKind::ExternalDataProcessor
            } else {
                NodeKind::ExternalReport
            };
            if depth == 1 {
                let Some(&name) = parts.last() else {
                    return Ok(());
                };
                for entry in
                    matching_names(source.root, name, physical_name(0), deadline, cancellation)?
                {
                    let Some(stem) = entry.to_str().and_then(|name| name.strip_suffix(".xml"))
                    else {
                        continue;
                    };
                    if entry
                        .to_str()
                        .is_some_and(|name| name.eq_ignore_ascii_case("ConfigDumpInfo.xml"))
                    {
                        continue;
                    }
                    let relative = PathBuf::from(&entry);
                    let file =
                        match retain_enumerated_child(source.root, &entry, deadline, cancellation)?
                        {
                            RetainedChildCapability::RegularFile(file) => file,
                            _ => return Err(unsafe_layout_entry()),
                        };
                    let (_, synonym) = require_target_identity(self.read_target_identity(
                        &file,
                        &relative,
                        source,
                        kind.as_str(),
                        Some(stem),
                        deadline,
                        cancellation,
                    )?)?;
                    add_path_match(
                        located,
                        find_document(
                            source,
                            &format!("{}:{}.{stem}", source.name, kind.as_str()),
                            kind.as_str(),
                            stem,
                            synonym.as_deref(),
                            &relative,
                        )?,
                    )?;
                }
            }
            if depth == 3 && parts.len() >= 3 {
                self.locate_nested_path(
                    source,
                    None,
                    &parts[parts.len() - 3..],
                    physical_names.as_deref(),
                    kind.as_str(),
                    deadline,
                    cancellation,
                    located,
                )?;
            }
            return Ok(());
        }

        let configuration_spelling_matches = match physical_name(0) {
            Some(name) => source
                .root
                .child_names_equivalent(name, OsStr::new("Configuration.xml"))
                .map_err(child_read_error)?,
            None => true,
        };
        if depth == 1
            && parts.last() == Some(&"configuration.xml")
            && configuration_spelling_matches
        {
            if let Some(child) = retain_optional_child(
                source.root,
                OsStr::new("Configuration.xml"),
                deadline,
                cancellation,
            )? {
                let RetainedChildCapability::RegularFile(file) = child else {
                    return Err(unsafe_layout_entry());
                };
                let relative = Path::new("Configuration.xml");
                let (name, synonym) = require_target_identity(self.read_target_identity(
                    &file,
                    relative,
                    source,
                    NodeKind::Configuration.as_str(),
                    None,
                    deadline,
                    cancellation,
                )?)?;
                add_path_match(
                    located,
                    find_document(
                        source,
                        &format!("{}:Configuration", source.name),
                        NodeKind::Configuration.as_str(),
                        &name,
                        synonym.as_deref(),
                        relative,
                    )?,
                )?;
            }
        }
        if depth == 2 && parts.len() >= 2 {
            let tail = &parts[parts.len() - 2..];
            for directory in matching_names(
                source.root,
                tail[0],
                physical_name(0),
                deadline,
                cancellation,
            )? {
                let Some(directory_name) = directory.to_str() else {
                    continue;
                };
                let Some(layout) = metadata_kind_by_directory(directory_name) else {
                    continue;
                };
                let collection =
                    match retain_enumerated_child(source.root, &directory, deadline, cancellation)?
                    {
                        RetainedChildCapability::Directory(collection) => collection,
                        _ => return Err(unsafe_layout_entry()),
                    };
                for entry in matching_names(
                    &collection,
                    tail[1],
                    physical_name(1),
                    deadline,
                    cancellation,
                )? {
                    let Some(stem) = entry.to_str().and_then(|name| name.strip_suffix(".xml"))
                    else {
                        continue;
                    };
                    let relative = PathBuf::from(directory_name).join(&entry);
                    let file =
                        match retain_enumerated_child(&collection, &entry, deadline, cancellation)?
                        {
                            RetainedChildCapability::RegularFile(file) => file,
                            _ => return Err(unsafe_layout_entry()),
                        };
                    let (_, synonym) = require_target_identity(self.read_target_identity(
                        &file,
                        &relative,
                        source,
                        layout.tag,
                        Some(stem),
                        deadline,
                        cancellation,
                    )?)?;
                    add_path_match(
                        located,
                        find_document(
                            source,
                            &format!("{}:{}.{stem}", source.name, layout.tag),
                            layout.tag,
                            stem,
                            synonym.as_deref(),
                            &relative,
                        )?,
                    )?;
                }
            }
        }
        if depth == 4 && parts.len() >= 4 {
            let tail = &parts[parts.len() - 4..];
            self.locate_common_module_file(
                source,
                query,
                tail,
                physical_names.as_deref(),
                deadline,
                cancellation,
                located,
                ambiguous_alias,
            )?;
            for directory in matching_names(
                source.root,
                tail[0],
                physical_name(0),
                deadline,
                cancellation,
            )? {
                let Some(directory_name) = directory.to_str() else {
                    continue;
                };
                let Some(layout) = metadata_kind_by_directory(directory_name) else {
                    continue;
                };
                let collection =
                    match retain_enumerated_child(source.root, &directory, deadline, cancellation)?
                    {
                        RetainedChildCapability::Directory(collection) => collection,
                        _ => return Err(unsafe_layout_entry()),
                    };
                self.locate_nested_path(
                    source,
                    Some((&collection, directory_name)),
                    &tail[1..],
                    physical_names.as_deref().and_then(|names| names.get(1..)),
                    layout.tag,
                    deadline,
                    cancellation,
                    located,
                )?;
            }
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn locate_common_module_file(
        &self,
        source: &LayoutFindSource<'_>,
        query: &str,
        tail: &[&str],
        physical: Option<&[&OsStr]>,
        deadline: ProviderDeadline,
        cancellation: &CancellationToken,
        located: &mut Option<FindDocument>,
        ambiguous_alias: &mut bool,
    ) -> Result<(), FindBuildError> {
        let [collection_query, owner_query, "ext", "module.bsl"] = tail else {
            return Ok(());
        };
        // The alias denotes a concrete module path. Check its source prefix
        // before even enumerating a foreign CommonModules collection.
        if physical.is_none() && !common_module_query_matches(source, query, &tail.join("/")) {
            return Ok(());
        }
        for collection_name in matching_names(
            source.root,
            collection_query,
            physical.and_then(|names| names.first().copied()),
            deadline,
            cancellation,
        )? {
            let Some(collection_text) = collection_name.to_str() else {
                continue;
            };
            if metadata_kind_by_directory(collection_text).map(|layout| layout.tag)
                != Some("CommonModule")
            {
                continue;
            }
            let collection = match retain_enumerated_child(
                source.root,
                &collection_name,
                deadline,
                cancellation,
            )? {
                RetainedChildCapability::Directory(collection) => collection,
                _ => return Err(unsafe_layout_entry()),
            };
            for owner_name in matching_names(
                &collection,
                owner_query,
                physical.and_then(|names| names.get(1).copied()),
                deadline,
                cancellation,
            )? {
                let Some(owner_text) = owner_name.to_str() else {
                    continue;
                };
                if owner_text.ends_with(".xml") {
                    continue;
                }
                let owner_relative = PathBuf::from(collection_text).join(owner_text);
                let expected_module_relative = path_text(&owner_relative.join("Ext/Module.bsl"));
                if physical.is_none()
                    && !common_module_query_matches(source, query, &expected_module_relative)
                {
                    continue;
                }
                let owner_root = match retain_enumerated_child(
                    &collection,
                    &owner_name,
                    deadline,
                    cancellation,
                )? {
                    RetainedChildCapability::Directory(owner_root) => owner_root,
                    _ => return Err(unsafe_layout_entry()),
                };
                let Some(module_relative) = common_module_path(
                    &owner_root,
                    &owner_relative,
                    physical.and_then(|names| names.get(2).copied()),
                    physical.and_then(|names| names.get(3).copied()),
                    deadline,
                    cancellation,
                )?
                else {
                    continue;
                };
                let owner_descriptor = format!("{owner_text}.xml");
                let Some(owner_file) = retain_optional_child(
                    &collection,
                    OsStr::new(&owner_descriptor),
                    deadline,
                    cancellation,
                )?
                else {
                    continue;
                };
                let RetainedChildCapability::RegularFile(owner_file) = owner_file else {
                    return Err(unsafe_layout_entry());
                };
                let descriptor_relative = PathBuf::from(collection_text).join(&owner_descriptor);
                let (_, synonym) = require_target_identity(self.read_target_identity(
                    &owner_file,
                    &descriptor_relative,
                    source,
                    "CommonModule",
                    Some(owner_text),
                    deadline,
                    cancellation,
                )?)?;
                owner_root
                    .validate_named_identity()
                    .map_err(|_| unsafe_layout_entry())?;
                collection
                    .validate_named_identity()
                    .map_err(|_| unsafe_layout_entry())?;
                let candidate = find_document(
                    source,
                    &format!("{}:CommonModule.{owner_text}", source.name),
                    "CommonModule",
                    owner_text,
                    synonym.as_deref(),
                    &descriptor_relative,
                )?
                .with_path(module_relative);
                if located
                    .as_ref()
                    .is_some_and(|previous| previous.at() != candidate.at())
                {
                    *ambiguous_alias = true;
                } else if !*ambiguous_alias {
                    add_path_match(located, candidate)?;
                }
            }
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn locate_nested_path(
        &self,
        source: &LayoutFindSource<'_>,
        collection: Option<(&RetainedDirectoryCapability, &str)>,
        parts: &[&str],
        physical: Option<&[&OsStr]>,
        owner_kind: &str,
        deadline: ProviderDeadline,
        cancellation: &CancellationToken,
        located: &mut Option<FindDocument>,
    ) -> Result<(), FindBuildError> {
        let [owner_query, family_query, leaf_query] = parts else {
            return Ok(());
        };
        if !NESTED_FAMILIES
            .iter()
            .any(|(family, _)| family.to_lowercase() == *family_query)
        {
            return Ok(());
        }
        let parent = collection.map_or(source.root, |(root, _)| root);
        for owner_name in matching_names(
            parent,
            owner_query,
            physical.and_then(|names| names.first().copied()),
            deadline,
            cancellation,
        )? {
            let Some(owner_name_text) = owner_name.to_str() else {
                continue;
            };
            let owner_root =
                match retain_enumerated_child(parent, &owner_name, deadline, cancellation)? {
                    RetainedChildCapability::Directory(owner_root) => owner_root,
                    RetainedChildCapability::RegularFile(_) => continue,
                    _ => return Err(unsafe_layout_entry()),
                };
            for (family_name, family_kind) in NESTED_FAMILIES {
                if family_name.to_lowercase() != *family_query {
                    continue;
                }
                if let Some(requested_family) = physical.and_then(|names| names.get(1).copied()) {
                    if !owner_root
                        .child_names_equivalent(requested_family, OsStr::new(family_name))
                        .map_err(child_read_error)?
                    {
                        continue;
                    }
                }
                let Some(family) = retain_optional_child(
                    &owner_root,
                    OsStr::new(family_name),
                    deadline,
                    cancellation,
                )?
                else {
                    continue;
                };
                let RetainedChildCapability::Directory(family) = family else {
                    return Err(unsafe_layout_entry());
                };
                for child_name in matching_names(
                    &family,
                    leaf_query,
                    physical.and_then(|names| names.get(2).copied()),
                    deadline,
                    cancellation,
                )? {
                    let Some(child_name_text) = child_name.to_str() else {
                        continue;
                    };
                    let descriptor_name = format!("{owner_name_text}.xml");
                    let Some(owner_descriptor) = retain_optional_child(
                        parent,
                        OsStr::new(&descriptor_name),
                        deadline,
                        cancellation,
                    )?
                    else {
                        return Err(unsafe_layout_entry());
                    };
                    let RetainedChildCapability::RegularFile(owner_file) = owner_descriptor else {
                        return Err(unsafe_layout_entry());
                    };
                    let owner_descriptor_relative = collection.map_or_else(
                        || PathBuf::from(&descriptor_name),
                        |(_, name)| PathBuf::from(name).join(&descriptor_name),
                    );
                    require_target_identity(self.read_target_identity(
                        &owner_file,
                        &owner_descriptor_relative,
                        source,
                        owner_kind,
                        Some(owner_name_text),
                        deadline,
                        cancellation,
                    )?)?;
                    let owner_relative = collection.map_or_else(
                        || PathBuf::from(owner_name_text),
                        |(_, name)| PathBuf::from(name).join(owner_name_text),
                    );
                    let relative = owner_relative.join(family_name).join(&child_name);
                    let child =
                        retain_enumerated_child(&family, &child_name, deadline, cancellation)?;
                    let (nested_name, synonym) = if family_kind == NodeKind::Command {
                        let command = match child {
                            RetainedChildCapability::Directory(command) => command,
                            // The platform has no Commands/<Name>.xml owner file.
                            RetainedChildCapability::RegularFile(_) => continue,
                            _ => return Err(unsafe_layout_entry()),
                        };
                        command
                            .validate_named_identity()
                            .map_err(|_| unsafe_layout_entry())?;
                        (child_name_text, None)
                    } else {
                        let Some(stem) = child_name_text.strip_suffix(".xml") else {
                            continue;
                        };
                        let RetainedChildCapability::RegularFile(file) = child else {
                            return Err(unsafe_layout_entry());
                        };
                        let (_, synonym) = require_target_identity(self.read_target_identity(
                            &file,
                            &relative,
                            source,
                            family_kind.as_str(),
                            Some(stem),
                            deadline,
                            cancellation,
                        )?)?;
                        (stem, synonym)
                    };
                    add_path_match(
                        located,
                        find_document(
                            source,
                            &format!(
                                "{}:{owner_kind}.{owner_name_text}.{}.{nested_name}",
                                source.name,
                                family_kind.as_str()
                            ),
                            family_kind.as_str(),
                            nested_name,
                            synonym.as_deref(),
                            &relative,
                        )?,
                    )?;
                }
            }
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn read_target_identity(
        &self,
        file: &RetainedRegularFileCapability,
        relative: &Path,
        source: &LayoutFindSource<'_>,
        kind: &str,
        expected_name: Option<&str>,
        deadline: ProviderDeadline,
        cancellation: &CancellationToken,
    ) -> Result<Option<(String, Option<String>)>, FindBuildError> {
        let mut evidence = DirectoryBuild {
            documents: Vec::new(),
            fact_bytes: 0,
            attempted_entries: 0,
            attempted_fact_bytes: 0,
            omissions: FindOmissions::default(),
            include_module_aliases: false,
        };
        let head = self.read_descriptor_head(
            file,
            relative,
            source,
            &mut evidence,
            deadline,
            cancellation,
        )?;
        let head = head.ok_or_else(|| {
            FindBuildError::with_detail(
                RefusalDetail::SourceUnreadable,
                "resolve cannot read the requested descriptor",
            )
        })?;
        let (name, synonym) = descriptor_identity(&head);
        let Some(name) = name else {
            return Ok(None);
        };
        if expected_name.is_some_and(|expected| expected != name) {
            return Ok(None);
        }
        if !declares_owner(&head, kind, &name) {
            return Ok(None);
        }
        Ok(Some((name, synonym)))
    }

    fn add_source(
        &self,
        source: &LayoutFindSource<'_>,
        build: &mut DirectoryBuild,
        deadline: ProviderDeadline,
        cancellation: &CancellationToken,
    ) -> Result<(), FindBuildError> {
        if matches!(
            source.kind,
            SourceSetKind::ExternalProcessor | SourceSetKind::ExternalReport
        ) {
            return self.add_external_source(source, build, deadline, cancellation);
        }
        let configuration = Path::new("Configuration.xml");
        let configuration_file = match retain_optional_child(
            source.root,
            OsStr::new("Configuration.xml"),
            deadline,
            cancellation,
        ) {
            Ok(Some(RetainedChildCapability::RegularFile(file))) => Some(file),
            Ok(Some(_)) => return Err(unsafe_layout_entry()),
            Ok(None) => None,
            Err(error) if error.is_local_unreadable() => {
                build.record_omission(source.name, "descriptor_unreadable");
                None
            }
            Err(error) => return Err(error),
        };
        if let Some(head) = configuration_file
            .as_ref()
            .map(|file| {
                self.read_descriptor_head(
                    file,
                    configuration,
                    source,
                    build,
                    deadline,
                    cancellation,
                )
            })
            .transpose()?
            .flatten()
        {
            let (name, synonym) = descriptor_identity(&head);
            self.push(
                build,
                source,
                &format!("{}:Configuration", source.name),
                NodeKind::Configuration.as_str(),
                name.as_deref().unwrap_or("Configuration"),
                synonym.as_deref(),
                configuration,
            )?;
        }
        for entry in immediate_names(
            source.root,
            self.max_collection_entries,
            deadline,
            cancellation,
            self.capacity_observer.as_deref(),
        )? {
            find_checkpoint(deadline, cancellation)?;
            let Some(directory) = entry.to_str() else {
                continue;
            };
            let Some(layout) = metadata_kind_by_directory(directory) else {
                continue;
            };
            let collection =
                match retain_enumerated_child(source.root, &entry, deadline, cancellation)? {
                    RetainedChildCapability::Directory(collection) => collection,
                    _ => return Err(unsafe_layout_entry()),
                };
            let mut proved_owners = HashMap::new();
            let mut owner_directories = Vec::new();
            let mut unsafe_owner_directories = HashSet::new();
            for owner in immediate_names(
                &collection,
                self.max_collection_entries,
                deadline,
                cancellation,
                self.capacity_observer.as_deref(),
            )? {
                find_checkpoint(deadline, cancellation)?;
                let Some(owner_name) = owner.to_str() else {
                    continue;
                };
                let owner_child =
                    match retain_enumerated_child(&collection, &owner, deadline, cancellation) {
                        Ok(child) => child,
                        Err(error)
                            if error.is_local_unreadable() && owner_name.ends_with(".xml") =>
                        {
                            build.record_omission(source.name, "descriptor_unreadable");
                            continue;
                        }
                        Err(error) => return Err(error),
                    };
                match owner_child {
                    RetainedChildCapability::RegularFile(file) => {
                        let Some(stem) = owner_name.strip_suffix(".xml") else {
                            continue;
                        };
                        let relative = PathBuf::from(directory).join(owner_name);
                        // A file whose name looks like an object is not one:
                        // only a descriptor that declares the expected owner
                        // element and name enters the directory.
                        let Some(head) = self.read_descriptor_head(
                            &file,
                            &relative,
                            source,
                            build,
                            deadline,
                            cancellation,
                        )?
                        else {
                            continue;
                        };
                        if !declares_owner(&head, layout.tag, stem) {
                            continue;
                        }
                        let synonym = descriptor_identity(&head).1;
                        let document_index = build.documents.len();
                        self.push(
                            build,
                            source,
                            &format!("{}:{}.{stem}", source.name, layout.tag),
                            layout.tag,
                            stem,
                            synonym.as_deref(),
                            &relative,
                        )?;
                        proved_owners.insert(
                            stem.to_string(),
                            (build.documents.len() > document_index).then_some(document_index),
                        );
                    }
                    RetainedChildCapability::Directory(owner_root) => {
                        if owner_name.ends_with(".xml") {
                            return Err(unsafe_layout_entry());
                        }
                        owner_directories.push((owner_name.to_string(), owner_root.identity()));
                    }
                    _ if owner_name.ends_with(".xml") => return Err(unsafe_layout_entry()),
                    RetainedChildCapability::ReparsePoint
                    | RetainedChildCapability::Unsupported => {
                        // The descriptor is the proof that this directory is
                        // an owner. A linked or unsupported matching directory
                        // must not silently hide its nested objects.
                        unsafe_owner_directories.insert(owner_name.to_string());
                    }
                }
            }
            if unsafe_owner_directories
                .iter()
                .any(|name| proved_owners.contains_key(name))
            {
                return Err(unsafe_layout_entry());
            }
            for (owner_name, original_identity) in owner_directories {
                find_checkpoint(deadline, cancellation)?;
                if let Some(document_index) = proved_owners.get(&owner_name) {
                    let owner_root = match retain_enumerated_child(
                        &collection,
                        OsStr::new(&owner_name),
                        deadline,
                        cancellation,
                    )? {
                        RetainedChildCapability::Directory(owner_root)
                            if owner_root.identity() == original_identity =>
                        {
                            owner_root
                        }
                        _ => return Err(unsafe_layout_entry()),
                    };
                    if build.include_module_aliases && layout.tag == "CommonModule" {
                        if let Some(document_index) = document_index {
                            let alias = match common_module_path(
                                &owner_root,
                                &PathBuf::from(directory).join(&owner_name),
                                None,
                                None,
                                deadline,
                                cancellation,
                            ) {
                                Ok(alias) => alias,
                                Err(error) if error.is_local_unreadable() => {
                                    find_checkpoint(deadline, cancellation)?;
                                    build.record_omission(source.name, "module_unreadable");
                                    None
                                }
                                Err(error) => return Err(error),
                            };
                            let alias = alias.map(|relative| FindPathAlias {
                                absolute: source
                                    .root
                                    .path()
                                    .join(&relative)
                                    .to_string_lossy()
                                    .into_owned(),
                                relative,
                            });
                            let next_total = build.fact_bytes.saturating_add(
                                alias.as_ref().map_or(0, FindPathAlias::estimated_bytes),
                            );
                            if next_total > self.max_total_fact_bytes {
                                return Err(FindBuildError::new(
                                    RefusalCode::ProviderLimitExceeded,
                                    "find directory exceeds the bounded workspace byte budget",
                                ));
                            }
                            build.fact_bytes = next_total;
                            build.documents[*document_index].set_path_alias(alias);
                        }
                    }
                    self.add_nested_families(
                        source,
                        build,
                        &owner_root,
                        layout.tag,
                        &owner_name,
                        &PathBuf::from(directory).join(&owner_name),
                        deadline,
                        cancellation,
                    )?;
                }
            }
        }
        Ok(())
    }

    /// An external processor or report keeps its single owner descriptor at
    /// the root of the source set.
    fn add_external_source(
        &self,
        source: &LayoutFindSource<'_>,
        build: &mut DirectoryBuild,
        deadline: ProviderDeadline,
        cancellation: &CancellationToken,
    ) -> Result<(), FindBuildError> {
        let kind = if source.kind == SourceSetKind::ExternalProcessor {
            NodeKind::ExternalDataProcessor
        } else {
            NodeKind::ExternalReport
        };
        for entry in immediate_names(
            source.root,
            self.max_collection_entries,
            deadline,
            cancellation,
            self.capacity_observer.as_deref(),
        )? {
            find_checkpoint(deadline, cancellation)?;
            let Some(entry_name) = entry.to_str() else {
                continue;
            };
            let Some(stem) = entry_name.strip_suffix(".xml") else {
                continue;
            };
            let file = match retain_enumerated_child(source.root, &entry, deadline, cancellation) {
                Ok(RetainedChildCapability::RegularFile(file)) => file,
                Err(error) if error.is_local_unreadable() => {
                    build.record_omission(source.name, "descriptor_unreadable");
                    continue;
                }
                Err(error) => return Err(error),
                Ok(_) => return Err(unsafe_layout_entry()),
            };
            let relative = PathBuf::from(entry_name);
            // A Designer dump keeps `ConfigDumpInfo.xml` next to the owner
            // descriptor; only a file that declares the expected owner element
            // is an object.
            let Some(head) =
                self.read_descriptor_head(&file, &relative, source, build, deadline, cancellation)?
            else {
                continue;
            };
            if !declares_owner(&head, kind.as_str(), stem) {
                continue;
            }
            let synonym = descriptor_identity(&head).1;
            self.push(
                build,
                source,
                &format!("{}:{}.{stem}", source.name, kind.as_str()),
                kind.as_str(),
                stem,
                synonym.as_deref(),
                &relative,
            )?;
            match retain_optional_child(source.root, OsStr::new(stem), deadline, cancellation)? {
                Some(RetainedChildCapability::Directory(owner_root)) => {
                    self.add_nested_families(
                        source,
                        build,
                        &owner_root,
                        kind.as_str(),
                        stem,
                        &PathBuf::from(stem),
                        deadline,
                        cancellation,
                    )?;
                }
                Some(_) => return Err(unsafe_layout_entry()),
                None => {}
            }
        }
        Ok(())
    }

    /// Forms and templates own a descriptor file; a command owns only its
    /// directory. Both are addressed straight from the layout.
    #[allow(clippy::too_many_arguments)]
    fn add_nested_families(
        &self,
        source: &LayoutFindSource<'_>,
        build: &mut DirectoryBuild,
        owner_root: &RetainedDirectoryCapability,
        owner_kind: &str,
        owner_name: &str,
        owner_relative: &Path,
        deadline: ProviderDeadline,
        cancellation: &CancellationToken,
    ) -> Result<(), FindBuildError> {
        for (family_directory, family_kind) in NESTED_FAMILIES {
            find_checkpoint(deadline, cancellation)?;
            let family = match retain_optional_child(
                owner_root,
                OsStr::new(family_directory),
                deadline,
                cancellation,
            )? {
                Some(RetainedChildCapability::Directory(family)) => family,
                Some(_) => return Err(unsafe_layout_entry()),
                None => continue,
            };
            for entry in immediate_names(
                &family,
                self.max_collection_entries,
                deadline,
                cancellation,
                self.capacity_observer.as_deref(),
            )? {
                find_checkpoint(deadline, cancellation)?;
                let Some(entry_name) = entry.to_str() else {
                    continue;
                };
                let child = match retain_enumerated_child(&family, &entry, deadline, cancellation) {
                    Ok(child) => child,
                    Err(error) if error.is_local_unreadable() && entry_name.ends_with(".xml") => {
                        build.record_omission(source.name, "descriptor_unreadable");
                        continue;
                    }
                    Err(error) => return Err(error),
                };
                let (child_name, relative, descriptor) = match child {
                    RetainedChildCapability::RegularFile(file) => {
                        let Some(stem) = entry_name.strip_suffix(".xml") else {
                            continue;
                        };
                        (
                            stem.to_string(),
                            owner_relative.join(family_directory).join(entry_name),
                            Some(file),
                        )
                    }
                    // A command has no descriptor file: its directory carries
                    // only the module, and the name is the directory itself.
                    RetainedChildCapability::Directory(_) if family_kind == NodeKind::Command => (
                        entry_name.to_string(),
                        owner_relative.join(family_directory).join(entry_name),
                        None,
                    ),
                    _ if entry_name.ends_with(".xml") => return Err(unsafe_layout_entry()),
                    RetainedChildCapability::ReparsePoint
                    | RetainedChildCapability::Unsupported
                        if family_kind == NodeKind::Command =>
                    {
                        return Err(unsafe_layout_entry());
                    }
                    _ => continue,
                };
                let synonym = if family_kind == NodeKind::Command {
                    None
                } else {
                    let Some(head) = self.read_descriptor_head(
                        descriptor.as_ref().expect("non-command has descriptor"),
                        &relative,
                        source,
                        build,
                        deadline,
                        cancellation,
                    )?
                    else {
                        continue;
                    };
                    if !declares_owner(&head, family_kind.as_str(), &child_name) {
                        continue;
                    }
                    descriptor_identity(&head).1
                };
                self.push(
                    build,
                    source,
                    &format!(
                        "{}:{owner_kind}.{owner_name}.{}.{child_name}",
                        source.name,
                        family_kind.as_str()
                    ),
                    family_kind.as_str(),
                    &child_name,
                    synonym.as_deref(),
                    &relative,
                )?;
            }
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn push(
        &self,
        build: &mut DirectoryBuild,
        source: &LayoutFindSource<'_>,
        at: &str,
        kind: &str,
        name: &str,
        synonym: Option<&str>,
        relative: &Path,
    ) -> Result<(), FindBuildError> {
        if QualifiedAddress::parse(at).is_err() {
            // A directory entry whose name is not addressable is not an
            // object; the layout simply does not describe it.
            return Ok(());
        }
        let _ = source;
        let path = path_text(relative);
        let mut facts = vec![
            FindFact::new(FindFactKind::Name, name),
            FindFact::new(FindFactKind::ExportPath, &path),
        ];
        if let Some(synonym) = synonym.filter(|value| !value.is_empty() && *value != name) {
            facts.push(FindFact::new(FindFactKind::Synonym, synonym));
        }
        let title = synonym.filter(|value| !value.is_empty()).unwrap_or(name);
        let document = FindDocument::new(at, kind, title, facts).with_path(path);
        let next_total = build
            .fact_bytes
            .saturating_add(document.estimated_identity_bytes());
        build.attempted_entries = build.documents.len().saturating_add(1);
        build.attempted_fact_bytes = next_total;
        if build.documents.len() == self.max_documents {
            return Err(FindBuildError::new(
                RefusalCode::ProviderLimitExceeded,
                "find directory exceeds the bounded workspace entry limit",
            ));
        }
        if next_total > self.max_total_fact_bytes {
            return Err(FindBuildError::new(
                RefusalCode::ProviderLimitExceeded,
                "find directory exceeds the bounded workspace byte budget",
            ));
        }
        build.fact_bytes = next_total;
        build.documents.push(document);
        Ok(())
    }

    fn read_descriptor_head(
        &self,
        file: &RetainedRegularFileCapability,
        relative: &Path,
        source: &LayoutFindSource<'_>,
        build: &mut DirectoryBuild,
        deadline: ProviderDeadline,
        cancellation: &CancellationToken,
    ) -> Result<Option<Vec<u8>>, FindBuildError> {
        find_checkpoint(deadline, cancellation)?;
        file.validate_named_identity()
            .map_err(|_| unsafe_layout_entry())?;
        let head = (self.read_head)(file, relative);
        find_checkpoint(deadline, cancellation)?;
        file.validate_named_identity()
            .map_err(|_| unsafe_layout_entry())?;
        match head {
            Ok(head) => Ok(Some(head)),
            Err(DescriptorHeadReadError::Local(io::ErrorKind::NotFound)) => {
                Err(FindBuildError::new(
                    RefusalCode::ConcurrentChange,
                    "retained descriptor disappeared during reading",
                ))
            }
            Err(DescriptorHeadReadError::Local(io::ErrorKind::OutOfMemory)) => {
                Err(FindBuildError::new(
                    RefusalCode::ProviderUnavailable,
                    "find could not allocate the descriptor read buffer",
                ))
            }
            Err(DescriptorHeadReadError::Local(_)) => {
                build.record_omission(source.name, "descriptor_unreadable");
                Ok(None)
            }
            Err(DescriptorHeadReadError::Capability) => Err(FindBuildError::new(
                RefusalCode::ProviderUnavailable,
                "find could not retain a descriptor read handle",
            )),
        }
    }
}

fn matching_names(
    directory: &RetainedDirectoryCapability,
    normalized_name: &str,
    physical_name: Option<&OsStr>,
    deadline: ProviderDeadline,
    cancellation: &CancellationToken,
) -> Result<Vec<std::ffi::OsString>, FindBuildError> {
    let mut found = Vec::new();
    let comparator = physical_name
        .map(|_| directory.child_name_comparator().map_err(child_read_error))
        .transpose()?;
    directory.visit_immediate_names(|name| {
        find_checkpoint(deadline, cancellation)?;
        let matches = if let (Some(physical_name), Some(comparator)) = (physical_name, &comparator)
        {
            comparator
                .names_equivalent(&name, physical_name)
                .map_err(child_read_error)?
        } else {
            name.to_str()
                .is_some_and(|name| name.to_lowercase() == normalized_name)
        };
        if matches {
            if !found.is_empty() {
                return Err(FindBuildError::new(
                    RefusalCode::BadValue,
                    "resolve path has case-insensitive filesystem ambiguity",
                ));
            }
            found.push(name);
        }
        Ok(())
    })?;
    Ok(found)
}

fn require_target_identity(
    identity: Option<(String, Option<String>)>,
) -> Result<(String, Option<String>), FindBuildError> {
    identity.ok_or_else(|| {
        FindBuildError::new(
            RefusalCode::InvalidSource,
            "resolve target descriptor does not declare the expected owner and name",
        )
    })
}

fn find_document(
    source: &LayoutFindSource<'_>,
    at: &str,
    kind: &str,
    name: &str,
    synonym: Option<&str>,
    relative: &Path,
) -> Result<FindDocument, FindBuildError> {
    if QualifiedAddress::parse(at)
        .ok()
        .is_none_or(|address| address.source_set() != source.name)
    {
        return Err(FindBuildError::new(
            RefusalCode::InvalidSource,
            "resolve target has no valid logical address",
        ));
    }
    let path = path_text(relative);
    let mut facts = vec![
        FindFact::new(FindFactKind::Name, name),
        FindFact::new(FindFactKind::ExportPath, &path),
    ];
    if let Some(synonym) = synonym.filter(|value| !value.is_empty() && *value != name) {
        facts.push(FindFact::new(FindFactKind::Synonym, synonym));
    }
    let title = synonym.filter(|value| !value.is_empty()).unwrap_or(name);
    Ok(FindDocument::new(at, kind, title, facts).with_path(path))
}

fn add_path_match(
    located: &mut Option<FindDocument>,
    candidate: FindDocument,
) -> Result<(), FindBuildError> {
    if let Some(previous) = located {
        let previous_len = previous.placed_path().map_or(0, str::len);
        let candidate_len = candidate.placed_path().map_or(0, str::len);
        if candidate_len > previous_len {
            *previous = candidate;
        } else if candidate_len == previous_len
            && (previous.at() != candidate.at()
                || previous.placed_path() != candidate.placed_path())
        {
            return Err(FindBuildError::new(
                RefusalCode::BadValue,
                "resolve path matches multiple equally specific objects; use their logical address",
            ));
        }
    } else {
        *located = Some(candidate);
    }
    Ok(())
}

fn common_module_path(
    owner: &RetainedDirectoryCapability,
    relative: &Path,
    physical_ext: Option<&OsStr>,
    physical_module: Option<&OsStr>,
    deadline: ProviderDeadline,
    cancellation: &CancellationToken,
) -> Result<Option<String>, FindBuildError> {
    if let Some(physical_ext) = physical_ext {
        if !owner
            .child_names_equivalent(physical_ext, OsStr::new("Ext"))
            .map_err(child_read_error)?
        {
            return Ok(None);
        }
    }
    let ext = match retain_optional_child(owner, OsStr::new("Ext"), deadline, cancellation)? {
        Some(RetainedChildCapability::Directory(ext)) => ext,
        None => return Ok(None),
        Some(_) => return Err(unsafe_layout_entry()),
    };
    if let Some(physical_module) = physical_module {
        if !ext
            .child_names_equivalent(physical_module, OsStr::new("Module.bsl"))
            .map_err(child_read_error)?
        {
            return Ok(None);
        }
    }
    match retain_optional_child(&ext, OsStr::new("Module.bsl"), deadline, cancellation)? {
        Some(RetainedChildCapability::RegularFile(file)) => {
            file.validate_named_identity()
                .map_err(|_| unsafe_layout_entry())?;
            ext.validate_named_identity()
                .map_err(|_| unsafe_layout_entry())?;
            owner
                .validate_named_identity()
                .map_err(|_| unsafe_layout_entry())?;
            find_checkpoint(deadline, cancellation)?;
            Ok(Some(path_text(&relative.join("Ext/Module.bsl"))))
        }
        None => Ok(None),
        Some(_) => Err(unsafe_layout_entry()),
    }
}

fn common_module_query_matches(source: &LayoutFindSource<'_>, query: &str, relative: &str) -> bool {
    let query_normalized = normalize_layout_path(query);
    let absolute = normalize_layout_path(&source.root.path().join(relative).to_string_lossy());
    if query_normalized == absolute {
        return true;
    }
    if layout_path_is_absolute(query, &query_normalized) {
        return false;
    }
    query_normalized == normalize_layout_path(relative)
        || absolute
            .strip_suffix(&query_normalized)
            .is_some_and(|prefix| prefix.ends_with('/'))
}

fn layout_path_is_absolute(query: &str, normalized: &str) -> bool {
    let bytes = normalized.as_bytes();
    Path::new(query).is_absolute()
        || normalized.starts_with("//")
        || (bytes.len() >= 3
            && bytes[0].is_ascii_alphabetic()
            && bytes[1] == b':'
            && bytes[2] == b'/')
}

fn source_path_ends_with_query(source: &LayoutFindSource<'_>, query: &str, relative: &str) -> bool {
    let candidate = normalize_layout_path(&source.root.path().join(relative).to_string_lossy());
    candidate == query
        || candidate
            .strip_suffix(query)
            .is_some_and(|prefix| prefix.ends_with('/'))
}

fn source_path_matches_query(source: &LayoutFindSource<'_>, query: &str, relative: &str) -> bool {
    query == relative || source_path_ends_with_query(source, query, relative)
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct AbsoluteTargetWitness {
    identity: FileIdentity,
    relative: PathBuf,
}

/// Compare the ancestor at the retained root's depth by its no-follow
/// identity. This accepts real case-insensitive aliases without folding two
/// distinct roots on a case-sensitive filesystem into one source.
fn absolute_query_relative_path(
    source: &LayoutFindSource<'_>,
    query: &str,
    deadline: ProviderDeadline,
    cancellation: &CancellationToken,
) -> Result<Option<PathBuf>, FindBuildError> {
    let query_path = Path::new(query);
    let root_depth = source.root.path().components().count();
    for ancestor in query_path.ancestors() {
        find_checkpoint(deadline, cancellation)?;
        if ancestor.components().count() != root_depth {
            continue;
        }
        // At each component, the *query* parent's real filesystem policy
        // decides name equivalence. This avoids opening a foreign A/a parent
        // on a case-sensitive FS and also accepts physical Unicode aliases.
        let source_prefixes = source.root.path().ancestors().collect::<Vec<_>>();
        let query_prefixes = ancestor.ancestors().collect::<Vec<_>>();
        if source_prefixes.len() != query_prefixes.len() {
            return Ok(None);
        }
        let mut prefixes = source_prefixes
            .iter()
            .rev()
            .zip(query_prefixes.iter().rev());
        let Some((source_anchor, query_anchor)) = prefixes.next() else {
            return Ok(None);
        };
        if source_anchor
            .to_str()
            .zip(query_anchor.to_str())
            .is_none_or(|(source, query)| {
                normalize_layout_path(source) != normalize_layout_path(query)
            })
        {
            return Ok(None);
        }
        let mut current =
            RetainedDirectoryCapability::open(query_anchor).map_err(child_read_error)?;
        for (source_prefix, query_prefix) in prefixes {
            find_checkpoint(deadline, cancellation)?;
            let (Some(source_name), Some(query_name)) =
                (source_prefix.file_name(), query_prefix.file_name())
            else {
                return Ok(None);
            };
            if !current
                .child_names_equivalent(source_name, query_name)
                .map_err(child_read_error)?
            {
                return Ok(None);
            }
            current = match retain_optional_child(&current, query_name, deadline, cancellation)? {
                Some(RetainedChildCapability::Directory(child)) => {
                    child.validate_named_identity().map_err(child_read_error)?;
                    child
                }
                None => {
                    return Err(FindBuildError::new(
                        RefusalCode::ConcurrentChange,
                        "resolve source root changed during path lookup",
                    ));
                }
                Some(_) => return Err(unsafe_layout_entry()),
            };
        }
        if current.identity() != source.root.identity() {
            return Err(FindBuildError::new(
                RefusalCode::ConcurrentChange,
                "resolve source root changed during path lookup",
            ));
        }
        let relative = query_path
            .strip_prefix(ancestor)
            .map_err(|_| unsafe_layout_entry())?;
        return Ok(Some(relative.to_path_buf()));
    }
    Ok(None)
}

/// The selected source retains exactly the requested target through its own
/// root; unrelated source trees and their descriptors are not consulted.
fn retain_absolute_target(
    source: &LayoutFindSource<'_>,
    relative: &Path,
    deadline: ProviderDeadline,
    cancellation: &CancellationToken,
) -> Result<Option<AbsoluteTargetWitness>, FindBuildError> {
    let depth = relative.components().count();
    if !(1..=4).contains(&depth) {
        return Ok(None);
    }
    let mut current = source.root.clone();
    for (index, component) in relative.components().enumerate() {
        let std::path::Component::Normal(name) = component else {
            return Ok(None);
        };
        let child = retain_optional_child(&current, name, deadline, cancellation)?;
        if index + 1 == depth {
            let identity = match child {
                Some(RetainedChildCapability::Directory(child)) => {
                    child
                        .validate_named_identity()
                        .map_err(|_| unsafe_layout_entry())?;
                    child.identity()
                }
                Some(RetainedChildCapability::RegularFile(child)) => {
                    child
                        .validate_named_identity()
                        .map_err(|_| unsafe_layout_entry())?;
                    child.identity()
                }
                None => return Ok(None),
                Some(_) => return Err(unsafe_layout_entry()),
            };
            return Ok(Some(AbsoluteTargetWitness {
                identity,
                relative: relative.to_path_buf(),
            }));
        }
        current = match child {
            Some(RetainedChildCapability::Directory(child)) => child,
            None => return Ok(None),
            Some(_) => return Err(unsafe_layout_entry()),
        };
    }
    Ok(None)
}

fn absolute_query_witness(
    source: &LayoutFindSource<'_>,
    query: &str,
    deadline: ProviderDeadline,
    cancellation: &CancellationToken,
) -> Result<Option<AbsoluteTargetWitness>, FindBuildError> {
    let Some(relative) = absolute_query_relative_path(source, query, deadline, cancellation)?
    else {
        return Ok(None);
    };
    retain_absolute_target(source, &relative, deadline, cancellation)
}

fn absolute_query_matches_placement(
    source: &LayoutFindSource<'_>,
    witness: &AbsoluteTargetWitness,
    placed_path: &str,
    deadline: ProviderDeadline,
    cancellation: &CancellationToken,
) -> Result<bool, FindBuildError> {
    let requested = witness.relative.components().collect::<Vec<_>>();
    let placed = Path::new(placed_path).components().collect::<Vec<_>>();
    let depth = requested.len();
    if depth != placed.len() || !(1..=4).contains(&depth) {
        return Ok(false);
    }
    let mut parent = source.root.clone();
    for (index, (requested, placed)) in requested.iter().zip(&placed).enumerate() {
        let (std::path::Component::Normal(requested), std::path::Component::Normal(placed)) =
            (requested, placed)
        else {
            return Ok(false);
        };
        if !parent
            .child_names_equivalent(requested, placed)
            .map_err(child_read_error)?
        {
            return Ok(false);
        }
        if index + 1 != depth {
            parent = match retain_optional_child(&parent, requested, deadline, cancellation)? {
                Some(RetainedChildCapability::Directory(next)) => next,
                None => return Ok(false),
                Some(_) => return Err(unsafe_layout_entry()),
            };
        }
    }
    let placed_absolute = source.root.path().join(placed_path);
    let Some(placed_absolute) = placed_absolute.to_str() else {
        return Ok(false);
    };
    Ok(
        absolute_query_witness(source, placed_absolute, deadline, cancellation)?
            .is_some_and(|placed| placed.identity == witness.identity),
    )
}

fn read_descriptor_head_prefix(
    file: &RetainedRegularFileCapability,
    _relative: &Path,
) -> Result<Vec<u8>, DescriptorHeadReadError> {
    use io::{Read, Seek, SeekFrom};

    let mut handle = file
        .try_clone_file()
        .map_err(|_| DescriptorHeadReadError::Capability)?;
    handle
        .seek(SeekFrom::Start(0))
        .map_err(|error| DescriptorHeadReadError::Local(error.kind()))?;
    let mut head = Vec::new();
    handle
        .take(DESCRIPTOR_HEAD_BYTES as u64)
        .read_to_end(&mut head)
        .map_err(|error| DescriptorHeadReadError::Local(error.kind()))?;
    Ok(head)
}

fn retain_optional_child(
    directory: &RetainedDirectoryCapability,
    name: &OsStr,
    deadline: ProviderDeadline,
    cancellation: &CancellationToken,
) -> Result<Option<RetainedChildCapability>, FindBuildError> {
    find_checkpoint(deadline, cancellation)?;
    match directory.retain_immediate_child_nofollow(name) {
        Ok(child) => Ok(Some(child)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(child_read_error(error)),
    }
}

fn retain_enumerated_child(
    directory: &RetainedDirectoryCapability,
    name: &OsStr,
    deadline: ProviderDeadline,
    cancellation: &CancellationToken,
) -> Result<RetainedChildCapability, FindBuildError> {
    find_checkpoint(deadline, cancellation)?;
    directory
        .retain_immediate_child_nofollow(name)
        .map_err(child_read_error)
}

fn child_read_error(error: io::Error) -> FindBuildError {
    if error.kind() == io::ErrorKind::PermissionDenied {
        return FindBuildError::with_detail(
            RefusalDetail::SourceUnreadable,
            "find source layout changed or could not be read safely",
        );
    }
    let code = match error.kind() {
        io::ErrorKind::NotFound => RefusalCode::ConcurrentChange,
        _ => RefusalCode::InvalidSource,
    };
    FindBuildError::new(
        code,
        "find source layout changed or could not be read safely",
    )
}

fn unsafe_layout_entry() -> FindBuildError {
    FindBuildError::new(
        RefusalCode::InvalidSource,
        "find source layout has an unexpected entry type",
    )
}

fn immediate_names(
    directory: &RetainedDirectoryCapability,
    maximum_entries: usize,
    deadline: ProviderDeadline,
    cancellation: &CancellationToken,
    observer: Option<&CapacityObserver>,
) -> Result<Vec<std::ffi::OsString>, FindBuildError> {
    directory
        .read_immediate_names_bounded(maximum_entries, || {
            find_checkpoint(deadline, cancellation)
                .map_err(|error| std::io::Error::other(error.to_string()))
        })
        .inspect(|names| {
            if let Some(observer) = observer {
                observer.record_find_collection(names.len() as u64, true);
            }
        })
        .map_err(|error| {
            if cancellation.is_cancelled() {
                FindBuildError::new(RefusalCode::Cancelled, "find directory build was cancelled")
            } else if deadline.remaining().is_zero() {
                FindBuildError::new(
                    RefusalCode::DeadlineExceeded,
                    "find directory build deadline elapsed",
                )
            } else if error.kind() == io::ErrorKind::PermissionDenied {
                FindBuildError::with_detail(
                    RefusalDetail::SourceUnreadable,
                    "find could not read the source layout",
                )
            } else if error.kind() == io::ErrorKind::FileTooLarge {
                if let Some(observer) = observer {
                    observer.record_find_collection(maximum_entries as u64 + 1, false);
                }
                FindBuildError::new(
                    RefusalCode::ProviderLimitExceeded,
                    "find source collection exceeds the bounded entry limit",
                )
            } else {
                FindBuildError::new(
                    RefusalCode::ProviderUnavailable,
                    "find could not read the source layout",
                )
            }
        })
}

/// Reads `Name` and the first localized `Synonym` out of a descriptor head
/// without parsing the document: a malformed or truncated descriptor simply
/// contributes no synonym instead of failing the directory.
/// Whether a descriptor head declares the expected owner element and name.
fn declares_owner(head: &[u8], kind: &str, name: &str) -> bool {
    let text = String::from_utf8_lossy(head);
    // XML allows any whitespace between the element name and its attributes,
    // and a pretty-printed descriptor may put the uuid on the next line.
    let open = format!("<{kind}");
    let opens = text.match_indices(&open).any(|(offset, _)| {
        matches!(
            text.as_bytes().get(offset + open.len()),
            Some(b'>' | b' ' | b'\t' | b'\r' | b'\n')
        )
    });
    opens && between(&text, "<Name>", "</Name>").is_some_and(|declared| declared == name)
}

fn descriptor_identity(head: &[u8]) -> (Option<String>, Option<String>) {
    let text = String::from_utf8_lossy(head);
    let name = between(&text, "<Name>", "</Name>").map(str::to_string);
    let synonym = text.find("<Synonym>").and_then(|start| {
        let rest = &text[start..];
        between(rest, "<v8:content>", "</v8:content>").map(str::to_string)
    });
    (name, synonym)
}

fn between<'a>(text: &'a str, open: &str, close: &str) -> Option<&'a str> {
    let start = text.find(open)? + open.len();
    let end = text[start..].find(close)? + start;
    Some(text[start..end].trim()).filter(|value| !value.is_empty())
}

fn path_text(relative: &Path) -> String {
    relative
        .components()
        .filter_map(|component| component.as_os_str().to_str())
        .collect::<Vec<_>>()
        .join("/")
}

fn find_checkpoint(
    deadline: ProviderDeadline,
    cancellation: &CancellationToken,
) -> Result<(), FindBuildError> {
    if cancellation.is_cancelled() {
        return Err(FindBuildError::new(
            RefusalCode::Cancelled,
            "find directory build was cancelled",
        ));
    }
    if deadline.remaining().is_zero() {
        return Err(FindBuildError::new(
            RefusalCode::DeadlineExceeded,
            "find directory build deadline elapsed",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{LayoutFindSource, WorkspaceFindDirectoryBuilder, DESCRIPTOR_HEAD_BYTES};
    use crate::application::v13::find::FindRequest;
    use crate::domain::cancellation::CancellationToken;
    use crate::domain::code_intelligence::ProviderDeadline;
    use crate::domain::project_sources::SourceSetKind;
    use crate::domain::refusal::RefusalCode;
    use crate::infrastructure::capacity_observation::CapacityObserver;
    use crate::infrastructure::platform::filesystem::RetainedDirectoryCapability;
    use std::fs;
    use std::io;
    use std::path::Path;
    use std::sync::Arc;
    use std::time::Duration;

    fn write(path: &Path, contents: &str) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, contents).unwrap();
    }

    fn owner(name: &str, kind: &str, synonym: &str, children: &str) -> String {
        format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" xmlns:v8="http://v8.1c.ru/8.1/data/core" version="2.20">
  <{kind} uuid="10000000-0000-4000-8000-000000000001">
    <Properties><Name>{name}</Name><Synonym><v8:item><v8:lang>ru</v8:lang><v8:content>{synonym}</v8:content></v8:item></Synonym></Properties>
    <ChildObjects>{children}</ChildObjects>
  </{kind}>
</MetaDataObject>"#
        )
    }

    struct Fixture {
        _root: tempfile::TempDir,
        source: std::path::PathBuf,
    }

    impl Fixture {
        fn new() -> Self {
            let root = tempfile::tempdir().unwrap();
            let source = root.path().canonicalize().unwrap().join("src");
            write(
                &source.join("Configuration.xml"),
                &owner(
                    "Магазин",
                    "Configuration",
                    "Магазин",
                    "<Catalog>Валюты</Catalog>",
                ),
            );
            write(
                &source.join("Catalogs/Валюты.xml"),
                &owner(
                    "Валюты",
                    "Catalog",
                    "Валюты и курсы",
                    "<Form>ФормаЭлемента</Form>",
                ),
            );
            write(
                &source.join("Catalogs/Валюты/Forms/ФормаЭлемента.xml"),
                &owner("ФормаЭлемента", "Form", "Форма элемента", ""),
            );
            write(
                &source.join("Catalogs/Валюты/Templates/Печать.xml"),
                &owner("Печать", "Template", "Печатная форма", ""),
            );
            write(
                &source.join("Catalogs/Валюты/Commands/Обновить/Ext/CommandModule.bsl"),
                "&AtClient\nProcedure CommandProcessing()\nEndProcedure\n",
            );
            // Module bodies must not be read by the directory at all.
            write(
                &source.join("Catalogs/Валюты/Ext/ObjectModule.bsl"),
                "Procedure СекретныйМетод()\nEndProcedure\n",
            );
            Self {
                _root: root,
                source,
            }
        }

        fn directory_for_paths(&self) -> crate::application::v13::find::FindIndex {
            let root = RetainedDirectoryCapability::open(&self.source).unwrap();
            WorkspaceFindDirectoryBuilder::default()
                .build_for_path(
                    &[LayoutFindSource::new(
                        "main",
                        SourceSetKind::Configuration,
                        &root,
                    )],
                    ProviderDeadline::from_budget(Duration::from_secs(7)),
                    &CancellationToken::new(),
                )
                .unwrap()
        }

        fn directory(&self) -> crate::application::v13::find::FindIndex {
            let root = RetainedDirectoryCapability::open(&self.source).unwrap();
            WorkspaceFindDirectoryBuilder::default()
                .build(
                    &[LayoutFindSource::new(
                        "main",
                        SourceSetKind::Configuration,
                        &root,
                    )],
                    ProviderDeadline::from_budget(Duration::from_secs(7)),
                    &CancellationToken::new(),
                )
                .unwrap()
        }
    }

    fn single(index: &crate::application::v13::find::FindIndex, query: &str) -> (String, String) {
        let found = index.find(FindRequest::new(query).unwrap());
        let candidate = found
            .candidates()
            .first()
            .unwrap_or_else(|| panic!("{query}: {found:?}"));
        assert!(!found.is_nearest(), "{query}: {found:?}");
        (
            candidate.at().to_string(),
            candidate.path().unwrap_or_default().to_string(),
        )
    }

    #[test]
    fn a_name_resolves_to_the_address_and_the_file_that_carries_it() {
        let index = Fixture::new().directory();
        for (query, at, path) in [
            ("Валюты", "main:Catalog.Валюты", "Catalogs/Валюты.xml"),
            (
                "main:Catalog.Валюты",
                "main:Catalog.Валюты",
                "Catalogs/Валюты.xml",
            ),
            (
                "ФормаЭлемента",
                "main:Catalog.Валюты.Form.ФормаЭлемента",
                "Catalogs/Валюты/Forms/ФормаЭлемента.xml",
            ),
            (
                "Печать",
                "main:Catalog.Валюты.Template.Печать",
                "Catalogs/Валюты/Templates/Печать.xml",
            ),
            (
                "Обновить",
                "main:Catalog.Валюты.Command.Обновить",
                "Catalogs/Валюты/Commands/Обновить",
            ),
            ("Магазин", "main:Configuration", "Configuration.xml"),
        ] {
            assert_eq!(single(&index, query), (at.to_string(), path.to_string()));
        }
    }

    #[test]
    fn the_bridge_locates_a_path_and_an_address_without_guessing() {
        let index = Fixture::new().directory();
        for query in [
            "Catalogs/Валюты.xml",
            "src/Catalogs/Валюты.xml",
            "/home/user/project/src/Catalogs/Валюты.xml",
        ] {
            let located = index
                .locate_path(query)
                .unwrap_or_else(|| panic!("{query} must locate its owner"));
            assert_eq!(located.owner.at(), "main:Catalog.Валюты", "{query}");
        }
        // Хвост принимается только целиком, посегментно: иначе «Валюты.xml»
        // притянул бы «НеВалюты.xml».
        assert!(index.locate_path("алюты.xml").is_none());
        assert!(index.locate_path("Catalogs/Нет.xml").is_none());

        let located = index
            .locate_address("main:Catalog.Валюты")
            .expect("an address locates its own place");
        assert_eq!(located.placed_path(), Some("Catalogs/Валюты.xml"));
        // Мост не гадает: близкого адреса для него не существует.
        assert!(index.locate_address("main:Catalog.Валют").is_none());
    }

    #[test]
    fn a_common_module_file_resolves_to_its_owner_without_becoming_a_name_fact() {
        let fixture = Fixture::new();
        write(
            &fixture.source.join("CommonModules/Main.xml"),
            &owner("Main", "CommonModule", "Main", ""),
        );
        write(
            &fixture.source.join("CommonModules/Main/Ext/Module.bsl"),
            "Procedure HiddenSymbol()\nEndProcedure\n",
        );
        let index = fixture.directory_for_paths();
        let absolute = fixture.source.join("CommonModules/Main/Ext/Module.bsl");
        for path in [
            "CommonModules/Main/Ext/Module.bsl",
            "src/CommonModules/Main/Ext/Module.bsl",
            absolute.to_str().unwrap(),
        ] {
            let found = index.locate_path(path).expect("existing module file");
            assert_eq!(found.owner.at(), "main:CommonModule.Main");
            assert_eq!(found.path, "CommonModules/Main/Ext/Module.bsl");
        }
        assert_eq!(
            single(&index, "Main"),
            (
                "main:CommonModule.Main".into(),
                "CommonModules/Main.xml".into()
            )
        );
        assert_eq!(
            index
                .find(
                    FindRequest::new("Main")
                        .unwrap()
                        .with_kind("CommonModule")
                        .unwrap()
                )
                .candidates()
                .len(),
            1
        );
        for query in ["Module.bsl", "HiddenSymbol"] {
            assert!(index.find(FindRequest::new(query).unwrap()).is_nearest());
        }
        assert_eq!(
            index
                .locate_address("main:CommonModule.Main")
                .unwrap()
                .placed_path(),
            Some("CommonModules/Main.xml")
        );
    }

    #[test]
    fn a_common_module_alias_requires_both_the_descriptor_and_the_file() {
        let fixture = Fixture::new();
        write(
            &fixture.source.join("CommonModules/Main.xml"),
            &owner("Main", "CommonModule", "Main", ""),
        );
        assert!(fixture
            .directory_for_paths()
            .locate_path("CommonModules/Main/Ext/Module.bsl")
            .is_none());
        for directory in ["CommonModules/Main", "CommonModules/Main/Ext"] {
            fs::create_dir_all(fixture.source.join(directory)).unwrap();
            let index = fixture.directory_for_paths();
            assert!(index
                .locate_path("CommonModules/Main/Ext/Module.bsl")
                .is_none());
            assert!(index.locate_address("main:CommonModule.Main").is_some());
        }
        fs::remove_file(fixture.source.join("CommonModules/Main.xml")).unwrap();
        write(
            &fixture.source.join("CommonModules/Main/Ext/Module.bsl"),
            "",
        );
        let index = fixture.directory_for_paths();
        assert!(index
            .locate_path("CommonModules/Main/Ext/Module.bsl")
            .is_none());
        assert!(index.locate_address("main:CommonModule.Main").is_none());
    }

    #[test]
    fn a_common_module_alias_is_not_chosen_between_source_sets() {
        let fixture = Fixture::new();
        write(
            &fixture.source.join("CommonModules/Main.xml"),
            &owner("Main", "CommonModule", "Main", ""),
        );
        write(
            &fixture.source.join("CommonModules/Main/Ext/Module.bsl"),
            "",
        );
        let root = RetainedDirectoryCapability::open(&fixture.source).unwrap();
        let index = WorkspaceFindDirectoryBuilder::default()
            .build_for_path(
                &[
                    LayoutFindSource::new("main", SourceSetKind::Configuration, &root),
                    LayoutFindSource::new("other", SourceSetKind::Extension, &root),
                ],
                ProviderDeadline::from_budget(Duration::from_secs(7)),
                &CancellationToken::new(),
            )
            .unwrap();
        assert!(index
            .locate_path("CommonModules/Main/Ext/Module.bsl")
            .is_none());
        assert!(index.locate_address("main:CommonModule.Main").is_some());
        assert!(index.locate_address("other:CommonModule.Main").is_some());
        assert_eq!(
            index
                .locate_path("CommonModules/Main.xml")
                .unwrap()
                .owner
                .at(),
            "other:CommonModule.Main"
        );
    }

    #[test]
    fn an_absolute_common_module_alias_identifies_its_source_root() {
        let main = Fixture::new();
        let extension = main.source.join("src/extension");
        for source in [&main.source, &extension] {
            write(
                &source.join("CommonModules/Main.xml"),
                &owner("Main", "CommonModule", "Main", ""),
            );
            write(&source.join("CommonModules/Main/Ext/Module.bsl"), "");
        }
        let main_root = RetainedDirectoryCapability::open(&main.source).unwrap();
        let extension_root = RetainedDirectoryCapability::open(&extension).unwrap();
        let index = WorkspaceFindDirectoryBuilder::default()
            .build_for_path(
                &[
                    LayoutFindSource::new("main", SourceSetKind::Configuration, &main_root),
                    LayoutFindSource::new("extension", SourceSetKind::Extension, &extension_root),
                ],
                ProviderDeadline::from_budget(Duration::from_secs(7)),
                &CancellationToken::new(),
            )
            .unwrap();
        assert!(index
            .locate_path("CommonModules/Main/Ext/Module.bsl")
            .is_none());
        for (source, expected) in [
            (&main.source, "main:CommonModule.Main"),
            (&extension, "extension:CommonModule.Main"),
        ] {
            let path = source.join("CommonModules/Main/Ext/Module.bsl");
            for query in [path.clone(), fs::canonicalize(&path).unwrap()] {
                let found = index
                    .locate_path(query.to_str().unwrap())
                    .expect("absolute alias selects its source");
                assert_eq!(found.owner.at(), expected);
                assert_eq!(found.path, "CommonModules/Main/Ext/Module.bsl");
            }
        }
        let found = index
            .locate_path("src/extension/CommonModules/Main/Ext/Module.bsl")
            .expect("workspace-relative alias selects its source");
        assert_eq!(found.owner.at(), "extension:CommonModule.Main");
        assert_eq!(found.path, "CommonModules/Main/Ext/Module.bsl");
        assert!(index
            .locate_path("missing-prefix/CommonModules/Main/Ext/Module.bsl")
            .is_none());
        assert!(index.locate_path("Module.bsl").is_none());
        assert!(index.locate_path("Main/Ext/Module.bsl").is_none());
        let outside = tempfile::tempdir().unwrap();
        assert!(index
            .locate_path(
                outside
                    .path()
                    .join("CommonModules/Main/Ext/Module.bsl")
                    .to_str()
                    .unwrap()
            )
            .is_none());
    }

    #[test]
    fn a_common_module_alias_consumes_the_directory_byte_budget() {
        let root = tempfile::tempdir().unwrap();
        write(
            &root.path().join("CommonModules/Main.xml"),
            &owner("Main", "CommonModule", "Main", ""),
        );
        let capability =
            RetainedDirectoryCapability::open(&root.path().canonicalize().unwrap()).unwrap();
        let sources = [LayoutFindSource::new(
            "main",
            SourceSetKind::Configuration,
            &capability,
        )];
        let deadline = ProviderDeadline::from_budget(Duration::from_secs(7));
        let cancellation = CancellationToken::new();
        let baseline = WorkspaceFindDirectoryBuilder::default()
            .build_for_path(&sources, deadline, &cancellation)
            .unwrap();
        let bytes = baseline
            .locate_address("main:CommonModule.Main")
            .unwrap()
            .estimated_identity_bytes();
        WorkspaceFindDirectoryBuilder::with_limits(1, bytes)
            .build_for_path(&sources, deadline, &cancellation)
            .unwrap();
        write(&root.path().join("CommonModules/Main/Ext/Module.bsl"), "");
        let failure = WorkspaceFindDirectoryBuilder::with_limits(1, bytes)
            .build_for_path(&sources, deadline, &cancellation)
            .unwrap_err();
        assert_eq!(failure.code(), RefusalCode::ProviderLimitExceeded);
        let index = WorkspaceFindDirectoryBuilder::with_limits(
            1,
            bytes
                + "CommonModules/Main/Ext/Module.bsl".len()
                + capability
                    .path()
                    .join("CommonModules/Main/Ext/Module.bsl")
                    .to_string_lossy()
                    .len(),
        )
        .build_for_path(&sources, deadline, &cancellation)
        .unwrap();
        assert!(index
            .locate_path("CommonModules/Main/Ext/Module.bsl")
            .is_some());
    }

    #[test]
    fn a_nonregular_common_module_alias_refuses_the_layout() {
        let fixture = Fixture::new();
        write(
            &fixture.source.join("CommonModules/Main.xml"),
            &owner("Main", "CommonModule", "Main", ""),
        );
        fs::create_dir_all(fixture.source.join("CommonModules/Main/Ext/Module.bsl")).unwrap();
        let root = RetainedDirectoryCapability::open(&fixture.source).unwrap();
        let sources = [LayoutFindSource::new(
            "main",
            SourceSetKind::Configuration,
            &root,
        )];
        let deadline = ProviderDeadline::from_budget(Duration::from_secs(7));
        let cancellation = CancellationToken::new();
        let search = WorkspaceFindDirectoryBuilder::default()
            .build_for_search(&sources, deadline, &cancellation)
            .unwrap();
        assert_eq!(search.omissions.total, 0);
        assert_eq!(single(&search.index, "Main").0, "main:CommonModule.Main");
        assert!(WorkspaceFindDirectoryBuilder::default()
            .build(&sources, deadline, &cancellation)
            .unwrap()
            .locate_address("main:CommonModule.Main")
            .is_some());
        let failure = WorkspaceFindDirectoryBuilder::default()
            .build_for_path(
                &[LayoutFindSource::new(
                    "main",
                    SourceSetKind::Configuration,
                    &root,
                )],
                ProviderDeadline::from_budget(Duration::from_secs(7)),
                &CancellationToken::new(),
            )
            .unwrap_err();
        assert_eq!(failure.code(), RefusalCode::InvalidSource);
    }

    #[test]
    fn a_linked_common_module_alias_refuses_the_layout() {
        use crate::infrastructure::platform::testing::{
            create_file_link_fixture_for_test, FileLinkFixtureOutcome,
        };

        let fixture = Fixture::new();
        write(
            &fixture.source.join("CommonModules/Main.xml"),
            &owner("Main", "CommonModule", "Main", ""),
        );
        let physical = fixture.source.join("physical-module.bsl");
        write(&physical, "");
        fs::create_dir_all(fixture.source.join("CommonModules/Main/Ext")).unwrap();
        let alias = fixture.source.join("CommonModules/Main/Ext/Module.bsl");
        match create_file_link_fixture_for_test(&physical, &alias).unwrap() {
            FileLinkFixtureOutcome::Created => {}
            FileLinkFixtureOutcome::Unsupported
            | FileLinkFixtureOutcome::WindowsPrivilegeUnavailable => return,
        }
        let root = RetainedDirectoryCapability::open(&fixture.source).unwrap();
        let failure = WorkspaceFindDirectoryBuilder::default()
            .build_for_path(
                &[LayoutFindSource::new(
                    "main",
                    SourceSetKind::Configuration,
                    &root,
                )],
                ProviderDeadline::from_budget(Duration::from_secs(7)),
                &CancellationToken::new(),
            )
            .unwrap_err();
        assert_eq!(failure.code(), RefusalCode::InvalidSource);
    }

    #[test]
    fn point_lookup_preserves_full_relative_paths_and_checks_every_source_before_not_found() {
        let fixture = Fixture::new();
        let empty = tempfile::tempdir().unwrap();
        let empty_root =
            RetainedDirectoryCapability::open(&empty.path().canonicalize().unwrap()).unwrap();
        let root = RetainedDirectoryCapability::open(&fixture.source).unwrap();
        let sources = [
            LayoutFindSource::new("empty", SourceSetKind::Configuration, &empty_root),
            LayoutFindSource::new("main", SourceSetKind::Configuration, &root),
        ];
        let builder = WorkspaceFindDirectoryBuilder::default();
        let locate = |path: &str| {
            builder.locate_path(
                &sources,
                path,
                ProviderDeadline::from_budget(Duration::from_secs(7)),
                &CancellationToken::new(),
            )
        };
        for (path, expected) in [
            ("Configuration.xml", "main:Configuration"),
            ("Catalogs/Валюты.xml", "main:Catalog.Валюты"),
            ("src/Catalogs/Валюты.xml", "main:Catalog.Валюты"),
            (
                "Catalogs/Валюты/Forms/ФормаЭлемента.xml",
                "main:Catalog.Валюты.Form.ФормаЭлемента",
            ),
            (
                "Catalogs/Валюты/Templates/Печать.xml",
                "main:Catalog.Валюты.Template.Печать",
            ),
            (
                "Catalogs/Валюты/Commands/Обновить",
                "main:Catalog.Валюты.Command.Обновить",
            ),
        ] {
            let entry = locate(path).unwrap().expect("unique suffix is placed");
            assert_eq!(entry.at(), expected, "{path}");
            assert!(entry.placed_path().is_some(), "{path}");
        }
        let absolute = fixture.source.join("Catalogs/Валюты.xml");
        assert_eq!(
            locate(absolute.to_str().unwrap()).unwrap().unwrap().at(),
            "main:Catalog.Валюты"
        );
        if std::path::MAIN_SEPARATOR == '\\' {
            let physical = absolute.to_string_lossy();
            let extended = if physical.starts_with(r"\\?\") {
                physical.into_owned()
            } else {
                format!(r"\\?\{physical}")
            };
            assert_eq!(
                locate(&extended).unwrap().unwrap().at(),
                "main:Catalog.Валюты"
            );
        }
        for path in [
            "алюты.xml",
            "Валюты.xml",
            "Forms/ФормаЭлемента.xml",
            "Catalogs/Нет.xml",
            "/workspace/src/catalogs/валюты.XML",
            r"C:\workspace\src\Catalogs\Валюты.xml",
        ] {
            assert!(locate(path).unwrap().is_none(), "{path}");
        }
        if std::path::MAIN_SEPARATOR == '/' {
            let non_layout = fixture.source.join(r"Catalogs\Валюты.xml");
            write(&non_layout, "unrelated physical file");
            write(&fixture.source.join("Catalogs/Валюты.xml"), "<broken");
            assert!(locate(non_layout.to_str().unwrap()).unwrap().is_none());
        }
    }

    #[test]
    fn point_lookup_common_module_file_requires_target_and_owner_without_full_directory() {
        let fixture = Fixture::new();
        let descriptor = fixture.source.join("CommonModules/Main.xml");
        let module = fixture.source.join("CommonModules/Main/Ext/Module.bsl");
        write(&descriptor, &owner("Main", "CommonModule", "Main", ""));
        let root = RetainedDirectoryCapability::open(&fixture.source).unwrap();
        let lookup = |query: &str| {
            WorkspaceFindDirectoryBuilder::with_fact_byte_limit_for_test(1).locate_path(
                &[LayoutFindSource::new(
                    "main",
                    SourceSetKind::Configuration,
                    &root,
                )],
                query,
                ProviderDeadline::from_budget(Duration::from_secs(7)),
                &CancellationToken::new(),
            )
        };
        let relative = "CommonModules/Main/Ext/Module.bsl";
        assert!(lookup(relative).unwrap().is_none(), "missing module file");
        write(&module, "// module\n");
        let placed = lookup(relative)
            .unwrap()
            .expect("module file and owner exist");
        assert_eq!(placed.at(), "main:CommonModule.Main");
        assert_eq!(placed.placed_path(), Some(relative));
        assert_eq!(
            lookup(module.to_str().unwrap()).unwrap().unwrap().at(),
            "main:CommonModule.Main"
        );
        assert!(lookup("other/CommonModules/Main/Ext/Module.bsl")
            .unwrap()
            .is_none());
        let outside = tempfile::tempdir().unwrap();
        assert!(lookup(outside.path().join(relative).to_str().unwrap())
            .unwrap()
            .is_none());

        write(&descriptor, "<broken");
        let failure = lookup(relative).expect_err("broken required owner must refuse");
        assert_eq!(failure.code(), RefusalCode::InvalidSource);
        fs::remove_file(&descriptor).unwrap();
        assert!(
            lookup(relative).unwrap().is_none(),
            "orphan module has no owner"
        );
        write(&descriptor, &owner("Main", "CommonModule", "Main", ""));
        fs::remove_file(&module).unwrap();
        fs::create_dir(&module).unwrap();
        let failure = lookup(relative).expect_err("non-file module must refuse");
        assert_eq!(failure.code(), RefusalCode::InvalidSource);
    }

    #[test]
    fn absolute_common_module_path_skips_a_broken_alias_in_another_source() {
        let first = Fixture::new();
        let second = Fixture::new();
        let relative = "CommonModules/Main/Ext/Module.bsl";
        for source in [&first.source, &second.source] {
            write(
                &source.join("CommonModules/Main.xml"),
                &owner("Main", "CommonModule", "Main", ""),
            );
        }
        write(&first.source.join(relative), "// valid\n");
        fs::create_dir_all(second.source.join(relative)).unwrap();
        let first_root = RetainedDirectoryCapability::open(&first.source).unwrap();
        let second_root = RetainedDirectoryCapability::open(&second.source).unwrap();
        let sources = [
            LayoutFindSource::new("main", SourceSetKind::Configuration, &first_root),
            LayoutFindSource::new("other", SourceSetKind::Extension, &second_root),
        ];
        let lookup = |path: &str| {
            WorkspaceFindDirectoryBuilder::default().locate_path(
                &sources,
                path,
                ProviderDeadline::from_budget(Duration::from_secs(7)),
                &CancellationToken::new(),
            )
        };
        let absolute = first.source.join(relative);
        let found = lookup(absolute.to_str().unwrap())
            .unwrap()
            .expect("the other source is outside this absolute path");
        assert_eq!(found.at(), "main:CommonModule.Main");
        assert_eq!(found.placed_path(), Some(relative));
        assert_eq!(
            lookup(relative)
                .expect_err("both sources can own a relative path")
                .code(),
            RefusalCode::InvalidSource
        );
    }

    #[test]
    fn absolute_lookup_keeps_distinct_source_roots_with_different_case() {
        use std::ffi::OsStr;

        let temp = tempfile::tempdir().unwrap();
        let workspace = temp.path().canonicalize().unwrap();
        let upper = workspace.join("src/A");
        let lower = workspace.join("src/a");
        fs::create_dir_all(&upper).unwrap();
        write(
            &upper.join("Catalogs/X.xml"),
            &owner("X", "Catalog", "X", ""),
        );
        match fs::create_dir(&lower) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                let root = RetainedDirectoryCapability::open(&upper).unwrap();
                let found = WorkspaceFindDirectoryBuilder::default()
                    .locate_path(
                        &[LayoutFindSource::new(
                            "main",
                            SourceSetKind::Configuration,
                            &root,
                        )],
                        lower.join("Catalogs/X.xml").to_str().unwrap(),
                        ProviderDeadline::from_budget(Duration::from_secs(7)),
                        &CancellationToken::new(),
                    )
                    .unwrap()
                    .expect("case-insensitive root spelling names the retained root");
                assert_eq!(found.at(), "main:Catalog.X");
                let moved = workspace.join("src/moved-A");
                fs::rename(&upper, &moved).unwrap();
                fs::write(&upper, "replaced root").unwrap();
                let error = WorkspaceFindDirectoryBuilder::default()
                    .locate_path(
                        &[LayoutFindSource::new(
                            "main",
                            SourceSetKind::Configuration,
                            &root,
                        )],
                        lower.join("Catalogs/X.xml").to_str().unwrap(),
                        ProviderDeadline::from_budget(Duration::from_secs(7)),
                        &CancellationToken::new(),
                    )
                    .expect_err("a replaced case-variant root must not look absent");
                assert_eq!(error.code(), RefusalCode::InvalidSource);
                return;
            }
            Err(error) => panic!("could not create case-distinct source root: {error}"),
        }
        write(&lower.join("Catalogs/X.xml"), "<broken");
        let upper_root = RetainedDirectoryCapability::open(&upper).unwrap();
        let lower_root = RetainedDirectoryCapability::open(&lower).unwrap();
        assert_ne!(upper_root.identity(), lower_root.identity());
        let lookup = |query: &str| {
            WorkspaceFindDirectoryBuilder::default().locate_path(
                &[
                    LayoutFindSource::new("main", SourceSetKind::Configuration, &upper_root),
                    LayoutFindSource::new("other", SourceSetKind::Configuration, &lower_root),
                ],
                query,
                ProviderDeadline::from_budget(Duration::from_secs(7)),
                &CancellationToken::new(),
            )
        };
        let found = lookup(upper.join("Catalogs/X.xml").to_str().unwrap())
            .unwrap()
            .expect("absolute path belongs only to the case-exact source");
        assert_eq!(found.at(), "main:Catalog.X");
        let collection = RetainedDirectoryCapability::open(&upper.join("Catalogs")).unwrap();
        if !collection
            .child_names_equivalent(OsStr::new("X.xml"), OsStr::new("x.xml"))
            .unwrap()
        {
            assert!(lookup(upper.join("Catalogs/x.xml").to_str().unwrap())
                .unwrap()
                .is_none());
            write(&upper.join("Catalogs/x.xml"), "<broken");
            let found = lookup(upper.join("Catalogs/X.xml").to_str().unwrap())
                .unwrap()
                .expect("case-distinct sibling must not block the requested target");
            assert_eq!(found.at(), "main:Catalog.X");
            assert_eq!(
                lookup(upper.join("Catalogs/x.xml").to_str().unwrap())
                    .expect_err("broken requested sibling must refuse")
                    .code(),
                RefusalCode::InvalidSource
            );
        }
    }

    #[test]
    fn absolute_lookup_accepts_a_physical_unicode_alias_of_the_source_root() {
        let temp = tempfile::tempdir().unwrap();
        let workspace = temp.path().canonicalize().unwrap();
        let source_parent = workspace.join("src");
        let source = source_parent.join("Й");
        let alias = source_parent.join("И\u{306}");
        write(
            &source.join("Catalogs/X.xml"),
            &owner("X", "Catalog", "X", ""),
        );
        write(
            &source.join("CommonModules/Main.xml"),
            &owner("Main", "CommonModule", "Main", ""),
        );
        write(
            &source.join("CommonModules/Main/Ext/Module.bsl"),
            "// module\n",
        );
        let parent = RetainedDirectoryCapability::open(&source_parent).unwrap();
        assert_ne!(
            super::normalize_layout_path(&source.to_string_lossy()),
            super::normalize_layout_path(&alias.to_string_lossy()),
            "the regression requires spellings that the string gate distinguishes"
        );
        if !parent
            .child_names_equivalent(std::ffi::OsStr::new("Й"), std::ffi::OsStr::new("И\u{306}"))
            .unwrap()
        {
            return;
        }
        assert!(alias.is_dir(), "the filesystem must resolve the alias");
        let retained = RetainedDirectoryCapability::open(&source).unwrap();
        let sources = [LayoutFindSource::new(
            "main",
            SourceSetKind::Configuration,
            &retained,
        )];
        let lookup = |path: &Path| {
            WorkspaceFindDirectoryBuilder::default().locate_path(
                &sources,
                path.to_str().unwrap(),
                ProviderDeadline::from_budget(Duration::from_secs(7)),
                &CancellationToken::new(),
            )
        };
        let catalog = lookup(&alias.join("Catalogs/X.xml"))
            .unwrap()
            .expect("physical Unicode alias of catalog source root");
        assert_eq!(catalog.at(), "main:Catalog.X");
        let module = lookup(&alias.join("CommonModules/Main/Ext/Module.bsl"))
            .unwrap()
            .expect("physical Unicode alias of module source root");
        assert_eq!(module.at(), "main:CommonModule.Main");
    }

    #[test]
    fn windows_drive_and_unc_prefixes_keep_the_same_root_anchor_shape() {
        if std::path::MAIN_SEPARATOR != '\\' {
            return;
        }
        for (ordinary, extended) in [
            (r"C:\workspace\src", r"\\?\C:\workspace\src"),
            (
                r"\\server\share\workspace\src",
                r"\\?\UNC\server\share\workspace\src",
            ),
        ] {
            let ordinary = Path::new(ordinary).ancestors().collect::<Vec<_>>();
            let extended = Path::new(extended).ancestors().collect::<Vec<_>>();
            assert_eq!(ordinary.len(), extended.len());
            for (ordinary, extended) in ordinary.into_iter().zip(extended) {
                assert_eq!(
                    super::normalize_layout_path(&ordinary.to_string_lossy()),
                    super::normalize_layout_path(&extended.to_string_lossy()),
                );
            }
        }
    }

    #[test]
    fn absolute_lookup_refuses_a_replaced_requested_root_ancestor() {
        let temp = tempfile::tempdir().unwrap();
        let workspace = temp.path().canonicalize().unwrap();
        let source = workspace.join("src");
        write(
            &source.join("Catalogs/X.xml"),
            &owner("X", "Catalog", "X", ""),
        );
        let retained = RetainedDirectoryCapability::open(&source).unwrap();
        let moved = workspace.join("moved-src");
        if let Err(error) = fs::rename(&source, &moved) {
            if error.kind() == io::ErrorKind::PermissionDenied {
                return;
            }
            panic!("could not replace retained source root: {error}");
        }
        fs::write(&source, "replaced source root").unwrap();

        let error = WorkspaceFindDirectoryBuilder::default()
            .locate_path(
                &[LayoutFindSource::new(
                    "main",
                    SourceSetKind::Configuration,
                    &retained,
                )],
                source.join("Catalogs/X.xml").to_str().unwrap(),
                ProviderDeadline::from_budget(Duration::from_secs(7)),
                &CancellationToken::new(),
            )
            .expect_err("a replaced requested source ancestor must not look absent");
        assert_eq!(error.code(), RefusalCode::InvalidSource);
    }

    #[test]
    fn absolute_lookup_skips_a_foreign_root_at_the_target_files_depth() {
        let temp = tempfile::tempdir().unwrap();
        let workspace = temp.path().canonicalize().unwrap();
        write(
            &workspace.join("Catalogs/X.xml"),
            &owner("X", "Catalog", "X", ""),
        );
        fs::create_dir_all(workspace.join("Other/Deep")).unwrap();
        let main = RetainedDirectoryCapability::open(&workspace).unwrap();
        let foreign = RetainedDirectoryCapability::open(&workspace.join("Other/Deep")).unwrap();
        let found = WorkspaceFindDirectoryBuilder::default()
            .locate_path(
                &[
                    LayoutFindSource::new("main", SourceSetKind::Configuration, &main),
                    LayoutFindSource::new("foreign", SourceSetKind::Extension, &foreign),
                ],
                workspace.join("Catalogs/X.xml").to_str().unwrap(),
                ProviderDeadline::from_budget(Duration::from_secs(7)),
                &CancellationToken::new(),
            )
            .unwrap()
            .expect("foreign root must not reopen the target XML as its own root");
        assert_eq!(found.at(), "main:Catalog.X");
    }

    #[test]
    fn absolute_lookup_distinguishes_a_file_from_a_case_folded_foreign_root() {
        let temp = tempfile::tempdir().unwrap();
        let workspace = temp.path().canonicalize().unwrap();
        write(
            &workspace.join("Catalogs/X.xml"),
            &owner("X", "Catalog", "X", ""),
        );
        let collection = RetainedDirectoryCapability::open(&workspace.join("Catalogs")).unwrap();
        if collection
            .child_names_equivalent(std::ffi::OsStr::new("X.xml"), std::ffi::OsStr::new("x.xml"))
            .unwrap()
        {
            return;
        }
        let foreign_path = workspace.join("Catalogs/x.xml");
        write(
            &foreign_path.join("Configuration.xml"),
            &owner("Foreign", "Configuration", "Foreign", ""),
        );
        let main = RetainedDirectoryCapability::open(&workspace).unwrap();
        let foreign = RetainedDirectoryCapability::open(&foreign_path).unwrap();
        let found = WorkspaceFindDirectoryBuilder::default()
            .locate_path(
                &[
                    LayoutFindSource::new("main", SourceSetKind::Configuration, &main),
                    LayoutFindSource::new("foreign", SourceSetKind::Extension, &foreign),
                ],
                workspace.join("Catalogs/X.xml").to_str().unwrap(),
                ProviderDeadline::from_budget(Duration::from_secs(7)),
                &CancellationToken::new(),
            )
            .unwrap()
            .expect("foreign directory must not block the requested file");
        assert_eq!(found.at(), "main:Catalog.X");
        let moved = workspace.join("moved-foreign");
        fs::rename(&foreign_path, &moved).unwrap();
        fs::write(&foreign_path, "replaced foreign root").unwrap();
        let still_found = WorkspaceFindDirectoryBuilder::default()
            .locate_path(
                &[
                    LayoutFindSource::new("main", SourceSetKind::Configuration, &main),
                    LayoutFindSource::new("foreign", SourceSetKind::Extension, &foreign),
                ],
                workspace.join("Catalogs/X.xml").to_str().unwrap(),
                ProviderDeadline::from_budget(Duration::from_secs(7)),
                &CancellationToken::new(),
            )
            .unwrap()
            .expect("damaged case-distinct foreign root must not block X.xml");
        assert_eq!(still_found.at(), "main:Catalog.X");
    }

    #[test]
    fn absolute_lookup_does_not_open_a_damaged_case_distinct_foreign_parent() {
        let temp = tempfile::tempdir().unwrap();
        let workspace = temp.path().canonicalize().unwrap();
        let upper = workspace.join("src/A");
        let lower = workspace.join("src/a");
        write(
            &lower.join("nested/Catalogs/X.xml"),
            &owner("X", "Catalog", "X", ""),
        );
        match fs::create_dir_all(upper.join("nested")) {
            Ok(()) => {}
            Err(error) => panic!("could not create foreign parent: {error}"),
        }
        let src = RetainedDirectoryCapability::open(&workspace.join("src")).unwrap();
        if src
            .child_names_equivalent(std::ffi::OsStr::new("A"), std::ffi::OsStr::new("a"))
            .unwrap()
        {
            return;
        }
        let main = RetainedDirectoryCapability::open(&lower.join("nested")).unwrap();
        let foreign = RetainedDirectoryCapability::open(&upper.join("nested")).unwrap();
        let moved = workspace.join("moved-A");
        if let Err(error) = fs::rename(&upper, &moved) {
            if error.kind() == io::ErrorKind::PermissionDenied {
                return;
            }
            panic!("could not replace foreign source parent: {error}");
        }
        fs::write(&upper, "replaced foreign parent").unwrap();

        let found = WorkspaceFindDirectoryBuilder::default()
            .locate_path(
                &[
                    LayoutFindSource::new("main", SourceSetKind::Configuration, &main),
                    LayoutFindSource::new("foreign", SourceSetKind::Extension, &foreign),
                ],
                lower.join("nested/Catalogs/X.xml").to_str().unwrap(),
                ProviderDeadline::from_budget(Duration::from_secs(7)),
                &CancellationToken::new(),
            )
            .unwrap()
            .expect("a damaged foreign parent must not block the requested source");
        assert_eq!(found.at(), "main:Catalog.X");
    }

    #[test]
    fn relative_source_prefix_skips_linked_collections_in_another_source() {
        use crate::infrastructure::platform::testing::{
            create_directory_link_fixture_for_test, FileLinkFixtureOutcome,
        };

        let temp = tempfile::tempdir().unwrap();
        let workspace = temp.path().canonicalize().unwrap();
        let extension = workspace.join("src/extension");
        write(
            &extension.join("Catalogs/Visible.xml"),
            &owner("Visible", "Catalog", "Visible", ""),
        );
        write(
            &extension.join("CommonModules/Main.xml"),
            &owner("Main", "CommonModule", "Main", ""),
        );
        write(
            &extension.join("CommonModules/Main/Ext/Module.bsl"),
            "// module\n",
        );
        for collection in ["Catalogs", "CommonModules"] {
            match create_directory_link_fixture_for_test(
                extension.join(collection),
                workspace.join(collection),
            )
            .unwrap()
            {
                FileLinkFixtureOutcome::Created => {}
                FileLinkFixtureOutcome::Unsupported
                | FileLinkFixtureOutcome::WindowsPrivilegeUnavailable => return,
            }
        }
        let main_root = RetainedDirectoryCapability::open(&workspace).unwrap();
        let extension_root = RetainedDirectoryCapability::open(&extension).unwrap();
        let sources = [
            LayoutFindSource::new("main", SourceSetKind::Configuration, &main_root),
            LayoutFindSource::new("extension", SourceSetKind::Extension, &extension_root),
        ];
        let locate = |query: &str| {
            WorkspaceFindDirectoryBuilder::default().locate_path(
                &sources,
                query,
                ProviderDeadline::from_budget(Duration::from_secs(7)),
                &CancellationToken::new(),
            )
        };
        assert_eq!(
            locate("src/extension/Catalogs/Visible.xml")
                .unwrap()
                .unwrap()
                .at(),
            "extension:Catalog.Visible"
        );
        assert_eq!(
            locate("src/extension/CommonModules/Main/Ext/Module.bsl")
                .unwrap()
                .unwrap()
                .at(),
            "extension:CommonModule.Main"
        );
        assert_eq!(
            locate("Catalogs/Visible.xml")
                .expect_err("without a source prefix the linked collection is a candidate")
                .code(),
            RefusalCode::InvalidSource
        );
    }

    #[test]
    fn point_lookup_refuses_ambiguous_paths_across_admitted_sources() {
        let first = Fixture::new();
        let second = Fixture::new();
        let first_root = RetainedDirectoryCapability::open(&first.source).unwrap();
        let second_root = RetainedDirectoryCapability::open(&second.source).unwrap();
        let error = WorkspaceFindDirectoryBuilder::default()
            .locate_path(
                &[
                    LayoutFindSource::new("main", SourceSetKind::Configuration, &first_root),
                    LayoutFindSource::new("other", SourceSetKind::Configuration, &second_root),
                ],
                "Catalogs/Валюты.xml",
                ProviderDeadline::from_budget(Duration::from_secs(7)),
                &CancellationToken::new(),
            )
            .expect_err("a suffix shared by two objects cannot select one");
        assert_eq!(error.code(), RefusalCode::BadValue);
    }

    #[test]
    fn point_lookup_uses_the_full_layout_path_even_with_a_shorter_suffix() {
        let fixture = Fixture::new();
        write(
            &fixture.source.join("Catalogs/Configuration.xml"),
            &owner("Configuration", "Catalog", "Configuration", ""),
        );
        write(&fixture.source.join("Configuration.xml"), "<broken");
        let root = RetainedDirectoryCapability::open(&fixture.source).unwrap();
        let entry = WorkspaceFindDirectoryBuilder::default()
            .locate_path(
                &[LayoutFindSource::new(
                    "main",
                    SourceSetKind::Configuration,
                    &root,
                )],
                "src/Catalogs/Configuration.xml",
                ProviderDeadline::from_budget(Duration::from_secs(7)),
                &CancellationToken::new(),
            )
            .unwrap()
            .expect("the requested catalog descriptor exists");
        assert_eq!(entry.at(), "main:Catalog.Configuration");

        write(
            &fixture.source.join("Configuration.xml"),
            &owner("Магазин", "Configuration", "Магазин", ""),
        );
        fs::remove_file(fixture.source.join("Catalogs/Configuration.xml")).unwrap();
        let missing = WorkspaceFindDirectoryBuilder::default()
            .locate_path(
                &[LayoutFindSource::new(
                    "main",
                    SourceSetKind::Configuration,
                    &root,
                )],
                "src/Catalogs/Configuration.xml",
                ProviderDeadline::from_budget(Duration::from_secs(7)),
                &CancellationToken::new(),
            )
            .unwrap();
        assert!(missing.is_none(), "the requested catalog file is absent");
    }

    #[test]
    fn point_lookup_checks_the_owner_of_a_nested_object() {
        let fixture = Fixture::new();
        let root = RetainedDirectoryCapability::open(&fixture.source).unwrap();
        let path = "Catalogs/Валюты/Forms/ФормаЭлемента.xml";
        let lookup = |builder: WorkspaceFindDirectoryBuilder| {
            builder.locate_path(
                &[LayoutFindSource::new(
                    "main",
                    SourceSetKind::Configuration,
                    &root,
                )],
                path,
                ProviderDeadline::from_budget(Duration::from_secs(7)),
                &CancellationToken::new(),
            )
        };
        let entry = lookup(WorkspaceFindDirectoryBuilder::default())
            .unwrap()
            .expect("the form and its owner are proven");
        assert_eq!(entry.at(), "main:Catalog.Валюты.Form.ФормаЭлемента");

        let error = lookup(
            WorkspaceFindDirectoryBuilder::default()
                .with_local_read_fault_for_test("Catalogs/Валюты.xml", io::ErrorKind::Other),
        )
        .expect_err("an unreadable owner must block the nested path");
        assert_eq!(
            error.detail(),
            Some(crate::domain::refusal::RefusalDetail::SourceUnreadable)
        );
    }

    #[test]
    fn point_lookup_does_not_publish_a_command_xml_lookalike() {
        let fixture = Fixture::new();
        write(
            &fixture.source.join("Catalogs/Валюты/Commands/Ложная.xml"),
            &owner("Ложная", "Command", "Ложная", ""),
        );
        let root = RetainedDirectoryCapability::open(&fixture.source).unwrap();
        let builder = WorkspaceFindDirectoryBuilder::default();
        let source = [LayoutFindSource::new(
            "main",
            SourceSetKind::Configuration,
            &root,
        )];
        assert!(builder
            .locate_path(
                &source,
                "Catalogs/Валюты/Commands/Ложная.xml",
                ProviderDeadline::from_budget(Duration::from_secs(7)),
                &CancellationToken::new(),
            )
            .unwrap()
            .is_none());
        assert_eq!(
            builder
                .locate_path(
                    &source,
                    "Catalogs/Валюты/Commands/Обновить",
                    ProviderDeadline::from_budget(Duration::from_secs(7)),
                    &CancellationToken::new(),
                )
                .unwrap()
                .unwrap()
                .at(),
            "main:Catalog.Валюты.Command.Обновить"
        );
    }

    #[test]
    fn point_lookup_refuses_a_linked_target_descriptor() {
        use crate::infrastructure::platform::testing::{
            create_file_link_fixture_for_test, FileLinkFixtureOutcome,
        };

        let fixture = Fixture::new();
        let descriptor = fixture.source.join("Catalogs/Валюты.xml");
        let physical = fixture.source.join("physical-owner.xml");
        fs::rename(&descriptor, &physical).unwrap();
        match create_file_link_fixture_for_test(&physical, &descriptor).unwrap() {
            FileLinkFixtureOutcome::Created => {}
            FileLinkFixtureOutcome::Unsupported
            | FileLinkFixtureOutcome::WindowsPrivilegeUnavailable => return,
        }
        let root = RetainedDirectoryCapability::open(&fixture.source).unwrap();
        let error = WorkspaceFindDirectoryBuilder::default()
            .locate_path(
                &[LayoutFindSource::new(
                    "main",
                    SourceSetKind::Configuration,
                    &root,
                )],
                "Catalogs/Валюты.xml",
                ProviderDeadline::from_budget(Duration::from_secs(7)),
                &CancellationToken::new(),
            )
            .expect_err("a linked target cannot stand for the admitted source file");
        assert_eq!(error.code(), RefusalCode::InvalidSource);
    }

    #[test]
    fn point_lookup_refuses_an_existing_broken_target_descriptor() {
        let fixture = Fixture::new();
        write(
            &fixture.source.join("Catalogs/Валюты.xml"),
            "<MetaDataObject><Catalog><Properties></Properties></Catalog></MetaDataObject>",
        );
        let root = RetainedDirectoryCapability::open(&fixture.source).unwrap();
        let error = WorkspaceFindDirectoryBuilder::default()
            .locate_path(
                &[LayoutFindSource::new(
                    "main",
                    SourceSetKind::Configuration,
                    &root,
                )],
                "Catalogs/Валюты.xml",
                ProviderDeadline::from_budget(Duration::from_secs(7)),
                &CancellationToken::new(),
            )
            .expect_err("a broken requested descriptor must not look absent");
        assert_eq!(error.code(), RefusalCode::InvalidSource);
    }

    #[test]
    fn point_lookup_refuses_an_existing_broken_external_target_descriptor() {
        let fixture = tempfile::tempdir().unwrap();
        let path = fixture.path().canonicalize().unwrap();
        write(
            &path.join("Импорт.xml"),
            "<MetaDataObject><ExternalReport><Properties><Name>Импорт</Name></Properties></ExternalReport></MetaDataObject>",
        );
        let root = RetainedDirectoryCapability::open(&path).unwrap();
        let error = WorkspaceFindDirectoryBuilder::default()
            .locate_path(
                &[LayoutFindSource::new(
                    "processor",
                    SourceSetKind::ExternalProcessor,
                    &root,
                )],
                "Импорт.xml",
                ProviderDeadline::from_budget(Duration::from_secs(7)),
                &CancellationToken::new(),
            )
            .expect_err("the existing external target must declare its actual kind");
        assert_eq!(error.code(), RefusalCode::InvalidSource);
    }

    #[test]
    fn point_lookup_observes_cancellation_and_deadline() {
        let fixture = Fixture::new();
        let root = RetainedDirectoryCapability::open(&fixture.source).unwrap();
        let source = [LayoutFindSource::new(
            "main",
            SourceSetKind::Configuration,
            &root,
        )];
        let cancelled = CancellationToken::new();
        cancelled.cancel();
        let builder = WorkspaceFindDirectoryBuilder::default();
        let error = builder
            .locate_path(
                &source,
                "Catalogs/Валюты.xml",
                ProviderDeadline::from_budget(Duration::from_secs(7)),
                &cancelled,
            )
            .unwrap_err();
        assert_eq!(error.code(), RefusalCode::Cancelled);
        let error = builder
            .locate_path(
                &source,
                "Catalogs/Валюты.xml",
                ProviderDeadline::from_budget(Duration::ZERO),
                &CancellationToken::new(),
            )
            .unwrap_err();
        assert_eq!(error.code(), RefusalCode::DeadlineExceeded);
    }

    #[test]
    fn point_lookup_refuses_when_target_name_is_outside_the_descriptor_sample() {
        let fixture = Fixture::new();
        for path in ["Catalogs/Валюты.xml", "Configuration.xml"] {
            let descriptor = fixture.source.join(path);
            let original = fs::read_to_string(&descriptor).unwrap();
            let late = original.replace(
                "<Properties><Name>",
                &format!(
                    "<Properties><!-- {} --><Name>",
                    " ".repeat(DESCRIPTOR_HEAD_BYTES)
                ),
            );
            write(&descriptor, &late);
        }
        let root = RetainedDirectoryCapability::open(&fixture.source).unwrap();
        for path in ["Catalogs/Валюты.xml", "Configuration.xml"] {
            let error = WorkspaceFindDirectoryBuilder::default()
                .locate_path(
                    &[LayoutFindSource::new(
                        "main",
                        SourceSetKind::Configuration,
                        &root,
                    )],
                    path,
                    ProviderDeadline::from_budget(Duration::from_secs(7)),
                    &CancellationToken::new(),
                )
                .expect_err("an unproven existing target cannot look absent");
            assert_eq!(error.code(), RefusalCode::InvalidSource, "{path}");
        }
    }

    #[test]
    fn point_lookup_ignores_the_search_collection_entry_limit() {
        let fixture = Fixture::new();
        fs::write(fixture.source.join("Catalogs/unrelated.xml"), []).unwrap();
        let root = RetainedDirectoryCapability::open(&fixture.source).unwrap();
        let source = [LayoutFindSource::new(
            "main",
            SourceSetKind::Configuration,
            &root,
        )];
        let builder = WorkspaceFindDirectoryBuilder::default().with_collection_limit_for_test(2);
        let error = builder
            .build(
                &source,
                ProviderDeadline::from_budget(Duration::from_secs(7)),
                &CancellationToken::new(),
            )
            .expect_err("the complete search directory has a separate collection limit");
        assert_eq!(error.code(), RefusalCode::ProviderLimitExceeded);
        let entry = builder
            .locate_path(
                &source,
                "Catalogs/Валюты.xml",
                ProviderDeadline::from_budget(Duration::from_secs(7)),
                &CancellationToken::new(),
            )
            .unwrap()
            .expect("the target does not inherit the search collection limit");
        assert_eq!(entry.at(), "main:Catalog.Валюты");
    }

    #[test]
    fn large_configuration_descriptor_still_has_a_layout_address() {
        let fixture = Fixture::new();
        let descriptor = fixture.source.join("Configuration.xml");
        let mut contents = std::fs::read(&descriptor).unwrap();
        contents.extend(vec![b' '; DESCRIPTOR_HEAD_BYTES + 1]);
        std::fs::write(&descriptor, contents).unwrap();

        let index = fixture.directory();
        let entry = index
            .locate_address("main:Configuration")
            .expect("the beginning of a large descriptor still places the root");
        assert_eq!(entry.placed_path(), Some("Configuration.xml"));
    }

    #[test]
    fn linked_configuration_descriptor_refuses_the_layout_build() {
        use crate::infrastructure::platform::testing::{
            create_file_link_fixture_for_test, FileLinkFixtureOutcome,
        };

        let fixture = Fixture::new();
        let descriptor = fixture.source.join("Configuration.xml");
        let physical = fixture.source.join("physical-configuration.xml");
        std::fs::rename(&descriptor, &physical).unwrap();
        match create_file_link_fixture_for_test(&physical, &descriptor).unwrap() {
            FileLinkFixtureOutcome::Created => {}
            FileLinkFixtureOutcome::Unsupported
            | FileLinkFixtureOutcome::WindowsPrivilegeUnavailable => return,
        }

        let root = RetainedDirectoryCapability::open(&fixture.source).unwrap();
        let refusal = WorkspaceFindDirectoryBuilder::default()
            .build_for_search(
                &[LayoutFindSource::new(
                    "main",
                    SourceSetKind::Configuration,
                    &root,
                )],
                ProviderDeadline::from_budget(Duration::from_secs(7)),
                &CancellationToken::new(),
            )
            .expect_err("a linked root descriptor must not look absent");
        assert_eq!(refusal.code(), RefusalCode::InvalidSource);
    }

    #[test]
    fn point_lookup_nested_path_ignores_an_unrelated_linked_owner_directory() {
        use crate::infrastructure::platform::testing::{
            create_directory_link_fixture_for_test, FileLinkFixtureOutcome,
        };

        let fixture = Fixture::new();
        write(
            &fixture.source.join("Catalogs/Скрытый.xml"),
            &owner("Скрытый", "Catalog", "Скрытый", ""),
        );
        let physical = fixture.source.join("physical-hidden");
        fs::create_dir_all(&physical).unwrap();
        match create_directory_link_fixture_for_test(
            &physical,
            fixture.source.join("Catalogs/Скрытый"),
        )
        .unwrap()
        {
            FileLinkFixtureOutcome::Created => {}
            FileLinkFixtureOutcome::Unsupported
            | FileLinkFixtureOutcome::WindowsPrivilegeUnavailable => return,
        }

        let root = RetainedDirectoryCapability::open(&fixture.source).unwrap();
        let found = WorkspaceFindDirectoryBuilder::default()
            .locate_path(
                &[LayoutFindSource::new(
                    "main",
                    SourceSetKind::Configuration,
                    &root,
                )],
                "Catalogs/Валюты/Forms/ФормаЭлемента.xml",
                ProviderDeadline::from_budget(Duration::from_secs(7)),
                &CancellationToken::new(),
            )
            .unwrap()
            .expect("the unrelated owner is outside the nested path");
        assert_eq!(found.at(), "main:Catalog.Валюты.Form.ФормаЭлемента");
    }

    #[test]
    fn linked_nested_descriptor_refuses_instead_of_looking_complete() {
        use crate::infrastructure::platform::testing::{
            create_file_link_fixture_for_test, FileLinkFixtureOutcome,
        };

        let fixture = Fixture::new();
        let descriptor = fixture
            .source
            .join("Catalogs/Валюты/Forms/ФормаЭлемента.xml");
        let physical = fixture.source.join("physical-form.xml");
        fs::rename(&descriptor, &physical).unwrap();
        match create_file_link_fixture_for_test(&physical, &descriptor).unwrap() {
            FileLinkFixtureOutcome::Created => {}
            FileLinkFixtureOutcome::Unsupported
            | FileLinkFixtureOutcome::WindowsPrivilegeUnavailable => return,
        }
        let root = RetainedDirectoryCapability::open(&fixture.source).unwrap();
        let refusal = WorkspaceFindDirectoryBuilder::default()
            .build_for_search(
                &[LayoutFindSource::new(
                    "main",
                    SourceSetKind::Configuration,
                    &root,
                )],
                ProviderDeadline::from_budget(Duration::from_secs(7)),
                &CancellationToken::new(),
            )
            .expect_err("linked nested descriptor must not become a local omission");
        assert_eq!(refusal.code(), RefusalCode::InvalidSource);
    }

    #[test]
    fn linked_proven_owner_directory_refuses_instead_of_hiding_nested_names() {
        use crate::infrastructure::platform::testing::{
            create_directory_link_fixture_for_test, FileLinkFixtureOutcome,
        };

        let fixture = Fixture::new();
        let owner_root = fixture.source.join("Catalogs/Валюты");
        let physical = fixture.source.join("physical-owner");
        fs::rename(&owner_root, &physical).unwrap();
        match create_directory_link_fixture_for_test(&physical, &owner_root).unwrap() {
            FileLinkFixtureOutcome::Created => {}
            FileLinkFixtureOutcome::Unsupported
            | FileLinkFixtureOutcome::WindowsPrivilegeUnavailable => return,
        }

        let root = RetainedDirectoryCapability::open(&fixture.source).unwrap();
        let refusal = WorkspaceFindDirectoryBuilder::default()
            .build_for_search(
                &[LayoutFindSource::new(
                    "main",
                    SourceSetKind::Configuration,
                    &root,
                )],
                ProviderDeadline::from_budget(Duration::from_secs(7)),
                &CancellationToken::new(),
            )
            .expect_err("a linked proven owner must not hide its nested names");
        assert_eq!(refusal.code(), RefusalCode::InvalidSource);
        let refusal = WorkspaceFindDirectoryBuilder::default()
            .locate_path(
                &[LayoutFindSource::new(
                    "main",
                    SourceSetKind::Configuration,
                    &root,
                )],
                "Catalogs/Валюты/Forms/ФормаЭлемента.xml",
                ProviderDeadline::from_budget(Duration::from_secs(7)),
                &CancellationToken::new(),
            )
            .expect_err("a linked required owner cannot place the form");
        assert_eq!(refusal.code(), RefusalCode::InvalidSource);
    }

    #[test]
    fn linked_command_directory_refuses_instead_of_looking_complete() {
        use crate::infrastructure::platform::testing::{
            create_directory_link_fixture_for_test, FileLinkFixtureOutcome,
        };

        let fixture = Fixture::new();
        let command = fixture.source.join("Catalogs/Валюты/Commands/Обновить");
        let physical = fixture.source.join("physical-command");
        fs::rename(&command, &physical).unwrap();
        match create_directory_link_fixture_for_test(&physical, &command).unwrap() {
            FileLinkFixtureOutcome::Created => {}
            FileLinkFixtureOutcome::Unsupported
            | FileLinkFixtureOutcome::WindowsPrivilegeUnavailable => return,
        }

        let root = RetainedDirectoryCapability::open(&fixture.source).unwrap();
        let refusal = WorkspaceFindDirectoryBuilder::default()
            .build_for_search(
                &[LayoutFindSource::new(
                    "main",
                    SourceSetKind::Configuration,
                    &root,
                )],
                ProviderDeadline::from_budget(Duration::from_secs(7)),
                &CancellationToken::new(),
            )
            .expect_err("a linked command must not look absent");
        assert_eq!(refusal.code(), RefusalCode::InvalidSource);
    }

    #[test]
    fn a_file_path_resolves_back_to_its_object_address() {
        let index = Fixture::new().directory();
        for (query, at) in [
            ("Catalogs/Валюты.xml", "main:Catalog.Валюты"),
            (
                "Catalogs/Валюты/Forms/ФормаЭлемента.xml",
                "main:Catalog.Валюты.Form.ФормаЭлемента",
            ),
            // The caller pastes what the shell gave them: the stored path is
            // the tail of an absolute or workspace-relative path.
            (
                "/home/user/project/src/Catalogs/Валюты.xml",
                "main:Catalog.Валюты",
            ),
            ("src/Catalogs/Валюты.xml", "main:Catalog.Валюты"),
        ] {
            assert_eq!(single(&index, query).0, at, "{query}");
        }
    }

    #[test]
    fn a_synonym_resolves_to_its_object() {
        let index = Fixture::new().directory();
        assert_eq!(single(&index, "Валюты и курсы").0, "main:Catalog.Валюты");
    }

    #[test]
    fn unreadable_descriptor_makes_name_search_partial_but_resolve_refuses() {
        let fixture = Fixture::new();
        let root = RetainedDirectoryCapability::open(&fixture.source).unwrap();
        let reader = Arc::new(
            |file: &crate::infrastructure::platform::filesystem::RetainedRegularFileCapability,
             relative: &Path| {
                if relative == Path::new("Catalogs/Валюты.xml") {
                    Err(super::DescriptorHeadReadError::Local(
                        io::ErrorKind::PermissionDenied,
                    ))
                } else {
                    super::read_descriptor_head_prefix(file, relative)
                }
            },
        );
        let builder = WorkspaceFindDirectoryBuilder::default().with_head_reader(reader);
        let sources = [LayoutFindSource::new(
            "main",
            SourceSetKind::Configuration,
            &root,
        )];
        let result = builder
            .build_for_search(
                &sources,
                ProviderDeadline::from_budget(Duration::from_secs(7)),
                &CancellationToken::new(),
            )
            .expect("a local read refusal keeps proven documents");
        assert_eq!(result.omissions.total, 1);
        assert_eq!(result.omissions.details[0].source_set, "main");
        assert_eq!(result.omissions.details[0].reason, "descriptor_unreadable");
        assert_eq!(single(&result.index, "Магазин").0, "main:Configuration");
        assert!(result.index.locate_address("main:Catalog.Валюты").is_none());
        assert!(result
            .index
            .locate_address("main:Catalog.Валюты.Form.ФормаЭлемента")
            .is_none());
        assert!(result
            .index
            .locate_address("main:Catalog.Валюты.Command.Обновить")
            .is_none());
        assert!(result
            .index
            .find(FindRequest::new("Валюты").unwrap())
            .candidates()
            .iter()
            .all(|candidate| candidate.at() != "main:Catalog.Валюты"));

        let refusal = builder
            .build(
                &sources,
                ProviderDeadline::from_budget(Duration::from_secs(7)),
                &CancellationToken::new(),
            )
            .expect_err("resolve must not use an incomplete directory");
        assert_eq!(refusal.code(), RefusalCode::ProviderUnavailable);
    }

    #[test]
    fn a_local_io_error_is_partial_but_handle_failure_is_a_refusal() {
        let fixture = Fixture::new();
        let root = RetainedDirectoryCapability::open(&fixture.source).unwrap();
        let sources = [LayoutFindSource::new(
            "main",
            SourceSetKind::Configuration,
            &root,
        )];
        let build_with = |fault| {
            let reader = Arc::new(
                move |file: &crate::infrastructure::platform::filesystem::RetainedRegularFileCapability,
                      relative: &Path| {
                    if relative == Path::new("Catalogs/Валюты.xml") {
                        Err(fault)
                    } else {
                        super::read_descriptor_head_prefix(file, relative)
                    }
                },
            );
            WorkspaceFindDirectoryBuilder::default()
                .with_head_reader(reader)
                .build_for_search(
                    &sources,
                    ProviderDeadline::from_budget(Duration::from_secs(7)),
                    &CancellationToken::new(),
                )
        };
        let partial = build_with(super::DescriptorHeadReadError::Local(io::ErrorKind::Other))
            .expect("an I/O error on one retained descriptor is local");
        assert_eq!(partial.omissions.total, 1);
        assert_eq!(single(&partial.index, "Магазин").0, "main:Configuration");

        let refusal = build_with(super::DescriptorHeadReadError::Capability)
            .expect_err("failure to retain the read handle is not local proof");
        assert_eq!(refusal.code(), RefusalCode::ProviderUnavailable);
    }

    #[test]
    fn cancellation_during_a_failed_descriptor_read_refuses_instead_of_returning_partial() {
        let fixture = Fixture::new();
        let root = RetainedDirectoryCapability::open(&fixture.source).unwrap();
        let cancellation = CancellationToken::new();
        let cancel_during_read = cancellation.clone();
        let reader = Arc::new(
            move |file: &crate::infrastructure::platform::filesystem::RetainedRegularFileCapability,
                  relative: &Path| {
                if relative == Path::new("Catalogs/Валюты.xml") {
                    cancel_during_read.cancel();
                    Err(super::DescriptorHeadReadError::Local(
                        io::ErrorKind::PermissionDenied,
                    ))
                } else {
                    super::read_descriptor_head_prefix(file, relative)
                }
            },
        );
        let refusal = WorkspaceFindDirectoryBuilder::default()
            .with_head_reader(reader)
            .build_for_search(
                &[LayoutFindSource::new(
                    "main",
                    SourceSetKind::Configuration,
                    &root,
                )],
                ProviderDeadline::from_budget(Duration::from_secs(7)),
                &cancellation,
            )
            .expect_err("cancellation must take precedence over a local read omission");
        assert_eq!(refusal.code(), RefusalCode::Cancelled);
    }

    #[test]
    fn omission_diagnostics_are_bounded_without_losing_the_total() {
        let mut build = super::DirectoryBuild {
            documents: Vec::new(),
            fact_bytes: 0,
            attempted_entries: 0,
            attempted_fact_bytes: 0,
            omissions: super::FindOmissions::default(),
            include_module_aliases: false,
        };
        for _ in 0..=super::MAX_OMISSION_DETAILS {
            build.record_omission("main", "descriptor_unreadable");
        }
        assert_eq!(build.omissions.total, super::MAX_OMISSION_DETAILS + 1);
        assert_eq!(build.omissions.details.len(), super::MAX_OMISSION_DETAILS);
    }

    #[test]
    fn find_observation_reports_attempted_entry_as_censored_on_capacity_refusal() {
        let fixture = Fixture::new();
        let root = RetainedDirectoryCapability::open(&fixture.source).unwrap();
        let sources = [LayoutFindSource::new(
            "main",
            SourceSetKind::Configuration,
            &root,
        )];
        let observer = Arc::new(CapacityObserver::default());
        let capped = WorkspaceFindDirectoryBuilder::with_document_limit(0)
            .with_capacity_observer(Arc::clone(&observer));
        let refusal = capped
            .build_for_search(
                &sources,
                ProviderDeadline::from_budget(Duration::from_secs(7)),
                &CancellationToken::new(),
            )
            .unwrap_err();
        assert_eq!(refusal.code(), RefusalCode::ProviderLimitExceeded);
        let complete =
            WorkspaceFindDirectoryBuilder::default().with_capacity_observer(Arc::clone(&observer));
        complete
            .build_for_search(
                &sources,
                ProviderDeadline::from_budget(Duration::from_secs(7)),
                &CancellationToken::new(),
            )
            .unwrap();
        let snapshot = observer.snapshot();
        assert_eq!(snapshot.find_entries.max_lower_bound, 1);
        assert_eq!(snapshot.find_entries.lower_bound_count, 1);
        assert_eq!(snapshot.find_entries.exact_count, 1);
        assert!(snapshot.find_identity_estimate_bytes.max_lower_bound > 0);
        assert!(snapshot.find_collection_entries.exact_count > 0);
    }

    #[test]
    fn many_owner_directories_do_not_exhaust_open_file_handles() {
        let fixture = Fixture::new();
        for index in 0..400 {
            let name = format!("Item{index:03}");
            write(
                &fixture.source.join(format!("Catalogs/{name}.xml")),
                &owner(&name, "Catalog", &name, ""),
            );
            fs::create_dir_all(fixture.source.join(format!("Catalogs/{name}/Forms"))).unwrap();
        }
        let root = RetainedDirectoryCapability::open(&fixture.source).unwrap();
        let built = WorkspaceFindDirectoryBuilder::default()
            .build_for_search(
                &[LayoutFindSource::new(
                    "main",
                    SourceSetKind::Configuration,
                    &root,
                )],
                ProviderDeadline::from_budget(Duration::from_secs(20)),
                &CancellationToken::new(),
            )
            .expect("each owner directory should close before the next opens");
        assert_eq!(built.omissions.total, 0);
        assert_eq!(single(&built.index, "Item399").0, "main:Catalog.Item399");
    }

    #[test]
    fn descriptor_identity_drift_remains_a_refusal() {
        let fixture = Fixture::new();
        let root = RetainedDirectoryCapability::open(&fixture.source).unwrap();
        let descriptor = fixture.source.join("Catalogs/Валюты.xml");
        let reader = Arc::new(
            move |file: &crate::infrastructure::platform::filesystem::RetainedRegularFileCapability,
                  relative: &Path| {
                if relative == Path::new("Catalogs/Валюты.xml") {
                    fs::rename(&descriptor, descriptor.with_extension("old")).unwrap();
                    fs::write(&descriptor, "replacement").unwrap();
                }
                super::read_descriptor_head_prefix(file, relative)
            },
        );
        let refusal = WorkspaceFindDirectoryBuilder::default()
            .with_head_reader(reader)
            .build_for_search(
                &[LayoutFindSource::new(
                    "main",
                    SourceSetKind::Configuration,
                    &root,
                )],
                ProviderDeadline::from_budget(Duration::from_secs(7)),
                &CancellationToken::new(),
            )
            .expect_err("replacement after retention must refuse the build");
        assert_eq!(refusal.code(), RefusalCode::InvalidSource);
    }

    #[test]
    fn the_directory_holds_objects_and_never_code_symbols_or_inner_nodes() {
        let index = Fixture::new().directory();
        for query in [
            "СекретныйМетод",
            "main:Catalog.Валюты.Module.Object",
            "main:Catalog.Валюты.Attribute.Код",
        ] {
            let found = index.find(FindRequest::new(query).unwrap());
            assert!(
                found.is_nearest() || found.candidates().is_empty(),
                "the directory answered a non-object query {query}: {found:?}"
            );
        }
    }

    #[test]
    fn the_directory_refuses_to_exceed_resource_bounds() {
        let fixture = Fixture::new();
        let root = RetainedDirectoryCapability::open(&fixture.source).unwrap();
        let build = |builder: &WorkspaceFindDirectoryBuilder, count: usize| {
            let names = (0..count)
                .map(|index| format!("source{index}"))
                .collect::<Vec<_>>();
            let sources = names
                .iter()
                .map(|name| LayoutFindSource::new(name, SourceSetKind::Configuration, &root))
                .collect::<Vec<_>>();
            builder.build(
                &sources,
                ProviderDeadline::from_budget(Duration::from_secs(7)),
                &CancellationToken::new(),
            )
        };
        let index = build(
            &WorkspaceFindDirectoryBuilder::default(),
            super::MAX_SOURCE_SETS,
        )
        .expect("the source-set limit itself must remain usable");
        let found = index.find(FindRequest::new("source0:Catalog.Валюты").unwrap());
        assert!(!found.is_nearest());
        assert!(found
            .candidates()
            .iter()
            .any(|candidate| candidate.at() == "source0:Catalog.Валюты"));

        for (label, builder, count) in [
            (
                "source sets",
                WorkspaceFindDirectoryBuilder::default(),
                super::MAX_SOURCE_SETS + 1,
            ),
            (
                "entries",
                WorkspaceFindDirectoryBuilder::with_document_limit(1),
                1,
            ),
            (
                "fact bytes",
                WorkspaceFindDirectoryBuilder::with_limits(super::DEFAULT_MAX_DOCUMENTS, 1),
                1,
            ),
        ] {
            let error = build(&builder, count).expect_err(label);
            assert_eq!(error.code().as_str(), "provider_limit_exceeded", "{label}");
        }
    }

    #[test]
    fn the_directory_observes_cancellation() {
        let fixture = Fixture::new();
        let root = RetainedDirectoryCapability::open(&fixture.source).unwrap();
        let cancellation = CancellationToken::new();
        cancellation.cancel();
        let error = WorkspaceFindDirectoryBuilder::default()
            .build(
                &[LayoutFindSource::new(
                    "main",
                    SourceSetKind::Configuration,
                    &root,
                )],
                ProviderDeadline::from_budget(Duration::from_secs(7)),
                &cancellation,
            )
            .unwrap_err();
        assert_eq!(error.code().as_str(), "cancelled");
    }

    #[test]
    fn an_external_root_publishes_its_owner_and_never_the_dump_sidecar() {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().canonicalize().unwrap().join("processor");
        write(
            &source.join("Импорт.xml"),
            &owner(
                "Импорт",
                "ExternalDataProcessor",
                "Импорт данных",
                "<Form>Основная</Form>",
            ),
        );
        write(
            &source.join("Импорт/Forms/Основная.xml"),
            &owner("Основная", "Form", "Основная форма", ""),
        );
        // A Designer dump keeps this sidecar beside the owner descriptor.
        write(
            &source.join("ConfigDumpInfo.xml"),
            r#"<?xml version="1.0" encoding="UTF-8"?><ConfigDumpInfo xmlns="http://v8.1c.ru/8.3/xcf/dumpinfo" format="Hierarchical" version="2.20"/>"#,
        );
        let retained = RetainedDirectoryCapability::open(&source).unwrap();
        let index = WorkspaceFindDirectoryBuilder::default()
            .build(
                &[LayoutFindSource::new(
                    "processor",
                    SourceSetKind::ExternalProcessor,
                    &retained,
                )],
                ProviderDeadline::from_budget(Duration::from_secs(7)),
                &CancellationToken::new(),
            )
            .unwrap();
        assert_eq!(
            single(&index, "Импорт"),
            (
                "processor:ExternalDataProcessor.Импорт".to_string(),
                "Импорт.xml".to_string()
            )
        );
        assert_eq!(
            single(&index, "Основная").0,
            "processor:ExternalDataProcessor.Импорт.Form.Основная"
        );
        for query in ["ConfigDumpInfo", "ConfigDumpInfo.xml"] {
            let found = index.find(FindRequest::new(query).unwrap());
            assert!(
                found.is_nearest() || found.candidates().is_empty(),
                "the dump sidecar became an object: {found:?}"
            );
        }
        // An external source set has no configuration root, so nothing may
        // answer its export path.
        let fabricated = index.find(FindRequest::new("Configuration.xml").unwrap());
        assert!(
            fabricated
                .candidates()
                .iter()
                .all(|candidate| candidate.reason() != "exportPath"),
            "an external source set advertised a configuration export path: {fabricated:?}"
        );
        let builder = WorkspaceFindDirectoryBuilder::default();
        let source = [LayoutFindSource::new(
            "processor",
            SourceSetKind::ExternalProcessor,
            &retained,
        )];
        for (path, expected) in [
            ("Импорт.xml", "processor:ExternalDataProcessor.Импорт"),
            (
                "Импорт/Forms/Основная.xml",
                "processor:ExternalDataProcessor.Импорт.Form.Основная",
            ),
        ] {
            let entry = builder
                .locate_path(
                    &source,
                    path,
                    ProviderDeadline::from_budget(Duration::from_secs(7)),
                    &CancellationToken::new(),
                )
                .unwrap()
                .expect("the external owner or form is placed");
            assert_eq!(entry.at(), expected);
        }
        assert!(builder
            .locate_path(
                &source,
                "ConfigDumpInfo.xml",
                ProviderDeadline::from_budget(Duration::from_secs(7)),
                &CancellationToken::new(),
            )
            .unwrap()
            .is_none());
    }

    #[test]
    fn the_directory_observes_its_operation_deadline() {
        let fixture = Fixture::new();
        let root = RetainedDirectoryCapability::open(&fixture.source).unwrap();
        let error = WorkspaceFindDirectoryBuilder::default()
            .build(
                &[LayoutFindSource::new(
                    "main",
                    SourceSetKind::Configuration,
                    &root,
                )],
                ProviderDeadline::from_budget(Duration::ZERO),
                &CancellationToken::new(),
            )
            .unwrap_err();
        assert_eq!(error.code().as_str(), "deadline_exceeded");
    }

    #[test]
    fn a_file_that_is_not_an_owner_descriptor_never_becomes_an_object() {
        let fixture = Fixture::new();
        // A stray file whose name looks like an object, a descriptor of the
        // wrong kind, and one whose declared name disagrees with the file.
        write(
            &fixture.source.join("Catalogs/Резервная копия.xml"),
            "<!-- not a descriptor -->",
        );
        write(
            &fixture.source.join("Catalogs/Подделка.xml"),
            &owner("Подделка", "Document", "Подделка", ""),
        );
        write(
            &fixture.source.join("Catalogs/Валюты/Forms/Чужая.xml"),
            &owner("Другая", "Form", "Другая форма", ""),
        );
        let root = RetainedDirectoryCapability::open(&fixture.source).unwrap();
        let index = WorkspaceFindDirectoryBuilder::default()
            .build(
                &[LayoutFindSource::new(
                    "main",
                    SourceSetKind::Configuration,
                    &root,
                )],
                ProviderDeadline::from_budget(Duration::from_secs(7)),
                &CancellationToken::new(),
            )
            .unwrap();
        for query in [
            "Резервная копия",
            "main:Catalog.Подделка",
            "main:Catalog.Валюты.Form.Чужая",
        ] {
            let found = index.find(FindRequest::new(query).unwrap());
            assert!(
                found.is_nearest() || found.candidates().is_empty(),
                "a file that declares no matching owner became an object: {query}: {found:?}"
            );
        }
        // The real objects beside them are still there.
        assert_eq!(single(&index, "Валюты").0, "main:Catalog.Валюты");
        assert_eq!(
            single(&index, "ФормаЭлемента").0,
            "main:Catalog.Валюты.Form.ФормаЭлемента"
        );
    }

    #[test]
    fn a_descriptor_whose_attributes_start_on_a_new_line_is_still_an_object() {
        let fixture = Fixture::new();
        write(
            &fixture.source.join("Catalogs/Склады.xml"),
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<MetaDataObject xmlns=\"http://v8.1c.ru/8.3/MDClasses\" xmlns:v8=\"http://v8.1c.ru/8.1/data/core\" version=\"2.20\">\n  <Catalog\n    uuid=\"10000000-0000-4000-8000-000000000002\">\n    <Properties><Name>Склады</Name></Properties>\n    <ChildObjects/>\n  </Catalog>\n</MetaDataObject>",
        );
        let root = RetainedDirectoryCapability::open(&fixture.source).unwrap();
        let index = WorkspaceFindDirectoryBuilder::default()
            .build(
                &[LayoutFindSource::new(
                    "main",
                    SourceSetKind::Configuration,
                    &root,
                )],
                ProviderDeadline::from_budget(Duration::from_secs(7)),
                &CancellationToken::new(),
            )
            .unwrap();
        assert_eq!(
            single(&index, "Склады"),
            (
                "main:Catalog.Склады".to_string(),
                "Catalogs/Склады.xml".to_string()
            )
        );
    }
}
