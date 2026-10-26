//! The editor door of the daemon (#2171): documents, URIs and positions as
//! Unica models them, with no transport. The LSP route calls into these.

pub(crate) mod document;
pub(crate) mod position;
pub(crate) mod uri;
