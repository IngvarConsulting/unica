//! Каталог имён между поисками и однократная проверка каталогов обхода.

use super::{
    read_descriptor_head_prefix, DescriptorHeadReadError, LayoutFindSource, NameCatalog,
    NameSearchOutcome, WorkspaceFindDirectoryBuilder,
};
use crate::application::v13::find::{FindRequest, NameRanker};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use unica_refusal::RefusalCode;
use unica_source_contract::events::{DomainEvent, DomainEventKind};
use unica_source_contract::read::{SourceDirectory, SourceFile};
use unica_source_contract::SourceSetKind;
use unica_work_control::cancellation::CancellationToken;
use unica_work_control::provider_deadline::ProviderDeadline;
use unica_workspace::test_support::retained_read_probe::{RetainedRead, RetainedReadProbe};

fn write(path: &Path, contents: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, contents).unwrap();
}

fn descriptor(kind: &str, name: &str, synonym: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" xmlns:v8="http://v8.1c.ru/8.1/data/core" version="2.20">
  <{kind} uuid="10000000-0000-4000-8000-000000000001">
    <Properties><Name>{name}</Name><Synonym><v8:item><v8:lang>ru</v8:lang><v8:content>{synonym}</v8:content></v8:item></Synonym></Properties>
  </{kind}>
</MetaDataObject>"#
    )
}

struct Layout {
    root: tempfile::TempDir,
    source: PathBuf,
}

impl Layout {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().canonicalize().unwrap().join("src");
        write(
            &source.join("Configuration.xml"),
            &descriptor("Configuration", "Магазин", "Магазин"),
        );
        write(
            &source.join("Catalogs/Валюты.xml"),
            &descriptor("Catalog", "Валюты", "Валюты и курсы"),
        );
        write(
            &source.join("Catalogs/Валюты/Forms/ФормаЭлемента.xml"),
            &descriptor("Form", "ФормаЭлемента", "Форма элемента"),
        );
        write(
            &source.join("Catalogs/Валюты/Templates/Печать.xml"),
            &descriptor("Template", "Печать", "Печатная форма"),
        );
        write(
            &source.join("Documents/Заказ.xml"),
            &descriptor("Document", "Заказ", "Заказ клиента"),
        );
        Self { root, source }
    }

    fn open(&self) -> SourceDirectory {
        unica_workspace::read::open_source_directory(&self.source).unwrap()
    }

    /// Returns once the filesystem clock has moved past every write so far,
    /// so the next change gets a stamp no earlier read could have seen. The
    /// marker lives beside the source root, outside the walked layout.
    fn after_filesystem_tick(&self) {
        let marker = self.root.path().join("tick");
        let stamp = || {
            fs::write(&marker, b"tick").unwrap();
            fs::metadata(&marker).unwrap().modified().unwrap()
        };
        let first = stamp();
        while stamp() <= first {
            std::thread::yield_now();
        }
    }
}

fn deadline() -> ProviderDeadline {
    ProviderDeadline::from_budget(Duration::from_secs(20))
}

fn search_with(
    builder: &WorkspaceFindDirectoryBuilder,
    root: &SourceDirectory,
    catalog: Option<&NameCatalog>,
    query: &str,
) -> Result<NameSearchOutcome, super::FindBuildError> {
    let source = LayoutFindSource::new("main", SourceSetKind::Configuration, root);
    let source = match catalog {
        Some(catalog) => source.with_names(catalog),
        None => source,
    };
    builder.search_names(
        &[source],
        NameRanker::new(FindRequest::new(query).unwrap(), [], usize::MAX, None),
        deadline(),
        &CancellationToken::new(),
    )
}

fn ranked(outcome: &NameSearchOutcome) -> Vec<String> {
    outcome
        .names
        .candidates
        .iter()
        .map(|(_, candidate)| candidate.at().to_owned())
        .collect()
}

/// The direct matches only: nearest names are no evidence of a name.
fn found(outcome: &NameSearchOutcome) -> Vec<String> {
    if outcome.names.nearest {
        Vec::new()
    } else {
        ranked(outcome)
    }
}

fn names(root: &SourceDirectory, catalog: &NameCatalog, query: &str) -> Vec<String> {
    found(
        &search_with(
            &WorkspaceFindDirectoryBuilder::default(),
            root,
            Some(catalog),
            query,
        )
        .unwrap(),
    )
}

fn opened(path: &str) -> RetainedRead {
    RetainedRead::Opened(PathBuf::from(path))
}

fn listed(path: &str) -> RetainedRead {
    RetainedRead::Listed(PathBuf::from(path))
}

/// Reads in the order the probe reports them.
fn expected_reads<const N: usize>(expected: [RetainedRead; N]) -> Vec<RetainedRead> {
    let mut expected = expected.to_vec();
    expected.sort();
    expected
}

#[test]
fn a_repeated_name_search_reads_again_only_what_changed() {
    let layout = Layout::new();
    let root = layout.open();
    let catalog = NameCatalog::with_settle_window_for_test(Duration::ZERO);
    let reads = RetainedReadProbe::watch(&layout.source);

    assert_eq!(names(&root, &catalog, "Валюты")[0], "main:Catalog.Валюты");
    assert!(reads.take().contains(&opened("Catalogs/Валюты.xml")));
    assert_eq!(names(&root, &catalog, "Валюты")[0], "main:Catalog.Валюты");
    assert_eq!(
        reads.take(),
        [],
        "an unchanged layout is neither listed nor opened again"
    );
    let kept = catalog.kept_paths_for_test();

    layout.after_filesystem_tick();
    write(
        &layout.source.join("Catalogs/Валюты.xml"),
        &descriptor("Catalog", "Валюты", "Денежные единицы"),
    );
    assert_eq!(
        names(&root, &catalog, "Денежные единицы"),
        ["main:Catalog.Валюты"]
    );
    assert_eq!(
        reads.take(),
        expected_reads([opened("Catalogs/Валюты.xml")])
    );

    layout.after_filesystem_tick();
    write(
        &layout.source.join("Catalogs/Склады.xml"),
        &descriptor("Catalog", "Склады", "Склады"),
    );
    assert_eq!(names(&root, &catalog, "Склады"), ["main:Catalog.Склады"]);
    assert_eq!(
        reads.take(),
        expected_reads([listed("Catalogs"), opened("Catalogs/Склады.xml")])
    );

    layout.after_filesystem_tick();
    fs::remove_file(layout.source.join("Catalogs/Склады.xml")).unwrap();
    assert!(names(&root, &catalog, "Склады").is_empty());
    assert_eq!(reads.take(), expected_reads([listed("Catalogs")]));
    assert_eq!(
        catalog.kept_paths_for_test(),
        kept,
        "a removed object leaves nothing behind"
    );

    layout.after_filesystem_tick();
    write(
        &layout.source.join("Catalogs/Валюты/Forms/ФормаСписка.xml"),
        &descriptor("Form", "ФормаСписка", "Форма списка"),
    );
    assert_eq!(
        names(&root, &catalog, "ФормаСписка"),
        ["main:Catalog.Валюты.Form.ФормаСписка"]
    );
    assert_eq!(
        reads.take(),
        expected_reads([
            listed("Catalogs/Валюты/Forms"),
            opened("Catalogs/Валюты/Forms/ФормаСписка.xml"),
        ])
    );

    layout.after_filesystem_tick();
    fs::rename(
        layout.source.join("Catalogs/Валюты/Templates/Печать.xml"),
        layout.source.join("Catalogs/Валюты/Templates/Бланк.xml"),
    )
    .unwrap();
    write(
        &layout.source.join("Catalogs/Валюты/Templates/Бланк.xml"),
        &descriptor("Template", "Бланк", "Бланк"),
    );
    assert_eq!(
        names(&root, &catalog, "Бланк"),
        ["main:Catalog.Валюты.Template.Бланк"]
    );
    assert!(names(&root, &catalog, "Печать").is_empty());
}

#[test]
fn a_descriptor_younger_than_the_settle_window_is_read_again() {
    let layout = Layout::new();
    let root = layout.open();
    let catalog = NameCatalog::with_settle_window_for_test(Duration::from_secs(3600));
    let reads = RetainedReadProbe::watch(&layout.source);
    names(&root, &catalog, "Валюты");
    reads.take();
    names(&root, &catalog, "Валюты");
    let again = reads.take();
    for path in [
        "Configuration.xml",
        "Catalogs/Валюты.xml",
        "Documents/Заказ.xml",
    ] {
        assert!(again.contains(&opened(path)), "{path}: {again:?}");
    }
    assert!(again.contains(&listed("Catalogs")), "{again:?}");
}

#[test]
fn an_unreadable_descriptor_stays_omitted_and_is_read_again() {
    let layout = Layout::new();
    let root = layout.open();
    let catalog = NameCatalog::with_settle_window_for_test(Duration::ZERO);
    let failing = Arc::new(AtomicBool::new(true));
    let fault = Arc::clone(&failing);
    let builder = WorkspaceFindDirectoryBuilder::default().with_head_reader(Arc::new(
        move |file: &SourceFile, relative: &Path| {
            if relative == Path::new("Catalogs/Валюты.xml") && fault.load(Ordering::SeqCst) {
                Err(DescriptorHeadReadError::Local(
                    std::io::ErrorKind::PermissionDenied,
                ))
            } else {
                read_descriptor_head_prefix(file, relative)
            }
        },
    ));
    let reads = RetainedReadProbe::watch(&layout.source);
    for _ in 0..2 {
        let partial = search_with(&builder, &root, Some(&catalog), "Валюты").unwrap();
        assert_eq!(partial.omissions.total, 1);
        assert_eq!(partial.omissions.details[0].reason, "descriptor_unreadable");
        assert!(!found(&partial).contains(&"main:Catalog.Валюты".to_owned()));
        assert!(reads.take().contains(&opened("Catalogs/Валюты.xml")));
    }
    failing.store(false, Ordering::SeqCst);
    let complete = search_with(&builder, &root, Some(&catalog), "Валюты").unwrap();
    assert_eq!(complete.omissions.total, 0);
    assert_eq!(found(&complete)[0], "main:Catalog.Валюты");
    let recovered = reads.take();
    assert!(recovered.contains(&opened("Catalogs/Валюты.xml")));
    for kept in ["Configuration.xml", "Documents/Заказ.xml"] {
        assert!(!recovered.contains(&opened(kept)), "{kept}: {recovered:?}");
    }
}

#[test]
fn nearest_names_rank_the_documents_of_a_single_walk() {
    let layout = Layout::new();
    let root = layout.open();
    let reads = RetainedReadProbe::watch(&layout.source);
    let nearest = search_with(
        &WorkspaceFindDirectoryBuilder::default(),
        &root,
        None,
        "Вылюты",
    )
    .unwrap();
    assert!(nearest.names.nearest);
    assert_eq!(ranked(&nearest)[0], "main:Catalog.Валюты");
    let walked = reads.take();
    for path in ["Catalogs/Валюты.xml", "Documents/Заказ.xml"] {
        assert_eq!(
            walked.iter().filter(|read| **read == opened(path)).count(),
            1,
            "{path}: {walked:?}"
        );
    }
}

#[test]
fn a_published_collection_is_read_again_and_the_others_are_kept() {
    let layout = Layout::new();
    let root = layout.open();
    let catalog = NameCatalog::with_settle_window_for_test(Duration::ZERO);
    let reads = RetainedReadProbe::watch(&layout.source);
    names(&root, &catalog, "Валюты");
    reads.take();

    // Another source set's publication changes nothing here.
    catalog.forget_published(
        "main",
        "main",
        &[DomainEvent::new(
            DomainEventKind::MetadataChanged,
            "extension:Catalog.Валюты",
        )],
    );
    names(&root, &catalog, "Валюты");
    assert_eq!(reads.take(), []);

    catalog.forget_published(
        "main",
        "main",
        &[DomainEvent::new(
            DomainEventKind::FormChanged,
            "Catalog.Валюты.Form.ФормаЭлемента",
        )],
    );
    names(&root, &catalog, "Валюты");
    let again = reads.take();
    assert!(again.contains(&listed("Catalogs")), "{again:?}");
    assert!(again.contains(&opened("Catalogs/Валюты.xml")), "{again:?}");
    assert!(
        again.iter().all(|read| !matches!(read,
            RetainedRead::Opened(path) | RetainedRead::Listed(path)
                if path.starts_with("Documents") || path == Path::new("Configuration.xml"))),
        "{again:?}"
    );

    // An event that names no collection drops the whole catalog.
    catalog.forget_published(
        "main",
        "extension",
        &[DomainEvent::new(
            DomainEventKind::ConfigXmlChanged,
            "main:Configuration",
        )],
    );
    names(&root, &catalog, "Валюты");
    let whole = reads.take();
    assert!(whole.contains(&opened("Documents/Заказ.xml")), "{whole:?}");
    assert!(whole.contains(&opened("Configuration.xml")), "{whole:?}");
}

/// The rule INV.APP.ACTOR-AUTHENTICATED-SOURCE-CAPABILITIES: reading many
/// files of one directory checks the identity of that directory once per
/// request, and no file is checked by name.
#[test]
fn a_walk_checks_each_directory_once_however_many_files_it_reads() {
    let validations = |extra: usize| {
        let layout = Layout::new();
        for index in 0..extra {
            let name = format!("Каталог{index:02}");
            write(
                &layout.source.join(format!("Catalogs/{name}.xml")),
                &descriptor("Catalog", &name, &name),
            );
        }
        let root = layout.open();
        let reads = RetainedReadProbe::watch(&layout.source);
        let outcome = search_with(
            &WorkspaceFindDirectoryBuilder::default(),
            &root,
            None,
            "Каталог",
        )
        .unwrap();
        assert_eq!(found(&outcome).len(), extra);
        let read_files = reads
            .take()
            .into_iter()
            .filter(|read| matches!(read, RetainedRead::Opened(_)))
            .count();
        assert_eq!(read_files, extra + 5);
        reads.take_validations()
    };
    let expected = [
        "",
        "Catalogs",
        "Catalogs/Валюты",
        "Catalogs/Валюты/Forms",
        "Catalogs/Валюты/Templates",
        "Documents",
    ]
    .map(PathBuf::from);
    assert_eq!(validations(2), expected);
    assert_eq!(validations(40), expected);
}

/// The rule INV.APP.ACTOR-AUTHENTICATED-SOURCE-CAPABILITIES: replacing the
/// root or a directory the walk reads through during the walk refuses its
/// answer.
#[test]
fn a_root_or_directory_replaced_during_the_walk_refuses_its_answer() {
    if !unica_workspace::test_support::supports_retained_root_replacement_test() {
        unica_process::test_support::skip_without_host_capability(
            unica_process::test_support::HostCapability::RenameUnderOpenHandle,
            "the platform keeps an open directory from being renamed or replaced",
        );
        return;
    }
    // Nothing below `Documents` is opened after its descriptor is read: only
    // the check before the answer can notice the replacement.
    for (replaced, during) in [
        ("Documents", "Documents/Заказ.xml"),
        ("", "Catalogs/Валюты.xml"),
    ] {
        let layout = Layout::new();
        let root = layout.open();
        let target = layout.source.join(replaced);
        let moved = layout.root.path().join("moved");
        let replace = Arc::new(AtomicBool::new(true));
        let once = Arc::clone(&replace);
        let builder = WorkspaceFindDirectoryBuilder::default().with_head_reader(Arc::new(
            move |file: &SourceFile, relative: &Path| {
                if relative == Path::new(during) && once.swap(false, Ordering::SeqCst) {
                    fs::rename(&target, &moved).unwrap();
                    copy_tree(&moved, &target);
                }
                read_descriptor_head_prefix(file, relative)
            },
        ));
        let catalog = NameCatalog::with_settle_window_for_test(Duration::ZERO);
        let refusal = search_with(&builder, &root, Some(&catalog), "Валюты")
            .err()
            .unwrap_or_else(|| panic!("replacing {replaced:?} must refuse the walk"));
        assert!(!replace.load(Ordering::SeqCst), "{replaced:?} was replaced");
        assert_eq!(refusal.code(), RefusalCode::InvalidSource, "{replaced:?}");
    }
}

/// A descriptor at the root is read through the root itself: replacing it
/// during the walk changes the root, which the walk confirms before its
/// answer.
#[test]
fn a_root_descriptor_replaced_during_the_walk_refuses_its_answer() {
    let layout = Layout::new();
    let root = layout.open();
    let descriptor = layout.source.join("Configuration.xml");
    let replace = Arc::new(AtomicBool::new(true));
    let once = Arc::clone(&replace);
    let builder = WorkspaceFindDirectoryBuilder::default().with_head_reader(Arc::new(
        move |file: &SourceFile, relative: &Path| {
            if relative == Path::new("Configuration.xml") && once.swap(false, Ordering::SeqCst) {
                fs::rename(&descriptor, descriptor.with_extension("old")).unwrap();
                fs::write(&descriptor, "replacement").unwrap();
            }
            read_descriptor_head_prefix(file, relative)
        },
    ));
    let refusal = search_with(&builder, &root, None, "Магазин")
        .err()
        .expect("replacing a root descriptor must refuse the walk");
    assert!(!replace.load(Ordering::SeqCst));
    // The root itself is unchanged in identity, only its contents moved.
    assert_eq!(refusal.code(), RefusalCode::ConcurrentChange);
}

fn copy_tree(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap();
    for entry in fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let target = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_tree(&entry.path(), &target);
        } else {
            fs::copy(entry.path(), target).unwrap();
        }
    }
}
