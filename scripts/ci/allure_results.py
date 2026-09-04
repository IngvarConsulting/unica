"""Результаты прогона в формате Allure, написанные нами.

Одна запись — один файл `{uuid}-result.json`. Формат описан у Allure как
«файл результата теста»; здесь — ровно те поля, которые читает отчёт: имя,
полное имя, статус, подробности, время, метки.

Почему пишем сами, а не адаптером и не JUnit напрямую, — в замысле площадки:
адаптер для Rust не видит отключённых и ломается под процессом на тест, а
тесты из JUnit приходят без меток и в чужом дереве. Здесь у двух языков
равные права: одно дерево, одни метки, один набор состояний.
"""

from __future__ import annotations

import hashlib
import json
import os
import re
import subprocess
import time
import uuid
import xml.etree.ElementTree as ET
from pathlib import Path

# Причина `#[ignore]` — конструкция языка, а не наше имя: nextest её не отдаёт
# ни в JUnit, ни в списке, а в атрибуте она есть всегда.
IGNORE_ATTRIBUTE = re.compile(
    r'#\[ignore\s*=\s*"((?:[^"\\]|\\.)*)"\]\s*(?:#\[[^\]]*\]\s*)*fn\s+(\w+)\s*\(', re.S
)
# Паника из assert — дефект продукта; любая другая — поломка самого теста.
# JUnit от nextest этого не различает, различаем по тексту.
ASSERTION = re.compile(r"assertion|assert_eq|assert_ne|left == right|left != right", re.I)


def now_ms() -> int:
    return int(time.time() * 1000)


def history_id(full_name: str, runner: str) -> str:
    """Один тест на двух раннерах — два теста, две истории.

    С одним `historyId` Allure счёл бы их повторами одного теста и показал бы
    только последний; результат второго раннера исчез бы.
    """
    return hashlib.md5(f"{full_name}@{runner}".encode("utf-8")).hexdigest()


def record(
    *,
    name: str,
    full_name: str,
    status: str,
    runner: str,
    labels: dict[str, str],
    tags: tuple[str, ...] = (),
    message: str | None = None,
    trace: str | None = None,
    start: int | None = None,
    stop: int | None = None,
) -> dict:
    """Запись результата. Метки — словарь плюс теги, потому что `tag` повторяется."""
    details: dict[str, str] = {}
    if message:
        details["message"] = message
    if trace:
        details["trace"] = trace
    return {
        "uuid": str(uuid.uuid4()),
        "historyId": history_id(full_name, runner),
        "name": name,
        "fullName": full_name,
        "status": status,
        "statusDetails": details,
        "start": start if start is not None else now_ms(),
        "stop": stop if stop is not None else now_ms(),
        "labels": [
            *({"name": key, "value": value} for key, value in labels.items()),
            {"name": "host", "value": runner},
            *({"name": "tag", "value": tag} for tag in tags),
        ],
    }


def write(out: Path, entry: dict) -> Path:
    out.mkdir(parents=True, exist_ok=True)
    path = out / f"{entry['uuid']}-result.json"
    path.write_text(json.dumps(entry, ensure_ascii=False), encoding="utf-8")
    return path


def write_run(out: Path, *, profile: str, runner: str, ecosystem: str) -> Path:
    """Подпись прогона: кто, когда и на чём собрал эти результаты.

    Сайт читает её оттуда, а не из события: у прогона по расписанию
    `head_branch` всегда `main`, даже когда он проверяет релизную линию.
    """
    env = os.environ.get
    repository = env("GITHUB_REPOSITORY", "")
    run_id = env("GITHUB_RUN_ID", "")
    signature = {
        "sha": env("GITHUB_SHA", ""),
        "ref": env("GITHUB_REF_NAME", ""),
        "run_id": run_id,
        "run_attempt": env("GITHUB_RUN_ATTEMPT", ""),
        "run_url": f"https://github.com/{repository}/actions/runs/{run_id}" if repository and run_id else "",
        "runner": runner,
        "os": env("RUNNER_OS", ""),
        "profile": profile,
        "ecosystem": ecosystem,
        "at": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
    }
    out.mkdir(parents=True, exist_ok=True)
    path = out / "run.json"
    path.write_text(json.dumps(signature, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    return path


def ignore_reasons(root: Path) -> dict[str, str]:
    """Имя функции → причина `#[ignore]`, по всем исходникам крейтов."""
    found: dict[str, str] = {}
    for path in (root / "crates").rglob("*.rs"):
        if "target" in path.parts:
            continue
        for reason, name in IGNORE_ATTRIBUTE.findall(path.read_text(encoding="utf-8", errors="replace")):
            found[name] = reason
    return found


def nextest_list(root: Path, profile: str) -> list[dict]:
    """Состав прогона по nextest: двоичный файл, имя, признак `ignored`.

    Это и план прогона, и источник дерева для отчёта. Проигнорированные
    перечисляются тоже: план обязан знать всё, что дерево знает.
    """
    completed = subprocess.run(
        ["cargo", "nextest", "list", "--workspace", "--profile", profile,
         "--run-ignored", "all", "--message-format", "json"],
        cwd=root, capture_output=True, text=True, check=True,
    )
    listed = json.loads(completed.stdout)
    entries = []
    for suite in listed["rust-suites"].values():
        for name, case in suite["testcases"].items():
            entries.append({"binary": suite["binary-id"], "name": name, "ignored": bool(case.get("ignored"))})
    return entries


def write_plan(out: Path, entries: list[dict]) -> Path:
    out.mkdir(parents=True, exist_ok=True)
    path = out / "plan.json"
    path.write_text(json.dumps(entries, ensure_ascii=False, indent=1) + "\n", encoding="utf-8")
    return path


def rust_labels(binary: str, name: str, profile: str) -> dict[str, str]:
    module = name.rsplit("::", 1)[0] if "::" in name else binary
    return {
        "language": "rust",
        "framework": "nextest",
        "parentSuite": "rust",
        "suite": binary,
        "subSuite": module,
        "profile": profile,
    }


def junit_records(
    junit: Path, *, runner: str, profile: str, reasons: dict[str, str], stop: int | None = None
) -> list[dict]:
    """Перевести JUnit от nextest в записи Allure.

    Статус и трейс — из JUnit; причина `#[ignore]` — из атрибута; полное имя
    — двоичный файл и путь теста. Паника из assert — `failed`, прочая —
    `broken`; отключённый — `skipped` с причиной автора.
    """
    root = ET.parse(junit).getroot()
    finished = stop if stop is not None else now_ms()
    entries = []
    for suite in root.iter("testsuite"):
        binary = suite.get("name", "")
        for case in suite.findall("testcase"):
            name = case.get("name", "")
            full_name = f"{binary}::{name}"
            seconds = float(case.get("time", "0") or 0)
            start = finished - int(seconds * 1000)
            failure = case.find("failure")
            error = case.find("error")
            skipped = case.find("skipped")
            message = trace = None
            if skipped is not None:
                fn = name.rsplit("::", 1)[-1]
                reason = reasons.get(fn)
                status = "skipped"
                message = f"отключён автором: {reason}" if reason else (skipped.get("message") or "пропущен")
            elif failure is not None or error is not None:
                node = failure if failure is not None else error
                body = (node.text or "").strip()
                err = case.find("system-err")
                stderr = ((err.text or "") if err is not None else "").strip()
                # Первая содержательная строка — сообщение; всё остальное — трейс.
                message = body.splitlines()[0] if body else (node.get("message") or "тест упал")
                trace = "\n".join(part for part in (body, stderr) if part)
                status = "failed" if ASSERTION.search(trace or message) else "broken"
            else:
                status = "passed"
            entries.append(
                record(
                    name=name,
                    full_name=full_name,
                    status=status,
                    runner=runner,
                    labels=rust_labels(binary, name, profile),
                    tags=(profile,),
                    message=message,
                    trace=trace,
                    start=start,
                    stop=finished,
                )
            )
    return entries
