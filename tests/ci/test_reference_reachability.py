"""Достижимость справочного корпуса из скиллов.

Корпус `plugins/unica/references/` целиком уезжает в поставку: упаковщик
копирует все отслеживаемые файлы плагина. Документ, который не назван ни одним
скиллом и до которого не ведёт цепочка ссылок от названного, пользователь всё
равно скачивает, а модель в нужный момент не находит. Это правило —
`INV-SKILL-REACHABLE-REFERENCES`.

Модель графа. Корни — документы корпуса, названные в `SKILL.md`. Рёбра — ссылки
между документами корпуса. Ссылкой считается любой относительный путь до `.md`,
который разрешается в существующий документ корпуса: и markdown-ссылка
`[имя](../specs/x.md)`, и путь в обратных кавычках. Второй формы в корпусе
подавляющее большинство — разделы «Related references» перечисляют соседей
именно так, — поэтому обход только по markdown-ссылкам не нашёл бы ни одного
ребра и «транзитивность» осталась бы мёртвым кодом.

Пути разрешаются лексически от каталога документа-источника с нормализацией
`.` и `..`, без обращения к файловой системе: ссылка на несуществующий файл
ребром не становится.
"""

from __future__ import annotations

import posixpath
import re
import unittest
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[2]
PLUGIN_ROOT = REPO_ROOT / "plugins" / "unica"
CORPUS_ROOT = PLUGIN_ROOT / "references"
SKILLS_ROOT = PLUGIN_ROOT / "skills"

CORPUS_PREFIX = CORPUS_ROOT.relative_to(REPO_ROOT).as_posix() + "/"

# Относительный путь до markdown-документа. Лукбехайнд не даёт зацепиться за
# хвост более длинного пути: `plugins/unica/references/specs/x.md` разбирается
# целиком и не превращается в ложную ссылку `specs/x.md`.
#
# Лукахед закрывает ту же дыру с другого конца. Без него `.md` совпадал
# посередине более длинного имени, и упоминание `x.md.bak`, `x.md5` или
# `x.mdx` давало ребро к существующему `x.md` — ложную достижимость, под
# которой прячется неназванный документ. Первое условие отбрасывает
# продолжение имени (`x.md5`, `x.mdx`), второе — продолжение расширения или
# пути (`x.md.bak`), но оставляет точку в конце предложения: там за ней стоит
# пробел или конец строки, а не символ пути.
DOC_PATH = re.compile(
    r"(?<![A-Za-z0-9._/-])((?:\.{1,2}/)*[A-Za-z0-9._/-]*\.md)"
    r"(?![A-Za-z0-9_-])(?![./][A-Za-z0-9._/-])"
)

# Зафиксированный долг, а не одобренное состояние.
#
# Каждая строка ниже — справочный документ, который уезжает в поставку, но не
# назван ни одним скиллом и не достижим по ссылкам от названного. Правильное
# лечение ровно одно: назвать документ в скилле, которому он нужен, и удалить
# строку отсюда. Расширение списка лечением не является — оно узаконивает
# новый неназванный документ.
#
# Корень долга виден по списку: индекс корпуса `references/README.md` ведёт
# почти ко всему остальному, но сам не назван ни одним скиллом, поэтому вся
# ветка под ним висит в воздухе. Обвязка идёт отдельным изменением: дефект
# существует до этого PR, а правка скиллов — правка промпт-видимого слоя, у
# которой своя приёмка.
KNOWN_UNREACHABLE = frozenset(
    {
        "README.md",
        "specs/1c-config-objects-spec.md",
        "specs/1c-configuration-spec.md",
        "specs/1c-dcs-spec.md",
        "specs/1c-epf-spec.md",
        "specs/1c-erf-spec.md",
        "specs/1c-extension-spec.md",
        "specs/1c-help-spec.md",
        "specs/1c-spreadsheet-spec.md",
        "specs/1c-subsystem-spec.md",
        "specs/README.md",
        "specs/format-index.md",
        "specs/web-spec.md",
        "tooling/runtime-build.md",
        "tooling/v8project.md",
        "use-cases/autonomous-server-debug.md",
        "use-cases/extensions-cfe.md",
        "use-cases/integrations.md",
        "use-cases/metadata-modeling.md",
        "use-cases/reports-printing.md",
        "use-cases/workspace-runtime.md",
    }
)


def corpus_documents() -> set[str]:
    """Пути документов корпуса относительно корня репозитория."""
    return {
        path.relative_to(REPO_ROOT).as_posix() for path in CORPUS_ROOT.rglob("*.md")
    }


def cited_documents(source: str, text: str, documents: set[str]) -> set[str]:
    """Документы корпуса, на которые ссылается текст из `source`."""
    base = posixpath.dirname(source)
    cited = set()
    for raw in DOC_PATH.findall(text):
        target = posixpath.normpath(posixpath.join(base, raw))
        if target != source and target in documents:
            cited.add(target)
    return cited


def skill_roots(documents: set[str]) -> dict[str, set[str]]:
    """Документы корпуса, названные в `SKILL.md`, и назвавшие их скиллы.

    Путь разрешается от каталога скилла, поэтому `../../references/specs/x.md`
    попадает в корпус, а собственный `references/x.md` скилла `v8-runner` —
    нет: это его локальный файл, а не общий корпус.
    """
    roots: dict[str, set[str]] = {}
    for skill in sorted(SKILLS_ROOT.glob("*/SKILL.md")):
        source = skill.relative_to(REPO_ROOT).as_posix()
        text = skill.read_text(encoding="utf-8")
        for target in cited_documents(source, text, documents):
            roots.setdefault(target, set()).add(skill.parent.name)
    return roots


def reachable_documents(documents: set[str]) -> set[str]:
    """Транзитивное замыкание корней по ссылкам между документами корпуса."""
    reached = set(skill_roots(documents))
    pending = list(reached)
    while pending:
        current = pending.pop()
        text = (REPO_ROOT / current).read_text(encoding="utf-8")
        for target in cited_documents(current, text, documents):
            if target not in reached:
                reached.add(target)
                pending.append(target)
    return reached


def short(path: str) -> str:
    """Путь относительно корня корпуса — в таком виде ведётся ратчет."""
    return path[len(CORPUS_PREFIX) :] if path.startswith(CORPUS_PREFIX) else path


class ReferenceReachabilityTests(unittest.TestCase):
    def setUp(self) -> None:
        self.documents = corpus_documents()
        self.reachable = reachable_documents(self.documents)
        self.unreachable = {short(path) for path in self.documents - self.reachable}

    def test_corpus_and_roots_are_not_empty(self) -> None:
        """Страховка от молчаливого самоотключения теста.

        Сломанный разбор путей дал бы пустое множество корней, и тогда «долг»
        сравнялся бы со всем корпусом; пустой корпус, наоборот, сделал бы
        правило вечно выполненным.
        """
        self.assertGreater(len(self.documents), 20, "справочный корпус разобран как пустой")
        self.assertGreater(len(skill_roots(self.documents)), 0, "ни один скилл не назвал документ корпуса")

    def test_a_longer_file_name_is_not_a_link_to_the_document_inside_it(self) -> None:
        """Ложная достижимость прячет неназванный документ.

        `x.md.bak`, `x.md5` и `x.mdx` — самостоятельные имена. Считая их
        ссылкой на `x.md`, обход объявил бы достижимым документ, который на
        деле не назван ниоткуда, и ратчет долга поехал бы вниз без обвязки.
        """
        source = "references/README.md"
        target = "references/specs/x.md"
        documents = {target}

        for probe in ("см. specs/x.md.bak", "sha: specs/x.md5",
                      "шаблон specs/x.md.tmpl", "см. specs/x.mdx"):
            with self.subTest(probe=probe):
                self.assertEqual(cited_documents(source, probe, documents), set())

        # Обратная сторона: ссылка в конце предложения ребром остаётся.
        for probe in ("см. specs/x.md.", "см. `specs/x.md`,",
                      "[имя](specs/x.md)", "см. specs/x.md в конце"):
            with self.subTest(probe=probe):
                self.assertEqual(cited_documents(source, probe, documents), {target})

    def test_shipped_reference_documents_are_reachable_from_a_skill(self) -> None:
        """INV-SKILL-REACHABLE-REFERENCES: новых неназванных документов не появляется."""
        new_debt = sorted(self.unreachable - KNOWN_UNREACHABLE)
        self.assertEqual(
            new_debt,
            [],
            "документ уезжает в поставку, но не достижим ни из одного скилла; "
            "назовите его в скилле, которому он нужен, а не добавляйте в KNOWN_UNREACHABLE:\n"
            + "\n".join(new_debt),
        )

    def test_debt_list_only_shrinks(self) -> None:
        """Ратчет: строка уходит из списка, когда документ обвязан или удалён.

        Проверяются обе стороны. Имя, которого больше нет на диске, — мусор,
        под которым может спрятаться новый долг. Имя, ставшее достижимым, —
        ложное утверждение о размере долга; список описывает то, что есть, а
        не то, что было.
        """
        missing = sorted(
            name for name in KNOWN_UNREACHABLE if not (CORPUS_ROOT / name).is_file()
        )
        self.assertEqual(
            missing,
            [],
            "в списке долга есть документ, которого нет на диске; удалите строку:\n"
            + "\n".join(missing),
        )

        repaid = sorted(KNOWN_UNREACHABLE - self.unreachable)
        self.assertEqual(
            repaid,
            [],
            "документ стал достижим из скилла; удалите строку из KNOWN_UNREACHABLE:\n"
            + "\n".join(repaid),
        )


if __name__ == "__main__":
    unittest.main()
