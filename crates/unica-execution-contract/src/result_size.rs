//! Pure bounded canonical-result accounting; no store or epoch clock.
use crate::invocation::DomainResult;
use std::io::{self, Write};

// resource-limit: result.canonical-bytes
// class: result
// unit: bytes
// scope: operation
// value: 8388608
// resource: Размер сериализованного канонического результата операции.
// status: unproven
// origin: коммит 215497a92 (fix(invocation): bound durable result publication)
// problem: неизвестно
// on-reach: закрытая ошибка result_too_large; результат не выдаётся и не сохраняется как успех
// issue: #1119
// evidence: Замер 25.10.2026 (Apple M3 Max, 14 ядер, macOS, git 2.54, релизная сборка ветки поверх main 07306a173, scripts/dev/measure-capacity.py, два прохода подряд, наблюдение ёмкости демона (--capacity-report)): наибольший ответ прогона — 21 694 байта (строка JSON-RPC целиком, верхняя оценка результата; 0,26% предела) (корневой check на выгрузке УТ 11 (48 159 файлов в индексе Git, 12 290 модулей BSL на 508 МБ, 31 009 XML на 692 МБ)); у остальных вызовов 0,4–21 КБ. Операции без страниц (превью apply, check по всей конфигурации, diff) этим прогоном не нагружались.
// decision: закреплено правилом
// decision-date: 2026-09-22
// decision-source: arch/rules/runtime/invocation-result-size.md
// review: #1119 PR9
pub const MAX_CANONICAL_RESULT_BYTES: usize = 8 * 1024 * 1024;

// resource-limit: result.task-record-envelope-bytes
// class: result
// unit: bytes
// scope: record
// value: 65536
// resource: Запас служебных полей записи задания и ответа сверх канонического результата.
// status: unproven
// origin: коммит 215497a92 (fix(invocation): bound durable result publication)
// problem: неизвестно
// on-reach: result_too_large при превышении суммы 8 МиБ + 64 КиБ
// issue: #1119
// decision: закреплено правилом
// decision-date: 2026-09-22
// decision-source: arch/rules/runtime/daemon-response-size.md
// review: #1119 PR9
pub const MAX_TASK_RECORD_ENVELOPE_BYTES: usize = 64 * 1024;

// resource-limit: result.task-record-bytes
// class: result
// unit: bytes
// scope: record
// value: 8454144
// derived: true
// resource: Размер сохранённой записи задания: канонический результат плюс запас конверта.
// status: unproven
// origin: коммит d3e9ceb45 (fix(invocation): bind durability to process lifetime)
// problem: неизвестно
// on-reach: запись или доставка отвергаются как result_too_large; результат не сохраняется как успех
// issue: #1119
// decision: закреплено правилом INV.APP.DAEMON-TASK-PERSISTENCE
// decision-date: 2026-09-22
// decision-source: arch/rules/runtime/task-record-shape.md
// review: #1119 PR9
pub const MAX_TASK_RECORD_BYTES: usize =
    MAX_CANONICAL_RESULT_BYTES + MAX_TASK_RECORD_ENVELOPE_BYTES;

#[derive(Debug, PartialEq, Eq)]
pub enum CanonicalResultSizeError<E> {
    TooLarge,
    Checkpoint(E),
    Serialization,
}

struct BoundedCountingWriter<'a, E, F>
where
    F: FnMut() -> Result<(), E>,
{
    bytes: usize,
    limit: usize,
    checkpoint: &'a mut F,
    failure: Option<CanonicalResultSizeError<E>>,
}

impl<E, F> Write for BoundedCountingWriter<'_, E, F>
where
    F: FnMut() -> Result<(), E>,
{
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        if let Err(error) = (self.checkpoint)() {
            self.failure = Some(CanonicalResultSizeError::Checkpoint(error));
            return Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "canonical result checkpoint rejected",
            ));
        }
        let Some(next) = self.bytes.checked_add(buffer.len()) else {
            self.failure = Some(CanonicalResultSizeError::TooLarge);
            return Err(io::Error::new(
                io::ErrorKind::FileTooLarge,
                "canonical result exceeds byte limit",
            ));
        };
        if next > self.limit {
            self.failure = Some(CanonicalResultSizeError::TooLarge);
            return Err(io::Error::new(
                io::ErrorKind::FileTooLarge,
                "canonical result exceeds byte limit",
            ));
        }
        self.bytes = next;
        Ok(buffer.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

pub fn canonical_result_size_with_checkpoint<E, F>(
    result: &DomainResult,
    mut checkpoint: F,
) -> Result<usize, CanonicalResultSizeError<E>>
where
    F: FnMut() -> Result<(), E>,
{
    let mut writer = BoundedCountingWriter {
        bytes: 0,
        limit: MAX_CANONICAL_RESULT_BYTES,
        checkpoint: &mut checkpoint,
        failure: None,
    };
    let serialized = serde_json::to_writer(&mut writer, result);
    if let Some(error) = writer.failure.take() {
        return Err(error);
    }
    if serialized.is_err() {
        return Err(CanonicalResultSizeError::Serialization);
    }
    let bytes = writer.bytes;
    drop(writer);
    checkpoint().map_err(CanonicalResultSizeError::Checkpoint)?;
    Ok(bytes)
}

pub fn canonical_result_size(
    result: &DomainResult,
) -> Result<usize, CanonicalResultSizeError<std::convert::Infallible>> {
    canonical_result_size_with_checkpoint(result, || Ok(()))
}
