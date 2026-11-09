#!/usr/bin/env python3
"""Assemble the ``all_kinds`` seed sources: one object of every kind.

The base is the acceptance workspace ``tests/fixtures/acceptance/workspace/src``
without the objects in ``BASE_DROPPED``. Every other kind is taken, one object
each, from another acceptance fixture (``SOURCES`` names the file). Fixture
files are only read: the copy gets fresh identifiers, is registered in
``Configuration.xml`` and passes through the repairs in ``REPAIRS`` that the
platform needs to load it. The result is not a reference by itself: ``refresh-platform-corpus.py``
loads it into a platform and keeps the platform export.

    build-all-kinds.py --out DIR

``--list`` prints the source of every kind and the repairs without writing.
"""

from __future__ import annotations

import argparse
import re
import shutil
import sys
import uuid
from dataclasses import dataclass
from pathlib import Path
from typing import Callable, Dict, List, Optional, Sequence

REPO = Path(__file__).resolve().parents[2]
FIXTURES = REPO / "tests" / "fixtures"
BASE = FIXTURES / "acceptance" / "workspace" / "src"
UUID = re.compile(rb"[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}")


@dataclass(frozen=True)
class Source:
    kind: str  # root element and ChildObjects tag
    folder: str  # directory of the kind in a hierarchical dump
    path: str  # descriptor, relative to tests/fixtures


# Base objects that name dozens of objects absent here: a library document with
# characteristics, the library administration subsystem and the journal of that
# document. They are left out of the copy; SOURCES gives these kinds instead.
BASE_DROPPED = (
    ("Document", "Documents", "АктОбУничтоженииПерсональныхДанных"),
    ("Subsystem", "Subsystems", "Администрирование"),
    ("DocumentJournal", "DocumentJournals", "ДокументыОрганизации"),
)

# One object for every kind the base lacks. Acceptance fixtures come first;
# registers and the charts they name come from one fixture.
SOURCES: Sequence[Source] = (
    Source("Constant", "Constants", "acceptance/workspace-functional-option/src/Constants/Enabled.xml"),
    Source("DefinedType", "DefinedTypes", "acceptance/workspace-defined-type-attribute/src/DefinedTypes/Inner.xml"),
    Source("ExchangePlan", "ExchangePlans", "acceptance/workspace-choice-mode/src/ExchangePlans/Codes.xml"),
    Source("ChartOfCharacteristicTypes", "ChartsOfCharacteristicTypes", "acceptance/workspace-choice-mode/src/ChartsOfCharacteristicTypes/Codes.xml"),
    Source("ChartOfAccounts", "ChartsOfAccounts", "acceptance/workspace-calculation-code-type/src/ChartsOfAccounts/Хозрасчетный.xml"),
    Source("ChartOfCalculationTypes", "ChartsOfCalculationTypes", "acceptance/workspace-calculation-code-type/src/ChartsOfCalculationTypes/ВидыРасчета.xml"),
    Source("Document", "Documents", "acceptance/workspace-calculation-code-type/src/Documents/Приход.xml"),
    Source("Document", "Documents", "acceptance/workspace-nested-member-properties/src/Documents/Orders.xml"),
    Source("DocumentJournal", "DocumentJournals", "acceptance/workspace-nested-member-properties/src/DocumentJournals/Entries.xml"),
    Source("AccumulationRegister", "AccumulationRegisters", "acceptance/workspace-calculation-code-type/src/AccumulationRegisters/Остатки.xml"),
    Source("AccountingRegister", "AccountingRegisters", "acceptance/workspace-calculation-code-type/src/AccountingRegisters/Бухгалтерский.xml"),
    Source("CalculationRegister", "CalculationRegisters", "acceptance/workspace-calculation-code-type/src/CalculationRegisters/Начисления.xml"),
    Source("BusinessProcess", "BusinessProcesses", "acceptance/workspace-business-process-task-mode/src/BusinessProcesses/Approval.xml"),
    Source("Task", "Tasks", "acceptance/workspace-business-process-task-mode/src/Tasks/Work.xml"),
    Source("DataProcessor", "DataProcessors", "acceptance/workspace-borrowed/src/cf/DataProcessors/Обработка.xml"),
    Source("Subsystem", "Subsystems", "acceptance/workspace-library-subsystems/src/Subsystems/СтандартныеПодсистемы.xml"),
    Source("CommonModule", "CommonModules", "acceptance/workspace-defined-type-attribute/src/CommonModules/EventHandlers.xml"),
    Source("CommonForm", "CommonForms", "acceptance/workspace-common-form-defaults/src/CommonForms/MainA.xml"),
    Source("CommonTemplate", "CommonTemplates", "acceptance/workspace-root-comments/src/CommonTemplates/Probe.xml"),
    Source("CommonPicture", "CommonPictures", "acceptance/workspace-root-comments/src/CommonPictures/Probe.xml"),
    Source("CommonCommand", "CommonCommands", "acceptance/workspace-root-comments/src/CommonCommands/Probe.xml"),
    Source("CommandGroup", "CommandGroups", "acceptance/workspace-root-comments/src/CommandGroups/Probe.xml"),
    Source("CommonAttribute", "CommonAttributes", "acceptance/workspace-root-comment-registration/src/CommonAttributes/Probe.xml"),
    Source("SessionParameter", "SessionParameters", "acceptance/workspace-root-comments/src/SessionParameters/SessionProbe.xml"),
    Source("FunctionalOption", "FunctionalOptions", "acceptance/workspace-functional-option/src/FunctionalOptions/ByConstant.xml"),
    Source("FunctionalOptionsParameter", "FunctionalOptionsParameters", "acceptance/workspace-root-comments/src/FunctionalOptionsParameters/Probe.xml"),
    Source("EventSubscription", "EventSubscriptions", "acceptance/workspace-defined-type-attribute/src/EventSubscriptions/CorpusEventSubscription.xml"),
    Source("ScheduledJob", "ScheduledJobs", "acceptance/workspace-language-code/src/ScheduledJobs/KeyProbe.xml"),
    Source("SettingsStorage", "SettingsStorages", "acceptance/workspace-root-comments/src/SettingsStorages/Probe.xml"),
    Source("FilterCriterion", "FilterCriteria", "acceptance/workspace-root-comments/src/FilterCriteria/Probe.xml"),
    Source("Sequence", "Sequences", "acceptance/workspace-root-comments/src/Sequences/Probe.xml"),
    Source("DocumentNumerator", "DocumentNumerators", "acceptance/workspace-numerator-unregistered/src/DocumentNumerators/Sales.xml"),
    Source("Style", "Styles", "acceptance/workspace-root-comments/src/Styles/Probe.xml"),
    Source("StyleItem", "StyleItems", "acceptance/workspace-root-comments/src/StyleItems/Probe.xml"),
    Source("WSReference", "WSReferences", "acceptance/workspace-root-comments/src/WSReferences/Probe.xml"),
    Source("IntegrationService", "IntegrationServices", "acceptance/workspace-root-comments/src/IntegrationServices/Probe.xml"),
)


@dataclass(frozen=True)
class Repair:
    """A change the platform needs to load the assembled sources."""

    why: str  # what the platform rejects without it
    apply: Callable[[Path], int]  # returns the number of changed places


TYPE_SUFFIXES = ("RecordManager", "RecordSet", "RecordKey", "Selection", "Manager", "Object", "List", "Ref")
TYPE_KINDS = {"Characteristic": "ChartOfCharacteristicTypes"}


def present_objects(out: Path) -> set:
    data = (out / "Configuration.xml").read_text(encoding="utf-8-sig")
    block = re.search(r"<ChildObjects>(.*)</ChildObjects>", data, re.S).group(1)
    return {f"{kind}.{name}" for kind, name in re.findall(r"<(\w+)>([^<]+)</\1>", block)}


def known_kinds(present: set) -> set:
    return {item.split(".", 1)[0] for item in present} | {kind for kind, _, _ in BASE_DROPPED}


def type_object(token: str) -> str:
    """``CatalogRef.X`` and ``Characteristic.X`` name the objects ``Catalog.X`` and so on."""
    kind, _, rest = token.partition(".")
    name = rest.split(".", 1)[0]
    if kind in TYPE_KINDS:
        return f"{TYPE_KINDS[kind]}.{name}"
    for suffix in TYPE_SUFFIXES:
        if kind.endswith(suffix) and len(kind) > len(suffix):
            return f"{kind[: -len(suffix)]}.{name}"
    return f"{kind}.{name}"


def retarget_absent(out: Path) -> int:
    """Point references to absent objects at objects the assembly has.

    A reference to a whole object (``Catalog.X``, ``cfg:CatalogRef.X``) is
    redirected to an object of the same kind that is present, so a list such
    as an event source or registered documents does not end up empty. A
    reference into an absent object (``Catalog.X.Attribute.Y``), a command
    interface entry or a reference with no object of its kind present is
    removed; a single value element becomes empty.
    """
    present = present_objects(out)
    kinds = known_kinds(present)
    substitute: Dict[str, str] = {}
    for item in sorted(present):
        kind, _, name = item.partition(".")
        substitute.setdefault(kind, name)

    def absent(reference: str) -> bool:
        kind, _, rest = reference.partition(".")
        return kind in kinds and f"{kind}.{rest.split('.', 1)[0]}" not in present

    def redirect(reference: str, kind: str) -> Optional[str]:
        """The reference with its object replaced, or None to remove it."""
        prefix, _, rest = reference.partition(".")
        if "." in rest or kind not in substitute:
            return None
        return f"{prefix}.{substitute[kind]}"

    def object_kind(reference: str) -> str:
        return reference.split(".", 1)[0]

    def type_kind(token: str) -> str:
        return type_object(token).split(".", 1)[0]

    # Matched on the text, not on lines: fixtures hold both indented and
    # single-line XML. A removed element takes its leading line break along.
    lead = r"(?:\r?\n[ \t]*)?"
    rules = (
        (re.compile(lead + r'(<xr:Item xsi:type="xr:MDObjectRef">)([^<]+)(</xr:Item>)'), object_kind, absent),
        (re.compile(lead + r"(<xr:Ref>)([^<]+)(</xr:Ref>)"), object_kind, absent),
        (
            re.compile(lead + r"(<v8:Type(?:Set)?>cfg:)([^<]+)(</v8:Type(?:Set)?>)"),
            type_kind,
            lambda token: absent(type_object(token)),
        ),
    )
    command = re.compile(lead + r'<Command name="([^"]+)">.*?</Command>', re.S)
    value = re.compile(r"<([\w:]+)>(\w+\.\w+(?:\.[\w.]+)?)</\1>")

    def apply_rule(match: "re.Match[str]", kind_of, is_absent) -> str:
        head, reference, tail = match.group(1), match.group(2), match.group(3)
        if not is_absent(reference):
            return match.group(0)
        target = redirect(reference, kind_of(reference))
        if target is None:
            return ""
        return match.group(0).replace(head + reference + tail, head + target + tail)

    def apply_value(match: "re.Match[str]") -> str:
        tag, reference = match.group(1), match.group(2)
        if not absent(reference):
            return match.group(0)
        target = redirect(reference, object_kind(reference))
        return f"<{tag}>{target}</{tag}>" if target else f"<{tag}/>"

    changed = 0
    for path in sorted(out.rglob("*.xml")):
        text = path.read_text(encoding="utf-8-sig")
        new = command.sub(lambda m: "" if absent(m.group(1)) else m.group(0), text)
        for pattern, kind_of, is_absent in rules:
            new = pattern.sub(lambda m: apply_rule(m, kind_of, is_absent), new)
        new = value.sub(apply_value, new)
        if new != text:
            path.write_text(new, encoding="utf-8-sig")
            changed += 1
    return changed


def handler_stubs(out: Path) -> int:
    """Give a common module the export procedure a handler names but it lacks."""
    changed = 0
    for tag, parameters in (("Handler", "Source, Cancel"), ("MethodName", "")):
        for path in sorted(out.rglob("*.xml")):
            for module, procedure in re.findall(
                rf"<{tag}>CommonModule\.(\w+)\.(\w+)</{tag}>", path.read_text(encoding="utf-8-sig")
            ):
                bsl = out / "CommonModules" / module / "Ext" / "Module.bsl"
                if not bsl.is_file():
                    continue
                text = bsl.read_text(encoding="utf-8-sig")
                if re.search(rf"^\s*(Procedure|Процедура)\s+{procedure}\s*\(", text, re.M | re.I):
                    continue
                eol = "\r\n" if "\r\n" in text else "\n"
                text = text.rstrip() + f"{eol}{eol}Procedure {procedure}({parameters}) Export{eol}EndProcedure{eol}"
                bsl.write_text(text, encoding="utf-8-sig")
                changed += 1
    return changed


# Filled from platform loads: every entry names the rejection it removes.
def drop_support(out: Path) -> int:
    """Remove the support description copied from the base."""
    path = out / "Ext" / "ParentConfigurations.bin"
    if not path.exists():
        return 0
    path.unlink()
    return 1


REPAIRS: Sequence[Repair] = (
    Repair(
        "Ext/ParentConfigurations.bin of the base is a made-up support description: "
        "ibcmd import stops with \"Ошибка формата потока\"; the configuration is "
        "assembled without support",
        drop_support,
    ),
    Repair(
        "references to objects absent from the assembly: ibcmd import stops with "
        "\"Неизвестный объект метаданных\" and \"Неизвестное имя типа\"; a reference "
        "to a whole object is redirected to a present object of its kind, a reference into "
        "an absent object and a command interface entry naming one are removed",
        retarget_absent,
    ),
    Repair(
        "a handler names a procedure its common module lacks: ibcmd import stops with "
        "\"Ссылка на неизвестный метод\"; an empty export procedure is added",
        handler_stubs,
    ),
)


def object_name(path: Path) -> str:
    match = re.search(rb"<Properties>.*?<Name>([^<]+)</Name>", path.read_bytes(), re.S)
    if not match:
        raise SystemExit(f"{path}: descriptor without <Name>")
    return match.group(1).decode("utf-8")


def fresh_uuids(paths: Sequence[Path], salt: str) -> None:
    """Give every identifier in ``paths`` a new deterministic value.

    One map serves all files of an object, so references between the
    descriptor and its forms, templates and modules stay consistent.
    """
    mapping: Dict[bytes, bytes] = {}

    def replace(match: "re.Match[bytes]") -> bytes:
        old = match.group(0)
        if old not in mapping:
            mapping[old] = str(uuid.uuid5(uuid.NAMESPACE_URL, f"unica-all-kinds|{salt}|{old.decode()}")).encode()
        return mapping[old]

    for path in paths:
        if path.suffix.lower() in (".xml", ".bsl", ".txt", ".html"):
            data = path.read_bytes()
            path.write_bytes(UUID.sub(replace, data))


def register(configuration: Path, kind: str, name: str) -> None:
    data = configuration.read_bytes()
    entry = f"<{kind}>{name}</{kind}>".encode("utf-8")
    if entry in data:
        return
    data, count = re.subn(rb"</ChildObjects>", entry + b"</ChildObjects>", data, count=1)
    if count != 1:
        raise SystemExit(f"{configuration}: no </ChildObjects>")
    configuration.write_bytes(data)


def add(out: Path, source: Source) -> str:
    descriptor = FIXTURES / source.path
    name = object_name(descriptor)
    target_dir = out / source.folder
    target_dir.mkdir(exist_ok=True)
    target = target_dir / f"{name}.xml"
    if target.exists():
        raise SystemExit(f"{source.kind}.{name} already present in the base")
    shutil.copyfile(descriptor, target)
    copied = [target]
    nested = descriptor.with_suffix("")
    if nested.is_dir():
        shutil.copytree(nested, target_dir / name)
        copied += [p for p in (target_dir / name).rglob("*") if p.is_file()]
    fresh_uuids(copied, f"{source.kind}.{name}")
    register(out / "Configuration.xml", source.kind, name)
    return name


def build(out: Path) -> List[str]:
    if out.exists():
        raise SystemExit(f"{out} exists; pass a new directory")
    shutil.copytree(BASE, out)
    report = [f"base: {BASE.relative_to(REPO).as_posix()}"]
    configuration = out / "Configuration.xml"
    for kind, folder, name in BASE_DROPPED:
        (out / folder / f"{name}.xml").unlink()
        shutil.rmtree(out / folder / name, ignore_errors=True)
        data = configuration.read_bytes()
        entry = f"<{kind}>{name}</{kind}>".encode("utf-8")
        if data.count(entry) != 1:
            raise SystemExit(f"base has no single {kind}.{name}")
        configuration.write_bytes(data.replace(entry, b""))
        report.append(f"base without {kind}.{name}")
    for source in SOURCES:
        name = add(out, source)
        report.append(f"{source.kind}.{name}: {source.path}")
    for repair in REPAIRS:
        report.append(f"repair ({repair.apply(out)}): {repair.why}")
    return report


def main(argv: Sequence[str]) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--out", type=Path, help="new directory for the assembled sources")
    parser.add_argument("--list", action="store_true", help="print sources and repairs, write nothing")
    args = parser.parse_args(argv)
    if args.list:
        for source in SOURCES:
            print(f"{source.kind}: {source.path}")
        for repair in REPAIRS:
            print(f"repair: {repair.why}")
        return 0
    if args.out is None:
        parser.error("--out is required")
    print("\n".join(build(args.out.resolve())))
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
