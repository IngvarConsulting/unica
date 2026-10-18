//! Класс маршрута логического адреса.
//!
//! Один и тот же вид сегмента живёт в разных деревьях: `Attribute` бывает
//! реквизитом объекта метаданных и реквизитом формы, `Parameter` — параметром
//! СКД и параметром операции веб-сервиса, `Method` — методом модуля и методом
//! HTTP-сервиса. Какой читатель владеет узлом, решают сегменты адреса целиком,
//! а не вид последнего сегмента. Это решение принимается здесь, без чтения
//! файлов: им пользуются и маршрутизатор чтения, и применимость операций
//! `apply`, поэтому второго классификатора быть не может.

use crate::domain::address::{NodeKind, QualifiedAddress};
use crate::domain::platform_profile::PlatformProfile;
use serde::Deserialize;

/// Дерево, которому принадлежит узел, — и читатель, который его проецирует.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum RouteClass {
    /// Корень набора исходников.
    Configuration,
    /// Объект метаданных, ветвь их видов и члены объекта: реквизиты, табличные
    /// части, измерения, команды, макеты как узлы владельца.
    Metadata,
    /// Форма и её подузлы: элементы, реквизиты, команды, события формы.
    Form,
    /// Роль и её права.
    Role,
    /// Подсистема и её состав.
    Subsystem,
    /// Командный интерфейс подсистемы.
    Interface,
    /// Схема компоновки данных внутри макета.
    Dcs,
    /// Табличный документ внутри макета.
    Mxl,
    /// Пакет XDTO.
    Xdto,
    /// Модуль и узлы его текста.
    Module,
}

/// Класс маршрута по сегментам адреса и профилю платформы.
///
/// `None` — адрес не может существовать ни в одном дереве: сегмент `Module`
/// вне модульного маршрута профиля. Ответ `Some` не доказывает существование
/// узла; наличие и форму адреса проверяет читатель своего маршрута.
pub(crate) fn classify_route(
    address: &QualifiedAddress,
    profile: PlatformProfile,
) -> Option<RouteClass> {
    if address.form_metadata_owner().is_some() {
        return Some(RouteClass::Metadata);
    }
    if profile.module_prefix_capability(address).is_some()
        || !profile.module_children(address).is_empty()
    {
        return Some(RouteClass::Module);
    }
    let segments = address.segments();
    let any = |kind: NodeKind| segments.iter().any(|segment| segment.kind() == kind);
    let any_named = |kind: NodeKind| {
        segments
            .iter()
            .any(|segment| segment.kind() == kind && segment.name().is_some())
    };
    let first = segments.first()?;
    if any(NodeKind::Module) {
        return None;
    }
    if matches!(segments, [root] if root.kind() == NodeKind::Configuration && root.name().is_none())
    {
        return Some(RouteClass::Configuration);
    }
    if matches!(segments, [branch] if branch.kind().is_metadata_kind() && branch.name().is_none()) {
        return Some(RouteClass::Metadata);
    }
    match first.kind() {
        NodeKind::Role => return Some(RouteClass::Role),
        NodeKind::Subsystem => {
            return Some(if any(NodeKind::Interface) {
                RouteClass::Interface
            } else {
                RouteClass::Subsystem
            })
        }
        _ => {}
    }
    if (first.kind() == NodeKind::CommonForm && first.name().is_some()) || any_named(NodeKind::Form)
    {
        return Some(RouteClass::Form);
    }
    let dcs_body = segments.iter().any(|segment| {
        matches!(
            segment.kind(),
            NodeKind::DataSet
                | NodeKind::Field
                | NodeKind::Query
                | NodeKind::DataSource
                | NodeKind::CalculatedField
                | NodeKind::Total
                | NodeKind::Setting
        )
    });
    let dcs_template_member = (any_named(NodeKind::Template)
        || any_named(NodeKind::CommonTemplate))
        && !any(NodeKind::Area)
        && segments
            .iter()
            .any(|segment| matches!(segment.kind(), NodeKind::Item | NodeKind::Parameter));
    if dcs_body || dcs_template_member {
        return Some(RouteClass::Dcs);
    }
    if any(NodeKind::Area) {
        return Some(RouteClass::Mxl);
    }
    if first.kind() == NodeKind::XdtoPackage {
        return Some(RouteClass::Xdto);
    }
    Some(RouteClass::Metadata)
}

#[cfg(test)]
mod tests {
    use super::{classify_route, RouteClass};
    use crate::domain::address::QualifiedAddress;
    use crate::domain::platform_profile::PlatformProfile;

    fn route(raw: &str) -> Option<RouteClass> {
        classify_route(
            &QualifiedAddress::parse(raw).unwrap(),
            PlatformProfile::v8_3_27(),
        )
    }

    /// Один вид сегмента в разных деревьях даёт разные маршруты: решает адрес
    /// целиком, а не вид последнего сегмента.
    #[test]
    fn the_same_terminal_kind_routes_by_its_ancestors() {
        for (raw, expected) in [
            ("main:Document.Order.Attribute.Total", RouteClass::Metadata),
            (
                "main:Document.Order.Form.Main.Attribute.Object",
                RouteClass::Form,
            ),
            ("main:CommonForm.Main.Attribute.Object", RouteClass::Form),
            ("main:Document.Order.Command.Print", RouteClass::Metadata),
            ("main:Document.Order.Form.Main.Command.Ok", RouteClass::Form),
            (
                "main:Report.Sales.Template.Schema.Parameter.Period",
                RouteClass::Dcs,
            ),
            (
                "main:Report.Sales.Template.Schema.DataSource.Local",
                RouteClass::Dcs,
            ),
            (
                "main:Report.Sales.Template.Schema.CalculatedField.Double",
                RouteClass::Dcs,
            ),
            (
                "main:Report.Sales.Template.Schema.Total.Amount",
                RouteClass::Dcs,
            ),
            (
                "main:Report.Sales.Template.Schema.Setting.Main.Item.ByCategory.Item.Details",
                RouteClass::Dcs,
            ),
            (
                "main:WebService.Exchange.Operation.Send.Parameter.Body",
                RouteClass::Metadata,
            ),
            (
                "main:HTTPService.Api.URLTemplate.Items.Method.Get",
                RouteClass::Metadata,
            ),
            ("main:CommonModule.Shared.Method.Run", RouteClass::Module),
            (
                "main:Document.Order.Module.Object.Region.Main",
                RouteClass::Module,
            ),
            ("main:Report.Sales.Template.Print", RouteClass::Metadata),
            (
                "main:Report.Sales.Template.Print.Area.Header",
                RouteClass::Mxl,
            ),
            ("main:Role.Reader.Right.Catalog_Items", RouteClass::Role),
            ("main:Subsystem.Sales.Interface", RouteClass::Interface),
            (
                "main:Subsystem.Sales.Subsystem.Inner",
                RouteClass::Subsystem,
            ),
            ("main:XDTOPackage.Exchange.Type.Order", RouteClass::Xdto),
            ("main:Configuration", RouteClass::Configuration),
            ("main:Catalog", RouteClass::Metadata),
        ] {
            assert_eq!(route(raw), Some(expected), "{raw}");
        }
    }

    #[test]
    fn a_module_segment_outside_a_module_route_has_no_tree() {
        assert_eq!(
            route("main:Document.Order.Module.Object.Method.Run.Module.Service"),
            None
        );
    }
}
