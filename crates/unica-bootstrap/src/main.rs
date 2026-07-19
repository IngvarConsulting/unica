use std::env;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;

use unica_bootstrap::{
    launch_runtime, HostTarget, HttpDownloader, Result, RuntimeInstaller, RuntimeManifest,
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
    let manifest = RuntimeManifest::load(&plugin_root.join("runtime-manifest.json"))?;
    let host = HostTarget::current()?;
    let cache_root = runtime_cache_root()?;
    let installer = RuntimeInstaller::new(cache_root, VERSION, Arc::new(HttpDownloader::default()));
    let installed = installer.ensure(&manifest, host)?;

    match command {
        "run" => launch_runtime(&installed.entrypoint, &[]),
        "verify" => {
            eprintln!(
                "verified Unica runtime {} at {}",
                VERSION,
                installed.root.display()
            );
            Ok(0)
        }
        _ => unreachable!("parse_command validates command"),
    }
}

fn parse_command(args: &[String]) -> Result<(&str, PathBuf)> {
    if args.len() != 3 || args[1] != "--plugin-root" {
        return Err(unica_bootstrap::BootstrapError::new(
            "usage: unica-bootstrap <run|verify> --plugin-root <path>",
        ));
    }
    let command = args[0].as_str();
    if !matches!(command, "run" | "verify") {
        return Err(unica_bootstrap::BootstrapError::new(format!(
            "unknown bootstrap command: {command}"
        )));
    }
    Ok((command, Path::new(&args[2]).to_path_buf()))
}

fn runtime_cache_root() -> Result<PathBuf> {
    if let Some(value) = env::var_os("UNICA_RUNTIME_CACHE_DIR") {
        return Ok(PathBuf::from(value));
    }
    if let Some(value) = env::var_os("CODEX_HOME") {
        return Ok(PathBuf::from(value).join("unica").join("runtimes"));
    }
    let home = env::var_os("HOME")
        .or_else(|| env::var_os("USERPROFILE"))
        .ok_or_else(|| {
            unica_bootstrap::BootstrapError::new(
                "CODEX_HOME, HOME, or USERPROFILE is required for the runtime cache",
            )
        })?;
    Ok(PathBuf::from(home)
        .join(".codex")
        .join("unica")
        .join("runtimes"))
}

fn normalize_exit_code(code: i32) -> u8 {
    if (0..=255).contains(&code) {
        code as u8
    } else {
        1
    }
}
