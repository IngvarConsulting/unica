// Local, fixed-schema observations of capacity pressure. A lower bound from a
// refused operation is never a measured size of the full workspace.

use crate::application::result_store::ViewCapacityObserver;
use crate::infrastructure::daemon::identity::DaemonStateDirectory;
use crate::infrastructure::platform::filesystem::{
    create_owner_only_file_child, file_identity, open_absolute_directory_path_nofollow,
    open_directory_child_nofollow, open_directory_ownership_lock, open_regular_child_nofollow,
    remove_identity_bound_regular_child, replace_identity_bound_regular_child,
    restrict_stage_to_owner, sync_directory, verify_owner_only_acl, RetainedDirectoryCapability,
};
use crate::infrastructure::platform::process_peak_rss_bytes;
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use std::ffi::OsStr;
use std::fs::File;
use std::io::{self, Read, Write};
use std::path::Path;
use std::sync::{mpsc, Arc, Mutex};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const DIRECTORY_NAME: &str = "capacity-observation";
const SNAPSHOT_NAME: &str = "snapshot-v1.json";
const STAGING_NAME: &str = ".snapshot-v1.tmp";
const WRITER_LOCK_NAME: &str = ".capacity-writer.lock";
const SCHEMA_VERSION: u8 = 1;
const BUCKETS: usize = 32;
const FLUSH_INTERVAL: Duration = Duration::from_secs(60);
const SHUTDOWN_FLUSH_WAIT: Duration = Duration::from_secs(2);
pub(crate) const MAX_SNAPSHOT_BYTES: usize = 16 * 1024;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct Distribution {
    pub(crate) exact_count: u64,
    pub(crate) lower_bound_count: u64,
    pub(crate) max_exact: u64,
    pub(crate) max_lower_bound: u64,
    exact_log4_buckets: [u64; BUCKETS],
}

impl Default for Distribution {
    fn default() -> Self {
        Self {
            exact_count: 0,
            lower_bound_count: 0,
            max_exact: 0,
            max_lower_bound: 0,
            exact_log4_buckets: [0; BUCKETS],
        }
    }
}

impl Distribution {
    fn record(&mut self, value: u64, exact: bool) {
        if exact {
            self.exact_count = self.exact_count.saturating_add(1);
            self.max_exact = self.max_exact.max(value);
            // Bin n covers [4^n, 4^(n+1)); zero shares bin zero with one.
            let bucket = (value.max(1).ilog2() / 2).min((BUCKETS - 1) as u32);
            let counter = &mut self.exact_log4_buckets[bucket as usize];
            *counter = counter.saturating_add(1);
        } else {
            self.lower_bound_count = self.lower_bound_count.saturating_add(1);
            self.max_lower_bound = self.max_lower_bound.max(value);
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct CapacitySnapshot {
    schema_version: u8,
    pub(crate) find_identity_estimate_bytes: Distribution,
    pub(crate) find_entries: Distribution,
    /// One immediate source collection, not a whole workspace directory.
    pub(crate) find_collection_entries: Distribution,
    pub(crate) view_snapshot_charge_bytes: Distribution,
    pub(crate) view_admission_refusals: u64,
    /// Capture time of the snapshot that was actually published on disk.
    pub(crate) snapshot_captured_unix_ms: Option<u64>,
    /// Highest daemon process lifetime RSS peak seen across persisted runs.
    /// It is process-wide, never attributable to a particular tool call.
    pub(crate) max_daemon_process_peak_rss_bytes: Option<u64>,
}

impl Default for CapacitySnapshot {
    fn default() -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            find_identity_estimate_bytes: Distribution::default(),
            find_entries: Distribution::default(),
            find_collection_entries: Distribution::default(),
            view_snapshot_charge_bytes: Distribution::default(),
            view_admission_refusals: 0,
            snapshot_captured_unix_ms: None,
            max_daemon_process_peak_rss_bytes: None,
        }
    }
}

#[derive(Default)]
pub(crate) struct CapacityObserver {
    data: Mutex<CapacitySnapshot>,
}

impl CapacityObserver {
    pub(crate) fn record_find(&self, fact_bytes: u64, entries: u64, complete: bool) {
        let mut data = self
            .data
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        data.find_identity_estimate_bytes
            .record(fact_bytes, complete);
        data.find_entries.record(entries, complete);
    }

    pub(crate) fn record_find_collection(&self, entries: u64, exact: bool) {
        let mut data = self
            .data
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        data.find_collection_entries.record(entries, exact);
    }

    pub(crate) fn record_view(&self, bytes: u64, exact: bool, admitted: bool) {
        let mut data = self
            .data
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        data.view_snapshot_charge_bytes.record(bytes, exact);
        if !admitted {
            data.view_admission_refusals = data.view_admission_refusals.saturating_add(1);
        }
    }

    pub(crate) fn record_view_admission_refusal(&self) {
        let mut data = self
            .data
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        data.view_admission_refusals = data.view_admission_refusals.saturating_add(1);
    }

    pub(crate) fn snapshot(&self) -> CapacitySnapshot {
        self.data
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    fn restore(&self, prior: CapacitySnapshot) {
        *self
            .data
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = prior;
    }

    fn sample_process_peak_rss(&self) {
        let Some(peak) = process_peak_rss_bytes() else {
            return;
        };
        let mut data = self
            .data
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        data.max_daemon_process_peak_rss_bytes = Some(
            data.max_daemon_process_peak_rss_bytes
                .unwrap_or(0)
                .max(peak),
        );
    }
}

impl ViewCapacityObserver for CapacityObserver {
    fn record_view(&self, bytes: u64, exact: bool, admitted: bool) {
        CapacityObserver::record_view(self, bytes, exact, admitted);
    }

    fn record_view_admission_refusal(&self) {
        CapacityObserver::record_view_admission_refusal(self);
    }
}

fn encode_snapshot(snapshot: &CapacitySnapshot) -> io::Result<Vec<u8>> {
    let mut bytes = serde_json::to_vec(snapshot).map_err(io::Error::other)?;
    bytes.push(b'\n');
    if bytes.len() > MAX_SNAPSHOT_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "capacity snapshot is too large",
        ));
    }
    Ok(bytes)
}

fn read_snapshot(directory: &File) -> io::Result<Option<CapacitySnapshot>> {
    let file = match open_regular_child_nofollow(directory, OsStr::new(SNAPSHOT_NAME)) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    verify_owner_only_acl(&file)?;
    let mut bytes = Vec::new();
    file.take((MAX_SNAPSHOT_BYTES + 1) as u64)
        .read_to_end(&mut bytes)?;
    if bytes.len() > MAX_SNAPSHOT_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "capacity snapshot is too large",
        ));
    }
    let snapshot: CapacitySnapshot = serde_json::from_slice(&bytes).map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "capacity snapshot has invalid schema",
        )
    })?;
    if snapshot.schema_version != SCHEMA_VERSION {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "capacity snapshot version is unsupported",
        ));
    }
    Ok(Some(snapshot))
}

/// This path is read-only. In particular it never calls DaemonStateDirectory::open,
/// which creates its missing directories.
pub(crate) fn read_existing_snapshot(
    daemon_identity_path: &Path,
) -> io::Result<Option<CapacitySnapshot>> {
    let identity = match open_absolute_directory_path_nofollow(daemon_identity_path) {
        Ok(directory) => directory,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    verify_owner_only_acl(&identity)?;
    let directory = match open_directory_child_nofollow(&identity, OsStr::new(DIRECTORY_NAME)) {
        Ok(directory) => directory,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    verify_owner_only_acl(&directory)?;
    read_snapshot(&directory)
}

pub(crate) fn read_existing_snapshot_json(
    daemon_identity_path: &Path,
) -> io::Result<Option<String>> {
    read_existing_snapshot(daemon_identity_path)?
        .map(|snapshot| {
            let bytes = encode_snapshot(&snapshot)?;
            String::from_utf8(bytes).map_err(io::Error::other)
        })
        .transpose()
}

fn remove_stale_stage(directory: &File) -> io::Result<()> {
    let file = match open_regular_child_nofollow(directory, OsStr::new(STAGING_NAME)) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };
    verify_owner_only_acl(&file)?;
    let identity = file_identity(&file)?;
    remove_identity_bound_regular_child(directory, OsStr::new(STAGING_NAME), identity, &file)?;
    Ok(())
}

fn persist_snapshot(directory: &File, snapshot: &CapacitySnapshot) -> io::Result<()> {
    let bytes = encode_snapshot(snapshot)?;
    remove_stale_stage(directory)?;
    let mut file = create_owner_only_file_child(directory, OsStr::new(STAGING_NAME))?;
    let identity = file_identity(&file)?;
    if let Err(error) = restrict_stage_to_owner(&file)
        .and_then(|()| file.write_all(&bytes))
        .and_then(|()| file.sync_all())
        .and_then(|()| {
            replace_identity_bound_regular_child(
                directory,
                OsStr::new(STAGING_NAME),
                identity,
                &file,
                OsStr::new(SNAPSHOT_NAME),
            )
        })
        .and_then(|()| sync_directory(directory))
    {
        let _ = remove_identity_bound_regular_child(
            directory,
            OsStr::new(STAGING_NAME),
            identity,
            &file,
        );
        return Err(error);
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CapacityFlushResult {
    Saved,
    WriteFailed,
    Unauthorized,
}

pub(crate) struct CapacityWriterGuard {
    stop: mpsc::Sender<()>,
    flushed: mpsc::Receiver<CapacityFlushResult>,
}

impl Drop for CapacityWriterGuard {
    fn drop(&mut self) {
        // Normal short-lived daemons must not discard the entire interval.
        // Diagnostic I/O cannot hold shutdown indefinitely if the filesystem stalls.
        let _ = self.stop.send(());
        if !matches!(
            self.flushed.recv_timeout(SHUTDOWN_FLUSH_WAIT),
            Ok(CapacityFlushResult::Saved | CapacityFlushResult::Unauthorized)
        ) {
            eprintln!("local capacity observation: shutdown snapshot was not saved");
        }
    }
}

pub(crate) fn start_background_writer(
    state: &DaemonStateDirectory,
    observer: Arc<CapacityObserver>,
    authority_valid: Arc<dyn Fn() -> bool + Send + Sync>,
) -> io::Result<CapacityWriterGuard> {
    let retained = state
        .create_private_retained_subdirectory(DIRECTORY_NAME)
        .map_err(io::Error::other)?;
    let directory = retained.try_clone_directory()?;
    // The receipt owner starts this writer only after it has published and
    // revalidated its endpoint. Keep a separate physical lock for the entire
    // observation lifetime, including the final flush, so a replacement
    // receipt owner cannot write the same aggregate concurrently.
    let writer_lock = open_directory_ownership_lock(&directory, OsStr::new(WRITER_LOCK_NAME))?;
    verify_owner_only_acl(&writer_lock)?;
    writer_lock.try_lock_exclusive()?;
    if let Some(prior) = read_snapshot(&directory)? {
        observer.restore(prior);
    }
    remove_stale_stage(&directory)?;
    let (stop, receiver) = mpsc::channel();
    let (flushed, completed) = mpsc::channel();
    thread::Builder::new()
        .name("unica-capacity-observation".to_string())
        .spawn(move || {
            background_flush_loop(
                retained,
                observer,
                authority_valid,
                writer_lock,
                receiver,
                flushed,
            )
        })?;
    Ok(CapacityWriterGuard {
        stop,
        flushed: completed,
    })
}

fn background_flush_loop(
    retained: RetainedDirectoryCapability,
    observer: Arc<CapacityObserver>,
    authority_valid: Arc<dyn Fn() -> bool + Send + Sync>,
    writer_lock: File,
    receiver: mpsc::Receiver<()>,
    flushed: mpsc::Sender<CapacityFlushResult>,
) {
    let mut write_failure_reported = false;
    loop {
        let stopping = match receiver.recv_timeout(FLUSH_INTERVAL) {
            Ok(()) | Err(mpsc::RecvTimeoutError::Disconnected) => true,
            Err(mpsc::RecvTimeoutError::Timeout) => false,
        };
        let result = if authority_valid() {
            observer.sample_process_peak_rss();
            if flush_snapshot(&retained, &observer).is_ok() {
                CapacityFlushResult::Saved
            } else {
                CapacityFlushResult::WriteFailed
            }
        } else {
            CapacityFlushResult::Unauthorized
        };
        if result == CapacityFlushResult::WriteFailed && !write_failure_reported && !stopping {
            eprintln!("local capacity observation: snapshot write failed");
        }
        if result != CapacityFlushResult::Unauthorized {
            write_failure_reported = result == CapacityFlushResult::WriteFailed;
        }
        if stopping {
            drop(writer_lock);
            let _ = flushed.send(result);
            return;
        }
    }
}

fn flush_snapshot(
    retained: &RetainedDirectoryCapability,
    observer: &CapacityObserver,
) -> io::Result<()> {
    retained
        .validate_named_identity()
        .map_err(io::Error::other)?;
    let directory = retained.try_clone_directory()?;
    let mut snapshot = observer.snapshot();
    snapshot.snapshot_captured_unix_ms = Some(
        u64::try_from(
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_err(io::Error::other)?
                .as_millis(),
        )
        .map_err(io::Error::other)?,
    );
    persist_snapshot(&directory, &snapshot)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_and_censored_find_samples_do_not_claim_the_same_coverage() {
        let observer = CapacityObserver::default();
        observer.record_find(1_000, 4, true);
        observer.record_find(16 * 1024 * 1024 + 12, 501, false);
        let snapshot = observer.snapshot();
        assert_eq!(snapshot.find_identity_estimate_bytes.exact_count, 1);
        assert_eq!(snapshot.find_identity_estimate_bytes.lower_bound_count, 1);
        assert_eq!(snapshot.find_identity_estimate_bytes.max_exact, 1_000);
        assert_eq!(
            snapshot.find_identity_estimate_bytes.max_lower_bound,
            16 * 1024 * 1024 + 12
        );
        assert_eq!(snapshot.find_entries.max_lower_bound, 501);
    }

    #[test]
    fn report_is_fixed_size_and_contains_no_workspace_identifiers() {
        let observer = CapacityObserver::default();
        observer.record_find(u64::MAX, u64::MAX, false);
        observer.record_view(u64::MAX, false, false);
        let bytes = encode_snapshot(&observer.snapshot()).unwrap();
        assert!(bytes.len() <= MAX_SNAPSHOT_BYTES);
        fn only_numeric_leaves(value: &serde_json::Value) -> bool {
            match value {
                serde_json::Value::Null | serde_json::Value::Number(_) => true,
                serde_json::Value::Array(items) => items.iter().all(only_numeric_leaves),
                serde_json::Value::Object(fields) => fields.values().all(only_numeric_leaves),
                serde_json::Value::Bool(_) | serde_json::Value::String(_) => false,
            }
        }
        assert!(only_numeric_leaves(
            &serde_json::from_slice(&bytes).unwrap()
        ));
    }

    #[test]
    fn reading_absent_observations_does_not_create_directories() {
        let root = tempfile::tempdir().unwrap();
        let absent = root.path().canonicalize().unwrap().join("absent");
        assert!(read_existing_snapshot(&absent).unwrap().is_none());
        assert!(!absent.exists());
    }

    #[test]
    fn snapshot_replacement_is_atomic_and_bounded() {
        use crate::infrastructure::daemon::identity::CoreIdentity;

        let root = tempfile::tempdir().unwrap();
        let root = root.path().canonicalize().unwrap();
        let state = DaemonStateDirectory::open(&root, &CoreIdentity::production_v5()).unwrap();
        let directory = state.create_private_subdirectory(DIRECTORY_NAME).unwrap();
        let observer = CapacityObserver::default();
        observer.record_find(500, 2, true);
        persist_snapshot(&directory, &observer.snapshot()).unwrap();
        let prior = read_existing_snapshot(state.path()).unwrap().unwrap();
        assert_eq!(prior.find_identity_estimate_bytes.max_exact, 500);

        observer.record_find(1_000, 4, true);
        persist_snapshot(&directory, &observer.snapshot()).unwrap();
        let latest = read_existing_snapshot(state.path()).unwrap().unwrap();
        assert_eq!(latest.find_identity_estimate_bytes.exact_count, 2);
        assert_eq!(latest.find_identity_estimate_bytes.max_exact, 1_000);
        assert!(!state
            .path()
            .join(DIRECTORY_NAME)
            .join(STAGING_NAME)
            .exists());
        assert!(
            std::fs::metadata(state.path().join(DIRECTORY_NAME).join(SNAPSHOT_NAME))
                .unwrap()
                .len()
                <= MAX_SNAPSHOT_BYTES as u64
        );
    }

    #[test]
    fn counters_saturate_without_expanding_the_report() {
        let observer = CapacityObserver::default();
        let mut prior = CapacitySnapshot::default();
        prior.find_identity_estimate_bytes.exact_count = u64::MAX;
        prior.find_identity_estimate_bytes.exact_log4_buckets[0] = u64::MAX;
        prior.view_admission_refusals = u64::MAX;
        for distribution in [
            &mut prior.find_identity_estimate_bytes,
            &mut prior.find_entries,
            &mut prior.find_collection_entries,
            &mut prior.view_snapshot_charge_bytes,
        ] {
            distribution.exact_count = u64::MAX;
            distribution.lower_bound_count = u64::MAX;
            distribution.max_exact = u64::MAX;
            distribution.max_lower_bound = u64::MAX;
            distribution.exact_log4_buckets.fill(u64::MAX);
        }
        observer.restore(prior);
        observer.record_find(1, 1, true);
        observer.record_view(42, true, false);
        let snapshot = observer.snapshot();
        assert_eq!(snapshot.find_identity_estimate_bytes.exact_count, u64::MAX);
        assert_eq!(
            snapshot.find_identity_estimate_bytes.exact_log4_buckets[0],
            u64::MAX
        );
        assert_eq!(snapshot.view_admission_refusals, u64::MAX);
        assert!(encode_snapshot(&snapshot).unwrap().len() <= MAX_SNAPSHOT_BYTES);
    }

    #[test]
    fn report_reader_rejects_symlink_without_following_it() {
        use crate::infrastructure::daemon::identity::CoreIdentity;
        use crate::infrastructure::platform::testing::{
            create_directory_link_fixture_for_test, FileLinkFixtureOutcome,
        };

        let root = tempfile::tempdir().unwrap();
        let root = root.path().canonicalize().unwrap();
        let state = DaemonStateDirectory::open(&root, &CoreIdentity::production_v5()).unwrap();
        let outside = root.join("outside");
        std::fs::create_dir(&outside).unwrap();
        match create_directory_link_fixture_for_test(&outside, state.path().join(DIRECTORY_NAME))
            .unwrap()
        {
            FileLinkFixtureOutcome::Created => {}
            FileLinkFixtureOutcome::Unsupported
            | FileLinkFixtureOutcome::WindowsPrivilegeUnavailable => return,
        }
        assert!(read_existing_snapshot(state.path()).is_err());
        assert!(start_background_writer(
            &state,
            Arc::new(CapacityObserver::default()),
            Arc::new(|| true),
        )
        .is_err());
        assert!(std::fs::read_dir(&outside).unwrap().next().is_none());
    }

    #[test]
    fn report_reader_rejects_linked_snapshot_and_storage_failure_is_fail_open() {
        use crate::infrastructure::daemon::identity::CoreIdentity;
        use crate::infrastructure::platform::testing::{
            create_file_link_fixture_for_test, FileLinkFixtureOutcome,
        };

        let root = tempfile::tempdir().unwrap();
        let root = root.path().canonicalize().unwrap();
        let state = DaemonStateDirectory::open(&root, &CoreIdentity::production_v5()).unwrap();
        state.create_private_subdirectory(DIRECTORY_NAME).unwrap();
        let outside = root.join("outside.json");
        std::fs::write(&outside, b"sentinel").unwrap();
        match create_file_link_fixture_for_test(
            &outside,
            state.path().join(DIRECTORY_NAME).join(SNAPSHOT_NAME),
        )
        .unwrap()
        {
            FileLinkFixtureOutcome::Created => {}
            FileLinkFixtureOutcome::Unsupported
            | FileLinkFixtureOutcome::WindowsPrivilegeUnavailable => return,
        }

        assert!(read_existing_snapshot(state.path()).is_err());
        let observer = Arc::new(CapacityObserver::default());
        assert!(start_background_writer(&state, Arc::clone(&observer), Arc::new(|| true)).is_err());
        observer.record_find(42, 1, true);
        assert_eq!(observer.snapshot().find_entries.max_exact, 1);
        assert_eq!(std::fs::read(&outside).unwrap(), b"sentinel");
    }

    #[test]
    fn short_lived_daemon_flushes_on_shutdown() {
        use crate::infrastructure::daemon::identity::CoreIdentity;

        let root = tempfile::tempdir().unwrap();
        let root = root.path().canonicalize().unwrap();
        let state = DaemonStateDirectory::open(&root, &CoreIdentity::production_v5()).unwrap();
        let observer = Arc::new(CapacityObserver::default());
        let guard =
            start_background_writer(&state, Arc::clone(&observer), Arc::new(|| true)).unwrap();
        observer.record_find(42, 1, true);
        drop(guard);
        let saved = read_existing_snapshot(state.path()).unwrap().unwrap();
        assert_eq!(saved.find_entries.max_exact, 1);
        assert!(saved.snapshot_captured_unix_ms.is_some());
    }

    #[test]
    fn shutdown_without_authority_preserves_snapshot_and_reports_unauthorized() {
        use crate::infrastructure::daemon::identity::CoreIdentity;

        let root = tempfile::tempdir().unwrap();
        let root = root.path().canonicalize().unwrap();
        let state = DaemonStateDirectory::open(&root, &CoreIdentity::production_v5()).unwrap();
        let retained = state
            .create_private_retained_subdirectory(DIRECTORY_NAME)
            .unwrap();
        let directory = retained.try_clone_directory().unwrap();
        let prior = CapacitySnapshot::default();
        persist_snapshot(&directory, &prior).unwrap();
        let before = std::fs::read(state.path().join(DIRECTORY_NAME).join(SNAPSHOT_NAME)).unwrap();
        let writer_lock =
            open_directory_ownership_lock(&directory, OsStr::new(WRITER_LOCK_NAME)).unwrap();
        writer_lock.try_lock_exclusive().unwrap();

        let observer = Arc::new(CapacityObserver::default());
        observer.record_find(42, 1, true);
        let (stop, receiver) = mpsc::channel();
        let (flushed, completed) = mpsc::channel();
        let worker = thread::spawn(move || {
            background_flush_loop(
                retained,
                observer,
                Arc::new(|| false),
                writer_lock,
                receiver,
                flushed,
            )
        });
        stop.send(()).unwrap();
        assert!(matches!(
            completed.recv_timeout(SHUTDOWN_FLUSH_WAIT),
            Ok(CapacityFlushResult::Unauthorized)
        ));
        worker.join().unwrap();
        assert_eq!(
            std::fs::read(state.path().join(DIRECTORY_NAME).join(SNAPSHOT_NAME)).unwrap(),
            before
        );
    }

    #[test]
    fn one_state_directory_admits_only_one_capacity_writer() {
        use crate::infrastructure::daemon::identity::CoreIdentity;

        let root = tempfile::tempdir().unwrap();
        let root = root.path().canonicalize().unwrap();
        let state = DaemonStateDirectory::open(&root, &CoreIdentity::production_v5()).unwrap();
        let first = start_background_writer(
            &state,
            Arc::new(CapacityObserver::default()),
            Arc::new(|| true),
        )
        .unwrap();
        assert!(start_background_writer(
            &state,
            Arc::new(CapacityObserver::default()),
            Arc::new(|| true),
        )
        .is_err());
        drop(first);
        assert!(start_background_writer(
            &state,
            Arc::new(CapacityObserver::default()),
            Arc::new(|| true),
        )
        .is_ok());
    }

    #[test]
    fn unsupported_snapshot_version_is_preserved_and_disables_observation() {
        use crate::infrastructure::daemon::identity::CoreIdentity;

        let root = tempfile::tempdir().unwrap();
        let root = root.path().canonicalize().unwrap();
        let state = DaemonStateDirectory::open(&root, &CoreIdentity::production_v5()).unwrap();
        let directory = state.create_private_subdirectory(DIRECTORY_NAME).unwrap();
        let mut future = CapacitySnapshot::default();
        future.schema_version += 1;
        let bytes = encode_snapshot(&future).unwrap();
        let mut file = create_owner_only_file_child(&directory, OsStr::new(SNAPSHOT_NAME)).unwrap();
        file.write_all(&bytes).unwrap();
        file.sync_all().unwrap();

        assert!(start_background_writer(
            &state,
            Arc::new(CapacityObserver::default()),
            Arc::new(|| true),
        )
        .is_err());
        assert_eq!(
            std::fs::read(state.path().join(DIRECTORY_NAME).join(SNAPSHOT_NAME)).unwrap(),
            bytes
        );
        assert!(!state
            .path()
            .join(DIRECTORY_NAME)
            .join(STAGING_NAME)
            .exists());
    }
}
