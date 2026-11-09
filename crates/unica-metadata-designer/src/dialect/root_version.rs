//! The `version` of a metadata file root (INV.SOURCE.PROFILE-CAPABILITIES).
//!
//! Readers and writers of the metadata codec ask here whether a root is in the
//! format the codec writes, and how a validation report names it. Supporting
//! another export format changes this module, not the readers and writers.
pub(crate) use unica_source_designer::format_profile::RootVersionReport;
use unica_source_designer::format_profile::{
    is_writable_root_version, require_supported_root_version, root_version_report,
    ACTIVE_FORMAT_PROFILE,
};

/// The export format version the codec writes.
pub(crate) fn writable() -> &'static str {
    ACTIVE_FORMAT_PROFILE.export_format
}

/// The platform line that writes [`writable`].
pub(crate) fn writable_platform_line() -> &'static str {
    ACTIVE_FORMAT_PROFILE.platform_line
}

/// Whether a root recording `version` is in the format the codec writes.
pub(crate) fn is_writable(version: Option<&str>) -> bool {
    is_writable_root_version(version)
}

/// The validation report line for a root recording `version` as written.
pub(crate) fn report(version: Option<&str>) -> RootVersionReport {
    root_version_report(version)
}

/// `Ok` for a root in the format the codec reads, else why it is not.
pub(crate) fn require_supported(version: Option<&str>) -> Result<(), String> {
    require_supported_root_version(version)
}
