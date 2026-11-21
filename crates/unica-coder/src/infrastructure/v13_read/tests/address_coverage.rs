use super::*;
use serde_json::Value;
use std::collections::BTreeSet;
use unica_address::address::NodeKind;
use unica_source_contract::format_identity::PlatformLine;
use unica_source_designer::owner::MD_CLASSES_NS;

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

fn registration_rows(source: &Path) -> BTreeSet<String> {
    let xml = fs::read_to_string(source.join("Configuration.xml")).unwrap();
    let document = roxmltree::Document::parse(xml.trim_start_matches('\u{feff}')).unwrap();
    let root = document.root_element();
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
            let kind = node.tag_name().name();
            format!("main:{kind}")
        })
        .collect()
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
#[test]
fn native_configuration_root_address_table_matches_published_branches() {
    println!("| Profile | Kind | Branch | Address or reason |");
    println!("| --- | --- | --- | --- |");
    for (xml, line) in [
        ("2.20", PlatformLine::new(8, 3, 27)),
        ("2.21", PlatformLine::new(8, 5, 1)),
        ("2.22", PlatformLine::new(8, 5, 4)),
    ] {
        let source = fixture_path(&format!("formats/platform_xml/{xml}/all_kinds/src"));
        let profile = PlatformProfile::for_line(line).unwrap();
        let cancellation = CancellationToken::new();
        let authority = native_authority(&cancellation, &source, profile);

        // The fixture's independent platform export describes one registration
        // for every collection kind on this line, with some extra members.
        let mut table = registration_rows(&source);
        assert!(table.insert("main:Module".into()));
        let published = read(&authority, "main:Configuration");
        assert_eq!(addresses(&published, "branches"), table, "XML {xml}");
        for at in &table {
            read(&authority, at);
            let branch = QualifiedAddress::parse(at).unwrap();
            let kind = branch.segments().last().unwrap().kind();
            println!("| {line} | Configuration | {} | {at} |", kind.as_str());
        }

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
            println!(
                "| {line} | Configuration | Module.{} | {at} |",
                role.as_str()
            );
        }
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

#[test]
fn pending_recalculation_module_has_no_address_in_the_profile() {
    let recalculation = "main:CalculationRegister.Payroll.Recalculation.Probe";
    let owner = QualifiedAddress::parse(recalculation).unwrap();
    assert_eq!(owner.segments()[1].kind(), NodeKind::Recalculation);
    let prospective = format!("{recalculation}.Module.RecordSet");
    for (line, _) in unica_source_contract::format_identity::DESCRIBED_PLATFORM_LINES {
        let profile = PlatformProfile::for_line(*line).unwrap();
        if let Ok(address) = QualifiedAddress::parse(&prospective) {
            assert!(
                profile.module_capability(&address).is_none(),
                "{line}: {prospective}"
            );
            assert!(
                route_logical_address(&address, profile).is_err(),
                "{line}: {prospective}"
            );
        }
        println!(
            "| {line} | Recalculation | Module.RecordSet | No canonical route. The XML generated type RecalculationRecordSet does not prove a BSL module. A native export of a registered recalculation with nonempty BSL is required. |"
        );
    }
}
