use serde_json::{Map, Value};
use unica_address::address::{escape_name, NodeKind, QualifiedAddress};
use unica_operations::recovery::{
    prefix_through, resolve_named, MemberPathLookup, PreparedRedirect, RetiredArgument,
};
pub const RETIRED_ARGUMENTS: &[RetiredArgument] = &[
    RetiredArgument {
        op: "item.add",
        key: "items[].into",
        address: "{item}",
        retired: true,
    },
    RetiredArgument {
        op: "item.remove",
        key: "items[].name",
        address: "{item}",
        retired: true,
    },
    RetiredArgument {
        op: "item.remove",
        key: "items[]",
        address: "{item}",
        retired: true,
    },
    RetiredArgument {
        op: "item.remove",
        key: "values.name",
        address: "{item}",
        retired: true,
    },
    RetiredArgument {
        op: "event.bind",
        key: "values.element",
        address: "{item}.Event.{event}",
        retired: true,
    },
    RetiredArgument {
        op: "event.bind",
        key: "values.element",
        address: "{item}.Event.{name}",
        retired: true,
    },
    RetiredArgument {
        op: "event.bind",
        key: "values.event",
        address: "{at-Event}.Event.{event}",
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
    let template = if template.contains("{event}") {
        template.replace("{event}", &escape_name(&named_form_event(at, args)?))
    } else {
        template.to_string()
    };
    let template = template.replace("{at-Event}", &without_event(at).to_string());
    let named = if template.contains("{item}") {
        let owner = prefix_through(at, |kind| kind == NodeKind::Form)?;
        Some(("{item}", resolve_named(&owner, "Item", value, members)?))
    } else {
        None
    };
    Some(PreparedRedirect { template, named })
}
/// Событие формы для снятого аргумента `event.bind`: `values.event` в имени
/// реестра, иначе событие самого адреса вызова.
fn named_form_event(at: &QualifiedAddress, args: &Map<String, Value>) -> Option<String> {
    if let Some(event) = args
        .get("values")
        .and_then(|values| values.get("event"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|event| !event.is_empty())
    {
        return Some(crate::canonical_form_event_name(event).to_string());
    }
    at.segments()
        .last()
        .filter(|segment| segment.kind() == NodeKind::Event)
        .and_then(|segment| segment.name())
        .map(str::to_string)
}

/// Адрес вызова без последнего сегмента `Event`: владелец события.
fn without_event(at: &QualifiedAddress) -> QualifiedAddress {
    match at.segments() {
        [owner @ .., last] if last.kind() == NodeKind::Event && !owner.is_empty() => {
            QualifiedAddress {
                line_range: None,
                source_set: at.source_set().to_string(),
                segments: owner.to_vec(),
            }
        }
        _ => at.clone(),
    }
}

pub const RETIRED_OPERATIONS: &[unica_operations::recovery::RetiredOperation] = &[
    unica_operations::recovery::RetiredOperation::new(
        "form.create",
        crate::operations::FormOperation::Add.descriptor(),
        Some(NodeKind::Form),
        "`items: [{name, type}]` names the form and its type",
    ),
    unica_operations::recovery::RetiredOperation::new(
        "element.add",
        crate::operations::FormOperation::ItemAdd.descriptor(),
        None,
        "the arguments are unchanged",
    ),
    unica_operations::recovery::RetiredOperation::new(
        "element.remove",
        crate::operations::FormOperation::ItemRemove.descriptor(),
        None,
        "the arguments are unchanged",
    ),
    unica_operations::recovery::RetiredOperation::new(
        "formAttribute.add",
        crate::operations::FormOperation::AttributeAdd.descriptor(),
        None,
        "the arguments are unchanged",
    ),
    unica_operations::recovery::RetiredOperation::new(
        "formCommand.add",
        crate::operations::FormOperation::CommandAdd.descriptor(),
        None,
        "the arguments are unchanged",
    ),
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::operations::FormOperation;
    use serde_json::json;
    use unica_operations::recovery::NamedLookup;

    #[test]
    fn named_nested_form_item_uses_the_form_owner_and_keeps_the_full_path() {
        let at = QualifiedAddress::parse("main:Document.Order.Form.Main").unwrap();
        let lookup = |owner: &QualifiedAddress, name: &str| {
            assert_eq!(owner, &at);
            assert_eq!(name, "Inner");
            NamedLookup::Found(vec!["Header".to_string(), "Inner".to_string()])
        };
        let entry = RETIRED_ARGUMENTS
            .iter()
            .find(|entry| entry.op == "item.add")
            .unwrap()
            .bind(FormOperation::ItemAdd.descriptor());
        let args = json!({"items":[{"name":"New", "into":"Inner"}]});
        let redirect = entry
            .redirect_with(&at, args.as_object().unwrap(), &lookup)
            .unwrap();
        assert_eq!(redirect.location, "items[0].into");
        assert_eq!(
            redirect.at.to_string(),
            "main:Document.Order.Form.Main.Item.Header.Item.Inner"
        );
    }
}
