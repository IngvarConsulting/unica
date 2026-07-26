use std::path::PathBuf;

use crate::domain::source_adapters::{
    FormatVersion, SourceBinding, SourceContext, SourceFamily, SourceLocation, SourceSnapshot,
};

pub(crate) mod registry;

pub(crate) struct SourceInput {
    pub(crate) workspace_root: PathBuf,
    pub(crate) source_root: PathBuf,
    pub(crate) target: PathBuf,
    pub(crate) configured_source_set: Option<String>,
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
    }
}

pub(crate) trait CapturedSourceSession: Send + Sync {
    fn binding(&self) -> &SourceBinding;
    fn source(&self) -> &SourceContext;
    fn snapshot(&self) -> &SourceSnapshot;
}
