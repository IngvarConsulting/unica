use crate::inspection::git::{read_git_index, GitIndexDigest, GitIndexEntry, IndexReadError};
use crate::inspection::layout::InspectedSourceRoot;
use crate::inspection::{
    SourceRootOwnerIndex, SourceRootOwnerIndexError, SourceRootOwnerLookupError,
};
use crate::{
    ProjectCheckId, ProjectCheckObservation, ProjectCheckOutcome, ProjectHealthFact,
    ProjectHealthFacts, ProjectHealthInspectionError,
};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsString;
use std::fs;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tempfile::TempDir;
#[cfg(test)]
use unica_process::runner::system_process_runner;
use unica_process::runner::{ProcessCommand, ProcessOutput, ProcessRunner};
use unica_process::StreamControl;
use unica_source_contract::SourceFormat;
use unica_work_control::cancellation::CancellationToken;
use unica_work_control::provider_deadline::ProviderDeadline;
use unica_workspace::paths::host_path_text;
use unica_workspace::paths::normalize_path_identity;
use unica_workspace::read::ReadFile as File;
use unica_workspace::read::{
    open_absolute_read_directory_nofollow, open_any_read_child_nofollow,
    open_read_child_for_secure_tree_use, OpenedChildKind,
};

pub(crate) const LFS_SINGLE_FILE_THRESHOLD_BYTES: u64 = 10 * 1024 * 1024;
pub(crate) const LFS_AGGREGATE_THRESHOLD_BYTES: u64 = 100 * 1024 * 1024;
const MAX_STAGED_POLICY_FILES: usize = 1024;
const MAX_STAGED_POLICY_TOTAL_BYTES: usize = 8 * 1024 * 1024;
const MAX_INDEX_EOL_FILE_BYTES: usize = 32 * 1024 * 1024;
const MAX_INDEX_EOL_BATCH_FILES: usize = 64;
const MAX_INDEX_EOL_BATCH_BLOB_BYTES: usize = 32 * 1024 * 1024;
/// Resources inspected together. A step of the inspection, not a bound of the
/// corpus: every page is inspected before a result is published.
const RESOURCE_PAGE_PATHS: usize = 4096;
const MAX_RESOURCE_OWNERSHIP_REASON_BYTES: usize = 512;
const MAX_CONTINUATION_WORKSPACES: usize = 4;
const CONTINUATION_TTL: Duration = Duration::from_secs(15 * 60);
const CONTINUATION_MAGIC: &[u8] = b"unica-resource-eol-v1\0";

#[cfg(any(test, feature = "test-support"))]
type WorkingEolBlockHook = Box<dyn FnOnce() -> Result<(), WorkingEolInspectionError>>;

#[cfg(any(test, feature = "test-support"))]
thread_local! {
    static BEFORE_WORKING_REVALIDATION: std::cell::RefCell<Option<Box<dyn FnOnce()>>> =
        std::cell::RefCell::new(None);
    static STAGED_EOL_STOP_AFTER: std::cell::Cell<Option<usize>> =
        const { std::cell::Cell::new(None) };
    static AFTER_WORKING_EOL_BLOCK: std::cell::RefCell<Option<WorkingEolBlockHook>> =
        std::cell::RefCell::new(None);
}

#[cfg(any(test, feature = "test-support"))]
pub fn stop_working_eol_after_block_for_test() {
    AFTER_WORKING_EOL_BLOCK.with(|slot| {
        *slot.borrow_mut() = Some(Box::new(|| Err(WorkingEolInspectionError::TimedOut)));
    });
}

#[cfg(any(test, feature = "test-support"))]
fn run_working_revalidation_test_hook() {
    BEFORE_WORKING_REVALIDATION.with(|slot| {
        if let Some(hook) = slot.borrow_mut().take() {
            hook();
        }
    });
}

#[cfg(not(any(test, feature = "test-support")))]
fn run_working_revalidation_test_hook() {}

#[cfg(any(test, feature = "test-support"))]
pub fn stop_staged_eol_after_for_test(completed: usize) {
    STAGED_EOL_STOP_AFTER.with(|slot| slot.set(Some(completed)));
}

#[cfg(any(test, feature = "test-support"))]
fn staged_eol_test_timeout() -> bool {
    STAGED_EOL_STOP_AFTER.with(|slot| match slot.get() {
        Some(0) => {
            slot.set(None);
            true
        }
        Some(remaining) => {
            slot.set(Some(remaining - 1));
            false
        }
        None => false,
    })
}

#[cfg(not(any(test, feature = "test-support")))]
fn staged_eol_test_timeout() -> bool {
    false
}

/// Continuations of root checks, one file per workspace in a directory
/// outside every workspace. The default store keeps none: each check then
/// inspects from the start.
#[derive(Default)]
pub struct RootCheckContinuationStore {
    directory: Option<PathBuf>,
    entries: Mutex<BTreeMap<PathBuf, ContinuationEntry>>,
}

struct ContinuationEntry {
    state: Arc<Mutex<ResourceContinuation>>,
    last_used: Instant,
}

impl RootCheckContinuationStore {
    /// Keep continuations in `directory`. Files not rewritten for longer
    /// than `retention` belong to abandoned workspaces and are removed now;
    /// a file that cannot be removed only stays behind.
    pub fn in_directory(directory: PathBuf, retention: Duration) -> Self {
        remove_stale_continuations(&directory, retention);
        Self {
            directory: Some(directory),
            entries: Mutex::default(),
        }
    }

    /// The continuation of the workspace whose canonical root is `root`.
    pub fn for_workspace(&self, root: &Path) -> Option<Arc<Mutex<ResourceContinuation>>> {
        let directory = self.directory.as_ref()?;
        let mut entries = self.entries.lock().ok()?;
        let now = Instant::now();
        entries.retain(|_, entry| {
            now.duration_since(entry.last_used) < CONTINUATION_TTL
                || Arc::strong_count(&entry.state) > 1
        });
        if let Some(entry) = entries.get_mut(root) {
            entry.last_used = now;
            return Some(Arc::clone(&entry.state));
        }
        if entries.len() >= MAX_CONTINUATION_WORKSPACES {
            let oldest_idle = entries
                .iter()
                .filter(|(_, entry)| Arc::strong_count(&entry.state) == 1)
                .min_by_key(|(_, entry)| entry.last_used)
                .map(|(root, _)| root.clone())?;
            entries.remove(&oldest_idle);
        }
        let state = Arc::new(Mutex::new(ResourceContinuation::new(
            &directory.join(continuation_file_name(root)),
        )));
        entries.insert(
            root.to_path_buf(),
            ContinuationEntry {
                state: Arc::clone(&state),
                last_used: now,
            },
        );
        Some(state)
    }
}

/// 32 lowercase hex digits of SHA-256 over the canonical root: a short name
/// that every host filesystem accepts.
fn continuation_file_name(root: &Path) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"unica-root-check-continuation-v1\0");
    hasher.update(root.as_os_str().as_encoded_bytes());
    let digest: [u8; 32] = hasher.finalize().into();
    let mut name = digest[..16]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    name.push_str(".bin");
    name
}

fn remove_stale_continuations(directory: &Path, retention: Duration) {
    let Ok(children) = fs::read_dir(directory) else {
        return;
    };
    let now = std::time::SystemTime::now();
    for child in children.flatten() {
        let stale = child.metadata().is_ok_and(|metadata| {
            metadata.is_file()
                && metadata
                    .modified()
                    .ok()
                    .and_then(|modified| now.duration_since(modified).ok())
                    .is_some_and(|age| age > retention)
        });
        if stale {
            let _ = fs::remove_file(child.path());
        }
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct ResourceInspectionProgress {
    pub text_resources: usize,
    pub staged_eol_retained: usize,
    pub working_eol_retained: usize,
}

/// Progress of the resource EOL inspection that a later call continues. The
/// evidence itself is a file in the cache root, read and rewritten in path
/// order one page at a time; only the adaptive batch width and the progress of
/// the current call stay in memory.
pub struct ResourceContinuation {
    file: PathBuf,
    index_eol_batch_files: Option<usize>,
    pub(crate) progress: ResourceInspectionProgress,
}

type EvidenceKey = [u8; 32];
/// SHA-256 of a working file's identity, size and times.
type FileFingerprint = [u8; 32];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum IndexEol {
    Lf,
    CrLf,
    Mixed,
    None,
    Binary,
}

impl IndexEol {
    fn from_git(value: &str) -> Option<Self> {
        match value {
            "lf" => Some(Self::Lf),
            "crlf" => Some(Self::CrLf),
            "mixed" => Some(Self::Mixed),
            "none" => Some(Self::None),
            "-text" => Some(Self::Binary),
            _ => None,
        }
    }

    fn as_git(self) -> &'static str {
        match self {
            Self::Lf => "lf",
            Self::CrLf => "crlf",
            Self::Mixed => "mixed",
            Self::None => "none",
            Self::Binary => "-text",
        }
    }

    fn from_byte(value: u8) -> Option<Self> {
        [Self::Lf, Self::CrLf, Self::Mixed, Self::None, Self::Binary]
            .into_iter()
            .find(|eol| *eol as u8 == value)
    }
}

/// What a working file was observed to hold. Partial bytes are not evidence of
/// readiness: their fingerprint must still match before the next read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WorkingEvidence {
    Absent,
    Partial {
        fingerprint: FileFingerprint,
        scanner: WorkingEolScanner,
    },
    Complete {
        fingerprint: FileFingerprint,
        eol: WorkingEol,
    },
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct WorkingEolScanner {
    offset: u64,
    previous_cr: bool,
    lf: bool,
    crlf: bool,
    bare_cr: bool,
}

/// Evidence kept for one text resource path.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct EolRecord {
    index: Option<(EvidenceKey, IndexEol)>,
    working: Option<WorkingEvidence>,
}

enum RecordReadError {
    /// A damaged or foreign record: the rest of the generation is unusable.
    Corrupt,
    Io(std::io::Error),
}

impl From<std::io::Error> for RecordReadError {
    fn from(error: std::io::Error) -> Self {
        if error.kind() == std::io::ErrorKind::UnexpectedEof {
            Self::Corrupt
        } else {
            Self::Io(error)
        }
    }
}

impl EolRecord {
    fn is_empty(&self) -> bool {
        self.index.is_none() && self.working.is_none()
    }

    fn write(&self, path: &str, out: &mut impl Write) -> std::io::Result<()> {
        out.write_all(&(path.len() as u64).to_be_bytes())?;
        out.write_all(path.as_bytes())?;
        match self.index {
            None => out.write_all(&[0])?,
            Some((key, eol)) => {
                out.write_all(&[1])?;
                out.write_all(&key)?;
                out.write_all(&[eol as u8])?;
            }
        }
        match self.working {
            None => out.write_all(&[0]),
            Some(WorkingEvidence::Absent) => out.write_all(&[1]),
            Some(WorkingEvidence::Partial {
                fingerprint,
                scanner,
            }) => {
                out.write_all(&[2])?;
                out.write_all(&fingerprint)?;
                out.write_all(&scanner.offset.to_be_bytes())?;
                out.write_all(&[u8::from(scanner.previous_cr)
                    | u8::from(scanner.lf) << 1
                    | u8::from(scanner.crlf) << 2
                    | u8::from(scanner.bare_cr) << 3])
            }
            Some(WorkingEvidence::Complete { fingerprint, eol }) => {
                out.write_all(&[3])?;
                out.write_all(&fingerprint)?;
                out.write_all(&[eol as u8])
            }
        }
    }

    /// The next record, or `None` at the clean end of the stream.
    fn read(input: &mut impl Read) -> Result<Option<(String, Self)>, RecordReadError> {
        let mut length = [0_u8; 8];
        let first = loop {
            match input.read(&mut length[..1]) {
                Ok(count) => break count,
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
                Err(error) => return Err(error.into()),
            }
        };
        if first == 0 {
            return Ok(None);
        }
        input.read_exact(&mut length[1..])?;
        let length = u64::from_be_bytes(length);
        let mut path = Vec::new();
        input.by_ref().take(length).read_to_end(&mut path)?;
        if path.len() as u64 != length {
            return Err(RecordReadError::Corrupt);
        }
        let path = String::from_utf8(path).map_err(|_| RecordReadError::Corrupt)?;
        let index = match read_byte(input)? {
            0 => None,
            1 => {
                let key = read_array(input)?;
                let eol = IndexEol::from_byte(read_byte(input)?).ok_or(RecordReadError::Corrupt)?;
                Some((key, eol))
            }
            _ => return Err(RecordReadError::Corrupt),
        };
        let working = match read_byte(input)? {
            0 => None,
            1 => Some(WorkingEvidence::Absent),
            2 => {
                let fingerprint = read_array(input)?;
                let offset = u64::from_be_bytes(read_array(input)?);
                let flags = read_byte(input)?;
                if flags > 0b1111 {
                    return Err(RecordReadError::Corrupt);
                }
                Some(WorkingEvidence::Partial {
                    fingerprint,
                    scanner: WorkingEolScanner {
                        offset,
                        previous_cr: flags & 1 != 0,
                        lf: flags & 2 != 0,
                        crlf: flags & 4 != 0,
                        bare_cr: flags & 8 != 0,
                    },
                })
            }
            3 => {
                let fingerprint = read_array(input)?;
                let eol =
                    WorkingEol::from_byte(read_byte(input)?).ok_or(RecordReadError::Corrupt)?;
                Some(WorkingEvidence::Complete { fingerprint, eol })
            }
            _ => return Err(RecordReadError::Corrupt),
        };
        Ok(Some((path, Self { index, working })))
    }
}

fn read_byte(input: &mut impl Read) -> Result<u8, RecordReadError> {
    Ok(read_array::<1>(input)?[0])
}

fn read_array<const N: usize>(input: &mut impl Read) -> Result<[u8; N], RecordReadError> {
    let mut bytes = [0_u8; N];
    input.read_exact(&mut bytes)?;
    Ok(bytes)
}

/// Sequential reader of one continuation generation. Records follow strictly
/// increasing paths; a damaged, foreign or out-of-order record ends the
/// generation as if the rest were absent.
struct GenerationReader {
    input: std::io::BufReader<fs::File>,
    last: Option<String>,
}

impl GenerationReader {
    /// `None` when there is no usable generation at `path`.
    fn open(path: &Path) -> std::io::Result<Option<Self>> {
        let file = match fs::File::open(path) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error),
        };
        let mut input = std::io::BufReader::new(file);
        let mut magic = vec![0; CONTINUATION_MAGIC.len()];
        match input.read_exact(&mut magic) {
            Ok(()) if magic == CONTINUATION_MAGIC => Ok(Some(Self { input, last: None })),
            Ok(()) => Ok(None),
            Err(error) if error.kind() == std::io::ErrorKind::UnexpectedEof => Ok(None),
            Err(error) => Err(error),
        }
    }

    fn next(&mut self) -> std::io::Result<Option<(String, EolRecord)>> {
        match EolRecord::read(&mut self.input) {
            Ok(Some((path, record))) if self.last.as_ref().is_none_or(|last| *last < path) => {
                self.last = Some(path.clone());
                Ok(Some((path, record)))
            }
            Ok(_) | Err(RecordReadError::Corrupt) => Ok(None),
            Err(RecordReadError::Io(error)) => Err(error),
        }
    }
}

/// The previous continuation generation, read by merge-join in path order,
/// and the next one, written beside it and published by an atomic rename.
struct EolGeneration {
    file: PathBuf,
    previous: Option<GenerationReader>,
    ahead: Option<(String, EolRecord)>,
    next: Option<std::io::BufWriter<tempfile::NamedTempFile>>,
    last_written: Option<String>,
}

impl EolGeneration {
    fn open(file: &Path) -> std::io::Result<Self> {
        Ok(Self {
            file: file.to_path_buf(),
            previous: GenerationReader::open(file)?,
            ahead: None,
            next: None,
            last_written: None,
        })
    }

    fn advance(&mut self) -> std::io::Result<()> {
        if self.ahead.is_none() {
            if let Some(previous) = self.previous.as_mut() {
                self.ahead = previous.next()?;
                if self.ahead.is_none() {
                    self.previous = None;
                }
            }
        }
        Ok(())
    }

    /// Previous records of `paths`, given in increasing order. Older paths
    /// that are no longer current are dropped on the way.
    fn take(&mut self, paths: &[&str]) -> std::io::Result<BTreeMap<String, EolRecord>> {
        let mut records = BTreeMap::new();
        let Some(last) = paths.last() else {
            return Ok(records);
        };
        loop {
            self.advance()?;
            match self.ahead.take() {
                Some((path, record)) if path.as_str() <= *last => {
                    if paths.binary_search(&path.as_str()).is_ok() {
                        records.insert(path, record);
                    }
                }
                ahead => {
                    self.ahead = ahead;
                    return Ok(records);
                }
            }
        }
    }

    fn write(&mut self, records: &BTreeMap<String, EolRecord>) -> std::io::Result<()> {
        for (path, record) in records {
            if !record.is_empty() {
                self.write_record(path, record)?;
            }
        }
        Ok(())
    }

    fn write_record(&mut self, path: &str, record: &EolRecord) -> std::io::Result<()> {
        if self
            .last_written
            .as_deref()
            .is_some_and(|last| last >= path)
        {
            return Err(std::io::Error::other(
                "resource EOL continuation records are not in path order",
            ));
        }
        if self.next.is_none() {
            let directory = self
                .file
                .parent()
                .ok_or_else(|| std::io::Error::other("continuation file has no directory"))?;
            fs::create_dir_all(directory)?;
            let mut next = std::io::BufWriter::new(tempfile::NamedTempFile::new_in(directory)?);
            next.write_all(CONTINUATION_MAGIC)?;
            self.next = Some(next);
        }
        let next = self.next.as_mut().expect("the next generation was created");
        record.write(path, next)?;
        self.last_written = Some(path.to_owned());
        Ok(())
    }

    /// A reader over the records written so far.
    fn written(&mut self) -> std::io::Result<Option<GenerationReader>> {
        let Some(next) = self.next.as_mut() else {
            return Ok(None);
        };
        next.flush()?;
        GenerationReader::open(next.get_ref().path())
    }

    /// Replace the previous generation by the next one. `keep_rest` carries
    /// over the previous records past the last page that was read.
    fn publish(mut self, keep_rest: bool) -> std::io::Result<()> {
        if self.next.is_none() {
            return Ok(());
        }
        if keep_rest {
            loop {
                self.advance()?;
                let Some((path, record)) = self.ahead.take() else {
                    break;
                };
                self.write_record(&path, &record)?;
            }
        }
        self.previous = None;
        let next = self
            .next
            .take()
            .expect("the next generation was created")
            .into_inner()
            .map_err(std::io::IntoInnerError::into_error)?;
        next.as_file().sync_all()?;
        next.persist(&self.file).map_err(|error| error.error)?;
        Ok(())
    }
}

impl ResourceContinuation {
    /// A continuation whose evidence lives in `file`.
    pub fn new(file: &Path) -> Self {
        Self {
            file: file.to_path_buf(),
            index_eol_batch_files: None,
            progress: ResourceInspectionProgress::default(),
        }
    }

    pub fn progress(&self) -> ResourceInspectionProgress {
        self.progress
    }

    pub fn reset_progress(&mut self) {
        self.progress = ResourceInspectionProgress::default();
    }

    #[cfg(any(test, feature = "test-support"))]
    fn records_for_test(&self) -> Vec<(String, EolRecord)> {
        let mut records = Vec::new();
        if let Some(mut reader) = GenerationReader::open(&self.file).unwrap() {
            while let Some(record) = reader.next().unwrap() {
                records.push(record);
            }
        }
        records
    }

    #[cfg(test)]
    fn index_eol_keys_for_test(&self) -> BTreeSet<EvidenceKey> {
        self.records_for_test()
            .into_iter()
            .filter_map(|(_, record)| record.index.map(|(key, _)| key))
            .collect()
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn working_eol_offset_for_test(&self, path: &str) -> Option<u64> {
        self.records_for_test()
            .into_iter()
            .find(|(candidate, _)| candidate == path)
            .and_then(|(_, record)| match record.working {
                Some(WorkingEvidence::Partial { scanner, .. }) => Some(scanner.offset),
                _ => None,
            })
    }

    fn index_eol_batch_files(&self) -> usize {
        self.index_eol_batch_files
            .unwrap_or(MAX_INDEX_EOL_BATCH_FILES)
    }

    fn reduce_index_eol_batch(&mut self, failed_files: usize) {
        self.index_eol_batch_files = Some((failed_files / 2).max(1));
    }
}

fn hash_evidence_field(hasher: &mut Sha256, field: &str) {
    hasher.update((field.len() as u64).to_be_bytes());
    hasher.update(field.as_bytes());
}

fn index_evidence_key(
    path: &str,
    oid: &str,
    staged: &AttributeValues,
    effective: &AttributeValues,
) -> EvidenceKey {
    let mut hasher = Sha256::new();
    hasher.update(b"unica-index-eol-v1\0");
    for field in [
        path,
        oid,
        &staged.text,
        &staged.eol,
        &staged.filter,
        &effective.text,
        &effective.eol,
        &effective.filter,
    ] {
        hash_evidence_field(&mut hasher, field);
    }
    hasher.finalize().into()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RepositoryResourceKind {
    Text,
    Binary,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RepositoryResource {
    pub(crate) source_set: String,
    pub(crate) repo_path: String,
    pub(crate) kind: RepositoryResourceKind,
    pub(crate) blob_oid: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ResourceOwnershipError {
    pub(crate) repo_path: String,
    pub(crate) source_sets: Vec<String>,
}

enum ResourceClassificationError {
    Cancelled,
    TimedOut,
    Ownership(ResourceOwnershipError),
}

enum ClassifiedEntry {
    Resource(RepositoryResource),
    Ownership(ResourceOwnershipError),
}

/// Assigns staged entries to the Platform XML resource roles of their deepest
/// source root, one entry at a time.
struct ResourceClassifier<'r> {
    owners: SourceRootOwnerIndex<'r>,
    excluded_runtime_sidecars: &'r BTreeSet<String>,
}

impl<'r> ResourceClassifier<'r> {
    fn new(
        repository_root: &Path,
        roots: &'r [InspectedSourceRoot],
        excluded_runtime_sidecars: &'r BTreeSet<String>,
        checkpoint: &mut dyn FnMut() -> Result<(), ResourceClassificationError>,
    ) -> Result<Self, ResourceClassificationError> {
        let repository_root = normalize_path_identity(repository_root).map_err(|_| {
            ResourceClassificationError::Ownership(ResourceOwnershipError {
                repo_path: repository_root.display().to_string(),
                source_sets: Vec::new(),
            })
        })?;
        let owners =
            SourceRootOwnerIndex::new_with_checkpoint(&repository_root, roots.iter(), checkpoint)
                .map_err(|error| match error {
                SourceRootOwnerIndexError::Checkpoint(error) => error,
                SourceRootOwnerIndexError::Path(reason) => {
                    ResourceClassificationError::Ownership(ResourceOwnershipError {
                        repo_path: reason,
                        source_sets: Vec::new(),
                    })
                }
            })?;
        Ok(Self {
            owners,
            excluded_runtime_sidecars,
        })
    }

    fn classify(
        &self,
        entry: &GitIndexEntry,
        checkpoint: &mut dyn FnMut() -> Result<(), ResourceClassificationError>,
    ) -> Result<Option<ClassifiedEntry>, ResourceClassificationError> {
        if self.excluded_runtime_sidecars.contains(&entry.repo_path) {
            return Ok(None);
        }
        let Some((owners, prefix_depth)) = self
            .owners
            .deepest_owners_with_checkpoint(&entry.repo_path, checkpoint)
            .map_err(|error| match error {
                SourceRootOwnerLookupError::Checkpoint(error) => error,
                SourceRootOwnerLookupError::Path(reason) => {
                    ResourceClassificationError::Ownership(ResourceOwnershipError {
                        repo_path: reason,
                        source_sets: Vec::new(),
                    })
                }
            })?
        else {
            return Ok(None);
        };
        if owners.len() > 1 {
            let mut source_sets = owners
                .iter()
                .map(|root| root.source_set.name.clone())
                .collect::<Vec<_>>();
            source_sets.sort();
            source_sets.dedup();
            return Ok(Some(ClassifiedEntry::Ownership(ResourceOwnershipError {
                repo_path: entry.repo_path.clone(),
                source_sets,
            })));
        }
        let root = owners[0];
        if root.source_set.source_format != SourceFormat::PlatformXml {
            return Ok(None);
        }
        let mut relative = String::new();
        for (component_index, component) in Path::new(&entry.repo_path)
            .components()
            .skip(prefix_depth)
            .enumerate()
        {
            if component_index % 256 == 0 {
                checkpoint()?;
            }
            if !relative.is_empty() {
                relative.push('/');
            }
            relative.push_str(&component.as_os_str().to_string_lossy());
        }
        let Some(kind) = classify_platform_xml_relative_path(&relative) else {
            return Ok(None);
        };
        let blob_oid = entry.blob_oid.clone().filter(|_| {
            entry
                .mode
                .as_deref()
                .is_some_and(|mode| matches!(mode, "100644" | "100755"))
        });
        Ok(Some(match blob_oid {
            Some(blob_oid) => ClassifiedEntry::Resource(RepositoryResource {
                source_set: root.source_set.name.clone(),
                repo_path: entry.repo_path.clone(),
                kind,
                blob_oid,
            }),
            None => ClassifiedEntry::Ownership(ResourceOwnershipError {
                repo_path: entry.repo_path.clone(),
                source_sets: vec![root.source_set.name.clone()],
            }),
        }))
    }
}

fn bounded_resource_ownership_reason(error: &ResourceOwnershipError) -> String {
    const PREFIX: &str =
        "resource does not have one regular stage-0 blob or has ambiguous source-set owners";
    let detailed_bytes = PREFIX
        .len()
        .saturating_add(2)
        .saturating_add(error.repo_path.len())
        .saturating_add(2)
        .saturating_add(error.source_sets.iter().fold(0_usize, |total, source_set| {
            total.saturating_add(source_set.len()).saturating_add(2)
        }));
    if detailed_bytes <= MAX_RESOURCE_OWNERSHIP_REASON_BYTES {
        return format!(
            "{PREFIX}: {}; {}",
            error.repo_path,
            error.source_sets.join(", ")
        );
    }
    format!(
        "resource ownership is ambiguous or non-regular; path bytes={}, owners={} (details omitted by inspection budget)",
        error.repo_path.len(),
        error.source_sets.len()
    )
}

pub(crate) struct RepositoryPolicyInspection {
    pub(crate) observations: Vec<ProjectCheckObservation>,
    pub(crate) facts: Vec<ProjectHealthFact>,
}

pub(crate) struct SourceResourcePolicyInspector<'a> {
    runner: &'a dyn ProcessRunner,
    response_handoff_window: Duration,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct AttributeValues {
    text: String,
    eol: String,
    filter: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct EolValues {
    index: String,
    worktree: String,
}

struct AttributePolicyEvaluation {
    facts: Vec<ProjectHealthFact>,
    text_marked_binary: BTreeSet<String>,
}

#[derive(Debug)]
struct IsolatedGitIndex {
    _root: TempDir,
    git_dir: PathBuf,
    worktree: PathBuf,
    index: PathBuf,
    object_directory: PathBuf,
}

#[cfg(test)]
impl SourceResourcePolicyInspector<'static> {
    pub(crate) fn new() -> Self {
        Self {
            runner: system_process_runner(),
            response_handoff_window: super::TEST_RESPONSE_HANDOFF_WINDOW,
        }
    }
}

/// Why the resource inspection stopped before its last page.
enum Stop {
    Cancelled,
    /// The inspection ends with this answer; later pages are not inspected.
    Answer(RepositoryPolicyInspection),
    Fatal(String),
}

/// The index EOL check stopped on a page; later pages inspect attributes only.
struct IndexEolStop {
    reason: String,
    fact: ProjectHealthFact,
    /// The working EOL and LFS checks report the same reason.
    dependents: bool,
}

/// Binary resources of one source set, summarized for the LFS advice.
#[derive(Default)]
struct LfsAggregate {
    count: usize,
    total_bytes: u64,
    largest: Option<(String, u64)>,
    paths: BTreeSet<String>,
}

impl LfsAggregate {
    fn add(&mut self, path: &str, size: u64) -> Result<(), String> {
        self.count = self
            .count
            .checked_add(1)
            .ok_or("LFS resource count overflow")?;
        self.total_bytes = self
            .total_bytes
            .checked_add(size)
            .ok_or("LFS resource size overflow")?;
        if self
            .largest
            .as_ref()
            .is_none_or(|(_, largest)| size > *largest)
        {
            self.largest = Some((path.to_owned(), size));
        }
        self.paths.insert(path.to_owned());
        if self.paths.len() > crate::MAX_PROJECT_DIAGNOSTIC_PATHS {
            self.paths.pop_last();
        }
        Ok(())
    }
}

/// One resource inspection, page by page in path order.
struct ResourcePass<'p, 'a> {
    inspector: &'p SourceResourcePolicyInspector<'a>,
    repository_root: &'p Path,
    entries: &'p [GitIndexEntry],
    cancellation: &'p CancellationToken,
    deadline: ProviderDeadline,
    continuation: Option<&'p mut ResourceContinuation>,
    generation: Option<EolGeneration>,
    /// Evidence of the current page, written when the page ends.
    records: BTreeMap<String, EolRecord>,
    observations: Vec<ProjectCheckObservation>,
    ownership: BTreeMap<String, (usize, String)>,
    staged_index: Option<IsolatedGitIndex>,
    effective: BTreeMap<String, Sha256>,
    attribute_facts: ProjectHealthFacts,
    index_eol_facts: ProjectHealthFacts,
    working_facts: ProjectHealthFacts,
    facts: Vec<ProjectHealthFact>,
    index_stop: Option<IndexEolStop>,
    working_timed_out: bool,
    lfs_timed_out: bool,
    working_incomplete: BTreeSet<String>,
    lfs_incomplete: BTreeSet<String>,
    lfs: BTreeMap<String, LfsAggregate>,
}

impl<'a> SourceResourcePolicyInspector<'a> {
    pub(crate) fn with_process_runner(
        runner: &'a dyn ProcessRunner,
        response_handoff_window: Duration,
    ) -> Self {
        Self {
            runner,
            response_handoff_window,
        }
    }

    #[cfg(test)]
    pub(crate) fn classify(
        repository_root: &Path,
        roots: &[InspectedSourceRoot],
        entries: &[GitIndexEntry],
        excluded_runtime_sidecars: &BTreeSet<String>,
    ) -> Result<Vec<RepositoryResource>, ResourceOwnershipError> {
        let uncontrolled = |error| match error {
            ResourceClassificationError::Cancelled | ResourceClassificationError::TimedOut => {
                unreachable!("uncontrolled resource classification cannot stop")
            }
            ResourceClassificationError::Ownership(error) => error,
        };
        let classifier = ResourceClassifier::new(
            repository_root,
            roots,
            excluded_runtime_sidecars,
            &mut || Ok(()),
        )
        .map_err(uncontrolled)?;
        let mut resources = Vec::new();
        for entry in entries {
            match classifier
                .classify(entry, &mut || Ok(()))
                .map_err(uncontrolled)?
            {
                Some(ClassifiedEntry::Resource(resource)) => resources.push(resource),
                Some(ClassifiedEntry::Ownership(error)) => return Err(error),
                None => {}
            }
        }
        Ok(resources)
    }

    #[cfg(test)]
    pub(crate) fn inspect(
        &self,
        repository_root: &Path,
        roots: &[InspectedSourceRoot],
        entries: &[GitIndexEntry],
        cancellation: &CancellationToken,
        deadline: ProviderDeadline,
    ) -> Result<RepositoryPolicyInspection, ProjectHealthInspectionError> {
        self.inspect_excluding(
            repository_root,
            roots,
            entries,
            &BTreeSet::new(),
            cancellation,
            deadline,
        )
    }

    #[cfg(test)]
    pub(crate) fn inspect_excluding(
        &self,
        repository_root: &Path,
        roots: &[InspectedSourceRoot],
        entries: &[GitIndexEntry],
        excluded_runtime_sidecars: &BTreeSet<String>,
        cancellation: &CancellationToken,
        deadline: ProviderDeadline,
    ) -> Result<RepositoryPolicyInspection, ProjectHealthInspectionError> {
        self.inspect_excluding_continued(
            repository_root,
            roots,
            entries,
            excluded_runtime_sidecars,
            cancellation,
            deadline,
            None,
        )
    }

    /// Inspect every staged Platform XML resource page by page. With a
    /// continuation, its evidence on disk lets this call reuse what an earlier
    /// call proved, and what this call proves before stopping is kept for the
    /// next one. A disk error is an inspection error.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn inspect_excluding_continued(
        &self,
        repository_root: &Path,
        roots: &[InspectedSourceRoot],
        entries: &[GitIndexEntry],
        excluded_runtime_sidecars: &BTreeSet<String>,
        cancellation: &CancellationToken,
        deadline: ProviderDeadline,
        continuation: Option<&mut ResourceContinuation>,
    ) -> Result<RepositoryPolicyInspection, ProjectHealthInspectionError> {
        if cancellation.is_cancelled() {
            return Err(ProjectHealthInspectionError::Cancelled);
        }
        let generation = match continuation.as_deref() {
            Some(state) => Some(EolGeneration::open(&state.file).map_err(continuation_error)?),
            None => None,
        };
        let mut pass = ResourcePass {
            inspector: self,
            repository_root,
            entries,
            cancellation,
            deadline,
            continuation,
            generation,
            records: BTreeMap::new(),
            observations: resource_observations(
                roots.iter().map(|root| &root.source_set),
                "repository resource check was not reached",
                true,
            ),
            ownership: BTreeMap::new(),
            staged_index: None,
            effective: BTreeMap::new(),
            attribute_facts: ProjectHealthFacts::default(),
            index_eol_facts: ProjectHealthFacts::default(),
            working_facts: ProjectHealthFacts::default(),
            facts: Vec::new(),
            index_stop: None,
            working_timed_out: false,
            lfs_timed_out: false,
            working_incomplete: BTreeSet::new(),
            lfs_incomplete: BTreeSet::new(),
            lfs: BTreeMap::new(),
        };
        let result = match ResourceClassifier::new(
            repository_root,
            roots,
            excluded_runtime_sidecars,
            &mut || resource_classification_checkpoint(cancellation, deadline),
        ) {
            Err(error) => Err(pass.classification_stop(error)),
            Ok(classifier) => pass
                .pages(&classifier)
                .and_then(|()| pass.complete(&classifier)),
        };
        let keep_rest = result.is_err() || pass.index_stop.is_some();
        let published = match pass.generation.take() {
            Some(generation) => generation.publish(keep_rest),
            None => Ok(()),
        };
        match result {
            Err(Stop::Cancelled) => Err(ProjectHealthInspectionError::Cancelled),
            Err(Stop::Fatal(reason)) => Err(ProjectHealthInspectionError::Fatal(reason)),
            Ok(policy) | Err(Stop::Answer(policy)) => {
                published.map_err(continuation_error)?;
                Ok(policy)
            }
        }
    }

    fn run<const N: usize>(
        &self,
        cwd: &Path,
        args: [&str; N],
        cancellation: &CancellationToken,
        deadline: ProviderDeadline,
    ) -> Result<ProcessOutput, String> {
        let result = self.runner.run(&process_command(
            cwd,
            args.into_iter().map(str::to_owned).collect(),
            cancellation,
            deadline,
        ));
        sticky_process_result(result, cancellation)
    }

    /// Run one Git command and hand each `delimiter`-terminated record of its
    /// output to `on_record`, without capturing the output.
    fn stream(
        &self,
        command: &ProcessCommand,
        input: &[u8],
        delimiter: u8,
        cancellation: &CancellationToken,
        deadline: ProviderDeadline,
        on_record: &mut dyn FnMut(&[u8]) -> Result<(), ResourceProtocolParseError>,
    ) -> Result<(), ResourceProtocolParseError> {
        let mut failed = None;
        let output =
            self.runner
                .run_streaming_records(command, input, delimiter, &mut |record| match on_record(
                    record,
                ) {
                    Ok(()) => StreamControl::Continue,
                    Err(error) => {
                        failed = Some(error);
                        StreamControl::Stop
                    }
                });
        if cancellation.is_cancelled() {
            return Err(ResourceProtocolParseError::Cancelled);
        }
        let output =
            output.map_err(|reason| resource_process_error(reason, cancellation, deadline))?;
        if let Some(error) = failed {
            return Err(error);
        }
        if output.cancelled {
            Err(ResourceProtocolParseError::Cancelled)
        } else if output.timed_out {
            Err(ResourceProtocolParseError::TimedOut)
        } else if let Some((_, error)) = output.line_error {
            Err(ResourceProtocolParseError::Malformed(format!(
                "Git resource inspection output is incomplete: {error}"
            )))
        } else if !output.status_success {
            Err(ResourceProtocolParseError::Malformed(
                if output.stderr.trim().is_empty() {
                    format!("Git resource inspection exited with {}", output.status)
                } else {
                    format!(
                        "Git resource inspection exited with {}: {}",
                        output.status,
                        output.stderr.trim()
                    )
                },
            ))
        } else {
            Ok(())
        }
    }

    /// `git check-attr` of `paths` with `command`, one triple per attribute.
    fn attributes(
        &self,
        command: &ProcessCommand,
        paths: &[String],
        cancellation: &CancellationToken,
        deadline: ProviderDeadline,
    ) -> Result<BTreeMap<String, AttributeValues>, ResourceProtocolParseError> {
        let checkpoint = &mut || resource_protocol_checkpoint(cancellation, deadline);
        let mut records = AttributeRecords::new(paths, checkpoint)?;
        self.stream(
            command,
            &nul_input(paths),
            0,
            cancellation,
            deadline,
            &mut |field| {
                records.push(field, &mut || {
                    resource_protocol_checkpoint(cancellation, deadline)
                })
            },
        )?;
        records.finish(checkpoint)
    }

    #[allow(clippy::too_many_arguments)]
    fn incremental_index_eol(
        &self,
        repository_root: &Path,
        resources: &[&RepositoryResource],
        blob_sizes: &BTreeMap<String, usize>,
        staged: &BTreeMap<String, AttributeValues>,
        effective: &BTreeMap<String, AttributeValues>,
        records: &mut BTreeMap<String, EolRecord>,
        continuation: &mut ResourceContinuation,
        cancellation: &CancellationToken,
        deadline: ProviderDeadline,
    ) -> Result<BTreeMap<String, EolValues>, ResourceProtocolParseError> {
        let mut result = BTreeMap::new();
        let mut pending = Vec::<(&RepositoryResource, EvidenceKey)>::new();
        let mut pending_bytes = 0_usize;
        let mut completed_batches = 0_usize;
        for resource in resources {
            resource_protocol_checkpoint(cancellation, deadline)?;
            let path = &resource.repo_path;
            let staged_attributes = staged.get(path).ok_or_else(|| {
                ResourceProtocolParseError::Malformed(format!(
                    "staged Git attribute policy omitted {path}"
                ))
            })?;
            let effective_attributes = effective.get(path).ok_or_else(|| {
                ResourceProtocolParseError::Malformed(format!(
                    "effective Git attribute policy omitted {path}"
                ))
            })?;
            let key = index_evidence_key(
                path,
                &resource.blob_oid,
                staged_attributes,
                effective_attributes,
            );
            if let Some((_, cached)) = records
                .get(path)
                .and_then(|record| record.index)
                .filter(|(cached_key, _)| *cached_key == key)
            {
                result.insert(
                    path.clone(),
                    EolValues {
                        index: cached.as_git().into(),
                        worktree: String::new(),
                    },
                );
                continuation.progress.staged_eol_retained += 1;
                continue;
            }
            if staged_eol_test_timeout() {
                if !pending.is_empty() {
                    self.commit_index_eol_batch(
                        repository_root,
                        &pending,
                        records,
                        continuation,
                        &mut result,
                        cancellation,
                        deadline,
                        completed_batches == 0,
                    )?;
                }
                return Err(ResourceProtocolParseError::TimedOut);
            }
            let size = *blob_sizes.get(&resource.blob_oid).ok_or_else(|| {
                ResourceProtocolParseError::Malformed(format!(
                    "Git omitted staged text blob metadata for {path}"
                ))
            })?;
            if !pending.is_empty()
                && (pending.len() >= continuation.index_eol_batch_files()
                    || pending_bytes.saturating_add(size) > MAX_INDEX_EOL_BATCH_BLOB_BYTES)
            {
                self.commit_index_eol_batch(
                    repository_root,
                    &pending,
                    records,
                    continuation,
                    &mut result,
                    cancellation,
                    deadline,
                    completed_batches == 0,
                )?;
                completed_batches += 1;
                pending.clear();
                pending_bytes = 0;
            }
            pending.push((resource, key));
            pending_bytes = pending_bytes.saturating_add(size);
        }
        if !pending.is_empty() {
            self.commit_index_eol_batch(
                repository_root,
                &pending,
                records,
                continuation,
                &mut result,
                cancellation,
                deadline,
                completed_batches == 0,
            )?;
        }
        // A completed page proves its paths for this snapshot; start the next
        // page at full throughput.
        continuation.index_eol_batch_files = None;
        Ok(result)
    }

    #[allow(clippy::too_many_arguments)]
    fn commit_index_eol_batch(
        &self,
        repository_root: &Path,
        batch: &[(&RepositoryResource, EvidenceKey)],
        records: &mut BTreeMap<String, EolRecord>,
        continuation: &mut ResourceContinuation,
        result: &mut BTreeMap<String, EolValues>,
        cancellation: &CancellationToken,
        deadline: ProviderDeadline,
        first_batch: bool,
    ) -> Result<(), ResourceProtocolParseError> {
        let budget_at_start = deadline.remaining();
        let resources = batch
            .iter()
            .map(|(resource, _)| *resource)
            .collect::<Vec<_>>();
        let inspected =
            self.inspect_index_eol_batch(repository_root, &resources, cancellation, deadline);
        let inspected = match inspected {
            Err(ResourceProtocolParseError::TimedOut) => {
                // A later batch normally inherits only the tail of this call's
                // deadline. Only a first batch given at least half of the
                // seven-second handoff window can indicate an oversized group.
                if first_batch
                    && budget_at_start
                        .is_some_and(|budget| budget >= self.response_handoff_window / 2)
                {
                    continuation.reduce_index_eol_batch(batch.len());
                }
                return Err(ResourceProtocolParseError::TimedOut);
            }
            other => other?,
        };
        // A timed-out, cancelled, truncated or malformed response must never
        // publish evidence for only a prefix of this batch.
        for ((resource, key), (index_eol, eol)) in batch.iter().zip(inspected) {
            records.entry(resource.repo_path.clone()).or_default().index = Some((*key, index_eol));
            continuation.progress.staged_eol_retained += 1;
            result.insert(resource.repo_path.clone(), eol);
        }
        Ok(())
    }

    fn inspect_index_eol_batch(
        &self,
        repository_root: &Path,
        batch: &[&RepositoryResource],
        cancellation: &CancellationToken,
        deadline: ProviderDeadline,
    ) -> Result<Vec<(IndexEol, EolValues)>, ResourceProtocolParseError> {
        let entries = batch
            .iter()
            .map(|resource| GitIndexEntry {
                repo_path: resource.repo_path.clone(),
                blob_oid: Some(resource.blob_oid.clone()),
                mode: Some("100644".into()),
            })
            .collect::<Vec<_>>();
        let index = self
            .create_isolated_index(
                repository_root,
                &entries.iter().collect::<Vec<_>>(),
                cancellation,
                deadline,
            )
            .map_err(|reason| resource_process_error(reason, cancellation, deadline))?;
        let command = isolated_git_command(
            repository_root,
            &index,
            vec!["ls-files".into(), "--eol".into(), "-z".into()],
            cancellation,
            deadline,
        );
        let checkpoint = &mut || resource_protocol_checkpoint(cancellation, deadline);
        let mut records = EolRecords::new(checkpoint)?;
        self.stream(&command, &[], 0, cancellation, deadline, &mut |record| {
            records.push(record, &mut || {
                resource_protocol_checkpoint(cancellation, deadline)
            })
        })?;
        if deadline.is_elapsed() {
            return Err(ResourceProtocolParseError::TimedOut);
        }
        let paths = entries
            .iter()
            .map(|entry| entry.repo_path.clone())
            .collect::<Vec<_>>();
        let records = records.finish(&paths, checkpoint)?;
        if records.len() != paths.len() {
            return Err(ResourceProtocolParseError::Malformed(
                "git ls-files --eol returned an unexpected path".into(),
            ));
        }
        let mut inspected = Vec::with_capacity(batch.len());
        for path in &paths {
            let eol = records
                .get(path)
                .expect("parser checks every expected path")
                .clone();
            let index_eol = IndexEol::from_git(&eol.index).ok_or_else(|| {
                ResourceProtocolParseError::Malformed(format!(
                    "git ls-files --eol returned unsupported index EOL metadata for {path}"
                ))
            })?;
            inspected.push((index_eol, eol));
        }
        resource_protocol_checkpoint(cancellation, deadline)?;
        Ok(inspected)
    }

    fn validate_resource_blob_sizes(
        &self,
        repository_root: &Path,
        resources: &[&RepositoryResource],
        cancellation: &CancellationToken,
        deadline: ProviderDeadline,
    ) -> Result<BTreeMap<String, usize>, String> {
        let entries = resources
            .iter()
            .map(|resource| GitIndexEntry {
                repo_path: resource.repo_path.clone(),
                blob_oid: Some(resource.blob_oid.clone()),
                mode: Some("100644".into()),
            })
            .collect::<Vec<_>>();
        self.validate_blob_sizes(
            repository_root,
            &entries.iter().collect::<Vec<_>>(),
            MAX_INDEX_EOL_FILE_BYTES,
            None,
            "staged text resource policy",
            cancellation,
            deadline,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn validate_blob_sizes(
        &self,
        repository_root: &Path,
        entries: &[&GitIndexEntry],
        max_file_bytes: usize,
        max_total_bytes: Option<usize>,
        label: &str,
        cancellation: &CancellationToken,
        deadline: ProviderDeadline,
    ) -> Result<BTreeMap<String, usize>, String> {
        let mut unique_oids = BTreeSet::new();
        for entry in entries {
            if cancellation.is_cancelled() {
                return Err(format!("{label} inspection was cancelled"));
            }
            if deadline.is_elapsed() {
                return Err(format!("{label} inspection timed out"));
            }
            let oid = entry.blob_oid.as_deref().ok_or_else(|| {
                format!(
                    "{label} path does not have one regular stage-0 blob: {}",
                    entry.repo_path
                )
            })?;
            if !entry
                .mode
                .as_deref()
                .is_some_and(|mode| matches!(mode, "100644" | "100755"))
            {
                return Err(format!(
                    "{label} path does not have one regular stage-0 blob: {}",
                    entry.repo_path
                ));
            }
            unique_oids.insert(oid.to_owned());
        }
        let mut input = Vec::new();
        for oid in &unique_oids {
            input.extend_from_slice(oid.as_bytes());
            input.push(b'\n');
        }
        let mut by_oid = BTreeMap::<String, usize>::new();
        if !unique_oids.is_empty() {
            let command = process_command(
                repository_root,
                vec![
                    "--no-replace-objects".into(),
                    "cat-file".into(),
                    "--batch-check=%(objectname) %(objecttype) %(objectsize)".into(),
                ],
                cancellation,
                deadline,
            );
            self.stream(
                &command,
                &input,
                b'\n',
                cancellation,
                deadline,
                &mut |line| {
                    let line = std::str::from_utf8(line).map_err(|_| {
                        ResourceProtocolParseError::Malformed(format!(
                            "Git returned non-UTF-8 {label} blob metadata"
                        ))
                    })?;
                    let fields = line.split_whitespace().collect::<Vec<_>>();
                    if fields.len() != 3 || fields[1] != "blob" {
                        return Err(ResourceProtocolParseError::Malformed(format!(
                            "Git returned invalid {label} blob metadata: {line}"
                        )));
                    }
                    let size = fields[2].parse::<usize>().map_err(|_| {
                        ResourceProtocolParseError::Malformed(format!(
                            "Git returned an invalid blob size: {line}"
                        ))
                    })?;
                    if by_oid.insert(fields[0].to_owned(), size).is_some() {
                        return Err(ResourceProtocolParseError::Malformed(format!(
                            "Git returned duplicate {label} blob metadata"
                        )));
                    }
                    Ok(())
                },
            )
            .map_err(|error| match error {
                ResourceProtocolParseError::Malformed(reason) => reason,
                ResourceProtocolParseError::Cancelled => {
                    format!("{label} inspection was cancelled")
                }
                ResourceProtocolParseError::TimedOut => format!("{label} inspection timed out"),
            })?;
            if by_oid.len() != unique_oids.len()
                || unique_oids.iter().any(|oid| !by_oid.contains_key(oid))
            {
                return Err(format!("Git omitted {label} blob metadata"));
            }
        }
        let mut total = 0_usize;
        for entry in entries {
            let oid = entry
                .blob_oid
                .as_deref()
                .expect("validated staged blob has an object id");
            let size = by_oid[oid];
            if size > max_file_bytes {
                return Err(format!(
                    "{label} path {} is {size} bytes; inspection supports at most {max_file_bytes} bytes per file",
                    entry.repo_path
                ));
            }
            total = total.saturating_add(size);
            if let Some(max_total_bytes) = max_total_bytes.filter(|bound| total > *bound) {
                return Err(format!(
                    "{label} is {total} bytes; inspection supports at most {max_total_bytes} bytes in total"
                ));
            }
        }
        Ok(by_oid)
    }

    fn create_isolated_index(
        &self,
        repository_root: &Path,
        entries: &[&GitIndexEntry],
        cancellation: &CancellationToken,
        deadline: ProviderDeadline,
    ) -> Result<IsolatedGitIndex, String> {
        let root = TempDir::new()
            .map_err(|error| format!("create isolated Git inspection directory: {error}"))?;
        let git_dir = root.path().join("git");
        let worktree = root.path().join("worktree");
        let index = root.path().join("index");
        fs::create_dir_all(git_dir.join("refs/heads"))
            .map_err(|error| format!("create isolated Git refs: {error}"))?;
        fs::create_dir_all(git_dir.join("objects"))
            .map_err(|error| format!("create isolated Git objects directory: {error}"))?;
        fs::create_dir_all(&worktree)
            .map_err(|error| format!("create isolated Git worktree: {error}"))?;
        fs::write(git_dir.join("HEAD"), b"ref: refs/heads/unborn\n")
            .map_err(|error| format!("write isolated Git HEAD: {error}"))?;
        let object_output = self.run(
            repository_root,
            [
                "rev-parse",
                "--path-format=absolute",
                "--git-path",
                "objects",
            ],
            cancellation,
            deadline,
        )?;
        if !semantic_success(&object_output) {
            return Err(output_reason(&object_output));
        }
        let object_directory_text = object_output.stdout.strip_suffix('\n').ok_or_else(|| {
            "Git object-directory output is missing its terminal newline".to_string()
        })?;
        let object_directory = PathBuf::from(object_directory_text);
        if !object_directory.is_absolute() {
            return Err("Git returned a non-absolute object directory".into());
        }
        let object_format_output = self.run(
            repository_root,
            ["rev-parse", "--show-object-format"],
            cancellation,
            deadline,
        )?;
        if !semantic_success(&object_format_output) {
            return Err(output_reason(&object_format_output));
        }
        let object_format = object_format_output
            .stdout
            .strip_suffix('\n')
            .ok_or_else(|| {
                "Git object-format output is missing its terminal newline".to_string()
            })?;
        let isolated_config = match object_format {
            "sha1" => "[core]\nrepositoryformatversion = 0\nbare = false\n".to_string(),
            "sha256" => "[core]\nrepositoryformatversion = 1\nbare = false\n[extensions]\nobjectFormat = sha256\n".to_string(),
            other => return Err(format!("unsupported Git object format: {other}")),
        };
        fs::write(git_dir.join("config"), isolated_config)
            .map_err(|error| format!("write isolated Git config: {error}"))?;
        let isolated = IsolatedGitIndex {
            _root: root,
            git_dir,
            worktree,
            index,
            object_directory,
        };
        let initialize = isolated_git_command(
            repository_root,
            &isolated,
            vec!["read-tree".into(), "--empty".into()],
            cancellation,
            deadline,
        );
        let output = sticky_process_result(self.runner.run(&initialize), cancellation)?;
        if !semantic_success(&output) {
            return Err(output_reason(&output));
        }
        if !entries.is_empty() {
            let mut index_info = Vec::new();
            for entry in entries {
                let path = Path::new(&entry.repo_path);
                if path.is_absolute()
                    || path
                        .components()
                        .any(|component| !matches!(component, std::path::Component::Normal(_)))
                {
                    return Err(format!(
                        "staged policy path is not repository-relative: {}",
                        entry.repo_path
                    ));
                }
                let mode = entry.mode.as_deref().ok_or_else(|| {
                    format!("staged policy path has no mode: {}", entry.repo_path)
                })?;
                let oid = entry.blob_oid.as_deref().ok_or_else(|| {
                    format!("staged policy path has no blob: {}", entry.repo_path)
                })?;
                index_info
                    .extend_from_slice(format!("{mode} {oid}\t{}\0", entry.repo_path).as_bytes());
            }
            let update = isolated_git_command(
                repository_root,
                &isolated,
                vec!["update-index".into(), "-z".into(), "--index-info".into()],
                cancellation,
                deadline,
            );
            let output = sticky_process_result(
                self.runner.run_with_input(&update, &index_info),
                cancellation,
            )?;
            if !semantic_success(&output) {
                return Err(output_reason(&output));
            }
        }
        Ok(isolated)
    }
}

impl ResourcePass<'_, '_> {
    fn checkpoint(&self) -> Result<(), ResourceProtocolParseError> {
        resource_protocol_checkpoint(self.cancellation, self.deadline)
    }

    fn classification_checkpoint(&self) -> Result<(), ResourceClassificationError> {
        resource_classification_checkpoint(self.cancellation, self.deadline)
    }

    fn classification_stop(&mut self, error: ResourceClassificationError) -> Stop {
        let mut observations = std::mem::take(&mut self.observations);
        let (reason, fact) = match error {
            ResourceClassificationError::Cancelled => return Stop::Cancelled,
            ResourceClassificationError::TimedOut => (
                "resource ownership classification exceeded the inspection deadline".to_string(),
                ProjectHealthFact::GitInspectionTimeout {
                    check: ProjectCheckId::RepositoryAttributes,
                    source_set: None,
                },
            ),
            ResourceClassificationError::Ownership(error) => {
                let reason = format!(
                    "resource ownership could not be proven from repository root {}",
                    error.repo_path
                );
                (
                    reason.clone(),
                    ProjectHealthFact::GitInspectionIncomplete {
                        check: ProjectCheckId::RepositoryAttributes,
                        source_set: None,
                        reason,
                    },
                )
            }
        };
        for check in resource_check_ids() {
            mark_check_not_run(&mut observations, check, &reason);
        }
        Stop::Answer(RepositoryPolicyInspection {
            observations,
            facts: vec![fact],
        })
    }

    /// An attribute or fence failure ends the inspection with `check` and its
    /// dependents not run.
    fn protocol_stop(&mut self, error: ResourceProtocolParseError, check: ProjectCheckId) -> Stop {
        let observations = std::mem::take(&mut self.observations);
        match error {
            ResourceProtocolParseError::Cancelled => Stop::Cancelled,
            ResourceProtocolParseError::TimedOut => {
                Stop::Answer(timeout_policy(observations, check))
            }
            ResourceProtocolParseError::Malformed(reason) => {
                Stop::Answer(incomplete_policy(observations, check, reason))
            }
        }
    }

    /// Classify the index in path order and inspect it a page at a time.
    fn pages(&mut self, classifier: &ResourceClassifier<'_>) -> Result<(), Stop> {
        let mut page = Vec::new();
        for (entry_index, entry) in self.entries.iter().enumerate() {
            if entry_index % 256 == 0 {
                if let Err(error) = self.classification_checkpoint() {
                    return Err(self.classification_stop(error));
                }
            }
            let classified = classifier.classify(entry, &mut || {
                resource_classification_checkpoint(self.cancellation, self.deadline)
            });
            match classified {
                Ok(Some(ClassifiedEntry::Resource(resource))) => {
                    page.push(resource);
                    if page.len() == RESOURCE_PAGE_PATHS {
                        self.page(std::mem::take(&mut page))?;
                    }
                }
                Ok(Some(ClassifiedEntry::Ownership(error))) => {
                    let reason = bounded_resource_ownership_reason(&error);
                    for source_set in error.source_sets {
                        let (count, _) = self
                            .ownership
                            .entry(source_set)
                            .or_insert_with(|| (0, reason.clone()));
                        *count = count.checked_add(1).ok_or_else(|| {
                            Stop::Fatal("resource ownership count overflow".into())
                        })?;
                    }
                }
                Ok(None) => {}
                Err(error) => return Err(self.classification_stop(error)),
            }
        }
        if !page.is_empty() {
            self.page(page)?;
        }
        Ok(())
    }

    fn page(&mut self, page: Vec<RepositoryResource>) -> Result<(), Stop> {
        let inspected = self.inspect_page(page);
        let records = std::mem::take(&mut self.records);
        let written = match self.generation.as_mut() {
            Some(generation) => generation.write(&records),
            None => Ok(()),
        };
        inspected?;
        written.map_err(|error| Stop::Fatal(continuation_error_reason(&error)))
    }

    fn inspect_page(&mut self, mut page: Vec<RepositoryResource>) -> Result<(), Stop> {
        // An ownership error found after a resource was paged excludes the
        // rest of its source set; `complete` drops what was already found.
        page.retain(|resource| !self.ownership.contains_key(&resource.source_set));
        if page.is_empty() {
            return Ok(());
        }
        let paths = page
            .iter()
            .map(|resource| resource.repo_path.clone())
            .collect::<Vec<_>>();
        let effective_command = process_command(
            self.repository_root,
            [
                "check-attr",
                "-z",
                "--cached",
                "text",
                "eol",
                "filter",
                "--stdin",
            ]
            .into_iter()
            .map(str::to_owned)
            .collect(),
            self.cancellation,
            self.deadline,
        );
        let effective = self
            .inspector
            .attributes(&effective_command, &paths, self.cancellation, self.deadline)
            .map_err(|error| self.protocol_stop(error, ProjectCheckId::RepositoryAttributes))?;
        let staged_command = self.staged_attribute_command()?;
        let staged = self
            .inspector
            .attributes(&staged_command, &paths, self.cancellation, self.deadline)
            .map_err(|error| self.protocol_stop(error, ProjectCheckId::RepositoryAttributes))?;
        for resource in &page {
            let attributes = &effective[&resource.repo_path];
            let digest = self
                .effective
                .entry(resource.source_set.clone())
                .or_default();
            for field in [
                resource.repo_path.as_str(),
                &attributes.text,
                &attributes.eol,
                &attributes.filter,
            ] {
                hash_evidence_field(digest, field);
            }
        }
        let policy =
            evaluate_attribute_policy_with_checkpoint(&page, &staged, &effective, &mut || {
                self.checkpoint()
            })
            .map_err(|error| self.protocol_stop(error, ProjectCheckId::RepositoryAttributes))?;
        self.attribute_facts
            .extend(policy.facts)
            .map_err(Stop::Fatal)?;
        if self.index_stop.is_some() {
            return Ok(());
        }
        self.index_eol_page(&page, &staged, &effective, &policy.text_marked_binary)?;
        if self.index_stop.is_some() {
            return Ok(());
        }
        self.working_page(&page, &staged, &policy.text_marked_binary)
    }

    /// `check-attr` that reads only the staged `.gitattributes` policy. The
    /// isolated index holding that policy is built once, for the first page.
    fn staged_attribute_command(&mut self) -> Result<ProcessCommand, Stop> {
        if self.staged_index.is_none() {
            let staged_attribute_entries = self
                .entries
                .iter()
                .filter(|entry| {
                    Path::new(&entry.repo_path)
                        .file_name()
                        .and_then(|name| name.to_str())
                        == Some(".gitattributes")
                })
                .collect::<Vec<_>>();
            if staged_attribute_entries.len() > MAX_STAGED_POLICY_FILES {
                return Err(Stop::Answer(incomplete_policy(
                    std::mem::take(&mut self.observations),
                    ProjectCheckId::RepositoryAttributes,
                    format!(
                        "staged attribute policy contains {} .gitattributes files; inspection supports at most {MAX_STAGED_POLICY_FILES}",
                        staged_attribute_entries.len()
                    ),
                )));
            }
            let prepared = self
                .inspector
                .validate_blob_sizes(
                    self.repository_root,
                    &staged_attribute_entries,
                    MAX_STAGED_POLICY_TOTAL_BYTES,
                    Some(MAX_STAGED_POLICY_TOTAL_BYTES),
                    "staged attribute policy",
                    self.cancellation,
                    self.deadline,
                )
                .and_then(|_| {
                    self.inspector.create_isolated_index(
                        self.repository_root,
                        &staged_attribute_entries,
                        self.cancellation,
                        self.deadline,
                    )
                });
            match prepared {
                Ok(index) => self.staged_index = Some(index),
                Err(_) if self.cancellation.is_cancelled() => return Err(Stop::Cancelled),
                Err(reason) => {
                    return Err(Stop::Answer(incomplete_policy(
                        std::mem::take(&mut self.observations),
                        ProjectCheckId::RepositoryAttributes,
                        reason,
                    )))
                }
            }
        }
        let staged_index = self.staged_index.as_ref().expect("prepared above");
        Ok(isolated_git_command(
            self.repository_root,
            staged_index,
            vec![
                "-c".into(),
                format!(
                    "core.attributesFile={}",
                    host_path_text(
                        staged_index
                            ._root
                            .path()
                            .join("empty-attributes")
                            .display()
                            .to_string()
                    )
                ),
                "check-attr".into(),
                "-z".into(),
                "--cached".into(),
                "text".into(),
                "eol".into(),
                "filter".into(),
                "--stdin".into(),
            ],
            self.cancellation,
            self.deadline,
        ))
    }

    fn stop_index(
        &mut self,
        error: ResourceProtocolParseError,
        dependents: bool,
    ) -> Result<(), Stop> {
        let (reason, fact) = match error {
            ResourceProtocolParseError::Cancelled => return Err(Stop::Cancelled),
            ResourceProtocolParseError::TimedOut => (
                "Git resource protocol parsing exceeded the inspection deadline".to_string(),
                ProjectHealthFact::GitInspectionTimeout {
                    check: ProjectCheckId::RepositoryIndexEol,
                    source_set: None,
                },
            ),
            ResourceProtocolParseError::Malformed(reason) => (
                reason.clone(),
                ProjectHealthFact::GitInspectionIncomplete {
                    check: ProjectCheckId::RepositoryIndexEol,
                    source_set: None,
                    reason,
                },
            ),
        };
        self.index_stop = Some(IndexEolStop {
            reason,
            fact,
            dependents,
        });
        Ok(())
    }

    fn index_eol_page(
        &mut self,
        page: &[RepositoryResource],
        staged: &BTreeMap<String, AttributeValues>,
        effective: &BTreeMap<String, AttributeValues>,
        text_marked_binary: &BTreeSet<String>,
    ) -> Result<(), Stop> {
        let text = page
            .iter()
            .filter(|resource| resource.kind == RepositoryResourceKind::Text)
            .collect::<Vec<_>>();
        if text.is_empty() {
            return Ok(());
        }
        if let Some(state) = self.continuation.as_deref_mut() {
            state.progress.text_resources += text.len();
        }
        let blob_sizes = match self.inspector.validate_resource_blob_sizes(
            self.repository_root,
            &text,
            self.cancellation,
            self.deadline,
        ) {
            Ok(sizes) => sizes,
            Err(_) if self.cancellation.is_cancelled() => return Err(Stop::Cancelled),
            Err(reason) => {
                return self.stop_index(ResourceProtocolParseError::Malformed(reason), false)
            }
        };
        let inspected = match (self.continuation.as_deref_mut(), self.generation.as_mut()) {
            (Some(state), Some(generation)) => {
                let text_paths = text
                    .iter()
                    .map(|resource| resource.repo_path.as_str())
                    .collect::<Vec<_>>();
                self.records = generation
                    .take(&text_paths)
                    .map_err(|error| Stop::Fatal(continuation_error_reason(&error)))?;
                self.inspector.incremental_index_eol(
                    self.repository_root,
                    &text,
                    &blob_sizes,
                    staged,
                    effective,
                    &mut self.records,
                    state,
                    self.cancellation,
                    self.deadline,
                )
            }
            _ => self
                .inspector
                .inspect_index_eol_batch(
                    self.repository_root,
                    &text,
                    self.cancellation,
                    self.deadline,
                )
                .map(|inspected| {
                    text.iter()
                        .zip(inspected)
                        .map(|(resource, (_, eol))| (resource.repo_path.clone(), eol))
                        .collect()
                }),
        };
        let eol = match inspected {
            Ok(eol) => eol,
            Err(error) => return self.stop_index(error, true),
        };
        let facts =
            match index_eol_facts_with_checkpoint(page, &eol, text_marked_binary, &mut || {
                self.checkpoint()
            }) {
                Ok(facts) => facts,
                Err(ResourceProtocolParseError::TimedOut) => {
                    self.index_stop = Some(IndexEolStop {
                        reason: "Git index EOL evaluation exceeded the inspection deadline".into(),
                        fact: ProjectHealthFact::GitInspectionTimeout {
                            check: ProjectCheckId::RepositoryIndexEol,
                            source_set: None,
                        },
                        dependents: false,
                    });
                    return Ok(());
                }
                Err(error) => return self.stop_index(error, true),
            };
        self.index_eol_facts.extend(facts).map_err(Stop::Fatal)
    }

    fn working_page(
        &mut self,
        page: &[RepositoryResource],
        staged: &BTreeMap<String, AttributeValues>,
        text_marked_binary: &BTreeSet<String>,
    ) -> Result<(), Stop> {
        for resource in page {
            if self.cancellation.is_cancelled() {
                return Err(Stop::Cancelled);
            }
            if self.deadline.is_elapsed() {
                match resource.kind {
                    RepositoryResourceKind::Text => self.working_timed_out = true,
                    RepositoryResourceKind::Binary => self.lfs_timed_out = true,
                }
                continue;
            }
            match resource.kind {
                RepositoryResourceKind::Text => {
                    // `-text` is the primary policy defect for a known platform XML
                    // resource. Git deliberately stops treating that path as text, so
                    // any EOL interpretation derived from its working bytes would be a
                    // misleading secondary diagnostic.
                    if text_marked_binary.contains(&resource.repo_path) {
                        if let Some(record) = self.records.get_mut(&resource.repo_path) {
                            record.working = None;
                        }
                        continue;
                    }
                    let path = Path::new(&resource.repo_path);
                    let inspected = if let Some(state) = self.continuation.as_deref_mut() {
                        let previous = self
                            .records
                            .get(&resource.repo_path)
                            .and_then(|record| record.working);
                        let (inspected, evidence) = inspect_working_eol_continued(
                            self.repository_root,
                            path,
                            previous,
                            &mut state.progress,
                            self.cancellation,
                            self.deadline,
                        );
                        match evidence {
                            Some(evidence) => {
                                self.records
                                    .entry(resource.repo_path.clone())
                                    .or_default()
                                    .working = Some(evidence);
                            }
                            None => {
                                if let Some(record) = self.records.get_mut(&resource.repo_path) {
                                    record.working = None;
                                }
                            }
                        }
                        inspected
                    } else {
                        inspect_working_eol(
                            self.repository_root,
                            path,
                            self.cancellation,
                            self.deadline,
                        )
                    };
                    let fact = match inspected {
                        Ok(Some(WorkingEol::Mixed)) => Some(ProjectHealthFact::MixedEol {
                            source_set: resource.source_set.clone(),
                            path: resource.repo_path.clone(),
                        }),
                        Ok(Some(WorkingEol::BareCr)) => {
                            Some(ProjectHealthFact::WorkingEolUnsupported {
                                source_set: resource.source_set.clone(),
                                path: resource.repo_path.clone(),
                                observed: "cr".into(),
                            })
                        }
                        Ok(Some(WorkingEol::Supported) | None) => None,
                        Err(WorkingEolInspectionError::Cancelled) => return Err(Stop::Cancelled),
                        Err(WorkingEolInspectionError::TimedOut) => {
                            self.working_timed_out = true;
                            None
                        }
                        Err(WorkingEolInspectionError::Incomplete(reason)) => {
                            let reason = format!("{}: {reason}", resource.repo_path);
                            self.working_incomplete.insert(resource.source_set.clone());
                            mark_source_check_not_run(
                                &mut self.observations,
                                ProjectCheckId::RepositoryWorkingEol,
                                &resource.source_set,
                                &reason,
                            );
                            self.facts.push(ProjectHealthFact::GitInspectionIncomplete {
                                check: ProjectCheckId::RepositoryWorkingEol,
                                source_set: Some(resource.source_set.clone()),
                                reason,
                            });
                            None
                        }
                    };
                    if let Some(fact) = fact {
                        self.working_facts.push(fact).map_err(Stop::Fatal)?;
                    }
                }
                RepositoryResourceKind::Binary => {
                    if staged[&resource.repo_path].filter == "lfs" {
                        continue;
                    }
                    match repository_regular_file_size(
                        self.repository_root,
                        Path::new(&resource.repo_path),
                    ) {
                        Ok(Some(size)) => self
                            .lfs
                            .entry(resource.source_set.clone())
                            .or_default()
                            .add(&resource.repo_path, size)
                            .map_err(Stop::Fatal)?,
                        Ok(None) => {}
                        Err(reason) => {
                            let reason = format!("{}: {reason}", resource.repo_path);
                            self.lfs_incomplete.insert(resource.source_set.clone());
                            mark_source_check_not_run(
                                &mut self.observations,
                                ProjectCheckId::RepositoryLfs,
                                &resource.source_set,
                                &reason,
                            );
                            self.facts.push(ProjectHealthFact::GitInspectionIncomplete {
                                check: ProjectCheckId::RepositoryLfs,
                                source_set: Some(resource.source_set.clone()),
                                reason,
                            });
                        }
                    }
                }
            }
        }
        Ok(())
    }

    /// Publish the checks once every page was inspected.
    fn complete(
        &mut self,
        classifier: &ResourceClassifier<'_>,
    ) -> Result<RepositoryPolicyInspection, Stop> {
        let incomplete = self.ownership.keys().cloned().collect::<BTreeSet<_>>();
        let aggregate_reason = format!(
            "resource ownership is incomplete for {} source sets",
            incomplete.len()
        );
        for observation in &mut self.observations {
            if !resource_check_ids().contains(&observation.id)
                || matches!(
                    observation.outcome,
                    ProjectCheckOutcome::NotApplicable { .. }
                )
            {
                continue;
            }
            let reason = match observation.source_set.as_deref() {
                Some(source_set) => self.ownership.get(source_set).map(|(_, reason)| reason),
                None if !incomplete.is_empty() => Some(&aggregate_reason),
                None => None,
            };
            if let Some(reason) = reason {
                observation.outcome = ProjectCheckOutcome::NotRun {
                    reason: reason.clone(),
                };
            }
        }
        let mut facts = self
            .ownership
            .iter()
            .map(|(source_set, (count, reason))| {
                ProjectHealthFact::GitInspectionIncompleteCounted {
                    check: ProjectCheckId::RepositoryAttributes,
                    source_set: Some(source_set.clone()),
                    reason: reason.clone(),
                    count: *count,
                }
            })
            .collect::<Vec<_>>();
        let complete_set = |source_set: Option<&str>| {
            source_set.is_none_or(|source_set| !incomplete.contains(source_set))
        };
        let mut attribute_facts = std::mem::take(&mut self.attribute_facts);
        attribute_facts.retain_source_sets(complete_set);
        facts.extend(attribute_facts.into_facts());
        mark_check_completed(
            &mut self.observations,
            ProjectCheckId::RepositoryAttributes,
            &incomplete,
        );
        if let Some(stop) = self.index_stop.as_ref() {
            mark_check_not_run(
                &mut self.observations,
                ProjectCheckId::RepositoryIndexEol,
                &stop.reason,
            );
            if stop.dependents {
                for check in [
                    ProjectCheckId::RepositoryWorkingEol,
                    ProjectCheckId::RepositoryLfs,
                ] {
                    mark_check_not_run(&mut self.observations, check, &stop.reason);
                }
            }
            facts.push(stop.fact.clone());
            return Ok(RepositoryPolicyInspection {
                observations: std::mem::take(&mut self.observations),
                facts,
            });
        }
        let mut index_eol_facts = std::mem::take(&mut self.index_eol_facts);
        index_eol_facts.retain_source_sets(complete_set);
        facts.extend(index_eol_facts.into_facts());
        mark_check_completed(
            &mut self.observations,
            ProjectCheckId::RepositoryIndexEol,
            &incomplete,
        );
        facts.append(&mut self.facts);

        let continued = self.continuation.is_some();
        if continued && !self.working_timed_out {
            run_working_revalidation_test_hook();
        }
        if self.cancellation.is_cancelled() {
            return Err(Stop::Cancelled);
        }
        let working_revalidation_error = if continued && !self.working_timed_out {
            self.revalidate_working()?
        } else {
            None
        };
        if let Some(reason) = working_revalidation_error {
            mark_check_not_run(
                &mut self.observations,
                ProjectCheckId::RepositoryWorkingEol,
                &reason,
            );
            facts.push(ProjectHealthFact::GitInspectionIncomplete {
                check: ProjectCheckId::RepositoryWorkingEol,
                source_set: None,
                reason,
            });
        } else if self.working_timed_out || (continued && self.deadline.is_elapsed()) {
            mark_check_not_run(
                &mut self.observations,
                ProjectCheckId::RepositoryWorkingEol,
                "deadline expired during working EOL inspection",
            );
            facts.push(ProjectHealthFact::GitInspectionTimeout {
                check: ProjectCheckId::RepositoryWorkingEol,
                source_set: None,
            });
        } else {
            let working_incomplete = incomplete
                .union(&self.working_incomplete)
                .cloned()
                .collect::<BTreeSet<_>>();
            let mut working_facts = std::mem::take(&mut self.working_facts);
            working_facts.retain_source_sets(|source_set| {
                source_set.is_none_or(|source_set| !working_incomplete.contains(source_set))
            });
            mark_check_completed(
                &mut self.observations,
                ProjectCheckId::RepositoryWorkingEol,
                &working_incomplete,
            );
            facts.extend(working_facts.into_facts());
        }
        if self.lfs_timed_out {
            mark_check_not_run(
                &mut self.observations,
                ProjectCheckId::RepositoryLfs,
                "deadline expired during LFS size inspection",
            );
            facts.push(ProjectHealthFact::GitInspectionTimeout {
                check: ProjectCheckId::RepositoryLfs,
                source_set: None,
            });
        } else {
            let lfs_incomplete = incomplete
                .union(&self.lfs_incomplete)
                .cloned()
                .collect::<BTreeSet<_>>();
            let mut lfs = std::mem::take(&mut self.lfs);
            lfs.retain(|source_set, _| !lfs_incomplete.contains(source_set));
            match lfs_facts_with_checkpoint(lfs, &mut || self.checkpoint()) {
                Ok(lfs_facts) => {
                    facts.extend(lfs_facts);
                    mark_check_completed(
                        &mut self.observations,
                        ProjectCheckId::RepositoryLfs,
                        &lfs_incomplete,
                    );
                }
                Err(ResourceProtocolParseError::Cancelled) => return Err(Stop::Cancelled),
                Err(ResourceProtocolParseError::TimedOut) => {
                    mark_check_not_run(
                        &mut self.observations,
                        ProjectCheckId::RepositoryLfs,
                        "deadline expired during LFS aggregation",
                    );
                    facts.push(ProjectHealthFact::GitInspectionTimeout {
                        check: ProjectCheckId::RepositoryLfs,
                        source_set: None,
                    });
                }
                Err(ResourceProtocolParseError::Malformed(reason)) => {
                    mark_check_not_run(
                        &mut self.observations,
                        ProjectCheckId::RepositoryLfs,
                        &reason,
                    );
                    facts.push(ProjectHealthFact::GitInspectionIncomplete {
                        check: ProjectCheckId::RepositoryLfs,
                        source_set: None,
                        reason,
                    });
                }
            }
        }
        if self.cancellation.is_cancelled() {
            return Err(Stop::Cancelled);
        }
        if continued
            && !self.observations.iter().any(|observation| {
                resource_check_ids().contains(&observation.id)
                    && matches!(observation.outcome, ProjectCheckOutcome::NotRun { .. })
            })
        {
            // A failed final fence cannot substantiate any earlier resource
            // finding. The helpers restart NotRun at RepositoryAttributes and
            // intentionally discard the facts from the old index snapshot.
            if self.deadline.is_elapsed() {
                return Err(self.protocol_stop(
                    ResourceProtocolParseError::TimedOut,
                    ProjectCheckId::RepositoryAttributes,
                ));
            }
            self.index_unchanged()?;
            self.effective_attributes_unchanged(classifier, &incomplete)?;
        }
        Ok(RepositoryPolicyInspection {
            observations: std::mem::take(&mut self.observations),
            facts,
        })
    }

    /// Every working file proven in this call still has the observed
    /// fingerprint, or is still absent. The witnesses are the records of the
    /// generation this call wrote.
    fn revalidate_working(&mut self) -> Result<Option<String>, Stop> {
        let Some(generation) = self.generation.as_mut() else {
            return Ok(None);
        };
        let Some(mut written) = generation
            .written()
            .map_err(|error| Stop::Fatal(continuation_error_reason(&error)))?
        else {
            return Ok(None);
        };
        while let Some((path, record)) = written
            .next()
            .map_err(|error| Stop::Fatal(continuation_error_reason(&error)))?
        {
            if self.cancellation.is_cancelled() || self.deadline.is_elapsed() {
                return Ok(None);
            }
            let expected = match record.working {
                Some(WorkingEvidence::Absent) => None,
                Some(WorkingEvidence::Complete { fingerprint, .. }) => Some(fingerprint),
                Some(WorkingEvidence::Partial { .. }) | None => continue,
            };
            let current =
                open_repository_regular_file_nofollow(self.repository_root, Path::new(&path))
                    .and_then(|file| working_file_fingerprint(&file));
            match (expected, current) {
                (Some(expected), Ok(current)) if current == expected => {}
                (None, Err(error)) if error.kind() == std::io::ErrorKind::NotFound => {}
                (_, Ok(_)) | (None, Err(_)) => {
                    return Ok(Some(format!(
                        "working path {path} changed during EOL inspection"
                    )))
                }
                (Some(_), Err(error)) => {
                    return Ok(Some(format!(
                        "working path {path} could not be revalidated: {error}"
                    )))
                }
            }
        }
        Ok(None)
    }

    fn index_unchanged(&mut self) -> Result<(), Stop> {
        let mut expected = GitIndexDigest::default();
        for entry in self.entries {
            expected.update(entry);
        }
        let mut current = GitIndexDigest::default();
        let read = read_git_index(
            self.inspector.runner,
            self.repository_root,
            self.cancellation,
            self.deadline,
            &mut |entry| current.update(&entry),
        );
        let error = match read {
            Ok(()) if current.finish() == expected.finish() => return Ok(()),
            Ok(()) => ResourceProtocolParseError::Malformed(
                "Git index changed during resource inspection".into(),
            ),
            Err(IndexReadError::Cancelled) => return Err(Stop::Cancelled),
            Err(IndexReadError::ProcessTimedOut | IndexReadError::ParseTimedOut) => {
                ResourceProtocolParseError::TimedOut
            }
            Err(
                IndexReadError::Failed(reason)
                | IndexReadError::Incomplete(reason)
                | IndexReadError::Malformed(reason),
            ) => ResourceProtocolParseError::Malformed(reason),
        };
        Err(self.protocol_stop(error, ProjectCheckId::RepositoryAttributes))
    }

    /// The effective attributes of every complete source set are what the
    /// pages observed, compared by digest page by page again.
    fn effective_attributes_unchanged(
        &mut self,
        classifier: &ResourceClassifier<'_>,
        incomplete: &BTreeSet<String>,
    ) -> Result<(), Stop> {
        let mut current = BTreeMap::<String, Sha256>::new();
        let mut paths = Vec::new();
        let mut owners = Vec::new();
        for (entry_index, entry) in self.entries.iter().enumerate() {
            let last = entry_index + 1 == self.entries.len();
            if entry_index % 256 == 0 {
                if let Err(error) = self.classification_checkpoint() {
                    return Err(self.classification_stop(error));
                }
            }
            match classifier.classify(entry, &mut || {
                resource_classification_checkpoint(self.cancellation, self.deadline)
            }) {
                Ok(Some(ClassifiedEntry::Resource(resource)))
                    if !incomplete.contains(&resource.source_set) =>
                {
                    paths.push(resource.repo_path);
                    owners.push(resource.source_set);
                }
                Ok(_) => {}
                Err(error) => return Err(self.classification_stop(error)),
            }
            if paths.is_empty() || (paths.len() < RESOURCE_PAGE_PATHS && !last) {
                continue;
            }
            let command = process_command(
                self.repository_root,
                [
                    "check-attr",
                    "-z",
                    "--cached",
                    "text",
                    "eol",
                    "filter",
                    "--stdin",
                ]
                .into_iter()
                .map(str::to_owned)
                .collect(),
                self.cancellation,
                self.deadline,
            );
            let effective = self
                .inspector
                .attributes(&command, &paths, self.cancellation, self.deadline)
                .map_err(|error| self.protocol_stop(error, ProjectCheckId::RepositoryAttributes))?;
            for (path, source_set) in paths.drain(..).zip(owners.drain(..)) {
                let attributes = &effective[&path];
                let digest = current.entry(source_set).or_default();
                for field in [
                    path.as_str(),
                    &attributes.text,
                    &attributes.eol,
                    &attributes.filter,
                ] {
                    hash_evidence_field(digest, field);
                }
            }
        }
        let observed = std::mem::take(&mut self.effective)
            .into_iter()
            .filter(|(source_set, _)| !incomplete.contains(source_set))
            .map(|(source_set, digest)| (source_set, digest.finalize()))
            .collect::<BTreeMap<_, _>>();
        let current = current
            .into_iter()
            .map(|(source_set, digest)| (source_set, digest.finalize()))
            .collect::<BTreeMap<_, _>>();
        if observed == current {
            Ok(())
        } else {
            Err(self.protocol_stop(
                ResourceProtocolParseError::Malformed(
                    "effective Git attributes changed during resource inspection".into(),
                ),
                ProjectCheckId::RepositoryAttributes,
            ))
        }
    }
}

fn resource_classification_checkpoint(
    cancellation: &CancellationToken,
    deadline: ProviderDeadline,
) -> Result<(), ResourceClassificationError> {
    if cancellation.is_cancelled() {
        Err(ResourceClassificationError::Cancelled)
    } else if deadline.is_elapsed() {
        Err(ResourceClassificationError::TimedOut)
    } else {
        Ok(())
    }
}

fn continuation_error_reason(error: &std::io::Error) -> String {
    format!("project health resource continuation could not be read or written: {error}")
}

fn continuation_error(error: std::io::Error) -> ProjectHealthInspectionError {
    ProjectHealthInspectionError::Fatal(continuation_error_reason(&error))
}

pub(crate) fn classify_platform_xml_relative_path(
    normalized_relative_path: &str,
) -> Option<RepositoryResourceKind> {
    let segments = normalized_relative_path.split('/').collect::<Vec<_>>();
    if segments.len() == 4
        && segments[0] == "XDTOPackages"
        && !segments[1].is_empty()
        && segments[2] == "Ext"
        && segments[3] == "Package.bin"
    {
        return Some(RepositoryResourceKind::Text);
    }
    let extension = Path::new(normalized_relative_path)
        .extension()
        .and_then(|extension| extension.to_str())?
        .to_ascii_lowercase();
    if matches!(extension.as_str(), "xml" | "bsl") {
        Some(RepositoryResourceKind::Text)
    } else if matches!(
        extension.as_str(),
        "bin"
            | "axdt"
            | "addin"
            | "cf"
            | "cfe"
            | "epf"
            | "erf"
            | "png"
            | "jpg"
            | "jpeg"
            | "gif"
            | "bmp"
            | "ico"
            | "zip"
            | "7z"
            | "gz"
    ) {
        Some(RepositoryResourceKind::Binary)
    } else {
        None
    }
}

#[cfg(test)]
fn parse_attribute_records(
    stdout: &str,
    expected_paths: &[String],
) -> Result<BTreeMap<String, AttributeValues>, String> {
    parse_attribute_records_with_checkpoint(stdout, expected_paths, &mut || Ok(()))
        .map_err(ResourceProtocolParseError::into_reason)
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ResourceProtocolParseError {
    Malformed(String),
    Cancelled,
    TimedOut,
}

impl ResourceProtocolParseError {
    #[cfg(test)]
    fn into_reason(self) -> String {
        match self {
            Self::Malformed(reason) => reason,
            Self::Cancelled => "Git resource protocol parsing was cancelled".into(),
            Self::TimedOut => "Git resource protocol parsing timed out".into(),
        }
    }
}

type ProtocolCheckpoint<'c> = &'c mut dyn FnMut() -> Result<(), ResourceProtocolParseError>;

#[cfg(test)]
fn parse_attribute_records_with_checkpoint(
    stdout: &str,
    expected_paths: &[String],
    checkpoint: ProtocolCheckpoint<'_>,
) -> Result<BTreeMap<String, AttributeValues>, ResourceProtocolParseError> {
    let mut records = AttributeRecords::new(expected_paths, checkpoint)?;
    if !stdout.is_empty() && !stdout.ends_with('\0') {
        return Err(ResourceProtocolParseError::Malformed(
            "Git check-attr output is missing its terminal NUL".into(),
        ));
    }
    for field in stdout.split_terminator('\0') {
        records.push(field.as_bytes(), checkpoint)?;
    }
    records.finish(checkpoint)
}

/// Reads the NUL-terminated fields of `git check-attr -z`: a path, an
/// attribute and its value per triple, for exactly the expected paths.
struct AttributeRecords<'p> {
    expected_paths: &'p [String],
    expected: BTreeSet<&'p str>,
    raw: BTreeMap<String, BTreeMap<String, String>>,
    triple: Vec<String>,
    triples: usize,
}

impl<'p> AttributeRecords<'p> {
    fn new(
        expected_paths: &'p [String],
        checkpoint: ProtocolCheckpoint<'_>,
    ) -> Result<Self, ResourceProtocolParseError> {
        checkpoint()?;
        let mut expected = BTreeSet::new();
        for (path_index, path) in expected_paths.iter().enumerate() {
            if path_index.is_multiple_of(256) {
                checkpoint()?;
            }
            expected.insert(path.as_str());
        }
        Ok(Self {
            expected_paths,
            expected,
            raw: BTreeMap::new(),
            triple: Vec::with_capacity(3),
            triples: 0,
        })
    }

    fn push(
        &mut self,
        field: &[u8],
        checkpoint: ProtocolCheckpoint<'_>,
    ) -> Result<(), ResourceProtocolParseError> {
        if self.triple.is_empty() && self.triples.is_multiple_of(256) {
            checkpoint()?;
        }
        let field = std::str::from_utf8(field).map_err(|_| {
            ResourceProtocolParseError::Malformed(
                "Git check-attr output contained invalid UTF-8".into(),
            )
        })?;
        self.triple.push(field.to_owned());
        if self.triple.len() < 3 {
            return Ok(());
        }
        let value = self.triple.pop().expect("a complete triple");
        let attribute = self.triple.pop().expect("a complete triple");
        let path = self.triple.pop().expect("a complete triple");
        if !self.expected.contains(path.as_str()) {
            return Err(ResourceProtocolParseError::Malformed(format!(
                "Git check-attr returned unexpected path {path}"
            )));
        }
        if self
            .raw
            .entry(path)
            .or_default()
            .insert(attribute, value)
            .is_some()
        {
            return Err(ResourceProtocolParseError::Malformed(
                "Git check-attr returned a duplicate attribute".into(),
            ));
        }
        self.triples += 1;
        Ok(())
    }

    fn finish(
        mut self,
        checkpoint: ProtocolCheckpoint<'_>,
    ) -> Result<BTreeMap<String, AttributeValues>, ResourceProtocolParseError> {
        if !self.triple.is_empty() {
            return Err(ResourceProtocolParseError::Malformed(
                "Git check-attr output is not a sequence of triples".into(),
            ));
        }
        let mut records = BTreeMap::new();
        for (path_index, path) in self.expected_paths.iter().enumerate() {
            if path_index.is_multiple_of(256) {
                checkpoint()?;
            }
            let attributes = self.raw.remove(path).ok_or_else(|| {
                ResourceProtocolParseError::Malformed(format!("Git check-attr omitted {path}"))
            })?;
            if attributes.len() != 3 {
                return Err(ResourceProtocolParseError::Malformed(format!(
                    "Git check-attr returned incomplete attributes for {path}"
                )));
            }
            records.insert(
                path.clone(),
                AttributeValues {
                    text: required_attribute(&attributes, "text", path)
                        .map_err(ResourceProtocolParseError::Malformed)?,
                    eol: required_attribute(&attributes, "eol", path)
                        .map_err(ResourceProtocolParseError::Malformed)?,
                    filter: required_attribute(&attributes, "filter", path)
                        .map_err(ResourceProtocolParseError::Malformed)?,
                },
            );
        }
        Ok(records)
    }
}

fn required_attribute(
    attributes: &BTreeMap<String, String>,
    name: &str,
    path: &str,
) -> Result<String, String> {
    attributes
        .get(name)
        .cloned()
        .ok_or_else(|| format!("Git check-attr omitted {name} for {path}"))
}

#[cfg(test)]
fn parse_eol_records(
    stdout: &str,
    expected_text_paths: &[String],
) -> Result<BTreeMap<String, EolValues>, String> {
    parse_eol_records_with_checkpoint(stdout, expected_text_paths, &mut || Ok(()))
        .map_err(ResourceProtocolParseError::into_reason)
}

#[cfg(test)]
fn parse_eol_records_with_checkpoint(
    stdout: &str,
    expected_text_paths: &[String],
    checkpoint: ProtocolCheckpoint<'_>,
) -> Result<BTreeMap<String, EolValues>, ResourceProtocolParseError> {
    let mut records = EolRecords::new(checkpoint)?;
    if !stdout.is_empty() && !stdout.ends_with('\0') {
        return Err(ResourceProtocolParseError::Malformed(
            "git ls-files --eol output is missing its terminal NUL".into(),
        ));
    }
    for record in stdout.split_terminator('\0') {
        records.push(record.as_bytes(), checkpoint)?;
    }
    records.finish(expected_text_paths, checkpoint)
}

/// Reads the NUL-terminated records of `git ls-files --eol -z`.
struct EolRecords {
    result: BTreeMap<String, EolValues>,
    records: usize,
}

impl EolRecords {
    fn new(checkpoint: ProtocolCheckpoint<'_>) -> Result<Self, ResourceProtocolParseError> {
        checkpoint()?;
        Ok(Self {
            result: BTreeMap::new(),
            records: 0,
        })
    }

    fn push(
        &mut self,
        record: &[u8],
        checkpoint: ProtocolCheckpoint<'_>,
    ) -> Result<(), ResourceProtocolParseError> {
        if self.records.is_multiple_of(256) {
            checkpoint()?;
        }
        self.records += 1;
        let record = std::str::from_utf8(record).map_err(|_| {
            ResourceProtocolParseError::Malformed(
                "git ls-files --eol output contained invalid UTF-8".into(),
            )
        })?;
        let (metadata, path) = record.split_once('\t').ok_or_else(|| {
            ResourceProtocolParseError::Malformed(
                "git ls-files --eol record has no tab separator".into(),
            )
        })?;
        if path.is_empty() {
            return Err(ResourceProtocolParseError::Malformed(
                "git ls-files --eol record has an empty path".into(),
            ));
        }
        let fields = metadata.split_whitespace().collect::<Vec<_>>();
        if fields.len() < 2 || !fields[0].starts_with("i/") || !fields[1].starts_with("w/") {
            return Err(ResourceProtocolParseError::Malformed(
                "git ls-files --eol record has invalid metadata".into(),
            ));
        }
        let index = &fields[0][2..];
        let worktree = &fields[1][2..];
        if !matches!(index, "" | "lf" | "crlf" | "mixed" | "none" | "-text") {
            return Err(ResourceProtocolParseError::Malformed(format!(
                "git ls-files --eol returned unsupported index EOL metadata {index}"
            )));
        }
        if !matches!(worktree, "" | "lf" | "crlf" | "mixed" | "none" | "-text") {
            return Err(ResourceProtocolParseError::Malformed(format!(
                "git ls-files --eol returned unsupported worktree EOL metadata {worktree}"
            )));
        }
        if self
            .result
            .insert(
                path.into(),
                EolValues {
                    index: index.into(),
                    worktree: worktree.into(),
                },
            )
            .is_some()
        {
            return Err(ResourceProtocolParseError::Malformed(
                "git ls-files --eol returned a duplicate path".into(),
            ));
        }
        Ok(())
    }

    fn finish(
        self,
        expected_text_paths: &[String],
        checkpoint: ProtocolCheckpoint<'_>,
    ) -> Result<BTreeMap<String, EolValues>, ResourceProtocolParseError> {
        for (path_index, path) in expected_text_paths.iter().enumerate() {
            if path_index.is_multiple_of(256) {
                checkpoint()?;
            }
            let observed = self.result.get(path).ok_or_else(|| {
                ResourceProtocolParseError::Malformed(format!("git ls-files --eol omitted {path}"))
            })?;
            if observed.index.is_empty() {
                return Err(ResourceProtocolParseError::Malformed(format!(
                    "git ls-files --eol returned empty index EOL metadata for tracked text path {path}"
                )));
            }
        }
        Ok(self.result)
    }
}

fn resource_protocol_checkpoint(
    cancellation: &CancellationToken,
    deadline: ProviderDeadline,
) -> Result<(), ResourceProtocolParseError> {
    if cancellation.is_cancelled() {
        Err(ResourceProtocolParseError::Cancelled)
    } else if deadline.is_elapsed() {
        Err(ResourceProtocolParseError::TimedOut)
    } else {
        Ok(())
    }
}

fn resource_process_error(
    reason: String,
    cancellation: &CancellationToken,
    deadline: ProviderDeadline,
) -> ResourceProtocolParseError {
    if cancellation.is_cancelled() {
        ResourceProtocolParseError::Cancelled
    } else if deadline.is_elapsed() {
        ResourceProtocolParseError::TimedOut
    } else {
        ResourceProtocolParseError::Malformed(reason)
    }
}

fn evaluate_attribute_policy_with_checkpoint(
    resources: &[RepositoryResource],
    staged: &BTreeMap<String, AttributeValues>,
    effective: &BTreeMap<String, AttributeValues>,
    checkpoint: &mut dyn FnMut() -> Result<(), ResourceProtocolParseError>,
) -> Result<AttributePolicyEvaluation, ResourceProtocolParseError> {
    let mut evaluation = AttributePolicyEvaluation {
        facts: Vec::new(),
        text_marked_binary: BTreeSet::new(),
    };
    for (resource_index, resource) in resources.iter().enumerate() {
        if resource_index.is_multiple_of(256) {
            checkpoint()?;
        }
        let attributes = staged.get(&resource.repo_path).ok_or_else(|| {
            ResourceProtocolParseError::Malformed(format!(
                "staged Git attribute policy omitted {}",
                resource.repo_path
            ))
        })?;
        let effective_attributes = effective.get(&resource.repo_path).ok_or_else(|| {
            ResourceProtocolParseError::Malformed(format!(
                "effective Git attribute policy omitted {}",
                resource.repo_path
            ))
        })?;
        let staged_policy_sufficient = policy_satisfied(resource.kind, attributes);
        if !staged_policy_sufficient && policy_satisfied(resource.kind, effective_attributes) {
            evaluation
                .facts
                .push(ProjectHealthFact::AttributesLocalOnly {
                    source_set: resource.source_set.clone(),
                    path: resource.repo_path.clone(),
                    evidence: vec![format!(
                        "local-only text={} eol={} filter={}",
                        effective_attributes.text,
                        effective_attributes.eol,
                        effective_attributes.filter
                    )],
                });
            continue;
        }
        match resource.kind {
            RepositoryResourceKind::Text if attributes.text == "unset" => {
                evaluation
                    .text_marked_binary
                    .insert(resource.repo_path.clone());
                evaluation
                    .facts
                    .push(ProjectHealthFact::TextResourceMarkedBinary {
                        source_set: resource.source_set.clone(),
                        path: resource.repo_path.clone(),
                    });
            }
            RepositoryResourceKind::Text if !text_policy_satisfied(attributes) => {
                evaluation.facts.push(ProjectHealthFact::TextPolicyMissing {
                    source_set: resource.source_set.clone(),
                    path: resource.repo_path.clone(),
                });
            }
            RepositoryResourceKind::Binary if attributes.text != "unset" => {
                evaluation
                    .facts
                    .push(ProjectHealthFact::BinaryPolicyMissing {
                        source_set: resource.source_set.clone(),
                        path: resource.repo_path.clone(),
                    });
            }
            RepositoryResourceKind::Text | RepositoryResourceKind::Binary => {}
        }
    }
    checkpoint()?;
    Ok(evaluation)
}

fn index_eol_facts_with_checkpoint(
    resources: &[RepositoryResource],
    eol: &BTreeMap<String, EolValues>,
    text_marked_binary: &BTreeSet<String>,
    checkpoint: &mut dyn FnMut() -> Result<(), ResourceProtocolParseError>,
) -> Result<Vec<ProjectHealthFact>, ResourceProtocolParseError> {
    let mut facts = Vec::new();
    for (resource_index, resource) in resources.iter().enumerate() {
        if resource_index.is_multiple_of(256) {
            checkpoint()?;
        }
        if resource.kind != RepositoryResourceKind::Text
            || text_marked_binary.contains(&resource.repo_path)
        {
            continue;
        }
        let observed = eol.get(&resource.repo_path).ok_or_else(|| {
            ResourceProtocolParseError::Malformed(format!(
                "git ls-files --eol omitted {}",
                resource.repo_path
            ))
        })?;
        if !matches!(observed.index.as_str(), "lf" | "none") {
            facts.push(ProjectHealthFact::IndexEolNotLf {
                source_set: resource.source_set.clone(),
                path: resource.repo_path.clone(),
                observed: observed.index.clone(),
            });
        }
    }
    checkpoint()?;
    Ok(facts)
}

fn lfs_facts_with_checkpoint(
    aggregates: BTreeMap<String, LfsAggregate>,
    checkpoint: ProtocolCheckpoint<'_>,
) -> Result<Vec<ProjectHealthFact>, ResourceProtocolParseError> {
    let mut facts = Vec::new();
    for (source_index, (source_set, aggregate)) in aggregates.into_iter().enumerate() {
        if source_index.is_multiple_of(256) {
            checkpoint()?;
        }
        if let Some((largest_path, largest_bytes)) = aggregate.largest.filter(|(_, largest)| {
            *largest >= LFS_SINGLE_FILE_THRESHOLD_BYTES
                || aggregate.total_bytes >= LFS_AGGREGATE_THRESHOLD_BYTES
        }) {
            facts.push(ProjectHealthFact::LfsConsider {
                source_set,
                count: aggregate.count,
                total_bytes: aggregate.total_bytes,
                largest_path,
                largest_bytes,
                single_threshold_bytes: LFS_SINGLE_FILE_THRESHOLD_BYTES,
                aggregate_threshold_bytes: LFS_AGGREGATE_THRESHOLD_BYTES,
                paths: aggregate.paths.into_iter().collect(),
            });
        }
    }
    checkpoint()?;
    Ok(facts)
}

fn process_command(
    cwd: &Path,
    args: Vec<String>,
    cancellation: &CancellationToken,
    deadline: ProviderDeadline,
) -> ProcessCommand {
    ProcessCommand {
        program: PathBuf::from("git"),
        args,
        cwd: cwd.to_path_buf(),
        env: vec![
            (OsString::from("LC_ALL"), OsString::from("C")),
            (OsString::from("LANG"), OsString::from("C")),
            (OsString::from("GIT_NO_LAZY_FETCH"), OsString::from("1")),
            (
                OsString::from("GIT_NO_REPLACE_OBJECTS"),
                OsString::from("1"),
            ),
            (OsString::from("GIT_OPTIONAL_LOCKS"), OsString::from("0")),
            (OsString::from("GIT_TERMINAL_PROMPT"), OsString::from("0")),
            (OsString::from("GIT_CONFIG_COUNT"), OsString::from("1")),
            (
                OsString::from("GIT_CONFIG_KEY_0"),
                OsString::from("core.fsmonitor"),
            ),
            (
                OsString::from("GIT_CONFIG_VALUE_0"),
                OsString::from("false"),
            ),
            (OsString::from("GIT_ATTR_NOSYSTEM"), OsString::from("1")),
        ],
        env_remove: super::git::git_environment_removals(),
        capture_limits: Some((
            super::PROJECT_HEALTH_STDOUT_CAPTURE_LIMIT,
            unica_process::STDERR_CAPTURE_LIMIT,
        )),
        timeout: deadline.remaining(),
        cancellation: cancellation.clone(),
    }
}

fn isolated_git_command(
    cwd: &Path,
    index: &IsolatedGitIndex,
    args: Vec<String>,
    cancellation: &CancellationToken,
    deadline: ProviderDeadline,
) -> ProcessCommand {
    let mut command = process_command(cwd, args, cancellation, deadline);
    command.args.splice(
        0..0,
        [
            format!(
                "--git-dir={}",
                host_path_text(index.git_dir.display().to_string())
            ),
            format!(
                "--work-tree={}",
                host_path_text(index.worktree.display().to_string())
            ),
        ],
    );
    command.env.extend([
        (
            OsString::from("GIT_INDEX_FILE"),
            index.index.as_os_str().to_os_string(),
        ),
        (
            OsString::from("GIT_OBJECT_DIRECTORY"),
            index.object_directory.as_os_str().to_os_string(),
        ),
    ]);
    command
}

fn semantic_success(output: &ProcessOutput) -> bool {
    output.status_success
        && !output.timed_out
        && !output.cancelled
        && !output.stdout_truncated
        && !output.stderr_truncated
        && !output.stdout_had_invalid_utf8
        && !output.stderr_had_invalid_utf8
}

fn sticky_process_result(
    result: Result<ProcessOutput, String>,
    cancellation: &CancellationToken,
) -> Result<ProcessOutput, String> {
    if cancellation.is_cancelled() {
        return Ok(ProcessOutput {
            cancelled: true,
            status_success: false,
            status: "cancelled".into(),
            stdout: String::new(),
            stderr: String::new(),
            timed_out: false,
            stdout_truncated: false,
            stderr_truncated: false,
            stdout_had_invalid_utf8: false,
            stderr_had_invalid_utf8: false,
        });
    }
    result
}

fn output_reason(output: &ProcessOutput) -> String {
    if output.timed_out {
        "Git resource inspection timed out".into()
    } else if output.stdout_truncated || output.stderr_truncated {
        "Git resource inspection output was truncated".into()
    } else if output.stdout_had_invalid_utf8 || output.stderr_had_invalid_utf8 {
        "Git resource inspection output contained invalid UTF-8".into()
    } else if output.stderr.trim().is_empty() {
        format!("Git resource inspection exited with {}", output.status)
    } else {
        format!(
            "Git resource inspection exited with {}: {}",
            output.status,
            output.stderr.trim()
        )
    }
}

fn incomplete_policy(
    mut observations: Vec<ProjectCheckObservation>,
    check: ProjectCheckId,
    reason: String,
) -> RepositoryPolicyInspection {
    let started = resource_check_ids()
        .into_iter()
        .position(|candidate| candidate == check)
        .unwrap_or(0);
    for dependent in resource_check_ids().into_iter().skip(started) {
        mark_check_not_run(&mut observations, dependent, &reason);
    }
    RepositoryPolicyInspection {
        observations,
        facts: vec![ProjectHealthFact::GitInspectionIncomplete {
            check,
            source_set: None,
            reason,
        }],
    }
}

fn timeout_policy(
    mut observations: Vec<ProjectCheckObservation>,
    check: ProjectCheckId,
) -> RepositoryPolicyInspection {
    let reason = "Git resource protocol parsing exceeded the inspection deadline";
    let started = resource_check_ids()
        .into_iter()
        .position(|candidate| candidate == check)
        .unwrap_or(0);
    for dependent in resource_check_ids().into_iter().skip(started) {
        mark_check_not_run(&mut observations, dependent, reason);
    }
    RepositoryPolicyInspection {
        observations,
        facts: vec![ProjectHealthFact::GitInspectionTimeout {
            check,
            source_set: None,
        }],
    }
}

fn mark_check_completed(
    observations: &mut [ProjectCheckObservation],
    check: ProjectCheckId,
    incomplete_source_sets: &BTreeSet<String>,
) {
    for observation in observations.iter_mut().filter(|observation| {
        observation.id == check
            && match observation.source_set.as_ref() {
                Some(source_set) => !incomplete_source_sets.contains(source_set),
                None => incomplete_source_sets.is_empty(),
            }
    }) {
        if matches!(observation.outcome, ProjectCheckOutcome::NotRun { .. }) {
            observation.outcome = ProjectCheckOutcome::Completed;
        }
    }
}

fn mark_check_not_run(
    observations: &mut [ProjectCheckObservation],
    check: ProjectCheckId,
    reason: &str,
) {
    for observation in observations.iter_mut().filter(|observation| {
        observation.id == check
            && !matches!(
                observation.outcome,
                ProjectCheckOutcome::NotApplicable { .. }
            )
    }) {
        observation.outcome = ProjectCheckOutcome::NotRun {
            reason: reason.to_owned(),
        };
    }
}

fn mark_source_check_not_run(
    observations: &mut [ProjectCheckObservation],
    check: ProjectCheckId,
    source_set: &str,
    reason: &str,
) {
    for observation in observations.iter_mut().filter(|observation| {
        observation.id == check
            && observation.source_set.as_deref() == Some(source_set)
            && !matches!(
                observation.outcome,
                ProjectCheckOutcome::NotApplicable { .. }
            )
    }) {
        observation.outcome = ProjectCheckOutcome::NotRun {
            reason: reason.to_owned(),
        };
    }
}

fn resource_check_ids() -> [ProjectCheckId; 4] {
    [
        ProjectCheckId::RepositoryAttributes,
        ProjectCheckId::RepositoryIndexEol,
        ProjectCheckId::RepositoryWorkingEol,
        ProjectCheckId::RepositoryLfs,
    ]
}

pub(super) fn resource_observations<'a>(
    source_sets: impl IntoIterator<Item = &'a unica_source_contract::ProjectSourceSet>,
    not_run_reason: &str,
    source_profiles_complete: bool,
) -> Vec<ProjectCheckObservation> {
    let mut source_sets = source_sets.into_iter().collect::<Vec<_>>();
    source_sets.sort_by(|left, right| left.name.cmp(&right.name));
    source_sets.dedup_by(|left, right| left.name == right.name);
    let has_platform_xml = source_sets
        .iter()
        .any(|source_set| source_set.source_format == SourceFormat::PlatformXml);
    let mut observations = Vec::new();
    for id in resource_check_ids() {
        observations.push(ProjectCheckObservation {
            id,
            scope: id.scope(),
            source_set: None,
            outcome: if source_profiles_complete && !has_platform_xml {
                ProjectCheckOutcome::NotApplicable {
                    reason: "no Platform XML source roots were proven".into(),
                }
            } else {
                ProjectCheckOutcome::NotRun {
                    reason: not_run_reason.into(),
                }
            },
        });
        observations.extend(
            source_sets
                .iter()
                .map(|source_set| ProjectCheckObservation {
                    id,
                    scope: id.scope(),
                    source_set: Some(source_set.name.clone()),
                    outcome: if source_set.source_format == SourceFormat::Edt {
                        ProjectCheckOutcome::NotApplicable {
                            reason: "EDT has no format-specific repository resource policy".into(),
                        }
                    } else {
                        ProjectCheckOutcome::NotRun {
                            reason: not_run_reason.into(),
                        }
                    },
                }),
        );
    }
    observations
}

fn text_policy_satisfied(attributes: &AttributeValues) -> bool {
    attributes.text != "unset"
        && (matches!(attributes.text.as_str(), "set" | "auto")
            || matches!(attributes.eol.as_str(), "lf" | "crlf"))
}

fn policy_satisfied(kind: RepositoryResourceKind, attributes: &AttributeValues) -> bool {
    match kind {
        RepositoryResourceKind::Text => text_policy_satisfied(attributes),
        RepositoryResourceKind::Binary => attributes.text == "unset",
    }
}

fn nul_input(paths: &[String]) -> Vec<u8> {
    let mut input = Vec::new();
    for path in paths {
        input.extend_from_slice(path.as_bytes());
        input.push(0);
    }
    input
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WorkingEol {
    Supported,
    Mixed,
    BareCr,
}

impl WorkingEol {
    fn from_byte(value: u8) -> Option<Self> {
        [Self::Supported, Self::Mixed, Self::BareCr]
            .into_iter()
            .find(|eol| *eol as u8 == value)
    }
}

#[derive(Debug, PartialEq, Eq)]
enum WorkingEolInspectionError {
    Cancelled,
    TimedOut,
    Incomplete(String),
}

impl std::fmt::Display for WorkingEolInspectionError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Cancelled => formatter.write_str("working EOL inspection was cancelled"),
            Self::TimedOut => formatter.write_str("deadline expired during working EOL inspection"),
            Self::Incomplete(reason) => formatter.write_str(reason),
        }
    }
}

fn inspect_working_eol(
    repository_root: &Path,
    repo_path: &Path,
    cancellation: &CancellationToken,
    deadline: ProviderDeadline,
) -> Result<Option<WorkingEol>, WorkingEolInspectionError> {
    let file = match open_repository_regular_file_nofollow(repository_root, repo_path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(WorkingEolInspectionError::Incomplete(error.to_string())),
    };
    inspect_working_eol_reader(file, cancellation, deadline)
}

/// Continue the working EOL scan of `repo_path` from its `previous`
/// evidence. Returns the result and the evidence to keep for the path:
/// `Partial` when the deadline stopped inside the file, `None` when nothing
/// may be reused.
fn inspect_working_eol_continued(
    repository_root: &Path,
    repo_path: &Path,
    previous: Option<WorkingEvidence>,
    progress: &mut ResourceInspectionProgress,
    cancellation: &CancellationToken,
    deadline: ProviderDeadline,
) -> (
    Result<Option<WorkingEol>, WorkingEolInspectionError>,
    Option<WorkingEvidence>,
) {
    let incomplete = |error: std::io::Error| {
        (
            Err(WorkingEolInspectionError::Incomplete(error.to_string())),
            None,
        )
    };
    let mut file = match open_repository_regular_file_nofollow(repository_root, repo_path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return (Ok(None), Some(WorkingEvidence::Absent))
        }
        Err(error) => return incomplete(error),
    };
    let before = match working_file_fingerprint(&file) {
        Ok(fingerprint) => fingerprint,
        Err(error) => return incomplete(error),
    };
    let mut scanner = match previous {
        Some(WorkingEvidence::Complete { fingerprint, eol }) if fingerprint == before => {
            progress.working_eol_retained += 1;
            return (Ok(Some(eol)), previous);
        }
        Some(WorkingEvidence::Partial {
            fingerprint,
            scanner,
        }) if fingerprint == before => scanner,
        _ => WorkingEolScanner::default(),
    };
    let size = match file.metadata() {
        Ok(metadata) => metadata.len(),
        Err(error) => return incomplete(error),
    };
    if scanner.offset > size {
        return (
            Err(WorkingEolInspectionError::Incomplete(
                "working EOL checkpoint exceeds file size".into(),
            )),
            None,
        );
    }
    if scanner.offset != 0 {
        if let Err(error) = file.seek(SeekFrom::Start(scanner.offset)) {
            return incomplete(error);
        }
    }
    let eol = match inspect_working_eol_reader_from_state(
        &mut file,
        &mut scanner,
        cancellation,
        deadline,
    ) {
        Ok(eol) => eol,
        Err(WorkingEolInspectionError::TimedOut) => {
            // Keep the original fingerprint, never a newer one attached to an
            // old prefix. Reopening on the next request validates this state;
            // saving it performs no filesystem work after the deadline.
            return (
                Err(WorkingEolInspectionError::TimedOut),
                Some(WorkingEvidence::Partial {
                    fingerprint: before,
                    scanner,
                }),
            );
        }
        Err(error) => return (Err(error), None),
    };
    match working_file_fingerprint(&file) {
        Ok(after) if after == before => {}
        Ok(_) => {
            return (
                Err(WorkingEolInspectionError::Incomplete(
                    "working file changed during EOL inspection".into(),
                )),
                None,
            )
        }
        Err(error) => return incomplete(error),
    }
    match open_repository_regular_file_nofollow(repository_root, repo_path)
        .and_then(|named| working_file_fingerprint(&named))
    {
        Ok(named) if named == before => {}
        Ok(_) => {
            return (
                Err(WorkingEolInspectionError::Incomplete(
                    "working path changed during EOL inspection".into(),
                )),
                None,
            )
        }
        Err(error) => return incomplete(error),
    }
    match eol {
        Some(eol) => {
            progress.working_eol_retained += 1;
            (
                Ok(Some(eol)),
                Some(WorkingEvidence::Complete {
                    fingerprint: before,
                    eol,
                }),
            )
        }
        None => (Ok(None), None),
    }
}

fn working_file_fingerprint(file: &File) -> std::io::Result<FileFingerprint> {
    let metadata = file.metadata()?;
    let mut hasher = Sha256::new();
    hasher.update(b"unica-working-file-v1\0");
    hasher.update(file.identity()?.stable_bytes());
    hasher.update(metadata.len().to_be_bytes());
    match metadata.modified()?.duration_since(std::time::UNIX_EPOCH) {
        Ok(since) => {
            hasher.update([1]);
            hasher.update(since.as_secs().to_be_bytes());
            hasher.update(since.subsec_nanos().to_be_bytes());
        }
        Err(before) => {
            hasher.update([0]);
            hasher.update(before.duration().as_secs().to_be_bytes());
            hasher.update(before.duration().subsec_nanos().to_be_bytes());
        }
    }
    let (seconds, nanoseconds) = file.change_time(&metadata)?;
    hasher.update(seconds.to_be_bytes());
    hasher.update(nanoseconds.to_be_bytes());
    Ok(hasher.finalize().into())
}

fn inspect_working_eol_reader(
    file: impl Read,
    cancellation: &CancellationToken,
    deadline: ProviderDeadline,
) -> Result<Option<WorkingEol>, WorkingEolInspectionError> {
    inspect_working_eol_reader_from_state(
        file,
        &mut WorkingEolScanner::default(),
        cancellation,
        deadline,
    )
}

fn working_eol_checkpoint(
    cancellation: &CancellationToken,
    deadline: ProviderDeadline,
) -> Result<(), WorkingEolInspectionError> {
    if cancellation.is_cancelled() {
        return Err(WorkingEolInspectionError::Cancelled);
    }
    if deadline.is_elapsed() {
        return Err(WorkingEolInspectionError::TimedOut);
    }
    Ok(())
}

fn inspect_working_eol_reader_from_state(
    mut file: impl Read,
    scanner: &mut WorkingEolScanner,
    cancellation: &CancellationToken,
    deadline: ProviderDeadline,
) -> Result<Option<WorkingEol>, WorkingEolInspectionError> {
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        working_eol_checkpoint(cancellation, deadline)?;
        let count = file
            .read(&mut buffer)
            .map_err(|error| WorkingEolInspectionError::Incomplete(error.to_string()))?;
        if count == 0 {
            working_eol_checkpoint(cancellation, deadline)?;
            break;
        }
        let next_offset = scanner.offset.checked_add(count as u64).ok_or_else(|| {
            WorkingEolInspectionError::Incomplete("working EOL offset overflow".into())
        })?;
        for byte in &buffer[..count] {
            if scanner.previous_cr {
                if *byte == b'\n' {
                    scanner.crlf = true;
                    scanner.previous_cr = false;
                    continue;
                }
                scanner.bare_cr = true;
                scanner.previous_cr = false;
            }
            match *byte {
                b'\r' => scanner.previous_cr = true,
                b'\n' => scanner.lf = true,
                _ => {}
            }
        }
        scanner.offset = next_offset;
        #[cfg(any(test, feature = "test-support"))]
        if let Some(hook) = AFTER_WORKING_EOL_BLOCK.with(|slot| slot.borrow_mut().take()) {
            hook()?;
        }
    }
    let bare_cr = scanner.bare_cr || scanner.previous_cr;
    let styles = usize::from(scanner.lf) + usize::from(scanner.crlf) + usize::from(bare_cr);
    Ok(Some(if styles > 1 {
        WorkingEol::Mixed
    } else if bare_cr {
        WorkingEol::BareCr
    } else {
        WorkingEol::Supported
    }))
}

fn open_repository_regular_file_nofollow(
    repository_root: &Path,
    repo_path: &Path,
) -> std::io::Result<File> {
    use std::path::Component;

    if !repository_root.is_absolute() || repo_path.is_absolute() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "repository root must be absolute and resource path relative",
        ));
    }
    let mut components = repo_path.components().peekable();
    let mut parent = open_absolute_read_directory_nofollow(repository_root)?;
    while let Some(component) = components.next() {
        let Component::Normal(name) = component else {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "resource path contains a non-normal component",
            ));
        };
        let (classification_anchor, kind) = open_any_read_child_nofollow(&parent, name)?;
        if components.peek().is_some() {
            if kind != OpenedChildKind::Directory {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "resource path contains a linked or non-directory parent",
                ));
            }
            parent = open_read_child_for_secure_tree_use(
                &parent,
                name,
                classification_anchor,
                OpenedChildKind::Directory,
            )?;
        } else if kind == OpenedChildKind::RegularFile {
            return open_read_child_for_secure_tree_use(
                &parent,
                name,
                classification_anchor,
                OpenedChildKind::RegularFile,
            );
        } else {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "resource path is linked or is not a regular file",
            ));
        }
    }
    Err(std::io::Error::new(
        std::io::ErrorKind::InvalidInput,
        "resource path is empty",
    ))
}

fn repository_regular_file_size(
    repository_root: &Path,
    repo_path: &Path,
) -> Result<Option<u64>, String> {
    let file = match open_repository_regular_file_nofollow(repository_root, repo_path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.to_string()),
    };
    let metadata = file.metadata().map_err(|error| error.to_string())?;
    if !metadata.is_file() {
        return Err("binary working path is not a regular file".into());
    }
    Ok(Some(metadata.len()))
}

#[cfg(test)]
#[path = "../../tests/platform/project_health_resources.rs"]
mod tests;
