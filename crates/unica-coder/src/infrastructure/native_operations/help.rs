use crate::application::AdapterOutcome;
use crate::domain::format_profile::ACTIVE_FORMAT_PROFILE;
use crate::domain::workspace::WorkspaceContext;
use roxmltree::Document;
use serde_json::{Map, Value};
use std::fs;
use std::path::{Component, Path, PathBuf};

use super::common::*;
use super::compile_transaction::CompileTransaction;
use super::meta::require_metadata_8_3_27_validation;
use super::template::template_add_object_type_folders;

struct HelpAddRun {
    stdout: String,
    changes: Vec<String>,
    artifacts: Vec<String>,
    warnings: Vec<String>,
}

pub(crate) fn add_help(args: &Map<String, Value>, context: &WorkspaceContext) -> AdapterOutcome {
    let result = (|| -> Result<HelpAddRun, String> {
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
        let object_path = target.object_dir.with_extension("xml");
        let object_preimage = fs::read(&object_path).map_err(|error| {
            format!(
                "failed to read metadata owner {}: {error}",
                object_path.display()
            )
        })?;

        let help_xml_path = ext_dir.join("Help.xml");
        let format_version = crate::domain::format_profile::ACTIVE_FORMAT_PROFILE
            .export_format
            .to_string();
        let help_xml = help_metadata_xml(lang, &format_version);
        validate_help_xml(&help_xml_path, &help_xml)?;
        let help_dir = ext_dir.join("Help");
        let help_html_path = help_dir.join(format!("{lang}.html"));
        let help_html = help_page_html(object_name);
        let mut transaction = CompileTransaction::new();
        transaction.create_utf8_bom_text(&help_xml_path, &help_xml)?;
        transaction.create_utf8_bom_text(&help_html_path, &help_html)?;

        let mut stdout = String::new();
        let mut changes = vec![
            format!("created {}", help_xml_path.display()),
            format!("created {}", help_html_path.display()),
        ];
        let mut artifacts = vec![
            help_xml_path.display().to_string(),
            help_html_path.display().to_string(),
        ];
        let mut form_snapshots = Vec::new();
        let forms_dir = target.object_dir.join("Forms");
        if forms_dir.is_dir() {
            let mut entries = fs::read_dir(&forms_dir)
                .map_err(|err| format!("failed to read {}: {err}", forms_dir.display()))?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|err| format!("failed to read entry in {}: {err}", forms_dir.display()))?;
            entries.sort_by_key(|entry| entry.file_name());
            for entry in entries {
                let form_path = entry.path();
                if form_path.extension().and_then(|value| value.to_str()) != Some("xml")
                    || !form_path.is_file()
                {
                    continue;
                }
                let snapshot = read_utf8_sig_snapshot(&form_path)?;
                if let Some(updated) = form_with_include_help(&snapshot.text) {
                    transaction.replace_bytes(
                        &form_path,
                        &snapshot.raw,
                        utf8_bom_bytes(&updated),
                    )?;
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
                form_snapshots.push((form_path, snapshot.raw, snapshot.text));
            }
        }
        guard_exact_preimage_if_unprotected(&mut transaction, &object_path, &object_preimage)?;
        for (path, preimage, _) in &form_snapshots {
            guard_exact_preimage_if_unprotected(&mut transaction, path, preimage)?;
        }
        let mut format_dependencies = vec![object_path.as_path()];
        format_dependencies.extend(form_snapshots.iter().map(|(path, _, _)| path.as_path()));
        guard_active_format_dependencies(&mut transaction, &format_dependencies, context)?;
        require_metadata_8_3_27_validation(&object_path, context, "help.add")?;
        for (path, _, text) in &form_snapshots {
            validate_help_form_owner_8_3_27(path, text)?;
            validate_help_xml(path, text)?;
        }

        let report = transaction.commit_with_post_validation(|| {
            require_metadata_8_3_27_validation(&object_path, context, "help.add")?;
            for (path, _, _) in &form_snapshots {
                let snapshot = read_utf8_sig_snapshot(path)?;
                validate_help_form_owner_8_3_27(path, &snapshot.text)?;
            }
            Ok(())
        })?;

        stdout.push_str(&format!("[OK] Создана справка: {object_name}\n"));
        stdout.push_str(&format!(
            "     Метаданные: {}\n",
            help_display_path(&help_xml_path, &context.cwd)
        ));
        stdout.push_str(&format!(
            "     Страница:   {}\n",
            help_display_path(&help_html_path, &context.cwd)
        ));

        Ok(HelpAddRun {
            stdout,
            changes,
            artifacts,
            warnings: report.cleanup_warnings,
        })
    })();

    match result {
        Ok(HelpAddRun {
            stdout,
            changes,
            artifacts,
            warnings,
        }) => AdapterOutcome {
            ok: true,
            summary: "unica.help.add completed with native help writer".to_string(),
            changes,
            warnings,
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

pub(crate) fn resolve_help_object_dir_for_format_guard(
    src_dir: &Path,
    object_name: &str,
) -> Result<PathBuf, String> {
    resolve_help_target(src_dir, object_name).map(|target| target.object_dir)
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
        "<!DOCTYPE html PUBLIC \"-//W3C//DTD HTML 4.0 Transitional//EN\">\
<html><head>\
<meta http-equiv=\"Content-Type\" content=\"text/html; charset=utf-8\"></meta>\
<link rel=\"stylesheet\" type=\"text/css\" href=\"v8help://service_book/service_style\"></link>\
</head><body>\n    <h1>{}</h1>\n    <p>Описание.</p>\n</body></html>",
        escape_xml(object_name)
    )
}

fn validate_help_xml(path: &Path, text: &str) -> Result<(), String> {
    Document::parse(text.trim_start_matches('\u{feff}'))
        .map(|_| ())
        .map_err(|error| format!("XML parse error in {}: {error}", path.display()))
}

fn validate_help_form_owner_8_3_27(path: &Path, text: &str) -> Result<(), String> {
    const MD_NS: &str = "http://v8.1c.ru/8.3/MDClasses";

    let document = Document::parse(text.trim_start_matches('\u{feff}'))
        .map_err(|error| format!("XML parse error in {}: {error}", path.display()))?;
    let root = document.root_element();
    if root.tag_name().name() != "MetaDataObject" || root.tag_name().namespace() != Some(MD_NS) {
        return Err(format!(
            "help.add form owner {} must have the fixed 8.3.27 MetaDataObject root",
            path.display()
        ));
    }
    if root.attribute("version") != Some(ACTIVE_FORMAT_PROFILE.export_format) {
        return Err(format!(
            "help.add form owner {} must use export format {}",
            path.display(),
            ACTIVE_FORMAT_PROFILE.export_format
        ));
    }

    let forms = root
        .children()
        .filter(|node| {
            node.is_element()
                && node.tag_name().namespace() == Some(MD_NS)
                && node.tag_name().name() == "Form"
        })
        .collect::<Vec<_>>();
    if forms.len() != 1 {
        return Err(format!(
            "help.add form owner {} must contain exactly one MDClasses Form element",
            path.display()
        ));
    }
    let form = forms[0];
    let properties = form
        .children()
        .find(|node| {
            node.is_element()
                && node.tag_name().namespace() == Some(MD_NS)
                && node.tag_name().name() == "Properties"
        })
        .ok_or_else(|| {
            format!(
                "help.add form owner {} is missing Form/Properties",
                path.display()
            )
        })?;
    let property = |name: &str| {
        properties.children().find(|node| {
            node.is_element()
                && node.tag_name().namespace() == Some(MD_NS)
                && node.tag_name().name() == name
        })
    };

    let form_type = property("FormType")
        .and_then(|node| node.text())
        .unwrap_or("");
    if !matches!(form_type, "Managed" | "Ordinary") {
        return Err(format!(
            "help.add form owner {} property Form.FormType value '{form_type}' is not valid for the fixed 8.3.27 contract; expected Managed or Ordinary",
            path.display()
        ));
    }
    if let Some(include_help) = property("IncludeHelpInContents") {
        let value = include_help.text().unwrap_or("");
        if !matches!(value, "true" | "false") {
            return Err(format!(
                "help.add form owner {} property Form.IncludeHelpInContents value '{value}' is not a canonical xs:boolean for the fixed 8.3.27 contract; expected true or false",
                path.display()
            ));
        }
    }

    Ok(())
}

fn form_with_include_help(text: &str) -> Option<String> {
    if text.contains("<IncludeHelpInContents>") {
        return None;
    }
    let insert_at = text
        .find("</FormType>")
        .map(|index| index + "</FormType>".len())?;
    let mut updated = text.to_string();
    updated.insert_str(
        insert_at,
        "\n\t\t\t<IncludeHelpInContents>false</IncludeHelpInContents>",
    );
    if !updated.ends_with('\n') {
        updated.push('\n');
    }
    Some(updated)
}

#[cfg(test)]
mod tests {
    use super::super::compile_transaction::{with_commit_failpoint, CommitFailpoint};
    use super::super::single_file_publisher::with_before_commit_hook;
    use super::*;
    use crate::application::UnicaApplication;
    use serde_json::json;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn normalized_path_text(value: &str) -> String {
        crate::infrastructure::platform::testing::normalize_path_text_for_test(value)
    }

    fn path_text(path: &Path) -> String {
        crate::infrastructure::platform::testing::path_text_for_test(path)
    }

    struct HelpFixture {
        root: PathBuf,
        context: WorkspaceContext,
        ext_dir: PathBuf,
        forms_dir: PathBuf,
    }

    impl HelpFixture {
        fn new(label: &str) -> Self {
            let root = std::env::temp_dir().join(format!(
                "unica-help-{label}-{}-{}",
                std::process::id(),
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            let object_dir = root.join("src/Catalogs/Items");
            let object_path = root.join("src/Catalogs/Items.xml");
            let ext_dir = object_dir.join("Ext");
            let forms_dir = object_dir.join("Forms");
            fs::create_dir_all(&ext_dir).unwrap();
            fs::create_dir_all(&forms_dir).unwrap();
            fs::write(&object_path, object_xml()).unwrap();
            let context = WorkspaceContext {
                cwd: root.clone(),
                workspace_root: root.clone(),
                cache_root: root.join(".build/unica"),
                workspace_epoch: 0,
            };
            Self {
                root,
                context,
                ext_dir,
                forms_dir,
            }
        }

        fn args(&self) -> Map<String, Value> {
            Map::from_iter([
                ("ObjectName".to_string(), json!("Catalogs/Items")),
                ("SrcDir".to_string(), json!("src")),
                ("Lang".to_string(), json!("ru")),
            ])
        }

        fn help_xml(&self) -> PathBuf {
            self.ext_dir.join("Help.xml")
        }

        fn help_html(&self) -> PathBuf {
            self.ext_dir.join("Help/ru.html")
        }
    }

    impl Drop for HelpFixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    fn form_xml(name: &str) -> Vec<u8> {
        format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20">
  <Form uuid="dddddddd-dddd-dddd-dddd-dddddddddddd">
    <Properties>
      <Name>{name}</Name>
      <FormType>Managed</FormType>
    </Properties>
  </Form>
</MetaDataObject>"#
        )
        .into_bytes()
    }

    fn object_xml() -> Vec<u8> {
        r#"<?xml version="1.0" encoding="UTF-8"?>
<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" xmlns:xr="http://v8.1c.ru/8.3/xcf/readable" version="2.20">
  <Catalog uuid="aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa">
    <InternalInfo>
      <xr:GeneratedType name="CatalogObject.Items" category="Object">
        <xr:TypeId>00000000-0000-4000-8000-000000000101</xr:TypeId>
        <xr:ValueId>00000000-0000-4000-8000-000000000102</xr:ValueId>
      </xr:GeneratedType>
      <xr:GeneratedType name="CatalogRef.Items" category="Ref">
        <xr:TypeId>00000000-0000-4000-8000-000000000103</xr:TypeId>
        <xr:ValueId>00000000-0000-4000-8000-000000000104</xr:ValueId>
      </xr:GeneratedType>
      <xr:GeneratedType name="CatalogSelection.Items" category="Selection">
        <xr:TypeId>00000000-0000-4000-8000-000000000105</xr:TypeId>
        <xr:ValueId>00000000-0000-4000-8000-000000000106</xr:ValueId>
      </xr:GeneratedType>
      <xr:GeneratedType name="CatalogList.Items" category="List">
        <xr:TypeId>00000000-0000-4000-8000-000000000107</xr:TypeId>
        <xr:ValueId>00000000-0000-4000-8000-000000000108</xr:ValueId>
      </xr:GeneratedType>
      <xr:GeneratedType name="CatalogManager.Items" category="Manager">
        <xr:TypeId>00000000-0000-4000-8000-000000000109</xr:TypeId>
        <xr:ValueId>00000000-0000-4000-8000-000000000110</xr:ValueId>
      </xr:GeneratedType>
    </InternalInfo>
    <Properties>
      <Name>Items</Name>
    </Properties>
    <ChildObjects/>
  </Catalog>
</MetaDataObject>"#
            .as_bytes()
            .to_vec()
    }

    fn assert_help_absent(fixture: &HelpFixture) {
        assert!(!fixture.help_xml().exists());
        assert!(!fixture.help_html().exists());
        assert!(!fixture.ext_dir.join("Help").exists());
    }

    #[test]
    fn help_form_owner_validator_uses_the_shared_active_export_format() {
        let source = include_str!("help.rs");
        let validator = source
            .split_once("fn validate_help_form_owner_8_3_27")
            .and_then(|(_, tail)| tail.split_once("fn form_with_include_help"))
            .map(|(body, _)| body)
            .expect("help form owner validator source must be present");

        assert!(
            validator.contains("ACTIVE_FORMAT_PROFILE.export_format"),
            "the validator must use the shared active format profile"
        );
        assert!(
            !validator.contains(r#"Some("2.20")"#),
            "the validator must not duplicate the active export-format literal"
        );
    }

    #[test]
    fn help_page_matches_the_8_3_27_export_serialization() {
        assert_eq!(
            help_page_html("Catalogs/CorpusCatalog"),
            concat!(
                "<!DOCTYPE html PUBLIC \"-//W3C//DTD HTML 4.0 Transitional//EN\">",
                "<html><head>",
                "<meta http-equiv=\"Content-Type\" content=\"text/html; charset=utf-8\"></meta>",
                "<link rel=\"stylesheet\" type=\"text/css\" ",
                "href=\"v8help://service_book/service_style\"></link>",
                "</head><body>\n",
                "    <h1>Catalogs/CorpusCatalog</h1>\n",
                "    <p>Описание.</p>\n",
                "</body></html>"
            )
        );
    }

    #[test]
    fn help_add_refuses_preexisting_html_without_creating_or_overwriting_anything() {
        let fixture = HelpFixture::new("preexisting-html");
        fs::create_dir_all(fixture.help_html().parent().unwrap()).unwrap();
        let html_before = b"existing help page".to_vec();
        fs::write(fixture.help_html(), &html_before).unwrap();
        let form_path = fixture.forms_dir.join("Main.xml");
        let form_before = form_xml("Main");
        fs::write(&form_path, &form_before).unwrap();

        let outcome = add_help(&fixture.args(), &fixture.context);

        assert!(!outcome.ok, "{outcome:?}");
        assert!(
            normalized_path_text(&outcome.errors.join("\n"))
                .contains(&path_text(&fixture.help_html())),
            "{outcome:?}"
        );
        assert!(!fixture.help_xml().exists(), "{outcome:?}");
        assert_eq!(fs::read(fixture.help_html()).unwrap(), html_before);
        assert_eq!(fs::read(form_path).unwrap(), form_before);
        assert!(outcome.changes.is_empty(), "{outcome:?}");
        assert!(outcome.artifacts.is_empty(), "{outcome:?}");
    }

    #[test]
    fn help_add_rejects_malformed_form_xml_before_creating_help_files() {
        let fixture = HelpFixture::new("malformed-form");
        let form_path = fixture.forms_dir.join("Broken.xml");
        let form_before = b"<MetaDataObject><FormType>Managed</FormType>".to_vec();
        fs::write(&form_path, &form_before).unwrap();

        let outcome = add_help(&fixture.args(), &fixture.context);

        assert!(!outcome.ok, "{outcome:?}");
        let errors = normalized_path_text(&outcome.errors.join("\n"));
        assert!(errors.contains("XML parse error"), "{outcome:?}");
        assert!(errors.contains(&path_text(&form_path)), "{outcome:?}");
        assert_help_absent(&fixture);
        assert_eq!(fs::read(form_path).unwrap(), form_before);
        assert!(outcome.changes.is_empty(), "{outcome:?}");
        assert!(outcome.artifacts.is_empty(), "{outcome:?}");
    }

    #[test]
    fn help_add_rejects_platform_invalid_form_owner_without_any_byte_changes_or_orphans() {
        let fixture = HelpFixture::new("invalid-form-owner-enum");
        let form_path = fixture.forms_dir.join("BrokenEnum.xml");
        let form_before = String::from_utf8(form_xml("BrokenEnum"))
            .unwrap()
            .replace("<FormType>Managed</FormType>", "<FormType>Bogus</FormType>")
            .into_bytes();
        fs::write(&form_path, &form_before).unwrap();

        let outcome = add_help(&fixture.args(), &fixture.context);

        assert!(!outcome.ok, "{outcome:?}");
        let errors = outcome.errors.join("\n");
        assert!(errors.contains("FormType"), "{outcome:?}");
        assert!(errors.contains("Bogus"), "{outcome:?}");
        assert_help_absent(&fixture);
        assert_eq!(fs::read(form_path).unwrap(), form_before);
        assert!(outcome.changes.is_empty(), "{outcome:?}");
        assert!(outcome.artifacts.is_empty(), "{outcome:?}");
    }

    #[test]
    fn help_add_late_unreadable_form_leaves_no_partial_writes() {
        let fixture = HelpFixture::new("unreadable-form");
        let valid_path = fixture.forms_dir.join("Valid.xml");
        let valid_before = form_xml("Valid");
        fs::write(&valid_path, &valid_before).unwrap();
        let invalid_path = fixture.forms_dir.join("Unreadable.xml");
        let invalid_before = vec![0xff, 0xfe, 0xfd];
        fs::write(&invalid_path, &invalid_before).unwrap();

        let outcome = add_help(&fixture.args(), &fixture.context);

        assert!(!outcome.ok, "{outcome:?}");
        let errors = normalized_path_text(&outcome.errors.join("\n"));
        assert!(errors.contains("valid UTF-8"), "{outcome:?}");
        assert!(errors.contains(&path_text(&invalid_path)), "{outcome:?}");
        assert_help_absent(&fixture);
        assert_eq!(fs::read(valid_path).unwrap(), valid_before);
        assert_eq!(fs::read(invalid_path).unwrap(), invalid_before);
        assert!(outcome.changes.is_empty(), "{outcome:?}");
        assert!(outcome.artifacts.is_empty(), "{outcome:?}");
    }

    #[test]
    fn help_add_post_write_failure_rolls_back_help_files_and_all_form_replacements() {
        let fixture = HelpFixture::new("rollback");
        let form_a = fixture.forms_dir.join("A.xml");
        let form_b = fixture.forms_dir.join("B.xml");
        let form_a_before = form_xml("A");
        let form_b_before = form_xml("B");
        fs::write(&form_a, &form_a_before).unwrap();
        fs::write(&form_b, &form_b_before).unwrap();

        let outcome = with_commit_failpoint(CommitFailpoint::PostWriteValidation, || {
            add_help(&fixture.args(), &fixture.context)
        });

        assert!(!outcome.ok, "{outcome:?}");
        assert!(
            outcome.errors.join("\n").contains("post-write validation"),
            "{outcome:?}"
        );
        assert_help_absent(&fixture);
        assert_eq!(fs::read(form_a).unwrap(), form_a_before);
        assert_eq!(fs::read(form_b).unwrap(), form_b_before);
        assert!(outcome.changes.is_empty(), "{outcome:?}");
        assert!(outcome.artifacts.is_empty(), "{outcome:?}");
    }

    #[test]
    fn help_add_rejects_concurrent_source_set_format_owner_change() {
        let fixture = HelpFixture::new("source-set-owner-guard");
        fs::write(
            fixture.root.join("v8project.yaml"),
            "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
        )
        .unwrap();
        let owner = fixture.root.join("src/Configuration.xml");
        fs::write(
            &owner,
            r#"<?xml version="1.0" encoding="UTF-8"?>
<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20">
  <Configuration uuid="55555555-5555-5555-5555-555555555555"/>
</MetaDataObject>
"#,
        )
        .unwrap();
        let concurrent_owner = fs::read_to_string(&owner)
            .unwrap()
            .replacen(r#"version="2.20""#, r#"version="2.21""#, 1)
            .into_bytes();
        let owner_for_hook = owner.clone();
        let concurrent_for_hook = concurrent_owner.clone();

        let outcome = with_before_commit_hook(
            move |_| fs::write(&owner_for_hook, concurrent_for_hook).unwrap(),
            || add_help(&fixture.args(), &fixture.context),
        );

        assert!(!outcome.ok, "{outcome:?}");
        assert!(
            outcome.errors.join("\n").contains("read guard"),
            "{outcome:?}"
        );
        assert_eq!(fs::read(&owner).unwrap(), concurrent_owner);
        assert_help_absent(&fixture);
        assert!(outcome.changes.is_empty(), "{outcome:?}");
        assert!(outcome.artifacts.is_empty(), "{outcome:?}");
    }

    #[test]
    fn help_add_rejects_concurrent_object_descriptor_change() {
        let fixture = HelpFixture::new("object-descriptor-guard");
        fs::write(
            fixture.root.join("v8project.yaml"),
            "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
        )
        .unwrap();
        fs::write(
            fixture.root.join("src/Configuration.xml"),
            r#"<?xml version="1.0" encoding="UTF-8"?>
<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20">
  <Configuration uuid="55555555-5555-5555-5555-555555555555"/>
</MetaDataObject>
"#,
        )
        .unwrap();
        let object_path = fixture.root.join("src/Catalogs/Items.xml");
        let concurrent_object = fs::read_to_string(&object_path)
            .unwrap()
            .replacen(
                "</MetaDataObject>",
                "<!-- concurrent -->\n</MetaDataObject>",
                1,
            )
            .into_bytes();
        let object_for_hook = object_path.clone();
        let concurrent_for_hook = concurrent_object.clone();

        let outcome = with_before_commit_hook(
            move |_| fs::write(&object_for_hook, concurrent_for_hook).unwrap(),
            || add_help(&fixture.args(), &fixture.context),
        );

        assert!(!outcome.ok, "{outcome:?}");
        assert!(
            outcome.errors.join("\n").contains("read guard"),
            "{outcome:?}"
        );
        assert_eq!(fs::read(&object_path).unwrap(), concurrent_object);
        assert_help_absent(&fixture);
        assert!(outcome.changes.is_empty(), "{outcome:?}");
        assert!(outcome.artifacts.is_empty(), "{outcome:?}");
    }

    #[test]
    fn help_add_exact_binds_an_unchanged_form_used_for_the_decision() {
        let fixture = HelpFixture::new("unchanged-form-guard");
        let form_path = fixture.forms_dir.join("AlreadyIncluded.xml");
        let form_before = String::from_utf8(form_xml("AlreadyIncluded"))
            .unwrap()
            .replace(
                "</FormType>",
                "</FormType>\n      <IncludeHelpInContents>false</IncludeHelpInContents>",
            )
            .into_bytes();
        fs::write(&form_path, &form_before).unwrap();
        let concurrent_form = String::from_utf8(form_before)
            .unwrap()
            .replace(
                "</Properties>",
                "<Comment>Concurrent</Comment>\n    </Properties>",
            )
            .into_bytes();
        let form_for_hook = form_path.clone();
        let concurrent_for_hook = concurrent_form.clone();

        let outcome = with_before_commit_hook(
            move |_| fs::write(&form_for_hook, &concurrent_for_hook).unwrap(),
            || add_help(&fixture.args(), &fixture.context),
        );

        assert!(!outcome.ok, "{outcome:?}");
        assert!(
            outcome.errors.join("\n").contains("read guard"),
            "{outcome:?}"
        );
        assert_eq!(fs::read(&form_path).unwrap(), concurrent_form);
        assert_help_absent(&fixture);
        assert!(outcome.changes.is_empty(), "{outcome:?}");
        assert!(outcome.artifacts.is_empty(), "{outcome:?}");
    }

    #[test]
    fn help_add_prioritizes_a_newer_form_over_an_older_form() {
        let fixture = HelpFixture::new("mixed-form-versions");
        let older = String::from_utf8(form_xml("Older"))
            .unwrap()
            .replacen(r#"version="2.20""#, r#"version="2.19""#, 1)
            .into_bytes();
        let newer = String::from_utf8(form_xml("Newer"))
            .unwrap()
            .replacen(r#"version="2.20""#, r#"version="2.21""#, 1)
            .into_bytes();
        fs::write(fixture.forms_dir.join("AOlder.xml"), &older).unwrap();
        fs::write(fixture.forms_dir.join("BNewer.xml"), &newer).unwrap();

        let outcome = add_help(&fixture.args(), &fixture.context);

        assert!(!outcome.ok, "{outcome:?}");
        let errors = outcome.errors.join("\n");
        assert!(errors.contains("newer than supported 2.20"), "{outcome:?}");
        assert!(errors.contains("1C 8.5 support is planned"), "{outcome:?}");
        assert!(!errors.contains("re-export the source"), "{outcome:?}");
        assert_help_absent(&fixture);
        assert_eq!(
            fs::read(fixture.forms_dir.join("AOlder.xml")).unwrap(),
            older
        );
        assert_eq!(
            fs::read(fixture.forms_dir.join("BNewer.xml")).unwrap(),
            newer
        );
    }

    #[test]
    fn public_help_add_prioritizes_newer_form_owner_over_older_object_owner() {
        let fixture = HelpFixture::new("public-mixed-owner-versions");
        fs::write(
            fixture.root.join("v8project.yaml"),
            "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
        )
        .unwrap();
        let configuration_path = fixture.root.join("src/Configuration.xml");
        let configuration = r#"<?xml version="1.0" encoding="UTF-8"?>
<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20">
  <Configuration uuid="55555555-5555-4555-8555-555555555555"/>
</MetaDataObject>
"#
        .as_bytes()
        .to_vec();
        fs::write(&configuration_path, &configuration).unwrap();

        let object_path = fixture.root.join("src/Catalogs/Items.xml");
        let older_object = String::from_utf8(object_xml())
            .unwrap()
            .replacen(r#"version="2.20""#, r#"version="2.19""#, 1)
            .into_bytes();
        fs::write(&object_path, &older_object).unwrap();
        let form_path = fixture.forms_dir.join("Main.xml");
        let newer_form = String::from_utf8(form_xml("Main"))
            .unwrap()
            .replacen(r#"version="2.20""#, r#"version="2.21""#, 1)
            .into_bytes();
        fs::write(&form_path, &newer_form).unwrap();

        let mut args = fixture.args();
        args.insert(
            "cwd".to_string(),
            json!(fixture.context.cwd.display().to_string()),
        );
        args.insert("dryRun".to_string(), json!(false));

        let outcome = UnicaApplication::new()
            .call_tool("unica.help.add", &args)
            .unwrap();

        assert!(!outcome.ok, "{outcome:?}");
        let diagnostic = &outcome.diagnostics.as_ref().unwrap()["formatCompatibility"];
        assert_eq!(diagnostic["code"], "platformVersionUnsupported");
        assert_eq!(diagnostic["actualFormat"], "2.21");
        let warning = outcome.warnings.join("\n");
        assert!(warning.contains("1С 8.5"), "{warning}");
        assert!(!warning.contains("миграц"), "{warning}");
        assert!(!warning.contains("повторно выгруз"), "{warning}");
        assert!(!warning.contains("re-export"), "{warning}");
        assert_eq!(fs::read(&configuration_path).unwrap(), configuration);
        assert_eq!(fs::read(&object_path).unwrap(), older_object);
        assert_eq!(fs::read(&form_path).unwrap(), newer_form);
        assert_help_absent(&fixture);
        assert!(outcome.changes.is_empty(), "{outcome:?}");
        assert!(outcome.artifacts.is_empty(), "{outcome:?}");
    }
}
