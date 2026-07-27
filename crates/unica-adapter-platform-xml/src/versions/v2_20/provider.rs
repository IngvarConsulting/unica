use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::Read,
    path::{Component, Path, PathBuf},
    sync::Arc,
};

use sha2::{Digest, Sha256};

use crate::domain::source_adapters::{
    SourceAdapterError, SourceAdapterErrorKind, SourceRevision, TargetIdentity,
};

use super::xml::{parse_bounded_xml_document, BoundedXmlError};

pub(crate) const MAX_CAPTURED_FILES: usize = 512;
pub(crate) const MAX_CAPTURED_FILE_BYTES: usize = 8 * 1024 * 1024;
pub(crate) const MAX_CAPTURED_TOTAL_BYTES: usize = 64 * 1024 * 1024;
/// Maximum directories visited by one capture or its verification pass,
/// including empty directories and aggregate roots.
pub(crate) const MAX_CAPTURE_DIRECTORIES: usize = 2_048;
/// Maximum descendant depth below a captured aggregate root.
pub(crate) const MAX_CAPTURE_DEPTH: usize = 64;
const VERIFY_READ_BUFFER_BYTES: usize = 64 * 1024;

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

pub(crate) struct AuthorizedOperationalLocation {
    pub(crate) target: PathBuf,
    pub(crate) boundary: PathBuf,
    pub(crate) target_is_directory: bool,
    pub(crate) nearest_existing_directory: PathBuf,
    pub(crate) configuration_root: Option<PathBuf>,
}

impl PlatformXmlProvider {
    pub(crate) const fn coverage_manifest_json() -> &'static str {
        include_str!("coverage.json")
    }

    pub(crate) fn open(root: impl AsRef<Path>) -> Result<Self, SourceAdapterError> {
        let root = root.as_ref();
        Self::capture_root_with_hook(root, || {})
    }

    pub(crate) fn capture_authorized_root(
        root: &Path,
        authorized_root: &Path,
    ) -> Result<Option<Self>, SourceAdapterError> {
        let before = Self::authorize_unscoped_target(root, authorized_root)?;
        let metadata = match fs::symlink_metadata(&before.target) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                let after = Self::authorize_unscoped_target(root, authorized_root)?;
                if before.target != after.target || before.boundary != after.boundary {
                    return Err(stale());
                }
                return Ok(None);
            }
            Err(_) => return Err(unavailable("authorized source root")),
        };
        if metadata.file_type().is_symlink() || !metadata.is_dir() || !before.target_is_directory {
            return Err(unavailable(
                "authorized source root must be a regular directory",
            ));
        }
        let provider = Self::open(&before.target)?;
        let after = Self::authorize_unscoped_target(root, authorized_root)?;
        if before.target != after.target
            || before.boundary != after.boundary
            || !after.target_is_directory
            || provider.captured_source_root() != before.target
        {
            return Err(stale());
        }
        Ok(Some(provider))
    }

    pub(crate) fn capture(
        target: impl AsRef<Path>,
        source_root: impl AsRef<Path>,
    ) -> Result<Self, SourceAdapterError> {
        Self::capture_with_hook(target.as_ref(), source_root.as_ref(), || {})
    }

    pub(crate) fn capture_operational(
        target: impl AsRef<Path>,
        source_root: impl AsRef<Path>,
        allow_missing_target: bool,
    ) -> Result<Self, SourceAdapterError> {
        Self::capture_operational_with_hook(
            target.as_ref(),
            source_root.as_ref(),
            allow_missing_target,
            || {},
        )
    }

    pub(crate) fn authorize_unscoped_target(
        target: &Path,
        authorized_root: &Path,
    ) -> Result<AuthorizedOperationalLocation, SourceAdapterError> {
        Self::authorize_unscoped_target_with_hook(target, authorized_root, || {})
    }

    fn authorize_unscoped_target_with_hook(
        target: &Path,
        authorized_root: &Path,
        after_authorization: impl FnOnce(),
    ) -> Result<AuthorizedOperationalLocation, SourceAdapterError> {
        ensure_directory(authorized_root)?;
        let boundary =
            fs::canonicalize(authorized_root).map_err(|_| unavailable("authorized root"))?;
        let requested_target = if target.is_absolute() {
            target.to_path_buf()
        } else {
            authorized_root.join(target)
        };
        let (lexical_root, relative) =
            if let Ok(relative) = requested_target.strip_prefix(authorized_root) {
                (authorized_root, relative)
            } else if let Ok(relative) = requested_target.strip_prefix(&boundary) {
                (boundary.as_path(), relative)
            } else {
                return Err(unavailable(
                    "target is outside the authorized workspace boundary",
                ));
            };
        if !relative.as_os_str().is_empty() {
            normalized_relative_key(relative)?;
        }
        authorize_operational_target(&requested_target, lexical_root, &boundary, true)?;
        after_authorization();
        let target = boundary.join(relative);
        let target_is_directory = match fs::symlink_metadata(&requested_target) {
            Ok(metadata) => metadata.is_dir(),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                target.extension().is_none()
            }
            Err(_) => return Err(unavailable("target path")),
        };
        let mut current = if target_is_directory {
            Some(target.as_path())
        } else {
            target.parent()
        };
        let mut configuration_root = None;
        let mut nearest_existing_directory = None;
        while let Some(directory) = current {
            if !directory.starts_with(&boundary) {
                break;
            }
            let directory_exists = match fs::symlink_metadata(directory) {
                Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
                    return Err(unavailable(
                        "authorized source ancestor must be a regular directory",
                    ))
                }
                Ok(_) => {
                    if nearest_existing_directory.is_none() {
                        nearest_existing_directory = Some(directory.to_path_buf());
                    }
                    true
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
                Err(_) => return Err(unavailable("authorized source ancestor")),
            };
            let candidate = directory.join("Configuration.xml");
            match directory_exists
                .then(|| fs::symlink_metadata(&candidate))
                .transpose()
            {
                Ok(None) => {}
                Ok(Some(metadata))
                    if metadata.file_type().is_symlink() || !metadata.is_file() =>
                {
                    return Err(unavailable(
                        "authorized configuration evidence must be a regular file",
                    ))
                }
                Ok(Some(_)) => {
                    configuration_root = Some(directory.to_path_buf());
                    break;
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(_) => return Err(unavailable("authorized configuration evidence")),
            }
            if directory == boundary {
                break;
            }
            current = directory.parent();
        }
        Ok(AuthorizedOperationalLocation {
            target,
            boundary,
            target_is_directory,
            nearest_existing_directory: nearest_existing_directory.unwrap_or_else(|| {
                unreachable!("the authorized boundary is an existing directory")
            }),
            configuration_root,
        })
    }

    fn capture_operational_with_hook(
        target: &Path,
        source_root: &Path,
        allow_missing_target: bool,
        after_first_capture: impl FnOnce(),
    ) -> Result<Self, SourceAdapterError> {
        ensure_directory(source_root)?;
        let captured_source_root =
            fs::canonicalize(source_root).map_err(|_| unavailable("source root"))?;
        let requested_target = if target.is_absolute() {
            target.to_path_buf()
        } else {
            source_root.join(target)
        };
        let relative = requested_target
            .strip_prefix(source_root)
            .map_err(|_| unavailable("target is outside the authorized source root"))?;
        let target_key = if relative.as_os_str().is_empty() {
            "Configuration.xml".to_string()
        } else {
            normalized_relative_key(relative)?
        };
        let before_target = authorize_operational_target(
            &requested_target,
            source_root,
            &captured_source_root,
            allow_missing_target,
        )?;
        let first =
            capture_contents(&captured_source_root, &captured_source_root, true)?;
        after_first_capture();
        verify_contents(
            &captured_source_root,
            &captured_source_root,
            true,
            &first,
        )?;
        let after_target = authorize_operational_target(
            &requested_target,
            source_root,
            &captured_source_root,
            allow_missing_target,
        )?;
        if before_target != after_target {
            return Err(stale());
        }
        let target_identity = TargetIdentity::from_normalized_relative_path(&target_key)?;
        let revision = revision_for(&first, &target_identity)?;
        Ok(Self {
            files: first.files,
            configuration: first.configuration,
            parent_configurations: first.parent_configurations,
            revision,
            target_identity,
            descriptor_key: target_key,
            captured_root: captured_source_root.clone(),
            captured_source_root,
        })
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
        verify_contents(&captured_source_root, &captured_source_root, true, &first)?;
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
        verify_contents(&descriptor, &captured_source_root, root_capture, &first)?;
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
        let (_, document) = parse_bounded_xml_document(&bytes).map_err(|error| match error {
            BoundedXmlError::InvalidUtf8 => unavailable("Configuration.xml is not valid UTF-8"),
            BoundedXmlError::Malformed => unavailable("Configuration.xml is malformed"),
            BoundedXmlError::ResourceLimit => {
                resource_limit("Configuration.xml nesting depth exceeds navigation limit")
            }
        })?;
        let root = document.root_element();
        if root.tag_name().name() != "MetaDataObject"
            || root.tag_name().namespace() != Some(super::schema::METADATA_NAMESPACE_2_20)
            || root.attribute("version") != Some("2.20")
        {
            return Err(unavailable(
                "Configuration.xml must have the official 2.20 MetaDataObject wrapper",
            ));
        }
        let mut children = root.children().filter(|node| node.is_element());
        let Some(configuration) = children.next() else {
            return Err(unavailable(
                "Configuration.xml must contain exactly one Configuration element",
            ));
        };
        if children.next().is_some() {
            return Err(unavailable(
                "Configuration.xml must contain exactly one Configuration element",
            ));
        }
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

fn authorize_operational_target(
    requested_target: &Path,
    source_root: &Path,
    captured_source_root: &Path,
    allow_missing_target: bool,
) -> Result<Option<PathBuf>, SourceAdapterError> {
    let relative = requested_target
        .strip_prefix(source_root)
        .map_err(|_| unavailable("target is outside the authorized source root"))?;
    let mut requested_cursor = source_root.to_path_buf();
    let mut canonical_cursor = captured_source_root.to_path_buf();
    let mut missing = false;
    for component in relative.components() {
        let Component::Normal(component) = component else {
            return Err(unavailable("target contains an invalid path component"));
        };
        requested_cursor.push(component);
        canonical_cursor.push(component);
        if missing {
            continue;
        }
        match fs::symlink_metadata(&requested_cursor) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err(unavailable("target path must not contain symbolic links"))
            }
            Ok(_) => {
                let canonical = fs::canonicalize(&requested_cursor)
                    .map_err(|_| unavailable("target path identity"))?;
                if canonical != canonical_cursor || !canonical.starts_with(captured_source_root) {
                    return Err(unavailable("target canonical identity is inconsistent"));
                }
            }
            Err(error)
                if error.kind() == std::io::ErrorKind::NotFound && allow_missing_target =>
            {
                missing = true;
            }
            Err(_) => return Err(unavailable("target path")),
        }
    }
    if missing {
        Ok(None)
    } else {
        fs::canonicalize(requested_target)
            .map(Some)
            .map_err(|_| unavailable("target path identity"))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CaptureContents {
    files: BTreeMap<String, Arc<[u8]>>,
    directories: BTreeSet<String>,
    configuration: Option<Arc<[u8]>>,
    parent_configurations: Option<Arc<[u8]>>,
    companion_present: bool,
}

#[derive(Clone, Copy)]
struct CaptureReservation {
    declared: usize,
    read_limit: usize,
}

#[derive(Default)]
struct CaptureBudget {
    files: usize,
    directories: usize,
    bytes: usize,
}

impl CaptureBudget {
    fn reserve_directory(&mut self) -> Result<(), SourceAdapterError> {
        if self.directories >= MAX_CAPTURE_DIRECTORIES {
            return Err(resource_limit(
                "Platform XML capture exceeds the directory limit",
            ));
        }
        self.directories = self
            .directories
            .checked_add(1)
            .ok_or_else(|| resource_limit("Platform XML directory accounting overflow"))?;
        Ok(())
    }

    fn reserve(&mut self, declared: u64) -> Result<CaptureReservation, SourceAdapterError> {
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
        let remaining = MAX_CAPTURED_TOTAL_BYTES
            .checked_sub(self.bytes)
            .ok_or_else(|| resource_limit("Platform XML capture byte accounting overflow"))?;
        let read_limit = MAX_CAPTURED_FILE_BYTES.min(remaining);
        if declared > read_limit {
            return Err(resource_limit(
                "Platform XML capture exceeds the total byte limit",
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
        Ok(CaptureReservation {
            declared,
            read_limit,
        })
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
    let mut directories = BTreeSet::new();
    let mut companion_present = false;
    if root_capture {
        directories.extend(capture_directory(source_root, "", &mut files, &mut budget)?);
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
            Ok(_) => {
                companion_present = true;
                directories.extend(capture_directory(
                    &companion,
                    stem,
                    &mut files,
                    &mut budget,
                )?);
            }
        }
    }
    let configuration = if root_capture {
        if directories.contains("Configuration.xml") {
            return Err(unavailable(
                "authorized configuration evidence must be a regular file",
            ));
        }
        files.get("Configuration.xml").cloned()
    } else {
        capture_optional_regular_file(source_root, "Configuration.xml", &mut budget)?
    };
    let parent_configurations = if root_capture {
        if directories.contains("Ext/ParentConfigurations.bin") {
            return Err(unavailable(
                "authorized support evidence must be a regular file",
            ));
        }
        files.get("Ext/ParentConfigurations.bin").cloned()
    } else {
        capture_optional_regular_file(source_root, "Ext/ParentConfigurations.bin", &mut budget)?
    };
    Ok(CaptureContents {
        files,
        directories,
        configuration,
        parent_configurations,
        companion_present,
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
) -> Result<BTreeSet<String>, SourceAdapterError> {
    walk_directory(directory, prefix, budget, |path, key, budget| {
        insert_file(
            files,
            key.to_string(),
            read_limited_regular_file(path, budget, "aggregate file")?,
        )
    })
}

fn walk_directory<F>(
    directory: &Path,
    prefix: &str,
    budget: &mut CaptureBudget,
    mut visit_file: F,
) -> Result<BTreeSet<String>, SourceAdapterError>
where
    F: FnMut(&Path, &str, &mut CaptureBudget) -> Result<(), SourceAdapterError>,
{
    ensure_directory(directory)?;
    budget.reserve_directory()?;
    let mut directories = BTreeSet::new();
    if !prefix.is_empty() {
        directories.insert(prefix.to_string());
    }
    let mut pending = vec![(directory.to_path_buf(), prefix.to_string(), 0_usize)];
    while let Some((current, current_prefix, depth)) = pending.pop() {
        let entries = fs::read_dir(&current).map_err(|_| unavailable("aggregate directory"))?;
        for entry in entries {
            let entry = entry.map_err(|_| unavailable("aggregate directory entry"))?;
            let name = entry
                .file_name()
                .into_string()
                .map_err(|_| unavailable("aggregate entry has a non-UTF-8 name"))?;
            if name.is_empty() || name.contains('\\') || name.contains('/') {
                return Err(unavailable("aggregate entry has an invalid name"));
            }
            let key = if current_prefix.is_empty() {
                name
            } else {
                format!("{current_prefix}/{name}")
            };
            let path = entry.path();
            let metadata =
                fs::symlink_metadata(&path).map_err(|_| unavailable("aggregate entry"))?;
            if metadata.file_type().is_symlink() {
                return Err(unavailable("aggregate must not contain symlinks"));
            }
            if metadata.is_dir() {
                let next_depth = depth.checked_add(1).ok_or_else(|| {
                    resource_limit("Platform XML capture directory depth cannot be represented")
                })?;
                if next_depth > MAX_CAPTURE_DEPTH {
                    return Err(resource_limit(
                        "Platform XML capture exceeds the directory depth limit",
                    ));
                }
                budget.reserve_directory()?;
                directories.insert(key.clone());
                pending.push((path, key, next_depth));
            } else if metadata.is_file() {
                visit_file(&path, &key, budget)?;
            } else {
                return Err(unavailable(
                    "aggregate entry is not a regular file or directory",
                ));
            }
        }
    }
    Ok(directories)
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
    let reservation = budget.reserve(before.len())?;
    let mut reader = fs::File::open(path).map_err(|_| unavailable(label))?;
    let mut bytes = Vec::with_capacity(reservation.declared);
    let actual = read_bounded(&mut reader, reservation.read_limit, label, |chunk| {
        bytes.extend_from_slice(chunk);
    })?;
    budget.reconcile(reservation.declared, actual)?;
    let after = fs::symlink_metadata(path).map_err(|_| unavailable(label))?;
    if after.file_type().is_symlink() || !after.is_file() {
        return Err(unavailable("Platform XML file changed while being read"));
    }
    Ok(Arc::from(bytes))
}

fn read_bounded<R, F>(
    reader: &mut R,
    read_limit: usize,
    label: &str,
    mut consume: F,
) -> Result<usize, SourceAdapterError>
where
    R: Read,
    F: FnMut(&[u8]),
{
    let mut buffer = [0_u8; VERIFY_READ_BUFFER_BYTES];
    let mut total = 0_usize;
    while total < read_limit {
        let requested = (read_limit - total).min(buffer.len());
        let read = reader
            .read(&mut buffer[..requested])
            .map_err(|_| unavailable(label))?;
        if read == 0 {
            break;
        }
        total = total
            .checked_add(read)
            .ok_or_else(|| resource_limit("Platform XML capture byte accounting overflow"))?;
        consume(&buffer[..read]);
    }
    if total == read_limit {
        let mut sentinel = [0_u8; 1];
        if reader.read(&mut sentinel).map_err(|_| unavailable(label))? != 0 {
            return Err(resource_limit(
                "Platform XML file exceeds the bounded capture read limit",
            ));
        }
    }
    Ok(total)
}

fn verify_contents(
    descriptor: &Path,
    source_root: &Path,
    root_capture: bool,
    expected: &CaptureContents,
) -> Result<(), SourceAdapterError> {
    let mut budget = CaptureBudget::default();
    let mut verified_files = 0_usize;
    let mut verified_directories = BTreeSet::new();
    if root_capture {
        verified_directories.extend(verify_directory(
            source_root,
            "",
            &expected.files,
            &mut budget,
            &mut verified_files,
        )?);
    } else {
        let descriptor_key = descriptor
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| unavailable("Platform XML descriptor has no UTF-8 file name"))?;
        verify_expected_file(
            descriptor,
            descriptor_key,
            &expected.files,
            &mut budget,
            &mut verified_files,
            "Platform XML descriptor",
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
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                if expected.companion_present {
                    return Err(stale());
                }
            }
            Err(_) => return Err(unavailable("Platform XML companion aggregate")),
            Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
                return Err(unavailable(
                    "Platform XML companion aggregate must be a non-symlink directory",
                ))
            }
            Ok(_) => {
                if !expected.companion_present {
                    return Err(stale());
                }
                verified_directories.extend(verify_directory(
                    &companion,
                    stem,
                    &expected.files,
                    &mut budget,
                    &mut verified_files,
                )?);
            }
        }
        verify_optional_regular_file(
            source_root,
            "Configuration.xml",
            expected.configuration.as_deref(),
            &mut budget,
        )?;
        verify_optional_regular_file(
            source_root,
            "Ext/ParentConfigurations.bin",
            expected.parent_configurations.as_deref(),
            &mut budget,
        )?;
    }
    if verified_files != expected.files.len() || verified_directories != expected.directories {
        return Err(stale());
    }
    Ok(())
}

fn verify_directory(
    directory: &Path,
    prefix: &str,
    expected_files: &BTreeMap<String, Arc<[u8]>>,
    budget: &mut CaptureBudget,
    verified_files: &mut usize,
) -> Result<BTreeSet<String>, SourceAdapterError> {
    walk_directory(directory, prefix, budget, |path, key, budget| {
        verify_expected_file(
            path,
            key,
            expected_files,
            budget,
            verified_files,
            "aggregate file",
        )
    })
}

fn verify_expected_file(
    path: &Path,
    key: &str,
    expected_files: &BTreeMap<String, Arc<[u8]>>,
    budget: &mut CaptureBudget,
    verified_files: &mut usize,
    label: &str,
) -> Result<(), SourceAdapterError> {
    let expected = expected_files.get(key).ok_or_else(stale)?;
    *verified_files = verified_files
        .checked_add(1)
        .ok_or_else(|| resource_limit("Platform XML verification file accounting overflow"))?;
    if *verified_files > expected_files.len() {
        return Err(stale());
    }
    verify_limited_regular_file(path, budget, label, expected)
}

fn verify_optional_regular_file(
    source_root: &Path,
    relative: &str,
    expected: Option<&[u8]>,
    budget: &mut CaptureBudget,
) -> Result<(), SourceAdapterError> {
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
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    return if expected.is_some() {
                        Err(stale())
                    } else {
                        Ok(())
                    };
                }
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
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            if expected.is_some() {
                Err(stale())
            } else {
                Ok(())
            }
        }
        Err(_) => Err(unavailable("authorized source evidence")),
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => Err(
            unavailable("authorized source evidence must be a regular file"),
        ),
        Ok(_) => match expected {
            Some(expected) => {
                verify_limited_regular_file(&path, budget, "authorized source evidence", expected)
            }
            None => Err(stale()),
        },
    }
}

fn verify_limited_regular_file(
    path: &Path,
    budget: &mut CaptureBudget,
    label: &str,
    expected: &[u8],
) -> Result<(), SourceAdapterError> {
    let before = fs::symlink_metadata(path).map_err(|_| unavailable(label))?;
    if before.file_type().is_symlink() || !before.is_file() {
        return Err(unavailable(
            "Platform XML capture requires regular non-symlink files",
        ));
    }
    let reservation = budget.reserve(before.len())?;
    let mut reader = fs::File::open(path).map_err(|_| unavailable(label))?;
    let mut digest = Sha256::new();
    let actual = read_bounded(&mut reader, reservation.read_limit, label, |chunk| {
        digest.update(chunk);
    })?;
    budget.reconcile(reservation.declared, actual)?;
    let after = fs::symlink_metadata(path).map_err(|_| unavailable(label))?;
    if after.file_type().is_symlink() || !after.is_file() {
        return Err(unavailable("Platform XML file changed while being read"));
    }
    if actual != expected.len()
        || digest.finalize().as_slice() != Sha256::digest(expected).as_slice()
    {
        return Err(stale());
    }
    Ok(())
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
    digest.update(b"unica:platform-xml:target-snapshot:v3\0");
    digest.update((target_identity.as_str().len() as u64).to_be_bytes());
    digest.update(target_identity.as_str().as_bytes());
    for directory in &contents.directories {
        digest.update([b'D']);
        digest.update((directory.len() as u64).to_be_bytes());
        digest.update(directory.as_bytes());
    }
    for (key, bytes) in &contents.files {
        let file_digest = Sha256::digest(bytes);
        digest.update([b'F']);
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

#[cfg(test)]
mod operational_capture_tests {
    use super::PlatformXmlProvider;
    use std::{
        fs,
        sync::{
            atomic::{AtomicBool, Ordering},
            Arc,
        },
        time::{SystemTime, UNIX_EPOCH},
    };

    fn temp_root(label: &str) -> std::path::PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!(
            "unica-operational-capture-{label}-{}-{nanos}",
            std::process::id()
        ))
    }

    fn fixture(label: &str) -> (std::path::PathBuf, std::path::PathBuf) {
        let root = temp_root(label);
        fs::create_dir_all(root.join("Ext")).unwrap();
        let owner = root.join("Configuration.xml");
        fs::write(&owner, b"<owner/>").unwrap();
        fs::write(root.join("Ext/ParentConfigurations.bin"), b"first").unwrap();
        (root, owner)
    }

    #[test]
    fn operational_capture_rejects_support_evidence_changed_during_capture() {
        let (root, owner) = fixture("support-race");
        let support = root.join("Ext/ParentConfigurations.bin");

        let result = PlatformXmlProvider::capture_operational_with_hook(
            &owner,
            &root,
            false,
            || fs::write(&support, b"second").unwrap(),
        );

        assert!(result.is_err());
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn operational_capture_rejects_support_symlink_swap() {
        use std::os::unix::fs::symlink;

        let (root, owner) = fixture("support-symlink-race");
        let support = root.join("Ext/ParentConfigurations.bin");
        let outside = temp_root("support-symlink-race-outside");
        fs::write(&outside, b"outside").unwrap();

        let result = PlatformXmlProvider::capture_operational_with_hook(
            &owner,
            &root,
            false,
            || {
                fs::remove_file(&support).unwrap();
                symlink(&outside, &support).unwrap();
            },
        );

        assert!(result.is_err());
        fs::remove_dir_all(root).unwrap();
        fs::remove_file(outside).unwrap();
    }

    #[test]
    fn operational_capture_authorizes_containment_before_reading() {
        let (root, _) = fixture("authorize-first");
        let outside = temp_root("authorize-first-outside");
        fs::create_dir_all(&outside).unwrap();
        let target = outside.join("Object.xml");
        fs::write(&target, b"outside").unwrap();
        let hook_called = Arc::new(AtomicBool::new(false));
        let hook_probe = hook_called.clone();

        let result = PlatformXmlProvider::capture_operational_with_hook(
            &target,
            &root,
            false,
            move || hook_probe.store(true, Ordering::SeqCst),
        );

        assert!(result.is_err());
        assert!(!hook_called.load(Ordering::SeqCst));
        fs::remove_dir_all(root).unwrap();
        fs::remove_dir_all(outside).unwrap();
    }

    #[test]
    fn unscoped_owner_probe_runs_only_after_target_authorization() {
        let root = temp_root("unscoped-authorize-first");
        let target = root.join("Nested/Object.xml");
        fs::create_dir_all(target.parent().unwrap()).unwrap();
        fs::write(&target, b"<object/>").unwrap();
        let owner = root.join("Configuration.xml");
        let owner_for_hook = owner.clone();

        let location = PlatformXmlProvider::authorize_unscoped_target_with_hook(
            &target,
            &root,
            move || fs::write(owner_for_hook, b"<owner/>").unwrap(),
        )
        .unwrap();

        assert_eq!(
            location.configuration_root.unwrap(),
            root.canonicalize().unwrap()
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn unscoped_out_of_root_target_is_rejected_before_owner_probe() {
        let root = temp_root("unscoped-out-of-root");
        let outside = temp_root("unscoped-out-of-root-target");
        fs::create_dir_all(&root).unwrap();
        fs::create_dir_all(&outside).unwrap();
        let target = outside.join("Object.xml");
        fs::write(&target, b"<object/>").unwrap();
        let hook_called = Arc::new(AtomicBool::new(false));
        let hook_probe = hook_called.clone();

        let result = PlatformXmlProvider::authorize_unscoped_target_with_hook(
            &target,
            &root,
            move || hook_probe.store(true, Ordering::SeqCst),
        );

        assert!(result.is_err());
        assert!(!hook_called.load(Ordering::SeqCst));
        fs::remove_dir_all(root).unwrap();
        fs::remove_dir_all(outside).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn unscoped_owner_symlink_swap_after_authorization_is_rejected() {
        use std::os::unix::fs::symlink;

        let root = temp_root("unscoped-owner-swap");
        let outside = temp_root("unscoped-owner-swap-outside");
        let target = root.join("Nested/Object.xml");
        fs::create_dir_all(target.parent().unwrap()).unwrap();
        fs::write(&target, b"<object/>").unwrap();
        fs::write(&outside, b"<owner/>").unwrap();
        let owner = root.join("Configuration.xml");
        let outside_for_hook = outside.clone();

        let result = PlatformXmlProvider::authorize_unscoped_target_with_hook(
            &target,
            &root,
            move || symlink(&outside_for_hook, &owner).unwrap(),
        );

        assert!(result.is_err());
        fs::remove_dir_all(root).unwrap();
        fs::remove_file(outside).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn unscoped_target_canonical_mismatch_is_rejected_before_owner_probe() {
        use std::os::unix::fs::symlink;

        let root = temp_root("unscoped-target-mismatch");
        let real = root.join("Real");
        fs::create_dir_all(&real).unwrap();
        fs::write(real.join("Object.xml"), b"<object/>").unwrap();
        symlink(&real, root.join("Alias")).unwrap();
        let target = root.join("Alias/Object.xml");
        let hook_called = Arc::new(AtomicBool::new(false));
        let hook_probe = hook_called.clone();

        let result = PlatformXmlProvider::authorize_unscoped_target_with_hook(
            &target,
            &root,
            move || hook_probe.store(true, Ordering::SeqCst),
        );

        assert!(result.is_err());
        assert!(!hook_called.load(Ordering::SeqCst));
        fs::remove_dir_all(root).unwrap();
    }

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
        io::{self, Read},
        path::PathBuf,
        sync::atomic::{AtomicU64, Ordering},
    };

    use super::{
        read_bounded, CaptureBudget, PlatformXmlProvider, MAX_CAPTURED_FILE_BYTES,
        MAX_CAPTURED_TOTAL_BYTES, MAX_CAPTURE_DEPTH, MAX_CAPTURE_DIRECTORIES,
    };
    use crate::{
        domain::source_adapters::SourceAdapterErrorKind,
        infrastructure::platform::filesystem::{
            create_dir_symlink_for_test, create_file_symlink_for_test,
        },
    };

    static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(0);

    #[test]
    fn bounded_reads_use_only_the_remaining_aggregate_budget_and_sentinel() {
        let mut budget = CaptureBudget {
            files: 0,
            directories: 0,
            bytes: MAX_CAPTURED_TOTAL_BYTES - 3,
        };
        let reservation = budget.reserve(3).unwrap();
        assert_eq!(reservation.read_limit, 3);
        let mut reader = RecordingReader::new(b"four");

        let error = read_bounded(
            &mut reader,
            reservation.read_limit,
            "test bounded reader",
            |_| {},
        )
        .unwrap_err();

        assert_eq!(error.kind, SourceAdapterErrorKind::ResourceLimit);
        assert_eq!(reader.requests, vec![3, 1]);
    }

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
    fn configuration_uuid_preflights_depth_and_bounds_element_cardinality() {
        let uuid = "11111111-1111-1111-1111-111111111111";
        let mut deep = String::from(
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20">"#,
        );
        for _ in 1..crate::domain::navigation_limits::MAX_NAVIGATION_NESTING_DEPTH {
            deep.push_str("<Nested>");
        }
        deep.push_str("<Leaf/>");
        let deep_fixture = fixture(&[("Configuration.xml", deep.as_bytes())]);
        assert_eq!(
            deep_fixture.provider.configuration_uuid().unwrap_err().kind,
            SourceAdapterErrorKind::ResourceLimit
        );

        let max_plus_one = fixture(&[(
            "Configuration.xml",
            format!(
                r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Configuration uuid="{uuid}"/><Configuration uuid="{uuid}"/></MetaDataObject>"#
            )
            .as_bytes(),
        )]);
        assert_eq!(
            max_plus_one.provider.configuration_uuid().unwrap_err().kind,
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
    fn provider_rejects_excessive_directory_count_before_traversing_them() {
        let root = fixture_root(&[]);
        for index in 0..=MAX_CAPTURE_DIRECTORIES {
            fs::create_dir(root.join(format!("directory-{index}"))).unwrap();
        }

        let error = PlatformXmlProvider::open(&root).unwrap_err();

        assert_eq!(error.kind, SourceAdapterErrorKind::ResourceLimit);
    }

    #[test]
    fn provider_rejects_depth_beyond_the_bounded_iterative_walk() {
        let root = fixture_root(&[]);
        let mut current = root.clone();
        for depth in 0..=MAX_CAPTURE_DEPTH {
            current.push(format!("depth-{depth}"));
            fs::create_dir(&current).unwrap();
        }

        let error = PlatformXmlProvider::open(&root).unwrap_err();

        assert_eq!(error.kind, SourceAdapterErrorKind::ResourceLimit);
    }

    #[test]
    fn empty_directory_changes_between_capture_passes_are_snapshot_stale() {
        let removed_root = fixture_root(&[]);
        fs::create_dir(removed_root.join("empty")).unwrap();
        let root_for_removal = removed_root.clone();
        let removed = PlatformXmlProvider::open_with_test_hook(&removed_root, move || {
            fs::remove_dir(root_for_removal.join("empty")).unwrap();
        })
        .unwrap_err();
        assert_eq!(removed.kind, SourceAdapterErrorKind::SnapshotStale);

        let added_root = fixture_root(&[]);
        let root_for_addition = added_root.clone();
        let added = PlatformXmlProvider::open_with_test_hook(&added_root, move || {
            fs::create_dir(root_for_addition.join("empty")).unwrap();
        })
        .unwrap_err();
        assert_eq!(added.kind, SourceAdapterErrorKind::SnapshotStale);
    }

    #[test]
    fn ordinary_capture_retains_files_with_directory_tracking() {
        let root = fixture_root(&[("Nested/Object.xml", b"before")]);
        fs::create_dir(root.join("Empty")).unwrap();

        let provider = PlatformXmlProvider::open(&root).unwrap();

        assert_eq!(
            provider
                .read_relative("Nested/Object.xml")
                .unwrap()
                .as_ref(),
            b"before"
        );
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

    struct RecordingReader {
        bytes: Vec<u8>,
        offset: usize,
        requests: Vec<usize>,
    }

    impl RecordingReader {
        fn new(bytes: impl AsRef<[u8]>) -> Self {
            Self {
                bytes: bytes.as_ref().to_vec(),
                offset: 0,
                requests: Vec::new(),
            }
        }
    }

    impl Read for RecordingReader {
        fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
            self.requests.push(buffer.len());
            let remaining = &self.bytes[self.offset..];
            let read = remaining.len().min(buffer.len());
            buffer[..read].copy_from_slice(&remaining[..read]);
            self.offset += read;
            Ok(read)
        }
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
