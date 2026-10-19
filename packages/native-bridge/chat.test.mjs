import { test } from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, copyFile, chmod, readFile, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { spawn } from 'node:child_process';
import { once } from 'node:events';
import { PassThrough } from 'node:stream';
import { StringDecoder } from 'node:string_decoder';
import { codexLaunch, CodexChat } from './chat-codex.mjs';
import { claudeLaunch, ClaudeChat } from './chat-claude.mjs';
import { validateChatInput } from './chat.mjs';
import { ChatBase, jsonLines, redactEvents } from './chat-common.mjs';

async function fixture(provider,run,args=[]){
  const root=await mkdtemp(join(tmpdir(),'tendly-chat-test-')),program=join(root,'chat-cli.mjs'),log=join(root,'native.jsonl');
  await copyFile(fileURLToPath(new URL('./fixtures/chat-cli.mjs',import.meta.url)),program);await chmod(program,0o755);
  const child=spawn(process.execPath,[fileURLToPath(new URL('./chat.mjs',import.meta.url))],{env:{PATH:process.env.PATH,TENDLY_CHAT_TEST_LOG:log,TENDLY_CHAT_TEST_INHERITED:'kept',TENDLY_SECRET_TEST_CREDENTIAL:'fixture-secret-value'},stdio:['pipe','pipe','pipe']});
  const events=[],waiters=new Set(),decoder=new StringDecoder('utf8');let buffer='',stderr='',all='';
  const exited=once(child,'exit');child.stderr.on('data',chunk=>{stderr+=chunk;});
  child.stdout.on('data',chunk=>{all+=chunk;buffer+=decoder.write(chunk);let i;while((i=buffer.indexOf('\n'))!==-1){events.push(JSON.parse(buffer.slice(0,i)));buffer=buffer.slice(i+1);for(const notify of [...waiters])notify();}});
  const wait=predicate=>new Promise((resolve,reject)=>{
    const check=()=>{const found=events.find(predicate);if(found){clearTimeout(timer);waiters.delete(check);resolve(found);}};
    const timer=setTimeout(()=>{waiters.delete(check);reject(Error('Timed out waiting for fixture event: '+JSON.stringify(events)));},5000);
    waiters.add(check);check();
  });
  const send=message=>child.stdin.write(JSON.stringify(message)+'\n');
  try{
    send({type:'init',provider,program,args,cwd:root});await wait(event=>event.type==='ready');
    await run({send,wait,events,log:async()=>{try{return(await readFile(log,'utf8')).trim().split('\n').filter(Boolean).map(line=>JSON.parse(line));}catch{return[];}}});
    send({type:'shutdown'});await wait(event=>event.type==='exit');
    const result=await Promise.race([exited,new Promise((_,reject)=>{const timer=setTimeout(()=>reject(Error('Bridge did not exit')),2500);timer.unref();})]);
    assert.equal(result[0],0);assert.equal(stderr,'');assert.equal(all.includes('fixture-secret-value'),false);
  }finally{child.kill('SIGTERM');if(child.exitCode===null&&child.signalCode===null)await new Promise(resolve=>{const timer=setTimeout(()=>{child.kill('SIGKILL');resolve();},2000);child.once('exit',()=>{clearTimeout(timer);resolve();});});await rm(root,{recursive:true,force:true});}
}

test('Codex launch preserves native configuration overrides, translates model and extracts resume without adding permission bypass',()=>{
  const options=codexLaunch({cwd:'/fixture',args:['resume','saved-thread','-c','approval_policy="on-request"','-c','model_reasoning_effort="high"','--model','chosen-model','--no-alt-screen']});
  assert.deepEqual(options.args,['app-server','-c','approval_policy="on-request"','-c','model_reasoning_effort="high"']);assert.equal(options.thread.model,'chosen-model');assert.equal(options.resume,'saved-thread');
  assert.throws(()=>codexLaunch({cwd:'/fixture',args:['--full-auto']}));
});
test('Claude native streaming preserves settings, model and native permission intent without SDK auth or bypass',()=>{
  const options=claudeLaunch({cwd:'/fixture',args:['--model','chosen-model','--permission-mode','plan','--effort','high','--resume=saved-session']});
  assert.equal(options.effort,'high','the launch depth is reported, not left as automatic');
  assert.ok(options.args.includes('plan'));assert.ok(options.args.includes('--effort'));assert.ok(options.args.includes('high'));assert.ok(options.args.includes('user,project,local'));assert.ok(options.args.includes('--permission-prompt-tool'));assert.ok(options.args.includes('stdio'));
  assert.equal(options.resume,'saved-session');assert.ok(!options.args.includes('--bare'));assert.ok(!options.args.includes('bypassPermissions'));
  assert.throws(()=>claudeLaunch({cwd:'/fixture',args:['--dangerously-skip-permissions']}));
});
test('Claude launch accepts a concrete session name for the native client',()=>{
  const options=claudeLaunch({cwd:'/fixture',args:['--name','Review billing']});
  assert.deepEqual(options.args.slice(0,2),['--name','Review billing']);
});

test('Tendly tools reach both clients as the server injects them (agent::equip)',()=>{
  const claude=claudeLaunch({cwd:'/fixture',args:['--resume=saved-session','--mcp-config=/state/agent-mcp/s.json','--allowedTools=mcp__tendly']});
  const at=claude.args.indexOf('--mcp-config');
  assert.equal(claude.args[at+1],'/state/agent-mcp/s.json');
  assert.equal(claude.args[claude.args.indexOf('--allowedTools')+1],'mcp__tendly');
  const codex=codexLaunch({cwd:'/fixture',args:['-c','mcp_servers.tendly.command="/bin/tendly"','-c','mcp_servers.tendly.args=["mcp"]','-c','mcp_servers.tendly.env={TENDLY_AGENT_TOKEN="adk_1"}','-c','mcp_servers.tendly.tool_timeout_sec=600','resume','saved-thread']});
  assert.deepEqual(codex.args,['app-server','-c','mcp_servers.tendly.command="/bin/tendly"','-c','mcp_servers.tendly.args=["mcp"]','-c','mcp_servers.tendly.env={TENDLY_AGENT_TOKEN="adk_1"}','-c','mcp_servers.tendly.tool_timeout_sec=600']);
  assert.equal(codex.resume,'saved-thread');
});

for(const provider of ['codex','claude_code']){
  test(`${provider}: ready performs no prompt; structured deltas and native session IDs appear only after explicit input`,async()=>fixture(provider,async({send,wait,log})=>{
    assert.ok(!(await log()).some(item=>item.method==='turn/start'||item.method==='thread/start'||item.type==='user'));
    send({type:'message',id:'one',content:'simple'});await wait(event=>event.type==='turn'&&event.id==='one'&&event.status==='completed');
    assert.equal((await wait(event=>event.type==='message'&&event.role==='assistant'&&event.delta===false)).text,'Hello 🦊');
    assert.ok((await wait(event=>event.type==='ready'&&event.native_session_id)).native_session_id);
    assert.equal((await wait(event=>event.type==='usage')).output_tokens,4);
  }));
  test(`${provider}: native approvals wait for an explicit decision, never auto-allow or persist new permission rules`,async()=>fixture(provider,async({send,wait,log})=>{
    send({type:'message',id:'approve',content:'approve'});const approval=await wait(event=>event.type==='approval');
    assert.deepEqual(approval.choices,['accept','decline','cancel']);
    assert.ok(!(await log()).some(item=>item.type==='control_response'||(item.id==='approval-1'&&!item.method)));
    send({type:'approval',request_id:approval.id,decision:'decline'});await wait(event=>event.type==='approval_resolved');await wait(event=>event.type==='turn'&&event.status==='completed');
    assert.equal((await wait(event=>event.type==='tool'&&event.status==='failed')).status,'failed');
  }));
  test(`${provider}: native questions preserve answer mapping`,async()=>fixture(provider,async({send,wait})=>{
    send({type:'message',id:'ask',content:'ask'});const approval=await wait(event=>event.type==='approval');assert.equal(approval.questions.length,1);
    send({type:'approval',request_id:approval.id,decision:'accept',answers:{[approval.questions[0].id]:['Brief']}});await wait(event=>event.type==='turn'&&event.status==='completed');
  }));
  test(`${provider}: accept maps to a one-time native decision and cancel interrupts rather than granting a persistent rule`,async()=>fixture(provider,async({send,wait,log})=>{
    send({type:'message',id:'accept',content:'approve'});const first=await wait(event=>event.type==='approval');
    send({type:'approval',request_id:first.id,decision:'accept'});await wait(event=>event.type==='turn'&&event.id==='accept'&&event.status==='completed');
    const records=await log();const reply=records.find(item=>provider==='codex'?item.id==='approval-1'&&!item.method:item.type==='control_response');
    assert.equal(provider==='codex'?reply.result.decision:reply.response.response.behavior,provider==='codex'?'accept':'allow');
    send({type:'message',id:'cancel',content:'approve'});const second=await wait(event=>event.type==='approval'&&event.id!==first.id);
    send({type:'approval',request_id:second.id,decision:'cancel'});await wait(event=>event.type==='turn'&&event.id==='cancel'&&event.status==='interrupted');
    const delivered=(await log()).find(item=>provider==='codex'?item.id==='approval-2'&&!item.method:item.type==='control_response'&&item.response.request_id==='approval-2');
    assert.equal(provider==='codex'?delivered.result.decision:delivered.response.response.interrupt,provider==='codex'?'cancel':true);
  }));
  test(`${provider}: invalid question answers do not settle or auto-approve the request`,async()=>fixture(provider,async({send,wait})=>{
    send({type:'message',id:'ask-invalid',content:'ask'});const approval=await wait(event=>event.type==='approval');
    send({type:'approval',request_id:approval.id,decision:'accept',answers:{}});await wait(event=>event.type==='error'&&event.message.includes('approval response'));
    send({type:'approval',request_id:approval.id,decision:'accept',answers:{[approval.questions[0].id]:['Brief']}});await wait(event=>event.type==='turn'&&event.id==='ask-invalid'&&event.status==='completed');
  }));
  test(`${provider}: interrupt stops the active turn, not the client, and duplicate message IDs do not trigger another model turn`,async()=>fixture(provider,async({send,wait,log})=>{
    send({type:'message',id:'hold',content:'hold'});await wait(event=>event.type==='tool'&&event.id==='hold-1');send({type:'message',id:'hold',content:'hold'});
    send({type:'interrupt'});await wait(event=>event.type==='turn'&&event.id==='hold'&&event.status==='interrupted');
    send({type:'message',id:'after',content:'simple'});await wait(event=>event.type==='turn'&&event.id==='after'&&event.status==='completed');
    const records=await log();assert.equal(records.filter(item=>provider==='codex'?item.method==='turn/start':item.type==='user').length,2);
  }));
  test(`${provider}: unsupported native requests explicitly fail closed`,async()=>fixture(provider,async({send,wait})=>{
    send({type:'message',id:'unknown',content:'unknown'});await wait(event=>event.type==='error'&&/unsupported|not supported/.test(event.message));await wait(event=>event.type==='turn'&&event.status==='failed');
  }));
}

test('Codex resume happens on the first explicit message and permission grants remain turn-scoped',async()=>fixture('codex',async({send,wait,log})=>{
  assert.ok(!(await log()).some(item=>item.method==='thread/resume'));
  send({type:'message',id:'permissions',content:'permissions'});const request=await wait(event=>event.type==='approval');
  send({type:'approval',request_id:request.id,decision:'decline'});await wait(event=>event.type==='turn'&&event.status==='completed');
  const records=await log();assert.equal(records.find(item=>item.method==='thread/resume').params.threadId,'saved-thread');
  assert.deepEqual(records.find(item=>item.id==='approval-1'&&!item.method).result,{permissions:{},scope:'turn'});
},['resume','saved-thread']));
test('Codex: leaving the unattended mode puts the thread\'s own sandbox back, since a turn\'s sandbox outlives it',async()=>fixture('codex',async({send,wait,log})=>{
  await wait(event=>event.type==='settings'&&event.models?.length);
  send({type:'permission',mode:'danger'});await wait(event=>event.type==='settings'&&event.permission_mode==='danger');
  send({type:'message',id:'free',content:'simple'});await wait(event=>event.type==='turn'&&event.id==='free'&&event.status==='completed');
  send({type:'permission',mode:'ask'});await wait(event=>event.type==='settings'&&event.permission_mode==='ask');
  send({type:'message',id:'guarded',content:'simple'});await wait(event=>event.type==='turn'&&event.id==='guarded'&&event.status==='completed');
  const turns=(await log()).filter(item=>item.method==='turn/start').map(item=>item.params);
  assert.deepEqual(turns.at(-2).sandboxPolicy,{type:'dangerFullAccess'});
  assert.equal(turns.at(-1).approvalPolicy,'on-request');
  assert.deepEqual(turns.at(-1).sandboxPolicy,{type:'readOnly'},'full access must not carry over into ask');
}));

test('Codex restates its models every turn, so a history trimmed past the first announcement still has them',async()=>fixture('codex',async({send,wait,events})=>{
  await wait(event=>event.type==='settings'&&event.models?.length);
  const before=events.filter(event=>event.type==='settings'&&event.models?.length).length;
  send({type:'message',id:'again',content:'simple'});await wait(event=>event.type==='turn'&&event.id==='again'&&event.status==='completed');
  assert.ok(events.filter(event=>event.type==='settings'&&event.models?.length&&event.permission_modes?.length).length>before);
}));

test('Codex file-change and MCP tool items have complete lifecycle events',async()=>fixture('codex',async({send,wait,events})=>{
  send({type:'message',id:'tools',content:'tools'});await wait(event=>event.type==='turn'&&event.status==='completed');
  assert.deepEqual(events.filter(event=>event.type==='tool'&&event.id==='file-1').map(event=>event.status),['running','completed']);
  assert.ok(events.some(event=>event.type==='tool'&&event.id==='file-1'&&event.text.includes('+fixture')));
  assert.ok(events.some(event=>event.type==='tool'&&event.id==='mcp-1'&&event.name==='fixture / read'&&event.status==='completed'));
}));

for(const provider of ['codex','claude_code']){
  test(`${provider}: the client publishes its own models, and choosing one keeps the native context`,async()=>fixture(provider,async({send,wait,events,log})=>{
    const announced=await wait(event=>event.type==='settings'&&event.models?.length);
    // Every entry comes from the client. A model that states no effort levels is
    // offered without any, rather than with a ladder invented here.
    assert.ok(announced.models.every(entry=>typeof entry.id==='string'&&typeof entry.name==='string'));
    assert.ok(announced.models.some(entry=>entry.efforts?.length));
    assert.ok(announced.models.some(entry=>entry.efforts===undefined));
    // A client marks entries it does not offer; showing them would put models
    // in the picker that the account cannot run.
    assert.equal(announced.models.some(entry=>entry.id==='gpt-hidden'),false);
    // A session that never chose a model still runs one, and the depth control
    // reads from the model in use — so the client's own default is reported.
    if(provider==='codex')assert.equal(announced.model,'gpt-fixture');

    send({type:'message',id:'before',content:'simple'});await wait(event=>event.type==='turn'&&event.id==='before'&&event.status==='completed');
    const startsBefore=(await log()).filter(item=>provider==='codex'?item.method==='thread/start':item.type==='control_request'&&item.request?.subtype==='initialize').length;

    const chosen=announced.models[0].id;
    send({type:'model',model:chosen});
    await wait(event=>event.type==='settings'&&event.model===chosen);

    // The guarantee: no context boundary, no restart, no lost session.
    assert.equal(events.some(event=>event.type==='configuration'),false);
    assert.equal(events.some(event=>event.type==='exit'),false);
    const startsAfter=(await log()).filter(item=>provider==='codex'?item.method==='thread/start':item.type==='control_request'&&item.request?.subtype==='initialize').length;
    assert.equal(startsAfter,startsBefore,'the native session was restarted');

    send({type:'message',id:'after',content:'simple'});await wait(event=>event.type==='turn'&&event.id==='after'&&event.status==='completed');
    const records=await log();
    if(provider==='codex')assert.equal(records.filter(item=>item.method==='turn/start').at(-1).params.model,chosen);
    else assert.ok(records.some(item=>item.type==='control_request'&&item.request?.subtype==='set_model'&&item.request.model===chosen));
  }));
  test(`${provider}: a model change is refused while a turn is running rather than applied mid-answer`,async()=>fixture(provider,async({send,wait})=>{
    send({type:'message',id:'hold',content:'hold'});await wait(event=>event.type==='tool'&&event.id==='hold-1');
    send({type:'model',model:'some-model'});
    await wait(event=>event.type==='error'&&/current turn/.test(event.message));
    send({type:'interrupt'});await wait(event=>event.type==='turn'&&event.id==='hold'&&event.status==='interrupted');
  }));
}

test('a refused model change reports the client\'s own reason, not a generic failure',async()=>fixture('claude_code',async({send,wait})=>{
  await wait(event=>event.type==='settings'&&event.models?.length);
  send({type:'model',model:'refused-model'});
  // "Check client settings and version" left the user with nothing to act on
  // when the refusal actually came from the upstream endpoint.
  const error=await wait(event=>event.type==='error');
  assert.match(error.message,/availability probe refused/);
}));

test('input validation rejects malformed controls and oversized prompts without reflecting their content',()=>{
  assert.throws(()=>validateChatInput({type:'message',id:'one',content:'x'.repeat(256*1024+1)}));
  assert.throws(()=>validateChatInput({type:'approval',request_id:'one',decision:'acceptForSession'}));
  assert.throws(()=>validateChatInput({type:'model',model:''}));
  assert.throws(()=>validateChatInput({type:'model',model:'ok','effort':'VERY-HIGH'}));
  assert.throws(()=>validateChatInput({type:'init',provider:'codex',program:'codex',cwd:'relative',args:[]},true));
});
test('native lines are UTF-8 safe, bounded, and reject incomplete streams',async()=>{
  const input=new PassThrough(),events=[],errors=[];const stop=jsonLines(input,value=>events.push(value),value=>errors.push(value),{maxLine:64,lineTimeout:25});
  const bytes=Buffer.from('{"text":"🦊"}\n');input.write(bytes.subarray(0,10));input.write(bytes.subarray(10));assert.equal(events[0].text,'🦊');
  input.write('x'.repeat(65));assert.equal(errors.length,1);stop();input.destroy();
  const partial=new PassThrough();await new Promise(resolve=>{jsonLines(partial,()=>{},message=>{assert.match(message,/timed out/);resolve();},{lineTimeout:10});partial.write('{');});partial.destroy();
});
test('a generated image, one line of megabytes in many chunks, is read whole',()=>{
  const input=new PassThrough(),events=[],errors=[];const stop=jsonLines(input,value=>events.push(value),value=>errors.push(value));
  const image='A'.repeat(6*1024*1024),line=Buffer.from(JSON.stringify({type:'image',result:image})+'\n{"type":"after"}\n');
  for(let at=0;at<line.length;at+=64*1024)input.write(line.subarray(at,at+64*1024));
  assert.deepEqual(errors,[]);assert.equal(events[0].result.length,image.length);assert.equal(events[1].type,'after');
  stop();input.destroy();
});
test('credential-shaped environment values never appear in structured bridge events',()=>{
  const events=[],emit=redactEvents({TENDLY_SECRET_TEST:'private-fixture',PATH:'/bin'},event=>events.push(event));
  emit({type:'error',message:'Diagnostic private-fixture'});assert.equal(events[0].message,'Diagnostic [redacted]');
});
test('oversized native question option lists fail closed before emitting an incompatible approval event',()=>{
  for(const Type of [CodexChat,ClaudeChat]){
    const events=[],sent=[],chat=new Type({},event=>events.push(event));chat.active={id:'active'};chat.nativeSessionId='fixture-thread';chat.port={send:message=>sent.push(message)};
    const questions=[{id:'choice',question:'Select',options:Array.from({length:33},(_,index)=>({label:String(index)}))}];
    chat.request(Type===CodexChat?{id:'request',method:'item/tool/requestUserInput',params:{threadId:'fixture-thread',questions}}:{type:'control_request',request_id:'request',request:{subtype:'can_use_tool',tool_name:'AskUserQuestion',input:{questions}}});
    assert.equal(events.some(event=>event.type==='approval'),false);assert.equal(events.some(event=>event.type==='error'),true);assert.equal(sent.length,1);
  }
});
test('a request outside the workspace says so, and can be allowed for the whole session without touching settings',()=>{
  // Resolved, so the same expectations hold where an absolute path gains a drive letter.
  const cwd=resolve('/work/repo'),hosts=resolve('/etc/hosts'),data=resolve('/srv/data');
  // Claude: its own suggestions are kept to this session.
  {
    const events=[],sent=[],chat=new ClaudeChat({cwd},event=>events.push(event));chat.active={id:'active'};chat.port={send:message=>sent.push(message)};
    try {
    chat.request({type:'control_request',request_id:'read',request:{subtype:'can_use_tool',tool_name:'Read',tool_use_id:'tool',blocked_path:'/etc/hosts',input:{file_path:'/etc/hosts'},permission_suggestions:[{type:'addDirectories',directories:['/etc'],destination:'localSettings'}]}});
    const approval=events.find(event=>event.type==='approval');
    assert.deepEqual(approval.scope,{access:'read',outside:[hosts]});
    assert.deepEqual(approval.choices,['accept','accept_session','decline','cancel']);
    chat.answer({request_id:approval.id,decision:'accept_session'});
    const response=sent.pop().response.response;
    assert.equal(response.behavior,'allow');
    assert.deepEqual(response.updatedPermissions,[{type:'addDirectories',directories:['/etc'],destination:'session'}]);
    // Inside the workspace there is nothing to flag, and no suggestion means no session choice.
    chat.request({type:'control_request',request_id:'edit',request:{subtype:'can_use_tool',tool_name:'Edit',tool_use_id:'tool2',input:{file_path:'src/app.ts'}}});
    const inside=events.filter(event=>event.type==='approval').pop();
    assert.deepEqual(inside.scope,{access:'write'});
    assert.deepEqual(inside.choices,['accept','decline','cancel']);
    } finally { chat.clearApprovals(false); }
  }
  // Codex: acceptForSession when the client offers it, and a session-scoped grant for permissions.
  {
    const events=[],sent=[],chat=new CodexChat({cwd},event=>events.push(event));chat.active={id:'active'};chat.nativeSessionId='fixture-thread';chat.port={send:message=>sent.push(message)};
    try {
    chat.request({id:'run',method:'item/commandExecution/requestApproval',params:{threadId:'fixture-thread',command:'cat /var/log/syslog',cwd,availableDecisions:['accept','acceptForSession','decline','cancel']}});
    const command=events.find(event=>event.type==='approval');
    assert.deepEqual(command.choices,['accept','accept_session','decline','cancel']);
    assert.equal(command.scope.access,'execute');assert.equal(command.scope.command,'cat /var/log/syslog');
    chat.answer({request_id:command.id,decision:'accept_session'});
    assert.deepEqual(sent.pop(),{id:'run',result:{decision:'acceptForSession'}});
    chat.request({id:'grant',method:'item/permissions/requestApproval',params:{threadId:'fixture-thread',reason:'Write outside',permissions:{fileSystem:{write:['/srv/data']}}}});
    const permission=events.filter(event=>event.type==='approval').pop();
    assert.deepEqual(permission.scope,{access:'write',outside:[data]});
    chat.answer({request_id:permission.id,decision:'accept_session'});
    assert.deepEqual(sent.pop(),{id:'grant',result:{permissions:{fileSystem:{write:['/srv/data']}},scope:'session'}});
    // An older client that names no decisions is offered nothing it may not accept.
    chat.request({id:'old',method:'item/fileChange/requestApproval',params:{threadId:'fixture-thread',grantRoot:'/opt'}});
    assert.deepEqual(events.filter(event=>event.type==='approval').pop().choices,['accept','decline','cancel']);
    } finally { chat.clearApprovals(false); }
  }
  assert.doesNotThrow(()=>validateChatInput({type:'approval',request_id:'one',decision:'accept_session'}));
});
test('Codex MCP tool confirmations: Tendly\'s own pass, others ask the person, forms fail closed',()=>{
  const events=[],sent=[],chat=new CodexChat({},event=>events.push(event));chat.active={id:'active'};chat.nativeSessionId='fixture-thread';chat.port={send:message=>sent.push(message)};
  const ask=(id,params)=>chat.request({id,method:'mcpServer/elicitation/request',params:{threadId:'fixture-thread',mode:'form',requestedSchema:{type:'object',properties:{}},message:'Allow tool?',...params}});
  ask('own',{serverName:'tendly'});
  assert.deepEqual(sent.pop(),{id:'own',result:{action:'accept',content:{}}});
  assert.equal(events.some(event=>event.type==='approval'),false);
  ask('other',{serverName:'github'});
  const approval=events.find(event=>event.type==='approval');
  assert.equal(approval.title,'Approve MCP tool call');assert.match(approval.text,/github/);assert.equal(sent.length,0);
  chat.answer({request_id:approval.id,decision:'decline'});
  assert.deepEqual(sent.pop(),{id:'other',result:{action:'decline',content:null}});
  ask('form',{serverName:'github',requestedSchema:{type:'object',properties:{name:{type:'string'}}}});
  assert.equal(sent.pop().error.code,-32601);
  ask('url',{serverName:'github',mode:'url',url:'https://example.test',elicitationId:'e'});
  assert.equal(sent.pop().error.code,-32601);
});
test('an idle message limit rejection closes the host pending turn, but busy rejection never completes another turn',()=>{
  const events=[],chat=new ChatBase({},event=>events.push(event));chat.ready=true;chat.seen=new Map(Array.from({length:4096},(_,index)=>[String(index),'hash']));
  assert.equal(chat.begin({id:'over-limit',content:'new'}),undefined);assert.ok(events.some(event=>event.type==='turn'&&event.id==='over-limit'&&event.status==='failed'));
  events.length=0;chat.active={id:'still-running'};chat.begin({id:'another',content:'new'});assert.equal(events.some(event=>event.type==='turn'),false);assert.equal(chat.active.id,'still-running');
});

test('context size comes from the live context, not the turn-cumulative totals', () => {
  const emitted = [];
  const chat = Object.create(ClaudeChat.prototype);
  chat.emit = event => emitted.push(event);
  chat.active = { id: 'turn-1' };
  chat.tools = new Map();
  chat.approvals = new Map();
  chat.closed = false;
  const usageEvents = () => emitted.filter(event => event.type === 'usage');

  // Each assistant message describes one API call: this is the live context.
  chat.notification({ type: 'assistant', message: { id: 'a1', content: [], usage: { input_tokens: 500, cache_read_input_tokens: 40000 } } });
  assert.equal(usageEvents().at(-1).context_tokens, 40500);
  chat.notification({ type: 'assistant', message: { id: 'a2', content: [], usage: { input_tokens: 700, cache_read_input_tokens: 41000 } } });
  assert.equal(usageEvents().at(-1).context_tokens, 41700);

  // The result totals every call in the turn. Deriving context from them would
  // report ~81k here — roughly double the real occupancy — and would climb with
  // each extra tool round.
  chat.notification({
    type: 'result', subtype: 'success',
    usage: { input_tokens: 1200, cache_read_input_tokens: 81000, output_tokens: 900 },
    modelUsage: { 'claude-x': { contextWindow: 200000 } },
  });
  // The cumulative total is what the turn cost, reported on its own.
  assert.deepEqual(usageEvents().at(-1).turn, { input: 82200, output: 900, cached: 81000 });
  const last = usageEvents().filter(event => !event.turn).at(-1);
  assert.equal(last.context_tokens, undefined, 'the cumulative total must not overwrite the live context');
  assert.equal(last.context_window, 200000, 'the window is still taken from the result');
});

test('context window is forwarded only when the native client states it', () => {
  const emitted = [];
  const chat = Object.create(ChatBase.prototype);
  chat.emit = event => emitted.push(event);
  // Claude reports its window per model; Codex reports one per thread.
  chat.usage({ input_tokens: 10, output_tokens: 4 }, 50000, 200000);
  assert.deepEqual(emitted.at(-1), { type: 'usage', input_tokens: 10, output_tokens: 4, context_tokens: 50000, context_window: 200000 });
  // A client that never states a window leaves the field out entirely, so no
  // percentage can be derived from an assumed model size.
  chat.usage({ input_tokens: 10 }, 50000, undefined);
  assert.equal('context_window' in emitted.at(-1), false);
  for (const invalid of [0, -1, 1.5, Number.NaN, Number.POSITIVE_INFINITY, '200000', null]) {
    chat.usage({ input_tokens: 1 }, 5, invalid);
    assert.equal('context_window' in emitted.at(-1), false, String(invalid));
  }
  // No usable numbers at all means no event rather than an empty one.
  const before = emitted.length;
  chat.usage(undefined, undefined, undefined);
  assert.equal(emitted.length, before);
});

test('Claude subagent output reaches its parent tool card instead of the main reply',()=>{
  const events=[];
  const chat=Object.create(ClaudeChat.prototype);
  chat.emit=event=>events.push(event);
  chat.tools=new Map([['task-1','Task']]);
  chat.active={id:'m1',interrupted:false};
  chat.assistantId='assistant-1';

  // Main-agent text: the ordinary path, unchanged.
  chat.notification({type:'assistant',message:{id:'assistant-1',content:[{type:'text',text:'Delegating.'}]}});
  // Subagent text and a nested tool call, both carrying the parent's id.
  chat.notification({type:'assistant',parent_tool_use_id:'task-1',message:{id:'sub-1',content:[
    {type:'text',text:'Scanning the bridge.'},{type:'tool_use',id:'nested-1',name:'Grep',input:{}},
  ]}});

  const messages=events.filter(event=>event.type==='message');
  // The original `return` existed because subagent text overwrote the main
  // reply. Nothing the subagent says may become a message of its own.
  assert.deepEqual(messages.map(event=>event.text),['Delegating.']);

  const progress=events.filter(event=>event.type==='tool'&&event.activity!==undefined);
  assert.equal(progress.length,1);
  assert.equal(progress[0].id,'task-1','progress must attach to the parent card');
  assert.equal(progress[0].name,'Task');
  assert.equal(progress[0].status,'running');
  assert.match(progress[0].activity,/Scanning the bridge\./);
  assert.match(progress[0].activity,/→ Grep/);
  // A nested tool gets a card of its own, under the subagent, never at the top.
  const nested=events.filter(event=>event.type==='tool'&&event.id==='nested-1');
  assert.equal(nested.length,1);
  assert.equal(nested[0].parent,'task-1');
  // And progress never carries `text`, which would clobber the tool's input.
  assert.equal(progress[0].text,undefined);
});

test('a background subagent stays visible on its card after the turn that started it has ended',()=>{
  const events=[];
  const chat=Object.create(ClaudeChat.prototype);
  chat.emit=event=>events.push(event);chat.tools=new Map();chat.seen=new Map();
  chat.active={id:'m1',interrupted:false};
  // The order Claude Code 2.1 sends: the Agent call, its task, then at once a
  // placeholder result, and the turn ends before the subagent does anything.
  chat.notification({type:'assistant',message:{id:'a1',content:[{type:'tool_use',id:'agent-1',name:'Agent',input:{}}]}});
  chat.notification({type:'system',subtype:'task_started',task_id:'t1',tool_use_id:'agent-1',description:'List /tmp',is_backgrounded:true});
  chat.notification({type:'user',message:{content:[{type:'tool_result',tool_use_id:'agent-1',content:'Async agent launched successfully.'}]}});
  chat.active=undefined;
  chat.notification({type:'assistant',parent_tool_use_id:'agent-1',message:{id:'s1',content:[{type:'tool_use',id:'n1',name:'Bash',input:{}}]}});
  chat.notification({type:'assistant',parent_tool_use_id:'agent-1',message:{id:'s2',content:[{type:'text',text:'Found 12 entries.'}]}});
  chat.notification({type:'system',subtype:'task_notification',task_id:'t1',tool_use_id:'agent-1',status:'completed',summary:'Listed /tmp'});

  const card=events.filter(event=>event.type==='tool'&&event.id==='agent-1');
  // The placeholder result must not settle a card whose work has not started.
  assert.equal(card.find(event=>event.text==='Async agent launched successfully.').status,'running');
  const activity=card.map(event=>event.activity).filter(Boolean).join('\n');
  assert.match(activity,/List \/tmp/);
  assert.match(activity,/→ Bash/);
  assert.match(activity,/Found 12 entries\./);
  assert.equal(card.at(-1).status,'completed');
  assert.ok(!events.some(event=>event.type==='message'),'subagent output never becomes a reply');
  assert.ok(!events.some(event=>event.type==='turn'),'subagent output opens no turn of its own');
});

test('a subagent\'s calls nest under it, down through the subagents it starts',()=>{
  const events=[];
  const chat=Object.create(ClaudeChat.prototype);
  chat.emit=event=>events.push(event);chat.tools=new Map([['agent-1','Agent']]);chat.active=undefined;
  chat.notification({type:'assistant',parent_tool_use_id:'agent-1',message:{content:[{type:'tool_use',id:'agent-2',name:'Agent',input:{description:'inner'}}]}});
  chat.notification({type:'assistant',parent_tool_use_id:'agent-2',message:{content:[{type:'tool_use',id:'read-1',name:'Read',input:{file_path:'/a'}}]}});
  chat.notification({type:'user',parent_tool_use_id:'agent-2',message:{content:[{type:'tool_result',tool_use_id:'read-1',content:'contents'}]}});
  const read=events.filter(event=>event.type==='tool'&&event.id==='read-1');
  assert.deepEqual(read.map(event=>[event.parent,event.status]),[['agent-2','running'],['agent-2','completed']]);
  assert.equal(read[1].text,'contents');
  assert.equal(events.find(event=>event.id==='agent-2').parent,'agent-1');
});

test('task progress carries the client\'s counts and a summary only when it changes',()=>{
  const events=[];
  const chat=Object.create(ClaudeChat.prototype);
  chat.emit=event=>events.push(event);chat.tools=new Map([['agent-1','Agent']]);
  const progress=summary=>chat.notification({type:'system',subtype:'task_progress',task_id:'t1',tool_use_id:'agent-1',description:'x',usage:{total_tokens:3400,tool_uses:12,duration_ms:80000},...(summary?{summary}:{})});
  progress('Reading the bridge');progress('Reading the bridge');progress();
  assert.equal(events.length,3);
  assert.deepEqual(events[0].stats,{tokens:3400,tools:12,ms:80000});
  assert.equal(typeof events[0].at,'number');
  assert.deepEqual(events.map(event=>event.activity),['Reading the bridge',undefined,undefined]);
});

test('a subagent\'s log names what each call was for, from the call\'s own input',async()=>{
  const { callSummary }=await import('./chat-claude.mjs');
  assert.equal(callSummary({command:'npm test',description:'Run the tests'}),'Run the tests');
  assert.equal(callSummary({command:'ls -la\nrm x'}),'ls -la');
  assert.equal(callSummary({file_path:'/home/u/project/src/a.ts'}),'src/a.ts');
  assert.equal(callSummary({pattern:'TODO',path:'/repo/src'}),'TODO · repo/src');
  assert.equal(callSummary({query:'vue scoped css'}),'vue scoped css');
  assert.equal(callSummary({url:'https://example.com/a'}),'example.com/a');
  assert.equal(callSummary({}),undefined);
  const events=[];
  const chat=Object.create(ClaudeChat.prototype);
  chat.emit=event=>events.push(event);chat.tools=new Map([['agent-1','Agent']]);
  chat.notification({type:'assistant',parent_tool_use_id:'agent-1',message:{content:[{type:'tool_use',id:'b1',name:'Bash',input:{command:'python3 check_length.py',description:'Check chapter lengths'}}]}});
  assert.match(events.find(event=>event.activity).activity,/^→ Bash · Check chapter lengths$/);
});

test('a background command is marked as one, and a command never logs itself',()=>{
  const events=[];
  const chat=Object.create(ClaudeChat.prototype);
  chat.emit=event=>events.push(event);chat.tools=new Map([['bash-1','Bash'],['bash-2','Bash']]);chat.active={id:'m1',interrupted:false};
  // Claude Code reports every command as a task, foreground ones too, with the command as its description.
  chat.notification({type:'system',subtype:'task_started',task_id:'t1',tool_use_id:'bash-1',task_type:'local_bash',description:'npm test'});
  chat.notification({type:'system',subtype:'task_started',task_id:'t2',tool_use_id:'bash-2',task_type:'local_bash',description:'npm run watch',is_backgrounded:true});
  chat.notification({type:'user',message:{content:[{type:'tool_result',tool_use_id:'bash-2',content:'Command running in background with ID: x'}]}});
  assert.ok(events.every(event=>event.activity===undefined),'the command is the card\'s input, not its log');
  assert.equal(events.find(event=>event.id==='bash-1').background,undefined);
  assert.ok(events.filter(event=>event.id==='bash-2').every(event=>event.background===true));
  assert.equal(events.at(-1).status,'running');
});

test('a failed turn says what the endpoint said, as the Rust adapter does',async()=>{
  const { failureReason }=await import('./chat-claude.mjs');
  assert.equal(failureReason({api_error_status:401,result:' Failed to authenticate. API Error: 401 bad key '}),'Claude Code could not complete the turn (HTTP 401). Check the key, and that the endpoint takes it as an x-api-key header. Claude Code said: Failed to authenticate. API Error: 401 bad key');
  assert.match(failureReason({api_error_status:404}),/\(HTTP 404\)\. Check the model name/);
  assert.match(failureReason({api_error_status:429}),/limiting requests/);
  assert.match(failureReason({api_error_status:529}),/The endpoint failed/);
  for(const message of [{result:''},{api_error_status:'401'},{api_error_status:7}])assert.equal(failureReason(message),'Claude Code could not complete the turn. Check its native account, endpoint and permission settings.');
});

test('a turn reports what it cost, and a call roughly what its result added',async()=>{
  const { turnUsage, resultTokens }=await import('./chat-claude.mjs');
  assert.deepEqual(turnUsage({input_tokens:100,cache_read_input_tokens:9000,cache_creation_input_tokens:900,output_tokens:450}),{input:10000,output:450,cached:9000});
  assert.equal(turnUsage({}),undefined);
  assert.equal(resultTokens('abcd'.repeat(250)),250);
  assert.equal(resultTokens('中文结果'),4);
  const events=[];
  const chat=Object.create(ClaudeChat.prototype);
  chat.emit=event=>events.push(event);chat.tools=new Map([['read-1','Read']]);chat.active={id:'m1',interrupted:false};
  chat.notification({type:'user',message:{content:[{type:'tool_result',tool_use_id:'read-1',content:'x'.repeat(400000)}]}});
  // Counted before the text is clipped for display.
  assert.equal(events[0].result_tokens,100000);
  assert.ok(events[0].text.length<400000);
});

test('a turn Claude starts by itself, after a background command finished, is shown rather than dropped',()=>{
  const events=[];
  const chat=Object.create(ClaudeChat.prototype);
  chat.emit=event=>events.push(event);chat.tools=new Map();chat.seen=new Map();chat.active=undefined;
  // The queued notification comes back as input Tendly never sent.
  chat.notification({type:'user',isReplay:true,uuid:'note-1',message:{role:'user',content:'<task-notification>…</task-notification>'}});
  chat.notification({type:'assistant',message:{id:'a-1',content:[{type:'text',text:'Tests passed; pushing.'}]}});
  const turns=events.filter(event=>event.type==='turn');
  assert.deepEqual(turns.map(event=>event.status),['running']);
  assert.match(turns[0].id,/^background-/,'its id is not mistaken for a message the user sent');
  assert.deepEqual(events.filter(event=>event.type==='message').map(event=>event.text),['Tests passed; pushing.']);

  // Without a replay first, the reply itself opens the turn.
  events.length=0;chat.active=undefined;
  chat.notification({type:'assistant',message:{id:'a-2',content:[{type:'text',text:'Carrying on.'}]}});
  assert.deepEqual(events.map(event=>event.type),['turn','message']);

  // A background subagent's progress is not a turn of the main agent.
  events.length=0;chat.active=undefined;
  chat.notification({type:'assistant',parent_tool_use_id:'task-1',message:{content:[{type:'text',text:'still going'}]}});
  assert.deepEqual(events,[]);
});

test('subagent output for an unknown parent is dropped rather than inventing a card',()=>{
  const events=[];
  const chat=Object.create(ClaudeChat.prototype);
  chat.emit=event=>events.push(event);chat.tools=new Map();chat.active={id:'m1',interrupted:false};
  chat.notification({type:'assistant',parent_tool_use_id:'never-announced',message:{content:[{type:'text',text:'orphan'}]}});
  assert.deepEqual(events,[]);
});

test('a model announcement keeps the marker naming the configuration default',()=>{
  const events=[];
  const chat=Object.create(ChatBase.prototype);
  chat.emit=event=>events.push(event);
  chat.settings([
    {id:'gpt-5.5',name:'GPT-5.5',efforts:['low','high']},
    {id:'gpt-6-astra',name:'GPT-6-Astra',isDefault:true,efforts:['low','max']},
  ],undefined,undefined);
  const rows=events.at(-1).models;
  // Dropping this here is what left a not-yet-started Codex session unable to
  // offer a thinking depth: with no model named, there was nothing to read
  // levels from, and inventing a list would have been a guess.
  assert.deepEqual(rows.filter(row=>row.isDefault).map(row=>row.id),['gpt-6-astra']);
  assert.deepEqual(rows.find(row=>row.id==='gpt-6-astra').efforts,['low','max']);
  // Only a literal true marks a default; anything else leaves the row unmarked.
  chat.settings([{id:'x',name:'X',isDefault:'yes'}],undefined,undefined);
  assert.equal(events.at(-1).models[0].isDefault,undefined);
});

test('permission mode is the client\'s answer, and the unsupported ones are refused', async () => {
  // Claude switches a session-wide mode; Codex decides per command and has no
  // plan or accept-edits. The offered list therefore comes from the client, so
  // the composer never shows a control that errors when used.
  const { ClaudeChat } = await import('./chat-claude.mjs');
  const { CodexChat } = await import('./chat-codex.mjs');
  assert.deepEqual(ClaudeChat.prototype.permissionModes, ['ask', 'plan', 'accept_edits', 'danger']);
  assert.deepEqual(CodexChat.prototype.permissionModes, ['ask', 'danger']);
});

test('the launch permits switching into the unattended mode without starting in it', async () => {
  const { claudeLaunch } = await import('./chat-claude.mjs');
  const { args } = claudeLaunch({ args: [], cwd: '/tmp', program: 'claude' });
  // The client refuses set_permission_mode to bypassPermissions unless launched
  // this way; this is the flag that allows rather than the one that enables.
  assert.ok(args.includes('--allow-dangerously-skip-permissions'), args.join(' '));
  assert.ok(!args.includes('--dangerously-skip-permissions'), 'that flag would start the session already bypassed');
  // And a launch option asking for bypass directly is still refused.
  assert.throws(() => claudeLaunch({ args: ['--permission-mode', 'bypassPermissions'], cwd: '/tmp', program: 'claude' }), /bypass/i);
});

test('an invented permission mode never reaches a client', async () => {
  const { validateChatInput } = await import('./chat.mjs');
  for (const mode of ['bypassPermissions', 'danger-full-access', '', 'DANGER', 'ask ', undefined])
    assert.throws(() => validateChatInput({ type: 'permission', mode }), /permission mode/i, String(mode));
  for (const mode of ['ask', 'plan', 'accept_edits', 'danger'])
    assert.equal(validateChatInput({ type: 'permission', mode }).mode, mode);
});

test('the client answers in its own vocabulary, and never in the interface\'s', async () => {
  const { ClaudeChat } = await import('./chat-claude.mjs');
  const settingsFor = async (reply, requested) => {
    const events = [];
    const chat = new ClaudeChat({ args: [] }, event => events.push(event), {});
    chat.port = { rpc: async () => reply };
    await chat.setPermissionMode({ mode: requested });
    return events.filter(event => event.type === 'settings').pop();
  };
  // `default` is what Claude calls asking every time. Reporting that word would
  // put a mode on screen that no control offers and no label names.
  assert.equal((await settingsFor({ mode: 'default' }, 'ask')).permission_mode, 'ask');
  assert.equal((await settingsFor({ mode: 'bypassPermissions' }, 'danger')).permission_mode, 'danger');
  assert.equal((await settingsFor({ mode: 'acceptEdits' }, 'accept_edits')).permission_mode, 'accept_edits');
  // A client that answers with silence still applied what was asked of it.
  assert.equal((await settingsFor({}, 'plan')).permission_mode, 'plan');
  // An answer this bridge cannot name is left for the status message to settle,
  // rather than reported as the mode that happened to be requested.
  assert.equal((await settingsFor({ mode: 'someFutureMode' }, 'ask'))?.permission_mode, undefined);
});

const STEER_A='aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa', STEER_B='bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb';
const turns=events=>events.filter(event=>event.type==='turn').map(event=>event.id+':'+event.status);

test('claude_code: a message steered into a running turn joins it when read in time',async()=>fixture('claude_code',async({send,wait,events,log})=>{
  send({type:'message',id:STEER_A,content:'hold'});await wait(event=>event.type==='turn'&&event.id===STEER_A&&event.status==='running');
  send({type:'steer',id:STEER_B,content:'join'});
  await wait(event=>event.type==='turn'&&event.id===STEER_A&&event.status==='completed');
  await new Promise(resolve=>setTimeout(resolve,150));
  assert.deepEqual(turns(events),[STEER_A+':running',STEER_A+':completed'],'one turn, no phantom second one');
  assert.ok((await log()).some(item=>item.type==='user'&&item.uuid===STEER_B),'written to the client under its own id');
  assert.ok(!events.some(event=>event.type==='message'&&event.id===STEER_B),'the host already recorded the message; the bridge does not echo it');
}));

test('claude_code: a steered message the turn ended without reading runs as the next turn under its own id',async()=>fixture('claude_code',async({send,wait,events})=>{
  send({type:'message',id:STEER_A,content:'hold'});await wait(event=>event.type==='turn'&&event.id===STEER_A&&event.status==='running');
  send({type:'steer',id:STEER_B,content:'late'});
  await wait(event=>event.type==='turn'&&event.id===STEER_B&&event.status==='completed');
  assert.deepEqual(turns(events),[STEER_A+':running',STEER_A+':completed',STEER_B+':running',STEER_B+':completed']);
}));

test('codex: steering goes through turn/steer for the active turn',async()=>fixture('codex',async({send,wait,events,log})=>{
  send({type:'message',id:STEER_A,content:'hold'});await wait(event=>event.type==='turn'&&event.id===STEER_A&&event.status==='running');
  await new Promise(resolve=>setTimeout(resolve,100));
  send({type:'steer',id:STEER_B,content:'also this'});
  await wait(event=>event.type==='turn'&&event.id===STEER_A&&event.status==='completed');
  const steer=(await log()).find(item=>item.method==='turn/steer');
  assert.equal(steer.params.expectedTurnId,'turn-1');assert.equal(steer.params.input[0].text,'also this');
  await new Promise(resolve=>setTimeout(resolve,150));
  assert.deepEqual(turns(events),[STEER_A+':running',STEER_A+':completed']);
}));

test('codex: a turn that refuses steering leaves the message for the next turn, which starts on its own',async()=>fixture('codex',async({send,wait,events,log})=>{
  send({type:'message',id:STEER_A,content:'review'});await wait(event=>event.type==='turn'&&event.id===STEER_A&&event.status==='running');
  await new Promise(resolve=>setTimeout(resolve,100));
  send({type:'steer',id:STEER_B,content:'afterwards'});
  await wait(event=>event.type==='turn'&&event.id===STEER_B&&event.status==='completed');
  assert.deepEqual(turns(events),[STEER_A+':running',STEER_A+':completed',STEER_B+':running',STEER_B+':completed']);
  assert.ok((await log()).some(item=>item.method==='turn/start'&&item.params.input[0].text==='afterwards'));
}));

test('a listed model stays selected when the client reports the id it resolved to',async()=>{
  const { keepsSelection } = await import('./chat-claude.mjs');
  const models=[{id:'default'},{id:'sonnet'},{id:'opus'},{id:'claude-fable-5-1[1m]'}];
  assert.equal(keepsSelection(models,'sonnet','claude-sonnet-5'),true);
  assert.equal(keepsSelection(models,'default','claude-opus-5-5'),true);
  assert.equal(keepsSelection(models,'claude-fable-5-1[1m]','claude-fable-5-1'),true);
  assert.equal(keepsSelection(models,'sonnet','claude-opus-5-5'),false,'a different model replaces the selection');
  assert.equal(keepsSelection(models,'my-custom','claude-sonnet-5'),false,'an unlisted selection follows the client');
});
