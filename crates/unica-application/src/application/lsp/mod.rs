//! The editor door of the daemon (#2171): documents, URIs and positions as
//! Unica models them, with no transport. The LSP route calls into these.

pub mod document;
pub mod position;
pub mod uri;
