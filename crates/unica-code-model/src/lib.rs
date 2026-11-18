//! The model of answers about BSL code that the editor service turns into its
//! protocol: symbols and completion. Plain data without parsing, so a service
//! that only translates answers does not depend on the analysis that builds
//! them; `unica-code-intelligence` builds them and re-exports these types.

pub mod completion;
pub mod symbols;
