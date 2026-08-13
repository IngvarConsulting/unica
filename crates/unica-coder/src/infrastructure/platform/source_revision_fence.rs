use crate::domain::cancellation::{cancelled_error, CancellationToken};
use crate::domain::code_intelligence::ProviderDeadline;
use crate::domain::source_revision::SourceRevisionTrustLoss;
use std::path::Path;
use std::sync::Arc;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FenceCapability {
    ProvenFast,
    Unsupported,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FenceOutcome {
    Proven { dirty: bool },
    TrustLost(SourceRevisionTrustLoss),
}

pub(crate) trait SourceRevisionFence: Send + Sync {
    fn capability(&self) -> FenceCapability;
    fn flush(
        &self,
        deadline: ProviderDeadline,
        cancellation: &CancellationToken,
    ) -> Result<FenceOutcome, String>;
}

#[cfg(target_os = "macos")]
pub(crate) fn platform_fence(
    root: &Path,
    cache_root: &Path,
) -> Result<Arc<dyn SourceRevisionFence>, String> {
    if !macos::is_local_apfs(root) {
        return platform_fence_for_capability(root, FenceCapability::Unsupported);
    }
    let fence_directory = cache_root.join("source-revision-fences");
    std::fs::create_dir_all(&fence_directory)
        .map_err(|error| format!("failed to create source revision fence cache: {error}"))?;
    let capability = if macos::is_local_apfs(&fence_directory)
        && macos::is_same_device(root, &fence_directory)
    {
        FenceCapability::ProvenFast
    } else {
        FenceCapability::Unsupported
    };
    match capability {
        FenceCapability::ProvenFast => macos::MacSourceRevisionFence::new(root, &fence_directory)
            .map(|fence| Arc::new(fence) as Arc<dyn SourceRevisionFence>),
        FenceCapability::Unsupported => platform_fence_for_capability(root, capability),
    }
}

#[cfg(target_os = "macos")]
fn platform_fence_for_capability(
    root: &Path,
    capability: FenceCapability,
) -> Result<Arc<dyn SourceRevisionFence>, String> {
    match capability {
        FenceCapability::ProvenFast => Err(format!(
            "a proven source revision fence for {} requires a same-device cache directory",
            root.display()
        )),
        FenceCapability::Unsupported => Ok(Arc::new(UnsupportedSourceRevisionFence)),
    }
}

#[cfg(not(target_os = "macos"))]
pub(crate) fn platform_fence(
    _root: &Path,
    _cache_root: &Path,
) -> Result<Arc<dyn SourceRevisionFence>, String> {
    Ok(Arc::new(UnsupportedSourceRevisionFence))
}

struct UnsupportedSourceRevisionFence;

impl SourceRevisionFence for UnsupportedSourceRevisionFence {
    fn capability(&self) -> FenceCapability {
        FenceCapability::Unsupported
    }

    fn flush(
        &self,
        _deadline: ProviderDeadline,
        cancellation: &CancellationToken,
    ) -> Result<FenceOutcome, String> {
        if cancellation.is_cancelled() {
            return Err(cancelled_error("source revision fence stopped"));
        }
        Ok(FenceOutcome::TrustLost(
            SourceRevisionTrustLoss::UnsupportedFence,
        ))
    }
}

#[cfg(target_os = "macos")]
mod macos {
    use super::*;
    use dispatch2::{DispatchQueue, DispatchRetained};
    use objc2_core_foundation::{CFArray, CFString};
    use objc2_core_services::*;
    use std::ffi::{c_void, CStr};
    use std::fs::{self, OpenOptions};
    use std::io::Write;
    use std::os::fd::AsRawFd;
    use std::path::PathBuf;
    use std::ptr::NonNull;
    use std::sync::atomic::{AtomicBool, AtomicU64, AtomicU8, Ordering};
    use std::sync::{Condvar, Mutex};

    const TRUSTED: u8 = 0;
    const WATCHER_GAP: u8 = 1;
    const OVERFLOW: u8 = 2;
    const ROOT_CHANGED: u8 = 3;

    struct WatcherState {
        dirty: AtomicBool,
        trust_loss: AtomicU8,
        source_root: Vec<u8>,
        marker_root: Vec<u8>,
        marker_event_id: Mutex<FSEventStreamEventId>,
        marker_changed: Condvar,
    }

    pub(super) struct MacSourceRevisionFence {
        stream: FSEventStreamRef,
        queue: DispatchRetained<DispatchQueue>,
        state: Box<WatcherState>,
        capability: FenceCapability,
        marker_path: PathBuf,
        marker_sequence: AtomicU64,
    }

    // FSEvents serializes callbacks on `_queue`; `flush` is the documented
    // synchronous barrier and atomics publish callback state to callers.
    unsafe impl Send for MacSourceRevisionFence {}
    unsafe impl Sync for MacSourceRevisionFence {}

    impl MacSourceRevisionFence {
        pub(super) fn new(root: &Path, fence_directory: &Path) -> Result<Self, String> {
            let marker_path = fence_directory.join(format!("{}.fence", uuid::Uuid::new_v4()));
            fs::write(&marker_path, b"0").map_err(|error| {
                format!("failed to initialize source revision fence marker: {error}")
            })?;
            let marker_path = fs::canonicalize(&marker_path).map_err(|error| {
                format!("failed to resolve source revision fence marker: {error}")
            })?;
            let marker_root = marker_path
                .parent()
                .ok_or_else(|| "source revision fence marker has no parent".to_string())?;
            let marker_root_bytes = path_bytes(marker_root, "source revision marker root")?;
            let source_root = fs::canonicalize(root)
                .map_err(|error| format!("failed to resolve source revision root: {error}"))?;
            let source_root_bytes = path_bytes(&source_root, "source revision root")?;
            let root = source_root
                .to_str()
                .ok_or_else(|| "source revision root is not UTF-8".to_string())?;
            let watched_path = CFString::from_str(root);
            let marker_directory = marker_path
                .parent()
                .and_then(Path::to_str)
                .ok_or_else(|| "source revision fence cache path is not UTF-8".to_string())?;
            let watched_marker_directory = CFString::from_str(marker_directory);
            let paths = CFArray::from_objects(&[&*watched_path, &*watched_marker_directory]);
            let erased_paths: &CFArray =
                unsafe { &*((paths.as_ref() as *const CFArray<CFString>).cast::<CFArray>()) };
            let mut state = Box::new(WatcherState {
                dirty: AtomicBool::new(false),
                trust_loss: AtomicU8::new(TRUSTED),
                source_root: source_root_bytes,
                marker_root: marker_root_bytes,
                marker_event_id: Mutex::new(0),
                marker_changed: Condvar::new(),
            });
            let mut context = FSEventStreamContext {
                version: 0,
                info: (&mut *state as *mut WatcherState).cast::<c_void>(),
                retain: None,
                release: None,
                copyDescription: None,
            };
            let flags = kFSEventStreamCreateFlagFileEvents
                | kFSEventStreamCreateFlagWatchRoot
                | kFSEventStreamCreateFlagNoDefer;
            let stream = unsafe {
                FSEventStreamCreate(
                    None,
                    Some(handle_events),
                    &mut context,
                    erased_paths,
                    kFSEventStreamEventIdSinceNow,
                    0.05,
                    flags,
                )
            };
            if stream.is_null() {
                return Err("failed to create FSEvents source revision stream".to_string());
            }
            let queue = DispatchQueue::new("io.unica.source-revision", None);
            unsafe {
                FSEventStreamSetDispatchQueue(stream, Some(&queue));
                if !FSEventStreamStart(stream) {
                    FSEventStreamInvalidate(stream);
                    FSEventStreamSetDispatchQueue(stream, None);
                    FSEventStreamRelease(stream);
                    return Err("failed to start FSEvents source revision stream".to_string());
                }
            }
            Ok(Self {
                stream,
                queue,
                state,
                capability: FenceCapability::ProvenFast,
                marker_path,
                marker_sequence: AtomicU64::new(0),
            })
        }
    }

    impl SourceRevisionFence for MacSourceRevisionFence {
        fn capability(&self) -> FenceCapability {
            self.capability
        }

        fn flush(
            &self,
            deadline: ProviderDeadline,
            cancellation: &CancellationToken,
        ) -> Result<FenceOutcome, String> {
            if cancellation.is_cancelled() {
                return Err(cancelled_error("source revision fence stopped"));
            }
            if deadline.remaining().is_zero() {
                return Err("source revision fence deadline exceeded".to_string());
            }
            if self.capability != FenceCapability::ProvenFast {
                return Ok(FenceOutcome::TrustLost(
                    SourceRevisionTrustLoss::UnsupportedFence,
                ));
            }
            // Drain everything before publishing the epoch marker. An older
            // callback cannot then satisfy the following event-ID boundary.
            unsafe { FSEventStreamFlushSync(self.stream) };
            self.queue.exec_sync(|| {});
            *self
                .state
                .marker_event_id
                .lock()
                .unwrap_or_else(|error| error.into_inner()) = 0;
            let sequence = self.marker_sequence.fetch_add(1, Ordering::AcqRel) + 1;
            let mut marker = OpenOptions::new()
                .write(true)
                .truncate(true)
                .open(&self.marker_path)
                .map_err(|error| {
                    format!("source revision fence marker cannot be opened: {error}")
                })?;
            marker
                .write_all(sequence.to_string().as_bytes())
                .and_then(|_| marker.sync_all())
                .map_err(|error| {
                    format!("source revision fence marker cannot be flushed: {error}")
                })?;
            if unsafe { libc::fcntl(marker.as_raw_fd(), libc::F_FULLFSYNC) } == -1 {
                return Err(format!(
                    "source revision fence marker cannot reach the filesystem journal: {}",
                    std::io::Error::last_os_error()
                ));
            }
            unsafe { FSEventStreamFlushSync(self.stream) };
            self.queue.exec_sync(|| {});
            let mut marker_event_id = self
                .state
                .marker_event_id
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            while *marker_event_id == 0 {
                if cancellation.is_cancelled() {
                    return Err(cancelled_error("source revision fence stopped"));
                }
                let remaining = deadline.remaining();
                if remaining.is_zero() {
                    return Err("source revision fence deadline exceeded".to_string());
                }
                let (guard, wait) = self
                    .state
                    .marker_changed
                    .wait_timeout(marker_event_id, remaining)
                    .unwrap_or_else(|error| error.into_inner());
                marker_event_id = guard;
                if wait.timed_out() && *marker_event_id == 0 {
                    return Err("source revision fence deadline exceeded".to_string());
                }
            }
            drop(marker_event_id);
            if cancellation.is_cancelled() {
                return Err(cancelled_error("source revision fence stopped"));
            }
            if deadline.remaining().is_zero() {
                return Err("source revision fence deadline exceeded".to_string());
            }
            let trust_loss = self.state.trust_loss.swap(TRUSTED, Ordering::AcqRel);
            let outcome = match trust_loss {
                WATCHER_GAP => FenceOutcome::TrustLost(SourceRevisionTrustLoss::WatcherGap),
                OVERFLOW => FenceOutcome::TrustLost(SourceRevisionTrustLoss::Overflow),
                ROOT_CHANGED => FenceOutcome::TrustLost(SourceRevisionTrustLoss::RootChanged),
                _ => FenceOutcome::Proven {
                    dirty: self.state.dirty.swap(false, Ordering::AcqRel),
                },
            };
            Ok(outcome)
        }
    }

    impl Drop for MacSourceRevisionFence {
        fn drop(&mut self) {
            unsafe {
                FSEventStreamStop(self.stream);
                FSEventStreamInvalidate(self.stream);
            }
            self.queue.exec_sync(|| {});
            unsafe {
                FSEventStreamSetDispatchQueue(self.stream, None);
                FSEventStreamRelease(self.stream);
            }
            let _ = fs::remove_file(&self.marker_path);
        }
    }

    unsafe extern "C-unwind" fn handle_events(
        _stream: ConstFSEventStreamRef,
        info: *mut c_void,
        event_count: usize,
        paths: NonNull<c_void>,
        flags: NonNull<FSEventStreamEventFlags>,
        ids: NonNull<FSEventStreamEventId>,
    ) {
        let _ = std::panic::catch_unwind(|| {
            let state = unsafe { &*(info.cast::<WatcherState>()) };
            let flags = unsafe { std::slice::from_raw_parts(flags.as_ptr(), event_count) };
            let ids = unsafe { std::slice::from_raw_parts(ids.as_ptr(), event_count) };
            let paths = unsafe {
                std::slice::from_raw_parts(paths.as_ptr().cast::<*const i8>(), event_count)
            };
            for ((flags, path), id) in flags.iter().zip(paths).zip(ids) {
                if path.is_null() {
                    state.trust_loss.store(WATCHER_GAP, Ordering::Release);
                    continue;
                }
                let loss = if flags & kFSEventStreamEventFlagRootChanged != 0 {
                    ROOT_CHANGED
                } else if flags
                    & (kFSEventStreamEventFlagUserDropped
                        | kFSEventStreamEventFlagKernelDropped
                        | kFSEventStreamEventFlagEventIdsWrapped)
                    != 0
                {
                    OVERFLOW
                } else if flags & kFSEventStreamEventFlagMustScanSubDirs != 0 {
                    WATCHER_GAP
                } else {
                    TRUSTED
                };
                if loss != TRUSTED {
                    state.trust_loss.store(loss, Ordering::Release);
                }
                let path = unsafe { CStr::from_ptr(*path) }.to_bytes();
                if path_is_within(path, &state.marker_root) {
                    let mut marker_event_id = state
                        .marker_event_id
                        .lock()
                        .unwrap_or_else(|error| error.into_inner());
                    *marker_event_id = (*marker_event_id).max(*id);
                    state.marker_changed.notify_all();
                    continue;
                }
                if !path_is_within(path, &state.source_root) {
                    continue;
                }
                state.dirty.store(true, Ordering::Release);
            }
        });
    }

    fn path_bytes(path: &Path, label: &str) -> Result<Vec<u8>, String> {
        path.to_str()
            .map(|path| path.as_bytes().to_vec())
            .ok_or_else(|| format!("{label} is not UTF-8"))
    }

    fn path_is_within(path: &[u8], root: &[u8]) -> bool {
        path == root
            || path
                .strip_prefix(root)
                .is_some_and(|relative| relative.first() == Some(&b'/'))
    }

    pub(super) fn is_local_apfs(root: &Path) -> bool {
        let Ok(path) = std::ffi::CString::new(root.as_os_str().as_encoded_bytes()) else {
            return false;
        };
        let mut stats = std::mem::MaybeUninit::<libc::statfs>::uninit();
        if unsafe { libc::statfs(path.as_ptr(), stats.as_mut_ptr()) } != 0 {
            return false;
        }
        let stats = unsafe { stats.assume_init() };
        let file_system = unsafe { CStr::from_ptr(stats.f_fstypename.as_ptr()) };
        file_system.to_bytes() == b"apfs"
    }

    pub(super) fn is_same_device(left: &Path, right: &Path) -> bool {
        let Ok(left) = fs::metadata(left) else {
            return false;
        };
        let Ok(right) = fs::metadata(right) else {
            return false;
        };
        use std::os::unix::fs::MetadataExt;
        left.dev() == right.dev()
    }
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;
    use crate::domain::cancellation::CancellationToken;
    use crate::domain::code_intelligence::ProviderDeadline;
    use std::fs;
    use std::time::Duration;
    use tempfile::tempdir;

    #[test]
    fn unsupported_volume_falls_back_without_touching_the_source_root() {
        let sandbox = tempdir().unwrap();
        let missing_root = sandbox.path().join("read-only-or-missing-source");

        let fence = platform_fence_for_capability(&missing_root, FenceCapability::Unsupported)
            .expect("unsupported filesystems must use the conservative fence");

        assert_eq!(fence.capability(), FenceCapability::Unsupported);
        assert!(!missing_root.exists());
    }

    #[test]
    fn macos_fsevents_flush_observes_external_write_without_sleep() {
        let root = tempdir().unwrap();
        let cache = tempdir().unwrap();
        let fence_cache = cache.path().join("revision-fence-cache");
        let fence = platform_fence(root.path(), &fence_cache).unwrap();
        if fence.capability() != FenceCapability::ProvenFast {
            return;
        }
        fence
            .flush(
                ProviderDeadline::from_budget(Duration::from_secs(2)),
                &CancellationToken::new(),
            )
            .unwrap();
        assert!(
            !root.path().join(".build").exists(),
            "a read-side freshness fence must not write inside the source root"
        );
        fs::write(root.path().join("Module.bsl"), "Процедура A()\n").unwrap();
        let outcome = fence
            .flush(
                ProviderDeadline::from_budget(Duration::from_secs(2)),
                &CancellationToken::new(),
            )
            .unwrap();
        assert_eq!(outcome, FenceOutcome::Proven { dirty: true });
    }
}
