import { test } from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, copyFile, chmod, readFile, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { spawn } from 'node:child_process';
import { once } from 'node:events';
import { StringDecoder } from 'node:string_decoder';
import { piLaunch } from './chat-pi.mjs';

/** The chat bridge driving the synthetic Pi in fixtures/pi-cli.mjs. */
async function pi(run,args=[]){
  const root=await mkdtemp(join(tmpdir(),'tendly-pi-test-')),program=join(root,'pi-cli.mjs'),log=join(root,'native.jsonl');
  await copyFile(fileURLToPath(new URL('./fixtures/pi-cli.mjs',import.meta.url)),program);await chmod(program,0o755);
  const child=spawn(process.execPath,[fileURLToPath(new URL('./chat.mjs',import.meta.url))],{env:{PATH:process.env.PATH,TENDLY_CHAT_TEST_LOG:log},stdio:['pipe','pipe','pipe']});
  const events=[],waiters=new Set(),decoder=new StringDecoder('utf8');let buffer='',stderr='';
  const exited=once(child,'exit');child.stderr.on('data',chunk=>{stderr+=chunk;});
  child.stdout.on('data',chunk=>{buffer+=decoder.write(chunk);let i;while((i=buffer.indexOf('\n'))!==-1){events.push(JSON.parse(buffer.slice(0,i)));buffer=buffer.slice(i+1);for(const notify of [...waiters])notify();}});
  const wait=predicate=>new Promise((resolve,reject)=>{
    const check=()=>{const found=events.find(predicate);if(found){clearTimeout(timer);waiters.delete(check);resolve(found);}};
    const timer=setTimeout(()=>{waiters.delete(check);reject(Error('Timed out waiting for a Pi event: '+JSON.stringify(events)));},5000);
    waiters.add(check);check();
  });
  const send=message=>child.stdin.write(JSON.stringify(message)+'\n');
  const sent=async()=>(await readFile(log,'utf8')).trim().split('\n').map(line=>JSON.parse(line));
  try{
    send({type:'init',provider:'pi',program,args,cwd:root});await wait(event=>event.type==='ready');
    await run({send,wait,events,sent});
    send({type:'shutdown'});await wait(event=>event.type==='exit');
    assert.equal((await exited)[0],0);assert.equal(stderr,'');
  }finally{child.kill('SIGTERM');await rm(root,{recursive:true,force:true});}
}

test('a Pi launch keeps its model, provider, depth and name, and resumes by session',()=>{
  const launch=piLaunch({cwd:'/work',args:['--model','tendly/zai-org/GLM-5.3','--thinking','high','--name','task','--session','pi-session-1']});
  assert.deepEqual(launch.args,['--mode','rpc','--model','tendly/zai-org/GLM-5.3','--thinking','high','--name','task','--session','pi-session-1']);
  assert.throws(()=>piLaunch({cwd:'/work',args:['--mcp-config','x']}),/Unsupported Pi launch option/);
  assert.throws(()=>piLaunch({cwd:'/work',args:['--session','../escape']}),/Invalid or conflicting/);
});

test('pi: a message streams its reply, tools and usage, and the turn settles',()=>pi(async({send,wait,events,sent})=>{
  const ready=await wait(event=>event.type==='ready');
  assert.equal(ready.native_session_id,'pi-session-1');assert.deepEqual(ready.commands,['skill:review']);
  const settings=await wait(event=>event.type==='settings'&&event.models);
  assert.deepEqual(settings.models.map(model=>model.id),['tendly/qwen-local','kunlun/zai-org/GLM-5.3']);
  assert.deepEqual(settings.models[1].efforts,['low','medium','high']);
  assert.equal(settings.model,'tendly/qwen-local');assert.equal(settings.effort,undefined,"Pi's `off` is no depth, not one Tendly rejects");assert.equal(settings.permission_modes,undefined,'no approval modes');
  assert.ok(!(await sent()).some(command=>command.type==='prompt'),'opening a session sends no prompt');
  send({type:'message',id:'one',content:'hello'});
  await wait(event=>event.type==='turn'&&event.id==='one'&&event.status==='completed');
  assert.equal(events.filter(event=>event.type==='message'&&event.role==='assistant'&&event.delta).map(event=>event.text).join(''),'Hello 🦊');
  assert.equal((await wait(event=>event.type==='message'&&event.role==='assistant'&&event.delta===false)).text,'Hello 🦊');
  const tool=await wait(event=>event.type==='tool'&&event.status==='completed');
  assert.equal(tool.name,'bash');assert.match(tool.text,/README\.md/);
  const usage=await wait(event=>event.type==='usage');
  assert.deepEqual([usage.output_tokens,usage.context_tokens,usage.context_window],[4,42,40000]);
  const messageUsage=await wait(event=>event.type==='usage'&&event.turn);
  assert.deepEqual([messageUsage.message_id,messageUsage.turn],['pi-assistant-1',{input:42,output:4,cached:30}]);
  assert.equal((await sent()).find(command=>command.type==='prompt').message,'hello');
}));

test('pi: an extension asking for confirmation waits for the person',()=>pi(async({send,wait,sent})=>{
  send({type:'message',id:'ask',content:'confirm'});
  const approval=await wait(event=>event.type==='approval');
  assert.equal(approval.title,'Delete build output?');assert.deepEqual(approval.choices,['accept','decline','cancel']);
  assert.ok(!(await sent()).some(command=>command.type==='extension_ui_response'),'nothing is answered for the person');
  send({type:'approval',request_id:approval.id,decision:'decline'});
  await wait(event=>event.type==='turn'&&event.id==='ask'&&event.status==='completed');
  assert.deepEqual((await sent()).find(command=>command.type==='extension_ui_response'),{type:'extension_ui_response',id:'ui-1',confirmed:false});
}));

test('pi: a model is chosen by provider and id, and an interrupt aborts the turn',()=>pi(async({send,wait,sent})=>{
  send({type:'model',model:'kunlun/zai-org/GLM-5.3',effort:'high'});
  await wait(event=>event.type==='settings'&&event.model==='kunlun/zai-org/GLM-5.3'&&event.effort==='high');
  const commands=await sent();
  const chosen=commands.find(command=>command.type==='set_model');
  assert.deepEqual([chosen.provider,chosen.modelId],['kunlun','zai-org/GLM-5.3']);
  assert.equal(commands.find(command=>command.type==='set_thinking_level').level,'high');
  send({type:'permission',mode:'danger'});await wait(event=>event.type==='error'&&/no approval modes/.test(event.message));
  send({type:'message',id:'hold',content:'hold'});await wait(event=>event.type==='turn'&&event.id==='hold'&&event.status==='running');
  send({type:'interrupt'});await wait(event=>event.type==='turn'&&event.id==='hold'&&event.status==='interrupted');
  assert.ok((await sent()).some(command=>command.type==='abort'));
}));
