//! The implementations the daemon reaches, chosen by the composition.
//!
//! The daemon routes requests; which engine does the work is decided here,
//! so the daemon names no adapter crate. Replacing an engine changes this
//! module and the composition beside it, not the daemon's manifest.

/// Candidate lines of the literal text search.
pub mod lexical {
    pub use unica_adapter_lexical::lines::{
        holds_foreign_work_tree, inside_work_tree, stream_candidate_lines, CandidateCorpus,
        CandidateEnd, CandidatePattern,
    };
}

/// The platform runtime: its job request and the full dump publication.
pub mod runtime {
    pub use unica_adapter_v8_runner::full_dump::{
        attest_selected_local, prepare_publication, write_projection, FullDumpSelection,
    };
    pub use unica_adapter_v8_runner::request::RuntimeJobRequest;

    #[cfg(any(test, feature = "test-support"))]
    pub use unica_adapter_v8_runner::backend::CoordinationOnlyRunner;
    #[cfg(any(test, feature = "test-support"))]
    pub use unica_adapter_v8_runner::full_dump::with_test_platform;
}

/// Diagnostics providers that record or fail the analysis deadline.
#[cfg(any(test, feature = "test-support"))]
pub mod diagnostics_probe {
    pub use unica_adapter_bsl_analyzer::adapter::AnalyzerRunFailureKind;
    pub use unica_adapter_bsl_analyzer::diagnostics::analyze_timeout_probe::{
        failing_provider, recording_provider,
    };
}
