//! Объявления видов узлов логического адреса.
//!
//! Каждый вид объявлен один раз: файл `kinds/<вид>.rs` с константой `KIND`.
//! `build.rs` собирает по именам файлов перечисление [`NodeKind`] и таблицу
//! объявлений, а списки видов строятся из неё: написания и коллекции адреса,
//! раскладка выгрузки, место в `ChildObjects` конфигурации и в дереве
//! Конфигуратора, линия появления, порождённые типы и порядок дочерних.
//! Новый вид — новый файл; общий список правкой строки не меняется, и два
//! изменения с разными видами не пересекаются текстуально.
//!
//! Имя файла — имя варианта в `snake_case`: `xdto_package.rs` даёт
//! `NodeKind::XdtoPackage`. Ведущие `//!` файла становятся документацией
//! варианта. Каноническое написание задаёт `KindDeclaration::new`.

include!(concat!(env!("OUT_DIR"), "/kinds.rs"));

declare_kind_modules!();

/// Всё, что Unica знает о виде как о целом.
#[derive(Debug, Clone, Copy)]
pub struct KindDeclaration {
    /// Каноническое английское написание: сегмент адреса и тег XML.
    pub tag: &'static str,
    /// Русские написания сегмента, которые разбор адреса принимает наравне
    /// с каноническим.
    pub russian: &'static [&'static str],
    /// Носит ли узел вида прикладное имя; см. [`NodeKind::takes_name`].
    pub named: bool,
    /// Вид объекта метаданных с адресом исходников: `<Вид>.<Имя>`.
    pub metadata: Option<MetadataDeclaration>,
    /// Место вида в корне конфигурации.
    pub root: Option<RootPlace>,
    /// Раскладка выгрузки: каталог и профиль 8.3.27.
    pub layout: Option<LayoutDeclaration>,
    /// Порождённые типы 8.3.27 (`InternalInfo`) с их категориями.
    pub generated_types_8_3_27: Option<&'static [(&'static str, &'static str)]>,
    /// Линия платформы, с которой вид есть, если позже первой описанной.
    pub since: Option<[u32; 3]>,
    /// Порядок, в котором выгрузка пишет виды дочерних объекта верхнего
    /// уровня его `ChildObjects`. `None` оставляет размещение вызывающему.
    pub child_order: Option<&'static [&'static str]>,
}

impl KindDeclaration {
    pub const fn new(tag: &'static str) -> Self {
        Self {
            tag,
            russian: &[],
            named: true,
            metadata: None,
            root: None,
            layout: None,
            generated_types_8_3_27: None,
            since: None,
            child_order: None,
        }
    }
}

/// Вид объекта метаданных в адресе исходников.
#[derive(Debug, Clone, Copy)]
pub struct MetadataDeclaration {
    /// Имена коллекции: английское первым, затем русские.
    pub collections: &'static [&'static str],
    /// Менеджер вида — свойство глобального контекста: `Справочники`.
    pub manager: bool,
    /// Объекты вида несут реквизиты и табличные части.
    pub object: bool,
}

impl MetadataDeclaration {
    pub const fn new(collections: &'static [&'static str]) -> Self {
        Self {
            collections,
            manager: false,
            object: false,
        }
    }

    pub const fn manager(self) -> Self {
        Self {
            manager: true,
            ..self
        }
    }

    pub const fn object(self) -> Self {
        Self {
            object: true,
            ..self
        }
    }
}

/// Место вида в корне конфигурации. Числа — ключи порядка с промежутками:
/// новый вид берёт свободное число между соседями и не трогает их файлов.
#[derive(Debug, Clone, Copy)]
pub struct RootPlace {
    /// Порядок, в котором платформа пишет виды в `ChildObjects` конфигурации.
    pub child_objects: u32,
    /// Порядок групп дерева Конфигуратора: «Общие» (от подсистем до языков),
    /// затем константы, справочники, документы с нумераторами
    /// и последовательностями, журналы и далее до задач. Выгрузка пишет виды
    /// в другом порядке; дерево показывает этот.
    pub tree: Option<u32>,
}

/// Раскладка вида в выгрузке.
#[derive(Debug, Clone, Copy)]
pub struct LayoutDeclaration {
    pub directory: &'static str,
    pub display_name_ru: &'static str,
    pub config_dump_prefix: Option<&'static str>,
    pub config_dump_module_suffix: Option<&'static str>,
    /// Требует ли 8.3.27 контейнер `ChildObjects` в корне описания, в том
    /// числе пустой.
    pub child_objects_8_3_27: bool,
}

impl LayoutDeclaration {
    pub const EMPTY: Self = Self {
        directory: "",
        display_name_ru: "",
        config_dump_prefix: None,
        config_dump_module_suffix: None,
        child_objects_8_3_27: false,
    };
}

/// Раскладка вида в выгрузке вместе с его тегом.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MetadataLayout {
    pub tag: &'static str,
    pub directory: &'static str,
    pub display_name_ru: &'static str,
    pub config_dump_prefix: Option<&'static str>,
    pub config_dump_module_suffix: Option<&'static str>,
    pub child_objects_8_3_27: bool,
}

/// Виды корня, которые выгружает платформа, но которых Unica не знает,
/// с местом в `ChildObjects` конфигурации. Их не создают; место только не
/// даёт новой записи другого вида встать не с той стороны от них.
pub const UNDECLARED_ROOT_KINDS: &[(&str, u32)] = &[("ExternalDataSource", 4650)];

impl NodeKind {
    pub const fn as_str(self) -> &'static str {
        self.declaration().tag
    }
}

/// Вид по каноническому написанию.
pub fn declared(tag: &str) -> Option<(NodeKind, &'static KindDeclaration)> {
    DECLARED
        .iter()
        .find(|(_, declaration)| declaration.tag == tag)
        .map(|(kind, declaration)| (*kind, declaration))
}

/// Вид по написанию сегмента: каноническому или русскому.
pub(crate) fn spelled(token: &str) -> Option<NodeKind> {
    DECLARED
        .iter()
        .find(|(_, declaration)| declaration.tag == token || declaration.russian.contains(&token))
        .map(|(kind, _)| *kind)
}

#[derive(Clone, Copy)]
enum Order {
    ChildObjects,
    Tree,
}

#[derive(Clone, Copy)]
enum Filter {
    Metadata,
    Root,
    Tree,
    Layout,
}

const fn order_key(declaration: &KindDeclaration, order: Order) -> u64 {
    match (declaration.root, order) {
        (Some(root), Order::ChildObjects) => root.child_objects as u64,
        (
            Some(RootPlace {
                tree: Some(tree), ..
            }),
            Order::Tree,
        ) => tree as u64,
        _ => u64::MAX,
    }
}

const fn passes(declaration: &KindDeclaration, filter: Filter) -> bool {
    match filter {
        Filter::Metadata => declaration.metadata.is_some(),
        Filter::Root => declaration.root.is_some(),
        Filter::Tree => matches!(declaration.root, Some(RootPlace { tree: Some(_), .. })),
        Filter::Layout => declaration.layout.is_some(),
    }
}

/// Устойчивая сортировка вставками: виды без ключа остаются в порядке
/// файлов после остальных.
const fn sorted<const N: usize>(
    mut items: [(NodeKind, KindDeclaration); N],
    order: Order,
) -> [(NodeKind, KindDeclaration); N] {
    let mut index = 1;
    while index < N {
        let mut position = index;
        while position > 0
            && order_key(&items[position - 1].1, order) > order_key(&items[position].1, order)
        {
            let previous = items[position - 1];
            items[position - 1] = items[position];
            items[position] = previous;
            position -= 1;
        }
        index += 1;
    }
    items
}

const fn count<const N: usize>(items: &[(NodeKind, KindDeclaration); N], filter: Filter) -> usize {
    let mut total = 0;
    let mut index = 0;
    while index < N {
        if passes(&items[index].1, filter) {
            total += 1;
        }
        index += 1;
    }
    total
}

const fn kinds<const N: usize, const M: usize>(
    items: &[(NodeKind, KindDeclaration); N],
    filter: Filter,
) -> [NodeKind; M] {
    let mut out = [items[0].0; M];
    let mut index = 0;
    let mut next = 0;
    while index < N {
        if passes(&items[index].1, filter) {
            out[next] = items[index].0;
            next += 1;
        }
        index += 1;
    }
    out
}

const fn layouts<const N: usize, const M: usize>(
    items: &[(NodeKind, KindDeclaration); N],
) -> [MetadataLayout; M] {
    let mut out = [MetadataLayout {
        tag: "",
        directory: "",
        display_name_ru: "",
        config_dump_prefix: None,
        config_dump_module_suffix: None,
        child_objects_8_3_27: false,
    }; M];
    let mut index = 0;
    let mut next = 0;
    while index < N {
        if let Some(layout) = items[index].1.layout {
            out[next] = MetadataLayout {
                tag: items[index].1.tag,
                directory: layout.directory,
                display_name_ru: layout.display_name_ru,
                config_dump_prefix: layout.config_dump_prefix,
                config_dump_module_suffix: layout.config_dump_module_suffix,
                child_objects_8_3_27: layout.child_objects_8_3_27,
            };
            next += 1;
        }
        index += 1;
    }
    out
}

const fn root_tags<const N: usize, const M: usize>(
    items: &[(NodeKind, KindDeclaration); N],
) -> [&'static str; M] {
    let mut out = [""; M];
    let mut index = 0;
    let mut next = 0;
    while index < N {
        if items[index].1.root.is_some() {
            out[next] = items[index].1.tag;
            next += 1;
        }
        index += 1;
    }
    out
}

const fn tags<const M: usize>(layouts: &[MetadataLayout; M]) -> [&'static str; M] {
    let mut out = [""; M];
    let mut index = 0;
    while index < M {
        out[index] = layouts[index].tag;
        index += 1;
    }
    out
}

const BY_CHILD_OBJECTS: [(NodeKind, KindDeclaration); DECLARED.len()] =
    sorted(DECLARED, Order::ChildObjects);
const BY_TREE: [(NodeKind, KindDeclaration); DECLARED.len()] = sorted(DECLARED, Order::Tree);

const METADATA_COUNT: usize = count(&DECLARED, Filter::Metadata);
const ROOT_COUNT: usize = count(&DECLARED, Filter::Root);
const TREE_COUNT: usize = count(&DECLARED, Filter::Tree);
const LAYOUT_COUNT: usize = count(&DECLARED, Filter::Layout);

/// Виды объектов метаданных с адресом исходников: виды корня в порядке
/// `ChildObjects`, затем остальные в порядке файлов.
pub(crate) const METADATA_KINDS: [NodeKind; METADATA_COUNT] =
    kinds(&BY_CHILD_OBJECTS, Filter::Metadata);

/// Виды корня в порядке дерева Конфигуратора.
pub(crate) const CONFIGURATOR_TREE_ORDER: [NodeKind; TREE_COUNT] = kinds(&BY_TREE, Filter::Tree);

/// Раскладки видов в порядке, в котором платформа пишет их в `ChildObjects`.
pub const METADATA_LAYOUTS: [MetadataLayout; LAYOUT_COUNT] = layouts(&BY_CHILD_OBJECTS);

/// Теги видов с раскладкой в том же порядке.
pub const METADATA_LAYOUT_TAGS: [&str; LAYOUT_COUNT] = tags(&METADATA_LAYOUTS);

/// Объявленные виды корня в порядке `ChildObjects` конфигурации.
pub const DECLARED_ROOT_KINDS: [&str; ROOT_COUNT] = root_tags(&BY_CHILD_OBJECTS);

/// Место вида корня в `ChildObjects` конфигурации, как его пишет платформа:
/// объявленные виды и виды, которых Unica не знает.
pub fn root_child_objects_place(tag: &str) -> Option<u32> {
    declared(tag)
        .and_then(|(_, declaration)| declaration.root)
        .map(|root| root.child_objects)
        .or_else(|| {
            UNDECLARED_ROOT_KINDS
                .iter()
                .find(|(kind, _)| *kind == tag)
                .map(|(_, place)| *place)
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::{BTreeMap, HashSet};

    /// Каждое написание сегмента, имя коллекции и ключ порядка принадлежат
    /// одному объявлению: второе объявление того же вида здесь падает.
    #[test]
    fn every_spelling_and_place_is_declared_once() {
        let mut spellings = BTreeMap::new();
        let mut collections = BTreeMap::new();
        let mut child_objects = BTreeMap::new();
        let mut tree = BTreeMap::new();
        let mut directories = BTreeMap::new();
        for (kind, declaration) in DECLARED {
            for spelling in
                std::iter::once(declaration.tag).chain(declaration.russian.iter().copied())
            {
                if let Some(other) = spellings.insert(spelling, kind) {
                    panic!("`{spelling}` is declared by {other:?} and {kind:?}");
                }
            }
            for name in declaration
                .metadata
                .iter()
                .flat_map(|metadata| metadata.collections)
            {
                if let Some(other) = collections.insert(name.to_lowercase(), kind) {
                    panic!("collection `{name}` is declared by {other:?} and {kind:?}");
                }
            }
            if let Some(root) = declaration.root {
                if let Some(other) = child_objects.insert(root.child_objects, kind) {
                    panic!(
                        "ChildObjects place {} is declared by {other:?} and {kind:?}",
                        root.child_objects
                    );
                }
                if let Some(place) = root.tree {
                    if let Some(other) = tree.insert(place, kind) {
                        panic!("tree place {place} is declared by {other:?} and {kind:?}");
                    }
                }
            }
            if let Some(layout) = declaration.layout {
                if let Some(other) = directories.insert(layout.directory.to_lowercase(), kind) {
                    panic!(
                        "directory `{}` is declared by {other:?} and {kind:?}",
                        layout.directory
                    );
                }
            }
        }
        for (name, _) in &collections {
            assert!(
                !spellings
                    .keys()
                    .any(|spelling| spelling.to_lowercase() == *name),
                "collection `{name}` collides with a segment spelling"
            );
        }
        for (kind, place) in UNDECLARED_ROOT_KINDS {
            assert!(
                declared(kind).is_none(),
                "{kind} is declared and undeclared"
            );
            assert!(
                !child_objects.contains_key(place),
                "ChildObjects place {place} of {kind} is taken"
            );
        }
    }

    #[test]
    fn every_kind_round_trips_through_its_canonical_spelling() {
        for (kind, declaration) in DECLARED {
            assert_eq!(kind.as_str(), declaration.tag);
            assert_eq!(spelled(declaration.tag), Some(kind));
            assert_eq!(
                declared(declaration.tag).map(|(found, _)| found),
                Some(kind)
            );
        }
    }

    /// Вид с раскладкой — вид корня с адресом и профилем порождённых типов.
    #[test]
    fn a_laid_out_kind_is_a_root_metadata_kind_with_generated_types() {
        for (kind, declaration) in DECLARED {
            if declaration.layout.is_some() {
                assert!(declaration.metadata.is_some(), "{kind:?} has no address");
                assert!(declaration.root.is_some(), "{kind:?} has no root place");
                assert!(
                    declaration.generated_types_8_3_27.is_some(),
                    "{kind:?} has no generated types profile"
                );
            }
            if declaration.since.is_some() {
                assert!(
                    declaration.root.is_some(),
                    "{kind:?} has a line but no root place"
                );
            }
        }
    }

    #[test]
    fn derived_lists_follow_the_declared_places() {
        let places = METADATA_LAYOUTS
            .iter()
            .map(|layout| root_child_objects_place(layout.tag).unwrap())
            .collect::<Vec<_>>();
        assert!(places.windows(2).all(|pair| pair[0] < pair[1]));
        let tree = CONFIGURATOR_TREE_ORDER
            .iter()
            .map(|kind| kind.declaration().root.unwrap().tree.unwrap())
            .collect::<Vec<_>>();
        assert!(tree.windows(2).all(|pair| pair[0] < pair[1]));
        assert_eq!(
            METADATA_KINDS.iter().collect::<HashSet<_>>().len(),
            METADATA_KINDS.len()
        );
        assert_eq!(METADATA_LAYOUT_TAGS.len(), METADATA_LAYOUTS.len());
    }
}
