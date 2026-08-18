---
id: INV.PKG.TWO-HOSTS-ONE-TREE
status: active
decision: DEC.2026-08-18.CARRIED-RULES
check: tests/ci/test_package_unica_plugin.py
scope: [pkg]
---

# Один каталог плагина обслуживает двух хостов

Codex и Claude Code получают один и тот же каталог; от хоста зависят только манифесты, и оба
несут одну версию. Публичный бинарник runtime называется `unica`.
