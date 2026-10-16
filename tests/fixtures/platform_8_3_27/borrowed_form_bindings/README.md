# Borrowed form binding evidence — 8.3.27.2214

These are preserved results of the local platform experiment on 9 October 2026,
not a new run or synthetic platform output. The disposable infobase used then
was deleted during the user's cleanup. No current user infobase was modified.
The platform was Windows x64 1C 8.3.27.2214, XML format 2.20.

Steps of the historical experiment:
1. Dump the parent ERP configuration using Designer /DumpConfigToFiles.
2. Create a disposable file infobase, load its parent with /LoadCfg and /UpdateDBCfg.
3. Scaffold BorrowFixProbe, borrow Document.ЗаказКлиента.Form.ФормаДокумента and
   Document.РеализацияТоваровУслуг.Form.ФормаДокумента with BorrowMainAttribute=Form.
4. Load the uncorrected shell using Designer /LoadConfigFromFiles <baseline>
   -Extension BorrowFixProbe /Out <load-baseline.log>. Platform result: 1.
5. Repeat borrowing with omitted-data binding cleanup and load the fixed shell
   using the same command with <fixed>. The preserved load-fixed.log has only
   a BOM and no diagnostics; its exit code was not preserved in this evidence set,
   so this file alone is not an independent proof of successful import.
6. Dump the fixed extension with /DumpCfg -Extension BorrowFixProbe.

load-baseline.log and baseline-exit.txt are the original unmodified bytes.
They demonstrate platform rejection of paths to omitted AdditionalColumns and
auxiliary form attributes in these two real forms. XML fragment files select
original export lines around AdditionalColumns, an auxiliary form attribute,
and Total-prefixed footer bindings. Their separated sections are deliberately
not presented as complete XML fixtures. Line numbers and raw source hashes
are recorded in manifest.json; no business module or user data is included.

The log does NOT establish that every Total-prefixed footer is rejected. That
broader inference is withdrawn: cleanup now requires an exact sibling DataPath
for a declared numeric tabular-section attribute, an undeclared Total+attribute
name, and the known Объект main-attribute convention. Unknown cases and real
Total-prefixed metadata fields are retained. This evidence does not prove UI,
posting, every form variant, or the future public API route planned for v0.15.
