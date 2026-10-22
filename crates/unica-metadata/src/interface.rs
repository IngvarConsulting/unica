//! Existing typed interface facts.
#[derive(Default)]
pub struct InterfaceEditCounters {
    pub added: usize,
    pub removed: usize,
    pub modified: usize,
}

/// Typed answer of `unica.interface.edit` (ADR-0023): what the edit did to the
/// command interface, beside the file it updated.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InterfaceEditData {
    pub added: usize,
    pub removed: usize,
    pub modified: usize,
    /// The edit commits only through post-validation, so this records that the
    /// validator actually ran over the written file — not merely that the write
    /// succeeded.
    pub validated: bool,
    pub mutation: unica_execution_contract::mutation::MutationData,
}
