//! Drops of the test fixture never panic: a broken invariant fails the test
//! when its owner is drained, not as an abort during unwinding.

use super::*;

#[test]
fn a_broken_admission_count_fails_the_drain_instead_of_the_drop() {
    let root = std::env::temp_dir().join(format!("unica-fixture-drop-{}", Uuid::new_v4()));
    let owner = create_test_fixture_owner(root.clone());
    let admission = acquire_test_service_admission(&root).expect("bound root admits");
    test_fixture_registry()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .owners
        .get_mut(&owner)
        .expect("live owner")
        .live_admissions = 0;

    // The underflow is recorded, not panicked: this Drop may run while unwinding.
    drop(admission);

    let drained = std::panic::catch_unwind(|| drain_test_fixture_owner(owner))
        .expect_err("the recorded violation fails the test at the drain");
    let message = drained
        .downcast_ref::<String>()
        .expect("formatted panic message");
    assert!(
        message.contains("test fixture service admission count underflow"),
        "{message}"
    );
}
