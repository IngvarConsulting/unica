use super::*;

/// rule: INV.SURFACE.CHECK-INFERS-VALIDATORS
#[test]
fn canonical_subsystem_check_validates_the_child_and_rejects_wrong_identity() {
    let fixture = nested_check_fixture();
    let address = QualifiedAddress::parse("main:Subsystem.Sales.Subsystem.Orders").unwrap();
    let path = fixture
        .source
        .join("Subsystems/Sales/Subsystems/Orders.xml");
    let original = fs::read_to_string(&path).unwrap();
    let checked = fixture
        .read_authority()
        .validate_subsystem(&address)
        .unwrap();
    let checked = crate::application::v13::check::normalize_native_outcome(
        &address,
        "Subsystem",
        crate::application::v13::check::CheckValidator::Subsystem,
        checked,
    )
    .unwrap();
    assert!(checked.ok(), "the call failed");
    fs::write(&path, original.replace("Orders", "Other")).unwrap();
    assert_eq!(
        fixture
            .read_authority()
            .validate_subsystem(&address)
            .unwrap_err()
            .code(),
        RefusalCode::ProviderUnavailable
    );
    fixture.cancellation.cancel();
    assert_eq!(
        fixture
            .read_authority()
            .validate_subsystem(&address)
            .unwrap_err()
            .code(),
        RefusalCode::Cancelled
    );
}

/// rule: INV.SURFACE.CHECK-INFERS-VALIDATORS
#[test]
fn canonical_subsystem_check_refuses_linked_dependencies() {
    use unica_workspace::test_support::{
        create_file_link_fixture_for_test, FileLinkFixtureOutcome,
    };
    for dependency in [
        "Orders.xml",
        "Orders/Ext/CommandInterface.xml",
        "Orders/Subsystems/Next.xml",
    ] {
        let fixture = nested_check_fixture();
        let child = fixture
            .source
            .join("Subsystems/Sales/Subsystems/Orders.xml");
        if dependency.ends_with("Next.xml") {
            let text = fs::read_to_string(&child).unwrap().replace(
                "<ChildObjects>",
                "<ChildObjects><Subsystem>Next</Subsystem>",
            );
            fs::write(&child, text).unwrap();
        }
        let path = fixture
            .source
            .join("Subsystems/Sales/Subsystems")
            .join(dependency);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        if path.exists() {
            fs::remove_file(&path).unwrap();
        }
        let foreign = fixture.source.join("foreign.xml");
        fs::write(&foreign, "<CommandInterface version=\"2.20\"/>").unwrap();
        let outcome = create_file_link_fixture_for_test(&foreign, &path).unwrap();
        if outcome != FileLinkFixtureOutcome::Created {
            unica_process::test_support::skip_without_host_capability(
                unica_process::test_support::HostCapability::Symlink,
                "creating a link fixture needs the symlink privilege",
            );
            return;
        }
        let address = QualifiedAddress::parse("main:Subsystem.Sales.Subsystem.Orders").unwrap();
        assert_eq!(
            fixture
                .read_authority()
                .validate_subsystem(&address)
                .unwrap_err()
                .code(),
            RefusalCode::ProviderUnavailable,
            "{dependency}"
        );
    }
}

/// rule: INV.SURFACE.CHECK-INFERS-VALIDATORS
#[test]
fn canonical_subsystem_check_refuses_replaced_root() {
    if !supports_retained_root_replacement_test() {
        unica_process::test_support::skip_without_host_capability(
            unica_process::test_support::HostCapability::RenameUnderOpenHandle,
            "the platform keeps an open handle from being renamed or replaced",
        );
        return;
    }
    let fixture = nested_check_fixture();
    let authority = fixture.read_authority();
    let displaced = fixture.source.with_extension("retained");
    fs::rename(&fixture.source, &displaced).unwrap();
    fs::create_dir_all(&fixture.source).unwrap();
    let address = QualifiedAddress::parse("main:Subsystem.Sales.Subsystem.Orders").unwrap();
    assert_eq!(
        authority.validate_subsystem(&address).unwrap_err().code(),
        RefusalCode::ProviderUnavailable
    );
}

/// rule: INV.SURFACE.CHECK-INFERS-VALIDATORS
#[test]
fn canonical_subsystem_check_preserves_bom_format_warning() {
    for version in ["2.20", "2.21"] {
        let fixture = nested_check_fixture();
        let path = fixture
            .source
            .join("Subsystems/Sales/Subsystems/Orders.xml");
        let text = fs::read_to_string(&path).unwrap().replace("2.20", version);
        fs::write(&path, format!("\u{feff}{text}")).unwrap();
        let address = QualifiedAddress::parse("main:Subsystem.Sales.Subsystem.Orders").unwrap();
        let native = fixture
            .read_authority()
            .validate_subsystem(&address)
            .unwrap();
        let checked = crate::application::v13::check::normalize_native_outcome(
            &address,
            "Subsystem",
            crate::application::v13::check::CheckValidator::Subsystem,
            native,
        )
        .unwrap();
        assert!(checked.ok());
        assert!(!checked
            .diagnostics()
            .iter()
            .any(|diagnostic| diagnostic.code() == "formatVersionInvalid"));
        assert_eq!(
            checked
                .diagnostics()
                .iter()
                .any(|diagnostic| diagnostic.code() == "formatVersionUnverified"),
            version == "2.21"
        );
    }
}
