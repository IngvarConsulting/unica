//! The form dialect of Designer export format 2.21 (platform 8.5.1): what the
//! all_kinds corpus of that version adds to 2.20. Format 2.22 keeps it.

/// The export format version that introduced the tables below.
pub(crate) const VERSION: &str = "2.21";

/// Every form descriptor of the corpus names, after `UsePurposes`, the
/// interface compatibility mode it is used in: `Any` by default.
pub(crate) const DESCRIPTOR_PROPERTIES_AFTER_USE_PURPOSES: &[&str] =
    &["<UseInInterfaceCompatibilityMode>Any</UseInInterfaceCompatibilityMode>"];
