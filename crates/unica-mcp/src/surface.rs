use super::*;

/// Executes one tool call synchronously without leaking SDK types into the application.
/// Injectable so transport tests can substitute slow or failing tools.
pub type ToolCallHandler = dyn Fn(
        &str,
        &Map<String, Value>,
        CancellationToken,
        Arc<dyn ProgressSink>,
    ) -> Result<OperationResult, (i32, String)>
    + Send
    + Sync;

#[derive(Clone)]
pub enum SurfaceToolRouter {
    #[allow(dead_code)] // constructed only by the explicit legacy test seam
    LegacyV12(Arc<ToolCallHandler>),
    CanonicalV13(CanonicalDaemonRouter),
}
