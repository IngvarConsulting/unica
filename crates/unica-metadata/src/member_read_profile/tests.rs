use super::MetaMemberReadProfile;
use unica_address::address::{NodeKind, QualifiedAddress};

#[test]
fn member_profile_context_requires_the_complete_named_catalog_path() {
    let profile = MetaMemberReadProfile::CatalogTabularSection220;
    for (at, kind) in [
        (
            "main:Catalog.Items.TabularSection.Lines",
            NodeKind::TabularSection,
        ),
        (
            "main:Catalog.Items.TabularSection.Lines.Attribute.Column",
            NodeKind::Attribute,
        ),
    ] {
        let address = QualifiedAddress::parse(at).unwrap();
        assert!(
            profile.context_for_address(&address, kind).is_some(),
            "{at}"
        );
    }
    let header = QualifiedAddress::parse("main:Catalog.Items.Attribute.Header").unwrap();
    let context = profile
        .context_for_address(&header, NodeKind::Attribute)
        .unwrap();
    assert_eq!(
        profile
            .properties(context)
            .map(|spec| spec.name)
            .collect::<Vec<_>>(),
        vec!["FillFromFillingValue"]
    );
    for name in [
        "LineNumberLength",
        "Use",
        "Indexing",
        "FullTextSearch",
        "DataHistory",
        "ToolTip",
    ] {
        assert!(
            !profile.owns_diagnostic(&format!("collections.attributes[0].properties.{name}")),
            "catalog tabular property leaked into header: {name}"
        );
    }
    for (at, kind) in [
        ("main:Catalog.Items", NodeKind::Catalog),
        (
            "main:Catalog.Items.TabularSection",
            NodeKind::TabularSection,
        ),
        (
            "main:Catalog.Items.TabularSection.Lines.Attribute",
            NodeKind::Attribute,
        ),
        (
            "main:Catalog.Items.TabularSection.Lines.StandardAttribute.LineNumber",
            NodeKind::StandardAttribute,
        ),
        (
            "main:Catalog.Items.Form.Edit.Attribute.Column",
            NodeKind::Attribute,
        ),
        (
            "main:Document.Items.TabularSection.Lines.Attribute.Column",
            NodeKind::Attribute,
        ),
        (
            "main:Catalog.Items.TabularSection.Lines.Attribute.Column.Attribute.Deep",
            NodeKind::Attribute,
        ),
        (
            "main:Catalog.Items.TabularSection.Lines.Attribute.Column",
            NodeKind::Column,
        ),
    ] {
        let address = QualifiedAddress::parse(at).unwrap();
        assert!(
            profile.context_for_address(&address, kind).is_none(),
            "{at}"
        );
    }
}

#[test]
fn member_profile_owns_only_exact_context_property_diagnostics() {
    let profile = MetaMemberReadProfile::CatalogTabularSection220;
    for field in [
        "collections.tabularSections[0].properties.LineNumberLength",
        "collections.tabularSections[12].properties.Use",
        "collections.tabularSections[0].attributes[1].properties.Indexing",
        "collections.tabularSections[12].attributes[34].properties.FullTextSearch",
        "collections.tabularSections[0].attributes[1].properties.ToolTip",
    ] {
        assert!(profile.owns_diagnostic(field), "lost owned field {field}");
    }
    for field in [
        "properties.Indexing",
        "collections.attributes[0].properties.Indexing",
        "collections.tabularSections[0].properties.Indexing",
        "collections.tabularSections[0].properties.ToolTip",
        "collections.tabularSections[0].attributes[1].properties.Use",
        "collections.tabularSections[0].attributes[1].properties.LineNumberLength",
        "collections.tabularSections[0].standardAttributes[1].properties.Indexing",
        "collections.tabularSections[0].standardAttributes[1].properties.ToolTip",
        "collections.tabularSections[0].attributes[1].attributes[2].properties.Indexing",
        "collections.tabularSections[0].attributes[1].other.properties.Indexing",
        "collections.tabularSections[0].other.attributes[1].properties.Indexing",
        "collections.tabularSections[].attributes[1].properties.Indexing",
        "collections.tabularSections[bad].attributes[1].properties.Indexing",
        "collections.tabularSections[0].attributes[-1].properties.Indexing",
        "collections.tabularSections[0].attributes[1x].properties.Indexing",
        "collections.tabularSections[0].attributes[1].properties.Indexings",
        "collections.tabularSections[0].attributes[1].properties.Indexing.extra",
        "collections.tabularSections[0].attributes[1].properties.ToolTips",
        "collections.tabularSections[0].attributes[1].properties.ToolTip.extra",
        "prefix.collections.tabularSections[0].attributes[1].properties.Indexing",
    ] {
        assert!(
            !profile.owns_diagnostic(field),
            "claimed foreign field {field}"
        );
    }
}

#[test]
fn filling_read_profiles_admit_exact_thirteen_owner_contexts() {
    use crate::{MetaMemberReadProfile, MetadataKind};
    use unica_source_contract::SourceSetKind;
    let expected = [
        ("Catalog", "attributes", false),
        ("Document", "attributes", false),
        ("ExchangePlan", "attributes", false),
        ("ChartOfAccounts", "attributes", false),
        ("ChartOfCharacteristicTypes", "attributes", false),
        ("ChartOfCalculationTypes", "attributes", false),
        ("BusinessProcess", "attributes", false),
        ("Task", "attributes", false),
        ("Report", "attributes", true),
        ("DataProcessor", "attributes", true),
        ("InformationRegister", "attributes", false),
        ("InformationRegister", "dimensions", false),
        ("InformationRegister", "resources", false),
    ];
    let mut actual = 0;
    for owner in MetadataKind::ALL {
        for section in [false, true] {
            for (collection, kind) in [
                ("attributes", NodeKind::Attribute),
                ("dimensions", NodeKind::Dimension),
                ("resources", NodeKind::Resource),
            ] {
                let field = if section {
                    format!("collections.tabularSections[0].{collection}[0]")
                } else {
                    format!("collections.{collection}[0]")
                };
                let address = QualifiedAddress::parse(&if section {
                    format!(
                        "main:{}.Sample.TabularSection.Rows.{}.Field",
                        owner.as_str(),
                        kind.as_str()
                    )
                } else {
                    format!("main:{}.Sample.{}.Field", owner.as_str(), kind.as_str())
                })
                .unwrap();
                let profile = MetaMemberReadProfile::for_source(
                    SourceSetKind::Configuration,
                    *owner,
                    Some("2.20"),
                );
                let has = profile
                    .and_then(|p| p.context_for_field(&field).map(|c| (p, c)))
                    .is_some_and(|(p, c)| {
                        p.properties(c).any(|s| s.name == "FillFromFillingValue")
                    });
                let addressed = profile
                    .and_then(|p| p.context_for_address(&address, kind).map(|c| (p, c)))
                    .is_some_and(|(p, c)| {
                        p.properties(c).any(|s| s.name == "FillFromFillingValue")
                    });
                let wanted = expected.contains(&(owner.as_str(), collection, section));
                assert_eq!(has, wanted, "{field} {owner:?}");
                assert_eq!(addressed, wanted, "{address}");
                actual += usize::from(has);
            }
        }
        for kind in [
            SourceSetKind::Extension,
            SourceSetKind::ExternalReport,
            SourceSetKind::ExternalProcessor,
        ] {
            assert!(MetaMemberReadProfile::for_source(kind, *owner, Some("2.20")).is_none());
        }
        assert!(MetaMemberReadProfile::for_source(
            SourceSetKind::Configuration,
            *owner,
            Some("2.19")
        )
        .is_none());
    }
    assert_eq!(actual, 13);
}
