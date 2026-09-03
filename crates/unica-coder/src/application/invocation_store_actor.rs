use crate::application::invocation_store::{
    InvocationStore, InvocationStoreError, NewInvocationRecord, SafeStatusMessage,
    StoredInvocationRecord, TaskTransition,
};
use crate::domain::cancellation::CancellationToken;
use crate::domain::code_intelligence::ProviderDeadline;
use crate::domain::invocation::TaskId;
use std::sync::mpsc::{self, Receiver, SyncSender, TrySendError};
use std::sync::Arc;
use std::time::{Duration, Instant};

const STORE_COMMAND_CAPACITY: usize = 64;
const STORE_WAIT_SLICE: Duration = Duration::from_millis(5);

type StoreReply = SyncSender<Result<StoredInvocationRecord, InvocationStoreError>>;

enum StoreCommand {
    CreateWorking {
        record: NewInvocationRecord,
        deadline: ProviderDeadline,
        cancellation: CancellationToken,
        reply: StoreReply,
    },
    Get {
        task_id: TaskId,
        deadline: ProviderDeadline,
        cancellation: CancellationToken,
        reply: StoreReply,
    },
    Update {
        task_id: TaskId,
        transition: TaskTransition,
        deadline: ProviderDeadline,
        cancellation: CancellationToken,
        reply: StoreReply,
    },
    Cancel {
        task_id: TaskId,
        status_message: SafeStatusMessage,
        deadline: ProviderDeadline,
        cancellation: CancellationToken,
        reply: StoreReply,
    },
}

#[derive(Clone)]
pub(crate) struct InvocationStoreActor {
    commands: SyncSender<StoreCommand>,
}

impl InvocationStoreActor {
    pub(crate) fn spawn(store: Arc<dyn InvocationStore>) -> Self {
        let (commands, receiver) = mpsc::sync_channel(STORE_COMMAND_CAPACITY);
        std::thread::spawn(move || run_store_worker(store, receiver));
        Self { commands }
    }

    pub(crate) fn create_working(
        &self,
        record: NewInvocationRecord,
        deadline: Instant,
        cancellation: &CancellationToken,
    ) -> Result<StoredInvocationRecord, InvocationStoreError> {
        self.call(
            |reply| StoreCommand::CreateWorking {
                record,
                deadline: ProviderDeadline::new(deadline),
                cancellation: cancellation.clone(),
                reply,
            },
            deadline,
            cancellation,
        )
    }

    pub(crate) fn get(
        &self,
        task_id: TaskId,
        deadline: Instant,
        cancellation: &CancellationToken,
    ) -> Result<StoredInvocationRecord, InvocationStoreError> {
        self.call(
            |reply| StoreCommand::Get {
                task_id,
                deadline: ProviderDeadline::new(deadline),
                cancellation: cancellation.clone(),
                reply,
            },
            deadline,
            cancellation,
        )
    }

    pub(crate) fn update(
        &self,
        task_id: TaskId,
        transition: TaskTransition,
        deadline: Instant,
        cancellation: &CancellationToken,
    ) -> Result<StoredInvocationRecord, InvocationStoreError> {
        self.call(
            |reply| StoreCommand::Update {
                task_id,
                transition,
                deadline: ProviderDeadline::new(deadline),
                cancellation: cancellation.clone(),
                reply,
            },
            deadline,
            cancellation,
        )
    }

    pub(crate) fn cancel(
        &self,
        task_id: TaskId,
        status_message: SafeStatusMessage,
        deadline: Instant,
        cancellation: &CancellationToken,
    ) -> Result<StoredInvocationRecord, InvocationStoreError> {
        self.call(
            |reply| StoreCommand::Cancel {
                task_id,
                status_message,
                deadline: ProviderDeadline::new(deadline),
                cancellation: cancellation.clone(),
                reply,
            },
            deadline,
            cancellation,
        )
    }

    fn call(
        &self,
        command: impl FnOnce(StoreReply) -> StoreCommand,
        deadline: Instant,
        cancellation: &CancellationToken,
    ) -> Result<StoredInvocationRecord, InvocationStoreError> {
        let (reply, response) = mpsc::sync_channel(1);
        let mut pending = command(reply);
        loop {
            store_checkpoint(deadline, cancellation)?;
            match self.commands.try_send(pending) {
                Ok(()) => break,
                Err(TrySendError::Full(command)) => pending = command,
                Err(TrySendError::Disconnected(_)) => {
                    return Err(InvocationStoreError::ActorUnavailable)
                }
            }
            std::thread::sleep(
                deadline
                    .saturating_duration_since(Instant::now())
                    .min(STORE_WAIT_SLICE),
            );
        }

        loop {
            store_checkpoint(deadline, cancellation)?;
            let wait = deadline
                .saturating_duration_since(Instant::now())
                .min(STORE_WAIT_SLICE);
            match response.recv_timeout(wait) {
                Ok(result) => return result,
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    return Err(InvocationStoreError::ActorUnavailable)
                }
            }
        }
    }
}

fn store_checkpoint(
    deadline: Instant,
    cancellation: &CancellationToken,
) -> Result<(), InvocationStoreError> {
    if cancellation.is_cancelled() {
        return Err(InvocationStoreError::Cancelled);
    }
    if Instant::now() >= deadline {
        return Err(InvocationStoreError::DeadlineExceeded);
    }
    Ok(())
}

fn run_store_worker(store: Arc<dyn InvocationStore>, commands: Receiver<StoreCommand>) {
    while let Ok(command) = commands.recv() {
        match command {
            StoreCommand::CreateWorking {
                record,
                deadline,
                cancellation,
                reply,
            } => {
                let _ = reply.send(store.create_working_before(record, deadline, &cancellation));
            }
            StoreCommand::Get {
                task_id,
                deadline,
                cancellation,
                reply,
            } => {
                let _ = reply.send(store.get_before(task_id, deadline, &cancellation));
            }
            StoreCommand::Update {
                task_id,
                transition,
                deadline,
                cancellation,
                reply,
            } => {
                let _ =
                    reply.send(store.update_before(task_id, transition, deadline, &cancellation));
            }
            StoreCommand::Cancel {
                task_id,
                status_message,
                deadline,
                cancellation,
                reply,
            } => {
                let _ = reply.send(store.cancel_before(
                    task_id,
                    status_message,
                    deadline,
                    &cancellation,
                ));
            }
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::InvocationStoreActor;
    use crate::application::invocation_store::{
        InvocationStore, InvocationStoreError, NewInvocationRecord, SafeStatusMessage,
        StoredInvocationRecord, TaskTransition, ToolIdentity,
    };
    use crate::domain::cancellation::CancellationToken;
    use crate::domain::invocation::{
        InvocationId, NormalizedArgumentsHash, SafeIdentityHash, TaskId,
    };
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{mpsc, Arc, Mutex};
    use std::time::{Duration, Instant};

    struct BlockingStore {
        entered: Mutex<Option<mpsc::Sender<()>>>,
        release: Mutex<mpsc::Receiver<()>>,
        get_calls: AtomicUsize,
    }

    struct PanickingStore;

    impl InvocationStore for PanickingStore {
        fn create(
            &self,
            _record: NewInvocationRecord,
        ) -> Result<StoredInvocationRecord, InvocationStoreError> {
            unreachable!()
        }

        fn create_working(
            &self,
            _record: NewInvocationRecord,
        ) -> Result<StoredInvocationRecord, InvocationStoreError> {
            panic!("injected store worker failure")
        }

        fn get(&self, _task_id: TaskId) -> Result<StoredInvocationRecord, InvocationStoreError> {
            unreachable!()
        }

        fn update(
            &self,
            _task_id: TaskId,
            _transition: TaskTransition,
        ) -> Result<StoredInvocationRecord, InvocationStoreError> {
            unreachable!()
        }

        fn cancel(
            &self,
            _task_id: TaskId,
            _status_message: SafeStatusMessage,
        ) -> Result<StoredInvocationRecord, InvocationStoreError> {
            unreachable!()
        }
    }

    impl InvocationStore for BlockingStore {
        fn create(
            &self,
            _record: NewInvocationRecord,
        ) -> Result<StoredInvocationRecord, InvocationStoreError> {
            unreachable!()
        }

        fn create_working(
            &self,
            record: NewInvocationRecord,
        ) -> Result<StoredInvocationRecord, InvocationStoreError> {
            if let Some(entered) = self.entered.lock().unwrap().take() {
                entered.send(()).unwrap();
            }
            self.release.lock().unwrap().recv().unwrap();
            Ok(record.into_working_stored(1))
        }

        fn get(&self, _task_id: TaskId) -> Result<StoredInvocationRecord, InvocationStoreError> {
            self.get_calls.fetch_add(1, Ordering::SeqCst);
            Err(InvocationStoreError::NotFound)
        }

        fn update(
            &self,
            _task_id: TaskId,
            _transition: TaskTransition,
        ) -> Result<StoredInvocationRecord, InvocationStoreError> {
            unreachable!()
        }

        fn cancel(
            &self,
            _task_id: TaskId,
            _status_message: SafeStatusMessage,
        ) -> Result<StoredInvocationRecord, InvocationStoreError> {
            unreachable!()
        }
    }

    fn new_record() -> NewInvocationRecord {
        NewInvocationRecord::new(
            InvocationId::new(),
            ToolIdentity::Run,
            NormalizedArgumentsHash::from_sha256([0x22; 32]),
            SafeIdentityHash::from_sha256([0x33; 32]),
            SafeStatusMessage::Queued,
            250,
            60_000,
            None,
        )
    }

    #[test]
    fn blocked_store_call_is_bounded_without_releasing_worker_barrier() {
        let (entered, entered_wait) = mpsc::channel();
        let (release, release_wait) = mpsc::channel();
        let actor = InvocationStoreActor::spawn(Arc::new(BlockingStore {
            entered: Mutex::new(Some(entered)),
            release: Mutex::new(release_wait),
            get_calls: AtomicUsize::new(0),
        }));
        let cancellation = CancellationToken::new();
        let (finished, finished_wait) = mpsc::channel();
        std::thread::spawn(move || {
            let result = actor.create_working(
                new_record(),
                Instant::now() + Duration::from_millis(40),
                &cancellation,
            );
            finished.send(result).unwrap();
        });

        entered_wait.recv_timeout(Duration::from_secs(1)).unwrap();
        let result = finished_wait
            .recv_timeout(Duration::from_secs(1))
            .expect("caller deadline must not wait for blocked store adapter");
        assert_eq!(result.unwrap_err(), InvocationStoreError::DeadlineExceeded);
        release.send(()).unwrap();
    }

    #[test]
    fn queued_call_expires_behind_a_stuck_store_worker_without_late_execution() {
        let (entered, entered_wait) = mpsc::channel();
        let (release, release_wait) = mpsc::channel();
        let store = Arc::new(BlockingStore {
            entered: Mutex::new(Some(entered)),
            release: Mutex::new(release_wait),
            get_calls: AtomicUsize::new(0),
        });
        let actor = InvocationStoreActor::spawn(store.clone());
        let first_actor = actor.clone();
        let first = std::thread::spawn(move || {
            first_actor.create_working(
                new_record(),
                Instant::now() + Duration::from_secs(1),
                &CancellationToken::new(),
            )
        });
        entered_wait.recv_timeout(Duration::from_secs(1)).unwrap();

        assert_eq!(
            actor
                .get(
                    TaskId::new(),
                    Instant::now() + Duration::from_millis(40),
                    &CancellationToken::new(),
                )
                .unwrap_err(),
            InvocationStoreError::DeadlineExceeded
        );
        release.send(()).unwrap();
        first.join().unwrap().unwrap();
        assert_eq!(
            actor
                .get(
                    TaskId::new(),
                    Instant::now() + Duration::from_secs(1),
                    &CancellationToken::new(),
                )
                .unwrap_err(),
            InvocationStoreError::NotFound
        );
        assert_eq!(store.get_calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn failed_store_worker_returns_a_closed_unavailable_error() {
        let actor = InvocationStoreActor::spawn(Arc::new(PanickingStore));
        assert_eq!(
            actor
                .create_working(
                    new_record(),
                    Instant::now() + Duration::from_secs(1),
                    &CancellationToken::new(),
                )
                .unwrap_err(),
            InvocationStoreError::ActorUnavailable
        );
    }

    #[test]
    pub(crate) fn daemon_store_actor_bounds_blocked_adapter_without_waiting() {
        blocked_store_call_is_bounded_without_releasing_worker_barrier();
        queued_call_expires_behind_a_stuck_store_worker_without_late_execution();
        failed_store_worker_returns_a_closed_unavailable_error();
    }
}
