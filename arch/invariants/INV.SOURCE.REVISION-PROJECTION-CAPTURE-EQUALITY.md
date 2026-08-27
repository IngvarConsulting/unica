---
id: INV.SOURCE.REVISION-PROJECTION-CAPTURE-EQUALITY
status: active
governs: product
decision: DEC.2026-08-27.ACTOR-REVISION-ARTIFACT-POLICY-SLICE
check: crates/unica-coder/src/infrastructure/source_revision.rs::projected_revision_artifacts_equal_retained_postpublication_capture
scope: [app, cache, platform, source]
---

# Projected revision воспроизводится retained capture

Каждый staged content postimage получает ту же классификацию, manifest kind и
digest, что две финальные retained capture. Staged presence или ignored path
отклоняется до публикации; успешный кандидат воспроизводится следующим
admission и после rebuild actor.
