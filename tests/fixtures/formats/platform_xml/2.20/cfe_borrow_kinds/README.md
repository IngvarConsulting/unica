# Borrowed objects of every kind: platform evidence (2.20)

Each descriptor here is what 1C **8.3.27.2342** exports for an extension (format
**2.20**, the `empty_extension` corpus) that borrows one object of every kind
of the `all_kinds` corpus of the same format. The objects were borrowed by
Unica (`object.borrow`), loaded together with
`DESIGNER /LoadConfigFromFiles ... -Extension` into a disposable file infobase
over `all_kinds` and exported with `/DumpConfigToFiles ... -Extension`. The
layout is the layout of the extension: `<Kind directory>/<Name>.xml`.

What the platform writes beyond the identity of a borrowed object:

- a common form, a role and a subsystem: `xr:PropertyState` of `Form`,
  `Rights` and `CommandInterface` respectively, state `Extended`, in
  `InternalInfo`; the export has no `Ext/` file for it, the descriptor is all
  the platform writes for such a borrowed object;
- a common template: its `TemplateType` from the parent;
- a subsystem: an empty `Content`;
- a catalog: `Hierarchical` and `CodeLength` from the parent;
- a sequence and a WS reference: the generated types of the parent, the
  platform refuses either without them.

`PaletteColor` and `Queue` are kinds of 8.5 and have no 2.20 descriptor.
