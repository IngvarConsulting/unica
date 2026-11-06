//! Подтверждение предыдущей диагностики после доставки при открытии MCP.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use rmcp::model::{RequestId, ServerJsonRpcMessage, ServerResult};

#[derive(Clone, Default)]
pub(super) struct StartupNoticeDelivery(Arc<Mutex<Delivery>>);

#[derive(Default)]
struct Delivery {
    notice: Option<String>,
    opener: Option<RequestId>,
    notice_sent: bool,
    opener_sent: bool,
}

#[derive(Clone, Copy, Default)]
pub(super) struct SentResponse {
    notice: bool,
    opener: bool,
}

impl StartupNoticeDelivery {
    pub(super) fn new(notice: Option<String>) -> Self {
        Self(Arc::new(Mutex::new(Delivery {
            notice,
            ..Delivery::default()
        })))
    }

    pub(super) fn opener(&self, id: RequestId) {
        self.0.lock().expect("startup notice delivery").opener = Some(id);
    }

    /// Классификация до отправки не меняет состояние доставки.
    pub(super) fn response(&self, message: &ServerJsonRpcMessage) -> SentResponse {
        let ServerJsonRpcMessage::Response(response) = message else {
            return SentResponse::default();
        };
        let delivery = self.0.lock().expect("startup notice delivery");
        let instructions = match &response.result {
            ServerResult::InitializeResult(result) => result.instructions.as_deref(),
            ServerResult::DiscoverResult(result) => result.instructions.as_deref(),
            _ => None,
        };
        SentResponse {
            notice: delivery.notice.as_ref().is_some_and(|notice| {
                instructions.is_some_and(|instructions| {
                    instructions.ends_with(&format!("Startup notice: {notice}"))
                })
            }),
            opener: delivery.opener.as_ref() == Some(&response.id),
        }
    }

    /// Вызывается только после завершившейся реальной записи, не при
    /// подавлении отменённого ответа и не при постановке его в очередь.
    pub(super) fn sent(&self, response: SentResponse) {
        let mut delivery = self.0.lock().expect("startup notice delivery");
        delivery.notice_sent |= response.notice;
        delivery.opener_sent |= response.opener;
    }

    pub(super) fn initialized(&self, acknowledgement: Option<StartupNoticeAck>) {
        let delivery = self.0.lock().expect("startup notice delivery");
        let delivered = delivery.notice_sent && delivery.opener_sent;
        drop(delivery);
        if delivered {
            if let Some(acknowledgement) = acknowledgement {
                // Неудачный ACK допускает повтор сообщения, но не отменяет
                // уже установленный сеанс и не выдаёт сырой журнал в stderr.
                let _ = unica_artifact_delivery::AttemptLog::in_cache(&acknowledgement.cache)
                    .report_ids(&acknowledgement.attempts);
            }
        }
    }
}

pub(super) struct StartupNoticeAck {
    cache: PathBuf,
    attempts: Vec<String>,
}

impl StartupNoticeAck {
    pub(super) fn from_environment() -> Option<Self> {
        let cache = std::env::var_os("UNICA_ARTIFACT_CACHE").filter(|value| !value.is_empty())?;
        let attempts =
            serde_json::from_str(&std::env::var("UNICA_STARTUP_NOTICE_ACK").ok()?).ok()?;
        Some(Self {
            cache: cache.into(),
            attempts,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::interfaces::mcp::{DiscoveryProbeTransport, ToolCallHandler, UnicaServer};
    use rmcp::transport::Transport;
    use rmcp::{RoleServer, ServerHandler};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use unica_artifact_delivery::{AttemptLog, AttemptSubject};

    struct ObservedWrite {
        writes: Arc<AtomicUsize>,
        fail: bool,
    }

    impl Transport<RoleServer> for ObservedWrite {
        type Error = std::io::Error;

        fn send(
            &mut self,
            _: ServerJsonRpcMessage,
        ) -> impl std::future::Future<Output = Result<(), Self::Error>> + Send + 'static {
            self.writes.fetch_add(1, Ordering::SeqCst);
            let fail = self.fail;
            async move {
                if fail {
                    Err(std::io::Error::from(std::io::ErrorKind::BrokenPipe))
                } else {
                    Ok(())
                }
            }
        }

        async fn receive(&mut self) -> Option<rmcp::model::ClientJsonRpcMessage> {
            std::future::pending().await
        }

        async fn close(&mut self) -> Result<(), Self::Error> {
            Ok(())
        }
    }

    /// rule: INV.PKG.STARTUP-NOTICE-RETENTION
    #[tokio::test]
    async fn failed_or_suppressed_transport_send_never_acknowledges_the_notice() {
        for (fail, suppressed, expected_writes, delivered) in [
            (true, false, 1, false),
            (false, true, 0, false),
            (false, false, 1, true),
        ] {
            let scratch = tempfile::tempdir().unwrap();
            let cache = scratch.path().join("cache");
            let log = AttemptLog::in_cache(&cache);
            let attempt = log
                .open(AttemptSubject {
                    artifact: "unica".into(),
                    version: "test".into(),
                    target: "test".into(),
                    url: "https://example.invalid/runtime".into(),
                    partial: scratch.path().join("partial"),
                })
                .unwrap();
            drop(attempt);
            let ids = log
                .unfinished()
                .unwrap()
                .into_iter()
                .map(|attempt| attempt.id)
                .collect();
            let handler: Arc<ToolCallHandler> = Arc::new(|_, _, _, _| unreachable!());
            let server = UnicaServer::legacy_with_startup_notice_for_test(
                handler,
                Some("previous failure".into()),
            );
            let writes = Arc::new(AtomicUsize::new(0));
            let mut transport = DiscoveryProbeTransport::new(
                ObservedWrite {
                    writes: writes.clone(),
                    fail,
                },
                &server,
            );
            let id = RequestId::Number(1);
            transport.startup_delivery.opener(id.clone());
            if suppressed {
                let calls = server.manual_calls.clone();
                assert!(calls.admit(id.clone(), true));
                assert!(calls.cancel(&id));
                transport.manual_calls = Some(calls);
            }
            let response = ServerJsonRpcMessage::response(
                ServerResult::InitializeResult(server.get_info()),
                id,
            );
            let sent = transport.send(response).await;
            assert_eq!(sent.is_err(), fail);
            transport
                .startup_delivery
                .initialized(Some(StartupNoticeAck {
                    cache,
                    attempts: ids,
                }));
            assert_eq!(writes.load(Ordering::SeqCst), expected_writes);
            assert_eq!(log.unfinished().unwrap().is_empty(), delivered);
            // BrokenPipe already closes the receive pump.
            let _ = transport.close().await;
        }
    }
}
