//! Closed, actor-owned directory removals. A file-only manifest never grants
//! authority over a parent: every directory name and identity is proved here.
use super::apply::{
    ApplyStagingError, ApplyStagingErrorKind, MAX_APPLY_NAMESPACE_DEPTH,
    MAX_APPLY_NAMESPACE_ENTRIES,
};

mod collection;
use crate::platform::filesystem::{
    portable_permissions, stable_path_identity_bytes, FileIdentity, PortablePermissions,
    RetainedChildCapability, RetainedDirectoryCapability, RetainedRegularFileCapability,
};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::ffi::{OsStr, OsString};
use std::io::ErrorKind;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use unica_work_control::cancellation::CancellationToken;
use unica_work_control::provider_deadline::ProviderDeadline;

#[derive(Debug, Clone)]
pub(super) struct RemovedFileImage {
    pub(super) len: usize,
    pub(super) sha256: [u8; 32],
}

#[derive(Debug, Clone)]
enum ChildImage {
    Directory(DirectoryImage),
    File(FileIdentity, RemovedFileImage),
}

#[derive(Debug, Clone)]
struct DirectoryImage {
    identity: FileIdentity,
    permissions: PortablePermissions,
    children: BTreeMap<OsString, ChildImage>,
}

#[derive(Debug, Clone)]
pub(super) struct DirectoryRemoval {
    pub(super) root: Arc<RetainedDirectoryCapability>,
    pub(super) relative: PathBuf,
    pub(super) owner_descriptor: PathBuf,
    route: Vec<FileIdentity>,
    image: DirectoryImage,
    deadline: ProviderDeadline,
    cancellation: CancellationToken,
}

#[derive(Debug)]
pub(super) struct PublishedDirectoryRemoval {
    pub(super) plan: DirectoryRemoval,
    pub(super) parent: RetainedDirectoryCapability,
    pub(super) recovery: RetainedDirectoryCapability,
    pub(super) recovery_name: OsString,
}

pub(super) struct DirectoryPublishFailure {
    pub(super) error: ApplyStagingError,
    pub(super) artifact: Option<Box<PublishedDirectoryRemoval>>,
}

fn io_error(error: std::io::Error) -> ApplyStagingError {
    let kind = match error.kind() {
        ErrorKind::NotFound | ErrorKind::AlreadyExists => ApplyStagingErrorKind::ConcurrentRevision,
        _ => ApplyStagingErrorKind::UnsupportedProvider,
    };
    ApplyStagingError::new(kind, format!("retained directory removal: {error}"))
}

fn changed(message: &str) -> ApplyStagingError {
    ApplyStagingError::new(ApplyStagingErrorKind::ConcurrentRevision, message)
}

fn wrong_kind() -> ApplyStagingError {
    ApplyStagingError::new(
        ApplyStagingErrorKind::ContainmentIdentity,
        "directory removal input changed kind or contains a link/reparse point",
    )
}

fn checkpoint(
    deadline: ProviderDeadline,
    cancellation: &CancellationToken,
) -> Result<(), ApplyStagingError> {
    if cancellation.is_cancelled() {
        return Err(ApplyStagingError::new(
            ApplyStagingErrorKind::Cancelled,
            "directory removal cancelled",
        ));
    }
    if deadline.is_elapsed() {
        return Err(ApplyStagingError::new(
            ApplyStagingErrorKind::Deadline,
            "directory removal deadline elapsed",
        ));
    }
    Ok(())
}

fn file_matches(
    file: &RetainedRegularFileCapability,
    identity: FileIdentity,
    image: &RemovedFileImage,
) -> Result<(), ApplyStagingError> {
    file.validate_named_identity().map_err(io_error)?;
    if file.identity() != identity || file.hard_link_count().map_err(io_error)? != 1 {
        return Err(changed(
            "directory removal file identity or hard-link count changed",
        ));
    }
    let bytes = file.read_bounded(image.len).map_err(|error| {
        if error.kind() == ErrorKind::InvalidData {
            changed("directory removal file grew beyond its captured preimage")
        } else {
            io_error(error)
        }
    })?;
    if bytes.len() != image.len || <[u8; 32]>::from(Sha256::digest(&bytes)) != image.sha256 {
        return Err(changed("directory removal file preimage changed"));
    }
    file.validate_named_identity().map_err(io_error)
}

fn read_names(
    directory: &RetainedDirectoryCapability,
    limit: usize,
    stop: &impl Fn() -> Result<(), ApplyStagingError>,
) -> Result<Vec<OsString>, ApplyStagingError> {
    let mut stopped = None;
    directory
        .read_immediate_names_bounded(limit, || {
            stop().map_err(|error| {
                stopped = Some(error);
                std::io::Error::other("directory removal observation stopped")
            })
        })
        .map_err(|error| stopped.unwrap_or_else(|| io_error(error)))
}

impl DirectoryImage {
    fn validate(
        &self,
        directory: &RetainedDirectoryCapability,
        stop: &impl Fn() -> Result<(), ApplyStagingError>,
    ) -> Result<(), ApplyStagingError> {
        stop()?;
        directory.validate_named_identity().map_err(io_error)?;
        let metadata = directory
            .try_clone_directory()
            .and_then(|f| f.metadata())
            .map_err(io_error)?;
        if directory.identity() != self.identity || !self.permissions.matches(&metadata) {
            return Err(changed("directory removal identity or permissions changed"));
        }
        let names = read_names(directory, self.children.len().saturating_add(1), stop)?;
        if names.len() != self.children.len()
            || names.iter().any(|name| !self.children.contains_key(name))
        {
            return Err(changed("directory removal inventory changed"));
        }
        for (name, expected) in &self.children {
            stop()?;
            match (
                expected,
                directory
                    .retain_immediate_child_nofollow(name)
                    .map_err(io_error)?,
            ) {
                (ChildImage::Directory(image), RetainedChildCapability::Directory(child)) => {
                    image.validate(&child, stop)?
                }
                (ChildImage::File(identity, image), RetainedChildCapability::RegularFile(file)) => {
                    file_matches(&file, *identity, image)?
                }
                _ => return Err(wrong_kind()),
            }
        }
        directory.validate_named_identity().map_err(io_error)
    }

    fn fingerprint(&self, hash: &mut Sha256) -> Result<(), ApplyStagingError> {
        hash.update(self.identity.stable_bytes());
        hash.update(self.permissions.stable_bytes());
        hash.update((self.children.len() as u64).to_be_bytes());
        for (name, child) in &self.children {
            let path = stable_path_identity_bytes(Path::new(name))
                .map_err(|error| io_error(std::io::Error::other(error)))?;
            hash.update((path.len() as u64).to_be_bytes());
            hash.update(path);
            match child {
                ChildImage::Directory(image) => {
                    hash.update([1]);
                    image.fingerprint(hash)?;
                }
                ChildImage::File(identity, image) => {
                    hash.update([2]);
                    hash.update(identity.stable_bytes());
                    hash.update((image.len as u64).to_be_bytes());
                    hash.update(image.sha256);
                }
            }
        }
        Ok(())
    }

    fn cleanup(&self, directory: &RetainedDirectoryCapability) -> Result<(), ApplyStagingError> {
        // The complete image is checked before the first destructive cleanup.
        // Each leaf is then re-proved; a late foreign entry is never selected.
        self.validate(directory, &|| Ok(()))?;
        for (name, image) in &self.children {
            match (
                image,
                directory
                    .retain_immediate_child_nofollow(name)
                    .map_err(io_error)?,
            ) {
                (ChildImage::Directory(image), RetainedChildCapability::Directory(child)) => {
                    image.cleanup(&child)?
                }
                (ChildImage::File(identity, image), RetainedChildCapability::RegularFile(file)) => {
                    file_matches(&file, *identity, image)?;
                    file.remove_named_identity_relative_for_retained_apply(&private_name())
                        .map_err(io_error)?;
                }
                _ => return Err(wrong_kind()),
            }
        }
        directory
            .remove_empty_named_identity_relative_for_retained_apply(&private_name())
            .map_err(io_error)
    }
}

fn private_name() -> OsString {
    OsString::from(format!(
        "{}dir-{}",
        crate::compile_transaction::PUBLICATION_ARTIFACT_PREFIX,
        uuid::Uuid::new_v4()
    ))
}

impl DirectoryRemoval {
    pub(super) fn capture(
        root: Arc<RetainedDirectoryCapability>,
        relative: &Path,
        owner_descriptor: &Path,
        files: &BTreeMap<FileIdentity, RemovedFileImage>,
        deadline: ProviderDeadline,
        cancellation: CancellationToken,
    ) -> Result<Vec<Self>, ApplyStagingError> {
        let Some((directory, route)) = retain_route(&root, relative, deadline, &cancellation)?
        else {
            return Ok(Vec::new());
        };
        let mut remaining = MAX_APPLY_NAMESPACE_ENTRIES;
        let mut output = Vec::new();
        let image = capture_image(
            &directory,
            relative,
            &route,
            &root,
            owner_descriptor,
            files,
            deadline,
            &cancellation,
            0,
            &mut remaining,
            &mut output,
        )?;
        if let Some(image) = image {
            output.clear();
            output.push(Self {
                root,
                relative: relative.to_path_buf(),
                owner_descriptor: owner_descriptor.to_path_buf(),
                route,
                image,
                deadline,
                cancellation,
            });
        }
        Ok(output)
    }

    pub(super) fn rebind(&mut self, deadline: ProviderDeadline, cancellation: CancellationToken) {
        self.deadline = deadline;
        self.cancellation = cancellation;
    }

    pub(super) fn contains(&self, path: &Path) -> bool {
        path.starts_with(&self.relative)
    }

    pub(super) fn fingerprint(&self, hash: &mut Sha256) -> Result<(), ApplyStagingError> {
        hash.update(self.root.identity().stable_bytes());
        for path in [&self.relative, &self.owner_descriptor] {
            let bytes = stable_path_identity_bytes(path)
                .map_err(|error| io_error(std::io::Error::other(error)))?;
            hash.update((bytes.len() as u64).to_be_bytes());
            hash.update(bytes);
        }
        for identity in &self.route {
            hash.update(identity.stable_bytes());
        }
        self.image.fingerprint(hash)
    }

    pub(super) fn partition(&self, surviving_paths: &[PathBuf]) -> Vec<Self> {
        if !surviving_paths.iter().any(|path| self.contains(path)) {
            return vec![self.clone()];
        }
        let mut result = Vec::new();
        for (name, child) in &self.image.children {
            if let ChildImage::Directory(image) = child {
                let mut plan = self.clone();
                plan.relative.push(name);
                plan.route.push(self.image.identity);
                plan.image = image.clone();
                result.extend(plan.partition(surviving_paths));
            }
        }
        result
    }

    pub(super) fn validate_source(
        &self,
    ) -> Result<(RetainedDirectoryCapability, RetainedDirectoryCapability), ApplyStagingError> {
        checkpoint(self.deadline, &self.cancellation)?;
        let mut directory = self.root.as_ref().clone();
        for (index, (part, identity)) in self.relative.components().zip(&self.route).enumerate() {
            checkpoint(self.deadline, &self.cancellation)?;
            directory.validate_named_identity().map_err(io_error)?;
            if directory.identity() != *identity {
                return Err(changed("directory removal route identity changed"));
            }
            let parent = directory.clone();
            directory = match directory
                .retain_immediate_child_nofollow(part.as_os_str())
                .map_err(io_error)?
            {
                RetainedChildCapability::Directory(child) => child,
                _ => return Err(wrong_kind()),
            };
            if index + 1 == self.route.len() {
                self.image.validate(&directory, &|| {
                    checkpoint(self.deadline, &self.cancellation)
                })?;
                return Ok((parent, directory));
            }
        }
        Err(changed(
            "directory removal route no longer reaches its original identity",
        ))
    }

    pub(super) fn publish(&self) -> Result<PublishedDirectoryRemoval, DirectoryPublishFailure> {
        let (parent, directory) =
            self.validate_source()
                .map_err(|error| DirectoryPublishFailure {
                    error,
                    artifact: None,
                })?;
        let recovery_name = private_name();
        match parent.displace_directory_child_no_replace(
            self.relative.file_name().expect("non-root directory"),
            &directory,
            &recovery_name,
        ) {
            Ok(recovery) => Ok(PublishedDirectoryRemoval {
                plan: self.clone(),
                parent,
                recovery,
                recovery_name,
            }),
            Err(error) => {
                let (error, artifact) = error.into_parts();
                Err(DirectoryPublishFailure {
                    error: io_error(error),
                    artifact: artifact.map(|(recovery, name, _)| {
                        Box::new(PublishedDirectoryRemoval {
                            plan: self.clone(),
                            parent,
                            recovery,
                            recovery_name: name,
                        })
                    }),
                })
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn capture_image(
    directory: &RetainedDirectoryCapability,
    relative: &Path,
    route: &[FileIdentity],
    root: &Arc<RetainedDirectoryCapability>,
    owner_descriptor: &Path,
    files: &BTreeMap<FileIdentity, RemovedFileImage>,
    deadline: ProviderDeadline,
    cancellation: &CancellationToken,
    depth: usize,
    remaining: &mut usize,
    output: &mut Vec<DirectoryRemoval>,
) -> Result<Option<DirectoryImage>, ApplyStagingError> {
    checkpoint(deadline, cancellation)?;
    if depth > MAX_APPLY_NAMESPACE_DEPTH {
        return Err(ApplyStagingError::new(
            ApplyStagingErrorKind::UnsupportedProvider,
            "directory removal exceeds namespace depth bound",
        ));
    }
    directory.validate_named_identity().map_err(io_error)?;
    // One generated child may be observed but is excluded from Source authority
    // and the shared non-generated entry budget, just as enumerate_tree does.
    let names = read_names(directory, remaining.saturating_add(1), &|| {
        checkpoint(deadline, cancellation)
    })?;
    let permissions = portable_permissions(
        &directory
            .try_clone_directory()
            .and_then(|f| f.metadata())
            .map_err(io_error)?,
    );
    let mut children = BTreeMap::new();
    let mut complete = true;
    for name in names {
        checkpoint(deadline, cancellation)?;
        let generated = directory
            .child_names_equivalent(&name, OsStr::new(".build"))
            .map_err(io_error)?;
        if generated {
            complete = false;
            continue;
        }
        *remaining = remaining.checked_sub(1).ok_or_else(|| {
            ApplyStagingError::new(
                ApplyStagingErrorKind::UnsupportedProvider,
                "directory removal exceeds namespace entry bound",
            )
        })?;
        match directory
            .retain_immediate_child_nofollow(&name)
            .map_err(io_error)?
        {
            RetainedChildCapability::Directory(child) => {
                let path = relative.join(&name);
                let mut child_route = route.to_vec();
                child_route.push(directory.identity());
                let start = output.len();
                if let Some(image) = capture_image(
                    &child,
                    &path,
                    &child_route,
                    root,
                    owner_descriptor,
                    files,
                    deadline,
                    cancellation,
                    depth + 1,
                    remaining,
                    output,
                )? {
                    output.truncate(start);
                    output.push(DirectoryRemoval {
                        root: Arc::clone(root),
                        relative: path,
                        owner_descriptor: owner_descriptor.to_path_buf(),
                        route: child_route,
                        image: image.clone(),
                        deadline,
                        cancellation: cancellation.clone(),
                    });
                    children.insert(name, ChildImage::Directory(image));
                } else {
                    complete = false;
                }
            }
            RetainedChildCapability::RegularFile(file) => {
                if let Some(image) = files.get(&file.identity()) {
                    file_matches(&file, file.identity(), image)?;
                    children.insert(name, ChildImage::File(file.identity(), image.clone()));
                } else {
                    complete = false;
                }
            }
            _ => return Err(wrong_kind()),
        }
    }
    directory.validate_named_identity().map_err(io_error)?;
    Ok(complete.then_some(DirectoryImage {
        identity: directory.identity(),
        permissions,
        children,
    }))
}

impl PublishedDirectoryRemoval {
    pub(super) fn validate(&self) -> Result<(), ApplyStagingError> {
        self.plan.image.validate(&self.recovery, &|| {
            checkpoint(self.plan.deadline, &self.plan.cancellation)
        })?;
        match self.parent.retain_immediate_child_nofollow(
            self.plan.relative.file_name().expect("non-root directory"),
        ) {
            Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
            Err(error) => Err(io_error(error)),
            Ok(_) => Err(changed("removed directory public name was occupied")),
        }
    }

    pub(super) fn rollback(&self) -> Result<(), ApplyStagingError> {
        self.plan.image.validate(&self.recovery, &|| Ok(()))?;
        self.parent
            .restore_displaced_directory_child_no_replace(
                &self.recovery,
                self.plan.relative.file_name().expect("non-root directory"),
            )
            .map(|_| ())
            .map_err(|error| {
                let (error, _) = error.into_parts();
                io_error(error)
            })
    }

    pub(super) fn cleanup(&self) -> Result<(), ApplyStagingError> {
        self.plan.image.cleanup(&self.recovery)
    }
}

#[cfg(test)]
mod plan_tests;

fn retain_route(
    root: &RetainedDirectoryCapability,
    relative: &Path,
    deadline: ProviderDeadline,
    cancellation: &CancellationToken,
) -> Result<Option<(RetainedDirectoryCapability, Vec<FileIdentity>)>, ApplyStagingError> {
    let components = relative
        .components()
        .map(|part| match part {
            Component::Normal(name) => Ok(name.to_os_string()),
            _ => Err(wrong_kind()),
        })
        .collect::<Result<Vec<_>, _>>()?;
    if components.is_empty() {
        return Err(wrong_kind());
    }
    let mut directory = root.clone();
    let mut route = Vec::new();
    for name in &components {
        checkpoint(deadline, cancellation)?;
        if directory
            .child_names_equivalent(name, OsStr::new(".build"))
            .map_err(io_error)?
        {
            return Err(wrong_kind());
        }
        route.push(directory.identity());
        directory = match directory.retain_immediate_child_nofollow(name) {
            Ok(RetainedChildCapability::Directory(child)) => child,
            Err(e) if e.kind() == ErrorKind::NotFound => return Ok(None),
            Ok(_) => return Err(wrong_kind()),
            Err(e) => return Err(io_error(e)),
        };
    }
    directory.validate_named_identity().map_err(io_error)?;
    Ok(Some((directory, route)))
}
