"""Properties acceptance on the shared packet, recorder and Codex review path."""
from __future__ import annotations

import hashlib
import json
import xml.etree.ElementTree as ET
from pathlib import Path

from tests.agent_evaluation.dcs_driver import (
    REPO, changed_files, evaluate as evaluate_contract, independent_review as review_contract,
    terminal_calls,
)
from tests.agent_evaluation.mcp_recorder import source_file_hashes
from tests.ci.acceptance_controls import read_file

FIXTURE = REPO / "tests/fixtures/acceptance/agent-properties"
SOURCE_FIXTURE = REPO / "tests/fixtures/acceptance/workspace"
MODULE = "main:CommonModule.GoogleПереводчик"
ROLE = "main:Role.SalesReader"
SUBSYSTEM = "main:Subsystem.Администрирование"
PATHS = {MODULE: "src/CommonModules/GoogleПереводчик.xml", ROLE: "src/Roles/SalesReader.xml",
         SUBSYSTEM: "src/Subsystems/Администрирование.xml"}
TARGET_VALUES = {
    MODULE: {"Comment": "Agent1721 module", "ClientOrdinaryApplication": False},
    ROLE: {"Synonym": "Agent1721 role", "Comment": "Agent1721 role comment"},
    SUBSYSTEM: {"Synonym": "Agent1721 subsystem", "Comment": "Agent1721 subsystem comment",
                "Explanation": "", "IncludeInCommandInterface": False},
}
ALIASES = {"synonym": "Synonym", "comment": "Comment", "explanation": "Explanation",
           "includeInCommandInterface": "IncludeInCommandInterface"}
MD = "{http://v8.1c.ru/8.3/MDClasses}"
CORE = "{http://v8.1c.ru/8.1/data/core}"
RUBRIC = (
    "Агент нашёл существующие CommonModule.GoogleПереводчик, Role.SalesReader и "
    "Subsystem.Администрирование, прочитал подробный contextual Can.props.set каждого "
    "до preview. У модуля Comment=Agent1721 module и ClientOrdinaryApplication=false. "
    "У роли Synonym=Agent1721 role и Comment=Agent1721 role comment; в одном patch "
    "использованы synonym и Comment. У подсистемы Synonym=Agent1721 subsystem, "
    "Comment=Agent1721 subsystem comment, Explanation очищено пустой строкой, "
    "IncludeInCommandInterface=false; один patch содержит Synonym/comment/explanation. "
    "Каждая публикация потребляет наблюдённый preview с тем же executionToken; "
    "preview, отказы и чтение не меняют исходники. Каждый изменённый узел перечитан "
    "и проверен после собственной последней публикации. Check роли/подсистемы passed; "
    "check общего модуля может честно сообщить provider_unavailable/provider_absent отсутствующего "
    "анализатора. Такой ответ не назван успешно выполненным анализом кода. "
    "Физические fileObservations независимо доказывают три полных descriptor postimage, "
    "BOM/EOL, UUID/Name, Rights/Content/ChildObjects/CommandInterface и все соседние "
    "исходники. В исходных трёх descriptors только ru; сохранение других переводов "
    "этим агентным сценарием не доказывается. Агент не выдаёт физическое чтение "
    "драйвера за собственное чтение XML. Ответ верно объясняет, что values не может "
    "быть пустым, boolean требует JSON false вместо строки, обе формы одного alias "
    "не допускаются, но synonym+Comment относятся к разным свойствам. Проверка "
    "опубликованного Can-каталога Configuration в ответе узла или по адресу .Can "
    "через Unica подтверждает отсутствие props.set у корня; "
    "агент не обещает поддержку Configuration. Нет ручной записи XML и незапрошенных "
    "изменений; нет заявления о загрузке в 1С без протокола такого запуска."
)


def recorded_calls(records):
    """Bind each public tool response to its recorder source observations."""
    requests, calls, allowed = {}, [], None
    for index, record in enumerate(records):
        payload = record["payload"]
        if record["direction"] == "request":
            if "id" in payload:
                requests[payload["id"]] = (index, record)
            continue
        request = requests.get(payload.get("id"))
        if request is None:
            continue
        start, before = request
        message = before["payload"]
        if message["method"] == "tools/list":
            allowed = {tool["name"] for tool in payload["result"]["tools"]}
        if message["method"] == "tools/call":
            result = payload.get("result") or {}
            calls.append({"index": start, "done": index, "params": message["params"],
                          "result": result.get("structuredContent"),
                          "isError": "error" in payload or result.get("isError") is True,
                          "before": before["sourceSha256"], "after": record["sourceSha256"],
                          "filesBefore": before.get("sourceFilesSha256"),
                          "filesAfter": record.get("sourceFilesSha256"),
                          "rootsBefore": before.get("sourceSetRoots"),
                          "rootsAfter": record.get("sourceSetRoots")})
    if not allowed:
        raise ValueError("Properties evaluation omitted the actual MCP inventory")
    return allowed, sorted(calls, key=lambda call: call["index"])


def audit(records):
    allowed, raw_calls = recorded_calls(records)
    calls = terminal_calls(raw_calls)
    contracts, plans, published = {}, {}, {}
    mixed_role = mixed_subsystem = discovery = configuration = False
    for call in calls:
        tool, args = call["params"]["name"], call["params"]["arguments"]
        result = call["result"] or {}
        data = result.get("data") or {}
        if result.get("ok") is True and call["isError"]:
            raise ValueError("successful Properties observation is flagged as an MCP error")
        if tool not in allowed or tool not in {"unica.view", "unica.search", "unica.check", "unica.apply", "unica.diff"}:
            raise ValueError("Properties call is outside its actual MCP inventory")
        before, after = call.get("filesBefore"), call.get("filesAfter")
        if not isinstance(before, dict) or not isinstance(after, dict):
            raise ValueError("Properties evaluation requires complete per-file recorder evidence")
        wrote = before != after or call["before"] != call["after"]
        if tool != "unica.apply":
            if wrote:
                raise ValueError("read-only Properties observation changed source bytes")
            if tool != "unica.view":
                continue
            at = args.get("at")
            if args == {} and result.get("ok") is True:
                discovery = True
            if at == "main:Configuration" and result.get("ok") is True:
                capabilities = data.get("can")
                if (result.get("at") == at and data.get("at") == at and data.get("kind") == "Configuration"
                        and isinstance(capabilities, list) and capabilities
                        and all(isinstance(item, dict) and isinstance(item.get("op"), str) and item["op"]
                                and item["op"] == item["op"].strip()
                                and isinstance(item.get("args"), str) and item["args"]
                                and type(item.get("implemented")) is bool for item in capabilities)):
                    operations = [item["op"] for item in capabilities]
                    if len(set(operations)) == len(operations) and "props.set" not in operations:
                        configuration = True
            if at in {"main:Configuration.Can", "main:Configuration.Can.props.set"}:
                if at.endswith(".Can") and result.get("ok") is True:
                    items = data.get("items")
                    if isinstance(items, list) and not any(item.get("props", {}).get("op") == "props.set"
                                                          or str(item.get("at", "")).endswith(".Can.props.set")
                                                          for item in items):
                        configuration = True
                elif (at.endswith(".Can.props.set") and result.get("ok") is False
                      and any(item.get("code") in {"not_found", "bad_value"} for item in result.get("diagnostics", []))):
                    configuration = True
            if result.get("ok") is True and isinstance(at, str):
                target = at.removesuffix(".Can.props.set")
                props = data.get("props") or {}
                schema = (props.get("contract") or {}).get("argsSchema") or {}
                if (target in PATHS and at == target + ".Can.props.set" and data.get("at") == at
                        and result.get("at") == at and data.get("kind") == "Can"
                        and props.get("op") == "props.set" and props.get("implemented") is True
                        and schema.get("type") == "object" and schema.get("additionalProperties") is False):
                    contracts[target] = min(contracts.get(target, call["done"]), call["done"])
            continue
        if result.get("ok") is not True:
            if wrote or data.get("executionToken"):
                raise ValueError("refused Properties apply changed source or minted a token")
            continue
        if call["rootsBefore"] != {"main": "src"} or call["rootsAfter"] != {"main": "src"}:
            raise ValueError("Properties source map differs from its fixed fixture")
        mode = data.get("mode")
        if mode == "preview":
            if wrote or type(data.get("effects")) is not int or data["effects"] < 0:
                raise ValueError("Properties preview changed source or omitted its effect count")
            operations = args.get("ops") or []
            targets = set()
            plan_mixed_role = plan_mixed_subsystem = False
            if not operations or not discovery:
                raise ValueError("Properties preview omitted discovery or operations")
            for operation in operations:
                arguments = operation.get("args") or {}
                target = arguments.get("at", args.get("at"))
                values = arguments.get("values")
                if args.get("at") not in PATHS or target != args["at"]:
                    raise ValueError("Properties plan target is outside its fixed owner scope")
                if (operation.get("op") != "props.set" or target not in PATHS
                        or set(arguments) - {"at", "values"} or not isinstance(values, dict) or not values):
                    raise ValueError("Properties plan includes an unrequested operation, target or argument")
                if contracts.get(target, call["index"]) >= call["index"]:
                    raise ValueError("target-specific Can contract did not precede Properties preview")
                for key, value in values.items():
                    canonical = ALIASES.get(key, key) if target != MODULE else key
                    expected = TARGET_VALUES[target].get(canonical)
                    if canonical not in TARGET_VALUES[target] or type(value) is not type(expected) or value != expected:
                        raise ValueError("Properties preview attempts an unrequested property value")
                if target == ROLE and {"synonym", "Comment"} <= values.keys():
                    plan_mixed_role = True
                if target == SUBSYSTEM and {"Synonym", "comment", "explanation"} <= values.keys():
                    plan_mixed_subsystem = True
                targets.add(target)
            token, plan_hash = data.get("executionToken"), data.get("planHash")
            if not isinstance(token, str) or not token or token in plans or not isinstance(plan_hash, str):
                raise ValueError("Properties preview omitted or repeated its saved identity")
            plans[token] = (targets, call["done"], after, plan_hash, data["effects"], plan_mixed_role, plan_mixed_subsystem)
        elif mode == "published":
            if set(args) != {"executionToken"}:
                raise ValueError("Properties publication did not use executionToken alone")
            plan = plans.pop(args["executionToken"], None)
            if (plan is None or plan[1] >= call["index"] or plan[2] != before
                    or data.get("planHash") != plan[3] or data.get("effects") != plan[4]):
                raise ValueError("Properties publication did not consume its unchanged observed preview")
            changes = changed_files(before, after)
            if not changes <= {PATHS[target] for target in plan[0]} or bool(changes) != (plan[4] > 0):
                raise ValueError("Properties publication wrote outside its planned descriptors")
            for target in plan[0]:
                if PATHS[target] in changes:
                    published[target] = call["done"]
            mixed_role |= plan[5] and PATHS[ROLE] in changes
            mixed_subsystem |= plan[6] and PATHS[SUBSYSTEM] in changes
        else:
            raise ValueError("Properties apply has no observed terminal mode")
    if set(published) != PATHS.keys() or not mixed_role or not mixed_subsystem or not configuration:
        raise ValueError("agent omitted Properties publication, mixed aliases or Configuration inspection")
    for target, done in published.items():
        later = [call for call in calls if call["index"] > done
                 and call["params"]["arguments"].get("at") == target]
        if not any(call["params"]["name"] == "unica.view" and (call["result"] or {}).get("ok") is True
                   for call in later):
            raise ValueError("agent did not reread its final Properties owner")
        checks = [call["result"] or {} for call in later if call["params"]["name"] == "unica.check"]
        if not any((result.get("ok") is True and (result.get("data") or {}).get("status") == "passed")
                   or (target == MODULE and result.get("ok") is False
                       and (result.get("data") or {}).get("tool") == "bsl-analyzer"
                       and (result.get("data") or {}).get("stage") == "manifest"
                       and any(item.get("code") == "provider_unavailable" and item.get("detailCode") == "provider_absent"
                               for item in result.get("diagnostics", [])))
                   for result in checks):
            raise ValueError("agent omitted the final Properties owner check or hid a failing validator")
    return allowed, raw_calls


def expected_descriptor_images():
    """Literal fixture substitutions, independent of the product writer."""
    images = {target: (SOURCE_FIXTURE / path).read_bytes() for target, path in PATHS.items()}
    substitutions = {
        MODULE: [("<Comment/>", "<Comment>Agent1721 module</Comment>"),
                 ("<ClientOrdinaryApplication>true</ClientOrdinaryApplication>",
                  "<ClientOrdinaryApplication>false</ClientOrdinaryApplication>")],
        ROLE: [("<v8:content>Читатель продаж</v8:content>", "<v8:content>Agent1721 role</v8:content>"),
               ("<Comment/>", "<Comment>Agent1721 role comment</Comment>")],
        SUBSYSTEM: [("<v8:content>Администрирование</v8:content>", "<v8:content>Agent1721 subsystem</v8:content>"),
                    ("<Comment/>", "<Comment>Agent1721 subsystem comment</Comment>"),
                    ("<IncludeInCommandInterface>true</IncludeInCommandInterface>",
                     "<IncludeInCommandInterface>false</IncludeInCommandInterface>"),
                    ("<Explanation>\r\n\t\t\t\t<v8:item>\r\n\t\t\t\t\t<v8:lang>ru</v8:lang>\r\n"
                     "\t\t\t\t\t<v8:content>Администрирование, настройка, сервисные функции</v8:content>\r\n"
                     "\t\t\t\t</v8:item>\r\n\t\t\t</Explanation>", "<Explanation/>")],
    }
    for target, replacements in substitutions.items():
        for old, new in replacements:
            if images[target].count(old.encode()) != 1:
                raise ValueError("Properties source fixture no longer matches its independent expected literal")
            images[target] = images[target].replace(old.encode(), new.encode())
    return {PATHS[target]: raw for target, raw in images.items()}


def verify_properties_images(workspace, source_before):
    """Verify requested namespaces, scalar values and all retained physical bytes."""
    expected = expected_descriptor_images()
    observations = {}

    def bound_before_image(path):
        raw = read_file(SOURCE_FIXTURE, path)
        if source_before.get(path) != hashlib.sha256(raw).hexdigest():
            raise ValueError("Properties before image is not bound to its recorded source hash")
        return raw

    def image_evidence(raw, owner=None):
        languages = ET.fromstring(raw.decode("utf-8-sig")).iter(CORE + "lang")
        image = {"xmlUtf8": raw.decode("utf-8"), "sha256": hashlib.sha256(raw).hexdigest(),
                 "bom": raw.startswith(b"\xef\xbb\xbf"), "crlf": b"\r\n" in raw,
                 "allLanguages": list(dict.fromkeys(node.text for node in languages))}
        if owner is not None:
            properties = owner.find(MD + "Properties")
            image["ownerName"] = properties.findtext(MD + "Name")
            image["uuid"] = owner.get("uuid")
            image["languages"] = {key: [item.findtext(CORE + "lang") for item in node.findall(CORE + "item")]
                                  for key in ["Synonym", "Explanation"]
                                  if (node := properties.find(MD + key)) is not None}
        return image

    for target, path in PATHS.items():
        before_raw = bound_before_image(path)
        raw = read_file(workspace, path)
        root = ET.fromstring(raw.decode("utf-8-sig"))
        kind, name = target.split(":", 1)[1].split(".", 1)
        nodes = root.findall(MD + kind)
        if root.tag != MD + "MetaDataObject" or len(nodes) != 1:
            raise ValueError("Properties descriptor has a wrong owner namespace or multiplicity")
        props = nodes[0].findall(MD + "Properties")
        if len(props) != 1 or props[0].findtext(MD + "Name") != name:
            raise ValueError("Properties descriptor lost its canonical Properties/Name")
        values = {}
        for key, requested in TARGET_VALUES[target].items():
            found = props[0].findall(MD + key)
            if len(found) != 1:
                raise ValueError("Properties descriptor omitted or duplicated a canonical property")
            if key in {"Synonym", "Explanation"}:
                items = found[0].findall(CORE + "item")
                values[key] = [(item.findtext(CORE + "lang"), item.findtext(CORE + "content")) for item in items]
                wanted = [] if requested == "" else [("ru", requested)]
                valid = values[key] == wanted
            else:
                values[key] = found[0].text or ""
                valid = values[key] == (str(requested).lower() if type(requested) is bool else requested)
            if not valid:
                raise ValueError("Properties descriptor has a wrong requested scalar value or type spelling")
        if raw != expected[path]:
            raise ValueError("Properties full descriptor postimage differs from the independent expected bytes")
        before_owner = ET.fromstring(before_raw.decode("utf-8-sig")).find(MD + kind)
        before_image, after_image = image_evidence(before_raw, before_owner), image_evidence(raw, nodes[0])
        observations[target] = {"path": path, "ownerName": name, "properties": values, "uuid": nodes[0].get("uuid"),
                                "sha256": after_image["sha256"], "bom": after_image["bom"], "crlf": after_image["crlf"],
                                "before": before_image, "after": after_image,
                                "expectedPostimageSHA256": hashlib.sha256(expected[path]).hexdigest(),
                                "exactExpectedPostimage": True}
    observed_files = {"src/" + path: digest for path, digest in source_file_hashes(workspace / "src").items()}
    expected_files = {**source_before, **{path: hashlib.sha256(raw).hexdigest() for path, raw in expected.items()}}
    if observed_files != expected_files:
        raise ValueError("Properties changed, created or removed an unrequested physical source file")
    companions = {}
    for path in ["src/Roles/SalesReader/Ext/Rights.xml", "src/Subsystems/Администрирование/Ext/CommandInterface.xml"]:
        before_raw, raw = bound_before_image(path), read_file(workspace, path)
        if raw != before_raw:
            raise ValueError("Properties changed a retained companion's exact bytes")
        companions[path] = {"before": image_evidence(before_raw), "after": image_evidence(raw), "byteEqual": True}
    project = read_file(workspace, "v8project.yaml")
    if project != (SOURCE_FIXTURE / "v8project.yaml").read_bytes():
        raise ValueError("Properties changed its project source map")
    return {"descriptors": observations, "companions": companions, "sourceFilesBeforeSha256": dict(source_before),
            "sourceFilesSha256": observed_files, "projectSha256": hashlib.sha256(project).hexdigest()}


def evaluate(server, scenario, proof_root):
    proof = evaluate_contract(server, scenario, proof_root, fixture=FIXTURE, auditor=audit,
                              changed_paths=set(PATHS.values()), source_roots={"main": "src"},
                              help_ref="skills/meta-edit/SKILL.md")
    manifest = json.loads((proof / "manifest.json").read_text())
    observations = verify_properties_images(server.workspace, manifest["sourceBefore"])
    path = proof / "file-observations.json"
    path.write_text(json.dumps(observations, ensure_ascii=False, indent=2))
    manifest["fileObservationsSha256"] = hashlib.sha256(path.read_bytes()).hexdigest()
    manifest["binarySha256"] = hashlib.sha256(Path(server.binary).read_bytes()).hexdigest()
    (proof / "manifest.json").write_text(json.dumps(manifest, indent=2))
    return proof


def independent_review(proof):
    review_contract(proof, rubric=RUBRIC)
