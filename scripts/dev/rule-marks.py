#!/usr/bin/env python3
"""Пометки `rule: <ID>` на тестах.

    python scripts/dev/rule-marks.py migrate [--report FILE]
        Перенести оставшиеся ссылки `check` из `arch/rules/` в пометки на
        тестах и удалить поле `check`. Повторный запуск ничего не меняет.
        Так же переводится ветка, в которой после слияния снова появились
        ссылки `check`.

    python scripts/dev/rule-marks.py pairs
        Напечатать пары «правило, тест» из пометок.

    python scripts/dev/rule-marks.py tests <ID>
        Напечатать тесты правила: точное совпадение ID, без тестов правил,
        чей ID начинается так же.

    python scripts/dev/rule-marks.py verify
        Найти пометки с неизвестным правилом или не над тестом.
"""

from __future__ import annotations

import argparse
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "ci"))
import rule_marks  # noqa: E402

ROOT = Path(__file__).resolve().parents[2]


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    commands = parser.add_subparsers(dest="command", required=True)
    migrate = commands.add_parser("migrate")
    migrate.add_argument("--report", type=Path)
    commands.add_parser("pairs")
    tests = commands.add_parser("tests")
    tests.add_argument("rule")
    commands.add_parser("verify")
    args = parser.parse_args(argv)

    if args.command == "pairs":
        for rule, address in sorted(rule_marks.marked_pairs(ROOT)):
            print(f"{rule}\t{address}")
        return 0
    if args.command == "tests":
        for rule, address in sorted(rule_marks.marked_pairs(ROOT)):
            if rule == args.rule:
                print(address)
        return 0
    if args.command == "verify":
        problems = rule_marks.stray_marks(ROOT)
        print("\n".join(problems) or "rule marks: ok")
        return 1 if problems else 0

    existing = rule_marks.marked_pairs(ROOT)
    before, after, problems = rule_marks.migrate(ROOT)
    expected = before | existing
    failures = [line for line in problems if not line.startswith("address normalized: ")]
    missing = sorted(expected - after)
    extra = sorted(after - expected)
    lines = [
        f"pairs from check: {len(before)}",
        f"pairs marked before: {len(existing)}",
        f"pairs marked after: {len(after)}",
        f"equal: {not missing and not extra}",
        *[f"missing: {rule}\t{address}" for rule, address in missing],
        *[f"extra: {rule}\t{address}" for rule, address in extra],
        *problems,
        "",
        *[f"{rule}\t{address}" for rule, address in sorted(after)],
    ]
    if args.report:
        args.report.write_text("\n".join(lines) + "\n", encoding="utf-8")
    print("\n".join(lines[:4 + len(missing) + len(extra) + len(problems)]))
    return 1 if failures or missing or extra else 0


if __name__ == "__main__":
    raise SystemExit(main())
