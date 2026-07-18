//! Infrastructure compatibility facade for the domain-owned discovery registry.

#[cfg(test)]
pub(crate) use crate::domain::discovery_registry::METADATA_KINDS;
pub(crate) use crate::domain::discovery_registry::{
    metadata_kind, metadata_kind_by_directory, metadata_kind_index, METADATA_KIND_TAGS,
};
