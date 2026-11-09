//! The Designer export format family.

pub(super) const FAMILY: &dyn unica_source_contract::format_identity::FormatFamily =
    &unica_source_designer::format_profile::PlatformXmlFamily;
