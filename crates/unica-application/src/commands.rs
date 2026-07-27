use serde_json::Value;
use unica_format_core::{
    navigation::ObjectKey,
    source::{SourceId, SourceRevision},
};

#[derive(Debug, Clone)]
pub struct MetadataNavigationCommand {
    pub target: MetadataNavigationTarget,
    pub selection: Option<Value>,
}

#[derive(Debug, Clone)]
pub enum MetadataNavigationTarget {
    ObjectPath(String),
    ObjectRef {
        source_id: SourceId,
        object_key: ObjectKey,
        snapshot_revision: SourceRevision,
    },
    Cursor(Value),
}
