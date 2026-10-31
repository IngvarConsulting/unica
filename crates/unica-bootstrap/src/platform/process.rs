use std::path::Path;
use std::process::Command;

use unica_artifact_delivery::{BootstrapError, Result};

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
    use super::job_object::{assign_to_job, spawn_supervised};

    let mut runtime = spawn_supervised(runtime_command(entrypoint, args, handoff), assign_to_job)
        .map_err(|failure| start_failure(entrypoint, failure))?;
    let status = runtime.wait().map_err(|error| {
        BootstrapError::new(format!("failed to wait for Unica runtime: {error}"))
    })?;
    Ok(status.code().unwrap_or(1))
}

/// Каждый отказ после `spawn` уже завершил приостановленного ребёнка: сироты
/// нет, и сообщение говорит об этом прямо.
#[cfg(windows)]
fn start_failure(entrypoint: &Path, failure: super::job_object::StartFailure) -> BootstrapError {
    use super::job_object::StartFailure;

    let entrypoint = entrypoint.display();
    match failure {
        StartFailure::Job(error) => BootstrapError::new(format!(
            "failed to create Windows Job Object for Unica runtime {entrypoint}: {error}"
        )),
        StartFailure::Spawn(error) => BootstrapError::new(format!(
            "failed to start Unica runtime {entrypoint}: {error}"
        )),
        StartFailure::Assign(error) => BootstrapError::new(format!(
            "failed to supervise Unica runtime {entrypoint} with a Windows Job Object: {error}; \
             the suspended runtime was terminated before it ran"
        )),
        StartFailure::Resume(error) => BootstrapError::new(format!(
            "failed to resume Unica runtime {entrypoint} inside its Windows Job Object: {error}; \
             the runtime was terminated"
        )),
    }
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

pub(crate) fn runtime_command(
    entrypoint: &Path,
    args: &[String],
    handoff: &RuntimeHandoff<'_>,
) -> Command {
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

    #[cfg(windows)]
    #[test]
    fn the_resumed_runtime_exit_code_reaches_the_host() {
        // Приостановленный ребёнок должен быть отпущен: иначе загрузчик ждал
        // бы его вечно, а код выхода рантайма до хоста бы не дошёл.
        let args = vec!["/D".to_string(), "/C".to_string(), "exit 7".to_string()];

        let code = launch_runtime(Path::new("cmd.exe"), &args, &silent_windows_handoff()).unwrap();

        assert_eq!(code, 7);
    }

    /// Надзор за рантаймом на Windows: группа загрузчика и её отказы.
    #[cfg(windows)]
    mod job {
        use crate::platform::job_object::{
            assign_to_job, spawn_supervised, RuntimeJob, StartFailure, SupervisedRuntime,
        };
        use std::io;
        use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
        use std::path::PathBuf;
        use std::process::{Child, Command};
        use std::time::Duration;
        use windows_sys::Win32::Foundation::HANDLE;
        use windows_sys::Win32::Foundation::{STILL_ACTIVE, WAIT_OBJECT_0};
        use windows_sys::Win32::System::JobObjects::IsProcessInJob;
        use windows_sys::Win32::System::Threading::{
            GetExitCodeProcess, OpenProcess, WaitForSingleObject,
            PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE,
        };

        fn cmd(script: &str) -> Command {
            let mut command = Command::new("cmd.exe");
            command.args(["/D", "/C", script]);
            command
        }

        fn scratch_dir(name: &str) -> PathBuf {
            let dir = std::env::temp_dir().join(format!(
                "unica-bootstrap-job-{name}-{}",
                uuid::Uuid::new_v4()
            ));
            std::fs::create_dir_all(&dir).unwrap();
            dir
        }

        /// Дескриптор процесса, независимый от `Child`: им тест видит судьбу
        /// ребёнка и после того, как `Child` уничтожен внутри проверяемого кода.
        fn watch(pid: u32) -> OwnedHandle {
            // SAFETY: наследование выключено; идентификатор принадлежит живому ребёнку.
            let handle = unsafe {
                OpenProcess(
                    PROCESS_SYNCHRONIZE | PROCESS_QUERY_LIMITED_INFORMATION,
                    0,
                    pid,
                )
            };
            assert!(!handle.is_null(), "{}", io::Error::last_os_error());
            // SAFETY: дескриптор только что открыт и принадлежит тесту.
            unsafe { OwnedHandle::from_raw_handle(handle) }
        }

        fn exited_within(process: &OwnedHandle, timeout: Duration) -> bool {
            let millis = u32::try_from(timeout.as_millis()).unwrap();
            // SAFETY: дескриптор открыт с правом SYNCHRONIZE.
            let waited = unsafe { WaitForSingleObject(process.as_raw_handle() as HANDLE, millis) };
            if waited != WAIT_OBJECT_0 {
                return false;
            }
            let mut code = 0u32;
            // SAFETY: дескриптор открыт с правом PROCESS_QUERY_LIMITED_INFORMATION.
            let queried =
                unsafe { GetExitCodeProcess(process.as_raw_handle() as HANDLE, &mut code) } != 0;
            queried && code != STILL_ACTIVE as u32
        }

        #[test]
        fn closing_the_bootstrap_job_terminates_the_runtime() {
            // Загрузчик умирает как угодно — вместе с ним закрывается последний
            // дескриптор группы. Рантайм не должен его пережить.
            let mut runtime =
                spawn_supervised(cmd("ping -n 60 127.0.0.1 >NUL"), assign_to_job).unwrap();
            let process = watch(runtime.child.id());

            let mut in_job = 0;
            // SAFETY: оба дескриптора живы на время вызова.
            let queried = unsafe {
                IsProcessInJob(
                    runtime.child.as_raw_handle() as HANDLE,
                    runtime._job.raw(),
                    &mut in_job,
                )
            };
            assert_ne!(queried, 0, "{}", io::Error::last_os_error());
            assert_ne!(in_job, 0, "the runtime must run inside the bootstrap job");
            assert!(
                runtime.child.try_wait().unwrap().is_none(),
                "the runtime must be running before the job closes"
            );

            let SupervisedRuntime {
                child: _child,
                _job: job,
            } = runtime;
            drop(job);

            assert!(
                exited_within(&process, Duration::from_secs(10)),
                "the runtime outlived the bootstrap job"
            );
        }

        #[test]
        fn a_failed_assignment_terminates_the_suspended_runtime() {
            // Отказ включения в группу не должен оставить ни сироты, ни следа
            // работы: ребёнок не получил ни одного кванта.
            fn refuse(_job: &RuntimeJob, child: &Child) -> io::Result<()> {
                WATCHED.with(|slot| *slot.borrow_mut() = Some(watch(child.id())));
                Err(io::Error::other("injected assignment failure"))
            }
            thread_local! {
                static WATCHED: std::cell::RefCell<Option<OwnedHandle>> =
                    const { std::cell::RefCell::new(None) };
            }

            let dir = scratch_dir("refused");
            let marker = dir.join("ran");
            let script = format!("echo ran> \"{}\"", marker.display());

            let failure = match spawn_supervised(cmd(&script), refuse) {
                Err(failure) => failure,
                Ok(_) => panic!("an injected assignment failure must be reported"),
            };

            assert!(
                matches!(&failure, StartFailure::Assign(error) if error.to_string() == "injected assignment failure"),
                "{failure:?}"
            );
            let process = WATCHED
                .with(|slot| slot.borrow_mut().take())
                .expect("the seam must have seen the child");
            assert!(
                exited_within(&process, Duration::ZERO),
                "the suspended runtime must be gone when the error is returned"
            );
            assert!(!marker.exists(), "the refused runtime must never have run");
            let _ = std::fs::remove_dir_all(dir);
        }
    }
}
