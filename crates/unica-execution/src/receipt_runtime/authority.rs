//! Stable receipt ownership, using the same retained namespace protocol.
use std::ffi::OsStr;
use std::io;
use std::time::{Duration, Instant};
use unica_workspace::state_fs::{
    open_state_ownership_lock, verify_state_owner_only, StateDirectory, StateHandle,
};

const RECEIPT_AUTHORITY_DIRECTORY_NAME: &str = ".receipt-authority";
const RECEIPT_AUTHORITY_LOCK_NAME: &str = ".receipt-authority.lock";

pub trait ReceiptStateRoot: Send + Sync {
    fn verify_identity(&self) -> Result<(), String>;
    fn create_private_retained_subdirectory(&self, name: &str) -> Result<StateDirectory, String>;
}

pub struct ReceiptAuthorityLock {
    file: StateHandle,
    _authority_directory: StateDirectory,
}

impl ReceiptAuthorityLock {
    pub fn try_acquire(state: &dyn ReceiptStateRoot) -> Result<Option<Self>, String> {
        state.verify_identity()?;
        let authority_directory =
            state.create_private_retained_subdirectory(RECEIPT_AUTHORITY_DIRECTORY_NAME)?;
        let authority_file = authority_directory
            .open_retained_handle()
            .map_err(|error| state_io_error("clone receipt authority directory", error))?;
        let file =
            open_state_ownership_lock(&authority_file, OsStr::new(RECEIPT_AUTHORITY_LOCK_NAME))
                .map_err(|error| {
                    state_io_error("open receipt authority ownership object", error)
                })?;
        match file.try_lock_exclusive() {
            Ok(()) => Ok(Some(ReceiptAuthorityLock {
                file,
                _authority_directory: authority_directory,
            })),
            Err(error) if unica_workspace::state_fs::is_file_lock_contended(&error) => Ok(None),
            Err(error) => Err(state_io_error(
                "lock receipt authority ownership object",
                error,
            )),
        }
    }

    pub fn acquire(state: &dyn ReceiptStateRoot, timeout: Duration) -> Result<Self, String> {
        state.verify_identity()?;
        // Unix ownership locks are directory-inode scoped. A dedicated retained
        // child keeps receipt authority distinct from the state-inode spawn gate.
        let authority_directory =
            state.create_private_retained_subdirectory(RECEIPT_AUTHORITY_DIRECTORY_NAME)?;
        let authority_file = authority_directory
            .open_retained_handle()
            .map_err(|error| state_io_error("clone receipt authority directory", error))?;
        let file =
            open_state_ownership_lock(&authority_file, OsStr::new(RECEIPT_AUTHORITY_LOCK_NAME))
                .map_err(|error| {
                    state_io_error("open receipt authority ownership object", error)
                })?;
        verify_state_owner_only(&file)
            .map_err(|error| state_io_error("verify receipt authority ownership object", error))?;
        let deadline = Instant::now() + timeout;
        loop {
            match file.try_lock_exclusive() {
                Ok(()) => {
                    authority_directory
                        .validate_named_identity()
                        .map_err(|error| {
                            state_io_error("validate named receipt authority directory", error)
                        })?;
                    return Ok(ReceiptAuthorityLock {
                        file,
                        _authority_directory: authority_directory,
                    });
                }
                Err(error)
                    if unica_workspace::state_fs::is_file_lock_contended(&error)
                        && Instant::now() < deadline =>
                {
                    std::thread::sleep(Duration::from_millis(10));
                }
                Err(error) if unica_workspace::state_fs::is_file_lock_contended(&error) => {
                    return Err("timed out waiting for stable receipt authority".to_string())
                }
                Err(error) => {
                    return Err(state_io_error(
                        "lock receipt authority ownership object",
                        error,
                    ))
                }
            }
        }
    }

    #[cfg(any(
        test,
        feature = "test-support",
        feature = "receipt-ledger-test-support"
    ))]
    pub fn ownership_file_identity_for_test(&self) -> unica_workspace::state_fs::FileIdentity {
        unica_workspace::state_fs::state_file_identity(&self.file)
            .expect("retained receipt authority identity")
    }
}

impl Drop for ReceiptAuthorityLock {
    fn drop(&mut self) {
        let _ = self.file.unlock();
    }
}

/// The existing state I/O error text, shared by the Daemon and its receipt owner.
pub fn state_io_error(operation: &str, error: io::Error) -> String {
    format!("{operation}: {error}")
}
