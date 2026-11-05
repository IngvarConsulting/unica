use super::*;
use crate::actor::test_support::{actor_fixture, policies_scope, snapshot_tree, ActorFixture};
use std::sync::atomic::AtomicBool;
use unica_work_control::cancellation::CancellationToken;
use unica_work_control::provider_deadline::ProviderDeadline;

struct RestoreFaults {
    failpoint: Option<RetainedApplyFailpoint>,
    rollback: Option<RetainedApplyBeforeRollbackHook>,
    observations: Vec<RetainedApplyObservedEvent>,
}
impl RestoreFaults {
    fn start() -> Self {
        Self {
            failpoint: RETAINED_APPLY_FAILPOINT.with(|slot| slot.replace(None)),
            observations: RETAINED_APPLY_OBSERVED_EVENTS.with(|slot| slot.replace(Vec::new())),
            rollback: TEST_RETAINED_APPLY_BEFORE_ROLLBACK_HOOK.with(|slot| slot.replace(None)),
        }
    }
}
impl Drop for RestoreFaults {
    fn drop(&mut self) {
        RETAINED_APPLY_OBSERVED_EVENTS.with(|slot| {
            slot.replace(std::mem::take(&mut self.observations));
        });
        RETAINED_APPLY_FAILPOINT.with(|slot| {
            slot.replace(self.failpoint.take());
        });
        TEST_RETAINED_APPLY_BEFORE_ROLLBACK_HOOK.with(|slot| {
            slot.replace(self.rollback.take());
        });
    }
}
struct CleanupFixture(Option<ActorFixture>);
impl Drop for CleanupFixture {
    fn drop(&mut self) {
        if let Some(ActorFixture { root, actor, .. }) = self.0.take() {
            drop(actor);
            let _ = fs::remove_dir_all(root);
        }
    }
}

pub fn retained_rollback_cleans_new_directories_without_deleting_foreign_children(
    policies: crate::policies::WorkspacePolicies,
) {
    let _policies = policies_scope(policies);
    for foreign in [false, true] {
        let _faults = RestoreFaults::start();
        let owned = CleanupFixture(Some(actor_fixture(
            &format!("nested-rollback-{foreign}"),
            &["src"],
        )));
        let fixture = owned.0.as_ref().unwrap();
        fs::write(fixture.roots[0].join("Stable.bsl"), b"keep stable").unwrap();
        let binding = fixture
            .actor
            .bind_provider_root("src", &fixture.roots[0])
            .unwrap();
        let admitted = fixture
            .actor
            .admit_apply(
                &binding,
                None,
                false,
                ProviderDeadline::from_budget(std::time::Duration::from_secs(5)),
                &CancellationToken::new(),
            )
            .unwrap();
        let mut state = admitted.staged_state().unwrap();
        state
            .create("Deep/Inner/New.bsl", b"published nested resource".to_vec())
            .unwrap();
        let mut effects = unica_source_contract::proposal::PlannedApplyEffects::default();
        effects.append(unica_source_contract::events::DomainEvent::new(
            unica_source_contract::events::DomainEventKind::MetadataChanged,
            "Catalog.Products",
        ));
        let prepared = admitted.prepare_with_effects(state, effects).unwrap();
        let source_before = snapshot_tree(&fixture.roots[0]);
        let cache_before = snapshot_tree(&fixture.root.join(".build/unica"));
        assert!(!fixture.roots[0].join("Deep").exists());
        let reached = Arc::new(AtomicBool::new(false));
        let observed = Arc::clone(&reached);
        let inner = fixture.roots[0].join("Deep/Inner");
        set_retained_apply_before_rollback_hook(move || {
            assert!(
                inner.is_dir(),
                "the publisher did not create the missing chain"
            );
            assert_eq!(
                fs::read(inner.join("New.bsl")).unwrap(),
                b"published nested resource"
            );
            observed.store(true, Ordering::Release);
            if foreign {
                fs::write(inner.join("foreign.txt"), b"foreign bytes must survive").unwrap();
            }
        });
        set_retained_apply_failpoint(RetainedApplyFailpoint::AfterAllPostimages);
        let error = fixture.actor.publish_prepared_apply(prepared).unwrap_err();
        assert!(
            reached.load(Ordering::Acquire),
            "late failure did not reach the real published chain"
        );
        assert!(!fixture.roots[0].join("Deep/Inner/New.bsl").exists());
        assert_eq!(
            snapshot_tree(&fixture.root.join(".build/unica")),
            cache_before
        );
        if foreign {
            assert_eq!(error.kind(), ApplyPublicationErrorKind::RollbackIncomplete);
            let mut expected = source_before.into_iter().collect::<BTreeMap<_, _>>();
            expected.insert(PathBuf::from("Deep"), None);
            expected.insert(PathBuf::from("Deep/Inner"), None);
            expected.insert(
                PathBuf::from("Deep/Inner/foreign.txt"),
                Some(b"foreign bytes must survive".to_vec()),
            );
            assert_eq!(
                snapshot_tree(&fixture.roots[0])
                    .into_iter()
                    .collect::<BTreeMap<_, _>>(),
                expected
            );
        } else {
            assert_eq!(
                error.kind(),
                ApplyPublicationErrorKind::ProviderPostvalidation
            );
            assert_eq!(snapshot_tree(&fixture.roots[0]), source_before);
            assert!(
                !fixture.roots[0].join("Deep").exists(),
                "rollback left an empty directory created by the party"
            );
        }
    }
}
