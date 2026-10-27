import test from 'node:test';
import assert from 'node:assert/strict';
import { GRANTS, GRANT_LABELS, GRANT_NOTES, foundAction, looksLikeInvite, remoteState, sidebarMachine, ticketFromHash } from './peers.ts';
import { zhCN } from '../i18n/index.ts';

test('a sign-in link from a paired machine carries its ticket in the fragment', () => {
  assert.equal(ticketFromHash('#tendly-ticket=tdt_abc'), 'tdt_abc');
  assert.equal(ticketFromHash('tendly-ticket=tdt_abc&x=1'), 'tdt_abc');
  assert.equal(ticketFromHash('#tendly-ticket=%20%20'), null);
  assert.equal(ticketFromHash('#tendly-token=abc'), null, 'a token is not a ticket');
  assert.equal(ticketFromHash(''), null);
});

test('only a link with an invite code in it is offered for pairing', () => {
  assert.equal(looksLikeInvite('https://box.tail1.ts.net/#tendly-pair=AB12CD34EF56'), true);
  assert.equal(looksLikeInvite('  http://192.168.0.5:28789/settings/machines#tendly-pair=X  '), true);
  assert.equal(looksLikeInvite('https://box.tail1.ts.net/'), false);
  assert.equal(looksLikeInvite('https://box.tail1.ts.net/?tendly-pair=X'), false, 'the code travels in the fragment only');
  assert.equal(looksLikeInvite('AB12CD34EF56'), false);
  assert.equal(looksLikeInvite(''), false);
});

test('every grant is named and explained, in both languages, from least to most', () => {
  assert.deepEqual([...GRANTS], ['view', 'spawn', 'control']);
  for (const grant of GRANTS) {
    for (const text of [GRANT_LABELS[grant], GRANT_NOTES[grant]]) {
      assert.ok(text, grant);
      assert.ok(zhCN[text], `missing translation: ${text}`);
    }
  }
});

test('a machine on the tailnet pairs in one click only when it is the same account\'s', () => {
  const machine = { name: 'box', url: 'https://box.tail1.ts.net/', os: 'linux', tendly: true, version: '0.1.106', pairs: true, paired: false, mine: true };
  assert.equal(foundAction(machine), 'pair');
  assert.equal(foundAction({ ...machine, mine: false }), 'invite', "someone else's needs an invite");
  assert.equal(foundAction({ ...machine, pairs: false }), 'update');
  assert.equal(foundAction({ ...machine, paired: true, pairs: false }), 'paired', 'paired wins over everything');
});

const session = (id, extra = {}) => ({ id, title: `task ${id}`, provider: 'claude_code', status: 'stopped', workspace_id: 'w1', branch: null, activity: 'stopped', ...extra });

test("a session there is waiting, working, idle or stopped, whether it is a chat or a terminal", () => {
  assert.equal(remoteState(session('a', { activity: 'waiting_for_approval', status: 'running' })), 'waiting');
  assert.equal(remoteState(session('a', { activity: 'working', status: 'running' })), 'working');
  assert.equal(remoteState(session('a', { activity: 'idle', status: 'running' })), 'idle');
  assert.equal(remoteState(session('a', { status: 'running' })), 'working', 'a terminal says only that it runs');
  assert.equal(remoteState(session('a', { status: 'waiting' })), 'waiting');
  assert.equal(remoteState(session('a')), 'stopped');
});

test('the sidebar shows what needs you first, a few at a time, and the search narrows it', () => {
  const machine = {
    id: 'm', name: 'box-there', granted: 'control', online: true,
    workspaces: [{ id: 'w1', name: 'shop' }, { id: 'w2', name: 'docs' }],
    sessions: [
      session('1'), session('2', { activity: 'working', status: 'running' }), session('3', { workspace_id: 'w2' }),
      session('4', { activity: 'waiting_for_approval', status: 'running' }), session('5', { temporary: true, status: 'running' }),
    ],
  };
  const all = sidebarMachine(machine);
  assert.deepEqual(all.rows.map(row => row.id), ['4', '2', '1', '3'], 'waiting, working, then newest first; no temporary');
  assert.deepEqual([all.waiting, all.working, all.more, all.hidden], [1, 1, 0, false]);
  assert.equal(all.rows[3].workspace, 'docs');
  assert.deepEqual(sidebarMachine(machine, '', 2).rows.map(row => row.id), ['4', '2']);
  assert.equal(sidebarMachine(machine, '', 2).more, 2);
  assert.deepEqual(sidebarMachine(machine, 'DOCS').rows.map(row => row.id), ['3'], 'by workspace, ignoring case');
  assert.deepEqual(sidebarMachine(machine, 'task 1').rows.map(row => row.id), ['1']);
  assert.equal(sidebarMachine(machine, 'box').rows.length, 4, "the machine's own name shows it all");
  assert.equal(sidebarMachine(machine, 'nothing').hidden, true);
});
