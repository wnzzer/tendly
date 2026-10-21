import { test } from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp,rm,realpath,readFile,chmod,copyFile } from 'node:fs/promises';
import { spawn } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { discover, transcriptEvents } from './history.mjs';
import { normalizeSessions } from './history-common.mjs';
import { codexSession } from './history-codex.mjs';

test('native metadata is scoped, bounded, and contains no transcript payload',async()=>{
  const root=await realpath(await mkdtemp(join(tmpdir(),'ad-history-test-')));
  try {
    const rows=await normalizeSessions([{id:'abc-123',name:'test',preview:'private transcript',cwd:root,updatedAt:1700000000},{id:'other',cwd:'/another',updatedAt:1700000000},{id:'--bad',cwd:root,updatedAt:1700000000},{id:'unscoped',updatedAt:1700000000},{id:'empty',cwd:'',updatedAt:1700000000}], 'codex',root,codexSession);
    assert.equal(rows.length,1);assert.equal(rows[0].title,'test');assert.equal(rows[0].preview,undefined);
  }finally{await rm(root,{recursive:true});}
});

async function runCodexFixture(repeatedCursor=false,nativeId=undefined) {
  const root=await realpath(await mkdtemp(join(tmpdir(),'ad-codex-transport-')));
  const fixture=join(root,'codex-history-cli.mjs');
  await copyFile(fileURLToPath(new URL('./fixtures/codex-history-cli.mjs',import.meta.url)),fixture);
  await chmod(fixture,0o755);
  const log=join(root,'requests.log');
  const child=spawn(process.execPath,[fileURLToPath(new URL('./history.mjs',import.meta.url))],{
    env:{...process.env,TENDLY_CODEX_BIN:fixture,TENDLY_SECRET_TEST_TOKEN:'fake-test-secret',TENDLY_TOKEN:'fake-test-token',TENDLY_HISTORY_TEST_LOG:log,TENDLY_HISTORY_TEST_REPEAT:repeatedCursor?'1':'0'},
    stdio:['pipe','pipe','pipe'],
  });
  const chunks=[],errors=[];child.stdout.on('data',chunk=>chunks.push(chunk));child.stderr.on('data',chunk=>errors.push(chunk));
  const timer=setTimeout(()=>child.kill('SIGKILL'),5000);
  try {
    const exited=new Promise((resolve,reject)=>{child.once('error',reject);child.once('close',resolve);});
    child.stdin.end(JSON.stringify({provider:'codex',cwd:root,config_dir:root,native_id:nativeId}));
    const code=await exited;
    assert.ok(chunks.length, `Bridge exited ${code}: ${Buffer.concat(errors).toString()}`);
    return {code,result:JSON.parse(Buffer.concat(chunks).toString()),methods:(await readFile(log,'utf8')).trim().split('\n')};
  }finally{clearTimeout(timer);child.kill('SIGKILL');await rm(root,{recursive:true});}
}

test('Node bridge uses only native metadata RPCs, scopes pages and preserves streamed Unicode',async()=>{
  const {code,result,methods}=await runCodexFixture();
  assert.equal(code,0);assert.equal(result.items.length,1);assert.equal(result.items[0].title,'修复 🦊 布局');
  assert.equal(result.truncated,false);
  assert.deepEqual(methods,['initialize','initialized','thread/list','thread/list']);
});

test('Node bridge rejects stuck pagination without an unbounded query loop',async()=>{
  const {code,result,methods}=await runCodexFixture(true);
  assert.equal(code,1);assert.match(result.error,/pagination did not advance/);
  assert.equal(methods.filter(method=>method==='thread/list').length,2);
});
test('official Claude metadata helper handles an empty configured source without a model call',async()=>{
  const root=await mkdtemp(join(tmpdir(),'ad-empty-claude-'));
  const previous=process.env.CLAUDE_CONFIG_DIR;
  try {const result=await discover({provider:'claude_code',cwd:root,config_dir:root});assert.deepEqual(result.items,[]);}
  finally{if(previous===undefined)delete process.env.CLAUDE_CONFIG_DIR;else process.env.CLAUDE_CONFIG_DIR=previous;await rm(root,{recursive:true});}
});


test('historical messages become UI events without replaying tools or approvals',()=>{
  assert.deepEqual(transcriptEvents([
    {id:'u',role:'user',content:[{type:'text',text:'历史 🦊'},{type:'tool_result',content:'private output'}]},
    {id:'a',role:'assistant',content:[{type:'tool_use',id:'tool'},{type:'text',text:'Done'}]},
    {id:'s',role:'system',content:'system'},
    {id:'t',role:'toolResult',content:'tool'},
  ]),[
    {type:'message',id:'native-u',role:'user',text:'历史 🦊'},
    {type:'message',id:'native-a',role:'assistant',text:'Done'},
  ]);
});


test('loading Codex history reads saved turns without starting or resuming a turn',async()=>{
  const {code,result,methods}=await runCodexFixture(false,'history-1');
  assert.equal(code,0);
  assert.deepEqual(methods,['initialize','initialized','thread/read']);
  assert.deepEqual(result.events.map(event=>[event.role,event.text]),[['user','历史 🦊'],['assistant','Loaded reply']]);
  assert.equal(result.messages,undefined);
});
