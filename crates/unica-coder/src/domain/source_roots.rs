use crate::domain::project_sources::{ProjectSourceSet, SourceSetKind};
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedSourceRoot {
    pub source_set: Option<String>,
    pub path: PathBuf,
}

pub fn select_default_source_set(
    source_sets: &[ProjectSourceSet],
) -> Result<&ProjectSourceSet, String> {
    if let Some(main) = source_sets
        .iter()
        .find(|source_set| source_set.name == "main")
    {
        return Ok(main);
    }

    let configurations = source_sets
        .iter()
        .filter(|source_set| source_set.kind == SourceSetKind::Configuration)
        .collect::<Vec<_>>();

    match configurations.as_slice() {
        [source_set] => Ok(source_set),
        [] => {
            Err("sourceDir is required because no configuration source set was found".to_string())
        }
        _ => Err(format!(
            "sourceDir is required because configuration source sets are ambiguous: {}",
            configurations
                .iter()
                .map(|source_set| source_set.name.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::select_default_source_set;
    use crate::domain::project_sources::{ProjectSourceSet, SourceFormat, SourceSetKind};

    fn source_set(name: &str, kind: SourceSetKind) -> ProjectSourceSet {
        ProjectSourceSet {
            name: name.to_string(),
            kind,
            path: name.to_string(),
            source_format: SourceFormat::Unknown,
            format_evidence: Vec::new(),
        }
    }

    #[test]
    fn main_source_set_wins_without_io() {
        let source_sets = vec![
            source_set("app", SourceSetKind::Configuration),
            source_set("main", SourceSetKind::Extension),
        ];

        assert_eq!(
            select_default_source_set(&source_sets).unwrap().name,
            "main"
        );
    }

    #[test]
    fn ambiguous_configuration_error_is_stable() {
        let source_sets = vec![
            source_set("app", SourceSetKind::Configuration),
            source_set("tests", SourceSetKind::Configuration),
        ];

        assert_eq!(
            select_default_source_set(&source_sets).unwrap_err(),
            "sourceDir is required because configuration source sets are ambiguous: app, tests"
        );
    }
}
