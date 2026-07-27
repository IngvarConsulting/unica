use std::{collections::BTreeSet, sync::Arc};

use unica_adapter_platform_xml::PlatformXmlAdapterFactory;
use unica_format_core::{
    navigation::{
        FacetSelection, NavigationEnvelope, NavigationQuery, NavigationSelection, NavigationStatus,
        NavigationTarget, PropertySelection,
    },
    ports::{
        CapturePort, CaptureResult, CapturedSource, FormatReadRequest, ProbePort, ProbeResult,
        ReadPort, SourceAdapterRegistration,
    },
    source::{
        AdapterManifest, SourceAdapterError, SourceAdapterErrorKind, SourceBinding,
        SourceDescriptor, SourceFamily,
    },
};

use crate::infrastructure::source_adapters::SourceInput;

struct RegisteredCapture {
    family: SourceFamily,
    port: Arc<dyn CapturePort>,
}

struct RegisteredProbe {
    family: SourceFamily,
    port: Arc<dyn ProbePort>,
}

struct RegisteredReader {
    manifest: AdapterManifest,
    port: Arc<dyn ReadPort>,
}

pub(crate) struct BuiltInSourceAdapterRegistry {
    capturers: Vec<RegisteredCapture>,
    probes: Vec<RegisteredProbe>,
    readers: Vec<RegisteredReader>,
}

impl BuiltInSourceAdapterRegistry {
    pub(crate) fn new() -> Self {
        Self::with_registrations(vec![PlatformXmlAdapterFactory::new().registration()])
    }

    fn with_registrations(registrations: Vec<SourceAdapterRegistration>) -> Self {
        let mut capturers = Vec::new();
        let mut probes = Vec::new();
        let mut readers = Vec::new();
        for registration in registrations {
            let family = registration.manifest.source_family.clone();
            capturers.push(RegisteredCapture {
                family: family.clone(),
                port: registration.capture,
            });
            probes.push(RegisteredProbe {
                family,
                port: registration.probe,
            });
            readers.push(RegisteredReader {
                manifest: registration.manifest,
                port: registration.read,
            });
        }
        Self {
            capturers,
            probes,
            readers,
        }
    }

    #[cfg(test)]
    fn with_ports(
        capturers: Vec<(SourceFamily, Arc<dyn CapturePort>)>,
        probes: Vec<(SourceFamily, Arc<dyn ProbePort>)>,
        readers: Vec<(AdapterManifest, Arc<dyn ReadPort>)>,
    ) -> Self {
        Self {
            capturers: capturers
                .into_iter()
                .map(|(family, port)| RegisteredCapture { family, port })
                .collect(),
            probes: probes
                .into_iter()
                .map(|(family, port)| RegisteredProbe { family, port })
                .collect(),
            readers: readers
                .into_iter()
                .map(|(manifest, port)| RegisteredReader { manifest, port })
                .collect(),
        }
    }

    #[cfg(test)]
    fn inspect(&self, input: SourceInput) -> Result<NavigationEnvelope, SourceAdapterError> {
        let session = self.capture(&input)?;
        self.inspect_captured(&input, &session)
    }

    pub(crate) fn capture(
        &self,
        input: &SourceInput,
    ) -> Result<CapturedSource, SourceAdapterError> {
        let source = input.source_context();
        let mut sessions = Vec::new();
        for capture in self
            .capturers
            .iter()
            .filter(|capture| capture.family == input.declared_family)
        {
            match capture.port.capture(&source)? {
                CaptureResult::NoMatch => {}
                CaptureResult::Captured(session) => sessions.push(session),
            }
        }
        match sessions.len() {
            0 => Err(SourceAdapterError::new(
                SourceAdapterErrorKind::SourceUnavailable,
                "no source capture adapter recognized the target",
            )),
            1 => Ok(sessions.pop().expect("one captured session")),
            _ => Err(SourceAdapterError::new(
                SourceAdapterErrorKind::ProbeAmbiguous,
                "multiple source capture adapters recognized the target",
            )),
        }
    }

    pub(crate) fn inspect_captured(
        &self,
        input: &SourceInput,
        session: &CapturedSource,
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
        let envelope = reader.port.read(&FormatReadRequest {
            captured: session.clone(),
            query: NavigationQuery {
                target: NavigationTarget::CapturedTarget(session.binding().target_identity.clone()),
                select: NavigationSelection {
                    properties: PropertySelection::All,
                    facets: FacetSelection::Full,
                    relations: Vec::new(),
                },
            },
        })?;
        validate_ready_envelope(&envelope, session, &reader.manifest)?;
        Ok(envelope)
    }

    fn probe(
        &self,
        input: &SourceInput,
        session: &CapturedSource,
    ) -> Result<SourceDescriptor, SourceAdapterError> {
        let mut matches = Vec::new();
        for probe in self
            .probes
            .iter()
            .filter(|probe| probe.family == input.declared_family)
        {
            if let ProbeResult::Match(descriptor) = probe.port.probe(session)? {
                validate_probe_descriptor(input, session, &descriptor)?;
                matches.push(descriptor);
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
            .flat_map(|descriptor| descriptor.probe_evidence.iter().cloned())
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
                let compatible = reader.manifest.source_family == descriptor.family
                    && reader
                        .manifest
                        .required_features
                        .is_subset(&descriptor.detected_features)
                    && reader
                        .manifest
                        .excluded_features
                        .is_disjoint(&descriptor.detected_features);
                reader
                    .manifest
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
    ) -> Result<Option<&'a RegisteredReader>, SourceAdapterError> {
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
            [reader_index] => Ok(Some(&self.readers[*reader_index])),
            _ => Err(SourceAdapterError::new(
                SourceAdapterErrorKind::ProbeAmbiguous,
                "multiple readers have equally narrow compatible format ranges",
            )),
        }
    }
}

struct Candidate<'a> {
    reader_index: usize,
    range: &'a unica_format_core::source::FormatRange,
}

fn range_is_no_wider_than(
    left: &unica_format_core::source::FormatRange,
    right: &unica_format_core::source::FormatRange,
) -> bool {
    left.min_inclusive >= right.min_inclusive && left.max_inclusive <= right.max_inclusive
}

fn range_is_strictly_narrower(
    left: &unica_format_core::source::FormatRange,
    right: &unica_format_core::source::FormatRange,
) -> bool {
    range_is_no_wider_than(left, right)
        && (left.min_inclusive != right.min_inclusive || left.max_inclusive != right.max_inclusive)
}

fn same_descriptor(left: &SourceDescriptor, right: &SourceDescriptor) -> bool {
    left.source_id == right.source_id
        && left.family == right.family
        && left.format_version == right.format_version
        && left.producer_version == right.producer_version
        && left.detected_features == right.detected_features
        && left.snapshot_evidence == right.snapshot_evidence
}

fn validate_probe_descriptor(
    input: &SourceInput,
    session: &CapturedSource,
    descriptor: &SourceDescriptor,
) -> Result<(), SourceAdapterError> {
    if descriptor.family != input.declared_family
        || descriptor.source_id != session.binding().source_id
    {
        return Err(SourceAdapterError::new(
            SourceAdapterErrorKind::SnapshotInconsistent,
            "source probe descriptor does not match the captured source",
        ));
    }
    if let Some(format) = session.binding().format.as_ref() {
        if descriptor.format_version != *format {
            return Err(SourceAdapterError::new(
                SourceAdapterErrorKind::SnapshotInconsistent,
                "source probe descriptor format does not match the declared source",
            ));
        }
    }
    let evidence = descriptor.snapshot_evidence.as_ref().ok_or_else(|| {
        SourceAdapterError::new(
            SourceAdapterErrorKind::SnapshotInconsistent,
            "source probe descriptor has no immutable snapshot evidence",
        )
    })?;
    if evidence.revision != session.binding().revision {
        return Err(SourceAdapterError::new(
            SourceAdapterErrorKind::SnapshotStale,
            "source probe descriptor revision differs from the captured source",
        ));
    }
    Ok(())
}

fn validate_ready_envelope(
    envelope: &NavigationEnvelope,
    session: &CapturedSource,
    manifest: &AdapterManifest,
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
    if snapshot.source_id != session.binding().source_id
        || snapshot.adapter_id != manifest.adapter_id
    {
        return Err(SourceAdapterError::new(
            SourceAdapterErrorKind::SnapshotInconsistent,
            "ready navigation snapshot identity does not match the selected reader",
        ));
    }
    if snapshot.revision != session.binding().revision {
        return Err(SourceAdapterError::new(
            SourceAdapterErrorKind::SnapshotStale,
            "ready navigation snapshot differs from the captured source",
        ));
    }
    validate_identity_bearing_navigation(session.binding(), envelope)
}
const MAX_BINDING_VALIDATION_DEPTH: usize =
    crate::domain::navigation_limits::MAX_NAVIGATION_NESTING_DEPTH;
/// Shared maximum count of typed navigation fields and containers that can
/// carry source or snapshot identity during validation and cache preflight.
pub(crate) const MAX_IDENTITY_BEARING_VALIDATION_ITEMS: usize =
    crate::domain::navigation_limits::MAX_NAVIGATION_IDENTITY_ITEMS;

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
        for relation in envelope.relation_index.iter() {
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
        if cursor.target_identity != self.binding.target_identity {
            return Err(SourceAdapterError::new(
                SourceAdapterErrorKind::SnapshotStale,
                "ready navigation cursor belongs to another captured target",
            ));
        }
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
mod registry_tests {
    use super::*;
    use std::{
        any::Any,
        sync::atomic::{AtomicUsize, Ordering},
    };
    use unica_format_core::{
        navigation::{
            CapabilityState, IdentityStrength, NavigationCursor, NavigationNode,
            NavigationRelationPage, NodeKind, ObjectKey, ObjectRef, PropertyCapability,
            PropertyProvenance, PropertyType, PropertyValue, PropertyValueState, RelationGroupRef,
            RelationKey, RelationKind, RelationRef, RelationRole, SemanticAction, SemanticProperty,
            SemanticRelation, SourceAdapterDiagnostic,
        },
        source::{
            AdapterMaturity, FormatRange, FormatVersion, SnapshotConsistency, SnapshotEvidence,
            SourceAccess, SourceId, SourceRevision, SourceSnapshot, TargetIdentity,
        },
    };

    struct StubCapture;

    struct StubCapturedSession {
        source: unica_format_core::source::SourceContext,
        snapshot: SourceSnapshot,
        binding: SourceBinding,
    }

    impl unica_format_core::ports::CapturedSourceSession for StubCapturedSession {
        fn source(&self) -> &unica_format_core::source::SourceContext {
            &self.source
        }

        fn snapshot(&self) -> &SourceSnapshot {
            &self.snapshot
        }

        fn binding(&self) -> &SourceBinding {
            &self.binding
        }

        fn as_any(&self) -> &dyn Any {
            self
        }
    }

    impl CapturePort for StubCapture {
        fn capture(
            &self,
            source: &unica_format_core::source::SourceContext,
        ) -> Result<CaptureResult, SourceAdapterError> {
            let snapshot = snapshot("capture");
            let binding = SourceBinding::new(
                snapshot.source_id.clone(),
                SourceFamily::PlatformXml,
                None,
                unica_format_core::source::TargetIdentity::from_normalized_relative_path(
                    "Demo.xml",
                )?,
                snapshot.revision.clone(),
            );
            Ok(CaptureResult::Captured(CapturedSource::new(
                StubCapturedSession {
                    source: source.clone(),
                    snapshot,
                    binding,
                },
            )))
        }
    }

    struct StubProbe {
        version: &'static str,
        evidence: &'static str,
    }

    impl ProbePort for StubProbe {
        fn probe(&self, captured: &CapturedSource) -> Result<ProbeResult, SourceAdapterError> {
            let snapshot = captured.snapshot().clone();
            Ok(ProbeResult::Match(SourceDescriptor {
                source_id: snapshot.source_id,
                family: SourceFamily::PlatformXml,
                format_version: FormatVersion::parse(self.version)?,
                producer_version: None,
                detected_features: BTreeSet::new(),
                probe_evidence: vec![self.evidence.to_string()],
                snapshot_evidence: Some(SnapshotEvidence {
                    revision: snapshot.revision,
                    root_descriptor_digest: "sha256:root".to_string(),
                }),
            }))
        }
    }

    struct StubRead {
        marker: &'static str,
        calls: Arc<AtomicUsize>,
        fail: bool,
    }

    impl ReadPort for StubRead {
        fn read(
            &self,
            _request: &FormatReadRequest,
        ) -> Result<NavigationEnvelope, SourceAdapterError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            if self.fail {
                return Err(SourceAdapterError::new(
                    SourceAdapterErrorKind::DecodeCorrupted,
                    self.marker,
                ));
            }
            Ok(NavigationEnvelope::unavailable(SourceAdapterError::new(
                SourceAdapterErrorKind::FormatUnsupported,
                self.marker,
            )))
        }
    }

    #[test]
    fn exact_reader_wins_over_a_broader_compatible_reader() {
        let broad_calls = Arc::new(AtomicUsize::new(0));
        let exact_calls = Arc::new(AtomicUsize::new(0));
        let registry = registry_with(
            vec![
                reader("broad", "2.0", "3.0", broad_calls.clone(), false),
                reader("exact", "2.20", "2.20", exact_calls.clone(), false),
            ],
            vec![probe("2.20", "root")],
        );

        let envelope = registry.inspect(input()).unwrap();

        assert_eq!(envelope.diagnostics[0].message, "exact");
        assert_eq!(exact_calls.load(Ordering::SeqCst), 1);
        assert_eq!(broad_calls.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn nearest_reader_is_never_selected() {
        let calls = Arc::new(AtomicUsize::new(0));
        let registry = registry_with(
            vec![reader("older", "2.19", "2.19", calls.clone(), false)],
            vec![probe("2.20", "root")],
        );

        let envelope = registry.inspect(input()).unwrap();

        assert_eq!(envelope.status, NavigationStatus::Unavailable);
        assert!(envelope.diagnostics[0]
            .message
            .contains("no reader supports"));
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn equally_narrow_readers_are_ambiguous() {
        let registry = registry_with(
            vec![
                reader(
                    "first",
                    "2.20",
                    "2.20",
                    Arc::new(AtomicUsize::new(0)),
                    false,
                ),
                reader(
                    "second",
                    "2.20",
                    "2.20",
                    Arc::new(AtomicUsize::new(0)),
                    false,
                ),
            ],
            vec![probe("2.20", "root")],
        );

        let error = registry.inspect(input()).unwrap_err();

        assert_eq!(error.kind, SourceAdapterErrorKind::ProbeAmbiguous);
    }

    #[test]
    fn selected_reader_error_does_not_fall_back() {
        let broad_calls = Arc::new(AtomicUsize::new(0));
        let exact_calls = Arc::new(AtomicUsize::new(0));
        let registry = registry_with(
            vec![
                reader("broad", "2.0", "3.0", broad_calls.clone(), false),
                reader(
                    "selected failure",
                    "2.20",
                    "2.20",
                    exact_calls.clone(),
                    true,
                ),
            ],
            vec![probe("2.20", "root")],
        );

        let error = registry.inspect(input()).unwrap_err();

        assert_eq!(error.message, "selected failure");
        assert_eq!(exact_calls.load(Ordering::SeqCst), 1);
        assert_eq!(broad_calls.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn agreeing_probes_merge_deterministic_evidence() {
        let registry = registry_with(
            vec![reader(
                "reader",
                "2.20",
                "2.20",
                Arc::new(AtomicUsize::new(0)),
                false,
            )],
            vec![probe("2.20", "zeta"), probe("2.20", "alpha")],
        );
        let input = input();
        let session = registry.capture(&input).unwrap();

        let descriptor = registry.probe(&input, &session).unwrap();

        assert_eq!(descriptor.probe_evidence, vec!["alpha", "zeta"]);
    }

    #[test]
    fn continuation_revision_is_validated_against_the_capture() {
        let snapshot = snapshot("capture");
        let binding = SourceBinding::new(
            snapshot.source_id.clone(),
            SourceFamily::PlatformXml,
            None,
            TargetIdentity::from_normalized_relative_path("Demo.xml").unwrap(),
            snapshot.revision.clone(),
        );
        let owner = ObjectRef::new(
            snapshot.source_id.clone(),
            ObjectKey::new("owner").unwrap(),
            IdentityStrength::Persistent,
            NodeKind::MetadataObject {
                metadata_type: "Document".to_string(),
            },
            "Owner",
        );
        let relation = RelationGroupRef {
            source_id: snapshot.source_id.clone(),
            group_key: RelationKey::new("children").unwrap(),
            owner: owner.clone(),
            role: RelationRole::Children,
            kind: RelationKind::Contains,
        };
        let selection = NavigationSelection {
            properties: PropertySelection::All,
            facets: FacetSelection::Full,
            relations: Vec::new(),
        };
        let cursor = NavigationCursor {
            schema_version: NavigationCursor::SCHEMA_VERSION,
            source_id: snapshot.source_id.clone(),
            snapshot_revision: SourceRevision::new("sha256:foreign").unwrap(),
            target_identity: binding.target_identity.clone(),
            target: owner.object_key.clone(),
            relation: relation.group_key.clone(),
            relation_role: relation.role,
            relation_kind: relation.kind,
            selection,
            selection_hash: "sha256:selection".to_string(),
            auth_tag: "tag".to_string(),
            next_position: 1,
        };
        let envelope = NavigationEnvelope {
            schema_version: "1".to_string(),
            status: NavigationStatus::Available,
            snapshot: Some(snapshot),
            root: Some(owner),
            nodes: Vec::new(),
            relations: vec![NavigationRelationPage {
                relation,
                items: Vec::new(),
                next_cursor: Some(cursor),
            }],
            diagnostics: Vec::new(),
            relation_index: Arc::new(Vec::new()),
        };

        let error = validate_identity_bearing_navigation(&binding, &envelope).unwrap_err();

        assert_eq!(error.kind, SourceAdapterErrorKind::SnapshotStale);
    }

    #[test]
    fn built_in_registry_registers_only_the_platform_xml_family() {
        let registry = BuiltInSourceAdapterRegistry::new();

        assert_eq!(registry.capturers.len(), 1);
        assert_eq!(registry.probes.len(), 1);
        assert_eq!(registry.readers.len(), 1);
        assert_eq!(
            registry.readers[0].manifest.source_family,
            SourceFamily::PlatformXml
        );
    }

    #[test]
    fn incomparable_overlapping_ranges_are_ambiguous() {
        let registry = registry_with(
            vec![
                reader("left", "2.0", "2.20", Arc::new(AtomicUsize::new(0)), false),
                reader(
                    "right",
                    "2.10",
                    "2.30",
                    Arc::new(AtomicUsize::new(0)),
                    false,
                ),
            ],
            vec![probe("2.15", "root")],
        );

        let error = registry.inspect(input()).unwrap_err();

        assert_eq!(error.kind, SourceAdapterErrorKind::ProbeAmbiguous);
    }

    #[test]
    fn family_and_feature_constraints_filter_readers_before_arbitration() {
        let mut edt = manifest("edt");
        edt.source_family = SourceFamily::Edt;
        edt.supported_formats = vec![exact("2.20")];
        let mut excluded = manifest("excluded");
        excluded.supported_formats = vec![exact("2.20")];
        excluded.excluded_features.insert("legacy".to_string());
        let mut required = manifest("required");
        required.supported_formats = vec![exact("2.20")];
        required
            .required_features
            .insert("root-properties".to_string());
        let calls = Arc::new(AtomicUsize::new(0));
        let registry = BuiltInSourceAdapterRegistry::with_ports(
            vec![(SourceFamily::PlatformXml, Arc::new(StubCapture))],
            vec![(
                SourceFamily::PlatformXml,
                Arc::new(FeatureProbe {
                    features: BTreeSet::from(["legacy".to_string(), "root-properties".to_string()]),
                }),
            )],
            vec![
                (
                    edt,
                    Arc::new(StubRead {
                        marker: "edt",
                        calls: calls.clone(),
                        fail: false,
                    }),
                ),
                (
                    excluded,
                    Arc::new(StubRead {
                        marker: "excluded",
                        calls: calls.clone(),
                        fail: false,
                    }),
                ),
                (
                    required,
                    Arc::new(StubRead {
                        marker: "required",
                        calls: calls.clone(),
                        fail: false,
                    }),
                ),
            ],
        );

        let envelope = registry.inspect(input()).unwrap();

        assert_eq!(envelope.diagnostics[0].message, "required");
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn conflicting_probes_are_ambiguous() {
        let registry = registry_with(
            vec![reader(
                "reader",
                "2.0",
                "3.0",
                Arc::new(AtomicUsize::new(0)),
                false,
            )],
            vec![probe("2.20", "first"), probe("2.21", "second")],
        );

        let error = registry.inspect(input()).unwrap_err();

        assert_eq!(error.kind, SourceAdapterErrorKind::ProbeAmbiguous);
    }

    #[test]
    fn a_second_family_uses_the_same_capture_probe_and_reader_path() {
        let calls = Arc::new(AtomicUsize::new(0));
        let mut cf_manifest = manifest("cf");
        cf_manifest.source_family = SourceFamily::Cf;
        cf_manifest.supported_formats = vec![exact("2.20")];
        let registry = BuiltInSourceAdapterRegistry::with_ports(
            vec![(SourceFamily::Cf, Arc::new(StubCapture))],
            vec![(
                SourceFamily::Cf,
                Arc::new(FamilyProbe {
                    family: SourceFamily::Cf,
                }),
            )],
            vec![(
                cf_manifest,
                Arc::new(StubRead {
                    marker: "cf",
                    calls: calls.clone(),
                    fail: false,
                }),
            )],
        );
        let mut source = input();
        source.declared_family = SourceFamily::Cf;
        source.configured_source_set_kind = None;

        let envelope = registry.inspect(source).unwrap();

        assert_eq!(envelope.diagnostics[0].message, "cf");
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn platform_and_second_family_registrations_coexist_without_cross_family_ambiguity() {
        let mut platform_manifest = manifest("platform");
        platform_manifest.supported_formats = vec![exact("2.20")];
        let mut cf_manifest = manifest("cf");
        cf_manifest.source_family = SourceFamily::Cf;
        cf_manifest.supported_formats = vec![exact("2.20")];
        let registry = BuiltInSourceAdapterRegistry::with_ports(
            vec![
                (SourceFamily::PlatformXml, Arc::new(StubCapture)),
                (SourceFamily::Cf, Arc::new(StubCapture)),
            ],
            vec![
                (
                    SourceFamily::PlatformXml,
                    Arc::new(StubProbe {
                        version: "2.20",
                        evidence: "platform",
                    }),
                ),
                (
                    SourceFamily::Cf,
                    Arc::new(FamilyProbe {
                        family: SourceFamily::Cf,
                    }),
                ),
            ],
            vec![
                (
                    platform_manifest,
                    Arc::new(StubRead {
                        marker: "platform",
                        calls: Arc::new(AtomicUsize::new(0)),
                        fail: false,
                    }),
                ),
                (
                    cf_manifest,
                    Arc::new(StubRead {
                        marker: "cf",
                        calls: Arc::new(AtomicUsize::new(0)),
                        fail: false,
                    }),
                ),
            ],
        );

        assert_eq!(
            registry.inspect(input()).unwrap().diagnostics[0].message,
            "platform"
        );
        let mut cf = input();
        cf.declared_family = SourceFamily::Cf;
        cf.configured_source_set_kind = None;
        assert_eq!(registry.inspect(cf).unwrap().diagnostics[0].message, "cf");
    }

    #[test]
    fn stale_reader_snapshot_and_foreign_root_are_rejected() {
        for (case, expected) in [
            (
                ReadyReaderCase::StaleRevision,
                SourceAdapterErrorKind::SnapshotStale,
            ),
            (
                ReadyReaderCase::ForeignRoot,
                SourceAdapterErrorKind::SnapshotInconsistent,
            ),
        ] {
            let registry = registry_with(
                vec![(
                    {
                        let mut manifest = manifest("ready");
                        manifest.supported_formats = vec![exact("2.20")];
                        manifest
                    },
                    Arc::new(ReadyRead { case }),
                )],
                vec![probe("2.20", "root")],
            );

            let error = registry.inspect(input()).unwrap_err();

            assert_eq!(error.kind, expected, "{case:?}");
        }
    }

    #[test]
    fn typed_identity_fields_fail_closed_without_inspecting_ordinary_data_keys() {
        for case in [
            BindingCase::Node,
            BindingCase::RelationGroup,
            BindingCase::RelationIndex,
            BindingCase::RelationItem,
            BindingCase::CursorSource,
            BindingCase::CursorRevision,
            BindingCase::CursorOwner,
            BindingCase::CursorGroup,
            BindingCase::NestedObjectRef,
            BindingCase::ActionTarget,
        ] {
            let (binding, envelope) = binding_envelope(Some(case));

            let error = validate_identity_bearing_navigation(&binding, &envelope).unwrap_err();

            assert_eq!(
                error.kind,
                if matches!(case, BindingCase::CursorRevision) {
                    SourceAdapterErrorKind::SnapshotStale
                } else {
                    SourceAdapterErrorKind::SnapshotInconsistent
                },
                "{case:?}",
            );
        }

        let (binding, envelope) = binding_envelope(None);
        validate_identity_bearing_navigation(&binding, &envelope).unwrap();
    }

    #[test]
    fn identity_validation_accepts_a_twenty_five_thousand_node_graph() {
        let binding = binding();
        let property = structure_property(PropertyValue::Null);
        let mut nodes = Vec::with_capacity(25_000);
        for index in 0..25_000 {
            let mut node = bound_node(bound_reference(&binding, &format!("ordinary-{index}")));
            node.properties = std::collections::BTreeMap::from([
                ("first".to_string(), property.clone()),
                ("second".to_string(), property.clone()),
            ]);
            nodes.push(node);
        }
        let envelope = available_envelope(None, nodes);

        validate_identity_bearing_navigation(&binding, &envelope).unwrap();
    }

    #[test]
    fn identity_validation_budget_accepts_the_limit_and_reports_every_failure_count() {
        let mut exact_budget = IdentityValidationBudget::default();
        exact_budget
            .charge(MAX_IDENTITY_BEARING_VALIDATION_ITEMS)
            .unwrap();
        assert_eq!(exact_budget.items(), MAX_IDENTITY_BEARING_VALIDATION_ITEMS);

        let mut over_budget = IdentityValidationBudget::default();
        let over = MAX_IDENTITY_BEARING_VALIDATION_ITEMS + 1;
        let error = over_budget.charge(over).unwrap_err();
        assert_eq!(over_budget.items(), over);
        assert_eq!(error.kind, SourceAdapterErrorKind::ResourceLimit);
        assert!(error.message.contains(&over.to_string()));

        let mut overflow_budget = IdentityValidationBudget::default();
        overflow_budget.charge(1).unwrap();
        let error = overflow_budget.charge(usize::MAX).unwrap_err();
        assert_eq!(overflow_budget.items(), 1);
        assert!(error.message.contains("overflow"));
    }

    #[test]
    fn pinned_format_and_foreign_probe_identity_fail_closed() {
        let mut pinned = input();
        pinned.declared_format = Some(FormatVersion::parse("2.20").unwrap());
        let registry = registry_with(
            vec![reader(
                "reader",
                "2.19",
                "2.19",
                Arc::new(AtomicUsize::new(0)),
                false,
            )],
            vec![probe("2.19", "root")],
        );
        assert_eq!(
            registry.inspect(pinned).unwrap_err().kind,
            SourceAdapterErrorKind::SnapshotInconsistent
        );

        for case in [ForeignProbeCase::Source, ForeignProbeCase::Revision] {
            let registry = BuiltInSourceAdapterRegistry::with_ports(
                vec![(SourceFamily::PlatformXml, Arc::new(StubCapture))],
                vec![(SourceFamily::PlatformXml, Arc::new(ForeignProbe { case }))],
                vec![reader(
                    "reader",
                    "2.20",
                    "2.20",
                    Arc::new(AtomicUsize::new(0)),
                    false,
                )],
            );
            assert!(matches!(
                registry.inspect(input()).unwrap_err().kind,
                SourceAdapterErrorKind::SnapshotInconsistent
                    | SourceAdapterErrorKind::SnapshotStale
            ));
        }
    }

    struct FeatureProbe {
        features: BTreeSet<String>,
    }

    impl ProbePort for FeatureProbe {
        fn probe(&self, _captured: &CapturedSource) -> Result<ProbeResult, SourceAdapterError> {
            Ok(ProbeResult::Match(test_descriptor(
                SourceFamily::PlatformXml,
                self.features.clone(),
            )))
        }
    }

    struct FamilyProbe {
        family: SourceFamily,
    }

    impl ProbePort for FamilyProbe {
        fn probe(&self, _captured: &CapturedSource) -> Result<ProbeResult, SourceAdapterError> {
            Ok(ProbeResult::Match(test_descriptor(
                self.family.clone(),
                BTreeSet::new(),
            )))
        }
    }

    #[derive(Debug, Clone, Copy)]
    enum ReadyReaderCase {
        StaleRevision,
        ForeignRoot,
    }

    struct ReadyRead {
        case: ReadyReaderCase,
    }

    impl ReadPort for ReadyRead {
        fn read(
            &self,
            request: &FormatReadRequest,
        ) -> Result<NavigationEnvelope, SourceAdapterError> {
            let source_id = request.captured.snapshot().source_id.clone();
            let revision = match self.case {
                ReadyReaderCase::StaleRevision => {
                    SourceRevision::new("sha256:stale-reader").unwrap()
                }
                ReadyReaderCase::ForeignRoot => request.captured.snapshot().revision.clone(),
            };
            let root = matches!(self.case, ReadyReaderCase::ForeignRoot).then(|| {
                ObjectRef::new(
                    SourceId::new("workspace:foreign").unwrap(),
                    ObjectKey::new("foreign-root").unwrap(),
                    IdentityStrength::Persistent,
                    NodeKind::Document,
                    "Foreign",
                )
            });
            Ok(NavigationEnvelope {
                schema_version: "1".to_string(),
                status: NavigationStatus::Available,
                snapshot: Some(SourceSnapshot {
                    source_id,
                    revision,
                    consistency: SnapshotConsistency::Consistent,
                    adapter_id: "ready".to_string(),
                }),
                root,
                nodes: Vec::new(),
                relations: Vec::new(),
                diagnostics: Vec::new(),
                relation_index: Arc::new(Vec::new()),
            })
        }
    }

    #[derive(Debug, Clone, Copy)]
    enum ForeignProbeCase {
        Source,
        Revision,
    }

    struct ForeignProbe {
        case: ForeignProbeCase,
    }

    impl ProbePort for ForeignProbe {
        fn probe(&self, _captured: &CapturedSource) -> Result<ProbeResult, SourceAdapterError> {
            let mut descriptor = test_descriptor(SourceFamily::PlatformXml, BTreeSet::new());
            match self.case {
                ForeignProbeCase::Source => {
                    descriptor.source_id = SourceId::new("workspace:foreign").unwrap();
                }
                ForeignProbeCase::Revision => {
                    descriptor.snapshot_evidence.as_mut().unwrap().revision =
                        SourceRevision::new("sha256:foreign").unwrap();
                }
            }
            Ok(ProbeResult::Match(descriptor))
        }
    }

    #[derive(Debug, Clone, Copy)]
    enum BindingCase {
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

    fn binding_envelope(case: Option<BindingCase>) -> (SourceBinding, NavigationEnvelope) {
        let binding = binding();
        let root = bound_reference(&binding, "binding-root");
        let mut envelope = available_envelope(Some(root.clone()), vec![bound_node(root.clone())]);
        match case {
            Some(BindingCase::Node) => {
                envelope.nodes[0].reference = foreign_reference("foreign-node");
            }
            Some(BindingCase::RelationGroup) => {
                let mut relation = bound_group(&binding, root.clone());
                relation.source_id = SourceId::new("workspace:foreign").unwrap();
                envelope.relations.push(NavigationRelationPage {
                    relation,
                    items: Vec::new(),
                    next_cursor: None,
                });
            }
            Some(BindingCase::RelationIndex) => {
                let group = bound_group(&binding, root.clone());
                Arc::make_mut(&mut envelope.relation_index).push(SemanticRelation {
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
            Some(BindingCase::RelationItem) => {
                envelope.relations.push(NavigationRelationPage {
                    relation: bound_group(&binding, root),
                    items: vec![bound_node(foreign_reference("foreign-item"))],
                    next_cursor: None,
                });
            }
            Some(BindingCase::CursorSource)
            | Some(BindingCase::CursorRevision)
            | Some(BindingCase::CursorOwner)
            | Some(BindingCase::CursorGroup) => {
                let relation = bound_group(&binding, root);
                let mut cursor = bound_cursor(&binding, &relation);
                match case.unwrap() {
                    BindingCase::CursorSource => {
                        cursor.source_id = SourceId::new("workspace:foreign").unwrap();
                    }
                    BindingCase::CursorRevision => {
                        cursor.snapshot_revision =
                            SourceRevision::new("sha256:foreign-cursor").unwrap();
                    }
                    BindingCase::CursorOwner => {
                        cursor.target = ObjectKey::new("foreign-owner").unwrap();
                    }
                    BindingCase::CursorGroup => {
                        cursor.relation = RelationKey::new("foreign-group").unwrap();
                    }
                    _ => unreachable!(),
                }
                envelope.relations.push(NavigationRelationPage {
                    relation,
                    items: Vec::new(),
                    next_cursor: Some(cursor),
                });
            }
            Some(BindingCase::NestedObjectRef) => {
                envelope.nodes[0].properties.insert(
                    "nested".to_string(),
                    structure_property(PropertyValue::Structure(std::collections::BTreeMap::from(
                        [(
                            "nested".to_string(),
                            PropertyValue::List(vec![PropertyValue::ObjectRef(foreign_reference(
                                "foreign-property",
                            ))]),
                        )],
                    ))),
                );
            }
            Some(BindingCase::ActionTarget) => {
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
                    structure_property(PropertyValue::Structure(std::collections::BTreeMap::from(
                        [
                            (
                                "sourceId".to_string(),
                                PropertyValue::String("workspace:foreign".to_string()),
                            ),
                            (
                                "snapshotRevision".to_string(),
                                PropertyValue::String("sha256:foreign".to_string()),
                            ),
                        ],
                    ))),
                );
                envelope.diagnostics.push(SourceAdapterDiagnostic {
                    code: "ordinary_data".to_string(),
                    message: "ordinary keys are not references".to_string(),
                    details: None,
                });
            }
        }
        (binding, envelope)
    }

    fn binding() -> SourceBinding {
        let snapshot = snapshot("capture");
        SourceBinding::new(
            snapshot.source_id,
            SourceFamily::PlatformXml,
            None,
            TargetIdentity::from_normalized_relative_path("Demo.xml").unwrap(),
            snapshot.revision,
        )
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
            NavigationSelection {
                properties: PropertySelection::All,
                facets: FacetSelection::Full,
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

    fn available_envelope(
        root: Option<ObjectRef>,
        nodes: Vec<NavigationNode>,
    ) -> NavigationEnvelope {
        NavigationEnvelope {
            schema_version: "1".to_string(),
            status: NavigationStatus::Available,
            snapshot: None,
            root,
            nodes,
            relations: Vec::new(),
            diagnostics: Vec::new(),
            relation_index: Arc::new(Vec::new()),
        }
    }

    fn test_descriptor(
        family: SourceFamily,
        detected_features: BTreeSet<String>,
    ) -> SourceDescriptor {
        let snapshot = snapshot("capture");
        SourceDescriptor {
            source_id: snapshot.source_id,
            family,
            format_version: FormatVersion::parse("2.20").unwrap(),
            producer_version: None,
            detected_features,
            probe_evidence: vec!["root".to_string()],
            snapshot_evidence: Some(SnapshotEvidence {
                revision: snapshot.revision,
                root_descriptor_digest: "sha256:root".to_string(),
            }),
        }
    }

    fn exact(version: &str) -> FormatRange {
        FormatRange {
            min_inclusive: FormatVersion::parse(version).unwrap(),
            max_inclusive: FormatVersion::parse(version).unwrap(),
        }
    }

    fn registry_with(
        readers: Vec<(AdapterManifest, Arc<dyn ReadPort>)>,
        probes: Vec<(SourceFamily, Arc<dyn ProbePort>)>,
    ) -> BuiltInSourceAdapterRegistry {
        BuiltInSourceAdapterRegistry::with_ports(
            vec![(SourceFamily::PlatformXml, Arc::new(StubCapture))],
            probes,
            readers,
        )
    }

    fn reader(
        marker: &'static str,
        min: &'static str,
        max: &'static str,
        calls: Arc<AtomicUsize>,
        fail: bool,
    ) -> (AdapterManifest, Arc<dyn ReadPort>) {
        let mut manifest = manifest(marker);
        manifest.supported_formats = vec![FormatRange {
            min_inclusive: FormatVersion::parse(min).unwrap(),
            max_inclusive: FormatVersion::parse(max).unwrap(),
        }];
        (
            manifest,
            Arc::new(StubRead {
                marker,
                calls,
                fail,
            }),
        )
    }

    fn probe(version: &'static str, evidence: &'static str) -> (SourceFamily, Arc<dyn ProbePort>) {
        (
            SourceFamily::PlatformXml,
            Arc::new(StubProbe { version, evidence }),
        )
    }

    fn manifest(adapter_id: &'static str) -> AdapterManifest {
        AdapterManifest {
            adapter_id,
            adapter_version: "test",
            source_family: SourceFamily::PlatformXml,
            supported_formats: Vec::new(),
            required_features: BTreeSet::new(),
            excluded_features: BTreeSet::new(),
            source_access: SourceAccess::ReadOnly,
            maturity: AdapterMaturity::ReadCompatible,
        }
    }

    fn snapshot(adapter_id: &str) -> SourceSnapshot {
        SourceSnapshot {
            source_id: SourceId::new("workspace:main").unwrap(),
            revision: SourceRevision::new("sha256:capture").unwrap(),
            consistency: SnapshotConsistency::Consistent,
            adapter_id: adapter_id.to_string(),
        }
    }

    fn input() -> SourceInput {
        SourceInput {
            workspace_root: std::path::PathBuf::from("/workspace"),
            source_root: std::path::PathBuf::from("/workspace/src"),
            target: std::path::PathBuf::from("/workspace/src/Demo.xml"),
            configured_source_set: Some("main".to_string()),
            configured_source_set_kind: Some(
                unica_format_core::source::ConfiguredSourceSetKind::Configuration,
            ),
            declared_family: SourceFamily::PlatformXml,
            declared_format: None,
        }
    }
}
