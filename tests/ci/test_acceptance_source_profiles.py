"""Explicit source-fixture admission survives independent Git additions."""
from __future__ import annotations

import json
import os
import shutil
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

from tests.ci.acceptance_profiles import REPO, SOURCE_PROFILE_DIR, load_source_workspaces


def register(root: Path, name: str) -> None:
    workspace = root / name
    workspace.mkdir()
    (workspace / "README.md").write_text(name, encoding="utf-8")
    directory = root / SOURCE_PROFILE_DIR
    directory.mkdir(exist_ok=True)
    (directory / f"{name}.json").write_text('{"profile":"source"}\n', encoding="utf-8")


class SourceProfileStorageTests(unittest.TestCase):
    def test_only_explicit_sibling_fixtures_are_registered(self):
        with tempfile.TemporaryDirectory() as raw:
            root = Path(raw)
            register(root, "workspace-second")
            register(root, "workspace-first")
            (root / "workspace-unregistered").mkdir()
            for name in (".DS_Store", ".ignored.json", "..json"):
                (root / SOURCE_PROFILE_DIR / name).write_bytes(b"not JSON")
            self.assertEqual(load_source_workspaces(root), {
                "tests/fixtures/acceptance/workspace-first",
                "tests/fixtures/acceptance/workspace-second",
            })

    def test_invalid_json_and_profile_schema_fail_closed(self):
        with tempfile.TemporaryDirectory() as raw:
            root = Path(raw)
            register(root, "workspace-first")
            declaration = root / SOURCE_PROFILE_DIR / "workspace-first.json"
            for content in (
                '[]', '{}', '{"profile":true}', '{"profile":"delivery"}',
                '{"profile":"source","extra":true}',
                '{"profile":"source","profile":"source"}',
                '{"profile":"source",}',
            ):
                with self.subTest(content=content):
                    declaration.write_text(content, encoding="utf-8")
                    with self.assertRaises(ValueError):
                        load_source_workspaces(root)

    def test_missing_or_nonregular_storage_is_rejected(self):
        for case in ("missing catalog", "file catalog", "wrong extension",
                     "directory declaration", "missing fixture", "file fixture"):
            with self.subTest(case=case), tempfile.TemporaryDirectory() as raw:
                root = Path(raw)
                catalog = root / SOURCE_PROFILE_DIR
                catalog.mkdir()
                declaration = catalog / "workspace-first.json"
                if case == "missing catalog":
                    catalog.rmdir()
                elif case == "file catalog":
                    catalog.rmdir()
                    catalog.write_text("not a directory", encoding="utf-8")
                elif case == "wrong extension":
                    declaration.with_suffix(".txt").write_text('{"profile":"source"}', encoding="utf-8")
                elif case == "directory declaration":
                    declaration.mkdir()
                else:
                    declaration.write_text('{"profile":"source"}', encoding="utf-8")
                    if case == "file fixture":
                        (root / "workspace-first").write_text("not a directory", encoding="utf-8")
                with self.assertRaises((ValueError, OSError)):
                    load_source_workspaces(root)

    def test_linked_catalog_declaration_and_workspace_are_rejected(self):
        for case in ("catalog", "declaration", "workspace", "dangling declaration"):
            with self.subTest(case=case), tempfile.TemporaryDirectory() as raw:
                root = Path(raw)
                register(root, "workspace-first")
                if case == "catalog":
                    path = root / SOURCE_PROFILE_DIR
                elif case in {"declaration", "dangling declaration"}:
                    path = root / SOURCE_PROFILE_DIR / "workspace-first.json"
                else:
                    path = root / "workspace-first"
                target = root / "linked-target"
                is_directory = path.is_dir()
                if case == "dangling declaration":
                    path.unlink()
                else:
                    path.rename(target)
                try:
                    path.symlink_to(target, target_is_directory=is_directory)
                except OSError as error:
                    self.skipTest(f"filesystem cannot create a test symlink: {error}")
                with self.assertRaises(ValueError):
                    load_source_workspaces(root)


class IndependentSourceAdditionTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.repo = Path(temporary.name) / "repo"
        self.repo.mkdir()
        # Execute the actual implementation in a new process and repository.
        # Empty package initializers keep unrelated installed 'tests' packages out.
        for package in ("tests", "tests/ci"):
            directory = self.repo / package
            directory.mkdir(exist_ok=True)
            (directory / "__init__.py").write_text("", encoding="utf-8")
        for relative in (
            "tests/ci/acceptance_profiles.py", "tests/ci/acceptance_controls.py",
            "scripts/ci/acceptance_corpus.py",
        ):
            destination = self.repo / relative
            destination.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(REPO / relative, destination)
        self.root = self.repo / "tests/fixtures/acceptance"
        self.root.mkdir(parents=True)
        register(self.root, "workspace-base")
        self.base = {"id": "S001", "wire": [{
            "tool": "unica.view", "args": {}, "expect": ["ok"],
        }]}
        (self.root / "scenario-corpus.json").write_text(json.dumps({
            "workspace": "tests/fixtures/acceptance/workspace-base",
            "scenarios": [self.base],
        }), encoding="utf-8")
        manifest = self.root / "scenario-manifest"
        manifest.mkdir()
        (manifest / "S001.steps").write_text("unica.view\n", encoding="utf-8")
        self.environment = {
            **os.environ, "GIT_CONFIG_GLOBAL": os.devnull, "GIT_CONFIG_NOSYSTEM": "1",
            "PYTHONPATH": str(self.repo), "PYTHONDONTWRITEBYTECODE": "1",
        }

    def git(self, *arguments):
        return subprocess.run(
            ["git", "-c", "user.name=source-profile-test",
             "-c", "user.email=source-profile@example.invalid", "-c", "commit.gpgsign=false",
             "-c", "init.defaultBranch=main", *arguments],
            cwd=self.repo, env=self.environment, check=True, capture_output=True, text=True,
        )

    def fresh_python(self, code):
        return subprocess.run(
            [sys.executable, "-B", "-c", code], cwd=self.repo, env=self.environment,
            check=True, capture_output=True, text=True,
        )

    def add_scenario(self, identifier, workspace):
        scenario = {**self.base, "id": identifier,
                    "workspace": f"tests/fixtures/acceptance/{workspace}"}
        directory = self.root / "scenarios"
        directory.mkdir(exist_ok=True)
        (directory / f"{identifier}.json").write_text(json.dumps(scenario), encoding="utf-8")
        (self.root / "scenario-manifest" / f"{identifier}.steps").write_text("unica.view\n", encoding="utf-8")

    def test_unregistered_existing_source_fixture_fails_even_in_other_profiles(self):
        (self.root / "workspace-unregistered").mkdir()
        self.add_scenario("S-9001-unregistered", "workspace-unregistered")
        observed = self.fresh_python('''
import json
from tests.ci.acceptance_controls import load_acceptance_corpus
from tests.ci.acceptance_profiles import select_profile
corpus = load_acceptance_corpus()
rejected = []
for profile in ("source", "delivery", "agent-evaluation"):
    try:
        select_profile(corpus, profile)
    except ValueError:
        rejected.append(profile)
print(json.dumps(rejected))
''')
        self.assertEqual(json.loads(observed.stdout), ["source", "delivery", "agent-evaluation"])

    def test_two_independent_fixture_additions_merge_and_are_admitted_on_fresh_import(self):
        self.git("init", "-q")
        self.git("add", "-A")
        self.git("commit", "-q", "-m", "base")
        for branch, identifier in (("first", "S-9001-first"), ("second", "S-9002-second")):
            self.git("switch", "-q", "-c", branch, "main")
            register(self.root, f"workspace-{branch}")
            self.add_scenario(identifier, f"workspace-{branch}")
            self.git("add", "-A")
            self.git("commit", "-q", "-m", f"add {branch} source fixture")
        self.git("switch", "-q", "main")
        self.git("merge", "-q", "--no-edit", "first")
        self.git("merge", "-q", "--no-edit", "second")
        observed = self.fresh_python('''
import json
from tests.ci.acceptance_controls import corpus_storage, load_acceptance_corpus
from tests.ci.acceptance_profiles import SOURCE_WORKSPACES, select_profile
storage = corpus_storage()
corpus = load_acceptance_corpus()
problems = storage.manifest_problems(corpus, storage.read_manifest())
if problems:
    raise AssertionError(problems)
selected = select_profile(corpus, "source")
print(json.dumps({"scenarios": [s["id"] for s in selected["scenarios"]],
                  "workspaces": sorted(SOURCE_WORKSPACES)}))
''')
        self.assertEqual(json.loads(observed.stdout), {
            "scenarios": ["S001", "S-9001-first", "S-9002-second"],
            "workspaces": [f"tests/fixtures/acceptance/workspace-{name}"
                           for name in ("base", "first", "second")],
        })
