use crate::index::IndexReadiness;
use crate::session::{RlmServiceResponse, WorkspaceServiceRlmCall};
use serde_json::{Map, Value};
use std::path::Path;
use std::time::Duration;
use unica_analysis_services::protocol::WorkspaceRlmOperation;
use unica_analysis_services::WorkspaceServiceManager;
use unica_source_contract::workspace::WorkspaceContext;
use unica_work_control::cancellation::CancellationToken;
pub trait RlmServiceClient {
    fn rlm_readiness(
        &self,
        context: &WorkspaceContext,
        source: &Path,
        args: &Map<String, Value>,
    ) -> Result<IndexReadiness, String>;
    fn rlm_readiness_cancellable(
        &self,
        context: &WorkspaceContext,
        source: &Path,
        args: &Map<String, Value>,
        cancel: &CancellationToken,
    ) -> Result<IndexReadiness, String>;
    fn call_rlm_cancellable(
        &self,
        context: &WorkspaceContext,
        source: &Path,
        operation: WorkspaceRlmOperation,
        timeout: Duration,
        cancel: &CancellationToken,
    ) -> Result<WorkspaceServiceRlmCall, String>;
    fn rlm_readiness_cancellable_with_timeout(
        &self,
        context: &WorkspaceContext,
        source: &Path,
        args: &Map<String, Value>,
        timeout: Duration,
        cancel: &CancellationToken,
    ) -> Result<IndexReadiness, String>;
}
impl RlmServiceClient for WorkspaceServiceManager<'_> {
    fn rlm_readiness(
        &self,
        context: &WorkspaceContext,
        source: &Path,
        args: &Map<String, Value>,
    ) -> Result<IndexReadiness, String> {
        Ok(self
            .rlm_readiness_response(context, source, args)?
            .index_readiness())
    }
    fn rlm_readiness_cancellable(
        &self,
        context: &WorkspaceContext,
        source: &Path,
        args: &Map<String, Value>,
        cancel: &CancellationToken,
    ) -> Result<IndexReadiness, String> {
        Ok(self
            .rlm_readiness_cancellable_response(context, source, args, cancel)?
            .index_readiness())
    }
    fn call_rlm_cancellable(
        &self,
        context: &WorkspaceContext,
        source: &Path,
        operation: WorkspaceRlmOperation,
        timeout: Duration,
        cancel: &CancellationToken,
    ) -> Result<WorkspaceServiceRlmCall, String> {
        self.call_rlm_response(context, source, operation, timeout, cancel)?
            .into_rlm_call()
    }
    fn rlm_readiness_cancellable_with_timeout(
        &self,
        context: &WorkspaceContext,
        source: &Path,
        args: &Map<String, Value>,
        timeout: Duration,
        cancel: &CancellationToken,
    ) -> Result<IndexReadiness, String> {
        Ok(self
            .rlm_readiness_cancellable_with_timeout_response(
                context, source, args, timeout, cancel,
            )?
            .index_readiness())
    }
}
