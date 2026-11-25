use super::*;
use crate::application::v13::resolve::ResolvedLines;
use crate::infrastructure::v13_find::{LayoutFindSource, WorkspaceFindDirectoryBuilder};
use serde_json::Value;
use unica_workspace::test_support::tree_snapshot;

const EXTERNAL_FIXTURES: [(&str, SourceSetKind, &str, &str); 2] = [
    (
        "epf",
        SourceSetKind::ExternalProcessor,
        "ExternalDataProcessor",
        "SyntheticProcessor",
    ),
    (
        "erf",
        SourceSetKind::ExternalReport,
        "ExternalReport",
        "SyntheticReport",
    ),
];

fn authority<'a>(
    source: &Path,
    alias: &str,
    kind: SourceSetKind,
    cancellation: &'a CancellationToken,
) -> LogicalViewReadAuthority<'a> {
    let source = source.canonicalize().unwrap();
    let admitted = unica_workspace::source_profile::platform_xml_source_profile(kind, &source);
    let profile = PlatformProfile::for_line_or_base(admitted.semantic_line());
    LogicalViewReadAuthority::new(
        cancellation,
        alias,
        "native-external-inline-members",
        kind,
        Arc::new(unica_workspace::read::open_source_directory(&source).unwrap()),
        profile,
    )
}

pub(super) fn expected_variant(name: &str) -> Option<Value> {
    Some(match name {
        "TextValue" => json!({"kind":"string","length":50,"allowedLength":"variable"}),
        "Amount" => json!({"kind":"number","digits":15,"fraction":2,"sign":"any"}),
        "DocDate" => json!({"kind":"date","fractions":"date"}),
        "Flag" => json!({"kind":"boolean"}),
        "Item" => json!({"kind":"string","length":100,"allowedLength":"variable"}),
        "Quantity" => json!({"kind":"number","digits":10,"fraction":3,"sign":"nonNegative"}),
        "Lines" => return None,
        _ => panic!("unexpected native member {name}"),
    })
}

/// rule: INV.SOURCE.EXTERNAL-ARTIFACT-ROOTS
/// rule: INV.SOURCE.LOGICAL-BRANCH-COUNT
/// rule: INV.SOURCE.RESOLVED-RESOURCE-IDENTITY
#[test]
fn native_external_inline_members_read_and_resolve_to_one_owner_xml() {
    for format in ["2.20", "2.21", "2.22"] {
        for (alias, kind, owner_kind, name) in EXTERNAL_FIXTURES {
            let source = fixture_path(&format!(
                "formats/platform_xml/{format}/external_artifacts/{alias}"
            ));
            let before = tree_snapshot(&source);
            let cancellation = CancellationToken::new();
            let reader = authority(&source, alias, kind, &cancellation);
            let service = ViewService::new(
                authority(&source, alias, kind, &cancellation),
                ViewCursorStore::default(),
            );
            let owner = format!("{alias}:{owner_kind}.{name}");
            let owner_xml = format!("{name}.xml");
            let view = |address: &str| {
                let result = service.view(ViewRequest::new(address).unwrap());
                assert!(result.ok, "{format} {address}: {:?}", result.diagnostics);
                result.data.unwrap()
            };
            let owner_view = view(&owner);
            for (branch, count) in [
                ("Attribute", 4),
                ("TabularSection", 1),
                ("Form", if alias == "epf" { 2 } else { 4 }),
                ("Template", if alias == "epf" { 3 } else { 4 }),
                ("Command", 0),
            ] {
                assert!(
                    owner_view["branches"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .any(
                            |row| row["at"] == format!("{owner}.{branch}") && row["count"] == count
                        ),
                    "{format} {owner} {branch}: {owner_view:?}"
                );
            }
            let section = view(&format!("{owner}.TabularSection.Lines"));
            assert!(section["branches"]
                .as_array()
                .unwrap()
                .iter()
                .any(
                    |row| row["at"] == format!("{owner}.TabularSection.Lines.Attribute")
                        && row["count"] == 2
                ));
            for (suffix, leaf_kind, names) in [
                (
                    "Attribute",
                    "Attribute",
                    &["TextValue", "Amount", "DocDate", "Flag"][..],
                ),
                ("TabularSection", "TabularSection", &["Lines"][..]),
                (
                    "TabularSection.Lines.Attribute",
                    "Attribute",
                    &["Item", "Quantity"][..],
                ),
            ] {
                let branch = format!("{owner}.{suffix}");
                let request = ViewRequest::new(&branch).unwrap().with_limit(1).unwrap();
                let mut cursor = None;
                for (index, name) in names.iter().enumerate() {
                    let page = service.view(match cursor.take() {
                        Some(previous) => request.clone().with_cursor(previous),
                        None => request.clone(),
                    });
                    assert!(page.ok, "{format} {branch}: {:?}", page.diagnostics);
                    let data = page.data.unwrap();
                    assert_eq!(data["total"], names.len(), "{format} {branch}");
                    assert_eq!(data["offset"], index, "{format} {branch}");
                    let [item] = data["items"].as_array().unwrap().as_slice() else {
                        panic!("{format} {branch}: {data:?}");
                    };
                    let address = format!("{branch}.{name}");
                    assert_eq!(item["at"], address);
                    let detail = view(&address);
                    assert_eq!(detail["at"], address);
                    assert_eq!(detail["kind"], leaf_kind);
                    if let Some(variant) = expected_variant(name) {
                        for value in [item, &detail] {
                            assert_eq!(
                                value["props"]["type"]["variants"],
                                json!([variant]),
                                "{address}"
                            );
                        }
                    }
                    let at = QualifiedAddress::parse(&address).unwrap();
                    let snapshot = reader.snapshot(&at).unwrap();
                    let (actual_kind, lines) = reader.source_lines(&at, &snapshot).unwrap();
                    assert_eq!(actual_kind, leaf_kind);
                    assert!(matches!(lines, ResolvedLines::NotLineBased));
                    assert_eq!(reader.existing_export_path(&at, None).unwrap(), owner_xml);
                    cursor = page.cursor;
                    assert_eq!(cursor.is_some(), index + 1 < names.len());
                }
            }
            let root =
                unica_workspace::read::open_source_directory(&source.canonicalize().unwrap())
                    .unwrap();
            let layout = [LayoutFindSource::new(alias, kind, &root)];
            let found = WorkspaceFindDirectoryBuilder::default()
                .locate_path(
                    &layout,
                    &owner_xml,
                    ProviderDeadline::no_deadline(),
                    &cancellation,
                )
                .unwrap()
                .unwrap();
            assert_eq!(found.at(), owner);
            assert_eq!(found.placed_path(), Some(owner_xml.as_str()));
            let missing = format!("{owner}.TabularSection.Lines.Attribute.Missing");
            let result = service.view(ViewRequest::new(&missing).unwrap());
            assert!(!result.ok);
            assert_eq!(result.diagnostics[0]["code"], "not_found");
            assert_eq!(tree_snapshot(&source), before);
        }
    }
}

const DAMAGE_OWNER: &str = "epf:ExternalDataProcessor.SyntheticProcessor";
const DAMAGE_FIXTURE: &str = "formats/platform_xml/2.20/external_artifacts/epf";

fn copied_processor() -> tempfile::TempDir {
    let fixture = tempfile::tempdir().unwrap();
    for (path, bytes) in tree_snapshot(&fixture_path(DAMAGE_FIXTURE)) {
        let target = fixture.path().join(path);
        fs::create_dir_all(target.parent().unwrap()).unwrap();
        fs::write(target, bytes).unwrap();
    }
    fixture
}

/// rule: INV.SOURCE.EXTERNAL-ARTIFACT-ROOTS
#[test]
fn external_inline_identity_damage_refuses_instead_of_reading_an_empty_branch() {
    let fixture = copied_processor();
    let path = fixture.path().join("SyntheticProcessor.xml");
    let native = fs::read_to_string(&path).unwrap();
    for (from, to) in [
        ("<ChildObjects>", "<ChildObjects/><ChildObjects>"),
        ("<Attribute uuid=", "<Attribute xmlns=\"urn:foreign\" uuid="),
        ("<Name>DocDate</Name>", "<Name>Flag</Name>"),
        ("<Name>Item</Name>", "<Name>Item</Name><Name>Other</Name>"),
        (
            "c6b5ca2d-cd6a-5cdd-873b-515bf1e67efc",
            "64d347bf-cfd2-5dfb-81c3-86ceed7a3b11",
        ),
        ("739565a7-d753-50ab-a829-c46dde496d78", "broken-uuid"),
    ] {
        assert!(native.contains(from));
        fs::write(&path, native.replacen(from, to, 1)).unwrap();
        let before = tree_snapshot(fixture.path());
        let cancellation = CancellationToken::new();
        let reader = authority(
            fixture.path(),
            "epf",
            SourceSetKind::ExternalProcessor,
            &cancellation,
        );
        let at = QualifiedAddress::parse(DAMAGE_OWNER).unwrap();
        let error = reader
            .snapshot(&at)
            .and_then(|snapshot| reader.read_exact(&at, &snapshot))
            .expect_err(from);
        assert_eq!(
            error.detail(),
            Some(unica_refusal::RefusalDetail::SourceUnreadable),
            "{from}: {error:?}"
        );
        assert_eq!(tree_snapshot(fixture.path()), before);
    }
}

/// rule: INV.SOURCE.OBSERVED-TYPE-CAPABILITY
#[test]
fn external_inline_type_damage_preserves_member_identity_and_typed_limits() {
    let fixture = copied_processor();
    let path = fixture.path().join("SyntheticProcessor.xml");
    let native = fs::read_to_string(&path).unwrap();
    let malformed = native.replacen("<v8:Length>50</v8:Length>", "<v8:Length>bad</v8:Length>", 1);
    assert_ne!(malformed, native);
    let mut unmodelled = native.replacen(
        "<v8:Type>xs:string</v8:Type>",
        "<v8:Type>v8:FutureOpaque</v8:Type>",
        1,
    );
    let first = unmodelled.find("<v8:StringQualifiers>").unwrap();
    let end = unmodelled.find("</v8:StringQualifiers>").unwrap() + "</v8:StringQualifiers>".len();
    unmodelled.replace_range(first..end, "");
    for (xml, rec) in [
        (malformed, "validation_failed"),
        (unmodelled, "validation_warning"),
    ] {
        fs::write(&path, xml).unwrap();
        let before = tree_snapshot(fixture.path());
        let cancellation = CancellationToken::new();
        let service = ViewService::new(
            authority(
                fixture.path(),
                "epf",
                SourceSetKind::ExternalProcessor,
                &cancellation,
            ),
            ViewCursorStore::default(),
        );
        let view = |suffix: &str| {
            let at = format!("{DAMAGE_OWNER}.{suffix}");
            let result = service.view(ViewRequest::new(&at).unwrap());
            assert!(result.ok, "{at}: {:?}", result.diagnostics);
            result.data.unwrap()
        };
        assert_eq!(view("Attribute")["total"], 4);
        let text = view("Attribute.TextValue");
        assert_eq!(text["props"]["incomplete"], true);
        assert!(text["props"].get("type").is_none());
        assert!(
            text["limits"]
                .as_array()
                .unwrap()
                .iter()
                .any(|limit| limit.as_str().is_some_and(|text| text.contains(rec))),
            "{text:?}"
        );
        assert_eq!(
            view("Attribute.Flag")["props"]["type"]["variants"],
            json!([{"kind":"boolean"}])
        );
        assert_eq!(tree_snapshot(fixture.path()), before);
    }
}
