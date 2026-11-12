//! The platform runtime the daemon reaches, chosen by the composition.
//!
//! The daemon routes requests; which runtime does the work is decided here,
//! so the daemon names no adapter crate. The items are the adapter's own,
//! re-exported: replacing the runtime changes this module and the composition
//! beside it, not the daemon's manifest, though a changed item still reaches
//! the daemon's code. Analysis providers reach the daemon through the ports
//! of `code-intelligence` and `diagnostics`, chosen in `analysis_composition`.

/// The platform runtime: its job request.
pub mod runtime {
    pub use unica_adapter_v8_runner::request::RuntimeJobRequest;

    #[cfg(any(test, feature = "test-support"))]
    pub use unica_adapter_v8_runner::backend::CoordinationOnlyRunner;
}
