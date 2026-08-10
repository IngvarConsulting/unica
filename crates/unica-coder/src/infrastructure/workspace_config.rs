use toml::Table;

const ROOT_FIELDS: &[&str] = &["operational", "network", "providers"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WorkspaceConfigRootErrorKind {
    InvalidToml,
    UnknownField,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct WorkspaceConfigRootError {
    kind: WorkspaceConfigRootErrorKind,
    field_path: String,
}

impl WorkspaceConfigRootError {
    pub(crate) const fn kind(&self) -> WorkspaceConfigRootErrorKind {
        self.kind
    }

    pub(crate) fn field_path(&self) -> &str {
        &self.field_path
    }
}

pub(crate) fn parse_workspace_config_root(
    contents: &str,
) -> Result<Table, WorkspaceConfigRootError> {
    let root = contents.parse::<Table>().map_err(|_| invalid_toml())?;
    reject_unknown_root_fields(&root)?;
    Ok(root)
}

fn invalid_toml() -> WorkspaceConfigRootError {
    WorkspaceConfigRootError {
        kind: WorkspaceConfigRootErrorKind::InvalidToml,
        field_path: "$".to_string(),
    }
}

fn reject_unknown_root_fields(root: &Table) -> Result<(), WorkspaceConfigRootError> {
    let Some(unknown) = root.keys().find(|key| !ROOT_FIELDS.contains(&key.as_str())) else {
        return Ok(());
    };
    Err(WorkspaceConfigRootError {
        kind: WorkspaceConfigRootErrorKind::UnknownField,
        field_path: unknown.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_operational_root_without_version() {
        let root = parse_workspace_config_root(
            "[operational.code_intelligence]\nsearch_total_timeout_seconds = 90\n",
        )
        .expect("unversioned operational root");
        assert!(root.contains_key("operational"));
    }

    #[test]
    fn rejects_version_as_unknown_root_field() {
        let error = parse_workspace_config_root("version = 1\n")
            .expect_err("version is outside the fixed schema");
        assert_eq!(error.kind(), WorkspaceConfigRootErrorKind::UnknownField);
        assert_eq!(error.field_path(), "version");
    }
}
