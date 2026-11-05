//! `CheckUnique`: its typed property specification and checks.
use super::*;

const CHECK_UNIQUE_KINDS: &[MetadataKind] = &[
    MetadataKind::Catalog,
    MetadataKind::Document,
    MetadataKind::ChartOfAccounts,
    MetadataKind::ChartOfCharacteristicTypes,
    MetadataKind::BusinessProcess,
    MetadataKind::Task,
];

pub(super) const SPEC: MetadataPropertySpec = property(
    "CheckUnique",
    MetaPropertyKey::CheckUnique,
    MetaPropertyValueKind::Boolean,
    CHECK_UNIQUE_KINDS,
);
