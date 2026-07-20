#!/usr/bin/env python3
"""Evaluate the stable aggregate result for the Unica GitHub Actions workflow."""

from __future__ import annotations

import json
import os
import sys
from pathlib import Path
from typing import NamedTuple


COMMON_JOBS = (
    "classify-changes",
    "verify-source",
    "test-rust-platforms",
)
CONDITIONAL_JOBS = (
    "build-tools",
    "package-runtime",
    "package-thin",
    "release-assessment",
)
PUBLISH_JOBS = (
    "publish-release-assets",
    "smoke-thin-plugin",
    "verify-published-assets",
)


class GateEvaluation(NamedTuple):
    event_name: str
    ref: str
    release_artifacts: str
    contour: str
    results: dict[str, str]
    expected: dict[str, str]
    unexpected: dict[str, tuple[str, str]]
    skipped_jobs: list[str]

    @property
    def ok(self) -> bool:
        return not self.unexpected


def expected_results(
    event_name: str,
    ref: str,
    release_artifacts: str,
) -> tuple[str, dict[str, str], dict[str, tuple[str, str]]]:
    expected = {job: "success" for job in COMMON_JOBS}
    invalid: dict[str, tuple[str, str]] = {}

    if release_artifacts not in {"true", "false"}:
        invalid["classification"] = (release_artifacts or "missing", "true or false")
        return "invalid", expected, invalid

    pipeline_result = "success" if release_artifacts == "true" else "skipped"
    expected.update({job: pipeline_result for job in CONDITIONAL_JOBS})

    if event_name == "pull_request":
        contour = "full" if release_artifacts == "true" else "light"
        expected["probe-thin-bootstrap"] = pipeline_result
        expected.update({job: "skipped" for job in PUBLISH_JOBS})
    elif event_name == "push" and ref.startswith("refs/tags/"):
        contour = "release"
        expected["probe-thin-bootstrap"] = "skipped"
        expected.update({job: "success" for job in PUBLISH_JOBS})
        if release_artifacts != "true":
            invalid["classification"] = (release_artifacts, "true")
    elif event_name == "workflow_dispatch":
        contour = "full"
        expected["probe-thin-bootstrap"] = "skipped"
        expected.update({job: "skipped" for job in PUBLISH_JOBS})
        if release_artifacts != "true":
            invalid["classification"] = (release_artifacts, "true")
    else:
        contour = "invalid"
        expected["probe-thin-bootstrap"] = "skipped"
        expected.update({job: "skipped" for job in PUBLISH_JOBS})
        invalid["event"] = (f"{event_name}:{ref}", "pull_request, tag push, or workflow_dispatch")

    return contour, expected, invalid


def evaluate_gate(
    event_name: str,
    ref: str,
    release_artifacts: str,
    results: dict[str, str],
) -> GateEvaluation:
    contour, expected, unexpected = expected_results(event_name, ref, release_artifacts)
    unexpected = dict(unexpected)

    for job, expected_result in expected.items():
        actual_result = results.get(job, "missing")
        if actual_result != expected_result:
            unexpected[job] = (actual_result, expected_result)

    skipped_jobs = [job for job in expected if results.get(job) == "skipped"]
    return GateEvaluation(
        event_name=event_name,
        ref=ref,
        release_artifacts=release_artifacts,
        contour=contour,
        results=dict(results),
        expected=expected,
        unexpected=unexpected,
        skipped_jobs=skipped_jobs,
    )


def render_summary(evaluation: GateEvaluation) -> str:
    lines = [
        "## Unica CI",
        "",
        f"- Event: `{evaluation.event_name}`",
        f"- Contour: `{evaluation.contour}`",
        f"- Release artifacts: `{evaluation.release_artifacts or 'missing'}`",
        f"- Gate: `{'success' if evaluation.ok else 'failure'}`",
        "",
        "| Job | Result | Expected |",
        "| --- | --- | --- |",
    ]
    for job, expected_result in evaluation.expected.items():
        actual_result = evaluation.results.get(job, "missing")
        lines.append(f"| `{job}` | `{actual_result}` | `{expected_result}` |")

    lines.extend(["", "### Skipped jobs", ""])
    if evaluation.skipped_jobs:
        lines.extend(f"- `{job}`" for job in evaluation.skipped_jobs)
    else:
        lines.append("- None")

    if evaluation.unexpected:
        lines.extend(["", "### Unexpected results", ""])
        for item, (actual, expected) in evaluation.unexpected.items():
            lines.append(f"- `{item}`: got `{actual}`, expected `{expected}`")

    return "\n".join(lines) + "\n"


def main() -> int:
    needs = json.loads(os.environ["NEEDS_JSON"])
    classifier = needs.get("classify-changes", {})
    release_artifacts = classifier.get("outputs", {}).get("release_artifacts", "")
    results = {
        job: details.get("result", "missing")
        for job, details in needs.items()
        if isinstance(details, dict)
    }
    evaluation = evaluate_gate(
        os.environ.get("GITHUB_EVENT_NAME", ""),
        os.environ.get("GITHUB_REF", ""),
        release_artifacts,
        results,
    )
    summary = render_summary(evaluation)
    print(summary, end="")
    summary_path = os.environ.get("GITHUB_STEP_SUMMARY")
    if summary_path:
        with Path(summary_path).open("a", encoding="utf-8") as stream:
            stream.write(summary)
    return 0 if evaluation.ok else 1


if __name__ == "__main__":
    sys.exit(main())
