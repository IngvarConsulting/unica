use std::{
    collections::BTreeMap,
    ffi::OsString,
    path::{Component, Path, PathBuf},
    sync::{Arc, OnceLock},
};

use sha2::{Digest, Sha256};

use crate::{
    domain::source_adapters::{
        SourceAdapterError, SourceAdapterErrorKind, SourceRevision, TargetIdentity,
    },
    safe_root::{
        ArtifactReadLimit, BoundArtifact, DirectoryPageLimit, DirectoryVisit, SafeRootError,
        SafeSourceRoot,
    },
};

use super::xml::{parse_bounded_xml_document, BoundedXmlError};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NavigationReadLimit {
    SelectedTarget,
}

impl NavigationReadLimit {
    const fn max_files(self) -> usize {
        match self {
            Self::SelectedTarget => 65_536,
        }
    }

    const fn max_directories(self) -> usize {
        match self {
            Self::SelectedTarget => 65_536,
        }
    }

    const fn max_total_bytes(self) -> usize {
        match self {
            Self::SelectedTarget => 512 * 1024 * 1024,
        }
    }

    const fn max_depth(self) -> usize {
        match self {
            Self::SelectedTarget => 64,
        }
    }
}

#[derive(Debug)]
struct NavigationReadBudget {
    limit: NavigationReadLimit,
    files: usize,
    directories: usize,
    bytes: usize,
}

impl NavigationReadBudget {
    fn selected_target() -> Self {
        Self {
            limit: NavigationReadLimit::SelectedTarget,
            files: 0,
            directories: 0,
            bytes: 0,
        }
    }

    fn reserve_directory(&mut self, depth: usize) -> Result<(), SourceAdapterError> {
        if depth > self.limit.max_depth() {
            return Err(resource_limit(
                "selected source scope exceeds the navigation depth limit",
            ));
        }
        self.directories = self
            .directories
            .checked_add(1)
            .ok_or_else(|| resource_limit("navigation directory accounting overflow"))?;
        if self.directories > self.limit.max_directories() {
            return Err(resource_limit(
                "selected source scope exceeds the navigation directory limit",
            ));
        }
        Ok(())
    }

    fn reserve_file(&mut self, bytes: usize) -> Result<(), SourceAdapterError> {
        self.files = self
            .files
            .checked_add(1)
            .ok_or_else(|| resource_limit("navigation file accounting overflow"))?;
        if self.files > self.limit.max_files() {
            return Err(resource_limit(
                "selected source scope exceeds the navigation file limit",
            ));
        }
        self.bytes = self
            .bytes
            .checked_add(bytes)
            .ok_or_else(|| resource_limit("navigation byte accounting overflow"))?;
        if self.bytes > self.limit.max_total_bytes() {
            return Err(resource_limit(
                "selected source scope exceeds the navigation byte limit",
            ));
        }
        Ok(())
    }
}

#[derive(Debug)]
pub(crate) struct PlatformXmlProvider {
    source_root: SafeSourceRoot,
    scope_root: SafeSourceRoot,
    descriptor_bytes: Arc<[u8]>,
    configuration: Option<Arc<[u8]>>,
    parent_configurations: Option<Arc<[u8]>>,
    revision: SourceRevision,
    target_identity: TargetIdentity,
    descriptor_key: String,
    navigation_files: OnceLock<BTreeMap<String, Arc<[u8]>>>,
}

impl PlatformXmlProvider {
    pub(crate) const fn coverage_manifest_json() -> &'static str {
        include_str!("coverage.json")
    }

    pub(crate) fn open(root: impl AsRef<Path>) -> Result<Self, SourceAdapterError> {
        let root = root.as_ref();
        Self::capture(root, root)
    }

    pub(crate) fn capture(
        target: impl AsRef<Path>,
        source_root: impl AsRef<Path>,
    ) -> Result<Self, SourceAdapterError> {
        Self::capture_with_hook(target.as_ref(), source_root.as_ref(), || {})
    }

    fn capture_with_hook(
        target: &Path,
        source_root: &Path,
        after_evidence_capture: impl FnOnce(),
    ) -> Result<Self, SourceAdapterError> {
        let root = SafeSourceRoot::capture(source_root, source_root)
            .map_err(|error| safe_error(error, "source root authorization failed"))?;
        let target = bind_requested_target(&root, target, source_root)?;
        let descriptor_relative = descriptor_relative(&target)?;
        let descriptor = root
            .bind_relative(&descriptor_relative, false)
            .map_err(|error| safe_error(error, "source descriptor authorization failed"))?;
        if descriptor.is_directory() || descriptor.is_missing() {
            return Err(unavailable("source descriptor is not a regular artifact"));
        }

        let descriptor_bytes = read_bound_arc(&root, &descriptor, ArtifactReadLimit::Descriptor)?;
        let configuration = if descriptor_relative == Path::new("Configuration.xml") {
            Some(Arc::clone(&descriptor_bytes))
        } else {
            optional_read_arc(
                &root,
                "Configuration.xml",
                ArtifactReadLimit::Descriptor,
                "configuration evidence",
            )?
        };
        let parent_configurations = optional_read_arc(
            &root,
            "Ext/ParentConfigurations.bin",
            ArtifactReadLimit::SupportEvidence,
            "support evidence",
        )?;

        after_evidence_capture();

        let descriptor_check =
            read_bound_arc(&root, &descriptor, ArtifactReadLimit::Descriptor)?;
        if descriptor_check.as_ref() != descriptor_bytes.as_ref() {
            return Err(stale());
        }
        let configuration_check = if descriptor_relative == Path::new("Configuration.xml") {
            Some(Arc::clone(&descriptor_check))
        } else {
            optional_read_arc(
                &root,
                "Configuration.xml",
                ArtifactReadLimit::Descriptor,
                "configuration evidence",
            )?
        };
        let parent_configurations_check = optional_read_arc(
            &root,
            "Ext/ParentConfigurations.bin",
            ArtifactReadLimit::SupportEvidence,
            "support evidence",
        )?;
        if !same_optional_bytes(&configuration, &configuration_check)
            || !same_optional_bytes(&parent_configurations, &parent_configurations_check)
        {
            return Err(stale());
        }

        let descriptor_source_key = descriptor
            .relative_key()
            .ok_or_else(|| unavailable("source descriptor identity is not normalized"))?;
        let target_identity =
            TargetIdentity::from_normalized_relative_path(&descriptor_source_key)?;
        let scope_prefix = descriptor_relative
            .parent()
            .unwrap_or_else(|| Path::new(""));
        let scope_root = root
            .subroot(scope_prefix)
            .map_err(|error| safe_error(error, "source descriptor scope changed"))?;
        let descriptor_key = descriptor_relative
            .file_name()
            .and_then(|name| name.to_str())
            .filter(|name| !name.is_empty())
            .ok_or_else(|| unavailable("source descriptor identity is not UTF-8"))?
            .to_string();
        let revision = revision_for(
            &target_identity,
            &descriptor_key,
            &descriptor_bytes,
            &configuration,
            &parent_configurations,
        )?;

        Ok(Self {
            source_root: root,
            scope_root,
            descriptor_bytes,
            configuration,
            parent_configurations,
            revision,
            target_identity,
            descriptor_key,
            navigation_files: OnceLock::new(),
        })
    }

    pub(crate) fn read_relative(
        &self,
        raw: impl AsRef<Path>,
    ) -> Result<Arc<[u8]>, SourceAdapterError> {
        let key = normalized_relative_key(raw.as_ref())?;
        if key == self.descriptor_key {
            return Ok(Arc::clone(&self.descriptor_bytes));
        }
        self.scope_root
            .read_relative(&key, ArtifactReadLimit::Descriptor)
            .map(|read| Arc::from(read.into_bytes()))
            .map_err(|error| safe_error(error, "selected source artifact is unavailable"))
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

    pub(crate) fn parent_configurations_bytes(&self) -> Option<Arc<[u8]>> {
        self.parent_configurations.clone()
    }

    pub(crate) fn configuration_bytes(&self) -> Option<Arc<[u8]>> {
        self.configuration.clone()
    }

    pub(crate) fn configuration_uuid(&self) -> Result<String, SourceAdapterError> {
        let bytes = self
            .configuration_bytes()
            .ok_or_else(|| unavailable("configuration descriptor evidence is absent"))?;
        let (_, document) = parse_bounded_xml_document(&bytes).map_err(|error| match error {
            BoundedXmlError::InvalidUtf8 => {
                unavailable("configuration descriptor is not valid UTF-8")
            }
            BoundedXmlError::Malformed => unavailable("configuration descriptor is malformed"),
            BoundedXmlError::ResourceLimit => {
                resource_limit("configuration descriptor exceeds the navigation nesting limit")
            }
        })?;
        let root = document.root_element();
        if root.tag_name().name() != "MetaDataObject"
            || root.tag_name().namespace() != Some(super::schema::METADATA_NAMESPACE_2_20)
            || root.attribute("version") != Some("2.20")
        {
            return Err(unavailable(
                "configuration descriptor has an unsupported root identity",
            ));
        }
        let mut children = root.children().filter(|node| node.is_element());
        let Some(configuration) = children.next() else {
            return Err(unavailable(
                "configuration descriptor has invalid element cardinality",
            ));
        };
        if children.next().is_some()
            || configuration.tag_name().name() != "Configuration"
            || configuration.tag_name().namespace()
                != Some(super::schema::METADATA_NAMESPACE_2_20)
        {
            return Err(unavailable(
                "configuration descriptor has invalid element identity",
            ));
        }
        uuid::Uuid::parse_str(configuration.attribute("uuid").unwrap_or_default())
            .map(|uuid| uuid.to_string())
            .map_err(|_| unavailable("configuration descriptor has an invalid identity"))
    }

    pub(crate) fn digest_relative(
        &self,
        raw: impl AsRef<Path>,
    ) -> Result<String, SourceAdapterError> {
        let bytes = self.read_relative(raw)?;
        Ok(format!("sha256:{:x}", Sha256::digest(bytes)))
    }

    pub(super) fn prepare_navigation_snapshot(&self) -> Result<(), SourceAdapterError> {
        if self.navigation_files.get().is_some() {
            return Ok(());
        }
        let mut files = BTreeMap::new();
        files.insert(
            self.descriptor_key.clone(),
            Arc::clone(&self.descriptor_bytes),
        );
        let mut budget = NavigationReadBudget::selected_target();
        budget.reserve_file(self.descriptor_bytes.len())?;

        let base_key = self
            .descriptor_key
            .strip_suffix(".xml")
            .ok_or_else(|| unavailable("source descriptor identity is not XML"))?;
        match self.scope_root.is_directory(Path::new(base_key)) {
            Ok(true) => self.collect_selected_directory(
                Path::new(base_key),
                1,
                &mut budget,
                &mut files,
            )?,
            Ok(false) | Err(SafeRootError::Missing) => {}
            Err(error) => {
                return Err(safe_error(
                    error,
                    "selected source companion scope is unavailable",
                ))
            }
        }

        let _ = self.navigation_files.set(files);
        Ok(())
    }

    pub(super) fn snapshot_files(&self) -> impl Iterator<Item = (&str, Arc<[u8]>)> + '_ {
        self.navigation_files
            .get()
            .expect("navigation snapshot must be prepared before decoding")
            .iter()
            .map(|(relative, bytes)| (relative.as_str(), Arc::clone(bytes)))
    }

    fn collect_selected_directory(
        &self,
        directory: &Path,
        depth: usize,
        budget: &mut NavigationReadBudget,
        files: &mut BTreeMap<String, Arc<[u8]>>,
    ) -> Result<(), SourceAdapterError> {
        budget.reserve_directory(depth)?;
        let directory_key = normalized_relative_key(directory)?;
        let mut names = Vec::<OsString>::new();
        self.scope_root
            .visit_directory(
                &directory_key,
                DirectoryPageLimit::MetadataRegistry,
                |name| {
                    names.push(name.to_os_string());
                    Ok(DirectoryVisit::Selected)
                },
            )
            .map_err(|error| safe_error(error, "selected source directory is unavailable"))?;

        for name in names {
            let relative = directory.join(name);
            match self.scope_root.is_directory(&relative) {
                Ok(true) => {
                    self.collect_selected_directory(&relative, depth + 1, budget, files)?;
                }
                Ok(false) => {
                    let key = normalized_relative_key(&relative)?;
                    if !self
                        .scope_root
                        .exists_regular(&key)
                        .map_err(|error| {
                            safe_error(error, "selected source artifact is unavailable")
                        })?
                    {
                        return Err(unavailable(
                            "selected source entry is not a regular artifact",
                        ));
                    }
                    let bytes: Arc<[u8]> = Arc::from(
                        self.scope_root
                            .read_relative(&key, ArtifactReadLimit::Descriptor)
                            .map_err(|error| {
                                safe_error(error, "selected source artifact is unavailable")
                            })?
                            .into_bytes(),
                    );
                    budget.reserve_file(bytes.len())?;
                    if files.insert(key, bytes).is_some() {
                        return Err(unavailable(
                            "selected source scope contains duplicate artifact identities",
                        ));
                    }
                }
                Err(error) => {
                    return Err(safe_error(
                        error,
                        "selected source entry is unavailable",
                    ))
                }
            }
        }
        Ok(())
    }
}

fn bind_requested_target(
    root: &SafeSourceRoot,
    target: &Path,
    source_root: &Path,
) -> Result<BoundArtifact, SourceAdapterError> {
    let result = if target.is_absolute() {
        match target.strip_prefix(source_root) {
            Ok(relative) => root.bind_relative(relative, false),
            Err(_) => root.bind_target(target, false),
        }
    } else {
        root.bind_relative(target, false)
    };
    result.map_err(|error| safe_error(error, "source target authorization failed"))
}

fn descriptor_relative(target: &BoundArtifact) -> Result<PathBuf, SourceAdapterError> {
    if !target.is_directory() {
        return Ok(target.relative().to_path_buf());
    }
    if target.is_source_root() {
        return Ok(PathBuf::from("Configuration.xml"));
    }
    let name = target
        .relative()
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .ok_or_else(|| unavailable("source directory identity is not UTF-8"))?;
    Ok(target
        .relative()
        .parent()
        .unwrap_or_else(|| Path::new(""))
        .join(format!("{name}.xml")))
}

fn read_bound_arc(
    root: &SafeSourceRoot,
    artifact: &BoundArtifact,
    limit: ArtifactReadLimit,
) -> Result<Arc<[u8]>, SourceAdapterError> {
    root.read_bound(artifact, limit)
        .map(|read| Arc::from(read.into_bytes()))
        .map_err(|error| safe_error(error, "source artifact evidence is unavailable"))
}

fn optional_read_arc(
    root: &SafeSourceRoot,
    relative: &str,
    limit: ArtifactReadLimit,
    context: &'static str,
) -> Result<Option<Arc<[u8]>>, SourceAdapterError> {
    match root.read_relative(relative, limit) {
        Ok(read) => Ok(Some(Arc::from(read.into_bytes()))),
        Err(SafeRootError::Missing) => Ok(None),
        Err(error) => Err(safe_error(error, context)),
    }
}

fn same_optional_bytes(left: &Option<Arc<[u8]>>, right: &Option<Arc<[u8]>>) -> bool {
    match (left, right) {
        (Some(left), Some(right)) => left.as_ref() == right.as_ref(),
        (None, None) => true,
        _ => false,
    }
}

fn normalized_relative_key(raw: &Path) -> Result<String, SourceAdapterError> {
    if raw.as_os_str().is_empty() || raw.is_absolute() {
        return Err(unavailable(
            "requested artifact identity must be non-empty and relative",
        ));
    }
    let mut parts = Vec::new();
    for component in raw.components() {
        let Component::Normal(part) = component else {
            return Err(unavailable(
                "requested artifact identity contains traversal",
            ));
        };
        let part = part
            .to_str()
            .filter(|part| !part.is_empty() && !part.contains('\\') && !part.contains('/'))
            .ok_or_else(|| unavailable("requested artifact identity is not normalized UTF-8"))?;
        parts.push(part);
    }
    Ok(parts.join("/"))
}

fn revision_for(
    target_identity: &TargetIdentity,
    descriptor_key: &str,
    descriptor: &[u8],
    configuration: &Option<Arc<[u8]>>,
    support: &Option<Arc<[u8]>>,
) -> Result<SourceRevision, SourceAdapterError> {
    let mut digest = Sha256::new();
    digest.update(b"unica:platform-xml:authorized-session:v4\0");
    digest.update((target_identity.as_str().len() as u64).to_be_bytes());
    digest.update(target_identity.as_str().as_bytes());
    digest.update((descriptor_key.len() as u64).to_be_bytes());
    digest.update(descriptor_key.as_bytes());
    digest.update((descriptor.len() as u64).to_be_bytes());
    digest.update(Sha256::digest(descriptor));
    for evidence in [configuration, support] {
        match evidence {
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

fn safe_error(error: SafeRootError, context: &'static str) -> SourceAdapterError {
    match error {
        SafeRootError::LimitExceeded => resource_limit(context),
        SafeRootError::IdentityChanged => stale(),
        _ => unavailable(context),
    }
}

fn unavailable(message: &'static str) -> SourceAdapterError {
    SourceAdapterError::new(SourceAdapterErrorKind::SourceUnavailable, message)
}

fn resource_limit(message: &'static str) -> SourceAdapterError {
    SourceAdapterError::new(SourceAdapterErrorKind::ResourceLimit, message)
}

fn stale() -> SourceAdapterError {
    SourceAdapterError::new(
        SourceAdapterErrorKind::SnapshotStale,
        "authorized source evidence changed during the operation",
    )
}

#[cfg(test)]
mod tests {
    use std::{
        fs::{self, File},
        sync::atomic::{AtomicU64, Ordering},
    };

    use super::*;

    static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(0);

    #[test]
    fn selected_navigation_scope_is_lazy_and_ignores_large_unrelated_content() {
        let root = fixture_root();
        write_descriptor(&root.join("Roles/Reader.xml"), "Role", "Reader");
        fs::create_dir_all(root.join("Roles/Reader/Ext")).unwrap();
        fs::write(root.join("Roles/Reader/Ext/Rights.xml"), b"<Rights/>").unwrap();
        for index in 0..700 {
            fs::write(root.join(format!("unrelated-{index}.bin")), b"x").unwrap();
        }
        let sparse = File::create(root.join("unrelated-sparse.bin")).unwrap();
        sparse.set_len(96 * 1024 * 1024).unwrap();

        let provider =
            PlatformXmlProvider::capture(root.join("Roles/Reader.xml"), &root).unwrap();
        assert!(provider.navigation_files.get().is_none());

        provider.prepare_navigation_snapshot().unwrap();
        let keys = provider
            .snapshot_files()
            .map(|(key, _)| key.to_string())
            .collect::<Vec<_>>();
        assert_eq!(keys, ["Reader.xml", "Reader/Ext/Rights.xml"]);
    }

    #[test]
    fn descriptor_evidence_change_is_snapshot_stale() {
        let root = fixture_root();
        let target = root.join("Order.xml");
        write_descriptor(&target, "Document", "Order");
        let replacement = target.clone();

        let error = PlatformXmlProvider::capture_with_hook(&target, &root, move || {
            write_descriptor(&replacement, "Document", "Changed");
        })
        .unwrap_err();

        assert_eq!(error.kind, SourceAdapterErrorKind::SnapshotStale);
    }

    #[test]
    fn target_symlink_is_rejected_before_any_artifact_read() {
        #[cfg(unix)]
        {
            use std::os::unix::fs::symlink;

            let root = fixture_root();
            let outside = fixture_root();
            write_descriptor(&outside.join("Outside.xml"), "Document", "Outside");
            symlink(outside.join("Outside.xml"), root.join("Linked.xml")).unwrap();

            let error = PlatformXmlProvider::capture(root.join("Linked.xml"), &root).unwrap_err();
            assert_eq!(error.kind, SourceAdapterErrorKind::SourceUnavailable);
        }
    }

    #[test]
    fn descriptor_has_an_explicit_per_artifact_limit() {
        let root = fixture_root();
        let target = root.join("Huge.xml");
        fs::write(&target, vec![0_u8; 8 * 1024 * 1024 + 1]).unwrap();

        let error = PlatformXmlProvider::capture(&target, &root).unwrap_err();

        assert_eq!(error.kind, SourceAdapterErrorKind::ResourceLimit);
    }

    #[test]
    fn configuration_identity_is_derived_from_captured_descriptor_evidence() {
        let root = fixture_root();
        let uuid = "11111111-1111-1111-1111-111111111111";
        fs::write(
            root.join("Configuration.xml"),
            format!(
                "<MetaDataObject xmlns=\"http://v8.1c.ru/8.3/MDClasses\" version=\"2.20\"><Configuration uuid=\"{uuid}\"/></MetaDataObject>"
            ),
        )
        .unwrap();

        let provider = PlatformXmlProvider::open(&root).unwrap();

        assert_eq!(provider.configuration_uuid().unwrap(), uuid);
    }

    fn fixture_root() -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "unica-platform-xml-lazy-provider-{}-{}",
            std::process::id(),
            NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed),
        ));
        fs::create_dir_all(&root).unwrap();
        root
    }

    fn write_descriptor(path: &Path, class: &str, name: &str) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(
            path,
            format!(
                "<MetaDataObject xmlns=\"http://v8.1c.ru/8.3/MDClasses\" version=\"2.20\"><{class}><Properties><Name>{name}</Name></Properties></{class}></MetaDataObject>"
            ),
        )
        .unwrap();
    }
}
