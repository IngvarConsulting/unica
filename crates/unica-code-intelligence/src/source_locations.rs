//! Current source containment and addressed result projection shared by providers.
use crate::model::CodeIntelligenceContext;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use unica_source_contract::location::SourceLocation;
use unica_source_contract::workspace::WorkspaceContext;
use unica_work_control::cancellation::{cancelled_error, CancellationToken, CANCELLED_PREFIX};
#[derive(Clone, Copy)]
pub struct SearchSourceBindings {
    pub resolver: SearchLocationResolver,
    pub engine_absent: fn(&Path, &str) -> bool,
}
pub struct SearchLocationProjector<'a> {
    context: &'a CodeIntelligenceContext,
    cancellation: &'a CancellationToken,
    resolver: SearchLocationResolver,
    cache: HashMap<PathBuf, SourceLocation>,
}

pub type SearchLocationResolver =
    fn(
        &WorkspaceContext,
        &unica_navigation::source_navigation::SourceLocateRequest,
        &CancellationToken,
    ) -> Result<unica_navigation::source_navigation::SourceLocateResult, String>;

impl<'a> SearchLocationProjector<'a> {
    pub fn with_resolver(
        context: &'a CodeIntelligenceContext,
        cancellation: &'a CancellationToken,
        resolver: SearchLocationResolver,
    ) -> Self {
        Self {
            context,
            cancellation,
            resolver,
            cache: HashMap::new(),
        }
    }

    pub fn project(&mut self, provider_path: &Path) -> Result<SourceLocation, String> {
        if self.cancellation.is_cancelled() {
            return Err(cancelled_error("search result location projection stopped"));
        }
        let relative_path = contained_provider_relative_path(self.context, provider_path)?;
        if self
            .context
            .search_scope
            .as_ref()
            .is_some_and(|scope| !scope.accepts(&relative_path))
        {
            return Err("provider path is outside the logical search scope".to_string());
        }
        if let Some(location) = self.cache.get(&relative_path) {
            return Ok(location.clone());
        }

        let source_set = self
            .context
            .source_root
            .source_set
            .clone()
            .unwrap_or_else(|| "legacy".to_string());
        let relative_text = relative_path.to_string_lossy().replace('\\', "/");
        let request = unica_navigation::source_navigation::SourceLocateRequest {
            source_set: source_set.clone(),
            path: relative_text.clone(),
        };
        let location = match (self.resolver)(&self.context.workspace, &request, self.cancellation) {
            Ok(located) if located.rejection.is_none() => SourceLocation::Addressed {
                source_set,
                metadata_path: located.metadata_path,
                target_kind: located
                    .target_kind
                    .unwrap_or(unica_address::source_target::TargetKind::SourceRoot),
            },
            Ok(located)
                if located.rejection
                    == Some(unica_source_contract::location::LocateRejection::OutsideSourceSet) =>
            {
                return Err("provider path is outside sourceSet".to_string());
            }
            Ok(located) => SourceLocation::Unaddressable {
                source_set,
                owner_metadata_path: located.owner_metadata_path,
                path: located.relative_path,
            },
            Err(error) if error.starts_with(CANCELLED_PREFIX) => return Err(error),
            Err(_) => SourceLocation::Unaddressable {
                source_set,
                owner_metadata_path: None,
                path: relative_text,
            },
        };
        self.cache.insert(relative_path, location.clone());
        Ok(location)
    }
}

pub fn contained_provider_relative_path(
    context: &CodeIntelligenceContext,
    provider_path: &Path,
) -> Result<PathBuf, String> {
    let raw_text = provider_path.to_string_lossy().replace('\\', "/");
    if raw_text.is_empty() || unica_workspace::paths::is_foreign_absolute_path(&raw_text) {
        return Err("provider path is outside sourceSet".to_string());
    }
    let normalized_root =
        unica_workspace::paths::normalize_path_identity(&context.source_root.path)
            .map_err(|_| "sourceSet path identity is unavailable".to_string())?;
    let raw = Path::new(&raw_text);
    let candidates = if raw.is_absolute() {
        vec![raw.to_path_buf()]
    } else {
        vec![
            context.workspace.workspace_root.join(raw),
            context.source_root.path.join(raw),
        ]
    };
    for candidate in candidates {
        let Ok(normalized) = unica_workspace::paths::normalize_path_identity(&candidate) else {
            continue;
        };
        let Ok(relative) = normalized.strip_prefix(&normalized_root) else {
            continue;
        };
        if relative.as_os_str().is_empty()
            || relative
                .components()
                .any(|component| !matches!(component, std::path::Component::Normal(_)))
        {
            continue;
        }
        return Ok(relative.to_path_buf());
    }
    Err("provider path is outside sourceSet".to_string())
}
