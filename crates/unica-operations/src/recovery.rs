//! Neutral recovery records, ordered expansion and candidate admission.
use crate::{ObservationLookup, OperationDescriptor};
use serde_json::{Map, Value};
use unica_address::address::{escape_name, AddressSegment, NodeKind, QualifiedAddress};
use unica_source_contract::context::{OperationSite, RegistryContext};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RetiredArgument {
    pub op: &'static str,
    pub key: &'static str,
    pub address: &'static str,
    /// Операция переехала на адрес, и снятый ключ получает отказ.
    pub retired: bool,
}

/// Отказ по снятому аргументу: где он записан и куда перенести вызов.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArgumentRedirect {
    /// Путь снятого ключа внутри `args`, с номером элемента массива.
    pub location: String,
    /// Адрес, на котором операция исполнима, или — при `listing` —
    /// коллекция, где подузел виден по имени.
    pub at: QualifiedAddress,
    /// Значение ключа не было именем подузла: операция исполнима на
    /// `<at>.<Имя>`, а имя читается в коллекции `at`.
    pub listing: bool,
    /// Значение называет несколько узлов: `at` — их ветвь, здесь — адреса
    /// узлов, где операция исполнима. Пусто — значение назвало один узел.
    pub candidates: Vec<QualifiedAddress>,
    /// Узлов с таким именем несколько: `at` — ветвь, где они видны.
    pub ambiguous: bool,
}

/// Подстановка цепочки имён: значение — путь, где неэкранированная точка
/// разделяет имена, а `\.` — буквальная точка имени (прежний
/// `propertyPath` XDTO). Каждая точка-разделитель становится переходом к
/// члену того же вида, что стоит перед подстановкой: `{at}.Property.{path}`
/// со значением `A\.B.C` — `{at}.Property.A\.B.Property.C`.
const PATH_PLACEHOLDER: &str = "{path}";

/// Путь из значения в записи адреса; `None` — путь пуст, содержит пустое имя
/// или экранирование, которого грамматика адреса не знает.
fn escaped_path(value: &str, kind: &str) -> Option<String> {
    let mut names = vec![String::new()];
    let mut characters = value.chars();
    while let Some(character) = characters.next() {
        match character {
            '\\' => {
                characters.next().filter(|next| *next == '.')?;
                names.last_mut()?.push('.');
            }
            '.' => names.push(String::new()),
            other => names.last_mut()?.push(other),
        }
    }
    if names.iter().any(String::is_empty) {
        return None;
    }
    Some(
        names
            .iter()
            .map(|name| escape_name(name).into_owned())
            .collect::<Vec<_>>()
            .join(&format!(".{kind}.")),
    )
}

/// Подстановка в шаблон адреса для записи, чей ключ называл подузел не
/// именем: проверяет, что операция исполнима на члене коллекции.
const LISTED_MEMBER: &str = "Member";

impl RetiredArgument {
    /// Адрес, который строит запись для вызова на `at` с `args`; `None`, если
    /// снятого ключа в вызове нет или значение не даёт адреса.
    /// То же, что `redirect`, с поиском набора схемы и элемента формы по имени.
    fn redirect_with(
        self,
        descriptor: OperationDescriptor,
        at: &QualifiedAddress,
        args: &Map<String, Value>,
        members: MemberPathLookup<'_>,
    ) -> Option<ArgumentRedirect> {
        let (location, value) = find_key(&Value::Object(args.clone()), self.key, String::new())?;
        // Шаблон подставляется строкой и разбирается заново; имя из значения
        // проходит `escape_name`, поэтому точка в нём остаётся внутри имени.
        // Событие и владелец события подставляются до поиска узла: суффикс
        // шаблона за найденным узлом уже не содержит подстановок.
        let prepared = descriptor.prepare_redirect(self.address, at, args, &value, members)?;
        let template = prepared.template;
        let named = prepared.named;
        let raw = match named {
            Some((placeholder, NamedNode::Found(node))) => {
                template.replace(placeholder, &node.to_string())
            }
            Some((placeholder, NamedNode::Ambiguous { branch, candidates })) => {
                let suffix = template.split_once(placeholder)?.1;
                let candidates = candidates
                    .iter()
                    .map(|candidate| QualifiedAddress::parse(&format!("{candidate}{suffix}")))
                    .collect::<Result<Vec<_>, _>>()
                    .ok()?;
                return Some(ArgumentRedirect {
                    location,
                    at: branch,
                    listing: false,
                    candidates,
                    ambiguous: true,
                });
            }
            None => expand_anchor(&template, at)?.replace("{at}", &at.to_string()),
        };
        let raw = match raw.find(PATH_PLACEHOLDER) {
            Some(index) => {
                let separator = raw[..index].rsplit('.').nth(1)?;
                raw.replace(PATH_PLACEHOLDER, &escaped_path(&value, separator)?)
            }
            None => raw,
        };
        if let Some(collection) = raw.strip_suffix(".{name}") {
            return Some(ArgumentRedirect {
                location,
                at: QualifiedAddress::parse(&collection.replace("{value}", &escape_name(&value)))
                    .ok()?,
                listing: true,
                candidates: Vec::new(),
                ambiguous: false,
            });
        }
        let target = QualifiedAddress::parse(&raw.replace("{value}", &escape_name(&value))).ok()?;
        Some(ArgumentRedirect {
            location,
            at: target,
            listing: false,
            candidates: Vec::new(),
            ambiguous: false,
        })
    }
}

impl ArgumentRedirect {
    /// Адрес, на котором операция исполнима: для записи с `listing` — член
    /// коллекции с подставленным именем.
    fn executable_at(&self) -> Option<QualifiedAddress> {
        if let Some(candidate) = self.candidates.first() {
            return Some(candidate.clone());
        }
        if !self.listing && !self.ambiguous {
            return Some(self.at.clone());
        }
        QualifiedAddress::parse(&format!("{}.{LISTED_MEMBER}", self.at)).ok()
    }
}

/// Поиск узла по имени под контейнером. Контейнер — последний сегмент
/// адреса: макет (набор данных, в том числе член объединения), вариант
/// настроек (именованный элемент структуры) или форма (элемент формы).
pub type MemberPathLookup<'a> = &'a dyn Fn(&QualifiedAddress, &str) -> NamedLookup;

/// Ответ поиска узла по имени: путь имён от контейнера (`["A", "B"]` —
/// `DataSet.A.DataSet.B` под макетом или `Item.A.Item.B` под формой
/// и вариантом настроек).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NamedLookup {
    /// Узел с таким именем один.
    Found(Vec<String>),
    /// Такого узла нет или тело контейнера не читается.
    Missing,
    /// Узлов с таким именем несколько; пути тех, что адресуемы.
    Ambiguous(Vec<Vec<String>>),
}

/// Поиск, который ничего не знает о телах контейнеров.
pub fn no_member_lookup(_: &QualifiedAddress, _: &str) -> NamedLookup {
    NamedLookup::Missing
}

/// Узел, который назвало значение снятого аргумента.
pub enum NamedNode {
    Found(QualifiedAddress),
    /// Имя повторяется: ветвь узлов этого вида и адреса совпавших.
    Ambiguous {
        branch: QualifiedAddress,
        candidates: Vec<QualifiedAddress>,
    },
}

/// Адрес владельца с путём имён: `{owner}.Kind.A.Kind.B`.
fn owner_path(owner: &QualifiedAddress, kind: &str, path: &[String]) -> Option<QualifiedAddress> {
    QualifiedAddress::parse(&format!(
        "{owner}{}",
        path.iter()
            .map(|name| format!(".{kind}.{}", escape_name(name)))
            .collect::<String>()
    ))
    .ok()
}

/// Начало адреса до последнего сегмента, вид которого подходит `kind`,
/// включительно.
pub fn prefix_through(
    at: &QualifiedAddress,
    kind: impl Fn(NodeKind) -> bool,
) -> Option<QualifiedAddress> {
    let length = at
        .segments()
        .iter()
        .rposition(|segment| kind(segment.kind()))?
        + 1;
    Some(QualifiedAddress {
        line_range: None,
        source_set: at.source_set().to_string(),
        segments: at.segments()[..length].to_vec(),
    })
}

/// Найденное по имени: один узел, несколько или ни одного (тогда — узел
/// верхнего уровня с этим именем, адрес которого `view` честно не найдёт).
pub fn resolve_named(
    owner: &QualifiedAddress,
    kind: &str,
    name: &str,
    lookup: MemberPathLookup<'_>,
) -> Option<NamedNode> {
    match lookup(owner, name) {
        NamedLookup::Found(path) => owner_path(owner, kind, &path).map(NamedNode::Found),
        NamedLookup::Missing => owner_path(owner, kind, &[name.to_string()]).map(NamedNode::Found),
        NamedLookup::Ambiguous(paths) => Some(NamedNode::Ambiguous {
            branch: QualifiedAddress::parse(&format!("{owner}.{kind}")).ok()?,
            candidates: paths
                .iter()
                .map(|path| owner_path(owner, kind, path))
                .collect::<Option<Vec<_>>>()?,
        }),
    }
}

/// Подставляет `{at^Kind}`: начало `at` до последнего сегмента вида `Kind`
/// включительно. `None` — такого сегмента в адресе нет.
fn expand_anchor(template: &str, at: &QualifiedAddress) -> Option<String> {
    let Some(start) = template.find("{at^") else {
        return Some(template.to_string());
    };
    let rest = &template[start + "{at^".len()..];
    let end = rest.find('}')?;
    let kind = &rest[..end];
    let matches = |segment: &AddressSegment| match kind {
        "Module" => matches!(segment.kind(), NodeKind::Module | NodeKind::CommonModule),
        kind => segment.kind().as_str() == kind,
    };
    let length = at.segments().iter().rposition(matches)? + 1;
    let anchor = QualifiedAddress {
        line_range: None,
        source_set: at.source_set().to_string(),
        segments: at.segments()[..length].to_vec(),
    };
    Some(format!(
        "{}{anchor}{}",
        &template[..start],
        &rest[end + 1..]
    ))
}

/// Первое строковое значение по пути `key` и его точное место.
fn find_key(value: &Value, key: &str, prefix: String) -> Option<(String, String)> {
    let (head, rest) = match key.split_once('.') {
        Some((head, rest)) => (head, Some(rest)),
        None => (key, None),
    };
    let join = |name: &str| {
        if prefix.is_empty() {
            name.to_string()
        } else {
            format!("{prefix}.{name}")
        }
    };
    if let Some(name) = head.strip_suffix("[]") {
        let items = value.get(name)?.as_array()?;
        // `items[]` без продолжения — сам элемент массива строкой.
        let Some(rest) = rest else {
            return items.iter().enumerate().find_map(|(index, item)| {
                item.as_str()
                    .filter(|text| !text.is_empty())
                    .map(|text| (format!("{}[{index}]", join(name)), text.to_string()))
            });
        };
        return items
            .iter()
            .enumerate()
            .find_map(|(index, item)| find_key(item, rest, format!("{}[{index}]", join(name))));
    }
    let child = value.get(head)?;
    match rest {
        Some(rest) => find_key(child, rest, join(head)),
        None => child
            .as_str()
            .filter(|text| !text.is_empty())
            .map(|text| (join(head), text.to_string())),
    }
}

/// Отказ по снятому аргументу из таблицы `entries`: запись операции, которая
/// переехала, чей ключ передан и по чьему адресу операция исполнима.
pub fn retired_argument_redirect_in(
    entries: &[RetiredArgument],
    descriptor: OperationDescriptor,
    site: &OperationSite<'_>,
    context: RegistryContext,
    args: &Map<String, Value>,
    members: MemberPathLookup<'_>,
) -> Option<ArgumentRedirect> {
    entries
        .iter()
        .filter(|entry| entry.retired && entry.op == descriptor.name())
        .filter_map(|entry| entry.redirect_with(descriptor, site.at(), args, members))
        .find(|redirect| {
            redirect.executable_at().is_some_and(|target| {
                context
                    .site(&target, site.source())
                    .is_some_and(|target| descriptor.applies_at(&target))
            })
        })
}

/// Family-specific existing placeholders are resolved by the same operation
/// that owns the row. The generic expander does not name those families.
pub struct PreparedRedirect {
    pub template: String,
    pub named: Option<(&'static str, NamedNode)>,
}
impl PreparedRedirect {
    pub fn literal(template: &str) -> Self {
        Self {
            template: template.to_string(),
            named: None,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct BoundRetiredArgument {
    pub argument: RetiredArgument,
    descriptor: OperationDescriptor,
}
impl std::ops::Deref for BoundRetiredArgument {
    type Target = RetiredArgument;
    fn deref(&self) -> &Self::Target {
        &self.argument
    }
}
impl RetiredArgument {
    pub const fn bind(self, descriptor: OperationDescriptor) -> BoundRetiredArgument {
        BoundRetiredArgument {
            argument: self,
            descriptor,
        }
    }
}
impl BoundRetiredArgument {
    pub fn redirect(
        self,
        at: &QualifiedAddress,
        args: &Map<String, Value>,
    ) -> Option<ArgumentRedirect> {
        self.redirect_with(at, args, &no_member_lookup)
    }
    pub fn redirect_with(
        self,
        at: &QualifiedAddress,
        args: &Map<String, Value>,
        members: MemberPathLookup<'_>,
    ) -> Option<ArgumentRedirect> {
        self.argument
            .redirect_with(self.descriptor, at, args, members)
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RetiredOperation {
    pub op: &'static str,
    replacement: OperationDescriptor,
    /// Вид узла, который снятая операция создавала по своему адресу;
    /// `None` — операция переименована и исполнима на том же адресе.
    pub creates: Option<NodeKind>,
    /// Как аргументы прежнего вызова выражаются в заменившей операции.
    pub arguments: &'static str,
}

impl RetiredOperation {
    pub const fn new(
        op: &'static str,
        replacement: OperationDescriptor,
        creates: Option<NodeKind>,
        arguments: &'static str,
    ) -> Self {
        Self {
            op,
            replacement,
            creates,
            arguments,
        }
    }
    pub fn replacement(self) -> &'static str {
        self.replacement.name()
    }
    pub const fn descriptor(self) -> Option<OperationDescriptor> {
        Some(self.replacement)
    }

    /// Узел, на котором исполнима заменившая операция: родитель узла вида
    /// `creates` в адресе вызова, иначе сам адрес вызова.
    pub fn replacement_at(self, at: &QualifiedAddress) -> QualifiedAddress {
        let Some(creates) = self.creates else {
            return at.clone();
        };
        match at
            .segments()
            .iter()
            .rposition(|segment| segment.kind() == creates)
        {
            Some(index) if index > 0 => QualifiedAddress {
                line_range: None,
                source_set: at.source_set().to_string(),
                segments: at.segments()[..index].to_vec(),
            },
            _ => at.clone(),
        }
    }

    /// Ближайший узел, где исполнима заменившая операция. Создающая
    /// операция исполнима на родителе созданного узла; переименованная —
    /// на самом адресе вызова, если место и роль элемента её принимают,
    /// иначе на ближайшем предке, как в отказе «не тот узел». `None` —
    /// такого узла в цепочке адреса нет.
    pub fn replacement_with_observations(
        self,
        site: &OperationSite<'_>,
        context: RegistryContext,
        observations: ObservationLookup<'_>,
    ) -> Option<QualifiedAddress> {
        if self.creates.is_some() {
            return Some(self.replacement_at(site.at()));
        }
        let replacement = self.descriptor()?;
        let placed = replacement.applies_at(site);
        let here = placed && replacement.admits_node(site, observations);
        if here {
            return Some(site.at().clone());
        }
        // Операция над самим узлом (`item.remove`), которую его роль не
        // принимает: автоматическую командную панель отдельно не удалить, и
        // другой узел — например, её таблица — вместо неё не подставляется.
        if placed && replacement.skeleton_key() == "at" {
            return None;
        }
        replacement.nearest_ancestor_with_observations(site, context, observations)
    }
}
