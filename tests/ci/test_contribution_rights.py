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
BOT_EMAIL = "49699333+dependabot[bot]@users.noreply.github.com"
EXEMPT_TABLE = f"""## Без оферты

| Кто | Почты | Почему | Допустимо в PR от |
| --- | --- | --- | --- |
| Claude | noreply@anthropic.com | агент, работает по поручению правообладателя | @zeegin, участники |
| @dependabot[bot] | {BOT_EMAIL} | бот | @dependabot[bot] |
"""


def registry(*participants: str, exempt: str = EXEMPT_TABLE) -> str:
    rows = "".join(f"{row}\n" for row in participants)
    return (
        "# Участники Unica\n\nТекст.\n\n## Участники\n\n"
        "| GitHub | ID | Оферта | PR | Почты |\n| --- | --- | --- | --- | --- |\n"
        f"{rows}\n{exempt}"
    )


# Первая строка таблицы «Участники» в `registry()` — строка 9 файла.
FIRST_ROW = 9
ALICE = "| @alice | 101 | 1.0 | #7 | alice@example.com, alice@work.example |"
BOB = "| @bob | 202 | 1.0 | #5 | bob@example.com |"
ZEEGIN = "| @zeegin | 729213 | 1.0 | #1 | i@ingvar.pro, 729213+zeegin@users.noreply.github.com |"


def pr(login: str = "alice", account_id: int = 101, body: str | None = OFFER, number: int = 7) -> dict:
    return {"number": number, "user": {"login": login, "id": account_id}, "body": body}


def commit(email: str, login: str | None = None, message: str = "Правка", sha: str = "abcdef1234") -> dict:
    return {"sha": sha, "commit": {"author": {"email": email}, "message": message},
            "author": {"login": login} if login else None}


class ParseTests(unittest.TestCase):
    def test_the_repository_registry_is_well_formed(self) -> None:
        parsed, problems = rights.parse((ROOT / "CONTRIBUTORS.md").read_text(encoding="utf-8"))
        self.assertEqual(problems, [])
        self.assertEqual([(row.login, row.account_id, row.offer) for row in parsed.participants], [("@zeegin", "729213", "1.0")])
        self.assertEqual([(row.who, row.allowed) for row in parsed.exempt], [
            ("Claude", ("@zeegin", "участники")),
            ("codex", ("@zeegin", "участники")),
            ("@dependabot[bot]", ("@dependabot[bot]",)),
        ])

    def test_a_good_row_is_read_with_its_emails_in_lower_case(self) -> None:
        parsed, problems = rights.parse(registry("| @Alice | 101 | 1.0 | #7 | Alice@Example.com,alice@work.example |"))
        self.assertEqual(problems, [])
        row = parsed.participants[0]
        self.assertEqual((row.login, row.account_id, row.offer, row.pr, row.emails),
                         ("@Alice", "101", "1.0", "#7", ("alice@example.com", "alice@work.example")))
        self.assertEqual(row.key, "alice")

    def test_a_broken_row_is_refused_with_its_line_number(self) -> None:
        text = registry(ALICE, "| alice2 | x1 | v1 | 7 | not-an-email |")
        parsed, problems = rights.parse(text)
        self.assertEqual([row.login for row in parsed.participants], ["@alice"])
        prefix = f"CONTRIBUTORS.md, строка {FIRST_ROW + 1}: "
        for message in ("логин «alice2»", "ID «x1»", "версия оферты «v1»", "PR «7»", "почты «not-an-email»"):
            self.assertTrue(any(problem.startswith(prefix) and message in problem for problem in problems), (message, problems))

    def test_a_row_with_the_wrong_number_of_columns_is_refused(self) -> None:
        _, problems = rights.parse(registry("| @alice | 101 | 1.0 | alice@example.com |"))
        self.assertTrue(any("нужно 5 столбцов, а их 4" in problem for problem in problems), problems)
        _, problems = rights.parse(registry("| @alice | 101 | 1.0 | #7 | alice@example.com"))
        self.assertTrue(any("начинаться и заканчиваться «|»" in problem for problem in problems), problems)
        _, problems = rights.parse(registry(exempt=EXEMPT_TABLE + "| bot | bot@example.com | бот |\n"))
        self.assertTrue(any("нужно 4 столбцов, а их 3" in problem for problem in problems), problems)

    def test_the_header_and_both_sections_are_fixed(self) -> None:
        _, problems = rights.parse(registry(ALICE).replace("| GitHub | ID |", "| Логин | ID |"))
        self.assertTrue(any("заголовок раздела «Участники»" in problem for problem in problems), problems)
        _, problems = rights.parse(registry(ALICE).replace(" | Допустимо в PR от |", " |"))
        self.assertTrue(any("заголовок раздела «Без оферты»" in problem for problem in problems), problems)
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

    def test_an_exempt_row_needs_who_emails_why_and_who_may_use_it(self) -> None:
        _, problems = rights.parse(registry(exempt=EXEMPT_TABLE + "| bot | bot@example.com |  | @zeegin |\n"))
        self.assertTrue(any("не указана причина" in problem for problem in problems), problems)
        for allowed in ("", "zeegin", "@zeegin; участники", "все"):
            with self.subTest(allowed=allowed):
                _, problems = rights.parse(registry(exempt=EXEMPT_TABLE + f"| bot | bot@example.com | бот | {allowed} |\n"))
                self.assertTrue(any("перечислите через запятую @login или «участники»" in problem for problem in problems), problems)

    def test_the_pr_column_takes_a_placeholder_until_the_pr_is_opened(self) -> None:
        _, problems = rights.parse(registry("| @alice | 101 | 1.0 | #0 | alice@example.com |"))
        self.assertEqual(problems, [])
        _, problems = rights.parse(registry("| @alice | 101 | 1.0 | #07 | alice@example.com |"))
        self.assertTrue(any("PR «#07»" in problem for problem in problems), problems)


class CheckTests(unittest.TestCase):
    def check(self, request: dict, commits: list[dict], head: str | None, base: str | None = None) -> list[str]:
        return rights.check(request, commits, registry() if base is None else base, head, HOLDER)

    def test_the_pr_needs_the_registry(self) -> None:
        self.assertEqual(self.check(pr(), [], None), ["В PR нет файла CONTRIBUTORS.md"])

    def test_every_commit_author_email_is_in_the_registry(self) -> None:
        head = registry(ZEEGIN, BOB)
        allowed = [commit("bob@example.com"), commit("I@Ingvar.pro"), commit("NoReply@Anthropic.com")]
        self.assertEqual(self.check(pr("bob", 202, ""), allowed, head, head), [])
        problems = self.check(pr("bob", 202, ""), [commit("stranger@example.com", sha="1234567890")], head, head)
        self.assertEqual(problems, [
            "Коммит 1234567: почта автора stranger@example.com не указана в CONTRIBUTORS.md. "
            "Примите оферту Unica CLA (CLA.md): добавьте свою строку с этой почтой в раздел «Участники» "
            "файла CONTRIBUTORS.md и отметьте пункт «Права на вклад» в описании PR"])

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

    def test_an_exempt_email_is_admitted_only_in_prs_of_the_authors_it_names(self) -> None:
        head = registry(ZEEGIN, BOB)
        claude = "Правка\n\nCo-authored-by: Claude <noreply@anthropic.com>\n"
        # «@zeegin» и «участники»: правообладатель и участник со строкой.
        self.assertEqual(self.check(pr("zeegin", 729213, ""), [commit("i@ingvar.pro", message=claude)], head, head), [])
        self.assertEqual(self.check(pr("bob", 202, ""), [commit("bob@example.com", message=claude)], head, head), [])
        self.assertEqual(self.check(pr("bob", 202, ""), [commit("noreply@anthropic.com")], head, head), [])
        # Автор без строки «Участники» с почтой агента — отказ, даже если сама почта в реестре есть.
        problems = self.check(pr("mallory", 404, ""), [commit("noreply@anthropic.com", sha="beefbeef00")], head, head)
        self.assertEqual(problems, [
            "Коммит beefbee: почта автора noreply@anthropic.com из таблицы «Без оферты» допустима только в PR "
            "от @zeegin, участники, а PR открыл @mallory"])
        problems = self.check(pr("mallory", 404, ""), [commit("bob@example.com", message=claude)], head, head)
        self.assertTrue(any("почта соавтора noreply@anthropic.com из таблицы «Без оферты»" in problem for problem in problems),
                        problems)
        # Бот допустим только в своих PR.
        self.assertEqual(self.check(pr("dependabot[bot]", 49699333, ""), [commit(BOT_EMAIL, "dependabot[bot]")], head, head), [])
        problems = self.check(pr("bob", 202, ""), [commit(BOT_EMAIL)], head, head)
        self.assertTrue(any("допустима только в PR от @dependabot[bot], а PR открыл @bob" in problem for problem in problems),
                        problems)

    def test_the_owner_adds_their_row_with_their_id_this_pr_and_the_offer_mark(self) -> None:
        self.assertEqual(self.check(pr(), [commit("alice@example.com", "alice")], registry(ALICE)), [])

    def test_someone_else_may_not_add_or_change_a_row(self) -> None:
        for request in (pr("bob", 202), pr("zeegin", 729213)):
            with self.subTest(author=request["user"]["login"]):
                problems = self.check(request, [], registry(ALICE))
                self.assertIn(f"Строку @alice (строка {FIRST_ROW}) меняет только её владелец, "
                              f"а PR открыл @{request['user']['login']}", problems)
        problems = self.check(pr("bob", 202), [], registry(ALICE.replace("#7", "#8")), registry(ALICE))
        self.assertTrue(any(problem.startswith("Строку @alice") for problem in problems), problems)

    def test_only_the_rights_holder_removes_a_row(self) -> None:
        self.assertEqual(self.check(pr("zeegin", 729213, ""), [], registry(ZEEGIN), registry(ZEEGIN, ALICE)), [])
        for request in (pr(), pr("bob", 202)):
            with self.subTest(author=request["user"]["login"]):
                problems = self.check(request, [], registry(ZEEGIN), registry(ZEEGIN, ALICE))
                self.assertEqual(problems, [f"Строку @alice удалил PR от @{request['user']['login']}: "
                                            "удалять строки «Участники» может только @zeegin"])

    def test_rows_the_pr_does_not_touch_need_nothing_from_its_author(self) -> None:
        self.assertEqual(self.check(pr("bob", 202, ""), [commit("bob@example.com")], registry(ALICE, BOB), registry(ALICE, BOB)), [])

    def test_the_row_id_is_the_id_of_the_authors_account(self) -> None:
        problems = self.check(pr(account_id=999), [], registry(ALICE))
        self.assertEqual(problems, [f"Строка {FIRST_ROW}: ID 101 не совпадает с ID аккаунта @alice (999)"])

    def test_an_added_or_changed_row_names_this_pr(self) -> None:
        self.assertEqual(self.check(pr(number=7), [], registry(ALICE)), [])
        self.assertEqual(self.check(pr(number=12), [], registry(ALICE)),
                         [f"Строка {FIRST_ROW}: в столбце PR #7, а это PR #12"])
        changed = ALICE.replace("alice@work.example", "alice@new.example")
        self.assertEqual(self.check(pr(number=7), [], registry(changed), registry(ALICE)), [])
        self.assertEqual(self.check(pr(number=9), [], registry(changed), registry(ALICE)),
                         [f"Строка {FIRST_ROW}: в столбце PR #7, а это PR #9"])
        placeholder = ALICE.replace("#7", "#0")
        self.assertEqual(self.check(pr(number=7), [], registry(placeholder)),
                         [f"Строка {FIRST_ROW}: в столбце PR #0, а это PR #7"])

    def test_the_offer_mark_is_checked_and_its_version_matches_the_row(self) -> None:
        for body in ("", OFFER.replace("[x]", "[ ]"), None):
            with self.subTest(body=body):
                problems = self.check(pr(body=body), [], registry(ALICE))
                self.assertEqual(problems, ["Отметьте в описании PR принятие оферты Unica CLA: блок «Права на вклад» шаблона PR"])
        problems = self.check(pr(body=OFFER.replace("версии 1.0", "версии 2.0")), [], registry(ALICE))
        self.assertEqual(problems, [f"Строка {FIRST_ROW}: оферта версии 1.0, а в описании PR отмечена версия 2.0"])
        self.assertEqual(self.check(pr(body=OFFER.replace("[x]", "[X]")), [], registry(ALICE)), [])

    def test_the_account_github_links_to_an_email_matches_the_row(self) -> None:
        head = registry(ZEEGIN, ALICE, BOB)
        self.assertEqual(self.check(pr("bob", 202, ""), [commit("bob@example.com", "Bob")], head, head), [])
        self.assertEqual(self.check(pr("zeegin", 729213, ""), [commit("i@ingvar.pro", "zeegin")], head, head), [])
        # Строка без логина: сравнивать не с чем.
        self.assertEqual(self.check(pr("bob", 202, ""), [commit("noreply@anthropic.com", "claude")], head, head), [])
        problems = self.check(pr("bob", 202, ""), [commit("alice@example.com", "bob", sha="feedface00")], head, head)
        self.assertEqual(problems, ["Коммит feedfac: GitHub связывает почту alice@example.com с @bob, "
                                    "а в CONTRIBUTORS.md она записана за @alice (строка 10)"])
        problems = self.check(pr("bob", 202, ""), [commit(BOT_EMAIL, "bob")], head, head)
        self.assertTrue(any("записана за @dependabot[bot]" in problem for problem in problems), problems)

    def test_only_the_rights_holder_changes_the_exempt_table(self) -> None:
        added = EXEMPT_TABLE + "| codex | codex@openai.com | агент, работает по поручению правообладателя | @zeegin, участники |\n"
        reworded = EXEMPT_TABLE.replace("агент,", "агент;")
        widened = EXEMPT_TABLE.replace("| @dependabot[bot] |\n", "| @dependabot[bot], участники |\n")
        for exempt in (added, reworded, widened):
            with self.subTest(exempt=exempt):
                self.assertEqual(self.check(pr("zeegin", 729213, ""), [], registry(exempt=exempt)), [])
                self.assertEqual(self.check(pr("bob", 202, ""), [], registry(exempt=exempt)),
                                 ["Таблицу «Без оферты» меняет только @zeegin, а PR открыл @bob"])

    def test_a_format_problem_in_the_pr_version_is_reported(self) -> None:
        problems = self.check(pr("bob", 202, ""), [], registry(BOB, "| @carol | 303 |"), registry(BOB))
        self.assertTrue(any(problem.startswith(f"CONTRIBUTORS.md, строка {FIRST_ROW + 1}:") for problem in problems), problems)

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

    def run_step(self, *, commits, head=None, script=True, strict=True, **variables):
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

    def test_the_step_runs_the_script_from_main_and_blocks_at_once(self) -> None:
        self.assertEqual(self.step["if"], "${{ !cancelled() }}")
        self.assertEqual(self.step["env"]["STRICT"], "true")
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

    def test_an_unknown_email_fails_the_check_and_says_what_to_do(self) -> None:
        for message in ("Права на вклад\n\nCo-authored-by: Dave <dave@example.com>\n", "Права на вклад"):
            email = "dave@example.com" if "Dave" in message else "stranger@example.com"
            author = "bob@example.com" if "Dave" in message else email
            with self.subTest(email=email):
                completed, _ = self.run_step(commits=[commit(author, message=message)])
                self.assertEqual(completed.returncode, 1, completed.stdout + completed.stderr)
                self.assertIn("::error::Коммит abcdef1: почта", completed.stdout)
                self.assertIn(email, completed.stdout)
                for step in ("Примите оферту Unica CLA", "добавьте свою строку", "отметьте пункт «Права на вклад» в описании PR"):
                    self.assertIn(step, completed.stdout)
                self.assertNotIn("::notice::", completed.stdout)

    def test_a_missing_registry_in_the_pr_is_a_problem(self) -> None:
        completed, _ = self.run_step(commits=[], head="")
        self.assertEqual(completed.returncode, 1)
        self.assertIn("::error::В PR нет файла CONTRIBUTORS.md", completed.stdout)
        completed, _ = self.run_step(commits=[], head=registry(BOB, "| @carol |"))
        self.assertIn(f"::error::CONTRIBUTORS.md, строка {FIRST_ROW + 1}:", completed.stdout)

    def test_without_the_script_in_main_nothing_is_checked_so_the_pr_bringing_it_can_merge(self) -> None:
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
