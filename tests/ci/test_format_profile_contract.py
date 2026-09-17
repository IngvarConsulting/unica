from pathlib import Path
import unittest


ROOT = Path(__file__).resolve().parents[2]
FULL_DUMP_PUBLICATION = (
    ROOT
    / "crates/unica-coder/src/infrastructure/platform/full_dump_publication.rs"
)
ACTIVE_SPEC_BANNER = (
    "> Активный контракт Unica: платформа `8.3.27`, формат выгрузки `2.20`."
)
LEGACY_START = "<!-- legacy-format-reference:start -->"
LEGACY_END = "<!-- legacy-format-reference:end -->"
ACTIVE_FORMAT_SPECS = (
    "1c-form-spec.md",
    "1c-config-objects-spec.md",
    "form-dsl-spec.md",
    "1c-dcs-spec.md",
    "1c-epf-spec.md",
    "1c-erf-spec.md",
    "1c-help-spec.md",
    "1c-extension-spec.md",
    "1c-configuration-spec.md",
    "1c-subsystem-spec.md",
    "1c-role-spec.md",
    "1c-spreadsheet-spec.md",
)


def without_legacy_format_references(text: str) -> str:
    current = []
    inside_legacy = False
    for line in text.splitlines():
        if line == LEGACY_START:
            if inside_legacy:
                raise AssertionError("nested legacy-format reference block")
            inside_legacy = True
            continue
        if line == LEGACY_END:
            if not inside_legacy:
                raise AssertionError("orphan legacy-format reference end marker")
            inside_legacy = False
            continue
        if not inside_legacy:
            current.append(line)
    if inside_legacy:
        raise AssertionError("unclosed legacy-format reference block")
    return "\n".join(current)


class FormatProfileContractTests(unittest.TestCase):
    def test_full_dump_uses_the_shared_active_profile(self):
        text = FULL_DUMP_PUBLICATION.read_text(encoding="utf-8")
        self.assertIn(
            "const TARGET_PLATFORM_LINE: &str = ACTIVE_FORMAT_PROFILE.platform_line;",
            text,
        )
        self.assertIn(
            "const TARGET_EXPORT_FORMAT: &str = ACTIVE_FORMAT_PROFILE.export_format;",
            text,
        )
        self.assertNotIn('const TARGET_PLATFORM_LINE: &str = "8.3.27";', text)
        self.assertNotIn('const TARGET_EXPORT_FORMAT: &str = "2.20";', text)

    def test_prompt_visible_specs_use_only_the_active_format_outside_history(self):
        specs = ROOT / "plugins/unica/references/specs"
        for name in ACTIVE_FORMAT_SPECS:
            with self.subTest(spec=name):
                text = (specs / name).read_text(encoding="utf-8")
                self.assertIn(ACTIVE_SPEC_BANNER, "\n".join(text.splitlines()[:12]))
                current = without_legacy_format_references(text)
                self.assertNotIn("2.17", current)
                self.assertNotIn("http://v8.3/", current)


if __name__ == "__main__":
    unittest.main()
