#!/usr/bin/env python3
"""Observe real FormItems.Move behavior on 1C 8.3.27; never edit a user infobase.

Requires an ibcmd and a GUI client from the same installation. The new, empty
--work-dir contains a disposable infobase, generated sources, logs and results.
XML import alone is not an oracle: 8.3.27.2214 accepts incompatible child kinds.
This harness executes Move on the server while creating real managed forms,
and records actual parent, kind and binding rather than relying on no exception.
"""
from __future__ import annotations

import argparse
import hashlib
import importlib.util
import json
import re
import shutil
import subprocess
import sys
import uuid
from pathlib import Path
import xml.etree.ElementTree as ET

LOGFORM = "http://v8.1c.ru/8.3/xcf/logform"
CORE = "http://v8.1c.ru/8.1/data/core"
PARENTS = ["Form", "UsualGroup", "Page", "Pages", "Table", "ColumnGroup",
           "CommandBar", "AutoCommandBar", "ButtonGroup", "Popup", "ContextMenu"]
CHILDREN = ["InputField", "LabelField", "CheckBoxField", "PictureField",
            "HTMLDocumentField", "UsualGroup", "ColumnGroup", "Page", "Pages",
            "Table", "Button", "ButtonGroup", "Popup", "CommandBar", "LabelDecoration"]
COMMAND_PARENTS = {"CommandBar", "AutoCommandBar", "ButtonGroup", "Popup", "ContextMenu"}
SPECIAL = {
    "CalendarField": ("DateValue", "xs:dateTime", None),
    "RadioButtonField": ("NumberValue", "xs:decimal", None),
    "ProgressBarField": ("NumberValue", "xs:decimal", None),
    "TrackBarField": ("NumberValue", "xs:decimal", None),
    "SpreadSheetDocumentField": ("SheetValue", "special:SpreadsheetDocument", "http://v8.1c.ru/8.2/data/spreadsheet"),
    "FormattedDocumentField": ("FormattedValue", "special:FormattedDocument", "http://v8.1c.ru/8.2/data/formatted-document"),
    "TextDocumentField": ("TextDocValue", "special:TextDocument", "http://v8.1c.ru/8.1/data/txtedt"),
    "ChartField": ("ChartValue", "special:Chart", "http://v8.1c.ru/8.2/data/chart"),
    "GanttChartField": ("GanttValue", "special:GanttChart", "http://v8.1c.ru/8.2/data/chart"),
    "GraphicalSchemaField": ("GraphValue", "special:FlowchartContextType", "http://v8.1c.ru/8.2/data/graphscheme"),
    "PlannerField": ("PlannerValue", "special:Planner", "http://v8.1c.ru/8.3/data/planner"),
    "PeriodField": ("PeriodValue", "v8:StandardPeriod", None),
    "DendrogramField": ("DendrogramValue", "special:Dendrogram", "http://v8.1c.ru/8.2/data/chart"),
    "GeographicalSchemaField": ("GeoValue", "special:GeographicalSchema", "http://v8.1c.ru/8.2/data/geo"),
    "PDFDocumentField": ("PdfValue", "special:PDFDocument", "http://v8.1c.ru/8.3/data/pdf"),
}
CHILDREN += ["PictureDecoration", *SPECIAL]
FIELDS = {"InputField", "LabelField", "CheckBoxField", "PictureField", "HTMLDocumentField", *SPECIAL}


def cases() -> list[dict]:
    result = []
    def add(parent, child, **extra):
        result.append({"form": f"Probe{len(result):03}", "parent": parent,
                       "child": child, "source": "natural", "targetRows": "Rows",
                       "buttonType": "CommandBarButton" if parent in COMMAND_PARENTS else "UsualButton",
                       "nestedColumn": False, **extra})
    for parent in PARENTS:
        for child in CHILDREN:
            add(parent, child)
    for parent in PARENTS:
        for button_type in ["UsualButton", "Hyperlink", "CommandBarButton", "CommandBarHyperlink", None]:
            add(parent, "Button", buttonType=button_type)
    for child in ["InputField", "LabelField", "CheckBoxField", "PictureField"]:
        for parent in ["Form", "UsualGroup", "Page", "Table", "ColumnGroup"]:
            for source, target_rows in [("column", "Rows"), ("column", "OtherRows"),
                                        ("currentData", "Rows")]:
                add(parent, child, source=source, targetRows=target_rows)
    for child in ["HTMLDocumentField", *SPECIAL]:
        for parent in ["Form", "Table", "ColumnGroup"]:
            add(parent, child, source="column")
    for parent in ["Form", "UsualGroup", "Table", "ColumnGroup"]:
        for target_rows in ["Rows", "OtherRows"]:
            for nested in [False, True]:
                add(parent, "ColumnGroup", targetRows=target_rows, nestedColumn=nested)
    # A leaf cannot acquire children merely because its XML can contain ChildItems.
    for parent in ["InputField", "LabelField", "CheckBoxField", "HTMLDocumentField", "LabelDecoration", "Button"]:
        add(parent, "UsualGroup")
    for parent in PARENTS:
        add(parent, "Button", buttonType=None, source="command")
    for parent in ["Table", "ColumnGroup"]:
        for source in ["unboundColumn", "blankColumn"]:
            add(parent, "InputField", source=source, targetRows="OtherRows")
        add(parent, "ColumnGroup", targetRows="OtherRows", nestedEmpty=True)
        add(parent, "ColumnGroup", targetRows="OtherRows", nestedUnbound=True)
    return result


def form_source(case: dict) -> tuple[bytes, str]:
    sequence = 0
    def node(tag, name):
        nonlocal sequence
        sequence += 1
        item = ET.Element(tag, {"name": name, "id": str(sequence)})
        if tag == "Button":
            if case["buttonType"] is not None:
                ET.SubElement(item, "Type").text = case["buttonType"]
            ET.SubElement(item, "CommandName").text = "Form.Command.Test"
        if tag in FIELDS:
            binding = SPECIAL[tag][0] if tag in SPECIAL else ("Flag" if tag == "CheckBoxField" else "Text")
            if name == "MovedChild" and case["source"] in {"column", "currentData"}:
                column = "Flag" if tag == "CheckBoxField" else "Value"
                binding = f"Rows.{column}" if case["source"] == "column" else f"Items.SourceTable.CurrentData.{column}"
            if name == "MovedChild" and case["source"] == "blankColumn":
                binding = ""
            if not (name == "MovedChild" and case["source"] == "unboundColumn"):
                ET.SubElement(item, "DataPath").text = binding
        if tag == "Table":
            ET.SubElement(item, "DataPath").text = case["targetRows"] if name.startswith("TargetParent") else "Rows"
        return item

    root = ET.Element("Form", {"xmlns": LOGFORM, "xmlns:v8": CORE,
                              "xmlns:xs": "http://www.w3.org/2001/XMLSchema", "version": "2.20"})
    bar = ET.SubElement(root, "AutoCommandBar", {"name": "MainBar", "id": "-1"})
    ET.SubElement(bar, "Autofill").text = "false"
    events = ET.SubElement(root, "Events")
    ET.SubElement(events, "Event", {"name": "OnCreateAtServer"}).text = "RunMove"
    section = ET.SubElement(root, "ChildItems")

    def natural(tag, name):
        if name == "MovedChild" and case["source"] in {"column", "blankColumn", "unboundColumn"}:
            owner = node("Table", "SourceTable")
            section.append(owner)
            destination = ET.SubElement(owner, "ChildItems")
        elif tag in {"Page", "ColumnGroup"}:
            owner = node("Pages" if tag == "Page" else "Table", name + "Owner")
            section.append(owner)
            destination = ET.SubElement(owner, "ChildItems")
        elif tag in {"ButtonGroup", "Popup"} or (name == "MovedChild" and case["source"] == "command"):
            destination = bar.find("ChildItems")
            if destination is None:
                destination = ET.SubElement(bar, "ChildItems")
        elif tag == "ContextMenu":
            destination = node("InputField", name + "Owner")
            section.append(destination)
        elif tag == "AutoCommandBar":
            return bar
        else:
            destination = section
        item = node(tag, name)
        destination.append(item)
        return item

    if case["source"] == "currentData":
        section.append(node("Table", "SourceTable"))
    moved = natural(case["child"], "MovedChild")
    if case["nestedColumn"]:
        nested = node("InputField", "NestedField")
        nested.find("DataPath").text = "Rows.Value"
        ET.SubElement(moved, "ChildItems").append(nested)
    if case.get("nestedEmpty") or case.get("nestedUnbound"):
        nested = node("ColumnGroup" if case.get("nestedEmpty") else "InputField", "NestedField")
        if nested.find("DataPath") is not None:
            nested.remove(nested.find("DataPath"))
        ET.SubElement(moved, "ChildItems").append(nested)
    if case["parent"] == "Form":
        destination = "ЭтаФорма"
    else:
        natural(case["parent"], "TargetParent")
        destination = {"AutoCommandBar": "КоманднаяПанель",
                       "ContextMenu": "Элементы.TargetParentOwner.КонтекстноеМеню"}.get(
                           case["parent"], "Элементы.TargetParent")
    attributes = ET.SubElement(root, "Attributes")
    for index, (name, kind) in enumerate([("Text", "xs:string"), ("Flag", "xs:boolean"),
                                        ("Rows", "v8:ValueTable"), ("OtherRows", "v8:ValueTable"),
                                        ("ProbeResult", "xs:string")], 1):
        attribute = ET.SubElement(attributes, "Attribute", {"name": name, "id": str(index)})
        ET.SubElement(ET.SubElement(attribute, "Type"), "v8:Type").text = kind
        if name in {"Rows", "OtherRows"}:
            columns = ET.SubElement(attribute, "Columns")
            for column_index, (column_name, column_type) in enumerate([("Value", "xs:string"), ("Flag", "xs:boolean")], 1):
                column = ET.SubElement(columns, "Column", {"name": column_name, "id": str(column_index)})
                column_type_node = ET.SubElement(ET.SubElement(column, "Type"), "v8:Type")
                if column_name == "Value" and case["child"] in SPECIAL:
                    _, column_type, column_namespace = SPECIAL[case["child"]]
                    if column_namespace is not None:
                        column_type_node.set("xmlns:special", column_namespace)
                column_type_node.text = column_type
    special = SPECIAL.get(case["child"])
    if special is not None:
        attr_name, attr_type, namespace = special
        attribute = ET.SubElement(attributes, "Attribute", {"name": attr_name, "id": "20"})
        type_node = ET.SubElement(ET.SubElement(attribute, "Type"), "v8:Type")
        if namespace is not None:
            type_node.set("xmlns:special", namespace)
        type_node.text = attr_type
    ET.SubElement(root, "Parameters")
    command = ET.SubElement(ET.SubElement(root, "Commands"), "Command", {"name": "Test", "id": "1"})
    ET.SubElement(command, "Action").text = "TestAction"
    path_expression = "Элементы.NestedField.ПутьКДанным" if case["nestedColumn"] else "Э.ПутьКДанным"
    module = f'''&НаСервере
Процедура RunMove(Отказ, СтандартнаяОбработка)
Попытка
Э = Элементы.MovedChild;
До = Строка(ТипЗнч(Э));
Попытка До = До + ":" + Строка(Э.Вид); Исключение КонецПопытки;
РодДо = Э.Родитель;
ПутьДо = "";
Попытка ПутьДо = {path_expression}; Исключение КонецПопытки;
Элементы.Переместить(Э, {destination});
После = Строка(ТипЗнч(Э));
Попытка После = После + ":" + Строка(Э.Вид); Исключение КонецПопытки;
ПутьПосле = "";
Попытка ПутьПосле = {path_expression}; Исключение КонецПопытки;
Род = "";
Попытка Род = Э.Родитель.Имя; Исключение Род = Строка(ТипЗнч(Э.Родитель)); КонецПопытки;
ProbeResult = "OK" + Символы.Таб + До + Символы.Таб + После + Символы.Таб
 + Строка(Э.Родитель = {destination}) + Символы.Таб + Род + Символы.Таб
 + ПутьДо + Символы.Таб + ПутьПосле;
Исключение
ProbeResult = "ERROR" + Символы.Таб + СтрЗаменить(ОписаниеОшибки(), Символы.ПС, " ");
КонецПопытки;
КонецПроцедуры
&НаКлиенте
Процедура TestAction(Команда)
КонецПроцедуры
'''
    return ET.tostring(root, encoding="utf-8", xml_declaration=True), module



def compact_report(report: dict) -> dict:
    """Deduplicate literal XML fragments without changing observed outcomes."""
    fragments = {}
    compact_cases = []
    for case in report["cases"]:
        case = dict(case)
        xml = case.pop("sourceXml")
        # Split only between adjacent tags; values and whitespace stay verbatim.
        parts = re.split(r"(?<=>)(?=<)", xml)
        case["sourceParts"] = [fragments.setdefault(part, len(fragments)) for part in parts]
        compact_cases.append(case)
    result = {key: value for key, value in report.items()
              if key not in {"ibcmdSha256", "clientSha256", "installSha256", "installFileCount"}}
    result.update(schemaVersion=2, sourceXmlFragments=list(fragments), cases=compact_cases)
    return result


def report_json(report: dict) -> str:
    text = json.dumps(report, ensure_ascii=False, indent=2)
    # Keep each sourceParts index list on one line, while retaining readable cases.
    return re.sub(r'("sourceParts": )(\[\s*\d+(?:,\s*\d+)*\s*\])',
                  lambda match: match[1] + json.dumps(json.loads(match[2])), text) + "\n"


def check_fixture(path: Path) -> None:
    report = json.loads(path.read_text())
    if report["schemaVersion"] != 2:
        raise ValueError("expected compact fixture schemaVersion 2")
    for case in report["cases"]:
        restored = "".join(report["sourceXmlFragments"][part] for part in case["sourceParts"])
        generated = form_source(case)[0].decode("utf-8")
        if restored != generated:
            raise ValueError(f"source XML differs for {case['form']}")
    print(f"exact source XML reconstruction: {len(report['cases'])} cases")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check-fixture", type=Path, help="verify compact inputs without running 1C")
    parser.add_argument("--ibcmd", type=Path)
    parser.add_argument("--client", type=Path)
    parser.add_argument("--work-dir", type=Path)
    args = parser.parse_args()
    if args.check_fixture is not None:
        check_fixture(args.check_fixture)
        return
    if any(value is None for value in (args.ibcmd, args.client, args.work_dir)):
        parser.error("--ibcmd, --client and --work-dir are required for a platform probe")
    work = args.work_dir.resolve()
    work.mkdir(parents=True, exist_ok=True)
    if any(work.iterdir()):
        parser.error("work-dir must be empty; no existing infobase is reused")
    for executable in [args.ibcmd, args.client]:
        if not executable.is_absolute() or not executable.is_file():
            parser.error("executables must be absolute existing files")
    sys.path.insert(0, str(Path(__file__).resolve().parents[2]))
    verifier_path = Path(__file__).resolve().parents[1] / "dev" / "verify-8-3-27-platform.py"
    spec = importlib.util.spec_from_file_location("form_move_platform_verifier", verifier_path)
    verifier = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = verifier
    spec.loader.exec_module(verifier)
    inventory = verifier.capture_platform_install_inventory(args.ibcmd)
    binary_hashes = {str(p): hashlib.sha256(p.read_bytes()).hexdigest() for p in [args.ibcmd, args.client]}
    commands = []
    def run(stage, command, timeout=120):
        for path, digest in binary_hashes.items():
            verifier.require_executable_identity(Path(path), digest)
        record = subprocess.run(command, capture_output=True, timeout=timeout)
        (work / f"{stage}.stdout").write_bytes(record.stdout)
        (work / f"{stage}.stderr").write_bytes(record.stderr)
        commands.append({"stage": stage, "argv": list(map(str, command)), "exitCode": record.returncode})
        print(stage, record.returncode, flush=True)
        if record.returncode:
            raise RuntimeError(record.stderr.decode(errors="replace"))
        return record
    version = run("version", [str(args.ibcmd), "--version"]).stdout.decode().strip()
    if re.fullmatch(r"8\.3\.27\.[0-9]+", version) is None:
        raise RuntimeError(f"unsupported platform version {version!r}")
    ib = work / "ib"
    options = [f"--{key}={ib / value}" for key, value in [
        ("db-path", "db"), ("data", "data"), ("temp", "temp"),
        ("users-data", "users"), ("session-data", "session"), ("log-data", "log")]]
    run("create", [str(args.ibcmd), "infobase", "create", *options])
    source = work / "source"
    run("baseline-export", [str(args.ibcmd), "config", *options, "export", str(source)])
    matrix = cases()
    references = []
    for case in matrix:
        name = case["form"]
        references.append(f"<CommonForm>{name}</CommonForm>")
        form_dir = source / "CommonForms" / name / "Ext"
        (form_dir / "Form").mkdir(parents=True)
        descriptor = f'<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><CommonForm uuid="{uuid.uuid5(uuid.NAMESPACE_URL, name)}"><Properties><Name>{name}</Name><FormType>Managed</FormType></Properties></CommonForm></MetaDataObject>'
        (source / "CommonForms" / f"{name}.xml").write_text(descriptor)
        xml, module = form_source(case)
        (form_dir / "Form.xml").write_bytes(xml)
        (form_dir / "Form" / "Module.bsl").write_text(module)
    configuration = source / "Configuration.xml"
    text = configuration.read_text(encoding="utf-8-sig")
    configuration.write_text(text.replace("<ChildObjects>", "<ChildObjects>" + "".join(references), 1))
    output = work / "runtime.tsv"
    lines = ["Процедура ПриНачалеРаботыСистемы()", f'Результат = Новый ЗаписьТекста("{output}", КодировкаТекста.UTF8);']
    for case in matrix:
        name = case["form"]
        lines.extend(["Попытка", f'Ф = ПолучитьФорму("ОбщаяФорма.{name}");',
                      f'Результат.ЗаписатьСтроку("{name}" + Символы.Таб + Ф.ProbeResult);',
                      "Исключение", f'Результат.ЗаписатьСтроку("{name}" + Символы.Таб + "CREATEERROR" + Символы.Таб + ОписаниеОшибки());', "КонецПопытки;"])
    lines.extend(["Результат.Закрыть();", "ЗавершитьРаботуСистемы(Ложь);", "КонецПроцедуры"])
    (source / "Ext").mkdir(exist_ok=True)
    (source / "Ext" / "ManagedApplicationModule.bsl").write_text("\n".join(lines))
    run("import", [str(args.ibcmd), "config", *options, "import", str(source)])
    run("check", [str(args.ibcmd), "config", *options, "check", "--force"])
    run("apply", [str(args.ibcmd), "config", *options, "apply", "--force"])
    run("runtime", [str(args.client), "ENTERPRISE", "/F", str(ib / "db"), "/DisableStartupMessages", "/DisableStartupDialogs", "/Out", str(work / "client.log")])
    observations = {}
    for line in output.read_text(encoding="utf-8-sig").splitlines():
        parts = line.split("\t")
        if len(parts) < 2 or parts[0] in observations:
            raise RuntimeError(f"malformed/duplicate result: {line!r}")
        if parts[1] != "OK":
            observations[parts[0]] = {"status": parts[1], "detail": "\t".join(parts[2:])}
        else:
            _, _, before, after, at_target, parent, path_before, path_after = parts
            observations[parts[0]] = {"status": "observed", "kindBefore": before, "kindAfter": after,
                                      "atTarget": at_target == "Да", "parentAfter": parent,
                                      "pathBefore": path_before, "pathAfter": path_after,
                                      "preserves": at_target == "Да" and before == after and path_before == path_after}
    if set(observations) != {case["form"] for case in matrix}:
        raise RuntimeError("missing or unexpected runtime cases")
    # Build literal XML post-images from the platform-normalized baseline.
    # Only runtime-observed preserving cases are moved; refused/converting cases
    # are retained as controls. This is independent from Unica's implementation.
    canonical = work / "canonical"
    run("canonical-export", [str(args.ibcmd), "config", *options, "export", str(canonical)])
    expected = {}
    for case in matrix:
        observation = observations[case["form"]]
        if not observation.get("preserves"):
            continue
        form_path = canonical / "CommonForms" / case["form"] / "Ext" / "Form.xml"
        document = verifier.etree.parse(str(form_path))
        root = document.getroot()
        moved = root.xpath(".//*[@name='MovedChild']")[0]
        if case["parent"] == "Form":
            parent = root
        else:
            name = "MainBar" if case["parent"] == "AutoCommandBar" else "TargetParent"
            parent = root.xpath(".//*[@name=$name]", name=name)[0]
        section = parent.find("{" + LOGFORM + "}ChildItems")
        if section is None:
            section = verifier.etree.SubElement(parent, "{" + LOGFORM + "}ChildItems")
        expected_semantic = verifier._semantic_element(moved, case["form"])
        section.append(moved)
        expected[case["form"]] = expected_semantic
        form_path.write_bytes(verifier.etree.tostring(document, encoding="utf-8", xml_declaration=True))
    roundtrip = []
    current = canonical
    for cycle in [1, 2]:
        exported = work / f"postmove-export-{cycle}"
        run(f"postmove-import-{cycle}", [str(args.ibcmd), "config", *options, "import", str(current)])
        run(f"postmove-export-{cycle}", [str(args.ibcmd), "config", *options, "export", str(exported)])
        for case in matrix:
            if case["form"] not in expected:
                continue
            path = exported / "CommonForms" / case["form"] / "Ext" / "Form.xml"
            document = verifier.etree.parse(str(path))
            matches = document.getroot().xpath(".//*[@name='MovedChild']")
            actual = verifier._semantic_element(matches[0], case["form"]) if len(matches) == 1 else None
            expected_parent = "MainBar" if case["parent"] == "AutoCommandBar" else "TargetParent"
            actual_parent = matches[0].getparent().getparent() if len(matches) == 1 else None
            parent_ok = actual_parent is not None and (actual_parent == document.getroot() if case["parent"] == "Form" else actual_parent.get("name") == expected_parent)
            same = actual == expected[case["form"]] and parent_ok
            roundtrip.append({"form": case["form"], "cycle": cycle, "subtreeAndParentPreserved": same})
            if not same:
                raise RuntimeError(f"post-move subtree changed: {case['form']} cycle {cycle}")
        current = exported
    after = verifier.capture_platform_install_inventory(args.ibcmd)
    if (after["sha256"], after["fileCount"]) != (inventory["sha256"], inventory["fileCount"]):
        raise RuntimeError("platform installation changed during probe")
    report = {"schemaVersion": 1, "platformVersion": version, "ibcmdSha256": binary_hashes[str(args.ibcmd)],
              "clientSha256": binary_hashes[str(args.client)], "installSha256": inventory["sha256"],
              "installFileCount": inventory["fileCount"], "method": "FormItems.Move in OnCreateAtServer of real managed CommonForms",
              "postMoveRoundtrip": {"cycles": 2, "caseCount": len(expected), "allSubtreesAndParentsPreserved": all(row["subtreeAndParentPreserved"] for row in roundtrip)},
              "harnessSha256": hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),
              "cases": [{**case, **observations[case["form"]], "sourceXml": (source / "CommonForms" / case["form"] / "Ext" / "Form.xml").read_text()} for case in matrix]}
    for case in report["cases"]:
        if case["status"] == "observed":
            case["outcome"] = "preserving" if case["preserves"] else ("refused" if not case["atTarget"] else "converted")
        else:
            case["outcome"] = "sourceNotInstantiated" if "Поле объекта не обнаружено (MovedChild)" in case.get("detail", "") else "rejected"
    report["limitations"] = [
        "FormItems.Move observations cover named tag/binding cases, not every legal property combination.",
        "A sourceNotInstantiated outcome is inconclusive for XML-only edits; import/check/export preserve these unbound fields but runtime does not instantiate them.",
        "Raw post-move checks compare platform-normalized subtrees with two subsequent imports/exports; they do not claim original source XML byte equality.",
    ]
    digests = []
    representatives = {}
    for case in report["cases"]:
        if not case.get("preserves"):
            continue
        form_path = current / "CommonForms" / case["form"] / "Ext" / "Form.xml"
        digests.append({"form": case["form"], "sha256": hashlib.sha256(form_path.read_bytes()).hexdigest()})
        representatives.setdefault(case["parent"], case)
    for tag in ["HTMLDocumentField", "PDFDocumentField", "ChartField", "SpreadSheetDocumentField"]:
        representatives["table-" + tag] = next(case for case in report["cases"] if case["child"] == tag and case["parent"] == "Table" and case.get("preserves"))
    representatives["nonempty-column-group"] = next(case for case in report["cases"] if case["child"] == "ColumnGroup" and case["parent"] == "ColumnGroup" and case["nestedColumn"] and case.get("preserves"))
    (work / "postmove").mkdir()
    for name, case in representatives.items():
        shutil.copy2(current / "CommonForms" / case["form"] / "Ext" / "Form.xml", work / "postmove" / f"{name}.xml")
    report["postMoveRoundtrip"]["exportedFormDigests"] = digests
    report["postMoveRoundtrip"]["representatives"] = [{"form": case["form"], "path": f"postmove/{name}.xml"} for name, case in representatives.items()]
    (work / "observations.json").write_text(report_json(compact_report(report)))
    (work / "commands.json").write_text(json.dumps(commands, ensure_ascii=False, indent=2) + "\n")
    print(f"observed {len(matrix)} cases; report: {work / 'observations.json'}")


if __name__ == "__main__":
    main()
