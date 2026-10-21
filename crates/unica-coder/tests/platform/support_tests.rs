use super::*;
use serde_json::json;
use std::fs;
use std::time::{SystemTime, UNIX_EPOCH};
use unica_workspace::single_file_publisher::with_before_commit_hook;

struct SupportFixture {
    root: PathBuf,
    context: WorkspaceContext,
    config_path: PathBuf,
    bin_path: PathBuf,
}

impl SupportFixture {
    fn new(label: &str, version: &str) -> Self {
        let root = std::env::temp_dir().join(format!(
            "unica-support-native-{label}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let source = root.join("src");
        fs::create_dir_all(source.join("Ext")).unwrap();
        fs::write(
                root.join("v8project.yaml"),
                "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
            )
            .unwrap();
        let config_path = source.join("Configuration.xml");
        fs::write(&config_path, configuration_xml(version)).unwrap();
        let bin_path = source.join("Ext/ParentConfigurations.bin");
        fs::write(&bin_path, parent_configurations()).unwrap();
        let vendor_dir = source.join("Ext/ParentConfigurations");
        fs::create_dir(&vendor_dir).unwrap();
        fs::write(vendor_dir.join("VendorConf.cf"), b"platform vendor payload").unwrap();
        let context = WorkspaceContext {
            cwd: root.clone(),
            workspace_root: root.clone(),
            cache_root: root.join(".build/unica"),
            workspace_epoch: 0,
        };
        Self {
            root,
            context,
            config_path,
            bin_path,
        }
    }

    fn capability_off_args(&self) -> Map<String, Value> {
        json!({
            "Path": "src",
            "Capability": "off"
        })
        .as_object()
        .unwrap()
        .clone()
    }

    fn object_target(&self, name: &str, version: &str, uuid: &str) -> PathBuf {
        let object_dir = self.root.join("src/Catalogs").join(name);
        let target = object_dir.join("Ext/ObjectModule.bsl");
        fs::create_dir_all(target.parent().unwrap()).unwrap();
        fs::write(&target, "Процедура Тест() Экспорт\nКонецПроцедуры\n").unwrap();
        fs::write(
            object_dir.with_extension("xml"),
            metadata_object_xml(name, version, uuid),
        )
        .unwrap();
        target
    }

    fn set_editable_args(&self, target: &Path) -> Map<String, Value> {
        json!({
            "Path": target.display().to_string(),
            "Set": "editable"
        })
        .as_object()
        .unwrap()
        .clone()
    }
}

impl Drop for SupportFixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn configuration_xml(version: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="{version}">
  <Configuration uuid="aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa">
    <Properties><Name>Demo</Name></Properties>
    <ChildObjects/>
  </Configuration>
</MetaDataObject>
"#
    )
}

fn metadata_object_xml(name: &str, version: &str, uuid: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="{version}">
  <Catalog uuid="{uuid}">
    <Properties><Name>{name}</Name></Properties>
    <ChildObjects/>
  </Catalog>
</MetaDataObject>
"#
    )
}

fn metadata_object_xml_without_uuid(name: &str, version: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="{version}">
  <Catalog>
    <Properties><Name>{name}</Name></Properties>
    <ChildObjects/>
  </Catalog>
</MetaDataObject>
"#
    )
}

fn parent_configurations() -> String {
    "\u{feff}{6,0,1,dddddddd-dddd-dddd-dddd-dddddddddddd,0,eeeeeeee-eeee-eeee-eeee-eeeeeeeeeeee,\"1.0\",\"Vendor\",\"VendorConf\",3,1,0,aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa,aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa,0,0,bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb,bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb}"
            .to_string()
}

fn locked_parent_configurations_without_payload() -> String {
    parent_configurations()
        .replacen("{6,0,1,", "{6,1,1,", 1)
        .replacen(
            "dddddddd-dddd-dddd-dddd-dddddddddddd,0,eeeeeeee-eeee-eeee-eeee-eeeeeeeeeeee",
            "dddddddd-dddd-dddd-dddd-dddddddddddd,1,eeeeeeee-eeee-eeee-eeee-eeeeeeeeeeee",
            1,
        )
}

fn editable_parent_configurations_with_unlocked_vendor_without_payload() -> String {
    parent_configurations().replacen(
        "dddddddd-dddd-dddd-dddd-dddddddddddd,0,eeeeeeee-eeee-eeee-eeee-eeeeeeeeeeee",
        "dddddddd-dddd-dddd-dddd-dddddddddddd,1,eeeeeeee-eeee-eeee-eeee-eeeeeeeeeeee",
        1,
    )
}

fn editable_parent_configurations_with_two_vendors() -> String {
    parent_configurations()
            .replacen("{6,0,1,", "{6,0,2,", 1)
            .replacen(
                ",3,1,0,aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa",
                ",ffffffff-ffff-ffff-ffff-ffffffffffff,0,cccccccc-cccc-cccc-cccc-cccccccccccc,\"2.0\",\"Second Vendor\",\"SecondVendor\",3,1,0,aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa",
                1,
            )
}

#[test]
fn support_edit_replaces_parent_configurations_transactionally() {
    let fixture = SupportFixture::new("transaction", "2.20");

    let outcome = edit_support_result(&fixture.capability_off_args(), &fixture.context).unwrap();

    assert!(outcome.ok, "{outcome:?}");
    assert!(fs::read_to_string(&fixture.bin_path)
        .unwrap()
        .contains("{6,1,"));
}

#[test]
pub(crate) fn repeated_support_edit_is_a_byte_and_identity_exact_noop() {
    use unica_workspace::test_support::file_identity_for_test;

    let fixture = SupportFixture::new("repeat-noop", "2.20");

    let first = edit_support_result(&fixture.capability_off_args(), &fixture.context).unwrap();
    assert!(first.ok, "{first:?}");
    let after = fs::read(&fixture.bin_path).unwrap();
    let identity = file_identity_for_test(&fixture.bin_path).unwrap();

    let repeated = edit_support_result(&fixture.capability_off_args(), &fixture.context).unwrap();
    assert!(repeated.ok, "{repeated:?}");
    assert!(repeated.changes.is_empty(), "{repeated:?}");
    assert_eq!(fs::read(&fixture.bin_path).unwrap(), after);
    assert_eq!(file_identity_for_test(&fixture.bin_path).unwrap(), identity);
}

#[test]
fn support_capability_off_only_changes_global_flag_and_locks_object_rules() {
    let fixture = SupportFixture::new("capability-off-semantics", "2.20");
    let vendor_dir = fixture.root.join("src/Ext/ParentConfigurations");
    fs::create_dir_all(&vendor_dir).unwrap();
    let vendor_payload = vendor_dir.join("VendorConf.cf");
    let vendor_bytes = b"platform vendor payload".to_vec();
    fs::write(&vendor_payload, &vendor_bytes).unwrap();

    let outcome = edit_support_result(&fixture.capability_off_args(), &fixture.context).unwrap();

    assert!(outcome.ok, "{outcome:?}");
    let updated = fs::read_to_string(&fixture.bin_path).unwrap();
    assert!(
        updated.contains(
            "dddddddd-dddd-dddd-dddd-dddddddddddd,0,eeeeeeee-eeee-eeee-eeee-eeeeeeeeeeee"
        ),
        "{updated}"
    );
    assert!(
        updated.contains(",0,0,aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa"),
        "{updated}"
    );
    assert!(
        updated.contains(",0,0,bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb"),
        "{updated}"
    );
    assert_eq!(fs::read(vendor_payload).unwrap(), vendor_bytes);
}

#[test]
fn support_capability_on_rejects_missing_required_vendor_payload_without_writing_bin() {
    let fixture = SupportFixture::new("capability-on-missing-vendor-payload", "2.20");
    fs::remove_dir_all(fixture.root.join("src/Ext/ParentConfigurations")).unwrap();
    fs::write(
        &fixture.bin_path,
        locked_parent_configurations_without_payload(),
    )
    .unwrap();
    let bin_before = fs::read(&fixture.bin_path).unwrap();
    let args = json!({
        "Path": "src",
        "Capability": "on"
    })
    .as_object()
    .unwrap()
    .clone();

    let error = edit_support_result(&args, &fixture.context).unwrap_err();

    assert!(error.contains("Capability=on"), "{error}");
    assert!(error.contains("required vendor payload"), "{error}");
    assert!(error.contains("VendorConf.cf"), "{error}");
    assert_eq!(fs::read(&fixture.bin_path).unwrap(), bin_before);
}

#[test]
fn support_capability_off_rejects_missing_required_vendor_payload_without_writing_bin() {
    let fixture = SupportFixture::new("capability-off-missing-vendor-payload", "2.20");
    fs::remove_dir_all(fixture.root.join("src/Ext/ParentConfigurations")).unwrap();
    fs::write(
        &fixture.bin_path,
        editable_parent_configurations_with_unlocked_vendor_without_payload(),
    )
    .unwrap();
    let bin_before = fs::read(&fixture.bin_path).unwrap();
    let args = json!({
        "Path": "src",
        "Capability": "off"
    })
    .as_object()
    .unwrap()
    .clone();

    let error = edit_support_result(&args, &fixture.context).unwrap_err();

    assert!(error.contains("Capability=off"), "{error}");
    assert!(error.contains("required vendor payload"), "{error}");
    assert!(error.contains("VendorConf.cf"), "{error}");
    assert_eq!(fs::read(&fixture.bin_path).unwrap(), bin_before);
}

#[test]
fn support_capability_off_rejects_missing_payload_for_already_locked_vendor() {
    let fixture = SupportFixture::new("capability-off-locked-vendor-missing-payload", "2.20");
    fs::remove_dir_all(fixture.root.join("src/Ext/ParentConfigurations")).unwrap();
    let bin_before = fs::read(&fixture.bin_path).unwrap();

    let error = edit_support_result(&fixture.capability_off_args(), &fixture.context).unwrap_err();

    assert!(error.contains("Capability=off"), "{error}");
    assert!(error.contains("VendorConf.cf"), "{error}");
    assert_eq!(fs::read(&fixture.bin_path).unwrap(), bin_before);
}

#[test]
fn support_capability_off_validates_every_vendor_in_the_post_image() {
    let fixture = SupportFixture::new("capability-off-multi-vendor-missing-payload", "2.20");
    fs::write(
        &fixture.bin_path,
        editable_parent_configurations_with_two_vendors(),
    )
    .unwrap();
    let vendor_dir = fixture.root.join("src/Ext/ParentConfigurations");
    fs::remove_dir_all(&vendor_dir).unwrap();
    fs::create_dir(&vendor_dir).unwrap();
    fs::write(vendor_dir.join("VendorConf.cf"), b"first vendor payload").unwrap();
    let bin_before = fs::read(&fixture.bin_path).unwrap();

    let error = edit_support_result(&fixture.capability_off_args(), &fixture.context).unwrap_err();

    assert!(error.contains("Capability=off"), "{error}");
    assert!(error.contains("SecondVendor.cf"), "{error}");
    assert!(!error.contains("VendorConf.cf,"), "{error}");
    assert_eq!(fs::read(&fixture.bin_path).unwrap(), bin_before);
}

#[test]
fn support_capability_on_accepts_and_preserves_matching_vendor_payload() {
    let fixture = SupportFixture::new("capability-on-with-vendor-payload", "2.20");
    fs::write(
        &fixture.bin_path,
        locked_parent_configurations_without_payload(),
    )
    .unwrap();
    let vendor_dir = fixture.root.join("src/Ext/ParentConfigurations");
    fs::remove_dir_all(&vendor_dir).unwrap();
    fs::create_dir(&vendor_dir).unwrap();
    let vendor_payload = vendor_dir.join("VendorConf.CF");
    let vendor_bytes = b"platform vendor payload".to_vec();
    fs::write(&vendor_payload, &vendor_bytes).unwrap();
    let args = json!({
        "Path": "src",
        "Capability": "on"
    })
    .as_object()
    .unwrap()
    .clone();

    let outcome = edit_support_result(&args, &fixture.context).unwrap();

    assert!(outcome.ok, "{outcome:?}");
    let updated = fs::read_to_string(&fixture.bin_path).unwrap();
    assert!(updated.contains("{6,0,1,"), "{updated}");
    assert!(
        updated.contains(
            "dddddddd-dddd-dddd-dddd-dddddddddddd,0,eeeeeeee-eeee-eeee-eeee-eeeeeeeeeeee"
        ),
        "{updated}"
    );
    assert_eq!(fs::read(vendor_payload).unwrap(), vendor_bytes);
}

#[test]
fn support_edit_rejects_a_concurrent_parent_configurations_change() {
    let fixture = SupportFixture::new("bin-race", "2.20");
    let mut concurrent = fs::read(&fixture.bin_path).unwrap();
    concurrent.extend_from_slice(b" ");
    let bin_for_hook = fixture.bin_path.clone();
    let concurrent_for_hook = concurrent.clone();

    let outcome = with_before_commit_hook(
        move |_| fs::write(&bin_for_hook, &concurrent_for_hook).unwrap(),
        || edit_support_result(&fixture.capability_off_args(), &fixture.context),
    )
    .unwrap_err();

    assert!(
        outcome.contains("changed") || outcome.contains("preimage"),
        "{outcome}"
    );
    assert_eq!(fs::read(&fixture.bin_path).unwrap(), concurrent);
}

#[test]
fn support_edit_rejects_a_concurrent_vendor_payload_change() {
    let fixture = SupportFixture::new("vendor-payload-race", "2.20");
    let vendor_dir = fixture.root.join("src/Ext/ParentConfigurations");
    fs::create_dir_all(&vendor_dir).unwrap();
    let vendor_payload = vendor_dir.join("VendorConf.cf");
    let original_vendor = b"original vendor payload".to_vec();
    let concurrent_vendor = b"concurrent vendor payload".to_vec();
    fs::write(&vendor_payload, &original_vendor).unwrap();
    let payload_for_hook = vendor_payload.clone();
    let concurrent_for_hook = concurrent_vendor.clone();
    let bin_before = fs::read(&fixture.bin_path).unwrap();

    let error = with_before_commit_hook(
        move |_| fs::write(&payload_for_hook, &concurrent_for_hook).unwrap(),
        || edit_support_result(&fixture.capability_off_args(), &fixture.context),
    )
    .unwrap_err();

    assert!(error.contains("read guard"), "{error}");
    assert_eq!(fs::read(&vendor_payload).unwrap(), concurrent_vendor);
    assert_eq!(fs::read(&fixture.bin_path).unwrap(), bin_before);
}

#[test]
fn support_edit_rejects_a_concurrent_vendor_payload_create_case_insensitively() {
    let fixture = SupportFixture::new("vendor-payload-create-race", "2.20");
    let vendor_dir = fixture.root.join("src/Ext/ParentConfigurations");
    fs::create_dir_all(&vendor_dir).unwrap();
    let existing_payload = vendor_dir.join("VendorConf.cf");
    fs::write(&existing_payload, b"original vendor payload").unwrap();
    let concurrent_payload = vendor_dir.join("ConcurrentVendor.CF");
    let concurrent_for_hook = concurrent_payload.clone();
    let bin_before = fs::read(&fixture.bin_path).unwrap();

    let error = with_before_commit_hook(
        move |_| fs::write(&concurrent_for_hook, b"concurrent vendor payload").unwrap(),
        || edit_support_result(&fixture.capability_off_args(), &fixture.context),
    )
    .unwrap_err();

    assert!(error.contains("directory membership guard"), "{error}");
    assert_eq!(
        fs::read(&concurrent_payload).unwrap(),
        b"concurrent vendor payload"
    );
    assert_eq!(fs::read(&fixture.bin_path).unwrap(), bin_before);
}

#[test]
pub(crate) fn support_edit_rejects_a_concurrent_configuration_owner_change() {
    let fixture = SupportFixture::new("owner-race", "2.20");
    let bin_before = fs::read(&fixture.bin_path).unwrap();
    let concurrent = configuration_xml("2.20").replace("<ChildObjects/>", "<!-- concurrent -->");
    let config_for_hook = fixture.config_path.clone();
    let concurrent_for_hook = concurrent.clone();

    let outcome = with_before_commit_hook(
        move |_| fs::write(&config_for_hook, &concurrent_for_hook).unwrap(),
        || edit_support_result(&fixture.capability_off_args(), &fixture.context),
    )
    .unwrap_err();

    assert!(outcome.contains("read guard"), "{outcome}");
    assert_eq!(
        fs::read_to_string(&fixture.config_path).unwrap(),
        concurrent
    );
    assert_eq!(fs::read(&fixture.bin_path).unwrap(), bin_before);
}

#[test]
fn support_edit_rejects_a_concurrent_uuid_descriptor_change() {
    let fixture = SupportFixture::new("uuid-descriptor-race", "2.20");
    let target = fixture.object_target("Items", "2.20", "bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb");
    let descriptor_path = fixture.root.join("src/Catalogs/Items.xml");
    let concurrent = metadata_object_xml("Items", "2.20", "cccccccc-cccc-cccc-cccc-cccccccccccc");
    let descriptor_for_hook = descriptor_path.clone();
    let concurrent_for_hook = concurrent.clone();
    let bin_before = fs::read(&fixture.bin_path).unwrap();

    let error = with_before_commit_hook(
        move |_| fs::write(&descriptor_for_hook, &concurrent_for_hook).unwrap(),
        || edit_support_result(&fixture.set_editable_args(&target), &fixture.context),
    )
    .unwrap_err();

    assert!(error.contains("read guard"), "{error}");
    assert_eq!(fs::read_to_string(&descriptor_path).unwrap(), concurrent);
    assert_eq!(fs::read(&fixture.bin_path).unwrap(), bin_before);
}

#[test]
fn support_edit_rejects_a_concurrent_earlier_uuid_probe_change() {
    let fixture = SupportFixture::new("uuid-probe-race", "2.20");
    let form = fixture
        .root
        .join("src/Catalogs/Items/Forms/Item/Ext/Form.xml");
    let wrapper = fixture.root.join("src/Catalogs/Items/Forms/Item.xml");
    fs::create_dir_all(form.parent().unwrap()).unwrap();
    fs::write(
        &form,
        r#"<Form xmlns="http://v8.1c.ru/8.3/xcf/logform" version="2.20"/>"#,
    )
    .unwrap();
    fs::write(
        &wrapper,
        metadata_object_xml("Item", "2.20", "bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb"),
    )
    .unwrap();
    let concurrent = r#"<Form xmlns="http://v8.1c.ru/8.3/xcf/logform" version="2.20"><!-- concurrent --></Form>"#;
    let form_for_hook = form.clone();
    let bin_before = fs::read(&fixture.bin_path).unwrap();

    let error = with_before_commit_hook(
        move |_| fs::write(&form_for_hook, concurrent).unwrap(),
        || edit_support_result(&fixture.set_editable_args(&form), &fixture.context),
    )
    .unwrap_err();

    assert!(error.contains("read guard"), "{error}");
    assert_eq!(fs::read_to_string(&form).unwrap(), concurrent);
    assert_eq!(fs::read(&fixture.bin_path).unwrap(), bin_before);
}

#[test]
fn support_edit_does_not_fall_back_past_an_xml_descriptor_without_uuid() {
    let fixture = SupportFixture::new("uuid-missing", "2.20");
    let target = fixture.object_target("Items", "2.20", "bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb");
    let descriptor_path = fixture.root.join("src/Catalogs/Items.xml");
    fs::write(
        &descriptor_path,
        metadata_object_xml_without_uuid("Items", "2.20"),
    )
    .unwrap();
    let bin_before = fs::read(&fixture.bin_path).unwrap();

    let error =
        edit_support_result(&fixture.set_editable_args(&target), &fixture.context).unwrap_err();

    assert!(
        error.contains("does not contain a metadata UUID"),
        "{error}"
    );
    assert_eq!(fs::read(&fixture.bin_path).unwrap(), bin_before);
}

#[test]
fn support_edit_refuses_a_newer_configuration_owner_without_writing_bin() {
    let fixture = SupportFixture::new("newer-owner", "2.21");
    let bin_before = fs::read(&fixture.bin_path).unwrap();

    let error = edit_support_result(&fixture.capability_off_args(), &fixture.context).unwrap_err();

    assert!(error.contains("newer than supported 2.20"), "{error}");
    assert!(error.contains("1C 8.5 support is planned"), "{error}");
    assert_eq!(fs::read(&fixture.bin_path).unwrap(), bin_before);
}

#[test]
fn support_capability_uses_direct_object_xml_only_as_a_config_locator() {
    let fixture = SupportFixture::new("capability-object-locator", "2.20");
    let target = fixture.object_target("Items", "2.21", "bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb");
    let descriptor = fixture.root.join("src/Catalogs/Items.xml");
    let args = json!({
        "Path": descriptor.display().to_string(),
        "Capability": "off"
    })
    .as_object()
    .unwrap()
    .clone();

    let outcome = edit_support_result(&args, &fixture.context).unwrap();

    assert!(outcome.ok, "{outcome:?}");
    assert!(fs::read_to_string(&fixture.bin_path)
        .unwrap()
        .contains("{6,1,"));
    assert!(target.exists());
}

#[test]
fn support_edit_refuses_a_newer_nearest_uuid_descriptor_without_writing_bin() {
    let fixture = SupportFixture::new("newer-uuid-descriptor", "2.20");
    let target = fixture.object_target("Items", "2.21", "bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb");
    let bin_before = fs::read(&fixture.bin_path).unwrap();

    let error =
        edit_support_result(&fixture.set_editable_args(&target), &fixture.context).unwrap_err();

    assert!(error.contains("newer than supported 2.20"), "{error}");
    assert!(error.contains("1C 8.5 support is planned"), "{error}");
    assert_eq!(fs::read(&fixture.bin_path).unwrap(), bin_before);
}

#[test]
fn support_edit_ignores_a_newer_unrelated_neighbor_descriptor() {
    let fixture = SupportFixture::new("unrelated-newer-descriptor", "2.20");
    let target = fixture.object_target("Items", "2.20", "bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb");
    fixture.object_target("Unrelated", "2.21", "cccccccc-cccc-cccc-cccc-cccccccccccc");

    let outcome =
        edit_support_result(&fixture.set_editable_args(&target), &fixture.context).unwrap();

    assert!(outcome.ok, "{outcome:?}");
    let updated = fs::read_to_string(&fixture.bin_path).unwrap();
    assert!(
        updated.contains("1,0,bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb"),
        "{updated}"
    );
}
