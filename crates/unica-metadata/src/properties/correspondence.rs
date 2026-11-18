//! `Correspondence`: its typed property specification and checks.
use super::*;

pub(super) const SPEC: MetadataPropertySpec = property(
    "Correspondence",
    MetaPropertyKey::Correspondence,
    MetaPropertyValueKind::Boolean,
    ACCOUNTING_REGISTER_KINDS,
);
