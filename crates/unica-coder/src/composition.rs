use crate::application::UnicaApplication;
use crate::infrastructure::application_ports::InfrastructureApplicationPorts;
use std::sync::Arc;

impl UnicaApplication {
    pub fn new() -> Self {
        Self::with_ports(Arc::new(InfrastructureApplicationPorts))
    }
}

impl Default for UnicaApplication {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::input_schema_for_tool;
    use serde_json::{Map, Value};

    #[test]
    fn new_and_default_have_identical_tools_and_deterministic_dry_run() {
        let created = UnicaApplication::new();
        let defaulted = UnicaApplication::default();
        let created_tools = created.tools();
        let defaulted_tools = defaulted.tools();

        assert_eq!(created_tools.len(), defaulted_tools.len());
        for (left, right) in created_tools.iter().zip(&defaulted_tools) {
            assert_eq!(left.name, right.name);
            assert_eq!(left.description, right.description);
            assert_eq!(left.mutating, right.mutating);
            assert_eq!(left.cache_access.reads, right.cache_access.reads);
            assert_eq!(left.cache_access.writes, right.cache_access.writes);
            assert_eq!(input_schema_for_tool(left), input_schema_for_tool(right));
        }

        let root =
            std::env::temp_dir().join(format!("unica-composition-default-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let mut args = Map::new();
        args.insert("cwd".to_string(), Value::String(root.display().to_string()));

        let created_result = created.call_tool("unica.form.edit", &args).unwrap();
        let defaulted_result = defaulted.call_tool("unica.form.edit", &args).unwrap();

        assert_eq!(
            serde_json::to_value(created_result).unwrap(),
            serde_json::to_value(defaulted_result).unwrap()
        );
        let _ = std::fs::remove_dir_all(root);
    }
}
