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

/// The export format version of the source set a new file joins, as the
/// owner it joins records it: a write never changes the format of a set
/// (INV.SOURCE.WRITABLE-PROFILE). A write is admitted only into the writable
/// format, so an owner without a readable version is that format.
pub(crate) fn of_set(owner_xml: &str) -> String {
    let source = owner_xml.trim_start_matches('\u{feff}');
    roxmltree::Document::parse(source)
        .ok()
        .and_then(|document| {
            unica_source_designer::roots::root_version_literal(source, document.root_element())
        })
        .unwrap_or_else(|| writable().to_string())
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
        // Numerically equal spellings are not the writable format.
        assert!(!is_writable(Some("02.20")));
        assert!(!is_writable(Some("2.20.0")));
    }

    /// rule: INV.SOURCE.WRITABLE-PROFILE
    #[test]
    fn a_new_file_takes_the_version_of_the_set_it_joins() {
        use super::of_set;
        let owner = |version: &str| {
            format!(
                "\u{feff}<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<MetaDataObject xmlns=\"http://v8.1c.ru/8.3/MDClasses\" version=\"{version}\"><Catalog/></MetaDataObject>"
            )
        };
        assert_eq!(of_set(&owner("2.21")), "2.21");
        assert_eq!(of_set(&owner(writable())), writable());
        // The written text, not a decoded or numeric equivalent.
        assert_eq!(of_set(&owner("2.&#50;1")), "2.&#50;1");
        // Without a readable version the owner is in the writable format.
        assert_eq!(of_set("<MetaDataObject/>"), writable());
        assert_eq!(of_set("not xml"), writable());

        let child = crate::edit::wrap_standalone_descriptor("<Form/>", &of_set(&owner("2.21")));
        let child = String::from_utf8(child).unwrap();
        assert!(child.contains("version=\"2.21\""), "{child}");
        let content = crate::exchange_content::empty_content("2.21");
        assert!(content.contains("version=\"2.21\""), "{content}");
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
