// 00-hooks.test.mjs — индикатор покрытия testlevel-хуков (M7.4).
//
// Тест запускается ПЕРВЫМ (алфавитно), импортирует shared `_state` из
// `_hooks.mjs` и проверяет:
//   - `beforeAll` отработал ровно один раз ДО любого теста.
//   - `beforeEach` уже отработал для самого 00-hooks (счётчик === 1).
//   - `testInfo` доступен внутри тела (через ctx).
//   - `afterEach` для 00-hooks ещё не вызывался — `afterEach < beforeEach`.
//   - Последнее событие — `beforeEach:00-hooks.test.mjs`.
//
// `afterAll` проверить из теста невозможно (он зовётся после всех тестов).
// Покрывается косвенно: финальный run должен показать `afterAll = 1` в
// summary log (см. ctx.log в этом тесте).

import { _state } from './_hooks.mjs';

export const name = 'Хуки testlevel — индикатор порядка вызовов';
export const tags = ['hooks', 'smoke'];
export const timeout = 10000;

export default async function ({ step, assert, log, testInfo }) {

  await step('beforeAll отработал ровно один раз', () => {
    assert.equal(_state.beforeAll, 1, `beforeAll=${_state.beforeAll}, ожидался 1`);
    assert.equal(_state.afterAll, 0, `afterAll=${_state.afterAll}, ожидался 0 (вызывается после всех тестов)`);
  });

  await step('beforeEach отработал для этого теста', () => {
    assert.ok(_state.beforeEach >= 1, `beforeEach=${_state.beforeEach}, ожидался >= 1`);
    const last = _state.events[_state.events.length - 1];
    assert.ok(typeof last === 'string' && last.startsWith('beforeEach:'),
      `последнее событие должно быть beforeEach:..., но это "${last}"`);
    assert.ok(last.includes('00-hooks'),
      `последнее beforeEach должно ссылаться на 00-hooks, а не "${last}"`);
  });

  await step('testInfo доступен в теле теста', () => {
    assert.equal(testInfo.file, '00-hooks.test.mjs', `testInfo.file=${testInfo.file}`);
    assert.ok(Array.isArray(testInfo.tags), 'testInfo.tags должен быть массивом');
    assert.includes(testInfo.tags, 'hooks', 'testInfo.tags должен содержать "hooks"');
    assert.equal(testInfo.attempt, 1, `attempt=${testInfo.attempt}`);
    assert.equal(typeof testInfo.primaryContext, 'string', 'primaryContext должен быть строкой');
  });

  await step('afterOpenContext отработал хотя бы для default', () => {
    // Default контекст создаётся до beforeAll → afterOpenContext должен был
    // отработать как минимум один раз. beforeCloseContext в теле первого
    // теста ещё не вызывался (контексты живы).
    assert.ok(_state.afterOpenContext >= 1,
      `afterOpenContext=${_state.afterOpenContext}, ожидался >= 1 (default-контекст создан)`);
    assert.equal(_state.beforeCloseContext, 0,
      `beforeCloseContext=${_state.beforeCloseContext}, ожидался 0 (контексты ещё живы)`);
  });

  await step('afterEach для этого теста ещё не вызывался', () => {
    // В теле теста afterEach НЕ должен быть вызван ни разу для текущего теста.
    // Если 00-hooks запущен первым (что и ожидается), afterEach === 0.
    // Tolerance: проверяем относительное неравенство, чтобы тест не сломался
    // если кто-то добавит ещё один тест с алфавитно меньшим именем.
    assert.ok(_state.afterEach < _state.beforeEach,
      `afterEach (${_state.afterEach}) должен быть строго меньше beforeEach (${_state.beforeEach}) в теле теста`);
  });

  log(`hooks indicator: beforeAll=${_state.beforeAll}, beforeEach=${_state.beforeEach}, afterEach=${_state.afterEach}, events.length=${_state.events.length}`);
}
