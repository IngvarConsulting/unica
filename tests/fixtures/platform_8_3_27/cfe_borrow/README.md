# Borrowed properties: platform evidence

These are raw descriptor exports from 1C **8.3.27.2074**, XML format **2.20**.
`manifest.json` records the platform binary and fixture SHA-256 hashes.
Each experiment used disposable local infobases; no existing user base was used.

The parent configuration and empty extension were generated from the repository's
`tests/fixtures/unica_mcp_script_parity/unica_reference_models/cf-init/scripts/cf-init.py`
and corresponding `cfe-init/scripts/cfe-init.py` scaffolds. The parent contained
`Catalog.CorpusCatalog` and `CommonModule.CorpusModule`; both were registered in
`Configuration.xml`. The four files here are exported object descriptors, not
complete importable configurations. Keep them in their `Catalogs/` and
`CommonModules/` locations inside complete scaffolds when repeating the experiment.

The following is the executed command sequence with temporary paths replaced by
relative placeholders. `parent-src` and `extension-src` are complete XML exports.
The same binary was used for every command; all commands exited with status zero.

```bash
ibcmd=/opt/1cv8/8.3.27.2074/ibcmd
round=./round1
connection=(--db-path="$round/ib/db" --data="$round/ib/data"
  --temp="$round/ib/temp" --users-data="$round/ib/users"
  --session-data="$round/ib/session" --log-data="$round/ib/log")
"$ibcmd" infobase create "${connection[@]}" --import=./parent-src --apply --force
"$ibcmd" extension "${connection[@]}" create --name=CorpusExtension --name-prefix=CorpusExtension_ --purpose=customization
"$ibcmd" config "${connection[@]}" import --extension=CorpusExtension ./extension-src
"$ibcmd" config "${connection[@]}" check --extension=CorpusExtension --force
"$ibcmd" config "${connection[@]}" apply --extension=CorpusExtension --force
"$ibcmd" config "${connection[@]}" export --extension=CorpusExtension "$round/export"
```

Repeat all six commands with `round=./round2`, the same parent sources, and
`./round1/export` as the extension import. Both extension descriptors were
byte-identical after the second export from this independently created infobase.

The Catalog fixture proves preservation of `DescriptionLength=50` with
`xr:PropertyState` `DescriptionLength/Extended`, alongside `CodeLength=3` and
`Hierarchical=false` without state markers. The CommonModule fixture proves
preservation of `Server=false` with `Server/Notify`. A separate `Server/Extended`
probe lost its marker on export; it is not evidence for that override format.
These observations establish XML import/check/apply/export behavior, not a
separate runtime `CheckCanApply` result.

A further execution used the current Unica canonical `object.borrow` output as
the extension input to the same two-cycle sequence. Before import, a parent
refresh changed `CodeLength` from 3 to 5 while preserving local
`DescriptionLength=50`, Comment, object UUID and generated type identities.
Both cycles preserved those values, `DescriptionLength/Extended` and
`Server/Notify`; both final descriptors were byte-identical between cycles.
The canonical protocol probe also checked write-free preview, rejection of a
revision made stale by a parent edit, and byte- and file-identity-preserving
repeat apply with the committed combined revision. This execution supplements
the fixture evidence; its refreshed descriptors are not the files in this folder.
