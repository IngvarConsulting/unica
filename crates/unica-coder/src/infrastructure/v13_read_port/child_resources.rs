use super::{RefusalDetail, SourceDirectory, ViewError};
use std::cell::RefCell;
use std::ffi::{OsStr, OsString};
use std::io;
use std::path::{Component, Path, PathBuf};
use unica_metadata_designer::child_resources::TypedChildResourceRead;
use unica_source_contract::discovery::{
    DiscoveryEntries, DiscoveryEntry, DiscoveryEntryReadPort, DiscoveryFileType,
};
use unica_source_contract::read::SourceEntry;

/// A child observer receives only reads through the authority's retained root.
/// Its display paths are never reopened through the ambient filesystem.
pub(super) struct RetainedChildResourceRead<'a> {
    root: &'a SourceDirectory,
    checkpoint: RefCell<&'a mut dyn FnMut() -> Result<(), ViewError>>,
    fatal: RefCell<Option<ViewError>>,
}

impl<'a> RetainedChildResourceRead<'a> {
    pub(super) fn new(
        root: &'a SourceDirectory,
        checkpoint: &'a mut dyn FnMut() -> Result<(), ViewError>,
    ) -> Self {
        Self {
            root,
            checkpoint: RefCell::new(checkpoint),
            fatal: RefCell::new(None),
        }
    }

    fn check(&self) -> io::Result<()> {
        if self.fatal.borrow().is_some() {
            return Err(io::Error::other("child resource read interrupted"));
        }
        let result = (self.checkpoint.borrow_mut())().and_then(|()| {
            self.root.validate_named_identity().map_err(|_| {
                ViewError::detailed(
                    RefusalDetail::SourceUnreadable,
                    "retained source root identity changed",
                )
            })
        });
        match result {
            Ok(()) => Ok(()),
            Err(error) => {
                *self.fatal.borrow_mut() = Some(error);
                Err(io::Error::other("child resource read interrupted"))
            }
        }
    }

    pub(super) fn finish(&self) -> Result<(), ViewError> {
        let _ = self.check();
        match self.fatal.borrow_mut().take() {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }

    fn relative<'p>(&self, path: &'p Path) -> io::Result<&'p Path> {
        let relative = if path.is_absolute() {
            path.strip_prefix(self.root.path()).map_err(|_| {
                io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "child resource is outside the retained source",
                )
            })?
        } else {
            path
        };
        if relative
            .components()
            .any(|component| !matches!(component, Component::Normal(_) | Component::CurDir))
        {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "child resource has an escaping path",
            ));
        }
        Ok(relative)
    }

    fn directory(&self, path: &Path) -> io::Result<SourceDirectory> {
        let mut directory = self.root.clone();
        for component in self.relative(path)?.components() {
            self.check()?;
            if let Component::Normal(name) = component {
                directory = directory.retain_directory_child(name)?;
            }
        }
        Ok(directory)
    }
}

impl TypedChildResourceRead for RetainedChildResourceRead<'_> {
    fn read_bytes(&self, path: &Path) -> io::Result<Vec<u8>> {
        self.check()?;
        let mut bytes = Vec::new();
        self.root.visit_relative_regular_chunks(
            self.relative(path)?,
            || self.check(),
            |chunk| {
                bytes.try_reserve(chunk.len()).map_err(io::Error::other)?;
                bytes.extend_from_slice(chunk);
                Ok(())
            },
        )?;
        Ok(bytes)
    }

    fn read_entries(&self, path: &Path) -> io::Result<DiscoveryEntries> {
        self.check()?;
        let directory = self.directory(path)?;
        let names = directory.read_immediate_names_with_limit(None, || self.check())?;
        Ok(Box::new(names.into_iter().map(move |name| {
            let path = directory.path().join(&name);
            Ok(DiscoveryEntry::from_port(RetainedChildEntry {
                parent: directory.clone(),
                name,
                path,
            }))
        })))
    }

    fn path_exists(&self, path: &Path) -> io::Result<bool> {
        let exists = || -> io::Result<()> {
            self.check()?;
            let relative = self.relative(path)?;
            let Some(name) = relative.file_name() else {
                return Ok(());
            };
            let parent = self.directory(relative.parent().unwrap_or(Path::new("")))?;
            parent.retain_immediate_child_nofollow(name)?;
            Ok(())
        };
        match exists() {
            Ok(()) => Ok(true),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
            Err(error) => Err(error),
        }
    }
}

struct RetainedChildEntry {
    parent: SourceDirectory,
    name: OsString,
    path: PathBuf,
}

impl DiscoveryEntryReadPort for RetainedChildEntry {
    fn file_name(&self) -> OsString {
        self.name.clone()
    }
    fn path(&self) -> PathBuf {
        self.path.clone()
    }
    fn file_type(&self) -> io::Result<DiscoveryFileType> {
        self.parent
            .retain_immediate_child_nofollow(OsStr::new(&self.name))
            .map(|entry| match entry {
                SourceEntry::Directory(_) => {
                    DiscoveryFileType::from_observation(false, true, false)
                }
                SourceEntry::RegularFile(_) => {
                    DiscoveryFileType::from_observation(true, false, false)
                }
                SourceEntry::ReparsePoint => {
                    DiscoveryFileType::from_observation(false, false, true)
                }
                SourceEntry::Unsupported => {
                    DiscoveryFileType::from_observation(false, false, false)
                }
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use unica_address::source_target::{MetadataAddress, PLATFORM_XML_8_3_27_FORMAT_2_20};
    use unica_metadata::MetaDiagnosticCode;
    use unica_metadata_designer::child_resources::observe_typed_child_resources_with;
    use unica_refusal::RefusalCode;
    use unica_workspace::test_support::retained_read_probe::{RetainedRead, RetainedReadProbe};

    const OWNER_XML: &str = r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Catalog><Properties><Name>Products</Name></Properties><ChildObjects><Template>Empty</Template></ChildObjects></Catalog></MetaDataObject>"#;

    fn binary_fixture() -> (tempfile::TempDir, PathBuf, MetadataAddress) {
        let temp = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(temp.path()).unwrap();
        let templates = root.join("Catalogs/Products/Templates");
        std::fs::create_dir_all(&templates).unwrap();
        let repo = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap();
        let descriptor =
            std::fs::read_to_string(repo.join(
                "tests/fixtures/platform_8_3_27/template-new-xml/native-binary-descriptor.xml",
            ))
            .unwrap()
            .replace("CorpusTemplate", "Empty");
        std::fs::write(templates.join("Empty.xml"), descriptor).unwrap();
        let owner =
            MetadataAddress::parse(PLATFORM_XML_8_3_27_FORMAT_2_20, "Catalog.Products").unwrap();
        (temp, root, owner)
    }

    #[test]
    fn retained_child_observation_preserves_cancellation_inside_large_payload() {
        let (_temp, path, owner) = binary_fixture();
        let payload = path.join("Catalogs/Products/Templates/Empty/Ext/Template.bin");
        std::fs::create_dir_all(payload.parent().unwrap()).unwrap();
        std::fs::write(&payload, vec![42; 8 * 1024 * 1024]).unwrap();
        let root = unica_workspace::read::open_source_directory(&path).unwrap();
        let reads = RetainedReadProbe::watch(&path);
        let mut calls = 0;
        let mut checkpoint = || {
            calls += 1;
            if calls == 64 {
                Err(ViewError::new(
                    RefusalCode::Cancelled,
                    "cancel inside child payload",
                ))
            } else {
                Ok(())
            }
        };
        let reader = RetainedChildResourceRead::new(&root, &mut checkpoint);
        let observed = observe_typed_child_resources_with(
            &reader,
            &path.join("Catalogs/Products.xml"),
            &owner,
            "Catalog",
            "Products",
            OWNER_XML,
        )
        .unwrap();
        assert!(observed
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == MetaDiagnosticCode::ProviderUnavailable));
        assert_eq!(reader.finish().unwrap_err().code(), RefusalCode::Cancelled);
        assert_eq!(calls, 64);
        assert!(reads.take().contains(&RetainedRead::Opened(PathBuf::from(
            "Catalogs/Products/Templates/Empty/Ext/Template.bin"
        ))));
        let mut checkpoint = || Ok(());
        let reader = RetainedChildResourceRead::new(&root, &mut checkpoint);
        let observed = observe_typed_child_resources_with(
            &reader,
            &path.join("Catalogs/Products.xml"),
            &owner,
            "Catalog",
            "Products",
            OWNER_XML,
        )
        .unwrap();
        reader.finish().unwrap();
        assert!(
            observed.diagnostics.is_empty(),
            "{:?}",
            observed.diagnostics
        );
        assert!(observed
            .validation_resources
            .iter()
            .any(|resource| resource.bytes.len() == 8 * 1024 * 1024));
    }

    #[test]
    fn retained_child_directory_listing_preserves_cancellation() {
        let temp = tempfile::tempdir().unwrap();
        let path = std::fs::canonicalize(temp.path()).unwrap();
        for index in 0..400 {
            std::fs::write(path.join(format!("{index}.bin")), []).unwrap();
        }
        let root = unica_workspace::read::open_source_directory(&path).unwrap();
        let mut calls = 0;
        let mut checkpoint = || {
            calls += 1;
            if calls == 32 {
                Err(ViewError::new(
                    RefusalCode::Cancelled,
                    "cancel child listing",
                ))
            } else {
                Ok(())
            }
        };
        let reader = RetainedChildResourceRead::new(&root, &mut checkpoint);
        assert!(reader.read_entries(&path).is_err());
        assert_eq!(reader.finish().unwrap_err().code(), RefusalCode::Cancelled);
        assert_eq!(calls, 32);
    }

    #[test]
    fn retained_child_reads_reject_escaping_routes_without_opening_outside() {
        let temp = tempfile::tempdir().unwrap();
        let path = std::fs::canonicalize(temp.path()).unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(path.join("own.bin"), b"own").unwrap();
        std::fs::write(outside.path().join("secret.bin"), b"secret").unwrap();
        let root = unica_workspace::read::open_source_directory(&path).unwrap();
        let reads = RetainedReadProbe::watch(outside.path());
        let mut checkpoint = || Ok(());
        let reader = RetainedChildResourceRead::new(&root, &mut checkpoint);
        assert_eq!(reader.read_bytes(&path.join("own.bin")).unwrap(), b"own");
        assert!(reader
            .read_bytes(&outside.path().join("secret.bin"))
            .is_err());
        assert!(reader.read_bytes(Path::new("../secret.bin")).is_err());
        assert!(reader.read_entries(outside.path()).is_err());
        assert!(reader.path_exists(Path::new("../secret.bin")).is_err());
        assert!(!reader.path_exists(Path::new("missing.bin")).unwrap());
        reader.finish().unwrap();
        assert!(reads.take().is_empty());
    }

    #[test]
    fn retained_child_root_replacement_is_fatal_instead_of_a_resource_limit() {
        if !unica_workspace::test_support::supports_retained_root_replacement_test() {
            unica_process::test_support::skip_without_host_capability(
                unica_process::test_support::HostCapability::RenameUnderOpenHandle,
                "the platform does not rename directories held by a read capability",
            );
            return;
        }
        let temp = tempfile::tempdir().unwrap();
        let base = std::fs::canonicalize(temp.path()).unwrap();
        let path = base.join("src");
        std::fs::create_dir(&path).unwrap();
        std::fs::write(path.join("own.bin"), b"original").unwrap();
        let root = unica_workspace::read::open_source_directory(&path).unwrap();
        let mut calls = 0;
        let mut checkpoint = || {
            calls += 1;
            if calls == 2 {
                std::fs::rename(&path, base.join("retained")).unwrap();
                std::fs::create_dir(&path).unwrap();
                std::fs::write(path.join("own.bin"), b"replacement").unwrap();
            }
            Ok(())
        };
        let reader = RetainedChildResourceRead::new(&root, &mut checkpoint);
        assert!(reader.read_bytes(&path.join("own.bin")).is_err());
        let error = reader.finish().unwrap_err();
        assert_eq!(error.detail(), Some(RefusalDetail::SourceUnreadable));
        assert!(error.to_string().contains("root identity changed"));
        assert!(!error
            .to_string()
            .contains(&path.to_string_lossy().to_string()));
    }

    #[test]
    fn unreadable_empty_binary_payload_is_not_reported_as_absent() {
        use unica_workspace::test_support::{set_unix_mode_for_test, unix_mode_for_test};
        struct Restore(PathBuf, u32);
        impl Drop for Restore {
            fn drop(&mut self) {
                let _ = set_unix_mode_for_test(&self.0, self.1);
            }
        }
        let temp = tempfile::tempdir().unwrap();
        let root_path = std::fs::canonicalize(temp.path()).unwrap();
        let templates = root_path.join("Catalogs/Products/Templates");
        let payload = templates.join("Empty");
        std::fs::create_dir_all(&payload).unwrap();
        let repo = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap();
        let descriptor =
            std::fs::read_to_string(repo.join(
                "tests/fixtures/platform_8_3_27/template-new-xml/native-binary-descriptor.xml",
            ))
            .unwrap()
            .replace("CorpusTemplate", "Empty");
        std::fs::write(templates.join("Empty.xml"), descriptor).unwrap();
        let root = unica_workspace::read::open_source_directory(&root_path).unwrap();
        let Some(mode) = unix_mode_for_test(&payload).unwrap() else {
            unica_process::test_support::skip_without_host_capability(
                unica_process::test_support::HostCapability::UnixPermissionBits,
                "the test needs Unix directory permission bits",
            );
            return;
        };
        let _restore = Restore(payload.clone(), mode);
        assert!(set_unix_mode_for_test(&payload, 0o000).unwrap());
        if std::fs::read_dir(&payload).is_ok() {
            unica_process::test_support::skip_without_host_capability(
                unica_process::test_support::HostCapability::EnforcedPermissionBits,
                "the test process can read directories despite their permission bits",
            );
            return;
        }
        let mut checkpoint = || Ok(());
        let reader = RetainedChildResourceRead::new(&root, &mut checkpoint);
        let owner =
            MetadataAddress::parse(PLATFORM_XML_8_3_27_FORMAT_2_20, "Catalog.Products").unwrap();
        let xml = r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Catalog><Properties><Name>Products</Name></Properties><ChildObjects><Template>Empty</Template></ChildObjects></Catalog></MetaDataObject>"#;
        let observed = observe_typed_child_resources_with(
            &reader,
            &root_path.join("Catalogs/Products.xml"),
            &owner,
            "Catalog",
            "Products",
            xml,
        )
        .unwrap();
        reader.finish().unwrap();
        assert!(
            observed.diagnostics.iter().any(|diagnostic| diagnostic.code
                == MetaDiagnosticCode::ProviderUnavailable
                && diagnostic
                    .metadata_path
                    .as_ref()
                    .is_some_and(|at| at.as_str() == "Catalog.Products.Template.Empty")),
            "unreadable topology cannot prove an empty template: {:?}",
            observed.diagnostics
        );
    }
}
