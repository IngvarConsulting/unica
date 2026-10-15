//! Честный `can`: двусторонняя сверка словаря узла с `apply` (#1351).
//!
//! Тест обходит реальные проекции чтения фикстур приёмочного корпуса и для
//! представительного узла каждой пары «маршрут, вид» сверяет:
//!
//! - словарь `can`, который отдаёт `view`;
//! - предварительную проверку `apply` (применимость к месту узла);
//! - разбор семейства в обход реестра: адрес, который семейство приняло бы,
//!   а реестр — нет, есть либо намеренное сужение из списка ниже, либо
//!   расхождение;
//! - фильтр формата макета, каким его применяет планировщик к настоящему
//!   макету;
//! - файлы, которые меняет план принятой операции: они принадлежат узлу
//!   `at` и его ресурсам, а не другому узлу.

use super::server::V5CanonicalInvocationRuntime;
use super::v13_service::{preflight_apply_operations, CanonicalV13ReadService};
use crate::application::invocation_store::ToolIdentity;
use crate::application::ports::TokioClock;
use crate::domain::address::{NodeKind, QualifiedAddress};
use crate::domain::apply::{
    ApplyRequest, FormItemRole, OperationDescriptor, OperationFamily, OperationRegistry,
    SourceSetTraits,
};
use crate::domain::cancellation::CancellationToken;
use crate::domain::code_intelligence::ProviderDeadline;
use crate::domain::invocation::DomainResult;
use crate::domain::logical_route::{classify_route, RouteClass};
use crate::domain::platform_profile::PlatformProfile;
use crate::domain::project_sources::{SourceFormat, SourceProfile, SourceSetKind};
use crate::domain::workspace::WorkspaceContext;
use crate::infrastructure::daemon::protocol::InvocationRequest;
use crate::infrastructure::native_operations::apply_families::{
    parse_family_operation_for_test, plan_hidden_v13_apply,
};
use crate::infrastructure::workspace_actor::{
    ProviderRootBinding, WorkspaceActor, WorkspaceIdentity, WorkspaceSourceSetInput,
};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

/// Узлов на набор не больше этого, и не больше двух на одну форму адреса:
/// обход — выборка разнообразных узлов, а не перепись.
const NODE_BUDGET: usize = 600;
const PER_SHAPE: usize = 2;

/// Намеренное сужение: разбор семейства в обход реестра принял бы операцию
/// на этом месте, а реестр её здесь не объявляет, и в рабочем пути
/// `refuse_misplaced_operation` отказывает раньше разбора.
struct Narrowing {
    /// Операция или `family:<семейство>`.
    op: &'static str,
    /// Маршрут узла; `None` — любой.
    route: Option<RouteClass>,
    /// Вид узла; `*` — любой.
    kind: &'static str,
    /// Фикстура, где сужение обязано встретиться: устаревшее сужение тест
    /// не прячет.
    witnessed_in: &'static str,
    reason: &'static str,
}

const INTENTIONAL_NARROWINGS: &[Narrowing] = &[
    Narrowing {
        op: "family:Dcs",
        route: Some(RouteClass::Dcs),
        kind: "*",
        witnessed_in: "workspace",
        reason: "разбор СКД не сверяет спуск адреса с операцией: схемные операции на \
                 наборе писали бы в корень схемы, операции настроек без `Setting` — в \
                 вариант по умолчанию, операции набора на поле — в набор",
    },
    Narrowing {
        op: "family:Dcs",
        route: Some(RouteClass::Metadata),
        kind: "Template",
        witnessed_in: "workspace",
        reason: "на самом макете разбор СКД не требует набора или варианта: операции \
                 набора и настроек писали бы в набор из аргументов или в вариант по \
                 умолчанию",
    },
    Narrowing {
        op: "code.insert",
        route: None,
        kind: "*",
        witnessed_in: "workspace",
        reason: "разбор вставки не читает адрес: модуль разрешает план, и на узле без \
                 модуля он отказал бы только при стадировании",
    },
    Narrowing {
        op: "event.implement",
        route: None,
        kind: "*",
        witnessed_in: "workspace",
        reason: "разбор события проверяет только форму адреса; событие и его модуль \
                 разрешает план",
    },
    Narrowing {
        op: "code.replace",
        route: Some(RouteClass::Metadata),
        kind: "Method",
        witnessed_in: "workspace",
        reason: "метод HTTP-сервиса — член метаданных, модуль по нему не разрешается",
    },
    Narrowing {
        op: "code.replace",
        route: Some(RouteClass::Mxl),
        kind: "Body",
        witnessed_in: "workspace-mxl",
        reason: "разбор кода принимает `Body` любого узла за тело модуля; модуль по \
                 адресу области макета не разрешается",
    },
    Narrowing {
        op: "family:Metadata",
        route: None,
        kind: "*",
        witnessed_in: "workspace",
        reason: "наличие коллекции (макетов, команд) у вида владельца проверяет движок \
                 правки при плане; реестр отсекает вид без коллекции раньше",
    },
    Narrowing {
        op: "family:Form",
        route: None,
        kind: "*",
        witnessed_in: "workspace",
        reason: "наличие коллекции форм у вида владельца проверяет движок правки при \
                 плане; реестр отсекает вид без коллекции раньше",
    },
];

/// Операции, чей разбор на этих узлах упирается в аргументы раньше вердикта
/// о цели даже с правдоподобными аргументами. Цель им проверяет реестр до
/// разбора (`refuse_misplaced_operation`), а здесь вердикт не окончательный.
const ARGUMENT_DEPENDENT: &[(&str, &str)] = &[
    (
        "relation.add",
        "имя связи зависит от вида владельца (Owners, BasedOn, …)",
    ),
    ("relation.remove", "имя связи зависит от вида владельца"),
    ("relation.replace", "имя связи зависит от вида владельца"),
    (
        "code.replace",
        "на адресе модуля замена требует `selector` с существующим методом",
    ),
    (
        "code.insert",
        "на адресе метода вставка требует `position` и существующий метод",
    ),
];

/// Операции, чей план законно меняет файлы вне узла `at`.
const WRITES_BEYOND_NODE: &[(&str, &str)] = &[
    (
        "object.remove",
        "удаление объекта снимает его регистрацию в корне и ссылки на него",
    ),
    (
        "supportRule.set",
        "правило поддержки хранится в файле поддержки конфигурации",
    ),
];

/// Операции, чей план на фикстуре проходит с правдоподобными аргументами и
/// чьи файлы сверяются с узлом `at`. Остальные операции словаря доходят до
/// плана только с именами, которых в фикстуре нет, и проверку записи здесь
/// не получают: прежде всего операции СКД, табличного документа и членов
/// метаданных.
const WRITES_CHECKED_AT_LEAST: &[(&str, &[&str])] = &[
    (
        "workspace",
        &[
            "attribute.add",
            "childSubsystem.add",
            "childSubsystem.remove",
            "code.insert",
            "column.remove",
            "column.set",
            "dimension.remove",
            "dimension.set",
            "enumValue.remove",
            "enumValue.set",
            "resource.remove",
            "resource.set",
            "tabularSection.remove",
            "tabularSection.set",
            "template.remove",
            "template.set",
            "code.replace",
            "commandPlacement.set",
            "commandVisibility.set",
            "content.add",
            "content.remove",
            "element.add",
            "event.bind",
            "form.remove",
            "formAttribute.add",
            "formCommand.add",
            "help.create",
            "objectType.add",
            "props.set",
            "right.set",
            "supportRule.set",
            "valueType.add",
        ],
    ),
    (
        "workspace-code",
        &["code.insert", "code.replace", "help.create"],
    ),
    (
        "workspace-resolve",
        &["code.insert", "code.replace", "form.remove"],
    ),
    (
        "workspace-predefined",
        &[
            "predefinedItem.add",
            "predefinedItem.remove",
            "predefinedItem.set",
        ],
    ),
];

/// Маршруты, до которых обход фикстур корпуса не доходит.
const COVERAGE_GAPS: &[(RouteClass, &str)] = &[];

struct Fixture {
    _root: tempfile::TempDir,
    workspace: PathBuf,
    actor: Arc<WorkspaceActor>,
    sets: Vec<(String, SourceSetKind, ProviderRootBinding)>,
}

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("repository root")
        .to_path_buf()
}

fn copy_tree(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let target = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_tree(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), target).unwrap();
        }
    }
}

/// Наборы исходников фикстуры из её `v8project.yaml`: имя, вид и путь.
fn source_sets(workspace: &Path) -> Vec<(String, SourceSetKind, PathBuf)> {
    let text = std::fs::read_to_string(workspace.join("v8project.yaml")).unwrap();
    let mut sets = Vec::new();
    let mut name = None;
    let mut kind = None;
    for line in text.lines() {
        let line = line.trim().trim_start_matches("- ");
        if let Some(value) = line.strip_prefix("name:") {
            name = Some(value.trim().to_string());
        } else if let Some(value) = line.strip_prefix("type:") {
            kind = Some(match value.trim() {
                "CONFIGURATION" => SourceSetKind::Configuration,
                "EXTENSION" => SourceSetKind::Extension,
                "EXTERNAL_DATA_PROCESSORS" => SourceSetKind::ExternalProcessor,
                "EXTERNAL_REPORTS" => SourceSetKind::ExternalReport,
                other => panic!("unknown source-set type {other}"),
            });
        } else if let Some(value) = line.strip_prefix("path:") {
            sets.push((
                name.take().expect("name precedes path"),
                kind.take().expect("type precedes path"),
                workspace.join(value.trim()),
            ));
        }
    }
    sets
}

fn fixture(name: &str) -> Fixture {
    let root = tempfile::tempdir().unwrap();
    let workspace = std::fs::canonicalize(root.path()).unwrap().join("ws");
    copy_tree(
        &repo_root().join("tests/fixtures/acceptance").join(name),
        &workspace,
    );
    let context = WorkspaceContext {
        cwd: workspace.clone(),
        workspace_root: workspace.clone(),
        cache_root: workspace.join(".build/unica"),
        workspace_epoch: 1,
    };
    let declared = source_sets(&workspace);
    let inputs = declared
        .iter()
        .map(|(name, kind, path)| {
            WorkspaceSourceSetInput::new(
                name,
                path,
                *kind,
                SourceFormat::PlatformXml,
                SourceProfile::platform_xml_8_3_27_format_2_20(),
            )
        })
        .collect::<Vec<_>>();
    let identity = WorkspaceIdentity::new(&context, inputs, "can-honesty-test").unwrap();
    let actor = Arc::new(WorkspaceActor::new(identity, context).unwrap());
    let sets = declared
        .into_iter()
        .map(|(name, kind, path)| {
            let binding = actor.bind_provider_root(&name, &path).unwrap();
            (name, kind, binding)
        })
        .collect();
    Fixture {
        _root: root,
        workspace,
        actor,
        sets,
    }
}

fn invoke(workspace: &Path, tool: ToolIdentity, arguments: Value) -> DomainResult {
    invoke_on(
        &V5CanonicalInvocationRuntime::new(
            Arc::new(CanonicalV13ReadService::default()),
            Arc::new(TokioClock),
        ),
        workspace,
        tool,
        arguments,
    )
}

/// Вызов на одном живом сервисе: план и его `executionToken` переживают
/// вызов только внутри одного сервиса.
fn invoke_on(
    runtime: &V5CanonicalInvocationRuntime,
    workspace: &Path,
    tool: ToolIdentity,
    arguments: Value,
) -> DomainResult {
    runtime
        .bind(InvocationRequest::new(tool, arguments, workspace.to_string_lossy(), 7_000).unwrap())
        .unwrap()
        .prepare()
        .unwrap()
        .execute(CancellationToken::new())
        .unwrap()
}

/// Узлы набора исходников, которые показывает чтение: ветви и элементы от
/// корня, в ширину, не больше `PER_SHAPE` на одну форму адреса.
fn walk(workspace: &Path, root: &str) -> Vec<String> {
    let mut seen = BTreeSet::new();
    let mut shapes = BTreeMap::<String, usize>::new();
    let mut queue = VecDeque::from([root.to_string()]);
    let mut nodes = Vec::new();
    while let Some(at) = queue.pop_front() {
        if nodes.len() >= NODE_BUDGET || !seen.insert(at.clone()) {
            continue;
        }
        let Ok(address) = QualifiedAddress::parse(&at) else {
            continue;
        };
        // Ветви графа вызовов `Caller` и `Callee` — коллекция анализатора, а
        // не узлы набора исходников: словаря `can` и фильтра у них нет.
        if address
            .segments()
            .last()
            .is_some_and(|segment| matches!(segment.kind(), NodeKind::Caller | NodeKind::Callee))
        {
            continue;
        }
        let count = shapes.entry(shape_key(&address)).or_default();
        if *count >= PER_SHAPE {
            continue;
        }
        let result = invoke(workspace, ToolIdentity::View, json!({"at": at}));
        let Some(data) = result.data.filter(|_| result.ok) else {
            continue;
        };
        *count += 1;
        nodes.push(at);
        for slot in ["branches", "items"] {
            for child in data[slot].as_array().into_iter().flatten() {
                if let Some(child) = child.get("at").and_then(Value::as_str) {
                    queue.push_back(child.to_string());
                }
            }
        }
    }
    nodes
}

/// Форма адреса: маршрут и виды сегментов с признаком имени.
fn shape_key(address: &QualifiedAddress) -> String {
    let route = classify_route(address, PlatformProfile::v8_3_27());
    let segments = address
        .segments()
        .iter()
        .map(|segment| {
            format!(
                "{}{}",
                segment.kind().as_str(),
                if segment.name().is_some() { "*" } else { "" }
            )
        })
        .collect::<Vec<_>>()
        .join(".");
    format!("{route:?}:{segments}")
}

/// Правдоподобные аргументы операции: разбор семейства проходит целиком,
/// и отказ по цели не прячется за отказом по аргументам.
fn sample_args(descriptor: OperationDescriptor) -> Value {
    let op = descriptor.name();
    if let Some(contract) = descriptor.argument_contract() {
        return contract.example_args;
    }
    match op {
        "object.borrow" => json!({"from": "parent:Catalog.Parent"}),
        "content.remove" => json!({"items": [{"object": "Catalog.Sample"}]}),
        "code.insert" | "code.replace" => json!({"at": "", "text": "// sample\n"}),
        "supportRule.set" => json!({"values": {"rule": "editable"}}),
        "supportCapability.set" => json!({"values": {"enabled": true}}),
        "props.set" => json!({"values": {"Comment": "sample"}}),
        "help.create" => json!({"values": {"lang": "ru"}}),
        "role.create" | "subsystem.create" => json!({"values": {"name": "Sample"}}),
        "object.create" => json!({"values": {"kind": "Catalog", "name": "Sample"}}),
        "object.remove"
        | "form.remove"
        | "type.remove"
        | "property.remove"
        | "event.implement"
        | "childSubsystem.remove" => json!({"at": ""}),
        "form.add" | "formAttribute.add" | "formCommand.add" | "element.add"
        | "childSubsystem.add" => json!({"items": [{"name": "Sample"}]}),
        "form.set" => json!({"values": {"defaultObjectForm": "Sample"}}),
        "event.bind" => json!({"values": {"handler": "Sample"}}),
        "content.add" => json!({"items": [{"object": "Catalog.Sample"}]}),
        "right.set" => json!({"values": {"right": "Read", "value": true}}),
        "right.add" => {
            json!({"items": [{"object": "Catalog.Sample", "rights": {"Read": true}}]})
        }
        "commandVisibility.set" => {
            json!({"items": [{"command": "Catalog.Sample.StandardCommand.OpenList", "visible": true}]})
        }
        "commandPlacement.set" => json!({"items": [{
            "command": "Catalog.Sample.StandardCommand.OpenList",
            "group": "NavigationPanelImportant"
        }]}),
        "commandOrder.set" => json!({"values": {
            "commands": ["Catalog.Sample.StandardCommand.OpenList"]
        }}),
        "groupOrder.set" => json!({"values": {"groups": ["NavigationPanelImportant"]}}),
        "subsystemOrder.set" => json!({"values": {"subsystems": ["Subsystem.Sample"]}}),
        "valueType.add" => json!({"values": {"name": "Sample", "base": "xs:string"}}),
        "objectType.add" => json!({"values": {"name": "Sample"}}),
        "property.add" => json!({"values": {"property": {"name": "Sample", "type": "xs:string"}}}),
        _ => serde_json::to_value(descriptor.copyable_skeleton()).unwrap()["args"].clone(),
    }
}

/// Аргументы на конкретном узле: удаляемый элемент формы называет только
/// адрес (#1355).
fn node_args(descriptor: OperationDescriptor, at: &QualifiedAddress) -> Value {
    let method_address = at
        .segments()
        .last()
        .is_some_and(|segment| segment.kind() == NodeKind::Method);
    match descriptor.name() {
        "element.remove" => json!({"at": ""}),
        // Место вставки у метода — до или после него (#1354).
        "code.insert" if method_address => {
            json!({"at": "", "text": "// sample\n", "position": "after"})
        }
        _ => sample_args(descriptor),
    }
}

fn apply_request(at: &str, op: &str, mut args: Value) -> ApplyRequest {
    // Скелет `{"at": ""}` называет цель самой операцией.
    if args.get("at").is_some() {
        args["at"] = json!(at);
    }
    let input = json!({"at": at, "ops": [{"op": op, "args": args}], "dryRun": true});
    let source_set = at.split(':').next().unwrap();
    ApplyRequest::parse(input.as_object().unwrap(), &[source_set, "parent"])
        .unwrap_or_else(|error| panic!("{op} at {at}: {error}"))
}

/// Вердикт разбора семейства в обход реестра: `Some(false)` — адрес отвергнут,
/// `Some(true)` — вызов принят целиком, `None` — отказ по аргументам.
fn family_verdict(request: &ApplyRequest, binding: &ProviderRootBinding) -> Result<bool, String> {
    match parse_family_operation_for_test(0, &request.ops()[0], binding) {
        Ok(()) => Ok(true),
        Err(error)
            if error
                .path()
                .is_some_and(|path| path == "ops[0].args.at" || path == "ops[0].at") =>
        {
            Ok(false)
        }
        Err(error) => Err(format!("{:?}: {error}", error.path())),
    }
}

struct Checked<'a> {
    workspace: &'a Path,
    actor: &'a WorkspaceActor,
    binding: &'a ProviderRootBinding,
    source_root: PathBuf,
    traits: SourceSetTraits,
}

/// Обход одной фикстуры: расхождения, достигнутые маршруты и пары
/// «маршрут, вид», встреченные сужения.
fn check_fixture(name: &str) -> (BTreeSet<RouteClass>, BTreeSet<(RouteClass, String)>) {
    let mut routes = BTreeSet::new();
    let mut covered = BTreeSet::new();
    let mut mismatches = Vec::new();
    let fixture = fixture(name);
    NARROWINGS_SEEN.with(|seen| seen.borrow_mut().clear());
    WRITES_CHECKED.with(|done| done.borrow_mut().clear());
    for (set, kind, binding) in &fixture.sets {
        let checked = Checked {
            workspace: &fixture.workspace,
            actor: &fixture.actor,
            binding,
            source_root: source_root(&fixture.workspace, set),
            traits: SourceSetTraits::of_source(
                *kind,
                SourceFormat::PlatformXml,
                SourceProfile::platform_xml_8_3_27_format_2_20(),
            ),
        };
        let mut representatives = BTreeMap::new();
        let mut pairs = BTreeMap::new();
        for at in walk(&fixture.workspace, &format!("{set}:Configuration")) {
            let address = QualifiedAddress::parse(&at).unwrap();
            let Some(route) = classify_route(&address, PlatformProfile::v8_3_27()) else {
                continue;
            };
            let terminal = address.segments().last().unwrap().kind().as_str();
            routes.insert(route);
            covered.insert((route, terminal.to_string()));
            // Не больше двух форм адреса на пару «маршрут, вид»: словарь
            // сверяется на каждой, а время теста остаётся в пределах. Ветвь
            // коллекции и именованный узел того же вида считаются раздельно:
            // ветви не занимают места членов, на которых исполняются операции.
            let named = address.segments().last().unwrap().name().is_some();
            let per_pair = pairs.entry((route, terminal, named)).or_insert(0usize);
            let shape = shape_key(&address);
            if *per_pair < 2 && !representatives.contains_key(&shape) {
                *per_pair += 1;
                representatives.insert(shape, at);
            }
        }
        let mut planned_pairs = BTreeSet::new();
        for at in representatives.values() {
            let address = QualifiedAddress::parse(at).unwrap();
            let terminal = address.segments().last().unwrap();
            // Вложенный узел того же вида (`Subsystem.A.Subsystem.B`) — своё
            // место: только у него исполнимо исключение из родителя.
            let nested = address
                .segments()
                .iter()
                .rev()
                .nth(1)
                .is_some_and(|parent| parent.kind() == terminal.kind());
            let pair = (
                classify_route(&address, PlatformProfile::v8_3_27()),
                terminal.kind().as_str(),
                terminal.name().is_some(),
                nested,
            );
            // Планы дороги: файлы плана проверяются на одном узле пары
            // «маршрут, вид», словарь — на каждом представителе. Ветвь
            // коллекции (`main:Catalog`, `…Column`) и именованный узел того же
            // вида — разные места: на ветви писать нечего, и план узла не
            // должен уступать ей очередь.
            let plan_writes = planned_pairs.insert(pair);
            check_node(&checked, at, plan_writes, &mut mismatches);
        }
    }
    assert_eq!(mismatches, Vec::<String>::new(), "{name}");
    // Проверка записи не пустая: план с правдоподобными аргументами
    // прошёл хотя бы для этих операций, и их файлы сверены с узлом.
    let writes_checked = WRITES_CHECKED.with(|done| done.borrow().clone());
    let required = WRITES_CHECKED_AT_LEAST
        .iter()
        .find(|(fixture, _)| *fixture == name)
        .map_or(&[][..], |(_, ops)| *ops);
    for op in required {
        assert!(
            writes_checked.contains(*op),
            "{name}: the plan of `{op}` was not checked against its node; checked {writes_checked:?}"
        );
    }
    let seen = NARROWINGS_SEEN.with(|seen| seen.borrow().clone());
    for (index, narrowing) in INTENTIONAL_NARROWINGS.iter().enumerate() {
        assert!(!narrowing.reason.is_empty(), "a narrowing names its reason");
        if narrowing.witnessed_in == name {
            assert!(
                seen.contains(&index),
                "intentional narrowing ({}, {:?}, {}) was not observed in {name}: drop it",
                narrowing.op,
                narrowing.route,
                narrowing.kind
            );
        }
    }
    (routes, covered)
}

/// Маршруты и пары «маршрут, вид», которые каждая фикстура обязана показать.
/// Их объединение — все варианты `RouteClass`, кроме названных пробелов.
/// Фикстура, маршруты и пары «маршрут, вид», которые она обязана показать.
type FixtureCoverage = (
    &'static str,
    &'static [RouteClass],
    &'static [(RouteClass, &'static str)],
);

const FIXTURE_COVERAGE: &[FixtureCoverage] = &[
    (
        "workspace",
        &[
            RouteClass::Configuration,
            RouteClass::Metadata,
            RouteClass::Form,
            RouteClass::Role,
            RouteClass::Subsystem,
            RouteClass::Interface,
            RouteClass::Dcs,
            RouteClass::Xdto,
            RouteClass::Module,
        ],
        &[
            (RouteClass::Metadata, "Attribute"),
            (RouteClass::Metadata, "Method"),
            (RouteClass::Metadata, "Template"),
            (RouteClass::Metadata, "TabularSection"),
            (RouteClass::Metadata, "Dimension"),
            (RouteClass::Metadata, "Resource"),
            (RouteClass::Metadata, "EnumValue"),
            (RouteClass::Metadata, "Column"),
            (RouteClass::Form, "Attribute"),
            (RouteClass::Form, "Item"),
            (RouteClass::Form, "Event"),
            (RouteClass::Module, "Event"),
            (RouteClass::Module, "Method"),
            // Узлы схемы (#1357): источник, итог и элемент структуры
            // варианта с вложенной таблицей и диаграммой.
            (RouteClass::Dcs, "DataSource"),
            (RouteClass::Dcs, "Total"),
            (RouteClass::Dcs, "Item"),
        ],
    ),
    (
        "workspace-dcs",
        &[RouteClass::Dcs],
        &[
            (RouteClass::Metadata, "Template"),
            (RouteClass::Dcs, "DataSource"),
            (RouteClass::Dcs, "Item"),
            (RouteClass::Dcs, "Setting"),
        ],
    ),
    (
        "workspace-mxl",
        &[RouteClass::Mxl],
        &[
            (RouteClass::Mxl, "Area"),
            (RouteClass::Metadata, "Template"),
        ],
    ),
    (
        "workspace-code",
        &[RouteClass::Module],
        &[
            (RouteClass::Module, "Event"),
            (RouteClass::Module, "Method"),
        ],
    ),
    (
        "workspace-predefined",
        &[RouteClass::Metadata],
        &[(RouteClass::Metadata, "PredefinedItem")],
    ),
    (
        "workspace-resolve",
        &[RouteClass::Form, RouteClass::Module],
        &[(RouteClass::Form, "Attribute")],
    ),
    (
        "workspace-external",
        &[RouteClass::Form, RouteClass::Mxl, RouteClass::Module],
        &[(RouteClass::Mxl, "Area")],
    ),
];

fn check_fixture_coverage(name: &str) {
    let (routes, covered) = check_fixture(name);
    let (_, expected_routes, expected_pairs) = FIXTURE_COVERAGE
        .iter()
        .find(|(fixture, _, _)| *fixture == name)
        .copied()
        .unwrap_or((name, &[], &[]));
    for route in expected_routes {
        assert!(routes.contains(route), "{name} did not reach {route:?}");
    }
    for (route, kind) in expected_pairs {
        assert!(
            covered.contains(&(*route, kind.to_string())),
            "{name} did not reach ({route:?}, {kind})"
        );
    }
}

#[test]
fn honest_can_on_the_main_corpus_workspace() {
    check_fixture_coverage("workspace");
}

#[test]
fn honest_can_on_the_dcs_workspace() {
    check_fixture_coverage("workspace-dcs");
}

#[test]
fn honest_can_on_the_mxl_workspace() {
    check_fixture_coverage("workspace-mxl");
}

#[test]
fn honest_can_on_the_code_and_extension_workspace() {
    check_fixture_coverage("workspace-code");
}

#[test]
fn honest_can_on_the_predefined_workspace() {
    check_fixture_coverage("workspace-predefined");
}

#[test]
fn honest_can_on_the_resolve_workspace() {
    check_fixture_coverage("workspace-resolve");
}

#[test]
fn honest_can_on_external_sources() {
    check_fixture_coverage("workspace-external");
}

/// Вместе фикстуры доходят до каждого маршрута, кроме названных пробелов, и
/// до видов, совпадающих по имени в разных деревьях.
#[test]
fn fixture_coverage_reaches_every_route_class() {
    let reached = FIXTURE_COVERAGE
        .iter()
        .flat_map(|(_, routes, _)| routes.iter().copied())
        .collect::<BTreeSet<_>>();
    for route in [
        RouteClass::Configuration,
        RouteClass::Metadata,
        RouteClass::Form,
        RouteClass::Role,
        RouteClass::Subsystem,
        RouteClass::Interface,
        RouteClass::Dcs,
        RouteClass::Mxl,
        RouteClass::Xdto,
        RouteClass::Module,
    ] {
        let gap = COVERAGE_GAPS.iter().any(|(gap, _)| *gap == route);
        assert_eq!(
            reached.contains(&route),
            !gap,
            "route {route:?}: reached={}, listed as gap={gap}",
            reached.contains(&route)
        );
    }
    let pairs = FIXTURE_COVERAGE
        .iter()
        .flat_map(|(_, _, pairs)| pairs.iter().copied())
        .collect::<BTreeSet<_>>();
    for pair in [
        (RouteClass::Metadata, "Attribute"),
        (RouteClass::Form, "Attribute"),
        (RouteClass::Metadata, "Template"),
        (RouteClass::Mxl, "Area"),
        (RouteClass::Module, "Method"),
        (RouteClass::Module, "Event"),
    ] {
        assert!(
            pairs.contains(&pair),
            "no fixture is bound to reach {pair:?}"
        );
    }
}

fn source_root(workspace: &Path, set: &str) -> PathBuf {
    source_sets(workspace)
        .into_iter()
        .find(|(name, _, _)| name == set)
        .map(|(_, _, path)| path)
        .expect("declared source set")
}

thread_local! {
    static NARROWINGS_SEEN: std::cell::RefCell<BTreeSet<usize>> =
        const { std::cell::RefCell::new(BTreeSet::new()) };
    /// Операции, чьи файлы плана проверены: проверка записи не пустая.
    static WRITES_CHECKED: std::cell::RefCell<BTreeSet<String>> =
        const { std::cell::RefCell::new(BTreeSet::new()) };
}

fn check_node(checked: &Checked<'_>, at: &str, plan_writes: bool, mismatches: &mut Vec<String>) {
    let viewed = invoke(checked.workspace, ToolIdentity::View, json!({"at": at}));
    assert!(viewed.ok, "{at}: {viewed:?}");
    let data = viewed.data.unwrap();
    let can = data["can"]
        .as_array()
        .unwrap_or_else(|| panic!("{at}: no can in {data}"))
        .iter()
        .map(|entry| entry["op"].as_str().unwrap().to_string())
        .collect::<BTreeSet<_>>();
    let address = QualifiedAddress::parse(at).unwrap();
    let route = classify_route(&address, PlatformProfile::v8_3_27()).unwrap();
    let terminal = address.segments().last().unwrap().kind().as_str();
    let template = terminal == NodeKind::Template.as_str();
    // Роль элемента формы — фильтр словаря поверх места, как формат макета.
    let item_role = (terminal == NodeKind::Item.as_str())
        .then(|| data["props"]["tag"].as_str().map(FormItemRole::from_tag))
        .flatten();
    NARROWINGS_SEEN.with(|narrowings_seen| {
    for descriptor in OperationRegistry::closed().descriptors().iter().copied() {
        let op = descriptor.name();
        let listed = can.contains(op);
        let request = apply_request(at, op, node_args(descriptor, &address));
        let preflight = preflight_apply_operations(&request, checked.traits, &|_| None);
        if !checked.traits.writes(descriptor.family()) {
            // Набор без записи: словарь пуст, предварительная проверка
            // отвечает отказом набора, а не «не тот узел».
            let refused = preflight
                .as_ref()
                .err()
                .map(|refusal| refusal.diagnostics[0]["code"].clone());
            if listed || refused != Some(json!("provider_unavailable")) {
                mismatches.push(format!(
                    "{at}: `{op}` in a read-only source: listed={listed}, preflight={refused:?}"
                ));
            }
            continue;
        }
        let placed = preflight.is_ok();
        let family = family_verdict(&request, checked.binding);
        match (placed, &family) {
            (false, Ok(true)) => {
                let family_key = format!("family:{:?}", descriptor.family());
                let narrowed = INTENTIONAL_NARROWINGS.iter().position(|narrowing| {
                    (narrowing.op == op || narrowing.op == family_key)
                        && narrowing.route.is_none_or(|r| r == route)
                        && (narrowing.kind == "*" || narrowing.kind == terminal)
                });
                if let Some(index) = narrowed {
                    narrowings_seen.borrow_mut().insert(index);
                } else {
                    mismatches.push(format!(
                        "{at}: the family accepts `{op}`, the registry does not place it here"
                    ));
                }
            }
            (true, Ok(false)) => mismatches.push(format!(
                "{at}: the registry places `{op}`, the family refuses its target"
            )),
            (true, Err(reason)) if !ARGUMENT_DEPENDENT.iter().any(|(name, _)| *name == op) => {
                mismatches.push(format!(
                    "{at}: `{op}` is placed, but its sample arguments never reach the target verdict: {reason}"
                ))
            }
            _ => {}
        }
        // Формат макета проверяет планировщик на настоящем описании.
        let format_refused = placed
            && template
            && matches!(
                descriptor.family(),
                OperationFamily::Dcs | OperationFamily::Mxl
            )
            && plan(checked, &request)
                .err()
                .is_some_and(|message| message.contains("of format"));
        // Роль элемента проверяет планировщик на настоящей форме: операцию,
        // которую фильтр роли не выпускает, он отвергает.
        let role_refused = placed && !descriptor.admits_form_item(item_role);
        if role_refused && plan(checked, &request).is_ok() {
            mismatches.push(format!(
                "{at}: `{op}` is filtered by the item role {item_role:?}, but the planner accepts it"
            ));
        }
        let expected = placed && !format_refused && !role_refused;
        if listed != expected {
            mismatches.push(format!(
                "{at}: `{op}` can={listed}, preflight={placed}, template format refused={format_refused}, item role refused={role_refused}"
            ));
            continue;
        }
        if plan_writes && listed && family == Ok(true) {
            if let Ok(paths) = plan(checked, &request) {
                WRITES_CHECKED.with(|done| done.borrow_mut().insert(op.to_string()));
                check_written_paths(checked, &address, route, op, &paths, mismatches);
            }
        }
    }
    });
}

/// План операции на настоящем наборе: изменённые файлы или текст отказа.
fn plan(checked: &Checked<'_>, request: &ApplyRequest) -> Result<Vec<PathBuf>, String> {
    let admission = checked
        .actor
        .admit_apply(
            checked.binding,
            None,
            true,
            ProviderDeadline::from_budget(Duration::from_secs(10)),
            &CancellationToken::new(),
        )
        .map_err(|error| format!("{error:?}"))?;
    let (staged, _) = plan_hidden_v13_apply(request, checked.binding, &admission)
        .map_err(|error| error.to_string())?;
    Ok(staged
        .planned_changes()
        .into_iter()
        .map(|change| change.relative_path)
        .collect())
}

/// Файлы плана принадлежат объекту `at`; узлы формы и макета не выходят за
/// свою форму или макет, кроме регистрации в описании владельца.
fn check_written_paths(
    checked: &Checked<'_>,
    at: &QualifiedAddress,
    route: RouteClass,
    op: &str,
    paths: &[PathBuf],
    mismatches: &mut Vec<String>,
) {
    if route == RouteClass::Configuration || WRITES_BEYOND_NODE.iter().any(|(name, _)| *name == op)
    {
        return;
    }
    let segments = at.segments();
    let owner = &segments[0];
    // Модуль самой конфигурации не имеет каталога объекта: его текст лежит
    // в `Ext/` корня набора, и запись за его пределы — ошибка.
    if owner.kind() == NodeKind::Module {
        let expected = owner.name().map(|role| format!("Ext/{role}Module.bsl"));
        for path in paths {
            let path = path.to_string_lossy().replace('\\', "/");
            if expected.as_deref() != Some(path.as_str()) {
                mismatches.push(format!(
                    "{at}: `{op}` writes `{path}` instead of its own configuration module file"
                ));
            }
        }
        return;
    }
    let Some(owner_name) = owner.name() else {
        return;
    };
    let Some(layout) = crate::infrastructure::metadata_kinds::metadata_kind(owner.kind().as_str())
    else {
        mismatches.push(format!("{at}: `{op}` has no owner layout to check writes"));
        return;
    };
    let owner_dir = format!("{}/{owner_name}", layout.directory);
    let descriptor = format!("{owner_dir}.xml");
    let nested = segments.iter().find_map(|segment| match segment.kind() {
        NodeKind::Form if route != RouteClass::Metadata || segments.len() > 1 => segment
            .name()
            .map(|name| format!("{owner_dir}/Forms/{name}")),
        NodeKind::Template if segments.len() > 1 => segment
            .name()
            .map(|name| format!("{owner_dir}/Templates/{name}")),
        _ => None,
    });
    let prefix = nested.unwrap_or_else(|| owner_dir.clone());
    for path in paths {
        let path = path.to_string_lossy().replace('\\', "/");
        let inside = path == descriptor
            || path == format!("{prefix}.xml")
            || path.starts_with(&format!("{prefix}/"));
        if !inside {
            mismatches.push(format!(
                "{at}: `{op}` writes `{path}` outside its node (source root {})",
                checked.source_root.display()
            ));
        }
    }
}

/// Неприменимая операция направляет к `can` узла, где она исполнима, а
/// подробный контракт просит только там, где он вычисляется.
#[test]
fn a_misplaced_operation_routes_next_to_the_node_where_it_applies() {
    for (at, op, args, target, contract) in [
        (
            "cf:Report.Sales.Template.Print.Area.Header",
            "area.add",
            json!({"items": [{"name": "Footer", "cells": {"R1C1": "x"}}]}),
            "cf:Report.Sales.Template.Print",
            true,
        ),
        (
            "cf:Role.Reader.Right.Catalog_Items",
            "right.add",
            json!({"items": [{"object": "Catalog.Orders", "rights": {"Read": true}}]}),
            "cf:Role.Reader",
            true,
        ),
        (
            "cf:Catalog.Items.Form.Main.Item.Header",
            "formAttribute.add",
            json!({"items": [{"name": "Comment"}]}),
            "cf:Catalog.Items.Form.Main",
            true,
        ),
        (
            "cf:Catalog.Items.Form.Main.Item.Header.Event.OnChange",
            "element.add",
            json!({"items": [{"name": "Comment"}]}),
            "cf:Catalog.Items.Form.Main.Item.Header",
            true,
        ),
        (
            "cf:Role.Reader",
            "role.create",
            json!({"values": {"name": "Reader"}}),
            "cf:Configuration",
            false,
        ),
    ] {
        let request = apply_request(at, op, args);
        let refusal = preflight_apply_operations(
            &request,
            SourceSetTraits::read(SourceSetKind::Configuration, PlatformProfile::v8_3_27()),
            &|template| {
                (template.to_string() == "cf:Report.Sales.Template.Print")
                    .then_some(crate::domain::apply::TemplateFormat::SpreadsheetDocument)
            },
        )
        .expect_err("a misplaced operation refuses before planning");
        let value = serde_json::to_value(&*refusal).unwrap();
        assert_eq!(value["diagnostics"][0]["code"], "bad_value", "{value}");
        assert_eq!(value["diagnostics"][0]["outcome"], "fixCall", "{value}");
        let next = &refusal.next[0];
        assert_eq!(next["tool"], "unica.view", "{value}");
        // Контракт операции читается её адресом там, где он вычисляется,
        // иначе — словарь самого узла.
        let expected = if contract {
            format!("{target}.Can.{op}")
        } else {
            target.to_string()
        };
        assert_eq!(next["args"], json!({"at": expected}), "{value}");
    }
}

/// Без узла, где операция исполнима, отказ называет форму нужного адреса, а
/// `next` ведёт к словарю того же узла, если он читается, иначе к корню.
#[test]
fn without_an_executable_ancestor_the_refusal_names_the_address_shape() {
    let traits = SourceSetTraits::read(SourceSetKind::Configuration, PlatformProfile::v8_3_27());
    for (at, op, args, next_at, shape) in [
        (
            "cf:Interface",
            "groupOrder.set",
            json!({"values": {"groups": ["NavigationPanelImportant"]}}),
            "cf:Configuration",
            "`Subsystem.Name[.Subsystem.Child].Interface`",
        ),
        (
            "cf:Catalog.Items.Attribute.Code",
            "element.add",
            json!({"items": [{"name": "Comment"}]}),
            "cf:Catalog.Items.Attribute.Code",
            "`Kind.Name.Form.F[.Item.I…]`",
        ),
        (
            "cf:Report.Sales.Template.Schema.Area.Header",
            "area.add",
            json!({"items": [{"name": "Footer", "cells": {"R1C1": "x"}}]}),
            "cf:Report.Sales.Template.Schema.Area.Header",
            "of a spreadsheet",
        ),
        // Запись `*.set` создаёт не предок: на владельце она не исполнима,
        // и отказ называет форму адреса подузла (#1354).
        (
            "cf:Role.Reader",
            "right.set",
            json!({"values": {"right": "Read", "value": true}}),
            "cf:Role.Reader",
            "`Role.Name.Right.<Kind>_<Name>`",
        ),
    ] {
        let request = apply_request(at, op, args);
        let refusal = preflight_apply_operations(&request, traits, &|template| {
            (template.to_string() == "cf:Report.Sales.Template.Schema")
                .then_some(crate::domain::apply::TemplateFormat::DataCompositionSchema)
        })
        .expect_err("a misplaced operation refuses");
        let message = refusal.diagnostics[0]["message"].as_str().unwrap();
        assert!(message.contains(shape), "{op} at {at}: {message}");
        assert!(!message.contains("it applies at"), "{message}");
        assert_eq!(refusal.next[0]["args"]["at"], next_at, "{op} at {at}");
    }
}

/// Узел, которого нет, создаёт операция родителя: `next` отказа «не найден»
/// по адресу операции ведёт к словарю родителя.
#[test]
fn a_missing_target_routes_next_to_the_parent_dictionary() {
    use crate::infrastructure::native_operations::apply::{ApplyPlanError, ApplyPlanErrorKind};
    for (at, parent) in [
        ("cf:Document.Order.Attribute.Missing", "cf:Document.Order"),
        ("cf:Catalog.Missing", "cf:Configuration"),
    ] {
        let request = apply_request(at, "props.set", json!({"values": {"Comment": "x"}}));
        let error =
            ApplyPlanError::new(ApplyPlanErrorKind::NotFound, "missing").at_path("ops[0].args.at");
        let refuse = |exists: Option<bool>| {
            super::v13_service::with_missing_target_parent(
                DomainResult::canonical_rejection(
                    None,
                    crate::domain::refusal::RefusalCode::NotFound,
                    "not found",
                ),
                &request,
                &error,
                &|_| exists,
            )
        };
        assert_eq!(refuse(Some(false)).next[0]["args"]["at"], parent);
        // Узел есть, а ресурса нет (`Rights.xml`, `Form.xml`) — создавать
        // на родителе нечего; неизвестное существование тоже не ведёт туда.
        assert!(refuse(Some(true)).next.is_empty(), "{at}");
        assert!(refuse(None).next.is_empty(), "{at}");
    }
}

/// Настоящий вызов: изменение реквизита, которого нет, отвечает «не найден»,
/// и `next` ведёт к словарю владельца, где реквизит создают.
#[test]
fn a_missing_member_answers_not_found_with_the_owner_dictionary_next() {
    let fixture = fixture("workspace");
    let result = invoke(
        &fixture.workspace,
        ToolIdentity::Apply,
        json!({
            "at": "main:Catalog.Валюты.Attribute.НетТакого",
            "ops": [{"op": "attribute.remove"}]
        }),
    );
    let value = serde_json::to_value(&result).unwrap();
    assert!(!result.ok, "{value}");
    assert_eq!(value["diagnostics"][0]["code"], "not_found", "{value}");
    assert!(!result.next.is_empty(), "{value}");
    assert_eq!(
        result.next[0]["args"],
        json!({"at": "main:Catalog.Валюты"}),
        "{value}"
    );
}

/// Матрица словаря формы (#1355): на форме, на контейнере любой глубины,
/// на поле, на автоматической командной панели и на событиях формы и
/// элемента `can` называет ровно те операции формы, которые `apply`
/// исполняет на этом адресе, и каждая из них строит предпросмотр.
#[test]
fn honest_can_on_form_items_and_events() {
    let fixture = fixture("workspace");
    let form = "main:Catalog.Валюты.Form.ФормаЭлемента";
    let list = "main:Catalog.Валюты.Form.ФормаСписка";
    let form_ops = [
        "form.remove",
        "element.add",
        "element.remove",
        "formAttribute.add",
        "formCommand.add",
        "event.bind",
        "event.implement",
    ];
    let cases: [(String, &[&str]); 8] = [
        (
            form.to_string(),
            &[
                "form.remove",
                "element.add",
                "formAttribute.add",
                "formCommand.add",
            ],
        ),
        (
            format!("{form}.Item.ГруппаСлужебная"),
            &["element.add", "element.remove"],
        ),
        (format!("{form}.Item.ПолеНаименование"), &["element.remove"]),
        (
            format!("{form}.Item.ФормаКоманднаяПанель"),
            &["element.add"],
        ),
        (
            format!("{list}.Item.List.Item.ListCommandBar"),
            &["element.add"],
        ),
        (
            format!("{form}.Event.OnOpen"),
            &["event.bind", "event.implement"],
        ),
        (
            format!("{form}.Item.ПолеНаименование.Event.OnChange"),
            &["event.bind", "event.implement"],
        ),
        // Событие источника динамического списка: каталог выбирает его
        // по пути данных таблицы и для узла, и для привязки.
        (
            format!("{list}.Item.List.Event.OnUpdateUserSettingSetAtServer"),
            &["event.bind", "event.implement"],
        ),
    ];
    let args = |op: &str, at: &str| match op {
        "element.add" => {
            json!({"items": [{"name": "Проба", "type": "Button", "stdCommand": "Close"}]})
        }
        "element.remove" | "form.remove" | "event.implement" => json!({"at": at}),
        "formAttribute.add" => json!({"items": [{"name": "Проба", "type": "String"}]}),
        "formCommand.add" => json!({"items": [{"name": "Проба"}]}),
        "event.bind" => json!({"values": {"handler": "Проба"}}),
        other => panic!("{other} has no sample"),
    };
    for (at, expected) in cases {
        let viewed = invoke(&fixture.workspace, ToolIdentity::View, json!({"at": at}));
        assert!(viewed.ok, "{at}: {viewed:?}");
        let can = viewed.data.unwrap()["can"]
            .as_array()
            .unwrap()
            .iter()
            .map(|entry| entry["op"].as_str().unwrap().to_string())
            .filter(|op| form_ops.contains(&op.as_str()))
            .collect::<BTreeSet<_>>();
        let expected = expected
            .iter()
            .map(|op| (*op).to_string())
            .collect::<BTreeSet<_>>();
        assert_eq!(can, expected, "{at}");
        for op in form_ops {
            let result = invoke(
                &fixture.workspace,
                ToolIdentity::Apply,
                json!({"at": at, "ops": [{"op": op, "args": args(op, &at)}]}),
            );
            assert_eq!(
                result.ok,
                expected.contains(op),
                "{op} at {at}: {}",
                serde_json::to_value(&result).unwrap()
            );
            // В поле элемент не добавить: отказ до планирования ведёт
            // к ближайшему узлу, где `element.add` исполнима.
            if op == "element.add" && at.ends_with(".Item.ПолеНаименование") {
                assert_eq!(result.at.as_deref(), Some("ops[0].at"), "{result:?}");
                assert_eq!(
                    result.next[0]["args"]["at"],
                    format!("{form}.Can.element.add"),
                    "{result:?}"
                );
            }
        }
        // Подробный контракт каждой операции узла читается её адресом, и его
        // пример строит предпросмотр на этом узле: контракт и планировщик
        // согласны (#1447).
        for op in expected.iter() {
            let detailed = invoke(
                &fixture.workspace,
                ToolIdentity::View,
                json!({"at": format!("{at}.Can.{op}")}),
            );
            assert!(detailed.ok, "{op} at {at}: {detailed:?}");
            let contract = detailed.data.unwrap()["props"]["contract"].clone();
            assert!(
                contract["argsSchema"].is_object(),
                "{op} at {at}: {contract}"
            );
            // Командная панель принимает только кнопки: её контракт публикует
            // один вид, а поле ввода в ней планировщик отвергает.
            let command_bar = at.ends_with("КоманднаяПанель") || at.ends_with("ListCommandBar");
            if *op == "element.add" {
                let kinds = contract["argsSchema"]["properties"]["items"]["items"]["anyOf"]
                    .as_array()
                    .unwrap()
                    .len();
                assert_eq!(kinds == 1, command_bar, "{at}: {kinds} element kinds");
                if command_bar {
                    let field = invoke(
                        &fixture.workspace,
                        ToolIdentity::Apply,
                        json!({"at": at, "ops": [{"op": op, "args":
                            {"items": [{"name": "Проба", "type": "InputField"}]}}]}),
                    );
                    assert!(!field.ok, "{at}: a field in a command bar is refused");
                }
            }
            let example = contract["exampleArgs"].clone();
            let planned = invoke(
                &fixture.workspace,
                ToolIdentity::Apply,
                json!({"at": at, "ops": [{"op": op, "args": example}]}),
            );
            assert!(
                planned.ok,
                "{op} at {at}: {}",
                serde_json::to_value(&planned).unwrap()
            );
        }
    }
}

/// Формы (#1355): контейнер, удаляемый элемент и событие, прежде названные
/// аргументом, задаёт адрес. Настоящий вызов со снятым аргументом получает
/// `bad_value` до планирования; первый `next` ведёт к узлу по его пути
/// в форме (вложенная командная панель таблицы — через таблицу), второй —
/// к ветви его вида; исправленный вызов по первому `next` строит
/// предпросмотр.
#[test]
fn retired_form_arguments_route_next_to_the_item_or_event_address() {
    let fixture = fixture("workspace");
    let element = "main:Catalog.Валюты.Form.ФормаЭлемента";
    let list = "main:Catalog.Валюты.Form.ФормаСписка";
    let button = json!({"name": "Проба", "type": "Button", "stdCommand": "Close"});
    for (at, op, args, location, next, branch, fixed) in [
        (
            list.to_string(),
            "element.add",
            json!({"items": [{"name": "Проба", "type": "Button", "stdCommand": "Close", "into": "ListCommandBar"}]}),
            "ops[0].args.items[0].into",
            format!("{list}.Item.List.Item.ListCommandBar"),
            format!("{list}.Item.List.Item"),
            json!({"items": [button.clone()]}),
        ),
        (
            element.to_string(),
            "element.remove",
            json!({"items": [{"name": "ГруппаСлужебная"}]}),
            "ops[0].args.items[0].name",
            format!("{element}.Item.ГруппаСлужебная"),
            format!("{element}.Item"),
            json!({}),
        ),
        (
            element.to_string(),
            "event.bind",
            json!({"values": {"event": "ПриИзменении", "handler": "ПолеНаименованиеПриИзменении", "element": "ПолеНаименование"}}),
            "ops[0].args.values.element",
            format!("{element}.Item.ПолеНаименование.Event.OnChange"),
            format!("{element}.Item.ПолеНаименование.Event"),
            json!({"values": {"handler": "ПолеНаименованиеПриИзменении"}}),
        ),
        (
            element.to_string(),
            "event.bind",
            json!({"values": {"event": "ПриОткрытии", "handler": "ПриОткрытии"}}),
            "ops[0].args.values.event",
            format!("{element}.Event.OnOpen"),
            format!("{element}.Event"),
            json!({"values": {"handler": "ПриОткрытии"}}),
        ),
    ] {
        let case = format!("{op} at {at}");
        let refused = invoke(
            &fixture.workspace,
            ToolIdentity::Apply,
            json!({"at": at, "ops": [{"op": op, "args": args}]}),
        );
        let value = serde_json::to_value(&refused).unwrap();
        assert!(!refused.ok, "{case}: {value}");
        assert_eq!(
            value["diagnostics"][0]["code"], "bad_value",
            "{case}: {value}"
        );
        assert_eq!(refused.at.as_deref(), Some(location), "{case}: {value}");
        let actual = refused
            .next
            .iter()
            .map(|next| next["args"]["at"].as_str().unwrap().to_string())
            .collect::<Vec<_>>();
        assert_eq!(
            actual,
            vec![format!("{next}.Can.{op}"), branch],
            "{case}: {value}"
        );
        let viewed = invoke(&fixture.workspace, ToolIdentity::View, json!({"at": next}));
        assert!(viewed.ok, "{case}: {viewed:?}");
        let can = &viewed.data.unwrap()["can"];
        assert!(
            can.as_array()
                .unwrap()
                .iter()
                .any(|entry| entry["op"] == op),
            "{case}: {can}"
        );
        let corrected = invoke(
            &fixture.workspace,
            ToolIdentity::Apply,
            json!({"at": next, "ops": [{"op": op, "args": fixed}]}),
        );
        assert!(
            corrected.ok,
            "{case}: {}",
            serde_json::to_value(&corrected).unwrap()
        );
    }
    // Без имени события значение называет только элемент: `next` ведёт
    // к его событиям, где видны имена.
    let refused = invoke(
        &fixture.workspace,
        ToolIdentity::Apply,
        json!({"at": element, "ops": [{"op": "event.bind", "args": {"values": {"handler": "H", "element": "ПолеНаименование"}}}]}),
    );
    assert!(!refused.ok);
    assert_eq!(
        refused.next[0]["args"]["at"],
        format!("{element}.Item.ПолеНаименование.Event")
    );
}

/// Рабочее пространство с двумя наборами конфигурации: первичный `base` и
/// `sales` с документом и справочником корпуса — проверка того, что адрес
/// сохраняет непервичный набор исходников.
fn two_set_workspace() -> (tempfile::TempDir, PathBuf) {
    let root = tempfile::tempdir().unwrap();
    let workspace = std::fs::canonicalize(root.path()).unwrap().join("ws");
    let fixtures = repo_root().join("tests/fixtures/acceptance");
    copy_tree(
        &fixtures.join("workspace-code/src/cf"),
        &workspace.join("src/base"),
    );
    copy_tree(
        &fixtures.join("workspace/src"),
        &workspace.join("src/sales"),
    );
    std::fs::write(
        workspace.join("v8project.yaml"),
        "format: DESIGNER\nsource-set:\n  - name: base\n    type: CONFIGURATION\n    path: src/base\n  - name: sales\n    type: CONFIGURATION\n    path: src/sales\n",
    )
    .unwrap();
    (root, workspace)
}

fn tree_digest(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    let mut files = BTreeMap::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).unwrap() {
            let entry = entry.unwrap();
            let path = entry.path();
            if entry.file_type().unwrap().is_dir() {
                if path.file_name().is_some_and(|name| name == ".build") {
                    continue;
                }
                stack.push(path);
            } else {
                files.insert(path.clone(), std::fs::read(&path).unwrap());
            }
        }
    }
    files
}

/// #1318: вызов со снятым `scope.tabularSection` получает отказ, `next`
/// которого ведёт к словарю табличной части с подробным контрактом
/// `attribute.add`; исправленный по нему вызов строит исполнимый план,
/// публикация кладёт реквизит в часть, а не в шапку, и сохраняет соседей.
/// Отказ, чтение и предпросмотр исходников не меняют. Набор исходников —
/// непервичный.
#[test]
fn a_retired_scope_call_recovers_through_next_to_a_published_tabular_attribute() {
    let (_root, workspace) = two_set_workspace();
    let document = "sales:Document.АктОбУничтоженииПерсональныхДанных";
    let section = format!("{document}.TabularSection.КатегорииДанных");
    let item = json!({
        "name": "Основание",
        "type": {"variants": [{"kind": "string", "length": 100, "allowedLength": "variable"}]}
    });
    let before = tree_digest(&workspace.join("src"));
    let runtime = V5CanonicalInvocationRuntime::new(
        Arc::new(CanonicalV13ReadService::default()),
        Arc::new(TokioClock),
    );
    let invoke =
        |workspace: &Path, tool, arguments| invoke_on(&runtime, workspace, tool, arguments);

    let refused = invoke(
        &workspace,
        ToolIdentity::Apply,
        json!({"at": document, "ops": [{"op": "attribute.add", "args": {
            "scope": {"tabularSection": "КатегорииДанных"},
            "items": [item.clone()]
        }}]}),
    );
    let value = serde_json::to_value(&refused).unwrap();
    assert!(!refused.ok, "{value}");
    assert_eq!(value["diagnostics"][0]["code"], "bad_value", "{value}");
    assert_eq!(value["diagnostics"][0]["outcome"], "fixCall", "{value}");
    assert_eq!(value["at"], "ops[0].args.scope.tabularSection", "{value}");
    let next = refused.next[0].clone();
    assert_eq!(next["tool"], "unica.view", "{value}");
    assert_eq!(
        next["args"],
        json!({"at": format!("{section}.Can.attribute.add")}),
        "{value}"
    );

    let viewed = invoke(&workspace, ToolIdentity::View, next["args"].clone());
    let viewed_value = serde_json::to_value(&viewed).unwrap();
    assert!(viewed.ok, "{viewed_value}");
    let entry = &viewed_value["data"]["props"];
    assert_eq!(entry["op"], "attribute.add", "{viewed_value}");
    assert_eq!(entry["implemented"], true, "{viewed_value}");
    let contract = &entry["contract"];
    assert!(
        contract["argsSchema"]["properties"]["items"]["items"]["properties"]["type"].is_object(),
        "the contract shows the attribute type: {contract}"
    );
    assert!(
        !contract["argsSchema"].to_string().contains("\"scope\""),
        "no scope argument is published: {contract}"
    );
    assert!(contract["exampleArgs"]["items"].is_array(), "{contract}");

    let preview = invoke(
        &workspace,
        ToolIdentity::Apply,
        json!({"at": section, "ops": [{"op": "attribute.add", "args": {"items": [item]}}]}),
    );
    let preview_value = serde_json::to_value(&preview).unwrap();
    assert!(preview.ok, "{preview_value}");
    assert_eq!(preview_value["data"]["executable"], true, "{preview_value}");
    assert_eq!(
        tree_digest(&workspace.join("src")),
        before,
        "refusal, reading and preview leave the sources untouched"
    );

    let token = preview_value["data"]["executionToken"].clone();
    let published = invoke(
        &workspace,
        ToolIdentity::Apply,
        json!({"executionToken": token}),
    );
    assert!(
        published.ok,
        "{}",
        serde_json::to_value(&published).unwrap()
    );
    let added = invoke(
        &workspace,
        ToolIdentity::View,
        json!({"at": format!("{section}.Attribute.Основание")}),
    );
    assert!(added.ok, "{:?}", added);
    let header = invoke(
        &workspace,
        ToolIdentity::View,
        json!({"at": format!("{document}.Attribute.Основание")}),
    );
    assert!(!header.ok, "the attribute is not a header attribute");
    for sibling in ["Субъект", "ИмяОбъекта"] {
        let kept = invoke(
            &workspace,
            ToolIdentity::View,
            json!({"at": format!("{section}.Attribute.{sibling}")}),
        );
        assert!(kept.ok, "{sibling} stays");
    }
    let descriptor = workspace.join("src/sales/Documents/АктОбУничтоженииПерсональныхДанных.xml");
    let after = std::fs::read_to_string(&descriptor).unwrap();
    let original = String::from_utf8(before[&descriptor].clone()).unwrap();
    assert_eq!(
        after.matches("<xr:GeneratedType").count(),
        original.matches("<xr:GeneratedType").count(),
        "an attribute brings no generated type"
    );
}

/// Реквизит шапки документа и реквизит справочника по-прежнему добавляются
/// на адресе владельца.
#[test]
fn header_and_catalog_attributes_still_add_at_the_owner() {
    let fixture = fixture("workspace");
    for owner in [
        "main:Document.АктОбУничтоженииПерсональныхДанных",
        "main:Catalog.Валюты",
    ] {
        let preview = invoke(
            &fixture.workspace,
            ToolIdentity::Apply,
            json!({"at": owner, "ops": [{"op": "attribute.add", "args": {"items": [{
                "name": "Примечание",
                "type": {"variants": [{"kind": "string", "length": 0, "allowedLength": "variable"}]}
            }]}}]}),
        );
        assert!(
            preview.ok,
            "{owner}: {}",
            serde_json::to_value(&preview).unwrap()
        );
    }
}

/// Аргументы, дублировавшие `at` (П2, #1353): вызов получает `bad_value` до
/// планирования, первый `next` ведёт к узлу, который называло значение (к его
/// контракту, где он вычисляется), второй — к ветви узлов этого вида. Когда
/// значение повторяло сам адрес вызова, отказ говорит снять аргумент.
#[test]
fn retired_duplicate_arguments_route_next_to_the_addressed_node() {
    let traits = SourceSetTraits::read(SourceSetKind::Configuration, PlatformProfile::v8_3_27());
    // Прежний псевдоним запроса ведёт к запросу набора, а операция набора —
    // к самому набору.
    for (op, args, target) in [
        (
            "query.set",
            json!({"values": {"query": "ВЫБРАТЬ 1"}}),
            "cf:Report.Sales.Template.Schema.DataSet.Main.Query.Can.query.set",
        ),
        (
            "field.add",
            json!({"items": [{"dataPath": "Amount"}]}),
            "cf:Report.Sales.Template.Schema.DataSet.Main.Can.field.add",
        ),
    ] {
        let request = apply_request("cf:Report.Sales.Template.Schema.Query.Main", op, args);
        let refusal = preflight_apply_operations(&request, traits, &|_| None)
            .expect_err("the retired query alias refuses");
        assert_eq!(refusal.diagnostics[0]["code"], "bad_value", "{op}");
        assert_eq!(refusal.at.as_deref(), Some("ops[0].at"), "{op}");
        assert_eq!(refusal.next.len(), 1, "{op}");
        assert_eq!(refusal.next[0]["args"], json!({"at": target}), "{op}");
    }
    let schema = "cf:Report.Sales.Template.Schema";
    // Операция запроса на самом наборе — обычный отказ «не тот узел» с `next`
    // к запросу набора.
    let request = apply_request(
        &format!("{schema}.DataSet.Main"),
        "query.set",
        json!({"values": {"query": "ВЫБРАТЬ 1"}}),
    );
    let refusal = preflight_apply_operations(&request, traits, &|_| None)
        .expect_err("query.set is not executable at the dataset itself");
    assert_eq!(refusal.at.as_deref(), Some("ops[0].at"));
    assert_eq!(
        refusal.next[0]["args"],
        json!({"at": format!("{schema}.DataSet.Main.Query.Can.query.set")})
    );
    // Набор, не названный адресом вызова, ищется в схеме: член объединения
    // получает свой вложенный адрес.
    let request = apply_request(
        &format!("{schema}.DataSet.Main.Query"),
        "query.set",
        json!({"values": {"dataSet": "Member", "query": "ВЫБРАТЬ 1"}}),
    );
    let refusal = super::v13_service::preflight_apply_operations_with(
        &request,
        traits,
        &|_| None,
        &|_| None,
        &|template, name| {
            if template.to_string() == schema && name == "Member" {
                crate::domain::apply::NamedLookup::Found(vec![
                    "Union".to_string(),
                    "Member".to_string(),
                ])
            } else {
                crate::domain::apply::NamedLookup::Missing
            }
        },
    )
    .expect_err("the dataSet argument is retired");
    let actual = refusal
        .next
        .iter()
        .map(|next| next["args"]["at"].clone())
        .collect::<Vec<_>>();
    assert_eq!(
        actual,
        vec![
            json!(format!(
                "{schema}.DataSet.Union.DataSet.Member.Query.Can.query.set"
            )),
            json!(format!("{schema}.DataSet.Union.DataSet")),
        ]
    );
    for (at, op, args, location, next_at, branch, duplicate) in [
        (
            format!("{schema}.DataSet.Main.Query"),
            "query.set",
            json!({"values": {"dataSet": "Main", "query": "ВЫБРАТЬ 1"}}),
            "ops[0].args.values.dataSet",
            format!("{schema}.DataSet.Main.Query.Can.query.set"),
            format!("{schema}.DataSet"),
            true,
        ),
        (
            format!("{schema}.DataSet.Main.Query"),
            "query.patch",
            json!({"values": {"dataSet": "Other", "find": "1"}}),
            "ops[0].args.values.dataSet",
            format!("{schema}.DataSet.Other.Query.Can.query.patch"),
            format!("{schema}.DataSet"),
            false,
        ),
        (
            format!("{schema}.Setting.Main"),
            "structure.set",
            json!({"values": {"variant": "Extra", "groupBy": []}}),
            "ops[0].args.values.variant",
            format!("{schema}.Setting.Extra.Can.structure.set"),
            format!("{schema}.Setting"),
            false,
        ),
        (
            "cf:Subsystem.Sales".to_string(),
            "childSubsystem.remove",
            json!({"items": [{"name": "Orders"}]}),
            "ops[0].args.items[0].name",
            "cf:Subsystem.Sales.Subsystem.Orders".to_string(),
            "cf:Subsystem.Sales.Subsystem".to_string(),
            false,
        ),
        (
            "cf:CommonModule.Shared".to_string(),
            "code.replace",
            json!({"selector": {"method": "Run"}, "text": "Procedure Run()\nEndProcedure"}),
            "ops[0].args.selector.method",
            "cf:CommonModule.Shared.Method.Run".to_string(),
            "cf:CommonModule.Shared.Method".to_string(),
            false,
        ),
    ] {
        let request = apply_request(&at, op, args);
        let refusal = preflight_apply_operations(&request, traits, &|_| None)
            .expect_err("a retired argument refuses before planning");
        let case = format!("{op} at {at}");
        assert_eq!(refusal.diagnostics[0]["code"], "bad_value", "{case}");
        assert_eq!(refusal.diagnostics[0]["outcome"], "fixCall", "{case}");
        assert_eq!(refusal.at.as_deref(), Some(location), "{case}");
        let message = refusal.diagnostics[0]["message"].as_str().unwrap();
        assert_eq!(
            message.contains("takes its subject from `at`"),
            duplicate,
            "{case}: {message}"
        );
        let expected = [next_at, branch]
            .into_iter()
            .filter(|at| !at.is_empty())
            .map(|at| json!({"at": at}))
            .collect::<Vec<_>>();
        let actual = refusal
            .next
            .iter()
            .map(|next| {
                assert_eq!(next["tool"], "unica.view", "{case}");
                next["args"].clone()
            })
            .collect::<Vec<_>>();
        assert_eq!(actual, expected, "{case}");
    }
}

/// Операции подузлов (#1354): подузел, прежде названный аргументом на
/// владельце, задаёт адрес. Прежний вызов получает `bad_value` до
/// планирования: первый `next` — к `can` подузла, второй — к ветви узлов
/// этого вида. `*.set`, чей узел создаёт операция родителя, называет её.
#[test]
fn retired_subnode_arguments_route_next_to_the_subnode_address() {
    let traits = SourceSetTraits::read(SourceSetKind::Configuration, PlatformProfile::v8_3_27());
    let schema = "cf:Report.Sales.Template.Schema";
    for (at, op, args, location, next_at, branch, creator) in [
        (
            "cf:Role.Reader".to_string(),
            "right.set",
            json!({"values": {"object": "Catalog.Items", "right": "Read", "value": true}}),
            "ops[0].args.values.object",
            "cf:Role.Reader.Right.Catalog_Items.Can.right.set".to_string(),
            "cf:Role.Reader.Right".to_string(),
            Some("right.add"),
        ),
        (
            "cf:Report.Sales.Template.Print".to_string(),
            "mxl.set",
            json!({"values": {"area": "Header", "cells": {"R1C1": "x"}}}),
            "ops[0].args.values.area",
            "cf:Report.Sales.Template.Print.Area.Header.Can.mxl.set".to_string(),
            "cf:Report.Sales.Template.Print.Area".to_string(),
            Some("area.add"),
        ),
        (
            schema.to_string(),
            "variant.set",
            json!({"values": {"name": "Main", "title": "x"}}),
            "ops[0].args.values.name",
            format!("{schema}.Setting.Main.Can.variant.set"),
            format!("{schema}.Setting"),
            None,
        ),
        (
            schema.to_string(),
            "parameter.rename",
            json!({"values": {"name": "Period", "newName": "Range"}}),
            "ops[0].args.values.name",
            format!("{schema}.Parameter.Period.Can.parameter.rename"),
            format!("{schema}.Parameter"),
            None,
        ),
        (
            format!("{schema}.DataSet.Main"),
            "fieldRole.set",
            json!({"values": {"field": "Amount", "role": {"dimension": true}}}),
            "ops[0].args.values.field",
            format!("{schema}.DataSet.Main.Field.Amount.Can.fieldRole.set"),
            format!("{schema}.DataSet.Main.Field"),
            None,
        ),
        (
            "cf:CommonModule.Shared".to_string(),
            "code.insert",
            json!({"selector": {"method": "Run"}, "position": "after", "text": "x"}),
            "ops[0].args.selector.method",
            "cf:CommonModule.Shared.Method.Run".to_string(),
            "cf:CommonModule.Shared.Method".to_string(),
            None,
        ),
    ] {
        let request = apply_request(&at, op, args);
        let refusal = preflight_apply_operations(&request, traits, &|_| None)
            .expect_err("a retired subnode argument refuses before planning");
        let case = format!("{op} at {at}");
        assert_eq!(refusal.diagnostics[0]["code"], "bad_value", "{case}");
        assert_eq!(refusal.at.as_deref(), Some(location), "{case}");
        let message = refusal.diagnostics[0]["message"].as_str().unwrap();
        match creator {
            Some(creator) => assert!(
                message.contains(&format!("created by `{creator}`")),
                "{case}: {message}"
            ),
            None => assert!(!message.contains("created by"), "{case}: {message}"),
        }
        let actual = refusal
            .next
            .iter()
            .map(|next| next["args"]["at"].clone())
            .collect::<Vec<_>>();
        // Узел создаёт операция родителя: её контракт — третий шаг.
        let mut expected = vec![json!(next_at), json!(branch)];
        if let Some(creator) = creator {
            let parent = branch.rsplit_once('.').unwrap().0;
            expected.push(json!(format!("{parent}.Can.{creator}")));
        }
        assert_eq!(actual, expected, "{case}");
    }
    // Контракт операций прав читается по адресу операции, как у других.
    for op in ["right.add", "right.set"] {
        let descriptor = OperationRegistry::closed().lookup(op).unwrap();
        let contract = descriptor.argument_contract().expect("role contract");
        assert!(
            contract.normalize(&contract.example_args, "args").is_ok(),
            "{op}"
        );
    }
    // Снятая `form.create` называет `form.add` на владельце формы.
    let request = apply_request(
        "cf:Catalog.Items.Form.Choice",
        "form.create",
        json!({"values": {"type": "ChoiceForm"}}),
    );
    let refusal = preflight_apply_operations(&request, traits, &|_| None)
        .expect_err("a retired operation refuses before planning");
    assert_eq!(refusal.diagnostics[0]["code"], "bad_value");
    assert_eq!(refusal.at.as_deref(), Some("ops[0].op"));
    let message = refusal.diagnostics[0]["message"].as_str().unwrap();
    assert!(
        message.contains("call `form.add` at `cf:Catalog.Items`"),
        "{message}"
    );
    assert_eq!(refusal.next.len(), 1);
    assert_eq!(refusal.next[0]["args"]["at"], "cf:Catalog.Items");
}

/// Новые узлы схемы (#1357): снятый `group`, `parent` или `name` ведёт
/// `next` к `can` элемента структуры и к ветви `Setting.V.Item`, а снятое
/// имя источника, вычисляемого поля и итога — к их адресу. Повторённое имя
/// ведёт к ветви и к `can` каждого совпавшего элемента.
#[test]
fn retired_dcs_container_arguments_route_next_to_the_structure_item() {
    let traits = SourceSetTraits::read(SourceSetKind::Configuration, PlatformProfile::v8_3_27());
    let schema = "cf:Report.Sales.Template.Schema";
    let setting = format!("{schema}.Setting.Main");
    let lookup = |owner: &QualifiedAddress, name: &str| match name {
        "Details" if owner.to_string().ends_with(".Setting.Main") => {
            crate::domain::apply::NamedLookup::Found(vec!["Top".into(), "Details".into()])
        }
        "Twin" if owner.to_string().ends_with(".Setting.Main") => {
            crate::domain::apply::NamedLookup::Ambiguous(vec![
                vec!["Top".into(), "Twin".into()],
                vec!["Table".into(), "Twin".into()],
            ])
        }
        _ => crate::domain::apply::NamedLookup::Missing,
    };
    for (at, op, args, location, expected) in [
        (
            setting.clone(),
            "filter.add",
            json!({"items": [{"field": "Amount", "group": "Details"}]}),
            "ops[0].args.items[0].group",
            vec![
                format!("{setting}.Item.Top.Item.Details.Can.filter.add"),
                format!("{setting}.Item.Top.Item"),
            ],
        ),
        (
            setting.clone(),
            "structure.patch",
            json!({"values": {"name": "Details", "groupBy": ["Amount"]}}),
            "ops[0].args.values.name",
            vec![
                format!("{setting}.Item.Top.Item.Details.Can.structure.patch"),
                format!("{setting}.Item.Top.Item"),
            ],
        ),
        (
            setting.clone(),
            "order.clear",
            json!({"values": {"group": "Twin"}}),
            "ops[0].args.values.group",
            vec![
                format!("{setting}.Item"),
                format!("{setting}.Item.Top.Item.Twin.Can.order.clear"),
                format!("{setting}.Item.Table.Item.Twin.Can.order.clear"),
            ],
        ),
        (
            schema.to_string(),
            "total.set",
            json!({"values": {"field": "Amount", "expression": "Сумма(Amount)"}}),
            "ops[0].args.values.field",
            vec![
                format!("{schema}.Total.Amount.Can.total.set"),
                format!("{schema}.Total"),
            ],
        ),
        (
            schema.to_string(),
            "dataSource.remove",
            json!({"values": {"name": "Local"}}),
            "ops[0].args.values.name",
            vec![
                format!("{schema}.DataSource.Local.Can.dataSource.remove"),
                format!("{schema}.DataSource"),
            ],
        ),
    ] {
        let request = apply_request(&at, op, args);
        let refusal = super::v13_service::preflight_apply_operations_with(
            &request,
            traits,
            &|_| None,
            &|_| None,
            &lookup,
        )
        .expect_err("a retired DCS argument refuses before planning");
        let case = format!("{op} at {at}");
        assert_eq!(refusal.diagnostics[0]["code"], "bad_value", "{case}");
        assert_eq!(refusal.at.as_deref(), Some(location), "{case}");
        let actual = refusal
            .next
            .iter()
            .map(|next| next["args"]["at"].clone())
            .collect::<Vec<_>>();
        let expected = expected.into_iter().map(|at| json!(at)).collect::<Vec<_>>();
        assert_eq!(actual, expected, "{case}");
    }
}

/// Снятые аргументы членов метаданных: прежний вызов на владельце получает
/// `bad_value` с адресом члена; предопределённый элемент, прежде названный
/// идентификатором, ведёт к коллекции, где видно его имя.
#[test]
fn retired_member_arguments_route_next_to_the_member_address() {
    let traits = SourceSetTraits::read(SourceSetKind::Configuration, PlatformProfile::v8_3_27());
    for (at, op, args, location, next_at, branch) in [
        (
            "cf:Catalog.Items",
            "tabularSection.set",
            json!({"values": {"name": "Rows", "synonym": "x"}}),
            "ops[0].args.values.name",
            "cf:Catalog.Items.TabularSection.Rows.Can.tabularSection.set",
            Some("cf:Catalog.Items.TabularSection"),
        ),
        (
            "cf:Report.Sales",
            "template.remove",
            json!({"values": {"name": "Print"}}),
            "ops[0].args.values.name",
            "cf:Report.Sales.Template.Print.Can.template.remove",
            Some("cf:Report.Sales.Template"),
        ),
        (
            "cf:Catalog.Items",
            "predefinedItem.set",
            json!({"values": {"id": "a1b2c3d4-1111-4111-8111-111111111111", "description": "x"}}),
            "ops[0].args.values.id",
            "cf:Catalog.Items.PredefinedItem",
            None,
        ),
    ] {
        let request = apply_request(at, op, args);
        let refusal = preflight_apply_operations(&request, traits, &|_| None)
            .expect_err("a retired argument refuses before planning");
        let value = serde_json::to_value(&*refusal).unwrap();
        assert_eq!(value["diagnostics"][0]["code"], "bad_value", "{value}");
        assert_eq!(value["diagnostics"][0]["outcome"], "fixCall", "{value}");
        assert_eq!(value["at"], location, "{value}");
        assert_eq!(refusal.next[0]["tool"], "unica.view", "{value}");
        assert_eq!(refusal.next[0]["args"], json!({"at": next_at}), "{value}");
        // Адрес из значения может не существовать: вторым шагом — ветвь
        // владельца, где видны имена членов этого вида.
        match branch {
            Some(branch) => {
                assert_eq!(refusal.next.len(), 2, "{value}");
                assert_eq!(refusal.next[1]["args"]["at"], branch, "{value}");
            }
            None => assert_eq!(refusal.next.len(), 1, "{value}"),
        }
    }
}
