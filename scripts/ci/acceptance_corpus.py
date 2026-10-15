#!/usr/bin/env python3
"""Load the acceptance corpus and guard it against silent loss.

The corpus is stored so that two changes, each adding its own scenario, never
touch the same lines:

* `tests/fixtures/acceptance/scenario-corpus.json` holds the header and the
  closed historical block `S001`…`S440`; nothing is appended to it any more;
* every later scenario lives in its own file
  `tests/fixtures/acceptance/scenarios/<id>.json` with a stable identifier
  `S-<issue>-<slug>` that does not depend on the order of addition;
* `tests/fixtures/acceptance/scenario-manifest/<id>.steps` freezes the step
  tools of each scenario, one per line.  A scenario without its manifest, a
  manifest without its scenario, or a wire whose steps differ from the
  manifest fails the corpus test; changing a manifest is the explicit
  decision that the scenario or its steps change.

Loading order is defined: the historical block in file order, then the
per-file scenarios by issue number and slug.

    python scripts/ci/acceptance_corpus.py --check          # verify the manifest
    python scripts/ci/acceptance_corpus.py --freeze S-1489-two-branches
"""

from __future__ import annotations

import argparse
import json
import math
import re
import sys
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[2]
ROOT = REPO_ROOT / "tests/fixtures/acceptance"
LEGACY_FILE = "scenario-corpus.json"
SCENARIO_DIR = "scenarios"
MANIFEST_DIR = "scenario-manifest"
MANIFEST_SUFFIX = ".steps"

# The historical block is closed: its numbers stay as they are, so links to
# `S###` keep finding their scenario, and a new scenario never takes the next
# number.
LEGACY_LAST = 440
LEGACY_ID = re.compile(r"S([0-9]{3})")
STABLE_ID = re.compile(r"S-([1-9][0-9]*)-([a-z0-9]+(?:-[a-z0-9]+)*)")


class CorpusError(ValueError):
    """The corpus storage breaks its own layout."""


def strict_load(path: Path):
    """Parse a corpus JSON file, refusing duplicate keys and non-JSON numbers."""

    def unique_object(pairs):
        result = {}
        for key, value in pairs:
            if key in result:
                raise ValueError(f"duplicate corpus key {key!r}")
            result[key] = value
        return result

    def invalid_constant(value):
        raise ValueError(f"non-JSON corpus number {value}")

    def finite_float(value):
        result = float(value)
        if not math.isfinite(result):
            invalid_constant(value)
        return result

    return json.loads(Path(path).read_text(encoding="utf-8"), object_pairs_hook=unique_object,
                      parse_constant=invalid_constant, parse_float=finite_float)


def stable_order(identifier: str) -> tuple[int, str]:
    match = STABLE_ID.fullmatch(identifier)
    # The issue number is above the closed block, so a number-first order
    # (the site sorts by the first number in the id) keeps S### first.
    if match is None or int(match.group(1)) <= LEGACY_LAST:
        raise CorpusError(
            f"{identifier!r}: a new scenario id is S-<issue>-<slug> with an issue number "
            f"above {LEGACY_LAST} (slug: lowercase latin letters, digits and hyphens)"
        )
    return int(match.group(1)), match.group(2)


def visible_files(directory: Path) -> list[Path]:
    """Files of a storage directory; hidden ones such as `.DS_Store` are not
    part of the corpus."""
    if not directory.is_dir():
        return []
    return sorted(path for path in directory.iterdir() if not path.name.startswith("."))


def load(root: Path = ROOT) -> dict:
    """The whole corpus: the header and scenarios in their defined order."""
    root = Path(root)
    corpus = strict_load(root / LEGACY_FILE)
    legacy = corpus["scenarios"]
    for scenario in legacy:
        match = LEGACY_ID.fullmatch(str(scenario.get("id")))
        if match is None or not 1 <= int(match.group(1)) <= LEGACY_LAST:
            raise CorpusError(
                f"{root / LEGACY_FILE}: {scenario.get('id')!r} is outside the closed block "
                f"S001-S{LEGACY_LAST:03d}; put a new scenario into {SCENARIO_DIR}/<id>.json"
            )
    added = []
    directory = root / SCENARIO_DIR
    for path in visible_files(directory):
        if path.suffix != ".json":
            raise CorpusError(f"{path}: only <id>.json scenario files belong here")
        scenario = strict_load(path)
        if not isinstance(scenario, dict) or scenario.get("id") != path.stem:
            raise CorpusError(f"{path}: the file holds one scenario whose id is {path.stem!r}")
        stable_order(path.stem)
        added.append(scenario)
    added.sort(key=lambda scenario: stable_order(scenario["id"]))
    identifiers = [scenario["id"] for scenario in legacy + added]
    duplicates = sorted({item for item in identifiers if identifiers.count(item) > 1})
    if duplicates:
        raise CorpusError(f"duplicate scenario ids: {', '.join(duplicates)}")
    return {**corpus, "scenarios": legacy + added}


def manifest_steps(scenario: dict) -> list[str]:
    return [step["tool"] for step in scenario["wire"]]


def manifest_text(scenario: dict) -> str:
    return "".join(f"{tool}\n" for tool in manifest_steps(scenario))


def read_manifest(root: Path = ROOT) -> dict[str, list[str]]:
    directory = Path(root) / MANIFEST_DIR
    manifest = {}
    for path in visible_files(directory):
        if path.suffix != MANIFEST_SUFFIX:
            raise CorpusError(f"{path}: only <id>{MANIFEST_SUFFIX} manifest files belong here")
        manifest[path.stem] = path.read_text(encoding="utf-8").splitlines()
    return manifest


def manifest_problems(corpus: dict, manifest: dict[str, list[str]]) -> list[str]:
    """Every way the corpus differs from its frozen manifest, by scenario."""
    problems = []
    scenarios = {scenario["id"]: scenario for scenario in corpus["scenarios"]}
    for identifier in sorted(set(manifest) - set(scenarios)):
        problems.append(
            f"{identifier}: the manifest freezes a scenario the corpus no longer has; "
            f"removing it is decided by deleting {MANIFEST_DIR}/{identifier}{MANIFEST_SUFFIX}"
        )
    for identifier, scenario in scenarios.items():
        if identifier not in manifest:
            problems.append(
                f"{identifier}: no manifest; freeze it with "
                f"`python scripts/ci/acceptance_corpus.py --freeze {identifier}`"
            )
            continue
        frozen, actual = manifest[identifier], manifest_steps(scenario)
        if frozen != actual:
            problems.append(
                f"{identifier}: the manifest freezes {len(frozen)} steps {frozen}, "
                f"the wire has {len(actual)} {actual}"
            )
    return problems


def freeze(identifiers: list[str], root: Path = ROOT) -> list[Path]:
    """Write the manifest of the named scenarios from their current wires."""
    scenarios = {scenario["id"]: scenario for scenario in load(root)["scenarios"]}
    written = []
    for identifier in identifiers:
        if identifier not in scenarios:
            raise CorpusError(f"{identifier}: no such scenario")
        path = Path(root) / MANIFEST_DIR / f"{identifier}{MANIFEST_SUFFIX}"
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(manifest_text(scenarios[identifier]), encoding="utf-8")
        written.append(path)
    return written


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    mode = parser.add_mutually_exclusive_group(required=True)
    mode.add_argument("--check", action="store_true", help="fail when the corpus differs from its manifest")
    mode.add_argument("--freeze", nargs="+", metavar="ID", help="write the manifest of these scenarios")
    args = parser.parse_args(argv)
    if args.freeze:
        for path in freeze(args.freeze):
            print(f"wrote {path.relative_to(REPO_ROOT)}")
        return 0
    problems = manifest_problems(load(), read_manifest())
    for problem in problems:
        print(problem, file=sys.stderr)
    if problems:
        return 1
    print("acceptance corpus matches its manifest")
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
