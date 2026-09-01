use crate::domain::events::DomainEvent;
use crate::infrastructure::native_operations::apply::{ApplyStagedState, PlannedApplyEffects};
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub(crate) struct IndexedPlanOperation<T> {
    index: usize,
    operation: T,
}

impl<T> IndexedPlanOperation<T> {
    pub(crate) fn new(index: usize, operation: T) -> Self {
        Self { index, operation }
    }

    pub(crate) const fn index(&self) -> usize {
        self.index
    }

    pub(crate) fn operation(&self) -> &T {
        &self.operation
    }
}

/// Operation-local evidence retained until the whole request postimage is
/// known. The request finalizer, rather than a family planner, owns conversion
/// into the actor-facing effect receipt.
#[derive(Debug)]
pub(crate) struct ProvisionalApplyEffect {
    paths: Vec<PathBuf>,
    event: DomainEvent,
    operation_index: usize,
}

impl ProvisionalApplyEffect {
    pub(crate) fn single(
        path: impl Into<PathBuf>,
        event: DomainEvent,
        operation_index: usize,
    ) -> Self {
        Self {
            paths: vec![path.into()],
            event,
            operation_index,
        }
    }

    pub(crate) fn event(&self) -> &DomainEvent {
        &self.event
    }

    pub(crate) const fn operation_index(&self) -> usize {
        self.operation_index
    }
}

/// Converts provisional family effects into one request-level effect receipt.
/// A candidate survives only when every path it touched is still changed in
/// the final staged postimage; stable deduplication happens afterwards.
pub(crate) fn reconcile_effects(
    staged: &ApplyStagedState,
    mut provisional: Vec<ProvisionalApplyEffect>,
) -> PlannedApplyEffects {
    let changed_paths = staged
        .planned_changes()
        .into_iter()
        .map(|change| change.relative_path)
        .collect::<std::collections::BTreeSet<_>>();
    let mut effects = PlannedApplyEffects::default();
    provisional.sort_by_key(ProvisionalApplyEffect::operation_index);
    for candidate in provisional {
        if !candidate.paths.is_empty()
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
