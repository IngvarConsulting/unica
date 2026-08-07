use super::{MetaCollection, MetaPropertyKey, MetadataKind};

/// Условие, доказуемое по самому объекту.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MetaObjectRequirement {
    /// Непусто хотя бы одна из перечисленных коллекций.
    AnyCollectionNonEmpty(&'static [MetaCollection]),
    /// Корневое свойство непусто.
    PropertyNonEmpty(MetaPropertyKey),
}

/// Запись заводится только под наблюдённое сообщение платформы: условие,
/// выведенное из рассуждения, проверить нечем, и запретить законный объект оно
/// способно так же, как пропустить сломанный (ADR-0030).
#[derive(Debug, Clone, Copy)]
pub(crate) struct MetaObjectIntegrityRule {
    pub(crate) kinds: &'static [MetadataKind],
    pub(crate) requirement: MetaObjectRequirement,
    pub(crate) platform_message: &'static str,
}

pub(crate) const META_OBJECT_INTEGRITY_RULES: &[MetaObjectIntegrityRule] = &[
    MetaObjectIntegrityRule {
        kinds: &[
            MetadataKind::InformationRegister,
            MetadataKind::AccumulationRegister,
        ],
        requirement: MetaObjectRequirement::AnyCollectionNonEmpty(&[
            MetaCollection::Dimensions,
            MetaCollection::Resources,
            MetaCollection::Attributes,
        ]),
        platform_message: "Register without dimensions, resources, and attributes",
    },
    MetaObjectIntegrityRule {
        kinds: &[MetadataKind::WebService],
        requirement: MetaObjectRequirement::PropertyNonEmpty(MetaPropertyKey::Namespace),
        platform_message: "Empty name space",
    },
];

pub(crate) fn meta_object_integrity_rules(
    kind: MetadataKind,
) -> impl Iterator<Item = &'static MetaObjectIntegrityRule> {
    META_OBJECT_INTEGRITY_RULES
        .iter()
        .filter(move |rule| rule.kinds.contains(&kind))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_rule_quotes_the_platform_message_it_was_derived_from() {
        assert!(!META_OBJECT_INTEGRITY_RULES.is_empty());
        for rule in META_OBJECT_INTEGRITY_RULES {
            assert!(!rule.kinds.is_empty(), "rule without kinds");
            assert!(
                !rule.platform_message.trim().is_empty(),
                "rule without the platform message it was derived from"
            );
        }
    }

    #[test]
    fn first_table_covers_the_two_observed_platform_rejections() {
        let registers =
            meta_object_integrity_rules(MetadataKind::InformationRegister).collect::<Vec<_>>();
        assert_eq!(registers.len(), 1);
        assert_eq!(
            registers[0].platform_message,
            "Register without dimensions, resources, and attributes"
        );
        assert!(matches!(
            registers[0].requirement,
            MetaObjectRequirement::AnyCollectionNonEmpty(_)
        ));

        let services = meta_object_integrity_rules(MetadataKind::WebService).collect::<Vec<_>>();
        assert_eq!(services.len(), 1);
        assert_eq!(services[0].platform_message, "Empty name space");

        assert_eq!(
            meta_object_integrity_rules(MetadataKind::Catalog).count(),
            0
        );
    }
}
