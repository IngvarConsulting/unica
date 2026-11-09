// module: private
//! The Designer export dialect of the metadata codec: what this codec knows
//! about the versions of the files it reads and writes. A new export format
//! version edits this module, not the readers and writers that ask it.
pub(crate) mod root_version;
pub(crate) mod v2_20;

use unica_metadata::MetadataKind;
use unica_source_contract::format_identity::{FormatKey, ReadResolution};

/// The tables of the metadata codec for one export format version.
pub(crate) struct MetadataDialect {
    /// The export format version the tables describe.
    pub(crate) version: &'static str,
    /// The platform line that writes that version.
    pub(crate) platform_line: &'static str,
    pub(crate) kind_declares_child_objects: fn(MetadataKind) -> bool,
    pub(crate) boolean_properties: fn(&str) -> &'static [&'static str],
}

static V2_20: MetadataDialect = MetadataDialect {
    version: root_version::writable(),
    platform_line: root_version::writable_platform_line_const(),
    kind_declares_child_objects: v2_20::kind_declares_child_objects,
    boolean_properties: v2_20::boolean_properties,
};

/// The dialects the codec has, one per export format version.
static DIALECTS: &[&MetadataDialect] = &[&V2_20];

/// The dialect of the version the codec writes.
pub(crate) fn writable() -> &'static MetadataDialect {
    &V2_20
}

/// The dialect that reads a root recording `version`: its own, or the one
/// the format family names for it (INV.SOURCE.WRITABLE-PROFILE). A version
/// the family does not read is read by the writable dialect, so a check
/// still reports what it finds; the format guard refuses the read itself.
pub(crate) fn for_version(version: Option<&str>) -> &'static MetadataDialect {
    let key = match unica_source_designer::format_profile::resolve_platform_xml_read(version) {
        ReadResolution::Dialect => FormatKey::new([version.unwrap_or_default()]),
        ReadResolution::Unverified { read_as } => read_as,
        ReadResolution::Unsupported => return writable(),
    };
    DIALECTS
        .iter()
        .find(|dialect| matches!(key.parts(), [version] if version == dialect.version))
        .copied()
        .unwrap_or_else(writable)
}

#[cfg(test)]
mod tests {
    use super::{for_version, writable};

    /// rule: INV.SOURCE.WRITABLE-PROFILE
    #[test]
    fn a_root_is_read_by_the_dialect_its_family_names() {
        // 2.20 has a dialect of its own; known versions without one and
        // versions newer than every known one are read by it; a version the
        // family does not read falls back to the writable dialect for checks.
        for version in [Some("2.20"), Some("2.17"), Some("2.22"), Some("2.30"), None] {
            assert!(std::ptr::eq(for_version(version), writable()), "{version:?}");
        }
        assert_eq!(writable().version, super::root_version::writable());
        assert_eq!(
            writable().platform_line,
            super::root_version::writable_platform_line()
        );
    }
}
