#!/usr/bin/env python3
"""Check the production component graph, including target and test edges."""
from __future__ import annotations

import argparse
from pathlib import Path
import re
import sys
import tomllib

sys.path.insert(0, str(Path(__file__).resolve().parent))
import crate_declarations  # noqa: E402

# The kinds below that only restrict a crate (family, coordinator) are read
# from each crate's `[package.metadata.unica] component`: a new crate states
# its kind and does not edit this guard. Each keeps a closed floor of the
# crates it holds today: a crate cannot lift a restriction by declaring a
# looser kind, only an edit of the floor here can. Lists that permit (shared
# contracts others may import, a family's semantic dependencies, closed edge
# sets) stay closed: a crate must not widen them by its own declaration.
FAMILY_FLOOR = frozenset({"metadata", "forms", "roles", "dcs", "xdto", "templates", "bsl"})
SEMANTIC_DEPENDENCIES = {
    "metadata": frozenset(), "forms": frozenset({"metadata"}),
    "roles": frozenset({"metadata"}), "dcs": frozenset({"metadata", "templates"}),
    "xdto": frozenset(), "templates": frozenset({"metadata"}),
    "bsl": frozenset({"metadata"}),
}
def _declared(component: str, declarations: dict | None = None) -> frozenset[str]:
    declarations = crate_declarations.repository() if declarations is None else declarations
    return frozenset(
        name.removeprefix("unica-")
        for name in crate_declarations.packages(declarations, component=component)
    )


ABSTRACTIONS = frozenset({"address", "source-contract", "operation-contract", "work-control", "refusal", "execution-contract", "xml-format", "redaction", "code-model"})
# Named apart from ABSTRACTIONS so that a new shared contract does not widen
# what the publication owner may import without a review of this list.
WORKSPACE_DEPENDENCIES = frozenset({"address", "source-contract", "operation-contract", "work-control", "refusal", "execution-contract", "xml-format", "redaction"})
COORDINATOR_FLOOR = frozenset({
    "operations", "bindings", "navigation", "code-intelligence", "diagnostics", "documentation",
    "analysis-services", "application", "domain", "lsp",
})
# A family is ruled by its semantic dependencies, which only this guard grants.
FAMILIES = FAMILY_FLOOR | (_declared("family") & SEMANTIC_DEPENDENCIES.keys())
# Everything the rule against importing a port implementation applies to.
PORT_CALLERS = COORDINATOR_FLOOR | _declared("coordinator")


def use_declarations(root: Path) -> None:
    """Derive the restriction sets from the crates under `root`, the tree checked."""
    global FAMILIES, PORT_CALLERS
    declarations = crate_declarations.read(root)
    FAMILIES = FAMILY_FLOOR | (_declared("family", declarations) & SEMANTIC_DEPENDENCIES.keys())
    PORT_CALLERS = COORDINATOR_FLOOR | _declared("coordinator", declarations)
CODEC_DEPENDENCIES = {"xdto": frozenset({"metadata-designer"})}
# Families whose codec sits behind the family port (#1826). Product composition
# selects the implementation in one module; everyone else calls the trait.
PORTED_FAMILIES = frozenset({"dcs"})
CODEC_SELECTION = "crates/unica-coder/src/infrastructure/family_codecs.rs"
# The product composition: the lower layer, the daemon with its doors over it,
# and the package of the product binary. Only these depend on one another.
PRODUCT_COMPOSITION = frozenset({"unica-coder", "unica-daemon", "unica-product"})
# Metadata read code of the composition parses descriptors only through the
# family port `MetadataReadCodec`. Write code still names the codec, so
# `metadata` is not in PORTED_FAMILIES.
METADATA_READ_SOURCES = "crates/unica-coder/src/infrastructure/v13_read"


# What a crate declares as `component` under `[package.metadata.unica]`. The
# declaration must match what `edge_errors` applies to the crate by name, so a
# new crate states its place instead of falling outside every list unnoticed.
COMPONENT_KINDS = {
    "family": "semantic family with its semantic dependencies in SEMANTIC_DEPENDENCIES",
    "abstraction": "shared contract, listed in ABSTRACTIONS",
    "coordinator": "invokes ports without implementations; the declaration alone makes it one",
    "codec": "`unica-<family>-designer` representation codec",
    "adapter": "`unica-adapter-*` implementation of a port",
    "integration-tests": "`unica-*-integration-tests` cross-component checks",
    "product": "the product composition: unica-coder, unica-daemon and unica-product",
    "service": "any other runtime component; a closed dependency rule is added to edge_errors by name",
}


def required_component(package: str, declared: str | None = None) -> str:
    """The kind the rules of this guard give a package: by its name, or by
    `declared` where the declaration alone may restrict it."""
    name = package.removeprefix("unica-")
    if name in FAMILY_FLOOR or (declared == "family" and name in SEMANTIC_DEPENDENCIES):
        return "family"
    if name in ABSTRACTIONS:
        return "abstraction"
    if name in COORDINATOR_FLOOR or declared == "coordinator":
        return "coordinator"
    if name.endswith("-integration-tests"):
        return "integration-tests"
    if name.endswith("-designer") and name != "source-designer":
        return "codec"
    if name.startswith("adapter-"):
        return "adapter"
    if f"unica-{name}" in PRODUCT_COMPOSITION:
        return "product"
    return "service"


def declaration_errors(members: dict[str, tuple[Path, dict]]) -> list[str]:
    """A crate that names no kind would pass every rule above unchecked."""
    hint = "; ".join(f"{kind}: {meaning}" for kind, meaning in COMPONENT_KINDS.items())
    errors: list[str] = []
    for package, (_, document) in sorted(members.items()):
        declared = document["package"].get("metadata", {}).get("unica", {}).get("component")
        required = required_component(package, declared)
        if declared not in COMPONENT_KINDS:
            errors.append(f'{package}: declare component under [package.metadata.unica] in its Cargo.toml; kinds are {hint}')
        elif declared != required:
            errors.append(f"{package}: component {declared} needs the rules of this guard to treat it so, and they treat it as {required}; a floor in check-component-boundaries.py or a closed list of permissions there decides it, not the declaration")
    return errors


def edge_errors(owner: str, dependency: str, kind: str) -> list[str]:
    """All edge kinds matter: a dev backedge also rebuilds a monolithic target."""
    name = owner.removeprefix("unica-")
    target = dependency.removeprefix("unica-")
    prefix = f"{owner} {kind} -> {dependency}: "
    reasons: list[str] = []
    if dependency == "rmcp" and owner != "unica-daemon":
        reasons.append("the MCP SDK belongs to the product composition")
    if dependency in {"bsl-parser", "bsl-syntax", "parser", "syntax"} and owner != "unica-bsl-parser":
        reasons.append("upstream parser types belong to the parser boundary")
    if dependency in PRODUCT_COMPOSITION - {"unica-product"} and owner not in PRODUCT_COMPOSITION:
        reasons.append("a component or its unit target must not depend on the product composition")
    if dependency == "unica-product":
        reasons.append("nothing depends on the package that declares the product binary")
    if name in FAMILIES and dependency.startswith("unica-"):
        allowed = ABSTRACTIONS | frozenset({"operations"}) | SEMANTIC_DEPENDENCIES[name]
        if name == "bsl":
            allowed |= frozenset({"bsl-parser"})
        if target not in allowed:
            reasons.append("semantic families use contracts and their declared semantic dependencies")
    if name == "operations" and dependency.startswith("unica-") and target not in ABSTRACTIONS:
        reasons.append("generic operation orchestration must not import a family or backend")
    if name in PORT_CALLERS and dependency.startswith("unica-"):
        # `source-designer` is the shared Designer layout, not a family codec:
        # the LSP service finds the file of a module address through it.
        layout = name == "lsp" and target == "source-designer"
        if target.startswith("adapter-") or (target.endswith("-designer") and not layout):
            reasons.append("coordinators invoke ports without importing their concrete implementations")
    if name == "source-designer" and dependency.startswith("unica-") and target not in ABSTRACTIONS:
        reasons.append("source layout uses source contracts without concrete families")
    if name == "workspace" and dependency.startswith("unica-"):
        # A closed list: any other component is a consumer, codec or backend,
        # and an edge to it would let the publication owner depend on them.
        allowed = WORKSPACE_DEPENDENCIES | (frozenset({"process"}) if kind == "dev-dependencies" else frozenset())
        if target not in allowed:
            reasons.append("the workspace owner must not import consumers, codecs or analysis implementations")
    if name == "project-health" and dependency.startswith("unica-"):
        allowed = ABSTRACTIONS | frozenset({"workspace", "process", "source-designer"})
        if target not in allowed:
            reasons.append("project health observes source and resource facts without execution or family backends")
    if name.endswith("-designer") and name != "source-designer" and dependency.startswith("unica-"):
        family = name.removesuffix("-designer")
        allowed = ABSTRACTIONS | frozenset({"source-designer", "operations", "bsl-parser", family, "metadata"})
        allowed |= CODEC_DEPENDENCIES.get(family, frozenset())
        if target not in allowed:
            reasons.append("a representation codec consumes neutral ports and its own family")
    if name == "execution" and dependency.startswith("unica-"):
        allowed = ABSTRACTIONS | frozenset({"workspace", "process", "runner"})
        if target not in allowed:
            reasons.append("execution owns outcomes and invokes typed backend ports")
    if name == "daemon-protocol" and dependency.startswith("unica-"):
        # `lsp-contract` is the port the LSP lane calls, not a component: the
        # service behind it stays out of the daemon's reach.
        allowed = ABSTRACTIONS | frozenset({"execution", "workspace", "process", "bootstrap", "artifact-delivery", "lsp-contract"})
        if target not in allowed:
            reasons.append("the daemon protocol owns transport and identity without product dispatch or family backends")
    if name == "lsp-contract" and dependency.startswith("unica-") and target != "work-control":
        # The request's cancellation token is the one the transport cancels
        # and the service's work watches; nothing else is shared.
        reasons.append("the LSP contract is a leaf shared by the transport and the service")
    if name == "lsp" and dependency.startswith("unica-"):
        # The analysis that builds the answers (`code-model`, a shared
        # abstraction) stays with the product.
        allowed = ABSTRACTIONS | frozenset({"lsp-contract", "workspace", "bsl", "source-designer"})
        if target not in allowed:
            reasons.append("the LSP service reaches its transport only through the LSP contract")
    if name == "code-model" and dependency.startswith("unica-") and target != "work-control":
        reasons.append("the code model is plain data and parses nothing")
    if name == "bsl-parser" and dependency.startswith("unica-") and target not in {"work-control"}:
        reasons.append("the syntax boundary remains a leaf without family policy")
    return [prefix + reason for reason in reasons]


def cycle_errors(graph: dict[str, set[str]], owner: str) -> list[str]:
    """Report a cycle through `owner`, the shortest one per dependency edge.

    Every edge kind counts: Cargo admits a dev-dependency cycle, but it would
    still let the owner's consumers become its own dependencies."""
    errors: list[str] = []
    for start in sorted(graph.get(owner, ())):
        previous: dict[str, str | None] = {start: None}
        queue = [start]
        while queue and owner not in previous:
            node = queue.pop(0)
            for dependency in sorted(graph.get(node, ())):
                if dependency not in previous:
                    previous[dependency] = node
                    queue.append(dependency)
        if owner in previous:
            path = [owner]
            while path[-1] != start:
                path.append(previous[path[-1]])
            errors.append("component cycle: " + " -> ".join([owner, *reversed(path)]))
    return errors


def codec_selection_errors(path: str, text: str) -> list[str]:
    """Rust source outside a family's codec and its integration tests names the
    concrete codec only at the composition's selection point."""
    if path == CODEC_SELECTION:
        return []
    errors = []
    for family in sorted(PORTED_FAMILIES):
        if path.startswith((f"crates/unica-{family}-designer/", f"crates/unica-{family}-integration-tests/")):
            continue
        crate = f"unica_{family}_designer"
        for number, line in enumerate(text.splitlines(), 1):
            if re.search(rf"\b{crate}\b", line):
                errors.append(f"{path}:{number}: {crate} is selected only in {CODEC_SELECTION}; call the family port")
    return errors


def metadata_read_codec_errors(path: str, text: str) -> list[str]:
    """Metadata read code reaches the metadata codec only through its family port."""
    if not path.startswith(METADATA_READ_SOURCES):
        return []
    return [f"{path}:{number}: metadata read code parses descriptors through MetadataReadCodec"
            for number, line in enumerate(text.splitlines(), 1)
            if re.search(r"\bunica_metadata_designer\b", line)]


def manifests(root: Path) -> dict[str, tuple[Path, dict]]:
    document = tomllib.loads((root / "Cargo.toml").read_text())
    result: dict[str, tuple[Path, dict]] = {}
    # Как Cargo: `exclude` снимает каталог с членства, и он не компонент.
    excluded = {root / path for path in document["workspace"].get("exclude", [])}
    for pattern in document["workspace"]["members"]:
        candidates = sorted(root.glob(pattern))
        if not candidates:
            raise ValueError(f"unresolved member {pattern}")
        for directory in candidates:
            if directory in excluded:
                continue
            manifest = directory / "Cargo.toml"
            parsed = tomllib.loads(manifest.read_text())
            name = parsed["package"]["name"]
            if name in result:
                raise ValueError(f"duplicate member {name}")
            result[name] = (directory, parsed)
    return result


def check(root: Path) -> list[str]:
    root = root.resolve()
    use_declarations(root)
    members = manifests(root)
    paths = {directory.resolve(): name for name, (directory, _) in members.items()}
    workspace = tomllib.loads((root / "Cargo.toml").read_text())["workspace"]
    shared = workspace.get("dependencies", {})
    diagnostics: list[str] = []
    graph: dict[str, set[str]] = {owner: set() for owner in members}
    for owner, (directory, document) in sorted(members.items()):
        for table in (document, *document.get("target", {}).values()):
            for kind in ("dependencies", "build-dependencies", "dev-dependencies"):
                for alias, raw in table.get(kind, {}).items():
                    inherited = isinstance(raw, dict) and raw.get("workspace")
                    dependency = shared[alias] if inherited else raw
                    name = dependency.get("package", alias) if isinstance(dependency, dict) else alias
                    if isinstance(dependency, dict) and "path" in dependency:
                        dependency_root = root if inherited else directory
                        path = (dependency_root / dependency["path"]).resolve()
                        if path not in paths:
                            raise ValueError(f"{owner}: local dependency {alias} is not a workspace member")
                        if name != paths[path]:
                            raise ValueError(f"{owner}: dependency {alias} names {name}, path provides {paths[path]}")
                        graph[owner].add(name)
                    diagnostics.extend(edge_errors(owner, name, kind))
    diagnostics.extend(cycle_errors(graph, "unica-workspace"))
    for source in sorted((root / "crates").glob("*/**/*.rs")):
        relative = source.relative_to(root).as_posix()
        if "/target/" not in relative:
            text = source.read_text(encoding="utf-8")
            diagnostics.extend(codec_selection_errors(relative, text))
            diagnostics.extend(metadata_read_codec_errors(relative, text))
    return sorted(set(diagnostics))


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--repo-root", type=Path, default=Path(__file__).resolve().parents[2])
    arguments = parser.parse_args()
    try:
        diagnostics = declaration_errors(manifests(arguments.repo_root.resolve())) + check(arguments.repo_root)
    except (OSError, ValueError, KeyError, TypeError) as error:
        print(f"component boundary input error: {error}")
        return 1
    for diagnostic in diagnostics:
        print(diagnostic)
    return int(bool(diagnostics))


if __name__ == "__main__":
    raise SystemExit(main())
