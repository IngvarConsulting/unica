#[cfg(unix)]
use super::contained_fs::observe_regular_file;
use super::contained_fs::{
    canonical_workspace, metadata_is_link_or_reparse_point, observe_open_file, open_no_follow,
    reject_link_components, resolve_contained_directory, slash_relative, FileObservation,
};
use super::platform_xml::{parse_configuration_registrations, parse_registered_descriptor};
use super::project_sources::resolve_source_selection;
use crate::application::discovery::ports::{
    SnapshotCaptureError, SnapshotCaptureReason, SourceSnapshotPort,
};
use crate::domain::discovery_registry::{EDT_DIAGNOSTIC_MARKERS_V1, SOURCE_ROOT_EXT_ARTIFACTS_V1};
use crate::domain::project_sources::SourceFormat;
use crate::domain::source_snapshot::{
    ManifestEntry, MaterialFile, OptionalMaterialTag, ResolvedSourceSet, SourceManifest,
    SourceReadError, SourceSetSnapshot, SourceSnapshot,
};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

pub(crate) const MAX_SNAPSHOT_FILES: usize = 200_000;
pub(crate) const MAX_SNAPSHOT_BYTES: u64 = 4 * 1024 * 1024 * 1024;
pub(crate) const MAX_SNAPSHOT_ELAPSED: Duration = Duration::from_secs(120);
pub(crate) const MAX_SNAPSHOT_TRAVERSAL_ENTRIES: usize = 1_600_000;
pub(crate) const MAX_SNAPSHOT_TRAVERSAL_DEPTH: usize = 64;
pub(crate) const MAX_SNAPSHOT_XML_BYTES: u64 = 64 * 1024 * 1024;
const OPTIONAL_PARENT_CONFIGURATIONS: &str = "Ext/ParentConfigurations.bin";
const IGNORED_REGISTERED_SUBTREE_DIRECTORIES: &[&str] = &[".git", ".build", "target", "dist"];

#[derive(Debug, Clone, Copy)]
struct SnapshotLimits {
    max_files: usize,
    max_bytes: u64,
    max_elapsed: Duration,
    max_traversal_entries: usize,
    max_traversal_depth: usize,
    max_xml_bytes: u64,
}

impl Default for SnapshotLimits {
    fn default() -> Self {
        Self {
            max_files: MAX_SNAPSHOT_FILES,
            max_bytes: MAX_SNAPSHOT_BYTES,
            max_elapsed: MAX_SNAPSHOT_ELAPSED,
            max_traversal_entries: MAX_SNAPSHOT_TRAVERSAL_ENTRIES,
            max_traversal_depth: MAX_SNAPSHOT_TRAVERSAL_DEPTH,
            max_xml_bytes: MAX_SNAPSHOT_XML_BYTES,
        }
    }
}

trait SnapshotClock: Send + Sync {
    fn now(&self) -> Duration;
}

#[allow(dead_code)]
struct SystemSnapshotClock {
    origin: Instant,
}

#[allow(dead_code)]
impl SystemSnapshotClock {
    fn new() -> Self {
        Self {
            origin: Instant::now(),
        }
    }
}

impl SnapshotClock for SystemSnapshotClock {
    fn now(&self) -> Duration {
        self.origin.elapsed()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum CaptureEvent {
    InitialPathScansComplete,
    #[cfg(unix)]
    BeforeContainedOpen(String),
    ContainedOpenEstablished(String),
    FileHashed(String),
    BeforeFinalIdentityValidation,
}

trait CaptureHook: Send + Sync {
    fn on_event(&self, event: &CaptureEvent);
}

#[allow(dead_code)]
struct NoopCaptureHook;

impl CaptureHook for NoopCaptureHook {
    fn on_event(&self, _event: &CaptureEvent) {}
}

pub(crate) struct FilesystemSourceSnapshots {
    workspace: PathBuf,
    limits: SnapshotLimits,
    clock: Arc<dyn SnapshotClock>,
    hook: Arc<dyn CaptureHook>,
}

impl FilesystemSourceSnapshots {
    // Constructed by Task 5 when concrete providers are wired publicly.
    #[allow(dead_code)]
    pub(crate) fn new(workspace: &Path) -> Result<Self, String> {
        Ok(Self {
            workspace: canonical_workspace(workspace)?,
            limits: SnapshotLimits::default(),
            clock: Arc::new(SystemSnapshotClock::new()),
            hook: Arc::new(NoopCaptureHook),
        })
    }

    fn capture_authoritative(
        &self,
        analysis: &ResolvedSourceSet,
        mutation_sources: &[ResolvedSourceSet],
        workspace_epoch: u64,
    ) -> Result<SourceSnapshot, SnapshotCaptureError> {
        let mutation_names = mutation_sources
            .iter()
            .map(|source| source.name.clone())
            .collect::<Vec<_>>();
        let before =
            resolve_source_selection(&self.workspace, Some(&analysis.name), &mutation_names)
                .map_err(classify_mapping_revalidation_error)?;
        if before.analysis != *analysis || before.mutations != mutation_sources {
            return Err(SnapshotCaptureError::source_changed(
                "source mapping no longer matches the resolved selection",
            ));
        }

        let started = self.clock.now();
        let mut budget = CaptureBudget::new(self.limits, started);
        let mut sources = Vec::with_capacity(1 + mutation_sources.len());
        sources.push(analysis.clone());
        sources.extend_from_slice(mutation_sources);
        let mut initial_plans = Vec::with_capacity(sources.len());
        for source in &sources {
            budget.check_deadline(self.clock.as_ref())?;
            let plan = scan_source_plan(&self.workspace, source, &mut budget, self.clock.as_ref())?;
            initial_plans.push(plan);
        }
        let unique_present = initial_plans
            .iter()
            .flat_map(|plan| plan.present.iter().cloned())
            .collect::<BTreeSet<_>>();
        budget.register_files(unique_present.len())?;
        self.hook.on_event(&CaptureEvent::InitialPathScansComplete);

        let mut captured = Vec::with_capacity(initial_plans.len());
        let mut material_cache = BTreeMap::<String, (MaterialFile, FileObservation)>::new();
        for plan in &initial_plans {
            let mut entries = BTreeMap::new();
            let mut observations = BTreeMap::new();
            for relative in &plan.present {
                budget.check_deadline(self.clock.as_ref())?;
                let (material, observation) = if let Some(cached) = material_cache.get(relative) {
                    cached.clone()
                } else {
                    let path = self.workspace.join(relative);
                    let read = read_stable_bytes(
                        &self.workspace,
                        &path,
                        budget.remaining_bytes()?,
                        Some(self.hook.as_ref()),
                    )?;
                    budget.register_bytes(read.bytes.len() as u64)?;
                    let digest = digest_bytes(&read.bytes);
                    let material = MaterialFile::new(read.bytes.len() as u64, digest)?;
                    let cached = (material, read.observation);
                    material_cache.insert(relative.clone(), cached.clone());
                    self.hook
                        .on_event(&CaptureEvent::FileHashed(relative.clone()));
                    cached
                };
                observations.insert(
                    relative.clone(),
                    (observation, material.content_digest.clone()),
                );
                entries.insert(relative.clone(), ManifestEntry::Present(material));
            }
            for (relative, tag) in &plan.absent_optional {
                entries.insert(relative.clone(), ManifestEntry::AbsentOptional(*tag));
            }
            captured.push((
                plan.source_set.clone(),
                SourceManifest::new(entries)?,
                observations,
            ));
        }

        let mut final_plans = Vec::with_capacity(sources.len());
        for source in &sources {
            budget.check_deadline(self.clock.as_ref())?;
            final_plans.push(
                scan_source_plan(&self.workspace, source, &mut budget, self.clock.as_ref())
                    .map_err(classify_final_scan_error)?,
            );
        }
        if initial_plans != final_plans {
            return Err(SnapshotCaptureError::source_changed(
                "authoritative path set changed during capture",
            ));
        }

        self.hook
            .on_event(&CaptureEvent::BeforeFinalIdentityValidation);
        let final_observations = captured
            .iter()
            .flat_map(|(_, _, observations)| observations.iter())
            .map(|(path, observation)| (path.clone(), observation.clone()))
            .collect::<BTreeMap<_, _>>();
        for (relative, (expected_observation, expected_digest)) in &final_observations {
            budget.check_deadline(self.clock.as_ref())?;
            let read = read_stable_bytes(
                &self.workspace,
                &self.workspace.join(relative),
                expected_observation.length,
                Some(self.hook.as_ref()),
            )
            .map_err(classify_final_present_revalidation_error)?;
            if &read.observation != expected_observation
                || &digest_bytes(&read.bytes) != expected_digest
            {
                return Err(SnapshotCaptureError::source_changed(format!(
                    "material file changed during capture: {relative}"
                )));
            }
        }
        let final_absent = captured
            .iter()
            .flat_map(|(_, manifest, _)| manifest.entries().iter())
            .filter_map(|(path, entry)| {
                matches!(entry, ManifestEntry::AbsentOptional(_)).then_some(path.clone())
            })
            .collect::<BTreeSet<_>>();
        for relative in final_absent {
            budget.check_deadline(self.clock.as_ref())?;
            let path = self.workspace.join(&relative);
            match std::fs::symlink_metadata(&path) {
                Ok(_) => {
                    return Err(SnapshotCaptureError::source_changed(format!(
                        "optional material appeared during capture: {relative}"
                    )));
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    reject_existing_ancestor_links(&self.workspace, &path).map_err(|detail| {
                        if detail.starts_with("symlink_or_reparse_escape:") {
                            SnapshotCaptureError::source_changed(format!(
                                "optional material topology changed during capture: {relative}"
                            ))
                        } else {
                            SnapshotCaptureError::classify(detail)
                        }
                    })?;
                }
                Err(error) => {
                    return Err(SnapshotCaptureError::classify(format!(
                        "material_file_unavailable: {}: {error}",
                        path.display()
                    )));
                }
            }
        }

        let after =
            resolve_source_selection(&self.workspace, Some(&analysis.name), &mutation_names)
                .map_err(classify_mapping_revalidation_error)?;
        if before != after {
            return Err(SnapshotCaptureError::source_changed(
                "source mapping changed during snapshot capture",
            ));
        }
        budget.check_deadline(self.clock.as_ref())?;

        let mut snapshots = captured
            .into_iter()
            .map(|(source, manifest, _)| SourceSetSnapshot::from_manifest(source, manifest))
            .collect::<Result<Vec<_>, _>>()?;
        let analysis_snapshot = snapshots.remove(0);
        Ok(SourceSnapshot::new(
            analysis_snapshot,
            snapshots,
            workspace_epoch,
        )?)
    }

    fn verified_read(
        &self,
        snapshot: &SourceSetSnapshot,
        workspace_relative_path: &str,
        optional: bool,
    ) -> Result<Option<Vec<u8>>, SourceReadError> {
        // Manifest membership is intentionally checked before touching the filesystem.
        let Some(entry) = snapshot.manifest.get(workspace_relative_path) else {
            return Err(SourceReadError::NotInManifest {
                path: workspace_relative_path.into(),
            });
        };
        if !path_belongs_to_source(&snapshot.source_set.relative_root, workspace_relative_path) {
            return Err(SourceReadError::NotInManifest {
                path: workspace_relative_path.into(),
            });
        }
        if let Err(detail) = snapshot.validate() {
            return Err(SourceReadError::SnapshotUnavailable {
                path: workspace_relative_path.into(),
                detail,
            });
        }
        match entry {
            ManifestEntry::AbsentOptional(_) => {
                if !optional {
                    return Err(SourceReadError::NotInManifest {
                        path: workspace_relative_path.into(),
                    });
                }
                match std::fs::symlink_metadata(self.workspace.join(workspace_relative_path)) {
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                        reject_existing_ancestor_links(
                            &self.workspace,
                            &self.workspace.join(workspace_relative_path),
                        )
                        .map_err(|detail| {
                            if detail.starts_with("symlink_or_reparse_escape:") {
                                SourceReadError::SourceFingerprintMismatch {
                                    path: workspace_relative_path.into(),
                                }
                            } else {
                                SourceReadError::SnapshotUnavailable {
                                    path: workspace_relative_path.into(),
                                    detail,
                                }
                            }
                        })?;
                        Ok(None)
                    }
                    Ok(_) => Err(SourceReadError::SourceFingerprintMismatch {
                        path: workspace_relative_path.into(),
                    }),
                    Err(error) => Err(SourceReadError::SnapshotUnavailable {
                        path: workspace_relative_path.into(),
                        detail: format!(
                            "material_file_unavailable: {workspace_relative_path}: {error}"
                        ),
                    }),
                }
            }
            ManifestEntry::Present(expected) => {
                let path = self.workspace.join(workspace_relative_path);
                match std::fs::symlink_metadata(&path) {
                    Ok(metadata)
                        if !metadata_is_link_or_reparse_point(&metadata)
                            && metadata.is_file()
                            && metadata.len() == expected.byte_length => {}
                    Ok(_) => {
                        return Err(SourceReadError::SourceFingerprintMismatch {
                            path: workspace_relative_path.into(),
                        });
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                        return Err(SourceReadError::SourceFingerprintMismatch {
                            path: workspace_relative_path.into(),
                        });
                    }
                    Err(error) => {
                        return Err(SourceReadError::SnapshotUnavailable {
                            path: workspace_relative_path.into(),
                            detail: format!(
                                "material_file_unavailable: {workspace_relative_path}: {error}"
                            ),
                        });
                    }
                }
                let read = read_stable_bytes(&self.workspace, &path, expected.byte_length, None)
                    .map_err(|detail| {
                        if detail.starts_with("source_snapshot_byte_limit:")
                            || detail.starts_with("source_snapshot_unavailable:")
                        {
                            SourceReadError::SourceFingerprintMismatch {
                                path: workspace_relative_path.into(),
                            }
                        } else {
                            SourceReadError::SnapshotUnavailable {
                                path: workspace_relative_path.into(),
                                detail,
                            }
                        }
                    })?;
                if read.bytes.len() as u64 != expected.byte_length
                    || digest_bytes(&read.bytes) != expected.content_digest
                {
                    return Err(SourceReadError::SourceFingerprintMismatch {
                        path: workspace_relative_path.into(),
                    });
                }
                Ok(Some(read.bytes))
            }
        }
    }
}

impl SourceSnapshotPort for FilesystemSourceSnapshots {
    fn capture(
        &self,
        analysis: &ResolvedSourceSet,
        mutation_sources: &[ResolvedSourceSet],
        workspace_epoch: u64,
    ) -> Result<SourceSnapshot, SnapshotCaptureError> {
        self.capture_authoritative(analysis, mutation_sources, workspace_epoch)
    }

    fn read_verified(
        &self,
        snapshot: &SourceSetSnapshot,
        workspace_relative_path: &str,
    ) -> Result<Vec<u8>, SourceReadError> {
        self.verified_read(snapshot, workspace_relative_path, false)?
            .ok_or_else(|| SourceReadError::NotInManifest {
                path: workspace_relative_path.into(),
            })
    }

    fn read_optional_verified(
        &self,
        snapshot: &SourceSetSnapshot,
        workspace_relative_path: &str,
    ) -> Result<Option<Vec<u8>>, SourceReadError> {
        self.verified_read(snapshot, workspace_relative_path, true)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SourcePlan {
    source_set: ResolvedSourceSet,
    present: BTreeSet<String>,
    absent_optional: BTreeMap<String, OptionalMaterialTag>,
}

fn scan_source_plan(
    workspace: &Path,
    source: &ResolvedSourceSet,
    budget: &mut CaptureBudget,
    clock: &dyn SnapshotClock,
) -> Result<SourcePlan, String> {
    source.validate()?;
    let root = resolve_contained_directory(workspace, &source.relative_root)?;
    match source.source_format {
        SourceFormat::PlatformXml => {
            scan_platform_xml_plan(workspace, &root, source, budget, clock)
        }
        SourceFormat::Edt => scan_edt_diagnostic_plan(workspace, &root, source, budget, clock),
        SourceFormat::Unknown | SourceFormat::Invalid => Err(format!(
            "unsupported_source_format: unsupported source format {:?}",
            source.source_format
        )),
    }
}

fn classify_final_scan_error(detail: String) -> SnapshotCaptureError {
    let classified = SnapshotCaptureError::classify(detail);
    match classified.reason {
        SnapshotCaptureReason::MalformedSourceMaterial
        | SnapshotCaptureReason::UnsupportedSourceLayout
        | SnapshotCaptureReason::InvalidSourcePath => {
            SnapshotCaptureError::source_changed(classified.detail)
        }
        SnapshotCaptureReason::UnsafeSourceTopology
            if !classified.detail.starts_with("file_identity_unavailable:") =>
        {
            SnapshotCaptureError::source_changed(classified.detail)
        }
        _ => classified,
    }
}

fn classify_mapping_revalidation_error(detail: String) -> SnapshotCaptureError {
    let classified = SnapshotCaptureError::classify(detail);
    match classified.reason {
        SnapshotCaptureReason::TransientSourceIo
        | SnapshotCaptureReason::SnapshotDeadlineExceeded
        | SnapshotCaptureReason::SnapshotResourceLimit => classified,
        SnapshotCaptureReason::UnsafeSourceTopology
            if classified.detail.starts_with("file_identity_unavailable:") =>
        {
            classified
        }
        _ => SnapshotCaptureError::source_changed(classified.detail),
    }
}

fn classify_final_present_revalidation_error(detail: String) -> SnapshotCaptureError {
    if detail.starts_with("source_snapshot_byte_limit:") {
        SnapshotCaptureError::source_changed(detail)
    } else {
        SnapshotCaptureError::classify(detail)
    }
}

fn scan_platform_xml_plan(
    workspace: &Path,
    root: &Path,
    source: &ResolvedSourceSet,
    budget: &mut CaptureBudget,
    clock: &dyn SnapshotClock,
) -> Result<SourcePlan, String> {
    let mut present = BTreeSet::new();
    let configuration_path = root.join("Configuration.xml");
    let configuration = read_stable_bytes(
        workspace,
        &configuration_path,
        budget.limits.max_xml_bytes,
        None,
    )?;
    let configuration_relative = slash_relative(workspace, &configuration_path)?;
    present.insert(configuration_relative);
    let registrations = parse_configuration_registrations(&configuration.bytes)?;
    collect_exact_ext_files(
        workspace,
        &root.join("Ext"),
        SOURCE_ROOT_EXT_ARTIFACTS_V1,
        &mut present,
        budget,
        clock,
    )?;
    for registration in registrations {
        budget.check_deadline(clock)?;
        let descriptor = root
            .join(&registration.directory)
            .join(format!("{}.xml", registration.name));
        require_regular_material(workspace, &descriptor)?;
        let descriptor_bytes =
            read_stable_bytes(workspace, &descriptor, budget.limits.max_xml_bytes, None)?;
        present.insert(slash_relative(workspace, &descriptor)?);
        let nested = parse_registered_descriptor(&descriptor_bytes.bytes, &registration)?;
        collect_registered_subtree(
            workspace,
            &root
                .join(&registration.directory)
                .join(&registration.name)
                .join("Ext"),
            &mut present,
            budget,
            clock,
        )?;
        for (collection, names) in [
            ("Forms", nested.forms),
            ("Templates", nested.templates),
            ("Commands", nested.commands),
        ] {
            for name in names {
                let nested_descriptor = root
                    .join(&registration.directory)
                    .join(&registration.name)
                    .join(collection)
                    .join(format!("{name}.xml"));
                require_regular_material(workspace, &nested_descriptor)?;
                present.insert(slash_relative(workspace, &nested_descriptor)?);
                collect_registered_subtree(
                    workspace,
                    &root
                        .join(&registration.directory)
                        .join(&registration.name)
                        .join(collection)
                        .join(&name)
                        .join("Ext"),
                    &mut present,
                    budget,
                    clock,
                )?;
            }
        }
    }

    let parent_configurations = root.join(OPTIONAL_PARENT_CONFIGURATIONS);
    let parent_relative = slash_relative(workspace, &parent_configurations)?;
    let mut absent_optional = BTreeMap::new();
    match std::fs::symlink_metadata(&parent_configurations) {
        Ok(metadata) => {
            if metadata_is_link_or_reparse_point(&metadata) || !metadata.is_file() {
                return Err(format!(
                    "material_file_not_regular: {}",
                    parent_configurations.display()
                ));
            }
            present.insert(parent_relative);
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            reject_existing_ancestor_links(workspace, &parent_configurations)?;
            absent_optional.insert(parent_relative, OptionalMaterialTag::ParentConfigurations);
        }
        Err(error) => {
            return Err(format!(
                "material_file_unavailable: {}: {error}",
                parent_configurations.display()
            ));
        }
    }
    Ok(SourcePlan {
        source_set: source.clone(),
        present,
        absent_optional,
    })
}

fn scan_edt_diagnostic_plan(
    workspace: &Path,
    root: &Path,
    source: &ResolvedSourceSet,
    budget: &mut CaptureBudget,
    clock: &dyn SnapshotClock,
) -> Result<SourcePlan, String> {
    let mut present = BTreeSet::new();
    let mut absent_optional = BTreeMap::new();
    for (relative, tag) in [
        (
            EDT_DIAGNOSTIC_MARKERS_V1[0],
            OptionalMaterialTag::EdtProject,
        ),
        (
            EDT_DIAGNOSTIC_MARKERS_V1[1],
            OptionalMaterialTag::EdtProjectPmf,
        ),
        (
            EDT_DIAGNOSTIC_MARKERS_V1[2],
            OptionalMaterialTag::EdtConfigurationMdo,
        ),
        (
            EDT_DIAGNOSTIC_MARKERS_V1[3],
            OptionalMaterialTag::EdtSourceConfigurationMdo,
        ),
    ] {
        budget.check_deadline(clock)?;
        let path = root.join(relative);
        match std::fs::symlink_metadata(&path) {
            Ok(metadata) => {
                if metadata_is_link_or_reparse_point(&metadata) || !metadata.is_file() {
                    return Err(format!("material_file_not_regular: {}", path.display()));
                }
                present.insert(slash_relative(workspace, &path)?);
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                reject_existing_ancestor_links(workspace, &path)?;
                absent_optional.insert(slash_relative(workspace, &path)?, tag);
            }
            Err(error) => {
                return Err(format!(
                    "material_file_unavailable: {}: {error}",
                    path.display()
                ))
            }
        }
    }
    if present.is_empty() {
        return Err("source_snapshot_unavailable: EDT diagnostic markers disappeared".into());
    }
    Ok(SourcePlan {
        source_set: source.clone(),
        present,
        absent_optional,
    })
}

fn collect_exact_ext_files(
    workspace: &Path,
    base: &Path,
    allowed: &[&str],
    present: &mut BTreeSet<String>,
    budget: &mut CaptureBudget,
    clock: &dyn SnapshotClock,
) -> Result<(), String> {
    let metadata = match std::fs::symlink_metadata(base) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => {
            return Err(format!(
                "material_subtree_unavailable: {}: {error}",
                base.display()
            ))
        }
    };
    if metadata_is_link_or_reparse_point(&metadata) || !metadata.is_dir() {
        return Err(format!(
            "material_subtree_not_directory: {}",
            base.display()
        ));
    }
    reject_link_components(workspace, base)?;
    for relative in allowed {
        budget.check_deadline(clock)?;
        budget.register_traversal_entry()?;
        let path = base.join(relative);
        match std::fs::symlink_metadata(&path) {
            Ok(metadata) if metadata_is_link_or_reparse_point(&metadata) || !metadata.is_file() => {
                return Err(format!("material_file_not_regular: {}", path.display()));
            }
            Ok(_) => {
                present.insert(slash_relative(workspace, &path)?);
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(format!(
                    "material_file_unavailable: {}: {error}",
                    path.display()
                ))
            }
        }
    }
    Ok(())
}

fn collect_registered_subtree(
    workspace: &Path,
    base: &Path,
    present: &mut BTreeSet<String>,
    budget: &mut CaptureBudget,
    clock: &dyn SnapshotClock,
) -> Result<(), String> {
    let metadata = match std::fs::symlink_metadata(base) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => {
            return Err(format!(
                "material_subtree_unavailable: {}: {error}",
                base.display()
            ))
        }
    };
    if metadata_is_link_or_reparse_point(&metadata) || !metadata.is_dir() {
        return Err(format!(
            "material_subtree_not_directory: {}",
            base.display()
        ));
    }
    reject_link_components(workspace, base)?;
    let mut pending = vec![(base.to_path_buf(), 0usize)];
    while let Some((directory, depth)) = pending.pop() {
        if depth > budget.limits.max_traversal_depth {
            return Err("source_snapshot_traversal_depth: authoritative snapshot discarded".into());
        }
        budget.check_deadline(clock)?;
        let entries = std::fs::read_dir(&directory).map_err(|error| {
            format!(
                "material_subtree_unreadable: {}: {error}",
                directory.display()
            )
        })?;
        let mut entries = entries.collect::<Result<Vec<_>, _>>().map_err(|error| {
            format!(
                "material_subtree_unreadable: {}: {error}",
                directory.display()
            )
        })?;
        entries.sort_by_key(|entry| entry.file_name());
        let mut child_directories = Vec::new();
        for entry in entries {
            budget.register_traversal_entry()?;
            let path = entry.path();
            let metadata = std::fs::symlink_metadata(&path).map_err(|error| {
                format!("material_file_unavailable: {}: {error}", path.display())
            })?;
            if metadata_is_link_or_reparse_point(&metadata) {
                return Err(format!("symlink_or_reparse_escape: {}", path.display()));
            }
            if metadata.is_dir() {
                let ignored = entry
                    .file_name()
                    .to_str()
                    .is_some_and(|name| IGNORED_REGISTERED_SUBTREE_DIRECTORIES.contains(&name));
                if !ignored {
                    let next_depth = depth
                        .checked_add(1)
                        .ok_or_else(|| "source_snapshot_traversal_depth: overflow".to_string())?;
                    child_directories.push((path, next_depth));
                }
            } else if metadata.is_file() {
                present.insert(slash_relative(workspace, &path)?);
            } else {
                return Err(format!("material_file_not_regular: {}", path.display()));
            }
        }
        pending.extend(child_directories.into_iter().rev());
    }
    Ok(())
}

fn require_regular_material(workspace: &Path, path: &Path) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| format!("invalid_material_path: {}", path.display()))?;
    reject_link_components(workspace, parent)?;
    let metadata = std::fs::symlink_metadata(path)
        .map_err(|error| format!("registered_material_missing: {}: {error}", path.display()))?;
    if metadata_is_link_or_reparse_point(&metadata) || !metadata.is_file() {
        return Err(format!("material_file_not_regular: {}", path.display()));
    }
    Ok(())
}

fn reject_existing_ancestor_links(workspace: &Path, path: &Path) -> Result<(), String> {
    let relative = path
        .strip_prefix(workspace)
        .map_err(|_| format!("path_escape: {}", path.display()))?;
    let mut current = workspace.to_path_buf();
    for component in relative.components() {
        current.push(component.as_os_str());
        match std::fs::symlink_metadata(&current) {
            Ok(metadata) if metadata_is_link_or_reparse_point(&metadata) => {
                return Err(format!("symlink_or_reparse_escape: {}", current.display()));
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => break,
            Err(error) => return Err(format!("path_unavailable: {}: {error}", current.display())),
        }
    }
    Ok(())
}

struct StableRead {
    bytes: Vec<u8>,
    observation: FileObservation,
}

fn read_stable_bytes(
    workspace: &Path,
    path: &Path,
    max_bytes: u64,
    hook: Option<&dyn CaptureHook>,
) -> Result<StableRead, String> {
    reject_link_components(workspace, path)?;
    let before_path_metadata = std::fs::symlink_metadata(path)
        .map_err(|error| format!("material_file_unreadable: {}: {error}", path.display()))?;
    if metadata_is_link_or_reparse_point(&before_path_metadata) || !before_path_metadata.is_file() {
        return Err(format!("material_file_not_regular: {}", path.display()));
    }
    #[cfg(unix)]
    let before = observe_regular_file(&before_path_metadata, path)?;
    let before_length = before_path_metadata.len();
    if before_length > max_bytes {
        return Err(format!("source_snapshot_byte_limit: {}", path.display()));
    }
    #[cfg(unix)]
    if let Some(hook) = hook {
        hook.on_event(&CaptureEvent::BeforeContainedOpen(slash_relative(
            workspace, path,
        )?));
    }
    let mut contained = open_no_follow(workspace, path).map_err(|detail| {
        classify_open_failure_after_observation(
            workspace,
            path,
            &before_path_metadata,
            #[cfg(unix)]
            &before,
            detail,
        )
    })?;
    let opened = observe_open_file(contained.file(), path).map_err(|detail| {
        if detail.starts_with("material_file_not_regular:") {
            format!(
                "source_snapshot_unavailable: opened material type changed after observation: {}",
                path.display()
            )
        } else {
            detail
        }
    })?;
    if let Some(hook) = hook {
        hook.on_event(&CaptureEvent::ContainedOpenEstablished(slash_relative(
            workspace, path,
        )?));
    }
    #[cfg(unix)]
    if before != opened {
        return Err(format!(
            "source_snapshot_unavailable: concurrent replacement: {}",
            path.display()
        ));
    }
    #[cfg(windows)]
    if before_length != opened.length {
        return Err(format!(
            "source_snapshot_unavailable: concurrent replacement: {}",
            path.display()
        ));
    }
    let capacity = usize::try_from(before_length.min(64 * 1024))
        .map_err(|_| format!("source_snapshot_byte_limit: {}", path.display()))?;
    let mut bytes = Vec::with_capacity(capacity);
    let read_limit = before_length
        .checked_add(1)
        .ok_or_else(|| format!("source_snapshot_byte_limit: {}", path.display()))?;
    contained
        .file_mut()
        .take(read_limit)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("material_file_unreadable: {}: {error}", path.display()))?;
    if bytes.len() as u64 > before_length {
        return Err(format!(
            "source_snapshot_unavailable: material grew during bounded read: {}",
            path.display()
        ));
    }
    let after_handle = observe_open_file(contained.file(), path)?;
    contained.validate_after_read()?;
    #[cfg(unix)]
    let baseline = before;
    #[cfg(windows)]
    let baseline = opened;
    #[cfg(unix)]
    let after_path = observe_path_after_read(path)?;
    #[cfg(windows)]
    let after_path = {
        let reopened = open_no_follow(workspace, path).map_err(|detail| {
            classify_reopen_failure_after_observation(workspace, path, baseline.length, detail)
        })?;
        let observation = observe_open_file(reopened.file(), path)?;
        reopened.validate_after_read()?;
        observation
    };
    if baseline != after_handle || baseline != after_path || bytes.len() as u64 != before_length {
        return Err(format!(
            "source_snapshot_unavailable: concurrent mutation: {}",
            path.display()
        ));
    }
    Ok(StableRead {
        bytes,
        observation: baseline,
    })
}

fn classify_open_failure_after_observation(
    workspace: &Path,
    path: &Path,
    before_metadata: &std::fs::Metadata,
    #[cfg(unix)] before: &FileObservation,
    detail: String,
) -> String {
    if let Err(ancestor_detail) = reject_existing_ancestor_links(workspace, path) {
        if ancestor_detail.starts_with("symlink_or_reparse_escape:") {
            return format!(
                "source_snapshot_unavailable: contained source topology changed before open: {}",
                path.display()
            );
        }
        return detail;
    }
    match std::fs::symlink_metadata(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => format!(
            "source_snapshot_unavailable: material disappeared before open: {}",
            path.display()
        ),
        Ok(metadata)
            if metadata_is_link_or_reparse_point(&metadata)
                || !metadata.is_file()
                || metadata.len() != before_metadata.len() =>
        {
            format!(
                "source_snapshot_unavailable: material type or length changed before open: {}",
                path.display()
            )
        }
        #[cfg(unix)]
        Ok(metadata) => match observe_regular_file(&metadata, path) {
            Ok(after) if &after != before => format!(
                "source_snapshot_unavailable: material identity changed before open: {}",
                path.display()
            ),
            Ok(_) | Err(_) => detail,
        },
        #[cfg(not(unix))]
        Ok(_) => detail,
        Err(_) => detail,
    }
}

#[cfg(any(windows, test))]
fn classify_reopen_failure_after_observation(
    workspace: &Path,
    path: &Path,
    expected_length: u64,
    detail: String,
) -> String {
    if let Err(ancestor_detail) = reject_existing_ancestor_links(workspace, path) {
        if ancestor_detail.starts_with("symlink_or_reparse_escape:") {
            return format!(
                "source_snapshot_unavailable: contained source topology changed after open: {}",
                path.display()
            );
        }
        return detail;
    }
    match std::fs::symlink_metadata(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => format!(
            "source_snapshot_unavailable: material disappeared after open: {}",
            path.display()
        ),
        Ok(metadata)
            if metadata_is_link_or_reparse_point(&metadata)
                || !metadata.is_file()
                || metadata.len() != expected_length =>
        {
            format!(
                "source_snapshot_unavailable: material type or length changed after open: {}",
                path.display()
            )
        }
        Ok(_) | Err(_) => detail,
    }
}

#[cfg(unix)]
fn observe_path_after_read(path: &Path) -> Result<FileObservation, String> {
    let metadata = match std::fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Err(format!(
                "source_snapshot_unavailable: material disappeared after open: {}",
                path.display()
            ));
        }
        Err(error) => {
            return Err(format!(
                "material_file_unavailable: {}: {error}",
                path.display()
            ));
        }
    };
    if metadata_is_link_or_reparse_point(&metadata) || !metadata.is_file() {
        return Err(format!(
            "source_snapshot_unavailable: material type changed after open: {}",
            path.display()
        ));
    }
    observe_regular_file(&metadata, path)
}

fn digest_bytes(bytes: &[u8]) -> String {
    format!("sha256:{:x}", Sha256::digest(bytes))
}

fn path_belongs_to_source(root: &str, path: &str) -> bool {
    root == "."
        || path
            .strip_prefix(root)
            .is_some_and(|rest| rest.starts_with('/'))
}

#[derive(Debug, Clone)]
struct CaptureBudget {
    limits: SnapshotLimits,
    started: Duration,
    files: usize,
    bytes: u64,
    traversal_entries: usize,
}

impl CaptureBudget {
    fn new(limits: SnapshotLimits, started: Duration) -> Self {
        Self {
            limits,
            started,
            files: 0,
            bytes: 0,
            traversal_entries: 0,
        }
    }

    fn check_deadline(&self, clock: &dyn SnapshotClock) -> Result<(), String> {
        let elapsed = clock
            .now()
            .checked_sub(self.started)
            .ok_or_else(|| "source_snapshot_clock_invalid: clock moved backwards".to_string())?;
        if elapsed > self.limits.max_elapsed {
            return Err("source_snapshot_deadline: authoritative snapshot discarded".into());
        }
        Ok(())
    }

    fn register_files(&mut self, count: usize) -> Result<(), String> {
        self.files = self
            .files
            .checked_add(count)
            .ok_or_else(|| "source_snapshot_file_limit: overflow".to_string())?;
        if self.files > self.limits.max_files {
            return Err("source_snapshot_file_limit: authoritative snapshot discarded".into());
        }
        Ok(())
    }

    fn register_bytes(&mut self, count: u64) -> Result<(), String> {
        self.bytes = self
            .bytes
            .checked_add(count)
            .ok_or_else(|| "source_snapshot_byte_limit: overflow".to_string())?;
        if self.bytes > self.limits.max_bytes {
            return Err("source_snapshot_byte_limit: authoritative snapshot discarded".into());
        }
        Ok(())
    }

    fn remaining_bytes(&self) -> Result<u64, String> {
        self.limits
            .max_bytes
            .checked_sub(self.bytes)
            .ok_or_else(|| "source_snapshot_byte_limit: aggregate overflow".to_string())
    }

    fn register_traversal_entry(&mut self) -> Result<(), String> {
        self.traversal_entries = self
            .traversal_entries
            .checked_add(1)
            .ok_or_else(|| "source_snapshot_traversal_limit: overflow".to_string())?;
        if self.traversal_entries > self.limits.max_traversal_entries {
            return Err("source_snapshot_traversal_limit: authoritative snapshot discarded".into());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn content_change_with_unchanged_len_and_mtime_changes_fingerprint() {
        let fixture = Fixture::new("snapshot-content");
        let (resolver, service) = fixture.services();
        let source = resolver.analysis;
        let before = service.capture_authoritative(&source, &[], 1).unwrap();
        let module = fixture.root.join("main/CommonModules/X/Ext/Module.bsl");
        #[cfg(unix)]
        let timestamps = unix_timestamps(&module);
        fs::write(&module, "BBBB").unwrap();
        #[cfg(unix)]
        restore_unix_timestamps(&module, timestamps);
        let after = service.capture_authoritative(&source, &[], 1).unwrap();
        assert_ne!(
            before.analysis.source_fingerprint,
            after.analysis.source_fingerprint
        );
    }

    #[test]
    fn platform_manifest_is_registration_aware_deterministic_and_excludes_generated_corpora() {
        let fixture = Fixture::new("snapshot-selection");
        write(
            &fixture.root.join("main/Configuration.xml"),
            "<MetaDataObject><Configuration><ChildObjects><CommonModule>X</CommonModule><Role>Admin</Role><Document>Sale</Document></ChildObjects></Configuration></MetaDataObject>",
        );
        write(
            &fixture.root.join("main/Roles/Admin.xml"),
            "<MetaDataObject><Role><Properties><Name>Admin</Name></Properties><ChildObjects/></Role></MetaDataObject>",
        );
        write(
            &fixture.root.join("main/Roles/Admin/Ext/Rights.xml"),
            "rights",
        );
        write(
            &fixture.root.join("main/Documents/Sale.xml"),
            "<MetaDataObject><Document><Properties><Name>Sale</Name></Properties><ChildObjects><Command>Post</Command></ChildObjects></Document></MetaDataObject>",
        );
        write(
            &fixture.root.join("main/Documents/Sale/Commands/Post.xml"),
            "registered command",
        );
        write(
            &fixture
                .root
                .join("main/Documents/Sale/Commands/Post/Ext/Module.bsl"),
            "command",
        );
        write(
            &fixture.root.join("main/Documents/Sale/Commands/Decoy.xml"),
            "decoy command",
        );
        write(
            &fixture
                .root
                .join("main/Documents/Sale/Commands/Decoy/Ext/Module.bsl"),
            "decoy",
        );
        write(&fixture.root.join("main/CommonModules/Decoy.xml"), "decoy");
        write(
            &fixture.root.join("main/docs/research/secret.bsl"),
            "secret",
        );
        write(
            &fixture
                .root
                .join("main/CommonModules/X/Ext/target/generated.bin"),
            "generated",
        );
        write(&fixture.root.join("main/Ext/Decoy.bsl"), "decoy");
        write(&fixture.root.join("main/Ext/SessionModule.bsl"), "session");
        let (selection, service) = fixture.services();
        let first = service
            .capture_authoritative(&selection.analysis, &[], 1)
            .unwrap();
        let paths = first
            .analysis
            .manifest
            .entries()
            .keys()
            .cloned()
            .collect::<Vec<_>>();
        assert_eq!(paths, {
            let mut sorted = paths.clone();
            sorted.sort();
            sorted
        });
        assert!(paths.iter().any(|path| path.ends_with("Configuration.xml")));
        assert!(paths
            .iter()
            .any(|path| path.ends_with("CommonModules/X.xml")));
        assert!(paths
            .iter()
            .any(|path| path.ends_with("CommonModules/X/Ext/Module.bsl")));
        assert!(paths
            .iter()
            .any(|path| path.ends_with("Ext/SessionModule.bsl")));
        assert!(paths
            .iter()
            .any(|path| path.ends_with("Roles/Admin/Ext/Rights.xml")));
        assert!(paths
            .iter()
            .any(|path| path.ends_with("Commands/Post/Ext/Module.bsl")));
        assert!(!paths.iter().any(|path| path.contains("Decoy")
            || path.contains("docs/research")
            || path.contains("target")));
        write(
            &fixture.root.join("main/CommonModules/Decoy.xml"),
            "changed",
        );
        let second = service
            .capture_authoritative(&selection.analysis, &[], 1)
            .unwrap();
        assert_eq!(
            first.analysis.source_fingerprint,
            second.analysis.source_fingerprint
        );
    }

    #[test]
    fn composite_capture_sorts_and_deduplicates_destinations_and_binds_each_role() {
        let fixture = Fixture::new("snapshot-composite");
        fixture.add_extension("ExtensionA", "ext-a", "A");
        fixture.add_extension("ExtensionB", "ext-b", "B");
        let selection = resolve_source_selection(
            &fixture.root,
            Some("main"),
            &[
                "ExtensionB".into(),
                "ExtensionA".into(),
                "ExtensionB".into(),
            ],
        )
        .unwrap();
        let service = FilesystemSourceSnapshots::new(&fixture.root).unwrap();
        let snapshot = service
            .capture_authoritative(&selection.analysis, &selection.mutations, 7)
            .unwrap();
        assert_eq!(snapshot.mutations.len(), 2);
        assert_eq!(snapshot.mutations[0].source_set.name, "ExtensionA");
        let only_a =
            resolve_source_selection(&fixture.root, Some("main"), &["ExtensionA".into()]).unwrap();
        let a = service
            .capture_authoritative(&only_a.analysis, &only_a.mutations, 7)
            .unwrap();
        assert_ne!(snapshot.composite_fingerprint, a.composite_fingerprint);
    }

    #[test]
    fn optional_parent_configurations_absence_and_presence_are_snapshot_bound() {
        let fixture = Fixture::new("snapshot-parent-config");
        let (selection, service) = fixture.services();
        let absent = service
            .capture_authoritative(&selection.analysis, &[], 1)
            .unwrap();
        let path = "main/Ext/ParentConfigurations.bin";
        assert_eq!(
            service
                .read_optional_verified(&absent.analysis, path)
                .unwrap(),
            None
        );
        write(&fixture.root.join(path), "parent");
        let mismatch = service
            .read_optional_verified(&absent.analysis, path)
            .unwrap_err();
        assert_eq!(mismatch.reason_code(), "source_fingerprint_mismatch");
        assert!(mismatch.retryable());
        let present = service
            .capture_authoritative(&selection.analysis, &[], 1)
            .unwrap();
        assert_eq!(
            service
                .read_optional_verified(&present.analysis, path)
                .unwrap(),
            Some(b"parent".to_vec())
        );
        assert_ne!(
            absent.analysis.source_fingerprint,
            present.analysis.source_fingerprint
        );
    }

    #[test]
    fn authoritative_root_interfaces_change_source_fingerprint() {
        for artifact in ["ClientApplicationInterface.xml", "HomePageWorkArea.xml"] {
            let fixture = Fixture::new(&format!("snapshot-root-{}", artifact.replace('.', "-")));
            let path = fixture.root.join("main/Ext").join(artifact);
            write(&path, "AAAA");
            let (selection, service) = fixture.services();
            let before = service
                .capture_authoritative(&selection.analysis, &[], 1)
                .unwrap();
            write(&path, "BBBB");
            let after = service
                .capture_authoritative(&selection.analysis, &[], 1)
                .unwrap();
            assert_ne!(
                before.analysis.source_fingerprint, after.analysis.source_fingerprint,
                "{artifact}"
            );
        }
    }

    #[test]
    fn verified_read_checks_manifest_membership_then_detects_byte_mismatch() {
        let fixture = Fixture::new("snapshot-verified-read");
        let (selection, service) = fixture.services();
        let snapshot = service
            .capture_authoritative(&selection.analysis, &[], 1)
            .unwrap();
        let outside = service
            .read_verified(&snapshot.analysis, "../outside")
            .unwrap_err();
        assert_eq!(outside.reason_code(), "source_path_not_in_manifest");
        assert!(!outside.retryable());
        let path = "main/CommonModules/X/Ext/Module.bsl";
        fs::write(fixture.root.join(path), "BBBB").unwrap();
        let mismatch = service.read_verified(&snapshot.analysis, path).unwrap_err();
        assert_eq!(mismatch.reason_code(), "source_fingerprint_mismatch");
        assert!(mismatch.retryable());
    }

    #[test]
    fn verified_read_rejects_oversized_replacement_as_fingerprint_mismatch() {
        let fixture = Fixture::new("snapshot-verified-read-bounded");
        let (selection, service) = fixture.services();
        let snapshot = service
            .capture_authoritative(&selection.analysis, &[], 1)
            .unwrap();
        let path = "main/CommonModules/X/Ext/Module.bsl";
        let expected_length = match snapshot.analysis.manifest.get(path).unwrap() {
            ManifestEntry::Present(file) => file.byte_length,
            ManifestEntry::AbsentOptional(_) => unreachable!(),
        };
        fs::write(
            fixture.root.join(path),
            vec![b'X'; usize::try_from(expected_length + 1).unwrap()],
        )
        .unwrap();
        let bounded = fixture.controlled_service(
            SnapshotLimits {
                max_bytes: expected_length,
                ..SnapshotLimits::default()
            },
            Arc::new(FixedClock),
            Arc::new(NoopCaptureHook),
        );

        let mismatch = bounded.read_verified(&snapshot.analysis, path).unwrap_err();
        assert_eq!(mismatch.reason_code(), "source_fingerprint_mismatch");
    }

    #[cfg(unix)]
    #[test]
    fn absent_optional_read_rejects_symlinked_ancestor() {
        use std::os::unix::fs::symlink;

        let fixture = Fixture::new("snapshot-absent-symlink-ancestor");
        let (selection, service) = fixture.services();
        let snapshot = service
            .capture_authoritative(&selection.analysis, &[], 1)
            .unwrap();
        let optional = "main/Ext/ParentConfigurations.bin";
        assert!(matches!(
            snapshot.analysis.manifest.get(optional),
            Some(ManifestEntry::AbsentOptional(_))
        ));
        let ext = fixture.root.join("main/Ext");
        let external = fixture.root.join("external-ext");
        fs::create_dir_all(&external).unwrap();
        symlink(&external, &ext).unwrap();

        let error = service
            .read_optional_verified(&snapshot.analysis, optional)
            .unwrap_err();
        assert_eq!(error.reason_code(), "source_fingerprint_mismatch");
    }

    #[test]
    fn file_and_byte_limits_accept_boundary_and_reject_boundary_plus_one_globally() {
        let fixture = Fixture::new("snapshot-bounds");
        fixture.add_extension("ExtensionA", "ext-a", "A");
        let selection =
            resolve_source_selection(&fixture.root, Some("main"), &["ExtensionA".into()]).unwrap();
        let baseline = FilesystemSourceSnapshots::new(&fixture.root).unwrap();
        let captured = baseline
            .capture_authoritative(&selection.analysis, &selection.mutations, 1)
            .unwrap();
        let files = captured
            .analysis
            .manifest
            .entries()
            .values()
            .filter(|entry| matches!(entry, ManifestEntry::Present(_)))
            .count()
            + captured
                .mutations
                .iter()
                .flat_map(|snapshot| snapshot.manifest.entries().values())
                .filter(|entry| matches!(entry, ManifestEntry::Present(_)))
                .count();
        let bytes = captured
            .analysis
            .manifest
            .entries()
            .values()
            .chain(
                captured
                    .mutations
                    .iter()
                    .flat_map(|snapshot| snapshot.manifest.entries().values()),
            )
            .filter_map(|entry| match entry {
                ManifestEntry::Present(file) => Some(file.byte_length),
                ManifestEntry::AbsentOptional(_) => None,
            })
            .sum();
        let exact = fixture.controlled_service(
            SnapshotLimits {
                max_files: files,
                max_bytes: bytes,
                max_elapsed: Duration::from_secs(60),
                ..SnapshotLimits::default()
            },
            Arc::new(FixedClock),
            Arc::new(NoopCaptureHook),
        );
        assert!(exact
            .capture_authoritative(&selection.analysis, &selection.mutations, 1)
            .is_ok());
        let file_short = fixture.controlled_service(
            SnapshotLimits {
                max_files: files - 1,
                max_bytes: bytes,
                max_elapsed: Duration::from_secs(60),
                ..SnapshotLimits::default()
            },
            Arc::new(FixedClock),
            Arc::new(NoopCaptureHook),
        );
        let error = file_short
            .capture_authoritative(&selection.analysis, &selection.mutations, 1)
            .unwrap_err();
        assert_eq!(error.reason, SnapshotCaptureReason::SnapshotResourceLimit);
        assert!(!error.retryable());
        let byte_short = fixture.controlled_service(
            SnapshotLimits {
                max_files: files,
                max_bytes: bytes - 1,
                max_elapsed: Duration::from_secs(60),
                ..SnapshotLimits::default()
            },
            Arc::new(FixedClock),
            Arc::new(NoopCaptureHook),
        );
        let error = byte_short
            .capture_authoritative(&selection.analysis, &selection.mutations, 1)
            .unwrap_err();
        assert_eq!(error.reason, SnapshotCaptureReason::SnapshotResourceLimit);
        assert!(!error.retryable());
    }

    #[test]
    fn composite_budget_counts_overlapping_present_paths_once() {
        let root = temp_root("snapshot-overlapping-composite-budget");
        write(
            &root.join("v8project.yaml"),
            "source-set:\n - { name: main, type: CONFIGURATION, path: base }\n - { name: extension, type: EXTENSION, path: base/CommonModules/X }\n",
        );
        write(
            &root.join("base/Configuration.xml"),
            "<MetaDataObject><Configuration><ChildObjects><CommonModule>X</CommonModule></ChildObjects></Configuration></MetaDataObject>",
        );
        write(
            &root.join("base/CommonModules/X.xml"),
            "<MetaDataObject><CommonModule><Properties><Name>X</Name></Properties><ChildObjects/></CommonModule></MetaDataObject>",
        );
        write(
            &root.join("base/CommonModules/X/Configuration.xml"),
            "<MetaDataObject><Configuration><ChildObjects/></Configuration></MetaDataObject>",
        );
        write(&root.join("base/CommonModules/X/Ext/Module.bsl"), "shared");
        let selection =
            resolve_source_selection(&root, Some("main"), &["extension".into()]).unwrap();
        let baseline = FilesystemSourceSnapshots::new(&root)
            .unwrap()
            .capture_authoritative(&selection.analysis, &selection.mutations, 1)
            .unwrap();
        let mut unique = BTreeMap::new();
        for snapshot in std::iter::once(&baseline.analysis).chain(baseline.mutations.iter()) {
            for (path, entry) in snapshot.manifest.entries() {
                if let ManifestEntry::Present(material) = entry {
                    unique.insert(path.clone(), material.byte_length);
                }
            }
        }
        let unique_bytes = unique.values().sum();
        let exact = FilesystemSourceSnapshots {
            workspace: canonical_workspace(&root).unwrap(),
            limits: SnapshotLimits {
                max_files: unique.len(),
                max_bytes: unique_bytes,
                ..SnapshotLimits::default()
            },
            clock: Arc::new(FixedClock),
            hook: Arc::new(NoopCaptureHook),
        };
        assert!(exact
            .capture_authoritative(&selection.analysis, &selection.mutations, 1)
            .is_ok());

        let short = FilesystemSourceSnapshots {
            limits: SnapshotLimits {
                max_files: unique.len() - 1,
                max_bytes: unique_bytes,
                ..SnapshotLimits::default()
            },
            ..exact
        };
        let error = short
            .capture_authoritative(&selection.analysis, &selection.mutations, 1)
            .unwrap_err();
        assert_eq!(error.reason, SnapshotCaptureReason::SnapshotResourceLimit);
        assert!(!error.retryable());
    }

    #[test]
    fn deadline_uses_injected_clock_and_discards_whole_snapshot() {
        let fixture = Fixture::new("snapshot-deadline");
        let selection = resolve_source_selection(&fixture.root, Some("main"), &[]).unwrap();
        let clock = Arc::new(AdvancingClock::default());
        let service = fixture.controlled_service(
            SnapshotLimits {
                max_files: 100,
                max_bytes: 1024 * 1024,
                max_elapsed: Duration::from_millis(2),
                ..SnapshotLimits::default()
            },
            clock,
            Arc::new(NoopCaptureHook),
        );
        let error = service
            .capture_authoritative(&selection.analysis, &[], 1)
            .unwrap_err();
        assert_eq!(
            error.reason,
            SnapshotCaptureReason::SnapshotDeadlineExceeded
        );
        assert!(error.retryable());
    }

    #[test]
    fn xml_and_traversal_bounds_accept_exact_boundary_and_reject_boundary_plus_one() {
        let fixture = Fixture::new("snapshot-structural-bounds");
        let selection = resolve_source_selection(&fixture.root, Some("main"), &[]).unwrap();
        let xml_max = [
            fixture.root.join("main/Configuration.xml"),
            fixture.root.join("main/CommonModules/X.xml"),
        ]
        .iter()
        .map(|path| fs::metadata(path).unwrap().len())
        .max()
        .unwrap();
        let exact_xml = fixture.controlled_service(
            SnapshotLimits {
                max_xml_bytes: xml_max,
                ..SnapshotLimits::default()
            },
            Arc::new(FixedClock),
            Arc::new(NoopCaptureHook),
        );
        assert!(exact_xml
            .capture_authoritative(&selection.analysis, &[], 1)
            .is_ok());
        let short_xml = fixture.controlled_service(
            SnapshotLimits {
                max_xml_bytes: xml_max - 1,
                ..SnapshotLimits::default()
            },
            Arc::new(FixedClock),
            Arc::new(NoopCaptureHook),
        );
        let error = short_xml
            .capture_authoritative(&selection.analysis, &[], 1)
            .unwrap_err();
        assert_eq!(error.reason, SnapshotCaptureReason::SnapshotResourceLimit);
        assert!(!error.retryable());

        let clock = FixedClock;
        let canonical_root = canonical_workspace(&fixture.root).unwrap();
        let mut measured = CaptureBudget::new(SnapshotLimits::default(), Duration::ZERO);
        scan_source_plan(&canonical_root, &selection.analysis, &mut measured, &clock).unwrap();
        scan_source_plan(&canonical_root, &selection.analysis, &mut measured, &clock).unwrap();
        let exact_count = measured.traversal_entries;
        let exact_traversal = fixture.controlled_service(
            SnapshotLimits {
                max_traversal_entries: exact_count,
                ..SnapshotLimits::default()
            },
            Arc::new(FixedClock),
            Arc::new(NoopCaptureHook),
        );
        assert!(exact_traversal
            .capture_authoritative(&selection.analysis, &[], 1)
            .is_ok());
        let short_traversal = fixture.controlled_service(
            SnapshotLimits {
                max_traversal_entries: exact_count - 1,
                ..SnapshotLimits::default()
            },
            Arc::new(FixedClock),
            Arc::new(NoopCaptureHook),
        );
        let error = short_traversal
            .capture_authoritative(&selection.analysis, &[], 1)
            .unwrap_err();
        assert_eq!(error.reason, SnapshotCaptureReason::SnapshotResourceLimit);
        assert!(!error.retryable());
    }

    #[test]
    fn concurrent_add_remove_write_and_replace_fail_the_whole_snapshot() {
        let actions = [
            RaceAction::Add,
            RaceAction::Remove,
            RaceAction::Write,
            RaceAction::Replace,
            #[cfg(unix)]
            RaceAction::ParentSymlinkSwap,
        ];
        for action in actions {
            assert_retryable_race(action);
        }
    }

    #[cfg(unix)]
    #[test]
    fn fifo_replacement_between_observation_and_open_is_nonblocking_and_retryable() {
        assert_retryable_race(RaceAction::FifoSwapBeforeOpen);
    }

    #[cfg(unix)]
    #[test]
    fn parent_symlink_swap_between_observation_and_open_is_retryable() {
        assert_retryable_race(RaceAction::ParentSymlinkSwapBeforeOpen);
    }

    #[test]
    fn same_length_replacement_after_contained_open_is_retryable() {
        assert_retryable_race(RaceAction::ReplaceAfterOpen);
    }

    #[test]
    fn growth_after_contained_open_is_bounded_and_retryable() {
        assert_retryable_race(RaceAction::GrowAfterOpen);
    }

    #[test]
    fn absent_optional_appearance_after_final_scan_discards_snapshot() {
        let fixture = Fixture::new("snapshot-optional-appears-before-final-validation");
        let selection = resolve_source_selection(&fixture.root, Some("main"), &[]).unwrap();
        let service = fixture.controlled_service(
            SnapshotLimits::default(),
            Arc::new(FixedClock),
            Arc::new(FinalValidationMutationHook::new(
                fixture.root.clone(),
                FinalValidationMutation::AddParentConfigurations,
            )),
        );

        let error = service
            .capture_authoritative(&selection.analysis, &[], 1)
            .unwrap_err();
        assert_eq!(
            error.reason,
            SnapshotCaptureReason::SourceChangedDuringCapture
        );
        assert!(error.retryable());
    }

    #[test]
    fn edt_absent_marker_appearance_after_final_scan_discards_snapshot() {
        let root = temp_root("snapshot-edt-marker-appears-before-final-validation");
        write(
            &root.join("v8project.yaml"),
            "format: EDT\nsource-set:\n - { name: main, type: CONFIGURATION, path: edt }\n",
        );
        write(&root.join("edt/.project"), "project");
        let selection = resolve_source_selection(&root, Some("main"), &[]).unwrap();
        let service = FilesystemSourceSnapshots {
            workspace: canonical_workspace(&root).unwrap(),
            limits: SnapshotLimits::default(),
            clock: Arc::new(FixedClock),
            hook: Arc::new(FinalValidationMutationHook::new(
                root,
                FinalValidationMutation::AddEdtProjectPmf,
            )),
        };

        let error = service
            .capture_authoritative(&selection.analysis, &[], 1)
            .unwrap_err();
        assert_eq!(
            error.reason,
            SnapshotCaptureReason::SourceChangedDuringCapture
        );
        assert!(error.retryable());
    }

    #[test]
    fn final_present_reread_is_bounded_by_captured_length() {
        let fixture = Fixture::new("snapshot-final-reread-bounded");
        let selection = resolve_source_selection(&fixture.root, Some("main"), &[]).unwrap();
        let baseline = FilesystemSourceSnapshots::new(&fixture.root)
            .unwrap()
            .capture_authoritative(&selection.analysis, &[], 1)
            .unwrap();
        let captured_bytes = baseline
            .analysis
            .manifest
            .entries()
            .values()
            .filter_map(|entry| match entry {
                ManifestEntry::Present(material) => Some(material.byte_length),
                ManifestEntry::AbsentOptional(_) => None,
            })
            .sum();
        let service = fixture.controlled_service(
            SnapshotLimits {
                max_bytes: captured_bytes,
                ..SnapshotLimits::default()
            },
            Arc::new(FixedClock),
            Arc::new(FinalValidationMutationHook::new(
                fixture.root.clone(),
                FinalValidationMutation::GrowPresent(captured_bytes + 1),
            )),
        );

        let error = service
            .capture_authoritative(&selection.analysis, &[], 1)
            .unwrap_err();
        assert_eq!(
            error.reason,
            SnapshotCaptureReason::SourceChangedDuringCapture
        );
        assert!(error.retryable());
    }

    #[test]
    fn malformed_registered_descriptor_is_stable_and_non_retryable() {
        let fixture = Fixture::new("snapshot-malformed-descriptor");
        write(
            &fixture.root.join("main/CommonModules/X.xml"),
            "<MetaDataObject><CommonModule>",
        );
        let selection = resolve_source_selection(&fixture.root, Some("main"), &[]).unwrap();
        let service = FilesystemSourceSnapshots::new(&fixture.root).unwrap();

        let error = service
            .capture_authoritative(&selection.analysis, &[], 1)
            .unwrap_err();
        assert_eq!(error.reason, SnapshotCaptureReason::MalformedSourceMaterial);
        assert!(!error.retryable());
    }

    #[test]
    fn final_scan_and_identity_failures_preserve_their_classification() {
        let io = classify_final_scan_error(
            "material_subtree_unreadable: stable permission failure".into(),
        );
        assert_eq!(io.reason, SnapshotCaptureReason::TransientSourceIo);
        assert!(io.retryable());

        let identity = classify_final_scan_error(
            "file_identity_unavailable: stable platform identity failure".into(),
        );
        assert_eq!(identity.reason, SnapshotCaptureReason::UnsafeSourceTopology);
        assert!(!identity.retryable());
    }

    #[test]
    fn post_open_reopen_failure_promotes_only_observed_change() {
        let root = temp_root("snapshot-reopen-classifier");
        let missing = root.join("missing.bsl");
        let changed = classify_reopen_failure_after_observation(
            &root,
            &missing,
            4,
            "material_file_unreadable: reopen failed".into(),
        );
        assert!(changed.starts_with("source_snapshot_unavailable:"));

        let stable = root.join("stable.bsl");
        write(&stable, "same");
        let io = classify_reopen_failure_after_observation(
            &root,
            &stable,
            4,
            "material_file_unreadable: stable share failure".into(),
        );
        assert_eq!(io, "material_file_unreadable: stable share failure");
    }

    #[test]
    fn mapping_revalidation_promotes_structural_changes_but_preserves_io() {
        let io = classify_mapping_revalidation_error(
            "source_map_config_unavailable: stable share failure".into(),
        );
        assert_eq!(io.reason, SnapshotCaptureReason::TransientSourceIo);
        assert!(io.retryable());

        let actions = [
            MappingMutation::RenameSource,
            MappingMutation::DeleteMap,
            MappingMutation::MalformedMap,
            #[cfg(unix)]
            MappingMutation::SymlinkMap,
        ];
        for action in actions {
            let fixture = Fixture::new(&format!("snapshot-mapping-race-{action:?}"));
            let selection = resolve_source_selection(&fixture.root, Some("main"), &[]).unwrap();
            let service = fixture.controlled_service(
                SnapshotLimits::default(),
                Arc::new(FixedClock),
                Arc::new(MappingMutationHook::new(fixture.root.clone(), action)),
            );

            let error = service
                .capture_authoritative(&selection.analysis, &[], 1)
                .unwrap_err();
            assert_eq!(
                error.reason,
                SnapshotCaptureReason::SourceChangedDuringCapture,
                "{action:?}: {error:?}"
            );
            assert!(error.retryable());
        }
    }

    #[test]
    fn missing_registered_descriptor_is_stable_and_non_retryable() {
        let fixture = Fixture::new("snapshot-missing-descriptor");
        fs::remove_file(fixture.root.join("main/CommonModules/X.xml")).unwrap();
        let selection = resolve_source_selection(&fixture.root, Some("main"), &[]).unwrap();
        let service = FilesystemSourceSnapshots::new(&fixture.root).unwrap();

        let error = service
            .capture_authoritative(&selection.analysis, &[], 1)
            .unwrap_err();
        assert_eq!(error.reason, SnapshotCaptureReason::MalformedSourceMaterial);
        assert!(!error.retryable());
    }

    #[cfg(unix)]
    #[test]
    fn non_utf8_material_path_is_rejected_non_retryably() {
        use std::os::unix::ffi::OsStringExt;

        let fixture = Fixture::new("snapshot-non-utf8-material");
        let path = fixture
            .root
            .join("main/CommonModules/X/Ext")
            .join(std::ffi::OsString::from_vec(vec![b'x', 0xff]));
        let detail = slash_relative(&fixture.root, &path).unwrap_err();
        let error = SnapshotCaptureError::classify(detail);
        assert_eq!(error.reason, SnapshotCaptureReason::InvalidSourcePath);
        assert!(!error.retryable());
    }

    #[cfg(unix)]
    #[test]
    fn symlink_special_and_unreadable_material_fail_closed() {
        use std::os::unix::fs::{symlink, PermissionsExt};
        let cases = ["symlink", "special", "unreadable"];
        for case in cases {
            let fixture = Fixture::new(&format!("snapshot-material-{case}"));
            let ext = fixture.root.join("main/CommonModules/X/Ext");
            let module = ext.join("Module.bsl");
            match case {
                "symlink" => {
                    fs::remove_file(&module).unwrap();
                    symlink("../outside.bsl", &module).unwrap();
                }
                "special" => {
                    fs::remove_file(&module).unwrap();
                    let c = std::ffi::CString::new(module.as_os_str().as_encoded_bytes()).unwrap();
                    assert_eq!(unsafe { libc::mkfifo(c.as_ptr(), 0o600) }, 0);
                }
                "unreadable" => {
                    fs::set_permissions(module, fs::Permissions::from_mode(0o0)).unwrap();
                }
                _ => unreachable!(),
            }
            let selection = resolve_source_selection(&fixture.root, Some("main"), &[]).unwrap();
            let service = FilesystemSourceSnapshots::new(&fixture.root).unwrap();
            let error = service
                .capture_authoritative(&selection.analysis, &[], 1)
                .unwrap_err();
            if matches!(case, "symlink" | "special") {
                assert_eq!(error.reason, SnapshotCaptureReason::UnsafeSourceTopology);
                assert!(!error.retryable());
            } else if case == "unreadable" {
                assert_eq!(error.reason, SnapshotCaptureReason::TransientSourceIo);
                assert!(error.retryable());
            }
        }
    }

    #[cfg(unix)]
    #[test]
    fn edt_absent_marker_rejects_symlinked_ancestor() {
        use std::os::unix::fs::symlink;

        let root = temp_root("snapshot-edt-absent-symlink-ancestor");
        write(
            &root.join("v8project.yaml"),
            "format: EDT\nsource-set:\n - { name: main, type: CONFIGURATION, path: edt }\n",
        );
        write(&root.join("edt/.project"), "project");
        let external = root.join("external-dt-inf");
        fs::create_dir_all(&external).unwrap();
        symlink(&external, root.join("edt/DT-INF")).unwrap();
        let selection = resolve_source_selection(&root, Some("main"), &[]).unwrap();
        let service = FilesystemSourceSnapshots::new(&root).unwrap();

        let error = service
            .capture_authoritative(&selection.analysis, &[], 1)
            .unwrap_err();
        assert_eq!(error.reason, SnapshotCaptureReason::UnsafeSourceTopology);
        assert!(!error.retryable());
    }

    #[cfg(unix)]
    #[test]
    fn registered_subtree_error_precedence_is_name_ordered() {
        use std::os::unix::fs::symlink;

        let mut reasons = Vec::new();
        for (index, names) in [["z-valid", "a-unsafe"], ["a-unsafe", "z-valid"]]
            .into_iter()
            .enumerate()
        {
            let root = temp_root(&format!("snapshot-subtree-order-{index}"));
            let subtree = root.join("subtree");
            fs::create_dir_all(&subtree).unwrap();
            for name in names {
                if name == "a-unsafe" {
                    symlink(&root, subtree.join(name)).unwrap();
                } else {
                    write(&subtree.join(name), "x");
                }
            }
            let mut present = BTreeSet::new();
            let mut budget = CaptureBudget::new(
                SnapshotLimits {
                    max_traversal_entries: 1,
                    ..SnapshotLimits::default()
                },
                Duration::ZERO,
            );
            let error =
                collect_registered_subtree(&root, &subtree, &mut present, &mut budget, &FixedClock)
                    .unwrap_err();
            reasons.push(SnapshotCaptureError::classify(error).reason);
            fs::remove_dir_all(root).unwrap();
        }
        assert_eq!(
            reasons,
            vec![
                SnapshotCaptureReason::UnsafeSourceTopology,
                SnapshotCaptureReason::UnsafeSourceTopology,
            ]
        );
    }

    #[test]
    fn recognized_edt_configuration_gets_marker_only_diagnostic_snapshot() {
        let root = temp_root("snapshot-edt");
        write(
            &root.join("v8project.yaml"),
            "format: EDT\nsource-set:\n - { name: main, type: CONFIGURATION, path: edt }\n",
        );
        write(&root.join("edt/.project"), "project");
        write(
            &root.join("edt/Configuration/Configuration.mdo"),
            "configuration",
        );
        write(&root.join("edt/src/unrelated.bsl"), "ignored");
        let selection = resolve_source_selection(&root, Some("main"), &[]).unwrap();
        assert_eq!(selection.analysis.source_format, SourceFormat::Edt);
        let service = FilesystemSourceSnapshots::new(&root).unwrap();
        let snapshot = service
            .capture_authoritative(&selection.analysis, &[], 1)
            .unwrap();
        assert_eq!(snapshot.analysis.manifest.entries().len(), 4);
        assert_eq!(
            snapshot
                .analysis
                .manifest
                .entries()
                .values()
                .filter(|entry| matches!(entry, ManifestEntry::Present(_)))
                .count(),
            2
        );
        assert!(!snapshot
            .analysis
            .manifest
            .entries()
            .keys()
            .any(|path| path.contains("unrelated")));
    }

    struct Fixture {
        root: PathBuf,
    }

    impl Fixture {
        fn new(name: &str) -> Self {
            let root = temp_root(name);
            write(&root.join("v8project.yaml"), "format: DESIGNER\nsource-set:\n - { name: main, type: CONFIGURATION, path: main }\n");
            write_platform_source(&root.join("main"), "X", "AAAA");
            Self { root }
        }

        fn add_extension(&self, name: &str, path: &str, object: &str) {
            let mut yaml = fs::read_to_string(self.root.join("v8project.yaml")).unwrap();
            yaml.push_str(&format!(
                " - {{ name: {name}, type: EXTENSION, path: {path} }}\n"
            ));
            write(&self.root.join("v8project.yaml"), &yaml);
            write_platform_source(&self.root.join(path), object, object);
        }

        fn services(
            &self,
        ) -> (
            crate::domain::source_snapshot::ResolvedSourceSelection,
            FilesystemSourceSnapshots,
        ) {
            (
                resolve_source_selection(&self.root, Some("main"), &[]).unwrap(),
                FilesystemSourceSnapshots::new(&self.root).unwrap(),
            )
        }

        fn controlled_service(
            &self,
            limits: SnapshotLimits,
            clock: Arc<dyn SnapshotClock>,
            hook: Arc<dyn CaptureHook>,
        ) -> FilesystemSourceSnapshots {
            FilesystemSourceSnapshots {
                workspace: canonical_workspace(&self.root).unwrap(),
                limits,
                clock,
                hook,
            }
        }
    }

    fn write_platform_source(root: &Path, object: &str, module: &str) {
        write(&root.join("Configuration.xml"), &format!("<MetaDataObject><Configuration><ChildObjects><CommonModule>{object}</CommonModule></ChildObjects></Configuration></MetaDataObject>"));
        write(&root.join(format!("CommonModules/{object}.xml")), &format!("<MetaDataObject><CommonModule><Properties><Name>{object}</Name></Properties><ChildObjects/></CommonModule></MetaDataObject>"));
        write(
            &root.join(format!("CommonModules/{object}/Ext/Module.bsl")),
            module,
        );
    }

    fn temp_root(name: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root =
            std::env::temp_dir().join(format!("unica-{name}-{}-{nonce}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        root
    }

    fn write(path: &Path, text: &str) {
        write_bytes(path, text.as_bytes());
    }

    fn write_bytes(path: &Path, bytes: &[u8]) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, bytes).unwrap();
    }

    #[derive(Default)]
    struct FixedClock;

    impl SnapshotClock for FixedClock {
        fn now(&self) -> Duration {
            Duration::ZERO
        }
    }

    #[derive(Default)]
    struct AdvancingClock(AtomicU64);

    impl SnapshotClock for AdvancingClock {
        fn now(&self) -> Duration {
            Duration::from_millis(self.0.fetch_add(1, Ordering::SeqCst))
        }
    }

    #[derive(Debug, Clone, Copy)]
    enum RaceAction {
        Add,
        Remove,
        Write,
        Replace,
        ReplaceAfterOpen,
        GrowAfterOpen,
        #[cfg(unix)]
        ParentSymlinkSwap,
        #[cfg(unix)]
        ParentSymlinkSwapBeforeOpen,
        #[cfg(unix)]
        FifoSwapBeforeOpen,
    }

    #[derive(Debug, Clone, Copy)]
    enum MappingMutation {
        RenameSource,
        DeleteMap,
        MalformedMap,
        #[cfg(unix)]
        SymlinkMap,
    }

    #[derive(Debug, Clone, Copy)]
    enum FinalValidationMutation {
        AddParentConfigurations,
        AddEdtProjectPmf,
        GrowPresent(u64),
    }

    struct FinalValidationMutationHook {
        root: PathBuf,
        mutation: FinalValidationMutation,
        fired: std::sync::atomic::AtomicBool,
    }

    impl FinalValidationMutationHook {
        fn new(root: PathBuf, mutation: FinalValidationMutation) -> Self {
            Self {
                root,
                mutation,
                fired: std::sync::atomic::AtomicBool::new(false),
            }
        }
    }

    impl CaptureHook for FinalValidationMutationHook {
        fn on_event(&self, event: &CaptureEvent) {
            if !matches!(event, CaptureEvent::BeforeFinalIdentityValidation)
                || self.fired.swap(true, Ordering::SeqCst)
            {
                return;
            }
            match self.mutation {
                FinalValidationMutation::AddParentConfigurations => write(
                    &self.root.join("main/Ext/ParentConfigurations.bin"),
                    "appeared",
                ),
                FinalValidationMutation::AddEdtProjectPmf => {
                    write(&self.root.join("edt/DT-INF/PROJECT.PMF"), "appeared")
                }
                FinalValidationMutation::GrowPresent(length) => write_bytes(
                    &self.root.join("main/CommonModules/X/Ext/Module.bsl"),
                    &vec![b'G'; usize::try_from(length).unwrap()],
                ),
            }
        }
    }

    struct MappingMutationHook {
        root: PathBuf,
        action: MappingMutation,
        fired: std::sync::atomic::AtomicBool,
    }

    impl MappingMutationHook {
        fn new(root: PathBuf, action: MappingMutation) -> Self {
            Self {
                root,
                action,
                fired: std::sync::atomic::AtomicBool::new(false),
            }
        }
    }

    impl CaptureHook for MappingMutationHook {
        fn on_event(&self, event: &CaptureEvent) {
            if !matches!(event, CaptureEvent::InitialPathScansComplete)
                || self.fired.swap(true, Ordering::SeqCst)
            {
                return;
            }
            let map = self.root.join("v8project.yaml");
            match self.action {
                MappingMutation::RenameSource => {
                    let yaml = fs::read_to_string(&map).unwrap();
                    write(&map, &yaml.replace("name: main", "name: renamed"));
                }
                MappingMutation::DeleteMap => fs::remove_file(map).unwrap(),
                MappingMutation::MalformedMap => write(&map, "source-set: ["),
                #[cfg(unix)]
                MappingMutation::SymlinkMap => {
                    use std::os::unix::fs::symlink;
                    fs::remove_file(&map).unwrap();
                    let outside = self.root.join("outside-map.yaml");
                    write(&outside, "source-set: []");
                    symlink(outside, map).unwrap();
                }
            }
        }
    }

    struct RaceHook {
        root: PathBuf,
        action: RaceAction,
        fired: std::sync::atomic::AtomicBool,
    }

    impl RaceHook {
        fn new(root: PathBuf, action: RaceAction) -> Self {
            Self {
                root,
                action,
                fired: std::sync::atomic::AtomicBool::new(false),
            }
        }
    }

    impl CaptureHook for RaceHook {
        fn on_event(&self, event: &CaptureEvent) {
            let target_event = match self.action {
                #[cfg(unix)]
                RaceAction::ParentSymlinkSwapBeforeOpen | RaceAction::FifoSwapBeforeOpen => {
                    matches!(event, CaptureEvent::BeforeContainedOpen(path) if path.ends_with("Module.bsl"))
                }
                RaceAction::ReplaceAfterOpen | RaceAction::GrowAfterOpen => {
                    matches!(event, CaptureEvent::ContainedOpenEstablished(path) if path.ends_with("Module.bsl"))
                }
                _ => {
                    matches!(event, CaptureEvent::FileHashed(path) if path.ends_with("Module.bsl"))
                }
            };
            if !target_event || self.fired.swap(true, Ordering::SeqCst) {
                return;
            }
            let module = self.root.join("main/CommonModules/X/Ext/Module.bsl");
            match self.action {
                RaceAction::Add => write(
                    &self.root.join("main/CommonModules/X/Ext/ManagerModule.bsl"),
                    "new",
                ),
                RaceAction::Remove => fs::remove_file(module).unwrap(),
                RaceAction::Write => fs::write(module, "BBBB").unwrap(),
                RaceAction::Replace => {
                    let replacement = self.root.join("replacement");
                    fs::write(&replacement, "CCCC").unwrap();
                    fs::remove_file(&module).unwrap();
                    fs::rename(replacement, module).unwrap();
                }
                RaceAction::ReplaceAfterOpen => {
                    let replacement = self.root.join("replacement-after-open");
                    fs::write(&replacement, "CCCC").unwrap();
                    fs::remove_file(&module).unwrap();
                    fs::rename(replacement, module).unwrap();
                }
                RaceAction::GrowAfterOpen => {
                    fs::write(module, vec![b'G'; 1024 * 1024]).unwrap();
                }
                #[cfg(unix)]
                RaceAction::ParentSymlinkSwap => {
                    use std::os::unix::fs::symlink;
                    let ext = self.root.join("main/CommonModules/X/Ext");
                    let saved = self.root.join("main/CommonModules/X/Ext-saved");
                    fs::rename(&ext, &saved).unwrap();
                    symlink("Ext-saved", ext).unwrap();
                }
                #[cfg(unix)]
                RaceAction::ParentSymlinkSwapBeforeOpen => {
                    use std::os::unix::fs::symlink;
                    let ext = self.root.join("main/CommonModules/X/Ext");
                    let saved = self.root.join("main/CommonModules/X/Ext-saved");
                    fs::rename(&ext, &saved).unwrap();
                    symlink("Ext-saved", ext).unwrap();
                }
                #[cfg(unix)]
                RaceAction::FifoSwapBeforeOpen => {
                    use std::os::unix::ffi::OsStrExt;
                    fs::remove_file(&module).unwrap();
                    let path = std::ffi::CString::new(module.as_os_str().as_bytes()).unwrap();
                    assert_eq!(unsafe { libc::mkfifo(path.as_ptr(), 0o600) }, 0);
                }
            }
        }
    }

    fn assert_retryable_race(action: RaceAction) {
        let fixture = Fixture::new(&format!("snapshot-race-{action:?}"));
        let selection = resolve_source_selection(&fixture.root, Some("main"), &[]).unwrap();
        let hook = Arc::new(RaceHook::new(fixture.root.clone(), action));
        let service =
            fixture.controlled_service(SnapshotLimits::default(), Arc::new(FixedClock), hook);
        let error = service
            .capture_authoritative(&selection.analysis, &[], 1)
            .unwrap_err();
        assert_eq!(
            error.reason,
            SnapshotCaptureReason::SourceChangedDuringCapture,
            "{action:?}: {error:?}"
        );
        assert!(error.retryable());
    }

    #[cfg(unix)]
    fn unix_timestamps(path: &Path) -> (libc::timespec, libc::timespec) {
        use std::os::unix::fs::MetadataExt;
        let metadata = fs::metadata(path).unwrap();
        (
            libc::timespec {
                tv_sec: metadata.atime(),
                tv_nsec: metadata.atime_nsec(),
            },
            libc::timespec {
                tv_sec: metadata.mtime(),
                tv_nsec: metadata.mtime_nsec(),
            },
        )
    }

    #[cfg(unix)]
    fn restore_unix_timestamps(path: &Path, times: (libc::timespec, libc::timespec)) {
        use std::os::unix::ffi::OsStrExt;
        let c = std::ffi::CString::new(path.as_os_str().as_bytes()).unwrap();
        let values = [times.0, times.1];
        assert_eq!(
            unsafe { libc::utimensat(libc::AT_FDCWD, c.as_ptr(), values.as_ptr(), 0) },
            0
        );
    }
}
