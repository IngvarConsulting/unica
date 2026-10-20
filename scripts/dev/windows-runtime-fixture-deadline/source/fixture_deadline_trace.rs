//! Records only the existing staged-transfer and retirement fixture calls.
//! The owned Results return unchanged; formatting and stderr I/O happen on drop.
use super::*;
use crate::application::invocation_store_v5::V5TaskStoreError;
use crate::application::receipt_ledger::ReceiptVersion;
use std::io::Write as _;

#[derive(Debug)]
enum Failure {
    Receipt(ReceiptLedgerError),
    TaskStore(V5TaskStoreError),
    Projection { error: ReceiptLedgerError, fail_stop: bool },
}

struct Phase {
    name: &'static str,
    entered: Instant,
    returned: Option<Instant>,
    epoch_ms: u64,
    failure: Option<Failure>,
    receipt_version: Option<u64>,
    task_version: Option<u64>,
    terminal_digest: Option<TerminalDigest>,
}

pub(super) struct FixtureDeadlineTrace {
    purpose: &'static str,
    task_id: TaskId,
    invocation_id: InvocationId,
    key_digest: ReceiptKeyDigest,
    origin: Instant,
    deadline: Instant,
    seed_epochs: Option<(u64, u64)>,
    phases: Vec<Phase>,
}

impl FixtureDeadlineTrace {
    pub(super) fn new(
        key: &ReceiptKey,
        purpose: &'static str,
        origin: Instant,
        deadline: Instant,
        seed_epochs: Option<(u64, u64)>,
    ) -> Self {
        Self {
            purpose,
            task_id: key.reserved_task_id(),
            invocation_id: key.invocation_id(),
            key_digest: crate::application::receipt_ledger::receipt_key_digest(key),
            origin,
            deadline,
            seed_epochs,
            // At most nine calls occur in either of these finite helper bodies.
            phases: Vec::with_capacity(9),
        }
    }

    fn enter(&mut self, name: &'static str, epoch_ms: u64) -> usize {
        let index = self.phases.len();
        self.phases.push(Phase {
            name, entered: Instant::now(), returned: None, epoch_ms,
            failure: None, receipt_version: None, task_version: None, terminal_digest: None,
        });
        index
    }

    pub(super) fn receipt<T>(
        &mut self, name: &'static str, epoch_ms: u64,
        call: impl FnOnce() -> Result<T, ReceiptLedgerError>,
    ) -> Result<T, ReceiptLedgerError> {
        let index = self.enter(name, epoch_ms);
        let result = call();
        self.phases[index].returned = Some(Instant::now());
        self.phases[index].failure = result.as_ref().err().cloned().map(Failure::Receipt);
        result
    }

    pub(super) fn projection<T>(
        &mut self, name: &'static str, epoch_ms: u64,
        call: impl FnOnce() -> Result<T, V5TaskProjectionFailure>,
    ) -> Result<T, V5TaskProjectionFailure> {
        let index = self.enter(name, epoch_ms);
        let result = call();
        self.phases[index].returned = Some(Instant::now());
        self.phases[index].failure = result.as_ref().err().map(|failure| Failure::Projection {
            error: failure.error.clone(), fail_stop: failure.fail_stop,
        });
        result
    }

    pub(super) fn task_read(
        &mut self, epoch_ms: u64,
        call: impl FnOnce() -> Result<V5StoredInvocationRecord, V5TaskStoreError>,
    ) -> Result<V5StoredInvocationRecord, V5TaskStoreError> {
        let index = self.enter("first_task_get", epoch_ms);
        let result = call();
        let phase = &mut self.phases[index];
        phase.returned = Some(Instant::now());
        match &result {
            Ok(record) => {
                phase.task_version = Some(record.version);
                phase.terminal_digest = record.task.terminal_digest().cloned();
            }
            Err(error) => phase.failure = Some(Failure::TaskStore(error.clone())),
        }
        result
    }

    pub(super) fn receipt_version(&mut self, version: ReceiptVersion) {
        if let Some(phase) = self.phases.last_mut() {
            phase.receipt_version = Some(version.get());
        }
    }
}

impl Drop for FixtureDeadlineTrace {
    fn drop(&mut self) {
        let elapsed = |at: Instant| {
            u64::try_from(at.saturating_duration_since(self.origin).as_nanos()).unwrap_or(u64::MAX)
        };
        let remaining = |at: Instant| {
            u64::try_from(self.deadline.saturating_duration_since(at).as_nanos()).unwrap_or(u64::MAX)
        };
        let phases = self.phases.iter().map(|phase| {
            let failure = phase.failure.as_ref().map(|failure| match failure {
                Failure::Receipt(error) => serde_json::json!({"origin":"receipt","error":format!("{error:?}")}),
                Failure::TaskStore(error) => serde_json::json!({"origin":"task_store","error":format!("{error:?}")}),
                Failure::Projection { error, fail_stop } => serde_json::json!({"origin":"projection","error":format!("{error:?}"),"failStop":fail_stop}),
            });
            serde_json::json!({
                "stage":phase.name,"enteredNs":elapsed(phase.entered),
                "returnedNs":phase.returned.map(elapsed),
                "remainingOnEnterNs":remaining(phase.entered),
                "remainingOnReturnNs":phase.returned.map(remaining),
                "epochMs":phase.epoch_ms,"failure":failure,
                "receiptVersion":phase.receipt_version,"taskVersion":phase.task_version,
                "terminalDigest":phase.terminal_digest,
            })
        }).collect::<Vec<_>>();
        let record = serde_json::json!({
            "purpose":self.purpose,"taskId":self.task_id,"invocationId":self.invocation_id,
            "receiptKeyDigest":self.key_digest,"deadlineNs":elapsed(self.deadline),
            "seedEpochs":self.seed_epochs,"unwinding":std::thread::panicking(),"phases":phases,
        });
        // A closed diagnostic cannot replace an operation's first error with an I/O panic.
        let _ = writeln!(std::io::stderr().lock(), "FIXTURE_DEADLINE {record}");
    }
}
