# Filling values: extra

Synthetic configuration exported by 1C 8.3.27.2214 after isolated native import, application, metadata check and execution. Unrelated research objects were removed before this independently validated export. No user database was used.

Catalog.Items.Attribute.FillProbe and the Rows.FillProbe table attributes of Report.FillReport and DataProcessor.FillProcessor start with FillFromFillingValue=true and string FillValue=seed. Standard Catalog object form uses input when true and seed when false; absent filling data uses seed. Report and processor table attributes have native import/apply/check/export evidence; functional table filling is not claimed.

The managed application runs the synthetic form probe only when /C supplies its output filename, then exits. MCP edits metadata only and does not run the probe. The exported source has no workstation-specific paths.
