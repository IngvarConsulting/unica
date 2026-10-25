//! Pure descriptor planning for an object's managed form borrowed into an
//! extension. The bytes follow the 8.3.27 export in
//! `tests/fixtures/platform_8_3_27/cfe_borrow/` (#1982): without
//! `FormType` the platform silently drops the form's own `Form.xml`.

use roxmltree::{Document, Node};

const MD: &str = "http://v8.1c.ru/8.3/MDClasses";

/// Identity of a parent form proven fit for borrowing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParentForm {
    pub uuid: String,
    pub version: String,
}

/// Why a form descriptor cannot take part in borrowing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BorrowFormError {
    /// The parent form is not a managed form.
    NotManaged,
    /// The descriptor is damaged or describes another object.
    InvalidSource(String),
}

impl std::fmt::Display for BorrowFormError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotManaged => formatter.write_str("only a managed form can be borrowed"),
            Self::InvalidSource(message) => formatter.write_str(message),
        }
    }
}

fn form<'a>(document: &'a Document<'a>, name: &str) -> Result<Node<'a, 'a>, BorrowFormError> {
    let invalid = |message: &str| BorrowFormError::InvalidSource(message.to_string());
    let root = document.root_element();
    let mut objects = root.children().filter(Node::is_element);
    let form = objects
        .next()
        .filter(|node| root.has_tag_name((MD, "MetaDataObject")) && node.has_tag_name((MD, "Form")))
        .ok_or_else(|| invalid("form descriptor must contain one MDClasses Form"))?;
    if objects.next().is_some() {
        return Err(invalid("form descriptor must contain one MDClasses Form"));
    }
    if property(form, "Name").as_deref() != Some(name) {
        return Err(invalid("form descriptor name differs from its address"));
    }
    Ok(form)
}

fn property(form: Node<'_, '_>, name: &str) -> Option<String> {
    form.children()
        .find(|node| node.has_tag_name((MD, "Properties")))?
        .children()
        .find(|node| node.has_tag_name((MD, name)))
        .map(|node| node.text().unwrap_or_default().to_string())
}

fn parse(bytes: &[u8]) -> Result<&str, BorrowFormError> {
    std::str::from_utf8(bytes)
        .map(|text| text.trim_start_matches('\u{feff}'))
        .map_err(|_| BorrowFormError::InvalidSource("form descriptor is not UTF-8".to_string()))
}

fn uuid(value: Option<&str>) -> Result<String, BorrowFormError> {
    value
        .and_then(|value| uuid::Uuid::parse_str(value).ok())
        .filter(|value| !value.is_nil())
        .map(|value| value.hyphenated().to_string())
        .ok_or_else(|| BorrowFormError::InvalidSource("form descriptor has no valid uuid".into()))
}

/// Reads the parent form descriptor `name` and proves it is a managed form.
pub fn parent_form(bytes: &[u8], name: &str) -> Result<ParentForm, BorrowFormError> {
    let text = parse(bytes)?;
    let document =
        Document::parse(text).map_err(|error| BorrowFormError::InvalidSource(error.to_string()))?;
    let form = form(&document, name)?;
    if property(form, "FormType").as_deref() != Some("Managed") {
        return Err(BorrowFormError::NotManaged);
    }
    Ok(ParentForm {
        uuid: uuid(form.attribute("uuid"))?,
        version: document
            .root_element()
            .attribute("version")
            .ok_or_else(|| BorrowFormError::InvalidSource("form descriptor has no version".into()))?
            .to_string(),
    })
}

/// An extension form descriptor: the parent uuid it borrows from (`None`
/// when the extension owns the form) and whether it keeps `FormType`
/// `Managed`, without which 8.3.27 drops the form's own `Form.xml`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtensionForm {
    pub parent: Option<String>,
    pub managed: bool,
}

pub fn extension_form(bytes: &[u8], name: &str) -> Result<ExtensionForm, BorrowFormError> {
    let text = parse(bytes)?;
    let document =
        Document::parse(text).map_err(|error| BorrowFormError::InvalidSource(error.to_string()))?;
    let form = form(&document, name)?;
    Ok(ExtensionForm {
        parent: property(form, "ExtendedConfigurationObject")
            .map(|value| uuid(Some(&value)))
            .transpose()?,
        managed: property(form, "FormType").as_deref() == Some("Managed"),
    })
}

/// Descriptor of the borrowed form exactly as 8.3.27 exports it: BOM, CRLF,
/// `PropertyState Form/Extended` and the mandatory `FormType`.
pub fn borrowed_form_descriptor(name: &str, uuid: &str, parent: &ParentForm) -> Vec<u8> {
    let escape = unica_xml_format::text::escape_xml;
    let text = [
        "\u{feff}<?xml version=\"1.0\" encoding=\"UTF-8\"?>".to_string(),
        format!(
            "<MetaDataObject {} version=\"{}\">",
            unica_source_designer::layout::full_md_namespace_declarations(),
            escape(&parent.version)
        ),
        format!("\t<Form uuid=\"{}\">", escape(uuid)),
        "\t\t<InternalInfo>".to_string(),
        "\t\t\t<xr:PropertyState>".to_string(),
        "\t\t\t\t<xr:Property>Form</xr:Property>".to_string(),
        "\t\t\t\t<xr:State>Extended</xr:State>".to_string(),
        "\t\t\t</xr:PropertyState>".to_string(),
        "\t\t</InternalInfo>".to_string(),
        "\t\t<Properties>".to_string(),
        "\t\t\t<ObjectBelonging>Adopted</ObjectBelonging>".to_string(),
        format!("\t\t\t<Name>{}</Name>", escape(name)),
        "\t\t\t<Comment/>".to_string(),
        format!(
            "\t\t\t<ExtendedConfigurationObject>{}</ExtendedConfigurationObject>",
            escape(&parent.uuid)
        ),
        "\t\t\t<FormType>Managed</FormType>".to_string(),
        "\t\t</Properties>".to_string(),
        "\t</Form>".to_string(),
        "</MetaDataObject>".to_string(),
    ]
    .join("\r\n");
    text.into_bytes()
}
