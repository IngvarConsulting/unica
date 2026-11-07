//! След попытки запуска, переживающий убийство снаружи.
//!
//! Убитый процесс не печатает ничего: провода ещё нет, деструкторы не бегут,
//! поток ошибок закрыт вместе с деревом процессов. Поэтому запись открывается
//! **до** начала стадии и закрывается только по её исходу. Всё, что осталось
//! незакрытым, — это попытка, которую убили, и о ней сообщает следующий запуск.
//!
//! Формат — JSON Lines: строка на событие, дописывание в конец. Компактация
//! публикует отдельный файл атомарной заменой; исходный журнал не обрезается.

use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use fs2::FileExt;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::error::{BootstrapError, Failure, Result};

/// Стадия установки, которую может не пережить процесс.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Stage {
    /// Байты едут по каналу.
    Download,
    /// Сумма архива сверяется с манифестом.
    Verify,
    /// Архив разворачивается в рабочий каталог.
    Extract,
    /// Готовая установка подменяет прежнюю.
    Publish,
}

impl Stage {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Download => "downloading",
            Self::Verify => "verifying",
            Self::Extract => "extracting",
            Self::Publish => "publishing",
        }
    }
}

/// Что доставляет попытка.
#[derive(Clone, Debug)]
pub struct AttemptSubject {
    pub artifact: String,
    pub version: String,
    pub target: String,
    pub url: String,
    /// Недокачка: по её размеру видно, сколько успело приехать.
    pub partial: PathBuf,
}

/// Журнал попыток внутри кеша артефактов.
///
/// Файл на артефакт, версию и цель. Короткая отдельная блокировка журнала
/// сериализует установщик и подтверждение доставки сообщения рантаймом.
pub struct AttemptLog {
    root: PathBuf,
}

/// Открытая попытка. Закрывается вызовом `finished` или `failed`; всё
/// остальное — включая уничтожение процесса — оставляет её открытой.
///
/// `Drop` намеренно ничего не делает: убитый процесс деструкторов не выполняет,
/// и запись, закрывающаяся сама, солгала бы ровно в том случае, ради которого
/// заведена.
pub struct OpenAttempt {
    path: PathBuf,
    id: String,
}

/// Попытка, которую никто не закрыл.
#[derive(Clone, Debug)]
pub struct UnfinishedAttempt {
    /// Чем отмечают попытку, о которой рассказали.
    pub id: String,
    pub artifact: String,
    pub version: String,
    pub target: String,
    pub url: String,
    pub stage: Stage,
    pub started: SystemTime,
    /// Сколько байтов лежит в недокачке на диске.
    pub received: u64,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", tag = "event")]
enum Event {
    Started {
        attempt: String,
        at: u64,
        artifact: String,
        version: String,
        target: String,
        url: String,
        partial: String,
        stage: Stage,
    },
    Reached {
        attempt: String,
        at: u64,
        stage: Stage,
    },
    Closed {
        attempt: String,
        at: u64,
        outcome: String,
    },
    /// Рантайм рассказал о попытке по проводу. Пишет эту строку он, тем же
    /// форматом и тем же дописыванием: иначе один и тот же убитый запуск
    /// повторялся бы каждую сессию.
    Reported { attempt: String, at: u64 },
}

impl Event {
    fn attempt(&self) -> &str {
        match self {
            Self::Started { attempt, .. }
            | Self::Reached { attempt, .. }
            | Self::Closed { attempt, .. }
            | Self::Reported { attempt, .. } => attempt,
        }
    }
}

impl AttemptLog {
    pub fn in_cache(cache_root: &Path) -> Self {
        Self {
            root: cache_root.join(".attempts"),
        }
    }

    /// Открыть запись перед началом первой стадии.
    ///
    /// Рассказанные попытки при этом уходят: закрытые сказали о себе кодом
    /// выхода и потоком ошибок, отмеченные — по проводу. Держать их значит
    /// растить журнал без пользы.
    pub fn open(&self, subject: AttemptSubject) -> Result<OpenAttempt> {
        let path = self.path_for(&subject.artifact, &subject.version, &subject.target);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let journal = JournalLock::acquire(&path)?;
        journal.compact(|| {})?;

        let id = Uuid::new_v4().to_string();
        append(
            &path,
            &Event::Started {
                attempt: id.clone(),
                at: now_ms(),
                artifact: subject.artifact,
                version: subject.version,
                target: subject.target,
                url: subject.url,
                partial: subject.partial.to_string_lossy().into_owned(),
                stage: Stage::Download,
            },
        )?;
        Ok(OpenAttempt { path, id })
    }

    /// Попытки, которых никто не закрыл, — то есть убитые снаружи.
    pub fn unfinished(&self) -> Result<Vec<UnfinishedAttempt>> {
        let mut found = Vec::new();
        for path in self.logs()? {
            let _journal = JournalLock::acquire(&path)?;
            found.extend(unfinished_in(&path)?);
        }
        found.sort_by_key(|attempt| attempt.started);
        Ok(found)
    }

    /// Отметить попытки рассказанными.
    ///
    /// Отметка нужна, потому что закрыть их некому: тот, кто мог бы, убит. Без
    /// неё один и тот же убитый запуск повторялся бы каждую сессию.
    ///
    /// Отдельно от чтения потому, что между ними стоит установка: читать надо
    /// до неё, пока недокачка на диске, а отмечать — когда рассказ ушёл.
    pub fn report(&self, attempts: &[UnfinishedAttempt]) -> Result<()> {
        let ids = attempts
            .iter()
            .map(|attempt| attempt.id.clone())
            .collect::<Vec<_>>();
        self.report_ids(&ids)
    }

    /// Подтвердить только идентификаторы попыток из переданного сообщения.
    /// Новые попытки, появившиеся после снимка bootstrap, не подтверждаются.
    pub fn report_ids(&self, ids: &[String]) -> Result<()> {
        if ids.is_empty() {
            return Ok(());
        }
        for path in self.logs()? {
            let _journal = JournalLock::acquire(&path)?;
            for attempt in unfinished_in(&path)? {
                if ids.contains(&attempt.id) {
                    append(
                        &path,
                        &Event::Reported {
                            attempt: attempt.id,
                            at: now_ms(),
                        },
                    )?;
                }
            }
        }
        Ok(())
    }

    fn path_for(&self, artifact: &str, version: &str, target: &str) -> PathBuf {
        self.root
            .join(artifact)
            .join(format!("{version}-{target}.jsonl"))
    }

    fn logs(&self) -> Result<Vec<PathBuf>> {
        let Ok(artifacts) = fs::read_dir(&self.root) else {
            return Ok(Vec::new());
        };
        let mut found = Vec::new();
        for artifact in artifacts.flatten() {
            let Ok(entries) = fs::read_dir(artifact.path()) else {
                continue;
            };
            for entry in entries.flatten() {
                if entry.path().extension().is_some_and(|kind| kind == "jsonl") {
                    found.push(entry.path());
                }
            }
        }
        found.sort();
        Ok(found)
    }
}

impl OpenAttempt {
    /// Перейти к следующей стадии.
    pub fn reached(&self, stage: Stage) -> Result<()> {
        let _journal = JournalLock::acquire(&self.path)?;
        append(
            &self.path,
            &Event::Reached {
                attempt: self.id.clone(),
                at: now_ms(),
                stage,
            },
        )
    }

    pub fn finished(self) -> Result<()> {
        self.close("finished")
    }

    /// Отказ, который bootstrap заметил сам: о нём уже сказано наружу.
    pub fn failed(self, failure: Failure, message: &str) -> Result<()> {
        self.close(&format!("{}: {message}", failure.reason()))
    }

    fn close(self, outcome: &str) -> Result<()> {
        let journal = JournalLock::acquire(&self.path)?;
        append(
            &self.path,
            &Event::Closed {
                attempt: self.id.clone(),
                at: now_ms(),
                outcome: outcome.to_owned(),
            },
        )?;
        // Прибираем здесь же: удачная установка не должна оставлять следов
        // вовсе. Убитая соседняя попытка это переживает — её никто не закрывал.
        journal.compact(|| {})
    }
}

/// Рассказ о том, чего никто не закрыл: место, состояние, лечение.
///
/// Попытки одного артефакта сходятся в один абзац. Три убийства подряд на
/// медленном канале — обычное дело, и три одинаковых абзаца читателю ничего не
/// добавляют; недокачка у них всё равно общая, и объём в ней один.
pub fn diagnose(attempts: &[UnfinishedAttempt]) -> Option<String> {
    if attempts.is_empty() {
        return None;
    }
    let mut told: Vec<(&UnfinishedAttempt, usize)> = Vec::new();
    for attempt in attempts {
        match told
            .iter_mut()
            .find(|(known, _)| known.same_subject(attempt))
        {
            // Свежайшая попытка представляет группу: её возраст ближе всего к
            // тому, что читатель только что пережил.
            Some((known, count)) => {
                if attempt.started > known.started {
                    *known = attempt;
                }
                *count += 1;
            }
            None => told.push((attempt, 1)),
        }
    }
    Some(
        told.iter()
            .map(|(attempt, count)| attempt.diagnosis(*count))
            .collect::<Vec<_>>()
            .join("\n"),
    )
}

impl UnfinishedAttempt {
    fn same_subject(&self, other: &Self) -> bool {
        self.artifact == other.artifact
            && self.version == other.version
            && self.target == other.target
    }

    /// Причина, место, состояние и лечение для того, кто это прочитает.
    fn diagnosis(&self, kills: usize) -> String {
        let age = SystemTime::now()
            .duration_since(self.started)
            .map(|elapsed| format!("{} s ago", elapsed.as_secs()))
            .unwrap_or_else(|_| "just now".to_owned());
        let headline = if kills == 1 {
            format!(
                "a Unica startup was killed {age} while {} {} {} for {}",
                self.stage.as_str(),
                self.artifact,
                self.version,
                self.target
            )
        } else {
            format!(
                "{kills} Unica startups were killed while {} {} {} for {}, the last {age}",
                self.stage.as_str(),
                self.artifact,
                self.version,
                self.target
            )
        };
        format!(
            "{headline}\n  place: {url}\n  state: {received} bytes are on disk\n  \
             cure: nothing to repair — the next call resumes from those bytes",
            url = self.url,
            received = self.received
        )
    }
}

fn append(path: &Path, event: &Event) -> Result<()> {
    let mut line = serde_json::to_string(event)?;
    line.push('\n');
    // Дописывание, а не перезапись, и без принудительной синхронизации: байты
    // уходят ядру тем же вызовом, а убийство процесса их не отменяет.
    let mut file = OpenOptions::new()
        .create(true)
        .read(true)
        .append(true)
        .open(path)?;
    let len = file.metadata()?.len();
    if len > 0 {
        file.seek(SeekFrom::Start(len - 1))?;
        let mut last = [0];
        file.read_exact(&mut last)?;
        if last[0] != b'\n' {
            // Keep a killed writer's incomplete event separate from the next
            // complete one. The parser can then discard only the torn line.
            file.write_all(b"\n")?;
        }
    }
    file.write_all(line.as_bytes())?;
    Ok(())
}

/// Попытка рассказана: её закрыл сам bootstrap или о ней сообщил рантайм.
fn told_about(event: &Event, attempt: &str) -> bool {
    match event {
        Event::Closed { attempt: told, .. } | Event::Reported { attempt: told, .. } => {
            told == attempt
        }
        _ => false,
    }
}

/// Файл блокировки не заменяется и не удаляется вместе с журналом: иначе
/// два процесса могли бы запереть разные файловые объекты под одним именем.
struct JournalLock {
    path: PathBuf,
    _file: File,
}

impl JournalLock {
    fn acquire(path: &Path) -> Result<Self> {
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(path.with_extension("lock"))?;
        file.lock_exclusive()?;
        Ok(Self {
            path: path.to_path_buf(),
            _file: file,
        })
    }

    fn compact(&self, before_publication: impl FnOnce()) -> Result<()> {
        compact_locked(&self.path, before_publication)
    }
}

#[cfg(test)]
fn compact_before_publication(path: &Path, before_publication: impl FnOnce()) -> Result<()> {
    JournalLock::acquire(path)?.compact(before_publication)
}

/// Вызывается только владельцем JournalLock.
fn compact_locked(path: &Path, before_publication: impl FnOnce()) -> Result<()> {
    let events = read_events(path)?;
    let told = events
        .iter()
        .filter_map(|event| match event {
            Event::Closed { attempt, .. } | Event::Reported { attempt, .. } => {
                Some(attempt.as_str())
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    if told.is_empty() {
        return Ok(());
    }
    let survivors = events
        .iter()
        .filter(|event| !told.contains(&event.attempt()))
        .map(serde_json::to_string)
        .collect::<std::result::Result<Vec<_>, _>>()?;
    if survivors.is_empty() {
        before_publication();
        fs::remove_file(path)?;
        return Ok(());
    }
    let mut text = survivors.join("\n");
    text.push('\n');
    let temporary = path.with_extension(format!("{}.tmp", Uuid::new_v4()));
    let staged = StagedJournal(temporary);
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&staged.0)?;
    file.write_all(text.as_bytes())?;
    file.sync_all()?;
    // Close before rename, including on Windows. A killed writer leaves the
    // old journal intact until the complete replacement is published.
    drop(file);
    before_publication();
    fs::rename(&staged.0, path)?;
    Ok(())
}

struct StagedJournal(PathBuf);

impl Drop for StagedJournal {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

fn unfinished_in(path: &Path) -> Result<Vec<UnfinishedAttempt>> {
    let events = read_events(path)?;
    let mut found = Vec::new();
    for event in &events {
        let Event::Started {
            attempt,
            at,
            artifact,
            version,
            target,
            url,
            partial,
            stage,
        } = event
        else {
            continue;
        };
        if events.iter().any(|other| told_about(other, attempt)) {
            continue;
        }
        let stage = events
            .iter()
            .filter_map(|other| match other {
                Event::Reached {
                    attempt: reached,
                    stage,
                    ..
                } if reached == attempt => Some(*stage),
                _ => None,
            })
            .next_back()
            .unwrap_or(*stage);
        found.push(UnfinishedAttempt {
            id: attempt.clone(),
            artifact: artifact.clone(),
            version: version.clone(),
            target: target.clone(),
            url: url.clone(),
            stage,
            started: UNIX_EPOCH + std::time::Duration::from_millis(*at),
            received: fs::metadata(partial).map(|meta| meta.len()).unwrap_or(0),
        });
    }
    Ok(found)
}

/// Строки, которые разобрались. Испорченная строка — не повод потерять журнал:
/// её писал процесс, которого убили посреди записи.
fn read_events(path: &Path) -> Result<Vec<Event>> {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        // Another owner may have compacted the journal after logs() listed it.
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => {
            return Err(BootstrapError::of(
                Failure::Disk,
                format!("failed to read the attempt log {}: {error}", path.display()),
            ))
        }
    };
    Ok(bytes
        .split(|byte| *byte == b'\n')
        .filter_map(|line| serde_json::from_slice::<Event>(line).ok())
        .collect())
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|since| since.as_millis() as u64)
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::fs;
    use std::path::PathBuf;

    fn cache(name: &str) -> PathBuf {
        let path =
            std::env::temp_dir().join(format!("unica-attempt-{name}-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&path).expect("cache root");
        path
    }

    fn subject(root: &Path) -> AttemptSubject {
        AttemptSubject {
            artifact: "unica".to_owned(),
            version: "0.13.0".to_owned(),
            target: "darwin-arm64".to_owned(),
            url: "https://github.com/IngvarConsulting/unica/releases/download/v0.13.0/unica-runtime-darwin-arm64.tar.gz"
                .to_owned(),
            partial: root.join(".partial/unica/0.13.0-darwin-arm64.tar.gz"),
        }
    }

    /// rule: INV.PKG.INSTALL-ATTEMPT-LOG
    #[test]
    fn an_attempt_is_discoverable_while_its_stage_runs() {
        // Убить процесс могут в любой момент стадии, поэтому запись открывается
        // до её начала, а не по её завершении.
        let root = cache("open");
        let log = AttemptLog::in_cache(&root);
        let _attempt = log.open(subject(&root)).expect("open the attempt");

        let unfinished = log.unfinished().expect("read the log");

        assert_eq!(unfinished.len(), 1);
        assert_eq!(unfinished[0].artifact, "unica");
        assert_eq!(unfinished[0].version, "0.13.0");
        assert_eq!(unfinished[0].target, "darwin-arm64");
        assert_eq!(unfinished[0].stage, Stage::Download);
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn a_finished_attempt_is_not_reported() {
        let root = cache("finished");
        let log = AttemptLog::in_cache(&root);

        log.open(subject(&root))
            .expect("open")
            .finished()
            .expect("close");

        assert!(log.unfinished().expect("read the log").is_empty());
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn a_failure_the_bootstrap_noticed_closes_the_attempt_too() {
        // О замеченном отказе уже сказано в поток ошибок и кодом выхода.
        // Повторять его следующей сессией значит показывать одно дважды.
        let root = cache("failed");
        let log = AttemptLog::in_cache(&root);

        log.open(subject(&root))
            .expect("open")
            .failed(Failure::Network, "connection refused")
            .expect("close");

        assert!(log.unfinished().expect("read the log").is_empty());
        fs::remove_dir_all(&root).ok();
    }

    /// rule: INV.PKG.INSTALL-ATTEMPT-LOG
    #[test]
    fn a_new_attempt_does_not_erase_the_one_nobody_closed() {
        // Убитый процесс не выполняет деструкторов: брошенная запись — это и
        // есть убийство снаружи.
        let root = cache("survives");
        let log = AttemptLog::in_cache(&root);

        drop(log.open(subject(&root)).expect("killed attempt"));
        log.open(subject(&root))
            .expect("open")
            .finished()
            .expect("close");

        assert_eq!(
            log.unfinished().expect("read the log").len(),
            1,
            "убитая попытка переживает следующую, пока о ней не сообщили"
        );
        fs::remove_dir_all(&root).ok();
    }

    /// rule: INV.PKG.INSTALL-ATTEMPT-LOG
    #[test]
    fn the_received_volume_is_read_from_the_partial_on_disk() {
        // Убитый процесс не успевает записать, сколько получил. Недокачка на
        // диске отвечает точнее любой записи по ходу — и не стоит обращений.
        let root = cache("received");
        let log = AttemptLog::in_cache(&root);
        let subject = subject(&root);
        fs::create_dir_all(subject.partial.parent().expect("partial root")).expect("partial root");
        fs::write(&subject.partial, vec![7_u8; 4096]).expect("partial bytes");

        drop(log.open(subject).expect("killed attempt"));

        assert_eq!(log.unfinished().expect("read the log")[0].received, 4096);
        fs::remove_dir_all(&root).ok();
    }

    /// rule: INV.PKG.INSTALL-ATTEMPT-LOG
    #[test]
    fn the_stage_reported_is_the_last_one_the_attempt_reached() {
        let root = cache("stage");
        let log = AttemptLog::in_cache(&root);
        let attempt = log.open(subject(&root)).expect("open");

        attempt.reached(Stage::Extract).expect("advance");
        drop(attempt);

        assert_eq!(
            log.unfinished().expect("read the log")[0].stage,
            Stage::Extract
        );
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn a_diagnosis_names_the_place_the_state_and_the_cure() {
        let root = cache("diagnosis");
        let log = AttemptLog::in_cache(&root);
        let subject = subject(&root);
        fs::create_dir_all(subject.partial.parent().expect("partial root")).expect("partial root");
        fs::write(&subject.partial, vec![7_u8; 4096]).expect("partial bytes");
        drop(log.open(subject).expect("killed attempt"));

        let diagnosis = diagnose(&log.unfinished().expect("read the log")).expect("a diagnosis");

        assert!(diagnosis.contains("unica 0.13.0"), "предмет: {diagnosis}");
        assert!(diagnosis.contains("darwin-arm64"), "цель: {diagnosis}");
        assert!(diagnosis.contains("place:"), "место: {diagnosis}");
        assert!(diagnosis.contains("4096 bytes"), "состояние: {diagnosis}");
        assert!(diagnosis.contains("cure:"), "лечение: {diagnosis}");
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn repeated_kills_of_one_artifact_are_told_once() {
        // Три убийства подряд — обычное дело на медленном канале. Три
        // одинаковых абзаца об одном и том же читателю ничего не добавляют, а
        // объём у них общий: недокачка на диске одна.
        let root = cache("repeated");
        let log = AttemptLog::in_cache(&root);
        for _ in 0..3 {
            drop(log.open(subject(&root)).expect("killed attempt"));
        }

        let diagnosis = diagnose(&log.unfinished().expect("read the log")).expect("a diagnosis");

        assert_eq!(
            diagnosis.matches("place:").count(),
            1,
            "об одном артефакте говорят один раз: {diagnosis}"
        );
        assert!(
            diagnosis.contains("3 Unica startups"),
            "число убийств названо: {diagnosis}"
        );
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn nothing_killed_is_nothing_to_tell() {
        assert_eq!(diagnose(&[]), None);
    }

    /// rule: INV.PKG.INSTALL-ATTEMPT-LOG
    /// rule: INV.PKG.STARTUP-NOTICE-RETENTION
    #[test]
    fn compaction_preserves_a_concurrent_ack_and_a_new_attempt() {
        use fs2::FileExt;
        use std::sync::mpsc;

        let root = cache("concurrent-ack");
        let log = AttemptLog::in_cache(&root);
        drop(log.open(subject(&root)).unwrap());
        let old = log.unfinished().unwrap();
        let closed = log.open(subject(&root)).unwrap();
        append(
            &closed.path,
            &Event::Closed {
                attempt: closed.id.clone(),
                at: now_ms(),
                outcome: "finished".into(),
            },
        )
        .unwrap();
        let path = closed.path.clone();
        let worker_root = root.clone();
        let (finished_tx, finished_rx) = mpsc::channel();
        let mut worker = None;
        compact_before_publication(&path, || {
            // A competing writer runs exactly after compaction took its snapshot.
            // If no lock excludes it, finish its writes before publishing that
            // stale snapshot, deterministically reproducing the lost ACK/start.
            let probe = OpenOptions::new()
                .create(true)
                .truncate(false)
                .write(true)
                .open(path.with_extension("lock"))
                .unwrap();
            let excluded = probe.try_lock_exclusive().is_err();
            drop(probe);
            worker = Some(std::thread::spawn(move || {
                let log = AttemptLog::in_cache(&worker_root);
                log.report(&old).unwrap();
                let new = log.open(subject(&worker_root)).unwrap();
                new.reached(Stage::Extract).unwrap();
                finished_tx.send(new.id.clone()).unwrap();
                new.id
            }));
            if !excluded {
                // No timing assertion: receipt proves the competing mutation ran.
                finished_rx
                    .recv_timeout(std::time::Duration::from_secs(5))
                    .unwrap();
            }
        })
        .unwrap();
        let new_id = worker.unwrap().join().unwrap();
        let remaining = log.unfinished().unwrap();
        assert_eq!(remaining.len(), 1, "only the new attempt must remain");
        assert_eq!(
            remaining[0].id, new_id,
            "ACK must not cover the new attempt"
        );
        assert_eq!(remaining[0].stage, Stage::Extract, "new stage must survive");
        fs::remove_dir_all(root).unwrap();
    }

    /// rule: INV.PKG.STARTUP-NOTICE-RETENTION
    #[test]
    fn exact_ids_acknowledge_only_the_snapshot_and_are_idempotent() {
        let root = cache("exact-ids");
        let log = AttemptLog::in_cache(&root);
        let old = log.open(subject(&root)).unwrap();
        let ids = vec![old.id.clone(), old.id, "unknown-attempt".into()];
        let new = log.open(subject(&root)).unwrap();
        let mut other = subject(&root);
        other.version = "0.14.0".into();
        let independent = log.open(other).unwrap();

        log.report_ids(&ids).unwrap();
        let path = new.path.clone();
        let acknowledged = fs::read(&path).unwrap();
        log.report_ids(&ids).unwrap();
        assert_eq!(
            fs::read(&path).unwrap(),
            acknowledged,
            "duplicate ACK must not append"
        );
        let mut remaining = log
            .unfinished()
            .unwrap()
            .into_iter()
            .map(|a| a.id)
            .collect::<Vec<_>>();
        remaining.sort();
        let mut expected = vec![new.id, independent.id];
        expected.sort();
        assert_eq!(remaining, expected);
        fs::remove_dir_all(root).unwrap();
    }

    /// rule: INV.PKG.STARTUP-NOTICE-RETENTION
    #[test]
    fn an_ack_after_a_torn_event_remains_readable() {
        let root = cache("torn-event");
        let log = AttemptLog::in_cache(&root);
        let old = log.open(subject(&root)).unwrap();
        let new = log.open(subject(&root)).unwrap();
        // A killed writer left only the prefix of its last event.
        OpenOptions::new()
            .append(true)
            .open(&old.path)
            .unwrap()
            .write_all(br#"{"event":"reached","attempt":""#)
            .unwrap();
        log.report_ids(&[old.id]).unwrap();
        let remaining = log.unfinished().unwrap();
        assert_eq!(
            remaining.len(),
            1,
            "the exact old ACK must survive the torn event"
        );
        assert_eq!(remaining[0].id, new.id);
        fs::remove_dir_all(root).unwrap();
    }

    /// rule: INV.PKG.INSTALL-ATTEMPT-LOG
    /// rule: INV.PKG.STARTUP-NOTICE-RETENTION
    #[test]
    fn a_torn_utf8_event_does_not_hide_complete_attempts_or_their_ack() {
        let root = cache("torn-utf8");
        let log = AttemptLog::in_cache(&root);
        let old = log.open(subject(&root)).unwrap();
        let new = log.open(subject(&root)).unwrap();
        new.reached(Stage::Extract).unwrap();
        // The last byte is the prefix of a multi-byte UTF-8 path character.
        OpenOptions::new()
            .append(true)
            .open(&old.path)
            .unwrap()
            .write_all(b"{\"event\":\"started\",\"partial\":\"/cache/\xe2")
            .unwrap();
        let before = log
            .unfinished()
            .expect("complete events survive a torn UTF-8 line");
        assert_eq!(before.len(), 2);
        log.open(subject(&root)).unwrap().finished().unwrap();
        let before_ack = log.unfinished().unwrap();
        assert_eq!(
            before_ack.len(),
            2,
            "compaction cannot discard an unacknowledged old attempt"
        );
        assert!(before_ack.iter().any(|attempt| attempt.id == old.id));
        OpenOptions::new()
            .append(true)
            .open(&old.path)
            .unwrap()
            .write_all(b"{\"event\":\"reached\",\"attempt\":\"\xf0\x9f")
            .unwrap();
        log.report_ids(std::slice::from_ref(&old.id)).unwrap();
        // Opening another attempt compacts the acknowledged old attempt, but
        // must retain the complete newer one on either side of the bad line.
        let latest = log.open(subject(&root)).unwrap();
        latest.finished().unwrap();
        let remaining = log.unfinished().unwrap();
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].id, new.id);
        assert_eq!(remaining[0].stage, Stage::Extract);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_journal_read_failure_is_not_treated_as_empty_during_compaction() {
        let root = cache("journal-read-error");
        let log = AttemptLog::in_cache(&root);
        let attempt = log.open(subject(&root)).unwrap();
        fs::remove_file(&attempt.path).unwrap();
        fs::create_dir(&attempt.path).unwrap();
        assert!(
            log.unfinished().is_err(),
            "a disk error is not an empty journal"
        );
        assert!(
            JournalLock::acquire(&attempt.path)
                .unwrap()
                .compact(|| {})
                .is_err(),
            "compaction must propagate the read error"
        );
        fs::remove_dir_all(root).unwrap();
    }

    /// rule: INV.PKG.INSTALL-ATTEMPT-LOG
    /// rule: INV.PKG.STARTUP-NOTICE-RETENTION
    #[test]
    fn medium_interrupted_compaction_keeps_unfinished_attempt() {
        use std::io::{BufRead, BufReader};
        use std::process::{Command, Stdio};
        use std::sync::mpsc;

        const CHILD_ROOT: &str = "UNICA_TEST_ATTEMPT_COMPACTION_ROOT";
        if let Some(root) = std::env::var_os(CHILD_ROOT) {
            let root = PathBuf::from(root);
            let log = AttemptLog::in_cache(&root);
            let s = subject(&root);
            compact_before_publication(&log.path_for(&s.artifact, &s.version, &s.target), || {
                std::io::stdout().write_all(b"journal-staged\n").unwrap();
                std::io::stdout().flush().unwrap();
                // The parent kills this process after observing the staged file,
                // while the original journal and its stable lock are still live.
                let mut byte = [0];
                std::io::stdin().read_exact(&mut byte).unwrap();
            })
            .unwrap();
            panic!("the parent must interrupt compaction before publication");
        }

        struct ChildGuard(std::process::Child);
        impl Drop for ChildGuard {
            fn drop(&mut self) {
                let _ = self.0.kill();
                let _ = self.0.wait();
            }
        }

        let root = cache("compaction-crash");
        let log = AttemptLog::in_cache(&root);
        let alive = log.open(subject(&root)).unwrap();
        alive.reached(Stage::Extract).unwrap();
        let old = log.open(subject(&root)).unwrap();
        log.report_ids(&[old.id]).unwrap();
        let original = fs::read(&alive.path).unwrap();
        let mut child = ChildGuard(
            Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "attempt::tests::medium_interrupted_compaction_keeps_unfinished_attempt",
                    "--nocapture",
                ])
                .env(CHILD_ROOT, &root)
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::inherit())
                .spawn()
                .unwrap(),
        );
        let stdout = child.0.stdout.take().unwrap();
        let (staged_tx, staged_rx) = mpsc::channel();
        let reader = std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                if line.unwrap().contains("journal-staged") {
                    staged_tx.send(()).unwrap();
                    break;
                }
            }
        });
        staged_rx
            .recv_timeout(std::time::Duration::from_secs(10))
            .unwrap();
        child.0.kill().unwrap();
        child.0.wait().unwrap();
        reader.join().unwrap();

        assert_eq!(
            fs::read(&alive.path).unwrap(),
            original,
            "crash must not truncate the original journal"
        );
        assert!(alive.path.with_extension("lock").is_file());
        // A killed compactor must also release the OS lock for the next writer.
        log.open(subject(&root)).unwrap().finished().unwrap();
        let remaining = log.unfinished().unwrap();
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].id, alive.id);
        assert_eq!(remaining[0].stage, Stage::Extract);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_log_nobody_wrote_reports_nothing() {
        let root = cache("empty");

        assert!(AttemptLog::in_cache(&root)
            .unfinished()
            .expect("read the log")
            .is_empty());
        fs::remove_dir_all(&root).ok();
    }
}
