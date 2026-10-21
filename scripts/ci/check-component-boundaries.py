#!/usr/bin/env python3
"""Check the production component graph, including target and test edges."""
from __future__ import annotations

import argparse
from pathlib import Path
import tomllib

FAMILIES = frozenset({"metadata", "forms", "roles", "dcs", "xdto", "templates", "bsl"})
SEMANTIC_DEPENDENCIES = {
    "metadata": frozenset(), "forms": frozenset({"metadata"}),
    "roles": frozenset({"metadata"}), "dcs": frozenset({"metadata", "templates"}),
    "xdto": frozenset(), "templates": frozenset({"metadata"}),
    "bsl": frozenset({"metadata"}),
}
ABSTRACTIONS = frozenset({"address", "source-contract", "operation-contract", "work-control", "refusal", "execution-contract", "xml-format", "redaction"})
COORDINATORS = frozenset({"operations", "bindings", "navigation", "code-intelligence", "diagnostics", "documentation"})
CODEC_DEPENDENCIES = {"xdto": frozenset({"metadata-designer"})}


def edge_errors(owner: str, dependency: str, kind: str) -> list[str]:
    """All edge kinds matter: a dev backedge also rebuilds a monolithic target."""
    name = owner.removeprefix("unica-")
    target = dependency.removeprefix("unica-")
    prefix = f"{owner} {kind} -> {dependency}: "
    reasons: list[str] = []
    if dependency == "rmcp" and owner != "unica-coder":
        reasons.append("the MCP SDK belongs to the product composition")
    if dependency in {"bsl-parser", "bsl-syntax", "parser", "syntax"} and owner != "unica-bsl-parser":
        reasons.append("upstream parser types belong to the parser boundary")
    if dependency == "unica-coder" and owner != "unica-coder":
        reasons.append("a component or its unit target must not depend on the product composition")
    if name in FAMILIES and dependency.startswith("unica-"):
        allowed = ABSTRACTIONS | frozenset({"operations"}) | SEMANTIC_DEPENDENCIES[name]
        if name == "bsl":
            allowed |= frozenset({"bsl-parser"})
        if target not in allowed:
            reasons.append("semantic families use contracts and their declared semantic dependencies")
    if name == "operations" and dependency.startswith("unica-") and target not in ABSTRACTIONS:
        reasons.append("generic operation orchestration must not import a family or backend")
    if name in COORDINATORS | {"analysis-services"} and dependency.startswith("unica-"):
        if target.startswith("adapter-") or target.endswith("-designer"):
            reasons.append("coordinators invoke ports without importing their concrete implementations")
    if name == "source-designer" and dependency.startswith("unica-") and target not in ABSTRACTIONS:
        reasons.append("source layout uses source contracts without concrete families")
    if name == "workspace" and dependency.startswith("unica-"):
        if target in FAMILIES | COORDINATORS or target.endswith("-designer") or target.startswith("adapter-") or target in {"analysis-services", "daemon", "execution", "coder"}:
            reasons.append("the workspace owner must not import consumers, codecs or analysis implementations")
    if name == "project-health" and dependency.startswith("unica-"):
        allowed = ABSTRACTIONS | frozenset({"workspace", "process", "source-designer"})
        if target not in allowed:
            reasons.append("project health observes source and resource facts without execution or family backends")
    if name.endswith("-designer") and name != "source-designer" and dependency.startswith("unica-"):
        family = name.removesuffix("-designer")
        allowed = ABSTRACTIONS | frozenset({"source-designer", "operations", "bsl-parser", family, "metadata"})
        allowed |= CODEC_DEPENDENCIES.get(family, frozenset())
        if target not in allowed:
            reasons.append("a representation codec consumes neutral ports and its own family")
    if name == "execution" and dependency.startswith("unica-"):
        allowed = ABSTRACTIONS | frozenset({"workspace", "process", "runner"})
        if target not in allowed:
            reasons.append("execution owns outcomes and invokes typed backend ports")
    if name == "daemon" and dependency.startswith("unica-"):
        allowed = ABSTRACTIONS | frozenset({"execution", "workspace", "process", "bootstrap", "artifact-delivery"})
        if target not in allowed:
            reasons.append("daemon owns transport and identity without product dispatch or family backends")
    if name == "bsl-parser" and dependency.startswith("unica-") and target not in {"work-control"}:
        reasons.append("the syntax boundary remains a leaf without family policy")
    return [prefix + reason for reason in reasons]


def manifests(root: Path) -> dict[str, tuple[Path, dict]]:
    document = tomllib.loads((root / "Cargo.toml").read_text())
    result: dict[str, tuple[Path, dict]] = {}
    for pattern in document["workspace"]["members"]:
        candidates = sorted(root.glob(pattern))
        if not candidates:
            raise ValueError(f"unresolved member {pattern}")
        for directory in candidates:
            manifest = directory / "Cargo.toml"
            parsed = tomllib.loads(manifest.read_text())
            name = parsed["package"]["name"]
            if name in result:
                raise ValueError(f"duplicate member {name}")
            result[name] = (directory, parsed)
    return result


def check(root: Path) -> list[str]:
    root = root.resolve()
    members = manifests(root)
    paths = {directory.resolve(): name for name, (directory, _) in members.items()}
    workspace = tomllib.loads((root / "Cargo.toml").read_text())["workspace"]
    shared = workspace.get("dependencies", {})
    diagnostics: list[str] = []
    for owner, (directory, document) in sorted(members.items()):
        for table in (document, *document.get("target", {}).values()):
            for kind in ("dependencies", "build-dependencies", "dev-dependencies"):
                for alias, raw in table.get(kind, {}).items():
                    inherited = isinstance(raw, dict) and raw.get("workspace")
                    dependency = shared[alias] if inherited else raw
                    name = dependency.get("package", alias) if isinstance(dependency, dict) else alias
                    if isinstance(dependency, dict) and "path" in dependency:
                        dependency_root = root if inherited else directory
                        path = (dependency_root / dependency["path"]).resolve()
                        if path not in paths:
                            raise ValueError(f"{owner}: local dependency {alias} is not a workspace member")
                        if name != paths[path]:
                            raise ValueError(f"{owner}: dependency {alias} names {name}, path provides {paths[path]}")
                    diagnostics.extend(edge_errors(owner, name, kind))
    return sorted(set(diagnostics))


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--repo-root", type=Path, default=Path(__file__).resolve().parents[2])
    arguments = parser.parse_args()
    try:
        diagnostics = check(arguments.repo_root)
    except (OSError, ValueError, KeyError, TypeError) as error:
        print(f"component boundary input error: {error}")
        return 1
    for diagnostic in diagnostics:
        print(diagnostic)
    return int(bool(diagnostics))


if __name__ == "__main__":
    raise SystemExit(main())
