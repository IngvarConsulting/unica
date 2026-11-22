use super::*;

const FORMAT: &str = "2.20";

pub(super) fn check_inherited_cfe_registrations(evidence: &mut EvidenceRows) {
    let workspace = fixture_path("acceptance/workspace-borrowed/src");
    let source = workspace.join("ext");
    let base = workspace.join("cf");
    let cancellation = CancellationToken::new();
    let line = export_format_line(FORMAT).unwrap();
    let authority = LogicalViewReadAuthority::new(
        &cancellation,
        "ext",
        "native-workspace-borrowed",
        SourceSetKind::Extension,
        Arc::new(
            unica_workspace::read::open_source_directory(&source.canonicalize().unwrap()).unwrap(),
        ),
        PlatformProfile::for_line(line).unwrap(),
    );
    let base_registrations = root_registrations(&base, FORMAT);
    let configuration = read(&authority, "ext:Configuration");
    let mut groups: BTreeMap<String, Vec<(String, PathBuf)>> = BTreeMap::new();
    for (kind, name) in root_registrations(&source, FORMAT) {
        assert!(
            base_registrations.contains(&(kind.clone(), name.clone())),
            "{kind}.{name}"
        );
        let (_, declaration) = unica_address::kinds::declared(&kind).unwrap();
        let directory = declaration.layout.unwrap().directory;
        let relative = Path::new(directory).join(format!("{name}.xml"));
        groups.entry(kind).or_default().push((name, relative));
    }
    assert!(!groups.is_empty());
    let mut inherited = InheritedCfe {
        authority: &authority,
        evidence,
        source: &source,
        base: &base,
    };
    for (kind, members) in groups {
        inherited.verify_branch(
            "ext:Configuration",
            &configuration,
            &kind,
            &source.join("Configuration.xml"),
            &members,
        );
    }

    let template = "ext:CommonTemplate.ОбщийМакет";
    let view = read(&authority, template);
    assert_eq!(view["props"]["content"], "inherited");
    assert!(view.get("branches").is_none());
    let companion = "CommonTemplates/ОбщийМакет/Ext/Template.xml";
    assert!(base.join(companion).is_file());
    assert!(!source.join(companion).exists());
    let area = QualifiedAddress::parse(&format!("{template}.Area")).unwrap();
    let failure = authority
        .snapshot(&area)
        .and_then(|snapshot| authority.read_exact(&area, &snapshot))
        .expect_err("a template with inherited content has no local Area branch");
    assert_eq!(failure.code(), RefusalCode::NotFound);
    assert!(failure.to_string().contains("Template.xml"), "{failure}");
}

struct InheritedCfe<'a, 'scope> {
    authority: &'a LogicalViewReadAuthority<'scope>,
    evidence: &'a mut EvidenceRows,
    source: &'a Path,
    base: &'a Path,
}

impl InheritedCfe<'_, '_> {
    fn verify_branch(
        &mut self,
        owner: &str,
        owner_view: &Value,
        kind: &str,
        descriptor: &Path,
        members: &[(String, PathBuf)],
    ) {
        let branch = if owner == "ext:Configuration" {
            format!("ext:{kind}")
        } else {
            format!("{owner}.{kind}")
        };
        let collection = QualifiedAddress::parse(&branch).unwrap();
        let expected: BTreeSet<_> = members
            .iter()
            .map(|(name, _)| collection.member(name).unwrap().to_string())
            .collect();
        assert_eq!(
            expected.len(),
            members.len(),
            "{branch}: duplicate registration"
        );
        check_native_branch(self.authority, owner_view, &branch, &expected);
        let line = self.authority.platform_profile().line();
        self.evidence.record(
            line,
            owner,
            kind,
            None,
            &fixture_origin(descriptor),
            AddressEvidence::Read(&branch),
        );
        for (name, path) in members {
            let local_path = self.source.join(path);
            let parent_path = self.base.join(path);
            let local_raw = read_fixture_descriptor(&local_path);
            let parent_raw = read_fixture_descriptor(&parent_path);
            let local_document = parse_fixture_descriptor(&local_raw, &local_path);
            let parent_document = parse_fixture_descriptor(&parent_raw, &parent_path);
            let local = fixture_artifact(&local_document, kind, &local_path);
            let parent = fixture_artifact(&parent_document, kind, &parent_path);
            assert_eq!(fixture_properties_name(local), Some(name.as_str()));
            assert_eq!(fixture_properties_name(parent), Some(name.as_str()));
            let properties = child(local, MD_CLASSES_NS, "Properties");
            assert_eq!(
                child(properties, MD_CLASSES_NS, "ObjectBelonging").text(),
                Some("Adopted")
            );
            assert_eq!(
                child(properties, MD_CLASSES_NS, "ExtendedConfigurationObject").text(),
                parent.attribute("uuid")
            );
            let at = collection.member(name).unwrap().to_string();
            let view = read(self.authority, &at);
            self.evidence.record(
                line,
                owner,
                kind,
                None,
                &fixture_origin(&local_path),
                AddressEvidence::Read(&at),
            );
            let mut children: BTreeMap<&str, Vec<_>> = BTreeMap::new();
            for item in native_child_objects(local) {
                let tag = item.tag_name().name();
                let child_name = item.text().unwrap();
                let directory = match (kind, tag) {
                    ("DataProcessor", "Form") => "Forms",
                    ("DataProcessor", "Template") => "Templates",
                    _ => panic!("{at}: unproved borrowed child layout for {tag}"),
                };
                assert!(
                    native_child_objects(parent)
                        .any(|candidate| candidate.tag_name() == item.tag_name()
                            && candidate.text() == Some(child_name)),
                    "{at}.{tag}.{child_name}"
                );
                children.entry(tag).or_default().push((
                    child_name.to_owned(),
                    path.with_extension("")
                        .join(directory)
                        .join(format!("{child_name}.xml")),
                ));
            }
            for (kind, members) in children {
                self.verify_branch(&at, &view, kind, &local_path, &members);
            }
        }
    }
}
