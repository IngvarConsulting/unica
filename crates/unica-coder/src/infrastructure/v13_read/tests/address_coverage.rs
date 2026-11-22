use super::*;
use crate::infrastructure::logical_tree::LogicalReader;
use crate::infrastructure::v13_find::{LayoutFindSource, WorkspaceFindDirectoryBuilder};
use crate::infrastructure::v13_read_projection::metadata_address_coverage::{
    item_facts, owner_facts, BranchFact,
};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use unica_address::address::NodeKind;
use unica_address::kinds::{nested_place, DECLARED_ROOT_KINDS, METADATA_LAYOUTS};
use unica_source_contract::format_identity::{
    export_format_line, metadata_kind_exists, metadata_kind_since, PlatformLine,
    DESCRIBED_PLATFORM_LINES,
};
use unica_source_designer::owner::MD_CLASSES_NS;
use unica_source_designer::registration::platform_child_object_order;
use unica_work_control::provider_deadline::ProviderDeadline;

// One sorted JSONL file per successful test, when explicitly requested. Use
// an empty output directory per exact run; the caller verifies the test result.
const EVIDENCE_DIRECTORY: &str = "UNICA_484_ADDRESS_JSONL_DIR";

enum AddressEvidence<'a> {
    Read(&'a str),
    RoundTrip(&'a str),
    IntroducedIn(PlatformLine),
}

enum AddressRequirement {
    Address,
    Read,
    RoundTrip,
    IntroducedIn(PlatformLine),
}

fn address_key(
    source: (PlatformLine, SourceSetKind),
    owner: &QualifiedAddress,
    branch: &str,
    role: Option<&str>,
) -> Value {
    let (line, source_kind) = source;
    let owner_shape: Vec<_> = owner
        .segments()
        .iter()
        .map(|segment| {
            serde_json::json!({
                "kind": segment.kind().as_str(),
                "named": segment.name().is_some(),
                "role": matches!(segment.kind(), NodeKind::Interface | NodeKind::Module)
                    .then_some(segment.name()).flatten(),
            })
        })
        .collect();
    serde_json::json!({
        "line": line.to_string(),
        "source_kind": source_kind,
        "owner_shape": owner_shape,
        "branch": branch,
        "role": role,
    })
}

struct RequiredKeys {
    output: Option<PathBuf>,
    rows: BTreeMap<String, Value>,
}

impl RequiredKeys {
    fn new(shard: &str) -> Self {
        Self {
            output: std::env::var_os(EVIDENCE_DIRECTORY)
                .map(|directory| PathBuf::from(directory).join(shard)),
            rows: BTreeMap::new(),
        }
    }

    fn record(
        &mut self,
        source: (PlatformLine, SourceSetKind),
        owner: &QualifiedAddress,
        branch: &str,
        role: Option<&str>,
        requirement: AddressRequirement,
    ) {
        if self.output.is_none() {
            return;
        }
        let key = address_key(source, owner, branch, role);
        let requirement = match requirement {
            AddressRequirement::Address => serde_json::json!({"kind": "address"}),
            AddressRequirement::Read => serde_json::json!({"kind": "read"}),
            AddressRequirement::RoundTrip => serde_json::json!({"kind": "roundtrip"}),
            AddressRequirement::IntroducedIn(since) => {
                serde_json::json!({"kind": "introduced_in", "line": since.to_string()})
            }
        };
        let row = serde_json::json!({
            "schema_version": 1,
            "record_type": "required",
            "key": key,
            "requirement": requirement,
        });
        match self.rows.entry(key.to_string()) {
            std::collections::btree_map::Entry::Vacant(entry) => {
                entry.insert(row);
            }
            std::collections::btree_map::Entry::Occupied(entry) => {
                assert_eq!(
                    entry.get(),
                    &row,
                    "conflicting address requirement for {key}"
                );
            }
        }
    }

    fn finish(self) {
        if let Some(output) = self.output {
            use std::io::Write;
            let mut file = fs::File::create_new(&output)
                .unwrap_or_else(|error| panic!("required addresses {}: {error}", output.display()));
            for row in self.rows.into_values() {
                writeln!(file, "{row}").unwrap_or_else(|error| {
                    panic!("required addresses {}: {error}", output.display())
                });
            }
        }
    }
}

struct EvidenceRows {
    directory: Option<std::path::PathBuf>,
    test_case: &'static str,
    source_kind: SourceSetKind,
    lines: Vec<String>,
}

impl EvidenceRows {
    fn new(test_case: &'static str, source_kind: SourceSetKind) -> Self {
        Self {
            directory: std::env::var_os(EVIDENCE_DIRECTORY).map(std::path::PathBuf::from),
            test_case,
            source_kind,
            lines: Vec::new(),
        }
    }

    fn record(
        &mut self,
        line: PlatformLine,
        owner: &str,
        branch: &str,
        role: Option<&str>,
        origin: &str,
        evidence: AddressEvidence<'_>,
    ) {
        self.record_for_source(
            (line, self.source_kind),
            owner,
            branch,
            role,
            origin,
            evidence,
        );
    }

    fn record_for_source(
        &mut self,
        source: (PlatformLine, SourceSetKind),
        owner: &str,
        branch: &str,
        role: Option<&str>,
        origin: &str,
        evidence: AddressEvidence<'_>,
    ) {
        if self.directory.is_none() {
            return;
        }
        let owner_address = QualifiedAddress::parse(owner).expect("witness owner address");
        assert_eq!(owner_address.to_string(), owner);
        let (at, level, reason) = match evidence {
            AddressEvidence::Read(at) => (Some(at), Some("read"), None),
            AddressEvidence::RoundTrip(at) => (Some(at), Some("roundtrip"), None),
            AddressEvidence::IntroducedIn(since) => (
                None,
                None,
                Some(serde_json::json!({"kind": "introduced_in", "line": since.to_string()})),
            ),
        };
        self.lines.push(
            serde_json::json!({
                "schema_version": 1,
                "record_type": "evidence",
                "key": address_key(source, &owner_address, branch, role),
                "witness": {
                    "owner": owner,
                    "at": at,
                    "level": level,
                    "origin": origin,
                    "test_case": self.test_case,
                    "reason": reason,
                },
            })
            .to_string(),
        );
    }

    fn finish(mut self) {
        if let Some(directory) = self.directory {
            self.lines.sort_unstable();
            let output = directory.join(format!("{}.jsonl", self.test_case));
            let mut file = fs::File::create(&output)
                .unwrap_or_else(|error| panic!("address evidence {}: {error}", output.display()));
            use std::io::Write;
            for row in self.lines {
                writeln!(file, "{row}").unwrap_or_else(|error| {
                    panic!("address evidence {}: {error}", output.display())
                });
            }
        }
    }
}

fn fixture_origin(path: &Path) -> String {
    let relative = path
        .strip_prefix(fixture_path(""))
        .expect("native test fixture");
    Path::new("tests")
        .join("fixtures")
        .join(relative)
        .to_str()
        .unwrap()
        .replace('\\', "/")
}

fn read(authority: &LogicalViewReadAuthority<'_>, address: &str) -> Value {
    let at = QualifiedAddress::parse(address).unwrap_or_else(|error| panic!("{address}: {error}"));
    assert_eq!(
        at.to_string(),
        address,
        "published address must be canonical"
    );
    route_logical_address(&at, authority.platform_profile())
        .unwrap_or_else(|error| panic!("{address}: {error}"));
    let admitted = authority
        .snapshot(&at)
        .unwrap_or_else(|error| panic!("{address}: {error:?}"));
    let view = authority
        .read_exact(&at, &admitted)
        .unwrap_or_else(|error| panic!("{address}: {error:?}"));
    assert_eq!(view.at(), address);
    serde_json::to_value(view).unwrap()
}

fn addresses(value: &Value, field: &str) -> BTreeSet<String> {
    let rows = value[field].as_array().unwrap();
    let addresses: BTreeSet<_> = rows
        .iter()
        .map(|row| row["at"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(
        addresses.len(),
        rows.len(),
        "duplicate addresses in {field}"
    );
    addresses
}

fn root_registrations(source: &Path, expected_format: &str) -> Vec<(String, String)> {
    let xml = fs::read_to_string(source.join("Configuration.xml")).unwrap();
    let document = roxmltree::Document::parse(xml.trim_start_matches('\u{feff}')).unwrap();
    let root = document.root_element();
    assert_eq!(root.attribute("version"), Some(expected_format));
    let owner = root
        .children()
        .find(|node| node.has_tag_name((MD_CLASSES_NS, "Configuration")))
        .unwrap();
    let registrations = owner
        .children()
        .find(|node| node.has_tag_name((MD_CLASSES_NS, "ChildObjects")))
        .unwrap();
    registrations
        .children()
        .filter(|node| node.is_element())
        .map(|node| {
            assert_eq!(node.tag_name().namespace(), Some(MD_CLASSES_NS));
            let kind = node.tag_name().name().to_string();
            let name = unica_source_designer::owner::registered_child_name(node)
                .expect("a native registration names its object");
            (kind, name)
        })
        .collect()
}

enum RootAddress {
    Canonical(String),
    IntroducedIn(PlatformLine),
}

fn native_authority<'a>(
    cancellation: &'a CancellationToken,
    source: &Path,
    profile: PlatformProfile,
) -> LogicalViewReadAuthority<'a> {
    let root = Arc::new(
        unica_workspace::read::open_source_directory(&fs::canonicalize(source).unwrap()).unwrap(),
    );
    LogicalViewReadAuthority::new(
        cancellation,
        "main",
        "address-coverage-native-root",
        SourceSetKind::Configuration,
        root,
        profile,
    )
}

/// rule: CTR.SOURCE.QUALIFIED-LOGICAL-ADDRESS
/// rule: INV.SOURCE.LOGICAL-BRANCH-COUNT
/// rule: INV.SOURCE.PLATFORM-CAPABILITY-EXISTENCE
/// rule: INV.SOURCE.PROFILE-CAPABILITIES
#[test]
fn native_configuration_root_address_table_matches_published_branches() {
    let profile_fixture = module_capability_fixture();
    assert_eq!(profile_fixture.profile, PlatformProfile::v8_3_27().id());
    record_required_configuration_module_roles(&profile_fixture);
    // These lines share the 8.3.27 configuration module roles. The frozen
    // fixture, independent of PlatformProfile::module_children, names each row.
    let mut expected_modules = BTreeMap::new();
    for case in &profile_fixture.module_capabilities {
        if !case.at.starts_with("main:Module.") || !case.exists {
            continue;
        }
        let role = case.role.as_deref().expect("an approved module has a role");
        assert_eq!(case.at, format!("main:Module.{role}"));
        assert!(expected_modules.insert(case.at.as_str(), role).is_none());
    }
    assert!(!expected_modules.is_empty());

    let expected_keys: BTreeSet<_> = DESCRIBED_PLATFORM_LINES
        .iter()
        .flat_map(|(line, _)| DECLARED_ROOT_KINDS.map(|kind| (*line, "Configuration", kind)))
        .collect();
    let mut required = RequiredKeys::new("required_configuration_root_keys.jsonl");
    let configuration = QualifiedAddress::parse("main:Configuration").unwrap();
    for &(line, _, kind) in &expected_keys {
        let requirement = metadata_kind_since(kind)
            .filter(|since| line < *since)
            .map_or(
                AddressRequirement::Address,
                AddressRequirement::IntroducedIn,
            );
        required.record(
            (line, SourceSetKind::Configuration),
            &configuration,
            kind,
            None,
            requirement,
        );
    }
    required.finish();
    let mut rows = BTreeMap::new();
    let mut modules = Vec::new();
    let mut evidence = EvidenceRows::new(
        "native_configuration_root_address_table_matches_published_branches",
        SourceSetKind::Configuration,
    );
    for xml in ["2.20", "2.21", "2.22"] {
        let line = export_format_line(xml).unwrap();
        let source = fixture_path(&format!("formats/platform_xml/{xml}/all_kinds/src"));
        let profile = PlatformProfile::for_line(line).unwrap();
        let cancellation = CancellationToken::new();
        let authority = native_authority(&cancellation, &source, profile);

        let (admitted, unavailable): (BTreeSet<_>, BTreeSet<_>) = DECLARED_ROOT_KINDS
            .into_iter()
            .partition(|kind| metadata_kind_exists(Some(line), kind));
        let registrations = root_registrations(&source, xml);
        let origin = fixture_origin(&source.join("Configuration.xml"));
        let native_kinds: BTreeSet<_> = registrations
            .iter()
            .map(|(kind, _)| kind.as_str())
            .collect();
        assert_eq!(native_kinds, admitted, "XML {xml}: registered root kinds");

        let published = read(&authority, "main:Configuration");
        check_native_named_configuration_roots(
            &authority,
            &cancellation,
            &source,
            xml,
            &published,
            &registrations,
            &mut evidence,
        );
        let mut branches = addresses(&published, "branches");
        assert!(
            branches.remove("main:Module"),
            "{line}: missing Module branch"
        );
        for at in branches {
            let spelling = at.strip_prefix("main:").unwrap();
            let &kind = admitted
                .get(spelling)
                .unwrap_or_else(|| panic!("{line}: unexpected root branch {at}"));
            evidence.record(
                line,
                "main:Configuration",
                kind,
                None,
                &origin,
                AddressEvidence::Read(&at),
            );
            assert!(rows
                .insert((line, "Configuration", kind), RootAddress::Canonical(at))
                .is_none());
        }
        for kind in unavailable {
            let since = metadata_kind_since(kind)
                .filter(|since| line < *since)
                .expect("a missing root kind must have a later introduction line");
            evidence.record(
                line,
                "main:Configuration",
                kind,
                None,
                "crates/unica-address/src/kinds.rs",
                AddressEvidence::IntroducedIn(since),
            );
            assert!(rows
                .insert(
                    (line, "Configuration", kind),
                    RootAddress::IntroducedIn(since)
                )
                .is_none());
        }

        let module_branch = published["branches"]
            .as_array()
            .unwrap()
            .iter()
            .find(|branch| branch["at"] == "main:Module")
            .unwrap();
        assert_eq!(
            module_branch["count"].as_u64(),
            Some(expected_modules.len() as u64)
        );
        modules.push((line, "Module".to_string(), "main:Module".to_string()));

        let published_modules = read(&authority, "main:Module");
        let mut observed_modules = BTreeMap::new();
        for item in published_modules["items"].as_array().unwrap() {
            assert_eq!(item["props"]["ownerKind"], "Configuration");
            let at = item["at"].as_str().unwrap();
            let role = item["props"]["role"].as_str().unwrap();
            assert!(
                observed_modules.insert(at, role).is_none(),
                "{line}: duplicate {at}"
            );
            let address = QualifiedAddress::parse(at).unwrap();
            assert_eq!(
                route_logical_address(&address, profile).unwrap().reader(),
                LogicalReader::Module
            );
            assert_eq!(read(&authority, at)["kind"], "Module", "{line}: {at}");
            modules.push((line, format!("Module.{role}"), at.to_string()));
        }
        assert_eq!(
            observed_modules, expected_modules,
            "{line}: configuration module rows"
        );
        let module_origin = "tests/fixtures/v013/address-profile-8.3.27.json";
        evidence.record(
            line,
            "main:Configuration",
            "Module",
            None,
            module_origin,
            AddressEvidence::Read("main:Module"),
        );
        for (at, role) in observed_modules {
            evidence.record(
                line,
                "main:Configuration",
                "Module",
                Some(role),
                module_origin,
                AddressEvidence::Read(at),
            );
        }
        check_native_configuration_root_module_paths(xml, &expected_modules, &mut evidence);
    }

    let actual_keys: BTreeSet<_> = rows.keys().copied().collect();
    assert_eq!(
        actual_keys, expected_keys,
        "declared root address table coverage"
    );
    println!("| Profile | Kind | Branch | Address or reason |");
    println!("| --- | --- | --- | --- |");
    for ((line, owner, kind), address) in rows {
        match address {
            RootAddress::Canonical(at) => println!("| {line} | {owner} | {kind} | {at} |"),
            RootAddress::IntroducedIn(since) => {
                println!("| {line} | {owner} | {kind} | introduced in {since} |")
            }
        }
    }
    for (line, role, at) in modules {
        println!("| {line} | Configuration | {role} | {at} |");
    }
    evidence.finish();
}

fn check_native_named_configuration_roots(
    authority: &LogicalViewReadAuthority<'_>,
    cancellation: &CancellationToken,
    source: &Path,
    format: &str,
    configuration: &Value,
    registrations: &[(String, String)],
    evidence: &mut EvidenceRows,
) {
    let root =
        unica_workspace::read::open_source_directory(&source.canonicalize().unwrap()).unwrap();
    let source_kind = SourceSetKind::Configuration;
    let layout = [LayoutFindSource::new("main", source_kind, &root)];
    let finder = WorkspaceFindDirectoryBuilder::default();
    let check_reverse = |at: &str, kind: &str, relative: &str| {
        let located = finder
            .locate_path(
                &layout,
                relative,
                ProviderDeadline::no_deadline(),
                cancellation,
            )
            .unwrap()
            .unwrap_or_else(|| panic!("{format}: no registered source at {relative}"));
        assert_eq!(located.at(), at, "{format}: {relative}");
        assert_eq!(located.kind(), kind, "{format}: {relative}");
        assert_eq!(located.placed_path(), Some(relative));
    };
    let mut members = BTreeMap::<String, BTreeSet<String>>::new();
    for (kind, name) in registrations {
        let branch = QualifiedAddress::parse(&format!("main:{kind}")).unwrap();
        let at = branch.member(name).unwrap();
        let at_text = at.to_string();
        assert!(
            members
                .entry(branch.to_string())
                .or_default()
                .insert(at_text.clone()),
            "{format}: duplicate native registration {at}"
        );

        let directory = METADATA_LAYOUTS
            .iter()
            .find(|layout| layout.tag == kind)
            .unwrap()
            .directory;
        let descriptor = format!("{directory}/{name}.xml");
        let path = source.join(&descriptor);
        let raw = read_fixture_descriptor(&path);
        let document = parse_fixture_descriptor(&raw, &path);
        assert_eq!(
            document.root_element().attribute("version"),
            Some(format),
            "{at}"
        );
        let artifact = fixture_artifact(&document, kind, &path);
        assert_eq!(
            fixture_properties_name(artifact),
            Some(name.as_str()),
            "{at}"
        );
        assert!(
            uuid::Uuid::parse_str(artifact.attribute("uuid").unwrap()).is_ok(),
            "{at}"
        );

        // The descriptor location is the forward metadata point used by resolve.
        let target =
            unica_address::source_target::MetadataAddress::parse_logical(&at.logical_path())
                .unwrap();
        let forward =
            unica_source_designer::layout::metadata_descriptor_relative(&target, source_kind)
                .unwrap();
        assert_eq!(
            forward.to_str().unwrap().replace('\\', "/"),
            descriptor,
            "{at}"
        );

        let view_kind = match kind.as_str() {
            "CommonForm" => "Form",
            "CommonModule" => "Module",
            kind => kind,
        };
        let view = read(authority, &at_text);
        assert_eq!(view["at"], at_text);
        assert_eq!(view["kind"], view_kind, "{at}");

        let resource = if kind == "CommonForm" {
            let metadata = format!("{at}.Metadata");
            let metadata_at = QualifiedAddress::parse(&metadata).unwrap();
            assert_eq!(read(authority, &metadata)["kind"], "Metadata", "{metadata}");
            assert_eq!(
                authority.existing_export_path(&metadata_at, None).unwrap(),
                descriptor
            );
            check_reverse(&metadata, "Metadata", &descriptor);
            let resource = format!("{directory}/{name}/Ext/Form.xml");
            let form_path = source.join(&resource);
            let raw = read_fixture_descriptor(&form_path);
            let form = parse_fixture_descriptor(&raw, &form_path);
            assert!(
                form.root_element().has_tag_name((NATIVE_FORM_NS, "Form")),
                "{at}"
            );
            assert_eq!(
                form.root_element().attribute("version"),
                Some(format),
                "{at}"
            );
            assert_eq!(authority.existing_export_path(&at, None).unwrap(), resource);
            check_reverse(&at_text, view_kind, &resource);
            resource
        } else {
            check_reverse(&at_text, kind, &descriptor);
            descriptor
        };
        evidence.record(
            authority.platform_profile().line(),
            "main:Configuration",
            kind,
            None,
            &fixture_origin(&source.join(resource)),
            AddressEvidence::RoundTrip(&at_text),
        );
    }
    for (branch, expected) in members {
        check_native_branch(authority, configuration, &branch, &expected);
    }
}

fn check_native_configuration_root_module_paths(
    format: &str,
    expected_modules: &BTreeMap<&str, &str>,
    evidence: &mut EvidenceRows,
) {
    let source = fixture_path(&format!(
        "formats/platform_xml/{format}/configuration_root_module_roles/src"
    ));
    let line = export_format_line(format).unwrap();
    let profile = PlatformProfile::for_line(line).unwrap();
    let cancellation = CancellationToken::new();
    let authority = native_authority(&cancellation, &source, profile);
    let root =
        unica_workspace::read::open_source_directory(&source.canonicalize().unwrap()).unwrap();
    let layout = [LayoutFindSource::new(
        "main",
        SourceSetKind::Configuration,
        &root,
    )];
    let finder = WorkspaceFindDirectoryBuilder::default();
    assert_eq!(
        addresses(&read(&authority, "main:Module"), "items"),
        expected_modules
            .keys()
            .map(|at| at.to_string())
            .collect::<BTreeSet<_>>(),
        "{line}: fixture root module roles",
    );
    for (&at, &role) in expected_modules {
        let relative = format!("Ext/{role}Module.bsl");
        let file = source.join(&relative);
        let view = read(&authority, at);
        assert_eq!(view["kind"], "Module", "{line}: {at}");
        assert_eq!(view["props"]["ownerKind"], "Configuration", "{line}: {at}");
        assert_eq!(view["props"]["role"], role, "{line}: {at}");
        let address = QualifiedAddress::parse(at).unwrap();
        assert_eq!(
            authority.existing_export_path(&address, None).unwrap(),
            relative,
        );
        let located = finder
            .locate_path(
                &layout,
                &relative,
                ProviderDeadline::no_deadline(),
                &cancellation,
            )
            .unwrap()
            .unwrap_or_else(|| panic!("{line}: missing native module {relative}"));
        assert_eq!(located.at(), at);
        assert_eq!(located.kind(), "Module");
        assert_eq!(located.placed_path(), Some(relative.as_str()));
        evidence.record(
            line,
            "main:Configuration",
            "Module",
            Some(role),
            &fixture_origin(&file),
            AddressEvidence::RoundTrip(at),
        );
        record_native_physical_module_body(&authority, at, &file, evidence);
    }
}

/// rule: CTR.SOURCE.QUALIFIED-LOGICAL-ADDRESS
#[test]
fn native_ordinary_interface_has_a_registration_only_root_branch() {
    let source = fixture_path("acceptance/workspace-ordinary-interface/src");
    let profile = PlatformProfile::v8_3_27();
    let cancellation = CancellationToken::new();
    let authority = native_authority(&cancellation, &source, profile);
    let mut evidence = EvidenceRows::new(
        "native_ordinary_interface_has_a_registration_only_root_branch",
        SourceSetKind::Configuration,
    );
    let origin = fixture_origin(&source.join("Configuration.xml"));
    let at = "main:OrdinaryInterface";
    assert!(addresses(&read(&authority, "main:Configuration"), "branches").contains(at));
    let branch = read(&authority, at);
    let items = addresses(&branch, "items");
    let config = fs::read_to_string(source.join("Configuration.xml")).unwrap();
    let document = roxmltree::Document::parse(config.trim_start_matches('\u{feff}')).unwrap();
    let registered = document
        .descendants()
        .filter(|node| {
            node.has_tag_name((MD_CLASSES_NS, "Interface"))
                && node
                    .parent()
                    .is_some_and(|parent| parent.has_tag_name((MD_CLASSES_NS, "ChildObjects")))
        })
        .map(|node| format!("{at}.{}", node.text().unwrap()))
        .collect::<BTreeSet<_>>();
    assert!(!registered.is_empty());
    assert_eq!(items, registered);
    evidence.record(
        profile.line(),
        "main:Configuration",
        "OrdinaryInterface",
        None,
        &origin,
        AddressEvidence::Read(at),
    );
    println!("| 8.3.27 | Configuration | OrdinaryInterface | {at} |");
    for item in items {
        let address = QualifiedAddress::parse(&item).unwrap();
        assert_eq!(address.segments()[0].kind(), NodeKind::OrdinaryInterface);
        read(&authority, &item);
        evidence.record(
            profile.line(),
            "main:Configuration",
            "OrdinaryInterface",
            None,
            &origin,
            AddressEvidence::Read(&item),
        );
        println!("| 8.3.27 | OrdinaryInterface | Member | {item} |");
    }
    for (format, line) in [
        ("2.21", PlatformLine::new(8, 5, 1)),
        ("2.22", PlatformLine::new(8, 5, 4)),
    ] {
        let source = fixture_path(&format!("formats/platform_xml/{format}/all_kinds/src"));
        assert!(!root_registrations(&source, format)
            .iter()
            .any(|(kind, _)| kind == "Interface"));
        let authority = native_authority(
            &cancellation,
            &source,
            PlatformProfile::for_line_or_base(Some(line)),
        );
        let view = read(&authority, at);
        assert_eq!(view["kind"], "OrdinaryInterface");
        assert!(view["items"].as_array().unwrap().is_empty(), "{line}: {at}");
        check_native_projection_refusal(
            &authority,
            "main:OrdinaryInterface.CovSwitchable_false",
            "ordinary application interface `CovSwitchable_false` is not registered",
        );
        evidence.record(
            line,
            "main:Configuration",
            "OrdinaryInterface",
            None,
            &fixture_origin(&source.join("Configuration.xml")),
            AddressEvidence::Read(at),
        );
    }
    evidence.finish();
}

/// rule: CTR.SOURCE.QUALIFIED-LOGICAL-ADDRESS
/// rule: INV.SOURCE.LOGICAL-BRANCH-COUNT
/// rule: INV.SOURCE.PROFILE-CAPABILITIES
#[test]
fn native_extension_root_language_matches_registration() {
    use unica_workspace::source_profile::platform_xml_source_profile;

    const OWNER: &str = "ext:Configuration";
    const BRANCH: &str = "ext:Language";
    const KIND: &str = "Language";
    const NAME: &str = "Русский";
    let mut evidence = EvidenceRows::new(
        "native_extension_root_language_matches_registration",
        SourceSetKind::Extension,
    );
    for (format, relative) in [
        ("2.20", "formats/platform_xml/2.20/empty_extension/ext"),
        ("2.21", "formats/platform_xml/2.21/empty_extension/ext"),
        ("2.22", "formats/platform_xml/2.22/empty_extension/ext"),
    ] {
        let source = fixture_path(relative);
        let config_path = source.join("Configuration.xml");
        let config_xml = read_fixture_descriptor(&config_path);
        let config = parse_fixture_descriptor(&config_xml, &config_path);
        assert_eq!(config.root_element().attribute("version"), Some(format));
        let extension = fixture_artifact(&config, "Configuration", &config_path);
        assert_eq!(fixture_properties_name(extension), Some("CorpusExtension"));
        let registered: Vec<_> = native_child_objects(extension).collect();
        let [language] = registered.as_slice() else {
            panic!("{format}: expected one native registration");
        };
        assert!(language.has_tag_name((MD_CLASSES_NS, KIND)));
        let name = language.text().expect("native Language registration name");
        assert_eq!(name, NAME, "{format}");
        let descriptor_path = source.join("Languages").join(format!("{name}.xml"));
        let descriptor_xml = read_fixture_descriptor(&descriptor_path);
        let descriptor = parse_fixture_descriptor(&descriptor_xml, &descriptor_path);
        assert_eq!(descriptor.root_element().attribute("version"), Some(format));
        assert_eq!(
            fixture_properties_name(fixture_artifact(&descriptor, KIND, &descriptor_path)),
            Some(name)
        );

        let source = fs::canonicalize(source).unwrap();
        let kind = SourceSetKind::Extension;
        let selected = platform_xml_source_profile(kind, &source);
        assert_eq!(
            selected.serialization_format(),
            Some(format),
            "{format}: detected extension profile"
        );
        let line = selected.semantic_line().unwrap();
        let cancellation = CancellationToken::new();
        let authority = LogicalViewReadAuthority::new(
            &cancellation,
            "ext",
            "address-coverage-native-extension",
            kind,
            Arc::new(unica_workspace::read::open_source_directory(&source).unwrap()),
            PlatformProfile::for_line(line).unwrap(),
        );
        let root = read(&authority, OWNER);
        assert_eq!(root["kind"], "Configuration");
        assert_eq!(root["props"]["name"], "CorpusExtension");
        let counts: Vec<_> = root["branches"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|row| row["at"] == BRANCH)
            .map(|row| row["count"].as_u64().unwrap())
            .collect();
        assert_eq!(counts, [1], "{format}: published {BRANCH}");

        let branch = QualifiedAddress::parse(BRANCH).unwrap();
        let at = branch.member(name).unwrap().to_string();
        let view = read(&authority, BRANCH);
        assert_eq!(addresses(&view, "items"), BTreeSet::from([at.clone()]));
        assert_eq!(read(&authority, &at)["kind"], KIND);
        evidence.record(
            line,
            OWNER,
            KIND,
            None,
            &fixture_origin(&config_path),
            AddressEvidence::Read(BRANCH),
        );
        evidence.record(
            line,
            OWNER,
            KIND,
            None,
            &fixture_origin(&descriptor_path),
            AddressEvidence::Read(&at),
        );
    }
    evidence.finish();
}

/// rule: INV.SOURCE.LOGICAL-READER-PARITY
/// rule: INV.SOURCE.RESOLVED-RESOURCE-IDENTITY
#[test]
fn native_cfe_ownership_addresses_follow_the_exported_registrations() {
    let mut evidence = EvidenceRows::new(
        "native_cfe_ownership_addresses_follow_the_exported_registrations",
        SourceSetKind::Extension,
    );
    for version in ["2.20", "2.21", "2.22"] {
        check_native_cfe_ownership(version, &mut evidence);
    }
    evidence.finish();
}

fn check_native_cfe_ownership(version: &str, evidence: &mut EvidenceRows) {
    use unica_workspace::source_profile::platform_xml_source_profile;
    use unica_workspace::test_support::tree_snapshot;

    const CATALOG_XML: &str = "Catalogs/CorpusCatalog.xml";
    const FORM_XML: &str = "Catalogs/CorpusCatalog/Forms/CorpusForm.xml";
    const FORM_BODY: &str = "Catalogs/CorpusCatalog/Forms/CorpusForm/Ext/Form.xml";
    const FORM_BSL: &str = "Catalogs/CorpusCatalog/Forms/CorpusForm/Ext/Form/Module.bsl";
    const MODULE_XML: &str = "CommonModules/CorpusExtension_OwnProbe.xml";
    const MODULE_BSL: &str = "CommonModules/CorpusExtension_OwnProbe/Ext/Module.bsl";
    const ROOT: &str = "ext:Configuration";
    const CATALOG: &str = "ext:Catalog.CorpusCatalog";
    const FORM: &str = "ext:Catalog.CorpusCatalog.Form.CorpusForm";
    const OWN: &str = "ext:CommonModule.CorpusExtension_OwnProbe";
    let line = export_format_line(version).unwrap();
    let profile = PlatformProfile::for_line(line).unwrap();
    let kind = SourceSetKind::Extension;
    let fixtures = fixture_path(&format!("acceptance/native-cfe-ownership-{version}"));
    let before = tree_snapshot(&fixtures);
    let cancellation = CancellationToken::new();
    let base = native_authority(&cancellation, &fixtures.join("cf"), profile);
    for (case, root_branch, named_address) in [
        ("adopted", "Catalog", CATALOG),
        ("own", "CommonModule", OWN),
    ] {
        let source = fixtures.join(case);
        let canonical = fs::canonicalize(&source).unwrap();
        assert_eq!(
            platform_xml_source_profile(kind, &canonical).serialization_format(),
            Some(version),
        );
        let directory = Arc::new(unica_workspace::read::open_source_directory(&canonical).unwrap());
        let authority = LogicalViewReadAuthority::new(
            &cancellation,
            "ext",
            "native-cfe-ownership",
            kind,
            Arc::clone(&directory),
            profile,
        );
        let layout = [LayoutFindSource::new("ext", kind, &directory)];
        let locate = |at: &str, relative: &str, expected_kind: &str| {
            let result = WorkspaceFindDirectoryBuilder::default()
                .locate_path(
                    &layout,
                    relative,
                    ProviderDeadline::no_deadline(),
                    &cancellation,
                )
                .unwrap()
                .unwrap_or_else(|| panic!("{relative}: no registered CFE address"));
            assert_eq!(result.at(), at, "{relative}");
            assert_eq!(result.kind(), expected_kind, "{relative}");
            assert_eq!(result.placed_path(), Some(relative), "{relative}");
        };
        let roundtrip = |address: &str, relative: &str, expected_kind: &str| {
            let at = QualifiedAddress::parse(address).unwrap();
            assert_eq!(authority.existing_export_path(&at, None).unwrap(), relative);
            locate(address, relative, expected_kind);
        };
        let not_registered = |reader: &LogicalViewReadAuthority<'_>, address: &str| {
            let at = QualifiedAddress::parse(address).unwrap();
            let failure = reader
                .snapshot(&at)
                .and_then(|snapshot| reader.read_exact(&at, &snapshot))
                .err()
                .expect("unregistered address must refuse");
            assert_eq!(failure.code(), RefusalCode::NotFound, "{at}: {failure:?}");
        };
        let mut record = |owner: &str,
                          branch: &str,
                          role: Option<&str>,
                          path: &str,
                          outcome: AddressEvidence<'_>| {
            evidence.record(
                line,
                owner,
                branch,
                role,
                &fixture_origin(&source.join(path)),
                outcome,
            );
        };
        let root_at = format!("ext:{root_branch}");
        check_native_branch(
            &authority,
            &read(&authority, ROOT),
            &root_at,
            &BTreeSet::from([named_address.to_owned()]),
        );
        record(
            ROOT,
            root_branch,
            None,
            "Configuration.xml",
            AddressEvidence::Read(&root_at),
        );
        let named = read(&authority, named_address);
        if case == "adopted" {
            assert_eq!(named["kind"], "Catalog");
            assert_eq!(named["props"]["belonging"], "borrowed");
            roundtrip(CATALOG, CATALOG_XML, "Catalog");
            record(
                ROOT,
                "Catalog",
                None,
                CATALOG_XML,
                AddressEvidence::RoundTrip(CATALOG),
            );
            let forms = format!("{CATALOG}.Form");
            check_native_branch(
                &authority,
                &named,
                &forms,
                &BTreeSet::from([FORM.to_owned()]),
            );
            record(
                CATALOG,
                "Form",
                None,
                CATALOG_XML,
                AddressEvidence::Read(&forms),
            );
            let form_view = read(&authority, FORM);
            assert_eq!(form_view["kind"], "Form");
            assert_eq!(form_view["props"]["isExtension"], true);
            assert_eq!(form_view["props"]["baseFormVersion"], version);
            roundtrip(FORM, FORM_BODY, "Form");
            record(
                CATALOG,
                "Form",
                None,
                FORM_BODY,
                AddressEvidence::RoundTrip(FORM),
            );
            let metadata = format!("{FORM}.Metadata");
            assert_eq!(read(&authority, &metadata)["kind"], "Metadata");
            roundtrip(&metadata, FORM_XML, "Metadata");
            record(
                FORM,
                "Metadata",
                None,
                FORM_XML,
                AddressEvidence::RoundTrip(&metadata),
            );
            let group = format!("{FORM}.Module");
            let module = format!("{group}.Form");
            check_native_branch(
                &authority,
                &form_view,
                &group,
                &BTreeSet::from([module.clone()]),
            );
            record(
                FORM,
                "Module",
                None,
                FORM_XML,
                AddressEvidence::Read(&group),
            );
            assert_eq!(read(&authority, &module)["kind"], "Module");
            roundtrip(&module, FORM_BSL, "Module");
            record(
                FORM,
                "Module",
                Some("Form"),
                FORM_BSL,
                AddressEvidence::RoundTrip(&module),
            );
            let body = format!("{module}.Body");
            let content = fs::read_to_string(source.join(FORM_BSL)).unwrap();
            let text = content.trim_start_matches('\u{feff}');
            check_native_bsl_body(&authority, &body, text, 1, text.lines().count());
            record(
                &module,
                "Body",
                None,
                FORM_BSL,
                AddressEvidence::Read(&body),
            );
            not_registered(&authority, &format!("{CATALOG}.Form.NotRegistered"));
        } else {
            assert_eq!(named["kind"], "Module");
            let own = QualifiedAddress::parse(OWN).unwrap();
            let borrowing = authority
                .object_borrowing(&own)
                .expect("read the CommonModule descriptor")
                .expect("the CommonModule belongs to the extension");
            assert!(borrowing.extends.is_none(), "{OWN} is declared as own");
            record(
                ROOT,
                "CommonModule",
                None,
                MODULE_XML,
                AddressEvidence::Read(OWN),
            );
            // The descriptor and BSL are separate resources even though reverse
            // location names the same CommonModule owner for both.
            locate(OWN, MODULE_XML, "CommonModule");
            locate(OWN, MODULE_BSL, "CommonModule");
            let body = format!("{OWN}.Body");
            let content = fs::read_to_string(source.join(MODULE_BSL)).unwrap();
            let text = content.trim_start_matches('\u{feff}');
            check_native_bsl_body(&authority, &body, text, 1, text.lines().count());
            let at = QualifiedAddress::parse(&body).unwrap();
            assert_eq!(
                authority.existing_export_path(&at, None).unwrap(),
                MODULE_BSL
            );
            record(OWN, "Body", None, MODULE_BSL, AddressEvidence::Read(&body));
            not_registered(&authority, CATALOG);
            not_registered(&base, "main:CommonModule.CorpusExtension_OwnProbe");
        }
    }
    assert_eq!(tree_snapshot(&fixtures), before);
}

/// rule: INV.SOURCE.EXTERNAL-ARTIFACT-ROOTS
/// rule: INV.SOURCE.LOGICAL-BRANCH-COUNT
/// rule: INV.SOURCE.RESOLVED-RESOURCE-IDENTITY
#[test]
fn native_external_root_descriptors_match_published_addresses() {
    use unica_workspace::source_profile::platform_xml_source_profile;

    let mut required = RequiredKeys::new("required_native_external_child_keys.jsonl");
    let mut evidence = EvidenceRows::new(
        "native_external_root_descriptors_match_published_addresses",
        SourceSetKind::ExternalProcessor,
    );
    for (expected_format, source_kind, relative) in [
        (
            "2.20",
            SourceSetKind::ExternalProcessor,
            "formats/platform_xml/2.20/external_artifacts/epf/SyntheticProcessor.xml",
        ),
        (
            "2.20",
            SourceSetKind::ExternalReport,
            "formats/platform_xml/2.20/external_artifacts/erf/SyntheticReport.xml",
        ),
        (
            "2.21",
            SourceSetKind::ExternalProcessor,
            "formats/platform_xml/2.21/empty_external_processor/epf/EmptyProcessor.xml",
        ),
        (
            "2.21",
            SourceSetKind::ExternalReport,
            "formats/platform_xml/2.21/empty_external_report/erf/EmptyReport.xml",
        ),
        (
            "2.22",
            SourceSetKind::ExternalProcessor,
            "formats/platform_xml/2.22/empty_external_processor/epf/EmptyProcessor.xml",
        ),
        (
            "2.22",
            SourceSetKind::ExternalReport,
            "formats/platform_xml/2.22/empty_external_report/erf/EmptyReport.xml",
        ),
    ] {
        let (alias, kind) = match source_kind {
            SourceSetKind::ExternalProcessor => ("epf", "ExternalDataProcessor"),
            SourceSetKind::ExternalReport => ("erf", "ExternalReport"),
            _ => unreachable!("external fixtures use external source kinds"),
        };
        let descriptor_path = fixture_path(relative);
        let file_name = descriptor_path.file_name().unwrap().to_str().unwrap();
        let descriptor_xml = read_fixture_descriptor(&descriptor_path);
        let descriptor = parse_fixture_descriptor(&descriptor_xml, &descriptor_path);
        let format = descriptor.root_element().attribute("version").unwrap();
        assert_eq!(format, expected_format, "{relative}");
        let artifact = fixture_artifact(&descriptor, kind, &descriptor_path);
        let name = fixture_properties_name(artifact).expect("external artifact Properties.Name");
        assert_eq!(descriptor_path.file_stem().unwrap().to_str(), Some(name));
        let line = export_format_line(format).unwrap();

        let source = descriptor_path.parent().unwrap().canonicalize().unwrap();
        let admitted = platform_xml_source_profile(source_kind, &source);
        let profile = PlatformProfile::for_line_or_base(admitted.semantic_line());
        println!(
            "{relative}: export XML {format}, line {line}; admitted profile {}, route profile {}",
            admitted.canonical_id(),
            profile.id(),
        );
        let root = Arc::new(unica_workspace::read::open_source_directory(&source).unwrap());
        let cancellation = CancellationToken::new();
        let authority = LogicalViewReadAuthority::new(
            &cancellation,
            alias,
            "address-coverage-native-external-root",
            source_kind,
            Arc::clone(&root),
            profile,
        );
        let owner = format!("{alias}:Configuration");
        let branch = format!("{alias}:{kind}");
        let view = read(&authority, &owner);
        assert_eq!(view["kind"], "Configuration", "{relative}");
        assert_eq!(
            addresses(&view, "branches"),
            BTreeSet::from([branch.clone()])
        );
        assert_eq!(view["branches"][0]["count"], 1, "{relative}");
        assert!(
            !source.join("Configuration.xml").exists(),
            "{relative}: virtual root"
        );

        let at = QualifiedAddress::parse(&branch)
            .unwrap()
            .member(name)
            .unwrap();
        let at_text = at.to_string();
        let collection = read(&authority, &branch);
        assert_eq!(
            addresses(&collection, "items"),
            BTreeSet::from([at_text.clone()])
        );
        assert_eq!(read(&authority, &at_text)["kind"], kind, "{relative}");
        assert_eq!(
            authority.existing_export_path(&at, None).unwrap(),
            file_name,
            "{relative}"
        );
        let layout = [LayoutFindSource::new(alias, source_kind, &root)];
        let located = WorkspaceFindDirectoryBuilder::default()
            .locate_path(
                &layout,
                file_name,
                ProviderDeadline::no_deadline(),
                &cancellation,
            )
            .unwrap()
            .unwrap_or_else(|| panic!("missing source identity for {relative}"));
        assert_eq!(located.at(), at_text, "{relative}");
        assert_eq!(located.placed_path(), Some(file_name), "{relative}");

        let origin = fixture_origin(&descriptor_path);
        evidence.record_for_source(
            (line, source_kind),
            &owner,
            kind,
            None,
            &origin,
            AddressEvidence::Read(&branch),
        );
        evidence.record_for_source(
            (line, source_kind),
            &owner,
            kind,
            None,
            &origin,
            AddressEvidence::RoundTrip(&at_text),
        );
        for &child_kind in platform_child_object_order(kind).unwrap() {
            // These children can have their own files; inline registrations
            // are checked below against their containing descriptor.
            if !matches!(child_kind, "Form" | "Template" | "Command") {
                continue;
            }
            required.record(
                (line, source_kind),
                &at,
                child_kind,
                None,
                AddressRequirement::Address,
            );
            let child_branch = QualifiedAddress::parse(&format!("{at_text}.{child_kind}")).unwrap();
            let registered: BTreeSet<_> = artifact
                .children()
                .filter(|node| node.has_tag_name((MD_CLASSES_NS, "ChildObjects")))
                .flat_map(|children| children.children())
                .filter(|node| node.has_tag_name((MD_CLASSES_NS, child_kind)))
                .map(|child| {
                    let child_name = if child.children().any(|node| node.is_element()) {
                        fixture_properties_name(child).expect("external child Properties.Name")
                    } else {
                        child.text().expect("external child registration").trim()
                    };
                    child_branch.member(child_name).unwrap().to_string()
                })
                .collect();
            let child_branch = child_branch.to_string();
            assert_eq!(
                addresses(&read(&authority, &child_branch), "items"),
                registered,
                "{relative}: {child_kind} follows the native registrations"
            );
            evidence.record_for_source(
                (line, source_kind),
                &at_text,
                child_kind,
                None,
                &origin,
                AddressEvidence::Read(&child_branch),
            );
        }
        check_native_modules_without_bsl(
            &authority,
            descriptor_path.parent().unwrap(),
            &layout,
            &cancellation,
            MissingNativeModules {
                line,
                owner: &at,
                descriptor: &descriptor_path,
                group_roles: Some(&["Object"]),
                roles: if format == "2.20" { &[] } else { &["Object"] },
            },
            &mut evidence,
        );
        if format == "2.20" {
            check_native_external_inline(
                &authority,
                (line, source_kind),
                &at_text,
                artifact,
                &descriptor_path,
                &mut evidence,
            );
            let child =
                native_registered_form_descriptor(artifact, &descriptor_path, "Main", format);
            let form = QualifiedAddress::parse(&format!("{at_text}.Form.Main")).unwrap();
            record_native_form_module_group(
                &authority,
                (line, source_kind),
                &form,
                &child,
                &mut evidence,
            );
        }
        if format == "2.20" && source_kind == SourceSetKind::ExternalReport {
            check_native_external_dcs_parameters(
                &authority,
                &at_text,
                artifact,
                &descriptor_path,
                line,
                &mut evidence,
            );
        }
    }
    for format in ["2.21", "2.22"] {
        for (source_kind, artifact) in [
            (
                SourceSetKind::ExternalProcessor,
                "epf/SyntheticProcessor.xml",
            ),
            (SourceSetKind::ExternalReport, "erf/SyntheticReport.xml"),
        ] {
            check_native_external_form_and_object_modules(
                format,
                source_kind,
                &format!("formats/platform_xml/{format}/external_artifacts/{artifact}"),
                &mut evidence,
            );
        }
    }
    required.finish();
    evidence.finish();
}

/// rule: CTR.SOURCE.QUALIFIED-LOGICAL-ADDRESS
/// rule: INV.SOURCE.LOGICAL-BRANCH-COUNT
/// rule: INV.SOURCE.PLATFORM-CAPABILITY-EXISTENCE
#[test]
fn native_recalculation_module_address_table_matches_registered_owner() {
    let mut evidence = EvidenceRows::new(
        "native_recalculation_module_address_table_matches_registered_owner",
        SourceSetKind::Configuration,
    );
    for format in ["2.20", "2.21", "2.22"] {
        check_native_recalculation_modules(format, &mut evidence);
    }
    evidence.finish();
}

fn check_native_recalculation_modules(format: &str, evidence: &mut EvidenceRows) {
    let source = fixture_path(&format!("formats/platform_xml/{format}/recalculation/src"));
    let line = export_format_line(format).unwrap();
    let profile = PlatformProfile::for_line(line).unwrap();
    let cancellation = CancellationToken::new();
    let authority = native_authority(&cancellation, &source, profile);
    let root =
        unica_workspace::read::open_source_directory(&source.canonicalize().unwrap()).unwrap();
    let layout = [LayoutFindSource::new(
        "main",
        SourceSetKind::Configuration,
        &root,
    )];
    let finder = WorkspaceFindDirectoryBuilder::default();
    let register = "main:CalculationRegister.Начисления";
    assert_eq!(
        root_registrations(&source, format)
            .iter()
            .filter(|(kind, name)| kind == "CalculationRegister" && name == "Начисления")
            .count(),
        1,
        "{format}: the calculation register has one native registration"
    );
    let parent_branch = format!("{register}.Recalculation");
    let descriptor = source.join("CalculationRegisters/Начисления.xml");
    let xml = read_fixture_descriptor(&descriptor);
    let document = parse_fixture_descriptor(&xml, &descriptor);
    assert_eq!(document.root_element().attribute("version"), Some(format));
    let artifact = fixture_artifact(&document, "CalculationRegister", &descriptor);
    let mut branches = native_described_branches(register, &descriptor, artifact);
    let native_owners = branches
        .remove(&parent_branch)
        .expect("native fixture must register a Recalculation");

    let parent = read(&authority, register);
    check_native_branch(&authority, &parent, &parent_branch, &native_owners);
    let owners = native_owners;
    let origin = fixture_origin(&descriptor);
    evidence.record(
        profile.line(),
        register,
        "Recalculation",
        None,
        &origin,
        AddressEvidence::Read(&parent_branch),
    );
    println!(
        "| {} | CalculationRegister | Recalculation | {parent_branch} |",
        profile.line()
    );
    for owner in owners {
        let node = read(&authority, &owner);
        assert_eq!(node["kind"], "Recalculation", "{owner}");
        evidence.record(
            profile.line(),
            register,
            "Recalculation",
            None,
            &origin,
            AddressEvidence::Read(&owner),
        );
        let owner_address = QualifiedAddress::parse(&owner).unwrap();
        let name = owner_address.segments().last().unwrap().name().unwrap();
        let relative_owner = format!("CalculationRegisters/Начисления/Recalculations/{name}");
        let owner_descriptor = source.join(format!("{relative_owner}.xml"));
        let raw = read_fixture_descriptor(&owner_descriptor);
        let document = parse_fixture_descriptor(&raw, &owner_descriptor);
        assert_eq!(document.root_element().attribute("version"), Some(format));
        let artifact = fixture_artifact(&document, "Recalculation", &owner_descriptor);
        assert_eq!(fixture_properties_name(artifact), Some(name));
        assert!(uuid::Uuid::parse_str(artifact.attribute("uuid").unwrap()).is_ok());

        let branch = format!("{owner}.Module");
        let role = format!("{branch}.RecordSet");
        check_native_branch(&authority, &node, &branch, &BTreeSet::from([role.clone()]));
        evidence.record(
            profile.line(),
            &owner,
            "Module",
            None,
            &fixture_origin(&owner_descriptor),
            AddressEvidence::Read(&branch),
        );
        let at = QualifiedAddress::parse(&role).unwrap();
        assert_eq!(
            route_logical_address(&at, profile).unwrap().reader(),
            LogicalReader::Module
        );
        let module = read(&authority, &role);
        assert_eq!(module["kind"], "Module");
        assert!(addresses(&module, "branches").contains(&format!("{role}.Body")));
        let relative = format!("{relative_owner}/Ext/RecordSetModule.bsl");
        let file = source.join(&relative);
        assert!(file.is_file(), "{relative}");
        assert_eq!(authority.existing_export_path(&at, None).unwrap(), relative);
        let placed = finder
            .locate_path(
                &layout,
                &relative,
                ProviderDeadline::no_deadline(),
                &cancellation,
            )
            .unwrap()
            .unwrap();
        assert_eq!(placed.at(), role);
        assert_eq!(placed.placed_path(), Some(relative.as_str()));
        evidence.record(
            profile.line(),
            &owner,
            "Module",
            Some("RecordSet"),
            &fixture_origin(&file),
            AddressEvidence::RoundTrip(&role),
        );
        record_native_bsl_body(
            &authority,
            (profile.line(), SourceSetKind::Configuration),
            &role,
            &file,
            evidence,
        );
    }
}

/// rule: CTR.SOURCE.QUALIFIED-LOGICAL-ADDRESS
/// rule: CTR.SOURCE.EMPTY-BRANCHES
#[test]
fn native_metadata_address_table_matches_declared_and_observed_child_branches() {
    let mut evidence = EvidenceRows::new(
        "native_metadata_address_table_matches_declared_and_observed_child_branches",
        SourceSetKind::Configuration,
    );
    let mut required = RequiredKeys::new("required_native_metadata_child_keys.jsonl");
    let mut required_eds = RequiredKeys::new("required_native_eds_module_keys.jsonl");
    println!("| Profile | Owner | Branch | Canonical address | Evidence |");
    println!("| --- | --- | --- | --- | --- |");
    let fixtures = [
        ("2.20", PlatformLine::new(8, 3, 27)),
        ("2.21", PlatformLine::new(8, 5, 1)),
        ("2.22", PlatformLine::new(8, 5, 4)),
    ]
    .map(|(xml, line)| {
        let source = fixture_path(&format!("formats/platform_xml/{xml}/all_kinds/src"));
        let source_profile = unica_workspace::source_profile::platform_xml_source_profile(
            SourceSetKind::Configuration,
            &source.canonicalize().unwrap(),
        );
        assert_eq!(source_profile.serialization_format(), Some(xml));
        assert_eq!(source_profile.semantic_line(), Some(line));
        let profile = PlatformProfile::for_line_or_base(source_profile.semantic_line());
        let registrations = root_registrations(&source, xml);
        record_required_metadata_children(&source, xml, profile, &registrations, &mut required);
        for (owner, _, roles) in EDS_MODULE_OWNERS {
            let owner = QualifiedAddress::parse(owner).unwrap();
            let source = (line, SourceSetKind::Configuration);
            required_eds.record(source, &owner, "Module", None, AddressRequirement::Read);
            for &role in roles {
                required_eds.record(
                    source,
                    &owner,
                    "Module",
                    Some(role),
                    AddressRequirement::RoundTrip,
                );
            }
        }
        (source, xml, profile, registrations)
    });
    required.finish();
    required_eds.finish();
    for (source, xml, profile, registrations) in fixtures {
        let line = profile.line();
        let cancellation = CancellationToken::new();
        let authority = native_authority(&cancellation, &source, profile);
        check_native_empty_nested_collections(
            &authority,
            &source,
            xml,
            &registrations,
            &mut evidence,
        );
        check_native_declaration_projections(&authority, &source, &registrations, &mut evidence);
        check_native_projection_companions(
            &authority,
            &source,
            &registrations,
            &cancellation,
            &mut evidence,
        );
        check_native_job_schedule_roundtrip(&source, xml, profile, &cancellation, &mut evidence);
        let mut verified = BTreeSet::new();
        for (kind, name) in registrations {
            let at = QualifiedAddress::parse(&format!("main:{kind}.{name}")).unwrap();
            let route = route_logical_address(&at, profile).unwrap();
            if route.reader() == LogicalReader::Metadata {
                check_metadata_owner(&authority, &source, &at, &mut verified, &mut evidence);
            } else if route.reader() == LogicalReader::Xdto && name == "EnterpriseData_1_17_3" {
                check_native_xdto_package(&authority, &source, &at, xml, &mut evidence);
                assert!(verified.insert(format!("{at}.Type")), "duplicate {at}");
            } else if route.reader() == LogicalReader::Role && name == "SalesReader" {
                check_native_sales_role_rights(&authority, &source, &at, xml, &mut evidence);
                assert!(verified.insert(format!("{at}.Right")), "duplicate {at}");
            } else if route.reader() == LogicalReader::Subsystem && name == "СтандартныеПодсистемы"
            {
                check_native_subsystem(&authority, &source, &at, xml, &mut evidence);
                assert!(verified.insert(format!("{at}.Subsystem")), "duplicate {at}");
            }
        }
        assert!(verified.contains("main:XDTOPackage.EnterpriseData_1_17_3.Type"));
        assert!(verified.contains("main:Role.SalesReader.Right"));
        assert!(verified.contains("main:Subsystem.СтандартныеПодсистемы.Subsystem"));
        assert!(
            verified.contains(
                "main:ExternalDataSource.AllKindsExternalSource.Cube.Sales.DimensionTable"
            ),
            "XML {xml}: native fixture must witness a registered DimensionTable under Cube.Sales"
        );
        for branch in [
            "main:WebService.ОбменДанными.Operation.ПринятьПакет.Parameter",
            "main:HTTPService.ПроверкаСвязи.URLTemplate.Пинг.Method",
        ] {
            assert!(
                verified.contains(branch),
                "XML {xml}: native fixture must witness inline {branch}"
            );
        }
        if metadata_kind_exists(Some(line), NodeKind::Queue.as_str()) {
            for branch in ["main:Queue.Probe.Attribute", "main:Queue.Probe.Dimension"] {
                assert!(
                    verified.contains(branch),
                    "XML {xml}: native fixture must witness inline {branch}"
                );
            }
        }
        check_eds_module_rows(&authority, &source, &cancellation, &mut evidence);
    }
    evidence.finish();
}

fn record_required_metadata_children(
    source: &Path,
    xml: &str,
    profile: PlatformProfile,
    registrations: &[(String, String)],
    required: &mut RequiredKeys,
) {
    if required.output.is_none() {
        return;
    }
    for (kind, name) in registrations {
        let Some(declared) = platform_child_object_order(kind) else {
            continue;
        };
        let owner = QualifiedAddress::parse(&format!("main:{kind}.{name}")).unwrap();
        if route_logical_address(&owner, profile).unwrap().reader() != LogicalReader::Metadata {
            continue;
        }
        assert!(
            metadata_kind_exists(Some(profile.line()), kind),
            "{xml}: unadmitted {owner}"
        );
        let directory = METADATA_LAYOUTS
            .iter()
            .find(|layout| layout.tag == kind)
            .unwrap()
            .directory;
        let descriptor = source.join(directory).join(format!("{name}.xml"));
        let raw = read_fixture_descriptor(&descriptor);
        let document = parse_fixture_descriptor(&raw, &descriptor);
        assert_eq!(document.root_element().attribute("version"), Some(xml));
        let artifact = fixture_artifact(&document, kind, &descriptor);
        assert_eq!(fixture_properties_name(artifact), Some(name.as_str()));
        assert!(uuid::Uuid::parse_str(artifact.attribute("uuid").unwrap()).is_ok());
        for &branch in declared {
            if branch == "AddressingAttribute" {
                // This conditional collection is known when its native container is present.
                let containers: Vec<_> = artifact
                    .children()
                    .filter(|node| node.is_element() && node.tag_name().name() == "ChildObjects")
                    .collect();
                assert_eq!(
                    containers.len(),
                    1,
                    "{owner}: native addressing eligibility"
                );
                assert!(containers[0].has_tag_name((MD_CLASSES_NS, "ChildObjects")));
            }
            assert!(
                metadata_kind_exists(Some(profile.line()), branch),
                "{xml}: declared child {owner}.{branch} has no admitted platform kind"
            );
            required.record(
                (profile.line(), SourceSetKind::Configuration),
                &owner,
                branch,
                None,
                AddressRequirement::Address,
            );
        }
    }
}

fn record_required_configuration_module_roles(fixture: &ModuleCapabilityFixture) {
    let mut required = RequiredKeys::new("required_configuration_module_role_keys.jsonl");
    let line = PlatformLine::parse(&fixture.profile).expect("address profile platform line");
    let configuration = QualifiedAddress::parse("main:Configuration").unwrap();
    for case in &fixture.module_capabilities {
        if !case.exists || !case.at.starts_with("main:") {
            continue;
        }
        let at = QualifiedAddress::parse(&case.at).expect("profile module address");
        assert_eq!(at.to_string(), case.at);
        let role = case.role.as_deref().expect("profile module role");
        let owner = match at.segments() {
            [common] if common.kind() == NodeKind::CommonModule => {
                assert!(common.name().is_some(), "{at}: expected named CommonModule");
                assert_eq!(role, "Common", "{at}");
                at.clone()
            }
            [owners @ .., module] if module.kind() == NodeKind::Module => {
                assert_eq!(module.name(), Some(role), "{at}");
                if owners.is_empty() {
                    configuration.clone()
                } else {
                    assert!(owners.iter().all(|owner| owner.name().is_some()), "{at}");
                    at.prefix(owners.len())
                }
            }
            _ => panic!("{at}: unsupported positive module shape in profile"),
        };
        required.record(
            (line, SourceSetKind::Configuration),
            &owner,
            "Module",
            Some(role),
            AddressRequirement::Address,
        );
    }
    required.finish();
}

/// rule: CTR.SOURCE.QUALIFIED-LOGICAL-ADDRESS
/// rule: INV.SOURCE.RESOLVED-RESOURCE-IDENTITY
#[test]
fn native_form_and_template_addresses_follow_text_registrations() {
    let mut evidence = EvidenceRows::new(
        "native_form_and_template_addresses_follow_text_registrations",
        SourceSetKind::Configuration,
    );
    for xml in ["2.20", "2.21", "2.22"] {
        let source = fixture_path(&format!("formats/platform_xml/{xml}/all_kinds/src"));
        let line = export_format_line(xml).unwrap();
        let profile = PlatformProfile::for_line(line).unwrap();
        let cancellation = CancellationToken::new();
        let authority = native_authority(&cancellation, &source, profile);
        let root =
            unica_workspace::read::open_source_directory(&source.canonicalize().unwrap()).unwrap();
        let layout = [LayoutFindSource::new(
            "main",
            SourceSetKind::Configuration,
            &root,
        )];
        let finder = WorkspaceFindDirectoryBuilder::default();
        let verify_path = |at: &QualifiedAddress, path: &Path| {
            let relative = path
                .strip_prefix(&source)
                .unwrap()
                .to_str()
                .unwrap()
                .replace('\\', "/");
            if matches!(
                at.segments().last().unwrap().kind(),
                NodeKind::Template | NodeKind::CommonTemplate
            ) {
                // resolve(at) uses the descriptor layout for a template, while
                // the typed view of a nested template reads through its owner.
                let target = unica_address::source_target::MetadataAddress::parse_logical(
                    &at.logical_path(),
                )
                .unwrap();
                let forward = unica_source_designer::layout::metadata_descriptor_relative(
                    &target,
                    SourceSetKind::Configuration,
                )
                .unwrap();
                assert_eq!(forward.to_str().unwrap().replace('\\', "/"), relative);
            } else {
                assert_eq!(authority.existing_export_path(at, None).unwrap(), relative);
            }
            let located = finder
                .locate_path(
                    &layout,
                    &relative,
                    ProviderDeadline::no_deadline(),
                    &cancellation,
                )
                .unwrap()
                .unwrap_or_else(|| panic!("missing source identity {relative} for {at}"));
            assert_eq!(located.at(), at.to_string(), "{relative}");
        };
        let registrations = root_registrations(&source, xml);
        let mut covered = BTreeSet::new();
        let configuration = read(&authority, "main:Configuration");
        for kind in ["CommonForm", "CommonTemplate"] {
            let branch = QualifiedAddress::parse(&format!("main:{kind}")).unwrap();
            let mut expected = BTreeSet::new();
            for (_, name) in registrations.iter().filter(|(tag, _)| tag == kind) {
                let at = branch.member(name).unwrap();
                let descriptor = source.join(format!("{kind}s/{name}.xml"));
                check_native_form_or_template(
                    &authority,
                    NativeFormOrTemplateSample {
                        at: &at,
                        kind,
                        name,
                        format: xml,
                        descriptor: &descriptor,
                    },
                    &verify_path,
                    &mut covered,
                    &mut evidence,
                );
                let resource = if kind == "CommonForm" {
                    descriptor.with_extension("").join("Ext/Form.xml")
                } else {
                    descriptor.clone()
                };
                evidence.record(
                    line,
                    "main:Configuration",
                    kind,
                    None,
                    &fixture_origin(&resource),
                    AddressEvidence::RoundTrip(&at.to_string()),
                );
                assert!(expected.insert(at.to_string()), "duplicate native {at}");
            }
            assert!(
                !expected.is_empty(),
                "XML {xml}: native fixture needs a {kind}"
            );
            let branch = branch.to_string();
            check_native_branch(&authority, &configuration, &branch, &expected);
            evidence.record(
                line,
                "main:Configuration",
                kind,
                None,
                &fixture_origin(&source.join("Configuration.xml")),
                AddressEvidence::Read(&branch),
            );
        }

        for (kind, name) in registrations {
            let directory = METADATA_LAYOUTS
                .iter()
                .find(|layout| layout.tag == kind)
                .unwrap()
                .directory;
            let descriptor = source.join(directory).join(format!("{name}.xml"));
            let raw = read_fixture_descriptor(&descriptor);
            let document = parse_fixture_descriptor(&raw, &descriptor);
            let artifact = fixture_artifact(&document, &kind, &descriptor);
            for (child_kind, directory) in [("Form", "Forms"), ("Template", "Templates")] {
                let mut names = native_child_objects(artifact)
                    .filter(|child| child.has_tag_name((MD_CLASSES_NS, child_kind)))
                    .peekable();
                if names.peek().is_none() {
                    continue;
                }
                let owner_at = format!("main:{kind}.{name}");
                let branch = QualifiedAddress::parse(&format!("{owner_at}.{child_kind}")).unwrap();
                let mut expected = BTreeSet::new();
                for child in names {
                    assert!(
                        child.attribute("uuid").is_none()
                            && !child.children().any(|node| node.is_element()),
                        "native fixture {descriptor:?}: {child_kind} must be a text registration"
                    );
                    let child_name = child
                        .text()
                        .map(str::trim)
                        .filter(|name| !name.is_empty())
                        .unwrap_or_else(|| {
                            panic!("native fixture {descriptor:?}: empty {child_kind} registration")
                        });
                    let at = branch.member(child_name).unwrap();
                    let descriptor = descriptor
                        .with_extension("")
                        .join(directory)
                        .join(format!("{child_name}.xml"));
                    check_native_form_or_template(
                        &authority,
                        NativeFormOrTemplateSample {
                            at: &at,
                            kind: child_kind,
                            name: child_name,
                            format: xml,
                            descriptor: &descriptor,
                        },
                        &verify_path,
                        &mut covered,
                        &mut evidence,
                    );
                    let address = at.to_string();
                    let resource = if child_kind == "Form" {
                        descriptor.with_extension("").join("Ext/Form.xml")
                    } else {
                        descriptor
                    };
                    evidence.record(
                        line,
                        &owner_at,
                        child_kind,
                        None,
                        &fixture_origin(&resource),
                        AddressEvidence::RoundTrip(&address),
                    );
                    assert!(expected.insert(address), "duplicate native {at}");
                }
                let owner = read(&authority, &owner_at);
                let branch = branch.to_string();
                check_native_branch(&authority, &owner, &branch, &expected);
                evidence.record(
                    line,
                    &owner_at,
                    child_kind,
                    None,
                    &fixture_origin(&descriptor),
                    AddressEvidence::Read(&branch),
                );
                covered.insert(branch);
            }
        }
        for witness in [
            "main:Catalog.Валюты.Form",
            "main:DataProcessor.Обработка.Form",
            "main:DataProcessor.Обработка.Form.Форма.Item",
            "main:Catalog.Валюты.Form.ФормаСписка.Item.List.Item",
            "main:Catalog.Валюты.Form.ФормаЭлемента.Item",
            "main:CommonForm.MainA.Item",
            "main:DataProcessor.Обработка.Template",
            "main:Report.АнализВерсийОбъектов.Template",
            "main:CommonForm.MainA.Metadata",
            "main:CommonForm.MainA.Module.Form",
            "main:DataProcessor.Обработка.Template.СхемаКомпоновки.DataSet",
            "main:Report.АнализВерсийОбъектов.Template.ОсновнаяСхемаКомпоновкиДанных.DataSet",
        ] {
            assert!(
                covered.contains(witness),
                "XML {xml}: native fixture must witness {witness}"
            );
        }
    }
    check_native_mxl_areas(&mut evidence);
    check_native_html_processor_form_module(&mut evidence);
    check_native_command_and_bound_events(&mut evidence);
    evidence.finish();
}

const MXL_NS: &str = "http://v8.1c.ru/8.2/data/spreadsheet";

fn check_native_mxl_areas(evidence: &mut EvidenceRows) {
    const OWNER: &str = "main:Report.АнализВерсийОбъектов.Template.МакетОтчета";
    const RESOURCE: &str = "Reports/АнализВерсийОбъектов/Templates/МакетОтчета/Ext/Template.xml";
    const REPORT: &str = "Reports/АнализВерсийОбъектов.xml";
    for xml in ["2.21", "2.22"] {
        let source = fixture_path(&format!("formats/platform_xml/{xml}/all_kinds/src"));
        let native = fixture_path(&format!("formats/platform_xml/{xml}/mxl-areas/{RESOURCE}"));
        let report = source.join(REPORT);
        let raw = read_fixture_descriptor(&report);
        let document = parse_fixture_descriptor(&raw, &report);
        let artifact = fixture_artifact(&document, "Report", &report);

        // Keep the original all_kinds export intact. Reading the template owner
        // also validates the other template descriptors registered in this report.
        let overlay = tempfile::tempdir().unwrap();
        let copy = |from: &Path, relative: &Path| {
            let to = overlay.path().join(relative);
            fs::create_dir_all(to.parent().unwrap()).unwrap();
            fs::copy(from, to).unwrap();
        };
        for relative in ["Configuration.xml", REPORT] {
            copy(&source.join(relative), Path::new(relative));
        }
        for child in native_child_objects(artifact)
            .filter(|node| node.has_tag_name((MD_CLASSES_NS, "Template")))
        {
            let name = child.text().unwrap();
            let relative = Path::new(REPORT)
                .with_extension("")
                .join("Templates")
                .join(format!("{name}.xml"));
            copy(&source.join(&relative), &relative);
        }
        copy(&native, Path::new(RESOURCE));

        let cancellation = CancellationToken::new();
        let line = export_format_line(xml).unwrap();
        let profile = PlatformProfile::for_line(line).unwrap();
        let authority = native_authority(&cancellation, overlay.path(), profile);
        let origin = fixture_origin(&native);
        let raw = read_fixture_descriptor(&native);
        let document = parse_fixture_descriptor(&raw, &native);
        let root = document.root_element();
        assert!(root.has_tag_name((MXL_NS, "document")));
        let areas: BTreeMap<_, _> = root
            .children()
            .filter(|node| node.has_tag_name((MXL_NS, "namedItem")))
            .map(|node| {
                assert_eq!(
                    node.attribute(("http://www.w3.org/2001/XMLSchema-instance", "type")),
                    Some("NamedItemCells")
                );
                (
                    mxl_child(node, "name").unwrap().text().unwrap(),
                    mxl_child(node, "area").unwrap(),
                )
            })
            .collect();
        assert_eq!(
            areas.keys().copied().collect::<BTreeSet<_>>(),
            BTreeSet::from(["Колонки", "Подвал", "Строка", "Угол", "Шапка"])
        );
        let branch = QualifiedAddress::parse(&format!("{OWNER}.Area")).unwrap();
        let expected = areas
            .keys()
            .map(|name| branch.member(name).unwrap().to_string())
            .collect();
        let branch_at = branch.to_string();
        let collection =
            check_native_branch(&authority, &read(&authority, OWNER), &branch_at, &expected);
        assert_eq!(collection["kind"], "Area", "{branch_at}");
        evidence.record(
            line,
            OWNER,
            "Area",
            None,
            &origin,
            AddressEvidence::Read(&branch_at),
        );

        for name in ["Подвал", "Строка", "Шапка"] {
            let area = areas[name];
            let value = |tag| mxl_child(area, tag).and_then(|node| node.text()).unwrap();
            assert_eq!(value("type"), "Rows", "{name}");
            let range =
                value("beginRow").parse::<usize>().unwrap()..=value("endRow").parse().unwrap();
            let at = branch.member(name).unwrap().to_string();
            let area_view = read(&authority, &at);
            assert_eq!(area_view["kind"], "Area", "{at}");
            let parameter_at = QualifiedAddress::parse(&format!("{at}.Parameter")).unwrap();
            let (body, parameters) = native_mxl_rows(root, range, &parameter_at);
            assert!(
                !body.is_empty() && !parameters.is_empty(),
                "{at}: populated row area"
            );
            for (kind, items) in [("Body", body), ("Parameter", parameters)] {
                let address = format!("{at}.{kind}");
                check_native_branch_count(&area_view, &address, items.len());
                let observed = read(&authority, &address);
                assert_eq!(observed["kind"], kind, "{address}");
                assert_eq!(observed["items"], serde_json::json!(items), "{address}");
                if kind == "Parameter" && name == "Строка" {
                    assert_eq!(
                        observed["items"].as_array().unwrap().last().unwrap(),
                        &serde_json::json!({"kind": "Parameter", "title": "Сумма [tpl]", "addressable": false})
                    );
                }
                evidence.record(
                    line,
                    &at,
                    kind,
                    None,
                    &origin,
                    AddressEvidence::Read(&address),
                );
            }
        }
    }
}

fn mxl_child<'a, 'input>(
    parent: roxmltree::Node<'a, 'input>,
    tag: &str,
) -> Option<roxmltree::Node<'a, 'input>> {
    parent
        .children()
        .find(|node| node.has_tag_name((MXL_NS, tag)))
}

fn native_mxl_rows(
    root: roxmltree::Node<'_, '_>,
    range: std::ops::RangeInclusive<usize>,
    parameter_at: &QualifiedAddress,
) -> (Vec<Value>, Vec<Value>) {
    const CORE: &str = "http://v8.1c.ru/8.1/data/core";
    assert!(!range.is_empty());
    let mut rows = BTreeMap::new();
    for item in root
        .children()
        .filter(|node| node.has_tag_name((MXL_NS, "rowsItem")))
    {
        let index = mxl_child(item, "index")
            .unwrap()
            .text()
            .unwrap()
            .parse::<usize>()
            .unwrap();
        assert!(
            rows.insert(index, mxl_child(item, "row").unwrap())
                .is_none(),
            "duplicate MXL row {index}"
        );
    }
    let mut body = Vec::new();
    let mut parameters = Vec::new();
    for (_, row) in rows.range(range) {
        for group in row
            .children()
            .filter(|node| node.has_tag_name((MXL_NS, "c")))
        {
            let cell = mxl_child(group, "c").unwrap();
            if let Some(name) = mxl_child(cell, "parameter").and_then(|node| node.text()) {
                parameters.push(serde_json::json!({
                    "at": parameter_at.member(name).unwrap().to_string(),
                    "kind": "Parameter",
                    "title": name,
                }));
            }
            let Some(text) = mxl_child(cell, "tl")
                .and_then(|label| {
                    label
                        .children()
                        .find(|node| node.has_tag_name((CORE, "item")))
                })
                .and_then(|item| {
                    item.children()
                        .find(|node| node.has_tag_name((CORE, "content")))
                })
                .and_then(|node| node.text())
                .filter(|text| !text.is_empty())
            else {
                continue;
            };
            let template = text.contains("[Сумма]");
            if template {
                parameters.push(serde_json::json!({
                    "kind": "Parameter",
                    "title": "Сумма [tpl]",
                    "addressable": false,
                }));
            }
            body.push(
                serde_json::json!({"index": body.len() + 1, "template": template, "text": text}),
            );
        }
    }
    (body, parameters)
}

#[allow(clippy::too_many_arguments)]
fn native_registered_form_descriptor(
    parent: roxmltree::Node<'_, '_>,
    parent_descriptor: &Path,
    name: &str,
    format: &str,
) -> PathBuf {
    let mut registrations = native_child_objects(parent).filter(|node| {
        node.has_tag_name((MD_CLASSES_NS, "Form")) && node.text().map(str::trim) == Some(name)
    });
    let registration = registrations.next().expect("native Form registration");
    assert!(
        registrations.next().is_none() && !registration.children().any(|node| node.is_element()),
        "{parent_descriptor:?}: expected one Form text registration for {name}"
    );
    let descriptor = parent_descriptor
        .with_extension("")
        .join("Forms")
        .join(format!("{name}.xml"));
    let raw = read_fixture_descriptor(&descriptor);
    let document = parse_fixture_descriptor(&raw, &descriptor);
    assert_eq!(document.root_element().attribute("version"), Some(format));
    let form = fixture_artifact(&document, "Form", &descriptor);
    assert_eq!(fixture_properties_name(form), Some(name));
    assert!(uuid::Uuid::parse_str(form.attribute("uuid").unwrap()).is_ok());
    descriptor
}

fn record_native_form_module_group(
    authority: &LogicalViewReadAuthority<'_>,
    source: (PlatformLine, SourceSetKind),
    form: &QualifiedAddress,
    descriptor: &Path,
    evidence: &mut EvidenceRows,
) -> QualifiedAddress {
    let owner = form.to_string();
    let branch = format!("{owner}.Module");
    let module = format!("{branch}.Form");
    let view = read(authority, &owner);
    assert_eq!(view["kind"], "Form", "{owner}");
    check_native_branch(authority, &view, &branch, &BTreeSet::from([module.clone()]));
    evidence.record_for_source(
        source,
        &owner,
        "Module",
        None,
        &fixture_origin(descriptor),
        AddressEvidence::Read(&branch),
    );
    QualifiedAddress::parse(&module).unwrap()
}

fn check_native_external_inline<'a, 'input: 'a>(
    authority: &LogicalViewReadAuthority<'_>,
    source: (PlatformLine, SourceSetKind),
    owner: &str,
    artifact: roxmltree::Node<'a, 'input>,
    descriptor: &Path,
    evidence: &mut EvidenceRows,
) {
    const READABLE_NS: &str = "http://v8.1c.ru/8.3/xcf/readable";

    let origin = fixture_origin(descriptor);
    let mut check = |owner: &str,
                     branch: &str,
                     members: &[(String, roxmltree::Node<'a, 'input>)]| {
        let at = format!("{owner}.{branch}");
        let address = QualifiedAddress::parse(&at).unwrap();
        let member_addresses: Vec<_> = members
            .iter()
            .map(|(name, _)| address.member(name).unwrap().to_string())
            .collect();
        let expected: BTreeSet<_> = member_addresses.iter().cloned().collect();
        assert_eq!(
            expected.len(),
            members.len(),
            "{at}: duplicate native registrations"
        );
        let collection = check_native_branch(authority, &read(authority, owner), &at, &expected);
        let order: Vec<_> = collection["items"]
            .as_array()
            .unwrap()
            .iter()
            .map(|item| item["at"].as_str().unwrap())
            .collect();
        assert_eq!(
            order,
            member_addresses
                .iter()
                .map(String::as_str)
                .collect::<Vec<_>>(),
            "{at}: inline registration order"
        );
        evidence.record_for_source(
            source,
            owner,
            branch,
            None,
            &origin,
            AddressEvidence::Read(&at),
        );
        for ((name, node), member) in members.iter().zip(&member_addresses) {
            let detail = read(authority, member);
            assert_eq!(detail["at"], member.as_str());
            assert_eq!(detail["kind"], branch);
            match branch {
                "Attribute" => {
                    let variant = super::external_inline_members::expected_variant(name)
                        .expect("native applied attribute type");
                    assert_eq!(
                        detail["props"]["type"]["variants"],
                        json!([variant]),
                        "{member}"
                    );
                }
                "StandardAttribute" => {
                    let fill_checking = node
                        .children()
                        .find(|child| child.has_tag_name((READABLE_NS, "FillChecking")))
                        .and_then(|child| child.text())
                        .expect("native standard attribute FillChecking");
                    assert_eq!(detail["props"]["FillChecking"], fill_checking, "{member}");
                }
                "TabularSection" => {}
                _ => unreachable!("not an external inline member family"),
            }
            evidence.record_for_source(
                source,
                owner,
                branch,
                None,
                &origin,
                AddressEvidence::Read(member),
            );
        }
    };

    let named_children = |node, branch| {
        native_child_objects(node)
            .filter(|child| child.has_tag_name((MD_CLASSES_NS, branch)))
            .map(|child| {
                let name = fixture_properties_name(child).expect("inline child Properties.Name");
                (name.to_owned(), child)
            })
            .collect::<Vec<_>>()
    };
    check(owner, "Attribute", &named_children(artifact, "Attribute"));
    let tables = named_children(artifact, "TabularSection");
    check(owner, "TabularSection", &tables);
    for (name, table) in tables {
        let section_at = format!("{owner}.TabularSection.{name}");
        check(
            &section_at,
            "Attribute",
            &named_children(table, "Attribute"),
        );
        let standard = table
            .children()
            .find(|child| child.has_tag_name((MD_CLASSES_NS, "Properties")))
            .and_then(|properties| {
                properties
                    .children()
                    .find(|child| child.has_tag_name((MD_CLASSES_NS, "StandardAttributes")))
            })
            .expect("native tabular section Properties.StandardAttributes");
        let standards: Vec<_> = standard
            .children()
            .filter(|child| child.has_tag_name((READABLE_NS, "StandardAttribute")))
            .map(|child| {
                (
                    child
                        .attribute("name")
                        .expect("standard attribute name")
                        .to_owned(),
                    child,
                )
            })
            .collect();
        check(&section_at, "StandardAttribute", &standards);
    }
}

fn check_native_external_form_and_object_modules(
    format: &str,
    source_kind: SourceSetKind,
    relative: &str,
    evidence: &mut EvidenceRows,
) {
    use unica_workspace::source_profile::platform_xml_source_profile;

    let (alias, kind) = match source_kind {
        SourceSetKind::ExternalProcessor => ("epf", "ExternalDataProcessor"),
        SourceSetKind::ExternalReport => ("erf", "ExternalReport"),
        _ => unreachable!("external module fixture uses an external source kind"),
    };
    let descriptor_path = fixture_path(relative);
    let raw = read_fixture_descriptor(&descriptor_path);
    let document = parse_fixture_descriptor(&raw, &descriptor_path);
    assert_eq!(document.root_element().attribute("version"), Some(format));
    let artifact = fixture_artifact(&document, kind, &descriptor_path);
    let owner_name = fixture_properties_name(artifact).expect("external owner name");
    assert_eq!(
        descriptor_path.file_stem().unwrap().to_str(),
        Some(owner_name)
    );
    let form_descriptor =
        native_registered_form_descriptor(artifact, &descriptor_path, "Main", format);
    let form_body = form_descriptor.with_extension("").join("Ext/Form.xml");
    let body = read_fixture_descriptor(&form_body);
    let body_xml = parse_fixture_descriptor(&body, &form_body);
    let body = body_xml.root_element();
    assert!(body.has_tag_name(("http://v8.1c.ru/8.3/xcf/logform", "Form")));
    assert_eq!(body.attribute("version"), Some(format));

    let source = descriptor_path.parent().unwrap().canonicalize().unwrap();
    let admitted = platform_xml_source_profile(source_kind, &source);
    let profile = PlatformProfile::for_line_or_base(admitted.semantic_line());
    let root = Arc::new(unica_workspace::read::open_source_directory(&source).unwrap());
    let cancellation = CancellationToken::new();
    let authority = LogicalViewReadAuthority::new(
        &cancellation,
        alias,
        "address-coverage-native-external-modules",
        source_kind,
        Arc::clone(&root),
        profile,
    );
    let owner = format!("{alias}:{kind}.{owner_name}");
    let branch = format!("{owner}.Form");
    let at = QualifiedAddress::parse(&format!("{branch}.Main")).unwrap();
    let owner_view = read(&authority, &owner);
    assert_eq!(owner_view["kind"], kind);
    assert!(addresses(&owner_view, "branches").contains(&branch));
    assert!(addresses(&read(&authority, &branch), "items").contains(&at.to_string()));

    let export_line = export_format_line(format).unwrap();
    check_native_external_inline(
        &authority,
        (export_line, source_kind),
        &owner,
        artifact,
        &descriptor_path,
        evidence,
    );
    let module = record_native_form_module_group(
        &authority,
        (export_line, source_kind),
        &at,
        &form_descriptor,
        evidence,
    );
    let object_module = QualifiedAddress::parse(&format!("{owner}.Module.Object")).unwrap();
    let layout = [LayoutFindSource::new(alias, source_kind, &root)];
    let finder = WorkspaceFindDirectoryBuilder::default();
    for (owner, module, role, module_path) in [
        (
            at.to_string(),
            module,
            "Form",
            format!("{owner_name}/Forms/Main/Ext/Form/Module.bsl"),
        ),
        (
            owner,
            object_module,
            "Object",
            format!("{owner_name}/Ext/ObjectModule.bsl"),
        ),
    ] {
        let module_at = module.to_string();
        let view = read(&authority, &module_at);
        assert_eq!(view["kind"], "Module");
        assert_eq!(view["props"]["role"], role);
        assert_eq!(view["props"]["ownerKind"], kind);
        let native_module = descriptor_path.parent().unwrap().join(&module_path);
        assert!(!read_fixture_descriptor(&native_module)
            .trim_start_matches('\u{feff}')
            .trim()
            .is_empty());
        assert_eq!(
            authority.existing_export_path(&module, None).unwrap(),
            module_path
        );
        let located = finder
            .locate_path(
                &layout,
                &module_path,
                ProviderDeadline::no_deadline(),
                &cancellation,
            )
            .unwrap()
            .unwrap_or_else(|| panic!("missing external module {module_path}"));
        assert_eq!(located.at(), module_at);
        assert_eq!(located.kind(), "Module");
        assert_eq!(located.placed_path(), Some(module_path.as_str()));
        evidence.record_for_source(
            (export_line, source_kind),
            &owner,
            "Module",
            Some(role),
            &fixture_origin(&native_module),
            AddressEvidence::RoundTrip(&module_at),
        );
        record_native_bsl_body(
            &authority,
            (export_line, source_kind),
            &module_at,
            &native_module,
            evidence,
        );
    }
}

fn check_native_configuration_form_and_command_modules(format: &str, evidence: &mut EvidenceRows) {
    let common = [
        (
            "Document",
            "Orders",
            "Documents",
            "Form",
            "AddressProbeDocumentForm",
        ),
        (
            "Catalog",
            "Валюты",
            "Catalogs",
            "Command",
            "AddressProbeCatalogCommand",
        ),
        (
            "DataProcessor",
            "Обработка",
            "DataProcessors",
            "Command",
            "AddressProbeProcessorCommand",
        ),
        (
            "Report",
            "АнализВерсийОбъектов",
            "Reports",
            "Form",
            "AddressProbeReportForm",
        ),
    ];
    let processor_form = (format != "2.20").then_some((
        "DataProcessor",
        "Обработка",
        "DataProcessors",
        "Form",
        "Форма",
    ));
    check_native_configuration_nested_modules(
        format,
        "nested_module_roles",
        common.into_iter().chain(processor_form),
        evidence,
    );
    check_native_configuration_nested_modules(
        format,
        "information_register_module_roles",
        [
            (
                "InformationRegister",
                "АдминистративнаяИерархия",
                "InformationRegisters",
                "Form",
                "AddressProbeRegisterForm",
            ),
            (
                "InformationRegister",
                "АдминистративнаяИерархия",
                "InformationRegisters",
                "Command",
                "AddressProbeRegisterCommand",
            ),
        ],
        evidence,
    );
    check_native_configuration_nested_modules(
        format,
        "settings_storage_module_roles",
        [(
            "SettingsStorage",
            "Probe",
            "SettingsStorages",
            "Form",
            "AddressProbeSettingsStorageForm",
        )],
        evidence,
    );
    check_native_configuration_nested_modules(
        format,
        "report_exchange_enum_module_roles",
        [
            (
                "Report",
                "АнализВерсийОбъектов",
                "Reports",
                "Command",
                "AddressProbeReportCommand",
            ),
            (
                "ExchangePlan",
                "Codes",
                "ExchangePlans",
                "Form",
                "AddressProbeExchangeForm",
            ),
            (
                "ExchangePlan",
                "Codes",
                "ExchangePlans",
                "Command",
                "AddressProbeExchangeCommand",
            ),
            (
                "Enum",
                "LanguageAware",
                "Enums",
                "Form",
                "AddressProbeEnumForm",
            ),
            (
                "Enum",
                "LanguageAware",
                "Enums",
                "Command",
                "AddressProbeEnumCommand",
            ),
        ],
        evidence,
    );
    check_native_configuration_nested_modules(
        format,
        "business_process_task_journal_module_roles",
        [
            (
                "BusinessProcess",
                "Approval",
                "BusinessProcesses",
                "Form",
                "AddressProbeBusinessProcessForm",
            ),
            (
                "BusinessProcess",
                "Approval",
                "BusinessProcesses",
                "Command",
                "AddressProbeBusinessProcessCommand",
            ),
            ("Task", "Work", "Tasks", "Form", "AddressProbeTaskForm"),
            (
                "Task",
                "Work",
                "Tasks",
                "Command",
                "AddressProbeTaskCommand",
            ),
            (
                "DocumentJournal",
                "Entries",
                "DocumentJournals",
                "Form",
                "AddressProbeJournalForm",
            ),
            (
                "DocumentJournal",
                "Entries",
                "DocumentJournals",
                "Command",
                "AddressProbeJournalCommand",
            ),
        ],
        evidence,
    );
    check_native_configuration_nested_modules(
        format,
        "register_family_module_roles",
        [
            (
                "AccumulationRegister",
                "Остатки",
                "AccumulationRegisters",
                "Form",
                "AddressProbeAccumulationRegisterForm",
            ),
            (
                "AccumulationRegister",
                "Остатки",
                "AccumulationRegisters",
                "Command",
                "AddressProbeAccumulationRegisterCommand",
            ),
            (
                "AccountingRegister",
                "Бухгалтерский",
                "AccountingRegisters",
                "Form",
                "AddressProbeAccountingRegisterForm",
            ),
            (
                "AccountingRegister",
                "Бухгалтерский",
                "AccountingRegisters",
                "Command",
                "AddressProbeAccountingRegisterCommand",
            ),
            (
                "CalculationRegister",
                "Начисления",
                "CalculationRegisters",
                "Form",
                "AddressProbeCalculationRegisterForm",
            ),
            (
                "CalculationRegister",
                "Начисления",
                "CalculationRegisters",
                "Command",
                "AddressProbeCalculationRegisterCommand",
            ),
        ],
        evidence,
    );
    check_native_configuration_nested_modules(
        format,
        "chart_family_module_roles",
        [
            (
                "ChartOfAccounts",
                "Хозрасчетный",
                "ChartsOfAccounts",
                "Form",
                "AddressProbeAccountsChartForm",
            ),
            (
                "ChartOfAccounts",
                "Хозрасчетный",
                "ChartsOfAccounts",
                "Command",
                "AddressProbeAccountsChartCommand",
            ),
            (
                "ChartOfCalculationTypes",
                "ВидыРасчета",
                "ChartsOfCalculationTypes",
                "Form",
                "AddressProbeCalculationTypesChartForm",
            ),
            (
                "ChartOfCalculationTypes",
                "ВидыРасчета",
                "ChartsOfCalculationTypes",
                "Command",
                "AddressProbeCalculationTypesChartCommand",
            ),
            (
                "ChartOfCharacteristicTypes",
                "Codes",
                "ChartsOfCharacteristicTypes",
                "Form",
                "AddressProbeCharacteristicTypesChartForm",
            ),
            (
                "ChartOfCharacteristicTypes",
                "Codes",
                "ChartsOfCharacteristicTypes",
                "Command",
                "AddressProbeCharacteristicTypesChartCommand",
            ),
        ],
        evidence,
    );
}

type NativeNestedModuleSample = (
    &'static str,
    &'static str,
    &'static str,
    &'static str,
    &'static str,
);

fn check_native_configuration_nested_modules(
    format: &str,
    fixture: &str,
    samples: impl IntoIterator<Item = NativeNestedModuleSample>,
    evidence: &mut EvidenceRows,
) {
    let source = fixture_path(&format!("formats/platform_xml/{format}/{fixture}/src"));
    let line = export_format_line(format).unwrap();
    let profile = PlatformProfile::for_line(line).unwrap();
    let cancellation = CancellationToken::new();
    let authority = native_authority(&cancellation, &source, profile);
    let root =
        unica_workspace::read::open_source_directory(&source.canonicalize().unwrap()).unwrap();
    let layout = [LayoutFindSource::new(
        "main",
        SourceSetKind::Configuration,
        &root,
    )];
    let finder = WorkspaceFindDirectoryBuilder::default();
    let registrations = root_registrations(&source, format);

    let mut checked_modules = BTreeSet::new();
    for (kind, name, directory, role, child) in samples {
        assert_eq!(
            registrations
                .iter()
                .filter(|(k, n)| k == kind && n == name)
                .count(),
            1,
            "{kind}.{name}: native configuration registration"
        );
        let descriptor = source.join(format!("{directory}/{name}.xml"));
        let xml = read_fixture_descriptor(&descriptor);
        let document = parse_fixture_descriptor(&xml, &descriptor);
        assert_eq!(document.root_element().attribute("version"), Some(format));
        let native_owner = fixture_artifact(&document, kind, &descriptor);
        assert_eq!(fixture_properties_name(native_owner), Some(name));
        assert!(uuid::Uuid::parse_str(native_owner.attribute("uuid").unwrap()).is_ok());
        let owner = format!("main:{kind}.{name}");

        let (child_at, origin, relative) = if role == "Form" {
            let form_descriptor =
                native_registered_form_descriptor(native_owner, &descriptor, child, format);
            let body = form_descriptor.with_extension("").join("Ext/Form.xml");
            let raw = read_fixture_descriptor(&body);
            let form = parse_fixture_descriptor(&raw, &body);
            assert!(form.root_element().has_tag_name((NATIVE_FORM_NS, "Form")));
            assert_eq!(form.root_element().attribute("version"), Some(format));
            (
                format!("{owner}.Form.{child}"),
                fixture_origin(&form_descriptor),
                format!("{directory}/{name}/Forms/{child}/Ext/Form/Module.bsl"),
            )
        } else {
            let commands = native_command_owners(&owner, native_owner);
            (
                commands[child].clone(),
                fixture_origin(&descriptor),
                format!("{directory}/{name}/Commands/{child}/Ext/CommandModule.bsl"),
            )
        };
        let group_at = format!("{child_at}.Module");
        let module_at = format!("{group_at}.{role}");
        let child_view = read(&authority, &child_at);
        assert_eq!(child_view["kind"], role, "{child_at}");
        let group = check_native_branch(
            &authority,
            &child_view,
            &group_at,
            &BTreeSet::from([module_at.clone()]),
        );
        assert_eq!(group["kind"], "Module", "{group_at}");
        let member = &group["items"][0];
        assert_eq!(member["kind"], "Module", "{module_at}");
        assert_eq!(member["props"]["ownerKind"], kind, "{module_at}");
        assert_eq!(member["props"]["role"], role, "{module_at}");
        evidence.record(
            line,
            &child_at,
            "Module",
            None,
            &origin,
            AddressEvidence::Read(&group_at),
        );

        assert_eq!(read(&authority, &module_at)["kind"], "Module");
        let module = QualifiedAddress::parse(&module_at).unwrap();
        let module_path = source.join(&relative);
        let raw_module = read_fixture_descriptor(&module_path);
        let module_text = raw_module.trim_start_matches('\u{feff}');
        assert!(!module_text.trim().is_empty());
        assert_eq!(
            authority.existing_export_path(&module, None).unwrap(),
            relative
        );
        let located = finder
            .locate_path(
                &layout,
                &relative,
                ProviderDeadline::no_deadline(),
                &cancellation,
            )
            .unwrap()
            .unwrap_or_else(|| panic!("missing native module {relative}"));
        assert_eq!(located.at(), module_at);
        assert_eq!(located.kind(), "Module");
        assert_eq!(located.placed_path(), Some(relative.as_str()));
        let module_origin = fixture_origin(&module_path);
        evidence.record(
            line,
            &child_at,
            "Module",
            Some(role),
            &module_origin,
            AddressEvidence::RoundTrip(&module_at),
        );
        let body = format!("{module_at}.Body");
        check_native_bsl_body(
            &authority,
            &body,
            module_text,
            1,
            module_text.lines().count(),
        );
        evidence.record(
            line,
            &module_at,
            "Body",
            None,
            &module_origin,
            AddressEvidence::Read(&body),
        );
        assert!(checked_modules.insert((child_at, module_path)));
    }
    let mut expected = native_nested_module_children(&source, format, &registrations);
    // Borrowed payloads travel with their owner fixture. The Report form
    // in the report/exchange/enum fixture is covered in nested_module_roles.
    let borrowed: &[&str] = match fixture {
        "nested_module_roles" => &[
            "Catalogs/Валюты/Forms/ФормаЭлемента/Ext/Form/Module.bsl",
            "Documents/Orders/Commands/Other/Ext/CommandModule.bsl",
            "Documents/Orders/Commands/Probe/Ext/CommandModule.bsl",
        ],
        "report_exchange_enum_module_roles" => {
            &["Reports/АнализВерсийОбъектов/Forms/AddressProbeReportForm/Ext/Form/Module.bsl"]
        }
        _ => &[],
    };
    for relative in borrowed {
        let file = source.join(relative);
        let count = expected.len();
        expected.retain(|(_, path)| path != &file);
        assert_eq!(expected.len() + 1, count, "{format}: borrowed {file:?}");
    }
    assert_eq!(
        checked_modules, expected,
        "{format}/{fixture}: physical Form and Command modules from native registrations"
    );
}

fn native_nested_module_children(
    source: &Path,
    format: &str,
    registrations: &[(String, String)],
) -> BTreeSet<(String, PathBuf)> {
    let mut modules = BTreeSet::new();
    for (kind, name) in registrations {
        let Some(layout) = METADATA_LAYOUTS
            .iter()
            .find(|layout| layout.tag == kind.as_str())
        else {
            continue;
        };
        let descriptor = source.join(layout.directory).join(format!("{name}.xml"));
        // These fixtures preserve Configuration.xml while selecting specific owners.
        if !native_inventory_has_file(&descriptor) {
            continue;
        }
        let raw = read_fixture_descriptor(&descriptor);
        let document = parse_fixture_descriptor(&raw, &descriptor);
        assert_eq!(document.root_element().attribute("version"), Some(format));
        let artifact = fixture_artifact(&document, kind, &descriptor);
        assert_eq!(fixture_properties_name(artifact), Some(name.as_str()));
        assert!(uuid::Uuid::parse_str(artifact.attribute("uuid").unwrap()).is_ok());
        let owner = format!("main:{kind}.{name}");
        let root = descriptor.with_extension("");
        for form in
            native_child_objects(artifact).filter(|node| node.has_tag_name((MD_CLASSES_NS, "Form")))
        {
            let name = form
                .text()
                .map(str::trim)
                .filter(|name| !name.is_empty())
                .expect("native Form name");
            let module = root.join("Forms").join(name).join("Ext/Form/Module.bsl");
            if native_inventory_has_file(&module) {
                native_registered_form_descriptor(artifact, &descriptor, name, format);
                assert!(modules.insert((format!("{owner}.Form.{name}"), module)));
            }
        }
        if native_child_objects(artifact).any(|node| node.has_tag_name((MD_CLASSES_NS, "Command")))
        {
            for (name, at) in native_command_owners(&owner, artifact) {
                let module = root
                    .join("Commands")
                    .join(name)
                    .join("Ext/CommandModule.bsl");
                if native_inventory_has_file(&module) {
                    assert!(modules.insert((at, module)));
                }
            }
        }
    }
    modules
}

fn native_inventory_has_file(path: &Path) -> bool {
    match fs::metadata(path) {
        Ok(metadata) => {
            assert!(metadata.is_file(), "{path:?}: expected native fixture file");
            true
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
        Err(error) => panic!("{path:?}: {error}"),
    }
}

fn check_native_html_processor_form_module(evidence: &mut EvidenceRows) {
    let format = "2.20";
    let source = fixture_path("formats/platform_xml/2.20/support-edit-bin-only/src");
    let line = export_format_line(format).unwrap();
    let registrations = root_registrations(&source, format);
    let kind = "DataProcessor";
    let name = "ТестHTML";
    assert_eq!(
        registrations
            .iter()
            .filter(|(tag, value)| tag == kind && value == name)
            .count(),
        1
    );
    let descriptor = source.join("DataProcessors/ТестHTML.xml");
    let raw = read_fixture_descriptor(&descriptor);
    let document = parse_fixture_descriptor(&raw, &descriptor);
    assert_eq!(document.root_element().attribute("version"), Some(format));
    let owner = fixture_artifact(&document, kind, &descriptor);
    assert_eq!(fixture_properties_name(owner), Some(name));
    assert!(uuid::Uuid::parse_str(owner.attribute("uuid").unwrap()).is_ok());
    let form_descriptor = native_registered_form_descriptor(owner, &descriptor, "Форма", format);

    let cancellation = CancellationToken::new();
    let source_kind = SourceSetKind::Configuration;
    let authority = native_authority(
        &cancellation,
        &source,
        PlatformProfile::for_line(line).unwrap(),
    );
    let form = QualifiedAddress::parse("main:DataProcessor.ТестHTML.Form.Форма").unwrap();
    let module = record_native_form_module_group(
        &authority,
        (line, source_kind),
        &form,
        &form_descriptor,
        evidence,
    );
    assert_eq!(read(&authority, &module.to_string())["kind"], "Module");
    let relative = "DataProcessors/ТестHTML/Forms/Форма/Ext/Form/Module.bsl";
    let file = source.join(relative);
    assert!(file.is_file(), "{file:?}");
    assert_eq!(
        authority.existing_export_path(&module, None).unwrap(),
        relative
    );
    let root =
        unica_workspace::read::open_source_directory(&source.canonicalize().unwrap()).unwrap();
    let layout = [LayoutFindSource::new("main", source_kind, &root)];
    let located = WorkspaceFindDirectoryBuilder::default()
        .locate_path(
            &layout,
            relative,
            ProviderDeadline::no_deadline(),
            &cancellation,
        )
        .unwrap()
        .unwrap();
    assert_eq!(located.at(), module.to_string());
    assert_eq!(located.placed_path(), Some(relative));
    evidence.record(
        line,
        &form.to_string(),
        "Module",
        Some("Form"),
        &fixture_origin(&file),
        AddressEvidence::RoundTrip(&module.to_string()),
    );
    record_native_bsl_body(
        &authority,
        (line, source_kind),
        &module.to_string(),
        &file,
        evidence,
    );
}

struct NativeFormOrTemplateSample<'a> {
    at: &'a QualifiedAddress,
    kind: &'a str,
    name: &'a str,
    format: &'a str,
    descriptor: &'a Path,
}

fn check_native_form_or_template(
    authority: &LogicalViewReadAuthority<'_>,
    sample: NativeFormOrTemplateSample<'_>,
    verify_path: &impl Fn(&QualifiedAddress, &Path),
    covered: &mut BTreeSet<String>,
    evidence: &mut EvidenceRows,
) {
    let NativeFormOrTemplateSample {
        at,
        kind,
        name,
        format,
        descriptor,
    } = sample;
    let raw = read_fixture_descriptor(descriptor);
    let document = parse_fixture_descriptor(&raw, descriptor);
    let artifact = fixture_artifact(&document, kind, descriptor);
    assert_eq!(
        document.root_element().attribute("version"),
        Some(format),
        "native fixture {descriptor:?}"
    );
    assert!(
        artifact
            .attribute("uuid")
            .is_some_and(|value| uuid::Uuid::parse_str(value).is_ok()),
        "native fixture {descriptor:?}: missing or invalid UUID"
    );
    let properties = artifact
        .children()
        .find(|node| node.has_tag_name((MD_CLASSES_NS, "Properties")))
        .unwrap_or_else(|| panic!("native fixture {descriptor:?}: missing Properties"));
    let property = |key| {
        properties
            .children()
            .find(|node| node.has_tag_name((MD_CLASSES_NS, key)))
            .and_then(|node| node.text())
    };
    assert_eq!(
        property("Name"),
        Some(name),
        "native fixture {descriptor:?}: registration name"
    );
    let ext = descriptor.with_extension("").join("Ext");
    let form = matches!(kind, "Form" | "CommonForm");
    let at_text = at.to_string();
    let line = authority.platform_profile().line();
    let status = if form {
        assert_eq!(
            property("FormType"),
            Some("Managed"),
            "native fixture {descriptor:?}"
        );
        let payload = ext.join("Form.xml");
        let xml = read_fixture_descriptor(&payload);
        let body = parse_fixture_descriptor(&xml, &payload);
        assert!(
            body.root_element()
                .has_tag_name(("http://v8.1c.ru/8.3/xcf/logform", "Form")),
            "native fixture {payload:?}"
        );
        assert_eq!(
            body.root_element().attribute("version"),
            Some(format),
            "native fixture {payload:?}"
        );
        let owner_view = read(authority, &at_text);
        assert_eq!(owner_view["kind"], "Form");
        let root = body.root_element();
        let origin = fixture_origin(&payload);
        check_native_form_items(authority, &at_text, root, &origin, covered, evidence);
        if let Some(attributes) = native_form_children(root, "Attributes").next() {
            let names: Vec<_> = native_form_children(attributes, "Attribute")
                .map(|node| node.attribute("name").expect("native form attribute name"))
                .collect();
            if names.is_empty() {
                let branch = format!("{at}.Attribute");
                assert!(!addresses(&owner_view, "branches").contains(&branch));
                let view = read(authority, &branch);
                assert!(addresses(&view, "items").is_empty(), "{branch}");
                evidence.record(
                    authority.platform_profile().line(),
                    &at_text,
                    "Attribute",
                    None,
                    &origin,
                    AddressEvidence::Read(&branch),
                );
            } else {
                check_native_form_collection(
                    authority,
                    &at_text,
                    "Attribute",
                    &names,
                    &origin,
                    evidence,
                );
            }
        }
        verify_path(at, &payload);
        let metadata = format!("{at}.Metadata");
        assert!(
            addresses(&owner_view, "branches").contains(&metadata),
            "{at}: missing Metadata"
        );
        assert_eq!(read(authority, &metadata)["kind"], "Metadata");
        verify_path(&QualifiedAddress::parse(&metadata).unwrap(), descriptor);
        evidence.record(
            line,
            &at_text,
            "Metadata",
            None,
            &fixture_origin(descriptor),
            AddressEvidence::RoundTrip(&metadata),
        );
        covered.insert(metadata);
        let module = ext.join("Form/Module.bsl");
        if module.is_file() {
            let module_at = format!("{at}.Module.Form");
            assert!(
                addresses(&read(authority, &format!("{at}.Module")), "items").contains(&module_at)
            );
            assert_eq!(read(authority, &module_at)["kind"], "Module");
            verify_path(&QualifiedAddress::parse(&module_at).unwrap(), &module);
            evidence.record(
                line,
                &at_text,
                "Module",
                Some("Form"),
                &fixture_origin(&module),
                AddressEvidence::RoundTrip(&module_at),
            );
            record_native_bsl_body(
                authority,
                (line, SourceSetKind::Configuration),
                &module_at,
                &module,
                evidence,
            );
            if at_text == "main:Catalog.Валюты.Form.ФормаЭлемента" {
                record_native_form_module_group(
                    authority,
                    (line, SourceSetKind::Configuration),
                    at,
                    descriptor,
                    evidence,
                );
            }
            covered.insert(module_at);
        }
        "Managed Form"
    } else {
        const MXL_NS: &str = "http://v8.1c.ru/8.2/data/spreadsheet";
        let template = property("TemplateType")
            .unwrap_or_else(|| panic!("native fixture {descriptor:?}: missing TemplateType"));
        let mut data_sets = BTreeSet::new();
        match template {
            "BinaryData" => {
                let payload = ext.join("Template.bin");
                assert!(!fs::read(&payload)
                    .unwrap_or_else(|error| panic!("native fixture {payload:?}: {error}"))
                    .is_empty());
            }
            "DataCompositionSchema" | "SpreadsheetDocument" => {
                let payload = ext.join("Template.xml");
                let xml = read_fixture_descriptor(&payload);
                let body = parse_fixture_descriptor(&xml, &payload);
                let root = body.root_element();
                if template == "DataCompositionSchema" {
                    assert!(
                        root.has_tag_name((DCS_NS, "DataCompositionSchema")),
                        "native fixture {payload:?}"
                    );
                    let branch = QualifiedAddress::parse(&format!("{at}.DataSet")).unwrap();
                    for data_set in root
                        .children()
                        .filter(|node| node.has_tag_name((DCS_NS, "dataSet")))
                    {
                        let name = data_set
                            .children()
                            .find(|node| node.has_tag_name((DCS_NS, "name")))
                            .and_then(|node| node.text())
                            .unwrap_or_else(|| {
                                panic!("native fixture {payload:?}: missing dataSet name")
                            });
                        assert!(
                            data_sets.insert(branch.member(name).unwrap().to_string()),
                            "native fixture {payload:?}: duplicate dataSet {name}"
                        );
                    }
                    assert!(
                        !data_sets.is_empty(),
                        "native fixture {payload:?}: missing DataSet witness"
                    );
                    check_native_dcs_groups(authority, &at_text, root, &payload, evidence);
                } else {
                    assert!(
                        root.has_tag_name((MXL_NS, "document")),
                        "native fixture {payload:?}"
                    );
                }
            }
            other => panic!("native fixture {descriptor:?}: unexpected TemplateType {other}"),
        }
        let owner_view = read(authority, &at_text);
        assert_eq!(owner_view["kind"], kind);
        verify_path(at, descriptor);
        if template == "DataCompositionSchema" {
            let branch = format!("{at}.DataSet");
            check_native_branch(authority, &owner_view, &branch, &data_sets);
            let origin = fixture_origin(&ext.join("Template.xml"));
            evidence.record(
                line,
                &at_text,
                "DataSet",
                None,
                &origin,
                AddressEvidence::Read(&branch),
            );
            for item in &data_sets {
                assert_eq!(read(authority, item)["kind"], "DataSet", "{item}");
                evidence.record(
                    line,
                    &at_text,
                    "DataSet",
                    None,
                    &origin,
                    AddressEvidence::Read(item),
                );
            }
            covered.insert(branch);
        }
        template
    };
    println!(
        "| {} | {kind} | {at} | {status}; native registration and descriptor |",
        authority.platform_profile().id()
    );
}

const NATIVE_FORM_NS: &str = "http://v8.1c.ru/8.3/xcf/logform";
const DCS_NS: &str = "http://v8.1c.ru/8.1/data-composition-system/schema";
const DCS_SETTINGS_NS: &str = "http://v8.1c.ru/8.1/data-composition-system/settings";

fn native_dcs_children<'a, 'input: 'a>(
    owner: roxmltree::Node<'a, 'input>,
    tag: &'a str,
) -> impl Iterator<Item = roxmltree::Node<'a, 'input>> + 'a {
    owner
        .children()
        .filter(move |node| node.has_tag_name((DCS_NS, tag)))
}

fn native_dcs_text<'a, 'input: 'a>(node: roxmltree::Node<'a, 'input>, tag: &str) -> &'a str {
    node.children()
        .find(|child| child.has_tag_name((DCS_NS, tag)))
        .and_then(|child| child.text())
        .unwrap_or_else(|| panic!("native DCS {} has no {tag}", node.tag_name().name()))
}

fn check_native_dcs_groups(
    authority: &LogicalViewReadAuthority<'_>,
    owner: &str,
    root: roxmltree::Node<'_, '_>,
    path: &Path,
    evidence: &mut EvidenceRows,
) {
    let origin = fixture_origin(path);
    let source = (
        authority.platform_profile().line(),
        SourceSetKind::Configuration,
    );
    let data_sources = native_dcs_children(root, "dataSource").map(|node| {
        let kind = native_dcs_text(node, "dataSourceType");
        assert_eq!(kind, "Local", "{path:?}");
        (native_dcs_text(node, "name"), vec![("kind", kind)])
    });
    check_native_dcs_branch(
        authority,
        owner,
        "DataSource",
        data_sources,
        &origin,
        source,
        evidence,
    );

    let settings = native_dcs_children(root, "settingsVariant").map(|node| {
        let name = node
            .children()
            .find(|child| child.has_tag_name((DCS_SETTINGS_NS, "name")))
            .and_then(|child| child.text())
            .expect("native DCS settings name");
        (name, Vec::new())
    });
    check_native_dcs_branch(
        authority, owner, "Setting", settings, &origin, source, evidence,
    );

    let totals = native_dcs_children(root, "totalField").map(|node| {
        let name = native_dcs_text(node, "dataPath");
        let expression = native_dcs_text(node, "expression");
        (name, vec![("expression", expression)])
    });
    check_native_dcs_branch(authority, owner, "Total", totals, &origin, source, evidence);

    let datasets = QualifiedAddress::parse(&format!("{owner}.DataSet")).unwrap();
    for dataset in native_dcs_children(root, "dataSet") {
        let at = datasets.member(native_dcs_text(dataset, "name")).unwrap();
        if owner.starts_with("main:DataProcessor.") || owner.starts_with("main:Report.") {
            let queries: Vec<_> = native_dcs_children(dataset, "query").collect();
            if let [query] = queries.as_slice() {
                let statement = query.text().expect("native nonempty query");
                assert!(!statement.trim().is_empty(), "{path:?}: empty query");
                let query_at = format!("{at}.Query");
                let view = read(authority, &at.to_string());
                assert!(addresses(&view, "branches").contains(&query_at), "{at}");
                let observed = read(authority, &query_at);
                let expected: Vec<_> = statement
                    .lines()
                    .enumerate()
                    .map(|(index, text)| serde_json::json!({"line": index + 1, "text": text}))
                    .collect();
                assert_eq!(observed["kind"], "Query");
                assert_eq!(observed["at"], query_at);
                assert_eq!(observed["items"], serde_json::json!(expected), "{query_at}");
                evidence.record_for_source(
                    source,
                    &at.to_string(),
                    "Query",
                    None,
                    &origin,
                    AddressEvidence::Read(&query_at),
                );
            } else {
                assert!(queries.is_empty(), "{path:?}: duplicate query elements");
            }
        }
        let fields = native_dcs_children(dataset, "field").map(|node| {
            let path = native_dcs_text(node, "dataPath");
            (path, vec![("field", native_dcs_text(node, "field"))])
        });
        check_native_dcs_branch(
            authority,
            &at.to_string(),
            "Field",
            fields,
            &origin,
            source,
            evidence,
        );
    }
}

fn check_native_dcs_branch<'a>(
    authority: &LogicalViewReadAuthority<'_>,
    owner: &str,
    kind: &str,
    rows: impl Iterator<Item = (&'a str, Vec<(&'a str, &'a str)>)>,
    origin: &str,
    source: (PlatformLine, SourceSetKind),
    evidence: &mut EvidenceRows,
) {
    let branch = format!("{owner}.{kind}");
    let address = QualifiedAddress::parse(&branch).unwrap();
    let mut expected = BTreeMap::new();
    for (name, props) in rows {
        let at = address.member(name).unwrap().to_string();
        assert!(
            expected.insert(at, props).is_none(),
            "duplicate native DCS member {name}"
        );
    }
    let owner_view = read(authority, owner);
    if expected.is_empty() {
        // Typed DCS advertises branches only for nonempty collections.
        assert!(
            !addresses(&owner_view, "branches").contains(&branch),
            "{branch}"
        );
        assert!(
            addresses(&read(authority, &branch), "items").is_empty(),
            "{branch}"
        );
        evidence.record_for_source(
            source,
            owner,
            kind,
            None,
            origin,
            AddressEvidence::Read(&branch),
        );
        return;
    }
    check_native_branch(
        authority,
        &owner_view,
        &branch,
        &expected.keys().cloned().collect(),
    );
    evidence.record_for_source(
        source,
        owner,
        kind,
        None,
        origin,
        AddressEvidence::Read(&branch),
    );
    for (at, props) in expected {
        let view = read(authority, &at);
        assert_eq!(view["kind"], kind, "{at}");
        for (key, value) in props {
            assert_eq!(view["props"][key], value, "{at}: {key}");
        }
        evidence.record_for_source(
            source,
            owner,
            kind,
            None,
            origin,
            AddressEvidence::Read(&at),
        );
    }
}

fn check_native_external_dcs_parameters(
    authority: &LogicalViewReadAuthority<'_>,
    report: &str,
    artifact: roxmltree::Node<'_, '_>,
    descriptor: &Path,
    line: PlatformLine,
    evidence: &mut EvidenceRows,
) {
    assert_eq!(
        native_child_objects(artifact)
            .filter(|node| {
                node.has_tag_name((MD_CLASSES_NS, "Template")) && node.text() == Some("MainSchema")
            })
            .count(),
        1,
        "{report}: native MainSchema registration"
    );
    let template_descriptor = descriptor
        .with_extension("")
        .join("Templates/MainSchema.xml");
    let raw = read_fixture_descriptor(&template_descriptor);
    let template_doc = parse_fixture_descriptor(&raw, &template_descriptor);
    let template_artifact = fixture_artifact(&template_doc, "Template", &template_descriptor);
    assert_eq!(
        fixture_properties_name(template_artifact),
        Some("MainSchema")
    );
    let path = template_descriptor
        .with_extension("")
        .join("Ext/Template.xml");
    let xml = read_fixture_descriptor(&path);
    let schema = parse_fixture_descriptor(&xml, &path);
    let root = schema.root_element();
    assert!(root.has_tag_name((DCS_NS, "DataCompositionSchema")));
    let datasets: Vec<_> = native_dcs_children(root, "dataSet").collect();
    let [dataset] = datasets.as_slice() else {
        panic!("expected native ERF dataset Main");
    };
    assert_eq!(native_dcs_text(*dataset, "name"), "Main");
    assert!(native_dcs_children(*dataset, "parameter").next().is_none());

    let mut parameters = BTreeMap::new();
    for param in native_dcs_children(root, "parameter") {
        let name = native_dcs_text(param, "name");
        let declared_type = native_dcs_children(param, "valueType").next().unwrap();
        let declared_type = declared_type
            .children()
            .find(|node| node.has_tag_name(("http://v8.1c.ru/8.1/data/core", "Type")))
            .and_then(|node| node.text())
            .unwrap();
        let value = native_dcs_children(param, "value").next().unwrap();
        assert_eq!(
            value.attribute(("http://www.w3.org/2001/XMLSchema-instance", "type")),
            Some(declared_type)
        );
        let default = value.text().unwrap();
        assert!(parameters.insert(name, (declared_type, default)).is_none());
    }
    assert_eq!(
        parameters,
        BTreeMap::from([
            ("Amount", ("xs:decimal", "10")),
            ("Label", ("xs:string", "Sample"))
        ])
    );
    let origin = fixture_origin(&path);
    let source = (line, SourceSetKind::ExternalReport);
    let template = format!("{report}.Template.MainSchema");
    for owner in [&template, &format!("{template}.DataSet.Main")] {
        let rows = parameters.iter().map(|(&name, &(xml_type, default))| {
            let props = match xml_type {
                "xs:string" => vec![
                    ("type", "String"),
                    ("valueType", xml_type),
                    ("value", default),
                ],
                "xs:decimal" => vec![("type", "Number"), ("value", default)],
                _ => unreachable!("fixture parameter type checked above"),
            };
            (name, props)
        });
        check_native_dcs_branch(
            authority,
            owner,
            "Parameter",
            rows,
            &origin,
            source,
            evidence,
        );
    }
}

fn native_form_children<'a, 'input: 'a>(
    owner: roxmltree::Node<'a, 'input>,
    tag: &'a str,
) -> impl Iterator<Item = roxmltree::Node<'a, 'input>> + 'a {
    owner
        .children()
        .filter(move |node| node.has_tag_name((NATIVE_FORM_NS, tag)))
}

fn check_native_form_items(
    authority: &LogicalViewReadAuthority<'_>,
    owner: &str,
    raw: roxmltree::Node<'_, '_>,
    origin: &str,
    covered: &mut BTreeSet<String>,
    evidence: &mut EvidenceRows,
) {
    let bars = native_form_children(raw, "AutoCommandBar").filter(|bar| {
        // Unnamed platform chrome has no logical item identity.
        bar.attribute("name").is_some_and(|name| !name.is_empty())
    });
    let children: Vec<_> = bars
        .chain(
            native_form_children(raw, "ChildItems")
                .flat_map(|container| container.children().filter(|node| node.is_element())),
        )
        .map(|node| {
            assert_eq!(node.tag_name().namespace(), Some(NATIVE_FORM_NS));
            let name = node.attribute("name").expect("native form item name");
            assert!(!name.is_empty());
            (name, node)
        })
        .collect();
    if children.is_empty() {
        return;
    }
    let names: Vec<_> = children.iter().map(|(name, _)| *name).collect();
    check_native_form_collection(authority, owner, "Item", &names, origin, evidence);
    let branch = format!("{owner}.Item");
    assert!(
        covered.insert(branch.clone()),
        "duplicate form branch {branch}"
    );
    let branch = QualifiedAddress::parse(&branch).unwrap();
    for (name, node) in children {
        let item = branch.member(name).unwrap().to_string();
        check_native_form_items(authority, &item, node, origin, covered, evidence);
    }
}

fn check_native_form_collection(
    authority: &LogicalViewReadAuthority<'_>,
    owner: &str,
    kind: &str,
    names: &[&str],
    origin: &str,
    evidence: &mut EvidenceRows,
) {
    let branch_at = format!("{owner}.{kind}");
    let branch = QualifiedAddress::parse(&branch_at).unwrap();
    let expected: BTreeSet<_> = names
        .iter()
        .map(|name| branch.member(name).unwrap().to_string())
        .collect();
    assert_eq!(
        expected.len(),
        names.len(),
        "{branch_at}: duplicate native name"
    );
    check_native_branch(authority, &read(authority, owner), &branch_at, &expected);
    let line = authority.platform_profile().line();
    evidence.record(
        line,
        owner,
        kind,
        None,
        origin,
        AddressEvidence::Read(&branch_at),
    );
    for item in expected {
        assert_eq!(read(authority, &item)["kind"], kind, "{item}");
        evidence.record(
            line,
            owner,
            kind,
            None,
            origin,
            AddressEvidence::Read(&item),
        );
    }
}

fn check_native_command_and_bound_events(evidence: &mut EvidenceRows) {
    const OWNER: &str = "main:DataProcessor.ТестHTML.Form.Форма";
    let source = fixture_path("formats/platform_xml/2.20/support-edit-bin-only/src");
    assert!(
        root_registrations(&source, "2.20").contains(&("DataProcessor".into(), "ТестHTML".into()))
    );
    let owner_path = source.join("DataProcessors/ТестHTML.xml");
    let xml = read_fixture_descriptor(&owner_path);
    let config_owner = parse_fixture_descriptor(&xml, &owner_path);
    let owner = fixture_artifact(&config_owner, "DataProcessor", &owner_path);
    assert_eq!(fixture_properties_name(owner), Some("ТестHTML"));
    assert_eq!(
        native_child_objects(owner)
            .filter(|node| {
                node.has_tag_name((MD_CLASSES_NS, "Form")) && node.text() == Some("Форма")
            })
            .count(),
        1
    );
    let form_path = source.join("DataProcessors/ТестHTML/Forms/Форма.xml");
    let xml = read_fixture_descriptor(&form_path);
    let form = parse_fixture_descriptor(&xml, &form_path);
    let descriptor = fixture_artifact(&form, "Form", &form_path);
    assert_eq!(fixture_properties_name(descriptor), Some("Форма"));
    assert_eq!(form.root_element().attribute("version"), Some("2.20"));
    let path = form_path.with_extension("").join("Ext/Form.xml");
    let xml = read_fixture_descriptor(&path);
    let form = parse_fixture_descriptor(&xml, &path);
    let root = form.root_element();
    assert!(root.has_tag_name((NATIVE_FORM_NS, "Form")));
    assert_eq!(root.attribute("version"), Some("2.20"));
    let origin = fixture_origin(&path);
    let cancellation = CancellationToken::new();
    let profile = PlatformProfile::v8_3_27();
    let authority = native_authority(&cancellation, &source, profile);
    let commands = native_form_children(root, "Commands").next().unwrap();
    let names: Vec<_> = native_form_children(commands, "Command")
        .map(|node| node.attribute("name").expect("native form command name"))
        .collect();
    assert_eq!(names, ["Команда1"]);
    check_native_form_collection(&authority, OWNER, "Command", &names, &origin, evidence);

    let items = native_form_children(root, "ChildItems").next().unwrap();
    let html = native_form_children(items, "HTMLDocumentField")
        .find(|node| node.attribute("name") == Some("ПолеHTML"))
        .unwrap();
    let item = format!("{OWNER}.Item.ПолеHTML");
    for (owner, node, event) in [(OWNER, root, "OnCreateAtServer"), (&item, html, "OnClick")] {
        let events = native_form_children(node, "Events").next().unwrap();
        let handler = native_form_children(events, "Event")
            .find(|node| node.attribute("name") == Some(event))
            .and_then(|node| node.text())
            .filter(|handler| !handler.is_empty())
            .expect("native bound form event");
        let event_at = format!("{owner}.Event.{event}");
        let view = read(&authority, &event_at);
        assert_eq!(view["kind"], "Event");
        assert_eq!(view["props"]["handler"], handler, "{event_at}");
        evidence.record(
            profile.line(),
            owner,
            "Event",
            None,
            &origin,
            AddressEvidence::Read(&event_at),
        );
    }
}

const EDS_MODULE_OWNERS: [(&str, &str, &[&str]); 3] = [
    (
        "main:ExternalDataSource.AllKindsExternalSource.Table.Items",
        "ExternalDataSources/AllKindsExternalSource/Tables/Items",
        &["Manager", "Object", "RecordSet"],
    ),
    (
        "main:ExternalDataSource.AllKindsExternalSource.Cube.Sales",
        "ExternalDataSources/AllKindsExternalSource/Cubes/Sales",
        &["Manager", "RecordSet"],
    ),
    (
        "main:ExternalDataSource.AllKindsExternalSource.Cube.Sales.DimensionTable.Products",
        "ExternalDataSources/AllKindsExternalSource/Cubes/Sales/DimensionTables/Products",
        &["Manager", "Object"],
    ),
];

fn check_eds_module_rows(
    authority: &LogicalViewReadAuthority<'_>,
    source: &Path,
    cancellation: &CancellationToken,
    evidence: &mut EvidenceRows,
) {
    let line = authority.platform_profile().line();
    let root =
        unica_workspace::read::open_source_directory(&source.canonicalize().unwrap()).unwrap();
    let find_sources = [LayoutFindSource::new(
        "main",
        SourceSetKind::Configuration,
        &root,
    )];
    let finder = WorkspaceFindDirectoryBuilder::default();
    for (owner, directory, roles) in EDS_MODULE_OWNERS {
        let branch = format!("{owner}.Module");
        let expected = roles
            .iter()
            .map(|role| format!("{branch}.{role}"))
            .collect();
        check_native_branch(authority, &read(authority, owner), &branch, &expected);
        evidence.record(
            line,
            owner,
            "Module",
            None,
            &fixture_origin(&source.join(format!("{directory}.xml"))),
            AddressEvidence::Read(&branch),
        );

        for &role in roles {
            let module = format!("{branch}.{role}");
            let at = QualifiedAddress::parse(&module).unwrap();
            let path = authority.existing_export_path(&at, None).unwrap();
            let native_path = format!("{directory}/Ext/{role}Module.bsl");
            assert_eq!(path, native_path, "{line}: {module}");
            let document = finder
                .locate_path(
                    &find_sources,
                    &native_path,
                    ProviderDeadline::no_deadline(),
                    cancellation,
                )
                .unwrap()
                .unwrap();
            assert_eq!(document.at(), module.as_str(), "{line}: {native_path}");
            let file = source.join(&native_path);
            evidence.record(
                line,
                owner,
                "Module",
                Some(role),
                &fixture_origin(&file),
                AddressEvidence::RoundTrip(&module),
            );
            record_native_bsl_body(
                authority,
                (line, SourceSetKind::Configuration),
                &module,
                &file,
                evidence,
            );
        }
    }
}

fn check_metadata_owner(
    authority: &LogicalViewReadAuthority<'_>,
    source: &Path,
    owner: &QualifiedAddress,
    verified: &mut BTreeSet<String>,
    evidence: &mut EvidenceRows,
) {
    let profile = authority.platform_profile();
    let route = route_logical_address(owner, profile).unwrap();
    let kind = owner.segments().last().unwrap().kind().as_str();
    let descriptor = source.join(authority.existing_export_path(owner, None).unwrap());
    let xml = read_fixture_descriptor(&descriptor);
    let document = parse_fixture_descriptor(&xml, &descriptor);
    let artifact = fixture_artifact(&document, kind, &descriptor);
    let owner_at = owner.to_string();
    let native_branches = native_described_branches(&owner_at, &descriptor, artifact);
    let mut inline = BTreeMap::new();
    collect_native_inline_branches(owner, artifact, &mut BTreeSet::new(), &mut inline);
    let admitted = authority.snapshot(owner).unwrap();
    let payload = authority
        .typed_payload_for(&route, &admitted)
        .unwrap_or_else(|error| panic!("{owner}: {error:?}"));
    let published = read(authority, &owner_at);
    let (facts, addressing_issue) = owner_facts(owner, &payload);
    let origin = fixture_origin(&descriptor);
    for (branch, expected) in native_branches {
        check_native_branch(authority, &published, &branch, &expected);
        let branch_address = QualifiedAddress::parse(&branch).unwrap();
        let child_kind = branch_address.segments().last().unwrap().kind().as_str();
        evidence.record(
            profile.line(),
            &owner_at,
            child_kind,
            None,
            &origin,
            AddressEvidence::Read(&branch),
        );
        for item in &expected {
            assert_eq!(read(authority, item)["kind"], child_kind, "{item}");
            evidence.record(
                profile.line(),
                &owner_at,
                child_kind,
                None,
                &origin,
                AddressEvidence::Read(item),
            );
        }
        println!(
            "| {} | {owner} | {child_kind} | {branch} | {} native registration(s) with matching descriptors |",
            profile.id(),
            expected.len()
        );
        verified.insert(branch);
    }
    for (inline_owner, branches) in inline {
        let nested_view = (inline_owner != owner_at).then(|| read(authority, &inline_owner));
        let owner_view = nested_view.as_ref().unwrap_or(&published);
        for (kind, expected) in branches {
            let branch = format!("{inline_owner}.{kind}");
            check_native_branch(authority, owner_view, &branch, &expected);
            evidence.record(
                profile.line(),
                &inline_owner,
                kind,
                None,
                &origin,
                AddressEvidence::Read(&branch),
            );
            for item in &expected {
                assert_eq!(read(authority, item)["kind"], kind, "{item}");
                evidence.record(
                    profile.line(),
                    &inline_owner,
                    kind,
                    None,
                    &origin,
                    AddressEvidence::Read(item),
                );
            }
            println!(
                "| {} | {inline_owner} | {kind} | {branch} | {} native inline registration(s) |",
                profile.id(),
                expected.len()
            );
            verified.insert(branch);
        }
    }
    check_native_child_registrations(owner, artifact, &facts);
    if owner.segments().len() == 1 {
        check_native_empty_metadata_branches(
            authority, owner, artifact, &published, &origin, verified, evidence,
        );
    }
    if let Some(reason) = addressing_issue {
        let limits = published["limits"].as_array().unwrap();
        assert!(limits
            .iter()
            .any(|limit| limit.as_str() == Some(reason.as_str())));
        println!(
            "| {} | {owner} | AddressingAttribute | — | {reason} |",
            profile.id()
        );
    }
    check_metadata_child_branches(
        authority, source, owner, &published, &facts, verified, evidence,
    );
}

const NATIVE_XDTO_NS: &str = "http://v8.1c.ru/8.1/xdto";

fn check_native_xdto_package(
    authority: &LogicalViewReadAuthority<'_>,
    source: &Path,
    owner: &QualifiedAddress,
    format: &str,
    evidence: &mut EvidenceRows,
) {
    let name = owner.segments().last().unwrap().name().unwrap();
    let descriptor = source.join(format!("XDTOPackages/{name}.xml"));
    let xml = read_fixture_descriptor(&descriptor);
    let document = parse_fixture_descriptor(&xml, &descriptor);
    assert_eq!(document.root_element().attribute("version"), Some(format));
    let artifact = fixture_artifact(&document, "XDTOPackage", &descriptor);
    assert_eq!(fixture_properties_name(artifact), Some(name));
    let descriptor_namespace = artifact
        .children()
        .find(|node| node.has_tag_name((MD_CLASSES_NS, "Properties")))
        .and_then(|node| {
            node.children()
                .find(|child| child.has_tag_name((MD_CLASSES_NS, "Namespace")))
        })
        .and_then(|node| node.text())
        .expect("native XDTO descriptor Namespace");

    let payload = descriptor.with_extension("").join("Ext/Package.bin");
    let xml = read_fixture_descriptor(&payload);
    let document = parse_fixture_descriptor(&xml, &payload);
    let root = document.root_element();
    assert!(root.has_tag_name((NATIVE_XDTO_NS, "package")));
    let namespace = root.attribute("targetNamespace").unwrap();
    assert_eq!(namespace, descriptor_namespace);
    let line = authority.platform_profile().line();
    let origin = fixture_origin(&payload);
    let owner = owner.to_string();
    let view = read(authority, &owner);
    assert_eq!(view["kind"], "XDTOPackage");
    assert_eq!(view["props"]["targetNamespace"], namespace);
    check_native_xdto_properties(authority, &owner, &view, root, &origin, evidence);
    let namespace_at = format!("{owner}.Namespace");
    check_native_branch_count(&view, &namespace_at, 1);
    let namespace_view = read(authority, &namespace_at);
    assert_eq!(namespace_view["kind"], "Namespace");
    assert_eq!(namespace_view["props"]["namespace"], namespace);
    evidence.record(
        line,
        &owner,
        "Namespace",
        None,
        &origin,
        AddressEvidence::Read(&namespace_at),
    );

    let branch_at = format!("{owner}.Type");
    let branch = QualifiedAddress::parse(&branch_at).unwrap();
    let mut types = BTreeMap::new();
    for node in root.children().filter(|node| {
        node.has_tag_name((NATIVE_XDTO_NS, "valueType"))
            || node.has_tag_name((NATIVE_XDTO_NS, "objectType"))
    }) {
        let at = branch
            .member(node.attribute("name").expect("native named XDTO type"))
            .unwrap();
        assert!(
            types.insert(at.to_string(), node).is_none(),
            "duplicate native XDTO type {at}"
        );
    }
    for kind in ["valueType", "objectType"] {
        assert!(types
            .values()
            .any(|node| node.has_tag_name((NATIVE_XDTO_NS, kind))));
    }
    check_native_branch(
        authority,
        &view,
        &branch_at,
        &types.keys().cloned().collect(),
    );
    evidence.record(
        line,
        &owner,
        "Type",
        None,
        &origin,
        AddressEvidence::Read(&branch_at),
    );
    for (at, node) in types {
        let view = read(authority, &at);
        assert_eq!(view["kind"], "Type");
        assert_eq!(view["props"]["kind"], node.tag_name().name(), "{at}");
        check_native_xdto_qname(node, "base", &view["props"]["base"], &at);
        evidence.record(
            line,
            &owner,
            "Type",
            None,
            &origin,
            AddressEvidence::Read(&at),
        );
        check_native_xdto_properties(authority, &at, &view, node, &origin, evidence);
    }
}

fn check_native_branch_count(view: &Value, at: &str, count: usize) {
    let rows: Vec<_> = view["branches"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|row| row["at"] == at)
        .map(|row| row["count"].as_u64().unwrap())
        .collect();
    assert_eq!(rows, [count as u64], "{at}: published branch count");
}

fn check_native_xdto_qname(
    node: roxmltree::Node<'_, '_>,
    attribute: &str,
    projected: &Value,
    at: &str,
) {
    if let Some(raw) = node.attribute(attribute) {
        let (prefix, local) = raw.split_once(':').unwrap();
        assert_eq!(projected["raw"], raw, "{at}: {attribute}");
        assert_eq!(projected["prefix"], prefix, "{at}: {attribute}");
        assert_eq!(projected["local"], local, "{at}: {attribute}");
        assert_eq!(
            projected["namespace"],
            node.lookup_namespace_uri(Some(prefix)).unwrap(),
            "{at}: {attribute}",
        );
    }
}

fn check_native_xdto_properties(
    authority: &LogicalViewReadAuthority<'_>,
    owner: &str,
    view: &Value,
    raw_owner: roxmltree::Node<'_, '_>,
    origin: &str,
    evidence: &mut EvidenceRows,
) {
    let properties: Vec<_> = raw_owner
        .children()
        .filter(|node| node.has_tag_name((NATIVE_XDTO_NS, "property")))
        .collect();
    if properties.is_empty() {
        return;
    }
    let branch_at = format!("{owner}.Property");
    let branch = QualifiedAddress::parse(&branch_at).unwrap();
    let mut named = BTreeMap::new();
    let mut references = BTreeMap::new();
    for property in &properties {
        if let Some(name) = property.attribute("name") {
            let at = branch.member(name).unwrap().to_string();
            assert!(named.insert(at, property).is_none());
        } else {
            // A ref-only declaration is a counted row with a QName, without an address.
            let raw = property
                .attribute("ref")
                .expect("unnamed native property reference");
            let (prefix, local) = raw.split_once(':').unwrap();
            let namespace = property.lookup_namespace_uri(Some(prefix)).unwrap();
            assert!(references.insert(raw, (local, namespace)).is_none());
        }
    }
    check_native_branch_count(view, &branch_at, properties.len());
    let collection = read(authority, &branch_at);
    let items = collection["items"].as_array().unwrap();
    assert_eq!(items.len(), properties.len(), "{branch_at}");
    let mut addressed = BTreeSet::new();
    for row in items {
        if let Some(at) = row["at"].as_str() {
            assert!(addressed.insert(at), "{branch_at}: duplicate {at}");
        } else {
            let reference = &row["ref"];
            let raw = reference["raw"]
                .as_str()
                .expect("unaddressed XDTO reference row");
            let (local, namespace) = references
                .remove(raw)
                .unwrap_or_else(|| panic!("{branch_at}: unexpected reference row {raw}"));
            assert_eq!(reference["local"], local);
            assert_eq!(reference["namespace"], namespace);
        }
    }
    assert!(references.is_empty(), "{branch_at}: missing reference rows");
    assert_eq!(
        addressed,
        named.keys().map(String::as_str).collect::<BTreeSet<_>>(),
        "{branch_at}"
    );
    let line = authority.platform_profile().line();
    evidence.record(
        line,
        owner,
        "Property",
        None,
        origin,
        AddressEvidence::Read(&branch_at),
    );
    for (at, raw) in named {
        let value = read(authority, &at);
        assert_eq!(value["kind"], "Property");
        check_native_xdto_qname(*raw, "type", &value["props"]["type"], &at);
        let lower = raw.attribute("lowerBound");
        let upper = raw.attribute("upperBound");
        assert_eq!(
            value["props"]["bounds"],
            serde_json::json!({
                "lower": lower.unwrap_or("1"),
                "upper": upper.unwrap_or("1"),
                "lowerExplicit": lower.is_some(),
                "upperExplicit": upper.is_some(),
                "unbounded": upper == Some("-1"),
            }),
            "{at}: native bounds",
        );
        evidence.record(
            line,
            owner,
            "Property",
            None,
            origin,
            AddressEvidence::Read(&at),
        );
        let mut definitions = raw
            .children()
            .filter(|node| node.has_tag_name((NATIVE_XDTO_NS, "typeDef")));
        if let Some(definition) = definitions.next() {
            assert!(
                definitions.next().is_none(),
                "{at}: native anonymous type must be unique"
            );
            let kind = definition
                .attribute(("http://www.w3.org/2001/XMLSchema-instance", "type"))
                .expect("native anonymous type discriminator");
            assert_eq!(value["props"]["anonymousType"], kind, "{at}");
            check_native_xdto_properties(authority, &at, &value, definition, origin, evidence);
        }
    }
}

fn check_native_sales_role_rights(
    authority: &LogicalViewReadAuthority<'_>,
    source: &Path,
    owner: &QualifiedAddress,
    format: &str,
    evidence: &mut EvidenceRows,
) {
    const NS: &str = "http://v8.1c.ru/8.2/roles";
    let descriptor = source.join("Roles/SalesReader.xml");
    let xml = read_fixture_descriptor(&descriptor);
    let document = parse_fixture_descriptor(&xml, &descriptor);
    assert_eq!(document.root_element().attribute("version"), Some(format));
    let role = fixture_artifact(&document, "Role", &descriptor);
    assert_eq!(fixture_properties_name(role), Some("SalesReader"));
    assert!(uuid::Uuid::parse_str(role.attribute("uuid").unwrap()).is_ok());

    let path = descriptor.with_extension("").join("Ext/Rights.xml");
    let xml = read_fixture_descriptor(&path);
    let document = parse_fixture_descriptor(&xml, &path);
    let root = document.root_element();
    assert!(root.has_tag_name((NS, "Rights")));
    assert_eq!(root.attribute("version"), Some(format));
    fn text<'a, 'input: 'a>(node: roxmltree::Node<'a, 'input>, tag: &str) -> &'a str {
        node.children()
            .find(|child| child.has_tag_name((NS, tag)))
            .and_then(|child| child.text())
            .unwrap()
    }
    let branch_at = format!("{owner}.Right");
    let branch = QualifiedAddress::parse(&branch_at).unwrap();
    let mut expected = BTreeMap::new();
    for object in root
        .children()
        .filter(|node| node.has_tag_name((NS, "object")))
    {
        let target = text(object, "name");
        let (kind, name) = target.split_once('.').unwrap();
        let mut rights = BTreeSet::new();
        for right in object
            .children()
            .filter(|node| node.has_tag_name((NS, "right")))
        {
            assert_eq!(text(right, "value"), "true", "{target}");
            assert!(
                !right
                    .children()
                    .any(|child| { child.has_tag_name((NS, "restrictionByCondition")) }),
                "{target}: fixture must have no RLS restriction"
            );
            assert!(rights.insert(text(right, "name")), "{target}");
        }
        assert_eq!(rights, BTreeSet::from(["Read", "View"]), "{target}");
        let at = branch
            .member(&format!("{kind}_{name}"))
            .unwrap()
            .to_string();
        assert!(expected
            .insert(at, (target, kind, name, rights.len()))
            .is_none());
    }
    assert_eq!(
        expected
            .values()
            .map(|(target, ..)| *target)
            .collect::<BTreeSet<_>>(),
        BTreeSet::from(["Catalog.Валюты", "Document.Orders"]),
    );
    let owner = owner.to_string();
    check_native_branch(
        authority,
        &read(authority, &owner),
        &branch_at,
        &expected.keys().cloned().collect(),
    );
    let line = authority.platform_profile().line();
    let origin = fixture_origin(&path);
    evidence.record(
        line,
        &owner,
        "Right",
        None,
        &origin,
        AddressEvidence::Read(&branch_at),
    );
    for (at, (_, kind, name, allowed)) in expected {
        let view = read(authority, &at);
        assert_eq!(view["kind"], "Right");
        assert_eq!(view["title"], name);
        assert_eq!(view["props"]["objectKind"], kind);
        assert_eq!(view["props"]["allowedCount"], allowed);
        assert_eq!(view["props"]["deniedCount"], 0);
        let rls = format!("{at}.RLS");
        assert!(
            !view["branches"]
                .as_array()
                .into_iter()
                .flatten()
                .any(|branch| branch["at"] == rls),
            "{at}: unexpected RLS branch"
        );
        assert!(
            addresses(&read(authority, &rls), "items").is_empty(),
            "{rls}"
        );
        evidence.record(line, &at, "RLS", None, &origin, AddressEvidence::Read(&rls));
        evidence.record(
            line,
            &owner,
            "Right",
            None,
            &origin,
            AddressEvidence::Read(&at),
        );
    }
}

fn check_native_subsystem(
    authority: &LogicalViewReadAuthority<'_>,
    source: &Path,
    owner: &QualifiedAddress,
    format: &str,
    evidence: &mut EvidenceRows,
) {
    const PARENT: &str = "СтандартныеПодсистемы";
    const CHILD: &str = "БазоваяФункциональность";
    const CONTENT_ITEM_NS: &str = "http://v8.1c.ru/8.3/xcf/readable";
    let parent_descriptor = source.join(format!("Subsystems/{PARENT}.xml"));
    let child_descriptor = parent_descriptor
        .with_extension("")
        .join(format!("Subsystems/{CHILD}.xml"));
    let parent = owner.to_string();
    let branch = QualifiedAddress::parse(&format!("{parent}.Subsystem")).unwrap();
    let child = branch.member(CHILD).unwrap().to_string();
    let line = authority.platform_profile().line();
    for (at, descriptor, name, uuid, children) in [
        (
            &parent,
            &parent_descriptor,
            PARENT,
            "a463669b-6d24-58c2-8f3c-9a043a8abd57",
            &[CHILD][..],
        ),
        (
            &child,
            &child_descriptor,
            CHILD,
            "019b1d43-0392-5735-bf96-57691a956f90",
            &[][..],
        ),
    ] {
        let xml = read_fixture_descriptor(descriptor);
        let document = parse_fixture_descriptor(&xml, descriptor);
        assert_eq!(document.root_element().attribute("version"), Some(format));
        let artifact = fixture_artifact(&document, "Subsystem", descriptor);
        assert_eq!(artifact.attribute("uuid"), Some(uuid));
        assert_eq!(fixture_properties_name(artifact), Some(name));
        assert_eq!(
            artifact
                .children()
                .filter(|node| node.has_tag_name((MD_CLASSES_NS, "ChildObjects")))
                .count(),
            1,
            "{at}: native ChildObjects container",
        );
        let registered: Vec<_> = native_child_objects(artifact)
            .map(|node| {
                assert!(node.has_tag_name((MD_CLASSES_NS, "Subsystem")));
                node.text().expect("native Subsystem registration")
            })
            .collect();
        assert_eq!(registered, children, "{at}: native children");
        let content = artifact
            .children()
            .find(|node| node.has_tag_name((MD_CLASSES_NS, "Properties")))
            .and_then(|props| {
                props
                    .children()
                    .find(|node| node.has_tag_name((MD_CLASSES_NS, "Content")))
            })
            .expect("native Subsystem Content");
        let refs: Vec<_> = content
            .children()
            .filter(|node| node.is_element())
            .map(|node| {
                assert!(node.has_tag_name((CONTENT_ITEM_NS, "Item")));
                node.text().expect("native Subsystem content target")
            })
            .collect();
        assert_eq!(refs, ["CommonModule.EventHandlers"], "{at}: native content");

        let origin = fixture_origin(descriptor);
        let view = read(authority, at);
        assert_eq!(view["kind"], "Subsystem");
        if at == &parent && matches!(format, "2.21" | "2.22") {
            assert_native_projection_file_absent(
                &parent_descriptor
                    .with_extension("")
                    .join("Ext/CommandInterface.xml"),
            );
            let interface = format!("{at}.Interface");
            assert!(!addresses(&view, "branches").contains(&interface));
            check_native_projection_refusal(
                authority,
                &interface,
                "subsystem has no command interface document",
            );
        }
        let relation = format!("{at}.Relation");
        let target = format!("main:{}", refs[0]);
        check_native_branch(
            authority,
            &view,
            &relation,
            &BTreeSet::from([target.clone()]),
        );
        assert_eq!(
            read(authority, &relation)["items"],
            serde_json::json!([{"relation": "content", "at": target, "kind": "CommonModule"}]),
            "{at}: exact Relation content",
        );
        evidence.record(
            line,
            at,
            "Relation",
            None,
            &origin,
            AddressEvidence::Read(&relation),
        );

        let subsystem = format!("{at}.Subsystem");
        if let [name] = registered.as_slice() {
            let member = branch.member(name).unwrap().to_string();
            check_native_branch(
                authority,
                &view,
                &subsystem,
                &BTreeSet::from([member.clone()]),
            );
            assert_eq!(member, child);
            evidence.record(
                line,
                at,
                "Subsystem",
                None,
                &origin,
                AddressEvidence::Read(&member),
            );
        } else {
            assert!(
                !addresses(&view, "branches").contains(&subsystem),
                "{at}: empty child list"
            );
            let empty = read(authority, &subsystem);
            assert_eq!(empty["kind"], "Subsystem");
            assert!(addresses(&empty, "items").is_empty(), "{subsystem}");
        }
        evidence.record(
            line,
            at,
            "Subsystem",
            None,
            &origin,
            AddressEvidence::Read(&subsystem),
        );
    }
}

fn check_native_empty_nested_collections(
    authority: &LogicalViewReadAuthority<'_>,
    source: &Path,
    format: &str,
    registrations: &[(String, String)],
    evidence: &mut EvidenceRows,
) {
    // These samples prove direct reads from empty exports. Whether the platform
    // allows a child kind is a separate question from what this descriptor holds.
    for (kind, name, directory, container_count, branches) in [
        (
            "Constant",
            "Enabled",
            "Constants",
            0,
            &["Form", "Command"][..],
        ),
        (
            "DocumentNumerator",
            "Sales",
            "DocumentNumerators",
            0,
            &["Form", "Command"][..],
        ),
        (
            "Sequence",
            "Probe",
            "Sequences",
            1,
            &["Form", "Command"][..],
        ),
        (
            "SettingsStorage",
            "Probe",
            "SettingsStorages",
            1,
            &["Form"][..],
        ),
    ] {
        let owner = format!("main:{kind}.{name}");
        assert!(
            registrations
                .iter()
                .any(|(tag, member)| tag == kind && member == name),
            "{owner}: missing root registration"
        );
        let descriptor = source.join(directory).join(format!("{name}.xml"));
        let xml = read_fixture_descriptor(&descriptor);
        let document = parse_fixture_descriptor(&xml, &descriptor);
        assert_eq!(document.root_element().attribute("version"), Some(format));
        let artifact = fixture_artifact(&document, kind, &descriptor);
        assert_eq!(fixture_properties_name(artifact), Some(name));
        let containers = artifact
            .children()
            .filter(|node| node.has_tag_name((MD_CLASSES_NS, "ChildObjects")));
        assert_eq!(containers.count(), container_count, "{owner}");
        assert!(native_child_objects(artifact).next().is_none(), "{owner}");
        let origin = fixture_origin(&descriptor);
        for &branch in branches {
            let at = format!("{owner}.{branch}");
            let collection = read(authority, &at);
            assert_eq!(collection["kind"], branch, "{at}");
            assert_eq!(collection["items"], serde_json::json!([]), "{at}");
            evidence.record(
                authority.platform_profile().line(),
                &owner,
                branch,
                None,
                &origin,
                AddressEvidence::Read(&at),
            );
        }
    }
}

fn check_native_projection_refusal(
    authority: &LogicalViewReadAuthority<'_>,
    address: &str,
    reason: &str,
) {
    let at = QualifiedAddress::parse(address).unwrap();
    let admitted = authority.snapshot(&at).unwrap();
    let error = authority
        .read_exact(&at, &admitted)
        .err()
        .unwrap_or_else(|| panic!("{at}: expected not_found"));
    assert_eq!(error.code(), unica_refusal::RefusalCode::NotFound, "{at}");
    assert!(error.to_string().contains(reason), "{at}: {error}");
}

fn native_named_declaration_members(
    container: roxmltree::Node<'_, '_>,
    branch: &str,
    kind: &str,
) -> BTreeSet<String> {
    const READABLE_NS: &str = "http://v8.1c.ru/8.3/xcf/readable";
    let collection = QualifiedAddress::parse(branch).unwrap();
    let mut members = BTreeSet::new();
    for member in container.children().filter(roxmltree::Node::is_element) {
        assert!(
            member.has_tag_name((READABLE_NS, kind)),
            "{branch}: {member:?}"
        );
        let name = member.attribute("name").expect("native declaration name");
        let at = collection.member(name).unwrap().to_string();
        assert!(members.insert(at), "{branch}: duplicate {name}");
    }
    assert!(
        !members.is_empty(),
        "{branch}: native fixture must have members"
    );
    members
}

fn check_native_named_declarations(
    authority: &LogicalViewReadAuthority<'_>,
    owner: &str,
    kind: &str,
    expected: &BTreeSet<String>,
    origin: &str,
    evidence: &mut EvidenceRows,
) {
    let branch = format!("{owner}.{kind}");
    check_native_branch(authority, &read(authority, owner), &branch, expected);
    for at in expected {
        assert_eq!(read(authority, at)["kind"], kind, "{at}");
    }
    evidence.record(
        authority.platform_profile().line(),
        owner,
        kind,
        None,
        origin,
        AddressEvidence::Read(&branch),
    );
}

fn check_native_declaration_projections(
    authority: &LogicalViewReadAuthority<'_>,
    source: &Path,
    registrations: &[(String, String)],
    evidence: &mut EvidenceRows,
) {
    const READABLE_NS: &str = "http://v8.1c.ru/8.3/xcf/readable";
    let line = authority.platform_profile().line();
    for (kind, name, relative) in [
        ("Catalog", "Валюты", "Catalogs/Валюты.xml"),
        (
            "ChartOfAccounts",
            "Хозрасчетный",
            "ChartsOfAccounts/Хозрасчетный.xml",
        ),
    ] {
        assert!(registrations
            .iter()
            .any(|(tag, item)| tag == kind && item == name));
        let owner = format!("main:{kind}.{name}");
        let descriptor = source.join(relative);
        let origin = fixture_origin(&descriptor);
        let xml = read_fixture_descriptor(&descriptor);
        let document = parse_fixture_descriptor(&xml, &descriptor);
        let artifact = fixture_artifact(&document, kind, &descriptor);
        assert_eq!(fixture_properties_name(artifact), Some(name));
        let properties = artifact
            .children()
            .find(|node| node.has_tag_name((MD_CLASSES_NS, "Properties")))
            .unwrap();
        let sections = properties
            .children()
            .find(|node| node.has_tag_name((MD_CLASSES_NS, "StandardTabularSections")));
        if kind == "Catalog" {
            let attributes = properties
                .children()
                .find(|node| node.has_tag_name((MD_CLASSES_NS, "StandardAttributes")))
                .unwrap();
            let branch = format!("{owner}.StandardAttribute");
            let members =
                native_named_declaration_members(attributes, &branch, "StandardAttribute");
            check_native_named_declarations(
                authority,
                &owner,
                "StandardAttribute",
                &members,
                &origin,
                evidence,
            );
            let characteristics = properties
                .children()
                .find(|node| node.has_tag_name((MD_CLASSES_NS, "Characteristics")))
                .unwrap();
            assert!(!characteristics.children().any(|node| node.is_element()));
            assert_native_projection_file_absent(
                &source.join("Catalogs/Валюты/Ext/Predefined.xml"),
            );
            for (kind, rejected_suffix, reason) in [
                (
                    "Characteristic",
                    "Any",
                    "a characteristic carries no applied name",
                ),
                (
                    "PredefinedItem",
                    "Absent",
                    "metadata node `Absent` was not found",
                ),
            ] {
                let branch = format!("{owner}.{kind}");
                let view = read(authority, &branch);
                assert_eq!(view["kind"], kind);
                assert!(view["items"].as_array().unwrap().is_empty(), "{branch}");
                check_native_projection_refusal(
                    authority,
                    &format!("{branch}.{rejected_suffix}"),
                    reason,
                );
                evidence.record(
                    line,
                    &owner,
                    kind,
                    None,
                    &origin,
                    AddressEvidence::Read(&branch),
                );
            }
            assert!(
                sections.is_none(),
                "{owner}: fixture has no StandardTabularSections"
            );
            for suffix in [
                "StandardTabularSection",
                "StandardTabularSection.ExtDimensionTypes.StandardAttribute",
            ] {
                check_native_projection_refusal(
                    authority,
                    &format!("{owner}.{suffix}"),
                    "metadata has no StandardTabularSection collection",
                );
            }
        } else {
            let sections = sections.unwrap();
            let branch = format!("{owner}.StandardTabularSection");
            let members =
                native_named_declaration_members(sections, &branch, "StandardTabularSection");
            check_native_named_declarations(
                authority,
                &owner,
                "StandardTabularSection",
                &members,
                &origin,
                evidence,
            );
            for section in sections.children().filter(roxmltree::Node::is_element) {
                let name = section.attribute("name").unwrap();
                let member = QualifiedAddress::parse(&branch)
                    .unwrap()
                    .member(name)
                    .unwrap()
                    .to_string();
                let attributes = section
                    .children()
                    .find(|node| node.has_tag_name((READABLE_NS, "StandardAttributes")))
                    .unwrap();
                let branch = format!("{member}.StandardAttribute");
                let members =
                    native_named_declaration_members(attributes, &branch, "StandardAttribute");
                check_native_named_declarations(
                    authority,
                    &member,
                    "StandardAttribute",
                    &members,
                    &origin,
                    evidence,
                );
            }
        }
    }
}

fn assert_native_projection_file_absent(path: &Path) {
    let error = fs::symlink_metadata(path).expect_err("native fixture path must be absent");
    assert_eq!(error.kind(), std::io::ErrorKind::NotFound, "{path:?}");
}

fn check_native_projection_companions(
    authority: &LogicalViewReadAuthority<'_>,
    source: &Path,
    registrations: &[(String, String)],
    cancellation: &CancellationToken,
    evidence: &mut EvidenceRows,
) {
    let line = authority.platform_profile().line();
    let root =
        unica_workspace::read::open_source_directory(&source.canonicalize().unwrap()).unwrap();
    let find_sources = [LayoutFindSource::new(
        "main",
        SourceSetKind::Configuration,
        &root,
    )];
    let finder = WorkspaceFindDirectoryBuilder::default();
    for (kind, name, branch, owner_file, companion) in [
        (
            "ScheduledJob",
            "KeyProbe",
            "Schedule",
            "ScheduledJobs/KeyProbe.xml",
            "ScheduledJobs/KeyProbe/Ext/Schedule.xml",
        ),
        (
            "WSReference",
            "Probe",
            "Definition",
            "WSReferences/Probe.xml",
            "WSReferences/Probe/Ext/WSDefinition.xml",
        ),
    ] {
        assert!(registrations
            .iter()
            .any(|(tag, item)| tag == kind && item == name));
        let owner = format!("main:{kind}.{name}");
        let descriptor = source.join(owner_file);
        let xml = read_fixture_descriptor(&descriptor);
        let document = parse_fixture_descriptor(&xml, &descriptor);
        let artifact = fixture_artifact(&document, kind, &descriptor);
        assert_eq!(fixture_properties_name(artifact), Some(name));
        let address = format!("{owner}.{branch}");
        let at = QualifiedAddress::parse(&address).unwrap();
        let view = read(authority, &address);
        assert_eq!(view["kind"], branch);
        let resource = source.join(companion);
        let find_path = || {
            finder
                .locate_path(
                    &find_sources,
                    companion,
                    ProviderDeadline::no_deadline(),
                    cancellation,
                )
                .unwrap()
        };
        let (origin, witness) = if branch == "Schedule" {
            assert_native_projection_file_absent(&resource);
            assert_eq!(view["props"], serde_json::json!({"state": "absent"}));
            let error = authority.existing_export_path(&at, None).unwrap_err();
            assert_eq!(
                error.code(),
                unica_refusal::RefusalCode::NotFound,
                "{address}"
            );
            assert!(find_path().is_none(), "{companion}");
            (fixture_origin(&descriptor), AddressEvidence::Read(&address))
        } else {
            let wsdl = read_fixture_descriptor(&resource);
            let document = parse_fixture_descriptor(&wsdl, &resource);
            let definition = document.root_element();
            assert!(definition.has_tag_name(("http://schemas.xmlsoap.org/wsdl/", "definitions")));
            let rows = view["items"].as_array().unwrap();
            assert!(rows.len() > 1, "{address}: expected declaration rows");
            assert_eq!(rows[0]["kind"], "definitions");
            assert_eq!(rows[0]["name"].as_str(), definition.attribute("name"));
            assert_eq!(
                rows[0]["targetNamespace"].as_str(),
                definition.attribute("targetNamespace")
            );
            for (index, row) in rows.iter().enumerate() {
                assert!(
                    row.get("at").is_none(),
                    "{address}: a WSDL row has no address"
                );
                assert_eq!(row["sourceOrder"].as_u64(), Some((index + 1) as u64));
            }
            assert_eq!(
                authority.existing_export_path(&at, None).unwrap(),
                companion
            );
            let located =
                find_path().unwrap_or_else(|| panic!("missing WSDL source identity {companion}"));
            assert_eq!(located.at(), address);
            assert_eq!(located.kind(), branch);
            assert_eq!(located.placed_path(), Some(companion));
            (
                fixture_origin(&resource),
                AddressEvidence::RoundTrip(&address),
            )
        };
        evidence.record(line, &owner, branch, None, &origin, witness);
    }
}

fn check_native_job_schedule_roundtrip(
    source: &Path,
    format: &str,
    profile: PlatformProfile,
    cancellation: &CancellationToken,
    evidence: &mut EvidenceRows,
) {
    const OWNER: &str = "main:ScheduledJob.KeyProbe";
    const ADDRESS: &str = "main:ScheduledJob.KeyProbe.Schedule";
    const COMPANION: &str = "ScheduledJobs/KeyProbe/Ext/Schedule.xml";
    const NS: &str = "http://v8.1c.ru/8.3/xcf/extrnprops";
    let native = fixture_path(&format!(
        "acceptance/native-scheduled-job-schedule/{format}/Schedule.xml"
    ));
    let overlay = tempfile::tempdir().unwrap();
    let resource = overlay.path().join(COMPANION);
    fs::create_dir_all(resource.parent().unwrap()).unwrap();
    for relative in ["Configuration.xml", "ScheduledJobs/KeyProbe.xml"] {
        fs::copy(source.join(relative), overlay.path().join(relative)).unwrap();
    }
    fs::copy(&native, resource).unwrap();

    let raw = read_fixture_descriptor(&native);
    let document = parse_fixture_descriptor(&raw, &native);
    let root = document.root_element();
    assert!(root.has_tag_name((NS, "JobSchedule")));
    assert_eq!(root.attribute("version"), Some(format));
    let schedule = root
        .children()
        .find(|node| node.has_tag_name((NS, "Schedule")))
        .unwrap();
    let authority = native_authority(cancellation, overlay.path(), profile);
    let view = read(&authority, ADDRESS);
    assert_eq!(view["kind"], "Schedule");
    assert_eq!(
        view["props"]["BeginTime"].as_str().unwrap(),
        schedule.attribute("BeginTime").unwrap()
    );

    let at = QualifiedAddress::parse(ADDRESS).unwrap();
    assert_eq!(
        authority.existing_export_path(&at, None).unwrap(),
        COMPANION
    );
    let root =
        unica_workspace::read::open_source_directory(&overlay.path().canonicalize().unwrap())
            .unwrap();
    let found = WorkspaceFindDirectoryBuilder::default()
        .locate_path(
            &[LayoutFindSource::new(
                "main",
                SourceSetKind::Configuration,
                &root,
            )],
            COMPANION,
            ProviderDeadline::no_deadline(),
            cancellation,
        )
        .unwrap()
        .unwrap_or_else(|| panic!("missing job schedule source identity {COMPANION}"));
    assert_eq!(found.at(), ADDRESS);
    assert_eq!(found.kind(), "Schedule");
    assert_eq!(found.placed_path(), Some(COMPANION));
    evidence.record(
        profile.line(),
        OWNER,
        "Schedule",
        None,
        &fixture_origin(&native),
        AddressEvidence::RoundTrip(ADDRESS),
    );
}

fn read_fixture_descriptor(path: &Path) -> String {
    fs::read_to_string(path)
        .unwrap_or_else(|error| panic!("native fixture descriptor {}: {error}", path.display()))
}

fn parse_fixture_descriptor<'a>(xml: &'a str, path: &Path) -> roxmltree::Document<'a> {
    roxmltree::Document::parse(xml.trim_start_matches('\u{feff}'))
        .unwrap_or_else(|error| panic!("invalid native fixture XML {}: {error}", path.display()))
}

fn fixture_artifact<'a, 'input: 'a>(
    document: &'a roxmltree::Document<'input>,
    kind: &str,
    path: &Path,
) -> roxmltree::Node<'a, 'input> {
    let root = document.root_element();
    assert!(
        root.has_tag_name((MD_CLASSES_NS, "MetaDataObject")),
        "native fixture {} has no MetaDataObject root",
        path.display()
    );
    root.children()
        .find(|node| node.has_tag_name((MD_CLASSES_NS, kind)))
        .unwrap_or_else(|| panic!("native fixture {} has no {kind} descriptor", path.display()))
}

fn fixture_properties_name<'a, 'input: 'a>(
    artifact: roxmltree::Node<'a, 'input>,
) -> Option<&'a str> {
    artifact
        .children()
        .find(|child| child.has_tag_name((MD_CLASSES_NS, "Properties")))?
        .children()
        .find(|child| child.has_tag_name((MD_CLASSES_NS, "Name")))?
        .text()
}

fn native_child_objects<'a, 'input: 'a>(
    artifact: roxmltree::Node<'a, 'input>,
) -> impl Iterator<Item = roxmltree::Node<'a, 'input>> {
    artifact
        .children()
        .filter(|node| node.has_tag_name((MD_CLASSES_NS, "ChildObjects")))
        .flat_map(|node| node.children().filter(roxmltree::Node::is_element))
}

fn native_described_branches(
    owner: &str,
    owner_descriptor: &Path,
    artifact: roxmltree::Node<'_, '_>,
) -> BTreeMap<String, BTreeSet<String>> {
    let mut branches = BTreeMap::<_, BTreeSet<_>>::new();
    for child in native_child_objects(artifact) {
        let kind = child.tag_name().name();
        let Some(directory) =
            nested_place(artifact.tag_name().name(), kind).and_then(|place| place.directory)
        else {
            continue;
        };
        assert_eq!(
            child.tag_name().namespace(),
            Some(MD_CLASSES_NS),
            "native fixture {owner}: foreign namespace for {kind}"
        );
        let name = child
            .text()
            .map(str::trim)
            .filter(|name| !name.is_empty())
            .unwrap_or_else(|| {
                panic!("native fixture {owner}: {kind} registration must name its descriptor")
            });
        let branch = format!("{owner}.{kind}");
        let at = format!("{branch}.{name}");
        let parsed = QualifiedAddress::parse(&at).unwrap_or_else(|error| {
            panic!("native fixture {owner}: invalid {kind} registration {name}: {error}")
        });
        assert_eq!(
            parsed.to_string(),
            at,
            "native fixture {owner}: noncanonical registration"
        );
        let descriptor = owner_descriptor
            .with_extension("")
            .join(directory)
            .join(format!("{name}.xml"));
        let xml = read_fixture_descriptor(&descriptor);
        let document = parse_fixture_descriptor(&xml, &descriptor);
        let resource = fixture_artifact(&document, kind, &descriptor);
        assert!(
            resource
                .attribute("uuid")
                .is_some_and(|uuid| uuid::Uuid::parse_str(uuid).is_ok()),
            "native fixture {} has no valid UUID",
            descriptor.display()
        );
        let descriptor_name = resource
            .children()
            .find(|node| node.has_tag_name((MD_CLASSES_NS, "Properties")))
            .and_then(|node| {
                node.children()
                    .find(|node| node.has_tag_name((MD_CLASSES_NS, "Name")))
            })
            .and_then(|node| node.text());
        assert_eq!(
            descriptor_name,
            Some(name),
            "native fixture {} disagrees with its registration",
            descriptor.display()
        );
        assert_eq!(
            document.root_element().attribute("version"),
            artifact.document().root_element().attribute("version"),
            "native fixture {} differs from owner format",
            descriptor.display()
        );
        assert!(
            branches.entry(branch).or_default().insert(at),
            "native fixture {owner}: duplicate {kind}.{name}"
        );
    }
    branches
}

type NativeInlineBranches = BTreeMap<String, BTreeMap<&'static str, BTreeSet<String>>>;

fn collect_native_inline_branches(
    owner: &QualifiedAddress,
    artifact: roxmltree::Node<'_, '_>,
    uuids: &mut BTreeSet<uuid::Uuid>,
    branches: &mut NativeInlineBranches,
) {
    let owner_kind = artifact.tag_name().name();
    for node in native_child_objects(artifact) {
        let kind = node.tag_name().name();
        assert_eq!(
            node.tag_name().namespace(),
            Some(MD_CLASSES_NS),
            "native fixture {owner}: foreign namespace for {kind}"
        );
        if nested_place(owner_kind, kind).is_some_and(|place| place.directory.is_some()) {
            continue;
        }
        // These registrations can name a separate object without embedding one.
        if matches!(kind, "Form" | "Template" | "Command" | "Subsystem")
            && node.attribute("uuid").is_none()
            && !node.children().any(|child| child.is_element())
        {
            assert!(
                node.text().is_some_and(|name| !name.trim().is_empty()),
                "native fixture {owner}: empty {kind} registration"
            );
            continue;
        }
        let uuid = node
            .attribute("uuid")
            .and_then(|value| uuid::Uuid::parse_str(value).ok())
            .unwrap_or_else(|| {
                panic!("native fixture {owner}: {kind} has missing or invalid UUID")
            });
        assert!(
            uuids.insert(uuid),
            "native fixture {owner}: duplicate {kind} UUID {uuid}"
        );
        let name = node
            .children()
            .find(|child| child.has_tag_name((MD_CLASSES_NS, "Properties")))
            .and_then(|properties| {
                properties
                    .children()
                    .find(|child| child.has_tag_name((MD_CLASSES_NS, "Name")))
            })
            .and_then(|name| name.text())
            .map(str::trim)
            .filter(|name| !name.is_empty())
            .unwrap_or_else(|| panic!("native fixture {owner}: {kind} has no Properties.Name"));
        let kind = NodeKind::parse(kind)
            .unwrap_or_else(|error| panic!("native fixture {owner}: {error}"))
            .as_str();
        let branch = QualifiedAddress::parse(&format!("{owner}.{kind}")).unwrap_or_else(|error| {
            panic!("native fixture {owner}: invalid {kind} branch: {error}")
        });
        let item = branch.member(name).unwrap_or_else(|error| {
            panic!("native fixture {owner}: invalid {kind} name {name}: {error}")
        });
        assert!(
            branches
                .entry(owner.to_string())
                .or_default()
                .entry(kind)
                .or_default()
                .insert(item.to_string()),
            "native fixture {owner}: duplicate {kind}.{name}"
        );
        collect_native_inline_branches(&item, node, uuids, branches);
    }
}

#[test]
#[should_panic(expected = "Attribute has missing or invalid UUID")]
fn native_inline_registration_does_not_skip_a_malformed_member() {
    let xml = format!(
        r#"<Catalog xmlns="{MD_CLASSES_NS}"><ChildObjects><Attribute uuid="00000000-0000-0000-0000-000000000001"><Properties><Name>Valid</Name></Properties></Attribute><Attribute><Properties><Name>MissingUuid</Name></Properties></Attribute></ChildObjects></Catalog>"#
    );
    let document = roxmltree::Document::parse(&xml).unwrap();
    let owner = QualifiedAddress::parse("main:Catalog.Probe").unwrap();
    collect_native_inline_branches(
        &owner,
        document.root_element(),
        &mut BTreeSet::new(),
        &mut BTreeMap::new(),
    );
}

fn check_native_branch(
    authority: &LogicalViewReadAuthority<'_>,
    owner_view: &Value,
    branch: &str,
    expected: &BTreeSet<String>,
) -> Value {
    let mut published = owner_view["branches"]
        .as_array()
        .expect("owner must publish its branches")
        .iter()
        .filter(|row| row["at"] == branch);
    let row = published
        .next()
        .unwrap_or_else(|| panic!("missing published branch {branch} for {expected:?}"));
    assert!(
        published.next().is_none(),
        "duplicate published branch {branch}"
    );
    assert_eq!(
        row["count"].as_u64(),
        Some(expected.len() as u64),
        "published branch count for {branch}"
    );
    let collection = read(authority, branch);
    let actual = addresses(&collection, "items");
    assert_eq!(&actual, expected, "published child addresses for {branch}");
    collection
}

fn check_native_empty_metadata_branches(
    authority: &LogicalViewReadAuthority<'_>,
    owner: &QualifiedAddress,
    artifact: roxmltree::Node<'_, '_>,
    published: &Value,
    origin: &str,
    verified: &mut BTreeSet<String>,
    evidence: &mut EvidenceRows,
) {
    let profile = authority.platform_profile();
    let owner_at = owner.to_string();
    let owner_kind = artifact.tag_name().name();
    let declared = platform_child_object_order(owner_kind).unwrap_or_default();
    let raw_kinds: BTreeSet<_> = native_child_objects(artifact)
        .map(|child| {
            assert_eq!(child.tag_name().namespace(), Some(MD_CLASSES_NS));
            child.tag_name().name()
        })
        .collect();

    // Keep missing declarations visible until their empty contracts are settled.
    let pending_empty: &[&str] = match owner_kind {
        "FilterCriterion" => &["Form", "Command"],
        "SettingsStorage" => &["Form", "Template"],
        "HTTPService" => &["URLTemplate"],
        "WebService" => &["Operation"],
        _ => &[],
    };
    for kind in pending_empty.iter().filter(|kind| !declared.contains(kind)) {
        println!(
            "| {} | {owner} | {kind} | — | empty branch outside declared order (#2224); native member present: {} |",
            profile.id(),
            raw_kinds.contains(kind),
        );
    }

    for &kind in declared {
        if raw_kinds.contains(kind) {
            continue;
        }
        if kind == "AddressingAttribute" {
            // Task requires a readable native container to distinguish empty from unknown.
            let containers = artifact
                .children()
                .filter(|node| node.has_tag_name((MD_CLASSES_NS, "ChildObjects")))
                .count();
            assert_eq!(owner_kind, "Task", "{owner}: addressing declaration");
            assert_eq!(
                containers, 1,
                "{owner}: cannot prove an empty AddressingAttribute"
            );
        }
        assert!(
            metadata_kind_exists(Some(profile.line()), kind),
            "{owner}: {kind} is declared but unavailable on {}",
            profile.id(),
        );
        let branch = format!("{owner}.{kind}");
        check_native_branch(authority, published, &branch, &BTreeSet::new());
        assert!(
            verified.insert(branch.clone()),
            "duplicate witness for {branch}"
        );
        evidence.record(
            profile.line(),
            &owner_at,
            kind,
            None,
            origin,
            AddressEvidence::Read(&branch),
        );
        println!(
            "| {} | {owner} | {kind} | {branch} | declared empty; no native registration |",
            profile.id(),
        );
    }
}

fn check_native_child_registrations(
    owner: &QualifiedAddress,
    artifact: roxmltree::Node<'_, '_>,
    facts: &[BranchFact<'_>],
) {
    // A registered child in the native XML is an independent witness that a
    // typed reader must generate its branch even if that reader drops a fact.
    let mut counts = BTreeMap::new();
    for child in native_child_objects(artifact) {
        assert_eq!(child.tag_name().namespace(), Some(MD_CLASSES_NS));
        *counts.entry(child.tag_name().name()).or_insert(0) += 1;
    }
    for (kind, count) in counts {
        let expected = facts
            .iter()
            .find(|fact| fact.kind.as_str() == kind)
            .unwrap_or_else(|| panic!("{owner}: native {kind} has no typed branch row"));
        assert_eq!(expected.count, count, "{owner}: native {kind} count");
    }
}

fn published_metadata_branches(
    owner: &QualifiedAddress,
    published: &Value,
    profile: PlatformProfile,
) -> Vec<(String, u64)> {
    published["branches"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|row| {
            let at = row["at"].as_str().unwrap();
            let parsed = QualifiedAddress::parse(at).unwrap();
            assert_eq!(parsed.to_string(), at);
            let route = route_logical_address(&parsed, profile).unwrap_or_else(|error| {
                panic!("{owner} published unreachable branch {at}: {error}")
            });
            match route.reader() {
                // BSL and the interior of a template have their own branch maps.
                LogicalReader::Module | LogicalReader::Dcs | LogicalReader::Mxl => None,
                LogicalReader::Metadata => Some((at.to_string(), row["count"].as_u64().unwrap())),
                other => panic!("{owner} published {at} through unexpected reader {other:?}"),
            }
        })
        .collect()
}

fn check_metadata_child_branches(
    authority: &LogicalViewReadAuthority<'_>,
    source: &Path,
    owner: &QualifiedAddress,
    published: &Value,
    facts: &[BranchFact<'_>],
    verified: &mut BTreeSet<String>,
    evidence: &mut EvidenceRows,
) {
    let profile = authority.platform_profile();
    let actual = published_metadata_branches(owner, published, profile);
    let expected: Vec<_> = facts
        .iter()
        .map(|fact| (format!("{owner}.{}", fact.kind.as_str()), fact.count as u64))
        .collect();
    assert_eq!(actual, expected, "{} {owner}", profile.id());

    for ((at, _), fact) in expected.iter().zip(facts) {
        let collection = read(authority, at);
        let evidence_label =
            if fact.kind == NodeKind::Schedule && collection["props"]["state"] == "absent" {
                "declared; native Schedule.xml absent"
            } else if fact.count == 0 {
                "declared; no member in this export"
            } else {
                "observed"
            };
        println!(
            "| {} | {owner} | {} | {at} | {evidence_label} |",
            profile.id(),
            fact.kind.as_str()
        );
        let branch = QualifiedAddress::parse(at).unwrap();
        for member in fact.members {
            let Some(name) = member["name"].as_str() else {
                // Characteristic facts are rows of data, not named descendants.
                assert_eq!(fact.kind, NodeKind::Characteristic, "{at}: {member}");
                continue;
            };
            let item = branch.member(name).unwrap();
            let item_at = item.to_string();
            let row = collection["items"]
                .as_array()
                .unwrap()
                .iter()
                .find(|row| row["at"].as_str() == Some(item_at.as_str()))
                .unwrap_or_else(|| panic!("{at}: typed member {name} has no address row"));
            let route = route_logical_address(&item, profile).unwrap();
            if route.reader() != LogicalReader::Metadata {
                continue;
            }
            if route
                .reader_metadata_path()
                .is_some_and(|target| target.as_str() == item.logical_path())
            {
                // EDS tables, cubes and dimension tables own descriptors and
                // their own typed collection facts.
                check_metadata_owner(authority, source, &item, verified, evidence);
                continue;
            }
            let branches = item_facts(member);
            let view = read(authority, &item_at);
            assert_eq!(
                published_metadata_branches(&item, row, profile),
                published_metadata_branches(&item, &view, profile),
                "{item}: collection row and direct view disagree"
            );
            check_metadata_child_branches(
                authority, source, &item, &view, &branches, verified, evidence,
            );
        }
    }
}

/// rule: CTR.SOURCE.QUALIFIED-LOGICAL-ADDRESS
/// rule: INV.SOURCE.PROFILE-CAPABILITIES
/// rule: INV.SOURCE.RESOLVED-RESOURCE-IDENTITY
#[test]
fn native_registered_modules_match_profile_and_physical_paths() {
    let profile_fixture = module_capability_fixture();
    assert_eq!(profile_fixture.profile, PlatformProfile::v8_3_27().id());
    let mut evidence = EvidenceRows::new(
        "native_registered_modules_match_profile_and_physical_paths",
        SourceSetKind::Configuration,
    );
    #[derive(Clone, Copy)]
    enum ModuleSamples {
        AllKinds,
        Report,
        DirectNamed,
    }

    for (xml, fixture, samples) in [
        (
            "2.20",
            "formats/platform_xml/2.20/all_kinds/src",
            ModuleSamples::AllKinds,
        ),
        (
            "2.21",
            "formats/platform_xml/2.21/all_kinds/src",
            ModuleSamples::AllKinds,
        ),
        (
            "2.22",
            "formats/platform_xml/2.22/all_kinds/src",
            ModuleSamples::AllKinds,
        ),
        (
            "2.20",
            "formats/platform_xml/2.20/template-new-xml/seed",
            ModuleSamples::Report,
        ),
        (
            "2.20",
            "formats/platform_xml/2.20/native_integration_service_module/src",
            ModuleSamples::DirectNamed,
        ),
        (
            "2.21",
            "formats/platform_xml/2.21/direct_named_module_roles/src",
            ModuleSamples::DirectNamed,
        ),
        (
            "2.22",
            "formats/platform_xml/2.22/direct_named_module_roles/src",
            ModuleSamples::DirectNamed,
        ),
    ] {
        let all_kinds = matches!(samples, ModuleSamples::AllKinds);
        let source = fixture_path(fixture);
        let line = export_format_line(xml).unwrap();
        let profile = PlatformProfile::for_line(line).unwrap();
        let cancellation = CancellationToken::new();
        let authority = native_authority(&cancellation, &source, profile);
        let root =
            unica_workspace::read::open_source_directory(&source.canonicalize().unwrap()).unwrap();
        let layout = [LayoutFindSource::new(
            "main",
            SourceSetKind::Configuration,
            &root,
        )];
        let finder = WorkspaceFindDirectoryBuilder::default();
        let registrations = root_registrations(&source, xml);
        if all_kinds {
            check_native_bsl_interiors(&authority, &source, xml, &registrations, &mut evidence);
            check_native_configuration_modules_without_bsl(
                &authority,
                &source,
                &layout,
                &cancellation,
                xml,
                &registrations,
                &mut evidence,
            );
            check_native_named_module_samples(
                &authority,
                &profile_fixture,
                &source,
                xml,
                &registrations,
                &mut evidence,
            );
        }
        let all_kinds_owners = [
            (
                "Catalog",
                "Catalogs/Валюты.xml",
                Some(&[][..]),
                &[
                    ("Manager", None, "Catalogs/Валюты/Ext/ManagerModule.bsl"),
                    ("Object", None, "Catalogs/Валюты/Ext/ObjectModule.bsl"),
                ][..],
            ),
            (
                "AccountingRegister",
                "AccountingRegisters/Бухгалтерский.xml",
                Some(&["Manager"][..]),
                &[(
                    "RecordSet",
                    None,
                    "AccountingRegisters/Бухгалтерский/Ext/RecordSetModule.bsl",
                )][..],
            ),
            (
                "AccumulationRegister",
                "AccumulationRegisters/Остатки.xml",
                Some(&["Manager"][..]),
                &[(
                    "RecordSet",
                    None,
                    "AccumulationRegisters/Остатки/Ext/RecordSetModule.bsl",
                )][..],
            ),
            (
                "BusinessProcess",
                "BusinessProcesses/Approval.xml",
                Some(&["Manager"][..]),
                &[(
                    "Object",
                    None,
                    "BusinessProcesses/Approval/Ext/ObjectModule.bsl",
                )][..],
            ),
            (
                "CalculationRegister",
                "CalculationRegisters/Начисления.xml",
                Some(&["Manager"][..]),
                &[(
                    "RecordSet",
                    None,
                    "CalculationRegisters/Начисления/Ext/RecordSetModule.bsl",
                )][..],
            ),
            (
                "ChartOfAccounts",
                "ChartsOfAccounts/Хозрасчетный.xml",
                Some(&["Manager"][..]),
                &[(
                    "Object",
                    None,
                    "ChartsOfAccounts/Хозрасчетный/Ext/ObjectModule.bsl",
                )][..],
            ),
            (
                "ChartOfCalculationTypes",
                "ChartsOfCalculationTypes/ВидыРасчета.xml",
                Some(&["Manager"][..]),
                &[(
                    "Object",
                    None,
                    "ChartsOfCalculationTypes/ВидыРасчета/Ext/ObjectModule.bsl",
                )][..],
            ),
            (
                "ChartOfCharacteristicTypes",
                "ChartsOfCharacteristicTypes/Codes.xml",
                Some(&["Manager"][..]),
                &[(
                    "Object",
                    None,
                    "ChartsOfCharacteristicTypes/Codes/Ext/ObjectModule.bsl",
                )][..],
            ),
            (
                "ExchangePlan",
                "ExchangePlans/Codes.xml",
                Some(&["Manager"][..]),
                &[("Object", None, "ExchangePlans/Codes/Ext/ObjectModule.bsl")][..],
            ),
            (
                "Task",
                "Tasks/Work.xml",
                Some(&["Manager"][..]),
                &[("Object", None, "Tasks/Work/Ext/ObjectModule.bsl")][..],
            ),
            (
                "Sequence",
                "Sequences/Probe.xml",
                None,
                &[("RecordSet", None, "Sequences/Probe/Ext/RecordSetModule.bsl")][..],
            ),
            (
                "Document",
                "Documents/Приход.xml",
                Some(&["Manager"][..]),
                &[("Object", None, "Documents/Приход/Ext/ObjectModule.bsl")][..],
            ),
            (
                "Document",
                "Documents/Orders.xml",
                None,
                &[
                    (
                        "Command",
                        Some("Probe"),
                        "Documents/Orders/Commands/Probe/Ext/CommandModule.bsl",
                    ),
                    (
                        "Command",
                        Some("Other"),
                        "Documents/Orders/Commands/Other/Ext/CommandModule.bsl",
                    ),
                ][..],
            ),
            (
                "HTTPService",
                "HTTPServices/ПроверкаСвязи.xml",
                None,
                &[(
                    "HTTPService",
                    None,
                    "HTTPServices/ПроверкаСвязи/Ext/Module.bsl",
                )][..],
            ),
            (
                "WebService",
                "WebServices/ОбменДанными.xml",
                None,
                &[(
                    "WebService",
                    None,
                    "WebServices/ОбменДанными/Ext/Module.bsl",
                )][..],
            ),
            (
                "Bot",
                "Bots/AllKindsBot.xml",
                None,
                &[("Bot", None, "Bots/AllKindsBot/Ext/Module.bsl")][..],
            ),
            (
                "WebSocketClient",
                "WebSocketClients/AllKindsWebSocketClient.xml",
                None,
                &[(
                    "WebSocketClient",
                    None,
                    "WebSocketClients/AllKindsWebSocketClient/Ext/Module.bsl",
                )][..],
            ),
        ];
        let report = [(
            "Report",
            "Reports/CorpusReport.xml",
            Some(&[][..]),
            &[
                (
                    "Manager",
                    None,
                    "Reports/CorpusReport/Ext/ManagerModule.bsl",
                ),
                ("Object", None, "Reports/CorpusReport/Ext/ObjectModule.bsl"),
            ][..],
        )];
        let integration_service = (
            "IntegrationService",
            "IntegrationServices/Probe.xml",
            None,
            &[(
                "IntegrationService",
                None,
                "IntegrationServices/Probe/Ext/Module.bsl",
            )][..],
        );
        let direct_named = [
            (
                "CommonCommand",
                "CommonCommands/Probe.xml",
                None,
                &[(
                    "Command",
                    None,
                    "CommonCommands/Probe/Ext/CommandModule.bsl",
                )][..],
            ),
            integration_service,
        ];
        let owners = match samples {
            ModuleSamples::AllKinds => &all_kinds_owners[..],
            ModuleSamples::Report => &report[..],
            ModuleSamples::DirectNamed if xml == "2.20" => {
                std::slice::from_ref(&integration_service)
            }
            ModuleSamples::DirectNamed => &direct_named[..],
        };
        let queue = (all_kinds && xml == "2.22").then_some((
            "Queue",
            "Queues/Probe.xml",
            Some(&[][..]),
            &[("Manager", None, "Queues/Probe/Ext/ManagerModule.bsl")][..],
        ));
        for (kind, descriptor, roles_without_files, modules) in owners.iter().copied().chain(queue)
        {
            let descriptor = source.join(descriptor);
            let raw = read_fixture_descriptor(&descriptor);
            let document = parse_fixture_descriptor(&raw, &descriptor);
            assert_eq!(document.root_element().attribute("version"), Some(xml));
            let artifact = fixture_artifact(&document, kind, &descriptor);
            let name = fixture_properties_name(artifact).expect("native owner Properties.Name");
            assert_eq!(descriptor.file_stem().unwrap().to_str(), Some(name));
            assert_eq!(
                registrations
                    .iter()
                    .filter(|(k, n)| k == kind && n == name)
                    .count(),
                1,
                "{xml}: exactly one {kind}.{name} registration",
            );
            let owner = format!("main:{kind}.{name}");
            // A role can be listed under Module without an exported BSL.
            // Only the native module files below are checked for round-trips.
            if let Some(roles_without_files) = roles_without_files {
                let branch = format!("{owner}.Module");
                let expected = modules
                    .iter()
                    .map(|(role, _, _)| *role)
                    .chain(roles_without_files.iter().copied())
                    .map(|role| format!("{branch}.{role}"))
                    .collect();
                check_native_branch(&authority, &read(&authority, &owner), &branch, &expected);
                evidence.record(
                    line,
                    &owner,
                    "Module",
                    None,
                    &fixture_origin(&descriptor),
                    AddressEvidence::Read(&branch),
                );
            }
            let expected_commands: BTreeSet<_> = modules
                .iter()
                .filter_map(|(_, command, _)| *command)
                .collect();
            let commands = if expected_commands.is_empty() {
                BTreeMap::new()
            } else {
                let commands = native_command_owners(&owner, artifact);
                assert_eq!(
                    commands.keys().copied().collect::<BTreeSet<_>>(),
                    expected_commands,
                    "{xml}: exact native command owners for {owner}",
                );
                check_native_branch(
                    &authority,
                    &read(&authority, &owner),
                    &format!("{owner}.Command"),
                    &commands.values().cloned().collect(),
                );
                commands
            };
            for &(role, command, relative) in modules {
                let parent = command.map_or(owner.as_str(), |name| commands[name].as_str());
                let at = if roles_without_files.is_some() {
                    let at = format!("{parent}.Module.{role}");
                    if kind != "Catalog" {
                        assert_eq!(read(&authority, &at)["kind"], "Module", "{at}");
                    }
                    if kind == "Queue" {
                        read(&authority, &format!("{at}.Body"));
                    }
                    QualifiedAddress::parse(&at).unwrap()
                } else {
                    check_registered_module_read(&authority, &profile_fixture, parent, role)
                };
                if matches!(
                    kind,
                    "Sequence" | "HTTPService" | "WebService" | "Bot" | "WebSocketClient"
                ) || command.is_some()
                {
                    evidence.record(
                        line,
                        parent,
                        "Module",
                        None,
                        &fixture_origin(&descriptor),
                        AddressEvidence::Read(&format!("{parent}.Module")),
                    );
                }
                let file = source.join(relative);
                let metadata = fs::metadata(&file).expect("native module resource");
                assert!(metadata.is_file(), "{xml}: {relative}");
                if matches!(samples, ModuleSamples::DirectNamed) {
                    assert!(!read_fixture_descriptor(&file)
                        .trim_start_matches('\u{feff}')
                        .trim()
                        .is_empty());
                }
                assert_eq!(authority.existing_export_path(&at, None).unwrap(), relative);
                let at_text = at.to_string();
                let (proof, level) = if kind == "WebSocketClient" {
                    // The reverse layout is tracked separately in #3256.
                    (AddressEvidence::Read(&at_text), "read + forward layout")
                } else {
                    let placed = finder
                        .locate_path(
                            &layout,
                            relative,
                            ProviderDeadline::no_deadline(),
                            &cancellation,
                        )
                        .unwrap()
                        .unwrap_or_else(|| panic!("{xml}: missing reverse layout for {relative}"));
                    assert_eq!(placed.at(), at_text);
                    if matches!(samples, ModuleSamples::DirectNamed) {
                        assert_eq!(placed.kind(), "Module");
                    }
                    assert_eq!(placed.placed_path(), Some(relative));
                    (AddressEvidence::RoundTrip(&at_text), "roundtrip")
                };
                evidence.record(
                    line,
                    parent,
                    "Module",
                    Some(role),
                    &fixture_origin(&file),
                    proof,
                );
                // Catalog and service bodies already have their own BSL interior witnesses.
                // WebSocketClient here is only a forward-layout witness.
                if !matches!(
                    kind,
                    "Catalog" | "HTTPService" | "WebService" | "WebSocketClient"
                ) {
                    record_native_bsl_body(
                        &authority,
                        (line, SourceSetKind::Configuration),
                        &at_text,
                        &file,
                        &mut evidence,
                    );
                }
                println!(
                    "{line} {at}: {level}; {relative} ({} bytes)",
                    metadata.len()
                );
            }
        }
    }
    for (format, module_count) in [("2.20", 17), ("2.21", 22), ("2.22", 22)] {
        check_native_configuration_form_and_command_modules(format, &mut evidence);
        check_native_configuration_physical_module_roles(format, module_count, &mut evidence);
    }
    evidence.finish();
}

fn check_native_configuration_physical_module_roles(
    format: &str,
    expected_count: usize,
    evidence: &mut EvidenceRows,
) {
    #[derive(serde::Deserialize)]
    struct Manifest {
        #[serde(rename = "formatVersion")]
        format: String,
        samples: Vec<Sample>,
    }

    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Sample {
        kind: String,
        name: String,
        role: String,
        owner_uuid: String,
        descriptor: String,
        path: String,
    }

    let fixture = fixture_path(&format!(
        "formats/platform_xml/{format}/configuration_physical_module_roles"
    ));
    let manifest: Manifest =
        serde_json::from_str(&read_fixture_descriptor(&fixture.join("manifest.json"))).unwrap();
    assert_eq!(manifest.format, format);
    assert_eq!(manifest.samples.len(), expected_count);
    let source = fixture.join("src");
    let registrations = root_registrations(&source, format);
    let line = export_format_line(format).unwrap();
    let profile = PlatformProfile::for_line(line).unwrap();
    let cancellation = CancellationToken::new();
    let authority = native_authority(&cancellation, &source, profile);
    let root =
        unica_workspace::read::open_source_directory(&source.canonicalize().unwrap()).unwrap();
    let layout = [LayoutFindSource::new(
        "main",
        SourceSetKind::Configuration,
        &root,
    )];
    let finder = WorkspaceFindDirectoryBuilder::default();
    let mut modules = BTreeSet::new();
    for sample in manifest.samples {
        let Sample {
            kind,
            name,
            role,
            owner_uuid,
            descriptor,
            path,
        } = sample;
        let owner = format!("main:{kind}.{name}");
        let at = QualifiedAddress::parse(&format!("{owner}.Module"))
            .unwrap()
            .member(role.as_str())
            .unwrap();
        let address = at.to_string();
        assert!(
            modules.insert(address.clone()),
            "{address}: duplicate sample"
        );
        assert_eq!(
            registrations
                .iter()
                .filter(|(tag, value)| tag == &kind && value == &name)
                .count(),
            1,
            "{address}: native owner registration"
        );
        let descriptor = source.join(descriptor);
        let xml = read_fixture_descriptor(&descriptor);
        let document = parse_fixture_descriptor(&xml, &descriptor);
        assert_eq!(document.root_element().attribute("version"), Some(format));
        let artifact = fixture_artifact(&document, &kind, &descriptor);
        assert_eq!(fixture_properties_name(artifact), Some(name.as_str()));
        assert_eq!(
            uuid::Uuid::parse_str(artifact.attribute("uuid").unwrap()).unwrap(),
            uuid::Uuid::parse_str(&owner_uuid).unwrap()
        );

        let file = source.join(&path);
        let view = read(&authority, &address);
        assert_eq!(view["kind"], "Module", "{address}");
        assert_eq!(view["props"]["ownerKind"], kind.as_str(), "{address}");
        assert_eq!(view["props"]["role"], role.as_str(), "{address}");
        assert_eq!(authority.existing_export_path(&at, None).unwrap(), path);
        let resolved = finder
            .locate_path(
                &layout,
                &path,
                ProviderDeadline::no_deadline(),
                &cancellation,
            )
            .unwrap()
            .unwrap_or_else(|| panic!("{format}: missing native module {path}"));
        assert_eq!(resolved.at(), address);
        assert_eq!(resolved.kind(), "Module");
        assert_eq!(resolved.placed_path(), Some(path.as_str()));
        evidence.record(
            line,
            &owner,
            "Module",
            Some(role.as_str()),
            &fixture_origin(&file),
            AddressEvidence::RoundTrip(&address),
        );
        record_native_physical_module_body(&authority, &address, &file, evidence);
    }
}

fn record_native_physical_module_body(
    authority: &LogicalViewReadAuthority<'_>,
    module: &str,
    file: &Path,
    evidence: &mut EvidenceRows,
) {
    let raw = read_fixture_descriptor(file);
    let source = raw.strip_prefix('\u{feff}').unwrap_or(&raw);
    assert!(!source.trim().is_empty(), "{module}: native module BSL");
    record_native_bsl_body_text(
        authority,
        (
            authority.platform_profile().line(),
            SourceSetKind::Configuration,
        ),
        module,
        file,
        source,
        evidence,
    );
}

struct MissingNativeModules<'a> {
    line: PlatformLine,
    owner: &'a QualifiedAddress,
    descriptor: &'a Path,
    group_roles: Option<&'a [&'a str]>,
    roles: &'a [&'a str],
}

fn check_native_modules_without_bsl(
    authority: &LogicalViewReadAuthority<'_>,
    source_dir: &Path,
    layout: &[LayoutFindSource<'_>],
    cancellation: &CancellationToken,
    sample: MissingNativeModules<'_>,
    evidence: &mut EvidenceRows,
) {
    let MissingNativeModules {
        line,
        owner,
        descriptor,
        group_roles,
        roles,
    } = sample;
    let branch = QualifiedAddress::parse(&format!("{owner}.Module")).unwrap();
    let expected: BTreeSet<_> = if let Some(roles) = group_roles {
        roles
            .iter()
            .map(|role| branch.member(role).unwrap().to_string())
            .collect()
    } else {
        authority
            .platform_profile()
            .module_children(&branch)
            .iter()
            .map(|entry| entry.at().to_string())
            .collect()
    };
    let owner_text = owner.to_string();
    let branch_text = branch.to_string();
    check_native_branch(
        authority,
        &read(authority, &owner_text),
        &branch_text,
        &expected,
    );

    let source = (line, authority.source_set_kind_for_test());
    let origin = fixture_origin(descriptor);
    if group_roles.is_some() {
        evidence.record_for_source(
            source,
            &owner_text,
            "Module",
            None,
            &origin,
            AddressEvidence::Read(&branch_text),
        );
    }
    let finder = WorkspaceFindDirectoryBuilder::default();
    for &role in roles {
        let at = branch.member(role).unwrap();
        let address = at.to_string();
        assert!(
            expected.contains(&address),
            "{address}: expected a logical role"
        );
        assert_eq!(read(authority, &address)["kind"], "Module", "{address}");

        let path = descriptor
            .with_extension("")
            .join(format!("Ext/{role}Module.bsl"));
        let missing = fs::symlink_metadata(&path).unwrap_err();
        assert_eq!(missing.kind(), std::io::ErrorKind::NotFound, "{path:?}");
        let relative = path
            .strip_prefix(source_dir)
            .unwrap()
            .to_str()
            .unwrap()
            .replace('\\', "/");
        assert_eq!(
            authority
                .existing_export_path(&at, None)
                .unwrap_err()
                .code(),
            RefusalCode::NotFound,
            "{address}"
        );
        assert!(
            finder
                .locate_path(
                    layout,
                    &relative,
                    ProviderDeadline::no_deadline(),
                    cancellation
                )
                .unwrap()
                .is_none(),
            "{relative}: a missing BSL must not resolve"
        );
        evidence.record_for_source(
            source,
            &owner_text,
            "Module",
            Some(role),
            &origin,
            AddressEvidence::Read(&address),
        );
    }
}

fn check_native_configuration_modules_without_bsl(
    authority: &LogicalViewReadAuthority<'_>,
    source: &Path,
    layout: &[LayoutFindSource<'_>],
    cancellation: &CancellationToken,
    format: &str,
    registrations: &[(String, String)],
    evidence: &mut EvidenceRows,
) {
    const MANAGER: &[&str] = &["Manager"];
    const MANAGER_OBJECT: &[&str] = &["Manager", "Object"];
    const MANAGER_RECORD_SET: &[&str] = &["Manager", "RecordSet"];
    const MANAGER_VALUE: &[&str] = &["Manager", "ValueManager"];

    let newer = format != "2.20";
    let line = export_format_line(format).unwrap();
    let constant_roles = if newer { MANAGER_VALUE } else { MANAGER };
    let information_register_roles = if newer { MANAGER_RECORD_SET } else { MANAGER };

    // Some groups or roles already have witnesses, such as Document.Manager
    // on 2.20. These samples add only the missing logical evidence.
    let common = [
        ("AccountingRegister", "Бухгалтерский", None, MANAGER),
        ("AccumulationRegister", "Остатки", None, MANAGER),
        ("BusinessProcess", "Approval", None, MANAGER),
        ("CalculationRegister", "Начисления", None, MANAGER),
        ("ChartOfAccounts", "Хозрасчетный", None, MANAGER),
        ("ChartOfCalculationTypes", "ВидыРасчета", None, MANAGER),
        ("ChartOfCharacteristicTypes", "Codes", None, MANAGER),
        ("Constant", "Enabled", Some(MANAGER_VALUE), constant_roles),
        (
            "DataProcessor",
            "Обработка",
            Some(MANAGER_OBJECT),
            MANAGER_OBJECT,
        ),
        ("DocumentJournal", "Entries", Some(MANAGER), MANAGER),
        ("Enum", "LanguageAware", Some(MANAGER), MANAGER),
        ("ExchangePlan", "Codes", None, MANAGER),
        ("FilterCriterion", "Probe", Some(MANAGER), MANAGER),
        (
            "InformationRegister",
            "АдминистративнаяИерархия",
            Some(MANAGER_RECORD_SET),
            information_register_roles,
        ),
        ("SettingsStorage", "Probe", Some(MANAGER), MANAGER),
        ("Task", "Work", None, MANAGER),
    ];
    let later = [
        ("Document", "Orders", None, MANAGER),
        (
            "Report",
            "АнализВерсийОбъектов",
            Some(MANAGER_OBJECT),
            MANAGER_OBJECT,
        ),
    ];
    for (kind, name, group_roles, roles) in common
        .into_iter()
        .chain(newer.then_some(later).into_iter().flatten())
    {
        assert_eq!(
            registrations
                .iter()
                .filter(|(tag, value)| tag == kind && value == name)
                .count(),
            1,
            "{format}: native registration for {kind}.{name}"
        );
        let directory = METADATA_LAYOUTS
            .iter()
            .find(|layout| layout.tag == kind)
            .unwrap()
            .directory;
        let descriptor = source.join(directory).join(format!("{name}.xml"));
        let bytes = read_fixture_descriptor(&descriptor);
        let document = parse_fixture_descriptor(&bytes, &descriptor);
        assert_eq!(document.root_element().attribute("version"), Some(format));
        let artifact = fixture_artifact(&document, kind, &descriptor);
        assert_eq!(fixture_properties_name(artifact), Some(name));
        assert!(uuid::Uuid::parse_str(artifact.attribute("uuid").unwrap()).is_ok());
        let owner = QualifiedAddress::parse(&format!("main:{kind}.{name}")).unwrap();
        check_native_modules_without_bsl(
            authority,
            source,
            layout,
            cancellation,
            MissingNativeModules {
                line,
                owner: &owner,
                descriptor: &descriptor,
                group_roles,
                roles,
            },
            evidence,
        );
    }

    let forbidden = match format {
        "2.20" => Some("main:Document.Приход.Module.RecordSet"),
        "2.22" => Some("main:Queue.Probe.Module.Object"),
        _ => None,
    };
    if let Some(forbidden) = forbidden {
        let at = QualifiedAddress::parse(forbidden).unwrap();
        assert_eq!(
            authority.snapshot(&at).unwrap_err().code(),
            RefusalCode::NotFound
        );
        assert_eq!(
            authority
                .existing_export_path(&at, None)
                .unwrap_err()
                .code(),
            RefusalCode::NotFound
        );
    }
}

fn check_native_bsl_interiors(
    authority: &LogicalViewReadAuthority<'_>,
    source: &Path,
    format: &str,
    registrations: &[(String, String)],
    evidence: &mut EvidenceRows,
) {
    // Each span is transcribed from these short native files: declaration,
    // first Body line (including attached comment), last Body line.
    for (kind, name, directory, role, file, methods, region) in [
        (
            "HTTPService",
            "ПроверкаСвязи",
            "HTTPServices",
            "HTTPService",
            "Module.bsl",
            &[("ПингGET", 1, 1, 5), ("ПринятьДанныеPOST", 7, 7, 13)][..],
            None,
        ),
        (
            "WebService",
            "ОбменДанными",
            "WebServices",
            "WebService",
            "Module.bsl",
            &[("Пинг", 1, 1, 3), ("ПринятьПакет", 5, 5, 11)][..],
            None,
        ),
        (
            "Catalog",
            "Валюты",
            "Catalogs",
            "Manager",
            "ManagerModule.bsl",
            &[("ВалютаПоКоду", 13, 5, 17)][..],
            Some(("ПрограммныйИнтерфейс", 3, 19)),
        ),
        (
            "Catalog",
            "Валюты",
            "Catalogs",
            "Object",
            "ObjectModule.bsl",
            &[("ПередЗаписью", 3, 3, 13)][..],
            Some(("ОбработчикиСобытий", 1, 15)),
        ),
    ] {
        assert_eq!(
            registrations
                .iter()
                .filter(|(tag, value)| tag == kind && value == name)
                .count(),
            1,
            "{format}: {kind}.{name} registration",
        );
        let descriptor = source.join(directory).join(format!("{name}.xml"));
        let xml = read_fixture_descriptor(&descriptor);
        let document = parse_fixture_descriptor(&xml, &descriptor);
        assert_eq!(document.root_element().attribute("version"), Some(format));
        let artifact = fixture_artifact(&document, kind, &descriptor);
        assert_eq!(fixture_properties_name(artifact), Some(name));
        assert!(uuid::Uuid::parse_str(artifact.attribute("uuid").unwrap()).is_ok());
        let relative = format!("{directory}/{name}/Ext/{file}");
        let module = format!("main:{kind}.{name}.Module.{role}");
        let at = QualifiedAddress::parse(&module).unwrap();
        assert_eq!(authority.existing_export_path(&at, None).unwrap(), relative);
        let path = source.join(relative);
        let raw = fs::read_to_string(&path).unwrap();
        let raw = raw.trim_start_matches('\u{feff}');
        let lines: Vec<_> = raw.split_inclusive('\n').collect();
        let view = read(authority, &module);
        assert_eq!(view["kind"], "Module");
        let origin = fixture_origin(&path);
        let line = authority.platform_profile().line();
        let body = format!("{module}.Body");
        check_native_branch_count(&view, &body, lines.len());
        check_native_bsl_body(authority, &body, raw, 1, lines.len());
        evidence.record(
            line,
            &module,
            "Body",
            None,
            &origin,
            AddressEvidence::Read(&body),
        );

        let method_branch = QualifiedAddress::parse(&format!("{module}.Method")).unwrap();
        let expected: BTreeSet<_> = methods
            .iter()
            .map(|(name, _, _, _)| method_branch.member(name).unwrap().to_string())
            .collect();
        assert_eq!(expected.len(), methods.len());
        check_native_branch(authority, &view, &method_branch.to_string(), &expected);
        evidence.record(
            line,
            &module,
            "Method",
            None,
            &origin,
            AddressEvidence::Read(&method_branch.to_string()),
        );
        for &(name, declaration, first, last) in methods {
            let (kind, signature) = lines[declaration - 1].trim().split_once(' ').unwrap();
            assert!(matches!(kind, "Функция" | "Процедура"));
            assert_eq!(signature.split_once('(').unwrap().0, name);
            assert!(matches!(
                lines[last - 1].trim(),
                "КонецФункции" | "КонецПроцедуры"
            ));
            assert!(lines[first - 1..declaration - 1]
                .iter()
                .all(|line| line.trim_start().starts_with("//")));

            let method = method_branch.member(name).unwrap().to_string();
            let method_view = read(authority, &method);
            assert_eq!(method_view["kind"], "Method");
            assert_eq!(method_view["props"]["declarationLine"], declaration);
            assert_eq!(method_view["props"]["line"], first);
            assert_eq!(method_view["props"]["endLine"], last);
            evidence.record(
                line,
                &module,
                "Method",
                None,
                &origin,
                AddressEvidence::Read(&method),
            );
            let body = format!("{method}.Body");
            check_native_branch_count(&method_view, &body, last + 1 - first);
            check_native_bsl_body(
                authority,
                &body,
                &lines[first - 1..last].concat(),
                first,
                last,
            );
            evidence.record(
                line,
                &method,
                "Body",
                None,
                &origin,
                AddressEvidence::Read(&body),
            );
        }

        let region_branch = format!("{module}.Region");
        if let Some((name, first, last)) = region {
            assert_eq!(lines[first - 1].trim(), format!("#Область {name}"));
            assert_eq!(lines[last - 1].trim(), "#КонецОбласти");
            let region_at = QualifiedAddress::parse(&region_branch)
                .unwrap()
                .member(name)
                .unwrap()
                .to_string();
            check_native_branch(
                authority,
                &view,
                &region_branch,
                &BTreeSet::from([region_at.clone()]),
            );
            let region_view = read(authority, &region_at);
            assert_eq!(region_view["kind"], "Region");
            assert_eq!(region_view["props"]["line"], first);
            assert_eq!(region_view["props"]["endLine"], last);
            assert_eq!(addresses(&region_view, "items"), expected);
            evidence.record(
                line,
                &module,
                "Region",
                None,
                &origin,
                AddressEvidence::Read(&region_at),
            );
        } else {
            assert!(!lines
                .iter()
                .any(|line| line.trim_start().starts_with("#Область")));
            check_native_branch(authority, &view, &region_branch, &BTreeSet::new());
        }
        evidence.record(
            line,
            &module,
            "Region",
            None,
            &origin,
            AddressEvidence::Read(&region_branch),
        );
    }
}

fn record_native_bsl_body(
    authority: &LogicalViewReadAuthority<'_>,
    source: (PlatformLine, SourceSetKind),
    module: &str,
    file: &Path,
    evidence: &mut EvidenceRows,
) {
    let raw = read_fixture_descriptor(file);
    let text = raw.trim_start_matches('\u{feff}');
    record_native_bsl_body_text(authority, source, module, file, text, evidence);
}

fn record_native_bsl_body_text(
    authority: &LogicalViewReadAuthority<'_>,
    source: (PlatformLine, SourceSetKind),
    module: &str,
    file: &Path,
    text: &str,
    evidence: &mut EvidenceRows,
) {
    let at = format!("{module}.Body");
    // An empty source still has a Body text window at line 1.
    check_native_bsl_body(authority, &at, text, 1, text.lines().count().max(1));
    evidence.record_for_source(
        source,
        module,
        "Body",
        None,
        &fixture_origin(file),
        AddressEvidence::Read(&at),
    );
}

fn check_native_bsl_body(
    authority: &LogicalViewReadAuthority<'_>,
    body: &str,
    expected: &str,
    first: usize,
    last: usize,
) {
    use crate::domain::node_view::SourceText;

    let at = QualifiedAddress::parse(body).unwrap();
    assert_eq!(at.to_string(), body);
    let snapshot = authority.snapshot(&at).unwrap();
    let mut view = authority.read_exact(&at, &snapshot).unwrap();
    assert_eq!(view.at(), body);
    assert_eq!(serde_json::to_value(&view).unwrap()["kind"], "Body");
    let (text, _) = view.take_text().expect("native BSL Body text");
    assert_eq!(
        text,
        SourceText::Window {
            text: Box::new(SourceText::Whole(expected.to_owned())),
            from: first,
            to: last,
        },
        "{body}: exact source text and absolute line span",
    );
}

fn check_native_named_module_samples(
    authority: &LogicalViewReadAuthority<'_>,
    profile_fixture: &ModuleCapabilityFixture,
    source: &Path,
    format: &str,
    registrations: &[(String, String)],
    evidence: &mut EvidenceRows,
) {
    use crate::application::v13::source_text::ModuleText;

    let profile = authority.platform_profile();
    for (kind, name, directory, role, terminal) in [
        (
            "CommonModule",
            "EventHandlers",
            "CommonModules",
            "Common",
            "Module.bsl",
        ),
        (
            "CommonForm",
            "MainA",
            "CommonForms",
            "Form",
            "Form/Module.bsl",
        ),
        (
            "CommonCommand",
            "Probe",
            "CommonCommands",
            "Command",
            "CommandModule.bsl",
        ),
        (
            "IntegrationService",
            "Probe",
            "IntegrationServices",
            "IntegrationService",
            "Module.bsl",
        ),
    ] {
        assert_eq!(
            registrations
                .iter()
                .filter(|(tag, value)| tag == kind && value == name)
                .count(),
            1,
            "{kind}.{name}: native configuration registration",
        );
        let descriptor = source.join(directory).join(format!("{name}.xml"));
        let raw = read_fixture_descriptor(&descriptor);
        let document = parse_fixture_descriptor(&raw, &descriptor);
        assert_eq!(document.root_element().attribute("version"), Some(format));
        let artifact = fixture_artifact(&document, kind, &descriptor);
        assert_eq!(fixture_properties_name(artifact), Some(name));
        assert!(uuid::Uuid::parse_str(artifact.attribute("uuid").unwrap()).is_ok());
        let owner = format!("main:{kind}.{name}");
        let relative = format!("{directory}/{name}/Ext/{terminal}");
        let module = source.join(&relative);
        if role == "Common" {
            assert!(
                profile_fixture.module_capabilities.iter().any(|case| {
                    if !case.exists || case.role.as_deref() != Some(role) {
                        return false;
                    }
                    let sample = QualifiedAddress::parse(&case.at).unwrap();
                    matches!(sample.segments(), [segment]
                    if sample.source_set() == "main"
                        && segment.kind() == NodeKind::CommonModule
                        && segment.name().is_some())
                }),
                "profile fixture has no bare CommonModule capability"
            );
            let at = QualifiedAddress::parse(&owner).unwrap();
            assert_eq!(
                profile.module_capability(&at).unwrap().role().as_str(),
                role
            );
            assert_eq!(read(authority, &owner)["kind"], "Module");
            let raw = fs::read_to_string(&module).unwrap();
            let expected = raw.trim_start_matches('\u{feff}');
            assert!(!expected.is_empty());
            let body = QualifiedAddress::parse(&format!("{owner}.Body")).unwrap();
            if format == "2.20" {
                let snapshot = authority.snapshot(&body).unwrap();
                assert_eq!(
                    authority.read_module_text(&body, &snapshot).unwrap(),
                    Some(ModuleText::whole(expected.to_owned())),
                );
                evidence.record(
                    profile.line(),
                    &owner,
                    "Module",
                    Some(role),
                    &fixture_origin(&module),
                    AddressEvidence::Read(&owner),
                );
                println!(
                    "{} {owner}: read on the bare CommonModule address",
                    profile.id()
                );
            } else {
                // Resolve at the bare owner selects its descriptor. Body selects the BSL.
                assert_eq!(
                    authority.existing_export_path(&body, None).unwrap(),
                    relative
                );
                let body_at = body.to_string();
                assert_eq!(expected.lines().count(), 8);
                check_native_bsl_body(authority, &body_at, expected, 1, 8);
                evidence.record(
                    profile.line(),
                    &owner,
                    "Body",
                    None,
                    &fixture_origin(&module),
                    AddressEvidence::Read(&body_at),
                );
            }
        } else {
            let at = check_registered_module_read(authority, profile_fixture, &owner, role);
            evidence.record(
                profile.line(),
                &owner,
                "Module",
                None,
                &fixture_origin(&descriptor),
                AddressEvidence::Read(&format!("{owner}.Module")),
            );
            if kind == "CommonForm" {
                assert!(module.is_file(), "{module:?}");
                assert_eq!(authority.existing_export_path(&at, None).unwrap(), relative);
                continue;
            }
            let missing = fs::symlink_metadata(&module).unwrap_err();
            assert_eq!(missing.kind(), std::io::ErrorKind::NotFound, "{module:?}");
            assert_eq!(
                authority
                    .existing_export_path(&at, None)
                    .unwrap_err()
                    .code(),
                RefusalCode::NotFound,
                "{at}: absent native BSL",
            );
            if kind == "IntegrationService" && format == "2.20" {
                evidence.record(
                    profile.line(),
                    &owner,
                    "Module",
                    Some(role),
                    &fixture_origin(&descriptor),
                    AddressEvidence::Read(&at.to_string()),
                );
                println!(
                    "{} {at}: read from registered owner; native BSL absent",
                    profile.id()
                );
            }
        }
    }
}

fn native_command_owners<'a, 'input: 'a>(
    owner: &str,
    descriptor: roxmltree::Node<'a, 'input>,
) -> BTreeMap<&'a str, String> {
    let collection = QualifiedAddress::parse(&format!("{owner}.Command")).unwrap();
    let mut result = BTreeMap::new();
    let mut ids = BTreeSet::new();
    for command in native_child_objects(descriptor)
        .filter(|node| node.has_tag_name((MD_CLASSES_NS, "Command")))
    {
        let name = fixture_properties_name(command).expect("native command Properties.Name");
        let id = uuid::Uuid::parse_str(command.attribute("uuid").unwrap()).unwrap();
        assert!(ids.insert(id), "{owner}: duplicate native Command UUID");
        let at = collection.member(name).unwrap().to_string();
        assert!(
            result.insert(name, at).is_none(),
            "{owner}: duplicate Command {name}"
        );
    }
    result
}

fn check_registered_module_read(
    authority: &LogicalViewReadAuthority<'_>,
    profile_fixture: &ModuleCapabilityFixture,
    owner: &str,
    role: &str,
) -> QualifiedAddress {
    let owner_address = QualifiedAddress::parse(owner).unwrap();
    let branch = QualifiedAddress::parse(&format!("{owner}.Module")).unwrap();
    let mut expected = BTreeSet::new();
    for case in profile_fixture
        .module_capabilities
        .iter()
        .filter(|case| case.exists)
    {
        let at = QualifiedAddress::parse(&case.at).unwrap();
        let (module, case_owner) = at.segments().split_last().unwrap();
        if at.source_set() == branch.source_set()
            && module.kind() == NodeKind::Module
            && case_owner
                .iter()
                .map(|part| part.kind())
                .eq(owner_address.segments().iter().map(|part| part.kind()))
        {
            let expected_role = case.role.as_deref().expect("recorded module role");
            assert_eq!(module.name(), Some(expected_role));
            expected.insert(branch.member(expected_role).unwrap().to_string());
        }
    }
    let at = branch.member(role).unwrap();
    let kind = owner_address.segments()[0].kind();
    let named_module = matches!(
        kind,
        NodeKind::Bot
            | NodeKind::CommonCommand
            | NodeKind::CommonForm
            | NodeKind::HttpService
            | NodeKind::IntegrationService
            | NodeKind::WebService
            | NodeKind::WebSocketClient
    );
    if named_module {
        assert_eq!(expected, BTreeSet::from([at.to_string()]), "{owner}");
    } else {
        assert!(
            expected.contains(&at.to_string()),
            "profile fixture does not admit {at}"
        );
    }
    let parent = read(authority, owner);
    if kind == NodeKind::CommonForm {
        assert_eq!(parent["kind"], "Form", "{owner}");
    }
    let group = check_native_branch(authority, &parent, &branch.to_string(), &expected);
    if named_module {
        assert_eq!(group["kind"], "Module", "{branch}");
        let [item] = group["items"].as_array().unwrap().as_slice() else {
            panic!("{branch}: expected one module member");
        };
        assert_eq!(item["kind"], "Module", "{at}");
        assert_eq!(item["props"]["ownerKind"], kind.as_str(), "{at}");
        assert_eq!(item["props"]["role"], role, "{at}");
    }
    assert_eq!(read(authority, &at.to_string())["kind"], "Module", "{at}");
    at
}

/// rule: CTR.SOURCE.QUALIFIED-LOGICAL-ADDRESS
#[test]
fn native_websocket_modules_have_a_readable_address_and_forward_layout() {
    let profile_fixture = module_capability_fixture();
    for (xml, line) in [
        ("2.20", PlatformLine::new(8, 3, 27)),
        ("2.21", PlatformLine::new(8, 5, 1)),
        ("2.22", PlatformLine::new(8, 5, 4)),
    ] {
        let source = fixture_path(&format!("formats/platform_xml/{xml}/all_kinds/src"));
        let profile = PlatformProfile::for_line(line).unwrap();
        let cancellation = CancellationToken::new();
        let authority = native_authority(&cancellation, &source, profile);
        let owner = "main:WebSocketClient.AllKindsWebSocketClient";
        let at =
            check_registered_module_read(&authority, &profile_fixture, owner, "WebSocketClient");
        let path = authority.existing_export_path(&at, None).unwrap();
        assert_eq!(
            path,
            "WebSocketClients/AllKindsWebSocketClient/Ext/Module.bsl"
        );
        assert!(source.join(&path).is_file());
        println!("| {line} | WebSocketClient | Module.WebSocketClient | {at} | native file / forward layout |");
    }
}
