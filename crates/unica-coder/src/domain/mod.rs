pub mod cache;
pub mod cancellation;
// Module-kind entries are consumed by the staged discovery providers before
// every registry API has a non-test caller.
#[allow(dead_code)]
pub(crate) mod discovery_registry;
pub mod events;
pub mod project_sources;
pub mod source_roots;
pub(crate) mod source_snapshot;
pub mod workspace;
