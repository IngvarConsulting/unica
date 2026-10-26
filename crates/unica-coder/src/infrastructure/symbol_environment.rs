//! The source set behind the symbol port: module addresses by layout, saved
//! module texts and metadata nodes proved by the logical reader.

use unica_address::address::QualifiedAddress;
use unica_code_intelligence::symbols::SymbolEnvironment;

use crate::infrastructure::v13_read::LogicalViewReadAuthority;

/// Field kinds a provider names with one `attribute`, in the order tried.
const FIELD_KINDS: [&str; 4] = ["Attribute", "Dimension", "Resource", "StandardAttribute"];

pub(crate) struct ReaderSymbolEnvironment<'a, 'b> {
    pub(crate) authority: &'a LogicalViewReadAuthority<'b>,
    pub(crate) source_set: &'a str,
}

impl ReaderSymbolEnvironment<'_, '_> {
    fn existing(&self, raw: &str) -> Option<String> {
        let address = QualifiedAddress::parse(raw).ok()?;
        self.authority
            .node_exists(&address)
            .then(|| address.to_string())
    }
}

impl SymbolEnvironment for ReaderSymbolEnvironment<'_, '_> {
    fn module_address(&self, path: &str) -> Option<String> {
        self.authority
            .module_address_for_path(path)
            .map(|address| address.to_string())
    }

    fn metadata_address(&self, kind: &str, object: &str, member: Option<&str>) -> Option<String> {
        let owner = format!("{}:{kind}.{object}", self.source_set);
        match member {
            None => self.existing(&owner),
            Some(member) => FIELD_KINDS
                .iter()
                .find_map(|field| self.existing(&format!("{owner}.{field}.{member}"))),
        }
    }

    fn module_text(&self, at: &str) -> Option<String> {
        self.authority
            .module_text(&QualifiedAddress::parse(at).ok()?)
            .ok()
    }
}
