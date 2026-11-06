//! Preserve the source of tool arguments in request-local SDK extensions.
//! JSON-RPC framing, compatibility parsing and message DTOs remain SDK-owned.
use rmcp::model::{ClientJsonRpcMessage, ClientRequest, ErrorData, ServerJsonRpcMessage};
use rmcp::transport::async_rw::{AsyncRwTransport, JsonRpcMessageCodec, JsonRpcMessageCodecError};
use rmcp::{transport::Transport, RoleServer};
use serde_json::value::RawValue;
use std::future::Future;
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, BufReader};
use tokio_util::{bytes::BytesMut, codec::Decoder};

#[derive(Clone)]
pub(super) struct RawToolArguments(pub String);

fn capture_arguments(
    message: &mut ClientJsonRpcMessage,
    source: &[u8],
) -> Result<(), serde_json::Error> {
    if let ClientJsonRpcMessage::Request(request) = message {
        if let ClientRequest::CallToolRequest(call) = &mut request.request {
            use std::collections::BTreeMap;
            let source = source.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(source);
            let envelope: BTreeMap<String, Box<RawValue>> = serde_json::from_slice(source)?;
            if let Some(params) = envelope.get("params") {
                let params: BTreeMap<String, Box<RawValue>> = serde_json::from_str(params.get())?;
                if let Some(arguments) = params.get("arguments").filter(|raw| raw.get() != "null") {
                    call.extensions
                        .insert(RawToolArguments(arguments.get().to_owned()));
                }
            }
        }
    }
    Ok(())
}

pub(super) struct CapturingTransport<R: AsyncRead, W: AsyncWrite> {
    reader: BufReader<R>,
    line: Vec<u8>,
    writer: AsyncRwTransport<RoleServer, tokio::io::Empty, W>,
}
impl<R, W> CapturingTransport<R, W>
where
    R: AsyncRead + Send + Unpin,
    W: AsyncWrite + Send + Unpin + 'static,
{
    pub(super) fn new(reader: R, writer: W) -> Self {
        Self {
            reader: BufReader::new(reader),
            line: Vec::new(),
            writer: AsyncRwTransport::new_server(tokio::io::empty(), writer),
        }
    }
}
impl<R, W> Transport<RoleServer> for CapturingTransport<R, W>
where
    R: AsyncRead + Send + Unpin,
    W: AsyncWrite + Send + Unpin + 'static,
{
    type Error = std::io::Error;
    fn send(
        &mut self,
        message: ServerJsonRpcMessage,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send + 'static {
        self.writer.send(message)
    }
    async fn receive(&mut self) -> Option<ClientJsonRpcMessage> {
        loop {
            // Persistent buffer retains partial reads across SDK cancellation.
            match self.reader.read_until(b'\n', &mut self.line).await {
                Ok(0) | Err(_) => return None,
                _ => {}
            }
            let source = std::mem::take(&mut self.line);
            let mut bytes = BytesMut::from(source.as_slice());
            let mut codec = JsonRpcMessageCodec::<ClientJsonRpcMessage>::new();
            let decoded = if bytes.ends_with(b"\n") {
                codec.decode(&mut bytes)
            } else {
                // AsyncRwTransport accepts a final complete JSON frame at EOF.
                codec.decode_eof(&mut bytes)
            };
            match decoded {
                Ok(Some(mut message)) => {
                    if capture_arguments(&mut message, &source).is_err() {
                        return None;
                    }
                    return Some(message);
                }
                Ok(None) => continue,
                Err(JsonRpcMessageCodecError::Serde(error))
                    if matches!(
                        error.classify(),
                        serde_json::error::Category::Data | serde_json::error::Category::Io
                    ) =>
                {
                    self.writer
                        .send(ServerJsonRpcMessage::error(
                            ErrorData::invalid_request("Invalid request", None),
                            None,
                        ))
                        .await
                        .ok()?;
                }
                Err(JsonRpcMessageCodecError::Serde(_)) => continue,
                Err(_) => return None,
            }
        }
    }
    async fn close(&mut self) -> Result<(), Self::Error> {
        self.writer.close().await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};
    use unica_daemon::protocol_v5::V5InvocationRequest;
    use unica_execution::receipt_ledger::V5ToolIdentity;
    use unica_execution_contract::invocation::{InvocationId, TaskId};
    use unica_xdto::operations::{XdtoOperation, XdtoPlanOperation};

    #[test]
    fn captured_arguments_keep_exact_numbers_and_original_json_types_through_daemon() {
        let mut hashes = Vec::new();
        for (raw, expected) in [
            ("9223372036854775807", Some(i64::MAX)),
            ("9223372036854775807.0", Some(i64::MAX)),
            ("9.223372036854775807e18", Some(i64::MAX)),
            ("9223372036854775808", None),
            ("9223372036854775808.0", None),
            ("9.223372036854775808e18", None),
            ("1.000000000000000000001", None),
            (
                r#"{"$serde_json::private::Number":"9223372036854775807.0"}"#,
                None,
            ),
            (r#""9223372036854775807.0""#, None),
            ("true", None),
            ("null", None),
        ] {
            let input = format!(
                r#"{{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{{"name":"unica.apply","arguments":{{"at":"main:XDTOPackage.Sample.Type.Target","ops":[{{"op":"property.add","args":{{"values":{{"property":{{"name":"Probe","type":"xs:string","upperBound":{raw}}}}}}}}}]}}}}}}"#
            );
            let mut message: ClientJsonRpcMessage = serde_json::from_str(&input).unwrap();
            capture_arguments(&mut message, input.as_bytes()).unwrap();
            let ClientJsonRpcMessage::Request(request) = message else {
                panic!("request")
            };
            let ClientRequest::CallToolRequest(call) = request.request else {
                panic!("tool call")
            };
            let original = call.extensions.get::<RawToolArguments>().unwrap().0.clone();
            let arguments = call.params.arguments.unwrap();
            if raw.starts_with('{') {
                assert!(
                    arguments["ops"][0]["args"]["values"]["property"]["upperBound"].is_object()
                );
            }
            let invocation = V5InvocationRequest::new(
                InvocationId::new(),
                TaskId::new(),
                V5ToolIdentity::Apply,
                arguments,
                unica_bootstrap::ResolvedWorkspace {
                    directory: "/workspace".into(),
                    origin: unica_bootstrap::WorkspaceOrigin::LaunchCwd { roots: None },
                },
                7000,
            )
            .unwrap()
            .with_raw_arguments(Some(original))
            .unwrap();
            let frame = unica_daemon::protocol_v5::V5ClientRequest::SubmitInvocation { invocation };
            let encoded = serde_json::to_vec(&frame).unwrap();
            let unica_daemon::protocol_v5::V5ClientRequest::SubmitInvocation { invocation } =
                unica_daemon::protocol_v5::decode_v5_client_request(&encoded).unwrap()
            else {
                panic!("submit")
            };
            let hash = unica_execution::invocation::arguments_hash_with_raw(
                invocation.arguments(),
                invocation.raw_arguments(),
            );
            hashes.push(hash);
            let mut planning = invocation.arguments().clone();
            planning.insert("dryRun".into(), json!(true));
            let parsed = crate::domain::apply::ApplyRequest::parse_with_raw(
                &planning,
                &["main"],
                invocation.raw_arguments(),
            );
            match expected {
                Some(expected) => {
                    let parsed = parsed.unwrap();
                    let op = &parsed.ops()[0];
                    match XdtoOperation::AddProperty
                        .decode(&Value::Object(op.args().clone()), "ops[0].args", "main")
                        .unwrap()
                    {
                        XdtoPlanOperation::AddProperty(args) => {
                            assert_eq!(args.property.max_occurs, Some(expected))
                        }
                        _ => panic!("wrong family"),
                    }
                }
                None => {
                    // Non-numeric operands are rejected by the usual typed decoder.
                    let error = match parsed {
                        Err(error) => error.location().to_owned(),
                        Ok(parsed) => {
                            XdtoOperation::AddProperty
                                .decode_with_number_errors(
                                    &Value::Object(parsed.ops()[0].args().clone()),
                                    "ops[0].args",
                                    "main",
                                    parsed.ops()[0].number_errors(),
                                )
                                .unwrap_err()
                                .path
                        }
                    };
                    assert_eq!(error, "ops[0].args.values.property.upperBound", "{raw}");
                }
            }
        }
        assert_ne!(
            hashes[1], hashes[4],
            "MAX.0 and 2^63.0 must not share a receipt identity"
        );
        assert_ne!(hashes[1], hashes[7], "an object is not a number");
    }

    #[test]
    fn sdk_progress_floats_and_private_number_keys_keep_their_message_types() {
        let message: ClientJsonRpcMessage = serde_json::from_value(json!({"jsonrpc":"2.0","method":"notifications/progress","params":{"progressToken":"p","progress":0.5,"total":1.5}})).unwrap();
        assert!(matches!(
            message,
            ClientJsonRpcMessage::Notification(rmcp::model::JsonRpcNotification {
                notification: rmcp::model::ClientNotification::ProgressNotification(_),
                ..
            })
        ));
        let object: Value =
            serde_json::from_str(r#"{"$serde_json::private::Number":"1.0"}"#).unwrap();
        assert!(object.is_object());
    }
    #[tokio::test]
    async fn capturing_transport_keeps_sdk_framing_and_cancelled_partial_reads() {
        use tokio::io::AsyncWriteExt;
        let valid = r#"{"jsonrpc":"2.0","id":8,"method":"ping"}"#;
        for input in [valid.to_owned(), format!("{valid}\n"), format!("\n{{malformed\n{valid}\r\n"), format!("{{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"tools/call\",\"params\":{{}}}}\n{valid}\n")] {
            let (mut sdk_peer, sdk_io) = tokio::io::duplex(8192);
            let (mut captured_peer, captured_io) = tokio::io::duplex(8192);
            sdk_peer.write_all(input.as_bytes()).await.unwrap();
            sdk_peer.shutdown().await.unwrap();
            captured_peer.write_all(input.as_bytes()).await.unwrap();
            captured_peer.shutdown().await.unwrap();
            let (read, write) = tokio::io::split(sdk_io);
            let mut sdk = AsyncRwTransport::new_server(read, write);
            let (read, write) = tokio::io::split(captured_io);
            let mut captured = CapturingTransport::new(read, write);
            loop {
                let sdk_message = sdk.receive().await;
                let captured_message = captured.receive().await;
                let eof = sdk_message.is_none();
                assert_eq!(serde_json::to_value(captured_message).unwrap(), serde_json::to_value(sdk_message).unwrap(), "{input}");
                if eof { break; }
            }
        }
        let (mut peer, io) = tokio::io::duplex(8192);
        let (read, write) = tokio::io::split(io);
        let mut captured = CapturingTransport::new(read, write);
        peer.write_all(b"{\"jsonrpc\":").await.unwrap();
        let mut receive = Box::pin(captured.receive());
        std::future::poll_fn(|cx| {
            assert!(receive.as_mut().poll(cx).is_pending());
            std::task::Poll::Ready(())
        })
        .await;
        drop(receive);
        peer.write_all(b"\"2.0\",\"id\":8,\"method\":\"ping\"}\n")
            .await
            .unwrap();
        assert!(matches!(
            captured.receive().await,
            Some(ClientJsonRpcMessage::Request(_))
        ));
    }
}
