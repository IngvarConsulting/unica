use super::*;
use crate::infrastructure::v13_large_configuration::RegistrationCache;
use unica_workspace::test_support::retained_read_probe::{RetainedRead, RetainedReadProbe};

const OWNER: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Catalog uuid="0b3c7d33-3f43-4d6e-9d39-0c8b3f2bc8a1"><Properties><Name>Products</Name></Properties><ChildObjects><Form>Main</Form></ChildObjects></Catalog></MetaDataObject>"#;

fn diagnostics(
    source: &std::path::Path,
    artifacts: &Arc<RegistrationCache>,
) -> Vec<unica_metadata::MetaDiagnostic> {
    ProviderReadAuthority::new(
        "main",
        "actor-fixture-main",
        SourceSetKind::Configuration,
        Arc::new(unica_workspace::read::open_source_directory(source).unwrap()),
    )
    .with_artifacts(Arc::clone(artifacts))
    .child_resource_diagnostics(
        &MetadataAddress::parse_logical("Catalog.Products").unwrap(),
        "Catalog",
        "Products",
        OWNER.as_bytes(),
        &mut || Ok(()),
    )
    .unwrap()
}

/// How many times the form descriptor was opened: once to stamp it for the
/// revision, once more when the check itself reads it.
fn descriptor_opens(probe: &RetainedReadProbe, source: &std::path::Path) -> usize {
    let descriptor = std::path::Path::new("Catalogs/Products/Forms/Main.xml");
    probe
        .take()
        .into_iter()
        .filter(|read| read.relative_to(source) == RetainedRead::Opened(descriptor.into()))
        .count()
}

/// `view` repeats the child resource check of an unchanged object from the
/// retained result; a changed child resource is checked again at once.
/// rule: INV.APP.META-OBSERVATION
#[test]
fn child_resource_check_is_retained_until_a_child_resource_changes() {
    let root = tempfile::tempdir().unwrap();
    let source = fs::canonicalize(root.path()).unwrap();
    let forms = source.join("Catalogs/Products/Forms");
    fs::create_dir_all(&forms).unwrap();
    fs::write(source.join("Catalogs/Products.xml"), OWNER).unwrap();
    fs::write(forms.join("Main.xml"), "not a form descriptor").unwrap();
    let artifacts = Arc::new(RegistrationCache::with_settle_window_for_test(
        std::time::Duration::ZERO,
    ));
    let probe = RetainedReadProbe::watch(&source);

    let first = diagnostics(&source, &artifacts);
    assert_eq!(descriptor_opens(&probe, &source), 2);
    let unavailable = first
        .iter()
        .find(|diagnostic| {
            diagnostic.code == unica_metadata::MetaDiagnosticCode::ProviderUnavailable
        })
        .unwrap_or_else(|| panic!("{first:?}"));
    assert_eq!(
        unavailable.metadata_path.as_ref().map(MetadataAddress::as_str),
        Some("Catalog.Products.Form.Main")
    );

    assert_eq!(diagnostics(&source, &artifacts), first);
    assert_eq!(
        descriptor_opens(&probe, &source),
        1,
        "an unchanged object is not checked again"
    );

    fs::create_dir_all(forms.join("Main")).unwrap();
    fs::write(forms.join("Main/unexpected.bin"), b"unexpected").unwrap();
    diagnostics(&source, &artifacts);
    assert_eq!(
        descriptor_opens(&probe, &source),
        2,
        "a new file in the form directory is checked again"
    );

    let unsettled = Arc::new(RegistrationCache::default());
    diagnostics(&source, &unsettled);
    assert_eq!(descriptor_opens(&probe, &source), 2);
    diagnostics(&source, &unsettled);
    assert_eq!(
        descriptor_opens(&probe, &source),
        2,
        "a file younger than the settle window is checked every time"
    );
}

/// Cancellation at any point of the check refuses the read instead of
/// turning into a resource error, and leaves nothing retained.
#[test]
fn cancelled_child_resource_check_refuses_the_read() {
    let root = tempfile::tempdir().unwrap();
    let source = fs::canonicalize(root.path()).unwrap();
    let forms = source.join("Catalogs/Products/Forms");
    fs::create_dir_all(forms.join("Main/Ext")).unwrap();
    fs::write(source.join("Catalogs/Products.xml"), OWNER).unwrap();
    fs::write(forms.join("Main.xml"), "not a form descriptor").unwrap();
    fs::write(forms.join("Main/Ext/Form.xml"), "form").unwrap();
    let target = MetadataAddress::parse_logical("Catalog.Products").unwrap();
    let run = |artifacts: &Arc<RegistrationCache>, stop_at: usize| {
        let mut calls = 0;
        let result = ProviderReadAuthority::new(
            "main",
            "actor-fixture-main",
            SourceSetKind::Configuration,
            Arc::new(unica_workspace::read::open_source_directory(&source).unwrap()),
        )
        .with_artifacts(Arc::clone(artifacts))
        .child_resource_diagnostics(&target, "Catalog", "Products", OWNER.as_bytes(), &mut || {
            calls += 1;
            if calls == stop_at {
                Err(ViewError::new(RefusalCode::Cancelled, "cancelled"))
            } else {
                Ok(())
            }
        });
        (result, calls)
    };
    let settled = || {
        Arc::new(RegistrationCache::with_settle_window_for_test(
            std::time::Duration::ZERO,
        ))
    };
    let (complete, total) = run(&settled(), 0);
    complete.unwrap();
    assert!(total > 2, "{total}");
    for stop_at in 1..=total {
        let artifacts = settled();
        let (refused, _) = run(&artifacts, stop_at);
        assert_eq!(
            refused.unwrap_err().code(),
            RefusalCode::Cancelled,
            "stop_at={stop_at}"
        );
        let (again, calls) = run(&artifacts, 0);
        again.unwrap();
        assert_eq!(calls, total, "a cancelled check retains nothing (stop_at={stop_at})");
    }
}
