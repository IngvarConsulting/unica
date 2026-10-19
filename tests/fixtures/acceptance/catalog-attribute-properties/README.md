# Catalog tabular attribute observations

This independent reader fixture retains the Представления section from the saved
platform XML fixture `tests/fixtures/unica_mcp_script_parity/bsp/meta/Catalogs/Валюты.xml`.
That upstream fixture is the declared XML 2.21 to 2.20 corpus projection, not a
byte-identical copy of the original Configurator export. The extracted section
keeps UUIDs, generated type identities, StandardAttribute, string qualifiers,
КодЯзыка / ПараметрыПрописи names, and the latter's nonempty Russian ToolTip.
The owner and generated type owner name are renamed to Items; a minimal owner,
Configuration, and independent header Attribute make it a dedicated workspace.
No Unica emitter generates the expected XML or public result.

Damaged.xml changes only the owner name and the first Indexing token to Unknown.
Read-only acceptance checks healthy observations, local limits, and SHA-256 of
all XML/project inputs. ToolTip is retained as input evidence; publishing it is
outside this increment's three enum properties.
