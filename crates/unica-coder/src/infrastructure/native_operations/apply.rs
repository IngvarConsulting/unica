use crate::domain::cancellation::CancellationToken;
use crate::domain::code_intelligence::ProviderDeadline;
use crate::infrastructure::native_operations::compile_transaction::CompileTransaction;
use crate::infrastructure::platform::filesystem::{
    RetainedChildCapability, RetainedDirectoryCapability,
};
use std::collections::BTreeMap;
use std::io::ErrorKind;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

const MAX_APPLY_FILE_BYTES: usize = 32 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum StagedFileState {
    Bytes(Vec<u8>),
    Absent,
}

impl StagedFileState {
    fn as_option(&self) -> Option<Vec<u8>> {
        match self {
            Self::Bytes(bytes) => Some(bytes.clone()),
            Self::Absent => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StagedChangeKind {
    Create,
    Replace,
    Remove,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct StagedApplyChange {
    pub(crate) relative_path: PathBuf,
    pub(crate) kind: StagedChangeKind,
    pub(crate) original: StagedFileState,
    pub(crate) current: StagedFileState,
}

#[derive(Debug, Clone)]
struct StagedEntry {
    original: StagedFileState,
    current: StagedFileState,
}

#[derive(Debug)]
pub(crate) struct ApplyStagedState {
    root: Arc<RetainedDirectoryCapability>,
    entries: BTreeMap<PathBuf, StagedEntry>,
    deadline: ProviderDeadline,
    cancellation: CancellationToken,
}

impl ApplyStagedState {
    pub(in crate::infrastructure) fn from_retained_root(
        root: Arc<RetainedDirectoryCapability>,
        deadline: ProviderDeadline,
        cancellation: CancellationToken,
    ) -> Self {
        Self {
            root,
            entries: BTreeMap::new(),
            deadline,
            cancellation,
        }
    }

    pub(crate) fn read(
        &mut self,
        relative: &Path,
        max_bytes: usize,
    ) -> Result<Option<Vec<u8>>, String> {
        let relative = strict_relative(relative)?;
        self.ensure_loaded(&relative, max_bytes)?;
        Ok(self
            .entries
            .get(&relative)
            .expect("loaded staged entry must remain present")
            .current
            .as_option())
    }

    pub(crate) fn create(
        &mut self,
        relative: impl AsRef<Path>,
        bytes: Vec<u8>,
    ) -> Result<(), String> {
        let relative = strict_relative(relative.as_ref())?;
        self.ensure_loaded(&relative, MAX_APPLY_FILE_BYTES)?;
        let entry = self.entries.get_mut(&relative).expect("loaded entry");
        if entry.current != StagedFileState::Absent {
            return Err(format!(
                "staged create target already exists: {}",
                relative.display()
            ));
        }
        entry.current = StagedFileState::Bytes(bytes);
        Ok(())
    }

    pub(crate) fn replace(
        &mut self,
        relative: impl AsRef<Path>,
        expected_current: impl AsRef<[u8]>,
        bytes: Vec<u8>,
    ) -> Result<(), String> {
        let relative = strict_relative(relative.as_ref())?;
        self.ensure_loaded(&relative, MAX_APPLY_FILE_BYTES)?;
        let entry = self.entries.get_mut(&relative).expect("loaded entry");
        if entry.current != StagedFileState::Bytes(expected_current.as_ref().to_vec()) {
            return Err(format!(
                "staged replace preimage changed: {}",
                relative.display()
            ));
        }
        entry.current = StagedFileState::Bytes(bytes);
        Ok(())
    }

    pub(crate) fn remove(
        &mut self,
        relative: impl AsRef<Path>,
        expected_current: impl AsRef<[u8]>,
    ) -> Result<(), String> {
        let relative = strict_relative(relative.as_ref())?;
        self.ensure_loaded(&relative, MAX_APPLY_FILE_BYTES)?;
        let entry = self.entries.get_mut(&relative).expect("loaded entry");
        if entry.current != StagedFileState::Bytes(expected_current.as_ref().to_vec()) {
            return Err(format!(
                "staged remove preimage changed: {}",
                relative.display()
            ));
        }
        entry.current = StagedFileState::Absent;
        Ok(())
    }

    pub(crate) fn planned_changes(&self) -> Vec<StagedApplyChange> {
        self.entries
            .iter()
            .filter(|(_, entry)| entry.original != entry.current)
            .map(|(relative_path, entry)| StagedApplyChange {
                relative_path: relative_path.clone(),
                kind: match (&entry.original, &entry.current) {
                    (StagedFileState::Absent, StagedFileState::Bytes(_)) => {
                        StagedChangeKind::Create
                    }
                    (StagedFileState::Bytes(_), StagedFileState::Bytes(_)) => {
                        StagedChangeKind::Replace
                    }
                    (StagedFileState::Bytes(_), StagedFileState::Absent) => {
                        StagedChangeKind::Remove
                    }
                    (StagedFileState::Absent, StagedFileState::Absent) => unreachable!(),
                },
                original: entry.original.clone(),
                current: entry.current.clone(),
            })
            .collect()
    }

    pub(crate) fn finalize(self) -> Result<CompileTransaction, String> {
        self.checkpoint("apply finalization")?;
        let mut transaction = CompileTransaction::new();
        for (relative_path, entry) in self.entries {
            if entry.original == entry.current {
                continue;
            }
            transaction.bind_retained_apply_change(
                Arc::clone(&self.root),
                relative_path,
                entry.original.as_option(),
                entry.current.as_option(),
            )?;
        }
        transaction.validate_retained_for_apply()?;
        Ok(transaction)
    }

    pub(in crate::infrastructure) fn retained_root_identity(
        &self,
    ) -> crate::infrastructure::platform::filesystem::FileIdentity {
        self.root.identity()
    }

    fn ensure_loaded(&mut self, relative: &Path, max_bytes: usize) -> Result<(), String> {
        self.checkpoint("apply staged read")?;
        if self.entries.contains_key(relative) {
            return Ok(());
        }
        let (parent, name) = self
            .root
            .retain_relative_parent_nofollow(relative)
            .map_err(|error| {
                format!("staged target parent rejected link/reparse traversal: {error}")
            })?;
        let original = match parent.retain_immediate_child_nofollow(&name) {
            Ok(RetainedChildCapability::RegularFile(file)) => {
                if file
                    .hard_link_count()
                    .map_err(|error| format!("hard-link count failed: {error}"))?
                    != 1
                {
                    return Err(format!(
                        "staged target has a hard-link alias: {}",
                        relative.display()
                    ));
                }
                StagedFileState::Bytes(
                    file.read_bounded(max_bytes)
                        .map_err(|error| format!("staged target read failed: {error}"))?,
                )
            }
            Ok(RetainedChildCapability::ReparsePoint) => {
                return Err(format!(
                    "staged target is a link/reparse point: {}",
                    relative.display()
                ))
            }
            Ok(RetainedChildCapability::Directory(_) | RetainedChildCapability::Unsupported) => {
                return Err(format!(
                    "staged target is not a regular file: {}",
                    relative.display()
                ))
            }
            Err(error) if error.kind() == ErrorKind::NotFound => StagedFileState::Absent,
            Err(error) => return Err(format!("staged target inspection failed: {error}")),
        };
        self.entries.insert(
            relative.to_path_buf(),
            StagedEntry {
                current: original.clone(),
                original,
            },
        );
        Ok(())
    }

    fn checkpoint(&self, phase: &str) -> Result<(), String> {
        if self.cancellation.is_cancelled() {
            Err(format!("{phase} cancelled"))
        } else if self.deadline.remaining().is_zero() {
            Err(format!("{phase} deadline exceeded"))
        } else {
            Ok(())
        }
    }
}

fn strict_relative(relative: &Path) -> Result<PathBuf, String> {
    if relative.as_os_str().is_empty()
        || relative
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(format!(
            "staged target must contain only normal relative components: {}",
            relative.display()
        ));
    }
    Ok(relative.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::{ApplyStagedState, StagedChangeKind, StagedFileState};
    use crate::domain::cancellation::CancellationToken;
    use crate::domain::code_intelligence::ProviderDeadline;
    use crate::infrastructure::platform::filesystem::RetainedDirectoryCapability;
    use crate::infrastructure::platform::testing::{
        create_directory_link_fixture_for_test, FileLinkFixtureOutcome,
    };
    use std::path::{Path, PathBuf};
    use std::sync::Arc;
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    fn staged(root: &Path) -> ApplyStagedState {
        let canonical = std::fs::canonicalize(root).unwrap();
        ApplyStagedState::from_retained_root(
            Arc::new(RetainedDirectoryCapability::open(&canonical).unwrap()),
            ProviderDeadline::from_budget(Duration::from_secs(5)),
            CancellationToken::new(),
        )
    }

    #[test]
    fn apply_staged_state_composes_ordered_same_file_postimages() {
        let root = temp_root("composition");
        std::fs::create_dir_all(root.join("Ext")).unwrap();
        std::fs::write(root.join("Ext/existing.txt"), b"original").unwrap();
        let mut state = staged(&root);

        assert_eq!(state.read(Path::new("Ext/new.txt"), 128).unwrap(), None);
        state.create("Ext/new.txt", b"created".to_vec()).unwrap();
        assert_eq!(
            state.read(Path::new("Ext/new.txt"), 128).unwrap(),
            Some(b"created".to_vec())
        );
        state
            .replace("Ext/new.txt", b"created", b"created-then-replaced".to_vec())
            .unwrap();
        assert_eq!(
            state.read(Path::new("Ext/new.txt"), 128).unwrap(),
            Some(b"created-then-replaced".to_vec())
        );

        state
            .replace("Ext/existing.txt", b"original", b"replacement-1".to_vec())
            .unwrap();
        state
            .replace(
                "Ext/existing.txt",
                b"replacement-1",
                b"replacement-2".to_vec(),
            )
            .unwrap();
        assert_eq!(
            state.read(Path::new("Ext/existing.txt"), 128).unwrap(),
            Some(b"replacement-2".to_vec())
        );

        let changes = state.planned_changes();
        assert_eq!(changes.len(), 2);
        assert_eq!(changes[0].relative_path, PathBuf::from("Ext/existing.txt"));
        assert_eq!(changes[0].kind, StagedChangeKind::Replace);
        assert_eq!(
            changes[0].original,
            StagedFileState::Bytes(b"original".to_vec())
        );
        assert_eq!(
            changes[0].current,
            StagedFileState::Bytes(b"replacement-2".to_vec())
        );
        assert_eq!(changes[1].relative_path, PathBuf::from("Ext/new.txt"));
        assert_eq!(changes[1].kind, StagedChangeKind::Create);

        let transaction = state.finalize().unwrap();
        assert_eq!(transaction.retained_planned_change_count_for_test(), 2);
        assert_eq!(
            std::fs::read(root.join("Ext/existing.txt")).unwrap(),
            b"original"
        );
        assert!(!root.join("Ext/new.txt").exists());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn apply_staged_state_collapses_create_remove_and_replace_remove_to_final_state() {
        let root = temp_root("removals");
        std::fs::create_dir_all(root.join("Ext")).unwrap();
        std::fs::write(root.join("Ext/existing.txt"), b"original").unwrap();
        let mut state = staged(&root);

        state
            .create("Ext/transient.txt", b"temporary".to_vec())
            .unwrap();
        state.remove("Ext/transient.txt", b"temporary").unwrap();
        state
            .replace("Ext/existing.txt", b"original", b"replacement".to_vec())
            .unwrap();
        state.remove("Ext/existing.txt", b"replacement").unwrap();

        let changes = state.planned_changes();
        assert_eq!(
            changes.len(),
            1,
            "create->remove must collapse to no physical change"
        );
        assert_eq!(changes[0].kind, StagedChangeKind::Remove);
        assert_eq!(
            changes[0].original,
            StagedFileState::Bytes(b"original".to_vec())
        );
        assert_eq!(changes[0].current, StagedFileState::Absent);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn apply_staged_state_allows_remove_then_create_from_the_staged_postimage() {
        let root = temp_root("remove-create");
        std::fs::create_dir_all(root.join("Ext")).unwrap();
        std::fs::write(root.join("Ext/existing.txt"), b"original").unwrap();
        let mut state = staged(&root);

        state.remove("Ext/existing.txt", b"original").unwrap();
        state
            .create("Ext/existing.txt", b"recreated".to_vec())
            .unwrap();

        let changes = state.planned_changes();
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].kind, StagedChangeKind::Replace);
        assert_eq!(
            changes[0].original,
            StagedFileState::Bytes(b"original".to_vec())
        );
        assert_eq!(
            changes[0].current,
            StagedFileState::Bytes(b"recreated".to_vec())
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn apply_staged_state_rejects_duplicate_create_escape_links_and_hard_link_aliases() {
        let root = temp_root("invalid-targets");
        std::fs::create_dir_all(root.join("Ext/real")).unwrap();
        std::fs::write(root.join("Ext/real/a.txt"), b"same inode").unwrap();
        let mut state = staged(&root);
        state.create("Ext/new.txt", b"one".to_vec()).unwrap();
        assert!(state
            .create("Ext/new.txt", b"two".to_vec())
            .unwrap_err()
            .contains("create"));
        assert!(state
            .read(Path::new("../escape.txt"), 128)
            .unwrap_err()
            .contains("normal"));

        let link_outcome =
            create_directory_link_fixture_for_test(root.join("Ext/real"), root.join("Ext/link"))
                .unwrap();
        if link_outcome == FileLinkFixtureOutcome::Created {
            assert!(state
                .read(Path::new("Ext/link/a.txt"), 128)
                .unwrap_err()
                .contains("link"));
            std::fs::create_dir_all(root.join("Race")).unwrap();
            std::fs::write(root.join("Race/Module.bsl"), b"original race bytes").unwrap();
            let external = temp_root("nested-link-external");
            std::fs::create_dir_all(&external).unwrap();
            std::fs::write(external.join("Module.bsl"), b"external decoy").unwrap();
            let mut raced = staged(&root);
            assert_eq!(
                raced.read(Path::new("Race/Module.bsl"), 128).unwrap(),
                Some(b"original race bytes".to_vec())
            );
            let displaced = root.join("Race-displaced");
            std::fs::rename(root.join("Race"), &displaced).unwrap();
            assert_eq!(
                create_directory_link_fixture_for_test(&external, root.join("Race")).unwrap(),
                FileLinkFixtureOutcome::Created
            );
            raced
                .replace(
                    "Race/Module.bsl",
                    b"original race bytes",
                    b"must not redirect".to_vec(),
                )
                .unwrap();
            assert!(raced.finalize().unwrap_err().contains("link/reparse"));
            assert_eq!(
                std::fs::read(displaced.join("Module.bsl")).unwrap(),
                b"original race bytes"
            );
            assert_eq!(
                std::fs::read(external.join("Module.bsl")).unwrap(),
                b"external decoy"
            );
            std::fs::remove_dir_all(external).unwrap();
        }
        std::fs::hard_link(root.join("Ext/real/a.txt"), root.join("Ext/real/b.txt")).unwrap();
        assert!(state
            .read(Path::new("Ext/real/a.txt"), 128)
            .unwrap_err()
            .contains("hard-link"));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn retained_finalize_validation_failure_changes_nothing() {
        let root = temp_root("precommit-validation");
        std::fs::create_dir_all(root.join("Ext")).unwrap();
        let target = root.join("Ext/Form.xml");
        std::fs::write(&target, b"<Form/>").unwrap();
        let mut state = staged(&root);
        state
            .replace("Ext/Form.xml", b"<Form/>", b"<broken".to_vec())
            .unwrap();
        assert!(state.finalize().unwrap_err().contains("XML"));
        assert_eq!(std::fs::read(&target).unwrap(), b"<Form/>");
        std::fs::remove_dir_all(root).unwrap();
    }

    fn temp_root(name: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!(
            "unica-apply-stage-{name}-{}-{nonce}",
            std::process::id()
        ))
    }
}
