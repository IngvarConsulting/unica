use std::process::ExitCode;

#[cfg(windows)]
const WINDOWS_MAIN_STACK_SIZE: usize = 8 * 1024 * 1024;

#[cfg(windows)]
pub fn run_platform_main(run: fn() -> ExitCode) -> ExitCode {
    let main_thread = std::thread::Builder::new()
        .name("unica-bootstrap-main".to_string())
        .stack_size(WINDOWS_MAIN_STACK_SIZE)
        .spawn(run)
        .unwrap_or_else(|error| panic!("failed to start Unica bootstrap main thread: {error}"));
    match main_thread.join() {
        Ok(code) => code,
        Err(panic) => std::panic::resume_unwind(panic),
    }
}

#[cfg(not(windows))]
pub fn run_platform_main(run: fn() -> ExitCode) -> ExitCode {
    run()
}
