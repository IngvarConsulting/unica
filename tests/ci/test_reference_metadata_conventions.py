from pathlib import Path
import unittest


REPO_ROOT = Path(__file__).resolve().parents[2]


class MetadataConventionReferenceTests(unittest.TestCase):
    def test_reference_describes_owner_aware_list_presentation_contract(self) -> None:
        text = (
            REPO_ROOT
            / "plugins"
            / "unica"
            / "references"
            / "platform"
            / "metadata-conventions.md"
        ).read_text(encoding="utf-8")
        for marker in (
            "ListPresentation",
            "Configuration.xml",
            "Languages/<Name>.xml",
            "ExternalReport",
            "ExternalDataProcessor",
        ):
            self.assertIn(marker, text)
        self.assertNotIn("наблюдаемым значениям `v8:lang`", text)
        self.assertNotIn("языконезависим", text)

    def test_subsystem_boolean_and_effective_role_are_fail_closed(self) -> None:
        specification = (
            REPO_ROOT
            / "plugins"
            / "unica"
            / "references"
            / "specs"
            / "1c-subsystem-spec.md"
        ).read_text(encoding="utf-8")
        conventions = (
            REPO_ROOT
            / "plugins"
            / "unica"
            / "references"
            / "platform"
            / "metadata-conventions.md"
        ).read_text(encoding="utf-8")

        for marker in (
            "`IncludeInCommandInterface` обязан присутствовать ровно один раз",
            "ровно `true` или `false`",
            "недоступно, а не пусто",
            "Subsystem.СтандартныеПодсистемы.Subsystem.Обсуждения",
        ):
            self.assertIn(marker, specification)
        self.assertNotIn(
            "Отсутствующий `IncludeInCommandInterface` равнозначен", conventions
        )
        for marker in ("Эффективная роль", "явный `false`", "ADR-0033"):
            self.assertIn(marker, conventions)
        for marker in (
            "`functionalSubsystems`",
            "`interfaceSubsystems`",
            "только подсистемы, чей `Content` содержит анализируемый объект",
            "адресом метаданных или UUID",
            "`[]`",
            "поля отсутствуют",
            "`provider_unavailable`",
        ):
            self.assertIn(marker, conventions)

    def test_reference_index_uses_path_relative_to_itself(self) -> None:
        text = (
            REPO_ROOT / "plugins" / "unica" / "references" / "README.md"
        ).read_text(encoding="utf-8")
        self.assertIn("`platform/metadata-conventions.md`", text)
        self.assertNotIn("`references/platform/metadata-conventions.md`", text)


if __name__ == "__main__":
    unittest.main()
