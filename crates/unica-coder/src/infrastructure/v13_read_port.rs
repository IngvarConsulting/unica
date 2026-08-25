use crate::application::ports::{MetaLocalInfo, MetadataChildProfile};
use crate::application::v13::view::ViewError;
use crate::domain::cancellation::CancellationToken;
use crate::domain::code_intelligence::ProviderDeadline;
use crate::domain::metadata::MetaSupportStatus;
use crate::domain::project_sources::SourceSetKind;
use crate::domain::source_target::{MetadataAddress, PLATFORM_XML_8_3_27_FORMAT_2_20};
use crate::domain::support_state::{
    ConfigurationSupportData, ConfigurationSupportState, ObjectSupportData, ObjectSupportState,
    SupportCounts,
};
use crate::infrastructure::metadata_kinds::metadata_kind;
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
use crate::infrastructure::source_revision::SourceRevisionService;
use serde_json::Value;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;

const MAX_CONFIGURATION_BYTES: usize = 8 * 1024 * 1024;

/// One actor-issued read authority for one admitted source set. Hidden v0.13
/// reads are descriptor-relative to this retained directory and revisions come
/// from the paired actor-owned service.
pub(crate) struct ProviderReadAuthority {
    source_set: String,
    source_set_identity: String,
    source_set_kind: SourceSetKind,
    root: Arc<RetainedDirectoryCapability>,
    revisions: Arc<SourceRevisionService>,
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
            revisions,
        }
    }

    pub(crate) fn source_set(&self) -> &str {
        &self.source_set
    }

    pub(crate) fn source_set_identity(&self) -> &str {
        &self.source_set_identity
    }

    pub(crate) fn root_path(&self) -> &Path {
        self.root.path()
    }

    pub(crate) fn exact_revision(
        &self,
        deadline: ProviderDeadline,
        cancellation: &CancellationToken,
    ) -> Result<String, ViewError> {
        self.root
            .validate_named_identity()
            .map_err(|error| ViewError::new("provider_unavailable", error.to_string()))?;
        self.revisions
            .snapshot(deadline, cancellation)
            .map(|revision| {
                format!(
                    "{}:{}:{}",
                    revision.algorithm, revision.generation, revision.digest
                )
            })
            .map_err(|error| ViewError::new("provider_unavailable", error))
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

    pub(crate) fn configuration_payload(&self) -> Result<Value, ViewError> {
        let bytes = self.read_relative(Path::new("Configuration.xml"), MAX_CONFIGURATION_BYTES)?;
        let text = std::str::from_utf8(&bytes).map_err(|_| {
            ViewError::new("provider_unavailable", "Configuration.xml is not UTF-8")
        })?;
        let parsed = parse_cf_info_xml(text, self.configuration_support()?, self.home_page()?)
            .map_err(|error| ViewError::new("provider_unavailable", error))?;
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
            serde_json::to_value(parsed.registered_objects)
                .map_err(|error| ViewError::new("provider_unavailable", error.to_string()))?,
        );
        Ok(Value::Object(payload))
    }

    pub(crate) fn form_payload(&self, target: &MetadataAddress) -> Result<Value, ViewError> {
        serde_json::to_value(self.form_data(target)?)
            .map_err(|error| ViewError::new("provider_unavailable", error.to_string()))
    }

    pub(crate) fn form_data(&self, target: &MetadataAddress) -> Result<FormInfoData, ViewError> {
        self.metadata_descriptor(target)?;
        let relative = attached_resource_relative(target, "Form.xml")?;
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
        let relative = attached_resource_relative(target, "Template.xml")?;
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
            &attached_resource_relative(target, "Rights.xml")?,
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
        let support = metadata_support_status(self.object_support(target)?);
        parse_typed_meta_local_info(&descriptor, target, support).map_err(|failure| {
            let message = serde_json::to_string(&failure.diagnostics)
                .unwrap_or_else(|_| "metadata reader failed".to_string());
            ViewError::new("provider_unavailable", message)
        })
    }

    pub(crate) fn mxl_payload(&self, target: &MetadataAddress) -> Result<Value, ViewError> {
        self.metadata_descriptor(target)?;
        let bytes = self.read_relative(
            &attached_resource_relative(target, "Template.xml")?,
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
            &attached_resource_relative(target, "Package.bin")?,
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
        let ci_relative = attached_resource_relative(target, "CommandInterface.xml")?;
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
        let relative = metadata_descriptor_relative(target)?;
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
        path_text(metadata_descriptor_relative(target)?)
    }

    pub(crate) fn attached_resource_export_path(
        &self,
        target: &MetadataAddress,
        resource: &str,
    ) -> Result<String, ViewError> {
        path_text(attached_resource_relative(target, resource)?)
    }

    pub(crate) fn module_export_path(&self, target: &MetadataAddress) -> Result<String, ViewError> {
        path_text(module_relative(target)?)
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
        let relative = module_relative(target)?;
        self.read_optional_relative(&relative, MAX_CONFIGURATION_BYTES)?
            .map(|bytes| {
                String::from_utf8(bytes)
                    .map(|text| text.trim_start_matches('\u{feff}').to_string())
                    .map_err(|_| ViewError::new("provider_unavailable", "BSL module is not UTF-8"))
            })
            .transpose()
    }

    pub(crate) fn object_support(
        &self,
        target: &MetadataAddress,
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
        let descriptor = self.metadata_descriptor(target)?;
        let uuid = support_root_uuid_from_bytes(&descriptor).ok_or_else(|| {
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
}

fn path_text(path: PathBuf) -> Result<String, ViewError> {
    path.to_str().map(str::to_string).ok_or_else(|| {
        ViewError::new(
            "provider_unavailable",
            "Platform XML export path is not valid UTF-8",
        )
    })
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

fn attached_resource_relative(
    target: &MetadataAddress,
    resource: &str,
) -> Result<PathBuf, ViewError> {
    let parts = target.as_str().split('.').collect::<Vec<_>>();
    let [owner_kind, owner_name, rest @ ..] = parts.as_slice() else {
        return Err(ViewError::new(
            "provider_unavailable",
            "typed reader target has no owner identity",
        ));
    };
    let owner = metadata_kind(owner_kind).ok_or_else(|| {
        ViewError::new(
            "provider_unavailable",
            format!("typed reader owner kind `{owner_kind}` has no platform layout"),
        )
    })?;
    let mut relative = PathBuf::from(owner.directory);
    relative.push(owner_name);
    match rest {
        [] if (*owner_kind == "CommonForm" && resource == "Form.xml")
            || (*owner_kind == "Role" && resource == "Rights.xml")
            || (*owner_kind == "Subsystem" && resource == "CommandInterface.xml")
            || (*owner_kind == "XDTOPackage" && resource == "Package.bin") =>
        {
            relative.push("Ext");
            relative.push(resource);
        }
        [nested_kind, nested_name] => {
            let directory = match *nested_kind {
                "Form" => "Forms",
                "Template" => "Templates",
                "Command" => "Commands",
                _ => {
                    return Err(ViewError::new(
                        "provider_unavailable",
                        format!("unsupported attached resource owner `{nested_kind}`"),
                    ));
                }
            };
            relative.push(directory);
            relative.push(nested_name);
            relative.push("Ext");
            relative.push(resource);
        }
        _ => {
            return Err(ViewError::new(
                "provider_unavailable",
                "typed reader target has an unsupported attached-resource depth",
            ));
        }
    }
    Ok(relative)
}

fn metadata_descriptor_relative(target: &MetadataAddress) -> Result<PathBuf, ViewError> {
    let parts = target.as_str().split('.').collect::<Vec<_>>();
    let [owner_kind, owner_name, rest @ ..] = parts.as_slice() else {
        return Err(ViewError::new(
            "provider_unavailable",
            "metadata descriptor target has no owner identity",
        ));
    };
    let owner = metadata_kind(owner_kind).ok_or_else(|| {
        ViewError::new(
            "provider_unavailable",
            format!("metadata kind `{owner_kind}` has no platform layout"),
        )
    })?;
    let mut relative = PathBuf::from(owner.directory);
    match rest {
        [] => relative.push(format!("{owner_name}.xml")),
        [nested_kind, nested_name] => {
            relative.push(owner_name);
            relative.push(match *nested_kind {
                "Form" => "Forms",
                "Template" => "Templates",
                "Command" => "Commands",
                _ => {
                    return Err(ViewError::new(
                        "provider_unavailable",
                        format!("unsupported nested descriptor kind `{nested_kind}`"),
                    ));
                }
            });
            relative.push(format!("{nested_name}.xml"));
        }
        _ => {
            return Err(ViewError::new(
                "provider_unavailable",
                "metadata descriptor target has unsupported depth",
            ));
        }
    }
    Ok(relative)
}

fn module_relative(target: &MetadataAddress) -> Result<PathBuf, ViewError> {
    let parts = target.as_str().split('.').collect::<Vec<_>>();
    match parts.as_slice() {
        [terminal] => Ok(PathBuf::from(format!("{terminal}.bsl"))),
        [owner_kind, owner_name, terminal] => {
            let owner = metadata_kind(owner_kind).ok_or_else(|| {
                ViewError::new(
                    "provider_unavailable",
                    format!("module owner kind `{owner_kind}` has no platform layout"),
                )
            })?;
            let file = if *owner_kind == "CommonForm" && *terminal == "FormModule" {
                PathBuf::from("Form/Module.bsl")
            } else {
                PathBuf::from(format!("{terminal}.bsl"))
            };
            Ok(PathBuf::from(owner.directory)
                .join(owner_name)
                .join("Ext")
                .join(file))
        }
        [owner_kind, owner_name, nested_kind, nested_name, terminal] => {
            let owner = metadata_kind(owner_kind).ok_or_else(|| {
                ViewError::new(
                    "provider_unavailable",
                    format!("module owner kind `{owner_kind}` has no platform layout"),
                )
            })?;
            let (directory, file) = match (*nested_kind, *terminal) {
                ("Form", "FormModule") => ("Forms", PathBuf::from("Form/Module.bsl")),
                ("Command", "CommandModule") => ("Commands", PathBuf::from("CommandModule.bsl")),
                _ => {
                    return Err(ViewError::new(
                        "provider_unavailable",
                        "nested module target has unsupported layout",
                    ));
                }
            };
            Ok(PathBuf::from(owner.directory)
                .join(owner_name)
                .join(directory)
                .join(nested_name)
                .join("Ext")
                .join(file))
        }
        _ => Err(ViewError::new(
            "provider_unavailable",
            "module target has unsupported platform layout",
        )),
    }
}
