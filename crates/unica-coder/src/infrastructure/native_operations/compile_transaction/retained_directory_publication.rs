use super::super::retained_directory::{DirectoryRemoval, PublishedDirectoryRemoval};
use super::*;

fn directory_validation_error(
    error: super::super::apply::ApplyStagingError,
    path: &Path,
) -> RetainedApplyValidationError {
    retained_guard_validation_error(error, path, ApplyInputRole::DirectoryMembership)
}

impl CompileTransaction {
    pub(in crate::infrastructure::native_operations) fn bind_retained_directory_removals(
        &mut self,
        plans: Vec<DirectoryRemoval>,
    ) {
        self.retained_directory_removals = plans;
    }

    pub(super) fn directory_contains(&self, entry: &PlannedRetainedApplyChange) -> bool {
        self.retained_directory_removals.iter().any(|plan| {
            plan.root.identity() == entry.root.identity() && plan.contains(&entry.relative_path)
        })
    }

    pub(super) fn validate_directory_plans(&self) -> Result<(), RetainedApplyValidationError> {
        for (index, plan) in self.retained_directory_removals.iter().enumerate() {
            let invariant = self
                .retained_apply_root
                .as_ref()
                .is_none_or(|root| root.identity() != plan.root.identity())
                || self.retained_directory_removals[..index]
                    .iter()
                    .any(|prior| prior.contains(&plan.relative) || plan.contains(&prior.relative))
                || self.retained_apply.iter().any(|entry| {
                    plan.root.identity() == entry.root.identity()
                        && plan.contains(&entry.relative_path)
                        && entry.current.is_some()
                })
                || self.retained_apply_read_guards.iter().any(|guard| {
                    plan.root.identity() == guard.root.identity()
                        && plan.contains(&guard.relative_path)
                });
            if invariant {
                return Err(RetainedApplyValidationError::new(RetainedApplyValidationErrorKind::Invariant,
                    "directory removal overlaps another participant, surviving image, or read-only guard"));
            }
            plan.validate_source()
                .map_err(|error| directory_validation_error(error, &plan.relative))?;
        }
        Ok(())
    }

    pub(super) fn validate_directory_cache_boundary(
        &self,
        cache: &Self,
        participant: &WorkspaceCacheParticipantAuthority,
    ) -> Result<(), String> {
        if !cache.retained_directory_removals.is_empty() {
            return Err(
                "workspace-cache participant cannot supply source directory removals".to_string(),
            );
        }
        for plan in &self.retained_directory_removals {
            let source = plan.root.path().join(&plan.relative);
            if participant.logical_root().starts_with(&source)
                || cache.retained_apply.iter().any(|entry| {
                    let cache_target = entry.root.path().join(&entry.relative_path);
                    cache_target.starts_with(&source) || source.starts_with(&cache_target)
                })
            {
                return Err(
                    "source directory and workspace-cache participant targets overlap".to_string(),
                );
            }
        }
        Ok(())
    }

    pub(super) fn publish_directories(
        &self,
        journal: &mut Vec<PublishedDirectoryRemoval>,
    ) -> Result<(), ApplyPublicationError> {
        for plan in &self.retained_directory_removals {
            match plan.publish() {
                Ok(change) => journal.push(change),
                Err(failure) => {
                    if let Some(change) = failure.artifact {
                        journal.push(*change);
                    }
                    return Err(retained_apply_validation_publication_error(
                        directory_validation_error(failure.error, &plan.relative),
                    ));
                }
            }
            #[cfg(test)]
            {
                let event = RetainedApplyObservedEvent::Source(plan.relative.clone());
                RETAINED_APPLY_OBSERVED_EVENTS
                    .with(|events| events.borrow_mut().push(event.clone()));
                retained_apply_fail_after(&event).map_err(|error| {
                    ApplyPublicationError::new(
                        ApplyPublicationErrorKind::ProviderPostvalidation,
                        error,
                    )
                })?;
            }
        }
        Ok(())
    }

    pub(super) fn validate_removed_directories(
        journal: &[PublishedDirectoryRemoval],
    ) -> Result<(), ApplyPublicationError> {
        for change in journal {
            change.validate().map_err(|error| {
                retained_apply_validation_publication_error(directory_validation_error(
                    error,
                    &change.plan.relative,
                ))
            })?;
        }
        Ok(())
    }

    pub(super) fn rollback_removed_directories(
        journal: &[PublishedDirectoryRemoval],
        errors: &mut Vec<String>,
    ) {
        for change in journal.iter().rev() {
            if let Err(error) = change.rollback() {
                errors.push(format!(
                    "directory {} recovery {}: {error}",
                    change.plan.relative.display(),
                    change.recovery.path().display()
                ));
            }
        }
    }

    pub(super) fn finalize_removed_directories(
        journal: &[PublishedDirectoryRemoval],
        report: &mut CommitReport,
    ) {
        for change in journal {
            if let Err(error) = change.cleanup() {
                report.cleanup_warnings.push(format!(
                    "retained directory recovery was preserved after success: {error}"
                ));
                report
                    .retained_apply_cleanup_diagnostics
                    .push(RetainedApplyCleanupDiagnostic {
                        logical_target: change.plan.relative.clone(),
                        artifact_name: change.recovery_name.clone(),
                    });
            }
        }
    }
}
