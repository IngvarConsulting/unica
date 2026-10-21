//! Pure bounded canonical-result accounting; no store or epoch clock.
use crate::invocation::DomainResult;
use std::io::{self, Write};

pub const MAX_CANONICAL_RESULT_BYTES: usize = 8 * 1024 * 1024;

pub const MAX_TASK_RECORD_ENVELOPE_BYTES: usize = 64 * 1024;

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
