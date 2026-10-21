#!/usr/bin/env python3
"""Explain a candidate Rust selection; mandatory CI always executes full."""
from __future__ import annotations

import argparse
import fnmatch
import importlib.util
import json
import os
from pathlib import Path
import posixpath
import subprocess
import tomllib

ROOT = Path(__file__).resolve().parents[2]


def git(root: Path, *arguments: str) -> str:
    return subprocess.check_output(["git", *arguments], cwd=root, text=True).strip()


def graph(root: Path, revision: str) -> dict:
    paths = git(root, "ls-tree", "-r", "--name-only", revision).splitlines()
    root_manifest = tomllib.loads(git(root, "show", f"{revision}:Cargo.toml"))
    workspace = root_manifest["workspace"]
    manifests = [path for path in paths if path.endswith("/Cargo.toml") and any(
        fnmatch.fnmatchcase(path.removesuffix("/Cargo.toml"), member) for member in workspace["members"])]
    if not manifests:
        raise ValueError("workspace has no resolved member manifests")
    packages, by_root, documents = {}, {}, {}
    for path in manifests:
        document = tomllib.loads(git(root, "show", f"{revision}:{path}"))
        package = document["package"]["name"]
        directory = path.removesuffix("/Cargo.toml")
        if package in packages:
            raise ValueError("duplicate workspace package")
        packages[package] = {"root": directory, "dependencies": [], "features": document.get("features", {})}
        documents[package], by_root[directory] = document, package
    for member in workspace["members"]:
        if not any(fnmatch.fnmatchcase(directory, member) for directory in by_root):
            raise ValueError(f"unresolved workspace member {member}")
    shared = workspace.get("dependencies", {})
    for package, document in documents.items():
        tables = [document, *document.get("target", {}).values()]
        for table in tables:
            for kind in ("dependencies", "build-dependencies", "dev-dependencies"):
                for alias, raw in table.get(kind, {}).items():
                    if not isinstance(raw, dict):
                        continue
                    dependency = shared[alias] if raw.get("workspace") else raw
                    if not isinstance(dependency, dict) or "path" not in dependency:
                        continue
                    directory = posixpath.normpath(posixpath.join("" if raw.get("workspace") else packages[package]["root"], dependency["path"]))
                    # Patches and nonmember dependencies are relevant inputs,
                    # but cannot be omitted through this member-only graph.
                    if directory not in by_root:
                        raise ValueError(f"local dependency absent from workspace graph: {directory}")
                    name = by_root[directory]
                    if dependency.get("package", alias) != name:
                        raise ValueError("dependency alias does not match its actual package")
                    packages[package]["dependencies"].append(name)
        packages[package]["dependencies"] = sorted(set(packages[package]["dependencies"]))
    return {"revision": revision, "packages": packages}


def select(base: dict, head: dict, changes: list[dict]) -> dict:
    decision = {"schemaVersion": 1, "mode": "full", "mandatoryMode": "full", "affectedAdmission": False,
                "baseSha": base.get("revision"), "headSha": head.get("revision"), "packages": [], "reasons": []}
    before, after = base.get("packages", {}), head.get("packages", {})
    if not before or not after or not changes:
        decision["reasons"] = ["missing graph or empty/unknown diff"]
        return decision
    roots, reverse = {}, {}
    for packages in (before, after):
        for name, package in packages.items():
            if not isinstance(package.get("root"), str) or not isinstance(package.get("dependencies"), list):
                decision["reasons"] = ["incomplete package graph"]
                return decision
            roots.setdefault(package["root"], set()).add(name)
            for dependency in package["dependencies"]:
                if dependency not in packages:
                    decision["reasons"] = ["unresolved consumer edge"]
                    return decision
                reverse.setdefault(dependency, set()).add(name)
    seeds = set()
    for change in changes:
        path = change.get("path", "")
        owners = {name for directory, names in roots.items() if path.startswith(directory + "/src/") for name in names}
        if change.get("status") not in ("A", "M") or not path.endswith(".rs") or len(owners) != 1:
            decision["reasons"] = [f"full input: {change.get('status', '?')} {path}"]
            return decision
        owner = next(iter(owners))
        if owner == "unica-coder" or owner not in before or owner not in after:
            decision["reasons"] = [f"composition or changed graph membership: {owner}"]
            return decision
        # Changed feature declarations invalidate the candidate graph even if
        # the diff passed to a caller accidentally omitted the manifest.
        if before[owner].get("features") != after[owner].get("features"):
            decision["reasons"] = [f"feature graph changed: {owner}"]
            return decision
        seeds.add(owner)
    selected, pending = set(seeds), list(seeds)
    while pending:
        for consumer in reverse.get(pending.pop(), set()):
            if consumer not in selected:
                selected.add(consumer)
                pending.append(consumer)
    if not selected or any(package not in after for package in selected):
        decision["reasons"] = ["empty candidate or deleted consumer"]
        return decision
    decision.update(mode="affected", packages=sorted(selected),
                    reasons=["modified package sources and reverse consumers in the union of base/head Cargo graphs"],
                    seedPackages=sorted(seeds))
    return decision


def from_event(root: Path) -> dict:
    spec = importlib.util.spec_from_file_location("shadow_workflow_classifier", Path(__file__).with_name("classify-workflow-changes.py"))
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    sha = os.environ.get("GITHUB_SHA", "")
    fallback = {"schemaVersion": 1, "mode": "full", "mandatoryMode": "full", "affectedAdmission": False,
                "headSha": sha, "packages": [], "reasons": []}
    try:
        event = json.loads(Path(os.environ["GITHUB_EVENT_PATH"]).read_text())
        scope = module.classify_event(root, os.environ.get("GITHUB_EVENT_NAME", ""), os.environ.get("GITHUB_REF", ""), sha, event)
        if not scope.base_sha or not scope.head_sha:
            raise ValueError("immutable event bounds are unavailable")
        changes = module.parse_raw_diff(git(root, "diff", "--raw", "-z", "--no-renames", scope.base_sha, scope.head_sha, "--"))
        return select(graph(root, scope.base_sha), graph(root, scope.head_sha),
                      [{"path": change.path, "status": change.status} for change in changes])
    except (KeyError, OSError, ValueError, TypeError, subprocess.CalledProcessError) as error:
        fallback["reasons"] = [f"unknown input; full fallback: {error}"]
        return fallback


def filterset(decision: dict) -> str | None:
    if decision.get("mandatoryMode") != "full" or decision.get("affectedAdmission") is not False:
        raise ValueError("shadow decision cannot admit affected mandatory CI")
    if decision.get("mode") == "full":
        return None
    packages = decision.get("packages")
    if decision.get("mode") != "affected" or not packages or any(
        not isinstance(name, str) or not name or any(character not in "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789_-" for character in name) for name in packages):
        raise ValueError("invalid or empty shadow package selection")
    return " | ".join(f"package(={name})" for name in packages)


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--repo-root", type=Path, default=ROOT)
    parser.add_argument("--out", type=Path, required=True)
    args = parser.parse_args()
    decision = from_event(args.repo_root)
    filterset(decision)
    args.out.parent.mkdir(parents=True, exist_ok=True)
    args.out.write_text(json.dumps(decision, ensure_ascii=False, indent=2) + "\n")
    print(json.dumps(decision, ensure_ascii=False))


if __name__ == "__main__":
    main()
