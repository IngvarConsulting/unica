use std::{
    collections::BTreeMap,
    fs,
    path::{Component, Path, PathBuf},
    sync::{Arc, Mutex},
};

use sha2::{Digest, Sha256};

use crate::domain::source_adapters::{
    SourceAdapterError, SourceAdapterErrorKind, SourceRevision,
};

pub(crate) struct PlatformXmlProvider {
    root: PathBuf,
    reads: Mutex<BTreeMap<PathBuf, Arc<[u8]>>>,
}

impl PlatformXmlProvider {
    pub(crate) fn new(root: impl AsRef<Path>) -> Result<Self, SourceAdapterError> {
        let root = fs::canonicalize(root.as_ref()).map_err(|error| unavailable("aggregate root", error))?;
        if !fs::metadata(&root)
            .map_err(|error| unavailable("aggregate root", error))?
            .is_dir()
        {
            return Err(SourceAdapterError::new(
                SourceAdapterErrorKind::SourceUnavailable,
                "Platform XML aggregate root is not a directory",
            ));
        }
        Ok(Self {
            root,
            reads: Mutex::new(BTreeMap::new()),
        })
    }

    pub(crate) fn read_relative(
        &self,
        raw: impl AsRef<Path>,
    ) -> Result<Arc<[u8]>, SourceAdapterError> {
        let relative = validate_relative(raw.as_ref())?;
        let mut reads = self.reads.lock().map_err(|_| {
            SourceAdapterError::new(
                SourceAdapterErrorKind::SourceUnavailable,
                "Platform XML read snapshot is unavailable",
            )
        })?;
        if let Some(bytes) = reads.get(&relative) {
            return Ok(Arc::clone(bytes));
        }

        let candidate = self.root.join(&relative);
        let metadata = fs::symlink_metadata(&candidate)
            .map_err(|error| unavailable("Platform XML source file", error))?;
        if metadata.file_type().is_symlink() {
            return Err(SourceAdapterError::new(
                SourceAdapterErrorKind::SourceUnavailable,
                "Platform XML source file must not be a symlink",
            ));
        }
        if !metadata.is_file() {
            return Err(SourceAdapterError::new(
                SourceAdapterErrorKind::SourceUnavailable,
                "Platform XML source path must be a regular file",
            ));
        }

        let resolved = fs::canonicalize(&candidate)
            .map_err(|error| unavailable("Platform XML source file", error))?;
        if !resolved.starts_with(&self.root) {
            return Err(SourceAdapterError::new(
                SourceAdapterErrorKind::SourceUnavailable,
                "Platform XML source path escapes the aggregate root",
            ));
        }

        let bytes: Arc<[u8]> = Arc::from(
            fs::read(&resolved).map_err(|error| unavailable("Platform XML source file", error))?,
        );
        reads.insert(relative, Arc::clone(&bytes));
        Ok(bytes)
    }

    pub(crate) fn revision(&self) -> Result<SourceRevision, SourceAdapterError> {
        let reads = self.reads.lock().map_err(|_| {
            SourceAdapterError::new(
                SourceAdapterErrorKind::SourceUnavailable,
                "Platform XML read snapshot is unavailable",
            )
        })?;
        let mut digest = Sha256::new();
        digest.update(b"unica:platform-xml:read-set:v1\0");
        for (relative, bytes) in reads.iter() {
            let relative = relative.to_str().ok_or_else(|| {
                SourceAdapterError::new(
                    SourceAdapterErrorKind::SourceUnavailable,
                    "Platform XML source path is not valid UTF-8",
                )
            })?;
            let file_digest = Sha256::digest(bytes);
            digest.update((relative.len() as u64).to_be_bytes());
            digest.update(relative.as_bytes());
            digest.update((bytes.len() as u64).to_be_bytes());
            digest.update(file_digest);
        }
        SourceRevision::new(format!("sha256:{:x}", digest.finalize()))
    }
}

fn validate_relative(raw: &Path) -> Result<PathBuf, SourceAdapterError> {
    if raw.as_os_str().is_empty()
        || raw.is_absolute()
        || raw.components().any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(SourceAdapterError::new(
            SourceAdapterErrorKind::SourceUnavailable,
            "Platform XML source path must be a non-empty relative file path",
        ));
    }
    Ok(raw.to_path_buf())
}

fn unavailable(subject: &str, error: std::io::Error) -> SourceAdapterError {
    SourceAdapterError::new(
        SourceAdapterErrorKind::SourceUnavailable,
        format!("{subject} is unavailable: {error}"),
    )
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        path::PathBuf,
        sync::atomic::{AtomicU64, Ordering},
    };

    use super::PlatformXmlProvider;
    use crate::domain::source_adapters::SourceAdapterErrorKind;

    static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(0);

    #[test]
    fn provider_rejects_parent_traversal_before_io() {
        let fixture = fixture_provider();
        let error = fixture.provider.read_relative("../outside.xml").unwrap_err();

        assert_eq!(error.kind, SourceAdapterErrorKind::SourceUnavailable);
    }

    #[test]
    fn provider_returns_the_same_immutable_bytes_for_repeated_reads() {
        let fixture = fixture_provider_with("Object.xml", b"<MetaDataObject/>");

        let first = fixture.provider.read_relative("Object.xml").unwrap();
        fs::write(fixture.root.join("Object.xml"), b"changed").unwrap();
        let second = fixture.provider.read_relative("Object.xml").unwrap();

        assert_eq!(first.as_ref(), second.as_ref());
    }

    #[test]
    fn revision_is_content_derived_without_a_physical_path() {
        let fixture = fixture_provider_with("Object.xml", b"<MetaDataObject/>");
        fixture.provider.read_relative("Object.xml").unwrap();

        let revision = fixture.provider.revision().unwrap();
        let serialized = serde_json::to_string(&revision).unwrap();

        assert!(serialized.starts_with("\"sha256:"));
        assert!(!serialized.contains(&fixture.root.display().to_string()));
    }

    struct Fixture {
        root: PathBuf,
        provider: PlatformXmlProvider,
    }

    fn fixture_provider() -> Fixture {
        fixture_provider_with("Object.xml", b"<MetaDataObject/>")
    }

    fn fixture_provider_with(relative: &str, contents: &[u8]) -> Fixture {
        let root = std::env::temp_dir().join(format!(
            "unica-platform-xml-provider-{}-{}",
            std::process::id(),
            NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed),
        ));
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join(relative), contents).unwrap();
        let provider = PlatformXmlProvider::new(&root).unwrap();
        Fixture { root, provider }
    }
}
