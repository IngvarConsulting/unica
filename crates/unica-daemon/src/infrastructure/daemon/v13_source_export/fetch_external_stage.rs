//! A set at the workspace root contains the public snapshot directory. Give the
//! provider a private, disjoint directory and copy the completed export only.
use super::*;
use std::io::{self, Read, Write};
use unica_workspace::paths::host_path_relative_to;
use unica_workspace::state_fs::{
    create_state_directory_child, create_state_file_child, state_file_identity,
    verify_state_owner_only,
};

pub(super) fn export<T>(
    root: &Path,
    public: &StateDirectory,
    cancellation: &CancellationToken,
    run: impl FnOnce(&StateDirectory) -> Result<T, DomainResult>,
) -> Result<T, DomainResult> {
    let scratch = Scratch::create(root).map_err(staging_error)?;
    let result = run(&scratch.payload).and_then(|receipt| {
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
    match (result, scratch.close()) {
        (result, Ok(())) => result,
        (Err(mut refused), Err(error)) => {
            refused.warnings.push(json!({"code":"fetch_temporary_cleanup_failed","message":format!("could not clean the temporary export: {error}")}));
            Err(refused)
        }
        (Ok(_), Err(error)) => Err(staging_error(error)),
    }
}

fn staging_error(error: io::Error) -> DomainResult {
    reject_fetch(
        RefusalCode::InvalidState,
        format!("fetch could not stage the root source set: {error}"),
    )
}

struct Scratch {
    owner: StateDirectory,
    payload: StateDirectory,
}

impl Scratch {
    fn create(root: &Path) -> io::Result<Self> {
        let excluded = protected_directories(root)?;
        let name = format!("unica-fetch-{}", uuid::Uuid::new_v4());
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
            let opened = StateDirectory::open(&parent)
                .and_then(|parent| private_child(&parent, OsStr::new(&name)));
            let owner = match opened {
                Ok(owner) => owner,
                Err(error) => {
                    reason = error;
                    continue;
                }
            };
            let payload = private_child(&owner, OsStr::new("stage"));
            return match payload {
                Ok(payload) => Ok(Self { owner, payload }),
                Err(error) => {
                    let cleanup = owner.remove_empty_named_identity_relative();
                    Err(cleanup.err().unwrap_or(error))
                }
            };
        }
        Err(reason)
    }

    fn close(self) -> io::Result<()> {
        self.owner
            .remove_retained_directory_tree_child(OsStr::new("stage"), &self.payload)
            .map_err(io::Error::other)?;
        drop(self.payload);
        self.owner
            .retain_cleanup_area(false)?
            .remove_empty_named_identity_relative()?;
        self.owner.remove_empty_named_identity_relative()
    }
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

fn protected_directories(root: &Path) -> io::Result<Vec<PathBuf>> {
    use super::super::super::runner::{effective_origin, file_infobase_path};
    use super::super::super::v13_workspace_bootstrap::read_yaml_config;
    let local = read_yaml_config(root, LOCAL_CONFIG_NAME).map_err(io::Error::other)?;
    let base = read_yaml_config(root, CONFIG_NAME).map_err(io::Error::other)?;
    let mut directories = vec![root.to_path_buf()];
    let mut exclude = |value: &str| -> io::Result<()> {
        directories.push(normalize_path_identity(&root.join(value)).map_err(io::Error::other)?);
        Ok(())
    };
    for (field, default) in [("workPath", ".build"), ("localPath", ".local")] {
        let value = [local.as_ref(), base.as_ref()]
            .into_iter()
            .flatten()
            .find_map(|layer| layer.get(field).and_then(serde_yaml::Value::as_str))
            .unwrap_or(default);
        exclude(value)?;
    }
    for layer in [base.as_ref(), local.as_ref()].into_iter().flatten() {
        if let Some(sets) = layer
            .get("source-set")
            .and_then(serde_yaml::Value::as_sequence)
        {
            for path in sets
                .iter()
                .filter_map(|set| set.get("path").and_then(serde_yaml::Value::as_str))
            {
                exclude(path)?;
            }
        }
    }
    let origin = effective_origin(root).map_err(io::Error::other)?;
    if let Some(file) = origin
        .as_ref()
        .and_then(|value| value.get("connection"))
        .and_then(serde_yaml::Value::as_str)
        .and_then(file_infobase_path)
    {
        exclude(&file)?;
    }
    Ok(directories)
}

fn copy_tree(
    from: &StateDirectory,
    to: &StateDirectory,
    cancellation: &CancellationToken,
) -> io::Result<()> {
    let mut pending = vec![(from.clone(), to.clone())];
    let mut buffer = vec![0; 64 * 1024];
    while let Some((source, destination)) = pending.pop() {
        source.validate_named_identity()?;
        destination.validate_named_identity()?;
        let mut entries = DirectoryCursor::open(&source)?;
        while let Some(name) = entries.next_name()? {
            checkpoint(cancellation)?;
            if let Ok(file) = source.retain_regular_child(&name) {
                let mut reader = file.open_retained_handle()?;
                let mut output =
                    create_state_file_child(&destination.open_retained_handle()?, &name)?;
                loop {
                    checkpoint(cancellation)?;
                    let size = reader.read(&mut buffer)?;
                    if size == 0 {
                        break;
                    }
                    output.write_all(&buffer[..size])?;
                }
                file.validate_named_identity()?;
                output.sync_all()?;
                let created = state_file_identity(&output)?;
                drop(output);
                if destination.retain_regular_child(&name)?.identity() != created {
                    return Err(io::Error::other("snapshot destination changed during copy"));
                }
            } else {
                let child = source.retain_directory_child(&name)?;
                let target = destination.create_directory_child(&name)?;
                pending.push((child, target));
            }
        }
        source.validate_named_identity()?;
        destination.validate_named_identity()?;
    }
    checkpoint(cancellation)
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
