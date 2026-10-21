//! Existing addressable DCS settings structure hierarchy.
use serde_json::Value;

/// Строки структуры варианта, чей ближайший именованный предок — элемент
/// `scope` (`None` — сам вариант). Безымянный элемент прозрачен: его потомки
/// принадлежат ближайшему именованному предку, поэтому именованная группа под
/// безымянной адресуется от варианта (#1357).
pub fn dcs_structure_scope(rows: &[Value], scope: Option<usize>) -> Vec<usize> {
    let named = |index: usize| {
        rows.get(index)
            .and_then(|row| row.get("name"))
            .and_then(Value::as_str)
            .is_some_and(|name| !name.is_empty())
    };
    let parent = |index: usize| {
        rows.get(index)
            .and_then(|row| row.get("parentIndex"))
            .and_then(Value::as_u64)
            .and_then(|parent| usize::try_from(parent).ok())
    };
    (0..rows.len())
        .filter(|index| {
            let mut current = parent(*index);
            // Глубина ограничена числом строк: испорченный `parentIndex`
            // не зацикливает обход.
            for _ in 0..rows.len() {
                match current {
                    Some(ancestor) if !named(ancestor) => current = parent(ancestor),
                    _ => break,
                }
            }
            current == scope
        })
        .collect()
}

/// Имя строки структуры, которое адресует её в области: непустое и
/// единственное среди строк этой области.
pub fn dcs_structure_address_name<'a>(
    rows: &'a [Value],
    scope: &[usize],
    index: usize,
) -> Option<&'a str> {
    let name = rows[index]
        .get("name")
        .and_then(Value::as_str)
        .filter(|name| !name.is_empty())?;
    (scope
        .iter()
        .filter(|other| rows[**other].get("name").and_then(Value::as_str) == Some(name))
        .count()
        == 1)
        .then_some(name)
}

/// Строка в области `scope` по имени сегмента адреса. Отсутствующее и
/// повторённое имя одинаково не дают узла: адрес не выбирает за читателя.
pub fn dcs_structure_child(rows: &[Value], scope: Option<usize>, name: &str) -> Option<usize> {
    let members = dcs_structure_scope(rows, scope);
    members
        .iter()
        .copied()
        .find(|index| dcs_structure_address_name(rows, &members, *index) == Some(name))
}

/// Пути от варианта до элементов структуры с именем `name`: цепочки имён
/// именованных предков и самого элемента. Пусто — элемента нет; в ответ
/// входят только те, чей путь адресует именно их (имя единственно на каждом
/// уровне), а `found` — сколько элементов с этим именем есть всего.
pub fn dcs_structure_paths(rows: &[Value], name: &str) -> (Vec<Vec<String>>, usize) {
    let row_name = |index: usize| {
        rows.get(index)
            .and_then(|row| row.get("name"))
            .and_then(Value::as_str)
            .filter(|name| !name.is_empty())
    };
    let parent = |index: usize| {
        rows.get(index)
            .and_then(|row| row.get("parentIndex"))
            .and_then(Value::as_u64)
            .and_then(|parent| usize::try_from(parent).ok())
    };
    let mut found = 0;
    let mut paths = Vec::new();
    for index in (0..rows.len()).filter(|index| row_name(*index) == Some(name)) {
        found += 1;
        let mut chain = vec![name.to_string()];
        let mut current = parent(index);
        for _ in 0..rows.len() {
            let Some(ancestor) = current else { break };
            if let Some(ancestor_name) = row_name(ancestor) {
                chain.push(ancestor_name.to_string());
            }
            current = parent(ancestor);
        }
        chain.reverse();
        let mut scope = None;
        for step in &chain {
            scope = dcs_structure_child(rows, scope, step);
            if scope.is_none() {
                break;
            }
        }
        if scope == Some(index) {
            paths.push(chain);
        }
    }
    (paths, found)
}
