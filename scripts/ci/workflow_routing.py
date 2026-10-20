"""Selection policy shared by the workflow, test runner and required gate."""

from __future__ import annotations

from collections.abc import Mapping
from typing import NamedTuple


RUST_JOBS = ("test-rust-platforms", "lint-rust-platforms")
CACHE_JOB = "warm-rust-cache"


class RustRoute(NamedTuple):
    required: bool
    warm_cache: bool


def rust_route(event_name: str, ref: str, flags: Mapping[str, bool], *, docs_only: bool) -> RustRoute:
    if event_name == "push" and ref == "refs/heads/main":
        return RustRoute(False, flags["toolchain_changed"] or flags["ci_changed"])
    if docs_only:
        return RustRoute(False, False)
    required = any(flags[name] for name in ("rust_changed", "platform_changed", "toolchain_changed", "ci_changed"))
    return RustRoute(required, False)


# Ворота конвейера плюс локальный `all` — все наборы, которыми управляет CI.
# Агентная приёмка требует локальной авторизации и намеренно исключена.
PROFILES = ("all", "pr", "queue", "main", "release", "large")
SIZES = ("small", "medium", "large")
# Профиль задаёт допущенные размеры; документационный маршрут дополнительно
# сужает Python, не меняя профиль и отбор Rust.
ADMITTED = {
    "all": SIZES,
    "pr": ("small",),
    "queue": ("small", "medium"),
    "main": ("small", "medium"),
    "release": SIZES,
    # Ночной прогон: только тяжёлый Rust-ярус; Python-наборов этого размера нет.
    "large": ("large",),
}

# Наборы Python: сначала проверки CI, потом инструментов разработчика.
# `--durations` показывает, какие проверки занимают время.
# Это размер набора по умолчанию; .config/python-sizes.toml выделяет medium
# внутри обоих наборов.
PYTHON_SUITES = (
    ("tests/ci", "small", ("--durations", "20")),
    ("tests/dev", "small", ("--durations", "20")),
)


def python_admitted(profile: str, *, docs_only: bool = False) -> tuple[str, ...]:
    if profile not in ADMITTED:
        raise ValueError(f"профиль {profile!r} для Python не описан")
    if docs_only and profile not in {"pr", "queue"}:
        raise ValueError("docs-only допускается только для PR и очереди")
    return ("small",) if docs_only else ADMITTED[profile]


# Наборы, которые в CI делятся на полосы по размеру: одна джоба на размер.
# `tests/ci` одним процессом шёл в очереди шесть с половиной минут, из них
# четыре — несколько `medium`-тестов; полоса `medium` идёт своей джобой.
LANED_SUITES = ("tests/ci",)


def python_suite_names() -> tuple[str, ...]:
    return tuple(suite for suite, _, _ in PYTHON_SUITES)


def python_slug(suite: str, lane: str) -> str:
    """Имя джобы и артефакта: основание набора и полоса, если она есть."""
    base = suite.rstrip("/").split("/")[-1]
    return f"{base}-{lane}" if lane else base


def python_matrix(profile: str, *, docs_only: bool = False) -> list[dict]:
    """Матрица джоб Python для ворот: набор на джобу, полосатый набор — по размеру.

    Считается швом, а не пишется в workflow: список наборов и допуски ворот
    живут здесь, и workflow берёт матрицу готовой.
    """
    admitted = python_admitted(profile, docs_only=docs_only)
    entries: list[dict] = []
    for suite, size, _ in PYTHON_SUITES:
        if size not in admitted:
            continue
        lanes = [lane for lane in SIZES if lane in admitted] if suite in LANED_SUITES or docs_only else [""]
        for lane in lanes:
            entries.append({"suite": suite, "lane": lane, "slug": python_slug(suite, lane)})
    return entries
