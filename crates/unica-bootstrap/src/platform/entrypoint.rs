use std::process::ExitCode;

#[cfg(windows)]
// resource-limit: bootstrap.windows-main-stack-bytes
// class: format
// unit: bytes
// scope: process
// value: 8388608
// resource: Размер стека главного потока bootstrap на Windows.
// status: unproven
// on-reach: переполнение стека аварийно завершает процесс
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
