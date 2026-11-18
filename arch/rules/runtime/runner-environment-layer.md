---
id: INV.RUNTIME.RUNNER-ENVIRONMENT-LAYER
---

# Отказ среды раннера называет отказавший слой

Отказ `environment_unavailable` раннера отвечает `provider_unavailable`
с уточнением по слою и `data` вида `{layer, runnerCode}`:

- агент конфигуратора не принял пользователя или пароль ИБ либо не нашёл
  лицензии — `credentials_rejected`, слой `infobase_user`;
- точка входа агента не ответила, не прошла SSH-рукопожатие или назвалась
  чужим ключом хоста — `endpoint_unreachable`, слой `agent_endpoint`;
- прочая среда раннера — `provider_absent`, слой `runner_environment`.

Отсутствие самого поставляемого раннера — [отказ «нет движка»](../mcp/missing-engine-refusal.md),
а не слой среды. Сообщение начинается шагом проверки настройки для слоя,
затем передаёт текст раннера после общей редакции секретов.

Слой берётся из поля `error.layer` отказа раннера: `infobase_user`,
`agent_endpoint`; `agent_start` и `platform` отвечают слоем
`runner_environment`, как и отказ без поля. Для раннеров без поля слой
один цикл выпуска опознаётся по устойчивой части текста ошибки агента.
