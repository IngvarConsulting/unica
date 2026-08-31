---
id: INV.SOURCE.RETAINED-LOGICAL-PUBLICATION
status: active
governs: product
decision: DEC.2026-08-25.LOGICAL-READ-CORE-SLICE
check: crates/unica-coder/src/infrastructure/daemon/server.rs::hidden_v13_logical_publication_contract_is_complete
scope: [app, platform, product, source]
---

# Retained logical read публикуется после стабилизации под actor mutation lane

При unsupported platform fence initial capture и final confirmation каждого
выбранного source set выполняют по два равных descriptor-relative passes.
Каждый pass post-order повторно проверяет named identities и membership,
ограничивает entries, file/aggregate bytes и проверяет одну operation deadline
и cancellation. Semantic manifest и отдельная physical identity evidence
должны совпасть; physical identity не меняет byte-compatible semantic digest.
Stable operation выполняет четыре passes на source set. Три bounded attempts
ограничивают capture шестью, operation двенадцатью passes независимо от числа
logical nodes.

Final publication один раз захватывает mutation lane `WorkspaceActor`, под ней
проверяет все actor-issued source fences и выполняет все final retained
confirmations, после чего source I/O нет. Lane wait использует ту же абсолютную
120-секундную logical-read deadline и cancellation, не создавая новый budget.
Malformed View address и valid unknown source не захватывают revision и
возвращают соответственно typed `bad_value` и `provider_unavailable`; zero-fence
publication принимает только такой закрытый typed rejection, а успешный result
без admitted fence fail closed.
