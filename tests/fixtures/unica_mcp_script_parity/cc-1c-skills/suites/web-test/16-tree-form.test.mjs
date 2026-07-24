export const name = 'tree-form: FormDataTree edit (ДеревоНоменклатуры obrabotka)';
export const tags = ['tree', 'table', 'picture'];
export const timeout = 90000;

// ДеревоНоменклатуры obrabotka: реквизит формы Дерево типа ДеревоЗначений
// заполняется в ПриСозданииНаСервере рекурсивным обходом справочника Номенклатура.
// Колонки: Номенклатура (CatalogRef, readOnly), Цена (Number, editable),
// Картинка (PictureField на булев, ValuesPicture=StdPicture.Favorites, иконка у Цена>1000),
// Флаг (CheckBoxField на тот же булев — кросс-проверка). Selection-обработчик
// ДеревоВыбор инвертирует Картинка по двойному клику в колонке.
// Покрывает: 05-table/edit-form (fillTableRow method:'direct' на FormDataTree-колонке)
// + 08-hierarchy/tree-edit (expand узла + edit Цены внутри expanded группы)
// + readTable picture-колонки (pic:N/'') и Selection-toggle.
// + дискриминатор choice-ячейки (fillChoiceCell): ДеревоРедактируемаяСтрока (кнопка iCB,
//   пустой НачалоВыбора, текст редактируется → method:'direct') vs ДеревоТипЗначения
//   (РедактированиеТекста=Ложь, текст отвергается → форма выбора, method:'choice').
// + редактируемые choice-ячейки Число/Дата (РедактируемоеЧисло/РедактируемаяДата): маск-инпут
//   переформатирует значение (1234.56 → «1 234,56») — регресс на баг с ложным F4→калькулятор.
// + булево как поле ввода (Булево, InputField, не флажок): выбор Да/Нет через dropdown-путь.

export default async function({ navigateLink, clickElement, closeForm, readTable, fillTableRow, assert, step, log }) {

  await step('setup: открыть обработку ДеревоНоменклатуры', async () => {
    const r = await navigateLink('Обработка.ДеревоНоменклатуры');
    log(`form=${r.form} activeTab=${r.activeTab}`);
    assert.equal(r.activeTab, 'Дерево номенклатуры', 'форма открыта');
    assert.ok(r.tables?.some(t => t.name === 'Дерево'), 'таблица Дерево присутствует');
  });

  await step('read-roots: на верхнем уровне видны группы (Товары, Услуги, БольшойСписок)', async () => {
    const t = await readTable('Дерево');
    log(`columns=${t.columns?.join(',')} rows=${t.rows?.length}`);
    assert.deepEqual(t.columns, ['Номенклатура', 'Цена', 'Картинка', 'Флаг', 'Тип значения', 'Редактируемая строка', 'Редактируемое число', 'Редактируемая дата', 'Булево'], 'колонки: Номенклатура + Цена + Картинка + Флаг + Тип значения + Редактируемая строка + Редактируемое число + Редактируемая дата + Булево');
    assert.equal(t.rows.length, 3, '3 корневые строки');
    const names = t.rows.map(r => r['Номенклатура']);
    assert.includes(names, 'Товары', 'есть Товары');
    assert.includes(names, 'Услуги', 'есть Услуги');
    assert.includes(names, 'БольшойСписок', 'есть БольшойСписок');
    assert.ok(t.rows.every(r => r._kind === 'group'), 'все корневые — group (есть expand-стрелка)');
  });

  await step('expand: clickElement({expand}) раскрывает Товары — 15 элементов', async () => {
    const r = await clickElement('Товары', { expand: true });
    log(`clicked: ${JSON.stringify(r.clicked)}`);
    assert.equal(r.clicked?.toggled, true, 'expand toggled');
    const t = await readTable('Дерево');
    log(`after expand: total=${t.total}`);
    assert.ok(t.total >= 16, `Товары + 15 элементов (got ${t.total})`);
    const tovar01 = t.rows.find(row => row['Номенклатура'] === 'Товар 01');
    assert.ok(tovar01, 'Товар 01 виден внутри Товары');
    assert.equal(tovar01['Цена'], '100,00', 'исходная Цена 100,00 (из справочника)');
  });

  await step('tree-edit: fillTableRow меняет Цену в развёрнутой группе', async () => {
    // row:1 — это Товар 01 (row:0 — Товары после expand). Используем index, т.к.
    // fillTableRow{row:'Товар 01'} ловит SyntaxError в JS-эвале — TODO в bug list.
    const r = await fillTableRow({ Цена: 1500 }, { row: 1 });
    log(`filled: ${JSON.stringify(r.filled)}`);
    assert.equal(r.filled?.length, 1, '1 поле заполнено');
    assert.equal(r.filled[0].field, 'Цена', 'поле Цена');
    assert.equal(r.filled[0].method, 'direct', 'method=direct (in-place edit)');
    assert.equal(r.filled[0].ok, true, 'ok=true');
    const t = await readTable('Дерево');
    const tovar01 = t.rows.find(row => row['Номенклатура'] === 'Товар 01');
    assert.ok(tovar01, 'Товар 01 виден');
    // 1С web использует non-breaking space ( ) как разделитель разрядов
    assert.equal(tovar01['Цена'], '1 500,00', 'Цена обновилась до 1 500,00');
  });

  await step('choice-direct: редактируемая choice-ячейка заполняется прямым вводом (method:direct)', async () => {
    // ДеревоРедактируемаяСтрока — поле с кнопкой выбора (iCB), но пустым НачалоВыбора и
    // РедактированиеТекста=Истина: текст ПРИЛИПАЕТ. fillChoiceCell определяет это поведенчески
    // (paste прилип → stuck) и вводит напрямую, не уходя в форму. Модель ячейки «Значение»
    // типовой Консоли запросов (была баг no_selection_form).
    const r = await fillTableRow({ 'Редактируемая строка': 'привет' }, { row: 1 });
    log(`filled: ${JSON.stringify(r.filled)}`);
    const cell = r.filled?.find(f => /Редактируем/i.test(f.field));
    assert.ok(cell, 'поле Редактируемая строка в результате');
    assert.equal(cell.ok, true, 'ok=true');
    assert.equal(cell.method, 'direct', 'method=direct (прямой ввод, форма не открывалась)');
    const t = await readTable('Дерево');
    const tovar01 = t.rows.find(row => row['Номенклатура'] === 'Товар 01');
    assert.equal(tovar01['Редактируемая строка'], 'привет', 'значение введено напрямую');
  });

  await step('choice-cell: fillTableRow задаёт ТипЗначения через форму выбора (НачалоВыбора)', async () => {
    // Колонка-строка с кнопкой выбора + обработчиком НачалоВыбора → СписокТипов.ПоказатьВыборЭлемента
    // («Выбрать тип»), РедактированиеТекста=Ложь. Прямой ввод ОТВЕРГАется — fillChoiceCell видит
    // stuck=false и открывает форму выбора, выбирая из списка (method:choice, не direct).
    const r = await fillTableRow({ ТипЗначения: 'Число' }, { row: 1 });
    log(`filled: ${JSON.stringify(r.filled)}`);
    const cell = r.filled?.find(f => f.field === 'ТипЗначения');
    assert.ok(cell, 'поле ТипЗначения в результате');
    assert.equal(cell.ok, true, 'ok=true');
    assert.equal(cell.method, 'choice', 'method=choice (выбор из списка, не direct)');
    const t = await readTable('Дерево');
    const tovar01 = t.rows.find(row => row['Номенклатура'] === 'Товар 01');
    assert.equal(tovar01['Тип значения'], 'Число', 'ТипЗначения = Число');
  });

  await step('choice-exact: при подстрочной неоднозначности выбирается точное совпадение', async () => {
    // СписокТипов содержит «Дата» и «Дата документа». Поиск «Дата» даёт 2 подстрочных
    // совпадения — pickFromTypeDialog должен предпочесть ТОЧНОЕ «Дата», а не ругаться
    // на неоднозначность и не выбрать «Дата документа» (Проблема 2 из bug-report).
    const r = await fillTableRow({ ТипЗначения: 'Дата' }, { row: 1 });
    const cell = r.filled?.find(f => f.field === 'ТипЗначения');
    assert.ok(cell, 'поле ТипЗначения в результате');
    assert.equal(cell.ok, true, 'ok=true (exact-match разрешил неоднозначность)');
    assert.equal(cell.method, 'choice', 'method=choice');
    const t = await readTable('Дерево');
    const tovar01 = t.rows.find(row => row['Номенклатура'] === 'Товар 01');
    assert.equal(tovar01['Тип значения'], 'Дата', 'выбрано точное «Дата», не «Дата документа»');
  });

  await step('choice-cell-negative: несуществующий тип → ok:false/not_found (форма не закрывается)', async () => {
    // not_found гасит только диалог выбора типа (умный dismiss), исходная форма остаётся —
    // следующие шаги (picture) это подтверждают.
    const r = await fillTableRow({ ТипЗначения: 'Нетакоготипа' }, { row: 1 });
    const cell = r.filled?.find(f => f.field === 'ТипЗначения');
    assert.ok(cell, 'поле ТипЗначения в результате');
    assert.equal(cell.ok, false, 'ok=false для несуществующего типа');
    assert.equal(cell.error, 'not_found', 'error=not_found');
  });

  await step('choice-number: редактируемая choice-ячейка Число — paste переформатируется, method:direct', async () => {
    // РедактируемоеЧисло — Number-колонка с кнопкой выбора (iCB) и пустым НачалоВыбора (модель
    // «Значение» КЗ). Маск-инпут ПЕРЕФОРМАТИРУЕТ «1234.56» → «1 234,56» (nbsp-разделитель, запятая).
    // Регресс на баг, где includes-проверка рвалась о переформатирование → ложное F4 → калькулятор.
    // Теперь дискриминатор поведенческий (инпут изменился + нет EDD → direct), формат не важен.
    const r = await fillTableRow({ 'Редактируемое число': '1234.56' }, { row: 1 });
    log(`filled: ${JSON.stringify(r.filled)}`);
    const cell = r.filled?.find(f => /Редактируемое число/i.test(f.field));
    assert.ok(cell, 'поле Редактируемое число в результате');
    assert.equal(cell.ok, true, 'ok=true');
    assert.equal(cell.method, 'direct', 'method=direct (числовой маск-инпут, без F4/калькулятора)');
    const t = await readTable('Дерево');
    const tovar01 = t.rows.find(row => row['Номенклатура'] === 'Товар 01');
    // 1С web использует неразрывный пробел как разделитель разрядов — убираем все пробелы перед сравнением.
    assert.equal((tovar01['Редактируемое число'] || '').replace(/[\s\u00A0]/g, ''), '1234,56', 'число переформатировано в 1234,56');
  });

  await step('choice-date: редактируемая choice-ячейка Дата — method:direct', async () => {
    // РедактируемаяДата — Date-колонка с кнопкой выбора и пустым НачалоВыбора. Та же модель «Значение».
    const r = await fillTableRow({ 'Редактируемая дата': '15.06.2025' }, { row: 1 });
    log(`filled: ${JSON.stringify(r.filled)}`);
    const cell = r.filled?.find(f => /Редактируемая дата/i.test(f.field));
    assert.ok(cell, 'поле Редактируемая дата в результате');
    assert.equal(cell.ok, true, 'ok=true');
    assert.equal(cell.method, 'direct', 'method=direct');
    const t = await readTable('Дерево');
    const tovar01 = t.rows.find(row => row['Номенклатура'] === 'Товар 01');
    assert.equal(tovar01['Редактируемая дата'], '15.06.2025', 'дата записана');
  });

  await step('bool-input: булева ячейка-поле-ввода (не флажок) заполняется выбором Да', async () => {
    // ДеревоБулево — InputField на булевом пути (НЕ CheckBoxField) с кнопкой выбора (iCB) и
    // пустым НачалоВыбора: в ячейке выбор Да/Нет, fillTableRow идёт через dropdown-путь, а не
    // toggle. Покрывает булево как поле ввода (модель «Значение» типа Булево в Консоли запросов).
    const r = await fillTableRow({ 'Булево': 'Да' }, { row: 1 });
    log(`filled: ${JSON.stringify(r.filled)}`);
    const cell = r.filled?.find(f => /Булево/i.test(f.field));
    assert.ok(cell, 'поле Булево в результате');
    assert.equal(cell.ok, true, 'ok=true');
    const t = await readTable('Дерево');
    const tovar01 = t.rows.find(row => row['Номенклатура'] === 'Товар 01');
    assert.equal(tovar01['Булево'], 'Да', 'Булево = Да');
  });

  await step('picture: колонка-картинка (pic:0/\'\') + кросс-проверка чекбоксом', async () => {
    const t = await readTable('Дерево');
    const t15 = t.rows.find(r => r['Номенклатура'] === 'Товар 15');  // Цена 1500 > 1000 → иконка
    const t02 = t.rows.find(r => r['Номенклатура'] === 'Товар 02');  // Цена 200 < 1000 → нет
    assert.ok(t15 && t02, 'Товар 15 и Товар 02 видны');
    assert.equal(t15['Картинка'], 'pic:0', 'Товар 15 — иконка показана (pic:0)');
    assert.equal(t15['Флаг'], 'true', 'Товар 15 — флаг true');
    assert.equal(t02['Картинка'], '', 'Товар 02 — иконки нет (пусто)');
    assert.equal(t02['Флаг'], 'false', 'Товар 02 — флаг false');
    // Инвариант: иконка есть тогда и только тогда, когда флаг true.
    const items = t.rows.filter(r => (r['Номенклатура'] || '').startsWith('Товар '));
    assert.ok(items.length >= 15, 'видны все 15 товаров');
    assert.ok(items.every(r => (r['Картинка'] === 'pic:0') === (r['Флаг'] === 'true')),
      'по всем товарам: картинка ⟺ флаг');
  });

  await step('picture-toggle: Selection инвертирует Картинка по двойному клику', async () => {
    await clickElement({ row: { 'Номенклатура': 'Товар 02' }, column: 'Картинка' }, { dblclick: true });
    const t = await readTable('Дерево');
    const t02 = t.rows.find(r => r['Номенклатура'] === 'Товар 02');
    assert.equal(t02['Картинка'], 'pic:0', 'после двойного клика иконка появилась');
    assert.equal(t02['Флаг'], 'true', 'и флаг стал true');
  });

  await step('cleanup: закрыть форму', async () => {
    await closeForm();
  });
}
