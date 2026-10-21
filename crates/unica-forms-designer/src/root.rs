//! Existing managed form XML root and lexical opening-tag rules.
pub const FORM_LOGFORM_NS: &str = "http://v8.1c.ru/8.3/xcf/logform";

pub fn require_form_root(root: roxmltree::Node<'_, '_>) -> Result<(), String> {
    let local_name = root.tag_name().name();
    if local_name != "Form" {
        return Err(format!("Root element is '{local_name}', expected 'Form'"));
    }
    let namespace = root.tag_name().namespace().unwrap_or("");
    if namespace != FORM_LOGFORM_NS {
        return Err(format!(
            "Root namespace is '{namespace}', expected '{FORM_LOGFORM_NS}'"
        ));
    }
    Ok(())
}

pub fn form_edit_opening_tag_end(text: &str, start: usize) -> Option<usize> {
    let mut quote = None::<char>;
    for (relative_idx, ch) in text[start..].char_indices() {
        if let Some(quote_ch) = quote {
            if ch == quote_ch {
                quote = None;
            }
            continue;
        }
        match ch {
            '"' | '\'' => quote = Some(ch),
            '>' => return Some(start + relative_idx),
            _ => {}
        }
    }
    None
}
