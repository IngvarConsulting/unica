use crate::domain::cancellation::CancellationToken;
use crate::domain::code_intelligence::ProviderDeadline;
use crate::infrastructure::native_operations::compile_transaction::{
    CompileTransaction, RetainedApplyChangeBinding,
};
use crate::infrastructure::platform::filesystem::{
    FileIdentity, RetainedChildCapability, RetainedDirectoryCapability,
};
use std::ffi::OsString;
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

#[derive(Debug)]
struct StagedEntry {
    relative_path: PathBuf,
    parent: RetainedDirectoryCapability,
    name: OsString,
    target_identity: StagedTargetIdentity,
    original: StagedFileState,
    current: StagedFileState,
    original_file:
        Option<crate::infrastructure::platform::filesystem::RetainedRegularFileCapability>,
}

#[derive(Debug)]
enum StagedTargetIdentity {
    Existing(FileIdentity),
    Absent {
        parent: FileIdentity,
        name: OsString,
    },
}

#[derive(Debug)]
pub(crate) struct ApplyStagedState {
    root: Arc<RetainedDirectoryCapability>,
    entries: Vec<StagedEntry>,
    deadline: ProviderDeadline,
    cancellation: CancellationToken,
    writer_authority: crate::infrastructure::workspace_actor::ApplyWriterAuthority,
    #[cfg(test)]
    absent_name_identity_for_test: Option<fn(&std::ffi::OsStr, &std::ffi::OsStr) -> bool>,
}

impl ApplyStagedState {
    pub(in crate::infrastructure) fn from_retained_root(
        root: Arc<RetainedDirectoryCapability>,
        deadline: ProviderDeadline,
        cancellation: CancellationToken,
        writer_authority: crate::infrastructure::workspace_actor::ApplyWriterAuthority,
    ) -> Self {
        Self {
            root,
            entries: Vec::new(),
            deadline,
            cancellation,
            writer_authority,
            #[cfg(test)]
            absent_name_identity_for_test: None,
        }
    }

    pub(crate) fn read(&mut self, relative: &Path) -> Result<Option<Vec<u8>>, String> {
        let relative = strict_relative(relative)?;
        let index = self.ensure_loaded(&relative)?;
        Ok(self.entries[index].current.as_option())
    }

    pub(crate) fn create(
        &mut self,
        relative: impl AsRef<Path>,
        bytes: Vec<u8>,
    ) -> Result<(), String> {
        let relative = strict_relative(relative.as_ref())?;
        let index = self.ensure_loaded(&relative)?;
        let entry = &mut self.entries[index];
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
        let index = self.ensure_loaded(&relative)?;
        let entry = &mut self.entries[index];
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
        let index = self.ensure_loaded(&relative)?;
        let entry = &mut self.entries[index];
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
        let mut changes = self
            .entries
            .iter()
            .filter(|entry| entry.original != entry.current)
            .map(|entry| StagedApplyChange {
                relative_path: entry.relative_path.clone(),
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
            .collect::<Vec<_>>();
        changes.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));
        changes
    }

    pub(crate) fn finalize(self) -> Result<CompileTransaction, String> {
        self.checkpoint("apply finalization")?;
        let mut transaction = CompileTransaction::new();
        transaction.bind_retained_apply_authority(&self.writer_authority)?;
        let mut entries = self.entries;
        entries.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));
        for entry in entries {
            if entry.original == entry.current {
                continue;
            }
            transaction.bind_retained_apply_change(
                RetainedApplyChangeBinding {
                    root: Arc::clone(&self.root),
                    relative_path: entry.relative_path,
                    parent: entry.parent,
                    name: entry.name,
                    original: entry.original.as_option(),
                    current: entry.current.as_option(),
                    original_file: entry.original_file,
                },
                &self.writer_authority,
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

    pub(in crate::infrastructure) fn has_writer_authority(
        &self,
        authority: &crate::infrastructure::workspace_actor::ApplyWriterAuthority,
    ) -> bool {
        &self.writer_authority == authority
    }

    #[cfg(test)]
    fn set_absent_name_identity_for_test(
        &mut self,
        comparator: fn(&std::ffi::OsStr, &std::ffi::OsStr) -> bool,
    ) {
        self.absent_name_identity_for_test = Some(comparator);
    }

    fn ensure_loaded(&mut self, relative: &Path) -> Result<usize, String> {
        self.checkpoint("apply staged read")?;
        if let Some(index) = self
            .entries
            .iter()
            .position(|entry| entry.relative_path == relative)
        {
            return Ok(index);
        }
        let (parent, name) = self
            .root
            .retain_relative_parent_nofollow(relative)
            .map_err(|error| {
                format!("staged target parent rejected link/reparse traversal: {error}")
            })?;
        let retained_child = parent.retain_immediate_child_nofollow(&name);
        if let Ok(RetainedChildCapability::RegularFile(file)) = retained_child.as_ref() {
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
        }
        let target_identity = match retained_child.as_ref() {
            Ok(RetainedChildCapability::RegularFile(file)) => {
                Some(StagedTargetIdentity::Existing(file.identity()))
            }
            Err(error) if error.kind() == ErrorKind::NotFound => {
                Some(StagedTargetIdentity::Absent {
                    parent: parent.identity(),
                    name: name.clone(),
                })
            }
            _ => None,
        };
        if let Some(target_identity) = target_identity.as_ref() {
            for (index, entry) in self.entries.iter().enumerate() {
                if self.same_target(&entry.target_identity, target_identity, &parent)? {
                    return Ok(index);
                }
            }
        }
        let (original, original_file) = match retained_child {
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
                let bytes = file
                    .read_bounded(MAX_APPLY_FILE_BYTES)
                    .map_err(|error| format!("staged target fixed read bound failed: {error}"))?;
                (StagedFileState::Bytes(bytes), Some(file))
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
            Err(error) if error.kind() == ErrorKind::NotFound => (StagedFileState::Absent, None),
            Err(error) => return Err(format!("staged target inspection failed: {error}")),
        };
        self.entries.push(StagedEntry {
            relative_path: relative.to_path_buf(),
            parent,
            name,
            target_identity: target_identity.expect("regular or absent target has an identity"),
            current: original.clone(),
            original,
            original_file,
        });
        Ok(self.entries.len() - 1)
    }

    fn same_target(
        &self,
        left: &StagedTargetIdentity,
        right: &StagedTargetIdentity,
        right_parent: &RetainedDirectoryCapability,
    ) -> Result<bool, String> {
        match (left, right) {
            (StagedTargetIdentity::Existing(left), StagedTargetIdentity::Existing(right)) => {
                Ok(left == right)
            }
            (
                StagedTargetIdentity::Absent {
                    parent: left_parent,
                    name: left_name,
                },
                StagedTargetIdentity::Absent {
                    parent: right_parent_identity,
                    name: right_name,
                },
            ) if left_parent == right_parent_identity => {
                #[cfg(test)]
                if let Some(comparator) = self.absent_name_identity_for_test {
                    return Ok(comparator(left_name, right_name));
                }
                right_parent
                    .child_names_equivalent(left_name, right_name)
                    .map_err(|error| {
                        format!("staged child-name identity cannot be proven: {error}")
                    })
            }
            _ => Ok(false),
        }
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
    use super::{ApplyStagedState, StagedChangeKind, StagedFileState, MAX_APPLY_FILE_BYTES};
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
            crate::infrastructure::workspace_actor::apply_writer_authority_for_test(),
        )
    }

    #[test]
    fn apply_staged_state_composes_ordered_same_file_postimages() {
        let root = temp_root("composition");
        std::fs::create_dir_all(root.join("Ext")).unwrap();
        std::fs::write(root.join("Ext/existing.txt"), b"original").unwrap();
        let mut state = staged(&root);

        assert_eq!(state.read(Path::new("Ext/new.txt")).unwrap(), None);
        state.create("Ext/new.txt", b"created".to_vec()).unwrap();
        assert_eq!(
            state.read(Path::new("Ext/new.txt")).unwrap(),
            Some(b"created".to_vec())
        );
        state
            .replace("Ext/new.txt", b"created", b"created-then-replaced".to_vec())
            .unwrap();
        assert_eq!(
            state.read(Path::new("Ext/new.txt")).unwrap(),
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
            state.read(Path::new("Ext/existing.txt")).unwrap(),
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
            .read(Path::new("../escape.txt"))
            .unwrap_err()
            .contains("normal"));

        let link_outcome =
            create_directory_link_fixture_for_test(root.join("Ext/real"), root.join("Ext/link"))
                .unwrap();
        if link_outcome == FileLinkFixtureOutcome::Created {
            assert!(state
                .read(Path::new("Ext/link/a.txt"))
                .unwrap_err()
                .contains("link"));
            std::fs::create_dir_all(root.join("Race")).unwrap();
            std::fs::write(root.join("Race/Module.bsl"), b"original race bytes").unwrap();
            let external = temp_root("nested-link-external");
            std::fs::create_dir_all(&external).unwrap();
            std::fs::write(external.join("Module.bsl"), b"external decoy").unwrap();
            let mut raced = staged(&root);
            assert_eq!(
                raced.read(Path::new("Race/Module.bsl")).unwrap(),
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
            .read(Path::new("Ext/real/a.txt"))
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

    #[test]
    fn retained_apply_plan_is_not_empty_and_generic_commit_refuses_actor_bypass() {
        let root = temp_root("actor-bypass");
        std::fs::create_dir_all(root.join("Ext")).unwrap();
        std::fs::write(root.join("Ext/Module.bsl"), b"original").unwrap();
        let mut state = staged(&root);
        state
            .replace("Ext/Module.bsl", b"original", b"bypass".to_vec())
            .unwrap();
        let transaction = state.finalize().unwrap();

        assert!(
            !transaction.is_empty(),
            "retained apply entries were ignored"
        );
        let error = transaction.commit().unwrap_err();
        assert!(error.contains("actor"), "{error}");
        assert_eq!(
            std::fs::read(root.join("Ext/Module.bsl")).unwrap(),
            b"original"
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn retained_create_stage_source_swap_leaves_no_destination_mutation_and_loses_neither_file() {
        if !crate::infrastructure::platform::testing::can_swap_named_child_behind_retained_handle_for_test() {
            return;
        }
        let root = temp_root("stage-source-swap");
        let parent = root.join("Ext");
        std::fs::create_dir_all(&parent).unwrap();
        let target = parent.join("new.txt");
        let owned_stage = parent.join("owned-stage.txt");
        let mut state = staged(&root);
        state
            .create("Ext/new.txt", b"apply-stage".to_vec())
            .unwrap();
        let authority = state.writer_authority.clone();
        let transaction = state.finalize().unwrap();
        let hook_parent = parent.clone();
        let hook_owned = owned_stage.clone();
        crate::infrastructure::platform::filesystem::set_before_identity_bound_no_replace_rename_hook(
            move || {
                let stage = apply_artifact(&hook_parent);
                std::fs::rename(&stage, &hook_owned).unwrap();
                std::fs::write(&stage, b"concurrent-stage").unwrap();
            },
        );

        let error = transaction
            .commit_retained_apply_with(authority, || Ok(()), || Ok(()))
            .unwrap_err();

        assert!(
            error.contains("identity") || error.contains("source"),
            "{error}"
        );
        assert!(
            !target.exists(),
            "reported failure left a destination mutation"
        );
        assert_eq!(std::fs::read(&owned_stage).unwrap(), b"apply-stage");
        assert!(apply_artifact_contents(&parent)
            .iter()
            .any(|bytes| bytes == b"concurrent-stage"));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn retained_rollback_restore_recovery_swap_never_reports_a_different_inode_restored() {
        if !crate::infrastructure::platform::testing::can_swap_named_child_behind_retained_handle_for_test() {
            return;
        }
        let root = temp_root("restore-recovery-swap");
        let parent = root.join("Ext");
        std::fs::create_dir_all(&parent).unwrap();
        let target = parent.join("Module.bsl");
        let owned_recovery = parent.join("owned-recovery.bsl");
        std::fs::write(&target, b"original").unwrap();
        let mut state = staged(&root);
        state
            .replace("Ext/Module.bsl", b"original", b"apply-bytes".to_vec())
            .unwrap();
        let authority = state.writer_authority.clone();
        let transaction = state.finalize().unwrap();
        let hook_parent = parent.clone();
        let hook_owned = owned_recovery.clone();
        crate::infrastructure::native_operations::compile_transaction::set_retained_apply_before_post_validation_hook(
            move || {
                crate::infrastructure::platform::filesystem::set_before_identity_bound_no_replace_rename_hook(
                    move || {
                        let recovery = apply_artifact(&hook_parent);
                        std::fs::rename(&recovery, &hook_owned).unwrap();
                        std::fs::write(&recovery, b"concurrent-recovery").unwrap();
                    },
                );
            },
        );

        let error = transaction
            .commit_retained_apply_with(
                authority,
                || Ok(()),
                || Err::<(), _>("post validation failure".to_string()),
            )
            .unwrap_err();

        assert!(error.contains("rollback"), "{error}");
        assert!(!target.exists(), "a different recovery inode was restored");
        assert_eq!(std::fs::read(&owned_recovery).unwrap(), b"original");
        assert!(apply_artifact_contents(&parent)
            .iter()
            .any(|bytes| bytes == b"concurrent-recovery"));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn retained_rollback_published_cleanup_never_unlinks_a_concurrent_child() {
        if !crate::infrastructure::platform::testing::can_swap_named_child_behind_retained_handle_for_test() {
            return;
        }
        let root = temp_root("published-cleanup-swap");
        let parent = root.join("Ext");
        std::fs::create_dir_all(&parent).unwrap();
        let target = parent.join("new.txt");
        let owned_published = parent.join("owned-published.txt");
        let mut state = staged(&root);
        state
            .create("Ext/new.txt", b"apply-bytes".to_vec())
            .unwrap();
        let authority = state.writer_authority.clone();
        let transaction = state.finalize().unwrap();
        let hook_target = target.clone();
        let hook_owned = owned_published.clone();
        crate::infrastructure::platform::filesystem::set_before_identity_bound_cleanup_mutation_hook(
            move || {
                std::fs::rename(&hook_target, &hook_owned).unwrap();
                std::fs::write(&hook_target, b"concurrent-target").unwrap();
            },
        );

        let error = transaction
            .commit_retained_apply_with(
                authority,
                || Ok(()),
                || Err::<(), _>("post validation failure".to_string()),
            )
            .unwrap_err();

        assert!(error.contains("rollback"), "{error}");
        assert_eq!(std::fs::read(&target).unwrap(), b"concurrent-target");
        assert_eq!(std::fs::read(&owned_published).unwrap(), b"apply-bytes");
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn retained_success_recovery_cleanup_never_unlinks_a_concurrent_child() {
        if !crate::infrastructure::platform::testing::can_swap_named_child_behind_retained_handle_for_test() {
            return;
        }
        let root = temp_root("recovery-cleanup-swap");
        let parent = root.join("Ext");
        std::fs::create_dir_all(&parent).unwrap();
        let target = parent.join("Module.bsl");
        let owned_recovery = parent.join("owned-recovery.bsl");
        std::fs::write(&target, b"original").unwrap();
        let mut state = staged(&root);
        state
            .replace("Ext/Module.bsl", b"original", b"apply-bytes".to_vec())
            .unwrap();
        let authority = state.writer_authority.clone();
        let transaction = state.finalize().unwrap();
        let hook_parent = parent.clone();
        let hook_owned = owned_recovery.clone();
        crate::infrastructure::platform::filesystem::set_before_identity_bound_cleanup_mutation_hook(
            move || {
                let recovery = apply_artifact(&hook_parent);
                std::fs::rename(&recovery, &hook_owned).unwrap();
                std::fs::write(&recovery, b"concurrent-recovery").unwrap();
            },
        );

        let (report, ()) = transaction
            .commit_retained_apply_with(authority, || Ok(()), || Ok(()))
            .unwrap();

        assert_eq!(std::fs::read(&target).unwrap(), b"apply-bytes");
        assert_eq!(std::fs::read(&owned_recovery).unwrap(), b"original");
        assert!(apply_artifact_contents(&parent)
            .iter()
            .any(|bytes| bytes == b"concurrent-recovery"));
        assert!(!report.cleanup_warnings.is_empty());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn retained_success_without_contention_leaves_no_apply_artifacts() {
        let root = temp_root("success-cleans-recovery");
        let parent = root.join("Ext");
        std::fs::create_dir_all(&parent).unwrap();
        let target = parent.join("Module.bsl");
        std::fs::write(&target, b"original").unwrap();
        let mut state = staged(&root);
        state
            .replace("Ext/Module.bsl", b"original", b"apply-bytes".to_vec())
            .unwrap();
        let authority = state.writer_authority.clone();
        let transaction = state.finalize().unwrap();

        let (report, ()) = transaction
            .commit_retained_apply_with(authority, || Ok(()), || Ok(()))
            .unwrap();

        assert_eq!(std::fs::read(&target).unwrap(), b"apply-bytes");
        assert!(report.cleanup_warnings.is_empty(), "{report:?}");
        assert!(apply_artifact_contents(&parent).is_empty());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn retained_absent_create_keeps_the_exact_staged_parent_authority() {
        let root = temp_root("absent-parent-authority");
        let nested = root.join("Nested");
        let retained_nested = root.join("Nested-retained");
        std::fs::create_dir_all(&nested).unwrap();
        let mut state = staged(&root);
        assert_eq!(state.read(Path::new("Nested/new.txt")).unwrap(), None);
        state
            .create("Nested/new.txt", b"must-not-redirect".to_vec())
            .unwrap();
        let authority = state.writer_authority.clone();
        let transaction = state.finalize().unwrap();
        std::fs::rename(&nested, &retained_nested).unwrap();
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::write(nested.join("marker.txt"), b"replacement-parent").unwrap();

        let error = transaction
            .commit_retained_apply_with(authority, || Ok(()), || Ok(()))
            .unwrap_err();

        assert!(
            error.contains("parent") || error.contains("identity"),
            "{error}"
        );
        assert!(!nested.join("new.txt").exists());
        assert!(!retained_nested.join("new.txt").exists());
        assert_eq!(
            std::fs::read(nested.join("marker.txt")).unwrap(),
            b"replacement-parent"
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn staged_aliases_share_one_physical_overlay_and_the_second_op_sees_the_first_postimage() {
        let root = temp_root("physical-overlay-alias");
        std::fs::create_dir_all(root.join("Ext")).unwrap();
        let mut state = staged(&root);
        state.set_absent_name_identity_for_test(|left, right| {
            left.to_string_lossy()
                .eq_ignore_ascii_case(&right.to_string_lossy())
        });

        state
            .create("Ext/Module.bsl", b"first-postimage".to_vec())
            .unwrap();
        assert_eq!(
            state.read(Path::new("Ext/module.bsl")).unwrap(),
            Some(b"first-postimage".to_vec())
        );
        state
            .replace(
                "Ext/module.bsl",
                b"first-postimage",
                b"second-postimage".to_vec(),
            )
            .unwrap();

        let changes = state.planned_changes();
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].relative_path, PathBuf::from("Ext/Module.bsl"));
        assert_eq!(
            changes[0].current,
            StagedFileState::Bytes(b"second-postimage".to_vec())
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn apply_staged_read_has_one_closed_bound_that_cannot_be_caller_bypassed() {
        let root = temp_root("closed-read-bound");
        std::fs::create_dir_all(root.join("Ext")).unwrap();
        std::fs::write(
            root.join("Ext/oversized.bin"),
            vec![0_u8; MAX_APPLY_FILE_BYTES + 1],
        )
        .unwrap();
        let mut state = staged(&root);

        let error = state.read(Path::new("Ext/oversized.bin")).unwrap_err();
        assert!(
            error.contains("too large") || error.contains("bound"),
            "{error}"
        );
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

    fn apply_artifact(parent: &Path) -> PathBuf {
        std::fs::read_dir(parent)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .find(|path| {
                path.file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.starts_with(".unica-apply-"))
            })
            .expect("retained apply artifact")
    }

    fn apply_artifact_contents(parent: &Path) -> Vec<Vec<u8>> {
        std::fs::read_dir(parent)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .filter(|path| {
                path.file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.starts_with(".unica-apply-"))
            })
            .map(|path| std::fs::read(path).unwrap())
            .collect()
    }
}
