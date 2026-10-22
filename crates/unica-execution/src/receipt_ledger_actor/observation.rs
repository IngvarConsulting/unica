//! Optional, bounded causal observation. Production never installs a buffer.
//! Events do not format strings, perform I/O, or change a deadline/result.
//! A ticket carries the caller's observer to its actual worker invocation;
//! scoped TLS restoration prevents another actor/test inheriting it.
use std::cell::RefCell;
use std::io;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const CAPACITY: usize = 256;
#[derive(Clone, Copy, Debug)]
struct Cause {
    class: &'static str,
    io_kind: Option<io::ErrorKind>,
    os_error: Option<i32>,
}
#[derive(Clone, Copy, Debug)]
struct Event {
    operation: &'static str,
    point: &'static str,
    at: Instant,
    remaining: Option<Duration>,
    cause: Option<Cause>,
}
struct Buffer {
    origin: Instant,
    events: Vec<Event>,
    first_error: Option<Event>,
    overflow: u64,
}
#[derive(Clone, Default)]
pub struct Context(Option<Arc<Mutex<Buffer>>>);
thread_local! {
    static CURRENT: RefCell<Context> = const { RefCell::new(Context(None)) };
}
pub struct Scope(Context);
impl Drop for Scope {
    fn drop(&mut self) {
        CURRENT.with(|slot| *slot.borrow_mut() = self.0.clone());
    }
}
pub fn capture() -> Context {
    CURRENT.with(|slot| slot.borrow().clone())
}
impl Context {
    pub fn install(&self) -> Scope {
        Scope(CURRENT.with(|slot| slot.replace(self.clone())))
    }
    fn record(
        &self,
        operation: &'static str,
        point: &'static str,
        remaining: Option<Duration>,
        cause: Option<Cause>,
    ) {
        let Some(buffer) = &self.0 else { return };
        let event = Event {
            operation,
            point,
            at: Instant::now(),
            remaining,
            cause,
        };
        // Observation must not manufacture a store failure if an observer
        // previously panicked. Poison is diagnostic data, not authority.
        let mut buffer = buffer
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if event.cause.is_some() && buffer.first_error.is_none() {
            buffer.first_error = Some(event);
        }
        if buffer.events.len() < CAPACITY {
            buffer.events.push(event);
        } else {
            buffer.overflow = buffer.overflow.saturating_add(1);
        }
    }
}
pub fn mark(operation: &'static str, point: &'static str, remaining: Option<Duration>) {
    CURRENT.with(|slot| slot.borrow().record(operation, point, remaining, None));
}
pub fn failure(operation: &'static str, class: &'static str, remaining: Option<Duration>) {
    CURRENT.with(|slot| {
        slot.borrow().record(
            operation,
            "error",
            remaining,
            Some(Cause {
                class,
                io_kind: None,
                os_error: None,
            }),
        )
    });
}
pub fn io_failure(operation: &'static str, error: &io::Error) {
    CURRENT.with(|slot| {
        slot.borrow().record(
            operation,
            "error",
            None,
            Some(Cause {
                class: "Io",
                io_kind: Some(error.kind()),
                os_error: error.raw_os_error(),
            }),
        )
    });
}

/// Only the contract harness can install an observer. The ordinary build
/// executes the same unobserved I/O/mapping path with an empty Context.
#[cfg(any(test, feature = "receipt-ledger-test-support"))]
impl Context {
    pub fn from_environment_for_test() -> Self {
        match std::env::var("UNICA_RECEIPT_CAUSAL_TRACE") {
            Ok(value) if value == "1" => Self::new_for_test(),
            Err(std::env::VarError::NotPresent) => Self::default(),
            _ => panic!("UNICA_RECEIPT_CAUSAL_TRACE must be 1 or absent"),
        }
    }
    pub fn emit_for_test(&self, label: &str, succeeded: bool) {
        if self.0.is_some() {
            eprintln!(
                "RECEIPT_CAUSAL {}",
                serde_json::json!({"label":label,"succeeded":succeeded,"trace":self.snapshot_for_test()})
            );
        }
    }
    pub fn new_for_test() -> Self {
        Self(Some(Arc::new(Mutex::new(Buffer {
            origin: Instant::now(),
            events: Vec::with_capacity(CAPACITY),
            first_error: None,
            overflow: 0,
        }))))
    }
    pub fn snapshot_for_test(&self) -> serde_json::Value {
        let buffer = self
            .0
            .as_ref()
            .expect("installed observer")
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        let encode = |event: &Event| {
            serde_json::json!({
                "operation": event.operation, "point": event.point,
                "elapsedNs": event.at.duration_since(buffer.origin).as_nanos(),
                "remainingNs": event.remaining.map(|value| value.as_nanos()),
                "cause": event.cause.map(|cause| serde_json::json!({"class":cause.class,"ioKind":cause.io_kind.map(|kind|format!("{kind:?}")),"osError":cause.os_error})),
            })
        };
        serde_json::json!({"capacity":CAPACITY,"overflow":buffer.overflow,"complete":buffer.overflow==0,"firstError":buffer.first_error.as_ref().map(encode),"events":buffer.events.iter().map(encode).collect::<Vec<_>>()})
    }
}
