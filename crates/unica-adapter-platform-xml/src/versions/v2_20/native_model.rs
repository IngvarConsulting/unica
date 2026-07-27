use std::collections::BTreeMap;
use std::ops::Deref;

use uuid::Uuid;

use crate::domain::{
    navigation::{CoverageState, RelationRole, SemanticObjectKind, TypeSetValue},
    source_adapters::SourceSnapshot,
};

use super::schema::MetadataClassRole;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) struct NativeIdentityDiscriminator(String);

impl NativeIdentityDiscriminator {
    pub(crate) fn new(value: String) -> Self {
        Self(value)
    }

    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PlatformXmlNativeSnapshot {
    pub(crate) source: SourceSnapshot,
    pub(crate) root: NativeMetadataNode,
    pub(crate) coverage: CoverageState,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct NativeMetadataNode {
    pub(crate) class: NativeMetadataClass,
    pub(crate) uuid: Option<Uuid>,
    pub(crate) name: String,
    pub(crate) target_discriminator: NativeIdentityDiscriminator,
    pub(crate) occurrence: Option<u32>,
    pub(crate) state: NativeNodeState,
    pub(crate) properties: BTreeMap<String, NativeProperty>,
    pub(crate) references: Vec<NativeReferenceRelation>,
    pub(crate) children: Vec<NativeMetadataChild>,
    pub(crate) unmapped_facts: usize,
    pub(crate) backing: NativeNodeBacking,
}

/// Assigned from the owning native collection, never recovered from the target
/// node kind by a projector.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct NativeMetadataChild {
    pub(crate) role: RelationRole,
    pub(crate) identity_discriminator: NativeIdentityDiscriminator,
    pub(crate) node: NativeMetadataNode,
}

impl Deref for NativeMetadataChild {
    type Target = NativeMetadataNode;

    fn deref(&self) -> &Self::Target {
        &self.node
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct NativeMetadataClass {
    pub(crate) canonical_name: String,
    pub(crate) role: MetadataClassRole,
    pub(crate) kind: SemanticObjectKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum NativeNodeBacking {
    None,
    Form(NativeForm),
    Template(NativeTemplate),
    Rights(NativeContentEvidence),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct NativeRegistrationEvidence {
    pub(crate) uuid: Option<Uuid>,
    pub(crate) name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum NativeNodeState {
    ResolvedInline,
    ResolvedRegistration {
        registration: NativeRegistrationEvidence,
    },
    UnresolvedRegistration {
        registration: NativeRegistrationEvidence,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum NativeEvidenceState {
    Absent,
    Validated,
    Unresolved,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct NativeDescriptorEvidence {
    pub(crate) state: NativeEvidenceState,
    pub(crate) relative_key: String,
    pub(crate) uuid: Option<Uuid>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct NativeContentEvidence {
    pub(crate) state: NativeEvidenceState,
    pub(crate) relative_key: String,
    pub(crate) digest: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct NativeForm {
    pub(crate) registration: NativeRegistrationEvidence,
    pub(crate) descriptor: NativeDescriptorEvidence,
    pub(crate) managed_content: NativeContentEvidence,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum NativeMxlRootKind {
    SpreadsheetDocument,
    LegacyDocument,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct NativeTemplate {
    pub(crate) registration: NativeRegistrationEvidence,
    pub(crate) descriptor: NativeDescriptorEvidence,
    pub(crate) descriptor_type: NativePropertyValue,
    pub(crate) canonical_content: NativeContentEvidence,
    pub(crate) mxl_root_kind: Option<NativeMxlRootKind>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct NativeProperty {
    pub(crate) canonical_id: String,
    pub(crate) value: NativePropertyValue,
    pub(crate) provenance: NativePropertyProvenance,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct NativeReferenceRelation {
    pub(crate) role: RelationRole,
    pub(crate) targets: Vec<NativeSemanticReference>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct NativeSemanticReference {
    pub(crate) kind: SemanticObjectKind,
    pub(crate) name: String,
    pub(crate) uuid: Option<Uuid>,
    pub(crate) target_discriminator: NativeIdentityDiscriminator,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum NativePropertyValue {
    Scalar(String),
    AnnotatedScalar {
        value: String,
        type_annotation: NativeScalarType,
    },
    /// A scalar whose annotation cannot be used safely. The original QName and
    /// attributes are deliberately discarded before the native snapshot exists.
    UnresolvedScalar {
        issue: NativeScalarAnnotationIssue,
    },
    EmptyReference,
    ReadableUnknownScalar {
        category: NativeUnknownScalarCategory,
        values: Vec<String>,
    },
    /// A type description is normalized while its original XML node and
    /// namespace scope are alive.  No detached XML is retained.
    TypeSet(TypeSetValue),
    LocalizedString(BTreeMap<String, String>),
    StringList(Vec<String>),
    Null,
    Structured,
    Absent,
    Unresolved,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum NativeUnknownScalarCategory {
    Reference,
}

/// Normalized schema-relevant `xsi:type` evidence for scalar XML values.
/// Invalid QNames are represented by `UnresolvedScalar`, never by this enum.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum NativeScalarType {
    String,
    Boolean,
    Decimal,
    Integer,
    Uuid,
    Date,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum NativeScalarAnnotationIssue {
    Missing,
    Unknown,
    Conflicting,
    Unqualified,
    InvalidLexical,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum NativePropertyProvenance {
    Explicit,
    Absent,
    Unresolved,
}
