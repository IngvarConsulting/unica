use crate::domain::source_target::{MetadataAddress, TargetKind};
use serde::Serialize;

/// Closed logical location shared by source navigation, code search, and diagnostics.
///
/// An unaddressable location is still contained by a named source set. It is
/// observable evidence, not a stable metadata identity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum SourceLocation {
    Addressed {
        source_set: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        metadata_path: Option<MetadataAddress>,
        target_kind: TargetKind,
    },
    Unaddressable {
        source_set: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        owner_metadata_path: Option<MetadataAddress>,
        path: String,
    },
}

/// Why a contained path carries no logical metadata address.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum LocateRejection {
    OutsideSourceSet,
    NotAddressable,
    OwnerUnproven,
}
