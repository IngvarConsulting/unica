use super::{PlannedApplyEffects, StagedApplyChange};
use crate::events::DomainEvent;
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct IndexedPlanOperation<T> {
    index: usize,
    operation: T,
}

impl<T> IndexedPlanOperation<T> {
    pub fn new(index: usize, operation: T) -> Self {
        Self { index, operation }
    }

    pub const fn index(&self) -> usize {
        self.index
    }

    pub fn operation(&self) -> &T {
        &self.operation
    }
}

/// Operation-local evidence retained until the whole request postimage is
/// known. The request finalizer, rather than a family planner, owns conversion
/// into the actor-facing effect receipt.
#[derive(Debug)]
pub struct ProvisionalApplyEffect {
    paths: Vec<PathBuf>,
    event: DomainEvent,
    operation_index: usize,
    /// Typed warnings the planner wants to travel with the plan even when
    /// the effect itself is reconciled away.
    warnings: Vec<serde_json::Value>,
}

impl ProvisionalApplyEffect {
    pub fn single(path: impl Into<PathBuf>, event: DomainEvent, operation_index: usize) -> Self {
        Self {
            paths: vec![path.into()],
            event,
            operation_index,
            warnings: Vec::new(),
        }
    }

    /// An effect a family planner ties to every path its batch changed.
    pub fn spanning(paths: Vec<PathBuf>, event: DomainEvent, operation_index: usize) -> Self {
        Self {
            paths,
            event,
            operation_index,
            warnings: Vec::new(),
        }
    }

    /// Attaches a typed warning that reaches the caller with the plan.
    pub fn with_warning(mut self, warning: serde_json::Value) -> Self {
        self.warnings.push(warning);
        self
    }

    pub fn event(&self) -> &DomainEvent {
        &self.event
    }

    pub const fn operation_index(&self) -> usize {
        self.operation_index
    }
}

/// Converts provisional family effects into one request-level effect receipt.
/// A candidate survives only when every path it touched is still changed in
/// the final staged postimage; stable deduplication happens afterwards.
pub fn reconcile_effects(
    changes: impl IntoIterator<Item = StagedApplyChange>,
    mut provisional: Vec<ProvisionalApplyEffect>,
) -> PlannedApplyEffects {
    let changed_paths = changes
        .into_iter()
        .map(|change| change.relative_path)
        .collect::<std::collections::BTreeSet<_>>();
    let mut effects = PlannedApplyEffects::default();
    provisional.sort_by_key(ProvisionalApplyEffect::operation_index);
    for candidate in provisional {
        for warning in candidate.warnings.iter().cloned() {
            effects.push_warning(warning);
        }
        if !candidate.paths.is_empty()
            // Every file the effect stands for must still differ; planners
            // name the file behind each event, so restoring one module of a
            // batch silences only that module's event.
            && candidate
                .paths
                .iter()
                .all(|path| changed_paths.contains(path))
        {
            effects.append(candidate.event);
        }
    }
    effects
}
