//! The Designer export dialect of the form codec: what this codec knows about
//! the versions of the files it reads and writes. A new export format version
//! edits this module, not the readers and writers that ask it.
pub(crate) mod root_version;
pub(crate) mod v2_20;

/// The values an enumerated form property admits in the export format the
/// codec writes.
pub(crate) fn enum_values(xml_name: &str) -> Option<&'static [&'static str]> {
    v2_20::enum_values(xml_name)
}
