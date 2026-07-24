export const name = 'fillFields: text, checkbox, date, dropdown, reference';
export const tags = ['fillfields', 'smoke'];
export const timeout = 120000;

const findField = (state, name) => state.fields?.find(f => f.name === name || f.label === name);

export default async function({ navigateSection, openCommand, clickElement, fillFields, fillTableRow, selectValue, filterList, closeForm, getFormState, assert, step, log }) {

  await step('text+checkbox+date+dropdown: fillFields на Номенклатура', async () => {
    await navigateSection('Склад');
    await openCommand('Номенклатура');
    await clickElement('Товары', { dblclick: true });   // войти в папку
    await clickElement('Товар 01', { dblclick: true });

    const result = await fillFields({
      'Артикул': 'TEST-001',
      'Активен': false,                       // Boolean → CheckBoxField, toggle
      'ДатаПоступления': '15.05.2026',        // date → CB iCalendB calendar, paste
      'Цена': '777',                          // Number → CB iCalcB calculator, paste
      'ВидНоменклатуры': 'Услуга',            // EnumRef dropdown
    });

    log('methods: ' + result.filled.map(f => `${f.field}=${f.method}`).join(', '));
    for (const f of result.filled) {
      assert.ok(f.ok, `fillField "${f.field}" должен вернуть ok=true`);
    }
    assert.equal(result.filled.find(f => f.field === 'Цена')?.method, 'paste',
      'Цена через paste (калькулятор ≠ форма выбора)');

    const state = await getFormState();
    assert.equal(findField(state, 'Артикул')?.value, 'TEST-001', 'Артикул text');
    assert.equal(findField(state, 'Активен')?.value, false, 'Активен checkbox=false');
    assert.equal(findField(state, 'ДатаПоступления')?.value, '15.05.2026', 'ДатаПоступления');
    assert.equal(findField(state, 'Цена')?.value, '777,00', 'Цена записалась (1С форматирует → 777,00)');
    assert.equal(findField(state, 'ВидНоменклатуры')?.value, 'Услуга', 'ВидНоменклатуры dropdown');

    await closeForm({ save: false });
  });

  await step('reference-dropdown: Организация → CatalogRef.Организации (quickChoice=true)', async () => {
    await navigateSection('Склад');
    await openCommand('Приходная накладная');
    await clickElement('Создать');

    const fillRes = await fillFields({
      'Организация': 'Альфа',
    });
    log('reference method: ' + fillRes.filled[0]?.method);
    assert.ok(fillRes.filled[0]?.ok, 'Организация fillField должна сработать');

    const state = await getFormState();
    const org = findField(state, 'Организация');
    log(`Организация value='${org?.value}'`);
    assert.includes(org?.value || '', 'Альфа', 'Организация должна показать выбранное значение');

    await closeForm({ save: false });
  });

  await step('clear: fillFields пустым значением очищает текстовое поле', async () => {
    await navigateSection('Склад');
    await openCommand('Контрагенты');
    await clickElement('ООО Север', { dblclick: true });
    const before = await getFormState();
    const phoneBefore = findField(before, 'Телефон')?.value;
    log(`phone before clear='${phoneBefore}'`);

    const r = await fillFields({ 'Телефон': '' });
    log('clear method: ' + r.filled[0]?.method);
    assert.ok(r.filled[0]?.ok, 'clear должен вернуть ok=true');
    assert.equal(r.filled[0]?.method, 'clear', 'method должен быть clear (Shift+F4)');

    const state = await getFormState();
    assert.equal(findField(state, 'Телефон')?.value, '', 'Телефон должен быть пустым');

    await closeForm({ save: false });
  });

  await step('reference-non-quickchoice: fillFields на Контрагент (quickChoice=false)', async () => {
    // Поле имеет DLB+CB → fillFields идёт через fillReferenceField (method=dropdown/typeahead).
    // Чистый method='form' путь требует поля без DLB (hasPick && !hasSelect) — в синтетике
    // такого поля нет, поэтому проверяем сам факт корректного заполнения через DLB.
    await navigateSection('Склад');
    await openCommand('Приходная накладная');
    await clickElement('Создать');

    const r = await fillFields({ 'Контрагент': 'ООО Север' });
    log('reference method: ' + r.filled[0]?.method);
    assert.ok(r.filled[0]?.ok, 'fillFields на Контрагент должен сработать');
    assert.ok(['dropdown', 'typeahead', 'form'].includes(r.filled[0]?.method),
      `method=${r.filled[0]?.method} должен быть один из dropdown|typeahead|form`);

    const state = await getFormState();
    const v = findField(state, 'Контрагент')?.value || '';
    log(`Контрагент value='${v}'`);
    assert.includes(v, 'Север', 'Контрагент должен содержать "Север"');

    await closeForm({ save: false });
  });

  await step('radio: КатегорияЦены (RadioButtons) через fillFields, СпособУчёта (Tumbler) через clickElement', async () => {
    // Tumbler-представление не парсится fillFields как radio-поле (см.
    // upload/web-test-bugs.md пункт 5). Но варианты тумблера видны в
    // state.buttons и кликаются через clickElement — покрываем через него.
    await navigateSection('Склад');
    await openCommand('Номенклатура');
    await filterList('Товар 02');
    await clickElement('Товар 02', { dblclick: true });

    // RadioButtons — fillFields с method=radio
    const result = await fillFields({ 'Категория цены': 'Оптовая' });
    log('RadioButtons method: ' + result.filled[0]?.method + ', value: ' + result.filled[0]?.value);
    assert.ok(result.filled[0]?.ok, 'КатегорияЦены fillField должна сработать');
    assert.equal(result.filled[0]?.method, 'radio', 'КатегорияЦены должна использовать method=radio');
    assert.includes(result.filled[0]?.value || '', 'Оптовая', 'КатегорияЦены = Оптовая');

    // Tumbler — варианты «По среднему» / «ФИФО» доступны как buttons
    const before = await getFormState();
    const tumblerButtons = (before.buttons || [])
      .map(b => b.name || b)
      .filter(n => n === 'По среднему' || n === 'ФИФО');
    log('Tumbler buttons: ' + tumblerButtons.join(', '));
    assert.equal(tumblerButtons.length, 2, 'Tumbler должен показывать оба варианта в buttons[]');

    await clickElement('ФИФО');
    log('Tumbler clicked: ФИФО');

    await closeForm({ save: false });
  });

  await step('composite: selectValue с {type} в шапке и ТЧ накладной', async () => {
    // ПриходнаяНакладная.Источник — составной тип:
    //   CatalogRef.Контрагенты + CatalogRef.Номенклатура + CatalogRef.Организации
    // fillFields без type→ошибка с подсказкой «specify the type»;
    // selectValue('Источник', value, {type:'Контрагенты'}) выбирает тип в диалоге.
    await navigateSection('Склад');
    await openCommand('Приходная накладная');
    await clickElement('Создать');

    // Шапка: выбор Контрагента в составном поле
    const headRes = await selectValue('Источник', 'ООО Север', { type: 'Контрагенты' });
    log('header: type=' + headRes.selected?.type + ' method=' + headRes.selected?.method);
    assert.equal(headRes.selected?.method, 'form', 'composite header → method=form');
    assert.equal(headRes.selected?.type, 'Контрагенты', 'type=Контрагенты выбран');

    const state1 = await getFormState();
    const headField = state1.fields?.find(f => f.name === 'Источник');
    assert.equal(headField?.value, 'ООО Север', 'значение в шапке установилось');

    // ТЧ: добавить строку, выбрать тип Организация (квик-чойс — без формы выбора)
    await clickElement('Добавить');
    const rowRes = await fillTableRow(
      { Источник: { value: 'Альфа', type: 'Организации' } },
      { row: 0 },
    );
    log('row: ' + JSON.stringify(rowRes.filled?.[0]));
    assert.equal(rowRes.filled?.[0]?.ok, true, 'composite row → ok');
    assert.equal(rowRes.filled?.[0]?.type, 'Организации', 'выбран тип Организации в ТЧ');

    await closeForm({ save: false });
  });

  await step('direct-edit-form: textEdit:false → fillFields method=form', async () => {
    // ПриходнаяНакладная.Поставщик — обычный CatalogRef.Контрагенты, но
    // элемент формы с textEdit:false: ручной ввод запрещён, выбор только
    // через форму выбора (не через paste/typeahead/dropdown).
    await navigateSection('Склад');
    await openCommand('Приходная накладная');
    await clickElement('Создать');

    const r = await fillFields({ 'Поставщик': 'ООО Юг' });
    log('Поставщик method=' + r.filled[0]?.method);
    assert.equal(r.filled[0]?.ok, true, 'Поставщик заполнен');
    assert.equal(r.filled[0]?.method, 'form',
      'textEdit:false принуждает к method=form (минуя paste/typeahead/dropdown)');

    const state = await getFormState();
    const p = state.fields?.find(f => f.name === 'Поставщик');
    assert.equal(p?.value, 'ООО Юг', 'значение Поставщик установилось');

    await closeForm({ save: false });
  });
}
