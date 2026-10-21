use serde_json::{Map, Value};
use unica_address::address::{NodeKind, QualifiedAddress};
use unica_operations::recovery::{
    prefix_through, resolve_named, MemberPathLookup, NamedNode, PreparedRedirect, RetiredArgument,
};
pub const RETIRED_ARGUMENTS: &[RetiredArgument] = &[
    RetiredArgument {
        op: "query.set",
        key: "values.dataSet",
        address: "{dataSet}.Query",
        retired: true,
    },
    RetiredArgument {
        op: "query.patch",
        key: "values.dataSet",
        address: "{dataSet}.Query",
        retired: true,
    },
    RetiredArgument {
        op: "structure.set",
        key: "values.variant",
        address: "{at^Template}.Setting.{value}",
        retired: true,
    },
    RetiredArgument {
        op: "structure.patch",
        key: "values.variant",
        address: "{at^Template}.Setting.{value}.Item.{name}",
        retired: true,
    },
    RetiredArgument {
        op: "variant.set",
        key: "values.name",
        address: "{at^Template}.Setting.{value}",
        retired: true,
    },
    RetiredArgument {
        op: "variant.remove",
        key: "values.name",
        address: "{at^Template}.Setting.{value}",
        retired: true,
    },
    RetiredArgument {
        op: "parameter.set",
        key: "values.name",
        address: "{at^Template}.Parameter.{value}",
        retired: true,
    },
    RetiredArgument {
        op: "parameter.rename",
        key: "values.name",
        address: "{at^Template}.Parameter.{value}",
        retired: true,
    },
    RetiredArgument {
        op: "field.set",
        key: "values.field",
        address: "{at^DataSet}.Field.{value}",
        retired: true,
    },
    RetiredArgument {
        op: "field.set",
        key: "values.dataPath",
        address: "{at^DataSet}.Field.{value}",
        retired: true,
    },
    RetiredArgument {
        op: "field.set",
        key: "values.name",
        address: "{at^DataSet}.Field.{value}",
        retired: true,
    },
    RetiredArgument {
        op: "fieldRole.set",
        key: "values.field",
        address: "{at^DataSet}.Field.{value}",
        retired: true,
    },
    RetiredArgument {
        op: "fieldRole.set",
        key: "values.dataPath",
        address: "{at^DataSet}.Field.{value}",
        retired: true,
    },
    RetiredArgument {
        op: "fieldRole.set",
        key: "values.name",
        address: "{at^DataSet}.Field.{value}",
        retired: true,
    },
    RetiredArgument {
        op: "dataSource.set",
        key: "values.name",
        address: "{at^Template}.DataSource.{value}",
        retired: true,
    },
    RetiredArgument {
        op: "dataSource.remove",
        key: "values.name",
        address: "{at^Template}.DataSource.{value}",
        retired: true,
    },
    RetiredArgument {
        op: "calculatedField.set",
        key: "values.name",
        address: "{at^Template}.CalculatedField.{value}",
        retired: true,
    },
    RetiredArgument {
        op: "calculatedField.remove",
        key: "values.name",
        address: "{at^Template}.CalculatedField.{value}",
        retired: true,
    },
    RetiredArgument {
        op: "total.set",
        key: "values.field",
        address: "{at^Template}.Total.{value}",
        retired: true,
    },
    RetiredArgument {
        op: "total.remove",
        key: "values.name",
        address: "{at^Template}.Total.{value}",
        retired: true,
    },
    RetiredArgument {
        op: "filter.add",
        key: "items[].group",
        address: "{item}",
        retired: true,
    },
    RetiredArgument {
        op: "filter.set",
        key: "values.group",
        address: "{item}",
        retired: true,
    },
    RetiredArgument {
        op: "filter.remove",
        key: "values.group",
        address: "{item}",
        retired: true,
    },
    RetiredArgument {
        op: "filter.clear",
        key: "values.group",
        address: "{item}",
        retired: true,
    },
    RetiredArgument {
        op: "selection.add",
        key: "items[].group",
        address: "{item}",
        retired: true,
    },
    RetiredArgument {
        op: "selection.set",
        key: "values.group",
        address: "{item}",
        retired: true,
    },
    RetiredArgument {
        op: "selection.remove",
        key: "values.group",
        address: "{item}",
        retired: true,
    },
    RetiredArgument {
        op: "selection.clear",
        key: "values.group",
        address: "{item}",
        retired: true,
    },
    RetiredArgument {
        op: "order.add",
        key: "items[].group",
        address: "{item}",
        retired: true,
    },
    RetiredArgument {
        op: "order.set",
        key: "values.group",
        address: "{item}",
        retired: true,
    },
    RetiredArgument {
        op: "order.remove",
        key: "values.group",
        address: "{item}",
        retired: true,
    },
    RetiredArgument {
        op: "order.clear",
        key: "values.group",
        address: "{item}",
        retired: true,
    },
    RetiredArgument {
        op: "conditionalAppearance.add",
        key: "items[].group",
        address: "{item}",
        retired: true,
    },
    RetiredArgument {
        op: "conditionalAppearance.remove",
        key: "values.group",
        address: "{item}",
        retired: true,
    },
    RetiredArgument {
        op: "conditionalAppearance.clear",
        key: "values.group",
        address: "{item}",
        retired: true,
    },
    RetiredArgument {
        op: "outputParameter.set",
        key: "values.group",
        address: "{item}",
        retired: true,
    },
    RetiredArgument {
        op: "outputParameter.remove",
        key: "values.group",
        address: "{item}",
        retired: true,
    },
    RetiredArgument {
        op: "structure.add",
        key: "items[].parent",
        address: "{item}",
        retired: true,
    },
    RetiredArgument {
        op: "structure.patch",
        key: "values.name",
        address: "{item}",
        retired: true,
    },
    RetiredArgument {
        op: "structure.remove",
        key: "values.name",
        address: "{item}",
        retired: true,
    },
];
pub fn prepare(
    template: &str,
    at: &QualifiedAddress,
    args: &Map<String, Value>,
    value: &str,
    members: MemberPathLookup<'_>,
) -> Option<PreparedRedirect> {
    let _ = args;
    let named = if template.contains("{dataSet}") {
        Some(("{dataSet}", named_data_set(at, value, members)?))
    } else if template.contains("{item}") {
        let owner = prefix_through(at, |kind| kind == NodeKind::Setting)?;
        Some(("{item}", resolve_named(&owner, "Item", value, members)?))
    } else {
        None
    };
    Some(PreparedRedirect {
        template: template.to_string(),
        named,
    })
}
/// Набор схемы, названный значением снятого `dataSet`: сегмент адреса вызова
/// с этим именем; иначе найденный в схеме макета; иначе набор верхнего уровня.
/// Несколько наборов с этим именем (члены разных объединений) — ветвь
/// наборов макета и их адреса (#1357).
pub fn named_data_set(
    at: &QualifiedAddress,
    name: &str,
    data_sets: MemberPathLookup<'_>,
) -> Option<NamedNode> {
    let segments = at.segments();
    if let Some(index) = segments
        .iter()
        .rposition(|segment| segment.kind() == NodeKind::DataSet && segment.name() == Some(name))
    {
        return Some(NamedNode::Found(QualifiedAddress {
            line_range: None,
            source_set: at.source_set().to_string(),
            segments: segments[..=index].to_vec(),
        }));
    }
    let template = prefix_through(at, |kind| kind == NodeKind::Template)?;
    resolve_named(&template, "DataSet", name, data_sets)
}

use unica_source_contract::context::{OperationSite, RegistryContext};
use unica_source_contract::route::RouteClass;
pub fn query_alias(
    operation: crate::operations::Primitive,
    site: &OperationSite<'_>,
    context: RegistryContext,
    data_sets: MemberPathLookup<'_>,
) -> Option<QualifiedAddress> {
    let segments = site.segments();
    let position = segments
        .iter()
        .position(|segment| segment.kind() == NodeKind::Query && segment.name().is_some())?;
    if site.route() != RouteClass::Dcs
        || position == 0
        || segments[position - 1].kind() != NodeKind::Template
    {
        return None;
    }
    let name = segments[position].name()?;
    let template = QualifiedAddress {
        line_range: None,
        source_set: site.at().source_set().to_string(),
        segments: segments[..position].to_vec(),
    };
    let NamedNode::Found(data_set) = named_data_set(&template, name, data_sets)? else {
        return None;
    };
    [".Query", ""].into_iter().find_map(|suffix| {
        let candidate = QualifiedAddress::parse(&format!("{data_set}{suffix}")).ok()?;
        context
            .site(&candidate, site.source())
            .is_some_and(|target| operation.descriptor().applies_at(&target))
            .then_some(candidate)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::operations::Primitive;
    use serde_json::json;
    use unica_operations::recovery::NamedLookup;

    #[test]
    fn named_nested_structure_item_uses_the_setting_owner_without_forms() {
        let at = QualifiedAddress::parse("main:Report.Sales.Template.Schema.Setting.Main").unwrap();
        let lookup = |owner: &QualifiedAddress, name: &str| {
            assert_eq!(owner, &at);
            assert_eq!(name, "Inner");
            NamedLookup::Found(vec!["Table".to_string(), "Inner".to_string()])
        };
        let entry = RETIRED_ARGUMENTS
            .iter()
            .find(|entry| entry.op == "filter.add" && entry.key == "items[].group")
            .unwrap()
            .bind(Primitive::FilterAdd.descriptor());
        let args = json!({"items":[{"field":"Amount", "group":"Inner"}]});
        let redirect = entry
            .redirect_with(&at, args.as_object().unwrap(), &lookup)
            .unwrap();
        assert_eq!(redirect.location, "items[0].group");
        assert_eq!(
            redirect.at.to_string(),
            "main:Report.Sales.Template.Schema.Setting.Main.Item.Table.Item.Inner"
        );
    }
}
