export const name = 'modal-select: clickElement по строке широкой модальной формы подбора (F4)';
export const tags = ['table', 'multi-select', 'modal'];
export const timeout = 90000;

// Регресс бага: на УЗКОЙ модальной форме подбора с МНОГИМ числом колонок
// (Контрагенты — 13 колонок, форма «Выбор контрагента») clickElement по строке
// не выделял строку. Причина: координата клика бралась как центр `.gridLine`,
// который шире окна (≈2775px) и уезжал за правый край модалки → mouse.click мимо.
// Фикс (ROW_CLICK_POINT_FN): клик в первую видимую текстовую ячейку строки.
//
// Отличие от 17-multiselect: тот гоняет ctrl/shift на ПОЛНОЭКРАННОМ списке, где
// центр gridLine ещё попадает во вьюпорт — баг там не воспроизводится.
//
// Открываем обработку МножественныйВыбор, фокус на поле «Контрагенты (список)»,
// F4 → модальная форма подбора Контрагентов (table 'Список').

export default async function({ navigateLink, clickElement, getPage, wait, readTable, getFormState, closeForm, assert, step, log }) {

  await step('setup: открыть МножественныйВыбор + F4 на «Контрагенты (список)»', async () => {
    const r = await navigateLink('Обработка.МножественныйВыбор');
    assert.equal(r.activeTab, 'Множественный выбор', 'обработка открыта');

    const f = await clickElement('Контрагенты (список)');
    assert.equal(f.focused?.ok, true, 'поле «Контрагенты (список)» сфокусировано');

    await getPage().keyboard.press('F4');
    await wait(2);

    const st = await getFormState();
    log(`formCount=${st.formCount} modal=${st.modal} tables=${JSON.stringify(st.tables?.map(t => t.name))}`);
    assert.ok(st.modal === true, 'открыта модальная форма подбора');
    assert.ok(st.tables?.some(t => t.name === 'Список'), 'таблица «Список» присутствует');
  });

  await step('single-select: clickElement по строке выделяет её (ядро бага)', async () => {
    await clickElement('ООО Восток', { table: 'Список' });
    const t = await readTable({ table: 'Список' });
    const target = t.rows.find(r => r['Наименование'] === 'ООО Восток');
    log(`selected: ${JSON.stringify(t.rows.filter(r => r._selected).map(r => r['Наименование']))}`);
    assert.ok(target, 'строка «ООО Восток» найдена');
    assert.equal(target._selected, true, 'строка выделилась кликом на модальной форме');
  });

  await step('ctrl-add: ctrl-click добавляет вторую строку к выделению', async () => {
    await clickElement('ООО Юг', { table: 'Список', modifier: 'ctrl' });
    const t = await readTable({ table: 'Список' });
    const selected = t.rows.filter(r => r._selected).map(r => r['Наименование']);
    log(`selected after ctrl: ${JSON.stringify(selected)}`);
    assert.ok(selected.length >= 2, `≥2 строк выделено ctrl-кликом на модалке, выделено ${selected.length}`);
    assert.includes(selected, 'ООО Восток', 'ООО Восток остался выделен');
    assert.includes(selected, 'ООО Юг', 'ООО Юг добавлен в выделение');
  });

  await step('cleanup: закрыть форму подбора и обработку', async () => {
    await closeForm();   // модальная форма подбора (Escape)
    await closeForm();   // обработка
  });
}
