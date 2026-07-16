use crate::domain::cancellation::CancellationToken;
use std::ffi::OsString;
use std::io::{self, Read};
use std::path::PathBuf;
use std::process::{Child, ChildStderr, ChildStdin, ChildStdout, Command, ExitStatus, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::{Duration, Instant};

const PROCESS_POLL_INTERVAL: Duration = Duration::from_millis(25);
const TERMINATION_WAIT_LIMIT: Duration = Duration::from_millis(500);
const READER_WAIT_LIMIT: Duration = Duration::from_millis(500);

#[derive(Debug, Clone)]
pub struct ManagedCommand {
    pub program: PathBuf,
    pub args: Vec<String>,
    pub cwd: PathBuf,
    pub env: Vec<(OsString, OsString)>,
    pub timeout: Option<Duration>,
    pub cancellation: CancellationToken,
}

#[derive(Debug, Clone)]
pub struct ManagedOutput {
    pub status_success: bool,
    pub status: String,
    pub stdout: String,
    pub stderr: String,
    pub timed_out: bool,
    pub cancelled: bool,
}

pub struct ManagedChild {
    child: Child,
    timeout: Option<Duration>,
    cancellation: CancellationToken,
}

impl ManagedChild {
    pub fn spawn(command: ManagedCommand) -> Result<Self, String> {
        let mut process = Command::new(&command.program);
        process
            .args(&command.args)
            .current_dir(&command.cwd)
            .envs(command.env)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        prepare_process_tree(&mut process).map_err(process_error)?;
        let child = process.spawn().map_err(process_error)?;

        Ok(Self {
            child,
            timeout: command.timeout,
            cancellation: command.cancellation,
        })
    }

    pub fn run(command: ManagedCommand) -> Result<ManagedOutput, String> {
        let mut child = Self::spawn(command)?;
        child.wait_for_output()
    }

    pub fn take_stdin(&mut self) -> Option<ChildStdin> {
        self.child.stdin.take()
    }

    pub fn take_stdout(&mut self) -> Option<ChildStdout> {
        self.child.stdout.take()
    }

    pub fn take_stderr(&mut self) -> Option<ChildStderr> {
        self.child.stderr.take()
    }

    pub fn wait_for_output(&mut self) -> Result<ManagedOutput, String> {
        self.wait_for_output_with_poll(PROCESS_POLL_INTERVAL, || {})
    }

    pub fn wait_for_output_with_poll<F>(
        &mut self,
        interval: Duration,
        mut callback: F,
    ) -> Result<ManagedOutput, String>
    where
        F: FnMut(),
    {
        drop(self.take_stdin());
        let stdout = start_reader(self.take_stdout());
        let stderr = start_reader(self.take_stderr());
        let started = Instant::now();
        let mut last_callback = Instant::now();

        loop {
            if let Some(status) = self.child.try_wait().map_err(process_error)? {
                return Ok(finish_output(status, stdout, stderr, false, false));
            }
            if self.cancellation.is_cancelled() {
                self.terminate()?;
                return self.finish_after_termination(stdout, stderr, false, true);
            }
            if self.timeout.is_some_and(|limit| started.elapsed() >= limit) {
                self.terminate()?;
                return self.finish_after_termination(stdout, stderr, true, false);
            }

            thread::sleep(PROCESS_POLL_INTERVAL);
            if last_callback.elapsed() >= interval {
                callback();
                last_callback = Instant::now();
            }
        }
    }

    pub fn terminate(&mut self) -> Result<(), String> {
        self.child.kill().map_err(process_error)
    }

    fn finish_after_termination(
        &mut self,
        stdout: Option<Receiver<Vec<u8>>>,
        stderr: Option<Receiver<Vec<u8>>>,
        timed_out: bool,
        cancelled: bool,
    ) -> Result<ManagedOutput, String> {
        let started = Instant::now();
        while started.elapsed() < TERMINATION_WAIT_LIMIT {
            if let Some(status) = self.child.try_wait().map_err(process_error)? {
                return Ok(finish_output(status, stdout, stderr, timed_out, cancelled));
            }
            thread::sleep(PROCESS_POLL_INTERVAL);
        }

        Ok(ManagedOutput {
            status_success: false,
            status: "termination pending".to_string(),
            stdout: receive_output(stdout),
            stderr: receive_output(stderr),
            timed_out,
            cancelled,
        })
    }
}

fn prepare_process_tree(_command: &mut Command) -> io::Result<()> {
    Ok(())
}

fn process_error(error: io::Error) -> String {
    format!("process_failed: {error}")
}

fn start_reader<R>(pipe: Option<R>) -> Option<Receiver<Vec<u8>>>
where
    R: Read + Send + 'static,
{
    pipe.map(|mut pipe| {
        let (sender, receiver) = mpsc::channel();
        thread::spawn(move || {
            let mut bytes = Vec::new();
            let _ = pipe.read_to_end(&mut bytes);
            let _ = sender.send(bytes);
        });
        receiver
    })
}

fn finish_output(
    status: ExitStatus,
    stdout: Option<Receiver<Vec<u8>>>,
    stderr: Option<Receiver<Vec<u8>>>,
    timed_out: bool,
    cancelled: bool,
) -> ManagedOutput {
    ManagedOutput {
        status_success: status.success(),
        status: status.to_string(),
        stdout: receive_output(stdout),
        stderr: receive_output(stderr),
        timed_out,
        cancelled,
    }
}

fn receive_output(receiver: Option<Receiver<Vec<u8>>>) -> String {
    let bytes = receiver
        .and_then(|receiver| receiver.recv_timeout(READER_WAIT_LIMIT).ok())
        .unwrap_or_default();
    String::from_utf8_lossy(&bytes).into_owned()
}

#[cfg(test)]
mod tests {
    use super::{ManagedChild, ManagedCommand, ManagedOutput};
    use crate::domain::cancellation::CancellationToken;
    use std::ffi::OsString;
    use std::io::Read;
    use std::thread;
    use std::time::{Duration, Instant};

    const HELPER_ENV: &str = "UNICA_MANAGED_CHILD_HELPER";

    #[test]
    fn managed_child_test_helper() {
        let Ok(mode) = std::env::var(HELPER_ENV) else {
            return;
        };

        match mode.as_str() {
            "success" => {
                print!("managed stdout");
                eprint!("managed stderr");
            }
            "read_stdin" => {
                let mut input = String::new();
                std::io::stdin().read_to_string(&mut input).unwrap();
                print!("stdin closed");
            }
            "sleep" => thread::sleep(Duration::from_secs(10)),
            other => panic!("unknown managed child helper mode: {other}"),
        }
    }

    fn run_helper(
        mode: &str,
        timeout: Duration,
        cancellation: CancellationToken,
    ) -> Result<ManagedOutput, String> {
        ManagedChild::run(ManagedCommand {
            program: std::env::current_exe().map_err(|error| error.to_string())?,
            args: vec![
                "--exact".to_string(),
                "infrastructure::managed_child::tests::managed_child_test_helper".to_string(),
                "--nocapture".to_string(),
            ],
            cwd: std::env::current_dir().map_err(|error| error.to_string())?,
            env: vec![(OsString::from(HELPER_ENV), OsString::from(mode))],
            timeout: Some(timeout),
            cancellation,
        })
    }

    #[test]
    fn managed_child_collects_stdout_and_stderr_on_success() {
        let output =
            run_helper("success", Duration::from_secs(2), CancellationToken::new()).unwrap();

        assert!(output.status_success, "status was {}", output.status);
        assert!(
            output.stdout.contains("managed stdout"),
            "{}",
            output.stdout
        );
        assert!(
            output.stderr.contains("managed stderr"),
            "{}",
            output.stderr
        );
        assert!(!output.timed_out);
        assert!(!output.cancelled);
    }

    #[test]
    fn managed_child_spawn_failure_uses_stable_process_failed_prefix() {
        let error = ManagedChild::spawn(ManagedCommand {
            program: std::env::temp_dir().join("unica-managed-child-missing-executable"),
            args: Vec::new(),
            cwd: std::env::current_dir().unwrap(),
            env: Vec::new(),
            timeout: None,
            cancellation: CancellationToken::new(),
        })
        .err()
        .expect("missing executable must fail to spawn");

        assert!(error.starts_with("process_failed:"), "{error}");
    }

    #[test]
    fn managed_child_timeout_returns_within_a_bounded_interval() {
        let started = Instant::now();
        let output = run_helper(
            "sleep",
            Duration::from_millis(100),
            CancellationToken::new(),
        )
        .unwrap();

        assert!(output.timed_out);
        assert!(!output.cancelled);
        assert!(started.elapsed() < Duration::from_secs(2));
    }

    #[test]
    fn managed_child_closes_unused_stdin_before_waiting() {
        let output = run_helper(
            "read_stdin",
            Duration::from_millis(300),
            CancellationToken::new(),
        )
        .unwrap();

        assert!(!output.timed_out);
        assert!(output.stdout.contains("stdin closed"), "{}", output.stdout);
    }

    #[test]
    fn managed_child_cancellation_returns_within_a_bounded_interval() {
        let cancellation = CancellationToken::new();
        let cancellation_for_thread = cancellation.clone();
        thread::spawn(move || {
            thread::sleep(Duration::from_millis(100));
            cancellation_for_thread.cancel();
        });
        let started = Instant::now();
        let output = run_helper("sleep", Duration::from_secs(10), cancellation).unwrap();

        assert!(output.cancelled);
        assert!(!output.timed_out);
        assert!(started.elapsed() < Duration::from_secs(2));
    }
}
