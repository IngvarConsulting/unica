use super::*;
use items::METADATA_ITEM_BRANCH_FIELDS;
use unica_source_designer::registration::platform_child_object_order;

pub(crate) struct BranchFact<'a> {
    pub(crate) kind: NodeKind,
    pub(crate) count: usize,
    pub(crate) members: &'a [Value],
}

fn members(value: Option<&Value>) -> &[Value] {
    value.and_then(Value::as_array).map_or(&[], Vec::as_slice)
}

/// Build the expected rows from the platform declarations and the typed
/// observation, independently of the addresses emitted by the projector.
pub(crate) fn owner_facts<'a>(
    owner: &QualifiedAddress,
    payload: &'a Value,
) -> (Vec<BranchFact<'a>>, Option<String>) {
    let kind = owner.segments().last().unwrap().kind();
    let declared = platform_child_object_order(kind.as_str()).unwrap_or_default();
    let order = metadata_branch_kinds();
    for tag in declared {
        let child = NodeKind::parse(tag).unwrap_or_else(|error| panic!("{owner}: {error}"));
        assert!(
            order.contains(&child),
            "{owner}: platform declares {tag}, but the metadata branch map has no row"
        );
    }
    let addressing_issue = task_addressing_applies(owner, payload)
        .then(|| task_addressing_issue(payload))
        .flatten();
    let mut facts = Vec::new();
    for &branch in order {
        if branch == NodeKind::AddressingAttribute
            && (!task_addressing_applies(owner, payload) || addressing_issue.is_some())
        {
            continue;
        }
        let observed = members(metadata_collection(payload, branch));
        let count = if branch == NodeKind::PredefinedItem {
            payload
                .pointer("/predefinedItems/total")
                .and_then(Value::as_u64)
                .map_or(observed.len(), |total| usize::try_from(total).unwrap())
        } else {
            observed.len()
        };
        if count != 0 || declared.contains(&branch.as_str()) {
            facts.push(BranchFact {
                kind: branch,
                count,
                members: observed,
            });
        }
    }
    if kind == NodeKind::WsReference {
        if let Some(rows) = payload
            .pointer("/wsdlDefinition/rows")
            .and_then(Value::as_array)
        {
            facts.push(BranchFact {
                kind: NodeKind::Definition,
                count: rows.len(),
                members: &[],
            });
        }
    }
    let companion = match kind {
        NodeKind::ScheduledJob => Some(NodeKind::Schedule),
        NodeKind::CommonPicture => Some(NodeKind::Picture),
        _ => None,
    };
    if let Some(kind) = companion {
        facts.push(BranchFact {
            kind,
            count: 1,
            members: &[],
        });
    }
    let relations = metadata_relations(owner, payload);
    if !relations.is_empty() {
        facts.push(BranchFact {
            kind: NodeKind::Relation,
            count: relations.len(),
            members: &[],
        });
    }
    (facts, addressing_issue)
}

pub(crate) fn item_facts(item: &Value) -> Vec<BranchFact<'_>> {
    let mut facts = Vec::new();
    for (key, kind) in METADATA_ITEM_BRANCH_FIELDS {
        let observed = members(item.get(key));
        if !observed.is_empty() {
            facts.push(BranchFact {
                kind,
                count: observed.len(),
                members: observed,
            });
        }
    }
    for dynamic in members(item.get("logicalBranches")) {
        let kind = NodeKind::parse(dynamic["kind"].as_str().unwrap()).unwrap();
        let count = usize::try_from(dynamic["count"].as_u64().unwrap()).unwrap();
        if count != 0 {
            facts.push(BranchFact {
                kind,
                count,
                members: &[],
            });
        }
    }
    facts
}
