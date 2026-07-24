export const name = 'errors: fetchErrorStack Path 1 + dismiss platform dialogs';
export const tags = ['errors', 'stack'];
export const timeout = 60000;

export default async function({ navigateLink, clickElement, closeForm, getFormState, getPage, assert, step, log }) {

  await step('path1: серверное ВызватьИсключение → автоматически фетчится стек через OpenReport', async () => {
    await navigateLink('Обработка.ТестовыеОшибки');
    let caught = null;
    try {
      await clickElement('Вызвать исключение');
    } catch (e) {
      caught = e;
    }
    assert.ok(caught, 'исключение брошено');
    const stack = caught.onecError?.stack;
    log(`stack entries: ${stack?.entries?.length}`);
    assert.ok(stack, 'onecError.stack присутствует');
    assert.ok(stack.timestamp, 'stack.timestamp');
    assert.ok(Array.isArray(stack.entries) && stack.entries.length >= 1, 'stack.entries — непустой массив');
    const root = stack.entries.find(e => /ОбщиеФункции/.test(e.location));
    assert.ok(root, 'в стеке есть кадр из ОбщегоМодуля ОбщиеФункции');
    assert.match(root.code, /ВызватьИсключение/, 'кадр содержит строку с ВызватьИсключение');
  });

  await step('dismiss-modal: оставленная error modal видна в state и закрывается closeForm', async () => {
    // Поток внутри wrapper'a clickElement автоматически зовёт fetchErrorStack и
    // закрывает модалку. Чтобы получить «висящую» модалку — кликаем напрямую
    // через page.click, минуя wrapper.
    await navigateLink('Обработка.ТестовыеОшибки');
    const page = await getPage();
    const btnId = await page.evaluate(() => {
      const el = document.querySelector('[id$="ВызватьИсключение_div"]');
      return el && el.offsetWidth > 0 ? el.id : null;
    });
    assert.ok(btnId, 'кнопка «Вызвать исключение» найдена в DOM');
    await page.click('#' + btnId);
    await page.waitForTimeout(2500);

    const withModal = await getFormState();
    log(`modal present: ${JSON.stringify(withModal.errors?.modal)}`);
    assert.equal(withModal.modal, true, 'state.modal=true пока модалка открыта');
    assert.ok(withModal.errors?.modal, 'state.errors.modal присутствует');
    assert.equal(withModal.errors.modal.message, 'Тестовое исключение', 'modal.message');

    await closeForm();
    const after = await getFormState();
    log(`after closeForm — modal: ${JSON.stringify(after.errors?.modal)} form: ${after.form}`);
    assert.ok(!after.errors?.modal, 'модалка закрыта');
    assert.ok(!after.modal, 'state.modal не true');
  });

  await step('dismiss-platform: открытый «О программе» виден в state.platformDialogs и закрывается closeForm', async () => {
    // Форма ТестовыеОшибки осталась открытой после предыдущего шага (модалка ушла сама)
    const page = await getPage();
    await page.click('#captionbarMore');
    await page.waitForTimeout(800);
    await page.getByText('О программе...', { exact: true }).click();
    await page.waitForTimeout(1500);

    const before = await getFormState();
    log(`platformDialogs: ${JSON.stringify(before.platformDialogs)}`);
    assert.ok(Array.isArray(before.platformDialogs) && before.platformDialogs.length === 1,
      'state.platformDialogs — массив с одним элементом');
    assert.equal(before.platformDialogs[0].type, 'about', 'тип = about');

    await closeForm();
    const after = await getFormState();
    log(`platformDialogs after closeForm: ${after.platformDialogs?.length || 0}`);
    assert.ok(!after.platformDialogs?.length, 'после closeForm нет platformDialogs');
  });

  await closeForm();
}
