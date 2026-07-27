use std::path::PathBuf;

use crate::domain::source_adapters::{
    ConfiguredSourceSetKind, FormatVersion, SourceContext, SourceFamily, SourceLocation,
};

pub(crate) mod registry;

pub(crate) struct SourceInput {
    pub(crate) workspace_root: PathBuf,
    pub(crate) source_root: PathBuf,
    pub(crate) target: PathBuf,
    pub(crate) configured_source_set: Option<String>,
    pub(crate) configured_source_set_kind: Option<ConfiguredSourceSetKind>,
    pub(crate) declared_family: SourceFamily,
    pub(crate) declared_format: Option<FormatVersion>,
}

impl SourceInput {
    pub(crate) fn source_context(&self) -> SourceContext {
        SourceContext::new(
            SourceLocation::new(
                self.workspace_root.clone(),
                self.source_root.clone(),
                self.target.clone(),
            ),
            self.configured_source_set.clone(),
            self.declared_family.clone(),
            self.declared_format.clone(),
        )
        .with_configured_source_set_kind(self.configured_source_set_kind)
    }
}
