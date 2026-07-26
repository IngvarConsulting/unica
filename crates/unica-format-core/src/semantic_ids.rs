//! Closed semantic vocabulary owned by the compiler.

use std::fmt::{Display, Formatter};

use serde::{Serialize, Serializer};

macro_rules! semantic_id {
    ($(#[$meta:meta])* $name:ident) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(&'static str);

        impl $name {
            const fn core(value: &'static str) -> Self {
                Self(value)
            }

            pub const fn as_str(self) -> &'static str {
                self.0
            }
        }

        impl Display for $name {
            fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
                formatter.write_str(self.0)
            }
        }

        impl Serialize for $name {
            fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
            where
                S: Serializer,
            {
                serializer.serialize_str(self.0)
            }
        }
    };
}

semantic_id!(
    /// Compiler-owned semantic property identifier.
    ///
    /// Adapter-defined strings cannot become semantic property IDs:
    ///
    /// ```compile_fail
    /// use unica_format_core::semantic_ids::SemanticPropertyId;
    /// let _: SemanticPropertyId = "adapter.custom".into();
    /// ```
    SemanticPropertyId
);

impl SemanticPropertyId {
    pub const NAME: Self = Self::core("name");
    pub const SYNONYM: Self = Self::core("synonym");
    pub const COMMENT: Self = Self::core("comment");
    pub const UUID: Self = Self::core("uuid");
}

semantic_id!(
    /// Compiler-owned semantic relation identifier.
    ///
    /// ```compile_fail
    /// use unica_format_core::semantic_ids::SemanticRelationId;
    /// let _: SemanticRelationId = "adapter.children".into();
    /// ```
    SemanticRelationId
);

impl SemanticRelationId {
    pub const CHILDREN: Self = Self::core("children");
    pub const ATTRIBUTES: Self = Self::core("attributes");
    pub const TABULAR_SECTIONS: Self = Self::core("tabularSections");
    pub const FORMS: Self = Self::core("forms");
    pub const COMMANDS: Self = Self::core("commands");
    pub const TEMPLATES: Self = Self::core("templates");
}

semantic_id!(
    /// Compiler-owned semantic facet identifier.
    ///
    /// ```compile_fail
    /// use unica_format_core::semantic_ids::SemanticFacetId;
    /// let _: SemanticFacetId = "adapter.nativeXml".into();
    /// ```
    SemanticFacetId
);

impl SemanticFacetId {
    pub const SUMMARY: Self = Self::core("summary");
    pub const DETAILS: Self = Self::core("details");
    pub const SOURCE: Self = Self::core("source");
}

semantic_id!(
    /// Compiler-owned semantic object kind.
    ///
    /// ```compile_fail
    /// use unica_format_core::semantic_ids::SemanticObjectKind;
    /// let _: SemanticObjectKind = "adapter.privateKind".into();
    /// ```
    SemanticObjectKind
);

impl SemanticObjectKind {
    pub const SOURCE_ROOT: Self = Self::core("source_root");
    pub const DOCUMENT: Self = Self::core("document");
    pub const METADATA_OBJECT: Self = Self::core("metadata_object");
    pub const ATTRIBUTE: Self = Self::core("attribute");
    pub const TABULAR_SECTION: Self = Self::core("tabular_section");
    pub const COMMAND: Self = Self::core("command");
    pub const FORM: Self = Self::core("form");
    pub const FORM_ATTRIBUTE: Self = Self::core("form_attribute");
    pub const FORM_COMMAND: Self = Self::core("form_command");
    pub const FORM_ELEMENT: Self = Self::core("form_element");
    pub const TEMPLATE: Self = Self::core("template");
}
