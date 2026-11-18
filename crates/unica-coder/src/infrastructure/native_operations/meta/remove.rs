//! Trusted binding of the existing removal race-test observations.
use unica_metadata_designer::remove::RemoveObservationHooks;
pub(crate) fn observation_hooks() -> RemoveObservationHooks {
    #[cfg(any(test, feature = "test-support"))]
    {
        RemoveObservationHooks {
            before_child_inspection: super::run_before_meta_remove_subsystem_child_inspection_hook,
            additional_reparse_observation: |path| {
                super::META_REMOVE_FORCED_REPARSE_PATHS.with(|slot| slot.borrow().contains(path))
            },
        }
    }
    #[cfg(not(any(test, feature = "test-support")))]
    {
        RemoveObservationHooks::NONE
    }
}
