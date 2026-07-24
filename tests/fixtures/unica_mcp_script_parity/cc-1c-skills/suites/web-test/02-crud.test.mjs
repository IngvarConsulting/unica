export const name = 'CRUD: открытие, чтение, закрытие с подтверждением';
export const tags = ['crud', 'smoke'];
export const timeout = 60000;

export default async function({ navigateSection, openCommand, clickElement, closeForm, readTable, fillField, getFormState, getPage, assert, step, log }) {

  await step('read: список Контрагентов отдаёт колонки/строки/total', async () => {
    await navigateSection('Склад');
    await openCommand('Контрагенты');
    const t = await readTable();
    log(`columns=${t.columns?.length} rows=${t.rows?.length} total=${t.total}`);
    assert.ok(t.total >= 4, `Должно быть >= 4 контрагента (got ${t.total})`);
    assert.ok(t.rows?.length >= 4, 'rows должен содержать заполненные строки');
    const names = t.rows.map(r => r['Наименование']);
    assert.includes(names, 'ООО Север', 'ООО Север должен быть в списке');
    await closeForm();
  });

  await step('open-item: dblclick открывает форму элемента', async () => {
    await navigateSection('Склад');
    await openCommand('Контрагенты');
    await clickElement('ООО Север', { dblclick: true });
    const state = await getFormState();
    const nameField = state.fields?.find(f => f.name === 'Наименование' || f.label === 'Наименование');
    log(`Opened form=${state.form} Наименование='${nameField?.value}'`);
    assert.ok(state.form, 'Форма элемента должна открыться (state.form задан)');
    assert.equal(nameField?.value, 'ООО Север', 'В открытой форме должен быть указан выбранный контрагент');
    await closeForm();
  });

  await step('close-clean: закрытие без изменений не показывает confirmation', async () => {
    await navigateSection('Склад');
    await openCommand('Контрагенты');
    await clickElement('ООО Юг', { dblclick: true });
    const before = await getFormState();
    const after = await closeForm();
    assert.ok(after.closed, 'Форма должна закрыться без диалога');
    assert.ok(!after.confirmation, 'Confirmation dialog не должен появиться');
    log(`closed=${after.closed} form-was=${before.form}`);
  });

  await step('confirm-save-yes: fillField + closeForm({save:true}) → значение сохранилось', async () => {
    await navigateSection('Склад');
    await openCommand('Контрагенты');
    await clickElement('ООО Восток', { dblclick: true });
    const newPhone = '+7 (999) 111-22-33';
    await fillField('Телефон', newPhone);
    await closeForm({ save: true });

    // Verify persisted
    await navigateSection('Склад');
    await openCommand('Контрагенты');
    await clickElement('ООО Восток', { dblclick: true });
    const state = await getFormState();
    const phoneField = state.fields?.find(f => f.name === 'Телефон' || f.label === 'Телефон');
    log(`Re-opened phone='${phoneField?.value}'`);
    assert.equal(phoneField?.value, newPhone, 'Телефон должен сохраниться');
    await closeForm();
  });

  await step('confirm-save-no: closeForm({save:false}) → изменения откатываются', async () => {
    await navigateSection('Склад');
    await openCommand('Контрагенты');
    await clickElement('ООО Восток', { dblclick: true });
    const before = await getFormState();
    const origPhone = before.fields?.find(f => f.name === 'Телефон')?.value;
    log(`origPhone='${origPhone}'`);
    await fillField('Телефон', '+7 (000) 000-00-00');
    const closed = await closeForm({ save: false });
    assert.ok(closed.closed, 'Форма должна закрыться через "Нет"');

    await navigateSection('Склад');
    await openCommand('Контрагенты');
    await clickElement('ООО Восток', { dblclick: true });
    const state = await getFormState();
    const phone = state.fields?.find(f => f.name === 'Телефон')?.value;
    log(`Re-opened phone after save:false='${phone}'`);
    assert.equal(phone, origPhone, 'Телефон не должен измениться (save:false откатил)');
    await closeForm();
  });

  await step('confirm-pending: closeForm() без решения → confirmation в state', async () => {
    await navigateSection('Склад');
    await openCommand('Контрагенты');
    await clickElement('ООО Север', { dblclick: true });
    await fillField('Телефон', '+7 (123) 456-78-90');
    const pending = await closeForm();
    log(`pending: closed=${pending.closed} confirmation=${JSON.stringify(pending.confirmation)}`);
    assert.ok(!pending.closed, 'Форма НЕ должна закрыться без решения');
    assert.ok(pending.confirmation, 'state.confirmation должен присутствовать');
    // Закрыть через явный отказ от сохранения
    await closeForm({ save: false });
  });

  await step('more-menu / submenu-read: clickElement("Ещё") возвращает submenu[] с типовыми пунктами', async () => {
    await navigateSection('Склад');
    await openCommand('Контрагенты');
    const r = await clickElement('Ещё');
    const items = r.submenu || [];
    log(`submenu items: ${items.length} sample=${items.slice(0, 5).join(', ')}`);
    assert.equal(r.clicked?.kind, 'submenu', 'clicked.kind=submenu');
    assert.ok(Array.isArray(r.submenu), 'clickElement("Ещё") должен вернуть submenu[]');
    assert.ok(items.length >= 5, `submenu должен содержать типовые пункты (got ${items.length})`);
    assert.includes(items, 'Создать', 'пункт «Создать»');
    assert.includes(items, 'Изменить', 'пункт «Изменить»');
    assert.includes(items, 'Расширенный поиск', 'пункт «Расширенный поиск»');
    // Закрыть submenu
    const page = await getPage();
    await page.keyboard.press('Escape');
    await closeForm();
  });
}
