// Process-wide memory high-water mark. No per-tool attribution is possible.

#[cfg(any(target_os = "linux", target_os = "macos"))]
pub fn process_peak_rss_bytes() -> Option<u64> {
    let mut usage = std::mem::MaybeUninit::<libc::rusage>::zeroed();
    if unsafe { libc::getrusage(libc::RUSAGE_SELF, usage.as_mut_ptr()) } != 0 {
        return None;
    }
    let value = unsafe { usage.assume_init() }.ru_maxrss;
    let value = u64::try_from(value).ok()?;
    #[cfg(target_os = "linux")]
    {
        value.checked_mul(1024)
    }
    #[cfg(target_os = "macos")]
    {
        Some(value)
    }
}

#[cfg(windows)]
pub fn process_peak_rss_bytes() -> Option<u64> {
    use windows_sys::Win32::System::ProcessStatus::{
        GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS,
    };
    use windows_sys::Win32::System::Threading::GetCurrentProcess;
    let mut counters = std::mem::MaybeUninit::<PROCESS_MEMORY_COUNTERS>::zeroed();
    let success = unsafe {
        GetProcessMemoryInfo(
            GetCurrentProcess(),
            counters.as_mut_ptr(),
            std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32,
        )
    };
    if success == 0 {
        return None;
    }
    Some(unsafe { counters.assume_init() }.PeakWorkingSetSize as u64)
}

#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
pub fn process_peak_rss_bytes() -> Option<u64> {
    None
}

/// When the OS says this process started, computed as v8-runner computes it
/// for a holder of its live agent (`--holder-started-at`): FILETIME of
/// `GetProcessTimes` on Windows, clock ticks after boot (`/proc/self/stat`,
/// field 22) plus `btime` on Linux, `proc_pidinfo` on macOS. `None` — the OS
/// does not name it.
#[cfg(windows)]
pub fn process_started_at() -> Option<std::time::SystemTime> {
    use windows_sys::Win32::Foundation::FILETIME;
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, GetProcessTimes};
    /// Seconds between 1601-01-01 and 1970-01-01.
    const FILETIME_TO_UNIX: u64 = 11_644_473_600;
    let zero = FILETIME {
        dwLowDateTime: 0,
        dwHighDateTime: 0,
    };
    let (mut created, mut exited, mut kernel, mut user) = (zero, zero, zero, zero);
    // SAFETY: the pseudo handle of the current process is always valid; the
    // pointers refer to live locals.
    let timed = unsafe {
        GetProcessTimes(
            GetCurrentProcess(),
            &mut created,
            &mut exited,
            &mut kernel,
            &mut user,
        )
    } != 0;
    if !timed {
        return None;
    }
    let ticks = (u64::from(created.dwHighDateTime) << 32) | u64::from(created.dwLowDateTime);
    let since_unix = (ticks / 10).checked_sub(FILETIME_TO_UNIX * 1_000_000)?;
    std::time::UNIX_EPOCH.checked_add(std::time::Duration::from_micros(since_unix))
}

#[cfg(target_os = "linux")]
pub fn process_started_at() -> Option<std::time::SystemTime> {
    let stat = std::fs::read_to_string("/proc/self/stat").ok()?;
    // Fields after the command name, which may hold spaces and parentheses:
    // the state is field 3, so the start time (field 22) is the 20th here.
    let after_name = &stat[stat.rfind(')')? + 1..];
    let started: u64 = after_name.split_whitespace().nth(19)?.parse().ok()?;
    let proc_stat = std::fs::read_to_string("/proc/stat").ok()?;
    let boot: u64 = proc_stat
        .lines()
        .find_map(|line| line.strip_prefix("btime "))?
        .trim()
        .parse()
        .ok()?;
    // SAFETY: `sysconf` takes a plain value and cannot harm memory.
    let ticks = u64::try_from(unsafe { libc::sysconf(libc::_SC_CLK_TCK) })
        .ok()
        .filter(|ticks| *ticks > 0)?;
    let micros = boot
        .checked_mul(1_000_000)?
        .checked_add(started.checked_mul(1_000_000)? / ticks)?;
    std::time::UNIX_EPOCH.checked_add(std::time::Duration::from_micros(micros))
}

#[cfg(target_os = "macos")]
pub fn process_started_at() -> Option<std::time::SystemTime> {
    let mut info = std::mem::MaybeUninit::<libc::proc_bsdinfo>::zeroed();
    let size = std::mem::size_of::<libc::proc_bsdinfo>() as libc::c_int;
    // SAFETY: the buffer is sized for PROC_PIDTBSDINFO.
    let written = unsafe {
        libc::proc_pidinfo(
            std::process::id() as libc::c_int,
            libc::PROC_PIDTBSDINFO,
            0,
            info.as_mut_ptr().cast(),
            size,
        )
    };
    if written != size {
        return None;
    }
    // SAFETY: the call above filled the structure.
    let info = unsafe { info.assume_init() };
    let micros = info
        .pbi_start_tvsec
        .checked_mul(1_000_000)?
        .checked_add(info.pbi_start_tvusec)?;
    std::time::UNIX_EPOCH.checked_add(std::time::Duration::from_micros(micros))
}

#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
pub fn process_started_at() -> Option<std::time::SystemTime> {
    None
}

#[cfg(test)]
mod tests {
    /// The OS names when this test process started: not later than now and
    /// not before the test run could have begun.
    #[cfg(any(target_os = "linux", target_os = "macos", windows))]
    #[test]
    fn this_process_has_a_start_time_from_the_os() {
        let started = super::process_started_at().expect("the OS names the start");
        let now = std::time::SystemTime::now();
        assert!(started <= now + std::time::Duration::from_secs(1));
        assert!(now.duration_since(started).unwrap() < std::time::Duration::from_secs(24 * 3600));
        // Asked twice, the OS names the same instant: the holder start time is
        // compared with a tolerance of one second.
        assert_eq!(super::process_started_at(), Some(started));
    }
}
