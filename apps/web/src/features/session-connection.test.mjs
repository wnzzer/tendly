import { test } from 'node:test';
import assert from 'node:assert/strict';
import { createSessionConnections } from './session-connection-state.ts';
import { createHistorySelection } from './session-connection-history.ts';
import { fuzzyFilter,fuzzyScore } from './fuzzy-search.ts';
const session=(status='stopped')=>({id:'test',status,created_at:'same',updated_at:'same',error:null});
function deferred(){let resolve,reject;const promise=new Promise((yes,no)=>{resolve=yes;reject=no;});return{promise,resolve,reject};}

test('an explicit open shares exactly one pending request across concurrent pane mounts',async()=>{
  let calls=0;const response=deferred();const state=createSessionConnections(()=>{calls++;return response.promise;});
  state.requestOpen('test');
  const a=state.ensure(session()),b=state.ensure(session());
  assert.equal(a,b);await Promise.resolve();assert.equal(calls,1);
  response.resolve(session('running'));await Promise.all([a,b]);
  assert.equal(state.get('test').pending,undefined);
  assert.equal(state.get('test').state,'attached');
  await state.ensure(session('running'));assert.equal(calls,1);
});
test('restoring a stopped layout never starts even a newly-created record',async()=>{
  let calls=0;const state=createSessionConnections(async s=>{calls++;return {...s,status:'running'};});
  assert.equal(await state.ensure(session()),undefined);
  assert.equal(state.get('test').state,'ended');assert.equal(calls,0);
  state.requestOpen('test');await state.ensure(session());assert.equal(calls,1);
});
test('normal running mounts only attach, while explicit open reconciles stale state with the server',async()=>{
  let calls=0;const state=createSessionConnections(async s=>{calls++;return s;});
  await state.ensure(session('running'));assert.equal(calls,0);assert.equal(state.get('test').state,'attached');
  state.requestOpen('test');await state.ensure(session('running'));assert.equal(calls,1);
});
test('repeated explicit opens during pending do not leave a deferred restart intent',async()=>{
  let calls=0;const response=deferred();const state=createSessionConnections(()=>{calls++;return response.promise;});
  state.requestOpen('test');const first=state.ensure(session());
  state.requestOpen('test');assert.equal(state.ensure(session()),first);
  state.requestOpen('test');assert.equal(state.get('test').epoch,1);
  response.resolve(session('running'));await first;
  await state.ensure(session('running'));
  assert.equal(calls,1);assert.equal(state.get('test').explicit,false);
});
test('ending does not resurrect on tab remount; explicit reopen is allowed',async()=>{
  let calls=0;const state=createSessionConnections(async s=>{calls++;return {...s,status:'running'};});
  state.requestOpen('test');await state.ensure(session());state.ended('test');
  await state.ensure(session());await state.ensure(session('running'));assert.equal(calls,1);
  state.requestOpen('test');await state.ensure(session());assert.equal(calls,2);
});
test('failed starts and previously ended records do not enter retry loops',async()=>{
  let calls=0;const state=createSessionConnections(async()=>{calls++;throw Error('offline');});
  state.requestOpen('test');
  await state.ensure(session());await state.ensure(session());assert.equal(calls,1);
  assert.equal(state.get('test').error,'offline');
  const fresh=createSessionConnections(async s=>{calls++;return s;});await fresh.ensure({...session(),updated_at:'after-exit'});assert.equal(calls,1);
  state.requestOpen('test');await state.ensure(session());assert.equal(calls,2);
});
test('a late success after ending is ignored and cannot locally attach a stopped session',async()=>{
  const response=deferred();const state=createSessionConnections(()=>response.promise);
  state.requestOpen('test');const pending=state.ensure(session());await Promise.resolve();
  state.ended('test');response.resolve(session('running'));
  assert.equal(await pending,undefined);assert.equal(state.get('test').state,'ended');
  assert.equal(await state.ensure(session('running')),undefined);
});
test('ending before a queued request dispatch prevents the start request entirely',async()=>{
  let calls=0;const state=createSessionConnections(async s=>{calls++;return{...s,status:'running'};});
  state.requestOpen('test');const pending=state.ensure(session());state.ended('test');
  assert.equal(await pending,undefined);assert.equal(calls,0);assert.equal(state.get('test').state,'ended');
});
test('a late failure after ending cannot overwrite the intentional ended state',async()=>{
  const response=deferred();const state=createSessionConnections(()=>response.promise);
  state.requestOpen('test');const pending=state.ensure(session());await Promise.resolve();
  state.ended('test');response.reject(Error('late offline'));
  assert.equal(await pending,undefined);assert.equal(state.get('test').state,'ended');
  assert.equal(state.get('test').error,undefined);assert.equal(state.get('test').pending,undefined);
});
test('an end/open race waits for the old request to settle and needs a new explicit reopen',async()=>{
  let calls=0;const response=deferred();const state=createSessionConnections(async()=>++calls===1?response.promise:session('running'));
  state.requestOpen('test');const pending=state.ensure(session());await Promise.resolve();
  state.ended('test');state.requestOpen('test');
  response.resolve(session('running'));await pending;
  assert.equal(state.get('test').state,'ended');assert.equal(state.get('test').explicit,false);
  await state.ensure(session());assert.equal(calls,1);
  state.requestOpen('test');await state.ensure(session());assert.equal(calls,2);
});
test('synchronous connection exceptions and non-running start responses are handled without retries',async()=>{
  const failing=createSessionConnections(()=>{throw Error('sync failure');});
  failing.requestOpen('test');assert.equal(await failing.ensure(session()),undefined);
  assert.equal(failing.get('test').state,'failed');assert.equal(failing.get('test').error,'sync failure');
  for(const status of ['stopped','failed']){
    const state=createSessionConnections(async()=>({...session(status),error:status==='failed'?'launch refused':null}));
    state.requestOpen('test');assert.equal(await state.ensure(session()),undefined);
    assert.equal(state.get('test').state,status==='failed'?'failed':'ended');
  }
});
test('a persisted failure is displayed without auto-retrying after a page reload',async()=>{
  let calls=0;const state=createSessionConnections(async s=>{calls++;return s;});
  assert.equal(await state.ensure({...session('failed'),error:'CLI missing'}),undefined);
  assert.equal(calls,0);assert.equal(state.get('test').state,'failed');assert.equal(state.get('test').error,'CLI missing');
});
test('session request state and ended invalidations are isolated by ID',async()=>{
  const one=deferred(),two=deferred();const state=createSessionConnections(s=>s.id==='one'?one.promise:two.promise);
  state.requestOpen('one');state.requestOpen('two');
  const a=state.ensure({...session(),id:'one'}),b=state.ensure({...session(),id:'two'});
  state.ended('one');one.resolve({...session('running'),id:'one'});two.resolve({...session('running'),id:'two'});
  assert.equal(await a,undefined);assert.equal((await b).id,'two');assert.equal(state.get('two').state,'attached');
});
test('native imported history is never resumed by a layout mount',async()=>{
  let calls=0;const state=createSessionConnections(async s=>{calls++;assert.equal(s.native_source_id,'native-codex');return{...s,status:'running'};});
  const history={...session(),native_source_id:'native-codex',provider_session_id:'original'};
  await state.ensure(history);assert.equal(calls,0);
  state.requestOpen('test');await state.ensure(history);assert.equal(calls,1);
});
test('history import requires explicit original-configuration consent for the displayed item',()=>{
  const selection=createHistorySelection();
  const item={id:'native-one',title:'Original thread',provider:'codex',cwd:'/workspace',updated_at:'now',imported_session_id:null};
  assert.equal(selection.payload('workspace','source'),undefined);
  selection.select('workspace','source',item);
  assert.equal(selection.payload('workspace','source'),undefined);
  selection.state.confirmed=true;
  assert.deepEqual(selection.payload('workspace','source'),{source_id:'source',native_id:'native-one',confirmed_original_config:true});
  assert.equal(selection.payload('different-workspace','source'),undefined);
  assert.equal(selection.payload('workspace','different-source'),undefined);
});
test('history can be chosen in batches: toggle, shift ranges, select all, one consent for all',()=>{
  const selection=createHistorySelection();
  const items=['a','b','c','d','e'].map(id=>({id,title:id.toUpperCase(),provider:'codex',cwd:'/w',updated_at:'now',imported_session_id:null}));
  selection.select('w','s',items[1]);
  selection.extend('w','s',items[3],items);
  assert.deepEqual(selection.state.items.map(item=>item.id),['b','c','d']);
  selection.toggle('w','s',items[0]);selection.toggle('w','s',items[2]);
  assert.deepEqual(selection.state.items.map(item=>item.id),['b','d','a']);
  selection.state.confirmed=true;
  assert.deepEqual(selection.payloads('w','s').map(body=>body.native_id),['b','d','a']);
  // Changing what is chosen asks for consent again.
  selection.toggle('w','s',items[4]);assert.equal(selection.state.confirmed,false);assert.equal(selection.payloads('w','s'),undefined);
  selection.toggleAll('w','s',items);assert.equal(selection.state.items.length,5);
  selection.toggleAll('w','s',items);assert.equal(selection.state.items.length,0);
  // Another source starts over rather than mixing items from both.
  selection.toggle('w','s',items[0]);selection.toggle('w','other',items[1]);
  assert.deepEqual(selection.state.items.map(item=>item.id),['b']);
  selection.state.confirmed=true;assert.equal(selection.payloads('w','s'),undefined);
  // Loaded items leave; the rest keep their consent for a retry.
  selection.toggleAll('w','other',items);selection.state.confirmed=true;selection.remove(['a','b']);
  assert.deepEqual(selection.state.items.map(item=>item.id),['c','d','e']);assert.equal(selection.state.confirmed,true);
});
test('changing or clearing history selection revokes previous consent',()=>{
  const selection=createHistorySelection();
  const item={id:'native-one',title:'Original thread',provider:'codex',cwd:'/workspace',updated_at:'now',imported_session_id:null};
  selection.select('workspace','source',item);selection.state.confirmed=true;
  selection.select('workspace','source',{...item,id:'native-two'});
  assert.equal(selection.state.confirmed,false);assert.equal(selection.payload('workspace','source'),undefined);
  selection.state.confirmed=true;selection.clear();
  assert.equal(selection.state.items.length,0);assert.equal(selection.state.confirmed,false);
  assert.equal(selection.payload('workspace','source'),undefined);
});
test('fuzzy sessions match subsequences, Chinese and fullwidth case variants',()=>{
  assert.notEqual(fuzzyScore('c d x','Codex review'),null);
  assert.notEqual(fuzzyScore('认证','修复实名认证流程'),null);
  assert.notEqual(fuzzyScore('ＣＯＤＥＸ','Codex'),null);
  assert.equal(fuzzyScore('not present','Codex'),null);
  assert.deepEqual(fuzzyFilter(['other','Codex','code review x'],'cdx',s=>s),['Codex','code review x']);
});
