//! The metadata dialect of Designer export format 2.22 (platform 8.5.4).
use super::MetadataDialect;

/// Format 2.22 (8.5.4): the 2.21 tables; the line adds its mode and the
/// analytics schema of a report.
pub(crate) static DIALECT: MetadataDialect = MetadataDialect {
    version: "2.22",
    platform_line: "8.5.4",
    kind_declares_child_objects: super::v2_20::kind_declares_child_objects,
    boolean_properties: super::v2_20::boolean_properties,
    line_compatibility_modes: &["Version8_5_1", "Version8_5_4"],
    added_child_properties: super::v2_21::added_child_properties,
    added_object_properties,
    adopted_properties,
};

/// The 8.5.4 export of a borrowing extension writes after
/// `ExtendedConfigurationObject` of every borrowed object and member what it
/// does when the configuration object is missing, `Auto` by default; a
/// borrowed form also names what it does on an extension change error,
/// `Warning` (`tests/fixtures/formats/platform_xml/2.22/cfe_borrow/`).
pub(crate) fn adopted_properties(node: super::AdoptedNode) -> &'static [&'static str] {
    const MISSING: &str = "<ActionOnConfigurationObjectMissing>Auto</ActionOnConfigurationObjectMissing>";
    match node {
        super::AdoptedNode::Object => &[MISSING],
        super::AdoptedNode::Form => &[
            MISSING,
            "<ActionOnConfigurationExtensionChangeError>Warning</ActionOnConfigurationExtensionChangeError>",
        ],
    }
}

/// The all_kinds corpus of 2.22 names the main analytics system schema of a
/// report, empty by default, right after its main data composition schema.
pub(crate) fn added_object_properties(kind: &str) -> &'static [(&'static str, &'static str)] {
    match kind {
        "Report" => &[("MainDataCompositionSchema", "<MainAnalyticsSystemSchema/>")],
        _ => &[],
    }
}
