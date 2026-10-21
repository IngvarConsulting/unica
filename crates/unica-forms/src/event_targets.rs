//! Typed logical property event owner identities; no source layout or XML.
use unica_address::address::QualifiedAddress;
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PropertyEventOwnerKind {
    Form,
    Element,
    Table,
    Column,
    Command,
    /// A direct logical `Item` has no element-type discriminator. This value
    /// never escapes the strict resolver; Form evidence must refine it first.
    UnresolvedItem,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PropertyEventOwner {
    pub kind: PropertyEventOwnerKind,
    pub at: QualifiedAddress,
}
