//! BSL-only failure-atomic publication transaction.
//!
//! Platform XML mutation transactions live in `unica-adapter-platform-xml`.
//! This host transaction exists solely for `unica.code.patch` and binds
//! adapter-provided source-owner evidence before publishing one BSL module.

use super::single_file_publisher::{
    prepare, with_publication_locks_mode_and_guard_targets, PublicationTreeLockMode, PublishMode,
    PublishRequest,
};
use crate::infrastructure::platform::filesystem::metadata_is_link_or_reparse_point;
use std::{
    collections::{BTreeMap, BTreeSet},
    ffi::OsString,
    fs,
    io::ErrorKind,
    path::{Path, PathBuf},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum DirectoryMembershipSelector {
    XmlFiles,
    CfFilesAsciiCaseInsensitive,
    AllDirectEntries,
}

#[derive(Debug, Default)]
pub(crate) struct CompileTransaction {
    replacement: Option<PlannedReplacement>,
    exact_guards: BTreeMap<PathBuf, Vec<u8>>,
    absence_guards: BTreeSet<PathBuf>,
    membership_guards: BTreeMap<(PathBuf, DirectoryMembershipSelector), Vec<OsString>>,
}

#[derive(Debug)]
struct PlannedReplacement {
    path: PathBuf,
    before: Vec<u8>,
    after: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CommitReport {
    pub(crate) updated_paths: Vec<PathBuf>,
}

impl CompileTransaction {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) fn protects_path(&self, path: &Path) -> Result<bool, String> {
        Ok(self
            .replacement
            .as_ref()
            .is_some_and(|replacement| replacement.path == path))
    }

    pub(crate) fn replace_bytes(
        &mut self,
        path: impl Into<PathBuf>,
        expected_preimage: impl AsRef<[u8]>,
        replacement: impl Into<Vec<u8>>,
    ) -> Result<(), String> {
        if self.replacement.is_some() {
            return Err("BSL publication transaction accepts exactly one replacement".to_string());
        }
        let path = path.into();
        let metadata = fs::symlink_metadata(&path)
            .map_err(|error| format!("failed to inspect {}: {error}", path.display()))?;
        if metadata_is_link_or_reparse_point(&metadata) || !metadata.is_file() {
            return Err(format!(
                "BSL publication target must be a regular non-link file: {}",
                path.display()
            ));
        }
        let before = fs::read(&path)
            .map_err(|error| format!("failed to read {}: {error}", path.display()))?;
        if before != expected_preimage.as_ref() {
            return Err(format!(
                "BSL publication target changed while planning: {}",
                path.display()
            ));
        }
        self.replacement = Some(PlannedReplacement {
            path,
            before,
            after: replacement.into(),
        });
        Ok(())
    }

    pub(crate) fn guard_or_verify_exact_preimage(
        &mut self,
        path: impl Into<PathBuf>,
        expected_preimage: impl AsRef<[u8]>,
    ) -> Result<(), String> {
        let path = path.into();
        let expected = expected_preimage.as_ref().to_vec();
        let actual = fs::read(&path)
            .map_err(|error| format!("failed to read guard {}: {error}", path.display()))?;
        if actual != expected {
            return Err(format!("guard changed while planning: {}", path.display()));
        }
        match self.exact_guards.get(&path) {
            Some(existing) if existing != &expected => {
                Err(format!("conflicting guard preimages: {}", path.display()))
            }
            _ => {
                self.exact_guards.insert(path, expected);
                Ok(())
            }
        }
    }

    pub(crate) fn guard_path_absent(&mut self, path: impl Into<PathBuf>) -> Result<(), String> {
        let path = path.into();
        match fs::symlink_metadata(&path) {
            Err(error) if error.kind() == ErrorKind::NotFound => {
                self.absence_guards.insert(path);
                Ok(())
            }
            Ok(_) => Err(format!(
                "compile transaction absence guard was violated while planning: {}",
                path.display()
            )),
            Err(error) => Err(format!("failed to inspect {}: {error}", path.display())),
        }
    }

    pub(crate) fn guard_or_verify_directory_membership(
        &mut self,
        directory: impl Into<PathBuf>,
        selector: DirectoryMembershipSelector,
        expected_names: Vec<OsString>,
    ) -> Result<(), String> {
        let directory = directory.into();
        let actual = selected_names(&directory, selector)?;
        let expected = normalized_names(expected_names);
        if actual != expected {
            let unexpected = actual
                .iter()
                .filter(|name| !expected.contains(name))
                .map(|name| name.to_string_lossy())
                .collect::<Vec<_>>();
            let missing = expected
                .iter()
                .filter(|name| !actual.contains(name))
                .map(|name| name.to_string_lossy())
                .collect::<Vec<_>>();
            return Err(format!(
                "directory membership changed while planning: {}; unexpected: [{}]; missing: [{}]",
                directory.display(),
                unexpected.join(", "),
                missing.join(", ")
            ));
        }
        self.membership_guards
            .insert((directory, selector), expected);
        Ok(())
    }

    pub(crate) fn commit(self) -> Result<CommitReport, String> {
        let targets = self
            .replacement
            .iter()
            .map(|replacement| replacement.path.clone())
            .collect::<Vec<_>>();
        let mut guard_targets = self.exact_guards.keys().cloned().collect::<Vec<_>>();
        guard_targets.extend(self.absence_guards.iter().cloned());
        guard_targets.extend(
            self.membership_guards
                .keys()
                .map(|(directory, _)| directory.clone()),
        );
        let result = with_publication_locks_mode_and_guard_targets(
            &targets,
            &guard_targets,
            PublicationTreeLockMode::Shared,
            |lock| -> Result<CommitReport, String> {
                self.verify_guards()?;
                let mut updated_paths = Vec::new();
                if let Some(replacement) = &self.replacement {
                    prepare(
                        lock,
                        PublishRequest {
                            target: &replacement.path,
                            replacement: &replacement.after,
                            mode: PublishMode::ReplaceExisting {
                                expected_preimage: &replacement.before,
                            },
                        },
                    )
                    .map_err(|error| error.to_string())?
                    .commit_with_guard(|| self.verify_guards())
                    .map_err(|error| error.to_string())?;
                    if replacement.before != replacement.after {
                        updated_paths.push(replacement.path.clone());
                    }
                }
                Ok(CommitReport { updated_paths })
            },
        )
        .map_err(|error| error.to_string())?;
        result
    }

    fn verify_guards(&self) -> Result<(), String> {
        for (path, expected) in &self.exact_guards {
            let actual = fs::read(path)
                .map_err(|error| format!("failed to recheck {}: {error}", path.display()))?;
            if &actual != expected {
                return Err(format!("guard changed before commit: {}", path.display()));
            }
        }
        for path in &self.absence_guards {
            match fs::symlink_metadata(path) {
                Err(error) if error.kind() == ErrorKind::NotFound => {}
                Ok(_) => return Err(format!("guarded path appeared: {}", path.display())),
                Err(error) => return Err(format!("failed to recheck {}: {error}", path.display())),
            }
        }
        for ((directory, selector), expected) in &self.membership_guards {
            if &selected_names(directory, *selector)? != expected {
                return Err(format!(
                    "directory membership changed before commit: {}",
                    directory.display()
                ));
            }
        }
        Ok(())
    }
}

fn normalized_names(mut names: Vec<OsString>) -> Vec<OsString> {
    names.sort();
    names.dedup();
    names
}

fn selected_names(
    directory: &Path,
    selector: DirectoryMembershipSelector,
) -> Result<Vec<OsString>, String> {
    let mut names = Vec::new();
    for entry in fs::read_dir(directory)
        .map_err(|error| format!("failed to read {}: {error}", directory.display()))?
    {
        let entry = entry.map_err(|error| format!("failed to read directory entry: {error}"))?;
        let metadata = fs::symlink_metadata(entry.path())
            .map_err(|error| format!("failed to inspect {}: {error}", entry.path().display()))?;
        if metadata_is_link_or_reparse_point(&metadata) {
            return Err(format!(
                "directory membership contains a link or reparse point: {}",
                entry.path().display()
            ));
        }
        let selected = match selector {
            DirectoryMembershipSelector::XmlFiles => {
                metadata.is_file()
                    && entry.path().extension().and_then(|value| value.to_str()) == Some("xml")
            }
            DirectoryMembershipSelector::CfFilesAsciiCaseInsensitive => {
                metadata.is_file()
                    && entry
                        .path()
                        .extension()
                        .and_then(|value| value.to_str())
                        .is_some_and(|value| value.eq_ignore_ascii_case("cf"))
            }
            DirectoryMembershipSelector::AllDirectEntries => {
                metadata.is_file() || metadata.is_dir()
            }
        };
        if selected {
            names.push(entry.file_name());
        }
    }
    Ok(normalized_names(names))
}
