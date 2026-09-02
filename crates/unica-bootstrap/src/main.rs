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

/// The one prompt-visible skill this check names.
///
/// The expected skill set is whatever the package ships, so adding or removing
/// a skill never touches this crate. The anchor is what tells a Unica skill
/// tree from any directory that happens to hold `SKILL.md` files: `code-search`
/// fronts `unica.search`, the entry point of the public surface, and dropping
/// it is a product decision, not a routine surface change, so it is the one
/// name a release is allowed to fail on by design.
const ANCHOR_SKILL: &str = "code-search";

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

/// The installed package exposes the prompt-visible skills it was built from.
///
/// The expected set is derived from the package itself rather than written
/// here: every directory under `skills/` is a skill the hosts will show, so
/// each has to be complete, the set has to be non-empty, and it has to carry
/// the [`ANCHOR_SKILL`].
fn verify_installed_skill_package(plugin_root: &Path) -> Result<()> {
    verify_installed_plugin_metadata(plugin_root, VERSION)?;

    let visible = prompt_visible_skills(&plugin_root.join("skills"))?;
    // The anchor would reject an empty package on its own. This branch is
    // here for the diagnosis: a packager that shipped nothing is a different
    // fault from one that shipped a tree missing this skill, and the release
    // log is what someone reads to tell them apart.
    if visible.is_empty() {
        return Err(unica_bootstrap::BootstrapError::new(
            "installed Unica plugin exposes no prompt-visible skills",
        ));
    }
    if !visible.contains(ANCHOR_SKILL) {
        return Err(unica_bootstrap::BootstrapError::new(format!(
            "installed prompt-visible skill is missing: {ANCHOR_SKILL}"
        )));
    }
    Ok(())
}

/// Every skill directory of the package, each proven complete by its `SKILL.md`.
fn prompt_visible_skills(skills_root: &Path) -> Result<std::collections::BTreeSet<String>> {
    let mut visible = std::collections::BTreeSet::new();
    for entry in std::fs::read_dir(skills_root)? {
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
    Ok(visible)
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

    use std::path::PathBuf;

    /// A packaged plugin root: both host manifests of this release and the
    /// named prompt-visible skills.
    struct PackageFixture {
        root: PathBuf,
    }

    impl PackageFixture {
        fn new(name: &str, skills: &[&str]) -> Self {
            let root = std::env::temp_dir()
                .join(format!("unica-skill-package-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&root);
            let manifests = [
                (
                    ".codex-plugin",
                    serde_json::json!({
                        "name": "unica",
                        "version": VERSION,
                        "skills": "./skills/",
                        "mcpServers": "./.mcp.json",
                    }),
                ),
                (
                    ".claude-plugin",
                    serde_json::json!({"name": "unica", "version": VERSION}),
                ),
            ];
            for (dir, body) in manifests {
                let manifest_dir = root.join(dir);
                std::fs::create_dir_all(&manifest_dir).unwrap();
                std::fs::write(
                    manifest_dir.join("plugin.json"),
                    serde_json::to_vec(&body).unwrap(),
                )
                .unwrap();
            }
            std::fs::create_dir_all(root.join("skills")).unwrap();
            let fixture = Self { root };
            for skill in skills {
                fixture.add_skill(skill);
            }
            fixture
        }

        fn add_skill(&self, name: &str) {
            let skill_dir = self.skill_dir(name);
            std::fs::write(skill_dir.join("SKILL.md"), format!("# {name}\n")).unwrap();
        }

        /// A directory under `skills/` that carries no `SKILL.md`.
        fn skill_dir(&self, name: &str) -> PathBuf {
            let skill_dir = self.root.join("skills").join(name);
            std::fs::create_dir_all(&skill_dir).unwrap();
            skill_dir
        }
    }

    impl Drop for PackageFixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    #[test]
    fn source_plugin_passes_the_installed_skill_package_check() {
        let plugin_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../plugins/unica");
        verify_installed_skill_package(&plugin_root).unwrap();
    }

    #[test]
    fn a_package_is_accepted_whatever_skills_it_ships_beside_the_anchor() {
        // Removing a skill changes the package, not this check: the expected
        // set is whatever the package ships, and only the anchor is named.
        let fixture = PackageFixture::new("any-skills", &[ANCHOR_SKILL, "api-design"]);
        verify_installed_skill_package(&fixture.root).unwrap();
    }

    #[test]
    fn a_package_without_the_anchor_skill_is_rejected() {
        let fixture = PackageFixture::new("no-anchor", &["api-design", "platform-help"]);
        let error = verify_installed_skill_package(&fixture.root).unwrap_err();
        assert!(error.to_string().contains(ANCHOR_SKILL), "{error}");
    }

    #[test]
    fn a_package_without_any_prompt_visible_skill_is_rejected() {
        let fixture = PackageFixture::new("no-skills", &[]);
        let error = verify_installed_skill_package(&fixture.root).unwrap_err();
        assert!(
            error.to_string().contains("no prompt-visible skills"),
            "{error}"
        );
    }

    #[test]
    fn a_skill_directory_without_skill_md_is_rejected() {
        let fixture = PackageFixture::new("incomplete-skill", &[ANCHOR_SKILL]);
        fixture.skill_dir("broken");
        let error = verify_installed_skill_package(&fixture.root).unwrap_err();
        let message = error.to_string();
        assert!(
            message.contains("incomplete") && message.contains("broken"),
            "{error}"
        );
    }
}
