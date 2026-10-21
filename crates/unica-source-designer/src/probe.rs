//! Classifies already-read Designer XML bytes without owning source access.

use roxmltree::Document;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigDumpInfoXmlKind {
    RuntimeSidecar,
    ExternalProcessor,
    ExternalReport,
    MetadataDescriptor,
    Other,
}

pub fn config_dump_info_xml_kind(bytes: &[u8]) -> ConfigDumpInfoXmlKind {
    classify_already_read_config_dump_info_xml(bytes)
}

pub fn classify_already_read_config_dump_info_xml(bytes: &[u8]) -> ConfigDumpInfoXmlKind {
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

    fn large_xml(root: &str, direct_child: &str) -> Vec<u8> {
        let mut bytes = format!("<{root}>{direct_child}<!--").into_bytes();
        bytes.extend(std::iter::repeat_n(b'x', 9 * 1024 * 1024));
        bytes.extend_from_slice(format!("--></{root}>").as_bytes());
        bytes
    }

    #[test]
    fn config_dump_info_kind_crosses_former_byte_limit_with_complete_xml() {
        for (root, child, expected) in [
            ("ConfigDumpInfo", "", ConfigDumpInfoXmlKind::RuntimeSidecar),
            (
                "MetaDataObject",
                "<ExternalDataProcessor/>",
                ConfigDumpInfoXmlKind::ExternalProcessor,
            ),
            (
                "MetaDataObject",
                "<ExternalReport/>",
                ConfigDumpInfoXmlKind::ExternalReport,
            ),
        ] {
            let bytes = large_xml(root, child);
            assert!(bytes.len() > 8 * 1024 * 1024);
            assert!(bytes.len() < 10 * 1024 * 1024);
            assert_eq!(config_dump_info_xml_kind(&bytes), expected, "root={root}");
        }
    }

    #[test]
    fn config_dump_info_kind_keeps_local_name_and_complete_document_semantics() {
        for (bytes, expected) in [
            (
                "<x:ConfigDumpInfo xmlns:x='urn:arbitrary'/>".as_bytes(),
                ConfigDumpInfoXmlKind::RuntimeSidecar,
            ),
            (
                "<x:MetaDataObject xmlns:x='urn:arbitrary'><y:ExternalDataProcessor xmlns:y='urn:other'/></x:MetaDataObject>".as_bytes(),
                ConfigDumpInfoXmlKind::ExternalProcessor,
            ),
            (
                "\u{feff}\u{feff}<MetaDataObject><ExternalReport/></MetaDataObject>".as_bytes(),
                ConfigDumpInfoXmlKind::ExternalReport,
            ),
            (
                "<MetaDataObject><ExternalDataProcessor/><ExternalReport/></MetaDataObject>".as_bytes(),
                ConfigDumpInfoXmlKind::MetadataDescriptor,
            ),
            (
                "<MetaDataObject><ExternalDataProcessor/><ExternalDataProcessor/></MetaDataObject>"
                    .as_bytes(),
                ConfigDumpInfoXmlKind::ExternalProcessor,
            ),
            (
                "<MetaDataObject><Child><ExternalDataProcessor/></Child></MetaDataObject>".as_bytes(),
                ConfigDumpInfoXmlKind::MetadataDescriptor,
            ),
            (
                "<MetaDataObject><ExternalDataProcessor/></Different>".as_bytes(),
                ConfigDumpInfoXmlKind::Other,
            ),
            (
                "<ConfigDumpInfo/><ConfigDumpInfo/>".as_bytes(),
                ConfigDumpInfoXmlKind::Other,
            ),
            (
                "<!DOCTYPE ConfigDumpInfo><ConfigDumpInfo/>".as_bytes(),
                ConfigDumpInfoXmlKind::Other,
            ),
            (
                "<x:ConfigDumpInfo/>".as_bytes(),
                ConfigDumpInfoXmlKind::Other,
            ),
            (
                "<ConfigDumpInfo a='1' a='2'/>".as_bytes(),
                ConfigDumpInfoXmlKind::Other,
            ),
            (
                "<ConfigDumpInfo xmlns:p='urn:same' xmlns:q='urn:same' p:a='1' q:a='2'/>"
                    .as_bytes(),
                ConfigDumpInfoXmlKind::Other,
            ),
            (
                "<ConfigDumpInfo>\u{1}</ConfigDumpInfo>".as_bytes(),
                ConfigDumpInfoXmlKind::Other,
            ),
            (
                "<ConfigDumpInfo><!--illegal--comment--></ConfigDumpInfo>".as_bytes(),
                ConfigDumpInfoXmlKind::Other,
            ),
            (
                "<?xml version='unknown' encoding='unknown' standalone='unknown'?><ConfigDumpInfo/>"
                    .as_bytes(),
                ConfigDumpInfoXmlKind::RuntimeSidecar,
            ),
            (
                "<?xml version='1.1'?><ConfigDumpInfo>&#1;</ConfigDumpInfo>".as_bytes(),
                ConfigDumpInfoXmlKind::Other,
            ),
            (
                "<?xml?><ConfigDumpInfo><?XML arbitrary?><?xml\tdata?></ConfigDumpInfo>"
                    .as_bytes(),
                ConfigDumpInfoXmlKind::RuntimeSidecar,
            ),
            (
                "<ConfigDumpInfo><?xml version='1.0'?></ConfigDumpInfo>".as_bytes(),
                ConfigDumpInfoXmlKind::Other,
            ),
        ] {
            assert_eq!(config_dump_info_xml_kind(bytes), expected);
        }
    }

    #[test]
    fn config_dump_info_kind_rejects_corruption_after_former_byte_limit() {
        let mut bytes = large_xml("MetaDataObject", "<ExternalDataProcessor/>");
        bytes.extend_from_slice(b"<unfinished");
        assert_eq!(
            config_dump_info_xml_kind(&bytes),
            ConfigDumpInfoXmlKind::Other
        );

        let mut bytes = large_xml("MetaDataObject", "<ExternalDataProcessor/>");
        let position = bytes.len() - b"--></MetaDataObject>".len() - 1;
        bytes[position] = 0xff;
        assert_eq!(
            config_dump_info_xml_kind(&bytes),
            ConfigDumpInfoXmlKind::Other
        );
    }

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
