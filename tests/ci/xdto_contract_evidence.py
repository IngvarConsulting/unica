"""Literal XDTO acceptance images, independent of the productive XML writer."""
from __future__ import annotations

import hashlib
import xml.etree.ElementTree as ET
from pathlib import Path

from tests.agent_evaluation.mcp_recorder import source_file_hashes
from tests.ci.acceptance_controls import read_file

REPO = Path(__file__).resolve().parents[2]
WORKSPACE = "tests/fixtures/acceptance/workspace-xdto-contract"
FIXTURE = REPO / WORKSPACE
PACKAGE = "main:XDTOPackage.EnterpriseData_1_17_3"
XML = "src/XDTOPackages/EnterpriseData_1_17_3/Ext/Package.bin"
DESCRIPTOR = "src/XDTOPackages/EnterpriseData_1_17_3.xml"
NESTED = PACKAGE + ".Type.ЛюбаяСсылка.Property.СсылкаНаОбъект"
DOTTED = NESTED + r".Property.Agent1721\.A"
OBJECT = PACKAGE + ".Type.Agent1721Object"
FIELD = OBJECT + ".Property.Value"
VALUE = PACKAGE + ".Type.Agent1721Value"
OLD_TYPE = PACKAGE + ".Type.СоставнойЛюбойОбъект"
OLD_PROPERTY = PACKAGE + ".Type.EnterpriseData.Property.Версия"
URI = "http://v8.1c.ru/edi/edi_stnd/EnterpriseData/1.17.3"
XDTO = "{http://v8.1c.ru/8.1/xdto}"
MD = "{http://v8.1c.ru/8.3/MDClasses}"


def sha(raw):
    return hashlib.sha256(raw).hexdigest()


def source_hashes(workspace):
    return {"src/" + path: digest for path, digest in source_file_hashes(workspace / "src").items()}


def expected_package(stage="changed"):
    """Concrete full postimages for the fixed developer task; no writer calls."""
    raw = (FIXTURE / XML).read_bytes()
    if stage == "base":
        return raw
    if stage == "types":
        return raw.replace(b'\t<objectType name="EnterpriseData">',
                           b'\t<valueType name="Agent1721Value" base="xs:string"/>\r\n\t<objectType name="EnterpriseData">').replace(
                               b'</package>', b'\t<objectType name="Agent1721Object"/>\r\n</package>')
    replacements = [
        ('\t<objectType name="EnterpriseData">',
         '\t<valueType name="Agent1721Value" base="xs:string"/>\r\n\t<objectType name="EnterpriseData">'),
        (f'\t\t<property xmlns:tns="{URI}" name="Версия" type="tns:ВерсияФормата" lowerBound="0"/>\r\n', ''),
        ('\t<objectType name="СоставнойЛюбойОбъект"/>\r\n', ''),
        (f'\t\t\t\t<property xmlns:tns="{URI}" name="Документ_ЗаказКлиента" type="tns:Документ_ЗаказКлиента" lowerBound="0" upperBound="-1"/>',
         f'\t\t\t\t<property xmlns:tns="{URI}" name="Документ_ЗаказКлиента" type="tns:Документ_ЗаказКлиента" lowerBound="0" upperBound="-1"/>\r\n'
         '\t\t\t\t<property name="Agent1721.A" type="xs:string"/>'),
        ('</package>', '\t<objectType name="Agent1721Object">\r\n'
         f'\t\t<property xmlns:tns="{URI}" name="Value" type="tns:Agent1721Value" lowerBound="0"/>\r\n'
         '\t</objectType>\r\n</package>'),
    ]
    for old, new in replacements:
        old_bytes, new_bytes = old.encode(), new.encode()
        if raw.count(old_bytes) != 1:
            raise ValueError("XDTO acceptance fixture differs from its independently expected literal")
        raw = raw.replace(old_bytes, new_bytes)
    if stage in {"peer", "forward"}:
        raw = raw.replace(b'\t<objectType name="EnterpriseData">',
                          b'\t<valueType name="Agent1721Peer" base="xs:string"/>\r\n\t<objectType name="EnterpriseData">')
    if stage == "forward":
        raw = raw.replace(b'\t<objectType name="EnterpriseData">',
                          (f'\t<valueType xmlns:tns="{URI}" name="Agent1721ForwardA" base="tns:Agent1721ForwardB"/>\r\n'
                           '\t<valueType name="Agent1721ForwardB" base="xs:string"/>\r\n\t<objectType name="EnterpriseData">').encode())
    if stage not in {"changed", "peer", "forward"}:
        raise ValueError("unknown fixed XDTO acceptance postimage")
    return raw


def verify_xdto_images(workspace, before, stage="changed"):
    expected = expected_package(stage)
    initial = source_hashes(FIXTURE)
    if before != initial:
        raise ValueError("XDTO recorded source preimage differs from the canonical fixture")
    observed = source_hashes(workspace)
    if observed != {**before, XML: sha(expected)}:
        raise ValueError("XDTO changed, created or removed an unrequested source file")
    if read_file(workspace, XML) != expected:
        raise ValueError("XDTO full package differs from its independently expected postimage")
    if read_file(workspace, "v8project.yaml") != read_file(FIXTURE, "v8project.yaml"):
        raise ValueError("XDTO changed its project source map")
    images = {}
    for path in [XML, DESCRIPTOR, "src/Configuration.xml"]:
        original, raw = read_file(FIXTURE, path), read_file(workspace, path)
        if before.get(path) != sha(original):
            raise ValueError("XDTO before XML is not bound to recorded source bytes")
        images[path] = {"before": {"xmlUtf8": original.decode("utf-8"), "sha256": sha(original)},
                        "after": {"xmlUtf8": raw.decode("utf-8"), "sha256": sha(raw)},
                        "bomPreserved": original.startswith(b"\xef\xbb\xbf") == raw.startswith(b"\xef\xbb\xbf"),
                        "crlfPreserved": b"\r\n" in original and b"\r\n" in raw,
                        "byteEqual": raw == original}
    package = ET.fromstring(expected)
    if package.tag != XDTO + "package" or package.get("targetNamespace") != URI:
        raise ValueError("XDTO package lost its QName or namespace")
    descriptor = ET.fromstring(read_file(workspace, DESCRIPTOR)).find(MD + "XDTOPackage")
    if descriptor is None or descriptor.findtext(MD + "Properties/" + MD + "Name") != "EnterpriseData_1_17_3":
        raise ValueError("XDTO descriptor lost its canonical owner")
    return {"images": images, "sourceFilesBeforeSha256": before, "sourceFilesSha256": observed,
            "expectedPostimageSHA256": sha(expected), "exactExpectedPostimage": True,
            "ownerName": "EnterpriseData_1_17_3", "uuid": descriptor.get("uuid"),
            "namespace": URI, "typeOrder": [node.get("name") for node in package if node.tag in {XDTO + "valueType", XDTO + "objectType"}],
            "projectSHA256": sha(read_file(workspace, "v8project.yaml"))}
