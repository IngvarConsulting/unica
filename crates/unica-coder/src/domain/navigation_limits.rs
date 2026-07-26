//! Shared resource limits for navigation parsing, projection, and retention.

pub(crate) const MAX_NAVIGATION_NODES: usize = 50_000;
pub(crate) const MAX_NAVIGATION_RELATIONS: usize = 100_000;
pub(crate) const MAX_NAVIGATION_PROPERTIES_PER_NODE: usize = 512;
pub(crate) const MAX_NAVIGATION_TYPE_VARIANTS: usize = 512;
pub(crate) const MAX_NAVIGATION_IDENTITY_ITEMS: usize = 1_000_000;
pub(crate) const MAX_NAVIGATION_NESTING_DEPTH: usize = 64;

pub(crate) const MAX_NAVIGATION_PROPERTY_BYTES: usize = 1024 * 1024;
pub(crate) const MAX_NAVIGATION_PROPERTY_VALUE_BYTES: usize = 768 * 1024;
pub(crate) const MAX_NAVIGATION_SEMANTIC_STRING_BYTES: usize = 512 * 1024;
pub(crate) const MAX_NAVIGATION_DIAGNOSTIC_DETAILS_BYTES: usize = 256 * 1024;
pub(crate) const MAX_NAVIGATION_DIAGNOSTICS_BYTES: usize = 1024 * 1024;

/// Public `select` and cursor selection strings are intentionally much
/// smaller than retained semantic strings.
pub(crate) const MAX_NAVIGATION_SELECTOR_STRING_BYTES: usize = 256;
pub(crate) const MAX_NAVIGATION_CURSOR_STRING_BYTES: usize = 1024;
pub(crate) const MAX_NAVIGATION_PROPERTY_SELECTORS: usize = 256;
pub(crate) const MAX_NAVIGATION_RELATION_SELECTORS: usize = 64;
pub(crate) const MAX_NAVIGATION_SELECT_JSON_BYTES: usize = 128 * 1024;
pub(crate) const MAX_NAVIGATION_CURSOR_JSON_BYTES: usize = 128 * 1024;
