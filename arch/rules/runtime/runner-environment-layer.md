---
id: INV.RUNTIME.RUNNER-ENVIRONMENT-LAYER
check:
  - crates/unica-coder/src/infrastructure/daemon/v13_infobase_exports.rs::runner_environment_refusals_name_the_layer_that_refused
  - crates/unica-coder/src/infrastructure/daemon/v13_infobase_exports.rs::an_absent_bundled_runner_names_the_missing_engine_in_data
---

# Отказ среды раннера называет отказавший слой

Отказ `environment_unavailable` раннера отвечает `provider_unavailable`
с уточнением по слою и `data` вида `{layer, runnerCode}`:

- агент конфигуратора не принял пользователя ИБ — `credentials_rejected`,
  слой `infobase_user`;
- точка входа агента не ответила, не прошла SSH-рукопожатие или назвалась
  чужим ключом хоста — `endpoint_unreachable`, слой `agent_endpoint`;
- прочая среда раннера — `provider_absent`, слой `runner_environment`.

Отсутствие самого поставляемого раннера — [отказ «нет движка»](../mcp/missing-engine-refusal.md),
а не слой среды. Сообщение начинается шагом проверки настройки для слоя,
затем передаёт текст раннера после общей редакции секретов.

Раннер 0.14 не различает эти слои кодом, и Unica опознаёт их по
устойчивой части собственного текста `AgentError` раннера. Нераспознанный
текст отвечает слоем `runner_environment`, а не отказом учётных данных.
Опознание по тексту временное: поле слоя запрошено у раннера
(IngvarConsulting/v8-runner-rust#485), и с ним маркеры заменяются чтением поля.
