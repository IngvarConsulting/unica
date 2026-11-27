use std::collections::{HashMap, HashSet};

use unica_source_contract::debug_source::{
    DebugModuleId, DebugModuleQuery, DebugModuleSource, ResolveDebugModules,
};

use crate::Result;

pub(crate) struct Modules {
    resolve: ResolveDebugModules,
    known: HashMap<DebugModuleId, String>,
    source_sets_revision: Option<u64>,
}

impl Modules {
    pub fn new(resolve: ResolveDebugModules) -> Self {
        Self {
            resolve,
            known: HashMap::new(),
            source_sets_revision: None,
        }
    }

    fn search(&mut self, query: DebugModuleQuery<'_>) -> Result<Vec<DebugModuleSource>> {
        let found = (self.resolve)(query)?;
        if self.source_sets_revision != Some(found.source_sets_revision) {
            self.known.clear();
            self.source_sets_revision = Some(found.source_sets_revision);
        }
        Ok(found.modules)
    }

    pub fn saved_id(&mut self, address: &str) -> Result<Option<DebugModuleId>> {
        let mut matches = self.search(DebugModuleQuery::Address(address))?.into_iter();
        let first = matches.next();
        if let Some(other) = matches.next() {
            return Err(format!("ambiguous debug module {}", other.address).into());
        }
        Ok(first.map(|source| source.runtime_id))
    }

    fn resolve_addresses(
        &mut self,
        ids: &[DebugModuleId],
    ) -> Result<HashMap<DebugModuleId, Option<String>>> {
        let mut addresses = HashMap::new();
        let mut missing = Vec::new();
        let mut seen = HashSet::new();
        let initial_revision = self.source_sets_revision;
        for id in ids {
            if !seen.insert(id) {
                continue;
            }
            if let Some(at) = self.known.get(id).cloned() {
                let current = self.saved_id(&at);
                if self.source_sets_revision != initial_revision {
                    return self.lookup_runtime(ids);
                }
                if current.ok().flatten().as_ref() == Some(id) {
                    addresses.insert(id.clone(), Some(at));
                    continue;
                }
            }
            self.known.remove(id);
            missing.push(id.clone());
        }
        if !missing.is_empty() {
            let fresh = self.lookup_runtime(&missing)?;
            if !addresses.is_empty() && self.source_sets_revision != initial_revision {
                return self.lookup_runtime(ids);
            }
            addresses.extend(fresh);
        }
        Ok(addresses)
    }

    fn lookup_runtime(
        &mut self,
        ids: &[DebugModuleId],
    ) -> Result<HashMap<DebugModuleId, Option<String>>> {
        let mut addresses = HashMap::new();
        if ids.is_empty() {
            return Ok(addresses);
        }
        for source in self.search(DebugModuleQuery::Runtime(ids))? {
            addresses
                .entry(source.runtime_id)
                .and_modify(|value| *value = None)
                .or_insert_with(|| Some(source.address.to_string()));
        }
        Ok(addresses)
    }

    /// RDBG replaces its entire breakpoint list. Recheck every remaining module
    /// before sending the replacement, including when one address is being cleared.
    pub fn breakpoint_addresses(
        &mut self,
        ids: &[DebugModuleId],
    ) -> Result<HashMap<DebugModuleId, Option<String>>> {
        for id in ids {
            self.known.remove(id);
        }
        self.lookup_runtime(ids)
    }

    pub fn remember<'a>(&mut self, matches: impl Iterator<Item = (&'a DebugModuleId, &'a str)>) {
        for (id, address) in matches {
            self.known
                .entry(id.clone())
                .or_insert_with(|| address.to_owned());
        }
    }

    /// Both ordinary and exception stops report their saved frames through this path.
    pub fn addresses(
        &mut self,
        ids: &[DebugModuleId],
    ) -> Result<HashMap<DebugModuleId, Option<String>>> {
        let addresses = self.resolve_addresses(ids)?;
        self.remember(
            addresses
                .iter()
                .filter_map(|(id, address)| Some((id, address.as_deref()?))),
        );
        Ok(addresses)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{cell::RefCell, rc::Rc};
    use unica_address::address::QualifiedAddress;
    use unica_source_contract::debug_source::DebugModuleMatches;
    use uuid::Uuid;

    const FIRST: &str = "main:CommonModule.First";
    const SECOND: &str = "ext:CommonModule.Second";

    fn id(n: u128) -> DebugModuleId {
        DebugModuleId {
            extension: if n == 2 {
                Some("Extension".into())
            } else {
                None
            },
            object: Uuid::from_u128(n),
            property: Uuid::from_u128(99),
        }
    }

    #[derive(Default)]
    struct Workspace {
        sources: Vec<(DebugModuleId, &'static str)>,
        reverse: Vec<Vec<DebugModuleId>>,
        revision: u64,
        fail_next_reverse: bool,
        fail_next_address: bool,
        forward: Vec<String>,
    }

    fn setup() -> (Modules, Rc<RefCell<Workspace>>) {
        let workspace = Rc::new(RefCell::new(Workspace {
            sources: vec![(id(1), FIRST), (id(2), SECOND)],
            revision: 1,
            ..Default::default()
        }));
        let read = Rc::clone(&workspace);
        let resolver: ResolveDebugModules = Box::new(move |query| {
            let mut ws = read.borrow_mut();
            if let DebugModuleQuery::Address(at) = query {
                ws.forward.push(at.to_owned());
                if std::mem::take(&mut ws.fail_next_address) {
                    return Err("address unavailable".into());
                }
            }
            if let DebugModuleQuery::Runtime(ids) = query {
                ws.reverse.push(ids.to_vec());
                if std::mem::take(&mut ws.fail_next_reverse) {
                    return Err("source lookup failed".into());
                }
            }
            let modules = ws
                .sources
                .iter()
                .filter(|(id, at)| match query {
                    DebugModuleQuery::Address(requested) => *at == requested,
                    DebugModuleQuery::Runtime(ids) => ids.contains(id),
                })
                .map(|(id, at)| DebugModuleSource {
                    address: QualifiedAddress::parse(at).unwrap(),
                    runtime_id: id.clone(),
                })
                .collect();
            Ok(DebugModuleMatches {
                modules,
                source_sets_revision: ws.revision,
            })
        });
        (Modules::new(resolver), workspace)
    }

    #[test]
    fn confirmed_breakpoint_and_previous_frame_skip_reverse_lookups_in_a_mixed_stack() {
        let (mut modules, workspace) = setup();
        let first = modules.saved_id(FIRST).unwrap().unwrap();
        let rejected = modules
            .breakpoint_addresses(std::slice::from_ref(&first))
            .unwrap();
        assert_eq!(rejected[&first].as_deref(), Some(FIRST));
        // No transport confirmation for the first attempt.
        let pending = modules
            .breakpoint_addresses(std::slice::from_ref(&first))
            .unwrap();
        assert_eq!(pending[&first].as_deref(), Some(FIRST));
        modules.remember(std::iter::once((&first, FIRST)));
        let stack = [first.clone(), id(2), id(2)];
        for _ in 0..2 {
            let frames = modules.addresses(&stack).unwrap();
            assert_eq!(frames[&first].as_deref(), Some(FIRST));
            assert_eq!(frames[&id(2)].as_deref(), Some(SECOND));
        }
        assert_eq!(
            workspace.borrow().reverse,
            [vec![id(1)], vec![id(1)], vec![id(2)]]
        );
        assert_eq!(workspace.borrow().forward, [FIRST, FIRST, FIRST, SECOND]);
    }

    #[test]
    fn ambiguous_and_missing_frames_are_resolved_again() {
        let (mut modules, workspace) = setup();
        workspace
            .borrow_mut()
            .sources
            .push((id(1), "main:CommonModule.Other"));
        let ids = [id(1), id(3)];
        let first = modules.addresses(&ids).unwrap();
        assert_eq!(first[&ids[0]], None);
        assert!(!first.contains_key(&ids[1]));
        workspace.borrow_mut().sources.pop();
        let second = modules.addresses(&ids).unwrap();
        assert_eq!(second[&ids[0]].as_deref(), Some(FIRST));
        assert_eq!(workspace.borrow().reverse, [ids.to_vec(), ids.to_vec()]);
    }

    #[test]
    fn a_new_revision_rechecks_a_known_frame_and_suppresses_its_ambiguous_source() {
        let (mut modules, workspace) = setup();
        let ids = [id(1)];
        assert_eq!(
            modules.addresses(&ids).unwrap()[&ids[0]].as_deref(),
            Some(FIRST)
        );
        {
            let mut ws = workspace.borrow_mut();
            ws.sources.push((id(1), "main:CommonModule.Other"));
            ws.revision = 2;
        }
        assert_eq!(modules.addresses(&ids).unwrap()[&ids[0]], None);
        assert_eq!(workspace.borrow().reverse, [ids.to_vec(), ids.to_vec()]);
    }

    #[test]
    fn failed_lookup_of_a_new_frame_is_retried_independently_of_the_known_frame() {
        let (mut modules, workspace) = setup();
        modules.addresses(&[id(1)]).unwrap();
        workspace.borrow_mut().fail_next_reverse = true;
        assert!(modules.addresses(&[id(1), id(2)]).is_err());
        let recovered = modules.addresses(&[id(1), id(2)]).unwrap();
        assert_eq!(recovered[&id(1)].as_deref(), Some(FIRST));
        assert_eq!(recovered[&id(2)].as_deref(), Some(SECOND));
        assert_eq!(
            workspace.borrow().reverse,
            [vec![id(1)], vec![id(2)], vec![id(2)]]
        );
    }

    #[test]
    fn an_address_error_falls_back_to_a_fresh_lookup_and_a_moved_or_deleted_module_loses_its_old_path(
    ) {
        let (mut modules, workspace) = setup();
        let ids = [id(1)];
        modules.addresses(&ids).unwrap();
        workspace.borrow_mut().fail_next_address = true;
        assert_eq!(
            modules.addresses(&ids).unwrap()[&id(1)].as_deref(),
            Some(FIRST)
        );
        let moved = "main:CommonModule.Renamed";
        workspace.borrow_mut().sources[0].1 = moved;
        assert_eq!(
            modules.addresses(&ids).unwrap()[&id(1)].as_deref(),
            Some(moved)
        );
        workspace.borrow_mut().sources.remove(0);
        assert!(!modules.addresses(&ids).unwrap().contains_key(&id(1)));
        assert_eq!(workspace.borrow().reverse, vec![ids.to_vec(); 4]);
    }

    #[test]
    fn changing_only_the_extension_invalidates_a_previously_known_address() {
        let (mut modules, workspace) = setup();
        modules.addresses(&[id(2)]).unwrap();
        workspace.borrow_mut().sources[1].0.extension = Some("Renamed".into());
        assert!(!modules.addresses(&[id(2)]).unwrap().contains_key(&id(2)));
        assert_eq!(workspace.borrow().reverse, [vec![id(2)], vec![id(2)]]);
    }

    #[test]
    fn replacing_breakpoints_rechecks_all_remaining_modules_even_for_known_frames() {
        let (mut modules, workspace) = setup();
        modules.addresses(&[id(1), id(2)]).unwrap();
        workspace
            .borrow_mut()
            .sources
            .push((id(1), "main:CommonModule.Duplicate"));
        let pending = modules.breakpoint_addresses(&[id(1), id(2)]).unwrap();
        assert_eq!(pending[&id(1)], None);
        assert_eq!(pending[&id(2)].as_deref(), Some(SECOND));
        assert!(modules.breakpoint_addresses(&[]).unwrap().is_empty());
        assert_eq!(
            workspace.borrow().reverse,
            [vec![id(1), id(2)], vec![id(1), id(2)]]
        );
    }
}
