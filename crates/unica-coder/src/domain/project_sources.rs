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
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceSetKind {
    Configuration,
    Extension,
    ExternalProcessor,
    ExternalReport,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceFormat {
    PlatformXml,
    Edt,
    Unknown,
    Invalid,
}

pub(crate) fn config_dump_info_xml_kind(bytes: &[u8]) -> ConfigDumpInfoXmlKind {
    if bytes.len() as u64 > MAX_RESERVED_EXTERNAL_DESCRIPTOR_BYTES {
        return ConfigDumpInfoXmlKind::Other;
    }
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
