//! Keep reading explicit cancellation while the SDK awaits an inline opener.
use super::manual_calls::ManualCalls;
use rmcp::model::{
    ClientJsonRpcMessage, ClientNotification, ClientRequest, ErrorData, ServerJsonRpcMessage,
};
use rmcp::transport::Transport;
use rmcp::RoleServer;
use std::sync::Arc;
use tokio::sync::{mpsc, oneshot};

type Completion<E> = oneshot::Sender<Result<(), PumpError<E>>>;
enum Outbound<E> {
    Send(Box<ServerJsonRpcMessage>, Completion<E>),
    Close(Completion<E>),
}

#[derive(Debug)]
pub(super) enum PumpError<E> {
    Transport(E),
    Closed,
    Panicked,
}

impl<E: std::fmt::Display> std::fmt::Display for PumpError<E> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Transport(error) => error.fmt(formatter),
            Self::Closed => formatter.write_str("MCP transport closed"),
            Self::Panicked => formatter.write_str("MCP transport send panicked"),
        }
    }
}
impl<E: std::error::Error + 'static> std::error::Error for PumpError<E> {}

async fn finish_send<E>(
    sending: impl std::future::Future<Output = Result<(), E>> + Send + 'static,
) -> Result<(), PumpError<E>> {
    let mut sending = Box::pin(sending);
    let result = std::future::poll_fn(|context| {
        match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            sending.as_mut().poll(context)
        })) {
            Ok(std::task::Poll::Ready(result)) => {
                std::task::Poll::Ready(result.map_err(PumpError::Transport))
            }
            Ok(std::task::Poll::Pending) => std::task::Poll::Pending,
            Err(_) => std::task::Poll::Ready(Err(PumpError::Panicked)),
        }
    })
    .await;
    if std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| drop(sending))).is_err() {
        return Err(PumpError::Panicked);
    }
    result
}

pub(super) struct ReceivePump<E> {
    incoming: mpsc::UnboundedReceiver<ClientJsonRpcMessage>,
    outgoing: mpsc::UnboundedSender<Outbound<E>>,
    calls: Arc<ManualCalls>,
}

impl<E: std::error::Error + Send + Sync + 'static> ReceivePump<E> {
    pub(super) fn new<T: Transport<RoleServer, Error = E> + 'static>(
        mut inner: T,
        calls: Arc<ManualCalls>,
        canonical: bool,
    ) -> Self {
        let (incoming_tx, incoming) = mpsc::unbounded_channel();
        let (outgoing, mut commands) = mpsc::unbounded_channel();
        let owner = calls.clone();
        tokio::spawn(async move {
            let mut incoming_tx = Some(incoming_tx);
            let mut sends = tokio::task::JoinSet::new();
            let mut failed_sends = Vec::new();
            let close_completion = loop {
                tokio::select! {
                    message = inner.receive(), if incoming_tx.is_some() => {
                        let Some(message) = message else {
                            // Input EOF does not close the output or accepted work.
                            incoming_tx.take();
                            continue;
                        };
                        if canonical {
                            if let ClientJsonRpcMessage::Request(request) = &message {
                                let duplicate = !calls.admit(request.id.clone(), matches!(&request.request, ClientRequest::CallToolRequest(_)));
                                if duplicate {
                                    let response = ServerJsonRpcMessage::error(
                                        ErrorData::invalid_request("request id is already active", None),
                                        Some(request.id.clone()),
                                    );
                                    let sending = inner.send(response);
                                    sends.spawn(async move { (None, finish_send(sending).await) });
                                    continue;
                                }
                            }
                            if let ClientJsonRpcMessage::Notification(notification) = &message {
                                if let ClientNotification::CancelledNotification(cancelled) = &notification.notification {
                                    if cancelled.params.request_id.as_ref().is_some_and(|id| calls.cancel(id)) {
                                        continue;
                                    }
                                }
                            }
                        }
                        if incoming_tx.as_ref().is_some_and(|sender| sender.send(message).is_err()) {
                            break None;
                        }
                    }
                    command = commands.recv() => {
                        match command {
                            Some(Outbound::Send(message, completion)) => {
                                // This future may already own partial output. Its
                                // lifetime is independent of the SDK receiver.
                                let sending = inner.send(*message);
                                sends.spawn(async move { (Some(completion), finish_send(sending).await) });
                            }
                            Some(Outbound::Close(completion)) => break Some(completion),
                            None => break None,
                        }
                    }
                    completed = sends.join_next(), if !sends.is_empty() => {
                        match completed {
                            Some(Ok((Some(completion), Ok(())))) => { let _ = completion.send(Ok(())); }
                            Some(Ok((None, Ok(())))) => {}
                            Some(Ok(failed)) => { failed_sends.push(failed); break None; }
                            Some(Err(_)) => break None,
                            None => {}
                        }
                    }
                }
            };
            // Stop registration before closing and publishing its proof.
            drop(incoming_tx);
            let result = inner.close().await;
            while let Some(completed) = sends.join_next().await {
                if let Ok(completed) = completed {
                    failed_sends.push(completed);
                }
            }
            drop(inner);
            calls.transport_closed();
            for (completion, outcome) in failed_sends {
                if let Some(completion) = completion {
                    let _ = completion.send(outcome);
                }
            }
            if let Some(completion) = close_completion {
                let _ = completion.send(result.map_err(PumpError::Transport));
            }
        });
        Self {
            incoming,
            outgoing,
            calls: owner,
        }
    }
}

impl<E: std::error::Error + Send + Sync + 'static> Transport<RoleServer> for ReceivePump<E> {
    type Error = PumpError<E>;

    fn send(
        &mut self,
        message: ServerJsonRpcMessage,
    ) -> impl std::future::Future<Output = Result<(), Self::Error>> + Send + 'static {
        let (completion, finished) = oneshot::channel();
        let queued = self
            .outgoing
            .send(Outbound::Send(Box::new(message), completion))
            .is_ok();
        let calls = self.calls.clone();
        async move {
            let result = if queued {
                finished.await.unwrap_or(Err(PumpError::Closed))
            } else {
                Err(PumpError::Closed)
            };
            if result.is_err() {
                calls.wait_transport_closed().await;
            }
            result
        }
    }

    async fn receive(&mut self) -> Option<ClientJsonRpcMessage> {
        self.incoming.recv().await
    }

    fn close(&mut self) -> impl std::future::Future<Output = Result<(), Self::Error>> + Send {
        let (completion, finished) = oneshot::channel();
        let queued = self.outgoing.send(Outbound::Close(completion)).is_ok();
        let calls = self.calls.clone();
        async move {
            let result = if queued {
                finished.await.unwrap_or(Err(PumpError::Closed))
            } else {
                Err(PumpError::Closed)
            };
            if result.is_err() {
                calls.wait_transport_closed().await;
            }
            result
        }
    }
}
