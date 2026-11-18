use std::collections::HashMap;

use unica_source_contract::debug_source::{DebugModuleId, DebugModuleSource};

use crate::Result;

#[derive(Default)]
pub(crate) struct Modules {
    by_address: HashMap<String, DebugModuleId>,
    by_id: HashMap<DebugModuleId, String>,
}

impl Modules {
    pub fn new(sources: Vec<DebugModuleSource>) -> Result<Self> {
        let mut index = Self::default();
        for source in sources {
            let address = source.address.to_string();
            let id = source.runtime_id;
            if index.by_id.insert(id.clone(), address.clone()).is_some()
                || index.by_address.insert(address.clone(), id).is_some()
            {
                return Err(format!("ambiguous debug module {address}").into());
            }
        }
        Ok(index)
    }

    pub fn id(&self, address: &str) -> Result<DebugModuleId> {
        self.by_address
            .get(address)
            .cloned()
            .ok_or_else(|| format!("unknown debug module {address}").into())
    }

    pub fn address(&self, id: &DebugModuleId) -> Option<&str> {
        self.by_id.get(id).map(String::as_str)
    }
}
