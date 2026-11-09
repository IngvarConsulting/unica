//! The metadata dialect of Designer export format 2.21 (platform 8.5.1).
use super::MetadataDialect;

/// Format 2.21 (8.5.1): the all_kinds corpus shows the 2.20 tables with new
/// root properties the reader classifies by name; the line adds its mode.
pub(crate) static DIALECT: MetadataDialect = MetadataDialect {
    version: "2.21",
    platform_line: "8.5.1",
    kind_declares_child_objects: super::v2_20::kind_declares_child_objects,
    boolean_properties: super::v2_20::boolean_properties,
    line_compatibility_modes: &["Version8_5_1"],
};
