#[cfg(windows)]
const WINDOWS_MAIN_STACK_SIZE: usize = 8 * 1024 * 1024;

#[cfg(windows)]
pub fn run_platform_main(run: fn()) {
    let main_thread = std::thread::Builder::new()
        .name("unica-main".to_string())
        .stack_size(WINDOWS_MAIN_STACK_SIZE)
        .spawn(run)
        .unwrap_or_else(|error| panic!("failed to start Unica main thread: {error}"));
    if let Err(panic) = main_thread.join() {
        std::panic::resume_unwind(panic);
    }
}

#[cfg(not(windows))]
pub fn run_platform_main(run: fn()) {
    run();
}
