export const name = 'validation: messages panel + exception modal';
export const tags = ['validation', 'errors'];
export const timeout = 60000;

export default async function({ navigateLink, clickElement, closeForm, getFormState, assert, step, log }) {

  await step('open: обработка ТестовыеОшибки доступна через navigateLink', async () => {
    const s = await navigateLink('Обработка.ТестовыеОшибки');
    log(`buttons: ${s.buttons?.map(b => b.name).join(', ')}`);
    assert.ok(s.buttons?.some(b => b.name === 'Показать сообщение'), 'кнопка «Показать сообщение»');
    assert.ok(s.buttons?.some(b => b.name === 'Вызвать исключение'), 'кнопка «Вызвать исключение»');
  });

  await step('messages: Сообщить() показывает текст в панели Сообщения', async () => {
    const r = await clickElement('Показать сообщение');
    log(`errors.messages: ${JSON.stringify(r.errors?.messages)}`);
    assert.ok(Array.isArray(r.errors?.messages), 'errors.messages — массив');
    assert.ok(r.errors.messages.includes('Тестовое сообщение'), 'содержит «Тестовое сообщение»');
    assert.ok(!r.errors.modal, 'модальной ошибки нет — это инфо-панель');
  });

  await step('exception-modal: ВызватьИсключение приводит к onecError.errors.modal', async () => {
    let caught = null;
    try {
      await clickElement('Вызвать исключение');
    } catch (e) {
      caught = e;
    }
    assert.ok(caught, 'clickElement должен бросить ошибку при платформенном исключении');
    assert.equal(caught.message, 'Тестовое исключение', 'e.message = текст исключения');
    const modal = caught.onecError?.errors?.modal;
    log(`modal: ${JSON.stringify(modal)}`);
    assert.ok(modal, 'onecError.errors.modal присутствует');
    assert.equal(modal.message, 'Тестовое исключение', 'modal.message');
    assert.ok(typeof modal.formNum === 'number', 'modal.formNum — число');
    // После throw fetchErrorStack автоматически закрыл модалку — проверим
    const after = await getFormState();
    assert.ok(!after.errors?.modal, 'модалка автоматически закрыта');
    assert.ok(!after.platformDialogs?.length, 'платформенные диалоги не оставлены');
  });

  await closeForm();
}
