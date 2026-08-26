use crate::domain::cancellation::CancellationToken;
use crate::domain::code_intelligence::ProviderDeadline;
use crate::infrastructure::platform::filesystem::{
    RetainedChildCapability, RetainedDirectoryCapability, RetainedRegularFileCapability,
};
use crate::infrastructure::source_roots::normalize_path_identity;
use serde_json::Value;
use std::collections::HashSet;
use std::ffi::OsStr;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

const POLICY_NAME: &str = ".v8-project.json";
const MAX_SUPPORT_POLICY_BYTES: usize = 32 * 1024 * 1024;

#[cfg(test)]
thread_local! {
    static SUPPORT_POLICY_CAPTURE_HOOK: std::cell::RefCell<Option<Box<dyn FnOnce()>>> =
        std::cell::RefCell::new(None);
    static SUPPORT_POLICY_VALIDATION_HOOK: std::cell::RefCell<Option<Box<dyn FnOnce()>>> =
        std::cell::RefCell::new(None);
    static SUPPORT_POLICY_AFTER_PRE_READ_IDENTITY_HOOK: std::cell::RefCell<Option<Box<dyn FnOnce()>>> =
        std::cell::RefCell::new(None);
    static SUPPORT_POLICY_AFTER_RETAINED_READ_BEFORE_ACCEPTANCE_HOOK: std::cell::RefCell<Option<Box<dyn FnOnce()>>> =
        std::cell::RefCell::new(None);
}

#[cfg(test)]
pub(crate) fn set_support_policy_capture_hook(hook: impl FnOnce() + 'static) {
    SUPPORT_POLICY_CAPTURE_HOOK.with(|slot| *slot.borrow_mut() = Some(Box::new(hook)));
}

#[cfg(test)]
pub(crate) fn set_support_policy_validation_hook(hook: impl FnOnce() + 'static) {
    SUPPORT_POLICY_VALIDATION_HOOK.with(|slot| *slot.borrow_mut() = Some(Box::new(hook)));
}

#[cfg(test)]
fn set_support_policy_after_pre_read_identity_hook(hook: impl FnOnce() + 'static) {
    SUPPORT_POLICY_AFTER_PRE_READ_IDENTITY_HOOK
        .with(|slot| *slot.borrow_mut() = Some(Box::new(hook)));
}

#[cfg(test)]
pub(crate) fn set_support_policy_after_retained_read_before_acceptance_hook(
    hook: impl FnOnce() + 'static,
) {
    SUPPORT_POLICY_AFTER_RETAINED_READ_BEFORE_ACCEPTANCE_HOOK
        .with(|slot| *slot.borrow_mut() = Some(Box::new(hook)));
}

#[cfg(test)]
fn run_support_policy_capture_hook() {
    SUPPORT_POLICY_CAPTURE_HOOK.with(|slot| {
        if let Some(hook) = slot.borrow_mut().take() {
            hook();
        }
    });
}

#[cfg(test)]
fn run_support_policy_validation_hook() {
    SUPPORT_POLICY_VALIDATION_HOOK.with(|slot| {
        if let Some(hook) = slot.borrow_mut().take() {
            hook();
        }
    });
}

#[cfg(test)]
fn run_support_policy_after_pre_read_identity_hook() {
    SUPPORT_POLICY_AFTER_PRE_READ_IDENTITY_HOOK.with(|slot| {
        if let Some(hook) = slot.borrow_mut().take() {
            hook();
        }
    });
}

#[cfg(test)]
fn run_support_policy_after_retained_read_before_acceptance_hook() {
    SUPPORT_POLICY_AFTER_RETAINED_READ_BEFORE_ACCEPTANCE_HOOK.with(|slot| {
        if let Some(hook) = slot.borrow_mut().take() {
            hook();
        }
    });
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SupportPolicyMode {
    Deny,
    Warn,
    Off,
}

pub(crate) fn v8_project_candidates_for_directory(start: &Path) -> Vec<PathBuf> {
    let mut current = start.to_path_buf();
    let mut candidates = Vec::new();
    for _ in 0..20 {
        candidates.push(current.join(POLICY_NAME));
        let Some(parent) = current.parent() else {
            break;
        };
        if parent == current {
            break;
        }
        current = parent.to_path_buf();
    }
    candidates
}

pub(crate) fn support_policy_mode_from_bytes(
    bytes: &[u8],
    project_dir: &Path,
    config_dir: &Path,
) -> SupportPolicyMode {
    let Ok(text) = std::str::from_utf8(bytes) else {
        return SupportPolicyMode::Deny;
    };
    let Ok(project) = serde_json::from_str::<Value>(text.trim_start_matches('\u{feff}')) else {
        return SupportPolicyMode::Deny;
    };
    support_policy_mode_from_value(&project, project_dir, config_dir)
}

fn support_policy_mode_from_value(
    project: &Value,
    project_dir: &Path,
    config_dir: &Path,
) -> SupportPolicyMode {
    let config_dir = normalize_guard_path(config_dir);

    if let Some(databases) = project.get("databases").and_then(Value::as_array) {
        for database in databases {
            let Some(config_src) = database.get("configSrc").and_then(Value::as_str) else {
                continue;
            };
            let config_src = PathBuf::from(config_src);
            let config_src = if config_src.is_absolute() {
                config_src
            } else {
                project_dir.join(config_src)
            };
            let config_src = normalize_guard_path(&config_src);
            if (config_dir == config_src || config_dir.starts_with(&config_src))
                && database
                    .get("editingAllowedCheck")
                    .and_then(Value::as_str)
                    .is_some()
            {
                return support_policy_mode_value(
                    database
                        .get("editingAllowedCheck")
                        .and_then(Value::as_str)
                        .expect("checked above"),
                );
            }
        }
    }

    project
        .get("editingAllowedCheck")
        .and_then(Value::as_str)
        .map(support_policy_mode_value)
        .unwrap_or(SupportPolicyMode::Deny)
}

pub(crate) fn support_policy_mode_value(value: &str) -> SupportPolicyMode {
    match value {
        "warn" => SupportPolicyMode::Warn,
        "off" => SupportPolicyMode::Off,
        _ => SupportPolicyMode::Deny,
    }
}

fn normalize_guard_path(path: &Path) -> PathBuf {
    normalize_path_identity(path).unwrap_or_else(|_| path.to_path_buf())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SupportPolicyEvidenceErrorKind {
    Cancelled,
    Deadline,
    ContainmentIdentity,
    Provider,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SupportPolicyEvidenceError {
    kind: SupportPolicyEvidenceErrorKind,
    message: String,
}

impl SupportPolicyEvidenceError {
    pub(crate) const fn kind(&self) -> SupportPolicyEvidenceErrorKind {
        self.kind
    }
}

impl std::fmt::Display for SupportPolicyEvidenceError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.message.fmt(formatter)
    }
}

impl std::error::Error for SupportPolicyEvidenceError {}

pub(crate) struct RetainedSupportPolicyEvidence {
    mode: SupportPolicyMode,
    candidates: Vec<RetainedPolicyCandidate>,
}

enum RetainedPolicyCandidate {
    Absent(RetainedDirectoryCapability),
    Exact {
        file: RetainedRegularFileCapability,
        bytes: Vec<u8>,
    },
    Oversized(RetainedRegularFileCapability),
    WrongKind {
        parent: RetainedDirectoryCapability,
        kind: RetainedWrongKind,
    },
    Unreadable(RetainedDirectoryCapability),
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum RetainedWrongKind {
    Directory,
    ReparsePoint,
    Unsupported,
}

impl RetainedSupportPolicyEvidence {
    pub(crate) fn capture(
        workspace_root: &Path,
        source_root: &Path,
        deadline: ProviderDeadline,
        cancellation: &CancellationToken,
    ) -> Result<Self, SupportPolicyEvidenceError> {
        checkpoint(deadline, cancellation, "support-policy capture")?;
        let mut ordered = Vec::new();
        let mut seen = HashSet::new();
        for start in [workspace_root, source_root, workspace_root] {
            for candidate in v8_project_candidates_for_directory(start) {
                if seen.insert(candidate.clone()) {
                    ordered.push(candidate);
                }
            }
        }
        let mut candidates = Vec::new();
        for candidate in ordered {
            checkpoint(deadline, cancellation, "support-policy capture")?;
            let parent_path = candidate
                .parent()
                .ok_or_else(|| SupportPolicyEvidenceError {
                    kind: SupportPolicyEvidenceErrorKind::ContainmentIdentity,
                    message: "support-policy candidate has no retained parent".to_string(),
                })?;
            let parent = RetainedDirectoryCapability::open(parent_path).map_err(|_| {
                SupportPolicyEvidenceError {
                    kind: SupportPolicyEvidenceErrorKind::ContainmentIdentity,
                    message: "support-policy candidate parent cannot be retained".to_string(),
                }
            })?;
            match parent.retain_immediate_child_nofollow(OsStr::new(POLICY_NAME)) {
                Err(error) if error.kind() == ErrorKind::NotFound => {
                    candidates.push(RetainedPolicyCandidate::Absent(parent));
                }
                Err(_) => {
                    candidates.push(RetainedPolicyCandidate::Unreadable(parent));
                    #[cfg(test)]
                    run_support_policy_capture_hook();
                    checkpoint(deadline, cancellation, "support-policy capture")?;
                    return Ok(Self {
                        mode: SupportPolicyMode::Deny,
                        candidates,
                    });
                }
                Ok(RetainedChildCapability::RegularFile(file)) => {
                    return match file.read_bounded(MAX_SUPPORT_POLICY_BYTES) {
                        Ok(bytes) => {
                            let mode =
                                support_policy_mode_from_bytes(&bytes, parent_path, source_root);
                            candidates.push(RetainedPolicyCandidate::Exact { file, bytes });
                            #[cfg(test)]
                            run_support_policy_capture_hook();
                            checkpoint(deadline, cancellation, "support-policy capture")?;
                            Ok(Self { mode, candidates })
                        }
                        Err(error) if error.kind() == ErrorKind::InvalidData => {
                            candidates.push(RetainedPolicyCandidate::Oversized(file));
                            #[cfg(test)]
                            run_support_policy_capture_hook();
                            checkpoint(deadline, cancellation, "support-policy capture")?;
                            Ok(Self {
                                mode: SupportPolicyMode::Deny,
                                candidates,
                            })
                        }
                        Err(_) => {
                            candidates.push(RetainedPolicyCandidate::Unreadable(parent));
                            #[cfg(test)]
                            run_support_policy_capture_hook();
                            checkpoint(deadline, cancellation, "support-policy capture")?;
                            Ok(Self {
                                mode: SupportPolicyMode::Deny,
                                candidates,
                            })
                        }
                    };
                }
                Ok(RetainedChildCapability::Directory(_)) => {
                    candidates.push(RetainedPolicyCandidate::WrongKind {
                        parent,
                        kind: RetainedWrongKind::Directory,
                    });
                    #[cfg(test)]
                    run_support_policy_capture_hook();
                    checkpoint(deadline, cancellation, "support-policy capture")?;
                    return Ok(Self {
                        mode: SupportPolicyMode::Deny,
                        candidates,
                    });
                }
                Ok(RetainedChildCapability::ReparsePoint) => {
                    candidates.push(RetainedPolicyCandidate::WrongKind {
                        parent,
                        kind: RetainedWrongKind::ReparsePoint,
                    });
                    #[cfg(test)]
                    run_support_policy_capture_hook();
                    checkpoint(deadline, cancellation, "support-policy capture")?;
                    return Ok(Self {
                        mode: SupportPolicyMode::Deny,
                        candidates,
                    });
                }
                Ok(RetainedChildCapability::Unsupported) => {
                    candidates.push(RetainedPolicyCandidate::WrongKind {
                        parent,
                        kind: RetainedWrongKind::Unsupported,
                    });
                    #[cfg(test)]
                    run_support_policy_capture_hook();
                    checkpoint(deadline, cancellation, "support-policy capture")?;
                    return Ok(Self {
                        mode: SupportPolicyMode::Deny,
                        candidates,
                    });
                }
            }
        }
        #[cfg(test)]
        run_support_policy_capture_hook();
        checkpoint(deadline, cancellation, "support-policy capture")?;
        Ok(Self {
            mode: SupportPolicyMode::Deny,
            candidates,
        })
    }

    pub(crate) const fn mode(&self) -> SupportPolicyMode {
        self.mode
    }

    pub(crate) fn validate(
        &self,
        deadline: ProviderDeadline,
        cancellation: &CancellationToken,
    ) -> Result<(), SupportPolicyEvidenceError> {
        self.validate_complete_pass(deadline, cancellation)?;
        self.validate_complete_pass(deadline, cancellation)
    }

    fn validate_complete_pass(
        &self,
        deadline: ProviderDeadline,
        cancellation: &CancellationToken,
    ) -> Result<(), SupportPolicyEvidenceError> {
        checkpoint(deadline, cancellation, "support-policy validation")?;
        for candidate in &self.candidates {
            checkpoint(deadline, cancellation, "support-policy validation")?;
            validate_candidate(candidate)?;
            #[cfg(test)]
            run_support_policy_validation_hook();
            checkpoint(deadline, cancellation, "support-policy validation")?;
        }
        checkpoint(deadline, cancellation, "support-policy validation")
    }
}

fn validate_candidate(
    candidate: &RetainedPolicyCandidate,
) -> Result<(), SupportPolicyEvidenceError> {
    match candidate {
        RetainedPolicyCandidate::Absent(parent) => {
            validate_parent(parent)?;
            match parent.retain_immediate_child_nofollow(OsStr::new(POLICY_NAME)) {
                Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
                _ => Err(changed("an absent support-policy candidate changed")),
            }
        }
        RetainedPolicyCandidate::Exact { file, bytes } => {
            file.validate_named_identity()
                .map_err(|_| changed("support-policy file identity changed"))?;
            #[cfg(test)]
            run_support_policy_after_pre_read_identity_hook();
            let current = file
                .read_bounded(MAX_SUPPORT_POLICY_BYTES)
                .map_err(|_| changed("support-policy file bytes became unreadable"))?;
            if &current != bytes {
                return Err(changed("support-policy file bytes changed"));
            }
            #[cfg(test)]
            run_support_policy_after_retained_read_before_acceptance_hook();
            file.validate_named_identity()
                .map_err(|_| changed("support-policy file identity changed during validation"))?;
            Ok(())
        }
        RetainedPolicyCandidate::Oversized(file) => {
            file.validate_named_identity()
                .map_err(|_| changed("oversized support-policy identity changed"))?;
            #[cfg(test)]
            run_support_policy_after_pre_read_identity_hook();
            match file.read_bounded(MAX_SUPPORT_POLICY_BYTES) {
                Err(error) if error.kind() == ErrorKind::InvalidData => {
                    file.validate_named_identity().map_err(|_| {
                        changed("oversized support-policy identity changed during validation")
                    })
                }
                _ => Err(changed("oversized support-policy evidence changed")),
            }
        }
        RetainedPolicyCandidate::WrongKind { parent, kind } => {
            validate_parent(parent)?;
            let observed = parent
                .retain_immediate_child_nofollow(OsStr::new(POLICY_NAME))
                .map_err(|_| changed("wrong-kind support-policy evidence changed"))?;
            let matches = matches!(
                (kind, observed),
                (
                    RetainedWrongKind::Directory,
                    RetainedChildCapability::Directory(_)
                ) | (
                    RetainedWrongKind::ReparsePoint,
                    RetainedChildCapability::ReparsePoint
                ) | (
                    RetainedWrongKind::Unsupported,
                    RetainedChildCapability::Unsupported
                )
            );
            if matches {
                Ok(())
            } else {
                Err(changed("wrong-kind support-policy evidence changed"))
            }
        }
        RetainedPolicyCandidate::Unreadable(parent) => {
            validate_parent(parent)?;
            match parent.retain_immediate_child_nofollow(OsStr::new(POLICY_NAME)) {
                Err(error) if error.kind() != ErrorKind::NotFound => Ok(()),
                _ => Err(changed("unreadable support-policy evidence changed")),
            }
        }
    }
}

fn validate_parent(parent: &RetainedDirectoryCapability) -> Result<(), SupportPolicyEvidenceError> {
    parent
        .validate_named_identity()
        .map_err(|_| changed("support-policy candidate parent physical identity changed"))
}

fn changed(message: &str) -> SupportPolicyEvidenceError {
    SupportPolicyEvidenceError {
        kind: SupportPolicyEvidenceErrorKind::ContainmentIdentity,
        message: message.to_string(),
    }
}

fn checkpoint(
    deadline: ProviderDeadline,
    cancellation: &CancellationToken,
    phase: &str,
) -> Result<(), SupportPolicyEvidenceError> {
    if cancellation.is_cancelled() {
        Err(SupportPolicyEvidenceError {
            kind: SupportPolicyEvidenceErrorKind::Cancelled,
            message: format!("{phase} cancelled"),
        })
    } else if deadline.remaining().is_zero() {
        Err(SupportPolicyEvidenceError {
            kind: SupportPolicyEvidenceErrorKind::Deadline,
            message: format!("{phase} deadline exceeded"),
        })
    } else {
        Ok(())
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    pub(crate) fn support_policy_database_paths_distinguish_nested_sources_from_prefix_siblings() {
        let temporary = tempfile::tempdir().unwrap();
        let project_dir = temporary.path().join("project");
        let source = project_dir.join("src");
        let nested = source.join("nested");
        let sibling = project_dir.join("src-copy");
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::create_dir_all(&sibling).unwrap();

        let relative = br#"{
            "editingAllowedCheck":"off",
            "databases":[{"configSrc":"src","editingAllowedCheck":"warn"}]
        }"#;
        assert_eq!(
            support_policy_mode_from_bytes(relative, &project_dir, &nested),
            SupportPolicyMode::Warn
        );
        assert_eq!(
            support_policy_mode_from_bytes(relative, &project_dir, &sibling),
            SupportPolicyMode::Off
        );

        let absolute = serde_json::to_vec(&serde_json::json!({
            "editingAllowedCheck": "deny",
            "databases": [{
                "configSrc": source.to_string_lossy(),
                "editingAllowedCheck": "off"
            }]
        }))
        .unwrap();
        assert_eq!(
            support_policy_mode_from_bytes(&absolute, &project_dir, &nested),
            SupportPolicyMode::Off
        );
        assert_eq!(
            support_policy_mode_from_bytes(&absolute, &project_dir, &sibling),
            SupportPolicyMode::Deny
        );
    }

    #[test]
    pub(crate) fn support_policy_candidate_search_stops_at_exact_twentieth_candidate() {
        let temporary = tempfile::tempdir().unwrap();
        let mut start = temporary.path().to_path_buf();
        for index in 0..24 {
            start.push(format!("level-{index}"));
        }
        std::fs::create_dir_all(&start).unwrap();

        let candidates = v8_project_candidates_for_directory(&start);

        assert_eq!(candidates.len(), 20);
        assert_eq!(candidates[0], start.join(POLICY_NAME));
        assert_eq!(
            candidates[19],
            start.ancestors().nth(19).unwrap().join(POLICY_NAME)
        );
        assert!(!candidates.contains(&start.ancestors().nth(20).unwrap().join(POLICY_NAME)));
    }

    #[test]
    pub(crate) fn support_policy_overlapping_chains_keep_first_occurrence_order_without_duplicates()
    {
        let temporary = tempfile::tempdir().unwrap();
        let mut workspace = temporary.path().to_path_buf();
        for index in 0..24 {
            workspace.push(format!("level-{index}"));
        }
        let source = workspace.join("src");
        std::fs::create_dir_all(&source).unwrap();
        let workspace = std::fs::canonicalize(workspace).unwrap();
        let source = std::fs::canonicalize(source).unwrap();

        let evidence = RetainedSupportPolicyEvidence::capture(
            &workspace,
            &source,
            ProviderDeadline::from_budget(Duration::from_secs(5)),
            &CancellationToken::new(),
        )
        .unwrap();
        let actual = evidence
            .candidates
            .iter()
            .map(|candidate| match candidate {
                RetainedPolicyCandidate::Absent(parent) => parent.path().join(POLICY_NAME),
                _ => panic!("fixture intentionally contains no policy candidates"),
            })
            .collect::<Vec<_>>();
        let mut expected = workspace
            .ancestors()
            .take(20)
            .map(|parent| parent.join(POLICY_NAME))
            .collect::<Vec<_>>();
        expected.push(source.join(POLICY_NAME));

        assert_eq!(actual, expected);
    }

    #[test]
    pub(crate) fn retained_support_policy_candidate_parent_replacement_is_rejected() {
        if !crate::infrastructure::platform::testing::can_swap_named_child_behind_retained_handle_for_test()
        {
            eprintln!(
                "[SKIPPED FIXTURE] retained support-policy parent replacement is unsupported while a directory handle is open"
            );
            return;
        }
        let temporary = tempfile::tempdir().unwrap();
        let workspace = temporary.path().join("workspace");
        let source = workspace.join("src");
        std::fs::create_dir_all(&source).unwrap();
        let workspace = std::fs::canonicalize(workspace).unwrap();
        let source = std::fs::canonicalize(source).unwrap();
        let evidence = RetainedSupportPolicyEvidence::capture(
            &workspace,
            &source,
            ProviderDeadline::from_budget(Duration::from_secs(5)),
            &CancellationToken::new(),
        )
        .unwrap();
        let displaced = temporary.path().join("workspace-displaced");
        std::fs::rename(&workspace, &displaced).unwrap();
        std::fs::create_dir_all(&workspace).unwrap();

        let error = evidence
            .validate(
                ProviderDeadline::from_budget(Duration::from_secs(5)),
                &CancellationToken::new(),
            )
            .unwrap_err();

        assert_eq!(
            error.kind(),
            SupportPolicyEvidenceErrorKind::ContainmentIdentity
        );
    }

    #[test]
    pub(crate) fn retained_support_policy_exact_and_oversized_reject_name_replacement_after_pre_read_identity(
    ) {
        if !crate::infrastructure::platform::testing::can_swap_named_child_behind_retained_handle_for_test()
        {
            eprintln!(
                "[SKIPPED FIXTURE] support-policy mid-validation name replacement is unsupported while a retained file handle is open"
            );
            return;
        }

        let mut rejected = Vec::new();
        for category in ["exact", "oversized"] {
            let temporary = tempfile::tempdir().unwrap();
            let workspace = temporary.path().join("workspace");
            let source = workspace.join("src");
            std::fs::create_dir_all(&source).unwrap();
            let policy = workspace.join(POLICY_NAME);
            match category {
                "exact" => std::fs::write(&policy, br#"{"editingAllowedCheck":"off"}"#).unwrap(),
                "oversized" => {
                    std::fs::write(&policy, vec![b' '; MAX_SUPPORT_POLICY_BYTES + 1]).unwrap()
                }
                _ => unreachable!(),
            }
            let workspace = std::fs::canonicalize(workspace).unwrap();
            let source = std::fs::canonicalize(source).unwrap();
            let evidence = RetainedSupportPolicyEvidence::capture(
                &workspace,
                &source,
                ProviderDeadline::from_budget(Duration::from_secs(10)),
                &CancellationToken::new(),
            )
            .unwrap();
            let displaced = temporary.path().join(format!("{category}-displaced.json"));
            let hook_policy = policy.clone();
            set_support_policy_after_pre_read_identity_hook(move || {
                std::fs::rename(&hook_policy, &displaced).unwrap();
                std::fs::write(&hook_policy, br#"{"editingAllowedCheck":"warn"}"#).unwrap();
            });

            rejected.push(
                evidence
                    .validate(
                        ProviderDeadline::from_budget(Duration::from_secs(10)),
                        &CancellationToken::new(),
                    )
                    .is_err(),
            );
        }
        assert_eq!(rejected, [true, true]);
    }

    #[test]
    pub(crate) fn retained_support_policy_exact_rejects_name_replacement_after_retained_read_before_acceptance(
    ) {
        if !crate::infrastructure::platform::testing::can_swap_named_child_behind_retained_handle_for_test()
        {
            eprintln!(
                "[SKIPPED FIXTURE] support-policy post-read name replacement is unsupported while a retained file handle is open"
            );
            return;
        }

        let temporary = tempfile::tempdir().unwrap();
        let workspace = temporary.path().join("workspace");
        let source = workspace.join("src");
        std::fs::create_dir_all(&source).unwrap();
        let policy = workspace.join(POLICY_NAME);
        std::fs::write(&policy, br#"{"editingAllowedCheck":"off"}"#).unwrap();
        let workspace = std::fs::canonicalize(workspace).unwrap();
        let source = std::fs::canonicalize(source).unwrap();
        let evidence = RetainedSupportPolicyEvidence::capture(
            &workspace,
            &source,
            ProviderDeadline::from_budget(Duration::from_secs(10)),
            &CancellationToken::new(),
        )
        .unwrap();
        let displaced = temporary.path().join("exact-displaced.json");
        let hook_policy = policy.clone();
        set_support_policy_after_retained_read_before_acceptance_hook(move || {
            std::fs::rename(&hook_policy, &displaced).unwrap();
            std::fs::write(&hook_policy, br#"{"editingAllowedCheck":"warn"}"#).unwrap();
        });

        let rejected = evidence
            .validate(
                ProviderDeadline::from_budget(Duration::from_secs(10)),
                &CancellationToken::new(),
            )
            .is_err();

        assert!(
            rejected,
            "exact evidence accepted a persistent fixed-name replacement after its retained read"
        );
    }

    #[test]
    pub(crate) fn retained_support_policy_exact_rejects_same_inode_change_between_stability_passes()
    {
        let temporary = tempfile::tempdir().unwrap();
        let workspace = temporary.path().join("workspace");
        let source = workspace.join("src");
        std::fs::create_dir_all(&source).unwrap();
        let policy = workspace.join(POLICY_NAME);
        let admitted = br#"{"editingAllowedCheck":"off"}"#;
        let changed = br#"{"editingAllowedCheck":"bad"}"#;
        assert_eq!(admitted.len(), changed.len());
        std::fs::write(&policy, admitted).unwrap();
        let workspace = std::fs::canonicalize(workspace).unwrap();
        let source = std::fs::canonicalize(source).unwrap();
        let evidence = RetainedSupportPolicyEvidence::capture(
            &workspace,
            &source,
            ProviderDeadline::from_budget(Duration::from_secs(10)),
            &CancellationToken::new(),
        )
        .unwrap();
        let admitted_identity = crate::infrastructure::platform::filesystem::file_identity(
            &std::fs::File::open(&policy).unwrap(),
        )
        .unwrap();
        let hook_policy = policy.clone();
        set_support_policy_after_retained_read_before_acceptance_hook(move || {
            std::fs::write(&hook_policy, changed).unwrap();
        });

        let error = evidence
            .validate(
                ProviderDeadline::from_budget(Duration::from_secs(10)),
                &CancellationToken::new(),
            )
            .unwrap_err();
        let current_identity = crate::infrastructure::platform::filesystem::file_identity(
            &std::fs::File::open(&policy).unwrap(),
        )
        .unwrap();

        assert_eq!(current_identity, admitted_identity);
        assert_eq!(
            error.kind(),
            SupportPolicyEvidenceErrorKind::ContainmentIdentity
        );
    }
}
