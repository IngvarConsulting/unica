export const name = 'DCS-отчёт: structured smoke + быстрый пользовательский фильтр';
export const tags = ['report', 'smoke'];
export const timeout = 90000;

export default async function({ navigateSection, openCommand, getFormState, getCommands, clickElement, selectValue, fillFields, readSpreadsheet, closeForm, wait, assert, step, log }) {

  await step('navigation: команда отчёта зарегистрирована в подсистеме Склад', async () => {
    const r = await navigateSection('Склад');
    const flat = (r.commands || []).flat();
    log(`commands: ${JSON.stringify(flat)}`);
    assert.ok(flat.includes('Остатки товаров'), 'В подсистеме Склад есть команда «Остатки товаров»');
  });

  await step('open: openCommand отрывает форму отчёта с кнопкой Сформировать', async () => {
    const s = await openCommand('Остатки товаров');
    log(`form=${s.form} formCount=${s.formCount} buttons=${s.buttons?.map(b => b.name).join(',')}`);
    assert.equal(s.formCount, 1, 'Открыта одна форма');
    const submit = s.buttons?.find(b => b.name === 'Сформировать');
    assert.ok(submit, 'Есть кнопка «Сформировать»');
    assert.equal(submit.default, true, '«Сформировать» — кнопка по умолчанию');
  });

  await step('guard: readSpreadsheet до «Сформировать» → осмысленная ошибка, не ReferenceError', async () => {
    // Регрессия: чтение несформированного отчёта идёт в ветку allCells.size===0,
    // которая вызывает checkForErrors(). После рефакторинга на модули символ не был
    // импортирован в spreadsheet.mjs → падало с "checkForErrors is not defined".
    // Теперь должно бросать осмысленное сообщение (+ подсказка из инфо-панели 1С).
    let threw = false, msg = '';
    try { await readSpreadsheet(); }
    catch (e) { threw = true; msg = e.message; }
    log(`readBeforeGenerate: threw=${threw} msg=${msg}`);
    assert.ok(threw, 'readSpreadsheet должен бросить — отчёт ещё не сформирован');
    assert.includes(msg, 'no SpreadsheetDocument', 'осмысленное сообщение readSpreadsheet');
    assert.ok(!/is not defined/.test(msg), 'не ReferenceError — checkForErrors импортирован');
  });

  await step('reset: сброс пользовательских настроек к стандартным', async () => {
    // 1С хранит пользовательские настройки между сессиями — сбрасываем к дефолту,
    // чтобы тест был идемпотентным независимо от предыдущих прогонов.
    await clickElement('Еще');
    await clickElement('Установить стандартные настройки');
  });

  await step('quickAccess: быстрый фильтр Номенклатура виден и выключен по умолчанию', async () => {
    const s = await getFormState();
    log(`reportSettings: ${JSON.stringify(s.reportSettings)}`);
    assert.ok(Array.isArray(s.reportSettings) && s.reportSettings.length === 1, 'Один быстрый фильтр в reportSettings');
    const f = s.reportSettings[0];
    assert.equal(f.name, 'Номенклатура', 'Имя фильтра — заголовок DCS-поля');
    assert.equal(f.enabled, false, '@off — выключен по умолчанию');
    assert.equal(f.value, '', 'Значение пустое');
    assert.ok(Array.isArray(f.actions) && f.actions.includes('select'), 'Доступно действие select');
  });

  let baseRowCount = 0;
  let baseTotalSum = '';

  await step('generate: отчёт без фильтра возвращает все строки', async () => {
    await clickElement('Сформировать');
    await wait(3);
    const r = await readSpreadsheet();
    log(`headers=${JSON.stringify(r.headers)} total=${r.total} totals=${JSON.stringify(r.totals)}`);
    assert.deepEqual(r.headers, ['Номенклатура', 'Количество', 'Сумма'], 'Заголовки колонок отчёта');
    assert.ok(r.data?.length >= 2, 'В отчёте есть строки данных');
    assert.ok(r.totals?.['Сумма'], 'Есть итог по Сумме');
    baseRowCount = r.data.length;
    baseTotalSum = r.totals['Сумма'];
  });

  await step('apply filter: selectValue включает чекбокс и подставляет значение', async () => {
    const r = await selectValue('Номенклатура', 'Товар 02');
    log(`selected: ${JSON.stringify(r.selected)}`);
    assert.ok(r.selected, 'selectValue вернул объект selected');
    const after = await getFormState();
    const f = after.reportSettings?.[0];
    log(`after filter: ${JSON.stringify(f)}`);
    assert.equal(f.enabled, true, 'Чекбокс быстрого фильтра автоматически включился');
    assert.equal(f.value, 'Товар 02', 'Подставилось выбранное значение');
  });

  await step('regenerate: отчёт с фильтром возвращает только подходящие строки', async () => {
    await clickElement('Сформировать');
    await wait(3);
    const r = await readSpreadsheet();
    log(`filtered total=${r.total} rows=${r.data?.length} totals=${JSON.stringify(r.totals)}`);
    assert.ok(r.data.length < baseRowCount, `Строк меньше чем без фильтра (${r.data.length} < ${baseRowCount})`);
    const named = r.data.filter(row => row['Номенклатура']);
    assert.ok(named.length >= 1, 'Есть хотя бы одна именованная строка');
    assert.ok(named.every(row => row['Номенклатура'] === 'Товар 02'), 'Все именованные строки относятся к «Товар 02»');
    const sumKey = Object.keys(r.totals).find(k => k.includes('Сумма'));
    assert.ok(sumKey, 'В totals есть колонка Суммы (платформа дописывает контекст фильтра)');
    assert.notEqual(r.totals[sumKey], baseTotalSum, 'Итог по Сумме изменился после применения фильтра');
  });

  await step('clear filter: выключение чекбокса возвращает полный набор данных', async () => {
    // Снять быстрый фильтр через toggle off — fillFields с 'false' выключает чекбокс,
    // value сохраняется (платформа помнит последний выбор для повторного включения),
    // но данные при перерасчёте возвращаются к нефильтрованному набору.
    const r = await fillFields({ 'Номенклатура': 'false' });
    log(`toggle off: ${JSON.stringify(r.filled)}`);
    const after = await getFormState();
    assert.equal(after.reportSettings[0].enabled, false, 'Чекбокс выключен');

    await clickElement('Сформировать');
    await wait(3);
    const report = await readSpreadsheet();
    log(`after clear: rows=${report.data?.length} totals=${JSON.stringify(report.totals)}`);
    assert.equal(report.data.length, baseRowCount, 'Восстановилось исходное число строк');
    assert.equal(report.totals['Сумма'], baseTotalSum, 'Восстановился исходный итог по Сумме');
  });

  await step('drill-down: dblclick по ячейке Номенклатура открывает форму элемента', async () => {
    // Сформируем отчёт ещё раз для чистого состояния
    await clickElement('Сформировать');
    await wait(3);
    const r = await readSpreadsheet();
    const namedIdx = r.data.findIndex(row => row['Номенклатура']);
    log(`first row with Номенклатура: idx=${namedIdx} value=${r.data[namedIdx]?.['Номенклатура']}`);
    assert.ok(namedIdx >= 0, 'есть строка с заполненной Номенклатурой');

    const beforeForm = await getFormState();
    const clicked = await clickElement({ row: namedIdx, column: 'Номенклатура' }, { dblclick: true });
    log(`clicked: ${JSON.stringify(clicked.clicked)}`);
    assert.equal(clicked.clicked?.kind, 'spreadsheetCell', 'clicked.kind=spreadsheetCell');
    await wait(1);

    const after = await getFormState();
    log(`after drill: form=${after.form} buttons=${after.buttons?.map(b => b.name).join(',')}`);
    assert.notEqual(after.form, beforeForm.form, 'открыта новая форма (form изменился)');
    const hasItemButton = after.buttons?.some(b => b.name === 'Записать и закрыть' || b.name === 'Записать');
    assert.ok(hasItemButton, 'открыта форма элемента (есть «Записать»)');
    await closeForm();
  });

  await step('cleanup: закрываем форму отчёта', async () => {
    const r = await closeForm();
    log(`closed=${r.closed} formCount=${r.formCount}`);
    assert.equal(r.closed, true, 'Форма закрылась');
  });
}
