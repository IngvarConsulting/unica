use std::path::Path;

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
