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
use crate::domain::apply::ApplyRequest;
use crate::domain::apply::SourceSetWriteSupport;

use crate::domain::logical_route::classify_route;
use unica_bsl::profile::PlatformProfile;
use unica_execution::clock::TokioClock;
use unica_execution::invocation_store::ToolIdentity;
use unica_forms::{FormItemRole, FormObservation};
use unica_operations::OperationDescriptor;
use unica_operations::OperationFamily;
use unica_source_contract::route::RouteClass;
use unica_source_contract::{SourceFormat, SourceProfile, SourceSetKind};

use crate::infrastructure::daemon::protocol::InvocationRequest;
use crate::infrastructure::native_operations::apply_families::{
    parse_family_operation_for_test, plan_hidden_v13_apply,
};
use crate::infrastructure::source_workspace::WorkspaceActor;
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use unica_address::address::{NodeKind, QualifiedAddress};
use unica_execution_contract::invocation::DomainResult;
use unica_source_contract::workspace::WorkspaceContext;
use unica_work_control::cancellation::CancellationToken;
use unica_work_control::provider_deadline::ProviderDeadline;
use unica_workspace::actor::{ProviderRootBinding, WorkspaceIdentity, WorkspaceSourceSetInput};

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
        op: "code.replace",
        route: None,
        kind: "*",
        witnessed_in: "workspace",
        reason: "разбор замены с якорем проверяет аргументы; существование BSL-модуля \
                 по адресу проверяется при стадировании, а реестр отсекает чужой маршрут раньше",
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
/// метаданных. Общее имя двух семейств (`attribute.add` объекта и формы,
/// #1465) записывается с семейством — `Form:attribute.add`, — чтобы
/// проверка одной записи не прятала непроверенную другую.
const WRITES_CHECKED_AT_LEAST: &[(&str, &[&str])] = &[
    (
        "workspace",
        &[
            "Metadata:attribute.add",
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
            "item.add",
            "event.bind",
            "form.remove",
            "Form:attribute.add",
            "Form:command.add",
            "help.create",
            "objectType.add",
            "Properties:props.set",
            "Form:props.set",
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

fn acceptance_fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/acceptance")
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
    copy_tree(&acceptance_fixtures().join(name), &workspace);
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
    let actor =
        Arc::new(crate::infrastructure::source_workspace::new_actor(identity, context).unwrap());
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
///
/// Каждый `at` из `branches` и `items`, до которого дошёл обход, обязан
/// открываться тем же `view`: адрес, который сервер выдал и сам отверг,
/// попадает в `unroutable` вместе с причиной отказа (#1467).
struct Walk {
    nodes: Vec<String>,
    unroutable: Vec<String>,
}

fn walk(workspace: &Path, root: &str) -> Walk {
    let mut seen = BTreeSet::new();
    let mut shapes = BTreeMap::<String, usize>::new();
    let mut queue = VecDeque::from([root.to_string()]);
    let mut nodes = Vec::new();
    let mut unroutable = Vec::new();
    while let Some(at) = queue.pop_front() {
        if nodes.len() >= NODE_BUDGET || !seen.insert(at.clone()) {
            continue;
        }
        let address = match QualifiedAddress::parse(&at) {
            Ok(address) => address,
            Err(error) => {
                unroutable.push(format!("{at}: {error}"));
                continue;
            }
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
        if !result.ok {
            if !dangling_reference(workspace, &address, &result) {
                unroutable.push(format!("{at}: {}", result.summary));
            }
            continue;
        }
        let Some(data) = result.data else {
            continue;
        };
        // Узел, который `view` прочитал, `check` тоже принимает: выбор
        // валидатора по виду не должен отвечать `bad_value`/`fixCall` на
        // адрес, который сервер сам выдал (#1467). Отказ поставщика или
        // отсутствие движка — другой исход.
        let checked = invoke(workspace, ToolIdentity::Check, json!({"at": at}));
        if !checked.ok
            && (refusal_field(&checked, "code") == Some("bad_value")
                || refusal_field(&checked, "outcome") == Some("fixCall"))
        {
            unroutable.push(format!("{at}: check: {}", checked.summary));
        }
        *count += 1;
        nodes.push(at);
        // Коллекция вида в корне идёт в порядке `ChildObjects` (#2156). Обход
        // берёт её членов по имени: выбор представителей формы адреса не
        // должен зависеть от авторского порядка фикстуры.
        let root_collection = matches!(address.segments(), [only] if only.name().is_none());
        for slot in ["branches", "items"] {
            let mut children = data[slot]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|child| child.get("at").and_then(Value::as_str))
                .map(str::to_string)
                .collect::<Vec<_>>();
            if root_collection && slot == "items" {
                children.sort();
            }
            queue.extend(children);
        }
    }
    Walk { nodes, unroutable }
}

/// Код отказа ответа: первый диагноз несёт `code` и `outcome`.
fn refusal_field<'a>(result: &'a DomainResult, field: &str) -> Option<&'a str> {
    result
        .diagnostics
        .first()
        .and_then(|diagnostic| diagnostic.get(field))
        .and_then(Value::as_str)
}

/// Ссылка на объект, которого нет в исходниках: подсистема или командный
/// интерфейс урезанной фикстуры называют объект, не вошедший в неё. Такой
/// адрес разобран и маршрутизирован, а `not_found` отвечает отсутствием
/// корневого объекта. Отсутствие доказывает файловая система фикстуры —
/// нет файла описания корневого объекта, — а не проверяемый `view`.
fn dangling_reference(
    workspace: &Path,
    address: &QualifiedAddress,
    refusal: &DomainResult,
) -> bool {
    if refusal_field(refusal, "code") != Some("not_found") {
        return false;
    }
    let Some(root) = address.segments().first() else {
        return false;
    };
    let (Some(name), Some(layout)) = (
        root.name(),
        unica_source_designer::layout::metadata_kind(root.kind().as_str()),
    ) else {
        return false;
    };
    let descriptor = source_root(workspace, address.source_set())
        .join(layout.directory)
        .join(format!("{name}.xml"));
    !descriptor.exists()
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
    // Аргументы зависят только от операции, а контракт строится заново на
    // каждый вызов: обход спрашивает их на каждом узле. Ключ — семейство и
    // имя: у общего имени двух семейств (#1465) свои аргументы.
    thread_local! {
        static SAMPLES: std::cell::RefCell<BTreeMap<String, Value>> =
            const { std::cell::RefCell::new(BTreeMap::new()) };
    }
    SAMPLES.with(|samples| {
        samples
            .borrow_mut()
            .entry(write_key(descriptor))
            .or_insert_with(|| build_sample_args(descriptor))
            .clone()
    })
}

fn build_sample_args(descriptor: OperationDescriptor) -> Value {
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
        "form.add" | "attribute.add" | "command.add" | "item.add" | "childSubsystem.add" => {
            json!({"items": [{"name": "Sample"}]})
        }
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
fn node_args(
    descriptor: OperationDescriptor,
    at: &QualifiedAddress,
    traits: unica_source_contract::context::SourceSetTraits,
) -> Value {
    let method_address = at
        .segments()
        .last()
        .is_some_and(|segment| segment.kind() == NodeKind::Method);
    match descriptor.name() {
        "props.set" if descriptor.family() == OperationFamily::Properties => {
            // The source set selects the profile: an extension's adopted
            // language publishes the Comment-only example.
            crate::domain::apply::operation_site(at, traits)
                .and_then(|site| {
                    unica_metadata::descriptor_operations::PropertiesSite::from_site(&site)
                })
                .map(|site| {
                    unica_metadata::descriptor_operations::PropertiesOperation::Set
                        .contract(site)
                        .example_args
                })
                .unwrap_or_else(|| sample_args(descriptor))
        }
        "item.remove" => json!({"at": ""}),
        // Место вставки у метода — до или после него (#1354).
        "code.insert" if method_address => {
            json!({"at": "", "text": "// sample\n", "position": "after"})
        }
        "code.replace"
            if at
                .segments()
                .last()
                .is_some_and(|s| matches!(s.kind(), NodeKind::Method | NodeKind::Body)) =>
        {
            json!({"text":"// sample\n"})
        }
        // Слоты форм у каждого вида владельца свои (#2144): пример берётся
        // из контракта на этом адресе.
        "form.set" => {
            unica_forms::contracts::form_set(
                at.segments().first().map(|segment| segment.kind().as_str()),
            )
            .example_args
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
    runtime: V5CanonicalInvocationRuntime,
    workspace: &'a Path,
    actor: &'a WorkspaceActor,
    binding: &'a ProviderRootBinding,
    source_root: PathBuf,
    traits: unica_source_contract::context::SourceSetTraits,
}

/// Обход одной фикстуры: расхождения, достигнутые маршруты и пары
/// «маршрут, вид», встреченные сужения.
fn check_fixture(name: &str) -> (BTreeSet<RouteClass>, BTreeSet<(RouteClass, String)>) {
    let mut routes = BTreeSet::new();
    let mut covered = BTreeSet::new();
    let mut mismatches = Vec::new();
    let mut unroutable = Vec::new();
    let fixture = fixture(name);
    NARROWINGS_SEEN.with(|seen| seen.borrow_mut().clear());
    WRITES_CHECKED.with(|done| done.borrow_mut().clear());
    for (set, kind, binding) in &fixture.sets {
        let checked = Checked {
            runtime: V5CanonicalInvocationRuntime::new(
                Arc::new(CanonicalV13ReadService::default()),
                Arc::new(TokioClock),
            ),
            workspace: &fixture.workspace,
            actor: &fixture.actor,
            binding,
            source_root: source_root(&fixture.workspace, set),
            traits: crate::domain::apply::source_set_traits_of_source(
                *kind,
                SourceFormat::PlatformXml,
                SourceProfile::platform_xml_8_3_27_format_2_20(),
            ),
        };
        let mut representatives = BTreeMap::new();
        let mut pairs = BTreeMap::new();
        let walked = walk(&fixture.workspace, &format!("{set}:Configuration"));
        unroutable.extend(walked.unroutable);
        for at in walked.nodes {
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
    assert_eq!(
        unroutable,
        Vec::<String>::new(),
        "{name}: view published addresses it refuses"
    );
    assert_eq!(mismatches, Vec::<String>::new(), "{name}");
    // Проверка записи не пустая: план с правдоподобными аргументами
    // прошёл хотя бы для этих операций, и их файлы сверены с узлом.
    let writes_checked = WRITES_CHECKED.with(|done| done.borrow().clone());
    let required = WRITES_CHECKED_AT_LEAST
        .iter()
        .find(|(fixture, _)| *fixture == name)
        .map_or(&[][..], |(_, ops)| *ops);
    for op in required {
        let key = required_write_key(op);
        assert!(
            writes_checked.contains(&key),
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

/// rule: INV.WIRE.V13-CAN-DICTIONARY
#[test]
fn medium_honest_can_on_the_main_corpus_workspace() {
    check_fixture_coverage("workspace");
}

/// rule: INV.WIRE.V13-CAN-DICTIONARY
#[test]
fn medium_honest_can_on_the_dcs_workspace() {
    check_fixture_coverage("workspace-dcs");
}

/// rule: INV.WIRE.V13-CAN-DICTIONARY
#[test]
fn medium_honest_can_on_the_mxl_workspace() {
    check_fixture_coverage("workspace-mxl");
}

/// rule: INV.WIRE.V13-CAN-DICTIONARY
#[test]
fn medium_honest_can_on_the_code_and_extension_workspace() {
    check_fixture_coverage("workspace-code");
}

/// rule: INV.WIRE.V13-CAN-DICTIONARY
#[test]
fn medium_honest_can_on_the_predefined_workspace() {
    check_fixture_coverage("workspace-predefined");
}

#[test]
fn medium_honest_can_on_the_resolve_workspace() {
    check_fixture_coverage("workspace-resolve");
}

/// rule: INV.WIRE.V13-CAN-DICTIONARY
#[test]
fn medium_honest_can_on_external_sources() {
    check_fixture_coverage("workspace-external");
}

#[test]
fn form_collection_check_reports_readability_without_a_content_validator() {
    for (workspace, addresses) in [
        (
            "workspace-resolve",
            vec!["main:Catalog.Items.Form", "main:CommonForm"],
        ),
        (
            "workspace-external",
            vec![
                "epf:ExternalDataProcessor.Import.Form",
                "epf:ExternalDataProcessor.Other.Form",
                "erf:ExternalReport.Sales.Form",
            ],
        ),
    ] {
        let fixture = fixture(workspace);
        for at in addresses {
            let viewed = invoke(&fixture.workspace, ToolIdentity::View, json!({"at": at}));
            assert!(viewed.ok, "{at}: {}", viewed.summary);
            let checked = invoke(&fixture.workspace, ToolIdentity::Check, json!({"at": at}));
            assert!(checked.ok, "{at}: {}", checked.summary);
            let data = checked.data.expect("результат проверки коллекции");
            assert_eq!(data["status"], "readable", "{at}: {data}");
            assert_eq!(data["validators"], json!([]), "{at}: {data}");
            assert_eq!(data["diagnostics"], json!([]), "{at}: {data}");
        }
    }
}

/// Вместе фикстуры доходят до каждого маршрута, кроме названных пробелов, и
/// до видов, совпадающих по имени в разных деревьях.
/// rule: INV.WIRE.V13-CAN-DICTIONARY
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

/// Ключ проверенной записи — семейство и имя: общее имя двух семейств
/// (#1465) проверяется у каждой записи отдельно.
fn write_key(descriptor: OperationDescriptor) -> String {
    format!("{:?}:{}", descriptor.family(), descriptor.name())
}

/// Ключ строки `WRITES_CHECKED_AT_LEAST`: с семейством как есть, без него —
/// только для имени одной записи реестра.
fn required_write_key(entry: &str) -> String {
    if entry.contains(':') {
        return entry.to_string();
    }
    let mut named = crate::domain::apply::operation_registry().named(entry);
    let descriptor = named
        .next()
        .unwrap_or_else(|| panic!("`{entry}` is registered"));
    assert!(
        named.next().is_none(),
        "`{entry}` names two registry records: write it with its family"
    );
    write_key(descriptor)
}

thread_local! {
    static NARROWINGS_SEEN: std::cell::RefCell<BTreeSet<usize>> =
        const { std::cell::RefCell::new(BTreeSet::new()) };
    /// Операции, чьи файлы плана проверены: проверка записи не пустая.
    static WRITES_CHECKED: std::cell::RefCell<BTreeSet<String>> =
        const { std::cell::RefCell::new(BTreeSet::new()) };
}

fn check_node(checked: &Checked<'_>, at: &str, plan_writes: bool, mismatches: &mut Vec<String>) {
    let viewed = invoke_on(
        &checked.runtime,
        checked.workspace,
        ToolIdentity::View,
        json!({"at": at}),
    );
    assert!(viewed.ok, "{at}: {viewed:?}");
    let data = viewed.data.unwrap();
    // Every operation the node's dictionary names publishes its closed
    // detailed contract.
    for entry in data["can"].as_array().into_iter().flatten() {
        let op = entry["op"].as_str().unwrap();
        let detailed = invoke_on(
            &checked.runtime,
            checked.workspace,
            ToolIdentity::View,
            json!({"at":format!("{at}.Can.{op}")}),
        );
        assert!(detailed.ok, "{op} at {at} has no contract: {detailed:?}");
        let schema = &detailed.data.as_ref().unwrap()["props"]["contract"]["argsSchema"];
        assert!(schema.is_object(), "{at}.{op}: {schema}");
        assert_eq!(schema["additionalProperties"], false, "{at}.{op}: {schema}");
    }
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
    let form_observation = if unica_forms::applicability::is_form_attribute(address.segments()) {
        data["props"]["type"]
            .as_str()
            .and_then(FormObservation::from_attribute_type)
    } else if unica_forms::applicability::is_form_column(address.segments()) {
        data["props"]["parentType"]
            .as_str()
            .and_then(FormObservation::from_column_parent_type)
    } else {
        item_role.map(FormObservation::Item)
    };
    NARROWINGS_SEEN.with(|narrowings_seen| {
    for descriptor in crate::domain::apply::operation_registry().descriptors().iter().copied() {
        let op = descriptor.name();
        // Общее имя (`attribute.add` объекта и формы, #1465): вызов на этом
        // адресе исполняет запись, которую выбирает адрес, и сверяется она.
        let resolved = crate::domain::apply::operation_site(&address, checked.traits)
            .and_then(|site| crate::domain::apply::operation_registry().lookup_at(op, &site));
        if resolved.is_some_and(|resolved| resolved.family() != descriptor.family()) {
            continue;
        }
        let listed = can.contains(op);
        let request = apply_request(at, op, node_args(descriptor, &address, checked.traits));
        let preflight = super::v13_service::preflight_apply_operations_observed(
            &request, checked.traits, &|_| None,
            &|at| (at == &address).then_some(form_observation).flatten(),
            &|_| None,
        );
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
        let placed = crate::domain::apply::operation_site(&address, checked.traits)
            .is_some_and(|site| descriptor.applies_at(&site));
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
        let role_refused = placed && descriptor.observation_type().is::<FormObservation>()
            && !crate::domain::apply::operation_site(&address, checked.traits).is_some_and(|site| descriptor.applies_with_observations(&site, &crate::domain::apply::observation_lookup(&|_| None, &|_| form_observation)));
        if preflight.is_ok() != (placed && !role_refused) {
            mismatches.push(format!("{at}: `{op}` preflight={} differs from structural placement={placed} and observed form refusal={role_refused}", preflight.is_ok()));
        }
        if role_refused && plan(checked, &request).is_ok() {
            mismatches.push(format!(
                "{at}: `{op}` is filtered by the form observation {form_observation:?}, but the planner accepts it"
            ));
        }
        let expected = placed && !format_refused && !role_refused;
        if listed != expected {
            mismatches.push(format!(
                "{at}: `{op}` can={listed}, preflight={placed}, template format refused={format_refused}, item role refused={role_refused}"
            ));
            continue;
        }
        if listed && (family == Ok(true) || ARGUMENT_DEPENDENT.iter().any(|(name, _)| *name == op)) {
            check_unknown_argument_is_named(checked, at, descriptor, &address, mismatches);
        }
        if plan_writes && listed && family == Ok(true) {
            if let Ok(paths) = plan(checked, &request) {
                WRITES_CHECKED.with(|done| done.borrow_mut().insert(write_key(descriptor)));
                check_written_paths(checked, &address, route, op, &paths, mismatches);
            }
        }
    }
    });
}

/// Лишний аргумент операции словаря отвергается, и отказ называет его ключ
/// путём `ops[0].args.<ключ>` (INV.WIRE.V13-CAN-DICTIONARY,
/// INV.WIRE.OPERATION-TARGET-ADDRESS): по этому пути ответ `apply` ведёт
/// `next` к контракту операции на адресованном узле, а агент видит, какой
/// из аргументов убрать.
fn check_unknown_argument_is_named(
    checked: &Checked<'_>,
    at: &str,
    descriptor: OperationDescriptor,
    address: &QualifiedAddress,
    mismatches: &mut Vec<String>,
) {
    const KEY: &str = "unicaUnknownArgument";
    let op = descriptor.name();
    let mut args = node_args(descriptor, address, checked.traits);
    let Some(object) = args.as_object_mut() else {
        return;
    };
    object.insert(KEY.to_string(), json!(true));
    let request = apply_request(at, op, args);
    let refusal = match preflight_apply_operations(&request, checked.traits, &|_| None) {
        Err(refusal) => *refusal,
        Ok(_) => match parse_family_operation_for_test(0, &request.ops()[0], checked.binding) {
            // Разбор семейства пропустил ключ: отказать обязан планировщик.
            Ok(()) => match plan_error(checked, &request) {
                None => {
                    mismatches.push(format!("{at}: `{op}` accepts the unknown argument `{KEY}`"));
                    return;
                }
                Some(error) => error,
            },
            Err(error) => super::v13_service::apply_plan_error_result(&error),
        },
    };
    let expected = format!("ops[0].args.{KEY}");
    if refusal.at.as_deref() != Some(expected.as_str()) {
        mismatches.push(format!(
            "{at}: `{op}` refuses an unknown argument at {:?}, not `{expected}`: {}",
            refusal.at, refusal.diagnostics[0]
        ));
        return;
    }
    // Ответ `apply` ведёт к контракту операции на адресованном узле.
    let routed =
        super::v13_service::with_unknown_argument_contract(refusal, &request, checked.traits);
    // Без подробного контракта на узле лишний ключ не отличить от плохого
    // значения: такой отказ продолжения не получает.
    let Some(site) = crate::domain::apply::operation_site(address, checked.traits) else {
        return;
    };
    if descriptor.argument_contract_at(&site).is_none() {
        return;
    }
    let contract = format!("{at}.Can.{op}");
    if !routed
        .next
        .iter()
        .any(|next| next["args"]["at"] == contract.as_str())
    {
        mismatches.push(format!(
            "{at}: `{op}` refuses an unknown argument without `next` to `{contract}`: {:?}",
            routed.next
        ));
    }
}

/// Отказ планировщика на запросе, как его отдаёт `apply`; `None`, если план
/// построен.
fn plan_error(checked: &Checked<'_>, request: &ApplyRequest) -> Option<DomainResult> {
    let admission = checked
        .actor
        .admit_apply(
            checked.binding,
            None,
            true,
            ProviderDeadline::from_budget(Duration::from_secs(10)),
            &CancellationToken::new(),
        )
        .expect("the honesty workspace admits a plan");
    plan_hidden_v13_apply(request, checked.binding, &admission)
        .err()
        .map(|error| super::v13_service::apply_plan_error_result(&error))
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
    let Some(layout) = unica_source_designer::layout::metadata_kind(owner.kind().as_str()) else {
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
/// rule: INV.WIRE.V13-CAN-DICTIONARY
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
            "attribute.add",
            json!({"items": [{"name": "Comment"}]}),
            "cf:Catalog.Items.Form.Main",
            true,
        ),
        (
            "cf:Catalog.Items.Form.Main.Item.Header.Event.OnChange",
            "item.add",
            json!({"items": [{"name": "Comment"}]}),
            "cf:Catalog.Items.Form.Main.Item.Header",
            true,
        ),
        (
            "cf:Role.Reader",
            "role.create",
            json!({"values": {"name": "Reader"}}),
            "cf:Configuration",
            true,
        ),
    ] {
        let request = apply_request(at, op, args);
        let refusal = preflight_apply_operations(
            &request,
            crate::domain::apply::source_set_traits_for_read(
                SourceSetKind::Configuration,
                PlatformProfile::v8_3_27(),
            ),
            &|template| {
                (template.to_string() == "cf:Report.Sales.Template.Print")
                    .then_some(unica_templates::TemplateFormat::SpreadsheetDocument)
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
/// rule: INV.WIRE.V13-CAN-DICTIONARY
#[test]
fn without_an_executable_ancestor_the_refusal_names_the_address_shape() {
    let traits = crate::domain::apply::source_set_traits_for_read(
        SourceSetKind::Configuration,
        PlatformProfile::v8_3_27(),
    );
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
            "item.add",
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
                .then_some(unica_templates::TemplateFormat::DataCompositionSchema)
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
/// rule: INV.WIRE.OPERATION-CREATION-PARENT
#[test]
fn a_missing_target_routes_next_to_the_parent_dictionary() {
    use unica_source_contract::proposal::{ApplyPlanError, ApplyPlanErrorKind};
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
                    unica_refusal::RefusalCode::NotFound,
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
/// rule: INV.SOURCE.FORM-OPERATION-TARGETS
/// rule: INV.WIRE.V13-CAN-DICTIONARY
#[test]
fn honest_can_on_form_items_and_events() {
    let fixture = fixture("workspace");
    let form = "main:Catalog.Валюты.Form.ФормаЭлемента";
    let list = "main:Catalog.Валюты.Form.ФормаСписка";
    let form_ops = [
        "form.remove",
        "item.add",
        "item.remove",
        "attribute.add",
        "command.add",
        "event.bind",
        "event.implement",
    ];
    let cases: [(String, &[&str]); 8] = [
        (
            form.to_string(),
            &["form.remove", "item.add", "attribute.add", "command.add"],
        ),
        (
            format!("{form}.Item.ГруппаСлужебная"),
            &["item.add", "item.remove"],
        ),
        (format!("{form}.Item.ПолеНаименование"), &["item.remove"]),
        (format!("{form}.Item.ФормаКоманднаяПанель"), &["item.add"]),
        (
            format!("{list}.Item.List.Item.ListCommandBar"),
            &["item.add"],
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
        "item.add" => {
            json!({"items": [{"name": "Проба", "type": "Button", "stdCommand": "Close"}]})
        }
        "item.remove" | "form.remove" | "event.implement" => json!({"at": at}),
        "attribute.add" => json!({"items": [{"name": "Проба", "type": "String"}]}),
        "command.add" => json!({"items": [{"name": "Проба"}]}),
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
            // к ближайшему узлу, где `item.add` исполнима.
            if op == "item.add" && at.ends_with(".Item.ПолеНаименование") {
                assert_eq!(result.at.as_deref(), Some("ops[0].at"), "{result:?}");
                assert_eq!(
                    result.next[0]["args"]["at"],
                    format!("{form}.Can.item.add"),
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
            if *op == "item.add" {
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

/// Рабочее пространство с двумя наборами конфигурации: первичный `base` и
/// `sales` с документом и справочником корпуса — проверка того, что адрес
/// сохраняет непервичный набор исходников.
fn two_set_workspace() -> (tempfile::TempDir, PathBuf) {
    let root = tempfile::tempdir().unwrap();
    let workspace = std::fs::canonicalize(root.path()).unwrap().join("ws");
    let fixtures = acceptance_fixtures();
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

/// Срезы `Interface` модуля — элементы без адреса: `…Interface.Public`
/// грамматика отвергает, и `view` его больше не выдаёт. Каждый `at` ответа
/// открывается, а методы среза — адреса, которые `view` читает (#1467).
/// rule: CTR.SOURCE.QUALIFIED-LOGICAL-ADDRESS
#[test]
fn interface_slices_publish_only_addresses_view_opens() {
    let fixture = fixture("workspace");
    let at = "main:Catalog.Валюты.Module.Manager.Interface";
    let result = invoke(&fixture.workspace, ToolIdentity::View, json!({"at": at}));
    assert!(result.ok, "{}", result.summary);
    let data = result.data.expect("interface collection");
    let items = data["items"].as_array().expect("interface items");
    let names = items
        .iter()
        .map(|item| item["interface"].as_str().unwrap_or_default())
        .collect::<Vec<_>>();
    assert_eq!(names, ["Public", "Library", "Override"]);
    let mut opened = Vec::new();
    for item in items {
        assert_eq!(item["addressable"], json!(false), "{item}");
        if let Some(at) = item.get("at").and_then(Value::as_str) {
            opened.push(at.to_string());
        }
        for method in item["methods"].as_array().into_iter().flatten() {
            opened.push(method.as_str().expect("method address").to_string());
        }
    }
    assert!(
        opened.contains(&"main:Catalog.Валюты.Module.Manager.Method.ВалютаПоКоду".to_string()),
        "{opened:?}"
    );
    for at in opened {
        let view = invoke(&fixture.workspace, ToolIdentity::View, json!({"at": at}));
        assert!(view.ok, "{at}: {}", view.summary);
    }
    // `check` ветви не путает её с командным интерфейсом подсистемы: срезы
    // читаются, код модуля проверяет шаг `bsl` на самом модуле.
    let checked = invoke(&fixture.workspace, ToolIdentity::Check, json!({"at": at}));
    assert!(checked.ok, "{}", checked.summary);
    let data = checked.data.expect("check verdict");
    assert_eq!(data["status"], "readable", "{data}");
    assert_eq!(data["validators"], json!([]), "{data}");
    assert_eq!(
        walk(&fixture.workspace, at).unroutable,
        Vec::<String>::new()
    );
}

/// Role creation and rights use the same contracts that the productive
/// planner checks. Preview keeps both source sets unchanged.
/// rule: INV.WIRE.V13-CAN-DICTIONARY
#[test]
fn every_role_family_contract_builds_a_preview_without_source_writes() {
    let (_root, workspace) = two_set_workspace();
    let before = tree_digest(&workspace.join("src"));
    let runtime = V5CanonicalInvocationRuntime::new(
        Arc::new(CanonicalV13ReadService::default()),
        Arc::new(TokioClock),
    );
    for (at, operation, args) in [
        ("sales:Configuration", "role.create", None),
        (
            "sales:Role.SalesReader",
            "right.add",
            Some(
                json!({"items":[{"object":"Report.АнализВерсийОбъектов","rights":{"Read":true}}]}),
            ),
        ),
        (
            "sales:Role.SalesReader.Right.Catalog_Валюты",
            "right.set",
            Some(json!({"values":{"right":"View","value":false}})),
        ),
    ] {
        let detailed = invoke_on(
            &runtime,
            &workspace,
            ToolIdentity::View,
            json!({"at":format!("{at}.Can.{operation}")}),
        );
        assert!(
            detailed.ok,
            "{operation} must publish its contract at {at}: {detailed:?}"
        );
        let data = detailed.data.unwrap();
        assert_eq!(data["kind"], "Can");
        let contract = &data["props"]["contract"];
        assert_eq!(contract["argsSchema"]["additionalProperties"], false);
        let args = args.unwrap_or_else(|| contract["exampleArgs"].clone());
        assert!(args.get("at").is_none());
        let planned = invoke_on(
            &runtime,
            &workspace,
            ToolIdentity::Apply,
            json!({"at":at,"ops":[{"op":operation,"args":args}]}),
        );
        assert!(planned.ok, "{operation}: {planned:?}");
        assert!(
            planned
                .data
                .as_ref()
                .and_then(|data| data.get("executionToken"))
                .and_then(Value::as_str)
                .is_some_and(|token| !token.is_empty()),
            "{planned:?}"
        );
        assert_eq!(tree_digest(&workspace.join("src")), before);
    }
    let batch = invoke_on(
        &runtime,
        &workspace,
        ToolIdentity::Apply,
        json!({"at":"sales:Configuration","ops":[
            {"op":"role.create","args":{"values":{"name":"RoleCanBatch"}}},
            {"op":"right.add","args":{"at":"sales:Role.RoleCanBatch","items":[
                {"object":"Catalog.Валюты","rights":{"Read":true,"View":true}}
            ]}}
        ]}),
    );
    assert!(batch.ok, "{batch:?}");
    assert!(
        batch
            .data
            .as_ref()
            .and_then(|data| data["executionToken"].as_str())
            .is_some_and(|token| !token.is_empty()),
        "{batch:?}"
    );
    assert_eq!(tree_digest(&workspace.join("src")), before);
    assert!(!workspace.join("src/sales/Roles/ContractRole.xml").exists());
    assert!(!workspace.join("src/sales/Roles/RoleCanBatch.xml").exists());
    for at in [
        "sales:Can.role.create",
        "sales:Конфигурация.Can.role.create",
    ] {
        let detailed = invoke_on(&runtime, &workspace, ToolIdentity::View, json!({"at":at}));
        assert!(detailed.ok, "{at}: {detailed:?}");
        assert_eq!(
            detailed.data.unwrap()["at"],
            "sales:Configuration.Can.role.create"
        );
    }
}

/// The advertised Support operations publish a copyable contract and its
/// example builds a preview on the addressed node without source writes.
/// rule: INV.WIRE.V13-CAN-DICTIONARY
#[test]
fn support_can_contract_examples_preview_without_source_changes() {
    let fixture = fixture("workspace-support-form");
    let before = tree_digest(&fixture.workspace.join("src"));
    let runtime = V5CanonicalInvocationRuntime::new(
        Arc::new(CanonicalV13ReadService::default()),
        Arc::new(TokioClock),
    );
    for (at, operation) in [
        ("main:Configuration", "supportCapability.set"),
        ("main:Catalog.Items", "supportRule.set"),
        ("main:Catalog.Items.Form.Main", "supportRule.set"),
    ] {
        let owner = invoke_on(
            &runtime,
            &fixture.workspace,
            ToolIdentity::View,
            json!({"at":at}),
        );
        assert!(owner.ok, "{at}: {owner:?}");
        assert!(owner.data.as_ref().unwrap()["can"]
            .as_array()
            .unwrap()
            .iter()
            .any(|entry| entry["op"] == operation));
        let detailed = invoke_on(
            &runtime,
            &fixture.workspace,
            ToolIdentity::View,
            json!({"at":format!("{at}.Can.{operation}")}),
        );
        assert!(
            detailed.ok,
            "{operation} at {at} has no usable detailed contract: {detailed:?}"
        );
        let data = detailed.data.unwrap();
        assert_eq!(data["kind"], "Can");
        let contract = &data["props"]["contract"];
        assert_eq!(contract["argsSchema"]["additionalProperties"], false);
        let example = &contract["exampleArgs"];
        assert!(example["values"].is_object(), "{contract}");
        assert!(
            example.get("at").is_none(),
            "the operation address supplies its subject"
        );
        let planned = invoke_on(
            &runtime,
            &fixture.workspace,
            ToolIdentity::Apply,
            json!({"at":at,"ops":[{"op":operation,"args":example}]}),
        );
        assert!(
            planned.ok,
            "contract example must preview: {operation} at {at}: {planned:?}"
        );
        assert!(
            planned
                .data
                .as_ref()
                .and_then(|data| data.get("executionToken"))
                .and_then(Value::as_str)
                .is_some_and(|token| !token.is_empty()),
            "successful Support preview must supply a nonempty execution token: {planned:?}"
        );
        assert_eq!(
            tree_digest(&fixture.workspace.join("src")),
            before,
            "reading Can and preview never publish Support changes"
        );
    }
}

/// rule: INV.WIRE.V13-CAN-DICTIONARY
#[test]
fn code_contract_context_matrix_matches_staged_parser_and_previews_without_writes() {
    use unica_bsl::input::parse_code_plan_operation;
    let fixture = fixture("workspace-code");
    let binding = &fixture
        .sets
        .iter()
        .find(|(name, _, _)| name == "main")
        .unwrap()
        .2;
    let before = tree_digest(&fixture.workspace.join("src"));
    let module = "main:CommonModule.Пример";
    for (suffix, op, args, accepted) in [
        ("", "code.insert", json!({"text":"// sample\n"}), true),
        ("", "code.insert", json!({"text":"\n"}), true),
        ("", "code.insert", json!({"text":" "}), true),
        ("", "code.insert", json!({"text":""}), false),
        (
            "",
            "code.insert",
            json!({"text":"// sample\n","selector":{"anchor":"    // insertion point"},"position":"after"}),
            true,
        ),
        (".Body", "code.insert", json!({"text":"// sample\n"}), true),
        (
            ".Method.ОбработатьПример",
            "code.insert",
            json!({"text":"// sample\n","position":"before"}),
            true,
        ),
        (
            ".Method.ОбработатьПример",
            "code.insert",
            json!({"text":"// sample\n","position":"after"}),
            true,
        ),
        (
            "",
            "code.replace",
            json!({"text":"// replacement","selector":{"anchor":"    // insertion point"}}),
            true,
        ),
        (
            ".Body",
            "code.replace",
            json!({"text":"// replacement\n"}),
            true,
        ),
        (
            ".Method.ОбработатьПример",
            "code.replace",
            json!({"text":"Функция ОбработатьПример() Экспорт\nВозврат Ложь;\nКонецФункции\n"}),
            true,
        ),
        (
            ".Method.ОбработатьПример.Body",
            "code.replace",
            json!({"text":"Возврат Ложь;\n"}),
            true,
        ),
        (
            "",
            "code.insert",
            json!({"text":"x","position":"after"}),
            false,
        ),
        (
            "",
            "code.insert",
            json!({"text":"x","selector":{"anchor":"x"}}),
            false,
        ),
        ("", "code.replace", json!({"text":"x"}), false),
        (
            "",
            "code.replace",
            json!({"text":"x","selector":{"method":"ОбработатьПример"}}),
            false,
        ),
        (
            ".Body",
            "code.insert",
            json!({"text":"x","selector":{"anchor":"x"},"position":"after"}),
            false,
        ),
        (
            ".Method.ОбработатьПример",
            "code.insert",
            json!({"text":"x"}),
            false,
        ),
        (
            ".Method.ОбработатьПример.Body",
            "code.replace",
            json!({"text":"x","selector":{"anchor":"x"}}),
            false,
        ),
    ] {
        let at = format!("{module}{suffix}");
        let detailed = invoke(
            &fixture.workspace,
            ToolIdentity::View,
            json!({"at":format!("{at}.Can.{op}")}),
        );
        assert!(detailed.ok, "{at}: {detailed:?}");
        let address = QualifiedAddress::parse(&at).unwrap();
        let site = crate::domain::apply::operation_site(
            &address,
            crate::domain::apply::source_set_traits_for_read(
                SourceSetKind::Configuration,
                PlatformProfile::v8_3_27(),
            ),
        )
        .unwrap();
        let contract = crate::domain::apply::operation_registry()
            .lookup_at(op, &site)
            .unwrap()
            .argument_contract_at(&site)
            .unwrap();
        assert_eq!(
            contract.normalize(&args, "args").is_ok(),
            accepted,
            "schema {at} {op} {args}"
        );
        let mut parsed_args = args.clone();
        parsed_args["at"] = json!(at);
        assert_eq!(
            parse_code_plan_operation(op, &parsed_args, 0, binding.source_set_name()).is_ok(),
            accepted,
            "parser {at} {op} {args}"
        );
        let preview = invoke(
            &fixture.workspace,
            ToolIdentity::Apply,
            json!({"at":at,"ops":[{"op":op,"args":args}]}),
        );
        assert_eq!(preview.ok, accepted, "preview {at} {op}: {preview:?}");
        assert_eq!(tree_digest(&fixture.workspace.join("src")), before);
    }
    let unavailable = invoke(
        &fixture.workspace,
        ToolIdentity::View,
        json!({"at":format!("{module}.Method.ОбработатьПример.Body.Can.code.insert")}),
    );
    assert!(!unavailable.ok);
    // A batch override selects the contextual contract of its actual target.
    let preview = invoke(
        &fixture.workspace,
        ToolIdentity::Apply,
        json!({"at":module,"ops":[{"op":"code.replace","args":{"at":format!("{module}.Method.ОбработатьПример.Body"),"text":"Возврат Ложь;\n"}}]}),
    );
    assert!(preview.ok, "{preview:?}");
    let stale_module_args = invoke(
        &fixture.workspace,
        ToolIdentity::Apply,
        json!({"at":module,"ops":[{"op":"code.replace","args":{"at":format!("{module}.Method.ОбработатьПример.Body"),"text":"Возврат Ложь;\n","selector":{"anchor":"    // insertion point"}}}]}),
    );
    assert!(
        !stale_module_args.ok,
        "an override must use the actual target's argument shape"
    );
    assert_eq!(tree_digest(&fixture.workspace.join("src")), before);
}

/// rule: INV.WIRE.V13-CAN-DICTIONARY
#[test]
fn subsystem_interface_contracts_preview_all_operations_without_source_writes() {
    let fixture = fixture("workspace");
    let src = fixture.workspace.join("src");
    let command = "Catalog.Валюты.StandardCommand.OpenList";
    let group = "NavigationPanelImportant";
    std::fs::write(src.join("Subsystems/Администрирование/Ext/CommandInterface.xml"), format!(r#"<CommandInterface xmlns="http://v8.1c.ru/8.3/xcf/extrnprops" xmlns:xr="http://v8.1c.ru/8.3/xcf/readable" version="2.20"><CommandsVisibility><Command name="{command}"><Visibility><xr:Common>true</xr:Common></Visibility></Command></CommandsVisibility><CommandsPlacement><Command name="{command}"><CommandGroup>{group}</CommandGroup></Command></CommandsPlacement><CommandsOrder><Command name="{command}"><CommandGroup>{group}</CommandGroup></Command></CommandsOrder><GroupsOrder><Group>{group}</Group></GroupsOrder></CommandInterface>"#)).unwrap();
    std::fs::create_dir_all(src.join("Ext")).unwrap();
    std::fs::write(src.join("Ext/CommandInterface.xml"), r#"<CommandInterface xmlns="http://v8.1c.ru/8.3/xcf/extrnprops" version="2.20"><SubsystemsOrder><Subsystem>Subsystem.Администрирование</Subsystem></SubsystemsOrder></CommandInterface>"#).unwrap();
    let before = tree_digest(&src);
    let subsystem = "main:Subsystem.Администрирование";
    for (at, op, args) in [
        (
            "main:Configuration".to_string(),
            "subsystem.create",
            json!({"values":{"name":"NewSubsystem"}}),
        ),
        (
            subsystem.to_string(),
            "content.add",
            json!({"items":["Catalog.Валюты"]}),
        ),
        (
            subsystem.to_string(),
            "content.remove",
            json!({"items":["Catalog.Валюты"]}),
        ),
        (
            subsystem.to_string(),
            "childSubsystem.add",
            json!({"values":{"name":"NewChild"}}),
        ),
        (
            format!("{subsystem}.Subsystem.КонтрольРаботыПользователей"),
            "childSubsystem.remove",
            json!({}),
        ),
        (
            format!("{subsystem}.Interface"),
            "commandVisibility.set",
            json!({"items":[{"command":command,"visible":false}]}),
        ),
        (
            format!("{subsystem}.Interface"),
            "commandPlacement.set",
            json!({"items":[{"command":command,"group":group}]}),
        ),
        (
            format!("{subsystem}.Interface.Group.{group}"),
            "commandOrder.set",
            json!({"values":{"commands":[command]}}),
        ),
        (
            format!("{subsystem}.Interface"),
            "groupOrder.set",
            json!({"values":{"groups":[group]}}),
        ),
        (
            "main:Configuration".to_string(),
            "subsystemOrder.set",
            json!({"values":{"subsystems":["Subsystem.Администрирование"]}}),
        ),
    ] {
        let detailed = invoke(
            &fixture.workspace,
            ToolIdentity::View,
            json!({"at":format!("{at}.Can.{op}")}),
        );
        assert!(detailed.ok, "{at}.{op}: {detailed:?}");
        let preview = invoke(
            &fixture.workspace,
            ToolIdentity::Apply,
            json!({"at":at,"ops":[{"op":op,"args":args}]}),
        );
        assert!(preview.ok, "{at}.{op}: {preview:?}");
        assert_eq!(tree_digest(&src), before, "{op}");
    }
}

/// #2164: a client that always passes `limit` reads an operation contract
/// like a source node: a page size changes nothing, and only a continuation
/// stays refused: one node has no pages.
#[test]
fn a_contract_node_takes_a_page_size_like_any_single_node() {
    let fixture = fixture("workspace-code");
    let view = |args| invoke(&fixture.workspace, ToolIdentity::View, args);
    let contract = "main:Catalog.Пример.Can.props.set";
    let plain = view(json!({"at": contract}));
    assert!(plain.ok, "{plain:?}");
    for limit in [1, 50] {
        let limited = view(json!({"at": contract, "limit": limit}));
        assert!(limited.ok, "limit {limit}: {limited:?}");
        assert_eq!(limited.data, plain.data, "limit {limit}");
    }
    let continued = view(json!({"at": contract, "cursor": "opaque"}));
    assert!(!continued.ok, "{continued:?}");
    assert_eq!(
        continued.diagnostics[0]["code"], "bad_value",
        "{continued:?}"
    );
}

/// #2182: a node property is one value; a page size changes nothing, and
/// only a continuation stays refused.
#[test]
fn a_node_property_takes_a_page_size_like_any_single_node() {
    let fixture = fixture("workspace-code");
    let view = |args| invoke(&fixture.workspace, ToolIdentity::View, args);
    let property = "main:Catalog.Пример.Props.CodeLength";
    let plain = view(json!({"at": property}));
    assert!(plain.ok, "{plain:?}");
    assert_eq!(plain.data.as_ref().unwrap()["kind"], "Props", "{plain:?}");
    for limit in [1, 50] {
        let limited = view(json!({"at": property, "limit": limit}));
        assert!(limited.ok, "limit {limit}: {limited:?}");
        assert_eq!(limited.data, plain.data, "limit {limit}");
    }
    let continued = view(json!({"at": property, "cursor": "opaque"}));
    assert!(!continued.ok, "{continued:?}");
    assert_eq!(
        continued.diagnostics[0]["code"], "bad_value",
        "{continued:?}"
    );
}

#[test]
fn object_remove_skeleton_previews_with_only_the_outer_target() {
    let fixture = fixture("workspace-code");
    let at = "main:Catalog.Пример";
    let before = tree_digest(&fixture.workspace.join("src"));
    let viewed = invoke(&fixture.workspace, ToolIdentity::View, json!({"at":at}));
    assert!(viewed.ok, "{viewed:?}");
    let data = viewed.data.unwrap();
    let entry = data["can"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["op"] == "object.remove")
        .unwrap();
    assert!(entry.get("implemented").is_none(), "{entry}");
    let skeleton = serde_json::to_value(
        crate::domain::apply::operation_registry()
            .lookup("object.remove")
            .unwrap()
            .copyable_skeleton(),
    )
    .unwrap();
    assert!(skeleton["args"].get("at").is_none(), "{skeleton}");
    let preview = invoke(
        &fixture.workspace,
        ToolIdentity::Apply,
        json!({"at":at,"ops":[skeleton]}),
    );
    assert!(preview.ok, "{preview:?}");
    assert_eq!(tree_digest(&fixture.workspace.join("src")), before);
}

#[test]
fn column_type_uses_the_same_parent_fact_for_dictionary_and_preflight() {
    let at = "main:Catalog.Items.Form.Main.Attribute.Rows.Column.Value";
    let traits = crate::domain::apply::source_set_traits_for_read(
        SourceSetKind::Configuration,
        PlatformProfile::v8_3_27(),
    );
    for (parent_type, allowed) in [
        ("ValueTable", true),
        ("ValueTree", false),
        ("boolean", false),
        ("", false),
    ] {
        let mut data = json!({"at":at,"kind":"Column","props":{"parentType":parent_type}});
        super::v13_read_modes::attach_can(&mut data, traits);
        let request = apply_request(
            at,
            "props.set",
            json!({"values":{"type":"DefinedType.Existing | CatalogRef.Additional"}}),
        );
        let observed = FormObservation::from_column_parent_type(parent_type);
        let admitted = super::v13_service::preflight_apply_operations_observed(
            &request,
            traits,
            &|_| None,
            &|_| observed,
            &|_| None,
        );
        assert_eq!(admitted.is_ok(), allowed, "{parent_type}: {admitted:?}");
        assert_eq!(
            data["can"]
                .as_array()
                .unwrap()
                .iter()
                .any(|entry| entry["op"] == "props.set"),
            allowed
        );
    }
    let request = apply_request(
        "main:Catalog.Items.Form.Main",
        "props.set",
        json!({"values":{"Width":60}}),
    );
    assert!(super::v13_service::preflight_apply_operations_observed(
        &request,
        traits,
        &|_| None,
        &|_| None,
        &|_| None,
    )
    .is_ok());
}

#[test]
fn column_add_uses_the_same_attribute_fact_for_dictionary_and_preflight() {
    let at = "main:Catalog.Items.Form.Main.Attribute.Rows";
    let traits = crate::domain::apply::source_set_traits_for_read(
        SourceSetKind::Configuration,
        PlatformProfile::v8_3_27(),
    );
    for (type_name, allowed) in [
        ("ValueTable", true),
        ("ValueTree", false),
        ("boolean", false),
        ("", false),
    ] {
        let mut data = json!({"at":at,"kind":"Attribute","props":{"type":type_name}});
        super::v13_read_modes::attach_can(&mut data, traits);
        let request = apply_request(
            at,
            "column.add",
            json!({"items":[{"name":"New","type":"Boolean"}]}),
        );
        let observed = FormObservation::from_attribute_type(type_name);
        let admitted = super::v13_service::preflight_apply_operations_observed(
            &request,
            traits,
            &|_| None,
            &|_| observed,
            &|_| None,
        );
        assert_eq!(admitted.is_ok(), allowed, "{type_name}: {admitted:?}");
        assert_eq!(
            data["can"]
                .as_array()
                .unwrap()
                .iter()
                .any(|entry| entry["op"] == "column.add"),
            allowed
        );
    }
}

#[test]
fn column_add_accepts_a_valuetable_qname_alias_through_view_and_preflight() {
    let fixture = fixture("workspace-form-column-add");
    let path = fixture
        .workspace
        .join("src/Catalogs/Items/Forms/Main/Ext/Form.xml");
    let original = std::fs::read_to_string(&path).unwrap();
    let aliased = original
        .replace("xmlns:v8=", "xmlns:core=")
        .replace("v8:", "core:");
    std::fs::write(&path, aliased).unwrap();
    let before = tree_digest(&fixture.workspace.join("src"));
    let at = "main:Catalog.Items.Form.Main.Attribute.FirstRows";
    let viewed = invoke(&fixture.workspace, ToolIdentity::View, json!({"at":at}));
    assert!(viewed.ok, "{viewed:?}");
    let listed = viewed.data.as_ref().unwrap()["can"]
        .as_array()
        .unwrap()
        .iter()
        .any(|entry| entry["op"] == "column.add");
    let preview = invoke(
        &fixture.workspace,
        ToolIdentity::Apply,
        json!({"at":at,"ops":[{"op":"column.add","args":{"items":[{"name":"Aliased","type":"Boolean"}]}}]}),
    );
    assert!(
        listed && preview.ok,
        "listed={listed}, preview={preview:?}, view={viewed:?}"
    );
    assert_eq!(tree_digest(&fixture.workspace.join("src")), before);
}

#[test]
fn column_add_does_not_admit_an_alias_with_a_foreign_uri_or_another_parent_type() {
    for foreign_uri in [true, false] {
        let fixture = fixture("workspace-form-column-add");
        let path = fixture
            .workspace
            .join("src/Catalogs/Items/Forms/Main/Ext/Form.xml");
        let original = std::fs::read_to_string(&path).unwrap();
        let alias = original
            .replace("xmlns:v8=", "xmlns:core=")
            .replace("v8:", "core:");
        let invalid = if foreign_uri {
            alias.replace("http://v8.1c.ru/8.1/data/core", "urn:foreign")
        } else {
            alias.replace("core:ValueTable", "core:ValueTree")
        };
        std::fs::write(&path, invalid).unwrap();
        let before = tree_digest(&fixture.workspace.join("src"));
        let at = "main:Catalog.Items.Form.Main.Attribute.FirstRows";
        let viewed = invoke(&fixture.workspace, ToolIdentity::View, json!({"at":at}));
        assert!(viewed.ok, "{viewed:?}");
        assert!(
            !viewed.data.as_ref().unwrap()["can"]
                .as_array()
                .unwrap()
                .iter()
                .any(|entry| entry["op"] == "column.add"),
            "{viewed:?}"
        );
        let preview = invoke(
            &fixture.workspace,
            ToolIdentity::Apply,
            json!({"at":at,"ops":[{"op":"column.add","args":{"items":[{"name":"Rejected","type":"Boolean"}]}}]}),
        );
        assert!(!preview.ok, "{preview:?}");
        assert_eq!(tree_digest(&fixture.workspace.join("src")), before);
    }
}

#[test]
fn column_add_does_not_advertise_an_unproven_literal_valuetable() {
    for damage in ["foreign_uri", "extra_member", "nested_member"] {
        let fixture = fixture("workspace-form-column-add");
        let path = fixture
            .workspace
            .join("src/Catalogs/Items/Forms/Main/Ext/Form.xml");
        let original = std::fs::read_to_string(&path).unwrap();
        let invalid = match damage {
            "foreign_uri" => original.replace("http://v8.1c.ru/8.1/data/core", "urn:foreign"),
            "extra_member" => original.replace(
                "<v8:Type>v8:ValueTable</v8:Type>",
                "<v8:Type>v8:ValueTable</v8:Type><v8:Unknown/>",
            ),
            _ => original.replace(
                "<v8:Type>v8:ValueTable</v8:Type>",
                "<v8:Type>v8:ValueTable<v8:Nested/></v8:Type>",
            ),
        };
        std::fs::write(&path, invalid).unwrap();
        let before = tree_digest(&fixture.workspace.join("src"));
        let at = "main:Catalog.Items.Form.Main.Attribute.FirstRows";
        let viewed = invoke(&fixture.workspace, ToolIdentity::View, json!({"at":at}));
        assert!(viewed.ok, "{viewed:?}");
        let listed = viewed.data.as_ref().unwrap()["can"]
            .as_array()
            .unwrap()
            .iter()
            .any(|entry| entry["op"] == "column.add");
        let preview = invoke(
            &fixture.workspace,
            ToolIdentity::Apply,
            json!({"at":at,"ops":[{"op":"column.add","args":{"items":[{"name":"Rejected","type":"Boolean"}]}}]}),
        );
        assert!(
            !listed && !preview.ok,
            "{damage}: listed={listed}, view={viewed:?}, preview={preview:?}"
        );
        assert_eq!(tree_digest(&fixture.workspace.join("src")), before);
    }
}

#[test]
fn column_add_default_qname_keeps_read_facts_and_preserves_final_validation() {
    let fixture = fixture("workspace-form-column-add");
    let path = fixture
        .workspace
        .join("src/Catalogs/Items/Forms/Main/Ext/Form.xml");
    let original = std::fs::read_to_string(&path).unwrap();
    let equivalent = original.replace(
        "<v8:Type>v8:ValueTable</v8:Type>",
        "<Type xmlns=\"http://v8.1c.ru/8.1/data/core\">ValueTable</Type>",
    );
    std::fs::write(&path, equivalent).unwrap();
    let before = tree_digest(&fixture.workspace.join("src"));
    let at = "main:Catalog.Items.Form.Main.Attribute.FirstRows";
    let viewed = invoke(&fixture.workspace, ToolIdentity::View, json!({"at":at}));
    assert!(viewed.ok, "{viewed:?}");
    let listed = viewed.data.as_ref().unwrap()["can"]
        .as_array()
        .unwrap()
        .iter()
        .any(|entry| entry["op"] == "column.add");
    let columns = invoke(
        &fixture.workspace,
        ToolIdentity::View,
        json!({"at":format!("{at}.Column")}),
    );
    let preview = invoke(
        &fixture.workspace,
        ToolIdentity::Apply,
        json!({"at":at,"ops":[{"op":"column.add","args":{"items":[{"name":"Additional","type":"Boolean"}]}}]}),
    );
    assert!(
        listed && columns.ok,
        "listed={listed}, columns={columns:?}, view={viewed:?}"
    );
    // QName identity admits the operation; the existing whole-form validator
    // still rejects this source spelling before issuing a publication token.
    assert!(!preview.ok, "{preview:?}");
    assert_eq!(preview.diagnostics[0]["code"], "postcondition_failed");
    assert!(
        preview
            .summary
            .contains("bare type without namespace prefix"),
        "{preview:?}"
    );
    assert!(
        preview.data.is_none(),
        "no execution token on refusal: {preview:?}"
    );
    assert_eq!(viewed.data.as_ref().unwrap()["props"]["type"], "ValueTable");
    let rows = &columns.data.as_ref().unwrap()["items"];
    assert_eq!(rows.as_array().unwrap().len(), 1);
    assert_eq!(rows[0]["at"], format!("{at}.Column.Original"));
    assert_eq!(rows[0]["props"]["type"], "CatalogRef.Existing");
    assert_eq!(tree_digest(&fixture.workspace.join("src")), before);
}

#[test]
fn column_add_refuses_a_default_valuetable_qname_outside_the_core_namespace() {
    for namespace in ["urn:foreign", "http://v8.1c.ru/8.3/xcf/logform"] {
        let fixture = fixture("workspace-form-column-add");
        let path = fixture
            .workspace
            .join("src/Catalogs/Items/Forms/Main/Ext/Form.xml");
        let original = std::fs::read_to_string(&path).unwrap();
        let invalid = original.replace(
            "<v8:Type>v8:ValueTable</v8:Type>",
            &format!("<v8:Type xmlns=\"{namespace}\">ValueTable</v8:Type>"),
        );
        std::fs::write(&path, invalid).unwrap();
        let before = tree_digest(&fixture.workspace.join("src"));
        let at = "main:Catalog.Items.Form.Main.Attribute.FirstRows";
        let viewed = invoke(&fixture.workspace, ToolIdentity::View, json!({"at":at}));
        assert!(viewed.ok, "{viewed:?}");
        assert!(
            !viewed.data.as_ref().unwrap()["can"]
                .as_array()
                .unwrap()
                .iter()
                .any(|entry| entry["op"] == "column.add"),
            "{viewed:?}"
        );
        let preview = invoke(
            &fixture.workspace,
            ToolIdentity::Apply,
            json!({"at":at,"ops":[{"op":"column.add","args":{"items":[{"name":"Rejected","type":"Boolean"}]}}]}),
        );
        assert!(!preview.ok, "{preview:?}");
        assert_eq!(tree_digest(&fixture.workspace.join("src")), before);
    }
}
