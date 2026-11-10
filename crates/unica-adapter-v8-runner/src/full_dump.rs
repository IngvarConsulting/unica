//! Runner project projection, platform installation roots and the pinned
//! full-dump flag.

use serde_yaml::{Mapping as YamlMapping, Value as YamlValue};
use std::path::PathBuf;
use unica_workspace::full_dump::{
    current_host_platform, HostPlatform, RunnerProjectStore, SerializedDumpProjection,
};

/// The one enumeration of platform installation roots in the project — reused
/// by `documentation_registry`'s installation resolver in
/// `infrastructure::application_ports` so a second such list never appears.
pub fn default_platform_roots() -> Vec<PathBuf> {
    match current_host_platform() {
        HostPlatform::Windows => vec![
            PathBuf::from(r"C:\Program Files\1cv8"),
            PathBuf::from(r"C:\Program Files (x86)\1cv8"),
        ],
        HostPlatform::Linux => vec![
            PathBuf::from("/opt/1cv8/x86_64"),
            PathBuf::from("/opt/1cv8/i386"),
            PathBuf::from("/usr/local/1cv8"),
        ],
        HostPlatform::Other => vec![PathBuf::from("/opt/1cv8")],
    }
}

/// The v8-runner flag that makes `dump` (`pull`) a full dump replacing the
/// source tree; the pinned runner refuses the former `--mode full`.
pub const FULL_DUMP_FLAG: &str = "--force";

fn yaml_mapping(value: &YamlValue) -> Result<&YamlMapping, String> {
    value
        .as_mapping()
        .ok_or_else(|| "expected a YAML mapping at the document root".to_string())
}

fn serialize_projection(
    primary: YamlValue,
    local: Option<YamlValue>,
    canonical: bool,
) -> Result<SerializedDumpProjection, String> {
    if canonical {
        yaml_mapping(&primary)?;
        if let Some(local) = &local {
            yaml_mapping(local)?;
        }
    }
    let local = local
        .as_ref()
        .map(serde_yaml::to_string)
        .transpose()
        .map_err(|e| e.to_string())?
        .map(String::into_bytes);
    let primary = serde_yaml::to_string(&primary)
        .map_err(|e| e.to_string())?
        .into_bytes();
    let guard = serde_yaml::to_string(&YamlValue::Null)
        .map_err(|e| e.to_string())?
        .into_bytes();
    Ok(SerializedDumpProjection::from_serialized(
        primary, local, guard,
    ))
}

pub fn write_projection(
    store: &mut RunnerProjectStore,
    primary: &YamlValue,
    local: Option<&YamlValue>,
) -> Result<(), String> {
    store.write_serialized_projection(&serialize_projection(
        primary.clone(),
        local.cloned(),
        true,
    )?)
}

#[cfg(test)]
mod runner_project_tests;
