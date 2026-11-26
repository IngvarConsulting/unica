use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};

use unica_source_contract::debug_source::{
    DebugModuleId, DebugModuleQuery, DebugModuleSource, ResolveDebugModules,
};

use crate::Result;

pub(crate) struct Modules {
    root: PathBuf,
    resolve: ResolveDebugModules,
}

impl Modules {
    pub fn open(
        root: &Path,
        source_set: Option<&str>,
        mut resolve: ResolveDebugModules,
    ) -> Result<Self> {
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
        })
    }

    fn lookup(&mut self, query: DebugModuleQuery<'_>) -> Result<Option<DebugModuleSource>> {
        let mut matches = (self.resolve)(&self.root, query)?.into_iter();
        let first = matches.next();
        if let Some(other) = matches.next() {
            return Err(format!("ambiguous debug module {}", other.address).into());
        }
        Ok(first)
    }

    pub fn saved_id(&mut self, address: &str) -> Result<Option<DebugModuleId>> {
        Ok(self
            .lookup(DebugModuleQuery::Address(address))?
            .map(|source| source.runtime_id))
    }

    /// A runtime identity with multiple saved owners cannot name a source file.
    pub fn addresses(
        &mut self,
        ids: &[DebugModuleId],
    ) -> Result<HashMap<DebugModuleId, Option<String>>> {
        let mut addresses = HashMap::new();
        if !ids.is_empty() {
            for source in (self.resolve)(&self.root, DebugModuleQuery::Runtime(ids))? {
                addresses
                    .entry(source.runtime_id)
                    .and_modify(|value| *value = None)
                    .or_insert_with(|| Some(source.address.to_string()));
            }
        }
        Ok(addresses)
    }
}
