//! Действующая возможность правки узла `view` по поддержке (#2877).

use super::*;
use crate::infrastructure::v13_read_port::{attach_editability, own_rule, shows_editability};
use unica_source_contract::proposal::SupportPolicyMode;

impl<'a> LogicalViewReadAuthority<'a> {
    /// Replaces the support facts of the projected node at `at` with its
    /// editability under the project policy (read only for such a node) and
    /// marks the `can` entries that `apply`
    /// would refuse without a support change. A node outside an object of a
    /// configuration or extension set is left as it is.
    pub fn attach_editability(
        &self,
        at: &QualifiedAddress,
        data: &mut Value,
        policy: impl FnOnce() -> SupportPolicyMode,
    ) -> Result<(), ViewError> {
        if !shows_editability(at, self.read.source_set_kind()) {
            return Ok(());
        }
        let policy = policy();
        self.read_checkpoint()?;
        let (editability, owner_rule) = self.read.node_editability(at, policy)?;
        let removes = data
            .get("can")
            .and_then(Value::as_array)
            .is_some_and(|entries| {
                entries
                    .iter()
                    .any(|entry| entry.get("op").and_then(Value::as_str) == Some("object.remove"))
            });
        let removal_blocked = if removes {
            self.read.removal_blocked_reason(at, &editability)?
        } else {
            None
        };
        let rule = own_rule(data, at, owner_rule);
        attach_editability(data, &editability, rule, removal_blocked);
        Ok(())
    }
}
