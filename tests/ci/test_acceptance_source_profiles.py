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
from unittest.mock import patch

from tests.ci.acceptance_profiles import (
    REPO, SOURCE_PROFILE_DIR, load_source_workspaces, load_source_workspace_profiles, prepare_source_workspace,
)


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

    def test_source_fixture_can_declare_git_initialization(self):
        with tempfile.TemporaryDirectory() as raw:
            root = Path(raw)
            register(root, "workspace-git")
            declaration = root / SOURCE_PROFILE_DIR / "workspace-git.json"
            for enabled in (True, False):
                with self.subTest(git=enabled):
                    declaration.write_text(json.dumps({"profile": "source", "git": enabled}), encoding="utf-8")
                    self.assertEqual(load_source_workspaces(root), {"tests/fixtures/acceptance/workspace-git"})
                    self.assertIs(load_source_workspace_profiles(root)["tests/fixtures/acceptance/workspace-git"]["git"], enabled)

    def test_invalid_json_and_profile_schema_fail_closed(self):
        with tempfile.TemporaryDirectory() as raw:
            root = Path(raw)
            register(root, "workspace-first")
            declaration = root / SOURCE_PROFILE_DIR / "workspace-first.json"
            for content in (
                '[]', '{}', '{"profile":true}', '{"profile":"delivery"}',
                '{"profile":"source","extra":true}',
                '{"profile":"source","git":null}', '{"profile":"source","git":1}',
                '{"profile":"source","git":0}', '{"profile":"source","git":"true"}',
                '{"profile":"source","git":[]}', '{"profile":"source","git":{}}',
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


class SourceFixtureGitPreparationTests(unittest.TestCase):
    @staticmethod
    def git(workspace, *arguments):
        environment = {
            **{key: value for key, value in os.environ.items() if not key.startswith("GIT_")},
            "GIT_CONFIG_GLOBAL": os.devnull, "GIT_CONFIG_NOSYSTEM": "1",
        }
        return subprocess.run(["git", *arguments], cwd=workspace, env=environment,
                              check=True, capture_output=True)

    def test_git_index_is_ready_before_server_start_and_only_the_copy_changes(self):
        from tests.ci import test_acceptance_scenarios as harness
        for profile in ({"profile": "source", "git": True},
                        {"profile": "source", "git": False}, {"profile": "source"}):
            with self.subTest(profile=profile), tempfile.TemporaryDirectory() as raw:
                root = Path(raw)
                original = root / "fixture"
                original.mkdir()
                module = b"// fixture needle\r\n"
                (original / "Module.bsl").write_bytes(module)
                starts = []
                case = self

                class Server:
                    def __init__(self, workspace, *_):
                        self.workspace = workspace
                        starts.append(workspace)
                        case.assertNotEqual(workspace.resolve(), original.resolve())
                        case.assertEqual((workspace / "Module.bsl").read_bytes(), module)
                        if profile.get("git", False):
                            case.assertEqual(case.git(workspace, "ls-files", "-z").stdout, b"Module.bsl\x00")
                            case.assertIn(b"fixture needle", case.git(workspace, "grep", "-n", "needle").stdout)
                        else:
                            case.assertFalse((workspace / ".git").exists())
                    def close(self):
                        pass

                corpus = {"workspace": "fixture", "protocolVersion": "2025-03-26",
                          "scenarios": [{"id": "git-fixture", "wire": []}]}
                with (patch.object(harness, "REPO_ROOT", root),
                      patch.object(harness, "SOURCE_WORKSPACE_PROFILES", {"fixture": profile}),
                      patch.object(harness, "AcceptanceServer", Server)):
                    harness.run_corpus(self, corpus)
                self.assertEqual(len(starts), 1)
                self.assertEqual(sorted(path.name for path in original.iterdir()), ["Module.bsl"])
                self.assertEqual((original / "Module.bsl").read_bytes(), module)

    def test_inherited_git_context_and_global_config_cannot_redirect_or_rewrite_the_index(self):
        with tempfile.TemporaryDirectory() as raw:
            root = Path(raw)
            foreign = root / "foreign"
            foreign.mkdir()
            (foreign / "sentinel.txt").write_text("foreign sentinel\n", encoding="utf-8")
            prepare_source_workspace(foreign, {"git": True})
            foreign_index = (foreign / ".git/index").read_bytes()
            workspace = root / "copy"
            workspace.mkdir()
            module = b"\xef\xbb\xbf// fixture needle\r\n"
            (workspace / "Module.bsl").write_bytes(module)
            global_config = root / "gitconfig"
            global_config.write_text("[core]\nautocrlf=true\n", encoding="utf-8")
            contamination = {
                "GIT_DIR": str(foreign / ".git"), "GIT_WORK_TREE": str(foreign),
                "GIT_INDEX_FILE": str(foreign / ".git/index"),
                "GIT_COMMON_DIR": str(foreign / ".git"),
                "GIT_OBJECT_DIRECTORY": str(foreign / ".git/objects"),
                "GIT_CONFIG_COUNT": "1", "GIT_CONFIG_KEY_0": "core.worktree",
                "GIT_CONFIG_VALUE_0": str(foreign), "GIT_CONFIG_GLOBAL": str(global_config),
            }
            with patch.dict(os.environ, contamination):
                prepare_source_workspace(workspace, {"git": True})
            self.assertEqual(self.git(workspace, "ls-files", "-z").stdout, b"Module.bsl\x00")
            self.assertEqual(self.git(workspace, "show", ":Module.bsl").stdout, module)
            self.assertEqual((workspace / "Module.bsl").read_bytes(), module)
            self.assertEqual((foreign / ".git/index").read_bytes(), foreign_index)
            self.assertEqual(self.git(foreign, "ls-files", "-z").stdout, b"sentinel.txt\x00")

    def test_fixture_gitdir_file_cannot_replace_a_foreign_repository_index(self):
        with tempfile.TemporaryDirectory() as raw:
            root = Path(raw)
            foreign = root / "foreign"
            foreign.mkdir()
            (foreign / "sentinel.txt").write_text("foreign sentinel\n", encoding="utf-8")
            prepare_source_workspace(foreign, {"git": True})
            before_index = (foreign / ".git/index").read_bytes()
            workspace = root / "copy"
            workspace.mkdir()
            (workspace / "Module.bsl").write_bytes(b"// fixture needle\n")
            pointer = workspace / ".git"
            pointer.write_text(f"gitdir: {foreign / '.git'}\n", encoding="utf-8")
            try:
                with self.assertRaisesRegex(ValueError, "existing .git"):
                    prepare_source_workspace(workspace, {"git": True})
            finally:
                self.assertEqual((foreign / ".git/index").read_bytes(), before_index)
                self.assertEqual(self.git(foreign, "ls-files", "-z").stdout, b"sentinel.txt\x00")

    def test_existing_git_entry_is_rejected_before_running_git(self):
        for kind in ("file", "directory", "dangling-link"):
            with self.subTest(kind=kind), tempfile.TemporaryDirectory() as raw:
                workspace = Path(raw)
                entry = workspace / ".git"
                if kind == "file":
                    entry.write_text("unexpected Git metadata", encoding="utf-8")
                elif kind == "directory":
                    entry.mkdir()
                else:
                    try:
                        entry.symlink_to(workspace / "missing-git-dir", target_is_directory=True)
                    except OSError as error:
                        self.skipTest(f"filesystem cannot create a test symlink: {error}")
                with patch("tests.ci.acceptance_profiles.subprocess.run") as run:
                    with self.assertRaisesRegex(ValueError, "existing .git"):
                        prepare_source_workspace(workspace, {"git": True})
                    run.assert_not_called()

    def test_git_initialization_or_index_failure_prevents_server_start(self):
        from tests.ci import test_acceptance_scenarios as harness
        for failed in ("init", "add"):
            with self.subTest(failed=failed), tempfile.TemporaryDirectory() as raw:
                root = Path(raw)
                (root / "fixture").mkdir()
                corpus = {"workspace": "fixture", "protocolVersion": "2025-03-26",
                          "scenarios": [{"id": "git-fixture", "wire": []}]}
                actual_run = subprocess.run

                def run(arguments, **kwargs):
                    if failed in arguments:
                        raise subprocess.CalledProcessError(128, arguments, stderr=b"git preparation sentinel")
                    return actual_run(arguments, **kwargs)

                with (patch.object(harness, "REPO_ROOT", root),
                      patch.object(harness, "SOURCE_WORKSPACE_PROFILES", {"fixture": {"git": True}}),
                      patch.object(harness, "AcceptanceServer") as server,
                      patch("tests.ci.acceptance_profiles.subprocess.run", side_effect=run)):
                    with self.assertRaises(subprocess.CalledProcessError):
                        harness.run_corpus(self, corpus)
                    server.assert_not_called()


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
            **{key: value for key, value in os.environ.items() if not key.startswith("GIT_")},
            "GIT_CONFIG_GLOBAL": os.devnull, "GIT_CONFIG_NOSYSTEM": "1",
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

    def test_inherited_git_context_cannot_redirect_fixture_operations(self):
        with tempfile.TemporaryDirectory() as raw:
            foreign = Path(raw) / "foreign"
            foreign.mkdir()
            clean_environment = {
                **{key: value for key, value in os.environ.items() if not key.startswith("GIT_")},
                "GIT_CONFIG_GLOBAL": os.devnull, "GIT_CONFIG_NOSYSTEM": "1",
            }

            def foreign_git(*arguments):
                return subprocess.run(
                    ["git", "-c", "user.name=foreign-test",
                     "-c", "user.email=foreign@example.invalid", "-c", "commit.gpgsign=false",
                     "-c", "init.defaultBranch=main", *arguments],
                    cwd=foreign, env=clean_environment, check=True, capture_output=True, text=True,
                )

            def foreign_files():
                return {path.relative_to(foreign).as_posix(): path.read_bytes()
                        for path in foreign.rglob("*")
                        if ".git" not in path.relative_to(foreign).parts and path.is_file()}

            (foreign / "sentinel.txt").write_text("keep foreign data\n", encoding="utf-8")
            foreign_git("init", "-q")
            foreign_git("add", "-A")
            foreign_git("commit", "-q", "-m", "foreign sentinel")
            before_head = foreign_git("rev-parse", "HEAD").stdout
            before_index = (foreign / ".git/index").read_bytes()
            before_files = foreign_files()
            alternate = Path(raw) / "alternate-objects"
            alternate.mkdir()
            contamination = {
                "GIT_DIR": str(foreign / ".git"), "GIT_WORK_TREE": str(foreign),
                "GIT_INDEX_FILE": str(foreign / ".git/index"),
                "GIT_COMMON_DIR": str(foreign / ".git"),
                "GIT_OBJECT_DIRECTORY": str(foreign / ".git/objects"),
                "GIT_ALTERNATE_OBJECT_DIRECTORIES": str(alternate),
                "GIT_PREFIX": "caller-prefix/", "GIT_CONFIG_COUNT": "1",
                "GIT_CONFIG_KEY_0": "core.worktree", "GIT_CONFIG_VALUE_0": str(foreign),
            }
            fixture = type(self)(self._testMethodName)
            self.addCleanup(fixture.doCleanups)
            with patch.dict(os.environ, {**clean_environment, **contamination}, clear=True):
                fixture.setUp()
            (fixture.repo / "fixture-only.txt").write_text("fixture data\n", encoding="utf-8")
            try:
                fixture.git("init", "-q")
                actual_root = Path(fixture.git("rev-parse", "--show-toplevel").stdout.strip())
                self.assertEqual(actual_root.resolve(), fixture.repo.resolve(),
                                 "inherited Git context redirected the fixture to the foreign repository")
                fixture.git("add", "-A")
                fixture.git("commit", "-q", "-m", "fixture data")
                self.assertEqual(fixture.git("show", "HEAD:fixture-only.txt").stdout, "fixture data\n")
            finally:
                self.assertEqual(foreign_git("rev-parse", "HEAD").stdout, before_head)
                self.assertEqual((foreign / ".git/index").read_bytes(), before_index)
                self.assertEqual(foreign_files(), before_files)

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
