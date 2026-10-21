use unica_address::address::AddressSegment;
use unica_address::address::NodeKind;
use unica_source_contract::context::OperationSite;
use unica_source_contract::route::RouteClass;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OperationApplicability {
    Code,
    CodeInsert,
}
impl OperationApplicability {
    pub const fn shape(self) -> &'static str {
        match self {
            Self::CodeInsert => "a module address, its `Body` or `Method.M`",
            Self::Code => "a module address, its `Body`, `Method.M` or `Method.M.Body`",
        }
    }
    pub fn matches(self, site: &OperationSite<'_>) -> bool {
        let segments = site.segments();
        let terminal = site.terminal();
        match self {
            Self::CodeInsert => {
                site.route() == RouteClass::Module
                    && (module_terminal(segments)
                        || (terminal.kind() == NodeKind::Body && !method_body(segments))
                        || (terminal.kind() == NodeKind::Method && terminal.name().is_some()))
            }
            Self::Code => {
                site.route() == RouteClass::Module
                    && (module_terminal(segments)
                        || terminal.kind() == NodeKind::Body
                        || (terminal.kind() == NodeKind::Method && terminal.name().is_some()))
            }
        }
    }
}
/// Адрес самого модуля: `CommonModule.X`, `Owner.X.Module.Object`,
/// `Module.Session`.
fn module_terminal(segments: &[AddressSegment]) -> bool {
    match segments.last() {
        Some(last) if last.kind() == NodeKind::Module => last.name().is_some(),
        Some(last) if last.kind() == NodeKind::CommonModule => {
            segments.len() == 1 && last.name().is_some()
        }
        _ => false,
    }
}

fn method_body(segments: &[AddressSegment]) -> bool {
    matches!(segments, [.., method, body]
        if method.kind() == NodeKind::Method && body.kind() == NodeKind::Body)
}
