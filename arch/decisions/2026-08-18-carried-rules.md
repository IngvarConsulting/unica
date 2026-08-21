---
id: DEC.2026-08-18.CARRIED-RULES
status: active
governs: process
realized: tests/arch/test_registry.py::test_every_rule_names_a_check_that_exists
supersedes: []
superseded-by: null
establishes: [INV.APP.DEFERRED-MANIFEST, INV.APP.DEFERRED-READ, INV.APP.DIAGNOSTIC-PROVIDERS, INV.APP.DOCUMENTATION-CONTAINER-FINGERPRINT, INV.APP.DOCUMENTATION-GET, INV.APP.DOCUMENTATION-NETWORK-POLICY, INV.APP.DOCUMENTATION-SECTIONS, INV.APP.EVENT-BINDING, INV.APP.EVENT-SOURCE, INV.APP.META-FINDINGS, INV.APP.META-INFO-COVERAGE, INV.APP.META-OBSERVATION, INV.APP.NO-DIRECT-GIT, INV.APP.NO-SCRIPT-BACKEND, INV.APP.OUTLINE-SOURCE, INV.APP.PARTIAL-FALLBACK, INV.APP.SEARCH-EXPANSIONS, INV.APP.SEARCH-TIE-ORDER, INV.APP.SKILL-REFERENCE-REACHABILITY, INV.APP.SKILL-SCRIPT-FIXTURES, INV.APP.SUPPORT-STATE, INV.CACHE.GENERATION-CUTOVER, INV.CACHE.OVERRIDE-PRIORITY, INV.CACHE.PERSISTED-STALENESS, INV.CACHE.RLM-REVISION, INV.CACHE.WORKSPACE-ROOT, INV.CACHE.WORKTREE-ISOLATION, INV.SURFACE.CODE-SEARCH-ROLES, INV.SURFACE.DCS-NAMING, INV.SURFACE.DIAGNOSTIC-TARGET, INV.SURFACE.EXECUTABLE-SKILL-EXAMPLES, INV.SURFACE.META-TOOLSET, INV.SURFACE.NO-ADAPTER-TARGETS, INV.SURFACE.NO-RAW-ADAPTER-ARGS, INV.SURFACE.PACKAGED-REFERENCES, INV.SURFACE.PROJECT-READINESS, INV.SURFACE.ROLE-EDIT-LOGICAL, INV.SURFACE.SKILL-NO-SCRIPT-ROUTE, INV.SURFACE.SKILL-PREVIEW-GUIDANCE, INV.SURFACE.SKILL-ROUTING, INV.SURFACE.SOURCE-SKILL-ROUTING, INV.SURFACE.SOURCE-TOOL-SPECS, INV.SURFACE.TOOL-VERSION-SOURCE, INV.SURFACE.XDTO-LOGICAL-TARGET, INV.WIRE.DATA-DRIVEN-TOOL-LIST, INV.WIRE.DIRECT-FIRST-LIFECYCLE, INV.WIRE.GUARANTEED-VERSIONS, INV.WIRE.PINNED-FALLBACK-VERSION, INV.WIRE.SDK-INITIALIZE, INV.WIRE.SDK-TRANSPORT]
---

# Неизменённое обязательство переносится вместе с точной проверкой

**Решение.** В этот реестр попадает только существовавшее до перехода на v2
обязательство, которое не меняет поведение и полностью фальсифицируется одной
названной живой проверкой. Сохранившаяся публичная идентичность переносится как
часть обязательства; удалённая идентичность не переносится. Если проверка
доказывает лишь часть составного правила v1, запись утверждает только эту часть,
а непокрытый остаток перенесённым не объявляется.

**Почему.** Сам переход между реестрами не удаляет действующее поведение и не
создаёт новое. Связь с точной проверкой отделяет сохранённое обязательство от
похожего по названию, но уже не доказанного текста.

**Цена.** Составное правило может распасться на несколько узких записей; часть,
для которой нет точного фальсификатора, остаётся явно неперенесённой.
