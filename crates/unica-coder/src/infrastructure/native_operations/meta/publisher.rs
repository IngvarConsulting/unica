#![allow(dead_code, unused_imports)]

use super::*;

pub(crate) fn fresh_meta_compile_uuid() -> String {
    uuid::Uuid::new_v4().to_string()
}

pub(crate) fn register_compiled_meta_in_configuration(
    output_dir: &Path,
    child_tag: &str,
    obj_name: &str,
) -> Result<Option<String>, String> {
    metadata_kind(child_tag).ok_or_else(|| format!("Unknown type '{child_tag}'"))?;
    let config_xml_path = output_dir.join("Configuration.xml");
    let mut transaction = CompileTransaction::new();
    let status = transaction.register_canonical_child(&config_xml_path, child_tag, obj_name)?;
    if status == RegistrationStatus::Added {
        transaction.commit()?;
    }
    Ok(Some(
        match status {
            RegistrationStatus::Added => "added",
            RegistrationStatus::AlreadyPresent => "already",
            RegistrationStatus::MissingTarget => "no-config",
        }
        .to_string(),
    ))
}
