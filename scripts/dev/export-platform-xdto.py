#!/usr/bin/env python3
"""Export the XDTO schemas a platform knows into the corpus of its format.

    export-platform-xdto.py [--build FORMAT=BUILD] [--work DIR] FORMAT...

For every format the newest installed build of its line (or the one given
with ``--build``) loads a configuration made of the 2.20 empty configuration
and one server module. Started in 1C:Enterprise, the module writes the XML
schema of every package of ``XDTOFactory`` and its list. The schemas land in
``tests/fixtures/formats/platform_xml/<format>/xdto/``, one file per schema
named after its namespace; ``manifest.json`` there names the build and the
packages. Values of enumerations the export format writes (``xcf/enums`` and
the others) are read from these files, not from memory.

The platform runs as ``1cv8``: create an infobase, load the configuration,
start the client with the output directory as the launch parameter.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import re
import shutil
import subprocess
import sys
import tempfile
import uuid
from pathlib import Path
from typing import Dict, List, Optional, Sequence, Tuple

REPO_ROOT = Path(__file__).resolve().parents[2]
CORPUS_ROOT = REPO_ROOT / "tests" / "fixtures" / "formats" / "platform_xml"
SEED = CORPUS_ROOT / "2.20" / "empty_configuration" / "src"
# Which platform line writes each format, as the format family's table says.
LINES = {"2.20": "8.3.27", "2.21": "8.5.1", "2.22": "8.5.4"}
# Other installed builds of a line whose export was compared with the one
# taken: the format of the line is the same, the XDTO differs as written.
OTHER_BUILDS = {
    "2.22": "8.5.4.1878: the same export format; its XDTO lacks the add-in architectures arm64 and e2k (managed-application/core, virtual-resource-system) and the package goskey/core",
}
COMMAND_TIMEOUT_SECONDS = 600
MODULE_NAME = "XdtoExport"

SERVER_MODULE = """Функция ЭкспортироватьСхемы(Каталог) Экспорт
	Строки = Новый Массив;
	Для Каждого Пакет Из ФабрикаXDTO.Пакеты Цикл
		Набор = ФабрикаXDTO.ЭкспортСхемыXML(Пакет.URIПространстваИмен);
		НомерСхемы = 0;
		Для Каждого Схема Из Набор Цикл
			НомерСхемы = НомерСхемы + 1;
			Если Схема.ДокументDOM = Неопределено Тогда
				Схема.ОбновитьЭлементDOM();
			КонецЕсли;
			Имя = Формат(Строки.Количество() + 1, "ЧЦ=4; ЧВН=; ЧГ=") + ".xsd";
			Запись = Новый ЗаписьXML;
			Запись.ОткрытьФайл(Каталог + "\\" + Имя, "UTF-8");
			ЗаписьDOM = Новый ЗаписьDOM;
			ЗаписьDOM.Записать(Схема.ДокументDOM, Запись);
			Запись.Закрыть();
			Строки.Добавить(Пакет.URIПространстваИмен + Символы.Таб + Имя);
		КонецЦикла;
	КонецЦикла;
	Текст = Новый ТекстовыйДокумент;
	Текст.УстановитьТекст(СтрСоединить(Строки, Символы.ПС));
	Текст.Записать(Каталог + "\\packages.tsv", "UTF-8");
	Возврат Строки.Количество();
КонецФункции
"""

APPLICATION_MODULE = f"""Процедура ПриНачалеРаботыСистемы()
	Если ЗначениеЗаполнено(ПараметрЗапуска) Тогда
		{MODULE_NAME}.ЭкспортироватьСхемы(ПараметрЗапуска);
		ЗавершитьРаботуСистемы(Ложь);
	КонецЕсли;
КонецПроцедуры
"""

COMMON_MODULE = f"""\ufeff<?xml version="1.0" encoding="UTF-8"?>
<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" xmlns:v8="http://v8.1c.ru/8.1/data/core" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" version="2.20">
	<CommonModule uuid="{{uuid}}">
		<Properties>
			<Name>{MODULE_NAME}</Name>
			<Synonym/>
			<Comment/>
			<Global>false</Global>
			<ClientManagedApplication>false</ClientManagedApplication>
			<Server>true</Server>
			<ExternalConnection>false</ExternalConnection>
			<ClientOrdinaryApplication>false</ClientOrdinaryApplication>
			<ServerCall>true</ServerCall>
			<Privileged>false</Privileged>
			<ReturnValuesReuse>DontUse</ReturnValuesReuse>
		</Properties>
	</CommonModule>
</MetaDataObject>
"""


class ExportError(RuntimeError):
    pass


def platform_executable(build: str) -> Path:
    for root in (Path("C:/Program Files/1cv8"), Path("C:/Program Files (x86)/1cv8")):
        executable = root / build / "bin" / "1cv8.exe"
        if executable.is_file():
            return executable
    raise ExportError(f"1cv8 of build {build} is not installed")


def newest_build(line: str) -> str:
    pattern = re.compile(rf"^{re.escape(line)}\.\d+$")
    builds = [
        entry.name
        for root in (Path("C:/Program Files/1cv8"), Path("C:/Program Files (x86)/1cv8"))
        if root.is_dir()
        for entry in root.iterdir()
        if pattern.match(entry.name)
    ]
    if not builds:
        raise ExportError(f"no installed build of platform line {line}")
    return max(builds, key=lambda build: tuple(int(part) for part in build.split(".")))


def write_seed(target: Path) -> None:
    shutil.copytree(SEED, target)
    configuration = target / "Configuration.xml"
    text = configuration.read_bytes()
    language = "<Language>Русский</Language>".encode("utf-8")
    if text.count(language) != 1:
        raise ExportError("the seed configuration does not list exactly one language")
    text = text.replace(
        language, language + f"\r\n\t\t\t<CommonModule>{MODULE_NAME}</CommonModule>".encode("utf-8")
    )
    configuration.write_bytes(text)
    module_dir = target / "CommonModules" / MODULE_NAME / "Ext"
    module_dir.mkdir(parents=True)
    (target / "CommonModules" / f"{MODULE_NAME}.xml").write_text(
        COMMON_MODULE.replace("{uuid}", str(uuid.uuid5(uuid.NAMESPACE_URL, MODULE_NAME))),
        encoding="utf-8",
        newline="\r\n",
    )
    (module_dir / "Module.bsl").write_text(
        "\ufeff" + SERVER_MODULE, encoding="utf-8", newline="\r\n"
    )
    (target / "Ext" / "ManagedApplicationModule.bsl").write_text(
        "\ufeff" + APPLICATION_MODULE, encoding="utf-8", newline="\r\n"
    )


def run_platform(executable: Path, arguments: Sequence[str], log: Path, step: str) -> None:
    try:
        completed = subprocess.run(
            [str(executable), *arguments, "/Out", str(log), "/DisableStartupDialogs"],
            timeout=COMMAND_TIMEOUT_SECONDS,
        )
    except (OSError, subprocess.TimeoutExpired) as error:
        raise ExportError(f"{step}: {error}") from error
    if completed.returncode != 0:
        detail = log.read_text(encoding="utf-8", errors="replace") if log.exists() else ""
        raise ExportError(f"{step} exited with {completed.returncode}: {detail.strip()}")


def export(build: str, work: Path) -> Path:
    executable = platform_executable(build)
    source = work / "src"
    base = work / "base"
    out = work / "out"
    write_seed(source)
    out.mkdir()
    run_platform(executable, ["CREATEINFOBASE", f"File={base};"], work / "create.log", "create")
    run_platform(
        executable,
        ["DESIGNER", "/F", str(base), "/LoadConfigFromFiles", str(source), "/UpdateDBCfg",
         "/DisableStartupMessages"],
        work / "load.log",
        "load",
    )
    run_platform(
        executable,
        ["ENTERPRISE", "/F", str(base), "/C", str(out), "/DisableStartupMessages"],
        work / "export.log",
        "export",
    )
    if not (out / "packages.tsv").is_file():
        raise ExportError(f"the client wrote no package list into {out}")
    return out


def schema_file_name(namespace: str, index: int) -> str:
    stem = re.sub(r"^[a-z]+://", "", namespace)
    stem = re.sub(r"[^A-Za-z0-9._-]+", "_", stem).strip("_")
    return f"{stem}.xsd" if index == 1 else f"{stem}.{index}.xsd"


def collect(out: Path) -> List[Tuple[str, str, bytes]]:
    seen: Dict[str, int] = {}
    schemas = []
    for line in (out / "packages.tsv").read_text(encoding="utf-8-sig").splitlines():
        if not line.strip():
            continue
        namespace, exported = line.split("\t")
        seen[namespace] = seen.get(namespace, 0) + 1
        schemas.append((namespace, schema_file_name(namespace, seen[namespace]),
                        (out / exported).read_bytes()))
    return schemas


def publish(export_format: str, build: str, schemas: List[Tuple[str, str, bytes]]) -> Path:
    target = CORPUS_ROOT / export_format / "xdto"
    if target.exists():
        shutil.rmtree(target)
    target.mkdir(parents=True)
    digest = hashlib.sha256()
    packages: Dict[str, List[str]] = {}
    for namespace, name, payload in sorted(schemas, key=lambda schema: schema[1]):
        (target / name).write_bytes(payload)
        digest.update(name.encode("utf-8") + b"\0" + payload)
        packages.setdefault(namespace, []).append(name)
    manifest = {
        "format": export_format,
        "build": build,
        "source": "XDTOFactory.ExportXMLSchema of every package, 1C:Enterprise client",
        "packages": len(packages),
        "schemas": len(schemas),
        "sha256": digest.hexdigest(),
        "otherBuilds": OTHER_BUILDS.get(export_format),
        "files": dict(sorted(packages.items())),
    }
    (target / "manifest.json").write_text(
        json.dumps(manifest, ensure_ascii=False, indent=2) + "\n", encoding="utf-8"
    )
    return target


def parse_builds(values: Sequence[str]) -> Dict[str, str]:
    builds = {}
    for value in values:
        export_format, separator, build = value.partition("=")
        if not separator or export_format not in LINES:
            raise ExportError(f"--build expects FORMAT=BUILD with a known format: {value}")
        builds[export_format] = build
    return builds


def refresh(export_format: str, build: str, work: Path) -> None:
    schemas = collect(export(build, work))
    target = publish(export_format, build, schemas)
    print(f"{export_format}: {build}, {len(schemas)} schemas -> {target.relative_to(REPO_ROOT)}")


def main(argv: Optional[Sequence[str]] = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("formats", nargs="+", choices=sorted(LINES))
    parser.add_argument("--build", action="append", default=[], metavar="FORMAT=BUILD")
    parser.add_argument("--work", type=Path, help="keep work files in this new directory")
    args = parser.parse_args(argv)
    try:
        builds = parse_builds(args.build)
        for export_format in args.formats:
            build = builds.get(export_format) or newest_build(LINES[export_format])
            if args.work:
                work = args.work / build
                work.mkdir(parents=True)
                refresh(export_format, build, work)
            else:
                with tempfile.TemporaryDirectory(prefix="unica-xdto-") as temporary:
                    refresh(export_format, build, Path(temporary))
    except ExportError as error:
        print(f"export-platform-xdto: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
