use crate::{
    ensure_truncation_diagnostics, ManagedChild, ManagedCommand, ManagedLineOutput, ManagedOutput,
    ManagedSpawnFailure, PendingProcessHandoff, PendingRunFailure, ProcessLineReply, StreamControl,
    STREAM_LINE_TOO_LONG_ERROR, STREAM_RECORD_UNTERMINATED_ERROR,
};
use std::{ffi::OsString, path::PathBuf, time::Duration};

/// A runner without its own streaming captured an output it cannot vouch for.
pub const STREAM_CAPTURE_INCOMPLETE_ERROR: &str = "captured stdout is truncated or is not UTF-8";
use unica_work_control::cancellation::CancellationToken;

#[derive(Debug, Clone)]
pub struct ProcessCommand {
    pub program: PathBuf,
    pub args: Vec<String>,
    pub cwd: PathBuf,
    pub env: Vec<(OsString, OsString)>,
    pub env_remove: Vec<OsString>,
    pub capture_limits: Option<(usize, usize)>,
    pub timeout: Option<Duration>,
    pub cancellation: CancellationToken,
}

#[derive(Debug, Clone)]
pub struct ProcessOutput {
    pub status_success: bool,
    pub status: String,
    pub stdout: String,
    pub stderr: String,
    pub timed_out: bool,
    pub cancelled: bool,
    pub stdout_truncated: bool,
    pub stderr_truncated: bool,
    pub stdout_had_invalid_utf8: bool,
    pub stderr_had_invalid_utf8: bool,
}

#[derive(Debug, Clone)]
pub struct ProcessStreamOutput {
    pub status_success: bool,
    pub status: String,
    pub stderr: String,
    pub timed_out: bool,
    pub cancelled: bool,
    pub stopped_by_consumer: bool,
    pub line_error: Option<(usize, String)>,
}

/// Where a process run failed: before the process existed, or while it was
/// being waited for or stopped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProcessStreamFailure {
    Spawn(String),
    Run(String),
}

impl ProcessStreamFailure {
    /// The failure's own text, whichever side of the start it happened on.
    pub fn message(&self) -> &str {
        match self {
            Self::Spawn(message) | Self::Run(message) => message,
        }
    }

    /// Only a launch the operating system refused is `Spawn`: a process tree
    /// or working directory that was not ready never tried the program, and
    /// a process that existed but could not be attached did start. Neither
    /// may be reported as an unstartable program.
    pub fn from_spawn(failure: ManagedSpawnFailure) -> Self {
        match failure {
            ManagedSpawnFailure::Launch(message) => Self::Spawn(message),
            ManagedSpawnFailure::Prepare(message) | ManagedSpawnFailure::Attach(message) => {
                Self::Run(message)
            }
        }
    }
}

fn managed_command(command: &ProcessCommand) -> ManagedCommand {
    ManagedCommand {
        program: command.program.clone(),
        args: command.args.iter().map(Into::into).collect(),
        cwd: command.cwd.clone(),
        env: command.env.clone(),
        env_remove: command.env_remove.clone(),
        capture_limits: command.capture_limits,
        timeout: command.timeout,
        cancellation: command.cancellation.clone(),
    }
}

pub trait ProcessRunner {
    fn run(&self, command: &ProcessCommand) -> Result<ProcessOutput, String>;

    /// [`ProcessRunner::run`] that keeps whether the process was ever
    /// started. A runner that cannot tell reports a failure after start, so
    /// nothing is classified as an unstartable engine by guess.
    fn run_staged(&self, command: &ProcessCommand) -> Result<ProcessOutput, ProcessStreamFailure> {
        self.run(command).map_err(ProcessStreamFailure::Run)
    }

    /// [`ProcessRunner::run_streaming`] that keeps whether the process was
    /// ever started. A runner that cannot tell reports a failure after start.
    fn run_streaming_staged(
        &self,
        command: &ProcessCommand,
        max_line_bytes: usize,
        on_line: &mut dyn FnMut(usize, &[u8]) -> StreamControl,
    ) -> Result<ProcessStreamOutput, ProcessStreamFailure> {
        self.run_streaming(command, max_line_bytes, on_line)
            .map_err(ProcessStreamFailure::Run)
    }

    /// Exchange live, bounded line replies with one owned process. Unlike the
    /// observation-only fallback, this cannot replay a captured `run` output:
    /// the process must receive each decision before producing its next frame.
    fn run_controlled_streaming_staged(
        &self,
        _command: &ProcessCommand,
        _max_line_bytes: usize,
        _on_line: &mut dyn FnMut(usize, &[u8]) -> ProcessLineReply,
    ) -> Result<ProcessStreamOutput, ProcessStreamFailure> {
        Err(ProcessStreamFailure::Run(
            "process_failed: process runner does not support controlled streaming".into(),
        ))
    }

    fn run_pending_handoff(
        &self,
        _command: &ProcessCommand,
    ) -> Result<(ProcessOutput, PendingProcessHandoff), String> {
        Err("process_failed: process runner does not support ownership handoff".to_string())
    }

    /// [`ProcessRunner::run_pending_handoff`] that keeps whether the launcher
    /// was ever started. A runner that cannot tell reports a failure after
    /// start.
    fn run_pending_handoff_staged(
        &self,
        command: &ProcessCommand,
    ) -> Result<(ProcessOutput, PendingProcessHandoff), ProcessStreamFailure> {
        self.run_pending_handoff(command)
            .map_err(ProcessStreamFailure::Run)
    }

    fn run_with_input(
        &self,
        command: &ProcessCommand,
        input: &[u8],
    ) -> Result<ProcessOutput, String> {
        if input.is_empty() {
            self.run(command)
        } else {
            Err("process_failed: process runner does not support stdin".to_string())
        }
    }

    /// Stream `delimiter`-terminated stdout records of any length to
    /// `on_record` while `input` is written to stdin. Output that grows with
    /// its input is never captured whole. An unterminated tail is reported in
    /// `line_error` and is not delivered.
    ///
    /// A runner without its own streaming answers from its captured output;
    /// a truncated or non-UTF-8 capture delivers no record and is reported in
    /// `line_error`.
    fn run_streaming_records(
        &self,
        command: &ProcessCommand,
        input: &[u8],
        delimiter: u8,
        on_record: &mut dyn FnMut(&[u8]) -> StreamControl,
    ) -> Result<ProcessStreamOutput, String> {
        let output = if input.is_empty() {
            self.run(command)?
        } else {
            self.run_with_input(command, input)?
        };
        let mut result = ProcessStreamOutput {
            status_success: output.status_success,
            status: output.status,
            stderr: output.stderr,
            timed_out: output.timed_out,
            cancelled: output.cancelled,
            stopped_by_consumer: false,
            line_error: None,
        };
        if output.stdout_truncated || output.stdout_had_invalid_utf8 {
            result.line_error = Some((0, STREAM_CAPTURE_INCOMPLETE_ERROR.to_string()));
            return Ok(result);
        }
        let mut records = output.stdout.as_bytes().split(|byte| *byte == delimiter);
        let tail = records.next_back().unwrap_or_default();
        let mut delivered = 0_usize;
        for record in records {
            delivered += 1;
            if on_record(record) == StreamControl::Stop {
                result.status_success = false;
                result.status = "stopped by consumer".to_string();
                result.stopped_by_consumer = true;
                return Ok(result);
            }
        }
        if !tail.is_empty() {
            result.line_error = Some((delivered + 1, STREAM_RECORD_UNTERMINATED_ERROR.to_string()));
        }
        Ok(result)
    }

    fn run_streaming(
        &self,
        command: &ProcessCommand,
        max_line_bytes: usize,
        on_line: &mut dyn FnMut(usize, &[u8]) -> StreamControl,
    ) -> Result<ProcessStreamOutput, String> {
        let output = self.run(command)?;
        let mut line_error = None;
        let logical_line_count = output.stdout.lines().count();
        for (index, bytes) in output
            .stdout
            .as_bytes()
            .split(|byte| *byte == b'\n')
            .enumerate()
        {
            if bytes.is_empty() && index + 1 == logical_line_count + 1 {
                continue;
            }
            if bytes.len() > max_line_bytes {
                line_error
                    .get_or_insert_with(|| (index + 1, STREAM_LINE_TOO_LONG_ERROR.to_string()));
            } else {
                if on_line(index + 1, bytes) == StreamControl::Stop {
                    return Ok(ProcessStreamOutput {
                        status_success: false,
                        status: "stopped by consumer".to_string(),
                        stderr: output.stderr,
                        timed_out: false,
                        cancelled: output.cancelled,
                        stopped_by_consumer: true,
                        line_error,
                    });
                }
            }
        }
        Ok(ProcessStreamOutput {
            status_success: output.status_success,
            status: output.status,
            stderr: output.stderr,
            timed_out: output.timed_out,
            cancelled: output.cancelled,
            stopped_by_consumer: false,
            line_error,
        })
    }
}

pub struct SystemProcessRunner;
static SYSTEM_PROCESS_RUNNER: SystemProcessRunner = SystemProcessRunner;
/// Тестовый наблюдатель запусков процессов системным исполнителем: записывает
/// программу каждого запуска, рабочий каталог которого лежит под корнем.
#[cfg(any(test, feature = "test-support"))]
pub mod process_launch_probe {
    use std::path::{Path, PathBuf};
    use std::sync::Mutex;

    static LAUNCHES: Mutex<Vec<(PathBuf, String)>> = Mutex::new(Vec::new());

    fn canonical(path: &Path) -> PathBuf {
        std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
    }

    pub(super) fn record(cwd: &Path, program: &std::ffi::OsStr) {
        LAUNCHES
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .push((canonical(cwd), program.to_string_lossy().into_owned()));
    }

    /// Programs launched so far with a working directory below `root`.
    pub fn launches_under_for_test(root: &Path) -> Vec<String> {
        let root = canonical(root);
        LAUNCHES
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .iter()
            .filter(|(cwd, _)| cwd.starts_with(&root))
            .map(|(_, program)| program.clone())
            .collect()
    }
}
pub fn system_process_runner() -> &'static (dyn ProcessRunner + Send + Sync) {
    &SYSTEM_PROCESS_RUNNER
}

impl ProcessRunner for SystemProcessRunner {
    fn run_staged(&self, command: &ProcessCommand) -> Result<ProcessOutput, ProcessStreamFailure> {
        #[cfg(any(test, feature = "test-support"))]
        process_launch_probe::record(&command.cwd, command.program.as_ref());
        let mut child = ManagedChild::spawn_staged(managed_command(command))
            .map_err(ProcessStreamFailure::from_spawn)?;
        let output = child.wait_for_output().map_err(ProcessStreamFailure::Run)?;
        Ok(map_managed_process_output(output))
    }

    fn run(&self, command: &ProcessCommand) -> Result<ProcessOutput, String> {
        #[cfg(any(test, feature = "test-support"))]
        process_launch_probe::record(&command.cwd, command.program.as_ref());
        let output = ManagedChild::run(ManagedCommand {
            program: command.program.clone(),
            args: command.args.iter().map(Into::into).collect(),
            cwd: command.cwd.clone(),
            env: command.env.clone(),
            env_remove: command.env_remove.clone(),
            capture_limits: command.capture_limits,
            timeout: command.timeout,
            cancellation: command.cancellation.clone(),
        })?;
        Ok(map_managed_process_output(output))
    }

    fn run_pending_handoff(
        &self,
        command: &ProcessCommand,
    ) -> Result<(ProcessOutput, PendingProcessHandoff), String> {
        self.run_pending_handoff_staged(command)
            .map_err(|failure| failure.message().to_string())
    }

    fn run_pending_handoff_staged(
        &self,
        command: &ProcessCommand,
    ) -> Result<(ProcessOutput, PendingProcessHandoff), ProcessStreamFailure> {
        #[cfg(any(test, feature = "test-support"))]
        process_launch_probe::record(&command.cwd, command.program.as_ref());
        let (output, handoff) = ManagedChild::run_pending_handoff_staged(managed_command(command))
            .map_err(|failure| match failure {
                PendingRunFailure::Spawn(failure) => ProcessStreamFailure::from_spawn(failure),
                PendingRunFailure::Run(message) => ProcessStreamFailure::Run(message),
            })?;
        Ok((map_managed_process_output(output), handoff))
    }

    fn run_with_input(
        &self,
        command: &ProcessCommand,
        input: &[u8],
    ) -> Result<ProcessOutput, String> {
        #[cfg(any(test, feature = "test-support"))]
        process_launch_probe::record(&command.cwd, command.program.as_ref());
        let output = ManagedChild::run_with_input(
            ManagedCommand {
                program: command.program.clone(),
                args: command.args.iter().map(Into::into).collect(),
                cwd: command.cwd.clone(),
                env: command.env.clone(),
                env_remove: command.env_remove.clone(),
                capture_limits: command.capture_limits,
                timeout: command.timeout,
                cancellation: command.cancellation.clone(),
            },
            input.to_vec(),
        )?;
        Ok(map_managed_process_output(output))
    }

    fn run_streaming_records(
        &self,
        command: &ProcessCommand,
        input: &[u8],
        delimiter: u8,
        on_record: &mut dyn FnMut(&[u8]) -> StreamControl,
    ) -> Result<ProcessStreamOutput, String> {
        #[cfg(any(test, feature = "test-support"))]
        process_launch_probe::record(&command.cwd, command.program.as_ref());
        let mut child = ManagedChild::spawn(managed_command(command))?;
        let output = child.wait_for_record_output(input.to_vec(), delimiter, on_record)?;
        Ok(map_managed_line_output(output))
    }

    fn run_streaming(
        &self,
        command: &ProcessCommand,
        max_line_bytes: usize,
        on_line: &mut dyn FnMut(usize, &[u8]) -> StreamControl,
    ) -> Result<ProcessStreamOutput, String> {
        self.run_streaming_staged(command, max_line_bytes, on_line)
            .map_err(|failure| match failure {
                ProcessStreamFailure::Spawn(message) | ProcessStreamFailure::Run(message) => {
                    message
                }
            })
    }

    fn run_streaming_staged(
        &self,
        command: &ProcessCommand,
        max_line_bytes: usize,
        on_line: &mut dyn FnMut(usize, &[u8]) -> StreamControl,
    ) -> Result<ProcessStreamOutput, ProcessStreamFailure> {
        #[cfg(any(test, feature = "test-support"))]
        process_launch_probe::record(&command.cwd, command.program.as_ref());
        let mut child = ManagedChild::spawn_staged(managed_command(command))
            .map_err(ProcessStreamFailure::from_spawn)?;
        let output = child
            .wait_for_line_output(max_line_bytes, on_line)
            .map_err(ProcessStreamFailure::Run)?;
        Ok(map_managed_line_output(output))
    }

    fn run_controlled_streaming_staged(
        &self,
        command: &ProcessCommand,
        max_line_bytes: usize,
        on_line: &mut dyn FnMut(usize, &[u8]) -> ProcessLineReply,
    ) -> Result<ProcessStreamOutput, ProcessStreamFailure> {
        #[cfg(any(test, feature = "test-support"))]
        process_launch_probe::record(&command.cwd, command.program.as_ref());
        let mut child = ManagedChild::spawn_staged(managed_command(command))
            .map_err(ProcessStreamFailure::from_spawn)?;
        let output = child
            .wait_for_controlled_line_output(max_line_bytes, on_line)
            .map_err(ProcessStreamFailure::Run)?;
        Ok(map_managed_line_output(output))
    }
}

fn map_managed_process_output(mut output: ManagedOutput) -> ProcessOutput {
    let stdout_truncated = output.stdout_truncated;
    let stderr_truncated = output.stderr_truncated;
    let stdout_had_invalid_utf8 = output.stdout_had_invalid_utf8;
    let stderr_had_invalid_utf8 = output.stderr_had_invalid_utf8;
    ensure_truncation_diagnostics(&mut output);
    let output = ProcessOutput {
        status_success: output.status_success,
        status: output.status,
        stdout: output.stdout,
        stderr: output.stderr,
        timed_out: output.timed_out,
        cancelled: output.cancelled,
        stdout_truncated,
        stderr_truncated,
        stdout_had_invalid_utf8,
        stderr_had_invalid_utf8,
    };
    debug_assert!(!(output.timed_out && output.cancelled));
    output
}

fn map_managed_line_output(output: ManagedLineOutput) -> ProcessStreamOutput {
    ProcessStreamOutput {
        status_success: output.status_success,
        status: output.status,
        stderr: output.stderr,
        timed_out: output.timed_out,
        cancelled: output.cancelled,
        stopped_by_consumer: output.stopped_by_consumer,
        line_error: output.line_error,
    }
}

#[cfg(test)]
#[test]
fn managed_truncation_is_visible_at_process_adapter_boundary() {
    let output = map_managed_process_output(ManagedOutput {
        status_success: false,
        status: "exit status: 0".into(),
        stdout: "tail".into(),
        stderr: "diagnostic tail".into(),
        timed_out: false,
        cancelled: false,
        stdout_truncated: true,
        stderr_truncated: true,
        stdout_had_invalid_utf8: true,
        stderr_had_invalid_utf8: true,
    });
    assert!(output.stderr.contains("stdout capture truncated"));
    assert!(output.stderr.contains("earlier stderr diagnostics omitted"));
    assert!(output.stdout_truncated);
    assert!(output.stderr_truncated);
    assert!(output.stdout_had_invalid_utf8);
    assert!(output.stderr_had_invalid_utf8);
}

#[cfg(test)]
mod record_fallback_tests {
    use super::{
        ProcessCommand, ProcessOutput, ProcessRunner, StreamControl,
        STREAM_CAPTURE_INCOMPLETE_ERROR, STREAM_RECORD_UNTERMINATED_ERROR,
    };
    use std::path::PathBuf;
    use unica_work_control::cancellation::CancellationToken;

    struct CapturedRunner(ProcessOutput);

    impl ProcessRunner for CapturedRunner {
        fn run(&self, _command: &ProcessCommand) -> Result<ProcessOutput, String> {
            Ok(self.0.clone())
        }
    }

    fn captured(stdout: &str, stdout_truncated: bool) -> CapturedRunner {
        CapturedRunner(ProcessOutput {
            status_success: true,
            status: "exit status: 0".into(),
            stdout: stdout.into(),
            stderr: String::new(),
            timed_out: false,
            cancelled: false,
            stdout_truncated,
            stderr_truncated: false,
            stdout_had_invalid_utf8: false,
            stderr_had_invalid_utf8: false,
        })
    }

    fn command() -> ProcessCommand {
        ProcessCommand {
            program: PathBuf::from("git"),
            args: Vec::new(),
            cwd: PathBuf::from("."),
            env: Vec::new(),
            env_remove: Vec::new(),
            capture_limits: None,
            timeout: None,
            cancellation: CancellationToken::new(),
        }
    }

    fn records(runner: &CapturedRunner) -> (Vec<Vec<u8>>, Option<(usize, String)>) {
        let mut records = Vec::new();
        let output = runner
            .run_streaming_records(&command(), &[], 0, &mut |record| {
                records.push(record.to_vec());
                StreamControl::Continue
            })
            .unwrap();
        (records, output.line_error)
    }

    #[test]
    fn captured_records_keep_empty_records_and_reject_an_unterminated_tail() {
        assert_eq!(
            records(&captured("a\0\0b\0", false)),
            (vec![b"a".to_vec(), Vec::new(), b"b".to_vec()], None)
        );
        assert_eq!(
            records(&captured("a\0tail", false)),
            (
                vec![b"a".to_vec()],
                Some((2, STREAM_RECORD_UNTERMINATED_ERROR.to_string()))
            )
        );
    }

    #[test]
    fn truncated_capture_delivers_no_record() {
        assert_eq!(
            records(&captured("a\0b\0", true)),
            (
                Vec::new(),
                Some((0, STREAM_CAPTURE_INCOMPLETE_ERROR.to_string()))
            )
        );
    }
}
