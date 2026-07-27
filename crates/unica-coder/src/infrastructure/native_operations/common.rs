use crate::domain::workspace::WorkspaceContext;
use crate::infrastructure::native_operations::compile_transaction::CompileTransaction;
use serde_json::{Map, Value};
use std::path::{Path, PathBuf};
use unica_format_core::ports::FormatCompatibilityKind;

pub(crate) fn required_string<'a>(
    args: &'a Map<String, Value>,
    names: &[&str],
    label: &str,
) -> Result<&'a str, String> {
    names
        .iter()
        .find_map(|name| args.get(*name).and_then(Value::as_str))
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| format!("missing required {label} argument"))
}

pub(crate) fn path_arg(args: &Map<String, Value>, names: &[&str]) -> Option<PathBuf> {
    names
        .iter()
        .find_map(|name| args.get(*name).and_then(Value::as_str))
        .filter(|value| !value.trim().is_empty())
        .map(PathBuf::from)
}

pub(crate) fn required_path(
    args: &Map<String, Value>,
    names: &[&str],
    label: &str,
) -> Result<PathBuf, String> {
    path_arg(args, names).ok_or_else(|| format!("missing required {label} argument"))
}

pub(crate) fn bool_arg(args: &Map<String, Value>, names: &[&str]) -> bool {
    names
        .iter()
        .any(|name| args.get(*name).and_then(Value::as_bool).unwrap_or(false))
}

pub(crate) fn int_arg(args: &Map<String, Value>, names: &[&str]) -> Option<i64> {
    names.iter().find_map(|name| {
        args.get(*name).and_then(|value| {
            value
                .as_i64()
                .or_else(|| value.as_u64().and_then(|value| i64::try_from(value).ok()))
        })
    })
}

pub(crate) fn absolutize(path: PathBuf, cwd: &Path) -> PathBuf {
    if path.is_absolute() {
        path
    } else {
        cwd.join(path)
    }
}

/// Binds the adapter-owned format-owner evidence to the BSL-only publication.
/// XML discovery and version interpretation stay behind the adapter ports.
pub(crate) fn guard_active_format_owner(
    transaction: &mut CompileTransaction,
    target: &Path,
    context: &WorkspaceContext,
) -> Result<(), String> {
    let resolution =
        crate::infrastructure::platform_xml_owner::resolve_platform_xml_owners_with_provenance(
            target, context,
        )
        .map_err(|error| error.message)?;
    if resolution
        .owners
        .iter()
        .any(|owner| owner.format.kind() != FormatCompatibilityKind::Supported)
    {
        return Err("Platform XML owner is not authorable by the active adapter".to_string());
    }
    resolution.provenance.bind_to(transaction)
}
