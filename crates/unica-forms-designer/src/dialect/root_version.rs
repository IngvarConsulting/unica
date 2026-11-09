//! The `version` of a form file root (INV.SOURCE.PROFILE-CAPABILITIES).
//!
//! Readers and writers of the form codec ask here whether a root is in the
//! format the codec writes, and how a validation report names it.
pub(crate) use unica_source_designer::format_profile::RootVersionReport;
use unica_source_designer::format_profile::{is_writable_root_version, root_version_report};

/// Whether a root recording `version` is in the format the codec writes.
pub(crate) fn is_writable(version: Option<&str>) -> bool {
    is_writable_root_version(version)
}

/// The validation report line for a root recording `version` as written.
pub(crate) fn report(version: Option<&str>) -> RootVersionReport {
    root_version_report(version)
}
