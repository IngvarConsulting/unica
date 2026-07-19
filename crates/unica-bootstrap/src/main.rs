use std::env;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;

use unica_bootstrap::{
    launch_runtime, verify_mcp_runtime, HostTarget, HttpDownloader, MigrationEngine, Result,
    RuntimeInstaller, RuntimeManifest, SystemCommandRunner,
};

const VERSION: &str = env!("CARGO_PKG_VERSION");

fn main() -> ExitCode {
    match run(env::args().skip(1).collect()) {
        Ok(code) => ExitCode::from(normalize_exit_code(code)),
        Err(error) => {
            eprintln!("unica-bootstrap: {error}");
            ExitCode::from(1)
        }
    }
}

fn run(args: Vec<String>) -> Result<i32> {
    if args.as_slice() == ["--version"] {
        println!("unica-bootstrap {VERSION}");
        return Ok(0);
    }
    let (command, plugin_root) = parse_command(&args)?;
    if matches!(command, Command::Migrate | Command::MigratePreflight) {
        let engine = MigrationEngine::new(codex_home_root()?, SystemCommandRunner);
        let plan = engine.preflight()?;
        if command == Command::MigratePreflight {
            println!("{}", serde_json::to_string_pretty(&plan)?);
        } else {
            let report = engine.apply(plan)?;
            println!("{}", serde_json::to_string_pretty(&report)?);
        }
        return Ok(0);
    }

    let manifest = RuntimeManifest::load(&plugin_root.join("runtime-manifest.json"))?;
    let host = HostTarget::current()?;
    let cache_root = runtime_cache_root()?;
    let installer = RuntimeInstaller::new(cache_root, VERSION, Arc::new(HttpDownloader::default()));
    let installed = installer.ensure(&manifest, host)?;

    match command {
        Command::Run => launch_runtime(&installed.entrypoint, &[]),
        Command::Verify => {
            verify_mcp_runtime(
                &installed.entrypoint,
                &installed.root,
                Duration::from_secs(20),
            )?;
            eprintln!(
                "verified Unica runtime {} and MCP tools at {}",
                VERSION,
                installed.root.display()
            );
            Ok(0)
        }
        Command::Migrate | Command::MigratePreflight => {
            unreachable!("migration commands return before runtime installation")
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Command {
    Run,
    Verify,
    Migrate,
    MigratePreflight,
}

fn parse_command(args: &[String]) -> Result<(Command, PathBuf)> {
    if args.len() != 3 || args[1] != "--plugin-root" {
        return Err(unica_bootstrap::BootstrapError::new(
            "usage: unica-bootstrap <run|verify|migrate|migrate-preflight> --plugin-root <path>",
        ));
    }
    let command = match args[0].as_str() {
        "run" => Command::Run,
        "verify" => Command::Verify,
        "migrate" => Command::Migrate,
        "migrate-preflight" => Command::MigratePreflight,
        command => {
            return Err(unica_bootstrap::BootstrapError::new(format!(
                "unknown bootstrap command: {command}"
            )))
        }
    };
    Ok((command, Path::new(&args[2]).to_path_buf()))
}

fn runtime_cache_root() -> Result<PathBuf> {
    if let Some(value) = env::var_os("UNICA_RUNTIME_CACHE_DIR") {
        return Ok(PathBuf::from(value));
    }
    Ok(codex_home_root()?.join("unica").join("runtimes"))
}

fn codex_home_root() -> Result<PathBuf> {
    if let Some(value) = env::var_os("CODEX_HOME") {
        return Ok(PathBuf::from(value));
    }
    let home = env::var_os("HOME")
        .or_else(|| env::var_os("USERPROFILE"))
        .ok_or_else(|| {
            unica_bootstrap::BootstrapError::new(
                "CODEX_HOME, HOME, or USERPROFILE is required for the runtime cache",
            )
        })?;
    Ok(PathBuf::from(home).join(".codex"))
}

fn normalize_exit_code(code: i32) -> u8 {
    if (0..=255).contains(&code) {
        code as u8
    } else {
        1
    }
}
