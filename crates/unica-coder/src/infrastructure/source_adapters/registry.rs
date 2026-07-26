use unica_adapter_platform_xml::{PlatformXmlAdapterFactory, PlatformXmlAdapterRegistration};
use unica_format_core::{
    navigation::{
        FacetSelection, NavigationEnvelope, NavigationQuery, NavigationSelection, NavigationStatus,
        NavigationTarget, PropertySelection,
    },
    ports::{CaptureResult, FormatReadRequest, ProbeResult},
    source::{
        SourceAdapterError, SourceAdapterErrorKind, SourceBinding, SourceDescriptor, TargetIdentity,
    },
};

use crate::infrastructure::source_adapters::{CapturedSourceSession, SourceInput};

pub(crate) struct BuiltInSourceAdapterRegistry {
    platform_xml: PlatformXmlAdapterRegistration,
}

impl BuiltInSourceAdapterRegistry {
    pub(crate) fn new() -> Self {
        Self {
            platform_xml: PlatformXmlAdapterFactory::new().registration(),
        }
    }

    pub(crate) fn capture(
        &self,
        input: &SourceInput,
    ) -> Result<Box<dyn CapturedSourceSession>, SourceAdapterError> {
        let source = input.source_context();
        let snapshot = match self.platform_xml.capture.capture(&source)? {
            CaptureResult::NoMatch => {
                return Err(SourceAdapterError::new(
                    SourceAdapterErrorKind::SourceUnavailable,
                    "no source capture adapter recognized the target",
                ));
            }
            CaptureResult::Captured(snapshot) => snapshot,
        };
        let target = input
            .target
            .strip_prefix(&input.source_root)
            .ok()
            .and_then(|path| path.to_str())
            .filter(|path| !path.is_empty())
            .unwrap_or("source")
            .replace('\\', "/");
        let binding = SourceBinding::new(
            snapshot.source_id.clone(),
            input.declared_family.clone(),
            input.declared_format.clone(),
            TargetIdentity::from_normalized_relative_path(&target)?,
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
        let descriptor = match self.platform_xml.probe.probe(session.source())? {
            ProbeResult::NoMatch => {
                return Err(SourceAdapterError::new(
                    SourceAdapterErrorKind::SourceUnavailable,
                    "no source probe recognized the target",
                ));
            }
            ProbeResult::Match(descriptor) => descriptor,
        };
        validate_probe_descriptor(input, session, &descriptor)?;
        if !self
            .platform_xml
            .manifest
            .supported_formats
            .iter()
            .any(|range| range.contains(&descriptor.format_version))
        {
            return Ok(NavigationEnvelope::unavailable(SourceAdapterError::new(
                SourceAdapterErrorKind::FormatUnsupported,
                format!(
                    "no reader supports {:?} format {}",
                    descriptor.family, descriptor.format_version
                ),
            )));
        }
        let envelope = self.platform_xml.read.read(&FormatReadRequest {
            source: session.source().clone(),
            snapshot: session.snapshot().clone(),
            query: NavigationQuery {
                target: NavigationTarget::ObjectPath(
                    input.target.to_string_lossy().replace('\\', "/"),
                ),
                select: NavigationSelection {
                    properties: PropertySelection::All,
                    facets: FacetSelection::Full,
                    relations: Vec::new(),
                },
            },
        })?;
        validate_ready_envelope(&envelope, session)?;
        Ok(envelope)
    }
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
        || snapshot.revision != session.binding().revision
    {
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
