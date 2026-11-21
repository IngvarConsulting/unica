use super::*;
use crate::infrastructure::logical_tree::LogicalReader;
use crate::infrastructure::v13_find::{LayoutFindSource, WorkspaceFindDirectoryBuilder};
use crate::infrastructure::v13_read_projection::metadata_address_coverage::{
    item_facts, owner_facts, BranchFact,
};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use unica_address::address::NodeKind;
use unica_address::kinds::DECLARED_ROOT_KINDS;
use unica_source_contract::format_identity::{
    export_format_line, metadata_kind_exists, metadata_kind_since, PlatformLine,
    DESCRIBED_PLATFORM_LINES,
};
use unica_source_designer::owner::MD_CLASSES_NS;
use unica_work_control::provider_deadline::ProviderDeadline;

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
/// rule: INV.SOURCE.PROFILE-CAPABILITIES
#[test]
fn native_configuration_root_address_table_matches_published_branches() {
    let expected_keys: BTreeSet<_> = DESCRIBED_PLATFORM_LINES
        .iter()
        .flat_map(|(line, _)| DECLARED_ROOT_KINDS.map(|kind| (*line, "Configuration", kind)))
        .collect();
    let mut rows = BTreeMap::new();
    let mut modules = Vec::new();
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
            assert!(rows
                .insert((line, "Configuration", kind), RootAddress::Canonical(at))
                .is_none());
        }
        for kind in unavailable {
            let since = metadata_kind_since(kind)
                .filter(|since| line < *since)
                .expect("a missing root kind must have a later introduction line");
            assert!(rows
                .insert(
                    (line, "Configuration", kind),
                    RootAddress::IntroducedIn(since)
                )
                .is_none());
        }

        modules.push((line, "Module".to_string(), "main:Module".to_string()));
        let branch = QualifiedAddress::parse("main:Module").unwrap();
        let role_addresses: BTreeSet<_> = profile
            .module_children(&branch)
            .into_iter()
            .map(|child| child.at().to_string())
            .collect();
        assert!(!role_addresses.is_empty());
        assert_eq!(
            addresses(&read(&authority, "main:Module"), "items"),
            role_addresses
        );
        for at in &role_addresses {
            let address = QualifiedAddress::parse(at).unwrap();
            assert_eq!(address.to_string(), *at);
            let route = route_logical_address(&address, profile).unwrap();
            assert_eq!(
                route.reader(),
                crate::infrastructure::logical_tree::LogicalReader::Module
            );
            let role = profile.module_capability(&address).unwrap().role();
            modules.push((line, format!("Module.{}", role.as_str()), at.clone()));
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
}

/// rule: CTR.SOURCE.QUALIFIED-LOGICAL-ADDRESS
#[test]
fn native_ordinary_interface_has_a_registration_only_root_branch() {
    let source = fixture_path("acceptance/workspace-ordinary-interface/src");
    let profile = PlatformProfile::v8_3_27();
    let cancellation = CancellationToken::new();
    let authority = native_authority(&cancellation, &source, profile);
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
    println!("| 8.3.27 | Configuration | OrdinaryInterface | {at} |");
    for item in items {
        let address = QualifiedAddress::parse(&item).unwrap();
        assert_eq!(address.segments()[0].kind(), NodeKind::OrdinaryInterface);
        read(&authority, &item);
        println!("| 8.3.27 | OrdinaryInterface | Member | {item} |");
    }
}

/// rule: INV.SOURCE.PLATFORM-CAPABILITY-EXISTENCE
#[test]
fn native_recalculation_module_address_table_matches_registered_owner() {
    let source = fixture_path("formats/platform_xml/2.20/recalculation/src");
    let profile = PlatformProfile::v8_3_27();
    let cancellation = CancellationToken::new();
    let authority = native_authority(&cancellation, &source, profile);
    let parent_branch = "main:CalculationRegister.Начисления.Recalculation";
    let owners = addresses(&read(&authority, parent_branch), "items");
    assert_eq!(
        owners,
        BTreeSet::from([format!("{parent_branch}.Перерасчет1")])
    );
    for owner in owners {
        let branch = format!("{owner}.Module");
        assert!(addresses(&read(&authority, &owner), "branches").contains(&branch));
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
}

/// rule: CTR.SOURCE.QUALIFIED-LOGICAL-ADDRESS
#[test]
fn native_metadata_address_table_matches_declared_and_observed_child_branches() {
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
        for (kind, name) in root_registrations(&source, xml) {
            let at = QualifiedAddress::parse(&format!("main:{kind}.{name}")).unwrap();
            let route = route_logical_address(&at, profile).unwrap();
            if route.reader() == LogicalReader::Metadata {
                check_metadata_owner(&authority, &at);
            }
        }
        check_eds_module_rows(&authority, &source, &cancellation);
    }
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

fn check_metadata_owner(authority: &LogicalViewReadAuthority<'_>, owner: &QualifiedAddress) {
    let profile = authority.platform_profile();
    let route = route_logical_address(owner, profile).unwrap();
    let admitted = authority.snapshot(owner).unwrap();
    let payload = authority
        .typed_payload_for(&route, &admitted)
        .unwrap_or_else(|error| panic!("{owner}: {error:?}"));
    let published = read(authority, &owner.to_string());
    let (facts, addressing_issue) = owner_facts(owner, &payload);
    let target = route.reader_metadata_path().unwrap();
    let descriptor = authority.read.metadata_descriptor(target).unwrap();
    check_native_child_registrations(owner, &descriptor, &facts);
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
    check_metadata_child_branches(authority, owner, &published, &facts);
}

fn check_native_child_registrations(
    owner: &QualifiedAddress,
    descriptor: &[u8],
    facts: &[BranchFact<'_>],
) {
    let xml = std::str::from_utf8(descriptor)
        .unwrap()
        .trim_start_matches('\u{feff}');
    let doc = roxmltree::Document::parse(xml).unwrap();
    let kind = owner.segments().last().unwrap().kind().as_str();
    let artifact = doc
        .root_element()
        .children()
        .find(|node| node.has_tag_name((MD_CLASSES_NS, kind)))
        .unwrap_or_else(|| panic!("{owner}: native artifact is missing"));

    // A registered child in the native XML is an independent witness that a
    // typed reader must generate its branch even if that reader drops a fact.
    let mut counts = BTreeMap::new();
    for children in artifact
        .children()
        .filter(|node| node.has_tag_name((MD_CLASSES_NS, "ChildObjects")))
    {
        for child in children.children().filter(roxmltree::Node::is_element) {
            assert_eq!(child.tag_name().namespace(), Some(MD_CLASSES_NS));
            *counts.entry(child.tag_name().name()).or_insert(0) += 1;
        }
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
    owner: &QualifiedAddress,
    published: &Value,
    facts: &[BranchFact<'_>],
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
        let evidence =
            if fact.kind == NodeKind::Schedule && collection["props"]["state"] == "absent" {
                "declared; native Schedule.xml absent"
            } else if fact.count == 0 {
                "declared; no member in this export"
            } else {
                "observed"
            };
        println!(
            "| {} | {owner} | {} | {at} | {evidence} |",
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
                check_metadata_owner(authority, &item);
                continue;
            }
            let branches = item_facts(member);
            let view = read(authority, &item_at);
            assert_eq!(
                published_metadata_branches(&item, row, profile),
                published_metadata_branches(&item, &view, profile),
                "{item}: collection row and direct view disagree"
            );
            check_metadata_child_branches(authority, &item, &view, &branches);
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
