import test from 'node:test';
import assert from 'node:assert/strict';
import { GRANTS, GRANT_LABELS, GRANT_NOTES, foundAction, looksLikeInvite, ticketFromHash } from './peers.ts';
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
