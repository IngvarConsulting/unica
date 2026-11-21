//! Which top-level metadata kinds creation writes, and why it refuses the rest.
//!
//! A refusal names what is wrong: a kind the set's platform line does not
//! have, a kind another operation creates, or a kind Unica reads and edits
//! but cannot create yet. Each is a different answer for the consumer, so
//! they never share one text.
use super::{MetaDiagnostic, MetaDiagnosticCode, MetadataKind};
use unica_source_contract::format_identity::{
    metadata_kind_exists, metadata_kind_since, PlatformLine,
};

/// A top-level kind that `object.create` writes from the template catalog:
/// the typed set, and the declared kinds whose declaration holds a creation
/// template (`unica_address::kinds::CreationDeclaration`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CreatableKind {
    Typed(MetadataKind),
    /// A declared root kind outside the typed set, by its tag.
    Declared(&'static str),
}

impl CreatableKind {
    /// The declared kinds outside the typed set that creation writes, in the
    /// order of the configuration's `ChildObjects`.
    pub fn declared() -> impl Iterator<Item = Self> {
        unica_address::kinds::DECLARED_ROOT_KINDS
            .iter()
            .copied()
            .filter(|tag| MetadataKind::parse(tag).is_err() && Self::creation(tag).is_some())
            .map(Self::Declared)
    }

    /// Every creatable kind: the typed set, then the declared ones.
    pub fn all() -> impl Iterator<Item = Self> {
        MetadataKind::ALL
            .iter()
            .copied()
            .map(Self::Typed)
            .chain(Self::declared())
    }

    pub fn requires_creation_namespace(self) -> bool {
        matches!(self, Self::Declared("XDTOPackage"))
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Typed(kind) => kind.as_str(),
            Self::Declared(tag) => tag,
        }
    }

    /// The typed kind, when the kind is one of the typed set.
    pub const fn typed(self) -> Option<MetadataKind> {
        match self {
            Self::Typed(kind) => Some(kind),
            Self::Declared(_) => None,
        }
    }

    /// The creation template a declared kind holds.
    pub fn template(self) -> Option<unica_address::kinds::CreationDeclaration> {
        match self {
            Self::Typed(_) => None,
            Self::Declared(tag) => Self::creation(tag),
        }
    }

    pub fn requires_creation_type(self) -> bool {
        self.template()
            .is_some_and(|template| template.requires_type)
    }

    /// Whether an extension creates its own object of the kind: it borrows
    /// the kinds the declaration keeps to the main configuration.
    pub fn created_in_extension(self) -> bool {
        !self
            .template()
            .is_some_and(|template| template.main_configuration_only)
    }

    fn creation(tag: &str) -> Option<unica_address::kinds::CreationDeclaration> {
        unica_address::kinds::declared(tag).and_then(|(_, declaration)| declaration.creation)
    }

    /// The kind `object.create` names, or the refusal that says why it
    /// cannot be created.
    pub fn parse(value: &str) -> Result<Self, MetaDiagnostic> {
        if let Ok(kind) = MetadataKind::parse(value) {
            return Ok(Self::Typed(kind));
        }
        Self::declared()
            .find(|kind| kind.as_str() == value)
            .ok_or_else(|| creation_refusal(value))
    }
}

/// The operation that creates a root kind outside the metadata family.
const OWN_CREATION_OPERATIONS: &[(&str, &str)] =
    &[("Role", "role.create"), ("Subsystem", "subsystem.create")];

/// Why a kind cannot be created by `object.create`: another operation creates
/// it, Unica reads and edits it but has no template for it yet, or it is no
/// top-level metadata kind at all.
pub fn creation_refusal(kind: &str) -> MetaDiagnostic {
    let message = if let Some((_, operation)) = OWN_CREATION_OPERATIONS
        .iter()
        .find(|(declared, _)| *declared == kind)
    {
        format!("metadata kind `{kind}` is created by the `{operation}` operation")
    } else if is_root_metadata_kind(kind) {
        format!(
            "Unica reads and edits metadata kind `{kind}` but cannot create it yet; create the object in Designer and export the source set again"
        )
    } else {
        format!("unsupported metadata kind `{kind}`")
    };
    MetaDiagnostic::error(MetaDiagnosticCode::UnsupportedKind, message).with_field("kind")
}

/// Whether `kind` is a declared top-level metadata kind of a configuration.
pub fn is_root_metadata_kind(kind: &str) -> bool {
    unica_address::kinds::DECLARED_ROOT_KINDS.contains(&kind)
}

/// Why a set of platform `line` cannot hold a top-level metadata `kind`: the
/// kind exists from a later line (INV.SOURCE.PROFILE-CAPABILITIES). `None`
/// when the kind is unknown or the line has it.
pub fn kind_line_refusal(kind: &str, line: Option<PlatformLine>) -> Option<String> {
    if !is_root_metadata_kind(kind) || metadata_kind_exists(line, kind) {
        return None;
    }
    let since = metadata_kind_since(kind)?;
    let set = match line {
        Some(line) => format!("this source set is 1C {line}, where the kind does not exist"),
        None => "this source set names no platform line, so it holds only the kinds of every described line".to_string(),
    };
    Some(format!(
        "`{kind}` is a metadata kind of 1C {since} and later; {set}. Only a set re-exported from {since} or later can hold {}.",
        indefinite_noun(kind)
    ))
}

/// The kind as an English noun with its article: `Queue` is «a queue»,
/// `XDTOPackage` «an XDTO package».
fn indefinite_noun(kind: &str) -> String {
    let mut words = Vec::new();
    let chars = kind.chars().collect::<Vec<_>>();
    let mut start = 0;
    for at in 1..chars.len() {
        let previous = chars[at - 1];
        let current = chars[at];
        let next_lower = chars.get(at + 1).is_some_and(|next| next.is_lowercase());
        if current.is_uppercase() && (previous.is_lowercase() || next_lower) {
            words.push(chars[start..at].iter().collect::<String>());
            start = at;
        }
    }
    words.push(chars[start..].iter().collect::<String>());
    let words = words
        .into_iter()
        .map(|word| {
            if word.chars().skip(1).any(char::is_uppercase) {
                word
            } else {
                word.to_lowercase()
            }
        })
        .collect::<Vec<_>>();
    let first = &words[0];
    // An acronym is read by its letters: «an XDTO», «an HTTP», «a WS».
    let vowel_sound = if first.chars().all(|letter| letter.is_uppercase()) {
        first.starts_with(['A', 'E', 'F', 'H', 'I', 'L', 'M', 'N', 'O', 'R', 'S', 'X'])
    } else {
        first.starts_with(['a', 'e', 'i', 'o', 'u'])
    };
    let article = if vowel_sound { "an" } else { "a" };
    format!("{article} {}", words.join(" "))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// rule: INV.SOURCE.PROFILE-CAPABILITIES
    #[test]
    fn a_kind_of_a_later_line_is_refused_with_its_line_and_the_set_line() {
        let line = PlatformLine::new(8, 3, 27);
        assert_eq!(
            kind_line_refusal("Queue", Some(line)).as_deref(),
            Some(
                "`Queue` is a metadata kind of 1C 8.5.4 and later; this source set is 1C 8.3.27, where the kind does not exist. Only a set re-exported from 8.5.4 or later can hold a queue."
            )
        );
        assert_eq!(
            kind_line_refusal("PaletteColor", Some(line)).as_deref(),
            Some(
                "`PaletteColor` is a metadata kind of 1C 8.5.1 and later; this source set is 1C 8.3.27, where the kind does not exist. Only a set re-exported from 8.5.1 or later can hold a palette color."
            )
        );
        assert!(kind_line_refusal("Queue", None)
            .unwrap()
            .contains("this source set names no platform line"));
        for (kind, line) in [
            ("Queue", PlatformLine::new(8, 5, 4)),
            ("PaletteColor", PlatformLine::new(8, 5, 1)),
            ("Catalog", PlatformLine::new(8, 3, 27)),
            ("Bot", PlatformLine::new(8, 3, 27)),
            ("NotAKind", PlatformLine::new(8, 3, 27)),
        ] {
            assert_eq!(kind_line_refusal(kind, Some(line)), None, "{kind}");
        }
    }

    #[test]
    fn the_noun_of_a_kind_keeps_its_acronyms() {
        for (kind, noun) in [
            ("Queue", "a queue"),
            ("PaletteColor", "a palette color"),
            ("XDTOPackage", "an XDTO package"),
            ("WSReference", "a WS reference"),
            ("HTTPService", "an HTTP service"),
            ("IntegrationService", "an integration service"),
        ] {
            assert_eq!(indefinite_noun(kind), noun);
        }
    }

    /// Root kinds supported by object.create parse, while a kind created by
    /// another operation and an unknown name get their own refusals.
    #[test]
    fn creation_refusals_name_what_is_wrong() {
        assert_eq!(
            CreatableKind::parse("Queue"),
            Ok(CreatableKind::Declared("Queue")),
            "the queue has a template"
        );
        assert_eq!(
            CreatableKind::parse("Catalog"),
            Ok(CreatableKind::Typed(MetadataKind::Catalog))
        );
        for (kind, message) in [
            (
                "Role",
                "metadata kind `Role` is created by the `role.create` operation",
            ),
            (
                "Subsystem",
                "metadata kind `Subsystem` is created by the `subsystem.create` operation",
            ),
            (
                "SyntheticMetadata",
                "unsupported metadata kind `SyntheticMetadata`",
            ),
            ("catalog", "unsupported metadata kind `catalog`"),
            ("Attribute", "unsupported metadata kind `Attribute`"),
        ] {
            let refused = CreatableKind::parse(kind).expect_err(kind);
            assert_eq!(refused.code, MetaDiagnosticCode::UnsupportedKind, "{kind}");
            assert_eq!(refused.field.as_deref(), Some("kind"));
            assert_eq!(refused.message, message);
        }
    }

    /// Every declared top-level kind outside the typed set either has a
    /// template, another creating operation, or the «cannot create yet»
    /// refusal: none falls through to «unsupported».
    #[test]
    fn every_declared_root_kind_has_a_distinguishable_answer() {
        let mut uncreatable = Vec::new();
        for kind in unica_address::kinds::DECLARED_ROOT_KINDS {
            if CreatableKind::parse(kind).is_ok() {
                continue;
            }
            let refused = CreatableKind::parse(kind).unwrap_err();
            assert!(
                !refused.message.starts_with("unsupported metadata kind"),
                "{kind}: {}",
                refused.message
            );
            uncreatable.push(kind);
        }
        uncreatable.sort_unstable();
        assert_eq!(uncreatable, ["Role", "Subsystem"]);
    }
}
