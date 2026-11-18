# Nested metadata property read fixture

Synthetic configuration exported by 1C:Enterprise 8.3.27.2214 after successful import, database update and configuration check. XML format is 2.20. The fixture was authored independently of the Unica reader and writer.

Document Orders has two commands with different Group, ParameterUseMode and ModifiesData values and command modules. InformationRegister Facts has string and CatalogRef.Items dimensions with paired Master/MainFilter/DenyIncompleteValues booleans, TransformValues/Deny TypeReductionMode and a known string FillValue `seed`. AccumulationRegister Movements uses Turnovers: its included dimension precedes the dimension with UseInTotals=false. DocumentJournal Entries contains all three Indexing values and references Orders.Status.

The export preserves UUIDs, types, qualifiers, fill values and command modules. Reading must preserve all source bytes. Negative tests modify copies to represent absence, malformed values, duplicates and foreign namespaces; those copies are not claimed to be valid Designer exports. Platform defaults observed on re-export are not defaults for the reader.
