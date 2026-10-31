# Calculation register base dimension

Synthetic configuration exported by 1C 8.3.27.2214 after native import,
application, metadata checking and runtime execution. It contains the linked
CalculationRegister.Accruals, ChartOfCalculationTypes.Kinds, Document.Orders,
CommonModule.Probe and Russian language. Both dimensions start with
BaseDimension=false. Employee and Department are variable strings of length 20;
Amount is decimal 10.2.

The server Probe.Run seeds three SALARY records once: Alice/A=100,
Alice/B=999 and Bob/A=777. A BONUS record for Alice/A depends on SALARY.
The native base query matches both dimensions and returns 100. Importing the
same register with Employee.BaseDimension=false→true→false preserves that
result and Department.BaseDimension=false on the same fixed database.

The managed application executes this isolated runtime check only when /C
supplies an output filename. It writes the result there and exits. There are
no workstation-specific output paths in the fixture. The MCP scenario only
edits metadata and does not run BSL or seed records.

Provenance: local synthetic research for #1701, then a compact configuration with
unrelated probe objects removed, independently imported/applied/checked and
exported on 8.3.27.2214. No user configuration or DemoERPUH database was used.
