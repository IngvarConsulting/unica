use std::path::Path;

use unica_format_core::{
    ports::{ReservedSourceArtifactKind, SourceSetMatch},
    source::{ConfiguredSourceSetKind, SourceAdapterError, SourceAdapterErrorKind},
};

use crate::safe_root::{
    ArtifactReadLimit, DirectoryPageLimit, DirectoryVisit, SafeRootError, SafeSourceRoot,
};

use super::xml::parse_bounded_xml_document;

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
    let root = match SafeSourceRoot::capture(authorized_root, source_root) {
        Ok(root) => root,
        Err(SafeRootError::Missing) => return Ok(SourceSetMatch::NoMatch),
        Err(error) => return Err(source_error(error)),
    };
    let matched = match root.read_relative("Configuration.xml", ArtifactReadLimit::Descriptor) {
        Ok(_) => true,
        Err(SafeRootError::Missing) => {
            matches!(
                kind,
                ConfiguredSourceSetKind::ExternalProcessor
                    | ConfiguredSourceSetKind::ExternalReport
            ) && inspect_external_root(&root, kind)?
        }
        Err(error) => return Err(source_error(error)),
    };

    Ok(if matched {
        SourceSetMatch::Match
    } else {
        SourceSetMatch::NoMatch
    })
}

fn inspect_external_root(
    root: &SafeSourceRoot,
    kind: ConfiguredSourceSetKind,
) -> Result<bool, SourceAdapterError> {
    let mut matched = false;
    root.visit_directory("", DirectoryPageLimit::RootDiscovery, |name| {
        let Some(name) = name.to_str() else {
            return Ok(DirectoryVisit::Ignore);
        };
        if Path::new(name)
            .extension()
            .and_then(|extension| extension.to_str())
            != Some("xml")
        {
            return Ok(DirectoryVisit::Ignore);
        }
        let bytes = root.read_relative(name, ArtifactReadLimit::Descriptor)?;
        if !is_reserved_sidecar(name, bytes.bytes(), kind) {
            matched = true;
        }
        Ok(DirectoryVisit::Selected)
    })
    .map_err(source_error)?;
    Ok(matched)
}

fn source_error(_error: SafeRootError) -> SourceAdapterError {
    SourceAdapterError::new(
        SourceAdapterErrorKind::SourceUnavailable,
        "source-set evidence could not be authorized and inspected",
    )
}

fn is_reserved_sidecar(relative: &str, bytes: &[u8], kind: ConfiguredSourceSetKind) -> bool {
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
        | ReservedDescriptorKind::MetadataDescriptor => ReservedSourceArtifactKind::AuthoredSource,
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
            inspect(&missing, &workspace, ConfiguredSourceSetKind::Configuration).unwrap(),
            SourceSetMatch::NoMatch
        );

        let outside = temp_root("outside");
        assert!(inspect(&outside, &workspace, ConfiguredSourceSetKind::Configuration).is_err());
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
