fn main() {
    unica_coder::run_platform_main(run);
}

fn run() {
    let args = std::env::args().collect::<Vec<_>>();
    // A subcommand is the first argument that is not a flag. MCP is started
    // without one; an unknown subcommand is refused at once instead of
    // starting MCP, whose silence a client would only notice by its timeout.
    // `unica mcp` names the default explicitly; tooling starts it that way.
    match args.get(1).map(String::as_str) {
        Some("lsp") => std::process::exit(unica_coder::interfaces::lsp::run_stdio()),
        Some("mcp") => {}
        Some(command) if !command.starts_with('-') => {
            eprintln!("unica: unknown command `{command}`; supported commands: mcp, lsp");
            std::process::exit(2);
        }
        _ => {}
    }
    if args.iter().any(|arg| arg == "--daemon") {
        if let Err(error) = unica_coder::interfaces::daemon::run_from_args(&args) {
            eprintln!("{error}");
            std::process::exit(1);
        }
        return;
    }
    if args.iter().any(|arg| arg == "--print-core-identity") {
        if let Err(error) = unica_coder::interfaces::daemon::print_core_identity() {
            eprintln!("{error}");
            std::process::exit(1);
        }
        return;
    }
    if args.iter().any(|arg| arg == "--capacity-report") {
        if let Err(error) = unica_coder::interfaces::daemon::print_capacity_report_from_args(&args)
        {
            eprintln!("{error}");
            std::process::exit(1);
        }
        return;
    }
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
        println!("Agent MCP over stdio: unica [mcp]");
        println!("Editor LSP over stdio: unica lsp");
        println!("Local capacity observations: --capacity-report");
        return;
    }

    unica_coder::interfaces::mcp::run_stdio();
}
