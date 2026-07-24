export const name = 'multi-select: clickElement с modifier ctrl/shift на gridRow';
export const tags = ['table', 'multi-select'];
export const timeout = 60000;

// Покрытие feature `modifier: 'ctrl' | 'shift'` у clickElement для grid-row.
// Ctrl+click добавляет строку в выделение; Shift+click выделяет диапазон от
// anchor'a (последнего non-modifier клика). readTable отмечает выделенные
// строки полем _selected: true.
//
// Свежая синтетика содержит ровно 4 Контрагентов (ООО Север, ООО Юг, ООО
// Восток, ООО Запад). Используем их для предсказуемых проверок.

export default async function({ navigateSection, openCommand, clickElement, readTable, closeForm, assert, step, log }) {

  await step('setup: открыть список Контрагентов', async () => {
    await navigateSection('Склад');
    await openCommand('Контрагенты');
    const t = await readTable();
    log(`Контрагентов в списке: ${t.total}`);
    assert.ok(t.total >= 3, `Нужно как минимум 3 строки для multi-select теста, есть ${t.total}`);
  });

  await step('ctrl-add: click ООО Север + ctrl-click ООО Юг → 2 выделенные строки', async () => {
    await clickElement('ООО Север');
    await clickElement('ООО Юг', { modifier: 'ctrl' });
    const t = await readTable();
    const selected = t.rows.filter(r => r._selected).map(r => r['Наименование']);
    log(`selected after ctrl-add: ${JSON.stringify(selected)}`);
    assert.equal(selected.length, 2, '2 выделенные строки после ctrl-add');
    assert.includes(selected, 'ООО Север', 'ООО Север выделен');
    assert.includes(selected, 'ООО Юг', 'ООО Юг выделен');
  });

  await step('shift-range: shift-click на третью строку → диапазон выделен', async () => {
    // Сбрасываем выделение одиночным кликом, anchor = ООО Север
    await clickElement('ООО Север');
    // Shift+click на ООО Восток (третий по списку) — должен выделить Север..Восток
    await clickElement('ООО Восток', { modifier: 'shift' });
    const t = await readTable();
    const selected = t.rows.filter(r => r._selected).map(r => r['Наименование']);
    log(`selected after shift-range: ${JSON.stringify(selected)}`);
    assert.ok(selected.length >= 2, `Диапазон должен включать минимум 2 строки, выделено ${selected.length}`);
    assert.includes(selected, 'ООО Север', 'anchor ООО Север в диапазоне');
    assert.includes(selected, 'ООО Восток', 'shift-target ООО Восток в диапазоне');
  });

  await step('cleanup: закрыть форму', async () => {
    await closeForm();
  });
}
