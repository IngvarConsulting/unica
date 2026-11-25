use super::*;
use std::io::Write;

pub(super) fn record_cross_line_module_roles() {
    let rows = cross_line_module_roles();
    if let Some(directory) = std::env::var_os(EVIDENCE_DIRECTORY) {
        let path = PathBuf::from(directory).join("required_cross_line_module_roles.jsonl");
        let mut file = fs::File::create_new(path).unwrap();
        for row in rows {
            writeln!(file, "{row}").unwrap();
        }
    }
}

fn cross_line_module_roles() -> Vec<Value> {
    let mut rows = Vec::new();
    for version in ["2.20", "2.21", "2.22"] {
        let line = export_format_line(version).unwrap();
        let profile = PlatformProfile::for_line(line).unwrap();
        let source = fixture_path(&format!("formats/platform_xml/{version}/all_kinds/src"));
        let registrations = root_registrations(&source, version);
        for (kind, name, directory, role, terminal) in [
            (
                "CommonModule",
                "EventHandlers",
                "CommonModules",
                "Common",
                "Module.bsl",
            ),
            ("Queue", "Probe", "Queues", "Manager", "ManagerModule.bsl"),
        ] {
            let registered = registrations
                .iter()
                .filter(|(registered_kind, registered_name)| {
                    registered_kind == kind && registered_name == name
                })
                .count();
            let admitted = metadata_kind_exists(Some(line), kind);
            assert_eq!(registered, usize::from(admitted), "{line}: {kind}.{name}");
            if !admitted {
                continue;
            }
            let owner = QualifiedAddress::parse(&format!("main:{kind}.{name}")).unwrap();
            let (address, level) = if kind == "CommonModule" {
                (owner.clone(), "read")
            } else {
                (
                    QualifiedAddress::parse(&format!("{owner}.Module.{role}")).unwrap(),
                    "roundtrip",
                )
            };
            assert_eq!(
                profile.module_capability(&address).unwrap().role().as_str(),
                role
            );
            let module = source.join(directory).join(name).join("Ext").join(terminal);
            assert!(module.is_file(), "{module:?}");
            rows.push(json!({
                "schema_version": 1, "record_type": "required",
                "key": address_key((line, SourceSetKind::Configuration), &owner, "Module", Some(role)),
                "requirement": {"kind": "exact", "observations": [{
                    "owner": owner.to_string(), "addresses": [address.to_string()],
                    "origin": fixture_origin(&module), "level": level,
                    "test_case": "native_registered_modules_match_profile_and_physical_paths"
                }]},
            }));
        }
    }
    rows
}
