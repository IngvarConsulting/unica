//! Holding the runner's live Designer agent of a working copy.
//!
//! The pinned runner keeps a Designer agent alive between its commands
//! (`provider.endpoint.live`). An agent nobody holds goes after 15 minutes
//! without commands; a held one stays while its holder lives. Unica holds the
//! agent of a working copy it works with, so its runner commands neither start
//! the Designer anew nor take another license session.
//!
//! The holder is this daemon process: `--holder-pid` and the start time the OS
//! names for it. Unica holds only an agent that already stays after a command
//! of its own (`kept`) and does not have it among its holders: the agent is
//! then the one the command used, and `agent hold` only marks it. Unica lets
//! go of it after [`IDLE`] without runner commands and without editor sessions
//! of the working copy, when the last editor session of the working copy
//! closes, and before it holds a third one ([`LIMIT`], the runner's
//! `live-max` by default). When the daemon exits, the runner removes the mark
//! of a holder that is gone. Holding and letting go run apart from the answer
//! of the command, and a hold that fails never fails a command.
//!
//! As the runner keeps it, a live agent stays connected to its infobase
//! between commands and so locks a file infobase for configuring;
//! `v8-runner agent release` in the working copy disconnects it.
use super::runner::PinnedRunner;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};
use unica_process::runner::{ProcessCommand, ProcessRunner};
use unica_work_control::cancellation::CancellationToken;
use unica_workspace::paths::normalize_path_identity;

/// Live agents Unica holds at most: the runner's `live-max` by default.
// resource-limit: live-agent.held-agents
// class: concurrency
// unit: count
// scope: process
// value: 2
// resource: Живые агенты Конфигуратора, которых держит один процесс демона Unica.
// status: unproven
// origin: предел live-max раннера v8-runner 0.17 по умолчанию (#2802)
// problem: третий удержанный агент занял бы место, которого у учётной записи ОС нет, и лишний сеанс лицензии
// on-reach: перед удержанием агента третьей рабочей копии Unica отпускает давно не нужного агента без сессий редактора
pub(super) const LIMIT: usize = 2;

/// Time without runner commands and editor sessions after which Unica lets go
/// of the agent of a working copy: the warm time of a workspace actor
/// (`WARM_WORKSPACE_ACTOR_TTL`).
pub(super) const IDLE: Duration = Duration::from_secs(600);

/// How long `agent hold` and `agent release` may take: they only mark an agent
/// that is already alive.
// resource-limit: live-agent.mark-timeout
// class: deadline
// unit: ms
// scope: request
// value: 30000
// resource: Время одной команды `agent hold` или `agent release` раннера.
// status: unproven
// origin: #2802
// problem: отметка живого агента, которая не завершилась, держала бы поток удержания
// on-reach: процесс раннера завершается; неоконченное удержание повторяется на следующей команде, отпуск считается сделанным
const MARK_TIMEOUT: Duration = Duration::from_secs(30);

/// Code of the warning that the agent of a command was not kept.
pub(super) const NOT_HELD_CODE: &str = "live_agent_not_held";

const NOT_HELD: &str = "the live Designer agent of this workspace is not held: this OS account already keeps the most live agents the runner allows (tools.designer_agent.live-max); this command ran with a one-off agent that starts the Designer anew. Unica tries to hold the agent again on the next runner command";

const OFF_NOTE: &str = "the live Designer agent is off in this workspace (tools.designer_agent.live: false): each runner command starts the Designer anew, and Unica does not hold an agent here until that setting changes";

const JOB_NOTE: &str = "the live Designer agent cannot be held: the process that started Unica keeps it in a Windows Job that does not allow leaving it; each runner command starts the Designer anew";

/// What a runner command that named its agent leads to.
#[derive(Debug, PartialEq)]
pub(super) struct AfterCommand {
    /// What the answer adds.
    pub(super) reply: Option<AgentReply>,
    /// What Unica does with live agents, apart from the answer
    /// ([`LiveAgents::perform`]).
    pub(super) marks: Vec<(PathBuf, Mark)>,
}

/// What the answer of a runner command adds about its agent.
#[derive(Debug, PartialEq)]
pub(super) enum AgentReply {
    /// A warning of the answer.
    Warning(serde_json::Value),
    /// `data.agent.note`.
    Note(&'static str),
}

/// What Unica asks of the agent of a working copy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Mark {
    Hold,
    Release,
}

/// This daemon process as the holder of live agents.
#[derive(Debug, Clone)]
pub(super) struct Holder {
    pid: u32,
    started_at: Option<String>,
}

impl Holder {
    fn of_this_process() -> Self {
        Self {
            pid: std::process::id(),
            started_at: unica_process::process_started_at()
                .map(|at| chrono::DateTime::<chrono::Utc>::from(at).to_rfc3339()),
        }
    }

    fn args(&self) -> Vec<String> {
        let mut args = vec!["--holder-pid".to_string(), self.pid.to_string()];
        if let Some(started_at) = &self.started_at {
            args.push("--holder-started-at".to_string());
            args.push(started_at.clone());
        }
        args
    }
}

#[derive(Debug, Default)]
struct Root {
    /// The runner of the working copy, from its last command.
    program: Option<PathBuf>,
    held: bool,
    last_used: Option<Instant>,
    editor_sessions: usize,
    /// The runner refused a hold for another reason than the limit: it is not
    /// repeated until the daemon restarts.
    gave_up: bool,
    /// Notes this working copy has already had.
    noted: HashSet<&'static str>,
}

pub(super) struct LiveAgents {
    holder: Holder,
    roots: Mutex<HashMap<PathBuf, Root>>,
    sweeping: Mutex<bool>,
}

/// The live agents of this daemon process.
pub(super) fn live_agents() -> &'static LiveAgents {
    static AGENTS: OnceLock<LiveAgents> = OnceLock::new();
    AGENTS.get_or_init(|| LiveAgents::new(Holder::of_this_process()))
}

impl LiveAgents {
    fn new(holder: Holder) -> Self {
        Self {
            holder,
            roots: Mutex::new(HashMap::new()),
            sweeping: Mutex::new(false),
        }
    }

    fn roots(&self) -> std::sync::MutexGuard<'_, HashMap<PathBuf, Root>> {
        self.roots.lock().unwrap_or_else(|error| error.into_inner())
    }

    /// A runner command of `root` starts: the working copy is in use, and a
    /// long command does not make its agent look idle.
    pub(super) fn touch(&self, root: &Path, now: Instant) {
        if let Ok(root) = normalize_path_identity(root) {
            self.roots().entry(root).or_default().last_used = Some(now);
        }
    }

    /// After a runner command of `root` whose answer named its agent
    /// (`data.agent`): says what the answer adds and what Unica does with live
    /// agents. Unica holds the agent when it stays and Unica is not among its
    /// holders; when the limit kept the command from an agent, it lets go of
    /// the least recently needed agent of another working copy.
    pub(super) fn after_command(
        &self,
        root: &Path,
        program: &Path,
        agent: &serde_json::Value,
        now: Instant,
    ) -> AfterCommand {
        let Ok(root) = normalize_path_identity(root) else {
            return AfterCommand {
                reply: None,
                marks: Vec::new(),
            };
        };
        let kept = agent["kept"] == true;
        let reason = agent["reason"].as_str();
        let mut roots = self.roots();
        let entry = roots.entry(root.clone()).or_default();
        entry.program = Some(program.to_path_buf());
        entry.last_used = Some(now);
        // Only a receipt of the live agent names its holders: a command that
        // ran with a one-off agent says nothing about the held one.
        if kept || agent["reused"] == true || reason == Some("removed") {
            entry.held = agent["heldByUnica"] == true;
        }
        let hold = kept && !entry.held && !entry.gave_up;
        let reply = match reason {
            Some("limit") => Some(AgentReply::Warning(serde_json::json!({
                "code": NOT_HELD_CODE,
                "message": NOT_HELD,
            }))),
            Some("live_agent_off") if entry.noted.insert(OFF_NOTE) => {
                Some(AgentReply::Note(OFF_NOTE))
            }
            Some("job") if entry.noted.insert(JOB_NOTE) => Some(AgentReply::Note(JOB_NOTE)),
            _ => None,
        };
        let mut marks = Vec::new();
        // Another working copy of Unica takes a place this one needs.
        if hold || reason == Some("limit") {
            if let Some(other) = least_recently_used(&roots, &root, LIMIT - 1) {
                marks.push((other, Mark::Release));
            }
        }
        if hold {
            marks.push((root, Mark::Hold));
        }
        AfterCommand { reply, marks }
    }

    /// Runs the marks of [`Self::after_command`]. Returns whether Unica holds
    /// an agent: then the sweep is needed ([`Self::keep_sweeping`]).
    pub(super) fn perform(&self, runner: &dyn ProcessRunner, marks: &[(PathBuf, Mark)]) -> bool {
        for (root, mark) in marks {
            let result = self.mark(runner, root, *mark);
            if *mark == Mark::Hold {
                let mut roots = self.roots();
                let entry = roots.entry(root.clone()).or_default();
                match result {
                    Ok(()) => entry.held = true,
                    // A refusal other than the limit is not repeated; the limit
                    // or a hold that did not finish may find room later.
                    Err(Some(code)) if code != "agent_limit" => entry.gave_up = true,
                    Err(_) => {}
                }
            }
        }
        self.holding()
    }

    fn holding(&self) -> bool {
        self.roots().values().any(|root| root.held)
    }

    /// An editor session of `root` opened: while it lasts, the held agent of
    /// the working copy is not let go for idleness. Nothing is started.
    pub(super) fn editor_session_opened(&self, root: &Path) {
        let Ok(root) = normalize_path_identity(root) else {
            return;
        };
        self.roots().entry(root).or_default().editor_sessions += 1;
    }

    /// An editor session of `root` closed. With the last one, the held agent
    /// of the working copy is to be let go: returns the working copy for
    /// [`Self::release`].
    pub(super) fn editor_session_closed(&self, root: &Path) -> Option<PathBuf> {
        let root = normalize_path_identity(root).ok()?;
        let mut roots = self.roots();
        let entry = roots.get_mut(&root)?;
        entry.editor_sessions = entry.editor_sessions.saturating_sub(1);
        (entry.editor_sessions == 0 && entry.held).then_some(root)
    }

    /// Lets go of the held agent of `root`.
    pub(super) fn release(&self, runner: &dyn ProcessRunner, root: &Path) {
        let _ = self.mark(runner, root, Mark::Release);
    }

    /// Lets go of the agents of working copies idle for [`IDLE`] without
    /// editor sessions. Returns whether Unica still holds an agent.
    pub(super) fn sweep(&self, runner: &dyn ProcessRunner, now: Instant) -> bool {
        let idle: Vec<PathBuf> = self
            .roots()
            .iter()
            .filter(|(_, root)| {
                root.held
                    && root.editor_sessions == 0
                    && root
                        .last_used
                        .is_none_or(|used| now.saturating_duration_since(used) >= IDLE)
            })
            .map(|(path, _)| path.clone())
            .collect();
        for root in idle {
            let _ = self.mark(runner, &root, Mark::Release);
        }
        self.holding()
    }

    /// Starts the background sweep once Unica holds an agent; it stops when
    /// Unica holds none. Whether to stop is decided under the same lock that
    /// starts it, so an agent held meanwhile is swept.
    pub(super) fn keep_sweeping(&'static self) {
        let mut sweeping = self
            .sweeping
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if *sweeping {
            return;
        }
        *sweeping = true;
        let started = std::thread::Builder::new()
            .name("unica-live-agent-sweep".to_string())
            .spawn(move || loop {
                std::thread::sleep(Duration::from_secs(60));
                self.sweep(&PinnedRunner::default(), Instant::now());
                let mut sweeping = self
                    .sweeping
                    .lock()
                    .unwrap_or_else(|error| error.into_inner());
                if !self.holding() {
                    *sweeping = false;
                    return;
                }
            });
        if started.is_err() {
            *sweeping = false;
        }
    }

    /// Runs `agent hold` or `agent release` of this holder for `root`. A
    /// refusal returns the runner's code.
    fn mark(
        &self,
        runner: &dyn ProcessRunner,
        root: &Path,
        mark: Mark,
    ) -> Result<(), Option<String>> {
        let Some(program) = self.roots().get(root).and_then(|root| root.program.clone()) else {
            return Err(None);
        };
        let mut args = vec![
            "--config".to_string(),
            root.join("v8project.yaml").display().to_string(),
            "--json-message".to_string(),
            "agent".to_string(),
            match mark {
                Mark::Hold => "hold",
                Mark::Release => "release",
            }
            .to_string(),
        ];
        args.extend(self.holder.args());
        let output = runner.run(&ProcessCommand {
            program,
            args,
            cwd: root.to_path_buf(),
            env: Vec::new(),
            env_remove: Vec::new(),
            capture_limits: Some((
                super::v13_infobase_exports::RUNNER_OUTPUT_LIMIT,
                super::v13_infobase_exports::RUNNER_OUTPUT_LIMIT,
            )),
            timeout: Some(MARK_TIMEOUT),
            cancellation: CancellationToken::new(),
        });
        let envelope: Option<serde_json::Value> = output
            .as_ref()
            .ok()
            .and_then(|output| serde_json::from_str(output.stdout.trim()).ok());
        let done = output.is_ok_and(|output| output.status_success)
            && envelope
                .as_ref()
                .is_some_and(|envelope| envelope["ok"] == true);
        if mark == Mark::Release {
            if let Some(entry) = self.roots().get_mut(root) {
                // A release that failed leaves nothing Unica can do: the runner
                // removes the mark when the daemon exits.
                entry.held = false;
            }
        }
        if done {
            Ok(())
        } else {
            Err(envelope
                .and_then(|envelope| envelope["error"]["code"].as_str().map(str::to_string)))
        }
    }
}

/// The working copy whose held agent Unica needed least recently, other than
/// `root` and without editor sessions, when Unica holds more than `keep`
/// agents of other working copies.
fn least_recently_used(
    roots: &HashMap<PathBuf, Root>,
    root: &Path,
    keep: usize,
) -> Option<PathBuf> {
    let others: Vec<(&PathBuf, &Root)> = roots
        .iter()
        .filter(|(path, other)| path.as_path() != root && other.held)
        .collect();
    if others.len() <= keep {
        return None;
    }
    others
        .into_iter()
        .filter(|(_, other)| other.editor_sessions == 0)
        .min_by_key(|(_, other)| other.last_used)
        .map(|(path, _)| path.clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use unica_process::runner::ProcessOutput;

    /// The runner of the tests: answers each `agent hold` and `agent release`
    /// with the next envelope and records the working copy and the command.
    #[derive(Default)]
    struct Runner {
        answers: Mutex<Vec<serde_json::Value>>,
        calls: Mutex<Vec<(PathBuf, String, Vec<String>)>>,
    }

    impl Runner {
        fn answering(answers: Vec<serde_json::Value>) -> Self {
            Self {
                answers: Mutex::new(answers),
                calls: Mutex::default(),
            }
        }

        fn calls(&self) -> Vec<(PathBuf, String)> {
            self.calls
                .lock()
                .unwrap()
                .iter()
                .map(|(root, mark, _)| (root.clone(), mark.clone()))
                .collect()
        }
    }

    impl ProcessRunner for Runner {
        fn run(&self, command: &ProcessCommand) -> Result<ProcessOutput, String> {
            let mark = command.args[4].clone();
            self.calls
                .lock()
                .unwrap()
                .push((command.cwd.clone(), mark, command.args.clone()));
            let mut answers = self.answers.lock().unwrap();
            let answer = if answers.is_empty() {
                serde_json::json!({"ok": true, "data": {}})
            } else {
                answers.remove(0)
            };
            Ok(ProcessOutput {
                status_success: answer["ok"] == true,
                status: String::new(),
                stdout: answer.to_string(),
                stderr: String::new(),
                timed_out: false,
                cancelled: false,
                stdout_truncated: false,
                stderr_truncated: false,
                stdout_had_invalid_utf8: false,
                stderr_had_invalid_utf8: false,
            })
        }
    }

    fn agents() -> LiveAgents {
        LiveAgents::new(Holder {
            pid: 4242,
            started_at: Some("2026-11-13T08:00:00+00:00".to_string()),
        })
    }

    fn root() -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = normalize_path_identity(dir.path()).unwrap();
        (dir, path)
    }

    fn kept(held_by_unica: bool) -> serde_json::Value {
        serde_json::json!({"reused": true, "kept": true, "holders": 0, "heldByUnica": held_by_unica})
    }

    fn one_off(reason: &str) -> serde_json::Value {
        serde_json::json!({
            "reused": false, "kept": false, "holders": 0, "heldByUnica": false, "reason": reason
        })
    }

    /// A command of `root` ends: what its answer adds, with the marks done.
    fn command(
        agents: &LiveAgents,
        runner: &Runner,
        root: &Path,
        agent: serde_json::Value,
        now: Instant,
    ) -> Option<AgentReply> {
        let after = agents.after_command(root, Path::new("v8-runner"), &agent, now);
        agents.perform(runner, &after.marks);
        after.reply
    }

    fn held(agents: &LiveAgents, root: &Path) -> bool {
        agents.roots().get(root).is_some_and(|root| root.held)
    }

    /// Unica держит агента, который остался после её команды и которого она
    /// ещё не держит: `agent hold` с процессом демона и временем его старта.
    /// Агента, которого она уже держит или который не остался, не трогает.
    #[test]
    fn a_kept_agent_is_held_once_by_the_daemon() {
        let (_dir, root) = root();
        let agents = agents();
        let runner = Runner::default();
        let now = Instant::now();
        let after = agents.after_command(&root, Path::new("v8-runner"), &kept(false), now);
        assert_eq!(after.reply, None);
        assert_eq!(after.marks, vec![(root.clone(), Mark::Hold)]);
        assert!(agents.perform(&runner, &after.marks));
        assert!(held(&agents, &root));
        let calls = runner.calls.lock().unwrap().clone();
        assert_eq!(calls.len(), 1);
        let (cwd, mark, args) = &calls[0];
        assert_eq!((cwd, mark.as_str()), (&root, "hold"));
        assert_eq!(args[0], "--config");
        assert_eq!(Path::new(&args[1]), root.join("v8project.yaml"));
        assert_eq!(
            &args[2..],
            [
                "--json-message",
                "agent",
                "hold",
                "--holder-pid",
                "4242",
                "--holder-started-at",
                "2026-11-13T08:00:00+00:00"
            ]
        );

        command(&agents, &runner, &root, kept(true), now);
        command(&agents, &runner, &root, one_off("live_agent_off"), now);
        assert_eq!(runner.calls().len(), 1);
    }

    /// Команда с разовым агентом (агент занят, сторож не ответил, команду
    /// прервали) о живом агенте не говорит: удержанный остаётся удержанным.
    #[test]
    fn a_one_off_command_keeps_the_held_agent_held() {
        let (_dir, root) = root();
        let agents = agents();
        let runner = Runner::default();
        let now = Instant::now();
        command(&agents, &runner, &root, kept(false), now);
        for reason in ["busy", "keeper_failed", "interrupted"] {
            command(&agents, &runner, &root, one_off(reason), now);
            assert!(held(&agents, &root), "{reason}");
        }
        // Сторож снял агента после команды: удержания больше нет.
        command(&agents, &runner, &root, one_off("removed"), now);
        assert!(!held(&agents, &root));
    }

    /// Мест у живых агентов два: третью рабочую копию Unica держит, отпустив
    /// давно не нужную, а копию с открытым редактором не отпускает.
    #[test]
    fn a_third_working_copy_takes_the_place_of_the_least_recently_used() {
        let ((_a, a), (_b, b), (_c, c)) = (root(), root(), root());
        let agents = agents();
        let runner = Runner::default();
        let start = Instant::now();
        command(&agents, &runner, &a, kept(false), start);
        command(
            &agents,
            &runner,
            &b,
            kept(false),
            start + Duration::from_secs(1),
        );
        agents.editor_session_opened(&a);
        command(
            &agents,
            &runner,
            &c,
            kept(false),
            start + Duration::from_secs(2),
        );
        assert!(held(&agents, &c));
        assert_eq!(
            runner.calls(),
            vec![
                (a.clone(), "hold".to_string()),
                (b.clone(), "hold".to_string()),
                (b.clone(), "release".to_string()),
                (c.clone(), "hold".to_string()),
            ]
        );
    }

    /// Предел не дал команде агента: ответ предупреждает, а Unica, если места
    /// заняты её агентами, освобождает давно не нужное для следующей команды.
    #[test]
    fn the_limit_warns_and_makes_room_for_the_next_command() {
        let ((_a, a), (_b, b), (_c, c)) = (root(), root(), root());
        let agents = agents();
        let runner = Runner::default();
        let start = Instant::now();
        command(&agents, &runner, &a, kept(false), start);
        command(
            &agents,
            &runner,
            &b,
            kept(false),
            start + Duration::from_secs(1),
        );
        let reply = command(
            &agents,
            &runner,
            &c,
            one_off("limit"),
            start + Duration::from_secs(2),
        );
        assert_eq!(
            reply,
            Some(AgentReply::Warning(serde_json::json!({
                "code": "live_agent_not_held",
                "message": NOT_HELD,
            })))
        );
        assert!(!held(&agents, &c));
        assert_eq!(
            runner.calls().last(),
            Some(&(a.clone(), "release".to_string()))
        );
    }

    /// Выключенный живой агент и Job без выхода — заметка, один раз на рабочую
    /// копию.
    #[test]
    fn off_and_job_are_noted_once() {
        let (_dir, root) = root();
        let agents = agents();
        let runner = Runner::default();
        let now = Instant::now();
        for (reason, note) in [("live_agent_off", OFF_NOTE), ("job", JOB_NOTE)] {
            let first = command(&agents, &runner, &root, one_off(reason), now);
            assert_eq!(first, Some(AgentReply::Note(note)));
            assert_eq!(command(&agents, &runner, &root, one_off(reason), now), None);
        }
        assert!(runner.calls().is_empty());
    }

    /// Отказ `agent hold` команду не роняет. Предел и незавершённый hold Unica
    /// пробует снова на следующей команде, иной отказ — уже нет, до перезапуска
    /// демона.
    #[test]
    fn a_refused_hold_is_retried_unless_the_runner_refused_it() {
        let (_dir, root) = root();
        let agents = agents();
        let now = Instant::now();
        let refused = |code: &str| serde_json::json!({"ok": false, "error": {"code": code}});
        let runner = Runner::answering(vec![
            refused("agent_limit"),
            serde_json::json!({"ok": false}),
            refused("environment_unavailable"),
        ]);
        for _ in 0..4 {
            command(&agents, &runner, &root, kept(false), now);
            assert!(!held(&agents, &root));
        }
        assert_eq!(runner.calls().len(), 3);
    }

    /// Простой без команд и без редактора отпускает агента; открытый редактор
    /// держит его, а закрытие последней сессии редактора отпускает. Начатая
    /// команда — не простой.
    #[test]
    fn idleness_and_the_last_editor_session_let_the_agent_go() {
        let ((_a, a), (_b, b)) = (root(), root());
        let agents = agents();
        let runner = Runner::default();
        let start = Instant::now();
        command(&agents, &runner, &a, kept(false), start);
        command(&agents, &runner, &b, kept(false), start);
        agents.editor_session_opened(&b);
        agents.editor_session_opened(&b);
        assert!(agents.sweep(&runner, start + IDLE - Duration::from_secs(1)));
        assert_eq!(runner.calls().len(), 2);
        // A command of `a` started and is still running.
        agents.touch(&a, start + IDLE - Duration::from_secs(1));
        assert!(agents.sweep(&runner, start + IDLE));
        assert_eq!(runner.calls().len(), 2);
        assert!(agents.sweep(&runner, start + IDLE * 2));
        assert_eq!(
            runner.calls().last(),
            Some(&(a.clone(), "release".to_string()))
        );
        assert_eq!(agents.editor_session_closed(&b), None);
        assert_eq!(agents.editor_session_closed(&b), Some(b.clone()));
        agents.release(&runner, &b);
        assert!(!agents.sweep(&runner, start + IDLE * 2));
        assert_eq!(
            runner.calls().last(),
            Some(&(b.clone(), "release".to_string()))
        );
    }
}
