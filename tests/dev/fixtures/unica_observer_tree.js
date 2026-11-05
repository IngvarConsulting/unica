const assert = require('node:assert/strict');
const vm = require('node:vm');
const input = JSON.parse(require('node:fs').readFileSync(0, 'utf8'));

class Element {
  constructor(tag) { this.tag = tag; this.children = []; this.hidden = false; this.textContent = ''; }
  appendChild(child) { child.parentElement = this; this.children.push(child); return child; }
  append(...children) { for (const child of children) this.appendChild(child); }
  insertAdjacentHTML(_, html) { this.meta = (this.meta || '') + html; }
  querySelector(selector) { return this.children.find(child => `.${child.className}` === selector); }
  remove() { const children = this.parentElement.children; children.splice(children.indexOf(this), 1); }
  get childElementCount() { return this.children.length; }
  get previousElementSibling() { return this.parentElement?.children[this.parentElement.children.indexOf(this) - 1]; }
  get nextElementSibling() { return this.parentElement?.children[this.parentElement.children.indexOf(this) + 1]; }
}
const event = (target, detail = 1) => ({target, detail, stopPropagation() {}, preventDefault() {}});
const tick = () => new Promise(resolve => setImmediate(resolve));
const row = wrap => wrap.children[0];
const kids = wrap => wrap.children[1];
const name = wrap => row(wrap).children[1].textContent;

async function run() {
  const calls = [], selected = [];
  const replies = {};
  const context = vm.createContext({
    document: {createElement: tag => new Element(tag)},
    ticking: () => () => {}, esc: String, isGraph: () => false,
    select: (at, element) => selected.push({at, element}),
    view: async at => { calls.push(at); return {result: {ok: true, data: replies[at]}}; },
  });
  vm.runInContext(input.source, context);
  for (const sourceSet of ['main', 'extension']) {
    const owner = `${sourceSet}:CommonModule.ОбщийМодуль`;
    const at = `${owner}.Interface`;
    const publicMethod = `${owner}.Method.Открытая`;
    const internalMethod = `${owner}.Method.Служебная`;
    const overrideMethod = `${owner}.Method.Переопределяемая`;
    replies[at] = {at, kind: 'Interface', items: [
      {kind: 'Interface', name: 'Public', addressable: false, methods: [publicMethod]},
      {kind: 'Interface', name: 'Library', addressable: false, methods: [publicMethod, internalMethod]},
      {kind: 'Interface', name: 'Override', addressable: false, methods: [overrideMethod]},
    ]};
    const root = new Element('div');
    await context.fill(root, at);
    assert.deepEqual(root.children.map(name), ['Public', 'Library', 'Override']);
    const countBefore = calls.length;
    for (const [index, methods] of [[0, [publicMethod]], [1, [publicMethod, internalMethod]], [2, [overrideMethod]]]) {
      const group = root.children[index];
      assert.match(row(group).meta, new RegExp(`>${methods.length}<`));
      assert.equal(kids(group).hidden, true);
      if (row(group).onclick) row(group).onclick(event(row(group)));
      assert.equal(selected.length, sourceSet === 'main' ? index : 3 + index);
      row(group).ondblclick(event(row(group), 2)); await tick();
      assert.equal(kids(group).hidden, false);
      assert.deepEqual(kids(group).children.map(name), methods.map(method => method.slice(owner.length + 1)));
      const methodRow = row(kids(group).children[0]);
      methodRow.onclick(event(methodRow));
      assert.equal(selected.at(-1).at, methods[0]);
      row(group).children[0].onclick(event(row(group).children[0])); await tick();
      assert.equal(kids(group).hidden, true);
      row(group).ondblclick(event(row(group), 2)); await tick();
      assert.equal(kids(group).childElementCount, methods.length);
      assert.equal(calls.length, countBefore, 'local group must never call view');
    }
  }
  const emptyAt = 'main:CommonModule.Empty.Interface';
  replies[emptyAt] = {at: emptyAt, kind: 'Interface', items: [
    ...['Public', 'Library', 'Override'].map(name => ({kind: 'Interface', name, addressable: false, methods: []})),
  ]};
  const empty = new Element('div'); await context.fill(empty, emptyAt);
  assert.deepEqual(empty.children.map(name), ['Public', 'Library', 'Override']);
  for (const group of empty.children) {
    assert.notEqual(row(group).children[0].textContent, '▸');
    assert.equal(kids(group).childElementCount, 0);
  }

  const raw = [{line: 1, text: 'source'}, {kind: 'Interface', name: 'Public', addressable: false, methods: ['main:CommonModule.Raw.Method.Probe']}];
  replies.raw = {kind: 'Body', items: raw};
  const data = new Element('div'); await context.fill(data, 'raw');
  assert.deepEqual(data.children.map(name), raw.map(item => JSON.stringify(item).slice(0, 160)));
  const malformed = [
    {kind: 'Interface', name: 'Public', addressable: false, methods: [42]},
    {kind: 'Interface', name: 'Vendor', addressable: false, methods: ['main:CommonModule.Raw.Method.Probe']},
  ];
  replies.malformed = {kind: 'Interface', items: malformed};
  const malformedRoot = new Element('div'); await context.fill(malformedRoot, 'malformed');
  assert.deepEqual(malformedRoot.children.map(name), malformed.map(item => JSON.stringify(item).slice(0, 160)));
  assert.ok(calls.every(at => !at.endsWith('.Public') && !at.endsWith('.Library') && !at.endsWith('.Override')));
}
run().catch(error => { console.error(error); process.exitCode = 1; });
