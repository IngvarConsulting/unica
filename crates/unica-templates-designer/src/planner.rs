//! Existing canonical spreadsheet proposal planning and area observation.
use crate::mxl_cells::{self, CellText, MxlEditError};
use unica_address::address::member_address;
use unica_address::source_target::MetadataAddress;
use unica_source_contract::events::{DomainEvent, DomainEventKind};
use unica_source_contract::proposal::{
    ApplyPlanError, ApplyPlanErrorKind, ApplyProposal, ApplySourceAuthority, ProvisionalApplyEffect,
};
use unica_source_designer::layout::attached_resource_relative;
use unica_templates::operations::MxlOperation;

#[derive(Debug, Clone)]
pub struct MxlEdit {
    pub template: MetadataAddress,
    /// Квалифицированный адрес макета для текста отказа.
    pub template_at: String,
    pub area: String,
    pub operation: MxlOperation,
    /// Путь аргументов области: `values` или `items[i]`.
    pub location: String,
    pub columns: Option<i64>,
    pub cells: Vec<CellText>,
}

pub fn stage_mxl_edit(
    staged: &mut ApplyProposal<'_>,
    authority: &dyn ApplySourceAuthority,
    edit: &MxlEdit,
    op_index: usize,
    provisional: &mut Vec<ProvisionalApplyEffect>,
) -> Result<(), ApplyPlanError> {
    let at_path = format!("ops[{op_index}].args.at");
    let values_path = edit.location.clone();
    let relative =
        attached_resource_relative(&edit.template, "Template.xml", authority.source_kind())
            .map_err(|message| {
                ApplyPlanError::new(ApplyPlanErrorKind::BadValue, message).at_path(at_path.clone())
            })?;
    let preimage = staged
        .read(&relative)
        .map_err(|error| ApplyPlanError::staging(error, at_path.clone()))?
        .ok_or_else(|| {
            ApplyPlanError::new(
                ApplyPlanErrorKind::NotFound,
                "the spreadsheet template (Ext/Template.xml) was not found",
            )
            .at_path(at_path.clone())
        })?;
    let (bom, body) = match preimage.strip_prefix(b"\xef\xbb\xbf") {
        Some(body) => (&b"\xef\xbb\xbf"[..], body),
        None => (&b""[..], preimage.as_slice()),
    };
    let mut text = String::from_utf8(body.to_vec()).map_err(|_| {
        ApplyPlanError::new(
            ApplyPlanErrorKind::InvalidSource,
            "the spreadsheet template is not UTF-8",
        )
        .at_path(at_path.clone())
    })?;
    let original = text.clone();
    let written = match edit.operation {
        MxlOperation::Set => {
            mxl_cells::set_area_cells(&mut text, &edit.area, edit.columns, &edit.cells)
        }
        MxlOperation::AreaAdd => {
            mxl_cells::add_area(&mut text, &edit.area, edit.columns, &edit.cells)
        }
    };
    written.map_err(|error| match error {
        MxlEditError::AreaMissing => ApplyPlanError::new(
            ApplyPlanErrorKind::NotFound,
            format!(
                "the template has no area `{}`; create it with area.add at `{}`",
                edit.area, edit.template_at
            ),
        )
        .at_path(at_path.clone()),
        MxlEditError::AreaSpelling(existing) => ApplyPlanError::new(
            ApplyPlanErrorKind::NotFound,
            format!(
                "the template has no area `{}`; its area is spelled `{existing}`: address `{}`",
                edit.area,
                member_address(&format!("{}.Area", edit.template_at), &existing)
            ),
        )
        .at_path(at_path.clone()),
        MxlEditError::AreaExists(existing) => ApplyPlanError::new(
            ApplyPlanErrorKind::BadValue,
            format!(
                "the template already has area `{existing}`; change its cells with mxl.set at `{}`",
                member_address(&format!("{}.Area", edit.template_at), &existing)
            ),
        )
        .at_path(format!("{values_path}.name")),
        MxlEditError::RowOutsideArea { key, rows } => ApplyPlanError::new(
            ApplyPlanErrorKind::BadValue,
            format!(
                "`{key}` is below area `{}` of {rows} rows; an area grows only when it is the last one and no row below it is used, add a new area with area.add at `{}`",
                edit.area, edit.template_at
            ),
        )
        .at_path(format!("{values_path}.cells.{key}")),
        MxlEditError::Cell { key, message } => {
            ApplyPlanError::new(ApplyPlanErrorKind::BadValue, message)
                .at_path(format!("{values_path}.cells.{key}"))
        }
        MxlEditError::Source(message) => {
            ApplyPlanError::new(ApplyPlanErrorKind::InvalidSource, message).at_path(at_path.clone())
        }
    })?;
    if text == original {
        return Ok(());
    }
    let mut postimage = Vec::with_capacity(bom.len() + text.len());
    postimage.extend_from_slice(bom);
    postimage.extend_from_slice(text.as_bytes());
    staged
        .replace(&relative, &preimage, postimage)
        .map_err(|error| ApplyPlanError::staging(error, at_path))?;
    provisional.push(ProvisionalApplyEffect::single(
        relative,
        DomainEvent::new(
            DomainEventKind::MxlChanged,
            edit.template.as_str().to_string(),
        ),
        op_index,
    ));
    Ok(())
}

/// Есть ли именованная область в стадированном табличном документе.
/// `None` — документ не читается.
pub fn spreadsheet_area_exists(bytes: &[u8], name: &str) -> Option<bool> {
    let text = std::str::from_utf8(bytes.strip_prefix(b"\xef\xbb\xbf").unwrap_or(bytes)).ok()?;
    let document = roxmltree::Document::parse(text).ok()?;
    Some(
        document
            .root_element()
            .children()
            .filter(|node| node.is_element() && node.tag_name().name() == "namedItem")
            .any(|node| {
                node.children().any(|part| {
                    part.is_element()
                        && part.tag_name().name() == "name"
                        && part.text().map(str::trim) == Some(name)
                })
            }),
    )
}
