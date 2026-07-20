#!/usr/bin/env python3
"""Evaluate the stable aggregate result for the routed Unica CI workflow."""

from __future__ import annotations

import json
import os
import sys
from collections.abc import Mapping
from pathlib import Path
from typing import NamedTuple


CLASSIFICATION_OUTPUTS = (
    "rust_changed",
    "platform_changed",
    "toolchain_changed",
    "package_changed",
    "plugin_content_changed",
    "ci_changed",
    "release_required",
)
ALWAYS_JOBS = (
    "classify-changes",
    "verify-source",
)
PACKAGE_JOBS = (
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
    classification: dict[str, str]
    contour: str
    results: dict[str, str]
    expected: dict[str, str]
    unexpected: dict[str, tuple[str, str]]
    skipped_jobs: list[str]

    @property
    def ok(self) -> bool:
        return not self.unexpected


def _validated_classification(
    outputs: Mapping[str, str],
) -> tuple[dict[str, bool], tuple[str, str] | None]:
    invalid = [
        f"{name}={outputs.get(name, 'missing')}"
        for name in CLASSIFICATION_OUTPUTS
        if outputs.get(name) not in {"true", "false"}
    ]
    if invalid:
        return {}, (", ".join(invalid), "all typed outputs are true or false")

    values = {name: outputs[name] == "true" for name in CLASSIFICATION_OUTPUTS}
    contradictions: list[str] = []
    if values["platform_changed"] and not (values["rust_changed"] or values["ci_changed"]):
        contradictions.append("platform_changed requires rust_changed or ci_changed")
    if values["toolchain_changed"] and not (
        values["rust_changed"] and values["package_changed"] and values["release_required"]
    ):
        contradictions.append("toolchain_changed requires rust/package/release contours")
    if values["package_changed"] and not values["release_required"]:
        contradictions.append("package_changed requires release_required")
    if contradictions:
        return values, ("; ".join(contradictions), "a consistent classification")
    return values, None


def expected_results(
    event_name: str,
    ref: str,
    classification: Mapping[str, str],
) -> tuple[str, dict[str, str], dict[str, tuple[str, str]]]:
    expected = {job: "success" for job in ALWAYS_JOBS}
    invalid: dict[str, tuple[str, str]] = {}
    values, classification_error = _validated_classification(classification)

    if classification_error is not None:
        invalid["classification"] = classification_error
        values = {name: False for name in CLASSIFICATION_OUTPUTS}

    is_tag = event_name == "push" and ref.startswith("refs/tags/")
    is_manual = event_name == "workflow_dispatch"
    is_pr = event_name == "pull_request"
    if not (is_tag or is_manual or is_pr):
        invalid["event"] = (f"{event_name}:{ref}", "pull_request, tag push, or workflow_dispatch")

    if (is_tag or is_manual) and not all(values.values()):
        invalid["classification"] = (
            ", ".join(name for name, enabled in values.items() if not enabled) or "invalid",
            "all contours enabled for tag or workflow_dispatch",
        )

    full_matrix = values["platform_changed"] or values["toolchain_changed"] or values["ci_changed"]
    primary_rust = values["rust_changed"] and not full_matrix
    package_pipeline = values["release_required"] or values["ci_changed"]

    expected["test-rust-primary"] = "success" if primary_rust else "skipped"
    expected["test-rust-platforms"] = "success" if full_matrix else "skipped"
    expected.update({job: "success" if package_pipeline else "skipped" for job in PACKAGE_JOBS})
    expected["probe-thin-bootstrap"] = (
        "success" if package_pipeline and (is_pr or is_manual) else "skipped"
    )
    expected.update({job: "success" if is_tag else "skipped" for job in PUBLISH_JOBS})

    if is_tag:
        contour = "release"
    elif is_manual:
        contour = "full"
    elif not is_pr:
        contour = "invalid"
    elif all(values.values()) or values["ci_changed"]:
        contour = "full"
    elif values["platform_changed"]:
        contour = "platform"
    elif values["toolchain_changed"]:
        contour = "toolchain"
    elif values["rust_changed"]:
        contour = "rust"
    elif package_pipeline:
        contour = "package"
    else:
        contour = "source"

    return contour, expected, invalid


def evaluate_gate(
    event_name: str,
    ref: str,
    classification: Mapping[str, str],
    results: Mapping[str, str],
) -> GateEvaluation:
    contour, expected, unexpected = expected_results(event_name, ref, classification)
    unexpected = dict(unexpected)

    for job, expected_result in expected.items():
        actual_result = results.get(job, "missing")
        if actual_result != expected_result:
            unexpected[job] = (actual_result, expected_result)

    skipped_jobs = [job for job in expected if results.get(job) == "skipped"]
    return GateEvaluation(
        event_name=event_name,
        ref=ref,
        classification={name: classification.get(name, "") for name in CLASSIFICATION_OUTPUTS},
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
        f"- Gate: `{'success' if evaluation.ok else 'failure'}`",
        "",
        "### Classification",
        "",
    ]
    for name in CLASSIFICATION_OUTPUTS:
        label = name.replace("_", " ").capitalize()
        lines.append(f"- {label}: `{evaluation.classification.get(name) or 'missing'}`")

    lines.extend(
        [
            "",
            "### Job results",
            "",
            "| Job | Result | Expected |",
            "| --- | --- | --- |",
        ]
    )
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
    outputs = classifier.get("outputs", {}) if isinstance(classifier, dict) else {}
    classification = {
        name: outputs.get(name, "") if isinstance(outputs, dict) else ""
        for name in CLASSIFICATION_OUTPUTS
    }
    results = {
        job: details.get("result", "missing")
        for job, details in needs.items()
        if isinstance(details, dict)
    }
    evaluation = evaluate_gate(
        os.environ.get("GITHUB_EVENT_NAME", ""),
        os.environ.get("GITHUB_REF", ""),
        classification,
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
