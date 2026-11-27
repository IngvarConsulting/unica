use unica_source_contract::format_identity::PlatformLine;

pub const PLATFORM_PROFILE_8_3_27: &str = "8.3.27";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlatformProfile {
    id: &'static str,
    line: PlatformLine,
}

impl PlatformProfile {
    pub const fn v8_3_27() -> Self {
        Self {
            id: PLATFORM_PROFILE_8_3_27,
            line: PlatformLine::new(8, 3, 27),
        }
    }

    /// The semantic profile of a described platform line.
    pub fn for_line(line: PlatformLine) -> Option<Self> {
        unica_source_contract::format_identity::DESCRIBED_PLATFORM_LINES
            .iter()
            .find(|(described, _)| *described == line)
            .map(|(line, id)| Self { id, line: *line })
    }

    /// Профиль описанной линии или младшей описанной линии, когда линия
    /// неизвестна или не описана. Форма выбирает каталог своих событий отдельно, по исходной
    /// линии из контекста (`INV.PLATFORM.MODULE-EVENT-CATALOG`).
    pub fn for_line_or_base(line: Option<PlatformLine>) -> Self {
        line.and_then(Self::for_line).unwrap_or_else(Self::v8_3_27)
    }

    pub const fn line(self) -> PlatformLine {
        self.line
    }

    pub const fn id(self) -> &'static str {
        self.id
    }
}

#[cfg(test)]
mod tests {
    use super::{PlatformLine, PlatformProfile};

    #[test]
    fn every_described_platform_line_has_a_semantic_profile() {
        for (line, id) in [
            (PlatformLine::new(8, 3, 27), "8.3.27"),
            (PlatformLine::new(8, 5, 1), "8.5.1"),
            (PlatformLine::new(8, 5, 4), "8.5.4"),
        ] {
            let profile = PlatformProfile::for_line(line).expect("a described line");
            assert_eq!((profile.line(), profile.id()), (line, id));
        }
        assert_eq!(
            PlatformProfile::for_line(PlatformLine::new(8, 3, 27)),
            Some(PlatformProfile::v8_3_27())
        );
        for line in [PlatformLine::new(8, 3, 26), PlatformLine::new(8, 5, 2)] {
            assert_eq!(PlatformProfile::for_line(line), None, "{line}");
        }
    }
}
