export const name = 'Страницы формы: переключение между Основное и Дополнительно';
export const tags = ['tabs', 'smoke'];
export const timeout = 60000;

export default async function({ navigateSection, openCommand, clickElement, closeForm, getFormState, assert, step, log }) {

  await step('switch: переключение страниц на форме номенклатуры', async () => {
    await navigateSection('Склад');
    await openCommand('Номенклатура');
    await clickElement('Товары', { dblclick: true });
    await clickElement('Товар 01', { dblclick: true });

    const s1 = await getFormState();
    const names1 = s1.fields?.map(f => f.name) || [];
    log(`page1 fields: ${names1.join(', ')}`);
    assert.includes(names1, 'Артикул', 'На странице Основное должен быть Артикул');

    await clickElement('Дополнительно');
    const s2 = await getFormState();
    const names2 = s2.fields?.map(f => f.name) || [];
    log(`page2 fields: ${names2.join(', ')}`);
    assert.notEqual(names2.join(','), names1.join(','), 'Набор полей на странице Дополнительно должен отличаться');

    await clickElement('Основное');
    const s3 = await getFormState();
    const names3 = s3.fields?.map(f => f.name) || [];
    log(`back to page1 fields: ${names3.join(', ')}`);
    assert.includes(names3, 'Артикул', 'После возврата на Основное снова виден Артикул');

    await closeForm({ save: false });
  });
}
