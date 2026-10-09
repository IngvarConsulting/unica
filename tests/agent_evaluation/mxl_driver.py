"""MXL acceptance uses the shared packet, transcript and independent review engine."""
from tests.agent_evaluation.dcs_driver import (
    REPO, audit_mcp, terminal_calls, evaluate as evaluate_contract, independent_review as review_contract,
)

OWNER = "cf:Report.F06Report"
TEMPLATE = OWNER + ".Template.Agent1311"
PREFIX = "src/cf/Reports/F06Report/"
RUBRIC = (
    "Агент нашёл и прочитал F06Template, области A/B, текст области A и параметры A. "
    "Из подробных can-контрактов он узнал template.add и mxl.set, создал SpreadsheetDocument "
    "Agent1311 и после применения создания заполнил Header. Вход mxl.set содержит "
    "R1C1=Товар, R1C2=Сумма, R2C1=Agent1311, R2C2=JSON-число 12.5, columns=4. "
    "По принятому контракту number сериализуется в текст: ожидаемый Body и XML "
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
                              required_operations={"template.add", "mxl.set"})
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


def evaluate(server, scenario, proof_root):
    return evaluate_contract(server, scenario, proof_root,
                             fixture=REPO / "tests/fixtures/acceptance/agent-mxl",
                             source="src", auditor=audit,
                             changed_paths={"src/cf/Reports/F06Report.xml",
                                            PREFIX + "Templates/Agent1311.xml",
                                            PREFIX + "Templates/Agent1311/Ext/Template.xml"})


def independent_review(proof):
    review_contract(proof, rubric=RUBRIC)
