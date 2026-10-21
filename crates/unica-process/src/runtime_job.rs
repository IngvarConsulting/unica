//! Owned runtime child trees, bounded tails and cleanup evidence.

use crate::{RuntimeProcessTreeHandle, RuntimeProcessTreeState, STDOUT_CAPTURE_LIMIT};
use std::{
    io::{self, Read},
    path::Path,
    process::{Child, Command, Stdio},
    sync::{mpsc, Arc, Mutex},
    thread,
    time::{Duration, Instant},
};
use unica_redaction as redaction;
use unica_redaction::bounded_tail;

type JobResult<T> = Result<T, String>;
const OUTPUT_TAIL_BYTES: usize = 16 * 1024;
const FALLBACK_RECEIPT_BYTES: usize = STDOUT_CAPTURE_LIMIT;
const STREAM_FINISH_TIMEOUT: Duration = Duration::from_secs(10);
const RUNTIME_STARTUP_CLEANUP_TIMEOUT: Duration = Duration::from_millis(500);
#[cfg(any(test, feature = "test-support"))]
thread_local! { static STREAM_FINISH_ATTEMPTS: std::cell::Cell<u32> = const { std::cell::Cell::new(0) }; }

fn redacted_error(message: &str) -> String {
    redaction::redactor(message)
}
fn io_error(context: &str, error: &io::Error) -> String {
    redacted_error(&format!("{context}: {error}"))
}

/// A process exit is intentionally nonblocking.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuntimeJobProcessState {
    Running,
    Exited {
        exit_code: i32,
    },
    // SystemRuntimeJobRunner delegates execution timeouts to v8-runner, while
    // the runner boundary still models timeout-capable implementations.
    #[allow(dead_code)]
    TimedOut {
        reason: String,
    },
}

#[derive(Debug, Clone, Default)]
pub struct RuntimeJobOutput {
    pub stdout: String,
    pub stderr: String,
    /// A reader that had to be abandoned before EOF, so the persisted tail may
    /// be missing the end of the process output.
    pub output_incomplete: bool,
    pub fallback_receipt: Option<String>,
    pub fallback_receipt_truncated: bool,
}

/// Process boundary for the core. Implementations must not expose shell snippets.
pub trait RuntimeJobProcess: Send {
    fn id(&self) -> u32;
    fn try_wait(&mut self) -> JobResult<RuntimeJobProcessState>;
    fn cancel(&mut self) -> JobResult<()>;
    /// Return at most `max_bytes` of each stream. The core redacts the retained tails again.
    fn output_tails(&mut self, max_bytes: usize) -> JobResult<RuntimeJobOutput>;
    fn output_tails_until(
        &mut self,
        max_bytes: usize,
        _deadline: Instant,
    ) -> JobResult<RuntimeJobOutput> {
        self.output_tails(max_bytes)
    }
    /// Binds all cleanup work, including Drop, to the caller's one absolute
    /// monotonic deadline. Generic fakes need no extra state.
    fn bind_cleanup_deadline(&mut self, _deadline: Instant) {}
    #[cfg(any(test, feature = "test-support"))]
    fn leader_exited_for_test(&self) -> bool {
        false
    }
    /// Test-owned fakes may provide a controlled terminal+complete-output
    /// transition before the real quarantine supervisor accepts them. Real and
    /// deliberately non-cooperative authorities keep the fail-closed default.
    #[cfg(any(test, feature = "test-support"))]
    fn prepare_controlled_test_supervision(&mut self) -> bool {
        false
    }
}

pub enum RuntimeJobSpawnFailure {
    /// No child ever existed, or the spawned tree was proven terminal before return.
    ProvenChildless(String),
    /// A child may still exist. Its retained capability must stay owned and the
    /// resource lease must remain quarantined.
    OwnershipRetained {
        error: String,
        process: Box<dyn RuntimeJobProcess>,
    },
}

impl std::fmt::Debug for RuntimeJobSpawnFailure {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ProvenChildless(error) => formatter
                .debug_tuple("ProvenChildless")
                .field(error)
                .finish(),
            Self::OwnershipRetained { error, .. } => formatter
                .debug_struct("OwnershipRetained")
                .field("error", error)
                .finish_non_exhaustive(),
        }
    }
}

pub type RuntimeJobSpawnResult = Result<Box<dyn RuntimeJobProcess>, RuntimeJobSpawnFailure>;

pub fn spawn_runtime_job(
    program: &Path,
    cwd: &Path,
    argv: &[String],
    retain_receipt: bool,
) -> RuntimeJobSpawnResult {
    let mut command = Command::new(program);
    command
        .args(argv)
        .current_dir(cwd)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let process_tree = RuntimeProcessTreeHandle::prepare(&mut command).map_err(|error| {
        RuntimeJobSpawnFailure::ProvenChildless(redacted_error(&format!(
            "prepare runtime job process: {error}"
        )))
    })?;
    let mut child = command.spawn().map_err(|error| {
        RuntimeJobSpawnFailure::ProvenChildless(redacted_error(&format!(
            "spawn runtime job process: {error}"
        )))
    })?;
    let stdout = child.stdout.take().map(|stdout| {
        if retain_receipt {
            StreamTail::spawn_with_receipt(stdout)
        } else {
            StreamTail::spawn(stdout)
        }
    });
    let stderr = child.stderr.take().map(StreamTail::spawn);
    let mut process = SystemRuntimeJobProcess {
        id: child.id(),
        child,
        process_tree,
        stdout,
        stderr,
        exited: false,
        cleanup_deadline: None,
        cleanup_complete: false,
    };
    if let Err(error) = process.process_tree.attach(&mut process.child) {
        return Err(process.classify_failed_start(redacted_error(&format!(
            "attach runtime job process tree: {error}"
        ))));
    }
    if process.stdout.is_none() || process.stderr.is_none() {
        return Err(process.classify_failed_start(redacted_error(
            "runtime job process output pipes are incomplete",
        )));
    }
    Ok(Box::new(process))
}
struct SystemRuntimeJobProcess {
    id: u32,
    child: Child,
    process_tree: RuntimeProcessTreeHandle,
    stdout: Option<StreamTail>,
    stderr: Option<StreamTail>,
    exited: bool,
    cleanup_deadline: Option<Instant>,
    cleanup_complete: bool,
}

impl SystemRuntimeJobProcess {
    fn classify_failed_start(mut self, error: String) -> RuntimeJobSpawnFailure {
        let cleanup_deadline = self.begin_cleanup_deadline();
        match self.cleanup_until(cleanup_deadline) {
            Ok(()) => RuntimeJobSpawnFailure::ProvenChildless(error),
            Err(cleanup) => RuntimeJobSpawnFailure::OwnershipRetained {
                error: redacted_error(&format!("{error}; startup cleanup uncertain: {cleanup}")),
                process: Box::new(self),
            },
        }
    }

    fn begin_cleanup_deadline(&mut self) -> Instant {
        *self.cleanup_deadline.get_or_insert_with(|| {
            let started = Instant::now();
            started
                .checked_add(RUNTIME_STARTUP_CLEANUP_TIMEOUT)
                .unwrap_or(started)
        })
    }

    fn cleanup_until(&mut self, cleanup_deadline: Instant) -> JobResult<()> {
        let tree_result = if self.exited {
            Ok(())
        } else {
            self.process_tree
                .terminate_and_reap_until(&mut self.child, cleanup_deadline)
                .map_err(|error| io_error("cleanup runtime process tree", &error))
        };
        if tree_result.is_ok() {
            self.exited = true;
        }
        // Reader accounting is finally-style: even a tree/probe error must not
        // skip either reader and let Drop open another cleanup window.
        let output_result = self.output_tails_until(OUTPUT_TAIL_BYTES, cleanup_deadline);
        if let Err(tree_error) = tree_result {
            let output = output_result
                .err()
                .map(|error| format!("; output cleanup: {error}"))
                .unwrap_or_default();
            return Err(redacted_error(&format!("{tree_error}{output}")));
        }
        let output = output_result?;
        if output.output_incomplete {
            return Err(redacted_error(
                "runtime output cleanup deadline elapsed before both readers reached EOF",
            ));
        }
        self.cleanup_complete = true;
        Ok(())
    }
}

impl RuntimeJobProcess for SystemRuntimeJobProcess {
    fn id(&self) -> u32 {
        self.id
    }

    fn try_wait(&mut self) -> JobResult<RuntimeJobProcessState> {
        match self
            .process_tree
            .poll(&mut self.child)
            .map_err(|error| redacted_error(&format!("poll runtime job process: {error}")))?
        {
            RuntimeProcessTreeState::Exited(status) => {
                self.exited = true;
                Ok(RuntimeJobProcessState::Exited {
                    exit_code: status.code().unwrap_or(1),
                })
            }
            RuntimeProcessTreeState::Running => Ok(RuntimeJobProcessState::Running),
        }
    }

    fn cancel(&mut self) -> JobResult<()> {
        self.process_tree
            .terminate(&mut self.child)
            .map_err(|error| redacted_error(&format!("cancel runtime job process tree: {error}")))
    }

    fn output_tails(&mut self, max_bytes: usize) -> JobResult<RuntimeJobOutput> {
        let deadline = Instant::now()
            .checked_add(STREAM_FINISH_TIMEOUT)
            .unwrap_or_else(Instant::now);
        self.output_tails_until(max_bytes, deadline)
    }

    fn output_tails_until(
        &mut self,
        max_bytes: usize,
        deadline: Instant,
    ) -> JobResult<RuntimeJobOutput> {
        // The receipt only decides anything once the process is gone, and it is
        // the whole retained buffer. Reading it on every 25 ms poll of a running
        // build would clone and re-validate it for nothing.
        let (output_incomplete, fallback_receipt, fallback_receipt_truncated) =
            if self.exited || self.cleanup_deadline.is_some() {
                let stdout_incomplete = self
                    .stdout
                    .as_mut()
                    .map(|stream| stream.finish_until(deadline))
                    .transpose()?
                    .unwrap_or(true);
                let stderr_incomplete = self
                    .stderr
                    .as_mut()
                    .map(|stream| stream.finish_until(deadline))
                    .transpose()?
                    .unwrap_or(true);
                let (receipt, receipt_truncated) = self
                    .stdout
                    .as_ref()
                    .map(StreamTail::receipt)
                    .transpose()?
                    .unwrap_or((None, true));
                (
                    stdout_incomplete || stderr_incomplete,
                    receipt,
                    receipt_truncated,
                )
            } else {
                (false, None, false)
            };
        let output = RuntimeJobOutput {
            stdout: self
                .stdout
                .as_ref()
                .map(|stream| stream.tail(max_bytes))
                .transpose()?
                .unwrap_or_default(),
            stderr: self
                .stderr
                .as_ref()
                .map(|stream| stream.tail(max_bytes))
                .transpose()?
                .unwrap_or_default(),
            output_incomplete,
            fallback_receipt,
            fallback_receipt_truncated,
        };
        if self.exited && !output.output_incomplete {
            self.cleanup_complete = true;
        }
        Ok(output)
    }

    fn bind_cleanup_deadline(&mut self, deadline: Instant) {
        self.cleanup_deadline = Some(
            self.cleanup_deadline
                .map(|current| current.min(deadline))
                .unwrap_or(deadline),
        );
    }

    #[cfg(any(test, feature = "test-support"))]
    fn leader_exited_for_test(&self) -> bool {
        self.process_tree.leader_exited()
    }
}

impl Drop for SystemRuntimeJobProcess {
    fn drop(&mut self) {
        if self.cleanup_complete {
            return;
        }
        if self.cleanup_deadline.is_none() {
            let cleanup_deadline = self.begin_cleanup_deadline();
            let _ = self.cleanup_until(cleanup_deadline);
        }
    }
}

pub struct StreamTail {
    text: Arc<Mutex<String>>,
    receipt: Option<Arc<Mutex<Vec<u8>>>>,
    receipt_truncated: Arc<std::sync::atomic::AtomicBool>,
    state: StreamTailState,
    finished: mpsc::Receiver<()>,
}

enum StreamTailState {
    Reading(Option<thread::JoinHandle<io::Result<()>>>),
    Eof,
    Failed(String),
}

impl StreamTail {
    pub fn spawn<R>(stream: R) -> Self
    where
        R: Read + Send + 'static,
    {
        Self::spawn_inner(stream, false)
    }

    pub fn spawn_with_receipt<R>(stream: R) -> Self
    where
        R: Read + Send + 'static,
    {
        Self::spawn_inner(stream, true)
    }

    fn spawn_inner<R>(mut stream: R, retain_receipt: bool) -> Self
    where
        R: Read + Send + 'static,
    {
        let text = Arc::new(Mutex::new(String::new()));
        let captured = Arc::clone(&text);
        let receipt = retain_receipt.then(|| Arc::new(Mutex::new(Vec::new())));
        let captured_receipt = receipt.clone();
        let receipt_truncated = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let capture_receipt_truncated = Arc::clone(&receipt_truncated);
        let (finished_sender, finished) = mpsc::channel::<()>();
        let reader = thread::spawn(move || {
            // Dropped on every exit path, which is what tells `finish` the
            // reader is joinable without blocking on it.
            let _finished = finished_sender;
            let mut buffer = [0_u8; 4096];
            let mut redactor = redaction::StreamRedactor::new();
            loop {
                let count = stream.read(&mut buffer)?;
                if count == 0 {
                    append_tail(&captured, &redactor.finish())?;
                    return Ok(());
                }
                if let Some(receipt) = &captured_receipt {
                    append_byte_tail(receipt, &capture_receipt_truncated, &buffer[..count])?;
                }
                let chunk = String::from_utf8_lossy(&buffer[..count]);
                append_tail(&captured, &redactor.push(&chunk))?;
            }
        });
        Self {
            text,
            receipt,
            receipt_truncated,
            state: StreamTailState::Reading(Some(reader)),
            finished,
        }
    }

    /// Drains the reader for a bounded time and reports whether it had to be
    /// abandoned. A child that exits while a grandchild still holds the
    /// inherited pipe never produces EOF, and this join runs inside the
    /// workspace lifecycle guard: waiting on it forever would wedge the guard
    /// and with it every other job transition, including cancellation.
    #[cfg(any(test, feature = "test-support"))]
    pub fn finish(&mut self) -> JobResult<bool> {
        self.finish_within(STREAM_FINISH_TIMEOUT)
    }

    pub fn finish_until(&mut self, deadline: Instant) -> JobResult<bool> {
        #[cfg(any(test, feature = "test-support"))]
        STREAM_FINISH_ATTEMPTS.with(|slot| slot.set(slot.get().saturating_add(1)));
        self.finish_within(deadline.saturating_duration_since(Instant::now()))
    }

    pub fn finish_within(&mut self, timeout: Duration) -> JobResult<bool> {
        match &self.state {
            StreamTailState::Eof => return Ok(false),
            StreamTailState::Failed(error) => return Err(error.clone()),
            StreamTailState::Reading(_) => {}
        }
        match self.finished.recv_timeout(timeout) {
            Ok(()) | Err(mpsc::RecvTimeoutError::Disconnected) => {
                let reader = match &mut self.state {
                    StreamTailState::Reading(reader) => reader
                        .take()
                        .expect("reading stream retains exactly one reader handle"),
                    StreamTailState::Eof | StreamTailState::Failed(_) => unreachable!(),
                };
                let result = reader
                    .join()
                    .map_err(|_| redacted_error("join runtime job output reader"))
                    .and_then(|result| {
                        result.map_err(|error| io_error("read runtime job output", &error))
                    });
                match result {
                    Ok(()) => {
                        self.state = StreamTailState::Eof;
                        Ok(false)
                    }
                    Err(error) => {
                        self.state = StreamTailState::Failed(error.clone());
                        Err(error)
                    }
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                // Preserve the reader capability so quarantine supervision can
                // later prove EOF. The partial bytes remain readable, but the
                // receipt cannot authorize a fallback until then.
                self.receipt_truncated
                    .store(true, std::sync::atomic::Ordering::Release);
                Ok(true)
            }
        }
    }

    pub fn tail(&self, max_bytes: usize) -> JobResult<String> {
        let text = self
            .text
            .lock()
            .map_err(|error| redacted_error(&format!("lock runtime job output: {error}")))?;
        Ok(bounded_tail(&text, max_bytes))
    }

    pub fn receipt(&self) -> JobResult<(Option<String>, bool)> {
        let Some(receipt) = &self.receipt else {
            return Ok((None, false));
        };
        let bytes = receipt
            .lock()
            .map_err(|error| redacted_error(&format!("lock runtime job receipt: {error}")))?;
        let truncated = self
            .receipt_truncated
            .load(std::sync::atomic::Ordering::Acquire)
            || bytes.len() > FALLBACK_RECEIPT_BYTES;
        if truncated {
            return Ok((None, true));
        }
        Ok((String::from_utf8(bytes.clone()).ok(), false))
    }
}

pub fn cancel_and_reap_with_budget(
    process: &mut dyn RuntimeJobProcess,
    budget: Duration,
) -> JobResult<()> {
    let started = Instant::now();
    let deadline = started.checked_add(budget).unwrap_or(started);
    process.bind_cleanup_deadline(deadline);
    let process_result = (|| {
        process.cancel()?;
        loop {
            match process.try_wait()? {
                RuntimeJobProcessState::Exited { .. } | RuntimeJobProcessState::TimedOut { .. } => {
                    return Ok(());
                }
                RuntimeJobProcessState::Running if Instant::now() >= deadline => {
                    return Err(redacted_error(
                        "runtime job process did not exit after cancellation request",
                    ));
                }
                RuntimeJobProcessState::Running => thread::sleep(
                    Duration::from_millis(10)
                        .min(deadline.saturating_duration_since(Instant::now())),
                ),
            }
        }
    })();
    // Reader accounting is mandatory on both success and failure and consumes
    // the same absolute deadline. System processes remember that deadline, so
    // their Drop cannot open a second cleanup window.
    let output_result = process.output_tails_until(OUTPUT_TAIL_BYTES, deadline);
    match process_result {
        Err(error) => Err(error),
        Ok(()) => {
            let output = output_result?;
            if output.output_incomplete {
                Err(redacted_error(
                    "runtime output cleanup deadline elapsed before both readers reached EOF",
                ))
            } else {
                Ok(())
            }
        }
    }
}

pub fn cancel_and_reap(process: &mut dyn RuntimeJobProcess) -> JobResult<()> {
    cancel_and_reap_with_budget(process, Duration::from_secs(5))
}

fn append_tail(target: &Arc<Mutex<String>>, addition: &str) -> io::Result<()> {
    let mut text = target
        .lock()
        .map_err(|_| io::Error::other("runtime job output lock is poisoned"))?;
    text.push_str(addition);
    if text.len() > OUTPUT_TAIL_BYTES {
        *text = bounded_tail(&text, OUTPUT_TAIL_BYTES);
    }
    Ok(())
}

fn append_byte_tail(
    target: &Arc<Mutex<Vec<u8>>>,
    truncated: &std::sync::atomic::AtomicBool,
    addition: &[u8],
) -> io::Result<()> {
    let mut bytes = target
        .lock()
        .map_err(|_| io::Error::other("runtime job receipt lock is poisoned"))?;
    bytes.extend_from_slice(addition);
    if bytes.len() > FALLBACK_RECEIPT_BYTES {
        let keep_from = bytes.len().saturating_sub(FALLBACK_RECEIPT_BYTES);
        bytes.drain(..keep_from);
        truncated.store(true, std::sync::atomic::Ordering::Release);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::sync::atomic::{AtomicBool, Ordering};
    struct NeverEndingStream {
        released: Arc<std::sync::atomic::AtomicBool>,
    }
    impl Read for NeverEndingStream {
        fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
            while !self.released.load(std::sync::atomic::Ordering::Acquire) {
                thread::sleep(Duration::from_millis(5));
            }
            buffer[0] = b'x';
            Ok(0)
        }
    }
    #[test]
    fn system_process_drop_uses_no_second_window_after_cleanup_deadline_is_consumed() {
        if !crate::test_support::unix_runtime_authority_tests_supported() {
            return;
        }
        let released = Arc::new(AtomicBool::new(false));
        let (mut process, id) = system_process_for_authority_failure("exec sleep 30");
        process.stdout = Some(StreamTail::spawn(NeverEndingStream {
            released: Arc::clone(&released),
        }));
        process.stderr = Some(StreamTail::spawn(NeverEndingStream {
            released: Arc::clone(&released),
        }));
        crate::test_support::reset_runtime_tree_cleanup_calls_for_test();
        crate::test_support::inject_runtime_tree_cleanup_timeout_for_test();
        STREAM_FINISH_ATTEMPTS.with(|slot| slot.set(0));

        let retained = process.classify_failed_start(redacted_error("injected startup failure"));
        assert!(matches!(
            &retained,
            RuntimeJobSpawnFailure::OwnershipRetained { .. }
        ));
        assert_eq!(
            crate::test_support::runtime_tree_cleanup_calls_for_test(),
            1
        );
        assert_eq!(
            STREAM_FINISH_ATTEMPTS.with(std::cell::Cell::get),
            2,
            "startup tree failure skipped one of the two output readers"
        );
        let drop_started = Instant::now();
        drop(retained);
        assert_eq!(
            crate::test_support::runtime_tree_cleanup_calls_for_test(),
            1,
            "Drop opened a second process-tree cleanup attempt"
        );
        assert_eq!(
            STREAM_FINISH_ATTEMPTS.with(std::cell::Cell::get),
            2,
            "Drop opened a second output-reader cleanup attempt"
        );
        assert!(
            drop_started.elapsed() < Duration::from_millis(100),
            "Drop waited after the absolute cleanup deadline was consumed"
        );
        released.store(true, Ordering::Release);
        reap_raw_test_child(id);
    }
    #[test]
    fn external_cancel_error_binds_system_drop_to_the_same_cleanup_deadline() {
        if !crate::test_support::unix_runtime_authority_tests_supported() {
            return;
        }
        let (mut process, id) = system_process_for_authority_failure("exec sleep 30");
        STREAM_FINISH_ATTEMPTS.with(|slot| slot.set(0));
        crate::test_support::reset_runtime_tree_cleanup_calls_for_test();
        crate::test_support::inject_unix_waitid_error_for_test();

        cancel_and_reap_with_budget(&mut process, Duration::from_millis(80))
            .expect_err("injected waitid failure must retain uncertain ownership");
        let reader_attempts = STREAM_FINISH_ATTEMPTS.with(std::cell::Cell::get);
        let tree_attempts = crate::test_support::runtime_tree_cleanup_calls_for_test();
        assert_eq!(reader_attempts, 2);
        drop(process);
        assert_eq!(
            STREAM_FINISH_ATTEMPTS.with(std::cell::Cell::get),
            reader_attempts,
            "Drop repeated reader cleanup after external cancellation failed"
        );
        assert_eq!(
            crate::test_support::runtime_tree_cleanup_calls_for_test(),
            tree_attempts,
            "Drop opened a second tree cleanup after external cancellation failed"
        );
        reap_raw_test_child(id);
    }
    #[test]
    fn waitid_authority_loss_makes_cancel_and_drop_send_zero_group_signals() {
        if !crate::test_support::unix_runtime_authority_tests_supported() {
            return;
        }
        let (mut process, id) = system_process_for_authority_failure("exec sleep 30");
        crate::test_support::reset_unix_signal_count_for_test();
        crate::test_support::inject_unix_waitid_error_for_test();
        process
            .try_wait()
            .expect_err("injected waitid failure loses generation authority");
        process
            .cancel()
            .expect_err("cancel must fail closed after authority loss");
        drop(process);
        assert_eq!(
            crate::test_support::unix_signal_count_for_test(),
            0,
            "cancel or Drop signalled after waitid authority loss"
        );
        reap_raw_test_child(id);
    }
    #[test]
    fn reap_authority_loss_makes_cancel_and_drop_send_zero_group_signals() {
        if !crate::test_support::unix_runtime_authority_tests_supported() {
            return;
        }
        let gate = std::env::temp_dir().join(format!("unica-reap-gate-{}", uuid::Uuid::new_v4()));
        let script = format!("while [ ! -e '{}' ]; do :; done", gate.display());
        let (mut process, id) = system_process_for_authority_failure(&script);
        crate::test_support::reset_unix_signal_count_for_test();
        crate::test_support::inject_unix_reap_error_for_test();
        fs::write(&gate, b"go").unwrap();
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            match process.try_wait() {
                Err(error) => {
                    assert!(error.contains("injected Unix reap failure"), "{error}");
                    break;
                }
                Ok(RuntimeJobProcessState::Running) if Instant::now() < deadline => {
                    thread::yield_now();
                }
                other => panic!("reap failure was not observed: {other:?}"),
            }
        }
        process
            .cancel()
            .expect_err("cancel must fail closed after reap authority loss");
        drop(process);
        assert_eq!(
            crate::test_support::unix_signal_count_for_test(),
            0,
            "cancel or Drop signalled after reap authority loss"
        );
        reap_raw_test_child(id);
        let _ = fs::remove_file(gate);
    }
    fn system_process_for_authority_failure(script: &str) -> (SystemRuntimeJobProcess, u32) {
        let mut command = Command::new("/bin/sh");
        command
            .arg("-c")
            .arg(script)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let process_tree = RuntimeProcessTreeHandle::prepare(&mut command).unwrap();
        let mut child = command.spawn().unwrap();
        let id = child.id();
        let stdout = child.stdout.take().map(StreamTail::spawn);
        let stderr = child.stderr.take().map(StreamTail::spawn);
        let mut process = SystemRuntimeJobProcess {
            id,
            child,
            process_tree,
            stdout,
            stderr,
            exited: false,
            cleanup_deadline: None,
            cleanup_complete: false,
        };
        process.process_tree.attach(&mut process.child).unwrap();
        (process, id)
    }
    fn reap_raw_test_child(process_id: u32) {
        crate::test_support::reap_runtime_authority_test_child(process_id);
    }
}
