use std::{collections::BTreeSet, sync::Arc};

use unica_adapter_platform_xml::PlatformXmlAdapterFactory;
use unica_format_core::{
    navigation::{
        FacetSelection, NavigationEnvelope, NavigationQuery, NavigationSelection, NavigationStatus,
        NavigationTarget, PropertySelection,
    },
    ports::{
        CapturePort, CaptureResult, FormatReadRequest, ProbePort, ProbeResult, ReadPort,
        SourceAdapterRegistration,
    },
    source::{
        AdapterManifest, SourceAdapterError, SourceAdapterErrorKind, SourceBinding,
        SourceDescriptor, SourceFamily, TargetIdentity,
    },
};

use crate::infrastructure::source_adapters::{CapturedSourceSession, SourceInput};

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
        self.inspect_captured(&input, session.as_ref())
    }

    pub(crate) fn capture(
        &self,
        input: &SourceInput,
    ) -> Result<Box<dyn CapturedSourceSession>, SourceAdapterError> {
        let source = input.source_context();
        let mut snapshots = Vec::new();
        for capture in self
            .capturers
            .iter()
            .filter(|capture| capture.family == input.declared_family)
        {
            match capture.port.capture(&source)? {
                CaptureResult::NoMatch => {}
                CaptureResult::Captured(snapshot) => snapshots.push(snapshot),
            }
        }
        let snapshot = match snapshots.len() {
            0 => {
                return Err(SourceAdapterError::new(
                    SourceAdapterErrorKind::SourceUnavailable,
                    "no source capture adapter recognized the target",
                ))
            }
            1 => snapshots.pop().expect("one captured snapshot"),
            _ => {
                return Err(SourceAdapterError::new(
                    SourceAdapterErrorKind::ProbeAmbiguous,
                    "multiple source capture adapters recognized the target",
                ))
            }
        };
        let binding = SourceBinding::new(
            snapshot.source_id.clone(),
            input.declared_family.clone(),
            input.declared_format.clone(),
            TargetIdentity::from_normalized_relative_path(&target_path(input)?)?,
            snapshot.revision.clone(),
        );
        Ok(Box::new(CoreCapturedSession {
            source,
            snapshot,
            binding,
        }))
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
        let envelope = reader.port.read(&FormatReadRequest {
            source: session.source().clone(),
            snapshot: session.snapshot().clone(),
            query: NavigationQuery {
                target: NavigationTarget::ObjectPath(target_path(input)?),
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
        session: &dyn CapturedSourceSession,
    ) -> Result<SourceDescriptor, SourceAdapterError> {
        let mut matches = Vec::new();
        for probe in self
            .probes
            .iter()
            .filter(|probe| probe.family == input.declared_family)
        {
            if let ProbeResult::Match(descriptor) = probe.port.probe(session.source())? {
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

fn target_path(input: &SourceInput) -> Result<String, SourceAdapterError> {
    let path = input
        .target
        .strip_prefix(&input.source_root)
        .map_err(|_| {
            SourceAdapterError::new(
                SourceAdapterErrorKind::SourceUnavailable,
                "source target is outside its source root",
            )
        })?
        .to_str()
        .ok_or_else(|| {
            SourceAdapterError::new(
                SourceAdapterErrorKind::SourceUnavailable,
                "source target path is not UTF-8",
            )
        })?
        .replace('\\', "/");
    Ok(if path.is_empty() {
        "source".to_string()
    } else {
        path
    })
}

struct CoreCapturedSession {
    source: unica_format_core::source::SourceContext,
    snapshot: unica_format_core::source::SourceSnapshot,
    binding: SourceBinding,
}

impl CapturedSourceSession for CoreCapturedSession {
    fn binding(&self) -> &SourceBinding {
        &self.binding
    }

    fn source(&self) -> &unica_format_core::source::SourceContext {
        &self.source
    }

    fn snapshot(&self) -> &unica_format_core::source::SourceSnapshot {
        &self.snapshot
    }
}

fn validate_probe_descriptor(
    input: &SourceInput,
    session: &dyn CapturedSourceSession,
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
    session: &dyn CapturedSourceSession,
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
    use std::sync::atomic::{AtomicUsize, Ordering};
    use unica_format_core::{
        navigation::{
            IdentityStrength, NavigationCursor, NavigationRelationPage, NodeKind, ObjectKey,
            ObjectRef, RelationGroupRef, RelationKey, RelationKind, RelationRole,
        },
        source::{
            AdapterMaturity, FormatRange, FormatVersion, SnapshotConsistency, SnapshotEvidence,
            SourceAccess, SourceId, SourceRevision, SourceSnapshot,
        },
    };

    struct StubCapture;

    impl CapturePort for StubCapture {
        fn capture(
            &self,
            _source: &unica_format_core::source::SourceContext,
        ) -> Result<CaptureResult, SourceAdapterError> {
            Ok(CaptureResult::Captured(snapshot("capture")))
        }
    }

    struct StubProbe {
        version: &'static str,
        evidence: &'static str,
    }

    impl ProbePort for StubProbe {
        fn probe(
            &self,
            _source: &unica_format_core::source::SourceContext,
        ) -> Result<ProbeResult, SourceAdapterError> {
            let snapshot = snapshot("capture");
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

        let descriptor = registry.probe(&input, session.as_ref()).unwrap();

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
