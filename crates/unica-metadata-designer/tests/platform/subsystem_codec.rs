use super::*;
#[test]
fn subsystem_edit_emits_platform_canonical_utf8_declaration() {
    let xml = emit_subsystem_edit_model(&SubsystemEditModel {
        version: "2.20".to_string(),
        uuid: "11111111-2222-4333-8444-555555555555".to_string(),
        name: "CanonicalEncoding".to_string(),
        synonym: String::new(),
        comment: String::new(),
        include_help: "true".to_string(),
        include_ci: "true".to_string(),
        use_one_command: "false".to_string(),
        explanation: String::new(),
        picture: String::new(),
        picture_load_transparent: None,
        content: Vec::new(),
        children: Vec::new(),
    });

    assert!(
        xml.starts_with("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n"),
        "subsystem XML must use the platform-canonical declaration: {}",
        xml.lines().next().unwrap_or_default()
    );
}
