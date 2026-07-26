use roxmltree::Document;
use serde_json::{Map, Value};
use std::path::{Path, PathBuf};
use unica_coder::application::UnicaApplication;
use uuid::Uuid;

const ACTIVE_EXPORT_FORMAT: &str = "2.20";
const PLATFORM_COMPATIBILITY_MODE: &str = "Version8_3_27";

struct TempWorkspace(PathBuf);

impl TempWorkspace {
    fn new(label: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "unica-cf-init-platform-contract-{label}-{}-{}",
            std::process::id(),
            Uuid::new_v4()
        ));
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempWorkspace {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn call_cf_init(workspace: &Path, output_dir: &str, compatibility_mode: Option<&str>) -> PathBuf {
    let mut args = Map::new();
    args.insert(
        "cwd".to_string(),
        Value::String(workspace.display().to_string()),
    );
    args.insert("dryRun".to_string(), Value::Bool(false));
    args.insert(
        "Name".to_string(),
        Value::String(format!("PlatformContract{}", output_dir.replace('/', "_"))),
    );
    args.insert(
        "OutputDir".to_string(),
        Value::String(output_dir.to_string()),
    );
    if let Some(compatibility_mode) = compatibility_mode {
        args.insert(
            "CompatibilityMode".to_string(),
            Value::String(compatibility_mode.to_string()),
        );
    }

    let result = UnicaApplication::new()
        .call_tool("unica.cf.init", &args)
        .unwrap();
    assert!(result.ok, "{:?}", result.errors);
    workspace.join(output_dir)
}

fn read_xml(path: &Path) -> String {
    std::fs::read_to_string(path)
        .unwrap()
        .trim_start_matches('\u{feff}')
        .to_string()
}

fn configuration_uuid(source_root: &Path) -> Uuid {
    let xml = read_xml(&source_root.join("Configuration.xml"));
    let document = Document::parse(&xml).unwrap();
    let configuration = document
        .descendants()
        .find(|node| node.has_tag_name("Configuration"))
        .expect("Configuration element");
    Uuid::parse_str(
        configuration
            .attribute("uuid")
            .expect("Configuration/@uuid"),
    )
    .unwrap()
}

fn configuration_property(source_root: &Path, property: &str) -> Option<String> {
    let xml = read_xml(&source_root.join("Configuration.xml"));
    let document = Document::parse(&xml).unwrap();
    document
        .descendants()
        .find(|node| node.has_tag_name(property))
        .and_then(|node| node.text())
        .map(str::to_string)
}

fn mobile_functionality(source_root: &Path, functionality: &str) -> Option<String> {
    let xml = read_xml(&source_root.join("Configuration.xml"));
    let document = Document::parse(&xml).unwrap();
    let name = document.descendants().find(|node| {
        node.tag_name().name() == "functionality" && node.text() == Some(functionality)
    })?;
    name.parent()?
        .children()
        .find(|node| node.tag_name().name() == "use")
        .and_then(|node| node.text())
        .map(str::to_string)
}

fn root_version(path: &Path) -> Option<String> {
    let xml = read_xml(path);
    let document = Document::parse(&xml).unwrap();
    document
        .root_element()
        .attribute("version")
        .map(str::to_string)
}

#[test]
fn public_cf_init_uses_non_nil_configuration_uuid() {
    let workspace = TempWorkspace::new("non-nil-uuid");
    let source_root = call_cf_init(workspace.path(), "src", None);

    assert!(!configuration_uuid(&source_root).is_nil());
}

#[test]
fn public_cf_init_uses_fresh_configuration_uuid_per_call() {
    let workspace = TempWorkspace::new("fresh-uuid");
    let first = call_cf_init(workspace.path(), "first", None);
    let second = call_cf_init(workspace.path(), "second", None);

    assert_ne!(configuration_uuid(&first), configuration_uuid(&second));
}

#[test]
fn public_cf_init_defaults_extension_compatibility_to_8_3_27() {
    let workspace = TempWorkspace::new("default-compatibility");
    let source_root = call_cf_init(workspace.path(), "src", None);

    assert_eq!(
        configuration_property(&source_root, "ConfigurationExtensionCompatibilityMode").as_deref(),
        Some(PLATFORM_COMPATIBILITY_MODE)
    );
}

#[test]
fn public_cf_init_includes_text_to_speech_disabled() {
    let workspace = TempWorkspace::new("text-to-speech");
    let source_root = call_cf_init(workspace.path(), "src", None);

    assert_eq!(
        mobile_functionality(&source_root, "TextToSpeech").as_deref(),
        Some("false")
    );
}

#[test]
fn public_cf_init_keeps_version_on_owner_xml_only() {
    let workspace = TempWorkspace::new("version-ownership");
    let source_root = call_cf_init(workspace.path(), "src", None);

    assert_eq!(
        root_version(&source_root.join("Configuration.xml")).as_deref(),
        Some(ACTIVE_EXPORT_FORMAT)
    );
    assert_eq!(
        root_version(&source_root.join("Languages/Русский.xml")).as_deref(),
        Some(ACTIVE_EXPORT_FORMAT)
    );
    assert_eq!(
        root_version(&source_root.join("Ext/ClientApplicationInterface.xml")),
        None
    );
}

#[test]
fn public_cf_init_preserves_explicit_compatibility_mode() {
    let workspace = TempWorkspace::new("explicit-compatibility");
    let source_root = call_cf_init(workspace.path(), "src", Some("Version8_3_24"));

    assert_eq!(
        configuration_property(&source_root, "ConfigurationExtensionCompatibilityMode").as_deref(),
        Some("Version8_3_24")
    );
    assert_eq!(
        configuration_property(&source_root, "CompatibilityMode").as_deref(),
        Some("Version8_3_24")
    );
}

#[test]
#[ignore = "writes a retained source tree for the manual 8.3.27 ibcmd checkpoint"]
fn public_cf_init_writes_platform_checkpoint_source() {
    let workspace = PathBuf::from(
        std::env::var_os("UNICA_CF_INIT_PLATFORM_EVIDENCE_DIR")
            .expect("UNICA_CF_INIT_PLATFORM_EVIDENCE_DIR"),
    );
    assert!(workspace.is_absolute(), "evidence path must be absolute");
    std::fs::create_dir_all(&workspace).unwrap();
    assert!(
        std::fs::read_dir(&workspace).unwrap().next().is_none(),
        "evidence path must be empty: {}",
        workspace.display()
    );

    let source_root = call_cf_init(&workspace, "source", None);
    println!("CF_INIT_PLATFORM_SOURCE={}", source_root.display());
}
