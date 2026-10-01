// Process-wide memory high-water mark. No per-tool attribution is possible.

#[cfg(any(target_os = "linux", target_os = "macos"))]
pub(crate) fn process_peak_rss_bytes() -> Option<u64> {
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
pub(crate) fn process_peak_rss_bytes() -> Option<u64> {
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
pub(crate) fn process_peak_rss_bytes() -> Option<u64> {
    None
}
