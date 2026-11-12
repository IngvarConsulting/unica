//! The editability a `view` node shows (#2877): the answer of the same
//! vendor-support gate `apply` asks, read from the retained root, together
//! with the project's support policy.

use super::{ProviderReadAuthority, ViewError};
use crate::infrastructure::native_operations::apply_families::{
    removes_support_node, support_gate_facts, SupportGateFacts, SupportGateMarker,
};
use serde_json::{json, Map, Value};
use std::path::{Path, PathBuf};
use unica_address::address::{NodeKind, QualifiedAddress};
use unica_refusal::RefusalDetail;
use unica_source_contract::proposal::{SupportLockKind, SupportPolicyMode};
use unica_source_contract::read::{SourceDirectory, SourceEntry};
use unica_source_contract::SourceSetKind;

/// Why a node may or may not be edited: the support state that decides it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SupportReason {
    NotSupported,
    EditableWithSupport,
    RemovedFromSupport,
    Locked,
    ConfigurationReadOnly,
    Extension,
    Unknown,
}

impl SupportReason {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::NotSupported => "notSupported",
            Self::EditableWithSupport => "editableWithSupport",
            Self::RemovedFromSupport => "removedFromSupport",
            Self::Locked => "locked",
            Self::ConfigurationReadOnly => "configurationReadOnly",
            Self::Extension => "extension",
            Self::Unknown => "unknown",
        }
    }

    /// The reason itself lets `apply` edit the node without a support change.
    const fn permits_edit(self) -> bool {
        matches!(
            self,
            Self::NotSupported
                | Self::EditableWithSupport
                | Self::RemovedFromSupport
                | Self::Extension
        )
    }
}

pub const fn policy_name(policy: SupportPolicyMode) -> &'static str {
    match policy {
        SupportPolicyMode::Deny => "deny",
        SupportPolicyMode::Warn => "warn",
        SupportPolicyMode::Off => "off",
    }
}

/// The editability of one node.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NodeEditability {
    pub reason: SupportReason,
    pub policy: SupportPolicyMode,
    /// The support cannot be read — a damaged marker or an owner whose
    /// descriptor names no uuid: `apply` refuses under `deny` and `off` alike
    /// (INV.SAFETY.APPLY-SUPPORT-GATE).
    pub marker_unreadable: bool,
}

impl NodeEditability {
    /// What `apply` does without a support change: a lock refuses only under
    /// `deny`; support that cannot be read refuses under `deny` and `off`
    /// and is edited, with a warning, only under `warn`.
    pub fn editable(&self) -> bool {
        if self.marker_unreadable {
            return self.policy == SupportPolicyMode::Warn;
        }
        self.reason.permits_edit() || self.policy != SupportPolicyMode::Deny
    }
}

/// Whether `at` is a node that belongs to an object of a configuration or
/// extension set: the root, or any address under a named top-level object.
pub fn shows_editability(at: &QualifiedAddress, kind: SourceSetKind) -> bool {
    if !matches!(
        kind,
        SourceSetKind::Configuration | SourceSetKind::Extension
    ) {
        return false;
    }
    // The gate's owner: the configuration root for the root and its own
    // nodes, otherwise the named top-level object. A branch of a kind has no
    // owner of its own: its items carry `support`.
    match at.segments() {
        [root, ..] if root.kind() == NodeKind::Configuration => true,
        [owner, ..] => owner.kind().is_metadata_kind() && owner.name().is_some(),
        [] => false,
    }
}

const SUPPORT_OPERATIONS: &[&str] = &["supportCapability.set", "supportRule.set"];

impl ProviderReadAuthority {
    /// The gate's facts for `at` from the retained root.
    pub(crate) fn support_gate_facts_for(
        &self,
        at: &QualifiedAddress,
        removes: bool,
    ) -> Result<SupportGateFacts, ViewError> {
        let root = &self.root;
        support_gate_facts(
            at,
            self.source_set_kind,
            removes,
            &mut |relative| self.read_optional_relative_whole(relative),
            &mut |relative| descriptor_tree(root, relative),
        )
    }

    /// The editability of `at` under `policy`. An unreadable marker is
    /// `unknown`, not "not on support" (INV.APP.SUPPORT-STATE).
    pub(crate) fn node_editability(
        &self,
        at: &QualifiedAddress,
        policy: SupportPolicyMode,
    ) -> Result<(NodeEditability, OwnerRecord), ViewError> {
        let none = OwnerRecord::default();
        if self.source_set_kind == SourceSetKind::Extension {
            return Ok((
                NodeEditability {
                    reason: SupportReason::Extension,
                    policy,
                    marker_unreadable: false,
                },
                none,
            ));
        }
        if let Err(error) = self.support_marker(&mut || Ok(())) {
            if error.detail() == Some(RefusalDetail::SourceUnreadable) {
                return Ok((
                    NodeEditability {
                        reason: SupportReason::Unknown,
                        policy,
                        marker_unreadable: true,
                    },
                    none,
                ));
            }
            return Err(error);
        }
        let facts = match self.support_gate_facts_for(at, false) {
            Ok(facts) => facts,
            // The marker or the owner's descriptor cannot be read: not "not
            // on support".
            Err(error) if error.detail() == Some(RefusalDetail::SourceUnreadable) => {
                return Ok((
                    NodeEditability {
                        reason: SupportReason::Unknown,
                        policy,
                        marker_unreadable: false,
                    },
                    none,
                ));
            }
            Err(error) => return Err(error),
        };
        let reason = match (&facts.lock, facts.marker) {
            _ if facts.unreadable.is_some() => SupportReason::Unknown,
            (Some(lock), _) if lock.kind == SupportLockKind::ConfigurationLocked => {
                SupportReason::ConfigurationReadOnly
            }
            (Some(_), _) => SupportReason::Locked,
            (None, None) => SupportReason::NotSupported,
            (None, Some(SupportGateMarker::Removed)) => SupportReason::RemovedFromSupport,
            // An owner whose descriptor names no uuid has a rule nobody can
            // look up: not "not on support" (#2883).
            (None, Some(SupportGateMarker::Supported { .. })) if facts.owner_uuid_missing => {
                SupportReason::Unknown
            }
            (None, Some(SupportGateMarker::Supported { .. })) => match facts.owner_rule {
                Some(1) => SupportReason::EditableWithSupport,
                Some(2) => SupportReason::RemovedFromSupport,
                _ => SupportReason::NotSupported,
            },
        };
        let records = matches!(facts.marker, Some(SupportGateMarker::Supported { .. }));
        // Under a configuration whose changes are disabled the gate stops
        // before the owner; its own record is still named.
        let rule = if reason == SupportReason::ConfigurationReadOnly {
            self.owner_record_rule(at).unwrap_or(None)
        } else {
            facts.owner_rule
        };
        Ok((
            NodeEditability {
                reason,
                policy,
                marker_unreadable: facts.unreadable.is_some(),
            },
            OwnerRecord {
                rule,
                records,
                node_rule: facts.node_rule,
            },
        ))
    }

    /// The rule of the own record of the owner of `at`, read from the marker
    /// and the owner's descriptor; `None` without one.
    fn owner_record_rule(&self, at: &QualifiedAddress) -> Result<Option<u8>, ViewError> {
        use crate::infrastructure::native_operations::apply_families::support_owner_relative;
        let Some((_, relative)) = support_owner_relative(at, self.source_set_kind) else {
            return Ok(None);
        };
        let Some(state) = self.support_marker(&mut || Ok(()))? else {
            return Ok(None);
        };
        Ok(self
            .read_optional_relative_whole(&relative)?
            .and_then(|bytes| {
                crate::infrastructure::family_codecs::metadata_read_codec()
                    .support_root_uuid(&bytes)
            })
            .and_then(|uuid| state.object_rule(&uuid)))
    }

    /// Whether removing the object at `at` is refused under `deny` without a
    /// support change: a locked object, a locked own child, a configuration
    /// whose changes are disabled, or support that cannot be read.
    pub(crate) fn removal_blocked_reason(
        &self,
        at: &QualifiedAddress,
        editability: &NodeEditability,
    ) -> Result<Option<SupportReason>, ViewError> {
        if editability.reason == SupportReason::Extension {
            return Ok(None);
        }
        // Support that cannot be read refuses the removal under `off` too.
        if editability.marker_unreadable && editability.policy == SupportPolicyMode::Off {
            return Ok(Some(SupportReason::Unknown));
        }
        if editability.policy != SupportPolicyMode::Deny {
            return Ok(None);
        }
        if matches!(
            editability.reason,
            SupportReason::Unknown | SupportReason::ConfigurationReadOnly
        ) {
            return Ok(Some(editability.reason));
        }
        // A lock on the object or on one of its own children: either way the
        // removal is refused for a `locked` rule.
        match self.support_gate_facts_for(at, true) {
            Ok(facts) => Ok(facts.lock.map(|_| SupportReason::Locked)),
            Err(error) if error.detail() == Some(RefusalDetail::SourceUnreadable) => {
                Ok(Some(SupportReason::Unknown))
            }
            Err(error) => Err(error),
        }
    }
}

/// The entries under `relative` (recursively), relative to `root`, as the
/// staged listing of `apply` gives them to the gate: every entry that is not
/// a directory counts as a file, and a tree deeper than the apply namespace
/// bound is refused. An absent directory is empty.
fn descriptor_tree(root: &SourceDirectory, relative: &Path) -> Result<Vec<PathBuf>, ViewError> {
    let unreadable = |error: std::io::Error| {
        ViewError::detailed(RefusalDetail::SourceUnreadable, error.to_string())
    };
    let mut directory = root.clone();
    for component in relative.components() {
        let std::path::Component::Normal(name) = component else {
            return Ok(Vec::new());
        };
        match directory.retain_immediate_child_nofollow(name) {
            Ok(SourceEntry::Directory(child)) => directory = child,
            Ok(_) => return Ok(Vec::new()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(unreadable(error)),
        }
    }
    // resource-limit: view.support-removal-tree-depth
    // class: format
    // unit: depth
    // scope: request
    // value: 256
    // resource: Глубина каталога объекта, который `view` обходит ради пометки удаления в `can`; та же граница, что у перечисления `apply` (MAX_APPLY_NAMESPACE_DEPTH).
    // status: unproven
    // origin: #2877
    // problem: неизвестно
    // on-reach: состояние поддержки удаления — `unknown`, удаление помечено `blocked`, как `apply` отказывает перечислению глубже границы
    const MAX_DEPTH: usize = 256;
    let initial_depth = relative.components().count();
    let mut files = Vec::new();
    let mut pending = vec![(directory, relative.to_path_buf())];
    while let Some((directory, path)) = pending.pop() {
        if path.components().count() - initial_depth > MAX_DEPTH {
            return Err(ViewError::detailed(
                RefusalDetail::SourceUnreadable,
                "namespace input exceeds maximum depth",
            ));
        }
        let mut names = Vec::new();
        directory
            .visit_immediate_names(|name| {
                names.push(name);
                Ok::<_, std::io::Error>(())
            })
            .map_err(unreadable)?;
        for name in names {
            match directory.retain_immediate_child_nofollow(&name) {
                Ok(SourceEntry::Directory(child)) => pending.push((child, path.join(&name))),
                Ok(_) => files.push(path.join(&name)),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(unreadable(error)),
            }
        }
    }
    files.sort();
    Ok(files)
}

/// What the owner's own support record says, for `rule`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct OwnerRecord {
    /// The owner's own rule: 0 `locked`, 1 editable with support, 2 taken
    /// off support.
    pub(crate) rule: Option<u8>,
    /// The marker holds per-object records: the configuration is on support
    /// and was not taken off as a whole.
    pub(crate) records: bool,
    /// The rule of the addressed node's own record below the owner, as the
    /// gate found it on the node's path.
    pub(crate) node_rule: Option<u8>,
}

/// The vendor rule of a node's own record, as `view` names it.
fn rule_name(rule: u8) -> Option<&'static str> {
    match rule {
        0 => Some("locked"),
        1 => Some("editableWithSupport"),
        2 => Some("removedFromSupport"),
        _ => None,
    }
}

/// Replaces the support facts of a projected node with one
/// `props.support {editable, reason, policy, rule?, objects?}` and marks the
/// node's `can` entries that `apply` would refuse under `deny` without a
/// support change. `own_rule` is the rule of the node's own record when it
/// has one (the object itself, a form).
pub(crate) fn attach_editability(
    data: &mut Value,
    at: &QualifiedAddress,
    editability: &NodeEditability,
    own_rule: Option<&str>,
    removal_blocked: Option<SupportReason>,
) {
    let Some(node) = data.as_object_mut() else {
        return;
    };
    let props = node
        .entry("props")
        .or_insert_with(|| Value::Object(Map::new()));
    let Some(props) = props.as_object_mut() else {
        return;
    };
    let objects = props
        .get("support")
        .and_then(|support| support.get("objects"))
        .cloned();
    for key in [
        "support",
        "supportRule",
        "supportRuleReason",
        "directEditSafe",
    ] {
        props.remove(key);
    }
    let mut support = json!({
        "editable": editability.editable(),
        "reason": editability.reason.as_str(),
        "policy": policy_name(editability.policy),
    });
    if let Some(rule) = own_rule {
        support["rule"] = json!(rule);
    }
    if let Some(objects) = objects {
        support["objects"] = objects;
    }
    props.insert("support".to_string(), support);

    let edit_blocked = (!editability.editable()).then_some(editability.reason);
    if let Some(Value::Array(entries)) = node.get_mut("can") {
        for entry in entries {
            let Some(op) = entry.get("op").and_then(Value::as_str) else {
                continue;
            };
            if SUPPORT_OPERATIONS.contains(&op) {
                continue;
            }
            let blocked = if removes_support_node(op, at) {
                removal_blocked
            } else {
                edit_blocked
            };
            if let Some(reason) = blocked {
                entry["blocked"] = json!({"reason": reason.as_str()});
            }
        }
    }
}

/// The own rule a projected node already names: a form's `supportRule`, or
/// the object's own record when the node is the object itself.
pub(crate) fn own_rule(
    data: &Value,
    at: &QualifiedAddress,
    owner: OwnerRecord,
) -> Option<&'static str> {
    if owner.records {
        if let Some(rule) = owner.node_rule {
            return rule_name(rule);
        }
    }
    if let Some(rule) = data
        .get("props")
        .and_then(|props| props.get("supportRule"))
        .and_then(Value::as_str)
    {
        return match rule {
            "locked" => Some("locked"),
            "editableWithSupport" => Some("editableWithSupport"),
            "removedFromSupport" => Some("removedFromSupport"),
            _ => None,
        };
    }
    let is_owner = at.segments().len() == 1;
    if is_owner {
        return owner.rule.and_then(rule_name);
    }
    // A nested node with its own record (a nested subsystem) names its own
    // state in the projected `props.support`. That is its record only while
    // the marker holds per-object records; a configuration taken off support
    // or with changes disabled projects its own state instead.
    if !owner.records {
        return None;
    }
    match data
        .get("props")
        .and_then(|props| props.get("support"))
        .and_then(Value::as_str)
    {
        Some("locked") => Some("locked"),
        Some("editableWithSupport") => Some("editableWithSupport"),
        Some("removedFromSupport") => Some("removedFromSupport"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn catalog() -> QualifiedAddress {
        QualifiedAddress::parse("main:Catalog.Items").unwrap()
    }

    fn node() -> Value {
        json!({
            "at": "main:Catalog.Items",
            "props": {"support": "locked", "supportRule": "locked", "directEditSafe": false},
            "can": [
                {"op": "props.set", "args": {}},
                {"op": "object.remove", "args": {}},
                {"op": "supportRule.set", "args": {}}
            ]
        })
    }

    fn blocked(data: &Value) -> Vec<Option<&str>> {
        data["can"]
            .as_array()
            .unwrap()
            .iter()
            .map(|entry| entry["blocked"]["reason"].as_str())
            .collect()
    }

    /// An unreadable support state is not "not on support": under `deny`
    /// the node is not editable and its edits are marked; the support
    /// operations stay open.
    /// rule: INV.APP.SUPPORT-STATE
    /// rule: INV.WIRE.V13-CAN-DICTIONARY
    #[test]
    fn unknown_support_is_not_editable_under_deny() {
        let mut data = node();
        let editability = NodeEditability {
            reason: SupportReason::Unknown,
            policy: SupportPolicyMode::Deny,
            marker_unreadable: false,
        };
        attach_editability(
            &mut data,
            &catalog(),
            &editability,
            None,
            Some(SupportReason::Unknown),
        );
        assert_eq!(
            data["props"],
            json!({"support": {"editable": false, "reason": "unknown", "policy": "deny"}})
        );
        assert_eq!(blocked(&data), [Some("unknown"), Some("unknown"), None]);
    }

    /// A support marker that cannot be read is not editable under `deny`
    /// and `off`, as `apply` refuses it there, and editable under `warn`.
    /// rule: INV.APP.SUPPORT-STATE
    /// rule: INV.SAFETY.APPLY-SUPPORT-GATE
    #[test]
    fn an_unreadable_marker_is_editable_only_under_warn() {
        for (policy, editable) in [
            (SupportPolicyMode::Deny, false),
            (SupportPolicyMode::Off, false),
            (SupportPolicyMode::Warn, true),
        ] {
            let editability = NodeEditability {
                reason: SupportReason::Unknown,
                policy,
                marker_unreadable: true,
            };
            assert_eq!(editability.editable(), editable, "{policy:?}");
            let mut data = node();
            attach_editability(&mut data, &catalog(), &editability, None, None);
            assert_eq!(data["props"]["support"]["editable"], editable, "{policy:?}");
        }
    }

    /// `warn` and `off` let `apply` edit whatever the reason: nothing is
    /// marked and the node is editable.
    /// rule: INV.APP.SUPPORT-STATE
    #[test]
    fn a_permissive_policy_marks_nothing() {
        for policy in [SupportPolicyMode::Warn, SupportPolicyMode::Off] {
            let mut data = node();
            let editability = NodeEditability {
                reason: SupportReason::Locked,
                policy,
                marker_unreadable: false,
            };
            attach_editability(&mut data, &catalog(), &editability, Some("locked"), None);
            assert_eq!(data["props"]["support"]["editable"], true);
            assert_eq!(data["props"]["support"]["rule"], "locked");
            assert_eq!(blocked(&data), [None, None, None]);
        }
    }

    /// The root keeps its object counts; the old fields are replaced.
    /// rule: INV.APP.SUPPORT-STATE
    #[test]
    fn the_root_keeps_its_object_counts() {
        let mut data = json!({
            "props": {"support": {"state": "supported", "editingEnabled": true,
                "objects": {"locked": 1, "editable": 2, "removed": 0}}}
        });
        let editability = NodeEditability {
            reason: SupportReason::EditableWithSupport,
            policy: SupportPolicyMode::Deny,
            marker_unreadable: false,
        };
        attach_editability(&mut data, &catalog(), &editability, None, None);
        assert_eq!(
            data["props"]["support"],
            json!({"editable": true, "reason": "editableWithSupport", "policy": "deny",
                "objects": {"locked": 1, "editable": 2, "removed": 0}})
        );
    }
}
