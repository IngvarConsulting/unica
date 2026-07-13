use serde::Serialize;
use serde_yaml::Value as YamlValue;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectSourceMap {
    pub workspace_root: String,
    pub config_path: Option<String>,
    pub source_sets: Vec<ProjectSourceSet>,
    #[serde(skip)]
    pub(crate) source_sets_from_config: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectSourceSet {
    pub name: String,
    pub kind: SourceSetKind,
    pub path: String,
    pub source_format: SourceFormat,
    pub format_evidence: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceSetKind {
    Configuration,
    Extension,
    ExternalProcessor,
    ExternalReport,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceFormat {
    PlatformXml,
    Edt,
    Unknown,
    Invalid,
}

#[derive(Debug, Clone)]
struct ConfigSourceSet {
    name: String,
    kind: SourceSetKind,
    path: String,
    default_format: Option<SourceFormat>,
}

pub fn discover_project_source_map(workspace_root: &Path) -> Result<ProjectSourceMap, String> {
    let config_path = find_project_config(workspace_root)?;
    let (mut source_sets, source_sets_from_config) = if let Some(path) = &config_path {
        let configured = read_config_source_sets(workspace_root, path)?;
        let authoritative = !configured.is_empty();
        (configured, authoritative)
    } else {
        (autodetect_source_sets(workspace_root), false)
    };

    if source_sets.is_empty() {
        source_sets = autodetect_source_sets(workspace_root);
    }

    let project_source_sets = source_sets
        .into_iter()
        .map(|source_set| detect_source_set_format(workspace_root, source_set))
        .collect::<Vec<_>>();

    Ok(ProjectSourceMap {
        workspace_root: workspace_root.display().to_string(),
        config_path: config_path.map(|path| path.display().to_string()),
        source_sets: project_source_sets,
        source_sets_from_config,
    })
}

fn find_project_config(workspace_root: &Path) -> Result<Option<PathBuf>, String> {
    let default = workspace_root.join("v8project.yaml");
    match std::fs::symlink_metadata(&default) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => Err(format!(
            "project config must be a regular non-symlink file: {}",
            default.display()
        )),
        Ok(_) => Ok(Some(default)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(format!(
            "failed to inspect project config {}: {error}",
            default.display()
        )),
    }
}

fn read_config_source_sets(
    workspace_root: &Path,
    config_path: &Path,
) -> Result<Vec<ConfigSourceSet>, String> {
    let text = std::fs::read_to_string(config_path)
        .map_err(|err| format!("failed to read {}: {err}", config_path.display()))?;
    let yaml = serde_yaml::from_str::<YamlValue>(&text)
        .map_err(|err| format!("failed to parse {}: {err}", config_path.display()))?;
    let default_format = yaml_string(&yaml, "format").and_then(source_format_from_config);
    let base_path = yaml_string(&yaml, "basePath").unwrap_or_else(|| ".".to_string());
    let source_set_value = yaml_mapping_get(&yaml, "source-set");
    let mut source_sets = Vec::new();

    match source_set_value {
        Some(YamlValue::Sequence(entries)) => {
            for entry in entries {
                if !entry.is_mapping() {
                    return Err(format!(
                        "{} field `source-set` list entries must be mappings",
                        config_path.display()
                    ));
                }
                source_sets.push(config_source_set_from_yaml(entry, default_format)?);
            }
        }
        Some(YamlValue::Mapping(_)) => {
            return Err(format!(
                "{} field `source-set` must be a list; v8-runner does not accept mapping-form source sets",
                config_path.display()
            ));
        }
        Some(YamlValue::Null) | None => {}
        Some(_) => {
            return Err(format!(
                "{} field `source-set` must be a list",
                config_path.display()
            ));
        }
    }

    for source_set in &mut source_sets {
        source_set.path = normalize_configured_path(workspace_root, &base_path, &source_set.path);
    }

    Ok(source_sets)
}

fn config_source_set_from_yaml(
    entry: &YamlValue,
    default_format: Option<SourceFormat>,
) -> Result<ConfigSourceSet, String> {
    let name = required_yaml_string(entry, "name", "source-set entry")?;
    config_source_set_from_named_yaml(&name, entry, default_format)
}

fn config_source_set_from_named_yaml(
    name: &str,
    entry: &YamlValue,
    default_format: Option<SourceFormat>,
) -> Result<ConfigSourceSet, String> {
    if name.trim().is_empty() || !is_safe_source_set_name(name) {
        return Err(format!(
            "source-set name `{name}` must be one safe path segment"
        ));
    }
    if let Some(entry_name) = optional_strict_yaml_string(entry, "name")? {
        if entry_name != name {
            return Err(format!(
                "source-set mapping key `{name}` does not match entry name `{entry_name}`"
            ));
        }
    }
    let source_type = required_yaml_string(entry, "type", &format!("source-set `{name}`"))?;
    let kind = source_set_kind_from_config(&source_type)?;
    let path = required_yaml_string(entry, "path", &format!("source-set `{name}`"))?;
    Ok(ConfigSourceSet {
        name: name.to_string(),
        kind,
        path,
        default_format,
    })
}

fn is_safe_source_set_name(value: &str) -> bool {
    use std::path::Component;

    if value.is_empty() {
        return false;
    }
    let mut components = Path::new(value).components();
    let single_normal =
        matches!(components.next(), Some(Component::Normal(_))) && components.next().is_none();
    single_normal
        && !value.chars().any(|character| {
            character.is_control()
                || matches!(
                    character,
                    '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*' | '\0'
                )
        })
}

fn optional_strict_yaml_string(value: &YamlValue, key: &str) -> Result<Option<String>, String> {
    let Some(value) = yaml_mapping_get(value, key) else {
        return Ok(None);
    };
    let text = value
        .as_str()
        .filter(|text| !text.trim().is_empty())
        .ok_or_else(|| format!("field `{key}` must be a non-blank string"))?;
    Ok(Some(text.to_string()))
}

fn required_yaml_string(value: &YamlValue, key: &str, context: &str) -> Result<String, String> {
    optional_strict_yaml_string(value, key)?
        .ok_or_else(|| format!("{context} requires non-blank string field `{key}`"))
}

fn normalize_configured_path(workspace_root: &Path, base_path: &str, raw_path: &str) -> String {
    let base = PathBuf::from(base_path);
    let path = PathBuf::from(raw_path);
    let resolved = if path.is_absolute() {
        path
    } else if base.is_absolute() {
        base.join(path)
    } else {
        workspace_root.join(base).join(path)
    };
    path_relative_to(workspace_root, &resolved)
}

fn autodetect_source_sets(workspace_root: &Path) -> Vec<ConfigSourceSet> {
    for path in [".", "src", "src/cf"] {
        let root = workspace_root.join(path);
        if root.join("Configuration.xml").is_file()
            || root.join("Configuration/Configuration.mdo").is_file()
            || root.join("src/Configuration/Configuration.mdo").is_file()
        {
            return vec![ConfigSourceSet {
                name: "main".to_string(),
                kind: SourceSetKind::Configuration,
                path: path.to_string(),
                default_format: None,
            }];
        }
    }
    Vec::new()
}

fn detect_source_set_format(
    workspace_root: &Path,
    source_set: ConfigSourceSet,
) -> ProjectSourceSet {
    let source_root = workspace_root.join(&source_set.path);
    let platform_evidence = platform_xml_evidence(workspace_root, &source_root, source_set.kind);
    let edt_evidence = edt_evidence(workspace_root, &source_root);
    let source_format = match (platform_evidence.is_empty(), edt_evidence.is_empty()) {
        (false, false) => SourceFormat::Invalid,
        (false, true) => SourceFormat::PlatformXml,
        (true, false) => SourceFormat::Edt,
        (true, true) => source_set.default_format.unwrap_or(SourceFormat::Unknown),
    };
    let mut format_evidence = Vec::new();
    format_evidence.extend(platform_evidence);
    format_evidence.extend(edt_evidence);
    if format_evidence.is_empty() {
        if let Some(default_format) = source_set.default_format {
            format_evidence.push(match default_format {
                SourceFormat::PlatformXml => "v8project.yaml:format=DESIGNER".to_string(),
                SourceFormat::Edt => "v8project.yaml:format=EDT".to_string(),
                SourceFormat::Unknown | SourceFormat::Invalid => {
                    "v8project.yaml:format".to_string()
                }
            });
        }
    }

    ProjectSourceSet {
        name: source_set.name,
        kind: source_set.kind,
        path: source_set.path,
        source_format,
        format_evidence,
    }
}

fn platform_xml_evidence(
    workspace_root: &Path,
    source_root: &Path,
    kind: SourceSetKind,
) -> Vec<String> {
    let mut evidence = Vec::new();
    for rel in ["Configuration.xml", "ConfigDumpInfo.xml"] {
        push_existing(&mut evidence, workspace_root, &source_root.join(rel));
    }

    if matches!(
        kind,
        SourceSetKind::ExternalProcessor | SourceSetKind::ExternalReport
    ) {
        if let Ok(entries) = std::fs::read_dir(source_root) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().and_then(|ext| ext.to_str()) == Some("xml") {
                    push_existing(&mut evidence, workspace_root, &path);
                }
            }
        }
    }
    evidence.sort();
    evidence.dedup();
    evidence
}

fn edt_evidence(workspace_root: &Path, source_root: &Path) -> Vec<String> {
    let mut evidence = Vec::new();
    for rel in [
        ".project",
        "DT-INF/PROJECT.PMF",
        "Configuration/Configuration.mdo",
        "src/Configuration/Configuration.mdo",
    ] {
        push_existing(&mut evidence, workspace_root, &source_root.join(rel));
    }
    evidence.sort();
    evidence.dedup();
    evidence
}

fn push_existing(evidence: &mut Vec<String>, workspace_root: &Path, path: &Path) {
    if path.is_file() {
        evidence.push(path_relative_to(workspace_root, path));
    }
}

fn path_relative_to(root: &Path, path: &Path) -> String {
    let path = path
        .strip_prefix(root)
        .unwrap_or(path)
        .display()
        .to_string();
    #[cfg(windows)]
    {
        path.replace('\\', "/")
    }
    #[cfg(not(windows))]
    {
        path
    }
}

fn source_set_kind_from_config(raw: &str) -> Result<SourceSetKind, String> {
    match raw.to_ascii_uppercase().as_str() {
        "CONFIGURATION" => Ok(SourceSetKind::Configuration),
        "EXTENSION" => Ok(SourceSetKind::Extension),
        "EXTERNAL_DATA_PROCESSORS" => Ok(SourceSetKind::ExternalProcessor),
        "EXTERNAL_REPORTS" => Ok(SourceSetKind::ExternalReport),
        other => Err(format!("unsupported source-set type `{other}`")),
    }
}

fn source_format_from_config(raw: String) -> Option<SourceFormat> {
    match raw.to_ascii_uppercase().as_str() {
        "DESIGNER" | "PLATFORM_XML" | "XML" => Some(SourceFormat::PlatformXml),
        "EDT" => Some(SourceFormat::Edt),
        _ => None,
    }
}

fn yaml_string(value: &YamlValue, key: &str) -> Option<String> {
    yaml_mapping_get(value, key).and_then(|value| match value {
        YamlValue::String(text) => Some(text.clone()),
        YamlValue::Number(number) => Some(number.to_string()),
        _ => None,
    })
}

fn yaml_mapping_get<'a>(value: &'a YamlValue, key: &str) -> Option<&'a YamlValue> {
    let mapping = value.as_mapping()?;
    mapping.get(YamlValue::String(key.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::{OsStr, OsString};
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn detects_edt_configuration_and_platform_external_processor_source_sets() {
        let root = temp_workspace("unica-source-map-multi");
        write(
            &root.join("v8project.yaml"),
            r#"
format: EDT
source-set:
  - name: main
    type: CONFIGURATION
    path: src
  - name: external-processors
    type: EXTERNAL_DATA_PROCESSORS
    path: epf
"#,
        );
        write(&root.join("src/.project"), "<projectDescription/>");
        write(
            &root.join("src/Configuration/Configuration.mdo"),
            "<mdclass:Configuration/>",
        );
        write(
            &root.join("epf/PriceLoader.xml"),
            "<MetaDataObject><ExternalDataProcessor/></MetaDataObject>",
        );

        let map = discover_project_source_map(&root).unwrap();

        assert_eq!(map.source_sets.len(), 2);
        assert_source_set(
            &map,
            "main",
            SourceSetKind::Configuration,
            SourceFormat::Edt,
            &["src/.project", "src/Configuration/Configuration.mdo"],
        );
        assert_source_set(
            &map,
            "external-processors",
            SourceSetKind::ExternalProcessor,
            SourceFormat::PlatformXml,
            &["epf/PriceLoader.xml"],
        );

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn detects_single_platform_configuration_source_set() {
        let root = temp_workspace("unica-source-map-platform");
        write(
            &root.join("v8project.yaml"),
            r#"
format: DESIGNER
source-set:
  - name: main
    type: CONFIGURATION
    path: src
"#,
        );
        write(&root.join("src/Configuration.xml"), "<MetaDataObject/>");
        write(&root.join("src/ConfigDumpInfo.xml"), "<ConfigDumpInfo/>");

        let map = discover_project_source_map(&root).unwrap();

        assert_source_set(
            &map,
            "main",
            SourceSetKind::Configuration,
            SourceFormat::PlatformXml,
            &["src/Configuration.xml", "src/ConfigDumpInfo.xml"],
        );

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn ignores_legacy_v8tr_config_environment_override() {
        let root = temp_workspace("unica-source-map-ignore-v8tr-config");
        write(
            &root.join("v8project.yaml"),
            r#"
format: DESIGNER
source-set:
  - name: main
    type: CONFIGURATION
    path: src
"#,
        );
        write(
            &root.join("custom.yaml"),
            r#"
format: DESIGNER
source-set:
  - name: env
    type: CONFIGURATION
    path: env-src
"#,
        );
        write(&root.join("src/Configuration.xml"), "<MetaDataObject/>");
        write(&root.join("env-src/Configuration.xml"), "<MetaDataObject/>");
        let _guard = EnvVarGuard::set("V8TR_CONFIG", root.join("custom.yaml"));

        let map = discover_project_source_map(&root).unwrap();

        assert_source_set(
            &map,
            "main",
            SourceSetKind::Configuration,
            SourceFormat::PlatformXml,
            &["src/Configuration.xml"],
        );
        assert!(
            map.source_sets
                .iter()
                .all(|source_set| source_set.name != "env"),
            "legacy V8TR_CONFIG source set must be ignored: {map:?}"
        );

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn detects_single_edt_configuration_source_set() {
        let root = temp_workspace("unica-source-map-edt");
        write(
            &root.join("v8project.yaml"),
            r#"
format: EDT
source-set:
  - name: main
    type: CONFIGURATION
    path: src
"#,
        );
        write(&root.join("src/.project"), "<projectDescription/>");
        write(
            &root.join("src/Configuration/Configuration.mdo"),
            "<mdclass:Configuration/>",
        );

        let map = discover_project_source_map(&root).unwrap();

        assert_source_set(
            &map,
            "main",
            SourceSetKind::Configuration,
            SourceFormat::Edt,
            &["src/.project", "src/Configuration/Configuration.mdo"],
        );

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn conflicting_markers_inside_one_source_set_are_invalid_not_mixed() {
        let root = temp_workspace("unica-source-map-invalid");
        write(
            &root.join("v8project.yaml"),
            r#"
source-set:
  - name: main
    type: CONFIGURATION
    path: src
"#,
        );
        write(&root.join("src/Configuration.xml"), "<MetaDataObject/>");
        write(
            &root.join("src/Configuration/Configuration.mdo"),
            "<mdclass:Configuration/>",
        );

        let map = discover_project_source_map(&root).unwrap();

        assert_source_set(
            &map,
            "main",
            SourceSetKind::Configuration,
            SourceFormat::Invalid,
            &[
                "src/Configuration.xml",
                "src/Configuration/Configuration.mdo",
            ],
        );

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn autodetected_sets_are_not_marked_as_runtime_configured() {
        for (name, config) in [
            ("missing-config", None),
            ("empty-config", Some("format: DESIGNER\n")),
            (
                "null-source-set",
                Some("format: DESIGNER\nsource-set: null\n"),
            ),
        ] {
            let root = temp_workspace(&format!("unica-source-map-{name}"));
            write(&root.join("src/Configuration.xml"), "<MetaDataObject/>");
            if let Some(config) = config {
                write(&root.join("v8project.yaml"), config);
            }

            let map = discover_project_source_map(&root).unwrap();

            assert_eq!(map.source_sets.len(), 1);
            assert!(!map.source_sets_from_config);
            fs::remove_dir_all(root).unwrap();
        }
    }

    #[test]
    fn malformed_source_set_shapes_fail_instead_of_becoming_authoritative_defaults() {
        for (name, source_set) in [
            ("null-list-entry", "source-set: [null]\n"),
            ("scalar-list-entry", "source-set: [main]\n"),
            (
                "numeric-map-key",
                "source-set:\n  1:\n    type: CONFIGURATION\n    path: src\n",
            ),
            ("scalar-map-value", "source-set:\n  main: 1\n"),
            (
                "numeric-name",
                "source-set:\n  - name: 1\n    type: CONFIGURATION\n    path: src\n",
            ),
            (
                "missing-path",
                "source-set:\n  - name: main\n    type: CONFIGURATION\n",
            ),
            (
                "missing-type",
                "source-set:\n  - name: main\n    path: src\n",
            ),
            (
                "unsafe-name",
                "source-set:\n  - name: bad/name\n    type: CONFIGURATION\n    path: src\n",
            ),
            (
                "dot-name",
                "source-set:\n  - name: .\n    type: CONFIGURATION\n    path: src\n",
            ),
        ] {
            let root = temp_workspace(&format!("unica-source-map-malformed-{name}"));
            write(&root.join("src/Configuration.xml"), "<MetaDataObject/>");
            write(
                &root.join("v8project.yaml"),
                &format!("format: DESIGNER\n{source_set}"),
            );

            assert!(
                discover_project_source_map(&root).is_err(),
                "malformed shape `{name}` was accepted"
            );
            fs::remove_dir_all(root).unwrap();
        }
    }

    fn assert_source_set(
        map: &ProjectSourceMap,
        name: &str,
        kind: SourceSetKind,
        source_format: SourceFormat,
        expected_evidence: &[&str],
    ) {
        let source_set = map
            .source_sets
            .iter()
            .find(|source_set| source_set.name == name)
            .unwrap_or_else(|| panic!("source set {name} not found in {map:?}"));
        assert_eq!(source_set.kind, kind);
        assert_eq!(source_set.source_format, source_format);
        for evidence in expected_evidence {
            assert!(
                source_set
                    .format_evidence
                    .iter()
                    .any(|actual| actual == evidence),
                "missing evidence {evidence} in {source_set:?}"
            );
        }
    }

    fn temp_workspace(prefix: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("{prefix}-{}-{nanos}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        root
    }

    fn write(path: &Path, text: &str) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, text).unwrap();
    }

    struct EnvVarGuard {
        key: &'static str,
        previous: Option<OsString>,
    }

    impl EnvVarGuard {
        fn set(key: &'static str, value: impl AsRef<OsStr>) -> Self {
            let previous = std::env::var_os(key);
            std::env::set_var(key, value);
            Self { key, previous }
        }
    }

    impl Drop for EnvVarGuard {
        fn drop(&mut self) {
            if let Some(previous) = &self.previous {
                std::env::set_var(self.key, previous);
            } else {
                std::env::remove_var(self.key);
            }
        }
    }
}
