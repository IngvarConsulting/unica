use std::collections::BTreeSet;

use uuid::Uuid;

use super::{
    provider::PlatformXmlProvider,
    schema::{
        child_metadata_class_profile, metadata_class_profile, MetadataClassProfile,
        MetadataClassRole, METADATA_NAMESPACE_2_20, ROOT_STRUCTURAL_CHILDREN,
    },
    semantic_map::unknown_metadata_class_profile,
    xml::{parse_bounded_xml_document, BoundedXmlError},
    ProbeOutcome,
};
use crate::domain::source_adapters::{
    FormatVersion, SnapshotEvidence, SourceAdapterError, SourceAdapterErrorKind, SourceBinding,
    SourceDescriptor, SourceFamily,
};

#[cfg(test)]
use super::SourceInput;
#[cfg(test)]
use crate::domain::source_adapters::source_id_for_configured_source_set;

pub(crate) struct PlatformXmlProbe;

impl PlatformXmlProbe {
    pub(crate) const fn new() -> Self {
        Self
    }

    #[cfg(test)]
    fn probe_path(&self, input: &SourceInput) -> Result<ProbeOutcome, SourceAdapterError> {
        let provider = PlatformXmlProvider::capture(&input.target, &input.source_root)?;
        let binding = SourceBinding::new(
            source_id_for_configured_source_set(
                input.configured_source_set.as_deref().ok_or_else(|| {
                    unavailable("Platform XML probe requires a configured source-set identity")
                })?,
            )?,
            SourceFamily::PlatformXml,
            input.declared_format.clone(),
            provider.target_identity().clone(),
            provider.revision()?,
        );
        self.probe_provider(&provider, provider.descriptor_key(), &binding)
    }

    pub(crate) fn probe_provider(
        &self,
        provider: &PlatformXmlProvider,
        descriptor_key: &str,
        binding: &SourceBinding,
    ) -> Result<ProbeOutcome, SourceAdapterError> {
        self.probe_snapshot(provider, descriptor_key, binding)
    }

    fn probe_snapshot(
        &self,
        provider: &PlatformXmlProvider,
        descriptor_key: &str,
        binding: &SourceBinding,
    ) -> Result<ProbeOutcome, SourceAdapterError> {
        let bytes = provider.read_relative(descriptor_key)?;
        let snapshot_evidence = SnapshotEvidence {
            revision: provider.revision()?,
            root_descriptor_digest: provider.digest_relative(descriptor_key)?,
        };
        self.probe_bytes(&bytes, snapshot_evidence, binding)
    }

    fn probe_bytes(
        &self,
        bytes: &[u8],
        snapshot_evidence: SnapshotEvidence,
        binding: &SourceBinding,
    ) -> Result<ProbeOutcome, SourceAdapterError> {
        let (_, document) = parse_bounded_xml_document(bytes).map_err(|error| match error {
            BoundedXmlError::InvalidUtf8 => corrupted("Platform XML descriptor is not valid UTF-8"),
            BoundedXmlError::Malformed => corrupted("Platform XML descriptor is malformed"),
            BoundedXmlError::ResourceLimit => SourceAdapterError::new(
                SourceAdapterErrorKind::ResourceLimit,
                "Platform XML descriptor nesting depth exceeds navigation limit",
            ),
        })?;
        let root = document.root_element();
        if root.tag_name().name() != "MetaDataObject"
            || root.tag_name().namespace() != Some(METADATA_NAMESPACE_2_20)
        {
            return Ok(ProbeOutcome::NoMatch);
        }

        let mut classes = root.children().filter(|node| node.is_element());
        let class = classes
            .next()
            .ok_or_else(|| corrupted("Platform XML metadata descriptor has no class"))?;
        if classes.next().is_some() {
            return Err(SourceAdapterError::new(
                SourceAdapterErrorKind::ProbeAmbiguous,
                "Platform XML metadata descriptor has multiple classes",
            ));
        }
        if class.tag_name().namespace() != Some(METADATA_NAMESPACE_2_20) {
            return Err(unsupported(
                "Platform XML metadata class has an unsupported namespace",
            ));
        }
        let class_name = class.tag_name().name();
        let profile =
            metadata_class_profile(class_name).unwrap_or_else(unknown_metadata_class_profile);
        let class_label = if profile == unknown_metadata_class_profile() {
            "unknown"
        } else {
            class_name
        };
        let mut detected_features = BTreeSet::new();
        detected_features.insert(format!("metadata-class:{class_label}"));
        inspect_structural_features(class, profile, true, &mut detected_features)?;
        let _uuid = match class.attribute("uuid") {
            Some(raw) => Some(
                Uuid::parse_str(raw)
                    .map_err(|_| corrupted("Platform XML metadata UUID is invalid"))?,
            ),
            None => None,
        };
        let version = root
            .attribute("version")
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| unsupported("Platform XML format version is missing"))?;
        let format_version = FormatVersion::parse(version.trim())
            .map_err(|_| unsupported("Platform XML format version is invalid"))?;

        let mut probe_evidence = vec![
            "platform-xml:metadata-namespace".to_string(),
            format!("platform-xml:metadata-class={class_label}"),
            format!("platform-xml:format-version={}", version.trim()),
        ];
        probe_evidence.extend(
            detected_features
                .iter()
                .map(|feature| format!("platform-xml:feature={feature}")),
        );

        Ok(ProbeOutcome::Match(SourceDescriptor {
            source_id: binding.source_id.clone(),
            family: SourceFamily::PlatformXml,
            format_version,
            producer_version: None,
            detected_features,
            probe_evidence,
            snapshot_evidence: Some(snapshot_evidence),
        }))
    }
}

fn inspect_structural_features(
    class: roxmltree::Node<'_, '_>,
    profile: &MetadataClassProfile,
    is_direct_metadata_root: bool,
    features: &mut BTreeSet<String>,
) -> Result<(), SourceAdapterError> {
    for child in structural_children(class) {
        let name = structural_child_name(child)?;
        if !ROOT_STRUCTURAL_CHILDREN.contains(&name) {
            return Err(unsupported(
                "Platform XML root contains an unsupported structural feature",
            ));
        }
        if let Some(scope) = structural_scope(profile, is_direct_metadata_root) {
            features.insert(format!("structural:{scope}:{name}"));
        }
        if name == "ChildObjects" {
            inspect_child_objects(child, profile, features)?;
        }
    }
    Ok(())
}

fn inspect_child_objects(
    child_objects: roxmltree::Node<'_, '_>,
    owner_profile: &MetadataClassProfile,
    features: &mut BTreeSet<String>,
) -> Result<(), SourceAdapterError> {
    for child in structural_children(child_objects) {
        let name = structural_child_name(child)?;
        let child_profile = child_metadata_class_profile(owner_profile, name).ok_or_else(|| {
            unsupported("Platform XML child objects contain an unsupported structural feature")
        })?;
        let child_label = if child_profile == unknown_metadata_class_profile() {
            "unknown"
        } else {
            name
        };
        features.insert(format!(
            "structural:{}:{child_label}",
            child_object_scope(owner_profile)
        ));
        inspect_structural_features(child, child_profile, false, features)?;
    }
    Ok(())
}

fn structural_scope(
    profile: &MetadataClassProfile,
    is_direct_metadata_root: bool,
) -> Option<&'static str> {
    if is_direct_metadata_root {
        Some("root")
    } else {
        match profile.role {
            MetadataClassRole::TabularSection => Some("tabular-section"),
            _ => None,
        }
    }
}

fn child_object_scope(owner_profile: &MetadataClassProfile) -> &'static str {
    match owner_profile.role {
        MetadataClassRole::Configuration => "configuration-child",
        MetadataClassRole::TabularSection => "tabular-section",
        _ => "child-object",
    }
}

fn structural_children<'a, 'input>(
    node: roxmltree::Node<'a, 'input>,
) -> impl Iterator<Item = roxmltree::Node<'a, 'input>> {
    node.children().filter(|child| child.is_element())
}

fn structural_child_name<'a, 'input>(
    child: roxmltree::Node<'a, 'input>,
) -> Result<&'input str, SourceAdapterError> {
    if child.tag_name().namespace() != Some(METADATA_NAMESPACE_2_20) {
        return Err(unsupported(
            "Platform XML structural feature has an unsupported namespace",
        ));
    }
    Ok(child.tag_name().name())
}

pub(super) fn probe_provider(
    provider: &PlatformXmlProvider,
    descriptor_key: &str,
    binding: &SourceBinding,
) -> Result<ProbeOutcome, SourceAdapterError> {
    PlatformXmlProbe::new().probe_provider(provider, descriptor_key, binding)
}

fn unavailable(message: &str) -> SourceAdapterError {
    SourceAdapterError::new(SourceAdapterErrorKind::SourceUnavailable, message)
}

fn corrupted(message: &str) -> SourceAdapterError {
    SourceAdapterError::new(SourceAdapterErrorKind::DecodeCorrupted, message)
}

fn unsupported(message: &str) -> SourceAdapterError {
    SourceAdapterError::new(SourceAdapterErrorKind::FormatUnsupported, message)
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
        domain::source_adapters::{FormatVersion, SourceAdapterErrorKind, SourceFamily},
        versions::v2_20::{ProbeOutcome, SourceInput},
    };

    static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(0);

    #[test]
    fn probe_recognizes_exact_platform_xml_2_20_with_snapshot_evidence() {
        let outcome = probe_fixture(
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Document uuid="11111111-1111-1111-1111-111111111111"/></MetaDataObject>"#,
            Some("main"),
        )
        .unwrap();

        let ProbeOutcome::Match(descriptor) = outcome else {
            panic!("expected Platform XML match")
        };
        assert_eq!(descriptor.family, SourceFamily::PlatformXml);
        assert_eq!(
            descriptor.format_version,
            FormatVersion::parse("2.20").unwrap()
        );
        assert!(descriptor
            .detected_features
            .contains("metadata-class:Document"));
        let snapshot = descriptor.snapshot_evidence.unwrap();
        assert!(snapshot.root_descriptor_digest.starts_with("sha256:"));
        assert!(serde_json::to_string(&snapshot)
            .unwrap()
            .contains("sha256:"));
    }

    #[test]
    fn probe_reports_but_does_not_guess_platform_xml_2_19() {
        let outcome = probe_fixture(
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.19"><Document/></MetaDataObject>"#,
            Some("main"),
        )
        .unwrap();

        let ProbeOutcome::Match(descriptor) = outcome else {
            panic!("family and version should still be evidenced")
        };
        assert_eq!(
            descriptor.format_version,
            FormatVersion::parse("2.19").unwrap()
        );
    }

    #[test]
    fn malformed_and_ambiguous_platform_xml_are_typed_failures() {
        let malformed = probe_fixture("<MetaDataObject", Some("main")).unwrap_err();
        assert_eq!(malformed.kind, SourceAdapterErrorKind::DecodeCorrupted);

        let ambiguous = probe_fixture(
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Document/><Document/></MetaDataObject>"#,
            Some("main"),
        )
        .unwrap_err();
        assert_eq!(ambiguous.kind, SourceAdapterErrorKind::ProbeAmbiguous);
    }

    #[test]
    fn probe_preflights_depth_and_bounds_metadata_class_cardinality() {
        let mut deep = String::from(
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20">"#,
        );
        for _ in 1..crate::domain::navigation_limits::MAX_NAVIGATION_NESTING_DEPTH {
            deep.push_str("<Nested>");
        }
        deep.push_str("<Leaf/>");
        let deep_error = probe_fixture(&deep, Some("main")).unwrap_err();
        assert_eq!(deep_error.kind, SourceAdapterErrorKind::ResourceLimit);

        let max_plus_one = probe_fixture(
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Document/><Document/></MetaDataObject>"#,
            Some("main"),
        )
        .unwrap_err();
        assert_eq!(max_plus_one.kind, SourceAdapterErrorKind::ProbeAmbiguous);
    }

    #[test]
    fn invalid_utf8_version_uuid_and_shape_are_typed_failures() {
        let invalid_utf8 = probe_bytes_fixture(&[0xff], Some("main")).unwrap_err();
        assert_eq!(invalid_utf8.kind, SourceAdapterErrorKind::DecodeCorrupted);

        let invalid_version = probe_fixture(
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="two.twenty"><Document/></MetaDataObject>"#,
            Some("main"),
        )
        .unwrap_err();
        assert_eq!(
            invalid_version.kind,
            SourceAdapterErrorKind::FormatUnsupported
        );

        let invalid_uuid = probe_fixture(
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Document uuid="not-a-uuid"/></MetaDataObject>"#,
            Some("main"),
        )
        .unwrap_err();
        assert_eq!(invalid_uuid.kind, SourceAdapterErrorKind::DecodeCorrupted);

        let missing_class = probe_fixture(
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"/>"#,
            Some("main"),
        )
        .unwrap_err();
        assert_eq!(missing_class.kind, SourceAdapterErrorKind::DecodeCorrupted);
    }

    #[test]
    fn supported_legacy_metadata_classes_are_recognized() {
        for class in ["Configuration", "Catalog", "Document"] {
            let outcome = probe_fixture(
                &format!(r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><{class}/></MetaDataObject>"#),
                Some("main"),
            )
            .unwrap();
            let ProbeOutcome::Match(descriptor) = outcome else {
                panic!("expected {class} match")
            };
            assert!(descriptor
                .detected_features
                .contains(&format!("metadata-class:{class}")));
        }
    }

    #[test]
    fn unknown_metadata_class_fails_closed() {
        let error = probe_fixture(
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><SyntheticMetadata/></MetaDataObject>"#,
            Some("main"),
        )
        .unwrap_err();

        assert_eq!(error.kind, SourceAdapterErrorKind::FormatUnsupported);
    }

    #[test]
    fn unknown_nested_structural_features_fail_closed_for_representative_classes() {
        for xml in [
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Configuration><ChildObjects><UnknownFeature/></ChildObjects></Configuration></MetaDataObject>"#,
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Catalog><ChildObjects><TabularSection><ChildObjects><UnknownFeature/></ChildObjects></TabularSection></ChildObjects></Catalog></MetaDataObject>"#,
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Document><ChildObjects><Attribute><UnknownFeature/></Attribute></ChildObjects></Document></MetaDataObject>"#,
        ] {
            let error = probe_fixture(xml, Some("main")).unwrap_err();
            assert_eq!(error.kind, SourceAdapterErrorKind::FormatUnsupported);
        }
    }

    #[test]
    fn foreign_namespaced_structural_features_fail_closed() {
        let error = probe_fixture(
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" xmlns:future="urn:future" version="2.20"><Document><future:Feature/></Document></MetaDataObject>"#,
            Some("main"),
        )
        .unwrap_err();

        assert_eq!(error.kind, SourceAdapterErrorKind::FormatUnsupported);
    }

    #[test]
    fn configuration_child_objects_accept_shared_top_level_metadata_classes() {
        let outcome = probe_fixture(
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Configuration><ChildObjects><Catalog/><Document/><CommonModule/></ChildObjects></Configuration></MetaDataObject>"#,
            Some("main"),
        )
        .unwrap();

        let ProbeOutcome::Match(descriptor) = outcome else {
            panic!("expected Configuration match")
        };
        assert!(descriptor
            .detected_features
            .contains("structural:configuration-child:Catalog"));
        assert!(descriptor
            .detected_features
            .contains("structural:configuration-child:Document"));
        assert!(descriptor
            .detected_features
            .contains("structural:configuration-child:CommonModule"));
    }

    #[test]
    fn configuration_unknown_child_fails_closed() {
        let error = probe_fixture(
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Configuration><ChildObjects><SyntheticMetadata/></ChildObjects></Configuration></MetaDataObject>"#,
            Some("main"),
        )
        .unwrap_err();

        assert_eq!(error.kind, SourceAdapterErrorKind::FormatUnsupported);
    }

    #[test]
    fn every_shared_supported_class_has_a_minimal_probe_descriptor() {
        for class in super::super::metadata_classes() {
            let outcome = probe_fixture(
                &format!(r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><{class}/></MetaDataObject>"#),
                Some("main"),
            )
            .unwrap();
            let ProbeOutcome::Match(descriptor) = outcome else {
                panic!("expected {class} match")
            };
            assert!(descriptor
                .detected_features
                .contains(&format!("metadata-class:{class}")));
        }
    }

    #[test]
    fn detected_structural_features_are_deterministic() {
        let xml = r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Document><Properties><Name>Shipment</Name></Properties><ChildObjects><TabularSection><Properties><Name>Lines</Name></Properties><ChildObjects><Attribute><Properties><Name>Item</Name></Properties></Attribute></ChildObjects></TabularSection><Command><Properties><Name>Post</Name></Properties></Command></ChildObjects></Document></MetaDataObject>"#;
        let first = probe_fixture(xml, Some("main")).unwrap();
        let second = probe_fixture(xml, Some("main")).unwrap();

        let ProbeOutcome::Match(first) = first else {
            panic!("expected first match")
        };
        let ProbeOutcome::Match(second) = second else {
            panic!("expected second match")
        };
        assert_eq!(first.detected_features, second.detected_features);
        assert_eq!(
            first.detected_features.into_iter().collect::<Vec<_>>(),
            vec![
                "metadata-class:Document",
                "structural:child-object:Command",
                "structural:child-object:TabularSection",
                "structural:root:ChildObjects",
                "structural:root:Properties",
                "structural:tabular-section:Attribute",
                "structural:tabular-section:ChildObjects",
                "structural:tabular-section:Properties",
            ],
        );
    }

    #[test]
    fn root_properties_feature_is_not_inferred_from_nested_command_properties() {
        let nested = probe_fixture(
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Document><ChildObjects><Command><Properties><Name>Post</Name></Properties></Command></ChildObjects></Document></MetaDataObject>"#,
            Some("main"),
        )
        .unwrap();
        let direct = probe_fixture(
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Document><Properties><Name>Shipment</Name></Properties></Document></MetaDataObject>"#,
            Some("main"),
        )
        .unwrap();

        let ProbeOutcome::Match(nested) = nested else {
            panic!("expected nested match")
        };
        let ProbeOutcome::Match(direct) = direct else {
            panic!("expected direct match")
        };
        assert!(!nested
            .detected_features
            .contains("structural:root:Properties"));
        assert!(direct
            .detected_features
            .contains("structural:root:Properties"));
    }

    #[test]
    fn foreign_parsed_root_remains_no_match() {
        assert!(matches!(
            probe_fixture("<foreign/>", Some("main")).unwrap(),
            ProbeOutcome::NoMatch
        ));
    }

    #[test]
    fn configured_identity_is_stable_without_path_or_content() {
        let root = fixture_root();
        let target = root.join("Configuration.xml");
        fs::write(&target, r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Document uuid="11111111-1111-1111-1111-111111111111"/></MetaDataObject>"#).unwrap();
        let first = PlatformXmlProbe::new()
            .probe_path(&input(&root, &target, Some("Основная")))
            .unwrap();
        fs::write(&target, r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Document uuid="11111111-1111-1111-1111-111111111111" changed="yes"/></MetaDataObject>"#).unwrap();
        let second = PlatformXmlProbe::new()
            .probe_path(&input(&root, &target, Some("Основная")))
            .unwrap();
        let first = source_id_json(first);
        assert_eq!(first, source_id_json(second));
        assert!(first.contains("workspace:encoded-"));
    }

    #[test]
    fn path_shaped_source_set_and_errors_do_not_leak_physical_paths() {
        let root = fixture_root();
        let target = root.join("Configuration.xml");
        fs::write(&target, r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Document/></MetaDataObject>"#).unwrap();
        let error = PlatformXmlProbe::new()
            .probe_path(&input(&root, &target, Some("../outside")))
            .unwrap();
        assert!(source_id_json(error).contains("workspace:encoded-"));
    }

    #[test]
    fn missing_configured_source_set_fails_without_project_map_fallback() {
        let root = fixture_root();
        let target = root.join("Configuration.xml");
        fs::write(&root.join("v8project.yaml"), "source-set: invalid").unwrap();
        fs::write(&target, r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Document uuid="11111111-1111-1111-1111-111111111111"/></MetaDataObject>"#).unwrap();

        let error = PlatformXmlProbe::new()
            .probe_path(&input(&root, &target, None))
            .unwrap_err();

        assert_eq!(error.kind, SourceAdapterErrorKind::SourceUnavailable);
        assert!(!error.message.contains(&root.display().to_string()));
    }

    fn probe_fixture(
        xml: &str,
        configured_source_set: Option<&str>,
    ) -> Result<ProbeOutcome, crate::domain::source_adapters::SourceAdapterError> {
        probe_bytes_fixture(xml.as_bytes(), configured_source_set)
    }

    fn probe_bytes_fixture(
        bytes: &[u8],
        configured_source_set: Option<&str>,
    ) -> Result<ProbeOutcome, crate::domain::source_adapters::SourceAdapterError> {
        let root = fixture_root();
        let target = root.join("Configuration.xml");
        fs::write(&target, bytes).unwrap();
        PlatformXmlProbe::new().probe_path(&input(&root, &target, configured_source_set))
    }

    fn input(
        root: &std::path::Path,
        target: &std::path::Path,
        configured_source_set: Option<&str>,
    ) -> SourceInput {
        SourceInput {
            workspace_root: PathBuf::from(root),
            source_root: PathBuf::from(root),
            target: PathBuf::from(target),
            configured_source_set: configured_source_set.map(str::to_string),
            declared_family: SourceFamily::PlatformXml,
            declared_format: None,
        }
    }

    fn source_id_json(outcome: ProbeOutcome) -> String {
        let ProbeOutcome::Match(descriptor) = outcome else {
            panic!("expected Platform XML match")
        };
        serde_json::to_string(&descriptor.source_id).unwrap()
    }

    fn fixture_root() -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "unica-platform-xml-probe-{}-{}",
            std::process::id(),
            NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed),
        ));
        fs::create_dir_all(&root).unwrap();
        root
    }
}
