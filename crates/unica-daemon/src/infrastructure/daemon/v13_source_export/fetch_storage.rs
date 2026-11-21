//! В каталоге снимков хранятся только права на удаление, выданные успешным fetch.
use super::{valid_snapshot_id, SNAPSHOT_ROOT};
use std::ffi::OsStr;
use std::io;
use std::path::Path;
use std::time::{Duration, SystemTime};
use unica_work_control::cancellation::CancellationToken;
use unica_workspace::runner_lock::RunnerLockFailure;
use unica_workspace::state_fs::{
    is_file_lock_contended, open_state_ownership_lock, remove_state_directory_tree_child,
    StateDirectory, StateFile, StateHandle,
};

pub(super) struct SnapshotStorage {
    directory: StateDirectory,
}

pub(super) struct OwnedSnapshot {
    pub(super) id: String,
    directory: StateDirectory,
    ownership: StateFile,
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
        Ok(Some(Self { directory }))
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

    /// Запись владения находится рядом со снимком. В самом снимке остаются
    /// только файлы выгрузки, и каталог по имени UUID ещё не даёт право удаления.
    pub(super) fn remember(&self, id: &str, stage: &StateDirectory) -> io::Result<()> {
        stage.validate_named_identity()?;
        let pending = format!(".{id}.owner-stage");
        let owner = ownership_name(id);
        self.directory
            .create_regular_child_atomically(
                OsStr::new(&pending),
                OsStr::new(&owner),
                &stage.identity().stable_bytes(),
            )
            .map(|_| ())
            .map_err(|error| error.into_parts().0)
    }

    pub(super) fn owned_snapshot(&self, id: &str) -> io::Result<Option<OwnedSnapshot>> {
        let owner = ownership_name(id);
        let Some(ownership) =
            optional_child(self.directory.retain_regular_child(OsStr::new(&owner)))?
        else {
            return Ok(None);
        };
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

    pub(super) fn remove(&self, snapshot: &OwnedSnapshot) -> io::Result<()> {
        snapshot.ownership.validate_named_identity()?;
        self.remove_stage(&snapshot.id, &snapshot.directory)?;
        snapshot.ownership.remove_named_identity_relative()
    }

    /// Только созданный либо подтверждённый каталог. Родитель удерживается;
    /// удаление не идёт по подставленным путям рабочей копии.
    pub(super) fn remove_stage(&self, id: &str, stage: &StateDirectory) -> io::Result<()> {
        let parent = self.directory.open_retained_handle()?;
        let mut retries = 9;
        loop {
            match stage.validate_named_identity() {
                Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
                result => result?,
            }
            match remove_state_directory_tree_child(&parent, OsStr::new(id)) {
                Ok(()) => return Ok(()),
                Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
                Err(error) if retries == 0 => return Err(error),
                Err(_) => {
                    retries -= 1;
                    std::thread::sleep(Duration::from_millis(500));
                }
            }
        }
    }
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
