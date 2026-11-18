//! `DistributedInfoBase`: its typed property specification and checks.
use super::*;

const EXCHANGE_PLAN_KINDS: &[MetadataKind] = &[MetadataKind::ExchangePlan];

pub(super) const SPEC: MetadataPropertySpec = property(
    "DistributedInfoBase",
    MetaPropertyKey::DistributedInfoBase,
    MetaPropertyValueKind::Boolean,
    EXCHANGE_PLAN_KINDS,
);
