use crate::application::ports::{MetaLocalInfo, MetadataChildProfile};
use crate::application::v13::view::ViewError;
use crate::domain::address::NodeKind;
use crate::domain::cancellation::CancellationToken;
use crate::domain::code_intelligence::ProviderDeadline;
use crate::domain::metadata::MetaSupportStatus;
use crate::domain::project_sources::{
    classify_already_read_config_dump_info_xml, ConfigDumpInfoXmlKind, SourceSetKind,
};
use crate::domain::source_target::{MetadataAddress, PLATFORM_XML_8_3_27_FORMAT_2_20};
use crate::domain::support_state::{
    ConfigurationSupportData, ConfigurationSupportState, ObjectSupportData, ObjectSupportState,
    SupportCounts,
};
use crate::infrastructure::logical_event_source::{
    attached_resource_relative as shared_attached_resource_relative,
    metadata_descriptor_relative as shared_metadata_descriptor_relative,
    module_relative as shared_module_relative,
};
use crate::infrastructure::native_operations::cf::{
    cf_home_page_item_data, parse_cf_home_page_xml_strict, parse_cf_info_xml, CfHomePageData,
};
use crate::infrastructure::native_operations::common::{
    parse_subsystem_info_xml, parse_support_state_strict_bytes, support_root_uuid_from_bytes,
};
use crate::infrastructure::native_operations::dcs::parse_dcs_info_xml;
use crate::infrastructure::native_operations::form::parse_form_info_xml;
use crate::infrastructure::native_operations::form::FormInfoData;
use crate::infrastructure::native_operations::meta::{
    parse_child_profile_from_bytes, parse_typed_meta_local_info,
};
use crate::infrastructure::native_operations::mxl::parse_mxl_info_xml;
use crate::infrastructure::native_operations::role::parse_role_info_xml;
use crate::infrastructure::native_operations::subsystem::{
    build_subsystem_info_result, parse_subsystem_command_interface_data,
};
use crate::infrastructure::native_operations::xdto::parse_xdto_info_bytes;
use crate::infrastructure::platform::filesystem::RetainedDirectoryCapability;
use crate::infrastructure::platform_xml_owner::{
    prove_already_read_metadata_owner, prove_already_read_source_set_owner,
    PlatformXmlSourceSetOwnerEvidence,
};
use crate::infrastructure::source_revision::{RetainedRevisionLease, SourceRevisionService};
use serde_json::{json, Value};
#[cfg(test)]
use std::cell::RefCell;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;

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

const MAX_CONFIGURATION_BYTES: usize = 8 * 1024 * 1024;
const MAX_EXTERNAL_OWNER_ENTRIES: usize = 256;
const MAX_EXTERNAL_INVENTORY_BYTES: usize = 32 * 1024 * 1024;

/// One actor-issued read authority for one admitted source set. Hidden v0.13
/// reads are descriptor-relative to this retained directory and revisions come
/// from the paired actor-owned service.
pub(crate) struct ProviderReadAuthority {
    source_set: String,
    source_set_identity: String,
    source_set_kind: SourceSetKind,
    root: Arc<RetainedDirectoryCapability>,
    revisions: ProviderRevisionAuthority,
    #[cfg(test)]
    module_source_reads: std::sync::Mutex<std::collections::BTreeMap<String, usize>>,
    #[cfg(test)]
    metadata_descriptor_reads: std::sync::Mutex<std::collections::BTreeMap<String, usize>>,
    #[cfg(test)]
    configuration_payload_reads: std::sync::atomic::AtomicUsize,
}

enum ProviderRevisionAuthority {
    Live(Arc<SourceRevisionService>),
    Operation(RetainedRevisionLease),
}

impl ProviderReadAuthority {
    pub(crate) fn new(
        source_set: impl Into<String>,
        source_set_identity: impl Into<String>,
        source_set_kind: SourceSetKind,
        root: Arc<RetainedDirectoryCapability>,
        revisions: Arc<SourceRevisionService>,
    ) -> Self {
        Self {
            source_set: source_set.into(),
            source_set_identity: source_set_identity.into(),
            source_set_kind,
            root,
            revisions: ProviderRevisionAuthority::Live(revisions),
            #[cfg(test)]
            module_source_reads: std::sync::Mutex::new(std::collections::BTreeMap::new()),
            #[cfg(test)]
            metadata_descriptor_reads: std::sync::Mutex::new(std::collections::BTreeMap::new()),
            #[cfg(test)]
            configuration_payload_reads: std::sync::atomic::AtomicUsize::new(0),
        }
    }

    pub(crate) fn new_with_revision_lease(
        source_set: impl Into<String>,
        source_set_identity: impl Into<String>,
        source_set_kind: SourceSetKind,
        root: Arc<RetainedDirectoryCapability>,
        revision: RetainedRevisionLease,
    ) -> Self {
        Self {
            source_set: source_set.into(),
            source_set_identity: source_set_identity.into(),
            source_set_kind,
            root,
            revisions: ProviderRevisionAuthority::Operation(revision),
            #[cfg(test)]
            module_source_reads: std::sync::Mutex::new(std::collections::BTreeMap::new()),
            #[cfg(test)]
            metadata_descriptor_reads: std::sync::Mutex::new(std::collections::BTreeMap::new()),
            #[cfg(test)]
            configuration_payload_reads: std::sync::atomic::AtomicUsize::new(0),
        }
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
        self.root
            .validate_named_identity()
            .map_err(|error| ViewError::new("provider_unavailable", error.to_string()))?;
        #[cfg(test)]
        REVIEW_AFTER_REVISION_IDENTITY.with(|slot| {
            if let Some(hook) = slot.borrow_mut().as_mut() {
                hook();
            }
        });
        match &self.revisions {
            ProviderRevisionAuthority::Live(revisions) => revisions
                .snapshot_retained(&self.root, deadline, cancellation)
                .map(|revision| {
                    format!(
                        "{}:{}:{}",
                        revision.algorithm, revision.generation, revision.digest
                    )
                })
                .map_err(|error| ViewError::new("provider_unavailable", error)),
            ProviderRevisionAuthority::Operation(revision) => {
                if cancellation.is_cancelled() {
                    Err(ViewError::new("cancelled", "logical read was cancelled"))
                } else if deadline.remaining().is_zero() {
                    Err(ViewError::new(
                        "provider_deadline",
                        "logical read operation deadline elapsed",
                    ))
                } else {
                    Ok(revision.revision_identity())
                }
            }
        }
    }

    pub(crate) fn read_relative(
        &self,
        relative: &Path,
        max_bytes: usize,
    ) -> Result<Vec<u8>, ViewError> {
        self.root
            .read_relative_regular_bounded(relative, max_bytes)
            .map_err(|error| ViewError::new("provider_unavailable", error.to_string()))
    }

    pub(crate) fn read_optional_relative(
        &self,
        relative: &Path,
        max_bytes: usize,
    ) -> Result<Option<Vec<u8>>, ViewError> {
        match self.root.read_relative_regular_bounded(relative, max_bytes) {
            Ok(bytes) => Ok(Some(bytes)),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(ViewError::new("provider_unavailable", error.to_string())),
        }
    }

    pub(crate) fn configuration_payload_with_checkpoint(
        &self,
        checkpoint: &mut dyn FnMut() -> Result<(), ViewError>,
    ) -> Result<Value, ViewError> {
        #[cfg(test)]
        self.configuration_payload_reads
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        checkpoint()?;
        if matches!(
            self.source_set_kind,
            SourceSetKind::ExternalProcessor | SourceSetKind::ExternalReport
        ) {
            return self.external_inventory_payload(checkpoint);
        }
        let bytes = self.read_relative(Path::new("Configuration.xml"), MAX_CONFIGURATION_BYTES)?;
        checkpoint()?;
        let text = std::str::from_utf8(&bytes).map_err(|_| {
            ViewError::new("provider_unavailable", "Configuration.xml is not UTF-8")
        })?;
        let parsed = parse_cf_info_xml(text, self.configuration_support()?, self.home_page()?)
            .map_err(|error| ViewError::new("provider_unavailable", error))?;
        let owner = prove_already_read_source_set_owner(
            Path::new("Configuration.xml"),
            &bytes,
            self.source_set_kind,
        )
        .map_err(|error| ViewError::new("provider_unavailable", error.message))?;
        let mut payload = serde_json::to_value(parsed.data)
            .map_err(|error| ViewError::new("provider_unavailable", error.to_string()))?
            .as_object()
            .cloned()
            .ok_or_else(|| {
                ViewError::new(
                    "provider_unavailable",
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

    fn external_inventory_payload(
        &self,
        checkpoint: &mut dyn FnMut() -> Result<(), ViewError>,
    ) -> Result<Value, ViewError> {
        let expected_kind = match self.source_set_kind {
            SourceSetKind::ExternalProcessor => "ExternalDataProcessor",
            SourceSetKind::ExternalReport => "ExternalReport",
            SourceSetKind::Configuration | SourceSetKind::Extension => {
                return Err(ViewError::new(
                    "provider_unavailable",
                    "external inventory requested for a configuration source set",
                ));
            }
        };
        let names = self
            .root
            .read_immediate_names_bounded(MAX_EXTERNAL_OWNER_ENTRIES, || {
                checkpoint().map_err(|error| {
                    std::io::Error::new(std::io::ErrorKind::Interrupted, error.to_string())
                })
            })
            .map_err(|error| match checkpoint() {
                Err(checkpoint_error) => checkpoint_error,
                Ok(()) => ViewError::new("provider_unavailable", error.to_string()),
            })?;
        let mut registered = std::collections::BTreeSet::new();
        let mut owner_path_mismatches = Vec::new();
        let mut retained_bytes = 0_usize;
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
            let remaining = MAX_EXTERNAL_INVENTORY_BYTES.saturating_sub(retained_bytes);
            if remaining == 0 {
                return Err(ViewError::new(
                    "provider_unavailable",
                    format!(
                        "external owner inventory exceeds {MAX_EXTERNAL_INVENTORY_BYTES} bytes"
                    ),
                ));
            }
            let bytes = self.read_relative(&relative, remaining.min(MAX_CONFIGURATION_BYTES))?;
            retained_bytes = retained_bytes.saturating_add(bytes.len());
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
                    .map_err(|error| ViewError::new("provider_unavailable", error.message))?;
            if evidence.artifact_kind() != expected_kind {
                return Err(ViewError::new(
                    "provider_unavailable",
                    format!(
                        "external descriptor `{}` has kind `{}`, expected `{expected_kind}`",
                        relative.display(),
                        evidence.artifact_kind()
                    ),
                ));
            }
            let artifact_name = evidence.artifact_name().ok_or_else(|| {
                ViewError::new(
                    "provider_unavailable",
                    format!(
                        "external descriptor `{}` has no non-empty Properties/Name",
                        relative.display()
                    ),
                )
            })?;
            if !registered.insert((expected_kind.to_string(), artifact_name.to_string())) {
                return Err(ViewError::new(
                    "provider_unavailable",
                    format!("external owner `{expected_kind}.{artifact_name}` is ambiguous"),
                ));
            }
            if path.file_stem().and_then(|value| value.to_str()) != Some(artifact_name) {
                owner_path_mismatches.push(format!(
                    "external descriptor `{}` belongs to `{artifact_name}`",
                    relative.display()
                ));
            }
        }
        if registered.is_empty() {
            return Err(ViewError::new(
                "provider_unavailable",
                "external source set has no valid top-level owner descriptors",
            ));
        }
        if let Some(message) = owner_path_mismatches.into_iter().next() {
            return Err(ViewError::new("provider_unavailable", message));
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

    pub(crate) fn form_payload(&self, target: &MetadataAddress) -> Result<Value, ViewError> {
        serde_json::to_value(self.form_data(target)?)
            .map_err(|error| ViewError::new("provider_unavailable", error.to_string()))
    }

    pub(crate) fn form_data(&self, target: &MetadataAddress) -> Result<FormInfoData, ViewError> {
        self.metadata_descriptor(target)?;
        let relative = self.attached_resource_relative(target, "Form.xml")?;
        let bytes = self.read_relative(&relative, MAX_CONFIGURATION_BYTES)?;
        let text = std::str::from_utf8(&bytes)
            .map_err(|_| ViewError::new("provider_unavailable", "Form.xml is not UTF-8"))?;
        let parts = target.as_str().split('.').collect::<Vec<_>>();
        let form_name = parts.last().copied().unwrap_or("Form").to_string();
        let object_context = if parts.first().copied() == Some("CommonForm") {
            String::new()
        } else {
            parts[..parts.len().saturating_sub(2)].join(".")
        };
        parse_form_info_xml(
            text,
            form_name,
            object_context,
            self.object_support(target)?,
        )
        .map_err(|error| ViewError::new("provider_unavailable", error))
    }

    pub(crate) fn dcs_payload(&self, target: &MetadataAddress) -> Result<Value, ViewError> {
        self.metadata_descriptor(target)?;
        let relative = self.attached_resource_relative(target, "Template.xml")?;
        let bytes = self.read_relative(&relative, MAX_CONFIGURATION_BYTES)?;
        let text = std::str::from_utf8(&bytes)
            .map_err(|_| ViewError::new("provider_unavailable", "DCS Template.xml is not UTF-8"))?;
        let data = parse_dcs_info_xml(text, self.object_support(target)?)
            .map_err(|error| ViewError::new("provider_unavailable", error))?;
        serde_json::to_value(data)
            .map_err(|error| ViewError::new("provider_unavailable", error.to_string()))
    }

    pub(crate) fn role_payload(&self, target: &MetadataAddress) -> Result<Value, ViewError> {
        let rights = self.read_relative(
            &self.attached_resource_relative(target, "Rights.xml")?,
            MAX_CONFIGURATION_BYTES,
        )?;
        let rights = std::str::from_utf8(&rights)
            .map_err(|_| ViewError::new("provider_unavailable", "Rights.xml is not UTF-8"))?;
        let descriptor = self.metadata_descriptor(target)?;
        let descriptor = std::str::from_utf8(&descriptor)
            .map_err(|_| ViewError::new("provider_unavailable", "role descriptor is not UTF-8"))?;
        let fallback_name = target.as_str().rsplit('.').next().unwrap_or("Role");
        let data = parse_role_info_xml(
            rights,
            Some(descriptor),
            fallback_name,
            self.object_support(target)?,
        )
        .map_err(|error| ViewError::new("provider_unavailable", error))?;
        serde_json::to_value(data)
            .map_err(|error| ViewError::new("provider_unavailable", error.to_string()))
    }

    pub(crate) fn metadata_local(
        &self,
        target: &MetadataAddress,
    ) -> Result<MetaLocalInfo, ViewError> {
        let descriptor = self.metadata_descriptor(target)?;
        let support = metadata_support_status(
            self.object_support_with_descriptor(target, Some(&descriptor))?,
        );
        parse_typed_meta_local_info(&descriptor, target, support).map_err(|failure| {
            let message = serde_json::to_string(&failure.diagnostics)
                .unwrap_or_else(|_| "metadata reader failed".to_string());
            ViewError::new("provider_unavailable", message)
        })
    }

    pub(crate) fn external_metadata_payload(
        &self,
        target: &MetadataAddress,
    ) -> Result<Option<Value>, ViewError> {
        if !self.is_external_source_set() {
            return Ok(None);
        }
        let parts = target.as_str().split('.').collect::<Vec<_>>();
        let [expected_kind, expected_name] = parts.as_slice() else {
            return Err(ViewError::new(
                "provider_unavailable",
                "external typed metadata target must name its root owner",
            ));
        };
        let descriptor = self.metadata_descriptor(target)?;
        let relative = self.metadata_descriptor_relative(target)?;
        let evidence =
            prove_already_read_source_set_owner(&relative, &descriptor, self.source_set_kind)
                .map_err(|error| ViewError::new("provider_unavailable", error.message))?;
        if evidence.artifact_kind() != *expected_kind
            || evidence.artifact_name() != Some(*expected_name)
        {
            return Err(ViewError::new(
                "provider_unavailable",
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
            .map_err(|error| ViewError::new("provider_unavailable", error.message))?;
        identity_metadata_payload_from_evidence(target, &evidence)
    }

    pub(crate) fn metadata_owner_evidence(
        &self,
        target: &MetadataAddress,
    ) -> Result<PlatformXmlSourceSetOwnerEvidence, ViewError> {
        let descriptor = self.metadata_descriptor(target)?;
        let relative = self.metadata_descriptor_relative(target)?;
        if self.is_external_source_set() && target.as_str().split('.').count() == 2 {
            prove_already_read_source_set_owner(&relative, &descriptor, self.source_set_kind)
                .map_err(|error| ViewError::new("provider_unavailable", error.message))
        } else {
            prove_already_read_metadata_owner(&relative, &descriptor)
                .map_err(|error| ViewError::new("provider_unavailable", error.message))
        }
    }

    pub(crate) fn mxl_payload(&self, target: &MetadataAddress) -> Result<Value, ViewError> {
        self.metadata_descriptor(target)?;
        let bytes = self.read_relative(
            &self.attached_resource_relative(target, "Template.xml")?,
            MAX_CONFIGURATION_BYTES,
        )?;
        let name = target.as_str().rsplit('.').next().unwrap_or("Template");
        let data = parse_mxl_info_xml(&bytes, name, self.object_support(target)?, false)
            .map_err(|error| ViewError::new("provider_unavailable", error))?;
        serde_json::to_value(data)
            .map_err(|error| ViewError::new("provider_unavailable", error.to_string()))
    }

    pub(crate) fn xdto_payload(
        &self,
        target: &MetadataAddress,
        type_name: Option<&str>,
    ) -> Result<Value, ViewError> {
        let descriptor = self.metadata_descriptor(target)?;
        let package = self.read_relative(
            &self.attached_resource_relative(target, "Package.bin")?,
            MAX_CONFIGURATION_BYTES,
        )?;
        let data =
            parse_xdto_info_bytes(&package, &descriptor, self.source_set(), target, type_name)
                .map_err(|error| ViewError::new("provider_unavailable", error))?;
        serde_json::to_value(data)
            .map_err(|error| ViewError::new("provider_unavailable", error.to_string()))
    }

    pub(crate) fn subsystem_payload(&self, target: &MetadataAddress) -> Result<Value, ViewError> {
        let descriptor = self.metadata_descriptor(target)?;
        let descriptor = std::str::from_utf8(&descriptor).map_err(|_| {
            ViewError::new("provider_unavailable", "subsystem descriptor is not UTF-8")
        })?;
        let ci_relative = self.attached_resource_relative(target, "CommandInterface.xml")?;
        let command_interface = self
            .read_optional_relative(&ci_relative, MAX_CONFIGURATION_BYTES)?
            .map(|bytes| {
                let text = std::str::from_utf8(&bytes).map_err(|_| {
                    ViewError::new("provider_unavailable", "CommandInterface.xml is not UTF-8")
                })?;
                parse_subsystem_command_interface_data(&ci_relative, text)
                    .map_err(|error| ViewError::new("provider_unavailable", error))
            })
            .transpose()?;
        let (data, _) = parse_subsystem_info_xml(
            Path::new(target.as_str()),
            descriptor,
            command_interface.is_some(),
        )
        .map_err(|error| ViewError::new("provider_unavailable", error))?;
        let result = build_subsystem_info_result(
            data,
            None,
            command_interface,
            self.object_support(target)?,
        );
        serde_json::to_value(result)
            .map_err(|error| ViewError::new("provider_unavailable", error.to_string()))
    }

    pub(crate) fn metadata_descriptor(
        &self,
        target: &MetadataAddress,
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
        self.read_optional_relative(&relative, MAX_CONFIGURATION_BYTES)?
            .ok_or_else(|| {
                ViewError::new(
                    "not_found",
                    format!("metadata descriptor `{}` is absent", relative.display()),
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

    pub(crate) fn metadata_child_profile(
        &self,
        target: &MetadataAddress,
    ) -> Result<MetadataChildProfile, ViewError> {
        let descriptor = self.metadata_descriptor(target)?;
        let owner_text = target
            .as_str()
            .split('.')
            .take(2)
            .collect::<Vec<_>>()
            .join(".");
        let owner = MetadataAddress::parse(PLATFORM_XML_8_3_27_FORMAT_2_20, &owner_text)
            .map_err(|error| ViewError::new("provider_unavailable", error.to_string()))?;
        let Some((parsed_target, profile)) = parse_child_profile_from_bytes(&descriptor, &owner)
            .map_err(|error| ViewError::new("provider_unavailable", error))?
        else {
            return Err(ViewError::new(
                "provider_unavailable",
                "registered physical child descriptor has the wrong artifact kind",
            ));
        };
        if parsed_target.as_str() != target.as_str() {
            return Err(ViewError::new(
                "provider_unavailable",
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
        self.read_optional_relative(&relative, MAX_CONFIGURATION_BYTES)?
            .map(|bytes| {
                String::from_utf8(bytes)
                    .map(|text| text.trim_start_matches('\u{feff}').to_string())
                    .map_err(|_| ViewError::new("provider_unavailable", "BSL module is not UTF-8"))
            })
            .transpose()
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

    pub(crate) fn object_support(
        &self,
        target: &MetadataAddress,
    ) -> Result<ObjectSupportData, ViewError> {
        self.object_support_with_descriptor(target, None)
    }

    /// A caller that already holds the descriptor bytes passes them in so the
    /// support lookup does not read the same file a second time.
    fn object_support_with_descriptor(
        &self,
        target: &MetadataAddress,
        descriptor: Option<&[u8]>,
    ) -> Result<ObjectSupportData, ViewError> {
        let Some(state) = self.support_state()? else {
            return Ok(unsupported_object());
        };
        let data = |state, direct_edit_safe| ObjectSupportData {
            state,
            direct_edit_safe,
        };
        if state.removed() {
            return Ok(data(ObjectSupportState::RemovedFromSupport, Some(true)));
        }
        if !state.global_editing_enabled() {
            return Ok(data(ObjectSupportState::ConfigurationReadOnly, Some(false)));
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
            ViewError::new(
                "provider_unavailable",
                "metadata descriptor has no support-state UUID evidence",
            )
        })?;
        Ok(match state.object_rule(&uuid) {
            Some(0) => data(ObjectSupportState::Locked, Some(false)),
            Some(1) => data(ObjectSupportState::EditableWithSupport, Some(true)),
            Some(2) => data(ObjectSupportState::RemovedFromSupport, Some(true)),
            _ => unsupported_object(),
        })
    }

    fn configuration_support(&self) -> Result<ConfigurationSupportData, ViewError> {
        let Some(state) = self.support_state()? else {
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
    ) -> Result<Option<crate::infrastructure::native_operations::common::SupportState>, ViewError>
    {
        let bytes = self.read_optional_relative(
            Path::new("Ext/ParentConfigurations.bin"),
            MAX_CONFIGURATION_BYTES,
        )?;
        parse_support_state_strict_bytes(bytes.as_deref())
            .map_err(|error| ViewError::new("provider_unavailable", error.to_string()))
    }

    fn home_page(&self) -> Result<Option<CfHomePageData>, ViewError> {
        let Some(bytes) = self.read_optional_relative(
            Path::new("Ext/HomePageWorkArea.xml"),
            MAX_CONFIGURATION_BYTES,
        )?
        else {
            return Ok(None);
        };
        let text = std::str::from_utf8(&bytes).map_err(|_| {
            ViewError::new("provider_unavailable", "HomePageWorkArea.xml is not UTF-8")
        })?;
        let layout = parse_cf_home_page_xml_strict(text)
            .map_err(|error| ViewError::new("provider_unavailable", error))?;
        Ok(Some(CfHomePageData {
            template: layout.template,
            left: cf_home_page_item_data(&layout.left),
            right: cf_home_page_item_data(&layout.right),
        }))
    }

    fn metadata_descriptor_relative(&self, target: &MetadataAddress) -> Result<PathBuf, ViewError> {
        shared_metadata_descriptor_relative(target, self.source_set_kind)
            .map_err(|error| ViewError::new("provider_unavailable", error))
    }

    fn attached_resource_relative(
        &self,
        target: &MetadataAddress,
        resource: &str,
    ) -> Result<PathBuf, ViewError> {
        shared_attached_resource_relative(target, resource, self.source_set_kind)
            .map_err(|error| ViewError::new("provider_unavailable", error))
    }

    fn module_relative(&self, target: &MetadataAddress) -> Result<PathBuf, ViewError> {
        shared_module_relative(target, self.source_set_kind)
            .map_err(|error| ViewError::new("provider_unavailable", error))
    }

    fn is_external_source_set(&self) -> bool {
        matches!(
            self.source_set_kind,
            SourceSetKind::ExternalProcessor | SourceSetKind::ExternalReport
        )
    }
}

fn identity_metadata_payload_from_evidence(
    target: &MetadataAddress,
    evidence: &PlatformXmlSourceSetOwnerEvidence,
) -> Result<Value, ViewError> {
    let parts = target.as_str().split('.').collect::<Vec<_>>();
    let [expected_kind, expected_name] = parts.as_slice() else {
        return Err(ViewError::new(
            "provider_unavailable",
            "identity metadata target must name one owner",
        ));
    };
    if evidence.artifact_kind() != *expected_kind
        || evidence.artifact_name() != Some(*expected_name)
    {
        return Err(ViewError::new(
            "provider_unavailable",
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
                return Err(ViewError::new(
                    "provider_unavailable",
                    "Platform XML export path is not a relative component path",
                ));
            }
        }
    }
    if components.is_empty() {
        return Err(ViewError::new(
            "provider_unavailable",
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
            ViewError::new(
                "provider_unavailable",
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
