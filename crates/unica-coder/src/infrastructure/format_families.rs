//! The format families the product plugs in (INV.APP.FORMAT-FAMILY-EXTENSION).
//!
//! Each file of `format_families/` declares one family as `FAMILY`; build.rs
//! declares the modules and lists their families, so a new family is a new
//! file here and its crate's dependency, without an edit to a shared list.

use unica_source_contract::format_identity::{FormatFamily, FormatFamilyId};
use unica_source_contract::SourceFormat;

include!(concat!(env!("OUT_DIR"), "/format_families_modules.rs"));
include!(concat!(env!("OUT_DIR"), "/format_families_registry.rs"));

/// Every plugged-in format family; in tests, also the families declared
/// only for tests.
pub fn format_families() -> impl Iterator<Item = &'static dyn FormatFamily> {
    let declared = DECLARED_FAMILIES.iter().copied();
    #[cfg(test)]
    let declared = declared.chain(super::format_family_tests::declared().iter().copied());
    declared
}

/// The plugged-in family named `id`.
pub fn format_family(id: FormatFamilyId) -> Option<&'static dyn FormatFamily> {
    format_families().find(|family| family.id() == id)
}

/// Whether the product reads sources of an observed format: its family is
/// plugged in.
pub fn is_supported(format: SourceFormat) -> bool {
    format.family_id().and_then(format_family).is_some()
}

#[cfg(test)]
mod tests {
    use super::{format_families, format_family, is_supported};
    use std::cmp::Ordering;
    use unica_source_contract::format_identity::test_support::TOOL_LINE_FAMILY;
    use unica_source_contract::format_identity::{FormatFamilyId, FormatKey, PlatformLine};
    use unica_source_contract::SourceFormat;

    /// rule: INV.APP.FORMAT-FAMILY-EXTENSION
    #[test]
    fn a_family_the_product_never_saw_plugs_in_by_its_declaration_alone() {
        // `format_family_tests/tool_line.rs` is the whole registration: the
        // profile, the application and the Designer family are untouched.
        let family = format_family(TOOL_LINE_FAMILY).expect("the test family is plugged in");
        let key = |tool: &str, line: &str| FormatKey::new([tool, line]);

        // Its tuple carries the line; the family alone reads and orders it.
        assert_eq!(
            family.platform_line(&key("2", "8.3.26")),
            Some(PlatformLine::new(8, 3, 26))
        );
        assert_eq!(
            family.compare(&key("1", "8.3.27"), &key("2", "8.3.27")),
            Some(Ordering::Less)
        );
        assert_eq!(
            family.compare(&key("2", "8.3.26"), &key("1", "8.3.27")),
            None
        );
        assert_eq!(family.known_keys(), vec![key("1", "8.3.27")]);

        // A tuple of another family means nothing to it.
        let designer = format_family(FormatFamilyId::new("platform_xml")).unwrap();
        assert_eq!(designer.platform_line(&key("1", "8.3.27")), None);
        assert_eq!(family.platform_line(&FormatKey::new(["2.20"])), None);
    }

    /// rule: INV.APP.FORMAT-FAMILY-EXTENSION
    #[test]
    fn declared_families_are_plugged_in_by_their_names_once_each() {
        let families = format_families().collect::<Vec<_>>();
        for (index, family) in families.iter().enumerate() {
            for other in &families[index + 1..] {
                assert_ne!(family.id(), other.id(), "a family is declared twice");
            }
            assert!(
                !family.known_keys().is_empty(),
                "{} knows no format",
                family.id()
            );
        }
        let designer = format_family(FormatFamilyId::new("platform_xml")).unwrap();
        assert_eq!(
            designer.platform_line(&FormatKey::new(["2.20"])),
            Some(PlatformLine::new(8, 3, 27))
        );
        // A format is supported when its family is plugged in, not by a list of formats.
        assert!(is_supported(SourceFormat::PlatformXml));
        assert!(!is_supported(SourceFormat::Edt));
        assert!(!is_supported(SourceFormat::Unknown));
        assert!(!is_supported(SourceFormat::Invalid));
        // The same answer as the observed format's own classification today.
        for format in [
            SourceFormat::PlatformXml,
            SourceFormat::Edt,
            SourceFormat::Unknown,
            SourceFormat::Invalid,
        ] {
            assert_eq!(is_supported(format), format.is_supported(), "{format:?}");
        }
    }
}
