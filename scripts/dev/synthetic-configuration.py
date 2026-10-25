#!/usr/bin/env python3
"""Generate a synthetic 1C configuration of a measured shape for capacity runs.

The shape (object counts and BSL size quantiles per module role) comes from a
profile such as ``synthetic-configuration-ut11.json``; the objects are clones
of the acceptance fixture ``tests/fixtures/acceptance/workspace`` with new
names and identifiers, and module text is generated. Nothing from the measured
configuration is copied. The result is a Git repository with a
``v8project.yaml`` workspace, ready for ``unica`` and ``git grep``; it is not
meant for loading into the platform (references to absent objects remain).

    synthetic-configuration.py --out DIR [--profile FILE] [--scale 1.0] [--seed 1119]

Every 97th module carries ``СинтетическаяМеткаПоиска`` so that a lexical
search has hits spread over the whole corpus.
"""

from __future__ import annotations

import argparse
import json
import random
import re
import shutil
import subprocess
import sys
import uuid
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]
TEMPLATE = REPO / "tests/fixtures/acceptance/workspace/src"
MARKER = "СинтетическаяМеткаПоиска"
BOM = "\ufeff"
# Written into every generated corpus; only such a directory is replaced.
SENTINEL = ".synthetic-configuration"
UUID = re.compile(r"[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}")
TEMPLATES = {
    "CommonModule": ("CommonModules", "GoogleПереводчик"),
    "Catalog": ("Catalogs", "Валюты"),
    "Document": ("Documents", "АктОбУничтоженииПерсональныхДанных"),
    "InformationRegister": ("InformationRegisters", "АдминистративнаяИерархия"),
    "Report": ("Reports", "АнализВерсийОбъектов"),
}
PREFIX = {
    "CommonModule": "ОбщийМодуль",
    "Catalog": "Справочник",
    "Document": "Документ",
    "InformationRegister": "Регистр",
    "Report": "Отчет",
}
WORDS = (
    "Номенклатура Контрагент Организация Склад Количество Сумма Цена Валюта "
    "Документ Строка Таблица Запрос Результат Выборка Параметры Отбор Период "
    "Партнер Соглашение Договор Заказ Реализация Поступление Остаток Движения"
).split()


def sample_size(rng: random.Random, points: list[float], quantiles: list[int]) -> int:
    q = rng.random()
    for (lo_q, hi_q), (lo, hi) in zip(
        zip(points, points[1:]), zip(quantiles, quantiles[1:])  # noqa: RUF007 - Python 3.9
    ):
        if q <= hi_q:
            share = 0 if hi_q == lo_q else (q - lo_q) / (hi_q - lo_q)
            return int(lo + (hi - lo) * share)
    return quantiles[-1]


def role_sizes(rng, profile, role: str, count: int, scale: float) -> list[int]:
    spec = profile["modules"][role]
    sizes = [
        max(3, sample_size(rng, profile["quantile_points"], spec["quantiles"]))
        for _ in range(count)
    ]
    # The role's measured bytes, scaled: when fewer modules of a role are
    # generated than measured, they are larger, so the corpus keeps its volume.
    target = spec["bytes"] * scale
    factor = target / max(1, sum(sizes))
    return [max(3, int(size * factor)) for size in sizes]


def module_text(rng: random.Random, size: int, marked: bool) -> bytes:
    """Whole functions up to about `size` bytes: the module always parses."""
    lines = ["#Область ПрограммныйИнтерфейс", ""]
    if marked:
        lines.append(f"// {MARKER}")
    body = sum(len(line.encode()) + 1 for line in lines) + 3
    index = 0
    while body < size:
        name = "".join(rng.choice(WORDS) for _ in range(2)) + str(index)
        words = " ".join(rng.choice(WORDS) for _ in range(6))
        block = [
            f"Функция {name}(Параметры) Экспорт",
            f"\t// {words}",
            "\tЗапрос = Новый Запрос;",
            f"\tЗапрос.Текст = \"ВЫБРАТЬ {rng.choice(WORDS)}.Ссылка ИЗ Справочник.{rng.choice(WORDS)} КАК {rng.choice(WORDS)}\";",
            "\tРезультат = Запрос.Выполнить().Выбрать();",
            f"\tВозврат Параметры.{rng.choice(WORDS)};",
            "КонецФункции",
            "",
        ]
        lines.extend(block)
        body += sum(len(line.encode()) + 1 for line in block)
        index += 1
    lines.append("#КонецОбласти")
    return (BOM + "\n".join(lines) + "\n").encode()


def fresh_uuids(text: str, salt: str) -> str:
    return UUID.sub(lambda m: str(uuid.uuid5(uuid.NAMESPACE_URL, salt + m.group(0))), text)


def clone(kind: str, name: str, src: Path) -> None:
    directory, template = TEMPLATES[kind]
    source = TEMPLATE / directory
    for path in [source / f"{template}.xml", *sorted((source / template).rglob("*"))]:
        if not path.exists() or path.is_dir() or path.suffix == ".bsl":
            continue
        relative = path.relative_to(source)
        target = src / directory / Path(*[part.replace(template, name) for part in relative.parts])
        target.parent.mkdir(parents=True, exist_ok=True)
        text = path.read_text(encoding="utf-8-sig")
        # Whole name only: `СпособыУстановкиКурсаВалюты` keeps its name.
        text = re.sub(rf"(?<!\w){re.escape(template)}(?!\w)", name, text)
        target.write_text(BOM + fresh_uuids(text, name), encoding="utf-8")


def add_forms(src: Path, name: str, forms: int) -> list[Path]:
    root = src / "Catalogs"
    descriptor = root / f"{name}.xml"
    text = descriptor.read_text(encoding="utf-8-sig")
    base_form = root / name / "Forms" / "ФормаЭлемента.xml"
    base_body = root / name / "Forms" / "ФормаЭлемента" / "Ext" / "Form.xml"
    modules = []
    entries = []
    for index in range(forms):
        form = "ФормаЭлемента" if index == 0 else ("ФормаСписка" if index == 1 else f"Форма{index}")
        if index >= 2:
            form_text = base_form.read_text(encoding="utf-8-sig").replace("ФормаЭлемента", form)
            (root / name / "Forms" / f"{form}.xml").write_text(
                BOM + fresh_uuids(form_text, name + form), encoding="utf-8"
            )
            body = root / name / "Forms" / form / "Ext" / "Form.xml"
            body.parent.mkdir(parents=True, exist_ok=True)
            body.write_text(
                BOM + fresh_uuids(base_body.read_text(encoding="utf-8-sig"), name + form),
                encoding="utf-8",
            )
            entries.append(f"<Form>{form}</Form>")
        modules.append(root / name / "Forms" / form / "Ext" / "Form" / "Module.bsl")
    if entries:
        text = text.replace("<Form>ФормаСписка</Form>", "<Form>ФормаСписка</Form>" + "".join(entries), 1)
        descriptor.write_text(BOM + text, encoding="utf-8")
    return modules


def generate(out: Path, profile: dict, scale: float, seed: int) -> dict:
    rng = random.Random(seed)
    if out.exists() and any(out.iterdir()):
        if not (out / SENTINEL).is_file():
            raise SystemExit(f"{out} is not empty and was not made by this script; choose another --out")
        shutil.rmtree(out)
    src = out / "src"
    src.mkdir(parents=True)
    (out / SENTINEL).write_text(f"{profile['name']} scale={scale} seed={seed}\n", encoding="utf-8")
    (out / "v8project.yaml").write_text(
        "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
        encoding="utf-8",
    )
    for extra in ["Languages", "Ext"]:
        shutil.copytree(TEMPLATE / extra, src / extra)
    counts = {kind: max(1, round(count * scale)) for kind, count in profile["objects"].items()}
    names = {kind: [f"{PREFIX[kind]}{i:05d}" for i in range(count)] for kind, count in counts.items()}
    for kind, items in names.items():
        for name in items:
            clone(kind, name, src)
    form_total = round(profile["modules"]["form"]["files"] * scale)
    per_catalog = [form_total // counts["Catalog"]] * counts["Catalog"]
    for i in range(form_total % counts["Catalog"]):
        per_catalog[i] += 1
    modules: dict[str, list[Path]] = {"common": [], "form": [], "object": [], "manager": [], "other": []}
    for name in names["CommonModule"]:
        modules["common"].append(src / "CommonModules" / name / "Ext" / "Module.bsl")
    for name, forms in zip(names["Catalog"], per_catalog):
        modules["form"].extend(add_forms(src, name, max(2, forms))[: forms or 0])
    for kind, directory in [("Catalog", "Catalogs"), ("Document", "Documents"), ("Report", "Reports")]:
        for name in names[kind]:
            modules["object"].append(src / directory / name / "Ext" / "ObjectModule.bsl")
    for kind, directory in [
        ("Catalog", "Catalogs"),
        ("Document", "Documents"),
        ("InformationRegister", "InformationRegisters"),
        ("Report", "Reports"),
    ]:
        for name in names[kind]:
            modules["manager"].append(src / directory / name / "Ext" / "ManagerModule.bsl")
    for name in names["InformationRegister"]:
        modules["other"].append(src / "InformationRegisters" / name / "Ext" / "RecordSetModule.bsl")
    written = {}
    ordinal = 0
    for role, paths in modules.items():
        sizes = role_sizes(rng, profile, role, len(paths), scale)
        total = 0
        for path, size in zip(paths, sizes):
            path.parent.mkdir(parents=True, exist_ok=True)
            data = module_text(rng, size, ordinal % 97 == 0)
            path.write_bytes(data)
            total += len(data)
            ordinal += 1
        written[role] = {"files": len(paths), "bytes": total}
    configuration = (TEMPLATE / "Configuration.xml").read_text(encoding="utf-8-sig")
    children = "".join(
        f"<{kind}>{name}</{kind}>" for kind in counts for name in names[kind]
    )
    configuration = re.sub(
        r"(<ChildObjects>\s*<Language>Русский</Language>\s*<Language>English</Language>).*?(</ChildObjects>)",
        lambda m: m.group(1) + children + m.group(2),
        configuration,
        count=1,
        flags=re.DOTALL,
    )
    (src / "Configuration.xml").write_text(BOM + configuration, encoding="utf-8")
    subprocess.run(["git", "init", "-q", str(out)], check=True)
    subprocess.run(["git", "-C", str(out), "add", "-A"], check=True)
    subprocess.run(
        ["git", "-C", str(out), "-c", "user.name=synthetic", "-c", "user.email=synthetic@example.invalid",
         "commit", "-q", "--no-gpg-sign", "-m", "synthetic configuration"],
        check=True,
    )
    files = [p for p in src.rglob("*") if p.is_file()]
    return {
        "profile": profile["name"],
        "scale": scale,
        "seed": seed,
        "objects": counts,
        "modules": written,
        "bsl": {"files": sum(v["files"] for v in written.values()), "bytes": sum(v["bytes"] for v in written.values())},
        "xml": {"files": sum(1 for p in files if p.suffix == ".xml"), "bytes": sum(p.stat().st_size for p in files if p.suffix == ".xml")},
    }


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--profile", type=Path, default=Path(__file__).with_name("synthetic-configuration-ut11.json"))
    parser.add_argument("--scale", type=float, default=1.0)
    parser.add_argument("--seed", type=int, default=1119)
    args = parser.parse_args(argv)
    profile = json.loads(args.profile.read_text(encoding="utf-8"))
    print(json.dumps(generate(args.out, profile, args.scale, args.seed), ensure_ascii=False, indent=1))
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
