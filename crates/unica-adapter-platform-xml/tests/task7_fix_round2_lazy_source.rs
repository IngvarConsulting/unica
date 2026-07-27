use std::{
    fs::{self, File},
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use unica_adapter_platform_xml::PlatformXmlAdapterFactory;
use unica_format_core::{
    ports::{
        AuthorabilityRequest, AuthorabilityRequirement, AuthorabilityResult, CompatibilityRequest,
        OwnerResolutionMode, ValidationContextRequest,
    },
    source::{ConfiguredSourceSetKind, SourceContext, SourceFamily, SourceLocation},
};

fn temp_root(label: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!(
        "unica-task7-fix2-{label}-{}-{nonce}",
        std::process::id()
    ))
}

fn owner(root: &Path) -> PathBuf {
    fs::create_dir_all(root).unwrap();
    let owner = root.join("Configuration.xml");
    fs::write(
        &owner,
        r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Configuration uuid="00000000-0000-0000-0000-000000000001"><Properties><Name>Main</Name></Properties><ChildObjects/></Configuration></MetaDataObject>"#,
    )
    .unwrap();
    owner
}

fn source(root: &Path, target: &Path) -> SourceContext {
    SourceContext::new(
        SourceLocation::new(root.to_path_buf(), root.to_path_buf(), target.to_path_buf()),
        Some("main".to_string()),
        SourceFamily::PlatformXml,
        None,
    )
}

#[test]
fn operational_reads_ignore_more_than_512_unrelated_files_and_sparse_64_mib() {
    let root = temp_root("large");
    let owner = owner(&root);
    let catalogs = root.join("Catalogs");
    fs::create_dir_all(&catalogs).unwrap();
    for index in 0..600u32 {
        fs::write(
            catalogs.join(format!("Catalog{index:04}.xml")),
            format!(
                r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Catalog uuid="00000000-0000-0000-0000-{index:012x}"><Properties><Name>Catalog{index:04}</Name></Properties><ChildObjects/></Catalog></MetaDataObject>"#
            ),
        )
        .unwrap();
    }
    let unrelated = root.join("unrelated");
    fs::create_dir_all(&unrelated).unwrap();
    for index in 0..600 {
        fs::write(unrelated.join(format!("{index:04}.bin")), b"not source").unwrap();
    }
    File::create(unrelated.join("large.sparse"))
        .unwrap()
        .set_len(70 * 1024 * 1024)
        .unwrap();

    let factory = PlatformXmlAdapterFactory::new();
    let session =
        factory.capture_operational_source(&source(&root, &owner), OwnerResolutionMode::Existing);
    let registration = factory.operational_registration();

    assert!(registration
        .compatibility()
        .inspect(&CompatibilityRequest::new(vec![session.clone()]).unwrap())
        .unwrap()
        .issue()
        .is_none());
    assert!(matches!(
        registration
            .authorability()
            .inspect(&AuthorabilityRequest::new(
                session.clone(),
                AuthorabilityRequirement::Editable,
            ))
            .unwrap(),
        AuthorabilityResult::Allowed(_)
    ));
    assert!(registration
        .validation_context()
        .inspect(&ValidationContextRequest::new(session))
        .is_ok());
    assert_eq!(
        factory
            .inspect_source_set(&root, &root, ConfiguredSourceSetKind::Configuration,)
            .unwrap(),
        unica_format_core::ports::SourceSetMatch::Match
    );

    fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn artifact_replacement_after_capture_is_not_satisfied_by_cached_whole_tree_bytes() {
    use std::os::unix::fs::symlink;

    let root = temp_root("swap");
    let owner = owner(&root);
    let outside = temp_root("outside");
    fs::write(
        &outside,
        r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Configuration/></MetaDataObject>"#,
    )
    .unwrap();

    let factory = PlatformXmlAdapterFactory::new();
    let session =
        factory.capture_operational_source(&source(&root, &owner), OwnerResolutionMode::Existing);
    fs::rename(&owner, root.join("owner.original")).unwrap();
    symlink(&outside, &owner).unwrap();

    let result = factory
        .operational_registration()
        .compatibility()
        .inspect(&CompatibilityRequest::new(vec![session]).unwrap())
        .unwrap();
    assert!(
        result.issue().is_some(),
        "operation must reopen through the authorized root and reject the swapped name"
    );

    fs::remove_file(owner).unwrap();
    fs::remove_dir_all(root).unwrap();
    fs::remove_file(outside).unwrap();
}

#[test]
fn tree_session_binds_selected_bytes_and_membership_but_ignores_unrelated_files() {
    let root = temp_root("tree-evidence");
    owner(&root);
    let catalogs = root.join("Catalogs");
    fs::create_dir_all(&catalogs).unwrap();
    let catalog = catalogs.join("Items.xml");
    fs::write(
        &catalog,
        r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Catalog uuid="00000000-0000-0000-0000-000000000002"><Properties><Name>Items</Name></Properties><ChildObjects/></Catalog></MetaDataObject>"#,
    )
    .unwrap();

    let factory = PlatformXmlAdapterFactory::new();
    let port = factory.operational_registration();
    let session = factory.capture_unscoped_tree_source(&root, &root, OwnerResolutionMode::Existing);
    let inspect = || {
        port.compatibility()
            .inspect(&CompatibilityRequest::new(vec![session.clone()]).unwrap())
            .unwrap()
    };
    assert!(inspect().issue().is_none());

    fs::write(root.join("unrelated.bin"), b"ignored").unwrap();
    let after_unrelated = inspect();
    assert!(
        after_unrelated.issue().is_none(),
        "unrelated files must not invalidate selected source evidence: {after_unrelated:?}"
    );

    fs::write(
        &catalog,
        r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Catalog uuid="00000000-0000-0000-0000-000000000002"><Properties><Name>Changed</Name></Properties><ChildObjects/></Catalog></MetaDataObject>"#,
    )
    .unwrap();
    assert!(
        inspect().issue().is_some(),
        "in-place selected-byte changes must invalidate bound evidence"
    );

    let membership_session =
        factory.capture_unscoped_tree_source(&root, &root, OwnerResolutionMode::Existing);
    let membership_request =
        || CompatibilityRequest::new(vec![membership_session.clone()]).unwrap();
    assert!(port
        .compatibility()
        .inspect(&membership_request())
        .unwrap()
        .issue()
        .is_none());
    fs::write(
        catalogs.join("Added.xml"),
        r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Catalog uuid="00000000-0000-0000-0000-000000000003"><Properties><Name>Added</Name></Properties><ChildObjects/></Catalog></MetaDataObject>"#,
    )
    .unwrap();
    assert!(
        port.compatibility()
            .inspect(&membership_request())
            .unwrap()
            .issue()
            .is_some(),
        "selected-entry additions must invalidate bound directory evidence"
    );

    fs::remove_dir_all(root).unwrap();
}
