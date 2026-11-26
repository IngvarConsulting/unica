"""Контракт бандла Unica для DeepSeek Harness.

DSH ставит плагин как npm-пакет: `package.json` в корне плагина объявляет
`dsh.bundle.patch`, а патч вставляет строки готовых пакетов установки —
MCP-клиент с сервером `unica` и провайдер навыков с корнем `skills/`. Своего
кода у бандла почти нет, поэтому проверять нечего, кроме объявлений: опечатка
в пути или в имени пакета видна только после установки, в чужом профиле.

Манифест лежит в корне плагина, а не в отдельном пакете: DSH ставит плагин из
каталога `plugins/unica` репозитория и из `unica-marketplace`, где pnpm забирает
только этот подкаталог. Соседний каталог в установку не попал бы.

Тест закрепляет то, что можно доказать без установки: манифест находит патч,
экспорты и `files` покрывают ресурсы, версия совпадает с манифестами хостов,
патч объявляет ровно две поверхности, а пути из `!!js` указывают в реальные
каталоги. Установку, отрисовку карточки плагина и живые инструменты он не
проверяет.
"""

from __future__ import annotations

import json
import re
import subprocess
import unittest
from pathlib import Path
from typing import Any

import yaml

REPO_ROOT = Path(__file__).resolve().parents[2]
PLUGIN_ROOT = REPO_ROOT / "plugins" / "unica"
PACKAGE_JSON = PLUGIN_ROOT / "package.json"
PATCH = PLUGIN_ROOT / "dsh" / "cordis.patch.yml"
LAUNCHER = PLUGIN_ROOT / "dsh" / "launch.mjs"
PACKAGED_LAUNCHER = PLUGIN_ROOT / "bootstrap" / "launch.sh"
LOCALES = PLUGIN_ROOT / "dsh" / "locale"
HOST_MANIFESTS = (
    PLUGIN_ROOT / ".codex-plugin" / "plugin.json",
    PLUGIN_ROOT / ".claude-plugin" / "plugin.json",
    PLUGIN_ROOT / ".zcode-plugin" / "plugin.json",
)
ICON_LIMIT_BYTES = 256 * 1024
# Манифест npm кладёт в архив всегда, поэтому в `files` его не перечисляют.
ALWAYS_PACKED_ENTRIES = {"package.json"}
# Каталоги, которые собирает упаковщик: в отслеживаемых файлах их нет.
GENERATED_ENTRIES = {"bin"}


class JsExpression(str):
    """Скаляр `!!js` как есть: выражение вычисляет Loader, а не тест."""


def load_patch() -> Any:
    class Loader(yaml.SafeLoader):
        pass

    Loader.add_constructor(
        "tag:yaml.org,2002:js",
        lambda loader, node: JsExpression(loader.construct_scalar(node)),
    )
    return yaml.load(PATCH.read_text(encoding="utf-8"), Loader=Loader)


def package() -> dict[str, Any]:
    return json.loads(PACKAGE_JSON.read_text(encoding="utf-8"))


def plugin_entries() -> set[str]:
    """Верхний уровень плагина, который попадает в пакет.

    Берутся файлы, которые Git видит и не игнорирует, — и добавленные, и ещё
    не добавленные в индекс: новый ресурс обязан попасть в `files` до
    коммита, а не после. Без Git список строится обходом каталога.
    """
    listed = subprocess.run(
        [
            "git",
            "-C",
            str(REPO_ROOT),
            "ls-files",
            "--cached",
            "--others",
            "--exclude-standard",
            "--",
            "plugins/unica",
        ],
        capture_output=True,
        text=True,
        check=False,
    )
    if listed.returncode != 0:
        return {entry.name for entry in PLUGIN_ROOT.iterdir()}
    prefix = "plugins/unica/"
    entries = set()
    for line in listed.stdout.splitlines():
        relative = line[len(prefix) :] if line.startswith(prefix) else ""
        if relative:
            entries.add(relative.split("/", 1)[0])
    return entries - ALWAYS_PACKED_ENTRIES - GENERATED_ENTRIES


def inserted_rows() -> list[dict[str, Any]]:
    rows: list[dict[str, Any]] = []
    for layer in load_patch():
        rows.extend(layer.get("insert", []))
    return rows


def row_by_id(identifier: str) -> dict[str, Any]:
    for row in inserted_rows():
        if row.get("id") == identifier:
            return row
    raise AssertionError(f"патч не объявляет строку {identifier}")


def plugin_root_expression_path(expression: str) -> Path | None:
    """Каталог плагина, к которому выражение добавляет сегменты.

    Выражения резолвят установленный пакет, а установки в проверке нет:
    проверяется, что выражение резолвит именно этот пакет и что сегменты,
    добавленные к его корню, существуют в дереве.
    """
    match = re.search(
        r"createRequire\(baseUrl\)\.resolve\('@ingvarconsulting/unica/package\.json'\)"
        r"\),((?:\s*'[^']+',?)+)\)",
        expression,
    )
    if match is None:
        return None
    segments = re.findall(r"'([^']+)'", match.group(1))
    return PLUGIN_ROOT.joinpath(*segments)


class DshPackageManifestTests(unittest.TestCase):
    def test_the_manifest_points_at_an_existing_patch(self) -> None:
        patch = package().get("dsh", {}).get("bundle", {}).get("patch")

        self.assertIsInstance(patch, str)
        self.assertFalse(Path(patch).is_absolute(), "путь патча относителен корню пакета")
        self.assertTrue((PLUGIN_ROOT / patch).is_file(), f"нет файла патча: {patch}")

    def test_the_bundle_version_matches_every_host_manifest(self) -> None:
        # Версию поднимает тот же выпуск, что и манифесты хостов: расхождение
        # здесь означает, что bump-version не знает о бандле.
        version = package().get("version")

        self.assertIsInstance(version, str)
        for manifest in HOST_MANIFESTS:
            declared = json.loads(manifest.read_text(encoding="utf-8")).get("version")
            self.assertEqual(version, declared, f"версия расходится с {manifest}")

    def test_exports_reach_the_resources_dsh_reads(self) -> None:
        exports = package().get("exports", {})

        # Разрешение собственного `package.json` — часть пути к корню пакета
        # в `!!js`; без этого экспорта выражение не найдёт пакет.
        self.assertEqual(exports.get("./package.json"), "./package.json")
        self.assertIn("./locale/*.json", exports)
        self.assertIn("./icon", exports)

        icon = PLUGIN_ROOT / exports["./icon"]
        self.assertTrue(icon.is_file(), f"нет иконки: {icon}")
        self.assertTrue(icon.resolve().is_relative_to(PLUGIN_ROOT.resolve()))
        self.assertEqual(icon.suffix, ".svg")
        self.assertLessEqual(icon.stat().st_size, ICON_LIMIT_BYTES)

    def test_locale_export_resolves_to_the_locale_directory(self) -> None:
        target = package()["exports"]["./locale/*.json"]

        self.assertEqual((PLUGIN_ROOT / Path(target).parent).resolve(), LOCALES.resolve())

    def test_files_cover_every_packaged_entry(self) -> None:
        # Шаблон `files` может называть каталог по частям (`locale/*.json`):
        # сверяется верхний уровень, который шаблон покрывает.
        declared = {entry.split("/", 1)[0] for entry in package().get("files", [])}

        missing = sorted(plugin_entries() - declared)
        self.assertEqual(missing, [], f"в files не хватает: {', '.join(missing)}")


class DshPatchTests(unittest.TestCase):
    def test_the_patch_declares_the_server_and_the_skill_root(self) -> None:
        rows = inserted_rows()

        self.assertEqual(
            sorted(row.get("id") for row in rows), ["unica-mcp", "unica-skills"]
        )
        server = row_by_id("unica-mcp")
        self.assertEqual(server.get("name"), "@deepseek-ai/dsh-mcp-client")
        self.assertEqual(server["config"].get("serverName"), "unica")
        self.assertEqual(server["config"].get("transport"), "stdio")
        skills = row_by_id("unica-skills")
        self.assertEqual(skills.get("name"), "@deepseek-ai/dsh-skill-filesystem")
        self.assertEqual(skills["config"].get("providerName"), "unica")
        self.assertIs(skills["config"].get("includeDefaultRoots"), False)

    def test_skill_root_is_declared_as_a_javascript_path(self) -> None:
        skills = row_by_id("unica-skills")["config"]

        directories = skills.get("customSkillDirs")
        self.assertEqual(len(directories), 1)
        self.assertIsInstance(directories[0], JsExpression)

    def test_javascript_paths_reach_the_launcher_and_the_skills(self) -> None:
        expressions = re.findall(r"!!js (.+)", PATCH.read_text(encoding="utf-8"))
        self.assertGreaterEqual(len(expressions), 3)

        resolved = set()
        for expression in expressions:
            path = plugin_root_expression_path(expression)
            if path is None:
                continue
            self.assertTrue(path.exists(), f"нет пути из выражения: {path}")
            resolved.add(path.resolve())
        self.assertIn(LAUNCHER.resolve(), resolved)
        self.assertIn((PLUGIN_ROOT / "skills").resolve(), resolved)

    def test_runtime_overrides_live_under_the_dsh_home(self) -> None:
        env = row_by_id("unica-mcp")["config"].get("env", {})

        for name in ("UNICA_RUNTIME_CACHE_DIR", "UNICA_PROVIDER_STATE_DIR"):
            self.assertIsInstance(env.get(name), JsExpression, name)
            self.assertIn("DSH_HOME", env[name])
            self.assertIn(".dsh", env[name])


class DshLauncherTests(unittest.TestCase):
    def test_the_launcher_starts_the_run_command_without_a_handshake(self) -> None:
        source = LAUNCHER.read_text(encoding="utf-8")

        self.assertIn("'run', '--plugin-root'", source)
        self.assertNotIn("verify", source)

    def test_the_launcher_serves_the_plugin_root(self) -> None:
        source = LAUNCHER.read_text(encoding="utf-8")

        self.assertIn("dirname(dirname(fileURLToPath(import.meta.url)))", source)
        self.assertIn("'bootstrap', 'bin'", source)

    def test_the_launcher_hands_the_project_through_the_override(self) -> None:
        source = LAUNCHER.read_text(encoding="utf-8")

        # Пакетный рантайм не берёт cwd запуска: проект передаётся только
        # переменной, которую знает `INV.WORKSPACE.HOST-CONTEXT`.
        self.assertIn("process.env.UNICA_PROJECT_DIR", source)

    def test_the_target_matrix_matches_the_packaged_launcher(self) -> None:
        packaged = set(
            re.findall(
                r"^\s+target=([\w-]+)$",
                PACKAGED_LAUNCHER.read_text(encoding="utf-8"),
                re.MULTILINE,
            )
        )
        launcher = set(
            re.findall(
                r"'([\w-]+)', 'unica-bootstrap(?:\.exe)?'",
                LAUNCHER.read_text(encoding="utf-8"),
            )
        )

        self.assertEqual(launcher, packaged)

    def test_the_launcher_reports_the_packaged_exit_codes(self) -> None:
        source = LAUNCHER.read_text(encoding="utf-8")

        # 66 и 78 — те же номера, что раздаёт `bootstrap/launch.sh`; 64 и 127
        # остаются за запускателем и загрузчиком соответственно.
        for code in ("EXIT_NO_INPUT = 66", "EXIT_UNSUPPORTED_HOST = 78"):
            self.assertIn(code, source)


class DshLocaleTests(unittest.TestCase):
    def test_every_locale_describes_the_plugin(self) -> None:
        files = sorted(LOCALES.glob("*.json"))

        self.assertIn(LOCALES / "en.json", files)
        self.assertIn(LOCALES / "ru.json", files)
        for path in files:
            meta = json.loads(path.read_text(encoding="utf-8")).get("meta", {})
            self.assertTrue(meta.get("title"), f"{path}: нет meta.title")
            self.assertTrue(meta.get("description"), f"{path}: нет meta.description")
            detail = meta.get("detail")
            if detail is not None:
                self.assertTrue((PLUGIN_ROOT / detail).is_file(), f"{path}: нет {detail}")


if __name__ == "__main__":
    unittest.main()
