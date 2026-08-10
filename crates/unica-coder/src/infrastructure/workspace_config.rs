use toml::Table;

const ROOT_FIELDS: &[&str] = &["version", "operational", "network", "providers"];
const SUPPORTED_VERSION: i64 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WorkspaceConfigRootErrorKind {
    InvalidToml,
    MissingVersion,
    InvalidVersionType,
    UnsupportedVersion,
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
    validate_present_version(&root)?;
    if root.contains_key("operational") && !root.contains_key("version") {
        return Err(missing_version());
    }
    Ok(root)
}

fn invalid_toml() -> WorkspaceConfigRootError {
    WorkspaceConfigRootError {
        kind: WorkspaceConfigRootErrorKind::InvalidToml,
        field_path: "$".to_string(),
    }
}

fn missing_version() -> WorkspaceConfigRootError {
    WorkspaceConfigRootError {
        kind: WorkspaceConfigRootErrorKind::MissingVersion,
        field_path: "version".to_string(),
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

fn validate_present_version(root: &Table) -> Result<(), WorkspaceConfigRootError> {
    let Some(value) = root.get("version") else {
        return Ok(());
    };
    let Some(version) = value.as_integer() else {
        return Err(WorkspaceConfigRootError {
            kind: WorkspaceConfigRootErrorKind::InvalidVersionType,
            field_path: "version".to_string(),
        });
    };
    if version != SUPPORTED_VERSION {
        return Err(WorkspaceConfigRootError {
            kind: WorkspaceConfigRootErrorKind::UnsupportedVersion,
            field_path: "version".to_string(),
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_legacy_network_only_root_without_version() {
        let root = parse_workspace_config_root("[network]\ndefault = \"deny\"\n")
            .expect("legacy network-only root");

        assert!(root.contains_key("network"));
    }

    #[test]
    fn requires_version_only_when_operational_subtree_is_present() {
        let error = parse_workspace_config_root(
            "[operational.code_intelligence]\nsearch_total_timeout_seconds = 90\n",
        )
        .expect_err("operational subtree without version must fail");

        assert_eq!(error.kind(), WorkspaceConfigRootErrorKind::MissingVersion);
        assert_eq!(error.field_path(), "version");
    }

    #[test]
    fn validates_any_present_version_and_unknown_root_field() {
        let cases = [
            (
                "version = \"one\"\n",
                WorkspaceConfigRootErrorKind::InvalidVersionType,
                "version",
            ),
            (
                "version = 2\n",
                WorkspaceConfigRootErrorKind::UnsupportedVersion,
                "version",
            ),
            (
                "version = 1\nunrelated = true\n",
                WorkspaceConfigRootErrorKind::UnknownField,
                "unrelated",
            ),
        ];

        for (contents, kind, field_path) in cases {
            let error = parse_workspace_config_root(contents).expect_err("invalid root");
            assert_eq!(error.kind(), kind);
            assert_eq!(error.field_path(), field_path);
        }
    }
}
