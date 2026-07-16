fn main() {
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
