use std::collections::BTreeSet;

use crate::{
    domain::{
        navigation::NavigationEnvelope,
        source_adapters::{
            FormatRange, SourceAdapterError, SourceAdapterErrorKind, SourceDescriptor,
        },
    },
    infrastructure::source_adapters::{
        platform_xml::{probe::PlatformXmlProbe, provider::PlatformXmlProvider, PlatformXmlReadAdapter}, ProbeOutcome, SourceInput,
        SourceProbe, SourceReadAdapter,
    },
};

pub(crate) struct BuiltInSourceAdapterRegistry {
    probes: Vec<Box<dyn SourceProbe>>,
    readers: Vec<Box<dyn SourceReadAdapter>>,
}

impl BuiltInSourceAdapterRegistry {
    pub(crate) fn new() -> Self {
        Self::with_adapters(
            vec![Box::new(PlatformXmlProbe::new())],
            vec![Box::new(PlatformXmlReadAdapter::new())],
        )
    }

    pub(crate) fn with_adapters(
        probes: Vec<Box<dyn SourceProbe>>,
        readers: Vec<Box<dyn SourceReadAdapter>>,
    ) -> Self {
        Self { probes, readers }
    }

    pub(crate) fn inspect(
        &self,
        input: SourceInput,
    ) -> Result<NavigationEnvelope, SourceAdapterError> {
        let descriptor = self.probe(&input)?;
        let candidates = self.compatible_readers(&descriptor);

        let Some(reader) = self.select_narrowest_reader(candidates)? else {
            return Ok(NavigationEnvelope::unavailable(SourceAdapterError::new(
                SourceAdapterErrorKind::FormatUnsupported,
                format!(
                    "no reader supports {:?} format {}",
                    descriptor.family, descriptor.format_version
                ),
            )));
        };

        reader.inspect(&input, &descriptor)
    }

    /// Inspects a Platform XML target from an already captured immutable
    /// provider. No path is reopened after this entrypoint is called.
    pub(crate) fn inspect_platform_xml_provider(
        &self,
        input: SourceInput,
        provider: &PlatformXmlProvider,
        descriptor_key: &str,
    ) -> Result<NavigationEnvelope, SourceAdapterError> {
        let outcome = PlatformXmlProbe::new().probe_provider(&input, provider, descriptor_key)?;
        let ProbeOutcome::Match(descriptor) = outcome else {
            return Err(SourceAdapterError::new(
                SourceAdapterErrorKind::SourceUnavailable,
                "captured provider did not recognize the target as Platform XML",
            ));
        };
        let Some(reader) = self.select_narrowest_reader(self.compatible_readers(&descriptor))? else {
            return Ok(NavigationEnvelope::unavailable(SourceAdapterError::new(
                SourceAdapterErrorKind::FormatUnsupported,
                format!("no reader supports {:?} format {}", descriptor.family, descriptor.format_version),
            )));
        };
        reader.inspect_platform_xml_provider(provider, &descriptor).ok_or_else(|| {
            SourceAdapterError::new(
                SourceAdapterErrorKind::SourceUnavailable,
                "selected reader cannot inspect the captured Platform XML provider",
            )
        })?
    }

    fn probe(&self, input: &SourceInput) -> Result<SourceDescriptor, SourceAdapterError> {
        let matches = self
            .probes
            .iter()
            .filter_map(|probe| match probe.probe(input) {
                Ok(ProbeOutcome::NoMatch) => None,
                Ok(ProbeOutcome::Match(descriptor)) => Some(Ok(descriptor)),
                Err(error) => Some(Err(error)),
            })
            .collect::<Result<Vec<_>, _>>()?;

        let Some(first) = matches.first() else {
            return Err(SourceAdapterError::new(
                SourceAdapterErrorKind::SourceUnavailable,
                "no source probe recognized the target",
            ));
        };

        if matches
            .iter()
            .skip(1)
            .any(|other| !same_descriptor(first, other))
        {
            return Err(SourceAdapterError::new(
                SourceAdapterErrorKind::ProbeAmbiguous,
                "source probes identified incompatible source descriptors",
            ));
        }

        let mut descriptor = first.clone();
        descriptor.probe_evidence = matches
            .iter()
            .flat_map(|match_| match_.probe_evidence.iter().cloned())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        Ok(descriptor)
    }

    fn compatible_readers<'a>(&'a self, descriptor: &SourceDescriptor) -> Vec<Candidate<'a>> {
        self.readers
            .iter()
            .enumerate()
            .flat_map(|(reader_index, reader)| {
                let manifest = reader.manifest();
                let compatible = manifest.source_family == descriptor.family
                    && manifest.required_features.is_subset(&descriptor.detected_features)
                    && manifest.excluded_features.is_disjoint(&descriptor.detected_features);

                manifest
                    .supported_formats
                    .iter()
                    .filter(move |range| compatible && range.contains(&descriptor.format_version))
                    .map(move |range| Candidate { reader_index, range })
            })
            .collect()
    }

    fn select_narrowest_reader<'a>(
        &'a self,
        candidates: Vec<Candidate<'a>>,
    ) -> Result<Option<&'a dyn SourceReadAdapter>, SourceAdapterError> {
        if candidates.is_empty() {
            return Ok(None);
        }

        let minimal = candidates
            .iter()
            .filter(|candidate| {
                !candidates.iter().any(|other| {
                    range_is_strictly_narrower(other.range, candidate.range)
                })
            })
            .collect::<Vec<_>>();

        let mut reader_indices = minimal
            .iter()
            .map(|candidate| candidate.reader_index)
            .collect::<Vec<_>>();
        reader_indices.sort_unstable();
        reader_indices.dedup();

        match reader_indices.as_slice() {
            [] => Ok(None),
            [reader_index] => Ok(Some(self.readers[*reader_index].as_ref())),
            _ => Err(SourceAdapterError::new(
                SourceAdapterErrorKind::ProbeAmbiguous,
                "multiple readers have equally narrow compatible format ranges",
            )),
        }
    }
}

struct Candidate<'a> {
    reader_index: usize,
    range: &'a FormatRange,
}

fn same_descriptor(left: &SourceDescriptor, right: &SourceDescriptor) -> bool {
    left.source_id == right.source_id
        && left.family == right.family
        && left.format_version == right.format_version
        && left.producer_version == right.producer_version
        && left.detected_features == right.detected_features
        && left.snapshot_evidence == right.snapshot_evidence
}

fn range_is_no_wider_than(left: &FormatRange, right: &FormatRange) -> bool {
    left.min_inclusive >= right.min_inclusive && left.max_inclusive <= right.max_inclusive
}

fn range_is_strictly_narrower(left: &FormatRange, right: &FormatRange) -> bool {
    range_is_no_wider_than(left, right)
        && (left.min_inclusive != right.min_inclusive || left.max_inclusive != right.max_inclusive)
}

#[cfg(test)]
mod tests {
    use std::{
        collections::BTreeSet,
        path::PathBuf,
        sync::{
            atomic::{AtomicUsize, Ordering},
            Arc,
        },
    };

    use crate::{
        domain::{
            navigation::{NavigationEnvelope, NavigationStatus, SourceAdapterDiagnostic},
            source_adapters::{
                AdapterManifest, AdapterMaturity, FormatRange, FormatVersion, SnapshotConsistency,
                SourceAccess, SourceAdapterError, SourceDescriptor, SourceFamily, SourceId,
                SourceRevision, SourceSnapshot,
            },
        },
        infrastructure::source_adapters::{ProbeOutcome, SourceInput, SourceProbe, SourceReadAdapter},
    };

    use super::BuiltInSourceAdapterRegistry;

    #[test]
    fn built_in_registry_registers_only_the_platform_xml_probe_and_reader() {
        let registry = BuiltInSourceAdapterRegistry::new();

        assert_eq!(registry.probes.len(), 1);
        assert_eq!(registry.readers.len(), 1);
        assert_eq!(registry.readers[0].manifest().adapter_id, "platform-xml-2.20");
    }

    #[test]
    fn exact_reader_is_selected_for_probe_evidence() {
        let registry = registry_with(
            vec![probe_match("2.20")],
            vec![reader("xml-2.20", exact("2.20"))],
        );

        let read = registry.inspect(input()).unwrap();

        assert_eq!(read.snapshot.unwrap().adapter_id, "xml-2.20");
    }

    #[test]
    fn nearest_reader_is_never_selected() {
        let registry = registry_with(
            vec![probe_match("2.19")],
            vec![reader("xml-2.20", exact("2.20"))],
        );

        let envelope = registry.inspect(input()).unwrap();

        assert_eq!(envelope.status, NavigationStatus::Unavailable);
        assert_eq!(envelope.diagnostics[0].code, "format_unsupported");
    }

    #[test]
    fn equally_specific_readers_are_ambiguous() {
        let registry = registry_with(
            vec![probe_match("2.20")],
            vec![
                reader("xml-a", exact("2.20")),
                reader("xml-b", exact("2.20")),
            ],
        );

        let error = registry.inspect(input()).unwrap_err();

        assert_eq!(error.kind, crate::domain::source_adapters::SourceAdapterErrorKind::ProbeAmbiguous);
    }

    #[test]
    fn incomparable_overlapping_ranges_are_ambiguous() {
        let registry = registry_with(
            vec![probe_match("2.15")],
            vec![
                reader("xml-a", range("2.0", "2.20")),
                reader("xml-b", range("2.10", "2.30")),
            ],
        );

        let error = registry.inspect(input()).unwrap_err();

        assert_eq!(error.kind, crate::domain::source_adapters::SourceAdapterErrorKind::ProbeAmbiguous);
    }

    #[test]
    fn matching_probes_merge_evidence_independently_of_registration_order() {
        let first = registry_with(
            vec![probe_with_evidence("2.20", "b.xml"), probe_with_evidence("2.20", "a.xml")],
            vec![reader("xml-2.20", exact("2.20"))],
        );
        let second = registry_with(
            vec![probe_with_evidence("2.20", "a.xml"), probe_with_evidence("2.20", "b.xml")],
            vec![reader("xml-2.20", exact("2.20"))],
        );

        let first_read = first.inspect(input()).unwrap();
        let second_read = second.inspect(input()).unwrap();

        assert_eq!(first_read.diagnostics[0].message, "a.xml,b.xml");
        assert_eq!(first_read.diagnostics, second_read.diagnostics);
    }

    #[test]
    fn family_and_feature_exclusions_prevent_reader_selection() {
        let family_mismatch = registry_with(
            vec![probe_match("2.20")],
            vec![reader_with("edt-2.20", SourceFamily::Edt, exact("2.20"), [], [])],
        );
        let excluded_feature = registry_with(
            vec![probe_with_feature("2.20", "legacy")],
            vec![reader_with(
                "xml-no-legacy",
                SourceFamily::PlatformXml,
                exact("2.20"),
                [],
                ["legacy"],
            )],
        );

        assert_eq!(family_mismatch.inspect(input()).unwrap().status, NavigationStatus::Unavailable);
        assert_eq!(excluded_feature.inspect(input()).unwrap().status, NavigationStatus::Unavailable);
    }

    #[test]
    fn conflicting_probes_are_ambiguous() {
        let registry = registry_with(
            vec![probe_match("2.20"), probe_match("2.21")],
            vec![reader("xml-2.20", exact("2.20"))],
        );

        let error = registry.inspect(input()).unwrap_err();

        assert_eq!(error.kind, crate::domain::source_adapters::SourceAdapterErrorKind::ProbeAmbiguous);
    }

    #[test]
    fn selected_reader_error_does_not_retry_a_fallback() {
        let fallback_calls = Arc::new(AtomicUsize::new(0));
        let registry = registry_with(
            vec![probe_match("2.20")],
            vec![
                Box::new(FailingReader { manifest: manifest("xml-exact", exact("2.20")), error: SourceAdapterError::new(crate::domain::source_adapters::SourceAdapterErrorKind::DecodeCorrupted, "corrupt XML") }),
                Box::new(CountingReader { manifest: manifest("xml-broad", range("2.0", "2.30")), calls: Arc::clone(&fallback_calls) }),
            ],
        );

        let error = registry.inspect(input()).unwrap_err();

        assert_eq!(error.kind, crate::domain::source_adapters::SourceAdapterErrorKind::DecodeCorrupted);
        assert_eq!(fallback_calls.load(Ordering::SeqCst), 0);
    }

    fn registry_with(
        probes: Vec<Box<dyn SourceProbe>>,
        readers: Vec<Box<dyn SourceReadAdapter>>,
    ) -> BuiltInSourceAdapterRegistry {
        BuiltInSourceAdapterRegistry::with_adapters(probes, readers)
    }

    fn input() -> SourceInput {
        SourceInput {
            workspace_root: PathBuf::from("/workspace"),
            target: PathBuf::from("/workspace/Configuration.xml"),
            configured_source_set: None,
        }
    }

    fn probe_match(version: &str) -> Box<dyn SourceProbe> {
        Box::new(FakeProbe { descriptor: descriptor(version) })
    }

    fn probe_with_evidence(version: &str, evidence: &str) -> Box<dyn SourceProbe> {
        let mut descriptor = descriptor(version);
        descriptor.probe_evidence = vec![evidence.to_string()];
        Box::new(FakeProbe { descriptor })
    }

    fn probe_with_feature(version: &str, feature: &str) -> Box<dyn SourceProbe> {
        let mut descriptor = descriptor(version);
        descriptor.detected_features.insert(feature.to_string());
        Box::new(FakeProbe { descriptor })
    }

    fn reader(adapter_id: &'static str, range: FormatRange) -> Box<dyn SourceReadAdapter> {
        reader_with(adapter_id, SourceFamily::PlatformXml, range, [], [])
    }

    fn reader_with<const REQUIRED: usize, const EXCLUDED: usize>(
        adapter_id: &'static str,
        source_family: SourceFamily,
        range: FormatRange,
        required_features: [&str; REQUIRED],
        excluded_features: [&str; EXCLUDED],
    ) -> Box<dyn SourceReadAdapter> {
        Box::new(FakeReader {
            manifest: manifest_with(adapter_id, source_family, range, required_features, excluded_features),
        })
    }

    fn manifest(adapter_id: &'static str, range: FormatRange) -> AdapterManifest {
        manifest_with(adapter_id, SourceFamily::PlatformXml, range, [], [])
    }

    fn manifest_with<const REQUIRED: usize, const EXCLUDED: usize>(
        adapter_id: &'static str,
        source_family: SourceFamily,
        range: FormatRange,
        required_features: [&str; REQUIRED],
        excluded_features: [&str; EXCLUDED],
    ) -> AdapterManifest {
        AdapterManifest {
            adapter_id,
            adapter_version: "1",
            source_family,
            supported_formats: vec![range],
            required_features: required_features.into_iter().map(str::to_string).collect(),
            excluded_features: excluded_features.into_iter().map(str::to_string).collect(),
            source_access: SourceAccess::ReadOnly,
            maturity: AdapterMaturity::ReadCompatible,
        }
    }

    fn descriptor(version: &str) -> SourceDescriptor {
        SourceDescriptor {
            source_id: SourceId::new("workspace:main").unwrap(),
            family: SourceFamily::PlatformXml,
            format_version: FormatVersion::parse(version).unwrap(),
            producer_version: None,
            detected_features: BTreeSet::new(),
            probe_evidence: vec!["configuration.xml".to_string()],
            snapshot_evidence: None,
        }
    }

    fn exact(version: &str) -> FormatRange {
        FormatRange::exact(FormatVersion::parse(version).unwrap())
    }

    fn range(minimum: &str, maximum: &str) -> FormatRange {
        FormatRange {
            min_inclusive: FormatVersion::parse(minimum).unwrap(),
            max_inclusive: FormatVersion::parse(maximum).unwrap(),
        }
    }

    struct FakeProbe {
        descriptor: SourceDescriptor,
    }

    impl SourceProbe for FakeProbe {
        fn probe(&self, _input: &SourceInput) -> Result<ProbeOutcome, SourceAdapterError> {
            Ok(ProbeOutcome::Match(self.descriptor.clone()))
        }
    }

    struct FakeReader {
        manifest: AdapterManifest,
    }

    impl SourceReadAdapter for FakeReader {
        fn manifest(&self) -> &AdapterManifest {
            &self.manifest
        }

        fn inspect(
            &self,
            _input: &SourceInput,
            descriptor: &SourceDescriptor,
        ) -> Result<NavigationEnvelope, SourceAdapterError> {
            Ok(NavigationEnvelope {
                schema_version: "1".to_string(),
                status: NavigationStatus::Available,
                snapshot: Some(SourceSnapshot {
                    source_id: descriptor.source_id.clone(),
                    revision: SourceRevision::new("sha256:fixture").unwrap(),
                    consistency: SnapshotConsistency::Consistent,
                    adapter_id: self.manifest.adapter_id.to_string(),
                }),
                root: None,
                nodes: Vec::new(),
                relations: Vec::new(),
                diagnostics: vec![SourceAdapterDiagnostic {
                    code: "probe_evidence".to_string(),
                    message: descriptor.probe_evidence.join(","),
                }],
                relation_index: Vec::new(),
            })
        }
    }

    struct FailingReader {
        manifest: AdapterManifest,
        error: SourceAdapterError,
    }

    impl SourceReadAdapter for FailingReader {
        fn manifest(&self) -> &AdapterManifest {
            &self.manifest
        }

        fn inspect(
            &self,
            _input: &SourceInput,
            _descriptor: &SourceDescriptor,
        ) -> Result<NavigationEnvelope, SourceAdapterError> {
            Err(self.error.clone())
        }
    }

    struct CountingReader {
        manifest: AdapterManifest,
        calls: Arc<AtomicUsize>,
    }

    impl SourceReadAdapter for CountingReader {
        fn manifest(&self) -> &AdapterManifest {
            &self.manifest
        }

        fn inspect(
            &self,
            _input: &SourceInput,
            _descriptor: &SourceDescriptor,
        ) -> Result<NavigationEnvelope, SourceAdapterError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Err(SourceAdapterError::new(
                crate::domain::source_adapters::SourceAdapterErrorKind::DecodeCorrupted,
                "fallback must not run",
            ))
        }
    }
}
