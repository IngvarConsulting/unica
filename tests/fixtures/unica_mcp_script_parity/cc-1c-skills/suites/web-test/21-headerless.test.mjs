export const name = 'headerless grids: readTable / getFormState / fillTableRow / clickElement без шапки';
export const tags = ['headerless', 'table', 'smoke'];
export const timeout = 180000;

// Покрывает поддержку гридов без .gridHead (фаза 2 мультивыбора):
//  - БезшапочнаяТаблица (ValueTable, отдельная колонка-чекбокс)
//  - МножественныйВыбор → Через флажки (марк-список, комбинированная ячейка галка+текст)
// Деривация колонок едина (synthHeaderlessColumns): Колонка{N} + (checkbox), привязка по colindex.
// Правка data-ячейки на безголовом ValueTable (число/ссылка через edit-Tab-loop) — тоже покрыта:
// в режиме редактирования INPUT рендерится в оверлее вне .gridBox, colindex резолвится по
// x-координате относительно ячеек тела (grid-edit v1.2), иначе synth-имя Колонка{N} не матчилось.

export default async function({
  navigateLink, clickElement, fillTableRow, readTable, getFormState, wait, assert, step, log, closeForm
}) {

  // ── ValueTable (отдельная колонка-чекбокс) ──────────────────────────────────
  await step('readTable: ValueTable без шапки → Колонка1..3 + (checkbox)', async () => {
    await navigateLink('Обработка.БезшапочнаяТаблица');
    await wait(1.2);
    const t = await readTable({ maxRows: 10 });
    log(`columns: ${JSON.stringify(t.columns)}`);
    assert.deepEqual(t.columns, ['Колонка1', 'Колонка2', 'Колонка3', '(checkbox)'],
      'синтетические колонки по colindex');
    assert.equal(t.total, 3, '3 строки');
    assert.equal(t.rows[0]['Колонка1'], 'Позиция 001', 'текст data-колонки');
    assert.ok(t.rows[0]['(checkbox)'] === 'true' || t.rows[0]['(checkbox)'] === 'false',
      '(checkbox) читается как true/false');
  });

  await step('getFormState: сводка таблицы показывает synth-колонки + rowCount', async () => {
    const fs = await getFormState();
    const tbl = (fs.tables || []).find(x => (x.columns || []).includes('(checkbox)'));
    log(`tables: ${JSON.stringify(fs.tables)}`);
    assert.ok(tbl, 'таблица с (checkbox) в сводке');
    assert.deepEqual(tbl.columns, ['Колонка1', 'Колонка2', 'Колонка3', '(checkbox)'], 'колонки сводки совпадают с readTable');
    assert.equal(tbl.rowCount, 3, 'rowCount');
  });

  await step('fillTableRow: тоггл (checkbox) по индексу строки', async () => {
    const before = (await readTable({ maxRows: 5 })).rows.map(r => r['(checkbox)']);
    const target = before[1] === 'true' ? 'false' : 'true';   // строка 1 — переключаем в противоположное
    const r = await fillTableRow({ '(checkbox)': target }, { row: 1 });
    log(`filled: ${JSON.stringify(r.filled)}`);
    assert.ok(r.filled?.[0]?.ok && r.filled[0].method === 'toggle', 'method=toggle, ok');
    const after = (await readTable({ maxRows: 5 })).rows.map(r => r['(checkbox)']);
    assert.equal(after[1], target, `строка 1 переключена в ${target}`);
  });

  await step('fillTableRow: тоггл (checkbox) по фильтру {Колонка1}', async () => {
    const r = await fillTableRow({ '(checkbox)': 'true' }, { row: { 'Колонка1': 'Позиция 002' } });
    log(`filled: ${JSON.stringify(r.filled)}`);
    assert.ok(r.filled?.[0]?.ok, 'ok через row-фильтр {Колонка1}');
    const t = await readTable({ maxRows: 5 });
    const row = t.rows.find(x => x['Колонка1'] === 'Позиция 002');
    assert.equal(row['(checkbox)'], 'true', 'Позиция 002 отмечена');
  });

  await step('clickElement: клик по ячейке (checkbox) тогглит галку', async () => {
    const t0 = await readTable({ maxRows: 5 });
    const r0 = t0.rows.find(x => x['Колонка1'] === 'Позиция 003');
    const want = r0['(checkbox)'] === 'true' ? 'false' : 'true';
    await clickElement({ row: { 'Колонка1': 'Позиция 003' }, column: '(checkbox)' });
    await wait(0.4);
    const t1 = await readTable({ maxRows: 5 });
    const r1 = t1.rows.find(x => x['Колонка1'] === 'Позиция 003');
    assert.equal(r1['(checkbox)'], want, `клик переключил галку Позиция 003 в ${want}`);
  });

  await step('fillTableRow: правка числовой data-ячейки Колонка2 по индексу (method=direct)', async () => {
    const r = await fillTableRow({ 'Колонка2': '777' }, { row: 1 });
    log(`filled: ${JSON.stringify(r.filled)}`);
    assert.ok(r.filled?.[0]?.ok && r.filled[0].method === 'direct', 'method=direct, ok');
    const t = await readTable({ maxRows: 5 });
    assert.ok(t.rows[1]['Колонка2'].startsWith('777'), `Колонка2 строки 1 = 777, got ${t.rows[1]['Колонка2']}`);
  });

  await step('fillTableRow: правка ссылочной data-ячейки Колонка1 по фильтру (выбор из списка)', async () => {
    const r = await fillTableRow({ 'Колонка1': 'Позиция 001' }, { row: { 'Колонка1': 'Позиция 003' } });
    log(`filled: ${JSON.stringify(r.filled)}`);
    assert.ok(r.filled?.[0]?.ok, 'ok');
    assert.ok(['dropdown', 'form'].includes(r.filled[0].method), `method dropdown|form, got ${r.filled[0].method}`);
    const t = await readTable({ maxRows: 5 });
    assert.equal(t.rows.filter(x => x['Колонка1'] === 'Позиция 001').length, 2, 'две строки Позиция 001 (была 001 + перезаписанная 003)');
  });

  await step('clickElement: клик по data-ячейке резолвит синтетическую колонку Колонка3', async () => {
    const r = await clickElement({ row: { 'Колонка1': 'Позиция 002' }, column: 'Колонка3' });
    log(`clicked: ${JSON.stringify(r.clicked)}`);
    assert.equal(r.clicked?.kind, 'gridCell', 'клик по ячейке грида');
    assert.equal(r.clicked?.column, 'Колонка3', 'резолвнутая колонка — синтетическая Колонка3');
  });

  // ── Марк-список (комбинированная ячейка галка+текст в одной .gridBox) ────────
  await step('марк-список: readTable → (checkbox) + Колонка1, расщепление одной ячейки', async () => {
    await navigateLink('Обработка.МножественныйВыбор');
    await clickElement('Через флажки');
    await wait(1.2);
    const t = await readTable({ maxRows: 10 });
    log(`columns: ${JSON.stringify(t.columns)} rows: ${JSON.stringify(t.rows)}`);
    assert.deepEqual(t.columns, ['(checkbox)', 'Колонка1'], 'комбинированная ячейка → 2 колонки');
    assert.ok(t.rows.some(r => r['Колонка1'] === 'Альфа'), 'Альфа в Колонка1');
    assert.ok(t.rows.every(r => r['(checkbox)'] === 'true' || r['(checkbox)'] === 'false'), '(checkbox) булев');
  });

  await step('марк-список: fillTableRow отмечает по фильтру {Колонка1: Альфа}', async () => {
    const r = await fillTableRow({ '(checkbox)': 'true' }, { row: { 'Колонка1': 'Альфа' } });
    log(`filled: ${JSON.stringify(r.filled)}`);
    assert.ok(r.filled?.[0]?.ok && r.filled[0].method === 'toggle', 'method=toggle, ok');
    const t = await readTable({ maxRows: 10 });
    const alpha = t.rows.find(x => x['Колонка1'] === 'Альфа');
    assert.equal(alpha['(checkbox)'], 'true', 'Альфа отмечена');
  });

  await step('cleanup: закрыть форму ввода значений', async () => {
    await closeForm({ save: false });
  });
}
