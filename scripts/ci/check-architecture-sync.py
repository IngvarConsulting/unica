#!/usr/bin/env python3.12
"""Fail a public MCP surface change with no contract-sync evidence.

This guard enforces the syntactic half of INV-MCP-SURFACE-SYNC: adding, removing,
or renaming a public `unica.*` tool is an architecture change, so at least one
contract-relevant ADR, registry, or acceptance artefact moves with it. Review
still has to prove that the actual ADR and registry owners plus the named check
changed together.

The trigger is deliberately narrow. Only added or removed tool-name declarations
in the public registry count. Refactoring inside a handler, editing a schema
field, or renaming a local variable does not trip the guard, because a guard
that cries wolf is turned off within a week.

Usage:

    python3.12 scripts/ci/check-architecture-sync.py --base origin/main
    UNICA_DIFF_BASE=origin/main python3.12 scripts/ci/check-architecture-sync.py
    git diff origin/main... | python3.12 scripts/ci/check-architecture-sync.py -

The base ref is the target branch of the change, and it is the guard's clock for
decision records: a record the target branch carries is history and immutable, a
record only this pull request has is still the author's to edit, renumber or
drop. `main` is the fallback, not an assumption -- a release flow that merges
into another branch passes that branch and gets judged against it.

Without a resolvable base the guard exits 0 and says so: a local checkout that
cannot name its base ref is not evidence of a violation.

That leniency is wrong in CI, where an unresolvable base means the job is
misconfigured, not that the change is clean. `--strict` turns every skip into a
failure, so a guard that cannot run reports itself instead of passing silently.
A shallow checkout is the usual cause: the three-dot diff needs a merge base,
which requires `fetch-depth: 0`.

"Could not run" is the easy half. The dangerous half is running and seeing
nothing, because that is indistinguishable from a clean change. Three ways to
arrange it are closed here:

* A governed file rendered as binary states no lines at all, so every rule
  reads an empty section. One `-diff` line in `.gitattributes` used to hide
  every later edit to every decision record. Reads now pass `--text` and
  `--no-ext-diff`, and a binary section that still names a governed file is a
  failure rather than a silent skip.
* A quoted path names no file the patterns recognise. git quotes any path with
  non-ASCII bytes by default, which in a repository whose records are written
  in Russian is the ordinary case, not an exotic one. Reads now set
  `core.quotePath=false` and the grammar decodes the quoting anyway, because a
  piped diff carries the sender's configuration.
* An anchor that moved leaves the guard reading a file that no longer holds the
  surface. Renaming the registry used to buy permanent silence. The anchors are
  verified against the tree before any diff is judged.
"""

from __future__ import annotations

import argparse
import os
import re
import subprocess
import sys
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[2]

TOOL_REGISTRY = "crates/unica-coder/src/application/mod.rs"

# A public tool declaration in the registry, for example:  name: "unica.form.edit",
TOOL_DECLARATION = re.compile(r'name:\s*"(?P<tool>unica\.[A-Za-z0-9_.]+)"')

# A Rust comment line. `*` needs the space or slash after it so that a deref
# statement is not mistaken for a block-comment continuation.
RUST_COMMENT = re.compile(r"^\s*(?://|/\*|\*(?:[\s/]|$))")

# Files accepted as contract-relevant synchronization evidence.
#
# ADR decisions and registry rules can own normative text; acceptance plans are
# supporting verification evidence, not owners. `spec/architecture/` as a whole
# would also match the glossary, risks and concept notes, where a comma moved is
# not evidence about a new public tool. The guard proves only that a relevant
# slice moved; identifying the actual owner stays a review judgement.
ARCHITECTURE_EVIDENCE = (
    "spec/acceptance/",
    "spec/architecture/invariants.md",
    "spec/architecture/quality-requirements.md",
)
# Where the catalogue lives. Both readers of the boundary -- the record pattern
# below and the read of the target branch -- take the prefix from here, so the
# catalogue cannot move for one of them and stay put for the other.
DECISIONS_DIR = "spec/decisions/"
# The four digits are the record's ID: `ADR-0011` is the file `0011-*.md`. The
# ID is what every citation elsewhere in the spec points at, so it is the thing
# the immutability rules below have to keep resolving.
DECISION_RECORD = re.compile(
    rf"^{re.escape(DECISIONS_DIR)}(?P<id>\d{{4}})-.+\.md$"
)


def is_architecture_evidence(path: str) -> bool:
    return path.startswith(ARCHITECTURE_EVIDENCE) or bool(DECISION_RECORD.match(path))


# `---` and `+++` name files only inside the header block of a file section,
# between `diff --git` and the first `@@`. Inside a hunk they are content: a
# removed line whose text is `-- spec/architecture/x.md` renders as
# `--- spec/architecture/x.md`, and an added line whose text is `++ b/x.md`
# renders as `+++ b/x.md`. Reading either as a file header lets a diff describe
# files it never touches, so the position is part of the grammar here.
DIFF_SECTION_START = "diff --git "
DIFF_OLD_HEADER = re.compile(r"^--- (?P<path>.+)$")
DIFF_NEW_HEADER = re.compile(r"^\+\+\+ (?P<path>.+)$")
# A pure rename carries no `---`/`+++` and no hunks at all: git states it with
# `rename from`/`rename to` and nothing else. Without these two, `git mv` is a
# way to move a file with the guard seeing an empty section.
DIFF_RENAME_FROM = re.compile(r"^rename from (?P<path>.+)$")
DIFF_RENAME_TO = re.compile(r"^rename to (?P<path>.+)$")
DEV_NULL = "/dev/null"

# A section whose content git refuses to render as text. It carries no hunk
# lines, so every rule below reads it as "nothing changed here" -- the quietest
# possible edit. `Binary files ... differ` is what a plain diff prints; the
# `GIT binary patch` payload is what `--binary` prints instead.
DIFF_BINARY_FILES = re.compile(r"^Binary files (?P<old>.+) and (?P<new>.+) differ$")
DIFF_BINARY_PATCH = "GIT binary patch"

# git quotes a path it cannot print literally: the name is wrapped in double
# quotes, and bytes outside printable ASCII become C escapes -- `\321\200` for a
# Russian letter, `\t` for a tab. With `core.quotePath` at its default this is
# the normal rendering for a record named in Russian, which this catalogue is
# full of. A path left quoted matches none of the patterns above, so the record
# it names is judged as no record at all.
QUOTED_PATH = re.compile(r'^"(?P<body>.*)"$')
C_ESCAPES = {
    "a": 0x07,
    "b": 0x08,
    "f": 0x0C,
    "n": 0x0A,
    "r": 0x0D,
    "t": 0x09,
    "v": 0x0B,
    '"': 0x22,
    "\\": 0x5C,
}
OCTAL_DIGITS = "01234567"

# A decision record becomes an immutable statement of what was chosen on the day
# it appears in the target branch, not on the day someone types `accepted`
# (INV-DOC-SUPERSEDE-NOT-EDIT). Until then the record exists only inside its own
# pull request, where editing, renumbering, merging and dropping it are ordinary
# review work; judging those as rewrites is how a guard invents violations.
#
# For a record the target branch already carries, the rewrites a diff gives away
# are all cheap to spot: moving the acceptance date, walking the status
# backwards, parking the record on a status the catalogue does not define,
# dropping the status or date field altogether, deleting the record, and moving
# or renumbering it so its ID stops resolving. Prose edits stay legal only when
# the diff also stamps the `Обновлено` field, which is what the invariant asks an
# editorial change to carry.
#
# Dropping a field and moving the file matter because both unaccept a record
# without ever writing a smaller status: a record with no `Статус` line is no
# longer accepted by the catalogue's own reading, and a record that left
# `spec/decisions/` takes its ID out of every citation that pointed at it.
DATE_FIELD = re.compile(r"^-\s*(?:Дата|Date):\s*`?(?P<value>[0-9]{4}-[0-9]{2}-[0-9]{2})`?")
STATUS_FIELD = re.compile(r"^-\s*(?:Статус|Status):\s*`?(?P<value>[A-Za-z]+)`?")
UPDATED_FIELD = re.compile(r"^-\s*(?:Обновлено|Updated):")
STATUS_ORDER = {"proposed": 0, "accepted": 1, "superseded": 2}
# Statuses that make a record binding. Deleting one is a rewrite by removal:
# every reference to its ID becomes a dangling pointer.
BINDING_STATUSES = frozenset({"accepted", "superseded"})

# `superseded` is a transition, not a starting point: it says this repository
# accepted the decision and later replaced it. A record that enters the
# catalogue already superseded states a history the target branch never had, so
# the status is only reachable from `accepted`.
SUPERSEDED = "superseded"
ACCEPTED = "accepted"

# How a record names the one that replaced it: the citation form `ADR-0021` or a
# link to the file that carries the ID. Either resolves for a reader; a bare
# "заменено более новым решением" does not.
REPLACEMENT_CITATION = re.compile(r"ADR-(?P<id>[0-9]{4})")
REPLACEMENT_LINK = re.compile(r"(?<![0-9])(?P<id>[0-9]{4})-[^\s()]*\.md")


def unquote_path(raw: str) -> str:
    """Decode git's C-style quoting of a path. An unquoted path passes through.

    Undecodable input is returned untouched rather than guessed at: a wrong
    path is worse than a quoted one, because it can name a file the diff never
    mentioned.
    """
    quoted = QUOTED_PATH.match(raw)
    if quoted is None:
        return raw

    body = quoted.group("body")
    decoded = bytearray()
    index = 0
    while index < len(body):
        character = body[index]
        if character != "\\":
            decoded.extend(character.encode("utf-8"))
            index += 1
            continue
        index += 1
        if index >= len(body):
            return raw
        escape = body[index]
        if escape in C_ESCAPES:
            decoded.append(C_ESCAPES[escape])
            index += 1
            continue
        if escape in OCTAL_DIGITS:
            digits = body[index : index + 3]
            if len(digits) < 3 or any(digit not in OCTAL_DIGITS for digit in digits):
                return raw
            decoded.append(int(digits, 8))
            index += 3
            continue
        return raw

    return decoded.decode("utf-8", errors="surrogateescape")


def header_path(raw: str, prefix: str) -> str:
    """The repository-relative path a diff header line names.

    Strips the `a/`/`b/` side prefix after unquoting, not before: in a quoted
    path the prefix sits inside the quotes. `diff.noprefix` output carries no
    prefix at all, which is why stripping is conditional.
    """
    path = unquote_path(raw)
    if path == DEV_NULL:
        return path
    if path.startswith(prefix):
        return path[len(prefix) :]
    return path


class FileDiff:
    """One file section of a unified diff: its two paths and its hunk lines."""

    def __init__(self, old_path: str | None, new_path: str | None) -> None:
        self.old_path = old_path
        self.new_path = new_path
        self.lines: list[str] = []
        # Set when git refused to render the content. The section then states
        # nothing, and "states nothing" must not read as "changed nothing".
        self.binary = False

    @property
    def created(self) -> bool:
        return self.old_path == DEV_NULL

    @property
    def deleted(self) -> bool:
        return self.new_path == DEV_NULL

    @property
    def path(self) -> str | None:
        """The name the change leaves behind, or the deleted name."""
        if self.new_path and self.new_path != DEV_NULL:
            return self.new_path
        if self.old_path and self.old_path != DEV_NULL:
            return self.old_path
        return None

    def paths(self) -> list[str]:
        return [
            path
            for path in (self.old_path, self.new_path)
            if path is not None and path != DEV_NULL
        ]


def iter_file_diffs(diff_text: str):
    """Split a unified diff into file sections, reading headers by position.

    A section opens at `diff --git`. Until the first `@@`, the lines are header
    lines and `---`/`+++`, `rename from`/`rename to` name the two sides of the
    file. From the first `@@` onwards every line is hunk content, whatever it
    starts with -- which is the whole point: content that looks like a header
    must not be read as one.

    A pure rename states its paths only in `rename from`/`rename to`; git emits
    no `---`/`+++` and no hunk for it. Reading those two lines is what stops
    `git mv` from being a blind spot. When both forms are present the `---`/`+++`
    pair comes last and wins, which is the same value by construction.

    A section that names no paths at all (a bare mode change) yields none, so
    callers skip it without a special case.
    """
    current: FileDiff | None = None
    in_header = False

    for line in diff_text.splitlines():
        if line.startswith(DIFF_SECTION_START):
            if current is not None:
                yield current
            current = FileDiff(None, None)
            in_header = True
            continue

        if current is None:
            continue

        if in_header:
            if line.startswith("@@"):
                in_header = False
                continue
            old_header = DIFF_OLD_HEADER.match(line)
            if old_header:
                current.old_path = header_path(old_header.group("path"), "a/")
                continue
            new_header = DIFF_NEW_HEADER.match(line)
            if new_header:
                current.new_path = header_path(new_header.group("path"), "b/")
                continue
            rename_from = DIFF_RENAME_FROM.match(line)
            if rename_from:
                current.old_path = unquote_path(rename_from.group("path"))
                continue
            rename_to = DIFF_RENAME_TO.match(line)
            if rename_to:
                current.new_path = unquote_path(rename_to.group("path"))
                continue
            binary_files = DIFF_BINARY_FILES.match(line)
            if binary_files:
                # The only line a plain binary section carries, and the only
                # place it names its two sides.
                current.binary = True
                current.old_path = header_path(binary_files.group("old"), "a/")
                current.new_path = header_path(binary_files.group("new"), "b/")
                continue
            if line.startswith(DIFF_BINARY_PATCH):
                current.binary = True
                continue
            # `index`, `old mode`, `similarity index` and the rest of the
            # extended header. Nothing here names a hunk line.
            continue

        if line.startswith("@@"):
            continue
        current.lines.append(line)

    if current is not None:
        yield current


class SurfaceChange:
    def __init__(self) -> None:
        self.added: set[str] = set()
        self.removed: set[str] = set()
        self.architecture_files: set[str] = set()

    @property
    def touches_public_surface(self) -> bool:
        return bool(self.added or self.removed)

    @property
    def touches_architecture(self) -> bool:
        return bool(self.architecture_files)

    @property
    def is_violation(self) -> bool:
        return self.touches_public_surface and not self.touches_architecture

    def describe(self) -> str:
        lines = []
        if self.added:
            lines.append("  added tools:   " + ", ".join(sorted(self.added)))
        if self.removed:
            lines.append("  removed tools: " + ", ".join(sorted(self.removed)))
        if self.architecture_files:
            lines.append(
                "  architecture files touched: "
                + ", ".join(sorted(self.architecture_files))
            )
        return "\n".join(lines)


def record_id(path: str | None) -> str | None:
    """The four-digit ID of a decision record, or None for anything else."""
    if path is None or path == DEV_NULL:
        return None
    match = DECISION_RECORD.match(path)
    return match.group("id") if match else None


def is_governed(path: str) -> bool:
    """True for the files this guard is the reader of."""
    return record_id(path) is not None or path == TOOL_REGISTRY


def unreadable_sections(diff_text: str) -> list[str]:
    """Report sections that hide a governed file behind a binary rendering.

    A binary section carries no hunk lines, so both analyses below walk over it
    and find nothing to object to. That is the failure mode this guard exists to
    not have: silence that looks like cleanliness. One `-diff` line in
    `.gitattributes`, or a single NUL byte in a record, was enough to buy it.

    A binary section naming only ordinary files is not this guard's business --
    an image is legitimately binary, and crying wolf about it is how a guard
    gets switched off. A section that names no path at all is reported, because
    an unattributable change is exactly the one that cannot be cleared.
    """
    problems: list[str] = []

    for section in iter_file_diffs(diff_text):
        if not section.binary:
            continue
        paths = section.paths()
        if not paths:
            problems.append(
                "a binary section names no path, so the guard cannot tell "
                "which file it hides"
            )
            continue
        governed = sorted({path for path in paths if is_governed(path)})
        if governed:
            problems.append(
                f"{', '.join(governed)}: rendered as binary, so its content is "
                "unreadable and every rule below it passes by default"
            )

    return problems


class BaseCatalogue:
    """The decision records the target branch already carries.

    Membership is what decides whether a record is history: an ID the target
    branch does not have belongs to an unfinished pull request, whatever the
    diff's own old side says about it. The status is read from the target branch
    too, so `accepted -> superseded` can be recognised even in a diff that shows
    only the added line.
    """

    def __init__(self, statuses: dict[str, str | None]) -> None:
        self.statuses = statuses

    def has(self, identifier: str) -> bool:
        return identifier in self.statuses

    def status(self, identifier: str) -> str | None:
        return self.statuses.get(identifier)


def read_base_records(base: str) -> BaseCatalogue | None:
    """Read `spec/decisions/` out of the target branch. None when git refuses.

    `-z` keeps every path literal: this catalogue is named in Russian, and the
    quoting git applies by default is the same trap the diff grammar already has
    to undo.

    Returning None on failure drops the guard back to judging the diff alone,
    which is the stricter of the two readings -- an unmerged record is then held
    to the same rules as a merged one. A guard that cannot see the target branch
    must not become more permissive than one that can.
    """
    listing = subprocess.run(
        ["git", "ls-tree", "-r", "--name-only", "-z", base, "--", DECISIONS_DIR],
        cwd=REPO_ROOT,
        capture_output=True,
        text=True,
    )
    if listing.returncode != 0:
        return None

    statuses: dict[str, str | None] = {}
    for path in listing.stdout.split("\0"):
        identifier = record_id(path) if path else None
        if identifier is None:
            continue
        content = subprocess.run(
            ["git", "show", f"{base}:{path}"],
            cwd=REPO_ROOT,
            capture_output=True,
            text=True,
        )
        if content.returncode != 0:
            statuses[identifier] = None
            continue
        statuses[identifier] = base_status(content.stdout)

    return BaseCatalogue(statuses)


def base_status(record_text: str) -> str | None:
    """The status a record file states, or None when it states none.

    The field patterns are written against diff bodies, where the leading `-` is
    the markdown bullet rather than the diff marker -- so the same pattern reads
    a plain file.
    """
    for line in record_text.splitlines():
        status = STATUS_FIELD.match(line)
        if status:
            return status.group("value").lower()
    return None


def analyze_decision_records(
    diff_text: str, base: BaseCatalogue | None = None
) -> list[str]:
    """Report decision records the diff rewrites (INV-DOC-SUPERSEDE-NOT-EDIT).

    Immutability starts at the target branch, not at the word `accepted`. With
    `base` given, a record whose ID that branch does not carry exists only inside
    this pull request: editing it, renumbering it, merging it into another record
    or dropping it are review work, not rewrites. Without `base` -- a diff piped
    in with no ref to read -- the diff's own old side is the only statement about
    history available, so a name that was already a record is treated as one.

    A record the target branch carries is held to its ID and to its fields. Its
    ID must still resolve after the change: deleting the file, moving it out of
    `spec/decisions/`, or renumbering it all leave every citation dangling, and
    a 100% rename is the quietest of the three. Its fields must not be walked
    back: a moved acceptance date, a status that goes backwards or leaves the
    catalogue, a status or date line dropped outright. Its prose must not move
    without the `Обновлено` stamp that marks an editorial change.

    `superseded` is held to its origin in both directions. A record reaches it
    only from `accepted`, and only while naming the record that replaced it: a
    record born superseded, or one promoted straight out of `proposed`, writes a
    history the target branch never had.
    """
    violations: list[str] = []

    for section in iter_file_diffs(diff_text):
        before_id = record_id(section.old_path)
        after_id = record_id(section.new_path)
        if before_id is None and after_id is None:
            continue
        # One side carried an ID, so at least one path is a real name.
        path = section.path

        dates: dict[str, list[str]] = {"-": [], "+": []}
        statuses: dict[str, list[str]] = {"-": [], "+": []}
        updated_stamped = False
        prose_edited = False
        replacements: set[str] = set()

        for line in section.lines:
            if not line or line[0] not in "+-":
                continue
            side, body = line[0], line[1:]
            if side == "+":
                for pattern in (REPLACEMENT_CITATION, REPLACEMENT_LINK):
                    for match in pattern.finditer(body):
                        replacements.add(match.group("id"))
            date = DATE_FIELD.match(body)
            if date:
                dates[side].append(date.group("value"))
                continue
            status = STATUS_FIELD.match(body)
            if status:
                statuses[side].append(status.group("value").lower())
                continue
            if UPDATED_FIELD.match(body):
                updated_stamped = updated_stamped or side == "+"
                continue
            if body.strip():
                prose_edited = True

        # The record's own ID is not a reference to the record that replaced it.
        replacements -= {
            identifier for identifier in (before_id, after_id) if identifier
        }

        # Does the target branch already carry this record? That, not the status
        # written in the file, is what makes the record history.
        if base is None:
            in_target_branch = before_id is not None
        else:
            in_target_branch = before_id is not None and base.has(before_id)

        if not in_target_branch:
            # Not history yet. The whole record is the pull request's to change,
            # with two exceptions. It must not land on a number the target branch
            # already spent -- that is what renumbering before merge is for, and
            # the collision would make one of the two IDs unciteable.
            if base is not None and after_id is not None and base.has(after_id):
                violations.append(
                    f"{path}: number {after_id} is already spent in the target "
                    "branch; a number is free only until it merges, so take the "
                    "next unused one"
                )
            # And it must not enter the catalogue as a decision this repository
            # never accepted.
            if SUPERSEDED in statuses["+"]:
                identifier = after_id or before_id
                violations.append(
                    f"{path}: record {identifier} enters the catalogue already "
                    f"`{SUPERSEDED}`, but no branch ever carried it as "
                    f"`{ACCEPTED}`; a record replaced before it merged is "
                    "deleted, rewritten or merged into the record that replaced "
                    "it, not filed as history"
                )
            continue

        # What the record held before this change. The removed side of the diff
        # states it directly; the target branch states it when the diff shows
        # only an added line, which is what a per-commit diff of a header edit
        # looks like.
        prior = base.status(before_id) if base is not None else None
        before_statuses = statuses["-"] or ([prior] if prior else [])

        # A record whose earlier status is visible and non-binding was never
        # something anyone could cite, so withdrawing it is legal. When neither
        # the diff nor the target branch states a status -- which is what a 100%
        # rename with no base ref looks like -- the record is treated as binding:
        # a guard that assumes the harmless case is how `git mv` becomes the way
        # around this rule.
        withdrawn = bool(before_statuses) and not any(
            status in BINDING_STATUSES for status in before_statuses
        )

        if section.deleted:
            if not withdrawn:
                binding = before_statuses[0] if before_statuses else "binding"
                violations.append(
                    f"{path}: {binding} record deleted; supersede it in place "
                    "so its ID keeps resolving"
                )
            continue

        if record_id(section.new_path) != before_id:
            if not withdrawn:
                after = section.new_path or "(unnamed)"
                violations.append(
                    f"{section.old_path} -> {after}: record "
                    f"{before_id} left the catalogue under a new name; every "
                    "citation of its ID stops resolving. Supersede it in place "
                    "instead of moving or renumbering it"
                )
            continue

        if dates["-"] and dates["+"] and dates["-"] != dates["+"]:
            violations.append(
                f"{path}: acceptance date rewritten "
                f"({', '.join(dates['-'])} -> {', '.join(dates['+'])}); "
                "record the editorial change with an Updated field instead"
            )
        if dates["-"] and not dates["+"]:
            violations.append(
                f"{path}: acceptance date removed; a record states what was "
                "chosen and on what date"
            )
        if statuses["-"] and not statuses["+"]:
            violations.append(
                f"{path}: status field removed; a record with no status sits "
                "outside the catalogue, which unaccepts it without saying so"
            )
        for after in statuses["+"]:
            if after not in STATUS_ORDER:
                violations.append(
                    f"{path}: unknown status {after!r}; the catalogue defines "
                    + ", ".join(sorted(STATUS_ORDER))
                )
        for before in before_statuses:
            for after in statuses["+"]:
                rank_before = STATUS_ORDER.get(before)
                rank_after = STATUS_ORDER.get(after)
                if rank_before is None or rank_after is None:
                    continue
                if rank_after < rank_before:
                    violations.append(
                        f"{path}: status moved backwards ({before} -> {after})"
                    )
        if SUPERSEDED in statuses["+"] and SUPERSEDED not in before_statuses:
            if ACCEPTED not in before_statuses:
                held = ", ".join(before_statuses) if before_statuses else "no status"
                violations.append(
                    f"{path}: status set to `{SUPERSEDED}` from {held}; the "
                    f"transition runs from `{ACCEPTED}` only, because a decision "
                    "the branch never accepted has nothing to replace"
                )
            elif not replacements:
                violations.append(
                    f"{path}: superseded without naming the replacing record; "
                    "state `ADR-NNNN` or link its file so a reader can follow "
                    "the chain"
                )
        if prose_edited and not updated_stamped:
            violations.append(
                f"{path}: text rewritten without an Updated field; an editorial "
                "change stamps `Обновлено`, a changed decision gets a new record"
            )

    return violations


def analyze_diff(diff_text: str) -> SurfaceChange:
    """Classify a unified diff. Pure function: no git, no filesystem."""
    change = SurfaceChange()

    for section in iter_file_diffs(diff_text):
        for path in section.paths():
            if is_architecture_evidence(path):
                change.architecture_files.add(path)

        # A deleted registry file still removes every tool it declared, so the
        # section counts when the registry is on either side.
        if TOOL_REGISTRY not in section.paths():
            continue

        for line in section.lines:
            if not line or line[0] not in "+-":
                continue
            body = line[1:]
            # A tool name inside a comment is prose about the registry, not a
            # declaration in it. Counting it made a comment a cancelling token:
            # removing a real declaration and adding `// name: "unica.x"` in the
            # same change left added and removed equal, and the retirement of a
            # public tool passed as a no-op.
            if RUST_COMMENT.match(body):
                continue
            side = change.added if line[0] == "+" else change.removed
            for match in TOOL_DECLARATION.finditer(body):
                side.add(match.group("tool"))

    # A pure rename inside one hunk shows the same tool on both sides only when
    # the name really changed, so nothing to cancel out here. A moved line with
    # an unchanged name does appear on both sides; drop that noise.
    unchanged = change.added & change.removed
    change.added -= unchanged
    change.removed -= unchanged
    return change


def anchor_problems(repo_root: Path | None = None) -> list[str]:
    """Report the guard's own reference points having moved out from under it.

    Every rule here is stated against three hard-coded paths. Nothing made those
    paths prove they still exist, so renaming the registry -- a pure rename, no
    hunks, not a surface change by any rule below -- left the guard reading a
    file that no longer holds the surface, reporting "public MCP surface
    unchanged" forever after. A declaration count is part of the check because
    an empty or restructured registry is the same blindness with the file still
    in place.
    """
    root = REPO_ROOT if repo_root is None else repo_root
    problems: list[str] = []

    registry = root / TOOL_REGISTRY
    if not registry.is_file():
        problems.append(
            f"{TOOL_REGISTRY}: the tool registry this guard reads is not in the "
            "tree; every surface rule below it is unenforced"
        )
    elif not TOOL_DECLARATION.search(registry.read_text(encoding="utf-8")):
        problems.append(
            f"{TOOL_REGISTRY}: no `unica.*` declaration left in the registry "
            "this guard reads; either the surface moved or the pattern stopped "
            "matching it"
        )

    for evidence in ARCHITECTURE_EVIDENCE:
        target = root / evidence
        present = target.is_dir() if evidence.endswith("/") else target.is_file()
        if not present:
            problems.append(
                f"{evidence}: named as contract-sync evidence but not in the "
                "tree, so no change can ever satisfy it"
            )

    return problems


def resolve_base(explicit: str | None) -> str | None:
    """The target branch the change is measured against.

    Given explicitly, it is taken as given: a release flow whose pull requests
    target a branch other than `main` is a normal arrangement, and the guard must
    not overrule the caller about it. `main` is only the fallback for a caller
    that named nothing, which is the repository's ordinary flow.
    """
    for candidate in (explicit, os.environ.get("UNICA_DIFF_BASE")):
        if candidate:
            return candidate
    for candidate in ("origin/main", "main"):
        result = subprocess.run(
            ["git", "rev-parse", "--verify", "--quiet", candidate],
            cwd=REPO_ROOT,
            capture_output=True,
            text=True,
        )
        if result.returncode == 0:
            return candidate
    return None


def read_diff(base: str) -> str | None:
    """The change under judgement, rendered so that nothing can hide in it.

    Every flag here closes a way to make a governed file unreadable:
    `--no-ext-diff` and `--text` defeat both a `-diff` attribute and a stray NUL
    byte, `core.quotePath=false` keeps a Russian file name literal, and the two
    prefix settings keep `a/`/`b/` where the grammar expects them whatever the
    caller's git configuration says.
    """
    result = subprocess.run(
        [
            "git",
            "-c",
            "core.quotePath=false",
            "-c",
            "diff.noprefix=false",
            "-c",
            "diff.mnemonicPrefix=false",
            "diff",
            "--no-ext-diff",
            "--text",
            "--unified=0",
            f"{base}...",
        ],
        cwd=REPO_ROOT,
        capture_output=True,
        text=True,
    )
    if result.returncode != 0:
        return None
    return result.stdout


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--base", help="git ref the change is measured against")
    parser.add_argument(
        "--strict",
        action="store_true",
        help="treat an unusable base ref as a failure instead of a skip",
    )
    parser.add_argument(
        "diff",
        nargs="?",
        help="read a unified diff from stdin when set to '-'",
    )
    args = parser.parse_args(argv)

    def unusable(message: str) -> int:
        if args.strict:
            print(f"check-architecture-sync: {message}")
            print(
                "--strict was requested, so this is a failure: the guard could "
                "not run and therefore proves nothing. A shallow checkout is "
                "the usual cause; the three-dot diff needs `fetch-depth: 0`."
            )
            return 2
        print(f"check-architecture-sync: {message}; skipping")
        return 0

    moved_anchors = anchor_problems()
    if moved_anchors:
        for problem in moved_anchors:
            print(f"check-architecture-sync: {problem}")
        return unusable(
            "the guard's own anchors moved, so it can no longer see what it guards"
        )

    # The target branch decides which records are already history. A piped diff
    # gets one only when the caller names it: auto-resolving a ref for input that
    # may have been produced against a different one would judge the change
    # against a branch nobody chose.
    if args.diff == "-":
        diff_text = sys.stdin.read()
        named_base = args.base or os.environ.get("UNICA_DIFF_BASE")
        base_records = read_base_records(named_base) if named_base else None
    else:
        base = resolve_base(args.base)
        if base is None:
            return unusable(
                "no base ref resolved (pass --base or set UNICA_DIFF_BASE)"
            )
        diff_text = read_diff(base)
        if diff_text is None:
            return unusable(f"cannot diff against {base!r}")
        base_records = read_base_records(base)
        if base_records is None:
            print(
                f"check-architecture-sync: cannot list decision records in "
                f"{base!r}; judging every record in the diff as already merged"
            )

    hidden = unreadable_sections(diff_text)
    if hidden:
        print(
            "check-architecture-sync: a governed file is unreadable in this diff"
        )
        for problem in hidden:
            print(f"  {problem}")
        print()
        print(
            "A decision record and the tool registry are text. Rendered as binary they\n"
            "state nothing, and a guard reading nothing reports success. Drop the\n"
            "`-diff` attribute or the stray byte so the change can be read."
        )
        return 1

    rewritten = analyze_decision_records(diff_text, base_records)
    if rewritten:
        print(
            "check-architecture-sync: a decision record breaks the catalogue's "
            "lifecycle"
        )
        for violation in rewritten:
            print(f"  {violation}")
        print()
        print(
            "INV-DOC-SUPERSEDE-NOT-EDIT: a record becomes history when it reaches the target\n"
            "branch, and states from then on what was chosen on its date. When the\n"
            "choice stops applying, supersede it with a new record instead of\n"
            "editing it to match the code. A record this pull request has not\n"
            "merged yet is not history: edit, renumber or drop it here rather than\n"
            "filing it as `superseded`."
        )
        return 1

    change = analyze_diff(diff_text)
    if not change.touches_public_surface:
        print("check-architecture-sync: public MCP surface unchanged")
        return 0

    if not change.is_violation:
        print("check-architecture-sync: surface change has contract-sync evidence")
        print(change.describe())
        return 0

    print("check-architecture-sync: public MCP surface changed without spec sync")
    print(change.describe())
    print()
    print(
        "INV-MCP-SURFACE-SYNC requires the owning decision record, the registry entry that\n"
        "derives from it, and the check named by that entry to change together.\n"
        "This guard proves only the weaker half: that contract-relevant sync\n"
        "evidence moved in the same change. Review must still identify and\n"
        "update the ADR/registry owner. Update one of:\n"
        "spec/decisions/NNNN-*.md,\n"
        "spec/architecture/invariants.md, spec/architecture/quality-requirements.md,\n"
        "spec/acceptance/."
    )
    return 1


if __name__ == "__main__":
    raise SystemExit(main())
