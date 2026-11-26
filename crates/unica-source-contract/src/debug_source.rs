//! A saved workspace module and the identity used by the platform debugger.

use unica_address::address::QualifiedAddress;
use uuid::Uuid;

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct DebugModuleId {
    pub extension: Option<String>,
    pub object: Uuid,
    pub property: Uuid,
}

#[derive(Debug)]
pub struct DebugModuleSource {
    pub address: QualifiedAddress,
    pub runtime_id: DebugModuleId,
}

/// Which saved module evidence the debugger needs for this request.
#[derive(Clone, Copy)]
pub enum DebugModuleQuery<'a> {
    Address(&'a str),
    Runtime(&'a [DebugModuleId]),
}

pub struct DebugModuleMatches {
    pub modules: Vec<DebugModuleSource>,
    /// Changes when the set of supported source roots observed by this resolver changes.
    pub source_sets_revision: u64,
}

/// A resolver belongs to one DAP session and may retain evidence for unchanged source files.
pub type ResolveDebugModules = Box<
    dyn FnMut(
        DebugModuleQuery<'_>,
    ) -> Result<DebugModuleMatches, Box<dyn std::error::Error + Send + Sync>>,
>;
