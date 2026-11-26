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
    FirstSaved { source_set: Option<&'a str> },
    Address(&'a str),
    Runtime(&'a DebugModuleId),
}
