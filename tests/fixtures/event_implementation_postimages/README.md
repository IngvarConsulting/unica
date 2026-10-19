# Event implementation: complete expected postimages

These manually constructed whole-file expectations belong to
S-1721-event-implementation-pair and S-1721-event-missing-platform.
They are not a new workspace or a platform export. The source is the existing
acceptance/workspace fixture; no product plan, diff or emitter produces them.

The pair Form.xml inserts one literal OnOpen/ПриОткрытии Events section after
the unique AutoCommandBar, preserving every original byte, BOM, LF and absent
terminal newline. Both form-module expectations insert the compatible AtClient
procedure plus one blank separator before the first form-handler region close,
keeping the existing BOM and CRLF. The platform expectation inserts the literal
ПередУдалением(Отказ) procedure without a directive before the sole object
handler region close, preserving the BOM and LF.

The Missing scenario first binds ExistingOpen with the existing event.bind
operation, executes that setup and captures its complete Form.xml hash. The
subsequent event.implement must preserve that preexisting bound Form.xml exactly;
its expected module is specified independently here. The setup binding is also
checked through XML and public Missing state. No event.implement output is used
to construct its own expected postimage.

Signatures/directive policy come from the checked 8.3.27 event catalog and
existing form-event handler rules. The source resources are:

- acceptance/workspace/src/Catalogs/Валюты/Forms/ФормаЭлемента/Ext/Form.xml
- acceptance/workspace/src/Catalogs/Валюты/Forms/ФормаЭлемента/Ext/Form/Module.bsl
- acceptance/workspace/src/Catalogs/Валюты/Ext/ObjectModule.bsl

The canonical scenarios compare the complete published SHA below and read back
the logical state and method. Final SHA checks preserve every original neighbor.
A separate scoped proof observes complete inventory and modes, including each
preview/refusal; the existing corpus DSL has no mode observation. These files
alone do not claim platform import or execution.

| Expected resource | SHA256 |
| --- | --- |
| pair/Form.xml | 433258850b63dbe9afa4799d27e198b6cd464b5a0badc6759c8a33ac769c9a1e |
| pair/Module.bsl | 2ef9115491b0074f9e8167cfc9f6492449cf977e86d8cced79b2ab6dc8f44bf4 |
| missing/Module.bsl | 1b4616915e7b11bdfc47c6f56d3ea5b568bc5e93c416884211fb6c34b56e9b9c |
| platform/ObjectModule.bsl | 92499d4acd2d18b6030b884ea2928ff1ac64a1c91348a4ea94395c5dba1d756b |
