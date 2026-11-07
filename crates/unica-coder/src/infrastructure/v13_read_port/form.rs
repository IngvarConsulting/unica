use super::{
    object_support_from_state, object_support_rule, FormValidationInput, ProviderReadAuthority,
    ViewError,
};
use serde_json::{json, Value};
use std::path::PathBuf;
use unica_address::address::NodeKind;
use unica_address::source_target::{MetadataAddress, PLATFORM_XML_8_3_27_FORMAT_2_20};
use unica_forms::facts::{FormInfoData, FormSupportDetails, FormSupportRuleReason};
use unica_forms_designer::codec::parse_form_info_xml;
use unica_metadata::support_state::ObjectSupportState;
use unica_metadata::support_state::SupportState;
use unica_metadata::validation_subject::MetadataChildProfile;
use unica_refusal::{RefusalCode, RefusalDetail};
use unica_source_contract::SourceSetKind;

impl ProviderReadAuthority {
    pub(crate) fn form_metadata_payload(
        &self,
        target: &MetadataAddress,
        checkpoint: &mut dyn FnMut() -> Result<(), ViewError>,
    ) -> Result<Value, ViewError> {
        let bytes = self.metadata_descriptor_with_checkpoint(target, checkpoint)?;
        let (mut properties, limits) = crate::infrastructure::family_codecs::metadata_read_codec()
            .form_metadata(&bytes, target)
            .map_err(|error| ViewError::detailed(RefusalDetail::SourceUnreadable, error))?;
        if self.source_set_kind == SourceSetKind::Extension {
            let borrowing = crate::infrastructure::family_codecs::metadata_read_codec()
                .borrowing(&bytes)
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
        if crate::infrastructure::family_codecs::metadata_read_codec()
            .form_profile(&descriptor)
            .is_ok_and(|profile| profile == MetadataChildProfile::OrdinaryForm)
        {
            return Err(ViewError::new(RefusalCode::UnsupportedSource, format!(
                "ordinary Form.bin content is not supported; read `{}:{}.Metadata` for the form declaration",
                self.source_set(), target.as_str(),
            )));
        }
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
}

/// Missing marker is proven absence; with a marker the descriptor UUID is
/// required even when a global restriction would hide the own rule.
fn form_support_rule(
    state: Option<&SupportState>,
    descriptor: &[u8],
) -> Result<Option<ObjectSupportState>, ViewError> {
    let Some(state) = state else { return Ok(None) };
    let uuid = crate::infrastructure::family_codecs::metadata_read_codec()
        .support_root_uuid(descriptor)
        .ok_or_else(|| {
            ViewError::detailed(
                RefusalDetail::SourceUnreadable,
                "metadata descriptor has no support-state UUID evidence",
            )
        })?;
    Ok(object_support_rule(state.object_rule(&uuid)))
}
