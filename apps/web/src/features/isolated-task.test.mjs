import { test } from 'node:test';
import assert from 'node:assert/strict';
import { copyLines, setupPayload, setupSummary, taskActions, taskBranchPreview, taskTotals } from './isolated-task.ts';

test('the branch preview spells the title as the server will', () => {
  assert.equal(taskBranchPreview('Fix the login page'), 'tendly/fix-the-login-page');
  assert.equal(taskBranchPreview('↳ Try: the other approach!'), 'tendly/try-the-other-approach');
  assert.equal(taskBranchPreview('修登录页'), 'tendly/task-…');
  assert.equal(taskBranchPreview('修登录页 bug'), 'tendly/bug');
  assert.equal(taskBranchPreview('a'.repeat(60)), `tendly/${'a'.repeat(40)}`);
});

test('setup paths are one per line, blank and repeated ones dropped', () => {
  assert.deepEqual(copyLines(' .env\n\nnode_modules\r\n.env\n'), ['.env', 'node_modules']);
  assert.deepEqual(setupPayload('.env', '  pnpm install '), { copy: ['.env'], command: 'pnpm install' });
  assert.deepEqual(setupPayload('', 'make'), { copy: [], command: 'make' });
  assert.equal(setupPayload(' \n', '  '), null);
  assert.deepEqual(setupSummary(null), { kind: 'none' });
  assert.deepEqual(setupSummary({ copy: [], command: ' ' }), { kind: 'none' });
  assert.deepEqual(setupSummary({ copy: ['.env'], command: null }), { kind: 'some', copy: ['.env'], command: null });
});

test('a task merges only committed work, and is put away once nothing is left', () => {
  assert.deepEqual(taskActions({ ahead: 2, uncommitted: 0, preparing: false }), { merge: true, finish: false, discard: true, push: true });
  assert.deepEqual(taskActions({ ahead: 2, uncommitted: 1, preparing: false }), { merge: false, finish: false, discard: true, push: true });
  assert.deepEqual(taskActions({ ahead: 0, uncommitted: 0, preparing: false }), { merge: false, finish: true, discard: true, push: false });
  assert.deepEqual(taskActions({ ahead: 1, uncommitted: 0, preparing: true }), { merge: false, finish: false, discard: false, push: false });
  assert.deepEqual(taskTotals({ files: [{ path: 'a', added: 3, removed: 1 }, { path: 'b', untracked: true }, { path: 'img.png' }] }), { files: 3, added: 3, removed: 1 });
});
