# Стандартные реквизиты оборотного регистра

Буквальный XML-экспорт `ibcmd config export` платформы 8.3.27.2214 из отдельной
временной базы. Вход: platform-фикстура `workspace-register-dimensions`;
у регистра Остатки задан Turnovers, удалён неприменимый RecordType, четырём
остальным StandardAttributes заданы подсказки `Corpus attribute`. Вход принят
через `ibcmd infobase create --import=... --apply --force`, затем выгружен.
Экспорт сохраняет Active, LineNumber, Recorder, Period и не содержит RecordType.
Сохранены BOM, CRLF, UUID и namespaces платформы; ConfigDumpInfo.xml исключён.

Это самостоятельный платформенный контрпример для #1603. Он не является
исходной выгрузкой УТ 11.6.1.61 из issue и не подтверждает полный закреплённый
гейт 8.3.27.2074. Пользовательская база не использовалась.

Версия ibcmd проверена через `--version`: `8.3.27.2214`; SHA256 бинаря:
`1a35bba7883a7fdf9a8678392e4b1520df452995ce005ff225f94594e70cfe25`.
