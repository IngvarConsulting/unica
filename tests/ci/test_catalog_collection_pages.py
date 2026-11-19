"""Перечисление справочников конфигурации класса УТ проходит до конца страницами ограниченного размера (#548)."""
from __future__ import annotations

from scripts.ci.prepared_binary import ensure_binary

import importlib.util
import json
import re
import tempfile
import unittest
from pathlib import Path

from tests.ci.acceptance_controls import tool_result
from tests.ci.acceptance_profiles import isolated_environment
from tests.ci.test_acceptance_scenarios import AcceptanceServer, REPO_ROOT
from scripts.ci.temp_tree import remove_tree

# Размер тестов модуля (scripts/ci/python_sizes.py).
# Собирает продукт и читает коллекцию через MCP.
MEDIUM = ("CatalogCollectionPagesTests",)

GENERATOR = REPO_ROOT / "scripts/dev/synthetic-configuration.py"
PROFILE = REPO_ROOT / "scripts/dev/synthetic-configuration-ut11.json"
# Граница размера страницы по умолчанию: строка коллекции не растёт с объектом,
# и чтение коллекции пополняет контекст агента предсказуемыми порциями. Точный
# счёт токенов здесь не нужен. Порог выведен из ориентира 2000 токенов
# на страницу (#548) по замеру 19.11.2026: tiktoken o200k_base на компактном
# JSON `structuredContent` каждой страницы `view main:Catalog` при limit 20.
# Синтетика УТ этого теста (471 справочник, 24 страницы) — наибольшая 1479
# токенов при 7073 байтах; КА 2 2.6.2.72 (кодовая база УТ 11, 1090
# справочников, 55 страниц) — 1922 токена при 8886 байтах. Кириллические имена
# и синонимы дают 4,47–4,55 байта на токен; 2000 × 4,4 = 8800.
PAGE_BYTES = 8800


def generator():
    spec = importlib.util.spec_from_file_location("synthetic_configuration", GENERATOR)
    assert spec and spec.loader
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


class CatalogCollectionPagesTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        ensure_binary(REPO_ROOT)

    def test_catalogs_of_a_ut_class_configuration_are_listed_to_the_end_in_small_pages(self):
        """rule: CTR.SOURCE.METADATA-COLLECTION-ROW"""
        root = Path(tempfile.mkdtemp(prefix="unica-catalog-pages-")).resolve()
        self.addCleanup(remove_tree, root, 30)
        workspace = root / "workspace"
        profile = json.loads(PROFILE.read_text(encoding="utf-8"))
        generator().generate(workspace, profile, 1.0, 1119, ("Catalog",), False)
        configuration = (workspace / "src/Configuration.xml").read_text(encoding="utf-8-sig")
        expected = re.findall(r"<Catalog>([^<]+)</Catalog>", configuration)
        self.assertEqual(len(expected), profile["objects"]["Catalog"])
        state = root / "state"
        state.mkdir()
        server = AcceptanceServer(
            workspace, state, "2025-11-25", isolated_environment(REPO_ROOT / "plugins/unica", state)
        )
        self.addCleanup(server.close)

        rows, sizes, arguments = [], [], {"at": "main:Catalog"}
        while True:
            response = server.call("unica.view", arguments)
            result = tool_result(response)
            self.assertTrue(result and result.get("ok"), f"page {len(sizes) + 1} failed: {result}")
            # Страница — то, что читает агент: структурный ответ инструмента
            # в компактном JSON, как его передаёт сервер.
            sizes.append(len(json.dumps(result, ensure_ascii=False, separators=(",", ":")).encode()))
            rows.extend(result["data"]["items"])
            if "cursor" not in result:
                break
            arguments = {"at": "main:Catalog", "cursor": result["cursor"]}

        self.assertEqual([row["at"] for row in rows], [f"main:Catalog.{name}" for name in expected])
        for row in rows:
            name = row["at"].removeprefix("main:Catalog.")
            # Синоним генератор пишет теми же словами, что имя; число детей —
            # записи ChildObjects шаблона, а не сумма счётчиков ветвей узла.
            self.assertEqual(row["title"], name)
            self.assertRegex(row["props"]["synonym"], r"^ru = '[^']+'$")
            spelled = row["props"]["synonym"].removeprefix("ru = '")[:-1].replace(" ", "")
            self.assertEqual(spelled.lower(), name.lower())
            self.assertGreater(row["props"]["children"], 0)
        over = [(index + 1, size) for index, size in enumerate(sizes) if size > PAGE_BYTES]
        self.assertEqual(over, [], f"pages over {PAGE_BYTES} bytes; sizes {sizes}")


if __name__ == "__main__":
    unittest.main()
