use crate::application::receipt_ledger::ReceiptKeyDigest;
use crate::infrastructure::platform::filesystem::{
    create_owner_only_directory_child, create_owner_only_file_child, file_identity,
    open_absolute_directory_path_nofollow, open_directory_child_nofollow,
    open_directory_ownership_lock, open_regular_child_nofollow, sync_directory,
    verify_owner_only_acl, RetainedDirectoryCapability, RetainedRegularFileCapability,
};
use fs2::FileExt;
use std::ffi::OsStr;
use std::fmt;
use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Component, Path};
use std::sync::Mutex;

const ACTIVE_DIRECTORY_NAME: &str = "active";
const GENERATION_FILE_NAME: &str = "generation";
const LEDGER_LOCK_FILE_NAME: &str = ".receipt-ledger.lock";
const MAX_GENERATION_FILE_BYTES: usize = 32;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MissingReceiptObservation {
    receipt_key_digest: ReceiptKeyDigest,
    generation_before: u64,
    generation_after: u64,
}

impl MissingReceiptObservation {
    pub(crate) fn receipt_key_digest(&self) -> &ReceiptKeyDigest {
        &self.receipt_key_digest
    }

    pub(crate) const fn generation_before(&self) -> u64 {
        self.generation_before
    }

    pub(crate) const fn generation_after(&self) -> u64 {
        self.generation_after
    }
}

/// Opaque proof that the retained ledger authority and its generation stayed
/// stable across a complete read observation.
///
/// Only `ReceiptLedgerStore` can construct this value. Feature-only production
/// reachability owners may consume its read-only generation evidence, but they
/// cannot forge a successful validation step.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct StableReceiptLedgerObservation {
    generation_before: u64,
    generation_after: u64,
}

impl StableReceiptLedgerObservation {
    pub(crate) const fn generation_before(&self) -> u64 {
        self.generation_before
    }

    pub(crate) const fn generation_after(&self) -> u64 {
        self.generation_after
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ReceiptLedgerError {
    AlreadyOwned,
    ConcurrentGenerationChange {
        generation_before: u64,
        generation_after: u64,
    },
    Corrupt(&'static str),
    ReceiptRowPresentUnsupported,
    Storage {
        operation: &'static str,
        message: String,
    },
}

impl fmt::Display for ReceiptLedgerError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AlreadyOwned => formatter.write_str("receipt ledger is already owned"),
            Self::ConcurrentGenerationChange {
                generation_before,
                generation_after,
            } => write!(
                formatter,
                "receipt ledger generation changed during exact inspection: {generation_before} -> {generation_after}"
            ),
            Self::Corrupt(message) => write!(formatter, "corrupt receipt ledger: {message}"),
            Self::ReceiptRowPresentUnsupported => {
                formatter.write_str("receipt row exists but record decoding is not implemented")
            }
            Self::Storage { operation, message } => write!(formatter, "{operation}: {message}"),
        }
    }
}

impl std::error::Error for ReceiptLedgerError {}

/// Retained receipt namespace authority.
///
/// The ownership lock lives inside the replaceable `receipts/` directory, so
/// every operation validates the complete named receipts/active/generation
/// chain before and after reading. Future mutating operations must preserve
/// that two-sided validation and classify a lost post-write name binding as an
/// uncertain commit; they must never report a successful mutation through the
/// displaced descriptor.
pub(crate) struct ReceiptLedgerStore {
    receipts: RetainedDirectoryCapability,
    receipts_file: File,
    active: RetainedDirectoryCapability,
    active_file: File,
    generation: RetainedRegularFileCapability,
    generation_file: Mutex<File>,
    _ownership_lock: File,
}

impl ReceiptLedgerStore {
    pub(crate) fn open(receipts_path: impl AsRef<Path>) -> Result<Self, ReceiptLedgerError> {
        let receipts_path = receipts_path.as_ref();
        let receipts_file = open_or_create_owner_only_directory(receipts_path)?;
        let receipts = RetainedDirectoryCapability::open(receipts_path)
            .map_err(|error| storage_error("retain named receipts directory", error))?;
        if file_identity(&receipts_file)
            .map_err(|error| storage_error("identify receipts directory", error))?
            != receipts.identity()
        {
            return Err(ReceiptLedgerError::Corrupt(
                "receipts directory changed while retaining its named identity",
            ));
        }
        Self::open_retained_directory_with_file(receipts, receipts_file)
    }

    pub(crate) fn open_retained_directory(
        receipts: RetainedDirectoryCapability,
    ) -> Result<Self, ReceiptLedgerError> {
        let receipts_file = receipts
            .try_clone_directory()
            .map_err(|error| storage_error("clone retained receipts directory", error))?;
        Self::open_retained_directory_with_file(receipts, receipts_file)
    }

    fn open_retained_directory_with_file(
        receipts: RetainedDirectoryCapability,
        receipts_file: File,
    ) -> Result<Self, ReceiptLedgerError> {
        receipts
            .validate_named_identity()
            .map_err(|error| storage_error("validate named receipts directory", error))?;
        verify_owner_only_acl(&receipts_file)
            .map_err(|error| storage_error("verify receipts directory ownership", error))?;
        let ownership_lock =
            open_directory_ownership_lock(&receipts_file, OsStr::new(LEDGER_LOCK_FILE_NAME))
                .map_err(|error| storage_error("open receipt ledger ownership object", error))?;
        verify_owner_only_acl(&ownership_lock)
            .map_err(|error| storage_error("verify receipt ledger ownership object", error))?;
        match FileExt::try_lock_exclusive(&ownership_lock) {
            Ok(()) => {}
            Err(error) if lock_is_contended(&error) => {
                return Err(ReceiptLedgerError::AlreadyOwned)
            }
            Err(error) => {
                return Err(storage_error(
                    "acquire receipt ledger ownership lock",
                    error,
                ))
            }
        }

        let generation_file = open_or_initialize_generation(&receipts_file)?;
        let generation = receipts
            .retain_regular_child(OsStr::new(GENERATION_FILE_NAME))
            .map_err(|error| storage_error("retain named generation record", error))?;
        if file_identity(&generation_file)
            .map_err(|error| storage_error("identify generation record", error))?
            != generation.identity()
        {
            return Err(ReceiptLedgerError::Corrupt(
                "generation record changed while retaining its named identity",
            ));
        }
        let active_file = open_or_create_owner_only_child(&receipts_file, ACTIVE_DIRECTORY_NAME)?;
        let active = receipts
            .retain_directory_child(OsStr::new(ACTIVE_DIRECTORY_NAME))
            .map_err(|error| storage_error("retain named receipt active directory", error))?;
        if file_identity(&active_file)
            .map_err(|error| storage_error("identify receipt active directory", error))?
            != active.identity()
        {
            return Err(ReceiptLedgerError::Corrupt(
                "receipt active directory changed while retaining its named identity",
            ));
        }
        let store = Self {
            receipts,
            receipts_file,
            active,
            active_file,
            generation,
            generation_file: Mutex::new(generation_file),
            _ownership_lock: ownership_lock,
        };
        store.verify_named_authority()?;
        store.generation()?;
        Ok(store)
    }

    pub(crate) fn generation(&self) -> Result<u64, ReceiptLedgerError> {
        self.verify_named_authority()?;
        let mut generation_file = self
            .generation_file
            .lock()
            .map_err(|_| ReceiptLedgerError::Corrupt("generation reader lock was poisoned"))?;
        verify_owner_only_acl(&generation_file)
            .map_err(|error| storage_error("verify generation record ownership", error))?;
        generation_file
            .seek(SeekFrom::Start(0))
            .map_err(|error| storage_error("rewind generation record", error))?;
        let mut bytes = Vec::new();
        Read::by_ref(&mut *generation_file)
            .take((MAX_GENERATION_FILE_BYTES + 1) as u64)
            .read_to_end(&mut bytes)
            .map_err(|error| storage_error("read generation record", error))?;
        let generation = parse_generation(&bytes)?;
        drop(generation_file);
        self.verify_named_authority()?;
        Ok(generation)
    }

    pub(crate) fn observe_stable_generation(
        &self,
    ) -> Result<StableReceiptLedgerObservation, ReceiptLedgerError> {
        self.verify_named_authority()?;
        let generation_before = self.generation()?;
        let generation_after = self.generation()?;
        if generation_after != generation_before {
            return Err(ReceiptLedgerError::ConcurrentGenerationChange {
                generation_before,
                generation_after,
            });
        }
        self.verify_named_authority()?;
        Ok(StableReceiptLedgerObservation {
            generation_before,
            generation_after,
        })
    }

    pub(crate) fn inspect_exact(
        &self,
        receipt_key_digest: &ReceiptKeyDigest,
    ) -> Result<MissingReceiptObservation, ReceiptLedgerError> {
        self.verify_named_authority()?;
        let generation_before = self.generation()?;
        let record_name = format!("{}.json", receipt_key_digest.as_str());
        match open_regular_child_nofollow(&self.active_file, OsStr::new(&record_name)) {
            Ok(record) => {
                verify_owner_only_acl(&record)
                    .map_err(|error| storage_error("verify receipt row ownership", error))?;
                return Err(ReceiptLedgerError::ReceiptRowPresentUnsupported);
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(storage_error("inspect exact receipt row", error)),
        }
        let generation_after = self.generation()?;
        if generation_after != generation_before {
            return Err(ReceiptLedgerError::ConcurrentGenerationChange {
                generation_before,
                generation_after,
            });
        }
        self.verify_named_authority()?;
        Ok(MissingReceiptObservation {
            receipt_key_digest: receipt_key_digest.clone(),
            generation_before,
            generation_after,
        })
    }

    fn verify_named_authority(&self) -> Result<(), ReceiptLedgerError> {
        self.receipts
            .validate_named_identity()
            .map_err(|error| storage_error("validate named receipts directory", error))?;
        self.active
            .validate_named_identity()
            .map_err(|error| storage_error("validate named receipt active directory", error))?;
        self.generation
            .validate_named_identity()
            .map_err(|error| storage_error("validate named generation record", error))?;
        verify_owner_only_acl(&self.receipts_file)
            .map_err(|error| storage_error("verify receipts directory ownership", error))?;
        verify_owner_only_acl(&self.active_file)
            .map_err(|error| storage_error("verify receipt active directory ownership", error))?;
        let generation_file = self
            .generation_file
            .lock()
            .map_err(|_| ReceiptLedgerError::Corrupt("generation reader lock was poisoned"))?;
        verify_owner_only_acl(&generation_file)
            .map_err(|error| storage_error("verify generation record ownership", error))
    }
}

fn open_or_create_owner_only_directory(path: &Path) -> Result<File, ReceiptLedgerError> {
    if !path.is_absolute() {
        return Err(ReceiptLedgerError::Storage {
            operation: "open receipts directory",
            message: "receipt ledger path must be absolute".to_string(),
        });
    }
    let parent_path = path.parent().ok_or_else(|| ReceiptLedgerError::Storage {
        operation: "open receipts directory",
        message: "receipt ledger path has no parent".to_string(),
    })?;
    let name = path
        .file_name()
        .ok_or_else(|| ReceiptLedgerError::Storage {
            operation: "open receipts directory",
            message: "receipt ledger path has no final component".to_string(),
        })?;
    if !matches!(path.components().next_back(), Some(Component::Normal(_))) {
        return Err(ReceiptLedgerError::Storage {
            operation: "open receipts directory",
            message: "receipt ledger path must end in one normal component".to_string(),
        });
    }
    let parent = open_absolute_directory_path_nofollow(parent_path)
        .map_err(|error| storage_error("open receipt ledger parent", error))?;
    let directory = match open_directory_child_nofollow(&parent, name) {
        Ok(directory) => directory,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            match create_owner_only_directory_child(&parent, name) {
                Ok(directory) => {
                    sync_directory(&parent).map_err(|error| {
                        storage_error("sync receipt ledger directory creation", error)
                    })?;
                    directory
                }
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                    open_directory_child_nofollow(&parent, name)
                        .map_err(|error| storage_error("open raced receipts directory", error))?
                }
                Err(error) => {
                    return Err(storage_error("create owner-only receipts directory", error))
                }
            }
        }
        Err(error) => return Err(storage_error("open receipts directory no-follow", error)),
    };
    verify_owner_only_acl(&directory)
        .map_err(|error| storage_error("verify receipts directory ownership", error))?;
    Ok(directory)
}

fn open_or_create_owner_only_child(
    parent: &File,
    name: &'static str,
) -> Result<File, ReceiptLedgerError> {
    let name = OsStr::new(name);
    let directory = match open_directory_child_nofollow(parent, name) {
        Ok(directory) => directory,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            match create_owner_only_directory_child(parent, name) {
                Ok(directory) => {
                    sync_directory(parent)
                        .map_err(|error| storage_error("sync receipt subdirectory", error))?;
                    directory
                }
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                    open_directory_child_nofollow(parent, name)
                        .map_err(|error| storage_error("open raced receipt subdirectory", error))?
                }
                Err(error) => {
                    return Err(storage_error(
                        "create owner-only receipt subdirectory",
                        error,
                    ))
                }
            }
        }
        Err(error) => return Err(storage_error("open receipt subdirectory no-follow", error)),
    };
    verify_owner_only_acl(&directory)
        .map_err(|error| storage_error("verify receipt subdirectory ownership", error))?;
    Ok(directory)
}

fn open_or_initialize_generation(receipts: &File) -> Result<File, ReceiptLedgerError> {
    let name = OsStr::new(GENERATION_FILE_NAME);
    let generation = match open_regular_child_nofollow(receipts, name) {
        Ok(generation) => generation,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            let mut generation = create_owner_only_file_child(receipts, name)
                .map_err(|error| storage_error("create generation record", error))?;
            generation
                .write_all(b"0\n")
                .and_then(|()| generation.sync_all())
                .map_err(|error| storage_error("persist initial generation", error))?;
            sync_directory(receipts)
                .map_err(|error| storage_error("sync initial generation", error))?;
            generation
        }
        Err(error) => return Err(storage_error("open generation record no-follow", error)),
    };
    verify_owner_only_acl(&generation)
        .map_err(|error| storage_error("verify generation record ownership", error))?;
    Ok(generation)
}

fn parse_generation(bytes: &[u8]) -> Result<u64, ReceiptLedgerError> {
    if bytes.is_empty() || bytes.len() > MAX_GENERATION_FILE_BYTES || !bytes.ends_with(b"\n") {
        return Err(ReceiptLedgerError::Corrupt(
            "generation record is not one bounded newline-terminated decimal",
        ));
    }
    let number = &bytes[..bytes.len() - 1];
    let text = std::str::from_utf8(number)
        .map_err(|_| ReceiptLedgerError::Corrupt("generation record is not UTF-8"))?;
    if text.is_empty()
        || !text.bytes().all(|byte| byte.is_ascii_digit())
        || (text.len() > 1 && text.starts_with('0'))
    {
        return Err(ReceiptLedgerError::Corrupt(
            "generation record is not canonical unsigned decimal",
        ));
    }
    text.parse()
        .map_err(|_| ReceiptLedgerError::Corrupt("generation record exceeds u64"))
}

fn storage_error(operation: &'static str, error: io::Error) -> ReceiptLedgerError {
    ReceiptLedgerError::Storage {
        operation,
        message: error.to_string(),
    }
}

fn lock_is_contended(error: &io::Error) -> bool {
    let expected = fs2::lock_contended_error();
    error.kind() == io::ErrorKind::WouldBlock
        || error
            .raw_os_error()
            .zip(expected.raw_os_error())
            .is_some_and(|(actual, expected)| actual == expected)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infrastructure::platform::filesystem::{
        open_directory_nofollow, open_regular_child_nofollow, verify_owner_only_acl,
    };
    use crate::infrastructure::platform::testing::{
        attempt_retained_directory_replacement_for_test, create_directory_link_fixture_for_test,
        FileLinkFixtureOutcome, RetainedDirectoryReplacementOutcome,
    };
    use std::ffi::OsStr;
    use std::fs;
    use std::io::Read;
    use std::str::FromStr;

    fn digest(byte: char) -> ReceiptKeyDigest {
        ReceiptKeyDigest::from_str(&byte.to_string().repeat(64)).expect("checked digest")
    }

    fn directory_names(path: &Path) -> Vec<String> {
        let mut names = fs::read_dir(path)
            .expect("read receipt directory")
            .map(|entry| {
                entry
                    .expect("read receipt entry")
                    .file_name()
                    .into_string()
                    .expect("receipt entry name is UTF-8")
            })
            .collect::<Vec<_>>();
        names.sort();
        names
    }

    #[test]
    fn open_persists_and_reopens_generation_zero_in_owner_only_receipts() {
        let root = tempfile::tempdir().expect("temporary root");
        let receipts = fs::canonicalize(root.path())
            .expect("physical temporary root")
            .join("receipts");

        {
            let store = ReceiptLedgerStore::open(&receipts).expect("open receipt ledger");
            assert_eq!(store.generation().expect("read generation"), 0);
        }

        let store = ReceiptLedgerStore::open(&receipts).expect("reopen receipt ledger");
        assert_eq!(store.generation().expect("reread generation"), 0);

        let receipts_handle = open_directory_nofollow(&receipts).expect("retained receipts");
        verify_owner_only_acl(&receipts_handle).expect("owner-only receipts");
        let generation = open_regular_child_nofollow(&receipts_handle, OsStr::new("generation"))
            .expect("generation record");
        verify_owner_only_acl(&generation).expect("owner-only generation");
        let active = crate::infrastructure::platform::filesystem::open_directory_child_nofollow(
            &receipts_handle,
            OsStr::new("active"),
        )
        .expect("retained active directory");
        verify_owner_only_acl(&active).expect("owner-only active directory");
    }

    #[test]
    fn exact_missing_observation_is_key_bound_and_store_minted() {
        let root = tempfile::tempdir().expect("temporary root");
        let receipts = fs::canonicalize(root.path())
            .expect("physical temporary root")
            .join("receipts");
        let store = ReceiptLedgerStore::open(&receipts).expect("open receipt ledger");
        let expected = digest('c');

        let observation = store
            .inspect_exact(&expected)
            .expect("inspect exact missing receipt");

        assert_eq!(observation.receipt_key_digest(), &expected);
        assert_eq!(observation.generation_before(), 0);
        assert_eq!(observation.generation_after(), 0);
    }

    #[test]
    fn stable_generation_observation_is_store_minted_after_two_sided_validation() {
        let root = tempfile::tempdir().expect("temporary root");
        let receipts = fs::canonicalize(root.path())
            .expect("physical temporary root")
            .join("receipts");
        let store = ReceiptLedgerStore::open(&receipts).expect("open receipt ledger");

        let observation = store
            .observe_stable_generation()
            .expect("observe a stable validated generation");

        assert_eq!(observation.generation_before(), 0);
        assert_eq!(observation.generation_after(), 0);
    }

    #[test]
    fn retained_named_capability_opens_without_reconstructing_the_receipts_path() {
        let root = tempfile::tempdir().expect("temporary root");
        let root = fs::canonicalize(root.path()).expect("physical temporary root");
        let receipts = root.join("receipts");
        {
            let store = ReceiptLedgerStore::open(&receipts).expect("initialize receipt ledger");
            assert_eq!(store.generation().expect("initial generation"), 0);
        }
        let parent = RetainedDirectoryCapability::open(&root).expect("retain state directory");
        let receipts = parent
            .retain_directory_child(OsStr::new("receipts"))
            .expect("retain named receipts child");

        let store = ReceiptLedgerStore::open_retained_directory(receipts)
            .expect("open from retained named capability");

        assert_eq!(store.generation().expect("retained generation"), 0);
    }

    #[test]
    fn exact_missing_receipt_keeps_the_same_persisted_generation_without_mutation() {
        let root = tempfile::tempdir().expect("temporary root");
        let receipts = fs::canonicalize(root.path())
            .expect("physical temporary root")
            .join("receipts");
        let store = ReceiptLedgerStore::open(&receipts).expect("open receipt ledger");
        let names_before = directory_names(&receipts);
        let mut generation_before = Vec::new();
        fs::File::open(receipts.join("generation"))
            .expect("generation record")
            .read_to_end(&mut generation_before)
            .expect("read generation bytes");

        let expected_digest = digest('a');
        let observation = store
            .inspect_exact(&expected_digest)
            .expect("inspect exact missing receipt");
        assert_eq!(observation.receipt_key_digest(), &expected_digest);
        assert_eq!(observation.generation_before(), 0);
        assert_eq!(observation.generation_after(), 0);

        assert_eq!(directory_names(&receipts), names_before);
        assert_eq!(
            fs::read(receipts.join("generation")).expect("reread generation bytes"),
            generation_before
        );
    }

    #[test]
    fn receipt_directory_link_or_reparse_point_is_rejected_without_touching_target() {
        let root = tempfile::tempdir().expect("temporary root");
        let outside = tempfile::tempdir().expect("outside directory");
        let receipts = fs::canonicalize(root.path())
            .expect("physical temporary root")
            .join("receipts");
        let outside = fs::canonicalize(outside.path()).expect("physical outside directory");
        match create_directory_link_fixture_for_test(&outside, &receipts)
            .expect("create directory-link fixture")
        {
            FileLinkFixtureOutcome::Created => {}
            FileLinkFixtureOutcome::Unsupported
            | FileLinkFixtureOutcome::WindowsPrivilegeUnavailable => return,
        }

        assert!(ReceiptLedgerStore::open(&receipts).is_err());
        assert!(directory_names(&outside).is_empty());
    }

    #[test]
    fn named_receipts_replacement_never_leaves_two_usable_owners() {
        let root = tempfile::tempdir().expect("temporary root");
        let root = fs::canonicalize(root.path()).expect("physical temporary root");
        let receipts = root.join("receipts");
        let displaced = root.join("receipts-displaced");
        let first = ReceiptLedgerStore::open(&receipts).expect("open first receipt owner");

        match attempt_retained_directory_replacement_for_test(&receipts, &displaced)
            .expect("attempt named receipt replacement")
        {
            RetainedDirectoryReplacementOutcome::Replaced => {
                let second = ReceiptLedgerStore::open(&receipts)
                    .expect("replacement may become the named receipt owner");
                assert_eq!(second.generation().expect("replacement generation"), 0);
                assert!(
                    first.generation().is_err(),
                    "the displaced owner remained usable beside the replacement owner"
                );
            }
            RetainedDirectoryReplacementOutcome::PreventedByRetainedHandle => {
                assert_eq!(first.generation().expect("retained generation"), 0);
                assert!(matches!(
                    ReceiptLedgerStore::open(&receipts),
                    Err(ReceiptLedgerError::AlreadyOwned)
                ));
            }
        }
    }

    #[test]
    fn named_active_replacement_invalidates_the_retained_owner() {
        let active_root = tempfile::tempdir().expect("active temporary root");
        let active_root =
            fs::canonicalize(active_root.path()).expect("physical active temporary root");
        let active_receipts = active_root.join("receipts");
        let active = active_receipts.join("active");
        let displaced_active = active_receipts.join("active-displaced");
        let active_store =
            ReceiptLedgerStore::open(&active_receipts).expect("open active receipt owner");

        match attempt_retained_directory_replacement_for_test(&active, &displaced_active)
            .expect("attempt named active replacement")
        {
            RetainedDirectoryReplacementOutcome::Replaced => {
                fs::create_dir(&active).expect("create replacement active directory");
                assert!(
                    active_store.inspect_exact(&digest('d')).is_err(),
                    "the owner accepted a replacement active directory"
                );
            }
            RetainedDirectoryReplacementOutcome::PreventedByRetainedHandle => {
                let expected_digest = digest('d');
                let observation = active_store
                    .inspect_exact(&expected_digest)
                    .expect("retained active inspection");
                assert_eq!(observation.receipt_key_digest(), &expected_digest);
                assert_eq!(observation.generation_before(), 0);
                assert_eq!(observation.generation_after(), 0);
            }
        }
    }

    #[test]
    fn named_generation_replacement_invalidates_the_retained_owner() {
        let generation_root = tempfile::tempdir().expect("generation temporary root");
        let generation_root =
            fs::canonicalize(generation_root.path()).expect("physical generation temporary root");
        let generation_receipts = generation_root.join("receipts");
        let generation = generation_receipts.join("generation");
        let displaced_generation = generation_receipts.join("generation-displaced");
        let generation_store =
            ReceiptLedgerStore::open(&generation_receipts).expect("open generation receipt owner");

        match fs::rename(&generation, &displaced_generation) {
            Ok(()) => {
                fs::write(&generation, b"0\n").expect("write replacement generation");
                assert!(
                    generation_store.generation().is_err(),
                    "the owner accepted a replacement generation record"
                );
            }
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::PermissionDenied | io::ErrorKind::WouldBlock
                ) =>
            {
                assert_eq!(
                    generation_store
                        .generation()
                        .expect("retained generation after prevented replacement"),
                    0
                );
                assert!(!displaced_generation.exists());
            }
            Err(error) => panic!("attempt named generation replacement: {error}"),
        }
    }
}
