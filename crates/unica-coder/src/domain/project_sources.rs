use crate::domain::format_profile::ACTIVE_FORMAT_PROFILE;
use crate::domain::platform_profile::PlatformProfile;
use crate::domain::source_target::PLATFORM_XML_8_3_27_FORMAT_2_20;
use roxmltree::Document;
use serde::Serialize;

const MAX_RESERVED_EXTERNAL_DESCRIPTOR_BYTES: u64 = 8 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ConfigDumpInfoXmlKind {
    RuntimeSidecar,
    ExternalProcessor,
    ExternalReport,
    MetadataDescriptor,
    Other,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectSourceMap {
    pub workspace_root: String,
    pub config_path: Option<String>,
    pub source_sets: Vec<ProjectSourceSet>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub effective_source_set: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub effective_source_root: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_selection_error: Option<String>,
    #[serde(skip_serializing)]
    pub(crate) configured_format_raw: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectSourceSet {
    pub name: String,
    pub kind: SourceSetKind,
    pub path: String,
    pub source_format: SourceFormat,
    pub format_evidence: Vec<String>,
    #[serde(skip_serializing)]
    pub(crate) format_probe_error: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceSetKind {
    Configuration,
    Extension,
    ExternalProcessor,
    ExternalReport,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceFormat {
    PlatformXml,
    Edt,
    Unknown,
    Invalid,
}

impl SourceSetKind {
    pub(crate) const fn stable_discriminant(self) -> u8 {
        match self {
            Self::Configuration => 1,
            Self::Extension => 2,
            Self::ExternalProcessor => 3,
            Self::ExternalReport => 4,
        }
    }
}

impl SourceFormat {
    pub(crate) const fn stable_discriminant(self) -> u8 {
        match self {
            Self::PlatformXml => 1,
            Self::Edt => 2,
            Self::Unknown => 3,
            Self::Invalid => 4,
        }
    }
}

/// Closed semantic profile carried by an actor-owned source binding.
///
/// `SourceFormat` remains a separate discriminant because the physical source
/// representation and the exact platform/serialization capability are
/// independently planner-significant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) enum SourceProfile {
    PlatformXml8_3_27Format2_20,
    LegacyWorkspaceServiceCompatibility,
    #[cfg(test)]
    TestPlatform8_3_28Format2_20,
    #[cfg(test)]
    TestPlatform8_3_27Format2_21,
}

impl SourceProfile {
    pub(crate) const fn platform_xml_8_3_27_format_2_20() -> Self {
        Self::PlatformXml8_3_27Format2_20
    }

    pub(crate) const fn legacy_workspace_service_compatibility() -> Self {
        Self::LegacyWorkspaceServiceCompatibility
    }

    pub(crate) const fn stable_discriminants(self) -> [u8; 3] {
        match self {
            Self::PlatformXml8_3_27Format2_20 => [1, 1, 1],
            Self::LegacyWorkspaceServiceCompatibility => [2, 0, 0],
            #[cfg(test)]
            Self::TestPlatform8_3_28Format2_20 => [1, 2, 1],
            #[cfg(test)]
            Self::TestPlatform8_3_27Format2_21 => [1, 1, 2],
        }
    }

    pub(crate) const fn platform_line(self) -> Option<&'static str> {
        match self {
            Self::PlatformXml8_3_27Format2_20 => Some(ACTIVE_FORMAT_PROFILE.platform_line),
            Self::LegacyWorkspaceServiceCompatibility => None,
            #[cfg(test)]
            Self::TestPlatform8_3_28Format2_20 => Some("8.3.28-test"),
            #[cfg(test)]
            Self::TestPlatform8_3_27Format2_21 => Some(ACTIVE_FORMAT_PROFILE.platform_line),
        }
    }

    pub(crate) const fn serialization_format(self) -> Option<&'static str> {
        match self {
            Self::PlatformXml8_3_27Format2_20 => Some(ACTIVE_FORMAT_PROFILE.export_format),
            Self::LegacyWorkspaceServiceCompatibility => None,
            #[cfg(test)]
            Self::TestPlatform8_3_28Format2_20 => Some(ACTIVE_FORMAT_PROFILE.export_format),
            #[cfg(test)]
            Self::TestPlatform8_3_27Format2_21 => Some("2.21-test"),
        }
    }

    pub(crate) const fn canonical_id(self) -> &'static str {
        match self {
            Self::PlatformXml8_3_27Format2_20 => PLATFORM_XML_8_3_27_FORMAT_2_20,
            Self::LegacyWorkspaceServiceCompatibility => "legacy-workspace-service-compatibility",
            #[cfg(test)]
            Self::TestPlatform8_3_28Format2_20 => "platform-xml-8.3.28-test-format-2.20",
            #[cfg(test)]
            Self::TestPlatform8_3_27Format2_21 => "platform-xml-8.3.27-format-2.21-test",
        }
    }

    pub(crate) fn platform_profile(self) -> Option<PlatformProfile> {
        match self {
            Self::PlatformXml8_3_27Format2_20 => {
                debug_assert_eq!(self.canonical_id(), PLATFORM_XML_8_3_27_FORMAT_2_20);
                debug_assert_eq!(self.platform_line(), Some(PlatformProfile::v8_3_27().id()));
                debug_assert_eq!(
                    self.serialization_format(),
                    Some(ACTIVE_FORMAT_PROFILE.export_format)
                );
                Some(PlatformProfile::v8_3_27())
            }
            Self::LegacyWorkspaceServiceCompatibility => None,
            #[cfg(test)]
            Self::TestPlatform8_3_28Format2_20 | Self::TestPlatform8_3_27Format2_21 => None,
        }
    }
}

pub(crate) fn config_dump_info_xml_kind(bytes: &[u8]) -> ConfigDumpInfoXmlKind {
    if bytes.len() as u64 > MAX_RESERVED_EXTERNAL_DESCRIPTOR_BYTES {
        return ConfigDumpInfoXmlKind::Other;
    }
    classify_already_read_config_dump_info_xml(bytes)
}

pub(crate) fn classify_already_read_config_dump_info_xml(bytes: &[u8]) -> ConfigDumpInfoXmlKind {
    let Ok(xml) = std::str::from_utf8(bytes) else {
        return ConfigDumpInfoXmlKind::Other;
    };
    let Ok(document) = Document::parse(xml.trim_start_matches('\u{feff}')) else {
        return ConfigDumpInfoXmlKind::Other;
    };
    let root = document.root_element();
    if root.tag_name().name() == "ConfigDumpInfo" {
        return ConfigDumpInfoXmlKind::RuntimeSidecar;
    }
    if root.tag_name().name() != "MetaDataObject" {
        return ConfigDumpInfoXmlKind::Other;
    }
    let has_external_processor = root
        .children()
        .any(|node| node.is_element() && node.tag_name().name() == "ExternalDataProcessor");
    let has_external_report = root
        .children()
        .any(|node| node.is_element() && node.tag_name().name() == "ExternalReport");
    match (has_external_processor, has_external_report) {
        (true, false) => ConfigDumpInfoXmlKind::ExternalProcessor,
        (false, true) => ConfigDumpInfoXmlKind::ExternalReport,
        (false, false) | (true, true) => ConfigDumpInfoXmlKind::MetadataDescriptor,
    }
}

#[cfg(test)]
mod tests {
    use super::{config_dump_info_xml_kind, ConfigDumpInfoXmlKind};

    #[test]
    fn classifies_config_dump_info_xml_from_bytes_without_io() {
        assert_eq!(
            config_dump_info_xml_kind(b"<ConfigDumpInfo/>"),
            ConfigDumpInfoXmlKind::RuntimeSidecar
        );
        assert_eq!(
            config_dump_info_xml_kind(b"<MetaDataObject><ExternalDataProcessor/></MetaDataObject>"),
            ConfigDumpInfoXmlKind::ExternalProcessor
        );
        assert_eq!(
            config_dump_info_xml_kind(b"not xml"),
            ConfigDumpInfoXmlKind::Other
        );
    }
}
