//! The Designer export dialect of the form codec: what this codec knows about
//! the versions of the files it reads and writes. A new export format version
//! edits this module, not the readers and writers that ask it.
pub(crate) mod root_version;
pub(crate) mod v2_20;
pub(crate) mod v2_21;

/// The values an enumerated form property admits in the export format the
/// codec writes.
pub(crate) fn enum_values(xml_name: &str) -> Option<&'static [&'static str]> {
    v2_20::enum_values(xml_name)
}

/// The properties, with their default values, a new form descriptor in
/// `format_version` writes right after `UsePurposes`.
pub(crate) fn descriptor_properties_after_use_purposes(
    format_version: &str,
) -> &'static [&'static str] {
    if unica_source_designer::format_profile::export_format_at_least(format_version, v2_21::VERSION)
    {
        v2_21::DESCRIPTOR_PROPERTIES_AFTER_USE_PURPOSES
    } else {
        &[]
    }
}
