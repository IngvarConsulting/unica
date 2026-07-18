//! Compatibility facade for the public source-root API introduced before the
//! project-discovery resolver moved filesystem behavior into infrastructure.

pub use crate::infrastructure::source_roots::{
    normalize_path_identity, resolve_source_root, select_default_source_set, ResolvedSourceRoot,
};
