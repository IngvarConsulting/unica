"""Сборщик результатов в тестах — без построения индекса правил по репозиторию.

`collect-results.py` метит результаты линии main правилами из пометок
`rule:`, а индекс для этого строится по всем исходникам репозитория. Тестам
сборщика на фикстурах с вымышленными именами тестов этот индекс ничего не
даёт, но стоит десятки секунд на модуль. Индекс и его сверку с репозиторием
проверяет `test_rule_marks`: пометки на всём дереве
(`test_every_mark_names_a_rule_and_sits_on_a_test`) и метки отчёта
(`test_report_entries_receive_the_rules_of_their_tests`).
"""

from __future__ import annotations

import unittest


def without_repository_rule_index(case: unittest.TestCase, collector) -> None:
    """Пока идёт тест, сборщик видит пустой индекс правил вместо индекса репозитория."""
    rule_marks = collector.rule_marks
    index = rule_marks.RuleIndex.__new__(rule_marks.RuleIndex)
    index.rust, index.by_name, index.python, index.unknown = [], {}, {}, []
    cache = rule_marks.RuleIndex._cache
    missing = object()
    previous = cache.get(collector.REPO_ROOT, missing)
    cache[collector.REPO_ROOT] = index

    def restore() -> None:
        if previous is missing:
            cache.pop(collector.REPO_ROOT, None)
        else:
            cache[collector.REPO_ROOT] = previous

    case.addCleanup(restore)
