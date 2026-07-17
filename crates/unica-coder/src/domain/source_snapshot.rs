use super::project_sources::{SourceFormat, SourceSetKind};
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ResolvedSourceSet {
    pub(crate) name: String,
    pub(crate) kind: SourceSetKind,
    pub(crate) relative_root: String,
    pub(crate) source_format: SourceFormat,
}

impl ResolvedSourceSet {
    pub(crate) fn validate(&self) -> Result<(), String> {
        stable_component(&self.name, "source-set name", 1024)?;
        contained_relative_path(&self.relative_root)?;
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
            left.source_set
                .name
                .cmp(&right.source_set.name)
                .then_with(|| left.source_fingerprint.cmp(&right.source_fingerprint))
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
        let mut fingerprints = BTreeMap::new();
        fingerprints.insert(
            self.analysis.source_set.name.as_str(),
            self.analysis.source_fingerprint.as_str(),
        );
        let mut previous = None;
        for mutation in &self.mutations {
            mutation.validate()?;
            if let Some(previous_snapshot) = previous {
                if snapshot_key(previous_snapshot) > snapshot_key(mutation) {
                    return Err("mutation source snapshots must be canonically sorted".to_string());
                }
            }
            if let Some(existing) = fingerprints.insert(
                mutation.source_set.name.as_str(),
                mutation.source_fingerprint.as_str(),
            ) {
                if existing != mutation.source_fingerprint {
                    return Err("one source-set cannot have conflicting fingerprints".to_string());
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

fn snapshot_key(snapshot: &SourceSetSnapshot) -> (&str, &str) {
    (
        snapshot.source_set.name.as_str(),
        snapshot.source_fingerprint.as_str(),
    )
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
}
