include!(concat!(env!("OUT_DIR"), "/v13_read_port_modules.rs"));
use metadata::identity_metadata_payload_from_evidence;
pub(crate) use support::object_support_by_uuid;
use support::{metadata_support_status, object_support_from_state, object_support_rule};
use unica_execution_contract::view_error::ViewError;
use unica_source_contract::portable_path::PortablePathExt;
use unica_source_designer::module_layout::module_relative as shared_module_relative;

use crate::infrastructure::v13_large_configuration::{RegistrationCache, RetainedSupportState};
#[cfg(any(test, feature = "test-support"))]
use std::cell::RefCell;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;
use unica_address::address::NodeKind;
use unica_address::source_target::MetadataAddress;
use unica_metadata::support_state::ObjectSupportState;
use unica_metadata::validation_subject::MetadataChildProfile;
use unica_refusal::{RefusalCode, RefusalDetail};
use unica_source_contract::read::{FileStamp, SourceDirectory};
use unica_source_contract::SourceSetKind;
use unica_source_designer::layout::{
    attached_resource_relative as shared_attached_resource_relative,
    metadata_descriptor_relative as shared_metadata_descriptor_relative,
};
use unica_work_control::cancellation::CancellationToken;
use unica_work_control::provider_deadline::ProviderDeadline;
use unica_workspace::documents::{DocumentText, SessionDocuments};

#[cfg(any(test, feature = "test-support"))]
thread_local! {
    static REVIEW_BEFORE_REVISION_IDENTITY: RefCell<Option<Box<dyn FnMut()>>> = RefCell::new(None);
    static REVIEW_AFTER_REVISION_IDENTITY: RefCell<Option<Box<dyn FnMut()>>> = RefCell::new(None);
}

#[cfg(any(test, feature = "test-support"))]
pub fn review_set_revision_identity_hooks(
    before: impl FnMut() + 'static,
    after: impl FnMut() + 'static,
) {
    REVIEW_BEFORE_REVISION_IDENTITY.with(|slot| *slot.borrow_mut() = Some(Box::new(before)));
    REVIEW_AFTER_REVISION_IDENTITY.with(|slot| *slot.borrow_mut() = Some(Box::new(after)));
}

#[cfg(any(test, feature = "test-support"))]
pub fn review_clear_revision_identity_hooks() {
    REVIEW_BEFORE_REVISION_IDENTITY.with(|slot| *slot.borrow_mut() = None);
    REVIEW_AFTER_REVISION_IDENTITY.with(|slot| *slot.borrow_mut() = None);
}

/// One captured template body and its read-only format verdict. Validation consumes
/// this value and does not use display paths as filesystem authority.
pub struct TemplateValidationInput {
    pub(crate) artifact: PathBuf,
    pub(crate) text: String,
    pub(crate) format_guard: crate::application::ports::FormatGuardCheck,
}

/// Сохранённые входы проверки формы; пути не дают права повторного чтения.
pub struct FormValidationInput {
    pub(crate) artifact: PathBuf,
    pub(crate) text: String,
    pub(crate) module: Option<String>,
    pub(crate) config_context: bool,
    pub(crate) metadata_owner: Option<NodeKind>,
    pub(crate) format_guard: crate::application::ports::FormatGuardCheck,
}

/// Size from which `Configuration.xml` is parsed by the streaming route. It
/// chooses a parser, not a refusal: every size is read (#1119).
// resource-limit: view.configuration-streaming-threshold-bytes
// class: step
// unit: bytes
// scope: file
// value: 8388608
// resource: Размер Configuration.xml, с которого корень разбирается потоковым путём вместо полного дерева.
// status: unproven
// origin: bbf0a901e
// problem: неизвестно
// on-reach: Выбор разборщика, а не отказ: больший файл читается потоком (#1130), меньший — полным деревом с кешем корня.
// issue: #1119
// evidence: Замер 25.10.2026: в выгрузке УТ 11 больше 8 МиБ только два XML — служебный ConfigDumpInfo.xml (16 480 480 байт, не узел и не в индексе Git) и табличный макет MXL (9 537 408 байт), который читается без этого предела; дескрипторов, форм и макетов СКД больше 8 МиБ нет.
const CONFIGURATION_STREAMING_THRESHOLD_BYTES: u64 = 8 * 1024 * 1024;

/// Один разбор дескриптора объекта: свойства и заимствование из тех же байтов.
pub(crate) struct MetadataLocalRead {
    pub(crate) info: unica_metadata::validation_subject::MetaLocalInfo,
    /// `None` — набор не вида `extension`: заимствования там не бывает.
    pub(crate) borrowing: Option<unica_metadata::borrowing::MetaBorrowing>,
    /// Точное состояние поддержки объекта: его отдаёт `view` (#2158), а
    /// `info.support` остаётся грубым статусом для проверок.
    pub(crate) support_state: ObjectSupportState,
    /// Идентификатор объекта из того же описания: состав подсистемы может
    /// назвать объект им, а не именем.
    pub(crate) uuid: Option<uuid::Uuid>,
}

/// Идентификатор объекта из его описания: первый `uuid` — у самого объекта,
/// он идёт сразу за корнем `MetaDataObject`.
pub(crate) fn descriptor_uuid(descriptor: &[u8]) -> Option<uuid::Uuid> {
    let text = std::str::from_utf8(descriptor).ok()?;
    let start = text.find("uuid=\"")? + "uuid=\"".len();
    uuid::Uuid::parse_str(text.get(start..start + 36)?).ok()
}

/// The stamps of the regular files of one directory, by name.
type DirectoryStamps = Arc<std::collections::HashMap<std::ffi::OsString, FileStamp>>;

/// One actor-issued read authority for one admitted source set. Reads are
/// descriptor-relative to this retained directory; the snapshot identity is
/// issued at admission and never derived from a scan of the source tree.
pub struct ProviderReadAuthority {
    source_set: String,
    source_set_identity: String,
    source_set_kind: SourceSetKind,
    root: Arc<SourceDirectory>,
    snapshot_identity: String,
    /// A physical-path lookup may prove one external owner without enumerating
    /// unrelated artifacts. It never supplies a partial inventory payload.
    point_external_artifact:
        Option<unica_source_designer::external_artifact::ExternalArtifactBinding>,
    /// Parsed root artifacts retained by the workspace actor across calls.
    artifacts: Arc<RegistrationCache>,
    /// The parsed `Ext/ParentConfigurations.bin` marker. One authority serves
    /// one admitted operation, so the marker is read and parsed once instead
    /// of once per owner descriptor that asks for its support state.
    support_state: std::sync::Mutex<Option<RetainedSupportState>>,
    /// Completed physical/logical external bindings for this admitted operation.
    external_artifacts: std::sync::Mutex<
        Option<Arc<unica_source_designer::external_artifact::ExternalArtifactInventory>>,
    >,
    /// Open editor buffers of the session this read serves; a module with a
    /// buffer is read from it, never from its file.
    documents: Option<SessionDocuments>,
    /// Stamps of the descriptors of one kind directory, taken in one pass per
    /// operation (`None`: the directory could not be listed that way).
    descriptor_stamps:
        std::sync::Mutex<std::collections::HashMap<PathBuf, Option<DirectoryStamps>>>,
    /// Whether descriptor stamps come from a pass over their directory: only
    /// an operation that lists a whole kind does so; a read of one object
    /// opens only its own files.
    directory_stamps: std::sync::atomic::AtomicBool,
    #[cfg(any(test, feature = "test-support"))]
    module_source_reads: std::sync::Mutex<std::collections::BTreeMap<String, usize>>,
    #[cfg(any(test, feature = "test-support"))]
    metadata_descriptor_reads: std::sync::Mutex<std::collections::BTreeMap<String, usize>>,
    #[cfg(any(test, feature = "test-support"))]
    configuration_payload_reads: std::sync::atomic::AtomicUsize,
    #[cfg(any(test, feature = "test-support"))]
    support_state_reads: std::sync::atomic::AtomicUsize,
}

impl ProviderReadAuthority {
    #[cfg(any(test, feature = "test-support"))]
    pub(crate) fn new(
        source_set: impl Into<String>,
        source_set_identity: impl Into<String>,
        source_set_kind: SourceSetKind,
        root: Arc<SourceDirectory>,
    ) -> Self {
        Self::new_local_snapshot(
            source_set,
            source_set_identity,
            source_set_kind,
            root,
            "test-local-snapshot".to_string(),
        )
    }

    pub fn new_local_snapshot(
        source_set: impl Into<String>,
        source_set_identity: impl Into<String>,
        source_set_kind: SourceSetKind,
        root: Arc<SourceDirectory>,
        snapshot_identity: String,
    ) -> Self {
        Self {
            source_set: source_set.into(),
            source_set_identity: source_set_identity.into(),
            source_set_kind,
            root,
            snapshot_identity,
            point_external_artifact: None,
            artifacts: Arc::new(RegistrationCache::default()),
            support_state: std::sync::Mutex::new(None),
            external_artifacts: std::sync::Mutex::new(None),
            documents: None,
            descriptor_stamps: std::sync::Mutex::new(std::collections::HashMap::new()),
            directory_stamps: std::sync::atomic::AtomicBool::new(false),
            #[cfg(any(test, feature = "test-support"))]
            module_source_reads: std::sync::Mutex::new(std::collections::BTreeMap::new()),
            #[cfg(any(test, feature = "test-support"))]
            metadata_descriptor_reads: std::sync::Mutex::new(std::collections::BTreeMap::new()),
            #[cfg(any(test, feature = "test-support"))]
            configuration_payload_reads: std::sync::atomic::AtomicUsize::new(0),
            #[cfg(any(test, feature = "test-support"))]
            support_state_reads: std::sync::atomic::AtomicUsize::new(0),
        }
    }

    pub(crate) fn with_external_point_binding(
        mut self,
        binding: unica_source_designer::external_artifact::ExternalArtifactBinding,
    ) -> Self {
        self.point_external_artifact = Some(binding);
        self
    }
    pub(crate) fn point_external_owner_matches(&self, kind: &str, name: &str) -> Option<bool> {
        self.point_external_artifact
            .as_ref()
            .map(|binding| binding.kind() == kind && binding.name() == name)
    }

    /// Shares the actor-retained root artifacts of this source set.
    pub fn with_artifacts(mut self, artifacts: Arc<RegistrationCache>) -> Self {
        self.artifacts = artifacts;
        self
    }

    /// Proves the descriptors of every registered top-level object once, in
    /// the background, so the first page of a metadata branch finds them
    /// retained. Runs once per retained artifact set; waits while an MCP call
    /// executes and stops when the actor lets the artifacts go.
    pub fn start_owner_warmup(&self) {
        if !matches!(
            self.source_set_kind,
            SourceSetKind::Configuration | SourceSetKind::Extension
        ) || !self.artifacts.claim_owner_warmup()
        {
            return;
        }
        let owner = Arc::downgrade(&self.artifacts);
        let source_set = self.source_set.clone();
        let identity = self.source_set_identity.clone();
        let kind = self.source_set_kind;
        let root = Arc::clone(&self.root);
        std::thread::spawn(move || {
            let wait = || loop {
                if owner.strong_count() == 0 {
                    return false;
                }
                if crate::infrastructure::name_index::foreground_calls() == 0 {
                    return true;
                }
                std::thread::sleep(crate::infrastructure::name_index::NAME_INDEX_YIELD_POLL);
            };
            // A read holds the artifacts only while it proves one descriptor,
            // so the actor can let them go between members.
            let read = || {
                let artifacts = owner.upgrade()?;
                let mut read = Self::new_local_snapshot(
                    source_set.clone(),
                    identity.clone(),
                    kind,
                    Arc::clone(&root),
                    String::new(),
                );
                read.artifacts = artifacts;
                Some(read)
            };
            if !wait() {
                return;
            }
            let Some(index) = read().and_then(|read| {
                read.streamed_configuration_registration_index(&|| Ok(()))
                    .ok()
            }) else {
                return;
            };
            for layout in unica_source_designer::layout::METADATA_KINDS {
                let Ok(names) = index.names_for_kind(layout.tag, &|| Ok(())) else {
                    continue;
                };
                for name in names {
                    if !wait() {
                        return;
                    }
                    let Ok(target) =
                        MetadataAddress::parse_logical(&format!("{}.{name}", layout.tag))
                    else {
                        continue;
                    };
                    let Some(read) = read() else {
                        return;
                    };
                    let _ = read.metadata_owner_evidence(&target, &mut || Ok(()));
                    if layout.tag == NodeKind::CommonModule.as_str() {
                        if let Ok(at) = unica_address::address::QualifiedAddress::parse(&format!(
                            "{}:{}.{name}",
                            read.source_set, layout.tag
                        )) {
                            let _ = read.common_module_synonym(&at);
                        }
                    }
                }
            }
        });
    }

    /// The stamp of a descriptor before it is read, for the retained owner
    /// facts: from one pass over its directory per operation, or from the
    /// file itself when the directory cannot be listed that way. `None` when
    /// the file is absent or unreadable; the caller then reads it uncached.
    pub(crate) fn descriptor_stamp(&self, relative: &Path) -> Option<FileStamp> {
        if self
            .directory_stamps
            .load(std::sync::atomic::Ordering::Relaxed)
        {
            if let (Some(parent), Some(name)) = (relative.parent(), relative.file_name()) {
                let batch = {
                    let mut batches = self.descriptor_stamps.lock().ok()?;
                    batches
                        .entry(parent.to_path_buf())
                        .or_insert_with(|| {
                            self.root
                                .relative_directory_file_stamps(parent)
                                .ok()
                                .flatten()
                                .map(|stamps| Arc::new(stamps.into_iter().collect()))
                        })
                        .clone()
                };
                if let Some(batch) = batch {
                    if let Some(stamp) = batch.get(name) {
                        return Some(*stamp);
                    }
                }
            }
        }
        self.root.relative_file_stamp(relative).ok().flatten()
    }

    /// This operation lists a whole kind: the stamps of its descriptors are
    /// taken in one pass over the kind directory.
    pub(crate) fn stamp_descriptors_by_directory(&self) {
        self.directory_stamps
            .store(true, std::sync::atomic::Ordering::Relaxed);
    }

    /// Reads modules of `documents` from their open buffers.
    pub fn with_documents(mut self, documents: SessionDocuments) -> Self {
        self.documents = (!documents.is_empty()).then_some(documents);
        self
    }

    pub(crate) fn source_set(&self) -> &str {
        &self.source_set
    }

    /// The retained source root every read of this authority is relative to.
    pub fn retained_root(&self) -> &SourceDirectory {
        &self.root
    }

    /// The set's artifacts retained by the workspace actor across calls.
    pub fn artifacts(&self) -> &Arc<RegistrationCache> {
        &self.artifacts
    }

    pub(crate) fn source_set_identity(&self) -> &str {
        &self.source_set_identity
    }

    pub(crate) const fn source_set_kind(&self) -> SourceSetKind {
        self.source_set_kind
    }

    pub(crate) fn root_path(&self) -> &Path {
        self.root.path()
    }

    pub(crate) fn exact_revision(
        &self,
        deadline: ProviderDeadline,
        cancellation: &CancellationToken,
    ) -> Result<String, ViewError> {
        #[cfg(any(test, feature = "test-support"))]
        REVIEW_BEFORE_REVISION_IDENTITY.with(|slot| {
            if let Some(hook) = slot.borrow_mut().as_mut() {
                hook();
            }
        });
        self.root.validate_named_identity().map_err(|error| {
            ViewError::detailed(RefusalDetail::SourceUnreadable, error.to_string())
        })?;
        #[cfg(any(test, feature = "test-support"))]
        REVIEW_AFTER_REVISION_IDENTITY.with(|slot| {
            if let Some(hook) = slot.borrow_mut().as_mut() {
                hook();
            }
        });
        if cancellation.is_cancelled() {
            return Err(ViewError::new(
                RefusalCode::Cancelled,
                "logical read was cancelled",
            ));
        }
        if deadline.is_elapsed() {
            return Err(ViewError::new(
                RefusalCode::DeadlineExceeded,
                "logical read operation deadline elapsed",
            ));
        }
        self.root.validate_named_identity().map_err(|error| {
            ViewError::detailed(RefusalDetail::SourceUnreadable, error.to_string())
        })?;
        Ok(self.snapshot_identity.clone())
    }

    fn read_relative_with_checkpoint(
        &self,
        relative: &Path,
        checkpoint: &mut dyn FnMut() -> Result<(), ViewError>,
    ) -> Result<Vec<u8>, ViewError> {
        self.read_optional_relative_with_checkpoint(relative, checkpoint)?
            .ok_or_else(|| {
                ViewError::detailed(
                    RefusalDetail::SourceUnreadable,
                    format!("relative source `{}` is absent", relative.portable()),
                )
            })
    }

    fn read_optional_relative_with_checkpoint(
        &self,
        relative: &Path,
        checkpoint: &mut dyn FnMut() -> Result<(), ViewError>,
    ) -> Result<Option<Vec<u8>>, ViewError> {
        if let Some(buffer) = self.open_buffer(relative) {
            checkpoint()?;
            return Ok(Some(buffer.text.as_bytes().to_vec()));
        }
        let mut bytes = Vec::new();
        let mut interrupted = None;
        let visited = self.root.visit_relative_regular_chunks(
            relative,
            || {
                checkpoint().map_err(|error| {
                    interrupted = Some(error);
                    std::io::Error::other("source read interrupted")
                })
            },
            |chunk| {
                bytes
                    .try_reserve(chunk.len())
                    .map_err(std::io::Error::other)?;
                bytes.extend_from_slice(chunk);
                Ok(())
            },
        );
        if let Some(error) = interrupted {
            return Err(error);
        }
        match visited {
            Ok(_) => {
                checkpoint()?;
                Ok(Some(bytes))
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                checkpoint()?;
                Ok(None)
            }
            Err(error) => Err(ViewError::detailed(
                RefusalDetail::SourceUnreadable,
                error.to_string(),
            )),
        }
    }

    /// The whole source file, whatever its size (#1119).
    pub(crate) fn read_relative_whole(&self, relative: &Path) -> Result<Vec<u8>, ViewError> {
        self.read_relative_with_checkpoint(relative, &mut || Ok(()))
    }

    /// The whole source file when it exists, whatever its size (#1119).
    pub(crate) fn read_optional_relative_whole(
        &self,
        relative: &Path,
    ) -> Result<Option<Vec<u8>>, ViewError> {
        self.read_optional_relative_with_checkpoint(relative, &mut || Ok(()))
    }

    /// Preserve the established parser and its full compatibility behavior
    /// for files that already fit the ordinary read. The streaming route is
    /// selected only when that read would refuse the file by total size.
    pub(crate) fn configuration_xml_requires_streaming(&self) -> bool {
        self.root
            .open_relative_regular_nofollow(Path::new("Configuration.xml"))
            .and_then(|file| file.metadata())
            .is_ok_and(|metadata| metadata.len() > CONFIGURATION_STREAMING_THRESHOLD_BYTES)
    }

    /// Заимствование объекта, чей сопутствующий файл расширение не несёт;
    /// см. [`inherited_companion`].
    pub(crate) fn inherited_resource(
        &self,
        target: &MetadataAddress,
        resource: &str,
    ) -> Result<Option<unica_metadata::borrowing::MetaBorrowing>, ViewError> {
        inherited_companion(&self.root, self.source_set_kind, target, resource)
            .map_err(|error| ViewError::detailed(RefusalDetail::SourceUnreadable, error))
    }

    pub fn metadata_descriptor(&self, target: &MetadataAddress) -> Result<Vec<u8>, ViewError> {
        self.metadata_descriptor_with_checkpoint(target, &mut || Ok(()))
    }

    fn metadata_descriptor_with_checkpoint(
        &self,
        target: &MetadataAddress,
        checkpoint: &mut dyn FnMut() -> Result<(), ViewError>,
    ) -> Result<Vec<u8>, ViewError> {
        #[cfg(any(test, feature = "test-support"))]
        {
            *self
                .metadata_descriptor_reads
                .lock()
                .expect("metadata descriptor read counter is not poisoned")
                .entry(target.as_str().to_string())
                .or_default() += 1;
        }
        let canonical = shared_metadata_descriptor_relative(target, self.source_set_kind)
            .map_err(|message| ViewError::detailed(RefusalDetail::SourceUnreadable, message))?;
        let relative = if self.is_external_source_set() {
            self.external_physical_path(target, canonical, checkpoint)?
        } else {
            canonical
        };
        self.read_optional_relative_with_checkpoint(&relative, checkpoint)?
            .ok_or_else(|| {
                ViewError::new(
                    RefusalCode::NotFound,
                    format!("metadata descriptor `{}` is absent", relative.portable()),
                )
            })
    }

    pub(crate) fn metadata_descriptor_export_path(
        &self,
        target: &MetadataAddress,
    ) -> Result<String, ViewError> {
        path_text(self.metadata_descriptor_relative(target)?)
    }

    pub(crate) fn attached_resource_export_path(
        &self,
        target: &MetadataAddress,
        resource: &str,
    ) -> Result<String, ViewError> {
        path_text(self.attached_resource_relative(target, resource)?)
    }

    pub(crate) fn module_export_path(&self, target: &MetadataAddress) -> Result<String, ViewError> {
        path_text(self.module_relative(target)?)
    }

    pub(crate) fn common_template_facts(
        &self,
        target: &MetadataAddress,
    ) -> Result<unica_metadata::validation_subject::CommonTemplateDescriptorFacts, ViewError> {
        let bytes = self.metadata_descriptor(target)?;
        crate::infrastructure::family_codecs::metadata_read_codec()
            .common_template(&bytes, target)
            .map_err(|error| ViewError::detailed(RefusalDetail::SourceUnreadable, error))
    }

    pub(crate) fn metadata_child_profile(
        &self,
        target: &MetadataAddress,
    ) -> Result<MetadataChildProfile, ViewError> {
        if target.as_str().starts_with("CommonTemplate.") {
            return self
                .common_template_facts(target)
                .map(|facts| MetadataChildProfile::Template(facts.template_type));
        }
        let descriptor = self.metadata_descriptor(target)?;
        let owner_text = target
            .as_str()
            .split('.')
            .take(2)
            .collect::<Vec<_>>()
            .join(".");
        let owner = MetadataAddress::parse_logical(&owner_text).map_err(|error| {
            ViewError::detailed(RefusalDetail::SourceUnreadable, error.to_string())
        })?;
        let Some((parsed_target, profile)) =
            crate::infrastructure::family_codecs::metadata_read_codec()
                .child_profile(&descriptor, &owner)
                .map_err(|error| ViewError::detailed(RefusalDetail::SourceUnreadable, error))?
        else {
            return Err(ViewError::detailed(
                RefusalDetail::SourceUnreadable,
                "registered physical child descriptor has the wrong artifact kind",
            ));
        };
        if parsed_target.as_str() != target.as_str() {
            return Err(ViewError::detailed(
                RefusalDetail::SourceUnreadable,
                "registered physical child descriptor belongs to another logical address",
            ));
        }
        Ok(profile)
    }

    /// The open buffer that stands for `relative` in this read. Every read
    /// of a source file goes through here, so one session never sees the
    /// buffer in one answer and the file in another.
    fn open_buffer(&self, relative: &Path) -> Option<&DocumentText> {
        self.documents.as_ref()?.get(relative)
    }

    pub(crate) fn form_source_exists(&self, target: &MetadataAddress) -> Result<bool, ViewError> {
        self.attached_resource_exists(target, "Form.xml")
    }

    /// Whether the owner's attached resource is a local regular file. A path
    /// built from the layout alone does not prove that the file exists.
    pub(crate) fn attached_resource_exists(
        &self,
        target: &MetadataAddress,
        resource: &str,
    ) -> Result<bool, ViewError> {
        self.relative_source_exists(&self.attached_resource_relative(target, resource)?)
    }

    fn relative_source_exists(&self, relative: &Path) -> Result<bool, ViewError> {
        if self.open_buffer(relative).is_some() {
            return Ok(true);
        }
        match self.root.open_relative_regular_nofollow(relative) {
            Ok(_) => Ok(true),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(error) => Err(ViewError::detailed(
                RefusalDetail::SourceUnreadable,
                error.to_string(),
            )),
        }
    }

    #[cfg(any(test, feature = "test-support"))]
    pub(crate) fn metadata_descriptor_read_count(&self, target: &str) -> usize {
        self.metadata_descriptor_reads
            .lock()
            .expect("metadata descriptor read counter is not poisoned")
            .get(target)
            .copied()
            .unwrap_or_default()
    }

    #[cfg(any(test, feature = "test-support"))]
    pub(crate) fn configuration_payload_read_count(&self) -> usize {
        self.configuration_payload_reads
            .load(std::sync::atomic::Ordering::Relaxed)
    }

    fn metadata_descriptor_relative(&self, target: &MetadataAddress) -> Result<PathBuf, ViewError> {
        let canonical = shared_metadata_descriptor_relative(target, self.source_set_kind)
            .map_err(|message| ViewError::detailed(RefusalDetail::SourceUnreadable, message))?;
        if self.is_external_source_set() {
            self.external_physical_path(target, canonical, &mut || Ok(()))
        } else {
            Ok(canonical)
        }
    }

    fn attached_resource_relative(
        &self,
        target: &MetadataAddress,
        resource: &str,
    ) -> Result<PathBuf, ViewError> {
        let canonical =
            shared_attached_resource_relative(target, resource, self.source_set_kind)
                .map_err(|message| ViewError::detailed(RefusalDetail::SourceUnreadable, message))?;
        if self.is_external_source_set() {
            self.external_physical_path(target, canonical, &mut || Ok(()))
        } else {
            Ok(canonical)
        }
    }

    fn module_relative(&self, target: &MetadataAddress) -> Result<PathBuf, ViewError> {
        let canonical = shared_module_relative(target, self.source_set_kind)
            .map_err(|message| ViewError::detailed(RefusalDetail::SourceUnreadable, message))?;
        if self.is_external_source_set() {
            self.external_physical_path(target, canonical, &mut || Ok(()))
        } else {
            Ok(canonical)
        }
    }

    fn is_external_source_set(&self) -> bool {
        matches!(
            self.source_set_kind,
            SourceSetKind::ExternalProcessor | SourceSetKind::ExternalReport
        )
    }
}

/// Объект, чей сопутствующий файл `resource` держит содержимое, в котором
/// лежит адрес: роль для `Rights.xml`, пакет для `Package.bin`, общая форма
/// или форма владельца для `Form.xml` (её модуль тоже), общий макет или макет
/// владельца для `Template.xml`. `None` — адрес вне такого содержимого.
pub(crate) fn companion_owner(
    at: &unica_address::address::QualifiedAddress,
    resource: &str,
) -> Option<MetadataAddress> {
    let (top, child) = match resource {
        "Rights.xml" => (NodeKind::Role, None),
        "Package.bin" => (NodeKind::XdtoPackage, None),
        "Form.xml" => (NodeKind::CommonForm, Some(NodeKind::Form)),
        "Template.xml" => (NodeKind::CommonTemplate, Some(NodeKind::Template)),
        _ => return None,
    };
    let segments = at.segments();
    let end = segments.iter().enumerate().find_map(|(index, segment)| {
        let kind = segment.kind();
        let matches = if index == 0 {
            kind == top
        } else {
            Some(kind) == child
        };
        (matches && segment.name().is_some()).then_some(index)
    })?;
    let path = segments[..=end]
        .iter()
        .map(|segment| {
            format!(
                "{}.{}",
                segment.kind().as_str(),
                segment.name().unwrap_or_default()
            )
        })
        .collect::<Vec<_>>()
        .join(".");
    MetadataAddress::parse_logical(&path).ok()
}

/// The whole regular file under `root`, read in chunks without a size limit.
fn read_whole_relative(root: &SourceDirectory, relative: &Path) -> std::io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    root.visit_relative_regular_chunks(
        relative,
        || Ok(()),
        |chunk| {
            bytes
                .try_reserve(chunk.len())
                .map_err(std::io::Error::other)?;
            bytes.extend_from_slice(chunk);
            Ok(())
        },
    )?;
    Ok(bytes)
}

/// Заимствование объекта, чей сопутствующий файл расширение не несёт.
///
/// Платформа 8.3.27 импортирует и выгружает заимствованные роль, форму,
/// пакет XDTO и макет без `Rights.xml`, `Form.xml`, `Package.bin` и
/// `Template.xml`: своего содержимого у такого объекта в расширении нет,
/// действует содержимое родителя. `Some` — именно этот случай: набор вида
/// `extension`, файла нет, а описание объекта указывает родителя через
/// `ExtendedConfigurationObject`. У дочернего объекта (формы или макета
/// владельца) родителя обязано указывать и описание владельца: у
/// собственного владельца расширения нет родителя, от которого ребёнок
/// унаследовал бы содержимое. Единственный владелец этого решения: его
/// спрашивают и чтение, и запись.
pub(crate) fn inherited_companion(
    root: &SourceDirectory,
    source_kind: SourceSetKind,
    target: &MetadataAddress,
    resource: &str,
) -> Result<Option<unica_metadata::borrowing::MetaBorrowing>, String> {
    if source_kind != SourceSetKind::Extension {
        return Ok(None);
    }
    let relative = shared_attached_resource_relative(target, resource, source_kind)?;
    match root.open_relative_regular_nofollow(&relative) {
        Ok(_) => return Ok(None),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.to_string()),
    }
    let descriptor_of = |target: &MetadataAddress| -> Result<Vec<u8>, String> {
        let relative = shared_metadata_descriptor_relative(target, source_kind)?;
        read_whole_relative(root, &relative).map_err(|error| error.to_string())
    };
    let descriptor = descriptor_of(target)?;
    if !crate::infrastructure::family_codecs::metadata_read_codec().extends_parent(&descriptor) {
        return Ok(None);
    }
    let parts = target.as_str().split('.').collect::<Vec<_>>();
    if parts.len() > 2 {
        let owner = MetadataAddress::parse_logical(&parts[..2].join("."))
            .map_err(|error| error.to_string())?;
        if !crate::infrastructure::family_codecs::metadata_read_codec()
            .extends_parent(&descriptor_of(&owner)?)
        {
            return Ok(None);
        }
    }
    crate::infrastructure::family_codecs::metadata_read_codec()
        .borrowing(&descriptor)
        .map(Some)
}

fn path_text(path: PathBuf) -> Result<String, ViewError> {
    let mut components = Vec::new();
    for component in path.components() {
        match component {
            std::path::Component::Normal(value) => components.push(value),
            _ => {
                return Err(ViewError::detailed(
                    RefusalDetail::SourceUnreadable,
                    "Platform XML export path is not a relative component path",
                ));
            }
        }
    }
    if components.is_empty() {
        return Err(ViewError::detailed(
            RefusalDetail::SourceUnreadable,
            "Platform XML export path is empty",
        ));
    }
    encode_export_path_components(components)
}

fn encode_export_path_components<'a>(
    components: impl IntoIterator<Item = &'a std::ffi::OsStr>,
) -> Result<String, ViewError> {
    let mut encoded = String::new();
    for component in components {
        let component = component.to_str().ok_or_else(|| {
            ViewError::detailed(
                RefusalDetail::SourceUnreadable,
                "Platform XML export path is not valid UTF-8",
            )
        })?;
        if !encoded.is_empty() {
            encoded.push('/');
        }
        encoded.push_str(component);
    }
    Ok(encoded)
}

/// Read access of the LSP host to one admitted source set.
impl unica_editor_analysis::ModuleSourceRead for ProviderReadAuthority {
    fn source_set(&self) -> &str {
        ProviderReadAuthority::source_set(self)
    }

    fn root_path(&self) -> &Path {
        ProviderReadAuthority::root_path(self)
    }

    fn module_source(&self, target: &MetadataAddress) -> Result<Option<String>, ViewError> {
        ProviderReadAuthority::module_source(self, target)
    }
}

#[cfg(test)]
mod export_path_tests {
    use std::ffi::OsStr;

    #[test]
    fn export_path_components_use_wire_slashes_independently_of_host() {
        let components = [
            OsStr::new("Reports"),
            OsStr::new("ParityReport"),
            OsStr::new("Forms"),
            OsStr::new("MainForm.xml"),
        ];

        assert_eq!(
            super::encode_export_path_components(components),
            Ok("Reports/ParityReport/Forms/MainForm.xml".to_string())
        );
    }
}
