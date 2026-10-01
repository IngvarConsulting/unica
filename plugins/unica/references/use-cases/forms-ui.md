# Forms And UI

## When to use

Use this when the user asks to add, inspect, compile, edit, validate, or remove
managed forms, form elements, attributes, commands, event hooks, or form module
behavior.

Do not use this for preparing a debug server or running browser tests. Use the
autonomous server debug use case for that.

## Primary path

Use native MCP tools through `unica`:

- `unica.apply` with `form.add` scaffolds forms from the owner node and `items`:
  metadata XML, `Form.xml`, `Module.bsl` and the owner's `<Form>` registration.
- `unica.apply` with `form.create` scaffolds one form from its own form address
  and `values`; both reach the same staging, so `Form.xml` is written either way.
- `unica.form.edit` applies point changes to an existing form.
- `unica.view` on the form node gives compact structure before editing.
- `unica.check` on the form node (validator `form`) checks XML and structural constraints.
- `unica.apply` with `form.remove` removes form metadata and files.

For form modules, combine this with platform form-module standards and targeted
source edits.

## Related references

- `../specs/1c-form-spec.md`
- `../specs/form-dsl-spec.md`
- `../specs/form-patterns.md`
- `../platform/development-standards.md`
