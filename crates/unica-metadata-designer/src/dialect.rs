// module: private
//! The Designer export dialect of the metadata codec: what this codec knows
//! about the versions of the files it reads and writes. A new export format
//! version edits this module, not the readers and writers that ask it.
pub(crate) mod root_version;
pub(crate) mod v2_20;
pub(crate) mod v2_21;
pub(crate) mod v2_22;

use unica_metadata::MetadataKind;
use unica_source_contract::format_identity::{FormatKey, ReadResolution};

/// The tables of the metadata codec for one export format version.
pub(crate) struct MetadataDialect {
    /// The export format version the tables describe.
    pub(crate) version: &'static str,
    /// The platform line that writes that version.
    pub(crate) platform_line: &'static str,
    pub(crate) kind_declares_child_objects: fn(MetadataKind) -> bool,
    pub(crate) member_choice_name: fn(unica_metadata::MemberChoiceProperty) -> &'static str,
    pub(crate) boolean_properties: fn(&str) -> &'static [&'static str],
    /// Compatibility modes the platform line adds to the 8.3 modes every
    /// line knows: the line writes its own mode into the configuration.
    pub(crate) line_compatibility_modes: &'static [&'static str],
}

impl MetadataDialect {
    /// Whether `value` is a compatibility mode the platform line of this
    /// dialect knows beyond the 8.3 modes.
    pub(crate) fn knows_line_compatibility_mode(&self, value: &str) -> bool {
        self.line_compatibility_modes.contains(&value)
    }
}

static V2_20: MetadataDialect = MetadataDialect {
    version: root_version::writable(),
    platform_line: root_version::writable_platform_line_const(),
    kind_declares_child_objects: v2_20::kind_declares_child_objects,
    member_choice_name: v2_20::member_choice_name,
    boolean_properties: v2_20::boolean_properties,
    line_compatibility_modes: &[],
};

/// The dialects the codec has, one per export format version.
static DIALECTS: &[&MetadataDialect] = &[&V2_20, &v2_21::DIALECT, &v2_22::DIALECT];

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
        // 2.20, 2.21 and 2.22 have dialects of their own; 2.17 (known, older
        // than every dialect) is read by 2.20; a version newer than every
        // known one by the nearest older dialect; a version the family does
        // not read falls back to the writable dialect for checks.
        for (version, dialect) in [
            (Some("2.20"), "2.20"),
            (Some("2.21"), "2.21"),
            (Some("2.22"), "2.22"),
            (Some("2.17"), "2.20"),
            (Some("2.30"), "2.22"),
            (None, "2.20"),
        ] {
            assert_eq!(for_version(version).version, dialect, "{version:?}");
        }
        assert_eq!(for_version(Some("2.21")).platform_line, "8.5.1");
        assert!(for_version(Some("2.22")).knows_line_compatibility_mode("Version8_5_4"));
        assert!(!writable().knows_line_compatibility_mode("Version8_5_1"));
        assert_eq!(writable().version, super::root_version::writable());
        assert_eq!(
            writable().platform_line,
            super::root_version::writable_platform_line_const()
        );
    }
}
