//! The lock serializing runner invocations of one working copy.
//!
//! The runner reads the project config in place; Unica only keeps two of its
//! own invocations for the same working copy from overlapping.
#[cfg(any(test, feature = "test-support"))]
use std::cell::RefCell;
use std::{
    fs::{self, File, OpenOptions},
    path::Path,
    time::Duration,
};
use unica_work_control::cancellation::CancellationToken;

const LOCK_FILE_NAME: &str = "runner.lock";

/// Held for the whole runner invocation; dropping it releases the lock.
#[derive(Debug)]
pub struct RunnerLock {
    _file: File,
}

#[derive(Debug)]
pub enum RunnerLockFailure {
    Cancelled,
    InvalidState(String),
}

impl std::fmt::Display for RunnerLockFailure {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Cancelled => formatter
                .write_str("runner invocation cancelled while waiting for the working copy"),
            Self::InvalidState(message) => formatter.write_str(message),
        }
    }
}

#[cfg(any(test, feature = "test-support"))]
thread_local! {
    static TEST_LOCK_CONTENDED: RefCell<Option<Box<dyn FnOnce()>>> = const { RefCell::new(None) };
}

impl RunnerLock {
    /// Waits for the lock kept in `directory` until it is free or the call is
    /// cancelled.
    pub fn acquire(
        directory: &Path,
        cancellation: &CancellationToken,
    ) -> Result<Self, RunnerLockFailure> {
        use fs2::FileExt;
        let failure = |action: &str, error: std::io::Error| {
            RunnerLockFailure::InvalidState(format!(
                "failed to {action} the runner lock in {}: {error}",
                directory.display()
            ))
        };
        fs::create_dir_all(directory).map_err(|error| failure("create", error))?;
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(directory.join(LOCK_FILE_NAME))
            .map_err(|error| failure("open", error))?;
        loop {
            if cancellation.is_cancelled() {
                return Err(RunnerLockFailure::Cancelled);
            }
            match file.try_lock_exclusive() {
                Ok(()) => return Ok(Self { _file: file }),
                Err(error) if crate::platform::filesystem::is_file_lock_contended(&error) => {
                    #[cfg(any(test, feature = "test-support"))]
                    TEST_LOCK_CONTENDED.with(|slot| {
                        if let Some(hook) = slot.borrow_mut().take() {
                            hook();
                        }
                    });
                    std::thread::sleep(Duration::from_millis(10));
                }
                Err(error) => return Err(failure("take", error)),
            }
        }
    }
}

/// Runs `run` with `hook` called once when this thread finds the lock taken.
#[cfg(any(test, feature = "test-support"))]
pub fn with_test_lock_contention<T>(hook: impl FnOnce() + 'static, run: impl FnOnce() -> T) -> T {
    struct Restore(Option<Box<dyn FnOnce()>>);
    impl Drop for Restore {
        fn drop(&mut self) {
            TEST_LOCK_CONTENDED.with(|slot| *slot.borrow_mut() = self.0.take());
        }
    }
    let _restore = Restore(TEST_LOCK_CONTENDED.with(|slot| slot.replace(Some(Box::new(hook)))));
    run()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_working_copy_waits_and_another_does_not() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("workspace-a");
        let owner = RunnerLock::acquire(&path, &CancellationToken::new()).unwrap();

        let cancellation = CancellationToken::new();
        let contender_cancellation = cancellation.clone();
        let contender_path = path.clone();
        let (contended_tx, contended_rx) = std::sync::mpsc::channel();
        let contender = std::thread::spawn(move || {
            with_test_lock_contention(
                move || contended_tx.send(()).unwrap(),
                || RunnerLock::acquire(&contender_path, &contender_cancellation).err(),
            )
        });
        contended_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("the same working copy waits for the lock");
        cancellation.cancel();
        assert!(matches!(
            contender.join().unwrap().unwrap(),
            RunnerLockFailure::Cancelled
        ));

        let sibling = root.path().join("workspace-b");
        RunnerLock::acquire(&sibling, &CancellationToken::new())
            .expect("another working copy has its own lock");
        drop(owner);
        RunnerLock::acquire(&path, &CancellationToken::new()).unwrap();
    }
}
