use crate::{NetworkError, NetworkFailure, TrustSource};
use bytes::Buf;
use std::future::Future;
use std::io::{self, Read};
use std::sync::{Mutex, OnceLock};

/// Own the executor for the process. Library calls are synchronous, including
/// calls made from another runtime. No caller enters or owns this runtime.
pub(crate) fn run_io<F: Future + Send + 'static>(future: F) -> Result<F::Output, NetworkError>
where
    F::Output: Send + 'static,
{
    static RUNTIME: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
    static BUILD: Mutex<()> = Mutex::new(());
    let runtime = match RUNTIME.get() {
        Some(runtime) => runtime,
        None => {
            let _guard = BUILD.lock().unwrap_or_else(|error| error.into_inner());
            match RUNTIME.get() {
                Some(runtime) => runtime,
                None => {
                    let runtime = tokio::runtime::Builder::new_multi_thread()
                        .enable_all()
                        .worker_threads(1)
                        .thread_name("unica-http")
                        .build()
                        .map_err(|_| transport("cannot start the HTTP executor"))?;
                    RUNTIME.get_or_init(|| runtime)
                }
            }
        }
    };
    let (send, receive) = std::sync::mpsc::sync_channel(1);
    runtime.spawn(async move {
        let _ = send.send(future.await);
    });
    receive
        .recv()
        .map_err(|_| transport("HTTP executor stopped"))
}

fn transport(message: &str) -> NetworkError {
    NetworkError::new(NetworkFailure::Transport, message)
}

// resource-limit: network.text-response-bytes
// class: result
// unit: bytes
// scope: request
// value: 10485760
// resource: Объём HTTP-ответа при чтении целиком в строку.
// status: unproven
// origin: #1792
// problem: Предел 10 МиБ сохранён из прежнего клиента для ограничения буферизации.
// on-reach: ответ большего размера отклоняется, потоковое чтение остаётся доступным
const MAX_TEXT_RESPONSE_BYTES: u64 = 10 * 1024 * 1024;

/// HTTP headers and a body read lazily, one network chunk at a time.
pub struct NetworkResponse {
    response: reqwest::Response,
    host: String,
    route: String,
    trust: TrustSource,
}

impl std::fmt::Debug for NetworkResponse {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("NetworkResponse")
            .field("status", &self.status())
            .finish_non_exhaustive()
    }
}

impl NetworkResponse {
    pub(crate) fn new(
        response: reqwest::Response,
        host: String,
        route: String,
        trust: TrustSource,
    ) -> Self {
        Self {
            response,
            host,
            route,
            trust,
        }
    }

    pub fn status(&self) -> u16 {
        self.response.status().as_u16()
    }

    pub fn header(&self, name: &str) -> Option<&str> {
        self.response.headers().get(name)?.to_str().ok()
    }

    pub fn into_reader(self) -> impl Read {
        BodyReader {
            response: Some(self),
            remaining: bytes::Bytes::new(),
        }
    }

    pub fn into_string(self) -> io::Result<String> {
        let mut text = Vec::new();
        self.into_reader()
            .take(MAX_TEXT_RESPONSE_BYTES + 1)
            .read_to_end(&mut text)?;
        if text.len() as u64 > MAX_TEXT_RESPONSE_BYTES {
            return Err(io::Error::other("response too big for into_string"));
        }
        Ok(String::from_utf8_lossy(&text).into_owned())
    }
}

struct BodyReader {
    response: Option<NetworkResponse>,
    remaining: bytes::Bytes,
}

impl Read for BodyReader {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        if buffer.is_empty() {
            return Ok(0);
        }
        while self.remaining.is_empty() {
            let Some(mut response) = self.response.take() else {
                return Ok(0);
            };
            let (response, chunk) = run_io(async move {
                let chunk = response.response.chunk().await;
                (response, chunk)
            })
            .map_err(io::Error::other)?;
            match chunk {
                Ok(Some(chunk)) => {
                    self.remaining = chunk;
                    self.response = Some(response);
                }
                Ok(None) => return Ok(0),
                Err(error) => {
                    let kind = if error.is_timeout() {
                        io::ErrorKind::TimedOut
                    } else {
                        io::ErrorKind::Other
                    };
                    return Err(io::Error::new(
                        kind,
                        NetworkError::from_transport(
                            &error,
                            &response.host,
                            &response.route,
                            response.trust,
                        ),
                    ));
                }
            }
        }
        let read = buffer.len().min(self.remaining.len());
        self.remaining.copy_to_slice(&mut buffer[..read]);
        Ok(read)
    }
}
