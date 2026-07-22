# Forms And UI

## When to use

Use this when the user asks to add, inspect, compile, edit, validate, or remove
managed forms, form elements, attributes, commands, event hooks, or form module
behavior.

Do not use this for preparing a debug server or running browser tests. Use the
autonomous server debug use case for that.

## Primary path

Use native MCP tools through `unica`:

- `unica.form.add` creates form metadata and registration.
- `unica.form.compile` creates `Form.xml` from JSON DSL or object metadata.
- `unica.form.edit` applies point changes to an existing form.
- `unica.form.info` gives compact structure before editing.
- `unica.form.validate` checks XML and structural constraints.
- `unica.form.remove` removes form metadata and files.

For form modules, combine this with platform form-module standards and targeted
source edits.

## Related references

- `references/specs/1c-form-spec.md`
- `references/specs/form-dsl-spec.md`
- `references/specs/form-patterns.md`
- `references/platform/development-standards.md`
