"""Права на вклад: разбор `CONTRIBUTORS.md`, правила проверки и шаг `PR metadata` над подставным `gh`."""

from __future__ import annotations

import contextlib
import io
import json
import os
import shutil
import stat
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

import yaml

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "scripts" / "ci"))
import contribution_rights as rights  # noqa: E402

METADATA = ROOT / ".github/workflows/pr-metadata.yml"
HOLDER = "zeegin"
OFFER = "- [x] Я принимаю оферту Unica CLA версии 1.0 (CLA.md) как физическое лицо от своего имени."
EXEMPT_TABLE = """## Без оферты

| Кто | Почты | Почему |
| --- | --- | --- |
| @zeegin | i@ingvar.pro, 729213+zeegin@users.noreply.github.com | правообладатель |
| Claude | noreply@anthropic.com | агент, работает по поручению правообладателя |
"""


def registry(*participants: str, exempt: str = EXEMPT_TABLE) -> str:
    rows = "".join(f"{row}\n" for row in participants)
    return (
        "# Участники Unica\n\nТекст.\n\n## Участники\n\n"
        "| GitHub | ID | Оферта | PR | Почты |\n| --- | --- | --- | --- | --- |\n"
        f"{rows}\n{exempt}"
    )


ALICE = "| @alice | 101 | 1.0 | #7 | alice@example.com, alice@work.example |"
BOB = "| @bob | 202 | 1.0 | #5 | bob@example.com |"


def pr(login: str = "alice", account_id: int = 101, body: str = OFFER) -> dict:
    return {"number": 7, "user": {"login": login, "id": account_id}, "body": body}


def commit(email: str, login: str | None = None, message: str = "Правка", sha: str = "abcdef1234") -> dict:
    return {"sha": sha, "commit": {"author": {"email": email}, "message": message},
            "author": {"login": login} if login else None}


class ParseTests(unittest.TestCase):
    def test_the_repository_registry_is_well_formed(self) -> None:
        parsed, problems = rights.parse((ROOT / "CONTRIBUTORS.md").read_text(encoding="utf-8"))
        self.assertEqual(problems, [])
        self.assertEqual(parsed.participants, [])
        self.assertEqual([row.who for row in parsed.exempt], ["@zeegin", "Claude", "codex", "@dependabot[bot]"])

    def test_a_good_row_is_read_with_its_emails_in_lower_case(self) -> None:
        parsed, problems = rights.parse(registry("| @Alice | 101 | 1.0 | #7 | Alice@Example.com,alice@work.example |"))
        self.assertEqual(problems, [])
        row = parsed.participants[0]
        self.assertEqual((row.login, row.account_id, row.offer, row.pr, row.emails),
                         ("@Alice", "101", "1.0", "#7", ("alice@example.com", "alice@work.example")))
        self.assertEqual(row.key, "alice")

    def test_a_broken_row_is_refused_with_its_line_number(self) -> None:
        text = registry(ALICE, "| alice2 | x1 | v1 | 7 | not-an-email |")
        line = text.splitlines().index("| alice2 | x1 | v1 | 7 | not-an-email |") + 1
        parsed, problems = rights.parse(text)
        self.assertEqual([row.login for row in parsed.participants], ["@alice"])
        prefix = f"CONTRIBUTORS.md, строка {line}: "
        for message in ("логин «alice2»", "ID «x1»", "версия оферты «v1»", "PR «7»", "почты «not-an-email»"):
            self.assertTrue(any(problem.startswith(prefix) and message in problem for problem in problems), (message, problems))

    def test_a_row_with_the_wrong_number_of_columns_is_refused(self) -> None:
        _, problems = rights.parse(registry("| @alice | 101 | 1.0 | alice@example.com |"))
        self.assertTrue(any("нужно 5 столбцов, а их 4" in problem for problem in problems), problems)
        _, problems = rights.parse(registry("| @alice | 101 | 1.0 | #7 | alice@example.com"))
        self.assertTrue(any("начинаться и заканчиваться «|»" in problem for problem in problems), problems)

    def test_the_header_and_both_sections_are_fixed(self) -> None:
        _, problems = rights.parse(registry(ALICE).replace("| GitHub | ID |", "| Логин | ID |"))
        self.assertTrue(any("заголовок раздела «Участники»" in problem for problem in problems), problems)
        _, problems = rights.parse(registry(ALICE, exempt=""))
        self.assertIn("CONTRIBUTORS.md: нет раздела «## Без оферты»", problems)
        _, problems = rights.parse(registry(ALICE) + "\n## Прочее\n\n| a | b |\n")
        self.assertTrue(any("таблица вне разделов" in problem for problem in problems), problems)

    def test_one_email_stands_in_one_row_only(self) -> None:
        _, problems = rights.parse(registry(ALICE, "| @carol | 303 | 1.0 | #9 | alice@example.com |"))
        self.assertTrue(any("почта alice@example.com уже указана в строке" in problem for problem in problems), problems)
        _, problems = rights.parse(registry("| @carol | 303 | 1.0 | #9 | noreply@anthropic.com |"))
        self.assertTrue(any("почта noreply@anthropic.com уже указана" in problem for problem in problems), problems)
        _, problems = rights.parse(registry(ALICE, "| @ALICE | 101 | 1.0 | #8 | other@example.com |"))
        self.assertTrue(any("@ALICE уже записан в строке" in problem for problem in problems), problems)

    def test_an_exempt_row_needs_who_emails_and_why(self) -> None:
        exempt = EXEMPT_TABLE + "| bot | bot@example.com |  |\n"
        _, problems = rights.parse(registry(exempt=exempt))
        self.assertTrue(any("не указана причина" in problem for problem in problems), problems)


class CheckTests(unittest.TestCase):
    def check(self, request: dict, commits: list[dict], head: str | None, base: str | None = None) -> list[str]:
        return rights.check(request, commits, registry() if base is None else base, head, HOLDER)

    def test_the_pr_needs_the_registry(self) -> None:
        self.assertEqual(self.check(pr(), [], None), ["В PR нет файла CONTRIBUTORS.md"])

    def test_every_commit_author_email_is_in_the_registry(self) -> None:
        head = registry(BOB)
        base = registry(BOB)
        allowed = [commit("bob@example.com"), commit("i@ingvar.pro"), commit("NoReply@Anthropic.com")]
        self.assertEqual(self.check(pr("bob", 202, ""), allowed, head, base), [])
        problems = self.check(pr("bob", 202, ""), [commit("stranger@example.com", sha="1234567890")], head, base)
        self.assertEqual(len(problems), 1)
        self.assertIn("Коммит 1234567: почта автора stranger@example.com не указана в CONTRIBUTORS.md", problems[0])

    def test_every_co_author_email_is_in_the_registry(self) -> None:
        head = registry(BOB)
        known = "Правка\n\nCo-authored-by: Claude <noreply@anthropic.com>\nco-authored-by: Bob <bob@example.com>\n"
        self.assertEqual(self.check(pr("bob", 202, ""), [commit("bob@example.com", message=known)], head, head), [])
        unknown = "Правка\n\nCo-authored-by: Dave <dave@example.com>\n"
        problems = self.check(pr("bob", 202, ""), [commit("bob@example.com", message=unknown)], head, head)
        self.assertEqual(len(problems), 1)
        self.assertIn("почта соавтора dave@example.com не указана", problems[0])

    def test_an_unknown_email_is_reported_once(self) -> None:
        commits = [commit("x@example.com", sha="1" * 10), commit("x@example.com", sha="2" * 10)]
        self.assertEqual(len(self.check(pr("bob", 202, ""), commits, registry(BOB), registry(BOB))), 1)

    def test_the_owner_adds_their_row_with_their_id_and_the_offer_mark(self) -> None:
        self.assertEqual(self.check(pr(), [commit("alice@example.com", "alice")], registry(ALICE)), [])

    def test_someone_else_may_not_add_or_change_a_row(self) -> None:
        problems = self.check(pr("bob", 202), [], registry(ALICE))
        self.assertTrue(any(problem.startswith("Строку @alice") and "меняет только её владелец, а PR открыл @bob" in problem
                            for problem in problems), problems)
        changed = ALICE.replace("#7", "#8")
        problems = self.check(pr("bob", 202), [], registry(changed), registry(ALICE))
        self.assertTrue(any("Строку @alice" in problem for problem in problems), problems)
        problems = self.check(pr("bob", 202), [], registry(), registry(ALICE))
        self.assertTrue(any("Строку @alice (удалена)" in problem for problem in problems), problems)

    def test_rows_the_pr_does_not_touch_need_nothing_from_its_author(self) -> None:
        self.assertEqual(self.check(pr("bob", 202, ""), [commit("bob@example.com")], registry(ALICE, BOB), registry(ALICE, BOB)), [])

    def test_the_row_id_is_the_id_of_the_authors_account(self) -> None:
        problems = self.check(pr(account_id=999), [], registry(ALICE))
        self.assertEqual(problems, ["Строка 9: ID 101 не совпадает с ID аккаунта @alice (999)"])

    def test_the_offer_mark_is_checked_and_its_version_matches_the_row(self) -> None:
        for body in ("", OFFER.replace("[x]", "[ ]"), None):
            with self.subTest(body=body):
                problems = self.check(pr(body=body), [], registry(ALICE))
                self.assertEqual(problems, ["Отметьте в описании PR принятие оферты Unica CLA: блок «Права на вклад» шаблона PR"])
        problems = self.check(pr(body=OFFER.replace("версии 1.0", "версии 2.0")), [], registry(ALICE))
        self.assertEqual(problems, ["Строка 9: оферта версии 1.0, а в описании PR отмечена версия 2.0"])
        self.assertEqual(self.check(pr(body=OFFER.replace("[x]", "[X]")), [], registry(ALICE)), [])

    def test_the_account_github_links_to_an_email_matches_the_row(self) -> None:
        head = registry(ALICE, BOB)
        self.assertEqual(self.check(pr("bob", 202, ""), [commit("bob@example.com", "Bob")], head, head), [])
        self.assertEqual(self.check(pr("bob", 202, ""), [commit("i@ingvar.pro", "zeegin")], head, head), [])
        # Строка без логина: сравнивать не с чем.
        self.assertEqual(self.check(pr("bob", 202, ""), [commit("noreply@anthropic.com", "claude")], head, head), [])
        problems = self.check(pr("bob", 202, ""), [commit("alice@example.com", "bob", sha="feedface00")], head, head)
        self.assertEqual(len(problems), 1)
        self.assertIn("Коммит feedfac: GitHub связывает почту alice@example.com с @bob, а в CONTRIBUTORS.md она записана за @alice",
                      problems[0])
        problems = self.check(pr("bob", 202, ""), [commit("i@ingvar.pro", "bob")], head, head)
        self.assertTrue(any("записана за @zeegin" in problem for problem in problems), problems)

    def test_only_the_rights_holder_changes_the_exempt_table(self) -> None:
        exempt = EXEMPT_TABLE + "| codex | codex@openai.com | агент, работает по поручению правообладателя |\n"
        self.assertEqual(self.check(pr("zeegin", 729213, ""), [], registry(exempt=exempt)), [])
        problems = self.check(pr("bob", 202, ""), [], registry(exempt=exempt))
        self.assertEqual(problems, ["Таблицу «Без оферты» меняет только @zeegin, а PR открыл @bob"])
        self.assertEqual(self.check(pr("bob", 202, ""), [], registry(exempt=EXEMPT_TABLE.replace("агент,", "агент;"))),
                         ["Таблицу «Без оферты» меняет только @zeegin, а PR открыл @bob"])

    def test_a_format_problem_in_the_pr_version_is_reported(self) -> None:
        problems = self.check(pr("bob", 202, ""), [], registry(BOB, "| @carol | 303 |"), registry(BOB))
        self.assertTrue(any(problem.startswith("CONTRIBUTORS.md, строка 10:") for problem in problems), problems)

    def test_the_main_reads_files_and_sets_the_exit_code(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "pr.json").write_text(json.dumps(pr()), encoding="utf-8")
            (root / "commits.jsonl").write_text(json.dumps(commit("alice@example.com")) + "\n", encoding="utf-8")
            (root / "head.md").write_text(registry(ALICE), encoding="utf-8")
            arguments = ["--pr", str(root / "pr.json"), "--commits", str(root / "commits.jsonl"),
                         "--base", str(root / "head.md"), "--head", str(root / "head.md"), "--holder", HOLDER]
            quiet = contextlib.redirect_stdout(io.StringIO())
            with quiet:
                self.assertEqual(rights.main(arguments), 0)
            (root / "commits.jsonl").write_text(json.dumps(commit("x@example.com")) + "\n", encoding="utf-8")
            with quiet:
                self.assertEqual(rights.main(arguments), 1)
            (root / "pr.json").write_text("{", encoding="utf-8")
            with quiet:
                self.assertEqual(rights.main(arguments), 2)


FAKE_GH = """#!{python}
import json, os, sys
args = " ".join(sys.argv[1:])
fixture = json.load(open(os.environ["FAKE_FIXTURE"], encoding="utf-8"))
with open(os.environ["FAKE_ARGS"], "a", encoding="utf-8") as calls:
    calls.write(args + "\\n")
if "contents/scripts/ci/contribution_rights.py?ref=main" in args:
    if fixture["script"] is None:
        sys.exit(1)
    print(fixture["script"], end="")
elif "contents/CONTRIBUTORS.md?ref=" in args:
    ref = args.split("?ref=", 1)[1].split()[0]
    if ref not in fixture["registry"]:
        sys.exit(1)
    print(fixture["registry"][ref], end="")
elif "/commits" in args:
    for item in fixture["commits"]:
        print(json.dumps(item))
elif "/compare/" in args:
    print(fixture["merge_base"])
elif "--jq" in args:
    print(fixture["base"] + " " + fixture["head"])
else:
    print(json.dumps(fixture["pr"]))
"""
BASE = "b" * 40
HEAD = "c" * 40
MERGE_BASE = "d" * 40


class StepTests(unittest.TestCase):
    """Шаг исполняется как есть: подставной `gh` отдаёт данные PR и скрипт из этого дерева."""

    def setUp(self) -> None:
        self.step = next(step for step in yaml.safe_load(METADATA.read_text(encoding="utf-8"))["jobs"]["metadata"]["steps"]
                         if step["name"] == "Права на вклад")

    def run_step(self, *, commits, head=None, script=True, strict=False, **variables):
        bash = shutil.which("bash")
        if bash is None:
            self.skipTest("bash is not available")
        python = Path(sys.executable).as_posix()
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            gh = root / "gh"
            gh.write_text(FAKE_GH.format(python=python), encoding="utf-8", newline="\n")
            gh.chmod(gh.stat().st_mode | stat.S_IEXEC)
            shim = root / "python3"
            shim.write_text(f'#!/usr/bin/env bash\nexec "{python}" "$@"\n', encoding="utf-8", newline="\n")
            shim.chmod(0o755)
            fixture = root / "fixture.json"
            fixture.write_text(json.dumps({
                "pr": pr("bob", 202, ""), "base": BASE, "head": HEAD, "merge_base": MERGE_BASE, "commits": commits,
                # head="" — файла нет в версии PR.
                "registry": {MERGE_BASE: registry(BOB),
                             **({} if head == "" else {HEAD: registry(BOB) if head is None else head})},
                "script": (ROOT / "scripts/ci/contribution_rights.py").read_text(encoding="utf-8") if script else None,
            }), encoding="utf-8")
            environment = {**os.environ, "PATH": f"{root}{os.pathsep}{os.environ['PATH']}", "GITHUB_REPOSITORY": "o/r",
                           "PR_NUMBER": "7", "RIGHTS_HOLDER": self.step["env"]["RIGHTS_HOLDER"],
                           "STRICT": "true" if strict else "false", "FAKE_FIXTURE": str(fixture),
                           "FAKE_ARGS": str(root / "calls"), "TMPDIR": str(root), **variables}
            completed = subprocess.run([bash, "-c", self.step["run"].replace("\r\n", "\n")], cwd=root, env=environment,
                                       capture_output=True, text=True, encoding="utf-8")
            calls = (root / "calls").read_text(encoding="utf-8") if (root / "calls").exists() else ""
        return completed, calls

    def test_the_step_runs_the_script_from_main_and_does_not_fail_yet(self) -> None:
        self.assertEqual(self.step["if"], "${{ !cancelled() }}")
        self.assertEqual(self.step["env"]["STRICT"], "false")
        self.assertEqual(self.step["env"]["RIGHTS_HOLDER"], HOLDER)
        self.assertIn("contents/scripts/ci/contribution_rights.py?ref=main", self.step["run"])
        self.assertIn('python3 -I "$work/check.py"', self.step["run"])
        self.assertNotIn("uses", self.step)

    def test_a_clean_pr_passes(self) -> None:
        completed, calls = self.run_step(commits=[commit("bob@example.com")])
        self.assertEqual(completed.returncode, 0, completed.stdout + completed.stderr)
        self.assertIn("права на вклад в порядке", completed.stdout)
        # Реестр версии PR и общей базы, а не ветки main.
        self.assertIn(f"contents/CONTRIBUTORS.md?ref={HEAD}", calls)
        self.assertIn(f"contents/CONTRIBUTORS.md?ref={MERGE_BASE}", calls)
        self.assertIn(f"compare/{BASE}...{HEAD}", calls)

    def test_problems_are_notices_until_strict(self) -> None:
        completed, _ = self.run_step(commits=[commit("stranger@example.com")])
        self.assertEqual(completed.returncode, 0, completed.stdout + completed.stderr)
        self.assertIn("::notice::Коммит abcdef1: почта автора stranger@example.com не указана", completed.stdout)
        self.assertNotIn("::error::", completed.stdout)
        completed, _ = self.run_step(commits=[commit("stranger@example.com")], strict=True)
        self.assertEqual(completed.returncode, 1)
        self.assertIn("::error::Коммит abcdef1", completed.stdout)

    def test_a_missing_registry_in_the_pr_is_a_problem(self) -> None:
        completed, _ = self.run_step(commits=[], head="", strict=True)
        self.assertEqual(completed.returncode, 1)
        completed, _ = self.run_step(commits=[], head=registry(BOB, "| @carol |"), strict=True)
        self.assertIn("::error::CONTRIBUTORS.md, строка 10:", completed.stdout)

    def test_without_the_script_in_main_nothing_is_checked(self) -> None:
        completed, calls = self.run_step(commits=[commit("stranger@example.com")], script=False)
        self.assertEqual(completed.returncode, 0, completed.stdout + completed.stderr)
        self.assertIn("::notice::Скрипта scripts/ci/contribution_rights.py нет в main", completed.stdout)
        self.assertNotIn("/commits", calls)

    def test_the_queue_checks_the_pr_named_by_its_ref(self) -> None:
        ref = "refs/heads/gh-readonly-queue/main/pr-2494-b8d576dca6e9ffa40b9b10fe006472736fcc5a23"
        completed, calls = self.run_step(commits=[commit("bob@example.com")], GITHUB_REF=ref, PR_NUMBER="")
        self.assertEqual(completed.returncode, 0, completed.stdout + completed.stderr)
        self.assertIn("repos/o/r/pulls/2494/commits", calls)
        completed, _ = self.run_step(commits=[], GITHUB_REF="refs/heads/main", PR_NUMBER="")
        self.assertEqual(completed.returncode, 1)
        self.assertIn("Номер PR не определён", completed.stdout)


if __name__ == "__main__":
    unittest.main()
