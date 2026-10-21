---
id: INV.SOURCE.BOUND-PREIMAGES
check:
  - crates/unica-coder/tests/platform/code_patch_composition.rs::staged_code_preserves_module_owner_support_format_and_preimage_guards
  - crates/unica-coder/tests/platform/code_patch_composition.rs::staged_code_reuses_actor_revision_and_race_fences
  - crates/unica-coder/tests/platform/code_patch_composition.rs::code_patch_rolls_back_if_owner_descriptor_changes_before_commit
gap: https://github.com/IngvarConsulting/unica/issues/954
---

# Изменение проверяет исходные данные перед публикацией

Правило действует для создания и изменения конфигурации, СКД, формы,
объекта метаданных, роли и подсистемы; создания расширения, внешней обработки,
внешнего отчёта и табличного документа; заимствования объекта и вставки
перехватчика в расширение; правки BSL и командного интерфейса.

Байты владельца и использованных зависимостей либо проверенное отсутствие
создаваемого файла сверяются перед публикацией. Изменившийся вход вызывает
отказ без перезаписи чужого результата.

Для правки BSL к входам относятся дескриптор владельца модуля и сам модуль.
Изменение любого из них после планирования отклоняет подготовленную правку.

Проверки охватывают планировщик BSL и публикацию через актора. Покрытие
остальных перечисленных семейств остаётся разрывом #954.
