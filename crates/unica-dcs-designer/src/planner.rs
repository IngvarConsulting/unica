//! Lossless DCS writes and final source-reference validation in a proposal.
use unica_address::source_target::MetadataAddress;
use unica_dcs::edit::DcsEdit;
use unica_dcs::operations::Primitive;
use unica_source_contract::events::{DomainEvent, DomainEventKind};
use unica_source_contract::proposal::{
    ApplyPlanError, ApplyPlanErrorKind, ApplyProposal, ApplySourceAuthority, ProvisionalApplyEffect,
};
use unica_source_contract::SourceSetKind;
use unica_source_designer::layout::attached_resource_relative;

/// Applies one DCS edit to `xml_text`. The default language of the source
/// set is read only when the edit writes a plain-string multilingual value,
/// so edits without such text do not depend on `Configuration.xml` (#1257).
fn apply_dcs_edit(
    staged: &mut ApplyProposal<'_>,
    authority: &dyn ApplySourceAuthority,
    xml_text: &mut String,
    edit: &DcsEdit,
    op_index: usize,
) -> Result<(), ApplyPlanError> {
    use crate::primitives::{apply, ApplyError};
    let apply_with = |xml_text: &mut String, lang: Option<&str>| {
        apply(xml_text, &edit.intent, &edit.target, lang)
    };
    let result = match apply_with(xml_text, None) {
        Err(ApplyError::TextLanguageRequired) => {
            let lang = unica_source_designer::language::staged_text_language(
                staged,
                authority.source_kind(),
                op_index,
            )?;
            apply_with(xml_text, Some(&lang))
        }
        result => result,
    };
    result.map_err(|error| {
        let path = error
            .argument_path()
            .map(|path| format!("ops[{op_index}].args.{path}"))
            .unwrap_or_else(|| format!("ops[{op_index}].args"));
        ApplyPlanError::new(ApplyPlanErrorKind::BadValue, error.to_string()).at_path(path)
    })
}

pub(crate) fn stage_dcs_edit(
    staged: &mut ApplyProposal<'_>,
    authority: &dyn ApplySourceAuthority,
    edit: &DcsEdit,
    op_index: usize,
    provisional: &mut Vec<ProvisionalApplyEffect>,
) -> Result<(), ApplyPlanError> {
    let at_path = format!("ops[{op_index}].args.at");
    let relative = attached_resource_relative(
        &edit.template,
        crate::codec::SCHEMA_RESOURCE,
        authority.source_kind(),
    )
    .map_err(|message| {
        ApplyPlanError::new(ApplyPlanErrorKind::BadValue, message).at_path(at_path.clone())
    })?;
    let preimage = staged
        .read(&relative)
        .map_err(|error| ApplyPlanError::staging(error, at_path.clone()))?
        .ok_or_else(|| {
            ApplyPlanError::new(
                ApplyPlanErrorKind::NotFound,
                "the data composition schema template was not found",
            )
            .at_path(at_path.clone())
        })?;
    let (bom, body) = match preimage.strip_prefix(b"\xef\xbb\xbf") {
        Some(body) => (&b"\xef\xbb\xbf"[..], body),
        None => (&b""[..], preimage.as_slice()),
    };
    let mut xml_text = String::from_utf8(body.to_vec()).map_err(|_| {
        ApplyPlanError::new(
            ApplyPlanErrorKind::InvalidSource,
            "the data composition schema is not UTF-8",
        )
        .at_path(at_path.clone())
    })?;
    {
        let document = roxmltree::Document::parse(&xml_text).map_err(|error| {
            ApplyPlanError::new(
                ApplyPlanErrorKind::InvalidSource,
                format!("the data composition schema is not well-formed XML: {error}"),
            )
            .at_path(at_path.clone())
        })?;
        crate::schema::require_dcs_root(document.root_element()).map_err(|error| {
            ApplyPlanError::new(ApplyPlanErrorKind::InvalidSource, error).at_path(at_path.clone())
        })?;
    }
    let original = xml_text.clone();
    apply_dcs_edit(staged, authority, &mut xml_text, edit, op_index)?;
    if xml_text == original {
        return Ok(());
    }
    let mut postimage = Vec::with_capacity(bom.len() + xml_text.len());
    postimage.extend_from_slice(bom);
    postimage.extend_from_slice(xml_text.as_bytes());
    staged
        .replace(&relative, &preimage, postimage)
        .map_err(|error| ApplyPlanError::staging(error, at_path))?;
    provisional.push(ProvisionalApplyEffect::single(
        relative,
        DomainEvent::new(
            DomainEventKind::DcsChanged,
            edit.template.as_str().to_string(),
        ),
        op_index,
    ));
    Ok(())
}

/// Discharge removed-source obligations against the complete request image,
/// after all family runs and before request-level effect reconciliation.
pub(crate) fn validate_final_dcs_resources<'a>(
    staged: &mut ApplyProposal<'_>,
    source_kind: SourceSetKind,
    operations: impl IntoIterator<Item = (usize, &'a DcsEdit)>,
) -> Result<(), ApplyPlanError> {
    let mut templates =
        std::collections::BTreeMap::<String, (MetadataAddress, Vec<(String, usize)>)>::new();
    for (op_index, edit) in operations {
        if !matches!(edit.intent.operation(), Primitive::DataSourceRemove) {
            continue;
        }
        let name = edit
            .target
            .terminal
            .as_ref()
            .expect("the typed source removal has an addressed DataSource");
        let (_, removed) = templates
            .entry(edit.template.as_str().to_string())
            .or_insert_with(|| (edit.template.clone(), Vec::new()));
        if !removed.iter().any(|(previous, _)| previous == name) {
            removed.push((name.clone(), op_index));
        }
    }
    let mut first_dangling = None;
    for (template, removed) in templates.into_values() {
        let first_index = removed[0].1;
        let at_path = format!("ops[{first_index}].args.at");
        let relative =
            attached_resource_relative(&template, crate::codec::SCHEMA_RESOURCE, source_kind)
                .map_err(|message| {
                    ApplyPlanError::new(ApplyPlanErrorKind::BadValue, message)
                        .at_path(at_path.clone())
                })?;
        let bytes = staged
            .read(&relative)
            .map_err(|error| ApplyPlanError::staging(error, at_path.clone()))?
            .ok_or_else(|| {
                ApplyPlanError::new(
                    ApplyPlanErrorKind::NotFound,
                    "the data composition schema template was not found",
                )
                .at_path(at_path.clone())
            })?;
        let body = bytes.strip_prefix(b"\xef\xbb\xbf").unwrap_or(&bytes);
        let text = std::str::from_utf8(body).map_err(|_| {
            ApplyPlanError::new(
                ApplyPlanErrorKind::InvalidSource,
                "the data composition schema is not UTF-8",
            )
            .at_path(at_path.clone())
        })?;
        let document = roxmltree::Document::parse(text).map_err(|error| {
            ApplyPlanError::new(
                ApplyPlanErrorKind::InvalidSource,
                format!("the data composition schema is not well-formed XML: {error}"),
            )
            .at_path(at_path.clone())
        })?;
        let root = document.root_element();
        crate::schema::require_dcs_root(root).map_err(|message| {
            ApplyPlanError::new(ApplyPlanErrorKind::InvalidSource, message).at_path(at_path)
        })?;
        for (name, op_index) in removed {
            use crate::primitives::{declares_data_source, references_data_source};
            if !declares_data_source(root, &name) && references_data_source(root, &name) {
                first_dangling =
                    Some(first_dangling.map_or(op_index, |previous: usize| previous.min(op_index)));
            }
        }
    }
    if let Some(op_index) = first_dangling {
        return Err(ApplyPlanError::new(
            ApplyPlanErrorKind::BadValue,
            "dataSource is still referenced by a dataset",
        )
        .at_path(format!("ops[{op_index}].args")));
    }
    Ok(())
}
