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
fn nested_profiles_are_closed_by_source_owner_version_and_exact_member_context() {
    use crate::{MetaMemberReadContext, MetadataKind};
    use unica_source_contract::SourceSetKind;
    for (owner, member, field, context, count) in [
        (
            MetadataKind::Document,
            NodeKind::Command,
            "commands",
            MetaMemberReadContext::DocumentCommand,
            3,
        ),
        (
            MetadataKind::InformationRegister,
            NodeKind::Dimension,
            "dimensions",
            MetaMemberReadContext::InformationRegisterDimension,
            4,
        ),
        (
            MetadataKind::AccumulationRegister,
            NodeKind::Dimension,
            "dimensions",
            MetaMemberReadContext::AccumulationRegisterDimension,
            1,
        ),
        (
            MetadataKind::DocumentJournal,
            NodeKind::Column,
            "columns",
            MetaMemberReadContext::DocumentJournalColumn,
            1,
        ),
    ] {
        let profile =
            MetaMemberReadProfile::for_source(SourceSetKind::Configuration, owner, Some("2.20"))
                .unwrap();
        assert_eq!(profile.owner_kind(), owner);
        assert_eq!(profile.properties(context).count(), count);
        for source in [
            SourceSetKind::Extension,
            SourceSetKind::ExternalProcessor,
            SourceSetKind::ExternalReport,
        ] {
            assert!(MetaMemberReadProfile::for_source(source, owner, Some("2.20")).is_none());
        }
        for version in [None, Some("2.19"), Some("2.21"), Some("02.20")] {
            assert!(MetaMemberReadProfile::for_source(
                SourceSetKind::Configuration,
                owner,
                version
            )
            .is_none());
        }
        let at = QualifiedAddress::parse(&format!(
            "main:{}.Owner.{}.Member",
            owner.as_str(),
            member.as_str()
        ))
        .unwrap();
        assert_eq!(profile.context_for_address(&at, member), Some(context));
        assert_eq!(
            profile.context_for_field(&format!("collections.{field}[12]")),
            Some(context)
        );
        for spec in profile.properties(context) {
            let diagnostic = format!("collections.{field}[12].properties.{}", spec.name);
            assert!(profile.owns_diagnostic(&diagnostic));
            assert!(!profile.owns_diagnostic(&format!("{diagnostic}.extra")));
        }
        for field in [
            format!("collections.{field}[]"),
            format!("collections.{field}[x]"),
            format!("collections.{field}[0].attributes[1]"),
            "collections.attributes[0]".into(),
        ] {
            assert!(profile.context_for_field(&field).is_none(), "{field}");
        }
        for at in [
            format!("main:{}.Owner.{}", owner.as_str(), member.as_str()),
            format!("main:Catalog.Owner.{}.Member", member.as_str()),
            format!(
                "main:{}.Owner.Form.Main.{}.Member",
                owner.as_str(),
                member.as_str()
            ),
        ] {
            assert!(
                profile
                    .context_for_address(&QualifiedAddress::parse(&at).unwrap(), member)
                    .is_none(),
                "{at}"
            );
        }
    }
}

#[test]
fn nested_profile_scalar_domains_preserve_false_and_accept_native_group_forms() {
    use crate::{MetaMemberReadContext, MetaObservedPropertyValue};
    let command = MetaMemberReadProfile::DocumentCommand220;
    let group = *command
        .properties(MetaMemberReadContext::DocumentCommand)
        .find(|spec| spec.name == "Group")
        .unwrap();
    for value in [
        "NavigationPanelImportant",
        "NavigationPanelOrdinary",
        "NavigationPanelSeeAlso",
        "FormNavigationPanelGoTo",
        "FormNavigationPanelImportant",
        "FormCommandBarImportant",
        "FormCommandBarCreateBasedOn",
        "ActionsPanelCreate",
        "ActionsPanelTools",
        "CommandGroup.Custom",
    ] {
        let parsed = group.parse_text(value).unwrap();
        assert!(group.accepts(&parsed), "{value}");
    }
    for invalid in [
        "",
        "Unknown",
        "NavigationPanelImportant ",
        "CommandGroup",
        "Catalog.Custom",
        "CommandGroup.X.Extra",
    ] {
        assert!(group.parse_text(invalid).is_none(), "{invalid}");
    }
    let modifies = *command
        .properties(MetaMemberReadContext::DocumentCommand)
        .find(|spec| spec.name == "ModifiesData")
        .unwrap();
    for (literal, value) in [("true", true), ("false", false)] {
        let expected = MetaObservedPropertyValue::Boolean { value };
        assert_eq!(modifies.parse_text(literal), Some(expected.clone()));
        assert!(modifies.accepts(&expected));
    }
    for invalid in ["", "0", "1", "False", " false", "true "] {
        assert!(modifies.parse_text(invalid).is_none());
    }
    assert!(!modifies.accepts(&MetaObservedPropertyValue::Text {
        value: "false".into()
    }));
}
