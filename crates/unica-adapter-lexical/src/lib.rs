//! Lexical search through the owned process runner: candidate lines of the
//! search without a role, and the provider of the former `code.search`.
pub mod lines;
pub mod provider;
pub use provider::GitGrepProvider;
