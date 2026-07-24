#!/usr/bin/env python3
"""Generate the public cc-1c-skills parity matrix from reviewed contracts."""

from __future__ import annotations

import argparse
import json
import re
import sys
from collections import Counter
from pathlib import Path
from typing import Any


UPSTREAM_ID = "cc-1c-skills"
FIXTURES = Path("tests/fixtures/unica_mcp_script_parity")
BASELINE = FIXTURES / "donor-baseline.json"
RELATIONS = FIXTURES / "donor-relations.json"
PROVENANCE = Path("plugins/unica/provenance/skill-upstreams.json")
MAPPING = Path("plugins/unica/provenance/donor-skill-map.json")
OUTPUT = Path("docs/donor-skill-parity.md")
TOOL_SOURCE = Path("crates/unica-coder/src/application/mod.rs")
TOOL_RELATIONS = {"direct", "supporting", "related"}
PARITY_ORDER = (
    "exact",
    "compatible",
    "unica_extension",
    "platform_override",
    "intentional_divergence",
    "donor_ahead",
)


class MatrixError(ValueError):
    pass


def parse_args(argv: list[str]) -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Generate the reviewed public donor skill parity matrix."
    )
    parser.add_argument("--repo-root", type=Path, required=True)
    mode = parser.add_mutually_exclusive_group(required=True)
    mode.add_argument("--write", action="store_true")
    mode.add_argument("--check", action="store_true")
    return parser.parse_args(argv)


def load_json(path: Path) -> dict[str, Any]:
    try:
        value = json.loads(path.read_text(encoding="utf-8"))
    except FileNotFoundError as error:
        raise MatrixError(f"required matrix input is missing: {path}") from error
    except json.JSONDecodeError as error:
        raise MatrixError(f"invalid matrix JSON: {path}: {error}") from error
    if not isinstance(value, dict):
        raise MatrixError(f"matrix JSON must be an object: {path}")
    return value


def main(argv: list[str] | None = None) -> int:
    args = parse_args(argv or sys.argv[1:])
    repo_root = args.repo_root.resolve()
    try:
        document = generate_document(repo_root)
        output_path = repo_root / OUTPUT
        if args.check:
            if not output_path.is_file() or output_path.read_text(
                encoding="utf-8"
            ) != document:
                raise MatrixError(
                    f"generated donor skill parity matrix is out of date: {OUTPUT}"
                )
        else:
            output_path.parent.mkdir(parents=True, exist_ok=True)
            output_path.write_text(document, encoding="utf-8")
        return 0
    except MatrixError as error:
        print(f"error: {error}", file=sys.stderr)
        return 1


def generate_document(repo_root: Path) -> str:
    baseline = load_json(repo_root / BASELINE)
    relations = load_json(repo_root / RELATIONS)
    provenance = load_json(repo_root / PROVENANCE)
    mapping = load_json(repo_root / MAPPING)
    errors = validate_inputs(repo_root, baseline, relations, provenance, mapping)
    if errors:
        raise MatrixError("\n".join(errors))

    donor_skills = baseline["corpusSkills"]
    corpus_tests = baseline["corpusTests"]
    case_scopes = corpus_tests["caseScopes"]
    suites = corpus_tests["suites"]
    executable_scopes = set(baseline["executableCaseScopes"])
    relation_records = relations["relations"]
    provenance_entries = donor_provenance_entries(provenance)
    rows = []
    for donor_skill in sorted(donor_skills):
        entry = mapping["skills"][donor_skill]
        scopes = entry["caseScopes"]
        rows.append(
            {
                "donor": donor_skill,
                "borrowed": borrowed_state(
                    donor_skill, provenance_entries.get(donor_skill)
                ),
                "tools": tool_state(entry["tools"]),
                "scripts": list_state(donor_skills[donor_skill]["scripts"]),
                "parity": parity_state(
                    donor_skill=donor_skill,
                    entry=entry,
                    baseline=baseline,
                    executable_scopes=executable_scopes,
                    relation_records=relation_records,
                ),
                "corpus": corpus_state(
                    case_scopes=case_scopes,
                    suites=suites,
                    entry=entry,
                ),
            }
        )

    adopted = sum(
        1
        for donor_skill in donor_skills
        if is_ported(provenance_entries.get(donor_skill))
    )
    script_count = sum(
        len(data["scripts"]) for data in donor_skills.values()
    )
    json_case_count = sum(
        len(data["caseIds"]) for data in case_scopes.values()
    )
    corpus_json_file_count = sum(
        1
        for item in baseline.get("files") or []
        if isinstance(item, dict)
        and isinstance(item.get("localPath"), str)
        and item["localPath"].startswith("cases/")
        and item["localPath"].endswith(".json")
        and not item["localPath"].endswith("/_skill.json")
    )
    nested_json_count = max(corpus_json_file_count - json_case_count, 0)
    executable_case_count = sum(
        len(case_scopes[scope]["caseIds"])
        for scope in executable_scopes
    )
    relation_counts = Counter(
        relation.get("relation")
        for relation in relation_records.values()
        if isinstance(relation, dict)
    )

    lines = [
        "# Матрица паритета навыков Николая и Unica",
        "",
        "Этот файл генерируется из принятого donor corpus, provenance, "
        "reviewed parity relations и явного семантического реестра. "
        "Не редактируйте его вручную.",
        "",
        f"- Донор: `{baseline['repository']}` @ "
        f"`{baseline['acceptedCommit']}` (`{baseline['trackingRef']}`).",
        "- Перегенерация: "
        "`python3.12 scripts/ci/generate-donor-skill-matrix.py "
        "--repo-root . --write`.",
        f"- Принято: {len(rows)} donor skills, {script_count} scripts, "
        f"{json_case_count} запускаемых JSON cases"
        + (
            f" и {nested_json_count} JSON snapshots/fixtures"
            if nested_json_count
            else ""
        )
        + f"; {adopted} skills с явным "
        "`ported-to-unica` provenance.",
        f"- Исполняемый паритет: {executable_case_count} cases; "
        + relation_summary(relation_counts)
        + ".",
        "",
        "## Как читать статусы",
        "",
        "`Да` в колонке заимствования означает только явный "
        "`ported-to-unica` в provenance. `related`-инструмент показывает "
        "пересечение возможностей и не является доказательством заимствования. "
        "`not_selected` означает, что тестовый корпус сохранён, но его "
        "сценарии ещё не выбраны для исполняемого паритета; `unmapped` — что "
        "нет утверждённого семантического соответствия Unica.",
        "",
        "## Матрица",
        "",
        "| Какие скиллы есть у Николая | Заимствовали ли мы этот скилл в Unica | "
        "Какие наши тулзы есть для этого скилла | Какие скрипты Николая "
        "используются у него в скиле | Какое состояние парити для этого скилла | "
        "Какое состояние тестового корпуса для этого скилла |",
        "|---|---|---|---|---|---|",
    ]
    for row in rows:
        lines.append(
            "| "
            + " | ".join(
                escape_cell(row[key])
                for key in (
                    "donor",
                    "borrowed",
                    "tools",
                    "scripts",
                    "parity",
                    "corpus",
                )
            )
            + " |"
        )
    lines.extend(["", "## Инварианты", ""])
    lines.extend(
        [
            "- Полный corpus — это сохранённые и хэшированные donor bytes; "
            "он не означает, что Unica заявляет совместимость.",
            "- Relations обязательны только для `executableCaseScopes`; "
            "неподдержанные сохранённые кейсы не маскируются как `donor_ahead`.",
            "- Donor scripts и corpus тестовые: они не входят в marketplace plugin.",
            "",
        ]
    )
    return "\n".join(lines)


def validate_inputs(
    repo_root: Path,
    baseline: dict[str, Any],
    relations: dict[str, Any],
    provenance: dict[str, Any],
    mapping: dict[str, Any],
) -> list[str]:
    errors = []
    if baseline.get("schemaVersion") != 2:
        errors.append("donor baseline schemaVersion must be 2")
        return errors
    if baseline.get("upstreamId") != UPSTREAM_ID:
        errors.append("donor baseline upstreamId must be cc-1c-skills")
    donor_skills = baseline.get("corpusSkills")
    corpus_tests = baseline.get("corpusTests")
    executable_scopes = baseline.get("executableCaseScopes")
    if not isinstance(donor_skills, dict):
        errors.append("donor baseline corpusSkills must be an object")
        donor_skills = {}
    if not isinstance(corpus_tests, dict):
        errors.append("donor baseline corpusTests must be an object")
        corpus_tests = {}
    case_scopes = corpus_tests.get("caseScopes")
    suites = corpus_tests.get("suites")
    if not isinstance(case_scopes, dict):
        errors.append("donor baseline corpusTests.caseScopes must be an object")
        case_scopes = {}
    if not isinstance(suites, dict):
        errors.append("donor baseline corpusTests.suites must be an object")
        suites = {}
    if not isinstance(executable_scopes, list):
        errors.append("donor baseline executableCaseScopes must be an array")

    if relations.get("upstreamId") != UPSTREAM_ID:
        errors.append("donor relations upstreamId must be cc-1c-skills")
    if not isinstance(relations.get("relations"), dict):
        errors.append("donor relations must be an object")

    if mapping.get("schemaVersion") != 1:
        errors.append("donor skill map schemaVersion must be 1")
    if mapping.get("upstreamId") != UPSTREAM_ID:
        errors.append("donor skill map upstreamId must be cc-1c-skills")
    skills = mapping.get("skills")
    if not isinstance(skills, dict):
        errors.append("donor skill map skills must be an object")
        skills = {}
    for donor_skill in sorted(set(donor_skills) - set(skills)):
        errors.append(f"mapping is missing donor skill: {donor_skill}")
    for donor_skill in sorted(set(skills) - set(donor_skills)):
        errors.append(f"mapping has unknown donor skill: {donor_skill}")

    available_skills = {
        path.name
        for path in (repo_root / "plugins" / "unica" / "skills").glob("*")
        if path.is_dir()
    }
    available_tools = discover_unica_tools(repo_root)
    for donor_skill, entry in sorted(skills.items()):
        label = f"mapping {donor_skill}"
        if not isinstance(entry, dict):
            errors.append(f"{label} must be an object")
            continue
        unica_skills = entry.get("unicaSkills")
        if not isinstance(unica_skills, list):
            errors.append(f"{label} unicaSkills must be an array")
        else:
            for skill in unica_skills:
                if skill not in available_skills:
                    errors.append(f"{label} has unknown Unica skill: {skill}")
        tools = entry.get("tools")
        if not isinstance(tools, list):
            errors.append(f"{label} tools must be an array")
        else:
            for tool in tools:
                if not isinstance(tool, dict):
                    errors.append(f"{label} tool must be an object")
                    continue
                name = tool.get("name")
                relation = tool.get("relation")
                if name not in available_tools:
                    errors.append(f"{label} has unknown Unica tool: {name}")
                if relation not in TOOL_RELATIONS:
                    errors.append(
                        f"{label} has invalid tool relation: {relation!r}"
                    )
        for field, known in (("caseScopes", case_scopes), ("testSuites", suites)):
            values = entry.get(field)
            if not isinstance(values, list):
                errors.append(f"{label} {field} must be an array")
                continue
            for value in values:
                if value not in known:
                    errors.append(f"{label} has unknown {field}: {value}")
    return errors


def discover_unica_tools(repo_root: Path) -> set[str]:
    source = repo_root / TOOL_SOURCE
    if not source.is_file():
        return set()
    return set(
        re.findall(
            r'name:\s*"(unica\.[a-z0-9_.-]+)"',
            source.read_text(encoding="utf-8"),
        )
    )


def donor_provenance_entries(provenance: dict[str, Any]) -> dict[str, dict[str, Any]]:
    for upstream in provenance.get("upstreams") or []:
        if isinstance(upstream, dict) and upstream.get("id") == UPSTREAM_ID:
            result = {}
            for entry in upstream.get("entries") or []:
                if not isinstance(entry, dict) or not isinstance(
                    entry.get("skill"), str
                ):
                    continue
                result.setdefault(entry["skill"], entry)
                for path in entry.get("upstreamPaths") or []:
                    if not isinstance(path, str):
                        continue
                    matched = re.fullmatch(
                        r"\.claude/skills/([^/]+)/\*\*", path
                    )
                    if matched:
                        result.setdefault(matched.group(1), entry)
            return result
    return {}


def is_ported(entry: dict[str, Any] | None) -> bool:
    return bool(entry and entry.get("status") == "ported-to-unica")


def borrowed_state(entry_skill: str, entry: dict[str, Any] | None) -> str:
    if not is_ported(entry):
        return "Нет — в provenance нет `ported-to-unica`"
    aliases = []
    for path in entry.get("localPaths") or []:
        prefix = "plugins/unica/skills/"
        if isinstance(path, str) and path.startswith(prefix):
            aliases.append(path.removeprefix(prefix))
    if aliases:
        return "Да — " + ", ".join(f"`{alias}`" for alias in sorted(aliases))
    return f"Да — `{entry_skill}`"


def tool_state(tools: list[dict[str, str]]) -> str:
    if not tools:
        return "—"
    return "<br>".join(
        f"`{tool['name']}` ({tool['relation']})" for tool in tools
    )


def list_state(paths: list[str]) -> str:
    if not paths:
        return "—"
    return "<br>".join(f"`{path}`" for path in paths)


def parity_state(
    *,
    donor_skill: str,
    entry: dict[str, Any],
    baseline: dict[str, Any],
    executable_scopes: set[str],
    relation_records: dict[str, Any],
) -> str:
    selected_scopes = set(entry["caseScopes"]) & executable_scopes
    case_ids = [
        case_id
        for scope in sorted(selected_scopes)
        for case_id in baseline["corpusTests"]["caseScopes"][scope]["caseIds"]
    ]
    if case_ids:
        counts = Counter(
            relation_records.get(case_id, {}).get("relation", "missing")
            for case_id in case_ids
        )
        labels = [
            f"{relation}: {counts[relation]}"
            for relation in PARITY_ORDER
            if counts[relation]
        ]
        if counts["missing"]:
            labels.append(f"missing: {counts['missing']}")
        return ", ".join(labels)

    has_selected_dependency = any(
        item.get("scope")
        and item.get("localPath", "").startswith(f"skills/{donor_skill}/")
        for item in baseline.get("files") or []
    )
    if has_selected_dependency:
        return "dependency_only"
    if not entry["unicaSkills"] and not entry["tools"]:
        return "unmapped"
    return "not_selected"


def corpus_state(
    *,
    case_scopes: dict[str, Any],
    suites: dict[str, Any],
    entry: dict[str, Any],
) -> str:
    case_count = sum(
        len(case_scopes[scope]["caseIds"])
        for scope in entry["caseScopes"]
    )
    result = [f"{case_count}/{case_count} JSON"]
    for suite in entry["testSuites"]:
        result.append(f"{suite}: {len(suites[suite]['files'])} files")
    return "; ".join(result)


def relation_summary(counts: Counter[str]) -> str:
    labels = [
        f"{relation}: {counts[relation]}"
        for relation in PARITY_ORDER
        if counts[relation]
    ]
    return ", ".join(labels) if labels else "relations отсутствуют"


def escape_cell(value: str) -> str:
    return value.replace("|", "\\|").replace("\n", "<br>")


if __name__ == "__main__":
    raise SystemExit(main())
