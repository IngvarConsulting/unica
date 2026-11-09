//! Shared by the tests of `metadata_can_tests`. A helper used by one test file
//! belongs in that file.
use super::*;

pub(super) type SourceImage = BTreeMap<PathBuf, Vec<u8>>;

pub(super) struct Fixture {
    pub(super) _root: tempfile::TempDir,
    pub(super) workspace: PathBuf,
    pub(super) source: PathBuf,
}

pub(super) fn acceptance_fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/acceptance")
}

pub(super) fn copy_image(from: &Path, to: &Path) {
    for (relative, bytes) in unica_workspace::test_support::tree_snapshot(from) {
        let path = to.join(relative);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, bytes).unwrap();
    }
}

impl Fixture {
    pub(super) fn new(name: &str) -> Self {
        let root = tempfile::tempdir().unwrap();
        let workspace =
            unica_workspace::test_support::canonical_path(root.path()).join("workspace");
        copy_image(&acceptance_fixtures().join(name), &workspace);
        Self {
            _root: root,
            source: workspace.join("src"),
            workspace,
        }
    }

    /// Минимальная конфигурация с одним языком: объекты создаёт сам тест
    /// каноническим `apply`, без прежнего `cf.init`/`meta.add`.
    pub(super) fn blank() -> Self {
        let root = tempfile::tempdir().unwrap();
        let workspace =
            unica_workspace::test_support::canonical_path(root.path()).join("workspace");
        let source = workspace.join("src");
        std::fs::create_dir_all(source.join("Languages")).unwrap();
        std::fs::write(
            workspace.join("v8project.yaml"),
            "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
        )
        .unwrap();
        std::fs::write(
            source.join("Configuration.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Configuration uuid="f3317000-0000-4000-8000-0000000000c1"><Properties><Name>Store</Name><DefaultLanguage>Language.Русский</DefaultLanguage></Properties><ChildObjects><Language>Русский</Language></ChildObjects></Configuration></MetaDataObject>"#,
        )
        .unwrap();
        std::fs::write(
            source.join("Languages/Русский.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Language uuid="33333333-3333-4333-8333-333333333333"><Properties><Name>Русский</Name><Comment/><LanguageCode>ru</LanguageCode></Properties></Language></MetaDataObject>"#,
        )
        .unwrap();
        Self {
            _root: root,
            source,
            workspace,
        }
    }

    pub(super) fn with_extension() -> Self {
        let fixture = Self::new("workspace");
        copy_image(
            &acceptance_fixtures().join("workspace-format/src-ext"),
            &fixture.workspace.join("src-ext"),
        );
        std::fs::write(
            fixture.workspace.join("v8project.yaml"),
            "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n  - name: ext\n    type: EXTENSION\n    path: src-ext\n",
        )
        .unwrap();
        fixture
    }

    pub(super) fn binding(&self, extension: bool) -> ProviderRootBinding {
        let (name, kind, source) = if extension {
            (
                "ext",
                SourceSetKind::Extension,
                self.workspace.join("src-ext"),
            )
        } else {
            ("main", SourceSetKind::Configuration, self.source.clone())
        };
        let context = WorkspaceContext {
            cwd: self.workspace.clone(),
            workspace_root: self.workspace.clone(),
            cache_root: self.workspace.join(".build/parser-test"),
            workspace_epoch: 1,
        };
        let identity = WorkspaceIdentity::new(
            &context,
            [WorkspaceSourceSetInput::new(
                name,
                &source,
                kind,
                SourceFormat::PlatformXml,
                SourceProfile::platform_xml_8_3_27_format_2_20(),
            )],
            "metadata-can-parser-test",
        )
        .unwrap();
        crate::infrastructure::source_workspace::new_actor(identity, context)
            .unwrap()
            .bind_provider_root(name, &source)
            .unwrap()
    }
}

pub(super) fn runtime() -> V5CanonicalInvocationRuntime {
    V5CanonicalInvocationRuntime::new(
        Arc::new(crate::infrastructure::daemon::v13_service::CanonicalV13ReadService::default()),
        Arc::new(TokioClock),
    )
}

pub(super) fn call(
    runtime: &V5CanonicalInvocationRuntime,
    fixture: &Fixture,
    tool: ToolIdentity,
    args: Value,
) -> DomainResult {
    direct_v5(
        runtime,
        InvocationRequest::new(tool, args, fixture.workspace.to_string_lossy(), 7_000).unwrap(),
    )
    .unwrap()
}

/// Канонический цикл `apply`: план без записи, затем исполнение по токену
/// с тем же хешем плана.
pub(super) fn publish(
    runtime: &V5CanonicalInvocationRuntime,
    fixture: &Fixture,
    args: Value,
) -> DomainResult {
    let before = unica_workspace::test_support::tree_snapshot(&fixture.workspace);
    let planned = call(runtime, fixture, ToolIdentity::Apply, args);
    assert!(planned.ok, "{planned:?}");
    assert_eq!(
        unica_workspace::test_support::tree_snapshot(&fixture.workspace),
        before,
        "planning writes nothing"
    );
    let planned = planned.data.unwrap();
    assert_eq!(planned["mode"], "preview");
    let published = call(
        runtime,
        fixture,
        ToolIdentity::Apply,
        json!({"executionToken":planned["executionToken"]}),
    );
    assert!(published.ok, "{published:?}");
    assert_eq!(published.data.as_ref().unwrap()["mode"], "published");
    assert_eq!(
        published.data.as_ref().unwrap()["planHash"],
        planned["planHash"]
    );
    published
}

pub(super) fn contract(
    runtime: &V5CanonicalInvocationRuntime,
    fixture: &Fixture,
    at: &str,
    op: &str,
) -> Value {
    let detailed = call(
        runtime,
        fixture,
        ToolIdentity::View,
        json!({"at":format!("{at}.Can.{op}")}),
    );
    assert!(detailed.ok, "{at}.Can.{op}: {detailed:?}");
    let data = detailed.data.unwrap();
    assert_eq!(data["kind"], "Can");
    assert_eq!(data["props"]["op"], op);
    assert!(data["props"].get("implemented").is_none());
    let contract = data["props"]["contract"].clone();
    assert_eq!(contract["argsSchema"]["additionalProperties"], false);
    assert!(contract["exampleArgs"].get("at").is_none());
    contract
}

pub(super) fn parse(
    binding: &ProviderRootBinding,
    at: &str,
    op: &str,
    args: &Value,
) -> Result<(), ApplyPlanError> {
    let input = json!({"at":at,"ops":[{"op":op,"args":args}],"dryRun":true});
    let request = ApplyRequest::parse(input.as_object().unwrap(), &["main", "ext"]).unwrap();
    parse_family_operation_for_test(3, &request.ops()[0], binding)
}

pub(super) fn assert_parity(
    schema: &Value,
    binding: &ProviderRootBinding,
    at: &str,
    op: &str,
    samples: &[(Value, bool)],
) {
    let oracle =
        jsonschema::validator_for(schema).expect("published Can schema is valid JSON Schema");
    for (args, expected) in samples {
        assert_eq!(
            oracle.is_valid(args),
            *expected,
            "schema {op} at {at}: {args}"
        );
        let parsed = parse(binding, at, op, args);
        assert_eq!(
            parsed.is_ok(),
            *expected,
            "productive parser {op} at {at}: {args}: {parsed:?}"
        );
    }
}

pub(super) fn assert_props_can_and_preview(
    at: &str,
    preview_args: Value,
    compatibility_preview: Option<Value>,
    samples: &[(Value, bool, bool)],
) {
    let fixture = Fixture::new("workspace");
    let runtime = runtime();
    let before = unica_workspace::test_support::tree_snapshot(&fixture.source);
    let project_before = std::fs::read(fixture.workspace.join("v8project.yaml")).unwrap();
    let operation_at = format!("{at}.Can.props.set");
    let detailed = call(
        &runtime,
        &fixture,
        ToolIdentity::View,
        json!({"at":operation_at}),
    );
    assert!(detailed.ok, "{operation_at}: {detailed:?}");
    assert_eq!(detailed.at.as_deref(), Some(operation_at.as_str()));
    let data = detailed.data.unwrap();
    assert_eq!(data["at"], operation_at);
    assert_eq!(data["kind"], "Can");
    assert_eq!(data["props"]["op"], "props.set");
    assert!(data["props"].get("implemented").is_none());
    let contract = &data["props"]["contract"];
    let schema = &contract["argsSchema"];
    assert_eq!(schema["additionalProperties"], false);
    assert_eq!(
        schema["properties"]["values"]["additionalProperties"],
        false
    );
    assert!(contract["exampleArgs"].get("at").is_none());
    let oracle = jsonschema::validator_for(schema).expect("Props Can publishes valid JSON Schema");
    assert!(oracle.is_valid(&preview_args), "{at}: {preview_args}");
    let binding = fixture.binding(false);
    for (args, published, accepted) in samples {
        assert_eq!(oracle.is_valid(args), *published, "published {at}: {args}");
        let parsed = parse(&binding, at, "props.set", args);
        assert_eq!(parsed.is_ok(), *accepted, "parser {at}: {args}: {parsed:?}");
    }
    if let Some(alias_args) = &compatibility_preview {
        assert!(
            !oracle.is_valid(alias_args),
            "compatibility spellings stay unpublished: {at}: {alias_args}"
        );
    }
    for args in [
        json!({}),
        json!({"values":null}),
        json!({"values":[]}),
        json!({"values":{}}),
        json!({"values":{"Comment":null}}),
        json!({"values":{"Comment":false}}),
        json!({"values":{"UnknownProperty":"x"}}),
        json!({"values":{"Comment":"x"},"unknown":true}),
        json!({"values":{"Comment":"x"},"at":at}),
    ] {
        assert!(!oracle.is_valid(&args), "{at}: {args}");
    }
    assert_eq!(
        unica_workspace::test_support::tree_snapshot(&fixture.source),
        before
    );
    assert_eq!(
        std::fs::read(fixture.workspace.join("v8project.yaml")).unwrap(),
        project_before
    );

    for preview_args in std::iter::once(preview_args).chain(compatibility_preview) {
        let preview = call(
            &runtime,
            &fixture,
            ToolIdentity::Apply,
            json!({"at":at,"ops":[{"op":"props.set","args":preview_args}]}),
        );
        assert!(preview.ok, "real props.set preview at {at}: {preview:?}");
        let planned = preview.data.unwrap();
        assert_eq!(planned["mode"], "preview");
        assert!(planned["executionToken"]
            .as_str()
            .is_some_and(|token| !token.is_empty()));
        assert!(planned["effects"]
            .as_u64()
            .is_some_and(|effects| effects > 0));
        assert_eq!(
            unica_workspace::test_support::tree_snapshot(&fixture.source),
            before
        );
        assert_eq!(
            std::fs::read(fixture.workspace.join("v8project.yaml")).unwrap(),
            project_before
        );
    }
}

pub(super) fn changed_paths(
    before: &SourceImage,
    after: &SourceImage,
) -> (BTreeSet<PathBuf>, BTreeSet<PathBuf>, BTreeSet<PathBuf>) {
    (
        after
            .keys()
            .filter(|path| !before.contains_key(*path))
            .cloned()
            .collect(),
        after
            .iter()
            .filter(|(path, bytes)| before.get(*path).is_some_and(|old| old != *bytes))
            .map(|(path, _)| path.clone())
            .collect(),
        before
            .keys()
            .filter(|path| !after.contains_key(*path))
            .cloned()
            .collect(),
    )
}

/// Задаёт состав определяемого типа: член с точкой и без префикса
/// `DefinedType.` пишется как `Type`, семейство и определяемый тип — как `TypeSet`.
pub(super) fn set_defined_type_members(source: &Path, name: &str, members: &[&str]) {
    let path = source.join(format!("DefinedTypes/{name}.xml"));
    let descriptor = std::fs::read_to_string(&path).unwrap();
    let body = members
        .iter()
        .map(|member| {
            let tag = if member.starts_with("DefinedType.") || !member.contains('.') {
                "TypeSet"
            } else {
                "Type"
            };
            format!("<v8:{tag}>cfg:{member}</v8:{tag}>")
        })
        .collect::<String>();
    let (start, end) = match descriptor.find("<Type/>") {
        Some(start) => (start, start + "<Type/>".len()),
        None => {
            let start = descriptor.find("<Type>").expect("DefinedType Type");
            let close = start + descriptor[start..].find("</Type>").unwrap();
            (start, close + "</Type>".len())
        }
    };
    let mut changed = descriptor.clone();
    changed.replace_range(start..end, &format!("<Type>{body}</Type>"));
    assert_ne!(changed, descriptor, "{name}: members must change");
    std::fs::write(path, changed).unwrap();
}

/// Заменяет единственное вхождение фрагмента в файле исходников.
pub(super) fn rewrite(path: &Path, from: &str, to: &str) {
    let text = std::fs::read_to_string(path).unwrap();
    assert_eq!(text.matches(from).count(), 1, "{}: {from}", path.display());
    std::fs::write(path, text.replacen(from, to, 1)).unwrap();
}

pub(super) fn diagnostics(result: &DomainResult) -> Vec<Value> {
    assert!(result.ok, "{result:?}");
    result.data.as_ref().unwrap()["diagnostics"]
        .as_array()
        .expect("check diagnostics")
        .clone()
}

pub(super) fn check_status(result: &DomainResult) -> &str {
    result.data.as_ref().unwrap()["status"].as_str().unwrap()
}

/// Подписка из корпуса без единого справочника: источник — семейство
/// документов, конфигурация не содержит `Catalog`.
pub(super) fn event_source_without_catalogs() -> Fixture {
    let fixture = Fixture::new("workspace-event-source-order");
    std::fs::remove_dir_all(fixture.source.join("Catalogs")).unwrap();
    let configuration = fixture.source.join("Configuration.xml");
    for name in ["ПримерВ", "ПримерА", "ПримерБ"] {
        rewrite(
            &configuration,
            &format!("\t\t\t<Catalog>{name}</Catalog>\n"),
            "",
        );
    }
    rewrite(
        &fixture.source.join("EventSubscriptions/CorpusEventSubscription.xml"),
        "\t\t\t\t<v8:Type>cfg:CatalogObject.ПримерВ</v8:Type>\n\t\t\t\t<v8:Type>cfg:CatalogObject.ПримерА</v8:Type>\n",
        "\t\t\t\t<v8:TypeSet>cfg:DocumentObject</v8:TypeSet>\n",
    );
    fixture
}

pub(super) fn subscription_source(fixture: &Fixture) -> Vec<(String, String)> {
    let xml = std::fs::read_to_string(
        fixture
            .source
            .join("EventSubscriptions/CorpusEventSubscription.xml"),
    )
    .unwrap();
    let document = roxmltree::Document::parse(xml.trim_start_matches('\u{feff}')).unwrap();
    let source = document
        .descendants()
        .find(|node| node.has_tag_name("Source"))
        .expect("Source");
    source
        .children()
        .filter(|node| node.is_element())
        .map(|node| {
            let (prefix, local) = node.text().unwrap().split_once(':').unwrap();
            assert_eq!(
                node.lookup_namespace_uri(Some(prefix)),
                Some("http://v8.1c.ru/8.1/data/enterprise/current-config")
            );
            (node.tag_name().name().to_owned(), local.to_owned())
        })
        .collect()
}

/// Справочник `Products` с реквизитом `Article` и константа `Rate`, оба
/// строкового типа длиной 10.
pub(super) fn catalog_and_constant(runtime: &V5CanonicalInvocationRuntime) -> Fixture {
    let fixture = Fixture::blank();
    for kind in ["Catalog", "Constant"] {
        let name = if kind == "Catalog" {
            "Products"
        } else {
            "Rate"
        };
        publish(
            runtime,
            &fixture,
            json!({"at":"main:Configuration","ops":[{"op":"object.create","args":{"values":{"kind":kind,"name":name}}}]}),
        );
    }
    publish(
        runtime,
        &fixture,
        json!({"at":"main:Catalog.Products","ops":[{"op":"attribute.add","args":{"items":[
            {"name":"Article","type":{"variants":[{"kind":"string","length":10,"allowedLength":"variable"}]}}
        ]}}]}),
    );
    publish(
        runtime,
        &fixture,
        json!({"at":"main:Constant.Rate","ops":[{"op":"props.set","args":{"values":{
            "Type":{"variants":[{"kind":"string","length":10,"allowedLength":"variable"}]}
        }}}]}),
    );
    fixture
}

/// Делает строковый тип немоделируемым, но синтаксически допустимым: имя
/// платформенного типа без квалификаторов строки.
pub(super) fn make_type_unmodelled(path: &Path) {
    rewrite(
        path,
        "<v8:Type>xs:string</v8:Type>",
        "<v8:Type>v8:FutureOpaque</v8:Type>",
    );
    let text = std::fs::read_to_string(path).unwrap();
    let start = text
        .find("<v8:StringQualifiers>")
        .expect("string qualifiers");
    let end = text[start..].find("</v8:StringQualifiers>").unwrap()
        + start
        + "</v8:StringQualifiers>".len();
    let mut text = text;
    text.replace_range(start..end, "");
    std::fs::write(path, text).unwrap();
}

pub(super) fn view(runtime: &V5CanonicalInvocationRuntime, fixture: &Fixture, at: &str) -> Value {
    let viewed = call(runtime, fixture, ToolIdentity::View, json!({"at": at}));
    assert!(viewed.ok, "{at}: {viewed:?}");
    viewed.data.unwrap()
}

pub(super) fn check(
    runtime: &V5CanonicalInvocationRuntime,
    fixture: &Fixture,
    at: &str,
) -> (String, Vec<Value>) {
    let checked = call(runtime, fixture, ToolIdentity::Check, json!({"at": at}));
    assert!(checked.ok, "{at}: {checked:?}");
    let data = checked.data.unwrap();
    (
        data["status"].as_str().unwrap().to_owned(),
        data["diagnostics"].as_array().unwrap().clone(),
    )
}

pub(super) fn limits(node: &Value) -> Vec<String> {
    node["limits"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::to_owned)
        .collect()
}

/// Единственная находка с кодом у поля: читатель и валидатор не называют
/// одну поломку дважды.
pub(super) fn finding<'a>(diagnostics: &'a [Value], code: &str, field: &str) -> &'a Value {
    let found = diagnostics
        .iter()
        .filter(|diagnostic| diagnostic["field"] == field)
        .collect::<Vec<_>>();
    assert_eq!(found.len(), 1, "{field}: {diagnostics:?}");
    assert_eq!(found[0]["code"], code, "{field}: {diagnostics:?}");
    found[0]
}
