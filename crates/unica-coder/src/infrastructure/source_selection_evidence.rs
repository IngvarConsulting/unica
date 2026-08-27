use crate::domain::cancellation::CancellationToken;
use crate::domain::code_intelligence::ProviderDeadline;
use crate::domain::project_sources::{ProjectSourceMap, SourceFormat, SourceSetKind};
use crate::infrastructure::platform::filesystem::{
    is_link_loop_error, FileIdentity, RetainedChildCapability, RetainedDirectoryCapability,
    RetainedRegularFileCapability,
};
use std::ffi::OsString;
use std::io::{ErrorKind, Read, Seek, SeekFrom};
use std::path::{Component, Path, PathBuf};

const SELECTION_READ_CHUNK_BYTES: usize = 64 * 1024;

pub(in crate::infrastructure) struct ResolvedProjectSourceAdmission {
    map: ProjectSourceMap,
    evidence: RetainedSourceSelectionEvidence,
}

pub(in crate::infrastructure) struct RetainedSourceSelectionEvidence {
    workspace: RetainedDirectoryCapability,
    inputs: Vec<RetainedSelectionInput>,
    admitted_semantic: CompleteProjectSourceSelection,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CompleteProjectSourceSelection {
    workspace_root: PathBuf,
    config_present: bool,
    configured_format_raw: Option<String>,
    source_sets: Vec<CompleteSourceSetSelection>,
    effective_source_set: Option<String>,
    effective_source_root: Option<String>,
    source_selection_error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct CompleteSourceSetSelection {
    name: String,
    kind: SourceSetKind,
    path: String,
    source_format: SourceFormat,
    format_evidence: Vec<String>,
    format_probe_error: Option<String>,
}

enum RetainedSelectionInput {
    ExactFile {
        relative: PathBuf,
        file: RetainedRegularFileCapability,
        bytes: Vec<u8>,
    },
    UnresolvedRoute {
        ancestor: RetainedDirectoryCapability,
        relative: PathBuf,
        route: Vec<OsString>,
        classification: SelectionPathClassification,
    },
    DirectoryRoute {
        relative: PathBuf,
        directory: RetainedDirectoryCapability,
    },
    DirectoryMembership {
        relative: PathBuf,
        directory: RetainedDirectoryCapability,
        members: Vec<SelectionMember>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SelectionPathClassification {
    Absent,
    Directory,
    RegularFile,
    ReparseOrUnsupported,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SelectionMember {
    name: OsString,
    kind: SelectionMemberKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SelectionMemberKind {
    Directory,
    RegularFile,
    ReparseOrUnsupported,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SelectionEvidenceFingerprint {
    workspace: FileIdentity,
    inputs: Vec<SelectionInputFingerprint>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum SelectionInputFingerprint {
    ExactFile(PathBuf, FileIdentity, Vec<u8>),
    Unresolved(
        PathBuf,
        FileIdentity,
        Vec<OsString>,
        SelectionPathClassification,
    ),
    Directory(PathBuf, FileIdentity),
    Membership(PathBuf, FileIdentity, Vec<SelectionMember>),
}

pub(in crate::infrastructure) enum RetainedRegularObservation {
    Exact(Vec<u8>),
    Absent,
    WrongKind,
}

pub(in crate::infrastructure) enum RetainedDirectoryObservation {
    Present(RetainedDirectoryCapability),
    AbsentOrWrongKind,
}

pub(in crate::infrastructure) struct RetainedMembershipEntry {
    pub(in crate::infrastructure) name: OsString,
    pub(in crate::infrastructure) directory: Option<RetainedDirectoryCapability>,
}

pub(in crate::infrastructure) struct RetainedSelectionPass {
    workspace: RetainedDirectoryCapability,
    inputs: Vec<RetainedSelectionInput>,
}

impl std::fmt::Debug for ResolvedProjectSourceAdmission {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ResolvedProjectSourceAdmission")
            .field("source_sets", &self.map.source_sets.len())
            .finish_non_exhaustive()
    }
}

impl std::fmt::Debug for RetainedSourceSelectionEvidence {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("RetainedSourceSelectionEvidence")
            .field("input_count", &self.inputs.len())
            .finish_non_exhaustive()
    }
}

impl ResolvedProjectSourceAdmission {
    pub(in crate::infrastructure) fn map(&self) -> &ProjectSourceMap {
        &self.map
    }

    pub(in crate::infrastructure) fn source_root_identity(
        &self,
        relative: &Path,
    ) -> Option<FileIdentity> {
        self.evidence.source_root_identity(relative)
    }

    pub(in crate::infrastructure) fn into_evidence(self) -> RetainedSourceSelectionEvidence {
        self.evidence
    }
}

pub(in crate::infrastructure) fn discover_project_source_admission(
    workspace_root: &Path,
    checkpoint: &mut dyn FnMut() -> Result<(), String>,
) -> Result<ResolvedProjectSourceAdmission, String> {
    checkpoint()?;
    let canonical_workspace =
        crate::infrastructure::source_roots::normalize_path_identity(workspace_root)?;
    let workspace = RetainedDirectoryCapability::open(&canonical_workspace).map_err(|error| {
        format!("project source-map workspace cannot be retained without following links: {error}")
    })?;
    workspace
        .validate_named_identity()
        .map_err(|error| format!("project source-map workspace identity changed: {error}"))?;

    let (first_map, first_pass) =
        crate::infrastructure::project_sources::discover_project_source_map_actor_pass(
            workspace.clone(),
            checkpoint,
        )?;
    checkpoint()?;
    let (second_map, second_pass) =
        crate::infrastructure::project_sources::discover_project_source_map_actor_pass(
            workspace.clone(),
            checkpoint,
        )?;
    workspace
        .validate_named_identity()
        .map_err(|error| format!("project source-map workspace identity changed: {error}"))?;

    let first_semantic = CompleteProjectSourceSelection::from_map(&first_map, workspace.path());
    let second_semantic = CompleteProjectSourceSelection::from_map(&second_map, workspace.path());
    if first_semantic != second_semantic || first_pass.fingerprint() != second_pass.fingerprint() {
        return Err("project source-map changed during retained actor admission".to_string());
    }
    Ok(ResolvedProjectSourceAdmission {
        map: second_map,
        evidence: RetainedSourceSelectionEvidence {
            workspace,
            inputs: second_pass.inputs,
            admitted_semantic: second_semantic,
        },
    })
}

impl CompleteProjectSourceSelection {
    fn from_map(map: &ProjectSourceMap, workspace_root: &Path) -> Self {
        let mut source_sets = map
            .source_sets
            .iter()
            .map(|source| CompleteSourceSetSelection {
                name: source.name.clone(),
                kind: source.kind,
                path: source.path.clone(),
                source_format: source.source_format,
                format_evidence: source.format_evidence.clone(),
                format_probe_error: source.format_probe_error.clone(),
            })
            .collect::<Vec<_>>();
        source_sets.sort();
        Self {
            workspace_root: workspace_root.to_path_buf(),
            config_present: map.config_path.is_some(),
            configured_format_raw: map.configured_format_raw.clone(),
            source_sets,
            effective_source_set: map.effective_source_set.clone(),
            effective_source_root: map.effective_source_root.clone(),
            source_selection_error: map.source_selection_error.clone(),
        }
    }
}

impl RetainedSelectionPass {
    pub(in crate::infrastructure) fn new(workspace: RetainedDirectoryCapability) -> Self {
        Self {
            workspace,
            inputs: Vec::new(),
        }
    }

    pub(in crate::infrastructure) fn workspace_path(&self) -> &Path {
        self.workspace.path()
    }

    pub(in crate::infrastructure) fn observe_regular(
        &mut self,
        relative: &Path,
        max_bytes: usize,
        checkpoint: &mut dyn FnMut() -> Result<(), String>,
    ) -> Result<RetainedRegularObservation, String> {
        checkpoint()?;
        let components = normal_components(relative)?;
        let Some((name, parents)) = components.split_last() else {
            return Ok(RetainedRegularObservation::WrongKind);
        };
        let mut ancestor = self.workspace.clone();
        self.record_directory(relative_prefix(&components, 0), ancestor.clone());
        for (index, component) in parents.iter().enumerate() {
            checkpoint()?;
            match ancestor.retain_immediate_child_nofollow(component) {
                Ok(RetainedChildCapability::Directory(directory)) => {
                    ancestor = directory;
                    self.record_directory(
                        relative_prefix(&components, index + 1),
                        ancestor.clone(),
                    );
                }
                Ok(child) => {
                    self.record_unresolved(
                        ancestor,
                        relative,
                        components[index..].to_vec(),
                        classification_of_child(&child),
                    );
                    return Ok(RetainedRegularObservation::WrongKind);
                }
                Err(error) if error.kind() == ErrorKind::NotFound => {
                    self.record_unresolved(
                        ancestor,
                        relative,
                        components[index..].to_vec(),
                        SelectionPathClassification::Absent,
                    );
                    return Ok(RetainedRegularObservation::Absent);
                }
                Err(error) if open_error_is_wrong_kind(&error) => {
                    self.record_unresolved(
                        ancestor,
                        relative,
                        components[index..].to_vec(),
                        SelectionPathClassification::ReparseOrUnsupported,
                    );
                    return Ok(RetainedRegularObservation::WrongKind);
                }
                Err(error) => {
                    return Err(format!(
                        "project source-map route {} cannot be inspected: {error}",
                        relative.display()
                    ));
                }
            }
        }
        checkpoint()?;
        match ancestor.retain_immediate_child_nofollow(name) {
            Ok(RetainedChildCapability::RegularFile(file)) => {
                file.validate_named_identity().map_err(|error| {
                    format!(
                        "project source-map input {} identity changed before read: {error}",
                        relative.display()
                    )
                })?;
                let bytes = read_regular_exact(&file, max_bytes, checkpoint)?;
                file.validate_named_identity().map_err(|error| {
                    format!(
                        "project source-map input {} identity changed after read: {error}",
                        relative.display()
                    )
                })?;
                self.inputs.push(RetainedSelectionInput::ExactFile {
                    relative: relative.to_path_buf(),
                    file,
                    bytes: bytes.clone(),
                });
                Ok(RetainedRegularObservation::Exact(bytes))
            }
            Ok(child) => {
                self.record_unresolved(
                    ancestor,
                    relative,
                    vec![name.clone()],
                    classification_of_child(&child),
                );
                Ok(RetainedRegularObservation::WrongKind)
            }
            Err(error) if error.kind() == ErrorKind::NotFound => {
                self.record_unresolved(
                    ancestor,
                    relative,
                    vec![name.clone()],
                    SelectionPathClassification::Absent,
                );
                Ok(RetainedRegularObservation::Absent)
            }
            Err(error) if open_error_is_wrong_kind(&error) => {
                self.record_unresolved(
                    ancestor,
                    relative,
                    vec![name.clone()],
                    SelectionPathClassification::ReparseOrUnsupported,
                );
                Ok(RetainedRegularObservation::WrongKind)
            }
            Err(error) => Err(format!(
                "project source-map input {} cannot be inspected: {error}",
                relative.display()
            )),
        }
    }

    pub(in crate::infrastructure) fn observe_directory(
        &mut self,
        relative: &Path,
        checkpoint: &mut dyn FnMut() -> Result<(), String>,
    ) -> Result<RetainedDirectoryObservation, String> {
        checkpoint()?;
        let components = normal_components(relative)?;
        let mut ancestor = self.workspace.clone();
        self.record_directory(PathBuf::new(), ancestor.clone());
        for (index, component) in components.iter().enumerate() {
            checkpoint()?;
            match ancestor.retain_immediate_child_nofollow(component) {
                Ok(RetainedChildCapability::Directory(directory)) => {
                    ancestor = directory;
                    self.record_directory(
                        relative_prefix(&components, index + 1),
                        ancestor.clone(),
                    );
                }
                Ok(child) => {
                    self.record_unresolved(
                        ancestor,
                        relative,
                        components[index..].to_vec(),
                        classification_of_child(&child),
                    );
                    return Ok(RetainedDirectoryObservation::AbsentOrWrongKind);
                }
                Err(error) if error.kind() == ErrorKind::NotFound => {
                    self.record_unresolved(
                        ancestor,
                        relative,
                        components[index..].to_vec(),
                        SelectionPathClassification::Absent,
                    );
                    return Ok(RetainedDirectoryObservation::AbsentOrWrongKind);
                }
                Err(error) if open_error_is_wrong_kind(&error) => {
                    self.record_unresolved(
                        ancestor,
                        relative,
                        components[index..].to_vec(),
                        SelectionPathClassification::ReparseOrUnsupported,
                    );
                    return Ok(RetainedDirectoryObservation::AbsentOrWrongKind);
                }
                Err(error) => {
                    return Err(format!(
                        "project source-map directory {} cannot be inspected: {error}",
                        relative.display()
                    ));
                }
            }
        }
        ancestor.validate_named_identity().map_err(|error| {
            format!(
                "project source-map directory {} identity changed: {error}",
                relative.display()
            )
        })?;
        Ok(RetainedDirectoryObservation::Present(ancestor))
    }

    pub(in crate::infrastructure) fn observe_membership(
        &mut self,
        relative: &Path,
        directory: &RetainedDirectoryCapability,
        limit: usize,
        checkpoint: &mut dyn FnMut() -> Result<(), String>,
    ) -> Result<Vec<RetainedMembershipEntry>, String> {
        directory.validate_named_identity().map_err(|error| {
            format!(
                "project source-map directory {} identity changed before enumeration: {error}",
                relative.display()
            )
        })?;
        let mut checkpoint_error = None;
        let names = directory.read_immediate_names_bounded(limit, || match checkpoint() {
            Ok(()) => Ok(()),
            Err(reason) => {
                checkpoint_error = Some(reason);
                Err(std::io::Error::new(
                    ErrorKind::Interrupted,
                    "project source-map checkpoint stopped enumeration",
                ))
            }
        });
        if let Some(reason) = checkpoint_error {
            return Err(reason);
        }
        let names = names.map_err(|error| {
            format!(
                "project source-map directory {} cannot be enumerated: {error}",
                relative.display()
            )
        })?;
        let mut members = Vec::with_capacity(names.len());
        let mut result = Vec::with_capacity(names.len());
        for name in names {
            checkpoint()?;
            let (kind, child_directory) = match directory.retain_immediate_child_nofollow(&name) {
                Ok(RetainedChildCapability::Directory(child)) => {
                    (SelectionMemberKind::Directory, Some(child))
                }
                Ok(RetainedChildCapability::RegularFile(_)) => {
                    (SelectionMemberKind::RegularFile, None)
                }
                Ok(
                    RetainedChildCapability::ReparsePoint | RetainedChildCapability::Unsupported,
                ) => (SelectionMemberKind::ReparseOrUnsupported, None),
                Err(error) if open_error_is_wrong_kind(&error) => {
                    (SelectionMemberKind::ReparseOrUnsupported, None)
                }
                Err(error) => {
                    return Err(format!(
                        "project source-map directory member {} changed during enumeration: {error}",
                        Path::new(relative).join(&name).display()
                    ));
                }
            };
            members.push(SelectionMember {
                name: name.clone(),
                kind,
            });
            result.push(RetainedMembershipEntry {
                name,
                directory: child_directory,
            });
        }
        directory.validate_named_identity().map_err(|error| {
            format!(
                "project source-map directory {} identity changed after enumeration: {error}",
                relative.display()
            )
        })?;
        self.inputs
            .push(RetainedSelectionInput::DirectoryMembership {
                relative: relative.to_path_buf(),
                directory: directory.clone(),
                members,
            });
        Ok(result)
    }

    fn record_directory(&mut self, relative: PathBuf, directory: RetainedDirectoryCapability) {
        self.inputs.push(RetainedSelectionInput::DirectoryRoute {
            relative,
            directory,
        });
    }

    fn record_unresolved(
        &mut self,
        ancestor: RetainedDirectoryCapability,
        relative: &Path,
        route: Vec<OsString>,
        classification: SelectionPathClassification,
    ) {
        self.inputs.push(RetainedSelectionInput::UnresolvedRoute {
            ancestor,
            relative: relative.to_path_buf(),
            route,
            classification,
        });
    }

    fn fingerprint(&self) -> SelectionEvidenceFingerprint {
        SelectionEvidenceFingerprint {
            workspace: self.workspace.identity(),
            inputs: self
                .inputs
                .iter()
                .map(|input| match input {
                    RetainedSelectionInput::ExactFile {
                        relative,
                        file,
                        bytes,
                    } => SelectionInputFingerprint::ExactFile(
                        relative.clone(),
                        file.identity(),
                        bytes.clone(),
                    ),
                    RetainedSelectionInput::UnresolvedRoute {
                        ancestor,
                        relative,
                        route,
                        classification,
                    } => SelectionInputFingerprint::Unresolved(
                        relative.clone(),
                        ancestor.identity(),
                        route.clone(),
                        *classification,
                    ),
                    RetainedSelectionInput::DirectoryRoute {
                        relative,
                        directory,
                    } => {
                        SelectionInputFingerprint::Directory(relative.clone(), directory.identity())
                    }
                    RetainedSelectionInput::DirectoryMembership {
                        relative,
                        directory,
                        members,
                    } => SelectionInputFingerprint::Membership(
                        relative.clone(),
                        directory.identity(),
                        members.clone(),
                    ),
                })
                .collect(),
        }
    }
}

impl RetainedSourceSelectionEvidence {
    pub(in crate::infrastructure) fn validate(
        &self,
        deadline: ProviderDeadline,
        cancellation: &CancellationToken,
    ) -> Result<(), SourceSelectionEvidenceError> {
        self.validate_complete_pass(deadline, cancellation)?;
        self.validate_complete_pass(deadline, cancellation)
    }

    fn validate_complete_pass(
        &self,
        deadline: ProviderDeadline,
        cancellation: &CancellationToken,
    ) -> Result<(), SourceSelectionEvidenceError> {
        selection_checkpoint(deadline, cancellation)?;
        self.workspace
            .validate_named_identity()
            .map_err(changed_identity)?;
        if self.admitted_semantic.workspace_root != self.workspace.path() {
            return Err(SourceSelectionEvidenceError::provider(
                "project source-map semantic workspace does not match retained authority",
            ));
        }
        for input in &self.inputs {
            selection_checkpoint(deadline, cancellation)?;
            match input {
                RetainedSelectionInput::ExactFile { file, bytes, .. } => {
                    file.validate_named_identity().map_err(changed_identity)?;
                    let current =
                        read_regular_validation(file, bytes.len(), deadline, cancellation)?;
                    if &current != bytes {
                        return Err(SourceSelectionEvidenceError::changed(
                            "project source-map retained file bytes changed",
                        ));
                    }
                    file.validate_named_identity().map_err(changed_identity)?;
                }
                RetainedSelectionInput::UnresolvedRoute {
                    ancestor,
                    relative: _,
                    route,
                    classification,
                } => {
                    validate_unresolved_route(ancestor, route, *classification)?;
                }
                RetainedSelectionInput::DirectoryMembership {
                    directory, members, ..
                } => {
                    validate_membership(directory, members, deadline, cancellation)?;
                }
                RetainedSelectionInput::DirectoryRoute { directory, .. } => {
                    directory
                        .validate_named_identity()
                        .map_err(changed_identity)?;
                }
            }
            selection_checkpoint(deadline, cancellation)?;
        }
        self.workspace
            .validate_named_identity()
            .map_err(changed_identity)?;
        selection_checkpoint(deadline, cancellation)
    }

    fn source_root_identity(&self, relative: &Path) -> Option<FileIdentity> {
        if relative.as_os_str().is_empty() || relative == Path::new(".") {
            return Some(self.workspace.identity());
        }
        self.inputs.iter().rev().find_map(|input| match input {
            RetainedSelectionInput::DirectoryRoute {
                relative: observed,
                directory,
            } if observed == relative => Some(directory.identity()),
            _ => None,
        })
    }

    pub(in crate::infrastructure) fn validate_dry_result(
        &self,
        deadline: ProviderDeadline,
        cancellation: &CancellationToken,
    ) -> Result<(), SourceSelectionEvidenceError> {
        self.validate(deadline, cancellation)
    }

    pub(in crate::infrastructure) fn validate_final(
        &self,
        deadline: ProviderDeadline,
        cancellation: &CancellationToken,
    ) -> Result<(), SourceSelectionEvidenceError> {
        self.validate(deadline, cancellation)
    }
}

fn validate_membership(
    directory: &RetainedDirectoryCapability,
    expected: &[SelectionMember],
    deadline: ProviderDeadline,
    cancellation: &CancellationToken,
) -> Result<(), SourceSelectionEvidenceError> {
    directory
        .validate_named_identity()
        .map_err(changed_identity)?;
    let mut checkpoint_error = None;
    let names = directory.read_immediate_names_bounded(expected.len().saturating_add(1), || {
        match selection_checkpoint(deadline, cancellation) {
            Ok(()) => Ok(()),
            Err(error) => {
                checkpoint_error = Some(error);
                Err(std::io::Error::new(
                    ErrorKind::Interrupted,
                    "project source-map membership validation stopped",
                ))
            }
        }
    });
    if let Some(error) = checkpoint_error {
        return Err(error);
    }
    let names = names.map_err(|error| {
        SourceSelectionEvidenceError::changed(format!(
            "project source-map retained directory membership changed: {error}"
        ))
    })?;
    let mut observed = Vec::with_capacity(names.len());
    for name in names {
        selection_checkpoint(deadline, cancellation)?;
        let kind = match directory.retain_immediate_child_nofollow(&name) {
            Ok(RetainedChildCapability::Directory(_)) => SelectionMemberKind::Directory,
            Ok(RetainedChildCapability::RegularFile(_)) => SelectionMemberKind::RegularFile,
            Ok(RetainedChildCapability::ReparsePoint | RetainedChildCapability::Unsupported) => {
                SelectionMemberKind::ReparseOrUnsupported
            }
            Err(error) if open_error_is_wrong_kind(&error) => {
                SelectionMemberKind::ReparseOrUnsupported
            }
            Err(error) => {
                return Err(SourceSelectionEvidenceError::changed(format!(
                    "project source-map retained directory member changed: {error}"
                )));
            }
        };
        observed.push(SelectionMember { name, kind });
    }
    if observed == expected {
        directory
            .validate_named_identity()
            .map_err(changed_identity)
    } else {
        Err(SourceSelectionEvidenceError::changed(
            "project source-map retained directory membership changed",
        ))
    }
}

fn validate_unresolved_route(
    ancestor: &RetainedDirectoryCapability,
    route: &[OsString],
    expected: SelectionPathClassification,
) -> Result<(), SourceSelectionEvidenceError> {
    ancestor
        .validate_named_identity()
        .map_err(changed_identity)?;
    let Some(name) = route.first() else {
        return Err(SourceSelectionEvidenceError::provider(
            "project source-map unresolved route is empty",
        ));
    };
    let observed = match ancestor.retain_immediate_child_nofollow(name) {
        Ok(child) => classification_of_child(&child),
        Err(error) if error.kind() == ErrorKind::NotFound => SelectionPathClassification::Absent,
        Err(error) if open_error_is_wrong_kind(&error) => {
            SelectionPathClassification::ReparseOrUnsupported
        }
        Err(error) => {
            return Err(SourceSelectionEvidenceError::provider(format!(
                "project source-map unresolved route cannot be inspected: {error}"
            )));
        }
    };
    ancestor
        .validate_named_identity()
        .map_err(changed_identity)?;
    if observed == expected {
        Ok(())
    } else {
        Err(SourceSelectionEvidenceError::changed(
            "project source-map unresolved route changed",
        ))
    }
}

fn normal_components(relative: &Path) -> Result<Vec<OsString>, String> {
    let mut components = Vec::new();
    for component in relative.components() {
        match component {
            Component::Normal(name) => components.push(name.to_os_string()),
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(format!(
                    "project source-map route is not a closed workspace-relative path: {}",
                    relative.display()
                ));
            }
        }
    }
    Ok(components)
}

fn relative_prefix(components: &[OsString], count: usize) -> PathBuf {
    components.iter().take(count).collect()
}

fn classification_of_child(child: &RetainedChildCapability) -> SelectionPathClassification {
    match child {
        RetainedChildCapability::Directory(_) => SelectionPathClassification::Directory,
        RetainedChildCapability::RegularFile(_) => SelectionPathClassification::RegularFile,
        RetainedChildCapability::ReparsePoint | RetainedChildCapability::Unsupported => {
            SelectionPathClassification::ReparseOrUnsupported
        }
    }
}

fn open_error_is_wrong_kind(error: &std::io::Error) -> bool {
    matches!(
        error.kind(),
        ErrorKind::InvalidInput | ErrorKind::InvalidData | ErrorKind::NotADirectory
    ) || is_link_loop_error(error)
}

fn read_regular_exact(
    file: &RetainedRegularFileCapability,
    max_bytes: usize,
    checkpoint: &mut dyn FnMut() -> Result<(), String>,
) -> Result<Vec<u8>, String> {
    let mut file = file
        .try_clone_file()
        .map_err(|error| format!("project source-map input cannot be cloned: {error}"))?;
    file.seek(SeekFrom::Start(0))
        .map_err(|error| format!("project source-map input cannot be rewound: {error}"))?;
    let mut bytes = Vec::new();
    let mut chunk = [0_u8; SELECTION_READ_CHUNK_BYTES];
    loop {
        checkpoint()?;
        let read = file
            .read(&mut chunk)
            .map_err(|error| format!("project source-map input cannot be read: {error}"))?;
        if read == 0 {
            break;
        }
        bytes.extend_from_slice(&chunk[..read]);
        if bytes.len() > max_bytes {
            return Err(format!(
                "project source-map input exceeds {max_bytes} bytes"
            ));
        }
        checkpoint()?;
    }
    Ok(bytes)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::infrastructure) enum SourceSelectionEvidenceErrorKind {
    Cancelled,
    Deadline,
    Changed,
    Provider,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::infrastructure) struct SourceSelectionEvidenceError {
    kind: SourceSelectionEvidenceErrorKind,
    message: String,
}

impl SourceSelectionEvidenceError {
    pub(in crate::infrastructure) const fn kind(&self) -> SourceSelectionEvidenceErrorKind {
        self.kind
    }

    fn changed(message: impl Into<String>) -> Self {
        Self {
            kind: SourceSelectionEvidenceErrorKind::Changed,
            message: message.into(),
        }
    }

    fn provider(message: impl Into<String>) -> Self {
        Self {
            kind: SourceSelectionEvidenceErrorKind::Provider,
            message: message.into(),
        }
    }
}

impl std::fmt::Display for SourceSelectionEvidenceError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.message.fmt(formatter)
    }
}

impl std::error::Error for SourceSelectionEvidenceError {}

fn selection_checkpoint(
    deadline: ProviderDeadline,
    cancellation: &CancellationToken,
) -> Result<(), SourceSelectionEvidenceError> {
    if cancellation.is_cancelled() {
        Err(SourceSelectionEvidenceError {
            kind: SourceSelectionEvidenceErrorKind::Cancelled,
            message: "project source-map validation cancelled".to_string(),
        })
    } else if deadline.remaining().is_zero() {
        Err(SourceSelectionEvidenceError {
            kind: SourceSelectionEvidenceErrorKind::Deadline,
            message: "project source-map validation deadline exceeded".to_string(),
        })
    } else {
        Ok(())
    }
}

fn changed_identity(error: std::io::Error) -> SourceSelectionEvidenceError {
    SourceSelectionEvidenceError::changed(format!(
        "project source-map retained identity changed: {error}"
    ))
}

fn read_regular_validation(
    file: &RetainedRegularFileCapability,
    max_bytes: usize,
    deadline: ProviderDeadline,
    cancellation: &CancellationToken,
) -> Result<Vec<u8>, SourceSelectionEvidenceError> {
    selection_checkpoint(deadline, cancellation)?;
    let mut file = file.try_clone_file().map_err(changed_identity)?;
    file.seek(SeekFrom::Start(0)).map_err(changed_identity)?;
    let mut bytes = Vec::new();
    let mut chunk = [0_u8; SELECTION_READ_CHUNK_BYTES];
    loop {
        selection_checkpoint(deadline, cancellation)?;
        let read = file.read(&mut chunk).map_err(changed_identity)?;
        if read == 0 {
            break;
        }
        bytes.extend_from_slice(&chunk[..read]);
        if bytes.len() > max_bytes {
            return Err(SourceSelectionEvidenceError::changed(
                "project source-map retained file length changed",
            ));
        }
        selection_checkpoint(deadline, cancellation)?;
    }
    Ok(bytes)
}
