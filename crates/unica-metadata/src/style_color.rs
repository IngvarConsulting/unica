//! The writable StyleItem Color vocabulary, independent of XML prefixes.
//!
//! Builtins follow the vendor 8.3.27 syntax-help pages
//! objects/catalog274/{WebColors,StyleColors}.html. These are platform names,
//! not the CSS color vocabulary. User-defined style references are read-only.

use serde_json::{json, Value};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorPalette {
    Web,
    Style,
}

/// A closed symbolic member; construction validates the platform vocabulary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PaletteColor {
    palette: ColorPalette,
    name: &'static str,
}

impl PaletteColor {
    pub fn new(palette: ColorPalette, name: &str) -> Option<Self> {
        let names = match palette {
            ColorPalette::Web => WEB_COLORS,
            ColorPalette::Style => STYLE_COLORS,
        };
        names
            .iter()
            .copied()
            .find(|candidate| *candidate == name)
            .map(|name| Self { palette, name })
    }
    pub const fn palette(self) -> ColorPalette {
        self.palette
    }
    pub const fn name(self) -> &'static str {
        self.name
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StyleItemColor {
    Rgb([u8; 3]),
    Symbolic(PaletteColor),
}

impl StyleItemColor {
    pub fn parse_argument(text: &str) -> Option<Self> {
        if let Some(rgb) = Self::parse_rgb(text) {
            return Some(Self::Rgb(rgb));
        }
        let (palette, name) = text.split_once(':')?;
        let palette = match palette {
            "web" => ColorPalette::Web,
            "style" => ColorPalette::Style,
            _ => return None,
        };
        PaletteColor::new(palette, name).map(Self::Symbolic)
    }
    pub fn parse_rgb(text: &str) -> Option<[u8; 3]> {
        if text.len() != 7
            || !text.starts_with('#')
            || !text.as_bytes()[1..].iter().all(u8::is_ascii_hexdigit)
        {
            return None;
        }
        let bytes = u32::from_str_radix(&text[1..], 16).ok()?.to_be_bytes();
        Some([bytes[1], bytes[2], bytes[3]])
    }
    pub fn public_text(self) -> String {
        match self {
            Self::Rgb([r, g, b]) => format!("#{r:02X}{g:02X}{b:02X}"),
            Self::Symbolic(color) => format!(
                "{}:{}",
                match color.palette {
                    ColorPalette::Web => "web",
                    ColorPalette::Style => "style",
                },
                color.name
            ),
        }
    }
    pub fn argument_schema() -> Value {
        let names = WEB_COLORS
            .iter()
            .map(|name| format!("web:{name}"))
            .chain(STYLE_COLORS.iter().map(|name| format!("style:{name}")))
            .collect::<Vec<_>>();
        json!({"anyOf":[{"type":"string","pattern":"^#[0-9A-Fa-f]{6}$"},{"type":"string","enum":names}],
            "description":"RGB #RRGGBB, a platform web color, or a builtin style color. Custom style references and Windows colors are outside this writer profile."})
    }
}

const WEB_COLORS: &[&str] = &[
    "AliceBlue",
    "AntiqueWhite",
    "Aqua",
    "Aquamarine",
    "Azure",
    "Beige",
    "Bisque",
    "Black",
    "BlanchedAlmond",
    "Blue",
    "BlueViolet",
    "Brown",
    "BurlyWood",
    "CadetBlue",
    "Chartreuse",
    "Chocolate",
    "Coral",
    "CornFlowerBlue",
    "CornSilk",
    "Cream",
    "Crimson",
    "Cyan",
    "DarkBlue",
    "DarkCyan",
    "DarkGoldenRod",
    "DarkGray",
    "DarkGreen",
    "DarkKhaki",
    "DarkMagenta",
    "DarkOliveGreen",
    "DarkOrange",
    "DarkOrchid",
    "DarkRed",
    "DarkSalmon",
    "DarkSeaGreen",
    "DarkSlateBlue",
    "DarkSlateGray",
    "DarkTurquoise",
    "DarkViolet",
    "DeepPink",
    "DeepSkyBlue",
    "DimGray",
    "DodgerBlue",
    "FireBrick",
    "FloralWhite",
    "ForestGreen",
    "Fuchsia",
    "Gainsboro",
    "GhostWhite",
    "Gold",
    "Goldenrod",
    "Gray",
    "Green",
    "GreenYellow",
    "HoneyDew",
    "HotPink",
    "IndianRed",
    "Indigo",
    "Ivory",
    "Khaki",
    "Lavender",
    "LavenderBlush",
    "LawnGreen",
    "LemonChiffon",
    "LightBlue",
    "LightCoral",
    "LightCyan",
    "LightGoldenRod",
    "LightGoldenRodYellow",
    "LightGray",
    "LightGreen",
    "LightPink",
    "LightSalmon",
    "LightSeaGreen",
    "LightSkyBlue",
    "LightSlateBlue",
    "LightSlateGray",
    "LightSteelBlue",
    "LightYellow",
    "Lime",
    "LimeGreen",
    "Linen",
    "Magenta",
    "Maroon",
    "MediumAquaMarine",
    "MediumBlue",
    "MediumGray",
    "MediumGreen",
    "MediumOrchid",
    "MediumPurple",
    "MediumSeaGreen",
    "MediumSlateBlue",
    "MediumSpringGreen",
    "MediumTurquoise",
    "MediumVioletRed",
    "MidnightBlue",
    "MintCream",
    "MistyRose",
    "Moccasin",
    "NavajoWhite",
    "Navy",
    "OldLace",
    "Olive",
    "Olivedrab",
    "Orange",
    "OrangeRed",
    "Orchid",
    "PaleGoldenrod",
    "PaleGreen",
    "PaleTurquoise",
    "PaleVioletRed",
    "PapayaWhip",
    "PeachPuff",
    "Peru",
    "Pink",
    "Plum",
    "PowderBlue",
    "Purple",
    "Red",
    "RosyBrown",
    "RoyalBlue",
    "SaddleBrown",
    "Salmon",
    "SandyBrown",
    "SeaShell",
    "Seagreen",
    "Sienna",
    "Silver",
    "SkyBlue",
    "SlateBlue",
    "SlateGray",
    "Snow",
    "SpringGreen",
    "SteelBlue",
    "Tan",
    "Teal",
    "Thistle",
    "Tomato",
    "Turquoise",
    "Violet",
    "VioletRed",
    "Wheat",
    "White",
    "WhiteSmoke",
    "Yellow",
    "YellowGreen",
];

const STYLE_COLORS: &[&str] = &[
    "AccentColor",
    "ActivityColor",
    "AuxiliaryNavigationColor",
    "BorderColor",
    "ButtonBackColor",
    "ButtonBorderColor",
    "ButtonTextColor",
    "FieldAlternativeBackColor",
    "FieldBackColor",
    "FieldSelectedTextColor",
    "FieldSelectionBackColor",
    "FieldTextColor",
    "FormBackColor",
    "FormTextColor",
    "ImportantColor",
    "NavigationColor",
    "NegativeTextColor",
    "ReportGroup1BackColor",
    "ReportGroup2BackColor",
    "ReportHeaderBackColor",
    "ReportLineColor",
    "SpecialTextColor",
    "TableFooterBackColor",
    "TableFooterTextColor",
    "TableHeaderBackColor",
    "TableHeaderTextColor",
    "ToolTipBackColor",
    "ToolTipTextColor",
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn platform_names_are_distinct_from_css_spellings() {
        for text in [
            "web:Cream",
            "web:MediumGray",
            "web:MediumGreen",
            "web:CornFlowerBlue",
            "web:LightGoldenRod",
            "web:Seagreen",
            "web:Olivedrab",
            "style:SpecialTextColor",
        ] {
            assert_eq!(
                StyleItemColor::parse_argument(text).unwrap().public_text(),
                text
            );
        }
        for text in [
            "web:CornflowerBlue",
            "web:SeaGreen",
            "web:OliveDrab",
            "web:Rebeccapurple",
            "style:Unknown",
            "win:ButtonFace",
            "#123",
            "#GG0000",
            "#00000000",
            " #000000",
            "#000000\u{a0}",
        ] {
            assert!(StyleItemColor::parse_argument(text).is_none(), "{text}");
        }
    }

    #[test]
    fn rgb_equality_does_not_depend_on_hex_case() {
        assert_eq!(
            StyleItemColor::parse_argument("#abCdEF"),
            StyleItemColor::parse_argument("#ABCDEF")
        );
        assert_ne!(
            StyleItemColor::parse_argument("#000000"),
            StyleItemColor::parse_argument("#000001")
        );
    }
}
