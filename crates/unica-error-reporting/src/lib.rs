//! Best-effort internal-error reporting with a strict data boundary.
//!
//! No project input, exception text, environment, hostname, logs, or disk spool
//! is collected. A full queue or unavailable collector never fails an operation.

use sentry::protocol::{EnvelopeItem, Event, Exception, Frame, Level, Stacktrace};
use sentry::{ClientInitGuard, ClientOptions, Envelope, Transport};
use std::borrow::Cow;
use std::sync::mpsc::{self, SyncSender};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

/// Public ingestion key, not an administrative or source-symbol upload token.
pub const BUILTIN_DSN: &str = "https://aaa378edb26a4a8ebd8fdc056987a042@glitch.ingvar.pro/3";
// resource-limit: error-reporting.shutdown-timeout
// class: deadline
// unit: ms
// scope: process
// value: 1000
// resource: Best-effort drain before process exit; a collector outage cannot hold shutdown.
// status: unproven
// origin: #3240
// problem: Default-on error reporting must remain bounded and best effort.
// on-reach: pending events are dropped when the deadline expires
const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(1);
// resource-limit: error-reporting.request-timeout
// class: deadline
// unit: ms
// scope: request
// value: 1000
// resource: Bound one best-effort HTTP submission independently of collector availability.
// status: unproven
// origin: #3240
// problem: Default-on error reporting must remain bounded and best effort.
// on-reach: submission is dropped without affecting the product operation
const REQUEST_TIMEOUT: Duration = Duration::from_secs(1);
// resource-limit: error-reporting.queue-capacity
// class: concurrency
// unit: count
// scope: process
// value: 8
// resource: Bound in-memory submissions; there is no disk spool.
// status: unproven
// origin: #3240
// problem: Default-on error reporting must remain bounded and best effort.
// on-reach: new event is dropped without blocking the caller
const QUEUE_CAPACITY: usize = 8;
// resource-limit: error-reporting.stack-frames
// class: result
// unit: count
// scope: record
// value: 64
// resource: Bound cleaned product stack metadata in each event.
// status: unproven
// origin: #3240
// problem: Default-on error reporting must remain bounded and best effort.
// on-reach: remaining stack frames are omitted
const MAX_FRAMES: usize = 64;

#[derive(Clone, Copy, Debug)]
pub enum Component {
    Core,
    Bootstrap,
}

impl Component {
    fn name(self) -> &'static str {
        match self {
            Self::Core => "core",
            Self::Bootstrap => "bootstrap",
        }
    }
}

/// Only known product failures enter reporting. Never pass a raw error string.
#[derive(Clone, Copy, Debug)]
pub enum InternalError {
    BootstrapInternal,
    McpRuntimeStart,
    TaskProtocolInvariant,
}

impl InternalError {
    fn name(self) -> &'static str {
        match self {
            Self::BootstrapInternal => "bootstrap_internal",
            Self::McpRuntimeStart => "mcp_runtime_start",
            Self::TaskProtocolInvariant => "task_protocol_invariant",
        }
    }
}

/// Retain this outside the platform main thread until process shutdown.
pub struct Guard(Option<ClientInitGuard>);

impl Guard {
    pub fn shutdown(mut self, timeout: Duration) {
        if let Some(guard) = self.0.take() {
            guard.close(Some(timeout));
        }
    }
}

/// Unset enables collection. Explicit opt-out and unrecognised values disable it.
fn enabled(value: Option<&str>) -> bool {
    match value {
        None => true,
        Some(value) => matches!(
            value.trim().to_ascii_lowercase().as_str(),
            "1" | "true" | "on"
        ),
    }
}

pub fn init(component: Component) -> Guard {
    let setting = std::env::var("UNICA_ERROR_REPORTING");
    // A non-Unicode setting is not an invitation to silently enable reporting.
    let enabled = match setting.as_ref() {
        Ok(value) => enabled(Some(value)),
        Err(std::env::VarError::NotPresent) => enabled(None),
        Err(std::env::VarError::NotUnicode(_)) => false,
    };
    init_if_enabled(enabled, component, || {
        let dsn = BUILTIN_DSN.parse::<sentry::types::Dsn>().ok()?;
        let url = dsn.envelope_api_url().to_string();
        let auth = dsn.to_auth(Some("unica-error-reporting")).to_string();
        Some(Arc::new(QueuedTransport::new(component, move |body| {
            if let Ok(client) = unica_network::NetworkClient::shared() {
                let _ = client.post(
                    &url,
                    &[
                        ("Content-Type", "application/x-sentry-envelope"),
                        ("X-Sentry-Auth", &auth),
                    ],
                    &body,
                    Some(REQUEST_TIMEOUT),
                );
            }
        })) as Arc<dyn Transport>)
    })
}

fn init_if_enabled(
    enabled: bool,
    component: Component,
    make_transport: impl FnOnce() -> Option<Arc<dyn Transport>>,
) -> Guard {
    if !enabled {
        return Guard(None);
    }
    let Some(transport) = make_transport() else {
        return Guard(None);
    };
    let Some(dsn) = BUILTIN_DSN.parse().ok() else {
        return Guard(None);
    };
    let mut options = ClientOptions::default();
    options.dsn = Some(dsn);
    options.default_integrations = false;
    options.integrations = vec![Arc::new(
        sentry::integrations::panic::PanicIntegration::new(),
    )];
    options.before_send = Some(Arc::new(move |event| sanitise(event, component)));
    options.before_breadcrumb = Some(Arc::new(|_| None));
    options.transport = Some(Arc::new(ReporterFactory(transport)));
    options.send_default_pii = false;
    options.auto_session_tracking = false;
    options.shutdown_timeout = SHUTDOWN_TIMEOUT;
    Guard(Some(sentry::init(options)))
}

pub fn capture_internal(error: InternalError) {
    sentry::capture_event(Event {
        message: Some(error.name().to_owned()),
        exception: vec![Exception {
            ty: "UnicaInternalError".to_owned(),
            value: Some(error.name().to_owned()),
            ..Exception::default()
        }]
        .into(),
        ..Event::default()
    });
}

/// Required at explicit process::exit sites, which bypass Rust destructors.
pub fn flush_before_exit() {
    if let Some(client) = sentry::Hub::main().client() {
        client.flush(Some(SHUTDOWN_TIMEOUT));
    }
}

fn event_kind(event: &Event<'_>) -> Option<&'static str> {
    if event
        .exception
        .values
        .iter()
        .any(|exception| exception.ty == "panic")
    {
        return Some("rust_panic");
    }
    match event.message.as_deref() {
        Some("rust_panic") => Some("rust_panic"),
        Some("bootstrap_internal") => Some("bootstrap_internal"),
        Some("mcp_runtime_start") => Some("mcp_runtime_start"),
        Some("task_protocol_invariant") => Some("task_protocol_invariant"),
        _ => None,
    }
}

/// Native function names come from our executable, never from MCP input.
/// Reject external/runtime frames and keep neither file paths nor source text.
fn clean_frame(frame: Frame) -> Option<Frame> {
    let function = frame.function?;
    if !(function.starts_with("unica_") || function.starts_with("unica::"))
        || function.len() > 256
        || !function
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"_:<> ,&()[];".contains(&byte))
    {
        return None;
    }
    Some(Frame {
        function: Some(function),
        lineno: frame.lineno,
        colno: frame.colno,
        in_app: Some(true),
        ..Frame::default()
    })
}

fn sanitise(event: Event<'static>, component: Component) -> Option<Event<'static>> {
    let kind = event_kind(&event)?;
    let frames = event
        .exception
        .values
        .into_iter()
        .filter_map(|exception| exception.stacktrace)
        .flat_map(|stack| stack.frames)
        .filter_map(clean_frame)
        .take(MAX_FRAMES)
        .collect::<Vec<_>>();
    let mut tags = sentry::protocol::Map::new();
    tags.insert("component".to_owned(), component.name().to_owned());
    tags.insert("os".to_owned(), std::env::consts::OS.to_owned());
    tags.insert("arch".to_owned(), std::env::consts::ARCH.to_owned());
    Some(Event {
        event_id: event.event_id,
        timestamp: event.timestamp,
        level: if kind == "rust_panic" {
            Level::Fatal
        } else {
            Level::Error
        },
        platform: Cow::Borrowed("native"),
        release: Some(Cow::Borrowed(concat!("unica@", env!("CARGO_PKG_VERSION")))),
        environment: Some(Cow::Borrowed("production")),
        message: Some(kind.to_owned()),
        fingerprint: Cow::Owned(vec![Cow::Borrowed(kind), Cow::Borrowed("{{ default }}")]),
        tags,
        exception: vec![Exception {
            ty: if kind == "rust_panic" {
                "RustPanic"
            } else {
                "UnicaInternalError"
            }
            .to_owned(),
            value: Some("Internal error details withheld".to_owned()),
            stacktrace: (!frames.is_empty()).then_some(Stacktrace {
                frames,
                ..Stacktrace::default()
            }),
            ..Exception::default()
        }]
        .into(),
        ..Event::default()
    })
}

struct ReporterFactory(Arc<dyn Transport>);

impl sentry::TransportFactory for ReporterFactory {
    fn create_transport_with_options(&self, _: sentry::TransportOptions) -> Arc<dyn Transport> {
        Arc::clone(&self.0)
    }
}

#[derive(Default)]
struct State {
    pending: usize,
    closed: bool,
}

struct QueuedTransport {
    sender: Mutex<Option<SyncSender<String>>>,
    state: Arc<(Mutex<State>, Condvar)>,
    component: Component,
}

impl QueuedTransport {
    fn new(component: Component, send: impl Fn(String) + Send + 'static) -> Self {
        let (sender, receiver) = mpsc::sync_channel::<String>(QUEUE_CAPACITY);
        let state = Arc::new((Mutex::new(State::default()), Condvar::new()));
        let worker_state = Arc::clone(&state);
        let worker = std::thread::Builder::new()
            .name("unica-errors".to_owned())
            .spawn(move || {
                while let Ok(body) = receiver.recv() {
                    send(body);
                    let (lock, changed) = &*worker_state;
                    let mut state = lock.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
                    state.pending = state.pending.saturating_sub(1);
                    changed.notify_all();
                }
            });
        Self {
            sender: Mutex::new(worker.ok().map(|_| sender)),
            state,
            component,
        }
    }
}

impl Transport for QueuedTransport {
    fn send_envelope(&self, envelope: Envelope) {
        // Rebuild the envelope too: raw SDK envelopes can bypass before_send.
        // Attachments, sessions, traces, SDK metadata and other items are dropped.
        for item in envelope.into_items() {
            let EnvelopeItem::Event(event) = item else {
                continue;
            };
            let Some(event) = sanitise(*event, self.component) else {
                continue;
            };
            let mut safe = Envelope::new();
            safe.add_item(event);
            let mut bytes = Vec::new();
            if safe.to_writer(&mut bytes).is_err() {
                continue;
            }
            let Ok(body) = String::from_utf8(bytes) else {
                continue;
            };
            let (lock, _) = &*self.state;
            let mut state = lock.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
            if state.closed {
                continue;
            }
            let sender = self
                .sender
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if let Some(sender) = sender.as_ref() {
                if sender.try_send(body).is_ok() {
                    state.pending += 1;
                }
            }
        }
    }

    fn flush(&self, timeout: Duration) -> bool {
        let deadline = Instant::now()
            .checked_add(timeout)
            .unwrap_or_else(Instant::now);
        let (lock, changed) = &*self.state;
        let mut state = lock.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        while state.pending != 0 {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return false;
            }
            let result = changed
                .wait_timeout(state, remaining)
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            state = result.0;
        }
        true
    }

    fn shutdown(&self, timeout: Duration) -> bool {
        {
            let (lock, _) = &*self.state;
            lock.lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .closed = true;
        }
        self.sender
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take();
        self.flush(timeout)
    }
}

#[cfg(test)]
mod tests;
