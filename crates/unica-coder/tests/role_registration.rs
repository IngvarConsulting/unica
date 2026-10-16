//! The real registration macros reject untyped Role and Support entries.
use quote::ToTokens;
use std::process::{Command, Output};

fn compile_registration(entry: &str) -> Output {
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
    let fixture = format!(
        r#"
        #![allow(dead_code)]
        enum OperationFamily {{ Role, Support, Metadata }}
        enum OperationApplicability {{ Configuration, SupportRule }}
        enum OperationSkeleton {{ Values }}
        enum RoleOperation {{ Create }}
        enum SupportOperation {{ CapabilitySet, RuleSet }}
        enum TypedOperation {{ Role(RoleOperation), Support(SupportOperation) }}
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
        }}
        {macros}
        operation_descriptors!({entry});
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
