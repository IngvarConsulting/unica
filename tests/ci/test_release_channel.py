from __future__ import annotations

import contextlib
import importlib.util
import io
import json
import tempfile
import unittest
from pathlib import Path


MODULE_PATH = Path(__file__).resolve().parents[2] / "scripts" / "ci" / "release-channel.py"


def load_module():
    spec = importlib.util.spec_from_file_location("release_channel", MODULE_PATH)
    if spec is None or spec.loader is None:
        raise RuntimeError(f"failed to load {MODULE_PATH}")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


channel_module = load_module()


def run(*argv: str) -> tuple[int, str, str]:
    stdout, stderr = io.StringIO(), io.StringIO()
    with contextlib.redirect_stdout(stdout), contextlib.redirect_stderr(stderr):
        code = channel_module.main(list(argv))
    return code, stdout.getvalue(), stderr.getvalue()


class ReleaseChannelTests(unittest.TestCase):
    def test_a_release_follows_its_own_prerelease(self) -> None:
        # `sort -V` ставит v0.13.0-rc.3 выше v0.13.0: с ним выход полной
        # версии после предвыпуска был бы отвергнут как откат.
        self.assertTrue(channel_module.is_forward("v0.13.0-rc.3", "v0.13.0"))
        self.assertFalse(channel_module.is_forward("v0.13.0", "v0.13.0-rc.3"))

    def test_versions_follow_semver_precedence(self) -> None:
        ordered = [
            "v0.12.3",
            "v0.13.0-alpha",
            "v0.13.0-alpha.1",
            "v0.13.0-alpha.beta",
            "v0.13.0-beta",
            "v0.13.0-beta.2",
            "v0.13.0-beta.11",
            "v0.13.0-rc.1",
            "v0.13.0-rc.9",
            "v0.13.0-rc.10",
            "v0.13.0",
            "v0.13.1-rc.1",
            "v0.13.1",
            "v1.0.0",
        ]
        for older, newer in zip(ordered, ordered[1:]):
            with self.subTest(older=older, newer=newer):
                self.assertTrue(channel_module.is_forward(older, newer))
                self.assertFalse(channel_module.is_forward(newer, older))

    def test_the_same_tag_is_forward_so_a_rerun_resumes(self) -> None:
        for tag in ("v0.13.0", "v0.13.0-rc.3"):
            with self.subTest(tag=tag):
                self.assertTrue(channel_module.is_forward(tag, tag))

    def test_an_unread_catalog_is_never_taken_for_a_first_publication(self) -> None:
        # Каталог всегда раздаёт выпуск; пустой ref значит, что его не прочли,
        # и защита «только вперёд» не должна молча отключаться.
        with self.assertRaises(channel_module.TagError):
            channel_module.is_forward("", "v0.13.0-rc.3")
        self.assertEqual(run("forward", "", "v0.13.0-rc.3")[0], 2)

    def test_the_suffix_decides_the_channel(self) -> None:
        self.assertEqual(channel_module.channel("v0.13.0"), "stable")
        self.assertEqual(channel_module.channel("v0.13.0-rc.3"), "next")
        # Другие предвыпуски существуют для замеров и никуда не публикуются.
        for tag in ("v0.13.0-alpha", "v0.13.0-rc", "v0.13.0-rc.3.1", "v0.13.0-measure.1"):
            with self.subTest(tag=tag):
                self.assertEqual(channel_module.channel(tag), "none")

    def test_anything_but_a_release_tag_is_refused(self) -> None:
        for tag in (
            "0.13.0",
            "v0.13",
            "v0.13.0.1",
            "v0.13.0-",
            "v0.13.0-rc..1",
            "v0.13.0+build.1",
            "v01.13.0",
            "v0.13.0-rc.01",
            "main",
            "",
        ):
            with self.subTest(tag=tag):
                with self.assertRaises(channel_module.TagError):
                    channel_module.channel(tag)

    def test_command_line_reports_the_decision_through_exit_codes(self) -> None:
        self.assertEqual(run("channel", "v0.13.0-rc.3")[:2], (0, "next\n"))
        self.assertEqual(run("channel", "v0.13.0")[:2], (0, "stable\n"))
        self.assertEqual(run("forward", "v0.13.0-rc.3", "v0.13.0")[0], 0)
        code, _, stderr = run("forward", "v0.13.0", "v0.13.0-rc.3")
        self.assertEqual(code, 1)
        self.assertIn("older than v0.13.0", stderr)
        self.assertEqual(run("forward", "latest", "v0.13.0")[0], 2)
        self.assertEqual(run("channel", "v0.13")[0], 2)


MARKETPLACE = "https://github.com/IngvarConsulting/unica-marketplace.git"


def write_payload(root: Path, tag: str, *, claude_tag: str | None = None) -> None:
    """The catalogs the packager puts into the thin artifact, pinned to `tag`."""
    def source(ref: str) -> dict:
        return {"source": "git-subdir", "url": MARKETPLACE, "path": "plugins/unica", "ref": ref}

    codex = {
        "name": "unica",
        "interface": {"displayName": "Unica"},
        "plugins": [{"name": "unica", "source": source(tag), "policy": {"installation": "AVAILABLE"}, "category": "Coding"}],
    }
    claude = {
        "name": "unica",
        "owner": {"name": "IngvarConsulting"},
        "plugins": [{"name": "unica", "source": source(claude_tag or tag), "version": tag.removeprefix("v")}],
    }
    for relative, catalog in (
        (".agents/plugins/marketplace.json", codex),
        (".claude-plugin/marketplace.json", claude),
    ):
        path = root / relative
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(json.dumps(catalog), encoding="utf-8")


def read_catalogs(root: Path) -> tuple[dict, dict]:
    return (
        json.loads((root / ".agents/plugins/marketplace.json").read_text(encoding="utf-8")),
        json.loads((root / ".claude-plugin/marketplace.json").read_text(encoding="utf-8")),
    )


class ChannelCatalogTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temp = tempfile.TemporaryDirectory()
        self.root = Path(self.temp.name)
        self.payload = self.root / "payload"
        self.channel = self.root / "channel"

    def tearDown(self) -> None:
        self.temp.cleanup()

    def refs(self, root: Path) -> tuple[Path, Path]:
        return root / ".agents/plugins/marketplace.json", root / ".claude-plugin/marketplace.json"

    def test_both_host_catalogs_must_serve_the_same_release(self) -> None:
        write_payload(self.payload, "v0.13.0-rc.3")
        self.assertEqual(channel_module.catalog_ref(*self.refs(self.payload)), "v0.13.0-rc.3")

        write_payload(self.payload, "v0.13.0-rc.3", claude_tag="v0.12.3")
        with self.assertRaises(channel_module.TagError):
            channel_module.catalog_ref(*self.refs(self.payload))

    def test_the_next_channel_is_a_separately_named_marketplace(self) -> None:
        # Одноимённый маркетплейс Claude Code заменил бы основной, а не встал рядом.
        write_payload(self.payload, "v0.13.0-rc.3")
        channel_module.write_catalogs("next", "v0.13.0-rc.3", self.payload, self.channel)

        codex, claude = read_catalogs(self.channel)
        self.assertEqual((codex["name"], claude["name"]), ("unica-next", "unica-next"))
        self.assertEqual(codex["interface"]["displayName"], "Unica (next)")
        self.assertEqual(channel_module.catalog_ref(*self.refs(self.channel)), "v0.13.0-rc.3")
        self.assertEqual(codex["plugins"][0]["name"], "unica")

    def test_a_stable_release_keeps_the_stable_name_in_both_channels(self) -> None:
        write_payload(self.payload, "v0.13.0")
        for target, name in (("stable", "unica"), ("next", "unica-next")):
            with self.subTest(target=target):
                channel_module.write_catalogs(target, "v0.13.0", self.payload, self.channel / target)
                codex, claude = read_catalogs(self.channel / target)
                self.assertEqual((codex["name"], claude["name"]), (name, name))

    def test_a_candidate_never_reaches_the_stable_catalog(self) -> None:
        write_payload(self.payload, "v0.13.0-rc.3")
        with self.assertRaises(channel_module.TagError):
            channel_module.write_catalogs("stable", "v0.13.0-rc.3", self.payload, self.channel)
        self.assertFalse(self.channel.exists())

    def test_catalogs_are_written_only_for_the_release_they_pin(self) -> None:
        write_payload(self.payload, "v0.13.0-rc.2")
        with self.assertRaises(channel_module.TagError):
            channel_module.write_catalogs("next", "v0.13.0-rc.3", self.payload, self.channel)
        write_payload(self.payload, "v0.13.0-alpha")
        with self.assertRaises(channel_module.TagError):
            channel_module.write_catalogs("next", "v0.13.0-alpha", self.payload, self.channel)
        self.assertFalse(self.channel.exists())


if __name__ == "__main__":
    unittest.main()
