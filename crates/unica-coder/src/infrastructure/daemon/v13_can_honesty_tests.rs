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
    ApplyRequest, OperationDescriptor, OperationFamily, OperationRegistry, SourceSetTraits,
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
        "attribute.add",
        "тип реквизита проверяется по видам типов владельца; пример не подходит всем",
    ),
    (
        "code.insert",
        "на адресе модуля вставка с `selector` требует существующий метод",
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
            "childSubsystem.add",
            "childSubsystem.remove",
            "code.insert",
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
    let runtime = V5CanonicalInvocationRuntime::new(
        Arc::new(CanonicalV13ReadService::default()),
        Arc::new(TokioClock),
    );
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
        "childSubsystem.remove" => json!({"items": [{"name": "Sample"}]}),
        "content.remove" => json!({"items": [{"object": "Catalog.Sample"}]}),
        "code.insert" | "code.replace" => json!({"at": "", "text": "// sample\n"}),
        "supportRule.set" => json!({"values": {"rule": "editable"}}),
        "supportCapability.set" => json!({"values": {"enabled": true}}),
        "props.set" => json!({"values": {"Comment": "sample"}}),
        "help.create" => json!({"values": {"lang": "ru"}}),
        "role.create" | "subsystem.create" => json!({"values": {"name": "Sample"}}),
        "object.create" => json!({"values": {"kind": "Catalog", "name": "Sample"}}),
        "object.remove" | "attribute.remove" | "form.remove" | "type.remove"
        | "event.implement" => json!({"at": ""}),
        "form.add" | "formAttribute.add" | "formCommand.add" | "element.add" | "command.add"
        | "tabularSection.add" | "childSubsystem.add" | "dimension.add" | "resource.add"
        | "enumValue.add" | "column.add" => json!({"items": [{"name": "Sample"}]}),
        "attribute.add" => json!({"items": [{"name": "Sample", "type": "String"}]}),
        "predefinedItem.add" => json!({"items": [{
            "id": "11111111-2222-4333-8444-555555555555",
            "name": "Sample"
        }]}),
        "predefinedItem.set" => json!({"values": {
            "id": "11111111-2222-4333-8444-555555555555",
            "description": "sample"
        }}),
        "predefinedItem.remove" => {
            json!({"values": {"id": "11111111-2222-4333-8444-555555555555"}})
        }
        "attribute.set" => json!({"values": {"comment": "sample"}}),
        "form.set" => json!({"values": {"defaultObjectForm": "Sample"}}),
        "command.set" | "tabularSection.set" | "template.set" | "dimension.set"
        | "resource.set" | "enumValue.set" | "column.set" => {
            json!({"values": {"name": "Sample", "comment": "sample"}})
        }
        "command.remove"
        | "tabularSection.remove"
        | "template.remove"
        | "dimension.remove"
        | "resource.remove"
        | "enumValue.remove"
        | "column.remove" => {
            json!({"values": {"name": "Sample"}})
        }
        "event.bind" => json!({"values": {"event": "OnOpen", "handler": "Sample"}}),
        "content.add" => json!({"items": [{"object": "Catalog.Sample"}]}),
        "right.set" => {
            json!({"values": {"object": "Catalog.Sample", "right": "Read", "value": true}})
        }
        "commandVisibility.set" => {
            json!({"items": [{"command": "Catalog.Sample.StandardCommand.OpenList", "visible": true}]})
        }
        "commandPlacement.set" => json!({"items": [{
            "command": "Catalog.Sample.StandardCommand.OpenList",
            "group": "NavigationPanelImportant"
        }]}),
        "commandOrder.set" => json!({"values": {
            "group": "NavigationPanelOrdinary",
            "commands": ["Catalog.Sample.StandardCommand.OpenList"]
        }}),
        "groupOrder.set" => json!({"values": {"groups": ["NavigationPanelImportant"]}}),
        "subsystemOrder.set" => json!({"values": {"subsystems": ["Subsystem.Sample"]}}),
        "valueType.add" => json!({"values": {"name": "Sample", "base": "xs:string"}}),
        "objectType.add" => json!({"values": {"name": "Sample"}}),
        "property.add" => json!({"values": {"property": {"name": "Sample", "type": "xs:string"}}}),
        "property.remove" => json!({"values": {"name": "Sample"}}),
        _ => serde_json::to_value(descriptor.copyable_skeleton()).unwrap()["args"].clone(),
    }
}

/// Аргументы на конкретном узле: удаление элемента на адресе элемента не
/// называет список, а на форме — называет.
fn node_args(descriptor: OperationDescriptor, at: &QualifiedAddress) -> Value {
    let item_address = at
        .segments()
        .last()
        .is_some_and(|segment| segment.kind() == NodeKind::Item);
    match descriptor.name() {
        "element.remove" if item_address => json!({"at": ""}),
        "element.remove" => json!({"items": [{"name": "Sample"}]}),
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
            // сверяется на каждой, а время теста остаётся в пределах.
            let per_pair = pairs.entry((route, terminal)).or_insert(0usize);
            let shape = shape_key(&address);
            if *per_pair < 2 && !representatives.contains_key(&shape) {
                *per_pair += 1;
                representatives.insert(shape, at);
            }
        }
        let mut planned_pairs = BTreeSet::new();
        for at in representatives.values() {
            let address = QualifiedAddress::parse(at).unwrap();
            let pair = (
                classify_route(&address, PlatformProfile::v8_3_27()),
                address.segments().last().unwrap().kind().as_str(),
            );
            // Планы дороги: файлы плана проверяются на одном узле пары
            // «маршрут, вид», словарь — на каждом представителе.
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
            (RouteClass::Form, "Attribute"),
            (RouteClass::Form, "Event"),
            (RouteClass::Module, "Event"),
            (RouteClass::Module, "Method"),
        ],
    ),
    (
        "workspace-dcs",
        &[RouteClass::Dcs],
        &[(RouteClass::Metadata, "Template")],
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
    ("workspace-predefined", &[RouteClass::Metadata], &[]),
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
    let viewed = invoke(
        checked.workspace,
        ToolIdentity::View,
        json!({"at": at, "filter": {"sections": ["can"]}}),
    );
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
        let expected = placed && !format_refused;
        if listed != expected {
            mismatches.push(format!(
                "{at}: `{op}` can={listed}, preflight={placed}, template format refused={format_refused}"
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
        for path in paths {
            let path = path.to_string_lossy().replace('\\', "/");
            if !(path.starts_with("Ext/") && path.ends_with(".bsl")) {
                mismatches.push(format!(
                    "{at}: `{op}` writes `{path}` outside the configuration module"
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
            "mxl.set",
            json!({"values": {"area": "Header", "cells": {"R1C1": "x"}}}),
            "cf:Report.Sales.Template.Print",
            true,
        ),
        (
            "cf:Catalog.Items.Form.Main.Item.Header",
            "element.add",
            json!({"items": [{"name": "Comment"}]}),
            "cf:Catalog.Items.Form.Main",
            false,
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
        assert_eq!(next["args"]["at"], target, "{value}");
        assert_eq!(
            next["args"]["filter"]["sections"],
            json!(["can"]),
            "{value}"
        );
        assert_eq!(
            next["args"]["filter"].get("can").is_some(),
            contract,
            "{value}"
        );
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
            "`Kind.Name.Form.F`",
        ),
        (
            "cf:Report.Sales.Template.Schema.Area.Header",
            "mxl.set",
            json!({"values": {"area": "Header", "cells": {"R1C1": "x"}}}),
            "cf:Report.Sales.Template.Schema.Area.Header",
            "of a spreadsheet",
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
        result.next[0]["args"]["at"], "main:Catalog.Валюты",
        "{value}"
    );
    assert_eq!(
        result.next[0]["args"]["filter"],
        json!({"sections": ["can"]}),
        "{value}"
    );
}
