use super::*;

/// `rmcp` treats its first `server/discover` as a permanent modern opener. A
/// host may probe first and then choose `initialize`; answer only that initial
/// probe before handing the actual opener to the SDK. Once an opener reaches
/// the SDK, all requests and their protocol checks belong to it unchanged.
pub struct DiscoveryProbeTransport<T: Transport<RoleServer>> {
    pub inner: receive_pump::ReceivePump<T::Error>,
    pub discovery: DiscoverResult,
    pub awaiting_opener: bool,
    pub manual_calls: Option<Arc<ManualCalls>>,
}

impl<T: Transport<RoleServer> + 'static> DiscoveryProbeTransport<T> {
    pub fn new(inner: T, server: &UnicaServer) -> Self {
        Self {
            inner: receive_pump::ReceivePump::new(
                inner,
                server.manual_calls.clone(),
                matches!(server.router, SurfaceToolRouter::CanonicalV13(_)),
            ),
            discovery: DiscoverResult::from_server_info(
                server.supported_protocol_versions().into_owned(),
                server.get_info(),
            ),
            awaiting_opener: true,
            manual_calls: matches!(server.router, SurfaceToolRouter::CanonicalV13(_))
                .then(|| server.manual_calls.clone()),
        }
    }
}

impl<T: Transport<RoleServer> + 'static> Transport<RoleServer> for DiscoveryProbeTransport<T> {
    type Error = receive_pump::PumpError<T::Error>;

    fn send(
        &mut self,
        item: ServerJsonRpcMessage,
    ) -> impl std::future::Future<Output = Result<(), Self::Error>> + Send + 'static {
        let tracked = self.manual_calls.as_ref().and_then(|calls| {
            let id = match &item {
                ServerJsonRpcMessage::Response(response) => Some(&response.id),
                ServerJsonRpcMessage::Error(error) => error.id.as_ref(),
                _ => None,
            }?;
            calls.get(id).map(|call| (calls.clone(), id.clone(), call))
        });
        if let Some((calls, _, call)) = &tracked {
            calls.response_seen(call);
        }
        // Once inner.send starts it may have written part of a JSON frame.
        // A later Cancel controls the daemon independently; it never drops
        // this send future or leaves a truncated frame on stdout.
        let suppressed = tracked
            .as_ref()
            .is_some_and(|(_, _, call)| call.cancellation.requested());
        let sending = (!suppressed).then(|| self.inner.send(item));
        async move {
            let result = match sending {
                Some(sending) => sending.await,
                None => Ok(()),
            };
            if result.is_ok() {
                if let Some((calls, id, call)) = tracked {
                    calls.delivery_done(&id, &call);
                }
            }
            result
        }
    }

    async fn receive(&mut self) -> Option<ClientJsonRpcMessage> {
        loop {
            let message = self.inner.receive().await?;
            if self.awaiting_opener {
                if let ClientJsonRpcMessage::Request(request) = &message {
                    match &request.request {
                        ClientRequest::DiscoverRequest(_)
                            if request
                                .request
                                .get_meta()
                                .missing_required_keys(&ProtocolVersion::V_2026_07_28)
                                .is_empty()
                                && request.request.get_meta().protocol_version().is_some_and(
                                    |version| self.discovery.supported_versions.contains(&version),
                                ) =>
                        {
                            let response = ServerJsonRpcMessage::response(
                                ServerResult::DiscoverResult(self.discovery.clone()),
                                request.id.clone(),
                            );
                            if let Err(error) = self.inner.send(response).await {
                                eprintln!("unica mcp discovery probe response failed: {error}");
                                return None;
                            }
                            continue;
                        }
                        ClientRequest::PingRequest(_) => return Some(message),
                        _ => {}
                    }
                }
                self.awaiting_opener = false;
            }
            return Some(message);
        }
    }

    fn close(&mut self) -> impl std::future::Future<Output = Result<(), Self::Error>> + Send {
        let calls = self.manual_calls.clone();
        async move {
            let result = self.inner.close().await;
            if let Some(calls) = calls {
                calls.closed();
            }
            result
        }
    }
}

/// Observe the SDK's actual dispatch completion, including validation refusal
/// before `call_tool`, without duplicating the SDK's request dispatch rules.
pub struct ObservedServer(pub UnicaServer);

impl ObservedServer {
    pub async fn serve<T: Transport<RoleServer> + 'static>(
        self,
        transport: T,
    ) -> Result<rmcp::service::RunningService<RoleServer, Self>, Box<ServerInitializeError>> {
        let calls = self.0.manual_calls.clone();
        let result = ServiceExt::serve(self, transport).await;
        if result.is_err() {
            // No running SDK loop was created. Stop the input pump and await
            // actual close before proving there is no future dispatch.
            calls.initialization_failed().await;
        }
        result.map_err(Box::new)
    }
}

impl rmcp::Service<RoleServer> for ObservedServer {
    async fn handle_request(
        &self,
        request: ClientRequest,
        context: RequestContext<RoleServer>,
    ) -> Result<ServerResult, ErrorData> {
        let is_call = matches!(&request, ClientRequest::CallToolRequest(_));
        let _dispatch = is_call
            .then(|| self.0.manual_calls.dispatch(context.id.clone()))
            .flatten();
        if is_call {
            self.0.manual_calls.before_handler().await;
        }
        rmcp::Service::<RoleServer>::handle_request(&self.0, request, context).await
    }

    async fn handle_notification(
        &self,
        notification: ClientNotification,
        context: NotificationContext<RoleServer>,
    ) -> Result<(), ErrorData> {
        rmcp::Service::<RoleServer>::handle_notification(&self.0, notification, context).await
    }

    fn get_info(&self) -> ServerInfo {
        ServerHandler::get_info(&self.0)
    }

    fn supported_protocol_versions(&self) -> std::borrow::Cow<'static, [ProtocolVersion]> {
        ServerHandler::supported_protocol_versions(&self.0)
    }
}
