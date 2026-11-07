use super::{ProviderReadAuthority, TemplateValidationInput, ViewError};
use serde_json::Value;
use std::path::PathBuf;
use unica_address::source_target::{MetadataAddress, PLATFORM_XML_8_3_27_FORMAT_2_20};
use unica_refusal::{RefusalCode, RefusalDetail};
use unica_templates_designer::mxl_codec::parse_mxl_info_xml;

impl ProviderReadAuthority {
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
}
