//! The `version` of a role file root (INV.SOURCE.PROFILE-CAPABILITIES).
//!
//! Readers and writers of the role codec ask here whether a root is in the
//! format the codec writes.
use unica_source_designer::format_profile::is_writable_root_version;

/// Whether a root recording `version` is in a format the codec writes.
pub(crate) fn is_writable(version: Option<&str>) -> bool {
    is_writable_root_version(version)
}

/// The writable export formats as a refusal lists them.
pub(crate) fn writable_formats() -> String {
    unica_source_designer::format_profile::writable_export_formats_text()
}
