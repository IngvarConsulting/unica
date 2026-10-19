//! The real registration macros reject raw tuples for typed operation families.
//! Registry completeness and descriptor consistency use the actual registry tests
//! in domain::apply; these smaller compile fixtures exercise registration syntax.
use quote::ToTokens;
use std::process::{Command, Output};

const METADATA_DIAGNOSTIC: &str =
    "Metadata operations must register a typed MetadataOperation with a mandatory contextual contract";

/// Build the complete typed probe from the real table rather than maintaining
/// another list of the migrated operation identities.
fn complete_metadata_entries() -> String {
    let source = syn::parse_file(include_str!("../src/domain/metadata_operation.rs")).unwrap();
    let invocation = source
        .items
        .iter()
        .find_map(|item| match item {
            syn::Item::Macro(item) if item.mac.path.is_ident("metadata_operations") => {
                Some(&item.mac.tokens)
            }
            _ => None,
        })
        .expect("exercise the production metadata table");
    struct Row(syn::Ident);
    impl syn::parse::Parse for Row {
        fn parse(input: syn::parse::ParseStream<'_>) -> syn::Result<Self> {
            let variant = input.parse()?;
            input.parse::<syn::Token![=>]>()?;
            let fields;
            syn::parenthesized!(fields in input);
            fields.parse::<syn::LitStr>()?;
            fields.parse::<syn::Token![,]>()?;
            fields.parse::<syn::Ident>()?;
            fields.parse::<syn::Token![,]>()?;
            fields.parse::<syn::Ident>()?;
            if !fields.is_empty() {
                return Err(fields.error("metadata rows have exactly three fields"));
            }
            Ok(Self(variant))
        }
    }
    let rows = syn::parse::Parser::parse2(
        syn::punctuated::Punctuated::<Row, syn::Token![,]>::parse_terminated,
        invocation.clone(),
    )
    .unwrap();
    assert!(
        !rows.is_empty(),
        "a complete typed probe must contain operations"
    );
    rows.iter()
        .map(|row| format!("(MetadataOperation::{})", row.0))
        .collect::<Vec<_>>()
        .join(",")
}

fn metadata_declaration(source: &str, name: &str) -> String {
    let source = syn::parse_file(source).unwrap();
    let items = source
        .items
        .iter()
        .filter_map(|item| match item {
            syn::Item::Macro(item)
                if item.ident.as_ref().is_some_and(|ident| ident == name)
                    || item.mac.path.is_ident(name) =>
            {
                Some(item.to_token_stream().to_string())
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        items.len(),
        2,
        "exercise the production generator and table"
    );
    items.join("\n")
}

fn compile_registration(entry: &str) -> Output {
    compile_registration_with_declarations(
        entry,
        &metadata_declaration(
            include_str!("../src/domain/metadata_operation.rs"),
            "metadata_operations",
        ),
        &metadata_declaration(
            include_str!("../src/domain/form_operation.rs"),
            "form_operations",
        ),
    )
}

fn compile_registration_with_declarations(entry: &str, metadata: &str, form: &str) -> Output {
    let source = syn::parse_file(include_str!("../src/domain/apply.rs")).unwrap();
    let macros = source
        .items
        .iter()
        .filter_map(|item| match item {
            syn::Item::Macro(item)
                if item.ident.as_ref().is_some_and(|name| {
                    name == "operation_descriptor" || name == "operation_descriptors"
                }) =>
            {
                Some(item.to_token_stream().to_string())
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(macros.len(), 2, "exercise both production macros");
    let name_comparison = source
        .items
        .iter()
        .find_map(|item| match item {
            syn::Item::Fn(item) if item.sig.ident == "operation_names_equal" => {
                Some(item.to_token_stream().to_string())
            }
            _ => None,
        })
        .expect("exercise the production const name comparison");
    let fixture = format!(
        r#"
        #![allow(dead_code)]
        mod domain {{
        mod metadata_operation {{
            use super::apply::{{OperationApplicability, OperationSkeleton}};
            {metadata}
        }}
        mod form_operation {{
            use super::apply::{{OperationApplicability, OperationSkeleton}};
            {form}
        }}
        mod apply {{
        use super::metadata_operation::MetadataOperation;
        use super::form_operation::FormOperation;
        enum OperationFamily {{ Role, Support, Metadata, Form }}
        pub(super) enum OperationApplicability {{
            Configuration, ExtensionRoot, Metadata, SupportRule, MetadataAttributeContainer,
            MetadataAttribute, MetadataCommands, MetadataCommand, MetadataForms,
            Form, FormItemContainer, FormItem, FormEvent,
            MetadataTabularSections, MetadataTabularSection,
            MetadataDimensions, MetadataDimension, MetadataResources, MetadataResource,
            MetadataEnumValues, MetadataEnumValue, MetadataColumns, MetadataColumn,
            MetadataTemplates, MetadataTemplate, MetadataPredefinedItems, MetadataPredefinedItem,
        }}
        pub(super) enum OperationSkeleton {{ Values, Items, Target, From }}
        enum RoleOperation {{ Create }}
        enum SupportOperation {{ CapabilitySet, RuleSet }}
        enum TypedOperation {{
            Role(RoleOperation), Support(SupportOperation),
            Metadata(MetadataOperation), Form(FormOperation),
        }}
        struct OperationDescriptor {{
            name: &'static str,
            family: OperationFamily,
            applicability: OperationApplicability,
            skeleton: OperationSkeleton,
            typed_operation: Option<TypedOperation>,
        }}
        impl OperationDescriptor {{
            const fn role(operation: RoleOperation) -> Self {{ Self {{
                name: "role.create", family: OperationFamily::Role,
                applicability: OperationApplicability::Configuration,
                skeleton: OperationSkeleton::Values,
                typed_operation: Some(TypedOperation::Role(operation)),
            }} }}
            const fn support(operation: SupportOperation) -> Self {{ Self {{
                name: "supportCapability.set", family: OperationFamily::Support,
                applicability: OperationApplicability::Configuration,
                skeleton: OperationSkeleton::Values,
                typed_operation: Some(TypedOperation::Support(operation)),
            }} }}
            const fn metadata(operation: MetadataOperation) -> Self {{ Self {{
                name: operation.name(), family: OperationFamily::Metadata,
                applicability: operation.applicability(), skeleton: operation.skeleton(),
                typed_operation: Some(TypedOperation::Metadata(operation)),
            }} }}
            const fn form(operation: FormOperation) -> Self {{ Self {{
                name: operation.name(), family: OperationFamily::Form,
                applicability: operation.applicability(), skeleton: operation.skeleton(),
                typed_operation: Some(TypedOperation::Form(operation)),
            }} }}
        }}
        {name_comparison}
        {macros}
        operation_descriptors!({entry});
        }}
        }}
        fn main() {{}}
    "#,
        macros = macros.join("\n"),
        entry = entry
    );
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("registration.rs");
    std::fs::write(&path, fixture).unwrap();
    Command::new("rustc")
        .args(["--edition=2021", "--emit=metadata"])
        .arg(&path)
        .arg("--out-dir")
        .arg(directory.path())
        .output()
        .unwrap()
}

#[test]
fn typed_support_registration_compiles_and_legacy_support_tuple_is_rejected() {
    let typed = compile_registration(
        r#"(RoleOperation::Create), (SupportOperation::CapabilitySet), (SupportOperation::RuleSet), (MetadataOperation::ObjectCreate)"#,
    );
    assert!(
        typed.status.success(),
        "{}",
        String::from_utf8_lossy(&typed.stderr)
    );
    let legacy = compile_registration(r#"("supportRule.set", Support, SupportRule, Values)"#);
    assert!(
        !legacy.status.success(),
        "the old Support registration must fail"
    );
    assert!(
        String::from_utf8_lossy(&legacy.stderr).contains(
            "Support operations must register a typed SupportOperation with a mandatory contract"
        ),
        "{}",
        String::from_utf8_lossy(&legacy.stderr)
    );
}

#[test]
fn typed_role_registration_compiles_and_legacy_role_tuple_is_rejected() {
    let typed =
        compile_registration(r#"(RoleOperation::Create), (MetadataOperation::ObjectCreate)"#);
    assert!(
        typed.status.success(),
        "{}",
        String::from_utf8_lossy(&typed.stderr)
    );
    let legacy = compile_registration(r#"("role.create", Role, Configuration, Values)"#);
    assert!(
        !legacy.status.success(),
        "the old Role registration must fail"
    );
    assert!(
        String::from_utf8_lossy(&legacy.stderr).contains(
            "Role operations must register a typed RoleOperation with a mandatory contract"
        ),
        "{}",
        String::from_utf8_lossy(&legacy.stderr)
    );
}

#[test]
fn typed_contextual_registrations_compile_and_migrated_raw_tuples_are_rejected() {
    let typed = compile_registration(
        r#"(MetadataOperation::AttributeAdd), (MetadataOperation::AttributeSet),
            (MetadataOperation::AttributeRemove), (MetadataOperation::CommandAdd),
            (MetadataOperation::CommandSet), (MetadataOperation::CommandRemove),
            (FormOperation::Add), (FormOperation::Set), (FormOperation::Remove),
            (FormOperation::ItemAdd), (FormOperation::ItemRemove), (FormOperation::ItemMove),
            (FormOperation::AttributeAdd), (FormOperation::CommandAdd),
            (FormOperation::EventBind),
            (MetadataOperation::ObjectCreate)"#,
    );
    assert!(
        typed.status.success(),
        "{}",
        String::from_utf8_lossy(&typed.stderr)
    );
    for (family, name, applicability, skeleton) in [
        (
            "Metadata",
            "attribute.add",
            "MetadataAttributeContainer",
            "Items",
        ),
        ("Metadata", "attribute.set", "MetadataAttribute", "Values"),
        (
            "Metadata",
            "attribute.remove",
            "MetadataAttribute",
            "Target",
        ),
        ("Metadata", "command.add", "MetadataCommands", "Items"),
        ("Metadata", "command.set", "MetadataCommand", "Values"),
        ("Metadata", "command.remove", "MetadataCommand", "Target"),
        ("Form", "form.add", "MetadataForms", "Items"),
        ("Form", "form.set", "MetadataForms", "Values"),
        ("Form", "form.remove", "Form", "Target"),
        ("Form", "item.add", "FormItemContainer", "Items"),
        ("Form", "item.remove", "FormItem", "Target"),
        ("Form", "item.move", "FormItem", "Values"),
        ("Form", "attribute.add", "Form", "Items"),
        ("Form", "command.add", "Form", "Items"),
        ("Form", "event.bind", "FormEvent", "Values"),
        ("Form", "future.operation", "Form", "Items"),
    ] {
        let legacy = compile_registration(&format!(
            r#"("{name}", {family}, {applicability}, {skeleton})"#
        ));
        assert!(
            !legacy.status.success(),
            "{family} {name} must require its typed registration"
        );
        let expected_diagnostic = if family == "Metadata" {
            METADATA_DIAGNOSTIC
        } else {
            "Form operations must register a typed FormOperation with a mandatory contract"
        };
        assert!(
            String::from_utf8_lossy(&legacy.stderr).contains(expected_diagnostic),
            "{}",
            String::from_utf8_lossy(&legacy.stderr)
        );
    }
}

#[test]
fn inline_metadata_member_registrations_require_their_typed_variants() {
    let operations = [
        (
            "TabularSectionAdd",
            "tabularSection.add",
            "MetadataTabularSections",
            "Items",
        ),
        (
            "TabularSectionSet",
            "tabularSection.set",
            "MetadataTabularSection",
            "Values",
        ),
        (
            "TabularSectionRemove",
            "tabularSection.remove",
            "MetadataTabularSection",
            "Target",
        ),
        (
            "DimensionAdd",
            "dimension.add",
            "MetadataDimensions",
            "Items",
        ),
        (
            "DimensionSet",
            "dimension.set",
            "MetadataDimension",
            "Values",
        ),
        (
            "DimensionRemove",
            "dimension.remove",
            "MetadataDimension",
            "Target",
        ),
        ("ResourceAdd", "resource.add", "MetadataResources", "Items"),
        ("ResourceSet", "resource.set", "MetadataResource", "Values"),
        (
            "ResourceRemove",
            "resource.remove",
            "MetadataResource",
            "Target",
        ),
        (
            "EnumValueAdd",
            "enumValue.add",
            "MetadataEnumValues",
            "Items",
        ),
        (
            "EnumValueSet",
            "enumValue.set",
            "MetadataEnumValue",
            "Values",
        ),
        (
            "EnumValueRemove",
            "enumValue.remove",
            "MetadataEnumValue",
            "Target",
        ),
        ("ColumnAdd", "column.add", "MetadataColumns", "Items"),
        ("ColumnSet", "column.set", "MetadataColumn", "Values"),
        ("ColumnRemove", "column.remove", "MetadataColumn", "Target"),
    ];
    let typed = operations
        .iter()
        .map(|(variant, ..)| format!("(MetadataOperation::{variant})"))
        .collect::<Vec<_>>()
        .join(",");
    let result = compile_registration(&typed);
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    for (_, name, applicability, skeleton) in operations {
        let result = compile_registration(&format!(
            r#"("{name}", Metadata, {applicability}, {skeleton})"#
        ));
        assert!(
            !result.status.success(),
            "{name} must require its typed registration"
        );
        assert!(
            String::from_utf8_lossy(&result.stderr).contains(METADATA_DIAGNOSTIC),
            "{name}: {}",
            String::from_utf8_lossy(&result.stderr)
        );
    }
}

#[test]
fn remaining_metadata_registrations_require_their_typed_variants() {
    let operations = [
        ("ObjectBorrow", "object.borrow", "ExtensionRoot", "From"),
        ("ObjectCreate", "object.create", "Configuration", "Values"),
        ("ObjectRemove", "object.remove", "Metadata", "Target"),
        ("RelationAdd", "relation.add", "Metadata", "Values"),
        ("RelationRemove", "relation.remove", "Metadata", "Values"),
        ("RelationReplace", "relation.replace", "Metadata", "Values"),
        ("HelpCreate", "help.create", "Metadata", "Values"),
        ("TemplateAdd", "template.add", "MetadataTemplates", "Items"),
        ("TemplateSet", "template.set", "MetadataTemplate", "Values"),
        (
            "TemplateRemove",
            "template.remove",
            "MetadataTemplate",
            "Target",
        ),
        (
            "PredefinedItemAdd",
            "predefinedItem.add",
            "MetadataPredefinedItems",
            "Items",
        ),
        (
            "PredefinedItemSet",
            "predefinedItem.set",
            "MetadataPredefinedItem",
            "Values",
        ),
        (
            "PredefinedItemRemove",
            "predefinedItem.remove",
            "MetadataPredefinedItem",
            "Target",
        ),
    ];
    let typed = operations
        .iter()
        .map(|(variant, ..)| format!("(MetadataOperation::{variant})"))
        .collect::<Vec<_>>()
        .join(",");
    let result = compile_registration(&typed);
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    for (_, name, applicability, skeleton) in operations {
        let result = compile_registration(&format!(
            r#"("{name}", Metadata, {applicability}, {skeleton})"#
        ));
        assert!(!result.status.success(), "raw {name} must fail");
        assert!(
            String::from_utf8_lossy(&result.stderr).contains(METADATA_DIAGNOSTIC),
            "{name}: {}",
            String::from_utf8_lossy(&result.stderr)
        );
    }
}

#[test]
fn a_complete_typed_metadata_table_does_not_admit_an_additional_raw_entry() {
    let complete = complete_metadata_entries();
    let valid = compile_registration(&complete);
    assert!(
        valid.status.success(),
        "{}",
        String::from_utf8_lossy(&valid.stderr)
    );
    // The family restriction also rejects names that no current identity owns;
    // retaining a complete typed table must not create a raw escape hatch.
    for name in ["object.create", "metadata.future", "renamed.uncontracted"] {
        let result = compile_registration(&format!(
            r#"{complete}, ("{name}", Metadata, Metadata, Values)"#
        ));
        assert!(
            !result.status.success(),
            "raw {name} beside typed Metadata must fail"
        );
        assert!(
            String::from_utf8_lossy(&result.stderr).contains(METADATA_DIAGNOSTIC),
            "{name}: {}",
            String::from_utf8_lossy(&result.stderr)
        );
    }
}

#[test]
fn raw_metadata_guards_follow_literals_while_the_form_family_requires_typed_registration() {
    let metadata = metadata_declaration(
        include_str!("../src/domain/metadata_operation.rs"),
        "metadata_operations",
    );
    let form = metadata_declaration(
        include_str!("../src/domain/form_operation.rs"),
        "form_operations",
    );
    let complete = complete_metadata_entries();
    for (family, applicability, variant, original) in [
        (
            "Metadata",
            "MetadataAttributeContainer",
            "AttributeAdd",
            "attribute.add",
        ),
        (
            "Metadata",
            "MetadataResources",
            "ResourceAdd",
            "resource.add",
        ),
        ("Metadata", "ExtensionRoot", "ObjectBorrow", "object.borrow"),
        ("Metadata", "Configuration", "ObjectCreate", "object.create"),
        ("Metadata", "Metadata", "ObjectRemove", "object.remove"),
        ("Metadata", "Metadata", "RelationAdd", "relation.add"),
        ("Metadata", "Metadata", "HelpCreate", "help.create"),
        (
            "Metadata",
            "MetadataTemplate",
            "TemplateSet",
            "template.set",
        ),
        (
            "Metadata",
            "MetadataPredefinedItem",
            "PredefinedItemRemove",
            "predefinedItem.remove",
        ),
        ("Form", "Form", "AttributeAdd", "attribute.add"),
    ] {
        let probe = format!("{}.probe", original.split_once('.').unwrap().0);
        let changed_metadata = if family == "Metadata" {
            let changed = metadata.replace(&format!("\"{original}\""), &format!("\"{probe}\""));
            assert_ne!(
                changed, metadata,
                "the metadata literal mutation must apply"
            );
            changed
        } else {
            metadata.clone()
        };
        let changed_form = if family == "Form" {
            let changed = form.replace(&format!("\"{original}\""), &format!("\"{probe}\""));
            assert_ne!(changed, form, "the form literal mutation must apply");
            changed
        } else {
            form.clone()
        };
        let entries = if family == "Metadata" {
            complete.clone()
        } else {
            format!("{complete}, (FormOperation::{variant})")
        };
        let typed =
            compile_registration_with_declarations(&entries, &changed_metadata, &changed_form);
        assert!(
            typed.status.success(),
            "{family} renamed literal: {}",
            String::from_utf8_lossy(&typed.stderr)
        );
        for name in [probe.as_str(), original] {
            let result = compile_registration_with_declarations(
                &format!(r#"{entries}, ("{name}", {family}, {applicability}, Items)"#),
                &changed_metadata,
                &changed_form,
            );
            assert!(
                !result.status.success(),
                "{family} {name}: {}",
                String::from_utf8_lossy(&result.stderr)
            );
            let diagnostic = if family == "Metadata" {
                METADATA_DIAGNOSTIC
            } else {
                "Form operations must register a typed FormOperation with a mandatory contract"
            };
            assert!(
                String::from_utf8_lossy(&result.stderr).contains(diagnostic),
                "{family} {name}: {}",
                String::from_utf8_lossy(&result.stderr)
            );
        }
    }
}
