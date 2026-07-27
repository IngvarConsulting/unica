# SDD ledger — plan: docs/superpowers/plans/2026-07-26-versioned-format-adapter-boundary.md
Task 1: dispatched (base e30fe037, implementer 019f9e7d-717d-7981-be82-f329e72c96ca)
Task 1: plan ruling — user approved hmac/uuid workspace dependencies; handwritten HMAC/UUID must be removed.
Task 1: fix round 1 started (base a071fc8).
Task 1: reviewer finding deferred by user to Task 4 — navigation property/selectors still use String during transitional compile slices; Task 4 must replace them with closed SemanticPropertyId.
Task 1: fix round 2 started (base 5146448a) — relative-path rejection and compile-fail guard.
Task 1: fix round 2/5 (2 addressed, 0 open; commit 5146448a..cf73bb2c).
Task 1: complete (commits e30fe037..cf73bb2c, review clean; closed-ID key integration assigned to Task 4 by user ruling).
Task 2: dispatched (base cf73bb2c, implementer 019f9eca-42e1-79c0-8182-e5e0f050a827).
Task 2: review failed — owner API leak; duplicate host profile/owner parsing; lost source-set kind validation; ReadPort ignores query; registry/owner coverage loss.
Task 2: fix round 1 started (base 205e69ed).
Task 2: fix round 1/5 (2 addressed, 3 findings/open groups remain; commit 205e69ed..75a5941a).
Task 2: fix round 2 started (base 75a5941a) — all-owner kind validation, full regression inventory, neutral core vocabulary, SourceContext-only ports.
Task 2: fix round 2/5 (4 addressed, 1 new Important open; commit 75a5941a..4be1314c).
Task 2: fix round 3 started (base 4be1314c) — restore exact artifact-family overwrite protection without native core vocabulary.
Task 2: fix round 3/5 (original finding addressed, 1 new Important open; commit 4be1314c..d952d4f7).
Task 2: fix round 4 started (base d952d4f7) — Windows reparse-point protection.
Task 2: fix round 4/5 (1 addressed, 0 open; commit d952d4f7..20cd5be9).
Task 2: complete (commits cf73bb2c..20cd5be9, review clean).
Task 3: dispatched (base 20cd5be9, implementer 019fa18b-f1eb-79b3-869d-509ba21dfa9c).
Task 3: review failed — capture/probe/read recapture; transport/path-shaped application command; weakened host resolver tests; target identity missing from cache lookup; malformed target bypasses structured unavailable.
Task 3: fix round 1 started (base 70855a6a).
Task 3: fix round 1/5 (5 addressed, 1 new Important open; commit 70855a6a..041839c0).
Task 3: fix round 2 started (base 041839c0) — strict cursor selection decoding and authenticated canonical selection.
Task 3: fix round 2/5 (finding not addressed; production Value collapses duplicate keys; preflight ordering regression; commit 041839c0..281b098b).
Task 3: user ruling — public cursor changes from JSON object to opaque base64url string; backward compatibility intentionally broken.
Task 3: fix round 3 started (base 281b098b).
Task 3: fix round 3/5 (cursor finding addressed, 0 open; commit 281b098b..1296e827).
Task 3: complete (commits 20cd5be9..1296e827, review clean; cursor contract changed to opaque base64url string by user ruling).
Task 4: dispatched (base 1296e827, implementer 019fa1d3-da56-73f2-943a-c95d865a78e3).
Task 4: review failed — incomplete object-kind vocabulary; bypassable property invariants/no core ID-type contract; partial nodes can yield ready; partial continuations lose diagnostics; filtered properties leave dangling facet IDs.
Task 4: fix round 1 started (base fcd6ad8d).
Task 4: fix round 1 review failed — type-set target remains arbitrary raw String/native escape; nested recursive values lose semantic types during serde round-trip; CoverageState::Unknown can remain ready.
Task 4: fix round 2 started (base 650acba4).
Task 4: fix round 2 review failed — recursive JSON deserialization via serde_json::Value collapses duplicate keys instead of rejecting ambiguous input.
Task 4: fix round 3 started (base c72c1c90).
Task 4: fix round 3 review approved — strict duplicate-key rejection closes recursive JSON ambiguity; no remaining scoped findings.
Task 4: complete at cbe3fd28.
Task 5: started.

## Task 5 completion (2026-07-27)

- Status: complete.
- Implementation commit: `2d5d7aa7d20a056c3124dca6f11b0c40849d9c6d`.
- Scoped validation: `cargo test -p unica-adapter-platform-xml --test legacy_parity --test unmapped_fact` passed (4 tests, 0 failures, no warnings).
- Detailed evidence and parity inventory: `task-5-report.md`.
