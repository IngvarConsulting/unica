use crate::application::discovery::ports::SourceInventoryPort;
use crate::domain::discovery::{
    DiscoveryQuery, ProviderCoverage, ProviderDiagnostic, ProviderOutcome, SourceFile,
    SourceInventory,
};
use crate::domain::project_sources::{config_dump_info_xml_kind, ConfigDumpInfoXmlKind};
use crate::infrastructure::platform::contained_file::{
    read_contained_regular_file, read_contained_regular_file_with_expected_identity,
    ContainedFileError,
};
use crate::infrastructure::platform::verified_directory::{
    read_verified_contained_directory_cancellable,
    read_verified_contained_directory_with_expected_identity_cancellable,
    VerifiedDirectoryEntryKind, VerifiedDirectoryError,
};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

pub(crate) struct ContainedSourceInventoryPort {
    canonical_root: PathBuf,
}

impl ContainedSourceInventoryPort {
    pub(crate) fn new(canonical_root: PathBuf) -> Self {
        Self { canonical_root }
    }

    fn capture(&self, query: &DiscoveryQuery<'_>) -> Result<Capture, CaptureError> {
        self.capture_observing(query, || {})
    }

    fn capture_observing(
        &self,
        query: &DiscoveryQuery<'_>,
        mut observe_visit: impl FnMut(),
    ) -> Result<Capture, CaptureError> {
        check_inventory_cancellation(query)?;
        let max_files = usize::try_from(query.limits().max_files).map_err(|_| {
            ProviderDiagnostic::material(
                "inventory_file_limit_overflow",
                "source inventory maxFiles is not representable on this host",
            )
        })?;
        let mut pending = BTreeMap::from([(
            self.canonical_root.clone(),
            (VerifiedDirectoryEntryKind::Directory, None),
        )]);
        let mut files = Vec::new();
        let mut files_seen = 0_u32;
        let mut bytes_analyzed = 0_u64;
        let mut bytes_read_budget = 0_u64;
        while let Some((path, (kind, expected_identity))) = pending.pop_first() {
            observe_visit();
            check_inventory_cancellation(query)?;
            match kind {
                VerifiedDirectoryEntryKind::Directory => {
                    let entries = match expected_identity {
                        Some(expected_identity) => {
                            read_verified_contained_directory_with_expected_identity_cancellable(
                                &self.canonical_root,
                                &path,
                                expected_identity,
                                || query.is_cancelled(),
                            )
                        }
                        None => read_verified_contained_directory_cancellable(
                            &self.canonical_root,
                            &path,
                            || query.is_cancelled(),
                        ),
                    }
                    .map_err(classify_verified_directory_error)?;
                    for entry in entries {
                        check_inventory_cancellation(query)?;
                        pending.insert(entry.path, (entry.kind, Some(entry.identity)));
                    }
                    continue;
                }
                VerifiedDirectoryEntryKind::RegularFile => {}
            }
            if !is_evidence_candidate(&path) {
                continue;
            }
            let needs_sidecar_classification = has_config_dump_info_filename(&path);
            if !needs_sidecar_classification {
                files_seen = checked_increment_files_seen(files_seen)?;
                if files.len() >= max_files {
                    return Ok(Capture::Bounded {
                        inventory: completed_inventory(files, files_seen, bytes_analyzed)?,
                        diagnostic: ProviderDiagnostic::material(
                            "source_inventory_file_bound",
                            "source inventory stopped at the maxFiles limit",
                        ),
                    });
                }
            }
            let remaining_bytes = query
                .limits()
                .max_bytes
                .checked_sub(bytes_read_budget)
                .ok_or_else(|| {
                    ProviderDiagnostic::material(
                        "inventory_byte_count_overflow",
                        "source inventory byte accounting exceeded maxBytes",
                    )
                })?;
            let verified_result = match expected_identity {
                Some(expected_identity) => read_contained_regular_file_with_expected_identity(
                    &self.canonical_root,
                    &path,
                    remaining_bytes,
                    expected_identity,
                ),
                None => read_contained_regular_file(&self.canonical_root, &path, remaining_bytes),
            };
            check_inventory_cancellation(query)?;
            let verified = match verified_result {
                Ok(verified) => verified,
                Err(ContainedFileError::SizeLimitExceeded { limit: _ }) => {
                    if needs_sidecar_classification {
                        files_seen = checked_increment_files_seen(files_seen)?;
                    }
                    return Ok(Capture::Bounded {
                        inventory: completed_inventory(files, files_seen, bytes_analyzed)?,
                        diagnostic: ProviderDiagnostic::material(
                            "source_inventory_byte_bound",
                            "source inventory stopped at the maxBytes limit",
                        ),
                    });
                }
                Err(error) => return Err(classify_contained_file_error(error)),
            };
            bytes_read_budget = bytes_read_budget
                .checked_add(verified.bytes_read)
                .ok_or_else(|| {
                    ProviderDiagnostic::material(
                        "inventory_byte_count_overflow",
                        "source inventory read budget overflowed",
                    )
                })?;
            if needs_sidecar_classification {
                let is_runtime_sidecar = match config_dump_info_xml_kind(&verified.bytes) {
                    ConfigDumpInfoXmlKind::RuntimeSidecar => true,
                    ConfigDumpInfoXmlKind::ExternalProcessor
                    | ConfigDumpInfoXmlKind::ExternalReport
                    | ConfigDumpInfoXmlKind::MetadataDescriptor
                    | ConfigDumpInfoXmlKind::Other => false,
                };
                if is_runtime_sidecar {
                    continue;
                }
            }
            if needs_sidecar_classification {
                files_seen = checked_increment_files_seen(files_seen)?;
                if files.len() >= max_files {
                    return Ok(Capture::Bounded {
                        inventory: completed_inventory(files, files_seen, bytes_analyzed)?,
                        diagnostic: ProviderDiagnostic::material(
                            "source_inventory_file_bound",
                            "source inventory stopped at the maxFiles limit",
                        ),
                    });
                }
            }
            bytes_analyzed = bytes_analyzed
                .checked_add(verified.bytes_read)
                .ok_or_else(|| {
                    ProviderDiagnostic::material(
                        "inventory_byte_count_overflow",
                        "source inventory byte count overflowed",
                    )
                })?;
            files.push(SourceFile {
                relative_path: verified.relative_path,
                bytes: verified.bytes.into(),
                raw_hash: verified.raw_sha256,
            });
        }
        Ok(Capture::Complete(completed_inventory(
            files,
            files_seen,
            bytes_analyzed,
        )?))
    }
}

fn check_inventory_cancellation(query: &DiscoveryQuery<'_>) -> Result<(), CaptureError> {
    crate::infrastructure::discovery::check_cancellation(query).map_err(CaptureError::Failed)
}

fn checked_increment_files_seen(files_seen: u32) -> Result<u32, ProviderDiagnostic> {
    files_seen.checked_add(1).ok_or_else(|| {
        ProviderDiagnostic::material(
            "inventory_file_count_overflow",
            "source inventory file count overflowed",
        )
    })
}

enum Capture {
    Complete(SourceInventory),
    Bounded {
        inventory: SourceInventory,
        diagnostic: ProviderDiagnostic,
    },
}

enum CaptureError {
    Unavailable(ProviderDiagnostic),
    Failed(ProviderDiagnostic),
    ContractViolation(ProviderDiagnostic),
}

impl From<ProviderDiagnostic> for CaptureError {
    fn from(diagnostic: ProviderDiagnostic) -> Self {
        Self::ContractViolation(diagnostic)
    }
}

fn completed_inventory(
    mut files: Vec<SourceFile>,
    files_seen: u32,
    bytes_analyzed: u64,
) -> Result<SourceInventory, ProviderDiagnostic> {
    files.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));
    let count = u32::try_from(files.len()).map_err(|_| {
        ProviderDiagnostic::material(
            "inventory_file_count_overflow",
            "source inventory file count overflowed",
        )
    })?;
    Ok(SourceInventory {
        files,
        coverage: ProviderCoverage::new(files_seen, count, bytes_analyzed, count),
    })
}

impl SourceInventoryPort for ContainedSourceInventoryPort {
    fn inventory(&self, query: &DiscoveryQuery<'_>) -> ProviderOutcome<SourceInventory> {
        match self.capture(query) {
            Ok(Capture::Complete(inventory)) => ProviderOutcome::Complete(inventory),
            Ok(Capture::Bounded {
                inventory,
                diagnostic,
            }) => ProviderOutcome::Bounded {
                data: inventory,
                diagnostic,
            },
            Err(CaptureError::Unavailable(diagnostic)) => ProviderOutcome::Unavailable(diagnostic),
            Err(CaptureError::Failed(diagnostic)) => ProviderOutcome::Failed(diagnostic),
            Err(CaptureError::ContractViolation(diagnostic)) => {
                ProviderOutcome::ContractViolation(diagnostic)
            }
        }
    }
}

fn is_evidence_candidate(path: &Path) -> bool {
    let extension_is_evidence = path
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            extension.eq_ignore_ascii_case("xml") || extension.eq_ignore_ascii_case("bsl")
        });
    extension_is_evidence || is_parent_configurations(path)
}

fn has_config_dump_info_filename(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.eq_ignore_ascii_case("ConfigDumpInfo.xml"))
}

fn is_parent_configurations(path: &Path) -> bool {
    let mut components = path.components().rev();
    let Some(file_name) = components.next() else {
        return false;
    };
    let Some(parent) = components.next() else {
        return false;
    };
    file_name
        .as_os_str()
        .to_str()
        .is_some_and(|name| name.eq_ignore_ascii_case("ParentConfigurations.bin"))
        && parent
            .as_os_str()
            .to_str()
            .is_some_and(|name| name.eq_ignore_ascii_case("Ext"))
}

fn classify_inventory_io(operation: &'static str, error: std::io::Error) -> CaptureError {
    let diagnostic = ProviderDiagnostic::material(
        operation,
        format!("source inventory I/O failed during {operation}: {error}"),
    );
    CaptureError::Failed(diagnostic)
}

fn classify_contained_file_error(error: ContainedFileError) -> CaptureError {
    match error {
        ContainedFileError::UnsupportedHost => {
            CaptureError::Unavailable(ProviderDiagnostic::material(
                "source_inventory_unsupported_host",
                "verified source reads are unavailable on this host",
            ))
        }
        ContainedFileError::Io { operation, source } => classify_inventory_io(operation, source),
        error @ (ContainedFileError::RootNotCanonical
        | ContainedFileError::RootNotDirectory
        | ContainedFileError::PathOutsideRoot
        | ContainedFileError::FinalPathOutsideRoot
        | ContainedFileError::FinalPathMismatch
        | ContainedFileError::AmbiguousHostPath
        | ContainedFileError::InvalidRelativePath(_)
        | ContainedFileError::SymlinkOrReparsePoint
        | ContainedFileError::NotRegularFile
        | ContainedFileError::IdentityChanged
        | ContainedFileError::SizeLimitExceeded { .. }
        | ContainedFileError::LengthOverflow) => {
            CaptureError::ContractViolation(ProviderDiagnostic::material(
                "inventory_verified_read",
                format!("verified source read failed: {error}"),
            ))
        }
    }
}

fn classify_verified_directory_error(error: VerifiedDirectoryError) -> CaptureError {
    match error {
        VerifiedDirectoryError::Cancelled => {
            CaptureError::Failed(crate::infrastructure::discovery::cancellation_diagnostic())
        }
        VerifiedDirectoryError::UnsupportedHost => {
            CaptureError::Unavailable(ProviderDiagnostic::material(
                "source_inventory_unsupported_host",
                "verified directory enumeration is unavailable on this host",
            ))
        }
        VerifiedDirectoryError::Io { operation, source } => {
            classify_inventory_io(operation, source)
        }
        error @ (VerifiedDirectoryError::SymlinkOrReparsePoint
        | VerifiedDirectoryError::NonRegularEntry
        | VerifiedDirectoryError::NotDirectory) => {
            CaptureError::ContractViolation(ProviderDiagnostic::material(
                "source_inventory_unsafe_entry",
                format!("source inventory contains an unsafe filesystem entry: {error}"),
            ))
        }
        error @ (VerifiedDirectoryError::RootNotCanonical
        | VerifiedDirectoryError::RootNotDirectory
        | VerifiedDirectoryError::PathOutsideRoot
        | VerifiedDirectoryError::FinalPathOutsideRoot
        | VerifiedDirectoryError::FinalPathMismatch
        | VerifiedDirectoryError::AmbiguousHostPath
        | VerifiedDirectoryError::InvalidRelativePath(_)
        | VerifiedDirectoryError::IdentityChanged
        | VerifiedDirectoryError::EntryLimitExceeded { .. }
        | VerifiedDirectoryError::LengthOverflow) => {
            CaptureError::ContractViolation(ProviderDiagnostic::material(
                "inventory_verified_directory",
                format!("verified source directory failed: {error}"),
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        classify_contained_file_error, classify_inventory_io, classify_verified_directory_error,
        CaptureError, ContainedSourceInventoryPort,
    };
    use crate::application::discovery::ports::SourceInventoryPort;
    use crate::domain::discovery::{
        DiscoveryQuery, DiscoveryQueryLimits, PortableRelativePath, ProviderCoverage,
        ProviderOutcome,
    };
    use crate::infrastructure::platform::contained_file::{
        create_non_regular_fixture_for_test, ContainedFileError, NonRegularFixtureOutcome,
    };
    use crate::infrastructure::platform::testing::{
        create_dir_symlink_for_test, create_file_link_fixture_for_test, FileLinkFixtureOutcome,
    };
    use crate::infrastructure::platform::verified_directory::VerifiedDirectoryError;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(1);

    #[test]
    fn returns_eligible_files_in_deterministic_portable_path_order() {
        let root = fixture_root("deterministic");
        write(&root.join("z/Module.bsl"), b"z");
        write(&root.join("a/Document.xml"), b"xml");
        write(&root.join("ignored/readme.txt"), b"ignored");
        let provider = ContainedSourceInventoryPort::new(root.clone());

        let outcome = provider.inventory(&query(10, 1_024));

        let ProviderOutcome::Complete(inventory) = outcome else {
            panic!("expected complete inventory");
        };
        assert_eq!(
            inventory
                .files
                .iter()
                .map(|file| file.relative_path.clone())
                .collect::<Vec<_>>(),
            vec![
                PortableRelativePath::parse_str("a/Document.xml").expect("portable XML path"),
                PortableRelativePath::parse_str("z/Module.bsl").expect("portable BSL path"),
            ]
        );
        assert_eq!(inventory.files[0].bytes.as_ref(), b"xml");
        assert_eq!(inventory.files[1].bytes.as_ref(), b"z");
        assert_eq!(inventory.coverage, ProviderCoverage::new(2, 2, 4, 2));
        cleanup(&root);
    }

    #[test]
    fn file_bound_preserves_only_the_verified_deterministic_prefix() {
        let root = fixture_root("file-bound");
        write(&root.join("z.bsl"), b"z");
        write(&root.join("a.xml"), b"a");
        let provider = ContainedSourceInventoryPort::new(root.clone());

        let outcome = provider.inventory(&query(1, 1_024));

        let ProviderOutcome::Bounded { data, diagnostic } = outcome else {
            panic!("expected bounded inventory");
        };
        assert_eq!(data.files.len(), 1);
        assert_eq!(
            data.files[0].relative_path,
            PortableRelativePath::parse_str("a.xml").expect("portable path")
        );
        assert_eq!(data.coverage, ProviderCoverage::new(2, 1, 1, 1));
        assert_eq!(diagnostic.code, "source_inventory_file_bound");
        cleanup(&root);
    }

    #[test]
    fn aggregate_byte_bound_preserves_only_fully_verified_records() {
        let root = fixture_root("byte-bound");
        write(&root.join("a.xml"), b"123");
        write(&root.join("z.bsl"), b"456");
        let provider = ContainedSourceInventoryPort::new(root.clone());

        let outcome = provider.inventory(&query(10, 4));

        let ProviderOutcome::Bounded { data, diagnostic } = outcome else {
            panic!("expected bounded inventory");
        };
        assert_eq!(data.files.len(), 1);
        assert_eq!(data.files[0].bytes.as_ref(), b"123");
        assert_eq!(data.coverage, ProviderCoverage::new(2, 1, 3, 1));
        assert_eq!(diagnostic.code, "source_inventory_byte_bound");
        cleanup(&root);
    }

    #[test]
    fn byte_bound_during_sidecar_classification_counts_the_n_plus_one_observation() {
        let root = fixture_root("sidecar-byte-bound-after-file-bound");
        write(&root.join("a.xml"), b"a");
        write(
            &root.join("z/ConfigDumpInfo.xml"),
            b"<MetaDataObject><ExternalDataProcessor/></MetaDataObject>",
        );
        let provider = ContainedSourceInventoryPort::new(root.clone());

        let outcome = provider.inventory(&query(1, 1));

        let ProviderOutcome::Bounded { data, diagnostic } = outcome else {
            panic!("expected bounded inventory");
        };
        assert_eq!(data.files.len(), 1);
        assert_eq!(data.files[0].bytes.as_ref(), b"a");
        assert_eq!(data.coverage, ProviderCoverage::new(2, 1, 1, 1));
        assert_eq!(diagnostic.code, "source_inventory_byte_bound");
        cleanup(&root);
    }

    #[test]
    fn cancellation_during_verified_enumeration_stops_inventory_capture() {
        let root = fixture_root("cancelled-during-enumeration");
        write(&root.join("a/first.xml"), b"first");
        write(&root.join("z/second.xml"), b"second");
        let provider = ContainedSourceInventoryPort::new(root.clone());
        let cancellation = crate::domain::cancellation::CancellationToken::new();
        let query = query(10, 1_024).with_cancellation(&cancellation);
        let mut visits = 0_u8;

        let outcome = provider.capture_observing(&query, || {
            visits += 1;
            if visits == 2 {
                cancellation.cancel();
            }
        });

        let Err(CaptureError::Failed(diagnostic)) = outcome else {
            panic!("cancelled inventory must stop with a failed provider outcome");
        };
        assert_eq!(diagnostic.code, "discovery_cancelled");
        assert_eq!(diagnostic.message, "discovery cancelled");
        cleanup(&root);
    }

    #[test]
    fn excludes_config_dump_info_only_when_bytes_identify_a_runtime_sidecar() {
        let root = fixture_root("runtime-sidecar");
        write(
            &root.join("ConfigDumpInfo.xml"),
            b"<?xml version=\"1.0\"?><ConfigDumpInfo><ConfigVersions/></ConfigDumpInfo>",
        );
        let provider = ContainedSourceInventoryPort::new(root.clone());

        let outcome = provider.inventory(&query(10, 1_024));

        let ProviderOutcome::Complete(inventory) = outcome else {
            panic!("expected complete inventory");
        };
        assert!(inventory.files.is_empty());
        assert_eq!(inventory.coverage, ProviderCoverage::empty());
        cleanup(&root);
    }

    #[test]
    fn preserves_legitimate_external_metadata_named_config_dump_info() {
        let root = fixture_root("external-config-dump-info");
        let bytes = b"<MetaDataObject><ExternalDataProcessor/></MetaDataObject>";
        write(&root.join("ConfigDumpInfo.xml"), bytes);
        let provider = ContainedSourceInventoryPort::new(root.clone());

        let outcome = provider.inventory(&query(10, 1_024));

        let ProviderOutcome::Complete(inventory) = outcome else {
            panic!("expected complete inventory");
        };
        assert_eq!(inventory.files.len(), 1);
        assert_eq!(inventory.files[0].bytes.as_ref(), bytes);
        assert_eq!(
            inventory.coverage,
            ProviderCoverage::new(1, 1, bytes.len() as u64, 1)
        );
        cleanup(&root);
    }

    #[test]
    fn a_symlink_or_reparse_file_invalidates_the_whole_inventory() {
        let root = fixture_root("file-link");
        write(&root.join("a.xml"), b"verified-before-violation");
        let target = root.join("target.txt");
        let link = root.join("z.xml");
        write(&target, b"target");
        let outcome = create_file_link_fixture_for_test(&target, &link).expect("link fixture");
        if outcome != FileLinkFixtureOutcome::Created {
            cleanup(&root);
            return;
        }
        let provider = ContainedSourceInventoryPort::new(root.clone());

        let outcome = provider.inventory(&query(10, 1_024));

        let ProviderOutcome::ContractViolation(diagnostic) = outcome else {
            panic!("unsafe entry must invalidate all records");
        };
        assert_eq!(diagnostic.code, "source_inventory_unsafe_entry");
        cleanup(&root);
    }

    #[test]
    fn includes_ext_parent_configurations_binary_as_support_evidence() {
        let root = fixture_root("parent-configurations");
        write(
            &root.join("Documents/Order/Ext/ParentConfigurations.bin"),
            b"support-state",
        );
        let provider = ContainedSourceInventoryPort::new(root.clone());

        let outcome = provider.inventory(&query(10, 1_024));

        let ProviderOutcome::Complete(inventory) = outcome else {
            panic!("expected complete inventory");
        };
        assert_eq!(inventory.files.len(), 1);
        assert_eq!(
            inventory.files[0].relative_path,
            PortableRelativePath::parse_str("Documents/Order/Ext/ParentConfigurations.bin")
                .expect("portable support path")
        );
        assert_eq!(inventory.files[0].bytes.as_ref(), b"support-state");
        cleanup(&root);
    }

    #[test]
    fn ordinary_io_failures_are_failed() {
        let denied = classify_inventory_io(
            "inventory_read_directory",
            std::io::Error::new(std::io::ErrorKind::PermissionDenied, "denied"),
        );
        let disappeared = classify_inventory_io(
            "inventory_inspect_entry",
            std::io::Error::new(std::io::ErrorKind::NotFound, "replaced"),
        );

        assert!(matches!(denied, CaptureError::Failed(_)));
        assert!(matches!(disappeared, CaptureError::Failed(_)));
    }

    #[test]
    fn unsupported_hosts_are_unavailable_but_security_failures_are_contract_violations() {
        let CaptureError::Failed(cancelled) =
            classify_verified_directory_error(VerifiedDirectoryError::Cancelled)
        else {
            panic!("cancelled enumeration must be a failed provider outcome");
        };
        assert_eq!(cancelled.code, "discovery_cancelled");
        assert!(matches!(
            classify_contained_file_error(ContainedFileError::UnsupportedHost),
            CaptureError::Unavailable(_)
        ));
        assert!(matches!(
            classify_verified_directory_error(VerifiedDirectoryError::UnsupportedHost),
            CaptureError::Unavailable(_)
        ));
        assert!(matches!(
            classify_contained_file_error(ContainedFileError::IdentityChanged),
            CaptureError::ContractViolation(_)
        ));
        assert!(matches!(
            classify_verified_directory_error(VerifiedDirectoryError::IdentityChanged),
            CaptureError::ContractViolation(_)
        ));
        assert!(matches!(
            classify_verified_directory_error(VerifiedDirectoryError::Io {
                operation: "resolve opened directory path",
                source: std::io::Error::new(std::io::ErrorKind::NotFound, "gone"),
            }),
            CaptureError::Failed(_)
        ));
        assert!(matches!(
            classify_contained_file_error(ContainedFileError::Io {
                operation: "resolve opened file path",
                source: std::io::Error::new(std::io::ErrorKind::NotFound, "procfd unavailable"),
            }),
            CaptureError::Failed(_)
        ));
    }

    #[test]
    fn a_symlink_or_reparse_directory_invalidates_the_whole_inventory() {
        let root = fixture_root("directory-link");
        let outside = fixture_root("directory-link-outside");
        write(&outside.join("Document.xml"), b"outside");
        let link = root.join("linked");
        let Some(link_result) = create_dir_symlink_for_test(&outside, &link) else {
            cleanup(&root);
            cleanup(&outside);
            return;
        };
        if let Err(error) = link_result {
            if error.raw_os_error() == Some(1_314) {
                cleanup(&root);
                cleanup(&outside);
                return;
            }
            panic!("directory-link fixture failed: {error}");
        }
        let provider = ContainedSourceInventoryPort::new(root.clone());

        let outcome = provider.inventory(&query(10, 1_024));

        let ProviderOutcome::ContractViolation(diagnostic) = outcome else {
            panic!("unsafe directory must invalidate all records");
        };
        assert_eq!(diagnostic.code, "source_inventory_unsafe_entry");
        cleanup(&root);
        cleanup(&outside);
    }

    #[test]
    fn a_non_regular_entry_invalidates_the_whole_inventory() {
        let root = fixture_root("fifo");
        let fifo = root.join("stream.xml");
        if create_non_regular_fixture_for_test(&fifo).expect("non-regular fixture")
            == NonRegularFixtureOutcome::Unsupported
        {
            cleanup(&root);
            return;
        }
        let provider = ContainedSourceInventoryPort::new(root.clone());

        let outcome = provider.inventory(&query(10, 1_024));

        let ProviderOutcome::ContractViolation(diagnostic) = outcome else {
            panic!("non-regular entry must invalidate all records");
        };
        assert_eq!(diagnostic.code, "source_inventory_unsafe_entry");
        cleanup(&root);
    }

    fn query(max_files: u32, max_bytes: u64) -> DiscoveryQuery<'static> {
        DiscoveryQuery::new(
            "discover",
            &[],
            &[],
            &[],
            DiscoveryQueryLimits {
                max_files,
                max_bytes,
                max_evidence: 1,
                max_candidates: 1,
                max_graph_depth: 1,
            },
        )
    }

    fn fixture_root(label: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock after epoch")
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "unica-source-inventory-{label}-{}-{nanos}-{}",
            std::process::id(),
            TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&root).expect("fixture root");
        fs::canonicalize(root).expect("canonical fixture root")
    }

    fn write(path: &Path, bytes: &[u8]) {
        fs::create_dir_all(path.parent().expect("fixture parent")).expect("fixture directories");
        fs::write(path, bytes).expect("fixture file");
    }

    fn cleanup(root: &Path) {
        fs::remove_dir_all(root).expect("fixture cleanup");
    }
}
