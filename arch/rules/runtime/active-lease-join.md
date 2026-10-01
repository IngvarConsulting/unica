---
id: INV.APP.RUNTIME-ACTIVE-LEASE-JOIN
check:
  - crates/unica-coder/src/infrastructure/runtime_jobs.rs::runtime_shared_work_joins_only_the_exact_active_resource_and_lease
  - crates/unica-coder/src/infrastructure/runtime_jobs.rs::runtime_shared_work_separates_physical_resources_and_distinct_v4_leases
  - crates/unica-coder/src/infrastructure/runtime_jobs.rs::stale_runtime_shared_work_authority_starts_no_producer_after_lock_replacement
  - crates/unica-coder/src/infrastructure/runtime_jobs.rs::replaced_jobs_directory_between_lifecycle_lock_and_admission_starts_no_producer
  - crates/unica-coder/src/infrastructure/runtime_jobs.rs::symlinked_jobs_directory_between_lifecycle_lock_and_admission_starts_no_producer
---

# Подключение к runtime требует того же активного владельца ресурса

Подключение к общей работе runtime проверяет физическую идентичность каталога
заданий и его `active.lock`, а также UUIDv4 внутри этой блокировки.
Один и тот же ресурс с тем же UUID использует общего исполнителя. Другой
ресурс или новый UUID создаёт отдельную работу по правилам
[SharedWork](shared-work.md).

Замена каталога заданий, перенаправление его ссылкой или смена блокировки
между получением права и подключением не запускает исполнителя со старым
правом. Ключ подключения формируется внутри проверки владения, а не из
аргументов операции, и вызывающий код его не получает.
