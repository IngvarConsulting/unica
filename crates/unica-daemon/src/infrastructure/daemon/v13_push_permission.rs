//! A person's remembered force-push permission. Grants belong to the user,
//! outside the project and the build-specific daemon state, and name only a
//! digest of one resolved infobase target. They authorize no other operation.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::io::Write;
use std::path::{Path, PathBuf};
use unica_workspace::paths::{normalize_path_identity, stable_path_identity_bytes};
use unica_workspace::state_fs::{self, StateDirectory};

const DIRECTORY: &str = "push-permissions";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Target {
    pub(super) digest: String,
    pub(super) label: String,
}

impl Target {
    pub(super) fn of_workspace(root: &Path) -> Option<Self> {
        let origin = super::runner::origin_configuration(root).ok()??;
        if origin
            .get("standalone")
            .is_some_and(|value| !value.is_null())
        {
            return None;
        }
        Self::from_connection(root, origin.get("connection")?.as_str()?)
    }

    fn from_connection(root: &Path, connection: &str) -> Option<Self> {
        // Do not guess the runner's target for raw argv, substitutions or
        // ambiguous quoting. Such connections retain one-call confirmation.
        let mut parameters = BTreeMap::new();
        for part in connection
            .split(';')
            .map(str::trim)
            .filter(|p| !p.is_empty())
        {
            let (key, value) = part.split_once('=')?;
            let key = key.trim().to_ascii_lowercase();
            if !["file", "srvr", "ref", "usr", "pwd"].contains(&key.as_str()) {
                return None;
            }
            let value = value.trim();
            let value = value
                .strip_prefix('"')
                .and_then(|v| v.strip_suffix('"'))
                .unwrap_or(value);
            if value.contains(['"', '\'', '$', '%', '\n', '\r', '\0'])
                || parameters.insert(key, value).is_some()
            {
                return None;
            }
        }
        let mut hash = Sha256::new();
        hash.update(b"unica-push-target-v1\0");
        let label = match (
            parameters.get("file"),
            parameters.get("srvr"),
            parameters.get("ref"),
        ) {
            (Some(file), None, None) if !file.is_empty() => {
                let path = Path::new(file);
                let path = normalize_path_identity(&root.join(path)).ok()?;
                hash.update(b"file\0");
                hash.update(stable_path_identity_bytes(&path).ok()?);
                format!("File={}", path.display())
            }
            (None, Some(server), Some(reference))
                if !server.is_empty() && !reference.is_empty() =>
            {
                let server = server.to_ascii_lowercase();
                hash.update(b"cluster\0");
                hash.update(server.as_bytes());
                hash.update(b"\0");
                hash.update(reference.as_bytes());
                format!("Srvr={server};Ref={reference}")
            }
            _ => return None,
        };
        let digest = hash
            .finalize()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        Some(Self { digest, label })
    }

    fn file_name(&self) -> String {
        format!("{}.json", self.digest)
    }
}

#[derive(Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct Grant {
    version: u8,
    operation: String,
    target: String,
    /// A credential-free label lets the user find and revoke this grant.
    infobase: String,
}

impl Grant {
    fn for_target(target: &Target) -> Self {
        Self {
            version: 1,
            operation: "push.force".into(),
            target: target.digest.clone(),
            infobase: target.label.clone(),
        }
    }
}

/// The root is shared by versions of Unica, not a build's collectible state.
#[derive(Debug, Clone)]
pub(super) struct Store {
    root: PathBuf,
}

impl Store {
    pub(super) fn for_user() -> Option<Self> {
        super::user_state::default_user_daemon_state_root()
            .ok()
            .map(Self::new)
    }

    pub(super) fn new(root: PathBuf) -> Self {
        Self { root }
    }

    fn directory(&self, create: bool) -> std::io::Result<StateDirectory> {
        let root = if create {
            StateDirectory::open_or_create(&self.root)?
        } else {
            StateDirectory::open(&self.root)?
        };
        let directory = match root.retain_directory_child(OsStr::new(DIRECTORY)) {
            Ok(directory) => directory,
            Err(error) if create && error.kind() == std::io::ErrorKind::NotFound => {
                match state_fs::create_state_directory_child(
                    &root.open_retained_handle()?,
                    OsStr::new(DIRECTORY),
                ) {
                    Ok(_created) => root.retain_directory_child(OsStr::new(DIRECTORY))?,
                    Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                        root.retain_directory_child(OsStr::new(DIRECTORY))?
                    }
                    Err(error) => return Err(error),
                }
            }
            Err(error) => return Err(error),
        };
        directory.verify_owner_only()?;
        Ok(directory)
    }

    /// Absence, revocation, corruption and unsafe storage all mean no grant.
    pub(super) fn permits(&self, target: &Target) -> bool {
        let read = || -> std::io::Result<bool> {
            let directory = self.directory(false)?;
            let file = directory.retain_regular_child(OsStr::new(&target.file_name()))?;
            state_fs::verify_state_owner_only(&file.open_retained_handle()?)?;
            let expected = Grant::for_target(target);
            let maximum = serde_json::to_vec(&expected)
                .expect("a grant serializes")
                .len();
            let bytes = file.read_bounded(maximum)?;
            Ok(serde_json::from_slice::<Grant>(&bytes).ok().as_ref() == Some(&expected))
        };
        read().unwrap_or(false)
    }

    pub(super) fn remember(&self, target: &Target) -> Result<(), String> {
        if self.permits(target) {
            return Ok(());
        }
        let directory = self
            .directory(true)
            .map_err(|_| "cannot open private push-permission storage")?;
        let bytes = serde_json::to_vec(&Grant::for_target(target)).expect("a grant serializes");
        let stage = format!(".{}.stage", uuid::Uuid::new_v4());
        let parent = directory
            .open_retained_handle()
            .map_err(|_| "cannot open permission directory")?;
        // Native state primitives create owner-only files on Windows and Unix;
        // general source-publication stages deliberately inherit other ACLs.
        let mut file = state_fs::create_state_file_child(&parent, OsStr::new(&stage))
            .map_err(|_| "cannot create private permission stage")?;
        let identity =
            state_fs::state_file_identity(&file).map_err(|_| "cannot identify permission stage")?;
        let mut publish = || -> std::io::Result<()> {
            file.write_all(&bytes)?;
            file.sync_all()?;
            directory.validate_named_identity()?;
            state_fs::replace_state_regular_child(
                &parent,
                OsStr::new(&stage),
                identity,
                &file,
                OsStr::new(&target.file_name()),
            )?;
            state_fs::sync_state_directory(&parent)?;
            directory.validate_named_identity()
        };
        if let Err(error) = publish() {
            let _ =
                state_fs::remove_state_regular_child(&parent, OsStr::new(&stage), identity, &file);
            return Err(format!("cannot save the force-push permission: {error}"));
        }
        if !self.permits(target) {
            return Err("cannot verify the saved force-push permission".into());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// rule: INV.RUNTIME.DESTRUCTIVE-CONFIRMATION
    #[test]
    fn a_grant_survives_reopening_but_does_not_authorize_another_target() {
        let root = tempfile::tempdir().unwrap();
        let store = Store::new(root.path().join("user-state"));
        let target = Target::from_connection(root.path(), "File=one").unwrap();
        let other = Target::from_connection(root.path(), "File=two").unwrap();
        assert!(!store.permits(&target));
        store.remember(&target).unwrap();
        let reopened = Store::new(root.path().join("user-state"));
        assert!(reopened.permits(&target));
        assert!(!reopened.permits(&other));
        // Removing this file is the user-facing revocation mechanism.
        let file = reopened
            .directory(false)
            .unwrap()
            .retain_regular_child(OsStr::new(&target.file_name()))
            .unwrap();
        file.remove_named_identity().unwrap();
        assert!(!reopened.permits(&target));
    }

    /// rule: INV.RUNTIME.DESTRUCTIVE-CONFIRMATION
    #[test]
    fn a_corrupt_grant_is_not_permission() {
        let root = tempfile::tempdir().unwrap();
        let store = Store::new(root.path().join("state"));
        let target = Target::from_connection(root.path(), "File=one").unwrap();
        store.remember(&target).unwrap();
        std::fs::write(store.root.join(DIRECTORY).join(target.file_name()), b"{}").unwrap();
        assert!(!store.permits(&target));
        store.remember(&target).unwrap();
        assert!(store.permits(&target));
    }

    /// rule: INV.RUNTIME.DESTRUCTIVE-CONFIRMATION
    #[test]
    fn target_identity_resolves_paths_and_excludes_credentials() {
        let root = tempfile::tempdir().unwrap();
        let absolute = format!(
            "File=\"{}\";Usr=Admin;Pwd=secret",
            root.path().join("base").display()
        );
        let relative = Target::from_connection(root.path(), "file=base").unwrap();
        let target = Target::from_connection(root.path(), &absolute).unwrap_or_else(|| {
            panic!(
                "unrecognized {absolute}; normalization: {:?}",
                normalize_path_identity(&root.path().join("base"))
            )
        });
        assert_eq!(relative, target);
        assert!(!target.label.contains("secret"));
        assert!(!serde_json::to_string(&Grant::for_target(&target))
            .unwrap()
            .contains("secret"));
        let cluster =
            Target::from_connection(root.path(), "Srvr=HOST:1541;Ref=demo;Pwd=secret").unwrap();
        assert_eq!(
            cluster,
            Target::from_connection(root.path(), "srvr=host:1541;ref=demo").unwrap()
        );
        assert_ne!(
            cluster,
            Target::from_connection(root.path(), "Srvr=elsewhere;Ref=demo").unwrap()
        );
        assert_ne!(
            cluster,
            Target::from_connection(root.path(), "Srvr=host:1541;Ref=other").unwrap()
        );
    }

    /// rule: INV.RUNTIME.DESTRUCTIVE-CONFIRMATION
    #[test]
    fn ambiguous_connections_have_no_remembered_permission() {
        for connection in [
            "/F base",
            "File=${BASE}",
            "File=%BASE%",
            "File=one;File=two",
            "File=one;Srvr=host;Ref=demo",
            "File=\"a;b\"",
            "Srvr=host",
            "",
            "File=base;unknown=one",
        ] {
            assert!(
                Target::from_connection(Path::new("."), connection).is_none(),
                "{connection}"
            );
        }
    }
}
