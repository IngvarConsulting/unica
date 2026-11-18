//! A cancel of a process that answers it itself ([`CancelPolicy::Interrupt`]).
//!
//! The helper counts interrupts (Unix: SIGTERM; Windows: CTRL_BREAK) and
//! exits after the given number of them, printing the count.

use super::{CancelPolicy, CancelStep, JobPolicy, ManagedChild, ManagedCommand};
use std::ffi::OsString;
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread;
use std::time::{Duration, Instant};
use unica_work_control::cancellation::CancellationToken;

const MODE_ENV: &str = "UNICA_INTERRUPT_HELPER_EXIT_AFTER";
const READY_ENV: &str = "UNICA_INTERRUPT_HELPER_READY";

static INTERRUPTS: AtomicUsize = AtomicUsize::new(0);

#[cfg(unix)]
extern "C" fn count_interrupt(_signal: libc::c_int) {
    INTERRUPTS.fetch_add(1, Ordering::SeqCst);
}

#[cfg(unix)]
fn count_interrupts() {
    // SAFETY: a zeroed sigaction with a plain handler is valid.
    let mut action: libc::sigaction = unsafe { std::mem::zeroed() };
    action.sa_sigaction = count_interrupt as *const () as usize;
    // SAFETY: the mask belongs to `action`; the handler only touches an atomic.
    let installed = unsafe {
        libc::sigemptyset(&mut action.sa_mask);
        libc::sigaction(libc::SIGTERM, &action, std::ptr::null_mut())
    };
    assert_eq!(installed, 0, "{}", std::io::Error::last_os_error());
}

#[cfg(windows)]
unsafe extern "system" fn count_interrupt(kind: u32) -> windows_sys::Win32::Foundation::BOOL {
    if kind == windows_sys::Win32::System::Console::CTRL_BREAK_EVENT {
        INTERRUPTS.fetch_add(1, Ordering::SeqCst);
        1
    } else {
        0
    }
}

#[cfg(windows)]
fn count_interrupts() {
    // SAFETY: the handler is a plain function that lives as long as the process.
    let installed = unsafe {
        windows_sys::Win32::System::Console::SetConsoleCtrlHandler(Some(count_interrupt), 1)
    };
    assert_ne!(installed, 0, "{}", std::io::Error::last_os_error());
}

#[test]
#[ignore = "helper process: spawned by the interrupt tests; not a standalone test"]
fn interrupt_test_helper() {
    let exit_after: usize = std::env::var(MODE_ENV).unwrap().parse().unwrap();
    count_interrupts();
    std::fs::write(
        std::env::var_os(READY_ENV).unwrap(),
        std::process::id().to_string(),
    )
    .unwrap();
    let started = Instant::now();
    while started.elapsed() < Duration::from_secs(60) {
        let count = INTERRUPTS.load(Ordering::SeqCst);
        if exit_after != 0 && count >= exit_after {
            println!("interrupts={count}");
            return;
        }
        thread::sleep(Duration::from_millis(10));
    }
}

/// Whether an interrupt can reach a child here: on Windows only through a
/// console of the test process.
fn interrupts_reach_children() -> bool {
    #[cfg(windows)]
    {
        super::windows_console::attached()
    }
    #[cfg(not(windows))]
    {
        true
    }
}

/// The helper, started as the pinned runner is, and ready for interrupts.
fn start(exit_after: usize, ready: &Path, cancellation: CancellationToken) -> ManagedChild {
    let child = ManagedChild::spawn_staged_in(
        ManagedCommand {
            program: std::env::current_exe().unwrap(),
            args: vec![
                "--exact".into(),
                "platform::process::interrupt_tests::interrupt_test_helper".into(),
                "--ignored".into(),
                "--nocapture".into(),
            ],
            cwd: std::env::current_dir().unwrap(),
            env: vec![
                (
                    OsString::from(MODE_ENV),
                    OsString::from(exit_after.to_string()),
                ),
                (OsString::from(READY_ENV), ready.as_os_str().to_os_string()),
            ],
            env_remove: Vec::new(),
            capture_limits: None,
            timeout: Some(Duration::from_secs(60)),
            cancellation,
        },
        JobPolicy::AllowsBreakaway,
        CancelPolicy::Interrupt,
    )
    .unwrap();
    let started = Instant::now();
    while !ready.exists() {
        assert!(
            started.elapsed() < Duration::from_secs(10),
            "helper did not start"
        );
        thread::sleep(Duration::from_millis(10));
    }
    child
}

fn wait_until(what: &str, condition: impl Fn() -> bool) {
    let started = Instant::now();
    while !condition() {
        assert!(started.elapsed() < Duration::from_secs(10), "{what}");
        thread::sleep(Duration::from_millis(10));
    }
}

/// The first cancel interrupts the process and waits for its answer: the run
/// is cancelled and carries what the process said.
/// rule: INV.WIRE.COOPERATIVE-CANCELLATION
#[test]
fn an_interrupted_process_answers_the_cancel_itself() {
    if !interrupts_reach_children() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let cancellation = CancellationToken::new();
    let mut child = start(1, &dir.path().join("ready"), cancellation.clone());
    cancellation.cancel();
    let output = child.wait_for_output().unwrap();
    assert!(output.cancelled);
    assert!(output.stdout.contains("interrupts=1"), "{output:?}");
    assert!(!cancellation.awaits_interrupted_work());
}

/// While the interrupted process works on, its token shows that the cancel
/// waits. A repeated cancel interrupts it again, not before the gap.
/// rule: INV.WIRE.COOPERATIVE-CANCELLATION
#[test]
fn a_repeated_cancel_interrupts_again_after_the_gap() {
    if !interrupts_reach_children() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let cancellation = CancellationToken::new();
    let mut child = start(2, &dir.path().join("ready"), cancellation.clone());
    let canceller = {
        let cancellation = cancellation.clone();
        thread::spawn(move || {
            cancellation.cancel();
            let first = Instant::now();
            wait_until("the cancel waits", || {
                cancellation.awaits_interrupted_work()
            });
            cancellation.cancel();
            first
        })
    };
    let output = child.wait_for_output().unwrap();
    let first = canceller.join().unwrap();
    assert!(first.elapsed() >= super::INTERRUPT_GAP);
    assert!(output.cancelled);
    assert!(output.stdout.contains("interrupts=2"), "{output:?}");
    assert!(!cancellation.awaits_interrupted_work());
}

/// A hard cancel (the daemon stops) ends an interrupted process at once.
/// rule: INV.WIRE.COOPERATIVE-CANCELLATION
#[test]
fn a_hard_cancel_ends_an_interrupted_process_at_once() {
    if !interrupts_reach_children() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let cancellation = CancellationToken::new();
    let mut child = start(0, &dir.path().join("ready"), cancellation.clone());
    let canceller = {
        let cancellation = cancellation.clone();
        thread::spawn(move || {
            cancellation.cancel();
            wait_until("the cancel waits", || {
                cancellation.awaits_interrupted_work()
            });
            cancellation.cancel_now();
        })
    };
    let started = Instant::now();
    let output = child.wait_for_output().unwrap();
    canceller.join().unwrap();
    assert!(started.elapsed() < Duration::from_secs(10));
    assert!(output.cancelled);
    assert!(!output.stdout.contains("interrupts="), "{output:?}");
    assert!(!cancellation.awaits_interrupted_work());
}

/// Without a repeated cancel the process is interrupted again after
/// [`super::SECOND_INTERRUPT_AFTER`], and [`super::KILL_AFTER_INTERRUPTS`]
/// after that without an answer its tree ends.
/// rule: INV.WIRE.COOPERATIVE-CANCELLATION
#[test]
fn unanswered_interrupts_are_repeated_and_then_end_the_tree() {
    if !interrupts_reach_children() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let cancellation = CancellationToken::new();
    let mut child = start(0, &dir.path().join("ready"), cancellation.clone());
    cancellation.cancel();
    let first = Instant::now();
    assert_eq!(child.cancel_step(first), CancelStep::Wait);
    assert!(cancellation.awaits_interrupted_work());
    let second = first + super::SECOND_INTERRUPT_AFTER;
    assert_eq!(
        child.cancel_step(second - Duration::from_millis(1)),
        CancelStep::Wait
    );
    assert!(child.interrupts.as_ref().unwrap().second.is_none());
    assert_eq!(child.cancel_step(second), CancelStep::Wait);
    assert_eq!(child.interrupts.as_ref().unwrap().second, Some(second));
    let kill = second + super::KILL_AFTER_INTERRUPTS;
    assert_eq!(
        child.cancel_step(kill - Duration::from_millis(1)),
        CancelStep::Wait
    );
    assert_eq!(child.cancel_step(kill), CancelStep::Kill);
    child.end_interrupt_wait();
    assert!(!cancellation.awaits_interrupted_work());
    child.terminate().unwrap();
}

/// A process that cannot be interrupted any more, or is not interrupted by
/// its policy, is ended by a cancel as before.
#[test]
fn a_cancel_without_an_interrupt_ends_the_tree_as_before() {
    let dir = tempfile::tempdir().unwrap();
    let cancellation = CancellationToken::new();
    let mut child = start(0, &dir.path().join("ready"), cancellation.clone());
    child.interrupts = None;
    cancellation.cancel();
    assert_eq!(child.cancel_step(Instant::now()), CancelStep::Terminate);
    let output = child.wait_for_output().unwrap();
    assert!(output.cancelled);
    assert!(!cancellation.awaits_interrupted_work());
}
