use super::project_sources::{SourceFormat, SourceSetKind};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fmt;

const SOURCE_FINGERPRINT_DOMAIN: &[u8] = b"unica.source-set-snapshot.v1";
const COMPOSITE_FINGERPRINT_DOMAIN: &[u8] = b"unica.source-composite.v1";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ResolvedSourceSet {
    pub(crate) name: String,
    pub(crate) kind: SourceSetKind,
    pub(crate) relative_root: String,
    pub(crate) source_format: SourceFormat,
    pub(crate) mapping_digest: String,
}

impl ResolvedSourceSet {
    pub(crate) fn new(
        name: String,
        kind: SourceSetKind,
        relative_root: String,
        source_format: SourceFormat,
        mapping_digest: String,
    ) -> Result<Self, String> {
        let source = Self {
            name,
            kind,
            relative_root,
            source_format,
            mapping_digest,
        };
        source.validate()?;
        Ok(source)
    }

    pub(crate) fn validate(&self) -> Result<(), String> {
        stable_component(&self.name, "source-set name", 1024)?;
        contained_relative_root(&self.relative_root)?;
        validate_fingerprint(&self.mapping_digest)?;
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ResolvedSourceSelection {
    pub(crate) mapping_digest: String,
    pub(crate) analysis: ResolvedSourceSet,
    pub(crate) mutations: Vec<ResolvedSourceSet>,
}

impl ResolvedSourceSelection {
    pub(crate) fn new(
        analysis: ResolvedSourceSet,
        mut mutations: Vec<ResolvedSourceSet>,
    ) -> Result<Self, String> {
        let mapping_digest = analysis.mapping_digest.clone();
        mutations.sort_by(|left, right| resolved_source_key(left).cmp(&resolved_source_key(right)));
        for pair in mutations.windows(2) {
            if pair[0].name.to_lowercase() == pair[1].name.to_lowercase() && pair[0] != pair[1] {
                return Err(
                    "one mutation source-set name cannot resolve to conflicting identities".into(),
                );
            }
        }
        mutations.dedup();
        let selection = Self {
            mapping_digest,
            analysis,
            mutations,
        };
        selection.validate()?;
        Ok(selection)
    }

    pub(crate) fn validate(&self) -> Result<(), String> {
        self.analysis.validate()?;
        for mutation in &self.mutations {
            mutation.validate()?;
        }
        if self.mapping_digest != self.analysis.mapping_digest
            || self
                .mutations
                .iter()
                .any(|mutation| mutation.mapping_digest != self.mapping_digest)
        {
            return Err("resolved sources must come from one mapping digest".into());
        }
        let mut normalized = self.mutations.clone();
        normalized
            .sort_by(|left, right| resolved_source_key(left).cmp(&resolved_source_key(right)));
        for pair in normalized.windows(2) {
            if pair[0].name.to_lowercase() == pair[1].name.to_lowercase() && pair[0] != pair[1] {
                return Err(
                    "one mutation source-set name cannot resolve to conflicting identities".into(),
                );
            }
        }
        normalized.dedup();
        if self.mutations != normalized {
            return Err("mutation source sets must be canonically sorted and unique".into());
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MaterialFile {
    pub(crate) byte_length: u64,
    pub(crate) content_digest: String,
}

impl MaterialFile {
    pub(crate) fn new(byte_length: u64, content_digest: String) -> Result<Self, String> {
        validate_fingerprint(&content_digest)?;
        Ok(Self {
            byte_length,
            content_digest,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ManifestEntry {
    Present(MaterialFile),
    AbsentOptional(OptionalMaterialTag),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum OptionalMaterialTag {
    ParentConfigurations,
    EdtProject,
    EdtProjectPmf,
    EdtConfigurationMdo,
    EdtSourceConfigurationMdo,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SourceManifest {
    entries: BTreeMap<String, ManifestEntry>,
}

impl SourceManifest {
    pub(crate) fn new(entries: BTreeMap<String, ManifestEntry>) -> Result<Self, String> {
        if entries.is_empty() {
            return Err("source manifest must not be empty".into());
        }
        for (path, entry) in &entries {
            contained_relative_file(path)?;
            match entry {
                ManifestEntry::Present(file) => validate_fingerprint(&file.content_digest)?,
                ManifestEntry::AbsentOptional(tag) if !optional_tag_matches_path(*tag, path) => {
                    return Err("optional-material tombstone must name its declared path".into());
                }
                ManifestEntry::AbsentOptional(_) => {}
            }
        }
        Ok(Self { entries })
    }

    // Task 5 evidence providers consume the immutable manifest directly.
    #[allow(dead_code)]
    pub(crate) fn entries(&self) -> &BTreeMap<String, ManifestEntry> {
        &self.entries
    }

    pub(crate) fn get(&self, path: &str) -> Option<&ManifestEntry> {
        self.entries.get(path)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SourceSetSnapshot {
    pub(crate) source_set: ResolvedSourceSet,
    pub(crate) source_fingerprint: String,
    pub(crate) manifest: SourceManifest,
}

impl SourceSetSnapshot {
    pub(crate) fn from_manifest(
        source_set: ResolvedSourceSet,
        manifest: SourceManifest,
    ) -> Result<Self, String> {
        source_set.validate()?;
        let source_fingerprint = source_fingerprint(&source_set, &manifest)?;
        Ok(Self {
            source_set,
            source_fingerprint,
            manifest,
        })
    }

    pub(crate) fn validate(&self) -> Result<(), String> {
        self.source_set.validate()?;
        validate_fingerprint(&self.source_fingerprint)?;
        if self.source_fingerprint != source_fingerprint(&self.source_set, &self.manifest)? {
            return Err("source fingerprint does not match source identity and manifest".into());
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SourceSnapshot {
    pub(crate) analysis: SourceSetSnapshot,
    pub(crate) mutations: Vec<SourceSetSnapshot>,
    pub(crate) composite_fingerprint: String,
    pub(crate) workspace_epoch: u64,
}

impl SourceSnapshot {
    pub(crate) fn new(
        analysis: SourceSetSnapshot,
        mut mutations: Vec<SourceSetSnapshot>,
        workspace_epoch: u64,
    ) -> Result<Self, String> {
        analysis.validate()?;
        for mutation in &mutations {
            mutation.validate()?;
        }
        mutations.sort_by(|left, right| snapshot_key(left).cmp(&snapshot_key(right)));
        for pair in mutations.windows(2) {
            if pair[0].source_set.name.to_lowercase() == pair[1].source_set.name.to_lowercase()
                && pair[0] != pair[1]
            {
                return Err(
                    "one mutation source-set name cannot have conflicting snapshots".into(),
                );
            }
        }
        mutations.dedup();
        let composite_fingerprint = composite_fingerprint(&analysis, &mutations)?;
        let snapshot = Self {
            analysis,
            mutations,
            composite_fingerprint,
            workspace_epoch,
        };
        snapshot.validate()?;
        Ok(snapshot)
    }

    pub(crate) fn validate(&self) -> Result<(), String> {
        self.analysis.validate()?;
        for mutation in &self.mutations {
            mutation.validate()?;
        }
        if self
            .mutations
            .windows(2)
            .any(|pair| snapshot_key(&pair[0]) >= snapshot_key(&pair[1]))
        {
            return Err("mutation source snapshots must be canonically sorted and unique".into());
        }
        let expected = composite_fingerprint(&self.analysis, &self.mutations)?;
        if self.composite_fingerprint != expected {
            return Err("composite fingerprint does not match linked source snapshots".into());
        }
        Ok(())
    }

    pub(crate) fn snapshot_named(&self, name: &str) -> Option<&SourceSetSnapshot> {
        std::iter::once(&self.analysis)
            .chain(self.mutations.iter())
            .find(|snapshot| snapshot.source_set.name == name)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SourceReadError {
    NotInManifest { path: String },
    SourceFingerprintMismatch { path: String },
    SnapshotUnavailable { path: String, detail: String },
}

impl SourceReadError {
    pub(crate) fn reason_code(&self) -> &'static str {
        match self {
            Self::NotInManifest { .. } => "source_path_not_in_manifest",
            Self::SourceFingerprintMismatch { .. } => "source_fingerprint_mismatch",
            Self::SnapshotUnavailable { .. } => "source_snapshot_unavailable",
        }
    }

    #[allow(dead_code)]
    pub(crate) fn retryable(&self) -> bool {
        !matches!(self, Self::NotInManifest { .. })
    }
}

impl fmt::Display for SourceReadError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotInManifest { path } => {
                write!(formatter, "{}: {path}", self.reason_code())
            }
            Self::SourceFingerprintMismatch { path } => {
                write!(formatter, "{}: {path}", self.reason_code())
            }
            Self::SnapshotUnavailable { path, detail } => {
                write!(formatter, "{}: {path}: {detail}", self.reason_code())
            }
        }
    }
}

impl std::error::Error for SourceReadError {}

fn source_fingerprint(
    source_set: &ResolvedSourceSet,
    manifest: &SourceManifest,
) -> Result<String, String> {
    let mut encoder = FingerprintEncoder::new(SOURCE_FINGERPRINT_DOMAIN);
    encode_source_identity(&mut encoder, source_set)?;
    encoder.write_u64(manifest.entries.len() as u64);
    for (path, entry) in &manifest.entries {
        encoder.write_string(path)?;
        match entry {
            ManifestEntry::Present(file) => {
                encoder.write_u8(1);
                encoder.write_u64(file.byte_length);
                encoder.write_string(&file.content_digest)?;
            }
            ManifestEntry::AbsentOptional(tag) => {
                encoder.write_u8(2);
                encoder.write_u8(match tag {
                    OptionalMaterialTag::ParentConfigurations => 1,
                    OptionalMaterialTag::EdtProject => 2,
                    OptionalMaterialTag::EdtProjectPmf => 3,
                    OptionalMaterialTag::EdtConfigurationMdo => 4,
                    OptionalMaterialTag::EdtSourceConfigurationMdo => 5,
                });
            }
        }
    }
    Ok(encoder.finish())
}

fn composite_fingerprint(
    analysis: &SourceSetSnapshot,
    mutations: &[SourceSetSnapshot],
) -> Result<String, String> {
    let mut encoder = FingerprintEncoder::new(COMPOSITE_FINGERPRINT_DOMAIN);
    encoder.write_u8(1);
    encode_source_identity(&mut encoder, &analysis.source_set)?;
    encoder.write_string(&analysis.source_fingerprint)?;
    encoder.write_u64(mutations.len() as u64);
    for mutation in mutations {
        encoder.write_u8(2);
        encode_source_identity(&mut encoder, &mutation.source_set)?;
        encoder.write_string(&mutation.source_fingerprint)?;
    }
    Ok(encoder.finish())
}

fn encode_source_identity(
    encoder: &mut FingerprintEncoder,
    source_set: &ResolvedSourceSet,
) -> Result<(), String> {
    source_set.validate()?;
    encoder.write_string(&source_set.name)?;
    encoder.write_u8(source_set_kind_tag(source_set.kind));
    encoder.write_u8(source_format_tag(source_set.source_format));
    encoder.write_string(&source_set.relative_root)?;
    encoder.write_string(&source_set.mapping_digest)?;
    Ok(())
}

struct FingerprintEncoder {
    hasher: Sha256,
}

impl FingerprintEncoder {
    fn new(domain: &[u8]) -> Self {
        let mut hasher = Sha256::new();
        hasher.update((domain.len() as u64).to_be_bytes());
        hasher.update(domain);
        Self { hasher }
    }

    fn write_u8(&mut self, value: u8) {
        self.hasher.update([value]);
    }

    fn write_u64(&mut self, value: u64) {
        self.hasher.update(value.to_be_bytes());
    }

    fn write_string(&mut self, value: &str) -> Result<(), String> {
        let length = u64::try_from(value.len()).map_err(|_| "value is too large to hash")?;
        self.write_u64(length);
        self.hasher.update(value.as_bytes());
        Ok(())
    }

    fn finish(self) -> String {
        format!("sha256:{:x}", self.hasher.finalize())
    }
}

fn resolved_source_key(source: &ResolvedSourceSet) -> (String, u8, u8, &str, &str) {
    (
        source.name.to_lowercase(),
        source_set_kind_tag(source.kind),
        source_format_tag(source.source_format),
        source.relative_root.as_str(),
        source.mapping_digest.as_str(),
    )
}

fn snapshot_key(snapshot: &SourceSetSnapshot) -> (String, u8, u8, &str, &str, &str) {
    let source = &snapshot.source_set;
    (
        source.name.to_lowercase(),
        source_set_kind_tag(source.kind),
        source_format_tag(source.source_format),
        source.relative_root.as_str(),
        source.mapping_digest.as_str(),
        snapshot.source_fingerprint.as_str(),
    )
}

fn source_set_kind_tag(kind: SourceSetKind) -> u8 {
    match kind {
        SourceSetKind::Configuration => 1,
        SourceSetKind::Extension => 2,
        SourceSetKind::ExternalProcessor => 3,
        SourceSetKind::ExternalReport => 4,
    }
}

fn source_format_tag(format: SourceFormat) -> u8 {
    match format {
        SourceFormat::PlatformXml => 1,
        SourceFormat::Edt => 2,
        SourceFormat::Unknown => 3,
        SourceFormat::Invalid => 4,
    }
}

fn stable_component(value: &str, field: &str, maximum: usize) -> Result<(), String> {
    if value.trim().is_empty() || value.len() > maximum || value.chars().any(char::is_control) {
        return Err(format!("{field} must contain 1..={maximum} stable bytes"));
    }
    Ok(())
}

fn contained_relative_root(path: &str) -> Result<(), String> {
    if path == "." {
        return Ok(());
    }
    contained_relative_file(path)
        .map_err(|_| "source root must be `.` or a contained workspace-relative slash path".into())
}

fn contained_relative_file(path: &str) -> Result<(), String> {
    if path.is_empty()
        || path == "."
        || path.len() > 4096
        || path.starts_with('/')
        || path.starts_with('\\')
        || path.contains('\\')
        || path.contains(':')
        || path.chars().any(char::is_control)
        || path
            .split('/')
            .any(|component| component.is_empty() || matches!(component, "." | ".."))
    {
        return Err("path must be a contained workspace-relative slash path".into());
    }
    Ok(())
}

fn validate_fingerprint(value: &str) -> Result<(), String> {
    let Some(digest) = value.strip_prefix("sha256:") else {
        return Err("fingerprint must start with sha256:".into());
    };
    if digest.len() != 64
        || !digest
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err("fingerprint must contain 64 lowercase hexadecimal characters".into());
    }
    Ok(())
}

fn optional_tag_matches_path(tag: OptionalMaterialTag, path: &str) -> bool {
    let suffix = match tag {
        OptionalMaterialTag::ParentConfigurations => "Ext/ParentConfigurations.bin",
        OptionalMaterialTag::EdtProject => ".project",
        OptionalMaterialTag::EdtProjectPmf => "DT-INF/PROJECT.PMF",
        OptionalMaterialTag::EdtConfigurationMdo => "Configuration/Configuration.mdo",
        OptionalMaterialTag::EdtSourceConfigurationMdo => "src/Configuration/Configuration.mdo",
    };
    path == suffix
        || path
            .strip_suffix(suffix)
            .is_some_and(|prefix| prefix.ends_with('/'))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn resolved(name: &str) -> ResolvedSourceSet {
        ResolvedSourceSet::new(
            name.into(),
            SourceSetKind::Extension,
            format!("src/{name}"),
            SourceFormat::PlatformXml,
            format!("sha256:{}", "a".repeat(64)),
        )
        .unwrap()
    }

    fn snapshot(name: &str, byte: u8) -> SourceSetSnapshot {
        let path = format!("src/{name}/Configuration.xml");
        let manifest = SourceManifest::new(BTreeMap::from([(
            path,
            ManifestEntry::Present(MaterialFile::new(1, format!("sha256:{:064x}", byte)).unwrap()),
        )]))
        .unwrap();
        SourceSetSnapshot::from_manifest(resolved(name), manifest).unwrap()
    }

    #[test]
    fn source_snapshot_computes_composite_and_sorts_deduplicates_mutations() {
        let analysis = snapshot("main", 1);
        let mutation_a = snapshot("a", 2);
        let mutation_b = snapshot("b", 3);
        let first = SourceSnapshot::new(
            analysis.clone(),
            vec![mutation_b.clone(), mutation_a.clone(), mutation_b.clone()],
            9,
        )
        .unwrap();
        let second = SourceSnapshot::new(analysis, vec![mutation_a, mutation_b], 9).unwrap();

        assert_eq!(first, second);
        assert_eq!(first.workspace_epoch, 9);
    }

    #[test]
    fn composite_snapshot_binds_analysis_and_destination() {
        let a =
            SourceSnapshot::new(snapshot("main", 1), vec![snapshot("ExtensionA", 2)], 9).unwrap();
        let b =
            SourceSnapshot::new(snapshot("main", 1), vec![snapshot("ExtensionB", 2)], 9).unwrap();

        assert_ne!(a.composite_fingerprint, b.composite_fingerprint);
    }

    #[test]
    fn source_fingerprint_binds_mapping_name_kind_format_and_root() {
        let baseline = snapshot("base", 1);
        let variants = [
            ResolvedSourceSet::new(
                "renamed".into(),
                baseline.source_set.kind,
                baseline.source_set.relative_root.clone(),
                baseline.source_set.source_format,
                baseline.source_set.mapping_digest.clone(),
            )
            .unwrap(),
            ResolvedSourceSet::new(
                baseline.source_set.name.clone(),
                SourceSetKind::Configuration,
                baseline.source_set.relative_root.clone(),
                baseline.source_set.source_format,
                baseline.source_set.mapping_digest.clone(),
            )
            .unwrap(),
            ResolvedSourceSet::new(
                baseline.source_set.name.clone(),
                baseline.source_set.kind,
                "different/root".into(),
                baseline.source_set.source_format,
                baseline.source_set.mapping_digest.clone(),
            )
            .unwrap(),
            ResolvedSourceSet::new(
                baseline.source_set.name.clone(),
                baseline.source_set.kind,
                baseline.source_set.relative_root.clone(),
                SourceFormat::Edt,
                baseline.source_set.mapping_digest.clone(),
            )
            .unwrap(),
            ResolvedSourceSet::new(
                baseline.source_set.name.clone(),
                baseline.source_set.kind,
                baseline.source_set.relative_root.clone(),
                baseline.source_set.source_format,
                format!("sha256:{}", "b".repeat(64)),
            )
            .unwrap(),
        ];
        for variant in variants {
            let changed =
                SourceSetSnapshot::from_manifest(variant, baseline.manifest.clone()).unwrap();
            assert_ne!(baseline.source_fingerprint, changed.source_fingerprint);
        }
    }

    #[test]
    fn workspace_root_dot_is_valid_but_embedded_dot_and_traversal_are_not() {
        assert!(ResolvedSourceSet::new(
            "main".into(),
            SourceSetKind::Configuration,
            ".".into(),
            SourceFormat::PlatformXml,
            format!("sha256:{}", "a".repeat(64)),
        )
        .is_ok());
        for invalid in ["", "./src", "src/./cf", "src/../cf", "/src", "C:/src"] {
            assert!(
                ResolvedSourceSet::new(
                    "main".into(),
                    SourceSetKind::Configuration,
                    invalid.into(),
                    SourceFormat::PlatformXml,
                    format!("sha256:{}", "a".repeat(64)),
                )
                .is_err(),
                "accepted {invalid}"
            );
        }
    }

    #[test]
    fn selection_rejects_mixed_mapping_versions() {
        let analysis = resolved("main");
        let mut mutation = resolved("extension");
        mutation.mapping_digest = format!("sha256:{}", "b".repeat(64));

        assert!(ResolvedSourceSelection::new(analysis, vec![mutation]).is_err());
    }

    #[test]
    fn selection_validation_rejects_forged_digest_order_and_duplicates() {
        let valid =
            ResolvedSourceSelection::new(resolved("main"), vec![resolved("a"), resolved("b")])
                .unwrap();

        let mut mixed_digest = valid.clone();
        mixed_digest.mapping_digest = format!("sha256:{}", "b".repeat(64));
        assert!(mixed_digest.validate().is_err());

        let mut reordered = valid.clone();
        reordered.mutations.reverse();
        assert!(reordered.validate().is_err());

        let mut duplicate = valid;
        duplicate.mutations.push(duplicate.mutations[0].clone());
        assert!(duplicate.validate().is_err());
    }
}
