"""MXL acceptance uses the shared packet, transcript and independent review engine."""
import hashlib
import json

from tests.agent_evaluation.dcs_driver import (
    REPO, audit_mcp, terminal_calls, evaluate as evaluate_contract, independent_review as review_contract,
)
from tests.ci.acceptance_controls import assert_value, file_value, read_file

OWNER = "cf:Report.F06Report"
TEMPLATE = OWNER + ".Template.Agent1311"
PREFIX = "src/cf/Reports/F06Report/"
RUBRIC = (
    "Агент нашёл и прочитал F06Template, области A/B, текст области A и параметры A. "
    "Агент прочитал подробные Can-контракты фактически выбранных операций до preview, "
    "создал SpreadsheetDocument Agent1311 с областью Header через публичные операции. "
    "В результате R1C1=Товар, R1C2=Сумма, R2C1=Agent1311, R2C2=12.5. "
    "По принятому контракту число представлено текстом: ожидаемый Body и XML "
    "содержат строку 12.5, сохранение числового типа в MXL не требуется. "
    "Независимые fileObservations доказывают область Header, две строки, ширину 4, "
    "точное расположение этих четырёх текстов и пустоту колонок 3–4. "
    "Каждая публикация потребляет свой наблюдённый preview; previews/refusals не меняют "
    "исходники. После публикации есть обратное чтение и check с mxl/status=passed. "
    "Все существующие макеты сохранены. Ответ правильно объясняет, что публичный "
    "Unica не предоставляет полный макет как JSON DSL и обратную сборку такого DSL "
    "с оформлением, шрифтами и объединениями; не обещает этот round-trip и не "
    "предлагает ручную запись XML в обход поверхности. Агент не выдаёт независимые "
    "физические наблюдения корпуса за собственное чтение XML. Ответ не заявляет "
    "о выполненной загрузке в 1С, сборке конфигурации или печати без подтверждающего "
    "протокола. Отдельное предупреждение об отсутствии этих проверок не требуется."
)


def audit(records):
    allowed, calls = audit_mcp(records, target_template=TEMPLATE, owner=OWNER,
                              required_operations={"template.add"})
    observations = terminal_calls(calls)
    first_publication = min(call["index"] for call in observations
                        if call["params"]["name"] == "unica.apply"
                        and ((call["result"] or {}).get("data") or {}).get("mode") == "published")
    for target in [OWNER + ".Template.F06Template.Area.A.Body",
                   OWNER + ".Template.F06Template.Area.A.Parameter"]:
        if not any(call["index"] < first_publication and call["params"]["name"] == "unica.view"
                   and call["params"]["arguments"].get("at") == target
                   and (call["result"] or {}).get("ok") is True for call in observations):
            raise ValueError("agent did not read the requested existing layout before creation")
    area = OWNER + ".Template.F06Template.Area.B"
    if not any(call["index"] < first_publication and call["params"]["name"] == "unica.view"
               and (call["result"] or {}).get("ok") is True
               and (call["params"]["arguments"].get("at") == area
                    or (call["params"]["arguments"].get("at") == OWNER + ".Template.F06Template.Area"
                        and any(item.get("at") == area for item in ((call["result"] or {}).get("data") or {}).get("items", []))))
               for call in observations):
        raise ValueError("agent did not read existing area B before creation")
    return allowed, calls


def template_registration(workspace):
    namespace = "{http://v8.1c.ru/8.3/MDClasses}"
    return file_value(workspace, {"kind": "xml", "path": "src/cf/Reports/F06Report.xml",
        "select": f"./{namespace}Report/{namespace}ChildObjects/{namespace}Template"})


def verify_mxl_images(workspace, scenario, source_before, registered_before):
    """Read the frozen corpus assertions and retained template bytes directly."""
    assertions = [assertion for step in scenario["wire"] for assertion in step.get("fileAssertions", [])]
    if not assertions:
        raise ValueError("MXL evaluation omitted its physical file assertions")
    sheet = "{http://v8.1c.ru/8.2/data/spreadsheet}"
    core = "{http://v8.1c.ru/8.1/data/core}"
    for row, column, text in [(0, 1, "Товар"), (0, 2, "Сумма"), (1, 1, "Agent1311"), (1, 2, "12.5")]:
        assertions.append({"kind": "xml", "path": PREFIX + "Templates/Agent1311/Ext/Template.xml",
            "select": f"./{sheet}rowsItem[{sheet}index='{row}']/{sheet}row/{sheet}c[{column}]"
                      f"/{sheet}c/{sheet}tl/{core}item/{core}content[.!='']",
            "eq": [text]})
    observations = []
    for assertion in assertions:
        observed = file_value(workspace, assertion)
        assert_value(observed, assertion, {})
        observations.append({"descriptor": assertion, "observed": observed})
    preserved = {path: digest for path, digest in source_before.items()
                 if path.startswith(PREFIX + "Templates/")}
    if not preserved:
        raise ValueError("MXL evaluation omitted its original template preimages")
    for path, expected in preserved.items():
        try:
            actual = hashlib.sha256(read_file(workspace, path)).hexdigest()
        except OSError as error:
            raise ValueError(f"existing template resource disappeared: {path}") from error
        if actual != expected:
            raise ValueError(f"existing template resource changed: {path}")
    registered = template_registration(workspace)
    if (registered.count("Agent1311") != 1
            or [name for name in registered if name != "Agent1311"] != registered_before):
        raise ValueError("MXL template registration lost an existing template or added an unrequested one")
    namespace = "{http://v8.1c.ru/8.3/MDClasses}"
    kind = file_value(workspace, {"kind": "xml", "path": PREFIX + "Templates/Agent1311.xml",
        "select": f"./{namespace}Template/{namespace}Properties/{namespace}TemplateType"})
    if kind != ["SpreadsheetDocument"]:
        raise ValueError("created MXL template descriptor is not SpreadsheetDocument")
    return {"template": TEMPLATE, "fileAssertions": observations, "templateType": kind[0],
            "preservedTemplatesSha256": preserved, "registeredTemplatesBefore": registered_before,
            "registeredTemplates": registered}


def evaluate(server, scenario, proof_root):
    registered_before = template_registration(server.workspace)
    proof = evaluate_contract(server, scenario, proof_root,
                             fixture=REPO / "tests/fixtures/acceptance/agent-mxl",
                             source="src", auditor=audit,
                             changed_paths={"src/cf/Reports/F06Report.xml",
                                            PREFIX + "Templates/Agent1311.xml",
                                            PREFIX + "Templates/Agent1311/Ext/Template.xml"})
    manifest = json.loads((proof / "manifest.json").read_text())
    observations = verify_mxl_images(server.workspace, scenario, manifest["sourceBefore"], registered_before)
    path = proof / "file-observations.json"
    path.write_text(json.dumps(observations, ensure_ascii=False, indent=2))
    manifest["fileObservationsSha256"] = hashlib.sha256(path.read_bytes()).hexdigest()
    (proof / "manifest.json").write_text(json.dumps(manifest, indent=2))
    return proof


def independent_review(proof):
    review_contract(proof, rubric=RUBRIC)
