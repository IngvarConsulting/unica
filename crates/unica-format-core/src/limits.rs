//! Shared resource limits for navigation parsing, projection, and retention.

use serde::Serialize;

pub const MAX_NAVIGATION_NODES: usize = 50_000;
pub const MAX_NAVIGATION_RELATIONS: usize = 100_000;
pub const MAX_NAVIGATION_PROPERTIES_PER_NODE: usize = 512;
pub const MAX_NAVIGATION_TYPE_VARIANTS: usize = 512;
pub const MAX_NAVIGATION_IDENTITY_ITEMS: usize = 1_000_000;
pub const MAX_NAVIGATION_NESTING_DEPTH: usize = 64;

pub const MAX_NAVIGATION_PROPERTY_BYTES: usize = 1024 * 1024;
pub const MAX_NAVIGATION_PROPERTY_VALUE_BYTES: usize = 768 * 1024;
pub const MAX_NAVIGATION_SEMANTIC_STRING_BYTES: usize = 512 * 1024;
pub const MAX_NAVIGATION_DIAGNOSTIC_DETAILS_BYTES: usize = 256 * 1024;
pub const MAX_NAVIGATION_DIAGNOSTICS_BYTES: usize = 1024 * 1024;

/// Public `select` and cursor selection strings are intentionally much
/// smaller than retained semantic strings.
pub const MAX_NAVIGATION_SELECTOR_STRING_BYTES: usize = 256;
pub const MAX_NAVIGATION_CURSOR_STRING_BYTES: usize = 1024;
pub const MAX_NAVIGATION_PROPERTY_SELECTORS: usize = 256;
pub const MAX_NAVIGATION_RELATION_SELECTORS: usize = 64;
pub const MAX_NAVIGATION_SELECT_JSON_BYTES: usize = 128 * 1024;
pub const MAX_NAVIGATION_CURSOR_JSON_BYTES: usize = 128 * 1024;

/// Complete resource budget used by a navigation implementation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NavigationResourceLimits {
    pub max_nodes: usize,
    pub max_relations: usize,
    pub max_properties_per_node: usize,
    pub max_type_variants: usize,
    pub max_identity_items: usize,
    pub max_nesting_depth: usize,
    pub max_property_bytes: usize,
    pub max_property_value_bytes: usize,
    pub max_semantic_string_bytes: usize,
    pub max_diagnostic_details_bytes: usize,
    pub max_diagnostics_bytes: usize,
    pub max_selector_string_bytes: usize,
    pub max_cursor_string_bytes: usize,
    pub max_property_selectors: usize,
    pub max_relation_selectors: usize,
    pub max_select_json_bytes: usize,
    pub max_cursor_json_bytes: usize,
}

impl Default for NavigationResourceLimits {
    fn default() -> Self {
        Self {
            max_nodes: MAX_NAVIGATION_NODES,
            max_relations: MAX_NAVIGATION_RELATIONS,
            max_properties_per_node: MAX_NAVIGATION_PROPERTIES_PER_NODE,
            max_type_variants: MAX_NAVIGATION_TYPE_VARIANTS,
            max_identity_items: MAX_NAVIGATION_IDENTITY_ITEMS,
            max_nesting_depth: MAX_NAVIGATION_NESTING_DEPTH,
            max_property_bytes: MAX_NAVIGATION_PROPERTY_BYTES,
            max_property_value_bytes: MAX_NAVIGATION_PROPERTY_VALUE_BYTES,
            max_semantic_string_bytes: MAX_NAVIGATION_SEMANTIC_STRING_BYTES,
            max_diagnostic_details_bytes: MAX_NAVIGATION_DIAGNOSTIC_DETAILS_BYTES,
            max_diagnostics_bytes: MAX_NAVIGATION_DIAGNOSTICS_BYTES,
            max_selector_string_bytes: MAX_NAVIGATION_SELECTOR_STRING_BYTES,
            max_cursor_string_bytes: MAX_NAVIGATION_CURSOR_STRING_BYTES,
            max_property_selectors: MAX_NAVIGATION_PROPERTY_SELECTORS,
            max_relation_selectors: MAX_NAVIGATION_RELATION_SELECTORS,
            max_select_json_bytes: MAX_NAVIGATION_SELECT_JSON_BYTES,
            max_cursor_json_bytes: MAX_NAVIGATION_CURSOR_JSON_BYTES,
        }
    }
}
