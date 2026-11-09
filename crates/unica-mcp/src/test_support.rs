//! Test support of the MCP door: the scripted fake daemon, an in-memory MCP
//! client and the cases that the door's own tests and the daemon's tests with
//! a live daemon both run.

mod fake_daemon;
mod router;
mod server;

pub use fake_daemon::*;
pub use router::*;
pub use server::*;

pub use rmcp;

// Внутренности двери, которые тесты демона собирают вокруг живого демона.
pub use crate::canonical_cancellation::*;
pub use crate::catalog::tool_definitions;
pub use crate::daemon_router::*;
pub use crate::in_flight::*;
pub use crate::manual_calls::ManualCalls;
pub use crate::server::UnicaServer;
pub use crate::surface::*;
pub use crate::tool_result::*;
pub use crate::transport::*;
pub mod manual_calls {
    pub use crate::manual_calls::*;
}
pub mod task_projection {
    pub use crate::task_projection::*;
}
