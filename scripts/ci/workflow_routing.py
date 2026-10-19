"""Rust job policy shared by the workflow classifier and required gate."""

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
