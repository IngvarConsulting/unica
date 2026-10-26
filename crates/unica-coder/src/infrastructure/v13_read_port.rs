mod job_schedule;
use crate::application::v13::source_text::{ModuleText, ModuleTextAccumulator};
use unica_execution_contract::view_error::ViewError;
use unica_metadata::configuration::{CfHomePageData, CfInterfaceData};
use unica_metadata_designer::configuration::{
    cf_home_page_item_data, parse_cf_command_interface_xml, parse_cf_home_page_xml_strict,
    parse_cf_info_xml,
};
use unica_source_contract::portable_path::PortablePathExt;
use unica_source_designer::module_layout::module_relative as shared_module_relative;

use crate::infrastructure::v13_large_configuration::{
    read_configuration_root, RegistrationCache, RegistrationIndex, RetainedSupportState,
    StreamedConfigurationRoot,
};
use serde_json::{json, Value};
#[cfg(test)]
use std::cell::RefCell;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;
use unica_address::address::NodeKind;
use unica_address::address::QualifiedAddress;
use unica_address::source_target::{MetadataAddress, PLATFORM_XML_8_3_27_FORMAT_2_20};
use unica_forms::facts::{FormInfoData, FormSupportDetails, FormSupportRuleReason};
use unica_forms_designer::codec::parse_form_info_xml;
use unica_metadata::support_state::{
    ConfigurationSupportData, ConfigurationSupportState, ObjectSupportData, ObjectSupportState,
    SupportCounts,
};
use unica_metadata::validation_subject::MetadataChildProfile;
use unica_metadata::{MetaPredefinedItemsData, MetaSupportStatus, MetadataKind};
use unica_metadata_designer::subsystem::{
    build_subsystem_info_result, parse_subsystem_command_interface_data,
};
use unica_metadata_designer::support::{
    support_root_uuid_from_bytes, SupportState, SupportStateStreamParser,
};
use unica_metadata_designer::validation::{
    parse_child_profile_from_bytes, parse_common_template_descriptor_from_bytes,
};
use unica_refusal::{RefusalCode, RefusalDetail};
use unica_roles_designer::info::parse_role_info_xml;
use unica_source_contract::read::SourceDirectory;
use unica_source_contract::SourceSetKind;
use unica_source_designer::layout::{
    attached_resource_relative as shared_attached_resource_relative,
    metadata_descriptor_relative as shared_metadata_descriptor_relative,
};
use unica_source_designer::owner::{
    prove_already_read_metadata_owner, prove_already_read_source_set_owner,
    PlatformXmlSourceSetOwnerEvidence,
};
use unica_source_designer::probe::{
    classify_already_read_config_dump_info_xml, ConfigDumpInfoXmlKind,
};
use unica_templates_designer::mxl_codec::parse_mxl_info_xml;
use unica_work_control::cancellation::CancellationToken;
use unica_work_control::provider_deadline::ProviderDeadline;
use unica_workspace::documents::{DocumentText, SessionDocuments};
use unica_xdto_designer::info::{parse_xdto_info_bytes, XdtoInfoError};

#[cfg(test)]
thread_local! {
    static REVIEW_BEFORE_REVISION_IDENTITY: RefCell<Option<Box<dyn FnMut()>>> = RefCell::new(None);
    static REVIEW_AFTER_REVISION_IDENTITY: RefCell<Option<Box<dyn FnMut()>>> = RefCell::new(None);
}

#[cfg(test)]
pub(crate) fn review_set_revision_identity_hooks(
    before: impl FnMut() + 'static,
    after: impl FnMut() + 'static,
) {
    REVIEW_BEFORE_REVISION_IDENTITY.with(|slot| *slot.borrow_mut() = Some(Box::new(before)));
    REVIEW_AFTER_REVISION_IDENTITY.with(|slot| *slot.borrow_mut() = Some(Box::new(after)));
}

#[cfg(test)]
pub(crate) fn review_clear_revision_identity_hooks() {
    REVIEW_BEFORE_REVISION_IDENTITY.with(|slot| *slot.borrow_mut() = None);
    REVIEW_AFTER_REVISION_IDENTITY.with(|slot| *slot.borrow_mut() = None);
}

/// One captured template body and its read-only format verdict. Validation consumes
/// this value and does not use display paths as filesystem authority.
pub(crate) struct TemplateValidationInput {
    pub(crate) artifact: PathBuf,
    pub(crate) text: String,
    pub(crate) format_guard: crate::application::ports::FormatGuardCheck,
}

/// Сохранённые входы проверки формы; пути не дают права повторного чтения.
pub(crate) struct FormValidationInput {
    pub(crate) artifact: PathBuf,
    pub(crate) text: String,
    pub(crate) module: Option<String>,
    pub(crate) config_context: bool,
    pub(crate) metadata_owner: Option<NodeKind>,
    pub(crate) format_guard: crate::application::ports::FormatGuardCheck,
}

/// Size from which `Configuration.xml` is parsed by the streaming route. It
/// chooses a parser, not a refusal: every size is read (#1119).
const CONFIGURATION_STREAMING_THRESHOLD_BYTES: u64 = 8 * 1024 * 1024;

/// Один разбор дескриптора объекта: свойства и заимствование из тех же байтов.
pub(crate) struct MetadataLocalRead {
    pub(crate) info: unica_metadata::validation_subject::MetaLocalInfo,
    /// `None` — набор не вида `extension`: заимствования там не бывает.
    pub(crate) borrowing: Option<unica_metadata_designer::xml_model::MetaBorrowing>,
}

/// One actor-issued read authority for one admitted source set. Reads are
/// descriptor-relative to this retained directory; the snapshot identity is
/// issued at admission and never derived from a scan of the source tree.
pub(crate) struct ProviderReadAuthority {
    source_set: String,
    source_set_identity: String,
    source_set_kind: SourceSetKind,
    root: Arc<SourceDirectory>,
    snapshot_identity: String,
    /// Parsed root artifacts retained by the workspace actor across calls.
    artifacts: Arc<RegistrationCache>,
    /// The parsed `Ext/ParentConfigurations.bin` marker. One authority serves
    /// one admitted operation, so the marker is read and parsed once instead
    /// of once per owner descriptor that asks for its support state.
    support_state: std::sync::Mutex<Option<RetainedSupportState>>,
    /// Open editor buffers of the session this read serves; a module with a
    /// buffer is read from it, never from its file.
    documents: Option<SessionDocuments>,
    #[cfg(test)]
    module_source_reads: std::sync::Mutex<std::collections::BTreeMap<String, usize>>,
    #[cfg(test)]
    metadata_descriptor_reads: std::sync::Mutex<std::collections::BTreeMap<String, usize>>,
    #[cfg(test)]
    configuration_payload_reads: std::sync::atomic::AtomicUsize,
    #[cfg(test)]
    support_state_reads: std::sync::atomic::AtomicUsize,
}

impl ProviderReadAuthority {
    #[cfg(test)]
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

    pub(crate) fn new_local_snapshot(
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
            artifacts: Arc::new(RegistrationCache::default()),
            support_state: std::sync::Mutex::new(None),
            documents: None,
            #[cfg(test)]
            module_source_reads: std::sync::Mutex::new(std::collections::BTreeMap::new()),
            #[cfg(test)]
            metadata_descriptor_reads: std::sync::Mutex::new(std::collections::BTreeMap::new()),
            #[cfg(test)]
            configuration_payload_reads: std::sync::atomic::AtomicUsize::new(0),
            #[cfg(test)]
            support_state_reads: std::sync::atomic::AtomicUsize::new(0),
        }
    }

    /// Shares the actor-retained root artifacts of this source set.
    pub(crate) fn with_artifacts(mut self, artifacts: Arc<RegistrationCache>) -> Self {
        self.artifacts = artifacts;
        self
    }

    /// Reads modules of `documents` from their open buffers.
    pub(crate) fn with_documents(mut self, documents: SessionDocuments) -> Self {
        self.documents = (!documents.is_empty()).then_some(documents);
        self
    }

    pub(crate) fn source_set(&self) -> &str {
        &self.source_set
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
        #[cfg(test)]
        REVIEW_BEFORE_REVISION_IDENTITY.with(|slot| {
            if let Some(hook) = slot.borrow_mut().as_mut() {
                hook();
            }
        });
        self.root.validate_named_identity().map_err(|error| {
            ViewError::detailed(RefusalDetail::SourceUnreadable, error.to_string())
        })?;
        #[cfg(test)]
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

    pub(crate) fn configuration_payload_with_checkpoint(
        &self,
        checkpoint: &mut dyn FnMut() -> Result<(), ViewError>,
    ) -> Result<Arc<Value>, ViewError> {
        checkpoint()?;
        if matches!(
            self.source_set_kind,
            SourceSetKind::ExternalProcessor | SourceSetKind::ExternalReport
        ) {
            #[cfg(test)]
            self.configuration_payload_reads
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            return self.external_inventory_payload(checkpoint).map(Arc::new);
        }
        self.artifacts
            .configuration_payload(&self.root, || self.parse_configuration_payload(checkpoint))
    }

    fn parse_configuration_payload(
        &self,
        checkpoint: &mut dyn FnMut() -> Result<(), ViewError>,
    ) -> Result<Value, ViewError> {
        #[cfg(test)]
        self.configuration_payload_reads
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        // Объявленный, но пустой набор — законное состояние, а не сбой. Без
        // этой ветки отсутствие выгрузки протекало наружу текстом `strerror`
        // («No such file or directory»), то есть файловый слой попадал в
        // логический ответ и ничего не говорил о том, что делать дальше.
        let Some(bytes) = self.read_optional_relative_whole(Path::new("Configuration.xml"))? else {
            let mut refusal = ViewError::new(
                RefusalCode::InvalidState,
                "source set is declared but its directory holds no configuration export;                  fill it with a scaffold before reading",
            );
            refusal.set_next(serde_json::json!({
                "tool": "unica.check",
                "args": {},
                "reason": "вердикт по набору и совет, чем его наполнить",
            }));
            return Err(refusal);
        };
        checkpoint()?;
        let text = std::str::from_utf8(&bytes).map_err(|_| {
            ViewError::detailed(
                RefusalDetail::SourceUnreadable,
                "Configuration.xml is not UTF-8",
            )
        })?;
        let parsed = parse_cf_info_xml(
            text,
            self.configuration_support(checkpoint)?,
            self.home_page()?,
            self.command_interface()?,
            self.panels()?,
        )
        .map_err(|error| ViewError::detailed(RefusalDetail::SourceUnreadable, error))?;
        let owner = prove_already_read_source_set_owner(
            Path::new("Configuration.xml"),
            &bytes,
            self.source_set_kind,
        )
        .map_err(|error| ViewError::detailed(RefusalDetail::SourceUnreadable, error.message))?;
        let mut payload = serde_json::to_value(parsed.data)
            .map_err(|error| ViewError::new(RefusalCode::ProviderUnavailable, error.to_string()))?
            .as_object()
            .cloned()
            .ok_or_else(|| {
                ViewError::detailed(
                    RefusalDetail::SourceUnreadable,
                    "configuration parser returned a non-object payload",
                )
            })?;
        payload.insert(
            "registeredObjects".to_string(),
            Value::Array(
                owner
                    .registrations()
                    .filter(|(kind, _)| NodeKind::parse(kind).is_ok_and(NodeKind::is_metadata_kind))
                    .map(|(kind, name)| json!({"kind": kind, "name": name}))
                    .collect(),
            ),
        );
        Ok(Value::Object(payload))
    }

    pub(crate) fn streamed_configuration_root(
        &self,
        checkpoint: &dyn Fn() -> Result<(), ViewError>,
    ) -> Result<StreamedConfigurationRoot, ViewError> {
        checkpoint()?;
        let file = match self
            .root
            .open_relative_regular_nofollow(Path::new("Configuration.xml"))
        {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                let mut refusal = ViewError::new(
                    RefusalCode::InvalidState,
                    "source set is declared but its directory holds no configuration export; fill it with a scaffold before reading",
                );
                refusal.set_next(serde_json::json!({
                    "tool": "unica.check",
                    "args": {},
                    "reason": "вердикт по набору и совет, чем его наполнить",
                }));
                return Err(refusal);
            }
            Err(error) => {
                return Err(ViewError::detailed(
                    RefusalDetail::SourceUnreadable,
                    error.to_string(),
                ));
            }
        };
        let support = serde_json::to_value(self.configuration_support(&mut || checkpoint())?)
            .map_err(|error| {
                ViewError::detailed(RefusalDetail::BackendBroken, error.to_string())
            })?;
        let home_page = serde_json::to_value(self.home_page()?).map_err(|error| {
            ViewError::detailed(RefusalDetail::BackendBroken, error.to_string())
        })?;
        let interface = serde_json::to_value(self.command_interface()?).map_err(|error| {
            ViewError::detailed(RefusalDetail::BackendBroken, error.to_string())
        })?;
        let panels = self.panels()?.unwrap_or(Value::Null);
        read_configuration_root(file, support, home_page, interface, panels, checkpoint)
    }

    /// Build root registration evidence from the complete retained XML once.
    /// The returned anonymous disk index can serve named reads in this source
    /// revision without retaining the inventory in process memory.
    pub(crate) fn streamed_configuration_registration_index(
        &self,
        checkpoint: &dyn Fn() -> Result<(), ViewError>,
    ) -> Result<Arc<RegistrationIndex>, ViewError> {
        self.artifacts.registrations(&self.root, checkpoint)
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

    fn external_inventory_payload(
        &self,
        checkpoint: &mut dyn FnMut() -> Result<(), ViewError>,
    ) -> Result<Value, ViewError> {
        let expected_kind = match self.source_set_kind {
            SourceSetKind::ExternalProcessor => "ExternalDataProcessor",
            SourceSetKind::ExternalReport => "ExternalReport",
            SourceSetKind::Configuration | SourceSetKind::Extension => {
                return Err(ViewError::detailed(
                    RefusalDetail::WrongSourceKind,
                    "external inventory requested for a configuration source set",
                ));
            }
        };
        let names = self
            .root
            .read_immediate_names_with_limit(None, || {
                checkpoint().map_err(|error| {
                    std::io::Error::new(std::io::ErrorKind::Interrupted, error.to_string())
                })
            })
            .map_err(|error| match checkpoint() {
                Err(checkpoint_error) => checkpoint_error,
                Ok(()) => ViewError::detailed(RefusalDetail::SourceUnreadable, error.to_string()),
            })?;
        let mut registered = std::collections::BTreeSet::new();
        let mut owner_path_mismatches = Vec::new();
        for name in names {
            checkpoint()?;
            let path = Path::new(&name);
            if path
                .extension()
                .and_then(|extension| extension.to_str())
                .is_none_or(|extension| !extension.eq_ignore_ascii_case("xml"))
            {
                continue;
            }
            let relative = PathBuf::from(&name);
            let bytes = self.read_relative_with_checkpoint(&relative, checkpoint)?;
            checkpoint()?;
            if path
                .file_name()
                .and_then(|value| value.to_str())
                .is_some_and(|value| value.eq_ignore_ascii_case("ConfigDumpInfo.xml"))
                && classify_already_read_config_dump_info_xml(&bytes)
                    == ConfigDumpInfoXmlKind::RuntimeSidecar
            {
                continue;
            }
            let evidence =
                prove_already_read_source_set_owner(&relative, &bytes, self.source_set_kind)
                    .map_err(|error| {
                        ViewError::detailed(RefusalDetail::SourceUnreadable, error.message)
                    })?;
            if evidence.artifact_kind() != expected_kind {
                return Err(ViewError::detailed(
                    RefusalDetail::SourceUnreadable,
                    format!(
                        "external descriptor `{}` has kind `{}`, expected `{expected_kind}`",
                        relative.portable(),
                        evidence.artifact_kind()
                    ),
                ));
            }
            let artifact_name = evidence.artifact_name().ok_or_else(|| {
                ViewError::detailed(
                    RefusalDetail::SourceUnreadable,
                    format!(
                        "external descriptor `{}` has no non-empty Properties/Name",
                        relative.portable()
                    ),
                )
            })?;
            if !registered.insert((expected_kind.to_string(), artifact_name.to_string())) {
                return Err(ViewError::detailed(
                    RefusalDetail::SourceUnreadable,
                    format!("external owner `{expected_kind}.{artifact_name}` is ambiguous"),
                ));
            }
            if path.file_stem().and_then(|value| value.to_str()) != Some(artifact_name) {
                owner_path_mismatches.push(format!(
                    "external descriptor `{}` belongs to `{artifact_name}`",
                    relative.portable()
                ));
            }
        }
        if registered.is_empty() {
            return Err(ViewError::detailed(
                RefusalDetail::SourceUnreadable,
                "external source set has no valid top-level owner descriptors",
            ));
        }
        if let Some(message) = owner_path_mismatches.into_iter().next() {
            return Err(ViewError::detailed(
                RefusalDetail::SourceUnreadable,
                message,
            ));
        }
        checkpoint()?;
        Ok(json!({
            "format": "PlatformXml",
            "name": self.source_set,
            "registeredObjects": registered
                .into_iter()
                .map(|(kind, name)| json!({"kind": kind, "name": name}))
                .collect::<Vec<_>>()
        }))
    }

    pub(crate) fn form_metadata_payload(
        &self,
        target: &MetadataAddress,
        checkpoint: &mut dyn FnMut() -> Result<(), ViewError>,
    ) -> Result<Value, ViewError> {
        let bytes = self.metadata_descriptor_with_checkpoint(target, checkpoint)?;
        let (mut properties, limits) =
            unica_metadata_designer::form_metadata::parse_form_metadata_properties(&bytes, target)
                .map_err(|error| ViewError::detailed(RefusalDetail::SourceUnreadable, error))?;
        if self.source_set_kind == SourceSetKind::Extension {
            let borrowing = unica_metadata_designer::xml_model::parse_meta_borrowing(&bytes)
                .map_err(|error| ViewError::detailed(RefusalDetail::SourceUnreadable, error))?;
            properties.extend(crate::infrastructure::v13_read_projection::borrowing_props(
                &borrowing,
            ));
        }
        checkpoint()?;
        Ok(
            json!({"kind":"Metadata", "name": properties.get("Name"), "properties":properties, "unreadable": limits.into_iter().map(|message| json!({"field":"formMetadata", "message":message})).collect::<Vec<_>>()}),
        )
    }

    pub(crate) fn form_payload(
        &self,
        target: &MetadataAddress,
        checkpoint: &mut dyn FnMut() -> Result<(), ViewError>,
    ) -> Result<Value, ViewError> {
        serde_json::to_value(self.form_data(target, checkpoint)?)
            .map_err(|error| ViewError::new(RefusalCode::ProviderUnavailable, error.to_string()))
    }

    pub(crate) fn form_data(
        &self,
        target: &MetadataAddress,
        checkpoint: &mut dyn FnMut() -> Result<(), ViewError>,
    ) -> Result<FormInfoData, ViewError> {
        let descriptor = self.metadata_descriptor(target)?;
        let relative = self.attached_resource_relative(target, "Form.xml")?;
        let bytes = self.read_relative_whole(&relative)?;
        let text = std::str::from_utf8(&bytes).map_err(|_| {
            ViewError::detailed(RefusalDetail::SourceUnreadable, "Form.xml is not UTF-8")
        })?;
        let parts = target.as_str().split('.').collect::<Vec<_>>();
        let form_name = parts.last().copied().unwrap_or("Form").to_string();
        let object_context = if parts.first().copied() == Some("CommonForm") {
            String::new()
        } else {
            parts[..parts.len().saturating_sub(2)].join(".")
        };
        // Keep one retained marker for the form and its top-level owner.
        let state = self.support_state(checkpoint)?;
        let own_rule = form_support_rule(state.as_deref(), &descriptor)?;
        let support = object_support_from_state(state.as_deref(), own_rule);
        let owner_support = if object_context.is_empty() {
            None
        } else {
            let owner =
                MetadataAddress::parse(PLATFORM_XML_8_3_27_FORMAT_2_20, &parts[..2].join("."))
                    .map_err(|error| {
                        ViewError::detailed(RefusalDetail::SourceUnreadable, error.to_string())
                    })?;
            let owner_descriptor = self.metadata_descriptor(&owner)?;
            let owner_rule = form_support_rule(state.as_deref(), &owner_descriptor)?;
            Some(object_support_from_state(state.as_deref(), owner_rule))
        };
        let direct_edit_safe = match (
            support.direct_edit_safe,
            owner_support
                .as_ref()
                .map_or(Some(true), |owner| owner.direct_edit_safe),
        ) {
            (Some(false), _) | (_, Some(false)) => Some(false),
            (Some(true), Some(true)) => Some(true),
            _ => None,
        };
        let support_rule_reason = if own_rule.is_some() {
            None
        } else {
            Some(match state.as_deref() {
                None => FormSupportRuleReason::MarkerAbsent,
                Some(state) if state.removed() => FormSupportRuleReason::ConfigurationRemoved,
                Some(_) => FormSupportRuleReason::RuleAbsent,
            })
        };
        let mut data = parse_form_info_xml(text, form_name, object_context, support)
            .map_err(|error| ViewError::detailed(RefusalDetail::SourceUnreadable, error))?;
        data.support_details = Some(FormSupportDetails {
            support_rule: own_rule,
            support_rule_reason,
            direct_edit_safe,
        });
        Ok(data)
    }

    pub(crate) fn form_validation_input(
        &self,
        target: &MetadataAddress,
        checkpoint: &mut dyn FnMut() -> Result<(), ViewError>,
    ) -> Result<FormValidationInput, ViewError> {
        let wrapper = self.metadata_descriptor(target)?;
        let wrapper_relative = self.metadata_descriptor_relative(target)?;
        let relative = self.attached_resource_relative(target, "Form.xml")?;
        let bytes = self.read_relative_with_checkpoint(&relative, checkpoint)?;
        let text = String::from_utf8(bytes).map_err(|_| {
            ViewError::detailed(
                RefusalDetail::SourceUnreadable,
                "Form.xml не является UTF-8",
            )
        })?;
        let artifact = self.root_path().join(&relative);
        let parts = target.as_str().split('.').collect::<Vec<_>>();
        let metadata_owner = if parts[0] == "CommonForm" {
            None
        } else {
            NodeKind::parse(parts[0]).ok()
        };
        let source_relative = if self.is_external_source_set() {
            let owner =
                MetadataAddress::parse(PLATFORM_XML_8_3_27_FORMAT_2_20, &parts[..2].join("."))
                    .map_err(|error| {
                        ViewError::detailed(RefusalDetail::SourceUnreadable, error.to_string())
                    })?;
            self.metadata_descriptor_relative(&owner)?
        } else {
            PathBuf::from("Configuration.xml")
        };
        let source_bytes = self.read_relative_with_checkpoint(&source_relative, checkpoint)?;
        let format_guard =
            crate::infrastructure::format_guard::evaluate_retained_template_format_guard(
                "form-validate",
                unica_source_designer::owner::MANAGED_FORM_ROOT,
                &artifact,
                &text,
                (self.root_path().join(wrapper_relative), wrapper),
                (
                    self.root_path().join(source_relative),
                    source_bytes,
                    self.source_set_kind,
                ),
            )
            .map_err(|error| {
                ViewError::detailed(RefusalDetail::SourceUnreadable, error.to_string())
            })?;
        // Отсутствующий или неразбираемый модуль не задаёт сигнатур. Ошибка
        // доступа и отмена сохраняются: ссылка не становится необязательным файлом.
        let module = if crate::infrastructure::form_composition::form_validation_requires_module(
            &text,
            metadata_owner,
        ) {
            let module_target = unica_source_designer::module_layout::form_module_target(target)
                .map_err(|error| ViewError::detailed(RefusalDetail::SourceUnreadable, error))?;
            let module_relative = self.module_relative(&module_target)?;
            self.read_optional_relative_with_checkpoint(&module_relative, checkpoint)?
                .and_then(|bytes| String::from_utf8(bytes).ok())
        } else {
            None
        };
        Ok(FormValidationInput {
            artifact,
            text,
            module,
            config_context: !self.is_external_source_set(),
            metadata_owner,
            format_guard,
        })
    }

    pub(crate) fn dcs_payload(
        &self,
        target: &MetadataAddress,
        checkpoint: &mut dyn FnMut() -> Result<(), ViewError>,
    ) -> Result<Value, ViewError> {
        let (_, text) = self.dcs_input(target)?;
        let data = crate::infrastructure::family_codecs::dcs_codec()
            .read(&text, self.object_support(target, checkpoint)?)
            .map_err(|error| ViewError::detailed(RefusalDetail::SourceUnreadable, error))?;
        serde_json::to_value(data)
            .map_err(|error| ViewError::new(RefusalCode::ProviderUnavailable, error.to_string()))
    }

    /// The retained source reader owns both DCS view and validation input.
    pub(crate) fn dcs_input(
        &self,
        target: &MetadataAddress,
    ) -> Result<(PathBuf, String), ViewError> {
        self.metadata_descriptor(target)?;
        let relative = self.attached_resource_relative(target, "Template.xml")?;
        let bytes = self.read_relative_whole(&relative)?;
        let text = String::from_utf8(bytes).map_err(|_| {
            ViewError::detailed(
                RefusalDetail::SourceUnreadable,
                "DCS Template.xml is not UTF-8",
            )
        })?;
        Ok((self.root_path().join(relative), text))
    }

    pub(crate) fn mxl_validation_input(
        &self,
        target: &MetadataAddress,
        checkpoint: &mut dyn FnMut() -> Result<(), ViewError>,
    ) -> Result<TemplateValidationInput, ViewError> {
        self.metadata_descriptor(target)?;
        let relative = self.attached_resource_relative(target, "Template.xml")?;
        let bytes = self.read_relative_with_checkpoint(&relative, checkpoint)?;
        let text = String::from_utf8(bytes).map_err(|_| {
            ViewError::detailed(
                RefusalDetail::SourceUnreadable,
                "MXL Template.xml is not UTF-8",
            )
        })?;
        let artifact = self.root_path().join(relative);
        let wrapper_relative = self.metadata_descriptor_relative(target)?;
        let wrapper = self.metadata_descriptor(target)?;
        let source_relative = if self.is_external_source_set() {
            let owner = target
                .as_str()
                .split('.')
                .take(2)
                .collect::<Vec<_>>()
                .join(".");
            let owner = MetadataAddress::parse(PLATFORM_XML_8_3_27_FORMAT_2_20, &owner).map_err(
                |error| ViewError::detailed(RefusalDetail::SourceUnreadable, error.to_string()),
            )?;
            self.metadata_descriptor_relative(&owner)?
        } else {
            PathBuf::from("Configuration.xml")
        };
        // The previous format guard read the complete owner without a size cap.
        // Retaining its authority must not add an 8 MiB rejection for large roots.
        let source_bytes = self.read_relative_with_checkpoint(&source_relative, checkpoint)?;
        let format_guard =
            crate::infrastructure::format_guard::evaluate_retained_template_format_guard(
                "mxl-validate",
                unica_source_designer::owner::MXL_ROOT,
                &artifact,
                &text,
                (self.root_path().join(wrapper_relative), wrapper),
                (
                    self.root_path().join(source_relative),
                    source_bytes,
                    self.source_set_kind,
                ),
            )
            .map_err(|error| {
                ViewError::detailed(RefusalDetail::SourceUnreadable, error.to_string())
            })?;
        Ok(TemplateValidationInput {
            artifact,
            text,
            format_guard,
        })
    }

    pub(crate) fn dcs_validation_input(
        &self,
        target: &MetadataAddress,
        checkpoint: &mut dyn FnMut() -> Result<(), ViewError>,
    ) -> Result<TemplateValidationInput, ViewError> {
        let (artifact, text) = self.dcs_input(target)?;
        let wrapper_relative = self.metadata_descriptor_relative(target)?;
        let wrapper = self.metadata_descriptor(target)?;
        let source_relative = if self.is_external_source_set() {
            let owner = target
                .as_str()
                .split('.')
                .take(2)
                .collect::<Vec<_>>()
                .join(".");
            let owner = MetadataAddress::parse(PLATFORM_XML_8_3_27_FORMAT_2_20, &owner).map_err(
                |error| ViewError::detailed(RefusalDetail::SourceUnreadable, error.to_string()),
            )?;
            self.metadata_descriptor_relative(&owner)?
        } else {
            PathBuf::from("Configuration.xml")
        };
        // The previous format guard read the complete owner without a size cap.
        // Retaining its authority must not add an 8 MiB rejection for large roots.
        let source_bytes = self.read_relative_with_checkpoint(&source_relative, checkpoint)?;
        let format_guard = crate::infrastructure::format_guard::evaluate_retained_dcs_format_guard(
            &artifact,
            &text,
            (self.root_path().join(wrapper_relative), wrapper),
            (
                self.root_path().join(source_relative),
                source_bytes,
                self.source_set_kind,
            ),
        )
        .map_err(|error| ViewError::detailed(RefusalDetail::SourceUnreadable, error.to_string()))?;
        Ok(TemplateValidationInput {
            artifact,
            text,
            format_guard,
        })
    }

    pub(crate) fn role_payload(
        &self,
        target: &MetadataAddress,
        checkpoint: &mut dyn FnMut() -> Result<(), ViewError>,
    ) -> Result<Value, ViewError> {
        let rights =
            self.read_relative_whole(&self.attached_resource_relative(target, "Rights.xml")?)?;
        let rights = std::str::from_utf8(&rights).map_err(|_| {
            ViewError::detailed(RefusalDetail::SourceUnreadable, "Rights.xml is not UTF-8")
        })?;
        let descriptor = self.metadata_descriptor(target)?;
        let descriptor = std::str::from_utf8(&descriptor).map_err(|_| {
            ViewError::detailed(
                RefusalDetail::SourceUnreadable,
                "role descriptor is not UTF-8",
            )
        })?;
        let fallback_name = target.as_str().rsplit('.').next().unwrap_or("Role");
        let data = parse_role_info_xml(
            rights,
            Some(descriptor),
            fallback_name,
            self.object_support(target, checkpoint)?,
        )
        .map_err(|error| ViewError::detailed(RefusalDetail::SourceUnreadable, error))?;
        serde_json::to_value(data)
            .map_err(|error| ViewError::new(RefusalCode::ProviderUnavailable, error.to_string()))
    }

    /// Заимствование объекта, чей сопутствующий файл расширение не несёт;
    /// см. [`inherited_companion`].
    pub(crate) fn inherited_resource(
        &self,
        target: &MetadataAddress,
        resource: &str,
    ) -> Result<Option<unica_metadata_designer::xml_model::MetaBorrowing>, ViewError> {
        inherited_companion(&self.root, self.source_set_kind, target, resource)
            .map_err(|error| ViewError::detailed(RefusalDetail::SourceUnreadable, error))
    }

    /// Локальное чтение объекта: разбор дескриптора и заимствование берутся из
    /// одних и тех же байтов.
    ///
    /// Дескриптор открывается один раз. Второе чтение того же файла стоило бы
    /// лишнего доступа и, что хуже, могло бы застать файл изменившимся между
    /// чтениями — тогда свойства объекта и его заимствование описывали бы
    /// разные состояния источника.
    pub(crate) fn metadata_local(
        &self,
        target: &MetadataAddress,
        checkpoint: &mut dyn FnMut() -> Result<(), ViewError>,
    ) -> Result<MetadataLocalRead, ViewError> {
        let descriptor = self.metadata_descriptor(target)?;
        let support = metadata_support_status(self.object_support_with_descriptor(
            target,
            Some(&descriptor),
            checkpoint,
        )?);
        // Заимствование отвечает только в наборе вида `extension`: в
        // конфигурации его не бывает, и разбирать дескриптор ради `own` у
        // каждого объекта незачем.
        let borrowing = if self.source_set_kind == SourceSetKind::Extension {
            Some(
                unica_metadata_designer::xml_model::parse_meta_borrowing(&descriptor)
                    .map_err(|error| ViewError::detailed(RefusalDetail::SourceUnreadable, error))?,
            )
        } else {
            None
        };
        let mut info = unica_metadata_designer::info::parse_typed_meta_local_info_for_source(
            &descriptor,
            target,
            support,
            self.source_set_kind,
        )
        .map_err(|failure| {
            let message = serde_json::to_string(&failure.diagnostics)
                .unwrap_or_else(|_| "metadata reader failed".to_string());
            ViewError::detailed(RefusalDetail::SourceUnreadable, message)
        })?;
        if info.kind == MetadataKind::ExchangePlan {
            let relative = Path::new("ExchangePlans")
                .join(&info.name)
                .join("Ext/Content.xml");
            let observation = match self
                .read_optional_relative_with_checkpoint(&relative, checkpoint)
            {
                Ok(Some(bytes)) => std::str::from_utf8(&bytes)
                    .map_err(|e| e.to_string())
                    .and_then(unica_metadata_designer::exchange_content::read_content)
                    .map(Some),
                Ok(None) => Ok(None),
                Err(error) if matches!(error.detail(), Some(RefusalDetail::SourceUnreadable)) => {
                    Err(error.to_string())
                }
                Err(error) => return Err(error),
            };
            let errors = match observation {
                Ok(Some(observation)) => {
                    info.relations.content = Some(Some(observation.items));
                    observation.errors
                }
                // Заимствованный план без своего `Ext/Content.xml` несёт состав
                // плана родителя: здесь он неизвестен, а не пуст (#1924).
                Ok(None) if borrowing.as_ref().is_some_and(|b| b.extends.is_some()) => {
                    info.relations.content = Some(None);
                    Vec::new()
                }
                // The platform omits Content.xml for an empty composition.
                Ok(None) => {
                    info.relations.content = Some(Some(Vec::new()));
                    Vec::new()
                }
                Err(error) => vec![error],
            };
            for error in errors {
                info.diagnostics.push(
                    unica_metadata::MetaDiagnostic::error(
                        unica_metadata::MetaDiagnosticCode::ValidationFailed,
                        error,
                    )
                    .with_metadata_path(target.clone())
                    .with_field("relations.content"),
                );
            }
        }
        Ok(MetadataLocalRead { info, borrowing })
    }

    /// Предопределённые элементы объекта.
    ///
    /// `None` — вид владельца этой коллекции не имеет, и ветви у него не будет.
    /// `Some` с нулём — файла предопределённых нет, и это доказанная пустота, а
    /// не незнание: платформа не пишет `Predefined.xml`, когда элементов нет.
    ///
    /// Разбор берётся у того же читателя, которым пользовался `meta.info`:
    /// второй разборщик того же файла расходился бы с ним молча.
    pub(crate) fn predefined_items(
        &self,
        target: &MetadataAddress,
        kind: MetadataKind,
        code_type: Option<&str>,
        checkpoint: &mut dyn FnMut() -> Result<(), ViewError>,
    ) -> Result<Option<MetaPredefinedItemsData>, ViewError> {
        if !unica_metadata::metadata_kind_collections(kind)
            .contains(&unica_metadata::MetaCollection::PredefinedItems)
        {
            return Ok(None);
        }
        let name = target.as_str().split('.').nth(1).unwrap_or_default();
        let relative = Path::new(unica_metadata_designer::layout::metadata_layout(kind).directory)
            .join(name)
            .join("Ext")
            .join("Predefined.xml");
        let Some(bytes) = self.read_optional_relative_with_checkpoint(&relative, checkpoint)?
        else {
            return Ok(Some(MetaPredefinedItemsData {
                total: 0,
                returned: 0,
                truncated: false,
                items: Vec::new(),
            }));
        };
        let code_type = match code_type.ok_or_else(|| {
            ViewError::detailed(
                RefusalDetail::SourceUnreadable,
                unica_metadata_designer::predefined::MISSING_PREDEFINED_CODE_TYPE,
            )
        })? {
            "String" => unica_metadata_designer::predefined::PredefinedCodeType::String,
            "Number" => unica_metadata_designer::predefined::PredefinedCodeType::Number,
            other => {
                return Err(ViewError::detailed(
                    RefusalDetail::SourceUnreadable,
                    format!("predefined owner CodeType `{other}` is unsupported"),
                ))
            }
        };
        let mut interrupted = None;
        let result =
            unica_metadata_designer::predefined::read_complete_predefined_items_for_code_type(
                &bytes,
                kind,
                code_type,
                &mut || {
                    checkpoint().map_err(|error| {
                        let message = error.to_string();
                        interrupted = Some(error);
                        message
                    })
                },
            );
        if let Some(error) = interrupted {
            return Err(error);
        }
        result
            .map(Some)
            .map_err(|message| ViewError::detailed(RefusalDetail::SourceUnreadable, message))
    }

    pub(crate) fn external_metadata_payload(
        &self,
        target: &MetadataAddress,
        checkpoint: &mut dyn FnMut() -> Result<(), ViewError>,
    ) -> Result<Option<Value>, ViewError> {
        if !self.is_external_source_set() {
            return Ok(None);
        }
        let parts = target.as_str().split('.').collect::<Vec<_>>();
        let [expected_kind, expected_name] = parts.as_slice() else {
            return Err(ViewError::new(
                RefusalCode::ProviderUnavailable,
                "external typed metadata target must name its root owner",
            ));
        };
        let descriptor = self.metadata_descriptor_with_checkpoint(target, checkpoint)?;
        let relative = self.metadata_descriptor_relative(target)?;
        let evidence =
            prove_already_read_source_set_owner(&relative, &descriptor, self.source_set_kind)
                .map_err(|error| {
                    ViewError::detailed(RefusalDetail::SourceUnreadable, error.message)
                })?;
        if evidence.artifact_kind() != *expected_kind
            || evidence.artifact_name() != Some(*expected_name)
        {
            return Err(ViewError::detailed(
                RefusalDetail::SourceUnreadable,
                format!(
                    "external descriptor identity does not match `{}`",
                    target.as_str()
                ),
            ));
        }
        identity_metadata_payload_from_evidence(target, &evidence).map(Some)
    }

    pub(crate) fn identity_metadata_payload(
        &self,
        target: &MetadataAddress,
    ) -> Result<Value, ViewError> {
        let descriptor = self.metadata_descriptor(target)?;
        let relative = self.metadata_descriptor_relative(target)?;
        let evidence = prove_already_read_metadata_owner(&relative, &descriptor)
            .map_err(|error| ViewError::detailed(RefusalDetail::SourceUnreadable, error.message))?;
        let mut payload = identity_metadata_payload_from_evidence(target, &evidence)?;
        if evidence.artifact_kind() == "StyleItem" {
            unica_metadata_designer::style_item_read::enrich_style_item(&descriptor, &mut payload)
                .map_err(|message| ViewError::detailed(RefusalDetail::SourceUnreadable, message))?;
        }
        Ok(payload)
    }

    pub(crate) fn metadata_owner_evidence(
        &self,
        target: &MetadataAddress,
        checkpoint: &mut dyn FnMut() -> Result<(), ViewError>,
    ) -> Result<PlatformXmlSourceSetOwnerEvidence, ViewError> {
        let external_root =
            self.is_external_source_set() && target.as_str().split('.').count() == 2;
        let descriptor = self.metadata_descriptor_with_checkpoint(target, checkpoint)?;
        let relative = self.metadata_descriptor_relative(target)?;
        if external_root {
            prove_already_read_source_set_owner(&relative, &descriptor, self.source_set_kind)
                .map_err(|error| {
                    ViewError::detailed(RefusalDetail::SourceUnreadable, error.message)
                })
        } else {
            prove_already_read_metadata_owner(&relative, &descriptor).map_err(|error| {
                ViewError::detailed(RefusalDetail::SourceUnreadable, error.message)
            })
        }
    }

    /// `with_content` carries the cell text. It is off for every structural
    /// read: the area count and its parameters are cheap, the text is not.
    pub(crate) fn mxl_payload(
        &self,
        target: &MetadataAddress,
        with_content: bool,
        checkpoint: &mut dyn FnMut() -> Result<(), ViewError>,
    ) -> Result<Value, ViewError> {
        self.metadata_descriptor(target)?;
        let bytes =
            self.read_relative_whole(&self.attached_resource_relative(target, "Template.xml")?)?;
        let name = target.as_str().rsplit('.').next().unwrap_or("Template");
        let data = parse_mxl_info_xml(
            &bytes,
            name,
            self.object_support(target, checkpoint)?,
            with_content,
        )
        .map_err(|error| ViewError::detailed(RefusalDetail::SourceUnreadable, error))?;
        serde_json::to_value(data)
            .map_err(|error| ViewError::new(RefusalCode::ProviderUnavailable, error.to_string()))
    }

    pub(crate) fn xdto_check_outcome(
        &self,
        target: &MetadataAddress,
        checkpoint: &mut dyn FnMut() -> Result<(), ViewError>,
    ) -> Result<crate::application::v13::check::NativeCheckOutcome, ViewError> {
        checkpoint()?;
        let descriptor = self.metadata_descriptor(target)?;
        let package = self.read_relative_with_checkpoint(
            &self.attached_resource_relative(target, "Package.bin")?,
            checkpoint,
        )?;
        let data = parse_xdto_info_bytes(&package, &descriptor, self.source_set(), target, None)
            .map_err(|error| {
                ViewError::detailed(RefusalDetail::SourceUnreadable, error.to_string())
            })?;
        checkpoint()?;
        Ok(crate::infrastructure::native_operations::xdto::xdto_check_outcome(data))
    }

    pub(crate) fn xdto_payload(
        &self,
        target: &MetadataAddress,
        type_name: Option<&str>,
    ) -> Result<Value, ViewError> {
        let descriptor = self.metadata_descriptor(target)?;
        let package =
            self.read_relative_whole(&self.attached_resource_relative(target, "Package.bin")?)?;
        let data =
            parse_xdto_info_bytes(&package, &descriptor, self.source_set(), target, type_name)
                .map_err(|error| {
                    let message = error.to_string();
                    match error {
                        XdtoInfoError::TypeNotFound(_) => {
                            ViewError::new(RefusalCode::NotFound, message)
                        }
                        XdtoInfoError::Source(_) => {
                            ViewError::detailed(RefusalDetail::SourceUnreadable, message)
                        }
                    }
                })?;
        serde_json::to_value(data)
            .map_err(|error| ViewError::new(RefusalCode::ProviderUnavailable, error.to_string()))
    }

    pub(crate) fn subsystem_payload(
        &self,
        target: &MetadataAddress,
        nested: &[String],
        checkpoint: &mut dyn FnMut() -> Result<(), ViewError>,
    ) -> Result<Value, ViewError> {
        let (descriptor_relative, descriptor_bytes) =
            self.registered_subsystem_descriptor(target, nested, checkpoint)?;
        let descriptor = std::str::from_utf8(&descriptor_bytes).map_err(|_| {
            ViewError::detailed(
                RefusalDetail::SourceUnreadable,
                "subsystem descriptor is not UTF-8",
            )
        })?;
        let ci_relative = if nested.is_empty() {
            self.attached_resource_relative(target, "CommandInterface.xml")?
        } else {
            nested_subsystem_resource(&descriptor_relative, "CommandInterface.xml")
        };
        let command_interface = self
            .read_optional_relative_whole(&ci_relative)?
            .map(|bytes| {
                let text = std::str::from_utf8(&bytes).map_err(|_| {
                    ViewError::detailed(
                        RefusalDetail::SourceUnreadable,
                        "CommandInterface.xml is not UTF-8",
                    )
                })?;
                parse_subsystem_command_interface_data(&ci_relative, text)
                    .map_err(|error| ViewError::detailed(RefusalDetail::SourceUnreadable, error))
            })
            .transpose()?;
        let (data, _) = unica_metadata_designer::subsystem::parse_subsystem_info_xml(
            if nested.is_empty() {
                Path::new(target.as_str())
            } else {
                descriptor_relative.as_path()
            },
            descriptor,
            command_interface.is_some(),
        )
        .map_err(|error| ViewError::detailed(RefusalDetail::SourceUnreadable, error))?;
        if let Some(expected) = nested.last() {
            if &data.name != expected {
                return Err(ViewError::detailed(
                    RefusalDetail::SourceUnreadable,
                    format!(
                        "subsystem descriptor `{}` names `{}` instead of `{expected}`",
                        descriptor_relative.portable(),
                        data.name
                    ),
                ));
            }
        }
        // Признак поддержки берётся из дескриптора читаемой подсистемы: у
        // вложенной он свой, а не корня.
        let support =
            self.object_support_with_descriptor(target, Some(&descriptor_bytes), checkpoint)?;
        let result = build_subsystem_info_result(data, None, command_interface, support);
        serde_json::to_value(result)
            .map_err(|error| ViewError::new(RefusalCode::ProviderUnavailable, error.to_string()))
    }

    /// Спускается от корневой подсистемы к вложенной только по регистрации:
    /// каждое следующее имя должно стоять в `ChildObjects` родителя. Файл,
    /// лежащий на месте незарегистрированной подсистемы, подсистемой не
    /// является. Читаются только дескрипторы цепочки, а не вся структура.
    fn registered_subsystem_descriptor(
        &self,
        target: &MetadataAddress,
        nested: &[String],
        checkpoint: &mut dyn FnMut() -> Result<(), ViewError>,
    ) -> Result<(PathBuf, Vec<u8>), ViewError> {
        let mut relative = self.metadata_descriptor_relative(target)?;
        let mut descriptor = self.metadata_descriptor(target)?;
        for (depth, name) in nested.iter().enumerate() {
            checkpoint()?;
            let text = std::str::from_utf8(&descriptor).map_err(|_| {
                ViewError::detailed(
                    RefusalDetail::SourceUnreadable,
                    "subsystem descriptor is not UTF-8",
                )
            })?;
            let (parent, _) = unica_metadata_designer::subsystem::parse_subsystem_info_xml(
                &relative, text, false,
            )
            .map_err(|error| ViewError::detailed(RefusalDetail::SourceUnreadable, error))?;
            // Промежуточный дескриптор обязан называть ту подсистему, по
            // регистрации которой к нему пришли; корень сверен владельцем.
            if let Some(expected) = depth.checked_sub(1).map(|index| &nested[index]) {
                if &parent.name != expected {
                    return Err(ViewError::detailed(
                        RefusalDetail::SourceUnreadable,
                        format!(
                            "subsystem descriptor `{}` names `{}` instead of `{expected}`",
                            relative.portable(),
                            parent.name
                        ),
                    ));
                }
            }
            if !parent.child_names.iter().any(|child| child == name) {
                return Err(ViewError::new(
                    RefusalCode::NotFound,
                    format!(
                        "subsystem `{name}` is not registered in `{}`",
                        relative.portable()
                    ),
                ));
            }
            relative = nested_subsystem_descriptor(&relative, name);
            descriptor = self
                .read_optional_relative_whole(&relative)?
                .ok_or_else(|| {
                    ViewError::new(
                        RefusalCode::NotFound,
                        format!("subsystem descriptor `{}` is absent", relative.portable()),
                    )
                })?;
        }
        Ok((relative, descriptor))
    }

    /// Check the selected descriptor and its direct dependencies under the same
    /// retained root used by view; legacy metadata paths cannot name this chain.
    pub(crate) fn validate_subsystem(
        &self,
        address: &QualifiedAddress,
        checkpoint: &mut dyn FnMut() -> Result<(), ViewError>,
    ) -> Result<crate::application::v13::check::NativeCheckOutcome, ViewError> {
        use crate::infrastructure::subsystem_composition::subsystem_validation_outcome;
        use unica_metadata::subsystem::SubsystemInfoFormatDocument;
        use unica_metadata_designer::subsystem::validate_subsystem_content;
        let route = crate::infrastructure::logical_tree::route_logical_address(
            address,
            unica_bsl::profile::PlatformProfile::v8_3_27(),
        )
        .map_err(|error| ViewError::new(RefusalCode::BadValue, error.to_string()))?;
        let target = route.reader_metadata_path().ok_or_else(|| {
            ViewError::new(RefusalCode::BadValue, "subsystem check has no descriptor")
        })?;
        let (relative, bytes) =
            self.registered_subsystem_descriptor(target, route.nested_subsystems(), checkpoint)?;
        let text = std::str::from_utf8(&bytes).map_err(|error| {
            ViewError::detailed(RefusalDetail::SourceUnreadable, error.to_string())
        })?;
        let (data, _) =
            unica_metadata_designer::subsystem::parse_subsystem_info_xml(&relative, text, false)
                .map_err(|error| ViewError::detailed(RefusalDetail::SourceUnreadable, error))?;
        let expected = address
            .segments()
            .last()
            .and_then(|segment| segment.name())
            .unwrap_or_default();
        if data.name != expected {
            return Err(ViewError::detailed(
                RefusalDetail::SourceUnreadable,
                format!(
                    "subsystem descriptor names `{}` instead of `{expected}`",
                    data.name
                ),
            ));
        }
        let ci_relative = nested_subsystem_resource(&relative, "CommandInterface.xml");
        let ci = self.read_optional_relative_with_checkpoint(&ci_relative, checkpoint)?;
        let mut documents = vec![SubsystemInfoFormatDocument {
            path: self.root_path().join(&relative),
            bytes: bytes.clone(),
        }];
        if let Some(bytes) = &ci {
            documents.push(SubsystemInfoFormatDocument {
                path: self.root_path().join(&ci_relative),
                bytes: bytes.clone(),
            });
        }
        let guard = crate::infrastructure::format_guard::evaluate_prepared_subsystem_validation_format_guard(&documents);
        let mut source_error = None;
        let result = validate_subsystem_content(
            text,
            relative,
            false,
            |path| {
                let result = (|| {
                    checkpoint()?;
                    match self.root.open_relative_regular_nofollow(path) {
                        Ok(_) => Ok(true),
                        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
                        Err(error) => Err(ViewError::detailed(
                            RefusalDetail::SourceUnreadable,
                            error.to_string(),
                        )),
                    }
                })();
                result.map_err(|error| {
                    let message = error.to_string();
                    source_error = Some(error);
                    message
                })
            },
            |_| {
                ci.as_ref()
                    .map(|bytes| {
                        String::from_utf8(bytes.clone()).map_err(|error| error.to_string())
                    })
                    .transpose()
            },
        );
        if let Some(error) = source_error {
            return Err(error);
        }
        checkpoint()?;
        Ok(
            crate::infrastructure::native_operations::v13_analysis::subsystem_validation_outcome(
                guard,
                subsystem_validation_outcome(result),
            ),
        )
    }

    pub(crate) fn nested_subsystem_export_path(
        &self,
        target: &MetadataAddress,
        nested: &[String],
        resource: Option<&str>,
    ) -> Result<String, ViewError> {
        let mut relative = self.metadata_descriptor_relative(target)?;
        for name in nested {
            relative = nested_subsystem_descriptor(&relative, name);
        }
        path_text(match resource {
            Some(resource) => nested_subsystem_resource(&relative, resource),
            None => relative,
        })
    }

    pub(crate) fn metadata_descriptor(
        &self,
        target: &MetadataAddress,
    ) -> Result<Vec<u8>, ViewError> {
        self.metadata_descriptor_with_checkpoint(target, &mut || Ok(()))
    }

    fn metadata_descriptor_with_checkpoint(
        &self,
        target: &MetadataAddress,
        checkpoint: &mut dyn FnMut() -> Result<(), ViewError>,
    ) -> Result<Vec<u8>, ViewError> {
        #[cfg(test)]
        {
            *self
                .metadata_descriptor_reads
                .lock()
                .expect("metadata descriptor read counter is not poisoned")
                .entry(target.as_str().to_string())
                .or_default() += 1;
        }
        let relative = self.metadata_descriptor_relative(target)?;
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
    ) -> Result<unica_metadata_designer::validation::CommonTemplateDescriptorFacts, ViewError> {
        let bytes = self.metadata_descriptor(target)?;
        parse_common_template_descriptor_from_bytes(&bytes, target)
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
        let owner = MetadataAddress::parse(PLATFORM_XML_8_3_27_FORMAT_2_20, &owner_text).map_err(
            |error| ViewError::detailed(RefusalDetail::SourceUnreadable, error.to_string()),
        )?;
        let Some((parsed_target, profile)) = parse_child_profile_from_bytes(
            &descriptor,
            &owner,
            unica_metadata_designer::validation::MetadataAdmission::Read,
        )
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

    pub(crate) fn module_source(
        &self,
        target: &MetadataAddress,
    ) -> Result<Option<String>, ViewError> {
        #[cfg(test)]
        {
            *self
                .module_source_reads
                .lock()
                .expect("module source read counter is not poisoned")
                .entry(target.as_str().to_string())
                .or_default() += 1;
        }
        let relative = self.module_relative(target)?;
        self.read_optional_relative_whole(&relative)?
            .map(|bytes| {
                String::from_utf8(bytes)
                    .map(|text| text.trim_start_matches('\u{feff}').to_string())
                    .map_err(|_| {
                        ViewError::detailed(
                            RefusalDetail::SourceUnreadable,
                            "BSL module is not UTF-8",
                        )
                    })
            })
            .transpose()
    }

    /// Only proves that the BSL file is present; the text is not read.
    pub(crate) fn module_source_exists(&self, target: &MetadataAddress) -> Result<bool, ViewError> {
        let relative = self.module_relative(target)?;
        self.relative_source_exists(&relative)
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

    /// Counts Body lines the way the Body snapshot numbers them, without
    /// copying or decoding the module. An absent file is `None`, not zero.
    pub(crate) fn module_line_count(
        &self,
        target: &MetadataAddress,
        checkpoint: impl FnMut() -> Result<(), ViewError>,
    ) -> Result<Option<usize>, ViewError> {
        const BOM: [u8; 3] = [0xef, 0xbb, 0xbf];
        let mut newlines = 0_usize;
        let mut bom_prefix = 0_u64;
        let mut in_bom_prefix = true;
        let mut last = None;
        let Some(len) = self.visit_module_utf8_chunks(target, checkpoint, |chunk| {
            for byte in chunk {
                if in_bom_prefix && *byte == BOM[(bom_prefix % 3) as usize] {
                    bom_prefix += 1;
                } else {
                    in_bom_prefix = false;
                }
            }
            newlines += chunk.iter().filter(|byte| **byte == b'\n').count();
            last = chunk.last().copied().or(last);
            Ok(())
        })?
        else {
            return Ok(None);
        };
        // A file of leading U+FEFF only is an empty Body, as in the snapshot.
        if len == bom_prefix {
            return Ok(Some(0));
        }
        Ok(Some(newlines + usize::from(last != Some(b'\n'))))
    }

    /// Текст модуля для `Body`: без начальных U+FEFF и не длиннее
    /// `MAX_SOURCE_TEXT_BYTES`. Больший файл дочитывается без накопления
    /// ради проверки UTF-8 и полного размера. Отсутствующий файл — пустой
    /// текст, как у прежнего `Body`.
    pub(crate) fn module_text_within_limit(
        &self,
        target: &MetadataAddress,
        checkpoint: impl FnMut() -> Result<(), ViewError>,
    ) -> Result<ModuleText, ViewError> {
        let mut text = ModuleTextAccumulator::default();
        let len = self.visit_module_utf8_chunks(target, checkpoint, |chunk| {
            text.push(chunk);
            Ok(())
        })?;
        if len.is_none() {
            return Ok(ModuleText::Text(String::new()));
        }
        text.finish().map_err(|_| {
            ViewError::detailed(RefusalDetail::SourceUnreadable, "BSL module is not UTF-8")
        })
    }

    /// Retains only the requested complete file lines, while validating the
    /// entire stream and preserving retained-source identity/cancellation.
    pub(crate) fn module_text_range(
        &self,
        target: &MetadataAddress,
        range: unica_address::address::BodyLineRange,
        checkpoint: impl FnMut() -> Result<(), ViewError>,
    ) -> Result<ModuleText, ViewError> {
        let mut text = ModuleTextAccumulator::with_range(range);
        let exists = self.visit_module_utf8_chunks(target, checkpoint, |chunk| {
            text.push(chunk);
            Ok(())
        })?;
        if exists.is_none() {
            return Err(ViewError::new(
                RefusalCode::NotFound,
                "the BSL module source file does not exist",
            ));
        }
        if range.to > text.line_count() {
            return Err(ViewError::new(
                RefusalCode::BadValue,
                "Body line selector exceeds the file's last line",
            ));
        }
        text.finish().map_err(|_| {
            ViewError::detailed(RefusalDetail::SourceUnreadable, "BSL module is not UTF-8")
        })
    }

    /// Streams the module only after one shared UTF-8 check, so every Body
    /// reader sees the same text and numbers the same lines. An absent file
    /// is `None`; the length is in bytes.
    fn visit_module_utf8_chunks(
        &self,
        target: &MetadataAddress,
        mut checkpoint: impl FnMut() -> Result<(), ViewError>,
        mut visit: impl FnMut(&[u8]) -> Result<(), ViewError>,
    ) -> Result<Option<u64>, ViewError> {
        let relative = self.module_relative(target)?;
        if let Some(buffer) = self.open_buffer(&relative) {
            checkpoint()?;
            visit(buffer.text.as_bytes())?;
            return Ok(Some(buffer.text.len() as u64));
        }
        let mut carry = Vec::<u8>::new();
        let interrupted = std::cell::RefCell::new(None);
        let result = self.root.visit_relative_regular_chunks(
            &relative,
            || {
                checkpoint().map_err(|error| {
                    *interrupted.borrow_mut() = Some(error);
                    std::io::Error::other("BSL module read interrupted")
                })
            },
            |chunk| {
                let mut candidate = Vec::with_capacity(carry.len() + chunk.len());
                candidate.extend_from_slice(&carry);
                candidate.extend_from_slice(chunk);
                match std::str::from_utf8(&candidate) {
                    Ok(_) => carry.clear(),
                    Err(error) if error.error_len().is_none() => {
                        carry = candidate[error.valid_up_to()..].to_vec();
                    }
                    Err(_) => {
                        return Err(std::io::Error::new(
                            std::io::ErrorKind::InvalidData,
                            "BSL module is not UTF-8",
                        ))
                    }
                }
                visit(chunk).map_err(|error| {
                    *interrupted.borrow_mut() = Some(error);
                    std::io::Error::other("BSL module chunk was refused")
                })
            },
        );
        let len = match result {
            Ok(len) => len,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => {
                return Err(interrupted.into_inner().unwrap_or_else(|| {
                    ViewError::detailed(RefusalDetail::SourceUnreadable, error.to_string())
                }))
            }
        };
        if !carry.is_empty() {
            return Err(ViewError::detailed(
                RefusalDetail::SourceUnreadable,
                "BSL module is not UTF-8",
            ));
        }
        Ok(Some(len))
    }

    #[cfg(test)]
    pub(crate) fn module_source_read_count(&self, target: &str) -> usize {
        self.module_source_reads
            .lock()
            .expect("module source read counter is not poisoned")
            .get(target)
            .copied()
            .unwrap_or_default()
    }

    #[cfg(test)]
    pub(crate) fn metadata_descriptor_read_count(&self, target: &str) -> usize {
        self.metadata_descriptor_reads
            .lock()
            .expect("metadata descriptor read counter is not poisoned")
            .get(target)
            .copied()
            .unwrap_or_default()
    }

    #[cfg(test)]
    pub(crate) fn configuration_payload_read_count(&self) -> usize {
        self.configuration_payload_reads
            .load(std::sync::atomic::Ordering::Relaxed)
    }

    #[cfg(test)]
    pub(crate) fn support_state_read_count(&self) -> usize {
        self.support_state_reads
            .load(std::sync::atomic::Ordering::Relaxed)
    }

    pub(crate) fn object_support(
        &self,
        target: &MetadataAddress,
        checkpoint: &mut dyn FnMut() -> Result<(), ViewError>,
    ) -> Result<ObjectSupportData, ViewError> {
        self.object_support_with_descriptor(target, None, checkpoint)
    }

    /// A caller that already holds the descriptor bytes passes them in so the
    /// support lookup does not read the same file a second time.
    fn object_support_with_descriptor(
        &self,
        target: &MetadataAddress,
        descriptor: Option<&[u8]>,
        checkpoint: &mut dyn FnMut() -> Result<(), ViewError>,
    ) -> Result<ObjectSupportData, ViewError> {
        let Some(state) = self.support_state(checkpoint)? else {
            return Ok(unsupported_object());
        };
        // Preserve the established early global answer for other readers:
        // only the form-specific own-rule projection requires UUID evidence
        // even when the global state already determines effective support.
        if state.removed() || !state.global_editing_enabled() {
            return Ok(object_support_from_state(Some(&state), None));
        }
        let owned;
        let descriptor = match descriptor {
            Some(bytes) => bytes,
            None => {
                owned = self.metadata_descriptor(target)?;
                owned.as_slice()
            }
        };
        let uuid = support_root_uuid_from_bytes(descriptor).ok_or_else(|| {
            ViewError::detailed(
                RefusalDetail::SourceUnreadable,
                "metadata descriptor has no support-state UUID evidence",
            )
        })?;
        Ok(object_support_from_state(
            Some(&state),
            object_support_rule(state.object_rule(&uuid)),
        ))
    }

    fn configuration_support(
        &self,
        checkpoint: &mut dyn FnMut() -> Result<(), ViewError>,
    ) -> Result<ConfigurationSupportData, ViewError> {
        let Some(state) = self.support_state(checkpoint)? else {
            return Ok(ConfigurationSupportData {
                state: if self.source_set_kind == SourceSetKind::Extension {
                    ConfigurationSupportState::Extension
                } else {
                    ConfigurationSupportState::NotSupported
                },
                editing_enabled: None,
                objects: None,
            });
        };
        if state.removed() {
            return Ok(ConfigurationSupportData {
                state: ConfigurationSupportState::Removed,
                editing_enabled: None,
                objects: None,
            });
        }
        let counts = state.counts();
        Ok(ConfigurationSupportData {
            state: ConfigurationSupportState::Supported,
            editing_enabled: Some(state.global_editing_enabled()),
            objects: Some(SupportCounts {
                locked: counts[0] as u64,
                editable: counts[1] as u64,
                removed: counts[2] as u64,
            }),
        })
    }

    fn support_state(
        &self,
        checkpoint: &mut dyn FnMut() -> Result<(), ViewError>,
    ) -> Result<Option<Arc<SupportState>>, ViewError> {
        checkpoint()?;
        let mut memo = self.support_state.lock().map_err(|_| {
            ViewError::detailed(
                RefusalDetail::CachePoisoned,
                "support-state cache is poisoned",
            )
        })?;
        // The memo answers only for the marker stamp it was read under: a
        // marker replaced during this operation is read again, so a payload
        // built later never combines a new stamp with an old state.
        if let Some((stamp, state)) = memo.as_ref() {
            let current = self
                .root
                .relative_file_stamp(Path::new("Ext/ParentConfigurations.bin"))
                .map_err(|error| {
                    ViewError::detailed(RefusalDetail::SourceUnreadable, error.to_string())
                })?;
            if current == *stamp {
                checkpoint()?;
                return Ok(state.clone());
            }
        }
        let (stamp, state) =
            self.artifacts
                .support_state(&self.root, checkpoint, |file, checkpoint| {
                    #[cfg(test)]
                    self.support_state_reads
                        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    parse_support_state(file, checkpoint)
                })?;
        checkpoint()?;
        *memo = Some((stamp, state.clone()));
        Ok(state)
    }

    /// Командный интерфейс конфигурации читается так же, как домашняя
    /// страница: отдельным документом рядом с `Configuration.xml`, и ложится
    /// свойством корня. Отсутствие документа — законное состояние, а не отказ.
    fn command_interface(&self) -> Result<Option<CfInterfaceData>, ViewError> {
        let Some(bytes) =
            self.read_optional_relative_whole(Path::new("Ext/CommandInterface.xml"))?
        else {
            return Ok(None);
        };
        let text = std::str::from_utf8(&bytes).map_err(|_| {
            ViewError::detailed(
                RefusalDetail::SourceUnreadable,
                "CommandInterface.xml is not UTF-8",
            )
        })?;
        parse_cf_command_interface_xml(text)
            .map(Some)
            .map_err(|error| ViewError::detailed(RefusalDetail::SourceUnreadable, error))
    }

    /// Раскладка панелей корня в форме, которую принимает `panels.set`.
    fn panels(&self) -> Result<Option<Value>, ViewError> {
        let Some(bytes) = self.read_optional_relative_whole(Path::new(
            unica_metadata_designer::client_interface::CLIENT_INTERFACE_RELATIVE,
        ))?
        else {
            return Ok(None);
        };
        let text = std::str::from_utf8(&bytes).map_err(|_| {
            ViewError::detailed(
                RefusalDetail::SourceUnreadable,
                "ClientApplicationInterface.xml is not UTF-8",
            )
        })?;
        unica_metadata_designer::client_interface::parse_panels(text)
            .map(|stored| Some(stored.to_json()))
            .map_err(|error| ViewError::detailed(RefusalDetail::SourceUnreadable, error))
    }

    fn home_page(&self) -> Result<Option<CfHomePageData>, ViewError> {
        let Some(bytes) =
            self.read_optional_relative_whole(Path::new("Ext/HomePageWorkArea.xml"))?
        else {
            return Ok(None);
        };
        let text = std::str::from_utf8(&bytes).map_err(|_| {
            ViewError::detailed(
                RefusalDetail::SourceUnreadable,
                "HomePageWorkArea.xml is not UTF-8",
            )
        })?;
        let layout = parse_cf_home_page_xml_strict(text)
            .map_err(|error| ViewError::detailed(RefusalDetail::SourceUnreadable, error))?;
        Ok(Some(CfHomePageData {
            template: layout.template,
            left: cf_home_page_item_data(&layout.left),
            right: cf_home_page_item_data(&layout.right),
        }))
    }

    fn metadata_descriptor_relative(&self, target: &MetadataAddress) -> Result<PathBuf, ViewError> {
        shared_metadata_descriptor_relative(target, self.source_set_kind)
            .map_err(|error| ViewError::detailed(RefusalDetail::SourceUnreadable, error))
    }

    fn attached_resource_relative(
        &self,
        target: &MetadataAddress,
        resource: &str,
    ) -> Result<PathBuf, ViewError> {
        shared_attached_resource_relative(target, resource, self.source_set_kind)
            .map_err(|error| ViewError::new(RefusalCode::ProviderUnavailable, error))
    }

    fn module_relative(&self, target: &MetadataAddress) -> Result<PathBuf, ViewError> {
        shared_module_relative(target, self.source_set_kind)
            .map_err(|error| ViewError::new(RefusalCode::ProviderUnavailable, error))
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
    MetadataAddress::parse(PLATFORM_XML_8_3_27_FORMAT_2_20, &path).ok()
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
) -> Result<Option<unica_metadata_designer::xml_model::MetaBorrowing>, String> {
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
    if !unica_metadata_designer::xml_model::descriptor_extends_parent(&descriptor) {
        return Ok(None);
    }
    let parts = target.as_str().split('.').collect::<Vec<_>>();
    if parts.len() > 2 {
        let owner = MetadataAddress::parse(PLATFORM_XML_8_3_27_FORMAT_2_20, &parts[..2].join("."))
            .map_err(|error| error.to_string())?;
        if !unica_metadata_designer::xml_model::descriptor_extends_parent(&descriptor_of(&owner)?) {
            return Ok(None);
        }
    }
    unica_metadata_designer::xml_model::parse_meta_borrowing(&descriptor).map(Some)
}

/// `Subsystems/A.xml` → `Subsystems/A/Subsystems/B.xml`.
fn nested_subsystem_descriptor(parent: &Path, name: &str) -> PathBuf {
    parent
        .with_extension("")
        .join("Subsystems")
        .join(format!("{name}.xml"))
}

/// `Subsystems/A/Subsystems/B.xml` → `Subsystems/A/Subsystems/B/Ext/<resource>`.
fn nested_subsystem_resource(descriptor: &Path, resource: &str) -> PathBuf {
    descriptor.with_extension("").join("Ext").join(resource)
}

fn identity_metadata_payload_from_evidence(
    target: &MetadataAddress,
    evidence: &PlatformXmlSourceSetOwnerEvidence,
) -> Result<Value, ViewError> {
    let parts = target.as_str().split('.').collect::<Vec<_>>();
    let [expected_kind, expected_name] = parts.as_slice() else {
        return Err(ViewError::new(
            RefusalCode::ProviderUnavailable,
            "identity metadata target must name one owner",
        ));
    };
    if evidence.artifact_kind() != *expected_kind
        || evidence.artifact_name() != Some(*expected_name)
    {
        return Err(ViewError::detailed(
            RefusalDetail::SourceUnreadable,
            format!(
                "metadata descriptor identity does not match `{}`",
                target.as_str()
            ),
        ));
    }
    let mut forms = Vec::new();
    let mut templates = Vec::new();
    let mut commands = Vec::new();
    for (kind, name) in evidence.registrations() {
        let value = json!({"name": name});
        match kind {
            "Form" => forms.push(value),
            "Template" => templates.push(value),
            "Command" => commands.push(value),
            _ => {}
        }
    }
    Ok(json!({
        "name": expected_name,
        "kind": expected_kind,
        "support": {"state": "not_supported"},
        "properties": {},
        "declarations": {},
        "relations": {},
        "collections": {
            "attributes": [],
            "tabularSections": [],
            "dimensions": [],
            "resources": [],
            "enumValues": [],
            "columns": [],
            "forms": forms,
            "templates": templates,
            "commands": commands
        }
    }))
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

/// Missing marker is proven absence; with a marker the descriptor UUID is
/// required even when a global restriction would hide the own rule.
fn form_support_rule(
    state: Option<&SupportState>,
    descriptor: &[u8],
) -> Result<Option<ObjectSupportState>, ViewError> {
    let Some(state) = state else { return Ok(None) };
    let uuid = support_root_uuid_from_bytes(descriptor).ok_or_else(|| {
        ViewError::detailed(
            RefusalDetail::SourceUnreadable,
            "metadata descriptor has no support-state UUID evidence",
        )
    })?;
    Ok(object_support_rule(state.object_rule(&uuid)))
}

fn object_support_rule(rule: Option<u8>) -> Option<ObjectSupportState> {
    match rule {
        Some(0) => Some(ObjectSupportState::Locked),
        Some(1) => Some(ObjectSupportState::EditableWithSupport),
        Some(2) => Some(ObjectSupportState::RemovedFromSupport),
        _ => None,
    }
}

fn object_support_from_state(
    state: Option<&SupportState>,
    own_rule: Option<ObjectSupportState>,
) -> ObjectSupportData {
    let Some(state) = state else {
        return unsupported_object();
    };
    let state = if state.removed() {
        ObjectSupportState::RemovedFromSupport
    } else if !state.global_editing_enabled() {
        ObjectSupportState::ConfigurationReadOnly
    } else {
        own_rule.unwrap_or(ObjectSupportState::NotSupported)
    };
    let direct_edit_safe = match state {
        ObjectSupportState::Locked | ObjectSupportState::ConfigurationReadOnly => Some(false),
        ObjectSupportState::EditableWithSupport | ObjectSupportState::RemovedFromSupport => {
            Some(true)
        }
        ObjectSupportState::NotSupported => None,
    };
    ObjectSupportData {
        state,
        direct_edit_safe,
    }
}

fn unsupported_object() -> ObjectSupportData {
    ObjectSupportData {
        state: ObjectSupportState::NotSupported,
        direct_edit_safe: None,
    }
}

fn metadata_support_status(support: ObjectSupportData) -> MetaSupportStatus {
    match support.state {
        ObjectSupportState::Locked | ObjectSupportState::ConfigurationReadOnly => {
            MetaSupportStatus::Locked
        }
        ObjectSupportState::RemovedFromSupport => MetaSupportStatus::Unsupported,
        ObjectSupportState::EditableWithSupport | ObjectSupportState::NotSupported => {
            MetaSupportStatus::Supported
        }
    }
}

/// Streams `Ext/ParentConfigurations.bin` through the support-state parser in
/// bounded chunks, observing the caller's checkpoint before each chunk.
fn parse_support_state(
    mut file: impl std::io::Read,
    checkpoint: &mut dyn FnMut() -> Result<(), ViewError>,
) -> Result<SupportState, ViewError> {
    let mut parser = SupportStateStreamParser::default();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        checkpoint()?;
        let count = match file.read(&mut buffer) {
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            result => result.map_err(|error| {
                ViewError::detailed(RefusalDetail::SourceUnreadable, error.to_string())
            })?,
        };
        if count == 0 {
            break;
        }
        parser.feed(&buffer[..count]).map_err(|error| {
            ViewError::detailed(RefusalDetail::SourceUnreadable, error.to_string())
        })?;
    }
    parser
        .finish()
        .map_err(|error| ViewError::detailed(RefusalDetail::SourceUnreadable, error.to_string()))
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
