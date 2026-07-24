export const name = 'hierarchy: groups + tree-grid (Номенклатура)';
export const tags = ['hierarchy'];
export const timeout = 90000;

export default async function({ navigateSection, openCommand, clickElement, closeForm, readTable, assert, step, log }) {

  await step('setup: открыть Номенклатуру и явно переключиться в иерархический список', async () => {
    await navigateSection('Склад');
    await openCommand('Номенклатура');
    // viewMode сохраняется между сессиями в пользовательских настройках формы
    // и НЕ сбрасывается «Установить стандартные настройки». Переключаем явно.
    await clickElement('Ещё');
    await clickElement('Режим просмотра');
    await clickElement('Иерархический список');
    // Сброс остальных настроек (раскрытие групп, фильтры и т.п.)
    await clickElement('Ещё');
    await clickElement('Установить стандартные настройки');
  });

  await step('read-groups: иерархический список возвращает группы верхнего уровня', async () => {
    const t = await readTable();
    log(`total=${t.total} rows=${t.rows?.length} viewMode=${t.viewMode}`);
    assert.equal(t.total, 3, 'три группы верхнего уровня (Товары, Услуги, БольшойСписок)');
    assert.ok(t.rows.every(r => r._kind === 'group'), 'все строки — группы (_kind=group)');
    const names = t.rows.map(r => r['Наименование']);
    assert.includes(names, 'Товары', 'есть группа Товары');
    assert.includes(names, 'Услуги', 'есть группа Услуги');
    assert.includes(names, 'БольшойСписок', 'есть группа БольшойСписок');
  });

  await step('group-expand: clickElement({expand}) раскрывает группу и показывает элементы', async () => {
    const r = await clickElement('Товары', { expand: true });
    log(`clicked: ${JSON.stringify(r.clicked)}`);
    assert.equal(r.clicked?.kind, 'gridGroup', 'kind=gridGroup');
    assert.equal(r.clicked?.toggled, true, 'toggled=true');
    const t = await readTable({ maxRows: 30 });
    log(`after expand: total=${t.total}`);
    assert.ok(t.total >= 16, `Товары + 15 элементов >= 16 строк (got ${t.total})`);
    const parent = t.rows.find(row => row['Наименование'] === 'Товары');
    assert.ok(parent, 'строка-родитель Товары присутствует');
    const items = t.rows.filter(row => /^Товар \d+/.test(row['Наименование'] || ''));
    assert.ok(items.length >= 15, `15 элементов внутри группы (got ${items.length})`);
    // Свернуть обратно для чистоты (expand:false = только свернуть)
    await clickElement('Товары', { expand: false });
  });

  await step('switch-tree: «Ещё → Режим просмотра → Дерево» переключает viewMode', async () => {
    await clickElement('Ещё');
    await clickElement('Режим просмотра');
    await clickElement('Дерево');
    const t = await readTable();
    log(`after switch: viewMode=${t.viewMode} total=${t.total}`);
    assert.equal(t.viewMode, 'tree', 'viewMode переключился в tree');
  });

  await step('read-tree: readTable в режиме Дерево возвращает _tree состояния', async () => {
    const t = await readTable();
    log(`tree rows: ${t.rows?.map(r => `${r['Наименование']}:${r._tree}`).join(' | ')}`);
    const groupRows = t.rows.filter(r => /^(Товары|Услуги|БольшойСписок)$/.test(r['Наименование'] || ''));
    assert.equal(groupRows.length, 3, 'все три группы видны в дереве');
    assert.ok(groupRows.every(r => r._tree === 'collapsed' || r._tree === 'expanded'),
      '_tree присутствует у каждой группы (collapsed или expanded)');
  });

  await step('tree-expand: clickElement({expand}) переключает состояние узла', async () => {
    // viewMode/expanded сохраняются между сессиями — приводим Товары в collapsed
    let t = await readTable();
    let tovary = t.rows.find(r => r['Наименование'] === 'Товары');
    if (tovary?._tree === 'expanded') {
      await clickElement('Товары', { expand: false }); // expand:false = свернуть
    }
    // Теперь явный expand и проверка
    const r = await clickElement('Товары', { expand: true });
    log(`clicked: ${JSON.stringify(r.clicked)}`);
    assert.equal(r.clicked?.kind, 'gridTreeNode', 'kind=gridTreeNode');
    assert.equal(r.clicked?.toggled, true, 'toggled=true');
    t = await readTable({ maxRows: 30 });
    log(`after tree-expand: total=${t.total}`);
    tovary = t.rows.find(row => row['Наименование'] === 'Товары');
    assert.ok(tovary, 'строка Товары присутствует');
    assert.equal(tovary._tree, 'expanded', 'Товары теперь expanded');
    const items = t.rows.filter(row => /^Товар \d+/.test(row['Наименование'] || ''));
    assert.ok(items.length >= 15, `видны элементы группы (${items.length})`);
  });

  await step('cleanup: восстановить иерархический список и закрыть форму', async () => {
    await clickElement('Ещё');
    await clickElement('Режим просмотра');
    await clickElement('Иерархический список');
    await closeForm();
  });
}
