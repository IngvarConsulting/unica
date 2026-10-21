use super::*;
use std::cell::Cell;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;
use unica_source_contract::events::DomainEventKind;

pub fn source_input(name: &str, root: impl AsRef<Path>) -> WorkspaceSourceSetInput {
    WorkspaceSourceSetInput::new(
        name,
        root.as_ref(),
        SourceSetKind::Configuration,
        SourceFormat::PlatformXml,
        SourceProfile::platform_xml_8_3_27_format_2_20(),
    )
}

pub fn support_policy_actor_test_now() -> Instant {
    SUPPORT_POLICY_ACTOR_TEST_NOW.get()
}

pub fn set_support_policy_actor_test_now(now: Instant) {
    SUPPORT_POLICY_ACTOR_TEST_NOW.set(now);
}

pub fn reset_support_policy_actor_test_now() {
    set_support_policy_actor_test_now(Instant::now());
}

pub fn retained_apply_deadline_test_now() -> Instant {
    RETAINED_APPLY_DEADLINE_TEST_NOW.get()
}

pub fn retained_apply_recovery_path(parent: &Path) -> PathBuf {
    std::fs::read_dir(parent)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .find(|path| {
            path.file_name()
                .and_then(std::ffi::OsStr::to_str)
                .is_some_and(|name| name.starts_with(".unica-apply-"))
        })
        .expect("source publication must retain one recovery sibling")
}

pub fn publish_cache_fixture(name: &str) -> (ActorFixture, super::ProviderRootBinding) {
    let fixture = actor_fixture(name, &["src"]);
    std::fs::write(fixture.roots[0].join("Module.bsl"), b"baseline").unwrap();
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
            ProviderDeadline::from_budget(Duration::from_secs(5)),
            &CancellationToken::new(),
        )
        .unwrap();
    let state = admitted.staged_state().unwrap();
    fixture
        .actor
        .publish_prepared_apply(
            admitted
                .prepare_with_cache_effects(
                    state,
                    &[unica_source_contract::events::DomainEvent::new(
                        unica_source_contract::events::DomainEventKind::MetadataChanged,
                        "Catalog.Products",
                    )],
                )
                .unwrap(),
        )
        .unwrap();
    (fixture, binding)
}

pub fn prepare_cache_fixture_apply(
    fixture: &ActorFixture,
    binding: &super::ProviderRootBinding,
) -> super::PreparedApplyBatch {
    let admitted = fixture
        .actor
        .admit_apply(
            binding,
            None,
            false,
            ProviderDeadline::from_budget(Duration::from_secs(5)),
            &CancellationToken::new(),
        )
        .unwrap();
    let mut state = admitted.staged_state().unwrap();
    state
        .replace("Module.bsl", b"baseline", b"must-rollback".to_vec())
        .unwrap();
    admitted
        .prepare_with_cache_effects(
            state,
            &[unica_source_contract::events::DomainEvent::new(
                unica_source_contract::events::DomainEventKind::MetadataChanged,
                "Catalog.Products",
            )],
        )
        .unwrap()
}

pub fn finish_saved_apply_test(
    mut result: unica_execution_contract::invocation::DomainResult,
    publication: Result<super::ApplyPublicationResult, super::ApplyPublicationError>,
) -> unica_execution_contract::invocation::DomainResult {
    match publication {
        Ok(publication) => {
            result.rev = Some(publication.rev().to_owned());
            result.data = Some(
                serde_json::json!({"commits": publication.commit_count_for_test(), "mode": "published"}),
            );
        }
        Err(error) => {
            result.ok = false;
            result.summary = error.to_string();
        }
    }
    result
}

pub fn assert_prepared_form_module_effect_subject(prepared: &PreparedApplyBatch) {
    assert_eq!(
        prepared.effects.events,
        [
            unica_source_contract::events::DomainEvent::new(
                DomainEventKind::FormChanged,
                "main:Catalog.Products.Form.Main",
            ),
            unica_source_contract::events::DomainEvent::new(
                DomainEventKind::ModuleChanged,
                "main:Catalog.Products.Form.Main.Module.Form",
            ),
        ]
    );
    assert_eq!(prepared.effects.cache.mode, "applied");
    assert_eq!(
        prepared.effects.cache.events,
        ["FormChanged", "ModuleChanged"]
    );
}

pub fn assert_form_module_effect_subject(
    receipt: &ApplyEffectReceipt,
    disposition: ApplyEffectDisposition,
) {
    assert_eq!(receipt.disposition(), disposition);
    assert_eq!(
        receipt.events(),
        &[
            unica_source_contract::events::DomainEvent::new(
                DomainEventKind::FormChanged,
                "main:Catalog.Products.Form.Main",
            ),
            unica_source_contract::events::DomainEvent::new(
                DomainEventKind::ModuleChanged,
                "main:Catalog.Products.Form.Main.Module.Form",
            ),
        ]
    );
    let cache = receipt.cache();
    assert_eq!(cache.mode, "applied");
    assert_eq!(cache.events, ["FormChanged", "ModuleChanged"]);
    assert_eq!(
        cache.invalidated,
        [
            "bsl_diagnostics",
            "bsl_index",
            "form_graph",
            "metadata_graph"
        ]
    );
    assert_eq!(cache.refreshed, ["metadata_graph"]);
}

pub fn write_actor_event_fixture(root: &Path) {
    const MD: &str = "http://v8.1c.ru/8.3/MDClasses";
    std::fs::create_dir_all(root.join("Catalogs/Products/Forms/Main/Ext")).unwrap();
    std::fs::write(
            root.join("Configuration.xml"),
            format!(
                "<MetaDataObject xmlns=\"{MD}\" version=\"2.20\"><Configuration><Properties><Name>Fixture</Name></Properties><ChildObjects><Catalog>Products</Catalog></ChildObjects></Configuration></MetaDataObject>"
            ),
        )
        .unwrap();
    std::fs::write(
            root.join("Catalogs/Products.xml"),
            format!(
                "<MetaDataObject xmlns=\"{MD}\" version=\"2.20\"><Catalog><Properties><Name>Products</Name></Properties><ChildObjects><Form>Main</Form></ChildObjects></Catalog></MetaDataObject>"
            ),
        )
        .unwrap();
    std::fs::write(
            root.join("Catalogs/Products/Forms/Main.xml"),
            format!(
                "<MetaDataObject xmlns=\"{MD}\" version=\"2.20\"><Form><Properties><Name>Main</Name></Properties></Form></MetaDataObject>"
            ),
        )
        .unwrap();
    std::fs::write(
        root.join("Catalogs/Products/Forms/Main/Ext/Form.xml"),
        concat!(
            "\u{feff}<?xml version=\"1.0\" encoding=\"UTF-8\"?>\r\n",
            "<Form xmlns=\"http://v8.1c.ru/8.3/xcf/logform\" version=\"2.20\">\r\n",
            "\t<AutoCommandBar name=\"Bar\" id=\"-1\"/>\r\n",
            "\t<ChildItems/>\r\n",
            "\t<Commands/>\r\n",
            "</Form>\r\n"
        ),
    )
    .unwrap();
}

pub fn write_missing_on_open_binding(root: &Path) {
    std::fs::write(
        root.join("Catalogs/Products/Forms/Main/Ext/Form.xml"),
        concat!(
            "\u{feff}<?xml version=\"1.0\" encoding=\"UTF-8\"?>\r\n",
            "<Form xmlns=\"http://v8.1c.ru/8.3/xcf/logform\" version=\"2.20\">\r\n",
            "\t<AutoCommandBar name=\"Bar\" id=\"-1\"/>\r\n",
            "\t<Events><Event name=\"OnOpen\">ПриОткрытии</Event></Events>\r\n",
            "\t<ChildItems/>\r\n",
            "\t<Commands/>\r\n",
            "</Form>\r\n"
        ),
    )
    .unwrap();
}

pub fn actor_fixture(name: &str, relative_roots: &[&str]) -> ActorFixture {
    let root = temp_root(name);
    let roots = relative_roots
        .iter()
        .map(|relative| root.join(relative))
        .collect::<Vec<_>>();
    for source_root in &roots {
        std::fs::create_dir_all(source_root).unwrap();
    }
    write_actor_source_map(&root, relative_roots);
    let context = context(&root);
    let source_sets = relative_roots
        .iter()
        .zip(roots.iter())
        .map(|(name, root)| source_input(name, root));
    let identity = WorkspaceIdentity::new(&context, source_sets, "test-provider").unwrap();
    let actor = Arc::new(
        super::WorkspaceActor::<(), ()>::new(identity, context)
            .map(bind_actor)
            .unwrap(),
    );
    ActorFixture { root, roots, actor }
}

pub fn actor_fixture_without_source_map(name: &str, relative_root: &str) -> ActorFixture {
    let root = temp_root(name);
    let source_root = root.join(relative_root);
    std::fs::create_dir_all(&source_root).unwrap();
    let context = context(&root);
    let identity = WorkspaceIdentity::new(
        &context,
        [source_input("main", &source_root)],
        "test-provider",
    )
    .unwrap();
    let actor = Arc::new(
        super::WorkspaceActor::<(), ()>::new(identity, context)
            .map(bind_actor)
            .unwrap(),
    );
    ActorFixture {
        root,
        roots: vec![source_root],
        actor,
    }
}

pub fn write_actor_source_map(root: &Path, relative_roots: &[&str]) {
    let mut yaml = String::from("format: DESIGNER\nsource-set:\n");
    for relative in relative_roots {
        yaml.push_str("  - name: ");
        yaml.push_str(&serde_json::to_string(relative).unwrap());
        yaml.push_str("\n    type: CONFIGURATION\n    path: ");
        yaml.push_str(&serde_json::to_string(relative).unwrap());
        yaml.push('\n');
    }
    std::fs::write(root.join("v8project.yaml"), yaml).unwrap();
}

pub fn context(root: &Path) -> WorkspaceContext {
    WorkspaceContext {
        cwd: root.to_path_buf(),
        workspace_root: root.to_path_buf(),
        cache_root: root.join(".build/unica"),
        workspace_epoch: 1,
    }
}

pub fn snapshot_tree(root: &Path) -> Vec<(PathBuf, Option<Vec<u8>>)> {
    if !root.exists() {
        return Vec::new();
    }
    let mut pending = vec![root.to_path_buf()];
    let mut observed = Vec::new();
    while let Some(path) = pending.pop() {
        let relative = path.strip_prefix(root).unwrap().to_path_buf();
        let metadata = std::fs::symlink_metadata(&path).unwrap();
        if metadata.is_dir() {
            observed.push((relative, None));
            let mut children = std::fs::read_dir(&path)
                .unwrap()
                .map(|entry| entry.unwrap().path())
                .collect::<Vec<_>>();
            children.sort();
            pending.extend(children.into_iter().rev());
        } else {
            let bytes = std::fs::read(&path).unwrap_or_else(|error| {
                panic!("snapshot_tree could not read {}: {error}", path.display())
            });
            observed.push((relative, Some(bytes)));
        }
    }
    observed
}

pub fn temp_root(name: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!(
        "unica-workspace-actor-{name}-{}-{nonce}",
        std::process::id()
    ))
}

thread_local! {
    pub(super) static SUPPORT_POLICY_ACTOR_TEST_NOW: Cell<Instant> = Cell::new(Instant::now());
    pub(super) static RETAINED_APPLY_DEADLINE_TEST_NOW: Cell<Instant> = Cell::new(Instant::now());
}
pub struct ActorFixture {
    pub root: PathBuf,
    pub roots: Vec<PathBuf>,
    pub actor: Arc<super::WorkspaceActor>,
}
impl ActorFixture {
    pub fn cleanup(self) {
        let _ = std::fs::remove_dir_all(self.root);
    }
}

thread_local! {
    static POLICIES: std::cell::RefCell<Option<crate::policies::WorkspacePolicies>> = const { std::cell::RefCell::new(None) };
}
pub(crate) struct PoliciesScope(Option<crate::policies::WorkspacePolicies>);
impl Drop for PoliciesScope {
    fn drop(&mut self) {
        POLICIES.with(|slot| *slot.borrow_mut() = self.0.take());
    }
}
pub(crate) fn policies_scope(policies: crate::policies::WorkspacePolicies) -> PoliciesScope {
    PoliciesScope(POLICIES.with(|slot| slot.borrow_mut().replace(policies)))
}
pub(crate) fn bind_actor(actor: super::WorkspaceActor<(), ()>) -> super::WorkspaceActor<(), ()> {
    match POLICIES.with(|slot| slot.borrow().clone()) {
        Some(policies) => actor.with_policies(policies),
        None => actor,
    }
}
pub(crate) fn registry_for_test() -> super::WorkspaceActorRegistry<()> {
    match POLICIES.with(|slot| slot.borrow().clone()) {
        Some(policies) => super::WorkspaceActorRegistry::with_policies(policies),
        None => super::WorkspaceActorRegistry::default(),
    }
}
pub(crate) fn idle_registry_for_test(ttl: Duration) -> super::WorkspaceActorRegistry<()> {
    let mut registry = super::WorkspaceActorRegistry::<()>::with_idle_policy_for_test(ttl);
    registry.policies = POLICIES.with(|slot| slot.borrow().clone());
    registry
}
pub fn actor_fixture_with_policies(
    name: &str,
    roots: &[&str],
    policies: crate::policies::WorkspacePolicies,
) -> ActorFixture {
    let _scope = policies_scope(policies);
    actor_fixture(name, roots)
}
