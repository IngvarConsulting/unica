//! Canonical DCS proposal decoding, lossless writes and final source-reference validation.
use serde_json::{Map, Value};
use unica_address::address::{NodeKind, QualifiedAddress};
use unica_address::source_target::{MetadataAddress, PLATFORM_XML_8_3_27_FORMAT_2_20};
use unica_dcs::operations::{DcsIntent, Primitive};
use unica_source_contract::events::{DomainEvent, DomainEventKind};
use unica_source_contract::proposal::{
    ApplyPlanError, ApplyPlanErrorKind, ApplyProposal, ApplySourceAuthority, ProvisionalApplyEffect,
};
use unica_source_contract::SourceSetKind;
use unica_source_designer::layout::attached_resource_relative;

/// A normalized basic operation and its captured logical target.
#[derive(Debug, Clone)]
pub struct DcsPrimitivePlan {
    pub template: MetadataAddress,
    pub intent: DcsIntent,
    pub target: crate::primitives::Target,
}

/// The DCS parts of a logical address: the template owner, plus the dataset,
/// settings variant and its structure items, or the field, parameter, data
/// source, calculated field or total the address descends into, and whether
/// it ends at the dataset's unnamed `Query`.
pub struct DcsTarget {
    pub template: MetadataAddress,
    pub datasets: Vec<String>,
    pub variant: String,
    pub structure: Vec<String>,
    pub terminal: Option<(NodeKind, String)>,
    pub query: bool,
}

pub fn dcs_target(target: &QualifiedAddress, op_index: usize) -> Result<DcsTarget, ApplyPlanError> {
    let at_path = format!("ops[{op_index}].args.at");
    let bad = |message: &str| {
        ApplyPlanError::new(ApplyPlanErrorKind::BadValue, message.to_string())
            .at_path(at_path.clone())
    };
    let segments = target.segments();
    let [owner, template, rest @ ..] = segments else {
        return Err(bad(
            "DCS operations address a schema template: `Owner.Name.Template.Name[.DataSet.Name[.Query]|.Setting.Name[.Item.Name…]|.DataSource.Name|.CalculatedField.Name|.Total.Name]`",
        ));
    };
    if template.kind() != NodeKind::Template {
        return Err(bad(
            "DCS operations address a `Template` node of a report or object",
        ));
    }
    let owner_name = owner
        .name()
        .ok_or_else(|| bad("the template owner must be named"))?;
    let template_name = template
        .name()
        .ok_or_else(|| bad("the template must be named"))?;
    let template = MetadataAddress::parse(
        PLATFORM_XML_8_3_27_FORMAT_2_20,
        &format!(
            "{}.{owner_name}.Template.{template_name}",
            owner.kind().as_str()
        ),
    )
    .map_err(|error| bad(&error.to_string()))?;
    let mut datasets = Vec::new();
    let mut variant = String::new();
    let mut structure = Vec::new();
    let mut terminal = None;
    let mut query = false;
    for segment in rest {
        if terminal.is_some() || query {
            return Err(bad("a DCS terminal has no addressed children"));
        }
        if segment.kind() == NodeKind::Query {
            // Запрос — узел набора без имени: `DataSet.D.Query`. Прежний
            // псевдоним набора `Template.T.Query.D` не принимается (#1353).
            match segment.name() {
                None if !datasets.is_empty() && variant.is_empty() => {
                    query = true;
                    continue;
                }
                Some(name) => {
                    return Err(bad(&format!(
                    "a query is addressed through its dataset: `Template.T.DataSet.{name}.Query`"
                )))
                }
                None => {
                    return Err(bad(
                        "a query is addressed through its dataset: `DataSet.D.Query`",
                    ))
                }
            }
        }
        let name = segment
            .name()
            .ok_or_else(|| bad("DCS address segments below the template must be named"))?
            .to_string();
        let at_schema = datasets.is_empty() && variant.is_empty();
        match segment.kind() {
            NodeKind::DataSet if variant.is_empty() => datasets.push(name),
            NodeKind::Setting if at_schema => variant = name,
            // Элемент структуры варианта: цепочка `Item.<имя>` (#1357).
            NodeKind::Item if !variant.is_empty() => structure.push(name),
            NodeKind::Field if !datasets.is_empty() && variant.is_empty() => {
                terminal = Some((NodeKind::Field, name))
            }
            NodeKind::Parameter if variant.is_empty() => {
                terminal = Some((NodeKind::Parameter, name))
            }
            kind @ (NodeKind::DataSource | NodeKind::CalculatedField | NodeKind::Total)
                if at_schema =>
            {
                terminal = Some((kind, name))
            }
            _ => return Err(bad(
                "DCS address must select a Template, nested DataSet, Setting with its Item chain, or terminal Field/Parameter/DataSource/CalculatedField/Total",
            )),
        }
    }
    Ok(DcsTarget {
        template,
        datasets,
        variant,
        structure,
        terminal,
        query,
    })
}

pub fn parse_dcs_operation(
    operation: Primitive,
    args: &Map<String, Value>,
    op_index: usize,
    source_set_name: &str,
) -> Result<DcsPrimitivePlan, ApplyPlanError> {
    let intent = operation
        .decode(
            &Value::Object(args.clone()),
            &format!("ops[{op_index}].args"),
        )
        .map_err(|error| {
            ApplyPlanError::new(
                ApplyPlanErrorKind::BadValue,
                format!("{}: {}", error.path, error.message),
            )
            .at_path(error.path)
        })?;
    let address =
        unica_source_contract::proposal::input::qualified_target(args, op_index, source_set_name)?;
    let target = dcs_target(&address, op_index)?;
    let (query_operation, variant_operation, structure_operation, terminal_kind) = match operation {
        Primitive::QuerySet | Primitive::QueryPatch => (true, false, false, None),
        Primitive::VariantSet | Primitive::VariantRemove => (false, true, false, None),
        Primitive::StructurePatch | Primitive::StructureRemove => (false, false, true, None),
        Primitive::FieldRemove | Primitive::FieldSet | Primitive::FieldRoleSet => {
            (false, false, false, Some(NodeKind::Field))
        }
        Primitive::ParameterRemove | Primitive::ParameterSet | Primitive::ParameterRename => {
            (false, false, false, Some(NodeKind::Parameter))
        }
        Primitive::DataSourceSet | Primitive::DataSourceRemove => {
            (false, false, false, Some(NodeKind::DataSource))
        }
        Primitive::CalculatedFieldSet | Primitive::CalculatedFieldRemove => {
            (false, false, false, Some(NodeKind::CalculatedField))
        }
        Primitive::TotalSet | Primitive::TotalRemove => {
            (false, false, false, Some(NodeKind::Total))
        }
        Primitive::DataSourceAdd
        | Primitive::DataSetAdd
        | Primitive::DataSetSet
        | Primitive::DataSetRemove
        | Primitive::FieldAdd
        | Primitive::ParameterAdd
        | Primitive::ParameterReorder
        | Primitive::CalculatedFieldAdd
        | Primitive::TotalAdd
        | Primitive::VariantAdd
        | Primitive::FilterAdd
        | Primitive::FilterSet
        | Primitive::FilterRemove
        | Primitive::FilterClear
        | Primitive::SelectionAdd
        | Primitive::SelectionClear
        | Primitive::OrderAdd
        | Primitive::OrderClear
        | Primitive::DataParameterAdd
        | Primitive::DataParameterSet
        | Primitive::OutputParameterSet
        | Primitive::ConditionalAppearanceAdd
        | Primitive::ConditionalAppearanceClear
        | Primitive::StructureAdd
        | Primitive::StructureSet
        | Primitive::DataSetLinkAdd
        | Primitive::SelectionSet
        | Primitive::SelectionRemove
        | Primitive::OrderSet
        | Primitive::OrderRemove
        | Primitive::DataParameterRemove
        | Primitive::OutputParameterRemove
        | Primitive::ConditionalAppearanceRemove
        | Primitive::DataSetLinkSet
        | Primitive::DataSetLinkRemove => (false, false, false, None),
    };
    let name = operation.name();
    if !target.query && query_operation {
        return Err(ApplyPlanError::new(
            ApplyPlanErrorKind::BadValue,
            format!("{name} addresses the dataset's query: `DataSet.D.Query`"),
        )
        .at_path(format!("ops[{op_index}].args.at")));
    }
    if target.query && !query_operation {
        return Err(ApplyPlanError::new(
            ApplyPlanErrorKind::BadValue,
            format!("{name} addresses the dataset, not its Query"),
        )
        .at_path(format!("ops[{op_index}].args.at")));
    }
    if variant_operation
        && (target.variant.is_empty() || target.terminal.is_some() || !target.structure.is_empty())
    {
        return Err(ApplyPlanError::new(
            ApplyPlanErrorKind::BadValue,
            format!("{name} addresses a settings variant: `Template.T.Setting.S`"),
        )
        .at_path(format!("ops[{op_index}].args.at")));
    }
    if structure_operation && target.structure.is_empty() {
        return Err(ApplyPlanError::new(
            ApplyPlanErrorKind::BadValue,
            format!("{name} addresses a named structure item: `Template.T.Setting.S.Item.I`"),
        )
        .at_path(format!("ops[{op_index}].args.at")));
    }
    if let Some(expected) = terminal_kind {
        if !target
            .terminal
            .as_ref()
            .is_some_and(|(kind, _)| *kind == expected)
        {
            return Err(ApplyPlanError::new(
                ApplyPlanErrorKind::BadValue,
                format!("address a named {}", expected.as_str()),
            )
            .at_path(format!("ops[{op_index}].args.at")));
        }
    }
    Ok(DcsPrimitivePlan {
        template: target.template,
        intent,
        target: crate::primitives::Target {
            datasets: target.datasets,
            variant: target.variant,
            structure: target.structure,
            terminal: target.terminal.map(|(_, name)| name),
        },
    })
}

/// Applies one DCS edit to `xml_text`. The default language of the source
/// set is read only when the edit writes a plain-string multilingual value,
/// so edits without such text do not depend on `Configuration.xml` (#1257).
fn apply_dcs_edit(
    staged: &mut ApplyProposal<'_>,
    authority: &dyn ApplySourceAuthority,
    xml_text: &mut String,
    edit: &DcsPrimitivePlan,
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

pub fn stage_dcs_edit(
    staged: &mut ApplyProposal<'_>,
    authority: &dyn ApplySourceAuthority,
    edit: &DcsPrimitivePlan,
    op_index: usize,
    provisional: &mut Vec<ProvisionalApplyEffect>,
) -> Result<(), ApplyPlanError> {
    let at_path = format!("ops[{op_index}].args.at");
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
pub fn validate_final_dcs_resources<'a>(
    staged: &mut ApplyProposal<'_>,
    source_kind: SourceSetKind,
    operations: impl IntoIterator<Item = (usize, &'a DcsPrimitivePlan)>,
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
        let relative = attached_resource_relative(&template, "Template.xml", source_kind).map_err(
            |message| {
                ApplyPlanError::new(ApplyPlanErrorKind::BadValue, message).at_path(at_path.clone())
            },
        )?;
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
