#![allow(dead_code, unused_imports)]
use crate::infrastructure::interface_composition::*;
use crate::infrastructure::subsystem_composition::*;
use unica_metadata::interface::*;
use unica_metadata::subsystem::*;
use unica_metadata_designer::interface::*;
use unica_metadata_designer::subsystem::*;
use unica_source_designer::read::file_stem_string;

use crate::application::operation_descriptors::{CFE_VALIDATE_PATH, CF_PATH, RIGHTS_PATH};
use crate::application::SupportGuardRequirement;
use crate::infrastructure::form_composition::*;
use crate::infrastructure::native_operations::logical_selector::{
    logical_selection, physical_selection, typed_reader_metadata_target, AttachedResource,
    ResolvedReadTarget,
};
use crate::infrastructure::platform_xml_source_targets::{
    resolve_platform_xml_target, revalidate_platform_xml_target, ClosedPlatformXmlTarget,
    TargetKindPolicy,
};
use roxmltree::Document;
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use unica_address::address::QualifiedAddress;
use unica_address::source_target::{
    MetadataAddress, ResolvedTarget, SourceTarget, SourceTargetError, SourceTargetErrorCode,
    TargetKind, PLATFORM_XML_8_3_27_FORMAT_2_20,
};
use unica_execution_contract::adapter_outcome::AdapterOutcome;
use unica_forms::facts::*;
use unica_forms_designer::codec::*;
use unica_metadata::support_state::{SupportReadError, SupportReadErrorCode};
use unica_metadata_designer::support::{
    parse_support_state_compat_bytes, parse_support_state_strict_bytes,
    support_root_uuid_from_bytes, SupportState,
};
use unica_source_contract::workspace::WorkspaceContext;
use unica_source_designer::format_profile::{
    classify_root_version, ExportFormatVersion, FormatCompatibility, ACTIVE_FORMAT_PROFILE,
};
use unica_workspace::paths::metadata_is_link_or_reparse_point;
use unica_workspace::paths::normalize_path_identity;

use super::{cf::*, cfe::*, dcs::*, meta::*, role::*};
use unica_workspace::compile_transaction::CompileTransaction;
use unica_workspace::single_file_publisher::{publish, PublishMode, PublishRequest};

/// Exact regular-file bytes used to derive a mutation plan.
///
/// Binding the input to the same [`CompileTransaction`] as the derived output
/// makes a concurrent edit fail the transaction instead of publishing output
/// calculated from stale bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ExactFileInput {
    path: PathBuf,
    raw: Vec<u8>,
}

impl ExactFileInput {
    pub(crate) fn new(path: impl Into<PathBuf>, raw: impl Into<Vec<u8>>) -> Self {
        Self {
            path: path.into(),
            raw: raw.into(),
        }
    }

    pub(crate) fn bind_to_staged(
        &self,
        root: &Path,
        staged: &mut unica_workspace::apply::ApplyStagedState,
    ) -> Result<(), unica_source_contract::proposal::ApplyStagingError> {
        use unica_source_contract::proposal::{ApplyStagingError, ApplyStagingErrorKind};
        let relative = self.path.strip_prefix(root).map_err(|_| {
            ApplyStagingError::new(
                ApplyStagingErrorKind::ContainmentIdentity,
                "apply input is outside the admitted source root",
            )
        })?;
        if staged.read(relative)?.as_deref() != Some(self.raw.as_slice()) {
            return Err(ApplyStagingError::new(
                ApplyStagingErrorKind::ConcurrentRevision,
                "file-backed apply input changed during planning",
            ));
        }
        Ok(())
    }

    pub(crate) fn bind_to(&self, transaction: &mut CompileTransaction) -> Result<(), String> {
        transaction.guard_or_verify_exact_preimage(&self.path, &self.raw)
    }
}

/// JSON parsed from one exact byte snapshot. The parsed value is deliberately
/// unavailable until those source bytes are bound to the publication
/// transaction.
pub(crate) struct FileBackedJson {
    input: ExactFileInput,
    value: Value,
}

impl FileBackedJson {
    pub(crate) fn read(
        path: &Path,
        parse_error: impl FnOnce(serde_json::Error) -> String,
    ) -> Result<Self, String> {
        let snapshot = read_utf8_sig_snapshot(path)?;
        let value = serde_json::from_str(snapshot.text.as_str()).map_err(parse_error)?;
        Ok(Self {
            input: ExactFileInput::new(path, snapshot.raw),
            value,
        })
    }

    pub(crate) fn bind_to(self, transaction: &mut CompileTransaction) -> Result<Value, String> {
        self.input.bind_to(transaction)?;
        Ok(self.value)
    }
}

pub(crate) fn read_utf8_sig_snapshot(
    path: &Path,
) -> Result<unica_xml_format::text::Utf8TextSnapshot, String> {
    // The legacy entry point binds the same ordinary, uncaptured reads. The
    // route is passed through unchanged; this session construction performs no IO.
    let reader = unica_workspace::discovery::ProjectDiscoveryReader::new(Path::new(""));
    unica_source_designer::read::read_utf8_sig_snapshot(&reader, path)
}

pub(crate) fn read_utf8_sig(path: &Path) -> Result<String, String> {
    let reader = unica_workspace::discovery::ProjectDiscoveryReader::new(Path::new(""));
    unica_source_designer::read::read_utf8_sig(&reader, path)
}

pub(crate) fn resolve_role_validate_rights_path(path: PathBuf) -> PathBuf {
    let mut rights_path = path;
    if rights_path.is_dir() {
        rights_path = rights_path.join("Ext").join("Rights.xml");
    }
    if !rights_path.exists()
        && rights_path.file_name().and_then(|value| value.to_str()) == Some("Rights.xml")
    {
        if let Some(parent) = rights_path.parent() {
            let candidate = parent.join("Ext").join("Rights.xml");
            if candidate.exists() {
                rights_path = candidate;
            }
        }
    }
    rights_path
}

const ROLE_KINDS: &[&str] = &["Role"];

pub(crate) fn typed_role_reader_target(address: &QualifiedAddress) -> Option<MetadataAddress> {
    typed_reader_metadata_target(address, ROLE_KINDS)
}

pub(crate) fn resolve_role_read_rights_path(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> Result<PathBuf, String> {
    if let Some(selection) = logical_selection(args, context, AttachedResource::Rights, ROLE_KINDS)
    {
        return selection
            .map(|selection| selection.resource_path)
            .map_err(|failure| failure.to_string());
    }
    let raw = unica_operation_contract::input::required_path(args, RIGHTS_PATH, "RightsPath")?;
    Ok(resolve_role_validate_rights_path(
        unica_operation_contract::input::absolutize(raw, &context.cwd),
    ))
}

pub(crate) fn resolve_role_info_target(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> Result<ResolvedReadTarget, String> {
    if let Some(selection) = logical_selection(args, context, AttachedResource::Rights, ROLE_KINDS)
    {
        return selection.map_err(|failure| failure.to_string());
    }
    let rights_path = resolve_role_read_rights_path(args, context)?;
    physical_selection(&rights_path, context, AttachedResource::Rights)
        .map_err(|failure| failure.to_string())
}

pub(crate) fn resolve_cf_edit_config_path(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> Result<PathBuf, String> {
    let mut config_path =
        unica_operation_contract::input::required_path(args, CF_PATH, "ConfigPath")
            .map(|path| unica_operation_contract::input::absolutize(path, &context.cwd))?;
    if config_path.is_dir() {
        let candidate = config_path.join("Configuration.xml");
        if candidate.is_file() {
            config_path = candidate;
        } else {
            return Err("No Configuration.xml in directory".to_string());
        }
    }
    if !config_path.is_file() {
        return Err(format!("File not found: {}", config_path.display()));
    }
    Ok(config_path)
}

pub(crate) fn resolve_cf_read_config_path(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> Result<PathBuf, String> {
    if let Some(selection) =
        logical_selection(args, context, AttachedResource::ConfigurationRoot, &[])
    {
        return selection
            .map(|selection| selection.resource_path)
            .map_err(|failure| failure.to_string());
    }
    resolve_configuration_read_path(args, CF_PATH, "ConfigPath", context)
}

pub(crate) fn resolve_cf_info_target(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> Result<ResolvedReadTarget, String> {
    if let Some(selection) =
        logical_selection(args, context, AttachedResource::ConfigurationRoot, &[])
    {
        return selection.map_err(|failure| failure.to_string());
    }
    let config_path = resolve_configuration_read_path(args, CF_PATH, "ConfigPath", context)?;
    physical_selection(&config_path, context, AttachedResource::ConfigurationRoot)
        .map_err(|failure| failure.to_string())
}

pub(crate) fn resolve_cfe_validate_config_path(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> Result<PathBuf, String> {
    resolve_configuration_read_path(args, CFE_VALIDATE_PATH, "ExtensionPath", context)
}

fn resolve_configuration_read_path(
    args: &Map<String, Value>,
    names: &[&str],
    label: &str,
    context: &WorkspaceContext,
) -> Result<PathBuf, String> {
    let raw = unica_operation_contract::input::required_path(args, names, label)?;
    let mut path = unica_operation_contract::input::absolutize(raw, &context.cwd);
    if path.is_dir() {
        let candidate = path.join("Configuration.xml");
        if candidate.is_file() {
            path = candidate;
        } else {
            return Err(format!(
                "[ERROR] No Configuration.xml found in directory: {}",
                path.display()
            ));
        }
    }
    if !path.is_file() {
        return Err(format!("[ERROR] File not found: {}", path.display()));
    }
    Ok(path.canonicalize().unwrap_or(path))
}

pub(crate) fn output_dir_arg(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
    names: &[&str],
    default: &str,
) -> PathBuf {
    let path = unica_operation_contract::input::path_arg(args, names)
        .unwrap_or_else(|| PathBuf::from(default));
    unica_operation_contract::input::absolutize(path, &context.cwd)
}

pub(crate) fn write_utf8_bom(path: &Path, content: &str) -> Result<(), String> {
    let bytes = unica_xml_format::text::utf8_bom_bytes(content);
    let expected_preimage = match fs::read(path) {
        Ok(bytes) => Some(bytes),
        Err(error) if error.kind() == ErrorKind::NotFound => None,
        Err(error) => {
            return Err(format!(
                "failed to inspect UTF-8 BOM publication target {}: {error}",
                path.display()
            ));
        }
    };
    let mode = match expected_preimage.as_deref() {
        Some(expected_preimage) => PublishMode::ReplaceExisting { expected_preimage },
        None => PublishMode::CreateOnly,
    };
    let report = publish(PublishRequest {
        target: path,
        replacement: &bytes,
        mode,
    })
    .map_err(|error| format!("failed to publish {}: {error}", path.display()))?;
    if report.cleanup_warnings.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "published {} but publication cleanup is incomplete: {}",
            path.display(),
            report
                .cleanup_warnings
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join("; ")
        ))
    }
}

#[cfg(test)]
mod mutation_tests {
    use super::{guard_active_format_owner, read_utf8_sig_snapshot, write_utf8_bom};
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};
    use unica_metadata::subsystem::SubsystemEditModel;
    use unica_metadata_designer::subsystem::emit_subsystem_edit_model;
    use unica_source_contract::workspace::WorkspaceContext;
    use unica_source_designer::format_profile::{ExportFormatVersion, FormatCompatibility};
    use unica_workspace::compile_transaction::CompileTransaction;
    use unica_workspace::single_file_publisher::{
        with_before_commit_hook, with_publish_failpoints, PublishCheckpoint,
    };

    fn format_owner_context(name: &str, version: &str) -> (WorkspaceContext, std::path::PathBuf) {
        let root = std::env::temp_dir().join(format!(
            "unica-common-format-owner-{name}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let source = root.join("src");
        fs::create_dir_all(source.join("Templates/Guarded/Ext")).unwrap();
        fs::write(
            root.join("v8project.yaml"),
            "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
        )
        .unwrap();
        fs::write(
            source.join("Configuration.xml"),
            format!(
                "<MetaDataObject xmlns=\"http://v8.1c.ru/8.3/MDClasses\" version=\"{version}\"><Configuration uuid=\"aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa\"><Properties><Name>Demo</Name></Properties><ChildObjects/></Configuration></MetaDataObject>"
            ),
        )
        .unwrap();
        let target = source.join("Templates/Guarded/Ext/Template.xml");
        (
            WorkspaceContext {
                cwd: root.clone(),
                workspace_root: root.clone(),
                cache_root: root.join(".build/unica"),
                workspace_epoch: 1,
            },
            target,
        )
    }

    #[test]
    fn utf8_bom_bytes_emits_exactly_one_bom() {
        assert_eq!(
            unica_xml_format::text::utf8_bom_bytes("<xml/>"),
            b"\xef\xbb\xbf<xml/>"
        );
        assert_eq!(
            unica_xml_format::text::utf8_bom_bytes("\u{feff}\u{feff}<xml/>"),
            b"\xef\xbb\xbf<xml/>"
        );
    }

    #[test]
    fn utf8_snapshot_keeps_raw_preimage_and_decodes_text_without_bom() {
        let root = std::env::temp_dir().join(format!(
            "unica-common-snapshot-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&root).unwrap();
        let path = root.join("Configuration.xml");
        let raw = b"\xef\xbb\xbf<xml/>\r\n";
        fs::write(&path, raw).unwrap();

        let snapshot = read_utf8_sig_snapshot(&path).unwrap();

        assert_eq!(snapshot.raw, raw);
        assert_eq!(snapshot.text, "<xml/>\r\n");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn utf8_bom_writer_never_overwrites_a_concurrent_replacement() {
        let root = std::env::temp_dir().join(format!(
            "unica-common-publish-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&root).unwrap();
        let path = root.join("Configuration.xml");
        fs::write(&path, b"\xef\xbb\xbforiginal").unwrap();
        let concurrent = b"\xef\xbb\xbfconcurrent";

        let result = with_before_commit_hook(
            move |target| fs::write(target, concurrent).unwrap(),
            || write_utf8_bom(&path, "replacement"),
        );

        let error = result.expect_err("stale preimage must abort publication");
        assert!(
            error.contains("differs from the expected preimage"),
            "{error}"
        );
        assert_eq!(fs::read(&path).unwrap(), concurrent);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn utf8_bom_writer_surfaces_cleanup_warning_after_committed_create() {
        let root = std::env::temp_dir().join(format!(
            "unica-common-cleanup-warning-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&root).unwrap();
        let path = root.join("Configuration.xml");

        let result = with_publish_failpoints(&[PublishCheckpoint::Cleanup], || {
            write_utf8_bom(&path, "<MetaDataObject/>")
        });

        let error = result.expect_err("committed cleanup warning must never be hidden");
        assert!(error.contains("published"), "{error}");
        assert!(error.contains("cleanup is incomplete"), "{error}");
        assert!(
            error.contains("injected publication cleanup failure"),
            "{error}"
        );
        assert_eq!(fs::read(&path).unwrap(), b"\xef\xbb\xbf<MetaDataObject/>");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn older_format_warning_offers_explicit_platform_reexport_without_auto_migration() {
        let warning = unica_source_designer::format_profile::format_compatibility_warning(
            &FormatCompatibility::Older {
                actual: ExportFormatVersion::parse("2.19").unwrap(),
            },
        );

        assert!(
            warning.contains("will not migrate it automatically"),
            "{warning}"
        );
        assert!(warning.contains("re-export"), "{warning}");
        assert!(warning.contains("1C:Enterprise 8.3.27"), "{warning}");
        assert!(!warning.contains("migrate the configuration before writing"));
    }

    #[test]
    fn newer_format_warning_keeps_8_5_roadmap_copy_without_downgrade_offer() {
        let warning = unica_source_designer::format_profile::format_compatibility_warning(
            &FormatCompatibility::Newer {
                actual: ExportFormatVersion::parse("2.21").unwrap(),
            },
        );

        assert!(warning.contains("1C 8.5 support is planned"), "{warning}");
        assert!(!warning.to_ascii_lowercase().contains("downgrade"));
    }

    #[test]
    fn active_format_owner_guard_reauthorizes_and_rejects_newer_snapshot() {
        let (context, target) = format_owner_context("newer", "2.21");
        let mut transaction = CompileTransaction::new();
        transaction
            .create_utf8_bom_text(&target, "<Template/>")
            .unwrap();

        let error = guard_active_format_owner(&mut transaction, &target, &context)
            .expect_err("handler-side guard must reject a newer owner snapshot");

        assert!(error.contains("2.21"), "{error}");
        assert!(error.contains("2.20"), "{error}");
        assert!(!target.exists());
        fs::remove_dir_all(context.workspace_root).unwrap();
    }

    #[test]
    fn active_format_owner_guard_binds_supported_snapshot_to_commit() {
        let (context, target) = format_owner_context("concurrent", "2.20");
        let owner = context.workspace_root.join("src/Configuration.xml");
        let mut transaction = CompileTransaction::new();
        transaction
            .create_utf8_bom_text(&target, "<Template/>")
            .unwrap();
        guard_active_format_owner(&mut transaction, &target, &context).unwrap();
        fs::write(
            &owner,
            "<MetaDataObject xmlns=\"http://v8.1c.ru/8.3/MDClasses\" version=\"2.21\"><Configuration/></MetaDataObject>",
        )
        .unwrap();

        let error = transaction
            .commit()
            .expect_err("owner drift after handler reauthorization must abort publication");

        assert!(error.contains("read guard"), "{error}");
        assert!(!target.exists());
        fs::remove_dir_all(context.workspace_root).unwrap();
    }
}

pub(crate) fn analyze_xml(
    operation: &str,
    tool_name: &str,
    target: &Path,
    text: &str,
) -> AdapterOutcome {
    match Document::parse(text) {
        Ok(doc) => {
            let root = doc.root_element();
            let element_count = doc.descendants().filter(|node| node.is_element()).count();
            let summary = json!({
                "operation": operation,
                "file": target.display().to_string(),
                "root": root.tag_name().name(),
                "name": unica_xml_format::read::first_text(&doc, "Name"),
                "synonym": unica_xml_format::read::first_text(&doc, "Synonym"),
                "elementCount": element_count,
                "topLevel": root
                    .children()
                    .filter(|node| node.is_element())
                    .map(|node| node.tag_name().name().to_string())
                    .collect::<Vec<_>>(),
            });
            AdapterOutcome {
                ok: true,
                summary: format!("{tool_name} completed with native XML parser"),
                changes: Vec::new(),
                warnings: validation_warnings(operation, &doc),
                errors: Vec::new(),
                artifacts: vec![target.display().to_string()],
                stdout: Some(
                    serde_json::to_string_pretty(&summary).unwrap_or_else(|_| summary.to_string()),
                ),
                stderr: None,
                command: None,
            }
        }
        Err(err) => AdapterOutcome {
            ok: false,
            summary: format!("{tool_name} failed native XML validation"),
            changes: Vec::new(),
            warnings: Vec::new(),
            errors: vec![format!("XML parse error in {}: {err}", target.display())],
            artifacts: vec![target.display().to_string()],
            stdout: None,
            stderr: None,
            command: None,
        },
    }
}

pub(crate) fn validation_warnings(operation: &str, doc: &Document<'_>) -> Vec<String> {
    let mut warnings = Vec::new();
    let root = doc.root_element().tag_name().name();
    if operation.starts_with("cf-") && root != "MetaDataObject" {
        warnings.push(format!("expected MetaDataObject root, got {root}"));
    }
    if operation.starts_with("role-") && !unica_xml_format::names::has_element(doc, "Rights") {
        warnings.push("expected role Rights content".to_string());
    }
    if operation.starts_with("form-")
        && !unica_xml_format::names::has_element(doc, "Form")
        && root != "Form"
    {
        warnings.push("expected managed form XML content".to_string());
    }
    warnings
}

pub(crate) fn resolve_target(
    operation: &str,
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> Result<PathBuf, String> {
    let path = if operation.starts_with("cf-") {
        unica_operation_contract::input::required_path(
            args,
            &["configPath", "ConfigPath", "path", "Path"],
            "ConfigPath",
        )?
    } else if operation.starts_with("cfe-") {
        unica_operation_contract::input::required_path(
            args,
            &["extensionPath", "ExtensionPath", "path", "Path"],
            "ExtensionPath",
        )?
    } else if operation.starts_with("meta-") {
        unica_operation_contract::input::required_path(
            args,
            &["objectPath", "ObjectPath", "path", "Path"],
            "ObjectPath",
        )?
    } else if operation.starts_with("form-") {
        unica_operation_contract::input::required_path(
            args,
            &["formPath", "FormPath", "path", "Path"],
            "FormPath",
        )?
    } else if operation.starts_with("interface-") {
        unica_operation_contract::input::required_path(
            args,
            &["ciPath", "CIPath", "path", "Path"],
            "CIPath",
        )?
    } else if operation.starts_with("subsystem-") {
        unica_operation_contract::input::required_path(
            args,
            &["subsystemPath", "SubsystemPath", "path", "Path"],
            "SubsystemPath",
        )?
    } else if operation.starts_with("dcs-") || operation.starts_with("mxl-") {
        unica_operation_contract::input::required_path(
            args,
            &["templatePath", "TemplatePath", "path", "Path"],
            "TemplatePath",
        )?
    } else if operation.starts_with("role-") {
        unica_operation_contract::input::required_path(
            args,
            &["rightsPath", "RightsPath", "path", "Path"],
            "RightsPath",
        )?
    } else {
        return Err(format!(
            "native operation {operation} does not define a path argument"
        ));
    };

    Ok(resolve_existing_path(
        operation,
        unica_operation_contract::input::absolutize(path, &context.cwd),
    ))
}

pub(crate) fn resolve_existing_path(operation: &str, path: PathBuf) -> PathBuf {
    if path.is_dir() {
        let leaf = path
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or_default();
        for candidate in directory_candidates(operation, &path, leaf) {
            if candidate.is_file() {
                return candidate;
            }
        }
    }

    if !path.is_file() && path.extension().and_then(|value| value.to_str()) == Some("xml") {
        if let Some(stem) = path.file_stem().and_then(|value| value.to_str()) {
            if let Some(parent) = path.parent() {
                let candidate = parent.join(stem).join("Ext").join(special_file(operation));
                if candidate.is_file() {
                    return candidate;
                }
            }
        }
    }

    path
}

pub(crate) fn directory_candidates(operation: &str, path: &Path, leaf: &str) -> Vec<PathBuf> {
    if operation.starts_with("cf-") || operation.starts_with("cfe-") {
        vec![path.join("Configuration.xml")]
    } else if operation.starts_with("form-") {
        vec![path.join("Ext").join("Form.xml")]
    } else if operation.starts_with("interface-") {
        vec![path.join("Ext").join("CommandInterface.xml")]
    } else if operation.starts_with("dcs-") || operation.starts_with("mxl-") {
        vec![path.join("Ext").join("Template.xml")]
    } else if operation.starts_with("role-") {
        vec![path.join("Ext").join("Rights.xml")]
    } else {
        vec![path.join(format!("{leaf}.xml"))]
    }
}

pub(crate) fn special_file(operation: &str) -> &'static str {
    if operation.starts_with("form-") {
        "Form.xml"
    } else if operation.starts_with("role-") {
        "Rights.xml"
    } else {
        "Template.xml"
    }
}

pub(crate) fn extension_name_prefix(config: &Path) -> Option<String> {
    let text = fs::read_to_string(config).ok()?;
    let doc = Document::parse(text.trim_start_matches('\u{feff}')).ok()?;
    doc.descendants()
        .find(|node| node.is_element() && node.tag_name().name() == "NamePrefix")
        .and_then(|node| node.text())
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

pub(crate) fn detect_format_version(
    target: &Path,
    context: &WorkspaceContext,
) -> Result<ExportFormatVersion, String> {
    let owners =
        crate::infrastructure::platform_xml_owner::resolve_platform_xml_owners(target, context)
            .map_err(|error| error.message)?;
    require_supported_platform_xml_owners(&owners)?;
    ExportFormatVersion::parse(ACTIVE_FORMAT_PROFILE.export_format)
        .map_err(|error| error.to_string())
}

/// Re-authorize the current version-owning XML bytes at handler planning time
/// and bind that exact snapshot to the transaction. The application guard is
/// user-facing early rejection; this guard closes the authorization/publication
/// window for cooperating Unica writers.
pub(crate) fn guard_active_format_owner(
    transaction: &mut CompileTransaction,
    target: &Path,
    context: &WorkspaceContext,
) -> Result<(), String> {
    guard_active_format_dependencies(transaction, &[target], context)
}

pub(crate) fn code_patch_source_target(
    args: &Map<String, Value>,
) -> Result<SourceTarget, SourceTargetError> {
    let source_set = args
        .get("sourceSet")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            SourceTargetError::new(
                SourceTargetErrorCode::SourceSetRequired,
                "sourceSet must name an exact project source set",
            )
        })?;
    let raw_metadata_path = args
        .get("metadataPath")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            SourceTargetError::new(
                SourceTargetErrorCode::MetadataAddressInvalid,
                "metadataPath must identify one existing module",
            )
        })?;
    let metadata_path = MetadataAddress::parse(PLATFORM_XML_8_3_27_FORMAT_2_20, raw_metadata_path)?;
    Ok(SourceTarget {
        source_set: source_set.to_string(),
        metadata_path: Some(metadata_path),
    })
}

/// Builds the logical target of a read-only object reader from typed arguments.
pub(crate) fn metadata_object_source_target(
    args: &Map<String, Value>,
) -> Result<SourceTarget, SourceTargetError> {
    let source_set = args
        .get("sourceSet")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            SourceTargetError::new(
                SourceTargetErrorCode::SourceSetRequired,
                "sourceSet must name an exact project source set",
            )
        })?;
    let raw_metadata_path = args
        .get("metadataPath")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            SourceTargetError::new(
                SourceTargetErrorCode::MetadataAddressInvalid,
                "metadataPath must identify one existing metadata object",
            )
        })?;
    let metadata_path = MetadataAddress::parse(PLATFORM_XML_8_3_27_FORMAT_2_20, raw_metadata_path)?;
    Ok(SourceTarget {
        source_set: source_set.to_string(),
        metadata_path: Some(metadata_path),
    })
}

/// Resolves a metadata object address to the descriptor that a reader parses.
/// A module terminal is refused by name: reading a module is `unica.code.*`
/// work, and silently reading its owner instead would answer another question.
pub(crate) fn resolve_metadata_object_descriptor(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> Result<(ResolvedTarget, PathBuf), String> {
    let target = metadata_object_source_target(args).map_err(|error| error.to_string())?;
    let resolution = resolve_platform_xml_target(context, &target, TargetKindPolicy::Any)
        .map_err(|error| error.to_string())?;
    if resolution.resolved.target_kind != TargetKind::MetadataObject {
        return Err(format!(
            "metadataPath `{}` names a module terminal; metadata object readers address the object itself",
            target
                .metadata_path
                .as_ref()
                .map(MetadataAddress::as_str)
                .unwrap_or_default()
        ));
    }
    let path = revalidate_platform_xml_target(context, &resolution.handle)
        .map_err(|error| error.to_string())?
        .path;
    Ok((resolution.resolved, path))
}

pub(crate) fn resolve_code_patch_guard_path(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> Result<PathBuf, String> {
    let target = code_patch_source_target(args).map_err(|error| error.to_string())?;
    let resolution = resolve_platform_xml_target(context, &target, TargetKindPolicy::ModuleOnly)
        .map_err(|error| error.to_string())?;
    revalidate_platform_xml_target(context, &resolution.handle)
        .map(|target| target.path)
        .map_err(|error| error.to_string())
}

pub(crate) fn guard_code_patch_resolved_target(
    transaction: &mut CompileTransaction,
    handle: &ClosedPlatformXmlTarget,
    context: &WorkspaceContext,
) -> Result<PathBuf, String> {
    // Belt and braces around the resolver policy: a handle that is not a module
    // never reaches a writer, whatever produced it.
    if handle.target_kind() != TargetKind::Module {
        return Err(SourceTargetError::new(
            SourceTargetErrorCode::TargetKindMismatch,
            "metadataPath does not identify a module terminal",
        )
        .to_string());
    }
    guard_resolved_platform_xml_target_dependencies(transaction, handle, context)
}

/// Revalidate a closed logical target and bind every currently available
/// format/support input used to authorize a write to the same transaction.
/// Target-kind admission remains the caller's responsibility: in particular,
/// `code.patch` keeps its explicit `ModuleOnly` check above.
pub(crate) fn guard_resolved_platform_xml_target_dependencies(
    transaction: &mut CompileTransaction,
    handle: &ClosedPlatformXmlTarget,
    context: &WorkspaceContext,
) -> Result<PathBuf, String> {
    let target = revalidate_platform_xml_target(context, handle)
        .map_err(|error| error.to_string())?
        .path;
    guard_active_format_owner(transaction, &target, context)?;
    for dependency in support_uuid_dependency_paths(&target) {
        let bytes = fs::read(&dependency)
            .map_err(|error| format!("failed to read {}: {error}", dependency.display()))?;
        guard_exact_preimage_if_unprotected(transaction, &dependency, bytes)?;
    }
    if let Some(config_dir) = find_support_config_dir(&target) {
        let support = config_dir.join("Ext").join("ParentConfigurations.bin");
        if support.is_file() {
            let bytes = fs::read(&support)
                .map_err(|error| format!("failed to read {}: {error}", support.display()))?;
            guard_exact_preimage_if_unprotected(transaction, &support, bytes)?;
        } else {
            transaction.guard_path_absent(&support)?;
        }
    }
    Ok(target)
}

pub(crate) fn guard_active_format_owner_with_exact_root(
    transaction: &mut CompileTransaction,
    target: &Path,
    context: &WorkspaceContext,
    expected_root: unica_source_designer::owner::PlatformXmlRootExpectation,
) -> Result<(), String> {
    guard_active_format_dependency_targets(
        transaction,
        &[FormatDependencyTarget {
            path: target,
            expected_root: Some(expected_root),
        }],
        context,
    )
}

pub(crate) fn guard_active_format_dependencies(
    transaction: &mut CompileTransaction,
    targets: &[&Path],
    context: &WorkspaceContext,
) -> Result<(), String> {
    let targets = targets
        .iter()
        .map(|path| FormatDependencyTarget {
            path,
            expected_root: None,
        })
        .collect::<Vec<_>>();
    guard_active_format_dependency_targets(transaction, &targets, context)
}

#[derive(Clone, Copy)]
struct FormatDependencyTarget<'a> {
    path: &'a Path,
    expected_root: Option<unica_source_designer::owner::PlatformXmlRootExpectation>,
}

fn guard_active_format_dependency_targets(
    transaction: &mut CompileTransaction,
    targets: &[FormatDependencyTarget<'_>],
    context: &WorkspaceContext,
) -> Result<(), String> {
    let mut owners = BTreeMap::new();
    let mut provenances = Vec::new();
    for target in targets {
        let resolution = match target.expected_root {
            Some(expected_root) => crate::infrastructure::platform_xml_owner::
                resolve_platform_xml_owners_for_exact_root_with_provenance(
                    target.path,
                    context,
                    expected_root,
                ),
            None => crate::infrastructure::platform_xml_owner::
                resolve_platform_xml_owners_with_provenance(target.path, context),
        }
        .map_err(|error| error.message)?;
        for owner in resolution.owners {
            owners.entry(owner.path.clone()).or_insert(owner);
        }
        provenances.push(resolution.provenance);
    }
    let owners = owners.into_values().collect::<Vec<_>>();
    require_supported_platform_xml_owners(&owners)?;
    for provenance in provenances {
        provenance.bind_to(transaction)?;
    }
    for owner in owners {
        if !transaction.protects_path(&owner.path)? {
            transaction.guard_exact_preimage(owner.path, owner.raw)?;
        }
    }
    Ok(())
}

pub(crate) fn guard_active_format_containing_owner_for_new_output(
    transaction: &mut CompileTransaction,
    target: &Path,
    context: &WorkspaceContext,
) -> Result<(), String> {
    let resolution = crate::infrastructure::platform_xml_owner::
        resolve_existing_platform_xml_owners_for_new_output_with_provenance(target, context)
        .map_err(|error| error.message)?;
    let owners = resolution.owners;
    require_supported_platform_xml_owners(&owners)?;
    resolution.provenance.bind_to(transaction)?;
    for owner in owners {
        if !transaction.protects_path(&owner.path)? {
            transaction.guard_exact_preimage(owner.path, owner.raw)?;
        }
    }
    Ok(())
}

pub(crate) fn guard_active_format_xml_tree(
    transaction: &mut CompileTransaction,
    target: &Path,
    context: &WorkspaceContext,
) -> Result<(), String> {
    guard_active_format_dependencies_and_xml_trees(transaction, &[], &[target], context)
}

pub(crate) fn guard_active_format_dependencies_and_xml_trees(
    transaction: &mut CompileTransaction,
    dependencies: &[&Path],
    trees: &[&Path],
    context: &WorkspaceContext,
) -> Result<(), String> {
    let mut paths = Vec::new();
    paths.extend(dependencies.iter().map(|path| (*path).to_path_buf()));
    for tree in trees {
        collect_platform_xml_tree_paths(tree, &mut paths)?;
    }
    paths.sort();
    paths.dedup();
    let targets = paths.iter().map(PathBuf::as_path).collect::<Vec<_>>();
    guard_active_format_dependencies(transaction, &targets, context)
}

fn collect_platform_xml_tree_paths(target: &Path, paths: &mut Vec<PathBuf>) -> Result<(), String> {
    let metadata = match fs::symlink_metadata(target) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(()),
        Err(error) => {
            return Err(format!("failed to inspect {}: {error}", target.display()));
        }
    };
    if unica_workspace::paths::metadata_is_link_or_reparse_point(&metadata) {
        return Err(format!(
            "platform XML dependency must not be a symbolic link or reparse point: {}",
            target.display()
        ));
    }
    if metadata.is_file() {
        if target
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| extension.eq_ignore_ascii_case("xml"))
        {
            paths.push(target.to_path_buf());
        }
        return Ok(());
    }
    if !metadata.is_dir() {
        return Err(format!(
            "platform XML dependency is neither a regular file nor a directory: {}",
            target.display()
        ));
    }
    let mut entries = fs::read_dir(target)
        .map_err(|error| format!("failed to inspect {}: {error}", target.display()))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("failed to inspect {}: {error}", target.display()))?;
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        collect_platform_xml_tree_paths(&entry.path(), paths)?;
    }
    Ok(())
}

fn require_supported_platform_xml_owners(
    owners: &[unica_source_designer::owner::PlatformXmlOwner],
) -> Result<(), String> {
    let mut older = None;
    let mut newer = None;
    for owner in owners {
        match classify_root_version(owner.version.as_deref()) {
            Ok(FormatCompatibility::Supported { .. }) => {}
            Ok(compatibility @ FormatCompatibility::Older { .. }) if older.is_none() => {
                older = Some(compatibility);
            }
            Ok(compatibility @ FormatCompatibility::Newer { .. }) if newer.is_none() => {
                newer = Some(compatibility);
            }
            Ok(FormatCompatibility::Older { .. } | FormatCompatibility::Newer { .. }) => {}
            Err(error) => return Err(error.to_string()),
        }
    }
    if let Some(compatibility) = newer.or(older) {
        return Err(
            unica_source_designer::format_profile::format_compatibility_warning(&compatibility),
        );
    }
    Ok(())
}

/// Bind bytes that were already used to derive a write plan. This differs
/// from taking a fresh snapshot immediately before commit: a later, still
/// valid `2.20` document must not authorize output calculated from older
/// bytes.
pub(crate) fn guard_exact_preimage_if_unprotected(
    transaction: &mut CompileTransaction,
    path: &Path,
    expected_preimage: impl AsRef<[u8]>,
) -> Result<(), String> {
    if !transaction.protects_path(path)? {
        transaction.guard_exact_preimage(path, expected_preimage)?;
    }
    Ok(())
}

pub(crate) fn support_state_lines_for_configuration(
    config_path: &Path,
    is_extension: bool,
) -> Vec<String> {
    let config_dir = if config_path.is_dir() {
        config_path
    } else {
        config_path.parent().unwrap_or_else(|| Path::new(""))
    };
    let bin_path = config_dir.join("Ext").join("ParentConfigurations.bin");
    let Some(state) = read_support_state(&bin_path) else {
        return vec![if is_extension {
            "Поддержка:      расширение (CFE), правки свободны".to_string()
        } else {
            "Поддержка:      не на поддержке (своя конфигурация)".to_string()
        }];
    };
    if state.removed {
        return vec!["Поддержка:      снята с поддержки полностью".to_string()];
    }

    let mut lines = vec!["Поддержка:      на поддержке".to_string()];
    if state.global_editing_enabled {
        lines.push("  Возможность изменения: включена".to_string());
        lines.push(format!(
            "  Объектов: на замке {} / редактируется {} / снято {}",
            state.counts[0], state.counts[1], state.counts[2]
        ));
    } else {
        lines.push(
            "  Возможность изменения: выключена — вся конфигурация read-only (правки заблокированы)"
                .to_string(),
        );
    }
    lines.push(format!("  Конфигураций поставщика: {}", state.vendor_count));
    if state.vendor_count > 1 {
        for vendor in &state.vendors {
            lines.push(format!(
                "  Поставщик: {} — {} {}",
                vendor.vendor, vendor.name, vendor.version
            ));
        }
    }
    lines
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SupportGuardViolation {
    pub code: &'static str,
    pub reason: String,
    pub target_path: PathBuf,
    pub config_dir: PathBuf,
}

pub(crate) fn support_guard_violation(
    target_path: &Path,
    requirement: SupportGuardRequirement,
) -> Option<SupportGuardViolation> {
    let target_path =
        normalize_path_identity(target_path).unwrap_or_else(|_| target_path.to_path_buf());
    let config_dir = find_support_config_dir(&target_path)?;
    let bin_path = config_dir.join("Ext").join("ParentConfigurations.bin");
    let state = read_support_state(&bin_path)?;
    if state.removed {
        return None;
    }
    if !state.global_editing_enabled {
        return Some(SupportGuardViolation {
            code: "capability-off",
            reason: "возможность изменения конфигурации выключена (вся конфигурация read-only)"
                .to_string(),
            target_path,
            config_dir,
        });
    }

    let object_uuid = support_object_uuid_for_path(&target_path)
        .or_else(|| support_root_uuid(&config_dir.join("Configuration.xml")));
    let object_rule = object_uuid
        .as_deref()
        .and_then(|uuid| state.object_rule(uuid));
    match requirement {
        SupportGuardRequirement::Removed if object_rule.is_some_and(|rule| rule != 2) => {
            Some(SupportGuardViolation {
                code: "not-removed",
                reason: "объект не снят с поддержки — удаление сломает обновления".to_string(),
                target_path,
                config_dir,
            })
        }
        SupportGuardRequirement::Editable if object_rule == Some(0) => {
            Some(SupportGuardViolation {
                code: "locked",
                reason: "объект на замке — редактирование сломает обновления".to_string(),
                target_path,
                config_dir,
            })
        }
        _ => None,
    }
}

pub(crate) fn read_support_state(bin_path: &Path) -> Option<SupportState> {
    if !bin_path.is_file() {
        return None;
    }
    let data = fs::read(bin_path).ok()?;
    parse_support_state_compat_bytes(Some(&data))
}

/// Strict read semantics for subject readers (ADR-0054).
///
/// The mutating guard deliberately keeps using [`read_support_state`] until its
/// pre-image and atomicity contract is migrated separately. Subject readers
/// must distinguish an absent marker from evidence that exists but cannot be
/// trusted.
pub(crate) fn read_support_state_strict(
    bin_path: &Path,
) -> Result<Option<SupportState>, SupportReadError> {
    let marker_parent = bin_path.parent().ok_or_else(|| {
        SupportReadError::new(
            SupportReadErrorCode::StateUnreadable,
            "support-state marker parent is unavailable",
        )
    })?;
    let parent_metadata = match fs::symlink_metadata(marker_parent) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
        Err(_) => {
            return Err(SupportReadError::new(
                SupportReadErrorCode::StateUnreadable,
                "support-state marker parent metadata is unreadable",
            ))
        }
    };
    if metadata_is_link_or_reparse_point(&parent_metadata) || !parent_metadata.is_dir() {
        return Err(SupportReadError::new(
            SupportReadErrorCode::StateUnreadable,
            "support-state marker parent is not a direct directory",
        ));
    }
    let metadata = match fs::symlink_metadata(bin_path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
        Err(_) => {
            return Err(SupportReadError::new(
                SupportReadErrorCode::StateUnreadable,
                "support-state marker metadata is unreadable",
            ))
        }
    };
    if metadata_is_link_or_reparse_point(&metadata) || !metadata.is_file() {
        return Err(SupportReadError::new(
            SupportReadErrorCode::StateUnreadable,
            "support-state marker is not a direct regular file",
        ));
    }
    let data = fs::read(bin_path).map_err(|_| {
        SupportReadError::new(
            SupportReadErrorCode::StateUnreadable,
            "support-state marker bytes are unreadable",
        )
    })?;
    parse_support_state_strict_bytes(Some(&data))
}

pub(crate) fn find_support_config_dir(target_path: &Path) -> Option<PathBuf> {
    let mut current = if target_path.is_dir() {
        target_path.to_path_buf()
    } else {
        target_path.parent()?.to_path_buf()
    };
    for _ in 0..20 {
        if current
            .join("Ext")
            .join("ParentConfigurations.bin")
            .exists()
            || current.join("Configuration.xml").exists()
        {
            return Some(current);
        }
        let Some(parent) = current.parent() else {
            break;
        };
        if parent == current {
            break;
        }
        current = parent.to_path_buf();
    }
    None
}

pub(crate) fn support_uuid_dependency_paths(target_path: &Path) -> Vec<PathBuf> {
    let mut dependencies = Vec::new();
    if target_path.is_file()
        && target_path
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| extension.eq_ignore_ascii_case("xml"))
    {
        dependencies.push(target_path.to_path_buf());
        if support_root_uuid(target_path).is_some() {
            return dependencies;
        }
    }
    let mut current = if target_path.is_dir() {
        target_path.to_path_buf()
    } else {
        let Some(parent) = target_path.parent() else {
            return dependencies;
        };
        parent.to_path_buf()
    };
    for _ in 0..20 {
        let candidate = current.with_extension("xml");
        if candidate.is_file() && !dependencies.contains(&candidate) {
            dependencies.push(candidate.clone());
            if support_root_uuid(&candidate).is_some() {
                return dependencies;
            }
        }
        let Some(parent) = current.parent() else {
            break;
        };
        if parent == current {
            break;
        }
        current = parent.to_path_buf();
    }
    dependencies
}

pub(crate) fn support_uuid_dependency_path(target_path: &Path) -> Option<PathBuf> {
    support_uuid_dependency_paths(target_path)
        .into_iter()
        .find(|path| support_root_uuid(path).is_some())
}

pub(crate) fn support_object_uuid_for_path(target_path: &Path) -> Option<String> {
    support_uuid_dependency_path(target_path)
        .as_deref()
        .and_then(support_root_uuid)
}

pub(crate) fn support_root_uuid(xml_path: &Path) -> Option<String> {
    let raw = fs::read(xml_path).ok()?;
    support_root_uuid_from_bytes(&raw)
}
