//! Типизированная правка СКД: намерение операции и узлы схемы, которые
//! называет её логический адрес. Разбор не читает представление.
use crate::operations::{DcsIntent, Primitive};
use serde_json::{Map, Value};
use unica_address::address::{NodeKind, QualifiedAddress};
use unica_address::source_target::MetadataAddress;
use unica_source_contract::proposal::{ApplyPlanError, ApplyPlanErrorKind};

/// Узлы схемы под макетом, которые называет адрес правки.
#[derive(Debug, Clone)]
pub struct Target {
    pub datasets: Vec<String>,
    pub variant: String,
    /// Цепочка имён элементов структуры варианта: `Setting.V.Item.A.Item.B`
    /// даёт `["A", "B"]` (#1357).
    pub structure: Vec<String>,
    pub terminal: Option<String>,
}

/// A normalized basic operation and its captured logical target.
#[derive(Debug, Clone)]
pub struct DcsEdit {
    pub template: MetadataAddress,
    pub intent: DcsIntent,
    pub target: Target,
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
    let (template, rest) = unica_templates::address::template_owner(
        target,
        op_index,
        "DCS operations address a schema template: `Owner.Name.Template.Name[.DataSet.Name[.Query]|.Setting.Name[.Item.Name…]|.DataSource.Name|.CalculatedField.Name|.Total.Name]`",
    )?;
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

pub fn plan_edit(
    operation: Primitive,
    args: &Map<String, Value>,
    op_index: usize,
    source_set_name: &str,
) -> Result<DcsEdit, ApplyPlanError> {
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
    Ok(DcsEdit {
        template: target.template,
        intent,
        target: Target {
            datasets: target.datasets,
            variant: target.variant,
            structure: target.structure,
            terminal: target.terminal.map(|(_, name)| name),
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn plan(operation: &str, args: &Value) -> Result<DcsEdit, ApplyPlanError> {
        let operation = Primitive::named(operation).expect("a DCS primitive");
        plan_edit(operation, args.as_object().expect("object args"), 0, "main")
    }

    #[test]
    fn dcs_contract_refuses_ignored_arguments_instead_of_planning_another_edit() {
        let at = "main:Report.Versions.Template.Schema.DataSet.НаборДанных1";
        for (op, args, path) in [
            (
                "query.patch",
                json!({"at":at,"values":{"find":"1","replace":true}}),
                "values.replace",
            ),
            (
                "query.patch",
                json!({"at":at,"values":{"find":"1","once":"true"}}),
                "values.once",
            ),
            (
                "structure.set",
                json!({"at":at,"values":{"details":"false"}}),
                "values.details",
            ),
            (
                "structure.set",
                json!({"at":at,"values":{"groupBy":["Amount",42]}}),
                "values.groupBy[1]",
            ),
            (
                "query.set",
                json!({"at":at,"values":{"query":"ВЫБРАТЬ 1","typo":1}}),
                "values.typo",
            ),
            (
                "field.add",
                json!({"at":at,"items":[{"dataPath":"New","typo":1}]}),
                "items[0].typo",
            ),
            ("filter.clear", json!({"at":at,"typo":1}), "typo"),
        ] {
            let error = plan(op, &args)
                .expect_err("a malformed request must not silently become a different edit");
            assert_eq!(
                error.kind(),
                ApplyPlanErrorKind::BadValue,
                "{op}: {error:?}"
            );
            assert_eq!(error.path(), Some(format!("ops[0].args.{path}").as_str()));
        }
    }
    #[test]
    fn a_query_is_addressed_through_its_dataset_and_retired_spellings_refuse() {
        let schema = "main:Report.Versions.Template.Schema";
        for (op, at, values) in [
            (
                "query.set",
                "DataSet.НаборДанных1.Query",
                json!({"query": "ВЫБРАТЬ 1"}),
            ),
            (
                "query.patch",
                "DataSet.НаборДанных1.Query",
                json!({"find": "1"}),
            ),
        ] {
            let args = json!({"at": format!("{schema}.{at}"), "values": values});
            let Ok(plan) = plan(op, &args) else {
                panic!("{op} at {at} is a DCS plan");
            };
            assert_eq!(plan.target.datasets, vec!["НаборДанных1".to_string()]);
        }
        for (op, at, values, path, says) in [
            (
                "query.set",
                "Query.НаборДанных1",
                json!({"query": "ВЫБРАТЬ 1"}),
                "at",
                "DataSet.НаборДанных1.Query",
            ),
            (
                "dataSet.set",
                "DataSet.НаборДанных1.Query",
                json!({"autoFillFields": false}),
                "at",
                "not its Query",
            ),
            (
                "query.set",
                "DataSet.НаборДанных1",
                json!({"query": "ВЫБРАТЬ 1"}),
                "at",
                "DataSet.D.Query",
            ),
            (
                "query.set",
                "DataSet.НаборДанных1.Query",
                json!({"dataSet": "НаборДанных1", "query": "ВЫБРАТЬ 1"}),
                "values.dataSet",
                "unknown argument",
            ),
            (
                "structure.set",
                "Setting.Основной",
                json!({"variant": "Основной"}),
                "values.variant",
                "unknown argument",
            ),
        ] {
            let args = json!({"at": format!("{schema}.{at}"), "values": values});
            let error = plan(op, &args).expect_err("a retired spelling is refused");
            assert_eq!(error.kind(), ApplyPlanErrorKind::BadValue, "{op} at {at}");
            assert_eq!(
                error.path(),
                Some(format!("ops[0].args.{path}").as_str()),
                "{op} at {at}"
            );
            let message = error.to_string();
            assert!(message.contains(says), "{op} at {at}: {message}");
        }
    }
    #[test]
    fn typed_dcs_aliases_have_one_owner_and_preserve_primary_priority() {
        let at = "main:Report.Versions.Template.Schema.DataSet.НаборДанных1";
        let query_at = "main:Report.Versions.Template.Schema.DataSet.НаборДанных1.Query";
        let field = |item| plan("field.add", &json!({"at":at,"items":[item]}));
        for item in [
            json!({"name":"Amount"}),
            json!({"dataPath":"Amount","name":""}),
            json!({"dataPath":"Amount","name":"Wrong"}),
        ] {
            let edit = field(item).unwrap();
            let DcsIntent::FieldAdd(items) = edit.intent else {
                panic!("field.add keeps its typed intent");
            };
            assert_eq!(items[0].data_path, "Amount");
        }
        assert!(field(json!({"dataPath":"","name":"Amount"})).is_err());
        let query = |values| plan("query.set", &json!({"at":query_at,"values":values}));
        for values in [
            json!({"text":"ВЫБРАТЬ 1"}),
            json!({"query":"ВЫБРАТЬ 1","text":""}),
            json!({"query":"ВЫБРАТЬ 1","text":"ВЫБРАТЬ 2"}),
        ] {
            let edit = query(values).unwrap();
            let DcsIntent::QuerySet(query) = edit.intent else {
                panic!("query.set keeps its typed intent");
            };
            assert_eq!(query, "ВЫБРАТЬ 1");
        }
        assert!(query(json!({"query":"","text":"ВЫБРАТЬ 1"})).is_err());
    }
    #[test]
    fn dcs_field_removal_needs_the_field_in_the_address() {
        let error = plan(
            "field.remove",
            &json!({"at": "main:Report.Versions.Template.Schema.DataSet.НаборДанных1"}),
        )
        .unwrap_err();
        assert_eq!(error.kind(), ApplyPlanErrorKind::BadValue);
        assert_eq!(error.path(), Some("ops[0].args.at"));
    }

    #[test]
    fn typed_dcs_operands_do_not_pass_through_editor_tokens() {
        let edit = plan("field.add",&json!({"at":"main:Report.Versions.Template.Schema.DataSet.НаборДанных1","items":[{"dataPath":"Sum","title":"Итого [шт] @user"}]})).unwrap();
        let DcsIntent::FieldAdd(items) = edit.intent else {
            panic!("field.add keeps its typed intent");
        };
        assert_eq!(
            items[0].title.as_ref().unwrap().value(),
            &json!("Итого [шт] @user")
        );
    }

    #[test]
    fn full_schema_input_is_not_a_dcs_primitive() {
        assert!(Primitive::named("dcs.set").is_none());
    }
}
