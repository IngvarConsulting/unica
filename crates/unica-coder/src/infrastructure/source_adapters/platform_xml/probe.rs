use std::{collections::BTreeSet, fs};

use roxmltree::Document;
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::{
    domain::source_adapters::{
        FormatVersion, SourceAdapterError, SourceAdapterErrorKind, SourceDescriptor, SourceFamily,
        SourceId,
    },
    infrastructure::{
        project_sources::discover_project_source_map,
        source_adapters::{ProbeOutcome, SourceInput, SourceProbe},
    },
};

const METADATA_NAMESPACE: &str = "http://v8.1c.ru/8.3/MDClasses";

pub(crate) struct PlatformXmlProbe;

impl PlatformXmlProbe {
    pub(crate) const fn new() -> Self {
        Self
    }

    fn probe_bytes(
        &self,
        input: &SourceInput,
        bytes: &[u8],
    ) -> Result<ProbeOutcome, SourceAdapterError> {
        let xml = match std::str::from_utf8(bytes.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(bytes)) {
            Ok(xml) => xml,
            Err(_) => return Ok(ProbeOutcome::NoMatch),
        };
        let document = match Document::parse(xml) {
            Ok(document) => document,
            Err(_) => return Ok(ProbeOutcome::NoMatch),
        };
        let root = document.root_element();
        if root.tag_name().name() != "MetaDataObject"
            || root.tag_name().namespace() != Some(METADATA_NAMESPACE)
        {
            return Ok(ProbeOutcome::NoMatch);
        }

        let classes = root
            .children()
            .filter(|node| node.is_element())
            .collect::<Vec<_>>();
        let [class] = classes.as_slice() else {
            return Ok(ProbeOutcome::NoMatch);
        };
        if class.tag_name().namespace() != Some(METADATA_NAMESPACE) {
            return Ok(ProbeOutcome::NoMatch);
        }
        if let Some(uuid) = class.attribute("uuid") {
            if Uuid::parse_str(uuid).is_err() {
                return Ok(ProbeOutcome::NoMatch);
            }
        }

        let Some(version) = root.attribute("version").filter(|value| !value.trim().is_empty()) else {
            return Ok(ProbeOutcome::NoMatch);
        };
        let Ok(format_version) = FormatVersion::parse(version.trim()) else {
            return Ok(ProbeOutcome::NoMatch);
        };

        let source_id = if let Some(source_set) = &input.configured_source_set {
            SourceId::new(format!("workspace:{source_set}"))?
        } else {
            discover_project_source_map(&input.workspace_root).map_err(|error| {
                SourceAdapterError::new(
                    SourceAdapterErrorKind::SourceUnavailable,
                    format!("failed to discover project source map: {error}"),
                )
            })?;
            SourceId::new(format!("adhoc:sha256:{:x}", Sha256::digest(bytes)))?
        };

        Ok(ProbeOutcome::Match(SourceDescriptor {
            source_id,
            family: SourceFamily::PlatformXml,
            format_version,
            producer_version: None,
            detected_features: BTreeSet::new(),
            probe_evidence: vec![
                "platform-xml:metadata-namespace".to_string(),
                format!("platform-xml:metadata-class={}", class.tag_name().name()),
                format!("platform-xml:format-version={}", version.trim()),
            ],
        }))
    }
}

impl SourceProbe for PlatformXmlProbe {
    fn probe(&self, input: &SourceInput) -> Result<ProbeOutcome, SourceAdapterError> {
        let bytes = fs::read(&input.target).map_err(|error| {
            SourceAdapterError::new(
                SourceAdapterErrorKind::SourceUnavailable,
                format!("Platform XML root descriptor is unavailable: {error}"),
            )
        })?;
        self.probe_bytes(input, &bytes)
    }
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        path::PathBuf,
        sync::atomic::{AtomicU64, Ordering},
    };

    use super::PlatformXmlProbe;
    use crate::{
        domain::source_adapters::{FormatVersion, SourceFamily},
        infrastructure::source_adapters::{ProbeOutcome, SourceInput, SourceProbe},
    };

    static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(0);

    #[test]
    fn probe_recognizes_exact_platform_xml_2_20() {
        let outcome = probe_fixture(
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses"
                 version="2.20"><Document uuid="11111111-1111-1111-1111-111111111111"/></MetaDataObject>"#,
        )
        .unwrap();

        let ProbeOutcome::Match(descriptor) = outcome else {
            panic!("expected Platform XML match");
        };
        assert_eq!(descriptor.family, SourceFamily::PlatformXml);
        assert_eq!(descriptor.format_version, FormatVersion::parse("2.20").unwrap());
    }

    #[test]
    fn probe_reports_but_does_not_guess_platform_xml_2_19() {
        let outcome = probe_fixture(
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses"
                 version="2.19"><Document/></MetaDataObject>"#,
        )
        .unwrap();

        let ProbeOutcome::Match(descriptor) = outcome else {
            panic!("family and version should still be evidenced");
        };
        assert_eq!(descriptor.format_version, FormatVersion::parse("2.19").unwrap());
    }

    #[test]
    fn probe_fails_closed_for_an_ambiguous_metadata_descriptor() {
        let outcome = probe_fixture(
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Document/><Catalog/></MetaDataObject>"#,
        )
        .unwrap();

        assert!(matches!(outcome, ProbeOutcome::NoMatch));
    }

    fn probe_fixture(xml: &str) -> Result<ProbeOutcome, crate::domain::source_adapters::SourceAdapterError> {
        let root = std::env::temp_dir().join(format!(
            "unica-platform-xml-probe-{}-{}",
            std::process::id(),
            NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed),
        ));
        fs::create_dir_all(&root).unwrap();
        let target = root.join("Configuration.xml");
        fs::write(&target, xml).unwrap();
        PlatformXmlProbe::new().probe(&SourceInput {
            workspace_root: PathBuf::from(&root),
            target,
            configured_source_set: Some("main".to_string()),
        })
    }
}
