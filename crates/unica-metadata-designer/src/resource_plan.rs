//! Planned images and observations; opaque dependency guards remain issued by composition.
use std::path::PathBuf;
use unica_address::source_target::MetadataAddress;
use unica_metadata::{MetaDiagnostic, MetaMutationEffect, MetaPublicationPlanEntry};
use unica_source_contract::proposal::{DirectoryMembershipSelector, DirectoryMembershipSnapshot};
/// The supplied physical descriptor and logical address, without reinterpreting XML identity.
#[derive(Clone, Copy)]
pub struct MetadataDescriptorTarget<'a> {
    pub path: &'a std::path::Path,
    pub address: &'a MetadataAddress,
}

/// An existing read/binding refusal, carrying no additional diagnostic.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MetadataReadFailure(());
impl MetadataReadFailure {
    pub const UNAVAILABLE: Self = Self(());
}

#[derive(Debug)]
pub struct TypedChildFileMutation {
    pub path: PathBuf,
    pub pre_image: Option<Vec<u8>>,
    pub post_image: Option<Vec<u8>>,
}

#[derive(Debug)]
pub struct TypedChildResourcePlan<G> {
    pub file_mutations: Vec<TypedChildFileMutation>,
    pub absent_path_guards: Vec<PathBuf>,
    pub exact_file_guards: Vec<(PathBuf, Vec<u8>)>,
    pub directory_guards: Vec<(
        PathBuf,
        DirectoryMembershipSelector,
        DirectoryMembershipSnapshot,
    )>,
    pub publication_plan: Vec<MetaPublicationPlanEntry>,
    pub validation_resources: Vec<unica_metadata::validation_subject::MetadataResourceImage>,
    pub validation_footprints:
        Vec<unica_metadata::validation_subject::MetadataChildFootprintEvidence>,
    pub relation_dependencies: Vec<TypedRelationDependency<G>>,
    pub expected_post_images: Vec<(PathBuf, Vec<u8>)>,
}

#[derive(Debug, Default)]
pub struct TypedChildObservation {
    pub validation_resources: Vec<unica_metadata::validation_subject::MetadataResourceImage>,
    pub validation_footprints:
        Vec<unica_metadata::validation_subject::MetadataChildFootprintEvidence>,
    pub diagnostics: Vec<MetaDiagnostic>,
}

#[derive(Debug)]
pub struct TypedRelationDependency<G> {
    pub handle: G,
    pub path: PathBuf,
    pub bytes: Vec<u8>,
    pub target: MetadataAddress,
    pub modules: Vec<TypedDependencyModule<G>>,
}

#[derive(Debug)]
pub struct TypedDependencyModule<G> {
    pub handle: G,
    pub path: PathBuf,
    pub bytes: Vec<u8>,
}

pub struct TypedOperationPostImage<G> {
    pub descriptor: Vec<u8>,
    pub child_resources: TypedChildResourcePlan<G>,
    pub effects: Vec<MetaMutationEffect>,
}

impl<G> Default for TypedChildResourcePlan<G> {
    fn default() -> Self {
        Self {
            file_mutations: Vec::new(),
            absent_path_guards: Vec::new(),
            exact_file_guards: Vec::new(),
            directory_guards: Vec::new(),
            publication_plan: Vec::new(),
            validation_resources: Vec::new(),
            validation_footprints: Vec::new(),
            relation_dependencies: Vec::new(),
            expected_post_images: Vec::new(),
        }
    }
}

pub struct ResolvedMetadataObject<G> {
    pub handle: G,
    pub resolved_target: unica_address::source_target::ResolvedTarget,
    pub metadata_path: MetadataAddress,
    pub descriptor_path: PathBuf,
    pub descriptor_preimage: Vec<u8>,
    pub source_root: PathBuf,
    pub owner_path: PathBuf,
    pub owner_preimage: Vec<u8>,
}
