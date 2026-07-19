#[cfg(windows)]
const WINDOWS_MAIN_STACK_SIZE: usize = 8 * 1024 * 1024;

#[cfg(windows)]
fn main() {
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
fn main() {
    run();
}

fn run() {
    let args = std::env::args().collect::<Vec<_>>();
    if args.iter().any(|arg| arg == "--workspace-service") {
        if let Err(error) = unica_coder::interfaces::workspace_service::run_from_args(&args) {
            eprintln!("{error}");
            std::process::exit(1);
        }
        return;
    }
    if args.iter().any(|arg| arg == "--runtime-job-worker") {
        if let Err(error) = unica_coder::interfaces::runtime_job_worker::run_from_args(&args) {
            eprintln!("{error}");
            std::process::exit(1);
        }
        return;
    }

    if std::env::args().any(|arg| arg == "--help" || arg == "-h") {
        println!("unica {}", env!("CARGO_PKG_VERSION"));
        println!("stdio MCP orchestrator for Unica workflows");
        println!("Supported MCP methods: initialize, tools/list, tools/call");
        return;
    }

    unica_coder::interfaces::mcp::run_stdio();
}
