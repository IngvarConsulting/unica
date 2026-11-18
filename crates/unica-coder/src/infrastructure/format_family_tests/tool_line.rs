//! A family only tests know: it plugs in by this file alone.

pub(super) const FAMILY: &dyn unica_source_contract::format_identity::FormatFamily =
    &unica_source_contract::format_identity::test_support::ToolLineFamily;
