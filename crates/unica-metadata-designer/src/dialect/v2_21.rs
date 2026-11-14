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
    added_child_properties,
    added_object_properties,
    adopted_properties: super::v2_20::adopted_properties,
};

/// The all_kinds corpus of 2.21 closes the properties of every enumeration
/// value with its color, `auto` by default, and writes after `UsePurposes`
/// of every form the interface compatibility mode it is used in, `Any`.
pub(crate) fn added_child_properties(child_type: &str) -> &'static [&'static str] {
    match child_type {
        "EnumValue" => &["<Color>auto</Color>"],
        "Form" => &["<UseInInterfaceCompatibilityMode>Any</UseInInterfaceCompatibilityMode>"],
        _ => &[],
    }
}

/// The all_kinds corpus of 2.21 names the auxiliary variant form of a report,
/// empty by default, right after its default variant form.
pub(crate) fn added_object_properties(kind: &str) -> &'static [(&'static str, &'static str)] {
    match kind {
        "Report" => &[("DefaultVariantForm", "<AuxiliaryVariantForm/>")],
        _ => &[],
    }
}
