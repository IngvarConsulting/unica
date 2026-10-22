// Hidden v0.13 apply foundation remains unreachable from the package-selected v0.12 surface.
#[allow(dead_code)]
pub(crate) mod apply;
pub(crate) mod call_graph_identity;
pub mod engine;
// Hidden v0.13 module projection remains unreachable from the package-selected v0.12 surface.
pub(crate) mod logical_route;
#[allow(dead_code)]
pub(crate) mod node_view;
#[cfg(test)]
mod operation_contract;
#[allow(dead_code)]
pub(crate) mod source_capabilities;
