# Native HTML template page

`native-ru.html` is the exact output of 1C 8.3.27.2074 (`ibcmd`) for the
`template-add-html-document` checkpoint. Two isolated databases each completed
import/apply, check and export; their page bytes were identical. The source
page differed only by the absent three-byte UTF-8 BOM.

No bytes were normalized. The fixture keeps the BOM, one internal LF and no
terminal newline. `manifest.json` records the native binary, installation,
source/output and original report hashes. Raw local evidence was retained in
`/tmp/unica-v014-native-nine.QipqCZ/evidence/template-add-html-document/`;
round 1 imports `input/source`, and round 2 imports round 1's export into a
new database. Both use `infobase create --import=<source> --apply --force`,
`config check --force`, then `config export <output>` with separate private
db/data/temp/users/session/log directories.

This fixture checks newly generated pages. Existing authored HTML remains
opaque UTF-8 content and must retain its bytes when metadata properties change.
