mod install;
mod peer;
mod pipe;

use super::daemon_gate::{cutoff_at, deadline_at};
use super::pending_daemon::{check_deadline, PendingDaemonProcess};
use super::process::{
    daemon_arguments, daemon_environment, runtime_command, wait_for_runtime, RuntimeHandoff,
};
use install::{FilePin, InstalledOwner};
use peer::{is_alive, verified_frontend, verified_process};
use pipe::Pipe;
use serde::{Deserialize, Serialize};
use std::ffi::{OsStr, OsString};
use std::io::{self, Write};
use std::os::windows::io::OwnedHandle;
use std::os::windows::process::ExitStatusExt;
use std::path::Path;
use std::process::{Command, ExitStatus};
use std::time::{Duration, Instant};
use unica_artifact_delivery::{BootstrapError, RuntimeManifest};

pub(super) const PIPE_ENV: &str = "UNICA_DAEMON_OWNER_PIPE";
pub(super) const IMAGE_ENV: &str = "UNICA_DAEMON_OWNER_IMAGE";
const OPT_IN_ENV: &str = "UNICA_WINDOWS_DAEMON_OWNER";
const CONTROL_LIMIT: Duration = Duration::from_secs(30);

pub fn daemon_owner_enabled() -> Result<bool, BootstrapError> {
    daemon_owner_opt_in(std::env::var_os(OPT_IN_ENV).as_deref())
}

fn daemon_owner_opt_in(value: Option<&OsStr>) -> Result<bool, BootstrapError> {
    match value {
        None => Ok(false),
        Some(value) if value == OsStr::new("1") => Ok(true),
        Some(_) => Err(BootstrapError::new(format!(
            "{OPT_IN_ENV} accepts 1 or an unset value"
        ))),
    }
}

pub fn launch_runtime_with_daemon_owner(
    plugin: &Path,
    runtime: &RuntimeManifest,
    entrypoint: &Path,
    handoff: &RuntimeHandoff<'_>,
) -> Result<i32, BootstrapError> {
    let installed = InstalledOwner::bind(plugin, handoff.provider_state_root, entrypoint, runtime)
        .map_err(|error| {
            if error.kind() == io::ErrorKind::NotFound {
                BootstrapError::new(
                    "run the approved Unica daemon owner for this installation and state",
                )
            } else {
                BootstrapError::new(format!("failed to bind the Windows daemon owner: {error}"))
            }
        })?;
    let mut command = runtime_command(
        &installed.entrypoint.path,
        &[],
        &RuntimeHandoff {
            provider_state_root: &installed.provider.path,
            runtime_manifest: &installed.manifest.path,
            ..*handoff
        },
    );
    command
        .env(PIPE_ENV, &installed.identity)
        .env(IMAGE_ENV, &installed.bootstrap.path);
    wait_for_runtime(&installed.entrypoint.path, command)
}

pub fn run_daemon_owner(
    plugin: &Path,
    runtime: &RuntimeManifest,
    entrypoint: &Path,
    handoff: &RuntimeHandoff<'_>,
) -> Result<i32, BootstrapError> {
    serve(plugin, runtime, entrypoint, handoff)
        .map(|()| 0)
        .map_err(|error| BootstrapError::new(format!("Windows daemon owner: {error}")))
}

fn serve(
    plugin: &Path,
    runtime: &RuntimeManifest,
    entrypoint: &Path,
    handoff: &RuntimeHandoff<'_>,
) -> io::Result<()> {
    if !handoff.provider_state_root.is_absolute() {
        return Err(invalid("daemon owner state root must be absolute"));
    }
    std::fs::create_dir_all(handoff.provider_state_root)?;
    let installed = InstalledOwner::bind(plugin, handoff.provider_state_root, entrypoint, runtime)?;
    let command = runtime_command(
        &installed.entrypoint.path,
        &[],
        &RuntimeHandoff {
            provider_state_root: &installed.provider.path,
            runtime_manifest: &installed.manifest.path,
            startup_notice: None,
            ..*handoff
        },
    );
    let environment = daemon_environment(&command);
    let pipe = Pipe::listen(&installed.identity)?;
    {
        let mut output = io::stdout().lock();
        writeln!(output, "Unica daemon owner ready")?;
        output.flush()?;
    }
    let mut previous = None;
    loop {
        pipe.accept(None)?;
        if let Ok(frontend) = pipe.client_pid().and_then(|pid| {
            verified_frontend(pid, &installed.entrypoint, &installed.bootstrap)
                .map(|owner| (pid, owner))
        }) {
            let _ = serve_client(&pipe, frontend, &installed, &environment, &mut previous);
        }
        pipe.disconnect();
    }
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
enum Request {
    Start {
        identity: String,
        idle_grace_ms: u64,
        readiness_cutoff: u64,
        cutoff: u64,
    },
    Recover {
        ticket: String,
    },
    Poll,
    Commit,
    Abort {
        cutoff: u64,
    },
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
enum Response {
    Started { pid: u32, ticket: String },
    Status { exited: Option<u32>, ready: bool },
    Committed,
    Aborted,
    Refused(String),
}

#[derive(Clone, Copy)]
enum Resolution {
    Committed,
    Aborted,
}

struct Startup {
    client: u32,
    frontend: OwnedHandle,
    ticket: String,
    resolution: Option<Resolution>,
    unfinished: Option<PendingDaemonProcess>,
}

impl Startup {
    fn finish_cleanup(&mut self, deadline: Instant) -> io::Result<()> {
        if let Some(child) = self.unfinished.as_mut() {
            child.abort(deadline)?;
            self.unfinished.take();
            self.resolution = Some(Resolution::Aborted);
        }
        Ok(())
    }

    fn recover(&self, client: u32, ticket: &str) -> Response {
        if client == self.client && is_alive(&self.frontend) && ticket == self.ticket {
            match self.resolution {
                Some(Resolution::Committed) => return Response::Committed,
                Some(Resolution::Aborted) => return Response::Aborted,
                None => {}
            }
        }
        Response::Refused("daemon handoff result unknown".into())
    }
}

fn serve_client(
    pipe: &Pipe,
    (pid, (frontend, launcher)): (u32, (OwnedHandle, OwnedHandle)),
    installed: &InstalledOwner,
    environment: &[(OsString, OsString)],
    previous: &mut Option<Startup>,
) -> io::Result<()> {
    let caller = Some(&frontend);
    let initial = Instant::now() + CONTROL_LIMIT;
    let (identity, idle_grace_ms, readiness_cutoff, cutoff) =
        match pipe.read::<Request>(Some(initial), caller)? {
            Request::Start {
                identity,
                idle_grace_ms,
                readiness_cutoff,
                cutoff,
            } => (identity, idle_grace_ms, readiness_cutoff, cutoff),
            Request::Recover { ticket } => {
                let response = previous.as_ref().map_or_else(
                    || Response::Refused("daemon handoff result unknown".into()),
                    |startup| startup.recover(pid, &ticket),
                );
                return finish(pipe, &response, initial, &frontend);
            }
            _ => return Err(invalid("expected daemon start or exact handoff recovery")),
        };
    let deadline = deadline_at(cutoff)?.min(initial);
    let readiness_deadline = deadline_at(readiness_cutoff)?.min(deadline);
    if !valid_digest(&identity) || idle_grace_ms == 0 {
        return Err(invalid(
            "invalid daemon identity or idle graceful shutdown duration",
        ));
    }
    check_deadline(readiness_deadline)?;
    if let Some(startup) = previous.as_mut() {
        if let Err(error) = startup.finish_cleanup(readiness_deadline) {
            return finish(
                pipe,
                &Response::Refused(format!(
                    "previous daemon startup cleanup is unconfirmed: {error}"
                )),
                deadline,
                &frontend,
            );
        }
    }
    if !is_alive(&frontend) || !is_alive(&launcher) {
        return Err(io::Error::new(io::ErrorKind::BrokenPipe, "frontend exited"));
    }
    let mut startup = Startup {
        client: pid,
        frontend: frontend.try_clone()?,
        ticket: uuid::Uuid::new_v4().to_string(),
        resolution: None,
        unfinished: None,
    };
    let args = daemon_arguments(
        &installed.provider.path,
        &identity,
        u128::from(idle_grace_ms),
    );
    let mut child = match PendingDaemonProcess::spawn(
        &installed.entrypoint.path,
        &args,
        &installed.provider.path,
        environment.to_vec(),
        readiness_deadline,
    ) {
        Ok(child) => child,
        Err(error) => {
            return finish(
                pipe,
                &Response::Refused(error.to_string()),
                deadline,
                &frontend,
            )
        }
    };
    let result = serve_pending(
        pipe,
        &frontend,
        &launcher,
        readiness_deadline,
        deadline,
        &mut child,
        &mut startup,
    );
    if startup.resolution.is_none() {
        if child.abort(deadline).is_ok() {
            startup.resolution = Some(Resolution::Aborted);
        } else {
            // A new start cannot displace this exact process until the Job
            // is empty and its retained process handle signals.
            startup.unfinished = Some(child);
        }
    }
    *previous = Some(startup);
    result
}

fn serve_pending(
    pipe: &Pipe,
    frontend: &OwnedHandle,
    launcher: &OwnedHandle,
    readiness: Instant,
    deadline: Instant,
    child: &mut PendingDaemonProcess,
    startup: &mut Startup,
) -> io::Result<()> {
    let caller = Some(frontend);
    pipe.write(
        &Response::Started {
            pid: child.pid(),
            ticket: startup.ticket.clone(),
        },
        Some(readiness),
        caller,
    )?;
    loop {
        let response = match pipe.read::<Request>(Some(readiness), caller)? {
            Request::Poll => response_or_refusal(child.try_wait().and_then(|status| {
                Ok(Response::Status {
                    exited: status
                        .and_then(|value| value.code())
                        .map(|code| code as u32),
                    ready: child.is_ready()?,
                })
            })),
            Request::Commit => {
                if !is_alive(frontend) || !is_alive(launcher) {
                    return Err(io::Error::new(io::ErrorKind::BrokenPipe, "frontend exited"));
                }
                let response =
                    response_or_refusal(child.commit(readiness).map(|()| Response::Committed));
                if matches!(response, Response::Committed) {
                    startup.resolution = Some(Resolution::Committed);
                    return finish(pipe, &response, deadline, frontend);
                }
                response
            }
            Request::Abort { cutoff } => {
                let deadline = deadline.min(deadline_at(cutoff)?);
                let response =
                    response_or_refusal(child.abort(deadline).map(|()| Response::Aborted));
                if matches!(response, Response::Aborted) {
                    startup.resolution = Some(Resolution::Aborted);
                }
                return finish(pipe, &response, deadline, frontend);
            }
            _ => return Err(invalid("daemon startup is already pending")),
        };
        pipe.write(&response, Some(readiness), caller)?;
    }
}

fn finish(
    pipe: &Pipe,
    response: &Response,
    deadline: Instant,
    frontend: &OwnedHandle,
) -> io::Result<()> {
    pipe.write(response, Some(deadline), Some(frontend))?;
    // An explicit disconnect discards unread replies. Wait for the peer to
    // close after it has consumed the acknowledgement.
    let _ = pipe.read::<Request>(Some(deadline), Some(frontend));
    Ok(())
}

fn response_or_refusal(result: io::Result<Response>) -> Response {
    result.unwrap_or_else(|error| Response::Refused(error.to_string()))
}

fn valid_digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn invalid(reason: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, reason)
}

#[doc(hidden)]
pub enum BootstrapDaemonStatus {
    Pending,
    Ready,
    Exited(ExitStatus),
}

/// A lost response is resolved by the original ticket or the retained
/// daemon's final authenticated endpoint, never by a replacement process.
#[doc(hidden)]
pub enum BootstrapDaemonCommit {
    Committed,
    Unknown,
}

#[doc(hidden)]
pub struct BootstrapDaemonChild {
    pipe: Option<Pipe>,
    address: String,
    server_pid: u32,
    server: OwnedHandle,
    daemon: OwnedHandle,
    pid: u32,
    ticket: String,
    confirmed_abort: bool,
}

impl BootstrapDaemonChild {
    pub fn spawn_if_supervised(
        state_root: &Path,
        identity: &str,
        idle_grace: Duration,
        readiness_deadline: Instant,
        deadline: Instant,
    ) -> io::Result<Option<Self>> {
        let address = match std::env::var(PIPE_ENV) {
            Ok(address) if valid_digest(&address) => address,
            Err(std::env::VarError::NotPresent) => return Ok(None),
            _ => return Err(invalid("invalid Windows daemon owner handoff")),
        };
        let expected_root = std::env::var_os("UNICA_PROVIDER_STATE_DIR");
        if expected_root.as_deref().map(Path::new) != Some(state_root) {
            return Err(invalid(
                "daemon state root differs from approved installation",
            ));
        }
        let image = std::env::var_os(IMAGE_ENV)
            .ok_or_else(|| invalid("missing installed daemon owner image"))?;
        let image = FilePin::file(Path::new(&image))?;
        let pipe = Pipe::connect(&address, deadline)?;
        let server_pid = pipe.server_pid()?;
        let server = verified_process(server_pid, &image)?;
        let idle_grace_ms = u64::try_from(idle_grace.as_millis())
            .map_err(|_| invalid("invalid daemon idle grace"))?;
        let cutoff = cutoff_at(deadline);
        pipe.write(
            &Request::Start {
                identity: identity.into(),
                idle_grace_ms,
                readiness_cutoff: cutoff_at(readiness_deadline).min(cutoff),
                cutoff,
            },
            Some(deadline),
            Some(&server),
        )?;
        match pipe.read::<Response>(Some(deadline), Some(&server))? {
            Response::Started { pid, ticket } => {
                // The broker retains its process handle until this connection
                // commits or aborts. The kernel cannot recycle this PID here.
                let core = FilePin::file(&std::env::current_exe()?)?;
                let daemon = verified_process(pid, &core)?;
                Ok(Some(Self {
                    pipe: Some(pipe),
                    address,
                    server_pid,
                    server,
                    daemon,
                    pid,
                    ticket,
                    confirmed_abort: false,
                }))
            }
            Response::Refused(reason) => Err(io::Error::other(reason)),
            _ => Err(invalid("invalid response to daemon start")),
        }
    }

    pub fn id(&self) -> u32 {
        self.pid
    }

    pub fn daemon_is_alive(&self) -> bool {
        is_alive(&self.daemon)
    }

    pub fn status(&mut self, deadline: Instant) -> io::Result<BootstrapDaemonStatus> {
        match self.request(Request::Poll, deadline)? {
            Response::Status {
                exited: Some(code), ..
            } => Ok(BootstrapDaemonStatus::Exited(ExitStatus::from_raw(code))),
            Response::Status {
                exited: None,
                ready: true,
            } => Ok(BootstrapDaemonStatus::Ready),
            Response::Status {
                exited: None,
                ready: false,
            } => Ok(BootstrapDaemonStatus::Pending),
            Response::Refused(reason) => Err(io::Error::other(reason)),
            _ => Err(invalid("invalid response to poll of pending daemon")),
        }
    }

    pub fn commit(&mut self, deadline: Instant) -> io::Result<BootstrapDaemonCommit> {
        check_deadline(deadline)?;
        // Give an interrupted acknowledgement part of the same budget for a
        // fresh, authenticated query bound to this exact daemon's startup.
        let ack_deadline = Instant::now() + deadline.saturating_duration_since(Instant::now()) / 2;
        match self.request(Request::Commit, ack_deadline) {
            Ok(Response::Committed) => Ok(BootstrapDaemonCommit::Committed),
            Ok(Response::Refused(reason)) => Err(io::Error::other(reason)),
            _ => {
                self.pipe.take();
                // Leave part of the original cutoff for the retained
                // daemon's authenticated endpoint if the owner disappeared.
                let recovery =
                    Instant::now() + deadline.saturating_duration_since(Instant::now()) / 2;
                match self.recover(recovery) {
                    Ok(Response::Committed) => Ok(BootstrapDaemonCommit::Committed),
                    Ok(Response::Aborted) => {
                        self.confirmed_abort = true;
                        Err(io::Error::other("daemon startup aborted before handoff"))
                    }
                    _ => Ok(BootstrapDaemonCommit::Unknown),
                }
            }
        }
    }

    pub fn abort(&mut self, deadline: Instant) -> io::Result<()> {
        if self.confirmed_abort {
            return Ok(());
        }
        let ack_deadline = Instant::now() + deadline.saturating_duration_since(Instant::now()) / 2;
        let response = match self.request(
            Request::Abort {
                cutoff: cutoff_at(deadline),
            },
            ack_deadline,
        ) {
            Ok(response @ (Response::Aborted | Response::Committed)) => response,
            // Transport failures in this request, and an abort that escaped
            // before proving the Job empty, both require an exact-ticket recheck.
            _ => {
                self.pipe.take();
                self.recover(deadline)?
            }
        };
        match response {
            Response::Aborted => {
                self.confirmed_abort = true;
                Ok(())
            }
            Response::Committed => Err(io::Error::other("daemon handoff already committed")),
            Response::Refused(reason) => Err(io::Error::other(reason)),
            _ => Err(invalid("invalid response to daemon abort recovery")),
        }
    }

    fn request(&mut self, request: Request, deadline: Instant) -> io::Result<Response> {
        let pipe = self
            .pipe
            .as_ref()
            .ok_or_else(|| io::Error::other("daemon startup connection lost"))?;
        let result = pipe
            .write(&request, Some(deadline), Some(&self.server))
            .and_then(|()| pipe.read::<Response>(Some(deadline), Some(&self.server)));
        if result.is_err() {
            self.pipe.take();
        }
        result
    }

    fn recover(&self, deadline: Instant) -> io::Result<Response> {
        let pipe = Pipe::connect(&self.address, deadline)?;
        if !is_alive(&self.server) || pipe.server_pid()? != self.server_pid {
            return Err(invalid("daemon owner changed during startup recovery"));
        }
        pipe.write(
            &Request::Recover {
                ticket: self.ticket.clone(),
            },
            Some(deadline),
            Some(&self.server),
        )?;
        pipe.read::<Response>(Some(deadline), Some(&self.server))
    }
}

#[cfg(test)]
mod medium_tests {
    use super::*;

    #[test]
    fn the_installed_windows_runtime_uses_the_owner_only_by_explicit_opt_in() {
        assert!(!daemon_owner_opt_in(None).unwrap());
        assert!(daemon_owner_opt_in(Some(OsStr::new("1"))).unwrap());
        for value in ["", "true", "0", "other installation"] {
            assert!(daemon_owner_opt_in(Some(OsStr::new(value))).is_err());
        }
    }

    #[test]
    fn owner_requests_cannot_supply_an_executable_or_working_directory() {
        let valid = r#"{"Start":{"identity":"fixture","idle_grace_ms":2000,"readiness_cutoff":1,"cutoff":2}}"#;
        assert!(matches!(
            serde_json::from_str::<Request>(valid),
            Ok(Request::Start { .. })
        ));
        for field in [
            "executable",
            "cwd",
            "argv",
            "environment",
            "provider_state_root",
        ] {
            let request = valid.replacen(
                "\"cutoff\":2",
                &format!(r#""cutoff":2,"{field}":"foreign""#),
                1,
            );
            assert!(
                serde_json::from_str::<Request>(&request).is_err(),
                "{field}"
            );
        }
    }
}
