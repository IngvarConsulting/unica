use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};

use unica_source_contract::debug_source::{DebugModuleId, DebugModuleQuery, DebugModuleSource};

use crate::{ResolveModules, Result};

pub(crate) struct Modules {
    root: PathBuf,
    resolve: ResolveModules,
    known: HashMap<DebugModuleId, String>,
}

impl Modules {
    pub fn open(root: &Path, source_set: Option<&str>, resolve: ResolveModules) -> Result<Self> {
        let sources = resolve(root, DebugModuleQuery::FirstSaved { source_set })?;
        if sources.is_empty() {
            return Err(match source_set {
                Some(set) => format!(
                    "debug source set {set} is not available; use the names resolved by workspace"
                ),
                None => "workspace has no supported saved debug modules".to_owned(),
            }
            .into());
        }
        Ok(Self {
            root: root.to_owned(),
            resolve,
            known: HashMap::new(),
        })
    }

    fn lookup(&self, query: DebugModuleQuery<'_>) -> Result<Option<DebugModuleSource>> {
        let mut matches = (self.resolve)(&self.root, query)?.into_iter();
        let first = matches.next();
        if let Some(other) = matches.next() {
            return Err(format!("ambiguous debug module {}", other.address).into());
        }
        Ok(first)
    }

    pub fn id(&mut self, address: &str) -> Result<DebugModuleId> {
        let source = self
            .lookup(DebugModuleQuery::Address(address))?
            .ok_or_else(|| format!("unknown debug module {address}"))?;
        let id = source.runtime_id;
        if let Some(previous) = self.known.get(&id) {
            if previous != address
                && self
                    .lookup(DebugModuleQuery::Address(previous))?
                    .is_some_and(|source| source.runtime_id == id)
            {
                return Err(format!("ambiguous debug module {address}").into());
            }
        }
        self.known
            .retain(|known_id, known_address| known_id == &id || known_address != address);
        self.known.insert(id.clone(), address.to_owned());
        Ok(id)
    }

    pub fn address(&mut self, id: &DebugModuleId) -> Result<Option<String>> {
        if let Some(address) = self.known.get(id) {
            if self
                .lookup(DebugModuleQuery::Address(address))?
                .is_some_and(|source| &source.runtime_id == id)
            {
                return Ok(Some(address.clone()));
            }
        }
        self.known.remove(id);
        let Some(source) = self.lookup(DebugModuleQuery::Runtime(id))? else {
            return Ok(None);
        };
        let address = source.address.to_string();
        self.known.insert(id.clone(), address.clone());
        Ok(Some(address))
    }
}
