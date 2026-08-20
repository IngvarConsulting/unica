use std::path::Path;
use std::process::Command;

use crate::error::{BootstrapError, Result};

/// Что загрузчик передаёт рантайму при запуске.
///
/// Собрано в одно место потому, что растёт: сперва каталог состояния, потом
/// кеш артефактов, теперь рассказ о прошлой попытке. Каждое из них — то, что
/// знает только тот, кто ставил, и не знает тот, кого запускают.
pub struct RuntimeHandoff<'a> {
    pub provider_state_root: &'a Path,
    /// Где лежат движки: рядом с ядром их больше нет.
    pub artifact_cache: &'a Path,
    /// Манифест поставки. Рантайм своим корнем считает каталог установки в
    /// кеше — там лежит манифест инструментов, упакованный в архив ядра, — а
    /// релизный манифест остался в каталоге плагина, и знает про него только
    /// загрузчик.
    pub runtime_manifest: &'a Path,
    /// О чём рассказать вызывающему: прошлый запуск убили, и своего провода у
    /// него не было. `None` — рассказывать нечего, и переменная не появляется.
    pub startup_notice: Option<&'a str>,
}

#[cfg(unix)]
pub fn launch_runtime(
    entrypoint: &Path,
    args: &[String],
    handoff: &RuntimeHandoff<'_>,
) -> Result<i32> {
    use std::os::unix::process::CommandExt;

    let error = runtime_command(entrypoint, args, handoff).exec();
    Err(BootstrapError::new(format!(
        "failed to exec Unica runtime {}: {error}",
        entrypoint.display()
    )))
}

#[cfg(windows)]
pub fn launch_runtime(
    entrypoint: &Path,
    args: &[String],
    handoff: &RuntimeHandoff<'_>,
) -> Result<i32> {
    use std::mem::size_of;
    use std::os::windows::io::AsRawHandle;
    use std::ptr;
    use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
    use windows_sys::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
        SetInformationJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
        JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    };

    let mut child = runtime_command(entrypoint, args, handoff)
        .spawn()
        .map_err(|error| {
            BootstrapError::new(format!(
                "failed to start Unica runtime {}: {error}",
                entrypoint.display()
            ))
        })?;

    let job = unsafe { CreateJobObjectW(ptr::null(), ptr::null()) };
    if job.is_null() {
        let _ = child.kill();
        let _ = child.wait();
        return Err(BootstrapError::new(
            "failed to create Windows Job Object for Unica runtime",
        ));
    }
    let mut information: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { std::mem::zeroed() };
    information.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
    let configured = unsafe {
        SetInformationJobObject(
            job,
            JobObjectExtendedLimitInformation,
            &information as *const _ as *const _,
            size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
        )
    };
    let assigned = unsafe { AssignProcessToJobObject(job, child.as_raw_handle() as HANDLE) };
    if configured == 0 || assigned == 0 {
        unsafe { CloseHandle(job) };
        let _ = child.kill();
        let _ = child.wait();
        return Err(BootstrapError::new(
            "failed to supervise Unica runtime with a Windows Job Object",
        ));
    }

    let status = child
        .wait()
        .map_err(|error| BootstrapError::new(format!("failed to wait for Unica runtime: {error}")));
    unsafe { CloseHandle(job) };
    let status = status?;
    Ok(status.code().unwrap_or(1))
}

#[cfg(not(any(unix, windows)))]
pub fn launch_runtime(
    entrypoint: &Path,
    _args: &[String],
    _handoff: &RuntimeHandoff<'_>,
) -> Result<i32> {
    Err(BootstrapError::new(format!(
        "runtime launch is unsupported on this platform: {}",
        entrypoint.display()
    )))
}

fn runtime_command(entrypoint: &Path, args: &[String], handoff: &RuntimeHandoff<'_>) -> Command {
    let mut command = Command::new(entrypoint);
    command
        .args(args)
        .env("UNICA_PROVIDER_STATE_DIR", handoff.provider_state_root)
        // Движки больше не лежат рядом с ядром: рантайм ищет их в кеше
        // артефактов, и адрес кеша знает только тот, кто туда ставил.
        .env("UNICA_ARTIFACT_CACHE", handoff.artifact_cache)
        .env("UNICA_RUNTIME_MANIFEST", handoff.runtime_manifest);
    if let Some(notice) = handoff.startup_notice {
        command.env("UNICA_STARTUP_NOTICE", notice);
    } else {
        command.env_remove("UNICA_STARTUP_NOTICE");
    }
    command
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    fn silent_handoff() -> RuntimeHandoff<'static> {
        RuntimeHandoff {
            provider_state_root: Path::new("/private/provider-state"),
            artifact_cache: Path::new("/private/artifact-cache"),
            runtime_manifest: Path::new("/plugins/unica/runtime-manifest.json"),
            startup_notice: None,
        }
    }

    /// Тот же провод, но путями этой платформы: на Windows unix-литерал ушёл бы
    /// ребёнку как есть и сравнивать его было бы не с чем.
    #[cfg(windows)]
    fn silent_windows_handoff() -> RuntimeHandoff<'static> {
        RuntimeHandoff {
            provider_state_root: Path::new(r"C:\private\provider-state"),
            artifact_cache: Path::new(r"C:\private\artifact-cache"),
            runtime_manifest: Path::new(r"C:\plugins\unica\runtime-manifest.json"),
            startup_notice: None,
        }
    }

    #[cfg(unix)]
    #[test]
    fn runtime_command_hands_the_release_manifest_to_the_child() {
        // Рантайм находит своим корнем каталог установки в кеше: там лежит
        // манифест инструментов, упакованный в архив ядра. Релизного манифеста
        // там нет — он остался в каталоге плагина, и знает про него только
        // загрузчик. Без этого доставка движка молча не начинается.
        let args = vec![
            "-c".to_string(),
            "printf %s \"$UNICA_RUNTIME_MANIFEST\"".to_string(),
        ];
        let handoff = RuntimeHandoff {
            provider_state_root: Path::new("/private/provider-state"),
            artifact_cache: Path::new("/private/artifact-cache"),
            runtime_manifest: Path::new("/plugins/unica/runtime-manifest.json"),
            startup_notice: None,
        };

        let output = runtime_command(Path::new("/bin/sh"), &args, &handoff)
            .output()
            .unwrap();

        assert_eq!(output.stdout, b"/plugins/unica/runtime-manifest.json");
    }

    #[cfg(unix)]
    #[test]
    fn runtime_command_hands_the_startup_notice_to_the_child() {
        // Убитая прошлая попытка своего провода не имела. Провод есть у
        // рантайма, и рассказать о ней он может только тем, что ему передали.
        let args = vec![
            "-c".to_string(),
            "printf %s \"$UNICA_STARTUP_NOTICE\"".to_string(),
        ];
        let handoff = RuntimeHandoff {
            provider_state_root: Path::new("/private/provider-state"),
            artifact_cache: Path::new("/private/artifact-cache"),
            runtime_manifest: Path::new("/plugins/unica/runtime-manifest.json"),
            startup_notice: Some("a Unica startup was killed while downloading unica 0.13.0"),
        };

        let output = runtime_command(Path::new("/bin/sh"), &args, &handoff)
            .output()
            .unwrap();

        assert!(output.status.success());
        assert_eq!(
            output.stdout,
            b"a Unica startup was killed while downloading unica 0.13.0"
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_startup_with_nothing_to_report_leaves_the_variable_unset() {
        // Пустое значение и отсутствие переменной — разные вещи: рантайм,
        // запущенный без загрузчика, должен вести себя ровно как раньше.
        let args = vec![
            "-c".to_string(),
            "printf %s \"${UNICA_STARTUP_NOTICE-unset}\"".to_string(),
        ];
        let handoff = RuntimeHandoff {
            provider_state_root: Path::new("/private/provider-state"),
            artifact_cache: Path::new("/private/artifact-cache"),
            runtime_manifest: Path::new("/plugins/unica/runtime-manifest.json"),
            startup_notice: None,
        };

        let mut command = runtime_command(Path::new("/bin/sh"), &args, &handoff);
        assert!(
            command
                .get_envs()
                .any(|(name, value)| { name == "UNICA_STARTUP_NOTICE" && value.is_none() }),
            "the child command must explicitly remove an inherited notice"
        );
        let output = command.output().unwrap();

        assert_eq!(output.stdout, b"unset");
    }

    #[cfg(unix)]
    #[test]
    fn runtime_command_passes_provider_state_root_to_child() {
        let args = vec![
            "-c".to_string(),
            "printf %s \"$UNICA_PROVIDER_STATE_DIR\"".to_string(),
        ];
        let output = runtime_command(Path::new("/bin/sh"), &args, &silent_handoff())
            .output()
            .unwrap();

        assert!(output.status.success());
        assert_eq!(output.stdout, b"/private/provider-state");
    }

    #[cfg(unix)]
    #[test]
    fn runtime_command_passes_the_artifact_cache_to_child() {
        let args = vec![
            "-c".to_string(),
            "printf %s \"$UNICA_ARTIFACT_CACHE\"".to_string(),
        ];
        let output = runtime_command(Path::new("/bin/sh"), &args, &silent_handoff())
            .output()
            .unwrap();

        assert!(output.status.success());
        assert_eq!(output.stdout, b"/private/artifact-cache");
    }

    #[cfg(windows)]
    #[test]
    fn runtime_command_passes_provider_state_root_to_child() {
        let args = vec![
            "/D".to_string(),
            "/C".to_string(),
            "echo(%UNICA_PROVIDER_STATE_DIR%".to_string(),
        ];
        let output = runtime_command(Path::new("cmd.exe"), &args, &silent_windows_handoff())
            .output()
            .unwrap();

        assert!(output.status.success());
        assert_eq!(output.stdout, b"C:\\private\\provider-state\r\n");
    }
}
