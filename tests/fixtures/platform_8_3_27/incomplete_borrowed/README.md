# Incomplete borrowed metadata: platform evidence

These files are raw XML exports from 1C **8.3.27.2130** on Windows,
format **2.20**. The manifest records the executable hash and each file hash.
They supplement the reader fixtures; this experiment does not replace the
pinned **8.3.27.2074** platform gate.

Two independent disposable infobases accepted the parent configuration and
extension through import, check, apply and export. The second infobase imported
the unmodified extension export from the first. All ten descriptors were
byte-identical between the two exports. No existing user infobase was used.

The extension exports demonstrate these omissions:

- `CommonModule`: all eight normalized properties, including execution flags,
  `Privileged` and `ReturnValuesReuse`, are absent. Name, synonym, comment and
  parent identity remain present.
- `Catalog`: `CodeType` is absent; explicit `Hierarchical=false`, `CodeLength=3`
  and `DescriptionLength=50` remain present.
- `Document`: descriptive properties are absent; `RegisterRecords` is explicitly
  empty.
- `InformationRegister`: `WriteMode` and other descriptive properties are absent.
- `ChartOfCalculationTypes`: `CodeType` and other descriptive properties are absent.

These observations establish the accepted exported shapes. They do not define
inherited values, execution contexts or platform defaults. Reader tests that
modify these files must identify their modified inputs as synthetic.

The parent and extension scaffolds came from
`tests/fixtures/unica_mcp_script_parity/unica_reference_models/cf-init/scripts/cf-init.py`
and `cfe-init/scripts/cfe-init.py`. Catalog and CommonModule inputs started from
`../cfe_borrow/`; the CommonModule input omitted all eight normalized properties
and its Server property-state marker. Document, InformationRegister and
ChartOfCalculationTypes inputs were synthetic minimal descriptions with generated
type identities. The parent register has one string resource. The saved fixtures
are the resulting platform exports, not those synthetic inputs.

The files here are descriptors, not complete importable configurations. Restore
them under their kind directories in complete scaffolds, register each object in
`Configuration.xml`, and preserve matching parent identities. The executed
sequence below replaces disposable paths with placeholders:

```powershell
$ibcmd = 'C:/Program Files/1cv8/8.3.27.2130/bin/ibcmd.exe'
$round = './round1'
$connection = @(
  "--db-path=$round/ib/db", "--data=$round/ib/data",
  "--temp=$round/ib/temp", "--users-data=$round/ib/users",
  "--session-data=$round/ib/session", "--log-data=$round/ib/log"
)
& $ibcmd infobase create @connection --import=./parent-src --apply --force
& $ibcmd config @connection export "$round/parent-export"
& $ibcmd extension @connection create --name=CorpusExtension --name-prefix=CorpusExtension_ --purpose=customization
& $ibcmd config @connection import --extension=CorpusExtension ./extension-src
& $ibcmd config @connection check --extension=CorpusExtension --force
& $ibcmd config @connection apply --extension=CorpusExtension --force
& $ibcmd config @connection export --extension=CorpusExtension "$round/export"
```

All commands exited with zero. Repeat in a new `round2` directory, using the same
parent sources and `./round1/export` as the extension import. Compare descriptors
from both exports without normalization. This checks import/check/apply/export;
it does not claim a separate runtime `CheckCanApply` result.

Provenance note added when this evidence was folded into the #1381 work:
`extension-catalog.xml`, `parent-catalog.xml` and `parent-common-module.xml`
are byte-identical to the files of the same names in `../cfe_borrow/`, which
were exported by 8.3.27.2074. The inputs started from those files and the
platform round trip is byte-exact, so identical bytes are expected; the
8.3.27.2130 run confirms these shapes rather than producing new ones.
