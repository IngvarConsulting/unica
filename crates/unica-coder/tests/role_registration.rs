//! The real registration macros reject migrated operations registered as raw tuples.
use quote::ToTokens;
use std::process::{Command, Output};

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
            Configuration, SupportRule, MetadataAttributeContainer,
            MetadataAttribute, MetadataCommands, MetadataCommand, Form, FormItemContainer,
        }}
        pub(super) enum OperationSkeleton {{ Values, Items, Target }}
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
        r#"(RoleOperation::Create), (SupportOperation::CapabilitySet), (SupportOperation::RuleSet), ("object.create", Metadata, Configuration, Values)"#,
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
    let typed = compile_registration(
        r#"(RoleOperation::Create), ("object.create", Metadata, Configuration, Values)"#,
    );
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
            (FormOperation::AttributeAdd), (FormOperation::CommandAdd),
            ("object.create", Metadata, Configuration, Values),
            ("item.add", Form, FormItemContainer, Items)"#,
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
        ("Form", "attribute.add", "Form", "Items"),
        ("Form", "command.add", "Form", "Items"),
    ] {
        let legacy = compile_registration(&format!(
            r#"("{name}", {family}, {applicability}, {skeleton})"#
        ));
        assert!(
            !legacy.status.success(),
            "{family} {name} must require its typed registration"
        );
        let expected_diagnostic = if family == "Metadata" {
            "Typed Metadata operations must register a MetadataOperation with a mandatory contextual contract"
        } else {
            "Form attribute.add and command.add must register a typed FormOperation with a mandatory contract"
        };
        assert!(
            String::from_utf8_lossy(&legacy.stderr)
                .contains(&format!("evaluation panicked: {expected_diagnostic}")),
            "{}",
            String::from_utf8_lossy(&legacy.stderr)
        );
    }
}

#[test]
fn raw_contextual_registration_guards_follow_the_actual_literal_table() {
    let metadata = metadata_declaration(
        include_str!("../src/domain/metadata_operation.rs"),
        "metadata_operations",
    );
    let form = metadata_declaration(
        include_str!("../src/domain/form_operation.rs"),
        "form_operations",
    );
    for (family, applicability) in [("Metadata", "MetadataAttributeContainer"), ("Form", "Form")] {
        let changed_metadata = if family == "Metadata" {
            metadata.replace("\"attribute.add\"", "\"attribute.probe\"")
        } else {
            metadata.clone()
        };
        let changed_form = if family == "Form" {
            form.replace("\"attribute.add\"", "\"attribute.probe\"")
        } else {
            form.clone()
        };
        for (name, allowed) in [("attribute.probe", false), ("attribute.add", true)] {
            let result = compile_registration_with_declarations(
                &format!(r#"("{name}", {family}, {applicability}, Items)"#),
                &changed_metadata,
                &changed_form,
            );
            assert_eq!(
                result.status.success(),
                allowed,
                "{family} {name}: {}",
                String::from_utf8_lossy(&result.stderr)
            );
        }
    }
}
