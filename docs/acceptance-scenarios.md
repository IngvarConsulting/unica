# Реестр приёмочных сценариев

Реестр переехал на сайт проекта:
[ingvarconsulting.github.io/unica/acceptance-scenarios.html](https://ingvarconsulting.github.io/unica/acceptance-scenarios.html).
Сценарий открывается по прежнему номеру как якорь страницы, например
[`#S047`](https://ingvarconsulting.github.io/unica/acceptance-scenarios.html#S047).

Реестр собирается из корпуса `tests/fixtures/acceptance/` при сборке сайта и
в репозитории не хранится: его счётчики и списки покрытия меняет каждый новый
сценарий, и общий файл ссорил бы между собой любые два таких изменения.
Локально его печатает `python scripts/ci/render-acceptance-registry.py`.
Как добавить сценарий, описано в
[CONTRIBUTING.md](../CONTRIBUTING.md#приёмочные-сценарии-поверхности).
