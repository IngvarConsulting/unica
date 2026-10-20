use crate::infrastructure::native_operations::logical_selector::{
    logical_selection, physical_selection, typed_reader_metadata_target, AttachedResource,
    ResolvedReadTarget,
};
use crate::infrastructure::support_state::WorkspaceSupportStateReader;
use serde_json::{Map, Value};
use std::fs;
use std::path::{Path, PathBuf};
use unica_address::address::QualifiedAddress;
use unica_execution_contract::adapter_outcome::AdapterOutcome;
use unica_metadata::support_state::SupportStateReader;
use unica_source_contract::workspace::WorkspaceContext;
use unica_templates::mxl::{MxlInfoData, TEMPLATE_KINDS};
use unica_templates_designer::mxl_codec::{parse_mxl_info_xml, validate_mxl_document};

pub(crate) fn typed_mxl_reader_target(
    address: &QualifiedAddress,
) -> Option<unica_address::source_target::MetadataAddress> {
    typed_reader_metadata_target(address, TEMPLATE_KINDS)
}

pub(crate) struct MxlInfoExecution {
    pub(crate) outcome: AdapterOutcome,
    pub(crate) data: Option<MxlInfoData>,
}

pub(crate) fn analyze_mxl_info(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
    support_reader: &dyn SupportStateReader,
) -> MxlInfoExecution {
    let result = (|| -> Result<(MxlInfoData, PathBuf), String> {
        let selection = resolve_mxl_info_target(args, context)?;
        let template_path = selection.resource_path;
        if !template_path.is_file() {
            return Err(format!("File not found: {}", template_path.display()));
        }
        let bytes = fs::read(&template_path)
            .map_err(|err| format!("failed to read {}: {err}", template_path.display()))?;
        let template_name = template_path
            .parent()
            .and_then(Path::parent)
            .and_then(|path| path.file_name())
            .and_then(|value| value.to_str())
            .unwrap_or("");
        let support = support_reader
            .object_support(&selection.target)
            .map_err(|error| error.to_string())?;
        let data = parse_mxl_info_xml(
            &bytes,
            template_name,
            support,
            unica_operation_contract::input::bool_arg(args, &["withText", "WithText"]),
        )
        .map_err(|error| mxl_info_path_error(error, &template_path))?;
        Ok((data, template_path))
    })();

    match result {
        Ok((data, artifact)) => MxlInfoExecution {
            outcome: AdapterOutcome {
                ok: true,
                summary: format!(
                    "unica.mxl.info described {} with {} named area(s)",
                    data.name,
                    data.areas.len()
                ),
                changes: Vec::new(),
                warnings: Vec::new(),
                errors: Vec::new(),
                artifacts: vec![artifact.display().to_string()],
                stdout: None,
                stderr: Some(String::new()),
                command: None,
            },
            data: Some(data),
        },
        Err(error) => MxlInfoExecution {
            outcome: AdapterOutcome {
                ok: false,
                summary: "unica.mxl.info failed in native spreadsheet analyzer".to_string(),
                changes: Vec::new(),
                warnings: Vec::new(),
                errors: vec![error.clone()],
                artifacts: Vec::new(),
                stdout: None,
                stderr: Some(format!("{error}\n")),
                command: None,
            },
            data: None,
        },
    }
}

fn mxl_info_path_error(error: String, template_path: &Path) -> String {
    if error == "Template.xml is not UTF-8" {
        return format!(
            "failed to read {}: stream did not contain valid UTF-8",
            template_path.display()
        );
    }
    if let Some(detail) = error.strip_prefix("XML parse error in Template.xml:") {
        return format!("XML parse error in {}:{detail}", template_path.display());
    }
    error
}

pub(crate) fn validate_mxl(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> AdapterOutcome {
    match resolve_mxl_validate_path(args, context).and_then(|path| {
        fs::read_to_string(&path)
            .map_err(|err| format!("failed to read {}: {err}", path.display()))
            .map(|text| (path, text))
    }) {
        Ok((path, text)) => validate_mxl_input(args, &text, path),
        Err(error) => mxl_validation_error(error),
    }
}

pub(crate) fn validate_mxl_input(
    args: &Map<String, Value>,
    text: &str,
    template_path: PathBuf,
) -> AdapterOutcome {
    let result = validate_mxl_document(args, text, template_path);

    match result {
        Ok((ok, stdout, artifact, validation_errors)) => AdapterOutcome {
            ok,
            summary: if ok {
                "unica.mxl.validate completed with native spreadsheet validator".to_string()
            } else {
                "unica.mxl.validate failed in native spreadsheet validator".to_string()
            },
            changes: Vec::new(),
            warnings: Vec::new(),
            errors: validation_errors,
            artifacts: vec![artifact.display().to_string()],
            stdout: Some(stdout),
            stderr: Some(String::new()),
            command: None,
        },
        Err(error) => mxl_validation_error(error),
    }
}

fn mxl_validation_error(error: String) -> AdapterOutcome {
    AdapterOutcome {
        ok: false,
        summary: "unica.mxl.validate failed in native spreadsheet validator".to_string(),
        changes: Vec::new(),
        warnings: Vec::new(),
        errors: vec![error.clone()],
        artifacts: Vec::new(),
        stdout: None,
        stderr: Some(format!("{error}\n")),
        command: None,
    }
}

/// Both documented forms of the physical `TemplatePath` name the same
/// descriptor: the `Template.xml` itself, or the layout directory holding
/// `Ext/Template.xml`. Every reader the published description lists shares this
/// step, so the promise stays true for all of them instead of per-handler.
pub(crate) fn resolve_template_layout_path(path: &Path, cwd: &Path) -> PathBuf {
    let absolute = unica_operation_contract::input::absolutize(path.to_path_buf(), cwd);
    if absolute.is_dir() {
        absolute.join("Ext").join("Template.xml")
    } else {
        absolute
    }
}

/// `unica.mxl.info` publishes two addresses: the logical selector and
/// `TemplatePath`. The composite `ProcessorName`/`TemplateName`/`SrcDir` form
/// guessed a physical layout and could never be reached past the schema, so it
/// is gone rather than left as an unreachable branch.
///
/// The physical address must still accept both documented forms — the
/// descriptor file and the layout directory holding it — the way
/// `resolve_mxl_validate_path` already does.
pub(crate) fn resolve_mxl_info_path(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> Result<PathBuf, String> {
    if let Some(selection) =
        logical_selection(args, context, AttachedResource::Template, TEMPLATE_KINDS)
    {
        return selection
            .map(|selection| selection.resource_path)
            .map_err(|failure| failure.to_string());
    }
    let path = unica_operation_contract::input::path_arg(
        args,
        &["templatePath", "TemplatePath", "path", "Path"],
    )
    .ok_or_else(|| "Specify -TemplatePath".to_string())?;
    Ok(resolve_template_layout_path(&path, &context.cwd))
}

pub(crate) fn resolve_mxl_info_target(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> Result<ResolvedReadTarget, String> {
    if let Some(selection) =
        logical_selection(args, context, AttachedResource::Template, TEMPLATE_KINDS)
    {
        return selection.map_err(|failure| failure.to_string());
    }
    let template_path = resolve_mxl_info_path(args, context)?;
    physical_selection(&template_path, context, AttachedResource::Template)
        .map_err(|failure| failure.to_string())
}

pub(crate) fn resolve_mxl_validate_path(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> Result<PathBuf, String> {
    if let Some(selection) =
        logical_selection(args, context, AttachedResource::Template, TEMPLATE_KINDS)
    {
        return selection
            .map(|selection| selection.resource_path)
            .map_err(|failure| failure.to_string());
    }
    let mut template_path = if let Some(path) = unica_operation_contract::input::path_arg(
        args,
        &["templatePath", "TemplatePath", "path", "Path"],
    ) {
        path
    } else {
        let processor_name = unica_operation_contract::input::required_string(
            args,
            &["processorName", "ProcessorName"],
            "ProcessorName",
        )?;
        let template_name = unica_operation_contract::input::required_string(
            args,
            &["templateName", "TemplateName"],
            "TemplateName",
        )?;
        let src_dir = unica_operation_contract::input::path_arg(args, &["srcDir", "SrcDir"])
            .unwrap_or_else(|| PathBuf::from("src"));
        src_dir
            .join(processor_name)
            .join("Templates")
            .join(template_name)
            .join("Ext")
            .join("Template.xml")
    };

    template_path = unica_operation_contract::input::absolutize(template_path, &context.cwd);

    if template_path.is_dir() {
        template_path = template_path.join("Ext").join("Template.xml");
    }
    if !template_path.exists()
        && template_path.file_name().and_then(|value| value.to_str()) == Some("Template.xml")
    {
        let candidate = template_path
            .parent()
            .unwrap_or_else(|| Path::new(""))
            .join("Ext")
            .join("Template.xml");
        if candidate.exists() {
            template_path = candidate;
        }
    }
    if !template_path.exists()
        && template_path
            .extension()
            .and_then(|value| value.to_str())
            .map(|ext| ext.eq_ignore_ascii_case("xml"))
            .unwrap_or(false)
    {
        if let Some(stem) = template_path.file_stem().and_then(|value| value.to_str()) {
            let candidate = template_path
                .parent()
                .unwrap_or_else(|| Path::new(""))
                .join(stem)
                .join("Ext")
                .join("Template.xml");
            if candidate.exists() {
                template_path = candidate;
            }
        }
    }

    if !template_path.exists() {
        return Err(format!("File not found: {}", template_path.display()));
    }

    Ok(template_path)
}

pub(crate) fn invoke_read(
    operation: &str,
    _tool_name: &str,
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> Option<Result<AdapterOutcome, String>> {
    match operation {
        // Typed answer; the data reaches the envelope through typed_result.rs.
        "mxl-info" => Some(Ok(analyze_mxl_info(
            args,
            context,
            &WorkspaceSupportStateReader::new(context),
        )
        .outcome)),
        "mxl-validate" => Some(Ok(validate_mxl(args, context))),
        _ => None,
    }
}

#[cfg(test)]
#[path = "../../../tests/platform/template_read_selector_bridge.rs"]
pub(super) mod mxl_read_selector_bridge_tests;
#[cfg(test)]
#[path = "../../../tests/platform/mxl_composition.rs"]
pub(crate) mod tests;
