//! Closed StyleItem Color observations and local Value edits.
use roxmltree::{Document, Node, NodeId};
use std::path::Path;
use unica_metadata::style_color::{ColorPalette, PaletteColor, StyleItemColor};
use unica_source_contract::portable_path::PortablePathExt;
use unica_source_designer::format_profile::writable_profile_refusal;
use unica_xml_format::{names::xml_ncname_is_valid, read::direct_text};

const MD: &str = "http://v8.1c.ru/8.3/MDClasses";
const UI: &str = "http://v8.1c.ru/8.1/data/ui";
const XSI: &str = "http://www.w3.org/2001/XMLSchema-instance";
const WEB: &str = "http://v8.1c.ru/8.1/data/ui/colors/web";
const STYLE: &str = "http://v8.1c.ru/8.1/data/ui/style";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ColorIssue {
    Invalid(String),
    NotFound(String),
    /// Outside the writable profile, without claiming that the platform
    /// accepts an arbitrary unmodelled spelling.
    ReadOnly(String),
}
impl std::fmt::Display for ColorIssue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Invalid(message) | Self::NotFound(message) | Self::ReadOnly(message) => {
                f.write_str(message)
            }
        }
    }
}
fn invalid(message: impl Into<String>) -> ColorIssue {
    ColorIssue::Invalid(message.into())
}
fn xml_trim(text: &str) -> &str {
    text.trim_matches([' ', '\t', '\r', '\n'])
}

fn single<'a, 'input>(
    parent: Node<'a, 'input>,
    name: &str,
) -> Result<Node<'a, 'input>, ColorIssue> {
    let mut nodes = parent
        .children()
        .filter(|child| child.is_element() && child.tag_name().name() == name);
    let node = nodes
        .next()
        .ok_or_else(|| invalid(format!("missing {name}")))?;
    if nodes.next().is_some() || node.tag_name().namespace() != Some(MD) {
        return Err(invalid(format!(
            "{name} must be a single MDClasses element"
        )));
    }
    Ok(node)
}
fn leaf(node: Node<'_, '_>) -> Result<String, ColorIssue> {
    if node
        .attribute((XSI, "nil"))
        .is_some_and(|nil| !matches!(xml_trim(nil), "false" | "0"))
    {
        return Err(invalid(format!(
            "{} requires an explicit non-nil value",
            node.tag_name().name()
        )));
    }
    if node.children().any(|child| child.is_element()) {
        return Err(invalid(format!(
            "{} must contain text only",
            node.tag_name().name()
        )));
    }
    Ok(direct_text(node))
}
fn expanded<'a>(
    node: Node<'a, '_>,
    text: &'a str,
) -> Result<(Option<&'a str>, &'a str), ColorIssue> {
    let text = xml_trim(text);
    let (prefix, local) = text
        .split_once(':')
        .map_or((None, text), |(prefix, local)| (Some(prefix), local));
    if !xml_ncname_is_valid(local) || prefix.is_some_and(|prefix| !xml_ncname_is_valid(prefix)) {
        return Err(invalid("invalid color QName"));
    }
    let ns = node.lookup_namespace_uri(prefix);
    if prefix.is_some() && ns.is_none() {
        return Err(invalid("color QName has an unbound prefix"));
    }
    Ok((ns, local))
}

/// A Color's semantic value. Unknown bound forms remain readable but are
/// not converted to a writable variant merely because they are QName-shaped.
pub fn color_value(node: Node<'_, '_>) -> Result<StyleItemColor, ColorIssue> {
    if let Some(nil) = node.attribute((XSI, "nil")) {
        if !matches!(xml_trim(nil), "false" | "0") {
            return Err(invalid("Color requires an explicit non-nil value"));
        }
    }
    let type_name = node
        .attribute((XSI, "type"))
        .ok_or_else(|| invalid("Color Value has no xsi:type"))?;
    if expanded(node, type_name)? != (Some(UI), "Color") {
        return Err(invalid("Value xsi:type does not identify the Color type"));
    }
    let raw = leaf(node)?;
    let text = xml_trim(&raw);
    if text.is_empty() {
        return Err(invalid("Color Value is empty"));
    }
    if text.starts_with('#') {
        return StyleItemColor::parse_rgb(text)
            .map(StyleItemColor::Rgb)
            .ok_or_else(|| invalid("Color RGB must contain six hexadecimal digits"));
    }
    let (namespace, name) = expanded(node, text)?;
    let palette = match namespace {
        Some(WEB) => ColorPalette::Web,
        Some(STYLE) => ColorPalette::Style,
        _ => {
            return Err(ColorIssue::ReadOnly(
                "Color namespace is outside the writable profile".into(),
            ))
        }
    };
    PaletteColor::new(palette, name)
        .map(StyleItemColor::Symbolic)
        .ok_or_else(|| {
            if palette == ColorPalette::Style {
                ColorIssue::ReadOnly("custom or unmodelled style color is read-only".into())
            } else {
                invalid("unknown platform web color")
            }
        })
}

fn descriptor_document<'input>(
    source: &'input str,
    name: &str,
    relative: &Path,
    require_writable: bool,
) -> Result<Document<'input>, ColorIssue> {
    let evidence = unica_source_designer::owner::prove_already_read_metadata_owner(
        relative,
        source.as_bytes(),
    )
    .map_err(|error| invalid(error.message))?;
    if require_writable {
        if let Some(refusal) = writable_profile_refusal(relative.portable(), evidence.version()) {
            return Err(invalid(refusal.message));
        }
    }
    if evidence.artifact_kind() != "StyleItem" || evidence.artifact_name() != Some(name) {
        return Err(invalid(
            "StyleItem descriptor does not match the addressed owner",
        ));
    }
    let document = Document::parse(source).map_err(|error| invalid(error.to_string()))?;
    let owner = single(document.root_element(), "StyleItem")?;
    owner
        .attribute("uuid")
        .and_then(|id| uuid::Uuid::parse_str(id).ok())
        .ok_or_else(|| invalid("StyleItem UUID is missing or invalid"))?;
    let properties = single(owner, "Properties")?;
    if xml_trim(&leaf(single(properties, "Name")?)?) != name {
        return Err(invalid("StyleItem Name does not match the addressed owner"));
    }
    Ok(document)
}

fn prove_registration(
    name: &str,
    parent: &[u8],
    source_kind: unica_source_contract::SourceSetKind,
) -> Result<(), ColorIssue> {
    let root = Path::new("Configuration.xml");
    let owner = unica_source_designer::owner::prove_already_read_source_set_owner(
        root,
        parent,
        source_kind,
    )
    .map_err(|error| invalid(error.message))?;
    if let Some(refusal) = writable_profile_refusal(root.portable(), owner.version()) {
        return Err(invalid(refusal.message));
    }
    if !owner.registers("StyleItem", name) {
        return Err(ColorIssue::NotFound(
            "StyleItem is not registered by its source-set owner".into(),
        ));
    }
    Ok(())
}

/// Independent capabilities on an already registered and readable descriptor.
pub fn observe_properties(
    source: &str,
    name: &str,
    relative: &Path,
) -> Result<unica_metadata::style_color::StyleItemPropertiesObservation, ColorIssue> {
    let document = descriptor_document(source, name, relative, false)?;
    let version =
        unica_source_designer::roots::root_version_literal(source, document.root_element());
    if writable_profile_refusal(relative.portable(), version.as_deref()).is_some() {
        return Ok(
            unica_metadata::style_color::StyleItemPropertiesObservation {
                comment: false,
                color: None,
            },
        );
    }
    Ok(
        unica_metadata::style_color::StyleItemPropertiesObservation {
            comment: crate::descriptor_comment::comment_patch_capability(
                source,
                unica_address::address::NodeKind::StyleItem,
                name,
            )
            .is_ok(),
            // Identity/XML failures were proved above; remaining failures belong
            // to the independent Type/Value field capability.
            color: StyleColorImage::parse(source, name, relative)
                .ok()
                .map(|image| image.color()),
        },
    )
}

/// Productive admission preserves the existing typed intent and Color refusals.
pub fn observe_registered_properties(
    source: &str,
    name: &str,
    relative: &Path,
    parent: &[u8],
    source_kind: unica_source_contract::SourceSetKind,
    intent: Option<&unica_metadata::property_input::PropertiesIntent>,
) -> Result<unica_metadata::style_color::StyleItemPropertiesObservation, ColorIssue> {
    use unica_metadata::property_input::PropertiesIntent;
    prove_registration(name, parent, source_kind)?;
    descriptor_document(source, name, relative, true)?;
    match intent {
        Some(PropertiesIntent::StyleItem(values)) => {
            let image =
                StyleColorImage::parse_registered(source, name, relative, parent, source_kind)?;
            let comment = crate::descriptor_comment::comment_patch_capability(
                source,
                unica_address::address::NodeKind::StyleItem,
                name,
            );
            if values.comment.is_some() {
                comment
                    .as_ref()
                    .map_err(|message| invalid(message.clone()))?;
            }
            Ok(
                unica_metadata::style_color::StyleItemPropertiesObservation {
                    comment: comment.is_ok(),
                    color: Some(image.color()),
                },
            )
        }
        Some(PropertiesIntent::CommentOnly {
            kind: unica_address::address::NodeKind::StyleItem,
            ..
        }) => {
            crate::descriptor_comment::comment_patch_capability(
                source,
                unica_address::address::NodeKind::StyleItem,
                name,
            )
            .map_err(invalid)?;
            Ok(
                unica_metadata::style_color::StyleItemPropertiesObservation {
                    comment: true,
                    color: None,
                },
            )
        }
        None => observe_properties(source, name, relative),
        Some(_) => Err(invalid("expected a StyleItem property request")),
    }
}

/// One checked descriptor image. Ranges and bindings always come from this
/// same input; neither preview nor publication reconstructs the descriptor.
pub struct StyleColorImage<'input> {
    document: Document<'input>,
    value: NodeId,
    color: StyleItemColor,
}
impl<'input> StyleColorImage<'input> {
    pub fn parse_registered(
        source: &'input str,
        name: &str,
        relative: &Path,
        parent: &[u8],
        source_kind: unica_source_contract::SourceSetKind,
    ) -> Result<Self, ColorIssue> {
        prove_registration(name, parent, source_kind)?;
        Self::parse(source, name, relative)
    }

    pub fn parse(source: &'input str, name: &str, relative: &Path) -> Result<Self, ColorIssue> {
        let document = descriptor_document(source, name, relative, true)?;
        let owner = single(document.root_element(), "StyleItem")?;
        let properties = single(owner, "Properties")?;
        let kind = leaf(single(properties, "Type")?)?;
        if xml_trim(&kind) != "Color" {
            return Err(ColorIssue::ReadOnly(
                "only StyleItem Type=Color has a color writer".into(),
            ));
        }
        let value = single(properties, "Value")?;
        let color = color_value(value)?;
        let id = value.id();
        Ok(Self {
            document,
            value: id,
            color,
        })
    }
    pub const fn color(&self) -> StyleItemColor {
        self.color
    }
    pub fn replace(&self, desired: StyleItemColor) -> String {
        let source = self.document.input_text();
        if self.color == desired {
            return source.to_owned();
        }
        let value = self
            .document
            .get_node(self.value)
            .expect("checked Value node belongs to this document");
        let original = &source[value.range()];
        let end = opening_end(original);
        let mut declaration = String::new();
        let text = match desired {
            StyleItemColor::Rgb(_) => desired.public_text(),
            StyleItemColor::Symbolic(color) => {
                let (namespace, preferred) = match color.palette() {
                    ColorPalette::Web => (WEB, "web"),
                    ColorPalette::Style => (STYLE, "style"),
                };
                if value.lookup_namespace_uri(None) == Some(namespace) {
                    color.name().to_owned()
                } else {
                    let prefix = value
                        .lookup_prefix(namespace)
                        .map(str::to_owned)
                        .unwrap_or_else(|| {
                            let mut prefix = preferred.to_owned();
                            let mut suffix = 1;
                            while value.lookup_namespace_uri(Some(&prefix)).is_some() {
                                prefix = format!("{preferred}{suffix}");
                                suffix += 1;
                            }
                            declaration = format!(" xmlns:{prefix}=\"{namespace}\"");
                            prefix
                        });
                    format!("{prefix}:{}", color.name())
                }
            }
        };
        // A checked explicit value is nonempty and therefore has a closing tag.
        let close = original
            .rfind("</")
            .expect("explicit Color has a closing tag");
        let replacement = format!(
            "{}{declaration}>{}{}",
            &original[..end - 1],
            unica_xml_format::text::escape_xml(&text),
            &original[close..]
        );
        let mut updated = source.to_owned();
        updated.replace_range(value.range(), &replacement);
        updated
    }
}
fn opening_end(source: &str) -> usize {
    let mut quote = None;
    for (offset, ch) in source.char_indices() {
        match (quote, ch) {
            (Some(current), ch) if current == ch => quote = None,
            (None, '\'' | '"') => quote = Some(ch),
            (None, '>') => return offset + 1,
            _ => {}
        }
    }
    unreachable!("parsed element has an opening tag")
}

#[cfg(test)]
mod tests;
