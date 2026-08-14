#!/usr/bin/env python3
"""Run reproducible, isolated benchmarks for the RLM BSL index CLI."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import platform
import re
import statistics
import subprocess
import sys
import time
from dataclasses import dataclass
from pathlib import Path, PurePosixPath, PureWindowsPath
from typing import Callable, TypeVar

try:
    import resource
except ImportError:  # pragma: no cover - unavailable on Windows
    resource = None


MARKER = "UNICA_RLM_BENCHMARK_MARKER"
SECTION_HEADING = "## Замер RLM v1.33.0"
RELEASE_TAG = "rlm-tools-bsl-v1.33.0-build.2"
SOURCE_COMMIT = "3e6920cd015a61af4ba7aa1a5f1fedd8bc935549"
TAIL_LIMIT = 4_000
HEX_40 = re.compile(r"[0-9a-f]{40}\Z")
HEX_64 = re.compile(r"[0-9a-f]{64}\Z")


@dataclass(frozen=True)
class Scenario:
    name: str
    repeats: int
    paths_key: str | None


SCENARIOS = (
    Scenario("cold-build", 1, None),
    Scenario("noop-update", 5, None),
    Scenario("bsl-1", 6, "bsl-1"),
    Scenario("bsl-10", 6, "bsl-10"),
    Scenario("bsl-100", 6, "bsl-100"),
    Scenario("xml-form-1", 6, "xml-form-1"),
    Scenario("xml-root-10", 6, "xml-root-10"),
)


@dataclass(frozen=True)
class Sample:
    duration_seconds: float
    peak_rss_bytes: int | None
    git_fast_path: bool | None
    final_status: str
    modules: int | None = None
    methods: int | None = None
    db_size_bytes: int | None = None
    index_size_bytes: int | None = None
    stdout_tail: str = ""
    stderr_tail: str = ""
    info_stdout_tail: str = ""
    info_stderr_tail: str = ""


@dataclass(frozen=True)
class CommandEvidence:
    scenario: str
    iteration: int | None
    phase: str
    action: str
    duration_seconds: float
    stdout_tail: str
    stderr_tail: str
    status: str
    git_fast_path: bool | None


@dataclass(frozen=True)
class _CommandResult:
    duration_seconds: float
    peak_rss_bytes: int | None
    stdout_tail: str
    stderr_tail: str


T = TypeVar("T")


SCENARIO_LABELS = {
    "cold-build": "Холодный полный build",
    "noop-update": "No-op update",
    "bsl-1": "Изменён 1 BSL-файл",
    "bsl-10": "Изменены 10 BSL-файлов",
    "bsl-100": "Изменены 100 BSL-файлов",
    "xml-form-1": "Изменён 1 XML формы",
    "xml-root-10": "Изменены 10 корневых XML документов",
}


BASELINE_V1_29_1_SECONDS = {
    "cold-build": 141.28,
    "noop-update": 3.82,
    "bsl-1": 5.23,
    "bsl-10": 7.71,
    "bsl-100": 15.53,
    "xml-form-1": 10.95,
    "xml-root-10": 10.46,
}


def _run_git(repo: Path, *args: str) -> subprocess.CompletedProcess[str]:
    completed = subprocess.run(
        ["git", *args],
        cwd=repo,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
    )
    if completed.returncode != 0:
        detail = (completed.stderr or completed.stdout).strip()
        raise RuntimeError(f"Git command failed: git {' '.join(args)}: {detail}")
    return completed


def ensure_clean(repo: Path) -> None:
    """Refuse a tracked worktree with staged or unstaged changes."""
    status = _run_git(repo, "status", "--porcelain", "--untracked-files=no").stdout
    if status:
        raise RuntimeError("tracked Git tree must be clean")


def _tracked_paths(repo: Path) -> list[Path]:
    output = _run_git(repo, "ls-files", "-z", "--").stdout
    return sorted(Path(item) for item in output.split("\0") if item)


def is_bsl(path: Path) -> bool:
    return path.suffix.lower() == ".bsl"


def is_form_xml(path: Path) -> bool:
    parts = path.parts
    return path.name == "Form.xml" and "Forms" in parts and "Ext" in parts


def is_root_xml(path: Path) -> bool:
    return (
        path.suffix.lower() == ".xml"
        and "Ext" not in path.parts
        and path.name != "Configuration.xml"
    )


def select_inputs(repo: Path) -> dict[str, list[Path]]:
    """Select deterministic, repository-relative inputs from Git's index."""
    tracked = _tracked_paths(repo)
    bsl = [path for path in tracked if is_bsl(path)]
    form_xml = [path for path in tracked if is_form_xml(path)]
    root_xml = [path for path in tracked if is_root_xml(path)]
    return {
        "bsl-1": bsl[:1],
        "bsl-10": bsl[:10],
        "bsl-100": bsl[:100],
        "xml-form-1": form_xml[:1],
        "xml-root-10": root_xml[:10],
    }


def _require_selection_sizes(selected: dict[str, list[Path]]) -> None:
    required_sizes = {
        "bsl-1": 1,
        "bsl-10": 10,
        "bsl-100": 100,
        "xml-form-1": 1,
        "xml-root-10": 10,
    }
    for name, required in required_sizes.items():
        actual = len(selected[name])
        if actual != required:
            raise RuntimeError(
                f"benchmark scenario {name} requires {required} tracked files; found {actual}"
            )


def _resolved_directory(path: Path, label: str) -> Path:
    try:
        resolved = path.expanduser().resolve(strict=True)
    except OSError as error:
        raise RuntimeError(f"{label} does not exist: {path}") from error
    if not resolved.is_dir():
        raise RuntimeError(f"{label} must be a directory: {path}")
    return resolved


def validate_index_dir(repo: Path, index_dir: Path) -> tuple[Path, Path]:
    """Require an existing, empty index directory disjoint from the repository."""
    resolved_repo = _resolved_directory(repo, "repository")
    resolved_index = _resolved_directory(index_dir, "index directory")
    if (
        resolved_repo == resolved_index
        or resolved_repo in resolved_index.parents
        or resolved_index in resolved_repo.parents
    ):
        raise RuntimeError("index directory and repository must not overlap")
    if next(resolved_index.iterdir(), None) is not None:
        raise RuntimeError("index directory must be empty before cold-build")
    return resolved_repo, resolved_index


def _validate_mutation_path(repo: Path, path: Path) -> Path:
    if path.is_absolute() or ".." in path.parts:
        raise RuntimeError(f"selected benchmark path must be repository-relative: {path}")
    candidate = repo / path
    if candidate.is_symlink():
        raise RuntimeError(f"selected benchmark path must not be a symlink: {path}")
    try:
        resolved_repo = repo.resolve(strict=True)
        resolved = candidate.resolve(strict=True)
        resolved.relative_to(resolved_repo)
    except (OSError, ValueError) as error:
        raise RuntimeError(f"selected benchmark path escapes the repository: {path}") from error
    if not resolved.is_file():
        raise RuntimeError(f"selected benchmark path must be a regular file: {path}")
    return resolved


def mutate(repo: Path, paths: list[Path], marker: str) -> None:
    marker_bytes = marker.encode("utf-8")
    for path in paths:
        candidate = _validate_mutation_path(repo, path)
        if path.suffix.lower() == ".xml":
            comment = b"\n<!-- " + marker_bytes + b" -->\n"
        else:
            comment = b"\n// " + marker_bytes + b"\n"
        with candidate.open("ab") as stream:
            stream.write(comment)


def run_incremental_scenario(
    *,
    repo: Path,
    paths: list[Path],
    marker: str,
    measured_update: Callable[[], T],
    reverse_update: Callable[[], object],
) -> T:
    """Measure one mutation and always restore both Git and index state."""
    try:
        mutate(repo, paths, marker)
        return measured_update()
    finally:
        _run_git(repo, "restore", "--source=HEAD", "--", *map(str, paths))
        reverse_update()
        ensure_clean(repo)


def _sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def _tail(text: str) -> str:
    return text[-TAIL_LIMIT:]


def _rss_bytes() -> int | None:
    """Return max RSS across all reaped child processes, not one RLM command."""
    if resource is None:
        return None
    maximum = resource.getrusage(resource.RUSAGE_CHILDREN).ru_maxrss
    if maximum <= 0:
        return None
    return int(maximum if sys.platform == "darwin" else maximum * 1024)


def _run_rlm(
    executable: Path,
    action: str,
    repo: Path,
    index_dir: Path,
    *,
    capture_peak_rss: bool = False,
) -> _CommandResult:
    executable_command = (
        [sys.executable, str(executable)]
        if executable.suffix.lower() == ".py"
        else [str(executable)]
    )
    command = [*executable_command, "index", action, str(repo)]
    env = {**os.environ, "RLM_INDEX_DIR": str(index_dir)}
    started = time.perf_counter()
    completed = subprocess.run(
        command,
        cwd=repo,
        env=env,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
    )
    duration = time.perf_counter() - started
    if completed.returncode != 0:
        detail = _tail(completed.stderr or completed.stdout).strip()
        raise RuntimeError(f"RLM index {action} failed with exit {completed.returncode}: {detail}")
    return _CommandResult(
        duration_seconds=duration,
        peak_rss_bytes=_rss_bytes() if capture_peak_rss else None,
        stdout_tail=_tail(completed.stdout),
        stderr_tail=_tail(completed.stderr),
    )


def _parse_int(label: str, text: str) -> int | None:
    match = re.search(
        rf"(?im)^[ \t]*{re.escape(label)}:[ \t]*([0-9][0-9 \t]*)[ \t]*$",
        text,
    )
    return int(re.sub(r"[ \t]", "", match.group(1))) if match else None


def _parse_db_size(text: str) -> int | None:
    match = re.search(
        r"(?im)^\s*DB size:\s*([0-9]+(?:[.,][0-9]+)?)\s*(B|KB|MB|GB)\s*$",
        text,
    )
    if not match:
        return None
    value = float(match.group(1).replace(",", "."))
    multiplier = {"B": 1, "KB": 1024, "MB": 1024**2, "GB": 1024**3}[match.group(2)]
    return int(round(value * multiplier))


def _parse_fast_path(text: str) -> bool | None:
    match = re.search(r"(?im)^\s*Fast path:\s*(True|False)\s*$", text)
    return match.group(1) == "True" if match else None


def _parse_status(text: str) -> str:
    match = re.search(r"(?im)^\s*Status:\s*(.+?)\s*$", text)
    return match.group(1).strip().lower() if match else "unknown"


def _recursive_size(path: Path) -> int:
    return sum(entry.stat().st_size for entry in path.rglob("*") if entry.is_file())


def _record_command(
    command_evidence: list[CommandEvidence],
    *,
    scenario: str,
    iteration: int | None,
    phase: str,
    action: str,
    result: _CommandResult,
) -> None:
    command_evidence.append(
        CommandEvidence(
            scenario=scenario,
            iteration=iteration,
            phase=phase,
            action=action,
            duration_seconds=result.duration_seconds,
            stdout_tail=result.stdout_tail,
            stderr_tail=result.stderr_tail,
            status=_parse_status(result.stdout_tail),
            git_fast_path=_parse_fast_path(result.stdout_tail),
        )
    )


def _measure_action(
    executable: Path,
    action: str,
    repo: Path,
    index_dir: Path,
    *,
    scenario: str,
    iteration: int,
    phase: str,
    command_evidence: list[CommandEvidence],
    capture_peak_rss: bool = False,
) -> Sample:
    measured = _run_rlm(
        executable,
        action,
        repo,
        index_dir,
        capture_peak_rss=capture_peak_rss,
    )
    _record_command(
        command_evidence,
        scenario=scenario,
        iteration=iteration,
        phase=phase,
        action=action,
        result=measured,
    )
    info = _run_rlm(executable, "info", repo, index_dir)
    _record_command(
        command_evidence,
        scenario=scenario,
        iteration=iteration,
        phase=f"{phase}-info",
        action="info",
        result=info,
    )
    combined = f"{measured.stdout_tail}\n{info.stdout_tail}"
    return Sample(
        duration_seconds=measured.duration_seconds,
        peak_rss_bytes=measured.peak_rss_bytes,
        git_fast_path=_parse_fast_path(measured.stdout_tail),
        final_status=_parse_status(info.stdout_tail),
        modules=_parse_int("Modules", combined),
        methods=_parse_int("Methods", combined),
        db_size_bytes=_parse_db_size(combined),
        index_size_bytes=_recursive_size(index_dir),
        stdout_tail=measured.stdout_tail,
        stderr_tail=measured.stderr_tail,
        info_stdout_tail=info.stdout_tail,
        info_stderr_tail=info.stderr_tail,
    )


def _require_fresh(sample: Sample, action: str) -> Sample:
    if sample.final_status != "fresh":
        raise RuntimeError(
            f"RLM index {action} did not finish fresh; status={sample.final_status}"
        )
    return sample


def _marker_absent(repo: Path, marker: str) -> bool:
    completed = subprocess.run(
        ["git", "grep", "-l", "-F", marker, "--", "."],
        cwd=repo,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
    )
    if completed.returncode not in (0, 1):
        raise RuntimeError(f"Git marker scan failed: {completed.stderr.strip()}")
    return completed.returncode == 1


def _sample_dict(sample: Sample) -> dict[str, object]:
    return {
        "durationSeconds": sample.duration_seconds,
        "peakRssBytes": sample.peak_rss_bytes,
        "gitFastPath": sample.git_fast_path,
        "finalStatus": sample.final_status,
        "modules": sample.modules,
        "methods": sample.methods,
        "dbSizeBytes": sample.db_size_bytes,
        "indexSizeBytes": sample.index_size_bytes,
        "stdoutTail": sample.stdout_tail,
        "stderrTail": sample.stderr_tail,
        "infoStdoutTail": sample.info_stdout_tail,
        "infoStderrTail": sample.info_stderr_tail,
    }


def _command_evidence_dict(
    evidence: CommandEvidence, sequence: int
) -> dict[str, object]:
    return {
        "sequence": sequence,
        "scenario": evidence.scenario,
        "iteration": evidence.iteration,
        "phase": evidence.phase,
        "action": evidence.action,
        "durationSeconds": evidence.duration_seconds,
        "stdoutTail": evidence.stdout_tail,
        "stderrTail": evidence.stderr_tail,
        "status": evidence.status,
        "gitFastPath": evidence.git_fast_path,
    }


def result_document(
    *,
    label: str,
    source_commit: str,
    executable_sha256: str,
    repo_head: str,
    selected: dict[str, list[Path]],
    samples: dict[str, list[Sample]],
    command_evidence: list[CommandEvidence],
    final_clean: bool,
) -> dict[str, object]:
    """Build the stable raw-result schema without discarding individual samples."""
    return {
        "schemaVersion": 1,
        "label": label,
        "sourceCommit": source_commit,
        "executableSha256": executable_sha256,
        "repoHead": repo_head,
        "python": {
            "implementation": platform.python_implementation(),
            "version": platform.python_version(),
        },
        "platform": {
            "system": platform.system(),
            "release": platform.release(),
            "machine": platform.machine(),
        },
        "selected": {
            name: [path.as_posix() for path in paths] for name, paths in selected.items()
        },
        "samples": {
            name: [_sample_dict(sample) for sample in raw_samples]
            for name, raw_samples in samples.items()
        },
        "commands": [
            _command_evidence_dict(evidence, sequence)
            for sequence, evidence in enumerate(command_evidence, start=1)
        ],
        "finalClean": final_clean,
    }


def run_benchmark(
    *,
    repo: Path,
    executable: Path,
    label: str,
    source_commit: str,
    index_dir: Path,
) -> dict[str, object]:
    """Run all benchmark scenarios against one fresh, isolated index directory."""
    if label not in {"packaged-v1.33.0", "source-v1.33.0"}:
        raise RuntimeError(f"unsupported benchmark label: {label}")
    if not HEX_40.fullmatch(source_commit):
        raise RuntimeError("source commit must be exactly 40 lowercase hex characters")
    resolved_repo, resolved_index = validate_index_dir(repo, index_dir)
    resolved_executable = executable.expanduser().resolve(strict=True)
    if not resolved_executable.is_file():
        raise RuntimeError(f"executable must be a file: {executable}")
    ensure_clean(resolved_repo)
    selected = select_inputs(resolved_repo)
    _require_selection_sizes(selected)
    repo_head = _run_git(resolved_repo, "rev-parse", "HEAD").stdout.strip()
    if not HEX_40.fullmatch(repo_head):
        raise RuntimeError(f"repository HEAD is not a 40-character Git object ID: {repo_head}")

    samples: dict[str, list[Sample]] = {scenario.name: [] for scenario in SCENARIOS}
    command_evidence: list[CommandEvidence] = []
    cold = _require_fresh(
        _measure_action(
            resolved_executable,
            "build",
            resolved_repo,
            resolved_index,
            scenario="cold-build",
            iteration=1,
            phase="measured",
            command_evidence=command_evidence,
            capture_peak_rss=True,
        ),
        "build",
    )
    samples["cold-build"].append(cold)

    for iteration in range(1, 6):
        samples["noop-update"].append(
            _require_fresh(
                _measure_action(
                    resolved_executable,
                    "update",
                    resolved_repo,
                    resolved_index,
                    scenario="noop-update",
                    iteration=iteration,
                    phase="measured",
                    command_evidence=command_evidence,
                ),
                "noop update",
            )
        )

    for scenario in SCENARIOS[2:]:
        paths = selected[scenario.paths_key]
        for iteration in range(1, scenario.repeats + 1):
            def reverse_update() -> None:
                _require_fresh(
                    _measure_action(
                        resolved_executable,
                        "update",
                        resolved_repo,
                        resolved_index,
                        scenario=scenario.name,
                        iteration=iteration,
                        phase="reverse",
                        command_evidence=command_evidence,
                    ),
                    "reverse update",
                )

            sample = run_incremental_scenario(
                repo=resolved_repo,
                paths=paths,
                marker=MARKER,
                measured_update=lambda: _require_fresh(
                    _measure_action(
                        resolved_executable,
                        "update",
                        resolved_repo,
                        resolved_index,
                        scenario=scenario.name,
                        iteration=iteration,
                        phase="measured",
                        command_evidence=command_evidence,
                    ),
                    scenario.name,
                ),
                reverse_update=reverse_update,
            )
            samples[scenario.name].append(sample)

    final_info = _run_rlm(resolved_executable, "info", resolved_repo, resolved_index)
    _record_command(
        command_evidence,
        scenario="final",
        iteration=None,
        phase="final-info",
        action="info",
        result=final_info,
    )
    final_status = _parse_status(final_info.stdout_tail)
    marker_absent = _marker_absent(resolved_repo, MARKER)
    try:
        ensure_clean(resolved_repo)
        final_clean = True
    except RuntimeError:
        final_clean = False
    if final_status != "fresh" or not marker_absent or not final_clean:
        raise RuntimeError(
            "benchmark final proof failed: "
            f"status={final_status}, markerAbsent={marker_absent}, finalClean={final_clean}"
        )

    return result_document(
        label=label,
        source_commit=source_commit,
        executable_sha256=_sha256(resolved_executable),
        repo_head=repo_head,
        selected=selected,
        samples=samples,
        command_evidence=command_evidence,
        final_clean=final_clean,
    )


def _is_absolute_path(value: str) -> bool:
    return PurePosixPath(value).is_absolute() or PureWindowsPath(value).is_absolute()


def _validate_summary_document(document: dict[str, object]) -> None:
    if document.get("schemaVersion") != 1:
        raise RuntimeError("summary requires benchmark schema version 1")
    if document.get("label") not in {"packaged-v1.33.0", "source-v1.33.0"}:
        raise RuntimeError("summary contains an unsupported benchmark label")
    if not HEX_40.fullmatch(str(document.get("sourceCommit", ""))):
        raise RuntimeError("summary contains an invalid source commit")
    if not HEX_64.fullmatch(str(document.get("executableSha256", ""))):
        raise RuntimeError("summary contains an invalid executable SHA-256")
    if not HEX_40.fullmatch(str(document.get("repoHead", ""))):
        raise RuntimeError("summary contains an invalid repository HEAD")
    if document.get("finalClean") is not True:
        raise RuntimeError("summary requires a clean final benchmark tree")
    selected = document.get("selected")
    if not isinstance(selected, dict):
        raise RuntimeError("summary requires selected input provenance")
    for paths in selected.values():
        if not isinstance(paths, list):
            raise RuntimeError("summary contains invalid selected input provenance")
        if any(not isinstance(path, str) or _is_absolute_path(path) for path in paths):
            raise RuntimeError("summary contains an absolute path")


def _validate_summary_pair(
    documents: list[dict[str, object]],
) -> list[dict[str, object]]:
    expected_labels = {"packaged-v1.33.0", "source-v1.33.0"}
    labels = [document.get("label") for document in documents]
    if len(documents) != 2 or set(labels) != expected_labels:
        raise RuntimeError(
            "summary requires exactly one packaged-v1.33.0 and one "
            "source-v1.33.0 result"
        )
    by_label = {str(document["label"]): document for document in documents}
    ordered = [by_label["packaged-v1.33.0"], by_label["source-v1.33.0"]]
    for document in ordered:
        _validate_summary_document(document)
        if document["sourceCommit"] != SOURCE_COMMIT:
            raise RuntimeError(
                f"summary requires exact source commit {SOURCE_COMMIT} for both results"
            )
    if ordered[0]["repoHead"] != ordered[1]["repoHead"]:
        raise RuntimeError("summary requires identical repoHead values")
    if ordered[0]["selected"] != ordered[1]["selected"]:
        raise RuntimeError("summary requires identical selected mapping")
    return ordered


def _duration_stats(
    document: dict[str, object], scenario: Scenario
) -> tuple[int, float, float, float]:
    samples = document["samples"]
    if not isinstance(samples, dict) or scenario.name not in samples:
        raise RuntimeError(f"summary is missing samples for {scenario.name}")
    raw_samples = samples[scenario.name]
    if not isinstance(raw_samples, list) or len(raw_samples) != scenario.repeats:
        raise RuntimeError(
            f"summary requires {scenario.repeats} raw samples for {scenario.name}"
        )
    durations = []
    for sample in raw_samples:
        if not isinstance(sample, dict):
            raise RuntimeError(f"summary contains an invalid sample for {scenario.name}")
        duration = sample.get("durationSeconds")
        if isinstance(duration, bool) or not isinstance(duration, (int, float)) or duration < 0:
            raise RuntimeError(f"summary contains an invalid duration for {scenario.name}")
        if scenario.name != "cold-build" and sample.get("gitFastPath") is not True:
            raise RuntimeError(
                f"summary requires gitFastPath=True for every {scenario.name} sample"
            )
        durations.append(float(duration))
    return len(durations), statistics.median(durations), min(durations), max(durations)


def _format_decimal(value: float) -> str:
    return f"{value:.2f}".replace(".", ",")


def _format_seconds(value: float) -> str:
    return _format_decimal(value) + " с"


def _format_bytes(value: int | None) -> str:
    if value is None:
        return "—"
    if value >= 1024**3:
        return f"{value / 1024**3:.2f}".replace(".", ",") + " ГиБ"
    if value >= 1024**2:
        return f"{value / 1024**2:.1f}".replace(".", ",") + " МиБ"
    if value >= 1024:
        return f"{value / 1024:.1f}".replace(".", ",") + " КиБ"
    return f"{value} Б"


def _format_integer(value: int | None) -> str:
    return f"{value:,}".replace(",", " ") if value is not None else "—"


def markdown_summary(documents: list[dict[str, object]]) -> str:
    """Produce a sanitized aggregate section; selected object paths are never emitted."""
    documents = _validate_summary_pair(documents)

    lines = [
        SECTION_HEADING,
        "",
        f"Toolchain release: `{RELEASE_TAG}`.",
        "",
        "Проверенная provenance:",
        "",
    ]
    for document in documents:
        lines.append(
            f"- `{document['label']}`: source commit `{document['sourceCommit']}`, "
            f"executable SHA-256 `{document['executableSha256']}`."
        )
    lines.extend(
        [
            "",
            "| Вариант | Сценарий | n | Медиана real | "
            "Наблюдённый диапазон |",
            "| --- | --- | ---: | ---: | ---: |",
        ]
    )
    stats_by_label: dict[str, dict[str, tuple[int, float, float, float]]] = {}
    for document in documents:
        label = str(document["label"])
        stats_by_label[label] = {}
        for scenario in SCENARIOS:
            count, median, minimum, maximum = _duration_stats(document, scenario)
            stats_by_label[label][scenario.name] = (count, median, minimum, maximum)
            lines.append(
                f"| {label} | {SCENARIO_LABELS[scenario.name]} | {count} | "
                f"{_format_seconds(median)} | "
                f"{_format_decimal(minimum)}–{_format_seconds(maximum)} |"
            )

    source_stats = stats_by_label.get("source-v1.33.0")
    if source_stats:
        lines.extend(
            [
                "",
                "Сравнение медиан source CLI с опубликованным замером "
                "RLM v1.29.1:",
                "",
                "| Сценарий | v1.29.1 | v1.33.0 | Изменение |",
                "| --- | ---: | ---: | ---: |",
            ]
        )
        for scenario in SCENARIOS:
            baseline = BASELINE_V1_29_1_SECONDS[scenario.name]
            current = source_stats[scenario.name][1]
            percent = (current - baseline) / baseline * 100
            lines.append(
                f"| {SCENARIO_LABELS[scenario.name]} | {_format_seconds(baseline)} | "
                f"{_format_seconds(current)} | {percent:+.1f}% |".replace(".", ",")
            )

    lines.extend(
        [
            "",
            "Параметры холодного build:",
            "",
            "| Вариант | Модули | Методы | DB size | Размер index dir | "
            "Макс. RSS среди всех завершённых дочерних процессов |",
            "| --- | ---: | ---: | ---: | ---: | ---: |",
        ]
    )
    for document in documents:
        cold_samples = document["samples"]["cold-build"]
        cold = cold_samples[0]
        lines.append(
            f"| {document['label']} | {_format_integer(cold.get('modules'))} | "
            f"{_format_integer(cold.get('methods'))} | {_format_bytes(cold.get('dbSizeBytes'))} | "
            f"{_format_bytes(cold.get('indexSizeBytes'))} | "
            f"{_format_bytes(cold.get('peakRssBytes'))} |"
        )

    incremental_max = max(
        maximum
        for per_label in stats_by_label.values()
        for name, (_, _, _, maximum) in per_label.items()
        if name not in {"cold-build", "noop-update"}
    )
    lines.extend(["", "Операционные выводы:", ""])
    if incremental_max <= 45:
        lines.append(
            "- Наблюдаемые данные не требуют менять `quiet period = 5 с`, "
            "`max batch delay = 30 с` или provider deadline `45 с`."
        )
    else:
        lines.append(
            "- `quiet period = 5 с` и `max batch delay = 30 с` остаются без "
            "изменений; наблюдаемый инкрементальный максимум превышает provider "
            "deadline `45 с`, "
            "поэтому deadline требует отдельного пересмотра."
        )
    lines.append(
        "- Это сырые повторы одного стенда; абсолютные секунды не являются "
        "универсальным SLA."
    )
    summary = "\n".join(lines).rstrip() + "\n"
    absolute_token = re.search(r"(?:^|[\s`(])(?:/[^\s`|)]+|[A-Za-z]:[\\/][^\s`|)]+)", summary)
    if absolute_token:
        raise RuntimeError("summary contains an absolute path")
    return summary


def replace_summary_section(body: str, summary: str) -> str:
    """Replace or append the v1.33 section without duplicating it."""
    normalized = summary.rstrip() + "\n"
    pattern = re.compile(
        rf"(?ms)^{re.escape(SECTION_HEADING)}\n.*?(?=^## |\Z)"
    )
    if pattern.search(body):
        updated = pattern.sub(normalized + "\n", body, count=1)
        return updated.rstrip() + "\n"
    prefix = body.rstrip()
    return (prefix + "\n\n" if prefix else "") + normalized


def _write_json(path: Path, document: dict[str, object]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(
        json.dumps(document, ensure_ascii=False, indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
    )


def _parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--repo", type=Path)
    parser.add_argument("--executable", type=Path)
    parser.add_argument(
        "--label", choices=("packaged-v1.33.0", "source-v1.33.0")
    )
    parser.add_argument("--source-commit")
    parser.add_argument("--index-dir", type=Path)
    parser.add_argument("--output", type=Path)
    parser.add_argument("--summarize", nargs="+", type=Path, metavar="RESULT")
    parser.add_argument("--append-to", type=Path, metavar="BODY")
    return parser


def main(argv: list[str] | None = None) -> int:
    parser = _parser()
    args = parser.parse_args(argv)
    if args.summarize is None and args.append_to is not None:
        parser.error("--append-to can only be used with --summarize")
    try:
        if args.summarize is not None:
            if args.append_to is None:
                parser.error("--summarize requires --append-to")
            benchmark_options = (
                args.repo,
                args.executable,
                args.label,
                args.source_commit,
                args.index_dir,
                args.output,
            )
            if any(option is not None for option in benchmark_options):
                parser.error("benchmark options cannot be combined with --summarize")
            documents = [
                json.loads(path.read_text(encoding="utf-8")) for path in args.summarize
            ]
            summary = markdown_summary(documents)
            body = args.append_to.read_text(encoding="utf-8")
            args.append_to.write_text(
                replace_summary_section(body, summary), encoding="utf-8"
            )
            return 0

        required = {
            "--repo": args.repo,
            "--executable": args.executable,
            "--label": args.label,
            "--source-commit": args.source_commit,
            "--index-dir": args.index_dir,
            "--output": args.output,
        }
        missing = [name for name, value in required.items() if value is None]
        if missing:
            parser.error("benchmark mode requires " + ", ".join(missing))
        document = run_benchmark(
            repo=args.repo,
            executable=args.executable,
            label=args.label,
            source_commit=args.source_commit,
            index_dir=args.index_dir,
        )
        _write_json(args.output, document)
        return 0
    except (OSError, RuntimeError, ValueError, json.JSONDecodeError) as error:
        print(f"error: {error}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
