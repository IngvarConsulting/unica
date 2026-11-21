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
/// borrowed form, common form and common template also name what they do on
/// an extension change error, `Warning`, after their own properties
/// (`tests/fixtures/formats/platform_xml/2.22/cfe_borrow/`,
/// `tests/fixtures/formats/platform_xml/2.22/cfe_borrow_kinds/`).
pub(crate) fn adopted_properties(node: super::AdoptedNode) -> &'static [&'static str] {
    const MISSING: &str =
        "<ActionOnConfigurationObjectMissing>Auto</ActionOnConfigurationObjectMissing>";
    const CHANGE_ERROR: &str =
        "<ActionOnConfigurationExtensionChangeError>Warning</ActionOnConfigurationExtensionChangeError>";
    match node {
        super::AdoptedNode::Object => &[MISSING],
        super::AdoptedNode::Form => &[MISSING, CHANGE_ERROR],
        super::AdoptedNode::Content => &[CHANGE_ERROR],
    }
}

/// The all_kinds corpus of 2.22 names the main analytics system schema of a
/// report, empty by default, right after its main data composition schema,
/// besides the auxiliary variant form of 2.21.
pub(crate) fn added_object_properties(kind: &str) -> &'static [(&'static str, &'static str)] {
    match kind {
        "CommonForm" => &[(
            "UsePurposes",
            "<UseInInterfaceCompatibilityMode>Any</UseInInterfaceCompatibilityMode>",
        )],
        "Report" => &[
            ("MainDataCompositionSchema", "<MainAnalyticsSystemSchema/>"),
            ("DefaultVariantForm", "<AuxiliaryVariantForm/>"),
        ],
        _ => &[],
    }
}
