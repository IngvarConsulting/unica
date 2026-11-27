use std::collections::{HashMap, HashSet};

use unica_source_contract::debug_source::{
    DebugFrameModule, DebugModuleId, DebugModuleQuery, DebugModuleSource, ResolveDebugModules,
};

use crate::{Breakpoint, Breakpoints, Result};

pub(crate) struct Modules {
    resolve: ResolveDebugModules,
    known: HashMap<DebugModuleId, KnownSource>,
    source_sets_revision: Option<u64>,
}

struct KnownSource {
    address: String,
    /// Presentations that had no match when a full ID lookup established this address.
    unmatched: HashSet<DebugFrameModule>,
}

enum NamedMatch {
    Found(String),
    Missing,
    NeedsLookup,
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

    fn saved_id(&mut self, address: &str) -> Result<Option<DebugModuleId>> {
        let mut matches = self.search(DebugModuleQuery::Address(address))?.into_iter();
        let first = matches.next();
        if let Some(other) = matches.next() {
            return Err(format!("ambiguous debug module {}", other.address).into());
        }
        if let Some(source) = &first {
            if self
                .known
                .get(&source.runtime_id)
                .is_some_and(|source| source.address != address)
            {
                self.known.remove(&source.runtime_id);
            }
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
            if let Some(at) = self.known.get(id).map(|source| source.address.clone()) {
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

    /// RDBG replaces the entire list. Check only the DAP sources included in the replacement.
    pub fn replace_breakpoints(
        &mut self,
        installed: &Breakpoints,
        address: &str,
        points: &[Breakpoint],
    ) -> Result<Breakpoints> {
        let mut replacement = Breakpoints::new();
        // A previously installed breakpoint can be cleared even after its file was deleted.
        if !points.is_empty() || !installed.contains_key(address) {
            let id = self
                .saved_id(address)?
                .ok_or_else(|| format!("unknown debug module {address}"))?;
            if !points.is_empty() {
                replacement.insert(address.to_owned(), (id, points.to_vec()));
            }
        }
        for (at, (id, lines)) in installed {
            if at == address || self.saved_id(at)?.as_ref() != Some(id) {
                continue;
            }
            if replacement
                .get(address)
                .is_some_and(|(requested, _)| requested == id)
            {
                return Err(format!("debugger cannot distinguish {address} from {at}").into());
            }
            replacement.insert(at.clone(), (id.clone(), lines.clone()));
        }
        Ok(replacement)
    }

    fn addresses(
        &mut self,
        ids: &[DebugModuleId],
    ) -> Result<HashMap<DebugModuleId, Option<String>>> {
        let addresses = self.resolve_addresses(ids)?;
        for (id, at) in &addresses {
            if let Some(at) = at {
                self.known.entry(id.clone()).or_insert_with(|| KnownSource {
                    address: at.clone(),
                    unmatched: HashSet::new(),
                });
            }
        }
        Ok(addresses)
    }

    fn named_address(&mut self, id: &DebugModuleId, module: &DebugFrameModule) -> NamedMatch {
        let result = match self.search(DebugModuleQuery::Frame { module, id }) {
            Ok(found) => match found.as_slice() {
                [source] if source.runtime_id == *id => {
                    NamedMatch::Found(source.address.to_string())
                }
                [] => NamedMatch::Missing,
                _ => NamedMatch::NeedsLookup,
            },
            Err(error) => {
                eprintln!("unica dap: cannot resolve frame presentation: {error}");
                NamedMatch::NeedsLookup
            }
        };
        // New evidence about this owner takes precedence over a previous ID-only lookup.
        if self.known.get(id).is_some_and(|known| match &result {
            NamedMatch::Found(at) => at != &known.address,
            NamedMatch::Missing => !known.unmatched.contains(module),
            NamedMatch::NeedsLookup => true,
        }) {
            self.known.remove(id);
        }
        result
    }

    pub fn external_processor_address(&mut self, id: &DebugModuleId, name: &str) -> Option<String> {
        let sources = self
            .search(DebugModuleQuery::ExternalProcessorFrame { name, id })
            .ok()?;
        match sources.as_slice() {
            [source] if source.runtime_id == *id => Some(source.address.to_string()),
            _ => None,
        }
    }

    /// Ordinary and exception stops use the same saved frame evidence. A qualified
    /// presentation can identify a source even when another owner shares its runtime ID.
    pub fn frame_addresses<'a>(
        &mut self,
        frames: impl Iterator<Item = (Option<&'a DebugModuleId>, &'a str, Option<&'a str>)>,
    ) -> Vec<Option<String>> {
        let frames: Vec<_> = frames
            .map(|(id, name, hit)| {
                let module = id.and_then(|id| DebugFrameModule::from_presentation(name, id));
                (id, module, hit)
            })
            .collect();
        let mut named = HashMap::new();
        let mut unresolved = HashSet::new();
        let mut addresses = Vec::with_capacity(frames.len());
        for &(id, ref hint, hit) in &frames {
            let source = match (id, hint, hit) {
                (Some(id), Some(module), _) => match named
                    .entry((id, module))
                    .or_insert_with(|| self.named_address(id, module))
                {
                    NamedMatch::Found(at) => Some(at.clone()),
                    NamedMatch::Missing | NamedMatch::NeedsLookup => None,
                },
                (Some(id), None, Some(at))
                    if self.saved_id(at).ok().flatten().as_ref() == Some(id) =>
                {
                    Some(at.to_owned())
                }
                _ => None,
            };
            if source.is_none() {
                unresolved.extend(id.cloned());
            }
            addresses.push(source);
        }
        let ids: Vec<_> = unresolved.into_iter().collect();
        let fallback = self.available_addresses(&ids);
        for ((id, module), result) in named {
            if matches!(result, NamedMatch::Missing)
                && fallback.get(id).is_some_and(Option::is_some)
            {
                if let Some(known) = self.known.get_mut(id) {
                    known.unmatched.insert(module.clone());
                }
            }
        }
        for (address, (id, _, _)) in addresses.iter_mut().zip(&frames) {
            if address.is_none() {
                *address = id.and_then(|id| fallback.get(id)).cloned().flatten();
            }
        }
        addresses
    }

    fn available_addresses(
        &mut self,
        ids: &[DebugModuleId],
    ) -> HashMap<DebugModuleId, Option<String>> {
        self.addresses(ids).unwrap_or_else(|error| {
            eprintln!("unica dap: cannot resolve stack frame sources: {error}");
            let mut available = HashMap::new();
            if ids.len() > 1 {
                for id in ids {
                    if let Ok(matches) = self.addresses(std::slice::from_ref(id)) {
                        available.extend(matches);
                    }
                }
            }
            available
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{cell::RefCell, rc::Rc};
    use unica_address::address::QualifiedAddress;
    use unica_source_contract::debug_source::DebugModuleMatches;
    use uuid::Uuid;

    const FIRST: &str = "main:Document.First.Form.Main.Module.Form";
    const SECOND: &str = "ext:Document.Second.Form.Main.Module.Form";
    const OTHER: &str = "main:Document.Other.Form.Main.Module.Form";
    const FIRST_NAME: &str = "Документ.First.Форма.Main.Форма.ПриСозданииНаСервере()";
    const OTHER_NAME: &str = "Документ.Other.Форма.Main.Форма.ПриОткрытии()";

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
        fail_next_frame: bool,
        forward: Vec<String>,
        named: Vec<DebugModuleId>,
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
            if let DebugModuleQuery::Frame { id, .. } = query {
                ws.named.push(id.clone());
                if std::mem::take(&mut ws.fail_next_frame) {
                    return Err("frame lookup failed".into());
                }
            }
            let modules = ws
                .sources
                .iter()
                .filter(|(id, at)| match query {
                    DebugModuleQuery::Address(requested) => *at == requested,
                    DebugModuleQuery::Runtime(ids) => ids.contains(id),
                    DebugModuleQuery::ExternalProcessorFrame { .. } => false,
                    DebugModuleQuery::Frame { module, id: wanted } => {
                        let address = QualifiedAddress::parse(at).unwrap();
                        id == wanted
                            && module.in_source_set(address.source_set()).unwrap() == address
                    }
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
    fn external_lookup_requires_one_exact_source_and_never_reuses_the_internal_id_cache() {
        let id = DebugModuleId {
            extension: None,
            object: Uuid::from_u128(17),
            property: unica_source_contract::debug_source::OBJECT_MODULE_PROPERTY_ID,
        };
        let expected = id.clone();
        let owner = "SyntheticProcessor";
        let epf = "epf:ExternalDataProcessor.SyntheticProcessor.Module.Object";
        let internal = "main:DataProcessor.SyntheticProcessor.Module.Object";
        let (mut external, mut runtime) = (0, 0);
        let mut modules = Modules::new(Box::new(move |query| {
            let source = |at: &str, runtime_id: DebugModuleId| DebugModuleSource {
                address: QualifiedAddress::parse(at).unwrap(),
                runtime_id,
            };
            let found = match query {
                DebugModuleQuery::Runtime(ids) => {
                    runtime += 1;
                    assert_eq!(runtime, 1, "external lookup must never scan internal ids");
                    assert_eq!(ids, std::slice::from_ref(&expected));
                    vec![source(internal, expected.clone())]
                }
                DebugModuleQuery::Address(at) => {
                    assert_eq!(at, internal);
                    vec![source(internal, expected.clone())]
                }
                DebugModuleQuery::ExternalProcessorFrame { id, name } => {
                    assert_eq!(id, &expected);
                    assert_eq!(name, owner);
                    external += 1;
                    match external {
                        1 => vec![source(epf, expected.clone())],
                        2 => vec![],
                        3 => vec![
                            source(epf, expected.clone()),
                            source(
                                "copy:ExternalDataProcessor.SyntheticProcessor.Module.Object",
                                expected.clone(),
                            ),
                        ],
                        4 => vec![source(
                            epf,
                            DebugModuleId {
                                object: Uuid::nil(),
                                ..expected.clone()
                            },
                        )],
                        5 => return Err("external source unavailable".into()),
                        _ => panic!("unexpected lookup"),
                    }
                }
                DebugModuleQuery::Frame { .. } => {
                    panic!("external frame reached configuration lookup")
                }
            };
            Ok(DebugModuleMatches {
                modules: found,
                source_sets_revision: 1,
            })
        }));
        assert_eq!(
            modules.frame_addresses([(Some(&id), "", None)].into_iter()),
            [Some(internal.into())]
        );
        assert_eq!(
            modules.external_processor_address(&id, owner).as_deref(),
            Some(epf)
        );
        for _ in 0..4 {
            assert_eq!(modules.external_processor_address(&id, owner), None);
        }
        assert_eq!(
            modules.frame_addresses([(Some(&id), "", None)].into_iter()),
            [Some(internal.into())]
        );
    }

    #[test]
    fn confirmed_breakpoint_and_previous_frame_skip_reverse_lookups_in_a_mixed_stack() {
        let (mut modules, workspace) = setup();
        let points = [Breakpoint {
            line: 7,
            condition: None,
        }];
        let mut installed = Breakpoints::new();
        let pending = modules
            .replace_breakpoints(&installed, FIRST, &points)
            .unwrap();
        assert!(installed.is_empty()); // A failed transport ACK leaves the installed list unchanged.
        assert_eq!(pending[FIRST].0, id(1));
        installed = modules
            .replace_breakpoints(&installed, FIRST, &points)
            .unwrap();
        let first = &installed[FIRST].0;
        let second = id(2);
        for _ in 0..2 {
            let frames = modules.frame_addresses(
                [
                    (Some(first), "Проверка()", Some(FIRST)),
                    (Some(&second), "Вызов()", None),
                    (Some(&second), "ДругойВызов()", None),
                ]
                .into_iter(),
            );
            assert_eq!(
                frames,
                [
                    Some(FIRST.to_owned()),
                    Some(SECOND.to_owned()),
                    Some(SECOND.to_owned())
                ]
            );
        }
        assert_eq!(workspace.borrow().reverse, [vec![id(2)]]);
        assert_eq!(
            workspace.borrow().forward,
            [FIRST, FIRST, FIRST, FIRST, SECOND]
        );
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
        workspace.borrow_mut().sources.push((id(1), OTHER));
        let points = [Breakpoint {
            line: 3,
            condition: Some("a > 1".to_owned()),
        }];
        let first = modules
            .replace_breakpoints(&Breakpoints::new(), FIRST, &points)
            .unwrap();
        assert_eq!(workspace.borrow().forward, [FIRST]);
        let both = modules
            .replace_breakpoints(&first, SECOND, &points)
            .unwrap();
        assert_eq!(both[FIRST].1[0].condition.as_deref(), Some("a > 1"));
        assert_eq!(both[SECOND].0, id(2));
        assert_eq!(workspace.borrow().forward, [FIRST, SECOND, FIRST]);
        assert!(modules.replace_breakpoints(&both, OTHER, &points).is_err());
        assert_eq!(both.len(), 2);
        workspace.borrow_mut().sources[0].0 = id(9);
        assert!(modules
            .replace_breakpoints(&both, SECOND, &[])
            .unwrap()
            .is_empty());
        workspace.borrow_mut().sources.remove(0);
        let cleared = modules.replace_breakpoints(&both, FIRST, &[]).unwrap();
        assert_eq!(cleared.len(), 1);
        assert!(cleared.contains_key(SECOND));
        assert!(modules
            .replace_breakpoints(&cleared, FIRST, &points)
            .is_err());
        assert_eq!(workspace.borrow().reverse, [vec![id(1), id(2)]]);
    }

    #[test]
    fn named_frames_check_their_own_owner_even_with_one_runtime_id() {
        let (mut modules, workspace) = setup();
        let id = id(1);
        assert_eq!(
            modules.addresses(std::slice::from_ref(&id)).unwrap()[&id].as_deref(),
            Some(FIRST)
        );
        workspace.borrow_mut().sources.push((id.clone(), OTHER));
        let frames = modules.frame_addresses(
            [
                (Some(&id), FIRST_NAME, None),
                (
                    Some(&id),
                    "Документ.First.Форма.Main.Форма.Обновить(Число = 1)",
                    None,
                ),
                (Some(&id), OTHER_NAME, Some(FIRST)),
                (Some(&id), "НеполноеИмя()", None),
                (None, FIRST_NAME, None),
            ]
            .into_iter(),
        );
        assert_eq!(
            frames,
            [
                Some(FIRST.to_owned()),
                Some(FIRST.to_owned()),
                Some(OTHER.to_owned()),
                None,
                None
            ]
        );
        assert_eq!(workspace.borrow().named, [id.clone(), id.clone()]);
        assert_eq!(
            workspace.borrow().reverse,
            [vec![id.clone()], vec![id.clone()]]
        );
        let next = modules.frame_addresses([(Some(&id), OTHER_NAME, None)].into_iter());
        assert_eq!(next, [Some(OTHER.to_owned())]);
        assert_eq!(workspace.borrow().reverse.len(), 2);
    }

    #[test]
    fn named_miss_retries_the_runtime_lookup_and_does_not_reuse_a_breakpoint_hint() {
        let (mut modules, workspace) = setup();
        let id = id(1);
        assert_eq!(
            modules.frame_addresses([(Some(&id), "", None)].into_iter()),
            [Some(FIRST.to_owned())]
        );
        workspace
            .borrow_mut()
            .sources
            .push((id.clone(), "main:CommonModule.Duplicate"));
        let frames = modules.frame_addresses(
            [
                (Some(&id), OTHER_NAME, Some(FIRST)),
                (Some(&id), "", Some(FIRST)),
                (Some(&id), "", None),
            ]
            .into_iter(),
        );
        assert_eq!(frames, [None, Some(FIRST.to_owned()), None]);
        assert_eq!(workspace.borrow().reverse, [vec![id.clone()], vec![id]]);
    }

    #[test]
    fn short_frame_with_another_property_reuses_the_runtime_match_across_methods() {
        let (mut modules, workspace) = setup();
        let id = id(1);
        for name in ["First.Open()", "Other.Close()"] {
            assert_eq!(
                modules.frame_addresses([(Some(&id), name, None)].into_iter()),
                [Some(FIRST.to_owned())]
            );
        }
        assert!(workspace.borrow().named.is_empty());
        assert_eq!(workspace.borrow().reverse, [vec![id]]);
        assert_eq!(workspace.borrow().forward, [FIRST]);
    }

    #[test]
    fn repeated_named_miss_rechecks_the_unique_address_without_another_reverse_search() {
        let (mut modules, workspace) = setup();
        let id = id(1);
        for _ in 0..2 {
            assert_eq!(
                modules.frame_addresses([(Some(&id), OTHER_NAME, None)].into_iter()),
                [Some(FIRST.to_owned())]
            );
        }
        assert_eq!(workspace.borrow().reverse, [vec![id.clone()]]);
        assert_eq!(workspace.borrow().forward, [FIRST]);
        workspace.borrow_mut().sources.push((id.clone(), OTHER));
        assert_eq!(
            modules.frame_addresses([(Some(&id), OTHER_NAME, None)].into_iter()),
            [Some(OTHER.to_owned())]
        );
        assert_eq!(
            modules.frame_addresses([(Some(&id), "", None)].into_iter()),
            [None]
        );
        assert_eq!(workspace.borrow().reverse, [vec![id.clone()], vec![id]]);
    }

    #[test]
    fn a_previous_named_miss_does_not_mask_two_new_matches_in_existing_source_sets() {
        let (mut modules, workspace) = setup();
        workspace
            .borrow_mut()
            .sources
            .push((id(3), "copy:CommonModule.Existing"));
        let id = id(1);
        let frame = [(Some(&id), OTHER_NAME, None)];
        assert_eq!(
            modules.frame_addresses(frame.into_iter()),
            [Some(FIRST.to_owned())]
        );
        workspace.borrow_mut().sources.extend([
            (id.clone(), OTHER),
            (id.clone(), "copy:Document.Other.Form.Main.Module.Form"),
        ]);
        assert_eq!(modules.frame_addresses(frame.into_iter()), [None]);
        assert_eq!(workspace.borrow().reverse, [vec![id.clone()], vec![id]]);
    }

    #[test]
    fn a_frame_lookup_error_after_a_named_miss_requires_fresh_runtime_evidence() {
        let (mut modules, workspace) = setup();
        let id = id(1);
        let frame = [(Some(&id), OTHER_NAME, None)];
        assert_eq!(
            modules.frame_addresses(frame.into_iter()),
            [Some(FIRST.to_owned())]
        );
        {
            let mut ws = workspace.borrow_mut();
            ws.sources.push((id.clone(), "main:CommonModule.Duplicate"));
            ws.fail_next_frame = true;
        }
        assert_eq!(modules.frame_addresses(frame.into_iter()), [None]);
        assert_eq!(workspace.borrow().reverse, [vec![id.clone()], vec![id]]);
    }
}
