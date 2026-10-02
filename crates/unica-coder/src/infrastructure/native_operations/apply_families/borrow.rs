use super::request::ProvisionalApplyEffect;
use crate::domain::address::{NodeKind, QualifiedAddress};
use crate::domain::events::{DomainEvent, DomainEventKind};
use crate::domain::project_sources::SourceSetKind;
use crate::infrastructure::native_operations::apply::{
    ApplyPlanError, ApplyPlanErrorKind, ApplyStagedState,
};
use crate::infrastructure::workspace_actor::{ApplyAdmission, ProviderRootBinding};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub(super) struct BorrowOperation {
    parent: QualifiedAddress,
    overrides: Option<Vec<String>>,
    index: usize,
}

pub(super) fn parse(
    args: &serde_json::Value,
    index: usize,
    binding: &ProviderRootBinding,
) -> Result<BorrowOperation, ApplyPlanError> {
    let fail = |message: &str| {
        ApplyPlanError::new(ApplyPlanErrorKind::BadValue, message)
            .at_path(format!("ops[{index}].args"))
    };
    if binding.source_kind() != SourceSetKind::Extension {
        return Err(fail("object.borrow requires an extension destination"));
    }
    super::validate_platform_xml_binding(binding, index)?;
    let args = args
        .as_object()
        .ok_or_else(|| fail("borrow args must be an object"))?;
    for key in args.keys() {
        if !["at", "from", "overrides"].contains(&key.as_str()) {
            return Err(fail("unknown object.borrow argument"));
        }
    }
    let target = QualifiedAddress::parse(
        args.get("at")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default(),
    )
    .map_err(|error| fail(&error.to_string()))?;
    if target.segments().len() != 1 || target.segments()[0].kind() != NodeKind::Configuration {
        return Err(fail(
            "object.borrow target must be the extension Configuration root",
        ));
    }
    let parent = QualifiedAddress::parse(
        args.get("from")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default(),
    )
    .map_err(|error| fail(&error.to_string()))?;
    if parent.source_set() == binding.source_set_name()
        || parent.segments().len() != 1
        || parent.segments()[0].name().is_none()
    {
        return Err(fail(
            "from must name a top-level object in a distinct parent source set",
        ));
    }
    let overrides = args
        .get("overrides")
        .map(|value| {
            let array = value
                .as_array()
                .ok_or_else(|| fail("overrides must be an array of property names"))?;
            array
                .iter()
                .map(|value| {
                    value
                        .as_str()
                        .filter(|value| !value.is_empty())
                        .map(str::to_string)
                        .ok_or_else(|| fail("override must be a nonempty property name"))
                })
                .collect::<Result<Vec<_>, _>>()
        })
        .transpose()?;
    if let Some(overrides) = &overrides {
        crate::infrastructure::native_operations::cfe_borrow_object::validate_requested_overrides(
            overrides,
        )
        .map_err(|error| fail(&error.to_string()))?;
    }
    Ok(BorrowOperation {
        parent,
        overrides,
        index,
    })
}

fn registered(owner: &[u8], kind: &str, name: &str) -> Result<bool, ApplyPlanError> {
    let text = std::str::from_utf8(owner)
        .map_err(|error| ApplyPlanError::new(ApplyPlanErrorKind::InvalidSource, error.to_string()))?
        .trim_start_matches('\u{feff}');
    let document = roxmltree::Document::parse(text).map_err(|error| {
        ApplyPlanError::new(ApplyPlanErrorKind::InvalidSource, error.to_string())
    })?;
    const NS: &str = "http://v8.1c.ru/8.3/MDClasses";
    let root = document.root_element();
    let objects = root
        .children()
        .filter(|node| node.is_element())
        .collect::<Vec<_>>();
    if !root.has_tag_name((NS, "MetaDataObject"))
        || root.attribute("version") != Some("2.20")
        || objects.len() != 1
        || !objects[0].has_tag_name((NS, "Configuration"))
    {
        return Err(ApplyPlanError::new(
            ApplyPlanErrorKind::InvalidSource,
            "parent/destination owner is not a supported Configuration descriptor",
        ));
    }
    let blocks = objects[0]
        .children()
        .filter(|node| node.has_tag_name((NS, "ChildObjects")))
        .collect::<Vec<_>>();
    if blocks.len() != 1 {
        return Err(ApplyPlanError::new(
            ApplyPlanErrorKind::InvalidSource,
            "configuration must contain one ChildObjects block",
        ));
    }
    let count = blocks[0]
        .children()
        .filter(|node| node.has_tag_name((NS, kind)) && node.text() == Some(name))
        .count();
    if count > 1 {
        return Err(ApplyPlanError::new(
            ApplyPlanErrorKind::InvalidSource,
            "configuration registers the parent object ambiguously",
        ));
    }
    Ok(count == 1)
}

pub(super) fn plan(
    staged: &mut ApplyStagedState,
    admission: &ApplyAdmission,
    binding: &ProviderRootBinding,
    operation: &BorrowOperation,
) -> Result<
    (
        Vec<ProvisionalApplyEffect>,
        crate::infrastructure::native_operations::apply::BorrowPlanDetail,
    ),
    ApplyPlanError,
> {
    admission.metadata_planning_authority(binding)?;
    let index = operation.index;
    let parent = &operation.parent;
    let owner = &parent.segments()[0];
    let kind = owner.kind().as_str();
    let name = owner.name().expect("parsed borrow has object name");
    let at = parent.to_string();
    let layout = crate::infrastructure::metadata_kinds::metadata_kind(kind).ok_or_else(|| {
        ApplyPlanError::new(
            ApplyPlanErrorKind::BadValue,
            "from is not a supported top-level metadata object",
        )
        .at_path(at.clone())
    })?;
    let relative = PathBuf::from(layout.directory).join(format!("{name}.xml"));
    let parent_owner =
        admission.read_apply_dependency(parent.source_set(), Path::new("Configuration.xml"))?;
    if !registered(&parent_owner, kind, name)? {
        return Err(ApplyPlanError::new(
            ApplyPlanErrorKind::NotFound,
            "parent object is not registered in its configuration",
        )
        .at_path(at));
    }
    let parent_bytes = admission.read_apply_dependency(parent.source_set(), &relative)?;
    if let Some(finding) = crate::infrastructure::format_guard::classify_staged_platform_xml_root(
        &relative,
        &parent_bytes,
    ) {
        return Err(
            ApplyPlanError::new(ApplyPlanErrorKind::InvalidSource, finding.message)
                .at_path(parent.to_string()),
        );
    }
    let owner_path = PathBuf::from("Configuration.xml");
    let destination_owner = staged
        .read(&owner_path)
        .map_err(|error| ApplyPlanError::staging(error, "Configuration"))?
        .ok_or_else(|| {
            ApplyPlanError::new(
                ApplyPlanErrorKind::NotFound,
                "extension Configuration descriptor is missing",
            )
        })?;
    let existing = staged
        .read(&relative)
        .map_err(|error| ApplyPlanError::staging(error, parent.to_string()))?;
    if let Some(finding) = existing.as_deref().and_then(|bytes| {
        crate::infrastructure::format_guard::classify_staged_platform_xml_root(&relative, bytes)
    }) {
        return Err(
            ApplyPlanError::new(ApplyPlanErrorKind::InvalidSource, finding.message)
                .at_path(format!("{}:{kind}.{name}", binding.source_set_name())),
        );
    }
    let is_registered = registered(&destination_owner, kind, name)?;
    if is_registered != existing.is_some() {
        return Err(ApplyPlanError::new(
            ApplyPlanErrorKind::InvalidSource,
            "extension object registration and descriptor disagree",
        ));
    }
    let borrowed = crate::infrastructure::native_operations::cfe_borrow_object::plan_borrow_object(
        &parent_bytes,
        existing.as_deref(),
        operation.overrides.as_deref(),
    )
    .map_err(|error| {
        let kind = match error.kind {
            crate::infrastructure::native_operations::cfe_borrow_object::BorrowObjectErrorKind::BadValue => ApplyPlanErrorKind::BadValue,
            crate::infrastructure::native_operations::cfe_borrow_object::BorrowObjectErrorKind::InvalidSource => ApplyPlanErrorKind::InvalidSource,
        };
        ApplyPlanError::new(kind, error.to_string()).at_path(if kind == ApplyPlanErrorKind::BadValue { format!("ops[{index}].args") } else { parent.to_string() })
    })?;
    if borrowed.kind != kind || borrowed.name != name {
        return Err(ApplyPlanError::new(
            ApplyPlanErrorKind::InvalidSource,
            "registered parent address and descriptor identity disagree",
        ));
    }
    let properties =
        |bytes: &[u8]| -> Result<std::collections::BTreeMap<String, String>, ApplyPlanError> {
            let text = std::str::from_utf8(bytes)
                .map_err(|error| {
                    ApplyPlanError::new(ApplyPlanErrorKind::InvalidSource, error.to_string())
                })?
                .trim_start_matches('\u{feff}');
            let document = roxmltree::Document::parse(text).map_err(|error| {
                ApplyPlanError::new(ApplyPlanErrorKind::InvalidSource, error.to_string())
            })?;
            let object = document
                .root_element()
                .children()
                .find(|node| node.is_element())
                .ok_or_else(|| {
                    ApplyPlanError::new(
                        ApplyPlanErrorKind::InvalidSource,
                        "borrow descriptor has no object",
                    )
                })?;
            Ok(object
                .children()
                .filter(|node| node.has_tag_name(("http://v8.1c.ru/8.3/MDClasses", "Properties")))
                .flat_map(|node| node.children())
                .filter(|node| node.is_element())
                .map(|node| {
                    (
                        node.tag_name().name().to_string(),
                        text[node.range()].to_string(),
                    )
                })
                .collect())
        };
    let previous_properties = existing
        .as_deref()
        .map(properties)
        .transpose()?
        .unwrap_or_default();
    let next_properties = properties(&borrowed.bytes)?;
    let changed_properties = previous_properties
        .keys()
        .chain(next_properties.keys())
        .cloned()
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .filter(|key| previous_properties.get(key) != next_properties.get(key))
        .collect();
    let detail = crate::infrastructure::native_operations::apply::BorrowPlanDetail {
        index,
        at: format!("{}:{kind}.{name}", binding.source_set_name()),
        from: parent.to_string(),
        parent_uuid: borrowed.parent_uuid.clone(),
        created: existing.is_none(),
        changed_properties,
        protected_overrides: crate::infrastructure::native_operations::meta::parse_meta_borrowing(
            &borrowed.bytes,
        )
        .map_err(|error| ApplyPlanError::new(ApplyPlanErrorKind::InvalidSource, error))?
        .overrides,
    };
    let mut effects = Vec::new();
    if existing.as_deref() != Some(borrowed.bytes.as_slice()) {
        match existing {
            Some(previous) => staged.replace(&relative, &previous, borrowed.bytes),
            None => staged.create(&relative, borrowed.bytes),
        }
        .map_err(|error| ApplyPlanError::staging(error, parent.to_string()))?;
        effects.push(ProvisionalApplyEffect::single(
            &relative,
            DomainEvent::new(
                DomainEventKind::MetadataChanged,
                format!("{}:{kind}.{name}", binding.source_set_name()),
            ),
            index,
        ));
    }
    if !is_registered {
        let updated =
            super::metadata::owner_registration_image(&destination_owner, kind, name, true, index)?
                .ok_or_else(|| {
                    ApplyPlanError::new(
                        ApplyPlanErrorKind::InvalidSource,
                        "new borrow could not register its object",
                    )
                })?;
        staged
            .replace(&owner_path, &destination_owner, updated)
            .map_err(|error| ApplyPlanError::staging(error, "Configuration"))?;
        effects.push(ProvisionalApplyEffect::single(
            owner_path,
            DomainEvent::new(
                DomainEventKind::MetadataChanged,
                format!("{}:Configuration", binding.source_set_name()),
            ),
            index,
        ));
    }
    Ok((effects, detail))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::apply::ApplyRequest;
    use crate::domain::cancellation::CancellationToken;
    use crate::domain::code_intelligence::ProviderDeadline;
    use crate::domain::project_sources::{SourceFormat, SourceProfile};
    use crate::domain::workspace::WorkspaceContext;
    use crate::infrastructure::workspace_actor::{
        WorkspaceActor, WorkspaceIdentity, WorkspaceSourceSetInput,
    };
    use std::time::Duration;

    struct Fixture {
        _root: tempfile::TempDir,
        actor: WorkspaceActor,
        destination: ProviderRootBinding,
        parent: ProviderRootBinding,
        dst: PathBuf,
        src: PathBuf,
    }
    impl Fixture {
        fn new() -> Self {
            let root = tempfile::tempdir().unwrap();
            let workspace = std::fs::canonicalize(root.path()).unwrap();
            let dst = workspace.join("extension");
            let src = workspace.join("main");
            std::fs::create_dir_all(dst.join("Catalogs")).unwrap();
            std::fs::create_dir_all(src.join("Catalogs")).unwrap();
            std::fs::write(workspace.join("v8project.yaml"), "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: main\n  - name: extension\n    type: EXTENSION\n    path: extension\n").unwrap();
            for (path, children) in [(&src, "<Catalog>Orders</Catalog>"), (&dst, "")] {
                std::fs::write(path.join("Configuration.xml"),format!(r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Configuration uuid="10000000-0000-4000-8000-000000000001"><Properties><Name>Sample</Name></Properties><ChildObjects>{children}</ChildObjects></Configuration></MetaDataObject>"#)).unwrap();
            }
            std::fs::write(src.join("Catalogs/Orders.xml"), r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Catalog uuid="00000000-0000-4000-8000-000000000001"><Properties><Name>Orders</Name><Comment>parent comment</Comment><CodeLength>9</CodeLength><DescriptionLength>25</DescriptionLength><Hierarchical>false</Hierarchical></Properties><ChildObjects/></Catalog></MetaDataObject>"#).unwrap();
            let context = WorkspaceContext {
                cwd: workspace.clone(),
                workspace_root: workspace.clone(),
                cache_root: workspace.join(".build/unica"),
                workspace_epoch: 1,
            };
            let identity = WorkspaceIdentity::new(
                &context,
                [
                    WorkspaceSourceSetInput::new(
                        "main",
                        &src,
                        SourceSetKind::Configuration,
                        SourceFormat::PlatformXml,
                        SourceProfile::platform_xml_8_3_27_format_2_20(),
                    ),
                    WorkspaceSourceSetInput::new(
                        "extension",
                        &dst,
                        SourceSetKind::Extension,
                        SourceFormat::PlatformXml,
                        SourceProfile::platform_xml_8_3_27_format_2_20(),
                    ),
                ],
                "borrow-test",
            )
            .unwrap();
            let actor = WorkspaceActor::new(identity, context).unwrap();
            let destination = actor.bind_provider_root("extension", &dst).unwrap();
            let parent = actor.bind_provider_root("main", &src).unwrap();
            Self {
                _root: root,
                actor,
                destination,
                parent,
                dst,
                src,
            }
        }
        fn run(
            &self,
            dry: bool,
            rev: Option<&str>,
            overrides: Option<serde_json::Value>,
        ) -> Result<crate::infrastructure::workspace_actor::ApplyPublicationResult, String>
        {
            let mut args = serde_json::json!({"from":"main:Catalog.Orders"});
            if let Some(overrides) = overrides {
                args["overrides"] = overrides;
            }
            let mut input = serde_json::json!({"at":"extension:Configuration","dryRun":dry,"ops":[{"op":"object.borrow","args":args}]});
            if let Some(rev) = rev {
                input["ifRev"] = serde_json::json!(rev);
            }
            let request = ApplyRequest::parse(input.as_object().unwrap(), &["main", "extension"])
                .map_err(|e| e.to_string())?;
            let admission = self
                .actor
                .admit_apply_with_dependencies(
                    &self.destination,
                    std::slice::from_ref(&self.parent),
                    request.if_rev(),
                    request.dry_run(),
                    ProviderDeadline::from_budget(Duration::from_secs(10)),
                    &CancellationToken::new(),
                )
                .map_err(|e| e.to_string())?;
            let (staged, effects) =
                super::super::plan_hidden_v13_apply(&request, &self.destination, &admission)
                    .map_err(|e| e.to_string())?;
            let prepared = admission
                .prepare_with_effects(staged, effects)
                .map_err(|e| e.to_string())?;
            self.actor
                .publish_prepared_apply(prepared)
                .map_err(|e| e.to_string())
        }
    }

    #[test]
    fn canonical_borrow_preview_refresh_and_noop_keep_local_identity_and_events_honest() {
        let fixture = Fixture::new();
        let preview = fixture
            .run(true, None, Some(serde_json::json!(["DescriptionLength"])))
            .unwrap();
        assert!(!fixture.dst.join("Catalogs/Orders.xml").exists());
        let committed = fixture
            .run(
                false,
                Some(preview.rev()),
                Some(serde_json::json!(["DescriptionLength"])),
            )
            .unwrap();
        let descriptor = fixture.dst.join("Catalogs/Orders.xml");
        let before = std::fs::read_to_string(&descriptor).unwrap();
        let before_doc = roxmltree::Document::parse(&before).unwrap();
        let identity = before_doc
            .root_element()
            .children()
            .find(|n| n.is_element())
            .unwrap()
            .attribute("uuid")
            .unwrap()
            .to_string();
        assert!(committed
            .rev()
            .starts_with("unica-apply-sources-sha256-v1:"));
        let description = before_doc
            .descendants()
            .find(|node| node.has_tag_name(("http://v8.1c.ru/8.3/MDClasses", "DescriptionLength")))
            .unwrap()
            .children()
            .find(|node| node.is_text())
            .unwrap()
            .range();
        let mut local = before.clone();
        local.replace_range(description, "30");
        std::fs::write(&descriptor, local).unwrap();
        let parent_path = fixture.src.join("Catalogs/Orders.xml");
        let parent = std::fs::read_to_string(&parent_path).unwrap();
        std::fs::write(
            &parent_path,
            parent
                .replace("<CodeLength>9</CodeLength>", "<CodeLength>11</CodeLength>")
                .replace(
                    "<DescriptionLength>25</DescriptionLength>",
                    "<DescriptionLength>60</DescriptionLength>",
                )
                .replace("parent comment", "new parent comment"),
        )
        .unwrap();
        assert!(fixture.run(false, Some(committed.rev()), None).is_err());
        let refresh = fixture.run(true, None, None).unwrap();
        assert!(!refresh.effects().events().is_empty());
        let refreshed = fixture.run(false, Some(refresh.rev()), None).unwrap();
        let after = std::fs::read_to_string(&descriptor).unwrap();
        let after_doc = roxmltree::Document::parse(&after).unwrap();
        let property = |name| {
            after_doc
                .descendants()
                .find(|node| node.has_tag_name(("http://v8.1c.ru/8.3/MDClasses", name)))
                .unwrap()
                .text()
        };
        assert_eq!(property("CodeLength"), Some("11"));
        assert_eq!(property("DescriptionLength"), Some("30"));
        assert!(after.contains(&identity));
        assert!(!after.contains("new parent comment"));
        let file_identity =
            crate::infrastructure::platform::testing::file_identity_for_test(&descriptor).unwrap();
        let noop = fixture.run(false, Some(refreshed.rev()), None).unwrap();
        assert!(noop.effects().events().is_empty());
        assert_eq!(std::fs::read_to_string(&descriptor).unwrap(), after);
        assert_eq!(
            crate::infrastructure::platform::testing::file_identity_for_test(&descriptor).unwrap(),
            file_identity
        );
    }

    #[test]
    fn canonical_borrow_bad_override_is_an_argument_error() {
        let fixture = Fixture::new();
        for overrides in [
            serde_json::json!(["Comment"]),
            serde_json::json!(["DescriptionLength", "DescriptionLength"]),
        ] {
            let error = parse(&serde_json::json!({"at":"extension:Configuration","from":"main:Catalog.Orders","overrides":overrides}), 0, &fixture.destination).expect_err("invalid override is rejected before source planning");
            assert_eq!(error.kind(), ApplyPlanErrorKind::BadValue);
        }
        use crate::infrastructure::native_operations::cfe_borrow_object::{
            plan_borrow_object, BorrowObjectErrorKind,
        };
        let parent = std::fs::read(fixture.src.join("Catalogs/Orders.xml")).unwrap();
        let unsupported =
            plan_borrow_object(&parent, None, Some(&["UnknownProperty".to_string()])).unwrap_err();
        assert_eq!(unsupported.kind, BorrowObjectErrorKind::BadValue);
        let first =
            plan_borrow_object(&parent, None, Some(&["DescriptionLength".to_string()])).unwrap();
        let changed = plan_borrow_object(&parent, Some(&first.bytes), Some(&[])).unwrap_err();
        assert_eq!(changed.kind, BorrowObjectErrorKind::BadValue);
    }

    #[test]
    fn canonical_borrow_refuses_parent_descriptor_outside_writable_profile() {
        let fixture = Fixture::new();
        let descriptor = fixture.src.join("Catalogs/Orders.xml");
        let parent = std::fs::read_to_string(&descriptor).unwrap();
        std::fs::write(
            &descriptor,
            parent.replace("version=\"2.20\"", "version=\"2.21\""),
        )
        .unwrap();
        let before = crate::test_support::tree_snapshot(&fixture.dst);
        let error = fixture
            .run(true, None, None)
            .expect_err("a newer parent descriptor must refuse before emitting borrowed XML");
        assert!(error.contains("2.21"), "{error}");
        assert_eq!(crate::test_support::tree_snapshot(&fixture.dst), before);
        assert!(!fixture.actor.context().cache_root.exists());
    }

    #[test]
    fn canonical_borrow_refuses_unregistered_parent_and_destination_own_object() {
        let fixture = Fixture::new();
        let preview = fixture
            .run(true, None, Some(serde_json::json!(["DescriptionLength"])))
            .unwrap();
        fixture
            .run(
                false,
                Some(preview.rev()),
                Some(serde_json::json!(["DescriptionLength"])),
            )
            .unwrap();
        let descriptor = fixture.dst.join("Catalogs/Orders.xml");
        let parent = std::fs::read(fixture.src.join("Catalogs/Orders.xml")).unwrap();
        std::fs::write(&descriptor, &parent).unwrap();
        assert!(fixture.run(true, None, None).is_err());
        assert_eq!(std::fs::read(&descriptor).unwrap(), parent);
        let owner = fixture.src.join("Configuration.xml");
        std::fs::write(
            &owner,
            std::fs::read_to_string(&owner)
                .unwrap()
                .replace("<Catalog>Orders</Catalog>", ""),
        )
        .unwrap();
        assert!(fixture
            .run(true, None, None)
            .unwrap_err()
            .contains("not registered"));
    }
}
