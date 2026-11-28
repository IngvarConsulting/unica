//! The working directories and the physical file infobase selected by the project.
use super::super::super::runner::{effective_origin, file_infobase_path};
use super::super::super::v13_workspace_bootstrap::read_yaml_config;
use super::*;
use unica_workspace::paths::{host_path_relative_to, same_host_path};

pub(super) struct RunnerPaths {
    state: [PathBuf; 2],
    file_infobase: Option<PathBuf>,
    sets: Vec<PathBuf>,
}

impl RunnerPaths {
    pub(super) fn read(root: &Path) -> Result<Self, String> {
        let local = read_yaml_config(root, LOCAL_CONFIG_NAME)?;
        let base = read_yaml_config(root, CONFIG_NAME)?;
        let layers = [local.as_ref(), base.as_ref()];
        let configured = |name: &str, default: &str| {
            let path = layers
                .iter()
                .flatten()
                .find_map(|layer| layer.get(name).and_then(serde_yaml::Value::as_str))
                .unwrap_or(default);
            normalize_path_identity(&root.join(path))
        };
        let state = [
            configured("workPath", ".build/v8-runner")?,
            configured("localPath", ".local")?,
        ];
        let origin = effective_origin(root)?;
        let file_infobase = origin
            .as_ref()
            .and_then(|value| value.get("connection"))
            .and_then(serde_yaml::Value::as_str)
            .and_then(file_infobase_path)
            .map(|file| normalize_path_identity(&root.join(file)))
            .transpose()?;
        let sets = layers
            .into_iter()
            .flatten()
            .filter_map(|layer| {
                layer
                    .get("source-set")
                    .and_then(serde_yaml::Value::as_sequence)
            })
            .flatten()
            .filter_map(|set| set.get("path").and_then(serde_yaml::Value::as_str))
            .map(|path| root.join(path))
            .collect();
        Ok(Self {
            state,
            file_infobase,
            sets,
        })
    }

    pub(super) fn protected_directories(&self, root: &Path) -> Result<Vec<PathBuf>, String> {
        let mut paths = vec![root.to_path_buf()];
        paths.extend(self.state.iter().cloned());
        paths.extend(self.file_infobase.iter().cloned());
        for set in &self.sets {
            paths.push(normalize_path_identity(set)?);
        }
        Ok(paths)
    }

    /// Work and local are directories, even when not created yet. Only the
    /// actual database file is excluded; sibling files remain user sources.
    pub(super) fn fingerprint_excludes(&self, root: &Path, set: &Path) -> Vec<PathBuf> {
        let database = self
            .file_infobase
            .as_ref()
            .map(|db| db.join("1Cv8.1CD"))
            .filter(|file| file.is_file())
            .and_then(|file| normalize_path_identity(&file).ok());
        let mut extra = self
            .state
            .iter()
            .filter(|path| !path.is_file())
            .cloned()
            .chain(database)
            .filter(|path| {
                !same_host_path(path, &root.join(CONFIG_NAME))
                    && !same_host_path(path, &root.join(LOCAL_CONFIG_NAME))
            })
            .filter(|path| host_path_relative_to(path, &root.join(GENERATED_DIR_NAME)).is_none())
            .filter_map(|path| host_path_relative_to(&path, set))
            .filter(|relative| {
                !relative.as_os_str().is_empty()
                    && !relative.components().any(|part| part.as_os_str() == ".git")
            })
            .collect::<Vec<_>>();
        extra.sort();
        let mut result = Vec::<PathBuf>::new();
        for path in extra {
            if !result.iter().any(|ancestor| {
                host_path_relative_to(&set.join(&path), &set.join(ancestor)).is_some()
            }) {
                result.push(path);
            }
        }
        result
    }
}
