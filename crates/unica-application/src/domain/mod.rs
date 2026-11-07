// Hidden v0.13 apply foundation remains unreachable from the package-selected v0.12 surface.
#[allow(dead_code)]
pub mod apply;
pub mod call_graph_identity;
pub mod engine;
// Hidden v0.13 module projection remains unreachable from the package-selected v0.12 surface.
pub mod logical_route;
#[allow(dead_code)]
pub mod node_view;
#[cfg(test)]
mod operation_contract;
#[allow(dead_code)]
pub mod source_capabilities;
