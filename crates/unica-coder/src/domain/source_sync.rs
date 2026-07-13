//! Domain model for durable source-to-infobase synchronization.
//!
//! The model deliberately fingerprints raw bytes. 1C module BOMs and line
//! endings are therefore part of the synchronization contract rather than an
//! incidental text-normalization detail.

use serde::{Deserialize, Deserializer, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

pub const SOURCE_SYNC_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct TargetId(String);

impl TargetId {
    pub fn new(value: impl Into<String>) -> Result<Self, String> {
        let value = value.into();
        validate_nonblank_identifier("target id", &value)?;
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl<'de> Deserialize<'de> for TargetId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::new(value).map_err(serde::de::Error::custom)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct SourceSetName(String);

impl SourceSetName {
    pub fn new(value: impl Into<String>) -> Result<Self, String> {
        let value = value.into();
        validate_nonblank_identifier("source-set name", &value)?;
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl<'de> Deserialize<'de> for SourceSetName {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::new(value).map_err(serde::de::Error::custom)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct RelativeSourcePath(String);

impl RelativeSourcePath {
    pub fn new(value: impl Into<String>) -> Result<Self, String> {
        let value = value.into();
        validate_relative_source_path(&value)?;
        Ok(Self(value))
    }

    pub fn workspace_root() -> Self {
        Self(".".to_string())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl<'de> Deserialize<'de> for RelativeSourcePath {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::new(value).map_err(serde::de::Error::custom)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct Sha256Digest(String);

impl Sha256Digest {
    pub fn from_bytes(bytes: &[u8]) -> Self {
        Self(format!("sha256:{:x}", Sha256::digest(bytes)))
    }

    pub fn parse(value: impl Into<String>) -> Result<Self, String> {
        let value = value.into();
        let Some(hex) = value.strip_prefix("sha256:") else {
            return Err("SHA-256 digest must start with `sha256:`".to_string());
        };
        if hex.len() != 64 || !hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err("SHA-256 digest must contain exactly 64 hexadecimal digits".to_string());
        }
        if hex.bytes().any(|byte| byte.is_ascii_uppercase()) {
            return Err("SHA-256 digest must use canonical lowercase hexadecimal".to_string());
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl<'de> Deserialize<'de> for Sha256Digest {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::parse(value).map_err(serde::de::Error::custom)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "camelCase", deny_unknown_fields)]
pub enum FileFingerprint {
    Present { sha256: Sha256Digest, bytes: u64 },
    Deleted,
}

impl FileFingerprint {
    pub fn present(bytes: &[u8]) -> Self {
        Self::Present {
            sha256: Sha256Digest::from_bytes(bytes),
            bytes: bytes.len() as u64,
        }
    }

    pub fn sha256(&self) -> Option<&Sha256Digest> {
        match self {
            Self::Present { sha256, bytes: _ } => Some(sha256),
            Self::Deleted => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "camelCase", deny_unknown_fields)]
pub enum SynchronizedFileFingerprint {
    Present { sha256: Sha256Digest, bytes: u64 },
    Deleted,
    Unknown,
}

impl SynchronizedFileFingerprint {
    pub fn matches_current(&self, current: &FileFingerprint) -> bool {
        match (self, current) {
            (
                Self::Present {
                    sha256: synchronized_hash,
                    bytes: synchronized_bytes,
                },
                FileFingerprint::Present {
                    sha256: current_hash,
                    bytes: current_bytes,
                },
            ) => synchronized_hash == current_hash && synchronized_bytes == current_bytes,
            (Self::Deleted, FileFingerprint::Deleted) => true,
            (Self::Present { .. }, FileFingerprint::Deleted)
            | (Self::Deleted, FileFingerprint::Present { .. })
            | (Self::Unknown, FileFingerprint::Present { .. } | FileFingerprint::Deleted) => false,
        }
    }
}

impl From<&FileFingerprint> for SynchronizedFileFingerprint {
    fn from(value: &FileFingerprint) -> Self {
        match value {
            FileFingerprint::Present { sha256, bytes } => Self::Present {
                sha256: sha256.clone(),
                bytes: *bytes,
            },
            FileFingerprint::Deleted => Self::Deleted,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SourceManifest {
    pub files: BTreeMap<RelativeSourcePath, FileFingerprint>,
}

impl SourceManifest {
    pub fn with_missing_paths_from(mut self, other: &Self) -> Self {
        for path in other.files.keys() {
            self.files
                .entry(path.clone())
                .or_insert(FileFingerprint::Deleted);
        }
        self
    }

    pub fn paths(&self) -> impl Iterator<Item = &RelativeSourcePath> {
        self.files.keys()
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SynchronizedManifest {
    pub files: BTreeMap<RelativeSourcePath, SynchronizedFileFingerprint>,
}

impl SynchronizedManifest {
    pub fn known(manifest: &SourceManifest) -> Self {
        Self {
            files: manifest
                .files
                .iter()
                .map(|(path, fingerprint)| (path.clone(), fingerprint.into()))
                .collect(),
        }
    }

    pub fn unknown(paths: impl IntoIterator<Item = RelativeSourcePath>) -> Self {
        Self {
            files: paths
                .into_iter()
                .map(|path| (path, SynchronizedFileFingerprint::Unknown))
                .collect(),
        }
    }

    pub fn matches_current(&self, current: &SourceManifest) -> bool {
        let paths = self
            .files
            .keys()
            .chain(current.files.keys())
            .collect::<BTreeSet<_>>();
        paths.into_iter().all(|path| {
            let current = current.files.get(path).unwrap_or(&FileFingerprint::Deleted);
            self.files
                .get(path)
                .unwrap_or(&SynchronizedFileFingerprint::Unknown)
                .matches_current(current)
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SourceTargetKind {
    Module,
    MetadataOwner,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum SourceTargetScope {
    Module {
        path: RelativeSourcePath,
    },
    MetadataOwner {
        descriptor_path: RelativeSourcePath,
        owner_directory: RelativeSourcePath,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SourceTarget {
    pub id: TargetId,
    pub target_kind: SourceTargetKind,
    pub source_set: Option<SourceSetName>,
    pub source_root: RelativeSourcePath,
    pub owner_selector: String,
    pub scope: SourceTargetScope,
}

impl SourceTarget {
    pub fn validate(&self) -> Result<(), String> {
        validate_nonblank_identifier("owner selector", &self.owner_selector)?;
        match (&self.target_kind, &self.scope) {
            (SourceTargetKind::Module, SourceTargetScope::Module { path }) => {
                validate_owned_path(self, path)?;
                if is_config_dump_info(path) {
                    return Err(format!(
                        "module target `{}` cannot point at ConfigDumpInfo.xml",
                        self.id.as_str()
                    ));
                }
                Ok(())
            }
            (
                SourceTargetKind::MetadataOwner,
                SourceTargetScope::MetadataOwner {
                    descriptor_path,
                    owner_directory,
                },
            ) => {
                validate_owned_path(self, descriptor_path)?;
                validate_owned_path(self, owner_directory)?;
                if descriptor_path == owner_directory || is_config_dump_info(descriptor_path) {
                    return Err(format!(
                        "metadata target `{}` has an invalid descriptor/owner scope",
                        self.id.as_str()
                    ));
                }
                Ok(())
            }
            (SourceTargetKind::Module, SourceTargetScope::MetadataOwner { .. })
            | (SourceTargetKind::MetadataOwner, SourceTargetScope::Module { .. }) => Err(format!(
                "target `{}` has inconsistent targetKind and scope",
                self.id.as_str()
            )),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SourceTargetRecord {
    pub target: SourceTarget,
    pub current: SourceManifest,
    pub synchronized: SynchronizedManifest,
}

impl SourceTargetRecord {
    pub fn is_dirty(&self) -> bool {
        !self.synchronized.matches_current(&self.current)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SourceSyncState {
    pub schema_version: u32,
    pub generation: u64,
    pub workspace_id: String,
    pub workspace_root: String,
    pub targets: BTreeMap<TargetId, SourceTargetRecord>,
}

impl SourceSyncState {
    pub fn empty(workspace_id: impl Into<String>, workspace_root: impl Into<String>) -> Self {
        Self {
            schema_version: SOURCE_SYNC_SCHEMA_VERSION,
            generation: 0,
            workspace_id: workspace_id.into(),
            workspace_root: workspace_root.into(),
            targets: BTreeMap::new(),
        }
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.schema_version != SOURCE_SYNC_SCHEMA_VERSION {
            return Err(format!(
                "unsupported source-sync schemaVersion {}; expected {}",
                self.schema_version, SOURCE_SYNC_SCHEMA_VERSION
            ));
        }
        validate_nonblank_identifier("workspace id", &self.workspace_id)?;
        validate_nonblank_identifier("workspace root", &self.workspace_root)?;
        for (id, record) in &self.targets {
            if id != &record.target.id {
                return Err(format!(
                    "source-sync target map key `{}` does not match record id `{}`",
                    id.as_str(),
                    record.target.id.as_str()
                ));
            }
            record.target.validate()?;
            validate_manifest_paths(record)?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BuildStepMode {
    EdtExport,
    Full,
    Partial { file_count: usize },
    Skipped,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BuildTerminalEntry {
    pub source_set: SourceSetName,
    pub mode: BuildStepMode,
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TargetTerminalStatus {
    Processed,
    Skipped,
    Conflicted,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DumpTerminalEntry {
    pub target_id: TargetId,
    pub source_set: Option<SourceSetName>,
    pub owner_selector: String,
    pub status: TargetTerminalStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expected: Option<SourceManifest>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub current: Option<SourceManifest>,
}

fn validate_manifest_paths(record: &SourceTargetRecord) -> Result<(), String> {
    for path in record
        .current
        .files
        .keys()
        .chain(record.synchronized.files.keys())
    {
        if is_config_dump_info(path) || !target_owns_manifest_path(&record.target, path) {
            return Err(format!(
                "target `{}` illegally fingerprints {}",
                record.target.id.as_str(),
                path.as_str()
            ));
        }
    }
    Ok(())
}

fn validate_owned_path(target: &SourceTarget, path: &RelativeSourcePath) -> Result<(), String> {
    if path_is_within(&target.source_root, path) && path != &target.source_root {
        Ok(())
    } else {
        Err(format!(
            "target `{}` path `{}` is outside source root `{}`",
            target.id.as_str(),
            path.as_str(),
            target.source_root.as_str()
        ))
    }
}

fn target_owns_manifest_path(target: &SourceTarget, path: &RelativeSourcePath) -> bool {
    if !path_is_within(&target.source_root, path) || path == &target.source_root {
        return false;
    }
    match &target.scope {
        SourceTargetScope::Module { path: module_path } => path == module_path,
        SourceTargetScope::MetadataOwner {
            descriptor_path,
            owner_directory,
        } => {
            path == descriptor_path
                || path != owner_directory && path_is_within(owner_directory, path)
        }
    }
}

fn path_is_within(root: &RelativeSourcePath, path: &RelativeSourcePath) -> bool {
    if root.as_str() == "." {
        return true;
    }
    path.as_str() == root.as_str()
        || path
            .as_str()
            .strip_prefix(root.as_str())
            .is_some_and(|suffix| suffix.starts_with('/'))
}

fn is_config_dump_info(path: &RelativeSourcePath) -> bool {
    path.as_str()
        .rsplit('/')
        .next()
        .is_some_and(|name| name.eq_ignore_ascii_case("ConfigDumpInfo.xml"))
}

fn validate_nonblank_identifier(name: &str, value: &str) -> Result<(), String> {
    if value.trim().is_empty() {
        return Err(format!("{name} must not be blank"));
    }
    if value.chars().any(char::is_control) {
        return Err(format!("{name} must not contain control characters"));
    }
    Ok(())
}

fn validate_relative_source_path(value: &str) -> Result<(), String> {
    if value == "." {
        return Ok(());
    }
    if value.is_empty() || value.starts_with('/') || value.starts_with('\\') {
        return Err(format!(
            "source path `{value}` must be a non-empty relative path"
        ));
    }
    if value.contains('\\') {
        return Err(format!("source path `{value}` must use `/` separators"));
    }
    if value
        .split('/')
        .any(|part| part.is_empty() || part == "." || part == "..")
    {
        return Err(format!(
            "source path `{value}` must not contain empty, `.` or `..` components"
        ));
    }
    if value.chars().any(char::is_control) {
        return Err(format!(
            "source path `{value}` contains a control character"
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn raw_fingerprint_is_canonical_and_sensitive_to_bom_and_eol() {
        let lf = FileFingerprint::present("Сообщить(\"ok\");\n".as_bytes());
        let crlf = FileFingerprint::present("Сообщить(\"ok\");\r\n".as_bytes());
        let bom_lf = FileFingerprint::present(
            [b"\xef\xbb\xbf".as_slice(), "Сообщить(\"ok\");\n".as_bytes()]
                .concat()
                .as_slice(),
        );

        assert_ne!(lf, crlf);
        assert_ne!(lf, bom_lf);
        let hash = lf.sha256().expect("present files have a hash");
        assert!(hash.as_str().starts_with("sha256:"));
        assert_eq!(hash.as_str().len(), 71);
    }

    #[test]
    fn dirty_is_derived_from_current_and_synchronized_files() {
        let path = RelativeSourcePath::new("src/CommonModules/Demo/Ext/Module.bsl").unwrap();
        let current = SourceManifest {
            files: BTreeMap::from([(path.clone(), FileFingerprint::present(b"A\n"))]),
        };
        let target = module_target(path);
        let clean = SourceTargetRecord {
            target: target.clone(),
            synchronized: SynchronizedManifest::known(&current),
            current: current.clone(),
        };
        assert!(!clean.is_dirty());

        let unknown = SourceTargetRecord {
            target,
            synchronized: SynchronizedManifest::unknown(current.files.keys().cloned()),
            current,
        };
        assert!(unknown.is_dirty());
    }

    #[test]
    fn serde_uses_stable_camel_case_and_rejects_invalid_digest() {
        let path = RelativeSourcePath::new("src/CommonModules/Demo/Ext/Module.bsl").unwrap();
        let current = SourceManifest {
            files: BTreeMap::from([(path.clone(), FileFingerprint::present(b"A\n"))]),
        };
        let state = SourceSyncState {
            schema_version: SOURCE_SYNC_SCHEMA_VERSION,
            generation: 3,
            workspace_id: "abc".to_string(),
            workspace_root: "/workspace".to_string(),
            targets: BTreeMap::from([(
                module_target(path).id.clone(),
                SourceTargetRecord {
                    target: module_target(
                        RelativeSourcePath::new("src/CommonModules/Demo/Ext/Module.bsl").unwrap(),
                    ),
                    synchronized: SynchronizedManifest::known(&current),
                    current,
                },
            )]),
        };

        let value = serde_json::to_value(&state).unwrap();
        assert_eq!(value["schemaVersion"], SOURCE_SYNC_SCHEMA_VERSION);
        assert!(value["workspaceId"].is_string());
        assert!(value["targets"].is_object());

        let mut invalid = value;
        let target = invalid["targets"]
            .as_object_mut()
            .unwrap()
            .values_mut()
            .next()
            .unwrap();
        let fingerprint = target["current"]["files"]
            .as_object_mut()
            .unwrap()
            .values_mut()
            .next()
            .unwrap();
        fingerprint["sha256"] = serde_json::json!("sha256:not-a-digest");
        assert!(serde_json::from_value::<SourceSyncState>(invalid).is_err());

        let metadata_scope = SourceTargetScope::MetadataOwner {
            descriptor_path: RelativeSourcePath::new("src/Catalogs/Goods.xml").unwrap(),
            owner_directory: RelativeSourcePath::new("src/Catalogs/Goods").unwrap(),
        };
        let scope_value = serde_json::to_value(metadata_scope).unwrap();
        assert_eq!(scope_value["kind"], "metadataOwner");
        assert!(scope_value.get("descriptorPath").is_some());
        assert!(scope_value.get("ownerDirectory").is_some());
        assert!(scope_value.get("descriptor_path").is_none());
    }

    #[test]
    fn rejects_unsafe_relative_paths_and_inconsistent_target_scope() {
        for invalid in ["", "/tmp/file", "../file", "src/../file", "src\\file"] {
            assert!(RelativeSourcePath::new(invalid).is_err(), "{invalid}");
        }

        let path = RelativeSourcePath::new("src/Module.bsl").unwrap();
        let mut target = module_target(path.clone());
        target.target_kind = SourceTargetKind::MetadataOwner;
        assert!(target.validate().is_err());

        let outside = module_target(RelativeSourcePath::new("other/Module.bsl").unwrap());
        assert!(outside.validate().is_err());
    }

    #[test]
    fn state_rejects_manifest_paths_not_owned_by_the_declared_target() {
        let module_path = RelativeSourcePath::new("src/CommonModules/Demo/Ext/Module.bsl").unwrap();
        let target = module_target(module_path);
        let foreign_path = RelativeSourcePath::new("src/Catalogs/Items.xml").unwrap();
        let current = SourceManifest {
            files: BTreeMap::from([(foreign_path, FileFingerprint::present(b"foreign"))]),
        };
        let state = SourceSyncState {
            schema_version: SOURCE_SYNC_SCHEMA_VERSION,
            generation: 1,
            workspace_id: "workspace".to_string(),
            workspace_root: "/workspace".to_string(),
            targets: BTreeMap::from([(
                target.id.clone(),
                SourceTargetRecord {
                    target,
                    synchronized: SynchronizedManifest::known(&current),
                    current,
                },
            )]),
        };

        assert!(state
            .validate()
            .unwrap_err()
            .contains("illegally fingerprints"));
    }

    fn module_target(path: RelativeSourcePath) -> SourceTarget {
        SourceTarget {
            id: TargetId::new("module:main:CommonModules/Demo/Ext/Module.bsl").unwrap(),
            target_kind: SourceTargetKind::Module,
            source_set: Some(SourceSetName::new("main").unwrap()),
            source_root: RelativeSourcePath::new("src").unwrap(),
            owner_selector: "CommonModule:Demo".to_string(),
            scope: SourceTargetScope::Module { path },
        }
    }
}
