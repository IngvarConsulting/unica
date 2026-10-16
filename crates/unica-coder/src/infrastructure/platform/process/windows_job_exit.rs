//! Complete lifetime coverage is required before a Windows runtime Job can release its lease.

use std::collections::BTreeMap;
use std::io;

enum Witness<H> {
    Handle(H),
    Gone,
    GoneThenHandle(H),
}

struct Coverage<H> {
    members: BTreeMap<u32, Witness<H>>,
    last_total: Option<u32>,
    accounting_invalid: bool,
}

impl<H> Coverage<H> {
    fn new() -> Self {
        Self {
            members: BTreeMap::new(),
            last_total: None,
            accounting_invalid: false,
        }
    }

    fn pinned(&self, pid: u32) -> bool {
        matches!(
            self.members.get(&pid),
            Some(Witness::Handle(_) | Witness::GoneThenHandle(_))
        )
    }

    #[cfg(test)]
    fn contains(&self, pid: u32) -> bool {
        self.members.contains_key(&pid)
    }

    fn record(&mut self, pid: u32, witness: Witness<H>) -> io::Result<()> {
        if pid == 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "zero Job process identifier",
            ));
        }
        // Duplicate historical observations do not identify another lifetime.
        self.members.entry(pid).or_insert(witness);
        Ok(())
    }

    fn observe_total(&mut self, total: u32) -> io::Result<()> {
        if self.last_total.is_some_and(|previous| total < previous) {
            self.accounting_invalid = true;
        }
        if self.accounting_invalid {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Windows Job lifetime accounting moved backwards",
            ));
        }
        self.last_total = Some(total);
        Ok(())
    }

    fn record_opened(
        &mut self,
        pid: u32,
        handle: H,
        in_job: bool,
        mut signaled: impl FnMut(&H) -> io::Result<bool>,
    ) -> io::Result<bool> {
        // A successful negative membership check alone cannot retire the
        // historical member. If this is a replacement PID, the former object
        // is gone; if it is still the original, its signal proves exit.
        // Keep a signaled handle too, to prevent another PID reuse.
        if in_job && matches!(self.members.get(&pid), Some(Witness::Gone)) {
            // Gone proves the former object was destroyed. A later retained
            // handle with positive membership proves a distinct owned lifetime.
            self.members.insert(pid, Witness::GoneThenHandle(handle));
            return Ok(true);
        }
        if in_job || signaled(&handle)? {
            self.record(pid, Witness::Handle(handle))?;
            return Ok(true);
        }
        Ok(false)
    }

    fn complete(&self, total: u32) -> bool {
        let extra = self
            .members
            .values()
            .filter(|witness| matches!(witness, Witness::GoneThenHandle(_)))
            .count();
        self.members.len() as u64 + extra as u64 == u64::from(total)
    }

    fn ready(
        &mut self,
        active: u32,
        total: u32,
        mut signaled: impl FnMut(&H) -> io::Result<bool>,
    ) -> io::Result<bool> {
        self.observe_total(total)?;
        if active != 0 || !self.complete(total) {
            return Ok(false);
        }
        for witness in self.members.values() {
            if let Witness::Handle(handle) | Witness::GoneThenHandle(handle) = witness {
                if !signaled(handle)? {
                    return Ok(false);
                }
            }
        }
        Ok(true)
    }
}

fn is_missing_process_error(error: Option<i32>) -> bool {
    // Exact OpenProcess(nonzero PID, fixed valid query/sync rights) classification
    // used by Microsoft .NET ProcessManager.Windows.IsProcessRunning. No other
    // error, including access denied, proves process-object destruction.
    error == Some(87) // ERROR_INVALID_PARAMETER
}

#[cfg(windows)]
use super::ScopedWindowsHandle;
#[cfg(windows)]
use windows_sys::Win32::{
    Foundation as wf,
    System::{JobObjects as wjob, SystemServices as ws, Threading as wt, IO as wio},
};

#[cfg(windows)]
use std::sync::{
    atomic::{AtomicBool, Ordering},
    mpsc::{self, Receiver, Sender, TryRecvError},
    Arc, Weak,
};

#[cfg(windows)]
struct Candidate {
    pid: u32,
    captured: Option<Witness<Arc<ScopedWindowsHandle>>>,
}

#[cfg(windows)]
enum Collection {
    Candidate(Candidate),
    Failed(io::Error),
}

#[cfg(windows)]
pub(super) struct Census {
    port: Arc<ScopedWindowsHandle>,
    stop: Arc<AtomicBool>,
    collected: Receiver<Collection>,
    collector_failed: bool,
    coverage: Coverage<Arc<ScopedWindowsHandle>>,
    pid_buffer: Vec<usize>,
    pending: std::collections::VecDeque<Candidate>,
}

#[cfg(windows)]
impl Census {
    // A work portion, never a lifetime/admission limit. The collector captures
    // handles independently of owner polling; each owner poll validates a portion.
    const PORTION: usize = 64;
    const KEY: usize = 1;
    const STOP_KEY: usize = 2;
    // A failed stop notification must not leave a detached reader forever.
    // This idle poll does not wait for process cleanup or extend its deadline.
    const COLLECTOR_IDLE_MS: u32 = 25;

    pub(super) fn associate(job: wf::HANDLE) -> io::Result<Self> {
        // SAFETY: INVALID_HANDLE_VALUE creates a new anonymous completion port.
        let port = unsafe {
            wio::CreateIoCompletionPort(wf::INVALID_HANDLE_VALUE, std::ptr::null_mut(), 0, 1)
        };
        if port.is_null() {
            return Err(io::Error::last_os_error());
        }
        let port = Arc::new(ScopedWindowsHandle(port));
        let association = wjob::JOBOBJECT_ASSOCIATE_COMPLETION_PORT {
            CompletionKey: Self::KEY as *mut _,
            CompletionPort: port.0,
        };
        // SAFETY: the caller owns the inactive private Job; the exact structure
        // and size are supplied. Neither anonymous handle is inherited.
        if unsafe {
            wjob::SetInformationJobObject(
                job,
                wjob::JobObjectAssociateCompletionPortInformation,
                &association as *const _ as *const _,
                std::mem::size_of_val(&association) as u32,
            )
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
        let stop = Arc::new(AtomicBool::new(false));
        let (sender, collected) = mpsc::channel();
        let collector_port = Arc::clone(&port);
        let collector_stop = Arc::clone(&stop);
        // The worker owns no Job handle or process-control authority. A spawn
        // failure occurs before the child is launched; Drop never joins it.
        std::thread::Builder::new()
            .name("windows-job-exit".into())
            .spawn(move || {
                if let Err(error) = Self::collect(&collector_port, &collector_stop, &sender) {
                    let _ = sender.send(Collection::Failed(error));
                }
            })?;
        Ok(Self {
            port,
            stop,
            collected,
            collector_failed: false,
            coverage: Coverage::new(),
            pid_buffer: vec![0; 8],
            pending: Default::default(),
        })
    }

    fn open_candidate(pid: u32) -> io::Result<Witness<Arc<ScopedWindowsHandle>>> {
        if pid == 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "zero Job process identifier",
            ));
        }
        // SAFETY: fixed valid access flags, non-inheritance and nonzero PID.
        // Query/sync grants no termination authority over a replacement PID.
        let handle = unsafe {
            wt::OpenProcess(
                wt::PROCESS_QUERY_LIMITED_INFORMATION | wt::PROCESS_SYNCHRONIZE,
                0,
                pid,
            )
        };
        if handle.is_null() {
            let error = io::Error::last_os_error();
            if is_missing_process_error(error.raw_os_error()) {
                return Ok(Witness::Gone);
            }
            return Err(error);
        }
        Ok(Witness::Handle(Arc::new(ScopedWindowsHandle(handle))))
    }

    fn collect(
        port: &ScopedWindowsHandle,
        stop: &AtomicBool,
        sender: &Sender<Collection>,
    ) -> io::Result<()> {
        // Weak entries only deduplicate already retained objects. They neither
        // pin handles themselves nor prevent a later capture after Gone.
        let mut retained: BTreeMap<u32, Weak<ScopedWindowsHandle>> = BTreeMap::new();
        while !stop.load(Ordering::Acquire) {
            let mut message = 0;
            let mut key = 0;
            let mut value = std::ptr::null_mut();
            // SAFETY: the private port remains owned by this worker. This is
            // its only consumer; each successful output is a Job notification.
            if unsafe {
                wio::GetQueuedCompletionStatus(
                    port.0,
                    &mut message,
                    &mut key,
                    &mut value,
                    Self::COLLECTOR_IDLE_MS,
                )
            } == 0
            {
                let error = io::Error::last_os_error();
                if value.is_null() && error.raw_os_error() == Some(wf::WAIT_TIMEOUT as i32) {
                    continue;
                }
                return Err(error);
            }
            if key == Self::STOP_KEY && stop.load(Ordering::Acquire) {
                return Ok(());
            }
            if key != Self::KEY {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "unexpected private Job completion key",
                ));
            }
            if message != ws::JOB_OBJECT_MSG_NEW_PROCESS {
                continue;
            }
            let pid = u32::try_from(value as usize).map_err(|_| {
                io::Error::new(io::ErrorKind::InvalidData, "Job PID outside DWORD range")
            })?;
            if retained.get(&pid).and_then(Weak::upgrade).is_some() {
                continue;
            }
            // Capture before handing the event to the owner. Ordinary delayed
            // owner polls must not turn whole cohorts into late PID opens.
            let captured = Self::open_candidate(pid).ok();
            if let Some(Witness::Handle(handle)) = &captured {
                retained.insert(pid, Arc::downgrade(handle));
            }
            if sender
                .send(Collection::Candidate(Candidate { pid, captured }))
                .is_err()
            {
                return Ok(());
            }
        }
        Ok(())
    }

    pub(super) fn retain_assigned_root(&mut self, pid: u32, process: wf::HANDLE) -> io::Result<()> {
        let mut duplicated = std::ptr::null_mut();
        // SAFETY: process is the original suspended Child already assigned
        // successfully to our Job. Duplicate only query/sync, without inheritance.
        let current = unsafe { wt::GetCurrentProcess() };
        if unsafe {
            wf::DuplicateHandle(
                current,
                process,
                current,
                &mut duplicated,
                wt::PROCESS_QUERY_LIMITED_INFORMATION | wt::PROCESS_SYNCHRONIZE,
                0,
                0,
            )
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
        self.coverage.record(
            pid,
            Witness::Handle(Arc::new(ScopedWindowsHandle(duplicated))),
        )
    }

    fn accounting(
        &mut self,
        job: wf::HANDLE,
    ) -> io::Result<wjob::JOBOBJECT_BASIC_ACCOUNTING_INFORMATION> {
        // SAFETY: this output is POD; the live Job and exact information class,
        // structure and byte length match the API contract.
        let mut info: wjob::JOBOBJECT_BASIC_ACCOUNTING_INFORMATION = unsafe { std::mem::zeroed() };
        if unsafe {
            wjob::QueryInformationJobObject(
                job,
                wjob::JobObjectBasicAccountingInformation,
                &mut info as *mut _ as *mut _,
                std::mem::size_of_val(&info) as u32,
                std::ptr::null_mut(),
            )
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
        self.coverage.observe_total(info.TotalProcesses)?;
        Ok(info)
    }

    fn process_signaled(handle: &Arc<ScopedWindowsHandle>) -> io::Result<bool> {
        // SAFETY: the owned handle has SYNCHRONIZE access and remains pinned.
        match unsafe { wt::WaitForSingleObject(handle.0, 0) } {
            wf::WAIT_OBJECT_0 => Ok(true),
            wf::WAIT_TIMEOUT => Ok(false),
            wf::WAIT_FAILED => Err(io::Error::last_os_error()),
            _ => Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "unexpected process wait result",
            )),
        }
    }

    // NEW_PROCESS or the full Job PID list provides historical membership.
    // Only the owner validates current membership against its private Job.
    fn capture_candidate(
        &mut self,
        job: wf::HANDLE,
        candidate: &mut Candidate,
    ) -> io::Result<bool> {
        if self.coverage.pinned(candidate.pid) {
            return Ok(true);
        }
        if candidate.captured.is_none() {
            candidate.captured = Some(Self::open_candidate(candidate.pid)?);
        }
        match candidate.captured.as_ref().expect("captured above") {
            Witness::Gone => {
                self.coverage.record(candidate.pid, Witness::Gone)?;
                Ok(true)
            }
            Witness::Handle(handle) => {
                let mut in_job = 0;
                // SAFETY: both handles remain live. Query failure is distinct
                // from a successful FALSE membership result. No queue is locked.
                if unsafe { wjob::IsProcessInJob(handle.0, job, &mut in_job) } == 0 {
                    return Err(io::Error::last_os_error());
                }
                self.coverage.record_opened(
                    candidate.pid,
                    Arc::clone(handle),
                    in_job != 0,
                    Self::process_signaled,
                )
            }
            Witness::GoneThenHandle(_) => unreachable!("open returns a single witness"),
        }
    }

    fn receive_candidates(&mut self) -> io::Result<()> {
        if self.collector_failed {
            return Err(io::Error::other("Windows Job collector failed"));
        }
        for _ in 0..Self::PORTION {
            match self.collected.try_recv() {
                Ok(Collection::Candidate(candidate)) => self.pending.push_back(candidate),
                Ok(Collection::Failed(error)) => {
                    self.collector_failed = true;
                    return Err(error);
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    self.collector_failed = true;
                    return Err(io::Error::other(
                        "Windows Job collector stopped unexpectedly",
                    ));
                }
            }
        }
        Ok(())
    }

    fn list_candidates(&mut self, job: wf::HANDLE) -> io::Result<()> {
        use std::mem::{offset_of, size_of};
        let bytes = self
            .pid_buffer
            .len()
            .checked_mul(size_of::<usize>())
            .ok_or_else(|| io::Error::other("Job PID buffer size overflow"))?;
        let size = u32::try_from(bytes)
            .map_err(|_| io::Error::other("Job PID buffer exceeds the Windows DWORD format"))?;
        let mut returned = 0;
        // SAFETY: usize storage has the structure's ULONG_PTR alignment and
        // remains writable for the entire supplied byte length.
        let ok = unsafe {
            wjob::QueryInformationJobObject(
                job,
                wjob::JobObjectBasicProcessIdList,
                self.pid_buffer.as_mut_ptr().cast(),
                size,
                &mut returned,
            )
        };
        if ok == 0 {
            let error = io::Error::last_os_error();
            if error.raw_os_error() != Some(wf::ERROR_MORE_DATA as i32) {
                return Err(error);
            }
            let next = self
                .pid_buffer
                .len()
                .checked_mul(2)
                .ok_or_else(|| io::Error::other("Job PID buffer growth overflow"))?;
            self.pid_buffer.resize(next, 0);
            return Ok(()); // Continue with the larger buffer on the next poll.
        }
        // SAFETY: successful output contains a complete header in aligned storage.
        let info = unsafe {
            &*self
                .pid_buffer
                .as_ptr()
                .cast::<wjob::JOBOBJECT_BASIC_PROCESS_ID_LIST>()
        };
        let offset = offset_of!(wjob::JOBOBJECT_BASIC_PROCESS_ID_LIST, ProcessIdList);
        let count = info.NumberOfProcessIdsInList as usize;
        if returned as usize > bytes
            || (returned as usize) < offset
            || count > (bytes - offset) / size_of::<usize>()
            || offset + count * size_of::<usize>() > returned as usize
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "invalid Job PID-list bounds",
            ));
        }
        if info.NumberOfAssignedProcesses != info.NumberOfProcessIdsInList {
            let next = self
                .pid_buffer
                .len()
                .checked_mul(2)
                .ok_or_else(|| io::Error::other("Job PID buffer growth overflow"))?;
            self.pid_buffer.resize(next, 0);
            return Ok(());
        }
        // SAFETY: output count and offset were checked against returned bytes
        // and allocation bounds, so every listed PID has a membership witness.
        let pids = unsafe {
            std::slice::from_raw_parts(
                self.pid_buffer
                    .as_ptr()
                    .cast::<u8>()
                    .add(offset)
                    .cast::<usize>(),
                count,
            )
        };
        for &pid in pids {
            let pid = u32::try_from(pid).map_err(|_| {
                io::Error::new(io::ErrorKind::InvalidData, "Job PID outside DWORD range")
            })?;
            if !self.coverage.pinned(pid) {
                self.pending.push_back(Candidate {
                    pid,
                    captured: None,
                });
            }
        }
        Ok(())
    }

    pub(super) fn observe(&mut self, job: wf::HANDLE) -> io::Result<()> {
        self.receive_candidates()?;
        let accounting = self.accounting(job)?;
        if self.pending.is_empty() && !self.coverage.complete(accounting.TotalProcesses) {
            self.list_candidates(job)?;
        }
        for _ in 0..Self::PORTION.min(self.pending.len()) {
            let Some(mut candidate) = self.pending.pop_front() else {
                break;
            };
            // Keep the exact captured handle across transient membership/wait
            // failures and foreign unsignaled candidates; never reopen its PID.
            match self.capture_candidate(job, &mut candidate) {
                Ok(true) => {}
                Ok(false) => self.pending.push_back(candidate),
                Err(error) => {
                    self.pending.push_front(candidate);
                    return Err(error);
                }
            }
        }
        Ok(())
    }

    pub(super) fn ready(&mut self, job: wf::HANDLE) -> io::Result<bool> {
        self.observe(job)?;
        let accounting = self.accounting(job)?; // Final accounting follows collection.
        self.coverage.ready(
            accounting.ActiveProcesses,
            accounting.TotalProcesses,
            Self::process_signaled,
        )
    }
}

#[cfg(windows)]
impl Drop for Census {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        // SAFETY: our owned port remains live through this call. A failed post
        // is covered by the worker's finite idle poll, without joining or
        // allocating a second process-cleanup wait window.
        unsafe {
            wio::PostQueuedCompletionStatus(self.port.0, 0, Self::STOP_KEY, std::ptr::null());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{is_missing_process_error, Coverage, Witness};

    #[test]
    fn only_the_exact_missing_process_error_can_retire_an_unopened_pid() {
        assert!(is_missing_process_error(Some(87)));
        for error in [None, Some(0), Some(5), Some(6), Some(1168)] {
            assert!(!is_missing_process_error(error));
        }
    }

    #[test]
    fn an_opened_member_is_pinned_until_its_signal() {
        let mut coverage = Coverage::new();
        coverage
            .record_opened(4, false, true, |ready| Ok(*ready))
            .unwrap();
        assert!(coverage.contains(4));
        assert!(!coverage.ready(0, 1, |ready| Ok(*ready)).unwrap());
    }

    #[test]
    fn a_signaled_historical_pid_is_retired_even_after_membership_disappears() {
        let mut coverage = Coverage::new();
        coverage
            .record_opened(4, true, false, |ready| Ok(*ready))
            .unwrap();
        assert!(coverage.ready(0, 1, |ready| Ok(*ready)).unwrap());
    }

    #[test]
    fn a_foreign_unsignaled_object_does_not_credit_the_historical_pid() {
        let mut coverage = Coverage::new();
        coverage
            .record_opened(4, false, false, |ready| Ok(*ready))
            .unwrap();
        assert!(!coverage.contains(4));
        assert!(!coverage.ready(0, 1, |ready| Ok(*ready)).unwrap());
    }

    #[test]
    fn a_failed_wait_during_historical_capture_never_credits_a_pid() {
        let mut coverage = Coverage::new();
        assert!(coverage
            .record_opened(4, (), false, |_| Err(std::io::Error::other("wait failed")))
            .is_err());
        assert!(!coverage.contains(4));
    }

    #[test]
    fn empty_accounting_does_not_release_an_unsignaled_process() {
        let mut coverage = Coverage::new();
        coverage.record(4, Witness::Handle(false)).unwrap();
        assert!(!coverage.ready(0, 1, |ready| Ok(*ready)).unwrap());
    }

    #[test]
    fn every_retained_process_must_signal() {
        let mut coverage = Coverage::new();
        coverage.record(4, Witness::Handle(true)).unwrap();
        coverage.record(8, Witness::Handle(false)).unwrap();
        assert!(!coverage.ready(0, 2, |ready| Ok(*ready)).unwrap());
        *coverage.members.get_mut(&8).unwrap() = Witness::Handle(true);
        assert!(coverage.ready(0, 2, |ready| Ok(*ready)).unwrap());
    }

    #[test]
    fn active_processes_prevent_release_even_with_full_coverage() {
        let mut coverage = Coverage::new();
        coverage.record(4, Witness::Handle(true)).unwrap();
        assert!(!coverage.ready(1, 1, |ready| Ok(*ready)).unwrap());
    }

    #[test]
    fn a_missing_notification_or_failed_admission_keeps_the_gap() {
        let mut coverage = Coverage::new();
        coverage.record(4, Witness::Handle(true)).unwrap();
        assert!(!coverage.ready(0, 2, |ready| Ok(*ready)).unwrap());
        assert!(coverage.contains(4));
        assert!(!coverage.contains(8));
    }

    #[test]
    fn gone_witness_completes_a_short_lived_member_without_a_handle() {
        let mut coverage = Coverage::new();
        coverage.record(4, Witness::Handle(true)).unwrap();
        coverage.record(8, Witness::Gone).unwrap();
        assert!(coverage.ready(0, 2, |ready| Ok(*ready)).unwrap());
    }

    #[test]
    fn duplicate_notifications_never_replace_an_unsignaled_handle() {
        let mut coverage = Coverage::new();
        coverage.record(4, Witness::Handle(false)).unwrap();
        coverage.record(4, Witness::Gone).unwrap();
        coverage.record(4, Witness::Handle(true)).unwrap();
        assert!(!coverage.ready(0, 1, |ready| Ok(*ready)).unwrap());
    }

    #[test]
    fn reused_gone_pid_does_not_earn_a_second_lifetime_credit() {
        let mut coverage = Coverage::new();
        coverage.record(4, Witness::Gone).unwrap();
        coverage.record(4, Witness::Handle(true)).unwrap();
        assert!(!coverage.ready(0, 2, |ready| Ok(*ready)).unwrap());
        assert!(coverage.ready(0, 1, |ready| Ok(*ready)).is_err());
    }

    #[test]
    fn gone_then_attested_owned_handle_proves_exactly_two_lifetimes() {
        let mut coverage = Coverage::new();
        coverage.record(4, Witness::Gone).unwrap();
        coverage
            .record_opened(4, false, true, |ready| Ok(*ready))
            .unwrap();
        assert!(coverage.pinned(4));
        assert!(!coverage.ready(0, 2, |ready| Ok(*ready)).unwrap());
        // Duplicates cannot replace the unsignaled pinned object or earn a third credit.
        coverage
            .record_opened(4, true, true, |ready| Ok(*ready))
            .unwrap();
        assert!(!coverage.ready(0, 2, |ready| Ok(*ready)).unwrap());
        *coverage.members.get_mut(&4).unwrap() = Witness::GoneThenHandle(true);
        assert!(coverage.ready(0, 2, |ready| Ok(*ready)).unwrap());
        assert!(!coverage.ready(0, 3, |ready| Ok(*ready)).unwrap());
    }

    #[test]
    fn gone_then_foreign_signaled_handle_cannot_prove_another_membership() {
        let mut coverage = Coverage::new();
        coverage.record(4, Witness::Gone).unwrap();
        coverage
            .record_opened(4, true, false, |ready| Ok(*ready))
            .unwrap();
        assert!(!coverage.pinned(4));
        assert!(!coverage.ready(0, 2, |ready| Ok(*ready)).unwrap());
    }

    #[cfg(windows)]
    #[test]
    fn a_stopped_collector_cannot_release_even_an_empty_job() {
        use super::{wjob, Census, Ordering, ScopedWindowsHandle};
        use std::time::{Duration, Instant};
        // SAFETY: create an anonymous private Job, owned by the fixture.
        let job = unsafe { wjob::CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
        assert!(!job.is_null());
        let job = ScopedWindowsHandle(job);
        let mut census = Census::associate(job.0).unwrap();
        census.stop.store(true, Ordering::Release);
        let deadline = Instant::now() + Duration::from_secs(5);
        // The receiver's disconnected state observes actual worker exit, not
        // a scheduling guess or a synthetic successful collection.
        while census.receive_candidates().is_ok() {
            assert!(Instant::now() < deadline, "collector did not stop");
            std::thread::yield_now();
        }
        assert!(census.ready(job.0).is_err());
    }

    #[test]
    fn backward_accounting_cannot_heal_into_a_release() {
        let mut coverage = Coverage::<bool>::new();
        assert!(!coverage.ready(1, 2, |ready| Ok(*ready)).unwrap());
        assert!(coverage.ready(0, 1, |ready| Ok(*ready)).is_err());
        coverage.record(4, Witness::Gone).unwrap();
        coverage.record(8, Witness::Gone).unwrap();
        assert!(coverage.ready(0, 2, |ready| Ok(*ready)).is_err());
    }

    #[test]
    fn process_wait_error_is_not_a_signal() {
        let mut coverage = Coverage::new();
        coverage.record(4, Witness::Handle(())).unwrap();
        assert!(coverage
            .ready(0, 1, |_| Err(std::io::Error::other("wait failed")))
            .is_err());
        assert!(!coverage.ready(0, 1, |_| Ok(false)).unwrap());
    }

    #[test]
    fn zero_pid_never_enters_the_coverage() {
        let mut coverage = Coverage::<bool>::new();
        assert!(coverage.record(0, Witness::Gone).is_err());
        assert!(!coverage.complete(1));
    }
}
