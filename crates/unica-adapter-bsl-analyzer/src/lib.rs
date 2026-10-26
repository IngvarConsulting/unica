//! BSL analyzer transport, diagnostics and current-source intelligence implementation.
pub mod adapter;
pub mod baseline;
pub mod diagnostics;
mod editor_codes;
pub mod environment;
pub mod filters;
pub mod jsonl;
mod private_failure;
pub mod provider;
pub mod session;

#[cfg(any(test, feature = "test-support"))]
pub mod test_support {
    pub fn write_private_failure_record(
        writer: &mut dyn std::io::Write,
        snapshot: unica_process::transport_failure::PrivateFailureSnapshot,
    ) -> std::io::Result<()> {
        crate::private_failure::write_record_to(writer, snapshot)
    }
}
