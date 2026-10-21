from __future__ import annotations

import copy
import json
import re
import tomllib
import unittest
from pathlib import Path


REPO_ROOT = Path(__file__).resolve().parents[2]
DEPENDENCIES = ("bsl-parser", "bsl-syntax")


def parser_owner_errors(workspace: dict, manifests: list[dict]) -> list[str]:
    errors = []
    for manifest in manifests:
        owner = manifest["package"]["name"]
        tables = [manifest, *manifest.get("target", {}).values()]
        for table in tables:
            for section in ("dependencies", "dev-dependencies", "build-dependencies"):
                for alias, used in table.get(section, {}).items():
                    resolved = workspace.get(alias, {}) if isinstance(used, dict) and used.get("workspace") else used
                    package = resolved.get("package", alias) if isinstance(resolved, dict) else alias
                    if (alias in DEPENDENCIES or package in ("parser", "syntax")) and owner != "unica-bsl-parser":
                        errors.append(f"{owner}: {alias} bypasses the parser component")
    return errors


def revision_errors(cargo: dict, lock: dict, tools: dict, consumer: dict) -> list[str]:
    """Check source relationships, without resolving dependencies or fixing a commit."""
    errors = []
    analyzers = [tool for tool in tools["tools"] if tool["name"] == "bsl-analyzer"]
    if len(analyzers) != 1:
        return ["tools.lock.json must identify one bsl-analyzer source"]
    analyzer = analyzers[0]
    commit = analyzer["sourceCommit"]
    repository = analyzer["repository"]
    if not isinstance(commit, str) or re.fullmatch(r"[0-9a-f]{40}", commit) is None:
        return ["bsl-analyzer sourceCommit must be a full commit"]

    for alias in DEPENDENCIES:
        dependency = cargo["workspace"]["dependencies"].get(alias)
        if not isinstance(dependency, dict):
            errors.append(f"{alias}: missing workspace git pin")
            continue
        package = alias.removeprefix("bsl-")
        if dependency.get("package") != package:
            errors.append(f"{alias}: unexpected Cargo package")
        if dependency.get("git") != repository:
            errors.append(f"{alias}: repository differs from bsl-analyzer")
        if dependency.get("rev") != commit or "branch" in dependency or "tag" in dependency:
            errors.append(f"{alias}: revision differs from bsl-analyzer full commit")

        used = consumer["dependencies"].get(alias)
        if isinstance(used, dict) and used.get("workspace") is True:
            if any(key in used for key in ("git", "rev", "branch", "tag", "path", "version", "package")):
                errors.append(f"{alias}: parser component combines inherited and direct sources")
            used = dependency
        if not isinstance(used, dict) or (
            used.get("package") != package
            or used.get("git") != repository
            or used.get("rev") != commit
            or any(key in used for key in ("branch", "tag", "path"))
        ):
            errors.append(f"{alias}: parser component source differs from bsl-analyzer")

        resolved = [entry for entry in lock["package"] if entry["name"] == package]
        if len(resolved) != 1:
            errors.append(f"{alias}: missing or ambiguous resolved package")
            continue
        # Cargo's git source records both the requested revision and resolved commit.
        if resolved[0].get("source") != f"git+{repository}?rev={commit}#{commit}":
            errors.append(f"{alias}: Cargo.lock source differs from bsl-analyzer")
    return errors


class ParserRevisionTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.cargo = tomllib.loads((REPO_ROOT / "Cargo.toml").read_text(encoding="utf-8"))
        cls.lock = tomllib.loads((REPO_ROOT / "Cargo.lock").read_text(encoding="utf-8"))
        cls.tools = json.loads(
            (REPO_ROOT / "plugins/unica/third-party/tools.lock.json").read_text(encoding="utf-8")
        )
        cls.consumer = tomllib.loads(
            (REPO_ROOT / "crates/unica-bsl-parser/Cargo.toml").read_text(encoding="utf-8")
        )

    def documents(self) -> tuple[dict, dict, dict, dict]:
        return copy.deepcopy((self.cargo, self.lock, self.tools, self.consumer))

    def another_commit(self) -> str:
        current = next(tool for tool in self.tools["tools"] if tool["name"] == "bsl-analyzer")["sourceCommit"]
        return "b" * 40 if current == "a" * 40 else "a" * 40

    def test_repository_parser_and_analyzer_sources_are_aligned(self) -> None:
        self.assertEqual(revision_errors(self.cargo, self.lock, self.tools, self.consumer), [])

    def test_only_the_parser_component_consumes_the_upstream_packages(self) -> None:
        manifests = [tomllib.loads((REPO_ROOT / member / "Cargo.toml").read_text(encoding="utf-8"))
                     for member in self.cargo["workspace"]["members"]]
        self.assertEqual(parser_owner_errors(self.cargo["workspace"]["dependencies"], manifests), [])

    def test_a_consumer_cannot_bypass_the_owner_through_a_renamed_target_dependency(self) -> None:
        workspace = self.cargo["workspace"]["dependencies"]
        for section in ("dependencies", "dev-dependencies", "build-dependencies"):
            for used in ({"workspace": True}, {"package": "parser", "path": "../fork"}):
                alias = "bsl-parser" if used.get("workspace") else "renamed"
                manifest = {"package": {"name": "another-owner"}, "target": {"cfg(unix)": {section: {alias: used}}}}
                with self.subTest(section=section, alias=alias):
                    self.assertEqual(parser_owner_errors(workspace, [manifest]),
                                     [f"another-owner: {alias} bypasses the parser component"])

    def test_changing_only_parser_or_syntax_is_rejected(self) -> None:
        for alias in ("bsl-parser", "bsl-syntax"):
            with self.subTest(dependency=alias):
                cargo, lock, tools, consumer = self.documents()
                cargo["workspace"]["dependencies"][alias]["rev"] = self.another_commit()
                self.assertIn(
                    f"{alias}: revision differs from bsl-analyzer full commit",
                    revision_errors(cargo, lock, tools, consumer),
                )

    def test_changing_only_a_lock_source_is_rejected(self) -> None:
        for package in ("parser", "syntax"):
            for part in ("requested", "resolved", "both"):
                with self.subTest(package=package, part=part):
                    cargo, lock, tools, consumer = self.documents()
                    entry = next(item for item in lock["package"] if item["name"] == package)
                    requested, resolved = entry["source"].split("#")
                    if part in ("requested", "both"):
                        requested = requested.split("?rev=")[0] + "?rev=" + self.another_commit()
                    if part in ("resolved", "both"):
                        resolved = self.another_commit()
                    entry["source"] = requested + "#" + resolved
                    self.assertIn(
                        f"bsl-{package}: Cargo.lock source differs from bsl-analyzer",
                        revision_errors(cargo, lock, tools, consumer),
                    )

    def test_changing_only_the_shipped_analyzer_is_rejected(self) -> None:
        cargo, lock, tools, consumer = self.documents()
        next(tool for tool in tools["tools"] if tool["name"] == "bsl-analyzer")["sourceCommit"] = self.another_commit()
        errors = revision_errors(cargo, lock, tools, consumer)
        for alias in ("bsl-parser", "bsl-syntax"):
            self.assertIn(f"{alias}: revision differs from bsl-analyzer full commit", errors)
            self.assertIn(f"{alias}: Cargo.lock source differs from bsl-analyzer", errors)

    def test_a_coherent_revision_update_is_accepted(self) -> None:
        cargo, lock, tools, consumer = self.documents()
        analyzer = next(tool for tool in tools["tools"] if tool["name"] == "bsl-analyzer")
        commit = self.another_commit()
        analyzer["sourceCommit"] = commit
        for alias in ("bsl-parser", "bsl-syntax"):
            cargo["workspace"]["dependencies"][alias]["rev"] = commit
        for entry in lock["package"]:
            if entry["name"] in ("parser", "syntax"):
                entry["source"] = f"git+{analyzer['repository']}?rev={commit}#{commit}"
        self.assertEqual(revision_errors(cargo, lock, tools, consumer), [])

    def test_a_matching_hash_from_another_repository_is_rejected(self) -> None:
        for alias in ("bsl-parser", "bsl-syntax"):
            with self.subTest(dependency=alias):
                cargo, lock, tools, consumer = self.documents()
                cargo["workspace"]["dependencies"][alias]["git"] = "https://example.invalid/other-analyzer"
                self.assertIn(
                    f"{alias}: repository differs from bsl-analyzer",
                    revision_errors(cargo, lock, tools, consumer),
                )

    def test_a_matching_direct_consumer_pin_is_accepted(self) -> None:
        cargo, lock, tools, consumer = self.documents()
        for alias in ("bsl-parser", "bsl-syntax"):
            consumer["dependencies"][alias] = copy.deepcopy(cargo["workspace"]["dependencies"][alias])
        self.assertEqual(revision_errors(cargo, lock, tools, consumer), [])

    def test_a_different_direct_consumer_source_is_rejected(self) -> None:
        for alias in ("bsl-parser", "bsl-syntax"):
            for changed in ("revision", "repository", "path"):
                with self.subTest(dependency=alias, changed=changed):
                    cargo, lock, tools, consumer = self.documents()
                    used = copy.deepcopy(cargo["workspace"]["dependencies"][alias])
                    if changed == "revision":
                        used["rev"] = self.another_commit()
                    elif changed == "repository":
                        used["git"] = "https://example.invalid/other-analyzer"
                    else:
                        used = {"package": alias.removeprefix("bsl-"), "path": "../other-parser"}
                    consumer["dependencies"][alias] = used
                    self.assertIn(
                        f"{alias}: parser component source differs from bsl-analyzer",
                        revision_errors(cargo, lock, tools, consumer),
                    )

    def test_missing_or_ambiguous_lock_entries_are_rejected(self) -> None:
        for package in ("parser", "syntax"):
            for duplicate in (False, True):
                with self.subTest(package=package, duplicate=duplicate):
                    cargo, lock, tools, consumer = self.documents()
                    entry = next(item for item in lock["package"] if item["name"] == package)
                    if duplicate:
                        lock["package"].append(copy.deepcopy(entry))
                    else:
                        lock["package"].remove(entry)
                    self.assertIn(
                        f"bsl-{package}: missing or ambiguous resolved package",
                        revision_errors(cargo, lock, tools, consumer),
                    )
