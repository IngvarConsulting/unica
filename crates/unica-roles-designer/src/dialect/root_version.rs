//! The `version` of a role file root (INV.SOURCE.PROFILE-CAPABILITIES).
//!
//! Readers and writers of the role codec ask here whether a root is in the
//! format the codec writes.
use unica_source_designer::format_profile::{is_writable_root_version, ACTIVE_FORMAT_PROFILE};

/// The export format version the codec writes.
pub(crate) const fn writable() -> &'static str {
    ACTIVE_FORMAT_PROFILE.export_format
}

/// Whether a root recording `version` is in the format the codec writes.
pub(crate) fn is_writable(version: Option<&str>) -> bool {
    is_writable_root_version(version)
}
