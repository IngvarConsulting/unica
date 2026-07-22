use crate::domain::project_sources::{
    config_dump_info_xml_kind, ConfigDumpInfoXmlKind, ProjectSourceMap, ProjectSourceSet,
    SourceFormat, SourceSetKind,
};
use crate::domain::source_roots::select_default_source_set;
use crate::infrastructure::platform::filesystem::host_path_text;
use crate::infrastructure::source_roots::{
    normalize_contained_source_root, normalize_path_identity,
};
use serde_yaml::Value as YamlValue;
use std::io::Read;
use std::path::{Path, PathBuf};

const MAX_RESERVED_EXTERNAL_DESCRIPTOR_BYTES: u64 = 8 * 1024 * 1024;

#[derive(Debug, Clone)]
struct ConfigSourceSet {
    name: String,
    kind: SourceSetKind,
    path: String,
    default_format: Option<SourceFormat>,
    discovery_path_is_safe: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ProjectSourceDeclaration {
    pub name: String,
    pub kind: SourceSetKind,
    pub path: String,
    pub discovery_path_is_safe: bool,
}

pub(crate) fn discover_project_source_declarations(
    workspace_root: &Path,
) -> Result<Vec<ProjectSourceDeclaration>, String> {
    let Some(config_path) = find_project_config(workspace_root) else {
        return Ok(Vec::new());
    };
    let (source_sets, _configured_format_raw) =
        read_config_source_sets(workspace_root, &config_path)?;
    Ok(source_sets
        .into_iter()
        .map(|source_set| ProjectSourceDeclaration {
            name: source_set.name,
            kind: source_set.kind,
            path: source_set.path,
            discovery_path_is_safe: source_set.discovery_path_is_safe,
        })
        .collect())
}

pub(crate) fn discover_project_source_map(
    workspace_root: &Path,
) -> Result<ProjectSourceMap, String> {
    let config_path = find_project_config(workspace_root);
    let (mut source_sets, configured_format_raw) = if let Some(path) = &config_path {
        read_config_source_sets(workspace_root, path)?
    } else {
        (autodetect_source_sets(workspace_root), None)
    };

    if source_sets.is_empty() {
        source_sets = autodetect_source_sets(workspace_root);
    }

    let project_source_sets = source_sets
        .into_iter()
        .map(|source_set| detect_source_set_format(workspace_root, source_set))
        .collect::<Vec<_>>();
    let (effective_source_set, effective_source_root, source_selection_error) =
        match select_default_source_set(&project_source_sets) {
            Ok(source_set) => {
                match normalize_contained_source_root(workspace_root, &source_set.path) {
                    Ok(root) => (
                        Some(source_set.name.clone()),
                        Some(root.display().to_string()),
                        None,
                    ),
                    Err(error) => (None, None, Some(format!("invalid_source_root: {error}"))),
                }
            }
            Err(error) => (None, None, Some(format!("invalid_source_root: {error}"))),
        };

    Ok(ProjectSourceMap {
        workspace_root: workspace_root.display().to_string(),
        config_path: config_path.map(|path| path.display().to_string()),
        source_sets: project_source_sets,
        effective_source_set,
        effective_source_root,
        source_selection_error,
        configured_format_raw,
    })
}

fn find_project_config(workspace_root: &Path) -> Option<PathBuf> {
    let default = workspace_root.join("v8project.yaml");
    default.is_file().then_some(default)
}

fn read_config_source_sets(
    workspace_root: &Path,
    config_path: &Path,
) -> Result<(Vec<ConfigSourceSet>, Option<String>), String> {
    let text = std::fs::read_to_string(config_path)
        .map_err(|err| format!("failed to read {}: {err}", config_path.display()))?;
    let yaml = serde_yaml::from_str::<YamlValue>(&text)
        .map_err(|err| format!("failed to parse {}: {err}", config_path.display()))?;
    let configured_format_raw = match yaml_mapping_get(&yaml, "format") {
        None => None,
        Some(YamlValue::String(value)) => Some(value.clone()),
        Some(_) => {
            return Err(format!(
                "{} field `format` must be a string",
                config_path.display()
            ));
        }
    };
    let default_format = configured_format_raw
        .clone()
        .and_then(source_format_from_config);
    let base_path = yaml_string(&yaml, "basePath").unwrap_or_else(|| ".".to_string());
    let source_set_value = yaml_mapping_get(&yaml, "source-set");
    let mut source_sets = Vec::new();

    match source_set_value {
        Some(YamlValue::Sequence(entries)) => {
            for entry in entries {
                source_sets.push(config_source_set_from_yaml(entry, default_format)?);
            }
        }
        Some(YamlValue::Mapping(entries)) => {
            for (key, entry) in entries {
                let name = key.as_str().unwrap_or("main");
                source_sets.push(config_source_set_from_named_yaml(
                    name,
                    entry,
                    default_format,
                )?);
            }
        }
        Some(YamlValue::Null) | None => {}
        Some(_) => {
            return Err(format!(
                "{} field `source-set` must be a list or mapping",
                config_path.display()
            ));
        }
    }

    for source_set in &mut source_sets {
        source_set.discovery_path_is_safe &= discovery_path_is_safe(&base_path);
        source_set.path = normalize_configured_path(workspace_root, &base_path, &source_set.path);
    }

    Ok((source_sets, configured_format_raw))
}

fn config_source_set_from_yaml(
    entry: &YamlValue,
    default_format: Option<SourceFormat>,
) -> Result<ConfigSourceSet, String> {
    let name = yaml_string(entry, "name").unwrap_or_else(|| "main".to_string());
    config_source_set_from_named_yaml(&name, entry, default_format)
}

fn config_source_set_from_named_yaml(
    name: &str,
    entry: &YamlValue,
    default_format: Option<SourceFormat>,
) -> Result<ConfigSourceSet, String> {
    let source_type = yaml_string(entry, "type")
        .or_else(|| yaml_string(entry, "purpose"))
        .unwrap_or_else(|| "CONFIGURATION".to_string());
    let kind = source_set_kind_from_config(&source_type)?;
    let path = yaml_string(entry, "path").unwrap_or_else(|| ".".to_string());
    let discovery_path_is_safe = discovery_path_is_safe(&path);
    Ok(ConfigSourceSet {
        name: name.to_string(),
        kind,
        path,
        default_format,
        discovery_path_is_safe,
    })
}

fn discovery_path_is_safe(raw: &str) -> bool {
    let path = Path::new(raw);
    !path.is_absolute()
        && !path.components().any(|component| {
            matches!(
                component,
                std::path::Component::ParentDir
                    | std::path::Component::RootDir
                    | std::path::Component::Prefix(_)
            )
        })
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
                discovery_path_is_safe: true,
            }];
        }
    }
    Vec::new()
}

fn detect_source_set_format(
    workspace_root: &Path,
    source_set: ConfigSourceSet,
) -> ProjectSourceSet {
    let normalized_workspace_root = match normalize_path_identity(workspace_root) {
        Ok(root) => root,
        Err(_error) => {
            return ProjectSourceSet {
                name: source_set.name,
                kind: source_set.kind,
                path: source_set.path,
                source_format: SourceFormat::Invalid,
                format_evidence: Vec::new(),
            };
        }
    };
    let source_root = match normalize_contained_source_root(workspace_root, &source_set.path) {
        Ok(source_root) => source_root,
        Err(_error) => {
            return ProjectSourceSet {
                name: source_set.name,
                kind: source_set.kind,
                path: source_set.path,
                source_format: SourceFormat::Invalid,
                format_evidence: Vec::new(),
            };
        }
    };
    let platform_evidence =
        platform_xml_evidence(&normalized_workspace_root, &source_root, source_set.kind);
    let edt_evidence = edt_evidence(&normalized_workspace_root, &source_root);
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
    push_existing(
        &mut evidence,
        workspace_root,
        &source_root.join("Configuration.xml"),
    );

    if matches!(
        kind,
        SourceSetKind::ExternalProcessor | SourceSetKind::ExternalReport
    ) {
        if let Ok(entries) = std::fs::read_dir(source_root) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().and_then(|ext| ext.to_str()) == Some("xml")
                    && !is_config_dump_info_sidecar(&path, kind)
                {
                    push_existing(&mut evidence, workspace_root, &path);
                }
            }
        }
    }
    evidence.sort();
    evidence.dedup();
    evidence
}

fn is_config_dump_info_sidecar(path: &Path, kind: SourceSetKind) -> bool {
    if !has_config_dump_info_filename(path) {
        return false;
    }
    !matches!(
        (config_dump_info_xml_file_kind(path), kind),
        (
            ConfigDumpInfoXmlKind::ExternalProcessor,
            SourceSetKind::ExternalProcessor
        ) | (
            ConfigDumpInfoXmlKind::ExternalReport,
            SourceSetKind::ExternalReport
        )
    )
}

fn config_dump_info_xml_file_kind(path: &Path) -> ConfigDumpInfoXmlKind {
    if !has_config_dump_info_filename(path) {
        return ConfigDumpInfoXmlKind::Other;
    }
    let Ok(link_metadata) = std::fs::symlink_metadata(path) else {
        return ConfigDumpInfoXmlKind::Other;
    };
    if link_metadata.file_type().is_symlink() || !link_metadata.file_type().is_file() {
        return ConfigDumpInfoXmlKind::Other;
    }
    let Ok(mut file) = std::fs::File::open(path) else {
        return ConfigDumpInfoXmlKind::Other;
    };
    let Ok(metadata) = file.metadata() else {
        return ConfigDumpInfoXmlKind::Other;
    };
    if !metadata.file_type().is_file() || metadata.len() > MAX_RESERVED_EXTERNAL_DESCRIPTOR_BYTES {
        return ConfigDumpInfoXmlKind::Other;
    }
    let mut bytes = Vec::new();
    if (&mut file)
        .take(MAX_RESERVED_EXTERNAL_DESCRIPTOR_BYTES + 1)
        .read_to_end(&mut bytes)
        .is_err()
        || bytes.len() as u64 > MAX_RESERVED_EXTERNAL_DESCRIPTOR_BYTES
    {
        return ConfigDumpInfoXmlKind::Other;
    }
    config_dump_info_xml_kind(&bytes)
}

fn has_config_dump_info_filename(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.eq_ignore_ascii_case("ConfigDumpInfo.xml"))
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
    host_path_text(
        path.strip_prefix(root)
            .unwrap_or(path)
            .display()
            .to_string(),
    )
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
    use crate::infrastructure::source_roots::resolve_source_root;
    use crate::infrastructure::workspace::discover_workspace;
    use std::ffi::{OsStr, OsString};
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    static TEMP_WORKSPACE_NONCE: AtomicU64 = AtomicU64::new(0);

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
        write(&root.join("epf/ConfigDumpInfo.xml"), "<ConfigDumpInfo/>");

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
    fn escaping_configured_source_is_not_probed_for_format_evidence() {
        let root = temp_workspace("unica-source-map-contained-probe");
        let outside = temp_workspace("unica-source-map-outside-probe");
        write(
            &root.join("v8project.yaml"),
            &format!(
                "source-set:\n  - name: escaped\n    type: CONFIGURATION\n    path: {}\n",
                outside.display()
            ),
        );
        write(
            &outside.join("Configuration.xml"),
            "<MetaDataObject><Configuration/></MetaDataObject>",
        );

        let map = discover_project_source_map(&root).unwrap();

        assert_source_set(
            &map,
            "escaped",
            SourceSetKind::Configuration,
            SourceFormat::Invalid,
            &[],
        );
        assert!(map
            .source_selection_error
            .as_deref()
            .is_some_and(|error| error.contains("workspace")));
        fs::remove_dir_all(root).unwrap();
        fs::remove_dir_all(outside).unwrap();
    }

    #[test]
    fn declaration_only_source_reader_does_not_classify_any_source_root() {
        let root = temp_workspace("unica-source-declarations-only");
        write(
            &root.join("v8project.yaml"),
            "source-set:\n  - name: app\n    type: CONFIGURATION\n    path: src\n  - name: external\n    type: EXTERNAL_DATA_PROCESSORS\n    path: epf\n",
        );
        fs::create_dir_all(root.join("src")).unwrap();
        fs::create_dir_all(root.join("epf")).unwrap();

        let declarations = discover_project_source_declarations(&root).unwrap();

        assert_eq!(declarations.len(), 2);
        assert_eq!(declarations[0].name, "app");
        assert_eq!(declarations[0].kind, SourceSetKind::Configuration);
        assert_eq!(declarations[0].path, "src");
        assert_eq!(declarations[1].name, "external");
        assert_eq!(declarations[1].kind, SourceSetKind::ExternalProcessor);
        assert_eq!(declarations[1].path, "epf");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn config_dump_info_alone_is_not_external_source_format_evidence() {
        let root = temp_workspace("unica-source-map-external-cdfi-runtime-state");
        write(
            &root.join("v8project.yaml"),
            r#"
source-set:
  - name: external-processors
    type: EXTERNAL_DATA_PROCESSORS
    path: epf
  - name: external-reports
    type: EXTERNAL_REPORTS
    path: erf
"#,
        );
        write(&root.join("epf/ConfigDumpInfo.xml"), "<ConfigDumpInfo/>");
        write(&root.join("erf/configdumpinfo.xml"), "<ConfigDumpInfo/>");

        let map = discover_project_source_map(&root).unwrap();

        assert_source_set(
            &map,
            "external-processors",
            SourceSetKind::ExternalProcessor,
            SourceFormat::Unknown,
            &[],
        );
        assert_source_set(
            &map,
            "external-reports",
            SourceSetKind::ExternalReport,
            SourceFormat::Unknown,
            &[],
        );

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn external_object_named_config_dump_info_remains_platform_xml_evidence() {
        let root = temp_workspace("unica-source-map-external-object-named-cdfi");
        write(
            &root.join("v8project.yaml"),
            r#"
source-set:
  - name: external-processors
    type: EXTERNAL_DATA_PROCESSORS
    path: epf
"#,
        );
        write(
            &root.join("epf/ConfigDumpInfo.xml"),
            "<MetaDataObject><ExternalDataProcessor/></MetaDataObject>",
        );

        let map = discover_project_source_map(&root).unwrap();

        assert_source_set(
            &map,
            "external-processors",
            SourceSetKind::ExternalProcessor,
            SourceFormat::PlatformXml,
            &["epf/ConfigDumpInfo.xml"],
        );

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn nested_external_tag_does_not_make_config_dump_info_source_evidence() {
        let root = temp_workspace("unica-source-map-nested-external-tag-cdfi");
        write(
            &root.join("v8project.yaml"),
            r#"
source-set:
  - name: external-processors
    type: EXTERNAL_DATA_PROCESSORS
    path: epf
"#,
        );
        write(
            &root.join("epf/ConfigDumpInfo.xml"),
            "<MetaDataObject><Properties><ExternalDataProcessor/></Properties></MetaDataObject>",
        );

        let map = discover_project_source_map(&root).unwrap();

        assert_source_set(
            &map,
            "external-processors",
            SourceSetKind::ExternalProcessor,
            SourceFormat::Unknown,
            &[],
        );

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn malformed_config_dump_info_is_not_external_source_format_evidence() {
        let root = temp_workspace("unica-source-map-malformed-external-cdfi");
        write(
            &root.join("v8project.yaml"),
            r#"
source-set:
  - name: external-reports
    type: EXTERNAL_REPORTS
    path: erf
"#,
        );
        write(
            &root.join("erf/ConfigDumpInfo.xml"),
            "<<<<<<< ours\n<ConfigDumpInfo/>\n=======\n<ConfigDumpInfo/>\n>>>>>>> theirs",
        );

        let map = discover_project_source_map(&root).unwrap();

        assert_source_set(
            &map,
            "external-reports",
            SourceSetKind::ExternalReport,
            SourceFormat::Unknown,
            &[],
        );

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn symlinked_config_dump_info_is_not_external_source_format_evidence() {
        let root = temp_workspace("unica-source-map-symlinked-external-cdfi");
        write(
            &root.join("v8project.yaml"),
            r#"
source-set:
  - name: external-processors
    type: EXTERNAL_DATA_PROCESSORS
    path: epf
"#,
        );
        write(
            &root.join("outside.xml"),
            "<MetaDataObject><ExternalDataProcessor/></MetaDataObject>",
        );
        fs::create_dir_all(root.join("epf")).unwrap();
        let Some(symlink_result) =
            crate::infrastructure::platform::filesystem::create_file_symlink_for_test(
                root.join("outside.xml"),
                root.join("epf/ConfigDumpInfo.xml"),
            )
        else {
            return;
        };
        symlink_result.unwrap();

        let map = discover_project_source_map(&root).unwrap();

        assert_source_set(
            &map,
            "external-processors",
            SourceSetKind::ExternalProcessor,
            SourceFormat::Unknown,
            &[],
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
            &["src/Configuration.xml"],
        );

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn config_dump_info_is_not_platform_source_format_evidence() {
        let root = temp_workspace("unica-source-map-cdfi-runtime-state");
        write(
            &root.join("v8project.yaml"),
            r#"
source-set:
  - name: main
    type: CONFIGURATION
    path: src
"#,
        );
        write(&root.join("src/ConfigDumpInfo.xml"), "<ConfigDumpInfo/>");

        let map = discover_project_source_map(&root).unwrap();

        assert_source_set(
            &map,
            "main",
            SourceSetKind::Configuration,
            SourceFormat::Unknown,
            &[],
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
    fn effective_source_root_rejects_relative_workspace_escape() {
        let root = temp_workspace("unica-source-map-relative-escape");
        write(
            &root.join("v8project.yaml"),
            "source-set:\n  - name: main\n    type: CONFIGURATION\n    path: ../outside\n",
        );

        let map = discover_project_source_map(&root).unwrap();

        assert!(map.effective_source_set.is_none());
        assert!(map.effective_source_root.is_none());
        assert!(map
            .source_selection_error
            .as_deref()
            .is_some_and(
                |error| error.starts_with("invalid_source_root:") && error.contains("workspace")
            ));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn effective_source_root_rejects_absolute_workspace_escape() {
        let root = temp_workspace("unica-source-map-absolute-escape");
        let outside = temp_workspace("unica-source-map-outside");
        write(
            &root.join("v8project.yaml"),
            &format!(
                "source-set:\n  - name: main\n    type: CONFIGURATION\n    path: {}\n",
                outside.display()
            ),
        );

        let map = discover_project_source_map(&root).unwrap();

        assert!(map.effective_source_root.is_none());
        assert!(map
            .source_selection_error
            .as_deref()
            .is_some_and(|error| error.starts_with("invalid_source_root:")));
        fs::remove_dir_all(root).unwrap();
        fs::remove_dir_all(outside).unwrap();
    }

    #[test]
    fn effective_source_root_uses_resolver_path_identity() {
        let root = temp_workspace("unica-source-map-normalized");
        write(
            &root.join("v8project.yaml"),
            "source-set:\n  - name: main\n    type: CONFIGURATION\n    path: src/../src/cf\n",
        );
        fs::create_dir_all(root.join("src/cf")).unwrap();
        let context = discover_workspace(Some(root.clone())).unwrap();

        let map = discover_project_source_map(&root).unwrap();
        let resolved = resolve_source_root(&context, None).unwrap();

        assert_eq!(map.effective_source_set.as_deref(), Some("main"));
        assert_eq!(
            map.effective_source_root.as_deref(),
            Some(resolved.path.to_string_lossy().as_ref())
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn selection_errors_use_the_stable_invalid_source_root_prefix() {
        let ambiguous = temp_workspace("unica-source-map-ambiguous-prefix");
        write(&ambiguous.join("v8project.yaml"), "source-set:\n  - name: app\n    type: CONFIGURATION\n    path: app\n  - name: tests\n    type: CONFIGURATION\n    path: tests\n");
        let map = discover_project_source_map(&ambiguous).unwrap();
        assert_eq!(map.source_selection_error.as_deref(), Some("invalid_source_root: sourceDir is required because configuration source sets are ambiguous: app, tests"));

        let missing = temp_workspace("unica-source-map-missing-prefix");
        let map = discover_project_source_map(&missing).unwrap();
        assert_eq!(map.source_selection_error.as_deref(), Some("invalid_source_root: sourceDir is required because no configuration source set was found"));
        fs::remove_dir_all(ambiguous).unwrap();
        fs::remove_dir_all(missing).unwrap();
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
        assert_eq!(
            source_set.format_evidence,
            expected_evidence
                .iter()
                .map(|evidence| (*evidence).to_string())
                .collect::<Vec<_>>()
        );
    }

    fn temp_workspace(prefix: &str) -> PathBuf {
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let nonce = TEMP_WORKSPACE_NONCE.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "{prefix}-{}-{timestamp}-{nonce}",
            std::process::id()
        ));
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
