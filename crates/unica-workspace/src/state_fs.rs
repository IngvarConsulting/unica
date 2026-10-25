//! Retained execution/runtime state operations. Source codecs use `read` and
//! actor-issued proposals instead of this state publication protocol.
use crate::platform::filesystem::{self, execution_store as physical};
use crate::read::{ReadFile, SourceDirectory};
use std::ffi::{OsStr, OsString};
use std::io::{self, Seek, SeekFrom, Write};
use std::path::Path;

pub use crate::platform::filesystem::FileIdentity;
pub use physical::{PublicationCertainty, PublicationFailure, PublishMode};

#[derive(Debug, Clone)]
pub struct StateDirectory {
    retained: filesystem::RetainedDirectoryCapability,
}

impl StateDirectory {
    pub fn open(path: &Path) -> io::Result<Self> {
        filesystem::RetainedDirectoryCapability::open(path).map(|retained| Self { retained })
    }

    pub fn open_or_create(path: &Path) -> io::Result<Self> {
        filesystem::RetainedDirectoryCapability::open_or_create(path)
            .map(|retained| Self { retained })
    }

    pub fn path(&self) -> &Path {
        self.retained.path()
    }

    pub fn identity(&self) -> FileIdentity {
        self.retained.identity()
    }

    pub fn validate_named_identity(&self) -> io::Result<()> {
        self.retained.validate_named_identity()
    }

    pub fn named_identity_matches(&self) -> io::Result<bool> {
        self.retained.named_identity_matches()
    }

    pub fn as_source(&self) -> SourceDirectory {
        crate::read::retained_source_directory(self.retained.clone())
    }

    pub fn retain_directory_child(&self, name: &OsStr) -> io::Result<Self> {
        self.retained
            .retain_directory_child(name)
            .map(|retained| Self { retained })
    }

    pub fn create_directory_child(&self, name: &OsStr) -> io::Result<Self> {
        self.retained
            .create_directory_child(name)
            .map(|retained| Self { retained })
    }

    pub fn retain_regular_child(&self, name: &OsStr) -> io::Result<StateFile> {
        self.retained
            .retain_regular_child(name)
            .map(|retained| StateFile { retained })
    }

    pub fn retain_or_create_regular_child(&self, name: &OsStr) -> io::Result<StateFile> {
        self.retained
            .retain_or_create_regular_child(name)
            .map(|retained| StateFile { retained })
    }

    pub fn create_regular_child_atomically(
        &self,
        stage: &OsStr,
        destination: &OsStr,
        bytes: &[u8],
    ) -> Result<StateFile, RetainedPublicationFailure> {
        self.retained
            .create_regular_child_atomically(stage, destination, bytes)
            .map(|retained| StateFile { retained })
            .map_err(|error| RetainedPublicationFailure { error })
    }

    pub fn replace_regular_child_atomically(
        &self,
        stage: &OsStr,
        destination: &OsStr,
        bytes: &[u8],
    ) -> Result<StateFile, RetainedPublicationFailure> {
        self.retained
            .replace_regular_child_atomically(stage, destination, bytes)
            .map(|retained| StateFile { retained })
            .map_err(|error| RetainedPublicationFailure { error })
    }

    pub fn remove_empty_named_identity_relative(&self) -> io::Result<()> {
        self.retained.remove_empty_named_identity_relative()
    }

    pub fn verify_owner_only(&self) -> io::Result<()> {
        physical::verify_owner_only_directory(&self.retained)
    }
}

/// Opaque retained stream for the existing execution state stores. No raw
/// descriptor or conversion to a source proposal is exposed.
#[derive(Debug)]
pub struct StateHandle {
    file: std::fs::File,
}

impl StateHandle {
    pub fn metadata(&self) -> io::Result<std::fs::Metadata> {
        self.file.metadata()
    }
    pub fn sync_all(&self) -> io::Result<()> {
    let _probe = crate::probe_timing::span(2);
        self.file.sync_all()
    }
    pub fn sync_data(&self) -> io::Result<()> {
        self.file.sync_data()
    }
    pub fn set_len(&self, size: u64) -> io::Result<()> {
        self.file.set_len(size)
    }
    pub fn try_clone(&self) -> io::Result<Self> {
        self.file.try_clone().map(|file| Self { file })
    }
    pub fn try_lock_exclusive(&self) -> io::Result<()> {
        fs2::FileExt::try_lock_exclusive(&self.file)
    }
    pub fn lock_exclusive(&self) -> io::Result<()> {
        fs2::FileExt::lock_exclusive(&self.file)
    }
    pub fn unlock(&self) -> io::Result<()> {
        fs2::FileExt::unlock(&self.file)
    }
}

impl std::io::Read for StateHandle {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        std::io::Read::read(&mut self.file, bytes)
    }
}
impl std::io::Read for &StateHandle {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        std::io::Read::read(&mut &self.file, bytes)
    }
}
impl Write for StateHandle {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.file.write(bytes)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.file.flush()
    }
}
impl Write for &StateHandle {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        (&self.file).write(bytes)
    }
    fn flush(&mut self) -> io::Result<()> {
        (&self.file).flush()
    }
}
impl Seek for StateHandle {
    fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
        self.file.seek(position)
    }
}
impl Seek for &StateHandle {
    fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
        (&self.file).seek(position)
    }
}

impl StateDirectory {
    pub fn open_retained_handle(&self) -> io::Result<StateHandle> {
        self.retained
            .try_clone_directory()
            .map(|file| StateHandle { file })
    }

    pub fn visit_immediate_names<E: From<io::Error>>(
        &self,
        visitor: impl FnMut(OsString) -> Result<(), E>,
    ) -> Result<(), E> {
        self.retained.visit_immediate_names(visitor)
    }
}
impl StateFile {
    pub fn open_retained_handle(&self) -> io::Result<StateHandle> {
        self.retained
            .try_clone_file()
            .map(|file| StateHandle { file })
    }
}

/// Read an already selected service-state path with the former File::open policy.
pub fn open_state_reader(path: &Path) -> io::Result<ReadFile> {
    crate::platform::state_stream::open_state_reader(path).map(|file| ReadFile { file })
}

/// Open a state lock without truncation; the caller retains lock/recovery policy.
pub fn open_state_lock(path: &Path, create_if_absent: bool) -> io::Result<StateHandle> {
    crate::platform::state_stream::open_state_lock(path, create_if_absent)
        .map(|file| StateHandle { file })
}

/// Reserve a fresh state stage, preserving AlreadyExists and existing bytes.
pub fn create_new_state_stage(path: &Path) -> io::Result<StateHandle> {
    crate::platform::state_stream::create_new_state_stage(path).map(|file| StateHandle { file })
}

/// Create/truncate the previously selected temporary state path.
pub fn create_state_stage(path: &Path) -> io::Result<StateHandle> {
    crate::platform::state_stream::create_state_stage(path).map(|file| StateHandle { file })
}

pub fn state_file_identity(handle: &StateHandle) -> io::Result<FileIdentity> {
    filesystem::file_identity(&handle.file)
}
/// Open a caller-selected namespace root without following its final component.
/// Ancestor aliases retain the original namespace-root opening policy.
pub fn open_state_namespace_root_nofollow(path: &Path) -> io::Result<StateHandle> {
    filesystem::open_directory_nofollow(path).map(|file| StateHandle { file })
}
pub fn open_state_directory_nofollow(path: &Path) -> io::Result<StateHandle> {
    filesystem::open_absolute_directory_path_nofollow(path).map(|file| StateHandle { file })
}
pub fn open_or_create_state_directory_nofollow(path: &Path) -> io::Result<StateHandle> {
    filesystem::open_or_create_absolute_directory_path_nofollow(path)
        .map(|file| StateHandle { file })
}
pub fn state_ownership_locks_pin_their_directory() -> bool {
    filesystem::ownership_locks_pin_their_directory()
}
pub fn remove_state_directory_tree_child(parent: &StateHandle, name: &OsStr) -> io::Result<()> {
    filesystem::remove_directory_tree_child(&parent.file, name)
}
pub fn rename_state_no_replace(source: &Path, target: &Path) -> io::Result<()> {
    filesystem::rename_no_replace(source, target)
}

/// Physical identity of a state stream already owned by the caller. This
/// observes a descriptor; it neither opens a path nor grants a writer.
pub fn observed_state_file_identity(file: &std::fs::File) -> io::Result<FileIdentity> {
    filesystem::file_identity(file)
}
pub fn verify_state_owner_only(handle: &StateHandle) -> io::Result<()> {
    filesystem::verify_owner_only_acl(&handle.file)
}
pub fn sync_state_directory(handle: &StateHandle) -> io::Result<()> {
    filesystem::sync_directory(&handle.file)
}
pub fn restrict_state_stage_to_owner(handle: &StateHandle) -> io::Result<()> {
    filesystem::restrict_stage_to_owner(&handle.file)
}

pub fn create_state_directory_child(parent: &StateHandle, name: &OsStr) -> io::Result<StateHandle> {
    filesystem::create_owner_only_directory_child(&parent.file, name)
        .map(|file| StateHandle { file })
}

pub fn create_state_file_child(parent: &StateHandle, name: &OsStr) -> io::Result<StateHandle> {
    let _probe = crate::probe_timing::span(5);
    filesystem::create_owner_only_file_child(&parent.file, name).map(|file| StateHandle { file })
}

pub fn open_state_directory_child(parent: &StateHandle, name: &OsStr) -> io::Result<StateHandle> {
    filesystem::open_directory_child_nofollow(&parent.file, name).map(|file| StateHandle { file })
}

pub fn open_state_regular_child(parent: &StateHandle, name: &OsStr) -> io::Result<StateHandle> {
    filesystem::open_regular_child_nofollow(&parent.file, name).map(|file| StateHandle { file })
}

pub fn open_state_ownership_lock(parent: &StateHandle, name: &OsStr) -> io::Result<StateHandle> {
    filesystem::open_directory_ownership_lock(&parent.file, name).map(|file| StateHandle { file })
}

pub fn remove_state_regular_child(
    parent: &StateHandle,
    name: &OsStr,
    identity: FileIdentity,
    retained: &StateHandle,
) -> io::Result<()> {
    let _probe = crate::probe_timing::span(6);
    filesystem::remove_identity_bound_regular_child(&parent.file, name, identity, &retained.file)
}
pub fn replace_state_regular_child(
    parent: &StateHandle,
    source: &OsStr,
    identity: FileIdentity,
    retained: &StateHandle,
    destination: &OsStr,
) -> io::Result<()> {
    filesystem::replace_identity_bound_regular_child(
        &parent.file,
        source,
        identity,
        &retained.file,
        destination,
    )
}
pub fn rename_state_regular_child_no_replace(
    source_parent: &StateHandle,
    source: &OsStr,
    identity: FileIdentity,
    retained: &StateHandle,
    destination_parent: &StateHandle,
    destination: &OsStr,
) -> io::Result<()> {
    let _probe = crate::probe_timing::span(7);
    filesystem::rename_identity_bound_regular_child_no_replace(
        &source_parent.file,
        source,
        identity,
        &retained.file,
        &destination_parent.file,
        destination,
    )
}

#[derive(Debug)]
pub struct StateFile {
    retained: filesystem::RetainedRegularFileCapability,
}

impl StateFile {
    pub fn identity(&self) -> FileIdentity {
        self.retained.identity()
    }

    pub fn read_bounded(&self, maximum_bytes: usize) -> io::Result<Vec<u8>> {
        self.retained.read_bounded(maximum_bytes)
    }

    pub fn hard_link_count(&self) -> io::Result<u64> {
        self.retained.hard_link_count()
    }

    pub fn validate_named_identity(&self) -> io::Result<()> {
        self.retained.validate_named_identity()
    }

    pub fn validate_named_identity_relative(&self) -> io::Result<()> {
        self.retained.validate_named_identity_relative()
    }

    pub fn remove_named_identity(&self) -> io::Result<()> {
        self.retained.remove_named_identity()
    }

    pub fn remove_named_identity_relative(&self) -> io::Result<()> {
        self.retained.remove_named_identity_relative()
    }

    pub fn try_clone_file(&self) -> io::Result<ReadFile> {
        self.retained.try_clone_file().map(|file| ReadFile { file })
    }

    pub fn open_named_identity_for_read(&self) -> io::Result<ReadFile> {
        self.retained
            .open_named_identity_for_read()
            .map(|file| ReadFile { file })
    }

    /// The runtime's existing retained-log rewrite, without a descriptor escape
    /// or an extra synchronization step.
    pub fn overwrite_retained(&self, bytes: &[u8]) -> io::Result<()> {
        let mut file = self.retained.try_clone_file()?;
        file.set_len(0)?;
        file.seek(SeekFrom::Start(0))?;
        file.write_all(bytes)
    }

    pub fn lock_exclusive(&self, nonblocking: bool) -> Result<HeldStateLock, StateLockError> {
        let file = self
            .retained
            .try_clone_file()
            .map_err(StateLockError::Clone)?;
        if nonblocking {
            fs2::FileExt::try_lock_exclusive(&file).map_err(StateLockError::Lock)?;
        } else {
            fs2::FileExt::lock_exclusive(&file).map_err(StateLockError::Lock)?;
        }
        Ok(HeldStateLock { file })
    }
}

/// Keep descriptor-clone failure distinct from lock acquisition failure so
/// the runtime owner preserves its existing error and contention mapping.
#[derive(Debug)]
pub enum StateLockError {
    Clone(io::Error),
    Lock(io::Error),
}

#[derive(Debug)]
pub struct HeldStateLock {
    file: std::fs::File,
}

impl Drop for HeldStateLock {
    fn drop(&mut self) {
        let _ = fs2::FileExt::unlock(&self.file);
    }
}

#[derive(Debug)]
pub struct RetainedPublicationFailure {
    error: filesystem::RetainedPublicationError,
}

impl RetainedPublicationFailure {
    pub fn into_parts(self) -> (io::Error, Option<StateFile>) {
        let (error, published) = self.error.into_parts();
        (error, published.map(|retained| StateFile { retained }))
    }

    pub fn io_error(&self) -> &io::Error {
        self.error.io_error()
    }
}

impl std::fmt::Display for RetainedPublicationFailure {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.error.fmt(formatter)
    }
}

impl std::error::Error for RetainedPublicationFailure {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.error)
    }
}

pub fn check_durability_support() -> io::Result<()> {
    physical::check_durability_support()
}

pub fn verify_owner_only_directory(directory: &StateDirectory) -> io::Result<()> {
    physical::verify_owner_only_directory(&directory.retained)
}

pub fn create_owner_only_directory(
    parent: &StateDirectory,
    name: &OsStr,
) -> io::Result<StateDirectory> {
    physical::create_owner_only_directory(&parent.retained, name)
        .map(|retained| StateDirectory { retained })
}

pub fn flush_directory(directory: &StateDirectory) -> io::Result<()> {
    physical::flush_directory(&directory.retained)
}

#[derive(Debug)]
pub struct Stage(physical::Stage);

impl Stage {
    pub fn create(parent: &StateDirectory, name: &OsStr) -> io::Result<Self> {
        physical::Stage::create(&parent.retained, name).map(Self)
    }

    pub fn seal(self) -> io::Result<SealedStage> {
        self.0.seal().map(SealedStage)
    }
}

impl Write for Stage {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0.write(bytes)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.0.flush()
    }
}

#[derive(Debug)]
pub struct SealedStage(physical::SealedStage);

impl SealedStage {
    pub fn publish(
        self,
        destination: &OsStr,
        mode: PublishMode,
    ) -> Result<StateFile, PublicationFailure> {
        self.0
            .publish(destination, mode)
            .map(|retained| StateFile { retained })
    }
}

#[derive(Debug)]
pub struct StableLock(physical::StableLock);

impl StableLock {
    pub fn acquire_existing(parent: &StateDirectory, name: &OsStr) -> io::Result<Self> {
        physical::StableLock::acquire_existing(&parent.retained, name).map(Self)
    }

    pub fn identity(&self) -> FileIdentity {
        self.0.identity()
    }

    pub fn validate(&self) -> io::Result<()> {
        self.0.validate()
    }
}

#[derive(Debug)]
pub struct DirectoryCursor(physical::DirectoryCursor);

impl DirectoryCursor {
    pub fn open(directory: &StateDirectory) -> io::Result<Self> {
        physical::DirectoryCursor::open(&directory.retained).map(Self)
    }

    pub fn next_name(&mut self) -> io::Result<Option<OsString>> {
        self.0.next_name()
    }
}

pub use filesystem::{
    is_file_lock_contended, replace_file_atomically, short_private_runtime_dir,
    sync_parent_directory,
};

/// Existing physical-publication fault fixtures for execution integration tests.
#[cfg(any(test, feature = "test-support"))]
pub mod test_support {
    pub fn state_handle_from_fixture_file(file: std::fs::File) -> super::StateHandle {
        super::StateHandle { file }
    }
    pub fn create_file_symlink_for_test(
        source: impl AsRef<std::path::Path>,
        target: impl AsRef<std::path::Path>,
    ) -> Option<std::io::Result<()>> {
        crate::platform::filesystem::create_file_symlink_for_test(source, target)
    }
    pub fn set_before_identity_bound_cleanup_mutation_hook(hook: impl FnOnce() + 'static) {
        crate::platform::filesystem::set_before_identity_bound_cleanup_mutation_hook(hook)
    }
    pub fn set_before_identity_bound_no_replace_rename_hook(hook: impl FnOnce() + 'static) {
        crate::platform::filesystem::set_before_identity_bound_no_replace_rename_hook(hook)
    }

    pub use crate::platform::filesystem::execution_store::{
        install_publication_hook, PublicationHookGuard, PublicationPoint,
    };
    pub use crate::platform::test_support::{install_stage_write_limit, StageWriteLimitGuard};

    pub fn file_identity(file: &std::fs::File) -> std::io::Result<super::FileIdentity> {
        crate::platform::filesystem::file_identity(file)
    }
}
