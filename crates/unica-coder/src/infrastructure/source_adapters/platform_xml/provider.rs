use std::{
    collections::BTreeMap,
    fs,
    io::Read,
    path::{Component, Path, PathBuf},
    sync::Arc,
};

use roxmltree::Document;
use sha2::{Digest, Sha256};

use crate::domain::source_adapters::{
    SourceAdapterError, SourceAdapterErrorKind, SourceRevision, TargetIdentity,
};

pub(crate) const MAX_CAPTURED_FILES: usize = 512;
pub(crate) const MAX_CAPTURED_FILE_BYTES: usize = 8 * 1024 * 1024;
pub(crate) const MAX_CAPTURED_TOTAL_BYTES: usize = 64 * 1024 * 1024;

#[derive(Debug)]
pub(crate) struct PlatformXmlProvider {
    files: BTreeMap<String, Arc<[u8]>>,
    configuration: Option<Arc<[u8]>>,
    parent_configurations: Option<Arc<[u8]>>,
    revision: SourceRevision,
    target_identity: TargetIdentity,
    descriptor_key: String,
    captured_root: PathBuf,
    captured_source_root: PathBuf,
}

impl PlatformXmlProvider {
    pub(crate) fn open(root: impl AsRef<Path>) -> Result<Self, SourceAdapterError> {
        let root = root.as_ref();
        Self::capture_root_with_hook(root, || {})
    }

    pub(crate) fn capture(
        target: impl AsRef<Path>,
        source_root: impl AsRef<Path>,
    ) -> Result<Self, SourceAdapterError> {
        Self::capture_with_hook(target.as_ref(), source_root.as_ref(), || {})
    }

    #[cfg(test)]
    fn open_with_test_hook(
        root: impl AsRef<Path>,
        after_first_capture: impl FnOnce(),
    ) -> Result<Self, SourceAdapterError> {
        Self::capture_root_with_hook(root.as_ref(), after_first_capture)
    }

    fn capture_root_with_hook(
        source_root: &Path,
        after_first_capture: impl FnOnce(),
    ) -> Result<Self, SourceAdapterError> {
        ensure_directory(source_root)?;
        let captured_source_root =
            fs::canonicalize(source_root).map_err(|_| unavailable("source root"))?;
        let target_identity = TargetIdentity::from_normalized_relative_path("Configuration.xml")?;
        let first = capture_contents(&captured_source_root, &captured_source_root, true)?;
        after_first_capture();
        let second = capture_contents(&captured_source_root, &captured_source_root, true)?;
        if first != second {
            return Err(stale());
        }
        let revision = revision_for(&first, &target_identity)?;
        Ok(Self {
            files: first.files,
            configuration: first.configuration,
            parent_configurations: first.parent_configurations,
            revision,
            target_identity,
            descriptor_key: "Configuration.xml".to_string(),
            captured_root: captured_source_root.clone(),
            captured_source_root,
        })
    }

    fn capture_with_hook(
        target: &Path,
        source_root: &Path,
        after_first_capture: impl FnOnce(),
    ) -> Result<Self, SourceAdapterError> {
        ensure_directory(source_root)?;
        ensure_no_symlink_components(source_root, target)?;
        let captured_source_root =
            fs::canonicalize(source_root).map_err(|_| unavailable("source root"))?;
        let requested_target = if target.is_absolute() {
            target.to_path_buf()
        } else {
            source_root.join(target)
        };
        let canonical_target =
            fs::canonicalize(&requested_target).map_err(|_| unavailable("Platform XML target"))?;
        let (descriptor, root_capture) =
            resolve_descriptor(&canonical_target, &captured_source_root)?;
        let descriptor_relative = relative_key(&captured_source_root, &descriptor)?;
        let target_identity = TargetIdentity::from_normalized_relative_path(&descriptor_relative)?;
        let captured_root = descriptor
            .parent()
            .expect("descriptor has parent")
            .to_path_buf();
        let descriptor_key = descriptor
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| unavailable("Platform XML descriptor has no UTF-8 file name"))?
            .to_string();
        let first = capture_contents(&descriptor, &captured_source_root, root_capture)?;
        after_first_capture();
        let second = capture_contents(&descriptor, &captured_source_root, root_capture)?;
        if first != second {
            return Err(SourceAdapterError::new(
                SourceAdapterErrorKind::SnapshotStale,
                "Platform XML aggregate changed while the snapshot was captured",
            ));
        }
        let revision = revision_for(&first, &target_identity)?;
        Ok(Self {
            files: first.files,
            configuration: first.configuration,
            parent_configurations: first.parent_configurations,
            revision,
            target_identity,
            descriptor_key,
            captured_root,
            captured_source_root,
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

    pub(crate) fn target_identity(&self) -> &TargetIdentity {
        &self.target_identity
    }

    pub(crate) fn descriptor_key(&self) -> &str {
        &self.descriptor_key
    }

    /// Canonical aggregate root whose bytes were captured. This is internal
    /// evidence only and must never be serialized into navigation output.
    pub(crate) fn captured_root(&self) -> &Path {
        &self.captured_root
    }

    pub(crate) fn captured_source_root(&self) -> &Path {
        &self.captured_source_root
    }

    /// Returns support evidence captured with this immutable aggregate.  The
    /// semantic reader must not reopen this file after snapshot acquisition.
    pub(crate) fn parent_configurations_bytes(&self) -> Option<Arc<[u8]>> {
        self.parent_configurations.clone()
    }

    pub(crate) fn configuration_bytes(&self) -> Option<Arc<[u8]>> {
        self.configuration.clone()
    }

    pub(crate) fn configuration_uuid(&self) -> Result<String, SourceAdapterError> {
        let bytes = self
            .configuration_bytes()
            .ok_or_else(|| unavailable("Configuration.xml is absent from the source snapshot"))?;
        let xml = std::str::from_utf8(bytes.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(&bytes))
            .map_err(|_| unavailable("Configuration.xml is not valid UTF-8"))?;
        let document =
            Document::parse(xml).map_err(|_| unavailable("Configuration.xml is malformed"))?;
        let root = document.root_element();
        if root.tag_name().name() != "MetaDataObject"
            || root.tag_name().namespace() != Some(super::schema::METADATA_NAMESPACE_2_20)
            || root.attribute("version") != Some("2.20")
        {
            return Err(unavailable(
                "Configuration.xml must have the official 2.20 MetaDataObject wrapper",
            ));
        }
        let children = root
            .children()
            .filter(|node| node.is_element())
            .collect::<Vec<_>>();
        let [configuration] = children.as_slice() else {
            return Err(unavailable(
                "Configuration.xml must contain exactly one Configuration element",
            ));
        };
        if configuration.tag_name().name() != "Configuration"
            || configuration.tag_name().namespace() != Some(super::schema::METADATA_NAMESPACE_2_20)
        {
            return Err(unavailable(
                "Configuration.xml Configuration element must use the official 2.20 namespace",
            ));
        }
        uuid::Uuid::parse_str(configuration.attribute("uuid").unwrap_or_default())
            .map(|uuid| uuid.to_string())
            .map_err(|_| unavailable("Configuration.xml has an invalid UUID"))
    }

    pub(crate) fn digest_relative(
        &self,
        raw: impl AsRef<Path>,
    ) -> Result<String, SourceAdapterError> {
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

#[derive(Debug, Clone, PartialEq, Eq)]
struct CaptureContents {
    files: BTreeMap<String, Arc<[u8]>>,
    configuration: Option<Arc<[u8]>>,
    parent_configurations: Option<Arc<[u8]>>,
}

#[derive(Default)]
struct CaptureBudget {
    files: usize,
    bytes: usize,
}

impl CaptureBudget {
    fn reserve(&mut self, declared: u64) -> Result<usize, SourceAdapterError> {
        let declared = usize::try_from(declared)
            .map_err(|_| resource_limit("Platform XML file size cannot be represented"))?;
        if self.files >= MAX_CAPTURED_FILES {
            return Err(resource_limit(
                "Platform XML capture exceeds the file limit",
            ));
        }
        if declared > MAX_CAPTURED_FILE_BYTES {
            return Err(resource_limit(
                "Platform XML file exceeds the per-file byte limit",
            ));
        }
        let next = self
            .bytes
            .checked_add(declared)
            .ok_or_else(|| resource_limit("Platform XML capture byte accounting overflow"))?;
        if next > MAX_CAPTURED_TOTAL_BYTES {
            return Err(resource_limit(
                "Platform XML capture exceeds the total byte limit",
            ));
        }
        self.files += 1;
        self.bytes = next;
        Ok(declared)
    }

    fn reconcile(&mut self, declared: usize, actual: usize) -> Result<(), SourceAdapterError> {
        if actual > MAX_CAPTURED_FILE_BYTES {
            return Err(resource_limit(
                "Platform XML file exceeds the per-file byte limit",
            ));
        }
        let next = self
            .bytes
            .checked_sub(declared)
            .and_then(|value| value.checked_add(actual))
            .ok_or_else(|| resource_limit("Platform XML capture byte accounting overflow"))?;
        if next > MAX_CAPTURED_TOTAL_BYTES {
            return Err(resource_limit(
                "Platform XML capture exceeds the total byte limit",
            ));
        }
        self.bytes = next;
        Ok(())
    }
}

fn capture_contents(
    descriptor: &Path,
    source_root: &Path,
    root_capture: bool,
) -> Result<CaptureContents, SourceAdapterError> {
    let mut budget = CaptureBudget::default();
    let mut files = BTreeMap::new();
    if root_capture {
        capture_directory(source_root, "", &mut files, &mut budget)?;
    } else {
        let descriptor_key = descriptor
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| unavailable("Platform XML descriptor has no UTF-8 file name"))?
            .to_string();
        insert_file(
            &mut files,
            descriptor_key.clone(),
            read_limited_regular_file(descriptor, &mut budget, "Platform XML descriptor")?,
        )?;
        let stem = descriptor_key
            .strip_suffix(".xml")
            .filter(|value| !value.is_empty())
            .ok_or_else(|| unavailable("Platform XML descriptor must have a .xml extension"))?;
        let companion = descriptor
            .parent()
            .expect("descriptor has parent")
            .join(stem);
        match fs::symlink_metadata(&companion) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return Err(unavailable("Platform XML companion aggregate")),
            Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
                return Err(unavailable(
                    "Platform XML companion aggregate must be a non-symlink directory",
                ))
            }
            Ok(_) => capture_directory(&companion, stem, &mut files, &mut budget)?,
        }
    }
    let configuration = if root_capture {
        files.get("Configuration.xml").cloned()
    } else {
        capture_optional_regular_file(source_root, "Configuration.xml", &mut budget)?
    };
    let parent_configurations = if root_capture {
        files.get("Ext/ParentConfigurations.bin").cloned()
    } else {
        capture_optional_regular_file(source_root, "Ext/ParentConfigurations.bin", &mut budget)?
    };
    Ok(CaptureContents {
        files,
        configuration,
        parent_configurations,
    })
}

fn capture_optional_regular_file(
    source_root: &Path,
    relative: &str,
    budget: &mut CaptureBudget,
) -> Result<Option<Arc<[u8]>>, SourceAdapterError> {
    if let Some(parent) = Path::new(relative)
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        let mut checked = source_root.to_path_buf();
        for component in parent.components() {
            let Component::Normal(name) = component else {
                return Err(unavailable("authorized source evidence path"));
            };
            checked.push(name);
            match fs::symlink_metadata(&checked) {
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
                Err(_) => return Err(unavailable("authorized source evidence")),
                Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
                    return Err(unavailable("authorized source evidence directory"))
                }
                Ok(_) => {}
            }
        }
    }
    let path = source_root.join(relative);
    match fs::symlink_metadata(&path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(_) => Err(unavailable("authorized source evidence")),
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => Err(
            unavailable("authorized source evidence must be a regular file"),
        ),
        Ok(_) => read_limited_regular_file(&path, budget, "authorized source evidence").map(Some),
    }
}

fn capture_directory(
    directory: &Path,
    prefix: &str,
    files: &mut BTreeMap<String, Arc<[u8]>>,
    budget: &mut CaptureBudget,
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
            capture_directory(&path, &key, files, budget)?;
        } else if metadata.is_file() {
            insert_file(
                files,
                key,
                read_limited_regular_file(&path, budget, "aggregate file")?,
            )?;
        } else {
            return Err(unavailable(
                "aggregate entry is not a regular file or directory",
            ));
        }
    }
    Ok(())
}

fn insert_file(
    files: &mut BTreeMap<String, Arc<[u8]>>,
    key: String,
    bytes: Arc<[u8]>,
) -> Result<(), SourceAdapterError> {
    if files.insert(key, bytes).is_some() {
        return Err(unavailable("aggregate contains duplicate relative keys"));
    }
    Ok(())
}

fn read_limited_regular_file(
    path: &Path,
    budget: &mut CaptureBudget,
    label: &str,
) -> Result<Arc<[u8]>, SourceAdapterError> {
    let before = fs::symlink_metadata(path).map_err(|_| unavailable(label))?;
    if before.file_type().is_symlink() || !before.is_file() {
        return Err(unavailable(
            "Platform XML capture requires regular non-symlink files",
        ));
    }
    let declared = budget.reserve(before.len())?;
    let mut reader = fs::File::open(path)
        .map_err(|_| unavailable(label))?
        .take((MAX_CAPTURED_FILE_BYTES as u64) + 1);
    let mut bytes = Vec::with_capacity(declared.min(MAX_CAPTURED_FILE_BYTES));
    reader
        .read_to_end(&mut bytes)
        .map_err(|_| unavailable(label))?;
    budget.reconcile(declared, bytes.len())?;
    let after = fs::symlink_metadata(path).map_err(|_| unavailable(label))?;
    if after.file_type().is_symlink() || !after.is_file() {
        return Err(unavailable("Platform XML file changed while being read"));
    }
    Ok(Arc::from(bytes))
}

fn resolve_descriptor(
    target: &Path,
    source_root: &Path,
) -> Result<(PathBuf, bool), SourceAdapterError> {
    let target = if target.is_absolute() {
        target.to_path_buf()
    } else {
        source_root.join(target)
    };
    if !target.starts_with(source_root) {
        return Err(unavailable(
            "Platform XML target is outside the authorized source root",
        ));
    }
    let metadata = fs::symlink_metadata(&target).map_err(|_| unavailable("Platform XML target"))?;
    if metadata.file_type().is_symlink() {
        return Err(unavailable("Platform XML target must not be a symlink"));
    }
    let (descriptor, root_capture) = if metadata.is_file() {
        (target, false)
    } else if metadata.is_dir() {
        if target == source_root {
            (source_root.join("Configuration.xml"), true)
        } else {
            let name = target
                .file_name()
                .and_then(|name| name.to_str())
                .filter(|name| !name.is_empty())
                .ok_or_else(|| unavailable("Platform XML directory target has no UTF-8 name"))?;
            (
                target
                    .parent()
                    .expect("directory has parent")
                    .join(format!("{name}.xml")),
                false,
            )
        }
    } else {
        return Err(unavailable(
            "Platform XML target is not a regular file or directory",
        ));
    };
    let metadata =
        fs::symlink_metadata(&descriptor).map_err(|_| unavailable("Platform XML descriptor"))?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(unavailable(
            "Platform XML descriptor must be a regular non-symlink file",
        ));
    }
    Ok((descriptor, root_capture))
}

fn ensure_no_symlink_components(
    source_root: &Path,
    target: &Path,
) -> Result<(), SourceAdapterError> {
    let target = if target.is_absolute() {
        target.to_path_buf()
    } else {
        source_root.join(target)
    };
    let relative = target
        .strip_prefix(source_root)
        .map_err(|_| unavailable("Platform XML target is outside the authorized source root"))?;
    let mut current = source_root.to_path_buf();
    for component in relative.components() {
        let Component::Normal(component) = component else {
            return Err(unavailable(
                "Platform XML target contains an invalid path component",
            ));
        };
        current.push(component);
        let metadata =
            fs::symlink_metadata(&current).map_err(|_| unavailable("Platform XML target"))?;
        if metadata.file_type().is_symlink() {
            return Err(unavailable(
                "Symbolic links are not allowed in Platform XML target paths",
            ));
        }
    }
    Ok(())
}

fn relative_key(source_root: &Path, target: &Path) -> Result<String, SourceAdapterError> {
    let relative = target
        .strip_prefix(source_root)
        .map_err(|_| unavailable("Platform XML target is outside the authorized source root"))?;
    normalized_relative_key(relative)
}

fn normalized_relative_key(raw: &Path) -> Result<String, SourceAdapterError> {
    if raw.as_os_str().is_empty() || raw.is_absolute() {
        return Err(unavailable(
            "requested path must be a non-empty relative path",
        ));
    }
    let mut parts = Vec::new();
    for component in raw.components() {
        let Component::Normal(part) = component else {
            return Err(unavailable(
                "requested path must not contain traversal components",
            ));
        };
        let part = part
            .to_str()
            .filter(|part| !part.is_empty() && !part.contains('\\') && !part.contains('/'))
            .ok_or_else(|| unavailable("requested path is not a normalized UTF-8 key"))?;
        parts.push(part);
    }
    Ok(parts.join("/"))
}

fn revision_for(
    contents: &CaptureContents,
    target_identity: &TargetIdentity,
) -> Result<SourceRevision, SourceAdapterError> {
    let mut digest = Sha256::new();
    digest.update(b"unica:platform-xml:target-snapshot:v2\0");
    digest.update((target_identity.as_str().len() as u64).to_be_bytes());
    digest.update(target_identity.as_str().as_bytes());
    for (key, bytes) in &contents.files {
        let file_digest = Sha256::digest(bytes);
        digest.update((key.len() as u64).to_be_bytes());
        digest.update(key.as_bytes());
        digest.update((bytes.len() as u64).to_be_bytes());
        digest.update(file_digest);
    }
    for (key, bytes) in [
        ("@source-root/Configuration.xml", &contents.configuration),
        (
            "@source-root/Ext/ParentConfigurations.bin",
            &contents.parent_configurations,
        ),
    ] {
        digest.update((key.len() as u64).to_be_bytes());
        digest.update(key.as_bytes());
        match bytes {
            Some(bytes) => {
                digest.update([1]);
                digest.update((bytes.len() as u64).to_be_bytes());
                digest.update(Sha256::digest(bytes));
            }
            None => digest.update([0]),
        }
    }
    SourceRevision::new(format!("sha256:{:x}", digest.finalize()))
}

fn unavailable(message: &str) -> SourceAdapterError {
    SourceAdapterError::new(SourceAdapterErrorKind::SourceUnavailable, message)
}

fn resource_limit(message: &str) -> SourceAdapterError {
    SourceAdapterError::new(SourceAdapterErrorKind::ResourceLimit, message)
}

fn stale() -> SourceAdapterError {
    SourceAdapterError::new(
        SourceAdapterErrorKind::SnapshotStale,
        "Platform XML capture changed while the snapshot was captured",
    )
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        path::PathBuf,
        sync::atomic::{AtomicU64, Ordering},
    };

    use super::{PlatformXmlProvider, MAX_CAPTURED_FILE_BYTES};
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
        let error = fixture
            .provider
            .read_relative("../outside.xml")
            .unwrap_err();

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

        assert_eq!(
            first.provider.revision().unwrap(),
            second.provider.revision().unwrap()
        );
    }

    #[test]
    fn unread_files_affect_the_aggregate_revision() {
        let without_extra = fixture(&[("Object.xml", b"one")]);
        let with_extra = fixture(&[("Object.xml", b"one"), ("Unread.xml", b"two")]);

        without_extra.provider.read_relative("Object.xml").unwrap();
        with_extra.provider.read_relative("Object.xml").unwrap();

        assert_ne!(
            without_extra.provider.revision().unwrap(),
            with_extra.provider.revision().unwrap()
        );
    }

    #[test]
    fn snapshot_is_immutable_after_successful_capture() {
        let fixture = fixture(&[("Object.xml", b"before")]);
        fs::write(fixture.root.join("Object.xml"), b"after").unwrap();
        fs::remove_file(fixture.root.join("Object.xml")).unwrap();

        assert_eq!(
            fixture
                .provider
                .read_relative("Object.xml")
                .unwrap()
                .as_ref(),
            b"before"
        );
    }

    #[test]
    fn support_bytes_are_immutable_after_successful_capture() {
        let fixture = fixture(&[("Ext/ParentConfigurations.bin", b"before" as &[u8])]);
        fs::write(fixture.root.join("Ext/ParentConfigurations.bin"), b"after").unwrap();

        assert_eq!(
            fixture
                .provider
                .parent_configurations_bytes()
                .unwrap()
                .as_ref(),
            b"before"
        );
    }

    #[test]
    fn target_capture_includes_only_its_aggregate_and_source_root_support() {
        let root = fixture_root(&[
            ("src/Documents/Shipment.xml", b"shipment"),
            ("src/Documents/Shipment/Ext/Form.xml", b"form"),
            ("src/Ext/ParentConfigurations.bin", b"support"),
            ("src/Configuration.xml", b"configuration"),
        ]);
        let target = root.join("src/Documents/Shipment.xml");
        let provider = PlatformXmlProvider::capture(&target, root.join("src")).unwrap();

        assert_eq!(
            provider.read_relative("Shipment.xml").unwrap().as_ref(),
            b"shipment"
        );
        assert_eq!(
            provider.parent_configurations_bytes().unwrap().as_ref(),
            b"support"
        );
        assert!(provider.read_relative("../Configuration.xml").is_err());
        assert!(serde_json::to_value(provider.revision().unwrap())
            .unwrap()
            .as_str()
            .unwrap()
            .starts_with("sha256:"));
    }

    #[test]
    fn sibling_targets_with_identical_bytes_have_distinct_capture_identity_and_revision() {
        let root = fixture_root(&[
            ("src/Catalogs/Items.xml", b"same"),
            ("src/Catalogs/Orders.xml", b"same"),
            ("src/Configuration.xml", b"configuration"),
        ]);
        let items =
            PlatformXmlProvider::capture(root.join("src/Catalogs/Items.xml"), root.join("src"))
                .unwrap();
        let orders =
            PlatformXmlProvider::capture(root.join("src/Catalogs/Orders.xml"), root.join("src"))
                .unwrap();
        assert_ne!(items.target_identity(), orders.target_identity());
        assert_ne!(items.revision().unwrap(), orders.revision().unwrap());
        assert!(items.read_relative("Orders.xml").is_err());
    }

    #[test]
    fn unrelated_oversized_sibling_is_not_read_but_actual_target_and_root_are_limited() {
        let root = fixture_root(&[
            ("src/Catalogs/Items.xml", b"items"),
            ("src/Configuration.xml", b"configuration"),
        ]);
        fs::write(
            root.join("src/Catalogs/Orders.xml"),
            vec![0_u8; MAX_CAPTURED_FILE_BYTES + 1],
        )
        .unwrap();
        assert!(PlatformXmlProvider::capture(
            root.join("src/Catalogs/Items.xml"),
            root.join("src")
        )
        .is_ok());
        assert_eq!(
            PlatformXmlProvider::capture(root.join("src"), root.join("src"))
                .unwrap_err()
                .kind,
            SourceAdapterErrorKind::ResourceLimit
        );
        fs::write(
            root.join("src/Catalogs/Items.xml"),
            vec![0_u8; MAX_CAPTURED_FILE_BYTES + 1],
        )
        .unwrap();
        assert_eq!(
            PlatformXmlProvider::capture(root.join("src/Catalogs/Items.xml"), root.join("src"))
                .unwrap_err()
                .kind,
            SourceAdapterErrorKind::ResourceLimit
        );
    }

    #[test]
    fn configuration_uuid_requires_official_220_wrapper_and_configuration_namespace() {
        let uuid = "11111111-1111-1111-1111-111111111111";
        let official = fixture(&[(
            "Configuration.xml",
            format!(r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Configuration uuid="{uuid}"/></MetaDataObject>"#).as_bytes(),
        )]);
        assert_eq!(official.provider.configuration_uuid().unwrap(), uuid);

        let alien_wrapper = fixture(&[(
            "Configuration.xml",
            format!(r#"<MetaDataObject xmlns="urn:alien" version="2.20"><Configuration uuid="{uuid}"/></MetaDataObject>"#).as_bytes(),
        )]);
        assert_eq!(
            alien_wrapper
                .provider
                .configuration_uuid()
                .unwrap_err()
                .kind,
            SourceAdapterErrorKind::SourceUnavailable
        );

        let alien_child = fixture(&[(
            "Configuration.xml",
            format!(r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" xmlns:alien="urn:alien" version="2.20"><alien:Configuration uuid="{uuid}"/></MetaDataObject>"#).as_bytes(),
        )]);
        assert_eq!(
            alien_child.provider.configuration_uuid().unwrap_err().kind,
            SourceAdapterErrorKind::SourceUnavailable
        );
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
        if let Some(result) =
            create_file_symlink_for_test(outside.join("outside.xml"), file_root.join("linked.xml"))
        {
            result.unwrap();
            assert_eq!(
                PlatformXmlProvider::open(&file_root).unwrap_err().kind,
                SourceAdapterErrorKind::SourceUnavailable
            );
        }

        let directory_root = fixture_root(&[]);
        if let Some(result) = create_dir_symlink_for_test(&outside, directory_root.join("linked")) {
            result.unwrap();
            assert_eq!(
                PlatformXmlProvider::open(&directory_root).unwrap_err().kind,
                SourceAdapterErrorKind::SourceUnavailable
            );
        }

        let root_link = std::env::temp_dir().join(format!(
            "unica-platform-xml-provider-root-link-{}-{}",
            std::process::id(),
            NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed),
        ));
        if let Some(result) = create_dir_symlink_for_test(&outside, &root_link) {
            result.unwrap();
            assert_eq!(
                PlatformXmlProvider::open(&root_link).unwrap_err().kind,
                SourceAdapterErrorKind::SourceUnavailable
            );
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
