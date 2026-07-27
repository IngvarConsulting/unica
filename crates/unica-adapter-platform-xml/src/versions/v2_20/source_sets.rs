use std::path::Path;

use unica_format_core::{
    ports::{ReservedSourceArtifactKind, SourceSetMatch},
    source::{ConfiguredSourceSetKind, SourceAdapterError},
};

use super::{provider::PlatformXmlProvider, xml::parse_bounded_xml_document};

const MAX_RESERVED_DESCRIPTOR_BYTES: usize = 8 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ReservedDescriptorKind {
    RuntimeSidecar,
    ExternalProcessor,
    ExternalReport,
    MetadataDescriptor,
    Other,
}

pub(crate) fn inspect(
    source_root: &Path,
    authorized_root: &Path,
    kind: ConfiguredSourceSetKind,
) -> Result<SourceSetMatch, SourceAdapterError> {
    let Some(provider) =
        PlatformXmlProvider::capture_authorized_root(source_root, authorized_root)?
    else {
        return Ok(SourceSetMatch::NoMatch);
    };

    let matched = provider.configuration_bytes().is_some()
        || matches!(
            kind,
            ConfiguredSourceSetKind::ExternalProcessor
                | ConfiguredSourceSetKind::ExternalReport
        ) && provider
            .snapshot_files()
            .any(|(relative, bytes)| {
                is_root_xml(relative) && !is_reserved_sidecar(relative, &bytes, kind)
            });

    Ok(if matched {
        SourceSetMatch::Match
    } else {
        SourceSetMatch::NoMatch
    })
}

fn is_root_xml(relative: &str) -> bool {
    let path = Path::new(relative);
    path.parent()
        .is_some_and(|parent| parent.as_os_str().is_empty())
        && path.extension().and_then(|extension| extension.to_str()) == Some("xml")
}

fn is_reserved_sidecar(
    relative: &str,
    bytes: &[u8],
    kind: ConfiguredSourceSetKind,
) -> bool {
    let is_reserved_name = Path::new(relative)
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.eq_ignore_ascii_case("ConfigDumpInfo.xml"));
    if !is_reserved_name {
        return false;
    }
    !matches!(
        (reserved_descriptor_kind(bytes), kind),
        (
            ReservedDescriptorKind::ExternalProcessor,
            ConfiguredSourceSetKind::ExternalProcessor
        ) | (
            ReservedDescriptorKind::ExternalReport,
            ConfiguredSourceSetKind::ExternalReport
        )
    )
}

fn reserved_descriptor_kind(bytes: &[u8]) -> ReservedDescriptorKind {
    if bytes.len() > MAX_RESERVED_DESCRIPTOR_BYTES {
        return ReservedDescriptorKind::Other;
    }
    let Ok((_, document)) = parse_bounded_xml_document(bytes) else {
        return ReservedDescriptorKind::Other;
    };
    let root = document.root_element();
    if root.tag_name().name() == "ConfigDumpInfo" {
        return ReservedDescriptorKind::RuntimeSidecar;
    }
    if root.tag_name().name() != "MetaDataObject" {
        return ReservedDescriptorKind::Other;
    }
    let has_processor = root
        .children()
        .any(|child| child.is_element() && child.tag_name().name() == "ExternalDataProcessor");
    let has_report = root
        .children()
        .any(|child| child.is_element() && child.tag_name().name() == "ExternalReport");
    match (has_processor, has_report) {
        (true, false) => ReservedDescriptorKind::ExternalProcessor,
        (false, true) => ReservedDescriptorKind::ExternalReport,
        (false, false) | (true, true) => ReservedDescriptorKind::MetadataDescriptor,
    }
}

pub(crate) fn classify_reserved_source_artifact(bytes: &[u8]) -> ReservedSourceArtifactKind {
    match reserved_descriptor_kind(bytes) {
        ReservedDescriptorKind::RuntimeSidecar => ReservedSourceArtifactKind::RuntimeState,
        ReservedDescriptorKind::ExternalProcessor
        | ReservedDescriptorKind::ExternalReport
        | ReservedDescriptorKind::MetadataDescriptor => {
            ReservedSourceArtifactKind::AuthoredSource
        }
        ReservedDescriptorKind::Other => ReservedSourceArtifactKind::Unknown,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        fs,
        path::PathBuf,
        sync::atomic::{AtomicU64, Ordering},
        time::{SystemTime, UNIX_EPOCH},
    };

    static NONCE: AtomicU64 = AtomicU64::new(0);

    #[test]
    fn configuration_presence_is_classified_without_parsing_it() {
        let root = temp_root("configuration");
        fs::write(root.join("Configuration.xml"), "<malformed").unwrap();

        assert_eq!(
            inspect(&root, &root, ConfiguredSourceSetKind::Configuration).unwrap(),
            SourceSetMatch::Match
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn reserved_runtime_and_malformed_descriptors_do_not_match() {
        for (label, bytes) in [
            ("runtime", "<ConfigDumpInfo/>"),
            ("malformed", "<<not-xml"),
            (
                "nested",
                "<MetaDataObject><Properties><ExternalDataProcessor/></Properties></MetaDataObject>",
            ),
        ] {
            let root = temp_root(label);
            fs::write(root.join("ConfigDumpInfo.xml"), bytes).unwrap();
            assert_eq!(
                inspect(&root, &root, ConfiguredSourceSetKind::ExternalProcessor).unwrap(),
                SourceSetMatch::NoMatch
            );
            fs::remove_dir_all(root).unwrap();
        }
    }

    #[test]
    fn reserved_external_descriptor_matches_only_its_semantic_kind() {
        let root = temp_root("reserved-external");
        fs::write(
            root.join("ConfigDumpInfo.xml"),
            "<MetaDataObject><ExternalDataProcessor/></MetaDataObject>",
        )
        .unwrap();

        assert_eq!(
            inspect(&root, &root, ConfiguredSourceSetKind::ExternalProcessor).unwrap(),
            SourceSetMatch::Match
        );
        assert_eq!(
            inspect(&root, &root, ConfiguredSourceSetKind::ExternalReport).unwrap(),
            SourceSetMatch::NoMatch
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn missing_and_out_of_root_sources_never_match() {
        let workspace = temp_root("boundary");
        let missing = workspace.join("missing");
        assert_eq!(
            inspect(
                &missing,
                &workspace,
                ConfiguredSourceSetKind::Configuration
            )
            .unwrap(),
            SourceSetMatch::NoMatch
        );

        let outside = temp_root("outside");
        assert!(inspect(
            &outside,
            &workspace,
            ConfiguredSourceSetKind::Configuration
        )
        .is_err());
        fs::remove_dir_all(workspace).unwrap();
        fs::remove_dir_all(outside).unwrap();
    }

    fn temp_root(label: &str) -> PathBuf {
        let nonce = NONCE.fetch_add(1, Ordering::Relaxed);
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "unica-task7-source-set-{label}-{}-{now}-{nonce}",
            std::process::id()
        ));
        fs::create_dir_all(&root).unwrap();
        root
    }
}
