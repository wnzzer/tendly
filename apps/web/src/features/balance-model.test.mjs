import { test } from 'node:test';
import assert from 'node:assert/strict';
import { authFields, authFromFields, balanceLabel, balanceIsStale, balanceLevel, balanceShort, roundDecimal, shouldAutoRefresh } from './balance-model.ts';
test('money retains decimal precision and original currency without conversion',()=>{
  assert.equal(balanceLabel({value:'123456789012345678.12',unit:{kind:'money',currency:'CNY'},checked_at:''}),'CNY 123456789012345678.12');
  assert.equal(balanceLabel({value:'0',unit:{kind:'percentage'},checked_at:''}),'0%');
});
test('old results become stale even without another query; errors retain the previous reading',()=>{
  const reading={value:'12',unit:{kind:'money',currency:'USD'},checked_at:'2026-10-15T00:00:00Z'};
  const now=Date.parse(reading.checked_at);
  assert.equal(balanceIsStale(reading,false,now+300001),true);
  assert.equal(balanceIsStale(reading,false,now+1000),false);
  assert.equal(balanceIsStale(reading,true,now+1000),true);
  assert.equal(balanceIsStale(null,false,now),false);
});
test('decimal rounding is exact for values a float cannot hold',()=>{
  assert.equal(roundDecimal('123456789012345678.125',2),'123456789012345678.13');
  assert.equal(roundDecimal('0.994',2),'0.99');
  assert.equal(roundDecimal('9.995',2),'10.00');
  assert.equal(roundDecimal('-0.001',2),'0.00');
  assert.equal(roundDecimal('-12.5',0),'-13');
  assert.equal(roundDecimal('12',2),'12.00');
  assert.equal(roundDecimal('not a number',2),'not a number');
});
const money=(value,currency='USD')=>({value,unit:{kind:'money',currency},checked_at:''});
const percent=value=>({value,unit:{kind:'percentage'},checked_at:''});
test('short labels use the currency symbol, grouping and two decimals; percentages stay readable when small',()=>{
  assert.equal(balanceShort(money('1234.5678'),'en-US'),'$1,234.57');
  assert.equal(balanceShort(money('-3.1','CNY'),'zh-CN'),'-¥3.10');
  assert.equal(balanceShort(money('5','XYZ'),'en-US'),'XYZ 5.00');
  assert.equal(balanceShort(percent('42.6')),'43%');
  assert.equal(balanceShort(percent('3.14')),'3.1%');
  assert.equal(balanceShort(money('1234.5678'),'en-US',true),'$1,235','compact drops cents from 100 up');
  assert.equal(balanceShort(money('99.5'),'en-US',true),'$99.50');
  assert.equal(balanceShort(money('-250.4'),'en-US',true),'-$250');
});
test('levels warn as a percentage runs low and when money reaches zero',()=>{
  assert.equal(balanceLevel(percent('60')),'ok');
  assert.equal(balanceLevel(percent('25')),'warn');
  assert.equal(balanceLevel(percent('10')),'danger');
  assert.equal(balanceLevel(money('0.00')),'danger');
  assert.equal(balanceLevel(money('-1')),'danger');
  assert.equal(balanceLevel(money('0.01')),'ok');
});
test('automatic refresh waits for a minute-old reading and never repeats within the minute',()=>{
  const now=Date.parse('2026-10-15T00:10:00Z');
  const fresh={reading:{value:'1',unit:{kind:'percentage'},checked_at:'2026-10-15T00:09:30Z'}};
  const old={reading:{value:'1',unit:{kind:'percentage'},checked_at:'2026-10-15T00:05:00Z'}};
  assert.equal(shouldAutoRefresh(fresh,undefined,now),false);
  assert.equal(shouldAutoRefresh(old,undefined,now),true);
  assert.equal(shouldAutoRefresh({reading:null},undefined,now),true);
  assert.equal(shouldAutoRefresh(old,now-30_000,now),false);
  assert.equal(shouldAutoRefresh(old,now-61_000,now),true);
});
test('authentication presets round-trip, and custom headers and query names keep what was typed',()=>{
  for (const auth of [
    {kind:'none'},
    {kind:'header',name:'Authorization',value:'Bearer {key}'},
    {kind:'header',name:'x-api-key',value:'{key}'},
    {kind:'header',name:'X-Token',value:'Token {key}'},
    {kind:'query',name:'apikey'},
  ]) assert.deepEqual(authFromFields(authFields(auth)),auth);
  assert.equal(authFields({kind:'header',name:'authorization',value:'Bearer {key}'}).preset,'bearer');
  assert.equal(authFields({kind:'header',name:'Authorization',value:'Token {key}'}).preset,'header');
  const fields=authFields({kind:'none'});
  assert.deepEqual(authFromFields({...fields,preset:'query'}),{kind:'query',name:'api_key'},'a sensible default name');
  assert.deepEqual(authFromFields({...fields,preset:'header',headerName:' X-Key ',headerValue:'{key}'}),{kind:'header',name:'X-Key',value:'{key}'});
});

test('a balance refreshes unless its endpoint turned it off; an account only once turned on', async () => {
  const { balanceAutoRefreshOn, accountAutoRefreshOn } = await import('./balance-model.ts');
  assert.equal(balanceAutoRefreshOn({}, {}), true);
  assert.equal(balanceAutoRefreshOn({ auto_refresh: null }, {}), true);
  assert.equal(balanceAutoRefreshOn({ auto_refresh: false }, {}), false);
  // 0.1.114's switch for every balance stands where the endpoint set nothing.
  assert.equal(balanceAutoRefreshOn({}, { balance_auto_refresh: false }), false);
  assert.equal(balanceAutoRefreshOn({ auto_refresh: true }, { balance_auto_refresh: false }), true);
  assert.equal(accountAutoRefreshOn('a', {}), false);
  assert.equal(accountAutoRefreshOn('a', { auto_refresh_accounts: ['a'] }), true);
  assert.equal(accountAutoRefreshOn('b', { auto_refresh_accounts: ['a'] }), false);
  assert.equal(accountAutoRefreshOn('b', { official_auto_refresh: true }), true);
});

test('switching one account replaces the old every-account switch with the list it meant', async () => {
  const { withAccountAutoRefresh } = await import('./balance-model.ts');
  assert.deepEqual(withAccountAutoRefresh({ codex: {} }, 'a', true, ['a', 'b']), { codex: {}, auto_refresh_accounts: ['a'] });
  assert.deepEqual(withAccountAutoRefresh({ auto_refresh_accounts: ['a', 'b'] }, 'a', false, ['a', 'b']), { auto_refresh_accounts: ['b'] });
  assert.deepEqual(withAccountAutoRefresh({ official_auto_refresh: true }, 'a', false, ['a', 'b', 'c']), { auto_refresh_accounts: ['b', 'c'] });
});

test('a balance is due once its reading and the last try are both an interval old', async () => {
  const { balanceDue, BALANCE_AUTO_REFRESH_MS: every } = await import('./balance-model.ts');
  const now = Date.parse('2026-11-10T12:00:00Z');
  const reading = at => ({ reading: { value: '1', unit: { kind: 'percentage' }, checked_at: new Date(at).toISOString() } });
  assert.equal(balanceDue({ reading: null }, undefined, now), true);
  assert.equal(balanceDue(reading(now - every + 1000), undefined, now), false);
  assert.equal(balanceDue(reading(now - every), undefined, now), true);
  assert.equal(balanceDue(reading(now - every), now - 1000, now), false, 'just tried, maybe failing');
});

test('an official account is due only when signed in, checkable, outside its wait and an interval old', async () => {
  const { officialDue, OFFICIAL_AUTO_REFRESH_MS: every } = await import('./balance-model.ts');
  const now = Date.parse('2026-11-10T12:00:00Z');
  const claude = { status: 'signed_in', checked_at: new Date(now - every).toISOString(), capabilities: { usage: true, quota: false, refresh_token: false } };
  assert.equal(officialDue(claude, undefined, undefined, now), true);
  assert.equal(officialDue({ ...claude, status: 'signed_out' }, undefined, undefined, now), false);
  assert.equal(officialDue({ ...claude, capabilities: { quota: false, refresh_token: false } }, undefined, undefined, now), false);
  assert.equal(officialDue(claude, undefined, now + 60_000, now), false, 'the provider asked to wait');
  assert.equal(officialDue(claude, now - 60_000, undefined, now), false, 'another tab just asked');
  assert.equal(officialDue({ ...claude, checked_at: new Date(now - 60_000).toISOString() }, undefined, undefined, now), false);
});
