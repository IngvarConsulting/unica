//! The editability a `view` node shows (#2877): the answer of the same
//! vendor-support gate `apply` asks, read from the retained root, together
//! with the project's support policy.

use super::{ProviderReadAuthority, ViewError};
use crate::infrastructure::native_operations::apply_families::{
    support_gate_facts, SupportGateFacts, SupportGateMarker,
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
            Self::NotSupported | Self::EditableWithSupport | Self::RemovedFromSupport | Self::Extension
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
}

impl NodeEditability {
    pub fn editable(&self) -> bool {
        self.reason.permits_edit() || self.policy != SupportPolicyMode::Deny
    }
}

/// Whether `at` is a node that belongs to an object of a configuration or
/// extension set: the root, or any address under a named top-level object.
pub fn shows_editability(at: &QualifiedAddress, kind: SourceSetKind) -> bool {
    if !matches!(kind, SourceSetKind::Configuration | SourceSetKind::Extension) {
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
    ) -> Result<(NodeEditability, Option<u8>), ViewError> {
        if self.source_set_kind == SourceSetKind::Extension {
            return Ok((
                NodeEditability {
                    reason: SupportReason::Extension,
                    policy,
                },
                None,
            ));
        }
        if let Err(error) = self.support_marker(&mut || Ok(())) {
            if error.detail() == Some(RefusalDetail::SourceUnreadable) {
                return Ok((
                    NodeEditability {
                        reason: SupportReason::Unknown,
                        policy,
                    },
                    None,
                ));
            }
            return Err(error);
        }
        let facts = self.support_gate_facts_for(at, false)?;
        let reason = match (&facts.lock, facts.marker) {
            (Some(lock), _) if lock.kind == SupportLockKind::ConfigurationLocked => {
                SupportReason::ConfigurationReadOnly
            }
            (Some(_), _) => SupportReason::Locked,
            (None, None) => SupportReason::NotSupported,
            (None, Some(SupportGateMarker::Removed)) => SupportReason::RemovedFromSupport,
            (None, Some(SupportGateMarker::Supported { .. })) => match facts.owner_rule {
                Some(1) => SupportReason::EditableWithSupport,
                Some(2) => SupportReason::RemovedFromSupport,
                _ => SupportReason::NotSupported,
            },
        };
        Ok((NodeEditability { reason, policy }, facts.owner_rule))
    }

    /// Whether removing the object at `at` is refused under `deny` without a
    /// support change: a locked object, a locked own child, a configuration
    /// whose changes are disabled, or support that cannot be read.
    pub(crate) fn removal_blocked_reason(
        &self,
        at: &QualifiedAddress,
        editability: &NodeEditability,
    ) -> Result<Option<SupportReason>, ViewError> {
        if editability.policy != SupportPolicyMode::Deny
            || editability.reason == SupportReason::Extension
        {
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
        let facts = self.support_gate_facts_for(at, true)?;
        Ok(facts.lock.map(|_| SupportReason::Locked))
    }
}

/// The regular files under `relative` (recursively), relative to `root`, as
/// the gate lists a removed object's own descriptors. An absent directory is
/// empty.
fn descriptor_tree(root: &SourceDirectory, relative: &Path) -> Result<Vec<PathBuf>, ViewError> {
    let unreadable =
        |error: std::io::Error| ViewError::detailed(RefusalDetail::SourceUnreadable, error.to_string());
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
    let mut files = Vec::new();
    let mut pending = vec![(directory, relative.to_path_buf())];
    while let Some((directory, path)) = pending.pop() {
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
                Ok(SourceEntry::RegularFile(_)) => files.push(path.join(&name)),
                Ok(_) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(unreadable(error)),
            }
        }
    }
    files.sort();
    Ok(files)
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
    for key in ["support", "supportRule", "supportRuleReason", "directEditSafe"] {
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
            let blocked = if op == "object.remove" {
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
    owner_rule: Option<u8>,
) -> Option<&'static str> {
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
        owner_rule.and_then(rule_name)
    } else {
        None
    }
}
