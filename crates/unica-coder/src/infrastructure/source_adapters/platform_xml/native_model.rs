use std::collections::BTreeMap;

use uuid::Uuid;

use crate::domain::{navigation::CoverageState, source_adapters::SourceSnapshot};

use super::schema::MetadataClassRole;

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
    pub(crate) properties: BTreeMap<String, NativeProperty>,
    pub(crate) children: Vec<NativeMetadataNode>,
    pub(crate) backing: NativeNodeBacking,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct NativeMetadataClass {
    pub(crate) canonical_name: &'static str,
    pub(crate) role: MetadataClassRole,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum NativeNodeBacking {
    None,
    Form(NativeForm),
    Template(NativeTemplate),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct NativeRegistrationEvidence {
    pub(crate) uuid: Option<Uuid>,
    pub(crate) name: String,
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
pub(crate) enum NativePropertyValue {
    Scalar(String),
    RawXml(String),
    Absent,
    Unresolved,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum NativePropertyProvenance {
    Explicit,
    Absent,
    Unresolved,
}
