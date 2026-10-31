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
fn accumulation_totals_profile_is_exactly_configuration_dimensions() {
    use super::*;
    let p = MetaMemberReadProfile::AccumulationRegisterDimension220;
    assert!(p.context_for_field("collections.dimensions[0]").is_some());
    for field in [
        "effect",
        "collections.resources[0]",
        "collections.dimensions[0].attributes[0]",
    ] {
        assert!(p.context_for_field(field).is_none());
    }
    for (at, allowed) in [
        ("main:AccumulationRegister.Facts.Dimension.Item", true),
        ("main:InformationRegister.Facts.Dimension.Item", false),
        ("main:AccumulationRegister.Facts.Resource.Item", false),
    ] {
        assert_eq!(
            p.context_for_address(&QualifiedAddress::parse(at).unwrap(), NodeKind::Dimension)
                .is_some(),
            allowed
        );
    }
    for kind in [
        SourceSetKind::Configuration,
        SourceSetKind::Extension,
        SourceSetKind::ExternalReport,
    ] {
        assert_eq!(
            MetaMemberReadProfile::for_source(
                kind,
                MetadataKind::AccumulationRegister,
                Some("2.20")
            )
            .is_some(),
            kind == SourceSetKind::Configuration
        );
    }
}
