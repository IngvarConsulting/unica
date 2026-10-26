//! Обмены маршрута LSP. Сессию открывает и закрывает её собственное
//! соединение; запросы идут по своим соединениям, отмена — по отдельному.
use super::*;
use crate::protocol_v5::lsp::{
    V5LspCall, V5LspFailure, V5LspNotification, V5LspOpening, V5LspOutcome,
    V5LspServerNotification, V5LspSessionId, V5LspWorkspaceFolder,
};

/// Срок короткого обмена отмены: она не ждёт ничего, кроме записи флага.
const LSP_CANCEL_TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Debug)]
pub enum V5LspExchangeError {
    /// Обмен с демоном не состоялся.
    Transport(String),
    /// Демон отказал кодом Unica.
    Refused(V5LspFailure),
    /// Демон ответил не тем видом ответа.
    Unexpected,
}

impl fmt::Display for V5LspExchangeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Transport(error) => formatter.write_str(error),
            Self::Refused(failure) => {
                write!(formatter, "daemon refused the LSP request: {failure:?}")
            }
            Self::Unexpected => formatter.write_str("daemon answered the LSP request unexpectedly"),
        }
    }
}

impl V5DaemonProcessOwner {
    pub fn lsp_open_before(
        &mut self,
        workspace_folders: Vec<V5LspWorkspaceFolder>,
        deadline: Instant,
    ) -> Result<(V5LspSessionId, V5LspOpening), V5LspExchangeError> {
        match self.lsp_exchange(
            V5ClientRequest::LspOpen { workspace_folders },
            "lsp open",
            deadline,
        )? {
            V5ServerResponse::LspOpened {
                session,
                capabilities,
                workspaces,
            } => Ok((
                session,
                V5LspOpening {
                    capabilities,
                    workspaces,
                },
            )),
            other => Err(unexpected(other)),
        }
    }

    pub fn lsp_notify_before(
        &mut self,
        session: &V5LspSessionId,
        notification: V5LspNotification,
        deadline: Instant,
    ) -> Result<(), V5LspExchangeError> {
        let request = V5ClientRequest::LspNotify {
            session: session.clone(),
            notification,
        };
        accepted(self.lsp_exchange(request, "lsp notify", deadline)?)
    }

    pub fn lsp_call_before(
        &mut self,
        session: &V5LspSessionId,
        request: u64,
        budget_ms: u64,
        call: V5LspCall,
        deadline: Instant,
    ) -> Result<V5LspOutcome, V5LspExchangeError> {
        let request = V5ClientRequest::LspCall {
            session: session.clone(),
            request,
            budget_ms,
            call,
        };
        match self.lsp_exchange(request, "lsp call", deadline)? {
            V5ServerResponse::LspAnswer { outcome } => Ok(outcome),
            other => Err(unexpected(other)),
        }
    }

    pub fn lsp_poll_before(
        &mut self,
        session: &V5LspSessionId,
        wait_ms: u64,
        deadline: Instant,
    ) -> Result<Vec<V5LspServerNotification>, V5LspExchangeError> {
        let request = V5ClientRequest::LspPoll {
            session: session.clone(),
            wait_ms,
        };
        match self.lsp_exchange(request, "lsp poll", deadline)? {
            V5ServerResponse::LspNotifications { items } => Ok(items),
            other => Err(unexpected(other)),
        }
    }

    pub fn lsp_close_before(
        &mut self,
        session: &V5LspSessionId,
        deadline: Instant,
    ) -> Result<(), V5LspExchangeError> {
        let request = V5ClientRequest::LspClose {
            session: session.clone(),
        };
        accepted(self.lsp_exchange(request, "lsp close", deadline)?)
    }

    /// Отменить запрос отдельным соединением: соединение запроса занято
    /// его ожиданием.
    pub fn lsp_cancel_at_endpoint(
        record: V5EndpointRecord,
        session: &V5LspSessionId,
        request: u64,
    ) -> Result<(), V5LspExchangeError> {
        let deadline = Instant::now() + LSP_CANCEL_TIMEOUT;
        let mut owner =
            Self::connect_before(record, deadline).map_err(V5LspExchangeError::Transport)?;
        let request = V5ClientRequest::LspCancel {
            session: session.clone(),
            request,
        };
        accepted(owner.lsp_exchange(request, "lsp cancel", deadline)?)
    }

    fn lsp_exchange(
        &mut self,
        request: V5ClientRequest,
        stage: &'static str,
        deadline: Instant,
    ) -> Result<V5ServerResponse, V5LspExchangeError> {
        match self.exchange_typed_before(request, stage, deadline) {
            Ok(V5ServerResponse::LspRefused { failure }) => {
                Err(V5LspExchangeError::Refused(failure))
            }
            Ok(response) => Ok(response),
            Err(error) => Err(V5LspExchangeError::Transport(error.to_string())),
        }
    }
}

fn accepted(response: V5ServerResponse) -> Result<(), V5LspExchangeError> {
    match response {
        V5ServerResponse::LspAccepted {} => Ok(()),
        other => Err(unexpected(other)),
    }
}

fn unexpected(_: V5ServerResponse) -> V5LspExchangeError {
    V5LspExchangeError::Unexpected
}
