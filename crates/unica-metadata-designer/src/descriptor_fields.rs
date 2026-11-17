//! Поля дескриптора, которые читают разные виды объектов одинаково:
//! единственный дочерний узел пространства имён и локализованный синоним.
use roxmltree::Node;

pub const MD: &str = "http://v8.1c.ru/8.3/MDClasses";

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

/// Полное значение синонима в форме НСтр.
pub fn synonym(node: Node<'_, '_>) -> Result<String, String> {
    crate::info_projection::localized_node_as_nstr(node).map(Option::unwrap_or_default)
}
