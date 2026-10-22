"""Real XDTO Codex acceptance using the existing packet and review engine."""
from __future__ import annotations

import json
from pathlib import Path

from jsonschema import Draft202012Validator, SchemaError, ValidationError

from tests.agent_evaluation.dcs_driver import (
    changed_files, evaluate as evaluate_contract, independent_review as review_contract, terminal_calls,
)
from tests.agent_evaluation.mcp_recorder import recorded_calls, tree_digest
from tests.ci.xdto_contract_evidence import (
    REPO, WORKSPACE, PACKAGE, XML, NESTED, DOTTED, OBJECT, FIELD, VALUE, OLD_TYPE, OLD_PROPERTY, sha, verify_xdto_images,
    FIXTURE as SOURCE_FIXTURE, source_hashes,
)

FIXTURE = REPO / "tests/fixtures/acceptance/agent-xdto"
REQUESTS = {
    (PACKAGE, "valueType.add"): {"values": {"name": "Agent1721Value", "base": "xs:string"}},
    (PACKAGE, "objectType.add"): {"values": {"name": "Agent1721Object"}},
    (OBJECT, "property.add"): {"values": {"property": {"name": "Value", "type": "tns:Agent1721Value", "minOccurs": 0}}},
    (NESTED, "property.add"): {"values": {"property": {"name": "Agent1721.A", "type": "xs:string"}}},
    (OLD_TYPE, "type.remove"): {},
    (OLD_PROPERTY, "property.remove"): {},
}
RUBRIC = (
    "Агент обнаружил main source-set и единственный изменяемый XDTOPackage.EnterpriseData_1_17_3. "
    "Все пять identity операций и оба site property.add подтверждены настоящими detailed Can до соответствующего preview. "
    "Созданы Agent1721Value base xs:string, Agent1721Object и его Value type tns:Agent1721Value с lowerBound=0. "
    "В существующем anonymous ObjectType СсылкаНаОбъект создано свойство Agent1721.A type xs:string без явного lowerBound; "
    "его логический адрес экранирует буквальную точку. Удалены СоставнойЛюбойОбъект и EnterpriseData.Версия. "
    "Каждая публикация потребляет именно наблюдённый executionToken/planHash; исходники не меняются от preview/чтения/отказа. "
    "После последней публикации прочитаны пакет, созданные типы и свойства, проверено отсутствие удалённых узлов. "
    "Check пакета passed, validators=[xdto]: находки профиля Unica, а не загрузка в 1С и не вердикт платформы. "
    "FileObservations независимо проверяют полный буквальный Package.bin до/после, namespace, BOM/CRLF, порядок узлов, "
    "сохранённые descriptor/Configuration UUID/Name/version/Namespace, readonly соседний пакет и полную карту исходников. "
    "Агент использовал values-only add, no-payload remove и корректно объяснил математический integer 0/1 для minOccurs, "
    "включая эквивалентные 0.0/1.0; дробные/вне диапазона/строки/bool/null недопустимы. "
    "Нет ручного редактирования XML, других источников, новых операций или обещания XDTO validator/проверки платформой. "
    "Физическое чтение драйвера не выдаётся за чтение XML самим агентом."
)


def requested(actual, expected):
    if isinstance(expected, dict):
        return isinstance(actual, dict) and actual.keys() == expected.keys() and all(requested(actual[k], v) for k, v in expected.items())
    if type(expected) is int:
        return type(actual) in {int, float} and actual == expected
    return type(actual) is type(expected) and actual == expected


def bind_recorded_images(records, calls):
    """Every recorder frame belongs to the fixture or an observed publication.

    A Task may finish writing before its terminal result is polled. Both bound
    images are therefore valid during that publication's observed interval;
    other files remain immutable even while a Task is in flight.
    """
    initial = source_hashes(SOURCE_FIXTURE)
    initial["v8project.yaml"] = sha((SOURCE_FIXTURE / "v8project.yaml").read_bytes())
    immutable = {path: digest for path, digest in initial.items() if path != XML}
    publications = [call for call in calls if call["params"]["name"] == "unica.apply"
                    and (call["result"] or {}).get("ok") is True
                    and ((call["result"] or {}).get("data") or {}).get("mode") == "published"]
    for index, record in enumerate(records):
        files = record.get("sourceFilesSha256")
        if (not isinstance(files, dict) or files.keys() != initial.keys()
                or {path: digest for path, digest in files.items() if path != XML} != immutable
                or record.get("sourceSetRoots") != {"main": "src"}):
            raise ValueError("XDTO recorder frame changed an immutable fixture input")
        source = {path.removeprefix("src/"): digest for path, digest in files.items() if path.startswith("src/")}
        if record.get("sourceSha256") != tree_digest(source):
            raise ValueError("XDTO recorder source digest is not bound to its complete file map")
        completed = [call for call in publications if call["done"] < index]
        current = max(completed, key=lambda call: call["done"])["filesAfter"][XML] if completed else initial[XML]
        allowed = {current}
        for call in publications:
            if call["index"] <= index <= call["done"]:
                allowed.update([call["filesBefore"][XML], call["filesAfter"][XML]])
        if files[XML] not in allowed:
            raise ValueError("XDTO recorder frame is not bound to the current publication state")
    final = max(publications, key=lambda call: call["done"])["filesAfter"] if publications else initial
    if not records or records[0].get("sourceFilesSha256") != initial or records[-1].get("sourceFilesSha256") != final:
        raise ValueError("XDTO recorder did not retain its canonical first and final publication images")


def audit(records):
    allowed, raw_calls = recorded_calls(records)
    calls = terminal_calls(raw_calls)
    contracts, plans, published = {}, {}, set()
    last_publication = -1
    discovery = False
    for call in calls:
        tool, args = call["params"]["name"], call["params"]["arguments"]
        if not isinstance(args, dict):
            raise ValueError("XDTO call has a malformed argument envelope")
        result = call["result"] or {}
        data = result.get("data") or {}
        if result.get("ok") is True and call["isError"]:
            raise ValueError("successful XDTO observation is flagged as an MCP error")
        if tool not in allowed or tool not in {"unica.view", "unica.search", "unica.check", "unica.apply", "unica.diff"}:
            raise ValueError("XDTO call is outside its actual MCP inventory")
        before, after = call.get("filesBefore"), call.get("filesAfter")
        if not isinstance(before, dict) or not isinstance(after, dict):
            raise ValueError("XDTO evaluation requires complete per-file recorder evidence")
        wrote = before != after or call["before"] != call["after"]
        if tool != "unica.apply":
            if wrote:
                raise ValueError("read-only XDTO observation changed source bytes")
            if tool == "unica.view" and result.get("ok") is True:
                if args == {}:
                    discovery = True
                at = args.get("at")
                for target, operation in REQUESTS:
                    operation_at = target + ".Can." + operation
                    props = data.get("props") or {}
                    schema = (props.get("contract") or {}).get("argsSchema") or {}
                    if (at == operation_at and result.get("at") == at and data.get("at") == at
                            and data.get("kind") == "Can" and props.get("op") == operation
                            and props.get("implemented") is True and isinstance(schema, dict) and schema.get("type") == "object"
                            and schema.get("additionalProperties") is False):
                        try:
                            Draft202012Validator.check_schema(schema)
                        except SchemaError as error:
                            raise ValueError("XDTO detailed Can published an invalid JSON Schema") from error
                        contracts[(target, operation)] = (call["done"], Draft202012Validator(schema))
            continue
        if result.get("ok") is not True:
            if wrote or data.get("executionToken"):
                raise ValueError("refused XDTO apply changed source or minted a token")
            continue
        if call["rootsBefore"] != {"main": "src"} or call["rootsAfter"] != {"main": "src"}:
            raise ValueError("XDTO source map differs from its fixed fixture")
        if data.get("mode") == "preview":
            if wrote or type(data.get("effects")) is not int or data["effects"] < 0:
                raise ValueError("XDTO preview wrote source or omitted its effect count")
            if (not discovery or set(args) != {"at", "ops"} or not isinstance(args["ops"], list) or not args["ops"]
                    or not isinstance(args["at"], str) or args["at"] not in {target for target, _ in REQUESTS}):
                raise ValueError("XDTO preview omitted discovery or canonical envelope")
            identities = set()
            for operation in args["ops"]:
                if not isinstance(operation, dict) or not isinstance(operation.get("args", {}), dict):
                    raise ValueError("XDTO preview has a malformed operation envelope")
                arguments = operation.get("args", {})
                target = arguments.get("at", args["at"])
                if not isinstance(target, str) or not isinstance(operation.get("op"), str):
                    raise ValueError("XDTO preview has a malformed operation identity")
                identity = target, operation["op"]
                if (identity not in REQUESTS or set(operation) - {"op", "args"}
                        or not (target == args["at"] or target.startswith(args["at"] + "."))
                        or not requested({k: v for k, v in arguments.items() if k != "at"}, REQUESTS[identity])):
                    raise ValueError("XDTO plan contains an unrequested target, value or argument")
                contract = contracts.get(identity)
                if contract is None or contract[0] >= call["index"]:
                    raise ValueError("target-specific XDTO Can did not precede preview")
                try:
                    contract[1].validate(arguments)
                except ValidationError as error:
                    raise ValueError("XDTO requested arguments do not satisfy their published Can schema") from error
                identities.add(identity)
            token, plan_hash = data.get("executionToken"), data.get("planHash")
            if not isinstance(token, str) or not token or token in plans or not isinstance(plan_hash, str):
                raise ValueError("XDTO preview omitted or repeated its saved identity")
            plans[token] = (call["done"], after, plan_hash, data["effects"], identities)
        elif data.get("mode") == "published":
            if set(args) != {"executionToken"}:
                raise ValueError("XDTO publication did not use executionToken alone")
            plan = plans.pop(args["executionToken"], None)
            if (plan is None or plan[0] >= call["index"] or plan[1] != before
                    or data.get("planHash") != plan[2] or data.get("effects") != plan[3]):
                raise ValueError("XDTO publication did not consume its unchanged observed preview")
            changes = changed_files(before, after)
            if not changes <= {XML} or bool(changes) != (plan[3] > 0):
                raise ValueError("XDTO publication wrote outside its planned package")
            if changes:
                published |= plan[4]
                last_publication = call["done"]
        else:
            raise ValueError("XDTO apply has no observed terminal mode")
    bind_recorded_images(records, calls)
    if published != REQUESTS.keys():
        raise ValueError("agent omitted a requested XDTO operation or contextual target")
    later = [call for call in calls if call["index"] > last_publication]
    for target in [PACKAGE, OBJECT, VALUE, FIELD, DOTTED]:
        if not any(call["params"]["name"] == "unica.view" and call["params"]["arguments"].get("at") == target
                   and (call["result"] or {}).get("ok") is True and (call["result"] or {}).get("at") == target
                   and ((call["result"] or {}).get("data") or {}).get("at") == target for call in later):
            raise ValueError("agent did not reread its final XDTO node")
    for target in [OLD_TYPE, OLD_PROPERTY]:
        if not any(call["params"]["name"] == "unica.view" and call["params"]["arguments"].get("at") == target
                   and (call["result"] or {}).get("ok") is False
                   and (call["result"] or {}).get("at") == target
                   and any(item.get("code") == "not_found" for item in (call["result"] or {}).get("diagnostics", [])) for call in later):
            raise ValueError("agent did not verify its removed XDTO node")
    if not any(call["params"]["name"] == "unica.check"
               and set(call["params"]["arguments"]) <= {"at", "limit", "cursor"}
               and call["params"]["arguments"].get("at") == PACKAGE
               and (call["params"]["arguments"].get("limit") is None
                    or (type(call["params"]["arguments"]["limit"]) is int
                        and 1 <= call["params"]["arguments"]["limit"] <= 50))
               and call["params"]["arguments"].get("cursor") in (None, "")
               and (call["result"] or {}).get("ok") is True and (call["result"] or {}).get("at") == PACKAGE
               and ((call["result"] or {}).get("data") or {}).get("status") == "passed"
               and ((call["result"] or {}).get("data") or {}).get("validators") == ["xdto"] for call in later):
        raise ValueError("agent omitted the final XDTO package check")
    return allowed, raw_calls


def bind_final_evidence(records, manifest, observations):
    """The last wire snapshot and the independently read physical image agree."""
    recorded = records[-1].get("sourceFilesSha256") if records else None
    declared = manifest.get("sourceAfter")
    observed = observations.get("sourceFilesSha256")
    if (not isinstance(recorded, dict) or not isinstance(declared, dict) or not isinstance(observed, dict)
            or {path: digest for path, digest in recorded.items() if path != "v8project.yaml"} != declared
            or declared != observed or recorded.get("v8project.yaml") != observations.get("projectSHA256")):
        raise ValueError("XDTO final recorder, manifest and physical source images are disconnected")


def evaluate(server, scenario, proof_root):
    if scenario.get("workspace") != WORKSPACE:
        raise ValueError("XDTO evaluation requires its canonical focused workspace")
    proof = evaluate_contract(server, scenario, proof_root, fixture=FIXTURE, auditor=audit,
                              changed_paths={XML}, source_roots={"main": "src"}, help_ref="skills/xdto/SKILL.md")
    manifest = json.loads((proof / "manifest.json").read_text())
    observations = verify_xdto_images(server.workspace, manifest["sourceBefore"])
    records = [json.loads(line) for line in (proof / "mcp.jsonl").read_text().splitlines()]
    bind_final_evidence(records, manifest, observations)
    path = proof / "file-observations.json"
    path.write_text(json.dumps(observations, ensure_ascii=False, indent=2))
    manifest["fileObservationsSha256"] = sha(path.read_bytes())
    manifest["binarySha256"] = sha(Path(server.binary).read_bytes())
    (proof / "manifest.json").write_text(json.dumps(manifest, indent=2))
    return proof


def independent_review(proof):
    review_contract(proof, rubric=RUBRIC)
