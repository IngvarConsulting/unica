use crate::application::AdapterOutcome;
use crate::domain::workspace::WorkspaceContext;
use serde_json::{Map, Value};
use std::fs;
use std::path::{Component, Path, PathBuf};

use super::common::*;
use super::template::template_add_object_type_folders;

pub(crate) fn add_help(args: &Map<String, Value>, context: &WorkspaceContext) -> AdapterOutcome {
    let result = (|| -> Result<(String, Vec<String>, Vec<String>), String> {
        let object_name = required_string(
            args,
            &["objectName", "ObjectName", "processorName", "ProcessorName"],
            "ObjectName",
        )?;
        let lang = string_arg(args, &["lang", "Lang", "language", "Language"]).unwrap_or("ru");
        validate_help_lang(lang)?;

        let src_dir = path_arg(args, &["srcDir", "SrcDir"]).unwrap_or_else(|| PathBuf::from("src"));
        let src_dir = absolutize(src_dir, &context.cwd);
        let target = resolve_help_target(&src_dir, object_name)?;
        let ext_dir = target.object_dir.join("Ext");
        if !ext_dir.is_dir() {
            return Err(format!(
                "Каталог объекта не найден: {}. Проверьте путь ObjectName (например Catalogs/МойСправочник).",
                ext_dir.display()
            ));
        }

        let help_xml_path = ext_dir.join("Help.xml");
        if help_xml_path.exists() {
            return Err(format!(
                "Справка уже существует: {}",
                help_xml_path.display()
            ));
        }

        let format_version = detect_format_version(&ext_dir);
        let help_xml = help_metadata_xml(lang, &format_version);
        write_utf8_bom(&help_xml_path, &help_xml)?;

        let help_dir = ext_dir.join("Help");
        fs::create_dir_all(&help_dir)
            .map_err(|err| format!("failed to create {}: {err}", help_dir.display()))?;
        let help_html_path = help_dir.join(format!("{lang}.html"));
        let help_html = help_page_html(object_name);
        write_utf8_bom(&help_html_path, &help_html)?;

        let mut stdout = String::new();
        let mut changes = vec![
            format!("created {}", help_xml_path.display()),
            format!("created {}", help_html_path.display()),
        ];
        let mut artifacts = vec![
            help_xml_path.display().to_string(),
            help_html_path.display().to_string(),
        ];
        let forms_dir = target.object_dir.join("Forms");
        if forms_dir.is_dir() {
            for entry in fs::read_dir(&forms_dir)
                .map_err(|err| format!("failed to read {}: {err}", forms_dir.display()))?
            {
                let entry = entry.map_err(|err| {
                    format!("failed to read entry in {}: {err}", forms_dir.display())
                })?;
                let form_path = entry.path();
                if form_path.extension().and_then(|value| value.to_str()) != Some("xml")
                    || !form_path.is_file()
                {
                    continue;
                }
                let Some(updated) = form_with_include_help(&form_path)? else {
                    continue;
                };
                write_utf8_bom(&form_path, &updated)?;
                let form_name = form_path
                    .file_name()
                    .and_then(|value| value.to_str())
                    .unwrap_or("form.xml");
                stdout.push_str(&format!(
                    "     IncludeHelpInContents добавлен: {form_name}\n"
                ));
                changes.push(format!("updated {}", form_path.display()));
                artifacts.push(form_path.display().to_string());
            }
        }

        stdout.push_str(&format!("[OK] Создана справка: {object_name}\n"));
        stdout.push_str(&format!(
            "     Метаданные: {}\n",
            help_display_path(&help_xml_path, &context.cwd)
        ));
        stdout.push_str(&format!(
            "     Страница:   {}\n",
            help_display_path(&help_html_path, &context.cwd)
        ));

        Ok((stdout, changes, artifacts))
    })();

    match result {
        Ok((stdout, changes, artifacts)) => AdapterOutcome {
            ok: true,
            summary: "unica.help.add completed with native help writer".to_string(),
            changes,
            warnings: Vec::new(),
            errors: Vec::new(),
            artifacts,
            stdout: Some(stdout),
            stderr: None,
            command: None,
        },
        Err(error) => AdapterOutcome {
            ok: false,
            summary: "unica.help.add failed".to_string(),
            changes: Vec::new(),
            warnings: Vec::new(),
            errors: vec![error.clone()],
            artifacts: Vec::new(),
            stdout: None,
            stderr: Some(format!("{error}\n")),
            command: None,
        },
    }
}

fn help_display_path(path: &Path, cwd: &Path) -> String {
    path.strip_prefix(cwd)
        .map(|value| value.display().to_string())
        .unwrap_or_else(|_| path.display().to_string())
}

struct HelpTarget {
    object_dir: PathBuf,
}

fn resolve_help_target(src_dir: &Path, object_name: &str) -> Result<HelpTarget, String> {
    let rel_path = validated_relative_object_path(object_name)?;
    let direct = HelpTarget {
        object_dir: src_dir.join(&rel_path),
    };
    if direct.object_dir.join("Ext").is_dir()
        || src_dir.join(&rel_path).with_extension("xml").is_file()
    {
        return Ok(direct);
    }

    if rel_path.components().count() != 1 {
        return Ok(direct);
    }

    let mut candidates = Vec::new();
    for folder in template_add_object_type_folders() {
        let object_dir = src_dir.join(folder).join(object_name);
        if object_dir.join("Ext").is_dir()
            || src_dir
                .join(folder)
                .join(format!("{object_name}.xml"))
                .is_file()
        {
            candidates.push(HelpTarget { object_dir });
        }
    }
    match candidates.len() {
        0 => Ok(direct),
        1 => Ok(candidates.remove(0)),
        _ => Err(format!(
            "Объект '{object_name}' найден в нескольких подпапках. Укажи ObjectName с типовой папкой, например Catalogs/{object_name}"
        )),
    }
}

fn validated_relative_object_path(object_name: &str) -> Result<PathBuf, String> {
    if object_name.trim().is_empty() {
        return Err("ObjectName is required".to_string());
    }
    if object_name.contains('\\') {
        return Err("ObjectName must use '/' separators, not '\\'".to_string());
    }
    let path = PathBuf::from(object_name);
    if path.is_absolute() {
        return Err("ObjectName must be relative to SrcDir".to_string());
    }
    if path
        .components()
        .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err("ObjectName must not contain '.' or '..' path components".to_string());
    }
    Ok(path)
}

fn validate_help_lang(lang: &str) -> Result<(), String> {
    if lang.trim().is_empty()
        || lang
            .chars()
            .any(|ch| !(ch.is_ascii_alphanumeric() || ch == '_' || ch == '-'))
    {
        return Err("Lang must be a simple language code, for example ru or en".to_string());
    }
    Ok(())
}

fn help_metadata_xml(lang: &str, format_version: &str) -> String {
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
<Help xmlns=\"http://v8.1c.ru/8.3/xcf/extrnprops\" xmlns:xs=\"http://www.w3.org/2001/XMLSchema\" xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\" version=\"{}\">\n\
\t<Page>{}</Page>\n\
</Help>",
        escape_xml(format_version),
        escape_xml(lang)
    )
}

fn help_page_html(object_name: &str) -> String {
    format!(
        "<!DOCTYPE html PUBLIC \"-//W3C//DTD HTML 4.0 Transitional//EN\">\n<html>\n<head>\n    <meta http-equiv=\"Content-Type\" content=\"text/html; charset=utf-8\"/>\n    <link rel=\"stylesheet\" type=\"text/css\" href=\"v8help://service_book/service_style\"/>\n</head>\n<body>\n    <h1>{}</h1>\n    <p>Описание.</p>\n</body>\n</html>",
        escape_xml(object_name)
    )
}

fn form_with_include_help(form_path: &Path) -> Result<Option<String>, String> {
    let text = read_utf8_sig(form_path)?;
    if text.contains("<IncludeHelpInContents>") {
        return Ok(None);
    }
    let Some(insert_at) = text
        .find("</FormType>")
        .map(|index| index + "</FormType>".len())
    else {
        return Ok(None);
    };
    let mut updated = text;
    updated.insert_str(
        insert_at,
        "\n\t\t\t<IncludeHelpInContents>false</IncludeHelpInContents>",
    );
    if !updated.ends_with('\n') {
        updated.push('\n');
    }
    Ok(Some(updated))
}
