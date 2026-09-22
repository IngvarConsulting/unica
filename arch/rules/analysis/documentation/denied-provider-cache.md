---
id: INV.APP.DOCUMENTATION-DENIED-CACHE
check:
  - crates/unica-coder/src/infrastructure/standards_documentation.rs::policy_deny_does_not_answer_from_the_search_cache
---

# Запрещённый поставщик стандартов не отвечает из кеша

Если действующая политика запрещает поставщика `v8std` (`network = deny`),
поиск возвращает для него `policy-denied` без результатов и без сетевого
обращения. Уже полученный ответ в локальном кеше не отменяет запрет,
даже если срок годности ответа ещё не истёк.

Правило относится к поиску стандартов у `v8std`. Вычисление разрешения
из общей и индивидуальной настроек описано в [политике сети](network-policy.md).
