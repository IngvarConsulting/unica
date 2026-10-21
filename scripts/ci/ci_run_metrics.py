#!/usr/bin/env python3
"""Собрать метрики завершённых прогонов `Unica CI` по списку run id.

Методика #1816 сравнивает серии «до/после» по одним и тем же величинам:
время до `Unica CI`, runner-minutes, фазы compile/link, execution и transfer,
ожидание раннера и попадание кеша. Всё это уже есть в GitHub: в jobs/steps
прогона, в логах джоб и в артефактах `ci_rust_lanes`. Скрипт только читает
их через `gh api` и ничего не запускает.

Источники разной точности не смешиваются, а лежат рядом:

* jobs API — время создания, старта и завершения джобы и её шагов (секунды);
  ожидание раннера — старт джобы минус её создание: GitHub создаёт джобу,
  когда завершились её `needs`;
* логи — образ раннера, toolchain, исход rust-cache и строки
  `Finished … in …` Cargo (compile/link; так измеряется и прогон до #1861,
  у которого ещё не было артефактов полос);
* `lane.json`/`shard.json` полос — build/list/execution из самого раннера.

Ручной запуск, в отличие от очереди, гонит поставочный хвост до `Unica CI`.
Поэтому рядом со временем до `Unica CI` считается время до последней
непоставочной джобы: только оно сравнимо между событиями.
"""

from __future__ import annotations

import argparse
from concurrent.futures import ThreadPoolExecutor
import csv
from datetime import datetime
import hashlib
import io
import json
import math
from pathlib import Path
import re
import statistics
import subprocess
import sys
import time
from typing import Callable
import zipfile

GATE_JOB = "Unica CI"
# Ответ, который не изменится от повтора. 403/429 бывают и от ограничения
# частоты API — их повторяем, а не принимаем за отсутствие данных.
FINAL = re.compile(r"HTTP 40[0124]|HTTP 41[0-9]|HTTP 422")
GONE = re.compile(r"HTTP 404|HTTP 410")
# Поставочный хвост: идёт после ворот только на теге и ручном запуске.
DELIVERY_TAIL = (
    "Build tools", "Package thin", "P0 dry RC", "Probe thin bootstrap", "Assess Linux runtime",
    "Publish runtime assets", "Smoke published thin plugin", "Re-download and verify",
)
# Джобы, чьи логи нужны для образа, toolchain, кеша и строк компиляции.
RUST_JOB = re.compile(r"^(Rust |Build Rust |Warm Rust |Ordinary binary)")
# Rust-тесты исполнялись: до #1861 — `Rust tests (<ОС>)`, после — полосы.
RUST_TESTS = re.compile(r"^(Rust tests \((?!complete)|Rust ordinary \()")

FINISHED = re.compile(r"Finished\S* `[^`]+` profile \[[^\]]*\] target\(s\) in ((?:\d+h )?(?:\d+m )?[\d.]+s)")
TOOLCHAIN = re.compile(r"\brustc (\d+\.\d+\.\d+(?:-[\w.]+)?) \((\w+) (\d{4}-\d\d-\d\d)\)")
NEXTEST = re.compile(r"\bcargo-nextest (\d+\.\d+\.\d+)")
PROFILE = re.compile(r"^\s*GATE_PROFILE: (\S+)\s*$")
LOG_TIME = re.compile(r"^\d{4}-\d\d-\d\dT[\d:.]+Z ?")


def parse_time(value: str | None) -> datetime | None:
    return datetime.fromisoformat(value.replace("Z", "+00:00")) if value else None


def seconds_between(start: str | None, end: str | None) -> float | None:
    begin, finish = parse_time(start), parse_time(end)
    return None if begin is None or finish is None else (finish - begin).total_seconds()


def cargo_seconds(text: str) -> float:
    """`1h 2m 05s`, `3m 26s`, `17.43s` — длительность строки `Finished` Cargo."""
    total = 0.0
    for amount, unit in re.findall(r"([\d.]+)([hms])", text):
        total += float(amount) * {"h": 3600, "m": 60, "s": 1}[unit]
    return total


def runner_os(labels: list[str]) -> str:
    joined = " ".join(labels).lower()
    for name in ("macos", "windows", "ubuntu"):
        if name in joined:
            return {"ubuntu": "linux"}.get(name, name)
    return "other"


def step_phase(name: str) -> str:
    """Отнести шаг jobs API к фазе. Неизвестное — `work`: это и есть работа джобы."""
    lowered = name.lower()
    if lowered in {"set up job", "complete job"}:
        return "setup"
    if "upload-artifact" in lowered or "download-artifact" in lowered:
        return "transfer"
    if "rust-cache" in lowered or "actions/cache" in lowered or "cache" in lowered or "кэш" in lowered:
        return "cache"
    if lowered.startswith("post "):
        return "setup"
    if any(tool in lowered for tool in ("actions/checkout", "rust-toolchain", "setup-python", "install-action", "pip install")):
        return "setup"
    return "work"


def executed(job: dict) -> bool:
    return bool(job.get("runner_name")) and job.get("started_at") is not None and job.get("conclusion") != "skipped"


def is_tail(name: str) -> bool:
    return name.startswith(DELIVERY_TAIL)


def parse_log(text: str) -> dict:
    """Образ, toolchain, nextest, профиль, кеш и строки компиляции из лога джобы."""
    lines = [LOG_TIME.sub("", line, count=1) for line in text.splitlines()]
    image: dict = {}
    in_image = False
    for line in lines[:80]:
        if line.startswith("##[group]Runner Image") and "Provisioner" not in line:
            in_image = True
        elif in_image and line.startswith("##[endgroup]"):
            break
        elif in_image and ": " in line:
            key, value = line.split(": ", 1)
            if key in {"Image", "Version"}:
                image[key.lower()] = value.strip()
    toolchain = next((name for name in map(toolchain_name, lines) if name), None)
    nextest = next((match for match in map(NEXTEST.search, lines) if match), None)
    profile = next((match.group(1) for match in map(PROFILE.match, lines) if match), None)
    return {
        "image": image or None,
        "toolchain": toolchain,
        "nextest": None if nextest is None else nextest.group(1),
        "profile": profile,
        "cache": parse_cache(lines),
        "cargoFinishedSeconds": [cargo_seconds(match.group(1)) for match in map(FINISHED.search, lines) if match],
    }


def parse_cache(lines: list[str]) -> list[dict]:
    """Исход каждого шага rust-cache/actions/cache в логе, по порядку.

    `lookup-only` проверяет ключ без восстановления: попадание в таком режиме
    не ускоряет сборку, и потому записывается отдельно от `restored`.
    """
    entries: list[dict] = []
    current: dict | None = None
    for line in lines:
        if line.startswith("##[group]Run ") and ("rust-cache" in line or "actions/cache" in line):
            current = {"action": "rust-cache" if "rust-cache" in line else "actions/cache",
                       "lookupOnly": False, "key": None, "match": "none", "restored": False}
            entries.append(current)
            continue
        if current is None:
            continue
        stripped = line.strip()
        if stripped == "lookup-only: true":
            current["lookupOnly"] = True
        elif stripped.startswith("Cache hit for: "):
            current["key"], current["match"] = stripped.removeprefix("Cache hit for: "), "exact"
        elif stripped.startswith("Cache hit for restore-key: "):
            current["key"], current["match"] = stripped.removeprefix("Cache hit for restore-key: "), "partial"
        elif match := re.match(r'(Restored from|Found) cache key "([^"]+)" full match: (true|false)\.', stripped):
            current.update(key=match.group(2), restored=match.group(1) == "Restored from",
                           match="exact" if match.group(3) == "true" else "partial")
        elif match := re.match(r"Cache restored from key: (\S+)", stripped):
            # actions/cache пишет эту строку при любом совпадении: partial
            # из `Cache hit for restore-key` не повышается до exact.
            current.update(key=match.group(1), restored=True,
                           match="partial" if current["match"] == "partial" else "exact")
        elif stripped == "No cache found." or stripped.startswith("Cache not found for input keys"):
            current["match"] = "none"
        elif stripped.startswith("##[group]Run ") or stripped.startswith("Post job cleanup"):
            current = None
    return entries


def job_metrics(job: dict, log: dict | None) -> dict:
    steps = job.get("steps") or []
    phases: dict[str, float] = {}
    for step in steps:
        duration = seconds_between(step.get("started_at"), step.get("completed_at"))
        if duration is not None and step.get("conclusion") != "skipped":
            phase = step_phase(step.get("name", ""))
            phases[phase] = phases.get(phase, 0.0) + duration
    return {
        "id": job.get("id"), "name": job["name"], "os": runner_os(job.get("labels") or []),
        "labels": job.get("labels") or [], "conclusion": job.get("conclusion"),
        "createdAt": job.get("created_at"), "startedAt": job.get("started_at"), "completedAt": job.get("completed_at"),
        "waitSeconds": seconds_between(job.get("created_at"), job.get("started_at")),
        "durationSeconds": seconds_between(job.get("started_at"), job.get("completed_at")),
        "phaseSeconds": phases, "log": log,
    }


def toolchain_name(text: str | None) -> str | None:
    """Одна запись toolchain для лога и `identity` полосы: `1.99.1 58cf8f9c3 2026-10-13`."""
    match = TOOLCHAIN.search(text or "")
    return None if match is None else " ".join(match.groups())


def lane_metrics(document: dict) -> dict:
    identity = document.get("identity") or {}
    return {
        "runner": document.get("runner"), "lane": document.get("lane"), "profile": document.get("profile"),
        "buildSeconds": document.get("buildSeconds"), "listSeconds": document.get("listSeconds"),
        "archiveBytes": document.get("archiveBytes"), "tests": len(document.get("inventory") or []),
        "shards": len(document.get("shards") or []), "sha": identity.get("sha"), "target": identity.get("target"),
        "toolchain": toolchain_name(identity.get("toolchain")),
        "nextest": (document.get("nextestVersion") or "").split("\n", 1)[0] or None,
        "attempt": document.get("attempt"),
    }


def shard_metrics(document: dict) -> dict:
    return {
        "runner": document.get("runner"), "lane": document.get("lane"), "shard": document.get("shard"),
        "executionSeconds": document.get("executionSeconds"), "exitCode": document.get("exitCode"),
        "tests": len(document.get("plan") or []), "attempt": document.get("attempt"),
    }


def run_metrics(run: dict, jobs: list[dict], logs: dict[int, dict], lanes: list[dict], shards: list[dict]) -> dict:
    """Метрики одного прогона из уже прочитанных документов GitHub."""
    attempt = run.get("run_attempt") or 1
    # Повторная попытка стартует позже создания run: считать от её старта.
    origin = run["created_at"] if attempt == 1 else run.get("run_started_at") or run["created_at"]
    measured = [job_metrics(job, logs.get(job.get("id"))) for job in jobs]
    # Перезапуск упавших переносит успешные джобы прошлой попытки: их время
    # уже учтено там и к этой попытке не относится.
    ran = [job for job, raw in zip(measured, jobs)
           if executed(raw) and (raw.get("run_attempt") or attempt) == attempt]
    gate = next((job for job in measured if job["name"] == GATE_JOB), None)
    gate_jobs = [job for job in ran if job["name"] != GATE_JOB and not is_tail(job["name"])]
    last = max(gate_jobs, key=lambda job: job["completedAt"], default=None)
    minutes: dict[str, float] = {}
    for job in ran:
        minutes[job["os"]] = minutes.get(job["os"], 0.0) + (job["durationSeconds"] or 0.0) / 60
    phases: dict[str, float] = {}
    for job in ran:
        for phase, value in job["phaseSeconds"].items():
            phases[phase] = phases.get(phase, 0.0) + value
    caches = [dict(entry, job=job["name"]) for job in ran if job["log"] for entry in job["log"]["cache"]]
    finished = [value for job in ran if job["log"] for value in job["log"]["cargoFinishedSeconds"]]
    identity = {
        "images": sorted({f"{job['log']['image'].get('image')} {job['log']['image'].get('version')}"
                          for job in ran if job["log"] and job["log"]["image"]}),
        "toolchains": sorted({job["log"]["toolchain"] for job in ran if job["log"] and job["log"]["toolchain"]}
                             | {lane["toolchain"] for lane in lanes if lane["toolchain"]}),
        "nextest": sorted({job["log"]["nextest"] for job in ran if job["log"] and job["log"]["nextest"]}),
        "targets": sorted({lane["target"] for lane in lanes if lane["target"]}),
        "profiles": sorted({job["log"]["profile"] for job in ran if job["log"] and job["log"]["profile"]}),
    }
    return {
        "runId": run["id"], "attempt": attempt, "event": run.get("event"), "headSha": run.get("head_sha"),
        "headBranch": run.get("head_branch"), "status": run.get("status"), "conclusion": run.get("conclusion"),
        "createdAt": run["created_at"], "origin": origin, "url": run.get("html_url"),
        "unicaCi": None if gate is None else {
            "conclusion": gate["conclusion"], "completedAt": gate["completedAt"],
            "seconds": seconds_between(origin, gate["completedAt"]) if gate["conclusion"] != "skipped" else None,
        },
        "gateEquivalentSeconds": None if last is None else seconds_between(origin, last["completedAt"]),
        "lastGateJob": None if last is None else last["name"],
        "deliveryTailRan": any(is_tail(job["name"]) for job in ran),
        # Документационная очередь пропускает Rust: её время не сравнимо с кодовой.
        "rustTestsRan": any(RUST_TESTS.match(job["name"]) for job in ran),
        "runnerMinutes": {"total": sum(minutes.values()), **minutes},
        "runnerWait": {
            "maxSeconds": max((job["waitSeconds"] or 0.0 for job in ran), default=0.0),
            "sumSeconds": sum(job["waitSeconds"] or 0.0 for job in ran),
            "macosMaxSeconds": max((job["waitSeconds"] or 0.0 for job in ran if job["os"] == "macos"), default=0.0),
        },
        "phaseSeconds": phases,
        "cargoFinished": {"count": len(finished), "sumSeconds": sum(finished), "maxSeconds": max(finished, default=0.0)},
        "cache": caches,
        "cacheRestored": {"exact": sum(1 for entry in caches if entry["restored"] and entry["match"] == "exact"),
                          "partial": sum(1 for entry in caches if entry["restored"] and entry["match"] == "partial"),
                          "lookupOnly": sum(1 for entry in caches if entry["lookupOnly"]),
                          "miss": sum(1 for entry in caches if entry["match"] == "none")},
        "lanes": lanes, "shards": shards,
        "laneTotals": {
            "buildSecondsMax": max((lane["buildSeconds"] or 0.0 for lane in lanes), default=None),
            "executionSecondsMax": max((shard["executionSeconds"] or 0.0 for shard in shards), default=None),
            "archiveBytes": sum(lane["archiveBytes"] or 0 for lane in lanes) or None,
        },
        "identity": identity, "jobs": measured,
    }


def nearest_rank(values: list[float], fraction: float) -> float:
    """Наблюдаемый перцентиль ближайшего ранга: при n ≤ 20 p95 — максимум."""
    ordered = sorted(values)
    return ordered[max(1, math.ceil(fraction * len(ordered))) - 1]


METRICS: dict[str, Callable[[dict], float | None]] = {
    "unicaCiSeconds": lambda run: (run["unicaCi"] or {}).get("seconds"),
    "gateEquivalentSeconds": lambda run: run["gateEquivalentSeconds"],
    "runnerMinutes": lambda run: run["runnerMinutes"]["total"],
    "macosRunnerMinutes": lambda run: run["runnerMinutes"].get("macos", 0.0),
    "runnerWaitMaxSeconds": lambda run: run["runnerWait"]["maxSeconds"],
    "transferSeconds": lambda run: run["phaseSeconds"].get("transfer", 0.0),
    "cacheSeconds": lambda run: run["phaseSeconds"].get("cache", 0.0),
    "cargoFinishedSumSeconds": lambda run: run["cargoFinished"]["sumSeconds"],
    "laneBuildSecondsMax": lambda run: run["laneTotals"]["buildSecondsMax"],
    "laneExecutionSecondsMax": lambda run: run["laneTotals"]["executionSecondsMax"],
}


def describe(values: list[float]) -> dict | None:
    if not values:
        return None
    return {"n": len(values), "p50": statistics.median(values), "p95": nearest_rank(values, 0.95),
            "min": min(values), "max": max(values)}


def summarize(runs: list[dict], pairs: tuple[str, str] | None = None, *, include_failed: bool = False) -> dict:
    """p50 и наблюдаемый p95 по каждой группе; при парах — по разностям «после − до».

    Упавший или отменённый прогон и прогон без Rust-тестов кончаются раньше
    и тянули бы p50 вниз: по умолчанию они в статистику не входят, но видны
    в `excluded` с причиной.
    """
    def reason(run: dict) -> str | None:
        if not run.get("rustTestsRan", True):
            return "без Rust-тестов"
        if run.get("conclusion") != "success" and not include_failed:
            return f"conclusion {run.get('conclusion')}"
        return None

    def counted(run: dict) -> bool:
        return reason(run) is None

    ordered: dict[str, list[dict]] = {}
    for run in runs:
        ordered.setdefault(run["label"], []).append(run)
    groups = {label: [run for run in members if counted(run)] for label, members in ordered.items()}
    excluded = {label: [{"runId": run["runId"], "reason": reason(run)} for run in members if not counted(run)]
                for label, members in ordered.items()}
    summary: dict = {"groups": {}}
    for label, members in groups.items():
        summary["groups"][label] = {
            "runs": len(members), "excluded": excluded.get(label, []),
            "conditions": {key: sorted({value for run in members for value in run["identity"][key]})
                           for key in ("images", "toolchains", "nextest", "targets", "profiles")},
            "events": sorted({run["event"] for run in members}),
            "deliveryTailRan": sum(1 for run in members if run["deliveryTailRan"]),
            "metrics": {name: describe([value for run in members if (value := metric(run)) is not None])
                        for name, metric in METRICS.items()},
        }
    if pairs is not None:
        before, after = (ordered.get(label, []) for label in pairs)
        if len(before) != len(after):
            raise ValueError(f"пары {pairs[0]}/{pairs[1]}: {len(before)} и {len(after)} прогонов")
        # Пара входит целиком или не входит: иначе разности сдвинутся.
        kept = [(x, y) for x, y in zip(before, after) if counted(x) and counted(y)]
        deltas = {}
        for name, metric in METRICS.items():
            values = [b - a for a, b in ((metric(x), metric(y)) for x, y in kept)
                      if a is not None and b is not None]
            deltas[name] = describe(values)
        summary["pairedDelta"] = {"before": pairs[0], "after": pairs[1], "pairs": len(kept),
                                  "droppedPairs": len(before) - len(kept), "metrics": deltas}
    return summary


TSV_COLUMNS = (
    "label", "runId", "attempt", "event", "headSha", "conclusion", "unicaCiSeconds", "gateEquivalentSeconds",
    "lastGateJob", "deliveryTailRan", "runnerMinutes", "linuxMinutes", "macosMinutes", "runnerWaitMaxSeconds",
    "transferSeconds", "cacheSeconds", "cargoFinishedSumSeconds", "laneBuildSecondsMax", "laneExecutionSecondsMax",
    "cacheExact", "cachePartial", "cacheLookupOnly", "cacheMiss", "images", "toolchains",
)


def rounded(value: float | None) -> float | None:
    return None if value is None else round(value, 2)


def tsv_row(run: dict) -> dict:
    return {
        "label": run["label"], "runId": run["runId"], "attempt": run["attempt"], "event": run["event"],
        "headSha": run["headSha"], "conclusion": run["conclusion"],
        "unicaCiSeconds": METRICS["unicaCiSeconds"](run), "gateEquivalentSeconds": run["gateEquivalentSeconds"],
        "lastGateJob": run["lastGateJob"], "deliveryTailRan": run["deliveryTailRan"],
        "runnerMinutes": round(run["runnerMinutes"]["total"], 2),
        "linuxMinutes": round(run["runnerMinutes"].get("linux", 0.0), 2),
        "macosMinutes": round(run["runnerMinutes"].get("macos", 0.0), 2),
        "runnerWaitMaxSeconds": run["runnerWait"]["maxSeconds"],
        "transferSeconds": run["phaseSeconds"].get("transfer", 0.0), "cacheSeconds": run["phaseSeconds"].get("cache", 0.0),
        "cargoFinishedSumSeconds": round(run["cargoFinished"]["sumSeconds"], 2),
        "laneBuildSecondsMax": rounded(run["laneTotals"]["buildSecondsMax"]),
        "laneExecutionSecondsMax": rounded(run["laneTotals"]["executionSecondsMax"]),
        "cacheExact": run["cacheRestored"]["exact"], "cachePartial": run["cacheRestored"]["partial"],
        "cacheLookupOnly": run["cacheRestored"]["lookupOnly"], "cacheMiss": run["cacheRestored"]["miss"],
        "images": "; ".join(run["identity"]["images"]), "toolchains": "; ".join(run["identity"]["toolchains"]),
    }


class GitHub:
    """`gh api` с локальным кешем ответов: завершённый прогон не меняется."""

    def __init__(self, repo: str, cache: Path | None, api: Callable[[str], bytes] | None = None,
                 sleep: Callable[[float], None] = time.sleep, attempts: int = 4) -> None:
        self.repo, self.cache = repo, cache
        self.api = api or self._gh
        self.sleep, self.attempts = sleep, attempts

    @staticmethod
    def _gh(path: str) -> bytes:
        completed = subprocess.run(["gh", "api", path], capture_output=True, check=False)
        if completed.returncode != 0:
            raise RuntimeError(f"gh api {path}: {completed.stderr.decode(errors='replace').strip()}")
        return completed.stdout

    def get(self, path: str, *, cached: bool = True) -> bytes:
        target = None if self.cache is None else self.cache / hashlib.sha256(path.encode()).hexdigest()
        if cached and target is not None and target.is_file():
            return target.read_bytes()
        body = self.fetch(path)
        if cached and target is not None:
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_bytes(body)
        return body

    def fetch(self, path: str) -> bytes:
        """Повторить транзиентный отказ (таймаут, 5xx, 403/429); прочие 4xx — ответ."""
        for attempt in range(1, self.attempts + 1):
            try:
                return self.api(path)
            except RuntimeError as error:
                if FINAL.search(str(error)) or attempt == self.attempts:
                    raise
                self.sleep(2.0 * attempt)
        raise AssertionError("unreachable")

    def json(self, path: str, *, cached: bool = True):
        return json.loads(self.get(path, cached=cached))

    def run(self, run_id: int, attempt: int | None) -> dict:
        suffix = "" if attempt is None else f"/attempts/{attempt}"
        return self.json(f"repos/{self.repo}/actions/runs/{run_id}{suffix}", cached=False)

    def paged(self, path: str, field: str, *, cached: bool = True) -> list[dict]:
        items: list[dict] = []
        page = 1
        while True:
            document = self.json(f"{path}{'&' if '?' in path else '?'}per_page=100&page={page}", cached=cached)
            items.extend(document[field])
            if len(items) >= document["total_count"] or not document[field]:
                return items
            page += 1

    def jobs(self, run_id: int, attempt: int) -> list[dict]:
        return self.paged(f"repos/{self.repo}/actions/runs/{run_id}/attempts/{attempt}/jobs", "jobs")

    def log(self, job_id: int) -> str | None:
        """Лог джобы; истёкший (404/410) — `None`, прогон остаётся в серии без него."""
        try:
            return self.get(f"repos/{self.repo}/actions/jobs/{job_id}/logs").decode("utf-8", errors="replace")
        except RuntimeError as error:
            if GONE.search(str(error)):
                return None
            raise

    def artifacts(self, run_id: int) -> list[dict]:
        # Список меняется с каждой попыткой (`overwrite: true` даёт новые id):
        # кешируются только сами архивы.
        return self.paged(f"repos/{self.repo}/actions/runs/{run_id}/artifacts", "artifacts", cached=False)

    def artifact_member(self, artifact_id: int, member: str) -> dict | None:
        try:
            archive = zipfile.ZipFile(io.BytesIO(self.get(f"repos/{self.repo}/actions/artifacts/{artifact_id}/zip")))
        except (RuntimeError, zipfile.BadZipFile):
            return None
        names = set(archive.namelist())
        return json.loads(archive.read(member)) if member in names else None


def collect(github: GitHub, run_id: int, attempt: int | None, *, logs: bool, artifacts: bool) -> dict:
    run = github.run(run_id, attempt)
    if run.get("status") != "completed":
        raise ValueError(f"прогон {run_id} ещё не завершён: {run.get('status')}")
    attempt = run.get("run_attempt") or 1
    jobs = github.jobs(run_id, attempt)
    logged = [job["id"] for job in jobs if logs and executed(job) and RUST_JOB.match(job["name"])]
    # Логи — основная часть времени сборщика; читать их параллельно.
    with ThreadPoolExecutor(max_workers=8) as pool:
        parsed_logs = dict(zip(logged, pool.map(lambda job_id: parse_log(text) if (text := github.log(job_id)) is not None else None, logged)))
    lanes: list[dict] = []
    shards: list[dict] = []
    if artifacts:
        for artifact in github.artifacts(run_id):
            if artifact.get("expired"):
                continue
            name = artifact["name"]
            if name.startswith("plan-rust-") and (document := github.artifact_member(artifact["id"], "lane.json")):
                lanes.append(lane_metrics(document))
            elif name.startswith("results-rust-") and (document := github.artifact_member(artifact["id"], "shard.json")):
                shards.append(shard_metrics(document))
        # Артефакты с `overwrite: true` переживают попытки: брать только свою.
        lanes = [lane for lane in lanes if str(lane["attempt"] or attempt) == str(attempt)]
        shards = [shard for shard in shards if str(shard["attempt"] or attempt) == str(attempt)]
    lanes.sort(key=lambda lane: (lane["runner"] or "", lane["lane"] or ""))
    shards.sort(key=lambda shard: (shard["runner"] or "", shard["lane"] or "", shard["shard"] or 0))
    return run_metrics(run, jobs, parsed_logs, lanes, shards)


def parse_run_argument(value: str) -> tuple[str, int, int | None]:
    """`[метка=]run_id[:attempt]` — например `before=39427121659` или `39532820676:2`."""
    label, _, rest = value.rpartition("=")
    run, _, attempt = rest.partition(":")
    if not run.isdigit() or (attempt and not attempt.isdigit()):
        raise argparse.ArgumentTypeError(f"ожидался [метка=]run_id[:attempt], получено {value!r}")
    return label or "all", int(run), int(attempt) if attempt else None


def format_seconds(value: float | None) -> str:
    if value is None:
        return "—"
    minutes, seconds = divmod(int(round(value)), 60)
    return f"{minutes}:{seconds:02d}"


def render(summary: dict) -> str:
    lines = []
    for label, group in summary["groups"].items():
        lines.append(f"{label}: {group['runs']} прогонов, события {', '.join(group['events']) or '—'}, "
                     f"поставочный хвост в {group['deliveryTailRan']}")
        if group["excluded"]:
            lines.append("  вне статистики: " + ", ".join(f"{item['runId']} ({item['reason']})" for item in group["excluded"]))
        for key, values in group["conditions"].items():
            lines.append(f"  {key}: {', '.join(values) or '—'}")
        for name, stats in group["metrics"].items():
            if stats is None:
                continue
            render_value = (lambda v: f"{v:.1f}") if "Minutes" in name else format_seconds
            lines.append(f"  {name}: p50 {render_value(stats['p50'])}, p95 {render_value(stats['p95'])} (n={stats['n']})")
    if "pairedDelta" in summary:
        delta = summary["pairedDelta"]
        lines.append(f"{delta['after']} − {delta['before']}: {delta['pairs']} пар, отброшено {delta['droppedPairs']}")
        for name, stats in delta["metrics"].items():
            if stats is not None:
                lines.append(f"  {name}: p50 {stats['p50']:+.1f}, p95 {stats['p95']:+.1f}")
    return "\n".join(lines)


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n", 1)[0])
    parser.add_argument("runs", nargs="+", type=parse_run_argument, metavar="[LABEL=]RUN_ID[:ATTEMPT]")
    parser.add_argument("--repo", default="IngvarConsulting/unica")
    parser.add_argument("--cache-dir", type=Path, help="локальный кеш ответов gh api (вне репозитория)")
    parser.add_argument("--json", type=Path, help="полные метрики и сводка")
    parser.add_argument("--tsv", type=Path, help="одна строка на прогон")
    parser.add_argument("--pairs", help="BEFORE,AFTER — разности по парам в порядке перечисления")
    parser.add_argument("--no-logs", action="store_true", help="не читать логи джоб (без образа, toolchain и кеша)")
    parser.add_argument("--no-artifacts", action="store_true", help="не читать артефакты полос")
    parser.add_argument("--include-failed", action="store_true", help="считать в сводке и неуспешные прогоны")
    args = parser.parse_args(argv)
    pairs = None
    if args.pairs:
        before, _, after = args.pairs.partition(",")
        if not before or not after:
            parser.error("--pairs ожидает BEFORE,AFTER")
        pairs = (before, after)
    github = GitHub(args.repo, args.cache_dir)
    runs = []
    for label, run_id, attempt in args.runs:
        metrics = collect(github, run_id, attempt, logs=not args.no_logs, artifacts=not args.no_artifacts)
        runs.append({"label": label, **metrics})
    summary = summarize(runs, pairs, include_failed=args.include_failed)
    if args.json:
        args.json.write_text(json.dumps({"runs": runs, "summary": summary}, ensure_ascii=False, indent=2) + "\n",
                             encoding="utf-8")
    if args.tsv:
        with args.tsv.open("w", encoding="utf-8", newline="") as stream:
            writer = csv.DictWriter(stream, fieldnames=TSV_COLUMNS, delimiter="\t")
            writer.writeheader()
            writer.writerows(tsv_row(run) for run in runs)
    print(render(summary))
    return 0


if __name__ == "__main__":
    sys.exit(main())
