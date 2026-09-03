use std::path::Path;

#[cfg(windows)]
use std::io;
#[cfg(windows)]
use std::thread;
#[cfg(windows)]
use std::time::{Duration, Instant};

#[cfg(windows)]
fn is_windows_sharing_violation(error: &io::Error) -> bool {
    const WINDOWS_SHARING_VIOLATION: i32 = 32;
    error.raw_os_error() == Some(WINDOWS_SHARING_VIOLATION)
}

#[cfg(windows)]
pub(super) fn remove_temp_tree(root: &Path) {
    const RETRY_BUDGET: Duration = Duration::from_secs(15);
    const RETRY_DELAY: Duration = Duration::from_millis(50);

    let deadline = Instant::now() + RETRY_BUDGET;
    loop {
        match std::fs::remove_dir_all(root) {
            Ok(()) => return,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return,
            Err(error) if is_windows_sharing_violation(&error) && Instant::now() < deadline => {
                thread::sleep(RETRY_DELAY);
            }
            Err(error) => panic!("cannot remove temporary tree {}: {error}", root.display()),
        }
    }
}

#[cfg(not(windows))]
pub(super) fn remove_temp_tree(root: &Path) {
    if let Err(error) = std::fs::remove_dir_all(root) {
        assert_eq!(
            error.kind(),
            std::io::ErrorKind::NotFound,
            "cannot remove temporary tree {}: {error}",
            root.display()
        );
    }
}

#[cfg(windows)]
#[test]
fn windows_corpus_cleanup_recognizes_the_observed_sharing_violation() {
    let error = io::Error::from_raw_os_error(32);
    assert!(is_windows_sharing_violation(&error));
}

#[cfg(unix)]
pub(super) fn require_single_link(path: &Path) -> Result<(), String> {
    use std::os::unix::fs::MetadataExt;

    let metadata = std::fs::metadata(path)
        .map_err(|error| format!("cannot inspect {}: {error}", path.display()))?;
    if metadata.nlink() != 1 {
        return Err(format!(
            "corpus payload hardlink alias is forbidden (link count {}): {}",
            metadata.nlink(),
            path.display()
        ));
    }
    Ok(())
}

#[cfg(not(unix))]
pub(super) fn require_single_link(_path: &Path) -> Result<(), String> {
    Ok(())
}

#[cfg(unix)]
pub(super) fn assert_independent_copy(source: &Path, copied: &Path) {
    use std::os::unix::fs::MetadataExt;

    let source_metadata = std::fs::metadata(source).unwrap();
    let copied_metadata = std::fs::metadata(copied).unwrap();
    assert_ne!(source_metadata.ino(), copied_metadata.ino());
    assert_eq!(copied_metadata.nlink(), 1);
}

#[cfg(not(unix))]
pub(super) fn assert_independent_copy(_source: &Path, _copied: &Path) {}
