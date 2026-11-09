use serde_json::Value;
use unica_address::address::NodeKind;

/// Ветви объекта в порядке дерева Конфигуратора: измерения, ресурсы,
/// реквизиты, табличные части, значения, графы, перерасчёты, операции,
/// шаблоны URL, затем формы, команды и макеты. Стандартные реквизиты и
/// табличные части идут за своими видами; характеристики и предопределённые
/// элементы, которых дерево не показывает, — в конце.
pub(super) fn metadata_branch_kinds() -> &'static [NodeKind] {
    &[
        NodeKind::Dimension,
        NodeKind::Resource,
        NodeKind::Attribute,
        NodeKind::AddressingAttribute,
        NodeKind::StandardAttribute,
        NodeKind::TabularSection,
        NodeKind::StandardTabularSection,
        NodeKind::EnumValue,
        NodeKind::Column,
        NodeKind::Recalculation,
        NodeKind::Operation,
        NodeKind::UrlTemplate,
        NodeKind::Form,
        NodeKind::Command,
        NodeKind::Template,
        NodeKind::Characteristic,
        NodeKind::PredefinedItem,
    ]
}

pub(super) fn metadata_collection(payload: &Value, kind: NodeKind) -> Option<&Value> {
    let collections = payload.get("collections")?;
    let declarations = payload.get("declarations");
    match kind {
        NodeKind::Attribute => collections.get("attributes"),
        NodeKind::AddressingAttribute
            if payload.get("kind").and_then(Value::as_str) == Some("Task") =>
        {
            collections.get("addressingAttributes")
        }
        NodeKind::StandardAttribute => declarations?.get("standardAttributes"),
        NodeKind::Characteristic => declarations?.get("characteristics"),
        NodeKind::StandardTabularSection => declarations?.get("standardTabularSections"),
        // Страница элементов лежит в `items`, а правдивый счёт — в `total`
        // рядом: при урезанной странице длина массива соврала бы.
        NodeKind::PredefinedItem => payload.get("predefinedItems")?.get("items"),
        NodeKind::TabularSection => collections.get("tabularSections"),
        NodeKind::Dimension => collections.get("dimensions"),
        NodeKind::Resource => collections.get("resources"),
        NodeKind::Recalculation => collections.get("recalculations"),
        NodeKind::EnumValue => collections.get("enumValues"),
        NodeKind::Column => collections.get("columns"),
        NodeKind::Form => collections.get("forms"),
        NodeKind::Template => collections.get("templates"),
        NodeKind::Command => collections.get("commands"),
        // Последовательности пофактовой части вида: платформа держит их не в
        // `collections`, но для читателя это такая же адресуемая коллекция.
        NodeKind::UrlTemplate => metadata_details(payload)?.get("urlTemplates"),
        NodeKind::Operation => metadata_details(payload)?.get("operations"),
        _ => None,
    }
}

/// Пофактовая часть вида: `details` приходит соседним тегом `{kind, details}`.
pub(super) fn metadata_details(payload: &Value) -> Option<&Value> {
    payload
        .get("details")
        .and_then(|adjacent| adjacent.get("details"))
}
