export const name = 'misc: openFile EPF + security confirm';
export const tags = ['openfile'];
export const timeout = 120000;

export default async function({ openFile, closeForm, getFormState, assert, step, log }) {
  const fs = await import('fs');
  const path = await import('path');

  const dir = 'test-tmp/13-openfile';
  const buildDir = path.join(dir, 'build');
  const epfPath = path.join(buildDir, 'ТестОткрытия.epf');

  await step('setup: тестовый EPF должен быть собран в prepare()', async () => {
    // Сборка переехала в tests/web-test/_hooks.mjs (EPF_SPEC + buildEpf).
    // Если EPF отсутствует — запустить с `-- --rebuild-epf` или `-- --rebuild-stand`.
    assert.ok(fs.existsSync(epfPath),
      `EPF не найден: ${epfPath}. Запустите раннер с '-- --rebuild-epf' для сборки.`);
    log(`EPF готов: ${epfPath} size=${fs.statSync(epfPath).size}`);
  });

  await step('openFile: открывает EPF с формой и текстовой декорацией (security confirm — авто)', async () => {
    const beforeForm = (await getFormState()).form;
    const r = await openFile(epfPath);
    log(`opened: form=${r.form} activeTab=${r.activeTab} texts=${JSON.stringify(r.texts)}`);
    assert.ok(r.form != null, 'state.form задан после openFile');
    assert.notEqual(r.form, beforeForm, 'открыта новая форма');
    assert.equal(r.activeTab, 'Тест открытия', 'заголовок формы из form-compile');
    // Security confirmation modal обрабатывается внутри openFile — наружу не пробивается
    assert.ok(!r.errors?.modal, 'нет оставшейся modal ошибки (security confirm обработан)');
    // Декорация видна в state.texts[]
    assert.ok(Array.isArray(r.texts) && r.texts.length >= 1, 'state.texts содержит декорации');
    const decor = r.texts.find(t => t.name === 'Заголовок');
    assert.ok(decor, 'декорация «Заголовок» присутствует в texts[]');
    assert.equal(decor.value, 'Это тестовая обработка для проверки openFile', 'текст декорации');
    // attempt=1 → security confirm не понадобился ИЛИ обработан с первой попытки
    assert.ok(r.opened?.attempt >= 1, 'opened.attempt задан');
  });

  await step('cleanup: закрываем форму обработки', async () => {
    await closeForm();
    const s = await getFormState();
    log(`after cleanup: form=${s.form} formCount=${s.formCount} activeTab=${s.activeTab}`);
    // Проверяем что наша EPF-форма точно закрылась. Между тестами в desktop
    // могут оставаться формы от других тестов — это не наш регресс.
    assert.notEqual(s.activeTab, 'Тест открытия', 'форма обработки ТестОткрытия закрыта');
  });
}
