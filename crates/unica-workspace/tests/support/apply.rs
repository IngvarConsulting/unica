use super::*;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
pub(super) fn staged(root: &Path) -> ApplyStagedState {
    staged_with_authority(root, crate::actor::apply_writer_authority_for_test())
}

pub(super) fn staged_with_authority(
    root: &Path,
    authority: crate::actor::ApplyWriterAuthority,
) -> ApplyStagedState {
    staged_with_authority_and_deadline(
        root,
        authority,
        ProviderDeadline::from_budget(Duration::from_secs(5)),
    )
}

pub(super) fn staged_with_authority_and_deadline(
    root: &Path,
    authority: crate::actor::ApplyWriterAuthority,
    deadline: ProviderDeadline,
) -> ApplyStagedState {
    let canonical = std::fs::canonicalize(root).unwrap();
    ApplyStagedState::from_retained_root(
        Arc::new(RetainedDirectoryCapability::open(&canonical).unwrap()),
        deadline,
        CancellationToken::new(),
        authority,
    )
}

pub(super) fn temp_root(name: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!(
        "unica-apply-stage-{name}-{}-{nonce}",
        std::process::id()
    ))
}
