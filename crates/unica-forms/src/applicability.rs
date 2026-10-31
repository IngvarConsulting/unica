use unica_address::address::AddressSegment;
use unica_address::address::NodeKind;
use unica_metadata::site::{metadata_collection_matches, MetadataSiteExt};
use unica_metadata::MetaCollection;
use unica_source_contract::context::OperationSite;
use unica_source_contract::route::RouteClass;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OperationApplicability {
    Form,
    FormEvent,
    FormAttribute,
    FormItem,
    FormItemContainer,
    MetadataForms,
}
impl OperationApplicability {
    pub const fn shape(self) -> &'static str {
        match self {
            Self::MetadataForms => "`Kind.Name` of a kind with forms",
            Self::Form => "`Kind.Name.Form.F`",
            Self::FormItemContainer => "`Kind.Name.Form.F[.Item.I…]`",
            Self::FormItem => "`Kind.Name.Form.F.Item.I[.Item.J…]`",
            Self::FormEvent => "`Kind.Name.Form.F[.Item.I…].Event.E`",
            Self::FormAttribute => "`Kind.Name.Form.F.Attribute.A`",
        }
    }
    pub fn matches(self, site: &OperationSite<'_>) -> bool {
        let segments = site.segments();
        match self {
            Self::MetadataForms => site.metadata_owner_with(MetaCollection::Forms),
            Self::Form => site.route() == RouteClass::Form && site.owned_form(),
            Self::FormItemContainer => site.form_items(segments).is_some(),
            Self::FormItem => site
                .form_items(segments)
                .is_some_and(|items| !items.is_empty()),
            Self::FormAttribute => match segments {
                [path @ .., attribute] => {
                    path.len() == 2
                        && attribute.kind() == NodeKind::Attribute
                        && attribute.name().is_some()
                        && site.form_items(path).is_some()
                }
                [] => false,
            },
            Self::FormEvent => match segments {
                [path @ .., event] => {
                    event.kind() == NodeKind::Event
                        && event.name().is_some()
                        && site.form_items(path).is_some()
                }
                [] => false,
            },
        }
    }
}
trait SiteExt {
    fn form_items<'s>(&self, segments: &'s [AddressSegment]) -> Option<&'s [AddressSegment]>;
}
impl SiteExt for OperationSite<'_> {
    /// Цепочка элементов формы объекта: `Owner.X.Form.F[.Item.A[.Item.B…]]`
    /// даёт сегменты `Item` после формы (пустые у самой формы). `None` —
    /// `segments` не такой адрес или маршрут не формы.
    fn form_items<'s>(&self, segments: &'s [AddressSegment]) -> Option<&'s [AddressSegment]> {
        match segments {
            [owner, form, items @ ..]
                if self.route() == RouteClass::Form
                    && owner.name().is_some()
                    && form.kind() == NodeKind::Form
                    && form.name().is_some()
                    && metadata_collection_matches(owner.kind(), MetaCollection::Forms)
                    && items
                        .iter()
                        .all(|item| item.kind() == NodeKind::Item && item.name().is_some()) =>
            {
                Some(items)
            }
            _ => None,
        }
    }
}
