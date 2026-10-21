#!/usr/bin/env python3
"""Сложить артефакты прогона-источника в результаты линий для сайта.

Джобы оставляют `results-*` с `allure-results` и подписью `run.json`, а
джобы тестов ещё и `plan-*` — план, выгруженный до тестов, с той же подписью:
для Rust — состав из `nextest list`, для Python — состав набора из `discover`.
Здесь всё это складывается по линиям, и по плану дописывается то, до чего
раннер не дошёл: такой тест попадает в отчёт `skipped` с причиной «раннер не
дошёл», а не исчезает из истории.

Линия берётся из подписи каждого артефакта, а не из события: у прогона по
расписанию `head_branch` всегда `main`, даже когда он проверяет релизную
линию, а ночной прогон несёт несколько линий сразу.

Раннер, упавший раньше плана, — сборка не прошла — оставляет подпись без
плана. Состав тогда берётся у сайта: записи последнего хранимого прогона того
же профиля, раннера и подписанной Rust-полосы, и каждый такой тест пишется
`skipped` с причиной. Без доказанной полосы прежние случаи не угадываются. Иначе
в объединении профилей его место заняли бы старые записи под свежей подписью.

Намеренный пропуск Rust для документации оставляет `scope_skip` вместо плана.
Он принимается только вместе с исходом джобы `skipped`. Прежний состав даёт
имена тестов, но не их старые исходы; происхождение состава сохраняется даже
через несколько таких пропусков. Без состава остаётся только метадата пропуска.
Python сохраняет точный состав текущей ревизии в плане: `scope_skip` у medium
отличает намеренный пропуск от сбоя оставшихся small в той же джобе.
"""

from __future__ import annotations

import argparse
import json
import re
import shutil
import sys
import tarfile
import tempfile
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import allure_results  # noqa: E402
from site_fetch import fetch  # noqa: E402

# Имена джоб в workflow: сайт читает их отсюда, а страж сверяет с workflow —
# переименование в одном месте красит тест. Linux исполняет всё в `Unica CI`,
# остальные ОС — каждая в своей джобе `Rust (<runner>)`.
GATE_JOB = "Unica CI"
OS_JOB = "Rust ({runner})"
RUST_LANES = ("ordinary", "receipt", "vendor")
PYTHON_JOB = GATE_JOB


def runner_job(runner: str) -> str:
    return GATE_JOB if runner == "ubuntu-latest" else OS_JOB.format(runner=runner)


RESULTS_PREFIX = "results-"
PLAN_PREFIX = "plan-"

CATEGORIES = [
    {"name": "Только документация", "matchedStatuses": ["skipped"], "messageRegex": "docs-only:.*"},
    {"name": "Инфраструктура: раннер не дошёл", "matchedStatuses": ["skipped"], "messageRegex": "раннер не дошёл.*"},
    {"name": "Отключены автором", "matchedStatuses": ["skipped"], "messageRegex": "отключён автором.*"},
    {"name": "Не в этом ярусе", "matchedStatuses": ["skipped"], "messageRegex": "cadence:.*"},
    {"name": "Поломка теста, а не продукта", "matchedStatuses": ["broken"]},
    {"name": "Дефект продукта", "matchedStatuses": ["failed"]},
]


def load_json(path: Path):
    return json.loads(path.read_text(encoding="utf-8"))


def label(entry: dict, name: str) -> str | None:
    for item in entry.get("labels", []):
        if item.get("name") == name:
            return item.get("value")
    return None


def signed_dirs(artifacts: Path, prefix: str) -> list[tuple[Path, dict]]:
    """Каталоги артефактов с подписью прогона; без подписи каталог не читается.

    Ищутся на любой глубине: ночной прогон выкладывает артефакты запущенных им
    прогонов одним своим, и внутри него лежат те же `results-*` и `plan-*`.
    """
    found = []
    for path in sorted(artifacts.rglob(f"{prefix}*")):
        if path.is_dir() and (path / "run.json").is_file():
            found.append((path, load_json(path / "run.json")))
    return found


def line_of(run: dict, fallback: str) -> str:
    line = run.get("ref") or fallback
    if not line:
        raise SystemExit("линия не определена: ни подписи прогона, ни --fallback-line")
    # Линия — имя ветки без разделителей: `main` или `release-vX.Y`. Ссылка
    # вида `722/merge` приходит с pull request, которого сайт не публикует, и
    # каталог с косой чертой внутри выдал бы вложенный путь вместо линии.
    if "/" in line or line.startswith("."):
        raise SystemExit(f"{line!r} — не имя линии; сайт публикует только ветки без разделителей")
    return line


def job_conclusions(jobs: Path | None) -> dict[str, str]:
    if jobs is None or not jobs.is_file():
        return {}
    listed = load_json(jobs)
    total = listed.get("total_count")
    if isinstance(total, int) and total > len(listed.get("jobs", [])):
        # Обрезанный список — исходы недошедших джоб стали бы «unknown» молча.
        raise SystemExit(f"список джоб обрезан: {len(listed['jobs'])} из {total}; читайте страницу целиком")
    return {job["name"]: (job.get("conclusion") or job.get("status") or "unknown") for job in listed.get("jobs", [])}


def rust_lane(run: dict) -> str | None:
    if "scope_skip" in run:
        if "configuration" in run:
            raise ValueError("scope_skip: whole Rust skip cannot carry a lane configuration")
        return None
    # The current non-archive runner signs the whole invocation, not a lane.
    if "configuration" not in run:
        if "inventoryScope" in run:
            raise ValueError("Rust lane signature lost its configuration")
        return None
    configuration = run["configuration"]
    if not isinstance(configuration, dict) or configuration.get("lane") not in RUST_LANES:
        raise ValueError("Rust signature has no closed lane configuration")
    return configuration["lane"]


def rust_job(run: dict, conclusions: dict[str, str]) -> tuple[str, str]:
    rust_lane(run)  # a signature with an unknown lane is refused, not attributed
    job = runner_job(run.get("runner", ""))
    return job, conclusions.get(job, "unknown")


def validate_run_identities(artifacts: list[tuple[Path, dict]], fallback: str) -> None:
    identities = {}
    configurations = {}
    for path, run in artifacts:
        line = line_of(run, fallback)
        identity = tuple(run.get(field) for field in ("sha", "profile", "run_id", "run_attempt", "run_url"))
        if line in identities and identities[line] != identity:
            raise ValueError(f"inconsistent run identity in {path}")
        identities[line] = identity
        if run.get("ecosystem") in ("rust", "all"):
            lane = rust_lane(run)
            key = (line, run.get("runner", ""), lane)
            if key in configurations and configurations[key] != run.get("configuration"):
                raise ValueError(f"inconsistent Rust configuration in {path}")
            configurations[key] = run.get("configuration")


def empty_seen() -> dict[str, set[str]]:
    return {"rust": set(), "python": set()}


def scope_skips(results: list[tuple[Path, dict]], plans: list[tuple[Path, dict]], fallback: str) -> dict[tuple[str, str], dict]:
    """Проверить явный пропуск: границы diff, исход джобы и отсутствие исполнения."""
    skips = {}
    planned_rust = {(line_of(run, fallback), run.get("runner", "")) for path, run in [*plans, *results]
                    if run.get("ecosystem") in ("rust", "all") and (path / "plan.json").is_file()}
    configured_rust = {(line_of(run, fallback), run.get("runner", "")) for _, run in [*plans, *results]
                       if run.get("ecosystem") in ("rust", "all") and "scope_skip" not in run}
    for path, run in results:
        if "scope_skip" not in run:
            continue
        marker = run["scope_skip"]
        runner = run.get("runner")
        if (
            not isinstance(marker, dict)
            or set(marker) != {"reason", "base_sha", "head_sha"}
            or marker["reason"] != "docs_only"
            or any(not isinstance(marker[key], str) or re.fullmatch(r"[0-9a-f]{40}", marker[key]) is None
                   or marker[key] == "0" * 40 for key in ("base_sha", "head_sha"))
            or marker["base_sha"] == marker["head_sha"]
            or marker["head_sha"] != run.get("sha")
            or run.get("ecosystem") != "rust"
            or run.get("profile") not in ("pr", "queue")
            or not isinstance(runner, str) or not runner
        ):
            raise ValueError(f"scope_skip: malformed or unsupported marker in {path}")
        key = (line_of(run, fallback), runner)
        # The runner's job still runs (guards, Python, routing); the skip is
        # proved by the absence of any Rust plan or result for that runner.
        if key in skips or key in planned_rust or key in configured_rust or (path / "plan.json").exists() or any(path.glob("*-result.json")):
            raise ValueError(f"scope_skip: duplicate marker or execution evidence in {path}")
        for other_path, other in [*results, *plans]:
            if line_of(other, fallback) != key[0]:
                continue
            if any(other.get(field) != run.get(field) for field in ("sha", "profile", "run_id", "run_attempt", "run_url")):
                raise ValueError(f"scope_skip: inconsistent run identity in {other_path}")
            if other.get("runner") == runner and any(
                label(load_json(record), "language") == "rust" for record in other_path.glob("*-result.json")
            ):
                raise ValueError(f"scope_skip: Rust execution evidence in {other_path}")
        skips[key] = {
            "ecosystem": "rust", "runner": runner, "scope_skip": marker,
            "inventory_count": 0, "inventory_sources": [],
        }
    return skips


def python_scope_skips(results: list[tuple[Path, dict]], plans: list[tuple[Path, dict]], fallback: str) -> dict[tuple[str, str, str], dict]:
    """Пропуск medium относится к точному плану suite, а не ко всей Python-job."""
    scoped = {}
    ordinary = set()
    signatures = {}
    routing = {}
    # results может содержать копию плана, уже загруженного как plan-*.
    # Совпадающая копия допустима; другой отбор того же случая — противоречие.
    for path, run in [*plans, *results]:
        if not (path / "plan.json").is_file():
            continue
        line = line_of(run, fallback)
        runner = run.get("runner", "")
        for case in load_json(path / "plan.json"):
            if "scope_skip" not in case:
                if case.get("ecosystem") == "python":
                    ordinary.add((line, runner, case.get("suite"), case.get("id")))
                continue
            if (
                case["scope_skip"] != {"reason": "docs_only"}
                or case.get("ecosystem") != "python"
                or case.get("size") != "medium" or case.get("lane") != "small"
                or case.get("suite") not in ("tests/ci", "tests/dev")
                or not isinstance(case.get("id"), str) or not case["id"]
                or "_FailedTest." in case["id"]
                or run.get("ecosystem") != "python" or run.get("profile") != "queue"
                or not isinstance(runner, str) or not runner
                or not isinstance(run.get("sha"), str)
                or re.fullmatch(r"[0-9a-f]{40}", run["sha"]) is None
                or run["sha"] == "0" * 40
            ):
                raise ValueError(f"scope_skip: malformed Python plan in {path}")
            key = (line, runner, case["suite"], case["id"])
            if key in scoped:
                if scoped[key] != case:
                    raise ValueError(f"scope_skip: inconsistent Python plan for {key}")
                continue
            scoped[key] = case
            signatures[line] = run
            group = (line, runner, case["suite"])
            if group not in routing:
                routing[group] = {
                    "ecosystem": "python", "runner": runner, "suite": case["suite"], "size": "medium",
                    "scope_skip": case["scope_skip"], "inventory_count": 0,
                    "inventory_sources": [{"sha": run["sha"], "run_url": run.get("run_url", "")}],
                }
            routing[group]["inventory_count"] += 1
    if ordinary.intersection(scoped):
        raise ValueError("scope_skip: Python case is also planned for execution")
    for path, run in [*plans, *results]:
        line = line_of(run, fallback)
        if line in signatures and any(
            run.get(field) != signatures[line].get(field)
            for field in ("sha", "profile", "run_id", "run_attempt", "run_url")
        ):
            raise ValueError(f"scope_skip: inconsistent Python run identity in {path}")
    for path, run in results:
        for record_path in path.glob("*-result.json"):
            entry = load_json(record_path)
            key = (line_of(run, fallback), run.get("runner", ""), label(entry, "suite"), entry.get("fullName"))
            marked = label(entry, "scope_skip")
            if key not in scoped and marked is None:
                continue
            if (
                key not in scoped or marked != "docs_only"
                or entry.get("status") != "skipped"
                or run.get("ecosystem") != "python"
                or label(entry, "language") != "python"
                or label(entry, "size") != "medium" or label(entry, "profile") != "queue"
                or label(entry, "host") != run.get("runner")
                or {"name": "runner", "value": run.get("runner")} not in entry.get("parameters", [])
                or not entry.get("statusDetails", {}).get("message", "").startswith("docs-only:")
                or not any(item == {"name": "tag", "value": "docs-only"} for item in entry.get("labels", []))
            ):
                raise ValueError(f"scope_skip: conflicting Python execution in {record_path}")
    return routing


def copy_results(path: Path, run: dict, out: Path) -> tuple[int, dict[str, set[str]]]:
    """Скопировать записи; вернуть счёт и полные имена тестов по языку."""
    seen = empty_seen()
    count = 0
    for record in path.glob("*-result.json"):
        entry = load_json(record)
        language = label(entry, "language")
        if language in ("rust", "python") and run.get("ecosystem") not in (language, "all"):
            raise ValueError(f"result language conflicts with signed identity in {record}")
        if language == "rust":
            lane = rust_lane(run)
            if "scope_skip" in run:
                raise ValueError(f"scope_skip: Rust result in {record}")
            declared = [item.get("value") for item in entry.get("labels", []) if item.get("name") == "rust_lane"]
            if any(value != lane for value in declared):
                raise ValueError(f"Rust result lane conflicts with signed configuration in {record}")
            if lane is not None and not declared:
                entry.setdefault("labels", []).append({"name": "rust_lane", "value": lane})
            (out / record.name).write_text(json.dumps(entry, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
        else:
            shutil.copy2(record, out / record.name)
        count += 1
        if language in seen:
            seen[language].add(entry["fullName"])
    return count, seen


def fill_gaps(plan_dir: Path, run: dict, seen: set[str], out: Path, conclusions: dict[str, str]) -> int:
    """Тест из плана без результата — раннер не дошёл. Записать как `skipped`."""
    runner = run.get("runner", "")
    profile = run.get("profile", "all")
    job, conclusion = rust_job(run, conclusions)
    filled = 0
    # Полоса, которую ворота не принимают, оставляет подпись без плана: пусто.
    if not (plan_dir / "plan.json").is_file():
        return 0
    for case in load_json(plan_dir / "plan.json"):
        if "binary" not in case:
            continue
        full_name = f"{case['binary']}::{case['name']}"
        if full_name in seen:
            continue
        message = f"раннер не дошёл: {job} · {conclusion}"
        if run.get("run_url"):
            message += f" · {run['run_url']}"
        labels = allure_results.rust_labels(case["binary"], case["name"], profile)
        lane = rust_lane(run)
        if lane is not None:
            labels["rust_lane"] = lane
        if "size" in case:
            if case["size"] not in ("small", "medium", "large"):
                raise ValueError(f"unknown Rust plan size: {case['size']!r}")
            labels["size"] = case["size"]
        allure_results.write(
            out,
            allure_results.record(
                name=case["name"],
                full_name=full_name,
                status="skipped",
                runner=runner,
                labels=labels,
                tags=(profile, "infrastructure"),
                message=message,
            ),
        )
        seen.add(full_name)
        filled += 1
    return filled


def fill_python_gaps(plan_dir: Path, run: dict, seen: set[str], out: Path, conclusions: dict[str, str]) -> int:
    """Тест Python из плана без результата — джоба не дошла. Записать как `skipped`."""
    runner = run.get("runner", "")
    profile = run.get("profile", "all")
    filled = 0
    if not (plan_dir / "plan.json").is_file():
        return 0
    for case in load_json(plan_dir / "plan.json"):
        if case.get("ecosystem") != "python" or case["id"] in seen:
            continue
        job = PYTHON_JOB
        conclusion = conclusions.get(job) or "unknown"
        scoped = "scope_skip" in case
        message = (f"docs-only: Python medium пропущен по области изменения · {job}" if scoped
                   else f"раннер не дошёл: {job} · {conclusion}")
        if run.get("run_url"):
            message += f" · {run['run_url']}"
        allure_results.write(
            out,
            allure_results.record(
                name=case["name"],
                full_name=case["id"],
                status="skipped",
                runner=runner,
                labels={
                    "language": "python",
                    "framework": "unittest",
                    "parentSuite": "python",
                    "suite": case["suite"],
                    "subSuite": case.get("subSuite", ""),
                    "profile": profile,
                    "size": case.get("size", "small"),
                    **({"scope_skip": "docs_only"} if scoped else {}),
                },
                tags=(profile, "docs-only" if scoped else "infrastructure"),
                message=message,
            ),
        )
        seen.add(case["id"])
        filled += 1
    return filled


def stored_rust_records(site: str, line: str, profile: str, runner: str, work: Path, lane: str | None) -> tuple[list[dict], dict] | None:
    """Записи Rust этого раннера из последнего хранимого прогона профиля на сайте."""
    archive = work / f"{line}-{profile}.tar.gz"
    if not fetch(f"{site}/data/{line}/results/{profile}.tar.gz", archive):
        return None
    unpacked = work / line / profile
    with tarfile.open(archive) as bundle:
        bundle.extractall(unpacked, filter="data")
    signature = unpacked / "run.json"
    source = load_json(signature) if signature.is_file() else {}
    records: dict[tuple[str, str | None], dict] = {}
    for path in sorted(unpacked.glob("*-result.json")):
        entry = load_json(path)
        declared = [item.get("value") for item in entry.get("labels", []) if item.get("name") == "rust_lane"]
        if len(set(declared)) > 1:
            raise ValueError(f"conflicting stored Rust lane labels in {path}")
        recorded_lane = declared[0] if declared else None
        if (label(entry, "language") == "rust" and label(entry, "host") == runner
                and (lane is None or recorded_lane == lane)):
            records.setdefault((entry["fullName"], recorded_lane), entry)
    return list(records.values()), source


def fill_from_site(site: str, line: str, run: dict, seen: set[str], out: Path, conclusions: dict[str, str], work: Path, routing_skip: dict | None = None, configuration: dict | None = None) -> int:
    """Подпись без плана: прежний состав даёт новые `skipped` с текущей причиной."""
    runner = run.get("runner", "")
    profile = run.get("profile", "all")
    job, conclusion = rust_job(run, conclusions)
    stored = stored_rust_records(site, line, profile, runner, work, rust_lane(run))
    if stored is None:
        return 0
    records, source = stored
    if routing_skip is not None and (source.get("profile", profile) != profile or source.get("ref", line) != line):
        raise ValueError(f"scope_skip: stored Rust inventory identity does not match {line}/{profile}")
    if configuration is not None and records:
        configuration["inventory_status"] = "prior"
        configuration["inventory_count"] = len(records)
    filled = 0
    for entry in records:
        if entry["fullName"] in seen:
            continue
        labels = {item["name"]: item["value"] for item in entry.get("labels", []) if item["name"] not in ("host", "tag")}
        # Прежний архив может сам быть docs-only/недошедшим прогоном. В таком
        # случае он несёт состав дальше, но не становится его новым источником.
        origin = {
            "sha": labels.get("inventory_sha", source.get("sha", "")),
            "run_url": labels.get("inventory_run_url", source.get("run_url", "")),
        }
        labels.update(profile=profile, inventory_sha=origin["sha"], inventory_run_url=origin["run_url"])
        if configuration is not None and origin not in configuration["inventory_sources"]:
            configuration["inventory_sources"].append(origin)
        if routing_skip is not None:
            message = f"docs-only: {job} пропущен по области изменения"
            tags = (profile, "docs-only")
            routing_skip["inventory_count"] += 1
            if origin not in routing_skip["inventory_sources"]:
                routing_skip["inventory_sources"].append(origin)
        else:
            message = f"раннер не дошёл до плана: {job} · {conclusion}"
            tags = (profile, "infrastructure")
        message += f" · состав взят из прогона {origin['sha'][:7] or '?'}"
        if origin["run_url"]:
            message += f" · {origin['run_url']}"
        if run.get("run_url"):
            message += f" · {run['run_url']}"
        allure_results.write(
            out,
            allure_results.record(
                name=entry["name"], full_name=entry["fullName"], status="skipped", runner=runner,
                labels=labels, tags=tags, message=message,
            ),
        )
        filled += 1
    return filled


def properties_line(key: str, value: str) -> str:
    """Java Properties в ASCII: экранирование синтаксиса и UTF-16 `\\uXXXX`."""

    def escape(text: str, *, is_key: bool = False) -> str:
        controls = {"\\": "\\\\", "\t": "\\t", "\n": "\\n", "\r": "\\r", "\f": "\\f"}
        escaped = []
        for index, character in enumerate(text):
            if character in controls:
                escaped.append(controls[character])
            elif (is_key and character in " =:#!") or (index == 0 and character == " "):
                # Пробел завершает ключ; перед значением начальные пробелы
                # отбрасываются до первого непробельного символа/escape.
                escaped.append("\\" + character)
            elif " " <= character <= "~":
                escaped.append(character)
            else:
                # Properties читает ISO-8859-1, а Unicode escape — ровно
                # четыре hex-цифры: non-BMP требует пары UTF-16 surrogates.
                units = character.encode("utf-16-be", errors="surrogatepass").hex()
                escaped.extend("\\u" + units[offset:offset + 4] for offset in range(0, len(units), 4))
        return "".join(escaped)

    return f"{escape(key, is_key=True)}={escape(value)}\n"


def write_metadata(out: Path, run: dict, line: str, runners: list[str], site: str) -> None:
    (out / "categories.json").write_text(json.dumps(CATEGORIES, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    environment = {
        "Ветка": line,
        "Коммит": run.get("sha", ""),
        "Прогон": run.get("run_url", ""),
        "Попытка": run.get("run_attempt", ""),
        "Профиль": run.get("profile", ""),
        "Раннеры": ", ".join(runners),
    }
    for skipped in run.get("routing_skips", []):
        key = (f"Python ({skipped['suite']}, {skipped['size']}, {skipped['runner']})"
               if skipped.get("ecosystem") == "python" else f"Rust ({skipped['runner']})")
        environment[key] = (
            f"docs-only: пропущен; известных тестов {skipped['inventory_count']}"
        )
    (out / "environment.properties").write_text(
        "".join(properties_line(key, value) for key, value in environment.items() if value), encoding="ascii"
    )
    executor = {
        "name": "GitHub Actions",
        "type": "github",
        "url": run.get("run_url", ""),
        "buildOrder": int(run["run_id"]) if str(run.get("run_id", "")).isdigit() else 0,
        "buildName": f"{line} · {run.get('sha', '')[:7]}",
        "buildUrl": run.get("run_url", ""),
        # Без завершающей косой черты: Allure сам дописывает `/#testresult/…`.
        "reportUrl": f"{site}/allure/{line}" if site else "",
    }
    (out / "executor.json").write_text(json.dumps(executor, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    # Подпись линии едет дальше: сборка сайта пишет по ней память о прогоне.
    (out / "run.json").write_text(json.dumps({**run, "ref": line}, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")


def collect(artifacts: Path, out_root: Path, jobs: Path | None, fallback_line: str, site: str) -> dict[str, dict]:
    """Сложить подписи, планы и исходы, не смешивая Rust-конфигурации."""
    results = signed_dirs(artifacts, RESULTS_PREFIX)
    plans = signed_dirs(artifacts, PLAN_PREFIX)
    signed = [*plans, *results]
    if not signed:
        raise SystemExit(f"в {artifacts} нет ни одного results-* или plan-* с подписью: складывать нечего")
    conclusions = job_conclusions(jobs)
    # Scope markers have their own identity checks and diagnostics.
    skips = scope_skips(results, plans, fallback_line)
    python_skips = python_scope_skips(results, plans, fallback_line)
    validate_run_identities(signed, fallback_line)
    by_line = {}
    seen = {}
    configurations = {}
    configuration_runs = {}
    inventory = {}
    for path, run in signed:
        line = line_of(run, fallback_line)
        out = out_root / line
        if line not in by_line:
            if out.exists():
                shutil.rmtree(out)
            out.mkdir(parents=True)
            by_line[line] = {"copied": 0, "filled": 0, "runners": set(), "run": run}
        if run.get("runner"):
            by_line[line]["runners"].add(run["runner"])
        if run.get("ecosystem") not in ("rust", "all") or "scope_skip" in run:
            continue
        lane = rust_lane(run)
        key = (line, run.get("runner", ""), lane)
        if key not in configurations:
            job, conclusion = rust_job(run, conclusions)
            configurations[key] = {
                "runner": key[1], "lane": lane, "configuration": run.get("configuration"),
                "scope": "whole_invocation" if lane is None else "lane",
                "signature_artifacts": [], "has_plan": False,
                "inventory_status": "unknown", "inventory_count": 0, "inventory_sources": [],
                "result_count": 0, "observed_statuses": {},
                "job": {"name": job, "conclusion": conclusion},
            }
            configuration_runs[key] = run
            inventory[key] = set()
        config = configurations[key]
        config["signature_artifacts"].append(path.relative_to(artifacts).as_posix())
        if (path / "plan.json").is_file():
            config["has_plan"] = True
            config["inventory_status"] = "current"
            inventory[key].update(f"{case['binary']}::{case['name']}" for case in load_json(path / "plan.json") if "binary" in case)
            config["inventory_count"] = len(inventory[key])
            origin = {"sha": run.get("sha", ""), "run_url": run.get("run_url", "")}
            if origin not in config["inventory_sources"]:
                config["inventory_sources"].append(origin)
    for path, run in results:
        line = line_of(run, fallback_line)
        lane = rust_lane(run) if run.get("ecosystem") in ("rust", "all") else None
        key = (line, run.get("runner", ""), lane)
        copied, names = copy_results(path, run, out_root / line)
        by_line[line]["copied"] += copied
        bucket = seen.setdefault(key, empty_seen())
        for language, found in names.items():
            bucket[language].update(found)
        if key in configurations:
            config = configurations[key]
            for record in path.glob("*-result.json"):
                entry = load_json(record)
                if label(entry, "language") == "rust":
                    config["result_count"] += 1
                    status = entry.get("status", "unknown")
                    config["observed_statuses"][status] = config["observed_statuses"].get(status, 0) + 1
    for line, host in skips:
        if any(names["rust"] for (seen_line, seen_host, _), names in seen.items() if (seen_line, seen_host) == (line, host)):
            raise ValueError(f"scope_skip: Rust execution evidence for {(line, host)}")
    for path, run in signed:
        if not (path / "plan.json").is_file():
            continue
        line = line_of(run, fallback_line)
        lane = rust_lane(run) if run.get("ecosystem") in ("rust", "all") else None
        have = seen.setdefault((line, run.get("runner", ""), lane), empty_seen())
        if run.get("ecosystem") == "python":
            by_line[line]["filled"] += fill_python_gaps(path, run, have["python"], out_root / line, conclusions)
        else:
            by_line[line]["filled"] += fill_gaps(path, run, have["rust"], out_root / line, conclusions)
    with tempfile.TemporaryDirectory(prefix="stored-") as work:
        for key, configuration in configurations.items():
            if configuration["has_plan"] or not site:
                continue
            run = configuration_runs[key]
            by_line[key[0]]["filled"] += fill_from_site(
                site, key[0], run, seen.get(key, empty_seen())["rust"], out_root / key[0], conclusions,
                Path(work), configuration=configuration,
            )
        for key, skip in skips.items():
            if site:
                run = next(run for _, run in results if "scope_skip" in run and (line_of(run, fallback_line), run.get("runner", "")) == key)
                by_line[key[0]]["filled"] += fill_from_site(
                    site, key[0], run, set(), out_root / key[0], conclusions, Path(work), routing_skip=skip,
                )
    for line, stats in by_line.items():
        stats["runners"] = sorted(stats["runners"])
        run = dict(stats.pop("run"))
        run["rust_configurations"] = [config for key, config in sorted(configurations.items(), key=lambda item: (item[0][0], item[0][1], item[0][2] or "")) if key[0] == line]
        routing = [skip for key, skip in sorted(skips.items()) if key[0] == line]
        routing.extend(skip for key, skip in sorted(python_skips.items()) if key[0] == line)
        if routing:
            run["routing_skips"] = routing
            run.pop("scope_skip", None)
        write_metadata(out_root / line, run, line, stats["runners"], site)
    return by_line


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--artifacts", type=Path, required=True, help="каталог со скачанными артефактами")
    parser.add_argument("--out", type=Path, required=True, help="куда сложить: <out>/<линия>/")
    parser.add_argument("--jobs", type=Path, default=None, help="ответ API со списком джоб прогона")
    parser.add_argument("--fallback-line", default="", help="линия, если подписи нет")
    parser.add_argument("--site", default="", help="адрес сайта для ссылки на отчёт")
    args = parser.parse_args(argv)

    lines = collect(args.artifacts, args.out, args.jobs, args.fallback_line, args.site)
    for line, stats in lines.items():
        print(
            f"{line}: записей {stats['copied']}, дописано недошедших {stats['filled']}, раннеры {', '.join(stats['runners'])}",
            file=sys.stderr,
        )
        print(line)
    return 0


if __name__ == "__main__":
    sys.exit(main())
