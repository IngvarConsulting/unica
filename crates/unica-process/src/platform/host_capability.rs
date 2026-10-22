//! Visible skips for tests that need a host capability (#1893).
//!
//! A test that cannot run without, say, symlink creation used to `return`
//! early and was recorded as PASS. It now calls
//! [`skip_without_host_capability`] right before that `return`. The call
//! names the capability and the reason on stderr and, under the CI lane
//! runner, writes a record into `UNICA_HOST_CAPABILITY_SKIPS`. The runner
//! turns the passing case into a skip with that reason, and refuses it on a
//! runner where the capability is required (`scripts/ci/ci_rust_lanes.py`).
//!
//! OS-specific `cfg` stays in platform facades (INV.PLATFORM.OS-BEHIND-FACADE):
//! a test outside them asks a facade probe and skips with an OS capability
//! name here instead of carrying `cfg_attr(not(unix), ignore)`.

use std::path::Path;

/// Directory the CI lane runner gives tests for skip records.
pub const HOST_CAPABILITY_SKIPS_ENV: &str = "UNICA_HOST_CAPABILITY_SKIPS";

/// Capabilities whose absence a test may report instead of passing.
/// The names are the contract with the runner's per-runner table.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HostCapability {
    /// Creating file and directory symlinks (Windows needs a privilege).
    Symlink,
    /// A volume that tells `a` from `A`.
    CaseSensitiveFilesystem,
    /// A volume that treats `a` and `A` as one name.
    CaseInsensitiveFilesystem,
    /// A volume that treats composed and decomposed Unicode as one name.
    NormalizationInsensitiveFilesystem,
    /// Permission bits that bind the test process (not root).
    EnforcedPermissionBits,
    /// Setting a directory's modification time.
    DirectoryMtime,
    /// File names that are not valid UTF-8.
    NonUtf8FileNames,
    /// Renaming or replacing a path while a handle to it stays open (Unix).
    RenameUnderOpenHandle,
    /// Unix process groups, `waitid` and signals.
    UnixProcessGroups,
    /// Unix permission bits on files and directories.
    UnixPermissionBits,
    /// Windows Job Objects.
    WindowsJobObject,
    /// Windows verbatim, drive and UNC path syntax.
    WindowsPathSyntax,
}

impl HostCapability {
    pub const ALL: [HostCapability; 12] = [
        HostCapability::Symlink,
        HostCapability::CaseSensitiveFilesystem,
        HostCapability::CaseInsensitiveFilesystem,
        HostCapability::NormalizationInsensitiveFilesystem,
        HostCapability::EnforcedPermissionBits,
        HostCapability::DirectoryMtime,
        HostCapability::NonUtf8FileNames,
        HostCapability::RenameUnderOpenHandle,
        HostCapability::UnixProcessGroups,
        HostCapability::UnixPermissionBits,
        HostCapability::WindowsJobObject,
        HostCapability::WindowsPathSyntax,
    ];

    pub const fn name(self) -> &'static str {
        match self {
            HostCapability::Symlink => "symlink",
            HostCapability::CaseSensitiveFilesystem => "case-sensitive-filesystem",
            HostCapability::CaseInsensitiveFilesystem => "case-insensitive-filesystem",
            HostCapability::NormalizationInsensitiveFilesystem => {
                "normalization-insensitive-filesystem"
            }
            HostCapability::EnforcedPermissionBits => "enforced-permission-bits",
            HostCapability::DirectoryMtime => "directory-mtime",
            HostCapability::NonUtf8FileNames => "non-utf8-file-names",
            HostCapability::RenameUnderOpenHandle => "rename-under-open-handle",
            HostCapability::UnixProcessGroups => "unix-process-groups",
            HostCapability::UnixPermissionBits => "unix-permission-bits",
            HostCapability::WindowsJobObject => "windows-job-object",
            HostCapability::WindowsPathSyntax => "windows-path-syntax",
        }
    }
}

/// Report a skip a contract test returned with its capability name: a
/// contract in a crate without this helper (for example `unica-workspace`,
/// which may use processes only in tests) names the capability, and its
/// `#[test]` wrapper reports it here. An unknown name is a defect.
pub fn skip_named_host_capability(name: &str, reason: &str) {
    let capability = HostCapability::ALL
        .into_iter()
        .find(|capability| capability.name() == name)
        .unwrap_or_else(|| panic!("unknown host capability `{name}`"));
    skip_without_host_capability(capability, reason);
}

/// Record that this test skips because the host lacks `capability`.
/// The caller returns immediately afterwards; nothing else may skip.
pub fn skip_without_host_capability(capability: HostCapability, reason: &str) {
    eprintln!(
        "unica-host-capability-skip: {}: {reason}",
        capability.name()
    );
    let Some(directory) = std::env::var_os(HOST_CAPABILITY_SKIPS_ENV) else {
        return;
    };
    let binary = std::env::var("NEXTEST_BINARY_ID")
        .expect("host capability skips are recorded only under nextest");
    let name = std::env::var("NEXTEST_TEST_NAME")
        .expect("host capability skips are recorded only under nextest");
    let attempt = std::env::var("NEXTEST_ATTEMPT").unwrap_or_else(|_| "1".to_owned());
    write_skip_record(
        Path::new(&directory),
        &binary,
        &name,
        &attempt,
        capability,
        reason,
    );
}

/// One file per case and attempt; the runner reads the case from the record.
fn write_skip_record(
    directory: &Path,
    binary: &str,
    name: &str,
    attempt: &str,
    capability: HostCapability,
    reason: &str,
) {
    let record = serde_json::json!({
        "binary": binary,
        "name": name,
        "attempt": attempt,
        "capability": capability.name(),
        "reason": reason,
    });
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    std::hash::Hash::hash(&(binary, name), &mut hasher);
    let file = format!(
        "{:016x}__{attempt}.json",
        std::hash::Hasher::finish(&hasher)
    );
    std::fs::write(directory.join(file), record.to_string())
        .expect("write the host capability skip record");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_contract_skip_names_a_known_capability() {
        // Under the lane runner this records a skip of this very test, so it
        // names a capability no runner requires; the lookup is what is tested.
        skip_named_host_capability("windows-job-object", "the name resolves");
        let unknown = std::panic::catch_unwind(|| {
            skip_named_host_capability("teleport", "no such capability")
        });
        assert!(unknown.is_err(), "an unknown capability name is a defect");
    }

    #[test]
    fn a_skip_under_the_runner_leaves_one_record_naming_the_case() {
        let directory = tempfile::tempdir().unwrap();
        // The record is written from explicit inputs: no process environment
        // is changed, so concurrent `cargo test` threads cannot interfere.
        write_skip_record(
            directory.path(),
            "unica-demo::links",
            "tests::needs_a_link",
            "2",
            HostCapability::Symlink,
            "needs the symlink privilege",
        );
        let records: Vec<_> = std::fs::read_dir(directory.path())
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .collect();
        assert_eq!(records.len(), 1);
        assert!(records[0].to_string_lossy().ends_with("__2.json"));
        let record: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&records[0]).unwrap()).unwrap();
        assert_eq!(
            record,
            serde_json::json!({
                "binary": "unica-demo::links",
                "name": "tests::needs_a_link",
                "attempt": "2",
                "capability": "symlink",
                "reason": "needs the symlink privilege",
            })
        );
    }

    #[test]
    fn capability_names_are_unique_kebab_case() {
        let names: std::collections::BTreeSet<_> = HostCapability::ALL
            .iter()
            .map(|capability| capability.name())
            .collect();
        assert_eq!(names.len(), HostCapability::ALL.len());
        assert!(names.iter().all(|name| name
            .chars()
            .all(|character| character.is_ascii_lowercase()
                || character.is_ascii_digit()
                || character == '-')));
    }
}
