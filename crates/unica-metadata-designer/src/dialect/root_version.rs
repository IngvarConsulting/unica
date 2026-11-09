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
pub(crate) const fn writable() -> &'static str {
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

#[cfg(test)]
mod tests {
    use super::{is_writable, report, require_supported, writable, RootVersionReport};

    #[test]
    fn only_the_written_version_is_writable() {
        assert!(is_writable(Some(writable())));
        assert!(!is_writable(None));
        assert!(!is_writable(Some("2.19")));
        assert!(!is_writable(Some("2.21")));
    }

    #[test]
    fn the_report_names_the_written_version() {
        let RootVersionReport::Ok(line) = report(Some(writable())) else {
            panic!("the written version is supported");
        };
        assert_eq!(line, format!("Export format: {}", writable()));
        assert!(matches!(report(Some("2.19")), RootVersionReport::Warn(_)));
        assert!(matches!(
            report(Some("latest")),
            RootVersionReport::Error(_)
        ));
        assert!(require_supported(Some(writable())).is_ok());
        assert!(require_supported(Some("2.21")).is_err());
    }
}
