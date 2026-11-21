use super::{
    create_owner_only_directory_child, FileIdentity, RetainedChildCapability,
    RetainedDirectoryCapability, RetainedDirectoryCapabilityInner, RetainedDirectoryParent,
};
use std::ffi::{OsStr, OsString};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;

const QUARANTINE_NAME: &str = ".fetch-cleanup";

#[derive(Debug)]
pub struct StateTreeRemovalError {
    error: io::Error,
    preserved_path: Option<PathBuf>,
}

impl StateTreeRemovalError {
    pub(crate) fn new(error: io::Error, retained: &RetainedDirectoryCapability) -> Self {
        Self {
            error,
            preserved_path: retained
                .validate_named_identity()
                .ok()
                .map(|()| retained.path().to_path_buf()),
        }
    }

    pub fn preserved_path(&self) -> Option<&Path> {
        self.preserved_path.as_deref()
    }
}

impl From<io::Error> for StateTreeRemovalError {
    fn from(error: io::Error) -> Self {
        Self {
            error,
            preserved_path: None,
        }
    }
}

impl std::fmt::Display for StateTreeRemovalError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.error.fmt(formatter)
    }
}

impl std::error::Error for StateTreeRemovalError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.error)
    }
}

pub(crate) fn remove(
    parent: &RetainedDirectoryCapability,
    name: &OsStr,
    expected: &RetainedDirectoryCapability,
) -> Result<(), StateTreeRemovalError> {
    parent
        .validate_named_identity()
        .and_then(|()| expected.validate_named_identity())
        .map_err(|error| StateTreeRemovalError::new(error, expected))?;

    let quarantine =
        open_quarantine(parent).map_err(|error| StateTreeRemovalError::new(error, expected))?;
    let payload_name = private_name();
    let payload = match parent.move_directory_child_no_replace_observed(
        name,
        expected,
        &quarantine,
        &payload_name,
    ) {
        Ok(payload) => payload,
        Err(failure) => {
            let (error, artifact) = failure.into_parts();
            if let Some((artifact, _, _)) = artifact {
                if artifact.identity() == expected.identity() {
                    return Err(StateTreeRemovalError::new(error, &artifact));
                }
                // An observed successor belongs to someone else. Leave its
                // containing quarantine alone when restoration was uncertain.
                return Err(StateTreeRemovalError::new(error, expected));
            }
            return Err(StateTreeRemovalError::new(error, expected));
        }
    };

    // A stage's creation descriptor on Windows cannot enumerate its entries.
    // Reopening the already-moved directory does not confer new authority.
    let result = quarantine
        .retain_directory_child(&payload_name)
        .and_then(|reopened| {
            if reopened.identity() != payload.identity() {
                return Err(io::Error::other(
                    "retained cleanup directory changed after moving",
                ));
            }
            remove_quarantined_tree(reopened)
        });
    result.map_err(|error| StateTreeRemovalError::new(error, &payload))
}

fn open_quarantine(
    parent: &RetainedDirectoryCapability,
) -> io::Result<RetainedDirectoryCapability> {
    let name = OsStr::new(QUARANTINE_NAME);
    let quarantine = match create_quarantine(parent, name) {
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
            parent.retain_directory_child(name)?
        }
        result => result?,
    };
    quarantine.validate_named_identity()?;
    super::verify_owner_only_acl(&quarantine.retained.directory)?;
    Ok(quarantine)
}

fn create_quarantine(
    parent: &RetainedDirectoryCapability,
    name: &OsStr,
) -> io::Result<RetainedDirectoryCapability> {
    let directory = create_owner_only_directory_child(&parent.retained.directory, name)?;
    let identity = super::file_identity(&directory)?;
    Ok(RetainedDirectoryCapability {
        path: parent.path().join(name),
        retained: Arc::new(RetainedDirectoryCapabilityInner {
            directory,
            identity,
        }),
        parent: Some(Arc::new(RetainedDirectoryParent {
            directory: parent.clone(),
            name: name.to_os_string(),
        })),
    })
}

struct OpenDirectory {
    directory: RetainedDirectoryCapability,
    children: std::vec::IntoIter<(OsString, FileIdentity)>,
}

impl OpenDirectory {
    fn open(directory: RetainedDirectoryCapability) -> io::Result<Self> {
        directory.validate_named_identity()?;
        let mut children = Vec::new();
        directory.visit_immediate_names(|name| {
            let child = directory.retain_immediate_child_nofollow(&name)?;
            let identity = match child {
                RetainedChildCapability::Directory(child) => child.identity(),
                RetainedChildCapability::RegularFile(file) => file.identity,
                RetainedChildCapability::ReparsePoint | RetainedChildCapability::Unsupported => {
                    return Err(io::Error::other(format!(
                        "unexpected entry {} in the retained cleanup directory",
                        Path::new(&name).display()
                    )));
                }
            };
            children.push((name, identity));
            Ok::<_, io::Error>(())
        })?;
        Ok(Self {
            directory,
            children: children.into_iter(),
        })
    }
}

fn remove_quarantined_tree(directory: RetainedDirectoryCapability) -> io::Result<()> {
    let mut parents = vec![OpenDirectory::open(directory)?];
    while let Some(parent) = parents.last_mut() {
        parent.directory.validate_named_identity()?;
        let Some((name, expected)) = parent.children.next() else {
            parent
                .directory
                .remove_empty_named_identity_relative_for_retained_apply(&private_name())?;
            parents.pop();
            continue;
        };
        match parent.directory.retain_immediate_child_nofollow(&name)? {
            RetainedChildCapability::Directory(child) if child.identity() == expected => {
                parents.push(OpenDirectory::open(child)?);
            }
            RetainedChildCapability::RegularFile(file) if file.identity == expected => {
                file.remove_named_identity_relative_for_retained_apply(&private_name())?;
            }
            _ => {
                return Err(io::Error::other(format!(
                    "entry {} changed in the retained cleanup directory",
                    Path::new(&name).display()
                )));
            }
        }
    }
    Ok(())
}

fn private_name() -> OsString {
    format!(".cleanup-{}", uuid::Uuid::new_v4().simple()).into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::platform::filesystem::set_after_retained_directory_move_hook;
    use std::fs;

    fn fixture() -> (
        tempfile::TempDir,
        RetainedDirectoryCapability,
        RetainedDirectoryCapability,
    ) {
        let temp = tempfile::tempdir().unwrap();
        let root = fs::canonicalize(temp.path()).unwrap();
        fs::create_dir_all(root.join("snapshot/nested")).unwrap();
        fs::write(root.join("snapshot/nested/Module.bsl"), b"module").unwrap();
        let parent = RetainedDirectoryCapability::open(&root).unwrap();
        let snapshot = parent
            .retain_directory_child(OsStr::new("snapshot"))
            .unwrap();
        (temp, parent, snapshot)
    }

    #[test]
    fn a_successor_at_the_public_name_survives_quarantined_cleanup() {
        let (_temp, parent, snapshot) = fixture();
        let public = snapshot.path().to_path_buf();
        let successor = public.clone();
        set_after_retained_directory_move_hook(move || {
            fs::create_dir(&successor)?;
            fs::write(successor.join("foreign.txt"), b"keep")
        });

        remove(&parent, OsStr::new("snapshot"), &snapshot).unwrap();

        assert_eq!(fs::read(public.join("foreign.txt")).unwrap(), b"keep");
        assert_eq!(
            fs::read_dir(parent.path().join(QUARANTINE_NAME))
                .unwrap()
                .count(),
            0
        );
    }

    #[test]
    fn a_late_failure_reports_the_verified_quarantine_without_deleting_the_tree() {
        let (_temp, parent, snapshot) = fixture();
        set_after_retained_directory_move_hook(|| Err(io::Error::other("observation failed")));

        let error = remove(&parent, OsStr::new("snapshot"), &snapshot).unwrap_err();

        let location = error.preserved_path().unwrap();
        assert_ne!(location, snapshot.path());
        assert!(location.starts_with(parent.path()));
        assert!(!snapshot.path().exists());
        assert_eq!(
            fs::read(location.join("nested/Module.bsl")).unwrap(),
            b"module"
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let private = location.parent().unwrap();
            assert_eq!(
                fs::metadata(private).unwrap().permissions().mode() & 0o077,
                0
            );
        }
    }

    #[test]
    fn a_stale_admission_cannot_remove_a_foreign_directory_or_report_its_path() {
        let (_temp, parent, snapshot) = fixture();
        let original = parent.path().join("original");
        fs::rename(snapshot.path(), &original).unwrap();
        fs::create_dir(snapshot.path()).unwrap();
        fs::write(snapshot.path().join("foreign.txt"), b"keep").unwrap();

        let error = remove(&parent, OsStr::new("snapshot"), &snapshot).unwrap_err();

        assert!(error.preserved_path().is_none());
        assert_eq!(
            fs::read(snapshot.path().join("foreign.txt")).unwrap(),
            b"keep"
        );
        assert_eq!(
            fs::read(original.join("nested/Module.bsl")).unwrap(),
            b"module"
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_shared_quarantine_is_refused_without_moving_the_snapshot() {
        use std::os::unix::fs::PermissionsExt;

        let (_temp, parent, snapshot) = fixture();
        let location = parent.path().join(QUARANTINE_NAME);
        fs::create_dir(&location).unwrap();
        fs::set_permissions(&location, fs::Permissions::from_mode(0o777)).unwrap();
        fs::write(location.join("foreign.txt"), b"keep").unwrap();

        let error = remove(&parent, OsStr::new("snapshot"), &snapshot).unwrap_err();

        assert_eq!(error.preserved_path(), Some(snapshot.path()));
        assert_eq!(
            fs::read(snapshot.path().join("nested/Module.bsl")).unwrap(),
            b"module"
        );
        assert_eq!(fs::read(location.join("foreign.txt")).unwrap(), b"keep");
    }
}
