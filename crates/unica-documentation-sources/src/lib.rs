//! Concrete documentation sources, local retrieval and network policy.
//! Provider-neutral contracts remain in unica-documentation.

pub mod configuration_help;
pub mod documentation_policy;
pub(crate) mod documentation_retrieval;
pub use documentation_retrieval::BilingualLexicon;
pub mod kb_1ci;
pub mod platform_help;
#[cfg(test)]
#[path = "../tests/platform/platform_help_oracle.rs"]
mod platform_help_oracle;
pub mod standards_documentation;
pub mod standards_transport;
