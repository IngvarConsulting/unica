//! Ordinary read observations used by the form codec at its existing phases.
use std::path::{Path, PathBuf};
use unica_source_contract::discovery::ProjectDiscoveryRead;
use unica_source_designer::read::path_is_dir;
pub fn resolve_form_info_path(
    reader: &dyn ProjectDiscoveryRead,
    mut form_path: PathBuf,
) -> PathBuf {
    if path_is_dir(reader, &form_path) {
        form_path = form_path.join("Ext").join("Form.xml");
    }
    if !reader.path_is_file(&form_path)
        && form_path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name == "Form.xml")
    {
        let candidate = form_path
            .parent()
            .unwrap_or_else(|| Path::new(""))
            .join("Ext")
            .join("Form.xml");
        if reader.path_is_file(&candidate) {
            form_path = candidate;
        }
    }
    if !reader.path_is_file(&form_path)
        && form_path
            .extension()
            .and_then(|ext| ext.to_str())
            .is_some_and(|ext| ext.eq_ignore_ascii_case("xml"))
    {
        let stem = form_path.file_stem().and_then(|stem| stem.to_str());
        if let (Some(stem), Some(parent)) = (stem, form_path.parent()) {
            let candidate = parent.join(stem).join("Ext").join("Form.xml");
            if reader.path_is_file(&candidate) {
                form_path = candidate;
            }
        }
    }
    form_path
}
pub fn resolve_form_add_object_path(
    reader: &dyn ProjectDiscoveryRead,
    mut object_path: PathBuf,
) -> Result<PathBuf, String> {
    if path_is_dir(reader, &object_path) {
        let dir_name = object_path
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or("")
            .to_string();
        let candidate = object_path.join(format!("{dir_name}.xml"));
        let sibling = object_path
            .parent()
            .map(|parent| parent.join(format!("{dir_name}.xml")))
            .unwrap_or_else(|| PathBuf::from(format!("{dir_name}.xml")));
        if reader.path_is_file(&candidate) {
            object_path = candidate;
        } else if reader.path_is_file(&sibling) {
            object_path = sibling;
        }
    }
    if !reader.path_is_file(&object_path) {
        return Err(format!("Файл объекта не найден: {}", object_path.display()));
    }
    Ok(reader.canonicalize(&object_path).unwrap_or(object_path))
}
