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
use unica_work_control::provider_deadline::ProviderDeadline;

// One sorted JSONL file per successful test, when explicitly requested. Use
// an empty output directory per exact run; the caller verifies the test result.
const EVIDENCE_DIRECTORY: &str = "UNICA_484_ADDRESS_JSONL_DIR";

enum AddressEvidence<'a> {
    Read(&'a str),
    RoundTrip(&'a str),
    IntroducedIn(PlatformLine),
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
        let (line, source_kind) = source;
        let owner_address = QualifiedAddress::parse(owner).expect("witness owner address");
        assert_eq!(owner_address.to_string(), owner);
        let owner_shape: Vec<_> = owner_address
            .segments()
            .iter()
            .map(|segment| {
                serde_json::json!({
                    "kind": segment.kind().as_str(),
                    "named": segment.name().is_some(),
                    "role": (segment.kind() == NodeKind::Interface).then_some(segment.name()).flatten(),
                })
            })
            .collect();
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
                "key": {
                    "line": line.to_string(),
                    "source_kind": source_kind,
                    "owner_shape": owner_shape,
                    "branch": branch,
                    "role": role,
                },
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
            read(&authority, &at);
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

/// rule: INV.SOURCE.EXTERNAL-ARTIFACT-ROOTS
/// rule: INV.SOURCE.LOGICAL-BRANCH-COUNT
/// rule: INV.SOURCE.RESOLVED-RESOURCE-IDENTITY
#[test]
fn native_external_root_descriptors_match_published_addresses() {
    use unica_workspace::source_profile::platform_xml_source_profile;

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
    }
    evidence.finish();
}

/// rule: CTR.SOURCE.QUALIFIED-LOGICAL-ADDRESS
/// rule: INV.SOURCE.LOGICAL-BRANCH-COUNT
/// rule: INV.SOURCE.PLATFORM-CAPABILITY-EXISTENCE
#[test]
fn native_recalculation_module_address_table_matches_registered_owner() {
    let source = fixture_path("formats/platform_xml/2.20/recalculation/src");
    let profile = PlatformProfile::v8_3_27();
    let cancellation = CancellationToken::new();
    let authority = native_authority(&cancellation, &source, profile);
    let mut evidence = EvidenceRows::new(
        "native_recalculation_module_address_table_matches_registered_owner",
        SourceSetKind::Configuration,
    );
    let register = "main:CalculationRegister.Начисления";
    let parent_branch = format!("{register}.Recalculation");
    let descriptor = source.join("CalculationRegisters/Начисления.xml");
    let xml = read_fixture_descriptor(&descriptor);
    let document = parse_fixture_descriptor(&xml, &descriptor);
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
        let branch = format!("{owner}.Module");
        assert!(addresses(&node, "branches").contains(&branch));
        let branch_at = QualifiedAddress::parse(&branch).unwrap();
        let roles = profile.module_children(&branch_at);
        let published = addresses(&read(&authority, &branch), "items");
        assert_eq!(
            published,
            roles
                .iter()
                .map(|child| child.at().to_string())
                .collect::<BTreeSet<_>>()
        );
        assert_eq!(published, BTreeSet::from([format!("{branch}.RecordSet")]));
        for role in roles {
            let at = role.at();
            let route = route_logical_address(at, profile).unwrap();
            assert_eq!(route.reader(), LogicalReader::Module);
            let node = read(&authority, &at.to_string());
            assert!(
                addresses(&node, "branches").contains(&format!("{at}.Body")),
                "{node:#}"
            );
            println!(
                "| {} | Recalculation | Module.{} | {at} |",
                profile.line(),
                role.capability().role().as_str()
            );
        }
    }
    evidence.finish();
}

/// rule: CTR.SOURCE.QUALIFIED-LOGICAL-ADDRESS
#[test]
fn native_metadata_address_table_matches_declared_and_observed_child_branches() {
    let mut evidence = EvidenceRows::new(
        "native_metadata_address_table_matches_declared_and_observed_child_branches",
        SourceSetKind::Configuration,
    );
    println!("| Profile | Owner | Branch | Canonical address | Evidence |");
    println!("| --- | --- | --- | --- | --- |");
    for (xml, line) in [
        ("2.20", PlatformLine::new(8, 3, 27)),
        ("2.21", PlatformLine::new(8, 5, 1)),
        ("2.22", PlatformLine::new(8, 5, 4)),
    ] {
        let source = fixture_path(&format!("formats/platform_xml/{xml}/all_kinds/src"));
        let profile = PlatformProfile::for_line(line).unwrap();
        let cancellation = CancellationToken::new();
        let authority = native_authority(&cancellation, &source, profile);
        let mut verified = BTreeSet::new();
        for (kind, name) in root_registrations(&source, xml) {
            let at = QualifiedAddress::parse(&format!("main:{kind}.{name}")).unwrap();
            let route = route_logical_address(&at, profile).unwrap();
            if route.reader() == LogicalReader::Metadata {
                check_metadata_owner(&authority, &source, &at, &mut verified, &mut evidence);
            }
        }
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
        check_eds_module_rows(&authority, &source, &cancellation);
    }
    evidence.finish();
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
                    &at,
                    kind,
                    name,
                    xml,
                    &descriptor,
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
                        &at,
                        child_kind,
                        child_name,
                        xml,
                        &descriptor,
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
    evidence.finish();
}

#[allow(clippy::too_many_arguments)]
fn check_native_form_or_template(
    authority: &LogicalViewReadAuthority<'_>,
    at: &QualifiedAddress,
    kind: &str,
    name: &str,
    format: &str,
    descriptor: &Path,
    verify_path: &impl Fn(&QualifiedAddress, &Path),
    covered: &mut BTreeSet<String>,
    evidence: &mut EvidenceRows,
) {
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
            covered.insert(module_at);
        }
        "Managed Form"
    } else {
        const DCS_NS: &str = "http://v8.1c.ru/8.1/data-composition-system/schema";
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

fn check_eds_module_rows(
    authority: &LogicalViewReadAuthority<'_>,
    source: &Path,
    cancellation: &CancellationToken,
) {
    let root_branch = "main:ExternalDataSource";
    let owners = addresses(&read(authority, root_branch), "items");
    let owner = owners
        .get("main:ExternalDataSource.AllKindsExternalSource")
        .unwrap();
    let branches = addresses(&read(authority, owner), "branches");
    let root =
        unica_workspace::read::open_source_directory(&source.canonicalize().unwrap()).unwrap();
    let find_sources = [LayoutFindSource::new(
        "main",
        SourceSetKind::Configuration,
        &root,
    )];
    let finder = WorkspaceFindDirectoryBuilder::default();
    for (branch, name, role, native_path) in [
        (
            "Table",
            "Items",
            "Manager",
            "ExternalDataSources/AllKindsExternalSource/Tables/Items/Ext/ManagerModule.bsl",
        ),
        (
            "Cube",
            "Sales",
            "RecordSet",
            "ExternalDataSources/AllKindsExternalSource/Cubes/Sales/Ext/RecordSetModule.bsl",
        ),
    ] {
        let collection = format!("{owner}.{branch}");
        assert!(branches.contains(&collection));
        let items = addresses(&read(authority, &collection), "items");
        let item = items.get(&format!("{collection}.{name}")).unwrap();
        let modules = format!("{item}.Module");
        assert!(addresses(&read(authority, item), "branches").contains(&modules));
        let items = addresses(&read(authority, &modules), "items");
        let module = items.get(&format!("{modules}.{role}")).unwrap();
        read(authority, &format!("{module}.Body"));

        let at = QualifiedAddress::parse(module).unwrap();
        let path = authority.existing_export_path(&at, None).unwrap();
        assert_eq!(path, native_path);
        let document = finder
            .locate_path(
                &find_sources,
                &path,
                ProviderDeadline::no_deadline(),
                cancellation,
            )
            .unwrap()
            .unwrap();
        assert_eq!(document.at(), module.as_str());
        println!(
            "| {} | {item} | Module.{role} | {module} | native owner and {branch}; file round-trip |",
            authority.platform_profile().id()
        );
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
) {
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
    let actual = addresses(&read(authority, branch), "items");
    assert_eq!(&actual, expected, "published child addresses for {branch}");
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
#[test]
fn native_websocket_modules_have_a_readable_address_and_forward_layout() {
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
        let branch = format!("{owner}.Module");
        assert!(addresses(&read(&authority, owner), "branches").contains(&branch));
        let items = addresses(&read(&authority, &branch), "items");
        let at = format!("{branch}.WebSocketClient");
        assert!(items.contains(&at));
        read(&authority, &at);
        let path = authority
            .existing_export_path(&QualifiedAddress::parse(&at).unwrap(), None)
            .unwrap();
        assert_eq!(
            path,
            "WebSocketClients/AllKindsWebSocketClient/Ext/Module.bsl"
        );
        assert!(source.join(&path).is_file());
        println!("| {line} | WebSocketClient | Module.WebSocketClient | {at} | native file / forward layout |");
    }
}
