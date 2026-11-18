//! `AutoOrderByCode`: its typed property specification and checks.
use super::*;

pub(super) const SPEC: MetadataPropertySpec = property(
    "AutoOrderByCode",
    MetaPropertyKey::AutoOrderByCode,
    MetaPropertyValueKind::Boolean,
    CHART_OF_ACCOUNTS_KINDS,
);
