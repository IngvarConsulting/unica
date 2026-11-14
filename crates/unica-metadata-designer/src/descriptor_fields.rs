//! Поля дескриптора, которые читают разные виды объектов одинаково:
//! единственный дочерний узел пространства имён и локализованный синоним.
use roxmltree::Node;

pub const MD: &str = "http://v8.1c.ru/8.3/MDClasses";
pub const CORE: &str = "http://v8.1c.ru/8.1/data/core";

/// Единственный дочерний узел `ns:name`. Второй такой узел — порча: читатель
/// не выбирает между двумя значениями одного свойства молча.
pub fn single<'a, 'input>(
    node: Node<'a, 'input>,
    ns: &str,
    name: &str,
) -> Result<Option<Node<'a, 'input>>, String> {
    let mut found = node
        .children()
        .filter(|child| child.has_tag_name((ns, name)));
    let first = found.next();
    if found.next().is_some() {
        return Err(format!("duplicate {name}"));
    }
    Ok(first)
}

/// Текст синонима на русском, а без него — на первом языке с текстом.
pub fn synonym(node: Node<'_, '_>) -> Result<String, String> {
    let entries = node
        .children()
        .filter(|child| child.has_tag_name((CORE, "item")))
        .filter_map(|item| {
            let text = |name| {
                item.children()
                    .find(|child| child.has_tag_name((CORE, name)))
                    .and_then(|child| child.text())
            };
            Some((text("lang"), text("content")?))
        })
        .filter(|(_, content)| !content.is_empty())
        .collect::<Vec<_>>();
    if entries.is_empty() && node.children().any(|child| child.is_element()) {
        return Err("invalid localized synonym".into());
    }
    Ok(entries
        .iter()
        .find(|(lang, _)| *lang == Some("ru"))
        .or(entries.first())
        .map(|(_, content)| content.to_string())
        .unwrap_or_default())
}
