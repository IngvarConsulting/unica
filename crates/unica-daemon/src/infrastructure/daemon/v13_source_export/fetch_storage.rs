//! В каталоге снимков хранятся только права на удаление, выданные успешным fetch.
use super::{valid_snapshot_id, SNAPSHOT_ROOT};
use std::ffi::OsStr;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};
use unica_work_control::cancellation::CancellationToken;
use unica_workspace::runner_lock::RunnerLockFailure;
use unica_workspace::state_fs::{
    is_file_lock_contended, open_state_ownership_lock, RetainedPublicationObservation,
    StateDirectory, StateFile, StateHandle, StateTreeRemovalError,
};

type SnapshotIdentity = [u8; 16];
const IDENTITY_LEN: usize = std::mem::size_of::<SnapshotIdentity>();

struct Retirement {
    stage: SnapshotIdentity,
    owner: Option<SnapshotIdentity>,
}

impl Retirement {
    fn bytes(&self) -> Vec<u8> {
        let mut bytes = self.stage.to_vec();
        if let Some(owner) = self.owner {
            bytes.extend(owner);
        }
        bytes
    }

    fn read(record: &StateFile) -> io::Result<Self> {
        record.validate_named_identity()?;
        let bytes = record.read_bounded(IDENTITY_LEN * 2)?;
        let (stage, rest) = bytes
            .split_at_checked(IDENTITY_LEN)
            .ok_or_else(ownership_changed)?;
        let stage = stage.try_into().expect("identity slice has checked length");
        let owner = if rest.is_empty() {
            None
        } else {
            Some(rest.try_into().map_err(|_| ownership_changed())?)
        };
        Ok(Self { stage, owner })
    }
}

pub(super) struct SnapshotStorage {
    directory: StateDirectory,
    records: Option<StateDirectory>,
}

pub(super) struct OwnedSnapshot {
    pub(super) id: String,
    directory: StateDirectory,
    ownership: StateFile,
}

pub(super) enum DropTarget {
    Ready(OwnedSnapshot),
    Retiring(StateFile),
}

#[derive(Debug)]
pub(super) enum RememberFailure {
    Rejected(io::Error, Option<StateFile>),
    Uncertain(CleanupFailure),
}

#[derive(Debug)]
pub(super) enum CleanupFailure {
    Tree(StateTreeRemovalError),
    Ownership(io::Error, Option<PathBuf>),
}

impl CleanupFailure {
    fn ownership(error: io::Error, directory: Option<&StateDirectory>) -> Self {
        let path = directory
            .filter(|directory| directory.validate_named_identity().is_ok())
            .map(|directory| directory.path().to_path_buf());
        Self::Ownership(error, path)
    }

    pub(super) fn preserved_path(&self) -> Option<&Path> {
        match self {
            Self::Tree(error) => error.preserved_path(),
            Self::Ownership(_, path) => path.as_deref(),
        }
    }
}

impl std::fmt::Display for CleanupFailure {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Tree(error) => error.fmt(formatter),
            Self::Ownership(error, _) => error.fmt(formatter),
        }
    }
}

impl OwnedSnapshot {
    pub(super) fn modified(&self) -> SystemTime {
        self.directory
            .open_retained_handle()
            .and_then(|handle| handle.metadata())
            .and_then(|metadata| metadata.modified())
            .unwrap_or(SystemTime::UNIX_EPOCH)
    }
}

impl SnapshotStorage {
    /// Удерживает каждый компонент от уже выбранного корня. Ссылки и точки
    /// повторной обработки Windows отвергаются средствами StateDirectory.
    pub(super) fn open(root: &Path, create: bool) -> io::Result<Option<Self>> {
        let mut directory = StateDirectory::open(root)?;
        for name in SNAPSHOT_ROOT.map(OsStr::new) {
            directory = match directory.retain_directory_child(name) {
                Ok(child) => child,
                Err(error) if error.kind() == io::ErrorKind::NotFound && create => {
                    match directory.create_directory_child(name) {
                        Ok(created) => {
                            // На Windows дескриптор создания не даёт читать список имён.
                            let child = directory.retain_directory_child(name)?;
                            if child.identity() != created.identity() {
                                return Err(io::Error::other(
                                    "the new fetch directory changed before it could be opened",
                                ));
                            }
                            child
                        }
                        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                            directory.retain_directory_child(name)?
                        }
                        Err(error) => return Err(error),
                    }
                }
                Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
                Err(error) => return Err(error),
            };
        }
        let records = if create {
            Some(directory.retain_cleanup_area(true)?)
        } else {
            optional_child(directory.retain_cleanup_area(false))?
        };
        Ok(Some(Self { directory, records }))
    }

    /// Одновременно один fetch или fetch.drop одной рабочей копии. Другие
    /// операции, в том числе правку исходников, эта блокировка не удерживает.
    pub(super) fn lock(
        &self,
        cancellation: &CancellationToken,
    ) -> Result<StateHandle, RunnerLockFailure> {
        let invalid = |error: io::Error| {
            RunnerLockFailure::InvalidState(format!(
                "could not lock the fetch snapshot directory: {error}"
            ))
        };
        let lock = self.open_lock().map_err(invalid)?;
        loop {
            if cancellation.is_cancelled() {
                return Err(RunnerLockFailure::Cancelled);
            }
            match lock.try_lock_exclusive() {
                Ok(()) => {
                    self.directory.validate_named_identity().map_err(invalid)?;
                    return Ok(lock);
                }
                Err(error) if is_file_lock_contended(&error) => {
                    std::thread::sleep(Duration::from_millis(10));
                }
                Err(error) => return Err(invalid(error)),
            }
        }
    }

    pub(super) fn open_lock(&self) -> io::Result<StateHandle> {
        let parent = self.directory.open_retained_handle()?;
        open_state_ownership_lock(&parent, OsStr::new(".fetch-lock"))
    }

    pub(super) fn create_stage(&self, id: &str) -> io::Result<StateDirectory> {
        self.directory.create_directory_child(OsStr::new(id))
    }

    /// Запись владения хранится в закрытом каталоге, отдельно от выгрузки.
    /// Каталог по имени UUID сам по себе не даёт право удаления.
    pub(super) fn remember(&self, id: &str, stage: &StateDirectory) -> Result<(), RememberFailure> {
        stage.validate_named_identity().map_err(|error| {
            RememberFailure::Uncertain(CleanupFailure::ownership(error, Some(stage)))
        })?;
        let pending = marker_stage_name();
        let owner = ownership_name(id);
        let records = self.require_records().map_err(|error| {
            RememberFailure::Uncertain(CleanupFailure::ownership(error, Some(stage)))
        })?;
        match records.create_regular_child_atomically(
            OsStr::new(&pending),
            OsStr::new(&owner),
            &stage.identity().stable_bytes(),
        ) {
            Ok(published) => {
                #[cfg(test)]
                AFTER_OWNER_PUBLICATION.with(|hook| {
                    if let Some(hook) = hook.take() {
                        hook();
                    }
                });
                let expected = stage.identity().stable_bytes();
                let confirm_owner = || {
                    let current = self.record(&owner)?.ok_or_else(ownership_changed)?;
                    same_record(&published, &current, &expected)
                };
                let Err(error) = confirm_owner().and_then(|()| stage.validate_named_identity())
                else {
                    return Ok(());
                };
                // Discard only the record this fetch published. A changed stage
                // name or owner is never permission to touch another tree.
                let error = confirm_owner()
                    .and_then(|()| published.remove_named_identity())
                    .err()
                    .unwrap_or(error);
                Err(RememberFailure::Uncertain(CleanupFailure::ownership(
                    error,
                    Some(stage),
                )))
            }
            Err(failure) => {
                let (error, observation) = failure.into_observation();
                let current = self.record(&owner);
                let expected: SnapshotIdentity = stage.identity().stable_bytes();
                match (observation, current) {
                    (RetainedPublicationObservation::NoArtifact, Ok(None)) => {
                        Err(RememberFailure::Rejected(error, None))
                    }
                    (RetainedPublicationObservation::Expected(retained), Ok(Some(current)))
                        if same_record(&retained, &current, &expected).is_ok() =>
                    {
                        Err(RememberFailure::Rejected(error, Some(current)))
                    }
                    _ => Err(RememberFailure::Uncertain(CleanupFailure::ownership(
                        error,
                        Some(stage),
                    ))),
                }
            }
        }
    }

    fn records(&self) -> io::Result<Option<StateDirectory>> {
        match &self.records {
            Some(records) => {
                records.validate_named_identity()?;
                Ok(Some(records.clone()))
            }
            // A concurrent fetch may have initialized storage before we
            // acquired this copy's lock.
            None => optional_child(self.directory.retain_cleanup_area(false)),
        }
    }

    fn require_records(&self) -> io::Result<StateDirectory> {
        self.records()?.ok_or_else(ownership_changed)
    }

    fn record(&self, name: &str) -> io::Result<Option<StateFile>> {
        match self.records()? {
            Some(records) => optional_child(records.retain_regular_child(OsStr::new(name))),
            None => Ok(None),
        }
    }

    pub(super) fn drop_target(&self, id: &str) -> io::Result<Option<DropTarget>> {
        if let Some(retiring) = self.record(&retirement_name(id))? {
            return Ok(Some(DropTarget::Retiring(retiring)));
        }
        self.owned_snapshot(id)
            .map(|target| target.map(DropTarget::Ready))
    }

    fn owned_snapshot(&self, id: &str) -> io::Result<Option<OwnedSnapshot>> {
        let owner = ownership_name(id);
        let Some(ownership) = self.record(&owner)? else {
            return Ok(None);
        };
        if self.record(&retirement_name(id))?.is_some() {
            return Ok(None);
        }
        let Some(directory) =
            optional_child(self.directory.retain_directory_child(OsStr::new(id)))?
        else {
            return Ok(None);
        };
        let identity = directory.identity().stable_bytes();
        if ownership.read_bounded(identity.len())?.as_slice() != identity.as_slice() {
            return Ok(None);
        }
        Ok(Some(OwnedSnapshot {
            id: id.to_string(),
            directory,
            ownership,
        }))
    }

    pub(super) fn owned_snapshots(&self) -> io::Result<Vec<OwnedSnapshot>> {
        let mut snapshots = Vec::new();
        self.directory
            .visit_immediate_names(|name| -> io::Result<()> {
                if let Some(id) = name.to_str().filter(|name| valid_snapshot_id(name)) {
                    if let Some(snapshot) = self.owned_snapshot(id)? {
                        snapshots.push(snapshot);
                    }
                }
                Ok(())
            })?;
        Ok(snapshots)
    }

    pub(super) fn remove(&self, snapshot: &OwnedSnapshot) -> Result<(), CleanupFailure> {
        self.remove_stage(&snapshot.id, &snapshot.directory, Some(&snapshot.ownership))
    }

    pub(super) fn remove_stage(
        &self,
        id: &str,
        stage: &StateDirectory,
        owner: Option<&StateFile>,
    ) -> Result<(), CleanupFailure> {
        let failure = |error| CleanupFailure::ownership(error, Some(stage));
        stage.validate_named_identity().map_err(failure)?;
        let bytes: SnapshotIdentity = stage.identity().stable_bytes();
        let current = self.record(&ownership_name(id)).map_err(failure)?;
        match (owner, current.as_ref()) {
            (Some(expected), Some(current)) => {
                same_record(expected, current, &bytes).map_err(failure)?
            }
            (None, None) => {}
            _ => return Err(failure(ownership_changed())),
        }
        // The retiring record is the last record removed. It identifies the
        // same physical directory at its original name or in quarantine.
        let retirement = Retirement {
            stage: bytes,
            owner: owner.map(|owner| owner.identity().stable_bytes()),
        };
        let retiring = self
            .require_records()
            .map_err(failure)?
            .create_regular_child_atomically(
                OsStr::new(&marker_stage_name()),
                OsStr::new(&retirement_name(id)),
                &retirement.bytes(),
            )
            .map_err(|error| failure(error.into_observation().0))?;
        self.remove_retiring(id, &retiring)
    }

    pub(super) fn remove_retiring(
        &self,
        id: &str,
        retiring: &StateFile,
    ) -> Result<(), CleanupFailure> {
        let unknown = |error| CleanupFailure::ownership(error, None);
        let expected = Retirement::read(retiring).map_err(unknown)?;
        let name = OsStr::new(id);
        let quarantined = self
            .require_records()
            .and_then(|records| optional_child(records.retain_directory_child(name)))
            .map_err(|error| {
                let public = self.directory.retain_directory_child(name).ok();
                let public = public
                    .as_ref()
                    .filter(|directory| directory.identity().stable_bytes() == expected.stage);
                CleanupFailure::ownership(error, public)
            })?;
        let stage = match quarantined {
            Some(directory) => Some((directory, true)),
            None => optional_child(self.directory.retain_directory_child(name))
                .map_err(unknown)?
                .map(|directory| (directory, false)),
        };
        let verified = stage.as_ref().and_then(|(directory, _)| {
            (directory.identity().stable_bytes() == expected.stage).then_some(directory)
        });
        let failure = |error| CleanupFailure::ownership(error, verified);
        if stage.is_some() && verified.is_none() {
            return Err(failure(ownership_changed()));
        }
        if let Some(owner) = self.record(&ownership_name(id)).map_err(failure)? {
            if Some(owner.identity().stable_bytes()) != expected.owner
                || read_record(&owner).map_err(failure)? != expected.stage
            {
                return Err(failure(ownership_changed()));
            }
            owner.remove_named_identity().map_err(failure)?;
        }
        retiring.validate_named_identity().map_err(failure)?;
        if let Some((directory, quarantined)) = &stage {
            if *quarantined {
                self.directory
                    .remove_quarantined_directory_child(name, directory)
            } else {
                self.directory
                    .remove_retained_directory_tree_child(name, directory)
            }
            .map_err(CleanupFailure::Tree)?;
        }
        retiring.remove_named_identity().map_err(unknown)
    }
}

fn read_record(file: &StateFile) -> io::Result<SnapshotIdentity> {
    file.validate_named_identity()?;
    file.read_bounded(IDENTITY_LEN)?
        .try_into()
        .map_err(|_| ownership_changed())
}

fn same_record(
    expected: &StateFile,
    observed: &StateFile,
    bytes: &SnapshotIdentity,
) -> io::Result<()> {
    if expected.identity() != observed.identity() || read_record(observed)? != *bytes {
        return Err(ownership_changed());
    }
    Ok(())
}

#[cfg(test)]
thread_local! {
    pub(super) static AFTER_OWNER_PUBLICATION: std::cell::Cell<Option<Box<dyn FnOnce()>>> =
        std::cell::Cell::new(None);
}

fn ownership_changed() -> io::Error {
    io::Error::other("fetch snapshot ownership could not be confirmed")
}

fn optional_child<T>(result: io::Result<T>) -> io::Result<Option<T>> {
    match result {
        Ok(child) => Ok(Some(child)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

fn ownership_name(id: &str) -> String {
    format!("{id}.owner")
}

fn retirement_name(id: &str) -> String {
    format!("{id}.retiring")
}

fn marker_stage_name() -> String {
    format!(".marker-stage-{}", uuid::Uuid::new_v4().simple())
}
