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
    for (at, kind) in [
        ("main:Catalog.Items", NodeKind::Catalog),
        ("main:Catalog.Items.Attribute.Header", NodeKind::Attribute),
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
fn base_dimension_profile_requires_exact_source_owner_format_and_dimension_context() {
    use crate::{MetaMemberReadContext, MetadataKind};
    use unica_source_contract::SourceSetKind;
    let profile = MetaMemberReadProfile::CalculationRegisterDimension220;
    for kind in [
        SourceSetKind::Configuration,
        SourceSetKind::Extension,
        SourceSetKind::ExternalReport,
        SourceSetKind::ExternalProcessor,
    ] {
        for format in [Some("2.20"), Some("2.19"), None] {
            assert_eq!(
                MetaMemberReadProfile::for_source(kind, MetadataKind::CalculationRegister, format),
                (kind == SourceSetKind::Configuration && format == Some("2.20")).then_some(profile)
            );
        }
    }
    assert_eq!(
        profile
            .properties(MetaMemberReadContext::CalculationRegisterDimension)
            .count(),
        1
    );
    assert!(profile
        .context_for_field("collections.dimensions[0]")
        .is_some());
    for field in [
        "collections.dimensions[]",
        "collections.resources[0]",
        "collections.dimensions[0].attributes[0]",
        "collections.tabularSections[0]",
    ] {
        assert!(profile.context_for_field(field).is_none());
    }
    let valid = QualifiedAddress::parse("main:CalculationRegister.Facts.Dimension.Item").unwrap();
    assert!(profile
        .context_for_address(&valid, NodeKind::Dimension)
        .is_some());
    for at in [
        "main:AccumulationRegister.Facts.Dimension.Item",
        "main:CalculationRegister.Facts.Resource.Item",
        "main:CalculationRegister.Facts.Dimension",
    ] {
        assert!(profile
            .context_for_address(&QualifiedAddress::parse(at).unwrap(), NodeKind::Dimension)
            .is_none());
    }
}
