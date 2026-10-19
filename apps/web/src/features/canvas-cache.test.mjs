import { test } from 'node:test';
import assert from 'node:assert/strict';
import { canvasStorageKey, readCanvasCache, writeCanvasCache, sameCanvasLayout } from './canvas-cache.ts';
import { capabilitiesFor } from './backend-capabilities.ts';
const first={id:'first-instance',created_at:'2026-01-01'},second={id:'second',created_at:'2026-02-01'};
const layout={version:1,root:{type:'stack',kind:'stack',id:'global',panes:[]}};
const memory=()=>{const data=new Map();return {getItem:key=>data.get(key)??null,setItem:(key,value)=>data.set(key,value)}};
test('server object key ordering does not invent an unsynced layout or recovery conflict',()=>{
  assert.equal(sameCanvasLayout(layout,{root:{panes:[],id:'global',kind:'stack',type:'stack'},version:1}),true);
  assert.equal(sameCanvasLayout(layout,{...layout,root:{...layout.root,id:'different'}}),false);
  const a={version:1,root:{type:'stack',kind:'stack',id:'a',panes:[{type:'pane',id:'a',kind:'editor'},{type:'pane',id:'b',kind:'editor'}]}};
  assert.equal(sameCanvasLayout(a,{...a,root:{...a.root,panes:[...a.root.panes].reverse()}}),false);
});
test('legacy layout cache uses a stable deployment scope when workspaces reorder or grow',()=>{
  const key=canvasStorageKey('http://localhost:5173',[first]);
  assert.equal(key,canvasStorageKey('http://localhost:5173',[second,first]));
  assert.notEqual(key,canvasStorageKey('http://localhost:8788',[first]));
  assert.notEqual(key,canvasStorageKey('http://localhost:5173',[{...first,id:'other-instance'}]));
  assert.equal(canvasStorageKey('http://localhost',[]),undefined);
});
test('local layout caches and unsynced recovery snapshots round trip without shared mutation',()=>{
  const storage=memory();assert.equal(writeCanvasCache(storage,'a',layout,true),true);
  const read=readCanvasCache(storage,'a');assert.equal(read.pending,true);assert.deepEqual(read.layout,layout);
  read.layout.root.id='changed';assert.equal(readCanvasCache(storage,'a').layout.root.id,'global');
  assert.equal(readCanvasCache(storage,'b'),undefined);
});
test('unavailable or corrupt local storage is not reported as durable saving',()=>{
  const broken={getItem(){throw Error('denied')},setItem(){throw Error('quota')}};
  assert.equal(readCanvasCache(broken,'a'),undefined);assert.equal(writeCanvasCache(broken,'a',layout,false),false);
  const storage=memory();storage.setItem('a','{bad');assert.equal(readCanvasCache(storage,'a'),undefined);
  assert.equal(writeCanvasCache(storage,'a',{version:1,root:{type:'bad'}},false),false);
});
test('legacy servers do not get shared-canvas, native history or native configuration calls',()=>{
  assert.deepEqual(capabilitiesFor({ok:true}),{sharedCanvas:false,nativeConfig:false,nativeHistory:false,directories:false,models:false,environment:false,structuredChat:false,accounts:false,sessionConfiguration:false,sessionArchive:false,accountImportNative:false,clients:false,ephemeralSessions:false,sessionTerminalEscape:false,customBalances:false,storedSecrets:false,agentTools:false,selfUpdate:false,modelProbe:false,modelProbePlan:false,resourceMonitoring:false,tailnet:false});
  assert.equal(capabilitiesFor({ok:true,api_version:2}).storedSecrets,false);
  assert.equal(capabilitiesFor({ok:true,api_version:2,capabilities:["stored_secrets"]}).storedSecrets,true);
  assert.equal(capabilitiesFor({ok:true,api_version:2,capabilities:["self_update"]}).selfUpdate,true);
  assert.equal(capabilitiesFor({ok:true,api_version:2}).resourceMonitoring,false);
  assert.equal(capabilitiesFor({ok:true,api_version:2,capabilities:["resource_monitoring"]}).resourceMonitoring,true);
  assert.equal(capabilitiesFor({ok:true,api_version:2,capabilities:["tailnet"]}).tailnet,true);
  // A model check needs the server that answers one; an older one still lists models.
  assert.equal(capabilitiesFor({ok:true,api_version:2,capabilities:["endpoint_models"]}).modelProbe,false);
  assert.equal(capabilitiesFor({ok:true,api_version:2,capabilities:["model_probe"]}).modelProbe,true);
  const explicit=capabilitiesFor({ok:true,api_version:2,capabilities:['shared_canvas']});
  assert.equal(explicit.sharedCanvas,true);assert.equal(explicit.nativeConfig,false);
  const earlier=capabilitiesFor({ok:true,api_version:2});assert.equal(earlier.sharedCanvas,false);assert.equal(earlier.nativeHistory,true);
  assert.equal(earlier.environment,false);
  assert.equal(capabilitiesFor({ok:true,api_version:2,capabilities:['session_environment']}).environment,true);
});
test('session archive is enabled only when explicitly advertised',()=>{
  assert.equal(capabilitiesFor({ok:true,api_version:2}).sessionArchive,false);
  assert.equal(capabilitiesFor({ok:true,api_version:2,capabilities:[]}).sessionArchive,false);
  assert.equal(capabilitiesFor({ok:true,api_version:2,capabilities:['session_archive']}).sessionArchive,true);
});
