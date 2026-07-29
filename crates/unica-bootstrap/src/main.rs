use std::env;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;

use unica_bootstrap::{
    launch_runtime, runtime_cache_root, verify_installed_plugin_metadata, verify_mcp_runtime,
    HostTarget, HttpDownloader, Result, RuntimeInstaller, RuntimeManifest,
};

const VERSION: &str = env!("CARGO_PKG_VERSION");

fn main() -> ExitCode {
    unica_bootstrap::run_platform_main(run_main)
}

fn run_main() -> ExitCode {
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
    match command {
        Command::Run => {
            let installed = install_runtime(&plugin_root)?;
            launch_runtime(&installed.entrypoint, &[])
        }
        Command::Verify => {
            install_and_verify_runtime(&plugin_root)?;
            Ok(0)
        }
    }
}

fn install_runtime(plugin_root: &Path) -> Result<unica_bootstrap::RuntimeInstallation> {
    let manifest = RuntimeManifest::load(&plugin_root.join("runtime-manifest.json"))?;
    let host = HostTarget::current()?;
    let cache_root = runtime_cache_root()?;
    let installer = RuntimeInstaller::new(cache_root, VERSION, Arc::new(HttpDownloader::default()));
    installer.ensure(&manifest, host)
}

fn install_and_verify_runtime(plugin_root: &Path) -> Result<()> {
    verify_installed_skill_package(plugin_root)?;
    let installed = install_runtime(plugin_root)?;
    verify_mcp_runtime(
        &installed.entrypoint,
        &installed.root,
        Duration::from_secs(20),
    )?;
    eprintln!(
        "verified Unica {} package, runtime, and MCP tools at {}",
        VERSION,
        installed.root.display()
    );
    Ok(())
}

fn verify_installed_skill_package(plugin_root: &Path) -> Result<()> {
    verify_installed_plugin_metadata(plugin_root, VERSION)?;

    let skills_root = plugin_root.join("skills");
    let mut visible = std::collections::BTreeSet::new();
    for entry in std::fs::read_dir(&skills_root)? {
        let entry = entry?;
        if !entry.file_type()?.is_dir() {
            continue;
        }
        let skill_file = entry.path().join("SKILL.md");
        if !skill_file.is_file() {
            return Err(unica_bootstrap::BootstrapError::new(format!(
                "installed prompt-visible skill is incomplete: {}",
                entry.path().display()
            )));
        }
        visible.insert(entry.file_name().to_string_lossy().into_owned());
    }
    for required in [
        "code-search",
        "platform-help",
        "release-support",
        "v8-runner",
    ] {
        if !visible.contains(required) {
            return Err(unica_bootstrap::BootstrapError::new(format!(
                "installed prompt-visible skill is missing: {required}"
            )));
        }
    }
    if visible.is_empty() {
        return Err(unica_bootstrap::BootstrapError::new(
            "installed Unica plugin exposes no prompt-visible skills",
        ));
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Command {
    Run,
    Verify,
}

fn parse_command(args: &[String]) -> Result<(Command, PathBuf)> {
    if args.len() != 3 || args[1] != "--plugin-root" {
        return Err(unica_bootstrap::BootstrapError::new(
            "usage: unica-bootstrap <run|verify> --plugin-root <path>",
        ));
    }
    let command = match args[0].as_str() {
        "run" => Command::Run,
        "verify" => Command::Verify,
        command => {
            return Err(unica_bootstrap::BootstrapError::new(format!(
                "unknown bootstrap command: {command}"
            )))
        }
    };
    Ok((command, Path::new(&args[2]).to_path_buf()))
}

fn normalize_exit_code(code: i32) -> u8 {
    if (0..=255).contains(&code) {
        code as u8
    } else {
        1
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_plugin_exposes_required_prompt_visible_skills() {
        let plugin_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../plugins/unica");
        verify_installed_skill_package(&plugin_root).unwrap();
    }
}
