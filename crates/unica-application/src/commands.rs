use unica_format_core::{
    navigation::{NavigationSelection, ObjectKey, OpaqueNavigationCursor},
    source::{SourceId, SourceRevision},
};

#[derive(Debug, Clone)]
pub struct MetadataNavigationCommand {
    pub target: MetadataNavigationTarget,
    pub selection: Option<NavigationSelection>,
}

#[derive(Debug, Clone)]
pub enum MetadataNavigationTarget {
    Source,
    ObjectRef {
        source_id: SourceId,
        object_key: ObjectKey,
        snapshot_revision: SourceRevision,
    },
    Cursor(OpaqueNavigationCursor),
}

#[cfg(test)]
mod tests {
    #[test]
    fn application_command_boundary_has_no_json_or_filesystem_transport_shapes() {
        let commands = include_str!("commands.rs");
        let orchestration = include_str!("navigation.rs");

        assert!(!commands.contains(concat!("serde_json", "::Value")));
        assert!(!commands.contains(concat!("ObjectPath", "(String)")));
        assert!(!orchestration.contains("source_target_path"));
    }
}
