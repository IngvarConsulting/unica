use super::project_sources::{SourceFormat, SourceSetKind};
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ResolvedSourceSet {
    pub(crate) name: String,
    pub(crate) kind: SourceSetKind,
    pub(crate) relative_root: String,
    pub(crate) source_format: SourceFormat,
    pub(crate) mapping_digest: String,
}

impl ResolvedSourceSet {
    pub(crate) fn validate(&self) -> Result<(), String> {
        stable_component(&self.name, "source-set name", 1024)?;
        contained_relative_path(&self.relative_root)?;
        validate_fingerprint(&self.mapping_digest)?;
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SourceSetSnapshot {
    pub(crate) source_set: ResolvedSourceSet,
    pub(crate) source_fingerprint: String,
}

impl SourceSetSnapshot {
    pub(crate) fn validate(&self) -> Result<(), String> {
        self.source_set.validate()?;
        validate_fingerprint(&self.source_fingerprint)
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
    // Concrete filesystem capture is delivered in Task 4; fakes and that
    // adapter construct snapshots through this invariant-preserving boundary.
    #[allow(dead_code)]
    pub(crate) fn new(
        analysis: SourceSetSnapshot,
        mut mutations: Vec<SourceSetSnapshot>,
        composite_fingerprint: String,
        workspace_epoch: u64,
    ) -> Result<Self, String> {
        analysis.validate()?;
        for mutation in &mutations {
            mutation.validate()?;
        }
        mutations.sort_by(|left, right| {
            snapshot_key(SnapshotRoleKey::Mutation, left)
                .cmp(&snapshot_key(SnapshotRoleKey::Mutation, right))
        });
        mutations.dedup();
        validate_fingerprint(&composite_fingerprint)?;
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
        validate_fingerprint(&self.composite_fingerprint)?;
        let mut role_identities = BTreeMap::new();
        role_identities.insert(
            (
                SnapshotRoleKey::Analysis,
                self.analysis.source_set.name.as_str(),
            ),
            &self.analysis,
        );
        let mut previous = None;
        for mutation in &self.mutations {
            mutation.validate()?;
            if let Some(previous_snapshot) = previous {
                if snapshot_key(SnapshotRoleKey::Mutation, previous_snapshot)
                    > snapshot_key(SnapshotRoleKey::Mutation, mutation)
                {
                    return Err("mutation source snapshots must be canonically sorted".to_string());
                }
            }
            if let Some(existing) = role_identities.insert(
                (SnapshotRoleKey::Mutation, mutation.source_set.name.as_str()),
                mutation,
            ) {
                if existing != mutation {
                    return Err(
                        "one source-set role cannot have conflicting snapshot identities"
                            .to_string(),
                    );
                }
            }
            previous = Some(mutation);
        }
        if self.mutations.windows(2).any(|pair| pair[0] == pair[1]) {
            return Err("mutation source snapshots must be deduplicated".to_string());
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum SnapshotRoleKey {
    Analysis,
    Mutation,
}

fn snapshot_key(
    role: SnapshotRoleKey,
    snapshot: &SourceSetSnapshot,
) -> (SnapshotRoleKey, &str, u8, u8, &str, &str, &str) {
    (
        role,
        snapshot.source_set.name.as_str(),
        source_set_kind_tag(snapshot.source_set.kind),
        source_format_tag(snapshot.source_set.source_format),
        snapshot.source_set.relative_root.as_str(),
        snapshot.source_set.mapping_digest.as_str(),
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

fn contained_relative_path(path: &str) -> Result<(), String> {
    if path.is_empty()
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
        return Err("source root must be a contained workspace-relative slash path".to_string());
    }
    Ok(())
}

fn validate_fingerprint(value: &str) -> Result<(), String> {
    let Some(digest) = value.strip_prefix("sha256:") else {
        return Err("fingerprint must start with sha256:".to_string());
    };
    if digest.len() != 64
        || !digest
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err("fingerprint must contain 64 lowercase hexadecimal characters".to_string());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn resolved(name: &str, source_format: SourceFormat) -> ResolvedSourceSet {
        ResolvedSourceSet {
            name: name.to_string(),
            kind: SourceSetKind::Extension,
            relative_root: format!("src/{name}"),
            source_format,
            mapping_digest: format!("sha256:{}", "a".repeat(64)),
        }
    }

    fn snapshot(name: &str, digit: char) -> SourceSetSnapshot {
        SourceSetSnapshot {
            source_set: resolved(name, SourceFormat::PlatformXml),
            source_fingerprint: format!("sha256:{}", digit.to_string().repeat(64)),
        }
    }

    #[test]
    fn source_snapshot_keeps_one_analysis_and_sorts_deduplicates_mutations() {
        let mutation_a = snapshot("a", '2');
        let mutation_b = snapshot("b", '3');
        let result = SourceSnapshot::new(
            SourceSetSnapshot {
                source_set: resolved("main", SourceFormat::Edt),
                source_fingerprint: format!("sha256:{}", "1".repeat(64)),
            },
            vec![mutation_b.clone(), mutation_a.clone(), mutation_b],
            format!("sha256:{}", "4".repeat(64)),
            9,
        )
        .unwrap();

        assert_eq!(result.analysis.source_set.source_format, SourceFormat::Edt);
        assert_eq!(result.mutations, [mutation_a, snapshot("b", '3')]);
        assert_eq!(result.workspace_epoch, 9);
    }

    #[test]
    fn source_snapshot_identity_is_permutation_invariant() {
        let analysis = SourceSetSnapshot {
            source_set: resolved("main", SourceFormat::Edt),
            source_fingerprint: format!("sha256:{}", "1".repeat(64)),
        };
        let mutation_a = snapshot("a", '2');
        let mutation_b = snapshot("b", '3');

        let first = SourceSnapshot::new(
            analysis.clone(),
            vec![mutation_b.clone(), mutation_a.clone()],
            format!("sha256:{}", "4".repeat(64)),
            9,
        )
        .unwrap();
        let second = SourceSnapshot::new(
            analysis,
            vec![mutation_a, mutation_b],
            format!("sha256:{}", "4".repeat(64)),
            9,
        )
        .unwrap();

        assert_eq!(first, second);
    }

    #[test]
    fn same_name_and_role_with_conflicting_mapping_identity_is_rejected() {
        let analysis = SourceSetSnapshot {
            source_set: resolved("main", SourceFormat::Edt),
            source_fingerprint: format!("sha256:{}", "1".repeat(64)),
        };
        let first = snapshot("extension", '2');
        let mut variants = Vec::new();
        let mut kind = first.clone();
        kind.source_set.kind = SourceSetKind::Configuration;
        variants.push(kind);
        let mut format = first.clone();
        format.source_set.source_format = SourceFormat::Edt;
        variants.push(format);
        let mut root = first.clone();
        root.source_set.relative_root = "different/extension".to_string();
        variants.push(root);
        let mut mapping = first.clone();
        mapping.source_set.mapping_digest = format!("sha256:{}", "b".repeat(64));
        variants.push(mapping);
        let mut content = first.clone();
        content.source_fingerprint = format!("sha256:{}", "3".repeat(64));
        variants.push(content);

        for conflicting in variants {
            let result = SourceSnapshot::new(
                analysis.clone(),
                vec![first.clone(), conflicting],
                format!("sha256:{}", "4".repeat(64)),
                9,
            );

            assert!(result.is_err());
        }
    }

    #[test]
    fn same_name_with_different_roles_keeps_distinct_identities() {
        let analysis = SourceSetSnapshot {
            source_set: resolved("shared", SourceFormat::Edt),
            source_fingerprint: format!("sha256:{}", "1".repeat(64)),
        };
        let mutation = snapshot("shared", '2');

        let result = SourceSnapshot::new(
            analysis.clone(),
            vec![mutation.clone()],
            format!("sha256:{}", "4".repeat(64)),
            9,
        )
        .unwrap();

        assert_eq!(result.analysis, analysis);
        assert_eq!(result.mutations, [mutation]);
    }
}
