use super::*;
use std::collections::BTreeSet;
use std::io::Write;
use std::path::{Path, PathBuf};

fn current_view(
    runtime: &V5CanonicalInvocationRuntime,
    workspace: &Path,
    probe: &RetainedReadProbe,
) -> (DomainResult, Vec<RetainedRead>) {
    probe.take();
    let read = submit_canonical(
        runtime,
        workspace,
        ToolIdentity::View,
        serde_json::json!({"at":"main:Catalog.Items"}),
    );
    assert!(read.ok, "{read:?}");
    let observed = probe.take();
    assert!(
        observed
            .iter()
            .any(|event| *event == RetainedRead::Opened("Catalogs/Items.xml".into())),
        "fresh read did not open its actual target: {observed:?}"
    );
    assert!(observed.iter().all(|event| matches!(event, RetainedRead::Opened(path) if !path.to_string_lossy().contains("Unrelated"))), "logical read walked unrelated source files: {observed:?}");
    (read, observed)
}

#[test]
fn canonical_logical_read_stays_current_and_addressed_after_apply_preview() {
    for changed in [false, true] {
        let (workspace, source) = source_selection_read_fixture();
        add_unrelated_catalogs(&source, 4);
        let target = source.join("Catalogs/Items.xml");
        let xml = std::fs::read_to_string(&target).unwrap().replace(
            "<Name>Items</Name>",
            "<Name>Items</Name><Comment>before</Comment>",
        );
        std::fs::write(&target, &xml).unwrap();
        let runtime = bootstrap_runtime();
        let warm = submit_canonical(
            &runtime,
            workspace.path(),
            ToolIdentity::View,
            serde_json::json!({"at":"main:Catalog.Items"}),
        );
        assert!(warm.ok, "{warm:?}");
        let probe = RetainedReadProbe::watch(&source);
        let (baseline, baseline_reads) = current_view(&runtime, workspace.path(), &probe);
        assert_eq!(
            baseline.data.as_ref().unwrap()["props"]["Comment"],
            "before"
        );
        let source_before = unica_workspace::test_support::tree_snapshot(&source);
        let before = unica_workspace::test_support::tree_snapshot(workspace.path());
        let preview = submit_canonical(
            &runtime,
            workspace.path(),
            ToolIdentity::Apply,
            serde_json::json!({"at":"main:Catalog.Items","ops":[{"op":"props.set","args":{"values":{"Comment":"planned"}}}]}),
        );
        assert!(preview.ok, "{preview:?}");
        assert_eq!(preview.data.as_ref().unwrap()["mode"], "preview");
        assert_eq!(
            unica_workspace::test_support::tree_snapshot(workspace.path()),
            before,
            "admission, planning and dry run must not publish"
        );
        let token = preview.data.unwrap()["executionToken"].clone();
        if changed {
            let identity = unica_workspace::test_support::file_identity_for_test(&target).unwrap();
            assert!(identity.is_some());
            let external = xml.replace("<Comment>before</Comment>", "<Comment>after!</Comment>");
            assert_eq!(external.len(), xml.len());
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .truncate(true)
                .open(&target)
                .unwrap();
            file.write_all(external.as_bytes()).unwrap();
            file.sync_all().unwrap();
            assert_eq!(
                unica_workspace::test_support::file_identity_for_test(&target).unwrap(),
                identity
            );
        }
        let current_image = unica_workspace::test_support::tree_snapshot(workspace.path());
        let (fresh, fresh_reads) = current_view(&runtime, workspace.path(), &probe);
        assert_eq!(
            fresh.data.as_ref().unwrap()["props"]["Comment"],
            if changed { "after!" } else { "before" }
        );
        let baseline_paths = baseline_reads
            .iter()
            .filter_map(|event| {
                if let RetainedRead::Opened(path) = event {
                    Some(path.clone())
                } else {
                    None
                }
            })
            .collect::<BTreeSet<_>>();
        assert!(
            fresh_reads.iter().all(
                |event| matches!(event, RetainedRead::Opened(path) if baseline_paths.contains(path))
            ),
            "preview widened subsequent source reads: {fresh_reads:?}"
        );
        assert!(
            fresh_reads.len() <= baseline_reads.len(),
            "preview expanded point reads: baseline={baseline_reads:?}, fresh={fresh_reads:?}"
        );
        assert_eq!(
            unica_workspace::test_support::tree_snapshot(workspace.path()),
            current_image,
            "the fresh view writes nothing"
        );
        let execution = submit_canonical(
            &runtime,
            workspace.path(),
            ToolIdentity::Apply,
            serde_json::json!({"executionToken":token}),
        );
        if changed {
            assert!(
                !execution.ok,
                "the original plan overwrote external input: {execution:?}"
            );
            assert_eq!(
                execution.diagnostics[0]["code"], "stale_revision",
                "{execution:?}"
            );
            assert_eq!(
                unica_workspace::test_support::tree_snapshot(workspace.path()),
                current_image
            );
        } else {
            assert!(
                execution.ok,
                "fresh view invalidated an unchanged original plan: {execution:?}"
            );
            let expected_xml =
                xml.replace("<Comment>before</Comment>", "<Comment>planned</Comment>");
            let mut expected = source_before;
            expected.insert(
                PathBuf::from("Catalogs/Items.xml"),
                expected_xml.into_bytes(),
            );
            assert_eq!(
                unica_workspace::test_support::tree_snapshot(&source),
                expected
            );
        }
        assert!(source
            .join(PathBuf::from("Catalogs/Unrelated0.xml"))
            .exists());
    }
}
