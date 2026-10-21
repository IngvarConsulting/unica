use super::test_support::*;
use super::*;

pub fn warm_registry_reuses_the_same_actor_across_sequential_admissions() {
    let root = temp_root("warm-reuse");
    std::fs::create_dir_all(root.join("src")).unwrap();
    let registry = registry_for_test();
    assert_eq!(registry.warm_ttl, Duration::from_secs(600));
    let context = context(&root);
    let first = registry
        .get_or_create(&context, [source_input("main", root.join("src"))], "p")
        .unwrap();
    let first = {
        let weak = Arc::downgrade(&first);
        drop(first);
        weak
    };
    let second = registry
        .get_or_create(&context, [source_input("main", root.join("src"))], "p")
        .unwrap();
    let first = first
        .upgrade()
        .expect("a released actor must stay warm for the next sequential admission");
    assert!(Arc::ptr_eq(&first, &second));
    assert_eq!(registry.warm_len_for_test().unwrap(), 1);
    assert_eq!(registry.live_len_for_test().unwrap(), 1);

    let main = Arc::downgrade(&second);
    drop((first, second));
    let mut others = Vec::new();
    for index in 0..7 {
        let source = root.join(format!("source-{index}"));
        std::fs::create_dir_all(&source).unwrap();
        let actor = registry
            .get_or_create(&context, [source_input("main", source)], "p")
            .unwrap();
        others.push(Arc::downgrade(&actor));
    }
    // Another profile must not evict an otherwise usable actor.
    let reused = registry
        .get_or_create(&context, [source_input("main", root.join("src"))], "p")
        .unwrap();
    assert!(Arc::ptr_eq(&main.upgrade().unwrap(), &reused));
    drop(reused);
    let source = root.join("extra");
    std::fs::create_dir_all(&source).unwrap();
    drop(
        registry
            .get_or_create(&context, [source_input("main", source)], "p")
            .unwrap(),
    );
    assert!(
        main.upgrade().is_some(),
        "recently used actor must stay warm"
    );
    assert!(
        others[0].upgrade().is_some(),
        "another profile must not evict a retained actor"
    );
    assert!(others[1..].iter().all(|actor| actor.upgrade().is_some()));
    assert_eq!(registry.warm_len_for_test().unwrap(), 9);
}

pub fn warm_actor_expires_after_the_idle_ttl_and_is_rebuilt() {
    let root = temp_root("warm-ttl");
    std::fs::create_dir_all(root.join("src")).unwrap();
    let registry = idle_registry_for_test(Duration::ZERO);
    let context = context(&root);
    let first = registry
        .get_or_create(&context, [source_input("main", root.join("src"))], "p")
        .unwrap();
    assert!(
        registry.has_retained_work(),
        "a live call may still publish a promise"
    );
    let first = {
        let weak = Arc::downgrade(&first);
        drop(first);
        weak
    };
    assert_eq!(registry.warm_len_for_test().unwrap(), 1);
    assert!(
        !registry.has_retained_work(),
        "an empty warm actor alone must not hold the listener"
    );
    registry.evict_idle_warm_actors().unwrap();
    assert_eq!(registry.warm_len_for_test().unwrap(), 0);
    assert_eq!(registry.live_len_for_test().unwrap(), 0);
    assert!(
        first.upgrade().is_none(),
        "an expired warm actor must be released, not handed to a later admission"
    );
    let second = registry
        .get_or_create(&context, [source_input("main", root.join("src"))], "p")
        .unwrap();
    assert_eq!(registry.warm_len_for_test().unwrap(), 1);
    drop(second);
}

pub fn warm_actor_whose_named_root_was_replaced_is_rebuilt() {
    let root = temp_root("warm-replaced-root");
    std::fs::create_dir_all(root.join("src")).unwrap();
    let registry = registry_for_test();
    let context = context(&root);
    let first = registry
        .get_or_create(&context, [source_input("main", root.join("src"))], "p")
        .unwrap();
    let first = {
        let weak = Arc::downgrade(&first);
        drop(first);
        weak
    };
    std::fs::rename(root.join("src"), root.join("src-old")).unwrap();
    std::fs::create_dir_all(root.join("src")).unwrap();
    assert!(
        first.upgrade().is_some(),
        "the stale actor is still warm before admission"
    );
    let second = registry
        .get_or_create(&context, [source_input("main", root.join("src"))], "p")
        .unwrap();
    assert!(
        first.upgrade().is_none(),
        "a warm actor whose root directory was replaced must be released and rebuilt"
    );
    assert!(second.retains_named_roots().unwrap());
    assert_eq!(registry.warm_len_for_test().unwrap(), 1);
}

pub fn warm_actors_do_not_block_a_distinct_identity() {
    let root = temp_root("warm-capacity");
    let registry = registry_for_test();
    let mut actors = Vec::new();
    for index in 0..3 {
        let workspace = root.join(format!("workspace-{index}"));
        std::fs::create_dir_all(workspace.join("src")).unwrap();
        let actor = registry
            .get_or_create(
                &context(&workspace),
                [source_input("main", workspace.join("src"))],
                "p",
            )
            .unwrap_or_else(|error| {
                panic!("warm actors must not block a distinct admission: {error}")
            });
        actors.push(Arc::downgrade(&actor));
    }
    assert_eq!(registry.live_len_for_test().unwrap(), 3);
    assert!(
        actors[2].upgrade().is_some(),
        "the newest admission stays warm alongside earlier idle actors"
    );
}
