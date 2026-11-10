use super::*;
use crate::infrastructure::v13_large_configuration::RegistrationCache;

const MODULE: &str = "main:CommonModule.РеактивныйСервер";

/// One operation of the actor: a read authority over the fixture sharing the
/// actor's retained artifacts.
fn operation(
    fixture: &RealReaderFixture,
    artifacts: &Arc<RegistrationCache>,
) -> ProviderReadAuthority {
    ProviderReadAuthority::new(
        "main",
        "actor-fixture-main",
        SourceSetKind::Configuration,
        Arc::new(unica_workspace::read::open_source_directory(&fixture.source).unwrap()),
    )
    .with_artifacts(Arc::clone(artifacts))
}

/// Rewrites `path` in the next clock step and restores its modification
/// time, as a tool that preserves timestamps would: only the change time and
/// the content tell the edit apart.
fn rewrite_keeping_mtime(path: &std::path::Path, text: &str) {
    let modified = fs::metadata(path).unwrap().modified().unwrap();
    std::thread::sleep(std::time::Duration::from_millis(50));
    fs::write(path, text).unwrap();
    fs::File::options()
        .write(true)
        .open(path)
        .unwrap()
        .set_modified(modified)
        .unwrap();
}

fn synonym_of(
    fixture: &RealReaderFixture,
    artifacts: &Arc<RegistrationCache>,
) -> Option<serde_json::Value> {
    operation(fixture, artifacts)
        .common_module_synonym(&QualifiedAddress::parse(MODULE).unwrap())
        .unwrap()
        .0
}

/// Changes the catalog's code length from 11 to 12: other content, the same
/// length. With the modification time restored only the change time tells
/// the edit apart.
fn same_length_edit(text: &str) -> String {
    text.replacen(
        "<CodeLength>11</CodeLength>",
        "<CodeLength>12</CodeLength>",
        1,
    )
}

/// A descriptor proven by one operation is not read again by the next one
/// while its stamp holds; an edit of the same length is read again even with
/// its modification time restored. Both ways of taking the stamp are checked:
/// the file's own and the pass over its kind directory of a branch read.
/// rule: INV.SOURCE.CONFIGURATION-ROOT-CACHE-REUSE
#[test]
fn owner_proof_is_retained_across_operations_until_the_descriptor_changes() {
    for by_directory in [false, true] {
        let fixture = RealReaderFixture::new();
        let artifacts = Arc::new(RegistrationCache::with_settle_window_for_test(
            std::time::Duration::ZERO,
        ));
        let target = MetadataAddress::parse_logical("Catalog.Items").unwrap();
        let reads = || {
            let read = operation(&fixture, &artifacts);
            if by_directory {
                read.stamp_descriptors_by_directory();
            }
            let evidence = read
                .metadata_owner_evidence(&target, &mut || Ok(()))
                .unwrap();
            assert_eq!(evidence.artifact_name(), Some("Items"));
            read.metadata_descriptor_read_count("Catalog.Items")
        };

        assert_eq!(reads(), 1, "by_directory={by_directory}");
        assert_eq!(
            reads(),
            0,
            "an unchanged descriptor must come from the retained proof (by_directory={by_directory})"
        );
        let descriptor = fixture.source.join("Catalogs/Items.xml");
        let text = fs::read_to_string(&descriptor).unwrap();
        let edited = same_length_edit(&text);
        assert_ne!(edited, text);
        assert_eq!(edited.len(), text.len());
        rewrite_keeping_mtime(&descriptor, &edited);
        assert_eq!(
            reads(),
            1,
            "a rewritten descriptor must be proven again (by_directory={by_directory})"
        );
    }
}

/// A retained proof is kept only under a settled stamp: a descriptor younger
/// than the settle window is read by every operation.
/// rule: INV.SOURCE.CONFIGURATION-ROOT-CACHE-REUSE
#[test]
fn an_unsettled_descriptor_is_proven_by_every_operation() {
    let fixture = RealReaderFixture::new();
    let artifacts = Arc::new(RegistrationCache::with_settle_window_for_test(
        std::time::Duration::from_secs(3600),
    ));
    let target = MetadataAddress::parse_logical("Catalog.Items").unwrap();
    for _ in 0..2 {
        let read = operation(&fixture, &artifacts);
        read.metadata_owner_evidence(&target, &mut || Ok(()))
            .unwrap();
        assert_eq!(read.metadata_descriptor_read_count("Catalog.Items"), 1);
    }
}

/// The synonym of a common module is retained with its proof, and an edited
/// synonym reaches the next operation.
/// rule: INV.SOURCE.CONFIGURATION-ROOT-CACHE-REUSE
#[test]
fn common_module_synonym_is_retained_until_its_descriptor_changes() {
    let fixture = RealReaderFixture::new();
    let artifacts = Arc::new(RegistrationCache::with_settle_window_for_test(
        std::time::Duration::ZERO,
    ));
    let before = synonym_of(&fixture, &artifacts).expect("the fixture names a synonym");
    let descriptor = fixture.source.join("CommonModules/РеактивныйСервер.xml");
    let reads = |artifacts: &Arc<RegistrationCache>| {
        let read = operation(&fixture, artifacts);
        read.common_module_synonym(&QualifiedAddress::parse(MODULE).unwrap())
            .unwrap();
        read.metadata_descriptor_read_count("CommonModule.РеактивныйСервер")
    };
    assert_eq!(
        reads(&artifacts),
        0,
        "an unchanged synonym must be retained"
    );

    // The same synonym reversed: other content of the same length.
    let before_text = before.as_str().unwrap().to_string();
    let reversed: String = before_text.chars().rev().collect();
    assert_ne!(
        reversed, before_text,
        "the fixture synonym must not be a palindrome"
    );
    let text = fs::read_to_string(&descriptor).unwrap();
    let edited = text.replacen(
        &format!("<v8:content>{before_text}</v8:content>"),
        &format!("<v8:content>{reversed}</v8:content>"),
        1,
    );
    assert_ne!(edited, text);
    assert_eq!(edited.len(), text.len());
    rewrite_keeping_mtime(&descriptor, &edited);
    let after = synonym_of(&fixture, &artifacts);
    assert_eq!(after, Some(json!(reversed)));
}

/// After the first read of a set the actor proves its top-level descriptors
/// in the background, so the next operation reads none of them.
/// rule: INV.SOURCE.CONFIGURATION-ROOT-CACHE-REUSE
#[test]
fn background_warmup_retains_top_level_descriptors_for_the_next_operation() {
    let fixture = RealReaderFixture::new();
    let artifacts = Arc::new(RegistrationCache::with_settle_window_for_test(
        std::time::Duration::ZERO,
    ));
    artifacts.allow_owner_warmup_for_test();
    operation(&fixture, &artifacts).start_owner_warmup();
    let root = unica_workspace::read::open_source_directory(&fixture.source).unwrap();
    let retained = |relative: &str, synonym: bool| {
        let relative = Path::new(relative);
        let Some(stamp) = root.relative_file_stamp(relative).unwrap() else {
            return false;
        };
        if synonym {
            artifacts
                .retained_common_module_synonym(relative, &stamp)
                .is_some()
        } else {
            artifacts
                .retained_owner_evidence(relative, &stamp)
                .is_some()
        }
    };
    // Only the warm-up reads here: the test inspects the retained records
    // without proving anything itself.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    while !(retained("Catalogs/Items.xml", false)
        && retained("CommonModules/РеактивныйСервер.xml", false)
        && retained("CommonModules/РеактивныйСервер.xml", true))
    {
        assert!(
            std::time::Instant::now() < deadline,
            "the warm-up did not retain the top-level descriptors"
        );
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    let target = MetadataAddress::parse_logical("Catalog.Items").unwrap();
    let next = operation(&fixture, &artifacts);
    next.metadata_owner_evidence(&target, &mut || Ok(()))
        .unwrap();
    assert_eq!(next.metadata_descriptor_read_count("Catalog.Items"), 0);
}
