#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StreamErrorKind {
    LineTooLong,
    InvalidUtf8,
    EmptyLine,
    InvalidEvent,
    AfterDone,
    DuplicateStart,
    EmptyVersion,
    FileBeforeStart,
    DuplicateFile,
    EmptyFileError,
    ConflictingFileError,
    DoneBeforeStart,
    InvalidElapsed,
    FileTotals,
    DiagnosticTotals,
    FailedFileTotals,
    EmptyCode,
    EmptyMessage,
    UnknownSeverity,
    InvalidRange,
    DuplicateTags,
    EmptyPath,
    PathTraversal,
    UnresolvedPath,
    OutsidePath,
    RootPath,
    ReadFailure,
}

use StreamErrorKind::*;

pub(crate) const MISSING_START_MESSAGE: &str = "stream is missing start event. Check compatibility between Unica and its bundled analyzer; if the problem persists, report this reason.";

const KINDS: &[(StreamErrorKind, &str)] = &[
    (LineTooLong, "line exceeds 8388608 bytes"),
    (InvalidUtf8, "line is not valid UTF-8"),
    (EmptyLine, "line is empty"),
    (InvalidEvent, "invalid JSON or event schema"),
    (AfterDone, "event appeared after terminal done"),
    (DuplicateStart, "duplicate start event"),
    (EmptyVersion, "start.version must be non-empty"),
    (FileBeforeStart, "file event appeared before start"),
    (DuplicateFile, "duplicate normalized file path"),
    (EmptyFileError, "file.error must be non-empty"),
    (
        ConflictingFileError,
        "file.error is mutually exclusive with diagnostics and metrics",
    ),
    (DoneBeforeStart, "done event appeared before start"),
    (
        InvalidElapsed,
        "done.elapsed_secs must be finite and non-negative",
    ),
    (FileTotals, "file totals disagree"),
    (DiagnosticTotals, "diagnostic totals disagree"),
    (FailedFileTotals, "failed file totals disagree"),
    (EmptyCode, "diagnostic.code must be non-empty"),
    (EmptyMessage, "diagnostic.message must be non-empty"),
    (UnknownSeverity, "unknown diagnostic severity"),
    (InvalidRange, "diagnostic range end precedes its start"),
    (DuplicateTags, "diagnostic tags must be unique"),
    (EmptyPath, "file.path must be non-empty"),
    (PathTraversal, "file.path contains invalid traversal"),
    (UnresolvedPath, "file.path could not be resolved safely"),
    (
        OutsidePath,
        "file.path resolves outside diagnostics source root",
    ),
    (
        RootPath,
        "file.path must name a file below the diagnostics source root",
    ),
    (ReadFailure, "failed to read diagnostics stream"),
];

impl StreamErrorKind {
    pub(crate) fn message(self, line: usize) -> String {
        let (_, reason) = KINDS.iter().find(|(kind, _)| *kind == self).unwrap();
        format!("line {line}: {reason}. Check compatibility between Unica and its bundled analyzer; if the problem persists, report this reason and line number.")
    }
}

pub(crate) fn canonical_stream_error(message: &str) -> Option<String> {
    if message == MISSING_START_MESSAGE {
        return Some(MISSING_START_MESSAGE.to_string());
    }
    let (line, _) = message.strip_prefix("line ")?.split_once(": ")?;
    let number = line.parse::<usize>().ok()?;
    if number == 0 || number.to_string() != line {
        return None;
    }
    KINDS.iter().find_map(|(kind, _)| {
        let canonical = kind.message(number);
        (canonical == message).then_some(canonical)
    })
}
