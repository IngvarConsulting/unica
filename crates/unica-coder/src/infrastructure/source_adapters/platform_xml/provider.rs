use std::{
    collections::BTreeMap,
    fs,
    path::{Component, Path},
    sync::Arc,
};

use sha2::{Digest, Sha256};

use crate::domain::source_adapters::{
    SourceAdapterError, SourceAdapterErrorKind, SourceRevision,
};

#[derive(Debug)]
pub(crate) struct PlatformXmlProvider {
    files: BTreeMap<String, Arc<[u8]>>,
    revision: SourceRevision,
}

impl PlatformXmlProvider {
    pub(crate) fn open(root: impl AsRef<Path>) -> Result<Self, SourceAdapterError> {
        Self::open_with_capture_hook(root, || {})
    }

    #[cfg(test)]
    fn open_with_test_hook(
        root: impl AsRef<Path>,
        after_first_capture: impl FnOnce(),
    ) -> Result<Self, SourceAdapterError> {
        Self::open_with_capture_hook(root, after_first_capture)
    }

    fn open_with_capture_hook(
        root: impl AsRef<Path>,
        after_first_capture: impl FnOnce(),
    ) -> Result<Self, SourceAdapterError> {
        let root = root.as_ref();
        ensure_directory(root)?;
        let first = capture(root)?;
        after_first_capture();
        let second = capture(root)?;
        if first != second {
            return Err(SourceAdapterError::new(
                SourceAdapterErrorKind::SnapshotStale,
                "Platform XML aggregate changed while the snapshot was captured",
            ));
        }
        let revision = revision_for(&first)?;
        Ok(Self {
            files: first,
            revision,
        })
    }

    pub(crate) fn read_relative(
        &self,
        raw: impl AsRef<Path>,
    ) -> Result<Arc<[u8]>, SourceAdapterError> {
        let key = normalized_relative_key(raw.as_ref())?;
        self.files.get(&key).cloned().ok_or_else(|| {
            SourceAdapterError::new(
                SourceAdapterErrorKind::SourceUnavailable,
                "Platform XML snapshot does not contain the requested relative file",
            )
        })
    }

    pub(crate) fn revision(&self) -> Result<SourceRevision, SourceAdapterError> {
        Ok(self.revision.clone())
    }

    pub(crate) fn digest_relative(&self, raw: impl AsRef<Path>) -> Result<String, SourceAdapterError> {
        let bytes = self.read_relative(raw)?;
        Ok(format!("sha256:{:x}", Sha256::digest(bytes)))
    }

    pub(super) fn snapshot_files(&self) -> impl Iterator<Item = (&str, Arc<[u8]>)> + '_ {
        self.files
            .iter()
            .map(|(relative, bytes)| (relative.as_str(), Arc::clone(bytes)))
    }
}

fn ensure_directory(path: &Path) -> Result<(), SourceAdapterError> {
    let metadata = fs::symlink_metadata(path).map_err(|_| unavailable("aggregate root"))?;
    if metadata.file_type().is_symlink() {
        return Err(unavailable("aggregate root must not be a symlink"));
    }
    if !metadata.is_dir() {
        return Err(unavailable("aggregate root is not a directory"));
    }
    Ok(())
}

fn capture(root: &Path) -> Result<BTreeMap<String, Arc<[u8]>>, SourceAdapterError> {
    let mut files = BTreeMap::new();
    capture_directory(root, "", &mut files)?;
    Ok(files)
}

fn capture_directory(
    directory: &Path,
    prefix: &str,
    files: &mut BTreeMap<String, Arc<[u8]>>,
) -> Result<(), SourceAdapterError> {
    ensure_directory(directory)?;
    let entries = fs::read_dir(directory).map_err(|_| unavailable("aggregate directory"))?;
    for entry in entries {
        let entry = entry.map_err(|_| unavailable("aggregate directory entry"))?;
        let name = entry
            .file_name()
            .into_string()
            .map_err(|_| unavailable("aggregate entry has a non-UTF-8 name"))?;
        if name.is_empty() || name.contains('\\') || name.contains('/') {
            return Err(unavailable("aggregate entry has an invalid name"));
        }
        let key = if prefix.is_empty() {
            name
        } else {
            format!("{prefix}/{name}")
        };
        let path = entry.path();
        let metadata = fs::symlink_metadata(&path).map_err(|_| unavailable("aggregate entry"))?;
        if metadata.file_type().is_symlink() {
            return Err(unavailable("aggregate must not contain symlinks"));
        }
        if metadata.is_dir() {
            capture_directory(&path, &key, files)?;
        } else if metadata.is_file() {
            let bytes: Arc<[u8]> = Arc::from(fs::read(&path).map_err(|_| unavailable("aggregate file"))?);
            let after_read = fs::symlink_metadata(&path).map_err(|_| unavailable("aggregate file"))?;
            if after_read.file_type().is_symlink() || !after_read.is_file() {
                return Err(unavailable("aggregate file changed while being read"));
            }
            if files.insert(key, bytes).is_some() {
                return Err(unavailable("aggregate contains duplicate relative keys"));
            }
        } else {
            return Err(unavailable("aggregate entry is not a regular file or directory"));
        }
    }
    Ok(())
}

fn normalized_relative_key(raw: &Path) -> Result<String, SourceAdapterError> {
    if raw.as_os_str().is_empty() || raw.is_absolute() {
        return Err(unavailable("requested path must be a non-empty relative path"));
    }
    let mut parts = Vec::new();
    for component in raw.components() {
        let Component::Normal(part) = component else {
            return Err(unavailable("requested path must not contain traversal components"));
        };
        let part = part
            .to_str()
            .filter(|part| !part.is_empty() && !part.contains('\\') && !part.contains('/'))
            .ok_or_else(|| unavailable("requested path is not a normalized UTF-8 key"))?;
        parts.push(part);
    }
    Ok(parts.join("/"))
}

fn revision_for(files: &BTreeMap<String, Arc<[u8]>>) -> Result<SourceRevision, SourceAdapterError> {
    let mut digest = Sha256::new();
    digest.update(b"unica:platform-xml:aggregate-snapshot:v1\0");
    for (key, bytes) in files {
        let file_digest = Sha256::digest(bytes);
        digest.update((key.len() as u64).to_be_bytes());
        digest.update(key.as_bytes());
        digest.update((bytes.len() as u64).to_be_bytes());
        digest.update(file_digest);
    }
    SourceRevision::new(format!("sha256:{:x}", digest.finalize()))
}

fn unavailable(message: &str) -> SourceAdapterError {
    SourceAdapterError::new(SourceAdapterErrorKind::SourceUnavailable, message)
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        path::PathBuf,
        sync::atomic::{AtomicU64, Ordering},
    };

    use super::PlatformXmlProvider;
    use crate::{
        domain::source_adapters::SourceAdapterErrorKind,
        infrastructure::platform::filesystem::{
            create_dir_symlink_for_test, create_file_symlink_for_test,
        },
    };

    static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(0);

    #[test]
    fn provider_rejects_parent_traversal_before_io() {
        let fixture = fixture(&[("Object.xml", b"<MetaDataObject/>" as &[u8])]);
        let error = fixture.provider.read_relative("../outside.xml").unwrap_err();

        assert_eq!(error.kind, SourceAdapterErrorKind::SourceUnavailable);
    }

    #[test]
    fn snapshot_revision_is_independent_of_root_and_read_order() {
        let first = fixture(&[("Object.xml", b"one"), ("Nested/Other.xml", b"two")]);
        let second = fixture(&[("Object.xml", b"one"), ("Nested/Other.xml", b"two")]);

        first.provider.read_relative("Nested/Other.xml").unwrap();
        first.provider.read_relative("Object.xml").unwrap();
        second.provider.read_relative("Object.xml").unwrap();
        second.provider.read_relative("Nested/Other.xml").unwrap();

        assert_eq!(first.provider.revision().unwrap(), second.provider.revision().unwrap());
    }

    #[test]
    fn unread_files_affect_the_aggregate_revision() {
        let without_extra = fixture(&[("Object.xml", b"one")]);
        let with_extra = fixture(&[("Object.xml", b"one"), ("Unread.xml", b"two")]);

        without_extra.provider.read_relative("Object.xml").unwrap();
        with_extra.provider.read_relative("Object.xml").unwrap();

        assert_ne!(without_extra.provider.revision().unwrap(), with_extra.provider.revision().unwrap());
    }

    #[test]
    fn snapshot_is_immutable_after_successful_capture() {
        let fixture = fixture(&[("Object.xml", b"before")]);
        fs::write(fixture.root.join("Object.xml"), b"after").unwrap();
        fs::remove_file(fixture.root.join("Object.xml")).unwrap();

        assert_eq!(fixture.provider.read_relative("Object.xml").unwrap().as_ref(), b"before");
    }

    #[test]
    fn capture_change_is_rejected_as_snapshot_stale() {
        let root = fixture_root(&[("Object.xml", b"before")]);
        let root_for_hook = root.clone();

        let error = PlatformXmlProvider::open_with_test_hook(&root, move || {
            fs::write(root_for_hook.join("Object.xml"), b"after").unwrap();
        })
        .unwrap_err();

        assert_eq!(error.kind, SourceAdapterErrorKind::SnapshotStale);
    }

    #[test]
    fn provider_rejects_file_and_directory_symlinks() {
        let outside = fixture_root(&[("outside.xml", b"outside")]);
        let file_root = fixture_root(&[]);
        if let Some(result) = create_file_symlink_for_test(outside.join("outside.xml"), file_root.join("linked.xml")) {
            result.unwrap();
            assert_eq!(PlatformXmlProvider::open(&file_root).unwrap_err().kind, SourceAdapterErrorKind::SourceUnavailable);
        }

        let directory_root = fixture_root(&[]);
        if let Some(result) = create_dir_symlink_for_test(&outside, directory_root.join("linked")) {
            result.unwrap();
            assert_eq!(PlatformXmlProvider::open(&directory_root).unwrap_err().kind, SourceAdapterErrorKind::SourceUnavailable);
        }

        let root_link = std::env::temp_dir().join(format!(
            "unica-platform-xml-provider-root-link-{}-{}",
            std::process::id(),
            NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed),
        ));
        if let Some(result) = create_dir_symlink_for_test(&outside, &root_link) {
            result.unwrap();
            assert_eq!(PlatformXmlProvider::open(&root_link).unwrap_err().kind, SourceAdapterErrorKind::SourceUnavailable);
        }
    }

    struct Fixture {
        root: PathBuf,
        provider: PlatformXmlProvider,
    }

    fn fixture(entries: &[(&str, &[u8])]) -> Fixture {
        let root = fixture_root(entries);
        let provider = PlatformXmlProvider::open(&root).unwrap();
        Fixture { root, provider }
    }

    fn fixture_root(entries: &[(&str, &[u8])]) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "unica-platform-xml-provider-{}-{}",
            std::process::id(),
            NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed),
        ));
        fs::create_dir_all(&root).unwrap();
        for (relative, bytes) in entries {
            let path = root.join(relative);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, bytes).unwrap();
        }
        root
    }
}
