//! The addressed descriptor owns its property vocabulary and argument shape.

use super::address::{NodeKind, QualifiedAddress};
use super::apply::{OperationApplicability, OperationSkeleton};
use super::metadata::{metadata_property_schemas, MetadataKind};
use super::operation_contract::{object, OperationContract};
use serde_json::{json, Map, Value};

macro_rules! properties_operations {
    ($($variant:ident => ($name:literal, $applicability:ident, $skeleton:ident)),+ $(,)?) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        pub(crate) enum PropertiesOperation { $($variant),+ }

        impl PropertiesOperation {
            #[cfg(test)]
            pub(crate) const ALL: &'static [Self] = &[$(Self::$variant),+];

            pub(crate) const fn name(self) -> &'static str {
                match self { $(Self::$variant => $name),+ }
            }

            pub(super) const fn applicability(self) -> OperationApplicability {
                match self { $(Self::$variant => OperationApplicability::$applicability),+ }
            }

            pub(super) const fn skeleton(self) -> OperationSkeleton {
                match self { $(Self::$variant => OperationSkeleton::$skeleton),+ }
            }
        }
    };
}

properties_operations!(
    Set => ("props.set", MetadataOrRoleOrSubsystem, Values),
);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PropertiesSite {
    Metadata(MetadataKind),
    Role,
    Subsystem,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DescriptorPropertyKind {
    String,
    Boolean,
}

macro_rules! descriptor_properties {
    ($($field:ident => ($name:literal, $alias:literal, $kind:ident, $role:literal)),+ $(,)?) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        pub(crate) enum DescriptorProperty {
            $($field),+
        }

        impl DescriptorProperty {
            const ALL: &'static [Self] = &[$(Self::$field),+];

            pub(crate) const fn names(self) -> &'static [&'static str] {
                match self { $(Self::$field => &[$name, $alias]),+ }
            }

            pub(crate) const fn kind(self) -> DescriptorPropertyKind {
                match self { $(Self::$field => DescriptorPropertyKind::$kind),+ }
            }

            fn applies(self, site: PropertiesSite) -> bool {
                match site {
                    PropertiesSite::Subsystem => true,
                    PropertiesSite::Role => match self { $(Self::$field => $role),+ },
                    PropertiesSite::Metadata(_) => false,
                }
            }
        }
    };
}

descriptor_properties!(
    Synonym => ("Synonym", "synonym", String, true),
    Comment => ("Comment", "comment", String, true),
    Explanation => ("Explanation", "explanation", String, false),
    IncludeHelpInContents => ("IncludeHelpInContents", "includeHelpInContents", Boolean, false),
    IncludeInCommandInterface => ("IncludeInCommandInterface", "includeInCommandInterface", Boolean, false),
    UseOneCommand => ("UseOneCommand", "useOneCommand", Boolean, false),
);

impl PropertiesSite {
    /// Read routing is deliberately irrelevant here: CommonModule is read as
    /// a module and still owns a metadata descriptor. Members and nested
    /// subsystems do not acquire the owner's write profile.
    pub(crate) fn from_address(at: &QualifiedAddress) -> Option<Self> {
        let [owner] = at.segments() else {
            return None;
        };
        owner.name()?;
        match owner.kind() {
            NodeKind::Role => Some(Self::Role),
            NodeKind::Subsystem => Some(Self::Subsystem),
            kind => MetadataKind::parse(kind.as_str()).ok().map(Self::Metadata),
        }
    }

    pub(crate) fn descriptor_fields(self) -> impl Iterator<Item = DescriptorProperty> {
        DescriptorProperty::ALL
            .iter()
            .copied()
            .filter(move |field| field.applies(self))
    }

    fn values_schema(self) -> Value {
        let mut properties = Map::new();
        let mut groups = Vec::new();
        match self {
            Self::Metadata(kind) => properties = metadata_property_schemas(Some(kind)),
            Self::Role | Self::Subsystem => {
                for field in self.descriptor_fields() {
                    let schema = match field.kind() {
                        DescriptorPropertyKind::String => json!({"type":"string"}),
                        DescriptorPropertyKind::Boolean => json!({"type":"boolean"}),
                    };
                    for name in field.names() {
                        properties.insert((*name).to_string(), schema.clone());
                    }
                    groups.push(field.names());
                }
            }
        }
        let mut values = json!({"type":"object", "additionalProperties":false,
            "minProperties":1, "properties":properties});
        if !groups.is_empty() {
            values["not"] = json!({"anyOf":groups.iter()
                .map(|names| json!({"required":names})).collect::<Vec<_>>()});
            // Each property is a separate group. The boundary reads original
            // spellings before conversion; generic alias normalization must
            // neither merge these groups nor erase diagnostic provenance.
            values["x-unica-alias-groups"] = json!(groups);
        }
        values
    }
}

impl PropertiesOperation {
    pub(crate) fn contract(self, site: PropertiesSite) -> OperationContract {
        match self {
            Self::Set => OperationContract {
                schema: object(&[("values", site.values_schema())], &["values"]),
                target: "The existing named top-level metadata object, role or subsystem at its full address. The address selects the property profile.",
                effect: "Change the supplied descriptor properties, preserving identity, other properties, translations and companion files.",
                notes: "values is a nonempty object. Use JSON booleans for flags. An empty string clears text; null is not omission. Use one spelling of each property, never both aliases. Argument errors are checked before descriptor state; valid arguments still require an existing readable descriptor. Preview writes nothing; execute only the saved execution token.",
                example_args: json!({"values":{"Comment":"Updated comment"}}),
                help: match site {
                    PropertiesSite::Metadata(_) => "skills/meta-edit/SKILL.md",
                    PropertiesSite::Role => "skills/role-edit/SKILL.md",
                    PropertiesSite::Subsystem => "skills/subsystem-edit/SKILL.md",
                },
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registration_and_context_cover_only_current_descriptor_owners() {
        let operation = PropertiesOperation::Set;
        assert_eq!(PropertiesOperation::ALL, &[operation]);
        assert_eq!(operation.name(), "props.set");
        assert_eq!(
            operation.applicability(),
            OperationApplicability::MetadataOrRoleOrSubsystem
        );
        assert_eq!(operation.skeleton(), OperationSkeleton::Values);
        for kind in MetadataKind::ALL {
            let at = QualifiedAddress::parse(&format!("main:{}.Example", kind.as_str())).unwrap();
            assert_eq!(
                PropertiesSite::from_address(&at),
                Some(PropertiesSite::Metadata(*kind))
            );
        }
        for (at, expected) in [
            ("main:Role.Reader", Some(PropertiesSite::Role)),
            ("main:Subsystem.Sales", Some(PropertiesSite::Subsystem)),
            ("main:Configuration", None),
            ("main:Catalog", None),
            ("main:CommonForm.Card.Metadata", None),
            ("main:Catalog.Items.Attribute.Code", None),
            ("main:Subsystem.Sales.Subsystem.Child", None),
        ] {
            assert_eq!(
                PropertiesSite::from_address(&QualifiedAddress::parse(at).unwrap()),
                expected,
                "{at}"
            );
        }
    }

    #[test]
    fn contextual_schema_preserves_metadata_types_and_owner_specific_enums() {
        for (kind, values, expected) in [
            (
                MetadataKind::Task,
                json!({"DefaultPresentation":"AsNumber"}),
                false,
            ),
            (
                MetadataKind::Task,
                json!({"DefaultPresentation":"AsCode"}),
                false,
            ),
            (
                MetadataKind::Catalog,
                json!({"DefaultPresentation":"AsCode"}),
                true,
            ),
            (
                MetadataKind::Catalog,
                json!({"DefaultPresentation":"AsNumber"}),
                false,
            ),
            (
                MetadataKind::InformationRegister,
                json!({"Periodicity":"Second"}),
                true,
            ),
            (
                MetadataKind::CalculationRegister,
                json!({"Periodicity":"Second"}),
                false,
            ),
            (
                MetadataKind::CalculationRegister,
                json!({"Periodicity":"Day"}),
                true,
            ),
            (
                MetadataKind::Constant,
                json!({"Type":{"variants":[{"kind":"boolean"}]}}),
                true,
            ),
            (
                MetadataKind::DefinedType,
                json!({"Type":{"variants":[{"kind":"boolean"}]}}),
                false,
            ),
            (
                MetadataKind::CommonModule,
                json!({"ClientOrdinaryApplication":false}),
                true,
            ),
            (
                MetadataKind::Document,
                json!({"ClientOrdinaryApplication":false}),
                false,
            ),
        ] {
            let contract = PropertiesOperation::Set.contract(PropertiesSite::Metadata(kind));
            let oracle = jsonschema::validator_for(&contract.schema).unwrap();
            assert_eq!(
                oracle.is_valid(&json!({"values":values})),
                expected,
                "{kind:?}: {values}"
            );
        }
    }

    #[test]
    fn normalization_keeps_independent_alias_groups_and_rejects_collisions() {
        for site in [PropertiesSite::Role, PropertiesSite::Subsystem] {
            let contract = PropertiesOperation::Set.contract(site);
            let args = json!({"values":{"synonym":"S","Comment":"C"}});
            assert_eq!(contract.normalize(&args, "args").unwrap(), args);
            for values in [
                json!({"Synonym":"S","synonym":"S"}),
                json!({"Comment":"C","comment":"different"}),
                json!({}),
            ] {
                assert!(contract
                    .normalize(&json!({"values":values}), "args")
                    .is_err());
            }
        }
    }
}
