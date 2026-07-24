export const name = 'clickElement: фокус на поле ввода (fallback) + клавиши';
export const tags = ['click', 'focus', 'smoke'];
export const timeout = 120000;

const findField = (state, name) => state.fields?.find(f => f.name === name || f.label === name);

export default async function({ navigateSection, openCommand, clickElement, getFormState, closeForm, getPage, wait, assert, step, log }) {

  await step('focus: clickElement по имени поля ставит фокус, не меняя значение', async () => {
    await navigateSection('Склад');
    await openCommand('Приходная накладная');
    await clickElement('Создать');

    const before = findField(await getFormState(), 'Контрагент')?.value || '';

    const r = await clickElement('Контрагент');
    log('focused: ' + JSON.stringify(r.focused));
    assert.ok(r.focused, 'должен вернуть focused (а не clicked)');
    assert.ok(!r.clicked, 'focus-fallback не должен возвращать clicked');
    assert.equal(r.focused.ok, true, 'фокус должен встать (focused.ok)');
    assert.includes(r.focused.field, 'Контрагент', 'focused.field — имя/заголовок поля');

    const after = findField(await getFormState(), 'Контрагент')?.value || '';
    assert.equal(after, before, 'значение поля не должно измениться от фокуса');

    await closeForm({ save: false });
  });

  await step('keyboard: F4 на сфокусированном поле открывает форму выбора', async () => {
    await navigateSection('Склад');
    await openCommand('Приходная накладная');
    await clickElement('Создать');
    const formCountBefore = (await getFormState()).formCount;

    const r = await clickElement('Контрагент');
    assert.equal(r.focused?.ok, true, 'поле сфокусировано перед F4');

    await getPage().keyboard.press('F4');
    await wait(2);

    const state = await getFormState();
    log(`formCount: ${formCountBefore} → ${state.formCount}`);
    assert.ok(state.formCount > formCountBefore, 'F4 должен открыть форму выбора (formCount вырос)');

    await closeForm({ save: false });   // закрыть форму выбора
    await closeForm({ save: false });   // закрыть накладную
  });

  await step('regress: clickElement по реальной кнопке возвращает clicked, не focused', async () => {
    await navigateSection('Склад');
    await openCommand('Приходная накладная');

    const r = await clickElement('Создать');   // настоящая кнопка
    assert.ok(r.clicked, 'кнопка → clicked');
    assert.ok(!r.focused, 'кнопка не должна резолвиться в focus-fallback');

    await closeForm({ save: false });
  });

  await step('negative: несуществующий таргет по-прежнему бросает not_found', async () => {
    await navigateSection('Склад');
    await openCommand('Приходная накладная');
    await clickElement('Создать');

    await assert.throws(
      () => clickElement('НесуществующееПолеИлиКнопкаXYZ'),
      'clickElement должен бросить, если нет ни контрола, ни поля',
    );

    await closeForm({ save: false });
  });
}
