//! The metadata dialect of Designer export format 2.22 (platform 8.5.4).
use super::MetadataDialect;

/// Format 2.22 (8.5.4): the 2.21 tables; the line adds its mode.
pub(crate) static DIALECT: MetadataDialect = MetadataDialect {
    version: "2.22",
    platform_line: "8.5.4",
    kind_declares_child_objects: super::v2_20::kind_declares_child_objects,
    boolean_properties: super::v2_20::boolean_properties,
    line_compatibility_modes: &["Version8_5_1", "Version8_5_4"],
};
