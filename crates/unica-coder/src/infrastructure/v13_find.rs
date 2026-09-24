use crate::application::v13::find::{FindDocument, FindFact, FindFactKind, FindIndex};
use crate::domain::address::{NodeKind, QualifiedAddress};
use crate::domain::cancellation::CancellationToken;
use crate::domain::code_intelligence::ProviderDeadline;
use crate::domain::project_sources::SourceSetKind;
use crate::domain::refusal::{RefusalCode, RefusalDetail};
use crate::infrastructure::metadata_kinds::metadata_kind_by_directory;
use crate::infrastructure::platform::filesystem::{
    RetainedChildCapability, RetainedDirectoryCapability, RetainedRegularFileCapability,
};
use std::collections::HashSet;
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

/// Builds the two-way directory between qualified logical addresses and where
/// objects live in the source layout. It reads that layout only: no typed
/// projection, no module source, no revision lease.
pub(crate) struct WorkspaceFindDirectoryBuilder {
    max_documents: usize,
    max_total_fact_bytes: usize,
    read_head: Arc<HeadReader>,
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
    omissions: FindOmissions,
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
            read_head: Arc::new(read_descriptor_head_prefix),
        }
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
    fn with_document_limit(max_documents: usize) -> Self {
        Self::with_limits(max_documents, DEFAULT_MAX_FACT_BYTES)
    }

    pub(crate) fn build(
        &self,
        sources: &[LayoutFindSource<'_>],
        deadline: ProviderDeadline,
        cancellation: &CancellationToken,
    ) -> Result<FindIndex, FindBuildError> {
        let outcome = self.build_for_search(sources, deadline, cancellation)?;
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
        if sources.len() > MAX_SOURCE_SETS {
            return Err(FindBuildError::new(
                RefusalCode::ProviderLimitExceeded,
                "find source-set count exceeds the bounded workspace limit",
            ));
        }
        let mut build = DirectoryBuild {
            documents: Vec::new(),
            fact_bytes: 0,
            omissions: FindOmissions::default(),
        };
        for source in sources {
            find_checkpoint(deadline, cancellation)?;
            self.add_source(source, &mut build, deadline, cancellation)?;
        }
        Ok(FindBuildOutcome {
            index: FindIndex::new(build.documents),
            omissions: build.omissions,
        })
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
        for entry in immediate_names(source.root, deadline, cancellation)? {
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
            let mut proved_owners = HashSet::new();
            let mut owner_directories = Vec::new();
            let mut unsafe_owner_directories = HashSet::new();
            for owner in immediate_names(&collection, deadline, cancellation)? {
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
                        self.push(
                            build,
                            source,
                            &format!("{}:{}.{stem}", source.name, layout.tag),
                            layout.tag,
                            stem,
                            synonym.as_deref(),
                            &relative,
                        )?;
                        proved_owners.insert(stem.to_string());
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
                .any(|name| proved_owners.contains(name))
            {
                return Err(unsafe_layout_entry());
            }
            for (owner_name, original_identity) in owner_directories {
                find_checkpoint(deadline, cancellation)?;
                if proved_owners.contains(&owner_name) {
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
        for entry in immediate_names(source.root, deadline, cancellation)? {
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
            for entry in immediate_names(&family, deadline, cancellation)? {
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
        if build.documents.len() == self.max_documents {
            return Err(FindBuildError::new(
                RefusalCode::ProviderLimitExceeded,
                "find directory exceeds the bounded workspace entry limit",
            ));
        }
        let next_total = build
            .fact_bytes
            .saturating_add(document.estimated_identity_bytes());
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
    deadline: ProviderDeadline,
    cancellation: &CancellationToken,
) -> Result<Vec<std::ffi::OsString>, FindBuildError> {
    directory
        .read_immediate_names_bounded(MAX_COLLECTION_ENTRIES, || {
            find_checkpoint(deadline, cancellation)
                .map_err(|error| std::io::Error::other(error.to_string()))
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
            assert_eq!(located.at(), "main:Catalog.Валюты", "{query}");
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
            omissions: super::FindOmissions::default(),
        };
        for _ in 0..=super::MAX_OMISSION_DETAILS {
            build.record_omission("main", "descriptor_unreadable");
        }
        assert_eq!(build.omissions.total, super::MAX_OMISSION_DETAILS + 1);
        assert_eq!(build.omissions.details.len(), super::MAX_OMISSION_DETAILS);
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
