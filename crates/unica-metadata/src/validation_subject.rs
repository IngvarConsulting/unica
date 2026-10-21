//! Immutable local images and evidence submitted to metadata validation.
use crate::subsystem::SubsystemAddress;
use crate::*;
use std::path::PathBuf;
use unica_address::source_target::MetadataAddress;
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MetadataAuxiliaryXmlKind {
    ExchangePlanContent,
    BusinessProcessFlowchart,
    PredefinedData(MetadataKind),
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MetadataChildResourceKind {
    FormContent,
    /// Ordinary form storage is opaque; validation proves its footprint only.
    OrdinaryFormContent,
    TemplateContent {
        template_type: MetadataTemplateType,
        part: MetadataTemplateResourcePart,
    },
    Module,
    /// A payload member the platform writes beside a child but Unica never
    /// interprets — form help, help assets, form item pictures, HTML template
    /// assets. It is carried verbatim; the closed guard still names the exact
    /// path shapes that may appear, so a mutation can never clobber bytes the
    /// contract does not recognise. Its relative path lives in the child's
    /// [`MetadataChildFootprintEvidence::retained`] at the resource's ordinal.
    Retained,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MetadataTemplateType {
    HtmlDocument,
    TextDocument,
    SpreadsheetDocument,
    BinaryData,
    DataCompositionSchema,
    /// External component archive. Same opaque payload as `BinaryData`; the
    /// dedicated type exists so mobile builds can mark components. Logical
    /// addressing stops at the template and never reads the payload.
    AddIn,
    /// Data composition appearance template: an opaque XML body whose
    /// internals are not addressed.
    DataCompositionAppearanceTemplate,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MetadataTemplateResourcePart {
    Primary,
    HtmlPage,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MetadataChildProfile {
    Form,
    OrdinaryForm,
    Command,
    Template(MetadataTemplateType),
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum MetadataChildDirectoryKind {
    Root,
    Extension,
    HtmlPages,
    /// A directory the contract does not name on its own — it exists only
    /// because a payload file sits under it, such as `Ext/Form` around a form
    /// module or `Ext/Help` around help pages. It repeats once per directory
    /// instead of naming one.
    Nested,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MetadataChildFootprintEvidence {
    pub child: MetadataAddress,
    pub profile: MetadataChildProfile,
    pub directories: Vec<MetadataChildDirectoryKind>,
    /// Relative paths of the retained payload members, sorted. A resource with
    /// [`MetadataChildResourceKind::Retained`] and ordinal `n` is this list's
    /// `n`-th entry, which is how the validator recovers the paths it needs to
    /// re-derive the directory topology.
    pub retained: Vec<PathBuf>,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MetadataResourceRole {
    Descriptor,
    Registration,
    Module {
        owner: MetadataAddress,
    },
    Form {
        owner: MetadataAddress,
        name: String,
    },
    Template {
        owner: MetadataAddress,
        name: String,
    },
    Command {
        owner: MetadataAddress,
        name: String,
    },
    ChildResource {
        child: MetadataAddress,
        kind: MetadataChildResourceKind,
        ordinal: usize,
    },
    Dependency {
        target: MetadataAddress,
    },
    AuxiliaryXml {
        owner: MetadataAddress,
        kind: MetadataAuxiliaryXmlKind,
    },
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MetadataResourceImage {
    /// Provider-neutral logical role and identity; physical paths stay opaque.
    pub role: MetadataResourceRole,
    pub bytes: Vec<u8>,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MetadataValidationSubject {
    pub target: MetadataAddress,
    pub resources: Vec<MetadataResourceImage>,
    pub child_footprints: Vec<MetadataChildFootprintEvidence>,
    pub registrar_evidence: MetadataEvidenceAvailability,
    /// `None` — подсистемы не просматривались: субъект ничего не утверждает о
    /// членствах объекта. `meta.info` собирает обе ролевые группы, а мутации
    /// оставляют поле пустым; полнота по умолчанию сделала бы несобранное
    /// доказанным.
    pub subsystem_evidence: Option<MetadataSubsystemEvidence>,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MetadataSubsystemEvidence {
    Complete {
        functional_subsystems: Vec<SubsystemAddress>,
        interface_subsystems: Vec<SubsystemAddress>,
    },
    Unavailable(Vec<MetaDiagnostic>),
}
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum MetadataEvidenceAvailability {
    #[default]
    Complete,
    Unavailable(Vec<MetaDiagnostic>),
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MetaLocalInfo {
    pub member_read_profile: Option<crate::MetaMemberReadProfile>,
    pub metadata_path: MetadataAddress,
    pub kind: MetadataKind,
    pub details: MetaInfoDetails,
    pub name: String,
    pub synonym: Option<String>,
    pub support: MetaSupportStatus,
    pub properties: Vec<MetaInfoPropertyData>,
    pub declarations: MetaInfoDeclarations,
    /// Internal owner context needed to validate typed Predefined.xml values.
    /// It is deliberately not copied into the public `meta.info` result.
    pub predefined_code_type: Option<String>,
    pub relations: MetaRelationsData,
    pub collections: MetaCollectionsData,
    pub diagnostics: Vec<MetaDiagnostic>,
}
pub struct MetadataRead {
    pub local: MetaLocalInfo,
    pub validation_subject: MetadataValidationSubject,
}
/// Everything a metadata read adds on top of the object's own descriptor.
///
/// The three parts have different natures and are kept apart on purpose:
/// `predefined_items` is the object's own content, `usage` is what the source
/// tree says references it, and `related` is what the code index says. Only the
/// last can be stale or missing, so only it carries index metadata.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct MetaEnrichment {
    pub predefined_items: Option<MetaPredefinedItemsData>,
    pub usage: MetaUsageData,
    pub diagnostics: Vec<MetaDiagnostic>,
}
pub type MetaRelatedData = MetaEnrichment;
pub type MetadataValidationResult = MetaValidationData;
impl MetadataTemplateType {
    pub fn from_descriptor_value(value: &str) -> Option<Self> {
        match value {
            "HTMLDocument" => Some(Self::HtmlDocument),
            "TextDocument" => Some(Self::TextDocument),
            "SpreadsheetDocument" => Some(Self::SpreadsheetDocument),
            "BinaryData" => Some(Self::BinaryData),
            "DataCompositionSchema" => Some(Self::DataCompositionSchema),
            "AddIn" => Some(Self::AddIn),
            "DataCompositionAppearanceTemplate" => Some(Self::DataCompositionAppearanceTemplate),
            _ => None,
        }
    }

    #[cfg(any(test, feature = "test-support"))]
    pub const fn descriptor_value(self) -> &'static str {
        match self {
            Self::HtmlDocument => "HTMLDocument",
            Self::TextDocument => "TextDocument",
            Self::SpreadsheetDocument => "SpreadsheetDocument",
            Self::BinaryData => "BinaryData",
            Self::DataCompositionSchema => "DataCompositionSchema",
            Self::AddIn => "AddIn",
            Self::DataCompositionAppearanceTemplate => "DataCompositionAppearanceTemplate",
        }
    }
}
impl MetaLocalInfo {
    pub fn into_info(
        self,
        validation: MetaValidationData,
        enrichment: MetaEnrichment,
        functional_subsystems: Option<Vec<SubsystemAddress>>,
        interface_subsystems: Option<Vec<SubsystemAddress>>,
    ) -> MetaInfoData {
        MetaInfoData {
            metadata_path: self.metadata_path,
            details: self.details,
            name: self.name,
            synonym: self.synonym,
            support: self.support,
            properties: self.properties,
            declarations: self.declarations,
            relations: self.relations,
            collections: self.collections,
            functional_subsystems,
            interface_subsystems,
            predefined_items: enrichment.predefined_items,
            usage: enrichment.usage,
            validation,
        }
    }
}
