use std::env;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;

use unica_bootstrap::{
    launch_runtime, provider_state_root, runtime_cache_root, verify_installed_plugin_metadata,
    verify_mcp_runtime, AttemptLog, HostTarget, HttpDownloader, Result, RuntimeHandoff,
    RuntimeInstaller, RuntimeManifest, UnfinishedAttempt,
};

const VERSION: &str = env!("CARGO_PKG_VERSION");

fn main() -> ExitCode {
    unica_bootstrap::run_platform_main(run_main)
}

fn run_main() -> ExitCode {
    match run(env::args().skip(1).collect()) {
        Ok(code) => ExitCode::from(normalize_exit_code(code)),
        Err(error) => {
            // Причина, место и лечение в потоке ошибок, различимый номер — в
            // коде выхода: убитая сессия текста не увидит, а код увидит хост.
            eprintln!("unica-bootstrap: {}", error.diagnosis());
            ExitCode::from(error.exit_code())
        }
    }
}

fn run(args: Vec<String>) -> Result<i32> {
    if args.as_slice() == ["--version"] {
        println!("unica-bootstrap {VERSION}");
        return Ok(0);
    }
    let (command, plugin_root) = parse_command(&args)?;
    let provider_state_root = provider_state_root()?;
    match command {
        Command::Run => {
            let artifact_cache = runtime_cache_root()?;
            // Читаем до установки: сколько получила убитая попытка, видно лишь
            // пока её недокачка на диске, а удачная установка её забирает.
            let killed = KilledStartups::read(&artifact_cache);
            let installed = install_runtime(&plugin_root)?;
            // Убитый прошлый запуск своего провода не имел. Провод есть у
            // рантайма — он и расскажет, если рассказывать есть о чём.
            let notice = killed.notice();
            launch_runtime(
                &installed.entrypoint,
                &[],
                &RuntimeHandoff {
                    provider_state_root: &provider_state_root,
                    artifact_cache: &artifact_cache,
                    runtime_manifest: &plugin_root.join("runtime-manifest.json"),
                    startup_notice: notice.as_deref(),
                },
            )
        }
        Command::Verify => {
            install_and_verify_runtime(&plugin_root, &provider_state_root)?;
            Ok(0)
        }
        Command::Prefetch => prefetch_runtime(&plugin_root),
    }
}

/// Довезти всё, что цели понадобится, и рассказать об этом сборке образа.
///
/// Ленивая доставка выигрывает старт и проигрывает закрытый контур: в образе
/// сети уже нет. Отсюда и вывод — не украшение, а единственное, что читает
/// человек, разбирая упавшую сборку.
fn prefetch_runtime(plugin_root: &Path) -> Result<i32> {
    let manifest_path = plugin_root.join("runtime-manifest.json");
    let manifest = RuntimeManifest::load(&manifest_path)?;
    if manifest.development {
        // Молчаливый успех соврал бы: образ уехал бы без инструментов, и
        // выяснилось бы это уже там, где сети нет.
        return Err(unica_bootstrap::BootstrapError::of(
            unica_bootstrap::Failure::Configuration,
            format!(
                "{} is a development manifest: it publishes no artifacts, \
                 so there is nothing to prefetch",
                manifest_path.display()
            ),
        ));
    }
    let host = HostTarget::current()?;
    let cache_root = runtime_cache_root()?;
    let installer = RuntimeInstaller::new(cache_root, VERSION, Arc::new(HttpDownloader::default()));
    let delivered = installer.prefetch(&manifest, host, &ReportProgress)?;
    for item in &delivered {
        eprintln!(
            "unica-bootstrap: {} {} {} at {}",
            item.artifact,
            item.version,
            if item.downloaded {
                "delivered"
            } else {
                "cached"
            },
            item.root.display()
        );
    }
    eprintln!(
        "unica-bootstrap: prefetched {} artifacts for {}",
        delivered.len(),
        host.as_str()
    );
    Ok(0)
}

/// Ход загрузки в журнал сборки: молчащий шаг на сотню мегабайт неотличим от
/// повисшего.
struct ReportProgress;

impl unica_bootstrap::DownloadObserver for ReportProgress {
    fn transferred(&self, received: u64, total: Option<u64>) {
        match total {
            Some(total) if total > 0 => eprintln!(
                "unica-bootstrap: {received} of {total} bytes ({}%)",
                received.saturating_mul(100) / total
            ),
            _ => eprintln!("unica-bootstrap: {received} bytes"),
        }
    }
}

/// Что осталось от запусков, которых убили снаружи.
///
/// Читается до установки — и это не порядок ради порядка: полученный объём
/// живёт в недокачке на диске, а удачная установка её забирает. Прочитать
/// после значило бы сообщить «получено 0 байт» о попытке, привёзшей полсотни
/// мегабайт.
///
/// Отказ чтения журнала запуск не отменяет: рассказ о прошлой беде не стоит
/// того, чтобы стать новой.
struct KilledStartups {
    log: AttemptLog,
    found: Vec<UnfinishedAttempt>,
}

impl KilledStartups {
    fn read(artifact_cache: &Path) -> Self {
        let log = AttemptLog::in_cache(artifact_cache);
        let found = log.unfinished().unwrap_or_default();
        Self { log, found }
    }

    /// Рассказ для вызывающего. Отмечает попытки рассказанными: второй раз о
    /// том же сообщать некому и незачем.
    fn notice(&self) -> Option<String> {
        let notice = unica_bootstrap::diagnose(&self.found);
        if notice.is_some() {
            let _ = self.log.report(&self.found);
        }
        notice
    }
}

fn install_runtime(plugin_root: &Path) -> Result<unica_bootstrap::RuntimeInstallation> {
    let manifest = RuntimeManifest::load(&plugin_root.join("runtime-manifest.json"))?;
    let host = HostTarget::current()?;
    let cache_root = runtime_cache_root()?;
    let installer = RuntimeInstaller::new(cache_root, VERSION, Arc::new(HttpDownloader::default()));
    installer.ensure(&manifest, host)
}

fn install_and_verify_runtime(plugin_root: &Path, provider_state_root: &Path) -> Result<()> {
    verify_installed_skill_package(plugin_root)?;
    let installed = install_runtime(plugin_root)?;
    verify_mcp_runtime(
        &installed.entrypoint,
        &installed.root,
        provider_state_root,
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
    Prefetch,
}

fn parse_command(args: &[String]) -> Result<(Command, PathBuf)> {
    if args.len() != 3 || args[1] != "--plugin-root" {
        return Err(unica_bootstrap::BootstrapError::new(
            "usage: unica-bootstrap <run|verify|prefetch> --plugin-root <path>",
        ));
    }
    let command = match args[0].as_str() {
        "run" => Command::Run,
        "verify" => Command::Verify,
        "prefetch" => Command::Prefetch,
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
