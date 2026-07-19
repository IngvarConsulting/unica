use std::env;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;

use unica_bootstrap::{
    launch_runtime, verify_mcp_runtime, CommandRunner, CommandSpec, HostTarget, HttpDownloader,
    MigrationEngine, Result, RuntimeInstaller, RuntimeManifest, SystemCommandRunner,
};

const VERSION: &str = env!("CARGO_PKG_VERSION");

#[cfg(windows)]
const WINDOWS_MAIN_STACK_SIZE: usize = 8 * 1024 * 1024;

#[cfg(windows)]
fn main() -> ExitCode {
    let main_thread = std::thread::Builder::new()
        .name("unica-bootstrap-main".to_string())
        .stack_size(WINDOWS_MAIN_STACK_SIZE)
        .spawn(run_main)
        .unwrap_or_else(|error| panic!("failed to start Unica bootstrap main thread: {error}"));
    match main_thread.join() {
        Ok(code) => code,
        Err(panic) => std::panic::resume_unwind(panic),
    }
}

#[cfg(not(windows))]
fn main() -> ExitCode {
    run_main()
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
    if matches!(command, Command::Migrate | Command::MigratePreflight) {
        let engine = MigrationEngine::new(codex_home_root()?, SystemCommandRunner);
        let plan = engine.preflight()?;
        if command == Command::MigratePreflight {
            println!("{}", serde_json::to_string_pretty(&plan)?);
        } else {
            let report = engine.apply(plan, install_and_verify_migration)?;
            println!("{}", serde_json::to_string_pretty(&report)?);
        }
        return Ok(0);
    }

    match command {
        Command::Run => {
            let installed = install_runtime(&plugin_root)?;
            launch_runtime(&installed.entrypoint, &[])
        }
        Command::Verify => {
            install_and_verify_runtime(&plugin_root)?;
            Ok(0)
        }
        Command::Migrate | Command::MigratePreflight => {
            unreachable!("migration commands return before runtime installation")
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

fn install_and_verify_migration(plugin_root: &Path) -> Result<()> {
    install_and_verify_runtime(plugin_root)?;
    verify_fresh_prompt_input(plugin_root)?;
    eprintln!(
        "verified Unica {} prompt-visible skills through fresh Codex prompt-input",
        VERSION
    );
    Ok(())
}

fn verify_installed_skill_package(plugin_root: &Path) -> Result<()> {
    let metadata_path = plugin_root.join(".codex-plugin").join("plugin.json");
    let metadata: serde_json::Value = serde_json::from_slice(&std::fs::read(&metadata_path)?)?;
    if metadata.get("name").and_then(serde_json::Value::as_str) != Some("unica")
        || metadata.get("version").and_then(serde_json::Value::as_str) != Some(VERSION)
        || metadata.get("skills").and_then(serde_json::Value::as_str) != Some("./skills/")
    {
        return Err(unica_bootstrap::BootstrapError::new(format!(
            "installed Unica plugin metadata does not expose version {VERSION} skills: {}",
            metadata_path.display()
        )));
    }

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

fn verify_fresh_prompt_input(plugin_root: &Path) -> Result<()> {
    let codex_home = codex_home_root()?;
    let output = SystemCommandRunner.run(&CommandSpec::codex(
        &codex_home,
        &["debug", "prompt-input", "Проверь доступность Unica skills"],
    ))?;
    let proof: serde_json::Value = serde_json::from_str(&output).map_err(|error| {
        unica_bootstrap::BootstrapError::new(format!(
            "invalid codex debug prompt-input JSON: {error}"
        ))
    })?;
    for required in [
        "unica:code-search",
        "unica:platform-help",
        "unica:release-support",
        "unica:v8-runner",
    ] {
        if !json_contains_text(&proof, required) {
            return Err(unica_bootstrap::BootstrapError::new(format!(
                "fresh Codex prompt-input does not expose installed skill {required}"
            )));
        }
    }
    let expected_root = format!("plugins/cache/unica/unica/{VERSION}/skills");
    if !json_contains_normalized_path(&proof, &expected_root) {
        return Err(unica_bootstrap::BootstrapError::new(format!(
            "fresh Codex prompt-input does not reference installed Unica skill root: {}",
            plugin_root.join("skills").display()
        )));
    }
    Ok(())
}

fn json_contains_text(value: &serde_json::Value, needle: &str) -> bool {
    match value {
        serde_json::Value::String(text) => text.contains(needle),
        serde_json::Value::Array(items) => {
            items.iter().any(|item| json_contains_text(item, needle))
        }
        serde_json::Value::Object(fields) => fields
            .values()
            .any(|field| json_contains_text(field, needle)),
        _ => false,
    }
}

fn json_contains_normalized_path(value: &serde_json::Value, needle: &str) -> bool {
    match value {
        serde_json::Value::String(text) => text
            .replace('\\', "/")
            .to_ascii_lowercase()
            .contains(needle),
        serde_json::Value::Array(items) => items
            .iter()
            .any(|item| json_contains_normalized_path(item, needle)),
        serde_json::Value::Object(fields) => fields
            .values()
            .any(|field| json_contains_normalized_path(field, needle)),
        _ => false,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_plugin_exposes_required_prompt_visible_skills() {
        let plugin_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../plugins/unica");
        verify_installed_skill_package(&plugin_root).unwrap();
    }

    #[test]
    fn prompt_proof_searches_nested_skill_names_and_normalized_paths() {
        let proof = serde_json::json!({
            "content": [{
                "text": r"- unica:code-search (file: C:\Codex\plugins\cache\unica\unica\0.7.3\skills\code-search\SKILL.md)"
            }]
        });

        assert!(json_contains_text(&proof, "unica:code-search"));
        assert!(json_contains_normalized_path(
            &proof,
            "c:/codex/plugins/cache/unica/unica/0.7.3/skills"
        ));
    }
}
