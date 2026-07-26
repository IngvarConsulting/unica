use std::collections::BTreeSet;

use crate::{
    domain::{
        navigation::{NavigationEnvelope, NavigationStatus},
        source_adapters::{
            FormatRange, SourceAdapterError, SourceAdapterErrorKind, SourceBinding,
            SourceDescriptor,
        },
    },
    infrastructure::source_adapters::{
        platform_xml::{
            probe::PlatformXmlProbe, PlatformXmlCaptureAdapter, PlatformXmlReadAdapter,
        },
        CaptureOutcome, CapturedSourceSession, ProbeOutcome, SourceCaptureAdapter, SourceInput,
        SourceProbe, SourceReadAdapter,
    },
};

pub(crate) struct BuiltInSourceAdapterRegistry {
    capturers: Vec<Box<dyn SourceCaptureAdapter>>,
    probes: Vec<Box<dyn SourceProbe>>,
    readers: Vec<Box<dyn SourceReadAdapter>>,
}

impl BuiltInSourceAdapterRegistry {
    pub(crate) fn new() -> Self {
        Self::with_adapters(
            vec![Box::new(PlatformXmlCaptureAdapter::new())],
            vec![Box::new(PlatformXmlProbe::new())],
            vec![Box::new(PlatformXmlReadAdapter::new())],
        )
    }

    pub(crate) fn with_adapters(
        capturers: Vec<Box<dyn SourceCaptureAdapter>>,
        probes: Vec<Box<dyn SourceProbe>>,
        readers: Vec<Box<dyn SourceReadAdapter>>,
    ) -> Self {
        Self {
            capturers,
            probes,
            readers,
        }
    }

    pub(crate) fn inspect(
        &self,
        input: SourceInput,
    ) -> Result<NavigationEnvelope, SourceAdapterError> {
        let session = self.capture(&input)?;
        self.inspect_captured(&input, session.as_ref())
    }

    pub(crate) fn capture(
        &self,
        input: &SourceInput,
    ) -> Result<Box<dyn CapturedSourceSession>, SourceAdapterError> {
        let mut sessions = Vec::new();
        for capturer in self
            .capturers
            .iter()
            .filter(|capturer| capturer.source_family() == input.declared_family)
        {
            match capturer.capture(input)? {
                CaptureOutcome::NoMatch => {}
                CaptureOutcome::Captured(session) => {
                    if session.source_family() != input.declared_family {
                        return Err(SourceAdapterError::new(
                            SourceAdapterErrorKind::SnapshotInconsistent,
                            "source capture adapter returned a session for a different family",
                        ));
                    }
                    sessions.push(session);
                }
            }
        }
        match sessions.len() {
            0 => Err(SourceAdapterError::new(
                SourceAdapterErrorKind::SourceUnavailable,
                "no source capture adapter recognized the target",
            )),
            1 => Ok(sessions.into_iter().next().expect("one captured session")),
            _ => Err(SourceAdapterError::new(
                SourceAdapterErrorKind::ProbeAmbiguous,
                "multiple source capture adapters recognized the target",
            )),
        }
    }

    pub(crate) fn inspect_captured(
        &self,
        input: &SourceInput,
        session: &dyn CapturedSourceSession,
    ) -> Result<NavigationEnvelope, SourceAdapterError> {
        let descriptor = self.probe(input, session)?;
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

        let envelope = reader.inspect_captured(session, &descriptor)?;
        validate_ready_envelope(&envelope, session, &descriptor, reader)?;
        Ok(envelope)
    }

    fn probe(
        &self,
        input: &SourceInput,
        session: &dyn CapturedSourceSession,
    ) -> Result<SourceDescriptor, SourceAdapterError> {
        let mut matches = Vec::new();
        for probe in self
            .probes
            .iter()
            .filter(|probe| probe.source_family() == session.source_family())
        {
            match probe.probe(input, session)? {
                ProbeOutcome::NoMatch => {}
                ProbeOutcome::Match(descriptor) => {
                    validate_probe_descriptor(input, session, &descriptor)?;
                    matches.push(descriptor);
                }
            }
        }

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
                    && manifest
                        .required_features
                        .is_subset(&descriptor.detected_features)
                    && manifest
                        .excluded_features
                        .is_disjoint(&descriptor.detected_features);

                manifest
                    .supported_formats
                    .iter()
                    .filter(move |range| compatible && range.contains(&descriptor.format_version))
                    .map(move |range| Candidate {
                        reader_index,
                        range,
                    })
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
                !candidates
                    .iter()
                    .any(|other| range_is_strictly_narrower(other.range, candidate.range))
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

fn validate_probe_descriptor(
    input: &SourceInput,
    session: &dyn CapturedSourceSession,
    descriptor: &SourceDescriptor,
) -> Result<(), SourceAdapterError> {
    if descriptor.family != input.declared_family || descriptor.family != session.source_family() {
        return Err(SourceAdapterError::new(
            SourceAdapterErrorKind::SnapshotInconsistent,
            "source probe descriptor family does not match the captured session",
        ));
    }
    if let Some(format) = session.declared_format() {
        if descriptor.format_version != *format {
            return Err(SourceAdapterError::new(
                SourceAdapterErrorKind::SnapshotInconsistent,
                "source probe descriptor format does not match the declared session format",
            ));
        }
    }
    let binding = session.binding();
    if descriptor.source_id != binding.source_id {
        return Err(SourceAdapterError::new(
            SourceAdapterErrorKind::SnapshotInconsistent,
            "source probe descriptor source id does not match the captured binding",
        ));
    }
    let evidence = descriptor.snapshot_evidence.as_ref().ok_or_else(|| {
        SourceAdapterError::new(
            SourceAdapterErrorKind::SnapshotInconsistent,
            "source probe descriptor has no immutable snapshot evidence",
        )
    })?;
    if evidence.revision != binding.revision {
        return Err(SourceAdapterError::new(
            SourceAdapterErrorKind::SnapshotStale,
            "source probe descriptor revision differs from the captured session",
        ));
    }
    Ok(())
}

fn validate_ready_envelope(
    envelope: &NavigationEnvelope,
    session: &dyn CapturedSourceSession,
    _descriptor: &SourceDescriptor,
    reader: &dyn SourceReadAdapter,
) -> Result<(), SourceAdapterError> {
    if envelope.status != NavigationStatus::Available {
        return Ok(());
    }
    let snapshot = envelope.snapshot.as_ref().ok_or_else(|| {
        SourceAdapterError::new(
            SourceAdapterErrorKind::SnapshotInconsistent,
            "ready navigation envelope has no snapshot",
        )
    })?;
    let binding = session.binding();
    if snapshot.source_id != binding.source_id
        || snapshot.adapter_id != reader.manifest().adapter_id
    {
        return Err(SourceAdapterError::new(
            SourceAdapterErrorKind::SnapshotInconsistent,
            "ready navigation snapshot identity does not match the selected descriptor and reader",
        ));
    }
    if snapshot.revision != binding.revision {
        return Err(SourceAdapterError::new(
            SourceAdapterErrorKind::SnapshotStale,
            "ready navigation snapshot revision differs from the captured session",
        ));
    }
    validate_identity_bearing_navigation(binding, envelope)
}

const MAX_BINDING_VALIDATION_DEPTH: usize = 64;
/// Shared maximum count of typed navigation fields and containers that can
/// carry source or snapshot identity during validation and cache preflight.
pub(crate) const MAX_IDENTITY_BEARING_VALIDATION_ITEMS: usize = 1_000_000;

#[derive(Default)]
pub(crate) struct IdentityValidationBudget {
    items: usize,
}

impl IdentityValidationBudget {
    pub(crate) fn charge(&mut self, count: usize) -> Result<(), SourceAdapterError> {
        self.items = self.items.checked_add(count).ok_or_else(|| {
            SourceAdapterError::new(
                SourceAdapterErrorKind::ResourceLimit,
                "navigation identity-bearing validation item accounting overflow",
            )
        })?;
        if self.items > MAX_IDENTITY_BEARING_VALIDATION_ITEMS {
            return Err(SourceAdapterError::new(
                SourceAdapterErrorKind::ResourceLimit,
                format!(
                    "navigation identity-bearing validation item count {} exceeds limit {}",
                    self.items, MAX_IDENTITY_BEARING_VALIDATION_ITEMS
                ),
            ));
        }
        Ok(())
    }

    #[cfg(test)]
    fn items(&self) -> usize {
        self.items
    }
}

pub(crate) fn validate_identity_bearing_navigation(
    binding: &SourceBinding,
    envelope: &NavigationEnvelope,
) -> Result<(), SourceAdapterError> {
    BindingValidator::new(binding).validate_envelope(envelope)
}

struct BindingValidator<'a> {
    binding: &'a SourceBinding,
    budget: IdentityValidationBudget,
}

impl<'a> BindingValidator<'a> {
    fn new(binding: &'a SourceBinding) -> Self {
        Self {
            binding,
            budget: IdentityValidationBudget::default(),
        }
    }

    fn validate_envelope(
        mut self,
        envelope: &NavigationEnvelope,
    ) -> Result<(), SourceAdapterError> {
        if let Some(root) = &envelope.root {
            self.validate_object_ref(root)?;
        }
        self.charge(envelope.nodes.len())?;
        for node in &envelope.nodes {
            self.validate_node(node)?;
        }
        self.charge(envelope.relations.len())?;
        for page in &envelope.relations {
            self.validate_group_ref(&page.relation)?;
            self.charge(page.items.len())?;
            for item in &page.items {
                self.validate_node(item)?;
            }
            if let Some(cursor) = &page.next_cursor {
                self.validate_cursor(cursor, &page.relation)?;
            }
        }
        self.charge(envelope.relation_index.len())?;
        for relation in &envelope.relation_index {
            self.validate_relation(relation)?;
        }
        Ok(())
    }

    fn charge(&mut self, count: usize) -> Result<(), SourceAdapterError> {
        self.budget.charge(count)
    }

    fn validate_node(
        &mut self,
        node: &crate::domain::navigation::NavigationNode,
    ) -> Result<(), SourceAdapterError> {
        self.validate_object_ref(&node.object_ref)?;
        self.validate_object_ref(&node.reference)?;
        self.charge(node.properties.len())?;
        for property in node.properties.values() {
            if let Some(value) = &property.value {
                self.validate_property_value(value, 0)?;
            }
        }
        self.charge(node.actions.len())?;
        for action in &node.actions {
            if let Some(target) = &action.target {
                self.validate_object_ref(target)?;
            }
            if let Some(relation) = &action.owning_relation {
                self.validate_relation_ref(relation)?;
            }
        }
        Ok(())
    }

    fn validate_relation(
        &mut self,
        relation: &crate::domain::navigation::SemanticRelation,
    ) -> Result<(), SourceAdapterError> {
        self.validate_relation_ref(&relation.relation_ref)?;
        self.validate_group_ref(&relation.group_ref)?;
        self.validate_object_ref(&relation.source)?;
        self.validate_object_ref(&relation.target)
    }

    fn validate_group_ref(
        &self,
        relation: &crate::domain::navigation::RelationGroupRef,
    ) -> Result<(), SourceAdapterError> {
        self.validate_source_id(&relation.source_id)?;
        self.validate_object_ref(&relation.owner)
    }

    fn validate_relation_ref(
        &self,
        relation: &crate::domain::navigation::RelationRef,
    ) -> Result<(), SourceAdapterError> {
        self.validate_source_id(&relation.source_id)
    }

    fn validate_object_ref(
        &self,
        reference: &crate::domain::navigation::ObjectRef,
    ) -> Result<(), SourceAdapterError> {
        self.validate_source_id(&reference.source_id)
    }

    fn validate_source_id(
        &self,
        source_id: &crate::domain::source_adapters::SourceId,
    ) -> Result<(), SourceAdapterError> {
        if source_id != &self.binding.source_id {
            return Err(SourceAdapterError::new(
                SourceAdapterErrorKind::SnapshotInconsistent,
                "ready navigation contains a foreign source reference",
            ));
        }
        Ok(())
    }

    fn validate_cursor(
        &self,
        cursor: &crate::domain::navigation::NavigationCursor,
        group: &crate::domain::navigation::RelationGroupRef,
    ) -> Result<(), SourceAdapterError> {
        self.validate_source_id(&cursor.source_id)?;
        if cursor.snapshot_revision != self.binding.revision {
            return Err(SourceAdapterError::new(
                SourceAdapterErrorKind::SnapshotStale,
                "ready navigation contains a cursor for another snapshot revision",
            ));
        }
        if cursor.target != group.owner.object_key
            || cursor.relation != group.group_key
            || cursor.relation_role != group.role
            || cursor.relation_kind != group.kind
        {
            return Err(SourceAdapterError::new(
                SourceAdapterErrorKind::SnapshotInconsistent,
                "ready navigation cursor does not belong to its relation group",
            ));
        }
        Ok(())
    }

    fn validate_property_value(
        &mut self,
        value: &crate::domain::navigation::PropertyValue,
        depth: usize,
    ) -> Result<(), SourceAdapterError> {
        if depth > MAX_BINDING_VALIDATION_DEPTH {
            return Err(SourceAdapterError::new(
                SourceAdapterErrorKind::ResourceLimit,
                "navigation binding validation exceeds the property nesting limit",
            ));
        }
        use crate::domain::navigation::{PropertyValue, TypeVariant};
        match value {
            PropertyValue::ObjectRef(reference) => self.validate_object_ref(reference),
            PropertyValue::List(values) => {
                self.charge(values.len())?;
                for value in values {
                    self.validate_property_value(value, depth + 1)?;
                }
                Ok(())
            }
            PropertyValue::Structure(values) => {
                self.charge(values.len())?;
                for value in values.values() {
                    self.validate_property_value(value, depth + 1)?;
                }
                Ok(())
            }
            PropertyValue::TypeSet(types) => {
                self.charge(types.variants.len())?;
                for variant in &types.variants {
                    if let TypeVariant::Primitive { qualifiers, .. } = variant {
                        self.charge(qualifiers.len())?;
                        for value in qualifiers.values() {
                            self.validate_property_value(value, depth + 1)?;
                        }
                    }
                }
                Ok(())
            }
            _ => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{
        collections::{BTreeMap, BTreeSet},
        fs,
        path::PathBuf,
        sync::{
            atomic::{AtomicUsize, Ordering},
            Arc,
        },
    };

    use crate::{
        domain::{
            navigation::{
                CapabilityState, IdentityStrength, NavigationCursor, NavigationEnvelope,
                NavigationNode, NavigationRelationPage, NavigationStatus, NodeKind, ObjectKey,
                ObjectRef, PropertyCapability, PropertyProvenance, PropertyType, PropertyValue,
                PropertyValueState, RelationGroupRef, RelationKind, RelationRef, RelationRole,
                SemanticAction, SemanticProperty, SemanticRelation, SourceAdapterDiagnostic,
            },
            source_adapters::{
                AdapterManifest, AdapterMaturity, FormatRange, FormatVersion, SnapshotConsistency,
                SourceAccess, SourceAdapterError, SourceAdapterErrorKind, SourceBinding,
                SourceDescriptor, SourceFamily, SourceId, SourceRevision, SourceSnapshot,
            },
        },
        infrastructure::source_adapters::{
            platform_xml::{
                probe::PlatformXmlProbe, PlatformXmlCaptureAdapter, PlatformXmlReadAdapter,
            },
            CaptureOutcome, CapturedSourceSession, ProbeOutcome, SourceCaptureAdapter, SourceInput,
            SourceProbe, SourceReadAdapter,
        },
    };

    use super::{
        validate_identity_bearing_navigation, BuiltInSourceAdapterRegistry,
        IdentityValidationBudget, MAX_IDENTITY_BEARING_VALIDATION_ITEMS,
    };

    #[test]
    fn built_in_registry_registers_only_the_platform_xml_probe_and_reader() {
        let registry = BuiltInSourceAdapterRegistry::new();

        assert_eq!(registry.capturers.len(), 1);
        assert_eq!(registry.probes.len(), 1);
        assert_eq!(registry.readers.len(), 1);
        assert_eq!(
            registry.readers[0].manifest().adapter_id,
            "platform-xml-2.20"
        );
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

        assert_eq!(
            error.kind,
            crate::domain::source_adapters::SourceAdapterErrorKind::ProbeAmbiguous
        );
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

        assert_eq!(
            error.kind,
            crate::domain::source_adapters::SourceAdapterErrorKind::ProbeAmbiguous
        );
    }

    #[test]
    fn matching_probes_merge_evidence_independently_of_registration_order() {
        let first = registry_with(
            vec![
                probe_with_evidence("2.20", "b.xml"),
                probe_with_evidence("2.20", "a.xml"),
            ],
            vec![reader("xml-2.20", exact("2.20"))],
        );
        let second = registry_with(
            vec![
                probe_with_evidence("2.20", "a.xml"),
                probe_with_evidence("2.20", "b.xml"),
            ],
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
            vec![reader_with(
                "edt-2.20",
                SourceFamily::Edt,
                exact("2.20"),
                [],
                [],
            )],
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

        assert_eq!(
            family_mismatch.inspect(input()).unwrap().status,
            NavigationStatus::Unavailable
        );
        assert_eq!(
            excluded_feature.inspect(input()).unwrap().status,
            NavigationStatus::Unavailable
        );
    }

    #[test]
    fn required_root_properties_feature_selects_only_a_matching_reader() {
        let missing = registry_with(
            vec![probe_match("2.20")],
            vec![reader_with(
                "xml-2.20-properties",
                SourceFamily::PlatformXml,
                exact("2.20"),
                ["structural:root:Properties"],
                [],
            )],
        );
        let present = registry_with(
            vec![probe_with_feature("2.20", "structural:root:Properties")],
            vec![reader_with(
                "xml-2.20-properties",
                SourceFamily::PlatformXml,
                exact("2.20"),
                ["structural:root:Properties"],
                [],
            )],
        );

        assert_eq!(
            missing.inspect(input()).unwrap().status,
            NavigationStatus::Unavailable
        );
        assert_eq!(
            present.inspect(input()).unwrap().status,
            NavigationStatus::Available
        );
    }

    #[test]
    fn conflicting_probes_are_ambiguous() {
        let registry = registry_with(
            vec![probe_match("2.20"), probe_match("2.21")],
            vec![reader("xml-2.20", exact("2.20"))],
        );

        let error = registry.inspect(input()).unwrap_err();

        assert_eq!(
            error.kind,
            crate::domain::source_adapters::SourceAdapterErrorKind::ProbeAmbiguous
        );
    }

    #[test]
    fn selected_reader_error_does_not_retry_a_fallback() {
        let fallback_calls = Arc::new(AtomicUsize::new(0));
        let registry = registry_with(
            vec![probe_match("2.20")],
            vec![
                Box::new(FailingReader {
                    manifest: manifest("xml-exact", exact("2.20")),
                    error: SourceAdapterError::new(
                        crate::domain::source_adapters::SourceAdapterErrorKind::DecodeCorrupted,
                        "corrupt XML",
                    ),
                }),
                Box::new(CountingReader {
                    manifest: manifest("xml-broad", range("2.0", "2.30")),
                    calls: Arc::clone(&fallback_calls),
                }),
            ],
        );

        let error = registry.inspect(input()).unwrap_err();

        assert_eq!(
            error.kind,
            crate::domain::source_adapters::SourceAdapterErrorKind::DecodeCorrupted
        );
        assert_eq!(fallback_calls.load(Ordering::SeqCst), 0);
    }

    fn registry_with(
        probes: Vec<Box<dyn SourceProbe>>,
        readers: Vec<Box<dyn SourceReadAdapter>>,
    ) -> BuiltInSourceAdapterRegistry {
        BuiltInSourceAdapterRegistry::with_adapters(
            vec![Box::new(FakeCapture {
                session: fake_session(SourceFamily::PlatformXml, None),
            })],
            probes,
            readers,
        )
    }

    fn input() -> SourceInput {
        SourceInput {
            workspace_root: PathBuf::from("/workspace"),
            source_root: PathBuf::from("/workspace"),
            target: PathBuf::from("/workspace/Configuration.xml"),
            configured_source_set: None,
            declared_family: SourceFamily::PlatformXml,
            declared_format: None,
        }
    }

    fn probe_match(version: &str) -> Box<dyn SourceProbe> {
        Box::new(FakeProbe {
            descriptor: descriptor(version),
        })
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
            manifest: manifest_with(
                adapter_id,
                source_family,
                range,
                required_features,
                excluded_features,
            ),
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
            snapshot_evidence: Some(crate::domain::source_adapters::SnapshotEvidence {
                revision: SourceRevision::new("sha256:fake-session").unwrap(),
                root_descriptor_digest: "sha256:fixture".to_string(),
            }),
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
        fn source_family(&self) -> SourceFamily {
            self.descriptor.family.clone()
        }

        fn probe(
            &self,
            _input: &SourceInput,
            _session: &dyn CapturedSourceSession,
        ) -> Result<ProbeOutcome, SourceAdapterError> {
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

        fn inspect_captured(
            &self,
            session: &dyn CapturedSourceSession,
            descriptor: &SourceDescriptor,
        ) -> Result<NavigationEnvelope, SourceAdapterError> {
            Ok(NavigationEnvelope {
                schema_version: "1".to_string(),
                status: NavigationStatus::Available,
                snapshot: Some(SourceSnapshot {
                    source_id: descriptor.source_id.clone(),
                    revision: session.revision()?,
                    consistency: SnapshotConsistency::Consistent,
                    adapter_id: self.manifest.adapter_id.to_string(),
                }),
                root: None,
                nodes: Vec::new(),
                relations: Vec::new(),
                diagnostics: vec![SourceAdapterDiagnostic {
                    code: "probe_evidence".to_string(),
                    message: descriptor.probe_evidence.join(","),
                    details: None,
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

        fn inspect_captured(
            &self,
            _session: &dyn CapturedSourceSession,
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

        fn inspect_captured(
            &self,
            _session: &dyn CapturedSourceSession,
            _descriptor: &SourceDescriptor,
        ) -> Result<NavigationEnvelope, SourceAdapterError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Err(SourceAdapterError::new(
                crate::domain::source_adapters::SourceAdapterErrorKind::DecodeCorrupted,
                "fallback must not run",
            ))
        }
    }

    #[derive(Clone)]
    struct FakeSession {
        binding: SourceBinding,
    }

    fn fake_session(family: SourceFamily, format: Option<FormatVersion>) -> FakeSession {
        FakeSession {
            binding: SourceBinding::new(
                SourceId::new("workspace:main").unwrap(),
                family,
                format,
                crate::domain::source_adapters::TargetIdentity::from_normalized_relative_path(
                    "Configuration.xml",
                )
                .unwrap(),
                SourceRevision::new("sha256:fake-session").unwrap(),
            ),
        }
    }

    impl CapturedSourceSession for FakeSession {
        fn binding(&self) -> &SourceBinding {
            &self.binding
        }

        fn evidence(&self) -> &[String] {
            &[]
        }

        fn as_any(&self) -> &dyn std::any::Any {
            self
        }
    }

    struct FakeCapture {
        session: FakeSession,
    }

    impl SourceCaptureAdapter for FakeCapture {
        fn source_family(&self) -> SourceFamily {
            self.session.binding.family.clone()
        }

        fn capture(&self, _input: &SourceInput) -> Result<CaptureOutcome, SourceAdapterError> {
            Ok(CaptureOutcome::Captured(Box::new(self.session.clone())))
        }
    }

    #[test]
    fn captured_second_family_uses_the_common_registry_path_without_platform_branch() {
        let mut descriptor = descriptor("2.20");
        descriptor.family = SourceFamily::Cf;
        let registry = BuiltInSourceAdapterRegistry::with_adapters(
            vec![Box::new(FakeCapture {
                session: fake_session(SourceFamily::Cf, None),
            })],
            vec![Box::new(FakeProbe { descriptor })],
            vec![reader_with(
                "fake-cf",
                SourceFamily::Cf,
                exact("2.20"),
                [],
                [],
            )],
        );

        let navigation = registry
            .inspect(SourceInput {
                declared_family: SourceFamily::Cf,
                ..input()
            })
            .expect("second family is inspectable");

        assert_eq!(navigation.snapshot.expect("snapshot").adapter_id, "fake-cf");
    }

    #[test]
    fn builtins_and_a_second_family_coexist_without_platform_ambiguity() {
        let mut cf_descriptor = descriptor("2.20");
        cf_descriptor.family = SourceFamily::Cf;
        let registry = BuiltInSourceAdapterRegistry::with_adapters(
            vec![
                Box::new(PlatformXmlCaptureAdapter::new()),
                Box::new(FakeCapture {
                    session: fake_session(SourceFamily::Cf, None),
                }),
            ],
            vec![
                Box::new(PlatformXmlProbe::new()),
                Box::new(FakeProbe {
                    descriptor: cf_descriptor,
                }),
            ],
            vec![
                Box::new(PlatformXmlReadAdapter::new()),
                reader_with("fake-cf", SourceFamily::Cf, exact("2.20"), [], []),
            ],
        );

        let cf = registry
            .inspect(SourceInput {
                declared_family: SourceFamily::Cf,
                ..input()
            })
            .expect("CF source is selected through the common registry");
        assert_eq!(cf.snapshot.expect("CF snapshot").adapter_id, "fake-cf");

        let root = std::env::temp_dir().join(format!(
            "unica-source-adapter-coexist-{}-{}",
            std::process::id(),
            AtomicUsize::new(0).fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&root).unwrap();
        let target = root.join("Configuration.xml");
        fs::write(
            &target,
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Configuration uuid="11111111-1111-1111-1111-111111111111"><Properties><Name>Main</Name></Properties></Configuration></MetaDataObject>"#,
        )
        .unwrap();
        let platform = registry
            .inspect(SourceInput {
                workspace_root: root.clone(),
                source_root: root.clone(),
                target,
                configured_source_set: Some("main".to_string()),
                declared_family: SourceFamily::PlatformXml,
                declared_format: None,
            })
            .expect("Platform XML source is selected with the same registry");
        assert_eq!(
            platform.snapshot.expect("Platform snapshot").adapter_id,
            "platform-xml-2.20"
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn stale_reader_snapshot_revision_is_rejected() {
        let registry = registry_with(
            vec![probe_match("2.20")],
            vec![Box::new(StaleReader {
                manifest: manifest("xml-2.20", exact("2.20")),
            })],
        );

        let error = registry.inspect(input()).unwrap_err();

        assert_eq!(error.kind, SourceAdapterErrorKind::SnapshotStale);
    }

    #[test]
    fn foreign_reader_object_reference_is_rejected() {
        let registry = registry_with(
            vec![probe_match("2.20")],
            vec![Box::new(ForeignReferenceReader {
                manifest: manifest("xml-2.20", exact("2.20")),
            })],
        );

        let error = registry.inspect(input()).unwrap_err();

        assert_eq!(error.kind, SourceAdapterErrorKind::SnapshotInconsistent);
    }

    #[test]
    fn typed_identity_fields_fail_closed_without_inspecting_ordinary_data_keys() {
        for case in [
            BindingReaderCase::Node,
            BindingReaderCase::RelationGroup,
            BindingReaderCase::RelationIndex,
            BindingReaderCase::RelationItem,
            BindingReaderCase::CursorSource,
            BindingReaderCase::CursorRevision,
            BindingReaderCase::CursorOwner,
            BindingReaderCase::CursorGroup,
            BindingReaderCase::NestedObjectRef,
            BindingReaderCase::ActionTarget,
        ] {
            let error = registry_with(
                vec![probe_match("2.20")],
                vec![Box::new(BindingTestReader {
                    manifest: manifest("xml-2.20", exact("2.20")),
                    case: Some(case),
                })],
            )
            .inspect(input())
            .unwrap_err();
            assert_eq!(
                error.kind,
                if matches!(case, BindingReaderCase::CursorRevision) {
                    SourceAdapterErrorKind::SnapshotStale
                } else {
                    SourceAdapterErrorKind::SnapshotInconsistent
                },
                "case {case:?}",
            );
        }

        let navigation = registry_with(
            vec![probe_match("2.20")],
            vec![Box::new(BindingTestReader {
                manifest: manifest("xml-2.20", exact("2.20")),
                case: None,
            })],
        )
        .inspect(input())
        .expect("ordinary property and diagnostic keys are not semantic references");
        assert_eq!(navigation.status, NavigationStatus::Available);
    }

    #[test]
    fn typed_identity_validation_accepts_a_twenty_five_thousand_node_graph() {
        let session = fake_session(SourceFamily::PlatformXml, None);
        let binding = &session.binding;
        let property = structure_property(PropertyValue::Null);
        let mut nodes = Vec::with_capacity(25_000);
        for index in 0..25_000 {
            let mut node = bound_node(bound_reference(binding, &format!("ordinary-{index}")));
            node.properties = BTreeMap::from([
                ("first".to_string(), property.clone()),
                ("second".to_string(), property.clone()),
            ]);
            nodes.push(node);
        }
        let envelope = NavigationEnvelope {
            schema_version: "1".to_string(),
            status: NavigationStatus::Available,
            snapshot: None,
            root: None,
            nodes,
            relations: Vec::new(),
            diagnostics: Vec::new(),
            relation_index: Vec::new(),
        };

        validate_identity_bearing_navigation(binding, &envelope).unwrap();
    }

    #[test]
    fn identity_validation_limit_reports_the_checked_item_count() {
        let mut budget = IdentityValidationBudget::default();
        let expected = MAX_IDENTITY_BEARING_VALIDATION_ITEMS
            .checked_add(1)
            .unwrap();

        let error = budget.charge(expected).unwrap_err();

        assert_eq!(budget.items(), expected);
        assert_eq!(error.kind, SourceAdapterErrorKind::ResourceLimit);
        assert!(error.message.contains(&expected.to_string()));
    }

    #[test]
    fn descriptor_format_must_match_a_pinned_captured_session() {
        let registry = BuiltInSourceAdapterRegistry::with_adapters(
            vec![Box::new(FakeCapture {
                session: fake_session(
                    SourceFamily::PlatformXml,
                    Some(FormatVersion::parse("2.20").unwrap()),
                ),
            })],
            vec![probe_match("2.19")],
            vec![reader("xml-2.19", exact("2.19"))],
        );

        let error = registry.inspect(input()).unwrap_err();

        assert_eq!(error.kind, SourceAdapterErrorKind::SnapshotInconsistent);
    }

    struct StaleReader {
        manifest: AdapterManifest,
    }

    struct ForeignReferenceReader {
        manifest: AdapterManifest,
    }

    #[derive(Debug, Clone, Copy)]
    enum BindingReaderCase {
        Node,
        RelationGroup,
        RelationIndex,
        RelationItem,
        CursorSource,
        CursorRevision,
        CursorOwner,
        CursorGroup,
        NestedObjectRef,
        ActionTarget,
    }

    struct BindingTestReader {
        manifest: AdapterManifest,
        case: Option<BindingReaderCase>,
    }

    #[test]
    fn probes_with_foreign_source_id_or_revision_fail_closed() {
        let mut foreign_source = descriptor("2.20");
        foreign_source.source_id = SourceId::new("workspace:foreign").unwrap();
        let mut foreign_revision = descriptor("2.20");
        foreign_revision
            .snapshot_evidence
            .as_mut()
            .unwrap()
            .revision = SourceRevision::new("sha256:foreign").unwrap();
        for descriptor in [foreign_source, foreign_revision] {
            let error = registry_with(
                vec![Box::new(FakeProbe { descriptor })],
                vec![reader("xml", exact("2.20"))],
            )
            .inspect(input())
            .unwrap_err();
            assert!(matches!(
                error.kind,
                SourceAdapterErrorKind::SnapshotInconsistent
                    | SourceAdapterErrorKind::SnapshotStale
            ));
        }
    }

    impl SourceReadAdapter for StaleReader {
        fn manifest(&self) -> &AdapterManifest {
            &self.manifest
        }

        fn inspect_captured(
            &self,
            _session: &dyn CapturedSourceSession,
            descriptor: &SourceDescriptor,
        ) -> Result<NavigationEnvelope, SourceAdapterError> {
            Ok(NavigationEnvelope {
                schema_version: "1".to_string(),
                status: NavigationStatus::Available,
                snapshot: Some(SourceSnapshot {
                    source_id: descriptor.source_id.clone(),
                    revision: SourceRevision::new("sha256:stale-reader").unwrap(),
                    consistency: SnapshotConsistency::Consistent,
                    adapter_id: self.manifest.adapter_id.to_string(),
                }),
                root: None,
                nodes: Vec::new(),
                relations: Vec::new(),
                diagnostics: Vec::new(),
                relation_index: Vec::new(),
            })
        }
    }

    impl SourceReadAdapter for ForeignReferenceReader {
        fn manifest(&self) -> &AdapterManifest {
            &self.manifest
        }

        fn inspect_captured(
            &self,
            session: &dyn CapturedSourceSession,
            descriptor: &SourceDescriptor,
        ) -> Result<NavigationEnvelope, SourceAdapterError> {
            Ok(NavigationEnvelope {
                schema_version: "1".to_string(),
                status: NavigationStatus::Available,
                snapshot: Some(SourceSnapshot {
                    source_id: descriptor.source_id.clone(),
                    revision: session.binding().revision.clone(),
                    consistency: SnapshotConsistency::Consistent,
                    adapter_id: self.manifest.adapter_id.to_string(),
                }),
                root: Some(ObjectRef::new(
                    SourceId::new("workspace:foreign").unwrap(),
                    ObjectKey::new("foreign-root").unwrap(),
                    IdentityStrength::Persistent,
                    NodeKind::Document,
                    "Foreign",
                )),
                nodes: Vec::new(),
                relations: Vec::new(),
                diagnostics: Vec::new(),
                relation_index: Vec::new(),
            })
        }
    }

    impl SourceReadAdapter for BindingTestReader {
        fn manifest(&self) -> &AdapterManifest {
            &self.manifest
        }

        fn inspect_captured(
            &self,
            session: &dyn CapturedSourceSession,
            descriptor: &SourceDescriptor,
        ) -> Result<NavigationEnvelope, SourceAdapterError> {
            let binding = session.binding();
            let root = bound_reference(binding, "binding-root");
            let mut envelope = NavigationEnvelope {
                schema_version: "1".to_string(),
                status: NavigationStatus::Available,
                snapshot: Some(SourceSnapshot {
                    source_id: descriptor.source_id.clone(),
                    revision: binding.revision.clone(),
                    consistency: SnapshotConsistency::Consistent,
                    adapter_id: self.manifest.adapter_id.to_string(),
                }),
                root: Some(root.clone()),
                nodes: vec![bound_node(root.clone())],
                relations: Vec::new(),
                diagnostics: Vec::new(),
                relation_index: Vec::new(),
            };
            match self.case {
                Some(BindingReaderCase::Node) => {
                    envelope.nodes[0].reference = foreign_reference("foreign-node");
                }
                Some(BindingReaderCase::RelationGroup) => {
                    let mut relation = bound_group(binding, root.clone());
                    relation.source_id = SourceId::new("workspace:foreign").unwrap();
                    envelope.relations.push(NavigationRelationPage {
                        relation,
                        items: Vec::new(),
                        next_cursor: None,
                    });
                }
                Some(BindingReaderCase::RelationIndex) => {
                    let group = bound_group(binding, root.clone());
                    envelope.relation_index.push(SemanticRelation {
                        relation_ref: RelationRef::new(
                            SourceId::new("workspace:foreign").unwrap(),
                            "foreign-relation",
                            RelationKind::Contains,
                        )
                        .unwrap(),
                        group_ref: group,
                        identity_strength: IdentityStrength::Persistent,
                        kind: RelationKind::Contains,
                        role: RelationRole::Attributes,
                        source: root.clone(),
                        target: root,
                        capability: envelope.nodes[0].capability.clone(),
                    });
                }
                Some(BindingReaderCase::RelationItem) => {
                    envelope.relations.push(NavigationRelationPage {
                        relation: bound_group(binding, root),
                        items: vec![bound_node(foreign_reference("foreign-item"))],
                        next_cursor: None,
                    });
                }
                Some(BindingReaderCase::CursorSource)
                | Some(BindingReaderCase::CursorRevision)
                | Some(BindingReaderCase::CursorOwner)
                | Some(BindingReaderCase::CursorGroup) => {
                    let relation = bound_group(binding, root);
                    let mut cursor = bound_cursor(binding, &relation);
                    match self.case.unwrap() {
                        BindingReaderCase::CursorSource => {
                            cursor.source_id = SourceId::new("workspace:foreign").unwrap();
                        }
                        BindingReaderCase::CursorRevision => {
                            cursor.snapshot_revision =
                                SourceRevision::new("sha256:foreign-cursor").unwrap();
                        }
                        BindingReaderCase::CursorOwner => {
                            cursor.target = ObjectKey::new("foreign-owner").unwrap();
                        }
                        BindingReaderCase::CursorGroup => {
                            cursor.relation = RelationRef::new(
                                binding.source_id.clone(),
                                "foreign-group",
                                RelationKind::Contains,
                            )
                            .unwrap()
                            .relation_key;
                        }
                        _ => unreachable!(),
                    }
                    envelope.relations.push(NavigationRelationPage {
                        relation,
                        items: Vec::new(),
                        next_cursor: Some(cursor),
                    });
                }
                Some(BindingReaderCase::NestedObjectRef) => {
                    envelope.nodes[0].properties.insert(
                        "nested".to_string(),
                        structure_property(PropertyValue::Structure(BTreeMap::from([(
                            "nested".to_string(),
                            PropertyValue::List(vec![PropertyValue::ObjectRef(foreign_reference(
                                "foreign-property",
                            ))]),
                        )]))),
                    );
                }
                Some(BindingReaderCase::ActionTarget) => {
                    envelope.nodes[0]
                        .actions
                        .push(SemanticAction::modeled_clone(
                            foreign_reference("foreign-action"),
                            None,
                        ));
                }
                None => {
                    envelope.nodes[0].properties.insert(
                        "ordinary".to_string(),
                        structure_property(PropertyValue::Structure(BTreeMap::from([
                            (
                                "sourceId".to_string(),
                                PropertyValue::String("workspace:foreign".to_string()),
                            ),
                            (
                                "snapshotRevision".to_string(),
                                PropertyValue::String("sha256:foreign".to_string()),
                            ),
                        ]))),
                    );
                    envelope.diagnostics.push(SourceAdapterDiagnostic {
                        code: "ordinary_data".to_string(),
                        message: "ordinary keys are not references".to_string(),
                        details: Some(serde_json::json!({
                            "sourceId": "workspace:foreign",
                            "snapshotRevision": "sha256:foreign",
                        })),
                    });
                }
            }
            Ok(envelope)
        }
    }

    fn bound_reference(binding: &SourceBinding, key: &str) -> ObjectRef {
        ObjectRef::new(
            binding.source_id.clone(),
            ObjectKey::new(key).unwrap(),
            IdentityStrength::Persistent,
            NodeKind::Document,
            "Bound",
        )
    }

    fn foreign_reference(key: &str) -> ObjectRef {
        ObjectRef::new(
            SourceId::new("workspace:foreign").unwrap(),
            ObjectKey::new(key).unwrap(),
            IdentityStrength::Persistent,
            NodeKind::Document,
            "Foreign",
        )
    }

    fn bound_node(reference: ObjectRef) -> NavigationNode {
        NavigationNode::new(reference, CapabilityState::resolved_authorable())
    }

    fn bound_group(binding: &SourceBinding, owner: ObjectRef) -> RelationGroupRef {
        RelationGroupRef::new(
            binding.source_id.clone(),
            owner,
            RelationRole::Attributes,
            RelationKind::Contains,
        )
        .unwrap()
    }

    fn bound_cursor(binding: &SourceBinding, group: &RelationGroupRef) -> NavigationCursor {
        NavigationCursor::issue(
            b"binding-test-cursor",
            binding.source_id.clone(),
            binding.revision.clone(),
            group.owner.object_key.clone(),
            group.clone(),
            crate::domain::navigation::NavigationSelection {
                properties: crate::domain::navigation::PropertySelection::All,
                facets: crate::domain::navigation::FacetSelection::Full,
                relations: Vec::new(),
            },
            1,
        )
        .unwrap()
    }

    fn structure_property(value: PropertyValue) -> SemanticProperty {
        SemanticProperty {
            value_type: PropertyType::Structure,
            value_state: PropertyValueState::Explicit,
            value: Some(value),
            provenance: PropertyProvenance::Descriptor,
            capability: PropertyCapability::ReadOnly,
        }
    }
}
