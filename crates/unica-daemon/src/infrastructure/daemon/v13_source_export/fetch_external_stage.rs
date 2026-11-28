//! A set at the workspace root contains the public snapshot directory. Give the
//! provider a private, disjoint directory and copy the completed export only.
use super::storage::optional_child;
use super::*;
use std::io::{self, Read, Write};
use unica_workspace::paths::host_path_relative_to;
use unica_workspace::state_fs::{
    create_state_directory_child, create_state_file_child, state_file_identity,
    verify_state_owner_only,
};

pub(super) fn export<T>(
    root: &Path,
    id: &str,
    storage: &SnapshotStorage,
    paths: &input_paths::RunnerPaths,
    public: &StateDirectory,
    cancellation: &CancellationToken,
    run: impl FnOnce(&StateDirectory) -> Result<T, DomainResult>,
) -> Result<T, DomainResult> {
    let scratch = Scratch::create(root, id, paths).map_err(staging_error)?;
    let record = scratch
        .record(root)
        .and_then(|bytes| storage.remember_external(id, &bytes));
    let result = record
        .as_ref()
        .map_err(|error| staging_error(io::Error::other(error.to_string())))
        .and_then(|_| run(&scratch.payload))
        .and_then(|receipt| {
            copy_tree(&scratch.payload, public, cancellation).map_err(|error| {
                if cancellation.is_cancelled() {
                    reject_fetch(
                        RefusalCode::Cancelled,
                        "fetch cancelled while copying the completed snapshot",
                    )
                } else {
                    staging_error(error)
                }
            })?;
            Ok(receipt)
        });
    let expected = scratch.payload.identity().stable_bytes();
    drop(scratch.payload);
    let cleanup = cleanup_owner(&scratch.owner, expected).and_then(|_| {
        if let Ok(record) = record {
            record
                .remove_named_identity()
                .map_err(|error| CleanupFailure::ownership(error, None))?;
        }
        Ok(())
    });
    match (result, cleanup) {
        (result, Ok(())) => result,
        (result, Err(error)) => {
            let mut refused = result.err().unwrap_or_else(|| {
                reject_fetch(
                    cleanup_refusal_code(&error),
                    format!("fetch could not remove its temporary export ({error})"),
                )
            });
            refused.warnings.push(snapshot_cleanup_warning(
                root,
                id,
                "fetch could not finish removing its temporary export",
                &error,
            ));
            Err(refused)
        }
    }
}

fn staging_error(error: io::Error) -> DomainResult {
    reject_fetch(
        RefusalCode::InvalidState,
        format!("fetch could not stage the root source set: {error}"),
    )
}

struct Scratch {
    parent: StateDirectory,
    owner: StateDirectory,
    payload: StateDirectory,
}

impl Scratch {
    fn create(root: &Path, id: &str, paths: &input_paths::RunnerPaths) -> io::Result<Self> {
        let excluded = paths
            .protected_directories(root)
            .map_err(io::Error::other)?;
        let name = format!("unica-fetch-{id}");
        let mut parents = vec![std::env::temp_dir()];
        for variable in ["TMPDIR", "TMP", "TEMP"] {
            parents.extend(std::env::var_os(variable).map(PathBuf::from));
        }
        #[cfg(windows)]
        parents
            .extend(std::env::var_os("LOCALAPPDATA").map(|path| PathBuf::from(path).join("Temp")));
        #[cfg(unix)]
        parents.push(PathBuf::from("/var/tmp"));
        let mut reason = io::Error::other(
            "no writable temporary directory outside the sources, runner state and file infobase",
        );
        for parent in parents {
            let Ok(parent) = normalize_path_identity(&parent) else {
                continue;
            };
            let candidate = parent.join(&name);
            if excluded.iter().any(|path| {
                host_path_relative_to(&candidate, path).is_some()
                    || host_path_relative_to(path, &candidate).is_some()
            }) {
                continue;
            }
            let opened = StateDirectory::open(&parent).and_then(|parent| {
                let owner = private_child(&parent, OsStr::new(&name))?;
                Ok((parent, owner))
            });
            let (parent, owner) = match opened {
                Ok(pair) => pair,
                Err(error) => {
                    reason = error;
                    continue;
                }
            };
            let payload = private_child(&owner, OsStr::new("stage"));
            return match payload {
                Ok(payload) => Ok(Self {
                    parent,
                    owner,
                    payload,
                }),
                Err(error) => {
                    let cleanup = owner.remove_empty_named_identity_relative();
                    Err(cleanup.err().unwrap_or(error))
                }
            };
        }
        Err(reason)
    }

    fn record(&self, root: &Path) -> io::Result<Vec<u8>> {
        self.payload.validate_named_identity()?;
        Ok(serde_json::to_vec(&Ownership {
            workspace: StateDirectory::open(root)?.identity().stable_bytes(),
            parent: self.parent.path().to_path_buf(),
            parent_identity: self.parent.identity().stable_bytes(),
            owner: self.owner.identity().stable_bytes(),
            payload: self.payload.identity().stable_bytes(),
        })?)
    }
}

#[derive(serde::Serialize, serde::Deserialize)]
struct Ownership {
    workspace: [u8; 16],
    parent: PathBuf,
    parent_identity: [u8; 16],
    owner: [u8; 16],
    payload: [u8; 16],
}

/// The record grants access to one generated name and one particular directory.
/// Moving a different directory into its place never grants cleanup authority.
pub(super) fn resume(root: &Path, id: &str, record: &StateFile) -> Result<bool, CleanupFailure> {
    let unknown = |error| CleanupFailure::ownership(error, None);
    record.validate_named_identity().map_err(unknown)?;
    let bytes = record.read_bounded(64 * 1024).map_err(unknown)?;
    let receipt: Ownership =
        serde_json::from_slice(&bytes).map_err(|e| unknown(io::Error::other(e)))?;
    if !receipt.parent.is_absolute()
        || StateDirectory::open(root)
            .map_err(unknown)?
            .identity()
            .stable_bytes()
            != receipt.workspace
    {
        return Err(unknown(io::Error::other(
            "temporary export belongs to a different workspace",
        )));
    }
    let parent = StateDirectory::open(&receipt.parent).map_err(unknown)?;
    if parent.identity().stable_bytes() != receipt.parent_identity {
        return Err(unknown(io::Error::other("temporary export parent changed")));
    }
    let name = format!("unica-fetch-{id}");
    let owner =
        optional_child(parent.retain_directory_child(OsStr::new(&name))).map_err(unknown)?;
    if let Some(owner) = &owner {
        if owner.identity().stable_bytes() != receipt.owner {
            return Err(unknown(io::Error::other(
                "temporary export ownership changed",
            )));
        }
        verify_state_owner_only(&owner.open_retained_handle().map_err(unknown)?)
            .map_err(unknown)?;
        cleanup_owner(owner, receipt.payload)?;
    }
    record.remove_named_identity().map_err(unknown)?;
    Ok(owner.is_some())
}

fn cleanup_owner(owner: &StateDirectory, expected: [u8; 16]) -> Result<(), CleanupFailure> {
    let failure = |error| CleanupFailure::ownership(error, Some(owner));
    owner.validate_named_identity().map_err(failure)?;
    let name = OsStr::new("stage");
    let public = optional_child(owner.retain_directory_child(name)).map_err(failure)?;
    let quarantine = optional_child(owner.retain_cleanup_area(false)).map_err(failure)?;
    let moved = match &quarantine {
        Some(area) => optional_child(area.retain_directory_child(name)).map_err(failure)?,
        None => None,
    };
    if public.is_some() && moved.is_some() {
        return Err(failure(io::Error::other(
            "two directories claim the temporary export",
        )));
    }
    if let Some(stage) = moved.as_ref().or(public.as_ref()) {
        if stage.identity().stable_bytes() != expected {
            return Err(failure(io::Error::other("temporary export stage changed")));
        }
        if moved.is_some() {
            owner.remove_quarantined_directory_child(name, stage)
        } else {
            owner.remove_retained_directory_tree_child(name, stage)
        }
        .map_err(CleanupFailure::Tree)?;
    }
    drop(public);
    drop(moved);
    drop(quarantine);
    if let Some(area) = optional_child(owner.retain_cleanup_area(false)).map_err(failure)? {
        area.remove_empty_named_identity_relative()
            .map_err(failure)?;
    }
    owner
        .remove_empty_named_identity_relative()
        .map_err(failure)
}

fn private_child(parent: &StateDirectory, name: &OsStr) -> io::Result<StateDirectory> {
    let handle = create_state_directory_child(&parent.open_retained_handle()?, name)?;
    let retained = parent.retain_directory_child(name)?;
    if retained.identity() != state_file_identity(&handle)? {
        return Err(io::Error::other(
            "temporary directory changed during creation",
        ));
    }
    if let Err(error) = verify_state_owner_only(&handle) {
        retained.remove_empty_named_identity_relative()?;
        return Err(error);
    }
    Ok(retained)
}

fn copy_tree(
    from: &StateDirectory,
    to: &StateDirectory,
    cancellation: &CancellationToken,
) -> io::Result<()> {
    let mut stack = vec![CopyDirectory::open(from.clone(), to.clone())?];
    let mut buffer = vec![0; 64 * 1024];
    while let Some(directory) = stack.last_mut() {
        checkpoint(cancellation)?;
        let Some(name) = directory.entries.next_name()? else {
            directory.source.validate_named_identity()?;
            directory.destination.validate_named_identity()?;
            stack.pop();
            continue;
        };
        if let Ok(file) = directory.source.retain_regular_child(&name) {
            copy_file(
                &file,
                &directory.destination,
                &name,
                &mut buffer,
                cancellation,
            )?;
        } else {
            let source = directory.source.retain_directory_child(&name)?;
            let destination = directory.destination.create_directory_child(&name)?;
            stack.push(CopyDirectory::open(source, destination)?);
        }
    }
    checkpoint(cancellation)
}

struct CopyDirectory {
    source: StateDirectory,
    destination: StateDirectory,
    entries: DirectoryCursor,
}

impl CopyDirectory {
    fn open(source: StateDirectory, destination: StateDirectory) -> io::Result<Self> {
        source.validate_named_identity()?;
        destination.validate_named_identity()?;
        let entries = DirectoryCursor::open(&source)?;
        Ok(Self {
            source,
            destination,
            entries,
        })
    }
}

fn copy_file(
    file: &StateFile,
    destination: &StateDirectory,
    name: &OsStr,
    buffer: &mut [u8],
    cancellation: &CancellationToken,
) -> io::Result<()> {
    let mut reader = file.open_retained_handle()?;
    let mut output = create_state_file_child(&destination.open_retained_handle()?, name)?;
    loop {
        checkpoint(cancellation)?;
        let size = reader.read(buffer)?;
        if size == 0 {
            break;
        }
        output.write_all(&buffer[..size])?;
    }
    file.validate_named_identity()?;
    output.sync_all()?;
    let created = state_file_identity(&output)?;
    drop(output);
    if destination.retain_regular_child(name)?.identity() != created {
        return Err(io::Error::other("snapshot destination changed during copy"));
    }
    Ok(())
}

fn checkpoint(cancellation: &CancellationToken) -> io::Result<()> {
    if cancellation.is_cancelled() {
        Err(io::Error::new(
            io::ErrorKind::Interrupted,
            "snapshot copy cancelled",
        ))
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod medium_tests {
    use super::*;
    use std::fs;

    /// rule: INV.RUNTIME.FETCH-LEAVES-THE-WORKING-COPY
    #[test]
    fn copies_a_wide_directory_with_bounded_open_handles() {
        #[cfg(unix)]
        if std::env::var_os("UNICA_FETCH_WIDE_CHILD").is_none() {
            let test = "infrastructure::daemon::v13_source_export::fetch::external_stage::medium_tests::copies_a_wide_directory_with_bounded_open_handles";
            let output = std::process::Command::new("sh")
                .args(["-c", r#"ulimit -n 64 && exec "$@""#, "fetch-test"])
                .arg(std::env::current_exe().unwrap())
                .args(["--exact", test, "--nocapture"])
                .env("UNICA_FETCH_WIDE_CHILD", "1")
                .output()
                .unwrap();
            assert!(output.status.success(), "{output:?}");
            assert!(String::from_utf8_lossy(&output.stdout).contains("1 passed"));
            return;
        }
        let source = tempfile::tempdir().unwrap();
        let destination = tempfile::tempdir().unwrap();
        for index in 0..128 {
            fs::create_dir(source.path().join(format!("Catalog{index}"))).unwrap();
        }
        let binary = [255, 0, 128, 13, 10, 7].repeat(16 * 1024);
        fs::write(source.path().join("Catalog97/resource.bin"), &binary).unwrap();
        let retain = |path: &Path| {
            StateDirectory::open(path.parent().unwrap())
                .unwrap()
                .retain_directory_child(path.file_name().unwrap())
                .unwrap()
        };
        copy_tree(
            &retain(source.path()),
            &retain(destination.path()),
            &CancellationToken::new(),
        )
        .unwrap();
        assert_eq!(fs::read_dir(destination.path()).unwrap().count(), 128);
        assert_eq!(
            fs::read_dir(destination.path().join("Catalog63"))
                .unwrap()
                .count(),
            0
        );
        assert_eq!(
            fs::read(destination.path().join("Catalog97/resource.bin")).unwrap(),
            binary
        );
    }
}
