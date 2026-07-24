export const name = 'Документ: создание, проведение, проверка в списке';
export const tags = ['document', 'smoke'];
export const timeout = 90000;

export default async function({ navigateSection, openCommand, clickElement, fillFields, fillTableRow, readTable, closeForm, getFormState, assert, step, log }) {

  const docId = `Тест-${Date.now()}`;

  await step('workflow: создать накладную, заполнить, провести и закрыть', async () => {
    await navigateSection('Склад');
    await openCommand('Приходная накладная');
    await clickElement('Создать');

    await fillFields({
      'Контрагент': 'ООО Север',
      'Комментарий': docId,
    });
    await fillTableRow(
      { 'Номенклатура': 'Товар 01', 'Количество': '5', 'Цена': '100' },
      { table: 'Товары', add: true }
    );
    await fillTableRow(
      { 'Номенклатура': 'Товар 02', 'Количество': '3', 'Цена': '200' },
      { table: 'Товары', add: true }
    );

    const before = await getFormState();
    await clickElement('Провести и закрыть');
    const after = await getFormState();
    log(`form before=${before.form} after=${after.form}`);
    assert.notEqual(after.form, before.form, 'После Провести и закрыть текущая форма должна смениться (документ закрылся)');
  });

  await step('verify-list: документ текущего прогона проведён (по Комментарий=docId)', async () => {
    await navigateSection('Склад');
    await openCommand('Приходная накладная');
    const t = await readTable({ maxRows: 50 });
    const candidates = t.rows.filter(r => r['Контрагент'] === 'ООО Север' && r['Проведён'] === 'Да');
    log(`candidates posted Север: ${candidates.length}`);
    assert.ok(candidates.length > 0, 'В списке должен быть хотя бы один проведённый документ Север');

    let foundOurs = null;
    for (const row of candidates) {
      await clickElement(row['Номер'], { dblclick: true });
      const s = await getFormState();
      const cmt = s.fields?.find(f => f.name === 'Комментарий')?.value;
      const num = row['Номер'];
      log(`№${num} Комментарий='${cmt}'`);
      await closeForm();
      if (cmt === docId) { foundOurs = num; break; }
    }
    assert.ok(foundOurs, `Среди проведённых должен быть документ с Комментарий='${docId}'`);
  });
}
