//! Format families declared only for tests. Each file of
//! `format_family_tests/` declares one family as `FAMILY`, the way
//! `format_families/` declares the product's (INV.APP.FORMAT-FAMILY-EXTENSION).

use unica_source_contract::format_identity::FormatFamily;

include!(concat!(env!("OUT_DIR"), "/format_family_tests_modules.rs"));
include!(concat!(env!("OUT_DIR"), "/format_family_tests_registry.rs"));

pub(super) fn declared() -> &'static [&'static dyn FormatFamily] {
    DECLARED_TEST_FAMILIES
}
